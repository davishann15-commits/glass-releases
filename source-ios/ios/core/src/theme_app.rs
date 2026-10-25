//! The theme (theme.toml) as the app needs it: every token the desktop reads
//! (src/theme_config.rs), published whenever the file or an edit changes it,
//! with the file's error and warnings. Edits go through the same `ThemeStore`
//! (validated, comments and unknown keys kept, last good theme on errors).

use crate::app::MobileApp;
use crate::bridge::CoreEvent;
use crate::theme_config::Rgba;

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LyricsStyleModel {
    pub current: String,
    pub other: String,
    pub highlight: String,
    pub size: f32,
    /// left, center, right
    pub align: String,
    /// top, upper, middle
    pub position: String,
    pub spacing: f32,
    pub fade: f32,
    /// glide, fade, pop, none
    pub animation: String,
    pub speed: f32,
    pub bold: bool,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct VizStyleModel {
    /// bars, mirror, wave
    pub style: String,
    pub color: String,
    pub color2: String,
    pub opacity: f32,
    pub bars: u32,
    pub height: f32,
    pub sensitivity: f32,
    pub smoothing: f32,
    pub glow: bool,
}

/// Every theme.toml value, colors as `#RRGGBB[AA]`.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct ThemeModel {
    pub background: String,
    pub foreground: String,
    pub accent: String,
    pub border: String,
    pub opacity: f32,
    pub blur: u32,
    pub surface: String,
    pub surface_raised: String,
    pub overlay: String,
    pub text_secondary: String,
    pub text_muted: String,
    pub on_accent: String,
    pub selection: String,
    pub hover: String,
    pub focus: String,
    pub player_background: String,
    pub player_progress: String,
    pub player_track: String,
    pub font_family: String,
    pub font_size: f32,
    pub lyrics_family: String,
    pub radius: f32,
    pub spacing: f32,
    pub palette: Vec<String>,
    /// [motion] reduce, resolved (the system's setting unless the file says).
    pub reduce_motion: bool,
    /// [motion] reduce as written: "system", "true" or "false".
    pub motion_reduce: String,
    pub lyrics: LyricsStyleModel,
    pub visualizer: VizStyleModel,
    /// Not applied: the reason (the last good theme stays).
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub path: String,
}

fn hex(c: &Rgba) -> String {
    c.hex()
}

impl MobileApp {
    pub fn theme_model(&self) -> ThemeModel {
        let t = self.theme.theme();
        ThemeModel {
            background: hex(&t.background),
            foreground: hex(&t.foreground),
            accent: hex(&t.accent),
            border: hex(&t.border),
            opacity: t.opacity,
            blur: t.blur,
            surface: hex(&t.surface),
            surface_raised: hex(&t.surface_raised),
            overlay: hex(&t.overlay),
            text_secondary: hex(&t.text_secondary),
            text_muted: hex(&t.text_muted),
            on_accent: hex(&t.on_accent),
            selection: hex(&t.selection),
            hover: hex(&t.hover),
            focus: hex(&t.focus),
            player_background: hex(&t.player_background),
            player_progress: hex(&t.player_progress),
            player_track: hex(&t.player_track),
            font_family: t.font_family.clone(),
            font_size: t.font_size,
            lyrics_family: t.lyrics_family.clone(),
            radius: t.radius,
            spacing: t.spacing,
            palette: t.palette.iter().map(hex).collect(),
            reduce_motion: t.reduce_motion,
            motion_reduce: self.theme.written("motion.reduce").unwrap_or_else(|| "system".into()),
            lyrics: LyricsStyleModel {
                current: hex(&t.lyrics.current),
                other: hex(&t.lyrics.other),
                highlight: hex(&t.lyrics.highlight),
                size: t.lyrics.size,
                align: t.lyrics.align.clone(),
                position: t.lyrics.position.clone(),
                spacing: t.lyrics.spacing,
                fade: t.lyrics.fade,
                animation: t.lyrics.animation.clone(),
                speed: t.lyrics.speed,
                bold: t.lyrics.bold,
            },
            visualizer: VizStyleModel {
                style: t.viz.style.clone(),
                color: hex(&t.viz.color),
                color2: hex(&t.viz.color2),
                opacity: t.viz.opacity,
                bars: t.viz.bars,
                height: t.viz.height,
                sensitivity: t.viz.sensitivity,
                smoothing: t.viz.smoothing,
                glow: t.viz.glow,
            },
            error: self.theme.error().map(str::to_string),
            warnings: self.theme.warnings().to_vec(),
            path: crate::paths::tilde(self.theme.path()),
        }
    }

    pub fn publish_theme(&mut self, events: &mut Vec<CoreEvent>) {
        let m = self.theme_model();
        if self.published.theme.as_ref() != Some(&m) {
            self.published.theme = Some(m.clone());
            events.push(CoreEvent::Theme { theme: m });
        }
    }
}


/// The hand-edited files, for export and import (iOS has no text editor to open them in).
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditableFile {
    /// theme.toml
    Theme,
    /// config.toml (lyrics and assistant settings)
    Config,
}

impl crate::app::MobileApp {
    /// The file to hand to the share sheet (written with its defaults if it isn't there yet).
    pub fn export_file(&mut self, which: EditableFile) -> Result<String, String> {
        match which {
            EditableFile::Theme => {
                self.theme.ensure_file()?;
                Ok(self.theme.path().display().to_string())
            }
            EditableFile::Config => {
                self.config.ensure_file()?;
                Ok(self.config.path().display().to_string())
            }
        }
    }

    /// Replaces a file with `text` after checking it parses; the old file is kept as
    /// `<name>.bak`. Nothing changes when it doesn't parse.
    pub fn import_file(&mut self, which: EditableFile, text: String) -> Result<String, String> {
        let path = match which {
            EditableFile::Theme => {
                crate::theme_config::parse(&text, crate::window_fx::system_reduce_motion()).map_err(|e| {
                    log::warn!("theme import: {e}");
                    "That file isn't a Glass theme, so nothing changed.".to_string()
                })?;
                self.theme.path().to_path_buf()
            }
            EditableFile::Config => {
                crate::user_config::parse(&text).map_err(|e| {
                    log::warn!("settings import: {e}");
                    "That file isn't a Glass settings file, so nothing changed.".to_string()
                })?;
                self.config.path().to_path_buf()
            }
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                log::warn!("import: {e}");
                "Couldn't import it. Try again.".to_string()
            })?;
        }
        if path.exists() {
            let _ = std::fs::copy(&path, path.with_extension("toml.bak"));
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| {
            log::warn!("import: {e}");
            "Couldn't import it. Try again.".to_string()
        })?;
        match which {
            EditableFile::Theme => {
                self.theme.reload();
                Ok("Theme imported".into())
            }
            EditableFile::Config => {
                self.config.reload();
                Ok("Settings imported".into())
            }
        }
    }
}
