//! The floating `theme.toml` panel. It edits the real file through
//! `ThemeStore` (comments and other keys are kept) and the window updates
//! as soon as a value is accepted. Edits made in a text editor show up here
//! too, since the store reloads the file when it changes.

use super::icons::Icon;
use super::theme::{self, col, px, sp};
use super::widgets;
use crate::app::App;
use crate::palette::AlbumParts;
use crate::theme_config::Rgba;
use eframe::egui::{self, Align2, Color32, Id, Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

const FIELDS: [(&str, &str); 6] =
    [("background", "theme.background"), ("foreground", "theme.foreground"), ("accent", "theme.accent"), ("border", "theme.border"), ("opacity", "theme.opacity"), ("blur", "theme.blur")];

fn current(app: &App, key: &str) -> String {
    let t = app.theme.theme();
    let (l, v) = (&t.lyrics, &t.viz);
    match key {
        "lyrics.current" => l.current.hex(),
        "lyrics.other" => l.other.hex(),
        "lyrics.highlight" => l.highlight.hex(),
        "lyrics.size" => format!("{:.2}", l.size),
        "lyrics.spacing" => format!("{:.2}", l.spacing),
        "lyrics.fade" => format!("{:.2}", l.fade),
        "lyrics.speed" => format!("{:.2}", l.speed),
        "lyrics.align" => l.align.clone(),
        "lyrics.position" => l.position.clone(),
        "lyrics.animation" => l.animation.clone(),
        "lyrics.bold" => l.bold.to_string(),
        "lyrics.word_lead" => l.word_lead.to_string(),
        "lyrics.word_glow" => l.word_glow.to_string(),
        "font.lyrics" => t.lyrics_family.clone(),
        "visualizer.style" => v.style.clone(),
        "visualizer.color" => v.color.hex(),
        "visualizer.color2" => v.color2.hex(),
        "visualizer.opacity" => format!("{:.2}", v.opacity),
        "visualizer.bars" => v.bars.to_string(),
        "visualizer.height" => format!("{:.2}", v.height),
        "visualizer.sensitivity" => format!("{:.2}", v.sensitivity),
        "visualizer.smoothing" => format!("{:.2}", v.smoothing),
        "visualizer.glow" => v.glow.to_string(),
        "theme.background" => t.background.hex(),
        "theme.foreground" => t.foreground.hex(),
        "theme.accent" => t.accent.hex(),
        "theme.border" => t.border.hex(),
        "theme.opacity" => format!("{:.2}", t.opacity),
        "theme.blur" => t.blur.to_string(),
        _ => String::new(),
    }
}

fn apply(app: &mut App, key: &str, raw: &str) {
    match app.theme.set(key, raw) {
        Ok(()) => {
            app.theme_fields.remove(key);
            app.theme_msg = None;
        }
        Err(e) => app.theme_msg = Some(e),
    }
}

pub fn show(app: &mut App, ctx: &egui::Context, region: Rect) {
    if !app.theme_panel {
        return;
    }
    let w = px(350.0).min(region.width() - sp(4.0));
    egui::Area::new(Id::new("theme-panel")).fixed_pos(region.right_bottom() - vec2(sp(1.5), sp(1.0))).pivot(Align2::RIGHT_BOTTOM).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::new().fill(col::overlay()).stroke(Stroke::new(1.0, col::border())).corner_radius(theme::radius()).inner_margin(egui::Margin::ZERO).show(ui, |ui| {
            ui.set_width(w);
            body(app, ui, w);
        });
    });
}

fn body(app: &mut App, ui: &mut Ui, w: f32) {
    let pad = sp(2.5);
    // Title.
    let (tr, _) = ui.allocate_exact_size(vec2(w, px(50.0)), Sense::hover());
    ui.painter().text(pos2(tr.left() + pad, tr.center().y), Align2::LEFT_CENTER, "theme.toml", theme::regular(15.5), col::text());
    let close = Rect::from_center_size(pos2(tr.right() - pad - px(6.0), tr.center().y), vec2(px(26.0), px(26.0)));
    ui.scope_builder(UiBuilder::new().max_rect(close), |ui| {
        if widgets::icon_btn(ui, Icon::Close, 12.0, "Close  esc", false).clicked() {
            app.theme_panel = false;
        }
    });
    ui.painter().line_segment([pos2(tr.left(), tr.bottom()), pos2(tr.right(), tr.bottom())], theme::hairline());

    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(pad as i8, sp(1.75) as i8)).show(ui, |ui| {
        ui.set_width(w - pad * 2.0);
        // Where the file is, and the live-reload status.
        ui.horizontal(|ui| {
            let dir = app.theme.path().parent().map(widgets::home_relative).unwrap_or_default();
            ui.label(RichText::new(format!("{dir}/")).font(theme::regular(12.5)).color(col::sub())).on_hover_text(app.theme.path().display().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, ok) = if app.theme.error().is_some() { ("file has errors", false) } else { ("live preview", true) };
                let font = theme::regular(11.5);
                let g = ui.painter().layout_no_wrap(label.to_string(), font.clone(), col::accent());
                let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + px(30.0), px(26.0)), Sense::click());
                let c = if ok { col::accent() } else { col::text() };
                ui.painter().rect(r, theme::small_radius(), col::accent_a(0.08), Stroke::new(1.0, if ok { col::accent_a(0.8) } else { col::border() }), StrokeKind::Inside);
                ui.painter().circle_filled(pos2(r.left() + px(11.0), r.center().y), 2.5, c);
                ui.painter().text(pos2(r.left() + px(20.0), r.center().y), Align2::LEFT_CENTER, label, font, c);
                resp.on_hover_text("Glass watches theme.toml and applies changes when you save it — from here or from any editor.");
            });
        });
        ui.add_space(sp(1.25));

        // Tabs: the [theme] values, [lyrics], [visualizer], and album colors.
        ui.horizontal(|ui| {
            for (i, name) in ["theme", "lyrics", "visualizer", "album"].iter().enumerate() {
                if ui.selectable_label(app.theme_tab == i as u8, RichText::new(*name).font(theme::regular(12.5))).clicked() {
                    app.theme_tab = i as u8;
                    app.theme_fields.clear();
                    app.theme_msg = None;
                }
            }
        });
        ui.add_space(sp(0.75));
        egui::Frame::new().stroke(Stroke::new(1.0, col::divider())).corner_radius(theme::small_radius()).inner_margin(egui::Margin::same(sp(1.25) as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match app.theme_tab {
                1 => {
                    egui::ScrollArea::vertical().id_salt("theme-lyrics").max_height(px(400.0)).show(ui, |ui| lyrics_tab(app, ui));
                    reset_row(app, ui, "lyrics");
                }
                2 => {
                    egui::ScrollArea::vertical().id_salt("theme-viz").max_height(px(400.0)).show(ui, |ui| visualizer_tab(app, ui));
                    reset_row(app, ui, "visualizer");
                }
                3 => album_tab(app, ui),
                _ => {
                    ui.label(RichText::new("[theme]").font(theme::regular(12.5)).color(col::sub()));
                    ui.add_space(sp(0.5));
                    for (label, key) in FIELDS {
                        field_row(app, ui, label, key);
                    }
                }
            }
        });
        if let Some(e) = app.theme_msg.clone().or_else(|| app.theme.error().map(str::to_string)) {
            ui.add_space(sp(0.75));
            ui.add(egui::Label::new(RichText::new(e).font(theme::regular(11.5)).color(col::accent())).wrap());
        }
        ui.add_space(sp(1.5));

        // Palette shortcuts for the accent, and reset.
        if app.theme_tab != 0 {
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("palette").font(theme::regular(12.5)).color(col::sub()));
            ui.add_space(sp(1.0));
            let palette = app.theme.theme().palette.clone();
            for c in palette {
                let (r, resp) = ui.allocate_exact_size(vec2(px(20.0), px(20.0)), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("accent {}", c.hex())));
                ui.painter().rect(r, egui::CornerRadius::same(3), c.color32(), Stroke::new(1.0, if resp.hovered() { col::text() } else { col::divider() }), StrokeKind::Inside);
                widgets::focus_ring(ui, &resp, r, egui::CornerRadius::same(3));
                if resp.on_hover_text(format!("Use {} as the accent", c.hex())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    apply(app, "theme.accent", &c.hex());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::outline_button(ui, "reset").on_hover_text("Restore the six [theme] values to the defaults").clicked() {
                    match app.theme.reset_panel_values() {
                        Ok(()) => {
                            app.theme_fields.clear();
                            app.theme_msg = None;
                        }
                        Err(e) => app.theme_msg = Some(e),
                    }
                }
            });
        });
        ui.add_space(sp(1.0));
        ui.horizontal(|ui| {
            if widgets::link(ui, "edit theme.toml ›", theme::regular(11.5), col::muted()).on_hover_text("Fonts, sizes, spacing and every color are in the file").clicked() {
                let _ = app.theme.ensure_file();
                let _ = open::that(app.theme.path());
            }
        });
    });
}

fn field_row(app: &mut App, ui: &mut Ui, label: &str, key: &str) {
    let is_color = !matches!(key, "theme.opacity" | "theme.blur" | "font.lyrics");
    ui.horizontal(|ui| {
        let label_w = ui.painter().layout_no_wrap("background".into(), theme::regular(12.5), col::sub()).size().x + px(30.0);
        let (lr, _) = ui.allocate_exact_size(vec2(label_w, px(30.0)), Sense::hover());
        ui.painter().text(pos2(lr.left() + px(6.0), lr.center().y), Align2::LEFT_CENTER, label, theme::regular(12.5), col::sub());
        ui.painter().text(pos2(lr.right() - px(4.0), lr.center().y), Align2::RIGHT_CENTER, "=", theme::regular(12.5), col::muted());
        ui.add_space(sp(0.5));
        let (fr, _) = ui.allocate_exact_size(vec2(ui.available_width(), px(30.0)), Sense::hover());
        let mut buf = app.theme_fields.get(key).cloned().unwrap_or_else(|| current(app, key));
        let invalid = app.theme_fields.contains_key(key) && app.theme_msg.as_deref().is_some_and(|m| m.strip_prefix(key).is_some_and(|r| r.starts_with(':')));
        let id = Id::new(("theme-field", key));
        let focused = ui.ctx().memory(|m| m.has_focus(id));
        let stroke = if invalid {
            Stroke::new(1.0, col::accent())
        } else if focused {
            Stroke::new(1.0, col::focus())
        } else {
            Stroke::new(1.0, col::divider())
        };
        ui.painter().rect(fr, egui::CornerRadius::same(4), col::surface(), stroke, StrokeKind::Inside);
        let swatch_w = if is_color { px(22.0) } else { 0.0 };
        let text_rect = Rect::from_min_max(pos2(fr.left() + px(8.0), fr.top() + px(6.0)), pos2(fr.right() - swatch_w - px(8.0), fr.bottom() - px(6.0)));
        let resp = ui
            .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
                ui.add(egui::TextEdit::singleline(&mut buf).id(id).frame(egui::Frame::NONE).font(theme::regular(12.5)).text_color(col::text()).desired_width(text_rect.width()))
            })
            .inner;
        if resp.changed() {
            app.theme_fields.insert(key.to_string(), buf.clone());
        }
        if resp.lost_focus() && app.theme_fields.contains_key(key) {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                app.theme_fields.remove(key);
            } else {
                apply(app, key, &buf);
            }
        }
        if is_color {
            let sw = Rect::from_center_size(pos2(fr.right() - px(15.0), fr.center().y), vec2(px(16.0), px(16.0)));
            let color = Rgba::parse(&current(app, key)).map(|c| c.color32()).unwrap_or(Color32::TRANSPARENT);
            let sresp = ui.interact(sw, id.with("swatch"), Sense::click()).on_hover_text("Pick a color");
            ui.painter().rect(sw, egui::CornerRadius::same(3), color, Stroke::new(1.0, col::divider()), StrokeKind::Inside);
            // While the picker is dragged the theme only previews; the file is written once,
            // when the mouse lets go.
            let pending = id.with("picked");
            egui::Popup::from_toggle_button_response(&sresp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                let mut c = color;
                if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::OnlyBlend) {
                    let [r, g, b, a] = c.to_srgba_unmultiplied();
                    let hex = Rgba([r, g, b, a]).hex();
                    app.theme.preview(key, &hex);
                    ui.data_mut(|d| d.insert_temp(pending, hex));
                }
            });
            if !ui.input(|i| i.pointer.any_down())
                && let Some(hex) = ui.data_mut(|d| d.remove_temp::<String>(pending))
            {
                apply(app, key, &hex);
            }
        }
    });
}

/// A label on the left, the control on the right.
fn labeled(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (lr, _) = ui.allocate_exact_size(vec2(px(92.0), px(28.0)), Sense::hover());
        ui.painter().text(pos2(lr.left() + px(6.0), lr.center().y), Align2::LEFT_CENTER, label, theme::regular(12.5), col::sub());
        control(ui);
    });
}

/// A slider that previews while dragged and writes theme.toml when let go.
fn slider_row(app: &mut App, ui: &mut Ui, label: &str, key: &str, range: std::ops::RangeInclusive<f64>, int: bool) {
    slider_with(app, ui, label, key, range, int, "", "");
}

/// `slider_row` with a unit after the value and a note on hover.
#[allow(clippy::too_many_arguments)]
fn slider_with(app: &mut App, ui: &mut Ui, label: &str, key: &str, range: std::ops::RangeInclusive<f64>, int: bool, unit: &str, hint: &str) {
    let mut v: f64 = current(app, key).parse().unwrap_or(*range.start());
    labeled(ui, label, |ui| {
        let mut s = egui::Slider::new(&mut v, range).show_value(true).suffix(unit);
        s = if int { s.integer() } else { s.fixed_decimals(2) };
        let mut r = ui.add(s);
        if !hint.is_empty() {
            r = r.on_hover_text(hint);
        }
        let raw = if int { format!("{}", v.round() as i64) } else { format!("{v:.2}") };
        if r.changed() {
            app.theme.preview(key, &raw);
        }
        if r.drag_stopped() || (r.changed() && !r.dragged()) {
            apply(app, key, &raw);
        }
    });
}

/// One of a few words, as chips.
fn choice_row(app: &mut App, ui: &mut Ui, label: &str, key: &str, words: &[&str]) {
    let cur = current(app, key);
    labeled(ui, label, |ui| {
        for w in words {
            let r = ui.selectable_label(cur == *w, RichText::new(*w).font(theme::regular(12.0)));
            if r.clicked() && cur != *w {
                apply(app, key, w);
            }
        }
    });
}

fn bool_row(app: &mut App, ui: &mut Ui, label: &str, key: &str) {
    let mut on = current(app, key) == "true";
    labeled(ui, label, |ui| {
        if ui.checkbox(&mut on, "").changed() {
            apply(app, key, if on { "true" } else { "false" });
        }
    });
}

fn reset_row(app: &mut App, ui: &mut Ui, table: &str) {
    ui.add_space(sp(0.75));
    ui.horizontal(|ui| {
        ui.label(RichText::new("saved as you go").font(theme::regular(11.0)).color(col::muted()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::outline_button(ui, "reset").on_hover_text(format!("Back to the defaults for [{table}]")).clicked() {
                match app.theme.reset_table(table) {
                    Ok(()) => {
                        app.theme_fields.clear();
                        app.theme_msg = None;
                    }
                    Err(e) => app.theme_msg = Some(e),
                }
            }
        });
    });
}

fn lyrics_tab(app: &mut App, ui: &mut Ui) {
    use crate::theme_config::{LYRICS_ALIGN, LYRICS_ANIMATION, LYRICS_POSITION};
    ui.label(RichText::new("[lyrics]").font(theme::regular(12.5)).color(col::sub()));
    ui.add_space(sp(0.5));
    field_row(app, ui, "current", "lyrics.current");
    field_row(app, ui, "other", "lyrics.other");
    field_row(app, ui, "highlight", "lyrics.highlight");
    // Songs with word timing: when words light up, and how long they stay lit.
    slider_with(
        app,
        ui,
        "words early",
        "lyrics.word_lead",
        -500.0..=500.0,
        true,
        " ms",
        "With word timing: light each word up this much before it's sung (negative: after). The lines keep their time; the song's own timing is in the lyrics footer.",
    );
    slider_with(app, ui, "word glow", "lyrics.word_glow", 0.0..=2000.0, true, " ms", "With word timing: how long a sung word stays bright before it settles into the line's color.");
    slider_row(app, ui, "size", "lyrics.size", 0.5..=2.0, false);
    choice_row(app, ui, "align", "lyrics.align", LYRICS_ALIGN);
    choice_row(app, ui, "position", "lyrics.position", LYRICS_POSITION);
    slider_row(app, ui, "spacing", "lyrics.spacing", 0.5..=2.5, false);
    slider_row(app, ui, "fade", "lyrics.fade", 0.0..=1.0, false);
    choice_row(app, ui, "animation", "lyrics.animation", LYRICS_ANIMATION);
    slider_row(app, ui, "speed", "lyrics.speed", 0.25..=3.0, false);
    bool_row(app, ui, "bold line", "lyrics.bold");
    field_row(app, ui, "font", "font.lyrics");
}

/// One of `AlbumParts`' switches.
type Part = fn(&mut AlbumParts) -> &mut bool;

/// Album colors (Settings ▸ Display): on or off, where they apply, and which parts take
/// them; the parts left out keep the theme's colors. Kept in Glass's settings, not
/// theme.toml.
fn album_tab(app: &mut App, ui: &mut Ui) {
    let mut on = app.settings.album_colors;
    ui.horizontal(|ui| {
        ui.label(RichText::new("album colors").font(theme::regular(12.5)).color(col::sub()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.checkbox(&mut on, RichText::new("on").font(theme::regular(12.0))).on_hover_text("Take colors from the cover of what's playing").changed() {
                app.settings.album_colors = on;
                app.mark_settings_dirty();
            }
        });
    });
    ui.add_space(sp(0.5));
    ui.add_enabled_ui(on, |ui| {
        let everywhere = app.settings.album_everywhere;
        let mut pick = None;
        labeled(ui, "where", |ui| {
            for (all, word) in [(false, "now playing"), (true, "everywhere")] {
                if ui.selectable_label(everywhere == all, RichText::new(word).font(theme::regular(12.0))).clicked() {
                    pick = Some(all);
                }
            }
        });
        if let Some(all) = pick.filter(|&a| a != everywhere) {
            app.settings.album_everywhere = all;
            app.mark_settings_dirty();
        }
        let rows: [(&str, &str, Part); 4] = [
            ("accent", "The accent, and the progress bar, focus and selection drawn with it", |p| &mut p.accent),
            ("lyrics", "The lyrics' line colors and the word being sung", |p| &mut p.lyrics),
            ("visualizer", "The visualizer's gradient", |p| &mut p.visualizer),
            ("background", "The cover's colors washed in behind, and the player bar's tint", |p| &mut p.background),
        ];
        ui.add_space(sp(0.25));
        ui.label(RichText::new("album colors for").font(theme::regular(12.0)).color(col::sub()));
        for (label, hint, field) in rows {
            let mut v = *field(&mut app.settings.album_parts);
            if ui.checkbox(&mut v, RichText::new(label).font(theme::regular(12.5))).on_hover_text(hint).changed() {
                *field(&mut app.settings.album_parts) = v;
                app.mark_settings_dirty();
            }
        }
    });
    ui.add_space(sp(0.75));
    let note = if !on {
        "Off: the theme's colors everywhere."
    } else if app.settings.album_everywhere {
        "Every page takes the album's colors, as on iPhone; unticked parts keep the theme's. A new song's colors bloom out from its cover."
    } else {
        "Now Playing and the player bar take the album's colors; other pages, and unticked parts, keep the theme's."
    };
    ui.add(egui::Label::new(RichText::new(note).font(theme::regular(11.5)).color(col::muted())).wrap());
    // What the playing cover gives: its accent and the three colors of its wash.
    if let Some(p) = app.album_colors.shown().filter(|_| on) {
        ui.add_space(sp(0.75));
        ui.horizontal(|ui| {
            ui.label(RichText::new("now").font(theme::regular(12.0)).color(col::sub()));
            ui.add_space(sp(1.0));
            for c in std::iter::once(p.accent).chain(p.wash) {
                let (r, _) = ui.allocate_exact_size(vec2(px(18.0), px(18.0)), Sense::hover());
                ui.painter().circle(r.center(), r.width() / 2.0, c.rgba(1.0).color32(), Stroke::new(1.0, col::divider()));
            }
        });
    }
}

fn visualizer_tab(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("[visualizer]").font(theme::regular(12.5)).color(col::sub()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut on = app.settings.visualizer;
            if ui.checkbox(&mut on, RichText::new("show  V").font(theme::regular(12.0))).changed() {
                app.settings.visualizer = on;
                app.mark_settings_dirty();
            }
        });
    });
    ui.add_space(sp(0.5));
    choice_row(app, ui, "style", "visualizer.style", crate::theme_config::VIZ_STYLE);
    field_row(app, ui, "color", "visualizer.color");
    field_row(app, ui, "color2", "visualizer.color2");
    slider_row(app, ui, "opacity", "visualizer.opacity", 0.1..=1.0, false);
    slider_row(app, ui, "bars", "visualizer.bars", 12.0..=128.0, true);
    slider_row(app, ui, "height", "visualizer.height", 0.15..=0.6, false);
    slider_row(app, ui, "sensitivity", "visualizer.sensitivity", 0.25..=4.0, false);
    slider_row(app, ui, "smoothing", "visualizer.smoothing", 0.0..=0.95, false);
    bool_row(app, ui, "glow", "visualizer.glow");
}
