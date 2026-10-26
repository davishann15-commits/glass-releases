//! Window layout, terminal-glass style:
//!
//! ```text
//! ┌ ● ● ●  ~/ music ─────────────────────────────────────────┐
//! │ rail │ page (library: list │ detail, ⌘K bar)   [panel]  │
//! │  ⌂   │                                                   │
//! │  ♫   │                                                   │
//! │  ⌕   │                                     [theme.toml]  │
//! │  ▭   │                                                   │
//! │  ⚙   │                                                   │
//! │ ╭──────────────────── player bar ─────────────────────╮ │
//! └─╰─────────────────────────────────────────────────────╯─┘
//! ```
//! The rail, title strip and player bar are fixed. The side panel (queue,
//! devices, lyrics or the assistant) takes its room from the page; Now
//! Playing is a page of its own; the theme panel floats over the page.

pub mod album_colors;
pub mod icons;
pub mod images;
pub mod theme;
pub mod widgets;

mod assistant;
mod command;
mod library;
pub mod local_library;
mod menus;
mod mini;
mod modals;
mod nowbar;
mod nowplaying;
mod pages;
mod right;
mod settings;
mod themepanel;
mod tracklist;
mod visualizer;

#[cfg(test)]
mod tests;

use crate::app::{Action, App, AuthState, FileTarget, LibFilter, Modal, NpView, RightPanel, Route};
use eframe::egui::{self, Align2, CornerRadius, Frame, Key, KeyboardShortcut, Margin, Modifiers, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use icons::Icon;
use theme::{col, px, sp};

pub fn root(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let t = app.theme.theme();
    if theme::sync(&ctx, t, app.theme.generation(), crate::window_fx::effective_opacity(t)) {
        // Fonts land next frame; drawing text now would use families that don't exist yet.
        ctx.request_discard("fonts installed");
        return;
    }

    let win = ui.max_rect();
    let frame_radius = CornerRadius::same((theme::radius_f() + 2.0) as u8);
    ui.painter().rect(win, frame_radius, col::window(), Stroke::new(1.0, col::divider()), StrokeKind::Inside);

    if app.auth_state != AuthState::SignedIn {
        mini::sign_in(app, ui);
        return;
    }
    shortcuts(app, &ctx);
    handle_close(app, &ctx);

    // The playing album's colors (Settings ▸ Display): `tint` is how much of them the
    // now-playing views get, `page` how much the whole window does (only on Now Playing).
    // The same size Now Playing shows, so the two share one download.
    let cover = app.now_cover(640).filter(|_| app.settings.album_colors);
    let palette = app.album_colors.frame(&ctx, cover.as_deref());
    let queue = app.queue.clone();
    if app.settings.album_colors
        && let Some(next) = queue.as_deref().and_then(|q| q["queue"].get(0))
        && let Some(uri) = app.cover_of(next, 640)
    {
        // The next song's colors are ready the moment it starts.
        app.album_colors.prefetch(&ctx, &uri);
    }
    let fade = if theme::reduce_motion() { 0.0 } else { 0.45 };
    let on = app.settings.album_colors && palette.is_some() && app.album_colors.has_colors();
    let tint = ctx.animate_value_with_time(egui::Id::new("album-colors"), if on { 1.0 } else { 0.0 }, fade);
    let page = ctx.animate_value_with_time(egui::Id::new("album-colors-page"), if on && app.route == Route::NowPlaying && !app.fullscreen { 1.0 } else { 0.0 }, fade);
    let colors = |amount: f32| palette.filter(|_| amount > 0.0).map(|p| theme::override_with((p.key(), amount.to_bits()), |t| p.apply(t, amount)));

    if app.fullscreen {
        let _colors = colors(tint);
        mini::fullscreen(app, ui);
        modals::show(app, &ctx);
        return;
    }

    if palette.is_some() && page > 0.0 {
        let inner = CornerRadius::same(frame_radius.nw.saturating_sub(1));
        app.album_colors.paint_wash(ui, win.shrink(1.0), inner, page);
    }
    let page_colors = colors(page);

    // The now-playing screen has its own controls, so the player bar gives way to key hints.
    let np = app.route == Route::NowPlaying;
    let strip_h = px(44.0);
    let rail_w = px(88.0);
    let bar_h = if np { px(54.0) } else { px(96.0) };
    let m = if np { 0.0 } else { sp(1.25) };
    let strip = Rect::from_min_max(win.min, pos2(win.right(), win.top() + strip_h));
    let bar = Rect::from_min_max(pos2(win.left() + m, win.bottom() - m - bar_h), pos2(win.right() - m, win.bottom() - m));
    let body_bottom = if np { bar.top() } else { bar.top() - sp(1.0) };
    let rail = Rect::from_min_max(pos2(win.left(), strip.bottom()), pos2(win.left() + rail_w, body_bottom));
    let mut content = Rect::from_min_max(pos2(rail.right(), strip.bottom()), pos2(win.right(), body_bottom));

    title_strip(app, ui, strip);
    ui.painter().line_segment([pos2(win.left(), strip.bottom()), pos2(win.right(), strip.bottom())], theme::hairline());
    nav_rail(app, ui, rail);
    ui.painter().line_segment([pos2(rail.right(), rail.top() + sp(1.0)), pos2(rail.right(), rail.bottom() - sp(1.0))], theme::hairline());

    // The side panel takes room from the page instead of covering it.
    if app.settings.right_open && !np {
        let w = if app.right == RightPanel::Assistant { px(400.0) } else { px(340.0) }.min(content.width() * 0.45);
        let r = Rect::from_min_max(pos2(content.right() - w - sp(2.0), content.top() + sp(2.0)), pos2(content.right() - sp(2.0), content.bottom()));
        content.max.x = r.left() - sp(1.0);
        ui.scope_builder(UiBuilder::new().max_rect(r).id_salt("side"), |ui| {
            theme::popup().fill(col::overlay().gamma_multiply(0.85)).inner_margin(Margin::ZERO).show(ui, |ui| {
                ui.set_min_size(r.size());
                ui.set_max_size(r.size());
                right::show(app, ui);
            });
        });
    }

    ui.scope_builder(UiBuilder::new().max_rect(content).id_salt("content"), |ui| {
        if app.route == Route::Library {
            library::show(app, ui, content);
        } else if np {
            nowplaying::show(app, ui, content);
            // ⌘K on Now Playing: the command bar over the top, only while it's in use.
            if app.cmd_open || app.focus_cmd {
                let cmd_w = (content.width() * 0.5).clamp(px(280.0), px(700.0));
                let cmd = Rect::from_min_size(pos2(content.center().x - cmd_w / 2.0, content.top() + sp(2.5)), vec2(cmd_w, px(44.0)));
                ui.painter().rect_filled(cmd, theme::small_radius(), col::window());
                command::bar(app, ui, cmd);
            }
        } else {
            let pad = sp(4.0);
            let cmd_w = (content.width() * 0.5).clamp(px(280.0), px(700.0));
            let cmd = Rect::from_min_size(pos2(content.right() - sp(3.0) - cmd_w, content.top() + sp(2.5)), vec2(cmd_w, px(44.0)));
            command::bar(app, ui, cmd);
            let page = Rect::from_min_max(pos2(content.left() + pad, cmd.bottom() + sp(1.5)), pos2(content.right() - sp(1.0), content.bottom()));
            ui.scope_builder(UiBuilder::new().max_rect(page).id_salt("page"), |ui| {
                // Nothing on the page paints under the side panel.
                ui.set_clip_rect(ui.clip_rect().intersect(page.expand2(vec2(0.0, sp(2.0)))));
                pages::show(app, ui)
            });
        }
    });

    if np {
        hint_strip(ui, bar);
    } else {
        let _bar_colors = colors(tint);
        ui.scope_builder(UiBuilder::new().max_rect(bar).id_salt("bar"), |ui| nowbar::show(app, ui));
    }

    // The theme panel floats over the page, never over the side panel.
    themepanel::show(app, &ctx, Rect::from_min_max(content.min, pos2(content.right(), body_bottom)));
    command::palette(app, &ctx);
    modals::show(app, &ctx);
    toasts(app, &ctx, bar);
    file_drop(app, &ctx, content);
    drag_ghost(&ctx);
    drop(page_colors);
    if app.mini {
        let _colors = colors(tint);
        mini::mini_player(app, &ctx);
    }
}

/// `‹ | ~ / music / now-playing` — where you are, as a path. Empty space
/// drags the window.
fn title_strip(app: &mut App, ui: &mut Ui, strip: Rect) {
    let bg = ui.interact(strip, ui.id().with("drag"), Sense::click_and_drag());
    if bg.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bg.double_clicked() {
        let maxed = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
    }
    // macOS draws its own window buttons in this corner.
    #[cfg(not(target_os = "macos"))]
    window_buttons(ui, strip);
    let path = match &app.route {
        Route::Library => "music",
        Route::NowPlaying => "music / now-playing",
        Route::Home => "home",
        Route::Search | Route::Genre { .. } => "search",
        Route::Settings => "settings",
        Route::Activity => "settings / activity",
        Route::Playlist(_) => "music / playlist",
        Route::Album(_) => "music / album",
        Route::Artist(_) | Route::Discography(_) => "music / artist",
        Route::Track(_) => "music / song",
        Route::Show(_) | Route::Episode(_) => "podcasts",
        Route::Audiobook(_) => "audiobooks",
        Route::Profile(_) => "profile",
        Route::LikedSongs => "music / liked",
        Route::YourEpisodes => "podcasts / saved",
        Route::LocalFiles => "music / local",
        Route::Recents => "music / recents",
        Route::Folder(_) => "music / folder",
        Route::LocalPlaylist(_) => "music / local / playlist",
        Route::LocalAlbum { .. } => "music / local / album",
        Route::Device { .. } => "music / local / device",
    };
    let mut x = strip.left() + px(84.0);
    if app.can_go_back() {
        let r = Rect::from_center_size(pos2(x + px(10.0), strip.center().y), vec2(px(26.0), px(26.0)));
        if ui.scope_builder(UiBuilder::new().max_rect(r), |ui| widgets::icon_btn(ui, Icon::Back, 13.0, "Back  ⌘ [", false).clicked()).inner {
            app.act(Action::Back);
        }
        ui.painter().line_segment([pos2(r.right() + sp(1.0), strip.center().y - px(9.0)), pos2(r.right() + sp(1.0), strip.center().y + px(9.0))], theme::hairline());
        x = r.right() + sp(2.25);
    }
    let path_rect = ui.painter().text(pos2(x, strip.center().y), Align2::LEFT_CENTER, format!("~ / {path}"), theme::regular(13.5), col::sub());
    // An update downloaded and ready: restart into it.
    if let crate::app::Update::Ready { version, .. } = &app.update {
        let text = format!("update to {version} · restart");
        let g = ui.painter().layout_no_wrap(text.clone(), theme::regular(12.0), col::accent());
        let r = Rect::from_min_size(pos2(path_rect.right() + sp(2.0), strip.center().y - px(13.0)), vec2(g.size().x + px(20.0), px(26.0)));
        let resp = ui
            .interact(r, ui.id().with("update-pill"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Installs the new version and reopens Glass (or it installs when you quit)");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &text));
        ui.painter().rect(r, theme::small_radius(), if resp.hovered() { col::accent_a(0.18) } else { col::accent_a(0.10) }, Stroke::new(1.0, col::accent()), StrokeKind::Inside);
        ui.painter().galley(pos2(r.left() + px(10.0), r.center().y - g.size().y / 2.0), g, col::accent());
        if resp.clicked() {
            app.act(Action::RestartToUpdate);
        }
    }

    if app.route == Route::NowPlaying {
        // Lyrics / queue toggles for the right-hand column.
        let font = theme::regular(13.0);
        let mut right = strip.right() - sp(3.0);
        for (view, label, icon) in [(NpView::Playlist, "playlist", Icon::Music), (NpView::Queue, "queue", Icon::Queue), (NpView::Lyrics, "lyrics", Icon::Lyrics)] {
            let g = ui.painter().layout_no_wrap(label.to_string(), font.clone(), col::text());
            let w = g.size().x + px(44.0);
            let r = Rect::from_min_max(pos2(right - w, strip.center().y - px(16.0)), pos2(right, strip.center().y + px(16.0)));
            let tip = match view {
                NpView::Lyrics => "Lyrics  L",
                NpView::Queue => "Queue  Q",
                _ => "The list that's playing  P",
            };
            let resp = ui.interact(r, ui.id().with(("np-view", view)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, app.np_view == view, label));
            let on = app.np_view == view;
            if on {
                ui.painter().rect(r, theme::small_radius(), col::accent_a(0.10), Stroke::new(1.2, col::accent()), StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, theme::small_radius(), col::hover());
            }
            let color = if on { col::accent() } else { col::sub() };
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + px(19.0), r.center().y), vec2(px(15.0), px(15.0))), icon, color);
            ui.painter().text(pos2(r.left() + px(34.0), r.center().y), Align2::LEFT_CENTER, label, font.clone(), color);
            widgets::focus_ring(ui, &resp, r, theme::small_radius());
            if resp.clicked() {
                app.np_view = view;
            }
            right = r.left() - sp(1.5);
        }
        // The visualizer toggle: a small level meter.
        ui.painter().line_segment([pos2(right + sp(0.5), strip.center().y - px(10.0)), pos2(right + sp(0.5), strip.center().y + px(10.0))], theme::hairline());
        right -= sp(1.0);
        let on = app.settings.visualizer;
        let r = Rect::from_min_max(pos2(right - px(36.0), strip.center().y - px(16.0)), pos2(right, strip.center().y + px(16.0)));
        let resp =
            ui.interact(r, ui.id().with("np-viz"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if on { "Hide the visualizer  V" } else { "Show the visualizer  V" });
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, "visualizer"));
        if on {
            ui.painter().rect(r, theme::small_radius(), col::accent_a(0.10), Stroke::new(1.2, col::accent()), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, theme::small_radius(), col::hover());
        }
        let color = if on { col::accent() } else { col::sub() };
        for (i, h) in [0.45f32, 0.9, 0.65, 1.0, 0.5].iter().enumerate() {
            let bx = r.center().x - px(6.0) + i as f32 * px(3.0);
            let bh = px(13.0) * h;
            ui.painter().line_segment([pos2(bx, r.center().y - bh / 2.0), pos2(bx, r.center().y + bh / 2.0)], Stroke::new(1.3, color));
        }
        widgets::focus_ring(ui, &resp, r, theme::small_radius());
        if resp.clicked() {
            app.settings.visualizer = !on;
            app.mark_settings_dirty();
        }
    }
}

/// Close, minimize and zoom, where and how the Mac's window buttons sit in
/// the title strip: grey while the window isn't focused, symbols while the
/// pointer is over them.
#[cfg(not(target_os = "macos"))]
fn window_buttons(ui: &mut Ui, strip: Rect) {
    use egui::Color32;
    // AppKit's metrics, in points: they don't follow the theme's font size.
    const D: f32 = 12.0;
    const COLORS: [Color32; 3] = [Color32::from_rgb(0xFF, 0x5F, 0x57), Color32::from_rgb(0xFE, 0xBC, 0x2E), Color32::from_rgb(0x28, 0xC8, 0x40)];
    const TIPS: [&str; 3] = ["Close", "Minimize", "Zoom"];
    let centers: [egui::Pos2; 3] = std::array::from_fn(|i| pos2(strip.left() + 14.0 + 20.0 * i as f32, strip.top() + 14.0));
    let group = Rect::from_min_max(centers[0], centers[2]).expand(D / 2.0 + 4.0);
    let show_symbols = ui.rect_contains_pointer(group);
    let focused = ui.ctx().input(|i| i.viewport().focused.unwrap_or(true));
    let symbol = Stroke::new(1.1, Color32::from_black_alpha(150));
    for (i, c) in centers.into_iter().enumerate() {
        let resp = ui.interact(Rect::from_center_size(c, vec2(D + 6.0, D + 6.0)), ui.id().with(("window-button", i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, TIPS[i]));
        let fill = if focused || show_symbols { COLORS[i] } else { Color32::from_white_alpha(46) };
        let fill = if resp.is_pointer_button_down_on() { fill.lerp_to_gamma(Color32::BLACK, 0.2) } else { fill };
        ui.painter().circle(c, D / 2.0, fill, Stroke::new(0.5, Color32::from_black_alpha(if focused { 50 } else { 20 })));
        if show_symbols {
            let a = 3.0;
            match i {
                0 => {
                    ui.painter().line_segment([c + vec2(-a, -a), c + vec2(a, a)], symbol);
                    ui.painter().line_segment([c + vec2(-a, a), c + vec2(a, -a)], symbol);
                }
                1 => {
                    ui.painter().line_segment([c + vec2(-a - 0.5, 0.0), c + vec2(a + 0.5, 0.0)], symbol);
                }
                _ => {
                    // Two corners pointing apart, like the Mac's full-screen arrows.
                    let fill = symbol.color;
                    ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-3.5, -3.5), c + vec2(1.5, -3.5), c + vec2(-3.5, 1.5)], fill, Stroke::NONE));
                    ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(3.5, 3.5), c + vec2(-1.5, 3.5), c + vec2(3.5, -1.5)], fill, Stroke::NONE));
                }
            }
        }
        if resp.clicked() {
            let ctx = ui.ctx();
            match i {
                0 => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                1 => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                _ => {
                    let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
                }
            }
        }
    }
}

/// The now-playing screen's footer: `space play / pause` … `L lyrics • Q queue • P playlist • V visualizer`.
fn hint_strip(ui: &mut Ui, bar: Rect) {
    ui.painter().line_segment([bar.left_top(), bar.right_top()], theme::hairline());
    let y = bar.center().y;
    let font = theme::regular(12.0);
    let x = bar.left() + sp(3.0);
    let x = x + widgets::keycap(ui, pos2(x, y), "space") + sp(1.5);
    ui.painter().text(pos2(x, y), Align2::LEFT_CENTER, "play / pause", font.clone(), col::muted());
    let mut rx = bar.right() - sp(3.0);
    for (text, key) in [("visualizer", false), ("V", true), ("•", false), ("playlist", false), ("P", true), ("•", false), ("queue", false), ("Q", true), ("•", false), ("lyrics", false), ("L", true)]
    {
        if key {
            rx -= ui.painter().layout_no_wrap(text.to_string(), theme::regular(11.0), col::sub()).size().x + sp(1.25);
            widgets::keycap(ui, pos2(rx, y), text);
        } else {
            rx -= ui.painter().layout_no_wrap(text.to_string(), font.clone(), col::muted()).size().x;
            ui.painter().text(pos2(rx, y), Align2::LEFT_CENTER, text, font.clone(), col::muted());
        }
        rx -= sp(1.25);
    }
}

fn nav_rail(app: &mut App, ui: &mut Ui, rail: Rect) {
    let items = [
        (Icon::Home, "Home", Route::Home),
        (Icon::Music, "Library", Route::Library),
        (Icon::Search, "Search  ⌘ L", Route::Search),
        (Icon::Folder, "Local Files", Route::LocalFiles),
        (Icon::Settings, "Settings  ⌘ ,", Route::Settings),
    ];
    let current = match &app.route {
        Route::Home => 0,
        Route::Search | Route::Genre { .. } => 2,
        Route::LocalFiles | Route::LocalPlaylist(_) | Route::LocalAlbum { .. } | Route::Device { .. } => 3,
        Route::Settings | Route::Activity => 4,
        _ => 1,
    };
    let size = px(46.0);
    let mut y = rail.top() + sp(3.0);
    for (i, (icon, tip, route)) in items.into_iter().enumerate() {
        let r = Rect::from_center_size(pos2(rail.center().x, y + size / 2.0), vec2(size, size));
        let resp = ui.interact(r, ui.id().with(("rail", i)), Sense::click()).on_hover_text(widgets::keys(tip)).on_hover_cursor(egui::CursorIcon::PointingHand);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, i == current, tip));
        let on = i == current;
        if on {
            ui.painter().rect(r, theme::small_radius(), col::accent_a(0.10), Stroke::new(1.2, col::accent()), StrokeKind::Inside);
            let marker = Rect::from_min_size(pos2(rail.left(), r.top() + size * 0.15), vec2(3.0, size * 0.7));
            ui.painter().rect_filled(marker, CornerRadius::same(2), col::accent());
        } else if resp.hovered() {
            ui.painter().rect_filled(r, theme::small_radius(), col::hover());
        }
        let color = if on {
            col::accent()
        } else if resp.hovered() {
            col::text()
        } else {
            col::sub()
        };
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(px(22.0), px(22.0))), icon, color);
        widgets::focus_ring(ui, &resp, r, theme::small_radius());
        if resp.clicked() {
            if route == Route::Search {
                app.focus_search = true;
            }
            app.act(Action::Navigate(route));
        }
        y += size + sp(2.25);
    }
}

/// Songs being dragged inside Glass (onto a local playlist).
#[derive(Clone)]
pub struct SongDrag {
    pub files: Vec<std::path::PathBuf>,
    pub label: String,
}

/// Where files dropped now would go: the playlist on screen when you can
/// add to it, otherwise Local Files.
pub fn file_target(app: &mut App) -> FileTarget {
    match app.route.clone() {
        Route::Playlist(id) => match menus::writable_playlists(app).into_iter().find(|p| crate::model::s(p, "id") == id) {
            Some(p) => FileTarget::Spotify { id, name: crate::model::s(&p, "name").to_string() },
            None => FileTarget::LocalFiles,
        },
        Route::LocalPlaylist(id) => match app.playlists.get(&id) {
            Some(p) => FileTarget::Local { name: p.name.clone(), id },
            None => FileTarget::LocalFiles,
        },
        _ => FileTarget::LocalFiles,
    }
}

/// Audio files and folders dragged in from Finder (or any file manager).
fn file_drop(app: &mut App, ctx: &egui::Context, area: Rect) {
    let (hovering, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>()));
    if !dropped.is_empty() {
        let target = file_target(app);
        app.act(Action::AddFiles { paths: dropped, target });
        return;
    }
    if !hovering {
        return;
    }
    let text = match file_target(app) {
        FileTarget::Spotify { name, .. } | FileTarget::Local { name, .. } => format!("Drop to add to “{name}”"),
        FileTarget::LocalFiles => "Drop to add to Local Files".to_string(),
    };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("file-drop")));
    let r = area.shrink(sp(1.5));
    painter.rect(r, theme::small_radius(), col::window().gamma_multiply(0.9), Stroke::new(2.0, col::accent()), StrokeKind::Inside);
    painter.text(r.center(), Align2::CENTER_CENTER, text, theme::regular(20.0), col::text());
    painter.text(r.center() + vec2(0.0, px(28.0)), Align2::CENTER_CENTER, "songs play from their folder, which joins Local Files", theme::regular(12.5), col::sub());
}

/// A small tag under the pointer while songs are dragged.
fn drag_ghost(ctx: &egui::Context) {
    let Some(d) = egui::DragAndDrop::payload::<SongDrag>(ctx) else { return };
    let Some(p) = ctx.pointer_latest_pos() else { return };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("song-drag")));
    let galley = painter.layout_no_wrap(d.label.clone(), theme::regular(13.0), col::text());
    let r = Rect::from_min_size(p + vec2(14.0, 10.0), galley.size() + vec2(20.0, 10.0));
    painter.rect(r, theme::small_radius(), col::overlay(), Stroke::new(1.0, col::accent_a(0.7)), StrokeKind::Inside);
    painter.galley(r.min + vec2(10.0, 5.0), galley, col::text());
}

fn toasts(app: &App, ctx: &egui::Context, bar: Rect) {
    let Some((text, _)) = app.toasts.last() else { return };
    egui::Area::new("toast".into()).fixed_pos(pos2(bar.center().x, bar.top() - sp(2.0))).pivot(Align2::CENTER_BOTTOM).interactable(false).order(egui::Order::Tooltip).show(ctx, |ui| {
        Frame::new().fill(col::overlay()).stroke(Stroke::new(1.0, col::accent_a(0.7))).corner_radius(theme::small_radius()).inner_margin(Margin::symmetric(sp(2.0) as i8, sp(1.0) as i8)).show(
            ui,
            |ui| {
                ui.label(RichText::new(format!("> {text}")).color(col::text()).font(theme::regular(13.0)));
            },
        );
    });
}

fn handle_close(app: &mut App, ctx: &egui::Context) {
    if app.settings.close_minimizes && !app.quitting && ctx.input(|i| i.viewport().close_requested()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
}

/// Full screen: ⌃⌘F on macOS; F11 elsewhere, where ⌘ and ⌃ are both Ctrl and Ctrl+F is the filter.
pub const FULL_SCREEN_KEY: &str = if cfg!(target_os = "macos") { "⌃ ⌘ F" } else { "F11" };

/// The ⌘ / sheet: each section and its (what, keys).
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Basic",
        &[
            ("Command bar: search, go to, run", "⌘ K"),
            ("Move down / up in a list", "j / k"),
            ("Open or play the selection", "Enter"),
            ("Create new playlist", "⌘ N"),
            ("Create new folder", "⇧ ⌘ N"),
            ("Open context menu", "Right-click"),
            ("Filter in page", "⌘ F"),
            ("Select all", "⌘ A"),
            ("Theme panel", "⌘ T"),
            ("Log out", "⇧ ⌘ W"),
            ("Keyboard shortcuts", "⌘ /"),
        ],
    ),
    (
        "Playback",
        &[
            ("Play / Pause", "Space"),
            ("Like", "⌥ ⇧ B"),
            ("Shuffle", "⌘ S"),
            ("Repeat", "⌘ R"),
            ("Skip to previous", "⌘ ←"),
            ("Skip to next", "⌘ →"),
            ("Seek backward", "⇧ ⌘ ←"),
            ("Seek forward", "⇧ ⌘ →"),
            ("Raise volume", "⌘ ↑"),
            ("Lower volume", "⌘ ↓"),
        ],
    ),
    (
        "Navigation",
        &[
            ("Library", "⌥ ⇧ 0"),
            ("Home", "⌥ ⇧ H"),
            ("Back in history", "⌘ ["),
            ("Forward in history", "⌘ ]"),
            ("Now playing", "⌥ ⇧ J"),
            ("Now playing: lyrics / queue / playlist / devices", "L / Q / P / D"),
            ("Visualizer", "V"),
            ("Lyrics timing for this song", "[ / ]"),
            ("Lyrics: original / + translation / translated", "T"),
            ("Search", "⌘ L"),
            ("Liked songs", "⌥ ⇧ S"),
            ("Your playlists", "⌥ ⇧ 1"),
            ("Your podcasts", "⌥ ⇧ 2"),
            ("Your artists", "⌥ ⇧ 3"),
            ("Your albums", "⌥ ⇧ 4"),
            ("Your audiobooks", "⌥ ⇧ 5"),
            ("Recents", "⌥ ⇧ R"),
            ("Settings", "⌘ ,"),
            ("Queue", "⌥ ⇧ Q"),
            ("Devices", "⌥ ⇧ D"),
        ],
    ),
    ("Layout", &[("Toggle side panel", "⌥ ⇧ ]"), ("Miniplayer", "⌥ ⇧ M"), ("Full screen", FULL_SCREEN_KEY), ("Close panel / dialog / command bar", "Esc")]),
    ("Assistant", &[("Open or close the assistant", "⌘ J"), ("Ask from the command bar", "⌘ K, then ?"), ("Send / new line", "Enter / ⇧ Enter"), ("Stop a reply", "Esc")]),
];

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    let typing = ctx.egui_wants_keyboard_input();
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
    let ctrl_cmd = Modifiers::COMMAND | Modifiers::CTRL;
    let take = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
    // A key without a chord. egui would also match it with ⌥⇧ held, and those belong to
    // the ⌥⇧ chords below. ⌥ or ⇧ alone still count: some layouts type [ ] / with them.
    let bare = |k: Key| ctx.input(|i| !(i.modifiers.alt && i.modifiers.shift)) && take(Modifiers::NONE, k);

    // An open menu closes itself on Esc, and a text field stops editing: don't take the key
    // from them (unless the command bar or a dialog is up, which Esc closes).
    let field_takes_esc = typing && !(app.cmd_open || app.focus_cmd || app.modal.is_some()) && !(app.ai.busy() && app.settings.right_open && app.right == RightPanel::Assistant);
    if !egui::Popup::is_any_open(ctx) && !field_takes_esc && take(Modifiers::NONE, Key::Escape) {
        if app.ai.busy() && app.settings.right_open && app.right == RightPanel::Assistant {
            // Esc first stops a running assistant request.
            app.act(Action::AiCancel);
        } else if app.cmd_open || app.focus_cmd {
            app.cmd_open = false;
            app.cmd_text.clear();
            ctx.memory_mut(|m| m.surrender_focus(command::field_id()));
        } else if app.modal.is_some() {
            app.modal = None;
        } else if app.theme_panel {
            app.theme_panel = false;
        } else if app.fullscreen {
            app.act(Action::ToggleFullscreen);
        } else if app.route == Route::NowPlaying {
            app.act(Action::Back);
        } else if app.settings.right_open {
            app.act(Action::ToggleRight);
        }
    }
    if !typing && take(Modifiers::NONE, Key::Space) {
        app.act(Action::TogglePlay);
    }
    // L / Q / P: the now-playing screen with the lyrics, the queue or the
    // playlist that's playing; again to leave.
    for (key, view) in [(Key::L, NpView::Lyrics), (Key::Q, NpView::Queue), (Key::P, NpView::Playlist)] {
        if !typing && bare(key) {
            if app.route == Route::NowPlaying && app.np_view == view {
                app.act(Action::Back);
            } else {
                app.np_view = view;
                if app.route != Route::NowPlaying {
                    app.act(Action::Navigate(Route::NowPlaying));
                }
            }
        }
    }
    // V: the visualizer under the now-playing view.
    if !typing && bare(Key::V) {
        app.settings.visualizer = !app.settings.visualizer;
        app.mark_settings_dirty();
        if app.settings.visualizer && app.route != Route::NowPlaying {
            app.act(Action::Navigate(Route::NowPlaying));
        }
    }
    // [ / ]: this song's lyrics 0.1 s later / earlier while they're on screen.
    if !typing && app.route == Route::NowPlaying && app.np_view == NpView::Lyrics {
        // T: original · original + translation · translated.
        if bare(Key::T) {
            let m = app.config.lyrics().translation.next();
            app.act(Action::LyricsTranslation(m));
            app.toast(format!("Lyrics: {}", m.label()));
        }
        // This song's timing; remembered for it.
        let offset = app.lyrics_song_offset();
        if bare(Key::OpenBracket) {
            app.act(Action::LyricsSongOffset(offset - 100));
        }
        if bare(Key::CloseBracket) {
            app.act(Action::LyricsSongOffset(offset + 100));
        }
    }
    // Seek before skip: the more specific chord must be consumed first.
    // Chords that are also text-editing keys (⇧⌘←, ⌘↑, ⌥⇧ letters…) only
    // count when no text field has focus.
    let take = |m: Modifiers, k: Key| !typing && take(m, k);
    if take(cmd_shift, Key::ArrowRight) {
        app.act(Action::SeekBy(5_000));
    }
    if take(cmd_shift, Key::ArrowLeft) {
        app.act(Action::SeekBy(-5_000));
    }
    if take(cmd_shift, Key::N) {
        app.modal = Some(Modal::NewFolder { name: "New Folder".into(), add: None });
    }
    if take(cmd_shift, Key::W) {
        app.modal =
            Some(Modal::Confirm { title: "Log out?".into(), body: "Glass will forget your Spotify session on this computer.".into(), confirm: "Log out".into(), action: Box::new(Action::LogOut) });
    }
    if !typing && take(cmd, Key::ArrowRight) {
        app.act(Action::Next);
    }
    if !typing && take(cmd, Key::ArrowLeft) {
        app.act(Action::Prev);
    }
    if take(cmd, Key::ArrowUp) {
        let v = app.pb.volume;
        app.act(Action::Volume((v + 0.1).min(1.0)));
        app.act(Action::VolumeCommit);
    }
    if take(cmd, Key::ArrowDown) {
        let v = app.pb.volume;
        app.act(Action::Volume((v - 0.1).max(0.0)));
        app.act(Action::VolumeCommit);
    }
    if take(cmd, Key::S) {
        let s = !app.pb.shuffle;
        app.act(Action::SetShuffle(s));
    }
    if take(cmd, Key::R) {
        app.act(Action::CycleRepeat);
    }
    // These work from anywhere, text fields included.
    let any = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
    // ⌘J: the assistant panel.
    if any(cmd, Key::J) {
        if app.settings.right_open && app.right == RightPanel::Assistant && app.route != Route::NowPlaying {
            app.act(Action::ToggleRight);
        } else {
            if app.route == Route::NowPlaying {
                app.act(Action::Back);
            }
            app.act(Action::SetRight(RightPanel::Assistant));
            app.ai.focus_input = true;
        }
    }
    if any(cmd, Key::K) {
        app.focus_cmd = true;
        app.cmd_open = true;
    }
    if any(cmd, Key::L) {
        app.act(Action::Navigate(Route::Search));
        app.focus_search = true;
    }
    if any(cmd, Key::T) {
        app.theme_panel = !app.theme_panel;
    }
    // ⌃⌘F before ⌘F, which would also match it (see FULL_SCREEN_KEY).
    let full_screen = if cfg!(target_os = "macos") { any(ctrl_cmd, Key::F) } else { take(Modifiers::NONE, Key::F11) };
    if full_screen {
        app.act(Action::ToggleFullscreen);
    }
    // ⌘F or /: the page's filter field (the command bar on the library).
    if any(cmd, Key::F) || (!typing && bare(Key::Slash)) {
        if app.route == Route::Library {
            app.focus_cmd = true;
            app.cmd_open = true;
        } else {
            app.focus_search = true;
        }
    }
    if take(cmd, Key::N) {
        app.act(Action::CreatePlaylist { name: "New Playlist".into(), uris: vec![] });
    }
    if any(cmd, Key::OpenBracket) {
        app.act(Action::Back);
    }
    if any(cmd, Key::CloseBracket) {
        app.act(Action::Forward);
    }
    if any(cmd, Key::Comma) {
        app.act(Action::Navigate(Route::Settings));
    }
    if any(cmd, Key::Slash) {
        app.modal = Some(Modal::Shortcuts);
    }
    if take(alt_shift, Key::B) {
        let uri = app.pb.uri().to_string();
        if !uri.is_empty() {
            let saved = app.is_saved(&uri).unwrap_or(false);
            app.act(Action::Save { uri, save: !saved });
        }
    }
    if take(alt_shift, Key::Num0) {
        app.act(Action::Navigate(Route::Library));
    }
    if take(alt_shift, Key::H) {
        app.act(Action::Navigate(Route::Home));
    }
    if take(alt_shift, Key::J) {
        app.act(Action::Navigate(Route::NowPlaying));
    }
    if take(alt_shift, Key::S) {
        app.act(Action::Navigate(Route::LikedSongs));
    }
    if take(alt_shift, Key::R) {
        app.act(Action::Navigate(Route::Recents));
    }
    // On the now-playing screen the queue and devices have their own views.
    if take(alt_shift, Key::Q) {
        if app.route == Route::NowPlaying {
            app.np_view = NpView::Queue;
        } else {
            app.act(Action::SetRight(RightPanel::Queue));
        }
    }
    if take(alt_shift, Key::D) || (app.route == Route::NowPlaying && !typing && bare(Key::D)) {
        if app.route == Route::NowPlaying {
            app.np_view = if app.np_view == NpView::Devices { NpView::Lyrics } else { NpView::Devices };
        } else {
            app.act(Action::SetRight(RightPanel::Devices));
        }
    }
    if take(alt_shift, Key::M) {
        app.act(Action::ToggleMini);
    }
    if take(alt_shift, Key::CloseBracket) {
        app.act(Action::ToggleRight);
    }
    for (k, tab, f) in
        [(Key::Num1, 0, LibFilter::Playlists), (Key::Num2, 1, LibFilter::Podcasts), (Key::Num3, 0, LibFilter::Artists), (Key::Num4, 0, LibFilter::Albums), (Key::Num5, 2, LibFilter::Audiobooks)]
    {
        if take(alt_shift, k) {
            app.lib_tab = tab;
            app.lib_filter = f;
            app.act(Action::Navigate(Route::Library));
        }
    }
    if take(cmd, Key::A) {
        app.select_all_pending = true;
    }
}
