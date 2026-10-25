//! Playlists of local files, kept by Glass on this computer (not on
//! Spotify). The assistant can propose them; saving, replacing and deleting
//! always come from the user, and the last change can be undone.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalPlaylist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tracks: Vec<PathBuf>,
    pub created: u64,
    pub updated: u64,
    /// The assistant draft this was saved from, so a retried save can't make a second copy.
    #[serde(default)]
    pub from_draft: Option<String>,
    /// The Spotify playlist it was downloaded from (Share ▸ Download as …): downloading
    /// that again brings this one up to date instead of making another.
    #[serde(default)]
    pub source: Option<String>,
    /// Its cover, an image file (a downloaded playlist's, beside its songs).
    #[serde(default)]
    pub cover: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
enum Undo {
    Created(String),
    Replaced(LocalPlaylist),
    Deleted(LocalPlaylist, usize),
}

pub struct Playlists {
    path: Option<PathBuf>,
    pub list: Vec<LocalPlaylist>,
    undo: Vec<Undo>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Playlists {
    pub fn path() -> PathBuf {
        crate::config::config_dir().join("local_playlists.json")
    }

    pub fn load(path: PathBuf) -> Self {
        let list = crate::config::read_json(&path);
        Self { path: Some(path), list, undo: Vec::new() }
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self { path: None, list: Vec::new(), undo: Vec::new() }
    }

    /// Saves `list` and only then makes it the playlists (with `undo` to go back), so
    /// what's on screen never differs from what's on disk.
    fn commit(&mut self, list: Vec<LocalPlaylist>, undo: Option<Undo>) -> Result<(), String> {
        if let Some(path) = &self.path {
            let text = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
            crate::config::write_atomic(path, text.as_bytes()).map_err(|e| format!("couldn't save playlists: {e}"))?;
        }
        self.list = list;
        if let Some(u) = undo {
            self.undo.push(u);
            // Each step can hold a whole playlist; the last 50 are plenty.
            if self.undo.len() > 50 {
                self.undo.remove(0);
            }
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&LocalPlaylist> {
        self.list.iter().find(|p| p.id == id)
    }

    /// Creates a playlist. Saving the same draft again returns the playlist
    /// already made from it instead of a duplicate.
    pub fn create(&mut self, name: &str, description: &str, tracks: Vec<PathBuf>, from_draft: Option<&str>) -> Result<(String, bool), String> {
        if let Some(d) = from_draft
            && let Some(p) = self.list.iter().find(|p| p.from_draft.as_deref() == Some(d))
        {
            return Ok((p.id.clone(), false));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err("A playlist needs a name.".into());
        }
        let id = format!("lp_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let t = now();
        let mut list = self.list.clone();
        list.push(LocalPlaylist {
            id: id.clone(),
            name: name.chars().take(120).collect(),
            description: description.chars().take(500).collect(),
            tracks,
            created: t,
            updated: t,
            from_draft: from_draft.map(str::to_string),
            source: None,
            cover: None,
        });
        self.commit(list, Some(Undo::Created(id.clone())))?;
        Ok((id, true))
    }

    /// Rewrites every song's and cover's path (iOS: the app's folder moved; see
    /// `paths::rebase`), keeping the list on disk in step. Not undoable: nothing changes
    /// for the user.
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    pub fn map_paths(&mut self, f: impl Fn(&std::path::Path) -> PathBuf) {
        let mut list = self.list.clone();
        for p in &mut list {
            p.tracks = p.tracks.iter().map(|t| f(t)).collect();
            p.cover = p.cover.as_deref().map(&f);
        }
        if list != self.list
            && let Err(e) = self.commit(list, None)
        {
            log::warn!("local playlists' paths weren't updated: {e}");
        }
    }

    /// The local copy of a downloaded Spotify playlist (`source`, its URI): made the
    /// first time, then its songs (in the playlist's order), name, description and
    /// cover brought up to date by each later download. Undoable; returns its ID.
    pub fn from_download(&mut self, source: &str, name: &str, description: &str, tracks: Vec<PathBuf>, cover: Option<PathBuf>) -> Result<String, String> {
        let name: String = name.trim().chars().take(120).collect();
        if name.is_empty() {
            return Err("A playlist needs a name.".into());
        }
        let mut list = self.list.clone();
        let t = now();
        if let Some(p) = list.iter_mut().find(|p| p.source.as_deref() == Some(source)) {
            let old = p.clone();
            let id = p.id.clone();
            p.tracks = tracks;
            p.name = name;
            if !description.is_empty() {
                p.description = description.chars().take(500).collect();
            }
            if cover.is_some() {
                p.cover = cover;
            }
            if *p == old {
                return Ok(id);
            }
            p.updated = t;
            self.commit(list, Some(Undo::Replaced(old)))?;
            return Ok(id);
        }
        let id = format!("lp_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        list.push(LocalPlaylist { id: id.clone(), name, description: description.chars().take(500).collect(), tracks, created: t, updated: t, from_draft: None, source: Some(source.to_string()), cover });
        self.commit(list, Some(Undo::Created(id.clone())))?;
        Ok(id)
    }

    /// Replaces a playlist's songs (and optionally its name); undoable.
    pub fn replace(&mut self, id: &str, tracks: Vec<PathBuf>, name: Option<&str>, from_draft: Option<&str>) -> Result<(), String> {
        let mut list = self.list.clone();
        let p = list.iter_mut().find(|p| p.id == id).ok_or("That playlist doesn't exist any more.")?;
        if from_draft.is_some() && p.from_draft.as_deref() == from_draft && p.tracks == tracks {
            return Ok(());
        }
        let old = p.clone();
        p.tracks = tracks;
        if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
            p.name = n.chars().take(120).collect();
        }
        p.from_draft = from_draft.map(str::to_string).or(p.from_draft.take());
        p.updated = now();
        self.commit(list, Some(Undo::Replaced(old)))
    }

    /// Adds songs to the end of a playlist, skipping ones already in it;
    /// undoable. Returns how many were added.
    pub fn append(&mut self, id: &str, tracks: &[PathBuf]) -> Result<usize, String> {
        let p = self.list.iter().find(|p| p.id == id).ok_or("That playlist doesn't exist any more.")?;
        let mut all = p.tracks.clone();
        let before = all.len();
        for t in tracks {
            if !all.contains(t) {
                all.push(t.clone());
            }
        }
        let added = all.len() - before;
        if added > 0 {
            self.replace(id, all, None, None)?;
        }
        Ok(added)
    }

    /// Takes the song at `index` out of a playlist; undoable.
    pub fn remove_at(&mut self, id: &str, index: usize) -> Result<(), String> {
        let p = self.get(id).ok_or("That playlist doesn't exist any more.")?;
        if index >= p.tracks.len() {
            return Ok(());
        }
        let mut tracks = p.tracks.clone();
        tracks.remove(index);
        self.replace(id, tracks, None, None)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        let i = self.list.iter().position(|p| p.id == id).ok_or("That playlist doesn't exist any more.")?;
        let mut list = self.list.clone();
        let p = list.remove(i);
        self.commit(list, Some(Undo::Deleted(p, i)))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether the last change was deleting playlist `id` (so undo brings it back).
    pub fn just_deleted(&self, id: &str) -> bool {
        matches!(self.undo.last(), Some(Undo::Deleted(p, _)) if p.id == id)
    }

    /// Reverts the last change. Returns what was undone.
    pub fn undo(&mut self) -> Result<Option<String>, String> {
        let Some(u) = self.undo.last().cloned() else { return Ok(None) };
        let mut list = self.list.clone();
        let what = match u {
            Undo::Created(id) => {
                let name = self.get(&id).map(|p| p.name.clone()).unwrap_or_default();
                list.retain(|p| p.id != id);
                format!("Removed “{name}”")
            }
            Undo::Replaced(old) => {
                let name = old.name.clone();
                if let Some(p) = list.iter_mut().find(|p| p.id == old.id) {
                    *p = old;
                }
                format!("Restored “{name}”")
            }
            Undo::Deleted(p, i) => {
                let name = p.name.clone();
                list.insert(i.min(list.len()), p);
                format!("Brought back “{name}”")
            }
        };
        self.commit(list, None)?;
        self.undo.pop();
        Ok(Some(what))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retried_saves_dont_duplicate() {
        let mut p = Playlists::in_memory();
        let (a, new) = p.create("Night Drive", "", vec!["/m/a.mp3".into()], Some("d1")).unwrap();
        assert!(new);
        let (b, new) = p.create("Night Drive", "", vec!["/m/a.mp3".into()], Some("d1")).unwrap();
        assert_eq!((a, new, p.list.len()), (b, false, 1));
    }

    #[test]
    fn a_downloaded_playlist_is_kept_up_to_date() {
        let mut p = Playlists::in_memory();
        let id = p.from_download("spotify:playlist:x", "Night Drive", "Slow songs", vec!["/m/a.m4a".into(), "/m/b.m4a".into()], Some("/m/Night Drive/cover.jpg".into())).unwrap();
        // Downloaded again, renamed and with a song more: the same playlist, in the new order.
        let again = p.from_download("spotify:playlist:x", "Night Drive II", "", vec!["/m/c.m4a".into(), "/m/a.m4a".into(), "/m/b.m4a".into()], None).unwrap();
        assert_eq!((id.clone(), p.list.len()), (again, 1));
        let pl = p.get(&id).unwrap();
        assert_eq!((pl.name.as_str(), pl.description.as_str(), pl.tracks.len()), ("Night Drive II", "Slow songs", 3));
        assert_eq!(pl.tracks[0], PathBuf::from("/m/c.m4a"));
        assert_eq!(pl.cover.as_deref(), Some(std::path::Path::new("/m/Night Drive/cover.jpg")), "kept when the new download has none");
        p.undo().unwrap();
        assert_eq!(p.get(&id).unwrap().tracks.len(), 2, "undo puts the earlier download back");
        p.from_download("spotify:playlist:y", "Other", "", vec![], None).unwrap();
        assert_eq!(p.list.len(), 2, "another playlist is another copy");
    }

    #[test]
    fn replace_delete_and_undo() {
        let mut p = Playlists::in_memory();
        let (id, _) = p.create("Mix", "", vec!["/m/a.mp3".into()], None).unwrap();
        p.replace(&id, vec!["/m/b.mp3".into(), "/m/c.mp3".into()], Some("Calmer Mix"), Some("d2")).unwrap();
        assert_eq!(p.get(&id).unwrap().name, "Calmer Mix");
        assert_eq!(p.undo().unwrap().unwrap(), "Restored “Mix”");
        assert_eq!(p.get(&id).unwrap().tracks, vec![PathBuf::from("/m/a.mp3")]);
        p.delete(&id).unwrap();
        assert!(p.get(&id).is_none());
        p.undo().unwrap();
        assert!(p.get(&id).is_some(), "deletes can be undone");
        p.undo().unwrap();
        assert!(p.list.is_empty(), "and so can the creation");
        assert_eq!(p.undo().unwrap(), None);
        assert!(p.create("  ", "", vec![], None).is_err());
    }

    #[test]
    fn add_and_remove_songs() {
        let mut p = Playlists::in_memory();
        let (id, _) = p.create("Mix", "", vec!["/m/a.mp3".into()], None).unwrap();
        assert_eq!(p.append(&id, &["/m/b.mp3".into(), "/m/a.mp3".into()]).unwrap(), 1, "a song already there is skipped");
        assert_eq!(p.append(&id, &["/m/a.mp3".into()]).unwrap(), 0);
        p.remove_at(&id, 0).unwrap();
        assert_eq!(p.get(&id).unwrap().tracks, vec![PathBuf::from("/m/b.mp3")]);
        p.undo().unwrap();
        assert_eq!(p.get(&id).unwrap().tracks.len(), 2, "removing can be undone");
        p.undo().unwrap();
        assert_eq!(p.get(&id).unwrap().tracks, vec![PathBuf::from("/m/a.mp3")], "and so can adding");
        assert!(p.append("nope", &["/m/a.mp3".into()]).is_err());
    }

    #[test]
    fn saved_to_disk() {
        let path = crate::config::test_dir("playlists.json");
        let _ = std::fs::remove_file(&path);
        let mut p = Playlists::load(path.clone());
        p.create("Disk", "d", vec!["/m/x.flac".into()], None).unwrap();
        assert_eq!(Playlists::load(path.clone()).list[0].name, "Disk");
        let _ = std::fs::remove_file(path);
    }
}
