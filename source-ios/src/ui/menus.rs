//! Right-click menus, mirroring the actions in Spotify's context menus.

use super::widgets::{menu_item, menu_note};
use crate::app::{Action, App, Modal, Route};
use crate::model::{self, id_of, s, web_link};
use eframe::egui::Ui;
use serde_json::Value;

/// Playlists the user can add songs to.
pub fn writable_playlists(app: &mut App) -> Vec<Value> {
    let me = app.user_id();
    app.want_all("/me/playlists?limit=50", 2000)
        .map(|v| model::items(&v).iter().filter(|p| p["owner"]["id"].as_str() == Some(me.as_str()) || model::b(p, "collaborative")).cloned().collect())
        .unwrap_or_default()
}

fn add_to_playlist_menu(app: &mut App, ui: &mut Ui, uris: Vec<String>, label: &str) {
    ui.menu_button(label, |ui| {
        ui.set_min_width(220.0);
        if menu_item(ui, "＋  New playlist") {
            app.act(Action::CreatePlaylist { name: "New Playlist".into(), uris: uris.clone() });
        }
        ui.separator();
        eframe::egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for p in writable_playlists(app) {
                if menu_item(ui, s(&p, "name")) {
                    app.act(Action::AddToPlaylist { playlist: s(&p, "id").into(), uris: uris.clone() });
                }
            }
        });
    });
}

/// "Add to playlist" for files on this computer. Spotify playlists take them
/// as `spotify:local:` entries, as in Spotify's own apps (every computer plays
/// its own copy of the file); Glass's local playlists keep the files' paths.
pub fn add_local_to_playlist(app: &mut App, ui: &mut Ui, tracks: &[crate::local::LocalTrack]) {
    let uris: Vec<String> = tracks.iter().map(crate::local::spotify_uri).collect();
    let paths: Vec<std::path::PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
    let label = if tracks.len() > 1 { format!("Add {} songs to playlist", tracks.len()) } else { "Add to playlist".to_string() };
    ui.menu_button(label, |ui| {
        ui.set_min_width(220.0);
        let signed_in = app.api.is_some();
        if signed_in && menu_item(ui, "＋  New playlist") {
            app.act(Action::CreatePlaylist { name: "New Playlist".into(), uris: uris.clone() });
        }
        eframe::egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            if signed_in {
                ui.separator();
                for p in writable_playlists(app) {
                    if menu_item(ui, s(&p, "name")) {
                        app.act(Action::AddToPlaylist { playlist: s(&p, "id").into(), uris: uris.clone() });
                    }
                }
                ui.separator();
            }
            super::widgets::caps_label(ui, "only on this computer", 10.5, super::theme::col::muted());
            if menu_item(ui, "＋  New local playlist") {
                let name = format!("Local Playlist #{}", app.playlists.list.len() + 1);
                app.act(Action::CreateLocalPlaylist { name, paths: paths.clone() });
            }
            for p in app.playlists.list.clone() {
                if menu_item(ui, &p.name) {
                    app.act(Action::AddToLocalPlaylist { id: p.id.clone(), paths: paths.clone() });
                }
            }
        });
    });
}

/// "Send to ▸ <computer>": local songs to Glass on another of your computers on
/// this network. Nothing when there are none.
pub fn send_to_device(app: &mut App, ui: &mut Ui, tracks: &[crate::local::LocalTrack]) {
    if app.share_peers.is_empty() || tracks.is_empty() {
        return;
    }
    let paths: Vec<std::path::PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
    let label = if tracks.len() > 1 { format!("Send {} songs to", tracks.len()) } else { "Send to".to_string() };
    ui.menu_button(label, |ui| {
        for p in app.share_peers.clone() {
            if menu_item(ui, &p.name) {
                app.act(Action::SendToDevice { peer: p.id.clone(), paths: paths.clone() });
            }
        }
    });
}

/// A playlist's local songs that aren't on this computer.
pub fn missing_local(app: &mut App, uris: &[String]) -> Vec<String> {
    uris.iter().filter(|u| u.starts_with("spotify:local:") && matches!(app.local_link(u), Some(None))).cloned().collect()
}

/// "Add local files…": pick songs (or a folder's worth) for a playlist.
pub fn pick_local_files(app: &mut App, target: crate::app::FileTarget) {
    if let Some(paths) = rfd::FileDialog::new().set_title("Add local files").add_filter("Audio", crate::local::EXTENSIONS).pick_files() {
        app.act(Action::AddFiles { paths, target });
    }
}

/// What Share ▸ "Download as …" can save.
enum Saveable<'a> {
    /// A Web API track.
    Song(&'a Value),
    /// A playlist, album or artist (its Web API object).
    Set(crate::save_song::SetKind, &'a Value),
}

fn share_menu(app: &mut App, ui: &mut Ui, uri: &str, noun: &str, save: Option<Saveable>) {
    // A local file has no link and can't be saved again: nothing to share.
    if web_link(uri).is_none() && save.is_none() {
        return;
    }
    ui.menu_button("Share", |ui| {
        if let Some(link) = web_link(uri) {
            if menu_item(ui, &format!("Copy link to {noun}")) {
                app.act(Action::Copy(link.clone()));
            }
            if menu_item(ui, "Copy Spotify URI") {
                app.act(Action::Copy(uri.to_string()));
            }
            if menu_item(ui, "Open in browser") {
                let _ = open::that(link);
            }
        }
        match save {
            Some(Saveable::Song(item)) => {
                ui.separator();
                save_song_item(app, ui, item);
            }
            Some(Saveable::Set(kind, value)) => {
                ui.separator();
                save_set_item(app, ui, uri, kind, value);
            }
            None => {}
        }
    });
}

/// "Download as M4A" (in the format from Settings ▸ Save songs), or how that save is going.
fn save_song_item(app: &mut App, ui: &mut Ui, item: &Value) {
    use crate::save_song::Status;
    let format = app.settings.save_format;
    let job = app.saving.get(s(item, "uri")).cloned();
    let save = |app: &mut App| app.act(Action::SaveSong(Box::new(item.clone())));
    match job.map(|j| (j.status, j.path)) {
        Some((Status::Queued | Status::Finding, _)) => menu_note(ui, &format!("Finding it to download as {}…", format.name())),
        Some((Status::Downloading(p), _)) => menu_note(ui, &format!("Downloading as {}… {p}%", format.name())),
        Some((Status::Converting, _)) => menu_note(ui, &format!("Saving as {}…", format.name())),
        Some((Status::Done, path)) if path.exists() && path.extension().is_some_and(|e| e == format.extension()) => {
            if menu_item(ui, &format!("Downloaded · {}", crate::local::REVEAL)) {
                crate::local::reveal(&path);
            }
        }
        Some((Status::Failed(_), _)) => {
            if menu_item(ui, &format!("Download as {} (try again)", format.name())) {
                save(app);
            }
        }
        _ => {
            if menu_item(ui, &format!("Download as {}", format.name())) {
                save(app);
            }
        }
    }
}

/// "Download playlist as M4A", how it's going, or where it went.
fn save_set_item(app: &mut App, ui: &mut Ui, uri: &str, kind: crate::save_song::SetKind, value: &Value) {
    let (format, noun) = (app.settings.save_format.name(), kind.noun());
    if let Some((saved, failed, stopped, going)) = app.set_progress(uri) {
        if going > 0 {
            let done = saved + failed + stopped;
            menu_note(ui, &format!("Downloading {noun} as {format}… {done} of {}", done + going));
            if menu_item(ui, "Stop downloading") {
                app.act(Action::StopSaveSet(uri.into()));
            }
            return;
        }
        if let Some(folder) = app.saving_sets.get(uri).map(|set| set.folder.clone()).filter(|f| f.exists())
            && menu_item(ui, &format!("Downloaded · {}", crate::local::REVEAL))
        {
            crate::local::reveal(&folder);
        }
    }
    // Again later: songs already saved are skipped, so this gets missing and new ones.
    if menu_item(ui, &format!("Download {noun} as {format}")) {
        let name = match kind {
            crate::save_song::SetKind::Album => match model::arr(value, "artists").first() {
                Some(artist) => format!("{} - {}", s(artist, "name"), s(value, "name")),
                None => s(value, "name").to_string(),
            },
            _ => s(value, "name").to_string(),
        };
        match set_tracks(app, kind, value) {
            Some(items) => app.act(Action::SaveSet { uri: uri.into(), name, kind, items }),
            None => app.act(Action::Toast(format!("Still loading this {} — try again in a moment", if kind == crate::save_song::SetKind::Artist { "artist" } else { noun }))),
        }
    }
}

/// A collection's tracks (Web API track objects, in order), once they've loaded.
fn set_tracks(app: &mut App, kind: crate::save_song::SetKind, value: &Value) -> Option<Vec<Value>> {
    use crate::save_song::SetKind;
    let id = s(value, "id");
    match kind {
        SetKind::Playlist => app.want_all(&format!("/playlists/{id}/tracks?limit=100"), 10_000).map(|v| model::items(&v).iter().map(|i| model::playlist_entry(i).clone()).collect()),
        SetKind::Album => {
            // An album's tracks come without the album; give each this one (minus its track list).
            let tracks = app.want_all(&format!("/albums/{id}/tracks?limit=50"), 2000)?;
            let mut album = value.clone();
            if let Some(fields) = album.as_object_mut() {
                fields.remove("tracks");
            }
            Some(
                model::items(&tracks)
                    .iter()
                    .map(|t| {
                        let mut t = t.clone();
                        t["album"] = album.clone();
                        t
                    })
                    .collect(),
            )
        }
        SetKind::Artist => app.want(&format!("/artists/{id}/top-tracks?market=from_token")).map(|v| model::arr(&v, "tracks").to_vec()),
    }
}

fn pin_item(app: &mut App, ui: &mut Ui, uri: &str, noun: &str) {
    let pinned = app.settings.pins.iter().any(|p| p == uri);
    if menu_item(ui, &format!("{} {noun}", if pinned { "Unpin" } else { "Pin" })) {
        app.act(if pinned { Action::Unpin(uri.into()) } else { Action::Pin(uri.into()) });
    }
}

/// "Download" / "Remove download" for a playlist or album.
pub fn download_item(app: &mut App, ui: &mut Ui, uri: &str, name: &str) {
    let kept = app.downloads.is_kept(uri);
    let label = match app.downloads.progress(uri) {
        Some((done, total)) if done < total => format!("Remove download ({done}/{total} done)"),
        Some(_) => "Remove download".to_string(),
        None => "Download".to_string(),
    };
    if menu_item(ui, &label) {
        app.act(if kept { Action::RemoveDownload(uri.into()) } else { Action::Download { uri: uri.into(), name: name.into() } });
    }
}

/// Track or episode. `playlist` = (id, can_edit). `selection` = the selected URIs; when it
/// holds this item and others, the menu acts on all of them.
pub fn track(app: &mut App, ui: &mut Ui, item: &Value, playlist: Option<(&str, bool)>, selection: &[String]) {
    let uri = s(item, "uri").to_string();
    let is_episode = s(item, "type") == "episode";
    let many = selection.len() > 1 && selection.contains(&uri);
    let uris: Vec<String> = if many { selection.to_vec() } else { vec![uri.clone()] };

    add_to_playlist_menu(app, ui, uris.clone(), "Add to playlist");
    if let Some((pid, true)) = playlist
        && menu_item(ui, "Remove from this playlist")
    {
        app.act(Action::RemoveFromPlaylist { playlist: pid.into(), uris: uris.clone() });
    }
    let saved = app.is_saved(&uri).unwrap_or(false);
    let save_label = if is_episode {
        if saved { "Remove from Your Episodes" } else { "Save to Your Episodes" }
    } else if saved {
        "Remove from Liked Songs"
    } else {
        "Save to Liked Songs"
    };
    if menu_item(ui, save_label) {
        app.act(Action::SaveAll { uris: uris.clone(), save: !saved });
    }
    if menu_item(ui, "Add to queue") {
        app.act(Action::Queue(uris.clone()));
    }
    ui.separator();
    if !is_episode {
        // A playlist's local songs have no Spotify song, artist or album to go to.
        let spotify = !uri.starts_with("spotify:local:");
        if spotify && !id_of(&uri).is_empty() && menu_item(ui, "Go to song") {
            app.act(Action::Navigate(Route::Track(id_of(&uri).into())));
        }
        if spotify && menu_item(ui, "Go to song radio") {
            app.act(Action::Radio(uri.clone()));
        }
        let artists: Vec<&Value> = model::arr(item, "artists").iter().filter(|a| !s(a, "id").is_empty()).collect();
        if artists.len() > 1 {
            ui.menu_button("Go to artist", |ui| {
                for a in artists {
                    if menu_item(ui, s(a, "name")) {
                        app.act(Action::Navigate(Route::Artist(s(a, "id").into())));
                    }
                }
            });
        } else if let Some(a) = artists.first()
            && menu_item(ui, "Go to artist")
        {
            app.act(Action::Navigate(Route::Artist(s(a, "id").into())));
        }
        if let Some(album) = item.get("album").filter(|a| !s(a, "id").is_empty())
            && menu_item(ui, "Go to album")
        {
            app.act(Action::Navigate(Route::Album(s(album, "id").into())));
        }
    } else {
        if menu_item(ui, "Go to episode") {
            app.act(Action::Navigate(Route::Episode(id_of(&uri).into())));
        }
        if let Some(show) = item.get("show")
            && menu_item(ui, "Go to podcast")
        {
            app.act(Action::Navigate(Route::Show(s(show, "id").into())));
        }
    }
    if model::b(item, "is_local") && !app.share_peers.is_empty() {
        let missing = missing_local(app, &uris);
        if !missing.is_empty() && menu_item(ui, "Get from your other computers") {
            app.act(Action::GetFromDevices(missing));
        }
    }
    if model::b(item, "is_local")
        && let Some(Some(t)) = app.local_link(&uri)
    {
        ui.separator();
        send_to_device(app, ui, std::slice::from_ref(&t));
        if menu_item(ui, "Edit song info…") {
            app.act(Action::EditTags(t.path.clone()));
        }
        if menu_item(ui, crate::local::REVEAL) {
            crate::local::reveal(&t.path);
        }
    }
    ui.separator();
    let song = (!is_episode && !model::b(item, "is_local")).then_some(Saveable::Song(item));
    share_menu(app, ui, &uri, if is_episode { "episode" } else { "song" }, song);
}

pub fn playlist(app: &mut App, ui: &mut Ui, p: &Value) {
    let uri = s(p, "uri").to_string();
    let id = s(p, "id").to_string();
    let me = app.user_id();
    let own = p["owner"]["id"].as_str() == Some(me.as_str());
    if menu_item(ui, "Add to queue") {
        app.act(Action::QueueCollection(uri.clone()));
    }
    // Only when another Glass is around to ask: the playlist's songs are needed to know what's missing.
    if !app.share_peers.is_empty()
        && let Some(v) = app.want_all(&format!("/playlists/{id}/tracks?limit=100"), 10_000)
    {
        let u: Vec<String> = model::items(&v).iter().map(|i| s(model::playlist_entry(i), "uri").to_string()).filter(|u| !u.is_empty()).collect();
        let missing = missing_local(app, &u);
        if !missing.is_empty() && menu_item(ui, &format!("Get {} missing local {} from your other computers", missing.len(), if missing.len() == 1 { "song" } else { "songs" })) {
            app.act(Action::GetFromDevices(missing));
        }
    }
    if (own || model::b(p, "collaborative")) && menu_item(ui, "Add local files…") {
        pick_local_files(app, crate::app::FileTarget::Spotify { id: id.clone(), name: s(p, "name").to_string() });
    }
    if own {
        if menu_item(ui, "Edit details") {
            app.modal = Some(Modal::EditPlaylist {
                id: id.clone(),
                name: s(p, "name").into(),
                desc: model::plain(s(p, "description")),
                public: model::b(p, "public"),
                collaborative: model::b(p, "collaborative"),
            });
        }
        if menu_item(ui, "Delete") {
            app.modal = Some(Modal::Confirm {
                title: "Delete from Your Library?".into(),
                body: format!("This will delete {} from Your Library.", s(p, "name")),
                confirm: "Delete".into(),
                action: Box::new(Action::DeletePlaylist(id.clone())),
            });
        }
    } else {
        let saved = app.is_saved(&uri).unwrap_or(false);
        if menu_item(ui, if saved { "Remove from Your Library" } else { "Save to Your Library" }) {
            app.act(Action::Save { uri: uri.clone(), save: !saved });
        }
    }
    download_item(app, ui, &uri, s(p, "name"));
    ui.separator();
    if menu_item(ui, "Create playlist") {
        app.act(Action::CreatePlaylist { name: "New Playlist".into(), uris: vec![] });
    }
    if menu_item(ui, "Create folder") {
        app.modal = Some(Modal::NewFolder { name: "New Folder".into(), add: Some(uri.clone()) });
    }
    if own {
        if model::b(p, "collaborative") {
            if menu_item(ui, "Make non-collaborative") {
                app.act(Action::UpdatePlaylist { id: id.clone(), body: serde_json::json!({ "collaborative": false }) });
            }
        } else if menu_item(ui, "Invite collaborators") {
            app.act(Action::UpdatePlaylist { id: id.clone(), body: serde_json::json!({ "collaborative": true, "public": false }) });
            app.act(Action::Copy(web_link(&uri).unwrap_or_default()));
            app.act(Action::Toast("Playlist is now collaborative — link copied".into()));
        }
        let public = model::b(p, "public");
        if menu_item(ui, if public { "Remove from profile" } else { "Add to profile" }) {
            app.act(Action::UpdatePlaylist { id: id.clone(), body: serde_json::json!({ "public": !public }) });
        }
    }
    let folders: Vec<(String, String)> = app.settings.folders.iter().map(|f| (f.id.clone(), f.name.clone())).collect();
    ui.menu_button("Move to folder", |ui| {
        if menu_item(ui, "＋  Create folder") {
            app.modal = Some(Modal::NewFolder { name: "New Folder".into(), add: Some(uri.clone()) });
        }
        for (fid, name) in &folders {
            if menu_item(ui, name) {
                app.act(Action::MoveToFolder { playlist: uri.clone(), folder: Some(fid.clone()) });
            }
        }
        if menu_item(ui, "Remove from folders") {
            app.act(Action::MoveToFolder { playlist: uri.clone(), folder: None });
        }
    });
    pin_item(app, ui, &uri, "playlist");
    ui.separator();
    share_menu(app, ui, &uri, "playlist", Some(Saveable::Set(crate::save_song::SetKind::Playlist, p)));
}

pub fn album(app: &mut App, ui: &mut Ui, a: &Value) {
    let uri = s(a, "uri").to_string();
    let saved = app.is_saved(&uri).unwrap_or(false);
    if menu_item(ui, if saved { "Remove from Your Library" } else { "Save to Your Library" }) {
        app.act(Action::Save { uri: uri.clone(), save: !saved });
    }
    if menu_item(ui, "Add to queue") {
        app.act(Action::QueueCollection(uri.clone()));
    }
    if menu_item(ui, "Go to artist radio")
        && let Some(ar) = model::arr(a, "artists").first()
    {
        app.act(Action::Radio(s(ar, "uri").into()));
    }
    if let Some(ar) = model::arr(a, "artists").first()
        && menu_item(ui, "Go to artist")
    {
        app.act(Action::Navigate(Route::Artist(s(ar, "id").into())));
    }
    let tracks: Vec<String> = a.get("tracks").map(|t| model::items(t).iter().map(|t| s(t, "uri").to_string()).collect()).unwrap_or_default();
    if !tracks.is_empty() {
        add_to_playlist_menu(app, ui, tracks, "Add to playlist");
    }
    pin_item(app, ui, &uri, "album");
    download_item(app, ui, &uri, s(a, "name"));
    ui.separator();
    share_menu(app, ui, &uri, "album", Some(Saveable::Set(crate::save_song::SetKind::Album, a)));
}

pub fn artist(app: &mut App, ui: &mut Ui, a: &Value) {
    let uri = s(a, "uri").to_string();
    let following = app.is_saved(&uri).unwrap_or(false);
    if menu_item(ui, if following { "Unfollow" } else { "Follow" }) {
        app.act(Action::Save { uri: uri.clone(), save: !following });
    }
    if menu_item(ui, "Go to artist radio") {
        app.act(Action::Radio(uri.clone()));
    }
    pin_item(app, ui, &uri, "artist");
    ui.separator();
    share_menu(app, ui, &uri, "artist", Some(Saveable::Set(crate::save_song::SetKind::Artist, a)));
}

pub fn show(app: &mut App, ui: &mut Ui, sh: &Value) {
    let uri = s(sh, "uri").to_string();
    let saved = app.is_saved(&uri).unwrap_or(false);
    if menu_item(ui, if saved { "Unfollow" } else { "Follow" }) {
        app.act(Action::Save { uri: uri.clone(), save: !saved });
    }
    pin_item(app, ui, &uri, "podcast");
    ui.separator();
    share_menu(app, ui, &uri, "podcast", None);
}

pub fn audiobook(app: &mut App, ui: &mut Ui, b: &Value) {
    let uri = s(b, "uri").to_string();
    let saved = app.is_saved(&uri).unwrap_or(false);
    if menu_item(ui, if saved { "Remove from Your Library" } else { "Save to Your Library" }) {
        app.act(Action::Save { uri: uri.clone(), save: !saved });
    }
    pin_item(app, ui, &uri, "audiobook");
    ui.separator();
    share_menu(app, ui, &uri, "audiobook", None);
}

pub fn user(app: &mut App, ui: &mut Ui, u: &Value) {
    let uri = s(u, "uri").to_string();
    // Following people isn't something Glass can do (Spotify's services for it
    // aren't known), so a profile offers no Follow.
    if s(u, "id") == app.user_id() && menu_item(ui, "Edit profile") {
        let _ = open::that("https://www.spotify.com/account/profile/");
    }
    share_menu(app, ui, &uri, "profile", None);
}

/// Dispatch by `type`.
pub fn any(app: &mut App, ui: &mut Ui, v: &Value) {
    match s(v, "type") {
        "track" | "episode" | "chapter" => track(app, ui, v, None, &[]),
        "playlist" => playlist(app, ui, v),
        "album" => album(app, ui, v),
        "artist" => artist(app, ui, v),
        "show" => show(app, ui, v),
        "audiobook" => audiobook(app, ui, v),
        "user" => user(app, ui, v),
        _ => {}
    }
}

/// Where clicking an item should go.
pub fn route_for(v: &Value) -> Option<Route> {
    let id = s(v, "id").to_string();
    Some(match s(v, "type") {
        "playlist" => Route::Playlist(id),
        "album" => Route::Album(id),
        "artist" => Route::Artist(id),
        "show" => Route::Show(id),
        "episode" => Route::Episode(id),
        "audiobook" => Route::Audiobook(id),
        "track" => Route::Track(id),
        "user" => Route::Profile(id),
        _ => return None,
    })
}

pub fn subtitle_for(v: &Value) -> String {
    match s(v, "type") {
        "playlist" => format!("Playlist • {}", s(&v["owner"], "display_name")),
        "album" => format!("{} • {}", model::year(s(v, "release_date")), model::artists_line(v)),
        "artist" => "Artist".into(),
        "show" => format!("Podcast • {}", s(v, "publisher")),
        "episode" => format!("{} • {}", model::fmt_date(s(v, "release_date")), model::fmt_long(model::u(v, "duration_ms"))),
        "audiobook" => format!("Audiobook • {}", model::artists_line(v)),
        "track" => format!("Song • {}", model::artists_line(v)),
        "user" => "Profile".into(),
        _ => String::new(),
    }
}
