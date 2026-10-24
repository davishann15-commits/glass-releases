//! The library screen, split in two: your library as a numbered list on the
//! left, the selected item on the right (artwork, play/shuffle, featured
//! artists), with the command bar above it.
//!
//! Click a row to select it, double-click (or Enter, or its ›) to open it.
//! j/k or ↑/↓ move the selection.

use super::command;
use super::icons::{self, Icon};
use super::menus;
use super::theme::{self, col, px, sp};
use super::widgets::{self, chip, icon_btn, paint_cover};
use crate::app::{Action, App, LibFilter, Modal, Route};
use crate::config::{LibrarySort, LibraryView};
use crate::model::{self, s};
use eframe::egui::{self, Align2, CursorIcon, Key, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct Entry {
    pub uri: String,
    pub name: String,
    pub sub: String,
    pub img: Option<String>,
    pub round: bool,
    pub filter: LibFilter,
    pub added: String,
    pub creator: String,
    /// The item as the library response has it (shared: rows clone entries freely).
    pub value: Arc<Value>,
    pub special: Option<Route>,
    /// Name and subtitle, lowercased, for the command bar's search.
    pub find: String,
}

fn entry(uri: &str, name: &str, sub: String, img: Option<String>, round: bool, filter: LibFilter, value: Value) -> Entry {
    let find = format!("{name} {sub}").to_lowercase();
    Entry { uri: uri.into(), name: name.into(), sub, img, round, filter, added: String::new(), creator: String::new(), value: Arc::new(value), special: None, find }
}

fn fingerprint(parts: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Identifies a cached response: a new response is a new allocation.
fn ptr(v: &Option<Arc<Value>>) -> usize {
    v.as_ref().map(|a| Arc::as_ptr(a) as usize).unwrap_or(0)
}

/// Everything saved in the library, unsorted. `true` while still loading.
/// Rebuilt only when one of the responses it's made from changes — not
/// every frame (copying a large library 60 times a second was most of the
/// Library screen's CPU and allocation cost).
pub fn entries(app: &mut App) -> (Arc<Vec<Entry>>, bool) {
    let sources = [
        app.want_all("/me/playlists?limit=50", 2000),
        app.want_all("/me/albums?limit=50", 2000),
        app.want_all("/me/following?type=artist&limit=50", 2000),
        app.want_all("/me/shows?limit=50", 500),
        app.want_all("/me/audiobooks?limit=50", 500),
    ];
    let me = app.user_id();
    let my_name = app.me.as_ref().map(|m| s(m, "display_name").to_string());
    let key = fingerprint((sources.iter().map(ptr).collect::<Vec<_>>(), &me, &my_name));
    let id = egui::Id::new("library-entries");
    if let Some((k, v, loading)) = app.ctx.data(|d| d.get_temp::<(u64, Arc<Vec<Entry>>, bool)>(id))
        && k == key
    {
        return (v, loading);
    }
    let [playlists, albums, artists, shows, books] = sources;
    let mut out = Vec::new();
    let mut loading = false;
    if let Some(v) = playlists {
        for p in model::items(&v) {
            let owner = s(&p["owner"], "display_name").to_string();
            let by = if p["owner"]["id"].as_str() == Some(me.as_str()) { my_name.clone().unwrap_or(owner.clone()) } else { owner.clone() };
            let mut e = entry(s(p, "uri"), s(p, "name"), format!("Playlist • {by}"), model::image(p, 300), false, LibFilter::Playlists, p.clone());
            e.creator = owner;
            out.push(e);
        }
    } else {
        loading = true;
    }
    if let Some(v) = albums {
        for it in model::items(&v) {
            let a = &it["album"];
            let mut e = entry(s(a, "uri"), s(a, "name"), format!("Album • {}", model::artists_line(a)), model::image(a, 300), false, LibFilter::Albums, a.clone());
            e.added = s(it, "added_at").into();
            e.creator = model::artists_line(a);
            out.push(e);
        }
    }
    if let Some(v) = artists {
        for a in model::items(&v) {
            out.push(entry(s(a, "uri"), s(a, "name"), "Artist".into(), model::image(a, 300), true, LibFilter::Artists, a.clone()));
        }
    }
    if let Some(v) = shows {
        for it in model::items(&v) {
            let sh = &it["show"];
            let mut e = entry(s(sh, "uri"), s(sh, "name"), format!("Podcast • {}", s(sh, "publisher")), model::image(sh, 300), false, LibFilter::Podcasts, sh.clone());
            e.added = s(it, "added_at").into();
            e.creator = s(sh, "publisher").into();
            out.push(e);
        }
    }
    if let Some(v) = books {
        for it in model::items(&v) {
            let b = it.get("audiobook").unwrap_or(it);
            let mut e = entry(s(b, "uri"), s(b, "name"), format!("Audiobook • {}", model::artists_line(b)), model::image(b, 300), false, LibFilter::Audiobooks, b.clone());
            e.added = s(it, "added_at").into();
            e.creator = model::artists_line(b);
            out.push(e);
        }
    }
    let out = Arc::new(out);
    app.ctx.data_mut(|d| d.insert_temp(id, (key, out.clone(), loading)));
    (out, loading)
}

fn special(uri: String, name: &str, sub: String, filter: LibFilter, route: Route) -> Entry {
    let find = format!("{name} {sub}").to_lowercase();
    Entry { uri, name: name.into(), sub, img: None, round: false, filter, added: String::new(), creator: String::new(), value: Arc::new(json!({})), special: Some(route), find }
}

/// Liked Songs, Your Episodes and Local Files.
pub fn specials(app: &mut App) -> Vec<Entry> {
    let uid = app.user_id();
    let liked = app.want("/me/tracks?limit=1").map(|v| model::u(&v, "total")).unwrap_or(0);
    let eps = app.want("/me/episodes?limit=1").map(|v| model::u(&v, "total")).unwrap_or(0);
    let mut out = vec![
        special(format!("spotify:user:{uid}:collection"), "Liked Songs", format!("Playlist • {}", model::songs(liked)), LibFilter::Playlists, Route::LikedSongs),
        special(
            format!("spotify:user:{uid}:collection:your-episodes"),
            "Your Episodes",
            format!("Podcast • {} {}", model::fmt_count(eps), if eps == 1 { "episode" } else { "episodes" }),
            LibFilter::Podcasts,
            Route::YourEpisodes,
        ),
    ];
    if app.settings.show_local_files {
        out.push(special("glass:local-files".into(), "Local Files", "Playlist • files on this computer".into(), LibFilter::Playlists, Route::LocalFiles));
    }
    out
}

fn recent_rank(app: &mut App) -> HashMap<String, usize> {
    let mut rank = HashMap::new();
    if let Some(v) = app.want("/me/player/recently-played?limit=50") {
        for it in model::items(&v) {
            let uri = it["context"]["uri"].as_str().map(str::to_string).or_else(|| it["track"]["album"]["uri"].as_str().map(str::to_string));
            if let Some(u) = uri {
                let n = rank.len();
                rank.entry(u).or_insert(n);
            }
        }
    }
    rank
}

/// The chips under each tab.
fn chips_for(tab: u8) -> &'static [(LibFilter, &'static str)] {
    match tab {
        0 => &[(LibFilter::Playlists, "playlists"), (LibFilter::Albums, "albums"), (LibFilter::Artists, "artists"), (LibFilter::Downloaded, "downloaded")],
        1 => &[(LibFilter::Podcasts, "shows"), (LibFilter::Downloaded, "downloaded")],
        _ => &[(LibFilter::Audiobooks, "audiobooks")],
    }
}

/// The rows to show for the current tab, chip, sort and filter text; like
/// `entries`, rebuilt only when something they depend on changes.
fn rows(app: &mut App) -> (Arc<Vec<(Entry, u8)>>, bool) {
    let chips = chips_for(app.lib_tab);
    if !chips.iter().any(|(f, _)| *f == app.lib_filter) {
        app.lib_filter = chips[0].0;
    }
    let (all, loading) = entries(app);
    let recent = if app.settings.library_sort == LibrarySort::Recents { app.want("/me/player/recently-played?limit=50") } else { None };
    let liked = app.want("/me/tracks?limit=1");
    let eps = app.want("/me/episodes?limit=1");
    let downloads = if app.lib_filter == LibFilter::Downloaded { app.downloads.collections() } else { Vec::new() };
    let mut expanded: Vec<&String> = app.expanded_folders.iter().collect();
    expanded.sort();
    let folders: Vec<(&String, &String, &Vec<String>)> = app.settings.folders.iter().map(|f| (&f.id, &f.name, &f.playlists)).collect();
    let key = fingerprint((
        Arc::as_ptr(&all) as usize,
        app.lib_tab,
        format!("{:?}{:?}", app.lib_filter, app.settings.library_sort),
        &app.cmd_text,
        &app.settings.pins,
        folders,
        expanded,
        &downloads,
        (ptr(&recent), ptr(&liked), ptr(&eps), app.settings.show_local_files),
    ));
    let id = egui::Id::new("library-rows");
    if let Some((k, v)) = app.ctx.data(|d| d.get_temp::<(u64, Arc<Vec<(Entry, u8)>>)>(id))
        && k == key
    {
        return (v, loading);
    }
    let out = Arc::new(build_rows(app, &all, downloads));
    app.ctx.data_mut(|d| d.insert_temp(id, (key, out.clone())));
    (out, loading)
}

/// The rows for the current tab, filter, sort and search. Entries are sorted by
/// reference; only the rows shown are copied.
fn build_rows(app: &mut App, all: &[Entry], downloads: Vec<(String, String)>) -> Vec<(Entry, u8)> {
    let filter = app.lib_filter;
    let mut all: Vec<&Entry> = all.iter().collect();
    match app.settings.library_sort {
        LibrarySort::Recents => {
            let rank = recent_rank(app);
            all.sort_by_key(|e| rank.get(&e.uri).cloned().unwrap_or(10_000));
        }
        LibrarySort::RecentlyAdded => all.sort_by(|a, b| b.added.cmp(&a.added)),
        LibrarySort::Alphabetical => all.sort_by_cached_key(|e| e.name.to_lowercase()),
        LibrarySort::Creator => all.sort_by_cached_key(|e| e.creator.to_lowercase()),
    }
    // Matched as the command bar does: every word. "? …" is a question for the
    // assistant, not a filter.
    let q = app.cmd_text.trim();
    let q = if q.starts_with('?') { String::new() } else { q.trim_start_matches('>').trim().to_lowercase() };
    let hit = |e: &Entry| q.split_whitespace().all(|w| e.find.contains(w));
    let by_uri: HashMap<&str, &Entry> = all.iter().map(|e| (e.uri.as_str(), *e)).collect();
    let mut out: Vec<(Entry, u8)> = Vec::new();

    if filter == LibFilter::Downloaded {
        let known = specials(app);
        for (uri, name) in downloads {
            let e = known
                .iter()
                .find(|e| e.uri == uri)
                .or_else(|| by_uri.get(uri.as_str()).copied())
                .cloned()
                .unwrap_or_else(|| entry(&uri, &name, "Download".into(), None, false, LibFilter::Downloaded, json!({ "uri": uri, "name": name })));
            let in_tab = match app.lib_tab {
                1 => matches!(e.filter, LibFilter::Podcasts),
                _ => !matches!(e.filter, LibFilter::Podcasts | LibFilter::Audiobooks),
            };
            if in_tab && hit(&e) {
                out.push((e, 0));
            }
        }
        return out;
    }

    for e in specials(app) {
        if e.filter == filter && hit(&e) {
            out.push((e, 0));
        }
    }
    let pins = &app.settings.pins;
    let in_folder: std::collections::HashSet<&str> = app.settings.folders.iter().flat_map(|f| f.playlists.iter().map(String::as_str)).collect();
    for p in pins {
        if let Some(e) = by_uri.get(p.as_str())
            && e.filter == filter
            && hit(e)
        {
            out.push(((*e).clone(), 0));
        }
    }
    if filter == LibFilter::Playlists {
        for f in &app.settings.folders {
            let mut folder = special(
                format!("glass:folder:{}", f.id),
                &f.name,
                format!("Folder • {} playlist{}", f.playlists.len(), if f.playlists.len() == 1 { "" } else { "s" }),
                LibFilter::Playlists,
                Route::Folder(f.id.clone()),
            );
            folder.value = Arc::new(json!({ "folder": f.id }));
            // A pinned playlist is already at the top.
            let children: Vec<&Entry> = f.playlists.iter().filter(|u| !pins.contains(u)).filter_map(|u| by_uri.get(u.as_str()).copied()).filter(|e| hit(e)).collect();
            if !hit(&folder) && children.is_empty() {
                continue;
            }
            out.push((folder, 0));
            if app.expanded_folders.contains(&f.id) || !q.is_empty() {
                out.extend(children.into_iter().map(|c| (c.clone(), 1)));
            }
        }
    }
    for e in all {
        if e.filter == filter && !pins.contains(&e.uri) && !in_folder.contains(e.uri.as_str()) && hit(e) {
            out.push((e.clone(), 0));
        }
    }
    out
}

pub fn show(app: &mut App, ui: &mut Ui, area: Rect) {
    let pad = sp(4.0);
    let list_w = (area.width() * 0.45).clamp(px(320.0), px(660.0));
    let detail_left = area.left() + pad + list_w + sp(5.0);
    let detail_w = area.right() - sp(3.0) - detail_left;
    let narrow = detail_w < px(300.0);
    let top = area.top() + sp(2.5);

    let (list_rect, cmd_rect) = if narrow {
        let cmd = Rect::from_min_size(pos2(area.left() + pad, top), vec2(area.width() - pad - sp(3.0), px(44.0)));
        (Rect::from_min_max(pos2(area.left() + pad, cmd.bottom() + sp(2.0)), pos2(area.right() - sp(3.0), area.bottom())), cmd)
    } else {
        (Rect::from_min_max(pos2(area.left() + pad, top), pos2(area.left() + pad + list_w, area.bottom())), Rect::from_min_size(pos2(detail_left, top), vec2(detail_w, px(44.0))))
    };
    // Read before the command bar: pressing Enter there gives up its focus,
    // and that same Enter mustn't also open the selected row.
    let typing = ui.ctx().egui_wants_keyboard_input();
    command::bar(app, ui, cmd_rect);

    let (rows, loading) = rows(app);
    let selected_idx = app.lib_selected.as_ref().and_then(|u| rows.iter().position(|(e, _)| &e.uri == u)).unwrap_or(0);
    let mut keyboard_moved = false;
    if !rows.is_empty() {
        let mut idx = selected_idx;
        if !typing && app.modal.is_none() && !egui::Popup::is_any_open(ui.ctx()) {
            let none = egui::Modifiers::NONE;
            let (down, up, play_key, enter, tab) = ui.input_mut(|i| {
                let down = i.consume_key(none, Key::J) || i.consume_key(none, Key::ArrowDown);
                let up = i.consume_key(none, Key::K) || i.consume_key(none, Key::ArrowUp);
                let play_key = i.consume_key(egui::Modifiers::SHIFT, Key::Enter);
                let enter = i.consume_key(none, Key::Enter);
                let tab = [Key::Num1, Key::Num2, Key::Num3].iter().position(|k| i.consume_key(none, *k));
                (down, up, play_key, enter, tab)
            });
            if down {
                idx = (idx + 1).min(rows.len() - 1);
            }
            if up {
                idx = idx.saturating_sub(1);
            }
            // Enter opens, ⇧Enter plays; 1/2/3 switch tabs.
            if enter {
                open(app, &rows[idx].0);
            }
            if play_key {
                let shuffle = app.pb.shuffle;
                play(app, &rows[idx].0, shuffle);
            }
            if let Some(t) = tab {
                app.lib_tab = t as u8;
            }
        }
        keyboard_moved = idx != selected_idx;
        app.lib_selected = Some(rows[idx].0.uri.clone());
    }
    let selected = rows.iter().find(|(e, _)| Some(&e.uri) == app.lib_selected.as_ref()).map(|(e, _)| e.clone());

    ui.scope_builder(UiBuilder::new().max_rect(list_rect).id_salt("lib-list"), |ui| list(app, ui, &rows, loading, keyboard_moved));
    if !narrow {
        let divider_x = list_rect.right() + sp(2.5);
        ui.painter().line_segment([pos2(divider_x, top + px(110.0)), pos2(divider_x, area.bottom() - sp(3.0))], theme::hairline());
        let detail = Rect::from_min_max(pos2(detail_left, cmd_rect.bottom() + sp(3.0)), pos2(detail_left + detail_w, area.bottom()));
        if let Some(e) = selected {
            ui.scope_builder(UiBuilder::new().max_rect(detail).id_salt("lib-detail"), |ui| self::detail(app, ui, &e));
        }
    }
}

fn list(app: &mut App, ui: &mut Ui, rows: &[(Entry, u8)], loading: bool, keyboard_moved: bool) {
    // Greeting and title.
    ui.label(RichText::new(model::greeting().to_lowercase()).font(theme::regular(14.0)).color(col::sub()));
    // The cursor blinks while the window is in front and in use, then holds still
    // (blinking forever would redraw the window forever).
    let (now, active) = ui.input(|i| (i.time, i.focused && (!i.events.is_empty() || i.pointer.is_moving())));
    let since = egui::Id::new("library-cursor-since");
    if active || ui.data(|d| d.get_temp::<f64>(since)).is_none() {
        ui.data_mut(|d| d.insert_temp(since, now));
    }
    let idle = now - ui.data(|d| d.get_temp::<f64>(since)).unwrap_or(now);
    let blink = !theme::reduce_motion() && ui.input(|i| i.focused) && idle < 10.0;
    let blink_on = !blink || (now % 1.2) < 0.7;
    if blink {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(600));
    }
    ui.label(RichText::new(format!("library{}", if blink_on { "_" } else { " " })).font(theme::regular(40.0)).color(col::text()));
    ui.add_space(sp(2.0));

    // Tabs.
    let tabs_y = ui.cursor().top();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = sp(5.0);
        for (i, name) in ["music", "podcasts", "audiobooks"].iter().enumerate() {
            let on = app.lib_tab == i as u8;
            let font = theme::regular(15.0);
            let galley = ui.painter().layout_no_wrap(name.to_string(), font.clone(), col::text());
            let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + sp(2.0), px(34.0)), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, *name));
            let color = if on {
                col::accent()
            } else if resp.hovered() {
                col::text()
            } else {
                col::sub()
            };
            ui.painter().text(pos2(r.left() + sp(1.0), r.center().y - px(2.0)), Align2::LEFT_CENTER, *name, font, color);
            if on {
                ui.painter().line_segment([pos2(r.left(), r.bottom() + px(4.0)), pos2(r.right(), r.bottom() + px(4.0))], Stroke::new(2.0, col::accent()));
            }
            widgets::focus_ring(ui, &resp, r, theme::small_radius());
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !on {
                app.lib_tab = i as u8;
                app.lib_filter = chips_for(app.lib_tab)[0].0;
                app.lib_selected = None;
            }
        }
    });
    let line_y = tabs_y + px(40.0);
    let full = ui.max_rect();
    ui.painter().line_segment([pos2(full.left(), line_y), pos2(full.right(), line_y)], theme::hairline());
    ui.add_space(sp(2.5));

    // Chips and sort. When the column is narrow the chips scroll sideways
    // and the sort control shrinks to its arrow.
    ui.horizontal(|ui| {
        let sort_room = if ui.available_width() < px(470.0) { px(34.0) } else { px(140.0) };
        let chips_w = (ui.available_width() - sort_room).max(px(60.0));
        ui.allocate_ui(vec2(chips_w, px(34.0)), |ui| {
            egui::ScrollArea::horizontal().id_salt(("lib-chips", app.lib_tab)).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = sp(1.25);
                    for (f, name) in chips_for(app.lib_tab) {
                        if chip(ui, name, app.lib_filter == *f) {
                            app.lib_filter = *f;
                            app.lib_selected = None;
                        }
                    }
                });
            });
        });
        sort_menu(app, ui, sort_room < px(100.0));
    });
    ui.add_space(sp(1.5));

    let view = app.settings.library_view;
    ScrollArea::vertical().id_salt(("library", app.lib_tab)).auto_shrink([false, false]).show(ui, |ui| {
        if rows.is_empty() && !loading {
            let (title, body) = match app.lib_filter {
                LibFilter::Downloaded => ("nothing downloaded yet", "Choose download on a playlist, album or podcast to keep it on this computer."),
                _ if !app.cmd_text.trim().is_empty() => ("no matches", "Nothing in your library matches the command bar text."),
                _ => ("nothing here yet", "Things you save on Spotify show up here."),
            };
            widgets::unavailable(ui, title, body);
        }
        if view == LibraryView::Grid {
            widgets::card_grid(ui, rows.len(), false, |ui, i, w| {
                let (e, _) = &rows[i];
                let r = widgets::card(ui, e.img.as_deref(), &e.name, &e.sub, e.round, w.min(px(150.0)), false);
                if r.open {
                    app.lib_selected = Some(e.uri.clone());
                }
                if r.play {
                    play(app, e, false);
                }
                if r.response.double_clicked() {
                    open(app, e);
                }
                let e = e.clone();
                r.response.context_menu(|ui| context_menu(app, ui, &e));
            });
        } else {
            for (i, (e, indent)) in rows.iter().enumerate() {
                row(app, ui, i, e, *indent, view == LibraryView::Compact, keyboard_moved);
            }
        }
        if loading {
            widgets::loading(ui);
        }
        ui.add_space(sp(2.0));
    });
}

fn sort_menu(app: &mut App, ui: &mut Ui, compact: bool) {
    let name = match app.settings.library_sort {
        LibrarySort::Recents => "recents",
        LibrarySort::RecentlyAdded => "recently added",
        LibrarySort::Alphabetical => "alphabetical",
        LibrarySort::Creator => "creator",
    };
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let font = theme::regular(12.5);
        let w = if compact { 0.0 } else { ui.painter().layout_no_wrap(name.to_string(), font.clone(), col::sub()).size().x };
        let (r, resp) = ui.allocate_exact_size(vec2(w + px(30.0), px(32.0)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, theme::small_radius(), col::hover());
        }
        if !compact {
            ui.painter().text(pos2(r.left() + px(4.0), r.center().y), Align2::LEFT_CENTER, name, font, col::sub());
        }
        let resp = resp.on_hover_text(format!("sorted by {name}"));
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.right() - px(10.0), r.center().y), vec2(px(12.0), px(12.0))), Icon::ChevronDown, col::sub());
        widgets::focus_ring(ui, &resp, r, theme::small_radius());
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        egui::Popup::menu(&resp).show(|ui| {
            ui.label(RichText::new("sort by").font(theme::regular(11.5)).color(col::muted()));
            for (srt, n) in [(LibrarySort::Recents, "recents"), (LibrarySort::RecentlyAdded, "recently added"), (LibrarySort::Alphabetical, "alphabetical"), (LibrarySort::Creator, "creator")] {
                if ui.selectable_label(app.settings.library_sort == srt, n).clicked() {
                    app.settings.library_sort = srt;
                    app.mark_settings_dirty();
                }
            }
            ui.separator();
            ui.label(RichText::new("view as").font(theme::regular(11.5)).color(col::muted()));
            for (v, n) in [(LibraryView::List, "list"), (LibraryView::Compact, "compact"), (LibraryView::Grid, "grid")] {
                if ui.selectable_label(app.settings.library_view == v, n).clicked() {
                    app.settings.library_view = v;
                    app.mark_settings_dirty();
                }
            }
            ui.separator();
            if widgets::menu_item(ui, "new playlist") {
                app.act(Action::CreatePlaylist { name: "New Playlist".into(), uris: vec![] });
            }
            if widgets::menu_item(ui, "new folder") {
                app.modal = Some(Modal::NewFolder { name: "New Folder".into(), add: None });
            }
        });
    });
}

fn special_art(ui: &Ui, r: Rect, route: &Route) {
    let icon = match route {
        Route::LikedSongs => Icon::HeartFilled,
        Route::YourEpisodes => Icon::Bell,
        Route::LocalFiles => Icon::Download,
        _ => Icon::Folder,
    };
    let radius = egui::CornerRadius::same((theme::radius_f() * 0.5) as u8);
    ui.painter().rect(r, radius, col::accent_a(0.14), Stroke::new(1.0, col::accent_a(0.5)), StrokeKind::Inside);
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(r.width() * 0.36, r.width() * 0.36)), icon, col::accent());
}

fn row(app: &mut App, ui: &mut Ui, i: usize, e: &Entry, indent: u8, compact: bool, keyboard_moved: bool) {
    let h = if compact { px(40.0) } else { px(74.0) };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
    let selected = app.lib_selected.as_deref() == Some(e.uri.as_str());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &e.name));
    if selected && keyboard_moved {
        ui.scroll_to_rect(rect, None);
    }
    if !ui.is_rect_visible(rect) {
        return;
    }
    let playing_here = !app.pb.context_uri.is_empty() && app.pb.context_uri == e.uri;
    let inner = rect.shrink2(vec2(0.0, px(3.0)));
    if selected {
        ui.painter().rect(inner, theme::small_radius(), col::selection(), Stroke::new(1.2, col::accent()), StrokeKind::Inside);
    } else {
        if resp.hovered() {
            ui.painter().rect_filled(inner, theme::small_radius(), col::hover());
        }
        ui.painter().line_segment([pos2(rect.left() + sp(1.0), rect.bottom()), pos2(rect.right() - sp(1.0), rect.bottom())], theme::hairline());
    }
    let x0 = rect.left() + sp(1.25) + indent as f32 * sp(2.0);
    let num_rect = Rect::from_min_size(pos2(x0, rect.center().y - px(9.0)), vec2(px(24.0), px(18.0)));
    // The number turns into a play button under the pointer. It's always
    // registered, so the click can't be lost.
    let playable = !matches!(e.special, Some(Route::Folder(_)) | Some(Route::YourEpisodes));
    let play_resp = ui.interact(num_rect.expand(px(6.0)), resp.id.with("play"), Sense::click());
    let icon = Rect::from_center_size(num_rect.center(), vec2(px(14.0), px(14.0)));
    if playable && ui.rect_contains_pointer(rect) {
        let icon_kind = if playing_here && app.pb.is_playing { Icon::Pause } else { Icon::Play };
        icons::paint(ui.painter(), icon, icon_kind, if play_resp.hovered() { col::accent() } else { col::text() });
        play_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Play {}", e.name)));
    } else if playing_here && app.pb.is_playing {
        widgets::equalizer_bars(ui, icon, col::accent());
    } else {
        ui.painter().text(pos2(x0, rect.center().y), Align2::LEFT_CENTER, format!("{:02}", i + 1), theme::regular(13.5), if selected { col::sub() } else { col::muted() });
    }
    if playable && play_resp.clicked() {
        app.lib_selected = Some(e.uri.clone());
        if playing_here {
            app.act(Action::TogglePlay);
        } else {
            let shuffle = app.pb.shuffle;
            play(app, e, shuffle);
        }
    }
    let mut x = x0 + px(38.0);
    if !compact {
        let c = Rect::from_min_size(pos2(x, rect.center().y - px(29.0)), vec2(px(58.0), px(58.0)));
        match &e.special {
            Some(r) => special_art(ui, c, r),
            None => paint_cover(ui, c, e.img.as_deref(), e.round),
        }
        x = c.right() + sp(2.75);
    }
    let max_w = rect.right() - x - px(36.0);
    let title_color = if playing_here { col::accent() } else { col::text() };
    if compact {
        let w = widgets::text_trunc(ui, pos2(x, rect.center().y), &e.name, theme::regular(14.0), title_color, max_w);
        widgets::text_trunc(ui, pos2(x + w + sp(1.5), rect.center().y), &e.sub, theme::regular(12.0), col::muted(), (max_w - w - sp(1.5)).max(0.0));
    } else {
        widgets::text_trunc(ui, pos2(x, rect.center().y - px(11.0)), &e.name, theme::regular(15.0), title_color, max_w);
        widgets::text_trunc(ui, pos2(x, rect.center().y + px(12.0)), &e.sub, theme::regular(12.5), col::muted(), max_w);
    }
    // Chevron opens.
    let chev = Rect::from_center_size(pos2(rect.right() - px(18.0), rect.center().y), vec2(px(28.0), px(28.0)));
    let chev_resp = ui.interact(chev, resp.id.with("open"), Sense::click()).on_hover_text("Open");
    chev_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Open {}", e.name)));
    icons::paint(ui.painter(), Rect::from_center_size(chev.center(), vec2(px(14.0), px(14.0))), Icon::Chevron, if chev_resp.hovered() { col::text() } else { col::muted() });
    if chev_resp.clicked() {
        open(app, e);
    }
    widgets::focus_ring(ui, &resp, inner, theme::small_radius());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);

    // Drag playlists into folders.
    if e.value.get("type").and_then(Value::as_str) == Some("playlist") && resp.drag_started() {
        resp.dnd_set_drag_payload(e.uri.clone());
    }
    if e.uri.starts_with("glass:folder:") {
        if resp.dnd_hover_payload::<String>().is_some() {
            ui.painter().rect_stroke(inner, theme::small_radius(), Stroke::new(1.5, col::accent()), StrokeKind::Inside);
        }
        if let Some(p) = resp.dnd_release_payload::<String>() {
            let fid = e.uri.trim_start_matches("glass:folder:").to_string();
            app.act(Action::MoveToFolder { playlist: (*p).clone(), folder: Some(fid) });
        }
    }
    if resp.clicked() {
        app.lib_selected = Some(e.uri.clone());
        if let Some(Route::Folder(fid)) = &e.special
            && !app.expanded_folders.remove(fid)
        {
            app.expanded_folders.insert(fid.clone());
        }
    }
    if resp.double_clicked() {
        open(app, e);
    }
    resp.context_menu(|ui| context_menu(app, ui, e));
}

fn open(app: &mut App, e: &Entry) {
    if let Some(r) = &e.special {
        app.act(Action::Navigate(r.clone()));
    } else if let Some(r) = menus::route_for(&e.value) {
        app.act(Action::Navigate(r));
    }
}

fn play(app: &mut App, e: &Entry, shuffle: bool) {
    match &e.special {
        Some(Route::LocalFiles) => app.act(Action::PlayLocal { index: 0 }),
        Some(Route::YourEpisodes) | Some(Route::Folder(_)) => open(app, e),
        _ => {
            if shuffle != app.pb.shuffle {
                app.act(Action::SetShuffle(shuffle));
            }
            app.act(Action::PlayContext { context: e.uri.clone(), track: None });
        }
    }
}

fn context_menu(app: &mut App, ui: &mut Ui, e: &Entry) {
    if let Some(Route::Folder(fid)) = &e.special {
        let fid = fid.clone();
        if widgets::menu_item(ui, "Rename") {
            app.modal = Some(Modal::RenameFolder { id: fid.clone(), name: e.name.clone() });
        }
        if widgets::menu_item(ui, "Delete") {
            app.modal = Some(Modal::Confirm {
                title: "Delete folder?".into(),
                body: format!("This will delete “{}”. Its playlists stay in Your Library.", e.name),
                confirm: "Delete".into(),
                action: Box::new(Action::DeleteFolder(fid)),
            });
        }
    } else if e.special.is_some() {
        if widgets::menu_item(ui, "Open") {
            open(app, e);
        }
        if matches!(e.special, Some(Route::LikedSongs) | Some(Route::YourEpisodes)) {
            let kept = app.downloads.is_kept(&e.uri);
            if widgets::menu_item(ui, if kept { "Remove download" } else { "Download" }) {
                app.act(if kept { Action::RemoveDownload(e.uri.clone()) } else { Action::Download { uri: e.uri.clone(), name: e.name.clone() } });
            }
        }
    } else {
        menus::any(app, ui, &e.value);
    }
}

/// Who's shown under the selection: artists fetched one by one, or a list of them.
enum Featured {
    Each(Vec<Arc<Value>>),
    List(Arc<Value>),
}

impl Featured {
    fn artists(&self) -> Vec<&Value> {
        match self {
            Featured::Each(list) => list.iter().map(|a| &**a).collect(),
            Featured::List(v) => model::arr(v, "artists").iter().collect(),
        }
    }
}

/// The artists who feature most in `v` (a page of songs), counted once per answer.
fn top_artists(ui: &Ui, v: &Arc<Value>) -> Arc<Vec<String>> {
    let id = egui::Id::new("featured-artists");
    let key = Arc::as_ptr(v) as usize;
    if let Some((k, ids)) = ui.data(|d| d.get_temp::<(usize, Arc<Vec<String>>)>(id))
        && k == key
    {
        return ids;
    }
    let count_artists = |items: &[Value]| -> Vec<String> {
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
        order
    };
    let ids = Arc::new(count_artists(model::items(v)));
    ui.data_mut(|d| d.insert_temp(id, (key, ids.clone())));
    ids
}

/// Artists to feature for a selection: the most frequent in a playlist,
/// an album's artists, or an artist's "fans also like".
fn featured(app: &mut App, ui: &Ui, e: &Entry) -> (&'static str, Featured) {
    let ids: Arc<Vec<String>> = match (&e.special, model::kind_of(&e.uri)) {
        // The first page is plenty to find who features most, and doesn't
        // load a whole library just to show four pictures.
        (Some(Route::LikedSongs), _) => app.want("/me/tracks?limit=50").map(|v| top_artists(ui, &v)).unwrap_or_default(),
        (None, "playlist") => app.want(&format!("/playlists/{}/tracks?limit=100", model::id_of(&e.uri))).map(|v| top_artists(ui, &v)).unwrap_or_default(),
        (None, "album") => Arc::new(model::arr(&e.value, "artists").iter().map(|a| s(a, "id").to_string()).collect()),
        (None, "artist") => {
            return match app.want(&format!("/artists/{}/related-artists", model::id_of(&e.uri))) {
                Some(v) => ("fans also like", Featured::List(v)),
                None => ("fans also like", Featured::Each(Vec::new())),
            };
        }
        _ => Arc::default(),
    };
    let artists = ids.iter().take(4).filter_map(|id| app.want(&format!("/artists/{id}"))).collect();
    ("featured artists", Featured::Each(artists))
}

fn detail(app: &mut App, ui: &mut Ui, e: &Entry) {
    let w = ui.available_width();
    let art = (w * 0.48).clamp(px(160.0), px(316.0)).min(ui.available_height() * 0.45);
    let (ar, aresp) = ui.allocate_exact_size(vec2(art, art), Sense::click());
    match &e.special {
        Some(r) => special_art(ui, ar, r),
        None => paint_cover(ui, ar, model::image(&e.value, 640).or_else(|| e.img.clone()).as_deref(), e.round),
    }
    if aresp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Open").clicked() {
        open(app, e);
    }
    ui.add_space(sp(2.0));
    let kind = match (&e.special, model::kind_of(&e.uri)) {
        (Some(Route::Folder(_)), _) => "folder",
        (Some(_), _) => "collection",
        (None, "playlist") => "playlist",
        (None, "album") => "album",
        (None, "artist") => "artist",
        (None, "show") if e.filter == LibFilter::Audiobooks => "audiobook",
        (None, "show") => "podcast",
        _ => "download",
    };
    widgets::caps_label(ui, kind, 11.0, col::muted());
    ui.add(egui::Label::new(RichText::new(&e.name).font(theme::regular(28.0)).color(col::text())).wrap());
    let subtitle = match (&e.special, model::kind_of(&e.uri)) {
        (None, "playlist") => {
            let desc = model::plain(s(&e.value, "description"));
            if desc.trim().is_empty() { format!("by {} · {}", e.sub.trim_start_matches("Playlist • "), model::songs(model::u(&e.value["tracks"], "total"))) } else { desc }
        }
        (None, "album") => format!("{} · {}", model::artists_line(&e.value), model::year(s(&e.value, "release_date"))),
        (None, "artist") => match app.want(&format!("/artists/{}", model::id_of(&e.uri))).and_then(|a| a["monthly_listeners"].as_u64()) {
            Some(n) => format!("{} monthly listeners", model::fmt_count(n)),
            None => "artist".into(),
        },
        _ => e.sub.clone(),
    };
    ui.add(egui::Label::new(RichText::new(subtitle).font(theme::regular(14.0)).color(col::sub())).truncate());
    ui.add_space(sp(2.0));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = sp(1.5);
        let here = !app.pb.context_uri.is_empty() && app.pb.context_uri == e.uri;
        let playing = here && app.pb.is_playing;
        // As on the row: folders and Your Episodes only open.
        let playable = !matches!(e.special, Some(Route::Folder(_)) | Some(Route::YourEpisodes));
        if playable && widgets::bracket_button(ui, Some(if playing { Icon::Pause } else { Icon::Play }), if playing { "pause" } else { "play" }, true).clicked() {
            if here {
                app.act(Action::TogglePlay);
            } else {
                play(app, e, false);
            }
        }
        // Local Files has no shuffle (it plays in order).
        if !matches!(e.special, Some(Route::Folder(_)) | Some(Route::YourEpisodes) | Some(Route::LocalFiles)) && widgets::bracket_button(ui, Some(Icon::Shuffle), "shuffle", false).clicked() {
            play(app, e, true);
        }
        // As on their pages: artists aren't downloaded.
        if (e.special.is_none() && !e.uri.starts_with("spotify:artist:")) || matches!(e.special, Some(Route::LikedSongs) | Some(Route::YourEpisodes)) {
            super::pages::download_toggle(app, ui, &e.uri, &e.name, 16.0);
        }
        let more = icon_btn(ui, Icon::More, 16.0, "More options", false);
        let e2 = e.clone();
        egui::Popup::menu(&more).show(|ui| context_menu(app, ui, &e2));
    });

    let (label, featured) = featured(app, ui, e);
    let artists = featured.artists();
    if artists.is_empty() {
        return;
    }
    ui.add_space(sp(2.25));
    widgets::hairline(ui);
    ui.add_space(sp(2.0));
    widgets::caps_label(ui, label, 12.5, col::sub());
    ui.add_space(sp(1.0));
    let d = px(84.0);
    let gap = sp(5.0);
    let fit = ((ui.available_width() + gap) / (d + gap)).floor().max(1.0) as usize;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for a in artists.iter().take(fit) {
            let (r, resp) = ui.allocate_exact_size(vec2(d, d + px(30.0)), Sense::click());
            let c = Rect::from_min_size(r.min, vec2(d, d));
            paint_cover(ui, c, model::image(a, 160).as_deref(), true);
            if resp.hovered() {
                ui.painter().circle_stroke(c.center(), d / 2.0, Stroke::new(1.2, col::accent()));
            }
            let name = s(a, "name");
            let font = theme::regular(12.5);
            let galley = ui.painter().layout_no_wrap(name.to_string(), font.clone(), col::text());
            if galley.size().x <= d + gap * 0.8 {
                ui.painter().text(pos2(c.center().x, c.bottom() + px(16.0)), Align2::CENTER_CENTER, name, font, col::text());
            } else {
                widgets::text_trunc(ui, pos2(c.left() - gap * 0.4, c.bottom() + px(16.0)), name, font, col::text(), d + gap * 0.8);
            }
            if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(name).clicked() {
                app.act(Action::Navigate(Route::Artist(s(a, "id").into())));
            }
        }
    });
}
