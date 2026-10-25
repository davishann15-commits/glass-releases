//! Local Files filed the way iTunes / Music do: albums by album artist (else
//! artist) and title, tracks by disc and number, artists beside their albums,
//! genres, and the songs in artist → album → track order. Shared by the desktop
//! (src/ui/local_library.rs) and the iOS core.

use crate::local::LocalTrack;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::SystemTime;

pub struct Album {
    pub title: String,
    pub artist: String,
    pub year: Option<u16>,
    pub genre: String,
    /// Indices into the library, in track order.
    pub tracks: Vec<usize>,
    pub(crate) cover: Option<usize>,
    pub added: SystemTime,
    pub duration_ms: u64,
    /// See `album_key`.
    pub(crate) key: String,
}

impl Album {
    pub fn cover_uri(&self, all: &[LocalTrack]) -> Option<String> {
        self.cover.and_then(|i| all[i].cover_uri())
    }

    /// `q` is lowercase.
    pub fn matches(&self, q: &str) -> bool {
        q.is_empty() || self.key.contains(q)
    }
}

/// What tells albums apart: artist and title with case folded (any script's case,
/// so "Été" and "ÉTÉ" are one album wherever it's looked up from).
pub fn album_key(artist: &str, title: &str) -> String {
    format!("{}\u{1f}{}", artist.to_lowercase(), title.to_lowercase())
}

/// The key of the album the local player is playing from, if it's playing.
pub struct Index {
    /// By artist, then year, then title.
    pub albums: Vec<Album>,
    /// Each artist and their albums (indices into `albums`).
    pub artists: Vec<(String, Vec<usize>)>,
    pub genres: Vec<(String, Vec<usize>)>,
    /// Every song, artist by artist and album by album, in track order.
    pub songs: Vec<usize>,
}

/// Sorts like iTunes: case doesn't matter and a leading "The" is skipped.
pub fn sort_key(s: &str) -> String {
    let l = s.trim().to_lowercase();
    l.strip_prefix("the ").map(str::to_string).unwrap_or(l)
}

/// Who an album is filed under: its album artist, else its artist.
pub fn album_artist(t: &LocalTrack) -> &str {
    [t.album_artist.as_str(), t.artist.as_str()].into_iter().find(|s| !s.trim().is_empty()).map(str::trim).unwrap_or("Unknown Artist")
}

pub fn album_title(t: &LocalTrack) -> &str {
    let a = t.album.trim();
    if a.is_empty() { "Unknown Album" } else { a }
}

pub fn track_order(a: &LocalTrack, b: &LocalTrack) -> Ordering {
    let n = |t: &LocalTrack| if t.track_no == 0 { u32::MAX } else { t.track_no };
    a.disc_no.max(1).cmp(&b.disc_no.max(1)).then(n(a).cmp(&n(b))).then_with(|| sort_key(&a.title).cmp(&sort_key(&b.title)))
}

pub fn index(all: &[LocalTrack]) -> Index {
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut albums: Vec<Album> = Vec::new();
    for (i, t) in all.iter().enumerate() {
        let key = album_key(album_artist(t), album_title(t));
        let a = match by_key.get(&key) {
            Some(&a) => a,
            None => {
                by_key.insert(key.clone(), albums.len());
                albums.push(Album {
                    title: album_title(t).into(),
                    artist: album_artist(t).into(),
                    year: None,
                    genre: String::new(),
                    tracks: Vec::new(),
                    cover: None,
                    added: SystemTime::UNIX_EPOCH,
                    duration_ms: 0,
                    key,
                });
                albums.len() - 1
            }
        };
        let al = &mut albums[a];
        al.tracks.push(i);
        al.year = match (al.year, t.year) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if al.genre.is_empty() {
            al.genre = t.genre.trim().to_string();
        }
        if al.cover.is_none() && t.has_cover {
            al.cover = Some(i);
        }
        al.added = al.added.max(t.added);
        al.duration_ms += t.duration_ms;
    }
    for al in &mut albums {
        al.tracks.sort_by(|&x, &y| track_order(&all[x], &all[y]));
    }
    albums.sort_by(|a, b| sort_key(&a.artist).cmp(&sort_key(&b.artist)).then(a.year.cmp(&b.year)).then_with(|| sort_key(&a.title).cmp(&sort_key(&b.title))));
    let mut artists: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, al) in albums.iter().enumerate() {
        match artists.last_mut() {
            Some((name, list)) if name.eq_ignore_ascii_case(&al.artist) || name.to_lowercase() == al.artist.to_lowercase() => list.push(i),
            _ => artists.push((al.artist.clone(), vec![i])),
        }
    }
    let mut genres: Vec<(String, Vec<usize>)> = Vec::new();
    let mut g_index: HashMap<String, usize> = HashMap::new();
    for (i, al) in albums.iter().enumerate() {
        let name = if al.genre.is_empty() { "Unknown Genre" } else { al.genre.as_str() };
        let g = *g_index.entry(name.to_lowercase()).or_insert_with(|| {
            genres.push((name.to_string(), Vec::new()));
            genres.len() - 1
        });
        genres[g].1.push(i);
    }
    genres.sort_by(|a, b| (a.0 == "Unknown Genre").cmp(&(b.0 == "Unknown Genre")).then_with(|| sort_key(&a.0).cmp(&sort_key(&b.0))));
    let songs = albums.iter().flat_map(|a| a.tracks.iter().copied()).collect();
    Index { albums, artists, genres, songs }
}

