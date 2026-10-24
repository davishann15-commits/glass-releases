//! Lyrics: one model for every provider, and how it lines up with playback.
//!
//! Providers hand back very different things — LRCLIB a JSON record with a
//! Lyricsfile (YAML), LRC text and plain text. Everything is normalized into
//! [`Lyrics`]: lines, and for each line the timed words or syllables only when
//! the provider actually timed them. Nothing here invents timestamps: a line
//! without timed words is highlighted as a whole, and plain lyrics aren't
//! highlighted at all.

pub mod cache;
pub mod choices;
pub mod lrclib;
pub mod matching;
pub mod parse;
pub mod service;
pub mod spotify;
pub mod translate;

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    Lrclib,
    Spotify,
}

impl ProviderId {
    /// The lowercase name shown in the lyrics view's status line.
    pub fn name(self) -> &'static str {
        match self {
            ProviderId::Lrclib => "lrclib",
            ProviderId::Spotify => "spotify",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ProviderId::Lrclib => "LRCLIB",
            ProviderId::Spotify => "Spotify",
        }
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
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Meaningless for plain lyrics.
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
    /// Timed words or syllables; empty when the provider only timed the line.
    pub tokens: Vec<Token>,
}

impl Line {
    pub fn untimed(text: impl Into<String>) -> Self {
        Line { start_ms: 0, end_ms: None, text: text.into(), tokens: Vec::new() }
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

#[derive(Clone, Debug, PartialEq)]
pub struct Lyrics {
    pub source: Source,
    /// ISO 639-1, when the provider says.
    pub language: Option<String>,
    pub timing: Timing,
    pub lines: Vec<Line>,
    /// Translations the provider supplied: (language, one entry per line).
    pub translations: Vec<(String, Vec<String>)>,
}

impl Lyrics {
    /// The line being sung at `pos` (already offset), if any has started.
    pub fn line_at(&self, pos: u64) -> Option<usize> {
        if self.timing == Timing::Plain {
            return None;
        }
        self.lines.iter().rposition(|l| l.start_ms <= pos)
    }

    /// The word or syllable being sung at `pos` within line `i`, if the
    /// provider timed them.
    pub fn token_at(&self, i: usize, pos: u64) -> Option<usize> {
        self.lines.get(i)?.tokens.iter().rposition(|t| t.start_ms <= pos)
    }
}

/// What playing music is, as far as lyrics care: enough to find them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackRef {
    /// Stable identity: a Spotify URI or `local:` plus a file path.
    pub key: String,
    pub title: String,
    /// Every credited artist, main artist first.
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
}

impl TrackRef {
    pub fn artist(&self) -> &str {
        self.artists.first().map(String::as_str).unwrap_or("")
    }
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
    /// The provider matched title, artist, album and duration itself.
    Exact,
    /// Picked from search results by Glass's own matching.
    Search,
    /// Picked by the user.
    Manual,
}

impl Outcome {
    #[cfg(test)]
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
        Lyrics {
            source: Source { provider: ProviderId::Lrclib, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 },
            language: None,
            timing,
            lines,
            translations: Vec::new(),
        }
    }

    fn line(start: u64, text: &str, tokens: &[(u64, &str)]) -> Line {
        Line { start_ms: start, end_ms: None, text: text.into(), tokens: tokens.iter().map(|(t, w)| Token { text: w.to_string(), start_ms: *t, end_ms: None }).collect() }
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
    }

    #[test]
    fn plain_lyrics_have_no_current_line() {
        let l = lyr(Timing::Plain, vec![Line::untimed("a"), Line::untimed("b")]);
        assert_eq!(l.line_at(60_000), None);
    }

    #[test]
    fn words_highlight_only_when_timed() {
        let l = lyr(Timing::Word, vec![line(1000, "one two", &[(1000, "one "), (1600, "two")]), line(3000, "untimed line", &[])]);
        assert_eq!(l.token_at(0, 1599), Some(0));
        assert_eq!(l.token_at(0, 1600), Some(1));
        assert_eq!(l.token_at(1, 3500), None, "a line the provider didn't time word by word is highlighted whole");
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
    fn timing_order() {
        assert!(Timing::Syllable > Timing::Word && Timing::Word > Timing::Line && Timing::Line > Timing::Plain);
    }
}
