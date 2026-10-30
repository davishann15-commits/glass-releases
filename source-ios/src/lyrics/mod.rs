//! Lyrics: one model for every provider, and how it lines up with playback.
//!
//! Providers hand back very different things — LRCLIB a JSON record with a
//! Lyricsfile (YAML), LRC text and plain text; lrc.red, Unison and Spicy Lyrics
//! TTML-shaped documents with voices and backing vocals; NetEase its own word
//! format. Everything is normalized into [`Lyrics`]: lines, and for each line
//! the timed words or syllables only when the provider actually timed them.
//! Nothing here invents timestamps: a line without timed words is highlighted
//! as a whole, and plain lyrics aren't highlighted at all.

pub mod agree;
pub mod cache;
pub mod choices;
pub mod http;
pub mod lrclib;
pub mod lrcmux;
pub mod lrcred;
pub mod matching;
pub mod musixmatch;
pub mod netease;
pub mod parse;
pub mod qobuzify;
pub mod sample;
pub mod service;
pub mod spicy;
pub mod spotify;
pub mod translate;
pub mod ttml;
pub mod unison;

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    Lrclib,
    Spotify,
    /// lrc.red: TTML by ISRC, word or syllable timed, with voices and backing vocals.
    LrcRed,
    /// Unison (Better Lyrics): community TTML.
    Unison,
    /// NetEase Cloud Music's word timing (yrc).
    NetEase,
    /// Musixmatch's developer API, with the user's own key.
    Musixmatch,
    /// Spicy Lyrics' developer API, with the user's own key.
    Spicy,
    /// Qobuzify: syllable or line timing in Spicy Lyrics' format, keyless.
    Qobuzify,
    /// lrcmux: word timing it gathers (KuGou's on its public server), keyless.
    Lrcmux,
}

impl ProviderId {
    pub const ALL: [ProviderId; 9] =
        [ProviderId::Lrclib, ProviderId::Spotify, ProviderId::LrcRed, ProviderId::Unison, ProviderId::NetEase, ProviderId::Musixmatch, ProviderId::Spicy, ProviderId::Qobuzify, ProviderId::Lrcmux];
    /// The sources asked for word timing, in the default order.
    pub const WORD: [ProviderId; 7] = [ProviderId::Spicy, ProviderId::LrcRed, ProviderId::Qobuzify, ProviderId::Musixmatch, ProviderId::Unison, ProviderId::NetEase, ProviderId::Lrcmux];

    /// The lowercase name shown in the lyrics view's status line.
    pub fn name(self) -> &'static str {
        match self {
            ProviderId::Lrclib => "lrclib",
            ProviderId::Spotify => "spotify",
            ProviderId::LrcRed => "lrc.red",
            ProviderId::Unison => "unison",
            ProviderId::NetEase => "netease",
            ProviderId::Musixmatch => "musixmatch",
            ProviderId::Spicy => "spicy lyrics",
            ProviderId::Qobuzify => "qobuzify",
            ProviderId::Lrcmux => "lrcmux",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ProviderId::Lrclib => "LRCLIB",
            ProviderId::Spotify => "Spotify",
            ProviderId::LrcRed => "lrc.red",
            ProviderId::Unison => "Unison",
            ProviderId::NetEase => "NetEase",
            ProviderId::Musixmatch => "Musixmatch",
            ProviderId::Spicy => "Spicy Lyrics",
            ProviderId::Qobuzify => "Qobuzify",
            ProviderId::Lrcmux => "lrcmux",
        }
    }
    /// The word config.toml uses (`lyrics.provider`, `lyrics.sources`).
    pub fn key(self) -> &'static str {
        match self {
            ProviderId::Lrclib => "lrclib",
            ProviderId::Spotify => "spotify",
            ProviderId::LrcRed => "lrcred",
            ProviderId::Unison => "unison",
            ProviderId::NetEase => "netease",
            ProviderId::Musixmatch => "musixmatch",
            ProviderId::Spicy => "spicy",
            ProviderId::Qobuzify => "qobuzify",
            ProviderId::Lrcmux => "lrcmux",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        ProviderId::ALL.into_iter().find(|p| p.key() == s || p.name() == s)
    }
    /// The provider needs a key the user brings.
    pub fn needs_key(self) -> bool {
        matches!(self, ProviderId::Musixmatch | ProviderId::Spicy)
    }
}

/// How precisely the provider timed the lyrics, from least to most precise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Timing {
    /// Text only.
    Plain,
    /// Each line has a start time.
    Line,
    /// Each word has its own start time.
    Word,
    /// Parts of words (syllables) have their own start times.
    Syllable,
}

impl Timing {
    pub fn label(self) -> &'static str {
        match self {
            Timing::Plain => "unsynced",
            Timing::Line => "line sync",
            Timing::Word => "word sync",
            Timing::Syllable => "syllable sync",
        }
    }
}

/// A timed piece of a line: a word (with the space after it, if any) or a
/// syllable. Pieces are drawn one after another, so their text carries its
/// own spacing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

/// Who sings a line, when the provider says (TTML's agents, Spicy Lyrics'
/// opposite-aligned lines).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Voice {
    /// The only singer, the first one of a duet, or not said.
    #[default]
    Lead,
    /// Another singer of a duet: drawn on the other side.
    Other,
    /// Everyone together.
    Group,
}

/// Backing vocals sung under a line, timed on their own.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Backing {
    pub text: String,
    /// Timed words; empty when only the line around them was timed.
    pub tokens: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Line {
    /// Meaningless for plain lyrics.
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
    /// Timed words or syllables; empty when the provider only timed the line.
    pub tokens: Vec<Token>,
    #[serde(default)]
    pub voice: Voice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backing: Option<Box<Backing>>,
}

impl Line {
    pub fn untimed(text: impl Into<String>) -> Self {
        Line { text: text.into(), ..Default::default() }
    }
}

/// The recording the lyrics were written for, as the provider describes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub provider: ProviderId,
    /// The provider's own ID for the record, e.g. LRCLIB's numeric ID.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
}

/// Who to credit under the lyrics (some providers require it), and where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credit {
    pub text: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub source: Source,
    /// ISO 639-1, when the provider says.
    pub language: Option<String>,
    pub timing: Timing,
    pub lines: Vec<Line>,
    /// Translations the provider supplied: (language, one entry per line).
    #[serde(default)]
    pub translations: Vec<(String, Vec<String>)>,
    #[serde(default)]
    pub credits: Vec<Credit>,
    /// The provider couldn't say which recording it timed (Qobuzify): its word
    /// timing is only used when it agrees with the song's line timing (`agree`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unconfirmed: bool,
}

/// A gap this short between two lines keeps the first one lit until the next
/// starts, so the highlight doesn't blink off between lines.
pub const HOLD_MS: u64 = 1_000;

impl Lyrics {
    pub fn new(source: Source, timing: Timing, lines: Vec<Line>) -> Self {
        Lyrics { source, language: None, timing, lines, translations: Vec::new(), credits: Vec::new(), unconfirmed: false }
    }

    /// What the top of the lyrics names as their source: the provider's
    /// credits (with the links some ask for), else the provider itself.
    pub fn named_by(&self) -> std::borrow::Cow<'_, [Credit]> {
        if self.credits.is_empty() { std::borrow::Cow::Owned(vec![Credit { text: self.source.provider.name().to_string(), url: None }]) } else { std::borrow::Cow::Borrowed(&self.credits) }
    }

    /// When line `i` stops being sung: its own end (or its last word's) when
    /// the provider timed it, held over a short gap to the next line;
    /// otherwise the next line's start. The last line without an end is sung
    /// to the end of the song.
    pub fn line_end(&self, i: usize) -> u64 {
        let Some(l) = self.lines.get(i) else { return 0 };
        let next = self.lines[i + 1..].iter().map(|n| n.start_ms).find(|&s| s > l.start_ms);
        let words = l.tokens.iter().chain(l.backing.iter().flat_map(|b| b.tokens.iter())).filter_map(|t| t.end_ms).max();
        let own = match (l.end_ms.filter(|&e| e > l.start_ms), words) {
            (Some(e), Some(w)) => Some(e.max(w)),
            (e, w) => e.or(w).filter(|&e| e > l.start_ms),
        };
        match (own, next) {
            (Some(e), Some(n)) if n > e && n - e <= HOLD_MS => n,
            (Some(e), _) => e,
            (None, Some(n)) => n,
            (None, None) => u64::MAX,
        }
    }

    /// Every line being sung at `pos`: more than one where voices overlap.
    pub fn active(&self, pos: u64) -> Vec<usize> {
        if self.timing == Timing::Plain {
            return Vec::new();
        }
        // Lines are sorted by start; only those that started can be active.
        let started = self.lines.partition_point(|l| l.start_ms <= pos);
        (0..started).filter(|&i| pos < self.line_end(i)).collect()
    }

    /// The line to keep in view at `pos`: the first one being sung, else the
    /// last one that started (it stays in place through an instrumental gap).
    pub fn line_at(&self, pos: u64) -> Option<usize> {
        if self.timing == Timing::Plain {
            return None;
        }
        let started = self.lines.partition_point(|l| l.start_ms <= pos);
        (0..started).find(|&i| pos < self.line_end(i)).or_else(|| started.checked_sub(1))
    }

    /// The share of sung lines timed word by word (0–1).
    pub fn word_coverage(&self) -> f32 {
        let sung: Vec<&Line> = self.lines.iter().filter(|l| !l.text.trim().is_empty()).collect();
        if sung.is_empty() {
            return 0.0;
        }
        sung.iter().filter(|l| !l.tokens.is_empty()).count() as f32 / sung.len() as f32
    }
}

/// When a piece stops being sung: its own end, else the next piece's start,
/// else the end of its line (`line_end`), and never before it starts.
pub fn token_end(tokens: &[Token], k: usize, line_end: u64) -> u64 {
    let t = &tokens[k];
    let end = t.end_ms.filter(|&e| e > t.start_ms).or_else(|| tokens.get(k + 1).map(|n| n.start_ms)).unwrap_or(line_end.min(t.start_ms.saturating_add(1_500)));
    end.max(t.start_ms + 1)
}

/// How far along `tokens` the singing is at `pos`: each piece's share sung,
/// 0 (not yet) to 1 (done). Lit pieces stay lit through the gaps between them.
pub fn token_progress(tokens: &[Token], line_end: u64, pos: u64) -> impl Iterator<Item = f32> + '_ {
    (0..tokens.len()).map(move |k| {
        let t = &tokens[k];
        if pos < t.start_ms {
            return 0.0;
        }
        let end = token_end(tokens, k, line_end);
        ((pos - t.start_ms) as f32 / (end - t.start_ms) as f32).min(1.0)
    })
}

/// Text direction, from the first letter that has one: right to left for
/// Hebrew, Arabic, Syriac, Thaana, N'Ko and their kin.
pub fn is_rtl(text: &str) -> bool {
    for c in text.chars() {
        let u = c as u32;
        if matches!(u, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF) {
            return true;
        }
        if c.is_alphabetic() {
            return false;
        }
    }
    false
}

/// What playing music is, as far as lyrics care: enough to find them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct TrackRef {
    /// Stable identity: a Spotify URI or `local:` plus a file path.
    pub key: String,
    pub title: String,
    /// Every credited artist, main artist first.
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
    /// The recording's ISRC, when known (Spotify's metadata, a file's tags).
    #[serde(default)]
    pub isrc: String,
}

impl TrackRef {
    pub fn artist(&self) -> &str {
        self.artists.first().map(String::as_str).unwrap_or("")
    }
    /// Spotify's 22-character track ID, for Spotify songs.
    pub fn spotify_id(&self) -> Option<&str> {
        self.key.strip_prefix("spotify:track:").filter(|id| id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric()))
    }
    /// The local file a `local:` key names.
    pub fn local_path(&self) -> Option<&str> {
        self.key.strip_prefix("local:")
    }
}

/// A Spotify song (the Web API's track shape, as the player reports it) as
/// lyrics need it. Podcasts and episodes have none.
pub fn track_ref(item: &serde_json::Value) -> Option<TrackRef> {
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    if item["type"].as_str() != Some("track") || s(item, "uri").is_empty() {
        return None;
    }
    Some(TrackRef {
        key: s(item, "uri"),
        title: s(item, "name"),
        artists: item["artists"].as_array().map(|a| a.iter().map(|x| s(x, "name")).filter(|a| !a.is_empty()).collect()).unwrap_or_default(),
        album: s(&item["album"], "name"),
        duration_ms: item["duration_ms"].as_u64().unwrap_or(0),
        isrc: item["external_ids"]["isrc"].as_str().and_then(clean_isrc).unwrap_or_default(),
    })
}

/// An ISRC as the providers expect it: 12 letters and digits, upper case.
pub fn clean_isrc(s: &str) -> Option<String> {
    let s: String = s.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_ascii_uppercase();
    (s.len() == 12 && s.chars().all(|c| c.is_ascii_alphanumeric())).then_some(s)
}

/// A record the user can pick when the automatic match is unsure or wrong.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// The whole record (search results carry the lyrics), so picking it
    /// needs no further request.
    pub record: Arc<parse::Record>,
    pub source: Source,
    pub timing: Timing,
    pub instrumental: bool,
    /// The first sung line, to tell versions apart.
    pub preview: String,
}

/// What one provider found for one track.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Found {
        lyrics: Arc<Lyrics>,
        how: Match,
    },
    /// The provider says the recording has no vocals.
    Instrumental {
        source: Source,
    },
    NotFound {
        provider: ProviderId,
    },
    /// Several records fit about equally well and their lyrics differ.
    Ambiguous {
        provider: ProviderId,
        candidates: Vec<Candidate>,
    },
    /// The provider can't be used at all (and why).
    Unavailable {
        provider: ProviderId,
        reason: String,
    },
    /// The provider could be used but the lookup failed (network, server…).
    Failed {
        provider: ProviderId,
        error: String,
    },
}

/// How a record was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Match {
    /// The provider matched the recording itself (its own exact match, the
    /// ISRC, or Spotify's track ID).
    Exact,
    /// Picked from search results by Glass's own matching.
    Search,
    /// Picked by the user.
    Manual,
}

impl Outcome {
    pub fn provider(&self) -> ProviderId {
        match self {
            Outcome::Found { lyrics, .. } => lyrics.source.provider,
            Outcome::Instrumental { source } => source.provider,
            Outcome::NotFound { provider } | Outcome::Ambiguous { provider, .. } | Outcome::Unavailable { provider, .. } | Outcome::Failed { provider, .. } => *provider,
        }
    }
    /// Whether this ends the search: anything but "nothing here" and errors.
    pub fn settles(&self) -> bool {
        matches!(self, Outcome::Found { .. } | Outcome::Instrumental { .. } | Outcome::Ambiguous { .. })
    }
    pub fn short(&self) -> &'static str {
        match self {
            Outcome::Found { lyrics, .. } if lyrics.timing >= Timing::Word => "found word sync",
            Outcome::Found { .. } => "found",
            Outcome::Instrumental { .. } => "instrumental",
            Outcome::NotFound { .. } => "no match",
            Outcome::Ambiguous { .. } => "several matches",
            Outcome::Unavailable { .. } => "unavailable",
            Outcome::Failed { .. } => "failed",
        }
    }
}

/// Playback position shifted by the user's timing offset. A positive offset
/// shows lyrics earlier (for lyrics that lag the music).
pub fn offset_position(pos_ms: u64, offset_ms: i64) -> u64 {
    (pos_ms as i64).saturating_add(offset_ms).max(0) as u64
}

/// Where to seek so that line `start_ms` shows as current with this offset.
pub fn seek_target(start_ms: u64, offset_ms: i64) -> u64 {
    (start_ms as i64).saturating_sub(offset_ms).max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lyr(timing: Timing, lines: Vec<Line>) -> Lyrics {
        Lyrics::new(Source { provider: ProviderId::Lrclib, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 }, timing, lines)
    }

    fn line(start: u64, text: &str, tokens: &[(u64, &str)]) -> Line {
        Line { start_ms: start, text: text.into(), tokens: tokens.iter().map(|(t, w)| Token { text: w.to_string(), start_ms: *t, end_ms: None }).collect(), ..Default::default() }
    }

    fn ended(start: u64, end: u64, text: &str) -> Line {
        Line { start_ms: start, end_ms: Some(end), text: text.into(), ..Default::default() }
    }

    #[test]
    fn current_line_follows_position_through_seeks() {
        let l = lyr(Timing::Line, vec![line(1000, "one", &[]), line(5000, "two", &[]), line(9000, "three", &[])]);
        assert_eq!(l.line_at(0), None, "before the first line");
        assert_eq!(l.line_at(1000), Some(0));
        // Seek forward, then back: the current line is a function of position only.
        assert_eq!(l.line_at(9500), Some(2));
        assert_eq!(l.line_at(4999), Some(0));
        assert_eq!(l.line_at(5000), Some(1));
        assert_eq!(l.active(5000), vec![1], "line timing: one line at a time");
        assert_eq!(l.active(60_000), vec![2], "the last line lasts to the end");
    }

    #[test]
    fn plain_lyrics_have_no_current_line() {
        let l = lyr(Timing::Plain, vec![Line::untimed("a"), Line::untimed("b")]);
        assert_eq!(l.line_at(60_000), None);
        assert!(l.active(60_000).is_empty());
    }

    #[test]
    fn overlapping_voices_are_lit_together() {
        // Two singers: the second starts before the first has finished.
        let l = lyr(Timing::Word, vec![ended(1000, 4000, "first"), ended(3000, 6000, "second"), ended(9000, 10_000, "third")]);
        assert_eq!(l.active(2000), vec![0]);
        assert_eq!(l.active(3500), vec![0, 1], "both sung at once");
        assert_eq!(l.line_at(3500), Some(0), "the view follows the one that came in first");
        assert_eq!(l.active(4500), vec![1]);
        assert_eq!(l.line_at(4500), Some(1));
        // An instrumental break: nothing is lit, the view holds the last line.
        assert!(l.active(7000).is_empty());
        assert_eq!(l.line_at(7000), Some(1));
    }

    #[test]
    fn short_gaps_hold_the_line() {
        let l = lyr(Timing::Line, vec![ended(1000, 3000, "a"), ended(3600, 5000, "b"), ended(9000, 9500, "c")]);
        assert_eq!(l.line_end(0), 3600, "a 0.6 s gap keeps the first line lit");
        assert_eq!(l.line_end(1), 5000, "a 4 s gap doesn't");
        assert_eq!(l.line_end(2), 9500);
        assert_eq!(l.active(3300), vec![0]);
    }

    #[test]
    fn words_highlight_only_when_timed() {
        let l = lyr(Timing::Word, vec![line(1000, "one two", &[(1000, "one "), (1600, "two")]), line(3000, "untimed line", &[])]);
        let at = |i: usize, pos| token_progress(&l.lines[i].tokens, l.line_end(i), pos).collect::<Vec<f32>>();
        assert_eq!(at(0, 1599)[1], 0.0);
        assert!(at(0, 1600)[0] == 1.0 && at(0, 1700)[1] > 0.0);
        assert!(at(1, 3500).is_empty(), "a line the provider didn't time word by word is highlighted whole");
    }

    #[test]
    fn word_progress_sweeps_across_each_word() {
        let tokens = vec![Token { text: "one ".into(), start_ms: 1000, end_ms: Some(1400) }, Token { text: "two".into(), start_ms: 1600, end_ms: None }];
        let at = |pos| token_progress(&tokens, 2600, pos).collect::<Vec<f32>>();
        assert_eq!(at(900), vec![0.0, 0.0]);
        assert_eq!(at(1200), vec![0.5, 0.0], "halfway through the first word");
        assert_eq!(at(1500), vec![1.0, 0.0], "a sung word stays lit through the gap");
        assert_eq!(at(2100), vec![1.0, 0.5], "the last word runs to the end of its line");
        assert_eq!(at(9000), vec![1.0, 1.0]);
        // Seeking back un-sings words: progress is a function of position only.
        assert_eq!(at(1100)[0], 0.25);
        assert_eq!(token_end(&tokens, 1, u64::MAX), 3100, "no end anywhere: a bounded guess, never forever");
    }

    #[test]
    fn right_to_left_lines() {
        assert!(is_rtl("שלום עולם"));
        assert!(is_rtl("  «مرحبا»"));
        assert!(!is_rtl("Hello שלום"), "the first letter decides");
        assert!(!is_rtl("123"));
    }

    #[test]
    fn offsets_shift_lyrics_and_seeks() {
        assert_eq!(offset_position(10_000, 300), 10_300, "positive shows lyrics earlier");
        assert_eq!(offset_position(10_000, -300), 9_700);
        assert_eq!(offset_position(100, -300), 0, "never negative");
        let l = lyr(Timing::Line, vec![line(1000, "one", &[]), line(5000, "two", &[])]);
        // Lyrics running 500 ms late: with +500 the second line arrives at 4.5 s.
        assert_eq!(l.line_at(offset_position(4500, 500)), Some(1));
        // Clicking a line seeks to where it becomes current under the offset.
        assert_eq!(seek_target(5000, 500), 4500);
        assert_eq!(l.line_at(offset_position(seek_target(5000, 500), 500)), Some(1));
        assert_eq!(seek_target(200, 500), 0);
    }

    #[test]
    fn identifiers() {
        let t = TrackRef { key: "spotify:track:0VjIjW4GlUZAMYd2vXMi3b".into(), ..Default::default() };
        assert_eq!(t.spotify_id(), Some("0VjIjW4GlUZAMYd2vXMi3b"));
        assert_eq!(TrackRef { key: "local:/m/a.mp3".into(), ..Default::default() }.local_path(), Some("/m/a.mp3"));
        assert_eq!(clean_isrc(" us-ug1-19-04206 "), Some("USUG11904206".into()));
        assert_eq!(clean_isrc("nope"), None);
        for p in ProviderId::ALL {
            assert_eq!(ProviderId::parse(p.key()), Some(p));
        }
        let item = serde_json::json!({ "type": "track", "uri": "spotify:track:0VjIjW4GlUZAMYd2vXMi3b", "name": "Song", "artists": [{ "name": "A" }, { "name": "B" }], "album": { "name": "Album" }, "duration_ms": 200_000, "external_ids": { "isrc": "usug11904206" } });
        let t = track_ref(&item).unwrap();
        assert_eq!((t.artists.len(), t.isrc.as_str(), t.duration_ms), (2, "USUG11904206", 200_000));
        assert!(track_ref(&serde_json::json!({ "type": "episode", "uri": "spotify:episode:x" })).is_none());
    }

    #[test]
    fn timing_order() {
        assert!(Timing::Syllable > Timing::Word && Timing::Word > Timing::Line && Timing::Line > Timing::Plain);
    }
}
