// Glass patch (see GLASS.md): upstream only plays a playlist's local song
// when a file produces the exact same `spotify:local:` URI — identical
// percent-encoding of artist, album and title, and the same length to the
// second — and skips MP4/M4A and files whose length isn't in the header.
// Files copied from a phone often miss on one of those. This version
// matches the way people would: the same title and artist (album optional),
// lengths within a few seconds, any common format, and it rescans the folders
// when a song isn't found (so newly copied files play without a restart).

use crate::symphonia_util;
use librespot_core::SpotifyUri;
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
    time::{Duration, Instant},
};
use symphonia::core::{
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey, Tag},
    probe::{Hint, ProbeResult},
};

const SUPPORTED_FILE_EXTENSIONS: &[&str] = &["mp3", "mp4", "m4a", "m4p", "aac", "flac", "ogg", "oga", "wav", "aif", "aiff"];

/// Seconds two lengths may differ by and still be the same recording.
const LENGTH_SLACK: u64 = 3;

/// Folders scanned on a miss at most this often.
const RESCAN_AFTER: Duration = Duration::from_secs(10);

/// The folders to look in. Set from the player config at start and, by the
/// app, whenever the user changes them.
static DIRECTORIES: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Replaces the folders local songs are looked for in. Takes effect on the
/// next lookup.
pub fn set_directories(dirs: Vec<PathBuf>) {
    *DIRECTORIES.write().unwrap_or_else(|e| e.into_inner()) = dirs;
}

struct Entry {
    path: PathBuf,
    title: String,
    artist: String,
    album: String,
    secs: Option<u64>,
    /// The file's size and modification time when it was read (Glass: a rescan
    /// reads only files that are new or changed).
    stamp: (u64, Option<std::time::SystemTime>),
}

#[derive(Default)]
struct Scan {
    dirs: Vec<PathBuf>,
    entries: Vec<Entry>,
    at: Option<Instant>,
}

#[derive(Default)]
pub struct LocalFileLookup(Mutex<Scan>);

impl LocalFileLookup {
    /// The file for a `spotify:local:` URI, if there's one that fits.
    pub fn get(&self, uri: &SpotifyUri) -> Option<PathBuf> {
        let SpotifyUri::Local { artist, album_title, track_title, duration } = uri else {
            return None;
        };
        let want = Wanted { title: normalize(&decode(track_title)), artist: normalize(&decode(artist)), album: normalize(&decode(album_title)), secs: duration.as_secs() };
        let mut scan = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let dirs = DIRECTORIES.read().unwrap_or_else(|e| e.into_inner()).clone();
        if scan.at.is_none() || scan.dirs != dirs {
            rescan(&mut scan, dirs.clone());
        }
        if let Some(p) = best(&scan.entries, &want) {
            return Some(p);
        }
        if scan.at.is_some_and(|t| t.elapsed() >= RESCAN_AFTER) {
            rescan(&mut scan, dirs);
            return best(&scan.entries, &want);
        }
        None
    }
}

/// The lookup starts empty and scans on first use, so startup isn't slowed.
pub fn create_local_file_lookup(directories: &[PathBuf]) -> LocalFileLookup {
    if !directories.is_empty() {
        set_directories(directories.to_vec());
    }
    LocalFileLookup::default()
}

struct Wanted {
    title: String,
    artist: String,
    album: String,
    secs: u64,
}

/// The best file for `want`: same title, the same (or an overlapping)
/// artist, a length within a few seconds; the album breaks ties.
fn best(entries: &[Entry], want: &Wanted) -> Option<PathBuf> {
    entries
        .iter()
        .filter_map(|e| {
            if e.title.is_empty() || e.title != want.title {
                return None;
            }
            if !want.artist.is_empty() && !e.artist.is_empty() && !artists_overlap(&e.artist, &want.artist) {
                return None;
            }
            let apart = match e.secs {
                Some(s) if want.secs > 0 => {
                    let d = s.abs_diff(want.secs);
                    if d > LENGTH_SLACK {
                        return None;
                    }
                    d
                }
                _ => LENGTH_SLACK,
            };
            let album = !want.album.is_empty() && e.album == want.album;
            Some(((!album) as u64 * 100 + apart, e))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, e)| e.path.clone())
}

/// One credit holds the other as whole words: "Artist" is in "Artist feat. Other"
/// and "Other, Artist", but "Ali" isn't "Alicia Keys". (Both are normalized: words
/// separated by single spaces.)
fn artists_overlap(a: &str, b: &str) -> bool {
    let within = |short: &str, long: &str| !short.is_empty() && format!(" {long} ").contains(&format!(" {short} "));
    a == b || within(a, b) || within(b, a)
}

/// `+` and `%XX` decoded, as in Spotify's local URIs.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lowercase letters and digits from any script, single-spaced.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else if c != '\'' && c != '’' {
            space = true;
        }
    }
    out
}

fn rescan(scan: &mut Scan, dirs: Vec<PathBuf>) {
    // What's already been read is kept for files that haven't changed since.
    let mut known: std::collections::HashMap<PathBuf, Entry> = std::mem::take(&mut scan.entries).into_iter().map(|e| (e.path.clone(), e)).collect();
    let mut entries = Vec::new();
    for d in &dirs {
        if d.is_dir() {
            visit_dir(d, 0, &mut known, &mut entries);
        } else {
            warn!("Ignoring local file source {}: not a directory", d.display());
        }
    }
    debug!("Local files: {} playable files in {} folders", entries.len(), dirs.len());
    *scan = Scan { dirs, entries, at: Some(Instant::now()) };
}

fn visit_dir(dir: &Path, depth: usize, known: &mut std::collections::HashMap<PathBuf, Entry>, out: &mut Vec<Entry>) {
    if depth > 8 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) || path.is_dir() {
            visit_dir(&path, depth + 1, known, out);
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase) else { continue };
        if !SUPPORTED_FILE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let stamp = entry.metadata().map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
        if let Some(e) = known.remove(&path).filter(|e| e.stamp == stamp && stamp.1.is_some()) {
            out.push(e);
            continue;
        }
        match read_entry(&path, &ext) {
            Some(e) => out.push(Entry { stamp, ..e }),
            None => warn!("Couldn't read local file {}", path.display()),
        }
    }
}

fn read_entry(path: &Path, ext: &str) -> Option<Entry> {
    let src = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let mut probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;

    let (mut artist, mut album, mut title) = (String::new(), String::new(), String::new());
    fn get_tags(probed: &mut ProbeResult) -> Option<Vec<Tag>> {
        let metadata = symphonia_util::get_latest_metadata(probed)?;
        let rev = metadata.current()?;
        Some(rev.tags().to_vec())
    }
    for tag in get_tags(&mut probed).unwrap_or_default() {
        match tag.std_key {
            Some(StandardTagKey::Album) => album = tag.value.to_string(),
            Some(StandardTagKey::Artist) => artist = tag.value.to_string(),
            Some(StandardTagKey::TrackTitle) => title = tag.value.to_string(),
            _ => {}
        }
    }
    // Untagged files are known by their name, as Spotify does.
    if title.trim().is_empty() {
        title = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    }
    let secs = probed.format.default_track().and_then(|t| Some(t.codec_params.time_base?.calc_time(t.codec_params.n_frames?).seconds));
    Some(Entry { path: path.to_path_buf(), title: normalize(&title), artist: normalize(&artist), album: normalize(&album), secs, stamp: (0, None) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, artist: &str, album: &str, secs: Option<u64>, path: &str) -> Entry {
        Entry { path: path.into(), title: normalize(title), artist: normalize(artist), album: normalize(album), secs, stamp: (0, None) }
    }

    fn want(uri: &str) -> Wanted {
        let parts: Vec<&str> = uri.split(':').skip(2).collect();
        Wanted { artist: normalize(&decode(parts[0])), album: normalize(&decode(parts[1])), title: normalize(&decode(parts[2])), secs: parts[3].parse().unwrap() }
    }

    #[test]
    fn decoding() {
        assert_eq!(decode("Simon+%26+Garfunkel"), "Simon & Garfunkel");
        assert_eq!(decode("Caf%C3%A9+del+Mar"), "Café del Mar");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%"), "%");
    }

    #[test]
    fn matches_what_a_person_would() {
        let files = vec![
            entry("Bloom", "The Paper Kites", "Woodland", Some(212), "/m/bloom.m4a"),
            entry("Bloom (Bonus)", "The Paper Kites", "Woodland", Some(212), "/m/bonus.m4a"),
            entry("Golden Hour", "", "", None, "/m/golden.mp3"),
            entry("Kyoto", "Phoebe Bridgers", "Punisher", Some(184), "/m/kyoto-album.mp3"),
            entry("Kyoto", "Phoebe Bridgers", "Kyoto - Single", Some(184), "/m/kyoto-single.mp3"),
        ];
        // Lengths a second apart and different escaping still match.
        assert_eq!(best(&files, &want("spotify:local:The+Paper+Kites:Woodland:Bloom:213")), Some("/m/bloom.m4a".into()));
        // No album in the playlist entry.
        assert_eq!(best(&files, &want("spotify:local:The+Paper+Kites::Bloom:211")), Some("/m/bloom.m4a".into()));
        // Untagged artist and unknown length: the title decides.
        assert_eq!(best(&files, &want("spotify:local:Someone:Something:Golden+Hour:200")), Some("/m/golden.mp3".into()));
        // The album breaks a tie.
        assert_eq!(best(&files, &want("spotify:local:Phoebe+Bridgers:Punisher:Kyoto:184")), Some("/m/kyoto-album.mp3".into()));
        // A different recording doesn't.
        assert_eq!(best(&files, &want("spotify:local:The+Paper+Kites:Woodland:Bloom:240")), None, "too far apart in length");
        assert_eq!(best(&files, &want("spotify:local:Another+Band:Woodland:Bloom:212")), None, "another artist");
        assert_eq!(best(&files, &want("spotify:local:The+Paper+Kites:Woodland:Bloom+Again:212")), None, "another title");
        // "feat." credits overlap.
        let feat = vec![entry("Levitating", "Dua Lipa feat. DaBaby", "", Some(203), "/m/lev.mp3")];
        assert_eq!(best(&feat, &want("spotify:local:Dua+Lipa::Levitating:203")), Some("/m/lev.mp3".into()));
    }
}
