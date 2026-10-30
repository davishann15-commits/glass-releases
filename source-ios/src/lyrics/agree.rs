//! Whether word timing keeps time with the song: a word-timed answer is
//! compared with the song's line-synced lyrics (LRCLIB's, or Spotify's) line
//! by line. Some services time a song to another version of it (another
//! master, a radio edit), seconds off throughout; those answers are caught
//! here instead of sweeping words at the wrong moments.
//!
//! Also fixes a NetEase habit: Japanese lyrics written with some simplified
//! Chinese characters (觉 for 覚, 无 for 無). When the line-synced lyrics
//! have the same line, their characters are used.

use super::{Lyrics, parse};

/// Lines that must be compared before the timing is judged.
pub const MIN_LINES: usize = 4;
/// How far apart the same lines may start and still be the same timing.
/// Word timing starts a little after a line's time (when the first word is
/// sung), so a few hundred ms is normal.
pub const AGREE_MS: i64 = 1_500;

/// Letters and digits, as written.
fn letters(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// A Han character (kanji, hanzi), the kind NetEase writes differently.
fn han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

fn same_letter(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// The same words, allowing a few Han characters written another way.
fn same_text(a: &[char], b: &[char]) -> bool {
    if a.len() < 2 || a.len() != b.len() {
        return false;
    }
    let mut differ = 0;
    for (&x, &y) in a.iter().zip(b) {
        if !same_letter(x, y) {
            if !(han(x) && han(y)) {
                return false;
            }
            differ += 1;
        }
    }
    differ * 4 <= a.len()
}

/// How much later `a`'s lines start than the same lines in `b` (the median, in
/// ms), when at least `MIN_LINES` lines are in both.
pub fn offset(a: &Lyrics, b: &Lyrics) -> Option<i64> {
    let theirs: Vec<(u64, Vec<char>)> = b.lines.iter().map(|l| (l.start_ms, letters(&l.text))).collect();
    let mut apart: Vec<i64> = a
        .lines
        .iter()
        .filter_map(|l| {
            let mine = letters(&l.text);
            // Choruses repeat: the nearest line with the same words.
            theirs.iter().filter(|(_, t)| same_text(&mine, t)).map(|(s, _)| l.start_ms as i64 - *s as i64).min_by_key(|d| d.abs())
        })
        .collect();
    if apart.len() < MIN_LINES {
        return None;
    }
    apart.sort_unstable();
    Some(apart[apart.len() / 2])
}

/// Whether `a` has the words of `b`, for lyrics without timing to compare: at least
/// `MIN_LINES` of `a`'s lines, and two thirds of them, are lines of `b`.
pub fn same_words(a: &Lyrics, b: &Lyrics) -> bool {
    let theirs: Vec<Vec<char>> = b.lines.iter().map(|l| letters(&l.text)).collect();
    let mine: Vec<Vec<char>> = a.lines.iter().map(|l| letters(&l.text)).filter(|l| l.len() >= 2).collect();
    let shared = mine.iter().filter(|m| theirs.iter().any(|t| same_text(m, t))).count();
    shared >= MIN_LINES && shared * 3 >= mine.len() * 2
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// The same lines start within `AGREE_MS`.
    Agrees,
    /// Timed to something else: this many ms later (negative: earlier).
    Disagrees(i64),
    /// Too few lines in common to tell.
    Unknown,
}

pub fn check(words: &Lyrics, reference: &Lyrics) -> Check {
    match offset(words, reference) {
        Some(d) if d.abs() <= AGREE_MS => Check::Agrees,
        Some(d) => Check::Disagrees(d),
        None => Check::Unknown,
    }
}

/// Writes `words`' Han characters as `reference` writes them, in lines that
/// are the same apart from those characters. Nothing else changes.
pub fn respell(words: &mut Lyrics, reference: &Lyrics) {
    let theirs: Vec<(u64, Vec<char>)> = reference.lines.iter().map(|l| (l.start_ms, letters(&l.text))).collect();
    for l in &mut words.lines {
        let mine = letters(&l.text);
        if !mine.iter().copied().any(han) {
            continue;
        }
        let Some(want) = theirs.iter().filter(|(_, t)| same_text(&mine, t) && *t != mine).min_by_key(|(s, _)| (l.start_ms as i64 - *s as i64).abs()).map(|(_, t)| t.clone()) else {
            continue;
        };
        let mut k = 0;
        let mut fix = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if !c.is_alphanumeric() {
                        return c;
                    }
                    let w = want.get(k).copied().unwrap_or(c);
                    k += 1;
                    if c != w && han(c) && han(w) { w } else { c }
                })
                .collect()
        };
        if l.tokens.is_empty() {
            l.text = fix(&l.text);
        } else {
            for t in &mut l.tokens {
                t.text = fix(&t.text);
            }
            l.text = l.tokens.iter().map(|t| t.text.as_str()).collect();
        }
    }
    // The pieces' spacing is unchanged, so the timing precision is too.
    debug_assert!(words.lines.iter().all(|l| l.tokens.is_empty() || parse::squash(&l.text) == parse::squash(&l.tokens.iter().map(|t| t.text.as_str()).collect::<String>())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::{Line, ProviderId, Source, Timing, Token};

    fn lyr(lines: &[(u64, &str)], words: bool) -> Lyrics {
        let source = Source { provider: ProviderId::NetEase, id: "1".into(), title: "t".into(), artist: "a".into(), album: String::new(), duration_ms: 0 };
        let lines = lines
            .iter()
            .map(|&(s, t)| {
                let tokens = if words { t.chars().enumerate().map(|(i, c)| Token { text: c.to_string(), start_ms: s + i as u64 * 100, end_ms: None }).collect() } else { Vec::new() };
                Line { start_ms: s, text: t.into(), tokens, ..Default::default() }
            })
            .collect();
        Lyrics::new(source, if words { Timing::Word } else { Timing::Line }, lines)
    }

    const LINES: [&str; 6] = ["Es escalofriante", "Tenerte de frente", "Daría cualquier cosa", "Disfruto", "Tenerte de frente", "Daría cualquier cosa"];

    fn at(starts: &[u64]) -> Vec<(u64, &'static str)> {
        starts.iter().copied().zip(LINES).collect()
    }

    #[test]
    fn the_same_timing_agrees_and_another_versions_doesnt() {
        let line = lyr(&at(&[31_960, 35_930, 43_100, 60_000, 90_000, 97_000]), false);
        // Word timing a little after each line's time: the same recording.
        let words = lyr(&at(&[32_716, 36_701, 43_500, 60_600, 90_400, 97_300]), true);
        assert_eq!(check(&words, &line), Check::Agrees);
        // Three seconds late throughout: timed to something else.
        let late = lyr(&at(&[35_170, 38_850, 46_100, 63_000, 93_000, 100_000]), true);
        assert_eq!(check(&late, &line), Check::Disagrees(3_000));
        // A repeated line is compared with its nearest repeat, not the first.
        assert_eq!(offset(&words, &line), Some(600));
        // Too little in common to tell.
        let other = lyr(&[(1_000, "something else"), (2_000, "entirely different"), (3_000, "Disfruto")], true);
        assert_eq!(check(&other, &line), Check::Unknown);
    }

    #[test]
    fn the_same_words_without_timing() {
        let plain = |lines: &[&str]| lyr(&lines.iter().map(|&t| (0, t)).collect::<Vec<_>>(), false);
        let words = lyr(&at(&[32_716, 36_701, 43_500, 60_600, 90_400, 97_300]), true);
        assert!(same_words(&words, &plain(&LINES)));
        assert!(same_words(&words, &plain(&["es escalofriante!", "Tenerte de frente", "Daría cualquier cosa", "Disfruto"])), "punctuation and case aside");
        assert!(!same_words(&words, &plain(&["Tenerte de frente", "Otra canción", "Disfruto"])), "a few lines in common isn't the same lyrics");
    }

    #[test]
    fn netease_kanji_are_written_as_the_line_lyrics_write_them() {
        let reference = lyr(&[(25_860, "やすらぎ覚まされた 昼下がりよ"), (33_590, "しばらく音沙汰も無かったのに"), (44_890, "勝手な人...")], false);
        let mut words = lyr(&[(25_840, "やすらぎ觉まされた 昼下がりよ"), (33_620, "しばらく音沙汰も无かったのに"), (44_940, "胜手な人…"), (50_000, "觉えてる")], true);
        respell(&mut words, &reference);
        let text: Vec<&str> = words.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(text, ["やすらぎ覚まされた 昼下がりよ", "しばらく音沙汰も無かったのに", "勝手な人…", "觉えてる"], "a line the reference doesn't have stays as it was");
        assert_eq!(words.lines[0].tokens[4].text, "覚", "the timed piece itself");
        assert_eq!(words.lines[0].tokens.len(), 15);
        // Other differences (another word) aren't touched.
        let mut words = lyr(&[(1_000, "觉まされた")], true);
        respell(&mut words, &lyr(&[(1_000, "起こされた")], false));
        assert_eq!(words.lines[0].text, "觉まされた");
    }
}
