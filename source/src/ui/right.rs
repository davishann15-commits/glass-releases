//! The side panel: queue, devices, lyrics and the assistant, beside the page.

use super::icons::{self, Icon};
use super::menus;
use super::theme::{self, col, sp};
use super::widgets::{self, icon_btn, link, paint_cover};
use crate::app::{Action, App, NpView, RightPanel, Route};
use crate::model::{self, s};
use eframe::egui::{self, Align, Layout, Margin, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use serde_json::Value;

pub fn show(app: &mut App, ui: &mut Ui) {
    let title = match app.right {
        RightPanel::Queue => "queue",
        RightPanel::Devices => "devices",
        RightPanel::Lyrics => "lyrics",
        RightPanel::Assistant => "assistant",
    };
    let pad = sp(2.25) as i8;
    egui::Frame::NONE.inner_margin(Margin::symmetric(pad, sp(1.5) as i8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).font(theme::regular(15.0)).color(col::text()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_btn(ui, Icon::Close, 12.0, "Close  esc", false).clicked() {
                    app.act(Action::ToggleRight);
                }
                // The other views, one click away.
                for (p, icon, tip) in [
                    (RightPanel::Assistant, Icon::Assistant, "Assistant  ⌘ J"),
                    (RightPanel::Devices, Icon::Devices, "Devices"),
                    (RightPanel::Lyrics, Icon::Lyrics, "Lyrics"),
                    (RightPanel::Queue, Icon::Queue, "Queue"),
                ] {
                    if icon_btn(ui, icon, 13.0, tip, app.right == p).clicked() {
                        app.right = p;
                    }
                }
                if icon_btn(ui, Icon::NowPlaying, 13.0, "Now playing  ⌥ ⇧ J", false).clicked() {
                    app.act(Action::Navigate(Route::NowPlaying));
                }
            });
        });
    });
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([pos2(r.left(), r.top()), pos2(r.right(), r.top())], theme::hairline());
    if app.right == RightPanel::Assistant {
        // Its own layout: the composer stays at the bottom.
        return super::assistant::panel(app, ui);
    }
    ScrollArea::vertical().id_salt(("right", app.right as u8)).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(Margin { left: pad, right: pad, top: sp(1.5) as i8, bottom: pad }).show(ui, |ui| match app.right {
            RightPanel::Queue => queue(app, ui),
            RightPanel::Devices => devices(app, ui),
            RightPanel::Lyrics => lyrics(app, ui),
            RightPanel::Assistant => {}
        });
    });
}

/// Lyrics in the side panel: the current line is bright with an accent mark;
/// click a line to jump there. The full view is on the now-playing screen.
fn lyrics(app: &mut App, ui: &mut Ui) {
    use crate::lyrics::Outcome;
    use crate::lyrics::service::Status;
    if app.lyrics_track().is_none() {
        widgets::unavailable(ui, "nothing playing", "Lyrics follow the song that's playing.");
        return;
    }
    let res = match &app.lyrics.status {
        Status::Done(res) => res.clone(),
        _ => return widgets::loading(ui),
    };
    let ly = match &res.outcome {
        Outcome::Found { lyrics, .. } => lyrics,
        other => {
            let (title, body) = match other {
                Outcome::Instrumental { .. } => ("instrumental".to_string(), "LRCLIB lists this recording as instrumental.".to_string()),
                Outcome::NotFound { provider } => ("no lyrics found".to_string(), format!("{} has no lyrics that match this recording.", provider.label())),
                Outcome::Ambiguous { .. } => ("several versions match".to_string(), "Choose one on the now-playing screen.".to_string()),
                Outcome::Unavailable { provider, reason } => (format!("{} lyrics aren't available", provider.label()), reason.clone()),
                Outcome::Failed { error, .. } => ("couldn't load lyrics".to_string(), error.clone()),
                Outcome::Found { .. } => unreachable!(),
            };
            widgets::unavailable(ui, &title, &body);
            ui.add_space(sp(1.5));
            ui.vertical_centered(|ui| {
                if widgets::bracket_button(ui, None, "open now playing", false).clicked() {
                    app.np_view = NpView::Lyrics;
                    app.act(Action::Navigate(Route::NowPlaying));
                }
            });
            return;
        }
    };
    let offset = app.lyrics_offset_total();
    let pos = app.lyrics_position();
    let current = ly.line_at(pos);
    let synced = ly.timing != crate::lyrics::Timing::Plain;
    // Back when the next line starts, rather than on a timer.
    let playing = app.pb.is_playing || app.local.as_ref().is_some_and(|l| l.is_playing());
    if synced
        && playing
        && let Some(next) = ly.lines.iter().map(|l| l.start_ms).find(|&s| s > pos)
    {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis((next - pos).clamp(30, 5_000)));
    }
    let last_shown = ui.ctx().data(|d| d.get_temp::<usize>(egui::Id::new("lyrics-line")));
    // Translation, as on the now-playing screen.
    let mode = app.config.lyrics().translation;
    let tr_state = app.lyrics_translation(ly);
    let tr: Option<&[String]> = match tr_state.as_ref().and_then(Option::as_deref) {
        Some(crate::lyrics::translate::Translation::Ready { lines, .. }) => Some(lines),
        _ => None,
    };
    for (i, l) in ly.lines.iter().enumerate() {
        let t = tr.as_ref().and_then(|t| t.get(i)).map(|t| t.trim()).filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case(l.text.trim()));
        let shown = match (mode, t) {
            (crate::lyrics::translate::Mode::Only, Some(t)) => t,
            _ => l.text.as_str(),
        };
        let words = if shown.trim().is_empty() { "♪" } else { shown };
        let color = match current {
            Some(c) if c == i => col::text(),
            Some(c) if i < c => col::sub(),
            _ if !synced => col::text(),
            _ => col::muted(),
        };
        let resp = ui.add(egui::Label::new(RichText::new(words).font(theme::regular(if Some(i) == current { 16.5 } else { 15.0 })).color(color)).wrap().sense(Sense::click()));
        if Some(i) == current {
            ui.painter().line_segment([pos2(resp.rect.left() - sp(1.25), resp.rect.top()), pos2(resp.rect.left() - sp(1.25), resp.rect.bottom())], egui::Stroke::new(2.0, col::accent()));
            if last_shown != Some(i) {
                resp.scroll_to_me(Some(Align::Center));
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("lyrics-line"), i));
            }
        }
        if synced && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.act(Action::Seek(crate::lyrics::seek_target(l.start_ms, offset)));
        }
        if mode == crate::lyrics::translate::Mode::Below
            && let Some(t) = t
        {
            ui.add(egui::Label::new(RichText::new(t).font(theme::regular(12.5)).color(col::muted())).wrap());
        }
        ui.add_space(sp(0.75));
    }
    ui.add_space(sp(2.0));
    ui.label(RichText::new(format!("{} · {}", ly.source.provider.name(), ly.timing.label())).font(theme::regular(11.0)).color(col::muted()));
}

/// The name of the playlist, album, artist or show that's playing, once known.
pub(super) fn context_name(app: &mut App) -> Option<String> {
    let ctx = app.pb.context_uri.clone();
    if ctx.is_empty() {
        return None;
    }
    if ctx.ends_with(":collection") {
        return Some("Liked Songs".into());
    }
    let path = match model::kind_of(&ctx) {
        "playlist" => format!("/playlists/{}?fields=name", model::id_of(&ctx)),
        "album" => format!("/albums/{}", model::id_of(&ctx)),
        "artist" => format!("/artists/{}", model::id_of(&ctx)),
        "show" => format!("/shows/{}", model::id_of(&ctx)),
        _ => return None,
    };
    app.want(&path).map(|v| s(&v, "name").to_string()).filter(|n| !n.is_empty())
}

/// One queue row. A click on the playing song pauses or resumes it; on a
/// later song, it plays from there (that song and everything after it).
fn queue_row(app: &mut App, ui: &mut Ui, it: &Value, current: bool, from_here: &[String], cursor: bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s(it, "name")));
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
    }
    if cursor {
        ui.painter().rect_stroke(rect, theme::small_radius(), egui::Stroke::new(1.0, col::focus()), egui::StrokeKind::Inside);
        ui.scroll_to_rect(rect, None);
    }
    let c = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 22.0), vec2(44.0, 44.0));
    paint_cover(ui, c, model::image(it, 64).as_deref(), false);
    let x = c.right() + 10.0;
    let max_w = rect.right() - x - 8.0;
    widgets::text_trunc(ui, pos2(x, rect.center().y - 9.0), s(it, "name"), theme::regular(14.0), if current { col::accent() } else { col::text() }, max_w);
    widgets::text_trunc(ui, pos2(x, rect.center().y + 10.0), &model::artists_line(it), theme::regular(12.5), col::sub(), max_w);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        jump(app, current, from_here);
    }
    resp.context_menu(|ui| menus::track(app, ui, it, None, &[]));
}

fn jump(app: &mut App, current: bool, from_here: &[String]) {
    if current {
        app.act(Action::TogglePlay);
    } else if !from_here.is_empty() {
        app.act(Action::PlayUris { uris: from_here.to_vec(), index: 0 });
    }
}

pub(super) fn queue(app: &mut App, ui: &mut Ui) {
    let Some(q) = app.queue.clone() else {
        match app.queue_error.clone() {
            // Right after connecting there's no player state yet; it's on its way.
            Some(e) if !e.contains("isn't ready") => {
                widgets::unavailable(ui, "couldn't load the queue", e.trim_start_matches(|c: char| c.is_ascii_digit() || c == ' '));
                ui.add_space(theme::sp(1.0));
                ui.vertical_centered(|ui| {
                    if widgets::bracket_button(ui, None, "try again", false).clicked() {
                        app.poll_soon();
                    }
                });
            }
            _ => widgets::loading(ui),
        }
        return;
    };
    let current = q.get("currently_playing").filter(|v| !v.is_null());
    let items = model::arr(&q, "queue");
    let uris: Vec<String> = items.iter().map(|i| s(i, "uri").to_string()).collect();
    // On the now-playing screen the queue takes j/k/↑/↓ and Enter.
    let keys = app.route == crate::app::Route::NowPlaying && !ui.ctx().egui_wants_keyboard_input() && !egui::Popup::is_any_open(ui.ctx());
    let cursor_id = egui::Id::new("queue-cursor");
    let mut cursor: Option<usize> = ui.ctx().data(|d| d.get_temp::<Option<usize>>(cursor_id)).flatten();
    if keys && !items.is_empty() {
        let none = egui::Modifiers::NONE;
        let (down, up, enter) = ui.input_mut(|i| {
            (
                i.consume_key(none, egui::Key::J) || i.consume_key(none, egui::Key::ArrowDown),
                i.consume_key(none, egui::Key::K) || i.consume_key(none, egui::Key::ArrowUp),
                i.consume_key(none, egui::Key::Enter),
            )
        });
        let last = items.len() - 1;
        cursor = match (down, up, cursor) {
            (true, _, None) | (_, true, None) => Some(0),
            (true, _, Some(c)) => Some((c + 1).min(last)),
            (_, true, Some(c)) => Some(c.saturating_sub(1)),
            (_, _, c) => c.map(|c| c.min(last)),
        };
        if enter && let Some(c) = cursor {
            jump(app, false, &uris[c..]);
            cursor = None;
        }
        ui.ctx().data_mut(|d| d.insert_temp(cursor_id, cursor));
    }
    if let Some(cur) = current {
        ui.label(RichText::new("Now playing").font(theme::regular(13.5)));
        queue_row(app, ui, cur, true, &[], false);
        ui.add_space(theme::sp(1.25));
    }
    if items.is_empty() {
        widgets::unavailable(ui, "Add to your queue", "Right-click a song and choose Add to queue.");
        return;
    }
    ui.horizontal(|ui| {
        let next = match context_name(app) {
            Some(name) => format!("Next from: {name}"),
            None => "Next up".to_string(),
        };
        ui.label(RichText::new(next).font(theme::regular(13.5)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if link(ui, "Clear queue", theme::medium(12.5), col::sub()).clicked() {
                app.modal = Some(crate::app::Modal::Confirm { title: "Clear queue?".into(), body: "You can't undo this.".into(), confirm: "Clear".into(), action: Box::new(Action::ClearQueue) });
            }
        });
    });
    for (i, it) in items.iter().enumerate() {
        queue_row(app, ui, it, false, &uris[i..], keys && cursor == Some(i));
    }
}

fn device_icon(kind: &str) -> Icon {
    match kind.to_lowercase().as_str() {
        "computer" => Icon::Devices,
        "smartphone" | "tablet" => Icon::Mini,
        "speaker" | "avr" | "stb" | "audiodongle" | "castaudio" => Icon::VolumeHigh,
        "tv" | "castvideo" | "gameconsole" => Icon::NowPlaying,
        _ => Icon::Devices,
    }
}

pub(super) fn devices(app: &mut App, ui: &mut Ui) {
    let own = app.engine_device().map(str::to_string);
    let list: Vec<Value> = app.devices.as_ref().map(|d| model::arr(d, "devices").to_vec()).unwrap_or_default();
    let active = list.iter().find(|d| model::b(d, "is_active")).cloned();

    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Current device").size(12.0).color(col::sub()));
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
            icons::paint(ui.painter(), r, if active.is_some() { device_icon(active.as_ref().map(|d| s(d, "type")).unwrap_or("")) } else { Icon::Devices }, col::accent());
            let name = match &active {
                Some(d) => s(d, "name").to_string(),
                None if app.engine.is_some() => format!("{} (this computer)", app.settings.device_name),
                None => "No active device".into(),
            };
            ui.label(RichText::new(name).font(theme::regular(15.0)).color(col::accent()));
        });
        widgets::subtle(ui, &app.engine_status);
    });
    ui.add_space(theme::sp(1.25));
    ui.label(RichText::new("Select another device").font(theme::regular(13.5)));
    ui.add_space(theme::sp(0.5));
    let (mut shown_own, mut rows) = (false, 0);
    for d in list.iter().filter(|d| !model::b(d, "is_active")) {
        let id = s(d, "id").to_string();
        let is_own = own.as_deref() == Some(id.as_str());
        shown_own |= is_own;
        device_row(app, ui, d, is_own);
        rows += 1;
    }
    if !shown_own
        && active.as_ref().map(|d| Some(s(d, "id")) != own.as_deref()).unwrap_or(false)
        && let Some(o) = own.clone()
    {
        let fake = serde_json::json!({ "id": o, "name": app.settings.device_name, "type": "Computer", "is_restricted": false });
        device_row(app, ui, &fake, true);
        rows += 1;
    }
    if rows == 0 {
        widgets::subtle(ui, "No other devices found. Open Spotify on another device on your account and it'll appear here.");
    }
}

fn device_row(app: &mut App, ui: &mut Ui, d: &Value, is_own: bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(22.0, 22.0)), device_icon(s(d, "type")), col::text());
    let name = if is_own { format!("{} (this computer)", s(d, "name")) } else { s(d, "name").to_string() };
    widgets::text_trunc(ui, pos2(rect.left() + 48.0, rect.center().y - 8.0), &name, theme::medium(14.0), col::text(), rect.width() - 60.0);
    let mut sub = s(d, "type").to_string();
    if let Some(v) = d["volume_percent"].as_u64() {
        sub = format!("{sub} • volume {v}%");
    }
    if model::b(d, "is_restricted") {
        sub = format!("{sub} • can't be controlled remotely");
    }
    widgets::text_trunc(ui, pos2(rect.left() + 48.0, rect.center().y + 10.0), &sub, theme::regular(12.0), col::sub(), rect.width() - 60.0);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !model::b(d, "is_restricted") {
        app.act(Action::Transfer(s(d, "id").to_string()));
    }
}
