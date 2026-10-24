//! Downloads, as in Spotify: whole tracks and episodes kept in Glass's audio
//! cache, so they start instantly and never stream again. They're at the
//! streaming quality; after it changes, they're downloaded again at the new one.
//!
//! Files stay encrypted exactly as Spotify serves them and only play inside
//! Glass while signed in to the account that downloaded them (each play still
//! fetches its decryption key, so a connection is needed). Spotify's audio is
//! never written out as plain files; Share ▸ "Download as …" (`save_song`)
//! saves a copy matched on JioSaavn or YouTube instead.
//!
//! Like Spotify's app, each song is fetched as one whole file straight from
//! Spotify's audio CDN, several at a time, and lands in the cache in one
//! piece. (Reading through librespot's streaming loader instead takes
//! hundreds of small requests per song, paced for playback, and runs into
//! librespot's per-domain request limit.) That loader stays as a fallback.
//! Failed songs are retried a few times with growing pauses.
//!
//! librespot's own cache limit evicts by age and would delete downloads, so
//! Glass runs the cache without a limit and prunes it here instead, skipping
//! downloaded files.

use crate::config::{Quality, config_dir};
use anyhow::{Context, Result, anyhow};
use librespot_audio::AudioFile;
use librespot_core::cache::Cache;
use librespot_core::cdn_url::CdnUrl;
use librespot_core::{FileId, Session, SpotifyUri};
use librespot_metadata::audio::{AudioFileFormat, AudioItem};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// Songs downloading at once.
const WORKERS: usize = 4;
/// At most this many songs start per second. Each start asks Spotify for the
/// song's metadata and CDN address, which shares librespot's request budget
/// (300 per 30 s per domain) with playback and the rest of Glass.
const STARTS_PER_SECOND: f64 = 2.0;
/// Whole-file tries before falling back to the streaming loader.
const ATTEMPTS: u32 = 3;
/// Rounds a failing song gets per session (each after a longer pause).
const ROUNDS: u32 = 3;
/// Give up on a transfer that sends nothing for this long.
const STALL: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Index {
    /// Downloaded playlists, albums, Liked Songs, shows... by URI.
    pub collections: BTreeMap<String, Collection>,
    /// Every downloaded track or episode, by URI.
    pub files: BTreeMap<String, Stored>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Collection {
    pub name: String,
    pub items: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Stored {
    pub file_id: String,
    pub bytes: u64,
    /// The streaming quality it was downloaded at (the player asks for that file).
    /// Missing in downloads from before it was kept: taken to be the current one.
    #[serde(default)]
    pub quality: Option<Quality>,
}

#[derive(Default)]
struct Shared {
    index: Index,
    queue: VecDeque<String>,
    active: HashSet<String>,
    /// Songs that failed: when to try again.
    retry: Vec<(String, Instant)>,
    /// Failed rounds so far, by song.
    rounds: HashMap<String, u32>,
    /// Songs given up on this session.
    failed: usize,
    /// Downloads finished since the index was last written.
    unsaved: usize,
    /// Which `run` the workers belong to; older workers stop.
    run: u64,
    /// `progress` answers by collection, with the sizes they were counted at
    /// (the downloaded playlist on screen asks every frame).
    progress_memo: HashMap<String, (usize, usize, usize)>,
    /// `total_bytes`, with the number of files it was counted at.
    bytes_memo: (usize, u64),
    /// The audio cache of the session `run` last got, for `add` to delete
    /// files no download uses any more.
    cache: Option<Arc<Cache>>,
}

impl Shared {
    fn wanted(&self, uri: &str) -> bool {
        self.index.collections.values().any(|c| c.items.iter().any(|u| u == uri))
    }

    /// Everything queued, being downloaded or waiting to be retried, to check
    /// many songs against at once.
    fn in_hand(&self) -> HashSet<String> {
        self.queue.iter().chain(self.active.iter()).chain(self.retry.iter().map(|(u, _)| u)).cloned().collect()
    }

    /// Forgets the downloads to fetch again: files gone from the cache, and those of
    /// another quality (left to be pruned like any cached file).
    fn forget_stale(&mut self, quality: Quality, in_cache: impl Fn(&Stored) -> bool) {
        for f in self.index.files.values_mut() {
            f.quality.get_or_insert(quality);
        }
        self.index.files.retain(|_, f| f.quality == Some(quality) && in_cache(f));
    }

    /// Queues the songs not downloaded and not already in hand, each once.
    fn queue_missing(&mut self, items: impl IntoIterator<Item = String>) {
        let mut seen = self.in_hand();
        for u in items {
            if !self.index.files.contains_key(&u) && seen.insert(u.clone()) {
                self.queue.push_back(u);
            }
        }
    }

    /// Lets go of what no collection holds any more: queued songs, and
    /// downloads (their file ids are returned, to delete from the cache).
    fn drop_unwanted(&mut self) -> Vec<String> {
        let keep: HashSet<String> = self.index.collections.values().flat_map(|c| c.items.iter().cloned()).collect();
        self.queue.retain(|u| keep.contains(u));
        self.retry.retain(|(u, _)| keep.contains(u));
        let orphans: Vec<String> = self.index.files.keys().filter(|u| !keep.contains(*u)).cloned().collect();
        orphans.iter().filter_map(|u| self.index.files.remove(u)).map(|f| f.file_id).collect()
    }

    /// The next song to download (retries that are due go first).
    fn take(&mut self, now: Instant) -> Option<String> {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.retry).into_iter().partition(|(_, at)| *at <= now);
        self.retry = later;
        for (u, _) in due.into_iter().rev() {
            if self.wanted(&u) {
                self.queue.push_front(u);
            }
        }
        let uri = self.queue.pop_front()?;
        self.active.insert(uri.clone());
        Some(uri)
    }

    /// Records how a download went; true when the index should be written.
    fn finish(&mut self, uri: &str, result: Result<Stored, String>, now: Instant) -> bool {
        self.active.remove(uri);
        match result {
            Ok(stored) => {
                self.rounds.remove(uri);
                // Skip if the collection was removed meanwhile.
                if self.wanted(uri) {
                    self.index.files.insert(uri.to_string(), stored);
                    self.unsaved += 1;
                }
            }
            Err(e) => {
                let n = {
                    let n = self.rounds.entry(uri.to_string()).or_insert(0);
                    *n += 1;
                    *n
                };
                if n < ROUNDS && self.wanted(uri) {
                    log::info!("download {uri} failed ({e}); trying again shortly");
                    self.retry.push((uri.to_string(), now + Duration::from_secs(15 * 2u64.pow(n))));
                } else {
                    log::warn!("download {uri} failed: {e}");
                    self.failed += 1;
                }
            }
        }
        let idle = self.queue.is_empty() && self.active.is_empty();
        self.unsaved > 0 && (self.unsaved >= 10 || idle)
    }
}

/// Cheap to clone; every clone sees the same downloads.
#[derive(Clone)]
pub struct Downloads {
    shared: Arc<Mutex<Shared>>,
    wake: Arc<tokio::sync::Notify>,
}

fn file_id(hex: &str) -> Option<FileId> {
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect();
    bytes.filter(|b| b.len() == 20).map(|b| FileId::from_raw(&b))
}

fn index_path() -> PathBuf {
    config_dir().join("downloads.json")
}

/// The same preference order librespot's player uses, so playback finds the file.
fn formats(q: Quality) -> [AudioFileFormat; 7] {
    use AudioFileFormat::*;
    match q {
        Quality::Normal => [OGG_VORBIS_96, MP3_96, OGG_VORBIS_160, MP3_160, MP3_256, OGG_VORBIS_320, MP3_320],
        Quality::High => [OGG_VORBIS_160, MP3_160, OGG_VORBIS_96, MP3_96, MP3_256, OGG_VORBIS_320, MP3_320],
        Quality::VeryHigh => [OGG_VORBIS_320, MP3_320, MP3_256, OGG_VORBIS_160, MP3_160, OGG_VORBIS_96, MP3_96],
    }
}

fn bytes_per_second(f: AudioFileFormat) -> usize {
    use AudioFileFormat::*;
    let kbps = match f {
        OGG_VORBIS_96 | MP3_96 => 12,
        OGG_VORBIS_160 | MP3_160 => 20,
        MP3_256 => 32,
        _ => 40,
    };
    kbps * 1024
}

impl Downloads {
    pub fn load() -> Self {
        let index = crate::config::read_json(&index_path());
        Self { shared: Arc::new(Mutex::new(Shared { index, ..Default::default() })), wake: Arc::default() }
    }

    fn save(index: &Index) {
        let _ = std::fs::create_dir_all(config_dir());
        if let Ok(s) = serde_json::to_string(index) {
            let tmp = index_path().with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, index_path());
            }
        }
    }

    /// Wakes every idle worker (and leaves a wake-up for one that's about to wait).
    fn wake_all(&self) {
        self.wake.notify_waiters();
        self.wake.notify_one();
    }

    /// Download every item of a collection (and keep it downloaded). Songs it
    /// no longer has are deleted, as in `remove`, unless another download has them.
    pub fn add(&self, uri: &str, name: &str, items: Vec<String>) {
        let mut s = self.shared.lock().unwrap();
        let items: Vec<String> = items.into_iter().filter(|u| u.starts_with("spotify:track:") || u.starts_with("spotify:episode:")).collect();
        s.index.collections.insert(uri.to_string(), Collection { name: name.to_string(), items: items.clone() });
        // Its counts can stay the same while a song is swapped for another.
        s.progress_memo.remove(uri);
        let orphans = s.drop_unwanted();
        s.queue_missing(items);
        let (index, cache) = (s.index.clone(), s.cache.clone());
        drop(s);
        Self::save(&index);
        if !orphans.is_empty() {
            std::thread::spawn(move || delete_files(cache, &orphans));
        }
        self.wake_all();
    }

    /// Stops keeping collections (`None`: all of them). Files no other download
    /// uses are deleted on another thread (a big download is thousands of files).
    pub fn remove(&self, uri: Option<&str>, cache: Option<Arc<Cache>>) {
        let (index, orphans) = {
            let mut s = self.shared.lock().unwrap();
            match uri {
                Some(u) => {
                    s.index.collections.remove(u);
                }
                None => s.index.collections.clear(),
            }
            let ids = s.drop_unwanted();
            (s.index.clone(), ids)
        };
        std::thread::spawn(move || {
            Self::save(&index);
            delete_files(cache, &orphans);
        });
    }

    pub fn is_kept(&self, uri: &str) -> bool {
        self.shared.lock().unwrap().index.collections.contains_key(uri)
    }

    /// (downloaded, total) items of a kept collection.
    pub fn progress(&self, uri: &str) -> Option<(usize, usize)> {
        let mut s = self.shared.lock().unwrap();
        let (files, items) = (s.index.files.len(), s.index.collections.get(uri)?.items.len());
        if let Some(&(f, i, done)) = s.progress_memo.get(uri)
            && (f, i) == (files, items)
        {
            return Some((done, items));
        }
        let c = s.index.collections.get(uri)?;
        let done = c.items.iter().filter(|u| s.index.files.contains_key(*u)).count();
        s.progress_memo.insert(uri.to_string(), (files, items, done));
        Some((done, items))
    }

    pub fn has_file(&self, uri: &str) -> bool {
        self.shared.lock().unwrap().index.files.contains_key(uri)
    }

    pub fn collections(&self) -> Vec<(String, String)> {
        self.shared.lock().unwrap().index.collections.iter().map(|(u, c)| (u.clone(), c.name.clone())).collect()
    }

    /// How many playlists, albums and other collections are downloaded.
    pub fn count(&self) -> usize {
        self.shared.lock().unwrap().index.collections.len()
    }

    pub fn total_bytes(&self) -> u64 {
        let mut s = self.shared.lock().unwrap();
        let files = s.index.files.len();
        if s.bytes_memo.0 != files {
            s.bytes_memo = (files, s.index.files.values().map(|f| f.bytes).sum());
        }
        s.bytes_memo.1
    }

    /// (items still to download, items that failed this session)
    pub fn pending(&self) -> (usize, usize) {
        let s = self.shared.lock().unwrap();
        (s.queue.len() + s.active.len() + s.retry.len(), s.failed)
    }

    fn kept_file_ids(&self) -> Vec<String> {
        self.shared.lock().unwrap().index.files.values().map(|f| f.file_id.clone()).collect()
    }

    /// Works through the queue on `session` until the session ends (a new
    /// session calls this again). Re-queues anything whose file went missing
    /// from the cache.
    pub fn run(&self, session: Session, quality: Quality, on_change: impl Fn() + Send + Sync + 'static) {
        let run = {
            let mut s = self.shared.lock().unwrap();
            s.run += 1;
            let cache = session.cache();
            s.cache = cache.cloned();
            // A look at each file, not an open: a big library has thousands.
            s.forget_stale(quality, |f| file_id(&f.file_id).and_then(|id| cache.and_then(|c| c.file_path(id))).is_some_and(|p| p.exists()));
            Self::save(&s.index);
            let wanted: Vec<String> = s.index.collections.values().flat_map(|c| c.items.iter().cloned()).collect();
            // Songs a previous session's workers had in hand start over here.
            s.active.clear();
            s.retry.clear();
            s.rounds.clear();
            s.queue_missing(wanted);
            s.failed = 0;
            s.run
        };
        let http = http_client(&session);
        let pacer = Arc::new(tokio::sync::Mutex::new(Instant::now()));
        let on_change: Arc<dyn Fn() + Send + Sync> = Arc::new(on_change);
        for _ in 0..WORKERS {
            let (this, session, http, pacer, on_change) = (self.clone(), session.clone(), http.clone(), pacer.clone(), on_change.clone());
            tokio::spawn(async move { this.worker(run, session, http, quality, pacer, on_change).await });
        }
        self.wake_all();
    }

    async fn worker(self, run: u64, session: Session, http: reqwest::Client, quality: Quality, pacer: Arc<tokio::sync::Mutex<Instant>>, on_change: Arc<dyn Fn() + Send + Sync>) {
        loop {
            if session.is_invalid() || self.shared.lock().unwrap().run != run {
                break;
            }
            let (next, retrying) = {
                let mut s = self.shared.lock().unwrap();
                (s.take(Instant::now()), !s.retry.is_empty())
            };
            let Some(uri) = next else {
                // Nothing to do: sleep until there is (`wake_all`), looking now and then
                // whether this session has ended.
                let nap = if retrying { Duration::from_secs(5) } else { Duration::from_secs(300) };
                tokio::select! {
                    _ = self.wake.notified() => {}
                    _ = tokio::time::sleep(nap) => {}
                }
                continue;
            };
            on_change();
            // Space out starts across all workers.
            {
                let mut next_start = pacer.lock().await;
                let now = Instant::now();
                if *next_start > now {
                    tokio::time::sleep(*next_start - now).await;
                }
                *next_start = Instant::now().max(*next_start) + Duration::from_secs_f64(1.0 / STARTS_PER_SECOND);
            }
            let r = download(&session, &http, &uri, quality).await.map_err(|e| format!("{e:#}"));
            // The index is written after the lock is let go (the UI takes it every frame).
            let mut s = self.shared.lock().unwrap();
            if s.run != run {
                // A new session took over; it has re-queued this song if needed.
                if let Ok(stored) = r
                    && s.wanted(&uri)
                {
                    s.index.files.insert(uri.clone(), stored);
                    s.queue.retain(|u| *u != uri);
                    let index = s.index.clone();
                    drop(s);
                    Self::save(&index);
                }
                break;
            }
            let save = s.finish(&uri, r, Instant::now()).then(|| {
                s.unsaved = 0;
                s.index.clone()
            });
            drop(s);
            if let Some(index) = save {
                Self::save(&index);
            }
            on_change();
        }
    }

    /// Keeps the streaming part of the audio cache under `limit` bytes,
    /// removing the least recently used files first and never a download.
    pub fn prune(&self, cache: &Cache, audio_dir: &Path, limit: u64) {
        let keep: HashSet<PathBuf> = self.kept_file_ids().iter().filter_map(|id| file_id(id)).filter_map(|id| cache.file_path(id)).collect();
        let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let Ok(dirs) = std::fs::read_dir(audio_dir) else { return };
        for d in dirs.flatten() {
            let Ok(entries) = std::fs::read_dir(d.path()) else { continue };
            for f in entries.flatten() {
                let p = f.path();
                // Skip downloads, and files still arriving (an hour-old part was left by a crash).
                if keep.contains(&p) {
                    continue;
                }
                if p.extension().is_some_and(|e| e == "part") {
                    let stale = f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(3600));
                    if stale {
                        let _ = std::fs::remove_file(&p);
                    }
                    continue;
                }
                if let Ok(m) = f.metadata() {
                    let used = m.accessed().or_else(|_| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                    files.push((p, m.len(), used));
                }
            }
        }
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        files.sort_by_key(|f| f.2);
        for (p, len, _) in files {
            if total <= limit {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total -= len;
            }
        }
    }
}

/// Deletes the files of downloads let go of (`None`: no cache yet; prune
/// clears them out later like any cached file).
fn delete_files(cache: Option<Arc<Cache>>, ids: &[String]) {
    if let Some(cache) = cache {
        for id in ids.iter().filter_map(|id| file_id(id)) {
            let _ = cache.remove_file(id);
        }
    }
}

/// For the CDN, through the same proxy as the Spotify session.
fn http_client(session: &Session) -> reqwest::Client {
    let mut b = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).pool_max_idle_per_host(WORKERS);
    if let Some(p) = session.config().proxy.as_ref().and_then(|u| reqwest::Proxy::all(u.as_str()).ok()) {
        b = b.proxy(p);
    }
    b.build().unwrap_or_default()
}

/// The song's audio, or as the player finds it for a song relinked in this
/// country (with no files of its own): the version Spotify plays instead.
async fn playable(session: &Session, uri: &str) -> Result<AudioItem> {
    let item = AudioItem::get_file(session, SpotifyUri::from_uri(uri)?).await.map_err(|e| anyhow!("{e}"))?;
    if let Err(why) = &item.availability {
        return Err(anyhow!("not available: {why}"));
    }
    if !item.files.is_empty() {
        return Ok(item);
    }
    for alt in item.alternatives.map(|a| a.0).unwrap_or_default() {
        if let Ok(a) = AudioItem::get_file(session, alt).await
            && a.availability.is_ok()
            && !a.files.is_empty()
        {
            return Ok(a);
        }
    }
    Err(anyhow!("no playable file"))
}

async fn download(session: &Session, http: &reqwest::Client, uri: &str, quality: Quality) -> Result<Stored> {
    let item = playable(session, uri).await?;
    let (format, file_id) = formats(quality).iter().find_map(|f| item.files.get(f).map(|id| (*f, *id))).context("no playable file")?;
    let cache = session.cache().context("no audio cache")?;
    let stored = |bytes: u64| Stored { file_id: file_id.to_base16().unwrap_or_default(), bytes, quality: Some(quality) };
    if let Some(f) = cache.file(file_id) {
        return Ok(stored(f.metadata().map(|m| m.len()).unwrap_or(0)));
    }
    let dest = cache.file_path(file_id).context("no cache path")?;
    let mut last = anyhow!("not tried");
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(750 * 2u64.pow(attempt))).await;
        }
        match whole_file(session, http, file_id, &dest).await {
            Ok(bytes) => return Ok(stored(bytes)),
            Err(e) => last = e,
        }
    }
    log::info!("download {uri}: CDN failed ({last:#}); using the streaming loader");
    streamed(session, cache, file_id, format).await.map(stored)
}

/// The whole file from Spotify's CDN, in one request, into the cache.
async fn whole_file(session: &Session, http: &reqwest::Client, file_id: FileId, dest: &Path) -> Result<u64> {
    let cdn = CdnUrl::new(file_id).resolve_audio(session).await.map_err(|e| anyhow!("{e}"))?;
    let mut last = anyhow!("no CDN address");
    for url in cdn.try_get_urls().map_err(|e| anyhow!("{e}"))? {
        match fetch_to(http, url, dest).await {
            Ok(n) => return Ok(n),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Downloads `url` to `dest`: into a `.part` file beside it, renamed into
/// place only once complete, so the cache never holds half a song.
async fn fetch_to(http: &reqwest::Client, url: &str, dest: &Path) -> Result<u64> {
    // CDN addresses carry access tokens: keep them out of errors and logs.
    let clean = |e: reqwest::Error| anyhow!("{}", e.without_url());
    // A Range header, as librespot's requests carry; open-ended here, for the whole file.
    let mut resp = http.get(url).header("Range", "bytes=0-").send().await.map_err(clean)?.error_for_status().map_err(clean)?;
    let expected = resp.content_length();
    // Its own name, so two downloads of one file never share it; prune skips `.part`.
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let part = dest.with_file_name(format!("{name}.{}.part", crate::model::random_hex(6)));
    tokio::fs::create_dir_all(dest.parent().context("no cache folder")?).await?;
    // tokio's files: writing and syncing to disk don't hold up the runtime's two workers.
    let mut file = tokio::fs::File::create(&part).await?;
    let mut n = 0u64;
    let body: Result<()> = async {
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = tokio::time::timeout(STALL, resp.chunk()).await.map_err(|_| anyhow!("download stalled"))?.map_err(clean)? {
            file.write_all(&chunk).await?;
            n += chunk.len() as u64;
        }
        if n == 0 || expected.is_some_and(|e| e != n) {
            return Err(anyhow!("incomplete download ({n} of {} bytes)", expected.unwrap_or(0)));
        }
        file.sync_all().await?;
        Ok(())
    }
    .await;
    drop(file);
    match body.and_then(|_| std::fs::rename(&part, dest).map_err(Into::into)) {
        Ok(()) => Ok(n),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// The fallback: librespot's streaming loader, read to the end, which then
/// saves the file to the cache itself.
async fn streamed(session: &Session, cache: &Cache, file_id: FileId, format: AudioFileFormat) -> Result<u64> {
    let mut file = AudioFile::open(session, file_id, bytes_per_second(format)).await.map_err(|e| anyhow!("{e}"))?;
    let read = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
        let mut buf = vec![0u8; 64 * 1024];
        let mut n = 0u64;
        loop {
            match file.read(&mut buf)? {
                0 => return Ok(n),
                k => n += k as u64,
            }
        }
    })
    .await??;
    for _ in 0..100 {
        if cache.file(file_id).is_some() {
            return Ok(read);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow!("downloaded but not cached"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_ids_round_trip() {
        let id = FileId::from_raw(&[0xab; 20]);
        assert_eq!(file_id(&id.to_base16().unwrap()), Some(id));
        assert_eq!(file_id("zz"), None);
    }

    #[test]
    fn formats_match_the_player() {
        assert_eq!(formats(Quality::VeryHigh)[0], AudioFileFormat::OGG_VORBIS_320);
        assert_eq!(formats(Quality::High)[0], AudioFileFormat::OGG_VORBIS_160);
        assert_eq!(formats(Quality::Normal)[0], AudioFileFormat::OGG_VORBIS_96);
        assert_eq!(bytes_per_second(AudioFileFormat::OGG_VORBIS_320), 40 * 1024);
    }

    #[test]
    fn downloads_follow_the_streaming_quality() {
        let mut s = Shared::default();
        let stored = |id: &str, q| Stored { file_id: id.into(), bytes: 1, quality: q };
        s.index.files.insert("spotify:track:old".into(), stored("a", None));
        s.index.files.insert("spotify:track:high".into(), stored("b", Some(Quality::High)));
        s.index.files.insert("spotify:track:gone".into(), stored("gone", Some(Quality::High)));
        s.forget_stale(Quality::High, |f| f.file_id != "gone");
        let mut left: Vec<&str> = s.index.files.keys().map(String::as_str).collect();
        left.sort();
        assert_eq!(left, ["spotify:track:high", "spotify:track:old"], "older downloads count as the current quality");
        s.forget_stale(Quality::VeryHigh, |_| true);
        assert!(s.index.files.is_empty(), "a new quality downloads everything again");
    }

    fn kept(s: &mut Shared, items: &[&str]) {
        s.index.collections.insert("spotify:playlist:p".into(), Collection { name: "p".into(), items: items.iter().map(|u| u.to_string()).collect() });
    }

    #[test]
    fn failures_retry_later_then_count_as_failed() {
        let mut s = Shared::default();
        kept(&mut s, &["spotify:track:a", "spotify:track:b"]);
        s.queue.extend(["spotify:track:a".to_string(), "spotify:track:b".to_string()]);
        let t0 = Instant::now();
        let a = s.take(t0).unwrap();
        assert!(s.active.contains(&a) && s.in_hand().contains("spotify:track:a"));
        s.finish(&a, Err("offline".into()), t0);
        assert_eq!((s.retry.len(), s.failed), (1, 0), "a failure is retried, not given up on");
        assert_eq!(s.take(t0).as_deref(), Some("spotify:track:b"), "the retry waits its turn");
        let save = s.finish("spotify:track:b", Ok(Stored { file_id: "x".into(), bytes: 1, quality: None }), t0);
        assert!(s.index.files.contains_key("spotify:track:b"));
        assert!(save, "the index is written once nothing is queued or running");
        // Once due, the retry comes back; after ROUNDS failures it's given up.
        let mut now = t0;
        for _ in 1..ROUNDS {
            now += Duration::from_secs(3600);
            assert_eq!(s.take(now).as_deref(), Some("spotify:track:a"));
            s.finish("spotify:track:a", Err("offline".into()), now);
        }
        assert_eq!((s.retry.len(), s.failed), (0, 1));
        assert!(s.take(now + Duration::from_secs(3600)).is_none());
    }

    #[test]
    fn a_song_dropped_meanwhile_is_not_recorded() {
        let mut s = Shared::default();
        kept(&mut s, &["spotify:track:a"]);
        s.queue.push_back("spotify:track:a".into());
        let a = s.take(Instant::now()).unwrap();
        s.index.collections.clear();
        assert!(!s.finish(&a, Ok(Stored { file_id: "x".into(), bytes: 1, quality: None }), Instant::now()));
        assert!(s.index.files.is_empty() && s.active.is_empty());
    }

    #[test]
    fn songs_taken_out_of_a_download_are_let_go() {
        let mut s = Shared::default();
        kept(&mut s, &["spotify:track:a", "spotify:track:b"]);
        for u in ["spotify:track:a", "spotify:track:b"] {
            s.index.files.insert(u.into(), Stored { file_id: format!("{u} file"), bytes: 1, quality: None });
        }
        kept(&mut s, &["spotify:track:a", "spotify:track:c"]);
        s.queue.extend(["spotify:track:b".to_string(), "spotify:track:c".to_string()]);
        assert_eq!(s.drop_unwanted(), ["spotify:track:b file"]);
        assert_eq!(s.index.files.keys().map(String::as_str).collect::<Vec<_>>(), ["spotify:track:a"]);
        assert_eq!(s.queue, ["spotify:track:c"]);
    }

    async fn serve(reply: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(&reply).await;
            sock.shutdown().await.ok();
        });
        format!("http://{addr}/audio/f00?__token__=secret")
    }

    #[tokio::test]
    async fn whole_files_land_in_one_piece() {
        let dir = crate::config::test_dir("fetch");
        let _ = std::fs::remove_dir_all(&dir);
        let dest = dir.join("ab").join("cdef");
        let http = reqwest::Client::new();
        let body = vec![7u8; 300_000];
        let mut ok = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        ok.extend(&body);
        assert_eq!(fetch_to(&http, &serve(ok).await, &dest).await.unwrap(), 300_000);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let no_parts = |d: &Path| std::fs::read_dir(d).unwrap().flatten().all(|e| e.path().extension().is_none_or(|x| x != "part"));
        assert!(no_parts(&dir.join("ab")));

        // Cut off halfway: nothing is left behind.
        let dest2 = dir.join("ab").join("0123");
        let mut short = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n".to_vec();
        short.extend([1u8; 400]);
        assert!(fetch_to(&http, &serve(short).await, &dest2).await.is_err());
        assert!(!dest2.exists() && no_parts(&dir.join("ab")));

        // Errors never carry the CDN address (it holds an access token).
        let e = fetch_to(&http, &serve(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()).await, &dest2).await.unwrap_err();
        assert!(!format!("{e:#}").contains("secret"), "{e:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One real song, whole from the CDN, into a throwaway cache (the user's
    /// downloads and Glass's device are untouched). Uses the signed-in
    /// account; nothing plays. `GLASS_TEST_TRACK` picks the song. Run with
    /// `cargo test --release live_download -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_download() {
        use librespot_core::SessionConfig;
        let creds = Cache::new(Some(config_dir()), None, None, None).unwrap().credentials().expect("sign in to Glass first");
        let dir = crate::config::test_dir("live-dl");
        let cache = Cache::new(None::<PathBuf>, None, Some(dir.clone()), None).unwrap();
        let session = Session::new(SessionConfig::default(), Some(cache));
        session.connect(creds, false).await.expect("connect");
        let uri = std::env::var("GLASS_TEST_TRACK").unwrap_or_else(|_| "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into());
        let item = playable(&session, &uri).await.expect("a playable song");
        let (_, id) = formats(Quality::VeryHigh).iter().find_map(|f| item.files.get(f).map(|id| (*f, *id))).expect("a file in a known format");
        let dest = session.cache().unwrap().file_path(id).unwrap();
        let t = Instant::now();
        let bytes = whole_file(&session, &http_client(&session), id, &dest).await.expect("CDN download");
        println!("ok  {bytes} bytes in {:.1} s from the CDN", t.elapsed().as_secs_f32());
        assert!(bytes > 500_000 && session.cache().unwrap().file(id).is_some());
        let stored = download(&session, &http_client(&session), &uri, Quality::VeryHigh).await.unwrap();
        assert_eq!(stored.bytes, bytes, "a cached song isn't fetched again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_downloads_and_recent_files() {
        let dir = crate::config::test_dir("prune");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new(None::<PathBuf>, None, Some(dir.clone()), None).unwrap();
        let ids: Vec<FileId> = (0u8..3).map(|i| FileId::from_raw(&[i; 20])).collect();
        for id in &ids {
            cache.save_file(*id, &mut std::io::Cursor::new(vec![0u8; 1000])).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        let d = Downloads { shared: Arc::default(), wake: Arc::default() };
        d.shared.lock().unwrap().index.files.insert("spotify:track:a".into(), Stored { file_id: ids[0].to_base16().unwrap(), bytes: 1000, quality: None });
        d.prune(&cache, &dir, 1000);
        assert!(cache.file(ids[0]).is_some(), "a download is never pruned");
        assert!(cache.file(ids[1]).is_none(), "the oldest streamed file goes first");
        assert!(cache.file(ids[2]).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
