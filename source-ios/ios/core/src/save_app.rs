//! Share ▸ "Download as M4A/MP3" on iOS: the desktop's save-song orchestration
//! (src/app.rs `save_song`, `save_set`, `start_save`, `song_save_moved`,
//! `check_set_done`) on the shared pipeline (src/save_song/: JioSaavn, then YouTube
//! Music; M4A kept as found, MP3 by LAME). Files go into Documents (Files ›
//! On My iPhone › Glass), which Local Files reads, so they play straight away.
//! Three at a time; a collection goes into a folder of its own, with an .m3u8 for
//! playlists, and Stop stops the songs not started yet. A playlist also becomes a
//! local playlist (its name, description, cover and order; downloading it again
//! updates that one), so it plays, and goes to Apple Music, as one.

use crate::app::{MobileApp, Msg};
use crate::bridge::CoreEvent;
use crate::model::s;
use crate::save_song::{self as save, Job, SetKind, Status};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// A playlist, album or artist being saved as files.
pub struct SaveSet {
    pub name: String,
    pub kind: SetKind,
    pub folder: std::path::PathBuf,
    pub keys: Vec<String>,
    pub skipped: usize,
    pub stop: Arc<std::sync::atomic::AtomicBool>,
    pub finished: bool,
    /// A playlist's description and cover image file, for its local playlist.
    pub description: String,
    pub cover: Option<std::path::PathBuf>,
}

pub struct SaveState {
    pub jobs: HashMap<String, Job>,
    pub sets: HashMap<String, SaveSet>,
    http: reqwest::Client,
    slots: Arc<tokio::sync::Semaphore>,
    published: Option<SavesModel>,
}

impl Default for SaveState {
    fn default() -> Self {
        Self { jobs: HashMap::new(), sets: HashMap::new(), http: save::http_client(), slots: Arc::new(tokio::sync::Semaphore::new(3)), published: None }
    }
}

#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum SaveStatus {
    Queued,
    Stopped,
    Finding,
    Downloading { percent: u8 },
    Converting,
    Done,
    Failed { why: String },
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct SaveJobModel {
    /// The song's URI (a lone save), or "<collection URI>|<song URI>".
    pub key: String,
    pub title: String,
    pub status: SaveStatus,
    pub path: String,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct SaveSetModel {
    pub uri: String,
    pub name: String,
    pub saved: u32,
    pub failed: u32,
    pub stopped: u32,
    pub going: u32,
    pub folder: String,
    pub finished: bool,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct SavesModel {
    pub jobs: Vec<SaveJobModel>,
    pub sets: Vec<SaveSetModel>,
    /// "M4A" or "MP3".
    pub format: String,
}

fn status_model(s: &Status) -> SaveStatus {
    match s {
        Status::Queued => SaveStatus::Queued,
        Status::Stopped => SaveStatus::Stopped,
        Status::Finding => SaveStatus::Finding,
        Status::Downloading(p) => SaveStatus::Downloading { percent: *p },
        Status::Converting => SaveStatus::Converting,
        Status::Done => SaveStatus::Done,
        Status::Failed(w) => SaveStatus::Failed { why: w.clone() },
    }
}

impl MobileApp {
    fn save_options(&self) -> save::Options {
        save::Options { format: self.settings.save_format, mp3_kbps: self.settings.save_mp3_kbps, dir: self.settings.save_dir.clone().unwrap_or_else(save::default_dir) }
    }

    /// Desktop `save_song`.
    pub fn save_song(&mut self, item: Value) {
        let uri = s(&item, "uri").to_string();
        let Some(song) = save::song_from_item(&item) else {
            self.toast("Only Spotify songs can be downloaded");
            return;
        };
        if self.saves.jobs.get(&uri).is_some_and(Job::busy) {
            return;
        }
        let opts = self.save_options();
        let (title, format) = (song.title.clone(), opts.format.name());
        match self.start_save(uri, song, opts, None) {
            Status::Done => self.toast(format!("“{title}” is already saved as {format}")),
            _ => self.toast(format!("Saving “{title}” as {format}…")),
        }
    }

    /// A playlist, album or artist (its top 10): the core fetches the songs, then `save_set`.
    pub fn save_collection(&mut self, uri: String, name: String) {
        let kind = match crate::model::kind_of(&uri) {
            "playlist" => SetKind::Playlist,
            "album" => SetKind::Album,
            "artist" => SetKind::Artist,
            _ => {
                self.toast("Only playlists, albums and artists can be downloaded as files");
                return;
            }
        };
        if self.demo {
            self.toast("Saving songs needs the network (not in the demo)");
            return;
        }
        let Some(api) = self.api.clone() else { return };
        let tx = self.tx.clone();
        let id = crate::model::id_of(&uri).to_string();
        let (http, folder) = (self.saves.http.clone(), self.save_options().dir.join(save::file_name(&name)));
        self.rt.spawn(async move {
            let (description, cover) = if kind == SetKind::Playlist { playlist_about(&api, &http, &id, &folder).await } else { (String::new(), None) };
            let items: anyhow::Result<Vec<Value>> = async {
                Ok(match kind {
                    SetKind::Playlist => crate::model::items(&api.get_all(&format!("/playlists/{id}/tracks?limit=100"), 10_000).await?).iter().map(|i| crate::model::playlist_entry(i).clone()).collect(),
                    SetKind::Album => {
                        let a = api.get(&format!("/albums/{id}")).await?;
                        let tracks = api.get_all(&format!("/albums/{id}/tracks?limit=50"), 2000).await?;
                        // Album tracks don't carry their album; songs need it for tags and covers.
                        let stub = serde_json::json!({ "id": s(&a, "id"), "name": s(&a, "name"), "uri": s(&a, "uri"), "images": a["images"].clone(), "release_date": a["release_date"].clone(), "artists": a["artists"].clone() });
                        crate::model::items(&tracks)
                            .iter()
                            .map(|t| {
                                let mut t = t.clone();
                                if t.get("album").is_none_or(Value::is_null) {
                                    t["album"] = stub.clone();
                                }
                                t
                            })
                            .collect()
                    }
                    SetKind::Artist => crate::model::arr(&api.get(&format!("/artists/{id}/top-tracks?market=from_token")).await?, "tracks").iter().take(10).cloned().collect(),
                })
            }
            .await;
            tx.send(match items {
                Ok(items) => Msg::SaveSetItems { uri, name, kind, items, description, cover },
                Err(e) => Msg::Failed(e.to_string()),
            });
        });
    }

    /// Desktop `save_set`.
    pub fn save_set(&mut self, uri: String, name: String, kind: SetKind, items: Vec<Value>, description: String, cover: Option<std::path::PathBuf>) {
        if self.saves.sets.get(&uri).is_some_and(|set| !set.finished) {
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
            if !seen.insert(song_uri.clone()) || !paths.insert(save::output_path(&song, &opts)) {
                continue;
            }
            let key = format!("{uri}|{song_uri}");
            if !self.saves.jobs.get(&key).is_some_and(|j: &Job| j.busy()) {
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
        self.saves.sets.insert(uri.clone(), SaveSet { name: name.clone(), kind, folder, keys, skipped, stop, finished: false, description, cover });
        let what = if kind == SetKind::Artist { format!("{name}'s top songs") } else { format!("“{name}”") };
        self.toast(format!("Saving {count} {} from {what} as {format}…", if count == 1 { "song" } else { "songs" }));
        self.check_set_done(&uri);
    }

    pub fn stop_save_set(&mut self, uri: &str) {
        if let Some(set) = self.saves.sets.get(uri) {
            set.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let name = set.name.clone();
            self.toast(format!("Stopping “{name}” after the songs already downloading"));
        }
    }

    /// Desktop `start_save`.
    fn start_save(&mut self, key: String, song: save::Song, opts: save::Options, stop: Option<Arc<std::sync::atomic::AtomicBool>>) -> Status {
        let path = save::output_path(&song, &opts);
        let title = song.title.clone();
        if path.exists() {
            self.saves.jobs.insert(key, Job { title, status: Status::Done, path });
            return Status::Done;
        }
        self.saves.jobs.insert(key.clone(), Job { title: title.clone(), status: Status::Queued, path });
        let (tx, http, slots) = (self.tx.clone(), self.saves.http.clone(), self.saves.slots.clone());
        let report: Arc<dyn Fn(Status) + Send + Sync> = Arc::new(move |status| tx.send(Msg::SaveSong(key.clone(), status)));
        self.rt.spawn(async move {
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
            let done = match save::save(http, song, opts, report.clone()).await {
                Ok(_) => Status::Done,
                Err(e) => {
                    log::warn!("couldn't save “{title}”: {e:#}");
                    Status::Failed(save::short_error(&e))
                }
            };
            report(done);
        });
        Status::Queued
    }

    /// Desktop `song_save_moved`.
    pub fn song_save_moved(&mut self, key: String, status: Status) {
        let Some(job) = self.saves.jobs.get_mut(&key) else { return };
        if !job.busy() {
            return;
        }
        job.status = status.clone();
        if job.busy() {
            return;
        }
        let (title, path) = (job.title.clone(), job.path.clone());
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
            Status::Failed(why) => self.toast(if why.contains("couldn't find") {
                format!("Couldn't find “{title}” to download")
            } else if why.contains("network") || why.contains("stalled") {
                format!("Couldn't download “{title}”. Check your connection.")
            } else {
                format!("Couldn't download “{title}”. Try again later.")
            }),
            _ => {}
        }
    }

    fn set_progress(&self, uri: &str) -> Option<(usize, usize, usize, usize)> {
        let set = self.saves.sets.get(uri)?;
        let mut counts = (0, 0, 0, 0);
        for job in set.keys.iter().filter_map(|k| self.saves.jobs.get(k)) {
            match job.status {
                Status::Done => counts.0 += 1,
                Status::Failed(_) => counts.1 += 1,
                Status::Stopped => counts.2 += 1,
                _ => counts.3 += 1,
            }
        }
        Some(counts)
    }

    /// Desktop `check_set_done`: one toast when the collection settles, and the playlist file.
    fn check_set_done(&mut self, uri: &str) {
        let Some((saved, failed, stopped, going)) = self.set_progress(uri) else { return };
        let Some(set) = self.saves.sets.get_mut(uri) else { return };
        if going > 0 || set.finished {
            return;
        }
        set.finished = true;
        let (name, kind, folder, skipped) = (set.name.clone(), set.kind, set.folder.clone(), set.skipped);
        let (description, cover) = (set.description.clone(), set.cover.clone().filter(|c| c.exists()));
        let saved_paths: Vec<std::path::PathBuf> = set.keys.iter().filter_map(|k| self.saves.jobs.get(k)).filter(|j| j.status == Status::Done).map(|j| j.path.clone()).collect();
        let mut local_playlist = false;
        if kind == SetKind::Playlist && !saved_paths.is_empty() {
            if let Err(e) = save::write_playlist_file(&folder, &name, &saved_paths) {
                log::warn!("couldn't write the playlist file for {name}: {e}");
            }
            // The songs play from Local Files: a folder picked for saving joins it.
            if !crate::local::read_by(&self.settings.local_dirs, &folder) && crate::local::add_source(&mut self.settings.local_dirs, folder.clone()) {
                self.mark_settings_dirty();
            }
            match self.playlists.from_download(uri, &name, &description, saved_paths.clone(), cover) {
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

    /// Saved files join Local Files when their folder is read by it (Documents is).
    fn rescan_if_in_local_files(&mut self, path: &std::path::Path) {
        let dir = if path.is_dir() { Some(path) } else { path.parent() };
        if dir.is_some_and(|d| crate::local::read_by(&self.settings.local_dirs, d)) {
            self.apply(crate::app::Action::RescanLocal);
        }
    }

    pub fn publish_saves(&mut self, events: &mut Vec<CoreEvent>) {
        let mut jobs: Vec<SaveJobModel> = self.saves.jobs.iter().map(|(k, j)| SaveJobModel { key: k.clone(), title: j.title.clone(), status: status_model(&j.status), path: j.path.to_string_lossy().into_owned() }).collect();
        jobs.sort_by(|a, b| a.key.cmp(&b.key));
        let mut sets: Vec<SaveSetModel> = self
            .saves
            .sets
            .iter()
            .map(|(uri, set)| {
                let (saved, failed, stopped, going) = self.set_progress(uri).unwrap_or_default();
                SaveSetModel { uri: uri.clone(), name: set.name.clone(), saved: saved as u32, failed: failed as u32, stopped: stopped as u32, going: going as u32, folder: set.folder.to_string_lossy().into_owned(), finished: set.finished }
            })
            .collect();
        sets.sort_by(|a, b| a.uri.cmp(&b.uri));
        let m = SavesModel { jobs, sets, format: self.settings.save_format.name().to_string() };
        if self.saves.published.as_ref() != Some(&m) {
            self.saves.published = Some(m.clone());
            events.push(CoreEvent::Saves { saves: m });
        }
    }
}

/// A playlist's description (plain text) and cover, the cover saved as `cover.jpg` in
/// the folder its songs go to. Either may be missing; the songs are saved regardless.
async fn playlist_about(api: &crate::api::Api, http: &reqwest::Client, id: &str, folder: &std::path::Path) -> (String, Option<std::path::PathBuf>) {
    let Ok(v) = api.get(&format!("/playlists/{id}?fields=description,images")).await else { return (String::new(), None) };
    let description = crate::model::plain(s(&v, "description"));
    let url = v["images"].as_array().and_then(|imgs| imgs.iter().max_by_key(|i| i["width"].as_u64().unwrap_or(0))).and_then(|i| i["url"].as_str()).map(str::to_string);
    let cover = match url {
        Some(url) => async {
            let bytes = http.get(&url).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
            std::fs::create_dir_all(folder).ok()?;
            let path = folder.join("cover.jpg");
            std::fs::write(&path, &bytes).ok()?;
            Some(path)
        }
        .await,
        None => None,
    };
    (description, cover)
}
