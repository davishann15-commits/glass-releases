//! The now-playing screen: artwork and controls on the left; on the right
//! the lyrics (L), the queue (Q), the playlist that's playing (P) or devices,
//! with the audio visualizer (V) under them.

use super::icons::Icon;
use super::menus;
use super::nowbar;
use super::right;
use super::theme::{self, col, px, sp};
use super::tracklist;
use super::visualizer;
use super::widgets::{self, icon_btn, paint_cover, play_btn};
use crate::app::{Action, App, NpView, Route};
use crate::lyrics::service::{ProviderSetting, Resolved, Status};
use crate::lyrics::translate::{Fix, Mode, Translation};
use crate::lyrics::{self, Candidate, Line, Lyrics, Match, Outcome, ProviderId, Timing, TrackRef};
use crate::model::{self, s};
use eframe::egui::{self, Align2, Color32, CursorIcon, Id, Rect, RichText, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use egui::text::{LayoutJob, TextFormat};
use serde_json::Value;

pub fn show(app: &mut App, ui: &mut Ui, area: Rect) {
    let pad = sp(4.5);
    let left_w = (area.width() * 0.36).clamp(px(280.0), px(520.0));
    let left = Rect::from_min_max(pos2(area.left() + pad, area.top() + sp(2.5)), pos2(area.left() + pad + left_w, area.bottom() - sp(2.0)));
    let divider_x = left.right() + sp(6.0);
    let right = Rect::from_min_max(pos2(divider_x + sp(6.0), area.top() + sp(2.5)), pos2(area.right() - sp(3.5), area.bottom() - sp(2.0)));
    ui.painter().line_segment([pos2(divider_x, area.top() + sp(4.0)), pos2(divider_x, area.bottom() - sp(3.0))], theme::hairline());
    ui.scope_builder(UiBuilder::new().max_rect(left).id_salt("np-left"), |ui| controls(app, ui, left));
    if app.np_view != NpView::Playlist {
        // Opening the playlist view again scrolls to the song that's playing.
        ui.ctx().data_mut(|d| {
            d.remove::<String>(Id::new("np-playlist-shown"));
            d.remove::<bool>(Id::new("np-playlist-local-shown"));
        });
    }
    // The visualizer takes the bottom of the right side, the view sits above
    // it, and the lyrics' footer (source, "wrong lyrics?", timing) goes last.
    let footer_h = px(28.0);
    let (view, viz) = if app.settings.visualizer {
        let h = (right.height() * theme::viz_style().height).max(px(64.0));
        let viz = Rect::from_min_max(pos2(right.left(), right.bottom() - footer_h - sp(1.0) - h), pos2(right.right(), right.bottom() - footer_h - sp(1.0)));
        (Rect::from_min_max(right.min, pos2(right.right(), viz.top() - sp(2.0))), Some(viz))
    } else {
        (right, None)
    };
    let footer = Rect::from_min_max(pos2(right.left(), right.bottom() - footer_h + px(6.0)), right.max);
    ui.scope_builder(UiBuilder::new().max_rect(view).id_salt("np-right"), |ui| match app.np_view {
        NpView::Queue => side_list(app, ui, "queue / up next", right::queue),
        NpView::Devices => side_list(app, ui, "devices", right::devices),
        NpView::Playlist => playlist_view(app, ui, view),
        NpView::Lyrics => lyrics_view(app, ui, view, footer),
    });
    if let Some(r) = viz {
        visualizer::show(app, ui, r);
    }
}

/// P: the playlist, album or list that's playing, to pick any song from it.
fn playlist_view(app: &mut App, ui: &mut Ui, rect: Rect) {
    // A local file: the list it's playing from.
    if app.local_file_active() {
        let Some(local) = app.local.as_ref() else { return };
        let (list, index) = (&local.queue, local.index);
        header(ui, rect, "playing from / local files", None);
        ui.add_space(px(40.0));
        let mut play = None;
        ScrollArea::vertical().id_salt("np-playlist-local").auto_shrink([false, false]).show(ui, |ui| {
            for (i, t) in list.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), px(44.0)), Sense::click());
                let on = i == index;
                // Rows out of sight only take their space (the whole library may be queued).
                if !on && !ui.is_rect_visible(r) {
                    continue;
                }
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} — {}", t.title, t.artist)));
                if resp.hovered() {
                    ui.painter().rect_filled(r, theme::small_radius(), col::hover());
                }
                let color = if on { col::accent() } else { col::text() };
                ui.painter().text(pos2(r.left() + sp(1.0), r.center().y), Align2::LEFT_CENTER, format!("{:02}", i + 1), theme::regular(12.0), col::muted());
                widgets::text_trunc(ui, pos2(r.left() + px(40.0), r.center().y - px(8.0)), &t.title, theme::regular(14.0), color, r.width() - px(100.0));
                widgets::text_trunc(ui, pos2(r.left() + px(40.0), r.center().y + px(9.0)), &t.artist, theme::regular(12.0), col::sub(), r.width() - px(100.0));
                ui.painter().text(pos2(r.right() - sp(1.0), r.center().y), Align2::RIGHT_CENTER, model::fmt_ms(t.duration_ms), theme::regular(12.0), col::sub());
                if on && !ui.ctx().data(|d| d.get_temp::<bool>(Id::new("np-playlist-local-shown")).unwrap_or(false)) {
                    ui.scroll_to_rect(r, Some(egui::Align::Center));
                    ui.ctx().data_mut(|d| d.insert_temp(Id::new("np-playlist-local-shown"), true));
                }
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    play = Some(i);
                }
            }
        });
        if let Some(i) = play {
            let list = local.queue.clone();
            app.play_local_list(list, i);
        }
        return;
    }
    let ctx = app.pb.context_uri.clone();
    let mut parts = ctx.split(':');
    let kind = parts.nth(1).unwrap_or("");
    let id = ctx.rsplit(':').next().unwrap_or("").to_string();
    // The list, borrowed from the page cache (it can be thousands of songs);
    // an album's songs are given their album (once per answer, see `album_tracks`).
    let none = || std::sync::Arc::new(Vec::new());
    let (name, data, album_tracks): (String, Option<std::sync::Arc<Value>>, std::sync::Arc<Vec<Value>>) = if ctx.ends_with(":collection") {
        ("Liked Songs".into(), app.want_all("/me/tracks?limit=50", 10_000), none())
    } else {
        match kind {
            "playlist" => (app.want(&format!("/playlists/{id}")).map(|p| s(&p, "name").to_string()).unwrap_or_default(), app.want_all(&format!("/playlists/{id}/tracks?limit=100"), 10_000), none()),
            "album" => match app.want(&format!("/albums/{id}")) {
                Some(a) => {
                    let tracks = super::pages::album_tracks(ui, &a, None);
                    (s(&a, "name").to_string(), Some(a.clone()), tracks)
                }
                None => (String::new(), None, none()),
            },
            "artist" => (app.want(&format!("/artists/{id}")).map(|a| s(&a, "name").to_string()).unwrap_or_default(), app.want(&format!("/artists/{id}/top-tracks?market=from_token")), none()),
            _ => (String::new(), None, none()),
        }
    };
    if ctx.is_empty() || (kind != "playlist" && kind != "album" && kind != "artist" && !ctx.ends_with(":collection")) {
        header(ui, rect, "playing from", None);
        return message(app, ui, rect, "not from a playlist", "This music isn't playing from a playlist, album, artist or Liked Songs. The queue (Q) shows what's next.", |app, ui| {
            if widgets::bracket_button(ui, None, "show the queue", false).clicked() {
                app.np_view = NpView::Queue;
            }
        });
    }
    let route = match kind {
        "playlist" => Some(Route::Playlist(id.clone())),
        "album" => Some(Route::Album(id.clone())),
        "artist" => Some(Route::Artist(id.clone())),
        _ if ctx.ends_with(":collection") => Some(Route::LikedSongs),
        _ => None,
    };
    // "playing from  <name>" — the name opens the page.
    let title = if name.is_empty() { "…".to_string() } else { name };
    let tr = Rect::from_min_size(rect.min, vec2(rect.width(), px(28.0)));
    ui.scope_builder(UiBuilder::new().max_rect(tr).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.label(RichText::new("playing from").font(theme::regular(13.0)).color(col::sub()));
        ui.add_space(sp(1.0));
        if widgets::link(ui, &title, theme::regular(15.0), col::text()).on_hover_text("Open").clicked()
            && let Some(r) = route
        {
            app.act(Action::Navigate(r));
        }
    });
    ui.add_space(sp(1.5));
    let Some(data) = data else {
        widgets::loading(ui);
        return;
    };
    let rows = match kind {
        "album" => tracklist::rows_plain(&album_tracks),
        "artist" => tracklist::rows_plain(model::arr(&data, "tracks")),
        _ => tracklist::rows_from_items(model::items(&data)),
    };
    // This list ignores the filter and sort of whatever page was open last.
    let saved = (std::mem::take(&mut app.page_filter), app.page_sort, app.page_sort_desc);
    app.page_sort = crate::app::TrackSort::Custom;
    app.page_sort_desc = false;
    let first = !ui.ctx().data(|d| d.get_temp::<String>(Id::new("np-playlist-shown")).is_some_and(|c| c == ctx));
    ScrollArea::vertical().id_salt(("np-playlist", &ctx)).auto_shrink([false, false]).show(ui, |ui| {
        tracklist::table(app, ui, &rows, &tracklist::Opts { cover: true, context: Some(ctx.clone()), keys: true, reveal_playing: first, ..Default::default() });
    });
    app.page_filter = saved.0;
    app.page_sort = saved.1;
    app.page_sort_desc = saved.2;
    ui.ctx().data_mut(|d| d.insert_temp(Id::new("np-playlist-shown"), ctx));
}

fn controls(app: &mut App, ui: &mut Ui, rect: Rect) {
    let blink_on = theme::reduce_motion() || !ui.input(|i| i.focused) || (ui.input(|i| i.time) % 1.2) < 0.7;
    ui.label(RichText::new(format!("now playing{}", if blink_on { "_" } else { " " })).font(theme::regular(14.0)).color(col::sub()));
    ui.add_space(sp(1.75));
    let item = app.pb.item.clone();
    let local = app.local.as_ref().and_then(|l| l.current().cloned());
    // Everything under the artwork needs about this much room.
    let below = px(310.0);
    let art = rect.width().min(ui.available_height() - below).max(px(120.0));
    // Centered in the column (it's narrower than the column when the window is short).
    let (row, _) = ui.allocate_exact_size(vec2(rect.width(), art), Sense::hover());
    let ar = Rect::from_center_size(row.center(), vec2(art, art));
    let aresp = ui.interact(ar, Id::new("np-artwork"), Sense::click());
    let img = match (&local, &item) {
        (Some(t), _) => t.cover_uri(),
        (None, Some(i)) => app.cover_of(i, 640),
        _ => None,
    };
    // With album colors, songs change with a crossfade, and the colors bloom from the artwork.
    let fade = if app.settings.album_colors {
        app.album_colors.set_origin(ui.ctx().content_rect(), ar.center());
        widgets::paint_cover_fading(ui, Id::new("np-cover"), ar, img.as_deref(), false)
    } else {
        paint_cover(ui, ar, img.as_deref(), false);
        1.0
    };
    if let Some(a) = item.as_ref().and_then(|i| i.get("album")).filter(|a| !s(a, "id").is_empty())
        && aresp.on_hover_text("Go to album").on_hover_cursor(CursorIcon::PointingHand).clicked()
    {
        app.act(Action::Navigate(Route::Album(s(a, "id").into())));
    }
    ui.add_space(sp(2.25));

    // Title, artists, like and more.
    let (title, sub) = match (&item, &local) {
        (_, Some(t)) => (t.title.clone(), t.artist.clone()),
        (Some(it), _) => (s(it, "name").to_string(), model::artists_line(it)),
        _ => ("nothing playing".to_string(), "pick something from your library".to_string()),
    };
    ui.scope(|ui| {
        // The new title fades in with its cover.
        ui.multiply_opacity(fade);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(rect.width() - px(84.0));
                ui.add(egui::Label::new(RichText::new(&title).font(theme::regular(26.0)).color(col::text())).truncate());
                match &item {
                    Some(it) if local.is_none() => {
                        ui.horizontal(|ui| widgets::artist_links(app, ui, it, 15.0, col::sub()));
                    }
                    _ => {
                        ui.label(RichText::new(&sub).font(theme::regular(15.0)).color(col::sub()));
                    }
                }
            });
            if let Some(it) = item.clone().filter(|_| local.is_none()) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let more = icon_btn(ui, Icon::More, 18.0, "More options", false);
                    egui::Popup::menu(&more).show(|ui| menus::track(app, ui, &it, None, &[]));
                    let uri = s(&it, "uri").to_string();
                    let saved = app.is_saved(&uri) == Some(true);
                    if icon_btn(ui, if saved { Icon::HeartFilled } else { Icon::Heart }, 19.0, if saved { "Remove from Liked Songs" } else { "Save to Liked Songs" }, saved).clicked() {
                        app.act(Action::Save { uri, save: !saved });
                    }
                });
            }
        });
    });
    ui.add_space(sp(2.25));

    // Progress.
    let (pos, dur) = match &local {
        Some(t) => (app.local.as_ref().map(|l| l.position_ms()).unwrap_or(0), t.duration_ms),
        None => (app.pb.position(), item.as_ref().map(|i| model::u(i, "duration_ms")).unwrap_or(0)),
    };
    let (resp, new) = nowbar::rail(ui, Id::new("np-seek"), rect.width(), if dur > 0 { pos as f32 / dur as f32 } else { 0.0 });
    if let Some(f) = new {
        if resp.drag_stopped() || resp.clicked() {
            app.scrub_ms = None;
            app.act(Action::Seek((f * dur as f32) as u64));
        } else if resp.dragged() {
            // The lyrics follow the bar while it's held.
            app.scrub_ms = Some(((f * dur as f32) as u64, std::time::Instant::now()));
        }
    }
    let (tr, _) = ui.allocate_exact_size(vec2(rect.width(), px(18.0)), Sense::hover());
    ui.painter().text(tr.left_center(), Align2::LEFT_CENTER, model::fmt_ms(pos), theme::regular(12.0), col::sub());
    ui.painter().text(tr.right_center(), Align2::RIGHT_CENTER, model::fmt_ms(dur), theme::regular(12.0), col::sub());
    ui.add_space(sp(2.0));

    // Transport.
    let playing = if local.is_some() { app.local.as_ref().is_some_and(|l| l.is_playing()) } else { app.pb.is_playing };
    let (row, _) = ui.allocate_exact_size(vec2(rect.width(), px(76.0)), Sense::hover());
    let slot = row.width() / 5.0;
    let at = |i: usize| pos2(row.left() + slot * (i as f32 + 0.5), row.center().y);
    let button = |ui: &mut Ui, i: usize, icon: Icon, tip: &str, on: bool, size: f32| -> bool {
        let r = Rect::from_center_size(at(i), vec2(px(40.0), px(40.0)));
        ui.scope_builder(UiBuilder::new().max_rect(r), |ui| icon_btn(ui, icon, size, tip, on).clicked()).inner
    };
    if button(ui, 0, Icon::Shuffle, "Shuffle  ⌘ S", app.pb.shuffle, 18.0) {
        let sh = !app.pb.shuffle;
        app.act(Action::SetShuffle(sh));
    }
    if button(ui, 1, Icon::Prev, "Previous  ⌘ ←", false, 22.0) {
        app.act(Action::Prev);
    }
    let pr = Rect::from_center_size(at(2), vec2(px(72.0), px(72.0)));
    if ui.scope_builder(UiBuilder::new().max_rect(pr), |ui| play_btn(ui, playing, 72.0).clicked()).inner {
        app.act(Action::TogglePlay);
    }
    if button(ui, 3, Icon::Next, "Next  ⌘ →", false, 22.0) {
        app.act(Action::Next);
    }
    let (icon, on) = match app.pb.repeat.as_str() {
        "context" => (Icon::Repeat, true),
        "track" => (Icon::RepeatOne, true),
        _ => (Icon::Repeat, false),
    };
    if button(ui, 4, icon, "Repeat  ⌘ R", on, 18.0) {
        app.act(Action::CycleRepeat);
    }
    ui.add_space(sp(3.0));

    // Volume and devices.
    ui.horizontal(|ui| {
        let v = app.pb.volume;
        let vi = if v <= 0.001 {
            Icon::VolumeMute
        } else if v < 0.5 {
            Icon::VolumeLow
        } else {
            Icon::VolumeHigh
        };
        if icon_btn(ui, vi, 18.0, if v <= 0.001 { "Unmute" } else { "Mute" }, false).clicked() {
            app.act(Action::ToggleMute);
        }
        let w = ui.available_width() - px(56.0);
        let (resp, new) = nowbar::rail(ui, Id::new("np-vol"), w, v);
        if let Some(v) = new {
            app.pb.volume_dragging = true;
            app.muted_volume = None;
            app.act(Action::Volume(v));
        }
        if resp.drag_stopped() || resp.clicked() {
            app.pb.volume_dragging = false;
            app.act(Action::VolumeCommit);
        }
        if icon_btn(ui, Icon::Devices, 18.0, "Devices  D", app.np_view == NpView::Devices).clicked() {
            app.np_view = if app.np_view == NpView::Devices { NpView::Lyrics } else { NpView::Devices };
        }
    });
}

fn side_list(app: &mut App, ui: &mut Ui, title: &str, body: impl FnOnce(&mut App, &mut Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(title).font(theme::regular(13.0)).color(col::sub()));
        });
    });
    ui.add_space(sp(2.0));
    ScrollArea::vertical().id_salt(("np-side", app.np_view)).auto_shrink([false, false]).show(ui, |ui| body(app, ui));
}

// ------------------------------------------------------------------ lyrics

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// The synced lyrics' layout facts, worked out once per song, width and font.
struct Measured {
    /// "translated": the translation in each line's place.
    translated: Option<Vec<Line>>,
    plain: Vec<f32>,
    bold: Vec<f32>,
    /// The backing vocals under each line, when it has them.
    back: Vec<Option<f32>>,
    /// The translation under each line ("original + translation").
    trans: Vec<Option<f32>>,
    /// Right-to-left text.
    rtl: Vec<bool>,
    /// Laid out piece by piece, for word or syllable timing (see `wipe`).
    tagged: Vec<bool>,
    /// Each line's alignment: the theme's, mirrored for right-to-left text and
    /// for a duet's other voice.
    align: Vec<egui::Align>,
}

/// `a` toward `b` by `t`, translucent colors included (Color32 is premultiplied,
/// so its channels mix as they are).
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// How bright a color looks (0 dark – 1 light), for picking the brighter of two.
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// The color of the word being sung: the theme's highlight when it stands out
/// from the line's own color; otherwise (a dark highlight, or album colors close
/// to the line's) the line's color lifted most of the way toward the text color,
/// so the word being sung always shows.
fn sung_color(highlight: Color32, current: Color32, text: Color32) -> Color32 {
    let (h, c, t) = (luminance(highlight), luminance(current), luminance(text));
    // "Stands out": noticeably further from the line's color, toward the text's side.
    if (h - c) * (t - c).signum() > 0.12 { highlight } else { lerp_color(current, text, 0.7) }
}

/// A piece's index hides in its glyphs' color when a timed line is laid out,
/// so `wipe` can find each piece's glyphs without laying the line out again.
const TAG: u8 = 0xA5;

fn tag(k: usize) -> Color32 {
    Color32::from_rgb((k & 0xFF) as u8, ((k >> 8) & 0xFF) as u8, TAG)
}

fn untag(c: Color32) -> Option<usize> {
    (c.b() == TAG && c.a() == 255).then(|| c.r() as usize | (c.g() as usize) << 8)
}

fn mirror(a: egui::Align) -> egui::Align {
    match a {
        egui::Align::LEFT => egui::Align::RIGHT,
        egui::Align::RIGHT => egui::Align::LEFT,
        c => c,
    }
}

/// One line laid out. Untimed lines are one piece of text (colored when
/// painted); timed ones (`tagged`) a section per word or syllable, tagged
/// with its index. A right-to-left line's pieces go in visual order: each
/// is shaped right to left, and egui lays sections out left to right.
fn line_job(text: &str, tokens: &[lyrics::Token], font: egui::FontId, wrap: f32, align: egui::Align, rtl: bool, tagged: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    job.halign = align;
    let format = |color| TextFormat { font_id: font.clone(), color, ..Default::default() };
    if tagged && !tokens.is_empty() {
        let order: Vec<usize> = if rtl { (0..tokens.len()).rev().collect() } else { (0..tokens.len()).collect() };
        for k in order {
            job.append(&tokens[k].text, 0.0, format(tag(k)));
        }
    } else {
        job.append(if text.is_empty() { "♪" } else { text }, 0.0, format(Color32::WHITE));
    }
    job
}

/// `g` (a tagged line) with each piece's sung share swept in, the way the
/// text reads: `progress[k]` is piece k's share sung (0–1), `lit[k]` its
/// color once sung, `waiting` the color of what's still to come. The edge is
/// a soft band about `band` wide, so the sweep reads as one motion even at a
/// modest frame rate. With Reduce Motion (`band` 0) nothing moves: each piece
/// brightens in place over its length instead. Only vertex colors change:
/// the layout is reused.
fn wipe(g: &std::sync::Arc<egui::Galley>, progress: &[f32], lit: &[Color32], waiting: Color32, rtl: bool, band: f32) -> std::sync::Arc<egui::Galley> {
    let mut out = (**g).clone();
    for placed in &mut out.rows {
        let mut row = (*placed.row).clone();
        let range = row.visuals.glyph_vertex_range.clone();
        // Each piece's extent on this row.
        let mut extent: Vec<(usize, f32, f32)> = Vec::new();
        for v in &row.visuals.mesh.vertices[range.clone()] {
            if let Some(k) = untag(v.color) {
                match extent.iter_mut().find(|e| e.0 == k) {
                    Some(e) => {
                        e.1 = e.1.min(v.pos.x);
                        e.2 = e.2.max(v.pos.x);
                    }
                    None => extent.push((k, v.pos.x, v.pos.x)),
                }
            }
        }
        for v in &mut row.visuals.mesh.vertices[range] {
            let Some(k) = untag(v.color) else { continue };
            let p = progress.get(k).copied().unwrap_or(0.0);
            let lit_k = lit.get(k).copied().unwrap_or(waiting);
            v.color = if p <= 0.0 {
                waiting
            } else if p >= 1.0 {
                lit_k
            } else if band <= 0.0 {
                // Lit the moment it starts, fully by its end.
                mix(waiting, lit_k, 0.6 + p * 0.4)
            } else {
                let &(_, x0, x1) = extent.iter().find(|e| e.0 == k).expect("measured above");
                let b = band.min((x1 - x0) * 0.6).max(1.0);
                // The edge runs from half a band before the piece to half a band after it.
                let edge = if rtl { x1 + b / 2.0 - p * (x1 - x0 + b) } else { x0 - b / 2.0 + p * (x1 - x0 + b) };
                let sung = if rtl { v.pos.x - edge } else { edge - v.pos.x };
                mix(waiting, lit_k, sung / b + 0.5)
            };
        }
        placed.row = std::sync::Arc::new(row);
    }
    std::sync::Arc::new(out)
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Overshoots a little, then settles: the "pop" animation.
fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    let c = 1.4;
    1.0 + (c + 1.0) * t * t * t + c * t * t
}

/// The scroll position easing from one line to the next. It animates only
/// when `line` changes; when the layout moves for another reason (fonts
/// arriving, a resize, a translation appearing) the text simply follows.
/// Returns (where it started, where it's going, progress 0–1).
fn tween(ui: &Ui, id: Id, line: i64, target: f32, secs: f32, ease: fn(f32) -> f32) -> (f32, f32, f32) {
    let now = ui.input(|i| i.time);
    let progress = |started: f64| if secs > 0.0 { ((now - started) as f32 / secs).clamp(0.0, 1.0) } else { 1.0 };
    // (from, to, started, line)
    let state: Option<(f32, f32, f64, i64)> = ui.ctx().data(|d| d.get_temp(id));
    let next = match state {
        None => (target, target, now - secs as f64, line),
        // Same line: shift with the layout, keeping any change in progress.
        Some((from, to, started, l)) if l == line => (from + (target - to), target, started, line),
        // A new line: start from wherever the last change had got to.
        Some((from, to, started, _)) => (from + (to - from) * ease(progress(started)), target, now, line),
    };
    ui.ctx().data_mut(|d| d.insert_temp(id, next));
    let p = progress(next.2);
    if p < 1.0 {
        ui.ctx().request_repaint();
    }
    (next.0, next.1, p)
}

/// Top left: what the translation is doing, or where the one showing came from.
fn translation_note(app: &mut App, ui: &mut Ui, rect: Rect, state: Option<Option<&Translation>>, lyrics: &Lyrics) {
    // Room left of the "lyrics / line sync" header.
    let header_w = ui.painter().layout_no_wrap(format!("lyrics / {}", lyrics.timing.label()), theme::regular(13.0), col::sub()).size().x
        + lyrics.named_by().iter().map(|c| ui.painter().layout_no_wrap(format!("{} · ", c.text), theme::regular(12.0), col::sub()).size().x).sum::<f32>();
    use crate::lyrics::translate::language_name;
    let Some(state) = state else { return };
    let (text, fix, retry) = match state {
        None => ("translating…".to_string(), None, false),
        Some(Translation::Ready { from, by, .. }) => {
            let from = if from.is_empty() { lyrics.language.clone().unwrap_or_default() } else { from.clone() };
            (if from.is_empty() { format!("translated by {by}") } else { format!("from {} · {by}", language_name(&from)) }, None, false)
        }
        Some(Translation::Same { lang }) => (format!("already in {}", language_name(lang)), None, false),
        Some(Translation::Unavailable { why, fix }) => (why.clone(), *fix, false),
        Some(Translation::Failed(e)) => (format!("translation failed: {e}"), None, true),
    };
    let band = Rect::from_min_size(rect.min, vec2((rect.width() - header_w - sp(3.0)).max(px(80.0)), px(28.0)));
    ui.scope_builder(UiBuilder::new().max_rect(band).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        let font = theme::regular(11.5);
        let action = match (fix, retry) {
            (Some(Fix::DownloadLanguage), _) => Some("open Language & Region"),
            (Some(Fix::TurnOnAssistant), _) => Some("assistant settings"),
            (None, true) => Some("try again"),
            _ => None,
        };
        let room = ui.available_width() - if action.is_some() { px(150.0) } else { 0.0 };
        ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(ui.cursor().min, vec2(room.max(px(60.0)), px(28.0)))), |ui| {
            ui.add(egui::Label::new(RichText::new(&text).font(font.clone()).color(col::muted())).truncate()).on_hover_text(&text);
        });
        if let Some(a) = action
            && widgets::link(ui, a, font, col::sub()).clicked()
        {
            match fix {
                Some(Fix::DownloadLanguage) => {
                    let _ = open::that("x-apple.systempreferences:com.apple.Localization-Settings.extension");
                }
                Some(Fix::TurnOnAssistant) => {
                    app.settings_query = "assistant".into();
                    app.act(Action::Navigate(Route::Settings));
                }
                None => app.translation.key.clear(),
            }
        }
    });
}

/// `lyrics / line sync` at the top right, with an explanation on hover.
fn header(ui: &mut Ui, rect: Rect, text: &str, tip: Option<&str>) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::regular(13.0), col::sub());
    let r = Rect::from_min_size(pos2(rect.right() - galley.size().x, rect.top() + px(14.0) - galley.size().y / 2.0), galley.size());
    ui.painter().galley(r.min, galley, col::sub());
    if let Some(tip) = tip {
        ui.interact(r, Id::new("lyrics-header"), Sense::hover()).on_hover_text(tip);
    }
}

/// Where the lyrics are from, left of the header at the top right: "lrc.red ·
/// lyrics / word sync". Services that ask for credit get it as a link (lrc.red,
/// Spicy Lyrics' syncers); the others are named ("netease · lyrics / word sync").
/// Returns how wide the header and credits are together.
fn credits(ui: &mut Ui, rect: Rect, lyrics: &Lyrics, header_w: f32) -> f32 {
    let font = theme::regular(12.0);
    let sep = " · ";
    let mut x = rect.right() - header_w;
    let y = rect.top() + px(14.0);
    for c in lyrics.named_by().iter().rev() {
        let g = ui.painter().layout_no_wrap(sep.to_string(), font.clone(), col::muted());
        x -= g.size().x;
        ui.painter().galley(pos2(x, y - g.size().y / 2.0), g, col::muted());
        let g = ui.painter().layout_no_wrap(c.text.clone(), font.clone(), col::sub());
        x -= g.size().x;
        let r = Rect::from_min_size(pos2(x, y - g.size().y / 2.0), g.size());
        if let Some(url) = &c.url {
            let resp = ui.interact(r, Id::new(("lyrics-credit", &c.text)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(url.as_str());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &c.text));
            let color = if resp.hovered() { col::text() } else { col::sub() };
            ui.painter().galley(r.min, g, color);
            if resp.hovered() {
                ui.painter().hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, color));
            }
            if resp.clicked() {
                let _ = open::that(url);
            }
        } else {
            ui.painter().galley(r.min, g, col::sub());
        }
    }
    rect.right() - x
}

/// A centered note with optional buttons under it.
fn message(app: &mut App, ui: &mut Ui, rect: Rect, title: &str, body: &str, actions: impl FnOnce(&mut App, &mut Ui)) {
    ui.add_space(rect.height() * 0.28);
    widgets::unavailable(ui, title, body);
    ui.add_space(sp(2.0));
    ui.vertical_centered(|ui| actions(app, ui));
}

fn provider_name(app: &App) -> &'static str {
    match app.config.lyrics().provider {
        ProviderSetting::Only(p) => p.label(),
        ProviderSetting::Auto => "LRCLIB",
    }
}

fn lyrics_view(app: &mut App, ui: &mut Ui, rect: Rect, footer_at: Rect) {
    if app.lyrics_picker.is_some() {
        return picker(app, ui, rect);
    }
    let Some(track) = app.lyrics_track() else {
        header(ui, rect, "lyrics", None);
        return message(app, ui, rect, "nothing playing", "Lyrics follow the song that's playing.", |_, _| {});
    };
    let res = match app.lyrics.status.clone() {
        Status::Done(res) => res,
        Status::Idle | Status::Loading => {
            header(ui, rect, "lyrics", None);
            let body = format!("Looking on {}…", provider_name(app));
            return message(app, ui, rect, "finding lyrics", &body, |_, _| {});
        }
    };
    let tried = |res: &Resolved| res.tried.iter().skip(1).map(|(p, what)| format!("{}: {what}", p.label())).collect::<Vec<_>>().join(" · ");
    match &res.outcome {
        Outcome::Found { lyrics, how } => show_lyrics(app, ui, rect, footer_at, &track, lyrics, *how, &res),
        Outcome::Instrumental { .. } => {
            header(ui, rect, "lyrics / instrumental", None);
            message(app, ui, rect, "♪  instrumental", "LRCLIB lists this recording as instrumental.", |app, ui| {
                if widgets::bracket_button(ui, None, "it has lyrics — choose", false).clicked() {
                    app.act(Action::LyricsOpenPicker);
                }
            });
        }
        Outcome::NotFound { provider } => {
            header(ui, rect, "lyrics", None);
            let mut body = format!("{} has no lyrics that match this recording.", provider.label());
            let others = tried(&res);
            if !others.is_empty() {
                body.push_str(&format!("\n{others}"));
            }
            message(app, ui, rect, "no lyrics found", &body, |app, ui| {
                if widgets::bracket_button(ui, None, "search LRCLIB", false).clicked() {
                    app.act(Action::LyricsOpenPicker);
                }
                ui.add_space(sp(1.0));
                if widgets::bracket_button(ui, None, "look again", false).clicked() {
                    app.act(Action::LyricsRetry);
                }
            });
        }
        Outcome::Ambiguous { candidates, .. } => {
            header(ui, rect, "lyrics / choose", None);
            ui.add_space(px(40.0));
            ui.label(RichText::new("several versions match").font(theme::regular(16.0)).color(col::text()));
            ui.label(RichText::new("They have different lyrics. Pick the one that fits what's playing.").font(theme::regular(12.5)).color(col::muted()));
            ui.add_space(sp(1.5));
            let h = (ui.available_height() - px(60.0)).max(px(80.0));
            candidate_list(app, ui, candidates, track.duration_ms, h);
            ui.add_space(sp(1.0));
            ui.horizontal(|ui| {
                if widgets::bracket_button(ui, None, "search", false).clicked() {
                    app.act(Action::LyricsOpenPicker);
                }
                if widgets::bracket_button(ui, None, "none of these", false).clicked() {
                    app.act(Action::LyricsNoneFit);
                }
            });
        }
        Outcome::Unavailable { provider, reason } => {
            header(ui, rect, "lyrics / unavailable", None);
            let title = format!("{} lyrics aren't available", provider.label());
            message(app, ui, rect, &title, reason, |app, ui| {
                if widgets::bracket_button(ui, None, "use LRCLIB (auto)", false).on_hover_text("Sets lyrics.provider = \"auto\" in config.toml").clicked() {
                    app.act(Action::LyricsProvider(ProviderSetting::Auto));
                }
            });
        }
        Outcome::Failed { provider, error } => {
            header(ui, rect, "lyrics", None);
            let title = format!("couldn't load lyrics from {}", provider.label());
            message(app, ui, rect, &title, error, |app, ui| {
                if widgets::bracket_button(ui, None, "try again", false).clicked() {
                    app.act(Action::LyricsRetry);
                }
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn show_lyrics(app: &mut App, ui: &mut Ui, rect: Rect, footer_at: Rect, track: &TrackRef, lyrics: &Lyrics, how: Match, res: &Resolved) {
    let st = theme::lyrics_style();
    let tip = match lyrics.timing {
        Timing::Syllable => "The provider timed every syllable; they light up as they're sung.",
        Timing::Word => "The provider timed every word; words light up as they're sung.",
        Timing::Line => "The provider timed each line, not each word, so lines are highlighted whole.",
        Timing::Plain => "These lyrics have no timing, so they don't follow the song.",
    };
    let title = format!("lyrics / {}", lyrics.timing.label());
    header(ui, rect, &title, Some(tip));
    let header_w = ui.painter().layout_no_wrap(title, theme::regular(13.0), col::sub()).size().x;
    credits(ui, rect, lyrics, header_w);
    // The footer sits at the very bottom of the right side (under the visualizer when it shows).
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + px(34.0)), pos2(rect.right(), rect.bottom().min(footer_at.top()) - px(6.0)));
    footer(app, ui, footer_at, lyrics, how, res);

    let wrap = rect.width();
    // In 2 pt steps: each new size is another set of glyphs in the font atlas,
    // so a size that follows the window width exactly grows it on every resize.
    let size = ((rect.width() * 0.05 * st.size).clamp(px(14.0), px(80.0)) / 2.0).round() * 2.0;
    let gap = size * 0.55 * st.spacing;
    let align = match st.align.as_str() {
        "center" => egui::Align::Center,
        "right" => egui::Align::RIGHT,
        _ => egui::Align::LEFT,
    };
    let regular = egui::FontId::new(size, theme::lyrics(1.0, false).family);
    let strong = egui::FontId::new(size, theme::lyrics(1.0, st.bold).family);

    if lyrics.timing == Timing::Plain {
        let mode = app.config.lyrics().translation;
        let tr_state = app.lyrics_translation(lyrics);
        translation_note(app, ui, rect, tr_state.as_ref().map(Option::as_deref), lyrics);
        let tr: Option<&[String]> = match tr_state.as_ref().and_then(Option::as_deref) {
            Some(Translation::Ready { lines, .. }) => Some(lines),
            _ => None,
        };
        let small = egui::FontId::new(((size * 0.58) / 2.0).round() * 2.0, theme::lyrics(1.0, false).family);
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ScrollArea::vertical().id_salt(("lyrics-static", &track.key)).auto_shrink([false, false]).show(ui, |ui| {
                let layout = egui::Layout::top_down(align);
                ui.with_layout(layout, |ui| {
                    for (i, l) in lyrics.lines.iter().enumerate() {
                        let t = tr.as_ref().and_then(|t| t.get(i)).map(|t| t.trim()).filter(|t| !t.is_empty());
                        let main = match (mode, t) {
                            (Mode::Only, Some(t)) => t,
                            _ => l.text.as_str(),
                        };
                        if main.is_empty() {
                            ui.add_space(gap);
                            continue;
                        }
                        ui.add(egui::Label::new(RichText::new(main).font(regular.clone()).color(st.current.color32())).wrap());
                        if mode == Mode::Below
                            && let Some(t) = t.filter(|t| !t.eq_ignore_ascii_case(main))
                        {
                            ui.add(egui::Label::new(RichText::new(t).font(small.clone()).color(st.other.color32())).wrap());
                        }
                        ui.add_space(gap * 0.4);
                    }
                });
            });
        });
        return;
    }

    // The translation, when it's wanted and ready.
    let mode = app.config.lyrics().translation;
    let tr_state = app.lyrics_translation(lyrics);
    let tr_lines: Option<&[String]> = match tr_state.as_ref().and_then(Option::as_deref) {
        Some(Translation::Ready { lines, .. }) => Some(lines),
        _ => None,
    };
    let only = mode == Mode::Only && tr_lines.is_some();
    let below = mode == Mode::Below && tr_lines.is_some();
    translation_note(app, ui, rect, tr_state.as_ref().map(Option::as_deref), lyrics);
    let small = egui::FontId::new(((size * 0.58) / 2.0).round() * 2.0, theme::lyrics(1.0, false).family);
    // Backing vocals: smaller, under their line.
    let back_font = egui::FontId::new(((size * 0.66) / 2.0).round() * 2.0, theme::lyrics(1.0, false).family);
    // "original + translation": the translation in smaller type under each line.
    let trans_job = |i: usize, shown: &[Line], align: egui::Align| -> Option<LayoutJob> {
        let t = tr_lines.as_ref().filter(|_| below)?.get(i)?.trim();
        (!t.is_empty() && !t.eq_ignore_ascii_case(shown[i].text.trim())).then(|| {
            let mut job = LayoutJob::simple(t.to_string(), small.clone(), Color32::WHITE, wrap);
            job.halign = align;
            job
        })
    };
    // Every line's height in both faces (a line's height is the larger, so the layout
    // doesn't shift as it becomes current), direction and alignment. They depend only
    // on the text, fonts and width, so they're worked out once, not every frame (the
    // visualizer repaints at 60 fps); to draw, only the lines in view are laid out.
    let key = Id::new((
        &track.key,
        lyrics.lines.as_ptr() as usize,
        lyrics.lines.len(),
        tr_lines.map(|t| (t.as_ptr() as usize, t.len())),
        (only, below, st.bold, align),
        (&regular, &strong, &small, &back_font),
        (wrap.to_bits(), ui.ctx().pixels_per_point().to_bits()),
    ));
    let measured = ui.data(|d| d.get_temp::<(Id, std::sync::Arc<Measured>)>(Id::new("lyrics-measured"))).filter(|(k, _)| *k == key).map(|(_, m)| m);
    let m = measured.unwrap_or_else(|| {
        // In "translated", the translation takes each line's place (whole lines: word timing is the original's).
        // Every line stays (a short translation leaves the rest as they are), so line
        // numbers match the original's.
        let translated: Option<Vec<Line>> = tr_lines.filter(|_| only).map(|tl| {
            lyrics
                .lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let text = tl.get(i).filter(|t| !t.trim().is_empty()).unwrap_or(&l.text).clone();
                    Line { start_ms: l.start_ms, end_ms: l.end_ms, text, voice: l.voice, ..Default::default() }
                })
                .collect()
        });
        let shown = translated.as_deref().unwrap_or(&lyrics.lines);
        let rtl: Vec<bool> = shown.iter().map(|l| lyrics::is_rtl(&l.text)).collect();
        // A duet puts its voices on two sides; centered lyrics can't, so there the
        // lead goes left, the other voice right and everyone together stays centered.
        let duet = shown.iter().any(|l| l.voice == lyrics::Voice::Other);
        let aligns: Vec<egui::Align> = shown
            .iter()
            .zip(&rtl)
            .map(|(l, &r)| {
                let base = match (align, l.voice) {
                    (egui::Align::Center, lyrics::Voice::Lead) if duet => egui::Align::LEFT,
                    (egui::Align::Center, lyrics::Voice::Other) => egui::Align::LEFT,
                    _ => align,
                };
                let a = if r { mirror(base) } else { base };
                if l.voice == lyrics::Voice::Other { mirror(a) } else { a }
            })
            .collect();
        // A right-to-left line is drawn piece by piece only when it fits on one row:
        // egui breaks rows left to right, which would put its last words first.
        let fits = |l: &Line| ui.painter().layout_no_wrap(l.text.clone(), strong.clone(), Color32::WHITE).size().x <= wrap;
        let tagged: Vec<bool> = shown.iter().zip(&rtl).map(|(l, &r)| !l.tokens.is_empty() && (!r || fits(l))).collect();
        let height = |font: &egui::FontId, i: usize| ui.painter().layout_job(line_job(&shown[i].text, &shown[i].tokens, font.clone(), wrap, aligns[i], rtl[i], tagged[i])).size().y;
        let plain: Vec<f32> = (0..shown.len()).map(|i| height(&regular, i)).collect();
        let bold = if st.bold { (0..shown.len()).map(|i| height(&strong, i)).collect() } else { plain.clone() };
        let back = shown
            .iter()
            .enumerate()
            .map(|(i, l)| l.backing.as_ref().map(|b| ui.painter().layout_job(line_job(&b.text, &b.tokens, back_font.clone(), wrap, aligns[i], rtl[i], !b.tokens.is_empty() && !rtl[i])).size().y))
            .collect();
        let trans = (0..shown.len()).map(|i| trans_job(i, shown, aligns[i]).map(|j| ui.painter().layout_job(j).size().y)).collect();
        let m = std::sync::Arc::new(Measured { translated, plain, bold, back, trans, rtl, tagged, align: aligns });
        ui.data_mut(|d| d.insert_temp(Id::new("lyrics-measured"), (key, m.clone())));
        m
    });
    let shown: &[Line] = m.translated.as_deref().unwrap_or(&lyrics.lines);

    let offset = app.lyrics_offset_total();
    let pos = app.lyrics_position();
    // The line the view follows, and every line being sung (more than one where voices overlap).
    let cur = lyrics.line_at(pos);
    let active = lyrics.active(pos);
    let reduce = theme::reduce_motion();
    // How a new line arrives. With Reduce Motion on, movement becomes a
    // cross-fade (as Apple recommends) instead of stopping altogether.
    let motion = match (st.animation.as_str(), reduce) {
        ("none", _) => "none",
        (_, true) => "fade",
        (m, false) => m,
    };
    let secs = 0.55 / st.speed.max(0.1);
    let line_secs = if motion == "none" { 0.0 } else { secs * 0.8 };
    // How "current" each nearby line is, easing in and out as lines change.
    let emphasis: Vec<f32> = (0..shown.len())
        .map(|i| {
            let on = active.contains(&i) || (active.is_empty() && Some(i) == cur && lyrics.line_end(i) == u64::MAX);
            let near = on || cur.is_some_and(|c| i.abs_diff(c) <= 2);
            if near { ease_out_cubic(ui.ctx().animate_value_with_time(Id::new(("lyric-e", &track.key, i)), if on { 1.0 } else { 0.0 }, line_secs)) } else { 0.0 }
        })
        .collect();
    let lay = |ui: &Ui, i: usize, font: &egui::FontId| ui.painter().layout_job(line_job(&shown[i].text, &shown[i].tokens, font.clone(), wrap, m.align[i], m.rtl[i], m.tagged[i]));
    let trans_gap = size * 0.14;
    // A line is as tall as it's drawn: the bold current line may wrap where
    // the regular one doesn't, so its height eases with its emphasis.
    let main_h = |i: usize| m.plain[i] + (m.bold[i] - m.plain[i]) * emphasis[i];
    let back_h = |i: usize| m.back[i].map(|h| trans_gap + h).unwrap_or(0.0);
    let height = |i: usize| main_h(i) + back_h(i) + m.trans[i].map(|h| trans_gap + h).unwrap_or(0.0);
    let mut tops = Vec::with_capacity(shown.len());
    let mut y = 0.0;
    for i in 0..shown.len() {
        tops.push(y);
        y += height(i) + gap;
    }
    let total = y;
    let target = match cur {
        Some(c) => tops[c] + height(c) / 2.0,
        None => -size,
    };

    let ease: fn(f32) -> f32 = if motion == "pop" { ease_out_back } else { ease_out_cubic };
    let (from, to, p) = tween(ui, Id::new(("lyrics-scroll", &track.key)), cur.map(|c| c as i64).unwrap_or(-1), target, if motion == "none" { 0.0 } else { secs }, ease);
    let follow = from + (to - from) * ease(p);
    // "pop": the new line flashes in the highlight color.
    let flash = if motion == "pop" { (1.0 - p).max(0.0) * 0.6 } else { 0.0 };

    // Scrolling by hand pauses following; a button brings the current line back.
    let manual_id = Id::new(("lyrics-manual", &track.key));
    let mut manual: Option<f32> = ui.ctx().data(|d| d.get_temp(manual_id));
    if ui.rect_contains_pointer(body) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            manual = Some((manual.unwrap_or(follow) - wheel).clamp(-size, total));
        }
    }
    let place = match st.position.as_str() {
        "top" => 0.2,
        "middle" => 0.5,
        _ => 0.38,
    };
    let anchor = body.top() + body.height() * place;
    let anchor_x = |a: egui::Align| match a {
        egui::Align::Center => rect.center().x,
        egui::Align::RIGHT => rect.right(),
        _ => rect.left(),
    };
    let painter = ui.painter().with_clip_rect(body.expand2(vec2(sp(2.0), 0.0)));
    let (current_color, other_color, highlight) = (st.current.color32(), st.other.color32(), st.highlight.color32());
    // Word timing: words still to come are dim, the word being sung lights up bright,
    // and sung words settle into the line's color a moment after they're sung.
    let bright = sung_color(highlight, current_color, col::text());
    let waiting_color = current_color.gamma_multiply(0.4);
    // The theme's word timing: words may light a little early (or late) and stay bright
    // for a while; the lines keep their own time.
    let word_pos = word_position(pos, st.word_lead);
    let glow = st.word_glow as f32;
    let sweep = |tokens: &[lyrics::Token], end: u64| -> (Vec<f32>, Vec<Color32>) {
        let progress: Vec<f32> = lyrics::token_progress(tokens, end, word_pos).collect();
        let lit = (0..tokens.len())
            .map(|k| {
                let done = lyrics::token_end(tokens, k, end);
                let settle = if word_pos < done {
                    0.0
                } else if glow <= 0.0 {
                    1.0
                } else {
                    ((word_pos - done) as f32 / glow).min(1.0)
                };
                mix(bright, current_color, settle)
            })
            .collect();
        (progress, lit)
    };
    // Reduce Motion: no moving edge (see `wipe`).
    let band = if reduce { 0.0 } else { size * 0.45 };
    let mut seek = None;
    // Draws every visible line at `scroll`, at `alpha`; only one pass takes clicks.
    let mut pass = |ui: &Ui, scroll: f32, alpha: f32, interactive: bool| {
        for i in 0..shown.len() {
            let h = height(i);
            let top = anchor - scroll + tops[i];
            if top + h < body.top() || top > body.bottom() {
                continue;
            }
            let e = emphasis[i];
            let x = anchor_x(m.align[i]);
            // Lines fade with distance from the current one.
            let dist = (((top + h / 2.0) - anchor).abs() / (body.height() * 0.6)).min(1.0);
            let other = other_color.gamma_multiply(1.0 - dist * st.fade);
            let mut color = lerp_color(other, current_color, e);
            if Some(i) == cur && flash > 0.0 {
                color = lerp_color(color, highlight, flash);
            }
            let hit = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), h));
            if interactive {
                let resp = ui.interact(hit.intersect(body), Id::new(("lyric-line", i)), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &shown[i].text));
                if resp.hovered() && !active.contains(&i) {
                    color = lerp_color(color, current_color, 0.6);
                }
                if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Play from this line").clicked() {
                    seek = Some(lyrics::seek_target(lyrics.lines[i].start_ms, offset));
                }
            }
            let mh = main_h(i);
            // Regular and bold cross-fade in place, from the line's top.
            let draw = |g: std::sync::Arc<egui::Galley>, at: egui::Pos2, a: f32, c: Option<Color32>| {
                if a * alpha <= 0.01 {
                    return;
                }
                let mut shape = egui::epaint::TextShape::new(at, g, color);
                shape.override_text_color = c;
                shape.opacity_factor = a * alpha;
                painter.add(shape);
            };
            let plain = lay(ui, i, &regular);
            let bold = |ui: &Ui| if st.bold { lay(ui, i, &strong) } else { plain.clone() };
            let at = pos2(x, top);
            let sung = m.tagged[i] && !only && e > 0.01;
            if sung {
                // Word by word: each piece sweeps from waiting to lit as it's sung.
                let (progress, lit) = sweep(&shown[i].tokens, lyrics.line_end(i));
                let waiting = mix(color, waiting_color, e);
                let lit: Vec<Color32> = lit.into_iter().map(|c| mix(color, c, e)).collect();
                draw(wipe(&plain, &progress, &lit, waiting, m.rtl[i], band), at, 1.0 - e, None);
                draw(wipe(&bold(ui), &progress, &lit, waiting, m.rtl[i], band), at, e, None);
            } else {
                // Bold only matters while it shows (and when it's another face).
                let b = if st.bold && e > 0.01 { bold(ui) } else { plain.clone() };
                draw(plain, at, 1.0 - e, Some(color));
                draw(b, at, e, Some(color));
            }
            // Backing vocals: quieter, under the line, swept on their own timing.
            if let Some(b) = shown[i].backing.as_ref().filter(|_| m.back[i].is_some()) {
                let tagged = !b.tokens.is_empty() && !m.rtl[i];
                let g = ui.painter().layout_job(line_job(&b.text, &b.tokens, back_font.clone(), wrap, m.align[i], m.rtl[i], tagged));
                let at = pos2(x, top + mh + trans_gap);
                let bc = lerp_color(other.gamma_multiply(0.75), current_color.gamma_multiply(0.8), e);
                if tagged && e > 0.01 {
                    let (progress, lit) = sweep(&b.tokens, lyrics.line_end(i));
                    let lit: Vec<Color32> = lit.into_iter().map(|c| mix(bc, c.gamma_multiply(0.85), e)).collect();
                    draw(wipe(&g, &progress, &lit, mix(bc, waiting_color.gamma_multiply(0.85), e), false, band * 0.66), at, 1.0, None);
                } else {
                    draw(g, at, 1.0, Some(bc));
                }
            }
            if m.trans[i].is_some()
                && let Some(job) = trans_job(i, shown, m.align[i])
            {
                // The translation: quieter than its line, brighter when it's current.
                let g = ui.painter().layout_job(job);
                let tc = lerp_color(other.gamma_multiply(0.8), current_color.gamma_multiply(0.82), e);
                let mut shape = egui::epaint::TextShape::new(pos2(x, top + mh + back_h(i) + trans_gap), g, tc);
                shape.override_text_color = Some(tc);
                shape.opacity_factor = alpha;
                painter.add(shape);
            }
        }
    };
    match manual {
        Some(m) => pass(ui, m, 1.0, true),
        // "fade": the lines dissolve from where they were to where they go.
        None if motion == "fade" && p < 1.0 => {
            let q = ease_out_cubic(p);
            pass(ui, from, 1.0 - q, false);
            pass(ui, to, q, true);
        }
        None => pass(ui, follow, 1.0, true),
    }
    if let Some(to) = seek {
        app.act(Action::Seek(to));
        manual = None;
    }
    if manual.is_some() {
        let r = Rect::from_center_size(pos2(body.center().x, body.bottom() - px(26.0)), vec2(px(240.0), px(40.0)));
        let back = ui.scope_builder(UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
            let size = vec2(px(236.0), px(40.0));
            ui.painter().rect_filled(Rect::from_center_size(r.center(), size), theme::small_radius(), col::overlay());
            widgets::bracket_button(ui, None, "↓ back to current line", true).clicked()
        });
        if back.inner {
            manual = None;
        }
    }
    ui.ctx().data_mut(|d| {
        if let Some(m) = manual {
            d.insert_temp(manual_id, m);
        } else {
            d.remove::<f32>(manual_id);
        }
    });
    if app.pb.is_playing || app.local.as_ref().is_some_and(|l| l.is_playing()) || app.scrub_ms.is_some() {
        let frame = next_frame(lyrics, &active, pos, word_pos, st.word_glow as u64, only, reduce, ui.input(|i| i.focused));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(frame));
    }
}

/// Where the words are: `pos` moved by the theme's `word_lead` (ms earlier).
fn word_position(pos: u64, lead: i32) -> u64 {
    pos.saturating_add_signed(lead as i64)
}

/// How long until the lyrics look different: a word being swept wants frames
/// (40 a second in front, 20 behind — the sweep's soft edge keeps that smooth,
/// and frames are what Glass's CPU goes on); otherwise the next word or line
/// that starts, or a line that ends (the scroll and fades animate by themselves).
/// Words go by `word_pos` and settle over `glow` ms; lines go by `pos`.
#[allow(clippy::too_many_arguments)]
fn next_frame(lyrics: &Lyrics, active: &[usize], pos: u64, word_pos: u64, glow: u64, only: bool, reduce: bool, focused: bool) -> u64 {
    let mut next = u64::MAX;
    // Word times, seen from the lines' clock.
    let lines_at = |word_ms: u64| word_ms.saturating_add(pos).saturating_sub(word_pos);
    if !only {
        for &i in active {
            let l = &lyrics.lines[i];
            let end = lyrics.line_end(i);
            for tokens in std::iter::once(&l.tokens).chain(l.backing.iter().map(|b| &b.tokens)) {
                for (k, t) in tokens.iter().enumerate() {
                    let done = lyrics::token_end(tokens, k, end);
                    if t.start_ms > word_pos {
                        next = next.min(lines_at(t.start_ms));
                    } else if word_pos < done {
                        // With Reduce Motion the word brightens in place: fewer frames do.
                        return match (focused, reduce) {
                            (true, false) => 25,
                            (false, false) | (true, true) => 50,
                            (false, true) => 100,
                        };
                    } else if word_pos < done + glow {
                        // A sung word settles into the line's color.
                        return if focused { 33 } else { 66 };
                    }
                }
            }
        }
    }
    let started = lyrics.lines.partition_point(|l| l.start_ms <= pos);
    if let Some(l) = lyrics.lines.get(started) {
        next = next.min(l.start_ms);
    }
    for &i in active {
        next = next.min(lyrics.line_end(i));
    }
    if next == u64::MAX { 1_000 } else { (next.saturating_sub(pos)).clamp(16, 5_000) }
}

/// How well the lyrics fit, plus the timing offset and the translation and
/// picker links. In a narrow window the links drop their words ("文A",
/// "− 0.0 s +") and the note on the left keeps only the parts that fit whole.
fn footer(app: &mut App, ui: &mut Ui, rect: Rect, lyrics: &Lyrics, how: Match, res: &Resolved) {
    let small = theme::regular(11.0);
    let offset = app.lyrics_song_offset();
    let value = if offset == 0 { "0.0 s".to_string() } else { format!("{:+.1} s", offset as f64 / 1000.0) };
    let mode = app.config.lyrics().translation;
    let short = match mode {
        Mode::Off => "original",
        Mode::Below => "+ translation",
        Mode::Only => "translated",
    };
    // The note: what's still happening first, then how the lyrics were found.
    let mut parts: Vec<String> = Vec::new();
    if res.pending && lyrics.word_coverage() < 0.8 {
        parts.push("looking for word timing…".into());
    }
    if app.lyrics_lines_only {
        parts.push("line by line".into());
    }
    parts.push(
        match how {
            Match::Exact => "exact match",
            Match::Search => "matched by search",
            Match::Manual => "your pick",
        }
        .into(),
    );
    if res.fell_back() {
        parts.push(format!("{} had none", res.tried[0].0.name()));
    }
    if let Some(l) = &lyrics.language {
        parts.push(l.clone());
    }
    // Word timing from a service, or line by line (LRCLIB's): the user's choice, per song.
    let lines_link = if app.lyrics_lines_only {
        Some(("word timing", "Look for word-by-word timing for this song again", false))
    } else if lyrics.source.provider != ProviderId::Lrclib && lyrics.word_coverage() >= 0.8 && res.tried.iter().any(|(p, what)| *p == ProviderId::Lrclib && what.starts_with("found")) {
        Some(("LRCLIB lines", "Show this song line by line with LRCLIB's lyrics (or your pick) instead of word timing. Remembered for this song.", true))
    } else {
        None
    };
    let painter = ui.painter().clone();
    let width = |t: &str| painter.layout_no_wrap(t.to_string(), small.clone(), col::muted()).size().x;
    let links = format!("文A {short}    {}wrong lyrics?    − timing {value} +", lines_link.map(|(l, ..)| format!("{l}    ")).unwrap_or_default());
    let compact = rect.width() - width(&links) < width(&parts[parts.len() - 1]) + sp(3.0);
    // Right: "文A original   wrong lyrics?   −  timing 0.0 s  +". Laid out first; the
    // note on the left gets what's left.
    let used = ui
        .scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            // This song's own timing (kept for it); [ and ] change it too.
            if widgets::link(ui, "+", theme::regular(13.0), col::sub()).on_hover_text("Show lyrics 0.1 s earlier for this song  ]").clicked() {
                app.act(Action::LyricsSongOffset(offset + 100));
            }
            let text = if compact { value.clone() } else { format!("timing {value}") };
            let label = ui.add(egui::Label::new(RichText::new(text).font(small.clone()).color(if offset == 0 { col::muted() } else { col::sub() })).sense(Sense::click()));
            if label.on_hover_text("This song's timing, remembered for it. Positive shows lyrics earlier. Double-click to reset.").double_clicked() {
                app.act(Action::LyricsSongOffset(0));
            }
            if widgets::link(ui, "−", theme::regular(13.0), col::sub()).on_hover_text("Show lyrics 0.1 s later for this song  [").clicked() {
                app.act(Action::LyricsSongOffset(offset - 100));
            }
            ui.add_space(sp(2.0));
            if widgets::link(ui, "wrong lyrics?", small.clone(), col::muted()).on_hover_text("Choose other lyrics for this song").clicked() {
                app.act(Action::LyricsOpenPicker);
            }
            ui.add_space(sp(2.0));
            if let Some((label, hint, on)) = lines_link {
                if widgets::link(ui, label, small.clone(), col::muted()).on_hover_text(hint).clicked() {
                    app.act(Action::LyricsLinesOnly(on));
                }
                ui.add_space(sp(2.0));
            }
            // Original / original + translation / translated  (T).
            let color = if mode == Mode::Off { col::muted() } else { col::accent() };
            let text = if compact { "文A".to_string() } else { format!("文A {short}") };
            if widgets::link(ui, &text, small.clone(), color).on_hover_text(format!("Translation: {short} — click or press T: original · original + translation · translated")).clicked() {
                app.act(Action::LyricsTranslation(mode.next()));
            }
        })
        .response
        .rect;
    let left = Rect::from_min_max(rect.min, pos2((used.left() - sp(2.0)).max(rect.left()), rect.bottom()));
    // As many whole parts as fit; never a cut-off word.
    let mut text = String::new();
    for p in &parts {
        let next = if text.is_empty() { p.clone() } else { format!("{text} · {p}") };
        if width(&next) > left.width() {
            break;
        }
        text = next;
    }
    let s = &lyrics.source;
    let detail = format!(
        "Lyrics from {} (record {}): “{}” — {}{} · {}.\n{}.\nLyrics belong to their rights holders and are shown here for personal use.",
        s.provider.label(),
        s.id,
        s.title,
        s.artist,
        if s.album.is_empty() { String::new() } else { format!(" · {}", s.album) },
        model::fmt_ms(s.duration_ms),
        parts.join(" · ")
    );
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        let r = if text.is_empty() { ui.allocate_response(left.size(), Sense::hover()) } else { ui.add(egui::Label::new(RichText::new(text).font(small.clone()).color(col::muted())).truncate()) };
        r.on_hover_text(detail);
    });
}

/// "+3 s" from the playing track's length, or "same length".
fn length_note(candidate_ms: u64, track_ms: u64) -> String {
    if track_ms == 0 || candidate_ms == 0 {
        return model::fmt_ms(candidate_ms);
    }
    let diff = (candidate_ms as i64 - track_ms as i64) as f64 / 1000.0;
    if diff.abs() <= 2.0 { format!("{} · same length", model::fmt_ms(candidate_ms)) } else { format!("{} · {:+.0} s", model::fmt_ms(candidate_ms), diff) }
}

fn candidate_list(app: &mut App, ui: &mut Ui, list: &[Candidate], track_ms: u64, height: f32) {
    ScrollArea::vertical().id_salt("lyrics-candidates").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
        for c in list {
            let w = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(vec2(w, px(62.0)), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} — {}", c.source.title, c.source.artist)));
            if resp.hovered() || resp.has_focus() {
                ui.painter().rect_filled(r, theme::small_radius(), col::hover());
            }
            widgets::focus_ring(ui, &resp, r, theme::small_radius());
            let x = r.left() + sp(1.5);
            let line = |y: f32, text: &str, font: egui::FontId, color| widgets::text_trunc(ui, pos2(x, r.top() + px(y)), text, font, color, w - sp(3.0));
            line(14.0, &format!("{} — {}", c.source.title, c.source.artist), theme::regular(14.0), col::text());
            let kind = if c.instrumental { "instrumental".to_string() } else { c.timing.label().to_string() };
            let album = if c.source.album.is_empty() { String::new() } else { format!("{} · ", c.source.album) };
            line(32.0, &format!("{album}{} · {kind}", length_note(c.source.duration_ms, track_ms)), theme::regular(12.0), col::sub());
            if !c.preview.is_empty() {
                line(48.0, &format!("“{}”", c.preview), theme::regular(11.5), col::muted());
            }
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                app.act(Action::LyricsPick(c.record.clone()));
            }
        }
    });
}

/// Searching LRCLIB by hand and picking the right record.
fn picker(app: &mut App, ui: &mut Ui, rect: Rect) {
    header(ui, rect, "lyrics / choose", None);
    ui.add_space(px(40.0));
    ui.label(RichText::new("choose lyrics for this song").font(theme::regular(16.0)).color(col::text()));
    let track = app.lyrics_track();
    if let Some(t) = &track {
        ui.label(RichText::new(format!("{} — {} · {}", t.title, t.artists.join(", "), model::fmt_ms(t.duration_ms))).font(theme::regular(12.5)).color(col::muted()));
    }
    ui.add_space(sp(1.5));
    let Some(p) = app.lyrics_picker.as_mut() else { return };
    let field = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text("search LRCLIB").font(theme::regular(14.0)).desired_width(rect.width()));
    let query = p.query.clone();
    let results = p.results.clone();
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        app.act(Action::LyricsSearch(query));
    }
    ui.add_space(sp(1.5));
    let h = (ui.available_height() - px(60.0)).max(px(80.0));
    match results {
        None => widgets::loading(ui),
        Some(Err(e)) => widgets::unavailable(ui, "search failed", &e),
        Some(Ok(list)) if list.is_empty() => widgets::unavailable(ui, "nothing found", "Try fewer words, or just the title."),
        Some(Ok(list)) => candidate_list(app, ui, &list, track.as_ref().map(|t| t.duration_ms).unwrap_or(0), h),
    }
    ui.add_space(sp(1.0));
    ui.horizontal(|ui| {
        if widgets::bracket_button(ui, None, "none of these", false).on_hover_text("Show no lyrics for this song").clicked() {
            app.act(Action::LyricsNoneFit);
        }
        if widgets::bracket_button(ui, None, "cancel", false).clicked() {
            app.act(Action::LyricsClosePicker);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A timed line laid out once, then swept: sung pieces take their lit color, the
    /// piece being sung is lit on its leading side only, the rest wait — and the
    /// layout itself is shared, not redone.
    #[test]
    fn the_sweep_recolors_without_laying_out_again() {
        let ctx = egui::Context::default();
        // A pass, so the fonts exist (its texture upload is nobody's business here).
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let tokens: Vec<lyrics::Token> =
            ["one ", "two ", "three"].iter().enumerate().map(|(k, t)| lyrics::Token { text: t.to_string(), start_ms: k as u64 * 1000, end_ms: Some(k as u64 * 1000 + 900) }).collect();
        let font = egui::FontId::monospace(20.0);
        let g = ctx.fonts_mut(|f| f.layout_job(line_job("one two three", &tokens, font.clone(), 400.0, egui::Align::LEFT, false, true)));
        let (lit, waiting) = (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 0, 255));
        let swept = wipe(&g, &[1.0, 0.5, 0.0], &[lit; 3], waiting, false, 10.0);
        let row = &swept.rows[0].row;
        // Glyph quads in reading order: "one" (3), "two" (3), "three" (5) — spaces have none.
        let quads: Vec<&[egui::epaint::Vertex]> = row.visuals.mesh.vertices[row.visuals.glyph_vertex_range.clone()].chunks(4).collect();
        assert_eq!(quads.len(), 11);
        assert!(quads[..3].iter().flat_map(|q| q.iter()).all(|v| v.color == lit), "sung");
        assert!(quads[6..].iter().flat_map(|q| q.iter()).all(|v| v.color == waiting), "still to come");
        // "two", half sung: lit at its start, waiting at its end, a blend at the edge.
        assert!(quads[3].iter().all(|v| v.color == lit));
        assert!(quads[5].iter().all(|v| v.color == waiting));
        // The edge falls inside "w": its leading corners lit, its trailing ones not, and the
        // GPU blends across the glyph between them.
        assert!(quads[4].iter().any(|v| v.color != waiting) && quads[4].iter().any(|v| v.color != lit), "the edge runs through the middle letter");
        // The same layout (shared glyphs), only the colors differ.
        assert_eq!(swept.rows[0].row.glyphs.len(), g.rows[0].row.glyphs.len());
        // Right to left, the sweep starts at the right.
        let rtl = wipe(&g, &[0.0, 0.5, 0.0], &[lit; 3], waiting, true, 10.0);
        let q: Vec<&[egui::epaint::Vertex]> = rtl.rows[0].row.visuals.mesh.vertices[rtl.rows[0].row.visuals.glyph_vertex_range.clone()].chunks(4).collect();
        assert!(q[5].iter().all(|v| v.color == lit) && q[3].iter().all(|v| v.color == waiting));
        // Reduce Motion (no band): the piece brightens in place, evenly.
        let still = wipe(&g, &[0.0, 0.5, 0.0], &[lit; 3], waiting, false, 0.0);
        let q: Vec<&[egui::epaint::Vertex]> = still.rows[0].row.visuals.mesh.vertices[still.rows[0].row.visuals.glyph_vertex_range.clone()].chunks(4).collect();
        let c = q[3][0].color;
        assert!(c != lit && c != waiting && q[3..6].iter().flat_map(|q| q.iter()).all(|v| v.color == c));
        // A translucent waiting color blends as it is, never darker than itself.
        let dim = Color32::from_rgb(200, 200, 200).gamma_multiply(0.45);
        assert_eq!(mix(dim, lit, 0.0), dim);
        assert!(mix(dim, Color32::WHITE, 0.5).r() > dim.r());
    }

    #[test]
    fn the_word_being_sung_always_stands_out() {
        let text = Color32::from_rgb(0xE9, 0xE4, 0xDC);
        // A dark theme highlight (#481947) under a purple line (#6D3A78): lifted toward the text color instead.
        let c = sung_color(Color32::from_rgb(0x48, 0x19, 0x47), Color32::from_rgb(0x6D, 0x3A, 0x78), text);
        assert!(luminance(c) > luminance(Color32::from_rgb(0x6D, 0x3A, 0x78)) + 0.2, "{c:?}");
        // A bright highlight is used as it is.
        let pink = Color32::from_rgb(0xF0, 0x80, 0xE0);
        assert_eq!(sung_color(pink, Color32::from_rgb(0x6D, 0x3A, 0x78), text), pink);
    }

    #[test]
    fn frames_only_while_a_word_is_being_sung() {
        let l = lyrics::sample::outcome(&TrackRef { key: "k".into(), title: "t".into(), artists: vec!["a".into()], duration_ms: 214_000, ..Default::default() });
        let Outcome::Found { lyrics: l, .. } = l else { panic!() };
        let s = lyrics::sample::start;
        let at = |pos: u64, reduce: bool, focused: bool| next_frame(&l, &l.active(pos), pos, pos, 400, false, reduce, focused);
        assert_eq!(at(s(0) + 100, false, true), 25, "a word being swept: 40 frames a second in front");
        assert_eq!(at(s(0) + 100, false, false), 50, "20 behind");
        assert_eq!(at(s(0) + 100, true, false), 100, "Reduce Motion: brightening in place needs fewer");
        assert_eq!(at(10_000, false, true), 5_000, "before the lyrics: nothing until the first line (looked at every 5 s)");
        assert_eq!(at(s(0) - 300, false, true), 300);
        assert_eq!(next_frame(&l, &l.active(s(0) + 100), s(0) + 100, s(0) + 100, 400, true, false, true), 5_000, "translated lines are whole: nothing until the next line (at most 5 s)");
        // The theme's word timing: words a little early or late, lines where they were.
        assert_eq!((word_position(10_000, 250), word_position(10_000, -300), word_position(100, -300)), (10_250, 9_700, 0));
        let first = &l.lines[0];
        let (word, end) = (first.tokens[0].start_ms, l.line_end(0));
        let late = word_position(word + 100, -500);
        assert_eq!(next_frame(&l, &l.active(word + 100), word + 100, late, 400, false, false, true), 400, "500 ms late, the first word lights in 400 ms");
        assert!(end > word + 1_000);
    }
}
