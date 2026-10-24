//! The data client the UI talks to. It keeps the Web API's request shapes
//! (`GET /me/playlists`, `PUT /me/player/play`, ...) but answers them from
//! Spotify's internal services through `sp` — see `sp/mod.rs` for why.
//!
//! Responses stay as `serde_json::Value`, so a field Spotify adds or drops
//! never breaks a whole page. Every request is written to `NetLog`, which the
//! Activity page shows.

use crate::auth::Auth;
use crate::sp::Sp;
use anyhow::Result;
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

/// Requests may still be written as full Web API URLs; this prefix is dropped.
pub const BASE: &str = "https://api.spotify.com/v1";

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
/// Long enough for a 10,000-song playlist with its metadata.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: SystemTime,
    pub method: String,
    pub path: String,
    pub status: String,
    pub millis: u128,
}

/// The last 500 requests, for the Activity page.
#[derive(Clone, Default)]
pub struct NetLog(Arc<Mutex<Log>>);

#[derive(Default)]
struct Log {
    entries: VecDeque<LogEntry>,
    /// The entries as last handed out, until another one comes in.
    shared: Option<Arc<Vec<LogEntry>>>,
}

impl NetLog {
    pub(crate) fn push(&self, e: LogEntry) {
        let mut log = self.0.lock().unwrap();
        if log.entries.len() >= 500 {
            log.entries.pop_front();
        }
        log.entries.push_back(e);
        log.shared = None;
    }

    /// The entries, oldest first: the same list each time until a request is added.
    pub fn snapshot(&self) -> Arc<Vec<LogEntry>> {
        let mut log = self.0.lock().unwrap();
        if log.shared.is_none() {
            log.shared = Some(Arc::new(log.entries.iter().cloned().collect()));
        }
        log.shared.clone().unwrap_or_default()
    }

    pub fn clear(&self) {
        let mut log = self.0.lock().unwrap();
        log.entries.clear();
        log.shared = None;
    }
}

#[derive(Clone)]
pub struct Api {
    auth: Arc<Auth>,
    proxy: String,
    sp: Arc<tokio::sync::Mutex<Connection>>,
    pub log: NetLog,
    /// Told when Spotify pushes a change to the player state.
    on_player_change: Option<crate::sp::Changed>,
}

/// The connection to Spotify's services, or why the last try failed and when.
#[derive(Default)]
struct Connection {
    sp: Option<Arc<Sp>>,
    failed: Option<(Instant, String)>,
}

/// Percent-encode a query value.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Api {
    pub fn new(auth: Arc<Auth>, proxy: &str, log: NetLog) -> Self {
        Self { auth, proxy: proxy.to_string(), sp: Arc::default(), log, on_player_change: None }
    }

    /// Calls `f` whenever Spotify pushes a change to the player state.
    pub fn on_player_change(mut self, f: crate::sp::Changed) -> Self {
        self.on_player_change = Some(f);
        self
    }

    pub fn auth(&self) -> &Arc<Auth> {
        &self.auth
    }

    /// Lets the connection's caches go of what hasn't been used lately (see `Sp::trim`).
    /// Skipped while a connection is being made.
    pub fn trim(&self) {
        if let Ok(slot) = self.sp.try_lock()
            && let Some(sp) = &slot.sp
        {
            sp.trim();
        }
    }

    /// The internal-services connection, (re)connecting when needed.
    async fn sp(&self) -> Result<Arc<Sp>> {
        let mut slot = self.sp.lock().await;
        if let Some(sp) = slot.sp.as_ref().filter(|s| !s.needs_reconnect()) {
            return Ok(sp.clone());
        }
        // Right after a failed try, the requests queued behind it fail at once with the
        // same reason, rather than each waiting out a connection attempt of its own.
        if let Some((at, why)) = &slot.failed
            && at.elapsed() < std::time::Duration::from_secs(5)
        {
            // A connection whose player state stopped still answers everything else.
            if let Some(sp) = slot.sp.as_ref().filter(|s| s.is_alive()) {
                return Ok(sp.clone());
            }
            anyhow::bail!("{why}");
        }
        // Never wait forever on a stalled network: everything else queues behind this.
        let connect = async {
            let token = self.auth.bearer().await?;
            Sp::connect(&token, &self.proxy, self.on_player_change.clone()).await
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, connect).await.unwrap_or_else(|_| Err(anyhow::anyhow!("timeout connecting to Spotify"))) {
            Ok(sp) => {
                *slot = Connection { sp: Some(sp.clone()), failed: None };
                Ok(sp)
            }
            Err(e) => {
                slot.failed = Some((Instant::now(), format!("{e:#}")));
                Err(e)
            }
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.send_raw(Method::GET, path, None, None).await
    }

    /// `get`, but a page `sp` keeps (an album, an artist, a show, a profile)
    /// comes back shared, not copied.
    pub async fn get_shared(&self, path: &str) -> Result<Arc<Value>> {
        self.send_shared(Method::GET, path, None, None).await
    }

    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.send_raw(method, path, body, None).await
    }

    async fn send_raw(&self, method: Method, path: &str, body: Option<Value>, cap: Option<usize>) -> Result<Value> {
        // Copied only if the page cache holds it too.
        self.send_shared(method, path, body, cap).await.map(Arc::unwrap_or_clone)
    }

    async fn send_shared(&self, method: Method, path: &str, body: Option<Value>, cap: Option<usize>) -> Result<Arc<Value>> {
        let short = path.trim_start_matches(BASE).to_string();
        let started = Instant::now();
        let r = match self.sp().await {
            Ok(sp) => {
                // Glass's own library index reads every playlist: give it longer.
                let limit = if short.starts_with("/glass/") { REQUEST_TIMEOUT * 4 } else { REQUEST_TIMEOUT };
                tokio::time::timeout(limit, sp.handle_shared(&method, &short, body.as_ref(), cap)).await.unwrap_or_else(|_| Err(anyhow::anyhow!("timeout {short}")))
            }
            Err(e) => Err(e),
        };
        let status = match &r {
            Ok(_) => "200".to_string(),
            Err(e) => e.to_string().split_whitespace().next().filter(|w| w.len() == 3 && w.chars().all(|c| c.is_ascii_digit())).unwrap_or("error").to_string(),
        };
        self.log.push(LogEntry { at: SystemTime::now(), method: method.to_string(), path: short.clone(), status, millis: started.elapsed().as_millis() });
        if let Err(e) = &r {
            log::warn!("{method} {short}: {e:#}");
        }
        r
    }

    /// Every item of a list endpoint, up to `cap`, as `{"items": [...], "total": n}`.
    /// Accepts lists wrapped in a key (e.g. `/me/following` -> `artists`).
    pub async fn get_all(&self, path: &str, cap: usize) -> Result<Value> {
        let mut v = self.send_raw(Method::GET, path, None, Some(cap)).await?;
        // Move the list out rather than copying it (it can be 10,000 songs).
        let page = unwrap_page_mut(&mut v);
        let total = page["total"].as_u64();
        let mut items = match page.get_mut("items").map(Value::take) {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        };
        items.truncate(cap);
        let total = total.unwrap_or(items.len() as u64);
        Ok(json!({ "items": items, "total": total }))
    }
}

/// The page in a list answer: the answer itself, or the one object inside it
/// (search wraps its pages: `{"artists": {"items": [...]}}`).
fn unwrap_page_mut(v: &mut Value) -> &mut Value {
    if v.get("items").is_none() && v.as_object().is_some_and(|o| o.len() == 1 && o.values().next().is_some_and(|i| i.get("items").is_some())) {
        return v.as_object_mut().and_then(|o| o.values_mut().next()).expect("checked above");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_keyed_pages() {
        let mut v = json!({ "artists": { "items": [1, 2], "next": null } });
        assert_eq!(unwrap_page_mut(&mut v)["items"].as_array().unwrap().len(), 2);
        let mut plain = json!({ "items": [1], "total": 1 });
        assert_eq!(unwrap_page_mut(&mut plain)["total"], 1);
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(enc("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(enc("spotify:track:1"), "spotify%3Atrack%3A1");
    }
}
