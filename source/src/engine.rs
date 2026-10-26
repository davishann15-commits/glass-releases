//! The local playback engine: a librespot session, player and Spotify Connect
//! device ("Glass") on this computer. Other Spotify apps see it in their
//! device picker, and Glass controls it like any other device.

use crate::config::{Quality, Settings, cache_dir, config_dir};
use crate::dsp::{DspParams, DspSink};
use anyhow::{Result, anyhow};
use librespot_connect::{ConnectConfig, LoadContextOptions, LoadRequest, LoadRequestOptions, PlayingTrack, Spirc};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::DeviceType;
use librespot_core::{Session, SessionConfig};
use librespot_metadata::audio::{AudioItem, UniqueFields};
use librespot_playback::audio_backend;
use librespot_playback::config::{Bitrate, PlayerConfig};
use librespot_playback::mixer::{self, MixerConfig};
use librespot_playback::player::{Player, PlayerEvent};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How Spotify shows this device to the account's other apps.
#[cfg(glass_mobile)]
const DEVICE_TYPE: DeviceType = DeviceType::Smartphone;
#[cfg(not(glass_mobile))]
const DEVICE_TYPE: DeviceType = DeviceType::Computer;

#[derive(Debug, Clone)]
pub enum EngineEvent {
    Playing {
        uri: String,
        position_ms: u32,
    },
    Paused {
        position_ms: u32,
    },
    Position {
        position_ms: u32,
    },
    /// A song starts loading (it plays once enough has arrived).
    Loading {
        uri: String,
    },
    /// The song about to play, Web-API shaped (no album page linked yet:
    /// that comes with Spotify's player state).
    Track(Value),
    /// The DJ has lines for a song starting now. When `hold`, the song was
    /// loaded paused: play `say`, then resume it. `ready` can be fetched ahead.
    Dj {
        uri: String,
        say: Vec<crate::dj::Line>,
        ready: Vec<crate::dj::Line>,
        hold: bool,
    },
    Stopped,
    EndOfTrack,
    Unavailable,
    Volume(u16),
    Shuffle(bool),
    Repeat {
        context: bool,
        track: bool,
    },
    /// This device's Connect session or task is gone (reported once per engine, and not
    /// for one the app dropped itself): start a new engine.
    SessionLost,
    /// What's next on this device: each song's URI and whether it was queued (librespot's
    /// own list, which `Engine::move_next` reorders). Only iOS's queue reads it.
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    Upcoming(Vec<(String, bool)>),
}

/// Shuffle and repeat for a new load. librespot turns both off with each one;
/// Spotify carries them from one play to the next, so Glass passes them on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayMode {
    pub shuffle: bool,
    pub repeat: bool,
    pub repeat_track: bool,
}

impl PlayMode {
    /// `shuffle` and `repeat` ("off", "context" or "track") for loading `body`. The
    /// DJ plays its songs in its own order, and podcasts and audiobooks in theirs.
    pub fn for_load(body: &Value, shuffle: bool, repeat: &str) -> Self {
        let context = body["context_uri"].as_str().unwrap_or("");
        let first = body["uris"][0].as_str().unwrap_or("");
        let in_order = crate::dj::is_dj(context) || context.starts_with("spotify:show:") || first.starts_with("spotify:episode:");
        Self { shuffle: shuffle && !in_order, repeat: repeat != "off", repeat_track: repeat == "track" }
    }
}

pub struct Engine {
    pub spirc: Spirc,
    pub device_id: String,
    pub session: Session,
    /// Given up: dropped by the app, or reported lost. It reports nothing more.
    gone: Arc<AtomicBool>,
    /// Its Connect task has ended.
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    ended: Arc<AtomicBool>,
}

impl Drop for Engine {
    /// An engine the app lets go of is shut down (its Connect task and player end)
    /// without reporting itself lost.
    fn drop(&mut self) {
        self.gone.store(true, Ordering::SeqCst);
        let _ = self.spirc.shutdown();
    }
}

pub fn audio_cache_dir() -> std::path::PathBuf {
    cache_dir().join("audio")
}

fn device_id() -> String {
    let path = config_dir().join("device_id");
    if let Ok(id) = std::fs::read_to_string(&path)
        && !id.trim().is_empty()
    {
        return id.trim().to_string();
    }
    // 40 hex digits, like Spotify's own device IDs.
    let id = crate::model::random_hex(40);
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(&path, &id);
    id
}

impl Engine {
    pub async fn start(settings: &Settings, access_token: String, dsp: Arc<DspParams>, on_event: impl Fn(EngineEvent) + Send + Sync + 'static) -> Result<Self> {
        Self::start_with(settings, Credentials::with_access_token(access_token), dsp, on_event).await
    }

    pub(crate) async fn start_with(settings: &Settings, credentials: Credentials, dsp: Arc<DspParams>, on_event: impl Fn(EngineEvent) + Send + Sync + 'static) -> Result<Self> {
        let proxy = crate::config::proxy_url(&settings.proxy)?;
        let session_config = SessionConfig { client_id: settings.client_id.clone(), device_id: device_id(), proxy, autoplay: Some(settings.autoplay), ..Default::default() };
        // No size limit here: librespot would evict downloads. `Downloads::prune`
        // keeps the streaming part of the cache within the user's limit instead.
        // iOS keeps no reusable login on disk (credentials belong in the Keychain): the
        // engine signs in with a fresh access token each time it starts.
        #[cfg(glass_mobile)]
        let cache = Cache::new(None::<std::path::PathBuf>, Some(config_dir()), Some(audio_cache_dir()), None).ok();
        #[cfg(not(glass_mobile))]
        let cache = Cache::new(Some(config_dir()), Some(config_dir()), Some(audio_cache_dir()), None).ok();
        let session = Session::new(session_config, cache);
        let device_id = session.device_id().to_string();

        let player_config = PlayerConfig {
            bitrate: match settings.streaming_quality {
                Quality::Normal => Bitrate::Bitrate96,
                Quality::High => Bitrate::Bitrate160,
                Quality::VeryHigh => Bitrate::Bitrate320,
            },
            gapless: settings.gapless,
            normalisation: settings.normalize,
            normalisation_pregain_db: settings.loudness.pregain_db(),
            // Where a playlist's local songs are looked for (whether or not
            // the Local Files page is shown).
            local_file_directories: settings.local_dirs.clone(),
            // The UI estimates the position itself; seeks still report it.
            position_update_interval: None,
            ..Default::default()
        };

        let mixer_fn = mixer::find(None).ok_or_else(|| anyhow!("no mixer"))?;
        let mixer = mixer_fn(MixerConfig::default()).map_err(|e| anyhow!("{e}"))?;
        // iOS: the phone's own volume is the volume (as in every other app), so the
        // audio goes out at full scale; Connect's volume only follows it. (The soft
        // mixer's log curve put 70 % at −18 dB, under the system volume.)
        #[cfg(glass_mobile)]
        let volume = Box::new(mixer::NoOpVolume);
        #[cfg(not(glass_mobile))]
        let volume = mixer.get_soft_volume();
        // The output opens when something first plays here, not before (see `DspSink`).
        let player = Player::new(player_config, session.clone(), volume, move || Box::new(DspSink::new(Box::new(audio_backend::try_open_default), dsp)));

        let on_event = Arc::new(on_event);
        let gone = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        // "Lost" once per engine, from whichever sign comes first: the player's
        // SessionDisconnected, or the Connect task ending (librespot's event used to come
        // only after requests that fail when the network is what went away). Without
        // it the app kept a dead engine: "Internal error { channel closed }".
        let lost = {
            let (gone, on_event) = (gone.clone(), on_event.clone());
            Arc::new(move || {
                if !gone.swap(true, Ordering::SeqCst) {
                    on_event(EngineEvent::SessionLost);
                }
            })
        };
        // The DJ's lines around each song Connect starts (see dj.rs).
        {
            let on_event = on_event.clone();
            librespot_connect::glass::set_track_hook(Box::new(move |t| {
                let (say, ready) = crate::dj::on_track_start(t);
                let hold = t.playing && !say.is_empty();
                // A song loaded paused only moves the closing line along; nothing is said.
                if t.playing && (!say.is_empty() || !ready.is_empty()) {
                    on_event(EngineEvent::Dj { uri: t.uri.to_string(), say, ready, hold });
                }
                hold
            }));
        }
        let mut events = player.get_player_event_channel();
        let lost_event = lost.clone();
        let on_player_event = on_event.clone();
        tokio::spawn(async move {
            let on_event = on_player_event;
            while let Some(ev) = events.recv().await {
                let mapped = match ev {
                    PlayerEvent::Playing { track_id, position_ms, .. } => Some(EngineEvent::Playing { uri: track_id.to_uri().unwrap_or_default(), position_ms }),
                    PlayerEvent::Paused { position_ms, .. } => Some(EngineEvent::Paused { position_ms }),
                    PlayerEvent::PositionChanged { position_ms, .. } | PlayerEvent::PositionCorrection { position_ms, .. } | PlayerEvent::Seeked { position_ms, .. } => {
                        Some(EngineEvent::Position { position_ms })
                    }
                    PlayerEvent::Loading { track_id, .. } => Some(EngineEvent::Loading { uri: track_id.to_uri().unwrap_or_default() }),
                    PlayerEvent::TrackChanged { audio_item } => Some(EngineEvent::Track(item_json(&audio_item))),
                    PlayerEvent::Stopped { .. } => Some(EngineEvent::Stopped),
                    PlayerEvent::EndOfTrack { .. } => Some(EngineEvent::EndOfTrack),
                    PlayerEvent::Unavailable { .. } => Some(EngineEvent::Unavailable),
                    PlayerEvent::VolumeChanged { volume } => Some(EngineEvent::Volume(volume)),
                    PlayerEvent::ShuffleChanged { shuffle } => Some(EngineEvent::Shuffle(shuffle)),
                    PlayerEvent::RepeatChanged { context, track } => Some(EngineEvent::Repeat { context, track }),
                    PlayerEvent::SessionDisconnected { .. } => {
                        lost_event();
                        None
                    }
                    _ => None,
                };
                if let Some(m) = mapped {
                    on_event(m);
                }
            }
        });

        let connect_config =
            ConnectConfig { name: settings.device_name.clone(), device_type: DEVICE_TYPE, initial_volume: (settings.volume.clamp(0.0, 1.0) * u16::MAX as f32) as u16, ..Default::default() };
        let (spirc, task) = Spirc::new(connect_config, session.clone(), credentials, player, mixer).await.map_err(|e| anyhow!("Spotify Connect: {e}"))?;
        // What's next, for the queue; an older device's last reports are left out.
        {
            let (id, gone, on_event) = (spirc.glass_id(), gone.clone(), on_event.clone());
            librespot_connect::glass::set_queue_hook(Box::new(move |from, upcoming| {
                if from == id && !gone.load(Ordering::SeqCst) {
                    on_event(EngineEvent::Upcoming(upcoming));
                }
            }));
        }
        {
            let ended = ended.clone();
            tokio::spawn(async move {
                task.await;
                ended.store(true, Ordering::SeqCst);
                lost();
            });
        }
        // librespot writes its reusable login with the default permissions; the folder
        // is private (see `config::make_private_dir`), and so is the file.
        let _ = crate::config::make_private_dir();
        #[cfg(all(unix, not(glass_mobile)))]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(config_dir().join("credentials.json"), std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self { spirc, device_id, session, gone, ended })
    }

    /// The engine can still play: its Connect task runs and its session is valid.
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    pub fn running(&self) -> bool {
        !self.gone.load(Ordering::SeqCst) && !self.ended.load(Ordering::SeqCst) && !self.session.is_invalid()
    }

    /// Moves the song listed next at `from` to `to` (indices into the last
    /// `EngineEvent::Upcoming` list; `uri` is the song meant).
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    pub fn move_next(&self, uri: &str, from: usize, to: usize) -> Result<()> {
        self.spirc.glass_move_next(uri.to_string(), from, to).map_err(|e| anyhow!("{e}"))
    }

    /// Plays a Web API play body (`context_uri` or `uris`, `offset`, `position_ms`) here.
    pub fn load(&self, body: &serde_json::Value, mode: PlayMode) -> Result<()> {
        let options = librespot_connect::Options { shuffle: mode.shuffle, repeat: mode.repeat, repeat_track: mode.repeat_track };
        let mut opts =
            LoadRequestOptions { start_playing: true, seek_to: body["position_ms"].as_u64().unwrap_or(0) as u32, context_options: Some(LoadContextOptions::Options(options)), ..Default::default() };
        let index = body["offset"]["position"].as_u64().map(|i| PlayingTrack::Index(i as u32));
        let req = if let Some(ctx) = body["context_uri"].as_str() {
            opts.playing_track = body["offset"]["uri"].as_str().map(|u| PlayingTrack::Uri(u.to_string())).or(index);
            LoadRequest::from_context_uri(ctx.to_string(), opts)
        } else {
            let uris: Vec<String> = body["uris"].as_array().map(|a| a.iter().filter_map(|u| u.as_str().map(str::to_string)).collect()).unwrap_or_default();
            if uris.is_empty() {
                return Err(anyhow!("nothing to play"));
            }
            opts.playing_track = index.or(Some(PlayingTrack::Index(0)));
            LoadRequest::from_tracks(uris, opts)
        };
        self.spirc.activate().ok();
        self.spirc.load(req).map_err(|e| anyhow!("{e}"))
    }

    /// "Radio": the autoplay variant of a context, i.e. similar tracks.
    pub fn load_radio(&self, seed_uri: &str) -> Result<()> {
        let opts = LoadRequestOptions { start_playing: true, context_options: Some(LoadContextOptions::Autoplay), ..Default::default() };
        self.spirc.activate().ok();
        self.spirc.load(LoadRequest::from_context_uri(seed_uri.into(), opts)).map_err(|e| anyhow!("{e}"))
    }
}

/// The player's own description of a song, in the shape the UI reads.
pub fn item_json(a: &AudioItem) -> Value {
    let uri = a.track_id.to_uri().unwrap_or_else(|_| a.uri.clone());
    let id = a.track_id.to_id().unwrap_or_default();
    let mut covers: Vec<_> = a.covers.iter().collect();
    covers.sort_by_key(|c| std::cmp::Reverse(c.width));
    let images: Vec<Value> = covers.iter().map(|c| json!({ "url": c.url, "width": c.width, "height": c.height })).collect();
    match &a.unique_fields {
        UniqueFields::Track { artists, album, .. } => {
            use librespot_metadata::artist::ArtistRole;
            let artists: Vec<Value> = artists
                .iter()
                .filter(|x| matches!(x.role, ArtistRole::ARTIST_ROLE_MAIN_ARTIST | ArtistRole::ARTIST_ROLE_FEATURED_ARTIST | ArtistRole::ARTIST_ROLE_UNKNOWN))
                .map(|x| json!({ "name": x.name, "id": x.id.to_id().unwrap_or_default(), "uri": x.id.to_uri().unwrap_or_default(), "type": "artist" }))
                .collect();
            json!({ "type": "track", "uri": uri, "id": id, "name": a.name, "duration_ms": a.duration_ms, "explicit": a.is_explicit, "artists": artists, "album": { "name": album, "images": images } })
        }
        UniqueFields::Episode { show_name, .. } => {
            json!({ "type": "episode", "uri": uri, "id": id, "name": a.name, "duration_ms": a.duration_ms, "explicit": a.is_explicit, "images": images, "show": { "name": show_name, "images": images } })
        }
        UniqueFields::Local { artists, album, .. } => {
            let artists: Vec<Value> = artists.iter().map(|n| json!({ "name": n, "type": "artist" })).collect();
            json!({ "type": "track", "uri": uri, "id": "", "is_local": true, "name": a.name, "duration_ms": a.duration_ms, "artists": artists, "album": { "name": album.clone().unwrap_or_default(), "images": [] } })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::Downloads;
    use std::time::Duration;

    /// Plays the DJ on a test device (its own config, volume 0) and skips
    /// through more than its first page. Skips when something is already
    /// playing on the account. `cargo test --release live_dj -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_dj() {
        let _ = env_logger::builder().is_test(true).try_init();
        let creds = Cache::new(Some(config_dir()), None, None, None).unwrap().credentials().expect("sign in to Glass first");
        let home = crate::config::test_dir("dj");
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: an ignored test run on its own (its own device IDs).
        unsafe { std::env::set_var("HOME", &home) };
        let sp = crate::sp::Sp::connect_cached(creds.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        if sp.handle(&http::Method::GET, "/me/player", None, None).await.map(|v| v["is_playing"] == true).unwrap_or(false) {
            println!("something is playing on the account; skipping");
            return;
        }
        let settings = Settings { volume: 0.0, device_name: "Glass (DJ test)".into(), ..Default::default() };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let dsp = Arc::new(DspParams::new(crate::dsp::DspState { eq_enabled: false, gains: [0.0; 6], mono: false }));
        let engine = Engine::start_with(&settings, creds, dsp, move |e| {
            let _ = tx.send(e);
        })
        .await
        .unwrap();
        engine.load(&json!({ "context_uri": "spotify:playlist:37i9dQZF1EYkqdzj48dyYq" }), PlayMode::default()).unwrap();
        let (http, cache) = (reqwest::Client::new(), crate::dj::Clips::default());
        let mut heard = Vec::new();
        let mut spoken = 0;
        while heard.len() < 4 {
            let e = tokio::time::timeout(Duration::from_secs(30), rx.recv()).await.ok().flatten().unwrap_or_else(|| panic!("the DJ stalled (heard {heard:?})"));
            match e {
                // What the app does: the song waits while X speaks, then plays.
                EngineEvent::Dj { say, hold, .. } => {
                    for l in &say {
                        println!("    X: {}", l.text());
                    }
                    if hold {
                        let clips = crate::dj::clips(&engine.session, &http, &cache, &say).await.expect("X's voice");
                        assert!(clips.iter().all(|c| c.len() > 1000), "audio clips");
                        // They decode and play (silently here) on Glass's voice player.
                        let voice = crate::dj::Voice::play(&clips, 0.0).expect("X's voice plays");
                        assert!(!voice.done());
                        drop(voice);
                        spoken += clips.len();
                        engine.spirc.play().unwrap();
                    }
                }
                EngineEvent::Playing { uri, .. } if heard.last() != Some(&uri) => {
                    println!("ok  DJ song {}: {uri}", heard.len() + 1);
                    heard.push(uri);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    crate::dj::forget_outro();
                    engine.spirc.next().unwrap();
                }
                _ => {}
            }
        }
        engine.spirc.pause().ok();
        assert!(spoken > 0, "X said something");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn shuffle_and_repeat_carry_into_a_new_load() {
        let playlist = json!({ "context_uri": "spotify:playlist:x", "offset": { "uri": "spotify:track:y" } });
        assert_eq!(PlayMode::for_load(&playlist, true, "context"), PlayMode { shuffle: true, repeat: true, repeat_track: false });
        assert_eq!(PlayMode::for_load(&playlist, false, "track"), PlayMode { shuffle: false, repeat: true, repeat_track: true });
        for in_order in [json!({ "context_uri": crate::dj::URI }), json!({ "context_uri": "spotify:show:z" }), json!({ "uris": ["spotify:episode:e"] })] {
            assert!(!PlayMode::for_load(&in_order, true, "off").shuffle, "{in_order}");
        }
    }

    #[test]
    fn the_players_song_reads_like_spotifys() {
        use librespot_core::SpotifyUri;
        use librespot_metadata::artist::{ArtistRole, ArtistWithRole, ArtistsWithRole};
        use librespot_metadata::audio::item::CoverImage;
        use librespot_metadata::image::ImageSize;
        let artist = |id: &str, name: &str, role| ArtistWithRole { id: SpotifyUri::from_uri(&format!("spotify:artist:{id}")).unwrap(), name: name.into(), role };
        let a = AudioItem {
            track_id: SpotifyUri::from_uri("spotify:track:4uLU6hMCjMI75M1A2tKUQC").unwrap(),
            uri: String::new(),
            files: Default::default(),
            name: "Plastic Love".into(),
            covers: vec![
                CoverImage { url: "https://i.scdn.co/image/small".into(), size: ImageSize::SMALL, width: 64, height: 64 },
                CoverImage { url: "https://i.scdn.co/image/big".into(), size: ImageSize::LARGE, width: 640, height: 640 },
            ],
            language: vec![],
            duration_ms: 474_000,
            is_explicit: false,
            availability: Ok(()),
            alternatives: None,
            unique_fields: UniqueFields::Track {
                artists: ArtistsWithRole(vec![
                    artist("1nkYaaSGuGCuSsNnL2lhAj", "Mariya Takeuchi", ArtistRole::ARTIST_ROLE_MAIN_ARTIST),
                    artist("0TnOYISbd1XYRBk9myaseg", "Tatsuro Yamashita", ArtistRole::ARTIST_ROLE_COMPOSER),
                ]),
                album: "Variety".into(),
                album_artists: vec![],
                popularity: 0,
                number: 1,
                disc_number: 1,
            },
        };
        let v = item_json(&a);
        assert_eq!(v["uri"], "spotify:track:4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(crate::model::artists_line(&v), "Mariya Takeuchi", "composers aren't listed as artists");
        assert_eq!(crate::model::image(&v, 300).as_deref(), Some("https://i.scdn.co/image/big"));
        assert_eq!((v["album"]["name"].as_str(), v["duration_ms"].as_u64()), (Some("Variety"), Some(474_000)));
    }

    async fn wait_for(rx: &mut tokio::sync::mpsc::UnboundedReceiver<EngineEvent>, want: fn(&EngineEvent) -> bool) -> bool {
        let found = async {
            while let Some(e) = rx.recv().await {
                if want(&e) {
                    return true;
                }
            }
            false
        };
        tokio::time::timeout(Duration::from_secs(25), found).await.unwrap_or(false)
    }

    /// Plays one Liked Song at volume 0, controls it through Connect, and
    /// downloads it. Skips when something is already playing on the account.
    /// Run with `cargo test --release live_playback -- --ignored --nocapture`
    /// while Glass itself is closed (they share a device ID).
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_playback() {
        let cache = Cache::new(Some(config_dir()), None, None, None).unwrap();
        let creds = cache.credentials().expect("sign in to Glass first");
        let sp = crate::sp::Sp::connect_cached(creds.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let get = |p: &'static str| {
            let sp = sp.clone();
            async move { sp.handle(&http::Method::GET, p, None, None).await.unwrap() }
        };
        if get("/me/player").await["is_playing"] == true {
            println!("something is playing on the account; skipping");
            return;
        }
        let liked = sp.handle(&http::Method::GET, "/me/tracks?limit=50", None, Some(200)).await.unwrap();
        let track = liked["items"].as_array().unwrap().iter().map(|i| &i["track"]).find(|t| t["is_local"] == false).unwrap().clone();
        let uri = track["uri"].as_str().unwrap().to_string();
        println!("track: {} — {}", track["name"], uri);

        let settings = Settings { volume: 0.0, device_name: "Glass (test)".into(), ..Default::default() };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let dsp = Arc::new(DspParams::new(crate::dsp::DspState { eq_enabled: false, gains: [0.0; 6], mono: false }));
        let engine = Engine::start_with(&settings, creds, dsp, move |e| {
            let _ = tx.send(e);
        })
        .await
        .unwrap();
        engine.load(&json!({ "uris": [uri] }), PlayMode::default()).unwrap();
        assert!(wait_for(&mut rx, |e| matches!(e, EngineEvent::Playing { .. })).await, "local playback started");
        println!("ok  local playback (Spirc load)");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let p = get("/me/player").await;
        assert_eq!(p["device"]["id"], json!(engine.device_id), "Connect shows Glass as the active device");
        assert_eq!(p["item"]["uri"], json!(uri));
        println!("ok  player state via Connect observer: {} on {}", p["item"]["name"], p["device"]["name"]);

        let dev = engine.device_id.clone();
        let cmd = |m: http::Method, path: String| {
            let sp = sp.clone();
            async move { sp.handle(&m, &path, None, None).await }
        };
        cmd(http::Method::PUT, format!("/me/player/pause?device_id={dev}")).await.unwrap();
        assert!(wait_for(&mut rx, |e| matches!(e, EngineEvent::Paused { .. })).await, "remote pause");
        println!("ok  Connect command: pause");
        cmd(http::Method::PUT, format!("/me/player/play?device_id={dev}")).await.unwrap();
        assert!(wait_for(&mut rx, |e| matches!(e, EngineEvent::Playing { .. })).await, "remote resume");
        println!("ok  Connect command: resume");
        let next = liked["items"].as_array().unwrap().iter().map(|i| i["track"]["uri"].as_str().unwrap_or("")).find(|u| u.starts_with("spotify:track:") && *u != uri).unwrap().to_string();
        cmd(http::Method::POST, format!("/me/player/queue?uri={}&device_id={dev}", crate::api::enc(&next))).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let q = get("/me/player/queue").await;
        assert!(q["queue"].as_array().unwrap().iter().any(|t| t["uri"] == json!(next)), "queued track shows in the queue");
        println!("ok  Connect command: add to queue");
        engine.spirc.pause().ok();

        let downloads = Downloads::load();
        let key = "glass:test-download";
        downloads.run(engine.session.clone(), settings.streaming_quality, || {});
        downloads.add(key, "test", vec![uri.clone()]);
        let mut done = false;
        for _ in 0..120 {
            if downloads.has_file(&uri) {
                done = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(done, "track downloaded into the cache");
        println!("ok  download into the encrypted audio cache");
        downloads.remove(Some(key), engine.session.cache().cloned());
        assert!(!downloads.has_file(&uri));
        println!("ok  download removed");
    }

    /// Starts one of your playlists on a test device at volume 0, with shuffle and
    /// repeat on, pauses it as soon as Spotify shows it, and checks that Spotify
    /// reports both still on, the songs after the first out of the playlist's order,
    /// and that the change was pushed to Glass's observer. Skips when something is
    /// playing on the account. Run while Glass is closed (they share a device ID):
    /// `cargo test --release live_shuffle -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_shuffle_and_repeat_survive_a_load() {
        let creds = Cache::new(Some(config_dir()), None, None, None).unwrap().credentials().expect("sign in to Glass first");
        // Counts the player states Spotify pushes to the observer.
        let pushes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = pushes.clone();
        let sp = crate::sp::Sp::connect_cached_watching(
            creds.clone(),
            Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let get = |p: String, cap: Option<usize>| {
            let sp = sp.clone();
            async move { sp.handle(&http::Method::GET, &p, None, cap).await.unwrap() }
        };
        if get("/me/player".into(), None).await["is_playing"] == true {
            println!("something is playing on the account; skipping");
            return;
        }
        let me = get("/me".into(), None).await;
        let lists = get("/me/playlists?limit=50".into(), Some(200)).await;
        let mine =
            lists["items"].as_array().unwrap().iter().find(|p| p["owner"]["id"] == me["id"] && p["tracks"]["total"].as_u64().unwrap_or(0) >= 12).expect("one of your playlists with 12 songs or more");
        let (uri, id) = (mine["uri"].as_str().unwrap().to_string(), mine["id"].as_str().unwrap().to_string());
        let order: Vec<String> =
            get(format!("/playlists/{id}/tracks?limit=100"), Some(100)).await["items"].as_array().unwrap().iter().filter_map(|i| i["track"]["uri"].as_str().map(str::to_string)).collect();
        println!("playlist {} ({} songs)", mine["name"], order.len());

        let settings = Settings { volume: 0.0, device_name: "Glass (test)".into(), ..Default::default() };
        let dsp = Arc::new(DspParams::new(crate::dsp::DspState { eq_enabled: false, gains: [0.0; 6], mono: false }));
        let engine = Engine::start_with(&settings, creds, dsp, |_| {}).await.unwrap();
        let mode = PlayMode::for_load(&json!({ "context_uri": uri }), true, "context");
        let before = pushes.load(std::sync::atomic::Ordering::Relaxed);
        engine.load(&json!({ "context_uri": uri }), mode).unwrap();
        // Spotify's view of this device, once it has the new state.
        let mut seen = Value::Null;
        let mut last = Value::Null;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let p = get("/me/player".into(), None).await;
            if p["device"]["id"] == json!(engine.device_id) && p["context"]["uri"] == json!(uri) {
                seen = p;
                break;
            }
            last = p;
        }
        if seen.is_null() {
            println!("last state: device {} ({}) context {} shuffle {}", last["device"]["name"], last["device"]["id"], last["context"]["uri"], last["shuffle_state"]);
        }
        let queue = get("/me/player/queue".into(), None).await;
        engine.spirc.pause().ok();
        drop(engine);
        assert!(!seen.is_null(), "Spotify never showed the playlist on the test device");
        let pushed = pushes.load(std::sync::atomic::Ordering::Relaxed) - before;
        println!("player states pushed meanwhile: {pushed}");
        assert!(pushed > 0, "the change on the test device was pushed to the observer");
        println!("shuffle_state={} repeat_state={} playing={}", seen["shuffle_state"], seen["repeat_state"], seen["is_playing"]);
        assert_eq!(seen["shuffle_state"], true, "shuffle stays on through the load");
        assert_eq!(seen["repeat_state"], "context", "and so does repeat");
        let first = seen["item"]["uri"].as_str().unwrap_or_default().to_string();
        let next: Vec<String> = queue["queue"].as_array().unwrap().iter().filter_map(|t| t["uri"].as_str().map(str::to_string)).take(10).collect();
        let at = order.iter().position(|u| *u == first).unwrap_or(0);
        let in_order: Vec<String> = order.iter().cycle().skip(at + 1).take(next.len()).cloned().collect();
        println!("first {first}; next {} songs, in playlist order: {}", next.len(), next == in_order);
        assert!(next.len() >= 5 && next != in_order, "the songs after the first are shuffled");
    }

    /// Plays a track inside each kind of context the UI uses (playlist,
    /// album, Liked Songs), at volume 0.
    /// Run with `cargo test --release live_contexts -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_contexts() {
        let cache = Cache::new(Some(config_dir()), None, None, None).unwrap();
        let creds = cache.credentials().expect("sign in to Glass first");
        let sp = crate::sp::Sp::connect_cached(creds.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let get = |p: String, cap: Option<usize>| {
            let sp = sp.clone();
            async move { sp.handle(&http::Method::GET, &p, None, cap).await.unwrap() }
        };
        if get("/me/player".into(), None).await["is_playing"] == true {
            println!("something is playing on the account; skipping");
            return;
        }
        let me = get("/me".into(), None).await;
        let pls = get("/me/playlists?limit=50".into(), Some(50)).await;
        let pl = pls["items"][0]["uri"].as_str().unwrap().to_string();
        let pl_items = get(format!("/playlists/{}/tracks?limit=100", pl.rsplit(':').next().unwrap()), Some(100)).await;
        let liked = get("/me/tracks?limit=50".into(), Some(100)).await;
        let spotify_tracks = |v: &Value| -> Vec<Value> {
            v["items"].as_array().unwrap().iter().map(|i| i["track"].clone()).filter(|t| t["is_local"] == false && t["uri"].as_str().is_some_and(|u| u.starts_with("spotify:track:"))).collect()
        };
        let pl_track = spotify_tracks(&pl_items)[2]["uri"].as_str().unwrap().to_string();
        let liked_track = spotify_tracks(&liked)[3].clone();
        let album = liked_track["album"]["uri"].as_str().unwrap().to_string();
        let album_v = get(format!("/albums/{}", album.rsplit(':').next().unwrap()), None).await;
        let album_track = album_v["tracks"]["items"].as_array().unwrap().last().unwrap()["uri"].as_str().unwrap().to_string();
        let collection = format!("spotify:user:{}:collection", me["id"].as_str().unwrap());

        let settings = Settings { volume: 0.0, device_name: "Glass (test)".into(), ..Default::default() };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let dsp = Arc::new(DspParams::new(crate::dsp::DspState { eq_enabled: false, gains: [0.0; 6], mono: false }));
        let engine = Engine::start_with(&settings, creds, dsp, move |e| {
            let _ = tx.send(e);
        })
        .await
        .unwrap();
        let mut failures = Vec::new();
        for (label, body, want) in [
            ("playlist + track", json!({ "context_uri": pl, "offset": { "uri": pl_track } }), pl_track.clone()),
            ("album + track", json!({ "context_uri": album, "offset": { "uri": album_track } }), album_track.clone()),
            ("liked songs + track", json!({ "context_uri": collection, "offset": { "uri": liked_track["uri"] } }), liked_track["uri"].as_str().unwrap().to_string()),
            ("playlist, from the top", json!({ "context_uri": pl }), String::new()),
        ] {
            while rx.try_recv().is_ok() {}
            if let Err(e) = engine.load(&body, PlayMode::default()) {
                failures.push(format!("{label}: load error {e}"));
                continue;
            }
            let found = tokio::time::timeout(Duration::from_secs(20), async {
                while let Some(e) = rx.recv().await {
                    if let EngineEvent::Playing { uri, .. } = e {
                        return Some(uri);
                    }
                }
                None
            })
            .await
            .ok()
            .flatten();
            match found {
                Some(u) if want.is_empty() || u == want => println!("ok  {label}: playing {u}"),
                Some(u) => failures.push(format!("{label}: playing {u}, expected {want}")),
                None => failures.push(format!("{label}: never started")),
            }
        }
        engine.spirc.pause().ok();
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
