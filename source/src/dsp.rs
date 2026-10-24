//! Audio effects applied after librespot decodes and normalizes each packet:
//! the 6-band equalizer and mono downmix. Settings changes apply live.
//!
//! `DspSink` also owns the audio output: it opens it when something first
//! plays on this computer, and again on the new default device when the
//! system output changes.

use crate::config::EQ_BANDS;
use biquad::{Biquad, Coefficients, DirectForm2Transposed, ToHertz, Type};
use librespot_playback::audio_backend::{Sink, SinkResult};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Every song reaches the sink at 44.1 kHz (Glass's librespot resamples local files to it).
const SAMPLE_RATE: f64 = 44_100.0;
const CHANNELS: usize = 2;
const Q: f64 = 1.0;
/// The longest a reopened output waits for the app's pause (see `DspSink::held`).
const HOLD: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq)]
pub struct DspState {
    pub eq_enabled: bool,
    pub gains: [f32; 6],
    pub mono: bool,
}

/// Shared between the UI thread (writes) and the audio thread (reads).
pub struct DspParams {
    state: Mutex<DspState>,
    version: AtomicU64,
}

impl DspParams {
    pub fn new(state: DspState) -> Self {
        Self { state: Mutex::new(state), version: AtomicU64::new(1) }
    }
    pub fn set(&self, s: DspState) {
        let mut cur = self.state.lock().unwrap();
        if *cur != s {
            *cur = s;
            self.version.fetch_add(1, Ordering::Release);
        }
    }
    fn get(&self) -> DspState {
        *self.state.lock().unwrap()
    }
}

type Filter = DirectForm2Transposed<f64>;

/// Opens the output.
pub type Open = Box<dyn Fn() -> SinkResult<Box<dyn Sink>> + Send>;

/// Wraps librespot's rodio sink, opening it the first time something plays and
/// again on the current default output when the system output changes (see
/// `crate::audio_out`) or after the device failed.
pub struct DspSink {
    inner: Option<Box<dyn Sink>>,
    open: Open,
    output_seen: u64,
    started: bool,
    /// Since when the output, reopened because the device playing went away,
    /// has stayed paused: the app is pausing playback, and the new output (the
    /// speakers, usually) waits for the player's next `start`.
    held: Option<Instant>,
    params: std::sync::Arc<DspParams>,
    seen_version: u64,
    state: DspState,
    filters: Vec<[Filter; CHANNELS]>,
    /// Whether the filters ran on the last packet.
    eq_ran: bool,
    preamp: f64,
}

fn coefficients(band: usize, gain_db: f64) -> Coefficients<f64> {
    let f0 = EQ_BANDS[band].0 as f64;
    let kind = match band {
        0 => Type::LowShelf(gain_db),
        5 => Type::HighShelf(gain_db),
        _ => Type::PeakingEQ(gain_db),
    };
    Coefficients::<f64>::from_params(kind, SAMPLE_RATE.hz(), f0.hz(), Q).unwrap_or_else(|_| Coefficients::<f64>::from_params(Type::PeakingEQ(0.0), SAMPLE_RATE.hz(), 1000.0.hz(), Q).unwrap())
}

impl DspSink {
    pub fn new(open: Open, params: std::sync::Arc<DspParams>) -> Self {
        let output_seen = crate::audio_out::generation();
        let mut s = Self {
            inner: None,
            open,
            output_seen,
            started: false,
            held: None,
            params,
            seen_version: 0,
            state: DspState { eq_enabled: false, gains: [0.0; 6], mono: false },
            filters: Vec::new(),
            eq_ran: false,
            preamp: 1.0,
        };
        s.refresh();
        s
    }

    fn refresh(&mut self) {
        let v = self.params.version.load(Ordering::Acquire);
        if v == self.seen_version {
            return;
        }
        self.seen_version = v;
        let st = self.params.get();
        if self.filters.is_empty() {
            self.filters = (0..EQ_BANDS.len())
                .map(|i| {
                    let c = coefficients(i, st.gains[i] as f64);
                    [Filter::new(c), Filter::new(c)]
                })
                .collect();
        } else if st.gains != self.state.gains {
            // New coefficients, same filter state: a band dragged in Settings doesn't click.
            for (i, band) in self.filters.iter_mut().enumerate() {
                let c = coefficients(i, st.gains[i] as f64);
                band.iter_mut().for_each(|f| f.update_coefficients(c));
            }
        }
        self.state = st;
        // A little headroom for boosts; the soft limiter below catches the rest.
        let max_boost = st.gains.iter().cloned().fold(0.0f32, f32::max) as f64;
        self.preamp = if st.eq_enabled { 10f64.powf(-max_boost * 0.25 / 20.0) } else { 1.0 };
    }

    fn process(&mut self, samples: &mut [f64]) {
        let eq = self.state.eq_enabled && self.state.gains.iter().any(|g| g.abs() > 0.01);
        if eq && !self.eq_ran {
            // Running again (turned on, or a preset after Flat): forget whatever
            // was playing when they last ran.
            self.filters.iter_mut().flatten().for_each(Filter::reset_state);
        }
        self.eq_ran = eq;
        // On but flat (the default): nothing to do.
        if !eq && !self.state.mono {
            return;
        }
        for frame in samples.as_chunks_mut::<CHANNELS>().0 {
            if self.state.mono {
                let m = (frame[0] + frame[1]) * 0.5;
                frame[0] = m;
                frame[1] = m;
            }
            if eq {
                for ch in 0..CHANNELS {
                    let mut x = frame[ch] * self.preamp;
                    for band in self.filters.iter_mut() {
                        x = band[ch].run(x);
                    }
                    frame[ch] = soft_limit(x);
                }
            }
        }
    }
}

/// Transparent below 0.8, then eases smoothly towards ±1.0 instead of clipping.
fn soft_limit(x: f64) -> f64 {
    const KNEE: f64 = 0.8;
    let a = x.abs();
    if a <= KNEE { x } else { x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh()) }
}

impl DspSink {
    /// The output to play on: opened on first use, and opened again on the new
    /// default device when the system output changed.
    fn output(&mut self) -> SinkResult<&mut Box<dyn Sink>> {
        let g = crate::audio_out::generation();
        if g != self.output_seen {
            self.output_seen = g;
            self.inner = None;
            self.held = crate::audio_out::went_away(g).then(Instant::now);
        }
        if self.inner.is_none() {
            self.inner = Some((self.open)()?);
        }
        Ok(self.inner.as_mut().expect("opened above"))
    }

    /// Writes to the output, opening it first if needed. A new output (the system's
    /// changed) starts playing at once if the old one was, unless the old one's
    /// device went away (`held`).
    fn write_out(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let fresh = self.inner.is_none() || crate::audio_out::generation() != self.output_seen;
        self.output()?;
        let mut start = fresh && self.started;
        if let Some(at) = self.held {
            if at.elapsed() < HOLD {
                start = false;
                // A paused output takes audio without waiting: keep the player at
                // the pace it would play.
                if let Ok(s) = packet.samples() {
                    std::thread::sleep(Duration::from_secs_f64(s.len() as f64 / (CHANNELS as f64 * SAMPLE_RATE)));
                }
            } else {
                // No pause came: play on.
                self.held = None;
                start = self.started;
            }
        }
        let out = self.inner.as_mut().expect("opened above");
        if start {
            out.start()?;
        }
        out.write(packet, converter)
    }
}

// The player exits if pausing or stopping fails, so those never do; a failed
// start or write drops the output, to be opened afresh next time.
impl Sink for DspSink {
    fn start(&mut self) -> SinkResult<()> {
        self.started = true;
        let r = self.output().and_then(|s| s.start());
        self.held = None;
        if r.is_err() {
            self.inner = None;
            self.started = false;
        }
        r
    }
    fn stop(&mut self) -> SinkResult<()> {
        self.started = false;
        if let Some(s) = self.inner.as_mut() {
            let _ = s.stop();
        }
        Ok(())
    }
    fn pause(&mut self) -> SinkResult<()> {
        self.started = false;
        if let Some(s) = self.inner.as_mut() {
            let _ = s.pause();
        }
        Ok(())
    }
    fn flush(&mut self) -> SinkResult<()> {
        if let Some(s) = self.inner.as_mut() {
            let _ = s.flush();
        }
        Ok(())
    }
    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        self.refresh();
        let packet = match packet {
            AudioPacket::Samples(mut s) => {
                if self.state.eq_enabled || self.state.mono {
                    self.process(&mut s);
                } else {
                    self.eq_ran = false;
                }
                AudioPacket::Samples(s)
            }
            raw => raw,
        };
        let r = self.write_out(packet, converter);
        if r.is_err() {
            self.inner = None;
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    struct Capture(Arc<StdMutex<Vec<f64>>>);
    impl Sink for Capture {
        fn write(&mut self, packet: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            if let AudioPacket::Samples(s) = packet {
                self.0.lock().unwrap().extend(s);
            }
            Ok(())
        }
    }

    /// Records what the player asked of the output.
    struct Calls(Arc<StdMutex<Vec<&'static str>>>);
    impl Sink for Calls {
        fn start(&mut self) -> SinkResult<()> {
            self.0.lock().unwrap().push("start");
            Ok(())
        }
        fn stop(&mut self) -> SinkResult<()> {
            self.0.lock().unwrap().push("stop");
            Ok(())
        }
        fn pause(&mut self) -> SinkResult<()> {
            self.0.lock().unwrap().push("pause");
            Ok(())
        }
        fn flush(&mut self) -> SinkResult<()> {
            self.0.lock().unwrap().push("flush");
            Ok(())
        }
        fn write(&mut self, _: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            Ok(())
        }
    }

    #[test]
    fn pause_and_flush_reach_the_output() {
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let c = calls.clone();
        let mut sink = DspSink::new(Box::new(move || Ok(Box::new(Calls(c.clone())) as Box<dyn Sink>)), Arc::new(DspParams::new(DspState { eq_enabled: false, gains: [0.0; 6], mono: false })));
        sink.start().unwrap();
        sink.pause().unwrap();
        sink.flush().unwrap();
        sink.start().unwrap();
        assert_eq!(*calls.lock().unwrap(), ["start", "pause", "flush", "start"], "pausing doesn't drain, so it's immediate");
    }

    fn run(state: DspState, input: Vec<f64>) -> Vec<f64> {
        let out = Arc::new(StdMutex::new(Vec::new()));
        let o = out.clone();
        let mut sink = DspSink::new(Box::new(move || Ok(Box::new(Capture(o.clone())) as Box<dyn Sink>)), Arc::new(DspParams::new(state)));
        sink.write(AudioPacket::Samples(input), &mut Converter::new(None)).unwrap();

        out.lock().unwrap().clone()
    }

    fn sine(freq: f64, n: usize) -> Vec<f64> {
        (0..n)
            .flat_map(|i| {
                let x = 0.25 * (i as f64 * freq * std::f64::consts::TAU / SAMPLE_RATE).sin();
                [x, x]
            })
            .collect()
    }

    fn rms(v: &[f64]) -> f64 {
        (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt()
    }

    #[test]
    fn disabled_is_bit_exact() {
        let input = sine(440.0, 4096);
        let out = run(DspState { eq_enabled: false, gains: [6.0; 6], mono: false }, input.clone());
        assert_eq!(input, out);
    }

    #[test]
    fn mono_averages_channels() {
        let out = run(DspState { eq_enabled: false, gains: [0.0; 6], mono: true }, vec![1.0, 0.0, 0.2, 0.6]);
        assert_eq!(out, vec![0.5, 0.5, 0.4, 0.4]);
    }

    #[test]
    fn bass_boost_raises_lows_more_than_highs() {
        let boost = DspState { eq_enabled: true, gains: [9.0, 6.0, 0.0, 0.0, 0.0, 0.0], mono: false };
        let low_gain = rms(&run(boost, sine(60.0, 44_100))[8_000..]) / rms(&sine(60.0, 44_100)[8_000..]);
        let high_gain = rms(&run(boost, sine(8_000.0, 44_100))[8_000..]) / rms(&sine(8_000.0, 44_100)[8_000..]);
        // Lows end up clearly louder in absolute terms (> +3 dB) and relative to highs (> +4 dB).
        assert!(low_gain > 1.41, "low {low_gain}");
        assert!(low_gain > high_gain * 1.6, "low {low_gain} high {high_gain}");
    }

    #[test]
    fn eq_starts_fresh_after_flat() {
        let boost = DspState { eq_enabled: true, gains: [9.0, 6.0, 0.0, 0.0, 0.0, 0.0], mono: false };
        let params = Arc::new(DspParams::new(boost));
        let out = Arc::new(StdMutex::new(Vec::new()));
        let o = out.clone();
        let mut sink = DspSink::new(Box::new(move || Ok(Box::new(Capture(o.clone())) as Box<dyn Sink>)), params.clone());
        let mut conv = Converter::new(None);
        sink.write(AudioPacket::Samples(sine(60.0, 4096)), &mut conv).unwrap();
        params.set(DspState { gains: [0.0; 6], ..boost });
        sink.write(AudioPacket::Samples(sine(60.0, 4096)), &mut conv).unwrap();
        params.set(boost);
        out.lock().unwrap().clear();
        sink.write(AudioPacket::Samples(sine(60.0, 4096)), &mut conv).unwrap();
        assert_eq!(*out.lock().unwrap(), run(boost, sine(60.0, 4096)), "no leftovers from before Flat");
    }

    #[test]
    fn eq_output_never_clips() {
        let loud = DspState { eq_enabled: true, gains: [12.0; 6], mono: false };
        let input: Vec<f64> = sine(100.0, 20_000).into_iter().map(|x| x * 3.9).collect();
        assert!(run(loud, input).iter().all(|x| x.abs() <= 1.0));
    }
}
