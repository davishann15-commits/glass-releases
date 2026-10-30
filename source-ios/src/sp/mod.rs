//! Spotify's internal services, answering the Web API calls the UI makes.
//!
//! The public Web API rate-limits the client ID Glass signs in with (it is
//! shared by every librespot-based player), so Glass talks to the services
//! Spotify's own desktop app uses instead. They were found by analyzing
//! Spotify.app 1.3.4 with REA (xpui bundle + native core strings):
//!
//!   playlist/v2          your playlists and their contents
//!   collection/v2        Liked Songs, saved albums, followed artists, podcasts
//!   extended-metadata    tracks, albums, artists, shows, episodes (batched)
//!   recently-played/v3   recently played contexts
//!   user-profile-view/v3 profiles
//!   connect-state/v1     live player state, queue, devices, remote control
//!   pathfinder (GraphQL) search, top items, artist pages, browse
//!
//! Every response is converted to the Web API's JSON shape, so the UI and
//! `App` work unchanged.

pub(crate) mod conv;
mod library;
mod pb;
mod player;

use crate::model::id_of;
use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use http::Method;
use librespot_core::authentication::Credentials;
use librespot_core::{Session, SessionConfig};
use librespot_protocol::connect::Cluster;
use librespot_protocol::extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::metadata;
use protobuf::Message;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub type Query = HashMap<String, String>;

/// Called when Spotify pushes a change to the player state (see `player`).
pub type Changed = Arc<dyn Fn() + Send + Sync>;

/// A cached value and when it was fetched.
type Stamped<T> = (Instant, Arc<T>);

/// One entry of the metadata cache. Songs are kept as the bytes Spotify sent, about a
/// tenth of their size as JSON, and converted again (a few microseconds) each time
/// they're asked for: a big library's songs would otherwise take tens of MB. Albums,
/// artists and shows are fewer, and their bytes carry whole track and release lists,
/// so they're kept converted.
#[derive(Clone)]
enum Meta {
    Song(Arc<[u8]>),
    Json(Value),
}

const PATHFINDER: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
const PAGE_TTL: Duration = Duration::from_secs(600);
/// An artist overview serves the requests of one artist page, which come together.
const OVERVIEW_TTL: Duration = Duration::from_secs(30);

fn page_ttl(key: &str) -> Duration {
    if key.starts_with("overview:") { OVERVIEW_TTL } else { PAGE_TTL }
}
/// Song/album/artist metadata kept in memory (a song is about 1 KB, see `Meta`). Past
/// it, only what the current request needs is kept; pages keep their own copies anyway.
const META_LIMIT: usize = 4_000;
/// What isn't asked for in this long is let go (see `Sp::trim`), as the app does with its pages.
const IDLE: Duration = Duration::from_secs(600);

/// Spotify's web UI bundle, which declares each GraphQL query as
/// `new X("<name>","query","<sha256>",...)`.
const XPUI: &str = "/Applications/Spotify.app/Contents/Resources/Apps/xpui.spa";

/// Persisted GraphQL queries, by operation name, as shipped in Spotify.app
/// 1.3.6's xpui bundle. Spotify rotates these with app updates; when one is
/// rejected, `rescan_queries` reads the current ones from the installed app (on
/// macOS, where Spotify.app can be installed).
const QUERIES: &[(&str, &str)] = &[
    ("searchDesktop", "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03"),
    ("userTopContent", "61b7353d3625780022dbc63d459eb64cd609e7e5e2a820f9160d65030efe2e15"),
    ("queryArtistOverview", "1ac33ddab5d39a3a9c27802774e6d78b9405cc188c6f75aed007df2a32737c72"),
    ("browseAll", "dbd8b55e09a58afc52eab438bc228ba28fd72ac2f2148c6c26354980e4579001"),
    ("browsePage", "f5c4e6d668f5716464a231c1cc8b22c1cbf6ad68b09929fd7de813a30581298b"),
];

pub struct Sp {
    session: Session,
    user: String,
    observer_id: String,
    http: reqwest::Client,
    /// The Connect state as Spotify last pushed it (shared, so reading it copies nothing).
    cluster: RwLock<Option<Arc<Cluster>>>,
    /// Tracks, albums, artists, shows and episodes by URI, with when each was last
    /// asked for (see `trim`).
    meta: Mutex<HashMap<String, (Instant, Meta)>>,
    sets: Mutex<HashMap<String, Stamped<Vec<library::Saved>>>>,
    rootlist: Mutex<Option<Stamped<library::Rootlist>>>,
    playlists: Mutex<HashMap<String, Stamped<library::Playlist>>>,
    mosaics: Mutex<HashMap<String, String>>,
    /// Assembled pages (artist overviews, albums, shows, profiles).
    pages: Mutex<HashMap<String, Stamped<Value>>>,
    queries: RwLock<HashMap<String, String>>,
    /// The player-state observer gave up (see `player::start`).
    observer_failed: std::sync::atomic::AtomicBool,
    /// Told about each player state Spotify pushes.
    on_change: Option<Changed>,
    connected_at: std::time::Instant,
}

fn queries_path() -> std::path::PathBuf {
    crate::config::config_dir().join("queries.json")
}

/// The built-in hashes, overridden by any found in a later Spotify.app.
fn load_queries() -> HashMap<String, String> {
    let mut q: HashMap<String, String> = QUERIES.iter().map(|(n, h)| (n.to_string(), h.to_string())).collect();
    if let Some(saved) = std::fs::read_to_string(queries_path()).ok().and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok()) {
        q.extend(saved);
    }
    q
}

/// Finds `"<name>","query","<64 hex>"` for each query name in `js`.
fn find_queries(js: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (name, _) in QUERIES {
        for kind in ["query", "mutation"] {
            let needle = format!("\"{name}\",\"{kind}\",\"");
            if let Some(i) = js.find(&needle) {
                let hash = &js[i + needle.len()..];
                if let Some(h) = hash.get(..64).filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit())) {
                    out.insert(name.to_string(), h.to_string());
                }
            }
        }
    }
    out
}

/// The query hashes in the installed Spotify.app.
fn installed_queries() -> HashMap<String, String> {
    let Ok(out) = std::process::Command::new("unzip").args(["-p", XPUI, "*.js"]).output() else { return HashMap::new() };
    find_queries(&String::from_utf8_lossy(&out.stdout))
}

/// Reads the current query hashes from Spotify.app and remembers them.
fn rescan_queries() -> HashMap<String, String> {
    let found = installed_queries();
    if !found.is_empty() {
        let _ = std::fs::write(queries_path(), serde_json::to_string(&found).unwrap_or_default());
    }
    found
}

/// iOS: the current query hashes from Spotify's own web player (open.spotify.com),
/// which declares them the same way as Spotify.app's bundle: the main script, then
/// the route scripts it loads lazily (search, browse, profile…), until each query
/// Glass uses is found. Remembered in queries.json like the desktop's.
#[cfg(glass_mobile)]
pub(crate) async fn web_player_queries(http: &reqwest::Client) -> HashMap<String, String> {
    // The desktop web player: a phone's is a different app without these queries.
    const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
    let get = |url: String| {
        let http = http.clone();
        async move { http.get(url).header("user-agent", UA).send().await.ok()?.text().await.ok() }
    };
    let Some(page) = get("https://open.spotify.com/".into()).await else { return HashMap::new() };
    const BASE: &str = "https://open.spotifycdn.com/cdn/build/web-player/";
    let Some(main) = page.match_indices(BASE).map(|(i, _)| &page[i..]).filter_map(|rest| rest.split('"').next()).find(|u| u.contains("/web-player.") && u.ends_with(".js")) else {
        return HashMap::new();
    };
    let Some(js) = get(main.to_string()).await else { return HashMap::new() };
    let mut found = find_queries(&js);
    for (name, hash) in web_player_chunks(&js) {
        if found.len() == QUERIES.len() {
            break;
        }
        if !["search", "browse", "genre", "profile", "home", "artist"].iter().any(|w| name.contains(w)) {
            continue;
        }
        if let Some(chunk) = get(format!("{BASE}{name}.{hash}.js")).await {
            for (k, v) in find_queries(&chunk) {
                found.entry(k).or_insert(v);
            }
        }
    }
    if !found.is_empty() {
        let _ = std::fs::write(queries_path(), serde_json::to_string(&found).unwrap_or_default());
    }
    found
}

/// The web player's lazily loaded scripts as (name, content hash), from its chunk
/// map: `(({id:"name",…})[e]||e)+"."+({id:"hash8",…})[e]+".js"`.
#[cfg(any(test, glass_mobile))]
fn web_player_chunks(js: &str) -> Vec<(String, String)> {
    let Some(end) = js.find("})[e]+\".js\"") else { return Vec::new() };
    let Some(hstart) = js[..end].rfind("({") else { return Vec::new() };
    let Some(nstart) = js[..hstart].rfind("({") else { return Vec::new() };
    let nend = js[nstart..hstart].find('}').map_or(hstart, |i| nstart + i);
    let pairs = |text: &str| -> Vec<(String, String)> {
        text.split(',')
            .filter_map(|kv| {
                let (k, v) = kv.split_once(':')?;
                let v = v.trim_matches('"');
                let ok = !k.is_empty() && k.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '~');
                ok.then(|| (k.to_string(), v.to_string()))
            })
            .collect()
    };
    let hashes: HashMap<String, String> = pairs(&js[hstart + 2..end]).into_iter().collect();
    pairs(&js[nstart + 2..nend]).into_iter().filter_map(|(id, name)| hashes.get(&id).map(|h| (name, h.clone()))).collect()
}

/// The device ID of the connection that reads the library and pages (made once,
/// kept in `data_device_id`; the player's own is `device_id`).
fn data_device_id() -> String {
    let path = crate::config::config_dir().join("data_device_id");
    if let Ok(id) = std::fs::read_to_string(&path)
        && id.trim().len() >= 34
        && id.trim().bytes().all(|b| b.is_ascii_hexdigit())
    {
        return id.trim().to_string();
    }
    let id = crate::model::random_hex(40);
    let _ = std::fs::create_dir_all(crate::config::config_dir());
    let _ = std::fs::write(&path, &id);
    id
}

fn extension(uri: &str) -> Option<ExtensionKind> {
    Some(match uri.split(':').nth(1)? {
        "track" => ExtensionKind::TRACK_V4,
        "episode" => ExtensionKind::EPISODE_V4,
        "album" => ExtensionKind::ALBUM_V4,
        "artist" => ExtensionKind::ARTIST_V4,
        "show" => ExtensionKind::SHOW_V4,
        _ => return None,
    })
}

fn kind(uri: &str) -> &str {
    uri.split(':').nth(1).unwrap_or("")
}

fn convert(uri: &str, b: &[u8]) -> Option<Value> {
    let mut v = match kind(uri) {
        "track" => conv::track(&metadata::Track::parse_from_bytes(b).ok()?),
        "episode" => conv::episode(&metadata::Episode::parse_from_bytes(b).ok()?),
        "album" => {
            let a = metadata::Album::parse_from_bytes(b).ok()?;
            let n = conv::album_track_uris(&a).len();
            let mut v = conv::album(&a, Vec::new());
            v.as_object_mut()?.remove("tracks");
            v["total_tracks"] = json!(n);
            v
        }
        "artist" => conv::artist(&metadata::Artist::parse_from_bytes(b).ok()?),
        "show" => conv::show(&metadata::Show::parse_from_bytes(b).ok()?),
        _ => return None,
    };
    // Relinked tracks come back under another ID; keep the one asked for.
    v["uri"] = json!(uri);
    v["id"] = json!(id_of(uri));
    Some(v)
}

fn split(path: &str) -> (&str, Query) {
    let path = path.trim_start_matches(crate::api::BASE);
    let (p, q) = path.split_once('?').unwrap_or((path, ""));
    let query = q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), conv::form_decode(v))).collect();
    (p, query)
}

fn page(items: Vec<Value>, limit: usize) -> Value {
    let total = items.len();
    json!({ "items": items.into_iter().take(limit).collect::<Vec<_>>(), "total": total })
}

fn list(q: &Query, key: &str) -> Vec<String> {
    q.get(key).map(|s| s.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

impl Drop for Sp {
    fn drop(&mut self) {
        self.session.shutdown();
    }
}

impl Sp {
    /// A second librespot session just for data (playback has its own), plus
    /// the Connect observer.
    pub async fn connect(access_token: &str, proxy: &str, on_change: Option<Changed>) -> Result<Arc<Self>> {
        Self::connect_with(Credentials::with_access_token(access_token), proxy, on_change).await
    }

    /// With librespot's saved login (tests; and the iOS core's debug-build feasibility probe).
    #[cfg(any(test, glass_mobile))]
    pub async fn connect_cached(credentials: Credentials) -> Result<Arc<Self>> {
        Self::connect_with(credentials, "", None).await
    }

    #[cfg(test)]
    pub async fn connect_cached_watching(credentials: Credentials, on_change: Changed) -> Result<Arc<Self>> {
        Self::connect_with(credentials, "", Some(on_change)).await
    }

    async fn connect_with(credentials: Credentials, proxy: &str, on_change: Option<Changed>) -> Result<Arc<Self>> {
        let device_id = data_device_id();
        let proxy = crate::config::proxy_url(proxy)?;
        let session = Session::new(SessionConfig { device_id: device_id.clone(), proxy: proxy.clone(), ..Default::default() }, None);
        session.connect(credentials, false).await.context("couldn't connect to Spotify")?;
        let mut http = reqwest::Client::builder().timeout(Duration::from_secs(20));
        if let Some(p) = proxy {
            http = http.proxy(reqwest::Proxy::all(p.as_str())?);
        }
        let sp = Arc::new(Self {
            user: session.username(),
            observer_id: format!("hobs_{}", device_id.get(..34).unwrap_or(&device_id)),
            session,
            http: http.build()?,
            cluster: RwLock::new(None),
            meta: Mutex::default(),
            sets: Mutex::default(),
            rootlist: Mutex::default(),
            playlists: Mutex::default(),
            mosaics: Mutex::default(),
            pages: Mutex::default(),
            queries: RwLock::new(load_queries()),
            observer_failed: Default::default(),
            on_change,
            connected_at: std::time::Instant::now(),
        });
        player::start(&sp);
        Ok(sp)
    }

    pub fn is_alive(&self) -> bool {
        !self.session.is_invalid()
    }

    /// Whether to replace this connection: the session ended, or player
    /// state stopped coming in (retried at most once a minute, since library
    /// data still works without it).
    pub fn needs_reconnect(&self) -> bool {
        !self.is_alive() || (self.observer_failed.load(std::sync::atomic::Ordering::Relaxed) && self.connected_at.elapsed() > Duration::from_secs(60))
    }

    fn display_name(&self) -> String {
        self.cached("me").and_then(|v| v["display_name"].as_str().map(str::to_string)).unwrap_or_else(|| self.user.clone())
    }

    fn cached(&self, key: &str) -> Option<Arc<Value>> {
        self.pages.lock().unwrap().get(key).filter(|(at, _)| at.elapsed() < page_ttl(key)).map(|(_, v)| v.clone())
    }

    fn remember(&self, key: &str, v: Value) -> Arc<Value> {
        let v = Arc::new(v);
        let mut pages = self.pages.lock().unwrap();
        pages.retain(|k, (t, _)| t.elapsed() < page_ttl(k));
        pages.insert(key.to_string(), (Instant::now(), v.clone()));
        v
    }

    // ------------------------------------------------------------ metadata

    /// Raw metadata protobufs, many entities per request.
    async fn raw_meta(&self, uris: &[String]) -> Vec<(String, Vec<u8>)> {
        let country = self.session.country();
        let chunks: Vec<Vec<String>> = uris.chunks(200).map(<[String]>::to_vec).collect();
        let pages: Vec<Vec<(String, Vec<u8>)>> = stream::iter(chunks)
            .map(|chunk| {
                let country = country.clone();
                async move {
                    let mut req = BatchedEntityRequest::new();
                    let h = req.header.mut_or_insert_default();
                    h.country = country;
                    h.catalogue = "premium".into();
                    for u in &chunk {
                        let Some(kind) = extension(u) else { continue };
                        let mut e = EntityRequest::new();
                        e.entity_uri = u.clone();
                        let mut q = ExtensionQuery::new();
                        q.extension_kind = kind.into();
                        e.query.push(q);
                        req.entity_request.push(e);
                    }
                    match self.session.spclient().get_extended_metadata(req).await {
                        Ok(resp) => resp
                            .extended_metadata
                            .into_iter()
                            .flat_map(|a| a.extension_data.into_iter())
                            .filter_map(|d| {
                                let value = d.extension_data.into_option()?.value;
                                (!value.is_empty()).then_some((d.entity_uri, value))
                            })
                            .collect(),
                        Err(e) => {
                            log::warn!("metadata batch failed: {e}");
                            Vec::new()
                        }
                    }
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;
        pages.into_iter().flatten().collect()
    }

    async fn raw_one(&self, uri: &str) -> Result<Vec<u8>> {
        self.raw_meta(&[uri.to_string()]).await.into_iter().next().map(|(_, b)| b).with_context(|| format!("404 {uri}: not found"))
    }

    /// Web API JSON for each URI, in order. Unknown ones get a placeholder.
    pub(super) async fn entities(&self, uris: &[String]) -> Vec<Value> {
        let mut found: HashMap<String, Value> = HashMap::new();
        let mut songs: Vec<(&str, Arc<[u8]>)> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        {
            let mut m = self.meta.lock().unwrap();
            let now = Instant::now();
            let mut seen = HashSet::new();
            for u in uris.iter().filter(|u| seen.insert(u.as_str())) {
                match m.get_mut(u) {
                    Some((at, meta)) => {
                        *at = now;
                        match meta {
                            Meta::Song(b) => songs.push((u, b.clone())),
                            Meta::Json(v) => {
                                found.insert(u.clone(), v.clone());
                            }
                        }
                    }
                    None if extension(u).is_some() => missing.push(u.clone()),
                    None => {}
                }
            }
        }
        // Converted with the lock released, so other requests aren't kept waiting.
        for (u, b) in songs {
            if let Some(v) = convert(u, &b) {
                found.insert(u.to_string(), v);
            }
        }
        if !missing.is_empty() {
            let raw = self.raw_meta(&missing).await;
            let fetched: Vec<(String, Meta, Value)> = raw
                .into_iter()
                .filter_map(|(uri, bytes)| {
                    let v = convert(&uri, &bytes)?;
                    let keep = if kind(&uri) == "track" { Meta::Song(bytes.into()) } else { Meta::Json(v.clone()) };
                    Some((uri, keep, v))
                })
                .collect();
            let mut m = self.meta.lock().unwrap();
            // Bounded: past the limit, keep only what this request needs.
            if m.len() + fetched.len() > META_LIMIT {
                let wanted: HashSet<&str> = uris.iter().map(String::as_str).collect();
                m.retain(|k, _| wanted.contains(k.as_str()));
            }
            let now = Instant::now();
            for (uri, keep, v) in fetched {
                m.insert(uri.clone(), (now, keep));
                found.insert(uri, v);
            }
        }
        // Each is moved out the first time it's listed; a URI listed again gets a copy.
        let mut first: HashMap<&str, usize> = HashMap::new();
        let mut out: Vec<Value> = Vec::with_capacity(uris.len());
        for u in uris {
            let v = match first.get(u.as_str()) {
                Some(&i) => out[i].clone(),
                None => {
                    first.insert(u, out.len());
                    match found.remove(u) {
                        Some(v) => v,
                        None if u.starts_with("spotify:local:") => conv::local_track(u),
                        None => conv::placeholder(u),
                    }
                }
            };
            out.push(v);
        }
        out
    }

    /// Lets go of what hasn't been asked for lately: song and album details unused
    /// for `IDLE` (the app keeps its own copy of what's on screen), and pages and
    /// lists past their time. Cheap; the app calls it every half minute.
    pub fn trim(&self) {
        {
            let mut m = self.meta.lock().unwrap();
            let before = m.len();
            m.retain(|_, (at, _)| at.elapsed() < IDLE);
            if m.len() < before / 2 {
                m.shrink_to_fit();
            }
        }
        self.pages.lock().unwrap().retain(|k, (t, _)| t.elapsed() < page_ttl(k));
        self.trim_library();
    }

    async fn gql(&self, name: &str, vars: Value) -> Result<Value> {
        match self.gql_once(name, &vars).await {
            Err(e) if e.to_string().contains("PersistedQuery") => {
                // A hash saved from an older Spotify.app can be the stale one: try this build's own first.
                let builtin = QUERIES.iter().find(|(n, _)| *n == name).map(|(_, h)| h.to_string());
                let current = self.queries.read().unwrap().get(name).cloned();
                if let Some(b) = builtin.filter(|b| current.as_ref() != Some(b)) {
                    self.queries.write().unwrap().insert(name.to_string(), b);
                    if let Ok(v) = self.gql_once(name, &vars).await {
                        return Ok(v);
                    }
                }
                #[cfg(not(glass_mobile))]
                log::warn!("{name}: query hash is stale; reading current ones from Spotify.app");
                #[cfg(not(glass_mobile))]
                let found = tokio::task::spawn_blocking(rescan_queries).await.unwrap_or_default();
                // iOS can't read Spotify.app: the same declarations are in Spotify's web player.
                #[cfg(glass_mobile)]
                log::warn!("{name}: query hash is stale; reading current ones from Spotify's web player");
                #[cfg(glass_mobile)]
                let found = web_player_queries(&self.http).await;
                if found.is_empty() {
                    return Err(e.context(if cfg!(target_os = "macos") {
                        "Spotify changed how this page is asked for; installing or updating Spotify.app (Glass reads it from there), or updating Glass, fixes it"
                    } else {
                        "Spotify changed how this page is asked for; a Glass update will follow it"
                    }));
                }
                self.queries.write().unwrap().extend(found);
                self.gql_once(name, &vars).await
            }
            r => r,
        }
    }

    async fn gql_once(&self, name: &str, vars: &Value) -> Result<Value> {
        let hash = self.queries.read().unwrap().get(name).cloned().context("unknown query")?;
        let token = self.session.login5().auth_token().await?;
        let client_token = self.session.spclient().client_token().await?;
        let body = json!({ "variables": vars, "operationName": name, "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } } });
        let r = self.http.post(PATHFINDER).bearer_auth(&token.access_token).header("client-token", client_token).header("app-platform", "OSX").json(&body).send().await?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() || v["data"].is_null() {
            bail!("{} {name}: {}", status.as_u16(), v["errors"][0]["message"].as_str().unwrap_or("no data"));
        }
        Ok(v["data"].clone())
    }

    // ------------------------------------------------------------ routing

    /// `handle`, but a page this keeps (an album, an artist, a show, your profile)
    /// comes back shared rather than copied: the app's cache and this one are then
    /// the same memory.
    pub async fn handle_shared(&self, method: &Method, path: &str, body: Option<&Value>, cap: Option<usize>) -> Result<Arc<Value>> {
        if *method == Method::GET {
            let (p, _) = split(path);
            let segs: Vec<&str> = p.trim_matches('/').split('/').collect();
            match segs.as_slice() {
                ["me"] => return self.me().await,
                ["albums", id] => return self.album(id).await,
                ["artists", id] => return self.artist(id).await,
                ["shows" | "audiobooks", id] => return self.show(id).await,
                ["users", id] => return self.user(id).await,
                _ => {}
            }
        }
        self.handle(method, path, body, cap).await.map(Arc::new)
    }

    /// Answers one Web API request. `cap` is set when the caller wants every
    /// page (up to `cap` items) rather than one page of `limit`.
    pub async fn handle(&self, method: &Method, path: &str, body: Option<&Value>, cap: Option<usize>) -> Result<Value> {
        let (p, q) = split(path);
        let segs: Vec<&str> = p.trim_matches('/').split('/').collect();
        let limit = cap.or_else(|| q.get("limit").and_then(|l| l.parse().ok())).unwrap_or(50);
        if *method != Method::GET {
            return self.write(method, &segs, &q, body).await;
        }
        match segs.as_slice() {
            ["me"] => Ok((*self.me().await?).clone()),
            ["me", "playlists"] => Ok(page(self.rootlist().await?.playlists.clone(), limit)),
            ["me", "tracks"] => self.saved("collection", &["track", "local"], "track", limit).await,
            ["me", "albums"] => self.saved("collection", &["album"], "album", limit).await,
            ["me", "episodes"] => self.saved("listenlater", &["episode"], "episode", limit).await,
            ["me", "following"] => self.saved("artist", &["artist"], "", limit).await,
            ["me", "shows"] => self.shows(false, limit).await,
            ["me", "audiobooks"] => self.shows(true, limit).await,
            ["me", "library" | "following", "contains"] => {
                self.contains(list(&q, "uris").into_iter().chain(list(&q, "ids").into_iter().map(|id| format!("spotify:{}:{id}", q.get("type").map(String::as_str).unwrap_or("artist")))).collect())
                    .await
            }
            ["me", k, "contains"] => {
                let kind = match *k {
                    "tracks" => "track",
                    "albums" => "album",
                    "shows" | "audiobooks" => "show",
                    "episodes" => "episode",
                    _ => "",
                };
                self.contains(list(&q, "ids").into_iter().map(|id| format!("spotify:{kind}:{id}")).collect()).await
            }
            ["playlists", id, "followers", "contains"] => self.contains(vec![format!("spotify:playlist:{id}")]).await,
            ["me", "player"] => self.player_json().await,
            ["me", "player", "queue"] => self.queue_json().await,
            ["me", "player", "devices"] => self.devices_json(),
            ["me", "player", "recently-played"] => self.recent().await,
            ["me", "top", k] => self.top(k, limit).await,
            ["playlists", id] => Ok(self.playlist(id).await?.json.clone()),
            ["playlists", id, "tracks" | "items"] => self.playlist_items_json(id, cap.unwrap_or(limit)).await,
            ["albums", id] => Ok((*self.album(id).await?).clone()),
            ["albums", id, "tracks"] => Ok(self.album(id).await?["tracks"].clone()),
            ["artists", id] => Ok((*self.artist(id).await?).clone()),
            ["artists", id, "top-tracks"] => self.top_tracks(id).await,
            ["artists", id, "albums"] => self.releases(id, q.get("include_groups").map(String::as_str).unwrap_or("album,single,compilation"), limit).await,
            ["artists", id, "related-artists"] => self.related(id).await,
            ["shows" | "audiobooks", id] => Ok((*self.show(id).await?).clone()),
            ["shows", id, "episodes"] | ["audiobooks", id, "chapters"] => self.show_episodes(id, limit).await,
            ["episodes", id] => Ok(self.entities(&[format!("spotify:episode:{id}")]).await.remove(0)),
            ["tracks", id] => Ok(self.entities(&[format!("spotify:track:{id}")]).await.remove(0)),
            ["search"] => self.search(q.get("q").map(String::as_str).unwrap_or(""), q.get("type").map(String::as_str).unwrap_or("track"), limit).await,
            // Glass's own: compact rows of every song in the user's playlists and Liked Songs.
            ["glass", "library-songs"] => self.library_songs().await,
            // Spotify's lyrics for a song (librespot's lyrics request).
            ["glass", "lyrics", id] => {
                let tid = librespot_core::SpotifyId::from_base62(id).map_err(|e| anyhow::anyhow!("bad track id: {e}"))?;
                let raw = self.session.spclient().get_lyrics(&tid).await.map_err(|e| {
                    let code = if e.kind == librespot_core::error::ErrorKind::NotFound { "404" } else { "error" };
                    anyhow::anyhow!("{code} lyrics: {e}")
                })?;
                Ok(serde_json::from_slice(&raw)?)
            }
            ["browse", "categories"] => self.categories().await,
            ["browse", "categories", id, "playlists"] => self.category(id).await,
            ["users", id] => Ok((*self.user(id).await?).clone()),
            ["users", id, "playlists"] => self.user_playlists(id).await,
            _ => bail!("404 {p}: not available"),
        }
    }

    async fn write(&self, method: &Method, segs: &[&str], q: &Query, body: Option<&Value>) -> Result<Value> {
        let remove = *method == Method::DELETE;
        let uris = |kind: &str| -> Vec<String> { list(q, "ids").into_iter().map(|id| format!("spotify:{kind}:{id}")).collect() };
        match segs {
            ["me", "player", rest @ ..] => return self.player_write(method, rest, q, body).await,
            ["me", "tracks"] => self.write_set("collection", &uris("track"), remove).await?,
            ["me", "albums"] => self.write_set("collection", &uris("album"), remove).await?,
            ["me", "shows" | "audiobooks"] => self.write_set("show", &uris("show"), remove).await?,
            ["me", "episodes"] => self.write_set("listenlater", &uris("episode"), remove).await?,
            ["me", "following"] if q.get("type").map(String::as_str) == Some("artist") => self.write_set("artist", &uris("artist"), remove).await?,
            ["me", "following"] => bail!("Following people isn't available in Glass yet"),
            ["me", "library"] => {
                for u in list(q, "uris") {
                    match kind(&u) {
                        "track" | "album" => self.write_set("collection", &[u], remove).await?,
                        "artist" => self.write_set("artist", &[u], remove).await?,
                        "show" => self.write_set("show", &[u], remove).await?,
                        "episode" => self.write_set("listenlater", &[u], remove).await?,
                        "playlist" => self.follow_playlist(&u, !remove).await?,
                        _ => bail!("Can't save {u}"),
                    }
                }
            }
            ["me", "playlists"] | ["users", _, "playlists"] => {
                let name = body.and_then(|b| b["name"].as_str()).unwrap_or("New Playlist");
                let uri = self.create_playlist(name).await?;
                return Ok(json!({ "id": id_of(&uri), "uri": uri }));
            }
            ["playlists", id, "followers"] => self.follow_playlist(&format!("spotify:playlist:{id}"), !remove).await?,
            ["playlists", _, "images"] => bail!("Changing playlist covers isn't available in Glass yet"),
            ["playlists", id, "tracks" | "items"] => {
                let b = body.cloned().unwrap_or_default();
                match *method {
                    Method::POST => {
                        let add: Vec<String> = b["uris"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_else(|| list(q, "uris"));
                        self.add_to_playlist(id, &add).await?;
                    }
                    Method::DELETE => {
                        let rows = b["tracks"].as_array().or(b["items"].as_array()).cloned().unwrap_or_default();
                        let rem: Vec<String> = rows.iter().filter_map(|r| r["uri"].as_str().map(str::to_string)).collect();
                        self.remove_from_playlist(id, &rem).await?;
                    }
                    _ => {
                        let from = b["range_start"].as_i64().context("range_start")? as i32;
                        let to = b["insert_before"].as_i64().context("insert_before")? as i32;
                        self.move_in_playlist(id, from, to).await?;
                    }
                }
            }
            ["playlists", id] => self.edit_playlist(id, &body.cloned().unwrap_or_default()).await?,
            _ => bail!("404 {}: not available", segs.join("/")),
        }
        Ok(Value::Null)
    }

    // ------------------------------------------------------------ handlers

    async fn me(&self) -> Result<Arc<Value>> {
        if let Some(v) = self.cached("me") {
            return Ok(v);
        }
        let mut v = self.profile(&self.user).await?;
        let product = self.session.user_data().attributes.get("type").cloned().unwrap_or_else(|| "premium".into());
        v["product"] = json!(product);
        v["country"] = json!(self.session.country());
        v["email"] = json!("");
        Ok(self.remember("me", v))
    }

    async fn profile(&self, user: &str) -> Result<Value> {
        let raw = self.session.spclient().get_user_profile(user, Some(0), Some(0)).await?;
        let p: Value = serde_json::from_slice(&raw)?;
        let img = p["image_url"].as_str().filter(|u| !u.is_empty());
        Ok(json!({
            "id": user, "uri": format!("spotify:user:{user}"), "type": "user",
            "display_name": p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(user),
            "images": img.map(|u| vec![json!({ "url": u, "width": 300, "height": 300 })]).unwrap_or_default(),
            "followers": { "total": p["followers_count"].as_u64().unwrap_or(0) },
        }))
    }

    async fn user(&self, id: &str) -> Result<Arc<Value>> {
        let key = format!("user:{id}");
        if let Some(v) = self.cached(&key) {
            return Ok(v);
        }
        let v = self.profile(id).await?;
        Ok(self.remember(&key, v))
    }

    async fn user_playlists(&self, id: &str) -> Result<Value> {
        // Spotify answers with up to about 300 of them, however many more are asked for.
        let raw = self.session.spclient().get_user_profile(id, Some(500), Some(0)).await?;
        let p: Value = serde_json::from_slice(&raw)?;
        let items: Vec<Value> = p["public_playlists"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|pl| {
                        let uri = pl["uri"].as_str().unwrap_or("");
                        let owner = pl["owner_uri"].as_str().unwrap_or("").rsplit(':').next().unwrap_or("");
                        json!({
                            "id": id_of(uri), "uri": uri, "type": "playlist", "name": conv::unescape(pl["name"].as_str().unwrap_or("")),
                            "images": pl["image_url"].as_str().map(|u| vec![json!({ "url": u })]).unwrap_or_default(),
                            "owner": { "id": owner, "display_name": pl["owner_name"], "type": "user" }, "public": true,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(page(items, 500))
    }

    /// A collection set as Web API "saved" items: `{added_at, <key>: entity}`,
    /// or plain entities when `key` is empty.
    async fn saved(&self, set: &str, kinds: &[&str], key: &str, limit: usize) -> Result<Value> {
        let all = self.set(set).await?;
        let rows: Vec<&library::Saved> = all.iter().filter(|s| kinds.contains(&kind(&s.uri))).collect();
        let total = rows.len();
        let rows: Vec<&library::Saved> = rows.into_iter().take(limit).collect();
        let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
        let ents = self.entities(&uris).await;
        let items: Vec<Value> = if key.is_empty() { ents } else { rows.iter().zip(ents).map(|(r, e)| json!({ "added_at": conv::iso(r.added_at * 1000), key: e })).collect() };
        Ok(json!({ "items": items, "total": total }))
    }

    /// Followed podcasts, or audiobooks (Spotify stores both as shows).
    async fn shows(&self, audiobooks: bool, limit: usize) -> Result<Value> {
        let all = self.set("show").await?;
        let uris: Vec<String> = all.iter().map(|s| s.uri.clone()).collect();
        let ents = self.entities(&uris).await;
        let key = if audiobooks { "audiobook" } else { "show" };
        let items: Vec<Value> = all.iter().zip(ents).filter(|(_, e)| (e["type"] == "audiobook") == audiobooks).map(|(s, e)| json!({ "added_at": conv::iso(s.added_at * 1000), key: e })).collect();
        Ok(page(items, limit))
    }

    async fn contains(&self, uris: Vec<String>) -> Result<Value> {
        let mut out = Vec::with_capacity(uris.len());
        for u in &uris {
            let found = match kind(u) {
                "track" | "album" | "local" => self.set("collection").await?.iter().any(|s| &s.uri == u),
                "artist" => self.set("artist").await?.iter().any(|s| &s.uri == u),
                "show" => self.set("show").await?.iter().any(|s| &s.uri == u),
                "episode" => self.set("listenlater").await?.iter().any(|s| &s.uri == u),
                "playlist" => self.rootlist().await?.playlists.iter().any(|p| p["uri"] == json!(u)),
                _ => false,
            };
            out.push(json!(found));
        }
        Ok(Value::Array(out))
    }

    async fn recent(&self) -> Result<Value> {
        let path = format!("/recently-played/v3/user/{}/recently-played?format=json&offset=0&limit=50&filter=default,collection-new-episodes", self.user);
        let raw = self.session.spclient().request_as_json(&Method::GET, &path, None, None).await?;
        let v: Value = serde_json::from_slice(&raw)?;
        let ctxs = v["playContexts"].as_array().cloned().unwrap_or_default();
        let tracks: Vec<String> = ctxs.iter().map(|c| c["lastPlayedTrackUri"].as_str().unwrap_or("").to_string()).collect();
        let ents = self.entities(&tracks).await;
        let items: Vec<Value> = ctxs.iter().zip(ents).map(|(c, t)| json!({ "track": t, "played_at": conv::iso(c["lastPlayedTime"].as_i64().unwrap_or(0)), "context": { "uri": c["uri"] } })).collect();
        Ok(json!({ "items": items }))
    }

    async fn top(&self, k: &str, limit: usize) -> Result<Value> {
        let input = json!({ "offset": 0, "limit": limit.min(50), "sortBy": "AFFINITY", "timeRange": "SHORT_TERM" });
        let artists = k == "artists";
        let d = self.gql("userTopContent", json!({ "includeTopArtists": artists, "topArtistsInput": input, "includeTopTracks": !artists, "topTracksInput": input })).await?;
        let list = &d["me"]["profile"][if artists { "topArtists" } else { "topTracks" }];
        Ok(json!({ "items": conv::glist(list) }))
    }

    async fn album(&self, id: &str) -> Result<Arc<Value>> {
        let key = format!("album:{id}");
        if let Some(v) = self.cached(&key) {
            return Ok(v);
        }
        let uri = format!("spotify:album:{id}");
        let a = metadata::Album::parse_from_bytes(&self.raw_one(&uri).await?)?;
        let tracks = self.entities(&conv::album_track_uris(&a)).await;
        let mut v = conv::album(&a, tracks);
        v["uri"] = json!(uri);
        v["id"] = json!(id);
        Ok(self.remember(&key, v))
    }

    /// The parts of an artist's overview Glass reads. Spotify's answer also carries
    /// every discography section, playlists, merch and concerts, ten times as much;
    /// it's kept only briefly (see `OVERVIEW_TTL`), for the artist page's requests.
    async fn overview(&self, id: &str) -> Result<Arc<Value>> {
        let key = format!("overview:{id}");
        if let Some(v) = self.cached(&key) {
            return Ok(v);
        }
        let mut d = self.gql("queryArtistOverview", json!({ "uri": format!("spotify:artist:{id}"), "locale": "", "includePrerelease": true })).await?;
        let a = &mut d["artistUnion"];
        let top: Vec<Value> = a["discography"]["topTracks"]["items"].as_array().map(|l| l.iter().map(|t| json!({ "track": { "uri": t["track"]["uri"] } })).collect()).unwrap_or_default();
        let trimmed = json!({
            "uri": a["uri"].take(),
            "profile": { "name": a["profile"]["name"].take(), "biography": { "text": a["profile"]["biography"]["text"].take() } },
            "visuals": { "avatarImage": a["visuals"]["avatarImage"].take() },
            "stats": { "monthlyListeners": a["stats"]["monthlyListeners"].take() },
            "discography": { "topTracks": { "items": top } },
            "relatedContent": { "relatedArtists": { "items": a["relatedContent"]["relatedArtists"]["items"].take() } },
        });
        Ok(self.remember(&key, trimmed))
    }

    async fn artist(&self, id: &str) -> Result<Arc<Value>> {
        let key = format!("artist:{id}");
        if let Some(v) = self.cached(&key) {
            return Ok(v);
        }
        let a = self.overview(id).await?;
        let mut v = conv::gentity(&json!({ "__typename": "Artist", "uri": a["uri"], "profile": a["profile"], "visuals": a["visuals"], "stats": a["stats"] })).context("404 artist")?;
        v["popularity"] = json!(0);
        v["genres"] = json!([]);
        v["monthly_listeners"] = a["stats"]["monthlyListeners"].clone();
        v["biography"] = json!(crate::model::plain(a["profile"]["biography"]["text"].as_str().unwrap_or("")));
        Ok(self.remember(&key, v))
    }

    async fn top_tracks(&self, id: &str) -> Result<Value> {
        let a = self.overview(id).await?;
        let uris: Vec<String> = a["discography"]["topTracks"]["items"].as_array().map(|l| l.iter().filter_map(|t| t["track"]["uri"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        Ok(json!({ "tracks": self.entities(&uris).await }))
    }

    async fn related(&self, id: &str) -> Result<Value> {
        let a = self.overview(id).await?;
        let list: Vec<Value> = a["relatedContent"]["relatedArtists"]["items"]
            .as_array()
            .map(|l| {
                l.iter()
                    .filter_map(|x| {
                        let mut x = x.clone();
                        x["__typename"] = json!("Artist");
                        conv::gentity(&x)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({ "artists": list }))
    }

    /// An artist's releases from its metadata (every one, not just a page),
    /// newest first.
    async fn releases(&self, id: &str, groups: &str, limit: usize) -> Result<Value> {
        let uri = format!("spotify:artist:{id}");
        let a = metadata::Artist::parse_from_bytes(&self.raw_one(&uri).await?)?;
        let wanted: Vec<&str> = groups.split(',').collect();
        let picked: Vec<(String, &str)> = conv::artist_release_uris(&a).into_iter().filter(|(_, g)| wanted.contains(g)).collect();
        let uris: Vec<String> = picked.iter().map(|(u, _)| u.clone()).collect();
        let mut albums: Vec<Value> = self
            .entities(&uris)
            .await
            .into_iter()
            .zip(picked.iter())
            .map(|(mut v, (_, g))| {
                v["album_group"] = json!(g);
                v
            })
            .filter(|v| v["name"] != "Unavailable")
            .collect();
        albums.sort_by(|x, y| y["release_date"].as_str().cmp(&x["release_date"].as_str()));
        Ok(page(albums, limit))
    }

    async fn show(&self, id: &str) -> Result<Arc<Value>> {
        let key = format!("show:{id}");
        if let Some(v) = self.cached(&key) {
            return Ok(v);
        }
        let uri = format!("spotify:show:{id}");
        let mut v = conv::show(&metadata::Show::parse_from_bytes(&self.raw_one(&uri).await?)?);
        v["uri"] = json!(uri);
        v["id"] = json!(id);
        Ok(self.remember(&key, v))
    }

    async fn show_episodes(&self, id: &str, limit: usize) -> Result<Value> {
        let s = metadata::Show::parse_from_bytes(&self.raw_one(&format!("spotify:show:{id}")).await?)?;
        let uris = conv::show_episode_uris(&s);
        let total = uris.len();
        let uris: Vec<String> = uris.into_iter().take(limit).collect();
        Ok(json!({ "items": self.entities(&uris).await, "total": total }))
    }

    async fn search(&self, query: &str, types: &str, limit: usize) -> Result<Value> {
        let vars = json!({
            "searchTerm": query, "offset": 0, "limit": limit.clamp(1, 50), "numberOfTopResults": 5,
            "includeAudiobooks": true, "includePreReleases": false, "includeLocalConcertsField": false,
            "includeArtistHasConcertsField": false, "includeAuthors": false, "includeEpisodeContentRatingsV2": false,
        });
        let d = self.gql("searchDesktop", vars).await?;
        let r = &d["searchV2"];
        let mut out = json!({});
        for t in types.split(',') {
            let (key, src) = match t {
                "track" => ("tracks", "tracksV2"),
                "artist" => ("artists", "artists"),
                "album" => ("albums", "albumsV2"),
                "playlist" => ("playlists", "playlists"),
                "show" => ("shows", "podcasts"),
                "episode" => ("episodes", "episodes"),
                "audiobook" => ("audiobooks", "audiobooks"),
                _ => continue,
            };
            out[key] = json!({ "items": conv::glist(&r[src]), "total": r[src]["totalCount"] });
        }
        // A single-type search is read as one page.
        if !types.contains(',')
            && let Some(key) = out.as_object().and_then(|o| o.keys().next().cloned())
        {
            return Ok(out[&key].clone());
        }
        Ok(out)
    }

    async fn categories(&self) -> Result<Value> {
        let d = self
            .gql("browseAll", json!({ "pagePagination": { "offset": 0, "limit": 10 }, "sectionPagination": { "offset": 0, "limit": 99 }, "browseEndUserIntegration": "INTEGRATION_DESKTOP" }))
            .await?;
        let mut items = Vec::new();
        for sec in d["browseStart"]["sections"]["items"].as_array().cloned().unwrap_or_default() {
            for it in sec["sectionItems"]["items"].as_array().cloned().unwrap_or_default() {
                let uri = it["uri"].as_str().unwrap_or("");
                if !uri.starts_with("spotify:page:") {
                    continue;
                }
                let card = &it["content"]["data"]["data"]["cardRepresentation"];
                items.push(json!({
                    "id": id_of(uri), "name": card["title"]["transformedLabel"],
                    "icons": card["artwork"]["sources"].as_array().map(|s| s.iter().take(1).map(|x| json!({ "url": x["url"] })).collect::<Vec<_>>()).unwrap_or_default(),
                    "color": card["backgroundColor"]["hex"],
                }));
            }
        }
        Ok(json!({ "categories": { "items": items, "total": items.len() } }))
    }

    async fn category(&self, id: &str) -> Result<Value> {
        let vars = json!({ "pagePagination": { "offset": 0, "limit": 20 }, "sectionPagination": { "offset": 0, "limit": 20 }, "uri": format!("spotify:page:{id}"), "browseEndUserIntegration": "INTEGRATION_DESKTOP", "includeEpisodeContentRatingsV2": false });
        let d = self.gql("browsePage", vars).await?;
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for sec in d["browse"]["sections"]["items"].as_array().cloned().unwrap_or_default() {
            for it in sec["sectionItems"]["items"].as_array().cloned().unwrap_or_default() {
                if let Some(v) = conv::gentity(&it["content"])
                    && seen.insert(v["uri"].as_str().unwrap_or("").to_string())
                {
                    items.push(v);
                }
            }
        }
        Ok(json!({ "playlists": { "items": items, "total": items.len() } }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_queries() {
        let (p, q) = split("/search?q=sigur%20r%C3%B3s&type=track,album&limit=10");
        assert_eq!(p, "/search");
        assert_eq!(q["q"], "sigur rós");
        assert_eq!(list(&q, "type"), vec!["track", "album"]);
        let (p, _) = split("https://api.spotify.com/v1/me/tracks?limit=50");
        assert_eq!(p, "/me/tracks");
    }

    /// Songs carry their preview clip, and the clip plays (read-only):
    /// `cargo test live_previews -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_previews() {
        let cache = librespot_core::cache::Cache::new(Some(crate::config::config_dir()), None, None, None).unwrap();
        let sp = Sp::connect_with(cache.credentials().expect("sign in to Glass first"), "", None).await.unwrap();
        let ids = ["0VjIjW4GlUZAMYd2vXMi3b", "50WkRwFR0BAo6If7GtXnA8", "20JcU4ZiYAuKE4oIal8WoV", "41sGGCCoHI2GLV9qadX80A", "1gQxoBZiyLK5hRLLoBg89x", "4ogaI0XfmAunA0LyjveMSF"];
        let mut tracks = Vec::new();
        for id in ids {
            tracks.push(sp.handle(&Method::GET, &format!("/tracks/{id}"), None, None).await.unwrap());
        }
        let mut with = 0;
        for t in &tracks {
            let url = t["preview_url"].as_str();
            println!("{:<34} {}", t["name"].as_str().unwrap_or(""), url.unwrap_or("(none)"));
            with += url.is_some() as usize;
        }
        let first = tracks.iter().find_map(|t| t["preview_url"].as_str()).expect("a song with a preview");
        let clip = reqwest::get(first).await.unwrap();
        assert_eq!(clip.status(), 200);
        let bytes = clip.bytes().await.unwrap();
        println!("{with} of {} have a preview; the first is {} KB", ids.len(), bytes.len() / 1024);
        assert!(rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec())).is_ok(), "the clip decodes");
    }

    /// Every route the UI uses, against the real account in Glass's cache.
    /// Read-only. Run with `cargo test --release live_routes -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_routes() {
        let cache = librespot_core::cache::Cache::new(Some(crate::config::config_dir()), None, None, None).unwrap();
        let sp = Sp::connect_with(cache.credentials().expect("sign in to Glass first"), "", None).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let get = |path: String, cap: Option<usize>| {
            let sp = sp.clone();
            async move {
                let t = Instant::now();
                let r = sp.handle(&Method::GET, &path, None, cap).await;
                let summary = match &r {
                    Ok(v) => {
                        let list = v["items"]
                            .as_array()
                            .or(v["tracks"].as_array())
                            .or(v["artists"].as_array())
                            .or(v["devices"].as_array())
                            .or(v["categories"]["items"].as_array())
                            .or(v["playlists"]["items"].as_array())
                            .or(v.as_array());
                        let first = list.and_then(|l| l.first()).map(|f| f.get("track").or(f.get("album")).or(f.get("show")).or(f.get("episode")).unwrap_or(f).clone()).unwrap_or(v.clone());
                        format!("ok  {:>4} items  {:?}", list.map(Vec::len).unwrap_or(0), first["name"].as_str().or(first["display_name"].as_str()).or(first.as_bool().map(|_| "bool")).unwrap_or(""))
                    }
                    Err(e) => format!("ERR {e:#}"),
                };
                println!("{:>5}ms  {path:<70} {summary}", t.elapsed().as_millis());
                r.unwrap_or(Value::Null)
            }
        };
        let me = get("/me".into(), None).await;
        assert!(me["id"].is_string());
        let pls = get("/me/playlists?limit=50".into(), Some(2000)).await;
        let liked = get("/me/tracks?limit=50".into(), Some(10_000)).await;
        get("/me/tracks?limit=1".into(), None).await;
        get("/me/albums?limit=50".into(), Some(2000)).await;
        let artists = get("/me/following?type=artist&limit=50".into(), Some(2000)).await;
        let shows = get("/me/shows?limit=50".into(), Some(500)).await;
        let books = get("/me/audiobooks?limit=50".into(), Some(500)).await;
        println!("        audiobook: {}", books["items"][0].to_string().chars().take(400).collect::<String>());
        let eps = get("/me/episodes?limit=10".into(), None).await;
        get("/me/player".into(), None).await;
        get("/me/player/queue".into(), None).await;
        get("/me/player/devices".into(), None).await;
        get("/me/player/recently-played?limit=50".into(), None).await;
        let top = get("/me/top/artists?limit=12&time_range=short_term".into(), None).await;
        get("/me/top/tracks?limit=12&time_range=short_term".into(), None).await;
        let id = |v: &Value, k: Option<&str>| -> String {
            let first = &v["items"][0];
            let e = k.map(|k| &first[k]).unwrap_or(first);
            e["id"].as_str().unwrap_or("").to_string()
        };
        let pl = id(&pls, None);
        get(format!("/playlists/{pl}"), None).await;
        get(format!("/playlists/{pl}/tracks?limit=100"), Some(10_000)).await;
        let first_spotify = liked["items"].as_array().unwrap().iter().find(|i| i["track"]["is_local"] == false).cloned().unwrap_or_default();
        let track = first_spotify["track"]["id"].as_str().unwrap_or("").to_string();
        get(format!("/tracks/{track}"), None).await;
        get(format!("/me/tracks/contains?ids={track}"), None).await;
        let album = first_spotify["track"]["album"]["id"].as_str().unwrap_or("").to_string();
        get(format!("/albums/{album}"), None).await;
        let artist = if top["items"][0]["id"].is_string() { id(&top, None) } else { id(&artists, None) };
        get(format!("/artists/{artist}"), None).await;
        get(format!("/artists/{artist}/top-tracks?market=from_token"), None).await;
        get(format!("/artists/{artist}/albums?include_groups=album,single,compilation&limit=50"), Some(200)).await;
        get(format!("/artists/{artist}/albums?include_groups=appears_on&limit=20"), None).await;
        get(format!("/artists/{artist}/related-artists"), None).await;
        let show = id(&shows, Some("show"));
        if !show.is_empty() {
            get(format!("/shows/{show}"), None).await;
            get(format!("/shows/{show}/episodes?limit=50"), Some(1000)).await;
        }
        let ep = id(&eps, Some("episode"));
        if !ep.is_empty() {
            get(format!("/episodes/{ep}"), None).await;
        }
        get("/search?q=radiohead&type=track,artist,album,playlist,show,episode,audiobook&limit=10".into(), None).await;
        get("/search?q=radiohead&type=track&limit=10".into(), Some(50)).await;
        let cats = get("/browse/categories?limit=50".into(), None).await;
        let cat = cats["categories"]["items"][0]["id"].as_str().unwrap_or("").to_string();
        get(format!("/browse/categories/{cat}/playlists?limit=50"), None).await;
        get("/users/spotify".into(), None).await;
        get(format!("/users/{}/playlists?limit=50", me["id"].as_str().unwrap()), Some(500)).await;
        get(format!("/playlists/{pl}/followers/contains?ids=x"), None).await;
    }

    #[test]
    fn query_hashes_from_bundle_text() {
        let h = "a".repeat(64);
        let js = format!(r#"x=new I.l("searchDesktop","query","{h}",null);y=new I.l("browseAll","query","short",null)"#);
        let found = find_queries(&js);
        assert_eq!(found.get("searchDesktop"), Some(&h));
        assert!(!found.contains_key("browseAll"));
    }

    #[test]
    fn web_player_chunk_map() {
        let js =
            r#"u.u=e=>""+(({1328:"xpui-pip-mini-player",4406:"xpui-routes-search",9908:"dwp-panel-section"})[e]||e)+"."+({1328:"38964775",4406:"0ab299ca",9908:"d73a3ce7"})[e]+".js",u.miniCssF=e=>"""#;
        let mut chunks = web_player_chunks(js);
        chunks.sort();
        assert_eq!(chunks, vec![("dwp-panel-section".into(), "d73a3ce7".into()), ("xpui-pip-mini-player".into(), "38964775".into()), ("xpui-routes-search".into(), "0ab299ca".into())]);
        assert!(web_player_chunks("no map here").is_empty());
    }

    #[test]
    fn installed_spotify_matches_built_in_hashes() {
        if !std::path::Path::new(XPUI).exists() {
            return;
        }
        let found = installed_queries();
        for (name, hash) in QUERIES {
            assert_eq!(found.get(*name).map(String::as_str), Some(*hash), "{name}");
        }
    }

    /// What's playing on the account right now, and where. Read-only.
    /// Run with `cargo test --release live_player_state -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_player_state() {
        let cache = librespot_core::cache::Cache::new(Some(crate::config::config_dir()), None, None, None).unwrap();
        let sp = Sp::connect_with(cache.credentials().expect("sign in to Glass first"), "", None).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let p = sp.handle(&Method::GET, "/me/player", None, None).await.unwrap();
        println!("playing={} item={} device={} ({}) progress={}", p["is_playing"], p["item"]["name"], p["device"]["name"], p["device"]["type"], p["progress_ms"]);
        let d = sp.handle(&Method::GET, "/me/player/devices", None, None).await.unwrap();
        for dev in d["devices"].as_array().cloned().unwrap_or_default() {
            println!("device: {} ({}) active={}", dev["name"], dev["type"], dev["is_active"]);
        }
    }

    /// Creates a throwaway playlist, edits it every way the UI can, then
    /// deletes it. Writes to the real account (with permission).
    /// Run with `cargo test --release live_playlist_edits -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_playlist_edits() {
        let cache = librespot_core::cache::Cache::new(Some(crate::config::config_dir()), None, None, None).unwrap();
        let sp = Sp::connect_with(cache.credentials().expect("sign in to Glass first"), "", None).await.unwrap();
        let call = |m: Method, path: String, body: Option<Value>| {
            let sp = sp.clone();
            async move { sp.handle(&m, &path, body.as_ref(), None).await }
        };
        let fresh = |id: String| {
            let sp = sp.clone();
            async move {
                sp.playlists.lock().unwrap().remove(&id);
                *sp.rootlist.lock().unwrap() = None;
                sp.handle(&Method::GET, &format!("/playlists/{id}/tracks"), None, Some(100)).await.unwrap()
            }
        };
        let uris = |v: &Value| -> Vec<String> { v["items"].as_array().unwrap().iter().map(|i| i["track"]["uri"].as_str().unwrap_or("").to_string()).collect() };

        let created = call(Method::POST, "/me/playlists".into(), Some(json!({ "name": "Glass test — delete me" }))).await.expect("create playlist");
        let id = created["id"].as_str().unwrap().to_string();
        let uri = format!("spotify:playlist:{id}");
        println!("ok  created {uri}");
        let result = async {
            let lib = sp.handle(&Method::GET, "/me/playlists", None, Some(2000)).await?;
            anyhow::ensure!(lib["items"][0]["uri"] == json!(uri), "new playlist is first in the library");
            println!("ok  it's at the top of Your Library");

            let liked = sp.handle(&Method::GET, "/me/tracks?limit=50", None, Some(50)).await?;
            let picks: Vec<String> =
                liked["items"].as_array().unwrap().iter().filter_map(|i| i["track"]["uri"].as_str()).filter(|u| u.starts_with("spotify:track:")).take(2).map(str::to_string).collect();
            call(Method::POST, format!("/playlists/{id}/tracks"), Some(json!({ "uris": picks }))).await?;
            anyhow::ensure!(uris(&fresh(id.clone()).await) == picks, "added two tracks in order");
            println!("ok  added 2 tracks");

            call(Method::PUT, format!("/playlists/{id}/tracks"), Some(json!({ "range_start": 1, "insert_before": 0 }))).await?;
            let moved = uris(&fresh(id.clone()).await);
            anyhow::ensure!(moved == vec![picks[1].clone(), picks[0].clone()], "moved: {moved:?}");
            println!("ok  reordered");

            call(Method::DELETE, format!("/playlists/{id}/tracks"), Some(json!({ "tracks": [{ "uri": picks[0] }] }))).await?;
            anyhow::ensure!(uris(&fresh(id.clone()).await) == vec![picks[1].clone()], "removed one");
            println!("ok  removed a track");

            call(Method::PUT, format!("/playlists/{id}"), Some(json!({ "name": "Glass test — renamed", "description": "temporary" }))).await?;
            sp.playlists.lock().unwrap().remove(&id);
            let p = sp.handle(&Method::GET, &format!("/playlists/{id}"), None, None).await?;
            anyhow::ensure!(p["name"] == "Glass test — renamed" && p["description"] == "temporary", "renamed: {} / {}", p["name"], p["description"]);
            println!("ok  renamed and described");
            Ok(())
        }
        .await;
        call(Method::DELETE, format!("/playlists/{id}/followers"), None).await.expect("delete playlist");
        *sp.rootlist.lock().unwrap() = None;
        let lib = sp.handle(&Method::GET, "/me/playlists", None, Some(2000)).await.unwrap();
        assert!(!lib["items"].as_array().unwrap().iter().any(|p| p["uri"] == json!(uri)), "deleted playlist left the library");
        println!("ok  deleted (removed from Your Library)");
        result.unwrap();
    }

    #[test]
    fn pages_count_everything() {
        let v = page(vec![json!(1), json!(2), json!(3)], 2);
        assert_eq!(v["total"], 3);
        assert_eq!(v["items"].as_array().unwrap().len(), 2);
    }
}
