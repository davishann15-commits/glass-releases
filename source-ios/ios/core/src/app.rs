//! The iOS core's state machine: the desktop `App` (src/app.rs) without egui.
//!
//! Same flow as the desktop: the UI asks for an `Action`, work runs on the
//! tokio runtime, and its answer comes back as a `Msg`, applied here in order.
//! The desktop redraws every frame and reads `App` directly; here an actor
//! thread owns this state, and after each batch of messages `publish` sends
//! Swift whatever changed (see publish.rs). Pages are built from the same
//! Web-API-shaped data, cached and refreshed the same way (`want`), so a page
//! updates by itself when what it shows changes (see views.rs).
//!
//! Logic here follows src/app.rs function by function; comments name the
//! desktop function where the behaviour (and the toast wording) comes from.

use crate::api::{Api, NetLog, enc};
use crate::auth::{self, Auth};
use crate::bridge::{CoreEvent, CoreListener};
use crate::config::Settings;
use crate::downloads::Downloads;
use crate::dsp::{DspParams, DspState};
use crate::engine::{Engine, EngineEvent};
use crate::local::{LocalPlayer, LocalTrack};
use crate::model::{self, id_of, kind_of, s};
use crate::theme_config::ThemeStore;
use crate::types::Route;
use crate::user_config::ConfigStore;
use http::Method;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

pub enum Load {
    Pending,
    Ready(Arc<Value>),
    Failed(String),
}

/// The engine status while the engine can't start (it's tried again by itself).
const ENGINE_FAILED: &str = "Can't connect to Spotify.";
/// The engine's status for an account that isn't Premium (Settings, Devices).
const NEEDS_PREMIUM: &str = "Spotify plays songs only for Premium accounts. Browsing, Local Files and Download as work without it.";

#[derive(Default)]
pub struct Playback {
    pub item: Option<Arc<Value>>,
    pub context_uri: String,
    pub is_playing: bool,
    pub progress_ms: u64,
    pub progress_at: Option<Instant>,
    pub shuffle: bool,
    pub repeat: String,
    pub device: Option<Value>,
    pub volume: f32,
    pub volume_dragging: bool,
}

impl Playback {
    pub fn position(&self) -> u64 {
        let base = self.progress_ms;
        let extra = if self.is_playing { self.progress_at.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0) } else { 0 };
        let dur = self.item.as_ref().map(|i| model::u(i, "duration_ms")).filter(|&d| d > 0).unwrap_or(u64::MAX);
        (base + extra).min(dur)
    }
    pub fn uri(&self) -> &str {
        self.item.as_ref().map(|i| s(i, "uri")).unwrap_or("")
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum AuthState {
    SignedOut,
    SigningIn,
    SignedIn,
}

/// Something to play once the playback engine is ready (desktop `PendingPlay`).
pub struct PendingPlay {
    pub body: Value,
    pub shuffle: Option<bool>,
    pub quiet: bool,
}

/// What the app asks for. The same actions as the desktop's, minus the ones
/// that are about its windows (those are Swift's own business on iOS).
#[derive(Clone, Debug)]
pub enum Action {
    PlayContext { context: String, track: Option<String> },
    PlayUris { uris: Vec<String>, index: usize },
    PlayLocal { index: usize },
    PlayLocalPaths { paths: Vec<String>, start: usize },
    Radio(String),
    TogglePlay,
    /// From the system's media controls: play only if paused, pause only if
    /// playing (AirPods send Pause when they come out; that must never start music).
    Play,
    Pause,
    Next,
    Prev,
    Seek(u64),
    SeekBy(i64),
    Volume(f32),
    VolumeCommit,
    ToggleMute,
    SetShuffle(bool),
    CycleRepeat,
    Queue(Vec<String>),
    QueueCollection(String),
    ClearQueue,
    Transfer(String),
    Save { uris: Vec<String>, save: bool },
    AddToPlaylist { playlist: String, uris: Vec<String> },
    RemoveFromPlaylist { playlist: String, uris: Vec<String> },
    MovePlaylistItem { playlist: String, from: usize, to: usize },
    CreatePlaylist { name: String, uris: Vec<String> },
    UpdatePlaylist { id: String, name: String, description: String, public: bool, collaborative: bool },
    DeletePlaylist(String),
    Pin(String),
    Unpin(String),
    CreateFolder { name: String, add: Option<String> },
    RenameFolder { id: String, name: String },
    DeleteFolder(String),
    MoveToFolder { playlist: String, folder: Option<String> },
    Invalidate(String),
    RestartEngine,
    RescanLocal,
    Download { uri: String, name: String },
    RemoveDownload(String),
    RemoveAllDownloads,
    ClearAudioCache,
    PlayLocalPlaylist { id: String, index: usize },
    DeleteLocalPlaylist(String),
    AddToLocalPlaylist { id: String, paths: Vec<String> },
    CreateLocalPlaylist { name: String, paths: Vec<String> },
    RemoveFromLocalPlaylist { id: String, index: usize },
    UndoPlaylistChange,
    SignIn,
    CancelSignIn,
    LogOut,
    Toast(String),
    Lyrics(crate::lyrics_app::LyricsAction),
    ThemeSet { key: String, value: String },
    /// Shown, not written (while a slider or color is being dragged).
    ThemePreview { key: String, value: String },
    ConfigSet { key: String, value: crate::bridge::ConfigValue },
    FilesAdded(Vec<String>),
    SaveSong(String),
    SaveCollection { uri: String, name: String },
    StopSaveSet(String),
    ShareBrowse(String),
    ShareGet { peer: String, ids: Vec<String> },
    AiSend(String),
    AiCancel,
    AiRetry,
    AiNewChat,
    AiSignIn { reconsent: bool, new_account: bool },
    AiCancelSignIn,
    AiSignOut,
    AiSelectAccount(String),
    AiApprove(String),
    AiDecline(String),
    AiSaveDraft { draft: String, replace: Option<String> },
    AiDiscardDraft(String),
    AiEditDraft { draft: String, edit: crate::assistant_app::DraftEdit },
    AiUndo,
    AiRefreshModels,
    AiSetModel(String),
    AiSetBilling(bool),
    AiEnable(bool),
    AiForgetApiKey,
    AiDismissWelcome,
    AiLimitAcknowledged,
    AiUseSpotify(bool),
    /// Files picked (already copied into Glass's folder) or dropped, for `target`.
    AddFiles { paths: Vec<String>, target: crate::types::FileTarget },
    /// A playlist's local songs that aren't here, from your other devices.
    GetMissingFromDevices(String),
    SendToDevice { peer: String, paths: Vec<String> },
    GetFromDevices(Vec<String>),
    AddLocalToPlaylist { playlist: String, paths: Vec<String> },
    SaveTags { path: String, tags: crate::local_app::TagsModel, cover: crate::local_app::CoverChange },
    RemoveSource(String),
    ThemeReset(String),
    SetLibrarySort(crate::config::LibrarySort),
    SetLibraryView(crate::config::LibraryView),
    /// A search that was used (tapped into) is kept, newest first, 10 at most.
    RememberSearch(String),
    ClearRecentSearches,
    /// Add to Apple Music (src/apple_music): a pairing file picked in Files.
    AppleMusicPairing(Vec<u8>),
    AppleMusicForgetPairing,
    /// These Local Files songs into the iPhone's music library (and into the
    /// playlist named, made if needed).
    AppleMusicAdd { paths: Vec<String>, playlist: Option<String> },
    AppleMusicStop,
    AppleMusicCheck,
    AppleMusicRestore,
    /// Moves a song in Now Playing's queue: `from` and `to` index the queue's
    /// `positions` (the player's own list); `uri` is the song meant.
    MoveInQueue { uri: String, from: usize, to: usize },
    /// Local Files songs into Local Files' own queue (while it plays).
    QueueLocal(Vec<String>),
}

pub enum Msg {
    Act(Action),
    /// Swift's view of things changed (pages shown, panels open, in front or not).
    Subscribe(Route),
    Unsubscribe(Route),
    Panels { queue: bool, devices: bool, now_playing: bool },
    Foreground(bool),
    /// Check whether these are in the library (hearts, Follow, Save buttons).
    WantSaved(Vec<String>),
    SetSettings(Box<Settings>),
    SetModel(Box<crate::settings_app::SettingsModel>),
    /// The audio route changed (`gone`: the device that was playing went away).
    RouteChanged { gone: bool },
    /// A phone call or Siri took the audio over (`began`), or gave it back.
    Interrupted { began: bool, resume: bool },
    SignInStarted(u64, Result<crate::signin::Pending, String>),
    SignedIn(u64, Result<Arc<Auth>, String>),
    Data { key: String, result: Result<Arc<Value>, String>, generation: u64 },
    PlayerPushed,
    Player(Result<Value, String>),
    Queue(Result<Value, String>),
    Devices(Value),
    Saved(Vec<(String, bool)>),
    SavedFailed(Vec<String>),
    EngineReady(Result<Engine, String>),
    Engine(EngineEvent),
    Done { toast: Option<String>, invalidate: Vec<String>, refresh_player: bool, navigate: Option<Route> },
    Failed(String),
    DjClips(u64, Result<Option<crate::dj::Voice>, String>),
    LocalScanned(Vec<LocalTrack>),
    Lyrics { generation: u64, resolved: crate::lyrics::service::Resolved },
    Palette(String, Option<crate::palette::Palette>),
    Measured { audio: u64, lyrics: (u64, u64) },
    TagsSaved(std::path::PathBuf, Result<(), String>),
    SaveSong(String, crate::save_song::Status),
    /// A collection's songs to save; a playlist's description and cover (saved beside them).
    SaveSetItems { uri: String, name: String, kind: crate::save_song::SetKind, items: Vec<Value>, description: String, cover: Option<std::path::PathBuf> },
    SaveSongItem(Value),
    Share(crate::share::Event),
    AppleMusic(crate::apple_music::Update),
    Ai { generation: u64, event: crate::assistant_app::AiEvent },
    AiWrote(crate::assistant_app::Wrote),
    AiRegistered(String),
    AiSignedIn(Result<crate::ai::account::SignedIn, crate::ai::oauth::AuthError>),
    AiModels { key: String, result: Result<Vec<crate::ai::openai::Model>, String> },
    AiSignedOut { confirmed: bool },
    AiOpenSignIn(String),
    AiKeySaved(Result<(), String>),
    /// The page on top and whether the assistant is on screen.
    Screen { route: Option<Route>, assistant: bool },
    /// The Activity page's data, measured.
    Activity(tokio::sync::oneshot::Sender<crate::activity_app::ActivityModel>),
    ClearRequestLog,
    ExportFile(crate::theme_app::EditableFile, tokio::sync::oneshot::Sender<Result<String, String>>),
    ImportFile(crate::theme_app::EditableFile, String, tokio::sync::oneshot::Sender<Result<String, String>>),
    /// Debug builds: the assistant talks to a local mock of OpenAI's API (UI tests).
    #[cfg(debug_assertions)]
    DebugAiMock(String),
    FilesRead(crate::types::FileTarget, Vec<std::path::PathBuf>, Vec<LocalTrack>),
    SettingsOpen(bool),
    LyricsSearch { generation: u64, result: Result<Vec<crate::lyrics::Candidate>, String> },
    LyricsTranslated { key: String, result: crate::lyrics::translate::Translation },
    /// Something the views read changed outside the messages above (a download finished).
    Changed,
    Shutdown,
}

/// A sender that also wakes the actor (it sleeps between ticks).
#[derive(Clone)]
pub struct Tx(pub Sender<Msg>);

impl Tx {
    pub fn send(&self, m: Msg) {
        let _ = self.0.send(m);
    }
}

pub struct MobileApp {
    pub rt: tokio::runtime::Handle,
    pub tx: Tx,
    pub emit: Arc<dyn CoreListener>,
    pub demo: bool,
    pub settings: Settings,
    settings_dirty: Option<Instant>,
    pub theme: ThemeStore,
    pub config: ConfigStore,
    pub playlists: crate::playlists::Playlists,
    pub auth_state: AuthState,
    pub auth_error: Option<String>,
    signin_gen: u64,
    resume_retry: Option<(Instant, u32)>,
    engine_retry: Option<(Instant, u32)>,
    /// An engine is being started (one at a time).
    engine_launching: bool,
    data_gen: u64,
    invalidated: Vec<(u64, String)>,
    pub api: Option<Api>,
    pub log: NetLog,
    pub engine: Option<Engine>,
    pub engine_status: String,
    pub(crate) pending_play: Option<PendingPlay>,
    /// What's next on this iPhone's own Connect device (librespot's list: URI, and
    /// whether the user queued it), while it plays. `None` otherwise.
    pub(crate) upcoming: Option<Vec<(String, bool)>>,
    /// Songs' details by URI from Spotify's queue answers, for that list (it has only URIs).
    pub(crate) queue_meta: HashMap<String, Value>,
    pub downloads: Downloads,
    last_prune: Instant,
    dsp: Arc<DspParams>,
    dsp_sent: Option<DspState>,
    pub me: Option<Value>,
    pub data: HashMap<String, Load>,
    fetched_at: HashMap<String, Instant>,
    used_at: HashMap<String, Instant>,
    last_evict: Instant,
    refreshing: HashSet<String>,
    pub saved: HashMap<String, bool>,
    /// Saved flags not sent to Swift yet.
    pub saved_changed: Vec<(String, bool)>,
    saved_wanted: HashSet<String>,
    saved_pending: HashSet<String>,
    saved_backoff: Option<Instant>,
    pub pb: Playback,
    pub queue: Option<Arc<Value>>,
    pub devices: Option<Value>,
    pub queue_error: Option<String>,
    next_poll: Instant,
    next_queue_poll: Instant,
    queue_read_for: String,
    next_devices_poll: Instant,
    devices_shown: bool,
    player_polling: bool,
    next_me_poll: Instant,
    pub(crate) local_touched: Option<Instant>,
    engine_uri: String,
    pub play_hint: Option<Value>,
    skipped_explicit: String,
    pub muted_volume: Option<f32>,
    last_volume_sent: Instant,
    dj_voice: Option<crate::dj::Voice>,
    dj_waiting: bool,
    dj_gen: u64,
    dj_line: Option<(String, Instant)>,
    dj_clips: crate::dj::Clips,
    dj_http: reqwest::Client,
    pending_local: Option<Action>,
    local_links: HashMap<String, Option<usize>>,
    pub local: Option<LocalPlayer>,
    pub local_tracks: Option<Arc<Vec<LocalTrack>>>,
    pub local_scanning: bool,
    local_rescan: bool,
    pub lyrics: crate::lyrics_app::LyricsCore,
    pub album: crate::colors_app::AlbumState,
    pub saves: crate::save_app::SaveState,
    pub shares: crate::share_app::ShareState,
    pub apple_music: crate::apple_music::AppleMusicState,
    /// The assistant (shared `ai`), and what only the iOS port keeps.
    pub ai: crate::ai::Assistant,
    pub ai_extra: crate::assistant_app::AiExtra,
    /// Settings is on screen (its live notes are measured then).
    pub settings_open: bool,
    pub measured: crate::settings_app::Measured,
    /// What the Local Files model was last built from (see `publish_local`).
    pub local_sig: Option<(usize, bool, Vec<std::path::PathBuf>, Vec<(String, String, usize, u64)>, bool)>,
    /// Pages Swift shows, by key; and which panels are open.
    pub views: HashMap<String, Route>,
    pub panel_queue: bool,
    pub panel_devices: bool,
    pub on_now_playing: bool,
    pub foreground: bool,
    /// The audio session was interrupted while playing here (resume when it ends, if iOS says so).
    interrupted_playing: bool,
    pub published: crate::publish::Published,
    /// Toasts and navigations not sent yet.
    pub outbox: Vec<CoreEvent>,
}

/// Library data kept for good (the desktop's `kept_for_good`): Your Library and what the command bar reads.
fn kept_for_good(key: &str) -> bool {
    key == "/me"
        || key.starts_with("ALL2000:/me/playlists")
        || key.starts_with("ALL2000:/me/albums")
        || key.starts_with("ALL2000:/me/following")
        || key.starts_with("ALL500:/me/shows")
        || key.starts_with("ALL500:/me/audiobooks")
        || key.starts_with("/me/tracks?limit=1")
        || key.starts_with("/me/episodes?limit=1")
}

impl MobileApp {
    pub fn new(rt: tokio::runtime::Handle, tx: Tx, emit: Arc<dyn CoreListener>, demo: bool) -> Self {
        let settings = Settings::load();
        let dsp = Arc::new(DspParams::new(DspState { eq_enabled: settings.eq_enabled, gains: settings.eq_gains, mono: settings.mono }));
        let volume = settings.volume;
        crate::config::use_proxy(&settings.proxy);
        let log = NetLog::default();
        let config = ConfigStore::load();
        let lyrics = crate::lyrics_app::LyricsCore::new(&config.lyrics().contact, &settings.proxy, log.clone(), demo);
        let mut app = Self {
            rt,
            tx,
            emit,
            demo,
            settings,
            settings_dirty: None,
            theme: ThemeStore::load(),
            config,
            lyrics,
            album: Default::default(),
            saves: Default::default(),
            shares: Default::default(),
            apple_music: Default::default(),
            ai: crate::assistant_app::new_assistant(demo),
            ai_extra: Default::default(),
            settings_open: false,
            measured: Default::default(),
            local_sig: None,
            playlists: crate::playlists::Playlists::load(crate::playlists::Playlists::path()),
            auth_state: AuthState::SignedOut,
            auth_error: None,
            signin_gen: 0,
            resume_retry: None,
            engine_retry: None,
            engine_launching: false,
            data_gen: 0,
            invalidated: Vec::new(),
            api: None,
            log,
            engine: None,
            engine_status: "Not started".into(),
            pending_play: None,
            upcoming: None,
            queue_meta: HashMap::new(),
            downloads: Downloads::load(),
            last_prune: Instant::now(),
            dsp,
            dsp_sent: None,
            me: None,
            data: HashMap::new(),
            fetched_at: HashMap::new(),
            used_at: HashMap::new(),
            last_evict: Instant::now(),
            refreshing: HashSet::new(),
            saved: HashMap::new(),
            saved_changed: Vec::new(),
            saved_wanted: HashSet::new(),
            saved_pending: HashSet::new(),
            saved_backoff: None,
            pb: Playback { volume, repeat: "off".into(), ..Default::default() },
            queue: None,
            devices: None,
            queue_error: None,
            next_poll: Instant::now(),
            next_queue_poll: Instant::now(),
            queue_read_for: String::new(),
            next_devices_poll: Instant::now(),
            devices_shown: false,
            player_polling: false,
            next_me_poll: Instant::now() + Duration::from_secs(5),
            local_touched: None,
            engine_uri: String::new(),
            play_hint: None,
            skipped_explicit: String::new(),
            muted_volume: None,
            last_volume_sent: Instant::now(),
            dj_voice: None,
            dj_waiting: false,
            dj_gen: 0,
            dj_line: None,
            dj_clips: Default::default(),
            dj_http: crate::config::http().build().unwrap_or_default(),
            pending_local: None,
            local_links: HashMap::new(),
            local: None,
            local_tracks: None,
            local_scanning: false,
            local_rescan: false,
            views: HashMap::new(),
            panel_queue: false,
            panel_devices: false,
            on_now_playing: false,
            foreground: true,
            interrupted_playing: false,
            published: Default::default(),
            outbox: Vec::new(),
        };
        // Folders and songs where they are this run: iOS moves the app's Documents when a
        // new build is installed, and another app's folder picked in Files when that app
        // is updated (see `paths::relocate`). A folder the app can't open any more (its
        // bookmark was lost) stops being a source; the folders picked as sources are ones.
        // (The demo never saves its settings; its local playlists aren't touched.)
        let picked = crate::paths::picked();
        let here = |d: &std::path::Path| crate::paths::relocate(d, picked);
        let open = |d: &std::path::Path| !cfg!(target_os = "ios") || crate::paths::readable(d, picked);
        let mut dirs: Vec<std::path::PathBuf> = app.settings.local_dirs.iter().map(|d| here(d)).filter(|d| open(d)).collect();
        for f in picked.iter().filter(|f| f.source && f.path.is_dir()) {
            if !crate::local::read_by(&dirs, &f.path) {
                dirs.push(f.path.clone());
            }
        }
        let tidy = crate::local::tidy_sources(&dirs);
        let save_dir = app.settings.save_dir.as_deref().map(here).filter(|d| open(d));
        if tidy != app.settings.local_dirs || save_dir != app.settings.save_dir {
            app.settings.local_dirs = tidy;
            app.settings.save_dir = save_dir;
            app.mark_settings_dirty();
        }
        if !demo {
            app.playlists.map_paths(here);
        }
        if !demo {
            let saves = app.settings.save_dir.clone().unwrap_or_else(crate::save_song::default_dir);
            let cache = app.lyrics.cache.clone();
            std::thread::spawn(move || {
                cache.prune();
                crate::save_song::sweep_work_files(&saves);
            });
        }
        if app.demo {
            crate::demo::install(&mut app);
        } else {
            if !app.settings.local_dirs.is_empty() {
                app.scan_local();
            }
            if auth::has_saved_session() {
                app.auth_state = AuthState::SigningIn;
                app.resume_session();
            }
        }
        app
    }

    // ------------------------------------------------------------ the loop

    /// The actor: applies messages, ticks, and tells Swift what changed. Returns at shutdown.
    pub fn run(mut self, rx: Receiver<Msg>) {
        self.publish();
        loop {
            let wait = self.next_wake();
            match rx.recv_timeout(wait) {
                Ok(Msg::Shutdown) => break,
                Ok(m) => {
                    self.handle(m);
                    // A burst (a page's requests answering together) is one publish.
                    while let Ok(m) = rx.try_recv() {
                        if matches!(m, Msg::Shutdown) {
                            self.save_now();
                            return;
                        }
                        self.handle(m);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
            self.publish();
        }
        self.save_now();
    }

    /// How long to sleep (desktop `tick`'s repaint scheduling, without frames to draw).
    fn next_wake(&self) -> Duration {
        if self.dj_voice.is_some() || self.dj_waiting {
            return Duration::from_millis(150);
        }
        let local_playing = self.local.as_ref().is_some_and(|l| l.is_playing());
        if self.pb.is_playing || local_playing || self.settings_dirty.is_some() {
            return Duration::from_millis(500);
        }
        self.next_poll.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(250), Duration::from_secs(30))
    }

    pub fn act(&mut self, a: Action) {
        self.apply(a);
    }

    pub fn toast(&mut self, t: impl Into<String>) {
        self.outbox.push(CoreEvent::Toast { text: t.into() });
    }

    fn navigate(&mut self, r: Route) {
        self.outbox.push(CoreEvent::Navigate { route: r });
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty.get_or_insert(Instant::now());
    }

    pub fn save_now(&mut self) {
        if !self.demo {
            self.settings.save();
        }
        self.settings_dirty = None;
        self.lyrics.choices.flush();
    }

    pub fn user_id(&self) -> String {
        self.me.as_ref().map(|m| s(m, "id").to_string()).unwrap_or_default()
    }

    pub fn is_premium(&self) -> bool {
        self.me.as_ref().map(|m| s(m, "product") == "premium").unwrap_or(true)
    }

    /// Desktop `account_known`: `/me` answered for the first time since signing in.
    /// Spotify streams only to Premium accounts: they get the playback engine; a free
    /// account browses, plays Local Files and saves songs (Download as), without one.
    fn account_known(&mut self) {
        if self.is_premium() {
            if self.engine.is_none() {
                self.start_engine();
            }
        } else {
            self.engine = None;
            self.engine_retry = None;
            self.pending_play = None;
            self.engine_status = NEEDS_PREMIUM.into();
        }
    }

    /// A song from Spotify was to play on a free account.
    fn needs_premium(&mut self, quiet: bool) {
        if quiet {
            self.ai.tell_model("Spotify plays songs only for Premium accounts and this one is Spotify Free, so nothing is playing.");
        } else {
            self.toast("Playing songs from Spotify needs Spotify Premium");
        }
    }

    // ------------------------------------------------------------ data

    pub fn want(&mut self, path: &str) -> Option<Arc<Value>> {
        self.want_key(path, None)
    }

    pub fn want_all(&mut self, path: &str, cap: usize) -> Option<Arc<Value>> {
        self.want_key(&format!("ALL{cap}:{path}"), Some((path, cap)))
    }

    /// Desktop `want_key`: cached answers at once; older than 5 minutes, refetched
    /// in the background and swapped in; a failure is tried again after 20 s.
    fn want_key(&mut self, key: &str, all: Option<(&str, usize)>) -> Option<Arc<Value>> {
        const TTL: Duration = Duration::from_secs(300);
        const RETRY: Duration = Duration::from_secs(20);
        match self.used_at.get_mut(key) {
            Some(t) => *t = Instant::now(),
            None => {
                self.used_at.insert(key.to_string(), Instant::now());
            }
        }
        let cached = match self.data.get(key) {
            Some(Load::Ready(v)) => Some(v.clone()),
            Some(Load::Failed(_)) if self.fetched_at.get(key).is_none_or(|t| t.elapsed() >= RETRY) => None,
            Some(_) => return None,
            None => None,
        };
        // The demo's data never goes stale (there's no network to refetch it from).
        if cached.is_some() && (self.demo || self.fetched_at.get(key).is_some_and(|t| t.elapsed() < TTL) || self.refreshing.contains(key)) {
            return cached;
        }
        let Some(api) = self.api.clone() else { return cached };
        if cached.is_some() {
            self.refreshing.insert(key.to_string());
        } else {
            self.data.insert(key.to_string(), Load::Pending);
        }
        let (key, all) = (key.to_string(), all.map(|(path, cap)| (path.to_string(), cap)));
        let (tx, generation) = (self.tx.clone(), self.data_gen);
        self.rt.spawn(async move {
            let failed = Answer::new(tx.clone(), Msg::Data { key: key.clone(), result: Err("the request failed".into()), generation });
            let r = match &all {
                Some((path, cap)) => api.get_all(path, *cap).await.map(Arc::new),
                None => api.get_shared(&key).await,
            };
            failed.disarm();
            tx.send(Msg::Data { key, result: r.map_err(|e| e.to_string()), generation });
        });
        cached
    }

    /// Whether a load failed, and why (for the page's error box).
    pub fn load_error(&self, path: &str) -> Option<String> {
        self.data.iter().find_map(|(k, v)| match v {
            Load::Failed(e) if k == path || k.ends_with(&format!(":{path}")) => Some(crate::errors::page(e)),
            _ => None,
        })
    }

    pub fn is_pending(&self, path: &str) -> bool {
        self.data.iter().any(|(k, v)| matches!(v, Load::Pending) && (k == path || k.ends_with(&format!(":{path}"))))
    }

    fn evict_unused(&mut self) {
        const IDLE: Duration = Duration::from_secs(600);
        if self.saved.len() > 20_000 {
            self.saved.clear();
        }
        let shown: Vec<String> = Vec::new();
        let old: Vec<String> = self.used_at.iter().filter(|(k, t)| t.elapsed() > IDLE && !kept_for_good(k) && !shown.contains(k)).map(|(k, _)| k.clone()).collect();
        for k in old {
            self.used_at.remove(&k);
            if matches!(self.data.get(&k), Some(Load::Ready(_)) | Some(Load::Failed(_))) {
                self.data.remove(&k);
                self.fetched_at.remove(&k);
            }
        }
    }

    /// Desktop `invalidate`: matching answers are refetched on next use (the old
    /// data stays meanwhile); failed ones are dropped so they retry.
    pub fn invalidate(&mut self, needle: &str) {
        self.data_gen += 1;
        self.invalidated.push((self.data_gen, needle.to_string()));
        if self.invalidated.len() > 64 {
            self.invalidated.remove(0);
        }
        self.data.retain(|k, v| !(matches!(v, Load::Failed(_)) && k.contains(needle)));
        for k in self.data.keys() {
            if k.contains(needle) {
                self.fetched_at.remove(k);
            }
        }
    }

    /// Desktop `is_saved`: `None` while unknown (the check is queued).
    pub fn is_saved(&mut self, uri: &str) -> Option<bool> {
        if let Some(v) = self.saved.get(uri) {
            return Some(*v);
        }
        if !uri.is_empty() && !self.saved_pending.contains(uri) {
            self.saved_wanted.insert(uri.to_string());
        }
        None
    }

    fn set_saved(&mut self, uri: String, v: bool) {
        if self.saved.get(&uri) != Some(&v) {
            self.saved_changed.push((uri.clone(), v));
        }
        self.saved.insert(uri, v);
    }

    /// Desktop `flush_saved_queries`.
    fn flush_saved_queries(&mut self) {
        if self.saved_wanted.is_empty() || self.saved_backoff.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
            return;
        }
        let Some(api) = self.api.clone() else { return };
        let mut by_kind: HashMap<String, Vec<String>> = HashMap::new();
        for uri in self.saved_wanted.drain() {
            self.saved_pending.insert(uri.clone());
            by_kind.entry(kind_of(&uri).to_string()).or_default().push(uri);
        }
        let me = self.user_id();
        for (kind, uris) in by_kind {
            for chunk in uris.chunks(40) {
                let chunk: Vec<String> = chunk.to_vec();
                let ids = chunk.iter().map(|u| id_of(u)).collect::<Vec<_>>().join(",");
                let primary = match kind.as_str() {
                    "track" => format!("/me/tracks/contains?ids={ids}"),
                    "album" => format!("/me/albums/contains?ids={ids}"),
                    "show" => format!("/me/shows/contains?ids={ids}"),
                    "episode" => format!("/me/episodes/contains?ids={ids}"),
                    "audiobook" => format!("/me/audiobooks/contains?ids={ids}"),
                    "artist" => format!("/me/following/contains?type=artist&ids={ids}"),
                    "playlist" => format!("/playlists/{}/followers/contains?ids={}", id_of(&chunk[0]), enc(&me)),
                    _ => {
                        for u in &chunk {
                            self.saved_pending.remove(u);
                        }
                        continue;
                    }
                };
                let chunk = if kind == "playlist" { chunk[..1].to_vec() } else { chunk };
                let fallback = format!("/me/library/contains?uris={}", enc(&chunk.join(",")));
                let (api, tx) = (api.clone(), self.tx.clone());
                let is_playlist = kind == "playlist";
                let rest: Vec<String> = if is_playlist { uris.iter().skip(1).cloned().collect() } else { Vec::new() };
                self.rt.spawn(async move {
                    let v = match api.get(&primary).await {
                        Ok(v) => Ok(v),
                        Err(_) => api.get(&fallback).await,
                    };
                    let msg = match v.ok().and_then(|v| v.as_array().map(|a| a.iter().map(|b| b.as_bool().unwrap_or(false)).collect::<Vec<bool>>())) {
                        Some(flags) => Msg::Saved(chunk.iter().cloned().zip(flags.into_iter().chain(std::iter::repeat(false))).collect()),
                        None => Msg::SavedFailed(chunk),
                    };
                    tx.send(msg);
                });
                if is_playlist {
                    for u in rest {
                        self.saved_pending.remove(&u);
                        // Asked again (one playlist per request) at the next flush.
                        self.saved_wanted.insert(u);
                    }
                    break;
                }
            }
        }
    }

    /// Desktop `mutate`: a write sent once; a 404 tries `fallback` (another shape of the endpoint).
    #[allow(clippy::too_many_arguments)]
    fn mutate(&self, method: Method, path: String, body: Option<Value>, fallback: Option<(String, Option<Value>)>, toast: Option<String>, invalidate: Vec<String>, refresh_player: bool) {
        let Some(api) = self.api.clone() else { return };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let mut r = api.send(method.clone(), &path, body).await;
            if r.as_ref().is_err_and(|e| e.to_string().starts_with("404"))
                && let Some((fp, fb)) = fallback
            {
                r = api.send(method, &fp, fb).await;
            }
            tx.send(match r {
                Ok(_) => Msg::Done { toast, invalidate, refresh_player, navigate: None },
                Err(e) => Msg::Failed(e.to_string()),
            });
        });
    }

    // ------------------------------------------------------------ playback

    pub fn engine_device(&self) -> Option<&str> {
        self.engine.as_ref().map(|e| e.device_id.as_str())
    }

    /// Playback happens on this iPhone (desktop `is_local`).
    pub fn is_local(&self) -> bool {
        match (self.engine_device(), self.pb.device.as_ref().map(|d| s(d, "id"))) {
            (Some(e), Some(d)) => e == d,
            (Some(_), None) => true,
            _ => false,
        }
    }

    fn target_device(&self) -> Option<String> {
        self.pb.device.as_ref().map(|d| s(d, "id").to_string()).filter(|d| !d.is_empty()).or_else(|| self.engine_device().map(str::to_string))
    }

    pub fn device_param(&self, first: bool) -> String {
        match self.target_device() {
            Some(d) => format!("{}device_id={}", if first { "?" } else { "&" }, enc(&d)),
            None => String::new(),
        }
    }

    pub fn local_file_active(&self) -> bool {
        self.local.as_ref().is_some_and(|l| l.active)
    }

    pub fn playing_here(&self) -> bool {
        if self.local_file_active() {
            return self.local.as_ref().is_some_and(|l| l.is_playing());
        }
        (self.is_local() && self.pb.is_playing) || self.dj_voice.is_some()
    }

    pub(crate) fn stop_local(&mut self) {
        if let Some(l) = self.local.as_mut() {
            l.stop();
        }
    }

    fn play_body(&mut self, body: Value) {
        self.start_play(PendingPlay { body, shuffle: None, quiet: false });
    }

    pub(crate) fn engine_starting(&self) -> bool {
        self.engine.is_none() && self.auth_state == AuthState::SignedIn && !self.engine_status.starts_with(ENGINE_FAILED)
    }

    /// Desktop `start_play`.
    fn start_play(&mut self, p: PendingPlay) {
        if !self.is_premium() {
            self.needs_premium(p.quiet);
            return;
        }
        self.dj_cancel();
        self.stop_local();
        self.replaced_dead_engine(false);
        self.pb.is_playing = true;
        if !p.quiet && self.settings.now_playing_on_play {
            self.outbox.push(CoreEvent::ShowNowPlaying);
        }
        if self.pb.device.is_none() && self.engine_starting() {
            self.pending_play = Some(p);
            return;
        }
        self.play_now(p.body, p.shuffle);
    }

    pub(crate) fn play_mode(&self, body: &Value, shuffle: Option<bool>) -> crate::engine::PlayMode {
        crate::engine::PlayMode::for_load(body, shuffle.unwrap_or(self.pb.shuffle), &self.pb.repeat)
    }

    fn play_now(&mut self, body: Value, shuffle: Option<bool>) {
        let mode = self.play_mode(&body, shuffle);
        if self.is_local()
            && let Some(e) = self.engine.as_ref()
        {
            if let Err(err) = e.load(&body, mode) {
                // Loading only fails when the engine is gone (its task ended, even if
                // that isn't known yet): a new one plays it.
                log::warn!("playback engine refused a load ({err}); starting a new one");
                self.engine_lost(false);
                self.pending_play = Some(PendingPlay { body, shuffle, quiet: true });
                self.pb.is_playing = true;
                return;
            }
            self.local_touched = Some(Instant::now());
            self.pb.shuffle = mode.shuffle;
            if let Some(c) = body["context_uri"].as_str() {
                self.pb.context_uri = c.to_string();
            }
            let index = body["offset"]["position"].as_u64().unwrap_or(0) as usize;
            let target = body["offset"]["uri"].as_str().or_else(|| body["uris"].get(index).and_then(Value::as_str)).unwrap_or("").to_string();
            if !target.is_empty()
                && target != self.pb.uri()
                && let Some(item) = self.known_item(&target)
            {
                self.show_track(item);
            }
            return;
        }
        if self.demo {
            self.demo_play(&body);
            return;
        }
        self.play_hint = None;
        let path = format!("/me/player/play{}", self.device_param(true));
        self.mutate(Method::PUT, path, Some(body), None, None, vec![], true);
        self.poll_soon();
    }

    pub fn poll_soon(&mut self) {
        self.next_poll = Instant::now() + Duration::from_millis(350);
        self.next_queue_poll = self.next_poll;
    }

    fn transport(&mut self, local: impl FnOnce(&Engine), method: Method, path: &str) {
        if self.replaced_dead_engine(self.pb.is_playing) {
            return;
        }
        if self.is_local()
            && let Some(e) = self.engine.as_ref()
        {
            local(e);
            self.local_touched = Some(Instant::now());
            return;
        }
        if self.demo {
            return;
        }
        let sep = !path.contains('?');
        let full = format!("{path}{}", self.device_param(sep));
        self.mutate(method, full, None, None, None, vec![], true);
        self.poll_soon();
    }

    /// Desktop `set_volume`. Playing here, the volume is the iPhone's own (the app
    /// moves the system volume; audio goes out at full scale): this only tells
    /// Spotify Connect, so other devices show it. Elsewhere it sets that device's.
    pub(crate) fn set_volume(&mut self, v: f32, commit: bool) {
        let v = v.clamp(0.0, 1.0);
        self.pb.volume = v;
        if self.muted_volume.is_none() {
            self.settings.volume = v;
            self.mark_settings_dirty();
        }
        if self.local_file_active() {
            return;
        }
        if self.is_local() {
            if let Some(e) = self.engine.as_ref() {
                let _ = e.spirc.set_volume((v * u16::MAX as f32) as u16);
            }
        } else if commit || self.last_volume_sent.elapsed() > Duration::from_millis(250) {
            self.last_volume_sent = Instant::now();
            let path = format!("/me/player/volume?volume_percent={}{}", (v * 100.0).round() as u32, self.device_param(false));
            self.mutate(Method::PUT, path, None, None, None, vec![], false);
        }
    }

    /// A song's details already at hand: the row just tapped, or the queue.
    fn known_item(&mut self, uri: &str) -> Option<Value> {
        if let Some(h) = self.play_hint.take()
            && s(&h, "uri") == uri
        {
            return Some(h);
        }
        self.queue.as_ref()?["queue"].as_array()?.iter().find(|t| s(t, "uri") == uri).cloned()
    }

    fn show_track(&mut self, item: Value) {
        self.pb.item = Some(Arc::new(item));
        self.pb.progress_ms = 0;
        self.pb.progress_at = Some(Instant::now());
        self.next_queue_poll = Instant::now();
        self.on_track_changed();
    }

    fn on_track_changed(&mut self) {
        self.invalidate("/me/player/recently-played");
    }

    /// Desktop `maybe_skip_explicit`.
    fn maybe_skip_explicit(&mut self) {
        if self.settings.allow_explicit || !self.pb.is_playing || !self.pb.item.as_ref().is_some_and(|i| model::b(i, "explicit")) {
            return;
        }
        if self.pb.position() > 300 && self.skipped_explicit != self.pb.uri() {
            self.skipped_explicit = self.pb.uri().to_string();
            self.pb.is_playing = false;
            self.toast("Skipped explicit content");
            self.apply(Action::Next);
        }
    }

    // ------------------------------------------------------------ DJ (desktop dj_*)

    fn dj_break(&mut self, uri: String, say: Vec<crate::dj::Line>, ready: Vec<crate::dj::Line>, hold: bool) {
        let session = self.engine.as_ref().map(|e| e.session.clone());
        if let Some(l) = say.last() {
            self.dj_line = Some((say.iter().map(|l| l.text()).collect::<Vec<_>>().join(" "), Instant::now()));
            log::info!("DJ before {uri}: {}", l.text());
        }
        if let Some(session) = session.clone()
            && !ready.is_empty()
        {
            let (http, cache) = (self.dj_http.clone(), self.dj_clips.clone());
            self.rt.spawn(async move {
                let _ = crate::dj::clips(&session, &http, &cache, &ready).await;
            });
        }
        if !hold {
            return;
        }
        let Some(session) = session else {
            self.dj_resume();
            return;
        };
        self.dj_gen += 1;
        let generation = self.dj_gen;
        self.dj_voice = None;
        self.dj_waiting = true;
        self.pb.is_playing = true;
        // Full scale, like the music (the iPhone's volume is the volume).
        let (http, cache, tx, volume) = (self.dj_http.clone(), self.dj_clips.clone(), self.tx.clone(), 1.0);
        self.rt.spawn(async move {
            let r = match tokio::time::timeout(Duration::from_secs(6), crate::dj::clips(&session, &http, &cache, &say)).await {
                Ok(r) => r.map_err(|e| format!("{e:#}")),
                Err(_) => Err("the DJ's voice took too long".to_string()),
            };
            let r = match r {
                Ok(clips) => tokio::task::spawn_blocking(move || crate::dj::Voice::play(&clips, volume)).await.map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            tx.send(Msg::DjClips(generation, r));
        });
    }

    fn dj_clips_ready(&mut self, generation: u64, r: Result<Option<crate::dj::Voice>, String>) {
        if generation != self.dj_gen || !self.dj_waiting {
            return;
        }
        self.dj_waiting = false;
        match r {
            Ok(Some(v)) => self.dj_voice = Some(v),
            Ok(None) => self.dj_resume(),
            Err(e) => {
                log::warn!("DJ voice: {e}");
                self.dj_resume();
            }
        }
    }

    fn dj_resume(&mut self) {
        self.dj_gen += 1;
        self.dj_voice = None;
        self.dj_waiting = false;
        if let Some(e) = self.engine.as_ref() {
            let _ = e.spirc.play();
        }
    }

    pub(crate) fn dj_cancel(&mut self) {
        if self.dj_voice.is_some() || self.dj_waiting {
            self.dj_gen += 1;
            self.dj_voice = None;
            self.dj_waiting = false;
        }
        crate::dj::forget_outro();
    }

    /// What the DJ is saying (or said in the last 20 s).
    pub fn dj_caption(&self) -> Option<String> {
        let (text, at) = self.dj_line.as_ref()?;
        (self.dj_voice.is_some() || self.dj_waiting || at.elapsed() < Duration::from_secs(20)).then(|| text.clone())
    }

    // ------------------------------------------------------------ actions

    pub fn apply(&mut self, a: Action) {
        match a {
            Action::PlayContext { context, track } => {
                let mut body = json!({ "context_uri": context });
                if let Some(t) = track {
                    body["offset"] = json!({ "uri": t });
                }
                self.play_body(body);
            }
            Action::PlayUris { uris, index } => {
                // Spotify takes at most 500 songs; the 500 around the one tapped go.
                const MAX: usize = 500;
                if uris.is_empty() {
                    return;
                }
                let index = index.min(uris.len() - 1);
                let start = index.saturating_sub(MAX / 2).min(uris.len().saturating_sub(MAX));
                let uris: Vec<String> = uris.into_iter().skip(start).take(MAX).collect();
                self.play_body(json!({ "uris": uris, "offset": { "position": index - start } }));
            }
            Action::PlayLocal { index } => {
                if self.local_tracks.is_none() {
                    self.pending_local = Some(Action::PlayLocal { index });
                    self.scan_local();
                    self.toast("Reading your local files…");
                    return;
                }
                let tracks = self.local_tracks.clone().unwrap_or_default();
                self.play_local_list(tracks, index);
            }
            Action::PlayLocalPaths { paths, start } => {
                let Some(all) = self.local_tracks.clone() else {
                    self.pending_local = Some(Action::PlayLocalPaths { paths, start });
                    self.scan_local();
                    self.toast("Reading your local files…");
                    return;
                };
                let by_path: HashMap<&std::path::Path, &LocalTrack> = all.iter().map(|t| (t.path.as_path(), t)).collect();
                let tracks: Vec<LocalTrack> = paths.iter().filter_map(|p| by_path.get(std::path::Path::new(p)).map(|t| (*t).clone())).collect();
                if !tracks.is_empty() {
                    let start = start.min(tracks.len() - 1);
                    self.play_local_list(tracks, start);
                }
            }
            Action::Radio(uri) => {
                if !self.is_premium() {
                    self.needs_premium(false);
                    return;
                }
                self.stop_local();
                let radio_ok = self.is_local() && self.engine.as_ref().map(|e| e.load_radio(&uri).is_ok()).unwrap_or(false);
                if radio_ok {
                    self.toast("Starting radio");
                    self.poll_soon();
                } else if kind_of(&uri) == "track" {
                    self.apply(Action::PlayUris { uris: vec![uri], index: 0 });
                } else {
                    self.apply(Action::PlayContext { context: uri, track: None });
                }
            }
            Action::Play => {
                if !self.is_playing_anything() {
                    self.apply(Action::TogglePlay);
                }
            }
            Action::Pause => {
                if self.is_playing_anything() {
                    self.apply(Action::TogglePlay);
                }
            }
            Action::TogglePlay => self.toggle_play(),
            Action::Next => {
                self.dj_cancel();
                if self.local_file_active() {
                    if let Some(l) = self.local.as_mut() {
                        l.next();
                    }
                    return;
                }
                if self.demo {
                    self.demo_skip(1);
                    return;
                }
                self.transport(
                    |e| {
                        let _ = e.spirc.next();
                    },
                    Method::POST,
                    "/me/player/next",
                );
            }
            Action::Prev => {
                self.dj_cancel();
                if self.local_file_active() {
                    if let Some(l) = self.local.as_mut() {
                        l.prev();
                    }
                    return;
                }
                if self.demo {
                    self.demo_skip(-1);
                    return;
                }
                self.transport(
                    |e| {
                        let _ = e.spirc.prev();
                    },
                    Method::POST,
                    "/me/player/previous",
                );
            }
            Action::Seek(ms) => {
                if self.dj_voice.is_some() || self.dj_waiting {
                    self.dj_resume();
                }
                if self.local_file_active() {
                    if let Some(l) = self.local.as_mut() {
                        l.seek(ms);
                    }
                    return;
                }
                self.pb.progress_ms = ms;
                self.pb.progress_at = Some(Instant::now());
                let path = format!("/me/player/seek?position_ms={ms}");
                self.transport(
                    |e| {
                        let _ = e.spirc.set_position_ms(ms as u32);
                    },
                    Method::PUT,
                    &path,
                );
            }
            Action::SeekBy(delta) => {
                let pos = if self.local_file_active() { self.local.as_ref().map(|l| l.position_ms()).unwrap_or(0) } else { self.pb.position() };
                let to = (pos as i64 + delta).max(0) as u64;
                self.apply(Action::Seek(to));
            }
            Action::Volume(v) => {
                // Turned up (the side buttons, say) while muted: no longer muted.
                if v > 0.0 {
                    self.muted_volume = None;
                }
                self.set_volume(v, false)
            }
            Action::VolumeCommit => {
                let v = self.pb.volume;
                self.set_volume(v, true);
            }
            Action::ToggleMute => {
                if let Some(v) = self.muted_volume.take() {
                    self.set_volume(v, true);
                } else {
                    self.muted_volume = Some(self.pb.volume.max(0.05));
                    self.set_volume(0.0, true);
                }
            }
            Action::SetShuffle(on) => {
                self.pb.shuffle = on;
                let path = format!("/me/player/shuffle?state={on}");
                self.transport(
                    |e| {
                        let _ = e.spirc.shuffle(on);
                    },
                    Method::PUT,
                    &path,
                );
            }
            Action::CycleRepeat => {
                let next = match self.pb.repeat.as_str() {
                    "off" => "context",
                    "context" => "track",
                    _ => "off",
                };
                self.pb.repeat = next.into();
                let path = format!("/me/player/repeat?state={next}");
                self.transport(
                    |e| {
                        let _ = e.spirc.repeat(next != "off");
                        let _ = e.spirc.repeat_track(next == "track");
                    },
                    Method::PUT,
                    &path,
                );
            }
            Action::Queue(uris) => {
                let dev = self.device_param(false);
                let Some(api) = self.api.clone() else {
                    if self.demo {
                        self.toast(if uris.len() == 1 { "Added to queue".to_string() } else { format!("Added {} songs to the queue", uris.len()) });
                    }
                    return;
                };
                let tx = self.tx.clone();
                self.rt.spawn(async move { tx.send(queue_uris(&api, &dev, &uris).await) });
            }
            Action::QueueCollection(uri) => {
                let dev = self.device_param(false);
                let Some(api) = self.api.clone() else { return };
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let msg = match collection_items(&api, &uri).await {
                        Ok(uris) if uris.is_empty() => Msg::Done { toast: Some("There's nothing in it to add".into()), invalidate: vec![], refresh_player: false, navigate: None },
                        Ok(uris) => queue_uris(&api, &dev, &uris).await,
                        Err(e) => Msg::Failed(e.to_string()),
                    };
                    tx.send(msg);
                });
            }
            Action::ClearQueue => {
                // No Web API for this: restarting the current context drops what was queued.
                if let Some(body) = self.resume_body() {
                    self.start_play(PendingPlay { body, shuffle: None, quiet: true });
                }
            }
            Action::Transfer(id) => {
                self.stop_local();
                self.mutate(Method::PUT, "/me/player".into(), Some(json!({ "device_ids": [id], "play": self.pb.is_playing })), None, None, vec![], true);
                self.next_devices_poll = Instant::now() + Duration::from_millis(600);
                self.poll_soon();
            }
            Action::Save { uris, save } => self.save(uris, save),
            Action::AddToPlaylist { playlist, uris } => {
                let Some(api) = self.api.clone() else { return };
                let tx = self.tx.clone();
                let n = uris.len();
                self.rt.spawn(async move {
                    let (added, failed) = add_items(&api, &playlist, &uris).await;
                    let toast = match (&failed, added) {
                        (None, 1) => "Added to playlist".to_string(),
                        (None, _) => format!("Added {n} songs to playlist"),
                        (Some(_), _) => format!("Added {added} of {n} songs; the rest couldn't be added"),
                    };
                    tx.send(match failed {
                        Some(e) if added == 0 => Msg::Failed(e),
                        _ => Msg::Done { toast: Some(toast), invalidate: vec![playlist.clone(), "/me/playlists".into()], refresh_player: false, navigate: None },
                    });
                });
            }
            Action::RemoveFromPlaylist { playlist, uris } => {
                let tracks: Vec<Value> = uris.iter().map(|u| json!({ "uri": u })).collect();
                self.mutate(
                    Method::DELETE,
                    format!("/playlists/{playlist}/tracks"),
                    Some(json!({ "tracks": tracks })),
                    Some((format!("/playlists/{playlist}/items"), Some(json!({ "items": tracks })))),
                    Some("Removed from playlist".into()),
                    vec![playlist],
                    false,
                );
            }
            Action::MovePlaylistItem { playlist, from, to } => {
                let body = json!({ "range_start": from, "insert_before": if to > from { to + 1 } else { to } });
                self.mutate(Method::PUT, format!("/playlists/{playlist}/tracks"), Some(body.clone()), Some((format!("/playlists/{playlist}/items"), Some(body))), None, vec![playlist], false);
            }
            Action::CreatePlaylist { name, uris } => {
                let Some(api) = self.api.clone() else { return };
                let me = self.user_id();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let body = json!({ "name": name, "public": false, "description": "" });
                    let created = match api.send(Method::POST, "/me/playlists", Some(body.clone())).await {
                        Err(e) if e.to_string().starts_with("404") => api.send(Method::POST, &format!("/users/{}/playlists", enc(&me)), Some(body)).await,
                        r => r,
                    };
                    let msg = match created {
                        Ok(v) => {
                            let id = s(&v, "id").to_string();
                            let (_, failed) = add_items(&api, &id, &uris).await;
                            let toast = match failed {
                                None => "Playlist created".to_string(),
                                Some(_) => "Playlist created, but not all the songs could be added".to_string(),
                            };
                            Msg::Done { toast: Some(toast), invalidate: vec!["/me/playlists".into(), id.clone()], refresh_player: false, navigate: Some(Route::Playlist { id }) }
                        }
                        Err(e) => Msg::Failed(e.to_string()),
                    };
                    tx.send(msg);
                });
            }
            Action::UpdatePlaylist { id, name, description, public, collaborative } => {
                if name.trim().is_empty() {
                    return;
                }
                // Collaborative playlists can't be public (modals.rs).
                let body = json!({ "name": name.trim(), "description": description, "public": public && !collaborative, "collaborative": collaborative });
                self.mutate(Method::PUT, format!("/playlists/{id}"), Some(body), None, None, vec![id, "/me/playlists".into()], false);
            }
            Action::DeletePlaylist(id) => {
                self.mutate(Method::DELETE, format!("/playlists/{id}/followers"), None, None, Some("Removed from Your Library".into()), vec!["/me/playlists".into()], false);
                self.outbox.push(CoreEvent::Left { route: Route::Playlist { id } });
            }
            Action::Pin(uri) => {
                if !self.settings.pins.contains(&uri) {
                    self.settings.pins.insert(0, uri);
                    self.mark_settings_dirty();
                }
            }
            Action::Unpin(uri) => {
                self.settings.pins.retain(|p| p != &uri);
                self.mark_settings_dirty();
            }
            Action::CreateFolder { name, add } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                let id = model::random_hex(16);
                // A playlist is in one folder at a time.
                if let Some(p) = &add {
                    for f in self.settings.folders.iter_mut() {
                        f.playlists.retain(|x| x != p);
                    }
                }
                self.settings.folders.push(crate::config::Folder { id, name, playlists: add.into_iter().collect() });
                self.mark_settings_dirty();
            }
            Action::RenameFolder { id, name } => {
                if name.trim().is_empty() {
                    return;
                }
                if let Some(f) = self.settings.folders.iter_mut().find(|f| f.id == id) {
                    f.name = name.trim().to_string();
                }
                self.mark_settings_dirty();
            }
            Action::DeleteFolder(id) => {
                self.settings.folders.retain(|f| f.id != id);
                self.mark_settings_dirty();
                self.outbox.push(CoreEvent::Left { route: Route::Folder { id } });
            }
            Action::MoveToFolder { playlist, folder } => {
                for f in self.settings.folders.iter_mut() {
                    f.playlists.retain(|p| p != &playlist);
                }
                if let Some(fid) = folder
                    && let Some(f) = self.settings.folders.iter_mut().find(|f| f.id == fid)
                {
                    f.playlists.push(playlist);
                }
                self.mark_settings_dirty();
            }
            Action::Invalidate(k) => self.invalidate(&k),
            Action::RestartEngine => {
                self.engine = None;
                self.engine_retry = None;
                self.engine_launching = false;
                self.start_engine();
            }
            Action::RescanLocal => {
                librespot_playback::local_file::set_directories(self.settings.local_dirs.clone());
                self.local_links.clear();
                if self.settings.local_dirs.is_empty() {
                    self.local_tracks = None;
                } else if self.local_scanning {
                    self.local_rescan = true;
                } else {
                    self.scan_local();
                }
            }
            Action::Download { uri, name } => self.download(uri, name),
            Action::RemoveDownload(uri) => {
                self.downloads.remove(Some(&uri), self.engine.as_ref().and_then(|e| e.session.cache().cloned()));
            }
            Action::RemoveAllDownloads => {
                self.downloads.remove(None, self.engine.as_ref().and_then(|e| e.session.cache().cloned()));
            }
            Action::ClearAudioCache => match self.engine.as_ref().and_then(|e| e.session.cache().cloned()) {
                Some(cache) => {
                    let d = self.downloads.clone();
                    std::thread::spawn(move || d.prune(&cache, &crate::engine::audio_cache_dir(), 0));
                    self.toast("Cache cleared");
                }
                None => self.toast("Still connecting to Spotify. Try again in a moment."),
            },
            Action::PlayLocalPlaylist { id, index } => {
                if self.local_tracks.is_none() {
                    self.pending_local = Some(Action::PlayLocalPlaylist { id, index });
                    self.scan_local();
                    self.toast("Reading your local files…");
                    return;
                }
                let Some(p) = self.playlists.get(&id).cloned() else { return };
                let all = self.local_tracks.clone().unwrap_or_default();
                let by_path: HashMap<&std::path::Path, &LocalTrack> = all.iter().map(|t| (t.path.as_path(), t)).collect();
                let tracks: Vec<LocalTrack> = p.tracks.iter().filter_map(|path| by_path.get(path.as_path()).map(|t| (*t).clone())).collect();
                if tracks.is_empty() {
                    self.toast("None of this playlist's files are in your local folders");
                } else {
                    let start = index.min(tracks.len() - 1);
                    self.play_local_list(tracks, start);
                }
            }
            Action::DeleteLocalPlaylist(id) => match self.playlists.delete(&id) {
                Ok(()) => self.toast("Playlist deleted (undo is on this page)"),
                Err(e) => self.toast(e),
            },
            Action::AddToLocalPlaylist { id, paths } => {
                let paths: Vec<std::path::PathBuf> = paths.into_iter().map(Into::into).collect();
                let name = self.playlists.get(&id).map(|p| p.name.clone()).unwrap_or_default();
                match self.playlists.append(&id, &paths) {
                    Ok(0) => self.toast(if paths.len() == 1 { format!("Already in “{name}”") } else { format!("Those songs are already in “{name}”") }),
                    Ok(1) => self.toast(format!("Added to “{name}”")),
                    Ok(n) => self.toast(format!("Added {n} songs to “{name}”")),
                    Err(e) => self.toast(e),
                }
            }
            Action::CreateLocalPlaylist { name, paths } => match self.playlists.create(&name, "", paths.into_iter().map(Into::into).collect(), None) {
                Ok((id, _)) => {
                    self.navigate(Route::LocalPlaylist { id });
                }
                Err(e) => self.toast(e),
            },
            Action::RemoveFromLocalPlaylist { id, index } => match self.playlists.remove_at(&id, index) {
                Ok(()) => self.toast("Removed from playlist (undo is on its page)"),
                Err(e) => self.toast(e),
            },
            Action::UndoPlaylistChange => match self.playlists.undo() {
                Ok(Some(what)) => self.toast(what),
                Ok(None) => self.toast("Nothing to undo"),
                Err(e) => self.toast(e),
            },
            Action::SignIn => self.sign_in(),
            Action::CancelSignIn => {
                self.signin_gen += 1;
                self.auth_state = AuthState::SignedOut;
                auth::cancel_sign_in();
            }
            Action::LogOut => self.log_out(),
            Action::Toast(t) => self.toast(t),
            Action::Lyrics(a) => self.lyrics_action(a),
            Action::ThemePreview { key, value } => self.theme.preview(&key, &value),
            Action::ThemeSet { key, value } => {
                if let Err(e) = self.theme.set(&key, &value) {
                    {
                        log::warn!("theme: {e}");
                        self.toast("That value can't be used. Nothing changed.");
                    }
                }
            }
            Action::SaveSong(uri) => {
                if !uri.starts_with("spotify:track:") {
                    self.toast("Only Spotify songs can be downloaded");
                    return;
                }
                if self.demo {
                    self.toast("Saving songs needs the network (not in the demo)");
                    return;
                }
                let Some(api) = self.api.clone() else { return };
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    tx.send(match api.get(&format!("/tracks/{}", id_of(&uri))).await {
                        Ok(item) => Msg::SaveSongItem(item),
                        Err(e) => Msg::Failed(e.to_string()),
                    });
                });
            }
            Action::SaveCollection { uri, name } => self.save_collection(uri, name),
            Action::StopSaveSet(uri) => self.stop_save_set(&uri),
            Action::ShareBrowse(peer) => self.share_browse(peer),
            Action::ShareGet { peer, ids } => self.share_get(peer, ids),
            Action::AiSend(t) => self.ai_send(t),
            Action::AiCancel => self.ai_cancel(),
            Action::AiRetry => self.ai_retry(),
            Action::AiNewChat => {
                self.ai_cancel();
                self.ai.clear();
            }
            Action::AiSignIn { reconsent, new_account } => self.ai_sign_in(reconsent, new_account),
            Action::AiCancelSignIn => self.ai_cancel_sign_in(),
            Action::AiSignOut => self.ai_sign_out(),
            Action::AiSelectAccount(id) => self.ai_select_account(id),
            Action::AiApprove(id) => self.ai_approve(&id),
            Action::AiDecline(id) => self.ai_decline(&id),
            Action::AiSaveDraft { draft, replace } => self.ai_save_draft(&draft, replace),
            Action::AiDiscardDraft(id) => self.ai_discard_draft(&id),
            Action::AiEditDraft { draft, edit } => self.ai_edit_draft(&draft, edit),
            Action::AiUndo => self.ai_undo_last(),
            Action::AiRefreshModels => {
                self.ai.models_for.clear();
                self.ai_fetch_models();
            }
            Action::AiSetModel(slug) => self.set_config("assistant.model", slug.into()),
            Action::AiSetBilling(api_key) => {
                self.ai_cancel();
                self.set_config("assistant.billing", if api_key { crate::ai::Billing::ApiKey } else { crate::ai::Billing::ChatGptPlan }.name().into());
                self.ai.models = crate::ai::Models::Unknown;
                self.ai.models_for.clear();
            }
            Action::AiEnable(on) => {
                if !on {
                    self.ai_cancel();
                }
                self.set_config("assistant.enabled", on.into());
            }
            Action::AiForgetApiKey => self.ai_forget_api_key(),
            Action::AiDismissWelcome => self.ai_extra.welcome = false,
            Action::AiLimitAcknowledged => self.ai.limit_reached = false,
            Action::AiUseSpotify(on) => {
                self.ai_cancel();
                self.set_config("assistant.spotify", on.into());
            }
            Action::AppleMusicPairing(file) => self.apple_music_import_pairing(file),
            Action::AppleMusicForgetPairing => self.apple_music_forget_pairing(),
            Action::AppleMusicAdd { paths, playlist } => self.apple_music_add(paths, playlist),
            Action::AppleMusicStop => self.apple_music_stop(),
            Action::AppleMusicCheck => self.apple_music_check(),
            Action::AppleMusicRestore => self.apple_music_restore(),
            Action::MoveInQueue { uri, from, to } => self.move_in_queue(uri, from, to),
            Action::QueueLocal(paths) => self.queue_local(paths),
            Action::SendToDevice { peer, paths } => self.send_to_device(peer, paths),
            Action::GetFromDevices(uris) => self.get_from_devices(uris),
            Action::GetMissingFromDevices(id) => {
                self.shares.missing_for = Some(id);
                self.get_missing_now();
            }
            Action::AddFiles { paths, target } => {
                let tx = self.tx.clone();
                let inbox = crate::paths::documents().join("Imported");
                self.rt.spawn_blocking(move || {
                    let paths: Vec<std::path::PathBuf> = paths.into_iter().map(Into::into).collect();
                    // A file waiting in a temporary folder would vanish: it's kept in Glass's folder.
                    let paths = crate::local::keep_temporary(paths, &inbox);
                    let tracks = crate::local::read_paths(&paths);
                    tx.send(Msg::FilesRead(target, paths, tracks));
                });
            }
            Action::FilesAdded(dirs) => self.files_added(dirs.into_iter().map(Into::into).collect()),
            Action::AddLocalToPlaylist { playlist, paths } => {
                let all = self.local_tracks.clone().unwrap_or_default();
                let uris: Vec<String> = paths.iter().filter_map(|p| all.iter().find(|t| t.path.to_string_lossy() == p.as_str())).map(crate::local::spotify_uri).collect();
                if !uris.is_empty() {
                    self.apply(Action::AddToPlaylist { playlist, uris });
                }
            }
            Action::SaveTags { path, tags, cover } => self.save_tags(path, tags, cover),
            Action::RemoveSource(dir) => {
                self.settings.local_dirs.retain(|d| d.to_string_lossy() != dir);
                self.mark_settings_dirty();
                self.apply(Action::RescanLocal);
            }
            Action::ConfigSet { key, value } => {
                let v: toml_edit::Value = match value {
                    crate::bridge::ConfigValue::Bool(b) => b.into(),
                    crate::bridge::ConfigValue::Text(t) => t.into(),
                };
                self.set_config(&key, v);
            }
            Action::ThemeReset(table) => {
                let r = if table == "theme" { self.theme.reset_panel_values() } else { self.theme.reset_table(&table) };
                if let Err(e) = r {
                    {
                        log::warn!("theme: {e}");
                        self.toast("That value can't be used. Nothing changed.");
                    }
                }
            }
            Action::SetLibrarySort(v) => {
                self.settings.library_sort = v;
                self.mark_settings_dirty();
            }
            Action::SetLibraryView(v) => {
                self.settings.library_view = v;
                self.mark_settings_dirty();
            }
            Action::RememberSearch(q) => {
                // Desktop `remember_search`.
                let q = q.trim().to_string();
                if q.is_empty() || self.settings.recent_searches.first() == Some(&q) {
                    return;
                }
                self.settings.recent_searches.retain(|r| r != &q);
                self.settings.recent_searches.insert(0, q);
                self.settings.recent_searches.truncate(10);
                self.mark_settings_dirty();
            }
            Action::ClearRecentSearches => {
                self.settings.recent_searches.clear();
                self.mark_settings_dirty();
            }
        }
    }

    fn is_playing_anything(&self) -> bool {
        if self.local_file_active() {
            return self.local.as_ref().is_some_and(|l| l.is_playing());
        }
        self.pb.is_playing
    }

    /// Desktop `Action::TogglePlay`.
    fn toggle_play(&mut self) {
        if let Some(v) = self.dj_voice.as_ref() {
            v.toggle();
            self.pb.is_playing = !v.is_paused();
            return;
        }
        if self.dj_waiting {
            self.dj_gen += 1;
            self.dj_waiting = false;
            self.pb.is_playing = false;
            return;
        }
        if self.local_file_active() {
            if let Some(l) = self.local.as_mut() {
                l.toggle();
            }
            return;
        }
        if self.replaced_dead_engine(false) && self.pb.is_playing {
            // It had stopped anyway; this was a Pause.
            self.pb.is_playing = false;
            return;
        }
        let playing = self.pb.is_playing;
        self.pb.progress_ms = self.pb.position();
        self.pb.progress_at = Some(Instant::now());
        self.pb.is_playing = !playing;
        if self.demo {
            return;
        }
        if !playing
            && self.pb.device.is_none()
            && let Some(item) = self.pb.item.clone()
        {
            let uri = s(&item, "uri").to_string();
            let mut body = if self.pb.context_uri.is_empty() { json!({ "uris": [uri] }) } else { json!({ "context_uri": self.pb.context_uri, "offset": { "uri": uri } }) };
            body["position_ms"] = json!(self.pb.position());
            self.play_body(body);
            return;
        }
        if self.is_local() && self.pb.item.is_none() {
            if let Some(d) = self.engine_device().map(str::to_string) {
                self.mutate(Method::PUT, "/me/player".into(), Some(json!({ "device_ids": [d], "play": true })), None, None, vec![], true);
            }
            return;
        }
        let path = if playing { "/me/player/pause" } else { "/me/player/play" };
        self.transport(
            |e| {
                let _ = if playing { e.spirc.pause() } else { e.spirc.play() };
            },
            Method::PUT,
            path,
        );
    }

    fn log_out(&mut self) {
        auth::forget_session();
        self.engine = None;
        self.api = None;
        self.me = None;
        self.data.clear();
        self.fetched_at.clear();
        self.refreshing.clear();
        self.saved.clear();
        self.queue = None;
        self.queue_error = None;
        self.devices = None;
        self.pb = Playback { volume: self.settings.volume, repeat: "off".into(), ..Default::default() };
        self.saved_wanted.clear();
        self.saved_pending.clear();
        self.used_at.clear();
        self.pending_play = None;
        self.engine_uri.clear();
        self.dj_voice = None;
        self.engine_retry = None;
        self.engine_launching = false;
        self.engine_status = "Not started".into();
        self.auth_state = AuthState::SignedOut;
        self.outbox.push(CoreEvent::SignedOut);
    }

    /// Desktop `save`: like, save or follow, 50 a request; hearts change at once and back on failure.
    fn save(&mut self, mut uris: Vec<String>, save: bool) {
        let Some(kind) = uris.first().map(|u| kind_of(u).to_string()) else { return };
        uris.retain(|u| kind_of(u) == kind);
        if kind == "playlist" {
            uris.truncate(1);
        }
        let n = uris.len();
        let library = if save { "Added to Your Library" } else { "Removed from Your Library" };
        let follow = if save { "Followed" } else { "Unfollowed" };
        let (inval, toast) = match (kind.as_str(), save) {
            ("track", true) => ("/me/tracks", if n == 1 { "Added to Liked Songs".to_string() } else { format!("Added {n} songs to Liked Songs") }),
            ("track", false) => ("/me/tracks", if n == 1 { "Removed from Liked Songs".to_string() } else { format!("Removed {n} songs from Liked Songs") }),
            ("episode", true) => ("/me/episodes", if n == 1 { "Saved to Your Episodes".to_string() } else { format!("Saved {n} episodes to Your Episodes") }),
            ("episode", false) => ("/me/episodes", if n == 1 { "Removed from Your Episodes".to_string() } else { format!("Removed {n} episodes from Your Episodes") }),
            ("album", _) => ("/me/albums", library.to_string()),
            ("audiobook", _) => ("/me/audiobooks", library.to_string()),
            ("playlist", _) => ("/me/playlists", library.to_string()),
            ("show", _) => ("/me/shows", follow.to_string()),
            ("artist", _) => ("/me/following", follow.to_string()),
            _ => return,
        };
        let requests: Vec<(String, Option<Value>, String)> = uris
            .chunks(50)
            .map(|chunk| {
                let ids = chunk.iter().map(|u| id_of(u)).collect::<Vec<_>>().join(",");
                let path = match kind.as_str() {
                    "artist" => format!("/me/following?type=artist&ids={ids}"),
                    "playlist" => format!("/playlists/{ids}/followers"),
                    _ => format!("{inval}?ids={ids}"),
                };
                let body = (kind == "playlist" && save).then(|| json!({ "public": true }));
                (path, body, format!("/me/library?uris={}", enc(&chunk.join(","))))
            })
            .collect();
        for u in &uris {
            self.set_saved(u.clone(), save);
        }
        if self.demo {
            self.toast(toast);
            return;
        }
        let Some(api) = self.api.clone() else { return };
        let (tx, m, inval) = (self.tx.clone(), if save { Method::PUT } else { Method::DELETE }, inval.to_string());
        self.rt.spawn(async move {
            let mut done = 0;
            let mut failed = None;
            for (path, body, fallback) in requests {
                let mut r = api.send(m.clone(), &path, body).await;
                if r.as_ref().is_err_and(|e| e.to_string().starts_with("404")) {
                    r = api.send(m.clone(), &fallback, None).await;
                }
                match r {
                    Ok(_) => done = (done + 50).min(uris.len()),
                    Err(e) => {
                        failed = Some(e.to_string());
                        break;
                    }
                }
            }
            if done > 0 {
                tx.send(Msg::Done { toast: failed.is_none().then_some(toast), invalidate: vec![inval], refresh_player: false, navigate: None });
            }
            if let Some(e) = failed {
                tx.send(Msg::Saved(uris[done..].iter().map(|u| (u.clone(), !save)).collect()));
                tx.send(Msg::Failed(e));
            }
        });
    }

    fn download(&mut self, uri: String, name: String) {
        if !self.is_premium() {
            self.toast("Downloading to play offline needs Spotify Premium. Share › Download as M4A or MP3 keeps a copy without it.");
            return;
        }
        let Some(api) = self.api.clone() else { return };
        let d = self.downloads.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let msg = match collection_items(&api, &uri).await {
                Ok(items) => {
                    let n = items.len();
                    d.add(&uri, &name, items);
                    Msg::Done { toast: Some(format!("Downloading {n} {}", if n == 1 { "item" } else { "items" })), invalidate: vec![], refresh_player: false, navigate: None }
                }
                Err(e) => Msg::Failed(e.to_string()),
            };
            tx.send(msg);
        });
    }

    /// Desktop `sync_download`: a downloaded playlist or Liked Songs that changed gets its new songs too.
    fn sync_download(&self, key: &str, v: &Value) {
        let (all, path) = key.split_once(':').unwrap_or(("", key));
        let cap = all.strip_prefix("ALL").and_then(|c| c.parse::<u64>().ok()).unwrap_or(u64::MAX);
        if (model::items(v).len() as u64) < model::u(v, "total").min(cap) {
            return;
        }
        let uri = if path.starts_with("/me/tracks") {
            format!("spotify:user:{}:collection", self.user_id())
        } else if let Some(id) = path.strip_prefix("/playlists/").and_then(|r| r.split('/').next()).filter(|_| path.contains("/tracks")) {
            format!("spotify:playlist:{id}")
        } else {
            return;
        };
        if !self.downloads.is_kept(&uri) {
            return;
        }
        let items: Vec<String> =
            model::items(v).iter().map(|i| s(model::playlist_entry(i), "uri").to_string()).filter(|u| u.starts_with("spotify:track:") || u.starts_with("spotify:episode:")).collect();
        let name = self.downloads.collections().into_iter().find(|(u, _)| *u == uri).map(|(_, n)| n).unwrap_or_default();
        if self.downloads.progress(&uri).map(|(_, total)| total) != Some(items.len()) || items.iter().any(|u| !self.downloads.has_file(u)) {
            self.downloads.add(&uri, &name, items);
        }
    }

    // ------------------------------------------------------------ local files

    /// Plays local files (pausing Spotify first); desktop `play_local_list`.
    pub fn play_local_list(&mut self, tracks: impl Into<Arc<Vec<LocalTrack>>>, index: usize) -> bool {
        let tracks = tracks.into();
        if self.local.is_none() {
            self.local = LocalPlayer::new();
        }
        if self.is_local() {
            if let Some(e) = self.engine.as_ref() {
                let _ = e.spirc.pause();
            }
        } else if self.pb.is_playing && !self.demo {
            self.mutate(Method::PUT, format!("/me/player/pause{}", self.device_param(true)), None, None, None, vec![], true);
        }
        self.dj_cancel();
        self.pb.is_playing = false;
        match self.local.as_mut() {
            Some(l) => {
                l.play(tracks, index);
                if self.settings.now_playing_on_play {
                    self.outbox.push(CoreEvent::ShowNowPlaying);
                }
                true
            }
            None => {
                self.toast("Can't play audio right now.");
                false
            }
        }
    }

    /// Which local file a playlist's `spotify:local:` entry is (desktop `local_link`).
    pub fn local_link(&mut self, uri: &str) -> Option<Option<LocalTrack>> {
        let tracks = self.local_tracks.as_ref()?;
        let i = match self.local_links.get(uri) {
            Some(i) => *i,
            None => *self.local_links.entry(uri.to_string()).or_insert_with(|| crate::local::find(tracks, uri)),
        };
        Some(i.and_then(|i| tracks.get(i).cloned()))
    }

    pub fn scan_local(&mut self) {
        if self.local_scanning || (self.demo && !crate::demo::real_folders()) {
            return;
        }
        self.local_scanning = true;
        let dirs = self.settings.local_dirs.clone();
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("glass-scan".into())
            .spawn(move || tx.send(Msg::LocalScanned(crate::local::scan(&dirs))))
            .ok();
    }

    pub fn clear_local_links(&mut self) {
        self.local_links.clear();
    }

    fn set_local_tracks(&mut self, t: Arc<Vec<LocalTrack>>) {
        self.local_tracks = Some(t);
        self.local_links.clear();
        self.share_library_changed();
    }

    // ------------------------------------------------------------ auth/engine

    fn resume_session(&self) {
        let (tx, id) = (self.tx.clone(), self.settings.client_id.clone());
        self.rt.spawn(async move {
            let r = Auth::resume(&id).await.map(Arc::new).map_err(|e| format!("{e:#}"));
            tx.send(Msg::SignedIn(0, r));
        });
    }

    /// Starts a sign-in: the core listens for Spotify's redirect, then Swift opens
    /// the page (CoreEvent::OpenSignIn) in the system's web sign-in sheet.
    pub fn sign_in(&mut self) {
        if self.demo {
            return;
        }
        auth::cancel_sign_in();
        self.auth_state = AuthState::SigningIn;
        self.auth_error = None;
        self.signin_gen += 1;
        let generation = self.signin_gen;
        let (tx, id) = (self.tx.clone(), self.settings.client_id.clone());
        self.rt.spawn(async move {
            // A listener from a sign-in just cancelled may still be closing.
            let mut last = None;
            for _ in 0..10 {
                match crate::signin::begin(&id).await {
                    Ok(p) => {
                        tx.send(Msg::SignInStarted(generation, Ok(p)));
                        return;
                    }
                    Err(e) => last = Some(e),
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            tx.send(Msg::SignInStarted(generation, Err(last.map(|e| format!("{e:#}")).unwrap_or_default())));
        });
    }

    /// Now Playing's queue reordered by dragging: Local Files' own queue, or the one on
    /// this iPhone's Connect device (Spotify's Web API can't reorder a queue, so not
    /// when another device plays).
    fn move_in_queue(&mut self, uri: String, from: usize, to: usize) {
        if self.local_file_active() {
            if let Some(l) = self.local.as_mut() {
                l.move_upcoming(from, to);
            }
            return;
        }
        let here = self.is_local() || self.demo;
        let Some(list) = self.upcoming.as_mut().filter(|_| here) else {
            self.toast("You can reorder the queue when music plays on this iPhone");
            return;
        };
        // At once here (the device's own list confirms it in a moment).
        if !move_upcoming(list, &uri, from, to) {
            return;
        }
        if self.replaced_dead_engine(true) {
            return;
        }
        if let Some(e) = self.engine.as_ref()
            && let Err(err) = e.move_next(&uri, from, to)
        {
            // Only a gone engine refuses: a new one carries on (its queue comes back fresh).
            log::warn!("playback engine refused a move ({err}); starting a new one");
            self.engine_lost(true);
        }
    }

    /// Local Files songs swiped into the queue while Local Files plays.
    fn queue_local(&mut self, paths: Vec<String>) {
        let all = self.local_tracks.clone().unwrap_or_default();
        let tracks: Vec<crate::local::LocalTrack> = paths.iter().filter_map(|p| all.iter().find(|t| t.path == std::path::Path::new(p)).cloned()).collect();
        let Some(l) = self.local.as_mut().filter(|l| l.active) else {
            self.toast("Play something from Local Files to queue its songs");
            return;
        };
        let n = l.queue_songs(tracks);
        self.toast(if n == 1 { "Added to queue".to_string() } else { format!("Added {n} songs to the queue") });
    }

    /// This iPhone's Connect device is gone (its session or connection died, e.g. after
    /// a long time in the background or a network change): start a new one. With
    /// `resume`, what was playing here carries on where it was once it's up.
    pub(crate) fn engine_lost(&mut self, resume: bool) {
        let playing_here = self.is_local() && self.pb.is_playing && self.engine.is_some();
        let body = if resume && playing_here { self.resume_body() } else { None };
        self.engine_status = "Reconnecting…".into();
        // Not a device any more: a Play meanwhile waits for the engine (and resumes).
        if self.is_local() {
            self.pb.device = None;
        }
        self.engine = None;
        self.engine_uri.clear();
        self.upcoming = None;
        self.engine_launching = false;
        self.start_engine();
        match body {
            Some(body) => {
                log::info!("engine lost while playing; resuming once it's back");
                self.pending_play = Some(PendingPlay { body, shuffle: Some(self.pb.shuffle), quiet: true });
            }
            None if playing_here => self.pb.is_playing = false,
            None => {}
        }
    }

    /// Plays the song that's on from where it is, in its context (what a reconnect, or
    /// Clear queue, starts again).
    pub(crate) fn resume_body(&self) -> Option<Value> {
        let item = self.pb.item.as_ref()?;
        let uri = s(item, "uri");
        if uri.is_empty() {
            return None;
        }
        let pos = self.pb.position();
        Some(if self.pb.context_uri.is_empty() {
            json!({ "uris": [uri], "position_ms": pos })
        } else {
            json!({ "context_uri": self.pb.context_uri, "offset": { "uri": uri }, "position_ms": pos })
        })
    }

    /// Before a command for this iPhone's player: a dead engine (one whose loss
    /// hasn't arrived yet) is replaced instead. True when it was.
    fn replaced_dead_engine(&mut self, resume: bool) -> bool {
        if self.is_local() && self.engine.as_ref().is_some_and(|e| !e.running()) {
            self.engine_lost(resume);
            return true;
        }
        false
    }

    fn start_engine(&mut self) {
        let Some(api) = self.api.clone() else { return };
        if !self.is_premium() {
            self.engine_status = NEEDS_PREMIUM.into();
            return;
        }
        if self.engine_launching {
            return;
        }
        self.engine_launching = true;
        self.engine_status = "Connecting…".into();
        let settings = self.settings.clone();
        let dsp = self.dsp.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let ev_tx = tx.clone();
            let r = async {
                let token = api.auth().bearer().await?;
                Engine::start(&settings, token, dsp, move |e| ev_tx.send(Msg::Engine(e))).await
            }
            .await;
            tx.send(Msg::EngineReady(r.map_err(|e| e.to_string())));
        });
    }

    // ------------------------------------------------------------ per-tick logic

    /// Desktop `tick`, minus drawing.
    pub fn tick(&mut self) {
        if self.auth_state == AuthState::SignedIn {
            self.poll();
            if let Some((at, n)) = self.engine_retry
                && self.engine.is_none()
                && Instant::now() >= at
            {
                self.engine_retry = Some((Instant::now() + Duration::from_secs(3600), n));
                self.start_engine();
            }
        } else if let Some((at, n)) = self.resume_retry
            && self.signin_gen == 0
            && Instant::now() >= at
        {
            self.resume_retry = Some((Instant::now() + Duration::from_secs(3600), n));
            self.resume_session();
        }
        if let Some(l) = self.local.as_mut() {
            l.tick();
        }
        if self.local.as_ref().is_some_and(|l| !l.active) {
            self.local = None;
        }
        crate::dj::SHARED.voice_on.store(self.settings.dj_voice, std::sync::atomic::Ordering::Relaxed);
        if self.dj_voice.as_ref().is_some_and(|v| v.done()) {
            self.dj_resume();
        }
        self.theme.poll();
        if self.config.poll() {
            self.refresh_lyrics_client();
        }
        self.sync_lyrics();
        self.sync_translation();
        self.sync_album_colors();
        self.measure_sizes();
        if self.auth_state == AuthState::SignedIn {
            self.keep_sharing();
            self.get_missing_now();
        }
        self.ai_tick();
        self.flush_saved_queries();
        let dsp = DspState { eq_enabled: self.settings.eq_enabled, gains: self.settings.eq_gains, mono: self.settings.mono };
        if self.dsp_sent != Some(dsp) {
            self.dsp_sent = Some(dsp);
            self.dsp.set(dsp);
        }
        self.maybe_skip_explicit();
        if let Some(t) = self.settings_dirty
            && t.elapsed() > Duration::from_millis(800)
        {
            self.save_now();
        }
        if self.last_prune.elapsed() > Duration::from_secs(600) {
            let cache = self.engine.as_ref().and_then(|e| e.session.cache().cloned());
            self.prune_cache(cache);
        }
        if self.last_evict.elapsed() > Duration::from_secs(30) {
            self.last_evict = Instant::now();
            self.evict_unused();
            if let Some(api) = self.api.clone() {
                self.rt.spawn(async move { api.trim() });
            }
        }
    }

    fn prune_cache(&mut self, cache: Option<Arc<librespot_core::cache::Cache>>) {
        self.last_prune = Instant::now();
        let Some(cache) = cache else { return };
        let d = self.downloads.clone();
        let limit = self.settings.cache_limit_mb * 1024 * 1024;
        std::thread::spawn(move || d.prune(&cache, &crate::engine::audio_cache_dir(), limit));
    }

    /// Desktop `poll`: the account, the player state (pushed by Spotify; this is the
    /// fallback), the queue and devices while they're on screen.
    fn poll(&mut self) {
        let Some(api) = self.api.clone() else { return };
        let now = Instant::now();
        if self.me.is_none() && now >= self.next_me_poll {
            self.next_me_poll = now + Duration::from_secs(15);
            let (tx, a, generation) = (self.tx.clone(), api.clone(), self.data_gen);
            self.rt.spawn(async move {
                let r = a.get("/me").await.map(Arc::new).map_err(|e| e.to_string());
                tx.send(Msg::Data { key: "/me".into(), result: r, generation });
            });
        }
        if now >= self.next_poll && !self.player_polling {
            self.player_polling = true;
            self.next_poll = now
                + if self.pb.is_playing {
                    Duration::from_secs(15)
                } else if self.foreground {
                    Duration::from_secs(30)
                } else {
                    Duration::from_secs(60)
                };
            let (tx, a) = (self.tx.clone(), api.clone());
            self.rt.spawn(async move {
                let failed = Answer::new(tx.clone(), Msg::Player(Err("the request failed".into())));
                let r = a.get("/me/player?additional_types=episode").await.map_err(|e| e.to_string());
                failed.disarm();
                tx.send(Msg::Player(r));
            });
        }
        let left_ms = self.pb.item.as_ref().map(|i| model::u(i, "duration_ms")).unwrap_or(0).saturating_sub(self.pb.position());
        let ahead = self.settings.album_colors && self.pb.is_playing && left_ms > 0 && left_ms < 25_000 && self.queue_read_for != self.pb.uri();
        if ahead {
            self.queue_read_for = self.pb.uri().to_string();
            self.next_queue_poll = now;
        }
        let want_queue = self.panel_queue || ahead;
        if want_queue && now >= self.next_queue_poll {
            self.next_queue_poll = now + Duration::from_secs(8);
            let (tx, a) = (self.tx.clone(), api.clone());
            self.rt.spawn(async move {
                let r = a.get("/me/player/queue").await.map_err(|e| e.to_string());
                tx.send(Msg::Queue(r));
            });
        }
        let want_devices = self.panel_devices;
        if want_devices && !self.devices_shown {
            self.next_devices_poll = now;
        }
        self.devices_shown = want_devices;
        if now >= self.next_devices_poll {
            self.next_devices_poll = now + Duration::from_secs(if want_devices { 5 } else { 60 });
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Ok(v) = api.get("/me/player/devices").await {
                    tx.send(Msg::Devices(v));
                }
            });
        }
    }

    // ------------------------------------------------------------ messages

    pub fn handle(&mut self, msg: Msg) {
        if self.api.is_none()
            && !self.demo
            && matches!(msg, Msg::Data { .. } | Msg::Player(_) | Msg::Queue(_) | Msg::Devices(_) | Msg::Saved(..) | Msg::SavedFailed(..) | Msg::EngineReady(_) | Msg::Engine(_))
        {
            return;
        }
        match msg {
            Msg::Act(a) => self.apply(a),
            Msg::Subscribe(r) => {
                self.views.insert(crate::views::route_key(&r), r);
            }
            Msg::Unsubscribe(r) => {
                let k = crate::views::route_key(&r);
                self.views.remove(&k);
                self.published.pages.remove(&k);
            }
            Msg::Panels { queue, devices, now_playing } => {
                if queue && !self.panel_queue {
                    self.next_queue_poll = Instant::now();
                }
                self.panel_queue = queue;
                self.panel_devices = devices;
                self.on_now_playing = now_playing;
            }
            Msg::Foreground(f) => {
                // Back in front: catch up with what other devices did meanwhile.
                if f && !self.foreground {
                    self.next_poll = Instant::now();
                    if self.panel_queue {
                        self.next_queue_poll = Instant::now();
                    }
                    // Its connection may have died while iOS had Glass suspended.
                    self.replaced_dead_engine(false);
                }
                self.foreground = f;
            }
            Msg::WantSaved(uris) => {
                for u in uris {
                    let _ = self.is_saved(&u);
                }
            }
            Msg::SetSettings(new) => self.set_settings(*new),
            Msg::SetModel(m) => {
                let new = crate::settings_app::apply_model(&self.settings, *m);
                self.set_settings(new);
            }
            Msg::RouteChanged { gone } => self.route_changed(gone),
            Msg::Interrupted { began, resume } => {
                if began {
                    self.interrupted_playing = self.playing_here();
                    if self.interrupted_playing {
                        self.apply(Action::Pause);
                    }
                } else {
                    // The system output may have been reconfigured: reopen it before playing.
                    crate::audio_out::changed(false);
                    if let Some(l) = self.local.as_mut() {
                        l.follow_output();
                    }
                    if std::mem::take(&mut self.interrupted_playing) && resume {
                        self.apply(Action::Play);
                    }
                }
            }
            Msg::SignInStarted(g, _) if g != self.signin_gen => {}
            Msg::SignInStarted(g, Ok(p)) => {
                self.outbox.push(CoreEvent::OpenSignIn { request: crate::types::SignInRequest { url: p.url.clone(), callback_scheme: crate::signin::CALLBACK_SCHEME.into() } });
                let (tx, id) = (self.tx.clone(), self.settings.client_id.clone());
                self.rt.spawn(async move {
                    let r = crate::signin::finish(p, &id).await.map(Arc::new).map_err(|e| format!("{e:#}"));
                    tx.send(Msg::SignedIn(g, r));
                });
            }
            Msg::SignInStarted(_, Err(e)) => {
                log::warn!("sign-in didn't start: {e}");
                self.auth_state = AuthState::SignedOut;
                self.auth_error = Some("Couldn't open Spotify's sign-in. Try again.".into());
            }
            Msg::SignedIn(g, _) if g != self.signin_gen => {}
            Msg::SignedIn(_, Ok(auth)) => {
                self.resume_retry = None;
                self.auth_error = None;
                let tx = self.tx.clone();
                let pushed: crate::sp::Changed = Arc::new(move || tx.send(Msg::PlayerPushed));
                self.api = Some(Api::new(auth, &self.settings.proxy, self.log.clone()).on_player_change(pushed));
                self.auth_state = AuthState::SignedIn;
                let (tx, api, generation) = (self.tx.clone(), self.api.clone().unwrap(), self.data_gen);
                // The engine starts once this says what kind of account it is
                // (`account_known`): Spotify streams only to Premium accounts.
                self.rt.spawn(async move {
                    let r = api.get("/me").await.map(Arc::new).map_err(|e| e.to_string());
                    tx.send(Msg::Data { key: "/me".into(), result: r, generation });
                });
            }
            Msg::SignedIn(g, Err(e)) => {
                let refused = ["no saved session", "invalid_grant", "invalid_client", "invalid_request", "unauthorized_client"].iter().any(|w| e.contains(w));
                let tries = self.resume_retry.map_or(0, |(_, n)| n);
                if g == 0 && !refused && tries < 8 {
                    let wait = [2, 5, 15, 30][(tries as usize).min(3)];
                    self.resume_retry = Some((Instant::now() + Duration::from_secs(wait), tries + 1));
                    self.auth_error = Some("Can't reach Spotify. Trying again…".into());
                } else {
                    self.resume_retry = None;
                    log::warn!("sign-in: {e}");
                    // Cancelling the sheet, or no saved session at launch, isn't an error worth showing.
                    self.auth_error = if e == "cancelled" || e.contains("no saved session") {
                        None
                    } else if refused {
                        Some("Your Spotify session ended. Sign in again.".into())
                    } else if crate::errors::offline(&e.to_lowercase()) {
                        Some("Can't reach Spotify. Check your connection and try again.".into())
                    } else {
                        Some("Couldn't sign in. Try again.".into())
                    };
                }
                self.auth_state = AuthState::SignedOut;
            }
            Msg::Data { key, result, generation } => {
                let was_refresh = self.refreshing.remove(&key);
                match result {
                    Ok(v) => {
                        if key == "/me" {
                            let first = self.me.is_none();
                            self.me = Some((*v).clone());
                            if first {
                                self.account_known();
                            }
                        }
                        if key.starts_with("ALL") {
                            self.sync_download(&key, &v);
                        }
                        // Liked Songs and saved lists say what's saved without asking.
                        if key.starts_with("ALL10000:/me/tracks") || key.starts_with("/me/tracks?") {
                            for it in model::items(&v) {
                                let u = s(&it["track"], "uri").to_string();
                                if !u.is_empty() {
                                    self.set_saved(u, true);
                                }
                            }
                        }
                        let stale = self.invalidated.iter().any(|(g, needle)| *g > generation && key.contains(needle.as_str()));
                        if stale {
                            self.fetched_at.remove(&key);
                        } else {
                            self.fetched_at.insert(key.clone(), Instant::now());
                        }
                        self.data.insert(key, Load::Ready(v));
                    }
                    Err(_) if was_refresh => {
                        self.fetched_at.insert(key, Instant::now());
                    }
                    Err(e) => {
                        // Not known what kind of account this is: start as before (a free
                        // account's engine is let go once it is).
                        if key == "/me" && self.me.is_none() {
                            self.start_engine();
                        }
                        self.fetched_at.insert(key.clone(), Instant::now());
                        self.data.insert(key, Load::Failed(e));
                    }
                }
            }
            Msg::PlayerPushed => {
                let now = Instant::now();
                self.next_poll = self.next_poll.min(now);
                self.next_queue_poll = self.next_queue_poll.min(now);
                if self.devices_shown {
                    self.next_devices_poll = self.next_devices_poll.min(now);
                }
            }
            Msg::Player(r) => {
                self.player_polling = false;
                if let Ok(v) = r {
                    self.apply_player_state(v);
                }
            }
            Msg::Queue(Ok(v)) => {
                if self.queue_meta.len() > 400 {
                    self.queue_meta.clear();
                }
                for t in v.get("currently_playing").into_iter().chain(model::arr(&v, "queue").iter()) {
                    let uri = s(t, "uri");
                    if !uri.is_empty() {
                        self.queue_meta.insert(uri.to_string(), t.clone());
                    }
                }
                self.queue = Some(Arc::new(v));
                self.queue_error = None;
            }
            Msg::Queue(Err(e)) => {
                self.queue_error = Some(e);
                self.next_queue_poll = Instant::now() + Duration::from_secs(2);
            }
            Msg::SavedFailed(uris) => {
                for u in uris {
                    self.saved_pending.remove(&u);
                }
                self.saved_backoff = Some(Instant::now());
            }
            Msg::Devices(v) => self.devices = Some(v),
            Msg::Saved(pairs) => {
                for (u, b) in pairs {
                    self.saved_pending.remove(&u);
                    self.set_saved(u, b);
                }
            }
            Msg::EngineReady(Ok(_)) if !self.is_premium() => {
                self.engine_launching = false;
                self.engine_status = NEEDS_PREMIUM.into();
            }
            Msg::EngineReady(Ok(engine)) => {
                self.engine_launching = false;
                self.engine_retry = None;
                self.engine_status = format!("Connected · other devices see this iPhone as “{}”", self.settings.device_name);
                let tx = self.tx.clone();
                let _guard = self.rt.enter();
                self.downloads.run(engine.session.clone(), self.settings.streaming_quality, move || tx.send(Msg::Changed));
                self.prune_cache(engine.session.cache().cloned());
                let pending = self.pending_play.take();
                self.engine = Some(engine);
                self.devices = None;
                self.next_devices_poll = Instant::now() + Duration::from_secs(1);
                self.poll_soon();
                if let Some(p) = pending {
                    self.start_play(p);
                }
            }
            Msg::EngineReady(Err(e)) => {
                self.engine_launching = false;
                if self.pending_play.take().is_some() {
                    self.pb.is_playing = false;
                }
                let tries = self.engine_retry.map_or(0, |(_, n)| n);
                let wait = [5, 15, 30, 60, 120][(tries as usize).min(4)];
                self.engine_retry = Some((Instant::now() + Duration::from_secs(wait), tries + 1));
                log::warn!("playback engine didn't start: {e}");
                self.engine_status = format!("{ENGINE_FAILED} Trying again in {wait} s.");
                if tries == 0 {
                    self.toast("Can't play on this iPhone right now. Trying again…");
                }
            }
            Msg::Engine(ev) => self.handle_engine(ev),
            Msg::Done { toast, invalidate, refresh_player, navigate } => {
                if let Some(t) = toast {
                    self.toast(t);
                }
                for k in invalidate {
                    self.invalidate(&k);
                }
                if refresh_player {
                    self.poll_soon();
                }
                if let Some(r) = navigate {
                    self.navigate(r);
                }
            }
            Msg::Failed(e) => {
                // Plain words, or nothing when the player state resyncs by itself.
                if let Some(t) = crate::errors::plain(&e) {
                    self.toast(t);
                }
                self.poll_soon();
            }
            Msg::DjClips(generation, r) => self.dj_clips_ready(generation, r),
            Msg::LocalScanned(t) => {
                self.set_local_tracks(Arc::new(t));
                if let Some(play) = self.pending_local.take() {
                    self.apply(play);
                }
                self.local_scanning = false;
                if std::mem::take(&mut self.local_rescan) {
                    self.scan_local();
                }
            }
            Msg::Lyrics { generation, resolved } => self.lyrics_answered(generation, resolved),
            Msg::Palette(url, p) => self.palette_arrived(url, p),
            Msg::Measured { audio, lyrics } => {
                self.measured.audio_cache = Some(audio);
                self.measured.lyrics = lyrics;
                self.measured.at = Some(Instant::now());
                self.measured.running = false;
            }
            Msg::TagsSaved(path, r) => self.tags_saved(path, r),
            Msg::SaveSong(key, status) => self.song_save_moved(key, status),
            Msg::SaveSetItems { uri, name, kind, items, description, cover } => self.save_set(uri, name, kind, items, description, cover),
            Msg::SaveSongItem(item) => self.save_song(item),
            Msg::Share(e) => self.share_event(e),
            Msg::AppleMusic(u) => self.apple_music_update(u),
            Msg::Ai { generation, event } => self.ai_event(generation, event),
            Msg::AiWrote(w) => self.ai_wrote(w),
            Msg::AiRegistered(id) => self.ai_registered(id),
            Msg::AiSignedIn(r) => self.ai_signed_in(r),
            Msg::AiModels { key, result } => self.ai_models_arrived(key, result),
            Msg::AiSignedOut { confirmed } => self.ai_signed_out(confirmed),
            Msg::AiOpenSignIn(url) => self.outbox.push(CoreEvent::OpenAssistantSignIn { request: crate::types::SignInRequest { url, callback_scheme: crate::assistant_app::SIGNED_IN_SCHEME.into() } }),
            Msg::AiKeySaved(r) => self.ai_api_key_saved(r),
            #[cfg(debug_assertions)]
            Msg::DebugAiMock(base) => crate::assistant_app::use_mock(self, base),
            Msg::Activity(reply) => self.activity(reply),
            Msg::ClearRequestLog => self.log.clear(),
            Msg::ExportFile(which, reply) => {
                let _ = reply.send(self.export_file(which));
            }
            Msg::ImportFile(which, text, reply) => {
                let _ = reply.send(self.import_file(which, text));
            }
            Msg::Screen { route, assistant } => {
                self.ai_extra.screen = route;
                self.ai_extra.panel = assistant;
            }
            Msg::FilesRead(target, paths, tracks) => self.files_read(target, paths, tracks),
            Msg::SettingsOpen(open) => {
                self.settings_open = open;
                if open {
                    self.measured.at = None;
                }
            }
            Msg::LyricsSearch { generation, result } => self.lyrics_search_answered(generation, result),
            Msg::LyricsTranslated { key, result } => self.lyrics_translated(key, result),
            Msg::Changed => {}
            Msg::Shutdown => {}
        }
    }

    /// Settings changed in the app. Desktop applies most of them on its next frame
    /// (`tick` reads them); the few that need doing now are done here.
    fn set_settings(&mut self, new: Settings) {
        let old = std::mem::replace(&mut self.settings, new);
        if old.local_dirs != self.settings.local_dirs {
            self.settings.local_dirs = crate::local::tidy_sources(&self.settings.local_dirs);
            self.apply(Action::RescanLocal);
        }
        if old.volume != self.settings.volume && self.muted_volume.is_none() {
            let v = self.settings.volume;
            self.set_volume(v, true);
        }
        // What the engine is started with: a new one takes them at once (and carries
        // on with the song if it was playing here), rather than at some later restart.
        let n = &self.settings;
        let engine_changed = (old.streaming_quality, old.gapless, old.normalize, old.loudness, old.autoplay, &old.device_name, &old.proxy)
            != (n.streaming_quality, n.gapless, n.normalize, n.loudness, n.autoplay, &n.device_name, &n.proxy);
        if engine_changed && self.engine.is_some() && !self.demo {
            self.engine_lost(true);
        }
        self.mark_settings_dirty();
    }

    /// iOS's audio route changed (desktop `follow_output`): the engine and local
    /// player reopen on the new route; if the device that was playing went away
    /// (headphones unplugged, AirPods out of range), playback pauses rather than
    /// carrying on through the speaker.
    fn route_changed(&mut self, gone: bool) {
        let pause = gone && self.playing_here();
        crate::audio_out::changed(gone);
        if pause {
            self.apply(Action::Pause);
        }
        if let Some(l) = self.local.as_mut() {
            l.follow_output();
        }
    }

    /// Desktop `apply_player_state`.
    fn apply_player_state(&mut self, v: Value) {
        if v.is_null() || v.get("device").is_none() {
            self.pb.progress_ms = self.pb.position();
            self.pb.progress_at = Some(Instant::now());
            self.pb.device = None;
            self.pb.is_playing = false;
            return;
        }
        let prev_uri = self.pb.uri().to_string();
        let item = v.get("item").filter(|i| !i.is_null()).cloned();
        let here = self.engine_device().is_some_and(|e| v["device"]["id"].as_str() == Some(e));
        let stale = here && !self.engine_uri.is_empty() && item.as_ref().is_some_and(|i| s(i, "uri") != self.engine_uri);
        if !stale && (item.is_some() || self.pb.item.is_none()) {
            self.pb.item = item.map(Arc::new);
        }
        let fresh_local = self.is_local() && self.local_touched.is_some_and(|t| t.elapsed() < Duration::from_millis(2500));
        if !here && !fresh_local {
            self.pb.is_playing = model::b(&v, "is_playing");
            self.pb.progress_ms = model::u(&v, "progress_ms");
            self.pb.progress_at = Some(Instant::now());
        }
        if !fresh_local {
            self.pb.shuffle = model::b(&v, "shuffle_state");
            self.pb.repeat = s(&v, "repeat_state").to_string();
            if self.pb.repeat.is_empty() {
                self.pb.repeat = "off".into();
            }
        }
        if !stale {
            self.pb.context_uri = v.get("context").map(|c| s(c, "uri").to_string()).unwrap_or_default();
        }
        self.pb.device = v.get("device").cloned();
        // Playing here, the volume is the iPhone's (the engine reports what another
        // device sets): Spotify's copy can lag behind the side buttons and would pull
        // the iPhone's volume back.
        if !self.pb.volume_dragging
            && self.muted_volume.is_none()
            && !here
            && !self.local_file_active()
            && let Some(vp) = v["device"]["volume_percent"].as_u64()
        {
            self.pb.volume = vp as f32 / 100.0;
        }
        if self.pb.uri() != prev_uri {
            self.next_queue_poll = Instant::now();
            self.on_track_changed();
        }
    }

    /// Desktop `handle_engine`.
    fn handle_engine(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::SessionLost => {
                self.engine_lost(true);
                return;
            }
            EngineEvent::Upcoming(list) => {
                self.upcoming = Some(list);
                return;
            }
            EngineEvent::Unavailable if self.is_local() => {
                self.toast("This song isn't available");
                return;
            }
            EngineEvent::Dj { uri, say, ready, hold } => {
                self.dj_break(uri, say, ready, hold);
                return;
            }
            _ => {}
        }
        if self.is_local() && !matches!(ev, EngineEvent::Volume(_)) {
            self.local_touched = Some(Instant::now());
        }
        if !self.is_local() {
            if matches!(ev, EngineEvent::Playing { .. } | EngineEvent::Loading { .. }) {
                self.poll_soon();
            }
            return;
        }
        match ev {
            EngineEvent::Loading { uri } => {
                if uri != self.pb.uri() {
                    match self.known_item(&uri) {
                        Some(item) => self.show_track(item),
                        None => {
                            self.pb.progress_ms = 0;
                            self.pb.progress_at = Some(Instant::now());
                        }
                    }
                }
                self.engine_uri = uri;
            }
            EngineEvent::Track(item) => {
                self.engine_uri = s(&item, "uri").to_string();
                if self.engine_uri != self.pb.uri() {
                    self.show_track(item);
                }
                self.next_poll = self.next_poll.min(Instant::now() + Duration::from_millis(1500));
            }
            EngineEvent::Playing { uri, position_ms } => {
                self.pb.is_playing = true;
                self.pb.progress_ms = position_ms as u64;
                self.pb.progress_at = Some(Instant::now());
                self.engine_uri = uri;
            }
            EngineEvent::Paused { position_ms, .. } => {
                self.pb.is_playing = self.dj_voice.as_ref().is_some_and(|v| !v.is_paused()) || self.dj_waiting;
                self.pb.progress_ms = position_ms as u64;
                self.pb.progress_at = Some(Instant::now());
            }
            EngineEvent::Position { position_ms } => {
                self.pb.progress_ms = position_ms as u64;
                self.pb.progress_at = Some(Instant::now());
            }
            EngineEvent::EndOfTrack => {}
            EngineEvent::Stopped => self.poll_soon(),
            EngineEvent::Unavailable | EngineEvent::SessionLost | EngineEvent::Dj { .. } | EngineEvent::Upcoming(_) => {}
            EngineEvent::Volume(v) => {
                if !self.pb.volume_dragging {
                    self.pb.volume = v as f32 / u16::MAX as f32;
                }
            }
            EngineEvent::Shuffle(sh) => self.pb.shuffle = sh,
            EngineEvent::Repeat { context, track } => {
                self.pb.repeat = if track {
                    "track"
                } else if context {
                    "context"
                } else {
                    "off"
                }
                .into();
            }
        }
    }

    // ------------------------------------------------------------ demo playback

    /// In the demo nothing streams: playing a song shows it, from the top.
    fn demo_play(&mut self, body: &Value) {
        let index = body["offset"]["position"].as_u64().unwrap_or(0) as usize;
        let target = body["offset"]["uri"].as_str().or_else(|| body["uris"].get(index).and_then(Value::as_str)).unwrap_or("").to_string();
        if let Some(c) = body["context_uri"].as_str() {
            self.pb.context_uri = c.to_string();
        }
        let item = crate::demo::track(&target).or_else(|| crate::demo::first_of(self, s(body, "context_uri")));
        if let Some(item) = item {
            self.show_track(item);
        }
        self.pb.is_playing = true;
    }

    fn demo_skip(&mut self, by: i64) {
        if let Some(item) = crate::demo::step(self.pb.uri(), by) {
            self.show_track(item);
        }
    }
}

/// Desktop `add_items`: 100 a request.
async fn add_items(api: &Api, playlist: &str, uris: &[String]) -> (usize, Option<String>) {
    let mut added = 0;
    for chunk in uris.chunks(100) {
        let body = json!({ "uris": chunk });
        let mut r = api.send(Method::POST, &format!("/playlists/{playlist}/tracks"), Some(body.clone())).await;
        if r.as_ref().is_err_and(|e| e.to_string().starts_with("404")) {
            r = api.send(Method::POST, &format!("/playlists/{playlist}/items"), Some(body)).await;
        }
        match r {
            Ok(_) => added += chunk.len(),
            Err(e) => return (added, Some(e.to_string())),
        }
    }
    (added, None)
}

/// Desktop `queue_uris`: one request a song (Spotify takes one at a time), up to 100.
async fn queue_uris(api: &Api, dev: &str, uris: &[String]) -> Msg {
    const MAX: usize = 100;
    for u in uris.iter().take(MAX) {
        if let Err(e) = api.send(Method::POST, &format!("/me/player/queue?uri={}{}", enc(u), dev), None).await {
            return Msg::Failed(e.to_string());
        }
    }
    let toast = match uris.len() {
        1 => "Added to queue".to_string(),
        n if n > MAX => format!("Added the first {MAX} of {n} songs to the queue"),
        n => format!("Added {n} songs to the queue"),
    };
    Msg::Done { toast: Some(toast), invalidate: vec![], refresh_player: true, navigate: None }
}

/// Desktop `collection_items`: a collection's songs or episodes, in order, without local files.
pub async fn collection_items(api: &Api, uri: &str) -> anyhow::Result<Vec<String>> {
    let uris = |v: &Value, key: Option<&str>| -> Vec<String> {
        model::items(v)
            .iter()
            .map(|i| match key {
                Some("entry") => model::playlist_entry(i),
                Some(k) => &i[k],
                None => i,
            })
            .map(|t| s(t, "uri").to_string())
            .filter(|u| !u.is_empty() && !u.starts_with("spotify:local:"))
            .collect()
    };
    Ok(match kind_of(uri) {
        _ if uri.ends_with(":collection") => uris(&api.get_all("/me/tracks?limit=50", 10_000).await?, Some("track")),
        _ if uri.ends_with(":your-episodes") => uris(&api.get_all("/me/episodes?limit=50", 2000).await?, Some("episode")),
        "playlist" => uris(&api.get_all(&format!("/playlists/{}/tracks?limit=100", id_of(uri)), 10_000).await?, Some("entry")),
        "album" => uris(&api.get_all(&format!("/albums/{}/tracks?limit=50", id_of(uri)), 2000).await?, None),
        "show" => uris(&api.get_all(&format!("/shows/{}/episodes?limit=50", id_of(uri)), 50).await?, None),
        "track" | "episode" => vec![uri.to_string()],
        _ => anyhow::bail!("This can't be downloaded"),
    })
}

/// Sends `msg` when dropped unless disarmed: a task that dies halfway still answers.
struct Answer {
    tx: Option<Tx>,
    msg: Option<Msg>,
}

impl Answer {
    fn new(tx: Tx, msg: Msg) -> Self {
        Self { tx: Some(tx), msg: Some(msg) }
    }
    fn disarm(mut self) {
        self.tx = None;
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        if let (Some(tx), Some(m)) = (self.tx.take(), self.msg.take()) {
            tx.send(m);
        }
    }
}

/// Moves `uri` (listed at `from`, else its first listing) to `to` in the device's list
/// of what's next, as librespot's `glass_move_next` does: queued songs stay ahead of
/// the rest; a song moved in among them becomes queued. False when it isn't listed.
pub(crate) fn move_upcoming(list: &mut Vec<(String, bool)>, uri: &str, from: usize, to: usize) -> bool {
    let Some(i) = list.get(from).filter(|e| e.0 == uri).map(|_| from).or_else(|| list.iter().position(|e| e.0 == uri)) else { return false };
    let mut entry = list.remove(i);
    let queued = list.iter().take_while(|e| e.1).count();
    let mut at = to.min(list.len());
    if entry.1 {
        at = at.min(queued);
    } else if at < queued {
        entry.1 = true;
    }
    list.insert(at, entry);
    true
}

#[cfg(test)]
mod queue_tests {
    use super::move_upcoming;

    fn list(spec: &[(&str, bool)]) -> Vec<(String, bool)> {
        spec.iter().map(|(u, q)| (u.to_string(), *q)).collect()
    }

    #[test]
    fn queued_songs_stay_ahead() {
        // Two queued (a, b), then the context (c, d, e).
        let base = list(&[("a", true), ("b", true), ("c", false), ("d", false), ("e", false)]);

        let mut l = base.clone();
        assert!(move_upcoming(&mut l, "b", 1, 0));
        assert_eq!(l, list(&[("b", true), ("a", true), ("c", false), ("d", false), ("e", false)]), "reordered within the queue");

        let mut l = base.clone();
        assert!(move_upcoming(&mut l, "a", 0, 4));
        assert_eq!(l, list(&[("b", true), ("a", true), ("c", false), ("d", false), ("e", false)]), "a queued song stays the last queued");

        let mut l = base.clone();
        assert!(move_upcoming(&mut l, "e", 4, 1));
        assert_eq!(l, list(&[("a", true), ("e", true), ("b", true), ("c", false), ("d", false)]), "moved in among queued songs: queued");

        let mut l = base.clone();
        assert!(move_upcoming(&mut l, "e", 4, 2));
        assert_eq!(l, list(&[("a", true), ("b", true), ("e", false), ("c", false), ("d", false)]), "right after the queue: still from the context");

        let mut l = base.clone();
        assert!(move_upcoming(&mut l, "d", 0, 2), "found by URI when the index is stale");
        assert_eq!(l[2].0, "d");
        assert!(!move_upcoming(&mut l, "zzz", 0, 1));
    }
}
