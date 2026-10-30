//! Deciding whether a lyrics record belongs to the recording that's playing.
//!
//! Title, artist, album and duration all count. Version markers in titles are
//! handled with care: a remaster or a "feat." credit is the same performance
//! (same words, same timing), but a live take, a radio edit, a remix or an
//! acoustic version isn't — its lyrics are timed differently or even differ —
//! so those must agree on both sides. The duration has to be within a few
//! seconds for synced lyrics to line up; anything further off is only offered
//! in the manual picker.

use super::TrackRef;
use super::parse::Record;
use std::collections::BTreeSet;

/// Markers that make a different recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    Live,
    Acoustic,
    Remix,
    Edit,
    Extended,
    Instrumental,
    Demo,
    SpedUp,
    Slowed,
    Karaoke,
    Acapella,
    ReRecorded,
}

/// A title split into its words and the version markers it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Title {
    pub base: String,
    pub versions: BTreeSet<Version>,
}

/// Lowercase letters and digits from any script, single-spaced.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c == '&' {
            if !out.is_empty() {
                out.push_str(" and");
            }
            space = true;
        } else if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else if c != '\'' && c != '’' {
            // Apostrophes join ("don't" = "dont"); everything else separates.
            space = true;
        }
    }
    out
}

fn has_word(text: &str, word: &str) -> bool {
    let t = format!(" {text} ");
    t.contains(&format!(" {word} "))
}

/// Version markers in one parenthetical or dash segment. `None` when the
/// segment isn't about versions at all (it's part of the title).
fn classify(segment: &str) -> Option<BTreeSet<Version>> {
    let s = normalize(segment);
    let mut v = BTreeSet::new();
    let year = s.split(' ').any(|w| w.len() == 4 && w.chars().all(|c| c.is_ascii_digit()));
    let neutral = [
        "remaster",
        "remastered",
        "mono",
        "stereo",
        "deluxe",
        "explicit",
        "clean",
        "bonus track",
        "feat",
        "ft",
        "featuring",
        "with",
        "from",
        "single version",
        "album version",
        "original",
        "version",
        "edition",
        "anniversary",
        "mixed",
        "prod",
        "bonus",
    ];
    let rules: &[(&[&str], Version)] = &[
        (&["live", "en vivo", "ao vivo", "en directo", "live at", "live from"], Version::Live),
        (&["acoustic", "unplugged", "stripped"], Version::Acoustic),
        (&["remix", "mix", "rmx", "vip", "bootleg", "rework", "flip"], Version::Remix),
        (&["edit", "radio edit", "single edit", "clean edit"], Version::Edit),
        (&["extended", "extended version", "club version"], Version::Extended),
        (&["instrumental", "inst", "backing track"], Version::Instrumental),
        (&["demo"], Version::Demo),
        (&["sped up", "speed up", "nightcore"], Version::SpedUp),
        (&["slowed", "reverb"], Version::Slowed),
        (&["karaoke"], Version::Karaoke),
        (&["acapella", "a cappella", "a capella", "vocals only"], Version::Acapella),
        (&["taylor s version", "taylors version", "re recorded", "rerecorded", "re record"], Version::ReRecorded),
    ];
    for (words, version) in rules {
        if words.iter().any(|w| has_word(&s, w)) {
            v.insert(*version);
        }
    }
    // "Remastered", "mono", "deluxe", "feat. X", "2011", "from the film"…:
    // the same recording.
    // "2019 Mix", "Stereo Mix", "Remastered & Mixed": a new mix of the same
    // take, not a remix.
    if v.contains(&Version::Remix)
        && !has_word(&s, "remix")
        && !has_word(&s, "rmx")
        && (year || ["stereo", "mono", "mixed", "remaster", "remastered", "original", "album"].iter().any(|w| has_word(&s, w)))
    {
        v.remove(&Version::Remix);
    }
    if !v.is_empty() {
        return Some(v);
    }
    (year || neutral.iter().any(|w| has_word(&s, w))).then_some(v)
}

/// Splits "Song (Live at X) - 2011 Remaster" into "song" and {Live}.
pub fn title(s: &str) -> Title {
    let mut base = String::new();
    let mut versions = BTreeSet::new();
    let mut rest = s;
    // Parenthetical and bracketed groups.
    loop {
        let Some(open) = rest.find(['(', '[']) else {
            base.push_str(rest);
            break;
        };
        let close_char = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close_char) else {
            base.push_str(rest);
            break;
        };
        let inner = &rest[open + 1..open + 1 + len];
        base.push_str(&rest[..open]);
        match classify(inner) {
            Some(v) => versions.extend(v),
            None => {
                base.push(' ');
                base.push_str(inner);
            }
        }
        rest = &rest[open + 1 + len + 1..];
    }
    // " - Live at Wembley", " - 2011 Remaster", " - Radio Edit".
    let mut parts: Vec<&str> = base.split(" - ").collect();
    while parts.len() > 1 {
        match classify(parts[parts.len() - 1]) {
            Some(v) => {
                versions.extend(v);
                parts.pop();
            }
            None => break,
        }
    }
    Title { base: normalize(&parts.join(" - ")), versions }
}

/// Album names lose their edition markers ("(Deluxe)", "(Remastered 2011)").
fn album(s: &str) -> String {
    let t = title(s);
    t.base
}

fn words(s: &str) -> BTreeSet<&str> {
    s.split(' ').filter(|w| !w.is_empty()).collect()
}

fn overlap(a: &str, b: &str) -> f32 {
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(&b).count() as f32 / a.len().min(b.len()) as f32
}

/// Whether a name is written in Latin letters (`Some(true)`), in another
/// script only (`Some(false)`), or mixes them or has no letters (`None`).
fn latin(s: &str) -> Option<bool> {
    let (mut latin, mut other) = (false, false);
    for c in s.chars().filter(|c| c.is_alphabetic()) {
        if c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c) || ('\u{1E00}'..='\u{1EFF}').contains(&c) {
            latin = true;
        } else {
            other = true;
        }
    }
    match (latin, other) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// The same name in two scripts can't be compared ("Anri" and "杏里"):
/// one is in Latin letters, the other has none.
fn other_script(a: &str, b: &str) -> bool {
    matches!((latin(a), latin(b)), (Some(x), Some(y)) if x != y)
}

/// Seconds apart that still count as the same recording.
pub const SAME_RECORDING_S: f32 = 3.0;

/// What a provider says about a recording, as far as matching cares.
#[derive(Clone, Copy, Debug, Default)]
pub struct Info<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    /// 0 when the provider doesn't say.
    pub duration_ms: u64,
    pub instrumental: bool,
    /// Line timing at least.
    pub synced: bool,
    /// Word timing.
    pub words: bool,
}

impl Record {
    pub fn info(&self) -> Info<'_> {
        Info {
            title: &self.track_name,
            artist: &self.artist_name,
            album: &self.album_name,
            duration_ms: self.duration_ms,
            instrumental: self.instrumental,
            synced: self.synced.is_some() || self.lyricsfile.as_deref().is_some_and(|f| f.contains("start_ms")),
            words: self.has_word_sync,
        }
    }
}

/// How well an LRCLIB record fits. `None` means it's a different recording
/// (or a different song) and must not be chosen automatically.
pub fn score(q: &TrackRef, r: &Record) -> Option<f32> {
    score_info(q, &r.info())
}

/// How well a recording fits (see [`score`]).
pub fn score_info(q: &TrackRef, r: &Info) -> Option<f32> {
    let (tq, tr) = (title(&q.title), title(r.title));
    if tq.versions != tr.versions {
        return None;
    }
    let title_score = if tq.base == tr.base {
        1.0
    } else if !tq.base.is_empty() && overlap(&tq.base, &tr.base) >= 0.999 && words(&tq.base).len().abs_diff(words(&tr.base).len()) <= 1 {
        // One title has an extra word ("the", a stray credit): close, not exact.
        0.6
    } else {
        return None;
    };
    // Artists: any credited artist appearing in the record's artist field
    // (providers often list only the main artist, or "A, B" / "A & B").
    let ra = normalize(r.artist);
    let same_album = !q.album.is_empty() && !album(&q.album).is_empty() && album(&q.album) == album(r.album);
    let artist_score = if q.artists.iter().map(|a| normalize(a)).any(|a| !a.is_empty() && (has_word(&ra, &a) || ra == a)) {
        1.0
    } else if q.artists.iter().any(|a| overlap(&normalize(a), &ra) >= 0.5) {
        0.5
    } else if tq.base == tr.base && same_album && q.artists.iter().any(|a| other_script(a, r.artist)) {
        // The artist's name in another script (Spotify's "Anri", NetEase's
        // "杏里"): the same title on the same album is the same recording.
        0.5
    } else {
        return None;
    };
    let duration_score = if q.duration_ms == 0 || r.duration_ms == 0 {
        0.5
    } else {
        let apart = (q.duration_ms as f32 - r.duration_ms as f32).abs() / 1000.0;
        if apart <= 1.0 {
            1.0
        } else if apart <= 2.0 {
            0.9
        } else if apart <= SAME_RECORDING_S {
            0.7
        } else {
            return None;
        }
    };
    let album_score = if same_album { 0.3 } else { 0.0 };
    let lyrics_score = match (r.instrumental, r.synced, r.words) {
        // A record marked instrumental for a song that isn't: probably wrong.
        (true, ..) if !tq.versions.contains(&Version::Instrumental) => -0.5,
        (true, ..) => 0.2,
        (_, _, true) => 0.25,
        (_, true, _) => 0.2,
        _ => 0.0,
    };
    Some(title_score + artist_score + duration_score + album_score + lyrics_score)
}

/// For word timing, the recording must be the very one playing: both lengths
/// known and within a few seconds, besides everything [`score_info`] checks.
pub fn strict(q: &TrackRef, r: &Info) -> Option<f32> {
    if q.duration_ms == 0 || r.duration_ms == 0 || (q.duration_ms as f32 - r.duration_ms as f32).abs() / 1000.0 > SAME_RECORDING_S {
        return None;
    }
    score_info(q, r)
}

/// The best of `found` for `q` under [`strict`] matching: its index.
pub fn best_strict<'a>(q: &TrackRef, found: impl IntoIterator<Item = Info<'a>>) -> Option<usize> {
    found.into_iter().enumerate().filter_map(|(i, r)| strict(q, &r).map(|s| (i, s))).max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map(|(i, _)| i)
}

/// The result of matching search results against a track.
#[derive(Debug, PartialEq)]
pub enum Choice {
    Best(usize),
    /// Indices of records that fit about equally well but disagree.
    Ambiguous(Vec<usize>),
    None,
}

/// Records closer than this to the best are contenders.
const CLOSE: f32 = 0.15;

/// The first few sung lines, to see whether two records are the same lyrics.
fn fingerprint(r: &Record) -> String {
    let text = r.plain.clone().or_else(|| r.lyrics().map(|l| l.lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n"))).unwrap_or_default();
    text.lines().map(normalize).filter(|l| !l.is_empty()).take(4).collect::<Vec<_>>().join("|")
}

/// Picks the record for `q`, or says the choice is the user's to make.
pub fn choose(q: &TrackRef, records: &[Record]) -> Choice {
    let mut scored: Vec<(usize, f32)> = records.iter().enumerate().filter_map(|(i, r)| score(q, r).map(|s| (i, s))).collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let Some(&(best, top)) = scored.first() else { return Choice::None };
    let contenders: Vec<usize> = scored.iter().filter(|(_, s)| *s >= top - CLOSE).map(|(i, _)| *i).collect();
    let first = fingerprint(&records[best]);
    let disagree = contenders.iter().any(|&i| records[i].instrumental != records[best].instrumental || fingerprint(&records[i]) != first);
    if contenders.len() > 1 && disagree { Choice::Ambiguous(contenders.into_iter().take(8).collect()) } else { Choice::Best(best) }
}

/// The title as written, without the remaster/feature notes services file
/// songs without ("Levitating (feat. DaBaby) - 2020 Remaster" → "Levitating"),
/// for services that look a title up as given. Live, remix and other version
/// markers stay: they name another recording.
pub fn plain_title(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    loop {
        let Some(open) = rest.find(['(', '[']) else {
            out.push_str(rest);
            break;
        };
        let close_char = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close_char) else {
            out.push_str(rest);
            break;
        };
        let inner = &rest[open + 1..open + 1 + len];
        out.push_str(&rest[..open]);
        if classify(inner).is_none_or(|v| !v.is_empty()) {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + 1 + len + 1..];
    }
    let mut parts: Vec<&str> = out.split(" - ").collect();
    while parts.len() > 1 && classify(parts[parts.len() - 1]).is_some_and(|v| v.is_empty()) {
        parts.pop();
    }
    let joined = parts.join(" - ");
    let t = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() { s.trim().to_string() } else { t }
}

/// The text to search LRCLIB with: the title without version noise that
/// would make keyword search miss ("feat. X", "2011 Remaster").
pub fn search_title(s: &str) -> String {
    let t = title(s);
    if t.base.is_empty() { s.to_string() } else { t.base }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artists: &[&str], album: &str, secs: u64) -> TrackRef {
        TrackRef { key: "spotify:track:x".into(), title: title.into(), artists: artists.iter().map(|s| s.to_string()).collect(), album: album.into(), duration_ms: secs * 1000, ..Default::default() }
    }

    fn rec(id: u64, title: &str, artist: &str, album: &str, secs: f64, synced: &str) -> Record {
        Record {
            id,
            track_name: title.into(),
            artist_name: artist.into(),
            album_name: album.into(),
            duration_ms: (secs * 1000.0) as u64,
            instrumental: false,
            has_word_sync: false,
            plain: None,
            synced: (!synced.is_empty()).then(|| synced.to_string()),
            lyricsfile: None,
        }
    }

    #[test]
    fn normalizing() {
        assert_eq!(normalize("Don't Stop Me Now!"), "dont stop me now");
        assert_eq!(normalize("Simon & Garfunkel"), "simon and garfunkel");
        assert_eq!(normalize("Beyoncé — ÉPICO"), "beyoncé épico");
        assert_eq!(normalize("夜に駆ける"), "夜に駆ける");
    }

    #[test]
    fn version_markers() {
        let t = title("Bohemian Rhapsody - Remastered 2011");
        assert_eq!((t.base.as_str(), t.versions.is_empty()), ("bohemian rhapsody", true));
        assert_eq!(title("Hey Jude (Live at Wembley) [2014 Remaster]").versions, BTreeSet::from([Version::Live]));
        assert_eq!(title("Blinding Lights - Radio Edit").versions, BTreeSet::from([Version::Edit]));
        assert_eq!(title("Levitating (feat. DaBaby)").base, "levitating");
        assert_eq!(title("Midnight City (Eric Prydz Remix)").versions, BTreeSet::from([Version::Remix]));
        assert_eq!(title("Song - 2019 Mix").versions, BTreeSet::new(), "a new stereo mix of the same take");
        // Parentheses that are part of the name stay.
        assert_eq!(title("(I Can't Get No) Satisfaction").base, "i cant get no satisfaction");
        assert_eq!(title("Hey Jude (Take 2)").base, "hey jude take 2");
        assert_eq!(title("Anti-Hero").base, "anti hero");
    }

    #[test]
    fn exact_recordings_match() {
        let q = track("Bohemian Rhapsody - Remastered 2011", &["Queen"], "A Night at the Opera (Deluxe Remastered Version)", 354);
        assert!(score(&q, &rec(1, "Bohemian Rhapsody", "Queen", "A Night at the Opera", 354.0, "[00:01.00]x")).is_some());
        // Featured artists: the main one is enough.
        let q = track("Levitating (feat. DaBaby)", &["Dua Lipa", "DaBaby"], "Future Nostalgia", 203);
        assert!(score(&q, &rec(2, "Levitating", "Dua Lipa feat. DaBaby", "Future Nostalgia", 203.0, "x")).is_some());
        assert!(score(&q, &rec(3, "Levitating", "Dua Lipa", "Future Nostalgia", 204.5, "x")).is_some());
    }

    #[test]
    fn other_recordings_dont() {
        let q = track("Hey Jude", &["The Beatles"], "1", 431);
        assert_eq!(score(&q, &rec(1, "Hey Jude (Live)", "The Beatles", "Live", 431.0, "x")), None, "live take");
        assert_eq!(score(&q, &rec(2, "Hey Jude", "The Beatles", "1", 425.0, "x")), None, "6 s off: another edit");
        assert_eq!(score(&q, &rec(3, "Hey Jude", "Wilson Pickett", "", 431.0, "x")), None, "a cover");
        assert_eq!(score(&q, &rec(4, "Let It Be", "The Beatles", "1", 431.0, "x")), None, "another song");
        let edit = track("Blinding Lights - Radio Edit", &["The Weeknd"], "", 200);
        assert_eq!(score(&edit, &rec(5, "Blinding Lights", "The Weeknd", "After Hours", 200.0, "x")), None);
        assert!(score(&edit, &rec(6, "Blinding Lights (Radio Edit)", "The Weeknd", "", 200.0, "x")).is_some());
    }

    #[test]
    fn best_record_wins_and_duplicates_arent_ambiguous() {
        let q = track("Song", &["Artist"], "Album", 200);
        let recs =
            vec![rec(1, "Song", "Artist", "Other", 202.5, "[00:01.00]hello there"), rec(2, "Song", "Artist", "Album", 200.0, "[00:01.00]hello there"), rec(3, "Song", "Artist", "Album", 200.0, "")];
        assert_eq!(choose(&q, &recs), Choice::Best(1), "same album, closest duration, synced");
        assert_eq!(choose(&q, &[rec(9, "Other", "Artist", "Album", 200.0, "x")]), Choice::None);
    }

    #[test]
    fn disagreeing_records_are_ambiguous() {
        let q = track("Intro", &["Artist"], "", 120);
        let mut a = rec(1, "Intro", "Artist", "First Album", 120.0, "[00:01.00]words from the first album");
        let mut b = rec(2, "Intro", "Artist", "Second Album", 120.5, "[00:01.00]completely different words");
        a.plain = Some("words from the first album".into());
        b.plain = Some("completely different words".into());
        assert!(matches!(choose(&q, &[a, b]), Choice::Ambiguous(v) if v.len() == 2));
    }

    #[test]
    fn instrumental_records_dont_win_over_lyrics() {
        let q = track("Song", &["Artist"], "Album", 200);
        let mut inst = rec(1, "Song", "Artist", "Album", 200.0, "");
        inst.instrumental = true;
        let words = rec(2, "Song", "Artist", "Album", 201.0, "[00:01.00]la la");
        assert_eq!(choose(&q, &[inst.clone(), words]), Choice::Best(1));
        assert_eq!(choose(&q, &[inst]), Choice::Best(0), "alone, an instrumental record is believed");
    }

    #[test]
    fn word_timing_needs_the_very_recording() {
        let q = track("Blinding Lights", &["The Weeknd"], "After Hours", 200);
        let hit = |title, artist, secs: u64| Info { title, artist, album: "", duration_ms: secs * 1000, ..Default::default() };
        assert!(strict(&q, &hit("Blinding Lights", "The Weeknd", 200)).is_some());
        assert!(strict(&q, &hit("Blinding Lights", "The Weeknd", 0)).is_none(), "an unknown length can't be trusted with word timing");
        assert!(strict(&q, &hit("Blinding Lights (Remix)", "The Weeknd, ROSALÍA", 216)).is_none(), "a remix");
        assert!(strict(&q, &hit("Blinding Lights", "The Weeknd", 205)).is_none(), "5 s off");
        let unknown = TrackRef { duration_ms: 0, ..q.clone() };
        assert!(strict(&unknown, &hit("Blinding Lights", "The Weeknd", 200)).is_none());
        let hits = [hit("Blinding Lights (Remix)", "The Weeknd, ROSALÍA", 216), hit("Blinding Lights", "The Weeknd", 202), hit("Blinding Lights", "The Weeknd", 200)];
        assert_eq!(best_strict(&q, hits), Some(2), "the closest length wins");
    }

    #[test]
    fn an_artist_named_in_another_script() {
        let q = track("DRIVING MY LOVE", &["Anri"], "Timely!!", 291);
        let hit = |artist, album| Info { title: "DRIVING MY LOVE", artist, album, duration_ms: 291_426, ..Default::default() };
        assert!(strict(&q, &hit("杏里", "Timely!!")).is_some(), "NetEase writes Anri as 杏里");
        assert!(strict(&q, &hit("杏里", "Surf City -Coool Breeze-")).is_none(), "on another album it could be anyone's");
        assert!(strict(&q, &hit("眠", "")).is_none());
        assert!(strict(&q, &hit("Kirbie", "Timely!!")).is_none(), "both in Latin letters: just another artist");
        let mixed = track("DRIVING MY LOVE", &["杏里 Anri"], "Timely!!", 291);
        assert!(strict(&mixed, &hit("杏里", "Timely!!")).is_some(), "a name in both scripts matches as usual");
        assert!(
            strict(&track("Sweet Agnes", &["Masayoshi Takanaka"], "TAKANAKA", 280), &Info { title: "Sweet Agnes", artist: "高中正義", album: "Takanaka", duration_ms: 280_013, ..Default::default() })
                .is_some()
        );
    }

    #[test]
    fn plain_titles() {
        assert_eq!(plain_title("Levitating (feat. DaBaby) - 2020 Remaster"), "Levitating");
        assert_eq!(plain_title("Don't Stop Me Now - Remastered 2011"), "Don't Stop Me Now");
        assert_eq!(plain_title("Hey Jude (Live at Wembley)"), "Hey Jude (Live at Wembley)", "another recording keeps its name");
        assert_eq!(plain_title("(I Can't Get No) Satisfaction"), "(I Can't Get No) Satisfaction");
    }

    #[test]
    fn search_text() {
        assert_eq!(search_title("Levitating (feat. DaBaby) - 2020 Remaster"), "levitating");
        assert_eq!(search_title("Hey Jude - Live"), "hey jude");
    }
}
