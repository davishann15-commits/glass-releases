//! Preferences (`settings.json`) and where Glass keeps its files.
//!
//! `config_dir()` (`~/Library/Application Support/Glass` on macOS,
//! `~/.config/glass` on Linux, the folder theme.toml is in) holds
//! `settings.json`, the Spotify session (`session.json`, `credentials.json`),
//! the list of downloads, local playlists, lyrics picks and the sharing key. `cache_dir()` holds the audio
//! (downloads included), saved lyrics and translations, artwork and updates. The
//! hand-edited `theme.toml` and `config.toml` live in `~/.config/glass`
//! (see `theme_config`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Written into settings.json; see `Settings::upgrade`.
const SETTINGS_VERSION: u32 = 2;

/// Spotify's desktop client ID, the same one librespot uses by default.
/// You can replace it with your own app's ID in settings.json.
pub const DEFAULT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "Glass").map(|d| d.config_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from(".glass"))
}

pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "Glass").map(|d| d.cache_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from(".glass-cache"))
}

/// The proxy setting as a URL: `None` when it's empty, an error when it doesn't
/// parse (rather than quietly connecting without it).
pub fn proxy_url(setting: &str) -> anyhow::Result<Option<url::Url>> {
    let s = setting.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse().map(Some).map_err(|_| anyhow::anyhow!("the proxy address “{s}” isn't valid (Settings → Proxy)"))
}

/// The licenses of the open-source software in Glass: in the app bundle on macOS,
/// /usr/share/licenses on Linux, or (debug builds) the repository's copy.
pub fn notices_file() -> Option<PathBuf> {
    const NAME: &str = "THIRD-PARTY-NOTICES.txt";
    let exe = std::env::current_exe().ok();
    let bundled = exe.as_ref().and_then(|e| e.parent()).map(|d| d.join("../Resources").join(NAME));
    // Only in debug builds: the path names the developer's folders.
    #[cfg(debug_assertions)]
    let repo = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packaging").join(NAME));
    #[cfg(not(debug_assertions))]
    let repo = None;
    [bundled, Some(PathBuf::from("/usr/share/licenses/glass").join(NAME)), repo].into_iter().flatten().find(|p| p.is_file())
}

/// The proxy for Glass's own connections, as Settings had it at startup.
static PROXY: std::sync::OnceLock<Option<reqwest::Proxy>> = std::sync::OnceLock::new();

/// Called once at startup (the proxy setting applies after a restart). An address
/// that isn't valid is logged and left out: those connections go direct.
pub fn use_proxy(setting: &str) {
    let proxy = match proxy_url(setting) {
        Ok(url) => url.and_then(|u| reqwest::Proxy::all(u.as_str()).ok()),
        Err(e) => {
            log::warn!("{e}");
            None
        }
    };
    let _ = PROXY.set(proxy);
}

/// A client builder that goes through the proxy, if one is set: for artwork, updates,
/// saving songs, the assistant and the DJ. The Spotify session and the lyrics client
/// are given the proxy themselves; signing in to Spotify (librespot's own client)
/// doesn't use it.
pub fn http() -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder();
    match PROXY.get().cloned().flatten() {
        Some(p) => b.proxy(p),
        None => b,
    }
}

/// Creates `config_dir()` if needed, open to this user only: the Spotify session lives in it.
pub fn make_private_dir() -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes beside `path` and renames over it, so a crash or a full disk never
/// leaves half a file.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Writes a file only this user can read (made that way, never readable by
/// others even for a moment), renamed into place so a crash can't leave half of it.
pub fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if path.starts_with(config_dir()) {
        make_private_dir()?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    // A new file each time: one left over (or put there) keeps its own permissions.
    let _ = std::fs::remove_file(&tmp);
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    open.open(&tmp)?.write_all(bytes)?;
    std::fs::rename(&tmp, path)
}

/// Reads one of Glass's JSON files. A missing file gives the default; one that
/// can't be read is kept aside (see `set_aside`) rather than overwritten by the
/// next save.
pub fn read_json<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            set_aside(path, &e);
            T::default()
        }),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("couldn't read {}: {e}", path.display());
            }
            T::default()
        }
    }
}

/// Moves a file Glass couldn't read out of the way (to `name.bad`), so saving
/// the defaults doesn't destroy what was in it.
pub fn set_aside(path: &std::path::Path, why: &dyn std::fmt::Display) {
    let mut bad = path.as_os_str().to_owned();
    bad.push(".bad");
    log::warn!("{} couldn't be read ({why}); it's kept as {}", path.display(), std::path::Path::new(&bad).display());
    let _ = std::fs::rename(path, &bad);
}

/// Where a test keeps its files: `name` in a folder of this test run's own,
/// which is removed when the run ends (passed or not).
#[cfg(test)]
pub fn test_dir(name: &str) -> PathBuf {
    static RUN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    extern "C" fn tidy() {
        if let Some(dir) = RUN.get() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    let run = RUN.get_or_init(|| {
        unsafe extern "C" {
            fn atexit(f: extern "C" fn()) -> std::ffi::c_int;
        }
        // SAFETY: registers a plain function for the C library to call as the process exits.
        unsafe { atexit(tidy) };
        let dir = std::env::temp_dir().join(format!("glass-tests-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    });
    run.join(name)
}

/// How Local Files is shown (as in iTunes / Music).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocalView {
    Albums,
    Songs,
    Artists,
    Genres,
    Recent,
    Playlists,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    /// "Low" was also 96 kbps.
    #[serde(alias = "Low")]
    Normal,
    High,
    VeryHigh,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Normal, Quality::High, Quality::VeryHigh];
    /// librespot offers 96, 160 and 320 kbps Ogg Vorbis.
    pub fn label(self) -> &'static str {
        match self {
            Quality::Normal => "Normal (96 kbps)",
            Quality::High => "High (160 kbps)",
            Quality::VeryHigh => "Very high (320 kbps)",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loudness {
    Quiet,
    Normal,
    Loud,
}

impl Loudness {
    /// Pre-gain applied on top of librespot's normalization.
    pub fn pregain_db(self) -> f64 {
        match self {
            Loudness::Quiet => -9.0,
            Loudness::Normal => 0.0,
            Loudness::Loud => 3.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Autostart {
    Off,
    Normal,
    Minimized,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibraryView {
    List,
    Compact,
    Grid,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibrarySort {
    Recents,
    RecentlyAdded,
    Alphabetical,
    Creator,
}

/// A playlist folder kept by Glass. Spotify's own folders aren't read or changed:
/// the rootlist's folder entries are skipped (see `sp::library`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub playlists: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    /// Which version of these settings the file was written by (see `upgrade`);
    /// 0 in a file from before there was one.
    #[serde(default)]
    pub version: u32,

    // Account
    pub client_id: String,
    pub device_name: String,

    // Audio quality
    pub streaming_quality: Quality,

    // Playback
    pub normalize: bool,
    pub loudness: Loudness,
    pub gapless: bool,
    pub autoplay: bool,
    pub mono: bool,
    pub allow_explicit: bool,

    // Equalizer: 6 bands, gains in dB (-12..=12)
    pub eq_enabled: bool,
    pub eq_preset: String,
    pub eq_gains: [f32; 6],

    // Library
    pub library_view: LibraryView,
    pub library_sort: LibrarySort,
    pub show_local_files: bool,
    pub local_dirs: Vec<PathBuf>,
    /// Local files with Glass on your other devices on this network (see `share`).
    pub share_local_files: bool,
    pub local_view: LocalView,
    /// Share ▸ "Download as …" (see `save_song`).
    pub save_format: crate::save_song::Format,
    pub save_mp3_kbps: u16,
    /// None: `~/Music/Glass`.
    pub save_dir: Option<PathBuf>,
    pub pins: Vec<String>,
    pub folders: Vec<Folder>,

    // Display
    /// Go to the now-playing screen when playback starts. (Renamed from
    /// `show_npv_on_play`, so the old default doesn't carry over.)
    pub now_playing_on_play: bool,
    pub track_notifications: bool,
    /// Now Playing and the player bar take their colors from the album cover.
    pub album_colors: bool,
    /// The audio visualizer under the lyrics on the now-playing screen (V).
    pub visualizer: bool,
    /// The DJ (X) talks between songs.
    pub dj_voice: bool,
    /// Download new versions in the background and install them on quit.
    pub auto_update: bool,

    // Startup and window
    pub autostart: Autostart,
    pub close_minimizes: bool,
    /// The window's size when it was last closed (Linux; macOS keeps the whole frame itself).
    pub window_size: Option<[f32; 2]>,

    // Network
    pub proxy: String,
    pub cache_limit_mb: u64,

    // Layout
    /// The side panel is open (for this run only: Glass always opens without it).
    #[serde(skip)]
    pub right_open: bool,
    pub volume: f32,

    // Search
    pub recent_searches: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        #[cfg(not(glass_mobile))]
        let (music, downloads) =
            (directories::UserDirs::new().and_then(|d| d.audio_dir().map(|p| p.to_path_buf())), directories::UserDirs::new().and_then(|d| d.download_dir().map(|p| p.to_path_buf())));
        // iOS: the app's own Documents folder (music imported, saved or received goes there).
        #[cfg(glass_mobile)]
        let (music, downloads): (Option<PathBuf>, Option<PathBuf>) = (Some(crate::paths::documents()), None);
        Self {
            version: SETTINGS_VERSION,
            client_id: DEFAULT_CLIENT_ID.into(),
            device_name: "Glass".into(),
            streaming_quality: Quality::High,
            normalize: true,
            loudness: Loudness::Normal,
            gapless: true,
            autoplay: true,
            mono: false,
            allow_explicit: true,
            eq_enabled: true,
            eq_preset: "Flat".into(),
            eq_gains: [0.0; 6],
            library_view: LibraryView::List,
            library_sort: LibrarySort::Recents,
            show_local_files: true,
            local_dirs: [music, downloads].into_iter().flatten().collect(),
            share_local_files: true,
            local_view: LocalView::Recent,
            save_format: crate::save_song::Format::M4a,
            save_mp3_kbps: 320,
            save_dir: None,
            pins: Vec::new(),
            folders: Vec::new(),
            now_playing_on_play: false,
            visualizer: true,
            dj_voice: true,
            auto_update: true,
            track_notifications: false,
            album_colors: true,
            autostart: Autostart::Off,
            close_minimizes: false,
            window_size: None,
            proxy: String::new(),
            cache_limit_mb: 2048,
            right_open: true,
            volume: 0.7,
            recent_searches: Vec::new(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("couldn't read {}: {e}", path.display());
                }
                return Self::default();
            }
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(mut s) => {
                s.upgrade();
                s.sanitize();
                s
            }
            Err(e) => {
                set_aside(&path, &e);
                Self::default()
            }
        }
    }

    /// Changes a settings file from an older Glass needs once.
    fn upgrade(&mut self) {
        // Version 2: sharing with your other devices needs nothing set up any more
        // (no pairing code), and is on, so they find each other by themselves. It
        // was opt-in from 1.0.0-beta.6 (version 1) while it needed pairing.
        if self.version < 2 {
            self.share_local_files = true;
        }
        self.version = SETTINGS_VERSION;
    }

    /// Brings hand-edited values back into range.
    fn sanitize(&mut self) {
        for g in &mut self.eq_gains {
            *g = if g.is_finite() { g.clamp(-12.0, 12.0) } else { 0.0 };
        }
        self.volume = if self.volume.is_finite() { self.volume.clamp(0.0, 1.0) } else { 0.7 };
        self.cache_limit_mb = self.cache_limit_mb.clamp(256, 50_000);
        self.window_size = self.window_size.filter(|[w, h]| w.is_finite() && h.is_finite() && *w >= 900.0 && *h >= 600.0 && *w <= 10_000.0 && *h <= 10_000.0);
        if !crate::save_song::BITRATES.contains(&self.save_mp3_kbps) {
            self.save_mp3_kbps = 320;
        }
        // A temporary folder, added by a dropped file before those were kept (it empties itself).
        self.local_dirs.retain(|d| !crate::local::is_temporary(d));
    }

    pub fn save(&self) {
        let Ok(text) = serde_json::to_string_pretty(self) else { return };
        if let Err(e) = write_atomic(&Self::path(), text.as_bytes()) {
            log::warn!("couldn't save settings: {e}");
        }
    }
}

/// Spotify's desktop equalizer has six filters (observed in the native binary
/// as low-shelf, low-peak, low-mid-peak, high-mid-peak, high-peak, high-shelf).
/// The center frequencies are the ones its UI labels.
pub const EQ_BANDS: [(f32, &str); 6] = [(60.0, "60Hz"), (150.0, "150Hz"), (400.0, "400Hz"), (1000.0, "1KHz"), (2400.0, "2.4KHz"), (15000.0, "15KHz")];

/// Preset names match Spotify's; the gain curves are Glass's own approximations.
pub const EQ_PRESETS: &[(&str, [f32; 6])] = &[
    ("Flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Acoustic", [4.5, 3.5, 1.0, 1.5, 3.0, 3.5]),
    ("Bass booster", [5.5, 4.0, 1.5, 0.0, 0.0, 0.0]),
    ("Bass reducer", [-5.5, -4.0, -1.5, 0.0, 0.0, 0.0]),
    ("Classical", [4.5, 3.0, -1.5, -1.5, 2.5, 4.0]),
    ("Dance", [4.0, 6.0, 2.0, 0.0, 3.5, 2.5]),
    ("Deep", [4.5, 3.0, 1.5, 0.5, -2.5, -4.5]),
    ("Electronic", [4.0, 3.5, 0.5, -1.5, 2.0, 4.5]),
    ("HipHop", [5.0, 4.0, 1.0, -1.0, 1.5, 3.0]),
    ("Jazz", [4.0, 2.5, -1.5, 1.5, 2.5, 3.5]),
    ("Latin", [4.5, 3.0, 0.0, 0.0, 2.0, 4.5]),
    ("Loudness", [6.0, 4.0, 0.0, -2.0, 1.5, 4.5]),
    ("Lounge", [-3.0, -1.5, 2.0, 1.5, 0.0, -1.5]),
    ("Piano", [3.0, 2.0, 0.0, 2.5, 3.0, 3.5]),
    ("Pop", [-1.5, 1.0, 4.0, 4.5, 2.0, -1.5]),
    ("RnB", [3.0, 7.0, 3.0, -1.5, 2.5, 3.0]),
    ("Rock", [5.0, 4.0, -0.5, 0.0, 2.5, 4.0]),
    ("Small speakers", [5.5, 4.0, 2.5, 1.5, -1.0, -4.0]),
    ("Spoken word", [-3.5, -0.5, 2.5, 3.5, 3.0, 1.0]),
    ("Treble booster", [0.0, 0.0, 0.0, 1.5, 3.5, 5.5]),
    ("Treble reducer", [0.0, 0.0, 0.0, -1.5, -3.5, -5.5]),
    ("Vocal booster", [-1.5, -3.0, 2.5, 4.0, 3.0, 1.0]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_is_on_after_upgrading_and_a_later_choice_is_kept() {
        let mut old: Settings = serde_json::from_str(r#"{"version": 1, "share_local_files": false, "volume": 3.0}"#).unwrap();
        old.upgrade();
        old.sanitize();
        assert!(old.share_local_files, "on now that it needs no pairing");
        assert_eq!((old.version, old.volume), (SETTINGS_VERSION, 1.0));
        let mut chosen: Settings = serde_json::from_str(r#"{"version": 2, "share_local_files": false}"#).unwrap();
        chosen.upgrade();
        assert!(!chosen.share_local_files, "turned off since: kept");
        assert!(Settings::default().share_local_files);
    }
}
