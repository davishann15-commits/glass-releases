//! Which providers to ask, when to fall back, and keeping requests in step
//! with the song that's playing.
//!
//! A lookup asks two things at once: the line-timed chain (LRCLIB, then
//! Spotify's lyrics when LRCLIB has none) and the word-timing services
//! (lrc.red, Qobuzify, Unison, NetEase, lrcmux, and Spicy Lyrics or
//! Musixmatch with the user's key). The first lyrics that arrive are shown
//! straight away; a validated word-timed answer replaces line timing when it
//! comes, and the final answer is the best of all of them. Word timing that
//! doesn't keep time with the line-synced lyrics is set aside (`agree`).
//! Each lookup has a generation, so one song's answer — early or late — can
//! never land on another.

use super::cache::Cache;
use super::http::Http;
use super::lrclib::Client;
use super::parse::Record;
use super::{Lyrics, Match, Outcome, ProviderId, Timing, TrackRef, agree};
use crate::ai::secrets::SecretStore;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the Spotify option is.
pub const SPOTIFY_NOTE: &str = "Spotify's own lyrics, read through your Spotify connection. Only for songs played from Spotify.";

/// The `lyrics.provider` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderSetting {
    /// LRCLIB first, then (with fallback on) Spotify's lyrics, with the word
    /// timing services asked at the same time.
    Auto,
    /// That provider only.
    Only(ProviderId),
}

impl ProviderSetting {
    pub fn name(self) -> &'static str {
        match self {
            ProviderSetting::Auto => "auto",
            ProviderSetting::Only(p) => p.key(),
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ProviderSetting::Auto),
            other => ProviderId::parse(other).map(ProviderSetting::Only),
        }
    }
}

/// The word timing services asked by default, in order. Spicy Lyrics and
/// Musixmatch join when the user adds a key.
pub const DEFAULT_SOURCES: [ProviderId; 5] = [ProviderId::LrcRed, ProviderId::Qobuzify, ProviderId::Unison, ProviderId::NetEase, ProviderId::Lrcmux];

/// What the user has set up that providers need.
#[derive(Clone, Copy, Debug, Default)]
pub struct Setup {
    /// Glass is connected to Spotify.
    pub spotify: bool,
    pub spicy_key: bool,
    pub musixmatch_key: bool,
}

/// Whether a provider can be used for this song, and why not.
pub fn availability(p: ProviderId, t: &TrackRef, setup: Setup) -> Result<(), &'static str> {
    let spotify_song = t.key.starts_with("spotify:track:");
    match p {
        ProviderId::Lrclib | ProviderId::LrcRed | ProviderId::Unison | ProviderId::NetEase | ProviderId::Qobuzify | ProviderId::Lrcmux => Ok(()),
        ProviderId::Spotify if !spotify_song => Err(super::spotify::NOT_SPOTIFY),
        ProviderId::Spotify if !setup.spotify => Err(super::spotify::NOT_CONNECTED),
        ProviderId::Spotify => Ok(()),
        ProviderId::Spicy if t.spotify_id().is_none() => Err(super::spicy::NOT_SPOTIFY),
        ProviderId::Spicy if !setup.spicy_key => Err(super::spicy::NO_KEY),
        ProviderId::Spicy => Ok(()),
        ProviderId::Musixmatch if !setup.musixmatch_key => Err(super::musixmatch::NO_KEY),
        ProviderId::Musixmatch => Ok(()),
    }
}

/// The providers to ask: the line-timed chain, in turn, and the word timing
/// services, all at once.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Plan {
    pub line: Vec<ProviderId>,
    pub word: Vec<ProviderId>,
}

pub fn plan(setting: ProviderSetting, fallback: bool, sources: &[ProviderId]) -> Plan {
    match setting {
        ProviderSetting::Auto => {
            let mut word: Vec<ProviderId> = Vec::new();
            for &p in sources {
                if !matches!(p, ProviderId::Lrclib | ProviderId::Spotify) && !word.contains(&p) {
                    word.push(p);
                }
            }
            Plan { line: if fallback { vec![ProviderId::Lrclib, ProviderId::Spotify] } else { vec![ProviderId::Lrclib] }, word }
        }
        ProviderSetting::Only(p @ (ProviderId::Lrclib | ProviderId::Spotify)) => Plan { line: vec![p], word: Vec::new() },
        ProviderSetting::Only(p) => Plan { line: Vec::new(), word: vec![p] },
    }
}

/// The final answer for a track, with what each provider said on the way.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub outcome: Outcome,
    pub tried: Vec<(ProviderId, &'static str)>,
    /// Other providers are still looking (for word timing): this may improve.
    pub pending: bool,
}

impl Resolved {
    pub fn new(outcome: Outcome, tried: Vec<(ProviderId, &'static str)>) -> Self {
        Resolved { outcome, tried, pending: false }
    }

    /// True when the lyrics shown came from a fallback because the first
    /// provider asked had none.
    pub fn fell_back(&self) -> bool {
        self.outcome.settles() && self.tried.first().is_some_and(|(p, what)| *p != self.outcome.provider() && !what.starts_with("found"))
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
            return Resolved::new(outcome, tried);
        }
        // Unavailable fallbacks don't replace a real "no match".
        let replace = first.as_ref().is_none_or(|f| matches!(f, Outcome::Unavailable { .. }) && !matches!(outcome, Outcome::Unavailable { .. }));
        if replace {
            first = Some(outcome);
        }
    }
    Resolved::new(first.unwrap_or(Outcome::NotFound { provider: plan.first().copied().unwrap_or(ProviderId::Lrclib) }), tried)
}

/// Word timing counts for ranking when this share of sung lines has it.
const WORD_COVERAGE: f32 = 0.8;

/// The lyrics an outcome found.
fn found(o: &Outcome) -> Option<&Lyrics> {
    match o {
        Outcome::Found { lyrics, .. } => Some(lyrics),
        _ => None,
    }
}

fn word_timed(l: &Lyrics) -> bool {
    l.timing >= Timing::Line && l.word_coverage() >= WORD_COVERAGE
}

/// How good an answer is: word timing beats line timing beats plain text;
/// among equals, word timing goes by the services' order (answers whose
/// recording isn't confirmed after the rest) and line timing prefers LRCLIB
/// (then the services, then Spotify's lyrics). `in_time`: its word timing
/// can be used (see `trusted`); if not, it counts as line timing.
fn rank(o: &Outcome, word_order: &[ProviderId], in_time: bool) -> Option<(u8, i32)> {
    let lyrics = found(o)?;
    let p = lyrics.source.provider;
    let class = if word_timed(lyrics) && in_time {
        3
    } else if lyrics.timing >= Timing::Line {
        2
    } else {
        1
    };
    let at = |list: &[ProviderId]| list.iter().position(|&x| x == p).map(|i| i as i32);
    let pref = match (class, p) {
        (3, ProviderId::Lrclib) => word_order.len() as i32,
        (3, _) => at(word_order).unwrap_or(50) + if lyrics.unconfirmed { 20 } else { 0 },
        (_, ProviderId::Lrclib) => -1,
        (_, ProviderId::Spotify) => 100,
        _ => at(word_order).unwrap_or(50),
    };
    Some((class, -pref))
}

/// Whether the word timing of `words[i]` keeps time with the song. Without
/// line-synced lyrics to compare with, it's taken as it is. With them, it
/// has to agree; where it doesn't, it's still taken from a service that
/// matched the recording by its ID (its timing is the recording's own), or
/// when another confirmed service's word timing agrees with it (then the
/// line-synced lyrics are the ones off). An unconfirmed answer (Qobuzify)
/// must agree outright.
fn trusted(i: usize, words: &[(ProviderId, Outcome)], reference: Option<&Lyrics>) -> bool {
    let (_, o) = &words[i];
    let (Some(w), Some(reference)) = (found(o), reference) else { return true };
    if w.source.provider == reference.source.provider {
        return true;
    }
    match agree::check(w, reference) {
        agree::Check::Agrees => true,
        agree::Check::Unknown => !w.unconfirmed,
        agree::Check::Disagrees(_) if w.unconfirmed => false,
        agree::Check::Disagrees(_) => {
            matches!(o, Outcome::Found { how: Match::Exact, .. })
                || words.iter().enumerate().any(|(j, (_, x))| {
                    j != i && found(x).is_some_and(|x| !x.unconfirmed && x.source.provider != w.source.provider && word_timed(x) && agree::offset(w, x).is_some_and(|d| d.abs() <= agree::AGREE_MS))
                })
        }
    }
}

/// Whether `o` is the lyrics the user picked (`pick`), so its timing may stand in for
/// theirs: the same lines at the same times, or, for a pick without timing, the same
/// words.
fn fits_pick(o: &Outcome, pick: &Lyrics) -> bool {
    let Some(l) = found(o) else { return false };
    if pick.timing >= Timing::Line { agree::check(l, pick) == agree::Check::Agrees } else { agree::same_words(l, pick) }
}

/// The best of what the line chain and the word services said so far. When the
/// line answer is the user's pick (`Match::Manual`), only the same lyrics, better
/// timed, can take its place, and they stay "your pick".
pub fn combine(line: Option<&Resolved>, words: &[(ProviderId, Outcome)], word_order: &[ProviderId]) -> Resolved {
    let reference = line.and_then(|r| found(&r.outcome)).filter(|l| l.timing >= Timing::Line);
    let pick = line.and_then(|r| match &r.outcome {
        Outcome::Found { lyrics, how: Match::Manual } => Some(&**lyrics),
        _ => None,
    });
    let in_time: Vec<bool> = (0..words.len()).map(|i| pick.map(|p| fits_pick(&words[i].1, p)).unwrap_or_else(|| trusted(i, words, reference))).collect();
    let mut tried: Vec<(ProviderId, &'static str)> = line.map(|r| r.tried.clone()).unwrap_or_default();
    tried.extend(words.iter().zip(&in_time).map(|((p, o), &ok)| {
        let what = match found(o) {
            _ if ok => o.short(),
            Some(_) if pick.is_some() => "not the lyrics you picked",
            Some(l) if word_timed(l) => "word sync timed to another version",
            _ => o.short(),
        };
        (*p, what)
    }));
    // Over a pick, other lyrics aren't candidates at all.
    let words_in = words.iter().map(|(_, o)| o).zip(in_time.iter().copied()).filter(|(_, ok)| *ok || pick.is_none());
    let candidates = line.map(|r| (&r.outcome, true)).into_iter().chain(words_in);
    let best = candidates.filter_map(|(o, ok)| rank(o, word_order, ok).map(|r| (r, o))).max_by_key(|(r, _)| *r).map(|(_, o)| o.clone());
    let best = match best {
        Some(Outcome::Found { lyrics, .. }) if pick.is_some() => Some(Outcome::Found { lyrics, how: Match::Manual }),
        b => b,
    };
    let outcome = match best {
        // Word timing over the line-synced lyrics: spelled as they spell it (NetEase's kanji).
        Some(Outcome::Found { lyrics, how }) if word_timed(&lyrics) && reference.is_some_and(|r| r.source.provider != lyrics.source.provider) => {
            let mut l = (*lyrics).clone();
            agree::respell(&mut l, reference.unwrap());
            Outcome::Found { lyrics: if l == *lyrics { lyrics } else { Arc::new(l) }, how }
        }
        Some(o) => o,
        None => match line {
            Some(r) => r.outcome.clone(),
            // No line chain (one word service chosen): its answer, whatever it is.
            None => words.iter().map(|(_, o)| o).find(|o| o.settles()).or(words.first().map(|(_, o)| o)).cloned().unwrap_or(Outcome::NotFound { provider: ProviderId::Lrclib }),
        },
    };
    Resolved { outcome: settle_language(outcome), tried, pending: false }
}

/// The language a source gives, unless the lyrics' own script rules it out: lrc.red
/// marks many Japanese songs "en", which would show as English and keep them from
/// being translated (see `translate::source_language`).
fn settle_language(o: Outcome) -> Outcome {
    let Outcome::Found { lyrics, how } = &o else { return o };
    let Some(given) = lyrics.language.as_deref() else { return o };
    let lines: Vec<String> = lyrics.lines.iter().map(|l| l.text.clone()).collect();
    let language = super::translate::source_language(Some(given), &lines);
    if language.as_deref() == Some(given) {
        return o;
    }
    let mut l = (**lyrics).clone();
    l.language = language;
    Outcome::Found { lyrics: Arc::new(l), how: *how }
}

/// How long to wait for the word services after the line chain found
/// nothing, before saying so (they can still answer later).
const PATIENCE: Duration = Duration::from_secs(5);

/// Looks lyrics up by `plan`. `send` gets each better answer as it arrives
/// (with `pending` set), and the final one last. `given`: the line answer is
/// already known (the user's pick), so the line chain isn't asked.
pub async fn lookup<F, Fut>(plan: &Plan, given: Option<Resolved>, avail: impl Fn(ProviderId) -> Result<(), &'static str>, fetch: F, mut send: impl FnMut(Resolved))
where
    F: Fn(ProviderId) -> Fut,
    Fut: Future<Output = Outcome>,
{
    use futures_util::stream::{FuturesUnordered, StreamExt};
    let fetch = &fetch;
    let avail = &avail;
    let mut words: Vec<(ProviderId, Outcome)> = Vec::new();
    let mut asking = FuturesUnordered::new();
    for &p in &plan.word {
        match avail(p) {
            Err(reason) => words.push((p, Outcome::Unavailable { provider: p, reason: reason.into() })),
            Ok(()) => asking.push(async move { (p, fetch(p).await) }),
        }
    }
    let has_line = given.is_none() && !plan.line.is_empty();
    let line_fut = async { if has_line { Some(resolve(&plan.line, avail, fetch).await) } else { None } };
    let mut line_fut = std::pin::pin!(line_fut);
    let mut line: Option<Resolved> = given;
    let mut line_done = !has_line;
    let mut shown: Option<(ProviderId, String, Timing)> = None;
    let mut deadline: Option<tokio::time::Instant> = None;
    let mut waited = false;
    let word_order = plan.word.clone();
    let identity = |r: &Resolved| match &r.outcome {
        Outcome::Found { lyrics, .. } => Some((lyrics.source.provider, lyrics.source.id.clone(), lyrics.timing)),
        _ => None,
    };
    // Known lyrics show at once while the word services look.
    if line.is_some() && !asking.is_empty() {
        let r = combine(line.as_ref(), &words, &word_order);
        shown = identity(&r);
        send(Resolved { pending: true, ..r });
    }
    loop {
        let more = !line_done || !asking.is_empty();
        if !more {
            break;
        }
        let sleep = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            r = &mut line_fut, if !line_done => {
                line_done = true;
                line = r;
            }
            Some((p, o)) = asking.next(), if !asking.is_empty() => words.push((p, o)),
            _ = sleep => {
                deadline = None;
                waited = true;
                // The line chain found nothing and the word services are slow: say so now.
                let mut r = combine(line.as_ref(), &words, &word_order);
                r.pending = true;
                shown = identity(&r);
                send(r);
                continue;
            }
        }
        if line_done && asking.is_empty() {
            break;
        }
        let r = combine(line.as_ref(), &words, &word_order);
        match identity(&r) {
            // Lyrics, or better lyrics: show them now.
            Some(id) if shown.as_ref() != Some(&id) => {
                shown = Some(id);
                send(Resolved { pending: true, ..r });
            }
            Some(_) => {}
            // Nothing yet: give the word services a moment once the line chain is done.
            None if line_done && !waited && deadline.is_none() && shown.is_none() => deadline = Some(tokio::time::Instant::now() + PATIENCE),
            None => {}
        }
    }
    send(combine(line.as_ref(), &words, &word_order));
}

/// What the user chose for a song (`None`: "none of these"), as a result.
pub fn chosen(record: Option<&Record>) -> Resolved {
    let outcome = match record {
        Some(r) => super::lrclib::record_outcome(r, Match::Manual),
        None => Outcome::NotFound { provider: ProviderId::Lrclib },
    };
    Resolved::new(outcome.clone(), vec![(ProviderId::Lrclib, outcome.short())])
}

// ---------------------------------------------------------------- services

/// Every service's client, the cache, and the user's keys: everything a
/// lookup needs, shared by the lookups (one per song).
pub struct Sources {
    pub lrclib: Arc<Client>,
    pub cache: Arc<Cache>,
    lrcred: Http,
    unison: Http,
    netease: Http,
    spicy: Http,
    musixmatch: Http,
    qobuzify: Http,
    lrcmux: Http,
    bases: HashMap<ProviderId, String>,
    keys: Keys,
}

/// Where the services are.
fn base(p: ProviderId) -> &'static str {
    match p {
        ProviderId::Lrclib => super::lrclib::BASE,
        ProviderId::Spotify => "",
        ProviderId::LrcRed => super::lrcred::BASE,
        ProviderId::Unison => super::unison::BASE,
        ProviderId::NetEase => super::netease::BASE,
        ProviderId::Musixmatch => super::musixmatch::BASE,
        ProviderId::Spicy => super::spicy::BASE,
        ProviderId::Qobuzify => super::qobuzify::BASE,
        ProviderId::Lrcmux => super::lrcmux::BASE,
    }
}

/// Keys the user brought (Spicy Lyrics, Musixmatch), read from the system's
/// secret store when first needed and kept in memory after.
struct Keys {
    store: Option<Arc<dyn SecretStore>>,
    known: Mutex<HashMap<ProviderId, Option<String>>>,
}

fn secret_name(p: ProviderId) -> String {
    format!("lyrics-key:{}", p.key())
}

impl Sources {
    /// `secrets`: where the keys live (none: keys can't be kept).
    pub fn new(contact: &str, proxy: &str, log: Option<crate::api::NetLog>, cache: Arc<Cache>, secrets: Option<Arc<dyn SecretStore>>) -> Arc<Self> {
        let http = |host| Http::new(host, contact, proxy, log.clone());
        Arc::new(Sources {
            lrclib: Client::new(contact, proxy, log.clone()),
            cache,
            lrcred: http("lrc.red"),
            unison: http("unison.boidu.dev"),
            netease: http("music.163.com"),
            spicy: http("api.spicylyrics.org"),
            musixmatch: http("api.musixmatch.com"),
            qobuzify: http("api.qobuzify.app"),
            lrcmux: http("api.lrcmux.dev"),
            bases: ProviderId::ALL.into_iter().map(|p| (p, base(p).to_string())).collect(),
            keys: Keys { store: secrets, known: Mutex::default() },
        })
    }

    /// Demo mode and tests: answers only from `cache`; every service points at
    /// a port nobody listens on, so a lookup fails at once.
    pub fn offline(cache: Arc<Cache>) -> Arc<Self> {
        let mut s = Self::new("", "", None, cache, None);
        let me = Arc::get_mut(&mut s).expect("new");
        me.lrclib = Client::with_base("http://127.0.0.1:9", "", "", None);
        for b in me.bases.values_mut() {
            *b = "http://127.0.0.1:9".into();
        }
        s
    }

    /// Tests: the services at `base` (a local server).
    #[cfg(test)]
    pub fn at(base: &str, cache: Arc<Cache>) -> Arc<Self> {
        let mut s = Self::new("", "", None, cache, None);
        let me = Arc::get_mut(&mut s).expect("new");
        me.lrclib = Client::with_base(base, "", "", None);
        for b in me.bases.values_mut() {
            *b = base.to_string();
        }
        s
    }

    /// The user's key for `p`, if they added one.
    pub fn key(&self, p: ProviderId) -> Option<String> {
        if let Some(k) = self.keys.known.lock().unwrap().get(&p) {
            return k.clone();
        }
        let k = self.keys.store.as_ref().and_then(|s| s.get(&secret_name(p)).map_err(|e| log::warn!("lyrics key for {}: {e}", p.label())).ok().flatten()).filter(|k| !k.trim().is_empty());
        self.keys.known.lock().unwrap().insert(p, k.clone());
        k
    }

    /// Keeps (or with `None`, forgets) the user's key for `p`.
    pub fn set_key(&self, p: ProviderId, key: Option<&str>) -> Result<(), String> {
        let key = key.map(str::trim).filter(|k| !k.is_empty());
        if let Some(store) = &self.keys.store {
            match key {
                Some(k) => store.set(&secret_name(p), k)?,
                None => store.delete(&secret_name(p))?,
            }
        }
        self.keys.known.lock().unwrap().insert(p, key.map(str::to_string));
        Ok(())
    }

    /// One provider's answer for `t`, through the cache. (Spotify's lyrics
    /// are asked through the app's Spotify connection: `spotify`.)
    pub async fn fetch(&self, p: ProviderId, t: &TrackRef, spotify: Option<Arc<dyn crate::ai::spotify::Source>>) -> Outcome {
        match p {
            ProviderId::Lrclib => {
                if let Some(l) = self.cache.get(t) {
                    return l.outcome();
                }
                match self.lrclib.lookup(t).await {
                    Ok(l) => {
                        self.cache.put(t, &l);
                        l.outcome()
                    }
                    Err(e) => Outcome::Failed { provider: p, error: e.to_string() },
                }
            }
            ProviderId::Spotify => super::spotify::fetch(spotify, t).await,
            _ => {
                if let Some(o) = self.cache.get_outcome(p, t) {
                    return o;
                }
                let b = &self.bases[&p];
                let o = match p {
                    ProviderId::LrcRed => super::lrcred::fetch(&self.lrcred, b, t).await,
                    ProviderId::Unison => super::unison::fetch(&self.unison, b, t).await,
                    ProviderId::NetEase => super::netease::fetch(&self.netease, b, t).await,
                    ProviderId::Spicy => super::spicy::fetch(&self.spicy, b, self.key(p).as_deref(), t).await,
                    ProviderId::Musixmatch => super::musixmatch::fetch(&self.musixmatch, b, self.key(p).as_deref(), t).await,
                    ProviderId::Qobuzify => super::qobuzify::fetch(&self.qobuzify, b, t).await,
                    ProviderId::Lrcmux => super::lrcmux::fetch(&self.lrcmux, b, t).await,
                    ProviderId::Lrclib | ProviderId::Spotify => unreachable!(),
                };
                if let Outcome::Failed { error, .. } = &o {
                    log::info!("lyrics: {error}");
                }
                self.cache.put_outcome(p, t, &o);
                o
            }
        }
    }

    /// Fills in what a lookup can use but the player didn't say: a local
    /// file's ISRC, from its tags.
    pub async fn complete(t: &mut TrackRef) {
        if !t.isrc.is_empty() {
            return;
        }
        let Some(path) = t.local_path().map(std::path::PathBuf::from) else { return };
        t.isrc = tokio::task::spawn_blocking(move || file_isrc(&path)).await.ok().flatten().unwrap_or_default();
    }
}

/// The ISRC in a file's tags, if any.
pub fn file_isrc(path: &std::path::Path) -> Option<String> {
    use lofty::file::TaggedFileExt;
    let f = lofty::read_from_path(path).ok()?;
    f.tags().iter().find_map(|t| t.get_string(lofty::tag::ItemKey::Isrc).and_then(super::clean_isrc))
}

/// Looks lyrics up for `t` with `sources` (the whole flow both apps run).
/// `send` gets each better answer, then the final one. `pick`: lyrics the user
/// chose for the song; they stand in for the line chain, and the word services
/// are asked for the same lyrics with word timing.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    sources: Arc<Sources>,
    setting: ProviderSetting,
    fallback: bool,
    word_sources: Vec<ProviderId>,
    mut t: TrackRef,
    pick: Option<Record>,
    spotify: Option<Arc<dyn crate::ai::spotify::Source>>,
    send: impl FnMut(Resolved),
) {
    Sources::complete(&mut t).await;
    let setup = Setup {
        spotify: spotify.is_some(),
        spicy_key: (setting == ProviderSetting::Only(ProviderId::Spicy) || word_sources.contains(&ProviderId::Spicy)) && sources.key(ProviderId::Spicy).is_some(),
        musixmatch_key: (setting == ProviderSetting::Only(ProviderId::Musixmatch) || word_sources.contains(&ProviderId::Musixmatch)) && sources.key(ProviderId::Musixmatch).is_some(),
    };
    let plan = plan(setting, fallback, &word_sources);
    let t = &t;
    lookup(&plan, pick.as_ref().map(|r| chosen(Some(r))), |p| availability(p, t, setup), |p| sources.fetch(p, t, spotify.clone()), send).await;
}

// ------------------------------------------------------------- request state

/// Everything a lookup depends on; a change means a new lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub track: TrackRef,
    pub setting: ProviderSetting,
    pub fallback: bool,
    pub sources: Vec<ProviderId>,
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

    /// The current lookup's generation (see `begin`).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn running(&mut self, task: tokio::task::AbortHandle) {
        self.task = Some(task);
    }

    /// Accepts an answer if it's for the current lookup (an early one, with
    /// `pending` set, may be followed by better ones).
    pub fn finish(&mut self, generation: u64, res: Resolved) -> bool {
        if generation != self.generation || self.request.is_none() {
            return false;
        }
        if !res.pending {
            self.task = None;
        }
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
    use crate::lyrics::{Line, Lyrics, Source, Token};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn lyrics(p: ProviderId, timing: Timing) -> Arc<Lyrics> {
        let source = Source { provider: p, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 };
        let tokens = if timing >= Timing::Word { vec![Token { text: "x".into(), start_ms: 0, end_ms: None }] } else { Vec::new() };
        Arc::new(Lyrics::new(source, timing, vec![Line { text: "x".into(), tokens, ..Default::default() }]))
    }

    fn found(p: ProviderId) -> Outcome {
        Outcome::Found { lyrics: lyrics(p, Timing::Line), how: Match::Exact }
    }

    fn words(p: ProviderId) -> Outcome {
        Outcome::Found { lyrics: lyrics(p, Timing::Word), how: Match::Exact }
    }

    fn block<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    fn all_available(_: ProviderId) -> Result<(), &'static str> {
        Ok(())
    }

    fn spotify_track() -> TrackRef {
        TrackRef { key: "spotify:track:0VjIjW4GlUZAMYd2vXMi3b".into(), title: "t".into(), artists: vec!["a".into()], duration_ms: 1, ..Default::default() }
    }

    #[test]
    fn plans() {
        let w = DEFAULT_SOURCES;
        assert_eq!(plan(ProviderSetting::Auto, true, &w), Plan { line: vec![ProviderId::Lrclib, ProviderId::Spotify], word: w.to_vec() });
        assert_eq!(plan(ProviderSetting::Auto, false, &[]).line, vec![ProviderId::Lrclib]);
        assert_eq!(plan(ProviderSetting::Only(ProviderId::Lrclib), true, &w), Plan { line: vec![ProviderId::Lrclib], word: vec![] }, "an explicit choice never falls back");
        assert_eq!(plan(ProviderSetting::Only(ProviderId::NetEase), true, &w), Plan { line: vec![], word: vec![ProviderId::NetEase] });
        assert_eq!(ProviderSetting::parse("LRC.red"), Some(ProviderSetting::Only(ProviderId::LrcRed)));
        assert_eq!(ProviderSetting::parse("genius"), None);
        let local = TrackRef { key: "local:/m/a.mp3".into(), ..Default::default() };
        assert!(availability(ProviderId::Spotify, &local, Setup { spotify: true, ..Default::default() }).is_err());
        assert!(availability(ProviderId::Spotify, &spotify_track(), Setup::default()).is_err());
        assert!(availability(ProviderId::Spicy, &spotify_track(), Setup { spicy_key: true, ..Default::default() }).is_ok());
        assert!(availability(ProviderId::Spicy, &local, Setup { spicy_key: true, ..Default::default() }).is_err(), "Spicy Lyrics needs a Spotify ID");
        assert!(availability(ProviderId::Musixmatch, &local, Setup::default()).is_err(), "no key, no Musixmatch");
        assert!(availability(ProviderId::NetEase, &local, Setup::default()).is_ok());
    }

    #[test]
    fn auto_prefers_lrclib_in_the_line_chain() {
        let mut asked = Vec::new();
        let r = block(resolve(&[ProviderId::Lrclib, ProviderId::Spotify], all_available, |p| {
            asked.push(p);
            async move { found(p) }
        }));
        assert_eq!(asked, vec![ProviderId::Lrclib], "Spotify isn't asked when LRCLIB has lyrics");
        assert_eq!(r.outcome.provider(), ProviderId::Lrclib);
        assert!(!r.fell_back());
    }

    #[test]
    fn auto_falls_back_to_an_available_provider() {
        let r = block(resolve(&[ProviderId::Lrclib, ProviderId::Spotify], all_available, |p| async move { if p == ProviderId::Lrclib { Outcome::NotFound { provider: p } } else { found(p) } }));
        assert_eq!(r.outcome.provider(), ProviderId::Spotify);
        assert!(r.fell_back());
        assert_eq!(r.tried, vec![(ProviderId::Lrclib, "no match"), (ProviderId::Spotify, "found")]);
    }

    #[test]
    fn auto_keeps_lrclibs_answer_when_spotify_is_unavailable() {
        let unavailable = |p| if p == ProviderId::Spotify { Err("not connected") } else { Ok(()) };
        let r = block(resolve(&[ProviderId::Lrclib, ProviderId::Spotify], unavailable, |p| async move { Outcome::NotFound { provider: p } }));
        assert_eq!(r.outcome, Outcome::NotFound { provider: ProviderId::Lrclib }, "no match on LRCLIB is the answer, not 'Spotify unavailable'");
        assert_eq!(r.tried, vec![(ProviderId::Lrclib, "no match"), (ProviderId::Spotify, "unavailable")]);
        let r = block(resolve(&[ProviderId::Lrclib, ProviderId::Spotify], unavailable, |p| async move { Outcome::Failed { provider: p, error: "offline".into() } }));
        assert!(matches!(r.outcome, Outcome::Failed { provider: ProviderId::Lrclib, .. }));
    }

    /// Lyrics with these lines (start ms, text), word-timed (a piece per letter) or not.
    fn song(p: ProviderId, lines: &[(u64, &str)], timing: Timing) -> Arc<Lyrics> {
        let source = Source { provider: p, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 };
        let lines = lines
            .iter()
            .map(|&(start, text)| {
                let tokens =
                    if timing >= Timing::Word { text.chars().enumerate().map(|(i, c)| Token { text: c.to_string(), start_ms: start + i as u64 * 100, end_ms: None }).collect() } else { Vec::new() };
                Line { start_ms: start, text: text.into(), tokens, ..Default::default() }
            })
            .collect();
        Arc::new(Lyrics::new(source, timing, lines))
    }

    const PICKED: [(u64, &str); 5] = [(10_000, "Es escalofriante"), (14_000, "Tenerte de frente"), (21_000, "Daría cualquier cosa"), (38_000, "Disfruto"), (45_000, "Tenerte otra vez")];

    #[test]
    fn a_pick_takes_word_timing_only_for_the_same_lyrics() {
        let order = [ProviderId::LrcRed, ProviderId::NetEase];
        let pick = |timing| chosen_lyrics(song(ProviderId::Lrclib, &PICKED, timing));
        let same = |shift: u64| Outcome::Found { lyrics: song(ProviderId::LrcRed, &PICKED.map(|(t, l)| (t + shift, l)), Timing::Word), how: Match::Search };
        let other = Outcome::Found {
            lyrics: song(ProviderId::NetEase, &[(10_000, "Otra canción"), (14_000, "con otras palabras"), (21_000, "y otro tiempo"), (38_000, "nada igual"), (45_000, "fin")], Timing::Word),
            how: Match::Exact,
        };

        // The same lines at the same times, word by word: they take the pick's place, and stay "your pick".
        let r = combine(Some(&pick(Timing::Line)), &[(ProviderId::LrcRed, same(400))], &order);
        let Outcome::Found { lyrics, how } = &r.outcome else { panic!("{r:?}") };
        assert_eq!((lyrics.source.provider, lyrics.timing, *how), (ProviderId::LrcRed, Timing::Word, Match::Manual));

        // Other lyrics never replace the user's choice, even matched by ID.
        let r = combine(Some(&pick(Timing::Line)), &[(ProviderId::NetEase, other.clone())], &order);
        assert_eq!(r.outcome.provider(), ProviderId::Lrclib);
        assert!(r.tried.contains(&(ProviderId::NetEase, "not the lyrics you picked")), "{:?}", r.tried);
        // Nor the same words timed to another version (3 s late).
        assert_eq!(combine(Some(&pick(Timing::Line)), &[(ProviderId::LrcRed, same(3_000))], &order).outcome.provider(), ProviderId::Lrclib);

        // A pick without timing takes the same words, timed.
        let r = combine(Some(&pick(Timing::Plain)), &[(ProviderId::NetEase, other), (ProviderId::LrcRed, same(3_000))], &order);
        assert_eq!((r.outcome.provider(), matches!(r.outcome, Outcome::Found { how: Match::Manual, .. })), (ProviderId::LrcRed, true));
    }

    /// `chosen` for these lyrics.
    fn chosen_lyrics(l: Arc<Lyrics>) -> Resolved {
        Resolved::new(Outcome::Found { lyrics: l, how: Match::Manual }, vec![(ProviderId::Lrclib, "found")])
    }

    #[test]
    fn a_pick_shows_at_once_while_word_timing_is_looked_for() {
        let plan = plan(ProviderSetting::Auto, true, &[ProviderId::LrcRed]);
        let given = chosen_lyrics(song(ProviderId::Lrclib, &PICKED, Timing::Line));
        let mut sent: Vec<(ProviderId, Timing, bool)> = Vec::new();
        let asked = std::sync::Mutex::new(Vec::new());
        block(lookup(
            &plan,
            Some(given),
            all_available,
            |p| {
                asked.lock().unwrap().push(p);
                async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Outcome::Found { lyrics: song(p, &PICKED.map(|(t, l)| (t + 300, l)), Timing::Word), how: Match::Search }
                }
            },
            |r| {
                let Outcome::Found { lyrics, .. } = &r.outcome else { panic!("{r:?}") };
                sent.push((lyrics.source.provider, lyrics.timing, r.pending));
            },
        ));
        assert_eq!(*asked.lock().unwrap(), vec![ProviderId::LrcRed], "LRCLIB isn't asked: the pick is the line answer");
        assert_eq!(sent, vec![(ProviderId::Lrclib, Timing::Line, true), (ProviderId::LrcRed, Timing::Word, false)]);
    }

    #[test]
    fn word_timing_beats_line_timing_and_order_breaks_ties() {
        let order = [ProviderId::LrcRed, ProviderId::Unison, ProviderId::NetEase];
        let line = Resolved::new(found(ProviderId::Lrclib), vec![(ProviderId::Lrclib, "found")]);
        let r = combine(Some(&line), &[(ProviderId::NetEase, words(ProviderId::NetEase)), (ProviderId::LrcRed, words(ProviderId::LrcRed))], &order);
        assert_eq!(r.outcome.provider(), ProviderId::LrcRed, "word timing, and lrc.red before NetEase");
        assert!(!r.fell_back(), "LRCLIB had lyrics; word timing isn't a fallback");
        // Line timing from a service doesn't push LRCLIB's aside.
        let r = combine(Some(&line), &[(ProviderId::LrcRed, found(ProviderId::LrcRed))], &order);
        assert_eq!(r.outcome.provider(), ProviderId::Lrclib);
        // Validated lyrics beat "several versions match" and "instrumental".
        let ambiguous = Resolved::new(Outcome::Ambiguous { provider: ProviderId::Lrclib, candidates: vec![] }, vec![(ProviderId::Lrclib, "several matches")]);
        assert_eq!(combine(Some(&ambiguous), &[(ProviderId::Unison, words(ProviderId::Unison))], &order).outcome.provider(), ProviderId::Unison);
        // Nothing anywhere: the line chain's answer, with everyone in `tried`.
        let none = Resolved::new(Outcome::NotFound { provider: ProviderId::Lrclib }, vec![(ProviderId::Lrclib, "no match")]);
        let r = combine(
            Some(&none),
            &[(ProviderId::LrcRed, Outcome::NotFound { provider: ProviderId::LrcRed }), (ProviderId::NetEase, Outcome::Failed { provider: ProviderId::NetEase, error: "x".into() })],
            &order,
        );
        assert_eq!(r.outcome, Outcome::NotFound { provider: ProviderId::Lrclib });
        assert_eq!(r.tried.len(), 3);
        // One service chosen on its own: its own answer, even an unavailable one.
        let r = combine(None, &[(ProviderId::Spicy, Outcome::Unavailable { provider: ProviderId::Spicy, reason: "no key".into() })], &[ProviderId::Spicy]);
        assert!(matches!(r.outcome, Outcome::Unavailable { provider: ProviderId::Spicy, .. }));
    }

    /// Lyrics of six lines starting at `at` + the usual times, word-timed or not.
    fn sung(p: ProviderId, at: i64, words: bool, how: Match, unconfirmed: bool) -> Outcome {
        let texts = ["Es escalofriante", "Tenerte de frente", "Daría cualquier cosa", "Disfruto", "Un rayo de luz", "觉まされた昼下がり"];
        let lines = texts
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let start = (10_000 + i as i64 * 6_000 + at) as u64;
                let tokens = if words { t.split_inclusive(' ').enumerate().map(|(k, w)| Token { text: w.into(), start_ms: start + k as u64 * 300, end_ms: None }).collect() } else { Vec::new() };
                Line { start_ms: start, text: t.to_string(), tokens, ..Default::default() }
            })
            .collect();
        let source = Source { provider: p, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 };
        let mut l = Lyrics::new(source, if words { Timing::Word } else { Timing::Line }, lines);
        l.unconfirmed = unconfirmed;
        Outcome::Found { lyrics: Arc::new(l), how }
    }

    fn picked(r: &Resolved) -> (ProviderId, Timing) {
        let Outcome::Found { lyrics, .. } = &r.outcome else { panic!("{r:?}") };
        (lyrics.source.provider, lyrics.timing)
    }

    #[test]
    fn word_timing_has_to_keep_time_with_the_line_lyrics() {
        let order = DEFAULT_SOURCES;
        let lrclib = Resolved::new(sung(ProviderId::Lrclib, 0, false, Match::Exact, false), vec![(ProviderId::Lrclib, "found")]);
        let qz = |at| (ProviderId::Qobuzify, sung(ProviderId::Qobuzify, at, true, Match::Exact, true));
        // In time (a little after each line's time, as sung): used.
        assert_eq!(picked(&combine(Some(&lrclib), &[qz(400)], &order)), (ProviderId::Qobuzify, Timing::Word));
        // Three seconds late throughout (another version): LRCLIB's lines stay.
        let r = combine(Some(&lrclib), &[qz(3_000)], &order);
        assert_eq!(picked(&r), (ProviderId::Lrclib, Timing::Line));
        assert!(r.tried.contains(&(ProviderId::Qobuzify, "word sync timed to another version")));
        // With nothing to compare against, it's better than nothing.
        assert_eq!(picked(&combine(None, &[qz(3_000)], &order)), (ProviderId::Qobuzify, Timing::Word));
        // A confirmed service found by search that disagrees on its own: set aside…
        let ne = (ProviderId::NetEase, sung(ProviderId::NetEase, -2_500, true, Match::Search, false));
        assert_eq!(picked(&combine(Some(&lrclib), &[ne.clone()], &order)).0, ProviderId::Lrclib);
        // …unless another confirmed service agrees with it: then LRCLIB is the one off.
        let mux = (ProviderId::Lrcmux, sung(ProviderId::Lrcmux, -2_300, true, Match::Search, false));
        assert_eq!(picked(&combine(Some(&lrclib), &[ne.clone(), mux], &order)).0, ProviderId::NetEase, "NetEase before lrcmux in the order");
        // Qobuzify repeating NetEase's timing doesn't vouch for it (it isn't confirmed).
        assert_eq!(picked(&combine(Some(&lrclib), &[ne, qz(-2_500)], &order)).0, ProviderId::Lrclib);
        // lrc.red by ISRC is the recording's own timing, whatever LRCLIB says.
        let red = (ProviderId::LrcRed, sung(ProviderId::LrcRed, 2_000, true, Match::Exact, false));
        assert_eq!(picked(&combine(Some(&lrclib), &[red], &order)).0, ProviderId::LrcRed);
        // Confirmed answers come before unconfirmed ones when both are in time.
        let un = (ProviderId::Unison, sung(ProviderId::Unison, 300, true, Match::Search, false));
        assert_eq!(picked(&combine(Some(&lrclib), &[qz(300), un], &order)).0, ProviderId::Unison, "Qobuzify is earlier in the order but unconfirmed");
    }

    #[test]
    fn the_chosen_word_timing_is_spelled_like_the_line_lyrics() {
        let mut reference = sung(ProviderId::Lrclib, 0, false, Match::Exact, false);
        if let Outcome::Found { lyrics, .. } = &mut reference {
            Arc::make_mut(lyrics).lines[5].text = "覚まされた昼下がり".into();
        }
        let lrclib = Resolved::new(reference, vec![(ProviderId::Lrclib, "found")]);
        let r = combine(Some(&lrclib), &[(ProviderId::NetEase, sung(ProviderId::NetEase, 300, true, Match::Search, false))], &DEFAULT_SOURCES);
        let Outcome::Found { lyrics, .. } = &r.outcome else { panic!() };
        assert_eq!((lyrics.source.provider, lyrics.lines[5].text.as_str()), (ProviderId::NetEase, "覚まされた昼下がり"));
    }

    #[test]
    fn a_japanese_song_marked_english_is_japanese() {
        let japanese = |language: &str| {
            let Outcome::Found { lyrics, how } = words(ProviderId::LrcRed) else { panic!() };
            let mut l = (*lyrics).clone();
            for (line, text) in l.lines.iter_mut().zip(["真夜中のドアをたたき", "Stay with me", "帰らないでと泣いた"]) {
                line.text = text.into();
                line.tokens.clear();
            }
            l.language = Some(language.into());
            Outcome::Found { lyrics: Arc::new(l), how }
        };
        let language = |o: Outcome| match combine(None, &[(ProviderId::LrcRed, o)], &[ProviderId::LrcRed]).outcome {
            Outcome::Found { lyrics, .. } => lyrics.language.clone(),
            o => panic!("{o:?}"),
        };
        assert_eq!(language(japanese("en")).as_deref(), Some("ja"), "lrc.red's \"en\" for kana");
        assert_eq!(language(japanese("ja")).as_deref(), Some("ja"));
        assert_eq!(language(words(ProviderId::LrcRed)), None, "none given: the translator works it out");
    }

    /// The flow: line lyrics show at once, word timing replaces them when it
    /// arrives, and the last answer is final.
    #[test]
    fn line_lyrics_first_then_the_upgrade() {
        let plan = plan(ProviderSetting::Auto, true, &DEFAULT_SOURCES);
        let mut sent: Vec<(ProviderId, Timing, bool)> = Vec::new();
        block(lookup(
            &plan,
            None,
            all_available,
            |p| async move {
                match p {
                    ProviderId::Lrclib => found(p),
                    ProviderId::LrcRed => {
                        tokio::time::sleep(Duration::from_millis(80)).await;
                        words(p)
                    }
                    _ => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Outcome::NotFound { provider: p }
                    }
                }
            },
            |r| {
                let Outcome::Found { lyrics, .. } = &r.outcome else { panic!("{r:?}") };
                sent.push((lyrics.source.provider, lyrics.timing, r.pending));
            },
        ));
        assert_eq!(sent, vec![(ProviderId::Lrclib, Timing::Line, true), (ProviderId::LrcRed, Timing::Word, false)], "the upgrade is the last answer, so it comes as the final one");
    }

    #[test]
    fn nothing_found_waits_a_little_for_the_word_services() {
        let plan = plan(ProviderSetting::Auto, false, &[ProviderId::NetEase]);
        let mut sent: Vec<(&'static str, bool)> = Vec::new();
        let started = std::time::Instant::now();
        block(lookup(
            &plan,
            None,
            all_available,
            |p| async move {
                if p == ProviderId::NetEase {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                Outcome::NotFound { provider: p }
            },
            |r| sent.push((r.outcome.short(), r.pending)),
        ));
        assert_eq!(sent, vec![("no match", false)], "one answer, after everyone has spoken");
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    fn request(key: &str) -> Request {
        Request {
            track: TrackRef { key: key.into(), title: "t".into(), artists: vec!["a".into()], duration_ms: 1, ..Default::default() },
            setting: ProviderSetting::Auto,
            fallback: true,
            sources: DEFAULT_SOURCES.to_vec(),
            nonce: 0,
        }
    }

    fn done(p: ProviderId) -> Resolved {
        Resolved::new(found(p), vec![(p, "found")])
    }

    #[test]
    fn stale_answers_are_ignored() {
        let mut s = LyricsState::default();
        let a = s.begin(request("spotify:track:a"));
        let b = s.begin(request("spotify:track:b"));
        assert!(!s.finish(a, done(ProviderId::Lrclib)), "the previous song's answer is dropped");
        assert!(!s.finish(a, Resolved { pending: true, ..done(ProviderId::LrcRed) }), "…early answers too");
        assert_eq!(s.status, Status::Loading);
        assert!(s.finish(b, Resolved { pending: true, ..done(ProviderId::Lrclib) }));
        assert!(s.finish(b, done(ProviderId::LrcRed)), "an upgrade for the same song lands");
        assert!(matches!(&s.status, Status::Done(r) if r.outcome.provider() == ProviderId::LrcRed));
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
        assert!(s.wants(&Request { setting: ProviderSetting::Only(ProviderId::Lrclib), ..request("spotify:track:a") }), "changing the provider asks again");
        assert!(s.wants(&Request { sources: vec![], ..request("spotify:track:a") }), "changing the word services asks again");
        assert!(s.wants(&Request { nonce: 1, ..request("spotify:track:a") }), "retry asks again");
    }

    /// The whole flow against the real services (opt-in, needs the network):
    /// `cargo test live_lookup -- --ignored --nocapture`. Prints every answer as
    /// it arrives, so the line-then-word upgrade shows.
    #[test]
    #[ignore]
    fn live_lookup() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let cache = Arc::new(Cache::in_memory());
        let sources = Sources::new("", "", None, cache.clone(), None);
        let t = |title: &str, artist: &str, album: &str, ms: u64, isrc: &str| TrackRef {
            key: format!("live:{title}"),
            title: title.into(),
            artists: vec![artist.into()],
            album: album.into(),
            duration_ms: ms,
            isrc: isrc.into(),
        };
        let cases = [
            ("word timing by ISRC", t("Blinding Lights", "The Weeknd", "After Hours", 200_040, "USUG11904206"), Some(Timing::Word)),
            ("word timing by search", t("Espresso", "Sabrina Carpenter", "Short n' Sweet", 175_459, ""), Some(Timing::Word)),
            ("line timing only", t("I Want to Live", "Borislav Slavov", "Baldur's Gate 3 (Original Game Soundtrack)", 233_000, ""), Some(Timing::Line)),
            ("no lyrics anywhere", t("Glass Test Track That Does Not Exist", "Nobody In Particular", "", 123_000, ""), None),
        ];
        for (name, track, want) in cases {
            let started = std::time::Instant::now();
            let mut seen: Vec<String> = Vec::new();
            let mut last: Option<Resolved> = None;
            rt.block_on(run(sources.clone(), ProviderSetting::Auto, false, DEFAULT_SOURCES.to_vec(), track, None, None, |r| {
                let what = match &r.outcome {
                    Outcome::Found { lyrics, how } => format!("{} {} ({how:?}, {} lines)", lyrics.source.provider.label(), lyrics.timing.label(), lyrics.lines.len()),
                    o => o.short().to_string(),
                };
                seen.push(format!("{:>5} ms{} {what}", started.elapsed().as_millis(), if r.pending { " (more coming)" } else { "" }));
                last = Some(r);
            }));
            let last = last.unwrap();
            println!("{name}:\n  {}\n  asked: {:?}", seen.join("\n  "), last.tried);
            assert!(!last.pending, "{name}: the last answer is final");
            match (want, &last.outcome) {
                (Some(Timing::Word), Outcome::Found { lyrics, .. }) => assert!(lyrics.timing >= Timing::Word, "{name}"),
                (Some(Timing::Line), Outcome::Found { lyrics, .. }) => assert_eq!(lyrics.timing, Timing::Line, "{name}"),
                (None, o) => assert!(!matches!(o, Outcome::Found { .. }), "{name}"),
                (_, o) => panic!("{name}: {o:?}"),
            }
            // LRCLIB asks for requests to be spread out.
            std::thread::sleep(Duration::from_secs(2));
        }
        // A second play of a song is answered from the cache: no requests, at once.
        let started = std::time::Instant::now();
        let mut answers = 0;
        rt.block_on(run(sources.clone(), ProviderSetting::Auto, false, DEFAULT_SOURCES.to_vec(), t("Blinding Lights", "The Weeknd", "After Hours", 200_040, "USUG11904206"), None, None, |_| {
            answers += 1
        }));
        println!("again, from the cache: {answers} answer(s) in {} ms", started.elapsed().as_millis());
        assert!(started.elapsed() < Duration::from_millis(200));
        // A service that can't be reached: the others still answer.
        let broken = Sources::new("", "", None, Arc::new(Cache::in_memory()), None);
        let mut b = broken;
        Arc::get_mut(&mut b).unwrap().bases.insert(ProviderId::LrcRed, "http://127.0.0.1:9".into());
        let mut last = None;
        rt.block_on(run(b, ProviderSetting::Auto, false, DEFAULT_SOURCES.to_vec(), t("Blinding Lights", "The Weeknd", "After Hours", 200_040, "USUG11904206"), None, None, |r| last = Some(r)));
        let last = last.unwrap();
        println!(
            "lrc.red down: {:?} → {:?}",
            last.tried,
            match &last.outcome {
                Outcome::Found { lyrics, .. } => format!("{} {}", lyrics.source.provider.label(), lyrics.timing.label()),
                o => o.short().into(),
            }
        );
        assert!(last.tried.contains(&(ProviderId::LrcRed, "failed")));
        assert!(matches!(&last.outcome, Outcome::Found { lyrics, .. } if lyrics.timing >= Timing::Word), "Unison or NetEase still gives word timing");
    }

    /// What each word service says about a list of songs (opt-in, needs the
    /// network): `GLASS_LYRICS_SURVEY=<file> cargo test live_survey --
    /// --ignored --nocapture`, the file holding `title⇥artist⇥album⇥ms[⇥isrc]`
    /// rows. Shows how much of someone's listening gets word timing.
    #[test]
    #[ignore]
    fn live_survey() {
        let Ok(path) = std::env::var("GLASS_LYRICS_SURVEY") else { return };
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let sources = Sources::new("", "", None, Arc::new(Cache::in_memory()), None);
        let mut words = 0;
        let rows: Vec<Vec<String>> = std::fs::read_to_string(path).unwrap().lines().map(|l| l.split('\t').map(str::to_string).collect()).filter(|r: &Vec<String>| r.len() >= 4).collect();
        for r in &rows {
            let t = TrackRef {
                key: format!("live:{}", r[0]),
                title: r[0].clone(),
                artists: r[1].split(", ").map(str::to_string).collect(),
                album: r[2].clone(),
                duration_ms: r[3].parse().unwrap_or(0),
                isrc: r.get(4).cloned().unwrap_or_default(),
            };
            // As the app does: LRCLIB's line timing and every word service, then the best that keeps time.
            let line = rt.block_on(resolve(&[ProviderId::Lrclib], |_| Ok(()), |p| sources.fetch(p, &t, None)));
            let answers: Vec<(ProviderId, Outcome)> = DEFAULT_SOURCES.iter().map(|&p| (p, rt.block_on(sources.fetch(p, &t, None)))).collect();
            let best = combine(Some(&line), &answers, &DEFAULT_SOURCES);
            let reference = super::found(&line.outcome).filter(|l| l.timing >= Timing::Line);
            let said: Vec<String> = answers
                .iter()
                .map(|(p, o)| match super::found(o) {
                    Some(l) => {
                        let off = reference.and_then(|r| agree::offset(l, r)).map(|d| format!("{:+.1} s", d as f64 / 1000.0)).unwrap_or_else(|| "?".into());
                        format!("{} {} ({} · {} · {}, {}), {off} vs LRCLIB", p.label(), l.timing.label(), l.source.artist, l.source.album, l.source.duration_ms, l.language.as_deref().unwrap_or("-"))
                    }
                    None => format!("{} {}", p.label(), o.short()),
                })
                .collect();
            let shown = match super::found(&best.outcome) {
                Some(l) => format!("{} {}, language {}", l.source.provider.label(), l.timing.label(), l.language.as_deref().unwrap_or("not given")),
                None => best.outcome.short().to_string(),
            };
            if super::found(&best.outcome).is_some_and(|l| l.timing >= Timing::Word) {
                words += 1;
            }
            println!("{} — {}: {shown}\n  {}", r[0], r[1], said.join("\n  "));
            // Qobuzify answers 15 requests a minute without a key.
            std::thread::sleep(Duration::from_millis(4_200));
        }
        println!("word timing for {words} of {} songs", rows.len());
    }

    /// Rapid switching through songs over real HTTP: every lookup but the
    /// last is aborted mid-request, and only the last song's answer lands.
    #[test]
    fn rapid_switching_lands_only_the_last_song() {
        use crate::lyrics::http::tests::reply;
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<(u64, Resolved)>();
        let mut state = LyricsState::default();
        let mut last = 0;
        rt.block_on(async {
            // A server that answers slowly: whatever's asked gets "not found" after a while.
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let Ok((mut sock, _)) = listener.accept().await else { break };
                    tokio::spawn(async move {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        let mut buf = vec![0u8; 4096];
                        let _ = sock.read(&mut buf).await;
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        let _ = sock.write_all(reply("404 Not Found", "application/json", "", "{}").as_bytes()).await;
                    });
                }
            });
            let sources = Sources::at(&format!("http://{addr}"), Arc::new(Cache::in_memory()));
            for i in 0..5 {
                let r = request(&format!("spotify:track:{i}"));
                let generation = state.begin(r.clone());
                last = generation;
                let (tx, sources) = (tx.clone(), sources.clone());
                let task = tokio::spawn(run(sources, r.setting, false, vec![ProviderId::LrcRed], r.track, None, None, move |res| {
                    let _ = tx.send((generation, res));
                }));
                state.running(task.abort_handle());
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        });
        let got: Vec<(u64, Resolved)> = rx.try_iter().collect();
        assert!(!got.is_empty(), "the last song got its answer");
        assert!(got.iter().all(|(g, _)| *g == last), "the songs skipped past were cancelled before answering");
        for (generation, res) in got {
            assert!(state.finish(generation, res));
        }
        assert_eq!(state.track_key(), Some("spotify:track:4"));
        assert!(matches!(&state.status, Status::Done(r) if !r.pending));
    }
}
