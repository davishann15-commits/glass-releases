//! Glass's shared Rust core for iOS.
//!
//! The service modules are the desktop app's own files, included by path
//! (`#[path = "../../../src/…"]`), so both apps build from one copy of the
//! Spotify, playback, lyrics, local-music, sharing and assistant code. The
//! few desktop-only pieces inside them are behind `#[cfg(not(glass_mobile))]`
//! (see build.rs). Everything that replaces the desktop's egui `App` lives in
//! this crate's own modules.

#![allow(dead_code, unused_imports)]

#[path = "../../../src/ai/mod.rs"]
mod ai;
#[path = "../../../src/api.rs"]
mod api;
#[path = "../../../src/audio_out.rs"]
mod audio_out;
#[path = "../../../src/auth.rs"]
mod auth;
#[path = "../../../src/config.rs"]
mod config;
#[path = "../../../src/dj.rs"]
mod dj;
#[path = "../../../src/downloads.rs"]
mod downloads;
#[path = "../../../src/dsp.rs"]
mod dsp;
#[path = "../../../src/engine.rs"]
mod engine;
#[path = "../../../src/local.rs"]
mod local;
#[path = "../../../src/local_index.rs"]
mod local_index;
#[path = "../../../src/lyrics/mod.rs"]
mod lyrics;
#[path = "../../../src/model.rs"]
mod model;
#[path = "../../../src/palette.rs"]
mod palette;
#[path = "../../../src/playlists.rs"]
mod playlists;
#[path = "../../../src/save_song/mod.rs"]
mod save_song;
#[path = "../../../src/share.rs"]
mod share;
#[path = "../../../src/sp/mod.rs"]
mod sp;
#[path = "../../../src/theme_config.rs"]
mod theme_config;
#[path = "../../../src/user_config.rs"]
mod user_config;
#[path = "../../../src/viz.rs"]
mod viz;

mod activity_app;
mod app;
mod apple_music;
mod assistant_app;
mod bonjour;
mod bridge;
mod colors_app;
mod demo;
mod errors;
mod local_app;
mod lyrics_app;
mod paths;
mod platform;
#[cfg(debug_assertions)]
mod probe;
mod publish;
mod save_app;
mod settings_app;
mod share_app;
mod signin;
mod theme_app;
mod types;
mod views;
mod window_fx;

uniffi::setup_scaffolding!();
