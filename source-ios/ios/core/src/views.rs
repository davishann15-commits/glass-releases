//! Page models: what each desktop page (src/ui/pages.rs) reads, as typed
//! records for SwiftUI. Builders ask for data with `want`/`want_all` exactly as
//! the desktop pages do (same paths, same caps), so caching, refreshing and
//! invalidation behave the same; what hasn't loaded yet is left out and the
//! page is rebuilt when it arrives.

use crate::app::MobileApp;
use crate::model::{self, id_of, kind_of, s};
use crate::types::*;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub fn route_key(r: &Route) -> String {
    match r {
        Route::Library => "library".into(),
        Route::Home => "home".into(),
        Route::Search { query, filter } => format!("search:{filter:?}:{query}"),
        Route::Genre { id, .. } => format!("genre:{id}"),
        Route::Playlist { id } => format!("playlist:{id}"),
        Route::Album { id } => format!("album:{id}"),
        Route::Artist { id } => format!("artist:{id}"),
        Route::Discography { id } => format!("discography:{id}"),
        Route::Show { id } => format!("show:{id}"),
        Route::Episode { id } => format!("episode:{id}"),
        Route::Audiobook { id } => format!("audiobook:{id}"),
        Route::Track { id } => format!("track:{id}"),
        Route::Profile { id } => format!("profile:{id}"),
        Route::LikedSongs => "liked".into(),
        Route::YourEpisodes => "episodes".into(),
        Route::LocalFiles => "local".into(),
        Route::Recents => "recents".into(),
        Route::Folder { id } => format!("folder:{id}"),
        Route::LocalPlaylist { id } => format!("local-playlist:{id}"),
        Route::LocalAlbum { artist, album } => format!("local-album:{artist}\u{1f}{album}"),
        Route::Device { id, .. } => format!("device:{id}"),
        Route::Settings => "settings".into(),
        Route::Activity => "activity".into(),
    }
}

/// The page for a Spotify item (desktop `menus::route_for`).
pub fn route_for(v: &Value) -> Option<Route> {
    let id = s(v, "id").to_string();
    Some(match s(v, "type") {
        "playlist" => Route::Playlist { id },
        "album" => Route::Album { id },
        "artist" => Route::Artist { id },
        "show" => Route::Show { id },
        "episode" => Route::Episode { id },
        "audiobook" => Route::Audiobook { id },
        "track" => Route::Track { id },
        "user" => Route::Profile { id },
        "collection" => Route::LikedSongs,
        _ => return None,
    })
}

/// Desktop `menus::subtitle_for`.
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

pub fn link_of(v: &Value) -> Link {
    let uri = s(v, "uri").to_string();
    let uri = if uri.is_empty() && !s(v, "id").is_empty() && !s(v, "type").is_empty() { format!("spotify:{}:{}", s(v, "type"), s(v, "id")) } else { uri };
    Link { name: s(v, "name").if_empty(s(v, "display_name")).to_string(), uri }
}

pub fn artist_links(v: &Value) -> Vec<Link> {
    model::arr(v, "artists").iter().map(link_of).collect()
}

pub fn card(v: &Value) -> Card {
    let kind = s(v, "type").to_string();
    Card {
        uri: s(v, "uri").to_string(),
        title: s(v, "name").if_empty(s(v, "display_name")).to_string(),
        subtitle: subtitle_for(v),
        image: model::image(v, 300),
        round: matches!(kind.as_str(), "artist" | "user"),
        group: s(v, "album_group").if_empty(s(v, "album_type")).to_string(),
        year: model::year(s(v, "release_date")).to_string(),
        kind,
    }
}

fn cards<'a>(items: impl IntoIterator<Item = &'a Value>) -> Vec<Card> {
    items.into_iter().filter(|v| !v.is_null() && !s(v, "uri").is_empty()).map(card).collect()
}

trait IfEmpty {
    fn if_empty<'a>(&'a self, other: &'a str) -> &'a str;
}
impl IfEmpty for str {
    fn if_empty<'a>(&'a self, other: &'a str) -> &'a str {
        if self.is_empty() { other } else { self }
    }
}

impl MobileApp {
    /// A song row (desktop tracklist): greyed when unavailable, explicit while
    /// explicit content is off, or a local song with no file here.
    pub fn row(&mut self, it: &Value, added_at: &str) -> Row {
        let uri = s(it, "uri").to_string();
        let is_local = model::b(it, "is_local") || uri.starts_with("spotify:local:");
        let kind = match s(it, "type") {
            "episode" => RowKind::Episode,
            "chapter" => RowKind::Chapter,
            _ if is_local => RowKind::LocalLink,
            _ => RowKind::Track,
        };
        let mut note = None;
        let mut path = None;
        let mut image = model::image(it, 64);
        let mut missing = false;
        if is_local {
            match self.local_link(&uri) {
                Some(Some(t)) => {
                    note = Some(format!("Local file: {}", display_path(&t.path)));
                    image = t.cover_uri();
                    path = Some(t.path.to_string_lossy().into_owned());
                }
                Some(None) => {
                    missing = true;
                    note = Some("This local song isn't on this device. Put the file in Glass's music folder or import it in Local Files to play it here.".to_string());
                }
                None => {}
            }
        }
        let explicit = model::b(it, "explicit");
        let unavailable = it.get("is_playable").and_then(Value::as_bool) == Some(false);
        if unavailable && note.is_none() {
            note = Some("This song isn't available".into());
        }
        let playable = !unavailable && !(!self.settings.allow_explicit && explicit) && !missing;
        let resume = &it["resume_point"];
        Row {
            title: s(it, "name").to_string(),
            artists: if kind == RowKind::Episode { Vec::new() } else { artist_links(it) },
            album: it.get("album").filter(|a| !a.is_null() && !s(a, "name").is_empty()).map(link_of),
            image,
            duration_ms: model::u(it, "duration_ms"),
            explicit,
            playable,
            note,
            added_at: added_at.to_string(),
            track_number: model::u(it, "track_number") as u32,
            disc_number: model::u(it, "disc_number").max(1) as u32,
            path,
            release_date: s(it, "release_date").to_string(),
            description: if kind == RowKind::Episode { s(it, "description").chars().take(300).collect() } else { String::new() },
            resume_ms: model::u(resume, "resume_position_ms"),
            played: model::b(resume, "fully_played"),
            parent: it.get("show").or_else(|| it.get("audiobook")).filter(|v| !v.is_null()).map(link_of),
            uri,
            kind,
        }
    }

    fn rows_from_items(&mut self, items: &[Value]) -> Vec<Row> {
        items
            .iter()
            .filter_map(|it| {
                let e = model::playlist_entry(it);
                (!e.is_null()).then(|| self.row(e, s(it, "added_at")))
            })
            .collect()
    }

    fn rows_plain(&mut self, items: &[Value]) -> Vec<Row> {
        items.iter().filter(|it| !it.is_null()).map(|it| self.row(it, "")).collect()
    }

    /// Ready, loading or failed, for a page whose main data is `path`.
    fn wait_state(&self, path: &str) -> LoadState {
        match self.load_error(path) {
            Some(message) => LoadState::Failed { message },
            None => LoadState::Loading,
        }
    }

    pub fn page(&mut self, route: &Route) -> PageModel {
        let mut p = PageModel { route: route.clone(), state: LoadState::Ready, header: None, blocks: Vec::new() };
        match route {
            Route::Home => self.home(&mut p),
            Route::Search { query, filter } => self.search(&mut p, query, *filter),
            Route::Genre { id, name } => self.genre(&mut p, id, name),
            Route::Playlist { id } => self.playlist(&mut p, id),
            Route::Album { id } => self.album(&mut p, id),
            Route::Artist { id } => self.artist(&mut p, id),
            Route::Discography { id } => self.discography(&mut p, id),
            Route::Show { id } => self.show(&mut p, id),
            Route::Episode { id } => self.episode(&mut p, id),
            Route::Audiobook { id } => self.audiobook(&mut p, id),
            Route::Track { id } => self.track(&mut p, id),
            Route::Profile { id } => self.profile(&mut p, id),
            Route::LikedSongs => self.liked(&mut p),
            Route::YourEpisodes => self.your_episodes(&mut p),
            Route::Recents => self.recents(&mut p),
            Route::Folder { id } => self.folder(&mut p, id),
            Route::LocalPlaylist { id } => self.local_playlist(&mut p, id),
            Route::Library | Route::LocalFiles | Route::LocalAlbum { .. } | Route::Device { .. } | Route::Settings | Route::Activity => {}
        }
        p
    }

    fn header(uri: &str, kind_label: &str, title: &str, image: Option<String>, art: ArtStyle) -> Header {
        Header {
            uri: uri.to_string(),
            kind_label: kind_label.to_string(),
            title: title.to_string(),
            links: Vec::new(),
            meta: String::new(),
            description: String::new(),
            image,
            art,
            save: SaveKind::None,
            can_play: !uri.is_empty(),
            can_shuffle: !uri.is_empty(),
            can_download: false,
            editable: false,
            can_add: false,
            collaborative: false,
            public: false,
            owner_id: String::new(),
            listeners: String::new(),
            missing_local: 0,
        }
    }

    // ---------------------------------------------------------------- Home

    /// Desktop `recent_contexts`: the contexts of the last plays, newest first.
    fn recent_contexts(&mut self) -> Vec<Value> {
        let Some(rp) = self.want("/me/player/recently-played?limit=50") else { return vec![] };
        let uid = self.user_id();
        let mine = self.want_all("/me/playlists?limit=50", 2000);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for it in model::items(&rp) {
            let track = &it["track"];
            let ctx = it.get("context").filter(|c| !c.is_null());
            let uri = ctx.map(|c| s(c, "uri").to_string()).unwrap_or_else(|| s(&track["album"], "uri").to_string());
            if uri.is_empty() || !seen.insert(uri.clone()) {
                continue;
            }
            let v = match kind_of(&uri) {
                "album" => Some(track["album"].clone()),
                "playlist" => mine
                    .as_ref()
                    .and_then(|m| model::items(m).iter().find(|p| s(p, "uri") == uri).cloned())
                    .or_else(|| self.want(&format!("/playlists/{}?fields=id,uri,type,name,images,owner(display_name,id)", id_of(&uri))).map(|v| (*v).clone())),
                "artist" => self.want(&format!("/artists/{}", id_of(&uri))).map(|v| (*v).clone()),
                "show" => self.want(&format!("/shows/{}", id_of(&uri))).map(|v| (*v).clone()),
                "collection" | "user" if uri.ends_with(":collection") => Some(json!({ "type": "collection", "uri": format!("spotify:user:{uid}:collection"), "name": "Liked Songs" })),
                _ => None,
            };
            if let Some(v) = v {
                out.push(v);
            }
            if out.len() >= 20 {
                break;
            }
        }
        out
    }

    /// Desktop `home`. All four chips' sections are built; Swift shows the ones the chip picks
    /// (block IDs say which: "music.*", "podcasts.*", "audiobooks.*").
    fn home(&mut self, p: &mut PageModel) {
        let here = crate::dj::is_dj(&self.pb.context_uri);
        p.blocks.push(Block::Dj { caption: self.dj_caption().filter(|_| here), voice_on: self.settings.dj_voice, playing_here: here && self.pb.is_playing });
        let recents = self.recent_contexts();
        let tiles: Vec<Card> = recents.iter().take(8).map(card).collect();
        if !tiles.is_empty() {
            p.blocks.push(Block::Shortcuts { cards: tiles });
        }
        if recents.len() > 8 {
            p.blocks.push(Block::Cards { id: "music.jump".into(), title: "Jump back in".into(), cards: cards(recents[8..].iter().filter(|v| s(v, "type") != "collection")), style: CardsStyle::Shelf, more: None });
        }
        if let Some(v) = self.want("/me/top/artists?limit=12&time_range=short_term")
            && !model::items(&v).is_empty()
        {
            p.blocks.push(Block::Cards { id: "music.top-artists".into(), title: "Your top artists".into(), cards: cards(model::items(&v)), style: CardsStyle::Shelf, more: None });
        }
        if let Some(v) = self.want("/me/top/tracks?limit=12&time_range=short_term") {
            let mut seen = HashSet::new();
            let albums: Vec<&Value> = model::items(&v).iter().filter_map(|t| t.get("album")).filter(|a| seen.insert(s(a, "id").to_string())).collect();
            if !albums.is_empty() {
                p.blocks.push(Block::Cards { id: "music.on-repeat".into(), title: "On repeat".into(), cards: cards(albums), style: CardsStyle::Shelf, more: None });
            }
        }
        if let Some(v) = self.want_all("/me/playlists?limit=50", 2000)
            && !model::items(&v).is_empty()
        {
            p.blocks.push(Block::Cards { id: "music.playlists".into(), title: "Your playlists".into(), cards: cards(model::items(&v)), style: CardsStyle::Shelf, more: Some(Route::Library) });
        }
        if let Some(v) = self.want_all("/me/albums?limit=50", 2000)
            && !model::items(&v).is_empty()
        {
            p.blocks.push(Block::Cards { id: "music.albums".into(), title: "Saved albums".into(), cards: cards(model::items(&v).iter().map(|i| &i["album"])), style: CardsStyle::Shelf, more: None });
        }
        if let Some(v) = self.want_all("/me/shows?limit=50", 500)
            && !model::items(&v).is_empty()
        {
            p.blocks.push(Block::Cards { id: "podcasts.shows".into(), title: "Your shows".into(), cards: cards(model::items(&v).iter().map(|i| &i["show"])), style: CardsStyle::Shelf, more: None });
        }
        if let Some(v) = self.want("/me/episodes?limit=10") {
            let eps: Vec<Value> = model::items(&v).iter().map(|i| i["episode"].clone()).filter(|e| !e.is_null()).collect();
            if !eps.is_empty() {
                let rows = self.rows_plain(&eps);
                p.blocks.push(Block::Rows {
                    id: "podcasts.episodes".into(),
                    title: "Your Episodes".into(),
                    rows,
                    style: RowsStyle::Episodes,
                    context: String::new(),
                    playlist: None,
                    more: Some(Route::YourEpisodes),
                    collapsed_count: 3,
                });
            }
        }
        if let Some(v) = self.want_all("/me/audiobooks?limit=50", 500) {
            let books: Vec<&Value> = model::items(&v).iter().map(|i| i.get("audiobook").unwrap_or(i)).collect();
            if books.is_empty() {
                p.blocks.push(Block::Empty { title: "no audiobooks yet".into(), body: "Audiobooks you save appear here.".into() });
            } else {
                p.blocks.push(Block::Cards { id: "audiobooks.books".into(), title: "Your audiobooks".into(), cards: cards(books), style: CardsStyle::Shelf, more: None });
            }
        }
    }

    // -------------------------------------------------------------- Search

    fn search(&mut self, p: &mut PageModel, query: &str, filter: SearchFilter) {
        let q = query.trim();
        if q.is_empty() {
            if !self.settings.recent_searches.is_empty() {
                p.blocks.push(Block::RecentSearches { queries: self.settings.recent_searches.clone() });
            }
            match self.want("/browse/categories?limit=50") {
                Some(v) => {
                    let categories = model::arr(&v["categories"], "items")
                        .iter()
                        .map(|c| Category {
                            id: s(c, "id").to_string(),
                            name: s(c, "name").to_string(),
                            image: c["icons"][0]["url"].as_str().map(str::to_string),
                            color: s(c, "color").to_string(),
                        })
                        .collect();
                    p.blocks.push(Block::Categories { categories });
                }
                None => p.state = self.wait_state("/browse/categories?limit=50"),
            }
            return;
        }
        let qe = crate::api::enc(q);
        let key = match filter {
            SearchFilter::All => None,
            SearchFilter::Songs => Some("track"),
            SearchFilter::Artists => Some("artist"),
            SearchFilter::Albums => Some("album"),
            SearchFilter::Playlists => Some("playlist"),
            SearchFilter::Podcasts => Some("show"),
            SearchFilter::Episodes => Some("episode"),
            SearchFilter::Audiobooks => Some("audiobook"),
        };
        if let Some(t) = key {
            let path = format!("/search?q={qe}&type={t}&limit=10");
            let Some(v) = self.want_all(&path, 50) else {
                p.state = self.wait_state(&path);
                return;
            };
            let items: Vec<Value> = model::items(&v).iter().filter(|i| !i.is_null()).cloned().collect();
            if items.is_empty() {
                p.state = no_results(q);
                return;
            }
            match t {
                "track" => {
                    let rows = self.rows_plain(&items);
                    p.blocks.push(Block::Rows { id: "songs".into(), title: String::new(), rows, style: RowsStyle::Table, context: String::new(), playlist: None, more: None, collapsed_count: 0 });
                }
                "episode" => {
                    let rows = self.rows_plain(&items);
                    p.blocks.push(Block::Rows { id: "episodes".into(), title: String::new(), rows, style: RowsStyle::Episodes, context: String::new(), playlist: None, more: None, collapsed_count: 0 });
                }
                _ => p.blocks.push(Block::Cards { id: t.into(), title: String::new(), cards: cards(&items), style: CardsStyle::Grid, more: None }),
            }
            return;
        }
        let path = format!("/search?q={qe}&type=track,artist,album,playlist,show,episode,audiobook&limit=10");
        let Some(v) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let list = |k: &str| -> Vec<Value> { v[k]["items"].as_array().map(|a| a.iter().filter(|i| !i.is_null()).cloned().collect()).unwrap_or_default() };
        let (tracks, artists, albums, playlists, shows, episodes, books) = (list("tracks"), list("artists"), list("albums"), list("playlists"), list("shows"), list("episodes"), list("audiobooks"));
        if [&tracks, &artists, &albums, &playlists, &shows, &episodes, &books].iter().all(|l| l.is_empty()) {
            p.state = no_results(q);
            return;
        }
        let ql = q.to_lowercase();
        if let Some(t) = artists.iter().find(|a| s(a, "name").to_lowercase() == ql).or_else(|| tracks.first()).or_else(|| artists.first()) {
            p.blocks.push(Block::TopResult { card: card(t) });
        }
        if !tracks.is_empty() {
            let rows = self.rows_plain(&tracks);
            p.blocks.push(Block::Rows { id: "songs".into(), title: "Songs".into(), rows, style: RowsStyle::Table, context: String::new(), playlist: None, more: None, collapsed_count: 4 });
        }
        for (id, title, items, f) in [
            ("artists", "Artists", artists, SearchFilter::Artists),
            ("albums", "Albums", albums, SearchFilter::Albums),
            ("playlists", "Playlists", playlists, SearchFilter::Playlists),
            ("shows", "Podcasts", shows, SearchFilter::Podcasts),
            ("episodes", "Episodes", episodes, SearchFilter::Episodes),
            ("audiobooks", "Audiobooks", books, SearchFilter::Audiobooks),
        ] {
            if items.is_empty() {
                continue;
            }
            p.blocks.push(Block::Cards { id: id.into(), title: title.into(), cards: cards(&items), style: CardsStyle::Shelf, more: Some(Route::Search { query: q.to_string(), filter: f }) });
        }
    }

    fn genre(&mut self, p: &mut PageModel, id: &str, name: &str) {
        p.header = Some(Self::header("", "", name, None, ArtStyle::Square));
        let path = format!("/browse/categories/{id}/playlists?limit=50");
        match self.want(&path) {
            Some(v) => {
                let items = v["playlists"]["items"].as_array().cloned().unwrap_or_default();
                p.blocks.push(Block::Cards { id: "playlists".into(), title: String::new(), cards: cards(&items), style: CardsStyle::Grid, more: None });
            }
            None if self.load_error(&path).is_some() => {
                p.blocks.push(Block::Empty { title: "couldn't load this category".into(), body: "Spotify didn't return this page. Searching for its name may still find its playlists.".into() });
                p.blocks.push(Block::SearchInstead { query: name.to_string() });
            }
            None => p.state = LoadState::Loading,
        }
    }

    // ------------------------------------------------------------ Playlist

    fn dj_page(&mut self, p: &mut PageModel) {
        let here = crate::dj::is_dj(&self.pb.context_uri);
        let mut h = Self::header(crate::dj::URI, "spotify", "DJ", Some(crate::dj::COVER.to_string()), ArtStyle::Dj);
        h.description = "Your personal DJ, X: songs you love, ones you used to, and new picks, with X introducing them as they play.".into();
        h.can_shuffle = false;
        p.header = Some(h);
        p.blocks.push(Block::Dj { caption: self.dj_caption().filter(|_| here), voice_on: self.settings.dj_voice, playing_here: here && self.pb.is_playing });
    }

    fn playlist(&mut self, p: &mut PageModel, id: &str) {
        if crate::dj::is_dj(&format!("spotify:playlist:{id}")) {
            return self.dj_page(p);
        }
        let path = format!("/playlists/{id}");
        let Some(pl) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let me = self.user_id();
        let own = pl["owner"]["id"].as_str() == Some(me.as_str());
        let collaborative = model::b(&pl, "collaborative");
        let editable = own || collaborative;
        let tracks_path = format!("/playlists/{id}/tracks?limit=100");
        let items_path = format!("/playlists/{id}/items?limit=100");
        let items = match self.want_all(&tracks_path, 10_000) {
            Some(v) => Some(v),
            None if self.load_error(&tracks_path).is_some() => self.want_all(&items_path, 10_000),
            None => None,
        };
        let rows = items.as_ref().map(|v| self.rows_from_items(model::items(v))).unwrap_or_default();
        let total = pl["tracks"]["total"].as_u64().or_else(|| pl["items"]["total"].as_u64()).unwrap_or(rows.len() as u64);
        let dur: u64 = rows.iter().map(|r| r.duration_ms).sum();
        let kind = if collaborative {
            "Collaborative Playlist"
        } else if model::b(&pl, "public") {
            "Public Playlist"
        } else {
            "Playlist"
        };
        let uri = s(&pl, "uri").to_string();
        let mut h = Self::header(&uri, kind, s(&pl, "name"), model::image(&pl, 640), ArtStyle::Square);
        h.description = model::plain(s(&pl, "description"));
        h.links = vec![Link { name: s(&pl["owner"], "display_name").to_string(), uri: format!("spotify:user:{}", s(&pl["owner"], "id")) }];
        h.missing_local = rows.iter().filter(|r| r.kind == RowKind::LocalLink && !r.playable).count() as u32;
        let saves = model::u(&pl["followers"], "total");
        let mut meta = Vec::new();
        if saves > 0 {
            meta.push(if saves == 1 { "1 save".to_string() } else { format!("{} saves", model::fmt_count(saves)) });
        }
        meta.push(format!("{}{}", model::songs(total), if dur > 0 { format!(", {}", model::fmt_long(dur)) } else { String::new() }));
        h.meta = meta.join(" • ");
        h.save = if own { SaveKind::None } else { SaveKind::Library };
        h.can_download = true;
        h.editable = own;
        h.can_add = editable;
        h.collaborative = collaborative;
        h.public = model::b(&pl, "public");
        h.owner_id = s(&pl["owner"], "id").to_string();
        p.header = Some(h);
        if items.is_none() {
            let waiting = if self.load_error(&tracks_path).is_some() { &items_path } else { &tracks_path };
            p.state = self.wait_state(waiting);
            return;
        }
        if rows.is_empty() && own {
            p.blocks.push(Block::FindMore { playlist: id.to_string(), empty: true });
            return;
        }
        p.blocks.push(Block::Rows { id: "tracks".into(), title: String::new(), rows, style: RowsStyle::Table, context: uri, playlist: editable.then(|| id.to_string()), more: None, collapsed_count: 0 });
        if own {
            p.blocks.push(Block::FindMore { playlist: id.to_string(), empty: false });
        }
        if let Some(v) = &items
            && let Some(b) = self.featured(&model::items(v)[..model::items(v).len().min(100)])
        {
            p.blocks.push(b);
        }
    }

    /// Desktop library detail pane's "featured artists": the four artists who appear
    /// most in these entries (playlist items or saved tracks), most first.
    fn featured(&mut self, items: &[Value]) -> Option<Block> {
        let mut n: HashMap<String, usize> = HashMap::new();
        let mut order = Vec::new();
        for it in items {
            for a in model::arr(model::playlist_entry(it), "artists") {
                let id = s(a, "id").to_string();
                if id.is_empty() {
                    continue;
                }
                let c = n.entry(id.clone()).or_insert(0);
                if *c == 0 {
                    order.push(id);
                }
                *c += 1;
            }
        }
        order.sort_by_key(|id| std::cmp::Reverse(n[id]));
        let artists: Vec<Arc<Value>> = order.iter().take(4).filter_map(|id| self.want(&format!("/artists/{id}"))).collect();
        (!artists.is_empty()).then(|| Block::Cards { id: "featured".into(), title: "Featured artists".into(), cards: cards(artists.iter().map(|a| a.as_ref())), style: CardsStyle::Shelf, more: None })
    }

    // --------------------------------------------------------------- Album

    fn album(&mut self, p: &mut PageModel, id: &str) {
        let path = format!("/albums/{id}");
        let Some(a) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let total = model::u(&a["tracks"], "total");
        let all = if (model::items(&a["tracks"]).len() as u64) < total { self.want_all(&format!("/albums/{id}/tracks?limit=50"), 2000) } else { None };
        let stub = json!({ "id": s(&a, "id"), "name": s(&a, "name"), "uri": s(&a, "uri"), "images": a["images"].clone() });
        let source: Vec<Value> = all.as_ref().map(|v| model::items(v).to_vec()).unwrap_or_else(|| model::items(&a["tracks"]).to_vec());
        let tracks: Vec<Value> = source
            .into_iter()
            .map(|mut t| {
                if t.get("album").is_none_or(Value::is_null) {
                    t["album"] = stub.clone();
                }
                t
            })
            .collect();
        let dur: u64 = tracks.iter().map(|t| model::u(t, "duration_ms")).sum();
        let kind = match s(&a, "album_type") {
            "single" => "Single",
            "compilation" => "Compilation",
            _ => "Album",
        };
        let uri = s(&a, "uri").to_string();
        let mut h = Self::header(&uri, kind, s(&a, "name"), model::image(&a, 640), ArtStyle::Square);
        h.links = artist_links(&a);
        h.meta = format!("{} • {}, {}", model::year(s(&a, "release_date")), model::songs(tracks.len() as u64), model::fmt_long(dur));
        h.save = SaveKind::Library;
        h.can_download = true;
        p.header = Some(h);
        let rows = self.rows_plain(&tracks);
        p.blocks.push(Block::Rows { id: "tracks".into(), title: String::new(), rows, style: RowsStyle::Album, context: uri, playlist: None, more: None, collapsed_count: 0 });
        let mut lines = vec![model::fmt_date(s(&a, "release_date"))];
        for c in model::arr(&a, "copyrights") {
            let sym = if s(c, "type") == "P" { "℗" } else { "©" };
            let text = s(c, "text");
            lines.push(if text.starts_with('©') || text.starts_with('℗') || text.starts_with("(C)") || text.starts_with("(P)") { text.to_string() } else { format!("{sym} {text}") });
        }
        p.blocks.push(Block::Lines { lines });
        if let Some(artist) = model::arr(&a, "artists").first().cloned() {
            let aid = s(&artist, "id").to_string();
            if let Some(v) = self.want(&format!("/artists/{aid}/albums?include_groups=album,single&limit=12")) {
                let others: Vec<&Value> = model::items(&v).iter().filter(|x| s(x, "id") != id).collect();
                if !others.is_empty() {
                    p.blocks.push(Block::Cards { id: "more".into(), title: format!("More by {}", s(&artist, "name")), cards: cards(others), style: CardsStyle::Shelf, more: Some(Route::Discography { id: aid }) });
                }
            }
        }
    }

    // -------------------------------------------------------------- Artist

    fn artist(&mut self, p: &mut PageModel, id: &str) {
        let path = format!("/artists/{id}");
        let Some(a) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let uri = s(&a, "uri").to_string();
        let mut h = Self::header(&uri, "artist", s(&a, "name"), model::image(&a, 640), ArtStyle::Banner);
        h.listeners = a["monthly_listeners"].as_u64().map(|n| format!("{} monthly listeners", model::fmt_count(n))).unwrap_or_else(|| format!("{} followers", model::fmt_count(model::u(&a["followers"], "total"))));
        h.save = SaveKind::Follow;
        p.header = Some(h);
        if let Some(v) = self.want(&format!("/artists/{id}/top-tracks?market=from_token")) {
            let tracks = model::arr(&v, "tracks").to_vec();
            if !tracks.is_empty() {
                let rows = self.rows_plain(&tracks[..tracks.len().min(10)]);
                p.blocks.push(Block::Rows { id: "popular".into(), title: "Popular".into(), rows, style: RowsStyle::Popular, context: String::new(), playlist: None, more: None, collapsed_count: 5 });
            }
        }
        if let Some(v) = self.want_all(&format!("/artists/{id}/albums?include_groups=album,single,compilation&limit=50"), 200) {
            let mut all: Vec<&Value> = model::items(&v).iter().collect();
            all.sort_by(|a, b| s(b, "release_date").cmp(s(a, "release_date")));
            if !all.is_empty() {
                p.blocks.push(Block::Cards { id: "discography".into(), title: "Discography".into(), cards: cards(all), style: CardsStyle::Shelf, more: Some(Route::Discography { id: id.to_string() }) });
            }
        }
        if let Some(v) = self.want(&format!("/artists/{id}/related-artists")) {
            let rel = model::arr(&v, "artists");
            if !rel.is_empty() {
                p.blocks.push(Block::Cards { id: "related".into(), title: "Fans also like".into(), cards: cards(rel), style: CardsStyle::Shelf, more: None });
            }
        }
        if let Some(v) = self.want(&format!("/artists/{id}/albums?include_groups=appears_on&limit=20")) {
            let ap = model::items(&v);
            if !ap.is_empty() {
                p.blocks.push(Block::Cards { id: "appears-on".into(), title: "Appears On".into(), cards: cards(ap), style: CardsStyle::Shelf, more: None });
            }
        }
        let mut about = vec![format!("{} followers", model::fmt_count(model::u(&a["followers"], "total")))];
        let genres: Vec<&str> = model::arr(&a, "genres").iter().filter_map(Value::as_str).collect();
        if !genres.is_empty() {
            about.push(genres.join(" · "));
        }
        if let Some(n) = a["monthly_listeners"].as_u64() {
            about.push(format!("{} monthly listeners", model::fmt_count(n)));
        }
        let bio = s(&a, "biography");
        if !bio.is_empty() {
            about.push(bio.to_string());
        }
        p.blocks.push(Block::Text { title: "About".into(), body: about.join("\n") });
    }

    fn discography(&mut self, p: &mut PageModel, id: &str) {
        let name = self.want(&format!("/artists/{id}")).map(|a| s(&a, "name").to_string()).unwrap_or_default();
        let mut h = Self::header("", "discography", &name, None, ArtStyle::Square);
        h.links = vec![Link { name: name.clone(), uri: format!("spotify:artist:{id}") }];
        p.header = Some(h);
        let path = format!("/artists/{id}/albums?include_groups=album,single,compilation&limit=50");
        let Some(v) = self.want_all(&path, 500) else {
            p.state = self.wait_state(&path);
            return;
        };
        let mut all: Vec<&Value> = model::items(&v).iter().collect();
        all.sort_by(|a, b| s(b, "release_date").cmp(s(a, "release_date")));
        p.blocks.push(Block::Cards { id: "discography".into(), title: String::new(), cards: cards(all), style: CardsStyle::Grid, more: None });
    }

    // ----------------------------------------------------- Podcasts, books

    fn show(&mut self, p: &mut PageModel, id: &str) {
        let path = format!("/shows/{id}");
        let Some(sh) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let uri = s(&sh, "uri").to_string();
        let mut h = Self::header(&uri, "Podcast", s(&sh, "name"), model::image(&sh, 640), ArtStyle::Square);
        h.links = vec![Link { name: s(&sh, "publisher").to_string(), uri: String::new() }];
        h.save = SaveKind::Follow;
        h.can_play = false;
        h.can_shuffle = false;
        h.can_download = true;
        p.header = Some(h);
        let n = model::u(&sh, "total_episodes");
        p.blocks.push(Block::Text { title: "About".into(), body: format!("{}\n{}", model::plain(s(&sh, "description")), if n == 1 { "1 episode".to_string() } else { format!("{n} episodes") }) });
        let eps_path = format!("/shows/{id}/episodes?limit=50");
        match self.want_all(&eps_path, 1000) {
            Some(v) => {
                let show = json!({ "id": s(&sh, "id"), "name": s(&sh, "name"), "uri": uri, "type": "show" });
                let eps: Vec<Value> = model::items(&v)
                    .iter()
                    .filter(|e| !e.is_null())
                    .map(|e| {
                        let mut e = e.clone();
                        if e.get("show").is_none_or(Value::is_null) {
                            e["show"] = show.clone();
                        }
                        e
                    })
                    .collect();
                let rows = self.rows_plain(&eps);
                p.blocks.push(Block::Rows { id: "episodes".into(), title: "All Episodes".into(), rows, style: RowsStyle::Episodes, context: String::new(), playlist: None, more: None, collapsed_count: 0 });
            }
            None => p.state = self.wait_state(&eps_path),
        }
    }

    fn episode(&mut self, p: &mut PageModel, id: &str) {
        let path = format!("/episodes/{id}");
        let Some(e) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let uri = s(&e, "uri").to_string();
        let mut h = Self::header(&uri, "Podcast Episode", s(&e, "name"), model::image(&e, 640), ArtStyle::Square);
        if let Some(show) = e.get("show").filter(|v| !v.is_null()) {
            h.links = vec![link_of(show)];
        }
        h.meta = format!("{} • {}", model::fmt_date(s(&e, "release_date")), model::fmt_long(model::u(&e, "duration_ms")));
        h.save = SaveKind::Like;
        h.can_shuffle = false;
        p.header = Some(h);
        p.blocks.push(Block::Text { title: "Episode Description".into(), body: s(&e, "description").to_string() });
        // The row the episode's own menu acts on.
        let row = self.row(&e, "");
        p.blocks.push(Block::Rows { id: "self".into(), title: String::new(), rows: vec![row], style: RowsStyle::Episodes, context: String::new(), playlist: None, more: None, collapsed_count: 0 });
    }

    fn audiobook(&mut self, p: &mut PageModel, id: &str) {
        let path = format!("/audiobooks/{id}");
        let Some(b) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let uri = s(&b, "uri").to_string();
        let mut h = Self::header(&uri, "Audiobook", s(&b, "name"), model::image(&b, 640), ArtStyle::Square);
        h.links = model::arr(&b, "authors").iter().map(|a| Link { name: s(a, "name").to_string(), uri: String::new() }).collect();
        let narrators: Vec<&str> = model::arr(&b, "narrators").iter().map(|n| s(n, "name")).collect();
        if !narrators.is_empty() {
            h.meta = format!("Narrated by {}", narrators.join(", "));
        }
        h.save = SaveKind::Library;
        p.header = Some(h);
        p.blocks.push(Block::Text { title: "Description".into(), body: model::plain(s(&b, "html_description").if_empty(s(&b, "description"))) });
        let ch_path = format!("/audiobooks/{id}/chapters?limit=50");
        match self.want_all(&ch_path, 2000) {
            Some(v) => {
                let rows = self.rows_plain(model::items(&v));
                p.blocks.push(Block::Rows { id: "chapters".into(), title: "Chapters".into(), rows, style: RowsStyle::Chapters, context: uri, playlist: None, more: None, collapsed_count: 0 });
            }
            None => p.state = self.wait_state(&ch_path),
        }
    }

    // --------------------------------------------------------------- Track

    fn track(&mut self, p: &mut PageModel, id: &str) {
        let path = format!("/tracks/{id}");
        let Some(t) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let album_uri = t.get("album").map(|a| s(a, "uri").to_string()).unwrap_or_default();
        let mut h = Self::header(&album_uri, "Song", s(&t, "name"), model::image(&t, 640), ArtStyle::Square);
        h.links = artist_links(&t);
        if let Some(a) = t.get("album").filter(|a| !a.is_null()) {
            h.links.push(link_of(a));
            h.meta = format!("{} • {} • Popularity {}", model::year(s(a, "release_date")), model::fmt_ms(model::u(&t, "duration_ms")), model::u(&t, "popularity"));
        }
        h.save = SaveKind::Like;
        h.can_shuffle = false;
        h.can_play = true;
        p.header = Some(h);
        let row = self.row(&t, "");
        p.blocks.push(Block::Rows { id: "self".into(), title: String::new(), rows: vec![row], style: RowsStyle::Plain, context: album_uri, playlist: None, more: None, collapsed_count: 0 });
        let artists = model::arr(&t, "artists").to_vec();
        p.blocks.push(Block::Links { title: "Artist".into(), links: artists.iter().map(link_of).collect() });
        if let Some(a) = artists.first() {
            let aid = s(a, "id").to_string();
            if let Some(v) = self.want(&format!("/artists/{aid}/top-tracks?market=from_token")) {
                let tracks = model::arr(&v, "tracks").to_vec();
                if !tracks.is_empty() {
                    let rows = self.rows_plain(&tracks[..tracks.len().min(5)]);
                    p.blocks.push(Block::Rows {
                        id: "popular".into(),
                        title: format!("Popular songs by {}", s(a, "name")),
                        rows,
                        style: RowsStyle::Popular,
                        context: String::new(),
                        playlist: None,
                        more: None,
                        collapsed_count: 0,
                    });
                }
            }
            if let Some(v) = self.want(&format!("/artists/{aid}/albums?include_groups=album,single&limit=12")) {
                p.blocks.push(Block::Cards { id: "releases".into(), title: format!("Releases by {}", s(a, "name")), cards: cards(model::items(&v)), style: CardsStyle::Shelf, more: Some(Route::Discography { id: aid }) });
            }
        }
    }

    // ------------------------------------------------------------- Profile

    fn profile(&mut self, p: &mut PageModel, id: &str) {
        let me_id = self.user_id();
        let is_me = id.is_empty() || id == me_id;
        let path = if is_me { "/me".to_string() } else { format!("/users/{}", crate::api::enc(id)) };
        let Some(u) = self.want(&path) else {
            p.state = self.wait_state(&path);
            return;
        };
        let lists = if is_me { self.want_all("/me/playlists?limit=50", 2000) } else { self.want_all(&format!("/users/{}/playlists?limit=50", crate::api::enc(id)), 500) };
        let playlists: Vec<Value> =
            lists.as_deref().map(|v| model::items(v).iter().filter(|p| !is_me || (p["owner"]["id"].as_str() == Some(me_id.as_str()) && model::b(p, "public"))).cloned().collect()).unwrap_or_default();
        let follows = if is_me { self.want_all("/me/following?type=artist&limit=50", 2000) } else { None };
        let following: Vec<Value> = follows.as_deref().map(|v| model::items(v).to_vec()).unwrap_or_default();
        let uri = s(&u, "uri").to_string();
        let mut h = Self::header("", "Profile", s(&u, "display_name"), model::image(&u, 300), ArtStyle::Round);
        h.uri = uri;
        h.can_play = false;
        h.can_shuffle = false;
        let mut meta = vec![format!("{} Public Playlists", playlists.len()), format!("{} Followers", model::fmt_count(model::u(&u["followers"], "total")))];
        if is_me {
            meta.push(format!("{} Following", following.len()));
        }
        h.meta = meta.join(" • ");
        h.editable = is_me;
        p.header = Some(h);
        if is_me {
            if let Some(v) = self.want("/me/top/artists?limit=10&time_range=short_term")
                && !model::items(&v).is_empty()
            {
                p.blocks.push(Block::Cards { id: "top-artists".into(), title: "Top artists this month".into(), cards: cards(model::items(&v)), style: CardsStyle::Shelf, more: None });
            }
            if let Some(v) = self.want("/me/top/tracks?limit=10&time_range=short_term")
                && !model::items(&v).is_empty()
            {
                let items = model::items(&v);
                let rows = self.rows_plain(&items[..items.len().min(4)]);
                p.blocks.push(Block::Rows { id: "top-songs".into(), title: "Top songs this month".into(), rows, style: RowsStyle::Popular, context: String::new(), playlist: None, more: None, collapsed_count: 0 });
            }
        }
        if !playlists.is_empty() {
            p.blocks.push(Block::Cards { id: "playlists".into(), title: "Public Playlists".into(), cards: cards(&playlists), style: CardsStyle::Grid, more: None });
        }
        if !following.is_empty() {
            p.blocks.push(Block::Cards { id: "following".into(), title: "Following".into(), cards: cards(&following), style: CardsStyle::Shelf, more: None });
        }
    }

    // --------------------------------------------------- Liked, episodes, recents

    fn liked(&mut self, p: &mut PageModel) {
        let uid = self.user_id();
        let items = self.want_all("/me/tracks?limit=50", 10_000);
        let n = items.as_ref().map(|v| model::u(v, "total")).unwrap_or(0);
        let name = self.me.as_ref().map(|m| s(m, "display_name").to_string()).unwrap_or_default();
        let ctx = format!("spotify:user:{uid}:collection");
        let mut h = Self::header(&ctx, "collection", "Liked Songs", None, ArtStyle::LikedSongs);
        h.links = vec![Link { name, uri: format!("spotify:user:{uid}") }];
        h.meta = model::songs(n);
        h.can_download = true;
        p.header = Some(h);
        let Some(v) = items else {
            p.state = self.wait_state("/me/tracks?limit=50");
            return;
        };
        let rows = self.rows_from_items(model::items(&v));
        if rows.is_empty() {
            p.blocks.push(Block::Empty { title: "songs you like will appear here".into(), body: "Tap the heart on any song to save it here.".into() });
            return;
        }
        p.blocks.push(Block::Rows { id: "tracks".into(), title: String::new(), rows, style: RowsStyle::Table, context: ctx, playlist: None, more: None, collapsed_count: 0 });
        let first = &model::items(&v)[..model::items(&v).len().min(50)];
        if let Some(b) = self.featured(first) {
            p.blocks.push(b);
        }
    }

    fn your_episodes(&mut self, p: &mut PageModel) {
        let items = self.want_all("/me/episodes?limit=50", 2000);
        let n = items.as_ref().map(|v| model::u(v, "total")).unwrap_or(0);
        let uid = self.user_id();
        let mut h = Self::header(&format!("spotify:user:{uid}:collection:your-episodes"), "collection", "Your Episodes", None, ArtStyle::YourEpisodes);
        h.meta = if n == 1 { "1 episode".to_string() } else { format!("{n} episodes") };
        h.can_shuffle = false;
        h.can_download = true;
        p.header = Some(h);
        let Some(v) = items else {
            p.state = self.wait_state("/me/episodes?limit=50");
            return;
        };
        let eps: Vec<Value> = model::items(&v).iter().map(|i| i["episode"].clone()).filter(|e| !e.is_null()).collect();
        for e in &eps {
            let u = s(e, "uri").to_string();
            if !self.saved.contains_key(&u) {
                self.saved.insert(u, true);
            }
        }
        if eps.is_empty() {
            p.blocks.push(Block::Empty { title: "add to Your Episodes".into(), body: "Tap + on an episode to save it here.".into() });
            return;
        }
        let rows = self.rows_plain(&eps);
        p.blocks.push(Block::Rows { id: "episodes".into(), title: String::new(), rows, style: RowsStyle::Episodes, context: String::new(), playlist: None, more: None, collapsed_count: 0 });
    }

    fn recents(&mut self, p: &mut PageModel) {
        let mut h = Self::header("", "", "Recents", None, ArtStyle::Square);
        h.description = "The last 50 songs you played, from Spotify's listening history.".into();
        p.header = Some(h);
        let path = "/me/player/recently-played?limit=50";
        let Some(v) = self.want(path) else {
            p.state = self.wait_state(path);
            return;
        };
        let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
        for it in model::items(&v) {
            let day = model::fmt_date(s(it, "played_at"));
            match groups.last_mut() {
                Some((d, rows)) if *d == day => rows.push(it["track"].clone()),
                _ => groups.push((day, vec![it["track"].clone()])),
            }
        }
        for (i, (day, items)) in groups.into_iter().enumerate() {
            let rows = self.rows_plain(&items);
            p.blocks.push(Block::Rows { id: format!("day{i}"), title: day, rows, style: RowsStyle::Plain, context: String::new(), playlist: None, more: None, collapsed_count: 0 });
        }
    }

    fn folder(&mut self, p: &mut PageModel, id: &str) {
        let Some(f) = self.settings.folders.iter().find(|f| f.id == id).cloned() else {
            p.state = LoadState::Missing { title: "folder not found".into(), body: "It may have been deleted.".into() };
            return;
        };
        let mut h = Self::header("", "Folder", &f.name, None, ArtStyle::Folder);
        let n = f.playlists.len();
        h.meta = if n == 1 { "1 playlist".to_string() } else { format!("{n} playlists") };
        h.can_play = false;
        h.can_shuffle = false;
        p.header = Some(h);
        let mine = self.want_all("/me/playlists?limit=50", 2000);
        let items: Vec<Value> = f.playlists.iter().filter_map(|u| mine.as_ref().and_then(|m| model::items(m).iter().find(|p| s(p, "uri") == u).cloned())).collect();
        if items.is_empty() {
            p.blocks.push(Block::Empty { title: "this folder looks empty".into(), body: "Add playlists to it with Move to folder in a playlist's menu.".into() });
            return;
        }
        p.blocks.push(Block::Cards { id: "playlists".into(), title: String::new(), cards: cards(&items), style: CardsStyle::Grid, more: None });
    }

    fn local_playlist(&mut self, p: &mut PageModel, id: &str) {
        let Some(pl) = self.playlists.get(id).cloned() else {
            p.state = if self.playlists.just_deleted(id) {
                LoadState::Missing { title: "playlist deleted".into(), body: "Undo brings it back, songs and order included.".into() }
            } else {
                LoadState::Missing { title: "playlist not found".into(), body: "It may have been deleted.".into() }
            };
            return;
        };
        let all = self.local_tracks.clone().unwrap_or_default();
        let by_path: std::collections::HashMap<&std::path::Path, &crate::local::LocalTrack> = all.iter().map(|t| (t.path.as_path(), t)).collect();
        let mut rows = Vec::new();
        let mut total = 0;
        let mut missing = 0;
        for path in &pl.tracks {
            match by_path.get(path.as_path()) {
                Some(t) => {
                    total += t.duration_ms;
                    rows.push(local_row(t));
                }
                None => {
                    missing += 1;
                    let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                    rows.push(Row {
                        uri: String::new(),
                        kind: RowKind::LocalFile,
                        title: format!("file missing: {name}"),
                        artists: Vec::new(),
                        album: None,
                        image: None,
                        duration_ms: 0,
                        explicit: false,
                        playable: false,
                        note: Some("This file isn't in Local Files any more.".into()),
                        added_at: String::new(),
                        track_number: 0,
                        disc_number: 1,
                        path: Some(path.to_string_lossy().into_owned()),
                        release_date: String::new(),
                        description: String::new(),
                        resume_ms: 0,
                        played: false,
                        parent: None,
                    });
                }
            }
        }
        let cover = pl.cover.as_deref().filter(|c| c.exists()).and_then(|c| url::Url::from_file_path(c).ok()).map(|u| u.to_string());
        let mut h = Self::header("", "local playlist", &pl.name, cover, ArtStyle::LocalFiles);
        h.description = pl.description.clone();
        let mut meta = format!("{} · {} · files on this device", model::songs((rows.len() - missing) as u64), model::fmt_ms(total));
        if missing > 0 {
            meta.push_str(&format!(" · {missing} missing"));
        }
        h.meta = meta;
        h.can_shuffle = false;
        h.editable = true;
        p.header = Some(h);
        p.blocks.push(Block::Rows { id: "tracks".into(), title: String::new(), rows, style: RowsStyle::Table, context: String::new(), playlist: Some(id.to_string()), more: None, collapsed_count: 0 });
    }
}

pub fn local_row(t: &crate::local::LocalTrack) -> Row {
    Row {
        uri: crate::local::spotify_uri(t),
        kind: RowKind::LocalFile,
        title: t.title.clone(),
        artists: if t.artist.is_empty() { Vec::new() } else { vec![Link { name: t.artist.clone(), uri: String::new() }] },
        album: (!t.album.is_empty()).then(|| Link { name: t.album.clone(), uri: String::new() }),
        image: t.cover_uri(),
        duration_ms: t.duration_ms,
        explicit: false,
        playable: true,
        note: None,
        added_at: String::new(),
        track_number: t.track_no,
        disc_number: t.disc_no.max(1),
        path: Some(t.path.to_string_lossy().into_owned()),
        release_date: t.year.map(|y| y.to_string()).unwrap_or_default(),
        description: String::new(),
        resume_ms: 0,
        played: false,
        parent: None,
    }
}

/// A path as the user knows it: inside the app's Documents, from there.
pub fn display_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    match s.find("/Documents/") {
        Some(i) => format!("On My iPhone › Glass › {}", &s[i + "/Documents/".len()..].replace('/', " › ")),
        None => s.into_owned(),
    }
}

fn no_results(q: &str) -> LoadState {
    LoadState::Missing { title: format!("No results found for “{q}”"), body: "Check the spelling, or try fewer or different words.".into() }
}
