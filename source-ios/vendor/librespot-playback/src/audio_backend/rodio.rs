use std::process::exit;
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use thiserror::Error;

use super::{Sink, SinkError, SinkResult};
use crate::config::AudioFormat;
use crate::convert::Converter;
use crate::decoder::AudioPacket;
use crate::{NUM_CHANNELS, SAMPLE_RATE};

#[cfg(all(
    feature = "rodiojack-backend",
    not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd"))
))]
compile_error!("Rodio JACK backend is currently only supported on linux.");

#[cfg(feature = "rodio-backend")]
pub fn mk_rodio(device: Option<String>, format: AudioFormat) -> Box<dyn Sink> {
    Box::new(open(cpal::default_host(), device, format))
}

#[cfg(feature = "rodiojack-backend")]
pub fn mk_rodiojack(device: Option<String>, format: AudioFormat) -> Box<dyn Sink> {
    Box::new(open(
        cpal::host_from_id(cpal::HostId::Jack).unwrap(),
        device,
        format,
    ))
}

#[derive(Debug, Error)]
pub enum RodioError {
    #[error("<RodioSink> No Device Available")]
    NoDeviceAvailable,
    #[error("<RodioSink> device \"{0}\" is Not Available")]
    DeviceNotAvailable(String),
    #[error("<RodioSink> Play Error: {0}")]
    PlayError(#[from] rodio::PlayError),
    #[error("<RodioSink> Stream Error: {0}")]
    StreamError(#[from] rodio::StreamError),
    #[error("<RodioSink> Cannot Get Audio Devices: {0}")]
    DevicesError(#[from] cpal::DevicesError),
    #[error("<RodioSink> {0}")]
    Samples(String),
    #[error("<RodioSink> Output Error: {0}")]
    Output(String),
}

impl From<RodioError> for SinkError {
    fn from(e: RodioError) -> SinkError {
        use RodioError::*;
        let es = e.to_string();
        match e {
            StreamError(_) | PlayError(_) | Samples(_) | Output(_) => SinkError::OnWrite(es),
            NoDeviceAvailable | DeviceNotAvailable(_) => SinkError::ConnectionRefused(es),
            DevicesError(_) => SinkError::InvalidParams(es),
        }
    }
}

impl From<cpal::DefaultStreamConfigError> for RodioError {
    fn from(_: cpal::DefaultStreamConfigError) -> RodioError {
        RodioError::NoDeviceAvailable
    }
}

impl From<cpal::SupportedStreamConfigsError> for RodioError {
    fn from(_: cpal::SupportedStreamConfigsError) -> RodioError {
        RodioError::NoDeviceAvailable
    }
}

// Glass: the output stream is built here, around rodio's mixer, rather than by
// `rodio::OutputStream`, so that it can be paused. A paused stream makes no
// audio callbacks at all (an `OutputStream` keeps pulling silence from the
// moment it opens until it's dropped), and the mixer keeps what's queued, so
// playback resumes from exactly where it stopped.
pub struct RodioSink {
    rodio_sink: rodio::Sink,
    mixer: rodio::mixer::Mixer,
    stream: cpal::Stream,
    running: bool,
}

fn list_formats(device: &cpal::Device) {
    match device.default_output_config() {
        Ok(cfg) => {
            debug!("  Default config:");
            debug!("    {cfg:?}");
        }
        Err(e) => {
            // Use loglevel debug, since even the output is only debug
            debug!("Error getting default rodio::Sink config: {e}");
        }
    };

    match device.supported_output_configs() {
        Ok(mut cfgs) => {
            if let Some(first) = cfgs.next() {
                debug!("  Available configs:");
                debug!("    {first:?}");
            } else {
                return;
            }

            for cfg in cfgs {
                debug!("    {cfg:?}");
            }
        }
        Err(e) => {
            debug!("Error getting supported rodio::Sink configs: {e}");
        }
    }
}

fn list_outputs(host: &cpal::Host) -> Result<(), cpal::DevicesError> {
    let mut default_device_name = None;

    if let Some(default_device) = host.default_output_device() {
        default_device_name = default_device.name().ok();
        println!(
            "Default Audio Device:\n  {}",
            default_device_name.as_deref().unwrap_or("[unknown name]")
        );

        list_formats(&default_device);

        println!("Other Available Audio Devices:");
    } else {
        warn!("No default device was found");
    }

    for device in host.output_devices()? {
        match device.name() {
            Ok(name) if Some(&name) == default_device_name.as_ref() => (),
            Ok(name) => {
                println!("  {name}");
                list_formats(&device);
            }
            Err(e) => {
                warn!("Cannot get device name: {e}");
                println!("   [unknown name]");
                list_formats(&device);
            }
        }
    }

    Ok(())
}

fn on_stream_error(e: cpal::StreamError) {
    warn!("audio output: {e}");
}

// Glass: feeds the device in the format `create_sink` chose: f32, i16, i32 or f64
// (f32 on macOS).
fn build_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut source: rodio::mixer::MixerSource,
) -> Result<cpal::Stream, RodioError> {
    use cpal::Sample;
    let cfg = config.config();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            &cfg,
            move |d, _| d.iter_mut().for_each(|s| *s = source.next().unwrap_or(0.0)),
            on_stream_error,
            None,
        ),
        cpal::SampleFormat::F64 => device.build_output_stream::<f64, _, _>(
            &cfg,
            move |d, _| {
                d.iter_mut()
                    .for_each(|s| *s = source.next().map(f64::from_sample).unwrap_or(0.0))
            },
            on_stream_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
            &cfg,
            move |d, _| {
                d.iter_mut()
                    .for_each(|s| *s = source.next().map(i16::from_sample).unwrap_or(0))
            },
            on_stream_error,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_output_stream::<i32, _, _>(
            &cfg,
            move |d, _| {
                d.iter_mut()
                    .for_each(|s| *s = source.next().map(i32::from_sample).unwrap_or(0))
            },
            on_stream_error,
            None,
        ),
        other => return Err(RodioError::Output(format!("unsupported sample format {other}"))),
    };
    stream.map_err(|e| RodioError::Output(e.to_string()))
}

// Glass: stereo in a format `build_stream` feeds (f32 first, what rodio mixes in;
// ALSA lists 8-bit formats first), at 44.1 kHz or else the device's default rate
// (rodio resamples), then whatever the default device config is (like mono).
fn choose_config(
    offered: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
    default_config: cpal::SupportedStreamConfig,
) -> cpal::SupportedStreamConfig {
    use cpal::SampleFormat::{F32, F64, I16, I32};
    let fed = |f: cpal::SampleFormat| [F32, I16, I32, F64].iter().position(|x| *x == f);
    let mut stereo: Vec<_> = offered
        .filter(|c| {
            c.channels() == NUM_CHANNELS as cpal::ChannelCount && fed(c.sample_format()).is_some()
        })
        .collect();
    stereo.sort_by_key(|c| fed(c.sample_format()));
    stereo
        .into_iter()
        .find_map(|c| {
            c.try_with_sample_rate(cpal::SampleRate(SAMPLE_RATE))
                .or_else(|| c.try_with_sample_rate(default_config.sample_rate()))
        })
        .unwrap_or(default_config)
}

fn create_sink(
    host: &cpal::Host,
    device: Option<String>,
    format: AudioFormat,
) -> Result<(rodio::Sink, rodio::mixer::Mixer, cpal::Stream), RodioError> {
    let cpal_device = match device.as_deref() {
        Some("?") => match list_outputs(host) {
            Ok(()) => exit(0),
            Err(e) => {
                error!("{e}");
                exit(1);
            }
        },
        Some(device_name) => {
            // Ignore devices for which getting name fails, or format doesn't match
            host.output_devices()?
                .find(|d| d.name().ok().is_some_and(|name| name == device_name)) // Ignore devices for which getting name fails
                .ok_or_else(|| RodioError::DeviceNotAvailable(device_name.to_string()))?
        }
        None => host
            .default_output_device()
            .ok_or(RodioError::NoDeviceAvailable)?,
    };

    let name = cpal_device.name().ok();
    info!(
        "Using audio device: {}",
        name.as_deref().unwrap_or("[unknown name]")
    );

    let default_config = cpal_device.default_output_config()?;
    let config = choose_config(cpal_device.supported_output_configs()?, default_config);

    // Glass: the format comes from the device (see `build_stream`); the
    // stream starts paused, so nothing runs until something plays.
    let _ = format;
    let (mixer, source) = rodio::mixer::mixer(config.channels(), config.sample_rate().0);
    let stream = build_stream(&cpal_device, &config, source)?;
    let _ = stream.pause();
    let sink = rodio::Sink::connect_new(&mixer);
    Ok((sink, mixer, stream))
}

/// Glass: opens the output, or says why it can't (rather than panicking the
/// player thread when there's no usable device).
pub fn try_open(host: cpal::Host, device: Option<String>, format: AudioFormat) -> Result<RodioSink, RodioError> {
    info!("Using Rodio sink with cpal host: {}", host.id().name());
    let (rodio_sink, mixer, stream) = create_sink(&host, device, format)?;
    debug!("Rodio sink was created");
    Ok(RodioSink { rodio_sink, mixer, stream, running: false })
}

pub fn open(host: cpal::Host, device: Option<String>, format: AudioFormat) -> RodioSink {
    try_open(host, device, format).unwrap()
}

/// Glass: the default output, as a boxed sink.
pub fn try_open_default() -> Result<Box<dyn Sink>, SinkError> {
    Ok(Box::new(try_open(cpal::default_host(), None, AudioFormat::F32)?))
}

impl RodioSink {
    /// Glass: lets the device pull audio again.
    fn run(&mut self) -> SinkResult<()> {
        if !self.running {
            self.stream.play().map_err(|e| SinkError::OnWrite(e.to_string()))?;
            self.running = true;
        }
        Ok(())
    }

    /// Glass: no more audio callbacks until `run`.
    fn halt(&mut self) {
        if self.running {
            let _ = self.stream.pause();
            self.running = false;
        }
    }
}

// Glass: pause, flush and stop never fail (the player exits if pause or stop
// does); start and write can, and the player then pauses. Nothing waits more
// than 2 s on an output that has stopped.
impl Sink for RodioSink {
    fn start(&mut self) -> SinkResult<()> {
        self.run()?;
        self.rodio_sink.play();
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        // What's queued (the end of the last song) plays out first, but only for
        // as long as it should take.
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.running && !self.rodio_sink.empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if !self.rodio_sink.empty() {
            self.flush()?;
        }
        self.rodio_sink.pause();
        self.halt();
        Ok(())
    }

    // Glass: pausing is immediate and stops the device's callbacks; what's
    // queued resumes with `start`.
    fn pause(&mut self) -> SinkResult<()> {
        self.rodio_sink.pause();
        self.halt();
        Ok(())
    }

    // Glass: a fresh queue on the same stream. Dropping the old sink silences
    // it at once (`Sink::clear` would wait for the mixer, and forever if the
    // device just went away).
    fn flush(&mut self) -> SinkResult<()> {
        let paused = self.rodio_sink.is_paused();
        self.rodio_sink = rodio::Sink::connect_new(&self.mixer);
        if paused {
            self.rodio_sink.pause();
        }
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet
            .samples()
            .map_err(|e| RodioError::Samples(e.to_string()))?;
        let samples_f32: &[f32] = &converter.f64_to_f32(samples);
        let source = rodio::buffer::SamplesBuffer::new(
            NUM_CHANNELS as cpal::ChannelCount,
            SAMPLE_RATE,
            samples_f32,
        );
        // Glass: the visualizer sees each sample as it's played.
        self.rodio_sink.append(crate::tap::tapped(source));

        // Chunk sizes seem to be about 256 to 3000 ish items long.
        // Assuming they're on average 1628 then a half second buffer is:
        // 44100 elements --> about 27 chunks
        // Glass: only while the device is taking audio, and an output that
        // stops taking it (its device went away) is an error, not a hang.
        let mut last = (self.rodio_sink.len(), Instant::now());
        while self.running && self.rodio_sink.len() > 26 {
            thread::sleep(Duration::from_millis(10));
            let len = self.rodio_sink.len();
            if len < last.0 {
                last = (len, Instant::now());
            } else if last.1.elapsed() > Duration::from_secs(2) {
                return Err(RodioError::Output("the audio output stopped taking audio".into()).into());
            }
        }
        Ok(())
    }
}

impl RodioSink {
    #[allow(dead_code)]
    pub const NAME: &'static str = "rodio";
}

#[cfg(test)]
mod glass_tests {
    use super::*;
    use crate::convert::Converter;
    use crate::decoder::AudioPacket;
    use std::time::Instant;
    use cpal::{SampleFormat, SampleRate, SupportedBufferSize, SupportedStreamConfig, SupportedStreamConfigRange};

    /// Glass: the default output opens and takes audio. On Linux, ALSA's default
    /// device lists 8-bit formats first, which `build_stream` can't feed:
    /// `cargo test -p librespot-playback --lib glass_default_output -- --ignored`.
    #[test]
    #[ignore]
    fn glass_default_output_opens() {
        let mut sink = open(cpal::default_host(), None, AudioFormat::F32);
        let mut conv = Converter::new(None);
        sink.start().unwrap();
        sink.write(AudioPacket::Samples(vec![0.0; 4096]), &mut conv).unwrap();
        sink.stop().unwrap();
    }

    /// Glass: pause and flush return at once; stop waits for the queue.
    /// Plays silence on the default output device:
    /// `cargo test -p librespot-playback --lib glass_sink -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn glass_sink_controls_are_immediate() {
        let mut sink = open(cpal::default_host(), None, AudioFormat::F32);
        let mut conv = Converter::new(None);
        let fill = |sink: &mut RodioSink, conv: &mut Converter| {
            for _ in 0..26 {
                sink.write(AudioPacket::Samples(vec![0.0; 1628]), conv).unwrap();
            }
        };
        sink.start().unwrap();
        fill(&mut sink, &mut conv);
        let t = Instant::now();
        sink.pause().unwrap();
        let pause = t.elapsed();
        assert!(sink.rodio_sink.is_paused() && sink.rodio_sink.len() > 0, "paused, queue kept for resuming");
        let t = Instant::now();
        sink.flush().unwrap();
        let flush = t.elapsed();
        assert_eq!(sink.rodio_sink.len(), 0, "queue dropped");
        assert!(sink.rodio_sink.is_paused(), "still paused after a flush");
        sink.start().unwrap();
        fill(&mut sink, &mut conv);
        let t = Instant::now();
        sink.stop().unwrap();
        let stop = t.elapsed();
        println!("pause {pause:?}, flush {flush:?}, stop (drains) {stop:?}");
        // Pausing stops the device's callbacks, which waits out one audio cycle (tens of ms);
        // it never waits for the queue.
        assert!(pause.as_millis() < 100 && flush.as_millis() < 20, "{pause:?} {flush:?}");
        assert!(stop.as_millis() > 100, "stop still plays the queue out: {stop:?}");
    }

    fn range(channels: u16, min: u32, max: u32, format: SampleFormat) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(channels, SampleRate(min), SampleRate(max), SupportedBufferSize::Unknown, format)
    }

    /// What ALSA's `default` device (plug, PulseAudio or PipeWire) offers, in cpal's
    /// order: every format for each channel count, 8-bit first.
    fn alsa_default() -> Vec<SupportedStreamConfigRange> {
        use SampleFormat::*;
        [I8, U8, I16, U16, I32, U32, F32, F64]
            .into_iter()
            .flat_map(|f| [1, 2].into_iter().map(move |ch| range(ch, 4000, 192_000, f)))
            .collect()
    }

    #[test]
    fn glass_picks_a_format_it_can_feed() {
        let default = SupportedStreamConfig::new(2, SampleRate(48_000), SupportedBufferSize::Unknown, SampleFormat::I16);
        let c = choose_config(alsa_default().into_iter(), default);
        assert_eq!((c.channels(), c.sample_rate(), c.sample_format()), (2, SampleRate(44_100), SampleFormat::F32), "stereo f32 at 44.1 kHz, not ALSA's first (8-bit)");
    }

    #[test]
    fn glass_falls_back_in_order() {
        use SampleFormat::*;
        let default = SupportedStreamConfig::new(1, SampleRate(48_000), SupportedBufferSize::Unknown, I16);
        // A device without f32 and without 44.1 kHz: 16-bit at its default rate.
        let c = choose_config([range(2, 48_000, 48_000, U8), range(2, 48_000, 48_000, I16)].into_iter(), default.clone());
        assert_eq!((c.sample_rate(), c.sample_format()), (SampleRate(48_000), I16));
        // Nothing usable in stereo: the device's default.
        let c = choose_config([range(2, 44_100, 48_000, U8)].into_iter(), default.clone());
        assert_eq!((c.channels(), c.sample_format()), (1, I16));
    }
}
