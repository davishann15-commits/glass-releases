use std::{io, time::Duration};

use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, SeekMode, SeekTo},
    io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions},
    meta::{MetadataOptions, StandardTagKey, Value},
    probe::{Hint, ProbeResult},
};

use super::{AudioDecoder, AudioPacket, AudioPacketPosition, DecoderError, DecoderResult};

use crate::{PAGES_PER_MS, SAMPLE_RATE, player::NormalisationData, symphonia_util};

pub struct SymphoniaDecoder {
    probe_result: ProbeResult,
    decoder: Box<dyn Decoder>,
    sample_buffer: Option<SampleBuffer<f64>>,
    /// Glass: set when the source isn't at the player's sample rate.
    resampler: Option<Resampler>,
}

/// Glass: converts interleaved stereo from one sample rate to another with
/// cubic (Catmull-Rom) interpolation, carrying state across packets. Upstream
/// refused every local file that wasn't exactly 44.1 kHz stereo.
struct Resampler {
    /// Input frames per output frame.
    step: f64,
    /// Read position in `hist` + the next packet's frames.
    pos: f64,
    /// The last input frames of the previous packet.
    hist: Vec<[f64; 2]>,
    /// Smooth before interpolating when shrinking a lot (e.g. 96 kHz).
    smooth: bool,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        let step = from as f64 / to as f64;
        Self { step, pos: 1.0, hist: vec![[0.0; 2]], smooth: step > 1.5 }
    }

    fn reset(&mut self) {
        self.pos = 1.0;
        self.hist = vec![[0.0; 2]];
    }

    fn process(&mut self, stereo: &[f64]) -> Vec<f64> {
        let mut frames = std::mem::take(&mut self.hist);
        frames.extend(stereo.chunks_exact(2).map(|c| [c[0], c[1]]));
        let src: Vec<[f64; 2]> = if self.smooth {
            (0..frames.len())
                .map(|i| {
                    let a = frames[i.saturating_sub(1)];
                    let b = frames[i];
                    let c = frames[(i + 1).min(frames.len() - 1)];
                    [0.25 * a[0] + 0.5 * b[0] + 0.25 * c[0], 0.25 * a[1] + 0.5 * b[1] + 0.25 * c[1]]
                })
                .collect()
        } else {
            frames.clone()
        };
        let mut out = Vec::with_capacity(((stereo.len() / 2) as f64 / self.step) as usize * 2 + 4);
        while (self.pos as usize) + 2 < src.len() {
            let i = self.pos as usize;
            let t = self.pos - i as f64;
            for ch in 0..2 {
                let (p0, p1, p2, p3) = (src[i - 1][ch], src[i][ch], src[i + 1][ch], src[i + 2][ch]);
                let v = p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
                out.push(v.clamp(-1.0, 1.0));
            }
            self.pos += self.step;
        }
        // Keep the last three frames for the next packet.
        let keep = frames.len().min(3);
        let drop = frames.len() - keep;
        self.hist = frames.split_off(drop);
        self.pos -= drop as f64;
        out
    }
}

/// Glass: any channel count as interleaved stereo (mono doubled, extra
/// channels after front left/right dropped).
fn to_stereo(samples: &[f64], channels: usize) -> Vec<f64> {
    match channels {
        2 => samples.to_vec(),
        1 => samples.iter().flat_map(|s| [*s, *s]).collect(),
        n => samples.chunks_exact(n.max(1)).flat_map(|f| [f[0], f.get(1).copied().unwrap_or(f[0])]).collect(),
    }
}

#[derive(Default)]
pub(crate) struct LocalFileMetadata {
    pub name: Option<String>,
    pub language: Option<String>,
    pub album: Option<String>,
    pub artists: Option<String>,
    pub album_artists: Option<String>,
    pub number: Option<u32>,
    pub disc_number: Option<u32>,
}

impl SymphoniaDecoder {
    pub fn new<R>(input: R, hint: Hint) -> DecoderResult<Self>
    where
        R: MediaSource + 'static,
    {
        let mss_opts = MediaSourceStreamOptions {
            buffer_len: librespot_audio::AudioFetchParams::get().minimum_download_size,
        };
        let mss = MediaSourceStream::new(Box::new(input), mss_opts);

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();

        let probe_result =
            symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        let format = &probe_result.format;

        let track = format.default_track().ok_or_else(|| {
            DecoderError::SymphoniaDecoder("Could not retrieve default track".into())
        })?;

        let decoder_opts: DecoderOptions = Default::default();

        let decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;

        let rate = decoder.codec_params().sample_rate.ok_or_else(|| {
            DecoderError::SymphoniaDecoder("Could not retrieve sample rate".into())
        })?;

        // Glass: other rates are resampled and other channel counts made
        // stereo, instead of refusing the file.
        let resampler = (rate != SAMPLE_RATE).then(|| Resampler::new(rate, SAMPLE_RATE));

        Ok(Self {
            probe_result,
            decoder,
            // We set the sample buffer when decoding the first full packet,
            // whose duration is also the ideal sample buffer size.
            sample_buffer: None,
            resampler,
        })
    }

    pub fn normalisation_data(&mut self) -> Option<NormalisationData> {
        let metadata = symphonia_util::get_latest_metadata(&mut self.probe_result)?;
        let tags = metadata.current()?.tags();

        if tags.is_empty() {
            None
        } else {
            let mut data = NormalisationData::default();

            for tag in tags {
                if let Value::Float(value) = tag.value {
                    match tag.std_key {
                        Some(StandardTagKey::ReplayGainAlbumGain) => data.album_gain_db = value,
                        Some(StandardTagKey::ReplayGainAlbumPeak) => data.album_peak = value,
                        Some(StandardTagKey::ReplayGainTrackGain) => data.track_gain_db = value,
                        Some(StandardTagKey::ReplayGainTrackPeak) => data.track_peak = value,
                        _ => (),
                    }
                }
            }

            Some(data)
        }
    }

    pub(crate) fn local_file_metadata(&mut self) -> Option<LocalFileMetadata> {
        let metadata = symphonia_util::get_latest_metadata(&mut self.probe_result)?;
        let tags = metadata.current()?.tags();
        let mut metadata = LocalFileMetadata::default();

        for tag in tags {
            if let Value::String(value) = &tag.value {
                match tag.std_key {
                    // We could possibly use mem::take here to avoid cloning, but that risks leaving
                    // the audio item metadata in a bad state.
                    Some(StandardTagKey::TrackTitle) => metadata.name = Some(value.clone()),
                    Some(StandardTagKey::Language) => metadata.language = Some(value.clone()),
                    Some(StandardTagKey::Artist) => metadata.artists = Some(value.clone()),
                    Some(StandardTagKey::AlbumArtist) => {
                        metadata.album_artists = Some(value.clone())
                    }
                    Some(StandardTagKey::Album) => metadata.album = Some(value.clone()),
                    Some(StandardTagKey::TrackNumber) => {
                        metadata.number = match value.parse::<u32>() {
                            Ok(value) => Some(value),
                            Err(e) => {
                                warn!(
                                    "Failed to parse local file's track number tag '{value}': {e}"
                                );
                                None
                            }
                        }
                    }
                    Some(StandardTagKey::DiscNumber) => {
                        metadata.disc_number = match value.parse::<u32>() {
                            Ok(value) => Some(value),
                            Err(e) => {
                                warn!(
                                    "Failed to parse local file's disc number tag '{value}': {e}"
                                );
                                None
                            }
                        }
                    }
                    _ => (),
                }
            } else if let Value::UnsignedInt(value) = &tag.value {
                match tag.std_key {
                    Some(StandardTagKey::TrackNumber) => metadata.number = Some(*value as u32),
                    Some(StandardTagKey::DiscNumber) => metadata.disc_number = Some(*value as u32),
                    _ => (),
                }
            } else if let Value::SignedInt(value) = &tag.value {
                match tag.std_key {
                    Some(StandardTagKey::TrackNumber) => metadata.number = Some(*value as u32),
                    Some(StandardTagKey::DiscNumber) => metadata.disc_number = Some(*value as u32),
                    _ => (),
                }
            }
        }

        Some(metadata)
    }

    #[inline]
    fn ts_to_ms(&self, ts: u64) -> u32 {
        match self.decoder.codec_params().time_base {
            Some(time_base) => {
                let time = Duration::from(time_base.calc_time(ts));
                time.as_millis() as u32
            }
            // Fallback in the unexpected case that the format has no base time set.
            None => (ts as f64 * PAGES_PER_MS) as u32,
        }
    }
}

impl AudioDecoder for SymphoniaDecoder {
    fn seek(&mut self, position_ms: u32) -> Result<u32, DecoderError> {
        // "Saturate" the position_ms to the duration of the track if it exceeds it.
        let mut target = Duration::from_millis(position_ms.into());
        let codec_params = self.decoder.codec_params();
        if let (Some(time_base), Some(n_frames)) = (codec_params.time_base, codec_params.n_frames) {
            let duration = Duration::from(time_base.calc_time(n_frames));
            if target > duration {
                target = duration;
            }
        }

        // `track_id: None` implies the default track ID (of the container, not of Spotify).
        let seeked_to_ts = self.probe_result.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time: target.into(),
                track_id: None,
            },
        )?;

        // Seeking is a `FormatReader` operation, so the decoder cannot reliably
        // know when a seek took place. Reset it to avoid audio glitches.
        self.decoder.reset();
        if let Some(r) = self.resampler.as_mut() {
            r.reset();
        }

        Ok(self.ts_to_ms(seeked_to_ts.actual_ts))
    }

    fn next_packet(&mut self) -> DecoderResult<Option<(AudioPacketPosition, AudioPacket)>> {
        let mut skipped = false;

        loop {
            let packet = match self.probe_result.format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(err)) => {
                    if err.kind() == io::ErrorKind::UnexpectedEof {
                        return Ok(None);
                    } else {
                        return Err(DecoderError::SymphoniaDecoder(err.to_string()));
                    }
                }
                Err(err) => {
                    return Err(err.into());
                }
            };

            let position_ms = self.ts_to_ms(packet.ts());
            let packet_position = AudioPacketPosition {
                position_ms,
                skipped,
            };

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let sample_buffer = match self.sample_buffer.as_mut() {
                        Some(buffer) => buffer,
                        None => {
                            let spec = *decoded.spec();
                            let duration = decoded.capacity() as u64;
                            self.sample_buffer.insert(SampleBuffer::new(duration, spec))
                        }
                    };

                    // The packet's own layout is authoritative (AAC often
                    // doesn't report channels until it decodes).
                    let channels = decoded.spec().channels.count().max(1);
                    sample_buffer.copy_interleaved_ref(decoded);
                    let stereo = to_stereo(sample_buffer.samples(), channels);
                    let samples = AudioPacket::Samples(match self.resampler.as_mut() {
                        Some(r) => r.process(&stereo),
                        None => stereo,
                    });

                    return Ok(Some((packet_position, samples)));
                }
                Err(Error::DecodeError(_)) => {
                    // The packet failed to decode due to corrupted or invalid data, get a new
                    // packet and try again.
                    warn!("Skipping malformed audio packet at {position_ms} ms");
                    skipped = true;
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}

#[cfg(test)]
mod glass_tests {
    use super::*;

    #[test]
    fn mono_and_surround_become_stereo() {
        assert_eq!(to_stereo(&[0.1, 0.2], 1), vec![0.1, 0.1, 0.2, 0.2]);
        assert_eq!(to_stereo(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6], 3), vec![0.1, 0.2, 0.4, 0.5]);
    }

    #[test]
    fn resampling_keeps_length_pitch_and_continuity() {
        // One second of a 1 kHz tone at 48 kHz, fed in uneven packets.
        let tone: Vec<f64> = (0..48_000).flat_map(|i| {
            let v = (i as f64 / 48_000.0 * 1000.0 * std::f64::consts::TAU).sin() * 0.5;
            [v, v]
        }).collect();
        let mut r = Resampler::new(48_000, 44_100);
        let mut out = Vec::new();
        for chunk in tone.chunks(2 * 1151) {
            out.extend(r.process(chunk));
        }
        let frames = out.len() / 2;
        assert!((44_090..=44_100).contains(&frames), "{frames} frames for one second");
        // Still 1 kHz: count upward zero crossings on the left channel.
        let left: Vec<f64> = out.chunks(2).map(|c| c[0]).collect();
        let crossings = left.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((998..=1001).contains(&crossings), "{crossings} cycles");
        // No jumps at packet boundaries: neighbouring samples stay close.
        let max_step = left.windows(2).skip(4).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        assert!(max_step < 0.08, "{max_step}");
    }
}

/// Glass: decodes a real file end to end (opt-in: set GLASS_TEST_AUDIO to an
/// audio file, e.g. a 48 kHz mono AAC .m4a).
#[cfg(test)]
mod glass_file_tests {
    use super::*;

    #[test]
    fn decodes_a_real_file_to_44k_stereo() {
        let Ok(path) = std::env::var("GLASS_TEST_AUDIO") else { return };
        let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
        let mut hint = Hint::new();
        hint.with_extension(&ext);
        let mut d = SymphoniaDecoder::new(std::fs::File::open(&path).unwrap(), hint).expect("opens");
        let mut frames = 0usize;
        while let Some((_, packet)) = d.next_packet().expect("decodes") {
            let s = packet.samples().unwrap();
            assert_eq!(s.len() % 2, 0, "stereo");
            frames += s.len() / 2;
        }
        let secs = frames as f64 / SAMPLE_RATE as f64;
        println!("{path}: {frames} frames at 44.1 kHz = {secs:.3} s");
        assert!(secs > 0.5);
    }
}
