//! Following the system's audio output, as Spotify does. When the output
//! changes (AirPods connect or come off, headphones are unplugged, a new
//! output is picked in Control Center), Glass's sound moves to the new
//! default; when the device that was playing disappears altogether, playback
//! pauses instead of carrying on through the speakers.

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Bumped on every output change; audio sinks reopen when they see a new value.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The last generation that came from the playing device going away.
static GONE: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Whether output change `generation` was the device going away. The app
/// pauses then, so sinks keep the new output paused until playback resumes.
pub fn went_away(generation: u64) -> bool {
    generation != 0 && GONE.load(Ordering::Acquire) == generation
}

#[derive(Debug, PartialEq)]
pub enum Change {
    None,
    /// The default output changed; `gone` when the previous device disappeared.
    Moved {
        from: String,
        to: String,
        gone: bool,
    },
}

fn default_name() -> Option<String> {
    rodio::cpal::default_host().default_output_device().and_then(|d| d.name().ok())
}

fn still_there(name: &str) -> bool {
    rodio::cpal::default_host().output_devices().map(|mut ds| ds.any(|d| d.name().is_ok_and(|n| n == name))).unwrap_or(true)
}

pub struct OutputWatch {
    current: Option<String>,
    next_check: Instant,
}

impl Default for OutputWatch {
    fn default() -> Self {
        Self { current: None, next_check: Instant::now() }
    }
}

impl OutputWatch {
    /// Call every frame; looks once a second.
    pub fn poll(&mut self) -> Change {
        if crate::window_fx::headless() || Instant::now() < self.next_check {
            return Change::None;
        }
        self.next_check = Instant::now() + Duration::from_secs(1);
        let Some(now) = default_name() else { return Change::None };
        match self.current.replace(now.clone()) {
            Some(before) if before != now => {
                let gone = !still_there(&before);
                // Marked before the bump, so a sink that sees the new generation sees this too.
                let g = generation() + 1;
                if gone {
                    GONE.store(g, Ordering::Release);
                }
                GENERATION.store(g, Ordering::Release);
                Change::Moved { from: before, to: now, gone }
            }
            _ => Change::None,
        }
    }
}
