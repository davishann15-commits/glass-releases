//! A Local Files song as the media library wants it (ByeTunes' SongMetadata, read
//! with lofty instead of AVFoundation).

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::*;
use lofty::tag::ItemKey;
use std::path::{Path, PathBuf};

/// What the Music app plays from its library (ByeTunes' list, plus AIFF). Not Ogg.
pub const PLAYABLE: &[&str] = &["mp3", "m4a", "mp4", "aac", "flac", "wav", "aif", "aiff"];

// Core Audio format IDs (four-character codes).
const MP3: i64 = 0x2E6D_7033; // '.mp3'
const AAC: i64 = 0x6161_6320; // 'aac '
const ALAC: i64 = 0x616C_6163; // 'alac'
const FLAC: i64 = 0x666C_6163; // 'flac'
const LPCM: i64 = 0x6C70_636D; // 'lpcm'

#[derive(Clone, Debug)]
pub struct Song {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: Option<String>,
    pub genre: String,
    pub year: i64,
    pub duration_ms: i64,
    pub file_size: i64,
    /// The extension the copy on the iPhone gets.
    pub ext: String,
    pub track: Option<u32>,
    pub track_count: Option<u32>,
    pub disc: Option<u32>,
    pub disc_count: Option<u32>,
    pub lyrics: String,
    pub copyright: String,
    /// 1 for explicit (iTunes advisory), else 0.
    pub explicit: i64,
    /// item_playback: ByeTunes' format by extension, the codec, kbps and Hz.
    pub audio_format: i64,
    pub codec: i64,
    pub bit_rate: i64,
    pub sample_rate: f64,
    /// The file has a cover (`cover` reads it when it's uploaded).
    pub artwork: bool,
    /// Its background color for the library (ByeTunes' `representativeArtworkColor`).
    pub artwork_color: Option<(u8, u8, u8)>,
    /// When the file was made (Unix seconds): the song's "date added".
    pub added: Option<i64>,
}

/// Reads a song file; why not, when the Music app couldn't take it.
pub fn read(path: &Path) -> Result<Song, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !PLAYABLE.contains(&ext.as_str()) {
        return Err(format!("{name} — the Music app can't play this kind of file"));
    }
    let meta = std::fs::metadata(path).map_err(|_| format!("{name} — not there any more"))?;
    if meta.len() == 0 {
        return Err(format!("{name} — empty file"));
    }
    let tagged = lofty::probe::Probe::open(path).ok().and_then(|p| p.guess_file_type().ok()).and_then(|p| p.read().ok()).ok_or_else(|| format!("{name} — can't be read"))?;
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let text = |v: Option<std::borrow::Cow<'_, str>>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());

    let ext = if ext == "mp4" { "m4a".to_string() } else { ext };
    let codec = match ext.as_str() {
        "mp3" => MP3,
        "flac" => FLAC,
        "wav" | "aif" | "aiff" => LPCM,
        _ => mp4_codec(path).unwrap_or(AAC),
    };
    // ByeTunes' `defaultAudioFormatForExtension` ('fLaC' and 'WAVE' are what it writes).
    let audio_format = match ext.as_str() {
        "mp3" => 301,
        "flac" => 0x664C_6143,
        "wav" => 0x5741_5645,
        "aif" | "aiff" => LPCM,
        _ => AAC,
    };
    let picture = tag.and_then(front_cover);
    let artwork_color = picture.and_then(|p| representative_color(p.data()));
    let added = meta.created().or_else(|_| meta.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);

    Ok(Song {
        path: path.to_path_buf(),
        title: text(tag.and_then(|t| t.title())).unwrap_or(stem),
        artist: text(tag.and_then(|t| t.artist())).unwrap_or_else(|| "Unknown Artist".into()),
        album: text(tag.and_then(|t| t.album())).unwrap_or_else(|| "Unknown Album".into()),
        album_artist: tag.and_then(|t| t.get_string(ItemKey::AlbumArtist)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        genre: text(tag.and_then(|t| t.genre())).unwrap_or_default(),
        year: tag.and_then(|t| t.date()).map(|d| d.year as i64).filter(|y| *y > 0).unwrap_or(0),
        duration_ms: props.duration().as_millis() as i64,
        file_size: meta.len() as i64,
        ext,
        track: tag.and_then(|t| t.track()).filter(|n| *n > 0),
        track_count: tag.and_then(|t| t.track_total()).filter(|n| *n > 0),
        disc: tag.and_then(|t| t.disk()).filter(|n| *n > 0),
        disc_count: tag.and_then(|t| t.disk_total()).filter(|n| *n > 0),
        lyrics: tag.and_then(|t| t.get_string(ItemKey::UnsyncLyrics).or_else(|| t.get_string(ItemKey::Lyrics))).map(clean_lyrics).unwrap_or_default(),
        copyright: tag.and_then(|t| t.get_string(ItemKey::CopyrightMessage)).map(|s| s.trim().to_string()).unwrap_or_default(),
        explicit: tag.and_then(|t| t.get_string(ItemKey::ParentalAdvisory)).map(|s| if s.trim() == "1" { 1 } else { 0 }).unwrap_or(0),
        audio_format,
        codec,
        bit_rate: props.audio_bitrate().filter(|b| *b > 0).unwrap_or(320) as i64,
        sample_rate: props.sample_rate().filter(|r| *r > 0).unwrap_or(44_100) as f64,
        artwork: picture.is_some(),
        artwork_color,
        added,
    })
}

fn front_cover(t: &lofty::tag::Tag) -> Option<&lofty::picture::Picture> {
    t.pictures().iter().find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront).or_else(|| t.pictures().first()).filter(|p| !p.data().is_empty())
}

/// The file's cover as it's stored (JPEG or PNG).
pub fn cover(path: &Path) -> Option<Vec<u8>> {
    let tagged = lofty::probe::Probe::open(path).ok()?.guess_file_type().ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    front_cover(tag).map(|p| p.data().to_vec())
}

/// AAC or Apple Lossless in an .m4a.
fn mp4_codec(path: &Path) -> Option<i64> {
    let mut f = std::fs::File::open(path).ok()?;
    let mp4 = lofty::mp4::Mp4File::read_from(&mut f, lofty::config::ParseOptions::new().read_tags(false).read_cover_art(false)).ok()?;
    match mp4.properties().codec()? {
        lofty::mp4::Mp4Codec::ALAC => Some(ALAC),
        lofty::mp4::Mp4Codec::MP3 => Some(MP3),
        lofty::mp4::Mp4Codec::FLAC => Some(FLAC),
        _ => Some(AAC),
    }
}

/// Lyrics from a tag without LRC times, [Chorus]-style labels and runs of empty lines.
fn clean_lyrics(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut depth = 0;
    for c in raw.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

pub fn luminance(c: (u8, u8, u8)) -> f64 {
    (0.2126 * c.0 as f64 + 0.7152 * c.1 as f64 + 0.0722 * c.2 as f64) / 255.0
}

/// `amount` of the way from `c` to `to`.
pub fn mix(c: (u8, u8, u8), to: (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    let m = |a: u8, b: u8| (a as f64 * (1.0 - amount) + b as f64 * amount).round().clamp(0.0, 255.0) as u8;
    (m(c.0, to.0), m(c.1, to.1), m(c.2, to.2))
}

/// The cover's most prominent color, darkened and saturated a little to sit behind
/// light text (ByeTunes' `representativeArtworkColor` and `displayBackgroundColor`).
fn representative_color(bytes: &[u8]) -> Option<(u8, u8, u8)> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let small = image::imageops::resize(&img, 40, 40, image::imageops::FilterType::Triangle);
    let mut buckets: std::collections::HashMap<u32, (f64, f64, f64, f64)> = std::collections::HashMap::new();
    for p in small.pixels() {
        let [r, g, b, a] = p.0;
        if a <= 200 {
            continue;
        }
        let l = luminance((r, g, b));
        if !(0.06..0.94).contains(&l) {
            continue;
        }
        let (max, min) = (r.max(g).max(b) as f64 / 255.0, r.min(g).min(b) as f64 / 255.0);
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        let weight = 1.0 + saturation * 3.0 + (l - 0.5).abs() * 0.35;
        let key = ((r as u32 / 24) << 16) | ((g as u32 / 24) << 8) | (b as u32 / 24);
        let e = buckets.entry(key).or_default();
        e.0 += weight;
        e.1 += r as f64 * weight;
        e.2 += g as f64 * weight;
        e.3 += b as f64 * weight;
    }
    let best = buckets.values().max_by(|a, b| a.0.total_cmp(&b.0)).filter(|b| b.0 > 0.0)?;
    let (r, g, b) = (best.1 / best.0 / 255.0, best.2 / best.0 / 255.0, best.3 / best.0 / 255.0);
    // To hue, saturation, brightness; adjusted; and back.
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let hue = if d == 0.0 {
        0.0
    } else if max == r {
        (((g - b) / d).rem_euclid(6.0)) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let sat = if max > 0.0 { d / max } else { 0.0 };
    let (s, v) = ((sat * 1.15).clamp(0.18, 0.9), (max * 0.82).clamp(0.18, 0.62));
    let h6 = hue * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f64| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Some((to(r1), to(g1), to(b1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_lose_times_and_labels() {
        let raw = "[ar:Someone]\n[00:12.30]First line\n\n\n[Chorus]\n[00:15.00]Second line\n\n";
        assert_eq!(clean_lyrics(raw), "First line\n\nSecond line");
    }

    #[test]
    fn a_tagged_mp3_is_read() {
        use lofty::config::WriteOptions;
        use lofty::picture::{MimeType, Picture, PictureType};
        use lofty::tag::{Accessor, Tag, TagExt, TagType};
        let dir = crate::config::test_dir("am-song");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Song File.mp3");
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor/lame-3.100/testcase.mp3"), &path).unwrap();
        let untagged = read(&path).unwrap();
        assert_eq!((untagged.title.as_str(), untagged.artist.as_str(), untagged.album.as_str()), ("Song File", "Unknown Artist", "Unknown Album"));
        assert!(!untagged.artwork);

        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([30, 90, 200]))).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("Title".into());
        tag.set_artist("Artist".into());
        tag.set_album("Album".into());
        tag.set_track(3);
        tag.set_track_total(9);
        tag.insert_text(ItemKey::UnsyncLyrics, "[00:01.00]Hello".into());
        tag.push_picture(Picture::unchecked(png.clone()).pic_type(PictureType::CoverFront).mime_type(MimeType::Png).build());
        tag.save_to_path(&path, WriteOptions::default()).unwrap();

        let s = read(&path).unwrap();
        assert_eq!((s.title.as_str(), s.artist.as_str(), s.album.as_str()), ("Title", "Artist", "Album"));
        assert_eq!((s.track, s.track_count), (Some(3), Some(9)));
        assert_eq!(s.lyrics, "Hello");
        assert_eq!((s.ext.as_str(), s.audio_format, s.codec), ("mp3", 301, MP3));
        assert!(s.sample_rate > 0.0 && s.bit_rate > 0 && s.file_size > 0);
        assert!(s.artwork);
        assert_eq!(cover(&path), Some(png));
        let (r, g, b) = s.artwork_color.unwrap();
        assert!(b > r && b > g, "{r} {g} {b}");
        assert!(read(&dir.join("x.ogg")).unwrap_err().contains("can't play this kind of file"));
    }

    #[test]
    fn cover_color_is_darkened_and_saturated() {
        let mut img = image::RgbImage::new(8, 8);
        for p in img.pixels_mut() {
            *p = image::Rgb([240, 120, 40]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (r, g, b) = representative_color(&png).unwrap();
        // Orange stays orange, at most 62% bright.
        assert!(r > g && g > b, "{r} {g} {b}");
        assert!(r as f64 <= 0.62 * 255.0 + 1.0);
    }
}
