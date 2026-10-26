//! Offline demo mode (`GLASS_DEMO=1`): fills the UI with sample data so the
//! layout can be checked without signing in. Makes no network requests, and runs
//! in a home folder of its own (see `use_own_home`).
//!
//! `GLASS_SCREENSHOT=/path.png` saves a screenshot of the window after it
//! settles, then quits. `GLASS_ROUTE=library|nowplaying|home|playlist|artist|search|settings|activity|liked|local|device`
//! picks the page; `GLASS_RIGHT=queue|devices|lyrics|assistant` opens a side panel and
//! `GLASS_THEME_PANEL=1` the theme panel. `GLASS_LYRICS=word|line|plain|instrumental|none|ambiguous`
//! picks what the (offline) lyrics lookup finds.

use crate::app::{App, AuthState, Load, Route};
use eframe::egui;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// Gives the demo a home folder of its own, so it never reads or changes the real
/// settings, library, local files or assistant accounts: a new one that's removed
/// on quit (returned), or with `GLASS_DEMO=<folder>`, that folder, kept between runs.
/// The XDG folder variables are cleared too: they come before HOME for theme.toml,
/// and on Linux for every folder. Called first thing in `main`, before any other thread starts.
pub fn use_own_home() -> Option<PathBuf> {
    let v = std::env::var_os("GLASS_DEMO")?;
    let (home, fresh) = if v.to_string_lossy().contains('/') { (PathBuf::from(v), false) } else { (std::env::temp_dir().join(format!("glass-demo-{}", std::process::id())), true) };
    if let Err(e) = std::fs::create_dir_all(&home) {
        log::warn!("couldn't make the demo's home folder {}: {e}", home.display());
        return None;
    }
    // SAFETY: no other thread is running yet (see above), so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", &home);
        for name in ["XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"] {
            std::env::remove_var(name);
        }
    }
    fresh.then_some(home)
}

fn art(ctx: &egui::Context, n: usize) -> String {
    let uri = format!("bytes://demo/{n}.png");
    let palette = [(236, 72, 153), (99, 102, 241), (16, 185, 129), (245, 158, 11), (14, 165, 233), (168, 85, 247), (239, 68, 68), (34, 197, 94)];
    let (a, b) = (palette[n % palette.len()], palette[(n * 3 + 1) % palette.len()]);
    let img = image::RgbImage::from_fn(128, 128, |x, y| {
        let t = (x + y) as f32 / 254.0;
        let mix = |p: u8, q: u8| (p as f32 * (1.0 - t) + q as f32 * t) as u8;
        image::Rgb([mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2)])
    });
    let mut png = Vec::new();
    let _ = img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    ctx.include_bytes(uri.clone(), png);
    uri
}

fn images(ctx: &egui::Context, n: usize) -> Value {
    json!([{ "url": art(ctx, n), "width": 300, "height": 300 }])
}

pub fn install(app: &mut App) {
    let ctx = app.ctx.clone();
    let me = json!({ "id": "me", "display_name": "Davis", "email": "you@example.com", "country": "US", "product": "premium", "uri": "spotify:user:me", "type": "user", "followers": { "total": 12 }, "images": [] });
    let artists: Vec<Value> = ["Aurora Lane", "The Paper Kites", "Nils Frahm", "Phoebe Bridgers", "Bonobo", "Khruangbin"]
        .iter()
        .enumerate()
        .map(|(i, n)| json!({ "id": format!("a{i}"), "name": n, "type": "artist", "uri": format!("spotify:artist:a{i}"), "images": images(&ctx, i + 20), "followers": { "total": 120_000 * (i + 1) }, "genres": ["indie folk", "ambient"], "popularity": 60 + i }))
        .collect();
    let albums: Vec<Value> = ["Quiet Hours", "Woodland", "Says", "Punisher", "Migration", "Mordechai", "Glass Garden", "Low Tide"]
        .iter()
        .enumerate()
        .map(|(i, n)| json!({ "id": format!("al{i}"), "name": n, "type": "album", "album_type": "album", "uri": format!("spotify:album:al{i}"), "images": images(&ctx, i), "release_date": format!("20{}-0{}-1{}", 15 + i, 1 + i % 9, i % 9), "artists": [artists[i % artists.len()].clone()] }))
        .collect();
    let titles = [
        "Golden Hour",
        "Bloom",
        "Says",
        "Kyoto",
        "Kerala",
        "Time (You and I)",
        "Pale Blue",
        "Afterglow",
        "Driftwood",
        "Northern Lights",
        "Paper Planes",
        "Satellite",
        "Coastline",
        "Morning Light",
        "Undertow",
    ];
    let tracks: Vec<Value> = titles
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let al = &albums[i % albums.len()];
            json!({ "id": format!("t{i}"), "name": n, "type": "track", "uri": format!("spotify:track:t{i}"), "duration_ms": 150_000 + i as u64 * 17_000, "explicit": i == 4, "album": al, "artists": [al["artists"][0].clone()], "track_number": i + 1, "disc_number": 1, "popularity": 70 })
        })
        .collect();
    let playlists: Vec<Value> = ["Late Night Drive", "Focus Flow", "Sunday Morning", "Discover Weekly", "Running 170bpm", "Rainy Day", "Deep Work", "Road Trip"]
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let owner = if i == 3 { json!({ "id": "spotify", "display_name": "Spotify" }) } else { json!({ "id": "me", "display_name": "Davis" }) };
            json!({ "id": format!("p{i}"), "name": n, "type": "playlist", "uri": format!("spotify:playlist:p{i}"), "images": images(&ctx, i + 40), "owner": owner, "public": i % 2 == 0, "collaborative": false, "description": if i == 0 { "Slow songs for empty roads." } else { "" }, "tracks": { "total": 15 }, "followers": { "total": 3 } })
        })
        .collect();
    let items = |v: Vec<Value>| json!({ "items": v.clone(), "total": v.len() });
    let wrapped = |key: &str, v: &[Value]| json!({ "items": v.iter().map(|x| json!({ "added_at": "2025-09-12T10:00:00Z", key: x })).collect::<Vec<_>>(), "total": v.len() });
    let recent: Vec<Value> = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let ctx_uri = if i % 2 == 0 { format!("spotify:playlist:p{}", i % 8) } else { format!("spotify:album:al{}", i % 8) };
            json!({ "track": t, "played_at": format!("2025-10-1{}T12:00:00Z", 3 - (i / 6).min(3)), "context": { "uri": ctx_uri, "type": "playlist" } })
        })
        .collect();

    let mut put = |k: &str, v: Value| {
        app.data.insert(k.to_string(), Load::Ready(Arc::new(v)));
    };
    put("/me", me.clone());
    put("ALL2000:/me/playlists?limit=50", items(playlists.clone()));
    put("ALL2000:/me/albums?limit=50", wrapped("album", &albums[..4]));
    put("ALL2000:/me/following?type=artist&limit=50", items(artists.clone()));
    put("ALL500:/me/shows?limit=50", items(vec![]));
    put("ALL500:/me/audiobooks?limit=50", items(vec![]));
    put("/me/tracks?limit=1", json!({ "items": [], "total": 1284 }));
    put("/me/episodes?limit=1", json!({ "items": [], "total": 7 }));
    put("/me/episodes?limit=10", items(vec![]));
    put("/me/player/recently-played?limit=50", items(recent));
    put("/me/top/artists?limit=12&time_range=short_term", items(artists.clone()));
    put("/me/top/tracks?limit=12&time_range=short_term", items(tracks.clone()));
    put(
        "/browse/categories?limit=50",
        json!({ "categories": items(["Pop", "Hip-Hop", "Rock", "Jazz", "Classical", "Focus", "Chill", "Workout"].iter().map(|n| json!({ "id": n.to_lowercase(), "name": n, "icons": [] })).collect()) }),
    );
    for p in &playlists {
        let id = p["id"].as_str().unwrap();
        put(&format!("/playlists/{id}"), p.clone());
        put(&format!("ALL10000:/playlists/{id}/tracks?limit=100"), wrapped("track", &tracks));
    }
    for (i, a) in artists.iter().enumerate() {
        put(&format!("/artists/a{i}"), a.clone());
        put(&format!("/artists/a{i}/top-tracks?market=from_token"), json!({ "tracks": tracks[..10].to_vec() }));
        put(&format!("ALL200:/artists/a{i}/albums?include_groups=album,single,compilation&limit=50"), items(albums.clone()));
        put(&format!("/artists/a{i}/related-artists"), json!({ "artists": artists.clone() }));
        put(&format!("/artists/a{i}/albums?include_groups=appears_on&limit=20"), items(albums[2..6].to_vec()));
    }
    for (i, t) in tracks.iter().enumerate() {
        app.saved.insert(t["uri"].as_str().unwrap().to_string(), i % 3 == 0);
    }
    for a in &artists {
        app.saved.insert(a["uri"].as_str().unwrap().to_string(), true);
    }
    app.me = Some(me);
    app.pb.item = Some(Arc::new(tracks[0].clone()));
    app.pb.context_uri = "spotify:playlist:p0".into();
    app.pb.is_playing = true;
    app.pb.progress_ms = 64_000;
    app.pb.progress_at = Some(std::time::Instant::now());
    app.pb.shuffle = true;
    app.queue = Some(Arc::new(json!({ "currently_playing": tracks[0], "queue": tracks[1..8].to_vec() })));
    app.devices = Some(json!({ "devices": [
        { "id": "phone", "name": "Davis's iPhone", "type": "Smartphone", "is_active": false, "volume_percent": 60 },
        { "id": "kitchen", "name": "Kitchen", "type": "Speaker", "is_active": false, "volume_percent": 35 }
    ] }));
    demo_lyrics(app, &tracks[0]);
    app.engine_status = "Demo mode — no network".into();
    // Glass of the account on other devices (one needing an update), and a transfer,
    // unless sharing runs for real (`GLASS_DEMO_SHARE`).
    if share_folder().is_none() {
        let peer = |id: &str, name: &str, version| crate::share::Peer { id: id.into(), name: name.into(), addrs: vec![], port: 0, version };
        app.share_peers = vec![peer("phone", "Davis's iPhone", crate::share::VERSION)];
        app.share_others = vec![peer("air", "Davis's MacBook Air", 3)];
        app.transfers = vec![crate::share::Transfer { id: 1, peer: "Davis's iPhone".into(), sending: false, files: 12, done: 12, bytes: 0, total: 0, state: crate::share::State::Done }];
    }
    // `GLASS_DEMO_SHARE=<folder>`: Local Files reads that folder, and sharing runs for real.
    if let Some(folder) = share_folder() {
        app.settings.local_dirs = vec![folder];
        app.settings.share_local_files = true;
        app.scan_local();
    }
    app.auth_state = AuthState::SignedIn;
    app.route = match std::env::var("GLASS_ROUTE").as_deref() {
        Ok("local") => Route::LocalFiles,
        Ok("device") => Route::Device { id: "phone".into(), name: "Davis's iPhone".into() },
        Ok("playlist") => Route::Playlist("p0".into()),
        Ok("artist") => Route::Artist("a0".into()),
        Ok("search") => Route::Search,
        Ok("settings") => Route::Settings,
        Ok("activity") => Route::Activity,
        Ok("liked") => Route::LikedSongs,
        Ok("home") => Route::Home,
        Ok("nowplaying") => Route::NowPlaying,
        _ => Route::Library,
    };
    app.theme_panel = std::env::var("GLASS_THEME_PANEL").is_ok();
    if let Ok(p) = std::env::var("GLASS_RIGHT") {
        app.right = match p.as_str() {
            "queue" => crate::app::RightPanel::Queue,
            "devices" => crate::app::RightPanel::Devices,
            "lyrics" => crate::app::RightPanel::Lyrics,
            "assistant" => crate::app::RightPanel::Assistant,
            _ => crate::app::RightPanel::Queue,
        };
        app.settings.right_open = true;
    }
    // GLASS_SETTINGS_QUERY=assistant types that into the settings page's search.
    if let Ok(q) = std::env::var("GLASS_SETTINGS_QUERY") {
        app.settings_query = q;
    }
    // GLASS_NP_VIEW=3 picks the now-playing view; GLASS_VIZ shows the visualizer
    // (fed a made-up signal, as nothing plays in demo mode); GLASS_THEME_TAB
    // picks the theme panel's tab.
    if let Ok(v) = std::env::var("GLASS_NP_VIEW") {
        use crate::app::NpView;
        app.np_view = match v.as_str() {
            "1" | "queue" => NpView::Queue,
            "2" | "devices" => NpView::Devices,
            "3" | "playlist" => NpView::Playlist,
            _ => NpView::Lyrics,
        };
    }
    if std::env::var("GLASS_VIZ").is_ok() {
        app.settings.visualizer = true;
        crate::viz::demo_signal();
    }
    if let Ok(t) = std::env::var("GLASS_THEME_TAB") {
        app.theme_tab = t.parse().unwrap_or(0);
    }
    // GLASS_CMD="? queue me…" opens the command bar with that text.
    if let Ok(t) = std::env::var("GLASS_CMD") {
        app.cmd_text = t;
        app.cmd_open = true;
        app.focus_cmd = true;
    }
    // GLASS_AI_DEMO=1 signs the assistant in with a sample conversation; =empty, with none yet.
    match std::env::var("GLASS_AI_DEMO").as_deref() {
        Ok("empty") => {
            demo_assistant(app);
            app.ai.transcript.clear();
            app.ai.drafts.clear();
            app.ai.approvals.clear();
            app.ai.usage = Default::default();
        }
        Ok(_) => demo_assistant(app),
        Err(_) => {}
    }
}

/// Sample songs and a sample conversation, to check the assistant panel's
/// layout. Nothing here is real: no account, no network.
fn demo_assistant(app: &mut App) {
    use crate::ai::tools::{Approval, ApprovalKind, Dest, Draft, DraftStatus};
    let songs = [
        ("Plastic Love", "Mariya Takeuchi", 1984, 291),
        ("Windy Summer", "Anri", 1983, 270),
        ("Remember Summer Days", "Anri", 1983, 252),
        ("Ride on Time", "Tatsuro Yamashita", 1980, 355),
        ("Sparkle", "Tatsuro Yamashita", 1982, 256),
        ("4:00 A.M.", "Taeko Ohnuki", 1978, 311),
        ("Mayonaka no Door", "Miki Matsubara", 1979, 297),
        ("Summer Connection", "Taeko Ohnuki", 1977, 214),
        ("Last Summer Whisper", "Anri", 1982, 300),
        ("Fly-Day Chinatown", "Yasuha", 1981, 262),
        ("Dancing Sea", "Junko Yagami", 1983, 285),
        ("Midnight Pretenders", "Tomoko Aran", 1983, 330),
    ];
    let tracks: Vec<crate::local::LocalTrack> = songs
        .iter()
        .map(|(t, a, y, s)| crate::local::LocalTrack {
            path: format!("/demo/{t}.mp3").into(),
            title: (*t).into(),
            artist: (*a).into(),
            album: String::new(),
            duration_ms: s * 1000,
            added: std::time::SystemTime::UNIX_EPOCH,
            year: Some(*y),
            genre: "City Pop".into(),
            has_cover: false,
            ..Default::default()
        })
        .collect();
    // As if the assistant had found them on Spotify.
    let ids: Vec<String> = (0..songs.len()).map(|i| format!("spotify:track:demo{i}")).collect();
    for (id, (t, a, _, secs)) in ids.iter().zip(songs.iter()) {
        app.ai.catalog.add(crate::ai::spotify::Item { uri: id.clone(), name: (*t).into(), by: (*a).into(), album: String::new(), duration_ms: *secs * 1000 });
    }
    app.local_tracks = Some(std::sync::Arc::new(tracks));
    app.set_config("assistant.enabled", true.into());
    app.ai.accounts.upsert(crate::ai::account::Registration { client_id: "oaiapp_demo".into(), subject: Some("demo".into()), email: Some("you@example.com".into()), plan_enabled: true });
    app.ai.accounts.set_active(Some("oaiapp_demo".into()));
    app.ai.models = crate::ai::Models::Ready(vec![
        crate::ai::openai::Model { slug: "gpt-6-luna".into(), name: "GPT-6 Luna".into() },
        crate::ai::openai::Model { slug: "gpt-6.1-sol".into(), name: "GPT-6.1 Sol".into() },
    ]);
    app.ai.models_for = "chatgpt:oaiapp_demo".into();
    app.ai.transcript = vec![
        crate::ai::Line::User("queue me up a couple of anri songs i haven't heard".into()),
        crate::ai::Line::Note("Queued 2 songs on Spotify.".into()),
        crate::ai::Line::Assistant("Queued “Last Summer Whisper” and “Shyness Boy” — probably new to you (not in your Liked Songs or recent plays).".into()),
        crate::ai::Line::User("Make an hour of Japanese city pop for a night drive.".into()),
        crate::ai::Line::Assistant(
            "Here's a 57-minute preview, opening with “Plastic Love” and ending on “Midnight Pretenders”. Save it to Spotify if it looks right, or tell me what to change.".into(),
        ),
    ];
    app.ai.drafts =
        vec![Draft { id: "d1".into(), name: "Night Drive — City Pop".into(), description: String::new(), tracks: ids, replaces: None, dest: Dest::Spotify, status: DraftStatus::AwaitingApproval }];
    app.ai.approvals = vec![Approval { id: "a1".into(), kind: ApprovalKind::SaveDraft { draft_id: "d1".into(), replace: None } }];
    app.ai.usage = crate::ai::openai::Usage { input: 5_840, output: 412 };
}

/// Whether Glass runs in demo mode (`GLASS_DEMO`), read once.
pub fn active() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("GLASS_DEMO").is_some());
    *ON
}

/// The demo's other device's songs (its page, `Route::Device`): a few of this
/// computer's (already here) and some it doesn't have. iOS's demo lists the same.
pub fn device_songs() -> Vec<crate::share::FileMeta> {
    let songs = [
        ("Plastic Love", "Mariya Takeuchi", "Variety", 294),
        ("Stay With Me", "Miki Matsubara", "Pocket Park", 312),
        ("Mayonaka no Door", "Miki Matsubara", "Pocket Park", 300),
        ("Sparkle", "Tatsuro Yamashita", "For You", 435),
        ("Ride on Time", "Tatsuro Yamashita", "Ride on Time", 352),
        ("Dancing Sea", "Junko Yagami", "Communication", 285),
        ("Midnight Pretenders", "Tomoko Aran", "Fuyü-Kükan", 330),
        ("4:00 A.M.", "Taeko Ohnuki", "Sunshower", 323),
        ("Telephone Number", "Junko Ohashi", "Magical", 291),
    ];
    songs
        .iter()
        .enumerate()
        .map(|(i, (t, a, al, s))| crate::share::FileMeta {
            id: format!("demo{i}"),
            name: format!("{t}.flac"),
            size: *s as u64 * 110_000,
            title: (*t).into(),
            artist: (*a).into(),
            album: (*al).into(),
            duration_ms: *s as u64 * 1000,
        })
        .collect()
}

/// `GLASS_DEMO_SHARE=<folder>`: the demo's Local Files are the songs in that folder,
/// and local files sharing runs for real, signed in as the demo's account, so two
/// demos (or a demo and the iPhone app's `-demo -demoShare`) share songs over the
/// network. Only other demos have that account: real devices never see them.
pub fn share_folder() -> Option<PathBuf> {
    static DIR: std::sync::LazyLock<Option<PathBuf>> = std::sync::LazyLock::new(|| active().then(|| std::env::var_os("GLASS_DEMO_SHARE").map(PathBuf::from)).flatten());
    DIR.clone()
}

/// Handles `GLASS_SCREENSHOT`: request a capture after the UI settles, save it, quit.
pub fn screenshot_tick(ctx: &egui::Context, frame_no: u64) {
    static PATH: std::sync::LazyLock<Option<String>> = std::sync::LazyLock::new(|| std::env::var("GLASS_SCREENSHOT").ok());
    let Some(path) = PATH.as_deref() else { return };
    // GLASS_SHOT_FRAME: capture later (e.g. after a translation arrives).
    let at = std::env::var("GLASS_SHOT_FRAME").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    if frame_no == at {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    ctx.request_repaint();
    let shot = ctx.input(|i| {
        i.raw.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(img) = shot {
        let [w, h] = img.size;
        // Flatten onto a wallpaper-like gradient, standing in for the macOS blur.
        let out = image::RgbImage::from_fn(w as u32, h as u32, |x, y| {
            let c = img.pixels[y as usize * w + x as usize];
            let t = (x as f32 / w as f32 + y as f32 / h as f32) / 2.0;
            let bg = [(40.0 + 60.0 * t), (50.0 + 20.0 * (1.0 - t)), (90.0 + 50.0 * (1.0 - t))];
            let a = c.a() as f32 / 255.0;
            // Pixels are premultiplied.
            image::Rgb([(c.r() as f32 + bg[0] * (1.0 - a)) as u8, (c.g() as f32 + bg[1] * (1.0 - a)) as u8, (c.b() as f32 + bg[2] * (1.0 - a)) as u8])
        });
        let _ = out.save(&path);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Sample lyrics for the demo track, written for the demo and shaped like an
/// LRCLIB record, so they go through the real parser and views. Lookups are
/// answered from memory only.
fn demo_lyrics(app: &mut App, track: &Value) {
    use crate::lyrics::cache::Cache;
    use crate::lyrics::lrclib::Lookup;
    use crate::lyrics::{Match, TrackRef, parse};
    let t = TrackRef {
        key: track["uri"].as_str().unwrap_or("").into(),
        title: track["name"].as_str().unwrap_or("").into(),
        artists: vec![track["artists"][0]["name"].as_str().unwrap_or("").into()],
        album: track["album"]["name"].as_str().unwrap_or("").into(),
        duration_ms: track["duration_ms"].as_u64().unwrap_or(0),
    };
    let lines = [
        "City lights are slowly fading",
        "Every corner hums a tune",
        "We let the evening carry us",
        "drifting softly past the moon",
        "Count the windows, one by one",
        "till the morning finds us here",
    ];
    let start = |i: usize| 52_000 + i as u64 * 5_200;
    let mode = std::env::var("GLASS_LYRICS").unwrap_or_else(|_| "word".into());
    let mut file = format!("version: '1.0'\nmetadata:\n  title: \"{}\"\n  artist: \"{}\"\n  language: en\nlines:\n", t.title, t.artist());
    for (i, l) in lines.iter().enumerate() {
        file.push_str(&format!("  - text: \"{l}\"\n    start_ms: {}\n    end_ms: {}\n", start(i), start(i) + 4_600));
        if mode == "word" {
            file.push_str("    words:\n");
            let words: Vec<&str> = l.split(' ').collect();
            for (k, w) in words.iter().enumerate() {
                let sep = if k + 1 < words.len() { " " } else { "" };
                file.push_str(&format!("      - text: \"{w}{sep}\"\n        start_ms: {}\n", start(i) + k as u64 * 4_200 / words.len() as u64));
            }
        }
    }
    let synced: String = lines.iter().enumerate().map(|(i, l)| format!("[{:02}:{:02}.{:02}] {l}\n", start(i) / 60_000, start(i) / 1000 % 60, start(i) % 1000 / 10)).collect();
    let record = |id: u64, plain: &str| {
        json!({
            "id": id, "trackName": t.title, "artistName": t.artist(), "albumName": t.album, "duration": t.duration_ms as f64 / 1000.0,
            "instrumental": mode == "instrumental", "hasWordSync": mode == "word", "plainLyrics": if mode == "instrumental" { Value::Null } else { json!(plain) },
            "syncedLyrics": if matches!(mode.as_str(), "word" | "line" | "ambiguous") { json!(synced) } else { Value::Null },
            "lyricsfile": if mode == "word" { json!(file) } else { json!("") }
        })
    };
    let plain = lines.join("\n");
    let cache = Cache::in_memory();
    let parsed = |v: Value| parse::record(&v).expect("demo record");
    match mode.as_str() {
        "none" => cache.put(&t, &Lookup::None),
        "ambiguous" => cache.put(
            &t,
            &Lookup::Ambiguous(vec![
                parsed(record(1, &plain)),
                parsed({
                    let mut r = record(2, "A different song with the same name");
                    r["albumName"] = json!("Other Album");
                    r["syncedLyrics"] = json!("[00:52.00] A different song with the same name\n");
                    r
                }),
            ]),
        ),
        _ => cache.put(&t, &Lookup::Found(parsed(record(1, &plain)), Match::Exact)),
    }
    app.offline_lyrics(cache);
}
