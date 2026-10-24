//! Glass: a hook on each song Connect starts, so the app can play the DJ's
//! lines (`narration.*` in the song's metadata) around it.

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The song about to load, and the one after it (to get ready for).
pub struct TrackStart<'a> {
    /// The song's URI.
    pub uri: &'a str,
    /// It was going to start playing (one loaded paused is reported but never held).
    pub playing: bool,
    /// Its metadata from the context (the DJ's `narration.*` lines).
    pub metadata: &'a HashMap<String, String>,
    /// The song after it, and its metadata.
    pub next: Option<(&'a str, &'a HashMap<String, String>)>,
    /// The song before it played to its end (not skipped, not replaced).
    pub after_end: bool,
}

/// Set while Connect moves on from a song that ended (see `spirc.rs`).
static ENDED: AtomicBool = AtomicBool::new(false);

pub(crate) fn ending(on: bool) {
    ENDED.store(on, Ordering::Relaxed);
}

pub(crate) fn ended() -> bool {
    ENDED.load(Ordering::Relaxed)
}

/// Returns true to load the song paused (the app resumes it, e.g. once the
/// DJ has spoken).
pub type Hook = Box<dyn Fn(&TrackStart) -> bool + Send + Sync>;

static HOOK: RwLock<Option<Hook>> = RwLock::new(None);

/// Sets the hook, replacing any earlier one (each new player sets its own).
pub fn set_track_hook(hook: Hook) {
    *HOOK.write().unwrap_or_else(|e| e.into_inner()) = Some(hook);
}

pub(crate) fn track_start(t: &TrackStart) -> bool {
    HOOK.read().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|h| h(t))
}
