//! Local Files, organized the way iTunes (Music) does it: albums as a grid of
//! covers, songs in a sortable table, artists beside their albums, genres,
//! and what was added recently. An album lists its songs in track order.

use super::icons::{self, Icon};
use super::menus;
use super::theme::{self, col, px, sp};
use super::widgets::{self, paint_cover};
use crate::app::{Action, App, Route};
use crate::config::LocalView;
use crate::local::LocalTrack;
use eframe::egui::{self, Align2, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

// ------------------------------------------------------------------ index

pub use crate::local_index::{Album, Index, album_artist, album_title, index};
use crate::local_index::{album_key, sort_key};

fn playing_album(app: &App) -> Option<String> {
    let t = app.local.as_ref()?.current()?;
    Some(album_key(album_artist(t), album_title(t)))
}

/// Which library a cached result belongs to. Holding a `Weak` keeps that
/// library's allocation reserved, so a newer library can't reuse its address
/// and be mistaken for it.
type Of = std::sync::Weak<Vec<LocalTrack>>;
type SongsOrder = (Of, SongSort, bool, String, Rc<Vec<usize>>);

fn same(of: &Of, all: &Arc<Vec<LocalTrack>>) -> bool {
    std::ptr::eq(of.as_ptr(), Arc::as_ptr(all))
}

thread_local! {
    /// The index of the library last shown (rebuilt when Local Files changes).
    static CACHE: RefCell<Option<(Of, Rc<Index>)>> = const { RefCell::new(None) };
    /// The Songs view's order, by (library, sort, filter).
    static ORDER: RefCell<Option<SongsOrder>> = const { RefCell::new(None) };
}

fn cached(all: &Arc<Vec<LocalTrack>>) -> Rc<Index> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        match &*c {
            Some((of, idx)) if same(of, all) => idx.clone(),
            _ => {
                let idx = Rc::new(index(all));
                *c = Some((Arc::downgrade(all), idx.clone()));
                idx
            }
        }
    })
}

// ------------------------------------------------------------------ songs order

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SongSort {
    /// Artist, then album, then track: how iTunes lists songs.
    #[default]
    Artist,
    Title,
    Album,
    Genre,
    Year,
    Time,
}

fn song_matches(t: &LocalTrack, q: &str) -> bool {
    q.is_empty() || t.title.to_lowercase().contains(q) || t.artist.to_lowercase().contains(q) || t.album.to_lowercase().contains(q) || t.genre.to_lowercase().contains(q)
}

fn songs_order(all: &Arc<Vec<LocalTrack>>, idx: &Index, sort: SongSort, desc: bool, q: &str) -> Rc<Vec<usize>> {
    if let Some(o) = ORDER.with(|o| o.borrow().as_ref().filter(|(of, s, d, f, _)| same(of, all) && *s == sort && *d == desc && f == q).map(|x| x.4.clone())) {
        return o;
    }
    // Ties keep the artist / album / track order.
    let mut order: Vec<usize> = idx.songs.iter().copied().filter(|&i| song_matches(&all[i], q)).collect();
    let rank: HashMap<usize, usize> = idx.songs.iter().enumerate().map(|(r, &i)| (i, r)).collect();
    match sort {
        SongSort::Artist => {}
        SongSort::Title => order.sort_by_cached_key(|&i| sort_key(&all[i].title)),
        SongSort::Album => order.sort_by_cached_key(|&i| (sort_key(&all[i].album), rank[&i])),
        SongSort::Genre => order.sort_by_cached_key(|&i| (all[i].genre.is_empty(), sort_key(&all[i].genre), rank[&i])),
        SongSort::Year => order.sort_by_key(|&i| (all[i].year.is_none(), all[i].year, rank[&i])),
        SongSort::Time => order.sort_by_key(|&i| all[i].duration_ms),
    }
    if desc {
        order.reverse();
    }
    let order = Rc::new(order);
    ORDER.with(|o| *o.borrow_mut() = Some((Arc::downgrade(all), sort, desc, q.to_string(), order.clone())));
    order
}

// ------------------------------------------------------------------ the page

pub fn show(app: &mut App, ui: &mut Ui) {
    let all = app.local_tracks.clone();
    let idx = all.as_ref().map(cached);
    header(app, ui, all.as_ref(), idx.as_deref());
    if !app.settings.show_local_files {
        widgets::unavailable(ui, "local files are turned off", "Turn on \"Show Local Files\" in Settings.");
        return;
    }
    let (Some(all), Some(idx)) = (all, idx) else {
        widgets::loading(ui);
        return;
    };
    tabs(app, ui);
    ui.add_space(sp(1.5));
    let q = app.page_filter.trim().to_lowercase();
    if all.is_empty() && app.settings.local_view != LocalView::Playlists {
        widgets::unavailable(ui, "listen to local files", "Add a folder of music with \"Add a source\", or drop songs onto this window.");
        return;
    }
    match app.settings.local_view {
        LocalView::Albums => {
            let list: Vec<usize> = (0..idx.albums.len()).filter(|&a| idx.albums[a].matches(&q)).collect();
            album_grid(app, ui, &all, &idx, &list);
        }
        LocalView::Recent => {
            let mut list: Vec<usize> = (0..idx.albums.len()).filter(|&a| idx.albums[a].matches(&q)).collect();
            list.sort_by_key(|&a| std::cmp::Reverse(idx.albums[a].added));
            album_grid(app, ui, &all, &idx, &list);
        }
        LocalView::Songs => songs(app, ui, &all, &idx, &q),
        LocalView::Artists => artists(app, ui, &all, &idx, &q),
        LocalView::Genres => genres(app, ui, &all, &idx, &q),
        LocalView::Playlists => super::pages::local_playlists_list(app, ui),
    }
}

fn header(app: &mut App, ui: &mut Ui, all: Option<&Arc<Vec<LocalTrack>>>, idx: Option<&Index>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Local Files").font(theme::regular(30.0)).color(col::text()));
        if let (Some(all), Some(idx)) = (all, idx) {
            let mins = all.iter().map(|t| t.duration_ms).sum::<u64>() / 60_000;
            let length = if mins >= 24 * 60 {
                format!("{:.1} days", mins as f32 / (24.0 * 60.0))
            } else if mins >= 60 {
                format!("{:.1} hours", mins as f32 / 60.0)
            } else {
                format!("{mins} minute{}", if mins == 1 { "" } else { "s" })
            };
            let count = |n: usize, one: &str, many: &str| format!("{} {}", crate::model::fmt_count(n as u64), if n == 1 { one } else { many });
            ui.add_space(sp(1.5));
            ui.label(RichText::new(format!("{} · {} · {}", count(all.len(), "song", "songs"), count(idx.albums.len(), "album", "albums"), length)).font(theme::regular(13.0)).color(col::muted()));
        }
    });
    ui.add_space(sp(1.0));
    ui.horizontal(|ui| {
        let playing = app.local.as_ref().is_some_and(|l| l.is_playing());
        let active = app.local.as_ref().is_some_and(|l| l.active);
        // The playlists view has no one list to play (each playlist plays from its page).
        if (active || app.settings.local_view != LocalView::Playlists) && widgets::play_btn(ui, playing, 44.0).on_hover_text("Play the list shown").clicked() {
            if app.local.as_ref().is_some_and(|l| l.active) {
                app.act(Action::TogglePlay);
            } else if let (Some(idx), Some(all)) = (idx, all) {
                let order = shown_songs(app, all, idx);
                if !order.is_empty() {
                    app.act(Action::PlayLocalTracks { indices: order, start: 0 });
                }
            }
        }
        if widgets::outline_button(ui, "Add a source").clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            if crate::local::add_source(&mut app.settings.local_dirs, dir.clone()) {
                app.mark_settings_dirty();
                app.act(Action::RescanLocal);
                app.toast(format!("Now showing songs from {}", dir.display()));
            } else {
                app.toast("That folder is already in Local Files");
            }
        }
        if widgets::outline_button(ui, "Rescan").clicked() {
            app.act(Action::RescanLocal);
        }
    });
    ui.add_space(sp(0.5));
    let srcs: Vec<String> = app.settings.local_dirs.iter().map(|d| d.display().to_string()).collect();
    widgets::subtle(ui, format!("Showing songs from: {}", if srcs.is_empty() { "no folders".into() } else { srcs.join(", ") }));
    ui.add_space(sp(1.0));
    super::pages::devices(app, ui);
}

/// The songs of what's shown, in order (for the play button).
fn shown_songs(app: &App, all: &Arc<Vec<LocalTrack>>, idx: &Index) -> Vec<usize> {
    let q = app.page_filter.trim().to_lowercase();
    match app.settings.local_view {
        LocalView::Recent => {
            let mut list: Vec<&Album> = idx.albums.iter().filter(|a| a.matches(&q)).collect();
            list.sort_by_key(|a| std::cmp::Reverse(a.added));
            list.iter().flat_map(|a| a.tracks.iter().copied()).collect()
        }
        // In the table's sort.
        LocalView::Songs => songs_order(all, idx, app.local_sort.0, app.local_sort.1, &q).to_vec(),
        // The artist or genre on screen, chosen as `artists` and `genres` choose it.
        LocalView::Artists => {
            let list: Vec<usize> = (0..idx.artists.len()).filter(|&a| q.is_empty() || idx.artists[a].0.to_lowercase().contains(&q)).collect();
            let Some(&first) = list.first() else { return Vec::new() };
            let chosen = app.local_artist.as_ref().and_then(|n| list.iter().copied().find(|&a| idx.artists[a].0.eq_ignore_ascii_case(n))).unwrap_or(first);
            idx.artists[chosen].1.iter().flat_map(|&a| idx.albums[a].tracks.iter().copied()).collect()
        }
        LocalView::Genres => {
            let chosen = app.local_genre.as_ref().and_then(|g| idx.genres.iter().position(|(n, _)| n.eq_ignore_ascii_case(g))).unwrap_or(0);
            idx.genres.get(chosen).map(|(_, albums)| albums.iter().filter(|&&a| idx.albums[a].matches(&q)).flat_map(|&a| idx.albums[a].tracks.iter().copied()).collect()).unwrap_or_default()
        }
        LocalView::Playlists => Vec::new(),
        LocalView::Albums => idx.albums.iter().filter(|a| a.matches(&q)).flat_map(|a| a.tracks.iter().copied()).collect(),
    }
}

fn tabs(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for (v, name) in [
            (LocalView::Albums, "Albums"),
            (LocalView::Songs, "Songs"),
            (LocalView::Artists, "Artists"),
            (LocalView::Genres, "Genres"),
            (LocalView::Recent, "Recently Added"),
            (LocalView::Playlists, "Playlists"),
        ] {
            if widgets::chip(ui, name, app.settings.local_view == v) && app.settings.local_view != v {
                app.settings.local_view = v;
                app.row_cursor = None;
                app.mark_settings_dirty();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let te = egui::TextEdit::singleline(&mut app.page_filter).hint_text(RichText::new("filter").color(col::muted())).font(theme::regular(12.5)).desired_width(px(180.0));
            let r = ui.add(te);
            if app.focus_search && app.route != Route::Search {
                r.request_focus();
                app.focus_search = false;
            }
        });
    });
}

// ------------------------------------------------------------------ albums

fn album_grid(app: &mut App, ui: &mut Ui, all: &Arc<Vec<LocalTrack>>, idx: &Index, list: &[usize]) {
    if list.is_empty() {
        widgets::subtle(ui, "Nothing matches the filter.");
        return;
    }
    let playing = playing_album(app);
    widgets::card_grid(ui, list.len(), false, |ui, i, w| {
        let al = &idx.albums[list[i]];
        let here = playing.as_ref() == Some(&al.key);
        let cover = al.cover_uri(all);
        let r = widgets::card(ui, cover.as_deref(), &al.title, &al.artist, false, w, here);
        if r.play {
            app.act(Action::PlayLocalTracks { indices: al.tracks.clone(), start: 0 });
        } else if r.open {
            app.act(Action::Navigate(Route::LocalAlbum { artist: al.artist.clone(), album: al.title.clone() }));
        }
        r.response.context_menu(|ui| album_menu(app, ui, all, al));
    });
}

fn album_menu(app: &mut App, ui: &mut Ui, all: &[LocalTrack], al: &Album) {
    if widgets::menu_item(ui, "Play") {
        app.act(Action::PlayLocalTracks { indices: al.tracks.clone(), start: 0 });
    }
    let tracks: Vec<LocalTrack> = al.tracks.iter().map(|&i| all[i].clone()).collect();
    menus::add_local_to_playlist(app, ui, &tracks);
    menus::send_to_device(app, ui, &tracks);
    if widgets::menu_item(ui, "Go to album") {
        app.act(Action::Navigate(Route::LocalAlbum { artist: al.artist.clone(), album: al.title.clone() }));
    }
}

/// One album: its cover and details, then its songs in track order (with a
/// heading per disc).
pub fn album_page(app: &mut App, ui: &mut Ui, artist: &str, album: &str) {
    let Some(all) = app.local_tracks.clone() else {
        widgets::loading(ui);
        return;
    };
    let idx = cached(&all);
    let key = album_key(artist, album);
    let Some(al) = idx.albums.iter().find(|a| a.key == key) else {
        widgets::unavailable(ui, "this album isn't in Local Files any more", "Its files may have moved; try Rescan in Local Files.");
        return;
    };
    let cover = al.cover_uri(&all);
    let mut meta = vec![al.artist.clone()];
    if !al.genre.is_empty() {
        meta.push(al.genre.clone());
    }
    if let Some(y) = al.year {
        meta.push(y.to_string());
    }
    meta.push(format!("{} {}, {}", al.tracks.len(), if al.tracks.len() == 1 { "song" } else { "songs" }, crate::model::fmt_ms(al.duration_ms)));
    widgets::entity_header(ui, cover.as_deref(), false, "album", &al.title, |ui| {
        ui.label(RichText::new(meta.join(" · ")).font(theme::regular(13.0)).color(col::sub()));
    });
    ui.add_space(sp(1.25));
    ui.horizontal(|ui| {
        let playing = app.local.as_ref().is_some_and(|l| l.is_playing()) && playing_album(app).as_ref() == Some(&al.key);
        if widgets::play_btn(ui, playing, 44.0).clicked() {
            if playing { app.act(Action::TogglePlay) } else { app.act(Action::PlayLocalTracks { indices: al.tracks.clone(), start: 0 }) }
        }
        let more = widgets::icon_btn(ui, Icon::More, 18.0, "More options", false);
        egui::Popup::menu(&more).show(|ui| album_menu(app, ui, &all, al));
    });
    ui.add_space(sp(1.0));
    // Several artists on one album (a compilation): show who sings each.
    let various = al.tracks.iter().any(|&i| all[i].artist.trim() != al.tracks.first().map(|&f| all[f].artist.trim()).unwrap_or(""));
    let discs: Vec<u32> = {
        let mut d: Vec<u32> = al.tracks.iter().map(|&i| all[i].disc_no.max(1)).collect();
        d.dedup();
        d
    };
    for d in &discs {
        if discs.len() > 1 {
            ui.add_space(sp(0.75));
            ui.label(RichText::new(format!("Disc {d}")).font(theme::medium(13.0)).color(col::sub()));
        }
        let order: Vec<usize> = al.tracks.iter().copied().filter(|&i| all[i].disc_no.max(1) == *d).collect();
        song_rows(app, ui, &all, &order, &al.tracks, RowOpts { number: Number::Track, artist: various, album: false, extra: false, keys: discs.len() == 1, salt: d.to_string() });
    }
}

// ------------------------------------------------------------------ songs

fn songs(app: &mut App, ui: &mut Ui, all: &Arc<Vec<LocalTrack>>, idx: &Index, q: &str) {
    let (sort, desc) = app.local_sort;
    let order = songs_order(all, idx, sort, desc, q);
    // Column headers sort, as in iTunes; again flips the direction.
    let cols = Cols::new(ui.available_width(), true, true, true);
    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let mut head = |x: f32, text: &str, s: SongSort| {
        let r = Rect::from_min_size(pos2(hr.left() + x, hr.top()), vec2(110.0, hr.height()));
        let resp = ui.interact(r, ui.id().with(("local-sort", text)), Sense::click()).on_hover_text(format!("Sort by {}", text.to_lowercase()));
        let on = app.local_sort.0 == s;
        if resp.clicked() {
            app.local_sort = if on { (s, !desc) } else { (s, false) };
        }
        let arrow = if on { if desc { " ▾" } else { " ▴" } } else { "" };
        widgets::caps(ui, pos2(r.left(), r.center().y), &format!("{text}{arrow}"), 11.0, if on { col::accent() } else { col::muted() });
    };
    head(cols.title, "Title", SongSort::Title);
    head(cols.artist, "Artist", SongSort::Artist);
    if cols.show_album {
        head(cols.album, "Album", SongSort::Album);
    }
    if cols.show_extra {
        head(cols.genre, "Genre", SongSort::Genre);
        head(cols.year, "Year", SongSort::Year);
    }
    head(cols.time, "Time", SongSort::Time);
    widgets::hairline(ui);
    if order.is_empty() {
        widgets::subtle(ui, "Nothing matches the filter.");
        return;
    }
    if app.select_all_pending {
        app.select_all_pending = false;
        app.local_selection = order.iter().map(|&i| all[i].path.clone()).collect();
    }
    song_rows(app, ui, all, &order, &order, RowOpts { number: Number::Row, artist: true, album: true, extra: true, keys: true, salt: "songs".into() });
}

// ------------------------------------------------------------------ artists

fn artists(app: &mut App, ui: &mut Ui, all: &Arc<Vec<LocalTrack>>, idx: &Index, q: &str) {
    let list: Vec<usize> = (0..idx.artists.len()).filter(|&a| q.is_empty() || idx.artists[a].0.to_lowercase().contains(q)).collect();
    if list.is_empty() {
        widgets::subtle(ui, "Nothing matches the filter.");
        return;
    }
    let chosen = app.local_artist.as_ref().and_then(|n| list.iter().copied().find(|&a| idx.artists[a].0.eq_ignore_ascii_case(n))).unwrap_or(list[0]);
    let height = (ui.clip_rect().bottom() - ui.cursor().top() - sp(1.0)).max(px(320.0));
    ui.horizontal_top(|ui| {
        // The artists, on the left, scrolling on their own.
        ui.vertical(|ui| {
            ui.set_width(px(240.0));
            // Only the rows in view are drawn (a big library has thousands of artists).
            ScrollArea::vertical().id_salt("local-artists").max_height(height).auto_shrink([false, true]).show_rows(ui, px(40.0), list.len(), |ui, range| {
                for &a in &list[range] {
                    let (name, albums) = &idx.artists[a];
                    let songs: usize = albums.iter().map(|&x| idx.albums[x].tracks.len()).sum();
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), px(40.0)), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, a == chosen, name));
                    if a == chosen {
                        ui.painter().rect_filled(rect, theme::small_radius(), col::selection());
                    } else if ui.rect_contains_pointer(rect) {
                        ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
                    }
                    widgets::text_trunc(ui, pos2(rect.left() + 12.0, rect.center().y - px(7.0)), name, theme::regular(13.5), col::text(), rect.width() - 24.0);
                    widgets::text_trunc(
                        ui,
                        pos2(rect.left() + 12.0, rect.center().y + px(9.0)),
                        &format!("{} {} · {songs} {}", albums.len(), if albums.len() == 1 { "album" } else { "albums" }, if songs == 1 { "song" } else { "songs" }),
                        theme::regular(11.5),
                        col::muted(),
                        rect.width() - 24.0,
                    );
                    if resp.clicked() {
                        app.local_artist = Some(name.clone());
                    }
                }
            });
        });
        ui.add_space(sp(2.0));
        // Their albums, each with its songs.
        ui.vertical(|ui| {
            let (name, albums) = &idx.artists[chosen];
            ui.label(RichText::new(name).font(theme::regular(24.0)).color(col::text()));
            ui.add_space(sp(1.0));
            for &x in albums {
                album_section(app, ui, all, &idx.albums[x], x);
                ui.add_space(sp(2.0));
            }
        });
    });
}

/// An album inside the Artists view: small cover and details, then songs.
fn album_section(app: &mut App, ui: &mut Ui, all: &Arc<Vec<LocalTrack>>, al: &Album, key: usize) {
    ui.horizontal_top(|ui| {
        let (art, resp) = ui.allocate_exact_size(vec2(px(96.0), px(96.0)), Sense::click());
        paint_cover(ui, art, al.cover_uri(all).as_deref(), false);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.act(Action::Navigate(Route::LocalAlbum { artist: al.artist.clone(), album: al.title.clone() }));
        }
        ui.add_space(sp(1.5));
        ui.vertical(|ui| {
            ui.add_space(px(18.0));
            if ui.add(egui::Label::new(RichText::new(&al.title).font(theme::regular(17.0)).color(col::text())).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.act(Action::Navigate(Route::LocalAlbum { artist: al.artist.clone(), album: al.title.clone() }));
            }
            let mut meta = Vec::new();
            if !al.genre.is_empty() {
                meta.push(al.genre.clone());
            }
            if let Some(y) = al.year {
                meta.push(y.to_string());
            }
            ui.label(RichText::new(meta.join(" · ")).font(theme::regular(12.5)).color(col::muted()));
        });
    });
    ui.add_space(sp(0.5));
    song_rows(app, ui, all, &al.tracks, &al.tracks, RowOpts { number: Number::Track, artist: false, album: false, extra: false, keys: false, salt: format!("artist-album-{key}") });
}

// ------------------------------------------------------------------ genres

fn genres(app: &mut App, ui: &mut Ui, all: &Arc<Vec<LocalTrack>>, idx: &Index, q: &str) {
    let chosen = app.local_genre.as_ref().and_then(|g| idx.genres.iter().position(|(n, _)| n.eq_ignore_ascii_case(g))).unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        for (i, (name, albums)) in idx.genres.iter().enumerate() {
            if widgets::chip(ui, &format!("{name}  {}", albums.len()), i == chosen) {
                app.local_genre = Some(name.clone());
            }
        }
    });
    ui.add_space(sp(1.5));
    if let Some((_, albums)) = idx.genres.get(chosen) {
        let list: Vec<usize> = albums.iter().copied().filter(|&a| idx.albums[a].matches(q)).collect();
        album_grid(app, ui, all, idx, &list);
    }
}

// ------------------------------------------------------------------ song rows

#[derive(Clone, Copy, PartialEq)]
enum Number {
    /// The row's position in the list.
    Row,
    /// The song's track number on its album.
    Track,
}

struct RowOpts {
    number: Number,
    artist: bool,
    album: bool,
    /// Genre and year columns.
    extra: bool,
    /// j/k, Enter and E work on this list.
    keys: bool,
    salt: String,
}

/// Column positions (from the row's left edge) for a width.
struct Cols {
    title: f32,
    artist: f32,
    album: f32,
    genre: f32,
    year: f32,
    time: f32,
    show_album: bool,
    show_extra: bool,
}

impl Cols {
    fn new(w: f32, artist: bool, album: bool, extra: bool) -> Self {
        let show_album = album && w > 640.0;
        let show_extra = extra && w > 900.0;
        let time = w - 64.0;
        let (genre, year) = (w * 0.70, w * 0.83);
        let end = if show_extra { genre } else { time - 16.0 };
        let title = 48.0;
        let (artist_x, album_x) = match (artist, show_album) {
            (true, true) => (title + (end - title) * 0.42, title + (end - title) * 0.70),
            (true, false) => (title + (end - title) * 0.58, end),
            (false, true) => (end, title + (end - title) * 0.62),
            (false, false) => (end, end),
        };
        Self { title, artist: artist_x, album: album_x, genre, year, time, show_album, show_extra }
    }
}

/// Songs as rows. A click plays `play_list` from that song; ⌘-click and
/// ⇧-click select; right-click for more; songs drag onto local playlists.
fn song_rows(app: &mut App, ui: &mut Ui, all: &Arc<Vec<LocalTrack>>, order: &[usize], play_list: &[usize], o: RowOpts) {
    let cur = app.local.as_ref().filter(|l| l.active).and_then(|l| l.path_playing());
    let w = ui.available_width();
    let cols = Cols::new(w, o.artist, o.album, o.extra);
    let row_h = if app.compact_rows { 32.0 } else { 40.0 };
    // Keyboard: j/k move, Enter plays, E edits the song's info.
    let mut cursor = if o.keys { app.row_cursor.map(|c| c.min(order.len().saturating_sub(1))) } else { None };
    let mut moved = false;
    if o.keys && !order.is_empty() && !ui.ctx().egui_wants_keyboard_input() && !egui::Popup::is_any_open(ui.ctx()) && app.modal.is_none() {
        let none = egui::Modifiers::NONE;
        let (down, up, enter, edit) = ui.input_mut(|i| {
            (
                i.consume_key(none, egui::Key::J) || i.consume_key(none, egui::Key::ArrowDown),
                i.consume_key(none, egui::Key::K) || i.consume_key(none, egui::Key::ArrowUp),
                i.consume_key(none, egui::Key::Enter),
                i.consume_key(none, egui::Key::E),
            )
        });
        let last = order.len() - 1;
        if down || up {
            cursor = Some(match (cursor, down) {
                (None, _) => 0,
                (Some(c), true) => (c + 1).min(last),
                (Some(c), false) => c.saturating_sub(1),
            });
            moved = true;
        }
        if let Some(c) = cursor {
            if enter {
                play_from(app, play_list, order[c]);
            }
            if edit {
                app.act(Action::EditTags(all[order[c]].path.clone()));
            }
        }
        app.row_cursor = cursor;
    }
    for (pos, &i) in order.iter().enumerate() {
        let t = &all[i];
        let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &t.title));
        if moved && cursor == Some(pos) {
            ui.scroll_to_rect(rect, None);
        }
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let hovered = ui.rect_contains_pointer(rect);
        let selected = app.local_selection.contains(&t.path);
        if selected {
            ui.painter().rect_filled(rect, theme::small_radius(), col::selection());
        } else if hovered {
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
        } else if pos % 2 == 1 {
            // Alternate rows, as in iTunes, faintly.
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover().gamma_multiply(0.35));
        }
        if cursor == Some(pos) {
            ui.painter().rect_stroke(rect, theme::small_radius(), egui::Stroke::new(1.0, col::focus()), egui::StrokeKind::Inside);
        }
        let current = cur.as_deref() == Some(t.path.as_path());
        let y = rect.center().y;
        let x0 = rect.left();
        let number = match o.number {
            Number::Row => (pos + 1).to_string(),
            Number::Track if t.track_no > 0 => t.track_no.to_string(),
            Number::Track => String::new(),
        };
        let num_rect = Rect::from_center_size(pos2(x0 + 22.0, y), vec2(14.0, 14.0));
        if current && app.local.as_ref().is_some_and(|l| l.is_playing()) {
            widgets::equalizer_bars(ui, num_rect, col::accent());
        } else if hovered {
            icons::paint(ui.painter(), num_rect, Icon::Play, col::text());
        } else {
            ui.painter().text(pos2(x0 + 30.0, y), Align2::RIGHT_CENTER, number, theme::regular(12.5), col::muted());
        }
        let text_color = if current { col::accent() } else { col::text() };
        let mut tx = x0 + cols.title;
        if o.number == Number::Row {
            // Small artwork in the song list, as in Music.
            let c = Rect::from_min_size(pos2(tx, y - (row_h - 10.0) / 2.0), vec2(row_h - 10.0, row_h - 10.0));
            paint_cover(ui, c, t.cover_uri().as_deref(), false);
            tx = c.right() + 10.0;
        }
        let title_end = if o.artist {
            x0 + cols.artist
        } else if cols.show_album {
            x0 + cols.album
        } else {
            x0 + cols.time
        };
        widgets::text_trunc(ui, pos2(tx, y), &t.title, theme::regular(13.5), text_color, title_end - tx - 12.0);
        if o.artist {
            let end = if cols.show_album {
                cols.album
            } else if cols.show_extra {
                cols.genre
            } else {
                cols.time
            };
            widgets::text_trunc(ui, pos2(x0 + cols.artist, y), &t.artist, theme::regular(13.0), col::sub(), end - cols.artist - 12.0);
        }
        if cols.show_album {
            let end = if cols.show_extra { cols.genre } else { cols.time };
            widgets::text_trunc(ui, pos2(x0 + cols.album, y), &t.album, theme::regular(13.0), col::sub(), end - cols.album - 12.0);
        }
        if cols.show_extra {
            widgets::text_trunc(ui, pos2(x0 + cols.genre, y), &t.genre, theme::regular(12.5), col::muted(), cols.year - cols.genre - 12.0);
            if let Some(yr) = t.year {
                ui.painter().text(pos2(x0 + cols.year, y), Align2::LEFT_CENTER, yr.to_string(), theme::regular(12.5), col::muted());
            }
        }
        ui.painter().text(pos2(rect.right() - 12.0, y), Align2::RIGHT_CENTER, crate::model::fmt_ms(t.duration_ms), theme::regular(12.5), col::sub());
        // Edit, always registered (so the click isn't lost), shown on hover.
        let er = Rect::from_center_size(pos2(rect.right() - 72.0, y), vec2(15.0, 15.0));
        let edit = ui.interact(er.expand(6.0), resp.id.with(("edit", &o.salt)), Sense::click());
        if hovered {
            icons::paint(ui.painter(), er, Icon::Edit, if edit.hovered() { col::text() } else { col::sub() });
        }
        // Drag onto a local playlist (the selection, when this row is in it).
        if resp.drag_started() {
            let files: Vec<std::path::PathBuf> =
                if app.local_selection.len() > 1 && selected { order.iter().map(|&k| all[k].path.clone()).filter(|p| app.local_selection.contains(p)).collect() } else { vec![t.path.clone()] };
            let label = if files.len() > 1 { format!("{} songs", files.len()) } else { t.title.clone() };
            resp.dnd_set_drag_payload(super::SongDrag { files, label });
        }
        // E works where the rows take keys (j/k to move).
        if edit.clone().on_hover_text(if o.keys { "Edit song info  E" } else { "Edit song info" }).clicked() {
            app.act(Action::EditTags(t.path.clone()));
        } else if resp.clicked() {
            let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            if o.keys {
                app.row_cursor = Some(pos);
            }
            if let Some(anchor) = app.sel_anchor.filter(|_| shift) {
                let a = anchor.min(order.len() - 1);
                let (lo, hi) = if a < pos { (a, pos) } else { (pos, a) };
                app.local_selection = order[lo..=hi].iter().map(|&k| all[k].path.clone()).collect();
            } else if cmd {
                if !app.local_selection.remove(&t.path) {
                    app.local_selection.insert(t.path.clone());
                }
                app.sel_anchor = Some(pos);
            } else {
                app.local_selection = [t.path.clone()].into();
                app.sel_anchor = Some(pos);
                // The second click of a double-click would pause the song the first one started.
                if !resp.double_clicked() {
                    if current { app.act(Action::TogglePlay) } else { play_from(app, play_list, i) }
                }
            }
        }
        if resp.secondary_clicked() && !selected {
            app.local_selection = [t.path.clone()].into();
            app.sel_anchor = Some(pos);
        }
        let path = t.path.clone();
        resp.context_menu(|ui| {
            let picked: Vec<LocalTrack> = if app.local_selection.len() > 1 && app.local_selection.contains(&path) {
                order.iter().map(|&k| &all[k]).filter(|t| app.local_selection.contains(&t.path)).cloned().collect()
            } else {
                vec![all[i].clone()]
            };
            if picked.len() == 1 && widgets::menu_item(ui, "Play") {
                play_from(app, play_list, i);
            }
            menus::add_local_to_playlist(app, ui, &picked);
            menus::send_to_device(app, ui, &picked);
            if picked.len() == 1 {
                ui.separator();
                let t = &all[i];
                if widgets::menu_item(ui, "Go to album") {
                    app.act(Action::Navigate(Route::LocalAlbum { artist: album_artist(t).to_string(), album: album_title(t).to_string() }));
                }
                if widgets::menu_item(ui, "Go to artist") {
                    app.local_artist = Some(album_artist(t).to_string());
                    app.settings.local_view = LocalView::Artists;
                    app.mark_settings_dirty();
                    app.act(Action::Navigate(Route::LocalFiles));
                }
                if widgets::menu_item(ui, "Edit song info…") {
                    app.act(Action::EditTags(path.clone()));
                }
                if widgets::menu_item(ui, crate::local::REVEAL) {
                    crate::local::reveal(&path);
                }
            }
        });
    }
}

/// Plays `list` (library indices) from the song `start` (a library index).
fn play_from(app: &mut App, list: &[usize], start: usize) {
    let at = list.iter().position(|&i| i == start).unwrap_or(0);
    app.act(Action::PlayLocalTracks { indices: list.to_vec(), start: at });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str, album: &str, track: u32, disc: u32) -> LocalTrack {
        LocalTrack {
            path: format!("/m/{artist}/{album}/{title}.mp3").into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            track_no: track,
            disc_no: disc,
            duration_ms: 200_000,
            ..Default::default()
        }
    }

    #[test]
    fn albums_artists_and_songs_are_filed_like_itunes() {
        let all = vec![
            t("Plastic Love", "Mariya Takeuchi", "Variety", 2, 1),
            t("Mayonaka no Door", "Miki Matsubara", "Pocket Park", 1, 0),
            t("Tokyo Rose", "Mariya Takeuchi", "Variety", 1, 1),
            t("Bonus", "Mariya Takeuchi", "Variety", 1, 2),
            t("Loose", "The Beatles", "", 0, 0),
            LocalTrack { album_artist: "Various Artists".into(), ..t("Midnight Pretenders", "Tomoko Aran", "City Pop Hits", 1, 0) },
            LocalTrack { album_artist: "Various Artists".into(), ..t("Fly-Day Chinatown", "Yasuha", "City Pop Hits", 2, 0) },
        ];
        let idx = index(&all);
        let names: Vec<(&str, &str)> = idx.albums.iter().map(|a| (a.artist.as_str(), a.title.as_str())).collect();
        // "The Beatles" sorts under B; a compilation sits under its album artist.
        assert_eq!(names, [("The Beatles", "Unknown Album"), ("Mariya Takeuchi", "Variety"), ("Miki Matsubara", "Pocket Park"), ("Various Artists", "City Pop Hits")]);
        let variety: Vec<&str> = idx.albums[1].tracks.iter().map(|&i| all[i].title.as_str()).collect();
        assert_eq!(variety, ["Tokyo Rose", "Plastic Love", "Bonus"], "disc, then track number");
        assert_eq!(idx.artists.len(), 4);
        assert_eq!(idx.songs.len(), all.len(), "every song once");
    }

    #[test]
    fn each_file_is_listed_once_however_folders_overlap() {
        let dir = crate::config::test_dir("overlap");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Synced files/Tatsuro Yamashita")).unwrap();
        std::fs::write(dir.join("Synced files/Tatsuro Yamashita/Sparkle.mp3"), b"").unwrap();
        let sources = vec![dir.clone(), dir.join("../").join(dir.file_name().unwrap()), dir.clone(), dir.join("Synced files"), dir.join("Synced files/Tatsuro Yamashita")];
        assert_eq!(crate::local::tidy_sources(&sources), vec![dir.clone()]);
        assert_eq!(crate::local::scan(&sources).len(), 1, "one song, not four");
        let mut list = vec![dir.join("Synced files")];
        assert!(crate::local::add_source(&mut list, dir.clone()), "a folder around a source replaces it");
        assert_eq!(list, vec![dir.clone()]);
        assert!(!crate::local::add_source(&mut list, dir.join("Synced files")), "a folder inside one is already there");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
