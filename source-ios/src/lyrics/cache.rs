//! Lyrics lookups remembered on disk, so each song is looked up once rather
//! than on every play.
//!
//! Matches are kept for 30 days (LRCLIB records get corrected), "nothing
//! found" and "several matches" for a day (LRCLIB fetches missing lyrics in
//! the background, so they may appear). The lyrics a user picks by hand are
//! kept by `choices`, not here. Lyrics stay in the cache folder on this
//! computer and are only shown in Glass.

use super::Match;
use super::TrackRef;
use super::lrclib::Lookup;
use super::parse::Record;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FOUND_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
const MISS_TTL: Duration = Duration::from_secs(24 * 3600);
const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Kind {
    Found(Match),
    Ambiguous,
    None,
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    v: u32,
    key: String,
    /// The track as it was looked up; a change (retagged file) is a miss.
    query: String,
    stored: u64,
    kind: Kind,
    records: Vec<StoredRecord>,
}

/// A record as stored: the fields LRCLIB returned.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredRecord {
    id: u64,
    track_name: String,
    artist_name: String,
    album_name: String,
    duration_ms: u64,
    instrumental: bool,
    has_word_sync: bool,
    plain: Option<String>,
    synced: Option<String>,
    lyricsfile: Option<String>,
}

impl From<&Record> for StoredRecord {
    fn from(r: &Record) -> Self {
        StoredRecord {
            id: r.id,
            track_name: r.track_name.clone(),
            artist_name: r.artist_name.clone(),
            album_name: r.album_name.clone(),
            duration_ms: r.duration_ms,
            instrumental: r.instrumental,
            has_word_sync: r.has_word_sync,
            plain: r.plain.clone(),
            synced: r.synced.clone(),
            lyricsfile: r.lyricsfile.clone(),
        }
    }
}

impl From<StoredRecord> for Record {
    fn from(r: StoredRecord) -> Self {
        Record {
            id: r.id,
            track_name: r.track_name,
            artist_name: r.artist_name,
            album_name: r.album_name,
            duration_ms: r.duration_ms,
            instrumental: r.instrumental,
            has_word_sync: r.has_word_sync,
            plain: r.plain,
            synced: r.synced,
            lyricsfile: r.lyricsfile,
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn query(t: &TrackRef) -> String {
    format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", t.title, t.artist(), t.album, t.duration_ms / 1000)
}

/// A stable file name for a key.
fn file_name(key: &str) -> String {
    format!("{:016x}.json", crate::model::fnv64(key))
}

pub struct Cache {
    dir: Option<PathBuf>,
    mem: Mutex<HashMap<String, Entry>>,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir: Some(dir), mem: Mutex::default() }
    }

    /// Memory only (demo mode).
    pub fn in_memory() -> Self {
        Self { dir: None, mem: Mutex::default() }
    }

    pub fn dir() -> PathBuf {
        crate::config::cache_dir().join("lyrics")
    }

    fn fresh(e: &Entry, t: &TrackRef, at: u64) -> bool {
        let ttl = match e.kind {
            Kind::Found(_) => FOUND_TTL,
            Kind::Ambiguous | Kind::None => MISS_TTL,
        };
        e.v == VERSION && e.key == t.key && e.query == query(t) && at.saturating_sub(e.stored) < ttl.as_secs()
    }

    fn read(&self, t: &TrackRef, at: u64) -> Option<Entry> {
        if let Some(e) = self.mem.lock().unwrap().get(&t.key).filter(|e| Self::fresh(e, t, at)) {
            return Some(e.clone());
        }
        let path = self.dir.as_ref()?.join(file_name(&t.key));
        let e: Entry = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        Self::fresh(&e, t, at).then(|| {
            self.remember(e.clone());
            e
        })
    }

    /// A remembered lookup for `t`, unless it has expired.
    pub fn get(&self, t: &TrackRef) -> Option<Lookup> {
        self.get_at(t, now())
    }

    fn get_at(&self, t: &TrackRef, at: u64) -> Option<Lookup> {
        let e = self.read(t, at)?;
        let mut records = e.records.into_iter().map(Record::from);
        Some(match e.kind {
            Kind::Found(how) => Lookup::Found(records.next()?, how),
            Kind::Ambiguous => Lookup::Ambiguous(records.collect()),
            Kind::None => Lookup::None,
        })
    }

    pub fn put(&self, t: &TrackRef, lookup: &Lookup) {
        self.put_at(t, lookup, now());
    }

    fn put_at(&self, t: &TrackRef, lookup: &Lookup, at: u64) {
        let (kind, records) = match lookup {
            Lookup::Found(r, how) => (Kind::Found(*how), vec![StoredRecord::from(r)]),
            Lookup::Ambiguous(rs) => (Kind::Ambiguous, rs.iter().map(StoredRecord::from).collect()),
            Lookup::None => (Kind::None, Vec::new()),
        };
        self.store(Entry { v: VERSION, key: t.key.clone(), query: query(t), stored: at, kind, records });
    }

    fn store(&self, e: Entry) {
        if let Some(dir) = &self.dir {
            let path = dir.join(file_name(&e.key));
            let write = || -> std::io::Result<()> {
                std::fs::create_dir_all(dir)?;
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, serde_json::to_vec(&e)?)?;
                std::fs::rename(&tmp, &path)
            };
            if let Err(err) = write() {
                log::warn!("lyrics cache: {err}");
            }
        }
        self.remember(e);
    }

    /// Keeps `e` in memory too, for the few songs played lately (the disk has the rest).
    fn remember(&self, e: Entry) {
        const KEEP: usize = 32;
        let mut mem = self.mem.lock().unwrap();
        if mem.len() >= KEEP
            && !mem.contains_key(&e.key)
            && let Some(k) = mem.keys().next().cloned()
        {
            mem.remove(&k);
        }
        mem.insert(e.key.clone(), e);
    }

    /// Drops what's remembered for one track (to look it up again).
    pub fn forget(&self, key: &str) {
        self.mem.lock().unwrap().remove(key);
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join(file_name(key)));
        }
    }

    /// Removes every cached lookup, and the saved translations kept in the same folder.
    pub fn clear(&self) -> std::io::Result<()> {
        self.mem.lock().unwrap().clear();
        match &self.dir {
            Some(dir) if dir.exists() => std::fs::remove_dir_all(dir),
            _ => Ok(()),
        }
    }

    /// (songs looked up, bytes) on disk. The bytes include the saved translations
    /// in their folder here, which `clear` removes too.
    pub fn usage(&self) -> (usize, u64) {
        let Some(dir) = &self.dir else { return (self.mem.lock().unwrap().len(), 0) };
        let mut usage = (0, 0);
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_file() {
                usage = (usage.0 + 1, usage.1 + m.len());
            } else if m.is_dir() {
                usage.1 += std::fs::read_dir(e.path()).into_iter().flatten().flatten().filter_map(|f| f.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum::<u64>();
            }
        }
        usage
    }

    /// Deletes expired entries. Run off the UI thread.
    pub fn prune(&self) {
        let Some(dir) = &self.dir else { return };
        let at = now();
        for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            // Saved translations live in a folder of their own here.
            if f.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            // Written within the shortest lifetime: nothing to check (or read).
            if f.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age < MISS_TTL) {
                continue;
            }
            let keep = std::fs::read(f.path()).ok().and_then(|b| serde_json::from_slice::<Entry>(&b).ok()).is_some_and(|e| {
                let ttl = if matches!(e.kind, Kind::Found(_)) { FOUND_TTL } else { MISS_TTL };
                e.v == VERSION && at.saturating_sub(e.stored) < ttl.as_secs()
            });
            if !keep {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str) -> TrackRef {
        TrackRef { key: key.into(), title: "Song".into(), artists: vec!["Artist".into()], album: "Album".into(), duration_ms: 200_000 }
    }

    fn rec() -> Record {
        crate::lyrics::parse::record(&serde_json::json!({ "id": 42, "trackName": "Song", "artistName": "Artist", "albumName": "Album", "duration": 200, "syncedLyrics": "[00:01.00]hello" })).unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let d = crate::config::test_dir(&format!("lyrics-cache-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn found_survives_a_restart_and_expires() {
        let dir = temp("found");
        let t = track("spotify:track:a");
        Cache::new(dir.clone()).put_at(&t, &Lookup::Found(rec(), Match::Exact), 1_000);
        let c = Cache::new(dir.clone());
        assert!(matches!(c.get_at(&t, 1_000 + 3600), Some(Lookup::Found(r, Match::Exact)) if r.id == 42), "read back from disk");
        assert!(c.get_at(&t, 1_000 + FOUND_TTL.as_secs() + 1).is_none(), "expired");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn misses_expire_sooner() {
        let c = Cache::new(temp("miss"));
        let t = track("spotify:track:b");
        c.put_at(&t, &Lookup::None, 1_000);
        assert_eq!(c.get_at(&t, 1_000 + 3600), Some(Lookup::None));
        assert!(c.get_at(&t, 1_000 + MISS_TTL.as_secs() + 1).is_none());
        c.put_at(&t, &Lookup::Found(rec(), Match::Search), 1_000);
        assert!(c.get_at(&t, 1_000 + MISS_TTL.as_secs() + 1).is_some(), "a match lasts longer");
        assert!(c.get_at(&t, 1_000 + FOUND_TTL.as_secs() + 1).is_none());
        c.forget(&t.key);
        assert!(c.get(&t).is_none());
        c.clear().unwrap();
    }

    #[test]
    fn changed_metadata_is_a_miss() {
        let c = Cache::in_memory();
        let mut t = track("local:/music/a.mp3");
        c.put(&t, &Lookup::Found(rec(), Match::Search));
        t.title = "Retagged".into();
        assert!(c.get(&t).is_none());
    }

    #[test]
    fn file_names_are_stable() {
        assert_eq!(file_name("spotify:track:abc"), file_name("spotify:track:abc"));
        assert_ne!(file_name("spotify:track:abc"), file_name("spotify:track:abd"));
    }
}
