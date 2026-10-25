//! Reusable pieces in the terminal-glass style: icon buttons, outlined chips
//! and `[ bracket ]` buttons, covers, cards, caps labels, links, headers.
//! Colors and sizes come from the theme.

use super::icons::{self, Icon};
use super::theme::{self, col, px, sp};
use crate::app::{Action, App, Route};
use crate::model::{self, s};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Image, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use serde_json::Value;

/// A visible ring around whatever has keyboard focus.
pub fn focus_ring(ui: &Ui, resp: &Response, rect: Rect, radius: CornerRadius) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), radius, Stroke::new(1.5, col::focus()), StrokeKind::Outside);
    }
}

/// A shortcut as this system writes it: ⌘ ⌥ ⇧ ⌃ on macOS; elsewhere Ctrl, Alt and
/// Shift ("Search  ⌘ L" is "Search  Ctrl+L", where ⌘ shortcuts are Ctrl ones).
pub fn keys(s: &str) -> std::borrow::Cow<'_, str> {
    if cfg!(target_os = "macos") || !s.contains(['⌘', '⌥', '⇧', '⌃']) {
        return s.into();
    }
    spelled_out(s).into()
}

/// A path with the home folder written `~` ("~/Music/Glass").
pub fn home_relative(path: &std::path::Path) -> String {
    match std::env::var_os("HOME").and_then(|h| path.strip_prefix(h).ok().map(|rest| format!("~/{}", rest.display()))) {
        Some(short) => short,
        None => path.display().to_string(),
    }
}

fn spelled_out(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 12);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        let name = match c {
            '⌘' | '⌃' => "Ctrl+",
            '⌥' => "Alt+",
            '⇧' => "Shift+",
            _ => {
                out.push(c);
                continue;
            }
        };
        // ⌃ ⌘ is one Ctrl here.
        if !out.ends_with(name) {
            out.push_str(name);
        }
        while chars.peek() == Some(&' ') {
            chars.next();
        }
    }
    out
}

pub fn icon_btn(ui: &mut Ui, icon: Icon, size: f32, tip: &str, active: bool) -> Response {
    let size = px(size);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size + sp(1.0)), Sense::click());
    let resp = resp.on_hover_text(keys(tip)).on_hover_cursor(CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
    }
    let color = if active {
        col::accent()
    } else if resp.hovered() {
        col::text()
    } else {
        col::sub()
    };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(size)), icon, color);
    focus_ring(ui, &resp, rect, theme::small_radius());
    resp
}

/// The round play/pause button: an accent ring, filled faintly on hover.
pub fn play_btn(ui: &mut Ui, playing: bool, size: f32) -> Response {
    let size = px(size);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let resp = resp.on_hover_text(if playing { "Pause" } else { "Play" }).on_hover_cursor(CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, if playing { "Pause" } else { "Play" }));
    let fill = if resp.hovered() { col::accent_a(0.18) } else { col::accent_a(0.06) };
    ui.painter().circle(rect.center(), size / 2.0 - 1.0, fill, Stroke::new(1.5, col::accent()));
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size * 0.42));
    icons::paint(ui.painter(), icon_rect, if playing { Icon::Pause } else { Icon::Play }, col::accent());
    focus_ring(ui, &resp, rect, CornerRadius::same((size / 2.0) as u8));
    resp
}

pub fn cover(ui: &mut Ui, url: Option<&str>, size: f32, round: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_cover(ui, rect, url, round);
    resp
}

thread_local! {
    /// When each cover was last drawn (egui's clock), to free the unused ones.
    static DRAWN: std::cell::RefCell<std::collections::HashMap<String, f64>> = Default::default();
}

/// Covers kept in memory even when unused.
const KEEP_IMAGES: usize = 120;
/// How long an unused cover stays in memory.
const IMAGE_IDLE_S: f64 = 60.0;

/// Notes that `uri` is on screen, so `forget_unused_images` keeps it. Every
/// image Glass draws from a URL goes through here.
pub fn mark_drawn(ctx: &egui::Context, uri: &str) {
    let now = ctx.input(|i| i.time);
    DRAWN.with(|d| {
        let mut d = d.borrow_mut();
        match d.get_mut(uri) {
            Some(t) => *t = now,
            None => {
                d.insert(uri.to_string(), now);
            }
        }
    });
}

/// Frees covers (downloaded bytes, decoded pixels and GPU texture) that
/// haven't been drawn for a minute. egui would otherwise keep every image it
/// ever loaded for the life of the app.
pub fn forget_unused_images(ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    DRAWN.with(|d| {
        let mut d = d.borrow_mut();
        if d.len() <= KEEP_IMAGES {
            return;
        }
        let old: Vec<String> = d.iter().filter(|(_, t)| now - **t > IMAGE_IDLE_S).map(|(u, _)| u.clone()).collect();
        for u in old {
            d.remove(&u);
            ctx.forget_image(&u);
        }
    });
}

pub fn paint_cover(ui: &Ui, rect: Rect, url: Option<&str>, round: bool) {
    // Off-screen covers aren't drawn, so they aren't downloaded or decoded either.
    if !ui.is_rect_visible(rect) {
        return;
    }
    let r = cover_radius(rect, round);
    ui.painter().rect_filled(rect, r, col::raised());
    cover_layer(ui, rect, url, r, 1.0);
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, col::divider()), StrokeKind::Inside);
}

fn cover_radius(rect: Rect, round: bool) -> CornerRadius {
    if round { CornerRadius::same((rect.width() / 2.0).min(255.0) as u8) } else { CornerRadius::same((theme::radius_f() * 0.5).clamp(2.0, rect.width() * 0.1) as u8) }
}

/// The cover image (or the placeholder note) at `alpha`, without the frame.
fn cover_layer(ui: &Ui, rect: Rect, url: Option<&str>, r: CornerRadius, alpha: f32) {
    let note = |ui: &Ui| icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(rect.width() * 0.36)), Icon::Music, col::muted().gamma_multiply(alpha));
    match url {
        Some(u) if !u.is_empty() => {
            mark_drawn(ui.ctx(), u);
            let img = Image::new(u.to_string()).corner_radius(r).show_loading_spinner(false).tint(Color32::WHITE.gamma_multiply(alpha));
            // A cover that can't be read shows the placeholder, not an error sign.
            if img.load_for_size(ui.ctx(), rect.size()).is_err() {
                note(ui);
            } else {
                img.paint_at(ui, rect);
            }
        }
        _ => note(ui),
    }
}

/// Where a fading cover is: the one settled on screen, and the one coming in.
#[derive(Clone, Default)]
struct CoverFade {
    shown: Option<String>,
    next: Option<String>,
    /// When `next` was asked for, and when it started fading in.
    asked: f64,
    since: Option<f64>,
}

/// How long a new cover takes to fade in, and the longest it waits to load first.
const COVER_FADE: f64 = 0.55;
const COVER_WAIT: f64 = 0.7;

/// `paint_cover` for what's playing: a new cover fades in over the old one once it has
/// loaded (so there's no empty frame between songs). Returns how far the newest cover has
/// faded in (0–1), for the title to follow.
pub fn paint_cover_fading(ui: &Ui, id: egui::Id, rect: Rect, url: Option<&str>, round: bool) -> f32 {
    if !ui.is_rect_visible(rect) {
        return 1.0;
    }
    let ctx = ui.ctx().clone();
    let now = ui.input(|i| i.time);
    let mut fade: CoverFade = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let url = url.filter(|u| !u.is_empty()).map(str::to_string);
    if url.is_none() {
        // No cover (a local file without artwork): show the placeholder, not the last song's.
        fade = CoverFade::default();
    } else if fade.shown.is_none() && fade.next.is_none() && fade.since.is_none() {
        // First sight: no fade.
        fade.shown = url.clone();
    } else if url != fade.shown && url != fade.next {
        fade.next = url.clone();
        fade.asked = now;
        fade.since = None;
    }
    if fade.next.is_some() && fade.since.is_none() {
        let ready = match &fade.next {
            Some(u) => !matches!(Image::new(u.clone()).load_for_size(&ctx, rect.size()), Ok(egui::load::TexturePoll::Pending { .. })),
            None => true,
        };
        if ready || now - fade.asked > COVER_WAIT {
            fade.since = Some(now);
        }
    }
    let k = match fade.since {
        _ if theme::reduce_motion() => 1.0,
        Some(since) => (((now - since) / COVER_FADE).clamp(0.0, 1.0)) as f32,
        None if fade.next.is_some() => 0.0,
        None => 1.0,
    };
    let eased = k * k * (3.0 - 2.0 * k);
    let r = cover_radius(rect, round);
    ui.painter().rect_filled(rect, r, col::raised());
    cover_layer(ui, rect, fade.shown.as_deref(), r, 1.0);
    if fade.since.is_some() && fade.next.is_some() {
        cover_layer(ui, rect, fade.next.as_deref(), r, eased);
    }
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, col::divider()), StrokeKind::Inside);
    if k >= 1.0 && fade.since.is_some() {
        fade.shown = fade.next.take();
        fade.since = None;
    }
    if fade.next.is_some() {
        ctx.request_repaint();
    }
    let shown = if fade.next.is_some() { eased } else { 1.0 };
    ctx.data_mut(|d| d.insert_temp(id, fade));
    shown
}

/// Outlined filter chip; the selected one takes the accent. Returns clicked.
pub fn chip(ui: &mut Ui, text: &str, selected: bool) -> bool {
    let font = theme::regular(12.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), col::text());
    let size = vec2(galley.size().x + sp(3.25), px(32.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, text));
    let (fill, stroke, color) = if selected {
        (col::accent_a(0.10), Stroke::new(1.2, col::accent()), col::text())
    } else if resp.hovered() {
        (col::hover(), Stroke::new(1.0, col::border()), col::text())
    } else {
        (Color32::TRANSPARENT, Stroke::new(1.0, col::divider()), col::sub())
    };
    ui.painter().rect(rect, theme::small_radius(), fill, stroke, StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    focus_ring(ui, &resp, rect, theme::small_radius());
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// `▶ [ play ]`: the bracketed action buttons. `primary` takes the accent.
pub fn bracket_button(ui: &mut Ui, icon: Option<Icon>, label: &str, primary: bool) -> Response {
    let font = theme::regular(14.0);
    let text = format!("[ {label} ]");
    let galley = ui.painter().layout_no_wrap(text.clone(), font.clone(), col::text());
    let icon_w = if icon.is_some() { px(16.0) + sp(1.25) } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + sp(4.5), px(40.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let hovered = resp.hovered();
    let (fill, stroke, color) = match (primary, hovered) {
        (true, false) => (col::accent_a(0.12), Stroke::new(1.2, col::accent()), col::accent()),
        (true, true) => (col::accent_a(0.22), Stroke::new(1.2, col::accent()), col::accent()),
        (false, false) => (Color32::TRANSPARENT, Stroke::new(1.0, col::border()), col::text()),
        (false, true) => (col::hover(), Stroke::new(1.0, col::sub()), col::text()),
    };
    ui.painter().rect(rect, theme::small_radius(), fill, stroke, StrokeKind::Inside);
    let mut x = rect.left() + sp(2.25);
    if let Some(i) = icon {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(x + px(8.0), rect.center().y), Vec2::splat(px(15.0))), i, color);
        x += icon_w;
    }
    ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, font, color);
    focus_ring(ui, &resp, rect, theme::small_radius());
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// `PLAYLIST`, `FEATURED ARTISTS`: small spaced capitals.
pub fn caps(ui: &Ui, left_center: egui::Pos2, text: &str, size: f32, color: Color32) -> f32 {
    let mut job = egui::text::LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, egui::TextFormat { font_id: theme::regular(size), color, extra_letter_spacing: px(size) * 0.18, ..Default::default() });
    let galley = ui.painter().layout_job(job);
    let w = galley.size().x;
    ui.painter().galley(left_center - vec2(0.0, galley.size().y / 2.0), galley, color);
    w
}

/// A caps label that takes up layout space.
pub fn caps_label(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    let h = px(size) * 1.6;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    caps(ui, pos2(rect.left(), rect.center().y), text, size, color);
}

/// A text label that underlines on hover and returns its response.
pub fn link(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> Response {
    let resp = ui.add(egui::Label::new(RichText::new(text).font(font.clone()).color(color)).sense(Sense::click()).truncate());
    if resp.hovered() {
        let r = resp.rect;
        ui.painter().line_segment([pos2(r.left(), r.bottom() - 1.0), pos2(r.right(), r.bottom() - 1.0)], Stroke::new(1.0, color));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Comma-separated, individually clickable artist names.
pub fn artist_links(app: &mut App, ui: &mut Ui, item: &Value, size: f32, color: Color32) {
    let artists = model::arr(item, "artists");
    if artists.is_empty() {
        if let Some(show) = item.get("show") {
            if link(ui, s(show, "name"), theme::regular(size), color).clicked() {
                app.act(Action::Navigate(Route::Show(s(show, "id").into())));
            }
        } else {
            ui.label(RichText::new(model::artists_line(item)).font(theme::regular(size)).color(color));
        }
        return;
    }
    // Names sit close together; the caller's spacing comes back after them.
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 0.0;
    for (i, a) in artists.iter().enumerate() {
        if i > 0 {
            ui.label(RichText::new(", ").font(theme::regular(size)).color(color));
        }
        if link(ui, s(a, "name"), theme::regular(size), color).clicked() {
            let id = s(a, "id");
            if !id.is_empty() {
                app.act(Action::Navigate(Route::Artist(id.into())));
            }
        }
    }
    ui.spacing_mut().item_spacing.x = spacing;
}

/// A section heading in spaced capitals, with an optional "show all ›".
pub fn section_title(ui: &mut Ui, title: &str, show_all: bool) -> bool {
    let mut clicked = false;
    ui.add_space(sp(3.0));
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2((ui.available_width() - px(90.0)).max(10.0), px(22.0)), Sense::hover());
        caps(ui, pos2(rect.left(), rect.center().y), title, 12.5, col::sub());
        if show_all {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clicked = link(ui, "show all ›", theme::regular(12.5), col::muted()).clicked();
            });
        }
    });
    ui.add_space(sp(1.0));
    clicked
}

pub struct CardResult {
    pub open: bool,
    pub play: bool,
    pub response: Response,
}

/// Cover + title + subtitle tile with a hover play button.
pub fn card(ui: &mut Ui, url: Option<&str>, title: &str, subtitle: &str, round: bool, width: f32, playing: bool) -> CardResult {
    let title_font = theme::regular(13.5);
    let sub_font = theme::regular(12.0);
    let height = width + px(52.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    // Hover from the pointer's position: egui reports only the button being
    // clicked as hovered, so `resp.hovered()` made the play button vanish
    // (and drop the click) the moment it was pressed.
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect.expand(sp(0.75)), theme::radius(), col::hover());
    }
    let art = Rect::from_min_size(rect.min, Vec2::splat(width));
    paint_cover(ui, art, url, round);
    let d = px(38.0);
    let b = Rect::from_center_size(art.right_bottom() - vec2(d * 0.75, d * 0.75), Vec2::splat(d));
    let pr = ui.interact(b, resp.id.with("play"), Sense::click());
    if hovered || playing {
        ui.painter().circle(b.center(), d / 2.0, col::overlay(), Stroke::new(1.5, col::accent()));
        icons::paint(ui.painter(), Rect::from_center_size(b.center(), Vec2::splat(d * 0.42)), if playing { Icon::Pause } else { Icon::Play }, col::accent());
    }
    let play = pr.clicked();
    text_trunc(ui, pos2(rect.left(), art.bottom() + px(16.0)), title, title_font, if playing { col::accent() } else { col::text() }, width);
    text_trunc(ui, pos2(rect.left(), art.bottom() + px(36.0)), subtitle, sub_font, col::muted(), width);
    focus_ring(ui, &resp, rect, theme::radius());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    CardResult { open: resp.clicked() && !play, play, response: resp }
}

/// Lay out cards in a responsive grid (or a single row when `one_row`).
pub fn card_grid(ui: &mut Ui, count: usize, one_row: bool, mut f: impl FnMut(&mut Ui, usize, f32)) {
    if count == 0 {
        return;
    }
    let avail = ui.available_width();
    let gap = sp(2.5);
    let min = px(150.0);
    let cols = ((avail + gap) / (min + gap)).floor().max(2.0) as usize;
    let w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let n = if one_row { count.min(cols) } else { count };
    // Rows are all as tall as the first: rows scrolled out of sight only take their space.
    let mut row_h: Option<f32> = None;
    for start in (0..n).step_by(cols) {
        if let Some(h) = row_h
            && !ui.is_rect_visible(Rect::from_min_size(ui.cursor().min, vec2(avail, h)))
        {
            ui.allocate_space(vec2(avail, h));
            ui.add_space(sp(2.0));
            continue;
        }
        let row = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for i in start..(start + cols).min(n) {
                f(ui, i, w);
            }
        });
        row_h.get_or_insert(row.response.rect.height());
        ui.add_space(sp(2.0));
    }
}

/// The header at the top of playlist/album/artist/show pages.
pub fn entity_header(ui: &mut Ui, url: Option<&str>, round: bool, kind: &str, title: &str, mut meta: impl FnMut(&mut Ui)) -> Response {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("page-title"), title.to_string()));
    let mut cover_resp = None;
    let art = (ui.available_width() * 0.28).clamp(px(140.0), px(220.0));
    ui.horizontal_top(|ui| {
        cover_resp = Some(cover(ui, url, art, round));
        ui.add_space(sp(3.0));
        ui.vertical(|ui| {
            ui.add_space((art - px(130.0)).max(0.0));
            caps_label(ui, kind, 11.0, col::muted());
            let size = if title.chars().count() > 40 { 22.0 } else { 30.0 };
            ui.add(egui::Label::new(RichText::new(title).font(theme::regular(size)).color(col::text())).wrap());
            ui.add_space(sp(0.75));
            ui.horizontal_wrapped(|ui| meta(ui));
        });
    });
    cover_resp.unwrap()
}

pub fn dot(ui: &mut Ui) {
    ui.label(RichText::new("•").color(col::muted()));
}

pub fn subtle(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(col::sub()).font(theme::regular(12.5)));
}

pub fn loading(ui: &mut Ui) {
    ui.add_space(sp(5.0));
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("loading…").font(theme::regular(12.5)).color(col::muted()));
    });
}

pub fn error_box(ui: &mut Ui, err: &str) -> bool {
    let mut retry = false;
    ui.add_space(sp(3.0));
    theme::card().show(ui, |ui| {
        ui.label(RichText::new("couldn't load this").font(theme::regular(14.0)).color(col::text()));
        ui.label(RichText::new(err).color(col::muted()).font(theme::regular(12.0)));
        ui.add_space(sp(0.5));
        retry = bracket_button(ui, None, "try again", false).clicked();
    });
    retry
}

/// An empty state, or a note about something Glass can't do.
pub fn unavailable(ui: &mut Ui, title: &str, body: &str) {
    ui.add_space(sp(3.0));
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).font(theme::regular(15.0)).color(col::text()));
        ui.add_space(sp(0.5));
        ui.add(egui::Label::new(RichText::new(body).color(col::muted()).font(theme::regular(12.5))).wrap());
    });
}

pub fn menu_item(ui: &mut Ui, text: &str) -> bool {
    let clicked = ui.add(egui::Button::new(RichText::new(text).font(theme::regular(13.0))).frame(false).min_size(vec2(px(220.0), px(28.0)))).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// A menu line that can't be clicked, e.g. how something in progress is going.
pub fn menu_note(ui: &mut Ui, text: &str) {
    ui.add_enabled(false, egui::Button::new(RichText::new(text).font(theme::regular(13.0))).frame(false).min_size(vec2(px(220.0), px(28.0))));
}

pub fn hairline(ui: &mut Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([pos2(r.left(), r.top()), pos2(r.right(), r.top())], theme::hairline());
    ui.add_space(1.0);
}

/// The "now playing" bars. Still when the system asks for reduced motion
/// or the window isn't in front (no point animating what nobody watches).
pub fn equalizer_bars(ui: &Ui, rect: Rect, color: Color32) {
    let still = theme::reduce_motion() || !ui.input(|i| i.focused);
    let t = if still { 0.6 } else { ui.input(|i| i.time) as f32 };
    for i in 0..3 {
        let h = (0.35 + 0.65 * ((t * 6.0 + i as f32 * 1.7).sin() * 0.5 + 0.5)) * rect.height();
        let w = rect.width() / 5.0;
        let x = rect.left() + w * (i as f32 * 2.0);
        ui.painter().rect_filled(Rect::from_min_max(pos2(x, rect.bottom() - h), pos2(x + w, rect.bottom())), 1.0, color);
    }
    if !still {
        // ~8 fps is plenty for three small bars.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
    }
}

pub fn outline_button(ui: &mut Ui, text: &str) -> Response {
    let font = theme::regular(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), col::text());
    let size = vec2(galley.size().x + sp(3.5), px(30.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    let stroke = if resp.hovered() { Stroke::new(1.0, col::sub()) } else { Stroke::new(1.0, col::border()) };
    ui.painter().rect(rect, theme::small_radius(), if resp.hovered() { col::hover() } else { Color32::TRANSPARENT }, stroke, StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, col::text());
    focus_ring(ui, &resp, rect, theme::small_radius());
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A small key cap, as in the key hints: `space`, `⌘ K`.
pub fn keycap(ui: &Ui, left_center: egui::Pos2, key: &str) -> f32 {
    let key = keys(key);
    let key = key.as_ref();
    let font = theme::regular(11.0);
    let galley = ui.painter().layout_no_wrap(key.to_string(), font.clone(), col::sub());
    let r = Rect::from_min_size(pos2(left_center.x, left_center.y - px(10.0)), vec2(galley.size().x + sp(1.25), px(20.0)));
    ui.painter().rect(r, CornerRadius::same(3), col::raised(), Stroke::new(1.0, col::divider()), StrokeKind::Inside);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, key, font, col::sub());
    r.width()
}

/// Paint single-line text, vertically centered on `left_center`, cut off with "…" at `max_w`.
pub fn text_trunc(ui: &Ui, left_center: egui::Pos2, text: &str, font: FontId, color: Color32, max_w: f32) -> f32 {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
    let galley = ui.painter().layout_job(job);
    let w = galley.size().x;
    ui.painter().galley(left_center - vec2(0.0, galley.size().y / 2.0), galley, color);
    w
}

#[cfg(test)]
mod cover_fade_tests {
    use super::*;

    /// A frame at `time`, with the cover `url`; returns how far the newest cover has faded in.
    fn frame(ctx: &egui::Context, url: &str, time: f64) -> f32 {
        let mut k = 0.0;
        let input = egui::RawInput { time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            k = paint_cover_fading(ui, egui::Id::new("cover"), Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 100.0)), Some(url), false);
        });
        out.textures_delta.clear();
        k
    }

    #[test]
    fn a_new_cover_fades_in_over_the_old_one() {
        let ctx = egui::Context::default();
        assert_eq!(frame(&ctx, "bytes://a.png", 0.0), 1.0, "the first cover just shows");
        assert_eq!(frame(&ctx, "bytes://a.png", 0.5), 1.0);
        // The next song: its cover can't load here, so it fades in right away rather than waiting.
        let start = frame(&ctx, "bytes://b.png", 1.0);
        assert!(start < 0.05, "starts hidden: {start}");
        let mid = frame(&ctx, "bytes://b.png", 1.0 + COVER_FADE / 2.0);
        assert!((0.3..0.7).contains(&mid), "halfway: {mid}");
        assert_eq!(frame(&ctx, "bytes://b.png", 1.0 + COVER_FADE + 0.05), 1.0, "then settles");
        assert_eq!(frame(&ctx, "bytes://b.png", 3.0), 1.0, "and stays");
    }
}

#[cfg(test)]
mod keys_tests {
    #[test]
    fn shortcuts_are_spelled_out_off_macos() {
        assert_eq!(super::spelled_out("Search  ⌘ L"), "Search  Ctrl+L");
        assert_eq!(super::spelled_out("Devices  ⌥ ⇧ D"), "Devices  Alt+Shift+D");
        assert_eq!(super::spelled_out("⌘K"), "Ctrl+K");
        assert_eq!(super::spelled_out("⌃ ⌘ F"), "Ctrl+F");
        assert_eq!(super::spelled_out("Press ⌘ / to show or hide this."), "Press Ctrl+/ to show or hide this.");
    }
}
