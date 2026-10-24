//! Which provider to ask, when to fall back, and keeping requests in step
//! with the song that's playing.

use super::cache::Cache;
use super::lrclib::Client;
use super::parse::Record;
use super::{Match, Outcome, ProviderId, TrackRef};
use std::future::Future;

/// What the Spotify option is.
pub const SPOTIFY_NOTE: &str = "Spotify's own lyrics, read through your Spotify connection. Only for songs played from Spotify.";

/// The `lyrics.provider` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderSetting {
    /// LRCLIB first, then (with fallback on) Spotify's lyrics.
    Auto,
    Lrclib,
    Spotify,
}

impl ProviderSetting {
    pub fn name(self) -> &'static str {
        match self {
            ProviderSetting::Auto => "auto",
            ProviderSetting::Lrclib => "lrclib",
            ProviderSetting::Spotify => "spotify",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ProviderSetting::Auto),
            "lrclib" => Some(ProviderSetting::Lrclib),
            "spotify" => Some(ProviderSetting::Spotify),
            _ => None,
        }
    }
}

/// Whether a provider can be used for this song, and why not.
pub fn availability(p: ProviderId, spotify: Result<(), &'static str>) -> Result<(), &'static str> {
    match p {
        ProviderId::Lrclib => Ok(()),
        ProviderId::Spotify => spotify,
    }
}

/// The providers to try, in order.
pub fn plan(setting: ProviderSetting, fallback: bool) -> Vec<ProviderId> {
    match setting {
        ProviderSetting::Auto if fallback => vec![ProviderId::Lrclib, ProviderId::Spotify],
        ProviderSetting::Auto | ProviderSetting::Lrclib => vec![ProviderId::Lrclib],
        ProviderSetting::Spotify => vec![ProviderId::Spotify],
    }
}

/// The final answer for a track, with what each provider said on the way.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub outcome: Outcome,
    pub tried: Vec<(ProviderId, &'static str)>,
}

impl Resolved {
    /// True when the lyrics shown came from a fallback provider.
    pub fn fell_back(&self) -> bool {
        self.tried.len() > 1 && self.outcome.settles()
    }
}

/// Asks each provider in `plan` in turn. An explicit choice (one provider)
/// is respected: its answer is shown even when it's "unavailable" or an
/// error. In Auto, a provider with no usable match (or an error) passes to
/// the next available one; if none has anything, the first provider's
/// answer is the result.
pub async fn resolve<F, Fut>(plan: &[ProviderId], avail: impl Fn(ProviderId) -> Result<(), &'static str>, mut fetch: F) -> Resolved
where
    F: FnMut(ProviderId) -> Fut,
    Fut: Future<Output = Outcome>,
{
    let mut tried = Vec::new();
    let mut first: Option<Outcome> = None;
    for &p in plan {
        let outcome = match avail(p) {
            Err(reason) => Outcome::Unavailable { provider: p, reason: reason.to_string() },
            Ok(()) => fetch(p).await,
        };
        tried.push((p, outcome.short()));
        if outcome.settles() {
            return Resolved { outcome, tried };
        }
        // Unavailable fallbacks don't replace a real "no match".
        let replace = first.as_ref().is_none_or(|f| matches!(f, Outcome::Unavailable { .. }) && !matches!(outcome, Outcome::Unavailable { .. }));
        if replace {
            first = Some(outcome);
        }
    }
    Resolved { outcome: first.unwrap_or(Outcome::NotFound { provider: ProviderId::Lrclib }), tried }
}

/// One provider lookup, through the cache ("look again" forgets the cached answer first).
pub async fn fetch(p: ProviderId, client: &Client, cache: &Cache, t: &TrackRef) -> Outcome {
    match p {
        ProviderId::Lrclib => {
            if let Some(l) = cache.get(t) {
                return l.outcome();
            }
            match client.lookup(t).await {
                Ok(l) => {
                    cache.put(t, &l);
                    l.outcome()
                }
                Err(e) => Outcome::Failed { provider: p, error: e.to_string() },
            }
        }
        // Asked through the app's Spotify connection (see `start_lyrics`).
        ProviderId::Spotify => Outcome::Unavailable { provider: p, reason: super::spotify::NOT_CONNECTED.into() },
    }
}

/// What the user chose for a song (`None`: "none of these"), as a result.
pub fn chosen(record: Option<&Record>) -> Resolved {
    let outcome = match record {
        Some(r) => super::lrclib::record_outcome(r, Match::Manual),
        None => Outcome::NotFound { provider: ProviderId::Lrclib },
    };
    Resolved { tried: vec![(ProviderId::Lrclib, outcome.short())], outcome }
}

// ------------------------------------------------------------- request state

/// Everything a lookup depends on; a change means a new lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub track: TrackRef,
    pub setting: ProviderSetting,
    pub fallback: bool,
    /// Bumped by "retry" to ask again.
    pub nonce: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Loading,
    /// Shared, since the lyrics view reads it every frame.
    Done(std::sync::Arc<Resolved>),
}

/// The lyrics for the song that's playing. Each lookup gets a generation;
/// starting another aborts the one in flight, and a late answer from an old
/// generation is ignored, so one song's lyrics can never land on another.
pub struct LyricsState {
    generation: u64,
    pub request: Option<Request>,
    pub status: Status,
    task: Option<tokio::task::AbortHandle>,
}

impl Default for LyricsState {
    fn default() -> Self {
        Self { generation: 0, request: None, status: Status::Idle, task: None }
    }
}

impl LyricsState {
    pub fn wants(&self, r: &Request) -> bool {
        self.request.as_ref() != Some(r)
    }

    /// Starts tracking `r`; returns its generation for the answer.
    pub fn begin(&mut self, r: Request) -> u64 {
        self.cancel();
        self.generation += 1;
        self.request = Some(r);
        self.status = Status::Loading;
        self.generation
    }

    pub fn running(&mut self, task: tokio::task::AbortHandle) {
        self.task = Some(task);
    }

    /// Accepts an answer if it's for the current lookup.
    pub fn finish(&mut self, generation: u64, res: Resolved) -> bool {
        if generation != self.generation || self.request.is_none() {
            return false;
        }
        self.task = None;
        self.status = Status::Done(std::sync::Arc::new(res));
        true
    }

    fn cancel(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }

    /// Nothing is playing (or nothing needs lyrics): stop and forget.
    pub fn reset(&mut self) {
        self.cancel();
        self.generation += 1;
        self.request = None;
        self.status = Status::Idle;
    }

    pub fn track_key(&self) -> Option<&str> {
        self.request.as_ref().map(|r| r.track.key.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::{Lyrics, Source, Timing};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn found(p: ProviderId) -> Outcome {
        let source = Source { provider: p, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 };
        Outcome::Found { lyrics: Arc::new(Lyrics { source, language: None, timing: Timing::Line, lines: Vec::new(), translations: Vec::new() }), how: Match::Exact }
    }

    fn block<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    fn all_available(_: ProviderId) -> Result<(), &'static str> {
        Ok(())
    }

    #[test]
    fn plans() {
        assert_eq!(plan(ProviderSetting::Auto, true), vec![ProviderId::Lrclib, ProviderId::Spotify]);
        assert_eq!(plan(ProviderSetting::Auto, false), vec![ProviderId::Lrclib]);
        assert_eq!(plan(ProviderSetting::Lrclib, true), vec![ProviderId::Lrclib], "an explicit choice never falls back");
        assert_eq!(plan(ProviderSetting::Spotify, true), vec![ProviderId::Spotify]);
        assert!(availability(ProviderId::Spotify, Err("not connected")).is_err());
        assert!(availability(ProviderId::Spotify, Ok(())).is_ok());
    }

    #[test]
    fn auto_prefers_lrclib() {
        let mut asked = Vec::new();
        let r = block(resolve(&plan(ProviderSetting::Auto, true), all_available, |p| {
            asked.push(p);
            async move { found(p) }
        }));
        assert_eq!(asked, vec![ProviderId::Lrclib], "Spotify isn't asked when LRCLIB has lyrics");
        assert_eq!(r.outcome.provider(), ProviderId::Lrclib);
        assert!(!r.fell_back());
    }

    #[test]
    fn auto_falls_back_to_an_available_provider() {
        let r = block(resolve(&plan(ProviderSetting::Auto, true), all_available, |p| async move { if p == ProviderId::Lrclib { Outcome::NotFound { provider: p } } else { found(p) } }));
        assert_eq!(r.outcome.provider(), ProviderId::Spotify);
        assert!(r.fell_back());
        assert_eq!(r.tried, vec![(ProviderId::Lrclib, "no match"), (ProviderId::Spotify, "found")]);
    }

    #[test]
    fn auto_keeps_lrclibs_answer_when_spotify_is_unavailable() {
        // A local file, or Spotify not connected.
        let unavailable = |p| availability(p, Err("not connected"));
        let r = block(resolve(&plan(ProviderSetting::Auto, true), unavailable, |p| async move { Outcome::NotFound { provider: p } }));
        assert_eq!(r.outcome, Outcome::NotFound { provider: ProviderId::Lrclib }, "no match on LRCLIB is the answer, not 'Spotify unavailable'");
        assert_eq!(r.tried, vec![(ProviderId::Lrclib, "no match"), (ProviderId::Spotify, "unavailable")]);
        let r = block(resolve(&plan(ProviderSetting::Auto, true), unavailable, |p| async move { Outcome::Failed { provider: p, error: "offline".into() } }));
        assert!(matches!(r.outcome, Outcome::Failed { provider: ProviderId::Lrclib, .. }));
    }

    #[test]
    fn explicit_choices_are_respected() {
        let mut asked = Vec::new();
        let r = block(resolve(
            &plan(ProviderSetting::Spotify, true),
            |p| availability(p, Err("not connected")),
            |p| {
                asked.push(p);
                async move { found(p) }
            },
        ));
        assert!(asked.is_empty(), "an unavailable provider isn't called");
        assert!(matches!(r.outcome, Outcome::Unavailable { provider: ProviderId::Spotify, .. }), "…and LRCLIB isn't used instead");
        let r = block(resolve(&plan(ProviderSetting::Lrclib, true), all_available, |p| async move { Outcome::NotFound { provider: p } }));
        assert_eq!(r.outcome, Outcome::NotFound { provider: ProviderId::Lrclib });
        assert_eq!(r.tried.len(), 1);
    }

    #[test]
    fn instrumental_and_ambiguous_settle() {
        let src = Source { provider: ProviderId::Lrclib, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 };
        let r = block(resolve(&plan(ProviderSetting::Auto, true), all_available, |p| {
            let src = src.clone();
            async move { if p == ProviderId::Lrclib { Outcome::Instrumental { source: src } } else { found(p) } }
        }));
        assert!(matches!(r.outcome, Outcome::Instrumental { .. }), "an instrumental answer isn't 'no match'");
    }

    fn request(key: &str) -> Request {
        Request { track: TrackRef { key: key.into(), title: "t".into(), artists: vec!["a".into()], album: String::new(), duration_ms: 1 }, setting: ProviderSetting::Auto, fallback: true, nonce: 0 }
    }

    fn done(p: ProviderId) -> Resolved {
        Resolved { outcome: found(p), tried: vec![(p, "found")] }
    }

    #[test]
    fn stale_answers_are_ignored() {
        let mut s = LyricsState::default();
        let a = s.begin(request("spotify:track:a"));
        let b = s.begin(request("spotify:track:b"));
        assert!(!s.finish(a, done(ProviderId::Lrclib)), "the previous song's answer is dropped");
        assert_eq!(s.status, Status::Loading);
        assert!(s.finish(b, done(ProviderId::Lrclib)));
        assert!(matches!(s.status, Status::Done(_)));
        assert_eq!(s.track_key(), Some("spotify:track:b"));
        // After a reset (nothing playing), even the latest answer is dropped.
        let c = s.begin(request("spotify:track:c"));
        s.reset();
        assert!(!s.finish(c, done(ProviderId::Lrclib)));
        assert_eq!(s.status, Status::Idle);
    }

    #[test]
    fn a_new_track_cancels_the_lookup_in_flight() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let finished = Arc::new(AtomicBool::new(false));
        let mut s = LyricsState::default();
        s.begin(request("spotify:track:a"));
        let f = finished.clone();
        let task = rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            f.store(true, Ordering::SeqCst);
        });
        s.running(task.abort_handle());
        s.begin(request("spotify:track:b"));
        let r = rt.block_on(task);
        assert!(r.unwrap_err().is_cancelled());
        assert!(!finished.load(Ordering::SeqCst), "the old lookup never completed");
    }

    #[test]
    fn requests_change_with_the_track_and_settings() {
        let s = LyricsState { request: Some(request("spotify:track:a")), ..Default::default() };
        assert!(!s.wants(&request("spotify:track:a")));
        assert!(s.wants(&request("spotify:track:b")));
        assert!(s.wants(&Request { setting: ProviderSetting::Lrclib, ..request("spotify:track:a") }), "changing the provider asks again");
        assert!(s.wants(&Request { nonce: 1, ..request("spotify:track:a") }), "retry asks again");
    }
}
