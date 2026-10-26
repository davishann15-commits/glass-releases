//! The song table used by playlists, albums, Liked Songs, search and more.

use super::icons::{self, Icon};
use super::menus;
use super::theme::{self, col};
use super::widgets::{self, link, paint_cover};
use crate::app::{Action, App, Route, TrackSort};
use crate::model::{self, s};
use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use serde_json::Value;
use std::sync::Arc;

/// One table row, borrowing from the cached API response (no per-frame clones).
pub struct Row<'a> {
    pub item: &'a Value,
    pub added_at: &'a str,
}

#[derive(Default)]
pub struct Opts {
    pub album_col: bool,
    pub added_col: bool,
    pub cover: bool,
    pub track_numbers: bool,
    pub discs: bool,
    /// (playlist id, editable)
    pub playlist: Option<(String, bool)>,
    /// Context URI to play within (playlist/album/collection). None = play URIs.
    pub context: Option<String>,
    pub header: bool,
    pub limit: Option<usize>,
    /// j/k, ↑/↓ and Enter work without the column header.
    pub keys: bool,
    /// Scroll so the song that's playing is in view (for one frame).
    pub reveal_playing: bool,
}

pub fn rows_from_items(items: &[Value]) -> Vec<Row<'_>> {
    items.iter().map(|it| Row { item: model::playlist_entry(it), added_at: s(it, "added_at") }).filter(|r| !r.item.is_null()).collect()
}

pub fn rows_plain(items: &[Value]) -> Vec<Row<'_>> {
    items.iter().map(|it| Row { item: it, added_at: "" }).collect()
}

/// Order + filter rows according to the page controls. Cached between frames
/// (rows borrow from cached responses, so their addresses identify the data);
/// sorting thousands of songs every frame is what made long playlists slow.
fn visible_order(app: &App, ui: &Ui, rows: &[Row<'_>]) -> Arc<Vec<usize>> {
    let ident = |r: Option<&Row<'_>>| r.map(|r| r.item as *const Value as usize).unwrap_or(0);
    let key = egui::Id::new((ident(rows.first()), ident(rows.last()), rows.len(), &app.page_filter, app.page_sort, app.page_sort_desc));
    let slot = ui.id().with("track-order");
    if let Some((k, order)) = ui.ctx().data(|d| d.get_temp::<(egui::Id, Arc<Vec<usize>>)>(slot))
        && k == key
    {
        return order;
    }
    let q = app.page_filter.to_lowercase();
    let mut idx: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            if q.is_empty() {
                return true;
            }
            let it = rows[i].item;
            s(it, "name").to_lowercase().contains(&q) || model::artists_line(it).to_lowercase().contains(&q) || it.get("album").is_some_and(|a| s(a, "name").to_lowercase().contains(&q))
        })
        .collect();
    if app.page_sort != TrackSort::Custom {
        // (number, text): lengths sort as numbers; names and ISO dates as text.
        idx.sort_by_cached_key(|i| {
            let it = rows[*i].item;
            match app.page_sort {
                TrackSort::Title => (0, s(it, "name").to_lowercase()),
                TrackSort::Artist => (0, model::artists_line(it).to_lowercase()),
                TrackSort::Album => (0, it.get("album").map(|a| s(a, "name").to_lowercase()).unwrap_or_default()),
                TrackSort::Added => (0, rows[*i].added_at.to_string()),
                TrackSort::Duration => (model::u(it, "duration_ms"), String::new()),
                TrackSort::Custom => (0, String::new()),
            }
        });
        if app.page_sort_desc {
            idx.reverse();
        }
    }
    let order = Arc::new(idx);
    ui.ctx().data_mut(|d| d.insert_temp(slot, (key, order.clone())));
    order
}

/// A column header click: ascending, then descending, then back to the list's own order.
fn cycle_sort(app: &mut App, sort: TrackSort) {
    match (app.page_sort == sort, app.page_sort_desc) {
        (true, false) => app.page_sort_desc = true,
        (true, true) => (app.page_sort, app.page_sort_desc) = (TrackSort::Custom, false),
        (false, _) => (app.page_sort, app.page_sort_desc) = (sort, false),
    }
}

pub fn table(app: &mut App, ui: &mut Ui, rows: &[Row<'_>], opts: &Opts) {
    let order = visible_order(app, ui, rows);
    let order: &[usize] = match opts.limit {
        Some(n) => &order[..n.min(order.len())],
        None => &order,
    };
    let row_h = if app.compact_rows { 36.0 } else { 56.0 } * theme::px(1.0);
    let width = ui.available_width();
    let show_album = opts.album_col && width > 620.0;
    let show_added = opts.added_col && width > 820.0;
    // Column x positions, relative to the row's left edge.
    // Sizes follow the theme's font size (`k` is 1 at the default 13 pt).
    let k = theme::px(1.0);
    let col_num = 16.0 * k;
    let col_title = 52.0 * k;
    let right_w = 150.0 * k;
    let flex = width - col_title - right_w;
    let (title_w, album_x, album_w, added_x) = match (show_album, show_added) {
        (true, true) => (flex * 0.45, col_title + flex * 0.47, flex * 0.3, col_title + flex * 0.8),
        (true, false) => (flex * 0.55, col_title + flex * 0.58, flex * 0.4, 0.0),
        (false, true) => (flex * 0.7, 0.0, 0.0, col_title + flex * 0.75),
        _ => (flex, 0.0, 0.0, 0.0),
    };

    if opts.header {
        let (hr, _) = ui.allocate_exact_size(vec2(width, 32.0 * k), Sense::hover());
        let y = hr.center().y;
        let mut head = |x: f32, text: &str, sort: Option<TrackSort>| {
            let r = Rect::from_min_size(pos2(hr.left() + x, hr.top()), vec2(140.0 * k, 32.0 * k));
            if let Some(sort) = sort {
                let resp = ui.interact(r, ui.id().with(("hdr", text)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                if resp.clicked() {
                    cycle_sort(app, sort);
                }
                let arrow = if app.page_sort == sort { if app.page_sort_desc { " ▾" } else { " ▴" } } else { "" };
                let color = if app.page_sort == sort { col::accent() } else { col::sub() };
                let w = widgets::caps(ui, pos2(r.left(), y), text, 11.0, color);
                ui.painter().text(pos2(r.left() + w, y), Align2::LEFT_CENTER, arrow, theme::regular(11.0), color);
            } else {
                widgets::caps(ui, pos2(r.left(), y), text, 11.0, col::muted());
            }
        };
        head(col_num, "#", None);
        head(col_title, "Title", Some(TrackSort::Title));
        if show_album {
            head(album_x, "Album", Some(TrackSort::Album));
        }
        if show_added {
            head(added_x, "Date added", Some(TrackSort::Added));
        }
        let cr = Rect::from_center_size(pos2(hr.right() - 48.0 * k, y), vec2(16.0, 16.0) * k);
        let resp = ui.interact(cr.expand(4.0), ui.id().with("hdr-dur"), Sense::click()).on_hover_text("Sort by duration");
        if resp.clicked() {
            cycle_sort(app, TrackSort::Duration);
        }
        icons::paint(ui.painter(), cr, Icon::Clock, if app.page_sort == TrackSort::Duration { col::accent() } else { col::sub() });
        widgets::hairline(ui);
        ui.add_space(theme::sp(0.75));
    }

    let playing_uri = app.pb.uri().to_string();
    let is_playing = app.pb.is_playing;
    let all_uris = || -> Vec<String> { order.iter().map(|&i| s(rows[i].item, "uri").to_string()).collect() };
    if app.select_all_pending {
        app.select_all_pending = false;
        app.selection = all_uris();
    }

    // Where each row goes. Album pages put a "Disc 2" heading between discs.
    let stride = row_h + ui.spacing().item_spacing.y;
    let disc_h = 30.0 * k;
    // Only in the album's own order: sorted, the discs are mixed together.
    let multi_disc = opts.discs && app.page_sort == TrackSort::Custom && rows.iter().any(|r| model::u(r.item, "disc_number") > 1);
    let mut tops = Vec::with_capacity(order.len());
    let mut headings = Vec::new();
    let mut y = 0.0;
    let mut last_disc = 0;
    for &i in order {
        if multi_disc {
            let d = model::u(rows[i].item, "disc_number");
            if d != last_disc {
                headings.push((y, d));
                y += disc_h;
            }
            last_disc = d;
        }
        tops.push(y);
        y += stride;
    }
    // One allocation for the whole table; only rows on screen are laid out.
    let (table, _) = ui.allocate_exact_size(vec2(width, y.max(0.0)), Sense::hover());
    for (hy, d) in headings {
        let r = Rect::from_min_size(table.min + vec2(0.0, hy), vec2(width, disc_h));
        if ui.is_rect_visible(r) {
            ui.painter().text(pos2(r.left() + col_num, r.center().y), Align2::LEFT_CENTER, format!("Disc {d}"), theme::medium(13.0), col::sub());
        }
    }

    // Keyboard: j/k or ↑/↓ move a cursor, Enter plays, ⇧Enter queues.
    let keyboard = (opts.header || opts.keys) && !ui.ctx().egui_wants_keyboard_input() && !egui::Popup::is_any_open(ui.ctx()) && !order.is_empty();
    if keyboard {
        let last = order.len() - 1;
        let (down, up, enter, queue) = ui.input_mut(|i| {
            let down = i.consume_key(egui::Modifiers::NONE, egui::Key::J) || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            let up = i.consume_key(egui::Modifiers::NONE, egui::Key::K) || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            let queue = i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter);
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            (down, up, enter, queue)
        });
        let cur = app.row_cursor.map(|c| c.min(last));
        let moved = match (down, up, cur) {
            (true, _, None) | (_, true, None) => Some(0),
            (true, _, Some(c)) => Some((c + 1).min(last)),
            (_, true, Some(c)) => Some(c.saturating_sub(1)),
            _ => None,
        };
        if let Some(c) = moved {
            app.row_cursor = Some(c);
            let r = Rect::from_min_size(table.min + vec2(0.0, tops[c]), vec2(width, row_h));
            ui.scroll_to_rect(r.expand(row_h), None);
        }
        if let Some(c) = app.row_cursor.map(|c| c.min(last)) {
            let uri = s(rows[order[c]].item, "uri").to_string();
            if enter {
                play_row(app, opts, &all_uris(), c, rows[order[c]].item, uri == playing_uri);
            }
            if queue {
                let uris = if app.selection.len() > 1 { app.selection.clone() } else { vec![uri] };
                app.act(Action::Queue(uris));
            }
        }
    }

    if opts.reveal_playing
        && let Some(pos) = order.iter().position(|&i| !playing_uri.is_empty() && s(rows[i].item, "uri") == playing_uri)
    {
        ui.scroll_to_rect(Rect::from_min_size(table.min + vec2(0.0, tops[pos]), vec2(width, row_h)), Some(egui::Align::Center));
    }
    for (pos, &i) in order.iter().enumerate() {
        let rect = Rect::from_min_size(table.min + vec2(0.0, tops[pos]), vec2(width, row_h));
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let it = rows[i].item;
        let uri = s(it, "uri").to_string();
        let id = ui.id().with(("row", i, &uri));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s(it, "name")));
        let selected = app.selection.contains(&uri);
        let current = !uri.is_empty() && uri == playing_uri;
        // A playlist's local song plays when its file is in one of your folders.
        let file = if model::b(it, "is_local") { app.local_link(&uri) } else { None };
        let missing = matches!(file, Some(None));
        let unplayable = it.get("is_playable").and_then(Value::as_bool) == Some(false) || (!app.settings.allow_explicit && model::b(it, "explicit")) || missing;
        let menu_id = id.with("menu");
        if let Some(l) = &file
            && ui.rect_contains_pointer(rect)
        {
            let note = match l {
                Some(t) => format!("Local file: {}", t.path.display()),
                None if !app.share_peers.is_empty() => "This local song isn't on this computer. Right-click to get it from your other devices.".to_string(),
                None => "This local song isn't on this computer. Put the file in a folder listed under Local Files to play it here.".to_string(),
            };
            resp.clone().on_hover_text(note);
        }
        // From the pointer's position, not `resp.hovered()`: while a click is
        // in progress egui reports only the clicked widget as hovered, which
        // used to make the row's play/heart/⋯ buttons vanish mid-click.
        let hovered = ui.rect_contains_pointer(rect) || resp.context_menu_opened() || egui::Popup::is_id_open(ui.ctx(), menu_id);
        let fill = if selected {
            col::selection()
        } else if hovered {
            col::hover()
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, theme::small_radius(), fill);
        if app.row_cursor == Some(pos) && (opts.header || opts.keys) {
            ui.painter().rect_stroke(rect, theme::small_radius(), Stroke::new(1.0, col::focus()), egui::StrokeKind::Inside);
        }
        let dim = if unplayable { 0.45 } else { 1.0 };
        let text_color = if current { col::accent() } else { col::text().gamma_multiply(dim) };
        let sub_color = col::sub().gamma_multiply(dim);

        // Reordering in editable playlists (drag a row onto another).
        if let Some((pid, true)) = &opts.playlist
            && app.page_sort == TrackSort::Custom
            && app.page_filter.is_empty()
        {
            if resp.drag_started() {
                resp.dnd_set_drag_payload(i);
            }
            if let Some(from) = resp.dnd_hover_payload::<usize>()
                && *from != i
            {
                let y = if *from < i { rect.bottom() } else { rect.top() };
                ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(2.0, col::accent()));
            }
            if let Some(from) = resp.dnd_release_payload::<usize>()
                && *from != i
            {
                app.act(Action::MovePlaylistItem { playlist: pid.clone(), from: *from, to: i });
            }
        }

        // # / play / equalizer. The play icon is only a picture: a click
        // anywhere on the row (this spot included) plays it.
        let num_rect = Rect::from_center_size(pos2(rect.left() + col_num + 8.0 * k, rect.center().y), vec2(16.0, 16.0) * k);
        if hovered && !unplayable {
            icons::paint(ui.painter(), num_rect, if current && is_playing { Icon::Pause } else { Icon::Play }, col::text());
        } else if current && is_playing {
            widgets::equalizer_bars(ui, num_rect.shrink(1.0), col::accent());
        } else {
            let n = if opts.track_numbers { model::u(it, "track_number") as usize } else { pos + 1 };
            ui.painter().text(num_rect.center(), Align2::CENTER_CENTER, format!("{n:02}"), theme::regular(13.0), if current { col::accent() } else { col::muted() });
        }

        // Title cell
        let mut tx = rect.left() + col_title;
        if opts.cover && !app.compact_rows {
            let c = Rect::from_min_size(pos2(tx, rect.center().y - 20.0 * k), vec2(40.0, 40.0) * k);
            let img = match &file {
                Some(Some(t)) => t.cover_uri(),
                _ => model::image(it, 64),
            };
            paint_cover(ui, c, img.as_deref(), false);
            tx += 52.0 * k;
        }
        let title_rect = Rect::from_min_max(pos2(tx, rect.top()), pos2(rect.left() + col_title + title_w - 8.0 * k, rect.bottom()));
        ui.scope_builder(UiBuilder::new().max_rect(title_rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if app.compact_rows {
                ui.add_space(theme::sp(1.125));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(RichText::new(s(it, "name")).font(theme::regular(14.5)).color(text_color));
                    ui.label(RichText::new("  ·  ").color(sub_color));
                    widgets::artist_links(app, ui, it, 13.0, sub_color);
                });
            } else {
                ui.add_space(theme::sp(1.125));
                // Plain text, so a click on the title plays the row like the rest of it;
                // one line, cut short on a narrow page.
                ui.add(egui::Label::new(RichText::new(s(it, "name")).font(theme::regular(15.0)).color(text_color)).truncate());
                ui.horizontal(|ui| {
                    if model::b(it, "explicit") {
                        explicit_badge(ui);
                    }
                    widgets::artist_links(app, ui, it, 13.0, sub_color);
                });
            }
        });

        if show_album && let Some(album) = it.get("album").or_else(|| it.get("show")) {
            let r = Rect::from_min_max(pos2(rect.left() + album_x, rect.top()), pos2(rect.left() + album_x + album_w - 8.0, rect.bottom()));
            ui.scope_builder(UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                if link(ui, s(album, "name"), theme::regular(13.5), sub_color).clicked() {
                    let aid = s(album, "id").to_string();
                    app.act(Action::Navigate(if album.get("publisher").is_some() { Route::Show(aid) } else { Route::Album(aid) }));
                }
            });
        }
        if show_added && !rows[i].added_at.is_empty() {
            ui.painter().text(pos2(rect.left() + added_x, rect.center().y), Align2::LEFT_CENTER, model::fmt_date(rows[i].added_at), theme::regular(13.5), sub_color);
        }

        // Right cluster: heart, duration, more. Their buttons always exist;
        // only drawing them waits for the pointer.
        let heart_rect = Rect::from_center_size(pos2(rect.right() - 108.0 * k, rect.center().y), vec2(16.0, 16.0) * k);
        let saved = app.is_saved(&uri);
        let liked = saved == Some(true);
        let hr = ui.interact(heart_rect.expand(6.0), id.with("heart"), Sense::click());
        if hovered || liked {
            let tip = match (s(it, "type") == "episode", liked) {
                (true, true) => "Remove from Your Episodes",
                (true, false) => "Save to Your Episodes",
                (false, true) => "Remove from Liked Songs",
                (false, false) => "Save to Liked Songs",
            };
            let hr = hr.clone().on_hover_text(tip);
            let color = if liked {
                col::accent()
            } else if hr.hovered() {
                col::text()
            } else {
                col::sub()
            };
            icons::paint(ui.painter(), heart_rect, if liked { Icon::HeartFilled } else { Icon::Heart }, color);
        }
        if hr.clicked() {
            app.act(Action::Save { uri: uri.clone(), save: !liked });
        }
        let dur = model::u(it, "duration_ms");
        ui.painter().text(pos2(rect.right() - 48.0 * k, rect.center().y), Align2::RIGHT_CENTER, model::fmt_ms(dur), theme::regular(13.5), sub_color);
        let mr = Rect::from_center_size(pos2(rect.right() - 22.0 * k, rect.center().y), vec2(16.0, 16.0) * k);
        let more = ui.interact(mr.expand(6.0), id.with("more"), Sense::click());
        if hovered {
            icons::paint(ui.painter(), mr, Icon::More, if more.hovered() { col::text() } else { col::sub() });
        }
        let pl = opts.playlist.as_ref().map(|(p, e)| (p.as_str(), *e));
        egui::Popup::menu(&more).id(menu_id).show(|ui| {
            let selection = app.selection.clone();
            menus::track(app, ui, it, pl, &selection)
        });

        // Selection and playback
        if resp.clicked() {
            let cmd = ui.input(|i| i.modifiers.command);
            let shift = ui.input(|i| i.modifiers.shift);
            app.row_cursor = Some(pos);
            if let Some(anchor) = app.sel_anchor.filter(|_| shift) {
                let all = all_uris();
                let a = anchor.min(all.len().saturating_sub(1));
                let (lo, hi) = if a < pos { (a, pos) } else { (pos, a) };
                app.selection = all[lo..=hi].to_vec();
            } else if cmd {
                if selected {
                    app.selection.retain(|u| u != &uri);
                } else {
                    app.selection.push(uri.clone());
                }
                app.sel_anchor = Some(pos);
            } else {
                app.selection = vec![uri.clone()];
                app.sel_anchor = Some(pos);
                // A plain click plays the song (or pauses it if it's the one playing). The
                // second click of a double-click would undo the first.
                if !unplayable && !resp.double_clicked() {
                    play_row(app, opts, &all_uris(), pos, it, current);
                }
            }
        }
        resp.context_menu(|ui| {
            let selection = app.selection.clone();
            menus::track(app, ui, it, pl, &selection)
        });
    }
    if order.is_empty() && !app.page_filter.is_empty() {
        ui.add_space(theme::sp(2.5));
        widgets::subtle(ui, format!("No results for “{}”", app.page_filter));
    }
}

fn play_row(app: &mut App, opts: &Opts, all: &[String], pos: usize, item: &Value, current: bool) {
    if current {
        app.act(Action::TogglePlay);
        return;
    }
    // The bar shows the song at once, before Spotify has a word to say.
    app.play_hint = Some(item.clone());
    let uri = s(item, "uri");
    match (&opts.context, app.page_sort == crate::app::TrackSort::Custom && app.page_filter.is_empty()) {
        (Some(ctx), true) => app.act(Action::PlayContext { context: ctx.clone(), track: Some(uri.to_string()) }),
        _ => app.act(Action::PlayUris { uris: all.to_vec(), index: pos }),
    }
}

pub fn explicit_badge(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(15.0, 15.0) * theme::px(1.0), Sense::hover());
    ui.painter().rect_stroke(r, CornerRadius::same(2), egui::Stroke::new(1.0, col::muted()), egui::StrokeKind::Inside);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, "E", theme::regular(9.5), col::sub());
    ui.add_space(theme::sp(0.5));
}

/// The search-in-page, sort and density controls above a table.
pub fn controls(app: &mut App, ui: &mut Ui, sortable: bool) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.menu_button(RichText::new(format!("{}  ⌄", sort_label(app).to_lowercase())).font(theme::regular(12.5)).color(col::sub()), |ui| {
            if sortable {
                ui.label(RichText::new("Sort by").size(12.0).color(col::sub()));
                for (sort, name) in [
                    (TrackSort::Custom, "Custom order"),
                    (TrackSort::Title, "Title"),
                    (TrackSort::Artist, "Artist"),
                    (TrackSort::Album, "Album"),
                    (TrackSort::Added, "Date added"),
                    (TrackSort::Duration, "Duration"),
                ] {
                    if ui.selectable_label(app.page_sort == sort, name).clicked() {
                        if app.page_sort == sort && sort != TrackSort::Custom {
                            app.page_sort_desc = !app.page_sort_desc;
                        } else {
                            app.page_sort = sort;
                            app.page_sort_desc = false;
                        }
                    }
                }
                ui.separator();
            }
            ui.label(RichText::new("View as").size(12.0).color(col::sub()));
            if ui.selectable_label(!app.compact_rows, "List").clicked() {
                app.compact_rows = false;
            }
            if ui.selectable_label(app.compact_rows, "Compact").clicked() {
                app.compact_rows = true;
            }
        })
        .response
        .on_hover_text("Sort and view");
        let te = egui::TextEdit::singleline(&mut app.page_filter).hint_text(RichText::new("filter list").color(col::muted())).font(theme::regular(12.5)).desired_width(theme::px(160.0));
        let r = ui.add(te);
        if app.focus_search && app.route != Route::Search {
            r.request_focus();
            app.focus_search = false;
        }
    });
}

fn sort_label(app: &App) -> &'static str {
    match app.page_sort {
        TrackSort::Custom => "Custom order",
        TrackSort::Title => "Title",
        TrackSort::Artist => "Artist",
        TrackSort::Album => "Album",
        TrackSort::Added => "Date added",
        TrackSort::Duration => "Duration",
    }
}
