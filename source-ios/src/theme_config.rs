//! The theme: every color, size and font the UI uses, read from
//! `~/.config/glass/theme.toml` (or `$XDG_CONFIG_HOME/glass/theme.toml`).
//!
//! The file is optional and so is every key: missing values come from the
//! defaults (`theme.example.toml` documents them). A malformed file never
//! takes the app down — the last valid theme stays on screen and the error is
//! reported. Edits from the theme panel go through `toml_edit`, so comments
//! and everything else in the file survive.
//!
//! App preferences (volume, EQ, library layout…) stay in `settings.json`
//! under the platform's app-data folder, where they have always been; this
//! file only holds the look.

#[cfg(not(glass_mobile))]
use eframe::egui::Color32;
// The iOS core (ios/core) has no egui; the color type is the same one, from ecolor.
#[cfg(glass_mobile)]
use ecolor::Color32;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use toml_edit::{DocumentMut, Item, Value, value};

/// The commented default file, also shipped as `theme.example.toml`.
pub const DEFAULT_FILE: &str = include_str!("../theme.example.toml");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    /// `#RGB`, `#RRGGBB` or `#RRGGBBAA` (case-insensitive, `#` optional).
    pub fn parse(s: &str) -> Result<Self, String> {
        let h = s.trim().trim_start_matches('#');
        let bad = || format!("expected a hex color like #D89B7A or #D89B7A80, got {s:?}");
        if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| bad());
        let nib = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).map(|v| v * 17).map_err(|_| bad());
        match h.len() {
            3 => Ok(Self([nib(0)?, nib(1)?, nib(2)?, 255])),
            6 => Ok(Self([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Ok(Self([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => Err(bad()),
        }
    }

    pub fn hex(&self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
    }

    /// The same color at `alpha` (0–1) opacity.
    pub fn with_alpha(&self, alpha: f32) -> Self {
        let [r, g, b, _] = self.0;
        Self([r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8])
    }

    pub fn color32(&self) -> Color32 {
        let [r, g, b, a] = self.0;
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Theme {
    // [theme]
    pub background: Rgba,
    pub foreground: Rgba,
    pub accent: Rgba,
    pub border: Rgba,
    /// Background opacity, 0 (clear) to 1 (solid).
    pub opacity: f32,
    /// Background blur radius in points; 0 turns blur off.
    pub blur: u32,
    // [colors]
    pub surface: Rgba,
    pub surface_raised: Rgba,
    pub overlay: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub on_accent: Rgba,
    pub selection: Rgba,
    pub hover: Rgba,
    pub focus: Rgba,
    pub player_background: Rgba,
    pub player_progress: Rgba,
    pub player_track: Rgba,
    // [font]
    pub font_family: String,
    pub font_size: f32,
    /// The lyrics typeface; "system" means the platform's UI sans.
    pub lyrics_family: String,
    // [layout]
    pub radius: f32,
    pub spacing: f32,
    // [palette]
    pub palette: Vec<Rgba>,
    // [motion] — already resolved against the system setting
    pub reduce_motion: bool,
    // [lyrics] and [visualizer]
    pub lyrics: LyricsStyle,
    pub viz: VizStyle,
}

/// How lyrics look on the now-playing screen (`[lyrics]`).
#[derive(Clone, PartialEq, Debug)]
pub struct LyricsStyle {
    /// The line being sung.
    pub current: Rgba,
    /// The other lines (they fade further with distance).
    pub other: Rgba,
    /// The word or syllable being sung, with word timing.
    pub highlight: Rgba,
    /// Text size, relative to the automatic size (0.5–2).
    pub size: f32,
    /// "left", "center" or "right".
    pub align: String,
    /// Where the current line sits: "top", "upper" or "middle".
    pub position: String,
    /// Line spacing, relative (0.5–2.5).
    pub spacing: f32,
    /// How much far lines fade, 0 (not at all) to 1.
    pub fade: f32,
    /// How a new line arrives: "glide", "fade", "pop" or "none".
    pub animation: String,
    /// Animation speed, relative (0.25–3; higher is quicker).
    pub speed: f32,
    /// The current line in bold.
    pub bold: bool,
    /// With word timing: how much earlier (ms; negative: later) words light up
    /// than they're sung. The lines keep their time.
    pub word_lead: i32,
    /// With word timing: how long (ms) a sung word stays bright before it
    /// settles into the line's color.
    pub word_glow: u32,
}

/// The audio visualizer under the lyrics (`[visualizer]`).
#[derive(Clone, PartialEq, Debug)]
pub struct VizStyle {
    /// "bars", "mirror" or "wave".
    pub style: String,
    /// Bottom of the gradient (quiet).
    pub color: Rgba,
    /// Top of the gradient (loud).
    pub color2: Rgba,
    pub opacity: f32,
    pub bars: u32,
    /// Share of the right side it takes under the lyrics (0.15–0.6).
    pub height: f32,
    pub sensitivity: f32,
    /// How smoothly it moves, 0 (jumpy) to 0.95 (slow).
    pub smoothing: f32,
    pub glow: bool,
}

pub const LYRICS_ALIGN: &[&str] = &["left", "center", "right"];
pub const LYRICS_POSITION: &[&str] = &["top", "upper", "middle"];
pub const LYRICS_ANIMATION: &[&str] = &["glide", "fade", "pop", "none"];
pub const VIZ_STYLE: &[&str] = &["bars", "mirror", "wave"];

impl Default for Theme {
    fn default() -> Self {
        Theme::from_doc(&DocumentMut::new(), false).expect("defaults are valid")
    }
}

/// What kind of value a key takes, and its allowed range.
enum Kind {
    Color,
    Number(f64, f64),
    Int(i64, i64),
    Text,
    /// One of these words.
    Choice(&'static [&'static str]),
    Bool,
}

/// Every key the file understands, with its kind. Keys missing from this
/// list are reported as warnings (likely typos), not errors.
const KEYS: &[(&str, Kind)] = &[
    ("theme.background", Kind::Color),
    ("theme.foreground", Kind::Color),
    ("theme.accent", Kind::Color),
    ("theme.border", Kind::Color),
    ("theme.opacity", Kind::Number(0.0, 1.0)),
    ("theme.blur", Kind::Int(0, 100)),
    ("colors.surface", Kind::Color),
    ("colors.surface_raised", Kind::Color),
    ("colors.overlay", Kind::Color),
    ("colors.text_secondary", Kind::Color),
    ("colors.text_muted", Kind::Color),
    ("colors.on_accent", Kind::Color),
    ("colors.selection", Kind::Color),
    ("colors.hover", Kind::Color),
    ("colors.focus", Kind::Color),
    ("colors.player_background", Kind::Color),
    ("colors.player_progress", Kind::Color),
    ("colors.player_track", Kind::Color),
    ("font.family", Kind::Text),
    ("font.size", Kind::Number(8.0, 32.0)),
    ("font.lyrics", Kind::Text),
    ("layout.radius", Kind::Number(0.0, 32.0)),
    ("layout.spacing", Kind::Number(2.0, 24.0)),
    ("motion.reduce", Kind::Text),
    ("palette.colors", Kind::Text),
    ("lyrics.current", Kind::Color),
    ("lyrics.other", Kind::Color),
    ("lyrics.highlight", Kind::Color),
    ("lyrics.size", Kind::Number(0.5, 2.0)),
    ("lyrics.align", Kind::Choice(LYRICS_ALIGN)),
    ("lyrics.position", Kind::Choice(LYRICS_POSITION)),
    ("lyrics.spacing", Kind::Number(0.5, 2.5)),
    ("lyrics.fade", Kind::Number(0.0, 1.0)),
    ("lyrics.animation", Kind::Choice(LYRICS_ANIMATION)),
    ("lyrics.speed", Kind::Number(0.25, 3.0)),
    ("lyrics.bold", Kind::Bool),
    ("lyrics.word_lead", Kind::Int(-500, 500)),
    ("lyrics.word_glow", Kind::Int(0, 2000)),
    ("visualizer.style", Kind::Choice(VIZ_STYLE)),
    ("visualizer.color", Kind::Color),
    ("visualizer.color2", Kind::Color),
    ("visualizer.opacity", Kind::Number(0.1, 1.0)),
    ("visualizer.bars", Kind::Int(12, 128)),
    ("visualizer.height", Kind::Number(0.15, 0.6)),
    ("visualizer.sensitivity", Kind::Number(0.25, 4.0)),
    ("visualizer.smoothing", Kind::Number(0.0, 0.95)),
    ("visualizer.glow", Kind::Bool),
];

fn kind_of(key: &str) -> Option<&'static Kind> {
    KEYS.iter().find(|(k, _)| *k == key).map(|(_, kind)| kind)
}

fn lookup<'a>(doc: &'a DocumentMut, key: &str) -> Option<&'a Item> {
    let (table, field) = key.split_once('.')?;
    doc.get(table)?.get(field)
}

fn number(item: &Item) -> Option<f64> {
    item.as_float().or_else(|| item.as_integer().map(|i| i as f64))
}

impl Theme {
    /// Builds a theme from a parsed file. Every problem is collected, so one
    /// pass reports them all.
    fn from_doc(doc: &DocumentMut, system_reduce_motion: bool) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        let color = |key: &str, default: Rgba, errors: &mut Vec<String>| -> Rgba {
            match lookup(doc, key) {
                None => default,
                Some(item) => match item.as_str().map(Rgba::parse) {
                    Some(Ok(c)) => c,
                    Some(Err(e)) => {
                        errors.push(format!("{key}: {e}"));
                        default
                    }
                    None => {
                        errors.push(format!("{key}: expected a quoted hex color like \"#D89B7A\""));
                        default
                    }
                },
            }
        };
        let num = |key: &str, default: f64, errors: &mut Vec<String>| -> f64 {
            let Some(item) = lookup(doc, key) else { return default };
            let (lo, hi) = match kind_of(key) {
                Some(Kind::Number(lo, hi)) => (*lo, *hi),
                Some(Kind::Int(lo, hi)) => {
                    // 12.5 bars isn't rounded to something: it's a mistake to point out.
                    if item.as_integer().is_none() && item.as_float().is_some() {
                        errors.push(format!("{key}: expected a whole number between {lo} and {hi}"));
                        return default;
                    }
                    (*lo as f64, *hi as f64)
                }
                _ => (f64::MIN, f64::MAX),
            };
            match number(item) {
                Some(v) if (lo..=hi).contains(&v) => v,
                Some(v) => {
                    errors.push(format!("{key}: {v} is out of range ({lo}–{hi})"));
                    default
                }
                None => {
                    errors.push(format!("{key}: expected a number between {lo} and {hi}"));
                    default
                }
            }
        };

        let background = color("theme.background", Rgba::rgb(0x0F, 0x11, 0x15), &mut errors);
        let foreground = color("theme.foreground", Rgba::rgb(0xE9, 0xE4, 0xDC), &mut errors);
        let accent = color("theme.accent", Rgba::rgb(0x4F, 0x83, 0xA3), &mut errors);
        let border = color("theme.border", Rgba::rgb(0x09, 0x0A, 0x0B), &mut errors);
        let opacity = num("theme.opacity", 0.45, &mut errors) as f32;
        let blur = num("theme.blur", 28.0, &mut errors) as u32;

        let fg = |a: f32| foreground.with_alpha(a);
        let surface = color("colors.surface", fg(0.04), &mut errors);
        let surface_raised = color("colors.surface_raised", fg(0.08), &mut errors);
        let overlay = color("colors.overlay", background.with_alpha(0.94), &mut errors);
        let text_secondary = color("colors.text_secondary", fg(0.80), &mut errors);
        let text_muted = color("colors.text_muted", fg(0.62), &mut errors);
        let on_accent = color("colors.on_accent", foreground, &mut errors);
        let selection = color("colors.selection", accent.with_alpha(0.16), &mut errors);
        let hover = color("colors.hover", fg(0.06), &mut errors);
        let focus = color("colors.focus", accent, &mut errors);
        let player_background = color("colors.player_background", background.with_alpha((opacity + 0.12).min(1.0)), &mut errors);
        let player_progress = color("colors.player_progress", accent, &mut errors);
        let player_track = color("colors.player_track", fg(0.25), &mut errors);

        let font_family = match lookup(doc, "font.family") {
            None => "SF Mono".to_string(),
            Some(item) => match item.as_str().map(str::trim) {
                Some(f) if !f.is_empty() => f.to_string(),
                _ => {
                    errors.push("font.family: expected a font name in quotes, like \"SF Mono\"".into());
                    "SF Mono".to_string()
                }
            },
        };
        let font_size = num("font.size", 13.0, &mut errors) as f32;
        let lyrics_family = match lookup(doc, "font.lyrics") {
            None => "SF Mono".to_string(),
            Some(item) => match item.as_str().map(str::trim) {
                Some(f) if !f.is_empty() => f.to_string(),
                _ => {
                    errors.push("font.lyrics: expected a font name in quotes, or \"system\"".into());
                    "SF Mono".to_string()
                }
            },
        };
        let radius = num("layout.radius", 10.0, &mut errors) as f32;
        let spacing = num("layout.spacing", 8.0, &mut errors) as f32;

        let reduce_motion = match lookup(doc, "motion.reduce") {
            None => system_reduce_motion,
            Some(item) => match (item.as_bool(), item.as_str()) {
                (Some(b), _) => b,
                (_, Some("system")) => system_reduce_motion,
                _ => {
                    errors.push("motion.reduce: expected true, false or \"system\"".into());
                    system_reduce_motion
                }
            },
        };

        let default_palette = vec![background, border, foreground, accent, Rgba::rgb(0x5D, 0xA9, 0xD6), Rgba::rgb(0xA8, 0x7C, 0x9F)];
        let palette = match lookup(doc, "palette.colors") {
            None => default_palette,
            Some(item) => match item.as_array() {
                Some(arr) => {
                    let mut out = Vec::new();
                    for (i, v) in arr.iter().enumerate() {
                        match v.as_str().map(Rgba::parse) {
                            Some(Ok(c)) => out.push(c),
                            Some(Err(e)) => errors.push(format!("palette.colors[{i}]: {e}")),
                            None => errors.push(format!("palette.colors[{i}]: expected a quoted hex color")),
                        }
                    }
                    out
                }
                None => {
                    errors.push("palette.colors: expected a list like [\"#171A1F\", \"#D89B7A\"]".into());
                    default_palette
                }
            },
        };

        let choice = |key: &str, default: &str, errors: &mut Vec<String>| -> String {
            let Some(item) = lookup(doc, key) else { return default.to_string() };
            let Some(Kind::Choice(words)) = kind_of(key) else { return default.to_string() };
            match item.as_str().map(str::trim) {
                Some(w) if words.contains(&w) => w.to_string(),
                _ => {
                    errors.push(format!("{key}: expected one of {}", words.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(", ")));
                    default.to_string()
                }
            }
        };
        let flag = |key: &str, default: bool, errors: &mut Vec<String>| -> bool {
            match lookup(doc, key) {
                None => default,
                Some(item) => item.as_bool().unwrap_or_else(|| {
                    errors.push(format!("{key}: expected true or false"));
                    default
                }),
            }
        };
        let lyrics = LyricsStyle {
            current: color("lyrics.current", Rgba::rgb(0x6D, 0x3A, 0x78), &mut errors),
            other: color("lyrics.other", Rgba::rgb(0x80, 0xD0, 0xD8), &mut errors),
            highlight: color("lyrics.highlight", accent, &mut errors),
            size: num("lyrics.size", 0.8, &mut errors) as f32,
            align: choice("lyrics.align", "center", &mut errors),
            position: choice("lyrics.position", "upper", &mut errors),
            spacing: num("lyrics.spacing", 1.5, &mut errors) as f32,
            fade: num("lyrics.fade", 0.59, &mut errors) as f32,
            animation: choice("lyrics.animation", "glide", &mut errors),
            speed: num("lyrics.speed", 0.75, &mut errors) as f32,
            bold: flag("lyrics.bold", true, &mut errors),
            word_lead: num("lyrics.word_lead", 0.0, &mut errors) as i32,
            word_glow: num("lyrics.word_glow", 400.0, &mut errors) as u32,
        };
        let viz = VizStyle {
            style: choice("visualizer.style", "mirror", &mut errors),
            color: color("visualizer.color", accent, &mut errors),
            color2: color("visualizer.color2", foreground.with_alpha(0.9), &mut errors),
            opacity: num("visualizer.opacity", 0.91, &mut errors) as f32,
            bars: num("visualizer.bars", 54.0, &mut errors) as u32,
            height: num("visualizer.height", 0.26, &mut errors) as f32,
            sensitivity: num("visualizer.sensitivity", 1.1, &mut errors) as f32,
            smoothing: num("visualizer.smoothing", 0.65, &mut errors) as f32,
            glow: flag("visualizer.glow", true, &mut errors),
        };

        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Theme {
            background,
            foreground,
            accent,
            border,
            opacity,
            blur,
            surface,
            surface_raised,
            overlay,
            text_secondary,
            text_muted,
            on_accent,
            selection,
            hover,
            focus,
            player_background,
            player_progress,
            player_track,
            font_family,
            font_size,
            lyrics_family,
            radius,
            spacing,
            palette,
            reduce_motion,
            lyrics,
            viz,
        })
    }
}

/// Keys in the file that Glass doesn't know (probably typos).
fn unknown_keys(doc: &DocumentMut) -> Vec<String> {
    let mut out = Vec::new();
    for (table, item) in doc.iter() {
        match item.as_table_like() {
            Some(t) => {
                for (field, _) in t.iter() {
                    let key = format!("{table}.{field}");
                    if kind_of(&key).is_none() {
                        out.push(key);
                    }
                }
            }
            None => out.push(table.to_string()),
        }
    }
    out
}

/// Parses a whole file: the theme, or every error found.
pub fn parse(text: &str, system_reduce_motion: bool) -> Result<(Theme, Vec<String>), String> {
    let doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| format!("not valid TOML: {}", e.to_string().trim()))?;
    let theme = Theme::from_doc(&doc, system_reduce_motion).map_err(|errs| errs.join("\n"))?;
    Ok((theme, unknown_keys(&doc).into_iter().map(|k| format!("unknown key {k} (ignored)")).collect()))
}

pub fn config_dir() -> PathBuf {
    // iOS: an app may only write inside Documents, Library and tmp of its container
    // (its "home"), so ~/.config can't be created ("Operation not permitted"). The
    // hand-edited files live with the rest of Glass's data, in Library/Application Support/Glass.
    #[cfg(glass_mobile)]
    {
        crate::config::config_dir()
    }
    #[cfg(not(glass_mobile))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("glass")
    }
}

/// Font files for a family: (path, index within a .ttc collection).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontFiles {
    pub regular: Option<(PathBuf, u32)>,
    pub medium: Option<(PathBuf, u32)>,
    pub bold: Option<(PathBuf, u32)>,
}

fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![
        home.join("Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        // Supplemental is read as part of this one.
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts"),
        home.join(".local/share/fonts"),
        home.join(".fonts"),
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ]
}

/// Letters and digits, lowercased: "SF Mono" and "SFMono-Regular" both start "sfmono".
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Every installed font file (upright styles only), with its squashed name.
/// Read once; fonts installed while Glass runs show up after a restart.
fn installed_fonts() -> &'static [(PathBuf, String)] {
    static FONTS: std::sync::OnceLock<Vec<(PathBuf, String)>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut out = Vec::new();
        let mut stack: Vec<(PathBuf, u8)> = font_dirs().into_iter().map(|d| (d, 0)).collect();
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    if depth < 3 {
                        stack.push((p, depth + 1));
                    }
                    continue;
                }
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                let stem = p.file_stem().and_then(|x| x.to_str()).map(squash).unwrap_or_default();
                if matches!(ext.as_str(), "ttf" | "otf" | "ttc") && !stem.contains("italic") && !stem.contains("oblique") {
                    out.push((p, stem));
                }
            }
        }
        out
    })
}

/// Finds a family's regular, medium and bold files among installed fonts.
/// Medium and bold fall back to the nearest weight that exists.
pub fn font_files(family: &str) -> FontFiles {
    let want = squash(family);
    if want.is_empty() {
        return FontFiles::default();
    }
    let found: Vec<PathBuf> = installed_fonts().iter().filter(|(_, stem)| stem.starts_with(&want)).map(|(p, _)| p.clone()).collect();
    let weight = |p: &Path| -> String { p.file_stem().and_then(|x| x.to_str()).map(squash).unwrap_or_default()[want.len()..].to_string() };
    let pick = |names: &[&str]| -> Option<(PathBuf, u32)> {
        for n in names {
            if let Some(p) = found.iter().find(|p| weight(p) == *n) {
                return Some((p.clone(), 0));
            }
        }
        None
    };
    // A single collection file (e.g. Menlo.ttc) holds regular at 0 and bold at 1.
    if let Some(ttc) = found.iter().find(|p| weight(p).is_empty() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttc"))) {
        return FontFiles { regular: Some((ttc.clone(), 0)), medium: Some((ttc.clone(), 0)), bold: Some((ttc.clone(), 1)) };
    }
    let regular = pick(&["regular", "", "book", "text"]);
    let medium = pick(&["medium", "semibold", "regular", ""]);
    let bold = pick(&["bold", "semibold", "heavy", "medium"]);
    FontFiles { medium: medium.or_else(|| regular.clone()), bold: bold.or_else(|| regular.clone()), regular }
}

/// When a file last changed (and its size), to notice edits cheaply.
pub(crate) fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
}

/// Writes beside the file, then renames, so a crash never leaves half a file.
/// A symlinked file (say, into a dotfiles repository) is written where it points.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path = real.as_path();
    let dir = path.parent().ok_or("bad config path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("couldn't replace {}: {e}", path.display()))
}

/// The theme file on disk and the theme currently in use.
pub struct ThemeStore {
    path: PathBuf,
    /// The file as last read, for previews that shouldn't touch the disk.
    doc: DocumentMut,
    theme: Theme,
    error: Option<String>,
    warnings: Vec<String>,
    stamp: Option<(SystemTime, u64)>,
    /// The file was found empty: most editors save by emptying it first, so it's
    /// taken as empty only if it's still that way at the next check.
    empty: Option<(SystemTime, u64)>,
    checked: Instant,
    generation: u64,
    system_reduce_motion: bool,
}

const POLL: Duration = Duration::from_millis(500);

impl ThemeStore {
    pub fn load() -> Self {
        Self::at(config_dir().join("theme.toml"), crate::window_fx::system_reduce_motion())
    }

    pub fn at(path: PathBuf, system_reduce_motion: bool) -> Self {
        let mut s = Self {
            path,
            doc: DocumentMut::new(),
            theme: Theme::from_doc(&DocumentMut::new(), system_reduce_motion).expect("defaults are valid"),
            error: None,
            warnings: Vec::new(),
            stamp: None,
            empty: None,
            checked: Instant::now(),
            generation: 0,
            system_reduce_motion,
        };
        s.reload();
        s
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Bumps whenever the theme in use changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Why the file couldn't be used, if it couldn't. The previous theme stays.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Non-fatal notes, e.g. unknown keys.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn file_stamp(&self) -> Option<(SystemTime, u64)> {
        file_stamp(&self.path)
    }

    /// Re-reads the file if it changed on disk (checked at most twice a
    /// second). True when the theme in use changed.
    pub fn poll(&mut self) -> bool {
        if self.checked.elapsed() < POLL {
            return false;
        }
        self.checked = Instant::now();
        let stamp = self.file_stamp();
        if stamp == self.stamp {
            return false;
        }
        if let Some(s) = stamp.filter(|s| s.1 == 0)
            && self.empty != Some(s)
        {
            self.empty = Some(s);
            return false;
        }
        self.empty = None;
        self.reload()
    }

    /// Reads the file now. A missing file means the defaults. True when the
    /// theme in use changed.
    pub fn reload(&mut self) -> bool {
        self.stamp = self.file_stamp();
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                self.error = Some(format!("{}: {e}", self.path.display()));
                return false;
            }
        };
        match parse(&text, self.system_reduce_motion) {
            Ok((theme, warnings)) => {
                self.error = None;
                self.warnings = warnings;
                self.doc = text.parse().unwrap_or_default();
                if theme != self.theme {
                    self.theme = theme;
                    self.generation += 1;
                    return true;
                }
                false
            }
            Err(e) => {
                log::warn!("theme.toml: {e}");
                self.error = Some(e);
                false
            }
        }
    }

    /// Writes the commented default file if there is none yet.
    pub fn ensure_file(&self) -> Result<(), String> {
        if self.path.exists() {
            return Ok(());
        }
        self.write(DEFAULT_FILE)
    }

    fn write(&self, text: &str) -> Result<(), String> {
        write_atomic(&self.path, text)
    }

    /// Edits the file with `f` and applies the result, refusing to touch a
    /// file that doesn't parse (so a typo there is never overwritten).
    fn edit(&mut self, f: impl FnOnce(&mut DocumentMut)) -> Result<(), String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_FILE.to_string(),
            Err(e) => return Err(format!("couldn't read {}: {e}", self.path.display())),
        };
        let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| format!("fix theme.toml first — it isn't valid TOML: {}", e.to_string().trim()))?;
        f(&mut doc);
        let out = doc.to_string();
        // Validate before writing; a bad value never reaches the file.
        parse(&out, self.system_reduce_motion)?;
        self.write(&out)?;
        self.reload();
        Ok(())
    }

    /// Sets one key, e.g. `set("theme.accent", "#D89B7A")` or
    /// `set("theme.opacity", "0.7")`, checking it first. Comments and every
    /// other key in the file are kept.
    pub fn set(&mut self, key: &str, raw: &str) -> Result<(), String> {
        let kind = kind_of(key).ok_or_else(|| format!("unknown theme key {key}"))?;
        let raw = raw.trim();
        let v: Value = match kind {
            Kind::Color => Rgba::parse(raw).map(|c| Value::from(c.hex())).map_err(|e| format!("{key}: {e}"))?,
            Kind::Number(lo, hi) => match raw.parse::<f64>() {
                Ok(n) if (*lo..=*hi).contains(&n) => Value::from(n),
                _ => return Err(format!("{key}: expected a number between {lo} and {hi}")),
            },
            Kind::Int(lo, hi) => match raw.parse::<i64>() {
                Ok(n) if (*lo..=*hi).contains(&n) => Value::from(n),
                _ => return Err(format!("{key}: expected a whole number between {lo} and {hi}")),
            },
            Kind::Text if key == "palette.colors" => return Err("edit the palette in theme.toml".into()),
            Kind::Text if key == "motion.reduce" => match raw {
                "true" => Value::from(true),
                "false" => Value::from(false),
                "system" => Value::from("system"),
                _ => return Err("motion.reduce: expected true, false or \"system\"".into()),
            },
            Kind::Text => Value::from(raw),
            Kind::Choice(words) => match words.iter().find(|w| **w == raw) {
                Some(w) => Value::from(*w),
                None => return Err(format!("{key}: expected one of {}", words.join(", "))),
            },
            Kind::Bool => match raw {
                "true" => Value::from(true),
                "false" => Value::from(false),
                _ => return Err(format!("{key}: expected true or false")),
            },
        };
        let (table, field) = key.split_once('.').expect("keys are table.field");
        self.edit(|doc| {
            if !doc.contains_table(table) {
                doc[table] = toml_edit::table();
            }
            // Replace the value but keep any comment after it on the same line.
            let decor = doc[table].get(field).and_then(Item::as_value).map(|old| old.decor().clone());
            let mut v = v;
            if let Some(d) = decor {
                *v.decor_mut() = d;
            }
            doc[table][field] = Item::Value(v);
        })
    }

    /// A key's value as the file has it ("system" for `motion.reduce = "system"`),
    /// or `None` when the file doesn't set it.
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    pub fn written(&self, key: &str) -> Option<String> {
        let v = lookup(&self.doc, key)?.as_value()?;
        Some(v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string().trim().to_string()))
    }

    /// Shows `key = raw` without writing the file (while a slider or color is
    /// dragged); `set` makes it stick. Ignored if the value isn't valid.
    pub fn preview(&mut self, key: &str, raw: &str) {
        let Some(kind) = kind_of(key) else { return };
        let mut doc = self.doc.clone();
        let v: Value = match kind {
            Kind::Color => match Rgba::parse(raw.trim()) {
                Ok(c) => Value::from(c.hex()),
                Err(_) => return,
            },
            Kind::Number(..) => match raw.parse::<f64>() {
                Ok(n) => Value::from(n),
                Err(_) => return,
            },
            Kind::Int(..) => match raw.parse::<i64>() {
                Ok(n) => Value::from(n),
                Err(_) => return,
            },
            _ => return,
        };
        let (table, field) = key.split_once('.').expect("keys are table.field");
        if !doc.contains_table(table) {
            doc[table] = toml_edit::table();
        }
        doc[table][field] = Item::Value(v);
        if let Ok(theme) = Theme::from_doc(&doc, self.system_reduce_motion)
            && theme != self.theme
        {
            self.theme = theme;
            self.generation += 1;
        }
    }

    /// Removes every key in `[table]` (e.g. "lyrics"), so the defaults apply.
    pub fn reset_table(&mut self, table: &str) -> Result<(), String> {
        let table = table.to_string();
        self.edit(|doc| {
            if let Some(t) = doc.get_mut(&table).and_then(Item::as_table_like_mut) {
                let keys: Vec<String> = t.iter().map(|(k, _)| k.to_string()).collect();
                for k in keys {
                    t.remove(&k);
                }
            }
        })
    }

    /// Restores the six `[theme]` values to the defaults, leaving the rest.
    pub fn reset_panel_values(&mut self) -> Result<(), String> {
        let d = Theme::from_doc(&DocumentMut::new(), self.system_reduce_motion).expect("defaults are valid");
        let pairs = [
            ("background", value(d.background.hex())),
            ("foreground", value(d.foreground.hex())),
            ("accent", value(d.accent.hex())),
            ("border", value(d.border.hex())),
            // Rounded: 0.45 as an f32 widens to 0.44999998807907104.
            ("opacity", value((d.opacity as f64 * 1000.0).round() / 1000.0)),
            ("blur", value(d.blur as i64)),
        ];
        self.edit(|doc| {
            if !doc.contains_table("theme") {
                doc["theme"] = toml_edit::table();
            }
            for (k, v) in pairs {
                let decor = doc["theme"].get(k).and_then(Item::as_value).map(|old| old.decor().clone());
                let mut v = v;
                if let (Some(d), Some(nv)) = (decor, v.as_value_mut()) {
                    *nv.decor_mut() = d;
                }
                doc["theme"][k] = v;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (ThemeStore, PathBuf) {
        let dir = crate::config::test_dir(&format!("theme-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("theme.toml");
        (ThemeStore::at(path.clone(), false), path)
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn colors() {
        assert_eq!(Rgba::parse("#D89B7A"), Ok(Rgba([0xD8, 0x9B, 0x7A, 255])));
        assert_eq!(Rgba::parse("d89b7a80"), Ok(Rgba([0xD8, 0x9B, 0x7A, 0x80])));
        assert_eq!(Rgba::parse("#fff"), Ok(Rgba([255, 255, 255, 255])));
        assert!(Rgba::parse("#12345").is_err());
        assert!(Rgba::parse("orange").is_err());
        assert!(Rgba::parse("#ééé").is_err());
        assert_eq!(Rgba([1, 2, 3, 255]).hex(), "#010203");
        assert_eq!(Rgba([1, 2, 3, 0x80]).hex(), "#01020380");
        assert_eq!(Rgba::rgb(9, 9, 9).with_alpha(0.5).0[3], 128);
    }

    #[test]
    fn defaults_match_the_mockup_and_the_example_file() {
        let d = Theme::default();
        assert_eq!(d.background.hex(), "#0F1115");
        assert_eq!(d.foreground.hex(), "#E9E4DC");
        assert_eq!(d.accent.hex(), "#4F83A3");
        assert_eq!(d.border.hex(), "#090A0B");
        assert_eq!((d.opacity, d.blur), (0.45, 28));
        assert_eq!((d.font_family.as_str(), d.lyrics_family.as_str()), ("SF Mono", "SF Mono"));
        // The shipped example spells out exactly the defaults.
        let (from_file, warnings) = parse(DEFAULT_FILE, false).unwrap();
        assert_eq!(from_file, d);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn the_example_files_commented_values_are_the_defaults() {
        // "# size = 0.8" says what you get without the line, so with every such
        // line uncommented the theme is still the default one.
        let uncommented: String = DEFAULT_FILE
            .lines()
            .map(|l| match l.strip_prefix("# ") {
                Some(rest) if rest.split_once(" = ").is_some_and(|(k, _)| k.trim_end().chars().all(|c| c.is_ascii_lowercase() || c == '_')) => rest,
                _ => l,
            })
            .map(|l| format!("{l}\n"))
            .collect();
        let (from_file, warnings) = parse(&uncommented, false).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(from_file, Theme::default());
    }

    #[test]
    fn lyrics_and_visualizer_styles() {
        let d = Theme::default();
        assert_eq!((d.lyrics.highlight, d.viz.color), (d.accent, d.accent), "the sung word and the visualizer follow the accent");
        assert_eq!((d.lyrics.current.hex(), d.lyrics.other.hex()), ("#6D3A78".to_string(), "#80D0D8".to_string()));
        assert_eq!((d.lyrics.align.as_str(), d.lyrics.animation.as_str(), d.viz.style.as_str(), d.viz.bars), ("center", "glide", "mirror", 54));
        let (t, w) = parse("[lyrics]\nalign = \"center\"\nsize = 1.4\nbold = false\ncurrent = \"#FFFFFF\"\n[visualizer]\nstyle = \"wave\"\nbars = 64\nglow = false", false).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((t.lyrics.align.as_str(), t.lyrics.size, t.lyrics.bold, t.lyrics.current.hex()), ("center", 1.4, false, "#FFFFFF".to_string()));
        assert_eq!((t.viz.style.as_str(), t.viz.bars, t.viz.glow), ("wave", 64, false));
        assert_eq!((d.lyrics.word_lead, d.lyrics.word_glow), (0, 400), "words light as they're sung and glow 0.4 s");
        let (t, _) = parse("[lyrics]\nword_lead = -150\nword_glow = 1200", false).unwrap();
        assert_eq!((t.lyrics.word_lead, t.lyrics.word_glow), (-150, 1200));
        let e = parse("[lyrics]\nalign = \"diagonal\"\nanimation = 3\nword_lead = 900\nword_glow = 0.5\n[visualizer]\nbars = 2\nglow = \"yes\"", false).unwrap_err();
        for key in ["lyrics.align", "lyrics.animation", "lyrics.word_lead", "lyrics.word_glow", "visualizer.bars", "visualizer.glow"] {
            assert!(e.contains(key), "{key} in {e}");
        }
    }

    #[test]
    fn previews_dont_write_and_resets_clear_a_table() {
        let (mut s, path) = store("preview");
        s.set("lyrics.align", "right").unwrap();
        s.set("visualizer.style", "wave").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        s.preview("lyrics.size", "1.7");
        assert_eq!(s.theme().lyrics.size, 1.7, "shown at once");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "but not written");
        assert!(s.set("lyrics.align", "sideways").is_err());
        s.reset_table("lyrics").unwrap();
        assert_eq!(s.theme().lyrics.align, "center");
        assert_eq!(s.theme().viz.style, "wave", "other tables are kept");
    }

    #[test]
    fn derived_colors_follow_the_base_colors() {
        let (t, _) = parse("[theme]\nforeground = \"#FFFFFF\"\naccent = \"#5DA9D6\"", false).unwrap();
        assert_eq!(t.text_muted.0, [255, 255, 255, 158]);
        assert_eq!(t.focus.hex(), "#5DA9D6");
        assert_eq!(t.selection.0[..3], [0x5D, 0xA9, 0xD6]);
        let (t, _) = parse("[colors]\ntext_muted = \"#11223344\"", false).unwrap();
        assert_eq!(t.text_muted.0, [0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn invalid_values_are_all_reported() {
        let e = parse("[theme]\naccent = \"tomato\"\nopacity = 1.5\nblur = 2.5\n[font]\nsize = \"big\"", false).unwrap_err();
        assert!(e.contains("theme.accent"), "{e}");
        assert!(e.contains("theme.opacity: 1.5 is out of range"), "{e}");
        assert!(e.contains("theme.blur"), "{e}");
        assert!(e.contains("font.size"), "{e}");
        assert!(parse("[theme\nbroken", false).unwrap_err().starts_with("not valid TOML"));
    }

    #[test]
    fn unknown_keys_are_warnings() {
        let (_, w) = parse("[theme]\naccnet = \"#fff\"", false).unwrap();
        assert_eq!(w, vec!["unknown key theme.accnet (ignored)".to_string()]);
    }

    #[test]
    fn motion_follows_the_system_unless_set() {
        assert!(parse("", true).unwrap().0.reduce_motion);
        assert!(!parse("[motion]\nreduce = false", true).unwrap().0.reduce_motion);
        assert!(parse("[motion]\nreduce = \"system\"", true).unwrap().0.reduce_motion);
        assert!(parse("[motion]\nreduce = \"sometimes\"", true).is_err());
    }

    #[test]
    fn missing_file_means_defaults() {
        let (s, path) = store("missing");
        assert!(!path.exists());
        assert_eq!(s.theme(), &Theme::default());
        assert!(s.error().is_none());
    }

    #[test]
    fn reload_picks_up_edits_and_keeps_the_last_good_theme() {
        let (mut s, path) = store("reload");
        write(&path, "[theme]\naccent = \"#5DA9D6\"\n");
        assert!(s.reload());
        assert_eq!(s.theme().accent.hex(), "#5DA9D6");
        let g = s.generation();

        write(&path, "[theme]\naccent = \"nope\"\n");
        assert!(!s.reload());
        assert_eq!(s.theme().accent.hex(), "#5DA9D6", "last valid theme stays");
        assert!(s.error().unwrap().contains("theme.accent"));
        assert_eq!(s.generation(), g);

        write(&path, "[theme]\naccent = \"#A87C9F\"\n");
        assert!(s.reload());
        assert!(s.error().is_none());
        assert_eq!(s.generation(), g + 1);
    }

    #[test]
    fn poll_notices_changes_on_disk() {
        let (mut s, path) = store("poll");
        write(&path, "[theme]\nblur = 10\n");
        s.checked = Instant::now() - POLL;
        assert!(s.poll());
        assert_eq!(s.theme().blur, 10);
        s.checked = Instant::now() - POLL;
        assert!(!s.poll(), "unchanged file isn't re-applied");
    }

    #[test]
    fn a_file_emptied_mid_save_is_waited_out() {
        let (mut s, path) = store("empty");
        write(&path, "[theme]\nblur = 10\n");
        s.reload();
        write(&path, "");
        s.checked = Instant::now() - POLL;
        assert!(!s.poll(), "an empty file is given one more check");
        assert_eq!(s.theme().blur, 10);
        s.checked = Instant::now() - POLL;
        assert!(s.poll(), "still empty: now it's the defaults");
        assert_eq!(s.theme().blur, 28);
    }

    #[test]
    fn whole_number_keys_refuse_fractions() {
        let Err(e) = parse("[theme]\nblur = 12.5\n[visualizer]\nbars = 40.0\n", false) else { panic!("fractions accepted") };
        assert!(e.contains("theme.blur: expected a whole number") && e.contains("visualizer.bars: expected a whole number"), "{e}");
    }

    #[test]
    fn set_keeps_comments_and_other_keys() {
        let (mut s, path) = store("set");
        write(&path, "# mine\n[theme]\naccent = \"#5DA9D6\" # my blue\nopacity = 0.5\n\n[custom]\nkeep = 1\n");
        s.reload();
        s.set("theme.accent", "#a87c9f").unwrap();
        s.set("theme.blur", "40").unwrap();
        s.set("font.size", "14").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine"), "{text}");
        assert!(text.contains("accent = \"#A87C9F\" # my blue"), "{text}");
        assert!(text.contains("opacity = 0.5"), "{text}");
        assert!(text.contains("[custom]\nkeep = 1"), "{text}");
        assert_eq!(s.theme().accent.hex(), "#A87C9F");
        assert_eq!((s.theme().blur, s.theme().font_size), (40, 14.0));
    }

    #[test]
    fn set_rejects_bad_values_without_writing() {
        let (mut s, path) = store("reject");
        write(&path, "[theme]\nopacity = 0.5\n");
        s.reload();
        assert!(s.set("theme.opacity", "2").is_err());
        assert!(s.set("theme.accent", "red").is_err());
        assert!(s.set("theme.nope", "1").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[theme]\nopacity = 0.5\n");
        // A broken file is never overwritten by the panel.
        write(&path, "[theme\n");
        assert!(s.set("theme.blur", "3").unwrap_err().contains("fix theme.toml first"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[theme\n");
    }

    #[test]
    fn first_edit_creates_the_commented_file() {
        let (mut s, path) = store("create");
        s.set("theme.opacity", "0.8").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Glass theme"));
        assert!(text.contains("opacity    = 0.8") || text.contains("opacity = 0.8"), "{text}");
        assert!(text.contains("# how solid the background is"), "comment kept: {text}");
    }

    #[test]
    fn reset_restores_only_the_panel_values() {
        let (mut s, path) = store("reset");
        write(&path, "[theme]\naccent = \"#5DA9D6\"\nopacity = 0.2\n[font]\nsize = 15\n");
        s.reload();
        s.reset_panel_values().unwrap();
        assert_eq!(s.theme().accent.hex(), "#4F83A3");
        assert_eq!(s.theme().opacity, 0.45);
        assert_eq!(s.theme().font_size, 15.0, "other sections untouched");
        assert!(std::fs::read_to_string(&path).unwrap().contains("size = 15"));
    }

    #[test]
    #[cfg(not(glass_mobile))] // iOS keeps it in the app's own folder
    fn xdg_config_home_is_respected() {
        // Only absolute paths count, per the XDG spec.
        let saved = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-test") };
        assert_eq!(config_dir(), PathBuf::from("/tmp/xdg-test/glass"));
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "relative") };
        assert!(config_dir().ends_with(".config/glass"));
        match saved {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
    }

    #[test]
    fn fonts_resolve_or_fall_back() {
        let none = font_files("No Such Font Family 123");
        assert_eq!(none, FontFiles::default());
        if std::path::Path::new("/System/Library/Fonts/Menlo.ttc").exists() {
            let m = font_files("Menlo");
            assert_eq!(m.bold.as_ref().map(|b| b.1), Some(1));
        }
    }
}
