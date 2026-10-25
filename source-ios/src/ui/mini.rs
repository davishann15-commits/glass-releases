//! Miniplayer window, full-screen now-playing, and the sign-in screen.

use super::icons::Icon;
use super::nowbar::rail;
use super::theme::{self, col};
use super::widgets::{icon_btn, paint_cover, play_btn};
use crate::app::{Action, App, AuthState};
use crate::model::{self, s};
use eframe::egui::{self, Align, Color32, Layout, Rect, RichText, Sense, Ui, ViewportBuilder, ViewportId, vec2};

fn now_text(app: &App) -> (String, String, Option<String>, u64, u64, bool) {
    if let Some(l) = app.local.as_ref().filter(|l| l.active)
        && let Some(t) = l.current()
    {
        return (t.title.clone(), t.artist.clone(), t.cover_uri(), l.position_ms(), t.duration_ms, l.is_playing());
    }
    match &app.pb.item {
        Some(i) => (s(i, "name").into(), model::artists_line(i), model::image(i, 640), app.pb.position(), model::u(i, "duration_ms"), app.pb.is_playing),
        None => ("nothing playing".into(), String::new(), None, 0, 0, false),
    }
}

fn controls(app: &mut App, ui: &mut Ui, big: bool) {
    let sz = if big { 24.0 } else { 18.0 };
    let (_, _, _, pos, dur, playing) = now_text(app);
    ui.horizontal(|ui| {
        let total = if big { 5.0 * (sz + 8.0) + 64.0 + 40.0 } else { 5.0 * (sz + 8.0) + 40.0 + 40.0 };
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        if icon_btn(ui, Icon::Shuffle, sz, "Shuffle", app.pb.shuffle).clicked() {
            let sh = !app.pb.shuffle;
            app.act(Action::SetShuffle(sh));
        }
        if icon_btn(ui, Icon::Prev, sz, "Previous", false).clicked() {
            app.act(Action::Prev);
        }
        if play_btn(ui, playing, if big { 64.0 } else { 40.0 }).clicked() {
            app.act(Action::TogglePlay);
        }
        if icon_btn(ui, Icon::Next, sz, "Next", false).clicked() {
            app.act(Action::Next);
        }
        let rep = app.pb.repeat.clone();
        if icon_btn(ui, if rep == "track" { Icon::RepeatOne } else { Icon::Repeat }, sz, "Repeat", rep != "off").clicked() {
            app.act(Action::CycleRepeat);
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(model::fmt_ms(pos)).size(11.0).color(col::sub()));
        let w = (ui.available_width() - 44.0).max(0.0);
        let frac = if dur > 0 { pos as f32 / dur as f32 } else { 0.0 };
        let (resp, new) = rail(ui, ui.id().with("mini-seek"), w, frac);
        if let Some(f) = new
            && (resp.drag_stopped() || resp.clicked())
        {
            app.act(Action::Seek((f * dur as f32) as u64));
        }
        ui.label(RichText::new(model::fmt_ms(dur)).size(11.0).color(col::sub()));
    });
}

pub fn mini_player(app: &mut App, ctx: &egui::Context) {
    let builder = ViewportBuilder::default()
        .with_title("Glass Miniplayer")
        .with_inner_size([300.0, 380.0])
        .with_min_inner_size([240.0, 300.0])
        .with_always_on_top()
        .with_transparent(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_fullsize_content_view(true);
    ctx.show_viewport_immediate(ViewportId::from_hash_of("mini"), builder, |ui, _class| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            app.mini = false;
        }
        let full = ui.max_rect();
        ui.painter().rect_filled(full, theme::radius(), col::overlay());
        let drag = ui.interact(full, ui.id().with("drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        egui::Frame::NONE.inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            let (title, artist, img, ..) = now_text(app);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if icon_btn(ui, Icon::Close, 12.0, "Close Miniplayer", false).clicked() {
                    app.mini = false;
                }
            });
            let w = ui.available_width();
            let side = (ui.available_height() - 110.0).min(w).max(80.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                paint_cover(ui, r, img.as_deref(), false);
            });
            ui.add_space(theme::sp(0.75));
            ui.label(RichText::new(title).font(theme::regular(15.0)).color(col::text()));
            ui.label(RichText::new(artist).size(12.5).color(col::sub()));
            controls(app, ui, false);
        });
    });
}

pub fn fullscreen(app: &mut App, ui: &mut Ui) {
    let (title, artist, img, ..) = now_text(app);
    let full = ui.max_rect();
    if let Some(u) = &img {
        // Large, dimmed cover behind everything, cropped to the window (not stretched).
        super::widgets::mark_drawn(ui.ctx(), u);
        let side = full.width().max(full.height());
        ui.scope(|ui| {
            ui.set_clip_rect(full);
            egui::Image::new(u.clone()).tint(Color32::from_white_alpha(40)).paint_at(ui, Rect::from_center_size(full.center(), vec2(side, side)));
        });
    }
    let side = (full.height() * 0.5).min(full.width() * 0.45);
    let area = Rect::from_center_size(full.center(), vec2(full.width() * 0.8, side + 220.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            paint_cover(ui, r, img.as_deref(), false);
            ui.add_space(theme::sp(2.25));
            ui.label(RichText::new(title).font(theme::regular(36.0)).color(col::text()));
            ui.label(RichText::new(artist).size(18.0).color(col::sub()));
            ui.add_space(theme::sp(1.5));
            ui.set_max_width(side * 1.6);
            controls(app, ui, true);
        });
    });
    let corner = Rect::from_min_size(full.right_top() + vec2(-60.0, 20.0), vec2(40.0, 40.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(corner), |ui| {
        if icon_btn(ui, Icon::ExitFullscreen, 20.0, "Exit full screen  esc", false).clicked() {
            app.act(Action::ToggleFullscreen);
        }
    });
}

pub fn sign_in(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let drag = ui.interact(Rect::from_min_size(full.min, vec2(full.width(), 40.0)), ui.id().with("drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    let card = Rect::from_center_size(full.center(), vec2(440.0, 400.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        theme::panel().inner_margin(egui::Margin::same(28)).show(ui, |ui| {
            ui.set_width(card.width() - 56.0);
            ui.label(RichText::new("Glass").font(theme::regular(34.0)));
            ui.label(RichText::new("A quiet, transparent Spotify player.").color(col::sub()));
            ui.add_space(theme::sp(2.0));
            let busy = app.auth_state == AuthState::SigningIn;
            let label = if busy { "Waiting for your browser…" } else { "Sign in with Spotify" };
            let btn =
                egui::Button::new(RichText::new(label).font(theme::regular(15.0)).color(col::on_accent())).fill(col::accent()).corner_radius(egui::CornerRadius::same(22)).min_size(vec2(0.0, 44.0));
            if ui.vertical_centered_justified(|ui| ui.add_enabled(!busy, btn)).inner.clicked() {
                app.act(Action::SignIn);
            }
            if busy {
                ui.add_space(theme::sp(0.75));
                if ui.link(RichText::new("Cancel").size(12.5)).clicked() {
                    app.act(Action::CancelSignIn);
                }
            }
            if let Some(e) = &app.auth_error {
                ui.add_space(theme::sp(1.0));
                ui.label(RichText::new(e).color(col::error()).size(12.5));
            }
            ui.add_space(theme::sp(2.25));
            let stored = format!("Glass keeps your session in {}, readable only by you.", crate::config::config_dir().display());
            for line in [
                "Your browser opens Spotify's own sign-in page. Glass never sees your password.",
                stored.as_str(),
                "Playing music needs Spotify Premium. The Activity page lists who Glass talks to and logs its Spotify and lyrics requests.",
            ] {
                ui.label(RichText::new(format!("•  {line}")).size(12.5).color(col::sub()));
                ui.add_space(theme::sp(0.25));
            }
        });
    });
}
