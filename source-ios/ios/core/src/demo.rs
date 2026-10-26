//! Demo mode: the desktop's sample data (src/demo.rs), for checking layouts
//! without an account or network. Nothing here is real. Cover art is
//! `glass-demo:<n>`, which the app paints as the same two-color gradients the
//! desktop demo makes.

use crate::app::{AuthState, Load, MobileApp};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};

struct Data {
    me: Value,
    artists: Vec<Value>,
    albums: Vec<Value>,
    tracks: Vec<Value>,
    playlists: Vec<Value>,
}

fn images(n: usize) -> Value {
    json!([{ "url": format!("glass-demo:{n}"), "width": 300, "height": 300 }])
}

fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| {
        let me = json!({ "id": "me", "display_name": "Davis", "email": "you@example.com", "country": "US", "product": "premium", "uri": "spotify:user:me", "type": "user", "followers": { "total": 12 }, "images": [] });
        let artists: Vec<Value> = ["Aurora Lane", "The Paper Kites", "Nils Frahm", "Phoebe Bridgers", "Bonobo", "Khruangbin"]
            .iter()
            .enumerate()
            .map(|(i, n)| json!({ "id": format!("a{i}"), "name": n, "type": "artist", "uri": format!("spotify:artist:a{i}"), "images": images(i + 20), "followers": { "total": 120_000 * (i + 1) }, "genres": ["indie folk", "ambient"], "popularity": 60 + i, "monthly_listeners": 480_000 * (i + 1) }))
            .collect();
        let albums: Vec<Value> = ["Quiet Hours", "Woodland", "Says", "Punisher", "Migration", "Mordechai", "Glass Garden", "Low Tide"]
            .iter()
            .enumerate()
            .map(|(i, n)| json!({ "id": format!("al{i}"), "name": n, "type": "album", "album_type": "album", "uri": format!("spotify:album:al{i}"), "images": images(i), "release_date": format!("20{}-0{}-1{}", 15 + i, 1 + i % 9, i % 9), "artists": [artists[i % artists.len()].clone()] }))
            .collect();
        let titles = ["Golden Hour", "Bloom", "Says", "Kyoto", "Kerala", "Time (You and I)", "Pale Blue", "Afterglow", "Driftwood", "Northern Lights", "Paper Planes", "Satellite", "Coastline", "Morning Light", "Undertow"];
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
                json!({ "id": format!("p{i}"), "name": n, "type": "playlist", "uri": format!("spotify:playlist:p{i}"), "images": images(i + 40), "owner": owner, "public": i % 2 == 0, "collaborative": false, "description": if i == 0 { "Slow songs for empty roads." } else { "" }, "tracks": { "total": 15 }, "followers": { "total": 3 } })
            })
            .collect();
        Data { me, artists, albums, tracks, playlists }
    })
}

/// A demo song by URI.
pub fn track(uri: &str) -> Option<Value> {
    data().tracks.iter().find(|t| t["uri"] == uri).cloned()
}

/// The first song of a demo context.
pub fn first_of(_app: &MobileApp, context: &str) -> Option<Value> {
    if context.starts_with("spotify:album:") {
        return data().tracks.iter().find(|t| t["album"]["uri"] == context).cloned();
    }
    data().tracks.first().cloned()
}

/// The song `by` places after (or before) `uri`, wrapping around.
pub fn step(uri: &str, by: i64) -> Option<Value> {
    let tracks = &data().tracks;
    let i = tracks.iter().position(|t| t["uri"] == uri).unwrap_or(0) as i64;
    let n = tracks.len() as i64;
    tracks.get((i + by).rem_euclid(n) as usize).cloned()
}

pub fn install(app: &mut MobileApp) {
    let d = data();
    let items = |v: Vec<Value>| json!({ "items": v.clone(), "total": v.len() });
    let wrapped = |key: &str, v: &[Value]| json!({ "items": v.iter().map(|x| json!({ "added_at": "2025-09-12T10:00:00Z", key: x })).collect::<Vec<_>>(), "total": v.len() });
    let recent: Vec<Value> = d
        .tracks
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
    put("/me", d.me.clone());
    put("ALL2000:/me/playlists?limit=50", items(d.playlists.clone()));
    put("ALL2000:/me/albums?limit=50", wrapped("album", &d.albums[..4]));
    put("ALL2000:/me/following?type=artist&limit=50", items(d.artists.clone()));
    put("ALL500:/me/shows?limit=50", items(vec![]));
    put("ALL500:/me/audiobooks?limit=50", items(vec![]));
    put("/me/tracks?limit=1", json!({ "items": [], "total": 1284 }));
    put("ALL10000:/me/tracks?limit=50", wrapped("track", &d.tracks));
    put("/me/episodes?limit=1", json!({ "items": [], "total": 7 }));
    put("/me/episodes?limit=10", items(vec![]));
    put("ALL2000:/me/episodes?limit=50", items(vec![]));
    put("/me/player/recently-played?limit=50", items(recent));
    put("/me/top/artists?limit=12&time_range=short_term", items(d.artists.clone()));
    put("/me/top/tracks?limit=12&time_range=short_term", items(d.tracks.clone()));
    put("/me/top/artists?limit=10&time_range=short_term", items(d.artists.clone()));
    put("/me/top/tracks?limit=10&time_range=short_term", items(d.tracks.clone()));
    let palette = ["#EC4899", "#6366F1", "#10B981", "#F59E0B", "#0EA5E9", "#A855F7", "#EF4444", "#22C55E"];
    put(
        "/browse/categories?limit=50",
        json!({ "categories": items(["Pop", "Hip-Hop", "Rock", "Jazz", "Classical", "Focus", "Chill", "Workout"].iter().enumerate().map(|(i, n)| json!({ "id": n.to_lowercase(), "name": n, "icons": [{ "url": format!("glass-demo:{}", i + 60) }], "color": palette[i] })).collect()) }),
    );
    for (i, c) in ["pop", "hip-hop", "rock", "jazz", "classical", "focus", "chill", "workout"].iter().enumerate() {
        put(&format!("/browse/categories/{c}/playlists?limit=50"), json!({ "playlists": items(d.playlists[(i % 4)..(i % 4 + 4)].to_vec()) }));
    }
    for p in &d.playlists {
        let id = p["id"].as_str().unwrap();
        put(&format!("/playlists/{id}"), p.clone());
        put(&format!("ALL10000:/playlists/{id}/tracks?limit=100"), wrapped("track", &d.tracks));
    }
    for (i, a) in d.artists.iter().enumerate() {
        put(&format!("/artists/a{i}"), a.clone());
        put(&format!("/artists/a{i}/top-tracks?market=from_token"), json!({ "tracks": d.tracks[..10].to_vec() }));
        put(&format!("ALL200:/artists/a{i}/albums?include_groups=album,single,compilation&limit=50"), items(d.albums.clone()));
        put(&format!("ALL500:/artists/a{i}/albums?include_groups=album,single,compilation&limit=50"), items(d.albums.clone()));
        put(&format!("/artists/a{i}/related-artists"), json!({ "artists": d.artists.clone() }));
        put(&format!("/artists/a{i}/albums?include_groups=appears_on&limit=20"), items(d.albums[2..6].to_vec()));
        put(&format!("/artists/a{i}/albums?include_groups=album,single&limit=12"), items(d.albums.clone()));
    }
    for (i, al) in d.albums.iter().enumerate() {
        let mut full = al.clone();
        let tracks: Vec<Value> = d.tracks.iter().filter(|t| t["album"]["id"] == al["id"]).cloned().collect();
        full["tracks"] = json!({ "items": tracks, "total": tracks.len() });
        full["copyrights"] = json!([{ "type": "C", "text": format!("20{} Demo Records", 15 + i) }, { "type": "P", "text": format!("20{} Demo Records", 15 + i) }]);
        put(&format!("/albums/al{i}"), full);
    }
    for t in &d.tracks {
        let id = t["id"].as_str().unwrap();
        put(&format!("/tracks/{id}"), t.clone());
    }
    for (i, t) in d.tracks.iter().enumerate() {
        app.saved.insert(t["uri"].as_str().unwrap().to_string(), i % 3 == 0);
    }
    for a in &d.artists {
        app.saved.insert(a["uri"].as_str().unwrap().to_string(), true);
    }
    app.resend_saved();
    app.me = Some(d.me.clone());
    // `-demoFree`: a Spotify Free account (UI tests of what it can and can't do).
    if std::env::args().any(|a| a == "-demoFree")
        && let Some(me) = app.me.as_mut()
    {
        me["product"] = "free".into();
    }
    // `-demoTrack N`: the playlist's Nth song plays instead of the first (UI tests).
    let args: Vec<String> = std::env::args().collect();
    let playing = args.iter().position(|a| a == "-demoTrack").and_then(|i| args.get(i + 1)?.parse::<usize>().ok()).filter(|n| *n < d.tracks.len()).unwrap_or(0);
    app.pb.item = Some(Arc::new(d.tracks[playing].clone()));
    app.pb.context_uri = "spotify:playlist:p0".into();
    app.pb.is_playing = true;
    app.pb.progress_ms = 64_000;
    app.pb.progress_at = Some(std::time::Instant::now());
    app.pb.shuffle = true;
    app.queue = Some(Arc::new(json!({ "currently_playing": d.tracks[0], "queue": d.tracks[1..8].to_vec() })));
    // As if playing on this iPhone: its own list of what's next, two songs queued.
    app.upcoming = Some(d.tracks[1..8].iter().enumerate().map(|(i, t)| (t["uri"].as_str().unwrap_or("").to_string(), i < 2)).collect());
    for t in &d.tracks[..8] {
        app.queue_meta.insert(t["uri"].as_str().unwrap_or("").to_string(), t.clone());
    }
    app.devices = Some(json!({ "devices": [
        { "id": "phone", "name": "Davis's iPhone", "type": "Smartphone", "is_active": false, "volume_percent": 60 },
        { "id": "kitchen", "name": "Kitchen", "type": "Speaker", "is_active": false, "volume_percent": 35 },
        { "id": "mac", "name": "Glass", "type": "Computer", "is_active": true, "volume_percent": 70 }
    ] }));
    app.pb.device = Some(json!({ "id": "mac", "name": "Glass", "type": "Computer", "is_active": true, "volume_percent": 70 }));
    app.engine_status = "Demo mode — no network".into();
    app.auth_state = AuthState::SignedIn;
    demo_lyrics(app, &d.tracks[0]);
    // Local Files: the desktop demo's city-pop files (sample data only; there are no
    // such files, so playing them is skipped).
    let songs = [
        ("Plastic Love", "Mariya Takeuchi", "Variety", 1984, 291),
        ("Windy Summer", "Anri", "Timely!!", 1983, 270),
        ("Remember Summer Days", "Anri", "Timely!!", 1983, 252),
        ("Ride on Time", "Tatsuro Yamashita", "Ride on Time", 1980, 355),
        ("Sparkle", "Tatsuro Yamashita", "For You", 1982, 256),
        ("4:00 A.M.", "Taeko Ohnuki", "Sunshower", 1978, 311),
        ("Mayonaka no Door", "Miki Matsubara", "Pocket Park", 1979, 297),
        ("Summer Connection", "Taeko Ohnuki", "Sunshower", 1977, 214),
        ("Last Summer Whisper", "Anri", "Heaven Beach", 1982, 300),
        ("Fly-Day Chinatown", "Yasuha", "Transit", 1981, 262),
        ("Dancing Sea", "Junko Yagami", "Communication", 1983, 285),
        ("Midnight Pretenders", "Tomoko Aran", "Fuyü-Kükan", 1983, 330),
    ];
    let tracks: Vec<crate::local::LocalTrack> = songs
        .iter()
        .enumerate()
        .map(|(i, (t, a, al, y, s))| crate::local::LocalTrack {
            path: crate::paths::documents().join(format!("Demo/{t}.m4a")),
            title: (*t).into(),
            artist: (*a).into(),
            album: (*al).into(),
            duration_ms: s * 1000,
            added: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_760_000_000 + i as u64 * 86_400),
            year: Some(*y),
            genre: "City Pop".into(),
            track_no: (i % 4 + 1) as u32,
            disc_no: 1,
            ..Default::default()
        })
        .collect();
    // `-demoFolders`: Local Files reads real folders instead (UI tests of adding one).
    if real_folders() {
        app.scan_local();
    } else {
        app.local_tracks = Some(Arc::new(tracks));
    }
    // Another Glass on the network, whose songs can be browsed (`share_browse`), unless
    // sharing runs for real (`-demoShare`).
    if !real_sharing() {
        // And a Mac with an older Glass, which can't share until it's updated.
        let peer = |id: &str, name: &str, version| crate::share::Peer { id: id.into(), name: name.into(), addrs: vec![], port: 0, version };
        app.share_event(crate::share::Event::Peers(vec![peer("arch", "Arch Desktop", crate::share::VERSION), peer("mac", "Davis's MacBook Air", 3)]));
    }
    // `-viz`: the visualizer on, fed the desktop demo's made-up signal (demo only:
    // nothing plays in the demo, and a real Glass never fakes it).
    if std::env::args().any(|a| a == "-viz") {
        app.settings.visualizer = true;
        crate::viz::demo_signal();
    }
}

/// The desktop demo's sample lyrics (written for the demo, shaped like an LRCLIB
/// record, so they go through the real parser), answered from memory. The mode comes
/// from the launch argument `-lyrics word|line|plain|instrumental|none|ambiguous`.
fn demo_lyrics(app: &mut MobileApp, track: &Value) {
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
    let lines = ["City lights are slowly fading", "Every corner hums a tune", "We let the evening carry us", "drifting softly past the moon", "Count the windows, one by one", "till the morning finds us here"];
    let start = |i: usize| 52_000 + i as u64 * 5_200;
    let args: Vec<String> = std::env::args().collect();
    let mode = args.iter().position(|a| a == "-lyrics").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "word".into());
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
    app.lyrics.cache = Arc::new(cache);
    app.lyrics.state.reset();
}

/// The demo's other device's songs: a few of this device's (already here) and some it doesn't have.
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
        .map(|(i, (t, a, al, s))| crate::share::FileMeta { id: format!("demo{i}"), name: format!("{t}.flac"), size: *s as u64 * 110_000, title: (*t).into(), artist: (*a).into(), album: (*al).into(), duration_ms: *s as u64 * 1000 })
        .collect()
}

/// `-demoShare`: local files sharing runs for real in the demo, signed in as the demo's
/// account, to share songs with another demo (the desktop's `GLASS_DEMO_SHARE`) on this
/// network. Only other demos have that account: real devices never see them.
pub fn real_sharing() -> bool {
    std::env::args().any(|a| a == "-demoShare")
}

/// `-demoFolders`: the demo's Local Files reads real folders, for UI tests of adding one.
pub fn real_folders() -> bool {
    std::env::args().any(|a| a == "-demoFolders")
}
