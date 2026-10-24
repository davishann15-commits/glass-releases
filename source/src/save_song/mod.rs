//! Share ▸ "Download as M4A/MP3": saves a song as a plain audio file. Everything is
//! built in; there's nothing else to install.
//!
//! The audio doesn't come from Spotify. The song is looked up on JioSaavn (AAC up to
//! 320 kbps), then YouTube Music (AAC 128 kbps), and the right recording is picked by
//! title words, artists and length, passing over other versions (see `matching`). M4A keeps that AAC
//! as it is; MP3 decodes it (symphonia) and encodes it (LAME). Tags and cover come from
//! the Spotify song: `mp4` writes an M4A's itself, lofty an MP3's. Files land in
//! Settings ▸ Save songs ▸ Folder, `~/Music/Glass` by default, which Local Files
//! already reads.

mod lame;
mod matching;
mod mp3;
mod mp4;
mod saavn;
mod youtube;

pub use mp3::BITRATES;

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, items::Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Format {
    /// AAC exactly as the source has it.
    #[default]
    M4a,
    /// Re-encoded at the chosen bitrate; plays everywhere.
    Mp3,
}

impl Format {
    pub const ALL: [Format; 2] = [Format::M4a, Format::Mp3];

    pub fn name(self) -> &'static str {
        match self {
            Format::M4a => "M4A",
            Format::Mp3 => "MP3",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::M4a => "m4a",
            Format::Mp3 => "mp3",
        }
    }
}

/// What to save and where (from Settings).
#[derive(Clone, Debug)]
pub struct Options {
    pub format: Format,
    pub mp3_kbps: u16,
    pub dir: PathBuf,
}

/// `~/Music/Glass`
pub fn default_dir() -> PathBuf {
    // Looked up once (on Linux it means reading user-dirs.dirs).
    static DIR: std::sync::LazyLock<PathBuf> = std::sync::LazyLock::new(|| {
        directories::UserDirs::new()
            .and_then(|d| d.audio_dir().map(Path::to_path_buf))
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join("Music")))
            .unwrap_or_else(|| PathBuf::from("Music"))
            .join("Glass")
    });
    DIR.clone()
}

/// The song's metadata, from Glass's Web API data.
#[derive(Clone, Debug, Default)]
pub struct Song {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artists: Vec<String>,
    pub year: Option<String>,
    pub track_number: Option<u32>,
    pub duration_sec: u32,
    pub cover_url: Option<String>,
    pub link: Option<String>,
}

/// A Web API track. None for episodes and local files.
pub fn song_from_item(item: &Value) -> Option<Song> {
    if item["type"].as_str() == Some("episode") || item["is_local"].as_bool() == Some(true) {
        return None;
    }
    let names = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).filter(|n| !n.is_empty()).map(String::from).collect()).unwrap_or_default() };
    let album = &item["album"];
    let artists = names(&item["artists"]);
    if artists.is_empty() {
        return None;
    }
    Some(Song {
        title: item["name"].as_str().filter(|t| !t.is_empty())?.to_string(),
        artists,
        album: album["name"].as_str().filter(|n| !n.is_empty()).map(String::from),
        album_artists: names(&album["artists"]),
        year: album["release_date"].as_str().filter(|d| !d.is_empty()).map(String::from),
        track_number: item["track_number"].as_u64().map(|n| n as u32),
        duration_sec: (item["duration_ms"].as_u64().unwrap_or(0) / 1000) as u32,
        cover_url: album["images"].as_array().and_then(|imgs| imgs.iter().max_by_key(|i| i["width"].as_u64().unwrap_or(0))).and_then(|i| i["url"].as_str()).map(String::from),
        link: item["external_urls"]["spotify"].as_str().map(String::from).or_else(|| item["uri"].as_str()?.strip_prefix("spotify:track:").map(|id| format!("https://open.spotify.com/track/{id}"))),
    })
}

/// `<folder>/<Artist> - <Title>.<ext>`
pub fn output_path(song: &Song, opts: &Options) -> PathBuf {
    let name = format!("{} - {}", song.artists.first().map(String::as_str).unwrap_or("Unknown"), song.title);
    opts.dir.join(format!("{}.{}", file_name(&name), opts.format.extension()))
}

/// A name that's safe as a file or folder name everywhere: no separators or
/// reserved characters, not hidden (no leading dot), and at most 200 bytes, so
/// it fits every file system's limit (255) with its extension.
pub fn file_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let mut clean = clean.trim().trim_matches('.').trim().to_string();
    if clean.len() > 200 {
        let cut = (0..=200).rev().find(|&i| clean.is_char_boundary(i)).unwrap_or(0);
        clean.truncate(cut);
        clean = clean.trim_end().trim_end_matches('.').to_string();
    }
    if clean.is_empty() { "Untitled".into() } else { clean }
}

/// A work file beside `dest`: hidden (so Local Files skips it) and unique to this
/// save, so two saves that end up with the same name never share one.
fn work_file(dest: &Path, id: &str, what: &str) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!(".{name}.{id}.{what}"))
}

/// Removes work files a save left behind when Glass quit or crashed mid-way, in
/// `dir` and its collection folders. Only ones an hour old: none is in use then.
pub fn sweep_work_files(dir: &Path) {
    let stale = |p: &Path| {
        let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        name.starts_with('.')
            && [".glass-part", ".glass-plain", ".glass-tmp"].iter().any(|w| name.ends_with(w))
            && std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(3600))
    };
    let Ok(top) = std::fs::read_dir(dir) else { return };
    for e in top.flatten() {
        let p = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            for f in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                if stale(&f.path()) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        } else if stale(&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// What a whole-collection save is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetKind {
    Playlist,
    Album,
    /// An artist's top songs.
    Artist,
}

impl SetKind {
    /// "Download <noun> as M4A"
    pub fn noun(self) -> &'static str {
        match self {
            SetKind::Playlist => "playlist",
            SetKind::Album => "album",
            SetKind::Artist => "top songs",
        }
    }
}

/// `<name>.m3u8` in `folder`, listing the saved songs in the collection's order.
pub fn write_playlist_file(folder: &Path, name: &str, songs: &[PathBuf]) -> std::io::Result<PathBuf> {
    let mut text = String::from("#EXTM3U\n");
    for path in songs {
        if let Some(file) = path.file_name() {
            text.push_str(&file.to_string_lossy());
            text.push('\n');
        }
    }
    let path = folder.join(format!("{}.m3u8", file_name(name)));
    std::fs::write(&path, text)?;
    Ok(path)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Queued,
    /// Stopped before it started (its collection's save was stopped).
    Stopped,
    Finding,
    Downloading(u8),
    Converting,
    Done,
    Failed(String),
}

/// A song being (or done being) saved this session.
#[derive(Clone, Debug)]
pub struct Job {
    pub title: String,
    pub status: Status,
    pub path: PathBuf,
}

impl Job {
    pub fn busy(&self) -> bool {
        matches!(self.status, Status::Queued | Status::Finding | Status::Downloading(_) | Status::Converting)
    }
}

/// For finding and downloading. A download that goes quiet for 30 seconds has stalled.
pub fn http_client() -> reqwest::Client {
    crate::config::http().connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(30)).build().unwrap_or_default()
}

/// Finds, downloads, converts and tags the song; returns where it was saved.
pub async fn save(http: reqwest::Client, song: Song, opts: Options, report: Arc<dyn Fn(Status) + Send + Sync>) -> Result<PathBuf> {
    let dest = output_path(&song, &opts);
    let dir = dest.parent().context("no folder")?.to_path_buf();
    tokio::fs::create_dir_all(&dir).await.with_context(|| format!("couldn't create {}", dir.display()))?;
    // Work files sit next to the result and go away however this ends.
    let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let (part, plain, tmp) = (work_file(&dest, &id, "glass-part"), work_file(&dest, &id, "glass-plain"), work_file(&dest, &id, "glass-tmp"));
    let _cleanup = Cleanup(vec![part.clone(), plain.clone(), tmp.clone()]);

    report(Status::Finding);
    // Each whole percent once (the download comes in many small chunks).
    let shown = std::sync::atomic::AtomicU8::new(u8::MAX);
    let progress = |done: u64, total: u64| {
        if let Some(pct) = (done * 100).checked_div(total).map(|p| p.min(100) as u8)
            && shown.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct
        {
            report(Status::Downloading(pct));
        }
    };
    // JioSaavn first (up to 320 kbps), then YouTube Music.
    let from_saavn = async {
        let (url, kbps) = saavn::find(&http, &song).await?;
        let mut resp = http.get(&url).send().await?.error_for_status()?;
        let total = resp.content_length().unwrap_or(0);
        let mut file = tokio::fs::File::create(&part).await?;
        let mut done = 0;
        while let Some(bytes) = resp.chunk().await? {
            file.write_all(&bytes).await?;
            done += bytes.len() as u64;
            progress(done, total);
        }
        file.flush().await?;
        anyhow::ensure!(done > 0 && (total == 0 || done == total), "JioSaavn's download was cut short");
        log::info!("save song: \"{}\" from JioSaavn at {kbps} kbps", song.title);
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = from_saavn {
        log::info!("save song: JioSaavn didn't have \"{}\" ({e:#}); trying YouTube Music", song.title);
        let found = youtube::find(&http, &song).await?;
        youtube::fetch(&http, &found.video_id, found.visitor, &part, progress).await?;
        log::info!("save song: \"{}\" from YouTube ({})", song.title, found.video_id);
    }

    report(Status::Converting);
    let cover = match &song.cover_url {
        Some(url) => async { http.get(url).timeout(TIMEOUT).send().await.ok()?.error_for_status().ok()?.bytes().await.ok() }.await,
        None => None,
    };
    let finish = {
        let dest = dest.clone();
        move || -> Result<()> {
            match opts.format {
                // Streams the audio into a plain M4A and writes the tags on the way.
                Format::M4a => mp4::rewrite(&part, &tmp, Some((&song, cover.as_deref())))?,
                Format::Mp3 => {
                    // The decoder reads plain MP4s, so fragmented YouTube audio is rewritten first.
                    mp4::rewrite(&part, &plain, None)?;
                    mp3::encode(&plain, &tmp, opts.mp3_kbps)?;
                    tag_mp3(&tmp, &song, cover.as_deref())?;
                }
            }
            std::fs::rename(&tmp, &dest)?;
            Ok(())
        }
    };
    tokio::task::spawn_blocking(finish).await??;
    Ok(dest)
}

/// Removes work files when dropped.
struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn tag_mp3(path: &Path, song: &Song, cover: Option<&[u8]>) -> Result<()> {
    let mut file = lofty::probe::Probe::open(path)?.guess_file_type()?.read()?;
    let kind = file.primary_tag_type();
    file.insert_tag(Tag::new(kind));
    let tag = file.primary_tag_mut().context("this file can't hold tags")?;
    tag.set_title(song.title.clone());
    tag.set_artist(song.artists.join(", "));
    if !song.album_artists.is_empty() {
        tag.insert_text(ItemKey::AlbumArtist, song.album_artists.join(", "));
    }
    if let Some(album) = &song.album {
        tag.set_album(album.clone());
    }
    if let Some(date) = song.year.as_deref().and_then(date) {
        tag.set_date(date);
    }
    if let Some(n) = song.track_number {
        tag.set_track(n);
    }
    if let Some(link) = &song.link {
        tag.set_comment(link.clone());
    }
    if let Some(data) = cover.filter(|c| !c.is_empty()) {
        let mime = if data.starts_with(&[0x89, b'P', b'N', b'G']) { MimeType::Png } else { MimeType::Jpeg };
        tag.push_picture(Picture::unchecked(data.to_vec()).pic_type(PictureType::CoverFront).mime_type(mime).build());
    }
    file.save_to_path(path, WriteOptions::default()).context("couldn't write the tags")?;
    Ok(())
}

/// "2020", "2020-03" or "2020-03-20".
fn date(s: &str) -> Option<Timestamp> {
    let mut parts = s.trim().split('-');
    let year = parts.next()?.get(..4)?.parse().ok()?;
    let month = parts.next().and_then(|m| m.parse().ok()).filter(|m| (1..=12).contains(m));
    let day = month.and(parts.next().and_then(|d| d.get(..2)?.parse().ok())).filter(|d| (1..=31).contains(d));
    Some(Timestamp { year, month, day, ..Default::default() })
}

/// The short reason a save failed, for a toast (the full error goes to the log).
/// Never the URL: download links carry the user's IP address and signatures.
pub fn short_error(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    if text.contains("no match on") {
        return "couldn't find it on JioSaavn or YouTube Music".into();
    }
    if text.contains("YouTube won't play") {
        return "YouTube wouldn't play it".into();
    }
    if text.contains("YouTube keeps refusing") {
        return "YouTube refused the download; try again later".into();
    }
    if let Some(r) = e.chain().find_map(|c| c.downcast_ref::<reqwest::Error>()) {
        return match r.status() {
            _ if r.is_timeout() => "the download stalled".into(),
            Some(status) => format!("the server refused it ({status})"),
            None => format!("network error: {}", reqwest_reason(r)),
        };
    }
    e.to_string()
}

/// A reqwest error's own words, without its URL.
fn reqwest_reason(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    if let Some(url) = e.url() {
        s = s.replace(url.as_str(), "");
    }
    s.replace(" for url ()", "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item() -> Value {
        json!({
            "type": "track", "uri": "spotify:track:0VjIjW4GlUZAMYd2vXMi3b", "name": "Blinding Lights",
            "duration_ms": 200040, "track_number": 9,
            "artists": [{"name": "The Weeknd"}],
            "album": {"name": "After Hours", "release_date": "2020-03-20", "artists": [{"name": "The Weeknd"}],
                      "images": [{"url": "https://i.scdn.co/image/small", "width": 64}, {"url": "https://i.scdn.co/image/ab67616d0000b2738863bc11d2aa12b54f5aeb36", "width": 640}]}
        })
    }

    #[test]
    fn web_api_tracks_become_songs() {
        let s = song_from_item(&item()).unwrap();
        assert_eq!((s.title.as_str(), s.duration_sec, s.track_number), ("Blinding Lights", 200, Some(9)));
        assert_eq!(s.cover_url.as_deref(), Some("https://i.scdn.co/image/ab67616d0000b2738863bc11d2aa12b54f5aeb36"));
        assert_eq!(s.link.as_deref(), Some("https://open.spotify.com/track/0VjIjW4GlUZAMYd2vXMi3b"));
        assert!(song_from_item(&json!({"type": "episode", "name": "x", "artists": [{"name": "a"}]})).is_none());
        assert!(song_from_item(&json!({"type": "track", "name": "x", "is_local": true, "artists": [{"name": "a"}]})).is_none());
    }

    #[test]
    fn file_names_are_safe() {
        let opts = Options { format: Format::Mp3, mp3_kbps: 320, dir: PathBuf::from("/m") };
        let song = Song { title: "AC/DC: Live?".into(), artists: vec!["Some*One".into()], ..Default::default() };
        assert_eq!(output_path(&song, &opts), PathBuf::from("/m/Some_One - AC_DC_ Live_.mp3"));
        assert_eq!(file_name("  ...  "), "Untitled");
        assert_eq!(date("2020-03-20").map(|d| (d.year, d.month, d.day)), Some((2020, Some(3), Some(20))));
    }

    #[test]
    fn leftover_work_files_are_swept() {
        let dir = crate::config::test_dir("sweep");
        let set = dir.join("A Playlist");
        std::fs::create_dir_all(&set).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(7200);
        let make = |p: PathBuf, age: Option<std::time::SystemTime>| {
            let f = std::fs::File::create(&p).unwrap();
            if let Some(t) = age {
                f.set_modified(t).unwrap();
            }
            p
        };
        let stale = [make(dir.join(".Song.m4a.1a2b3c4d.glass-part"), Some(old)), make(set.join(".Other.mp3.5e6f7a8b.glass-tmp"), Some(old))];
        let kept = [make(dir.join(".Song.m4a.9c0d1e2f.glass-plain"), None), make(dir.join("Song.m4a"), Some(old)), make(set.join(".hidden"), Some(old))];
        sweep_work_files(&dir);
        assert!(stale.iter().all(|p| !p.exists()), "old work files go");
        assert!(kept.iter().all(|p| p.exists()), "songs, other files and a save in progress stay");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The whole pipeline against the real services: `cargo test live_save -- --ignored`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_save() {
        let dir = crate::config::test_dir("save");
        let report: Arc<dyn Fn(Status) + Send + Sync> = Arc::new(|s| eprintln!("{s:?}"));
        // Not on JioSaavn (YouTube), and on JioSaavn.
        let kesariya = json!({
            "type": "track", "uri": "spotify:track:x", "name": "Kesariya", "duration_ms": 268000,
            "artists": [{"name": "Pritam"}, {"name": "Arijit Singh"}, {"name": "Amitabh Bhattacharya"}],
            "album": {"name": "Brahmastra", "release_date": "2022-07-17"}
        });
        for item in [item(), kesariya] {
            for format in Format::ALL {
                let opts = Options { format, mp3_kbps: 192, dir: dir.clone() };
                let path = save(http_client(), song_from_item(&item).unwrap(), opts, report.clone()).await.unwrap();
                eprintln!("saved {}", path.display());
                assert!(path.exists());
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Six songs, three at a time, like a collection save: `cargo test live_save_many -- --ignored`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_save_many() {
        let dir = crate::config::test_dir("save-many");
        let album = json!({ "name": "After Hours", "release_date": "2020-03-20", "artists": [{ "name": "The Weeknd" }], "images": [] });
        let titles = [("Alone Again", 250), ("Too Late", 239), ("Hardest To Love", 211), ("Scared To Live", 191), ("Snowchild", 247), ("Escape From LA", 355)];
        let slots = Arc::new(tokio::sync::Semaphore::new(3));
        let started = std::time::Instant::now();
        let mut tasks = Vec::new();
        for (i, (title, secs)) in titles.into_iter().enumerate() {
            let item = json!({ "type": "track", "uri": format!("spotify:track:{i}"), "name": title, "duration_ms": secs * 1000, "track_number": i + 1, "artists": [{ "name": "The Weeknd" }], "album": album });
            let (slots, opts) = (slots.clone(), Options { format: Format::M4a, mp3_kbps: 320, dir: dir.clone() });
            tasks.push(tokio::spawn(async move {
                let _slot = slots.acquire_owned().await;
                save(http_client(), song_from_item(&item).unwrap(), opts, Arc::new(|_| {})).await.map_err(|e| format!("{title}: {e:#}"))
            }));
        }
        let mut failed = Vec::new();
        for t in tasks {
            if let Err(e) = t.await.unwrap() {
                failed.push(e);
            }
        }
        eprintln!("6 songs in {:.1}s; failed: {failed:?}", started.elapsed().as_secs_f32());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failed.is_empty());
    }
}
