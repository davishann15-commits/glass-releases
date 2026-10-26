//! App state and all side effects.
//!
//! Data flows one way:
//!   UI code reads `App` and pushes `Action`s ->
//!   `apply()` turns actions into Web API / engine calls on the tokio runtime ->
//!   results come back as `Msg`s -> `handle()` updates `App`.

use crate::api::{Api, NetLog, enc};
use crate::auth::{self, Auth};
use crate::config::{Autostart, Settings};
use crate::downloads::Downloads;
use crate::dsp::{DspParams, DspState};
use crate::engine::{Engine, EngineEvent};
use crate::local::{self, LocalPlayer, LocalTrack};
use crate::lyrics::cache::Cache as LyricsCache;
use crate::lyrics::lrclib::Client as LrclibClient;
use crate::lyrics::service::{self as lyrics_service, LyricsState, ProviderSetting, Request as LyricsRequest, Resolved, Status as LyricsStatus};
use crate::lyrics::{Candidate, Outcome, TrackRef};
use crate::model::{self, id_of, kind_of, s};
use crate::theme_config::ThemeStore;
use crate::ui;
use crate::user_config::ConfigStore;
use eframe::egui;
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- types

pub enum Load {
    Pending,
    Ready(Arc<Value>),
    Failed(String),
}

/// What the now-playing screen shows beside the controls.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum NpView {
    #[default]
    Lyrics,
    Queue,
    Devices,
    /// The playlist, album or list that's playing (P).
    Playlist,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    /// The split-view library: list on the left, the selection on the right.
    Library,
    /// Artwork and controls on the left; on the right, lyrics, the queue, devices or the playlist (`NpView`).
    NowPlaying,
    Home,
    Search,
    Genre {
        id: String,
        name: String,
    },
    Playlist(String),
    Album(String),
    Artist(String),
    Discography(String),
    Show(String),
    Episode(String),
    Audiobook(String),
    Track(String),
    Profile(String),
    LikedSongs,
    YourEpisodes,
    LocalFiles,
    Recents,
    Folder(String),
    /// A Glass local playlist (local files).
    LocalPlaylist(String),
    /// An album in Local Files, by its (album) artist and title.
    LocalAlbum {
        artist: String,
        album: String,
    },
    /// Another device's Local Files, to get some of its songs (by its `Peer::id`).
    Device {
        id: String,
        name: String,
    },
    Settings,
    Activity,
}

/// Where updating stands (see update.rs).
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    Idle,
    Checking,
    UpToDate,
    /// Downloaded and verified: installs on quit, or now with a restart.
    Ready {
        version: String,
        notes: String,
        zip: std::path::PathBuf,
    },
    /// Out, but this copy can't install it itself (a development build,
    /// Linux, or a folder Glass can't write to).
    Available {
        version: String,
        notes: String,
    },
    /// Installed; the new version opens next.
    Installed,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RightPanel {
    Queue,
    Devices,
    Lyrics,
    Assistant,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LibFilter {
    Playlists,
    Podcasts,
    Albums,
    Artists,
    Audiobooks,
    Downloaded,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SearchFilter {
    All,
    Songs,
    Artists,
    Albums,
    Playlists,
    Podcasts,
    Episodes,
    Audiobooks,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TrackSort {
    #[default]
    Custom,
    Title,
    Artist,
    Album,
    Added,
    Duration,
}

pub enum Modal {
    Shortcuts,
    About,
    EditPlaylist {
        id: String,
        name: String,
        desc: String,
        public: bool,
        collaborative: bool,
    },
    NewFolder {
        name: String,
        add: Option<String>,
    },
    RenameFolder {
        id: String,
        name: String,
    },
    Confirm {
        title: String,
        body: String,
        confirm: String,
        action: Box<Action>,
    },
    /// The tag editor for a local file.
    EditTags(Box<TagEditor>),
    /// Shown once, the first time ChatGPT plan usage is enabled.
    AiWelcome,
}

/// What the tag editor is showing.
#[derive(Clone)]
pub struct TagEditor {
    pub path: std::path::PathBuf,
    pub tags: local::Tags,
    pub cover: local::Cover,
    /// The cover shown: the file's own, or a newly chosen image.
    pub preview: Option<String>,
    pub error: Option<String>,
    pub saving: bool,
}

#[derive(Clone)]
pub enum Action {
    Navigate(Route),
    Back,
    Forward,
    // playback
    PlayContext {
        context: String,
        track: Option<String>,
    },
    PlayUris {
        uris: Vec<String>,
        index: usize,
    },
    PlayLocal {
        index: usize,
    },
    Radio(String),
    TogglePlay,
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
    /// Adds a playlist's or album's songs to the queue, fetching them first.
    QueueCollection(String),
    ClearQueue,
    Transfer(String),
    // library
    Save {
        uri: String,
        save: bool,
    },
    /// Several songs (or episodes) at once.
    SaveAll {
        uris: Vec<String>,
        save: bool,
    },
    AddToPlaylist {
        playlist: String,
        uris: Vec<String>,
    },
    RemoveFromPlaylist {
        playlist: String,
        uris: Vec<String>,
    },
    MovePlaylistItem {
        playlist: String,
        from: usize,
        to: usize,
    },
    CreatePlaylist {
        name: String,
        uris: Vec<String>,
    },
    UpdatePlaylist {
        id: String,
        body: Value,
    },
    DeletePlaylist(String),
    Pin(String),
    Unpin(String),
    CreateFolder {
        name: String,
        add: Option<String>,
    },
    RenameFolder {
        id: String,
        name: String,
    },
    DeleteFolder(String),
    MoveToFolder {
        playlist: String,
        folder: Option<String>,
    },
    // ui
    Copy(String),
    Toast(String),
    SetRight(RightPanel),
    ToggleRight,
    ToggleMini,
    ToggleFullscreen,
    Invalidate(String),
    RestartEngine,
    RescanLocal,
    /// Open the tag editor for a local file.
    EditTags(std::path::PathBuf),
    /// Pick an image for the tag editor's cover.
    ChooseCover,
    SaveTags,
    /// Keep a playlist, album, show, Liked Songs (`spotify:user:<id>:collection`)
    /// or a single track downloaded.
    Download {
        uri: String,
        name: String,
    },
    RemoveDownload(String),
    RemoveAllDownloads,
    /// Share ▸ "Download as …": save this song (a Web API track) as an audio file
    /// in the format from Settings ▸ Save songs.
    SaveSong(Box<Value>),
    /// The same for a whole playlist, album or artist's top songs (`items`: its tracks,
    /// in order), into a folder named after it.
    SaveSet {
        uri: String,
        name: String,
        kind: crate::save_song::SetKind,
        items: Vec<Value>,
    },
    /// Stop a collection's save: songs not started yet are skipped.
    StopSaveSet(String),
    /// Empty the streaming cache, keeping downloads.
    ClearAudioCache,
    // assistant
    AiSend(String),
    AiCancel,
    AiRetry,
    AiNewChat,
    AiSignIn {
        reconsent: bool,
        new_account: bool,
    },
    AiCancelSignIn,
    AiSignOut,
    AiSelectAccount(String),
    AiApprove(String),
    AiDecline(String),
    /// The user saves a preview (their approval); `replace` names the playlist it replaces.
    AiSaveDraft {
        draft: String,
        replace: Option<String>,
    },
    AiDiscardDraft(String),
    AiEditDraft {
        draft: String,
        edit: crate::assistant_app::DraftEdit,
    },
    AiUndo,
    AiRefreshModels,
    AiSetModel(String),
    AiSetBilling(crate::ai::Billing),
    AiEnable(bool),
    AiSaveApiKey,
    AiForgetApiKey,
    AiDismissWelcome,
    AiLimitAcknowledged,
    /// Let the assistant use (or stop using) the user's Spotify data.
    AiUseSpotify(bool),
    // local playlists
    PlayLocalPlaylist {
        id: String,
        /// Where to start, counting only the playlist's files that are still there.
        index: usize,
    },
    DeleteLocalPlaylist(String),
    /// Add files to one of Glass's local playlists.
    AddToLocalPlaylist {
        id: String,
        paths: Vec<std::path::PathBuf>,
    },
    CreateLocalPlaylist {
        name: String,
        paths: Vec<std::path::PathBuf>,
    },
    RemoveFromLocalPlaylist {
        id: String,
        index: usize,
    },
    /// Play these Local Files songs (indices into `local_tracks`), from `start`.
    PlayLocalTracks {
        indices: Vec<usize>,
        start: usize,
    },
    CheckForUpdates,
    /// Install the downloaded update and open it (it takes over from this one).
    RestartToUpdate,
    /// Send local songs to Glass on another of your computers (by its `Peer::id`).
    SendToDevice {
        peer: String,
        paths: Vec<std::path::PathBuf>,
    },
    /// Get a playlist's missing local songs (`spotify:local:` URIs) from
    /// your other devices.
    GetFromDevices(Vec<String>),
    /// List another device's Local Files (on its page, `Route::Device`).
    BrowseDevice(String),
    /// Get these of the browsed device's songs (`FileMeta::id`s).
    GetFromDevice {
        peer: String,
        ids: Vec<String>,
    },
    /// Audio files or folders dropped on the window or picked with "Add
    /// local files…": they join Local Files (their folders become sources
    /// so they play) and, when there's a target, that playlist.
    AddFiles {
        paths: Vec<std::path::PathBuf>,
        target: FileTarget,
    },
    UndoPlaylistChange,
    // lyrics
    /// Look the playing song's lyrics up again, skipping the cache and forgetting a
    /// match picked by hand.
    LyricsRetry,
    /// The user picked this LRCLIB record for the playing song.
    LyricsPick(Arc<crate::lyrics::parse::Record>),
    /// The user says none of the records fit: no lyrics for this song.
    LyricsNoneFit,
    LyricsOpenPicker,
    LyricsSearch(String),
    LyricsClosePicker,
    LyricsProvider(ProviderSetting),
    /// The new timing offset in milliseconds (saved to config.toml).
    LyricsOffset(i64),
    /// The playing song's own timing offset, remembered for that song.
    LyricsSongOffset(i64),
    /// Original, original + translation, or translated.
    LyricsTranslation(crate::lyrics::translate::Mode),
    ClearLyricsCache,
    SignIn,
    CancelSignIn,
    LogOut,
}

/// Where added files go besides Local Files.
#[derive(Clone, Debug, PartialEq)]
pub enum FileTarget {
    LocalFiles,
    /// A Spotify playlist you can edit (as `spotify:local:` entries).
    Spotify {
        id: String,
        name: String,
    },
    /// One of Glass's local playlists.
    Local {
        id: String,
        name: String,
    },
}

pub enum Msg {
    /// Dropped or added files, read: where they go, where they are now (see `local::keep_temporary`), and their songs.
    FilesRead(FileTarget, Vec<std::path::PathBuf>, Vec<LocalTrack>),
    /// Another copy of Glass was started.
    OtherCopy(crate::instance::Request),
    /// The DJ's voice for a break (by `dj_gen`), ready to play (`None`: nothing could
    /// be played).
    DjClips(u64, Result<Option<crate::dj::Voice>, String>),
    /// An update check: a newer version (downloaded, when this copy can
    /// install it), none, or why not; and whether it was asked for.
    Update(Result<Option<(crate::update::Manifest, Option<std::path::PathBuf>)>, String>, bool),
    Share(crate::share::Event),
    /// (sign-in attempt, result). Results from cancelled attempts are ignored.
    SignedIn(u64, Result<Arc<Auth>, String>),
    Data {
        key: String,
        result: Result<Arc<Value>, String>,
        /// `App::data_gen` when the request was sent.
        generation: u64,
    },
    Player(Result<Value, String>),
    /// Spotify pushed a change to the player state: look at it now.
    PlayerPushed,
    Queue(Result<Value, String>),
    SavedFailed(Vec<String>),
    Devices(Value),
    Saved(Vec<(String, bool)>),
    EngineReady(Result<Engine, String>),
    Engine(EngineEvent),
    Done {
        toast: Option<String>,
        invalidate: Vec<String>,
        refresh_player: bool,
        navigate: Option<Route>,
    },
    Failed(String),
    /// Lyrics for lookup `generation`; dropped if a newer lookup started.
    Lyrics {
        generation: u64,
        resolved: Resolved,
    },
    LyricsSearch {
        generation: u64,
        result: Result<Vec<Candidate>, String>,
    },
    Media(souvlaki::MediaControlEvent),
    LocalScanned(Vec<LocalTrack>),
    /// Files added to `base` (the library when they were read): the whole new list.
    LocalAdded {
        base: Arc<Vec<LocalTrack>>,
        all: Vec<LocalTrack>,
    },
    TagsSaved(std::path::PathBuf, Result<(), String>),
    /// A song being saved moved on: (its key in `App::saving`, its new state).
    SaveSong(String, crate::save_song::Status),
    /// A change the assistant asked for, done (or not) against Spotify.
    AiWrote(crate::assistant_app::Wrote),
    /// A translation of the lyrics on screen.
    LyricsTranslated {
        key: String,
        result: crate::lyrics::translate::Translation,
    },
    /// An event from assistant request `generation`.
    Ai {
        generation: u64,
        event: crate::assistant_app::AiEvent,
    },
    AiRegistered(String),
    AiSignedIn(Result<crate::ai::account::SignedIn, crate::ai::oauth::AuthError>),
    AiModels {
        key: String,
        result: Result<Vec<crate::ai::openai::Model>, String>,
    },
    AiSignedOut {
        confirmed: bool,
    },
    /// A verified, locally saved cover for the Now Playing widget: (track uri, file path).
    Artwork(String, std::path::PathBuf),
}

/// A page's own state, kept in the history (its scroll offset is kept by
/// egui, per route).
#[derive(Clone, Default)]
pub struct PageState {
    filter: String,
    sort: TrackSort,
    sort_desc: bool,
    cursor: Option<usize>,
}

/// The manual lyrics picker: a search on LRCLIB and its results.
pub struct LyricsPicker {
    pub query: String,
    /// Shared: the picker reads it every frame.
    pub results: Option<Result<Arc<Vec<Candidate>>, String>>,
    pub generation: u64,
}

#[derive(Default)]
pub struct Playback {
    /// Shared: the UI reads it every frame.
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

#[derive(PartialEq)]
pub enum AuthState {
    SignedOut,
    SigningIn,
    SignedIn,
}

/// Songs saved at once (each holds a few MB while it works, and YouTube prefers it calm).
const SAVES_AT_ONCE: usize = 3;

/// A playlist, album or artist being saved as files (see `App::save_set`).
pub struct SaveSet {
    pub name: String,
    pub kind: crate::save_song::SetKind,
    pub folder: std::path::PathBuf,
    /// Its songs' keys in `App::saving`, in order.
    pub keys: Vec<String>,
    /// Episodes and local files, which can't be saved.
    pub skipped: usize,
    pub stop: Arc<std::sync::atomic::AtomicBool>,
    /// The summary went out.
    pub finished: bool,
}

/// The playback engine's status for an account that isn't Premium (Settings, Devices).
const NEEDS_PREMIUM: &str = "Not started: Spotify plays songs only for Premium accounts. Browsing, Local Files and Share ▸ Download as work without it.";

/// Something to play once the playback engine is ready (see `App::start_play`).
pub(crate) struct PendingPlay {
    pub body: Value,
    /// Shuffle turned on or off after loading (an assistant request).
    pub shuffle: Option<bool>,
    /// Started by the assistant, which works in the background: Now Playing doesn't open.
    pub quiet: bool,
}

pub struct App {
    pub ctx: egui::Context,
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub settings: Settings,
    settings_dirty: Option<Instant>,
    /// The look, from ~/.config/glass/theme.toml; reloads when the file changes.
    pub theme: ThemeStore,
    /// Other file settings (lyrics, assistant), from ~/.config/glass/config.toml.
    pub config: ConfigStore,
    /// The optional assistant (on by default; it does nothing until signed in or given a key).
    pub ai: crate::ai::Assistant,
    pub ai_undo: Option<crate::assistant_app::AiUndo>,
    /// Where the assistant reads Spotify from when there's no live connection
    /// (tests use a fake; otherwise demo data).
    pub ai_source_override: Option<Arc<dyn crate::ai::spotify::Source>>,
    /// Glass's own playlists of local files.
    pub playlists: crate::playlists::Playlists,
    /// Lyrics for the song that's playing.
    pub lyrics: LyricsState,
    pub lyrics_cache: Arc<LyricsCache>,
    /// The user's lyrics picks and per-song timing, kept for good.
    pub lyrics_choices: crate::lyrics::choices::Choices,
    /// The translation of the lyrics on screen, and saved translations.
    pub translation: crate::translate_app::TranslationState,
    pub translation_store: crate::lyrics::translate::Store,
    lyrics_client: Arc<LrclibClient>,
    /// The (contact, proxy) the LRCLIB client was made with.
    lyrics_client_for: (String, String),
    lyrics_nonce: u64,
    pub lyrics_picker: Option<LyricsPicker>,
    pub auth_state: AuthState,
    pub auth_error: Option<String>,
    signin_gen: u64,
    /// The saved session couldn't be refreshed for want of a network: when to try
    /// again, and how many times it has been tried.
    resume_retry: Option<(Instant, u32)>,
    /// The playback engine failed to start: when to try again, and how many times it has.
    engine_retry: Option<(Instant, u32)>,
    /// An engine is being started (one at a time).
    engine_launching: bool,
    /// The playing song's lyrics timing offset, by its lyrics key (see `lyrics_song_offset`).
    song_offset: std::cell::RefCell<Option<(String, i64)>>,
    /// Bumped by `invalidate`, which notes what it invalidated: answers to matching
    /// requests sent before it count as stale.
    data_gen: u64,
    invalidated: Vec<(u64, String)>,
    /// Numbers lyrics searches (see `lyrics_search`).
    lyrics_search_seq: u64,
    pub api: Option<Api>,
    pub log: NetLog,
    pub engine: Option<Engine>,
    pub engine_status: String,
    /// A play request made while the engine was still connecting.
    pub(crate) pending_play: Option<PendingPlay>,
    pub downloads: Downloads,
    /// Local files sharing with Glass on your other devices (while it's on).
    share: Option<crate::share::Share>,
    /// Sharing couldn't start; not retried until it's turned off and on.
    share_failed: bool,
    /// Glass on your other devices, on this network, that songs can go to and from.
    pub share_peers: Vec<crate::share::Peer>,
    /// This account's Glass on other devices that speaks another sharing protocol
    /// (one of them needs updating).
    pub share_others: Vec<crate::share::Peer>,
    /// Another device's Local Files, on its page.
    pub device: Option<crate::share::Browsed>,
    /// The songs picked there (`FileMeta::id`s).
    pub device_picks: HashSet<String>,
    /// Recent transfers, newest last.
    pub transfers: Vec<crate::share::Transfer>,
    /// The DJ speaking (its song waits until it's done).
    dj_voice: Option<crate::dj::Voice>,
    /// The DJ's lines are being fetched while its song waits.
    dj_waiting: bool,
    /// Which DJ break is current (an older fetch arriving late is ignored).
    dj_gen: u64,
    /// What the DJ said last, and when.
    dj_line: Option<(String, Instant)>,
    dj_clips: crate::dj::Clips,
    dj_http: reqwest::Client,
    pub update: Update,
    next_update_check: Instant,
    /// Local Files as sharing sees it (kept in step with `local_tracks`).
    share_library: crate::share::Library,
    /// Local Files should be read again when the scan running now ends.
    pub(crate) local_rescan: bool,
    last_prune: Instant,
    dsp: Arc<DspParams>,
    pub me: Option<Value>,
    pub data: HashMap<String, Load>,
    fetched_at: HashMap<String, Instant>,
    /// When each cached response was last asked for (to drop unused ones).
    used_at: HashMap<String, Instant>,
    last_evict: Instant,
    /// The blur radius the window was last given.
    window_blur: Option<u32>,
    /// Whether the window had focus last frame.
    was_focused: bool,
    /// The equalizer settings last given to the audio thread.
    dsp_sent: Option<DspState>,
    refreshing: HashSet<String>,
    pub saved: HashMap<String, bool>,
    saved_wanted: HashSet<String>,
    saved_pending: HashSet<String>,
    pub pb: Playback,
    pub queue: Option<Arc<Value>>,
    pub devices: Option<Value>,
    next_poll: Instant,
    next_queue_poll: Instant,
    /// The song the queue was last read near the end of (album colors' look-ahead).
    queue_read_for: String,
    next_devices_poll: Instant,
    /// The device list was on screen at the last poll check (it's read again when it appears).
    devices_shown: bool,
    /// Why the queue couldn't be read, shown instead of a spinner.
    pub queue_error: Option<String>,
    /// A player-state request is in flight (one at a time).
    player_polling: bool,
    next_me_poll: Instant,
    /// When this computer's player last changed something itself; a poll
    /// right after can carry the old state and mustn't undo it.
    pub(crate) local_touched: Option<Instant>,
    /// The song this computer's player has (from its own events).
    engine_uri: String,
    /// The row just clicked to play, so the bar can show it straight away.
    pub play_hint: Option<Value>,
    /// The explicit song already skipped (so it's skipped once).
    skipped_explicit: String,
    /// After a failed "is it saved?" check, wait before asking again.
    saved_backoff: Option<Instant>,
    /// Local songs to play once the folders have been scanned (a `PlayLocal` or
    /// `PlayLocalPlaylist`).
    pending_local: Option<Action>,
    /// Which local file each `spotify:local:` playlist entry is (cleared on rescan).
    local_links: HashMap<String, Option<usize>>,
    cover_previews: u32,
    pub local: Option<LocalPlayer>,
    /// Local Files as last scanned (shared, so pages borrow it without copying).
    pub local_tracks: Option<Arc<Vec<LocalTrack>>>,
    pub(crate) local_scanning: bool,
    media: Option<souvlaki::MediaControls>,
    media_uri: String,
    media_cover: Option<std::path::PathBuf>,
    media_playing: bool,
    notified_uri: String,
    // ui state
    pub route: Route,
    /// History, with each page's filter and sort so Back returns to it as it was.
    back: Vec<(Route, PageState)>,
    fwd: Vec<(Route, PageState)>,
    pub right: RightPanel,
    pub modal: Option<Modal>,
    pub toasts: Vec<(String, Instant)>,
    /// Songs saved as files this session: by URI, or `<collection URI>|<song URI>` for
    /// songs saved as part of a collection.
    pub saving: HashMap<String, crate::save_song::Job>,
    /// Collections being (or done being) saved, by URI.
    pub saving_sets: HashMap<String, SaveSet>,
    /// The playing album's colors (Settings ▸ Display).
    pub album_colors: crate::ui::album_colors::AlbumColors,
    save_http: reqwest::Client,
    /// Saves run a few at a time.
    save_slots: Arc<tokio::sync::Semaphore>,
    pub search_text: String,
    pub search_filter: SearchFilter,
    pub lib_filter: LibFilter,
    pub expanded_folders: HashSet<String>,
    pub page_filter: String,
    pub page_sort: TrackSort,
    pub page_sort_desc: bool,
    pub home_filter: u8,
    pub disco_filter: u8,
    pub muted_volume: Option<f32>,
    /// The position changed by a seek: tell the system's media controls.
    media_resync: bool,
    pub mini: bool,
    pub fullscreen: bool,
    pub actions: Vec<Action>,
    pub scroll_top: bool,
    pub settings_query: String,
    pub focus_search: bool,
    pub selection: Vec<String>,
    pub sel_anchor: Option<usize>,
    /// Selected songs on the Local Files page.
    pub local_selection: std::collections::HashSet<std::path::PathBuf>,
    /// Local Files' Songs view: sorted by, and descending.
    pub local_sort: (crate::ui::local_library::SongSort, bool),
    /// The artist and genre picked in Local Files' Artists and Genres views.
    pub local_artist: Option<String>,
    pub local_genre: Option<String>,
    /// Quitting for real (another Glass build is taking over), not minimizing.
    pub quitting: bool,
    /// The keyboard cursor in the page's song table (j/k, Enter).
    pub row_cursor: Option<usize>,
    pub compact_rows: bool,
    pub select_all_pending: bool,
    pub pl_find: String,
    pub artist_more: bool,
    pub search_committed: String,
    pub search_edit_at: Option<Instant>,
    pub last_volume_sent: Instant,
    // ---- library screen and chrome (UI state only)
    /// 0 music, 1 podcasts, 2 audiobooks.
    pub lib_tab: u8,
    /// The library row shown in the detail pane.
    pub lib_selected: Option<String>,
    /// The command bar's text, whether its result list is open, and the highlighted result.
    pub cmd_text: String,
    pub cmd_open: bool,
    pub cmd_index: usize,
    pub focus_cmd: bool,
    pub theme_panel: bool,
    /// What the now-playing screen shows beside the controls.
    pub np_view: NpView,
    /// The visualizer's analysis (it runs only while the visualizer shows).
    pub viz: crate::viz::Analyzer,
    /// Follows the system's audio output (headphones coming and going).
    output: crate::audio_out::OutputWatch,
    /// What's typed in the theme panel's fields, by key, until it's valid.
    pub theme_fields: HashMap<String, String>,
    /// The theme panel's tab: 0 theme, 1 lyrics, 2 visualizer.
    pub theme_tab: u8,
    pub theme_msg: Option<String>,
}

// ---------------------------------------------------------------- setup

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).on_thread_park(crate::heap::on_idle).build().expect("tokio runtime");
        let (tx, rx) = channel();
        let settings = Settings::load();
        let dsp = Arc::new(DspParams::new(DspState { eq_enabled: settings.eq_enabled, gains: settings.eq_gains, mono: settings.mono }));
        let volume = settings.volume;
        let config = ConfigStore::load();
        let contact = config.lyrics().contact.clone();
        let proxy = settings.proxy.clone();
        crate::config::use_proxy(&proxy);
        // Every request Glass makes, for the Activity page.
        let log = NetLog::default();
        let mut app = Self {
            ctx,
            rt,
            tx,
            rx,
            settings,
            settings_dirty: None,
            theme: ThemeStore::load(),
            ai: if crate::window_fx::headless() || std::env::var("GLASS_DEMO").is_ok() {
                // Tests and the demo never touch the real keyring or OpenAI.
                crate::ai::Assistant::new(
                    Arc::new(crate::ai::secrets::MemoryStore::default()),
                    crate::ai::account::Endpoints {
                        api: "http://127.0.0.1:9".into(),
                        token: "http://127.0.0.1:9".into(),
                        discovery: "http://127.0.0.1:9".into(),
                        authorize: "http://127.0.0.1:9".into(),
                    },
                    crate::ai::account::Accounts::dir(),
                )
            } else {
                crate::ai::Assistant::load()
            },
            ai_undo: None,
            ai_source_override: None,
            playlists: crate::playlists::Playlists::load(crate::playlists::Playlists::path()),
            lyrics: LyricsState::default(),
            lyrics_cache: Arc::new(LyricsCache::new(LyricsCache::dir())),
            lyrics_choices: crate::lyrics::choices::Choices::load(crate::lyrics::choices::Choices::path()),
            translation: Default::default(),
            translation_store: crate::lyrics::translate::Store::new(crate::lyrics::translate::Store::dir()),
            lyrics_client: LrclibClient::new(&contact, &proxy, Some(log.clone())),
            lyrics_client_for: (contact, proxy),
            lyrics_nonce: 0,
            lyrics_picker: None,
            config,
            auth_state: AuthState::SignedOut,
            auth_error: None,
            signin_gen: 0,
            resume_retry: None,
            engine_retry: None,
            engine_launching: false,
            song_offset: Default::default(),
            lyrics_search_seq: 0,
            data_gen: 0,
            invalidated: Vec::new(),
            api: None,
            log,
            engine: None,
            engine_status: "Not started".into(),
            pending_play: None,
            downloads: Downloads::load(),
            last_prune: Instant::now(),
            dsp,
            me: None,
            data: HashMap::new(),
            fetched_at: HashMap::new(),
            used_at: HashMap::new(),
            last_evict: Instant::now(),
            window_blur: None,
            was_focused: true,
            dsp_sent: None,
            refreshing: HashSet::new(),
            saved: HashMap::new(),
            saved_wanted: HashSet::new(),
            saved_pending: HashSet::new(),
            pb: Playback { volume, repeat: "off".into(), ..Default::default() },
            queue: None,
            devices: None,
            next_poll: Instant::now(),
            next_queue_poll: Instant::now(),
            queue_read_for: String::new(),
            next_devices_poll: Instant::now(),
            devices_shown: false,
            queue_error: None,
            player_polling: false,
            next_me_poll: Instant::now() + Duration::from_secs(5),
            local_touched: None,
            engine_uri: String::new(),
            play_hint: None,
            skipped_explicit: String::new(),
            saved_backoff: None,
            pending_local: None,
            local_links: HashMap::new(),
            cover_previews: 0,
            local: None,
            local_tracks: None,
            local_scanning: false,
            media: None,
            media_uri: String::new(),
            media_cover: None,
            media_playing: false,
            notified_uri: String::new(),
            route: Route::Library,
            back: Vec::new(),
            fwd: Vec::new(),
            right: RightPanel::Queue,
            modal: None,
            toasts: Vec::new(),
            saving: HashMap::new(),
            saving_sets: HashMap::new(),
            album_colors: Default::default(),
            save_http: crate::save_song::http_client(),
            save_slots: Arc::new(tokio::sync::Semaphore::new(SAVES_AT_ONCE)),
            search_text: String::new(),
            search_filter: SearchFilter::All,
            lib_filter: LibFilter::Playlists,
            expanded_folders: HashSet::new(),
            page_filter: String::new(),
            page_sort: TrackSort::Custom,
            page_sort_desc: false,
            home_filter: 0,
            disco_filter: 0,
            muted_volume: None,
            media_resync: false,
            mini: false,
            fullscreen: false,
            actions: Vec::new(),
            scroll_top: false,
            settings_query: String::new(),
            focus_search: false,
            selection: Vec::new(),
            sel_anchor: None,
            local_selection: Default::default(),
            local_sort: Default::default(),
            local_artist: None,
            local_genre: None,
            share: None,
            share_failed: false,
            share_peers: Vec::new(),
            share_others: Vec::new(),
            device: None,
            device_picks: HashSet::new(),
            transfers: Vec::new(),
            dj_voice: None,
            dj_waiting: false,
            dj_gen: 0,
            dj_line: None,
            dj_clips: Default::default(),
            dj_http: crate::config::http().build().unwrap_or_default(),
            update: Update::Idle,
            next_update_check: Instant::now() + Duration::from_secs(20),
            share_library: Arc::default(),
            local_rescan: false,
            quitting: false,
            row_cursor: None,
            compact_rows: false,
            select_all_pending: false,
            pl_find: String::new(),
            artist_more: false,
            search_committed: String::new(),
            search_edit_at: None,
            last_volume_sent: Instant::now(),
            lib_tab: 0,
            lib_selected: None,
            cmd_text: String::new(),
            cmd_open: false,
            cmd_index: 0,
            focus_cmd: false,
            theme_panel: false,
            np_view: NpView::Lyrics,
            viz: Default::default(),
            output: Default::default(),
            theme_fields: HashMap::new(),
            theme_tab: 0,
            theme_msg: None,
        };
        app.setup_media_keys();
        // Read the local folders now, so playlists know which of their local songs are here.
        // The same folder twice, or one inside another, listed every song in
        // it more than once.
        let tidy = local::tidy_sources(&app.settings.local_dirs);
        if tidy != app.settings.local_dirs {
            app.settings.local_dirs = tidy;
            app.mark_settings_dirty();
        }
        if !app.settings.local_dirs.is_empty() && std::env::var("GLASS_DEMO").is_err() {
            app.scan_local();
        }
        let cache = app.lyrics_cache.clone();
        let saves = app.settings.save_dir.clone().unwrap_or_else(crate::save_song::default_dir);
        std::thread::spawn(move || {
            cache.prune();
            crate::save_song::sweep_work_files(&saves);
            // Songs from other devices arrive in Music/Glass, wherever saves go.
            if crate::share::inbox() != saves {
                crate::save_song::sweep_work_files(&crate::share::inbox());
            }
        });
        // The side panel opens on demand; the window starts on the plain layout.
        app.settings.right_open = false;
        if std::env::var("GLASS_DEMO").is_ok() {
            crate::demo::install(&mut app);
        } else if auth::has_saved_session() {
            // GLASS_ROUTE=nowplaying|home|search|settings|local opens on that screen (used to check layouts).
            app.route = match std::env::var("GLASS_ROUTE").as_deref() {
                Ok("nowplaying") => Route::NowPlaying,
                Ok("home") => Route::Home,
                Ok("search") => Route::Search,
                Ok("settings") => Route::Settings,
                Ok("local") => Route::LocalFiles,
                _ => Route::Library,
            };
            app.auth_state = AuthState::SigningIn;
            app.resume_session();
        }
        app
    }

    /// Signs in with the saved session (generation 0: any sign-in started by hand wins).
    fn resume_session(&self) {
        let (tx, ctx, id) = (self.tx.clone(), self.ctx.clone(), self.settings.client_id.clone());
        self.rt.spawn(async move {
            let r = Auth::resume(&id).await.map(Arc::new).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::SignedIn(0, r));
            ctx.request_repaint();
        });
    }

    fn setup_media_keys(&mut self) {
        if crate::window_fx::headless() {
            return;
        }
        let config = souvlaki::PlatformConfig { dbus_name: "glass", display_name: "Glass", hwnd: None };
        if let Ok(mut mc) = souvlaki::MediaControls::new(config) {
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            if mc
                .attach(move |e| {
                    let _ = tx.send(Msg::Media(e));
                    ctx.request_repaint();
                })
                .is_ok()
            {
                self.media = Some(mc);
            }
        }
    }

    /// Settings are written a moment after the last change (a dragged slider changes them every frame).
    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty.get_or_insert(Instant::now());
        self.ctx.request_repaint_after(Duration::from_millis(900));
    }

    pub fn user_id(&self) -> String {
        self.me.as_ref().map(|m| s(m, "id").to_string()).unwrap_or_default()
    }

    pub fn is_premium(&self) -> bool {
        self.me.as_ref().map(|m| s(m, "product") == "premium").unwrap_or(true)
    }

    /// `/me` answered for the first time since signing in. Spotify streams only to
    /// Premium accounts: they get the playback engine; a free account browses, plays
    /// Local Files and saves songs (Share ▸ Download as), without one.
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

    fn page_state(&self) -> PageState {
        PageState { filter: self.page_filter.clone(), sort: self.page_sort, sort_desc: self.page_sort_desc, cursor: self.row_cursor }
    }

    fn restore_page(&mut self, s: PageState) {
        self.page_filter = s.filter;
        self.page_sort = s.sort;
        self.page_sort_desc = s.sort_desc;
        self.row_cursor = s.cursor;
    }

    /// Back/Forward: the page comes back with its filter, sort and scroll position.
    fn return_to(&mut self, s: PageState) {
        self.restore_page(s);
        self.selection.clear();
        self.local_selection.clear();
        self.sel_anchor = None;
        self.scroll_top = false;
        self.focus_search = false;
        self.select_all_pending = false;
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn act(&mut self, a: Action) {
        self.actions.push(a);
    }

    pub fn toast(&mut self, t: impl Into<String>) {
        self.toasts.push((t.into(), Instant::now()));
    }

    // ------------------------------------------------------------ save songs

    pub fn save_options(&self) -> crate::save_song::Options {
        crate::save_song::Options { format: self.settings.save_format, mp3_kbps: self.settings.save_mp3_kbps, dir: self.settings.save_dir.clone().unwrap_or_else(crate::save_song::default_dir) }
    }

    /// Saves a song as an audio file (see `save_song`).
    fn save_song(&mut self, item: Value) {
        use crate::save_song::Status;
        let uri = s(&item, "uri").to_string();
        let Some(song) = crate::save_song::song_from_item(&item) else {
            self.toast("Only Spotify songs can be downloaded");
            return;
        };
        if self.saving.get(&uri).is_some_and(crate::save_song::Job::busy) {
            return;
        }
        let opts = self.save_options();
        let (title, format) = (song.title.clone(), opts.format.name());
        match self.start_save(uri, song, opts, None) {
            Status::Done => self.toast(format!("“{title}” is already saved as {format}")),
            _ => self.toast(format!("Saving “{title}” as {format}…")),
        }
    }

    /// Saves a playlist's, album's or artist's songs into a folder named after it.
    fn save_set(&mut self, uri: String, name: String, kind: crate::save_song::SetKind, items: Vec<Value>) {
        use crate::save_song::{self as save, Job, SetKind};
        if self.saving_sets.get(&uri).is_some_and(|set| !set.finished) {
            return;
        }
        let mut opts = self.save_options();
        opts.dir = opts.dir.join(save::file_name(&name));
        let format = opts.format.name();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (mut keys, mut skipped, mut seen, mut paths) = (Vec::new(), 0, HashSet::new(), HashSet::new());
        for item in &items {
            let song_uri = s(item, "uri").to_string();
            let Some(song) = save::song_from_item(item) else {
                skipped += 1;
                continue;
            };
            // Once per song, and once per file (an album and a single version share a name).
            if !seen.insert(song_uri.clone()) || !paths.insert(save::output_path(&song, &opts)) {
                continue;
            }
            let key = format!("{uri}|{song_uri}");
            // A failed or stopped song gets another go; a saved one stays saved.
            if !self.saving.get(&key).is_some_and(|j: &Job| j.busy()) {
                self.start_save(key.clone(), song, opts.clone(), Some(stop.clone()));
            }
            keys.push(key);
        }
        if keys.is_empty() {
            self.toast(format!("“{name}” has no songs that can be downloaded"));
            return;
        }
        let count = keys.len();
        let folder = opts.dir.clone();
        self.saving_sets.insert(uri.clone(), SaveSet { name: name.clone(), kind, folder, keys, skipped, stop, finished: false });
        let what = if kind == SetKind::Artist { format!("{name}'s top songs") } else { format!("“{name}”") };
        self.toast(format!("Saving {count} {} from {what} as {format}…", if count == 1 { "song" } else { "songs" }));
        // Everything may already be there.
        self.check_set_done(&uri);
    }

    /// Starts (or, if the file exists, records) one save; returns its first status.
    fn start_save(&mut self, key: String, song: crate::save_song::Song, opts: crate::save_song::Options, stop: Option<Arc<std::sync::atomic::AtomicBool>>) -> crate::save_song::Status {
        use crate::save_song::{Job, Status};
        let path = crate::save_song::output_path(&song, &opts);
        let title = song.title.clone();
        if path.exists() {
            self.saving.insert(key, Job { title, status: Status::Done, path });
            return Status::Done;
        }
        self.saving.insert(key.clone(), Job { title: title.clone(), status: Status::Queued, path });
        // No live downloads in tests.
        if crate::window_fx::headless() {
            return Status::Queued;
        }
        let (tx, ctx, http, slots) = (self.tx.clone(), self.ctx.clone(), self.save_http.clone(), self.save_slots.clone());
        let report: Arc<dyn Fn(Status) + Send + Sync> = Arc::new(move |status| {
            let _ = tx.send(Msg::SaveSong(key.clone(), status));
            ctx.request_repaint();
        });
        self.rt.spawn(async move {
            // Waits its turn (three at a time), but Stop counts while waiting too.
            let stopped = || stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed));
            let _slot = loop {
                if stopped() {
                    report(Status::Stopped);
                    return;
                }
                if let Ok(Ok(slot)) = tokio::time::timeout(Duration::from_millis(300), slots.clone().acquire_owned()).await {
                    break slot;
                }
            };
            if stopped() {
                report(Status::Stopped);
                return;
            }
            let done = match crate::save_song::save(http, song, opts, report.clone()).await {
                Ok(_) => Status::Done,
                Err(e) => {
                    log::warn!("couldn't save “{title}”: {e:#}");
                    Status::Failed(crate::save_song::short_error(&e))
                }
            };
            report(done);
        });
        Status::Queued
    }

    fn song_save_moved(&mut self, key: String, status: crate::save_song::Status) {
        use crate::save_song::Status;
        let Some(job) = self.saving.get_mut(&key) else { return };
        // A late progress report never undoes the outcome.
        if !job.busy() {
            return;
        }
        job.status = status.clone();
        if job.busy() {
            // Progress: the Share menu reads it from `saving` when it's open.
            return;
        }
        let (title, path) = (job.title.clone(), job.path.clone());
        // Songs saved as part of a collection report once, with the collection.
        if let Some((set, _)) = key.split_once('|') {
            let set = set.to_string();
            self.check_set_done(&set);
            return;
        }
        match status {
            Status::Done => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.toast(format!("Saved “{title}” as {name}"));
                self.rescan_if_in_local_files(&path);
            }
            Status::Failed(why) => self.toast(format!("Couldn't save “{title}”: {why}")),
            _ => {}
        }
    }

    /// (saved, failed, stopped, still going) for a collection's songs.
    pub fn set_progress(&self, uri: &str) -> Option<(usize, usize, usize, usize)> {
        use crate::save_song::Status;
        let set = self.saving_sets.get(uri)?;
        let mut counts = (0, 0, 0, 0);
        for job in set.keys.iter().filter_map(|k| self.saving.get(k)) {
            match job.status {
                Status::Done => counts.0 += 1,
                Status::Failed(_) => counts.1 += 1,
                Status::Stopped => counts.2 += 1,
                _ => counts.3 += 1,
            }
        }
        Some(counts)
    }

    /// When a collection's last song settles: one toast, and the playlist file.
    fn check_set_done(&mut self, uri: &str) {
        use crate::save_song::{SetKind, Status};
        let Some((saved, failed, stopped, going)) = self.set_progress(uri) else { return };
        let Some(set) = self.saving_sets.get_mut(uri) else { return };
        if going > 0 || set.finished {
            return;
        }
        set.finished = true;
        let (name, kind, folder, skipped) = (set.name.clone(), set.kind, set.folder.clone(), set.skipped);
        let saved_paths: Vec<std::path::PathBuf> = set.keys.iter().filter_map(|k| self.saving.get(k)).filter(|j| j.status == Status::Done).map(|j| j.path.clone()).collect();
        let mut local_playlist = false;
        if kind == SetKind::Playlist && !saved_paths.is_empty() {
            if let Err(e) = crate::save_song::write_playlist_file(&folder, &name, &saved_paths) {
                log::warn!("couldn't write the playlist file for {name}: {e}");
            }
            // It becomes a local playlist too (downloading it again updates that one),
            // and its folder joins Local Files so the songs play from it.
            if !local::read_by(&self.settings.local_dirs, &folder) && local::add_source(&mut self.settings.local_dirs, folder.clone()) {
                self.mark_settings_dirty();
            }
            match self.playlists.from_download(uri, &name, "", saved_paths.clone(), None) {
                Ok(_) => local_playlist = true,
                Err(e) => log::warn!("couldn't make the local playlist for {name}: {e}"),
            }
        }
        let total = saved + failed + stopped;
        let mut text = match (saved, total) {
            (1, 1) => format!("Saved the song from “{name}”"),
            (s, t) if s == t => format!("Saved all {t} songs from “{name}”"),
            (s, t) => format!("Saved {s} of {t} songs from “{name}”"),
        };
        if failed > 0 {
            text.push_str(&format!(" · {failed} failed"));
        }
        if stopped > 0 {
            text.push_str(&format!(" · {stopped} stopped"));
        }
        if skipped > 0 {
            text.push_str(&if skipped == 1 { " · 1 episode or local file skipped".to_string() } else { format!(" · {skipped} episodes or local files skipped") });
        }
        if local_playlist {
            text.push_str(" · it's in Local Files › Playlists");
        }
        self.toast(text);
        self.rescan_if_in_local_files(&folder);
    }

    /// Saved files join Local Files when their folder is one of its sources.
    fn rescan_if_in_local_files(&mut self, path: &std::path::Path) {
        if self.settings.show_local_files && path.parent().is_some_and(|dir| local::read_by(&self.settings.local_dirs, dir)) {
            self.add_local_files(vec![path.to_path_buf()]);
        }
    }

    /// New files (or a folder of them) join Local Files: only they are read, not
    /// the whole library again. Before Local Files is first read there's nothing to do.
    fn add_local_files(&mut self, paths: Vec<std::path::PathBuf>) {
        let Some(base) = self.local_tracks.clone() else { return };
        if self.local_scanning {
            self.local_rescan = true;
            return;
        }
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let new = local::read_paths(&paths);
            let fresh: std::collections::HashSet<&std::path::Path> = new.iter().map(|t| t.path.as_path()).collect();
            let mut all: Vec<LocalTrack> = base.iter().filter(|t| !fresh.contains(t.path.as_path())).cloned().collect();
            all.extend(new);
            // As a scan has them: newest first.
            all.sort_by_key(|t| std::cmp::Reverse(t.added));
            let _ = tx.send(Msg::LocalAdded { base, all });
            ctx.request_repaint();
        });
    }

    /// The library is now `t` (from a scan, or with files added).
    fn set_local_tracks(&mut self, t: Arc<Vec<LocalTrack>>) {
        self.local_links.clear();
        if let Ok(mut lib) = self.share_library.write() {
            *lib = t.clone();
        }
        // Songs got from a device are here now: they can't be picked any more.
        if let Some(d) = self.device.as_mut() {
            d.mark_here(&t);
            let here: HashSet<&str> = d.list().filter(|(_, here)| *here).map(|(f, _)| f.id.as_str()).collect();
            self.device_picks.retain(|id| !here.contains(id.as_str()));
        }
        self.local_tracks = Some(t);
    }

    // ------------------------------------------------------------ data cache

    /// Return cached JSON for a GET path, starting the request if needed.
    pub fn want(&mut self, path: &str) -> Option<Arc<Value>> {
        self.want_key(path, None)
    }

    /// Like `want`, but for every item of a list endpoint, up to `cap` of them.
    pub fn want_all(&mut self, path: &str, cap: usize) -> Option<Arc<Value>> {
        self.want_key(&format!("ALL{cap}:{path}"), Some((path, cap)))
    }

    /// Cached responses are served immediately; anything older than `TTL`
    /// is refetched in the background and swapped in when it arrives.
    fn want_key(&mut self, key: &str, all: Option<(&str, usize)>) -> Option<Arc<Value>> {
        const TTL: Duration = Duration::from_secs(300);
        // A failed load is tried again after this long (the network may be back).
        const RETRY: Duration = Duration::from_secs(20);
        // Looked up by `&str`: the key is only copied when something is stored under it.
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
        if cached.is_some() && (self.fetched_at.get(key).is_some_and(|t| t.elapsed() < TTL) || self.refreshing.contains(key)) {
            return cached;
        }
        let Some(api) = self.api.clone() else { return cached };
        if cached.is_some() {
            self.refreshing.insert(key.to_string());
        } else {
            self.data.insert(key.to_string(), Load::Pending);
        }
        let (key, all) = (key.to_string(), all.map(|(path, cap)| (path.to_string(), cap)));
        let (tx, ctx, generation) = (self.tx.clone(), self.ctx.clone(), self.data_gen);
        self.rt.spawn(async move {
            // Should the request die halfway (a panic decoding a reply), the key still gets an
            // answer, rather than loading forever.
            let failed = Answer::new(tx.clone(), ctx.clone(), Msg::Data { key: key.clone(), result: Err("the request failed".into()), generation });
            let r = match &all {
                Some((path, cap)) => api.get_all(path, *cap).await.map(Arc::new),
                None => api.get_shared(&key).await,
            };
            failed.disarm();
            let _ = tx.send(Msg::Data { key, result: r.map_err(|e| e.to_string()), generation });
            ctx.request_repaint();
        });
        cached
    }

    /// Pages not looked at for 10 minutes are dropped (they're fetched again
    /// if you go back). Your library itself stays.
    fn evict_unused(&mut self) {
        const IDLE: Duration = Duration::from_secs(600);
        // Hearts for every song ever shown: past this many, they're asked again as needed.
        if self.saved.len() > 20_000 {
            self.saved.clear();
        }
        let old: Vec<String> = self.used_at.iter().filter(|(k, t)| t.elapsed() > IDLE && !kept_for_good(k)).map(|(k, _)| k.clone()).collect();
        for k in old {
            self.used_at.remove(&k);
            if matches!(self.data.get(&k), Some(Load::Ready(_)) | Some(Load::Failed(_))) {
                self.data.remove(&k);
                self.fetched_at.remove(&k);
            }
        }
    }

    pub fn load_error(&self, path: &str) -> Option<String> {
        self.data.iter().find_map(|(k, v)| match v {
            Load::Failed(e) if k == path || k.ends_with(&format!(":{path}")) => Some(e.clone()),
            _ => None,
        })
    }

    /// Mark matching responses stale: they're refetched on next use while the
    /// old data stays on screen. Failed loads are dropped so they retry.
    pub fn invalidate(&mut self, needle: &str) {
        // Answers to matching requests sent before this are shown but count as stale (they
        // may predate the change), so they're asked for again.
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

    /// Is this URI in the user's library? `None` while unknown (queued).
    pub fn is_saved(&mut self, uri: &str) -> Option<bool> {
        if let Some(v) = self.saved.get(uri) {
            return Some(*v);
        }
        if !uri.is_empty() && !self.saved_pending.contains(uri) {
            self.saved_wanted.insert(uri.to_string());
        }
        None
    }

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
                let ids: Vec<&str> = chunk.iter().map(|u| id_of(u)).collect();
                let ids = ids.join(",");
                let primary = match kind.as_str() {
                    "track" => format!("/me/tracks/contains?ids={ids}"),
                    "album" => format!("/me/albums/contains?ids={ids}"),
                    "show" => format!("/me/shows/contains?ids={ids}"),
                    "episode" => format!("/me/episodes/contains?ids={ids}"),
                    "audiobook" => format!("/me/audiobooks/contains?ids={ids}"),
                    "artist" => format!("/me/following/contains?type=artist&ids={ids}"),
                    "playlist" => format!("/playlists/{}/followers/contains?ids={}", id_of(&chunk[0]), enc(&me)),
                    _ => continue,
                };
                let chunk = if kind == "playlist" { chunk[..1].to_vec() } else { chunk };
                let fallback = format!("/me/library/contains?uris={}", enc(&chunk.join(",")));
                let api = api.clone();
                let tx = self.tx.clone();
                let ctx = self.ctx.clone();
                let is_playlist = kind == "playlist";
                let rest: Vec<String> = if is_playlist { uris.iter().skip(1).cloned().collect() } else { Vec::new() };
                self.rt.spawn(async move {
                    let v = match api.get(&primary).await {
                        Ok(v) => Ok(v),
                        Err(_) => api.get(&fallback).await,
                    };
                    // A failed check isn't "not saved": ask again later.
                    let msg = match v.ok().and_then(|v| v.as_array().map(|a| a.iter().map(|b| b.as_bool().unwrap_or(false)).collect::<Vec<bool>>())) {
                        Some(flags) => Msg::Saved(chunk.iter().cloned().zip(flags.into_iter().chain(std::iter::repeat(false))).collect()),
                        None => Msg::SavedFailed(chunk),
                    };
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                });
                if is_playlist {
                    // One request per playlist: the rest are asked again if they're still on screen.
                    for u in rest {
                        self.saved_pending.remove(&u);
                    }
                    break;
                }
            }
        }
    }

    /// Fire-and-forget write. If the endpoint isn't there (404), tries `fallback`
    /// (another shape of it). Any other failure isn't sent again: the first may
    /// have gone through, and a second would do it twice.
    #[allow(clippy::too_many_arguments)]
    fn mutate(&self, method: Method, path: String, body: Option<Value>, fallback: Option<(String, Option<Value>)>, toast: Option<String>, invalidate: Vec<String>, refresh_player: bool) {
        let Some(api) = self.api.clone() else { return };
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn(async move {
            let mut r = api.send(method.clone(), &path, body).await;
            if r.as_ref().is_err_and(|e| e.to_string().starts_with("404"))
                && let Some((fp, fb)) = fallback
            {
                r = api.send(method, &fp, fb).await;
            }
            let _ = tx.send(match r {
                Ok(_) => Msg::Done { toast, invalidate, refresh_player, navigate: None },
                Err(e) => Msg::Failed(e.to_string()),
            });
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------ playback

    pub fn engine_device(&self) -> Option<&str> {
        self.engine.as_ref().map(|e| e.device_id.as_str())
    }

    /// True when playback happens in this app (not on a phone, speaker, ...).
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

    pub(crate) fn device_param(&self, first: bool) -> String {
        match self.target_device() {
            Some(d) => format!("{}device_id={}", if first { "?" } else { "&" }, enc(&d)),
            None => String::new(),
        }
    }

    pub(crate) fn local_file_active(&self) -> bool {
        self.local.as_ref().is_some_and(|l| l.active)
    }

    /// Starts or stops local files sharing to match the setting, once Glass
    /// is signed in (the account is what proves a device is yours).
    fn keep_sharing(&mut self) {
        let want = self.settings.share_local_files && !crate::window_fx::headless() && (!crate::demo::active() || crate::demo::share_folder().is_some());
        if !want {
            if self.share.take().is_some() {
                self.share_peers.clear();
                self.share_others.clear();
            }
            self.share_failed = false;
            return;
        }
        if self.share.is_some() || self.share_failed {
            return;
        }
        let Some(me) = self.me.as_ref() else { return };
        let username = s(me, "id");
        if username.is_empty() {
            return;
        }
        let instance = model::random_hex(12);
        let who = crate::share::Me { instance, name: crate::share::computer_name(), account: crate::share::account_secret(username, s(me, "email")), username: username.to_string() };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        match crate::share::Share::start(self.rt.handle().clone(), who, self.share_library.clone(), crate::share::inbox(), move |e| {
            let _ = tx.send(Msg::Share(e));
            ctx.request_repaint();
        }) {
            Ok(share) => self.share = Some(share),
            Err(e) => {
                log::warn!("local files sharing didn't start: {e:#}");
                self.share_failed = true;
            }
        }
    }

    fn share_event(&mut self, e: crate::share::Event) {
        use crate::share::{Event, State};
        match e {
            Event::Peers(p) => (self.share_peers, self.share_others) = p.into_iter().partition(|p| p.ready()),
            Event::Library { peer, songs } => {
                let mine = self.local_tracks.clone().unwrap_or_default();
                if let Some(d) = self.device.as_mut() {
                    d.answered(&peer, songs, &mine);
                }
            }
            // iOS only (the desktop finds devices on its own sockets).
            Event::NoLocalNetwork => {}
            Event::Transfer(t) => {
                if let State::Failed(why) = &t.state {
                    self.toast(format!("Couldn't {} {}: {why}", if t.sending { "send to" } else { "receive from" }, t.peer));
                } else if t.state == State::Done && t.sending && t.files > 0 {
                    self.toast(format!("Sent {} {} to {}", t.files, if t.files == 1 { "song" } else { "songs" }, t.peer));
                } else if t.state == State::Done && t.sending && t.files == 0 {
                    self.toast(format!("{} already has those songs", t.peer));
                }
                match self.transfers.iter_mut().find(|x| x.id == t.id && x.sending == t.sending && x.peer == t.peer) {
                    Some(x) => *x = t,
                    None => {
                        self.transfers.push(t);
                        if self.transfers.len() > 8 {
                            self.transfers.remove(0);
                        }
                    }
                }
            }
            Event::Received { from, paths } => {
                let n = paths.len();
                // The folder they're in joins Local Files, so they play.
                if local::add_source(&mut self.settings.local_dirs, crate::share::inbox()) {
                    self.mark_settings_dirty();
                    self.apply(Action::RescanLocal);
                } else {
                    self.add_local_files(paths);
                }
                self.toast(if n == 1 { format!("Got a song from {from}") } else { format!("Got {n} songs from {from}") });
            }
        }
    }

    // ------------------------------------------------------------ updates

    fn check_updates(&mut self, manual: bool) {
        if matches!(self.update, Update::Checking | Update::Installed) || (matches!(self.update, Update::Ready { .. }) && !manual) {
            return;
        }
        self.update = Update::Checking;
        // This copy installs updates itself only when it's a release build
        // of the Mac app, in a folder it can write to.
        let install_here = crate::update::RELEASE && cfg!(target_os = "macos") && crate::update::can_install();
        let dir = crate::config::cache_dir().join("updates");
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let http = crate::config::http().connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(30)).build().unwrap_or_default();
            let r = async {
                let Some(m) = crate::update::check(&http).await? else { return anyhow::Ok(None) };
                let zip = if install_here { Some(crate::update::download(&http, &m, &dir).await?) } else { None };
                Ok(Some((m, zip)))
            }
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Update(r, manual));
            ctx.request_repaint();
        });
    }

    fn update_checked(&mut self, r: Result<Option<(crate::update::Manifest, Option<std::path::PathBuf>)>, String>, manual: bool) {
        match r {
            Ok(None) => {
                self.update = Update::UpToDate;
                if manual {
                    self.toast(format!("Glass {} is up to date", crate::update::VERSION));
                }
            }
            Ok(Some((m, Some(zip)))) => {
                self.toast(format!("Glass {} is ready: it installs when you quit, or restart from the top bar", m.version));
                self.update = Update::Ready { version: m.version, notes: m.notes, zip };
            }
            Ok(Some((m, None))) => {
                if manual || !matches!(&self.update, Update::Available { version, .. } if *version == m.version) {
                    self.toast(format!("Glass {} is out", m.version));
                }
                self.update = Update::Available { version: m.version, notes: m.notes };
            }
            Err(e) => {
                log::info!("update check: {e}");
                if manual {
                    self.toast(format!("Couldn't check for updates: {e}"));
                }
                self.update = Update::Failed(e);
            }
        }
    }

    /// Quitting with an update downloaded: put it in place for next time.
    pub fn install_update_on_quit(&mut self) {
        if let Update::Ready { version, zip, .. } = self.update.clone()
            && crate::update::install(&zip, &version).is_ok()
        {
            let _ = std::fs::remove_file(&zip);
            self.update = Update::Installed;
        }
    }

    // ------------------------------------------------------------ the DJ

    /// The DJ has lines for a song that's starting. When `hold`, the song
    /// waits (loaded paused) until they've been said.
    fn dj_break(&mut self, uri: String, say: Vec<crate::dj::Line>, ready: Vec<crate::dj::Line>, hold: bool) {
        let session = self.engine.as_ref().map(|e| e.session.clone());
        if let Some(l) = say.last() {
            self.dj_line = Some((say.iter().map(|l| l.text()).collect::<Vec<_>>().join(" "), Instant::now()));
            log::info!("DJ before {uri}: {}", l.text());
        }
        // Get the next lines ready, so they're said without a pause.
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
        let (http, cache, tx, ctx, volume) = (self.dj_http.clone(), self.dj_clips.clone(), self.tx.clone(), self.ctx.clone(), self.pb.volume);
        self.rt.spawn(async move {
            // Never keep the song waiting long for the voice.
            let r = match tokio::time::timeout(Duration::from_secs(6), crate::dj::clips(&session, &http, &cache, &say)).await {
                Ok(r) => r.map_err(|e| format!("{e:#}")),
                Err(_) => Err("the DJ's voice took too long".to_string()),
            };
            // Opening the output and reading the clips takes a moment: not on the UI thread.
            let r = match r {
                Ok(clips) => tokio::task::spawn_blocking(move || crate::dj::Voice::play(&clips, volume)).await.map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            let _ = tx.send(Msg::DjClips(generation, r));
            ctx.request_repaint();
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

    /// The DJ is done (or skipped): its song starts.
    fn dj_resume(&mut self) {
        self.dj_gen += 1;
        self.dj_voice = None;
        self.dj_waiting = false;
        if let Some(e) = self.engine.as_ref() {
            let _ = e.spirc.play();
        }
    }

    /// Something else was chosen: the DJ stops, and the song it was ending
    /// with isn't talked about.
    pub(crate) fn dj_cancel(&mut self) {
        if self.dj_voice.is_some() || self.dj_waiting {
            self.dj_gen += 1;
            self.dj_voice = None;
            self.dj_waiting = false;
        }
        crate::dj::forget_outro();
    }

    /// What the DJ is saying (or said a moment ago), for the now-playing bar.
    pub fn dj_caption(&self) -> Option<&str> {
        let (text, at) = self.dj_line.as_ref()?;
        (self.dj_voice.is_some() || self.dj_waiting || at.elapsed() < Duration::from_secs(20)).then_some(text.as_str())
    }

    /// Another copy of Glass was started: come forward, or make way for it.
    #[cfg(unix)]
    pub fn answer_other_copies(&self, listener: std::os::unix::net::UnixListener) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        crate::instance::serve(listener, move |r| {
            let _ = tx.send(Msg::OtherCopy(r));
            ctx.request_repaint();
        });
    }

    fn other_copy(&mut self, r: crate::instance::Request) {
        match r {
            crate::instance::Request::Focus => {
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            crate::instance::Request::Quit => {
                log::info!("another Glass build started; quitting so it can take over");
                self.quitting = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Added files are read: put them where they were meant to go.
    fn files_read(&mut self, target: FileTarget, tracks: Vec<LocalTrack>) {
        if tracks.is_empty() {
            self.toast("Nothing there Glass can play (it takes MP3, M4A, AAC, FLAC, Ogg, WAV and AIFF files)");
            return;
        }
        let n = tracks.len();
        match target {
            FileTarget::Spotify { id, .. } => self.apply(Action::AddToPlaylist { playlist: id, uris: tracks.iter().map(local::spotify_uri).collect() }),
            FileTarget::Local { id, .. } => self.apply(Action::AddToLocalPlaylist { id, paths: tracks.into_iter().map(|t| t.path).collect() }),
            FileTarget::LocalFiles => self.toast(if n == 1 { "Added to Local Files".to_string() } else { format!("Added {n} songs to Local Files") }),
        }
    }

    /// Sound is coming out of this computer right now.
    pub(crate) fn playing_here(&self) -> bool {
        if self.local_file_active() {
            return self.local.as_ref().is_some_and(|l| l.is_playing());
        }
        self.is_local() && self.pb.is_playing
    }

    /// The system output changed. Spotify's engine moves over by itself (see
    /// `DspSink`); local files reopen on the new output. When the device that
    /// was playing went away (headphones unplugged or switched off), pause
    /// rather than carry on through the speakers, as Spotify does.
    fn follow_output(&mut self) {
        let crate::audio_out::Change::Moved { from, to, gone } = self.output.poll() else { return };
        log::info!("audio output: {from} -> {to}{}", if gone { " (previous output gone)" } else { "" });
        let pause = gone && self.playing_here();
        if pause {
            self.apply(Action::TogglePlay);
        }
        if let Some(l) = self.local.as_mut() {
            l.follow_output();
        }
        self.ctx.request_repaint();
    }

    // ------------------------------------------------------------ lyrics

    /// The playing song, as lyrics providers need it. Podcasts have none.
    pub fn lyrics_track(&self) -> Option<TrackRef> {
        if self.local_file_active() {
            let t = self.local.as_ref()?.current()?;
            return Some(TrackRef { key: format!("local:{}", t.path.display()), title: t.title.clone(), artists: vec![t.artist.clone()], album: t.album.clone(), duration_ms: t.duration_ms });
        }
        let it = self.pb.item.as_ref()?;
        if s(it, "type") != "track" || s(it, "uri").is_empty() {
            return None;
        }
        Some(TrackRef {
            key: s(it, "uri").to_string(),
            title: s(it, "name").to_string(),
            artists: model::arr(it, "artists").iter().map(|a| s(a, "name").to_string()).filter(|a| !a.is_empty()).collect(),
            album: s(&it["album"], "name").to_string(),
            duration_ms: model::u(it, "duration_ms"),
        })
    }

    /// Where the lyrics are: the player's position plus the timing offset.
    /// It follows seeks, pauses and resumes because it's read from the
    /// player every frame. (Glass always plays at 1×.)
    pub fn lyrics_position(&self) -> u64 {
        let pos = if self.local_file_active() { self.local.as_ref().map(|l| l.position_ms()).unwrap_or(0) } else { self.pb.position() };
        crate::lyrics::offset_position(pos, self.lyrics_offset_total())
    }

    /// The key lyrics are kept under for the playing song (its URI, or `local:` and the
    /// file's path), without building a whole `TrackRef`.
    fn lyrics_key(&self) -> Option<std::borrow::Cow<'_, str>> {
        if self.local_file_active() {
            let t = self.local.as_ref()?.current()?;
            return Some(format!("local:{}", t.path.display()).into());
        }
        let it = self.pb.item.as_ref()?;
        (s(it, "type") == "track" && !s(it, "uri").is_empty()).then(|| s(it, "uri").into())
    }

    /// The playing song's own timing offset (remembered for that song). Looked up once
    /// per song: the lyrics view asks several times a frame.
    pub fn lyrics_song_offset(&self) -> i64 {
        let Some(key) = self.lyrics_key() else { return 0 };
        if let Some((k, offset)) = &*self.song_offset.borrow()
            && *k == *key
        {
            return *offset;
        }
        let offset = self.lyrics_track().map(|t| self.lyrics_choices.offset(&t)).unwrap_or(0);
        self.song_offset.replace(Some((key.into_owned(), offset)));
        offset
    }

    /// Everything that shifts the lyrics: the general offset in config.toml
    /// (e.g. for a speaker's delay) plus this song's own.
    pub fn lyrics_offset_total(&self) -> i64 {
        self.config.lyrics().offset_ms + self.lyrics_song_offset()
    }

    /// Lyrics are only looked up while they're on screen.
    pub fn lyrics_visible(&self) -> bool {
        (self.route == Route::NowPlaying && self.np_view == NpView::Lyrics) || (self.route != Route::NowPlaying && self.settings.right_open && self.right == RightPanel::Lyrics)
    }

    fn lyrics_request(&self, track: TrackRef) -> LyricsRequest {
        let c = self.config.lyrics();
        LyricsRequest { track, setting: c.provider, fallback: c.fallback, nonce: self.lyrics_nonce }
    }

    /// Once per frame: drop a stale lookup when the song changes, and start
    /// one when lyrics are on screen and nothing has been asked yet.
    fn sync_lyrics(&mut self) {
        let key = self.lyrics_key().map(std::borrow::Cow::into_owned);
        if self.lyrics.request.is_some() && self.lyrics.track_key() != key.as_deref() {
            self.lyrics.reset();
            self.lyrics_picker = None;
        }
        if key.is_none() || !self.lyrics_visible() {
            return;
        }
        // Already asked for this song with these settings: nothing to build.
        let c = self.config.lyrics();
        if let Some(r) = &self.lyrics.request
            && (r.setting, r.fallback, r.nonce) == (c.provider, c.fallback, self.lyrics_nonce)
        {
            return;
        }
        let Some(track) = self.lyrics_track() else { return };
        let req = self.lyrics_request(track);
        if self.lyrics.wants(&req) {
            self.start_lyrics(req);
        }
    }

    fn start_lyrics(&mut self, req: LyricsRequest) {
        // The user chose lyrics (or none) for this song before: use that, no lookup.
        if let Some(choice) = self.lyrics_choices.pick(&req.track) {
            let generation = self.lyrics.begin(req);
            self.lyrics.finish(generation, lyrics_service::chosen(choice.as_ref()));
            return;
        }
        let generation = self.lyrics.begin(req.clone());
        let plan = lyrics_service::plan(req.setting, req.fallback);
        // Spotify's lyrics: Spotify songs, while connected.
        let spotify = self.spotify_source();
        let spotify_ok = if !req.track.key.starts_with("spotify:track:") {
            Err(crate::lyrics::spotify::NOT_SPOTIFY)
        } else if spotify.is_none() {
            Err(crate::lyrics::spotify::NOT_CONNECTED)
        } else {
            Ok(())
        };
        let (client, cache, tx, ctx) = (self.lyrics_client.clone(), self.lyrics_cache.clone(), self.tx.clone(), self.ctx.clone());
        let task = self.rt.spawn(async move {
            let t = req.track;
            let resolved = lyrics_service::resolve(
                &plan,
                |p| lyrics_service::availability(p, spotify_ok),
                |p| {
                    let (client, cache, t, spotify) = (client.clone(), cache.clone(), t.clone(), spotify.clone());
                    async move {
                        match p {
                            crate::lyrics::ProviderId::Spotify => crate::lyrics::spotify::fetch(spotify, &t).await,
                            _ => lyrics_service::fetch(p, &client, &cache, &t).await,
                        }
                    }
                },
            )
            .await;
            let _ = tx.send(Msg::Lyrics { generation, resolved });
            ctx.request_repaint();
        });
        self.lyrics.running(task.abort_handle());
    }

    /// Demo mode and tests: lyrics only from `cache`, never the network.
    pub fn offline_lyrics(&mut self, cache: LyricsCache) {
        self.lyrics_cache = Arc::new(cache);
        // Nothing listens on the discard port, so a lookup fails at once.
        self.lyrics_client = LrclibClient::with_base("http://127.0.0.1:9", "", "", None);
        self.lyrics_client_for = (String::from("offline"), String::new());
        self.lyrics.reset();
    }

    /// A new LRCLIB client when the contact or proxy changes.
    fn refresh_lyrics_client(&mut self) {
        let want = (self.config.lyrics().contact.clone(), self.settings.proxy.clone());
        if want != self.lyrics_client_for {
            self.lyrics_client = LrclibClient::new(&want.0, &want.1, Some(self.log.clone()));
            self.lyrics_client_for = want;
        }
    }

    pub(crate) fn set_config(&mut self, key: &str, v: toml_edit::Value) {
        if let Err(e) = self.config.set(key, v) {
            self.toast(e);
        }
    }

    fn lyrics_search(&mut self, q: String) {
        let Some(track) = self.lyrics_track() else { return };
        // Counted across pickers, so a slow search for the last song never lands in this one.
        self.lyrics_search_seq += 1;
        let generation = self.lyrics_search_seq;
        self.lyrics_picker = Some(LyricsPicker { query: q.clone(), results: None, generation });
        let (client, tx, ctx) = (self.lyrics_client.clone(), self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = client.search_text(&q).await.map_err(|e| e.to_string()).map(|mut found| {
                // Best fits first; the rest by how close their length is.
                found.sort_by(|a, b| {
                    let (sa, sb) = (crate::lyrics::matching::score(&track, a), crate::lyrics::matching::score(&track, b));
                    sb.unwrap_or(f32::MIN).total_cmp(&sa.unwrap_or(f32::MIN)).then(a.duration_ms.abs_diff(track.duration_ms).cmp(&b.duration_ms.abs_diff(track.duration_ms)))
                });
                found.iter().map(|r| r.candidate()).collect()
            });
            let _ = tx.send(Msg::LyricsSearch { generation, result });
            ctx.request_repaint();
        });
    }

    pub(crate) fn stop_local(&mut self) {
        if let Some(l) = self.local.as_mut() {
            l.stop();
        }
    }

    fn play_body(&mut self, body: Value) {
        self.start_play(PendingPlay { body, shuffle: None, quiet: false });
    }

    /// The playback engine is connecting (at sign-in, or trying again), so a Play can
    /// wait for it; not when it failed and waits to try again.
    pub(crate) fn engine_starting(&self) -> bool {
        self.engine.is_none() && self.auth_state == AuthState::SignedIn && !self.engine_status.starts_with("Playback engine failed")
    }

    /// Plays `p.body`, or does once the engine is ready.
    fn start_play(&mut self, p: PendingPlay) {
        if !self.is_premium() {
            self.needs_premium(p.quiet);
            return;
        }
        self.dj_cancel();
        self.stop_local();
        self.pb.is_playing = true;
        if !p.quiet && self.settings.now_playing_on_play && self.route != Route::NowPlaying {
            self.apply(Action::Navigate(Route::NowPlaying));
        }
        // Nothing is playing anywhere and the engine is still connecting: play here once it's ready.
        if self.pb.device.is_none() && self.engine_starting() {
            if !p.quiet {
                self.toast("Starting playback…");
            }
            self.pending_play = Some(p);
            return;
        }
        self.play_now(p.body, p.shuffle);
    }

    /// Shuffle (`shuffle`, or as it is now) and repeat for a load here.
    pub(crate) fn play_mode(&self, body: &Value, shuffle: Option<bool>) -> crate::engine::PlayMode {
        crate::engine::PlayMode::for_load(body, shuffle.unwrap_or(self.pb.shuffle), &self.pb.repeat)
    }

    fn play_now(&mut self, body: Value, shuffle: Option<bool>) {
        // This computer plays through the engine directly; other devices get a Connect command.
        let mode = self.play_mode(&body, shuffle);
        if self.is_local()
            && let Some(e) = self.engine.as_ref()
        {
            if let Err(err) = e.load(&body, mode) {
                self.toast(format!("Couldn't start playback: {err}"));
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
        if self.is_local()
            && let Some(e) = self.engine.as_ref()
        {
            // The player's own events update the bar; Spotify needn't be asked.
            local(e);
            self.local_touched = Some(Instant::now());
            return;
        }
        let sep = !path.contains('?');
        let full = format!("{path}{}", self.device_param(sep));
        self.mutate(method, full, None, None, None, vec![], true);
        self.poll_soon();
    }

    pub(crate) fn set_volume(&mut self, v: f32, commit: bool) {
        let v = v.clamp(0.0, 1.0);
        if let Some(voice) = self.dj_voice.as_ref() {
            voice.set_volume(v);
        }
        self.pb.volume = v;
        // Muted, the volume to come back to is the one kept (Glass opens at it, not silent).
        if self.muted_volume.is_none() {
            self.settings.volume = v;
            self.mark_settings_dirty();
        }
        if let Some(l) = self.local.as_mut() {
            l.set_volume(v);
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

    // ------------------------------------------------------------ actions

    pub fn apply_actions(&mut self) {
        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            self.apply(a);
        }
    }

    fn apply(&mut self, a: Action) {
        match a {
            Action::Navigate(r) => {
                if r != self.route {
                    // Now Playing sits on top of the page: going there and
                    // back leaves the page exactly as it was.
                    let overlay = r == Route::NowPlaying;
                    let state = self.page_state();
                    self.back.push((std::mem::replace(&mut self.route, r.clone()), state));
                    if self.back.len() > 100 {
                        self.back.remove(0);
                    }
                    self.fwd.clear();
                    if !overlay {
                        self.restore_page(PageState::default());
                        self.selection.clear();
                        self.local_selection.clear();
                        self.sel_anchor = None;
                        // A fresh visit starts at the top (each route keeps its own scroll offset).
                        self.scroll_top = !matches!(r, Route::Library | Route::Settings);
                    }
                    // ⌘L and the Search icon ask for the field's focus just before coming here.
                    self.focus_search &= r == Route::Search;
                    self.select_all_pending = false;
                }
                if self.fullscreen {
                    self.apply(Action::ToggleFullscreen);
                }
            }
            Action::Back => {
                if let Some((r, state)) = self.back.pop() {
                    let here = self.page_state();
                    self.fwd.push((std::mem::replace(&mut self.route, r), here));
                    self.return_to(state);
                }
            }
            Action::Forward => {
                if let Some((r, state)) = self.fwd.pop() {
                    let here = self.page_state();
                    self.back.push((std::mem::replace(&mut self.route, r), here));
                    self.return_to(state);
                }
            }
            Action::PlayContext { context, track } => {
                let mut body = json!({ "context_uri": context });
                if let Some(t) = track {
                    body["offset"] = json!({ "uri": t });
                }
                self.play_body(body);
            }
            Action::PlayUris { uris, index } => {
                // Spotify takes at most 500 songs; send the 500 around the one clicked.
                const MAX: usize = 500;
                let index = index.min(uris.len().saturating_sub(1));
                let start = index.saturating_sub(MAX / 2).min(uris.len().saturating_sub(MAX));
                let uris: Vec<String> = uris.into_iter().skip(start).take(MAX).collect();
                self.play_body(json!({ "uris": uris, "offset": { "position": index - start } }));
            }
            Action::PlayLocal { index } => {
                if self.local_tracks.is_none() {
                    // Not scanned yet (e.g. played from the library): scan, then play.
                    self.pending_local = Some(Action::PlayLocal { index });
                    self.scan_local();
                    self.toast("Reading your local files…");
                    return;
                }
                // The library itself, shared (not a copy of every song).
                let tracks = self.local_tracks.clone().unwrap_or_default();
                self.play_local_list(tracks, index);
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
                } else {
                    // Remote device: play the seed (Spotify's autoplay continues with similar tracks).
                    let kind = kind_of(&uri);
                    if kind == "track" { self.apply(Action::PlayUris { uris: vec![uri], index: 0 }) } else { self.apply(Action::PlayContext { context: uri, track: None }) }
                }
            }
            Action::TogglePlay => {
                // Pausing while the DJ talks pauses the DJ.
                if let Some(v) = self.dj_voice.as_ref() {
                    v.toggle();
                    self.pb.is_playing = !v.is_paused();
                    return;
                }
                // Paused while the voice loads: the song (held paused) stays paused, unannounced.
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
                // Settle the position under the old state first: a pause keeps the time
                // played so far, and a resume doesn't count the time spent paused.
                let playing = self.pb.is_playing;
                self.pb.progress_ms = self.pb.position();
                self.pb.progress_at = Some(Instant::now());
                self.pb.is_playing = !playing;
                if !playing
                    && self.pb.device.is_none()
                    && let Some(item) = self.pb.item.clone()
                {
                    // The device that was playing is gone: start the same song
                    // here, from the same spot.
                    let uri = s(&item, "uri").to_string();
                    let mut body = if self.pb.context_uri.is_empty() { json!({ "uris": [uri] }) } else { json!({ "context_uri": self.pb.context_uri, "offset": { "uri": uri } }) };
                    body["position_ms"] = json!(self.pb.position());
                    self.play_body(body);
                    return;
                }
                if self.is_local() && self.pb.item.is_none() {
                    // Nothing loaded yet: resume whatever the account last played, here.
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
            Action::Next => {
                self.dj_cancel();
                if self.local_file_active() {
                    if let Some(l) = self.local.as_mut() {
                        l.next();
                    }
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
                self.transport(
                    |e| {
                        let _ = e.spirc.prev();
                    },
                    Method::POST,
                    "/me/player/previous",
                );
            }
            Action::Seek(ms) => {
                // Control Center's scrubber follows.
                self.media_resync = true;
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
            Action::Volume(v) => self.set_volume(v, false),
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
                let Some(api) = self.api.clone() else { return };
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                self.rt.spawn(async move {
                    let _ = tx.send(Self::queue_uris(&api, &dev, &uris).await);
                    ctx.request_repaint();
                });
            }
            Action::QueueCollection(uri) => {
                let dev = self.device_param(false);
                let Some(api) = self.api.clone() else { return };
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                self.rt.spawn(async move {
                    let msg = match Self::collection_items(&api, &uri).await {
                        Ok(uris) if uris.is_empty() => Msg::Failed("There's nothing in it to add".into()),
                        Ok(uris) => Self::queue_uris(&api, &dev, &uris).await,
                        Err(e) => Msg::Failed(e.to_string()),
                    };
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                });
            }
            Action::ClearQueue => {
                // The Web API has no "clear queue"; restarting the current context drops queued items.
                if let Some(item) = self.pb.item.clone() {
                    let pos = self.pb.position();
                    let ctx_uri = self.pb.context_uri.clone();
                    let body = if ctx_uri.is_empty() {
                        json!({ "uris": [s(&item, "uri")], "position_ms": pos })
                    } else {
                        json!({ "context_uri": ctx_uri, "offset": { "uri": s(&item, "uri") }, "position_ms": pos })
                    };
                    // The same song carries on: no trip to Now Playing.
                    self.start_play(PendingPlay { body, shuffle: None, quiet: true });
                    self.toast("Queue cleared");
                }
            }
            Action::Transfer(id) => {
                self.stop_local();
                self.mutate(Method::PUT, "/me/player".into(), Some(json!({ "device_ids": [id], "play": self.pb.is_playing })), None, Some("Switched device".into()), vec![], true);
                self.next_devices_poll = Instant::now() + Duration::from_millis(600);
                self.poll_soon();
            }
            Action::Save { uri, save } => self.save(vec![uri], save),
            Action::SaveAll { uris, save } => self.save(uris, save),
            Action::AddToPlaylist { playlist, uris } => {
                let Some(api) = self.api.clone() else { return };
                let tx = self.tx.clone();
                let ctx = self.ctx.clone();
                let n = uris.len();
                self.rt.spawn(async move {
                    let (added, failed) = Self::add_items(&api, &playlist, &uris).await;
                    let toast = match (&failed, added) {
                        (None, 1) => "Added to playlist".to_string(),
                        (None, _) => format!("Added {n} songs to playlist"),
                        (Some(e), _) => format!("Added {added} of {n} songs; the rest couldn't be added ({e})"),
                    };
                    let _ = tx.send(match failed {
                        Some(e) if added == 0 => Msg::Failed(e),
                        _ => Msg::Done { toast: Some(toast), invalidate: vec![playlist.clone(), "/me/playlists".into()], refresh_player: false, navigate: None },
                    });
                    ctx.request_repaint();
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
                let ctx = self.ctx.clone();
                self.rt.spawn(async move {
                    let body = json!({ "name": name, "public": false, "description": "" });
                    let created = match api.send(Method::POST, "/me/playlists", Some(body.clone())).await {
                        Err(e) if e.to_string().starts_with("404") => api.send(Method::POST, &format!("/users/{}/playlists", enc(&me)), Some(body)).await,
                        r => r,
                    };
                    let msg = match created {
                        Ok(v) => {
                            let id = s(&v, "id").to_string();
                            let (_, failed) = Self::add_items(&api, &id, &uris).await;
                            let toast = match failed {
                                None => "Playlist created".to_string(),
                                Some(e) => format!("Playlist created, but not all the songs could be added ({e})"),
                            };
                            Msg::Done { toast: Some(toast), invalidate: vec!["/me/playlists".into(), id.clone()], refresh_player: false, navigate: Some(Route::Playlist(id)) }
                        }
                        Err(e) => Msg::Failed(e.to_string()),
                    };
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                });
            }
            Action::UpdatePlaylist { id, body } => {
                self.mutate(Method::PUT, format!("/playlists/{id}"), Some(body), None, Some("Playlist updated".into()), vec![id, "/me/playlists".into()], false);
            }
            Action::DeletePlaylist(id) => {
                self.mutate(Method::DELETE, format!("/playlists/{id}/followers"), None, None, Some("Removed from Your Library".into()), vec!["/me/playlists".into()], false);
                if self.route == Route::Playlist(id) {
                    self.apply(Action::Navigate(Route::Library));
                }
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
                let id = model::random_hex(16);
                self.settings.folders.push(crate::config::Folder { id: id.clone(), name, playlists: add.into_iter().collect() });
                self.expanded_folders.insert(id);
                self.mark_settings_dirty();
                self.toast("Folder created");
            }
            Action::RenameFolder { id, name } => {
                if let Some(f) = self.settings.folders.iter_mut().find(|f| f.id == id) {
                    f.name = name;
                }
                self.mark_settings_dirty();
            }
            Action::DeleteFolder(id) => {
                self.settings.folders.retain(|f| f.id != id);
                self.mark_settings_dirty();
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
            Action::Copy(text) => {
                self.ctx.copy_text(text);
                self.toast("Copied to clipboard");
            }
            Action::Toast(t) => self.toast(t),
            Action::SetRight(p) => {
                if self.settings.right_open && self.right == p {
                    self.settings.right_open = false;
                } else {
                    self.right = p;
                    self.settings.right_open = true;
                }
                if p == RightPanel::Devices {
                    self.next_devices_poll = Instant::now();
                }
            }
            Action::ToggleRight => {
                self.settings.right_open = !self.settings.right_open;
            }
            Action::ToggleMini => self.mini = !self.mini,
            Action::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
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
                    // One scan at a time (a slower, older one could finish last); this one follows.
                    self.local_rescan = true;
                } else {
                    // The old list stays on screen until the new one is ready.
                    self.scan_local();
                }
            }
            Action::EditTags(path) => {
                let track = self.local_tracks.as_ref().and_then(|t| t.iter().find(|t| t.path == path).cloned());
                let Some(track) = track.or_else(|| path.exists().then(|| local::rescan_file(&LocalTrack { path: path.clone(), ..Default::default() }))) else {
                    self.toast("That file isn't there any more");
                    return;
                };
                self.modal = Some(Modal::EditTags(Box::new(TagEditor { path, tags: local::read_tags(&track), cover: local::Cover::Keep, preview: track.cover_uri(), error: None, saving: false })));
            }
            Action::ChooseCover => {
                let Some(Modal::EditTags(ed)) = self.modal.as_mut() else { return };
                let Some(file) = rfd::FileDialog::new().add_filter("Image", &["jpg", "jpeg", "png"]).pick_file() else { return };
                match local::prepare_cover(&file) {
                    Ok(jpeg) => {
                        self.cover_previews += 1;
                        let uri = format!("bytes://cover-preview/{}.jpg", self.cover_previews);
                        self.ctx.include_bytes(uri.clone(), jpeg.clone());
                        ed.preview = Some(uri);
                        ed.cover = local::Cover::Set(Arc::new(jpeg));
                        ed.error = None;
                    }
                    Err(e) => ed.error = Some(e),
                }
            }
            Action::SaveTags => {
                let Some(Modal::EditTags(ed)) = self.modal.as_mut() else { return };
                if let Err(e) = local::validate(&ed.tags) {
                    ed.error = Some(e);
                    return;
                }
                ed.saving = true;
                let (path, tags, cover) = (ed.path.clone(), ed.tags.clone(), ed.cover.clone());
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                std::thread::spawn(move || {
                    let r = local::write_tags(&path, &tags, &cover);
                    let _ = tx.send(Msg::TagsSaved(path, r));
                    ctx.request_repaint();
                });
            }
            Action::Download { uri, name } => self.download(uri, name),
            Action::SaveSong(item) => self.save_song(*item),
            Action::SaveSet { uri, name, kind, items } => self.save_set(uri, name, kind, items),
            Action::StopSaveSet(uri) => {
                if let Some(set) = self.saving_sets.get(&uri) {
                    set.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    let name = set.name.clone();
                    self.toast(format!("Stopping “{name}” after the songs already downloading"));
                }
            }
            Action::RemoveDownload(uri) => {
                self.downloads.remove(Some(&uri), self.engine.as_ref().and_then(|e| e.session.cache().cloned()));
                self.toast("Removed download");
            }
            Action::ClearAudioCache => match self.engine.as_ref().and_then(|e| e.session.cache().cloned()) {
                Some(cache) => {
                    let d = self.downloads.clone();
                    std::thread::spawn(move || d.prune(&cache, &crate::engine::audio_cache_dir(), 0));
                    self.toast("Cleared the cache (downloads kept)");
                }
                None => self.toast("The playback engine is still starting; try again in a moment"),
            },
            Action::RemoveAllDownloads => {
                self.downloads.remove(None, self.engine.as_ref().and_then(|e| e.session.cache().cloned()));
                self.toast("Removed all downloads");
            }
            Action::LyricsRetry => {
                if let Some(k) = self.lyrics.track_key() {
                    self.lyrics_cache.forget(k);
                }
                if let Some(t) = self.lyrics_track() {
                    self.lyrics_choices.forget_pick(&t);
                    self.song_offset.replace(None);
                }
                self.lyrics_nonce += 1;
            }
            Action::LyricsPick(record) => {
                let Some(track) = self.lyrics_track() else { return };
                self.lyrics_picker = None;
                self.lyrics_nonce += 1;
                self.lyrics_choices.set_pick(&track, Some(&record));
                self.song_offset.replace(None);
                // Kept by `lyrics_choices`; shown at once.
                let resolved = lyrics_service::chosen(Some(&record));
                let generation = self.lyrics.begin(self.lyrics_request(track));
                self.lyrics.finish(generation, resolved);
            }
            Action::LyricsNoneFit => {
                if let Some(track) = self.lyrics_track() {
                    self.lyrics_choices.set_pick(&track, None);
                    self.song_offset.replace(None);
                }
                self.lyrics_picker = None;
                self.lyrics_nonce += 1;
            }
            Action::LyricsOpenPicker => {
                let Some(track) = self.lyrics_track() else { return };
                // Several matches already: show those; otherwise search.
                if let LyricsStatus::Done(res) = &self.lyrics.status
                    && let Outcome::Ambiguous { candidates, .. } = &res.outcome
                {
                    let candidates = Arc::new(candidates.clone());
                    self.lyrics_picker = Some(LyricsPicker { query: format!("{} {}", track.title, track.artist()), results: Some(Ok(candidates)), generation: 0 });
                } else {
                    self.lyrics_search(format!("{} {}", crate::lyrics::matching::search_title(&track.title), track.artist()));
                }
            }
            Action::LyricsSearch(q) => {
                if !q.trim().is_empty() {
                    self.lyrics_search(q);
                }
            }
            Action::LyricsClosePicker => self.lyrics_picker = None,
            Action::LyricsProvider(p) => self.set_config("lyrics.provider", p.name().into()),
            Action::LyricsOffset(ms) => {
                let ms = ms.clamp(-crate::user_config::OFFSET_LIMIT, crate::user_config::OFFSET_LIMIT);
                self.set_config("lyrics.offset_ms", ms.into());
            }
            Action::LyricsTranslation(m) => self.set_config("lyrics.translation", m.name().into()),
            Action::LyricsSongOffset(ms) => {
                if let Some(t) = self.lyrics_track() {
                    let ms = ms.clamp(-crate::user_config::OFFSET_LIMIT, crate::user_config::OFFSET_LIMIT);
                    self.lyrics_choices.set_offset(&t, ms);
                    self.song_offset.replace(None);
                }
            }
            Action::ClearLyricsCache => {
                match self.lyrics_cache.clear() {
                    Ok(()) => self.toast("Cleared saved lyrics"),
                    Err(e) => self.toast(format!("Couldn't clear saved lyrics: {e}")),
                }
                self.lyrics_nonce += 1;
            }
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
            Action::AiSetBilling(b) => {
                self.ai_cancel();
                self.set_config("assistant.billing", b.name().into());
                self.ai.models = crate::ai::Models::Unknown;
                self.ai.models_for.clear();
            }
            Action::AiEnable(on) => {
                if !on {
                    self.ai_cancel();
                }
                self.set_config("assistant.enabled", on.into());
            }
            Action::AiSaveApiKey => self.ai_save_api_key(),
            Action::AiForgetApiKey => self.ai_forget_api_key(),
            Action::AiDismissWelcome => self.modal = None,
            Action::AiLimitAcknowledged => self.ai.limit_reached = false,
            Action::AiUseSpotify(on) => {
                self.ai_cancel();
                self.set_config("assistant.spotify", on.into());
            }
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
                // The page stays: it (like the list of playlists) offers the undo.
                Ok(()) => self.toast("Playlist deleted (undo is on this page)"),
                Err(e) => self.toast(e),
            },
            Action::AddToLocalPlaylist { id, paths } => {
                let name = self.playlists.get(&id).map(|p| p.name.clone()).unwrap_or_default();
                match self.playlists.append(&id, &paths) {
                    Ok(0) => self.toast(if paths.len() == 1 { format!("Already in “{name}”") } else { format!("Those songs are already in “{name}”") }),
                    Ok(1) => self.toast(format!("Added to “{name}”")),
                    Ok(n) => self.toast(format!("Added {n} songs to “{name}”")),
                    Err(e) => self.toast(e),
                }
            }
            Action::CreateLocalPlaylist { name, paths } => match self.playlists.create(&name, "", paths, None) {
                Ok((id, _)) => {
                    self.toast("Local playlist created");
                    self.apply(Action::Navigate(Route::LocalPlaylist(id)));
                }
                Err(e) => self.toast(e),
            },
            Action::RemoveFromLocalPlaylist { id, index } => match self.playlists.remove_at(&id, index) {
                Ok(()) => self.toast("Removed from playlist (undo is on its page)"),
                Err(e) => self.toast(e),
            },
            Action::PlayLocalTracks { indices, start } => {
                let Some(all) = self.local_tracks.clone() else { return };
                let tracks: Vec<LocalTrack> = indices.iter().filter_map(|&i| all.get(i).cloned()).collect();
                if !tracks.is_empty() {
                    let start = start.min(tracks.len() - 1);
                    self.play_local_list(tracks, start);
                }
            }
            Action::CheckForUpdates => self.check_updates(true),
            Action::RestartToUpdate => {
                let Update::Ready { version, zip, .. } = self.update.clone() else { return };
                match crate::update::install(&zip, &version) {
                    Ok(bundle) => {
                        let _ = std::fs::remove_file(&zip);
                        self.update = Update::Installed;
                        self.save_now();
                        crate::update::relaunch(&bundle);
                    }
                    Err(e) => self.toast(format!("Couldn't update: {e}")),
                }
            }
            Action::SendToDevice { peer, paths } => {
                let (Some(share), Some(p)) = (self.share.as_ref(), self.share_peers.iter().find(|p| p.id == peer).cloned()) else {
                    self.toast("That device isn't on this network any more");
                    return;
                };
                let all = self.local_tracks.as_deref().map(Vec::as_slice).unwrap_or_default();
                let wanted: HashSet<&std::path::PathBuf> = paths.iter().collect();
                let tracks: Vec<LocalTrack> = all.iter().filter(|t| wanted.contains(&t.path)).cloned().collect();
                let n = tracks.len();
                share.send(p.clone(), tracks);
                self.toast(if n == 1 { format!("Sending to {}…", p.name) } else { format!("Sending {n} songs to {}…", p.name) });
            }
            Action::BrowseDevice(peer) => {
                self.device_picks.clear();
                let Some(p) = self.share_peers.iter().find(|p| p.id == peer).cloned() else {
                    let name = self.device.as_ref().filter(|d| d.peer == peer).map_or_else(|| "That device".to_string(), |d| d.name.clone());
                    self.device = Some(crate::share::Browsed {
                        peer,
                        name,
                        songs: Some(Err("It isn't on this network any more. Open Glass on it, on the same Wi-Fi as this computer.".into())),
                        here: Vec::new(),
                    });
                    return;
                };
                self.device = Some(crate::share::Browsed::new(&p));
                if let Some(share) = self.share.as_ref() {
                    share.browse(p);
                } else if crate::demo::active() {
                    self.share_event(crate::share::Event::Library { peer: p.id, songs: Ok(crate::demo::device_songs()) });
                }
            }
            Action::GetFromDevice { peer, ids } => {
                let Some(p) = self.share_peers.iter().find(|p| p.id == peer).cloned() else {
                    self.toast("That device isn't on this network any more");
                    return;
                };
                let n = ids.len();
                match self.share.as_ref() {
                    Some(share) => share.get(p.clone(), ids),
                    None if crate::demo::active() => {}
                    None => return self.toast("Turn on sharing with your other devices in Settings"),
                }
                self.device_picks.clear();
                self.toast(if n == 1 { format!("Getting a song from {}…", p.name) } else { format!("Getting {n} songs from {}…", p.name) });
            }
            Action::GetFromDevices(uris) => match self.share.as_ref() {
                Some(share) if !self.share_peers.is_empty() => {
                    let n = uris.len();
                    share.fetch(uris);
                    self.toast(if n == 1 { "Getting it from your other devices…".to_string() } else { format!("Getting {n} songs from your other devices…") });
                }
                _ => self.toast("None of your other devices with Glass are on this network"),
            },
            Action::AddFiles { paths, target } => {
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                let inbox = self.settings.save_dir.clone().unwrap_or_else(crate::save_song::default_dir);
                self.rt.spawn_blocking(move || {
                    // A file waiting in a temporary folder would vanish: it's kept in Glass's folder.
                    let paths = local::keep_temporary(paths, &inbox);
                    let tracks = local::read_paths(&paths);
                    let _ = tx.send(Msg::FilesRead(target, paths, tracks));
                    ctx.request_repaint();
                });
            }
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
            Action::LogOut => {
                auth::forget_session();
                self.share = None;
                self.share_peers.clear();
                self.share_others.clear();
                self.device = None;
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
                self.lyrics.reset();
                self.lyrics_picker = None;
                self.pb = Playback { volume: self.settings.volume, repeat: "off".into(), ..Default::default() };
                self.saved_wanted.clear();
                self.saved_pending.clear();
                self.used_at.clear();
                self.pending_play = None;
                self.engine_uri.clear();
                self.back.clear();
                self.fwd.clear();
                self.dj_voice = None;
                self.engine_retry = None;
                self.engine_launching = false;
                self.engine_status = "Not started".into();
                self.auth_state = AuthState::SignedOut;
            }
        }
    }

    /// Adds songs to a playlist, 100 a request. Returns how many went in, and the error
    /// that stopped the rest (those before it stay added).
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

    /// Adds songs to the queue, one request each (Spotify takes one at a time), up to 100.
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

    /// The songs (or episodes) in a collection, in order, without local files.
    async fn collection_items(api: &Api, uri: &str) -> anyhow::Result<Vec<String>> {
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

    fn download(&mut self, uri: String, name: String) {
        if !self.is_premium() {
            self.toast("Downloading to play offline needs Spotify Premium. Share ▸ Download as M4A or MP3 keeps a copy without it.");
            return;
        }
        let Some(api) = self.api.clone() else { return };
        let d = self.downloads.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn(async move {
            let msg = match Self::collection_items(&api, &uri).await {
                Ok(items) => {
                    let n = items.len();
                    d.add(&uri, &name, items);
                    Msg::Done { toast: Some(format!("Downloading {n} {}", if n == 1 { "item" } else { "items" })), invalidate: vec![], refresh_player: false, navigate: None }
                }
                Err(e) => Msg::Failed(e.to_string()),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    /// A downloaded playlist or Liked Songs that changed gets its new items
    /// downloaded too, as in Spotify.
    fn sync_download(&self, key: &str, v: &Value) {
        let (all, path) = key.split_once(':').unwrap_or(("", key));
        // Only a whole list (every item, up to the cap it was asked with) replaces a
        // download's songs: those it doesn't have are deleted.
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
        // What a download keeps (local files and unavailable entries are left out).
        let items: Vec<String> =
            model::items(v).iter().map(|i| s(model::playlist_entry(i), "uri").to_string()).filter(|u| u.starts_with("spotify:track:") || u.starts_with("spotify:episode:")).collect();
        let name = self.downloads.collections().into_iter().find(|(u, _)| *u == uri).map(|(_, n)| n).unwrap_or_default();
        if self.downloads.progress(&uri).map(|(_, total)| total) != Some(items.len()) || items.iter().any(|u| !self.downloads.has_file(u)) {
            self.downloads.add(&uri, &name, items);
        }
    }

    /// Likes, saves or follows `uris`, all of one kind (a selection is songs, or
    /// episodes): 50 to a request, as Spotify takes them, and one refresh at the end.
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
        // The hearts change at once, and change back if Spotify says no.
        for u in &uris {
            self.saved.insert(u.clone(), save);
        }
        let Some(api) = self.api.clone() else { return };
        let (tx, ctx, m, inval) = (self.tx.clone(), self.ctx.clone(), if save { Method::PUT } else { Method::DELETE }, inval.to_string());
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
                let _ = tx.send(Msg::Done { toast: failed.is_none().then_some(toast), invalidate: vec![inval], refresh_player: false, navigate: None });
            }
            if let Some(e) = failed {
                let _ = tx.send(Msg::Saved(uris[done..].iter().map(|u| (u.clone(), !save)).collect()));
                let _ = tx.send(Msg::Failed(e));
            }
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------ auth/engine

    pub fn sign_in(&mut self) {
        self.auth_state = AuthState::SigningIn;
        self.auth_error = None;
        self.signin_gen += 1;
        let gen_ = self.signin_gen;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let id = self.settings.client_id.clone();
        self.rt.spawn(async move {
            let r = Auth::sign_in(&id).await.map(Arc::new).map_err(|e| e.to_string());
            let _ = tx.send(Msg::SignedIn(gen_, r));
            ctx.request_repaint();
        });
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
        let ctx = self.ctx.clone();
        self.rt.spawn(async move {
            let ev_tx = tx.clone();
            let ev_ctx = ctx.clone();
            let r = async {
                let token = api.auth().bearer().await?;
                Engine::start(&settings, token, dsp, move |e| {
                    let _ = ev_tx.send(Msg::Engine(e));
                    ev_ctx.request_repaint();
                })
                .await
            }
            .await;
            let _ = tx.send(Msg::EngineReady(r.map_err(|e| e.to_string())));
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------ per-frame logic

    pub fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }
        // Back in front: catch up with what other devices did meanwhile.
        let focused = self.ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            self.next_poll = Instant::now();
        }
        self.was_focused = focused;
        if self.auth_state == AuthState::SignedIn {
            self.poll();
            if let Some((at, n)) = self.engine_retry
                && self.engine.is_none()
                && Instant::now() >= at
            {
                // Marked far off until this attempt answers.
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
        // Nothing left to play: let go of the output, which keeps pulling silence while it's open.
        if self.local.as_ref().is_some_and(|l| !l.active) {
            self.local = None;
        }
        self.follow_output();
        self.keep_sharing();
        // Release builds look for updates shortly after starting, then every 6 hours.
        if self.settings.auto_update
            && crate::update::RELEASE
            && Instant::now() >= self.next_update_check
            && !crate::window_fx::headless()
            && (!crate::demo::active() || std::env::var_os("GLASS_UPDATE_FEED").is_some())
        {
            self.next_update_check = Instant::now() + Duration::from_secs(6 * 3600);
            self.check_updates(false);
        }
        crate::dj::SHARED.voice_on.store(self.settings.dj_voice, std::sync::atomic::Ordering::Relaxed);
        if self.dj_voice.as_ref().is_some_and(|v| v.done()) {
            self.dj_resume();
        }
        if self.dj_voice.is_some() || self.dj_waiting {
            self.ctx.request_repaint_after(Duration::from_millis(150));
        }
        if self.theme.poll() {
            self.ctx.request_repaint();
        }
        // The window's blur follows the theme, whether the file, the theme panel or a reload changed it.
        if self.window_blur != Some(self.theme.theme().blur) {
            self.window_blur = Some(self.theme.theme().blur);
            crate::window_fx::apply(self.theme.theme());
        }
        if self.config.poll() {
            self.refresh_lyrics_client();
            self.ctx.request_repaint();
        }
        self.sync_lyrics();
        self.sync_translation();
        let (panel, settings) = (self.ai_panel_visible(), self.route == Route::Settings);
        self.ai_tick(panel, settings);
        self.flush_saved_queries();
        // Handed to the audio thread only when it changes (that takes a lock it shares).
        let dsp = DspState { eq_enabled: self.settings.eq_enabled, gains: self.settings.eq_gains, mono: self.settings.mono };
        if self.dsp_sent != Some(dsp) {
            self.dsp_sent = Some(dsp);
            self.dsp.set(dsp);
        }
        self.update_media();
        self.maybe_skip_explicit();
        if self.settings.show_local_files && self.local_tracks.is_none() && self.route == Route::LocalFiles {
            self.scan_local();
        }
        if let Some(t) = self.settings_dirty
            && t.elapsed() > Duration::from_millis(800)
        {
            self.settings.save();
            self.settings_dirty = None;
        }
        if self.last_prune.elapsed() > Duration::from_secs(600) {
            let cache = self.engine.as_ref().and_then(|e| e.session.cache().cloned());
            self.prune_cache(cache);
        }
        self.toasts.retain(|(_, t)| t.elapsed() < Duration::from_secs(3));
        // Playing: keep the progress bar moving, twice a second in front. Behind other
        // windows it moves every 5 s (Now Playing, which you watch, every second): each
        // frame keeps the graphics driver's ~50 MB of buffers in use, and after a few
        // seconds without one macOS counts them as reclaimable. Otherwise nothing changes
        // on its own: wake for the next poll (the network and the player wake the window).
        let local_playing = self.local.as_ref().is_some_and(|l| l.is_playing());
        if self.pb.is_playing || local_playing {
            let focused = self.ctx.input(|i| i.focused);
            let every = if focused {
                500
            } else if self.route == Route::NowPlaying {
                1000
            } else {
                5000
            };
            self.ctx.request_repaint_after(Duration::from_millis(every));
        } else {
            let next_poll = self.next_poll.saturating_duration_since(Instant::now());
            self.ctx.request_repaint_after(next_poll.clamp(Duration::from_millis(250), Duration::from_secs(30)));
        }
        if let Some((_, shown)) = self.toasts.first() {
            self.ctx.request_repaint_after(Duration::from_secs(3).saturating_sub(shown.elapsed()));
        }
        if self.last_evict.elapsed() > Duration::from_secs(30) {
            self.last_evict = Instant::now();
            self.evict_unused();
            crate::ui::widgets::forget_unused_images(&self.ctx);
            let api = self.api.clone();
            self.rt.spawn(async move {
                if let Some(api) = api {
                    api.trim();
                }
                crate::heap::trimmed();
            });
        }
    }

    /// Plays a list of local files (pausing Spotify playback first). False
    /// when there's no audio output.
    pub(crate) fn play_local_list(&mut self, tracks: impl Into<Arc<Vec<LocalTrack>>>, index: usize) -> bool {
        let tracks = tracks.into();
        if self.local.is_none() {
            self.local = LocalPlayer::new();
        }
        if self.is_local() {
            if let Some(e) = self.engine.as_ref() {
                let _ = e.spirc.pause();
            }
        } else if self.pb.is_playing {
            self.mutate(Method::PUT, format!("/me/player/pause{}", self.device_param(true)), None, None, None, vec![], true);
        }
        self.pb.is_playing = false;
        let vol = self.pb.volume;
        match self.local.as_mut() {
            Some(l) => {
                l.set_volume(vol);
                l.play(tracks, index);
                true
            }
            None => {
                self.toast("Couldn't open an audio output device");
                false
            }
        }
    }

    /// The local file a playlist's `spotify:local:` entry refers to. `None`
    /// while the folders haven't been read yet, `Some(None)` when it isn't here.
    pub fn local_link(&mut self, uri: &str) -> Option<Option<LocalTrack>> {
        let tracks = self.local_tracks.as_ref()?;
        let i = match self.local_links.get(uri) {
            Some(i) => *i,
            None => *self.local_links.entry(uri.to_string()).or_insert_with(|| local::find(tracks, uri)),
        };
        Some(i.and_then(|i| tracks.get(i).cloned()))
    }

    /// The cover of what's playing (a local file's own, or the album's at about `size` px).
    pub fn now_cover(&mut self, size: u64) -> Option<String> {
        if let Some(t) = self.local.as_ref().and_then(|l| l.current()) {
            return t.cover_uri();
        }
        let item = self.pb.item.clone()?;
        self.cover_of(&item, size)
    }

    /// Cover art for a song: a Spotify image, or for a playlist's local song
    /// the cover embedded in the file it's linked to.
    pub fn cover_of(&mut self, item: &Value, size: u64) -> Option<String> {
        if model::b(item, "is_local") {
            return self.local_link(s(item, "uri")).flatten().and_then(|t| t.cover_uri());
        }
        model::image(item, size)
    }

    pub(crate) fn scan_local(&mut self) {
        if self.local_scanning {
            return;
        }
        self.local_scanning = true;
        let dirs = self.settings.local_dirs.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::LocalScanned(local::scan(&dirs)));
            ctx.request_repaint();
        });
    }

    /// Trims the streaming cache to the user's limit, off the UI thread.
    fn prune_cache(&mut self, cache: Option<Arc<librespot_core::cache::Cache>>) {
        self.last_prune = Instant::now();
        let Some(cache) = cache else { return };
        let d = self.downloads.clone();
        let limit = self.settings.cache_limit_mb * 1024 * 1024;
        std::thread::spawn(move || d.prune(&cache, &crate::engine::audio_cache_dir(), limit));
    }

    /// Linux: the window opens at the size it last had.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn remember_window_size(&mut self) {
        let Some(size) = self.ctx.input(|i| i.viewport().inner_rect.map(|r| r.size())) else { return };
        let changed = self.settings.window_size.is_none_or(|[w, h]| (w - size.x).abs() > 1.0 || (h - size.y).abs() > 1.0);
        if changed && size.x >= 900.0 && size.y >= 600.0 && !self.fullscreen {
            self.settings.window_size = Some([size.x, size.y]);
            self.mark_settings_dirty();
        }
    }

    pub fn save_now(&mut self) {
        self.settings.save();
        self.settings_dirty = None;
    }

    fn poll(&mut self) {
        let Some(api) = self.api.clone() else { return };
        let now = Instant::now();
        // The account, if the first try failed (it's needed for Liked Songs and editing).
        if self.me.is_none() && now >= self.next_me_poll {
            self.next_me_poll = now + Duration::from_secs(15);
            let (tx, ctx, a, generation) = (self.tx.clone(), self.ctx.clone(), api.clone(), self.data_gen);
            self.rt.spawn(async move {
                let r = a.get("/me").await.map(Arc::new).map_err(|e| e.to_string());
                let _ = tx.send(Msg::Data { key: "/me".into(), result: r, generation });
                ctx.request_repaint();
            });
        }
        if now >= self.next_poll && !self.player_polling {
            self.player_polling = true;
            // Spotify pushes each change to the player state, and Glass looks at it
            // then (`Msg::PlayerPushed`); this is only in case a push goes missing.
            // Each look draws a frame (see `tick`), and macOS counts the graphics
            // driver's buffers as reclaimable only after a few seconds without one,
            // so an idle window looks rarely. Coming back to the window looks at once.
            let focused = self.ctx.input(|i| i.focused);
            self.next_poll = now
                + if self.pb.is_playing {
                    Duration::from_secs(15)
                } else if focused {
                    Duration::from_secs(30)
                } else {
                    Duration::from_secs(60)
                };
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            let a = api.clone();
            self.rt.spawn(async move {
                // Answered even if this dies halfway, or polling would stop for good.
                let failed = Answer::new(tx.clone(), ctx.clone(), Msg::Player(Err("the request failed".into())));
                let r = a.get("/me/player?additional_types=episode").await.map_err(|e| e.to_string());
                failed.disarm();
                let _ = tx.send(Msg::Player(r));
                ctx.request_repaint();
            });
        }
        // The queue and devices show on the now-playing screen or in the side panel.
        let np = self.route == Route::NowPlaying;
        let side = self.settings.right_open && !np;
        // With album colors, the queue is also read once near the end of each song, so the
        // next song's cover and colors are loaded before it starts.
        let left_ms = self.pb.item.as_ref().map(|i| model::u(i, "duration_ms")).unwrap_or(0).saturating_sub(self.pb.position());
        let ahead = self.settings.album_colors && self.pb.is_playing && left_ms > 0 && left_ms < 25_000 && self.queue_read_for != self.pb.uri();
        if ahead {
            self.queue_read_for = self.pb.uri().to_string();
            self.next_queue_poll = now;
        }
        let want_queue = (np && self.np_view == NpView::Queue) || (side && self.right == RightPanel::Queue) || ahead;
        if want_queue && now >= self.next_queue_poll {
            self.next_queue_poll = now + Duration::from_secs(8);
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            let a = api.clone();
            self.rt.spawn(async move {
                let r = a.get("/me/player/queue").await.map_err(|e| e.to_string());
                let _ = tx.send(Msg::Queue(r));
                ctx.request_repaint();
            });
        }
        let want_devices = (np && self.np_view == NpView::Devices) || (side && self.right == RightPanel::Devices);
        if want_devices && !self.devices_shown {
            self.next_devices_poll = now;
        }
        self.devices_shown = want_devices;
        if now >= self.next_devices_poll {
            self.next_devices_poll = now + Duration::from_secs(if want_devices { 5 } else { 60 });
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            self.rt.spawn(async move {
                if let Ok(v) = api.get("/me/player/devices").await {
                    let _ = tx.send(Msg::Devices(v));
                    ctx.request_repaint();
                }
            });
        }
    }

    fn handle(&mut self, msg: Msg) {
        // Signed out: answers to the old session's requests still on their way are dropped
        // (a late player state or engine would fill the old account back in).
        if self.api.is_none() && matches!(msg, Msg::Data { .. } | Msg::Player(_) | Msg::Queue(_) | Msg::Devices(_) | Msg::Saved(..) | Msg::SavedFailed(..) | Msg::EngineReady(_) | Msg::Engine(_)) {
            return;
        }
        match msg {
            Msg::SignedIn(g, _) if g != self.signin_gen => {}
            Msg::SignedIn(_, Ok(auth)) => {
                self.resume_retry = None;
                self.auth_error = None;
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                let pushed: crate::sp::Changed = Arc::new(move || {
                    let _ = tx.send(Msg::PlayerPushed);
                    ctx.request_repaint();
                });
                self.api = Some(Api::new(auth, &self.settings.proxy, self.log.clone()).on_player_change(pushed));
                self.auth_state = AuthState::SignedIn;
                let tx = self.tx.clone();
                let ctx = self.ctx.clone();
                let api = self.api.clone().unwrap();
                let generation = self.data_gen;
                // The engine starts once this says what kind of account it is
                // (`account_known`): Spotify streams only to Premium accounts.
                self.rt.spawn(async move {
                    let r = api.get("/me").await.map(Arc::new).map_err(|e| e.to_string());
                    let _ = tx.send(Msg::Data { key: "/me".into(), result: r, generation });
                    ctx.request_repaint();
                });
            }
            Msg::SignedIn(g, Err(e)) => {
                // Spotify turning the saved session down means signing in again. Anything else
                // (no network yet when Glass opens at login, say) is tried again by itself.
                let refused = ["no saved session", "invalid_grant", "invalid_client", "invalid_request", "unauthorized_client"].iter().any(|w| e.contains(w));
                let tries = self.resume_retry.map_or(0, |(_, n)| n);
                if g == 0 && !refused && tries < 8 {
                    let wait = [2, 5, 15, 30][(tries as usize).min(3)];
                    self.resume_retry = Some((Instant::now() + Duration::from_secs(wait), tries + 1));
                    self.ctx.request_repaint_after(Duration::from_secs(wait));
                    self.auth_error = Some("Can't reach Spotify. Trying again…".into());
                } else {
                    self.resume_retry = None;
                    self.auth_error = Some(e);
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
                        let stale = self.invalidated.iter().any(|(g, needle)| *g > generation && key.contains(needle.as_str()));
                        if stale {
                            self.fetched_at.remove(&key);
                        } else {
                            self.fetched_at.insert(key.clone(), Instant::now());
                        }
                        self.data.insert(key, Load::Ready(v));
                    }
                    // A failed background refresh keeps the old data.
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
            // The state is already here (pushed): reading it takes no request. A burst
            // of pushes is one look, made later this frame.
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
                self.queue = Some(Arc::new(v));
                self.queue_error = None;
            }
            Msg::Queue(Err(e)) => {
                // Usually "not ready yet" right after connecting: try again soon.
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
                    self.saved.insert(u, b);
                }
            }
            Msg::EngineReady(Ok(_)) if !self.is_premium() => {
                self.engine_launching = false;
                self.engine_status = NEEDS_PREMIUM.into();
            }
            Msg::EngineReady(Ok(engine)) => {
                self.engine_launching = false;
                self.engine_retry = None;
                self.engine_status = format!("Ready · this computer appears as “{}”", self.settings.device_name);
                let ctx = self.ctx.clone();
                let _guard = self.rt.enter();
                self.downloads.run(engine.session.clone(), self.settings.streaming_quality, move || ctx.request_repaint());
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
                // The assistant said it would play once the player was ready: say it didn't.
                if let Some(p) = self.pending_play.take() {
                    // It isn't going to play: the button stops showing that it is.
                    self.pb.is_playing = false;
                    if p.quiet {
                        self.ai.note(format!("Couldn't start playback on this computer: {e}."));
                        self.ai.tell_model(format!("Playback on this computer couldn't start ({e}), so nothing is playing."));
                    }
                }
                // Tried again by itself, less and less often (the network may be down for a while).
                let tries = self.engine_retry.map_or(0, |(_, n)| n);
                let wait = [5, 15, 30, 60, 120][(tries as usize).min(4)];
                self.engine_retry = Some((Instant::now() + Duration::from_secs(wait), tries + 1));
                self.ctx.request_repaint_after(Duration::from_secs(wait));
                self.engine_status = format!("Playback engine failed: {e}. Trying again in {wait} s.");
                if tries == 0 {
                    self.toast("Can't play on this computer right now; other devices still work");
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
                    self.apply(Action::Navigate(r));
                }
            }
            Msg::Failed(e) => {
                let short = if e.contains("403") && e.contains("player") {
                    "Controlling playback needs Spotify Premium".to_string()
                } else if e.contains("404") && e.contains("player") {
                    "No active device — pick one in the device panel".to_string()
                } else {
                    e
                };
                self.toast(short);
                self.poll_soon();
            }
            Msg::FilesRead(target, paths, tracks) => {
                // Each file plays from Local Files: its folder becomes a source.
                let new = local::missing_sources(&paths, &self.settings.local_dirs);
                if !new.is_empty() {
                    self.settings.local_dirs.extend(new);
                    self.mark_settings_dirty();
                    self.apply(Action::RescanLocal);
                }
                self.files_read(target, tracks)
            }
            Msg::Share(e) => self.share_event(e),
            Msg::DjClips(generation, r) => self.dj_clips_ready(generation, r),
            Msg::Update(r, manual) => self.update_checked(r, manual),
            Msg::OtherCopy(r) => self.other_copy(r),
            Msg::Media(e) => {
                use souvlaki::{MediaControlEvent as M, SeekDirection};
                // Play and Pause mean exactly that: AirPods send Pause when
                // they come off, and that must never start paused music.
                let playing = if self.local_file_active() { self.local.as_ref().is_some_and(|l| l.is_playing()) } else { self.pb.is_playing };
                let a = match e {
                    M::Play => (!playing).then_some(Action::TogglePlay),
                    M::Pause | M::Stop => playing.then_some(Action::TogglePlay),
                    M::Toggle => Some(Action::TogglePlay),
                    M::Next => Some(Action::Next),
                    M::Previous => Some(Action::Prev),
                    M::Seek(SeekDirection::Forward) => Some(Action::SeekBy(10_000)),
                    M::Seek(SeekDirection::Backward) => Some(Action::SeekBy(-10_000)),
                    M::SeekBy(SeekDirection::Forward, d) => Some(Action::SeekBy(d.as_millis() as i64)),
                    M::SeekBy(SeekDirection::Backward, d) => Some(Action::SeekBy(-(d.as_millis() as i64))),
                    M::SetPosition(p) => Some(Action::Seek(p.0.as_millis() as u64)),
                    M::SetVolume(v) => Some(Action::Volume(v as f32)),
                    _ => None,
                };
                if let Some(a) = a {
                    self.apply(a);
                }
            }
            Msg::Ai { generation, event } => self.ai_event(generation, event),
            Msg::AiWrote(w) => self.ai_wrote(w),
            Msg::LyricsTranslated { key, result } => self.lyrics_translated(key, result),
            Msg::AiRegistered(id) => self.ai_registered(id),
            Msg::AiSignedIn(r) => self.ai_signed_in(r),
            Msg::AiModels { key, result } => self.ai_models_arrived(key, result),
            Msg::AiSignedOut { confirmed, .. } => self.ai_signed_out(confirmed),
            Msg::SaveSong(uri, status) => self.song_save_moved(uri, status),
            Msg::TagsSaved(path, result) => match result {
                Ok(()) => {
                    if let Some(Modal::EditTags(ed)) = &self.modal
                        && ed.path == path
                    {
                        self.modal = None;
                    }
                    let uri = format!("{}{}", local::COVER_SCHEME, path.display());
                    self.ctx.forget_image(&uri);
                    let refresh = |t: &mut LocalTrack| {
                        if t.path == path {
                            *t = local::rescan_file(t);
                        }
                    };
                    if let Some(list) = self.local_tracks.as_mut() {
                        Arc::make_mut(list).iter_mut().for_each(refresh);
                        // Sharing answers from the same list.
                        if let Ok(mut lib) = self.share_library.write() {
                            *lib = list.clone();
                        }
                    }
                    if let Some(l) = self.local.as_mut() {
                        Arc::make_mut(&mut l.queue).iter_mut().for_each(refresh);
                    }
                    self.local_links.clear();
                    self.toast("Saved song info");
                }
                Err(e) => {
                    if let Some(Modal::EditTags(ed)) = self.modal.as_mut() {
                        ed.saving = false;
                        ed.error = Some(e);
                    } else {
                        self.toast(format!("Song info wasn't saved. {e}"));
                    }
                }
            },
            Msg::LocalAdded { base, all } => {
                // Made from the library as it was: if it has changed since, read it all.
                if self.local_tracks.as_ref().is_some_and(|t| Arc::ptr_eq(t, &base)) && !self.local_scanning {
                    self.set_local_tracks(Arc::new(all));
                } else {
                    self.act(Action::RescanLocal);
                }
            }
            Msg::LocalScanned(t) => {
                self.set_local_tracks(Arc::new(t));
                if let Some(play) = self.pending_local.take() {
                    self.apply(play);
                }
                self.local_scanning = false;
                // Asked for again while that scan ran (a song was saved, say): once more.
                if std::mem::take(&mut self.local_rescan) {
                    self.scan_local();
                }
            }
            Msg::Lyrics { generation, resolved } => {
                self.lyrics.finish(generation, resolved);
            }
            Msg::LyricsSearch { generation, result } => {
                if let Some(p) = self.lyrics_picker.as_mut().filter(|p| p.generation == generation) {
                    p.results = Some(result.map(Arc::new));
                }
            }
            Msg::Artwork(uri, path) => {
                // Only the current song's cover is kept (a late one for the last song goes too).
                let keep = (uri == self.media_uri).then_some(path.clone());
                if let Ok(entries) = std::fs::read_dir(crate::config::cache_dir().join("artwork")) {
                    for e in entries.flatten() {
                        if Some(e.path()) != keep {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
                if keep.is_some() {
                    self.media_cover = Some(path);
                    self.media_uri.clear(); // re-send metadata, now with the cover
                }
            }
        }
    }

    fn apply_player_state(&mut self, v: Value) {
        if v.is_null() || v.get("device").is_none() {
            // Nothing is active anywhere. Keep the spot it stopped at, for Play to resume from.
            self.pb.progress_ms = self.pb.position();
            self.pb.progress_at = Some(Instant::now());
            self.pb.device = None;
            self.pb.is_playing = false;
            return;
        }
        let prev_uri = self.pb.uri().to_string();
        let item = v.get("item").filter(|i| !i.is_null()).cloned();
        // While this computer plays, its player knows the song: Spotify's
        // report can still name the one before.
        let here = self.engine_device().is_some_and(|e| v["device"]["id"].as_str() == Some(e));
        let stale = here && !self.engine_uri.is_empty() && item.as_ref().is_some_and(|i| s(i, "uri") != self.engine_uri);
        if !stale && (item.is_some() || self.pb.item.is_none()) {
            self.pb.item = item.map(Arc::new);
        }
        // Right after this computer's player changed something, Spotify may
        // still report the old state; the engine's own events are newer.
        let fresh_local = self.is_local() && self.local_touched.is_some_and(|t| t.elapsed() < Duration::from_millis(2500));
        // Playing and position here come from the player itself, as they happen.
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
        if !self.pb.volume_dragging
            && self.muted_volume.is_none()
            && let Some(vp) = v["device"]["volume_percent"].as_u64()
        {
            self.pb.volume = vp as f32 / 100.0;
        }
        if self.pb.uri() != prev_uri {
            self.next_queue_poll = Instant::now();
            self.on_track_changed();
        }
    }

    fn handle_engine(&mut self, ev: EngineEvent) {
        // These matter whether or not this computer is the one playing.
        match ev {
            EngineEvent::SessionLost => {
                self.engine_status = "Disconnected; reconnecting…".into();
                // If this computer was the one playing, it isn't a device any more: a Play
                // meanwhile waits for the engine instead of going to the old device.
                if self.is_local() {
                    self.pb.device = None;
                }
                self.engine = None;
                self.engine_uri.clear();
                self.start_engine();
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
            // The desktop's queue comes from Spotify's Web API.
            EngineEvent::Upcoming(_) => return,
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
                // Spotify's state then adds album and artist pages and the context.
                self.next_poll = self.next_poll.min(Instant::now() + Duration::from_millis(1500));
            }
            EngineEvent::Playing { uri, position_ms } => {
                self.pb.is_playing = true;
                self.pb.progress_ms = position_ms as u64;
                self.pb.progress_at = Some(Instant::now());
                self.engine_uri = uri;
            }
            EngineEvent::Paused { position_ms, .. } => {
                // The song waits, paused, while the DJ talks: that's still playing.
                self.pb.is_playing = self.dj_voice.as_ref().is_some_and(|v| !v.is_paused()) || self.dj_waiting;
                self.pb.progress_ms = position_ms as u64;
                self.pb.progress_at = Some(Instant::now());
            }
            EngineEvent::Position { position_ms } => {
                self.pb.progress_ms = position_ms as u64;
                self.pb.progress_at = Some(Instant::now());
                self.media_resync = true;
            }
            // The next song announces itself; a stop may mean another device took over.
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

    /// A song's details already at hand: the row just clicked, or the queue.
    fn known_item(&mut self, uri: &str) -> Option<Value> {
        if let Some(h) = self.play_hint.take()
            && s(&h, "uri") == uri
        {
            return Some(h);
        }
        self.queue.as_ref()?["queue"].as_array()?.iter().find(|t| s(t, "uri") == uri).cloned()
    }

    /// The bar shows `item` as the song playing here, from the start.
    fn show_track(&mut self, item: Value) {
        self.pb.item = Some(Arc::new(item));
        self.pb.progress_ms = 0;
        self.pb.progress_at = Some(Instant::now());
        self.next_queue_poll = Instant::now();
        self.on_track_changed();
    }

    fn on_track_changed(&mut self) {
        let Some(item) = self.pb.item.clone() else { return };
        let uri = s(&item, "uri").to_string();
        if self.settings.track_notifications && self.notified_uri != uri && !self.notified_uri.is_empty() {
            notify(s(&item, "name"), &model::artists_line(&item));
        }
        self.notified_uri = uri;
        self.invalidate("/me/player/recently-played");
    }

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

    fn update_media(&mut self) {
        if self.media.is_none() {
            return;
        }
        // Runs every frame: only compare until something changed.
        let local = self.local.as_ref().and_then(|l| l.current().map(|t| (t, l)));
        let (same, playing) = match (local, self.pb.item.as_ref()) {
            (Some((t, l)), _) => (t.path.to_string_lossy() == self.media_uri.as_str(), l.is_playing()),
            (None, Some(i)) => (s(i, "uri") == self.media_uri, self.pb.is_playing),
            (None, None) => return,
        };
        if same && playing == self.media_playing && !self.media_resync {
            return;
        }
        let (uri, title, artist, album, cover, dur, pos) = if let Some((t, l)) = self.local.as_ref().and_then(|l| l.current().map(|t| (t, l))) {
            (t.path.to_string_lossy().into_owned(), t.title.clone(), t.artist.clone(), t.album.clone(), None, t.duration_ms, l.position_ms())
        } else if let Some(i) = self.pb.item.as_ref() {
            let album = i.get("album").map(|a| s(a, "name").to_string()).unwrap_or_default();
            (s(i, "uri").to_string(), s(i, "name").to_string(), model::artists_line(i), album, model::image(i, 300), model::u(i, "duration_ms"), self.pb.position())
        } else {
            return;
        };
        if !same {
            let fresh_track = self.media_cover.as_ref().map(|p| !p.to_string_lossy().contains(&cover_key(&uri))).unwrap_or(true);
            if fresh_track {
                self.media_cover = None;
                if let Some(url) = cover {
                    self.fetch_artwork(uri.clone(), url);
                }
            }
            self.media_uri = uri;
            // souvlaki aborts the process if macOS can't load the cover URL, so it
            // only ever gets a local file that Glass has already decoded once.
            let cover_url = self.media_cover.as_ref().map(|p| format!("file://{}", p.display()));
            if let Some(mc) = self.media.as_mut() {
                let _ = mc.set_metadata(souvlaki::MediaMetadata {
                    title: Some(&title),
                    artist: Some(&artist),
                    album: Some(&album),
                    cover_url: cover_url.as_deref(),
                    duration: Some(Duration::from_millis(dur)),
                });
            }
            self.media_playing = !playing;
        }
        if playing != self.media_playing || self.media_resync {
            self.media_playing = playing;
            self.media_resync = false;
            let progress = Some(souvlaki::MediaPosition(Duration::from_millis(pos)));
            if let Some(mc) = self.media.as_mut() {
                let _ = mc.set_playback(if playing { souvlaki::MediaPlayback::Playing { progress } } else { souvlaki::MediaPlayback::Paused { progress } });
            }
        }
    }

    fn fetch_artwork(&self, uri: String, url: String) {
        if !url.starts_with("https://") {
            return;
        }
        let (tx, ctx, http) = (self.tx.clone(), self.ctx.clone(), self.save_http.clone());
        self.rt.spawn(async move {
            let Ok(resp) = http.get(&url).timeout(Duration::from_secs(20)).send().await else { return };
            let Ok(bytes) = resp.bytes().await else { return };
            if image::load_from_memory(&bytes).is_err() {
                return;
            }
            let dir = crate::config::cache_dir().join("artwork");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("{}.jpg", cover_key(&uri)));
            if std::fs::write(&path, &bytes).is_ok() {
                let _ = tx.send(Msg::Artwork(uri, path));
                ctx.request_repaint();
            }
        });
    }

    /// Starts Glass at login: a LaunchAgent on macOS, an XDG autostart entry on Linux.
    pub fn apply_autostart(&self) {
        if cfg!(target_os = "macos") { self.autostart_macos() } else { self.autostart_linux() }
    }

    fn autostart_linux(&self) {
        let Some(dirs) = directories::BaseDirs::new() else { return };
        let path = dirs.config_dir().join("autostart/glass.desktop");
        if self.settings.autostart == Autostart::Off {
            let _ = std::fs::remove_file(path);
            return;
        }
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "glass".into());
        // Quoted as the desktop entry spec asks, so a path with spaces works.
        let exe: String = exe.chars().flat_map(|c| if matches!(c, '"' | '`' | '$' | '\\') { vec!['\\', c] } else { vec![c] }).collect();
        let min = if self.settings.autostart == Autostart::Minimized { " --minimized" } else { "" };
        let entry = format!("[Desktop Entry]\nType=Application\nName=Glass\nExec=\"{exe}\"{min}\nIcon=glass\nX-GNOME-Autostart-enabled=true\n");
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(path, entry);
    }

    fn autostart_macos(&self) {
        let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) else { return };
        let path = home.join("Library/LaunchAgents/app.glass.player.plist");
        if self.settings.autostart == Autostart::Off {
            let _ = std::fs::remove_file(path);
            return;
        }
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
        // The path is XML text in the plist (a folder named "R&B" would break it otherwise).
        let exe = exe.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let min = if self.settings.autostart == Autostart::Minimized { "<string>--minimized</string>" } else { "" };
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict><key>Label</key><string>app.glass.player</string>\
<key>ProgramArguments</key><array><string>{exe}</string>{min}</array>\
<key>RunAtLoad</key><true/></dict></plist>\n"
        );
        let _ = std::fs::create_dir_all(home.join("Library/LaunchAgents"));
        let _ = std::fs::write(path, plist);
    }
}

// ---------------------------------------------------------------- eframe glue

pub struct GlassApp {
    pub app: App,
    pub frames: u64,
}

impl eframe::App for GlassApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.tick();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::root(&mut self.app, ui);
        self.app.apply_actions();
        #[cfg(not(target_os = "macos"))]
        self.app.remember_window_size();
        self.frames += 1;
        crate::demo::screenshot_tick(ui.ctx(), self.frames);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.save_now();
        // A timing nudge made just before quitting.
        self.app.lyrics_choices.flush();
        self.app.install_update_on_quit();
    }
}

/// Sends its message when dropped, unless disarmed: a background task's way of
/// answering even if it dies before it can.
struct Answer {
    send: Option<(std::sync::mpsc::Sender<Msg>, egui::Context, Msg)>,
}

impl Answer {
    fn new(tx: std::sync::mpsc::Sender<Msg>, ctx: egui::Context, msg: Msg) -> Self {
        Self { send: Some((tx, ctx, msg)) }
    }

    fn disarm(mut self) {
        self.send = None;
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        if let Some((tx, ctx, msg)) = self.send.take() {
            let _ = tx.send(msg);
            ctx.request_repaint();
        }
    }
}

/// Data kept however long it goes unused, so Your Library looks the same when the
/// window comes back after a while hidden: the account, the lists it's made from
/// (which the command bar searches too), the counts beside Liked Songs and Your
/// Episodes, and the listening history its Recents order follows. The rest, Liked
/// Songs' whole list included, goes after `IDLE` unused.
fn kept_for_good(key: &str) -> bool {
    let path = key.split_once(':').filter(|(all, _)| all.starts_with("ALL")).map_or(key, |(_, p)| p);
    path == "/me" || path.ends_with("limit=1") || ["/me/playlists", "/me/albums", "/me/following", "/me/shows", "/me/audiobooks", "/me/player/recently-played"].iter().any(|p| path.starts_with(p))
}

/// A desktop notification. The text goes in as arguments, never as script, since
/// song and show names come from whoever published them.
fn notify(title: &str, body: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("osascript");
        c.args(["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", title, body]);
        c
    } else {
        let mut c = std::process::Command::new("notify-send");
        c.args(["--app-name=Glass", "--", title, body]);
        c
    };
    // Waited for on its own thread, so no finished process is left behind.
    std::thread::spawn(move || {
        let _ = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    });
}

fn cover_key(uri: &str) -> String {
    uri.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::kept_for_good;

    #[test]
    fn what_your_library_shows_is_kept_while_unused() {
        for key in ["/me", "ALL2000:/me/playlists?limit=50", "ALL500:/me/audiobooks?limit=50", "/me/tracks?limit=1", "/me/player/recently-played?limit=50"] {
            assert!(kept_for_good(key), "{key}");
        }
        for key in ["ALL10000:/me/tracks?limit=50", "/me/tracks?limit=50", "/me/top/artists?limit=12&time_range=short_term", "/artists/x", "ALL2000:/playlists/x/tracks?limit=100"] {
            assert!(!kept_for_good(key), "{key}");
        }
    }
}
