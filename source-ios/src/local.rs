//! Local files: scan the folders listed in Settings, play them with rodio,
//! find the ones a Spotify playlist refers to, and edit their tags.
//! These files leave this computer only when sharing is on, and then only for
//! your own paired computers (see `share`).

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{Tag, items::Timestamp};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// What Local Files plays (not .m4p: those are DRM-protected and never decode).
pub const EXTENSIONS: &[&str] = &["mp3", "m4a", "mp4", "flac", "ogg", "oga", "wav", "aac", "aif", "aiff"];

#[derive(Clone, Debug)]
pub struct LocalTrack {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub added: SystemTime,
    pub year: Option<u16>,
    pub genre: String,
    pub has_cover: bool,
    /// The album's artist when tagged (compilations), else empty.
    pub album_artist: String,
    /// Track and disc numbers, 0 when untagged.
    pub track_no: u32,
    pub disc_no: u32,
}

impl Default for LocalTrack {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            added: SystemTime::UNIX_EPOCH,
            year: None,
            genre: String::new(),
            has_cover: false,
            album_artist: String::new(),
            track_no: 0,
            disc_no: 0,
        }
    }
}

impl LocalTrack {
    /// The embedded cover, for egui (see `CoverLoader`).
    pub fn cover_uri(&self) -> Option<String> {
        self.has_cover.then(|| format!("{COVER_SCHEME}{}", self.path.display()))
    }
}

/// Every song in `dirs`, each file once (folders inside other listed
/// folders are read once, as part of the outer one).
pub fn scan(dirs: &[PathBuf]) -> Vec<LocalTrack> {
    let mut out = Vec::new();
    for d in tidy_sources(dirs) {
        walk(&d, 0, &mut out);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.path.canonicalize().unwrap_or_else(|_| t.path.clone())));
    out.sort_by_key(|t| std::cmp::Reverse(t.added));
    out
}

/// The folders as a source list should hold them: each once, and none that
/// sits inside another within `DEPTH` folders (it's read as part of that one).
pub fn tidy_sources(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let real = |d: &PathBuf| d.canonicalize().unwrap_or_else(|_| d.components().collect());
    let all: Vec<(PathBuf, PathBuf)> = dirs.iter().map(|d| (d.clone(), real(d))).collect();
    // Outer folders first, so each is checked against the ones kept (a folder
    // inside a dropped one may be too deep for the folder that dropped it).
    let mut order: Vec<usize> = (0..all.len()).collect();
    order.sort_by_key(|&i| all[i].1.components().count());
    let mut keep = vec![false; all.len()];
    for &i in &order {
        keep[i] = !order.iter().any(|&j| keep[j] && covers(&all[j].1, &all[i].1));
    }
    all.into_iter().zip(keep).filter_map(|((d, _), k)| k.then_some(d)).collect()
}

/// Adds a folder to a source list. False when it's already covered.
pub fn add_source(dirs: &mut Vec<PathBuf>, dir: PathBuf) -> bool {
    let before = tidy_sources(dirs);
    let mut with = dirs.clone();
    with.push(dir);
    let after = tidy_sources(&with);
    if after == before {
        return false;
    }
    *dirs = after;
    true
}

/// How many folders down a source is read.
const DEPTH: usize = 5;

/// Whether a scan of `source` reads folder `dir`: it's inside, at most `DEPTH` down.
fn covers(source: &Path, dir: &Path) -> bool {
    dir.starts_with(source) && dir.components().count() - source.components().count() <= DEPTH
}

/// Whether a scan of `sources` reads folder `dir`.
pub fn read_by(sources: &[PathBuf], dir: &Path) -> bool {
    sources.iter().any(|s| covers(s, dir))
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<LocalTrack>) {
    if depth > DEPTH || out.len() > 20_000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) {
            continue;
        }
        if path.is_dir() {
            walk(&path, depth + 1, out);
            continue;
        }
        if is_audio(&path) && !in_cloud(&entry) {
            out.push(read_track(&path));
        }
    }
}

/// A song in a folder from iCloud Drive (or another cloud in Files) that isn't on the
/// iPhone yet: left until the app has downloaded it (reading it here would download
/// the songs one at a time while the scan waits).
#[cfg(target_os = "ios")]
fn in_cloud(entry: &std::fs::DirEntry) -> bool {
    use std::os::ios::fs::MetadataExt;
    const SF_DATALESS: u32 = 0x4000_0000;
    entry.metadata().is_ok_and(|m| m.st_flags() & SF_DATALESS != 0)
}

#[cfg(not(target_os = "ios"))]
fn in_cloud(_: &std::fs::DirEntry) -> bool {
    false
}

/// A file Glass can play, by its extension.
pub fn is_audio(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    EXTENSIONS.contains(&ext.as_str())
}

fn read_track(path: &Path) -> LocalTrack {
    let added = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string();
    let mut t = LocalTrack { path: path.to_path_buf(), title: stem, added, ..Default::default() };
    if let Some(tagged) = open(path) {
        t.duration_ms = tagged.properties().duration().as_millis() as u64;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            fill(&mut t, tag);
        }
    }
    t
}

/// The songs in files and folders picked or dropped by the user, in the
/// order given (a folder's songs by name).
pub fn read_paths(paths: &[PathBuf]) -> Vec<LocalTrack> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut inner = Vec::new();
            walk(p, 0, &mut inner);
            inner.sort_by(|a, b| a.path.cmp(&b.path));
            out.extend(inner);
        } else if is_audio(p) {
            out.push(read_track(p));
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.path.clone()));
    out
}

/// Whether `p` is where macOS keeps a file another app hands over and later clears:
/// a screenshot's thumbnail (`TemporaryItems`), or a file dragged out of Mail,
/// Messages and the like (a `com.apple.…` folder in the temporary folder).
pub fn is_temporary(p: &Path) -> bool {
    if p.components().any(|c| c.as_os_str() == "TemporaryItems") {
        return true;
    }
    let tmp = std::env::temp_dir();
    let tmp = tmp.canonicalize().unwrap_or(tmp);
    let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    p.strip_prefix(&tmp).ok().and_then(|rest| rest.components().next()).is_some_and(|c| c.as_os_str().to_string_lossy().starts_with("com.apple."))
}

/// Audio files from temporary folders, copied into `into` (they'd vanish, and their
/// folder mustn't become a source); other paths as they are.
pub fn keep_temporary(paths: Vec<PathBuf>, into: &Path) -> Vec<PathBuf> {
    paths
        .into_iter()
        .map(|p| {
            if !(p.is_file() && is_audio(&p) && is_temporary(&p)) {
                return p;
            }
            let Some(name) = p.file_name() else { return p };
            let mut dest = into.join(name);
            let (stem, ext) = (dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default());
            for i in 2..100 {
                if !dest.exists() {
                    break;
                }
                dest = into.join(format!("{stem} ({i}).{ext}"));
            }
            match std::fs::create_dir_all(into).and_then(|_| std::fs::copy(&p, &dest)) {
                Ok(_) => dest,
                Err(e) => {
                    log::warn!("couldn't keep {}: {e}", p.display());
                    p
                }
            }
        })
        .collect()
}

/// The folders to add as Local Files sources so that `paths` play: each
/// file's folder, or the folder itself, unless a source already covers it.
/// Temporary folders never are (see `keep_temporary`).
pub fn missing_sources(paths: &[PathBuf], sources: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        let dir = if p.is_dir() { p.clone() } else { p.parent().map(Path::to_path_buf).unwrap_or_default() };
        if dir.as_os_str().is_empty() || is_temporary(&dir) || sources.iter().chain(&out).any(|s| covers(s, &dir)) {
            continue;
        }
        out.retain(|o| !covers(&dir, o));
        out.push(dir);
    }
    out
}

fn open(path: &Path) -> Option<lofty::file::TaggedFile> {
    lofty::probe::Probe::open(path).ok()?.guess_file_type().ok()?.read().ok()
}

fn fill(t: &mut LocalTrack, tag: &Tag) {
    if let Some(v) = tag.title().filter(|v| !v.trim().is_empty()) {
        t.title = v.to_string();
    }
    t.artist = tag.artist().map(|a| a.to_string()).unwrap_or_default();
    t.album = tag.album().map(|a| a.to_string()).unwrap_or_default();
    t.genre = tag.genre().map(|a| a.to_string()).unwrap_or_default();
    t.year = tag.date().map(|d| d.year).filter(|y| *y > 0);
    t.has_cover = !tag.pictures().is_empty();
    t.album_artist = tag.get_string(lofty::tag::ItemKey::AlbumArtist).map(|a| a.trim().to_string()).unwrap_or_default();
    t.track_no = tag.track().unwrap_or(0);
    t.disc_no = tag.disk().unwrap_or(0);
}

/// Reads one file again (after its tags were edited).
pub fn rescan_file(t: &LocalTrack) -> LocalTrack {
    let mut out = t.clone();
    if let Some(tagged) = open(&t.path) {
        out.duration_ms = tagged.properties().duration().as_millis() as u64;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            fill(&mut out, tag);
        } else {
            out.has_cover = false;
        }
    }
    out
}

// ----------------------------------------------------------- playlist links

/// The `spotify:local:artist:album:title:seconds` entry for a file, as
/// Spotify's own apps write it (each part URL-encoded, `+` for spaces), so
/// it can go into a Spotify playlist. Like Spotify, the playlist keeps only
/// this description; each computer plays its own copy of the file.
pub fn spotify_uri(t: &LocalTrack) -> String {
    let enc = |s: &str| {
        let mut out = String::with_capacity(s.len());
        for b in s.trim().bytes() {
            match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'*' => out.push(b as char),
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    };
    format!("spotify:local:{}:{}:{}:{}", enc(&t.artist), enc(&t.album), enc(&t.title), (t.duration_ms + 500) / 1000)
}

/// Shows a file in the system's file manager.
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = open::that(path.parent().unwrap_or(path));
}

/// What `reveal` is called on this system.
pub const REVEAL: &str = if cfg!(target_os = "macos") { "Show in Finder" } else { "Show in folder" };

/// The file a playlist's `spotify:local:artist:album:title:seconds` entry
/// refers to: same title, one artist credit holding the other as whole words
/// ("Artist" is in "Artist feat. Other", "Ali" isn't "Alicia Keys"), a length
/// within 3 s; the album breaks ties. Spotify keeps only this description, not
/// the file, so this is how a copied file is recognized (the player matches the
/// same way: vendor/librespot-playback/src/local_file.rs).
pub fn find(tracks: &[LocalTrack], uri: &str) -> Option<usize> {
    let parts: Vec<String> = uri.strip_prefix("spotify:local:")?.split(':').map(|p| crate::lyrics::matching::normalize(&crate::sp::conv::form_decode(p))).collect();
    let [artist, album, title, secs] = parts.as_slice() else { return None };
    let secs: u64 = secs.parse().unwrap_or(0);
    let norm = crate::lyrics::matching::normalize;
    tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            if norm(&t.title) != *title || title.is_empty() {
                return None;
            }
            let a = norm(&t.artist);
            let within = |short: &str, long: &str| !short.is_empty() && format!(" {long} ").contains(&format!(" {short} "));
            if !artist.is_empty() && !a.is_empty() && !(a == *artist || within(artist, &a) || within(&a, artist)) {
                return None;
            }
            let apart = if secs > 0 && t.duration_ms > 0 {
                let d = (t.duration_ms / 1000).abs_diff(secs).min((t.duration_ms.div_ceil(1000)).abs_diff(secs));
                if d > 3 {
                    return None;
                }
                d
            } else {
                3
            };
            let same_album = !album.is_empty() && norm(&t.album) == *album;
            Some((i, (!same_album) as u64 * 100 + apart))
        })
        .min_by_key(|(_, score)| *score)
        .map(|(i, _)| i)
}

// --------------------------------------------------------------------- tags

/// The fields the tag editor changes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cover {
    Keep,
    Remove,
    /// A JPEG to embed as the front cover.
    Set(Arc<Vec<u8>>),
}

pub fn read_tags(t: &LocalTrack) -> Tags {
    Tags { title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone(), year: t.year.map(|y| y.to_string()).unwrap_or_default(), genre: t.genre.clone() }
}

/// Checks what the editor would write; the error says which field is wrong.
pub fn validate(tags: &Tags) -> Result<Option<u16>, String> {
    if tags.title.trim().is_empty() {
        return Err("A song needs a title.".into());
    }
    match tags.year.trim() {
        "" => Ok(None),
        y => match y.parse::<u16>() {
            Ok(n) if (1000..=2999).contains(&n) => Ok(Some(n)),
            _ => Err("The year should be four digits, like 2019.".into()),
        },
    }
}

/// Writes `tags` (and the cover) into the file. The file is copied, the copy
/// edited and then moved over the original, so a failure leaves it as it
/// was; its modification date is kept so it doesn't jump to the top of
/// "recently added".
pub fn write_tags(path: &Path, tags: &Tags, cover: &Cover) -> Result<(), String> {
    let year = validate(tags)?;
    // A symlinked song is edited where it really is (renaming over the link would replace it).
    let real = path.canonicalize().map_err(|e| format!("Couldn't find the file: {e}"))?;
    let path = real.as_path();
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("This file's name can't be read.")?;
    let tmp = path.with_file_name(format!(".{name}.glass-edit"));
    std::fs::copy(path, &tmp).map_err(|e| format!("Couldn't copy the file: {e}"))?;
    let result = (|| -> Result<(), String> {
        let mut tagged = open(&tmp).ok_or("This file's format can't be edited.")?;
        if tagged.primary_tag().is_none() {
            let kind = tagged.primary_tag_type();
            tagged.insert_tag(Tag::new(kind));
        }
        let tag = tagged.primary_tag_mut().ok_or("This file can't hold tags.")?;
        let text = |v: &str| v.trim().to_string();
        match text(&tags.title) {
            v if v.is_empty() => tag.remove_title(),
            v => tag.set_title(v),
        }
        match text(&tags.artist) {
            v if v.is_empty() => tag.remove_artist(),
            v => tag.set_artist(v),
        }
        match text(&tags.album) {
            v if v.is_empty() => tag.remove_album(),
            v => tag.set_album(v),
        }
        match text(&tags.genre) {
            v if v.is_empty() => tag.remove_genre(),
            v => tag.set_genre(v),
        }
        match year {
            Some(y) => tag.set_date(Timestamp { year: y, ..Default::default() }),
            None => tag.remove_date(),
        }
        match cover {
            Cover::Keep => {}
            Cover::Remove => {
                tag.remove_picture_type(PictureType::CoverFront);
                tag.remove_picture_type(PictureType::Other);
            }
            Cover::Set(jpeg) => {
                tag.remove_picture_type(PictureType::CoverFront);
                tag.remove_picture_type(PictureType::Other);
                tag.push_picture(Picture::unchecked(jpeg.as_ref().clone()).pic_type(PictureType::CoverFront).mime_type(MimeType::Jpeg).build());
            }
        }
        tagged.save_to_path(&tmp, WriteOptions::default()).map_err(|e| format!("Couldn't write the tags: {e}"))?;
        if let Some(m) = modified
            && let Ok(f) = std::fs::File::options().write(true).open(&tmp)
        {
            let _ = f.set_modified(m);
        }
        std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't replace the file: {e}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// An image file as a cover: at most 1000 px on a side, as JPEG.
pub fn prepare_cover(image_path: &Path) -> Result<Vec<u8>, String> {
    let img = image::open(image_path).map_err(|e| format!("Couldn't read the image: {e}"))?;
    let img = if img.width() > 1000 || img.height() > 1000 { img.resize(1000, 1000, image::imageops::FilterType::Triangle) } else { img };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img.to_rgb8()).map_err(|e| format!("Couldn't convert the image: {e}"))?;
    Ok(out)
}

// ------------------------------------------------------------------- covers

pub const COVER_SCHEME: &str = "localcover://";

/// Covers by URI: `None` while being read; then the cover, or `None` inside for a file without one.
type Covers = HashMap<String, Option<Option<Arc<[u8]>>>>;

/// Serves embedded covers to egui's image loaders: `localcover://<path>`.
/// Files are read off the UI thread, two at a time (a folder of albums on a
/// slow disk would otherwise stall the window). (The iOS app has its own; see ios/.)
#[cfg(not(glass_mobile))]
pub struct CoverLoader {
    rt: tokio::runtime::Handle,
    slots: Arc<tokio::sync::Semaphore>,
    cache: Arc<Mutex<Covers>>,
}

#[cfg(not(glass_mobile))]
impl CoverLoader {
    pub fn new(rt: tokio::runtime::Handle) -> Self {
        Self { rt, slots: Arc::new(tokio::sync::Semaphore::new(2)), cache: Arc::default() }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Covers> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(crate) fn read_cover(path: &Path) -> Option<Vec<u8>> {
    let tagged = open(path)?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag.pictures().iter().find(|p| p.pic_type() == PictureType::CoverFront).or_else(|| tag.pictures().first())?;
    Some(pic.data().to_vec())
}

/// The type of an embedded cover, from its first bytes (tags often say
/// "image/jpeg" whatever it is).
fn cover_mime(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if b.starts_with(b"BM") {
        "image/bmp"
    } else {
        "image/jpeg"
    }
}

#[cfg(not(glass_mobile))]
impl eframe::egui::load::BytesLoader for CoverLoader {
    fn id(&self) -> &str {
        "glass::local::CoverLoader"
    }

    fn load(&self, ctx: &eframe::egui::Context, uri: &str) -> eframe::egui::load::BytesLoadResult {
        use eframe::egui::load::{Bytes, BytesPoll, LoadError};
        let Some(path) = uri.strip_prefix(COVER_SCHEME) else { return Err(LoadError::NotSupported) };
        match self.cache().get(uri) {
            Some(Some(Some(b))) => return Ok(BytesPoll::Ready { size: None, bytes: Bytes::Shared(b.clone()), mime: Some(cover_mime(b).to_string()) }),
            Some(Some(None)) => return Err(LoadError::Loading("no cover in this file".into())),
            Some(None) => return Ok(BytesPoll::Pending { size: None }),
            None => {}
        }
        self.cache().insert(uri.to_string(), None);
        let (slots, cache, ctx, uri, path) = (self.slots.clone(), self.cache.clone(), ctx.clone(), uri.to_string(), PathBuf::from(path));
        self.rt.spawn(async move {
            let Ok(_slot) = slots.acquire_owned().await else { return };
            let cover = tokio::task::spawn_blocking(move || read_cover(&path).map(Arc::<[u8]>::from)).await.ok().flatten();
            let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
            // Forgotten meanwhile (scrolled away): drop it.
            if let Some(slot @ None) = c.get_mut(&uri) {
                *slot = Some(cover);
                drop(c);
                ctx.request_repaint();
            }
        });
        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.cache().remove(uri);
    }

    fn forget_all(&self) {
        self.cache().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache().values().flatten().flatten().map(|b| b.len()).sum()
    }

    fn has_pending(&self) -> bool {
        self.cache().values().any(Option::is_none)
    }
}

/// A minimal queue player for local files.
pub struct LocalPlayer {
    /// The output; `sink` plays into its mixer. Let go after a while paused
    /// (an open output pulls silence all the time), and opened again to resume.
    stream: Option<rodio::OutputStream>,
    sink: rodio::Sink,
    /// When playback was paused.
    paused_at: Option<Instant>,
    /// Where to carry on from while the output is let go.
    resume_at: u64,
    /// Shared with Local Files when that's what's playing (copied only if it's edited).
    pub queue: Arc<Vec<LocalTrack>>,
    pub index: usize,
    pub active: bool,
    /// The slider position (0–1); every new file starts at it.
    volume: f32,
    /// How many songs right after the current one were queued (`queue_songs`).
    queued: usize,
}

impl LocalPlayer {
    pub fn new() -> Option<Self> {
        let stream = open_output()?;
        let sink = rodio::Sink::connect_new(stream.mixer());
        Some(Self { stream: Some(stream), sink, paused_at: None, resume_at: 0, queue: Arc::default(), index: 0, active: false, volume: 1.0, queued: 0 })
    }

    /// The file playing (to mark it in lists, even after a rescan reorders them).
    pub fn path_playing(&self) -> Option<PathBuf> {
        self.current().map(|t| t.path.clone())
    }

    pub fn current(&self) -> Option<&LocalTrack> {
        if self.active { self.queue.get(self.index) } else { None }
    }

    pub fn play(&mut self, queue: Arc<Vec<LocalTrack>>, index: usize) {
        self.queue = queue;
        self.index = index;
        self.queued = 0;
        self.start_current();
    }

    fn start_current(&mut self) {
        self.sink.stop();
        self.paused_at = None;
        if self.stream.is_none() {
            self.stream = open_output();
        }
        let Some(stream) = self.stream.as_ref() else {
            self.active = false;
            return;
        };
        self.sink = rodio::Sink::connect_new(stream.mixer());
        self.set_volume(self.volume);
        self.active = false;
        // A file that can't be opened or decoded is skipped, not the end of playback.
        while let Some(t) = self.queue.get(self.index) {
            if let Ok(file) = std::fs::File::open(&t.path)
                && let Ok(dec) = rodio::Decoder::try_from(file)
            {
                // The visualizer sees what's played (see `crate::viz`).
                self.sink.append(librespot_playback::tap::tapped(dec));
                self.sink.play();
                self.active = true;
                return;
            }
            log::warn!("can't play {}", t.path.display());
            self.index += 1;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.active && self.stream.is_some() && !self.sink.is_paused() && !self.sink.empty()
    }

    /// The system output changed: carry on from the same spot on the new
    /// default device (paused if it was paused). With the output let go,
    /// resuming opens the new default anyway.
    pub fn follow_output(&mut self) {
        if self.stream.is_none() {
            return;
        }
        let Some(stream) = open_output() else { return };
        let (was_active, paused_at, pos) = (self.active, self.paused_at, self.position_ms());
        self.sink.stop();
        self.sink = rodio::Sink::connect_new(stream.mixer());
        self.stream = Some(stream);
        self.set_volume(self.volume);
        if was_active {
            self.start_current();
            self.seek(pos);
            if paused_at.is_some() {
                self.sink.pause();
                self.paused_at = paused_at;
            }
        }
    }

    /// Adds songs after the current one, or at the end. Returns how many.
    pub fn enqueue(&mut self, tracks: Vec<LocalTrack>, next: bool) -> usize {
        let n = tracks.len();
        if next && self.active {
            let at = (self.index + 1).min(self.queue.len());
            Arc::make_mut(&mut self.queue).splice(at..at, tracks);
        } else {
            Arc::make_mut(&mut self.queue).extend(tracks);
        }
        n
    }

    /// Queues songs: after the current one and those queued before them, in order.
    // Now Playing's queue on iOS (the desktop's assistant uses `enqueue`).
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    pub fn queue_songs(&mut self, tracks: Vec<LocalTrack>) -> usize {
        let n = tracks.len();
        let at = (self.index + 1 + self.queued).min(self.queue.len());
        Arc::make_mut(&mut self.queue).splice(at..at, tracks);
        self.queued += n;
        n
    }

    /// How many of the songs after the current one were queued.
    // Now Playing's queue on iOS (the desktop's assistant uses `enqueue`).
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    pub fn queued(&self) -> usize {
        self.queued.min(self.queue.len().saturating_sub(self.index + 1))
    }

    /// Moves the song `from` places after the current one to `to` places after it.
    /// One moved into or out of the queued songs joins or leaves them.
    // Now Playing's queue on iOS (the desktop's assistant uses `enqueue`).
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    pub fn move_upcoming(&mut self, from: usize, to: usize) -> bool {
        let (a, b) = (self.index + 1 + from, self.index + 1 + to);
        if !self.active || a >= self.queue.len() || b >= self.queue.len() || a == b {
            return false;
        }
        let queued = self.queued();
        if from < queued && to >= queued {
            self.queued -= 1;
        } else if from >= queued && to < queued {
            self.queued += 1;
        }
        let q = Arc::make_mut(&mut self.queue);
        let t = q.remove(a);
        q.insert(b, t);
        true
    }

    /// Removes queued (not yet playing) copies of these files: undo for `enqueue`.
    pub fn dequeue(&mut self, paths: &[PathBuf]) -> usize {
        let mut removed = 0;
        let mut i = self.index + 1;
        while i < self.queue.len() {
            if paths.contains(&self.queue[i].path) {
                Arc::make_mut(&mut self.queue).remove(i);
                removed += 1;
            } else {
                i += 1;
            }
        }
        removed
    }

    /// What's playing and what comes after it.
    pub fn upcoming(&self) -> Vec<PathBuf> {
        if !self.active {
            return Vec::new();
        }
        self.queue.iter().skip(self.index).map(|t| t.path.clone()).collect()
    }

    pub fn toggle(&mut self) {
        if self.stream.is_none() {
            let at = self.resume_at;
            self.start_current();
            self.seek(at);
        } else if self.sink.is_paused() {
            self.sink.play();
            self.paused_at = None;
        } else {
            self.sink.pause();
            self.paused_at = Some(Instant::now());
        }
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.active = false;
    }

    pub fn next(&mut self) {
        if self.index + 1 < self.queue.len() {
            self.index += 1;
            self.queued = self.queued.saturating_sub(1);
            self.start_current();
        } else {
            self.stop();
        }
    }

    pub fn prev(&mut self) {
        if self.position_ms() > 3000 || self.index == 0 {
            self.seek(0);
        } else {
            self.index -= 1;
            self.queued = 0;
            self.start_current();
        }
    }

    pub fn seek(&mut self, ms: u64) {
        // Without an output nothing reads the file, and rodio's seek would wait for that.
        if self.stream.is_none() {
            self.resume_at = ms;
            return;
        }
        let _ = self.sink.try_seek(Duration::from_millis(ms));
    }

    pub fn position_ms(&self) -> u64 {
        if self.stream.is_none() { self.resume_at } else { self.sink.get_pos().as_millis() as u64 }
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v;
        self.sink.set_volume(v * v);
    }

    /// Call every frame: moves to the next file when one finishes, and lets
    /// go of the output after half a minute paused.
    pub fn tick(&mut self) {
        if !self.active || self.stream.is_none() {
            return;
        }
        if self.sink.empty() {
            self.next();
        } else if self.paused_at.is_some_and(|t| t.elapsed() > Duration::from_secs(30)) {
            self.resume_at = self.position_ms();
            self.sink.stop();
            self.stream = None;
        }
    }
}

fn open_output() -> Option<rodio::OutputStream> {
    let mut stream = rodio::OutputStreamBuilder::open_default_stream().ok()?;
    stream.log_on_drop(false);
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str, album: &str, secs: u64) -> LocalTrack {
        LocalTrack {
            path: format!("/m/{title}.mp3").into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration_ms: secs * 1000 + 400,
            added: SystemTime::UNIX_EPOCH,
            year: None,
            genre: String::new(),
            has_cover: false,
            ..Default::default()
        }
    }

    #[test]
    fn playlist_entries_find_their_files() {
        let files =
            vec![t("Bloom", "The Paper Kites", "Woodland", 212), t("Kyoto", "Phoebe Bridgers", "Punisher", 184), t("Kyoto", "Phoebe Bridgers", "Kyoto", 184), t("Café del Mar", "Energy 52", "", 300)];
        assert_eq!(find(&files, "spotify:local:The+Paper+Kites:Woodland:Bloom:213"), Some(0), "a second off and + for spaces");
        assert_eq!(find(&files, "spotify:local:Phoebe+Bridgers:Punisher:Kyoto:184"), Some(1), "the album breaks the tie");
        assert_eq!(find(&files, "spotify:local:Energy+52::Caf%C3%A9+del+Mar:300"), Some(3), "percent-encoded Unicode");
        assert_eq!(find(&files, "spotify:local:Someone+Else::Bloom:212"), None, "another artist");
        // Glass's own entries find their file again, whatever the tags hold.
        for (i, t) in files.iter().enumerate() {
            assert_eq!(find(&files, &spotify_uri(t)), Some(i), "{}", spotify_uri(t));
        }
        assert_eq!(find(&files, "spotify:local:The+Paper+Kites::Bloom:260"), None, "another length");
        assert_eq!(find(&files, "spotify:track:abc"), None);
    }

    #[test]
    fn spotify_entries_for_files() {
        let files = vec![t("Ah: The+Song", "Tatsuro Yamashita", "Big Wave", 279), t("山下", "", "", 165)];
        assert_eq!(spotify_uri(&files[0]), "spotify:local:Tatsuro+Yamashita:Big+Wave:Ah%3A+The%2BSong:279", "as Spotify writes them");
        for (i, f) in files.iter().enumerate() {
            assert_eq!(find(&files, &spotify_uri(f)), Some(i));
        }
        let shown = crate::sp::conv::local_track(&spotify_uri(&files[0]));
        assert_eq!((shown["name"].as_str(), shown["album"]["name"].as_str(), shown["duration_ms"].as_u64()), (Some("Ah: The+Song"), Some("Big Wave"), Some(279_000)));
    }

    #[test]
    fn files_handed_over_through_temporary_folders_are_kept() {
        // A file dragged out of Mail. (A screenshot's `TemporaryItems` folder is checked
        // by name only: macOS keeps anything from deleting one made for a test.)
        assert!(is_temporary(Path::new("/private/var/folders/x/T/TemporaryItems/NSIRD_screencaptureui_x/song.mp3")));
        let mail = std::env::temp_dir().join(format!("com.apple.mail.glass-test-{}", std::process::id()));
        let dir = crate::config::test_dir("keep");
        let inbox = dir.join("inbox");
        std::fs::create_dir_all(&mail).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(mail.join("song.mp3"), b"x").unwrap();
        std::fs::write(dir.join("other.mp3"), b"y").unwrap();
        let kept = keep_temporary(vec![mail.join("song.mp3"), dir.join("other.mp3")], &inbox);
        assert_eq!(kept, [inbox.join("song.mp3"), dir.join("other.mp3")], "only the handed-over file is copied");
        assert!(inbox.join("song.mp3").exists());
        assert!(missing_sources(&[mail.join("song.mp3")], &[]).is_empty(), "a temporary folder never becomes a source");
        let _ = std::fs::remove_dir_all(mail);
    }

    #[test]
    fn sources_for_added_files() {
        let dir = crate::config::test_dir("sources");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("music/a")).unwrap();
        std::fs::create_dir_all(dir.join("dl")).unwrap();
        for f in ["music/a/1.mp3", "music/2.flac", "dl/3.ogg", "dl/readme.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let sources = vec![dir.join("music")];
        let picked = vec![dir.join("music/a/1.mp3"), dir.join("dl/3.ogg"), dir.join("dl/readme.txt")];
        assert_eq!(missing_sources(&picked, &sources), vec![dir.join("dl")], "covered folders aren't added twice");
        assert_eq!(missing_sources(&[dir.join("music/a/1.mp3"), dir.clone()], &[]), vec![dir.clone()], "a wider folder replaces a narrower one");
        let read: Vec<PathBuf> = read_paths(&[dir.join("dl/3.ogg"), dir.join("music"), dir.join("dl/readme.txt"), dir.join("dl/3.ogg")]).into_iter().map(|t| t.path).collect();
        assert_eq!(read, vec![dir.join("dl/3.ogg"), dir.join("music/2.flac"), dir.join("music/a/1.mp3")], "audio only, folders by name, no repeats");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_source_too_deep_to_be_read_with_an_outer_one_stays() {
        let top = PathBuf::from("/glass-test-none/music");
        let deep = top.join("1/2/3/4/5/6");
        assert_eq!(tidy_sources(&[top.clone(), top.join("1/2"), deep.clone(), deep.join("x")]), vec![top.clone(), deep.clone()]);
        let mut list = vec![top.clone()];
        assert!(!add_source(&mut list, top.join("1/2/3/4/5")), "read as part of the outer one");
        assert!(add_source(&mut list, deep.clone()), "too deep for that");
        assert_eq!(list, vec![top, deep]);
    }

    #[test]
    fn tag_values_are_checked() {
        let ok = Tags { title: "Song".into(), year: "2019".into(), ..Default::default() };
        assert_eq!(validate(&ok), Ok(Some(2019)));
        assert!(validate(&Tags { title: " ".into(), ..Default::default() }).is_err());
        assert!(validate(&Tags { title: "x".into(), year: "19".into(), ..Default::default() }).is_err());
        assert_eq!(validate(&Tags { title: "x".into(), ..Default::default() }), Ok(None));
    }

    /// A tiny valid MP3: a few silent MPEG-1 Layer III frames.
    fn silent_mp3() -> Vec<u8> {
        // 128 kbps, 44.1 kHz, no padding: 417-byte frames.
        let mut frame = vec![0xFF, 0xFB, 0x90, 0x64];
        frame.resize(417, 0);
        frame.repeat(40)
    }

    /// Needs an audio output (it plays nothing audible): `cargo test -- --ignored paused_player`.
    #[test]
    #[ignore]
    fn a_paused_player_lets_go_of_the_output_and_resumes_where_it_was() {
        let dir = crate::config::test_dir("pause");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.mp3");
        std::fs::write(&path, silent_mp3()).unwrap();
        let mut p = LocalPlayer::new().expect("an audio output");
        p.set_volume(0.0);
        p.play(Arc::new(vec![LocalTrack { path, ..Default::default() }]), 0);
        assert!(p.is_playing());
        p.toggle();
        p.seek(400);
        p.paused_at = Some(Instant::now() - Duration::from_secs(31));
        p.tick();
        assert!(p.stream.is_none() && p.current().is_some(), "let go, still the song");
        assert_eq!(p.position_ms(), 400);
        p.seek(300);
        p.toggle();
        assert!(p.stream.is_some() && p.is_playing());
        assert!((300..700).contains(&p.position_ms()), "{}", p.position_ms());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_and_covers_are_written_and_read_back() {
        let dir = crate::config::test_dir("tags");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.mp3");
        std::fs::write(&path, silent_mp3()).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut img = image::RgbImage::new(4, 4);
        img.put_pixel(0, 0, image::Rgb([200, 10, 10]));
        let png = dir.join("cover.png");
        img.save(&png).unwrap();
        let jpeg = prepare_cover(&png).unwrap();

        let tags = Tags { title: "Nuevo Día".into(), artist: "Ana".into(), album: "Álbum".into(), year: "2021".into(), genre: "Indie".into() };
        write_tags(&path, &tags, &Cover::Set(Arc::new(jpeg))).unwrap();
        let blank = t("x", "", "", 0);
        let got = rescan_file(&LocalTrack { path: path.clone(), ..blank.clone() });
        assert_eq!(read_tags(&got), tags);
        assert!(got.has_cover);
        assert!(read_cover(&path).is_some_and(|b| b.starts_with(&[0xFF, 0xD8])), "a JPEG cover");
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before, "the file keeps its date");
        assert!(!dir.join(".song.mp3.glass-edit").exists(), "no temporary file left behind");

        // Clearing fields and removing the cover.
        let cleared = Tags { title: "Nuevo Día".into(), ..Default::default() };
        write_tags(&path, &cleared, &Cover::Remove).unwrap();
        let got = rescan_file(&LocalTrack { path: path.clone(), ..blank.clone() });
        assert_eq!(read_tags(&got), cleared);
        assert!(!got.has_cover);

        // A bad year writes nothing.
        let bad = Tags { title: "x".into(), year: "soon".into(), ..Default::default() };
        assert!(write_tags(&path, &bad, &Cover::Keep).is_err());
        assert_eq!(rescan_file(&LocalTrack { path: path.clone(), ..blank }).title, "Nuevo Día");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
