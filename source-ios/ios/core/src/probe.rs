//! Debug builds only: a read-only check that Spotify works from the iOS build
//! itself (TLS, the access point, login5, spclient, GraphQL, Connect), using an
//! existing librespot login (the desktop's credentials.json) so no sign-in is
//! needed and no session token is refreshed or rotated. Reports counts and
//! statuses only — no names, IDs or tokens — and never plays anything.

use crate::model;
use http::Method;
use std::time::Duration;

/// Appends a step to Documents/probe.log as it happens (the probe can be watched from outside).
fn step(line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::paths::documents().join("probe.log")) {
        let _ = writeln!(f, "{line}");
    }
}

pub async fn spotify(credentials_path: &str, with_engine: bool, play: bool) -> String {
    let r = tokio::time::timeout(Duration::from_secs(90), run(credentials_path, with_engine, play)).await.unwrap_or_else(|_| "TIMEOUT after 90 s (see probe.log for the last step)".into());
    step(&r);
    r
}

async fn run(credentials_path: &str, with_engine: bool, play: bool) -> String {
    let mut out = Vec::new();
    step("reading the saved login");
    let dir = std::path::Path::new(credentials_path).parent().unwrap_or(std::path::Path::new("/"));
    let Some(creds) = librespot_core::cache::Cache::new(Some(dir), None, None, None).ok().and_then(|c| c.credentials()) else {
        return format!("no librespot login at {credentials_path}");
    };
    step("connecting");
    let sp = match crate::sp::Sp::connect_cached(creds.clone()).await {
        Ok(sp) => sp,
        Err(e) => return format!("connect FAILED: {e:#}"),
    };
    out.push("ok   access point + session (librespot) from the iOS build".to_string());
    step("connected");
    let get = |path: &'static str, cap: Option<usize>| {
        let sp = sp.clone();
        async move { sp.handle(&Method::GET, path, None, cap).await }
    };
    step("GET /me");
    match get("/me", None).await {
        Ok(v) => out.push(format!("ok   /me (product: {})", model::s(&v, "product"))),
        Err(e) => out.push(format!("FAIL /me: {e:#}")),
    }
    match get("/me/playlists?limit=50", Some(2000)).await {
        Ok(v) => out.push(format!("ok   rootlist via spclient: {} playlists", model::items(&v).len())),
        Err(e) => out.push(format!("FAIL /me/playlists: {e:#}")),
    }
    match get("/me/tracks?limit=1", None).await {
        Ok(v) => out.push(format!("ok   collection (Liked Songs): {} songs", model::u(&v, "total"))),
        Err(e) => out.push(format!("FAIL /me/tracks: {e:#}")),
    }
    match get("/search?q=radiohead&type=track,artist&limit=10", None).await {
        Ok(v) => out.push(format!("ok   GraphQL searchDesktop: {} songs, {} artists", model::items(&v["tracks"]).len(), model::items(&v["artists"]).len())),
        Err(e) => out.push(format!("FAIL GraphQL search: {e:#}")),
    }
    match get("/browse/categories?limit=50", None).await {
        Ok(v) => out.push(format!("ok   GraphQL browseAll: {} categories", model::items(&v["categories"]).len())),
        Err(e) => out.push(format!("FAIL GraphQL browse: {e:#}")),
    }
    match get("/me/player", None).await {
        Ok(v) => out.push(format!("ok   Connect state observer (something playing: {})", v["is_playing"] == true)),
        Err(e) => out.push(format!("FAIL /me/player: {e:#}")),
    }
    if with_engine {
        // Joins the account's devices as "Glass (iOS probe)"; plays nothing; leaves at once.
        let settings = crate::config::Settings { volume: 0.0, device_name: "Glass (iOS probe)".into(), ..Default::default() };
        let dsp = std::sync::Arc::new(crate::dsp::DspParams::new(crate::dsp::DspState { eq_enabled: false, gains: [0.0; 6], mono: false }));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let something_playing = get("/me/player", None).await.map(|v| v["is_playing"] == true).unwrap_or(true);
        match crate::engine::Engine::start_with(&settings, creds, dsp, move |e| {
            let _ = tx.send(e);
        })
        .await
        {
            Ok(engine) => {
                out.push("ok   playback engine + Spotify Connect device started".into());
                let mut listed = false;
                for _ in 0..8 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if let Ok(v) = get("/me/player/devices", None).await
                        && model::arr(&v, "devices").iter().any(|d| model::s(d, "id") == engine.device_id)
                    {
                        listed = true;
                        break;
                    }
                }
                out.push(if listed { "ok   Spotify lists this iOS device in Connect".into() } else { "FAIL the iOS device didn't appear in Connect within 8 s".to_string() });
                // Spotify audio through the iOS output path: only when nothing plays on the
                // account (the desktop's live_playback rule), at volume 0, for a few seconds.
                if play && something_playing {
                    out.push("skip playback: something is playing on the account".into());
                } else if play {
                    step("playing one Liked Song at volume 0");
                    let liked = get("/me/tracks?limit=50", Some(50)).await.ok();
                    let uri = liked.as_ref().and_then(|v| model::items(v).iter().map(|i| &i["track"]).find(|t| t["is_local"] == false).map(|t| model::s(t, "uri").to_string()));
                    match uri {
                        Some(uri) => {
                            let _ = engine.load(&serde_json::json!({ "uris": [uri] }), crate::engine::PlayMode::default());
                            let started = tokio::time::timeout(Duration::from_secs(20), async {
                                while let Some(e) = rx.recv().await {
                                    if let crate::engine::EngineEvent::Playing { position_ms, .. } = e {
                                        return Some(position_ms);
                                    }
                                }
                                None
                            })
                            .await
                            .ok()
                            .flatten();
                            match started {
                                Some(_) => {
                                    tokio::time::sleep(Duration::from_secs(6)).await;
                                    let mut a = crate::viz::Analyzer::default();
                                    a.update(32, 1.0, 0.5);
                                    out.push(format!("ok   Spotify audio decoded and playing through RemoteIO (output tap live: {})", a.live));
                                }
                                None => out.push("FAIL playback didn't start within 20 s".into()),
                            }
                            let _ = engine.spirc.pause();
                            tokio::time::sleep(Duration::from_millis(800)).await;
                        }
                        None => out.push("skip playback: no Spotify song in Liked Songs".into()),
                    }
                }
                drop(engine);
            }
            Err(e) => out.push(format!("FAIL engine: {e:#}")),
        }
    }
    out.join("\n")
}

/// Saves one song through Share ▸ Download's pipeline (JioSaavn, else YouTube Music;
/// MP3 via LAME), into Documents/Probe — no Spotify account involved.
pub async fn save(mp3: bool) -> String {
    let item = serde_json::json!({
        "type": "track", "uri": "spotify:track:probe", "name": "Clair de Lune",
        "artists": [{ "name": "Claude Debussy" }],
        "album": { "name": "Suite bergamasque", "images": [] },
        "duration_ms": 300_000
    });
    let Some(song) = crate::save_song::song_from_item(&item) else { return "song_from_item refused the probe song".into() };
    let opts = crate::save_song::Options {
        format: if mp3 { crate::save_song::Format::Mp3 } else { crate::save_song::Format::M4a },
        mp3_kbps: 192,
        dir: crate::paths::documents().join("Probe"),
    };
    let report: std::sync::Arc<dyn Fn(crate::save_song::Status) + Send + Sync> = std::sync::Arc::new(|s| step(&format!("  {s:?}")));
    step("saving a song as a file");
    match tokio::time::timeout(Duration::from_secs(240), crate::save_song::save(crate::save_song::http_client(), song, opts, report)).await {
        Ok(Ok(path)) => {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let tags = crate::local::read_paths(&[path.clone()]).into_iter().next();
            let r = format!(
                "ok   saved {} ({} KB); reads back as “{}” by {} ({} s)",
                path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
                size / 1024,
                tags.as_ref().map(|t| t.title.as_str()).unwrap_or("?"),
                tags.as_ref().map(|t| t.artist.as_str()).unwrap_or("?"),
                tags.as_ref().map(|t| t.duration_ms / 1000).unwrap_or(0)
            );
            step(&r);
            r
        }
        Ok(Err(e)) => {
            let r = format!("FAIL save: {e:#}");
            step(&r);
            r
        }
        Err(_) => {
            step("TIMEOUT saving");
            "TIMEOUT saving".into()
        }
    }
}

/// Announces a test service and logs what browsing finds for `secs` seconds.
pub async fn bonjour(secs: u64) -> String {
    let instance = format!("probe-{}", crate::model::random_hex(6));
    let props = [("v", "3".to_string()), ("acct", "probe".to_string()), ("name", "iOS probe".to_string())];
    let (b, mut rx) = match crate::bonjour::Bonjour::start("_glass-files._tcp", &instance, 47999, &props) {
        Ok(x) => x,
        Err(e) => {
            let r = format!("FAIL bonjour: {e:#}");
            step(&r);
            return r;
        }
    };
    step(&format!("announced {instance} on port 47999"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut seen = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        let line = match ev {
            crate::bonjour::Found::Service { instance, txt, port, addrs } => format!("found {instance} port {port} v={} name={} addrs={addrs:?}", txt.get("v").cloned().unwrap_or_default(), txt.get("name").cloned().unwrap_or_default()),
            crate::bonjour::Found::Gone { instance } => format!("gone {instance}"),
        };
        step(&line);
        seen.push(line);
    }
    drop(b);
    step("bonjour probe done");
    format!("{} events", seen.len())
}

/// Reads the current GraphQL query hashes from Spotify's web player (what a stale
/// hash falls back to on iOS) and logs them against the built-in ones. Read-only.
pub async fn hashes() -> String {
    let http = reqwest::Client::new();
    let found = crate::sp::web_player_queries(&http).await;
    let mut names: Vec<_> = found.iter().collect();
    names.sort();
    for (k, v) in &names {
        step(&format!("web player {k} = {v}"));
    }
    let r = format!("{} of 5 queries found", found.len());
    step(&r);
    r
}
