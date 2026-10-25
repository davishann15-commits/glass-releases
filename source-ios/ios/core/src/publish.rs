//! After each batch of messages the actor builds what Swift shows and sends only
//! what changed: one event per part (account, playback, queue, devices, library,
//! each page on screen). Swift keeps the last of each in its one store.

use crate::app::{AuthState, MobileApp};
use crate::bridge::CoreEvent;
use crate::model::{self, s};
use crate::types::*;
use crate::views::{artist_links, link_of};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct Published {
    account: Option<AccountModel>,
    playback: Option<PlaybackModel>,
    queue: Option<QueueModel>,
    devices: Option<DevicesModel>,
    library: Option<LibraryModel>,
    pub theme: Option<crate::theme_app::ThemeModel>,
    pub settings: Option<crate::settings_app::SettingsModel>,
    pub config: Option<crate::settings_app::ConfigModel>,
    pub status: Option<crate::settings_app::StatusModel>,
    pub pages: HashMap<String, PageModel>,
}

fn unix_ms(at: Option<Instant>) -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    match at {
        Some(t) => now.saturating_sub(t.elapsed().as_millis() as u64),
        None => now,
    }
}

impl MobileApp {
    pub fn publish(&mut self) {
        let mut events = std::mem::take(&mut self.outbox);
        let account = self.account_model();
        if self.published.account.as_ref() != Some(&account) {
            self.published.account = Some(account.clone());
            events.push(CoreEvent::Account { account });
        }
        self.publish_theme(&mut events);
        self.publish_settings(&mut events);
        self.publish_local(&mut events);
        self.publish_saves(&mut events);
        self.publish_share(&mut events);
        self.publish_apple_music(&mut events);
        self.publish_assistant(&mut events);
        let signed_in = self.auth_state == AuthState::SignedIn;
        if signed_in || self.demo {
            let playback = self.playback_model();
            if self.published.playback.as_ref() != Some(&playback) {
                self.published.playback = Some(playback.clone());
                events.push(CoreEvent::Playback { playback });
            }
            let queue = self.queue_model();
            if self.published.queue.as_ref() != Some(&queue) {
                self.published.queue = Some(queue.clone());
                events.push(CoreEvent::Queue { queue });
            }
            let devices = self.devices_model();
            if self.published.devices.as_ref() != Some(&devices) {
                self.published.devices = Some(devices.clone());
                events.push(CoreEvent::Devices { devices });
            }
            let library = self.library_model();
            if self.published.library.as_ref() != Some(&library) {
                self.published.library = Some(library.clone());
                events.push(CoreEvent::Library { library });
            }
            self.publish_lyrics(&mut events);
            self.publish_album_colors(&mut events);
            let routes: Vec<(String, Route)> = self.views.iter().map(|(k, r)| (k.clone(), r.clone())).collect();
            for (key, route) in routes {
                let page = self.page(&route);
                if self.published.pages.get(&key) != Some(&page) {
                    self.published.pages.insert(key.clone(), page.clone());
                    events.push(CoreEvent::Page { key, page });
                }
            }
        }
        if !self.saved_changed.is_empty() {
            let changes = std::mem::take(&mut self.saved_changed).into_iter().map(|(uri, saved)| SavedFlag { uri, saved }).collect();
            events.push(CoreEvent::Saved { changes });
        }
        for e in events {
            self.emit.on_event(e);
        }
    }

    /// Everything a page might ask about saved state, and the whole map when Swift asks again.
    pub fn resend_saved(&mut self) {
        self.saved_changed = self.saved.iter().map(|(u, b)| (u.clone(), *b)).collect();
    }

    pub fn forget_published(&mut self) {
        self.published = Default::default();
    }

    fn account_model(&self) -> AccountModel {
        let me = self.me.as_ref();
        AccountModel {
            phase: match self.auth_state {
                AuthState::SignedOut => AuthPhase::SignedOut,
                AuthState::SigningIn => AuthPhase::SigningIn,
                AuthState::SignedIn => AuthPhase::SignedIn,
            },
            error: self.auth_error.clone(),
            user_id: me.map(|m| s(m, "id").to_string()).unwrap_or_default(),
            display_name: me.map(|m| s(m, "display_name").to_string()).unwrap_or_default(),
            email: me.map(|m| s(m, "email").to_string()).unwrap_or_default(),
            country: me.map(|m| s(m, "country").to_string()).unwrap_or_default(),
            product: me.map(|m| s(m, "product").to_string()).unwrap_or_default(),
            image: me.and_then(|m| model::image(m, 64)),
            demo: self.demo,
        }
    }

    pub fn now_item(&mut self) -> Option<NowItem> {
        if let Some(t) = self.local.as_ref().filter(|l| l.active).and_then(|l| l.current()).cloned() {
            return Some(NowItem {
                uri: crate::local::spotify_uri(&t),
                kind: RowKind::LocalFile,
                title: t.title.clone(),
                artists: if t.artist.is_empty() { vec![] } else { vec![Link { name: t.artist.clone(), uri: String::new() }] },
                album: (!t.album.is_empty()).then(|| Link { name: t.album.clone(), uri: String::new() }),
                show: None,
                image: t.cover_uri(),
                image_small: t.cover_uri(),
                duration_ms: t.duration_ms,
                explicit: false,
                path: Some(t.path.to_string_lossy().into_owned()),
            });
        }
        let item = self.pb.item.clone()?;
        let uri = s(&item, "uri").to_string();
        let is_local = model::b(&item, "is_local") || uri.starts_with("spotify:local:");
        let episode = s(&item, "type") == "episode";
        // The small cover is the 300 px one: the player bar shows it at ~130 px on a 3x
        // screen (64 px looked soft), and Now Playing shows it while the 640 loads.
        let (mut image, mut image_small, mut path) = (model::image(&item, 640), model::image(&item, 300), None);
        if is_local && let Some(Some(t)) = self.local_link(&uri) {
            image = t.cover_uri();
            image_small = t.cover_uri();
            path = Some(t.path.to_string_lossy().into_owned());
        }
        Some(NowItem {
            kind: if episode {
                RowKind::Episode
            } else if is_local {
                RowKind::LocalLink
            } else {
                RowKind::Track
            },
            title: s(&item, "name").to_string(),
            artists: if episode { vec![] } else { artist_links(&item) },
            album: item.get("album").filter(|a| !a.is_null() && !s(a, "name").is_empty()).map(link_of),
            show: item.get("show").filter(|v| !v.is_null()).map(link_of),
            image,
            image_small,
            duration_ms: model::u(&item, "duration_ms"),
            explicit: model::b(&item, "explicit"),
            path,
            uri,
        })
    }

    fn device_model(&self, d: &Value) -> DeviceModel {
        let id = s(d, "id").to_string();
        DeviceModel {
            is_this: self.engine_device() == Some(id.as_str()),
            name: s(d, "name").to_string(),
            kind: s(d, "type").to_string(),
            volume_percent: d["volume_percent"].as_u64().map(|v| v as u32),
            is_active: model::b(d, "is_active"),
            restricted: model::b(d, "is_restricted"),
            id,
        }
    }

    /// The context's name, from what's already loaded (Library, the page).
    fn context_name(&self, uri: &str) -> String {
        if uri.is_empty() {
            return String::new();
        }
        if uri.ends_with(":collection") {
            return "Liked Songs".into();
        }
        if crate::dj::is_dj(uri) {
            return "DJ".into();
        }
        let id = model::id_of(uri);
        let path = match model::kind_of(uri) {
            "playlist" => format!("/playlists/{id}"),
            "album" => format!("/albums/{id}"),
            "artist" => format!("/artists/{id}"),
            "show" => format!("/shows/{id}"),
            _ => return String::new(),
        };
        if let Some(crate::app::Load::Ready(v)) = self.data.get(&path) {
            return s(v, "name").to_string();
        }
        for key in ["ALL2000:/me/playlists?limit=50", "ALL2000:/me/albums?limit=50"] {
            if let Some(crate::app::Load::Ready(v)) = self.data.get(key) {
                for it in model::items(v) {
                    let e = it.get("album").unwrap_or(it);
                    if s(e, "uri") == uri {
                        return s(e, "name").to_string();
                    }
                }
            }
        }
        String::new()
    }

    fn playback_model(&mut self) -> PlaybackModel {
        let item = self.now_item();
        let local_file = self.local_file_active();
        let (is_playing, position_ms, position_at) = if local_file {
            let l = self.local.as_ref().unwrap();
            let pos = l.position_ms();
            // Rounded so the model doesn't change on every tick while playing: Swift
            // moves the position on itself, and gets a fresh one when it jumps.
            (l.is_playing(), pos, 0)
        } else {
            (self.pb.is_playing, self.pb.progress_ms, unix_ms(self.pb.progress_at))
        };
        let (position_ms, position_at) = if local_file { self.stable_local_position(is_playing, position_ms) } else { (position_ms, position_at) };
        let context_uri = if local_file { String::new() } else { self.pb.context_uri.clone() };
        PlaybackModel {
            item,
            context_name: if local_file { "Local Files".into() } else { self.context_name(&context_uri) },
            context_uri,
            is_playing,
            position_ms,
            position_at,
            shuffle: self.pb.shuffle,
            repeat: match self.pb.repeat.as_str() {
                "context" => Repeat::Context,
                "track" => Repeat::Track,
                _ => Repeat::Off,
            },
            volume: self.pb.volume,
            muted: self.muted_volume.is_some(),
            device: self.pb.device.as_ref().map(|d| self.device_model(d)),
            here: local_file || self.is_local(),
            local_file,
            dj_caption: self.dj_caption(),
            premium: self.is_premium(),
            engine_status: self.engine_status.clone(),
            engine_ready: self.engine.is_some(),
            reconnecting: self.engine.is_none() && self.pending_play.is_some(),
        }
    }

    /// The local player's position as (position, at): kept from the last report while
    /// it moves on as expected, so the model only changes when it jumps (seek, next, pause).
    fn stable_local_position(&mut self, playing: bool, pos: u64) -> (u64, u64) {
        let now = unix_ms(None);
        if let Some(prev) = &self.published.playback
            && prev.local_file
            && prev.is_playing == playing
        {
            let expected = if playing { prev.position_ms + now.saturating_sub(prev.position_at) } else { prev.position_ms };
            if expected.abs_diff(pos) < 1500 {
                return (prev.position_ms, prev.position_at);
            }
        }
        (pos, now)
    }

    fn queue_model(&mut self) -> QueueModel {
        if self.local_file_active() {
            let (current, next) = {
                let l = self.local.as_ref().unwrap();
                (l.current().cloned(), l.upcoming())
            };
            let all = self.local_tracks.clone().unwrap_or_default();
            let by_path: HashMap<&std::path::Path, &crate::local::LocalTrack> = all.iter().map(|t| (t.path.as_path(), t)).collect();
            let queued = self.local.as_ref().map_or(0, |l| l.queued()) as u32;
            // `upcoming` starts with the song playing.
            let rows: Vec<(u32, Row)> = next.iter().skip(1).enumerate().filter_map(|(i, p)| by_path.get(p.as_path()).map(|t| (i as u32, crate::views::local_row(t)))).collect();
            let (positions, next) = rows.into_iter().unzip();
            return QueueModel { current: current.as_ref().map(crate::views::local_row), next, next_from: "Next from: Local Files".into(), loading: false, error: None, local: true, queued, positions, prefetch: Vec::new() };
        }
        let Some(q) = self.queue.clone() else {
            return QueueModel { current: None, next: vec![], next_from: String::new(), loading: self.api.is_some(), error: None, local: false, queued: 0, positions: Vec::new(), prefetch: Vec::new() };
        };
        let current = q.get("currently_playing").filter(|c| !c.is_null()).map(|c| self.row(c, ""));
        let name = self.context_name(&self.pb.context_uri.clone());
        // Playing here: this iPhone's own list (its order, and which songs you queued),
        // with the songs' details from Spotify's answers; songs not described yet are
        // left out until they are. Elsewhere: Spotify's queue as it is.
        let (next, queued, positions) = match self.upcoming.clone().filter(|_| self.is_local() || self.demo) {
            Some(up) => {
                let mut next = Vec::new();
                let mut positions = Vec::new();
                let mut queued = 0;
                for (i, (uri, is_queued)) in up.iter().enumerate().take(80) {
                    let Some(item) = self.queue_meta.get(uri).cloned() else { continue };
                    if *is_queued && queued == next.len() {
                        queued += 1;
                    }
                    next.push(self.row(&item, ""));
                    positions.push(i as u32);
                }
                (next, queued as u32, positions)
            }
            None => (model::arr(&q, "queue").iter().filter(|t| !t.is_null()).map(|t| self.row(t, "")).collect(), 0, Vec::new()),
        };
        QueueModel {
            current,
            next,
            queued,
            positions,
            next_from: if name.is_empty() { "Next up".into() } else { format!("Next from: {name}") },
            loading: false,
            // It's asked for again every few seconds: nothing to tell.
            error: None,
            local: false,
            // The next two songs' covers (Now Playing's and the player bar's), fetched ahead.
            prefetch: model::arr(&q, "queue").iter().filter(|t| !t.is_null()).take(2).flat_map(|t| [model::image(t, 640), model::image(t, 300)]).flatten().collect(),
        }
    }

    fn devices_model(&self) -> DevicesModel {
        let mut devices: Vec<DeviceModel> = self.devices.as_ref().map(|v| model::arr(v, "devices").iter().map(|d| self.device_model(d)).collect()).unwrap_or_default();
        // This iPhone is listed even before Spotify knows it.
        if let Some(id) = self.engine_device()
            && !devices.iter().any(|d| d.id == id)
        {
            devices.insert(
                0,
                DeviceModel {
                    id: id.to_string(),
                    name: self.settings.device_name.clone(),
                    kind: "Smartphone".into(),
                    volume_percent: Some((self.pb.volume * 100.0).round() as u32),
                    is_active: self.is_local() && self.pb.device.is_some(),
                    is_this: true,
                    restricted: false,
                },
            );
        }
        DevicesModel { devices, loaded: self.devices.is_some(), engine_status: self.engine_status.clone() }
    }

    /// Desktop `library::entries` + `specials` + `recent_rank`, unfiltered: Swift picks
    /// the tab, chip and search (see LibraryView.swift), in the same row order.
    fn library_model(&mut self) -> LibraryModel {
        let sources = [
            self.want_all("/me/playlists?limit=50", 2000),
            self.want_all("/me/albums?limit=50", 2000),
            self.want_all("/me/following?type=artist&limit=50", 2000),
            self.want_all("/me/shows?limit=50", 500),
            self.want_all("/me/audiobooks?limit=50", 500),
        ];
        let me = self.user_id();
        let my_name = self.me.as_ref().map(|m| s(m, "display_name").to_string());
        let mut rank: HashMap<String, u32> = HashMap::new();
        if let Some(v) = self.want("/me/player/recently-played?limit=50") {
            for it in model::items(&v) {
                let uri = it["context"]["uri"].as_str().map(str::to_string).or_else(|| it["track"]["album"]["uri"].as_str().map(str::to_string));
                if let Some(u) = uri {
                    let n = rank.len() as u32;
                    rank.entry(u).or_insert(n);
                }
            }
        }
        let pins = self.settings.pins.clone();
        let folder_of: HashMap<String, String> = self.settings.folders.iter().flat_map(|f| f.playlists.iter().map(move |p| (p.clone(), f.id.clone()))).collect();
        let liked = self.want("/me/tracks?limit=1").map(|v| model::u(&v, "total")).unwrap_or(0);
        let eps = self.want("/me/episodes?limit=1").map(|v| model::u(&v, "total")).unwrap_or(0);
        let uid = me.clone();
        let kept = |uri: &str| -> bool { self.downloads.is_kept(uri) };
        let mut items = Vec::new();
        let item = |uri: &str, kind: LibKind, title: &str, subtitle: String, image: Option<String>, owner_id: &str, added_at: &str, detail: String| LibItem {
            uri: uri.to_string(),
            kind,
            title: title.to_string(),
            subtitle,
            image,
            owner_id: owner_id.to_string(),
            added_at: added_at.to_string(),
            recent_rank: rank.get(uri).copied().unwrap_or(u32::MAX),
            pin: pins.iter().position(|p| p == uri).map(|i| i as u32),
            folder: folder_of.get(uri).cloned(),
            children: 0,
            editable: false,
            collaborative: false,
            downloaded: kept(uri),
            detail,
        };
        items.push(item(&format!("spotify:user:{uid}:collection"), LibKind::LikedSongs, "Liked Songs", format!("Playlist • {}", model::songs(liked)), None, &uid, "", String::new()));
        items.push(item(
            &format!("spotify:user:{uid}:collection:your-episodes"),
            LibKind::YourEpisodes,
            "Your Episodes",
            format!("Podcast • {} {}", model::fmt_count(eps), if eps == 1 { "episode" } else { "episodes" }),
            None,
            &uid,
            "",
            String::new(),
        ));
        // Always (the Local tab is always there too).
        items.push(item("glass:local-files", LibKind::LocalFiles, "Local Files", "Playlist • files on this iPhone".into(), None, "", "", String::new()));
        let [playlists, albums, artists, shows, books] = &sources;
        let loading = playlists.is_none();
        if let Some(v) = playlists {
            for p in model::items(v) {
                let owner = s(&p["owner"], "display_name").to_string();
                let owner_id = s(&p["owner"], "id").to_string();
                let by = if owner_id == me { my_name.clone().unwrap_or(owner.clone()) } else { owner.clone() };
                let total = p["tracks"]["total"].as_u64().or_else(|| p["items"]["total"].as_u64()).unwrap_or(0);
                let desc = model::plain(s(p, "description"));
                let mut e = item(s(p, "uri"), LibKind::Playlist, s(p, "name"), format!("Playlist • {by}"), model::image(p, 300), &owner_id, s(p, "added_at"), if desc.is_empty() { format!("by {by} · {}", model::songs(total)) } else { desc });
                e.editable = owner_id == me;
                e.collaborative = model::b(p, "collaborative");
                items.push(e);
            }
        }
        if let Some(v) = albums {
            for it in model::items(v) {
                let a = &it["album"];
                let detail = format!("{} · {}", model::artists_line(a), model::year(s(a, "release_date")));
                items.push(item(s(a, "uri"), LibKind::Album, s(a, "name"), format!("Album • {}", model::artists_line(a)), model::image(a, 300), "", s(it, "added_at"), detail));
            }
        }
        if let Some(v) = artists {
            for a in model::items(v) {
                items.push(item(s(a, "uri"), LibKind::Artist, s(a, "name"), "Artist".into(), model::image(a, 300), "", "", "artist".into()));
            }
        }
        if let Some(v) = shows {
            for it in model::items(v) {
                let sh = &it["show"];
                items.push(item(s(sh, "uri"), LibKind::Show, s(sh, "name"), format!("Podcast • {}", s(sh, "publisher")), model::image(sh, 300), "", s(it, "added_at"), s(sh, "publisher").into()));
            }
        }
        if let Some(v) = books {
            for it in model::items(v) {
                let b = it.get("audiobook").unwrap_or(it);
                items.push(item(s(b, "uri"), LibKind::Audiobook, s(b, "name"), format!("Audiobook • {}", model::artists_line(b)), model::image(b, 300), "", s(it, "added_at"), model::artists_line(b)));
            }
        }
        // Creator sort uses the creator's name; it rides in owner_id for playlists, so
        // the sort happens here, as the desktop does it.
        let creator = |e: &LibItem| -> String {
            match e.kind {
                LibKind::Playlist => e.subtitle.trim_start_matches("Playlist • ").to_lowercase(),
                LibKind::Album => e.subtitle.trim_start_matches("Album • ").to_lowercase(),
                LibKind::Show => e.detail.to_lowercase(),
                LibKind::Audiobook => e.detail.to_lowercase(),
                _ => String::new(),
            }
        };
        let specials = items.iter().take_while(|e| matches!(e.kind, LibKind::LikedSongs | LibKind::YourEpisodes | LibKind::LocalFiles)).count();
        let rest = &mut items[specials..];
        match self.settings.library_sort {
            crate::config::LibrarySort::Recents => rest.sort_by_key(|e| e.recent_rank),
            crate::config::LibrarySort::RecentlyAdded => rest.sort_by(|a, b| b.added_at.cmp(&a.added_at)),
            crate::config::LibrarySort::Alphabetical => rest.sort_by_cached_key(|e| e.title.to_lowercase()),
            crate::config::LibrarySort::Creator => rest.sort_by_cached_key(creator),
        }
        for f in &self.settings.folders {
            let n = f.playlists.len();
            let mut e = item(&format!("glass:folder:{}", f.id), LibKind::Folder, &f.name, format!("Folder • {n} playlist{}", if n == 1 { "" } else { "s" }), None, "", "", String::new());
            e.children = n as u32;
            items.push(e);
        }
        let downloads: Vec<Link> = self.downloads.collections().into_iter().map(|(uri, name)| Link { name, uri }).collect();
        let download_progress = downloads
            .iter()
            .filter_map(|l| self.downloads.progress(&l.uri).map(|(done, total)| crate::types::DownloadProgress { uri: l.uri.clone(), done: done as u32, total: total as u32 }))
            .collect();
        LibraryModel {
            state: if loading && self.api.is_some() { self.load_error("/me/playlists?limit=50").map(|message| LoadState::Failed { message }).unwrap_or(LoadState::Loading) } else { LoadState::Ready },
            items,
            sort: match self.settings.library_sort {
                crate::config::LibrarySort::Recents => LibrarySort::Recents,
                crate::config::LibrarySort::RecentlyAdded => LibrarySort::RecentlyAdded,
                crate::config::LibrarySort::Alphabetical => LibrarySort::Alphabetical,
                crate::config::LibrarySort::Creator => LibrarySort::Creator,
            },
            view: match self.settings.library_view {
                crate::config::LibraryView::List => LibraryView::List,
                crate::config::LibraryView::Compact => LibraryView::Compact,
                crate::config::LibraryView::Grid => LibraryView::Grid,
            },
            downloads,
            download_progress,
            greeting: model::greeting().to_lowercase(),
        }
    }
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct SavedFlag {
    pub uri: String,
    pub saved: bool,
}
