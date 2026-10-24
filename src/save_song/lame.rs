//! The few calls Glass makes to LAME, the MP3 encoder. LAME is a library of its
//! own (see build.rs): constant bitrate, LAME's "near best" quality, no Xing tag.

use std::ffi::{c_float, c_int, c_uchar};

/// LAME's encoder state (opaque).
#[repr(C)]
struct Flags {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn lame_init() -> *mut Flags;
    fn lame_set_num_channels(f: *mut Flags, channels: c_int) -> c_int;
    fn lame_set_in_samplerate(f: *mut Flags, rate: c_int) -> c_int;
    fn lame_set_brate(f: *mut Flags, kbps: c_int) -> c_int;
    fn lame_set_quality(f: *mut Flags, quality: c_int) -> c_int;
    #[allow(non_snake_case)]
    fn lame_set_bWriteVbrTag(f: *mut Flags, on: c_int) -> c_int;
    fn lame_init_params(f: *mut Flags) -> c_int;
    fn lame_encode_buffer_ieee_float(f: *mut Flags, left: *const c_float, right: *const c_float, samples: c_int, out: *mut c_uchar, size: c_int) -> c_int;
    fn lame_encode_flush(f: *mut Flags, out: *mut c_uchar, size: c_int) -> c_int;
    fn lame_close(f: *mut Flags) -> c_int;
}

/// LAME's quality setting 2, "near best" (0 is slowest, 9 fastest).
const NEAR_BEST: c_int = 2;
/// What a flush may write at most, per LAME's documentation.
const FLUSH_ROOM: usize = 7200;

pub struct Encoder {
    f: *mut Flags,
    stereo: bool,
}

impl Encoder {
    /// `kbps` must be one of LAME's bitrates (see `mp3::BITRATES`).
    pub fn new(channels: u8, rate: u32, kbps: u16) -> Result<Self, String> {
        // SAFETY: lame_init takes nothing and returns a new state, or null.
        let f = unsafe { lame_init() };
        if f.is_null() {
            return Err("the MP3 encoder couldn't start".into());
        }
        // Closed on drop, also if a setting below is refused.
        let enc = Encoder { f, stereo: channels == 2 };
        let check = |r: c_int, what: &str| if r < 0 { Err(format!("the MP3 encoder refused the {what} ({r})")) } else { Ok(()) };
        // SAFETY: `f` is a live state; these only set fields of it.
        unsafe {
            check(lame_set_num_channels(f, c_int::from(channels)), "channel count")?;
            check(lame_set_in_samplerate(f, rate as c_int), "sample rate")?;
            check(lame_set_brate(f, c_int::from(kbps)), "bitrate")?;
            check(lame_set_quality(f, NEAR_BEST), "quality")?;
            // The Xing/LAME tag needs a second pass over the file; constant bitrate doesn't.
            check(lame_set_bWriteVbrTag(f, 0), "tag setting")?;
            check(lame_init_params(f), "settings")?;
        }
        Ok(enc)
    }

    /// Encodes a block of samples (`right` too in stereo), appending the MP3 to `out`.
    pub fn encode(&mut self, left: &[f32], right: Option<&[f32]>, out: &mut Vec<u8>) -> Result<(), String> {
        let right = match (self.stereo, right) {
            (true, Some(r)) if r.len() == left.len() => r.as_ptr(),
            (false, _) => std::ptr::null(),
            _ => return Err("the MP3 encoder got uneven channels".into()),
        };
        // LAME's own bound for one call: 1.25 × samples + 7200 bytes.
        let room = left.len() * 5 / 4 + FLUSH_ROOM;
        out.reserve(room);
        let start = out.len();
        // SAFETY: `out` has `room` bytes of spare capacity at `start`, and LAME writes at
        // most `room`; `left` (and `right`) hold `left.len()` samples.
        let written = unsafe { lame_encode_buffer_ieee_float(self.f, left.as_ptr(), right, left.len() as c_int, out.as_mut_ptr().add(start), room as c_int) };
        self.wrote(out, start, written)
    }

    /// Encodes the samples LAME still holds (the song's last frames) and writes them out.
    pub fn flush(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        out.reserve(FLUSH_ROOM);
        let start = out.len();
        // SAFETY: as in `encode`, with FLUSH_ROOM bytes to spare. With no ID3 tag set in
        // LAME, it writes none.
        let written = unsafe { lame_encode_flush(self.f, out.as_mut_ptr().add(start), FLUSH_ROOM as c_int) };
        self.wrote(out, start, written)
    }

    fn wrote(&self, out: &mut Vec<u8>, start: usize, written: c_int) -> Result<(), String> {
        let Ok(n) = usize::try_from(written) else { return Err(format!("the MP3 encoder failed ({written})")) };
        // SAFETY: LAME initialized `n` bytes from `start`, within the reserved space.
        unsafe { out.set_len(start + n) };
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: the state from lame_init, closed once.
        unsafe { lame_close(self.f) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::{codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};

    /// Decoded sample frames in an MP3.
    fn decoded_frames(mp3: Vec<u8>) -> usize {
        let src = MediaSourceStream::new(Box::new(std::io::Cursor::new(mp3)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("mp3");
        let mut format = symphonia::default::get_probe().format(&hint, src, &FormatOptions::default(), &MetadataOptions::default()).unwrap().format;
        let track = format.default_track().unwrap().clone();
        let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).unwrap();
        let mut frames = 0;
        while let Ok(packet) = format.next_packet() {
            frames += dec.decode(&packet).map(|b| b.frames()).unwrap_or(0);
        }
        frames
    }

    #[test]
    fn the_end_of_the_song_is_encoded() {
        let rate = 44_100;
        let tone: Vec<f32> = (0..rate).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect();
        let mut enc = Encoder::new(2, rate as u32, 192).unwrap();
        let mut mp3 = Vec::new();
        for (l, r) in tone.chunks(4096).zip(tone.chunks(4096)) {
            enc.encode(l, Some(r), &mut mp3).unwrap();
        }
        enc.flush(&mut mp3).unwrap();
        // LAME adds its delay at the start; a flush that dropped the last samples comes out short.
        assert!(decoded_frames(mp3) >= tone.len(), "a second in, at least a second out");
    }
}
