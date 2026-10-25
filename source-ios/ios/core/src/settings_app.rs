//! Settings for the app: settings.json (src/config.rs `Settings`) and config.toml
//! (src/user_config.rs), as the desktop's Settings page reads and writes them,
//! plus the live notes that page shows (engine, caches, downloads). Desktop-only
//! keys (updates, open at login, close button, window size) are kept untouched.

use crate::app::{MobileApp, Msg};
use crate::bridge::CoreEvent;
use crate::config::{Folder, Loudness, LocalView, Quality, Settings};

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamingQuality {
    Normal,
    High,
    VeryHigh,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VolumeLevel {
    Quiet,
    Normal,
    Loud,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocalFilesView {
    Albums,
    Songs,
    Artists,
    Genres,
    Recent,
    Playlists,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveFormat {
    M4a,
    Mp3,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct FolderModel {
    pub id: String,
    pub name: String,
    pub playlists: Vec<String>,
}

/// settings.json, as the app edits it.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct SettingsModel {
    pub client_id: String,
    pub device_name: String,
    pub streaming_quality: StreamingQuality,
    pub normalize: bool,
    pub loudness: VolumeLevel,
    pub gapless: bool,
    pub autoplay: bool,
    pub mono: bool,
    pub allow_explicit: bool,
    pub eq_enabled: bool,
    pub eq_preset: String,
    /// dB for 60 Hz, 150 Hz, 400 Hz, 1 kHz, 2.4 kHz, 15 kHz (−12…12).
    pub eq_gains: Vec<f32>,
    pub show_local_files: bool,
    pub local_dirs: Vec<String>,
    pub share_local_files: bool,
    pub local_view: LocalFilesView,
    pub save_format: SaveFormat,
    pub save_mp3_kbps: u16,
    pub save_dir: Option<String>,
    pub now_playing_on_play: bool,
    pub track_notifications: bool,
    pub album_colors: bool,
    pub visualizer: bool,
    pub dj_voice: bool,
    pub proxy: String,
    pub cache_limit_mb: u64,
    pub folders: Vec<FolderModel>,
    pub recent_searches: Vec<String>,
}

/// config.toml's lyrics and assistant settings, and the file's state.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct ConfigModel {
    pub lyrics_provider: String,
    pub lyrics_fallback: bool,
    pub lyrics_offset_ms: i64,
    pub lyrics_contact: String,
    pub translation: String,
    pub translate_to: String,
    /// The language "system" means, and its name ("English").
    pub system_language: String,
    pub translate_to_name: String,
    pub translator: String,
    pub apple_translation_available: bool,
    pub assistant_enabled: bool,
    pub assistant_billing: String,
    pub assistant_model: String,
    pub assistant_spotify: bool,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub path: String,
}

/// The live notes beside some settings.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct StatusModel {
    pub version: String,
    pub engine_status: String,
    pub engine_ready: bool,
    pub audio_cache_dir: String,
    /// The streaming cache (downloads included), bytes; None until measured.
    pub audio_cache_bytes: Option<u64>,
    pub lyrics_cached_songs: u64,
    pub lyrics_cache_bytes: u64,
    pub downloads: u64,
    pub download_bytes: u64,
    pub downloads_pending: u64,
    pub downloads_failed: u64,
    pub save_dir_effective: String,
    pub share_inbox: String,
}

/// The EQ presets (desktop config.rs `EQ_PRESETS`) and band labels, for the UI.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct EqPreset {
    pub name: String,
    pub gains: Vec<f32>,
}

#[uniffi::export]
pub fn eq_presets() -> Vec<EqPreset> {
    crate::config::EQ_PRESETS.iter().map(|(n, g)| EqPreset { name: n.to_string(), gains: g.to_vec() }).collect()
}

#[uniffi::export]
pub fn eq_bands() -> Vec<String> {
    crate::config::EQ_BANDS.iter().map(|(_, l)| l.to_string()).collect()
}

#[uniffi::export]
pub fn quality_label(q: StreamingQuality) -> String {
    to_quality(q).label().to_string()
}

fn to_quality(q: StreamingQuality) -> Quality {
    match q {
        StreamingQuality::Normal => Quality::Normal,
        StreamingQuality::High => Quality::High,
        StreamingQuality::VeryHigh => Quality::VeryHigh,
    }
}

pub fn settings_model(s: &Settings) -> SettingsModel {
    SettingsModel {
        client_id: s.client_id.clone(),
        device_name: s.device_name.clone(),
        streaming_quality: match s.streaming_quality {
            Quality::Normal => StreamingQuality::Normal,
            Quality::High => StreamingQuality::High,
            Quality::VeryHigh => StreamingQuality::VeryHigh,
        },
        normalize: s.normalize,
        loudness: match s.loudness {
            Loudness::Quiet => VolumeLevel::Quiet,
            Loudness::Normal => VolumeLevel::Normal,
            Loudness::Loud => VolumeLevel::Loud,
        },
        gapless: s.gapless,
        autoplay: s.autoplay,
        mono: s.mono,
        allow_explicit: s.allow_explicit,
        eq_enabled: s.eq_enabled,
        eq_preset: s.eq_preset.clone(),
        eq_gains: s.eq_gains.to_vec(),
        show_local_files: s.show_local_files,
        local_dirs: s.local_dirs.iter().map(|d| d.to_string_lossy().into_owned()).collect(),
        share_local_files: s.share_local_files,
        local_view: match s.local_view {
            LocalView::Albums => LocalFilesView::Albums,
            LocalView::Songs => LocalFilesView::Songs,
            LocalView::Artists => LocalFilesView::Artists,
            LocalView::Genres => LocalFilesView::Genres,
            LocalView::Recent => LocalFilesView::Recent,
            LocalView::Playlists => LocalFilesView::Playlists,
        },
        save_format: match s.save_format {
            crate::save_song::Format::M4a => SaveFormat::M4a,
            crate::save_song::Format::Mp3 => SaveFormat::Mp3,
        },
        save_mp3_kbps: s.save_mp3_kbps,
        save_dir: s.save_dir.as_ref().map(|d| d.to_string_lossy().into_owned()),
        now_playing_on_play: s.now_playing_on_play,
        track_notifications: s.track_notifications,
        album_colors: s.album_colors,
        visualizer: s.visualizer,
        dj_voice: s.dj_voice,
        proxy: s.proxy.clone(),
        cache_limit_mb: s.cache_limit_mb,
        folders: s.folders.iter().map(|f| FolderModel { id: f.id.clone(), name: f.name.clone(), playlists: f.playlists.clone() }).collect(),
        recent_searches: s.recent_searches.clone(),
    }
}

/// The app's edit applied over the current settings (desktop-only keys untouched),
/// brought back into range as `Settings::load` does.
pub fn apply_model(base: &Settings, m: SettingsModel) -> Settings {
    let mut s = base.clone();
    s.client_id = if m.client_id.trim().is_empty() { crate::config::DEFAULT_CLIENT_ID.into() } else { m.client_id.trim().into() };
    s.device_name = if m.device_name.trim().is_empty() { base.device_name.clone() } else { m.device_name.trim().into() };
    s.streaming_quality = to_quality(m.streaming_quality);
    s.normalize = m.normalize;
    s.loudness = match m.loudness {
        VolumeLevel::Quiet => Loudness::Quiet,
        VolumeLevel::Normal => Loudness::Normal,
        VolumeLevel::Loud => Loudness::Loud,
    };
    s.gapless = m.gapless;
    s.autoplay = m.autoplay;
    s.mono = m.mono;
    s.allow_explicit = m.allow_explicit;
    s.eq_enabled = m.eq_enabled;
    s.eq_preset = m.eq_preset;
    for (i, g) in m.eq_gains.iter().take(6).enumerate() {
        s.eq_gains[i] = if g.is_finite() { g.clamp(-12.0, 12.0) } else { 0.0 };
    }
    s.show_local_files = m.show_local_files;
    s.local_dirs = m.local_dirs.into_iter().map(Into::into).collect();
    s.share_local_files = m.share_local_files;
    s.local_view = match m.local_view {
        LocalFilesView::Albums => LocalView::Albums,
        LocalFilesView::Songs => LocalView::Songs,
        LocalFilesView::Artists => LocalView::Artists,
        LocalFilesView::Genres => LocalView::Genres,
        LocalFilesView::Recent => LocalView::Recent,
        LocalFilesView::Playlists => LocalView::Playlists,
    };
    s.save_format = match m.save_format {
        SaveFormat::M4a => crate::save_song::Format::M4a,
        SaveFormat::Mp3 => crate::save_song::Format::Mp3,
    };
    s.save_mp3_kbps = if crate::save_song::BITRATES.contains(&m.save_mp3_kbps) { m.save_mp3_kbps } else { 320 };
    s.save_dir = m.save_dir.filter(|d| !d.is_empty()).map(Into::into);
    s.now_playing_on_play = m.now_playing_on_play;
    s.track_notifications = m.track_notifications;
    s.album_colors = m.album_colors;
    s.visualizer = m.visualizer;
    s.dj_voice = m.dj_voice;
    s.proxy = m.proxy.trim().to_string();
    s.cache_limit_mb = m.cache_limit_mb.clamp(256, 50_000);
    s.folders = m.folders.into_iter().map(|f| Folder { id: f.id, name: f.name, playlists: f.playlists }).collect();
    s.recent_searches = m.recent_searches.into_iter().take(10).collect();
    s
}

/// Sizes measured off the actor thread now and then (desktop Activity/Settings `sizes`).
#[derive(Default)]
pub struct Measured {
    pub audio_cache: Option<u64>,
    pub lyrics: (u64, u64),
    pub at: Option<std::time::Instant>,
    pub running: bool,
}

pub fn dir_size(p: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten().map(|e| if e.file_type().is_ok_and(|t| t.is_dir()) { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()
}

impl MobileApp {
    pub fn config_model(&self) -> ConfigModel {
        let c = self.config.lyrics();
        let a = self.config.assistant();
        let target = crate::lyrics::translate::target(&c.translate_to);
        ConfigModel {
            lyrics_provider: c.provider.name().into(),
            lyrics_fallback: c.fallback,
            lyrics_offset_ms: c.offset_ms,
            lyrics_contact: c.contact.clone(),
            translation: c.translation.name().into(),
            translate_to: c.translate_to.clone(),
            system_language: crate::lyrics::translate::system_language(),
            translate_to_name: crate::lyrics::translate::language_name(&target),
            translator: c.translator.name().into(),
            apple_translation_available: crate::lyrics::translate::apple_available(),
            assistant_enabled: a.enabled,
            assistant_billing: a.billing.clone(),
            assistant_model: a.model.clone(),
            assistant_spotify: a.spotify,
            error: self.config.error().map(str::to_string),
            warnings: self.config.warnings().to_vec(),
            path: crate::paths::tilde(self.config.path()),
        }
    }

    /// Kicks off a measurement of the caches every 10 s at most, while Settings is open.
    pub fn measure_sizes(&mut self) {
        if !self.settings_open || self.measured.running || self.measured.at.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(10)) {
            return;
        }
        self.measured.running = true;
        let (tx, cache) = (self.tx.clone(), self.lyrics.cache.clone());
        std::thread::spawn(move || {
            let audio = dir_size(&crate::engine::audio_cache_dir());
            let (n, bytes) = cache.usage();
            tx.send(Msg::Measured { audio, lyrics: (n as u64, bytes) });
        });
    }

    pub fn status_model(&self) -> StatusModel {
        let (pending, failed) = self.downloads.pending();
        StatusModel {
            version: env!("CARGO_PKG_VERSION").into(),
            engine_status: self.engine_status.clone(),
            engine_ready: self.engine.is_some(),
            audio_cache_dir: crate::engine::audio_cache_dir().display().to_string(),
            audio_cache_bytes: self.measured.audio_cache,
            lyrics_cached_songs: self.measured.lyrics.0,
            lyrics_cache_bytes: self.measured.lyrics.1,
            downloads: self.downloads.count() as u64,
            download_bytes: self.downloads.total_bytes(),
            downloads_pending: pending as u64,
            downloads_failed: failed as u64,
            save_dir_effective: self.settings.save_dir.clone().unwrap_or_else(crate::save_song::default_dir).display().to_string(),
            share_inbox: crate::share::inbox().display().to_string(),
        }
    }

    pub fn publish_settings(&mut self, events: &mut Vec<CoreEvent>) {
        let settings = settings_model(&self.settings);
        let config = self.config_model();
        let status = self.status_model();
        let changed = self.published.settings.as_ref() != Some(&settings) || self.published.config.as_ref() != Some(&config) || self.published.status.as_ref() != Some(&status);
        if changed {
            self.published.settings = Some(settings.clone());
            self.published.config = Some(config.clone());
            self.published.status = Some(status.clone());
            events.push(CoreEvent::Settings { settings, config, status });
        }
    }
}
