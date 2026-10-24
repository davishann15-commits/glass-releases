//! Glass: a copy of the audio as it's played, for the app's visualizer.
//!
//! `Tapped` wraps a rodio source and hands the samples it plays to the tap
//! the app registered with `set`, in small blocks, at the moment rodio pulls
//! them for the output device — so the copy is in step with what's heard.
//! Without a registered tap it only forwards samples.

use std::sync::OnceLock;
use std::time::Duration;

use rodio::source::SeekError;

/// Receives (interleaved samples, channels, sample rate).
pub type Tap = Box<dyn Fn(&[f32], u16, u32) + Send + Sync>;

static TAP: OnceLock<Tap> = OnceLock::new();

/// Registers the tap (once; later calls are ignored).
pub fn set(tap: Tap) {
    let _ = TAP.set(tap);
}

/// Samples per block handed to the tap (about 12 ms of stereo at 44.1 kHz).
const BLOCK: usize = 1024;

pub struct Tapped<S> {
    inner: S,
    buf: Vec<f32>,
}

/// The block is allocated here, on the player thread, so the audio thread never allocates.
pub fn tapped<S: rodio::Source>(inner: S) -> Tapped<S> {
    let buf = if TAP.get().is_some() { Vec::with_capacity(BLOCK) } else { Vec::new() };
    Tapped { inner, buf }
}

impl<S: rodio::Source> Iterator for Tapped<S> {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        let s = self.inner.next();
        if let Some(tap) = TAP.get() {
            match s {
                Some(v) => {
                    self.buf.push(v);
                    if self.buf.len() >= BLOCK {
                        tap(&self.buf, self.inner.channels(), self.inner.sample_rate());
                        self.buf.clear();
                    }
                }
                None if !self.buf.is_empty() => {
                    tap(&self.buf, self.inner.channels(), self.inner.sample_rate());
                    self.buf.clear();
                }
                None => {}
            }
        }
        s
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: rodio::Source> rodio::Source for Tapped<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.buf.clear();
        self.inner.try_seek(pos)
    }
}
