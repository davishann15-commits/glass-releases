//! The iPhone's media library (iTunes_Control/iTunes/MediaLibrary.sqlitedb), edited
//! on a copy: ByeTunes' MediaLibraryBuilder cut down to adding songs, and a playlist
//! of them. The rows written are ByeTunes' own (they're what makes the Music app
//! list a file as a song in the library); where Glass differs it says so:
//! - a song Glass (or ByeTunes) already put in the library (same title, artist and
//!   album, a file in the music folder) is left alone instead of being written again;
//!   an Apple Music copy of it (a store or cloud item) doesn't count, so the file is
//!   added beside it, and a playlist gets the file;
//! - albums are told apart by album artist as well as by name;
//! - the library's own albums and artists aren't touched (ByeTunes also gave every
//!   album and artist without a sync ID one, and marked it to be kept on the device).

use super::song::Song;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The base location of the songs Glass adds (`iTunes_Control/Music/F00` unless the
/// library says otherwise).
pub const MUSIC_LOCATION: i64 = 3840;

/// Where the placeholder store IDs of songs Glass (and ByeTunes) adds start.
const PLACEHOLDER_IDS: i64 = 9_000_000_000;

/// The iOS version the library belongs to (this iPhone's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

impl Version {
    /// Artwork for local files has its own source type (300) from iOS 26.4 on.
    pub fn local_artwork_source(&self) -> bool {
        self.major > 26 || (self.major == 26 && self.minor >= 4)
    }

    /// `user_version` for a new library (ByeTunes' iOS 26 value).
    fn user_version(&self) -> i64 {
        2_320_030
    }
}

/// A song's place in the library once added: what to upload where.
#[derive(Clone, Debug)]
pub struct Added {
    /// Index into the songs given.
    pub song: usize,
    pub item_pid: i64,
    /// The file's name in the music folder.
    pub file: String,
    /// The cover's path under iTunes_Control/iTunes/Artwork/Originals ("ab/cdef…").
    pub artwork: Option<String>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub added: Vec<Added>,
    /// Songs already in the library: index into the songs given, and its entry.
    pub already: Vec<(usize, i64)>,
}

fn random_pid() -> i64 {
    use rand::Rng;
    rand::thread_rng().gen_range(1_000_000_000_000_000_000..=i64::MAX)
}

/// A random 12-character file name (ByeTunes' `generateRemoteFilename`).
pub fn random_file_name(ext: &str) -> String {
    use rand::Rng;
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    let name: String = (0..12).map(|_| LETTERS[rng.gen_range(0..LETTERS.len())] as char).collect();
    format!("{name}.{ext}")
}

/// The library's sort key for a name: A–Z as 1–26, spaces as 4, slashes as 10, the
/// rest left out (ByeTunes' `generateGroupingKey`).
pub fn grouping_key(text: &str) -> Vec<u8> {
    text.to_uppercase()
        .chars()
        .filter_map(|c| match c {
            'A'..='Z' => Some(c as u8 - b'A' + 1),
            ' ' => Some(0x04),
            '/' => Some(0x0A),
            _ => None,
        })
        .collect()
}

/// A name's first letter as a section (A = 0 … Z = 25), accents folded; 26 for the rest.
fn section(name: &str) -> i64 {
    let c = name.chars().next().map(|c| c.to_uppercase().next().unwrap_or(c)).unwrap_or(' ');
    let folded = match c {
        'À'..='Å' => 'A',
        'Ç' => 'C',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ñ' => 'N',
        'Ò'..='Ö' | 'Ø' => 'O',
        'Ù'..='Ü' => 'U',
        'Ý' => 'Y',
        c => c,
    };
    if folded.is_ascii_uppercase() { (folded as u8 - b'A') as i64 } else { 26 }
}

/// Unknown or empty genres are "Music" (ByeTunes' `canonicalGenre`).
pub fn canonical_genre(raw: &str) -> String {
    let t = raw.trim();
    match t.to_lowercase().as_str() {
        "" | "unknown" | "unknown genre" | "n/a" | "na" | "none" | "null" | "(null)" => "Music".into(),
        _ => t.to_string(),
    }
}

fn column_exists(db: &Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = db.prepare(&format!("PRAGMA table_info({table})")) else { return false };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else { return false };
    rows.flatten().any(|c| c == column)
}

/// The name's order and section in `sort_map`, added at the end when it's new.
fn sort_entry(db: &Connection, name: &str) -> rusqlite::Result<(i64, i64)> {
    if let Some(found) = db.query_row("SELECT name_order, name_section FROM sort_map WHERE name = ?", [name], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(26)))).optional()? {
        return Ok(found);
    }
    let max: i64 = db.query_row("SELECT IFNULL(MAX(name_order), 0) FROM sort_map", [], |r| r.get(0))?;
    let (order, sec) = (max + 1, section(name));
    db.execute("INSERT OR IGNORE INTO sort_map (name, name_order, name_section, sort_key) VALUES (?, ?, ?, ?)", params![name, order, sec, grouping_key(name)])?;
    Ok((order, sec))
}

/// The music folder the library keeps its songs in ("/iTunes_Control/Music/F00").
pub fn music_folder(db: &Connection) -> String {
    let raw: Option<String> = db.query_row("SELECT path FROM base_location WHERE base_location_id = ?", [MUSIC_LOCATION], |r| r.get(0)).optional().ok().flatten();
    raw.map(|p| {
        let p = if p.starts_with('/') { p } else { format!("/{p}") };
        if p == "/iTunes_Control" || p.starts_with("/iTunes_Control/") { p } else { format!("/iTunes_Control{p}") }
    })
    .filter(|p| p.starts_with("/iTunes_Control/Music/"))
    .unwrap_or_else(|| "/iTunes_Control/Music/F00".into())
}

/// Opens a library copied off the device (with its -wal and -shm beside it, if it had
/// them), folds the WAL into it and checks it: an unreadable or damaged library is
/// never written to.
pub fn open_copy(path: &Path) -> Result<Connection, String> {
    let db = Connection::open(path).map_err(|e| format!("the library couldn't be opened ({e})"))?;
    let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(|e| format!("the library couldn't be read ({e})"))?;
    if check != "ok" {
        return Err(format!("the iPhone's music library failed a check ({check}); restart the iPhone and try again"));
    }
    Ok(db)
}

/// Whether a copied library can be read at all (ByeTunes' `hasUsableExistingLibrary`).
pub fn usable(path: &Path) -> bool {
    Connection::open(path).and_then(|db| db.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))).is_ok()
}

/// A new, empty library (an iPhone that never had one).
pub fn create(path: &Path, version: Version) -> Result<Connection, String> {
    let _ = std::fs::remove_file(path);
    let db = Connection::open(path).map_err(|e| e.to_string())?;
    db.execute_batch(&format!("PRAGMA journal_mode=DELETE; PRAGMA encoding='UTF-8'; PRAGMA user_version = {};", version.user_version())).map_err(|e| e.to_string())?;
    db.execute_batch(include_str!("schema.sql")).map_err(|e| format!("the new library couldn't be made ({e})"))?;
    Ok(db)
}

/// Removes the entries of songs whose files aren't in the music folder any more
/// (`files`: what's there), as ByeTunes does before adding.
pub fn remove_ghosts(db: &Connection, files: &HashSet<String>) -> rusqlite::Result<usize> {
    let mut stmt = db.prepare("SELECT item.item_pid, item_extra.location FROM item JOIN item_extra ON item.item_pid = item_extra.item_pid WHERE item.base_location_id = ?")?;
    let ghosts: Vec<i64> = stmt
        .query_map([MUSIC_LOCATION], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .flatten()
        .filter(|(_, loc)| {
            let name = loc.rsplit('/').next().unwrap_or(loc);
            !loc.is_empty() && !files.contains(name)
        })
        .map(|(pid, _)| pid)
        .collect();
    for pid in &ghosts {
        delete_item(db, *pid)?;
    }
    Ok(ghosts.len())
}

fn delete_item(db: &Connection, pid: i64) -> rusqlite::Result<()> {
    db.execute_batch("SAVEPOINT delete_item_rows")?;
    for table in ["item", "item_extra", "item_playback", "item_stats", "item_store", "item_video", "item_search", "lyrics", "chapter"] {
        // A table this iOS doesn't have is skipped.
        let Ok(mut stmt) = db.prepare(&format!("DELETE FROM {table} WHERE item_pid = ?")) else { continue };
        if let Err(e) = stmt.execute([pid]) {
            let _ = db.execute_batch("ROLLBACK TO delete_item_rows; RELEASE delete_item_rows");
            return Err(e);
        }
    }
    db.execute_batch("RELEASE delete_item_rows")
}

/// Every file name the library refers to.
pub fn locations(db: &Connection) -> HashSet<String> {
    let Ok(mut stmt) = db.prepare("SELECT location FROM item_extra") else { return HashSet::new() };
    stmt.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

fn names(db: &Connection, sql: &str) -> HashMap<String, i64> {
    let Ok(mut stmt) = db.prepare(sql) else { return HashMap::new() };
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

/// Every song in the library by "title|artist|album".
/// The songs already added as files (Glass's, ByeTunes', synced ones): "title|artist|album"
/// and their entry. Apple Music's own items aren't among them (no file in the music
/// folder, or a store or iCloud ID): the user's file is added even when Apple Music
/// has the song.
fn signatures(db: &Connection) -> HashMap<String, i64> {
    // A real store ID (Glass's and ByeTunes' songs get placeholders, from 9 000 000 000,
    // which Apple's IDs don't reach), or a subscription song.
    let mut store: Vec<String> = ["store_item_id", "subscription_store_item_id"]
        .iter()
        .filter(|c| column_exists(db, "item_store", c))
        .map(|c| format!("(IFNULL(s.{c}, 0) != 0 AND s.{c} NOT BETWEEN {PLACEHOLDER_IDS} AND {PLACEHOLDER_IDS} + 999999999)"))
        .collect();
    if column_exists(db, "item_store", "is_subscription") {
        store.push("IFNULL(s.is_subscription, 0) != 0".into());
    }
    let not_apple_music = if store.is_empty() { String::new() } else { format!(" AND NOT EXISTS (SELECT 1 FROM item_store s WHERE s.item_pid = item.item_pid AND ({}))", store.join(" OR ")) };
    let Ok(mut stmt) = db.prepare(&format!(
        "SELECT item.item_pid, IFNULL(item_extra.title, ''), IFNULL(item_artist.item_artist, ''), IFNULL(album.album, '') FROM item \
         LEFT JOIN item_extra ON item.item_pid = item_extra.item_pid \
         LEFT JOIN item_artist ON item.item_artist_pid = item_artist.item_artist_pid \
         LEFT JOIN album ON item.album_pid = album.album_pid \
         WHERE item.media_type = 8 AND item.base_location_id = {MUSIC_LOCATION} AND IFNULL(item_extra.location, '') != ''{not_apple_music}",
    )) else {
        return HashMap::new();
    };
    stmt.query_map([], |r| Ok((format!("{}|{}|{}", r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?), r.get::<_, i64>(0)?)))
        .map(|rows| rows.flatten().filter(|(s, _)| !s.starts_with('|')).collect())
        .unwrap_or_default()
}

/// The library's colors for a cover (ByeTunes' `ColorAnalysis` JSON); from iOS 26.4
/// on they're taken from the cover itself.
fn color_analysis(song: &Song, version: Version) -> String {
    let background = if version.local_artwork_source() { song.artwork_color } else { None };
    let Some(bg) = background else {
        return r##"{"ColorAnalysis":{"1":{"primaryTextColorLight":"NO","secondaryTextColorLight":"NO","secondaryTextColor":"#FFFFFF","tertiaryTextColorLight":"NO","primaryTextColor":"#FFFFFF","tertiaryTextColor":"#CCCCCC","backgroundColorLight":"NO","backgroundColor":"#333333"}}}"##.into();
    };
    let light = super::song::luminance(bg) > 0.62;
    let text_light = !light;
    let towards = if text_light { (255, 255, 255) } else { (0, 0, 0) };
    let (a, b, c) = if text_light { (0.92, 0.82, 0.66) } else { (0.86, 0.72, 0.58) };
    let hex = |c: (u8, u8, u8)| format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2);
    let yes = |b: bool| if b { "YES" } else { "NO" };
    format!(
        r#"{{"ColorAnalysis":{{"1":{{"primaryTextColorLight":"{tl}","secondaryTextColorLight":"{tl}","secondaryTextColor":"{s}","tertiaryTextColorLight":"NO","primaryTextColor":"{p}","tertiaryTextColor":"{t}","backgroundColorLight":"{bl}","backgroundColor":"{bg}"}}}}}}"#,
        tl = yes(text_light),
        s = hex(super::song::mix(bg, towards, b)),
        p = hex(super::song::mix(bg, towards, a)),
        t = hex(super::song::mix(bg, towards, c)),
        bl = yes(light),
        bg = hex(bg),
    )
}

/// The cover's path under Artwork/Originals: SHA-1 of its token, split after two characters.
fn artwork_path(token: &str) -> String {
    use sha1::{Digest, Sha1};
    let hash: String = Sha1::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    format!("{}/{}", &hash[..2], &hash[2..])
}

/// Adds `songs` to the library (ByeTunes' `insertSongsWithExisting`). `taken`: file
/// names that mustn't be used (the music folder's and the library's).
pub fn add_songs(db: &Connection, songs: &[Song], version: Version, taken: &HashSet<String>) -> Result<Outcome, String> {
    add(db, songs, version, taken).map_err(|e| format!("the songs couldn't be added to the library ({e})"))
}

fn add(db: &Connection, songs: &[Song], version: Version, taken: &HashSet<String>) -> rusqlite::Result<Outcome> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut out = Outcome::default();
    let mut taken = taken.clone();
    let mut seen = signatures(db);

    let mut artists = names(db, "SELECT item_artist, item_artist_pid FROM item_artist");
    let mut album_artists = names(db, "SELECT album_artist, album_artist_pid FROM album_artist");
    let mut genres = names(db, "SELECT genre, genre_id FROM genre");
    // Albums by (name, album artist).
    let mut albums: HashMap<(String, i64), (i64, i64)> = HashMap::new();
    {
        let mut stmt = db.prepare("SELECT album, album_pid, album_year, album_artist_pid FROM album")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?.flatten() {
            albums.insert((row.0, row.3), (row.1, row.2));
        }
    }

    // New artists, album artists, albums and genres: pid and the first song in them.
    let mut new_artists: Vec<(String, i64, i64)> = Vec::new();
    let mut new_album_artists: Vec<(String, i64, i64)> = Vec::new();
    let mut new_albums: Vec<(String, i64, i64, i64, i64)> = Vec::new();
    let mut new_genres: Vec<(String, i64, i64)> = Vec::new();
    let mut album_years: Vec<(i64, i64)> = Vec::new();
    let mut album_art_done: HashSet<i64> = HashSet::new();

    let variant_artwork = column_exists(db, "artwork", "artwork_variant_type");
    let variant_token = column_exists(db, "artwork_token", "artwork_variant_type");
    let variant_best = column_exists(db, "best_artwork_token", "artwork_variant_type");
    let local_source = version.local_artwork_source() && column_exists(db, "artwork_token", "primary_text_color");
    let catalog_lyrics = column_exists(db, "lyrics", "downloaded_catalog_lyrics_available");
    let aa_sort = column_exists(db, "album_artist", "sort_order");
    let aa_name = column_exists(db, "album_artist", "name_order");
    let album_store = column_exists(db, "album", "store_id");

    for (i, song) in songs.iter().enumerate() {
        let signature = format!("{}|{}|{}", song.title, song.artist, song.album);
        if let Some(&pid) = seen.get(&signature) {
            out.already.push((i, pid));
            continue;
        }
        let item_pid = random_pid();
        seen.insert(signature, item_pid);
        let file = loop {
            let f = random_file_name(&song.ext);
            if taken.insert(f.clone()) {
                break f;
            }
        };

        let artist_pid = *artists.entry(song.artist.clone()).or_insert_with(|| {
            let pid = random_pid();
            new_artists.push((song.artist.clone(), pid, item_pid));
            pid
        });
        let aa_name_str = song.album_artist.clone().unwrap_or_else(|| song.artist.clone());
        let album_artist_pid = *album_artists.entry(aa_name_str.clone()).or_insert_with(|| {
            let pid = random_pid();
            new_album_artists.push((aa_name_str.clone(), pid, item_pid));
            pid
        });
        let album_pid = match albums.get(&(song.album.clone(), album_artist_pid)) {
            Some(&(pid, year)) => {
                if year == 0 && song.year != 0 {
                    album_years.push((pid, song.year));
                    albums.insert((song.album.clone(), album_artist_pid), (pid, song.year));
                }
                pid
            }
            None => {
                let pid = random_pid();
                albums.insert((song.album.clone(), album_artist_pid), (pid, song.year));
                new_albums.push((song.album.clone(), pid, album_artist_pid, song.year, item_pid));
                pid
            }
        };
        let genre = canonical_genre(&song.genre);
        let genre_id = *genres.entry(genre.clone()).or_insert_with(|| {
            let pid = random_pid();
            new_genres.push((genre.clone(), pid, item_pid));
            pid
        });

        let title_sort = sort_entry(db, &song.title)?;
        let artist_sort = sort_entry(db, &song.artist)?;
        let album_sort = sort_entry(db, &song.album)?;
        let genre_sort = sort_entry(db, &genre)?;
        sort_entry(db, &aa_name_str)?;

        let added = song.added.unwrap_or(now);
        db.execute(
            "INSERT INTO item (item_pid, media_type, title_order, title_order_section, item_artist_pid, item_artist_order, item_artist_order_section, \
             series_name_order, series_name_order_section, album_pid, album_order, album_order_section, album_artist_pid, album_artist_order, album_artist_order_section, \
             composer_pid, composer_order, composer_order_section, genre_id, genre_order, genre_order_section, disc_number, track_number, episode_sort_id, \
             base_location_id, remote_location_id, exclude_from_shuffle, keep_local, keep_local_status, keep_local_status_reason, keep_local_constraints, \
             in_my_library, is_compilation, date_added, show_composer, is_music_show, date_downloaded, download_source_container_pid) \
             VALUES (?1, 8, ?2, ?3, ?4, ?5, ?6, 0, 26, ?7, ?8, ?9, ?10, ?5, ?6, 0, 0, 26, ?11, ?12, ?13, ?14, ?15, 1, ?16, 0, 0, 1, 2, 0, 0, 1, 0, ?17, 0, 0, ?17, 0)",
            params![
                item_pid,
                title_sort.0,
                title_sort.1,
                artist_pid,
                artist_sort.0,
                artist_sort.1,
                album_pid,
                album_sort.0,
                album_sort.1,
                album_artist_pid,
                genre_id,
                genre_sort.0,
                genre_sort.1,
                song.disc.unwrap_or(1),
                song.track.unwrap_or(0),
                MUSIC_LOCATION,
                added
            ],
        )?;
        db.execute(
            "INSERT INTO item_extra (item_pid, title, sort_title, disc_count, track_count, total_time_ms, year, location, file_size, integrity, \
             is_audible_audio_book, date_modified, media_kind, content_rating, content_rating_level, is_user_disabled, bpm, genius_id, location_kind_id, copyright) \
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, X'', 0, ?9, 1, ?10, 0, 0, 0, 0, 42, ?11)",
            params![item_pid, song.title, song.disc_count.unwrap_or(1), song.track_count.unwrap_or(0), song.duration_ms, song.year, file, song.file_size, now, song.explicit, song.copyright],
        )?;
        db.execute(
            "INSERT INTO item_playback (item_pid, audio_format, bit_rate, codec_type, codec_subtype, data_kind, duration, has_video, relative_volume, sample_rate) \
             VALUES (?, ?, ?, ?, 0, 0, 0, 0, 0, ?)",
            params![item_pid, song.audio_format, song.bit_rate, song.codec, song.sample_rate],
        )?;
        db.execute("INSERT OR REPLACE INTO item_stats (item_pid, date_accessed) VALUES (?, ?)", params![item_pid, now])?;
        // ByeTunes' store fields for a song with no Apple catalog match: placeholder store
        // IDs and cloud_in_my_library make the library's triggers list it as in the library.
        let placeholder = PLACEHOLDER_IDS + (item_pid % 900_000_000).abs();
        db.execute(
            "INSERT OR REPLACE INTO item_store (item_pid, sync_id, sync_in_my_library, is_subscription, store_saga_id, match_redownload_params, cloud_status, \
             store_xid, store_item_id, storefront_id, store_composer_id, store_genre_id, store_playlist_id, date_released, subscription_store_item_id, \
             is_mastered_for_itunes, store_flavor, cloud_asset_available, cloud_in_my_library, playback_endpoint_type, cloud_playback_endpoint_type) \
             VALUES (?1, ?2, 1, 0, ?3, '', 8, '', ?3, 143441, 0, 0, ?3, 0, ?3, 0, '', 0, 1, 0, 0)",
            params![item_pid, random_pid(), placeholder],
        )?;
        db.execute("INSERT OR REPLACE INTO item_video (item_pid, hls_asset_traits) VALUES (?, 0)", [item_pid])?;
        db.execute(
            "INSERT OR REPLACE INTO item_search (item_pid, search_title, search_album, search_artist, search_composer, search_album_artist) VALUES (?, ?, ?, ?, 0, ?)",
            params![item_pid, title_sort.0, album_sort.0, artist_sort.0, artist_sort.0],
        )?;
        if catalog_lyrics {
            db.execute("INSERT OR REPLACE INTO lyrics (item_pid, lyrics, store_lyrics_available, time_synced_lyrics_available, downloaded_catalog_lyrics_available) VALUES (?, ?, 1, 1, 0)", params![item_pid, song.lyrics])?;
        } else {
            db.execute("INSERT OR REPLACE INTO lyrics (item_pid, lyrics, store_lyrics_available, time_synced_lyrics_available) VALUES (?, ?, 1, 1)", params![item_pid, song.lyrics])?;
        }
        db.execute("INSERT OR REPLACE INTO chapter (item_pid) VALUES (?)", [item_pid])?;

        let mut artwork = None;
        if song.artwork {
            let token = item_pid.to_string();
            let path = artwork_path(&token);
            let colors = color_analysis(song, version);
            if variant_artwork {
                db.execute("INSERT OR REPLACE INTO artwork (artwork_token, artwork_source_type, relative_path, artwork_type, interest_data, artwork_variant_type) VALUES (?, 1, ?, 1, ?, 0)", params![token, path, colors])?;
            } else {
                db.execute("INSERT OR REPLACE INTO artwork (artwork_token, artwork_source_type, relative_path, artwork_type, interest_data) VALUES (?, 1, ?, 1, ?)", params![token, path, colors])?;
            }
            // The song, its album (as 1 and 4) and its artist show this cover.
            for (entity, kind) in [(item_pid, 0), (album_pid, 1), (album_pid, 4), (artist_pid, 2)] {
                if variant_token {
                    db.execute("INSERT OR REPLACE INTO artwork_token (artwork_token, artwork_source_type, artwork_type, entity_pid, entity_type, artwork_variant_type) VALUES (?, 1, 1, ?, ?, 0)", params![token, entity, kind])?;
                } else {
                    db.execute("INSERT OR REPLACE INTO artwork_token (artwork_token, artwork_source_type, artwork_type, entity_pid, entity_type) VALUES (?, 1, 1, ?, ?)", params![token, entity, kind])?;
                }
            }
            let first_of_album = album_art_done.insert(album_pid);
            let mut best = vec![(item_pid, 0)];
            if first_of_album {
                best.extend([(album_pid, 1), (album_pid, 4)]);
            }
            best.push((artist_pid, 2));
            for (entity, kind) in best {
                if variant_best {
                    db.execute("INSERT OR REPLACE INTO best_artwork_token (entity_pid, entity_type, artwork_type, available_artwork_token, fetchable_artwork_token, fetchable_artwork_source_type, artwork_variant_type) VALUES (?, ?, 1, ?, '', 0, 0)", params![entity, kind, token])?;
                } else {
                    db.execute("INSERT OR REPLACE INTO best_artwork_token (entity_pid, entity_type, artwork_type, available_artwork_token, fetchable_artwork_token, fetchable_artwork_source_type) VALUES (?, ?, 1, ?, '', 0)", params![entity, kind, token])?;
                }
            }
            if local_source {
                db.execute("INSERT OR REPLACE INTO artwork (artwork_token, artwork_source_type, relative_path, artwork_type, interest_data, artwork_variant_type) VALUES (?, 300, ?, 6, ?, 0)", params![token, path, colors])?;
                db.execute("INSERT OR REPLACE INTO artwork_token (artwork_token, artwork_source_type, artwork_type, entity_pid, entity_type, artwork_variant_type) VALUES (?, 300, 1, ?, 0, 0)", params![token, item_pid])?;
                db.execute("INSERT OR REPLACE INTO best_artwork_token (entity_pid, entity_type, artwork_type, available_artwork_token, fetchable_artwork_token, fetchable_artwork_source_type, artwork_variant_type) VALUES (?, 0, 1, ?, '', 0, 0)", params![item_pid, token])?;
                db.execute("INSERT OR REPLACE INTO artwork_token (artwork_token, artwork_source_type, artwork_type, entity_pid, entity_type, artwork_variant_type) VALUES (?, 300, 6, ?, 4, 0)", params![token, album_pid])?;
                db.execute("INSERT OR REPLACE INTO best_artwork_token (entity_pid, entity_type, artwork_type, available_artwork_token, fetchable_artwork_token, fetchable_artwork_source_type, artwork_variant_type) VALUES (?, 4, 6, ?, '', 0, 0)", params![album_pid, token])?;
            }
            artwork = Some(path);
        }
        out.added.push(Added { song: i, item_pid, file, artwork });
    }

    for (name, pid, rep) in &new_artists {
        db.execute(
            "INSERT INTO item_artist (item_artist_pid, item_artist, sort_item_artist, series_name, grouping_key, sync_id, keep_local, representative_item_pid, store_id) VALUES (?1, ?2, ?2, '', ?3, ?4, 1, ?5, 0)",
            params![pid, name, grouping_key(name), random_pid(), rep],
        )?;
    }
    for (name, pid, rep) in &new_album_artists {
        let (order, sec) = sort_entry(db, name)?;
        if aa_sort && aa_name {
            db.execute(
                "INSERT INTO album_artist (album_artist_pid, album_artist, sort_album_artist, grouping_key, sync_id, keep_local, representative_item_pid, sort_order, sort_order_section, name_order, store_id) VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?6, 0)",
                params![pid, name, grouping_key(name), random_pid(), rep, order, sec],
            )?;
        } else if aa_sort {
            db.execute(
                "INSERT INTO album_artist (album_artist_pid, album_artist, sort_album_artist, grouping_key, sync_id, keep_local, representative_item_pid, sort_order, sort_order_section, store_id) VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, ?6, ?7, 0)",
                params![pid, name, grouping_key(name), random_pid(), rep, order, sec],
            )?;
        } else {
            db.execute(
                "INSERT INTO album_artist (album_artist_pid, album_artist, sort_album_artist, grouping_key, sync_id, keep_local, representative_item_pid, store_id) VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, 0)",
                params![pid, name, grouping_key(name), random_pid(), rep],
            )?;
        }
    }
    for (name, pid, aa_pid, year, rep) in &new_albums {
        let year = albums.get(&(name.clone(), *aa_pid)).map(|a| a.1).unwrap_or(*year);
        if album_store {
            db.execute(
                "INSERT INTO album (album_pid, album, sort_album, album_artist_pid, grouping_key, album_year, keep_local, sync_id, representative_item_pid, store_id) VALUES (?1, ?2, ?2, ?3, ?4, ?5, 1, ?6, ?7, 0)",
                params![pid, name, aa_pid, grouping_key(name), year, random_pid(), rep],
            )?;
        } else {
            db.execute(
                "INSERT INTO album (album_pid, album, sort_album, album_artist_pid, grouping_key, album_year, keep_local, sync_id, representative_item_pid) VALUES (?1, ?2, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
                params![pid, name, aa_pid, grouping_key(name), year, random_pid(), rep],
            )?;
        }
    }
    for (pid, year) in &album_years {
        db.execute("UPDATE album SET album_year = ? WHERE album_pid = ? AND album_year = 0", params![year, pid])?;
    }
    for (name, pid, rep) in &new_genres {
        db.execute("INSERT INTO genre (genre_id, genre, grouping_key, representative_item_pid) VALUES (?, ?, ?, ?)", params![pid, name, grouping_key(name), rep])?;
    }
    Ok(out)
}

/// Adds the songs to the playlist called `name` (made when there's none; songs it
/// has already aren't added again). ByeTunes' `createPlaylist` / `addToPlaylist`.
pub fn add_to_playlist(db: &Connection, name: &str, pids: &[i64]) -> Result<(), String> {
    playlist(db, name, pids).map_err(|e| format!("the playlist couldn't be made ({e})"))
}

/// A song's "title|artist|album" in SQL, over `ci_item` (an item) joined as below.
const SIGNATURE: &str = "IFNULL(e.title, '') || '|' || IFNULL(a.item_artist, '') || '|' || IFNULL(al.album, '')";
const SIGNATURE_ITEM_JOINS: &str = "LEFT JOIN item_extra e ON e.item_pid = ci_item.item_pid LEFT JOIN item_artist a ON a.item_artist_pid = ci_item.item_artist_pid LEFT JOIN album al ON al.album_pid = ci_item.album_pid";
const SIGNATURE_JOINS: &str = "JOIN item ci_item ON ci_item.item_pid = ci.item_pid LEFT JOIN item_extra e ON e.item_pid = ci_item.item_pid LEFT JOIN item_artist a ON a.item_artist_pid = ci_item.item_artist_pid LEFT JOIN album al ON al.album_pid = ci_item.album_pid";

fn playlist(db: &Connection, name: &str, pids: &[i64]) -> rusqlite::Result<()> {
    use rand::Rng;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let existing: Option<i64> = db
        .query_row("SELECT container_pid FROM container WHERE name = ? AND contained_media_type = 8 AND distinguished_kind = 0 AND smart_criteria IS NULL LIMIT 1", [name], |r| r.get(0))
        .optional()?;
    let container = match existing {
        Some(pid) => pid,
        None => {
            let pid = rand::thread_rng().gen_range(1_000_000_000..=9_999_999_999_999i64);
            let (order, _) = sort_entry(db, name)?;
            db.execute(
                "INSERT INTO container (container_pid, name, name_order, date_created, date_modified, contained_media_type, is_owner, is_editable, distinguished_kind) VALUES (?, ?, ?, ?, ?, 8, 1, 1, 0)",
                params![pid, name, order, now, now],
            )?;
            pid
        }
    };
    let mut have: HashSet<i64> = {
        let mut stmt = db.prepare("SELECT item_pid FROM container_item WHERE container_pid = ?")?;
        stmt.query_map([container], |r| r.get(0))?.flatten().collect()
    };
    // The playlist's songs that aren't being added ("title|artist|album" → entry): an
    // Apple Music copy of a song added now gives its place to the file.
    let mut others: HashMap<String, i64> = {
        let mut stmt = db.prepare(&format!("SELECT ci.container_item_pid, {SIGNATURE} FROM container_item ci {SIGNATURE_JOINS} WHERE ci.container_pid = ?"))?;
        stmt.query_map([container], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(0)?)))?.flatten().collect()
    };
    let wanted: HashSet<i64> = pids.iter().copied().collect();
    let mut position: i64 = db.query_row("SELECT IFNULL(MAX(position), -1) FROM container_item WHERE container_pid = ?", [container], |r| r.get(0))?;
    for pid in pids {
        if !have.insert(*pid) {
            continue;
        }
        let signature: Option<String> = db.query_row(&format!("SELECT {SIGNATURE} FROM item ci_item {SIGNATURE_ITEM_JOINS} WHERE ci_item.item_pid = ?"), [pid], |r| r.get(0)).optional()?;
        if let Some(entry) = signature.and_then(|sig| others.remove(&sig)) {
            let old: i64 = db.query_row("SELECT item_pid FROM container_item WHERE container_item_pid = ?", [entry], |r| r.get(0))?;
            if !wanted.contains(&old) {
                db.execute("UPDATE container_item SET item_pid = ? WHERE container_item_pid = ?", params![pid, entry])?;
                continue;
            }
        }
        position += 1;
        let item = rand::thread_rng().gen_range(1_000_000_000..=9_999_999_999_999i64);
        db.execute(
            "INSERT INTO container_item (container_item_pid, container_pid, item_pid, position, uuid) VALUES (?, ?, ?, ?, ?)",
            params![item, container, pid, position, uuid::Uuid::new_v4().to_string().to_uppercase()],
        )?;
    }
    if existing.is_some() {
        db.execute("UPDATE container SET date_modified = ? WHERE container_pid = ?", params![now, container])?;
    }
    Ok(())
}

/// Puts `sort_map` back in alphabetical order and renumbers every song's sort columns
/// to match (new names were added at the end): ByeTunes' `repairAlphabeticalOrdering`,
/// run after every addition. Leaves the file ready to upload (no WAL).
pub fn repair_order(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "UPDATE sort_map SET name_section = CASE
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) BETWEEN 65 AND 90 THEN UNICODE(UPPER(SUBSTR(name, 1, 1))) - 65
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) BETWEEN 192 AND 197 THEN 0
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) = 199 THEN 2
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) BETWEEN 200 AND 203 THEN 4
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) BETWEEN 204 AND 207 THEN 8
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) = 209 THEN 13
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) BETWEEN 210 AND 214 THEN 14
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) BETWEEN 217 AND 220 THEN 20
            WHEN UNICODE(UPPER(SUBSTR(name, 1, 1))) = 221 THEN 24
            ELSE 26 END;
        DROP TABLE IF EXISTS _sort_reorder;
        CREATE TEMP TABLE _sort_reorder AS
            SELECT name, name_order AS old_order, name_section,
                   ROW_NUMBER() OVER (ORDER BY CASE name_section WHEN 26 THEN -1 ELSE name_section END ASC, sort_key ASC) AS new_order
            FROM sort_map;
        UPDATE item SET
            title_order = COALESCE((SELECT new_order FROM _sort_reorder WHERE old_order = item.title_order), title_order),
            title_order_section = COALESCE((SELECT name_section FROM _sort_reorder WHERE old_order = item.title_order), title_order_section),
            item_artist_order = COALESCE((SELECT new_order FROM _sort_reorder WHERE old_order = item.item_artist_order), item_artist_order),
            item_artist_order_section = COALESCE((SELECT name_section FROM _sort_reorder WHERE old_order = item.item_artist_order), item_artist_order_section),
            album_order = COALESCE((SELECT new_order FROM _sort_reorder WHERE old_order = item.album_order), album_order),
            album_order_section = COALESCE((SELECT name_section FROM _sort_reorder WHERE old_order = item.album_order), album_order_section),
            album_artist_order = COALESCE((SELECT new_order FROM _sort_reorder WHERE old_order = item.album_artist_order), album_artist_order),
            album_artist_order_section = COALESCE((SELECT name_section FROM _sort_reorder WHERE old_order = item.album_artist_order), album_artist_order_section),
            genre_order = COALESCE((SELECT new_order FROM _sort_reorder WHERE old_order = item.genre_order), genre_order),
            genre_order_section = COALESCE((SELECT name_section FROM _sort_reorder WHERE old_order = item.genre_order), genre_order_section);
        UPDATE sort_map SET name_order = -(name_order + 1);
        UPDATE sort_map SET name_order = (SELECT new_order FROM _sort_reorder WHERE _sort_reorder.name = sort_map.name);
        UPDATE item_search SET
            search_title = (SELECT title_order FROM item WHERE item.item_pid = item_search.item_pid),
            search_album = (SELECT album_order FROM item WHERE item.item_pid = item_search.item_pid),
            search_artist = (SELECT item_artist_order FROM item WHERE item.item_pid = item_search.item_pid),
            search_album_artist = (SELECT album_artist_order FROM item WHERE item.item_pid = item_search.item_pid);
        DROP TABLE IF EXISTS _sort_reorder;",
    )
    .map_err(|e| format!("the library's alphabetical order couldn't be repaired ({e})"))?;
    finish(db)
}

/// Folds any WAL back into the file and leaves it in rollback-journal mode, one file.
pub fn finish(db: &Connection) -> Result<(), String> {
    let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    db.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get::<_, String>(0)).map(|_| ()).map_err(|e| format!("the library couldn't be saved ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const V: Version = Version { major: 26, minor: 4 };

    fn song(title: &str, artist: &str, album: &str, track: u32) -> Song {
        Song {
            path: PathBuf::from(format!("/music/{title}.m4a")),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: None,
            genre: String::new(),
            year: 2019,
            duration_ms: 200_000,
            file_size: 4_000_000,
            ext: "m4a".into(),
            track: Some(track),
            track_count: Some(10),
            disc: None,
            disc_count: None,
            lyrics: String::new(),
            copyright: String::new(),
            explicit: 0,
            audio_format: 0x6161_6320,
            codec: 0x6161_6320,
            bit_rate: 256,
            sample_rate: 44_100.0,
            artwork: true,
            artwork_color: Some((120, 60, 30)),
            added: Some(1_700_000_000),
        }
    }

    fn fresh(name: &str) -> (Connection, PathBuf) {
        let dir = crate::config::test_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("MediaLibrary.sqlitedb");
        (create(&path, V).unwrap(), path)
    }

    fn count(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn songs_land_in_a_new_library() {
        let (db, _) = fresh("am-new");
        // iOS 26.4's libraries have text colors on artwork tokens (ByeTunes' schema doesn't).
        db.execute_batch("ALTER TABLE artwork_token ADD COLUMN primary_text_color TEXT NOT NULL DEFAULT ''").unwrap();
        let songs = [song("Zebra", "Band", "First", 2), song("apple", "Band", "First", 1), song("Mango", "Other", "Second", 1)];
        let out = add_songs(&db, &songs, V, &HashSet::new()).unwrap();
        assert_eq!(out.added.len(), 3);
        assert!(out.already.is_empty());
        assert_eq!(count(&db, "SELECT COUNT(*) FROM item WHERE media_type = 8 AND base_location_id = 3840"), 3);
        // The store triggers put them in the library.
        assert_eq!(count(&db, "SELECT COUNT(*) FROM item WHERE in_my_library = 1"), 3);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM album"), 2);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM item_artist"), 2);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM album_artist"), 2);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM genre WHERE genre = 'Music'"), 1);
        // Covers: the song's own rows, and local-source rows from iOS 26.4.
        assert_eq!(count(&db, "SELECT COUNT(*) FROM artwork WHERE artwork_source_type = 1"), 3);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM artwork WHERE artwork_source_type = 300"), 3);
        let a = &out.added[0];
        let rel = a.artwork.clone().unwrap();
        assert_eq!(rel, artwork_path(&a.item_pid.to_string()));
        assert_eq!(rel.len(), 41);
        let interest: String = db.query_row("SELECT interest_data FROM artwork WHERE relative_path = ? AND artwork_source_type = 1", [&rel], |r| r.get(0)).unwrap();
        assert!(interest.contains("\"backgroundColor\":\"#"), "{interest}");
        // File names: 12 characters and the extension, all different.
        let files: HashSet<_> = out.added.iter().map(|a| a.file.clone()).collect();
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|f| f.len() == 16 && f.ends_with(".m4a")));
        assert_eq!(locations(&db), files);
    }

    #[test]
    fn order_is_alphabetical_after_repair() {
        let (db, _) = fresh("am-order");
        add_songs(&db, &[song("Zebra", "B", "X", 1), song("apple", "B", "X", 2), song("Mango", "B", "X", 3)], V, &HashSet::new()).unwrap();
        repair_order(&db).unwrap();
        let titles: Vec<String> = db
            .prepare("SELECT item_extra.title FROM item JOIN item_extra USING (item_pid) ORDER BY item.title_order")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(titles, ["apple", "Mango", "Zebra"]);
        let sections: Vec<i64> = db.prepare("SELECT title_order_section FROM item ORDER BY title_order").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
        assert_eq!(sections, [0, 12, 25]);
        let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "delete");
    }

    #[test]
    fn an_apple_music_copy_doesnt_keep_the_file_out() {
        let (db, _) = fresh("am-catalog");
        let first = add_songs(&db, &[song("Sailing", "Christopher Cross", "Christopher Cross", 1)], V, &HashSet::new()).unwrap();
        let pid = first.added[0].item_pid;
        // Make that entry Apple Music's: a catalog song in the library, no file of the user's.
        db.execute("UPDATE item SET base_location_id = 0 WHERE item_pid = ?", [pid]).unwrap();
        db.execute("UPDATE item_store SET store_item_id = 1440765584, is_subscription = 1 WHERE item_pid = ?", [pid]).unwrap();
        let again = add_songs(&db, &[song("Sailing", "Christopher Cross", "Christopher Cross", 1)], V, &locations(&db)).unwrap();
        assert!(again.already.is_empty(), "Apple Music's copy isn't the user's file");
        assert_eq!(again.added.len(), 1);
        // A downloaded Apple Music song (a file, but a store item) doesn't count either.
        let (db, _) = fresh("am-downloaded");
        let first = add_songs(&db, &[song("Sailing", "Christopher Cross", "Christopher Cross", 1)], V, &HashSet::new()).unwrap();
        db.execute("UPDATE item_store SET store_item_id = 1440765584 WHERE item_pid = ?", [first.added[0].item_pid]).unwrap();
        let again = add_songs(&db, &[song("Sailing", "Christopher Cross", "Christopher Cross", 1)], V, &locations(&db)).unwrap();
        assert_eq!(again.added.len(), 1);
    }

    #[test]
    fn the_file_takes_its_apple_music_copys_place_in_a_playlist() {
        let (db, _) = fresh("am-swap");
        let first = add_songs(&db, &[song("Sailing", "Christopher Cross", "Christopher Cross", 1), song("Babe", "Styx", "Cornerstone", 2)], V, &HashSet::new()).unwrap();
        let (copy, babe) = (first.added[0].item_pid, first.added[1].item_pid);
        add_to_playlist(&db, "Yacht Rock", &[copy, babe]).unwrap();
        // "Sailing" there is Apple Music's (as when the playlist was made with Apple Music's songs).
        db.execute("UPDATE item SET base_location_id = 0 WHERE item_pid = ?", [copy]).unwrap();
        db.execute("UPDATE item_store SET store_item_id = 1440765584, is_subscription = 1 WHERE item_pid = ?", [copy]).unwrap();
        let again = add_songs(&db, &[song("Sailing", "Christopher Cross", "Christopher Cross", 1), song("Babe", "Styx", "Cornerstone", 2)], V, &locations(&db)).unwrap();
        assert_eq!(again.added.len(), 1, "only Sailing's file is new");
        let file = again.added[0].item_pid;
        add_to_playlist(&db, "Yacht Rock", &[file, babe]).unwrap();
        let order: Vec<i64> = {
            let mut stmt = db.prepare("SELECT item_pid FROM container_item ORDER BY position").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().flatten().collect()
        };
        assert_eq!(order, vec![file, babe], "the file in the copy's place, no duplicate");
    }

    #[test]
    fn a_song_already_there_is_left_alone() {
        let (db, _) = fresh("am-again");
        let first = add_songs(&db, &[song("One", "A", "X", 1)], V, &HashSet::new()).unwrap();
        let again = add_songs(&db, &[song("One", "A", "X", 1), song("Two", "A", "X", 2)], V, &locations(&db)).unwrap();
        assert_eq!(again.already, vec![(0, first.added[0].item_pid)]);
        assert_eq!(again.added.len(), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM item"), 2);
        // Same album, same artist: no second album.
        assert_eq!(count(&db, "SELECT COUNT(*) FROM album"), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM item_artist"), 1);
    }

    #[test]
    fn albums_are_told_apart_by_album_artist() {
        let (db, _) = fresh("am-albums");
        add_songs(&db, &[song("A", "One", "Greatest Hits", 1), song("B", "Two", "Greatest Hits", 1)], V, &HashSet::new()).unwrap();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM album"), 2);
        // Without that column (before 26.4) only the song's own cover rows are written.
        assert_eq!(count(&db, "SELECT COUNT(*) FROM artwork WHERE artwork_source_type = 300"), 0);
    }

    #[test]
    fn playlists_are_made_once_and_added_to() {
        let (db, _) = fresh("am-playlist");
        let out = add_songs(&db, &[song("A", "X", "Y", 1), song("B", "X", "Y", 2)], V, &HashSet::new()).unwrap();
        let pids: Vec<i64> = out.added.iter().map(|a| a.item_pid).collect();
        add_to_playlist(&db, "Road trip", &pids).unwrap();
        add_to_playlist(&db, "Road trip", &pids[1..]).unwrap();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM container WHERE name = 'Road trip'"), 1);
        let order: Vec<i64> = db.prepare("SELECT item_pid FROM container_item ORDER BY position").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
        assert_eq!(order, pids);
    }

    #[test]
    fn ghosts_are_removed_and_the_music_folder_is_read() {
        let (db, path) = fresh("am-ghosts");
        let out = add_songs(&db, &[song("Kept", "A", "X", 1), song("Gone", "A", "X", 2)], V, &HashSet::new()).unwrap();
        finish(&db).unwrap();
        drop(db);
        let db = open_copy(&path).unwrap();
        assert_eq!(music_folder(&db), "/iTunes_Control/Music/F00");
        let there: HashSet<String> = [out.added[0].file.clone()].into();
        assert_eq!(remove_ghosts(&db, &there).unwrap(), 1);
        let left: String = db.query_row("SELECT title FROM item_extra", [], |r| r.get(0)).unwrap();
        assert_eq!(left, "Kept");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM item"), 1);
    }

    #[test]
    fn a_damaged_library_is_refused() {
        let dir = crate::config::test_dir("am-damaged");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("MediaLibrary.sqlitedb");
        std::fs::write(&path, b"not a database at all").unwrap();
        assert!(!usable(&path));
        assert!(open_copy(&path).is_err());
    }

    #[test]
    fn names_and_genres() {
        assert_eq!(grouping_key("AC/DC rocks"), vec![1, 3, 10, 4, 3, 4, 18, 15, 3, 11, 19]);
        assert_eq!(section("Élan"), 4);
        assert_eq!(section("42"), 26);
        assert_eq!(canonical_genre("  unknown "), "Music");
        assert_eq!(canonical_genre("Jazz"), "Jazz");
    }
}
