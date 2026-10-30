//! The floating player bar: what's playing on the left, transport and
//! progress in the middle, volume / queue / devices and key hints on the right.

use super::icons::Icon;
use super::theme::{self, col, px, sp};
use super::widgets::{self, icon_btn, link, paint_cover, play_btn};
use crate::app::{Action, App, NpView, RightPanel, Route};
use crate::model::{self, s};
use eframe::egui::{self, Align, Align2, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

/// Seek bar or volume bar: a thin line with a knob, accent where filled.
pub fn rail(ui: &mut Ui, id: egui::Id, width: f32, frac: f32) -> (egui::Response, Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, px(16.0)), Sense::hover());
    rail_at(ui, id, rect, frac)
}

fn rail_at(ui: &mut Ui, id: egui::Id, rect: Rect, frac: f32) -> (egui::Response, Option<f32>) {
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let hot = resp.hovered() || resp.dragged();
    let line = Rect::from_center_size(rect.center(), vec2(rect.width(), if hot { 4.0 } else { 3.0 }));
    let mut f = frac.clamp(0.0, 1.0);
    let mut new = None;
    // The release frame counts too: that's when a seek bar acts on a drag.
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked() || resp.drag_stopped())
    {
        f = ((p.x - line.left()) / line.width()).clamp(0.0, 1.0);
        new = Some(f);
    }
    ui.painter().rect_filled(line, egui::CornerRadius::same(2), col::track());
    let filled = Rect::from_min_max(line.min, pos2(line.left() + line.width() * f, line.bottom()));
    ui.painter().rect_filled(filled, egui::CornerRadius::same(2), col::progress());
    ui.painter().circle(pos2(filled.right(), line.center().y), if hot { 6.5 } else { 5.5 }, col::progress(), Stroke::new(1.5, col::player_bg()));
    widgets::focus_ring(ui, &resp, rect, egui::CornerRadius::same(4));
    (resp, new)
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let bar = ui.max_rect();
    ui.painter().rect(bar, egui::CornerRadius::same((theme::radius_f() + 2.0) as u8), col::player_bg(), Stroke::new(1.0, col::border()), StrokeKind::Inside);
    let remote = !app.is_local() && app.pb.device.is_some();
    let local = app.local.as_ref().and_then(|l| l.current().cloned());
    let local_playing = app.local.as_ref().is_some_and(|l| l.is_playing());
    let item = app.pb.item.clone();
    let is_episode = item.as_ref().is_some_and(|i| s(i, "type") == "episode");
    let w = bar.width();
    let pad = sp(3.0);

    // ---------------- left: what's playing
    let left = Rect::from_min_size(bar.min + vec2(pad, 0.0), vec2(w * 0.3 - pad, bar.height()));
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(Layout::left_to_right(Align::Center)), |ui| {
        let cover = px(64.0);
        // With album colors on every page, a new song's colors bloom from this cover.
        if app.settings.album_colors && app.settings.album_everywhere {
            app.album_colors.set_origin(ui.ctx().content_rect(), left.left_center() + vec2(cover / 2.0, 0.0));
        }
        if let Some(t) = &local {
            widgets::cover(ui, t.cover_uri().as_deref(), cover, false);
            ui.add_space(sp(1.0));
            ui.vertical(|ui| {
                ui.set_max_width((left.width() - cover - sp(2.0)).max(0.0));
                ui.add_space(px(14.0));
                ui.add(egui::Label::new(egui::RichText::new(&t.title).font(theme::regular(15.0)).color(col::text())).truncate());
                ui.add(egui::Label::new(egui::RichText::new(&t.artist).font(theme::regular(12.0)).color(col::sub())).truncate());
            });
        } else if let Some(it) = &item {
            let (cr, cresp) = ui.allocate_exact_size(vec2(cover, cover), Sense::click());
            let img = app.cover_of(it, 300);
            let fade = if app.settings.album_colors {
                widgets::paint_cover_fading(ui, egui::Id::new("bar-cover"), cr, img.as_deref(), false)
            } else {
                paint_cover(ui, cr, img.as_deref(), false);
                1.0
            };
            if cresp.on_hover_text(widgets::keys("Now playing  ⌥ ⇧ J")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.act(Action::Navigate(Route::NowPlaying));
            }
            ui.add_space(sp(1.0));
            // Zoomed in far on a narrow window, there may be no room left for the title.
            let text_w = (left.width() - cover - sp(2.0) - px(34.0)).max(0.0);
            ui.vertical(|ui| {
                ui.multiply_opacity(fade);
                ui.set_max_width(text_w);
                ui.add_space(px(14.0));
                if link(ui, s(it, "name"), theme::regular(15.0), col::text()).clicked() {
                    let r = match (s(it, "type"), it.get("album")) {
                        ("episode", _) => Route::Episode(s(it, "id").into()),
                        (_, Some(a)) if !s(a, "id").is_empty() => Route::Album(s(a, "id").into()),
                        _ => Route::Track(s(it, "id").into()),
                    };
                    app.act(Action::Navigate(r));
                }
                // While the DJ talks (and a moment after): what X says.
                match app.dj_caption().map(str::to_string) {
                    Some(line) => {
                        ui.add(egui::Label::new(egui::RichText::new(format!("DJ X · {line}")).font(theme::regular(12.0)).color(col::accent())).truncate()).on_hover_text(line);
                    }
                    None => {
                        ui.horizontal(|ui| widgets::artist_links(app, ui, it, 12.0, col::sub()));
                    }
                }
            });
            let uri = s(it, "uri").to_string();
            let saved = app.is_saved(&uri) == Some(true);
            let tip = match (saved, is_episode) {
                (true, true) => "Remove from Your Episodes",
                (true, false) => "Remove from Liked Songs",
                (false, true) => "Save to Your Episodes",
                (false, false) => "Save to Liked Songs",
            };
            if icon_btn(ui, if saved { Icon::HeartFilled } else { Icon::Heart }, 15.0, tip, saved).clicked() {
                app.act(Action::Save { uri, save: !saved });
            }
        } else {
            widgets::cover(ui, None, cover, false);
            ui.add_space(sp(1.0));
            ui.label(egui::RichText::new("nothing playing").font(theme::regular(13.0)).color(col::muted()));
        }
    });

    // ---------------- center: transport + progress
    let cw = (w * 0.38).clamp(px(300.0), px(560.0));
    let center = Rect::from_center_size(bar.center(), vec2(cw, bar.height()));
    let playing = if local.is_some() { local_playing } else { app.pb.is_playing };
    let row_y = bar.top() + bar.height() * 0.36;
    let btn = |ui: &mut Ui, x: f32, icon: Icon, tip: &str, on: bool| -> bool {
        let r = Rect::from_center_size(pos2(x, row_y), vec2(px(30.0), px(30.0)));
        ui.scope_builder(UiBuilder::new().max_rect(r), |ui| icon_btn(ui, icon, if matches!(icon, Icon::Prev | Icon::Next) { 17.0 } else { 14.0 }, tip, on).clicked()).inner
    };
    let cx = center.center().x;
    let step = px(88.0);
    if is_episode {
        if btn(ui, cx - step * 1.9, Icon::Skip15Back, "Back 15 seconds", false) {
            app.act(Action::SeekBy(-15_000));
        }
        if btn(ui, cx + step * 1.9, Icon::Skip15Fwd, "Forward 15 seconds", false) {
            app.act(Action::SeekBy(15_000));
        }
    } else {
        if btn(ui, cx - step * 1.9, Icon::Shuffle, if app.pb.shuffle { "Shuffle is on  ⌘ S" } else { "Shuffle  ⌘ S" }, app.pb.shuffle) {
            let s = !app.pb.shuffle;
            app.act(Action::SetShuffle(s));
        }
        let (icon, tip, on) = match app.pb.repeat.as_str() {
            "context" => (Icon::Repeat, "Repeat all  ⌘ R", true),
            "track" => (Icon::RepeatOne, "Repeat one  ⌘ R", true),
            _ => (Icon::Repeat, "Repeat  ⌘ R", false),
        };
        if btn(ui, cx + step * 1.9, icon, tip, on) {
            app.act(Action::CycleRepeat);
        }
    }
    if btn(ui, cx - step, Icon::Prev, "Previous  ⌘ ←", false) {
        app.act(Action::Prev);
    }
    if btn(ui, cx + step, Icon::Next, "Next  ⌘ →", false) {
        app.act(Action::Next);
    }
    let pr = Rect::from_center_size(pos2(cx, row_y), vec2(px(46.0), px(46.0)));
    if ui.scope_builder(UiBuilder::new().max_rect(pr), |ui| play_btn(ui, playing, 46.0).clicked()).inner {
        app.act(Action::TogglePlay);
    }
    let (pos, dur) = if let Some(t) = &local {
        (app.local.as_ref().map(|l| l.position_ms()).unwrap_or(0), t.duration_ms)
    } else {
        (app.pb.position(), item.as_ref().map(|i| model::u(i, "duration_ms")).unwrap_or(0))
    };
    let prog_y = bar.top() + bar.height() * 0.73;
    let tw = px(40.0);
    ui.painter().text(pos2(center.left() + tw, prog_y), Align2::RIGHT_CENTER, model::fmt_ms(pos), theme::regular(11.5), col::muted());
    ui.painter().text(pos2(center.right() - tw, prog_y), Align2::LEFT_CENTER, model::fmt_ms(dur), theme::regular(11.5), col::muted());
    let seek = Rect::from_min_max(pos2(center.left() + tw + sp(1.5), prog_y - px(8.0)), pos2(center.right() - tw - sp(1.5), prog_y + px(8.0)));
    let frac = if dur > 0 { pos as f32 / dur as f32 } else { 0.0 };
    let (resp, new) = rail_at(ui, ui.id().with("seek"), seek, frac);
    if let Some(p) = resp.hover_pos() {
        let at = (((p.x - seek.left()) / seek.width()).clamp(0.0, 1.0) * dur as f32) as u64;
        resp.clone().on_hover_text_at_pointer(model::fmt_ms(at));
    }
    if let Some(f) = new {
        if resp.drag_stopped() || resp.clicked() {
            app.scrub_ms = None;
            app.act(Action::Seek((f * dur as f32) as u64));
        } else if resp.dragged() {
            // The lyrics follow the bar while it's held.
            app.scrub_ms = Some(((f * dur as f32) as u64, std::time::Instant::now()));
        }
    }

    // ---------------- right: volume, panels, hints
    let right = Rect::from_min_max(pos2(bar.right() - w * 0.3, bar.top()), pos2(bar.right() - pad, bar.top() + bar.height() * 0.62));
    ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = sp(1.0);
        let open = app.settings.right_open;
        if icon_btn(ui, Icon::Devices, 18.0, "Devices  ⌥ ⇧ D", open && app.right == RightPanel::Devices || remote).clicked() {
            app.act(Action::SetRight(RightPanel::Devices));
        }
        if icon_btn(ui, Icon::Queue, 18.0, "Queue  ⌥ ⇧ Q", open && app.right == RightPanel::Queue).clicked() {
            app.act(Action::SetRight(RightPanel::Queue));
        }
        if icon_btn(ui, Icon::Lyrics, 16.0, "Lyrics  L", false).clicked() {
            app.np_view = NpView::Lyrics;
            app.act(Action::Navigate(Route::NowPlaying));
        }
        ui.add_space(sp(1.0));
        let vol_w = (right.width() * 0.34).clamp(px(70.0), px(136.0));
        let (resp, new) = rail(ui, ui.id().with("vol"), vol_w, app.pb.volume);
        if let Some(v) = new {
            app.pb.volume_dragging = true;
            app.muted_volume = None;
            app.act(Action::Volume(v));
        }
        if resp.drag_stopped() || resp.clicked() {
            app.pb.volume_dragging = false;
            app.act(Action::VolumeCommit);
        }
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
    });
    let hints_y = bar.top() + bar.height() * 0.76;
    let mut x = bar.right() - pad;
    // Hints stop where the progress bar's times begin; a group that doesn't
    // fit is left out rather than drawn over them.
    let limit = center.right() + sp(1.0);
    let group = |ui: &Ui, x: &mut f32, items: &[(&str, bool)]| {
        let need: f32 = items.iter().map(|(t, key)| hint_width(ui, t, *key)).sum();
        if *x - need < limit {
            return false;
        }
        for (t, key) in items {
            hint(ui, x, hints_y, t, *key);
        }
        true
    };
    // Only the keys that work on this screen.
    let lists = matches!(app.route, Route::Library | Route::Playlist(_) | Route::Album(_) | Route::LikedSongs);
    let first: &[(&str, bool)] = if lists {
        &[(if app.route == Route::Library { "open" } else { "play" }, false), ("↵", true), ("move", false), ("j/k", true), ("|", false)]
    } else {
        &[("commands", false), ("⌘ K", true), ("|", false)]
    };
    group(ui, &mut x, first);
    if !group(ui, &mut x, &[("play/pause", false), ("space", true)]) {
        return;
    }
    if !app.is_premium() && !group(ui, &mut x, &[("|", false), ("playback needs spotify premium", false)]) {
        return;
    }
    // Up to 200 px for the device's name, cut short to fit; left out below 80.
    let room = (x - hint_width(ui, "|", false) - limit).min(px(200.0));
    if remote && room >= px(80.0) {
        let name = app.pb.device.as_ref().map(|d| s(d, "name").to_string()).unwrap_or_default();
        hint(ui, &mut x, hints_y, "|", false);
        let r = Rect::from_min_max(pos2(x - room, hints_y - px(9.0)), pos2(x, hints_y + px(9.0)));
        let resp = ui.interact(r, ui.id().with("remote"), Sense::click());
        widgets::text_trunc(ui, pos2(r.left(), hints_y), &format!("playing on {name}"), theme::regular(11.5), col::accent(), r.width());
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.act(Action::SetRight(RightPanel::Devices));
        }
    }
}

/// One key hint, laid out right to left from `x`.
/// A key is measured as its cap shows it (spelled out off macOS: "⌘ K" is "Ctrl+K").
fn hint_width(ui: &Ui, text: &str, key: bool) -> f32 {
    let (text, font) = if key { (widgets::keys(text), theme::regular(11.0)) } else { (text.into(), theme::regular(11.5)) };
    ui.painter().layout_no_wrap(text.into_owned(), font, col::muted()).size().x + if key { sp(1.25) } else { 0.0 } + sp(1.0)
}

fn hint(ui: &Ui, x: &mut f32, y: f32, text: &str, key: bool) {
    if key {
        let g = ui.painter().layout_no_wrap(widgets::keys(text).into_owned(), theme::regular(11.0), col::sub());
        *x -= g.size().x + sp(1.25);
        widgets::keycap(ui, pos2(*x, y), text);
    } else {
        let font = theme::regular(11.5);
        let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), col::muted());
        *x -= g.size().x;
        ui.painter().text(pos2(*x, y), Align2::LEFT_CENTER, text, font, col::muted());
    }
    *x -= sp(1.0);
}
