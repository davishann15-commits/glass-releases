//! AAC (in a plain MP4) to MP3: symphonia decodes, LAME encodes (see `lame`), one
//! packet at a time, so memory stays flat whatever the song's length.

use super::lame;
use anyhow::{Context, Result, anyhow, bail};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const BITRATES: [u16; 5] = [128, 160, 192, 256, 320];

/// The nearest of `BITRATES` at or above `kbps` (320 at most).
fn bitrate(kbps: u16) -> u16 {
    BITRATES.iter().copied().find(|&b| b >= kbps).unwrap_or(320)
}

pub fn encode(src: &Path, dst: &Path, kbps: u16) -> Result<()> {
    let stream = MediaSourceStream::new(Box::new(File::open(src)?), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let mut format = symphonia::default::get_probe().format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default()).context("reading the downloaded audio")?.format;
    let track = format.default_track().context("no audio track")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.context("unknown sample rate")?;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2).clamp(1, 2);
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).context("decoding the downloaded audio")?;

    let mut encoder = lame::Encoder::new(channels as u8, rate, bitrate(kbps)).map_err(|e| anyhow!(e))?;

    let mut out = BufWriter::new(File::create(dst)?);
    let mut pcm: Option<SampleBuffer<f32>> = None;
    let mut mp3 = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(DecodeError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged frame: skip it, like players do.
            Err(DecodeError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let frames = decoded.frames();
        if frames == 0 {
            continue;
        }
        let buf = pcm.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, *decoded.spec()));
        if buf.capacity() < decoded.capacity() * decoded.spec().channels.count() {
            *buf = SampleBuffer::new(decoded.capacity() as u64, *decoded.spec());
        }
        buf.copy_planar_ref(decoded);
        let samples = buf.samples();
        mp3.clear();
        let left = &samples[..frames];
        let right = (channels == 2 && samples.len() >= 2 * frames).then(|| &samples[frames..2 * frames]);
        encoder.encode(left, right, &mut mp3).map_err(|e| anyhow!(e))?;
        out.write_all(&mp3)?;
    }
    mp3.clear();
    encoder.flush(&mut mp3).map_err(|e| anyhow!(e))?;
    out.write_all(&mp3)?;
    out.flush()?;
    if out.get_ref().metadata()?.len() < 1024 {
        bail!("the converted MP3 came out empty");
    }
    Ok(())
}
