//! The audio visualizer's analysis. The tap in librespot-playback hands over
//! every sample as it's played (Spotify through the playback engine and
//! local files alike); the last 2048 are kept here, and a spectrum is worked
//! out from them on demand, only while the visualizer is on screen.
//!
//! The audio thread never waits (if the buffer is busy, that block is skipped)
//! and never allocates; the analysis reuses its buffers from frame to frame.

use std::sync::Mutex;
use std::time::{Duration, Instant};

const N: usize = 2048;

struct Ring {
    data: [f32; N],
    pos: usize,
    rate: u32,
    last: Option<Instant>,
}

static RING: Mutex<Ring> = Mutex::new(Ring { data: [0.0; N], pos: 0, rate: 44_100, last: None });

/// Starts receiving played audio. Call once at startup.
pub fn install() {
    librespot_playback::tap::set(Box::new(push));
}

/// Demo mode only: something for the visualizer to show without real audio.
pub fn demo_signal() {
    std::thread::spawn(|| {
        let rate = 44_100u32;
        let mut t = 0u64;
        loop {
            let block: Vec<f32> = (0..1024)
                .map(|i| {
                    let s = (t + i) as f32 / rate as f32;
                    let beat = (1.0 - (s * 2.0).fract()).powi(3);
                    let notes = [55.0, 110.0, 220.0, 330.0, 440.0, 660.0, 880.0, 1320.0, 2640.0, 5280.0];
                    let mut v = beat * 0.5 * (2.0 * std::f32::consts::PI * 55.0 * s).sin();
                    for (k, f) in notes.iter().enumerate() {
                        let wobble = 0.5 + 0.5 * (s * (0.3 + k as f32 * 0.17)).sin();
                        v += 0.12 * wobble * (2.0 * std::f32::consts::PI * f * s).sin() / (1.0 + k as f32 * 0.25);
                    }
                    v
                })
                .collect();
            t += 1024;
            push(&block, 1, rate);
            std::thread::sleep(std::time::Duration::from_micros(1024 * 1_000_000 / rate as u64));
        }
    });
}

fn push(samples: &[f32], channels: u16, rate: u32) {
    let Ok(mut r) = RING.try_lock() else { return };
    r.rate = rate.max(1);
    let ch = channels.max(1) as usize;
    for frame in samples.chunks(ch) {
        let m = frame.iter().sum::<f32>() / frame.len() as f32;
        let p = r.pos;
        r.data[p] = m;
        r.pos = (p + 1) % N;
    }
    r.last = Some(Instant::now());
}

/// Copies the latest samples into `out` (`N` of them, oldest first) and gives
/// their rate, or `None` when nothing has been played here lately (paused, or
/// playing on another device).
fn latest_into(out: &mut [f32]) -> Option<u32> {
    let r = RING.lock().ok()?;
    if r.last.is_none_or(|t| t.elapsed() > Duration::from_millis(200)) {
        return None;
    }
    let (old, new) = r.data.split_at(r.pos);
    out[..new.len()].copy_from_slice(new);
    out[new.len()..].copy_from_slice(old);
    Some(r.rate)
}

/// In-place radix-2 FFT (N is a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
                let t = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = t;
            }
        }
        len <<= 1;
    }
}

/// Turns the latest audio into smoothed band levels (0–1) for drawing.
pub struct Analyzer {
    window: Vec<f32>,
    /// Working space, kept between frames: the samples (then the spectrum's real
    /// part), its imaginary part, the magnitudes and the bands' new levels.
    re: Vec<f32>,
    im: Vec<f32>,
    mags: Vec<f32>,
    target: Vec<f32>,
    pub bands: Vec<f32>,
    pub peaks: Vec<f32>,
    /// The loudest recent level, for automatic gain (so volume doesn't matter).
    gain_ref: f32,
    last: Option<Instant>,
    /// Audio is arriving (something plays on this computer).
    pub live: bool,
}

impl Default for Analyzer {
    fn default() -> Self {
        let window = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N - 1) as f32).cos()).collect();
        Self { window, re: vec![0.0; N], im: vec![0.0; N], mags: vec![0.0; N / 2], target: Vec::new(), bands: Vec::new(), peaks: Vec::new(), gain_ref: 1e-3, last: None, live: false }
    }
}

impl Analyzer {
    /// One step: `n` bands, `sensitivity` (1 = normal), `smoothing` 0–0.95.
    pub fn update(&mut self, n: usize, sensitivity: f32, smoothing: f32) {
        let now = Instant::now();
        let dt = self.last.map(|t| (now - t).as_secs_f32()).unwrap_or(1.0 / 60.0).clamp(0.001, 0.1);
        self.last = Some(now);
        if self.bands.len() != n {
            self.bands = vec![0.0; n];
            self.peaks = vec![0.0; n];
        }
        match latest_into(&mut self.re) {
            Some(rate) => {
                self.live = true;
                self.analyze(rate, n, sensitivity, dt);
            }
            None => {
                self.live = false;
                self.target.clear();
                self.target.resize(n, 0.0);
            }
        }
        let target = std::mem::take(&mut self.target);
        self.advance(&target, smoothing, dt);
        self.target = target;
    }

    /// Moves the bands towards `target`: up quickly, down at the chosen
    /// smoothness (independent of the frame rate); peaks hold, then drop.
    fn advance(&mut self, target: &[f32], smoothing: f32, dt: f32) {
        let n = self.bands.len().max(1) as f32;
        let smoothing = smoothing.clamp(0.0, 0.95);
        for (i, &t) in target.iter().enumerate().take(self.bands.len()) {
            // Bass moves more slowly: each low band is only a bin or two wide,
            // so it jumps more, and kick drums hit it hardest.
            let low = (1.0 - i as f32 / (n * 0.3)).clamp(0.0, 1.0);
            let rise = (0.35 + 0.33 * low).powf(dt * 60.0);
            let fall = (smoothing + (0.97 - smoothing) * 0.45 * low).powf(dt * 60.0);
            let v = self.bands[i];
            self.bands[i] = if t > v { t + (v - t) * rise } else { t + (v - t) * fall };
            self.peaks[i] = if self.bands[i] >= self.peaks[i] { self.bands[i] } else { (self.peaks[i] - dt * 0.6).max(self.bands[i]) };
        }
    }

    /// `samples` (`N` of them) as band levels, for tests.
    #[cfg(test)]
    fn spectrum(&mut self, samples: &[f32], rate: u32, n: usize, sensitivity: f32, dt: f32) -> Vec<f32> {
        self.re.copy_from_slice(samples);
        self.analyze(rate, n, sensitivity, dt);
        self.target.clone()
    }

    /// The samples in `re` as `n` band levels (0–1), into `target`.
    fn analyze(&mut self, rate: u32, n: usize, sensitivity: f32, dt: f32) {
        for (s, w) in self.re.iter_mut().zip(&self.window) {
            *s *= w;
        }
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im);
        let (re, im) = (&self.re, &self.im);
        for (k, m) in self.mags.iter_mut().enumerate() {
            *m = (re[k] * re[k] + im[k] * im[k]).sqrt();
        }
        let mags = &self.mags;
        let hz_per_bin = rate as f32 / N as f32;
        // Log-spaced bands from 40 Hz to 16 kHz (or Nyquist).
        let (lo, hi) = (40.0f32, (rate as f32 / 2.0).min(16_000.0));
        let out = &mut self.target;
        out.clear();
        for b in 0..n {
            let f0 = lo * (hi / lo).powf(b as f32 / n as f32);
            let f1 = lo * (hi / lo).powf((b + 1) as f32 / n as f32);
            let k0 = ((f0 / hz_per_bin) as usize).clamp(1, N / 2 - 1);
            let k1 = ((f1 / hz_per_bin).ceil() as usize).clamp(k0 + 1, N / 2);
            // Wide (high) bands take their peak; narrow (low) ones average with
            // their neighbors, which steadies the bass.
            let m = if k1 - k0 >= 3 {
                mags[k0..k1].iter().cloned().fold(0.0, f32::max)
            } else {
                let (a, z) = (k0.saturating_sub(1).max(1), (k1 + 1).min(N / 2));
                mags[a..z].iter().sum::<f32>() / (z - a) as f32
            };
            // Higher bands carry less energy; tilt them up so the bars look even,
            // and the lowest down a little.
            let w = b as f32 / n as f32;
            out.push(m * (0.8 + 2.7 * w));
        }
        // Automatic gain: follow the loudest band, falling back slowly.
        let loud = out.iter().cloned().fold(0.0, f32::max);
        self.gain_ref = if loud > self.gain_ref { loud } else { (self.gain_ref * (1.0 - dt * 0.35)).max(loud).max(1e-3) };
        let scale = sensitivity / self.gain_ref;
        for m in out.iter_mut() {
            *m = ((*m * scale).max(1e-4).log10() * 0.55 + 1.0).clamp(0.0, 1.0);
        }
    }

    /// Anything still moving (keep repainting).
    pub fn moving(&self) -> bool {
        self.live || self.bands.iter().chain(&self.peaks).any(|v| *v > 0.002)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_lights_its_band() {
        let mut a = Analyzer::default();
        let rate = 44_100.0;
        // 1 kHz sine, then the band holding 1 kHz should be the loudest.
        let samples: Vec<f32> = (0..N).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate).sin() * 0.5).collect();
        let bands = a.spectrum(&samples, 44_100, 32, 1.0, 1.0 / 60.0);
        let loudest = bands.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap().0;
        let f0 = 40.0f32 * (16_000.0f32 / 40.0).powf(loudest as f32 / 32.0);
        let f1 = 40.0f32 * (16_000.0f32 / 40.0).powf((loudest + 1) as f32 / 32.0);
        assert!(f0 <= 1050.0 && f1 >= 950.0, "band {loudest} ({f0:.0}–{f1:.0} Hz)");
        assert!(bands.iter().all(|b| (0.0..=1.0).contains(b)));
    }

    #[test]
    fn silence_settles_to_nothing() {
        let mut a = Analyzer { bands: vec![1.0; 8], peaks: vec![1.0; 8], ..Default::default() };
        for _ in 0..240 {
            a.advance(&[0.0; 8], 0.65, 1.0 / 60.0);
        }
        assert!(!a.moving(), "{:?}", a.bands);
    }
}
