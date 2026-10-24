//! Glass: a quiet, transparent Spotify client.
//!
//! The modules, roughly in the order data flows through them:
//!
//! - Spotify: `auth` (sign-in), `api` (the data client and its request log),
//!   `sp` (Spotify's internal services behind it), `engine` (librespot
//!   playback and the Spotify Connect device), `dsp` (equalizer and mono, and
//!   the audio output), `audio_out` (following the system output), `viz`
//!   (the visualizer's spectrum), `downloads` (songs kept on this computer), `dj`.
//! - Your own music: `local` (scanning and playing files), `playlists` (local
//!   playlists), `share` (sending songs between your computers), `save_song`
//!   (Share ▸ Download as M4A/MP3).
//! - Lyrics: `lyrics` (providers, matching, timing, translation) and
//!   `translate_app`.
//! - The assistant: `ai` and `assistant_app`.
//! - The app: `app` (state, actions and messages), `model` (helpers over the
//!   Web API's JSON), `config` (settings and where files go), `theme_config`
//!   and `user_config` (theme.toml and config.toml, reloaded live), `ui`
//!   (everything you see), `window_fx` (transparency and blur), `update`,
//!   `instance` (one Glass at a time), `heap` (giving freed memory back),
//!   `demo` (sample data for screenshots).

mod ai;
mod api;
mod app;
mod assistant_app;
mod audio_out;
mod auth;
mod config;
mod demo;
mod dj;
mod downloads;
mod dsp;
mod engine;
mod heap;
mod instance;
mod local;
mod lyrics;
mod model;
mod playlists;
mod save_song;
mod share;
mod sp;
mod theme_config;
mod translate_app;
mod ui;
mod update;
mod user_config;
mod viz;
mod window_fx;

use eframe::egui;

// macOS's allocator keeps freed pages and counts them against Glass: on a real
// library, 60 MB held for 23–28 MB in use. mimalloc hands them back (49 MB held).
// On Linux, glibc's is used. Both are trimmed now and then (see `heap`).
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> eframe::Result {
    let demo_home = demo::use_own_home();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    viz::install();
    let minimized = std::env::args().any(|a| a == "--minimized");
    // One Glass at a time: another build replaces the running one; the same
    // build just brings it forward.
    let instance = match instance::claim() {
        instance::Claim::Exit => return Ok(()),
        instance::Claim::Run(listener) => listener,
    };
    // The Spotify session is kept here: only this user may open the folder.
    let _ = config::make_private_dir();

    // GLASS_WINDOW=900x600 opens at that size (used to check small layouts). Otherwise the
    // size it last had (macOS restores the whole frame itself, see window_fx).
    let size = std::env::var("GLASS_WINDOW")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?])))
        .or_else(|| if cfg!(target_os = "macos") { None } else { config::Settings::load().window_size })
        .unwrap_or([1280.0, 820.0]);
    let viewport = egui::ViewportBuilder::default()
        .with_title("Glass")
        // Linux: matches glass.desktop, so docks and launchers group the window with it.
        .with_app_id("glass")
        .with_inner_size(size)
        .with_min_inner_size([900.0, 600.0])
        .with_transparent(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_fullsize_content_view(true)
        .with_visible(!minimized);
    // macOS takes the icon from the app bundle. Left unset, eframe would decode its
    // own logo (a megabyte) and put it in the Dock in place of Glass's; an empty
    // icon tells it not to.
    #[cfg(target_os = "macos")]
    let viewport = viewport.with_icon(egui::IconData::default());
    // The window icon on Linux (taskbars, Alt-Tab).
    #[cfg(not(target_os = "macos"))]
    let viewport = match image::load_from_memory_with_format(include_bytes!("../packaging/icon/png/glass-256.png"), image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (width, height) = rgba.dimensions();
            viewport.with_icon(egui::IconData { rgba: rgba.into_raw(), width, height })
        }
        Err(_) => viewport,
    };

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "Glass",
        options,
        Box::new(move |cc| {
            let app = app::App::new(cc.egui_ctx.clone());
            ui::theme::install(&cc.egui_ctx, app.theme.theme(), app.theme.generation());
            ui::images::install(&cc.egui_ctx, app.rt.handle().clone());
            // Covers embedded in local files.
            cc.egui_ctx.add_bytes_loader(std::sync::Arc::new(local::CoverLoader::new(app.rt.handle().clone())));
            #[cfg(unix)]
            if let Some(listener) = instance {
                app.answer_other_copies(listener);
            }
            #[cfg(not(unix))]
            let _ = instance;
            window_fx::install(cc, app.theme.theme());
            Ok(Box::new(app::GlassApp { app, frames: 0 }))
        }),
    );
    if let Some(home) = demo_home {
        let _ = std::fs::remove_dir_all(home);
    }
    result
}
