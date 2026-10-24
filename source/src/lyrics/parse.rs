//! Reading what LRCLIB returns: the JSON record, the Lyricsfile (YAML) inside
//! it, LRC text and plain text.
//!
//! Records are user-contributed, so nothing is trusted: missing fields fall
//! back, malformed timing is dropped rather than guessed, and a Lyricsfile
//! that doesn't parse falls back to the record's LRC or plain text.

use super::{Candidate, Line, Lyrics, ProviderId, Source, Timing, Token};
use serde::Deserialize;
use serde_json::Value;

/// One LRCLIB lyrics record (`/api/get`, `/api/get/{id}`, `/api/search`).
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: u64,
    pub track_name: String,
    pub artist_name: String,
    pub album_name: String,
    /// LRCLIB stores seconds (sometimes fractional).
    pub duration_ms: u64,
    pub instrumental: bool,
    pub has_word_sync: bool,
    pub plain: Option<String>,
    pub synced: Option<String>,
    pub lyricsfile: Option<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string)
}

/// Reads one record. Only `id` is required; everything else falls back.
pub fn record(v: &Value) -> Result<Record, String> {
    let id = v.get("id").and_then(Value::as_u64).ok_or("lyrics record without an id")?;
    let seconds = v.get("duration").and_then(Value::as_f64).filter(|d| d.is_finite() && *d >= 0.0).unwrap_or(0.0);
    Ok(Record {
        id,
        track_name: text(v, "trackName").or_else(|| text(v, "name")).unwrap_or_default(),
        artist_name: text(v, "artistName").unwrap_or_default(),
        album_name: text(v, "albumName").unwrap_or_default(),
        duration_ms: (seconds * 1000.0).round() as u64,
        instrumental: v.get("instrumental").and_then(Value::as_bool).unwrap_or(false),
        has_word_sync: v.get("hasWordSync").and_then(Value::as_bool).unwrap_or(false),
        plain: text(v, "plainLyrics"),
        synced: text(v, "syncedLyrics"),
        lyricsfile: text(v, "lyricsfile"),
    })
}

/// Reads a search result: an array of records. Malformed entries are skipped.
pub fn records(v: &Value) -> Result<Vec<Record>, String> {
    let list = v.as_array().ok_or("expected a list of lyrics records")?;
    Ok(list.iter().filter_map(|r| record(r).ok()).collect())
}

impl Record {
    pub fn source(&self) -> Source {
        Source {
            provider: ProviderId::Lrclib,
            id: self.id.to_string(),
            title: self.track_name.clone(),
            artist: self.artist_name.clone(),
            album: self.album_name.clone(),
            duration_ms: self.duration_ms,
        }
    }

    /// The best lyrics in the record: Lyricsfile (it carries word timing and
    /// language), then LRC, then plain. `None` when it has none (instrumental
    /// or empty).
    pub fn lyrics(&self) -> Option<Lyrics> {
        if self.instrumental {
            return None;
        }
        let source = self.source();
        let from_file = self.lyricsfile.as_deref().and_then(|f| match lyricsfile(f) {
            Ok(doc) => Some(doc),
            Err(e) => {
                log::debug!("lrclib {}: unusable lyricsfile ({e}); using LRC/plain", self.id);
                None
            }
        });
        if let Some(doc) = &from_file {
            if doc.instrumental {
                return None;
            }
            if !doc.lines.is_empty() {
                return Some(Lyrics { source, language: doc.language.clone(), timing: doc.timing, lines: doc.lines.clone(), translations: Vec::new() });
            }
        }
        let language = from_file.as_ref().and_then(|d| d.language.clone());
        if let Some((lines, timing)) = self.synced.as_deref().map(lrc).filter(|(l, _)| !l.is_empty()) {
            return Some(Lyrics { source, language, timing, lines, translations: Vec::new() });
        }
        let plain_text = self.plain.clone().or_else(|| from_file.and_then(|d| d.plain));
        let lines = plain(plain_text.as_deref()?);
        (!lines.is_empty()).then_some(Lyrics { source, language, timing: Timing::Plain, lines, translations: Vec::new() })
    }

    pub fn candidate(&self) -> Candidate {
        let lyrics = self.lyrics();
        let preview = lyrics.as_ref().and_then(|l| l.lines.iter().map(|l| l.text.trim()).find(|t| !t.is_empty()).map(str::to_string)).unwrap_or_default();
        Candidate {
            record: std::sync::Arc::new(self.clone()),
            source: self.source(),
            timing: lyrics.as_ref().map(|l| l.timing).unwrap_or(Timing::Plain),
            instrumental: self.instrumental || lyrics.is_none(),
            preview,
        }
    }
}

// ------------------------------------------------------------------ Lyricsfile

#[derive(Deserialize)]
struct FileDoc {
    #[serde(default)]
    metadata: Option<FileMeta>,
    #[serde(default)]
    lines: Option<Vec<FileLine>>,
    #[serde(default)]
    plain: Option<String>,
}

#[derive(Deserialize, Default)]
struct FileMeta {
    #[serde(default)]
    offset_ms: Option<i64>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    instrumental: Option<bool>,
}

#[derive(Deserialize)]
struct FileLine {
    #[serde(default)]
    text: Option<String>,
    start_ms: i64,
    #[serde(default)]
    end_ms: Option<i64>,
    #[serde(default)]
    words: Option<Vec<FileWord>>,
}

#[derive(Deserialize)]
struct FileWord {
    #[serde(default)]
    text: Option<String>,
    start_ms: i64,
    #[serde(default)]
    end_ms: Option<i64>,
}

/// A Lyricsfile, normalized.
#[derive(Debug)]
pub struct LyricsFile {
    pub lines: Vec<Line>,
    pub timing: Timing,
    pub language: Option<String>,
    pub instrumental: bool,
    pub plain: Option<String>,
}

/// Anything bigger is not a lyrics file.
const MAX_FILE: usize = 1 << 20;

/// Parses a Lyricsfile (LRCLIB's YAML format: `metadata`, `lines[]` with
/// `start_ms`/`end_ms`, optional `words[]`, and `plain`). The file's own
/// `offset_ms` is applied to every timestamp.
pub fn lyricsfile(src: &str) -> Result<LyricsFile, String> {
    if src.len() > MAX_FILE {
        return Err("lyricsfile too large".into());
    }
    let doc: FileDoc = serde_saphyr::from_str(src).map_err(|e| e.to_string().lines().next().unwrap_or("bad YAML").to_string())?;
    let meta = doc.metadata.unwrap_or_default();
    // Same convention as LRC: a positive offset makes lyrics appear sooner.
    let offset = meta.offset_ms.unwrap_or(0).clamp(-86_400_000, 86_400_000);
    let at = |ms: i64| ms.saturating_sub(offset).max(0) as u64;
    let mut lines: Vec<Line> = doc
        .lines
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.start_ms >= 0)
        .map(|l| {
            let text = l.text.unwrap_or_default();
            let tokens = l
                .words
                .unwrap_or_default()
                .into_iter()
                .filter(|w| w.start_ms >= 0)
                .map(|w| Token { text: w.text.unwrap_or_default(), start_ms: at(w.start_ms), end_ms: w.end_ms.filter(|e| *e >= 0).map(at) })
                .collect();
            Line { start_ms: at(l.start_ms), end_ms: l.end_ms.filter(|e| *e >= 0).map(at), text, tokens }
        })
        .collect();
    lines.sort_by_key(|l| l.start_ms);
    let timing = settle_tokens(&mut lines);
    let language = meta.language.map(|s| s.trim().to_lowercase()).filter(|s| (2..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphabetic() || c == '-'));
    Ok(LyricsFile { lines, timing, language, instrumental: meta.instrumental.unwrap_or(false), plain: doc.plain.filter(|p| !p.trim().is_empty()) })
}

/// Drops word timing that doesn't hold together (out of order, outside its
/// line, or text that doesn't match the line) and says how precisely the
/// remaining lyrics are timed. Lines keep their own timing either way.
fn settle_tokens(lines: &mut [Line]) -> Timing {
    let starts: Vec<u64> = lines.iter().map(|l| l.start_ms).collect();
    for (i, l) in lines.iter_mut().enumerate() {
        if l.tokens.is_empty() {
            continue;
        }
        if l.text.trim().is_empty() {
            l.text = l.tokens.iter().map(|t| t.text.as_str()).collect();
        }
        let next = starts.get(i + 1).copied().unwrap_or(u64::MAX);
        let ordered = l.tokens.windows(2).all(|p| p[0].start_ms <= p[1].start_ms);
        // A little slack: hand-timed words often start a beat before the line.
        let inside = l.tokens.iter().all(|t| t.start_ms + 500 >= l.start_ms && t.start_ms <= next.saturating_add(500));
        let joined: String = l.tokens.iter().map(|t| t.text.as_str()).collect();
        let same_text = squash(&joined) == squash(&l.text);
        if !(ordered && inside && same_text) {
            l.tokens.clear();
        }
    }
    let sung: Vec<&Line> = lines.iter().filter(|l| !l.text.trim().is_empty()).collect();
    if sung.is_empty() {
        return Timing::Line;
    }
    // Word timing only counts when every sung line has it; otherwise the
    // lyrics are labeled by their weakest lines (timed ones still highlight).
    if sung.iter().all(|l| !l.tokens.is_empty()) { if sung.iter().any(|l| splits_words(&l.tokens)) { Timing::Syllable } else { Timing::Word } } else { Timing::Line }
}

/// Letters and digits only, lowercased: for comparing texts that differ in
/// spacing or punctuation.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// True when some tokens are parts of a word (no space between two letters
/// of the same word), i.e. syllable timing. CJK text has no spaces between
/// words, so a join between two CJK characters doesn't count.
fn splits_words(tokens: &[Token]) -> bool {
    tokens.windows(2).any(|p| {
        let (Some(a), Some(b)) = (p[0].text.chars().last(), p[1].text.chars().next()) else { return false };
        a.is_alphanumeric() && b.is_alphanumeric() && !(is_cjk(a) || is_cjk(b))
    })
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x2FA1F)
}

// ------------------------------------------------------------------------ LRC

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx` (also `:` before the fraction).
fn timestamp(s: &str) -> Option<u64> {
    let (m, rest) = s.trim().split_once(':')?;
    let (sec, frac) = match rest.find(['.', ':']) {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    let m: u64 = m.trim().parse().ok()?;
    let sec: u64 = sec.parse().ok()?;
    // Under 100 hours: anything longer is a broken file, not a song.
    if m >= 6_000 || sec >= 60 || frac.len() > 3 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let ms = if frac.is_empty() { 0 } else { frac.parse::<u64>().ok()? * 10u64.pow(3 - frac.len() as u32) };
    Some(m * 60_000 + sec * 1000 + ms)
}

/// Parses LRC: `[mm:ss.xx]` line times (several per line allowed), the
/// `[offset:]` tag, and enhanced-LRC `<mm:ss.xx>` word times when present.
pub fn lrc(src: &str) -> (Vec<Line>, Timing) {
    let mut offset: i64 = 0;
    let mut out: Vec<Line> = Vec::new();
    for raw in src.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(body) = rest.strip_prefix('[') {
            let Some(close) = body.find(']') else { break };
            let tag = &body[..close];
            match timestamp(tag) {
                Some(t) => times.push(t),
                None => {
                    if let Some(v) = tag.strip_prefix("offset:") {
                        // Within a day either way; anything else is a broken tag.
                        offset = v.trim().trim_start_matches('+').parse::<i64>().unwrap_or(0).clamp(-86_400_000, 86_400_000);
                    }
                }
            }
            rest = &body[close + 1..];
        }
        if times.is_empty() {
            continue;
        }
        let (text, words) = enhanced(rest);
        for t in times {
            out.push(Line { start_ms: t, end_ms: None, text: text.clone(), tokens: words.iter().map(|(w, s, e)| Token { text: w.clone(), start_ms: s.unwrap_or(t), end_ms: *e }).collect() });
        }
    }
    // A positive LRC offset makes lyrics appear sooner.
    let shift = |t: u64| (t as i64 - offset).max(0) as u64;
    for l in &mut out {
        l.start_ms = shift(l.start_ms);
        for w in &mut l.tokens {
            w.start_ms = shift(w.start_ms);
            w.end_ms = w.end_ms.map(shift);
        }
    }
    out.sort_by_key(|l| l.start_ms);
    let timing = settle_tokens(&mut out);
    (out, timing)
}

type Word = (String, Option<u64>, Option<u64>);

/// Splits `<00:12.00>The <00:12.50>club` into timed words. Returns the plain
/// text and the words (empty when the line has no word tags).
fn enhanced(s: &str) -> (String, Vec<Word>) {
    if !s.contains('<') {
        return (s.trim().to_string(), Vec::new());
    }
    let mut words: Vec<Word> = Vec::new();
    let mut pending: Option<u64> = None;
    let mut rest = s;
    let mut tagged = false;
    loop {
        let (before, after) = match rest.find('<') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        if !before.is_empty() {
            words.push((before.to_string(), pending.take(), None));
        }
        let Some(after) = after else { break };
        match after.find('>').and_then(|c| timestamp(&after[..c]).map(|t| (c, t))) {
            Some((c, t)) => {
                tagged = true;
                // A tag right after a word closes it; the next text starts with it.
                if let Some(last) = words.last_mut().filter(|w| w.2.is_none() && pending.is_none()) {
                    last.2 = Some(t);
                }
                pending = Some(t);
                rest = &after[c + 1..];
            }
            None => {
                // Not a time tag: keep the '<' as text.
                words.push(("<".into(), pending.take(), None));
                rest = after;
            }
        }
    }
    let text: String = words.iter().map(|w| w.0.as_str()).collect::<String>().trim().to_string();
    if !tagged {
        return (s.trim().to_string(), Vec::new());
    }
    // Words with no time of their own (text before the first tag) start with
    // the line. Trim the outer whitespace so the tokens join back into the line.
    let mut words: Vec<Word> = words.into_iter().filter(|w| !w.0.trim().is_empty() || w.1.is_some()).collect();
    if let Some(f) = words.first_mut() {
        f.0 = f.0.trim_start().to_string();
    }
    if let Some(l) = words.last_mut() {
        l.0 = l.0.trim_end().to_string();
    }
    words.retain(|w| !w.0.is_empty());
    (text, words)
}

// ---------------------------------------------------------------------- plain

/// Plain lyrics as lines; runs of blank lines become one (a stanza break).
pub fn plain(src: &str) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for l in src.lines().map(str::trim_end) {
        if l.trim().is_empty() && out.last().is_none_or(|p| p.text.is_empty()) {
            continue;
        }
        out.push(Line::untimed(l.trim()));
    }
    while out.last().is_some_and(|l| l.text.is_empty()) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A real `/api/get/3396226` response, trimmed to three lines (fixture).
    fn real_line_synced() -> Value {
        json!({
            "id": 3396226, "name": "I Want to Live", "trackName": "I Want to Live", "artistName": "Borislav Slavov",
            "albumName": "Baldur's Gate 3 (Original Game Soundtrack)", "duration": 233.0, "instrumental": false, "hasWordSync": false,
            "plainLyrics": "I feel your breath upon my neck\nA soft caress as cold as death\n",
            "syncedLyrics": "[00:17.12] I feel your breath upon my neck\n[00:20.41] A soft caress as cold as death\n[03:25.72] ",
            "lyricsfile": "version: '1.0'\nmetadata:\n  title: I Want to Live\n  artist: Borislav Slavov\n  album: Baldur's Gate 3 (Original Game Soundtrack)\n  duration_ms: 233000\n  instrumental: false\nlines:\n- text: I feel your breath upon my neck\n  start_ms: 17120\n  end_ms: 20410\n- text: A soft caress as cold as death\n  start_ms: 20410\n  end_ms: 29600\n- text: ''\n  start_ms: 49720\n  end_ms: 51120\n"
        })
    }

    /// A word-synced Lyricsfile in the documented format (fixture, written
    /// from LRCLIB's spec; no live record with word timing was found).
    const WORD_FILE: &str = "version: '1.0'\nmetadata:\n  title: Song\n  artist: Someone\n  language: EN\n  instrumental: false\nlines:\n  - text: 'Waiting in a car'\n    start_ms: 23500\n    end_ms: 26800\n    words:\n      - text: 'Waiting '\n        start_ms: 23500\n        end_ms: 24400\n      - text: 'in '\n        start_ms: 24400\n      - text: 'a '\n        start_ms: 24700\n      - text: 'car'\n        start_ms: 25000\n        end_ms: 26800\n  - text: 'Café 夜 ünïcode'\n    start_ms: 27000\n    words:\n      - text: 'Café '\n        start_ms: 27000\n      - text: '夜 '\n        start_ms: 27600\n      - text: 'ünïcode'\n        start_ms: 28100\n";

    #[test]
    fn real_record_reads_as_line_sync() {
        let r = record(&real_line_synced()).unwrap();
        assert_eq!((r.id, r.duration_ms, r.has_word_sync), (3396226, 233_000, false));
        let l = r.lyrics().unwrap();
        assert_eq!(l.timing, Timing::Line);
        assert_eq!(l.lines[0].start_ms, 17_120);
        assert_eq!(l.lines[0].end_ms, Some(20_410), "the lyricsfile's end times are kept");
        assert!(l.lines.iter().all(|l| l.tokens.is_empty()), "no words are invented");
        assert_eq!(l.lines[2].text, "", "instrumental gaps stay as empty lines");
    }

    #[test]
    fn word_timing_from_lyricsfile() {
        let f = lyricsfile(WORD_FILE).unwrap();
        assert_eq!(f.timing, Timing::Word);
        assert_eq!(f.language.as_deref(), Some("en"));
        let w: Vec<(&str, u64)> = f.lines[0].tokens.iter().map(|t| (t.text.as_str(), t.start_ms)).collect();
        assert_eq!(w, vec![("Waiting ", 23500), ("in ", 24400), ("a ", 24700), ("car", 25000)]);
        assert_eq!(f.lines[0].tokens[0].end_ms, Some(24400));
        assert_eq!(f.lines[1].tokens[1].text, "夜 ", "Unicode survives");
    }

    #[test]
    fn syllables_are_told_apart_from_words() {
        let file = "metadata: {title: a, artist: b}\nlines:\n  - text: 'beautiful day'\n    start_ms: 1000\n    words:\n      - {text: 'beau', start_ms: 1000}\n      - {text: 'ti', start_ms: 1200}\n      - {text: 'ful ', start_ms: 1400}\n      - {text: 'day', start_ms: 1800}\n";
        assert_eq!(lyricsfile(file).unwrap().timing, Timing::Syllable);
        // CJK has no spaces between words; that's not syllable timing.
        let cjk = "metadata: {title: a, artist: b}\nlines:\n  - text: '夜に駆ける'\n    start_ms: 1000\n    words:\n      - {text: '夜に', start_ms: 1000}\n      - {text: '駆ける', start_ms: 1400}\n";
        assert_eq!(lyricsfile(cjk).unwrap().timing, Timing::Word);
    }

    #[test]
    fn broken_word_timing_is_dropped_not_guessed() {
        // Words out of order, and words whose text doesn't match the line.
        let file = "metadata: {title: a, artist: b}\nlines:\n  - text: 'one two'\n    start_ms: 1000\n    words:\n      - {text: 'one ', start_ms: 1500}\n      - {text: 'two', start_ms: 1100}\n  - text: 'three four'\n    start_ms: 3000\n    words:\n      - {text: 'something ', start_ms: 3000}\n      - {text: 'else', start_ms: 3300}\n";
        let f = lyricsfile(file).unwrap();
        assert!(f.lines.iter().all(|l| l.tokens.is_empty()));
        assert_eq!(f.timing, Timing::Line);
        // Some lines word-timed, others not: labeled by the weakest.
        let mixed = "metadata: {title: a, artist: b}\nlines:\n  - text: 'one two'\n    start_ms: 1000\n    words:\n      - {text: 'one ', start_ms: 1000}\n      - {text: 'two', start_ms: 1300}\n  - text: 'three'\n    start_ms: 3000\n";
        let f = lyricsfile(mixed).unwrap();
        assert_eq!(f.timing, Timing::Line);
        assert_eq!(f.lines[0].tokens.len(), 2, "…but the timed line keeps its words");
    }

    #[test]
    fn lyricsfile_offset_and_bad_documents() {
        let f = lyricsfile("metadata: {title: a, artist: b, offset_ms: 500}\nlines:\n  - {text: x, start_ms: 2000}\n").unwrap();
        assert_eq!(f.lines[0].start_ms, 1500);
        assert!(lyricsfile("lines: [unclosed").is_err());
        assert!(lyricsfile("lines:\n  - {text: x, start_ms: 'soon'}\n").is_err());
        assert!(lyricsfile(&"a".repeat(MAX_FILE + 1)).is_err());
        // A bad lyricsfile falls back to the record's LRC.
        let mut v = real_line_synced();
        v["lyricsfile"] = json!("lines: [unclosed");
        let l = record(&v).unwrap().lyrics().unwrap();
        assert_eq!((l.timing, l.lines[0].start_ms), (Timing::Line, 17_120));
    }

    #[test]
    fn lrc_lines_offsets_and_repeats() {
        let (l, t) = lrc("[ar:Someone]\n[offset:+200]\n[00:01.50]first\n[00:10.00][00:30.00]chorus\n[01:02.123]third\nnot a lyric line\n[00:20]\n");
        assert_eq!(t, Timing::Line);
        let got: Vec<(u64, &str)> = l.iter().map(|l| (l.start_ms, l.text.as_str())).collect();
        assert_eq!(got, vec![(1300, "first"), (9800, "chorus"), (19800, ""), (29800, "chorus"), (61923, "third")]);
    }

    #[test]
    fn enhanced_lrc_words() {
        let (l, t) = lrc("[00:12.00]<00:12.00>The <00:12.50>club <00:13.00>isn't<00:13.80>\n[00:14.00]<00:14.00>So <00:14.40>the <00:14.90>bar\n");
        assert_eq!(t, Timing::Word);
        assert_eq!(l[0].text, "The club isn't");
        let w: Vec<(&str, u64, Option<u64>)> = l[0].tokens.iter().map(|t| (t.text.as_str(), t.start_ms, t.end_ms)).collect();
        assert_eq!(w, vec![("The ", 12_000, Some(12_500)), ("club ", 12_500, Some(13_000)), ("isn't", 13_000, Some(13_800))]);
        // A '<' that isn't a time stays text.
        let (l, t) = lrc("[00:01.00]a <3 b\n");
        assert_eq!((l[0].text.as_str(), t), ("a <3 b", Timing::Line));
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp("01:02.5"), Some(62_500));
        assert_eq!(timestamp("01:02.05"), Some(62_050));
        assert_eq!(timestamp("01:02:005"), Some(62_005));
        assert_eq!(timestamp("12:00"), Some(720_000));
        assert_eq!(timestamp("ar:Someone"), None);
        assert_eq!(timestamp("01:75.00"), None);
    }

    #[test]
    fn plain_and_instrumental_records() {
        let r = record(
            &json!({ "id": 7, "trackName": "T", "artistName": "A", "duration": 180, "instrumental": false, "plainLyrics": "line one\n\n\n\nline two\n\n", "syncedLyrics": null, "lyricsfile": "" }),
        )
        .unwrap();
        let l = r.lyrics().unwrap();
        assert_eq!(l.timing, Timing::Plain);
        assert_eq!(l.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["line one", "", "line two"]);
        let i = record(&json!({ "id": 8, "trackName": "T", "artistName": "A", "duration": 180, "instrumental": true, "plainLyrics": null, "syncedLyrics": null })).unwrap();
        assert!(i.lyrics().is_none());
        assert!(i.candidate().instrumental);
        // Nothing usable at all.
        assert!(record(&json!({ "id": 9 })).unwrap().lyrics().is_none());
    }

    #[test]
    fn malformed_records() {
        assert!(record(&json!({ "trackName": "no id" })).is_err());
        assert!(record(&json!("text")).is_err());
        let r = record(&json!({ "id": 1, "duration": "long", "trackName": 5 })).unwrap();
        assert_eq!((r.duration_ms, r.track_name.as_str()), (0, ""));
        assert_eq!(records(&json!([{ "id": 1 }, { "nope": 1 }, 3])).unwrap().len(), 1);
        assert!(records(&json!({ "message": "busy" })).is_err());
    }
}
