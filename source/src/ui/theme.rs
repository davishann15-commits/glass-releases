//! The look, from `~/.config/glass/theme.toml` (see `theme_config`).
//!
//! Colors come from `col::*`, fonts from `regular`, `medium`, `mono` and
//! `lyrics` (scaled by the theme's font size), spacing from `sp`, corners
//! from `radius`. The error red is fixed (`col::error`), and so is the Liked
//! Songs tile, named where it's drawn. `sync` rebuilds all of it when the theme
//! file changes.

use crate::theme_config::{Theme, font_files};
use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, Stroke};
use std::cell::RefCell;
use std::sync::Arc;

/// Mockup scale: sizes in the UI code are written for a 13 pt base font.
const BASE_FONT: f32 = 13.0;

#[derive(Clone)]
struct Current {
    theme: Theme,
    opacity: f32,
    generation: Option<u64>,
    family: String,
    /// The key of the override in effect (0: none), so one inside another is its own.
    over: u64,
}

thread_local! {
    static CURRENT: RefCell<Current> = RefCell::new(Current { theme: Theme::default(), opacity: 0.68, generation: None, family: String::new(), over: 0 });
    /// Themes recent overrides made, by key: a few at most (the page and the player
    /// bar each have one), kept so a change that holds still copies nothing.
    static MADE: RefCell<Vec<(u64, Theme)>> = const { RefCell::new(Vec::new()) };
}

fn with<R>(f: impl FnOnce(&Theme) -> R) -> R {
    CURRENT.with(|c| f(&c.borrow().theme))
}

/// How lyrics look (`[lyrics]` in theme.toml).
pub fn lyrics_style() -> crate::theme_config::LyricsStyle {
    with(|t| t.lyrics.clone())
}

/// How the visualizer looks (`[visualizer]` in theme.toml).
pub fn viz_style() -> crate::theme_config::VizStyle {
    with(|t| t.viz.clone())
}

/// Changes the theme for whatever draws while the guard lives (album colors on Now
/// Playing); the theme comes back when it's dropped.
#[must_use]
pub struct Override {
    saved: Option<Theme>,
    key: u64,
    outer: u64,
}

/// `key` says what `edit` does (the same key, the same change): the changed theme is
/// kept from one frame to the next and swapped in, so only a change in progress (a
/// fade) is worked out again.
pub fn override_with(key: impl std::hash::Hash, edit: impl FnOnce(&mut Theme)) -> Override {
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        let outer = c.over;
        // Also keyed by the theme file's version and the override it's inside.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            (key, outer, c.generation).hash(&mut h);
            h.finish()
        };
        let made = MADE.with(|m| {
            let mut m = m.borrow_mut();
            m.iter().position(|(k, _)| *k == key).map(|i| m.swap_remove(i).1)
        });
        let mut theme = made.unwrap_or_else(|| {
            let mut t = c.theme.clone();
            edit(&mut t);
            t
        });
        std::mem::swap(&mut c.theme, &mut theme);
        c.over = key;
        Override { saved: Some(theme), key, outer }
    })
}

impl Drop for Override {
    fn drop(&mut self) {
        let Some(theme) = self.saved.take() else { return };
        let made = CURRENT.with(|c| {
            let mut c = c.borrow_mut();
            c.over = self.outer;
            std::mem::replace(&mut c.theme, theme)
        });
        MADE.with(|m| {
            let mut m = m.borrow_mut();
            m.push((self.key, made));
            if m.len() > 4 {
                m.remove(0);
            }
        });
    }
}

/// Semantic colors.
pub mod col {
    use super::with;
    use eframe::egui::Color32;

    pub fn text() -> Color32 {
        with(|t| t.foreground.color32())
    }
    pub fn sub() -> Color32 {
        with(|t| t.text_secondary.color32())
    }
    pub fn muted() -> Color32 {
        with(|t| t.text_muted.color32())
    }
    pub fn accent() -> Color32 {
        with(|t| t.accent.color32())
    }
    /// The accent at `a` (0–1) of its alpha: tints and outlines.
    pub fn accent_a(a: f32) -> Color32 {
        with(|t| t.accent.with_alpha(a * t.accent.0[3] as f32 / 255.0).color32())
    }
    pub fn on_accent() -> Color32 {
        with(|t| t.on_accent.color32())
    }
    /// Errors and failed requests: a warm red that reads on any tint.
    pub fn error() -> Color32 {
        Color32::from_rgb(240, 140, 120)
    }
    pub fn border() -> Color32 {
        with(|t| t.border.color32())
    }
    /// The border, fainter: dividers between rows.
    pub fn divider() -> Color32 {
        with(|t| t.border.with_alpha(0.45 * t.border.0[3] as f32 / 255.0).color32())
    }
    pub fn surface() -> Color32 {
        with(|t| t.surface.color32())
    }
    pub fn raised() -> Color32 {
        with(|t| t.surface_raised.color32())
    }
    pub fn overlay() -> Color32 {
        with(|t| t.overlay.color32())
    }
    pub fn selection() -> Color32 {
        with(|t| t.selection.color32())
    }
    pub fn hover() -> Color32 {
        with(|t| t.hover.color32())
    }
    pub fn focus() -> Color32 {
        with(|t| t.focus.color32())
    }
    pub fn player_bg() -> Color32 {
        with(|t| t.player_background.color32())
    }
    pub fn progress() -> Color32 {
        with(|t| t.player_progress.color32())
    }
    pub fn track() -> Color32 {
        with(|t| t.player_track.color32())
    }
    /// The window's background tint, at the effective opacity.
    pub fn window() -> Color32 {
        super::CURRENT.with(|c| {
            let c = c.borrow();
            c.theme.background.with_alpha(c.opacity).color32()
        })
    }
}

/// `n` spacing units.
pub fn sp(n: f32) -> f32 {
    with(|t| t.spacing * n)
}

/// The theme's corner radius.
pub fn radius() -> CornerRadius {
    CornerRadius::same(with(|t| t.radius).round().clamp(0.0, 255.0) as u8)
}
pub fn radius_f() -> f32 {
    with(|t| t.radius)
}
pub fn small_radius() -> CornerRadius {
    CornerRadius::same((with(|t| t.radius) * 0.6).round().clamp(0.0, 255.0) as u8)
}

pub fn reduce_motion() -> bool {
    with(|t| t.reduce_motion)
}

fn scale() -> f32 {
    with(|t| t.font_size / BASE_FONT)
}

/// A size written at the 13 pt mockup scale, in the theme's scale.
pub fn px(size: f32) -> f32 {
    size * scale()
}

pub fn medium(size: f32) -> FontId {
    FontId::new(px(size), FontFamily::Name("medium".into()))
}
pub fn regular(size: f32) -> FontId {
    FontId::new(px(size), FontFamily::Proportional)
}
/// The lyrics typeface.
pub fn lyrics(size: f32, strong: bool) -> FontId {
    FontId::new(px(size), FontFamily::Name(if strong { "lyrics-bold" } else { "lyrics" }.into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::new(px(size), FontFamily::Monospace)
}

pub fn edge() -> Stroke {
    Stroke::new(1.0, col::border())
}
pub fn hairline() -> Stroke {
    Stroke::new(1.0, col::divider())
}

/// A glass card: faint surface, thin border.
pub fn panel() -> Frame {
    Frame::new().fill(col::surface()).stroke(edge()).corner_radius(radius()).inner_margin(Margin::same(sp(1.5) as i8))
}
pub fn card() -> Frame {
    Frame::new().fill(col::surface()).stroke(hairline()).corner_radius(radius()).inner_margin(Margin::same(sp(1.5) as i8))
}
/// Menus, popups, dialogs: nearly solid so text stays readable over anything.
pub fn popup() -> Frame {
    Frame::new().fill(col::overlay()).stroke(edge()).corner_radius(radius()).inner_margin(Margin::same(sp(0.75) as i8))
}

/// A font file, memory-mapped once and shared for the life of the app.
///
/// egui copies fonts it owns, so reading files into memory cost twice their
/// size (CJK fallbacks alone are ~100 MB). Mapped pages are read from disk
/// only for glyphs actually drawn, and the OS can drop them again at will.
fn mapped(path: &std::path::Path) -> Option<&'static [u8]> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static MAPS: Mutex<Option<HashMap<std::path::PathBuf, &'static [u8]>>> = Mutex::new(None);
    let mut maps = MAPS.lock().unwrap_or_else(|e| e.into_inner());
    let maps = maps.get_or_insert_with(HashMap::new);
    if let Some(b) = maps.get(path) {
        return Some(b);
    }
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: font files are read-only system or user fonts; the mapping is
    // never written and lives as long as the process.
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let bytes: &'static [u8] = Box::leak(Box::new(map));
    maps.insert(path.to_path_buf(), bytes);
    Some(bytes)
}

fn font_file(path: &std::path::Path, index: u32) -> Option<FontData> {
    let mut fd = FontData::from_static(mapped(path)?);
    fd.index = index;
    Some(fd)
}

/// The lyrics typeface at two weights: (regular, bold).
fn lyrics_fonts(family: &str) -> (Option<FontData>, Option<FontData>) {
    let variable = |path: &str, weight: f32| -> Option<FontData> {
        let mut fd = font_file(std::path::Path::new(path), 0)?;
        fd.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
        Some(fd)
    };
    let from_files = |name: &str| -> (Option<FontData>, Option<FontData>) {
        let f = font_files(name);
        let load = |p: &Option<(std::path::PathBuf, u32)>| p.as_ref().and_then(|(path, index)| font_file(path, *index));
        (load(&f.regular), load(&f.bold))
    };
    if family.eq_ignore_ascii_case("system") {
        if std::path::Path::new("/System/Library/Fonts/SFNS.ttf").exists() {
            return (variable("/System/Library/Fonts/SFNS.ttf", 520.0), variable("/System/Library/Fonts/SFNS.ttf", 720.0));
        }
        for name in ["Inter", "Noto Sans", "Cantarell", "DejaVu Sans"] {
            let f = from_files(name);
            if f.0.is_some() {
                return f;
            }
        }
        return (None, None);
    }
    from_files(family)
}

/// Fonts: the theme's family (regular / medium / bold), then CJK and
/// symbol fallbacks so any title renders, then egui's built-ins. Lyrics get
/// their own family.
fn install_fonts(ctx: &egui::Context, family: &str, lyrics: &str) {
    let mut fonts = FontDefinitions::default();
    let builtin: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut files = font_files(family);
    if files.regular.is_none() {
        // The default (SF Mono) is macOS's; elsewhere use the first monospace font installed.
        for name in ["Menlo", "JetBrains Mono", "JetBrainsMono Nerd Font Mono", "DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono", "Hack"] {
            files = font_files(name);
            if files.regular.is_some() {
                log::info!("font {family:?} isn't installed; using {name}");
                break;
            }
        }
    }
    let mut load = |name: &str, f: &Option<(std::path::PathBuf, u32)>| -> Option<String> {
        let (path, index) = f.as_ref()?;
        fonts.font_data.insert(name.to_string(), Arc::new(font_file(path, *index)?));
        Some(name.to_string())
    };
    let regular = load("ui-regular", &files.regular);
    let medium = load("ui-medium", &files.medium);
    let bold = load("ui-bold", &files.bold);
    // Fallbacks: (file, face in the collection). Bold text gets bold Japanese,
    // Chinese and Korean before the regular ones, so a bold line stays bold.
    let mut add = |prefix: &str, files: &[(&str, u32)]| -> Vec<String> {
        let mut names = Vec::new();
        for (i, (path, index)) in files.iter().enumerate() {
            if let Some(fd) = font_file(std::path::Path::new(path), *index) {
                let name = format!("{prefix}-{i}");
                fonts.font_data.insert(name.clone(), Arc::new(fd));
                names.push(name);
            }
        }
        names
    };
    let fallbacks = add(
        "fallback",
        &[
            ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", 0),
            ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
            ("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0),
            ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
            ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
        ],
    );
    let bold_fallbacks = add(
        "fallback-bold",
        &[
            ("/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc", 0),
            ("/System/Library/Fonts/Hiragino Sans GB.ttc", 2),
            ("/System/Library/Fonts/AppleSDGothicNeo.ttc", 6),
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc", 0),
            ("/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc", 0),
        ],
    );
    let chain = |first: &Option<String>, bold: bool| -> Vec<String> {
        let mut c: Vec<String> = first.iter().cloned().chain(regular.iter().cloned()).collect();
        c.dedup();
        if bold {
            c.extend(bold_fallbacks.iter().cloned());
        }
        c.extend(fallbacks.iter().cloned());
        c.extend(builtin.iter().cloned());
        c
    };
    fonts.families.insert(FontFamily::Proportional, chain(&regular, false));
    fonts.families.insert(FontFamily::Monospace, chain(&regular, false));
    fonts.families.insert(FontFamily::Name("medium".into()), chain(&medium, false));
    fonts.families.insert(FontFamily::Name("bold".into()), chain(&bold, true));
    let (lr, lb) = lyrics_fonts(lyrics);
    let lr = lr.map(|fd| {
        fonts.font_data.insert("lyrics-regular".into(), Arc::new(fd));
        "lyrics-regular".to_string()
    });
    let lb = lb.map(|fd| {
        fonts.font_data.insert("lyrics-bold".into(), Arc::new(fd));
        "lyrics-bold".to_string()
    });
    fonts.families.insert(FontFamily::Name("lyrics".into()), chain(&lr.or(regular.clone()), false));
    fonts.families.insert(FontFamily::Name("lyrics-bold".into()), chain(&lb.or(bold.clone()), true));
    ctx.set_fonts(fonts);
}

fn install_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let reduce = reduce_motion();
    ctx.all_styles_mut(|s| {
        s.visuals = egui::Visuals::dark();
        let v = &mut s.visuals;
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = col::overlay();
        v.window_stroke = edge();
        v.window_corner_radius = radius();
        v.extreme_bg_color = col::surface();
        v.faint_bg_color = col::hover();
        v.code_bg_color = col::raised();
        v.override_text_color = Some(col::text());
        v.selection.bg_fill = col::accent_a(0.35);
        v.selection.stroke = Stroke::new(1.0, col::accent());
        v.hyperlink_color = col::accent();
        v.text_cursor.stroke = Stroke::new(2.0, col::accent());
        for w in [&mut v.widgets.inactive, &mut v.widgets.noninteractive] {
            w.bg_fill = Color32::TRANSPARENT;
            w.weak_bg_fill = Color32::TRANSPARENT;
            w.bg_stroke = Stroke::NONE;
            w.corner_radius = small_radius();
            w.fg_stroke = Stroke::new(1.0, col::text());
        }
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, col::sub());
        v.widgets.noninteractive.bg_stroke = hairline();
        v.widgets.inactive.weak_bg_fill = col::surface();
        v.widgets.inactive.bg_fill = col::raised();
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, col::divider());
        v.widgets.hovered.weak_bg_fill = col::hover();
        v.widgets.hovered.bg_fill = col::raised();
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, col::border());
        v.widgets.hovered.corner_radius = small_radius();
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, col::text());
        v.widgets.active.weak_bg_fill = col::selection();
        v.widgets.active.bg_fill = col::accent_a(0.6);
        v.widgets.active.bg_stroke = Stroke::new(1.0, col::accent());
        v.widgets.active.corner_radius = small_radius();
        v.widgets.active.fg_stroke = Stroke::new(1.0, col::text());
        v.widgets.open = v.widgets.active;
        v.menu_corner_radius = radius();
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(80) };
        v.window_shadow = v.popup_shadow;
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;

        s.spacing.item_spacing = egui::vec2(sp(1.0), sp(0.75));
        s.spacing.button_padding = egui::vec2(sp(1.25), sp(0.6));
        s.spacing.slider_rail_height = 3.0;
        s.spacing.interact_size.y = px(26.0);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.text_styles.insert(egui::TextStyle::Body, regular(13.5));
        s.text_styles.insert(egui::TextStyle::Button, regular(13.0));
        s.text_styles.insert(egui::TextStyle::Small, regular(11.5));
        s.text_styles.insert(egui::TextStyle::Monospace, mono(13.0));
        s.text_styles.insert(egui::TextStyle::Heading, medium(22.0));
        s.interaction.selectable_labels = false;
        s.animation_time = if reduce { 0.0 } else { 0.12 };
    });
}

/// Applies the theme in `theme` if it changed since the last frame. Returns
/// true when this was the first font install, whose families only exist from
/// the next frame on.
pub fn sync(ctx: &egui::Context, theme: &Theme, generation: u64, opacity: f32) -> bool {
    let changed = CURRENT.with(|c| {
        let c = c.borrow();
        c.generation != Some(generation) || (c.opacity - opacity).abs() > f32::EPSILON
    });
    if !changed {
        return false;
    }
    let family_changed = CURRENT.with(|c| c.borrow().family != format!("{}|{}", theme.font_family, theme.lyrics_family));
    let first = CURRENT.with(|c| c.borrow().family.is_empty());
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        c.theme = theme.clone();
        c.opacity = opacity;
        c.generation = Some(generation);
        c.family = format!("{}|{}", theme.font_family, theme.lyrics_family);
    });
    if family_changed {
        install_fonts(ctx, &theme.font_family, &theme.lyrics_family);
    }
    install_style(ctx);
    ctx.request_repaint();
    first
}

/// Called once at startup, before the first frame, with the theme in use (so
/// fonts are found and installed once, not for the defaults and then again).
pub fn install(ctx: &egui::Context, theme: &Theme, generation: u64) {
    let _ = sync(ctx, theme, generation, crate::window_fx::effective_opacity(theme));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_config::Rgba;

    #[test]
    fn overrides_come_off_and_a_kept_one_is_the_same() {
        let base = col::accent();
        let red = |t: &mut Theme| t.accent = Rgba::rgb(255, 0, 0);
        {
            let _o = override_with("red", red);
            assert_eq!(col::accent(), Color32::from_rgb(255, 0, 0));
        }
        assert_eq!(col::accent(), base, "the theme comes back");
        // The second time the change is the kept one (`edit` isn't run): same colors.
        {
            let _o = override_with("red", |_| panic!("worked out again"));
            assert_eq!(col::accent(), Color32::from_rgb(255, 0, 0));
            // One inside another is its own, on top of the outer change.
            let _inner = override_with("green", |t| t.focus = Rgba::rgb(0, 255, 0));
            assert_eq!((col::accent(), col::focus()), (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0)));
        }
        assert_eq!(col::accent(), base);
        // The same key inside another override is a different change.
        let _o = override_with("blue", |t| t.accent = Rgba::rgb(0, 0, 255));
        let _inner = override_with("red", red);
        assert_eq!(col::accent(), Color32::from_rgb(255, 0, 0));
    }
}
