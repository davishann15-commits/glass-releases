//! Local Files on iOS: the library as the app shows it (indexed exactly as the
//! desktop's, src/local_index.rs), its sources, Glass's local playlists, imports,
//! and the tag editor (src/local.rs: written on a copy that replaces the file).
//!
//! Sources on iOS: the app's Documents folder (where imports, saved songs and songs
//! from your other devices go; "On My iPhone › Glass" in Files), plus any folders you
//! add from Files (kept by the app as security-scoped bookmarks and handed to the
//! core as paths while it can read them).

use crate::app::{MobileApp, Msg};
use crate::bridge::{CoreError, CoreEvent};
use crate::local::LocalTrack;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LocalTrackModel {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<u16>,
    pub duration_ms: u64,
    pub track_no: u32,
    pub disc_no: u32,
    pub has_cover: bool,
    /// When the file came (Unix seconds), for Recently Added.
    pub added: u64,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LocalAlbumModel {
    pub title: String,
    pub artist: String,
    pub year: Option<u16>,
    pub genre: String,
    /// A track whose file has a cover (its path), if any.
    pub cover: Option<String>,
    pub added: u64,
    pub duration_ms: u64,
    /// Indices into `LocalModel::tracks`, disc by disc, in track order.
    pub tracks: Vec<u32>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LocalGroup {
    pub name: String,
    /// Indices into `LocalModel::albums`.
    pub albums: Vec<u32>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LocalPlaylistModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub paths: Vec<String>,
    /// Its cover image (a downloaded playlist's), as a file URL.
    pub cover: Option<String>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LocalModel {
    /// The folders have been read at least once.
    pub loaded: bool,
    pub scanning: bool,
    pub tracks: Vec<LocalTrackModel>,
    /// Sorted by artist (a leading "The" skipped), year, title.
    pub albums: Vec<LocalAlbumModel>,
    pub artists: Vec<LocalGroup>,
    /// "Unknown Genre" last.
    pub genres: Vec<LocalGroup>,
    /// Every song in artist → album → track order.
    pub songs: Vec<u32>,
    pub sources: Vec<String>,
    pub playlists: Vec<LocalPlaylistModel>,
    pub can_undo: bool,
    /// The app's own music folder (Documents).
    pub documents: String,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct TagsModel {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub has_cover: bool,
}

#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum CoverChange {
    Keep,
    Remove,
    /// A JPEG made by `prepare_cover`.
    Set { jpeg: Vec<u8> },
}

fn secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The fields of a song's tags, for the editor (desktop `local::read_tags`).
#[uniffi::export]
pub fn read_song_tags(path: String) -> Option<TagsModel> {
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return None;
    }
    let t = crate::local::rescan_file(&LocalTrack { path: p, ..Default::default() });
    let tags = crate::local::read_tags(&t);
    Some(TagsModel { title: tags.title, artist: tags.artist, album: tags.album, year: tags.year, genre: tags.genre, has_cover: t.has_cover })
}

/// An image (JPEG or PNG file) made into the cover Glass writes: at most 1000 px, JPEG.
#[uniffi::export]
pub fn prepare_cover(image_path: String) -> Result<Vec<u8>, CoreError> {
    crate::local::prepare_cover(std::path::Path::new(&image_path)).map_err(|e| {
        log::warn!("cover: {e}");
        CoreError::Failed { message: "That image can't be used.".into() }
    })
}

/// Desktop `local::validate`: a title, and a four-digit year if any.
#[uniffi::export]
pub fn validate_tags(tags: TagsModel) -> Option<String> {
    crate::local::validate(&to_tags(&tags)).err()
}

fn to_tags(t: &TagsModel) -> crate::local::Tags {
    crate::local::Tags { title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone(), year: t.year.clone(), genre: t.genre.clone() }
}

impl MobileApp {
    pub fn local_model(&self) -> LocalModel {
        let all = self.local_tracks.clone().unwrap_or_default();
        let idx = crate::local_index::index(&all);
        LocalModel {
            loaded: self.local_tracks.is_some(),
            scanning: self.local_scanning,
            tracks: all
                .iter()
                .map(|t| LocalTrackModel {
                    path: t.path.to_string_lossy().into_owned(),
                    title: t.title.clone(),
                    artist: t.artist.clone(),
                    album: t.album.clone(),
                    album_artist: t.album_artist.clone(),
                    genre: t.genre.clone(),
                    year: t.year,
                    duration_ms: t.duration_ms,
                    track_no: t.track_no,
                    disc_no: t.disc_no,
                    has_cover: t.has_cover,
                    added: secs(t.added),
                })
                .collect(),
            albums: idx
                .albums
                .iter()
                .map(|a| LocalAlbumModel {
                    title: a.title.clone(),
                    artist: a.artist.clone(),
                    year: a.year,
                    genre: a.genre.clone(),
                    cover: a.cover.map(|i| all[i].path.to_string_lossy().into_owned()),
                    added: secs(a.added),
                    duration_ms: a.duration_ms,
                    tracks: a.tracks.iter().map(|&i| i as u32).collect(),
                })
                .collect(),
            artists: idx.artists.iter().map(|(n, a)| LocalGroup { name: n.clone(), albums: a.iter().map(|&i| i as u32).collect() }).collect(),
            genres: idx.genres.iter().map(|(n, a)| LocalGroup { name: n.clone(), albums: a.iter().map(|&i| i as u32).collect() }).collect(),
            songs: idx.songs.iter().map(|&i| i as u32).collect(),
            sources: self.settings.local_dirs.iter().map(|d| d.to_string_lossy().into_owned()).collect(),
            playlists: self
                .playlists
                .list
                .iter()
                .map(|p| LocalPlaylistModel {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    description: p.description.clone(),
                    paths: p.tracks.iter().map(|t| t.to_string_lossy().into_owned()).collect(),
                    cover: p.cover.as_deref().filter(|c| c.exists()).and_then(|c| url::Url::from_file_path(c).ok()).map(|u| u.to_string()),
                })
                .collect(),
            can_undo: self.playlists.can_undo(),
            documents: crate::paths::documents().to_string_lossy().into_owned(),
        }
    }

    /// Sent when the library, its sources or the local playlists change (the index is
    /// rebuilt only then, not every tick).
    pub fn publish_local(&mut self, events: &mut Vec<CoreEvent>) {
        let sig = (
            self.local_tracks.as_ref().map(|t| Arc::as_ptr(t) as usize).unwrap_or(0),
            self.local_scanning,
            self.settings.local_dirs.clone(),
            self.playlists.list.iter().map(|p| (p.id.clone(), p.name.clone(), p.tracks.len(), p.updated)).collect::<Vec<_>>(),
            self.playlists.can_undo(),
        );
        if self.local_sig.as_ref() == Some(&sig) {
            return;
        }
        self.local_sig = Some(sig);
        events.push(CoreEvent::Local { local: self.local_model() });
    }

    /// Files the app has just put in Documents (imports) or a folder it can read now:
    /// read again, so they play straight away.
    pub fn files_added(&mut self, dirs: Vec<std::path::PathBuf>) {
        let new = crate::local::missing_sources(&dirs, &self.settings.local_dirs);
        if !new.is_empty() {
            self.settings.local_dirs.extend(new);
            self.settings.local_dirs = crate::local::tidy_sources(&self.settings.local_dirs);
            self.mark_settings_dirty();
        }
        self.apply(crate::app::Action::RescanLocal);
    }

    pub fn save_tags(&mut self, path: String, tags: TagsModel, cover: CoverChange) {
        let tags = to_tags(&tags);
        if let Err(e) = crate::local::validate(&tags) {
            self.outbox.push(CoreEvent::TagsSaved { path, error: Some(e) });
            return;
        }
        let cover = match cover {
            CoverChange::Keep => crate::local::Cover::Keep,
            CoverChange::Remove => crate::local::Cover::Remove,
            CoverChange::Set { jpeg } => crate::local::Cover::Set(Arc::new(jpeg)),
        };
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let p = std::path::PathBuf::from(&path);
            let r = crate::local::write_tags(&p, &tags, &cover);
            tx.send(Msg::TagsSaved(p, r));
        });
    }

    /// Desktop `Msg::TagsSaved`.
    pub fn tags_saved(&mut self, path: std::path::PathBuf, r: Result<(), String>) {
        match r {
            Ok(()) => {
                let refresh = |t: &mut LocalTrack| {
                    if t.path == path {
                        *t = crate::local::rescan_file(t);
                    }
                };
                if let Some(list) = self.local_tracks.as_mut() {
                    Arc::make_mut(list).iter_mut().for_each(refresh);
                }
                if let Some(l) = self.local.as_mut() {
                    Arc::make_mut(&mut l.queue).iter_mut().for_each(refresh);
                }
                self.clear_local_links();
                self.outbox.push(CoreEvent::TagsSaved { path: path.to_string_lossy().into_owned(), error: None });
            }
            Err(e) => {
                log::warn!("song info not saved: {e}");
                let why = if e.starts_with("Couldn't find the file") { "This file isn't there any more." } else { "Glass can't change this file." };
                self.outbox.push(CoreEvent::TagsSaved { path: path.to_string_lossy().into_owned(), error: Some(why.into()) })
            }
        }
    }
}
