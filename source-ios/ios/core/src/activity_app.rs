//! The Activity page's data (desktop pages.rs `activity`): what Glass keeps on this
//! device and the requests it made. Asked for while the page is open; sizes are
//! measured off the core's thread.

use crate::app::MobileApp;
use crate::types::{LogEntryModel, StoredFile};

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct ActivityModel {
    pub files: Vec<StoredFile>,
    /// "Encrypted audio files · 120.4 MB of 2048 MB"
    pub audio_cache: String,
    pub lyrics_cache: String,
    pub folders: Vec<String>,
    pub engine_status: String,
    /// Oldest first (the page shows the newest on top).
    pub log: Vec<LogEntryModel>,
}

pub fn fmt_bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

impl MobileApp {
    /// Builds the page's data and answers `reply` once the sizes are measured.
    pub fn activity(&mut self, reply: tokio::sync::oneshot::Sender<ActivityModel>) {
        let cfg = crate::config::config_dir();
        let edited = crate::theme_config::config_dir();
        let cache = crate::config::cache_dir();
        let files: Vec<(&str, std::path::PathBuf, &str)> = vec![
            ("settings.json", cfg.join("settings.json"), "Your preferences, folders and pins"),
            ("device_id", cfg.join("device_id"), "This device's Spotify Connect ID"),
            ("data_device_id", cfg.join("data_device_id"), "The ID of Glass's connection for your library and pages"),
            ("queries.json", cfg.join("queries.json"), "Spotify's current page queries, read from Spotify's web player when the built-in ones go stale"),
            ("downloads.json", cfg.join("downloads.json"), "What you downloaded"),
            ("local_playlists.json", cfg.join("local_playlists.json"), "Your local playlists"),
            ("lyrics_choices.json", cfg.join("lyrics_choices.json"), "Lyrics you picked, and timing you nudged"),
            ("theme.toml", edited.join("theme.toml"), "Your theme"),
            ("config.toml", edited.join("config.toml"), "Lyrics and assistant settings"),
            ("assistant", cfg.join("assistant"), "The ChatGPT app registration (its sign-in and any API key are in the Keychain)"),
        ];
        let log: Vec<LogEntryModel> = self
            .log
            .snapshot()
            .iter()
            .map(|e| LogEntryModel { time: crate::model::clock(e.at), method: e.method.clone(), path: e.path.clone(), status: e.status.clone(), millis: e.millis as u64 })
            .collect();
        let (engine_status, limit) = (self.engine_status.clone(), self.settings.cache_limit_mb);
        let lyrics = self.lyrics.cache.clone();
        let mut folders = vec![cfg.display().to_string()];
        if edited != cfg {
            folders.push(edited.display().to_string());
        }
        folders.push(cache.display().to_string());
        folders.push(crate::paths::documents().display().to_string());
        std::thread::spawn(move || {
            let size = |p: &std::path::Path| match std::fs::metadata(p) {
                Ok(m) if m.is_dir() => Some(crate::settings_app::dir_size(p)),
                Ok(m) => Some(m.len()),
                Err(_) => None,
            };
            let files = files
                .into_iter()
                .map(|(label, path, note)| StoredFile { label: label.into(), size: size(&path).map(fmt_bytes).unwrap_or_else(|| "not present".into()), path: path.display().to_string(), note: note.into() })
                .collect();
            let audio = crate::settings_app::dir_size(&crate::engine::audio_cache_dir());
            let (n, bytes) = lyrics.usage();
            let _ = reply.send(ActivityModel {
                files,
                audio_cache: format!("Encrypted audio files · {} of {limit} MB", fmt_bytes(audio)),
                lyrics_cache: format!("LRCLIB answers and saved translations · {} songs · {}", n, fmt_bytes(bytes)),
                folders,
                engine_status,
                log,
            });
        });
    }
}
