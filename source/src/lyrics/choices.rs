//! The user's own lyrics decisions, kept for good in Glass's data folder
//! (never in the cache, which can be cleared): the lyrics they chose for a
//! song, or "none of these", and the song's timing offset. Found by the
//! song's key first, then by title, version markers, main artist and length,
//! so a choice follows the song when it plays from another album, playlist or
//! release, but not to its instrumental, live take or remix.

use super::TrackRef;
use super::cache::StoredRecord;
use super::matching::{normalize, title as split_title};
use super::parse::Record;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Picked {
    Lyrics { record: StoredRecord },
    None,
}

#[derive(Clone, Serialize, Deserialize)]
struct Choice {
    key: String,
    /// Base title and main artist, normalized, then the title's version
    /// markers if it has any.
    song: String,
    secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pick: Option<Picked>,
    #[serde(default, skip_serializing_if = "is_zero")]
    offset_ms: i64,
    /// Line by line: LRCLIB's lyrics (or the pick as it is), no word timing.
    #[serde(default, skip_serializing_if = "is_false")]
    lines_only: bool,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn song(t: &TrackRef) -> String {
    let title = split_title(&t.title);
    let song = format!("{}\u{1f}{}", title.base, normalize(t.artist()));
    // Without markers the key is what it always was, so older choices still match.
    if title.versions.is_empty() { song } else { format!("{song}\u{1f}{:?}", title.versions) }
}

pub struct Choices {
    path: PathBuf,
    list: Mutex<Vec<Choice>>,
    /// See `save`.
    writer: Arc<Mutex<Writer>>,
}

/// The newest file contents not yet written, and whether a writer thread is at it.
#[derive(Default)]
struct Writer {
    next: Option<Vec<u8>>,
    running: bool,
}

impl Choices {
    pub fn path() -> PathBuf {
        crate::config::config_dir().join("lyrics_choices.json")
    }

    pub fn load(path: PathBuf) -> Self {
        let list = crate::config::read_json(&path);
        Self { path, list: Mutex::new(list), writer: Arc::default() }
    }

    fn find(list: &[Choice], t: &TrackRef) -> Option<usize> {
        if list.is_empty() {
            return None;
        }
        list.iter().position(|c| c.key == t.key).or_else(|| {
            let (s, secs) = (song(t), t.duration_ms / 1000);
            list.iter().position(|c| c.song == s && c.secs.abs_diff(secs) <= 2)
        })
    }

    /// Changes this song's entry (made if missing), drops it once it holds
    /// nothing, and saves.
    fn edit(&self, t: &TrackRef, f: impl FnOnce(&mut Choice)) {
        let mut list = self.list.lock().unwrap();
        let i = match Self::find(&list, t) {
            Some(i) => i,
            None => {
                list.push(Choice { key: t.key.clone(), song: song(t), secs: t.duration_ms / 1000, pick: None, offset_ms: 0, lines_only: false });
                list.len() - 1
            }
        };
        f(&mut list[i]);
        if list[i].pick.is_none() && list[i].offset_ms == 0 && !list[i].lines_only {
            list.remove(i);
        }
        match serde_json::to_vec(&*list) {
            Ok(bytes) => self.save(bytes),
            Err(e) => log::warn!("lyrics choices: {e}"),
        }
    }

    /// Writes the file on a thread of its own (a nudge of the timing is a keypress,
    /// and the file holds the lyrics of every pick). Only the newest contents are
    /// written, and one write at a time, so an older one never lands last.
    fn save(&self, bytes: Vec<u8>) {
        let mut w = self.writer.lock().unwrap();
        w.next = Some(bytes);
        if w.running {
            return;
        }
        w.running = true;
        let (writer, path) = (self.writer.clone(), self.path.clone());
        std::thread::spawn(move || {
            loop {
                // Taking the last contents and stopping happen under one lock, so an
                // edit either lands in this loop or starts a writer of its own.
                let bytes = {
                    let mut w = writer.lock().unwrap();
                    match w.next.take() {
                        Some(b) => b,
                        None => {
                            w.running = false;
                            return;
                        }
                    }
                };
                let write = || -> std::io::Result<()> {
                    if let Some(d) = path.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    let tmp = path.with_extension("json.tmp");
                    std::fs::write(&tmp, &bytes)?;
                    std::fs::rename(&tmp, &path)
                };
                if let Err(e) = write() {
                    log::warn!("lyrics choices: {e}");
                }
            }
        });
    }

    /// Waits for the file to be written (at quit, and in tests that read it back).
    pub fn flush(&self) {
        while self.writer.lock().unwrap().running {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// What the user chose for this song: `Some(Some(lyrics))`, `Some(None)`
    /// for "none of these", or `None` if they haven't chosen.
    pub fn pick(&self, t: &TrackRef) -> Option<Option<Record>> {
        let list = self.list.lock().unwrap();
        match &list[Self::find(&list, t)?].pick {
            Some(Picked::Lyrics { record }) => Some(Some(record.clone().into())),
            Some(Picked::None) => Some(None),
            None => None,
        }
    }

    pub fn set_pick(&self, t: &TrackRef, record: Option<&Record>) {
        let p = match record {
            Some(r) => Picked::Lyrics { record: r.into() },
            None => Picked::None,
        };
        self.edit(t, |c| c.pick = Some(p));
    }

    /// "Look again": the song goes back to automatic matching.
    pub fn forget_pick(&self, t: &TrackRef) {
        self.edit(t, |c| c.pick = None);
    }

    /// This song's timing offset (ms; positive shows lyrics earlier).
    pub fn offset(&self, t: &TrackRef) -> i64 {
        let list = self.list.lock().unwrap();
        Self::find(&list, t).map(|i| list[i].offset_ms).unwrap_or(0)
    }

    pub fn set_offset(&self, t: &TrackRef, ms: i64) {
        self.edit(t, |c| c.offset_ms = ms);
    }

    /// The user would rather have this song's lyrics line by line (LRCLIB's, or
    /// their pick as it is) than word timing from the other services.
    pub fn lines_only(&self, t: &TrackRef) -> bool {
        let list = self.list.lock().unwrap();
        Self::find(&list, t).is_some_and(|i| list[i].lines_only)
    }

    pub fn set_lines_only(&self, t: &TrackRef, on: bool) {
        self.edit(t, |c| c.lines_only = on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str, title: &str, secs: u64) -> TrackRef {
        TrackRef { key: key.into(), title: title.into(), artists: vec!["Anri".into()], album: "Timely!!".into(), duration_ms: secs * 1000, ..Default::default() }
    }

    fn record(id: u64) -> Record {
        crate::lyrics::parse::record(&serde_json::json!({ "id": id, "trackName": "Cat's Eye", "artistName": "Anri", "duration": 230, "syncedLyrics": "[00:01.00] Hi" })).unwrap()
    }

    #[test]
    fn picks_and_offsets_stick_to_the_song() {
        let path = crate::config::test_dir("choices.json");
        let _ = std::fs::remove_file(&path);
        let c = Choices::load(path.clone());
        let album = track("spotify:track:a", "Cat's Eye", 230);
        c.set_pick(&album, Some(&record(7)));
        c.set_offset(&album, 400);
        c.flush();
        // Kept on disk, for the same song from another release.
        let again = Choices::load(path.clone());
        let single = track("spotify:track:b", "Cat's Eye - 2018 Remaster", 231);
        assert_eq!(again.pick(&single).flatten().map(|r| r.id), Some(7));
        assert_eq!(again.offset(&single), 400);
        assert_eq!(again.offset(&track("spotify:track:c", "Windy Summer", 270)), 0, "other songs keep their own timing");
        assert!(again.pick(&track("spotify:track:d", "Cat's Eye", 300)).is_none(), "a much longer recording isn't the same one");
        // "None of these" is a choice too; "look again" clears it; empty entries go.
        again.set_pick(&album, None);
        assert_eq!(again.pick(&album), Some(None));
        again.forget_pick(&album);
        again.set_offset(&album, 0);
        assert!(again.pick(&album).is_none());
        assert!(again.list.lock().unwrap().is_empty());
        // Line by line instead of word timing is kept too, until it's turned off.
        again.set_lines_only(&album, true);
        assert!(again.lines_only(&single) && !again.lines_only(&track("spotify:track:c", "Windy Summer", 270)));
        again.set_lines_only(&album, false);
        assert!(again.list.lock().unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn another_version_keeps_its_own_choices() {
        let path = crate::config::test_dir("choices-versions.json");
        let _ = std::fs::remove_file(&path);
        let c = Choices::load(path.clone());
        c.set_offset(&track("spotify:track:a", "Cat's Eye", 230), 400);
        let karaoke = track("spotify:track:k", "Cat's Eye - Karaoke Version", 230);
        assert_eq!(c.offset(&karaoke), 0, "the same length, but not the same recording");
        c.set_offset(&karaoke, -300);
        assert_eq!(c.offset(&track("spotify:track:k2", "Cat's Eye (Karaoke Version)", 231)), -300, "but it follows its own releases");
        assert_eq!(c.offset(&track("spotify:track:a2", "Cat's Eye - 2018 Remaster", 230)), 400);
        c.flush();
        let _ = std::fs::remove_file(path);
    }
}
