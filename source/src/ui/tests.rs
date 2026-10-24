//! Headless UI tests: render the real UI with demo data, click and type
//! like a user, and check what the app did. They run with a private HOME, so
//! no real settings, theme file or account is touched.

use super::root;
use crate::app::{App, LibFilter, NpView, RightPanel, Route};
use crate::lyrics::service::{ProviderSetting, Status};
use crate::lyrics::{Outcome, Timing};
use eframe::egui::{self, Key, Modifiers, accesskit::Role, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use std::path::PathBuf;
use std::sync::Mutex;

/// The tests share HOME, so they take turns.
static LOCK: Mutex<()> = Mutex::new(());

fn home() -> PathBuf {
    crate::config::test_dir("ui-tests")
}

fn harness(size: egui::Vec2, setup: impl FnOnce(&mut App)) -> Harness<'static, App> {
    harness_dt(size, 0.25, setup)
}

/// A harness whose frames are `dt` seconds apart (1/60 to see animations as they run).
fn harness_dt(size: egui::Vec2, dt: f32, setup: impl FnOnce(&mut App)) -> Harness<'static, App> {
    crate::window_fx::set_headless();
    let home = home();
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".config/glass")).unwrap();
    // Reduce Motion (no slides or springs; lyrics only cross-fade), so a few frames settle.
    std::fs::write(home.join(".config/glass/theme.toml"), "[motion]\nreduce = true\n").unwrap();
    // SAFETY: the tests hold LOCK, so nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("GLASS_ROUTE");
        std::env::remove_var("GLASS_THEME_PANEL");
        std::env::remove_var("GLASS_RIGHT");
        std::env::remove_var("GLASS_LYRICS");
    }
    let mut app = App::new(egui::Context::default());
    crate::demo::install(&mut app);
    setup(&mut app);
    let mut h = Harness::builder().with_size(size).with_step_dt(dt).build_ui_state(
        |ui, app: &mut App| {
            app.tick();
            root(app, ui);
            app.apply_actions();
        },
        app,
    );
    let ctx = h.ctx.clone();
    h.state_mut().ctx = ctx;
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'static, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(3);
}

/// Clicks at a point, the way a mouse would.
fn click_at(h: &mut Harness<'static, App>, pos: egui::Pos2) {
    h.hover_at(pos);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
}

/// Focuses the text field showing `value`, replaces its text and presses Enter.
fn retype(h: &mut Harness<'static, App>, value: &str, text: &str) {
    let field = h.query_all_by_role(Role::TextInput).find(|n| n.value().as_deref() == Some(value)).unwrap_or_else(|| panic!("no field showing {value}"));
    field.focus();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
}

/// Runs frames until the lyrics lookup (on the app's runtime) has answered.
fn settle_lyrics(h: &mut Harness<'static, App>) {
    for _ in 0..200 {
        h.run_steps(1);
        if matches!(h.state().lyrics.status, Status::Done(_)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("lyrics never arrived");
}

fn outcome(h: &Harness<'static, App>) -> Outcome {
    match &h.state().lyrics.status {
        Status::Done(r) => r.outcome.clone(),
        s => panic!("no lyrics yet: {s:?}"),
    }
}

fn config_file() -> String {
    std::fs::read_to_string(home().join(".config/glass/config.toml")).unwrap_or_default()
}

fn toasted(h: &Harness<'static, App>, text: &str) -> bool {
    h.state().toasts.iter().any(|(t, _)| t.contains(text))
}

#[test]
fn library_screen() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |_| {});
    assert_eq!(h.state().route, Route::Library, "the library is the start screen");

    click(&mut h, "Sunday Morning");
    assert_eq!(h.state().lib_selected.as_deref(), Some("spotify:playlist:p2"), "a click selects the row");

    click(&mut h, "play");
    assert!(toasted(&h, "Starting playback"), "[ play ] starts the selection (queued until the engine connects)");

    h.key_press(Key::J);
    h.run_steps(2);
    assert_eq!(h.state().lib_selected.as_deref(), Some("spotify:playlist:p4"), "j moves down");
    h.key_press(Key::K);
    h.run_steps(2);
    assert_eq!(h.state().lib_selected.as_deref(), Some("spotify:playlist:p2"), "k moves up");

    click(&mut h, "albums");
    assert_eq!(h.state().lib_filter, LibFilter::Albums);
    click(&mut h, "podcasts");
    assert_eq!(h.state().lib_tab, 1);

    click(&mut h, "Home");
    assert_eq!(h.state().route, Route::Home, "the rail navigates");
}

#[test]
fn command_bar_runs_commands() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.settings.right_open = false);
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    h.run_steps(3);
    assert!(h.state().cmd_open, "⌘K opens the command bar");
    h.event(egui::Event::Text("queue".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().settings.right_open && h.state().right == RightPanel::Queue, "Enter runs the highlighted command");
    assert!(!h.state().cmd_open);
}

#[test]
fn theme_panel_edits_the_file() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.settings.right_open = false);
    let file = home().join(".config/glass/theme.toml");
    h.key_press_modifiers(Modifiers::COMMAND, Key::T);
    h.run_steps(3);
    assert!(h.state().theme_panel, "⌘T opens the theme panel");

    click(&mut h, "accent #5DA9D6");
    assert_eq!(h.state().theme.theme().accent.hex(), "#5DA9D6", "a palette swatch sets the accent");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("accent = \"#5DA9D6\""), "…and writes it to theme.toml:\n{text}");
    assert!(text.contains("reduce = true"), "other settings in the file are kept");

    // Type a new opacity into its field.
    retype(&mut h, "0.45", "0.8");
    assert_eq!(h.state().theme.theme().opacity, 0.8, "typing a value and pressing Enter applies it");

    // A bad value is refused and nothing is written.
    let before = std::fs::read_to_string(&file).unwrap();
    retype(&mut h, "28", "lots");
    assert!(h.state().theme_msg.as_deref().is_some_and(|m| m.contains("theme.blur")), "the error is shown");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "a bad value never reaches the file");

    click(&mut h, "reset");
    assert_eq!(h.state().theme.theme().accent.hex(), "#4F83A3");
    assert_eq!(h.state().theme.theme().opacity, 0.45);
}

#[test]
fn now_playing_and_lyrics() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |_| {});
    assert_eq!(h.state().lyrics.status, Status::Idle, "lyrics aren't looked up while they're off screen");
    h.key_press(Key::L);
    h.run_steps(3);
    assert_eq!(h.state().route, Route::NowPlaying, "L opens now playing");
    assert_eq!(h.state().np_view, NpView::Lyrics);
    settle_lyrics(&mut h);
    assert!(matches!(outcome(&h), Outcome::Found { lyrics, .. } if lyrics.timing == Timing::Word));

    click(&mut h, "City lights are slowly fading");
    assert_eq!(h.state().pb.progress_ms, 52_000, "clicking a lyric line seeks to it");

    // ] shows lyrics earlier for this song only: remembered for it, and seeks account for it.
    h.key_press(Key::CloseBracket);
    h.run_steps(3);
    assert_eq!(h.state().lyrics_song_offset(), 100);
    assert_eq!(h.state().config.lyrics().offset_ms, 0, "the general offset is untouched");
    let choices = crate::lyrics::choices::Choices::path();
    h.state().lyrics_choices.flush();
    assert!(std::fs::read_to_string(&choices).unwrap_or_default().contains("\"offset_ms\":100"), "kept on disk for this song");
    click(&mut h, "Every corner hums a tune");
    assert_eq!(h.state().pb.progress_ms, 57_100, "seeks to where the line shows with the offset");
    h.key_press(Key::OpenBracket);
    h.run_steps(3);
    assert_eq!(h.state().lyrics_song_offset(), 0);

    h.key_press(Key::Q);
    h.run_steps(3);
    assert_eq!(h.state().np_view, NpView::Queue, "Q shows the queue");
    click(&mut h, "lyrics");
    assert_eq!(h.state().np_view, NpView::Lyrics, "the lyrics toggle switches back");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().route, Route::Library, "Esc goes back");
}

#[test]
fn track_rows_play_on_click() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::Playlist("p0".into()));
    h.state_mut().toasts.clear();
    // Click the row's title area (its middle is the album link, which opens the album).
    let r = h.get_by_role_and_label(Role::Button, "Bloom").rect();
    click_at(&mut h, egui::pos2(r.left() + r.width() * 0.2, r.center().y));
    assert!(toasted(&h, "Starting playback"), "one click on a song plays it");
}

#[test]
fn small_window_and_long_titles() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let long = "A very long playlist title that keeps going well past the edge of any reasonable window — and then some more words";
    let mut h = harness(vec2(900.0, 600.0), |app| {
        let key = "ALL2000:/me/playlists?limit=50";
        if let Some(crate::app::Load::Ready(v)) = app.data.get(key) {
            let mut v = (**v).clone();
            let mut p = v["items"][0].clone();
            p["name"] = serde_json::json!(long);
            p["uri"] = serde_json::json!("spotify:playlist:long");
            v["items"].as_array_mut().unwrap().insert(0, p);
            app.data.insert(key.into(), crate::app::Load::Ready(std::sync::Arc::new(v)));
        }
    });
    h.get_by_role_and_label(Role::Button, long).scroll_to_me();
    h.run_steps(3);
    let r = h.get_by_role_and_label(Role::Button, long).rect();
    click_at(&mut h, egui::pos2(r.left() + r.width() * 0.3, r.center().y));
    assert_eq!(h.state().lib_selected.as_deref(), Some("spotify:playlist:long"));
    h.get_by_role_and_label(Role::Button, "downloaded");
    for route in [Route::NowPlaying, Route::Home, Route::Playlist("p0".into()), Route::Artist("a0".into()), Route::Search, Route::Settings, Route::LikedSongs] {
        h.state_mut().route = route.clone();
        h.run_steps(3);
    }
}

/// Spotify's lyrics for the demo song, from a fake Spotify connection.
fn spotify_with_lyrics(h: &mut Harness<'static, App>) {
    let mut f = crate::ai::spotify::tests::fake();
    f.data.insert(
        "/glass/lyrics/t0".into(),
        serde_json::json!({ "lyrics": { "syncType": "LINE_SYNCED", "language": "en", "providerLyricsId": "mx1", "lines": [
            { "startTimeMs": "52000", "words": "Spotify's first line", "endTimeMs": "0" },
            { "startTimeMs": "57200", "words": "Spotify's second line", "endTimeMs": "0" }
        ] } }),
    );
    h.state_mut().ai_source_override = Some(std::sync::Arc::new(f));
}

#[test]
fn spotify_lyrics_are_a_source_again() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::NowPlaying);
    spotify_with_lyrics(&mut h);
    settle_lyrics(&mut h);
    assert!(matches!(outcome(&h), Outcome::Found { lyrics, .. } if lyrics.source.provider == crate::lyrics::ProviderId::Lrclib), "LRCLIB first");

    // Spotify only: its lyrics, through Glass's Spotify connection.
    std::fs::write(home().join(".config/glass/config.toml"), "[lyrics]\nprovider = \"spotify\"\n").unwrap();
    h.state_mut().config.reload();
    settle_lyrics(&mut h);
    assert!(matches!(outcome(&h), Outcome::Found { lyrics, .. } if lyrics.source.provider == crate::lyrics::ProviderId::Spotify && lyrics.lines[0].text == "Spotify's first line"));
    h.get_by_label("Spotify's first line");

    // The Settings chips write config.toml, and Spotify can be chosen again.
    h.state_mut().route = Route::Settings;
    h.state_mut().settings_query = "lyrics".into();
    h.run_steps(3);
    click(&mut h, "LRCLIB");
    assert_eq!(h.state().config.lyrics().provider, ProviderSetting::Lrclib);
    assert!(config_file().contains("provider = \"lrclib\""));
    click(&mut h, "Spotify");
    assert_eq!(h.state().config.lyrics().provider, ProviderSetting::Spotify);
}

#[test]
fn spotify_lyrics_need_a_spotify_song() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::NowPlaying);
    std::fs::write(home().join(".config/glass/config.toml"), "[lyrics]\nprovider = \"spotify\"\n").unwrap();
    h.state_mut().config.reload();
    // Demo data has no lyrics service: an honest failure, not LRCLIB's lyrics instead.
    settle_lyrics(&mut h);
    assert!(matches!(outcome(&h), Outcome::Failed { provider: crate::lyrics::ProviderId::Spotify, .. } | Outcome::NotFound { provider: crate::lyrics::ProviderId::Spotify }), "{:?}", outcome(&h));
}

#[test]
fn a_new_song_drops_the_old_lookup() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| {
        a.route = Route::NowPlaying;
        // A song the offline cache doesn't know: its lookup fails slowly (retries).
        let mut other = a.pb.item.as_deref().cloned().unwrap();
        other["uri"] = serde_json::json!("spotify:track:other");
        other["name"] = serde_json::json!("Something Else");
        a.pb.item = Some(std::sync::Arc::new(other));
    });
    h.run_steps(2);
    assert_eq!(h.state().lyrics.status, Status::Loading);
    // The song changes while that lookup is in flight.
    let t0 = h.state().queue.as_ref().unwrap()["currently_playing"].clone();
    h.state_mut().pb.item = Some(std::sync::Arc::new(t0));
    settle_lyrics(&mut h);
    assert_eq!(h.state().lyrics.track_key(), Some("spotify:track:t0"));
    assert!(matches!(outcome(&h), Outcome::Found { .. }));
    // Long enough for the first lookup to have failed, had it not been stopped.
    std::thread::sleep(std::time::Duration::from_millis(3500));
    h.run_steps(3);
    assert!(matches!(outcome(&h), Outcome::Found { .. }), "the old song's answer never replaces the new one");
}

#[test]
fn several_matches_are_left_to_the_user() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::NowPlaying);
    // Two records that fit equally well but have different words.
    {
        let app = h.state_mut();
        let t = app.lyrics_track().unwrap();
        let rec = |id: u64, album: &str, line: &str| {
            crate::lyrics::parse::record(
                &serde_json::json!({ "id": id, "trackName": t.title, "artistName": t.artist(), "albumName": album, "duration": t.duration_ms / 1000, "syncedLyrics": format!("[00:52.00] {line}\n") }),
            )
            .unwrap()
        };
        let cache = crate::lyrics::cache::Cache::in_memory();
        cache.put(&t, &crate::lyrics::lrclib::Lookup::Ambiguous(vec![rec(1, "First", "City lights are slowly fading"), rec(2, "Second", "A different song entirely")]));
        app.offline_lyrics(cache);
    }
    settle_lyrics(&mut h);
    assert!(matches!(outcome(&h), Outcome::Ambiguous { .. }));
    h.run_steps(2);
    let title = h.state().lyrics_track().unwrap().title;
    let artist = h.state().lyrics_track().unwrap().artist().to_string();
    // Both candidates are listed; picking one shows it at once and remembers it.
    assert_eq!(h.query_all_by_label(&format!("{title} — {artist}")).count(), 2);
    let second = h.query_all_by_label(&format!("{title} — {artist}")).nth(1).unwrap().rect();
    click_at(&mut h, second.center());
    match outcome(&h) {
        Outcome::Found { lyrics, how } => {
            assert_eq!(how, crate::lyrics::Match::Manual);
            assert_eq!(lyrics.lines[0].text, "A different song entirely");
            assert_eq!(lyrics.timing, Timing::Line);
        }
        o => panic!("{o:?}"),
    }
    let t = h.state().lyrics_track().unwrap();
    assert_eq!(h.state().lyrics_choices.pick(&t).flatten().map(|r| r.id), Some(2), "the pick is kept for the song");
}

/// Hovers first and then clicks, like a real mouse (press and release in
/// separate frames), at `pos`.
fn real_click(h: &mut Harness<'static, App>, pos: egui::Pos2) {
    h.hover_at(pos);
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn the_play_spot_on_a_track_row_plays_it() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::Playlist("p0".into()));
    h.state_mut().toasts.clear();
    // Where the row number turns into ▶ on hover.
    let r = h.get_by_role_and_label(Role::Button, "Bloom").rect();
    real_click(&mut h, egui::pos2(r.left() + 24.0, r.center().y));
    assert!(toasted(&h, "Starting playback"), "clicking ▶ in the number column plays the song");
}

#[test]
fn the_library_play_button_plays() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |_| {});
    h.state_mut().toasts.clear();
    let r = h.get_by_role_and_label(Role::Button, "Sunday Morning").rect();
    real_click(&mut h, egui::pos2(r.left() + 22.0, r.center().y));
    assert!(toasted(&h, "Starting playback"), "▶ on a library row plays it");
    assert_eq!(h.state().route, Route::Library, "…without leaving the library");
}

#[test]
fn songs_can_be_played_from_the_keyboard() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::Playlist("p0".into()));
    h.state_mut().toasts.clear();
    for k in [Key::J, Key::J, Key::ArrowDown, Key::K] {
        h.key_press(k);
        h.run_steps(2);
    }
    assert_eq!(h.state().row_cursor, Some(1), "j/↓ move down, k moves up");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(toasted(&h, "Starting playback"), "Enter plays the row under the cursor");
}

#[test]
fn back_from_now_playing_keeps_your_place() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(900.0, 600.0), |a| a.route = Route::Playlist("p0".into()));
    assert!(h.query_by_role_and_label(Role::Button, "Undertow").is_none(), "starts at the top; the last song is off screen");
    // Walk to the last song; the list scrolls with the cursor.
    for _ in 0..14 {
        h.key_press(Key::J);
        h.run_steps(2);
    }
    h.run_steps(4);
    assert!(h.query_by_role_and_label(Role::Button, "Undertow").is_some(), "the list followed the cursor down");
    h.key_press(Key::L);
    h.run_steps(3);
    assert_eq!(h.state().route, Route::NowPlaying);
    h.key_press(Key::L);
    h.run_steps(4);
    assert_eq!(h.state().route, Route::Playlist("p0".into()));
    assert!(h.query_by_role_and_label(Role::Button, "Undertow").is_some(), "the playlist is where it was, not back at the top");
    assert_eq!(h.state().row_cursor, Some(13), "and so is the cursor (the first j lands on row 0)");
}

#[test]
fn the_queue_shows_and_plays_from_now_playing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |_| {});
    h.key_press(Key::Q);
    h.run_steps(3);
    assert_eq!((h.state().route.clone(), h.state().np_view), (Route::NowPlaying, NpView::Queue));
    assert!(h.query_by_role_and_label(Role::Button, "Says").is_some(), "queued songs are listed");
    h.state_mut().toasts.clear();
    h.key_press(Key::J);
    h.run_steps(2);
    let cur = h.ctx.data(|d| d.get_temp::<Option<usize>>(egui::Id::new("queue-cursor")));
    assert_eq!(cur, Some(Some(0)), "j puts the cursor on the first queued song");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(toasted(&h, "Starting playback"), "Enter jumps to the song under the cursor: {:?}", h.state().toasts);
}

/// A tiny valid MP3 (silent MPEG-1 Layer III frames), tagged.
fn local_song(dir: &std::path::Path, file: &str, title: &str, artist: &str) -> std::path::PathBuf {
    let mut frame = vec![0xFF, 0xFB, 0x90, 0x64];
    frame.resize(417, 0);
    let path = dir.join(file);
    std::fs::write(&path, frame.repeat(40)).unwrap();
    let tags = crate::local::Tags { title: title.into(), artist: artist.into(), ..Default::default() };
    crate::local::write_tags(&path, &tags, &crate::local::Cover::Keep).unwrap();
    path
}

fn settle_local(h: &mut Harness<'static, App>) {
    for _ in 0..200 {
        h.run_steps(1);
        // A rescan keeps the old list on screen until the new one is in.
        let a = h.state();
        if a.local_tracks.is_some() && !a.local_scanning && !a.local_rescan {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("local files never scanned");
}

#[test]
fn playlist_local_songs_find_copied_files() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let music = home().join("Music");
    let mut h = harness(vec2(1280.0, 820.0), |a| {
        // A playlist with a local song (as Spotify describes it: artist, album, title, seconds).
        let key = "ALL10000:/playlists/p0/tracks?limit=100";
        if let Some(crate::app::Load::Ready(v)) = a.data.get(key) {
            let mut v = (**v).clone();
            let local = crate::sp::conv::local_track("spotify:local:Ana+Tijoux::Somos+Sur:1");
            v["items"].as_array_mut().unwrap().insert(0, serde_json::json!({ "track": local, "added_at": "" }));
            a.data.insert(key.into(), crate::app::Load::Ready(std::sync::Arc::new(v)));
        }
        a.route = Route::Playlist("p0".into());
    });
    std::fs::create_dir_all(&music).unwrap();
    {
        let app = h.state_mut();
        app.settings.local_dirs = vec![music.clone()];
        app.act(crate::app::Action::RescanLocal);
    }
    settle_local(&mut h);
    assert!(matches!(h.state_mut().local_link("spotify:local:Ana+Tijoux::Somos+Sur:1"), Some(None)), "no file yet");
    // Copy the file in (from a phone, say) and rescan.
    local_song(&music, "somos sur.mp3", "Somos Sur", "Ana Tijoux");
    h.state_mut().act(crate::app::Action::RescanLocal);
    settle_local(&mut h);
    let found = h.state_mut().local_link("spotify:local:Ana+Tijoux::Somos+Sur:1");
    assert!(found.is_some_and(|f| f.is_some_and(|t| t.path.ends_with("somos sur.mp3"))), "the playlist's local song is linked to the copied file");
    h.state_mut().toasts.clear();
    let r = h.get_by_role_and_label(Role::Button, "Somos Sur").rect();
    click_at(&mut h, egui::pos2(r.left() + r.width() * 0.2, r.center().y));
    assert!(toasted(&h, "Starting playback"), "and it plays");
}

#[test]
fn local_songs_go_into_playlists() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let music = home().join("Music");
    let mut h = harness(vec2(1280.0, 820.0), |a| {
        a.route = Route::LocalFiles;
        a.settings.show_local_files = true;
        a.settings.local_view = crate::config::LocalView::Songs;
    });
    std::fs::create_dir_all(&music).unwrap();
    let a = local_song(&music, "a.mp3", "Plastic Love", "Mariya Takeuchi");
    let b = local_song(&music, "b.mp3", "Mayonaka no Door", "Miki Matsubara");
    {
        let app = h.state_mut();
        app.settings.local_dirs = vec![music.clone()];
        app.act(crate::app::Action::RescanLocal);
    }
    settle_local(&mut h);
    // Scroll the songs clear of the player bar.
    let r = h.get_by_role_and_label(Role::Button, "Mayonaka no Door").rect();
    h.hover_at(r.center());
    h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -400.0), phase: egui::TouchPhase::Move, modifiers: Modifiers::NONE });
    h.run_steps(4);
    // ⌘-click both songs (nothing plays), then right-click one of them.
    for title in ["Mayonaka no Door", "Plastic Love"] {
        h.run_steps(2);
        let r = h.get_by_role_and_label(Role::Button, title).rect();
        let pos = egui::pos2(r.left() + r.width() * 0.2, r.center().y);
        h.hover_at(pos);
        h.run_steps(1);
        // Held for the whole frame, as a real ⌘ is.
        h.event(egui::Event::ModifiersChanged(Modifiers::COMMAND));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::COMMAND });
        }
        h.run_steps(2);
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
    }
    assert_eq!(h.state().local_selection.len(), 2);
    h.get_by_role_and_label(Role::Button, "Plastic Love").click_secondary();
    h.run_steps(3);
    h.get_by_label("Add 2 songs to playlist ⏵").click();
    h.run_steps(3);
    h.get_by_label("＋  New local playlist").click();
    h.run_steps(3);
    let p = h.state().playlists.list.last().cloned().expect("a local playlist");
    let mut got = p.tracks.clone();
    got.sort();
    let mut want = vec![a.clone(), b.clone()];
    want.sort();
    assert_eq!(got, want, "both selected songs");
    assert_eq!(h.state().route, Route::LocalPlaylist(p.id.clone()));
    // Adding one again to that playlist changes nothing.
    h.state_mut().act(crate::app::Action::AddToLocalPlaylist { id: p.id.clone(), paths: vec![a.clone()] });
    h.run_steps(2);
    assert!(toasted(&h, "Already in"));
    assert_eq!(h.state().playlists.get(&p.id).unwrap().tracks.len(), 2);
}

#[test]
fn local_files_tab_and_sending_to_your_other_devices() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let music = home().join("Music");
    let mut h = harness(vec2(1280.0, 820.0), |a| {
        a.route = Route::Home;
        a.settings.show_local_files = true;
        // Off by default: turned on here.
        a.settings.share_local_files = true;
    });
    std::fs::create_dir_all(&music).unwrap();
    local_song(&music, "a.mp3", "Plastic Love", "Mariya Takeuchi");
    h.state_mut().settings.local_dirs = vec![music.clone()];
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Local Files").click();
    h.run_steps(2);
    assert_eq!(h.state().route, Route::LocalFiles, "its own tab");
    h.state_mut().settings.local_view = crate::config::LocalView::Songs;
    h.state_mut().act(crate::app::Action::RescanLocal);
    settle_local(&mut h);
    h.state_mut().share_peers = vec![crate::share::Peer { id: "p1".into(), name: "Arch Desktop".into(), addrs: vec![], port: 1 }];
    h.run_steps(2);
    assert!(h.query_by_label_contains("On this network: Arch Desktop").is_some(), "your other devices are listed");
    h.get_by_role_and_label(Role::Button, "Plastic Love").click_secondary();
    h.run_steps(3);
    h.get_by_label("Send to ⏵").click();
    h.run_steps(3);
    h.get_by_label("Arch Desktop").click();
    h.run_steps(3);
    // No live sharing in tests: the device is gone by the time it's asked.
    assert!(toasted(&h, "isn't on this network any more"), "the send went through to sharing");
}

#[test]
fn local_albums_open_with_their_songs_in_track_order() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let music = home().join("Music");
    let mut h = harness(vec2(1280.0, 1000.0), |a| {
        a.route = Route::LocalFiles;
        a.settings.show_local_files = true;
        a.settings.local_view = crate::config::LocalView::Albums;
    });
    std::fs::create_dir_all(&music).unwrap();
    for (file, title, track) in [("b.mp3", "Plastic Love", 2), ("a.mp3", "Tokyo Rose", 1)] {
        let p = local_song(&music, file, title, "Mariya Takeuchi");
        let tags = crate::local::Tags { title: title.into(), artist: "Mariya Takeuchi".into(), album: "Variety".into(), ..Default::default() };
        crate::local::write_tags(&p, &tags, &crate::local::Cover::Keep).unwrap();
        // The track number, as a tagger would set it.
        use lofty::prelude::*;
        let mut f = lofty::read_from_path(&p).unwrap();
        f.primary_tag_mut().unwrap().set_track(track);
        f.save_to_path(&p, lofty::config::WriteOptions::default()).unwrap();
    }
    h.state_mut().settings.local_dirs = vec![music.clone()];
    h.state_mut().act(crate::app::Action::RescanLocal);
    settle_local(&mut h);
    h.get_by_role_and_label(Role::Button, "Variety").click();
    h.run_steps(3);
    assert_eq!(h.state().route, Route::LocalAlbum { artist: "Mariya Takeuchi".into(), album: "Variety".into() });
    let y = |title: &str| h.get_by_role_and_label(Role::Button, title).rect().top();
    assert!(y("Tokyo Rose") < y("Plastic Love"), "track 1 before track 2");
}

/// A file dropped from Finder, as eframe hands it over.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);
impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn files_dropped_on_a_playlist_join_it() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.settings.show_local_files = true);
    // A song somewhere that isn't a Local Files source yet.
    let elsewhere = home().join("Downloads");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let song = local_song(&elsewhere, "new.mp3", "Stay With Me", "Miki Matsubara");
    std::fs::write(elsewhere.join("notes.txt"), "not music").unwrap();
    let id = {
        let app = h.state_mut();
        app.settings.local_dirs.clear();
        let (id, _) = app.playlists.create("Mix", "", vec![], None).unwrap();
        app.route = Route::LocalPlaylist(id.clone());
        id
    };
    h.run_steps(2);
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(song.clone())));
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(elsewhere.join("notes.txt"))));
    h.run_steps(1);
    for _ in 0..200 {
        h.run_steps(1);
        if !h.state().playlists.get(&id).unwrap().tracks.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(h.state().playlists.get(&id).unwrap().tracks, vec![song.clone()], "the song, not the text file");
    assert_eq!(h.state().settings.local_dirs, vec![elsewhere.clone()], "its folder became a source, so it plays");
    assert!(toasted(&h, "Added to “Mix”"));
}

#[test]
fn local_song_info_can_be_edited() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let music = home().join("Music");
    let mut h = harness(vec2(1280.0, 820.0), |a| {
        a.route = Route::LocalFiles;
        a.settings.show_local_files = true;
        a.settings.local_view = crate::config::LocalView::Songs;
    });
    std::fs::create_dir_all(&music).unwrap();
    let path = local_song(&music, "track01.mp3", "Track 01", "Unknown");
    {
        let app = h.state_mut();
        app.settings.local_dirs = vec![music.clone()];
        app.act(crate::app::Action::RescanLocal);
    }
    settle_local(&mut h);
    assert!(h.query_by_role_and_label(Role::Button, "Track 01").is_some());
    // j then E: the editor for the first song.
    h.key_press(Key::J);
    h.run_steps(2);
    h.key_press(Key::E);
    h.run_steps(3);
    assert!(matches!(h.state().modal, Some(crate::app::Modal::EditTags(_))), "E opens the editor");
    // Fill the fields (Enter in any of them saves, so it comes last).
    // Title, artist, album, year, genre, in that order.
    for (index, to) in [(0, "Paper Planes"), (1, "M.I.A."), (3, "2007")] {
        // The editor's five fields come after the page's own (filter, command bar).
        let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
        let field = &inputs[inputs.len() - 5 + index];
        field.focus();
        h.run_steps(2);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(to.into()));
        h.run_steps(2);
    }
    h.key_press(Key::Enter);
    for _ in 0..100 {
        h.run_steps(1);
        if h.state().modal.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(h.state().modal.is_none(), "saving closes the editor");
    assert!(toasted(&h, "Saved song info"));
    let t = h.state().local_tracks.as_ref().unwrap().iter().find(|t| t.path == path).unwrap().clone();
    assert_eq!((t.title.as_str(), t.artist.as_str(), t.year), ("Paper Planes", "M.I.A.", Some(2007)), "written to the file and read back");
}

// ------------------------------------------------------------ assistant

use crate::ai::agent::tests::{mock, sse};
use crate::ai::{Line, Models};

const PLAN_TOKEN: &str = "test-access-token-not-real";

/// The app with three local songs, the assistant panel open, signed in to a
/// mock ChatGPT account whose requests go to `replies` (a mock OpenAI).
fn ai_harness(replies: Vec<(u16, String)>, enable: bool) -> (Harness<'static, App>, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>, Vec<std::path::PathBuf>) {
    let music = home().join("Music");
    let mut h = harness(vec2(1280.0, 820.0), |a| {
        a.settings.right_open = true;
        a.right = RightPanel::Assistant;
        a.route = Route::LocalFiles;
        a.settings.show_local_files = true;
        a.settings.local_view = crate::config::LocalView::Songs;
    });
    std::fs::create_dir_all(&music).unwrap();
    let files = vec![
        local_song(&music, "plastic.mp3", "Plastic Love", "Mariya Takeuchi"),
        local_song(&music, "remember.mp3", "Remember Summer Days", "Anri"),
        local_song(&music, "windy.mp3", "Windy Summer", "Anri"),
    ];
    let store = std::sync::Arc::new(crate::ai::secrets::MemoryStore::default());
    crate::ai::secrets::save_tokens(
        store.as_ref(),
        "oaiapp_test",
        &crate::ai::secrets::TokenSet {
            access_token: PLAN_TOKEN.into(),
            refresh_token: "r".into(),
            id_token: "i".into(),
            expires_at: crate::ai::oauth::now() + 3600,
            earliest_refresh_at: 0,
            scopes: vec!["openid".into(), "chatgpt.tokens.use.direct".into()],
        },
    )
    .unwrap();
    let (base, seen) = {
        let app = h.state_mut();
        app.settings.local_dirs = vec![music.clone()];
        app.act(crate::app::Action::RescanLocal);
        app.rt.block_on(mock(replies))
    };
    {
        let app = h.state_mut();
        app.ai_use_test_backend(store, crate::ai::account::Endpoints { api: base, ..Default::default() });
        app.ai.accounts.upsert(crate::ai::account::Registration { client_id: "oaiapp_test".into(), subject: Some("user-1".into()), email: Some("me@example.com".into()), plan_enabled: true });
        app.ai.accounts.set_active(Some("oaiapp_test".into()));
        // Set either way, before the next frame: tests don't depend on the default.
        app.act(crate::app::Action::AiEnable(enable));
        app.apply_actions();
    }
    settle_local(&mut h);
    (h, seen, files)
}

fn settle(h: &mut Harness<'static, App>, done: impl Fn(&App) -> bool) {
    for _ in 0..800 {
        h.run_steps(1);
        if done(h.state()) {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out; transcript: {:?}", h.state().ai.transcript);
}

fn models_reply() -> (u16, String) {
    (
        200,
        serde_json::json!({ "models": [{ "slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list" }, { "slug": "gpt-6-luna", "display_name": "GPT-6 Luna", "visibility": "list" }] })
            .to_string(),
    )
}

/// Types into the composer and presses Enter.
fn ask(h: &mut Harness<'static, App>, text: &str) {
    h.state_mut().ai.focus_input = true;
    h.run_steps(2);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(2);
}

#[test]
fn the_assistant_sends_nothing_while_its_off() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut h, seen, _) = ai_harness(vec![models_reply()], false);
    h.run_steps(5);
    assert!(!h.state().ai_enabled(), "turned off");
    h.get_by_label("The assistant is off. It's optional: turn it on in Settings → Assistant.");
    assert!(seen.lock().unwrap().is_empty(), "no request to OpenAI while it's off");
    // Playback is unaffected.
    h.state_mut().toasts.clear();
    h.state_mut().route = Route::Playlist("p0".into());
    h.run_steps(3);
    let r = h.get_by_role_and_label(Role::Button, "Bloom").rect();
    click_at(&mut h, egui::pos2(r.left() + r.width() * 0.2, r.center().y));
    assert!(toasted(&h, "Starting playback"));
}

#[test]
fn a_request_builds_a_preview_that_saves_once() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let music = home().join("Music");
    let (remember, windy) = (crate::ai::tools::track_id(&music.join("remember.mp3")), crate::ai::tools::track_id(&music.join("windy.mp3")));
    let search = serde_json::json!({ "query": "", "artist": "anri", "genre": null, "year_from": null, "year_to": null, "limit": null });
    let propose = serde_json::json!({ "name": "Anri Night Drive", "description": null, "track_ids": [remember, windy, "t_made_up_by_model"], "replaces_playlist_id": null });
    let replies = vec![
        models_reply(),
        (200, sse("", &[("c1", "music.search_library", search)])),
        (200, sse("", &[("c2", "propose_playlist", propose)])),
        (200, sse("Here's a 2-song Anri preview. Save it if it looks right.", &[])),
    ];
    let (mut h, seen, files) = ai_harness(replies, true);
    // Local files only: with Spotify data off, nothing from Spotify is sent.
    h.state_mut().act(crate::app::Action::AiUseSpotify(false));
    settle(&mut h, |a| matches!(a.ai.models, Models::Ready(_)));
    assert_eq!(h.state().ai_model().as_deref(), Some("gpt-6-luna"), "Luna preferred when offered");
    h.get_by_label("Using ChatGPT plan");
    h.get_by_label("Manage usage");

    ask(&mut h, "Make a night drive playlist from my Anri songs");
    assert!(h.state().ai.busy());
    settle(&mut h, |a| !a.ai.busy());
    let app = h.state();
    assert!(matches!(app.ai.transcript.last(), Some(Line::Assistant(t)) if t.contains("2-song Anri preview")), "{:?}", app.ai.transcript);
    assert_eq!(app.ai.drafts.len(), 1);
    assert_eq!(app.ai.drafts[0].tracks, vec![remember.clone(), windy.clone()], "the made-up ID never made it in");
    assert!(app.playlists.list.is_empty(), "nothing saved without approval");

    // What was sent: plan-path rules, tags but no paths, no Spotify data.
    let bodies = seen.lock().unwrap().clone();
    assert_eq!(bodies.len(), 4, "models + three Responses requests");
    for b in &bodies[1..] {
        assert_eq!((b["store"].as_bool(), b["stream"].as_bool()), (Some(false), Some(true)));
        assert_eq!(b["model"], "gpt-6-luna");
        let text = b.to_string();
        assert!(!text.contains(&music.display().to_string()) && !text.contains(".mp3"), "no file paths leave the app");
        assert!(!text.contains("Bloom") && !text.contains("Golden Hour") && !text.contains("spotify:track:t") && !text.contains("spotify:playlist:p"), "no Spotify data");
        assert!(!text.contains(PLAN_TOKEN));
    }
    assert!(bodies[2].to_string().contains("Remember Summer Days"), "tool results carry the tags");
    // No secrets in the activity log.
    assert!(app.log.snapshot().iter().all(|e| !e.path.contains(PLAN_TOKEN) && !e.path.contains("Bearer")));

    // The preview shows real songs and their total length; saving is the approval.
    h.get_by_label("Remember Summer Days — Anri");
    click(&mut h, "save");
    let app = h.state();
    assert_eq!(app.playlists.list.len(), 1);
    let p = &app.playlists.list[0];
    assert_eq!((p.name.as_str(), p.tracks.clone()), ("Anri Night Drive", vec![files[1].clone(), files[2].clone()]));
    // A retried save (or a second click) makes no second playlist.
    let draft = app.ai.drafts[0].id.clone();
    h.state_mut().act(crate::app::Action::AiSaveDraft { draft, replace: None });
    h.run_steps(2);
    assert_eq!(h.state().playlists.list.len(), 1, "no duplicate");
    // The model is told what actually happened, next turn.
    assert!(h.state().ai.history.iter().any(|m| m["content"].as_str().is_some_and(|c| c.contains("Saved local playlist “Anri Night Drive”"))));
    // Undo removes it again.
    h.get_by_label("undo").click();
    h.run_steps(3);
    assert!(h.state().playlists.list.is_empty());
}

#[test]
fn the_usage_limit_pauses_and_never_switches_to_an_api_key() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let replies = vec![models_reply(), (429, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"limit"}}"#.into())];
    let (mut h, seen, _) = ai_harness(replies, true);
    // An API key is saved too: it must not be used.
    h.state_mut().ai.auth.store.set(crate::ai::secrets::API_KEY_KEY, "sk-should-not-be-used").unwrap();
    h.state_mut().ai.set_has_api_key(true);
    settle(&mut h, |a| matches!(a.ai.models, Models::Ready(_)));
    ask(&mut h, "anything");
    settle(&mut h, |a| !a.ai.busy());
    let app = h.state();
    assert!(app.ai.limit_reached);
    assert_eq!(app.ai_billing(), crate::ai::Billing::ChatGptPlan, "billing unchanged");
    h.run_steps(2);
    h.get_by_label("Usage limit reached");
    h.get_by_role_and_label(Role::Button, "Manage usage");
    // Further requests are paused, not retried or rerouted.
    h.state_mut().act(crate::app::Action::AiSend("again".into()));
    h.run_steps(5);
    assert_eq!(seen.lock().unwrap().len(), 2, "models + the one request");
}

#[test]
fn cancelling_drops_late_results() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // A server that accepts the request and never answers.
    let (mut h, _, _) = ai_harness(vec![models_reply()], true);
    settle(&mut h, |a| matches!(a.ai.models, Models::Ready(_)));
    let hang = h.state().rt.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = l.accept().await {
                held.push(s);
            }
        });
        format!("http://{addr}")
    });
    h.state_mut().ai.auth = std::sync::Arc::new(crate::ai::account::Auth::new(h.state().ai.auth.store.clone(), crate::ai::account::Endpoints { api: hang, ..Default::default() }));
    ask(&mut h, "make a playlist");
    assert!(h.state().ai.busy());
    let old = h.state().ai.generation;
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().ai.busy(), "Esc stops it");
    assert!(matches!(h.state().ai.transcript.last(), Some(Line::Note(n)) if n == "Stopped."));
    // A late answer from the cancelled request changes nothing.
    let before = h.state().ai.transcript.clone();
    h.state_mut().ai_event(old, crate::assistant_app::AiEvent::Turn(crate::ai::agent::Event::Text("late".into())));
    h.state_mut().ai_event(
        old,
        crate::assistant_app::AiEvent::Turn(crate::ai::agent::Event::Draft(crate::ai::tools::Draft {
            id: "d9".into(),
            name: "x".into(),
            description: String::new(),
            tracks: vec![],
            replaces: None,
            dest: crate::ai::tools::Dest::Local,
            status: crate::ai::tools::DraftStatus::Editing,
        })),
    );
    assert_eq!(h.state().ai.transcript, before);
    assert!(h.state().ai.drafts.is_empty());
}

#[test]
fn offline_the_assistant_fails_politely_and_playback_carries_on() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut h, _, _) = ai_harness(vec![models_reply()], true);
    settle(&mut h, |a| matches!(a.ai.models, Models::Ready(_)));
    // OpenAI unreachable from here on.
    h.state_mut().ai.auth =
        std::sync::Arc::new(crate::ai::account::Auth::new(h.state().ai.auth.store.clone(), crate::ai::account::Endpoints { api: "http://127.0.0.1:9".into(), ..Default::default() }));
    ask(&mut h, "hello");
    assert!(h.state().ai.busy());
    // While it retries, the player still responds at once.
    h.state_mut().toasts.clear();
    h.state_mut().act(crate::app::Action::PlayLocal { index: 0 });
    h.run_steps(2);
    assert!(h.state().local.as_ref().is_some_and(|l| l.active) || toasted(&h, "audio output"), "local playback starts while the AI request is pending");
    settle(&mut h, |a| !a.ai.busy());
    assert!(matches!(h.state().ai.transcript.last(), Some(Line::Error(e)) if e.contains("Couldn't reach OpenAI")), "{:?}", h.state().ai.transcript);
}

#[test]
fn without_luna_the_user_chooses_the_model() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let only_sol = (200, serde_json::json!({ "models": [{ "slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list" }] }).to_string());
    let (mut h, seen, _) = ai_harness(vec![only_sol], true);
    settle(&mut h, |a| matches!(a.ai.models, Models::Ready(_)));
    assert_eq!(h.state().ai_model(), None, "no silent pick of another model");
    ask(&mut h, "hi");
    assert!(matches!(h.state().ai.transcript.last(), Some(Line::Error(e)) if e.contains("Choose a model")));
    assert_eq!(seen.lock().unwrap().len(), 1, "nothing sent");
    // Choosing it is remembered in config.toml.
    h.state_mut().act(crate::app::Action::AiSetModel("gpt-6.1-sol".into()));
    h.run_steps(2);
    assert_eq!(h.state().ai_model().as_deref(), Some("gpt-6.1-sol"));
    assert!(config_file().contains("model = \"gpt-6.1-sol\""));
}

#[test]
fn sign_in_states_are_explained() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut h, _, _) = ai_harness(vec![], true);
    // Plan permission not granted: signed in, but no requests.
    let r = h.state().ai.accounts.active().cloned().unwrap();
    h.state_mut().ai.accounts.upsert(crate::ai::account::Registration { plan_enabled: false, ..r });
    h.run_steps(3);
    assert!(h.state().ai_ready().unwrap_err().contains("permission to use your ChatGPT plan wasn't granted"));
    // Not signed in at all.
    h.state_mut().ai.accounts.set_active(None);
    h.run_steps(3);
    assert!(h.state().ai_ready().unwrap_err().contains("Sign in with ChatGPT"));
    // A declined consent in the browser leaves things as they were, with a note.
    h.state_mut().ai_signed_in(Err(crate::ai::oauth::AuthError::Declined));
    assert!(h.state().ai.account_note.as_deref().unwrap().contains("declined"));
    assert!(h.state().ai.accounts.active().is_none());
    // A sign-in without plan permission keeps the identity but disables the assistant.
    h.state_mut().ai_signed_in(Ok(crate::ai::account::SignedIn {
        registration: crate::ai::account::Registration { client_id: "oaiapp_2".into(), subject: Some("u2".into()), email: None, plan_enabled: false },
        plan_enabled: false,
    }));
    assert!(h.state().ai_ready().is_err());
    assert!(h.state().modal.is_none(), "no 'using your plan' message without the permission");
    // With the permission: the one-time welcome.
    h.state_mut().ai_signed_in(Ok(crate::ai::account::SignedIn {
        registration: crate::ai::account::Registration { client_id: "oaiapp_3".into(), subject: Some("u3".into()), email: None, plan_enabled: true },
        plan_enabled: true,
    }));
    assert!(matches!(h.state().modal, Some(crate::app::Modal::AiWelcome)));
    assert!(config_file().contains("welcomed = true"), "shown once");
}

#[test]
fn replacing_a_playlist_needs_approval_and_can_be_undone() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut h, _, files) = ai_harness(vec![], true);
    let music = home().join("Music");
    let ids: Vec<String> = ["plastic.mp3", "remember.mp3", "windy.mp3"].iter().map(|f| crate::ai::tools::track_id(&music.join(f))).collect();
    let app = h.state_mut();
    let (pid, _) = app.playlists.create("Drive", "", vec![files[0].clone(), files[1].clone()], None).unwrap();
    // The model proposed a calmer version and asked to replace (as the tools would report).
    let draft = crate::ai::tools::Draft {
        id: "d1".into(),
        name: "Drive".into(),
        description: String::new(),
        tracks: vec![ids[1].clone(), ids[2].clone()],
        replaces: Some(pid.clone()),
        dest: crate::ai::tools::Dest::Local,
        status: crate::ai::tools::DraftStatus::AwaitingApproval,
    };
    app.ai.generation = 1;
    app.ai_event(1, crate::assistant_app::AiEvent::Turn(crate::ai::agent::Event::Draft(draft)));
    app.ai_event(
        1,
        crate::assistant_app::AiEvent::Turn(crate::ai::agent::Event::Approval(crate::ai::tools::Approval {
            id: "a2".into(),
            kind: crate::ai::tools::ApprovalKind::SaveDraft { draft_id: "d1".into(), replace: Some(pid.clone()) },
        })),
    );
    assert_eq!(app.playlists.get(&pid).unwrap().tracks, vec![files[0].clone(), files[1].clone()], "not replaced before approval");
    // Declining changes nothing and tells the model.
    app.ai_decline("a2");
    assert_eq!(app.playlists.get(&pid).unwrap().tracks.len(), 2);
    assert!(app.ai.history.iter().any(|m| m["content"].as_str().is_some_and(|c| c.contains("declined"))));
    // Approving (the user clicks "replace") replaces it…
    h.run_steps(3);
    click(&mut h, "replace “Drive”");
    let app = h.state_mut();
    assert_eq!(app.playlists.get(&pid).unwrap().tracks, vec![files[1].clone(), files[2].clone()]);
    // …and undo restores the original.
    app.ai_undo_last();
    assert_eq!(app.playlists.get(&pid).unwrap().tracks, vec![files[0].clone(), files[1].clone()]);
}

/// The assistant's Spotify, faked: a fixed library to read, every change recorded.
fn fake_spotify(h: &mut Harness<'static, App>) -> std::sync::Arc<crate::ai::spotify::tests::Fake> {
    let f = std::sync::Arc::new(crate::ai::spotify::tests::fake());
    h.state_mut().ai_source_override = Some(f.clone());
    f
}

fn tool_calls(c: &[(&str, &str, serde_json::Value)]) -> (u16, String) {
    (200, sse("", c))
}

fn sent(f: &crate::ai::spotify::tests::Fake) -> Vec<String> {
    f.sent.lock().unwrap().iter().map(|(m, p, _)| format!("{m} {p}")).collect()
}

#[test]
fn queue_me_new_songs_just_happens() {
    use serde_json::json;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let replies = vec![
        models_reply(),
        tool_calls(&[("c1", "spotify.unheard_songs", json!({ "artist": "Anri", "limit": 2 }))]),
        tool_calls(&[("c2", "spotify.queue_spotify", json!({ "track_ids": ["spotify:track:shyness", "spotify:track:last"], "position": "queue" }))]),
        (200, sse("Queued Shyness Boy and Last Summer Whisper — probably new to you.", &[])),
    ];
    let (mut h, seen, _) = ai_harness(replies, true);
    let fake = fake_spotify(&mut h);
    settle(&mut h, |a| matches!(a.ai.models, Models::Ready(_)));
    let route = h.state().route.clone();

    ask(&mut h, "queue me up a couple of new anri songs i haven't heard");
    settle(&mut h, |a| !a.ai.busy());
    let changes = sent(&fake);
    assert!(changes[0].starts_with("POST /me/player/queue?uri=spotify%3Atrack%3Ashyness") && changes[1].starts_with("POST /me/player/queue?uri=spotify%3Atrack%3Alast"), "{changes:?}");
    assert_eq!(changes.len(), 2, "exactly the two unheard songs, straight away, and nothing else");
    let app = h.state();
    assert_eq!(app.route, route, "nothing moved on screen");
    assert!(app.ai.approvals.is_empty(), "no approval needed for a couple of songs");
    let t = &app.ai.transcript;
    assert!(matches!(&t[t.len() - 2], Line::Note(n) if n == "Queued 2 songs on Spotify."), "Glass's own receipt: {t:?}");
    assert!(matches!(t.last(), Some(Line::Assistant(a)) if a.starts_with("Queued Shyness")));

    let bodies = seen.lock().unwrap().clone();
    assert_eq!(bodies[1]["tools"][0]["name"], "spotify");
    let note = bodies[1]["input"][0]["content"].as_str().unwrap();
    assert!(note.starts_with("[Glass app note]") && note.contains("Golden Hour") && note.contains("spotify:track:t0"), "what's playing goes along: {note}");
    assert!(bodies[3].to_string().contains("Queued 2 songs on Spotify."), "the model hears what really happened");
    assert!(bodies.iter().all(|b| !b.to_string().contains(PLAN_TOKEN)));
}

#[test]
fn asking_from_the_command_bar_stays_out_of_the_way() {
    use serde_json::json;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let replies = vec![models_reply(), tool_calls(&[("c1", "app.player", json!({ "action": "next", "volume_percent": null }))]), (200, sse("Skipped.", &[]))];
    let (mut h, _, _) = ai_harness(replies, true);
    let fake = fake_spotify(&mut h);
    {
        let a = h.state_mut();
        a.settings.right_open = false;
        a.route = Route::NowPlaying;
        a.toasts.clear();
    }
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    h.run_steps(3);
    assert!(h.state().cmd_open, "⌘K works on Now Playing too");
    h.event(egui::Event::Text("? skip this one".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(toasted(&h, "Asking the assistant"));
    settle(&mut h, |a| !a.ai.busy() && a.ai.pending.is_none() && matches!(a.ai.transcript.last(), Some(Line::Assistant(t)) if !t.is_empty()));
    assert!(sent(&fake).iter().any(|c| c.starts_with("POST /me/player/next")), "{:?}", sent(&fake));
    let app = h.state();
    assert!(app.route == Route::NowPlaying && !app.settings.right_open, "Now Playing stays; the panel stays closed");
    assert!(toasted(&h, "Skipped."), "the answer arrives as a toast");
}

#[test]
fn a_spotify_preview_saves_with_one_click_and_undoes() {
    use serde_json::json;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ids = ["spotify:track:remember", "spotify:track:shyness", "spotify:track:windy"];
    let replies = vec![
        models_reply(),
        tool_calls(&[("c1", "spotify_artist", json!({ "artist_id": "anri" }))]),
        tool_calls(&[("c2", "propose_spotify_playlist", json!({ "name": "Anri Deep Cuts", "description": null, "track_ids": ids, "add_to_playlist_id": null }))]),
        (200, sse("Here's a preview of three deeper Anri songs.", &[])),
    ];
    let (mut h, _, _) = ai_harness(replies, true);
    let fake = fake_spotify(&mut h);
    settle(&mut h, |a| matches!(a.ai.models, Models::Ready(_)));
    ask(&mut h, "make me an anri deep cuts playlist");
    settle(&mut h, |a| !a.ai.busy());
    assert!(sent(&fake).is_empty(), "a preview changes nothing on Spotify");
    assert_eq!(h.state().ai.drafts[0].dest, crate::ai::tools::Dest::Spotify);
    h.get_by_label("Remember Summer Days — Anri");

    click(&mut h, "save to Spotify");
    settle(&mut h, |a| matches!(a.ai.drafts[0].status, crate::ai::tools::DraftStatus::Saved(_)));
    let all = fake.sent.lock().unwrap().clone();
    assert_eq!((all[0].0.as_str(), all[0].1.as_str()), ("POST", "/me/playlists"));
    assert_eq!((all[0].2.as_ref().unwrap()["name"].as_str(), all[0].2.as_ref().unwrap()["public"].as_bool()), (Some("Anri Deep Cuts"), Some(false)), "private by default");
    assert_eq!(all[1].1, "/playlists/newpl/tracks");
    assert_eq!(all[1].2.as_ref().unwrap()["uris"], json!(ids));
    let app = h.state();
    assert!(app.ai.transcript.iter().any(|l| matches!(l, Line::Note(n) if n == "Saved “Anri Deep Cuts” to Spotify — 3 songs · 12:43. Undo is available.")), "{:?}", app.ai.transcript);
    assert!(app.ai.history.last().unwrap().to_string().contains("spotify:playlist:newpl"), "the model can refer to it later");

    // A second click makes no second playlist.
    h.state_mut().ai_save_draft("d1", None);
    h.run_steps(5);
    assert_eq!(fake.sent.lock().unwrap().len(), 2);

    h.get_by_label("undo").click();
    h.run_steps(3);
    settle(&mut h, |_| fake.sent.lock().unwrap().len() == 3);
    assert_eq!(sent(&fake)[2], "DELETE /playlists/newpl/followers");
    settle(&mut h, |a| a.ai.drafts[0].status == crate::ai::tools::DraftStatus::Editing);
}

#[test]
fn a_lyrics_pick_sticks_to_the_song() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |_| {});
    h.key_press(Key::L);
    h.run_steps(3);
    settle_lyrics(&mut h);
    // Chosen from "wrong lyrics?".
    let record = std::sync::Arc::new(
        crate::lyrics::parse::record(
            &serde_json::json!({ "id": 99, "trackName": "Golden Hour", "artistName": "Aurora Lane", "duration": 150, "syncedLyrics": "[00:01.00] My own pick\n[00:09.00] Second line" }),
        )
        .unwrap(),
    );
    h.state_mut().act(crate::app::Action::LyricsPick(record));
    h.run_steps(3);
    assert!(matches!(outcome(&h), Outcome::Found { how: crate::lyrics::Match::Manual, .. }));
    // The next time it plays — even with the lyrics cache cleared — the pick is used.
    h.state_mut().act(crate::app::Action::ClearLyricsCache);
    h.state_mut().lyrics.reset();
    h.run_steps(3);
    settle_lyrics(&mut h);
    assert!(matches!(outcome(&h), Outcome::Found { lyrics, how: crate::lyrics::Match::Manual } if lyrics.lines[0].text == "My own pick"));
    h.get_by_label("My own pick");
    // "Look again" goes back to automatic matching.
    h.state_mut().act(crate::app::Action::LyricsRetry);
    h.run_steps(2);
    assert!(h.state().lyrics_track().is_some_and(|t| h.state().lyrics_choices.pick(&t).is_none()));
}

#[test]
fn p_shows_the_playlist_thats_playing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.settings.right_open = false);
    h.key_press(Key::P);
    h.run_steps(4);
    assert_eq!((h.state().route.clone(), h.state().np_view), (Route::NowPlaying, NpView::Playlist), "P opens the playlist view");
    h.get_by_label("playing from");
    h.get_by_label("Late Night Drive");
    // Any song in it plays within the playlist.
    h.get_by_role_and_label(Role::Button, "Bloom").click();
    h.run_steps(3);
    let body = h.state().pending_play.as_ref().map(|p| p.body.clone()).expect("playback requested");
    assert_eq!((body["context_uri"].as_str(), body["offset"]["uri"].as_str()), (Some("spotify:playlist:p0"), Some("spotify:track:t1")));
    h.key_press(Key::P);
    h.run_steps(3);
    assert_ne!(h.state().route, Route::NowPlaying, "P again leaves");
}

#[test]
fn v_toggles_the_visualizer_under_the_lyrics() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |_| {});
    h.key_press(Key::L);
    h.run_steps(3);
    assert!(h.state().settings.visualizer, "on by default");
    h.get_by_label("visualizer");
    settle_lyrics(&mut h);
    h.get_by_label("City lights are slowly fading");
    h.key_press(Key::V);
    h.run_steps(3);
    assert!(!h.state().settings.visualizer, "V hides it");
    h.key_press(Key::V);
    h.run_steps(3);
    assert!(h.state().settings.visualizer, "and shows it again");
}

#[test]
fn lyrics_style_is_edited_in_the_theme_panel() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.settings.right_open = false);
    let file = home().join(".config/glass/theme.toml");
    h.key_press_modifiers(Modifiers::COMMAND, Key::T);
    h.run_steps(3);
    h.get_by_label("lyrics").click();
    // The panel settles at its new height over a few frames.
    h.run_steps(12);
    assert_eq!(h.state().theme_tab, 1);
    let r = h.get_by_label("right").rect();
    click_at(&mut h, r.center());
    assert_eq!(h.state().theme.theme().lyrics.align, "right");
    assert!(std::fs::read_to_string(&file).unwrap().contains("align = \"right\""));
    let r = h.get_by_label("pop").rect();
    click_at(&mut h, r.center());
    assert_eq!(h.state().theme.theme().lyrics.animation, "pop");
    h.run_steps(12);
    let r = h.get_by_role_and_label(Role::Button, "reset").rect();
    click_at(&mut h, r.center());
    assert_eq!(h.state().theme.theme().lyrics.align, "center", "reset brings the defaults back");
    assert!(!std::fs::read_to_string(&file).unwrap().contains("align = \"right\""));
}

#[test]
fn lyrics_translate_below_or_instead_with_the_assistant() {
    use serde_json::json;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let spanish = [
        "Las luces de la ciudad se apagan",
        "Cada esquina tararea una canción",
        "Dejamos que la tarde nos lleve",
        "flotando suave junto a la luna",
        "Cuenta las ventanas, una a una",
        "hasta que la mañana nos encuentre aquí",
    ];
    let answer = json!({ "source_language": "en", "lines": spanish }).to_string();
    let (mut h, seen, _) = ai_harness(vec![models_reply(), (200, sse(&answer, &[]))], true);
    {
        let a = h.state_mut();
        a.route = Route::NowPlaying;
        a.settings.right_open = false;
        a.set_config("lyrics.translate_to", "es".into());
        a.set_config("lyrics.translation", "below".into());
    }
    settle_lyrics(&mut h);
    settle(&mut h, |a| matches!(a.translation.result.as_deref(), Some(crate::lyrics::translate::Translation::Ready { .. })));
    let bodies = seen.lock().unwrap().clone();
    let req = bodies.last().unwrap();
    assert_eq!((req["store"].as_bool(), req["stream"].as_bool()), (Some(false), Some(true)));
    assert!(req["instructions"].as_str().unwrap().contains("Spanish"));
    assert!(req["input"][0]["content"].as_str().unwrap().starts_with("1. City lights are slowly fading"), "the lyrics, numbered");
    assert!(req.get("tools").is_none(), "translation can't call tools");
    // Below: the original lines stay.
    h.run_steps(3);
    h.get_by_label("City lights are slowly fading");
    // T: translated instead.
    h.key_press(Key::T);
    h.run_steps(3);
    assert_eq!(h.state().config.lyrics().translation, crate::lyrics::translate::Mode::Only);
    h.get_by_label("Las luces de la ciudad se apagan");
    assert!(h.query_by_label("City lights are slowly fading").is_none());
    // Saved: a second time costs nothing.
    h.state_mut().translation = Default::default();
    h.run_steps(3);
    assert!(matches!(h.state().translation.result.as_deref(), Some(crate::lyrics::translate::Translation::Ready { .. })));
    assert_eq!(seen.lock().unwrap().len(), bodies.len(), "no second request");
    // T again: back to the original.
    h.key_press(Key::T);
    h.run_steps(3);
    h.get_by_label("City lights are slowly fading");
}

#[test]
fn spotifys_own_translation_is_used_first() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::NowPlaying);
    let mut f = crate::ai::spotify::tests::fake();
    f.data.insert(
        "/glass/lyrics/t0".into(),
        serde_json::json!({ "lyrics": { "syncType": "LINE_SYNCED", "language": "ja", "lines": [{ "startTimeMs": "52000", "words": "真夜中のドア", "endTimeMs": "0" }],
            "alternatives": [{ "language": "es", "lines": ["La puerta de medianoche"] }] } }),
    );
    h.state_mut().ai_source_override = Some(std::sync::Arc::new(f));
    {
        let a = h.state_mut();
        a.set_config("lyrics.provider", "spotify".into());
        a.set_config("lyrics.translate_to", "es".into());
        a.set_config("lyrics.translation", "only".into());
    }
    settle_lyrics(&mut h);
    h.run_steps(4);
    assert!(matches!(h.state().translation.result.as_deref(), Some(crate::lyrics::translate::Translation::Ready { by, .. }) if by == "Spotify"));
    h.get_by_label("La puerta de medianoche");
}

#[test]
fn lyric_lines_animate_only_when_the_line_changes() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 60 frames a second, as on screen; Reduce Motion on (the cross-fade).
    let mut h = harness_dt(vec2(1280.0, 820.0), 1.0 / 60.0, |a| a.route = Route::NowPlaying);
    settle_lyrics(&mut h);
    let key = h.state().lyrics_track().unwrap().key;
    let tween = |h: &Harness<'static, App>| -> (f32, f32, f64, i64) { h.ctx.data(|d| d.get_temp(egui::Id::new(("lyrics-scroll", &key)))).expect("lyrics on screen") };
    h.run_steps(5);
    let before = tween(&h);
    // A layout change (a translation appearing, fonts, a resize) moves the text without an animation.
    h.state_mut().set_config("lyrics.translation", "below".into());
    h.state_mut().translation = crate::translate_app::TranslationState::default();
    h.run_steps(3);
    let after = tween(&h);
    assert_eq!((after.2, after.3), (before.2, before.3), "no new animation for a layout change");
    // The next line: an animation starts (and is over 0.6 s later; it takes about half a second).
    h.state_mut().set_config("lyrics.translation", "off".into());
    let line = before.3;
    let start = h.state().pb.progress_ms;
    h.state_mut().pb.progress_ms = start + 5_200;
    let mut started = None;
    for _ in 0..90 {
        h.run_steps(1);
        let t = tween(&h);
        if t.3 != line {
            started.get_or_insert(t.2);
        }
    }
    let t = tween(&h);
    assert_eq!(t.3, line + 1, "the next line became current");
    assert!(started.is_some(), "and it animated");
    let now = h.ctx.input(|i| i.time);
    assert!(now - t.2 > 0.6, "and has had time to finish");
}

#[test]
fn share_downloads_a_song() {
    use crate::save_song::{Format, Status};
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::Playlist("p0".into()));
    h.state_mut().toasts.clear();
    let share = |h: &mut Harness<'static, App>| {
        h.get_by_role_and_label(Role::Button, "Bloom").click_secondary();
        h.run_steps(3);
        h.get_by_label("Share ⏵").click();
        h.run_steps(3);
    };

    share(&mut h);
    h.get_by_label("Download as M4A").click();
    h.run_steps(3);
    assert!(toasted(&h, "Saving “Bloom” as M4A"), "the save is announced");
    let job = h.state().saving.get("spotify:track:t1").cloned().expect("a save started");
    assert_eq!(job.status, Status::Queued);
    assert!(job.path.ends_with("Music/Glass/The Paper Kites - Bloom.m4a"), "saved as <artist> - <title>: {}", job.path.display());

    share(&mut h);
    assert!(h.query_by_label("Finding it to download as M4A…").is_some(), "the menu shows it's on its way");
    h.key_press(Key::Escape);
    h.run_steps(2);

    // The download reports back through Msg::SaveSong.
    let send = |h: &mut Harness<'static, App>, s: Status| {
        let _ = h.state().tx.send(crate::app::Msg::SaveSong("spotify:track:t1".into(), s));
        h.run_steps(3);
    };
    send(&mut h, Status::Downloading(42));
    share(&mut h);
    assert!(h.query_by_label("Downloading as M4A… 42%").is_some());
    h.key_press(Key::Escape);
    h.run_steps(2);
    std::fs::create_dir_all(job.path.parent().unwrap()).unwrap();
    std::fs::write(&job.path, b"m4a").unwrap();
    send(&mut h, Status::Done);
    assert!(toasted(&h, "Saved “Bloom” as The Paper Kites - Bloom.m4a"));
    share(&mut h);
    assert!(h.query_by_label_contains("Downloaded ·").is_some(), "the menu offers to show the file");
    h.key_press(Key::Escape);
    h.run_steps(2);

    // The label follows the format in Settings.
    h.state_mut().settings.save_format = Format::Mp3;
    share(&mut h);
    assert!(h.query_by_label("Download as MP3").is_some());
}

#[test]
fn share_downloads_a_whole_playlist() {
    use crate::save_song::Status;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::Playlist("p0".into()));
    h.state_mut().toasts.clear();
    let share = |h: &mut Harness<'static, App>| {
        h.query_all_by_role_and_label(Role::Button, "More options").next().expect("the playlist's menu").click();
        h.run_steps(3);
        h.get_by_label("Share ⏵").click();
        h.run_steps(3);
    };
    let close = |h: &mut Harness<'static, App>| {
        h.key_press(Key::Escape);
        h.run_steps(2);
        h.key_press(Key::Escape);
        h.run_steps(2);
    };

    share(&mut h);
    h.get_by_label("Download playlist as M4A").click();
    h.run_steps(3);
    let set = &h.state().saving_sets["spotify:playlist:p0"];
    let (keys, folder) = (set.keys.clone(), set.folder.clone());
    let n = keys.len();
    assert!(n >= 10, "every song of the playlist: {n}");
    assert!(folder.ends_with("Music/Glass/Late Night Drive"), "a folder named after it: {}", folder.display());
    assert!(toasted(&h, &format!("Saving {n} songs from “Late Night Drive” as M4A")));
    assert!(keys.iter().all(|k| h.state().saving[k].status == Status::Queued));

    // Songs report back one by one; the menu counts them.
    let finish = |h: &mut Harness<'static, App>, key: &str, status: Status| {
        if status == Status::Done {
            let path = h.state().saving[key].path.clone();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"m4a").unwrap();
        }
        let _ = h.state().tx.send(crate::app::Msg::SaveSong(key.to_string(), status));
        h.run_steps(2);
    };
    h.state_mut().toasts.clear();
    finish(&mut h, &keys[0], Status::Done);
    assert!(h.state().toasts.is_empty(), "no toast per song");
    share(&mut h);
    assert!(h.query_by_label(&format!("Downloading playlist as M4A… 1 of {n}")).is_some());
    assert!(h.query_by_label("Stop downloading").is_some());
    close(&mut h);

    // One summary when the last one settles, and a playlist file in its order.
    for key in &keys[1..n - 1] {
        finish(&mut h, key, Status::Done);
    }
    finish(&mut h, &keys[n - 1], Status::Failed("couldn't find it".into()));
    assert!(toasted(&h, &format!("Saved {} of {n} songs from “Late Night Drive” · 1 failed", n - 1)));
    let m3u = std::fs::read_to_string(folder.join("Late Night Drive.m3u8")).expect("a playlist file");
    assert_eq!(m3u.lines().count(), n, "a header and the {} saved songs", n - 1);
    assert_eq!(m3u.lines().nth(1).unwrap(), h.state().saving[&keys[0]].path.file_name().unwrap().to_string_lossy());

    // Afterwards it can be shown, or run again for what's missing.
    share(&mut h);
    assert!(h.query_by_label_contains("Downloaded ·").is_some());
    h.get_by_label("Download playlist as M4A").click();
    h.run_steps(3);
    assert_eq!(h.state().saving[&keys[n - 1]].status, Status::Queued, "the missing song is tried again");
    assert_eq!(h.state().saving[&keys[0]].status, Status::Done, "saved songs stay saved");
}

#[test]
fn albums_and_artists_download_from_share_too() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::Artist("a0".into()));
    h.state_mut().toasts.clear();
    h.query_all_by_role_and_label(Role::Button, "More options").next().expect("the artist's menu").click();
    h.run_steps(3);
    h.get_by_label("Share ⏵").click();
    h.run_steps(3);
    h.get_by_label("Download top songs as M4A").click();
    h.run_steps(3);
    let set = &h.state().saving_sets["spotify:artist:a0"];
    assert_eq!(set.keys.len(), 10, "the artist's top 10");
    assert!(toasted(&h, "Saving 10 songs from Aurora Lane's top songs as M4A"));

    // An album's tracks come without the album; the save gives them this one.
    let album = serde_json::json!({ "id": "alx", "name": "Quiet Hours", "type": "album", "uri": "spotify:album:alx",
        "artists": [{ "id": "a0", "name": "Aurora Lane", "uri": "spotify:artist:a0" }], "release_date": "2015-01-10", "images": [], "tracks": { "items": [], "total": 3 } });
    let tracks: Vec<serde_json::Value> = ["Dawn", "Noon", "Dusk"]
        .iter()
        .enumerate()
        .map(|(i, n)| serde_json::json!({ "id": format!("x{i}"), "name": n, "type": "track", "uri": format!("spotify:track:x{i}"), "duration_ms": 200_000, "track_number": i + 1, "artists": [{ "id": "a0", "name": "Aurora Lane" }] }))
        .collect();
    let ready = |v| crate::app::Load::Ready(std::sync::Arc::new(v));
    h.state_mut().data.insert("/albums/alx".into(), ready(album));
    h.state_mut().data.insert("ALL2000:/albums/alx/tracks?limit=50".into(), ready(serde_json::json!({ "items": tracks })));
    h.state_mut().act(crate::app::Action::Navigate(Route::Album("alx".into())));
    h.run_steps(4);
    h.query_all_by_role_and_label(Role::Button, "More options").next().expect("the album's menu").click();
    h.run_steps(3);
    h.get_by_label("Share ⏵").click();
    h.run_steps(3);
    h.get_by_label("Download album as M4A").click();
    h.run_steps(3);
    let set = &h.state().saving_sets["spotify:album:alx"];
    assert_eq!(set.keys.len(), 3);
    assert!(set.folder.ends_with("Music/Glass/Aurora Lane - Quiet Hours"), "{}", set.folder.display());
    assert!(toasted(&h, "Saving 3 songs from “Aurora Lane - Quiet Hours” as M4A"));
}

#[test]
fn now_playing_takes_the_album_colors() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = harness(vec2(1280.0, 820.0), |a| a.route = Route::NowPlaying);
    // The harness draws with its own context, which doesn't have the demo's covers: give
    // it this song's (pink to indigo, like the demo's first), and start over.
    let uri = h.state_mut().now_cover(64).expect("the playing song's cover");
    let img = image::RgbImage::from_fn(64, 64, |x, y| {
        let t = (x + y) as f32 / 126.0;
        let mix = |p: f32, q: f32| (p * (1.0 - t) + q * t) as u8;
        image::Rgb([mix(236.0, 99.0), mix(72.0, 102.0), mix(153.0, 241.0)])
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    h.ctx.include_bytes(uri, png);
    h.state_mut().album_colors = Default::default();
    // The cover is sampled off the UI thread.
    for _ in 0..100 {
        if h.state().album_colors.shown().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    let p = h.state().album_colors.shown().expect("the playing cover's colors");
    // The demo cover runs from pink to indigo.
    let [r, g, b] = p.accent.to_rgb();
    assert!(r > g + 30 && b > g + 30, "a pink-violet accent: {:?}", [r, g, b]);
    assert!(h.state().settings.album_colors, "on unless turned off");

    // At strength 0 the palette changes nothing.
    let mut theme = h.state().theme.theme().clone();
    let before = theme.accent;
    p.apply(&mut theme, 0.0);
    assert_eq!(theme.accent, before);
}

/// Every page and panel at the smallest window, zoomed in up to 2× (⌘+): egui's
/// debug checks catch a widget asked to be a negative size, which in a release
/// build quietly lays out wrong (a title squeezed to nothing, labels a letter wide).
#[test]
fn every_page_lays_out_at_any_zoom() {
    use crate::app::NpView;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // (route, right panel, now-playing view, theme panel)
    let mut cases: Vec<(Route, Option<RightPanel>, Option<NpView>, bool)> = [
        Route::Library,
        Route::NowPlaying,
        Route::Home,
        Route::Search,
        Route::Playlist("p0".into()),
        Route::Album("al0".into()),
        Route::Artist("a0".into()),
        Route::Discography("a0".into()),
        Route::Track("t0".into()),
        Route::Profile("me".into()),
        Route::LikedSongs,
        Route::YourEpisodes,
        Route::LocalFiles,
        Route::Recents,
        Route::Settings,
        Route::Activity,
    ]
    .into_iter()
    .map(|r| (r, None, None, false))
    .collect();
    for p in [RightPanel::Queue, RightPanel::Devices, RightPanel::Lyrics, RightPanel::Assistant] {
        cases.push((Route::Home, Some(p), None, false));
        cases.push((Route::Playlist("p0".into()), Some(p), None, false));
    }
    for v in [NpView::Lyrics, NpView::Queue, NpView::Devices, NpView::Playlist] {
        cases.push((Route::NowPlaying, None, Some(v), false));
    }
    cases.push((Route::Home, None, None, true));
    let mut bad = Vec::new();
    for zoom in [1.0f32, 1.25, 1.5, 2.0] {
        for case in &cases {
            let (route, right, np, panel) = case.clone();
            // A thread each: the theme remembers per thread which fonts it installed.
            let r = std::thread::spawn(move || {
                let mut h = harness(vec2(900.0, 600.0), |a| {
                    if let Some(p) = right {
                        a.settings.right_open = true;
                        a.right = p;
                    }
                    if let Some(v) = np {
                        a.np_view = v;
                    }
                    a.theme_panel = panel;
                });
                h.ctx.set_zoom_factor(zoom);
                h.run_steps(2);
                h.state_mut().route = route;
                h.run_steps(4);
            })
            .join();
            if r.is_err() {
                bad.push(format!("zoom {zoom} {case:?}"));
            }
        }
    }
    assert!(bad.is_empty(), "these panicked (see above): {bad:#?}");
}
