//! Requests to the word-timing lyrics services: bounded in time and size,
//! named in the User-Agent, and polite when a service is busy or the network
//! is gone. A `429`/`503` (or a run of network failures) pauses that service
//! for a while instead of retrying, so skipping through songs offline or
//! while rate limited costs nothing and asks nobody.

use crate::api::{LogEntry, NetLog};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const TIMEOUT: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Lyrics are a few kilobytes; a TTML with every syllable is under 200 KB.
const MAX_BODY: usize = 4 << 20;
/// Between two requests to the same service.
const GAP: Duration = Duration::from_millis(150);
/// A rate limit without Retry-After, and the longest one honored.
const DEFAULT_WAIT: Duration = Duration::from_secs(30);
const MAX_WAIT: Duration = Duration::from_secs(15 * 60);
/// After the network failed twice in a row.
const OFFLINE_WAIT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The service asked to wait (or is resting after failures).
    Busy(Duration),
    Http(u16),
    Network(String),
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Busy(d) => write!(f, "busy; try again in {} s", d.as_secs().max(1)),
            Error::Http(code) => write!(f, "answered {code}"),
            Error::Network(e) => write!(f, "couldn't reach it ({e})"),
            Error::Malformed(e) => write!(f, "sent something unreadable ({e})"),
        }
    }
}

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Result<serde_json::Value, Error> {
        serde_json::from_slice(&self.body).map_err(|e| Error::Malformed(e.to_string()))
    }
    pub fn text(&self) -> Result<String, Error> {
        String::from_utf8(self.body.clone()).map_err(|_| Error::Malformed("not UTF-8".into()))
    }
}

#[derive(Default)]
struct State {
    last: Option<Instant>,
    paused_until: Option<Instant>,
    failures: u32,
}

/// One service's client.
pub struct Http {
    http: reqwest::Client,
    /// Shown in the Activity page's request list ("lrc.red").
    host: &'static str,
    log: Option<NetLog>,
    state: tokio::sync::Mutex<State>,
    /// Mirrors `state.paused_until` for a quick check without waiting on the lock.
    paused: Mutex<Option<Instant>>,
}

/// `Glass/<version> (<contact>; <homepage>)`: services ask clients to name
/// themselves and say how to reach whoever runs them.
pub fn user_agent(contact: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let contact: String = contact.chars().filter(|c| !c.is_control()).take(200).collect();
    let contact = contact.trim();
    if contact.is_empty() { format!("Glass/{version} ({HOMEPAGE})") } else { format!("Glass/{version} ({contact}; {HOMEPAGE})") }
}

/// Where Glass is published.
pub const HOMEPAGE: &str = "https://github.com/davishann15-commits/glass-releases";

impl Http {
    pub fn new(host: &'static str, contact: &str, proxy: &str, log: Option<NetLog>) -> Self {
        let mut b = reqwest::Client::builder().user_agent(user_agent(contact)).timeout(TIMEOUT).connect_timeout(CONNECT_TIMEOUT);
        if !proxy.trim().is_empty()
            && let Ok(p) = reqwest::Proxy::all(proxy.trim())
        {
            b = b.proxy(p);
        }
        Self { http: b.build().unwrap_or_default(), host, log, state: tokio::sync::Mutex::new(State::default()), paused: Mutex::new(None) }
    }

    /// How long the service is still resting, if it is.
    pub fn paused_for(&self) -> Option<Duration> {
        self.paused.lock().unwrap().and_then(|u| u.checked_duration_since(Instant::now()))
    }

    fn note(&self, shown: &str, status: String, started: Instant) {
        if let Some(log) = &self.log {
            log.push(LogEntry { at: SystemTime::now(), method: "GET".into(), path: format!("{}{shown}", self.host), status, millis: started.elapsed().as_millis() });
        }
    }

    /// GET `url` with `query` and `headers`. Any status comes back as a
    /// `Reply` except rate limits and server errors, which are `Err`s. One
    /// retry for a server error or a dropped connection; none for the rest.
    pub async fn get(&self, url: &str, query: &[(&str, String)], headers: &[(&str, String)]) -> Result<Reply, Error> {
        if let Some(d) = self.paused_for() {
            return Err(Error::Busy(d));
        }
        let mut state = self.state.lock().await;
        let shown = url.split_once("://").map(|(_, rest)| rest.split_once('/').map(|(_, p)| format!("/{p}")).unwrap_or_default()).unwrap_or_default();
        let mut attempt = 0;
        loop {
            if let Some(last) = state.last
                && last.elapsed() < GAP
            {
                tokio::time::sleep(GAP - last.elapsed()).await;
            }
            let started = Instant::now();
            state.last = Some(started);
            let mut req = self.http.get(url).query(query);
            for (k, v) in headers {
                req = req.header(*k, v);
            }
            let outcome: Result<(), Error> = match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    self.note(&shown, status.to_string(), started);
                    let retry = resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|h| h.to_str().ok()).and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s >= 0.0);
                    if status == 429 || status == 503 {
                        let wait = retry.map(|s| Duration::from_secs_f64(s.min(MAX_WAIT.as_secs_f64()))).unwrap_or(DEFAULT_WAIT).max(Duration::from_secs(1));
                        self.pause(&mut state, wait);
                        return Err(Error::Busy(wait));
                    }
                    if status >= 500 {
                        Err(Error::Http(status))
                    } else {
                        let body = read_capped(resp).await;
                        match body {
                            Ok(body) => {
                                state.failures = 0;
                                return Ok(Reply { status, body });
                            }
                            Err(e) => Err(e),
                        }
                    }
                }
                Err(e) => {
                    self.note(&shown, if e.is_timeout() { "timeout".into() } else { "error".into() }, started);
                    Err(Error::Network(if e.is_timeout() { "timed out".into() } else { e.without_url().to_string() }))
                }
            };
            let err = outcome.unwrap_err();
            let again = matches!(err, Error::Network(_) | Error::Http(500 | 502 | 504)) && attempt == 0;
            if again {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(600)).await;
                continue;
            }
            if matches!(err, Error::Network(_)) {
                state.failures += 1;
                if state.failures >= 2 {
                    self.pause(&mut state, OFFLINE_WAIT);
                }
            }
            return Err(err);
        }
    }

    fn pause(&self, state: &mut State, wait: Duration) {
        let until = Instant::now() + wait.min(MAX_WAIT);
        state.paused_until = Some(until);
        *self.paused.lock().unwrap() = Some(until);
    }
}

async fn read_capped(mut resp: reqwest::Response) -> Result<Vec<u8>, Error> {
    if resp.content_length().is_some_and(|n| n as usize > MAX_BODY) {
        return Err(Error::Malformed("too large".into()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| Error::Network(e.without_url().to_string()))? {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(Error::Malformed("too large".into()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-connection-per-response HTTP server on localhost; answers with
    /// `responses` in turn and hands back what it was asked.
    pub async fn serve(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            for r in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap();
                seen.push(String::from_utf8_lossy(&buf[..n]).to_string());
                sock.write_all(r.as_bytes()).await.unwrap();
                sock.shutdown().await.ok();
            }
            seen
        });
        (format!("http://{addr}"), task)
    }

    pub fn reply(status: &str, content_type: &str, extra: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{extra}Connection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    #[tokio::test]
    async fn rate_limits_pause_the_service() {
        let (base, server) = serve(vec![reply("429 Too Many Requests", "text/plain", "Retry-After: 120\r\n", "slow down")]).await;
        let h = Http::new("test", "", "", None);
        assert_eq!(h.get(&format!("{base}/a"), &[], &[]).await.err(), Some(Error::Busy(Duration::from_secs(120))));
        // Paused: the next request isn't even sent.
        assert!(matches!(h.get(&format!("{base}/b"), &[], &[]).await, Err(Error::Busy(_))));
        assert_eq!(server.await.unwrap().len(), 1);
        assert!(h.paused_for().is_some_and(|d| d > Duration::from_secs(100)));
    }

    #[tokio::test]
    async fn the_app_is_named_and_statuses_come_back() {
        let (base, server) = serve(vec![reply("404 Not Found", "application/json", "", "{}"), reply("200 OK", "application/json", "", r#"{"a":1}"#)]).await;
        let h = Http::new("test", "me@example.com", "", None);
        assert_eq!(h.get(&format!("{base}/x"), &[("q", "a b".into())], &[]).await.unwrap().status, 404);
        assert_eq!(h.get(&format!("{base}/y"), &[], &[("Authorization", "Bearer k".into())]).await.unwrap().json().unwrap()["a"], 1);
        let seen = server.await.unwrap();
        assert!(seen[0].starts_with("GET /x?q=a+b ") || seen[0].starts_with("GET /x?q=a%20b "), "{}", seen[0]);
        assert!(seen[0].to_lowercase().contains(&format!("user-agent: glass/{} (me@example.com; {})", env!("CARGO_PKG_VERSION"), HOMEPAGE)), "{}", seen[0]);
        assert!(seen[1].contains("Bearer k"));
    }

    #[tokio::test]
    async fn offline_pauses_after_two_failures() {
        // Nothing listens on the discard port.
        let h = Http::new("test", "", "", None);
        assert!(matches!(h.get("http://127.0.0.1:9/a", &[], &[]).await, Err(Error::Network(_))));
        assert!(matches!(h.get("http://127.0.0.1:9/b", &[], &[]).await, Err(Error::Network(_))));
        assert!(matches!(h.get("http://127.0.0.1:9/c", &[], &[]).await, Err(Error::Busy(_))), "resting instead of trying again");
    }
}
