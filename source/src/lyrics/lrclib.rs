//! LRCLIB (<https://lrclib.net>): a free, community-maintained lyrics
//! database with an open API and no keys.
//!
//! Its documentation (lrclib.net/docs) asks clients to identify themselves in
//! the User-Agent, send requests one at a time with a short pause between
//! them, and honor `429`/`503` with `Retry-After` — the body of those isn't
//! guaranteed to be JSON. This client does all of that: requests go through
//! one gate, at most three attempts each, and a long `Retry-After` pauses
//! every LRCLIB request rather than being retried right away.

use super::matching::{self, Choice};
use super::parse::{self, Record};
use super::{Match, Outcome, ProviderId, TrackRef};
use crate::api::{LogEntry, NetLog};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

pub const BASE: &str = "https://lrclib.net";
/// Pause between requests (the docs suggest 200–500 ms).
const GAP: Duration = Duration::from_millis(300);
const TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const ATTEMPTS: u32 = 3;
/// A longer `Retry-After` isn't waited out inside one lookup.
const MAX_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// LRCLIB asked us to wait this long.
    RateLimited(Duration),
    Http(u16, String),
    Network(String),
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::RateLimited(d) => write!(f, "LRCLIB is busy; try again in {} s", d.as_secs().max(1)),
            Error::Http(code, msg) if msg.is_empty() => write!(f, "LRCLIB answered {code}"),
            Error::Http(code, msg) => write!(f, "LRCLIB answered {code}: {msg}"),
            Error::Network(e) => write!(f, "couldn't reach LRCLIB ({e})"),
            Error::Malformed(e) => write!(f, "LRCLIB sent something unreadable ({e})"),
        }
    }
}

/// What to do after a failed attempt.
#[derive(Debug, PartialEq)]
pub enum Step {
    Retry(Duration),
    Fail(Error),
}

/// `Retry-After` in seconds. (LRCLIB sends seconds; an HTTP date isn't
/// understood and falls back to the default.)
fn retry_after(header: Option<&str>) -> Option<Duration> {
    header?.trim().parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0).map(|s| Duration::from_secs_f64(s.min(3600.0)))
}

/// The retry policy. `status` is `None` for network errors and timeouts;
/// `attempt` counts from 0.
pub fn after_failure(status: Option<u16>, retry_header: Option<&str>, message: &str, attempt: u32) -> Step {
    let last = attempt + 1 >= ATTEMPTS;
    match status {
        Some(429) | Some(503) => {
            let wait = retry_after(retry_header).unwrap_or(if status == Some(503) { Duration::from_secs(1) } else { Duration::from_secs(2) });
            if last || wait > MAX_WAIT { Step::Fail(Error::RateLimited(wait)) } else { Step::Retry(wait) }
        }
        Some(500) | Some(502) | Some(504) | None => {
            if last {
                Step::Fail(match status {
                    Some(code) => Error::Http(code, message.to_string()),
                    None => Error::Network(message.to_string()),
                })
            } else {
                Step::Retry(Duration::from_secs(1 << attempt))
            }
        }
        Some(code) => Step::Fail(Error::Http(code, message.to_string())),
    }
}

/// `Glass/<version> (<contact>)`, as LRCLIB asks: the contact setting, or else
/// the crate's homepage, or else "desktop music player".
pub fn user_agent(contact: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let contact: String = contact.chars().filter(|c| !c.is_control()).take(200).collect();
    let contact = contact.trim();
    let home = [env!("CARGO_PKG_HOMEPAGE"), env!("CARGO_PKG_REPOSITORY")].into_iter().find(|s| !s.is_empty());
    match (contact.is_empty(), home) {
        (false, _) => format!("Glass/{version} ({contact})"),
        (true, Some(h)) => format!("Glass/{version} ({h})"),
        #[cfg(glass_mobile)]
        (true, None) => format!("Glass/{version} (iPhone music player)"),
        #[cfg(not(glass_mobile))]
        (true, None) => format!("Glass/{version} (desktop music player)"),
    }
}

struct Gate {
    last: Option<Instant>,
    blocked_until: Option<Instant>,
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    gate: tokio::sync::Mutex<Gate>,
    log: Option<NetLog>,
}

impl Client {
    pub fn new(contact: &str, proxy: &str, log: Option<NetLog>) -> Arc<Self> {
        Self::with_base(BASE, contact, proxy, log)
    }

    pub fn with_base(base: &str, contact: &str, proxy: &str, log: Option<NetLog>) -> Arc<Self> {
        let mut b = reqwest::Client::builder().user_agent(user_agent(contact)).timeout(TIMEOUT).connect_timeout(CONNECT_TIMEOUT);
        if !proxy.trim().is_empty()
            && let Ok(p) = reqwest::Proxy::all(proxy.trim())
        {
            b = b.proxy(p);
        }
        let http = b.build().unwrap_or_default();
        Arc::new(Self { http, base: base.trim_end_matches('/').to_string(), gate: tokio::sync::Mutex::new(Gate { last: None, blocked_until: None }), log })
    }

    fn note(&self, path: &str, status: String, started: Instant) {
        if let Some(log) = &self.log {
            log.push(LogEntry { at: SystemTime::now(), method: "GET".into(), path: format!("lrclib.net{path}"), status, millis: started.elapsed().as_millis() });
        }
    }

    /// GET `path` with `query`. `Ok(None)` for 404 (no such record).
    pub async fn get_json(&self, path: &str, query: &[(&str, String)]) -> Result<Option<Value>, Error> {
        // One request at a time, with a pause between them.
        let mut gate = self.gate.lock().await;
        let url = format!("{}{path}", self.base);
        let shown = if query.is_empty() { path.to_string() } else { format!("{path}?{}", query.iter().map(|(k, v)| format!("{k}={}", crate::api::enc(v))).collect::<Vec<_>>().join("&")) };
        let mut attempt = 0;
        loop {
            let now = Instant::now();
            if let Some(until) = gate.blocked_until.filter(|u| *u > now) {
                let wait = until - now;
                if wait > MAX_WAIT {
                    return Err(Error::RateLimited(wait));
                }
                tokio::time::sleep(wait).await;
            }
            if let Some(last) = gate.last {
                let since = last.elapsed();
                if since < GAP {
                    tokio::time::sleep(GAP - since).await;
                }
            }
            let started = Instant::now();
            gate.last = Some(started);
            let (status, header, message) = match self.http.get(&url).query(query).send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let header = resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|h| h.to_str().ok()).map(str::to_string);
                    let json = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|h| h.to_str().ok()).is_some_and(|c| c.contains("json"));
                    self.note(&shown, status.to_string(), started);
                    match status {
                        200 if json => return resp.json::<Value>().await.map(Some).map_err(|e| Error::Malformed(e.to_string())),
                        200 => return Err(Error::Malformed("not JSON".into())),
                        404 => return Ok(None),
                        _ => {}
                    }
                    // Error bodies are JSON with a message, except rate limits.
                    let message = if json && status != 429 { resp.json::<Value>().await.ok().and_then(|v| v["message"].as_str().map(str::to_string)).unwrap_or_default() } else { String::new() };
                    (Some(status), header, message)
                }
                Err(e) => {
                    self.note(&shown, if e.is_timeout() { "timeout".into() } else { "error".into() }, started);
                    (None, None, if e.is_timeout() { "timed out".to_string() } else { e.without_url().to_string() })
                }
            };
            let step = after_failure(status, header.as_deref(), &message, attempt);
            if matches!(status, Some(429) | Some(503)) {
                let wait = match &step {
                    Step::Retry(d) => *d,
                    Step::Fail(Error::RateLimited(d)) => *d,
                    _ => Duration::ZERO,
                };
                gate.blocked_until = Some(Instant::now() + wait);
            }
            match step {
                Step::Retry(d) => {
                    log::debug!("lrclib {path}: retrying in {d:?} (status {status:?})");
                    tokio::time::sleep(d).await;
                    attempt += 1;
                }
                Step::Fail(e) => return Err(e),
            }
        }
    }

    pub async fn search(&self, query: &[(&str, String)]) -> Result<Vec<Record>, Error> {
        match self.get_json("/api/search", query).await? {
            Some(v) => parse::records(&v).map_err(Error::Malformed),
            None => Ok(Vec::new()),
        }
    }

    /// Free-text search for the manual picker.
    pub async fn search_text(&self, q: &str) -> Result<Vec<Record>, Error> {
        self.search(&[("q", q.trim().to_string())]).await
    }

    /// Finds lyrics for `t`: first LRCLIB's own exact match on title, artist,
    /// album and duration, then (only if that has nothing) a search checked
    /// by Glass's matching. At most three lookups, each tried up to three times.
    pub async fn lookup(&self, t: &TrackRef) -> Result<Lookup, Error> {
        if t.title.trim().is_empty() || t.artist().trim().is_empty() {
            return Ok(Lookup::None);
        }
        let mut q = vec![("track_name", t.title.clone()), ("artist_name", t.artist().to_string())];
        if !t.album.is_empty() {
            q.push(("album_name", t.album.clone()));
        }
        let secs = (t.duration_ms as f64 / 1000.0).round() as u64;
        if (1..=3600).contains(&secs) {
            q.push(("duration", secs.to_string()));
        }
        if let Some(v) = self.get_json("/api/get", &q).await? {
            let r = parse::record(&v).map_err(Error::Malformed)?;
            // LRCLIB matched it, but a live or edited version must not pass.
            if matching::score(t, &r).is_some() {
                return Ok(Lookup::Found(r, Match::Exact));
            }
        }
        // Search: structured first, then free text, stopping at the first hit.
        let title = matching::search_title(&t.title);
        let attempts = [vec![("track_name", title.clone()), ("artist_name", t.artist().to_string())], vec![("q", format!("{title} {}", t.artist()))]];
        for query in attempts {
            let found = self.search(&query).await?;
            match matching::choose(t, &found) {
                Choice::Best(i) => return Ok(Lookup::Found(found[i].clone(), Match::Search)),
                Choice::Ambiguous(ix) => return Ok(Lookup::Ambiguous(ix.into_iter().map(|i| found[i].clone()).collect())),
                Choice::None => {}
            }
        }
        Ok(Lookup::None)
    }
}

/// What a lookup found, before it's turned into lyrics.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    Found(Record, Match),
    Ambiguous(Vec<Record>),
    None,
}

impl Lookup {
    pub fn outcome(&self) -> Outcome {
        match self {
            Lookup::Found(r, how) => record_outcome(r, *how),
            Lookup::Ambiguous(rs) => Outcome::Ambiguous { provider: ProviderId::Lrclib, candidates: rs.iter().map(Record::candidate).collect() },
            Lookup::None => Outcome::NotFound { provider: ProviderId::Lrclib },
        }
    }
}

pub fn record_outcome(r: &Record, how: Match) -> Outcome {
    match r.lyrics() {
        Some(l) => Outcome::Found { lyrics: Arc::new(l), how },
        None if r.instrumental => Outcome::Instrumental { source: r.source() },
        // A record with nothing in it is as good as no record.
        None => Outcome::NotFound { provider: ProviderId::Lrclib },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn retry_policy() {
        assert_eq!(after_failure(Some(429), Some("1"), "", 0), Step::Retry(Duration::from_secs(1)), "Retry-After is honored");
        assert_eq!(after_failure(Some(503), None, "", 0), Step::Retry(Duration::from_secs(1)));
        assert_eq!(after_failure(Some(429), Some("120"), "", 0), Step::Fail(Error::RateLimited(Duration::from_secs(120))), "a long wait isn't sat out");
        assert_eq!(after_failure(Some(429), Some("1"), "", 2), Step::Fail(Error::RateLimited(Duration::from_secs(1))), "bounded");
        assert_eq!(after_failure(None, None, "timed out", 0), Step::Retry(Duration::from_secs(1)));
        assert_eq!(after_failure(None, None, "timed out", 1), Step::Retry(Duration::from_secs(2)));
        assert_eq!(after_failure(None, None, "timed out", 2), Step::Fail(Error::Network("timed out".into())));
        assert_eq!(
            after_failure(Some(400), None, "duration: must be between 1 and 3600", 0),
            Step::Fail(Error::Http(400, "duration: must be between 1 and 3600".into())),
            "client errors aren't retried"
        );
        assert_eq!(retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")), None);
    }

    #[test]
    fn user_agent_names_the_app() {
        assert!(user_agent("").starts_with(&format!("Glass/{} (", env!("CARGO_PKG_VERSION"))));
        assert_eq!(user_agent("https://example.com\r\nX-Evil: 1"), format!("Glass/{} (https://example.comX-Evil: 1)", env!("CARGO_PKG_VERSION")), "no header injection");
    }

    /// A one-connection-per-response HTTP server on localhost.
    async fn serve(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            for r in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                seen.push(String::from_utf8_lossy(&buf[..n]).to_string());
                sock.write_all(r.as_bytes()).await.unwrap();
                sock.shutdown().await.ok();
            }
            seen
        });
        (format!("http://{addr}"), task)
    }

    fn reply(status: &str, content_type: &str, extra: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{extra}Connection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    fn busy() -> String {
        reply("429 Too Many Requests", "text/html", "Retry-After: 1\r\n", "<b>no</b>")
    }
    fn ok() -> String {
        reply("200 OK", "application/json", "", r#"{"id":5,"trackName":"Song","artistName":"Artist","albumName":"","duration":200,"syncedLyrics":"[00:01.00]hi"}"#)
    }
    fn missing() -> String {
        reply("404 Not Found", "application/json", "", r#"{"message":"Failed to find specified track","name":"TrackNotFound","statusCode":404}"#)
    }

    #[tokio::test]
    async fn rate_limits_are_waited_out_and_the_app_is_named() {
        let (base, server) = serve(vec![busy(), ok()]).await;
        let c = Client::with_base(&base, "", "", None);
        let started = Instant::now();
        let v = c.get_json("/api/get/5", &[]).await.unwrap().unwrap();
        assert_eq!(parse::record(&v).unwrap().track_name, "Song");
        assert!(started.elapsed() >= Duration::from_millis(950), "waited for Retry-After");
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].to_lowercase().contains("user-agent: glass/"), "{}", seen[0]);
    }

    #[tokio::test]
    async fn missing_and_unreadable_responses() {
        let (base, _s) = serve(vec![missing(), reply("200 OK", "text/plain", "", "ok")]).await;
        let c = Client::with_base(&base, "", "", None);
        assert_eq!(c.get_json("/api/get/1", &[]).await.unwrap(), None);
        assert!(matches!(c.get_json("/api/get/2", &[]).await, Err(Error::Malformed(_))), "a 200 that isn't JSON is refused");
    }

    #[tokio::test]
    async fn requests_are_spaced_out() {
        let (base, _s) = serve(vec![missing(), missing()]).await;
        let c = Client::with_base(&base, "", "", None);
        let started = Instant::now();
        c.get_json("/api/get/1", &[]).await.unwrap();
        c.get_json("/api/get/2", &[]).await.unwrap();
        assert!(started.elapsed() >= GAP);
    }

    #[tokio::test]
    async fn network_failures_are_reported() {
        // Nothing listens on this port.
        let c = Client::with_base("http://127.0.0.1:9", "", "", None);
        let e = c.get_json("/api/get/1", &[]).await.unwrap_err();
        assert!(matches!(e, Error::Network(_)), "{e:?}");
    }

    /// Real lookups on lrclib.net (opt-in, needs the network):
    /// `cargo test --release live_lrclib -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_lrclib() {
        use crate::lyrics::Timing;
        let c = Client::new("", "", None);
        let t = |title: &str, artist: &str, album: &str, secs: u64| TrackRef {
            key: format!("live:{title}:{artist}"),
            title: title.into(),
            artists: vec![artist.into()],
            album: album.into(),
            duration_ms: secs * 1000,
        };
        let cases = [
            ("line-synced", t("I Want to Live", "Borislav Slavov", "Baldur's Gate 3 (Original Game Soundtrack)", 233), "line"),
            ("Spotify-style remaster title", t("Bohemian Rhapsody - Remastered 2011", "Queen", "A Night At The Opera (2011 Remaster)", 355), "line"),
            ("plain only", t("Bad Guy", "Filter", "Bad Guy", 199), "plain"),
            ("instrumental", t("Levitating", "Vitamin String Quartet", "Levitating", 195), "instrumental"),
            ("live version of a studio song", t("Bohemian Rhapsody - Live Aid", "Queen", "Live Aid", 358), "any"),
            ("no lyrics anywhere", t("Glass Test Track That Does Not Exist", "Nobody In Particular", "", 123), "none"),
        ];
        for (name, track, want) in cases {
            // Paced like songs going by: LRCLIB rate-limits a burst of lookups.
            tokio::time::sleep(Duration::from_secs(3)).await;
            let l = c.lookup(&track).await.expect("lookup");
            let got = match l.outcome() {
                Outcome::Found { lyrics, how } => {
                    println!(
                        "{name}: {} via {how:?} — record {} “{}” / {} / {} ({} lines)",
                        lyrics.timing.label(),
                        lyrics.source.id,
                        lyrics.source.title,
                        lyrics.source.artist,
                        lyrics.source.album,
                        lyrics.lines.len()
                    );
                    match lyrics.timing {
                        Timing::Plain => "plain",
                        Timing::Line => "line",
                        _ => "word",
                    }
                }
                Outcome::Instrumental { source } => {
                    println!("{name}: instrumental — record {}", source.id);
                    "instrumental"
                }
                Outcome::Ambiguous { candidates, .. } => {
                    println!("{name}: {} candidates", candidates.len());
                    "ambiguous"
                }
                o => {
                    println!("{name}: {}", o.short());
                    "none"
                }
            };
            if want != "any" {
                assert_eq!(got, want, "{name}");
            }
        }
        // Word timing: look for any word-synced record in a few searches.
        let mut word = None;
        for q in ["still alive portal", "bohemian rhapsody queen", "blinding lights the weeknd", "shape of you ed sheeran", "hello adele"] {
            if let Some(r) = c.search_text(q).await.unwrap_or_default().into_iter().find(|r| r.has_word_sync) {
                word = Some(r);
                break;
            }
        }
        match word.and_then(|r| r.lyrics().map(|l| (r, l))) {
            Some((r, l)) => {
                println!("word sync: record {} “{}” reads as {}", r.id, r.track_name, l.timing.label());
                assert!(l.timing >= Timing::Line);
            }
            None => println!("word sync: no record with hasWordSync in these searches"),
        }
    }

    #[test]
    fn outcomes() {
        let r = parse::record(&serde_json::json!({ "id": 1, "trackName": "T", "artistName": "A", "duration": 100, "instrumental": true })).unwrap();
        assert!(matches!(record_outcome(&r, Match::Exact), Outcome::Instrumental { .. }));
        let r = parse::record(&serde_json::json!({ "id": 2, "trackName": "T", "artistName": "A", "duration": 100 })).unwrap();
        assert_eq!(record_outcome(&r, Match::Exact), Outcome::NotFound { provider: ProviderId::Lrclib });
    }
}
