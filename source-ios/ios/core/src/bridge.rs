//! The boundary the Swift app talks to (UniFFI).
//!
//! - Swift calls `GlassCore` methods. Each turns into an `Action` (or a view
//!   change) sent to the core's actor thread, which owns all state; the calls
//!   return at once and never block on the network.
//! - The actor answers with `CoreEvent`s through `CoreListener`, from its own
//!   thread. Swift applies them to its single store on the main actor.
//! - Panics never cross: UniFFI catches them at every exported call, and the
//!   actor and runtime run on their own threads.
//! - Playback position isn't streamed: `PlaybackModel` carries a position and
//!   when it was taken, and Swift moves it on by itself while playing.

use crate::app::{Action, MobileApp, Msg, Tx};
use crate::lyrics_app::LyricsAction;
use crate::publish::SavedFlag;
use crate::types::*;
use std::sync::{Arc, Mutex};

/// Where the app keeps things, as iOS gives them (all inside the app's container).
#[derive(uniffi::Record, Clone, Debug)]
pub struct CoreConfig {
    /// Library/Application Support/Glass: settings, local playlists, lyrics picks, device IDs.
    pub data_dir: String,
    /// Library/Caches/Glass: lyrics answers, artwork (iOS may purge it).
    pub cache_dir: String,
    /// Documents: music the user brings in (visible in the Files app).
    pub documents_dir: String,
    /// Sample data, no network and no accounts (like the desktop's GLASS_DEMO).
    pub demo: bool,
    /// The device's name for Spotify Connect when the setting is still the default.
    pub device_name: String,
    /// The user's first preferred language ("en-US"): "system" for translations.
    pub language: String,
    /// UIAccessibility's Reduce Motion at launch ([motion] reduce = "system" follows it).
    pub reduce_motion: bool,
}

/// Events from the core, in the order they happened.
#[derive(uniffi::Enum, Clone, Debug)]
pub enum CoreEvent {
    Account { account: AccountModel },
    Playback { playback: PlaybackModel },
    Queue { queue: QueueModel },
    Devices { devices: DevicesModel },
    Library { library: LibraryModel },
    /// A page Swift subscribed to (by `key`, see `route_key`) was built again.
    Page { key: String, page: PageModel },
    /// Saved/liked/followed flags that changed.
    Saved { changes: Vec<SavedFlag> },
    Toast { text: String },
    /// Open this page (a playlist just created, say).
    Navigate { route: Route },
    /// This page went away (deleted): leave it if it's open.
    Left { route: Route },
    /// Open Now Playing (Settings: "Open the now-playing screen when playback starts").
    ShowNowPlaying,
    /// The assistant: conversation, previews, approvals, models, account.
    Assistant { assistant: crate::assistant_app::AssistantModel },
    /// The ChatGPT sign-in page, for the system's web sign-in sheet.
    OpenAssistantSignIn { request: SignInRequest },
    /// Sharing with Glass on your other devices: who's here, pairing, transfers.
    Share { share: crate::share_app::ShareModel },
    /// Another device's Local Files, while they're on screen (see `share_browse`).
    DeviceLibrary { library: crate::share_app::DeviceLibraryModel },
    /// Add to Apple Music: the pairing file, a run's progress and its result.
    AppleMusic { apple_music: crate::apple_music::AppleMusicModel },
    /// Songs being saved as files, and collections being saved.
    Saves { saves: crate::save_app::SavesModel },
    /// Local Files: the library, its sources and Glass's local playlists.
    Local { local: crate::local_app::LocalModel },
    /// The tag editor's save finished (`error`: why not; the file is untouched then).
    TagsSaved { path: String, error: Option<String> },
    /// settings.json, config.toml and the live notes beside them.
    Settings { settings: crate::settings_app::SettingsModel, config: crate::settings_app::ConfigModel, status: crate::settings_app::StatusModel },
    /// The playing album's colors (none: the theme's own; Settings ▸ Display).
    AlbumColors { colors: Option<crate::colors_app::AlbumColorsModel> },
    /// theme.toml changed (or was edited), or its error/warnings did.
    Theme { theme: crate::theme_app::ThemeModel },
    /// The lyrics for the playing song changed (looked up only while on screen).
    Lyrics { lyrics: crate::lyrics_app::LyricsModel },
    /// Present Spotify's sign-in page in ASWebAuthenticationSession.
    OpenSignIn { request: SignInRequest },
    SignedOut,
}

/// Implemented in Swift. Called from the core's thread; it must not block.
#[uniffi::export(with_foreign)]
pub trait CoreListener: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

#[derive(Clone, Debug)]
pub enum ConfigValue {
    Bool(bool),
    Text(String),
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{message}")]
    Failed { message: String },
}

#[derive(uniffi::Object)]
pub struct GlassCore {
    rt: tokio::runtime::Runtime,
    tx: Tx,
    actor: Mutex<Option<std::thread::JoinHandle<()>>>,
    demo: bool,
    viz: Mutex<crate::viz::Analyzer>,
}

#[uniffi::export]
impl GlassCore {
    /// Starts the core: its runtime, then the actor (which loads settings and,
    /// with a saved session, signs in). `keychain`: the app's Keychain (none in the demo).
    #[uniffi::constructor]
    pub fn new(config: CoreConfig, listener: Arc<dyn CoreListener>, keychain: Option<Arc<dyn crate::platform::Keychain>>, translator: Option<Arc<dyn crate::platform::Translator>>) -> Arc<Self> {
        crate::paths::install(&config);
        crate::window_fx::set_system_reduce_motion(config.reduce_motion);
        crate::platform::install_language(&config.language);
        crate::platform::install_translator(translator);
        crate::platform::install_keychain(if config.demo { None } else { keychain });
        let _ = env_logger_init();
        crate::viz::install();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("glass-core").enable_all().build().expect("tokio runtime");
        let (sender, rx) = std::sync::mpsc::channel();
        let tx = Tx(sender);
        let (handle, tx2, demo) = (rt.handle().clone(), tx.clone(), config.demo);
        let device_name = config.device_name.clone();
        let actor = std::thread::Builder::new()
            .name("glass-actor".into())
            .spawn(move || {
                let mut app = MobileApp::new(handle, tx2, listener, demo);
                // "Glass" is the desktop's default; on a phone, its own name reads better in pickers.
                if app.settings.device_name == "Glass" && !device_name.is_empty() {
                    app.settings.device_name = device_name;
                }
                app.run(rx);
            })
            .expect("actor thread");
        Arc::new(Self { rt, tx, actor: Mutex::new(Some(actor)), demo: config.demo, viz: Default::default() })
    }

    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    pub fn is_demo(&self) -> bool {
        self.demo
    }

    /// Saves and stops the core (the app is being torn down).
    pub fn shutdown(&self) {
        self.tx.send(Msg::Shutdown);
        if let Some(h) = self.actor.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    // ---------------------------------------------------------- views

    /// A page is on screen: build it now and whenever what it shows changes.
    pub fn subscribe(&self, route: Route) {
        self.tx.send(Msg::Subscribe(route));
    }

    pub fn unsubscribe(&self, route: Route) {
        self.tx.send(Msg::Unsubscribe(route));
    }

    /// Which live panels are visible (they're polled only while shown).
    pub fn set_panels(&self, queue: bool, devices: bool, now_playing: bool) {
        self.tx.send(Msg::Panels { queue, devices, now_playing });
    }

    pub fn set_foreground(&self, foreground: bool) {
        self.tx.send(Msg::Foreground(foreground));
    }

    /// Ask whether these are saved (hearts, Follow, Save): answered with `Saved` events.
    pub fn check_saved(&self, uris: Vec<String>) {
        self.tx.send(Msg::WantSaved(uris));
    }

    /// Mark a page's data stale (pull to refresh, [try again]).
    pub fn refresh(&self, key: String) {
        self.act(Action::Invalidate(key));
    }

    // ---------------------------------------------------------- account

    pub fn sign_in(&self) {
        self.act(Action::SignIn);
    }

    pub fn cancel_sign_in(&self) {
        self.act(Action::CancelSignIn);
    }

    pub fn log_out(&self) {
        self.act(Action::LogOut);
    }

    // ---------------------------------------------------------- playback

    pub fn play_context(&self, context: String, track: Option<String>) {
        self.act(Action::PlayContext { context, track });
    }

    pub fn play_uris(&self, uris: Vec<String>, index: u32) {
        self.act(Action::PlayUris { uris, index: index as usize });
    }

    pub fn play_local(&self, paths: Vec<String>, start: u32) {
        self.act(Action::PlayLocalPaths { paths, start: start as usize });
    }

    pub fn radio(&self, uri: String) {
        self.act(Action::Radio(uri));
    }

    pub fn toggle_play(&self) {
        self.act(Action::TogglePlay);
    }

    /// From the system's media controls: play only if paused.
    pub fn play(&self) {
        self.act(Action::Play);
    }

    /// From the system's media controls: pause only if playing.
    pub fn pause(&self) {
        self.act(Action::Pause);
    }

    pub fn next(&self) {
        self.act(Action::Next);
    }

    pub fn previous(&self) {
        self.act(Action::Prev);
    }

    pub fn seek(&self, position_ms: u64) {
        self.act(Action::Seek(position_ms));
    }

    pub fn seek_by(&self, delta_ms: i64) {
        self.act(Action::SeekBy(delta_ms));
    }

    /// `commit`: the slider was let go (remote devices get one request then, not one per move).
    pub fn set_volume(&self, volume: f32, commit: bool) {
        self.act(Action::Volume(volume));
        if commit {
            self.act(Action::VolumeCommit);
        }
    }

    pub fn toggle_mute(&self) {
        self.act(Action::ToggleMute);
    }

    pub fn set_shuffle(&self, on: bool) {
        self.act(Action::SetShuffle(on));
    }

    pub fn cycle_repeat(&self) {
        self.act(Action::CycleRepeat);
    }

    pub fn queue(&self, uris: Vec<String>) {
        self.act(Action::Queue(uris));
    }

    pub fn queue_collection(&self, uri: String) {
        self.act(Action::QueueCollection(uri));
    }

    pub fn clear_queue(&self) {
        self.act(Action::ClearQueue);
    }

    /// Moves a song in Now Playing's queue (indices from `QueueModel::positions`).
    pub fn move_in_queue(&self, uri: String, from: u32, to: u32) {
        self.act(Action::MoveInQueue { uri, from: from as usize, to: to as usize });
    }

    /// Local Files songs into Local Files' own queue (while it plays).
    pub fn queue_local(&self, paths: Vec<String>) {
        self.act(Action::QueueLocal(paths));
    }

    pub fn transfer(&self, device_id: String) {
        self.act(Action::Transfer(device_id));
    }

    pub fn restart_engine(&self) {
        self.act(Action::RestartEngine);
    }

    // ---------------------------------------------------------- library

    pub fn save(&self, uris: Vec<String>, save: bool) {
        self.act(Action::Save { uris, save });
    }

    pub fn add_to_playlist(&self, playlist: String, uris: Vec<String>) {
        self.act(Action::AddToPlaylist { playlist, uris });
    }

    pub fn remove_from_playlist(&self, playlist: String, uris: Vec<String>) {
        self.act(Action::RemoveFromPlaylist { playlist, uris });
    }

    pub fn move_playlist_item(&self, playlist: String, from: u32, to: u32) {
        self.act(Action::MovePlaylistItem { playlist, from: from as usize, to: to as usize });
    }

    pub fn create_playlist(&self, name: String, uris: Vec<String>) {
        self.act(Action::CreatePlaylist { name, uris });
    }

    pub fn update_playlist(&self, id: String, name: String, description: String, public: bool, collaborative: bool) {
        self.act(Action::UpdatePlaylist { id, name, description, public, collaborative });
    }

    pub fn delete_playlist(&self, id: String) {
        self.act(Action::DeletePlaylist(id));
    }

    pub fn pin(&self, uri: String, pinned: bool) {
        self.act(if pinned { Action::Pin(uri) } else { Action::Unpin(uri) });
    }

    pub fn create_folder(&self, name: String, add: Option<String>) {
        self.act(Action::CreateFolder { name, add });
    }

    pub fn rename_folder(&self, id: String, name: String) {
        self.act(Action::RenameFolder { id, name });
    }

    pub fn delete_folder(&self, id: String) {
        self.act(Action::DeleteFolder(id));
    }

    pub fn move_to_folder(&self, playlist: String, folder: Option<String>) {
        self.act(Action::MoveToFolder { playlist, folder });
    }

    pub fn download(&self, uri: String, name: String) {
        self.act(Action::Download { uri, name });
    }

    pub fn remove_download(&self, uri: String) {
        self.act(Action::RemoveDownload(uri));
    }

    pub fn remove_all_downloads(&self) {
        self.act(Action::RemoveAllDownloads);
    }

    pub fn clear_audio_cache(&self) {
        self.act(Action::ClearAudioCache);
    }

    pub fn set_library_sort(&self, sort: LibrarySort) {
        self.act(Action::SetLibrarySort(match sort {
            LibrarySort::Recents => crate::config::LibrarySort::Recents,
            LibrarySort::RecentlyAdded => crate::config::LibrarySort::RecentlyAdded,
            LibrarySort::Alphabetical => crate::config::LibrarySort::Alphabetical,
            LibrarySort::Creator => crate::config::LibrarySort::Creator,
        }));
    }

    pub fn set_library_view(&self, view: LibraryView) {
        self.act(Action::SetLibraryView(match view {
            LibraryView::List => crate::config::LibraryView::List,
            LibraryView::Compact => crate::config::LibraryView::Compact,
            LibraryView::Grid => crate::config::LibraryView::Grid,
        }));
    }

    pub fn remember_search(&self, query: String) {
        self.act(Action::RememberSearch(query));
    }

    pub fn clear_recent_searches(&self) {
        self.act(Action::ClearRecentSearches);
    }

    // ---------------------------------------------------------- local playlists

    pub fn play_local_playlist(&self, id: String, index: u32) {
        self.act(Action::PlayLocalPlaylist { id, index: index as usize });
    }

    pub fn create_local_playlist(&self, name: String, paths: Vec<String>) {
        self.act(Action::CreateLocalPlaylist { name, paths });
    }

    pub fn add_to_local_playlist(&self, id: String, paths: Vec<String>) {
        self.act(Action::AddToLocalPlaylist { id, paths });
    }

    pub fn remove_from_local_playlist(&self, id: String, index: u32) {
        self.act(Action::RemoveFromLocalPlaylist { id, index: index as usize });
    }

    pub fn delete_local_playlist(&self, id: String) {
        self.act(Action::DeleteLocalPlaylist(id));
    }

    pub fn undo_playlist_change(&self) {
        self.act(Action::UndoPlaylistChange);
    }

    pub fn rescan_local(&self) {
        self.act(Action::RescanLocal);
    }

    /// Files were put in a folder Glass reads (an import into Documents), or a folder
    /// from Files can be read now (`dirs`): it joins Local Files and is read again.
    pub fn files_added(&self, dirs: Vec<String>) {
        self.act(Action::FilesAdded(dirs));
    }

    /// Adds local songs to one of your Spotify playlists, as Spotify's local-file
    /// entries (title, artist, album, length), as Spotify's apps do.
    pub fn add_local_to_playlist(&self, playlist: String, paths: Vec<String>) {
        self.act(Action::AddLocalToPlaylist { playlist, paths });
    }

    /// Share ▸ Download as M4A/MP3: finds the song on JioSaavn or YouTube Music and
    /// saves a tagged file in Glass's folder (not Spotify's audio).
    pub fn save_song(&self, uri: String) {
        self.act(Action::SaveSong(uri));
    }

    /// The same for a playlist, an album or an artist's top songs, into a folder of its own.
    pub fn save_collection(&self, uri: String, name: String) {
        self.act(Action::SaveCollection { uri, name });
    }

    /// Stops a collection's save after the songs already downloading.
    pub fn stop_save_collection(&self, uri: String) {
        self.act(Action::StopSaveSet(uri));
    }

    /// The Activity page: stored files and sizes, caches, folders, the request log.
    pub async fn activity(&self) -> Option<crate::activity_app::ActivityModel> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(Msg::Activity(tx));
        let task = self.rt.spawn(async move { rx.await.ok() });
        task.await.ok().flatten()
    }

    /// theme.toml or config.toml, for the share sheet: its path (the file is written
    /// with its defaults first if it isn't there yet).
    pub async fn export_file(&self, which: crate::theme_app::EditableFile) -> Result<String, CoreError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(Msg::ExportFile(which, tx));
        let task = self.rt.spawn(async move { rx.await.unwrap_or_else(|_| Err("Glass is shutting down".into())) });
        task.await.unwrap_or_else(|e| Err(e.to_string())).map_err(|message| CoreError::Failed { message })
    }

    /// Replaces theme.toml or config.toml with `text`, only if it parses. Answers what
    /// happened, in words.
    pub async fn import_file(&self, which: crate::theme_app::EditableFile, text: String) -> Result<String, CoreError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(Msg::ImportFile(which, text, tx));
        let task = self.rt.spawn(async move { rx.await.unwrap_or_else(|_| Err("Glass is shutting down".into())) });
        task.await.unwrap_or_else(|e| Err(e.to_string())).map_err(|message| CoreError::Failed { message })
    }

    pub fn clear_request_log(&self) {
        self.tx.send(Msg::ClearRequestLog);
    }

    // ---------------------------------------------------------------- assistant

    /// The page on top (the assistant's one-line context note names it) and whether
    /// the assistant's sheet or panel is on screen.
    pub fn set_screen(&self, route: Option<Route>, assistant: bool) {
        self.tx.send(Msg::Screen { route, assistant });
    }

    pub fn ai_send(&self, text: String) {
        self.act(Action::AiSend(text));
    }

    pub fn ai_cancel(&self) {
        self.act(Action::AiCancel);
    }

    pub fn ai_retry(&self) {
        self.act(Action::AiRetry);
    }

    pub fn ai_new_chat(&self) {
        self.act(Action::AiNewChat);
    }

    pub fn ai_sign_in(&self, reconsent: bool, new_account: bool) {
        self.act(Action::AiSignIn { reconsent, new_account });
    }

    pub fn ai_cancel_sign_in(&self) {
        self.act(Action::AiCancelSignIn);
    }

    pub fn ai_sign_out(&self) {
        self.act(Action::AiSignOut);
    }

    pub fn ai_select_account(&self, client_id: String) {
        self.act(Action::AiSelectAccount(client_id));
    }

    pub fn ai_approve(&self, id: String) {
        self.act(Action::AiApprove(id));
    }

    pub fn ai_decline(&self, id: String) {
        self.act(Action::AiDecline(id));
    }

    pub fn ai_save_draft(&self, draft: String, replace: Option<String>) {
        self.act(Action::AiSaveDraft { draft, replace });
    }

    pub fn ai_discard_draft(&self, draft: String) {
        self.act(Action::AiDiscardDraft(draft));
    }

    pub fn ai_edit_draft(&self, draft: String, edit: crate::assistant_app::DraftEdit) {
        self.act(Action::AiEditDraft { draft, edit });
    }

    pub fn ai_undo(&self) {
        self.act(Action::AiUndo);
    }

    pub fn ai_refresh_models(&self) {
        self.act(Action::AiRefreshModels);
    }

    pub fn ai_set_model(&self, slug: String) {
        self.act(Action::AiSetModel(slug));
    }

    pub fn ai_set_billing(&self, api_key: bool) {
        self.act(Action::AiSetBilling(api_key));
    }

    pub fn ai_enable(&self, on: bool) {
        self.act(Action::AiEnable(on));
    }

    pub fn ai_use_spotify(&self, on: bool) {
        self.act(Action::AiUseSpotify(on));
    }

    /// Straight into the Keychain from here: the key never travels through the
    /// core's messages (which can be logged) and is never shown or written to a file.
    pub fn ai_save_api_key(&self, key: String) {
        let key = key.trim().to_string();
        if key.is_empty() {
            return;
        }
        let store = crate::platform::secrets();
        let (tx, demo) = (self.tx.clone(), self.demo);
        self.rt.spawn_blocking(move || {
            let r = if demo { Err("The demo doesn't keep keys.".to_string()) } else { store.set(crate::ai::secrets::API_KEY_KEY, &key) };
            tx.send(Msg::AiKeySaved(r));
        });
    }

    pub fn ai_forget_api_key(&self) {
        self.act(Action::AiForgetApiKey);
    }

    pub fn ai_dismiss_welcome(&self) {
        self.act(Action::AiDismissWelcome);
    }

    pub fn ai_limit_acknowledged(&self) {
        self.act(Action::AiLimitAcknowledged);
    }

    /// Lists another device's Local Files (a `DeviceLibrary` event follows).
    pub fn share_browse(&self, peer: String) {
        self.act(Action::ShareBrowse(peer));
    }

    /// Gets the picked songs (`DeviceSongModel::id`s) from another device.
    pub fn share_get(&self, peer: String, ids: Vec<String>) {
        self.act(Action::ShareGet { peer, ids });
    }

    pub fn send_to_device(&self, peer: String, paths: Vec<String>) {
        self.act(Action::SendToDevice { peer, paths });
    }

    /// Add to Apple Music: a pairing file picked in Files, its bytes (the core keeps
    /// it in the Keychain; it's never written to a file).
    pub fn apple_music_import_pairing(&self, file: Vec<u8>) {
        self.act(Action::AppleMusicPairing(file));
    }

    pub fn apple_music_forget_pairing(&self) {
        self.act(Action::AppleMusicForgetPairing);
    }

    /// Copies these Local Files songs into the iPhone's music library; `playlist`:
    /// also into that playlist (made when there's none by that name).
    pub fn apple_music_add(&self, paths: Vec<String>, playlist: Option<String>) {
        self.act(Action::AppleMusicAdd { paths, playlist });
    }

    /// Stops a run while that leaves Apple Music unchanged.
    pub fn apple_music_stop(&self) {
        self.act(Action::AppleMusicStop);
    }

    /// Connects and reads the music folder, to check the setup.
    pub fn apple_music_check(&self) {
        self.act(Action::AppleMusicCheck);
    }

    /// Puts back the music library as it was before the last change Glass made.
    pub fn apple_music_restore(&self) {
        self.act(Action::AppleMusicRestore);
    }

    /// Songs that aren't on this device, from your other devices.
    pub fn get_from_devices(&self, uris: Vec<String>) {
        self.act(Action::GetFromDevices(uris));
    }

    /// A playlist's local songs that aren't on this device, from your other devices.
    pub fn get_missing_from_devices(&self, playlist: String) {
        self.act(Action::GetMissingFromDevices(playlist));
    }

    /// Audio files for `target` (picked, so already copied into Glass's folder, or dropped).
    pub fn add_files(&self, paths: Vec<String>, target: crate::types::FileTarget) {
        self.act(Action::AddFiles { paths, target });
    }

    /// Stop reading a folder (its files stay where they are).
    pub fn remove_source(&self, dir: String) {
        self.act(Action::RemoveSource(dir));
    }

    /// The tag editor's Save: written on a copy that then replaces the file.
    pub fn save_tags(&self, path: String, tags: crate::local_app::TagsModel, cover: crate::local_app::CoverChange) {
        self.act(Action::SaveTags { path, tags, cover });
    }

    // ---------------------------------------------------------- platform

    /// AVAudioSession's route changed. `gone`: the device that was playing went away
    /// (`.oldDeviceUnavailable`), so playback pauses instead of moving to the speaker.
    pub fn audio_route_changed(&self, gone: bool) {
        self.tx.send(Msg::RouteChanged { gone });
    }

    /// AVAudioSession was interrupted (a call, Siri) or the interruption ended (`resume`: iOS says to).
    pub fn audio_interrupted(&self, began: bool, resume: bool) {
        self.tx.send(Msg::Interrupted { began, resume });
    }

    pub fn set_accessibility(&self, reduce_motion: bool) {
        crate::window_fx::set_system_reduce_motion(reduce_motion);
    }

    pub fn toast(&self, text: String) {
        self.act(Action::Toast(text));
    }

    // ---------------------------------------------------------- lyrics

    /// Lyrics are on screen (they're looked up only then).
    pub fn lyrics_visible(&self, visible: bool) {
        self.lyrics(LyricsAction::Visible(visible));
    }

    /// "look again" / "try again": forgets the saved answer and your pick (timing stays).
    pub fn lyrics_retry(&self) {
        self.lyrics(LyricsAction::Retry);
    }

    /// Choose a candidate (from "several versions match" or the picker's results).
    pub fn lyrics_pick(&self, index: u32) {
        self.lyrics(LyricsAction::Pick { index: index as usize });
    }

    /// "none of these": no lyrics for this song.
    pub fn lyrics_none_fit(&self) {
        self.lyrics(LyricsAction::NoneFit);
    }

    /// "wrong lyrics?": the candidates, or a search for this song.
    pub fn lyrics_open_picker(&self) {
        self.lyrics(LyricsAction::OpenPicker);
    }

    pub fn lyrics_search(&self, query: String) {
        self.lyrics(LyricsAction::Search(query));
    }

    pub fn lyrics_close_picker(&self) {
        self.lyrics(LyricsAction::ClosePicker);
    }

    /// "auto", "lrclib" or "spotify" (config.toml `lyrics.provider`).
    pub fn set_lyrics_provider(&self, provider: String) {
        self.lyrics(LyricsAction::Provider(provider));
    }

    pub fn set_lyrics_fallback(&self, on: bool) {
        self.lyrics(LyricsAction::Fallback(on));
    }

    /// All songs (config.toml `lyrics.offset_ms`), ±10 s; positive shows lyrics earlier.
    pub fn set_lyrics_offset(&self, ms: i64) {
        self.lyrics(LyricsAction::Offset(ms));
    }

    /// This song only, kept for it; positive shows lyrics earlier.
    pub fn set_song_lyrics_offset(&self, ms: i64) {
        self.lyrics(LyricsAction::SongOffset(ms));
    }

    /// "off", "below" or "only".
    pub fn set_lyrics_translation(&self, mode: String) {
        self.lyrics(LyricsAction::Translation(mode));
    }

    /// "system" or a language code.
    pub fn set_translate_to(&self, language: String) {
        self.lyrics(LyricsAction::TranslateTo(language));
    }

    /// "auto", "apple" or "assistant".
    pub fn set_translator(&self, engine: String) {
        self.lyrics(LyricsAction::Translator(engine));
    }

    pub fn retry_translation(&self) {
        self.lyrics(LyricsAction::RetryTranslation);
    }

    pub fn clear_lyrics_cache(&self) {
        self.lyrics(LyricsAction::ClearCache);
    }

    // ---------------------------------------------------------- settings

    /// Replaces settings.json's values (the app sends the whole model back, edited).
    pub fn set_settings(&self, settings: crate::settings_app::SettingsModel) {
        self.tx.send(Msg::SetModel(Box::new(settings)));
    }

    /// Settings is open: measure the caches for its notes.
    pub fn settings_open(&self, open: bool) {
        self.tx.send(Msg::SettingsOpen(open));
    }

    /// config.toml's `assistant.enabled` and `assistant.spotify` (the rest of the assistant comes with it).
    pub fn set_config_bool(&self, key: String, value: bool) {
        self.act(Action::ConfigSet { key, value: ConfigValue::Bool(value) });
    }

    pub fn set_config_text(&self, key: String, value: String) {
        self.act(Action::ConfigSet { key, value: ConfigValue::Text(value) });
    }

    // ---------------------------------------------------------- theme

    /// Sets a theme.toml key ("theme.accent", "lyrics.size", "visualizer.bars"…) to a
    /// raw value ("#4F83A3", "0.8", "true"): validated, written in place.
    pub fn set_theme_value(&self, key: String, value: String) {
        self.act(Action::ThemeSet { key, value });
    }

    /// Shows `key = value` at once without writing theme.toml (while dragging);
    /// `set_theme_value` makes it stick. Ignored when the value isn't valid.
    pub fn preview_theme_value(&self, key: String, value: String) {
        self.act(Action::ThemePreview { key, value });
    }

    /// Back to the defaults for a table ("lyrics", "visualizer"), or the six [theme]
    /// values for "theme".
    pub fn reset_theme(&self, table: String) {
        self.act(Action::ThemeReset(table));
    }

    // ---------------------------------------------------------- visualizer

    /// The visualizer's bars (0…1) for audio played on this device, analysed from
    /// what reaches the output right now; empty when nothing has played here in
    /// the last moment. Called at display rate while the visualizer is on screen.
    pub fn visualizer_bands(&self, bars: u32, sensitivity: f32, smoothing: f32) -> Vec<f32> {
        let mut a = self.viz.lock().unwrap();
        a.update(bars.clamp(12, 128) as usize, sensitivity, smoothing);
        if a.live { a.bands.clone() } else { Vec::new() }
    }
}

/// The key a page's events carry (`CoreEvent::Page`), for a route.
#[uniffi::export]
pub fn route_key(route: Route) -> String {
    crate::views::route_key(&route)
}

/// The cover art embedded in a local file (JPEG or PNG bytes), read off the
/// main thread by the caller.
#[uniffi::export]
pub fn local_cover(path: String) -> Option<Vec<u8>> {
    crate::local::read_cover(std::path::Path::new(&path))
}

#[cfg(debug_assertions)]
#[uniffi::export]
impl GlassCore {
    /// Debug builds only: the read-only Spotify feasibility check (see probe.rs), run
    /// on the core's runtime (librespot's tasks need tokio's).
    pub async fn debug_probe_spotify(&self, credentials_path: String, with_engine: bool, play: bool) -> String {
        let task = self.rt.spawn(async move { crate::probe::spotify(&credentials_path, with_engine, play).await });
        task.await.unwrap_or_else(|e| format!("the probe stopped: {e}"))
    }

    /// Debug builds only: the assistant uses a local mock of OpenAI's API (UI tests).
    pub fn debug_assistant_mock(&self, base: String) {
        self.tx.send(Msg::DebugAiMock(base));
    }

    /// Debug builds only: reads query hashes from Spotify's web player (see probe.rs).
    pub async fn debug_probe_hashes(&self) -> String {
        let task = self.rt.spawn(async move { crate::probe::hashes().await });
        task.await.unwrap_or_else(|e| format!("the probe stopped: {e}"))
    }

    /// Debug builds only: announces and browses on Bonjour for `secs` seconds (see probe.rs).
    pub async fn debug_probe_bonjour(&self, secs: u64) -> String {
        let task = self.rt.spawn(async move { crate::probe::bonjour(secs).await });
        task.await.unwrap_or_else(|e| format!("the probe stopped: {e}"))
    }

    /// Debug builds only: saves one song through Share ▸ Download's pipeline (see probe.rs).
    pub async fn debug_probe_save(&self, mp3: bool) -> String {
        let task = self.rt.spawn(async move { crate::probe::save(mp3).await });
        task.await.unwrap_or_else(|e| format!("the probe stopped: {e}"))
    }
}

impl GlassCore {
    fn act(&self, a: Action) {
        self.tx.send(Msg::Act(a));
    }

    fn lyrics(&self, a: LyricsAction) {
        self.act(Action::Lyrics(a));
    }
}

fn env_logger_init() -> Result<(), log::SetLoggerError> {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Info && (m.target().starts_with("glass") || m.level() <= log::Level::Warn)
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info))
}
