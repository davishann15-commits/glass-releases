//! Add to Apple Music: copies Local Files songs into this iPhone's own music
//! library, the one the Music app shows. A stripped-down port of ByeTunes
//! (github.com/EduAlexxis/ByeTunes, MIT): its pairing, connection and library
//! code for just this, without its downloads, metadata lookups, ringtones,
//! backups or library tools.
//!
//! What it needs from the user, once: LocalDevVPN (an App Store VPN app that
//! routes 10.7.0.1 back to the phone itself) and a pairing file for this iPhone
//! made on a computer with idevice_pair (an RP pairing file from iOS 26.4 on). The
//! pairing file holds a key that unlocks the phone's device services, so it's kept
//! in the Keychain, never as a file.
//!
//! What a run does (device.rs, library.rs): connect; close the Music app; download
//! MediaLibrary.sqlitedb (keeping a copy of it as it was); add the songs to the copy;
//! upload the songs into /iTunes_Control/Music/F00 under random names, and their
//! covers; upload the new library beside the old one and swap them. The songs show
//! up in the Music app after the iPhone restarts (as with ByeTunes).

pub mod device;
pub mod library;
pub mod song;

use crate::app::{MobileApp, Msg, Tx};
use crate::bridge::CoreEvent;
use base64::Engine;
use device::{Device, Listing, Pairing};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The Keychain item with the pairing file (base64 of the file as imported).
const SECRET: &str = "apple-music-pairing";
const LIBRARY: &str = "/iTunes_Control/iTunes/MediaLibrary.sqlitedb";
const ARTWORK: &str = "/iTunes_Control/iTunes/Artwork/Originals";
/// Songs uploaded at once (one AFC connection each).
const UPLOADERS: usize = 3;
/// The job thread's stack (see `apple_music_start`); only what's touched is used.
const JOB_STACK: usize = 16 << 20;

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Debug)]
pub enum PairingKind {
    /// A classic pairing file (lockdown).
    Lockdown,
    /// An RP pairing file (RemotePairing).
    RemotePairing,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct AppleMusicResult {
    pub ok: bool,
    pub text: String,
    /// Songs left out, and why.
    pub details: Vec<String>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct AppleMusicModel {
    /// The pairing file imported, if any.
    pub pairing: Option<PairingKind>,
    /// This iPhone takes only RP pairing files (iOS 26.4 and later).
    pub needs_remote_pairing: bool,
    /// "26.4"
    pub ios: String,
    /// Adding songs (or checking the connection) right now.
    pub busy: bool,
    /// What it's doing: "Copying songs (3 of 12)…".
    pub step: String,
    /// 0–1 while songs are copied.
    pub progress: Option<f32>,
    /// It can still be stopped (until the library is replaced).
    pub can_stop: bool,
    /// The last run: what was added, or why not.
    pub result: Option<AppleMusicResult>,
    /// A copy of the library from before Glass's last change is kept.
    pub can_restore: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Run {
    Add,
    Check,
    Restore,
}

pub enum Update {
    Step { text: String, progress: Option<f32>, can_stop: bool },
    Done(AppleMusicResult),
}

#[derive(Default)]
pub struct AppleMusicState {
    /// Read from the Keychain once (None: not looked yet).
    pairing: Option<Option<PairingKind>>,
    stop: Option<Arc<AtomicBool>>,
    step: String,
    progress: Option<f32>,
    can_stop: bool,
    result: Option<AppleMusicResult>,
    published: Option<AppleMusicModel>,
}

fn kind(p: &Pairing) -> PairingKind {
    match p {
        Pairing::Lockdown(_) => PairingKind::Lockdown,
        Pairing::Remote(_) => PairingKind::RemotePairing,
    }
}

fn stored_pairing() -> Option<Pairing> {
    let text = crate::platform::secrets().get(SECRET).ok()??;
    let bytes = base64::engine::general_purpose::STANDARD.decode(text).ok()?;
    Pairing::parse(&bytes)
}

/// A run that failed, in plain words (what went wrong is logged). Glass's own
/// explanations are kept; the device's, the network's and SQLite's aren't shown.
fn plain(what: Run, e: &str) -> String {
    log::warn!("Add to Apple Music ({what:?}): {e}");
    let ours = ["Glass can't reach this iPhone", "This iPhone doesn't accept", "Stopped.", "Set up Add to Apple Music", "None of these songs"];
    if ours.iter().any(|p| e.starts_with(p)) {
        return e.to_string();
    }
    let l = e.to_lowercase();
    if ["tunnel", "pairing", "heartbeat", "file access", "stopped answering", "timed out", "timeout", "connect", "lockdown", "rsd"].iter().any(|w| l.contains(w)) {
        "Glass can't reach this iPhone. Open LocalDevVPN and connect it, then try again.".into()
    } else if l.contains("failed a check") {
        "Your Apple Music library looks damaged. Restart your iPhone and try again.".into()
    } else if l.contains("couldn't be put back") {
        "Glass couldn't finish, and couldn't put the old library back. Tap Undo last add.".into()
    } else if what == Run::Restore {
        "Couldn't undo. Try again.".into()
    } else {
        "Couldn't add the songs; nothing was changed. Try again.".into()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

impl MobileApp {
    fn apple_music_pairing_kind(&mut self) -> Option<PairingKind> {
        if self.apple_music.pairing.is_none() {
            self.apple_music.pairing = Some(stored_pairing().as_ref().map(kind));
        }
        self.apple_music.pairing.flatten()
    }

    /// A pairing file picked in Files: checked, then kept in the Keychain.
    pub fn apple_music_import_pairing(&mut self, bytes: Vec<u8>) {
        let Some(pairing) = Pairing::parse(&bytes) else {
            return self.toast("That isn't a pairing file.");
        };
        let (major, minor) = device::os_version();
        let rp = device::needs_remote_pairing(major, minor);
        match (&pairing, rp) {
            (Pairing::Lockdown(_), true) => return self.toast("This iPhone needs an RP pairing file. Save that kind in idevice_pair."),
            (Pairing::Remote(_), false) => return self.toast("This iPhone needs a classic pairing file, not an RP one. Save that kind in idevice_pair."),
            _ => {}
        }
        let text = base64::engine::general_purpose::STANDARD.encode(&bytes);
        match crate::platform::secrets().set(SECRET, &text) {
            Ok(()) => {
                self.apple_music.pairing = Some(Some(kind(&pairing)));
                self.apple_music.result = None;
                self.toast("Set up. You can add songs now.");
            }
            Err(e) => {
                log::warn!("the pairing file couldn't be saved: {e}");
                self.toast("Couldn't save the pairing file. Try again.")
            }
        }
    }

    pub fn apple_music_forget_pairing(&mut self) {
        let _ = crate::platform::secrets().delete(SECRET);
        self.apple_music.pairing = Some(None);
        self.apple_music.result = None;
    }

    pub fn apple_music_stop(&mut self) {
        if let Some(stop) = &self.apple_music.stop
            && self.apple_music.can_stop
        {
            stop.store(true, Ordering::Relaxed);
            self.apple_music.step = "Stopping…".into();
            self.apple_music.can_stop = false;
        }
    }

    /// Connects, reads the music folder and disconnects: is everything in place?
    pub fn apple_music_check(&mut self) {
        self.apple_music_start(Vec::new(), None, Run::Check);
    }

    pub fn apple_music_restore(&mut self) {
        if !backup_dir().join("MediaLibrary.sqlitedb").exists() {
            return self.toast("Nothing to undo");
        }
        self.apple_music_start(Vec::new(), None, Run::Restore);
    }

    pub fn apple_music_add(&mut self, paths: Vec<String>, playlist: Option<String>) {
        if paths.is_empty() {
            return;
        }
        self.apple_music_start(paths.into_iter().map(PathBuf::from).collect(), playlist.filter(|p| !p.trim().is_empty()), Run::Add);
    }

    fn apple_music_start(&mut self, paths: Vec<PathBuf>, playlist: Option<String>, what: Run) {
        if self.apple_music.stop.is_some() {
            return self.toast("Glass is already adding songs to Apple Music");
        }
        let pairing = if self.demo { None } else { stored_pairing() };
        if pairing.is_none() && !self.demo {
            self.apple_music.pairing = Some(None);
            self.apple_music.result = Some(AppleMusicResult { ok: false, text: "Set up Add to Apple Music first (below).".into(), details: Vec::new() });
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.apple_music.stop = Some(stop.clone());
        self.apple_music.result = None;
        self.apple_music.step = if what == Run::Add { "Getting ready…".into() } else { "Connecting to this iPhone…".into() };
        self.apple_music.progress = None;
        self.apple_music.can_stop = what != Run::Restore;
        let tx = self.tx.clone();
        let demo = self.demo;
        // Its own thread and runtime, with a big stack: unoptimized (Debug) builds of this
        // code and idevice's need about 1.6 MB of stack in the RemotePairing handshake
        // under `run`, and overflowed the core runtime's 2 MB worker stacks (a crash).
        // It also keeps a long copy off the two workers that play music.
        let job = move || {
            let say = Say(tx.clone());
            let result = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt.block_on(async move {
                    match (pairing, what) {
                        _ if demo => demo_run(&say, paths.len(), what).await,
                        (Some(pairing), Run::Check) => check_connection(&pairing, &say).await,
                        (Some(pairing), Run::Restore) => restore(&pairing, &say).await,
                        (Some(pairing), Run::Add) => run(Job { pairing, paths, playlist, stop }, say.clone()).await,
                        (None, _) => Err("Set up Add to Apple Music first (below).".into()),
                    }
                }),
                Err(e) => Err(format!("no runtime: {e}")),
            };
            let result = result.unwrap_or_else(|e| AppleMusicResult { ok: false, text: plain(what, &e), details: Vec::new() });
            tx.send(Msg::AppleMusic(Update::Done(result)));
        };
        if let Err(e) = std::thread::Builder::new().name("glass-apple-music".into()).stack_size(JOB_STACK).spawn(job) {
            self.apple_music.stop = None;
            self.apple_music.step.clear();
            log::warn!("Add to Apple Music didn't start: {e}");
            self.apple_music.result = Some(AppleMusicResult { ok: false, text: "That didn't work. Try again.".into(), details: Vec::new() });
        }
    }

    pub fn apple_music_update(&mut self, u: Update) {
        match u {
            Update::Step { text, progress, can_stop } => {
                // A stop asked for stays shown until the run ends.
                if self.apple_music.stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed)) {
                    return;
                }
                self.apple_music.step = text;
                self.apple_music.progress = progress;
                self.apple_music.can_stop = can_stop;
            }
            Update::Done(result) => {
                self.apple_music.stop = None;
                self.apple_music.step.clear();
                self.apple_music.progress = None;
                self.apple_music.can_stop = false;
                self.apple_music.result = Some(result);
            }
        }
    }

    pub fn publish_apple_music(&mut self, events: &mut Vec<CoreEvent>) {
        let (major, minor) = device::os_version();
        let m = AppleMusicModel {
            pairing: self.apple_music_pairing_kind(),
            needs_remote_pairing: device::needs_remote_pairing(major, minor),
            ios: format!("{major}.{minor}"),
            busy: self.apple_music.stop.is_some(),
            step: self.apple_music.step.clone(),
            progress: self.apple_music.progress,
            can_stop: self.apple_music.can_stop,
            result: self.apple_music.result.clone(),
            can_restore: backup_dir().join("MediaLibrary.sqlitedb").exists(),
        };
        if self.apple_music.published.as_ref() != Some(&m) {
            self.apple_music.published = Some(m.clone());
            events.push(CoreEvent::AppleMusic { apple_music: m });
        }
    }
}

/// Progress, sent to the actor.
#[derive(Clone)]
struct Say(Tx);

impl Say {
    fn step(&self, text: impl Into<String>, can_stop: bool) {
        self.0.send(Msg::AppleMusic(Update::Step { text: text.into(), progress: None, can_stop }));
    }
    fn progress(&self, text: impl Into<String>, progress: f32) {
        self.0.send(Msg::AppleMusic(Update::Step { text: text.into(), progress: Some(progress.clamp(0.0, 1.0)), can_stop: true }));
    }
}

struct Job {
    pairing: Pairing,
    paths: Vec<PathBuf>,
    playlist: Option<String>,
    stop: Arc<AtomicBool>,
}

impl Job {
    fn stopped(&self) -> Result<(), String> {
        if self.stop.load(Ordering::Relaxed) { Err("Stopped. Apple Music wasn't changed.".into()) } else { Ok(()) }
    }
}

/// RemotePairing ports Bonjour finds in two seconds (for when it isn't the usual one).
async fn remote_pairing_ports() -> Vec<u16> {
    let Ok((browser, mut rx)) = crate::bonjour::Bonjour::browse("_remotepairing._tcp") else { return Vec::new() };
    let mut ports = Vec::new();
    let deadline = tokio::time::sleep(Duration::from_secs(2));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            found = rx.recv() => match found {
                Some(crate::bonjour::Found::Service { port, .. }) => if !ports.contains(&port) { ports.push(port) },
                Some(_) => {}
                None => break,
            },
            _ = &mut deadline => break,
        }
    }
    drop(browser);
    ports
}

async fn connect(pairing: &Pairing) -> Result<Device, String> {
    match Device::connect(pairing, &[]).await {
        Ok(d) => Ok(d),
        Err(first) if matches!(pairing, Pairing::Remote(_)) => {
            let ports = remote_pairing_ports().await;
            if ports.is_empty() {
                return Err(first);
            }
            Device::connect(pairing, &ports).await
        }
        Err(e) => Err(e),
    }
}

async fn check_connection(pairing: &Pairing, say: &Say) -> Result<AppleMusicResult, String> {
    let mut dev = connect(pairing).await?;
    say.step("Checking…", false);
    let mut afc = dev.afc().await?;
    let songs = match device::list(&mut afc, "/iTunes_Control/Music").await? {
        Listing::Files(dirs) => dirs.len(),
        Listing::Missing => 0,
    };
    log::info!("Add to Apple Music: {songs} music folders");
    log::info!("Add to Apple Music: library there: {}", device::exists(&mut afc, LIBRARY).await);
    Ok(AppleMusicResult { ok: true, text: "Connected. Ready to add songs.".into(), details: Vec::new() })
}

/// The demo: the steps, without a device.
async fn demo_run(say: &Say, n: usize, what: Run) -> Result<AppleMusicResult, String> {
    say.step("Connecting to this iPhone…", what == Run::Add);
    tokio::time::sleep(Duration::from_millis(600)).await;
    match what {
        Run::Check => return Ok(AppleMusicResult { ok: true, text: "Connected. Ready to add songs.".into(), details: Vec::new() }),
        Run::Restore => return Err("The demo has no library to put back.".into()),
        Run::Add => {}
    }
    for i in 0..n {
        say.progress(format!("Copying songs ({} of {n})…", i + 1), i as f32 / n as f32);
        tokio::time::sleep(Duration::from_millis(900)).await;
    }
    say.step("Finishing up…", false);
    tokio::time::sleep(Duration::from_millis(400)).await;
    Ok(AppleMusicResult { ok: true, text: format!("Added {}. Restart your iPhone to see them in the Music app.", plural(n, "song", "songs")), details: Vec::new() })
}

/// Where the library is copied to and changed (the app's caches; emptied each run).
fn work_dir() -> PathBuf {
    crate::config::cache_dir().join("apple-music")
}

/// The library as it was before the last run (Application Support, kept).
pub fn backup_dir() -> PathBuf {
    crate::config::config_dir().join("apple-music-backup")
}

fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", db.display()))
}

async fn run(job: Job, say: Say) -> Result<AppleMusicResult, String> {
    let (major, minor) = device::os_version();
    let version = library::Version { major, minor };

    // The songs, read here first: nothing is changed when none can go in.
    let paths = job.paths.clone();
    let read = tokio::task::spawn_blocking(move || paths.iter().map(|p| song::read(p)).collect::<Vec<_>>()).await.map_err(|e| e.to_string())?;
    let mut songs = Vec::new();
    let mut left_out = Vec::new();
    for r in read {
        match r {
            Ok(s) => songs.push(s),
            Err(why) => left_out.push(why),
        }
    }
    if songs.is_empty() {
        return Ok(AppleMusicResult { ok: false, text: "None of these songs could be added.".into(), details: left_out });
    }
    job.stopped()?;

    say.step("Connecting to this iPhone…", true);
    let mut dev = connect(&job.pairing).await?;
    job.stopped()?;
    say.step("Closing the Music app…", true);
    dev.quit_music().await;

    // The library as it is, copied here (and kept as it was).
    say.step("Reading your Apple Music library…", true);
    let work = work_dir();
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let db_path = work.join("MediaLibrary.sqlitedb");
    let mut afc = dev.afc().await?;
    let had_library = device::download(&mut afc, LIBRARY, &db_path).await?;
    for suffix in ["-wal", "-shm"] {
        device::download(&mut afc, &format!("{LIBRARY}{suffix}"), &sidecar(&db_path, suffix)).await?;
    }
    // Off the runtime's threads: a big library takes a while to copy and check.
    let file = db_path.clone();
    let (usable, music) = tokio::task::spawn_blocking(move || -> Result<(bool, String), String> {
        let usable = had_library && library::usable(&file);
        if !usable {
            // Nothing to go back to: an older copy mustn't be put back over this run.
            let _ = std::fs::remove_dir_all(backup_dir());
            return Ok((false, "/iTunes_Control/Music/F00".to_string()));
        }
        keep_backup(&file);
        Ok((true, library::music_folder(&library::open_copy(&file)?)))
    })
    .await
    .map_err(|e| e.to_string())??;
    job.stopped()?;

    // What the music folder has.
    let on_device: HashSet<String> = match device::list(&mut afc, &music).await? {
        Listing::Files(names) => names.into_iter().collect(),
        Listing::Missing => HashSet::new(),
    };

    // The songs added to the copy.
    say.step("Adding the songs to your library…", true);
    let (db_file, playlist, songs_for_db) = (db_path.clone(), job.playlist.clone(), songs.clone());
    let outcome = tokio::task::spawn_blocking(move || -> Result<library::Outcome, String> {
        let db = if usable {
            let db = library::open_copy(&db_file)?;
            let _ = db.execute_batch(
                "INSERT OR IGNORE INTO base_location (base_location_id, path) VALUES (3840, 'iTunes_Control/Music/F00');
                 INSERT OR IGNORE INTO base_location (base_location_id, path) VALUES (3900, 'iTunes_Control/Ringtones');",
            );
            let ghosts = library::remove_ghosts(&db, &on_device).map_err(|e| e.to_string())?;
            if ghosts > 0 {
                log::info!("Add to Apple Music: removed {ghosts} entries whose files are gone");
            }
            db
        } else {
            library::create(&db_file, version)?
        };
        let mut taken = on_device;
        taken.extend(library::locations(&db));
        let outcome = library::add_songs(&db, &songs_for_db, version, &taken)?;
        if let Some(name) = playlist {
            let mut pids: Vec<(usize, i64)> = outcome.added.iter().map(|a| (a.song, a.item_pid)).chain(outcome.already.iter().copied()).collect();
            pids.sort_by_key(|p| p.0);
            library::add_to_playlist(&db, &name, &pids.iter().map(|p| p.1).collect::<Vec<_>>())?;
        }
        library::repair_order(&db)?;
        Ok(outcome)
    })
    .await
    .map_err(|e| e.to_string())??;
    for (i, _) in &outcome.already {
        left_out.push(format!("{} — added before", songs[*i].title));
    }
    if outcome.added.is_empty() && job.playlist.is_none() {
        return Ok(AppleMusicResult { ok: true, text: "Glass added these songs before; nothing new to add.".into(), details: left_out });
    }
    job.stopped()?;

    // The songs and their covers, then the library.
    device::make_dirs(&mut afc, &music).await;
    let uploaded = Arc::new(Mutex::new(Vec::<String>::new()));
    let copied = upload_songs(&mut dev, &songs, &outcome.added, &music, &job.stop, &say, &uploaded).await;
    if let Err(e) = copied.and_then(|_| job.stopped()) {
        say.step("Cancelling…", false);
        let mut afc = dev.afc().await.unwrap_or(afc);
        remove_all(&mut afc, &uploaded).await;
        return Err(e);
    }
    say.step("Adding artwork…", true);
    for a in &outcome.added {
        let Some(rel) = &a.artwork else { continue };
        let path = songs[a.song].path.clone();
        let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || song::cover(&path)).await else { continue };
        let dir = format!("{ARTWORK}/{}", rel.split('/').next().unwrap_or("00"));
        device::make_dirs(&mut afc, &dir).await;
        if let Err(e) = device::put(&mut afc, &bytes, &format!("{ARTWORK}/{rel}")).await {
            log::warn!("Add to Apple Music: a cover wasn't copied: {e}");
        }
    }
    if let Err(e) = job.stopped() {
        say.step("Cancelling…", false);
        remove_all(&mut afc, &uploaded).await;
        return Err(e);
    }

    say.step("Finishing up…", false);
    let replaced = replace_library(&mut afc, &db_path).await;
    if let Err(e) = replaced {
        remove_all(&mut afc, &uploaded).await;
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&work);

    let n = outcome.added.len();
    let mut text = match (n, &job.playlist) {
        (0, Some(name)) => format!("Playlist “{name}” updated."),
        (0, None) => "Nothing new to add.".to_string(),
        (n, Some(name)) => format!("Added {} (also in “{name}”).", plural(n, "song", "songs")),
        (n, None) => format!("Added {}.", plural(n, "song", "songs")),
    };
    text.push_str(" Restart your iPhone to see them in the Music app.");
    Ok(AppleMusicResult { ok: true, text, details: left_out })
}

/// Removes the songs copied by a run that didn't finish (the library never saw them).
async fn remove_all(afc: &mut idevice::afc::AfcClient, uploaded: &Mutex<Vec<String>>) {
    let paths = uploaded.lock().unwrap().clone();
    for path in paths {
        let _ = device::remove(afc, &path).await;
    }
}

/// Puts the library from before the last change back (the songs that change added
/// stay in the music folder, unlisted).
async fn restore(pairing: &Pairing, say: &Say) -> Result<AppleMusicResult, String> {
    let work = work_dir();
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let db = work.join("MediaLibrary.sqlitedb");
    let file = db.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        for suffix in ["", "-wal", "-shm"] {
            let from = backup_dir().join(format!("MediaLibrary.sqlitedb{suffix}"));
            if from.exists() {
                std::fs::copy(&from, sidecar(&file, suffix)).map_err(|e| format!("The kept library couldn't be read ({e})"))?;
            }
        }
        library::finish(&library::open_copy(&file)?)
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut dev = connect(pairing).await?;
    say.step("Closing the Music app…", false);
    dev.quit_music().await;
    say.step("Undoing…", false);
    let mut afc = dev.afc().await?;
    replace_library(&mut afc, &db).await?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(AppleMusicResult { ok: true, text: "Undone. Restart your iPhone to see the change in the Music app.".into(), details: Vec::new() })
}

/// The library downloaded (and its -wal/-shm) as it was, replacing the last copy.
fn keep_backup(db: &Path) {
    let dir = backup_dir();
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for suffix in ["", "-wal", "-shm"] {
        let from = sidecar(db, suffix);
        if from.exists() {
            let _ = std::fs::copy(&from, dir.join(format!("MediaLibrary.sqlitedb{suffix}")));
        }
    }
}

/// Uploads the added songs, a few at a time, each on its own AFC connection.
async fn upload_songs(dev: &mut Device, songs: &[song::Song], added: &[library::Added], music: &str, stop: &Arc<AtomicBool>, say: &Say, uploaded: &Arc<Mutex<Vec<String>>>) -> Result<(), String> {
    let total_bytes: u64 = added.iter().map(|a| songs[a.song].file_size as u64).sum::<u64>().max(1);
    let queue: Arc<Mutex<VecDeque<(PathBuf, String)>>> = Arc::new(Mutex::new(added.iter().map(|a| (songs[a.song].path.clone(), format!("{music}/{}", a.file))).collect()));
    let total = added.len();
    let sent = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicU64::new(0));
    let mut workers = Vec::new();
    for _ in 0..UPLOADERS.min(total) {
        let mut afc = dev.afc().await?;
        let (queue, sent, done, stop, uploaded, say) = (queue.clone(), sent.clone(), done.clone(), stop.clone(), uploaded.clone(), say.clone());
        workers.push(tokio::spawn(async move {
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let Some((from, to)) = queue.lock().unwrap().pop_front() else { return Ok(()) };
                uploaded.lock().unwrap().push(to.clone());
                let report = |n: u64| {
                    let s = sent.fetch_add(n, Ordering::Relaxed) + n;
                    let d = done.load(Ordering::Relaxed) as usize;
                    say.progress(format!("Copying songs ({} of {total})…", (d + 1).min(total)), s as f32 / total_bytes as f32);
                };
                device::upload(&mut afc, &from, &to, report).await?;
                done.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    let mut first_error = None;
    for w in workers {
        let r = w.await.map_err(|e| e.to_string()).and_then(|r: Result<(), String>| r);
        if let Err(e) = r {
            stop.store(true, Ordering::Relaxed);
            first_error.get_or_insert(e);
        }
    }
    match first_error {
        Some(e) => Err(format!("A song couldn't be copied to the iPhone: {e}")),
        None => Ok(()),
    }
}

/// Uploads the new library beside the old one and swaps them; on any failure the old
/// one stays (ByeTunes' `replaceRemoteMediaLibrary`).
async fn replace_library(afc: &mut idevice::afc::AfcClient, db: &Path) -> Result<(), String> {
    let temp = format!("{LIBRARY}.temp");
    let backup = format!("{LIBRARY}.backup");
    device::make_dirs(afc, "/iTunes_Control/iTunes").await;
    device::upload(afc, db, &temp, |_| {}).await.map_err(|e| format!("The new library couldn't be copied to the iPhone: {e}"))?;
    let _ = device::remove(afc, &format!("{LIBRARY}-shm")).await;
    let _ = device::remove(afc, &format!("{LIBRARY}-wal")).await;
    let _ = device::remove(afc, &backup).await;
    let had = device::exists(afc, LIBRARY).await;
    if had && device::rename(afc, LIBRARY, &backup).await.is_err() {
        let _ = device::remove(afc, &temp).await;
        return Err("The library couldn't be replaced (the old one couldn't be moved aside). Nothing was changed.".into());
    }
    if let Err(e) = device::rename(afc, &temp, LIBRARY).await {
        let back = if had { device::rename(afc, &backup, LIBRARY).await.is_ok() } else { true };
        let _ = device::remove(afc, &temp).await;
        return Err(if back {
            format!("The library couldn't be replaced ({e}). The old one was put back.")
        } else {
            format!("The library couldn't be replaced ({e}), and the old one couldn't be put back: it's {backup} on the iPhone. Try Restore.")
        });
    }
    if had {
        let _ = device::remove(afc, &backup).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `run` on a job thread as `apple_music_start` makes it (current-thread runtime,
    /// JOB_STACK). The deep path that overflowed 2 MB (connect → RemotePairing tunnel →
    /// RSD handshake) needs an iPhone; here the run stops at reading the songs.
    #[test]
    fn run_gets_a_big_stack() {
        assert!(JOB_STACK >= 8 << 20);
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = crate::config::test_dir("am-stack");
        std::fs::create_dir_all(&dir).unwrap();
        let job = Job {
            pairing: Pairing::Remote(Box::new(idevice::remote_pairing::RpPairingFile::generate("test"))),
            paths: vec![dir.join("missing.mp3")],
            playlist: None,
            stop: Arc::new(AtomicBool::new(false)),
        };
        let r = std::thread::Builder::new()
            .stack_size(JOB_STACK)
            .spawn(move || tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(run(job, Say(Tx(tx)))))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        assert!(!r.ok);
        assert_eq!(r.text, "None of these songs could be added.");
        assert!(r.details[0].contains("can't play") || r.details[0].contains("not there"), "{:?}", r.details);
        drop(rx);
    }
}
