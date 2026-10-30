//! Sample lyrics for the demo (and screenshots): words written for Glass,
//! shaped like a lrc.red TTML document with everything word timing can do —
//! syllables, a second voice on the other side, two voices overlapping,
//! backing vocals — so the demo goes through the real parser and views.

use super::{Credit, Lyrics, Match, Outcome, ProviderId, Source, TrackRef, ttml};
use std::sync::Arc;

/// The demo's lines, each sung over `LINE_MS` from `start(i)`.
pub const LINES: [&str; 6] =
    ["City lights are slowly fading", "Every corner hums a tune", "We let the evening carry us", "drifting softly past the moon", "Count the windows, one by one", "till the morning finds us here"];
const LINE_MS: u64 = 4_600;

pub fn start(i: usize) -> u64 {
    52_000 + i as u64 * 5_200
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}

/// The words of `text` as spans from `from` to `to`, a syllable split where a word has a hyphen.
fn spans(text: &str, from: u64, to: u64) -> String {
    let pieces: Vec<(String, bool)> = text
        .split(' ')
        .flat_map(|w| {
            let parts: Vec<&str> = w.split('-').collect();
            let n = parts.len();
            parts.into_iter().enumerate().map(move |(k, p)| (p.to_string(), k + 1 == n)).collect::<Vec<_>>()
        })
        .collect();
    let each = (to - from) / pieces.len().max(1) as u64;
    let mut out = String::new();
    for (k, (p, word_end)) in pieces.iter().enumerate() {
        let b = from + k as u64 * each;
        out.push_str(&format!(r#"<span begin="{}" end="{}">{p}</span>"#, clock(b), clock(b + each)));
        if *word_end && k + 1 < pieces.len() {
            out.push(' ');
        }
    }
    out
}

/// The TTML document.
pub fn document(duration_ms: u64) -> String {
    let mut body = String::new();
    for (i, line) in LINES.iter().enumerate() {
        let b = start(i);
        // The third line is the second voice's, and the fourth comes in before it ends.
        let (agent, from, to) = match i {
            2 => ("v2", b, b + 6_000),
            5 => ("v3", b, b + LINE_MS),
            _ => ("v1", b, b + LINE_MS),
        };
        // "slow-ly" and "eve-ning": syllables.
        let words = line.replace("slowly", "slow-ly").replace("evening", "eve-ning");
        let mut p = spans(&words, from, to - 400);
        if i == 4 {
            p.push_str(&format!(r#" <span ttm:role="x-bg">{}</span>"#, spans("(one by one)", b + 2_600, to + 300)));
        }
        body.push_str(&format!(r#"<p begin="{}" end="{}" lrc:key="L{}" ttm:agent="{agent}">{p}</p>"#, clock(from), clock(to + if i == 4 { 300 } else { 0 }), i + 1));
    }
    format!(
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:lrc="http://lrc.red/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" lrc:timing="Word" xml:lang="en"><head><metadata><ttm:agent type="person" xml:id="v1"/><ttm:agent type="person" xml:id="v2"/><ttm:agent type="group" xml:id="v3"/></metadata></head><body dur="{}"><div>{body}</div></body></tt>"#,
        clock(duration_ms)
    )
}

/// The sample as lrc.red's answer for `t`.
pub fn outcome(t: &TrackRef) -> Outcome {
    let doc = ttml::parse(&document(t.duration_ms)).expect("the sample parses");
    let source = Source { provider: ProviderId::LrcRed, id: "DEMO00000001".into(), title: t.title.clone(), artist: t.artist().to_string(), album: t.album.clone(), duration_ms: t.duration_ms };
    let mut l = Lyrics::new(source, doc.timing, doc.lines);
    l.language = Some("en".into());
    l.credits = vec![Credit { text: "lrc.red".into(), url: Some("https://lrc.red".into()) }];
    Outcome::Found { lyrics: Arc::new(l), how: Match::Exact }
}

/// Japanese lines for the `cjk` demo, also written for Glass: a long one that
/// wraps, and one that mixes in English words.
pub const CJK_LINES: [&str; 6] = [
    "街の灯りがゆっくり消えていく",
    "どの角にも歌が聞こえる",
    "夕暮れに身をまかせて",
    "月のそばを静かに流れていく",
    "窓をひとつずつ数えながら、朝が来るまでここで二人で待っている",
    "Driving through the night 朝が見つけるまで",
];

/// The Japanese sample as NetEase's answer for `t`, timed a character at a
/// time the way NetEase's yrc is (English words whole), read by the real
/// yrc parser.
pub fn cjk_outcome(t: &TrackRef) -> Outcome {
    let mut yrc = String::new();
    for (i, line) in CJK_LINES.iter().enumerate() {
        let mut pieces: Vec<String> = Vec::new();
        for c in line.chars() {
            let latin = c.is_ascii_alphabetic() || c == ' ';
            match pieces.last_mut() {
                // English words keep their letters and the space after; "、" goes with the character before.
                Some(p) if (latin && p.ends_with(|x: char| x.is_ascii_alphabetic())) || (c == ' ' && !p.is_empty()) || c == '、' => p.push(c),
                _ => pieces.push(c.to_string()),
            }
        }
        let b = start(i);
        let each = (LINE_MS - 400) / pieces.len() as u64;
        yrc.push_str(&format!("[{b},{LINE_MS}]"));
        for (k, p) in pieces.iter().enumerate() {
            yrc.push_str(&format!("({},{each},0){p}", b + k as u64 * each));
        }
        yrc.push('\n');
    }
    let mut lines = super::netease::lines(&yrc, None);
    let timing = super::parse::settle_tokens(&mut lines);
    let source = Source { provider: ProviderId::NetEase, id: "0".into(), title: t.title.clone(), artist: t.artist().to_string(), album: t.album.clone(), duration_ms: t.duration_ms };
    let mut l = Lyrics::new(source, timing, lines);
    l.language = Some("ja".into());
    Outcome::Found { lyrics: Arc::new(l), how: Match::Search }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::{Timing, Voice};

    #[test]
    fn the_japanese_sample_is_timed_by_the_character() {
        let t = TrackRef { key: "spotify:track:demo".into(), title: "Demo".into(), artists: vec!["Glass".into()], duration_ms: 214_000, ..Default::default() };
        let Outcome::Found { lyrics, .. } = cjk_outcome(&t) else { panic!() };
        assert_eq!(lyrics.timing, Timing::Word);
        assert_eq!(lyrics.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), CJK_LINES);
        assert_eq!(lyrics.lines[0].tokens.len(), CJK_LINES[0].chars().count());
        assert_eq!(lyrics.lines[4].tokens[11].text, "ら、");
        assert_eq!(lyrics.lines[5].tokens.iter().take(4).map(|t| t.text.as_str()).collect::<Vec<_>>(), ["Driving ", "through ", "the ", "night "]);
    }

    #[test]
    fn the_sample_has_everything() {
        let t = TrackRef { key: "spotify:track:demo".into(), title: "Demo".into(), artists: vec!["Glass".into()], duration_ms: 214_000, ..Default::default() };
        let Outcome::Found { lyrics, .. } = outcome(&t) else { panic!() };
        assert_eq!(lyrics.timing, Timing::Syllable);
        assert_eq!(lyrics.lines.len(), 6);
        assert_eq!(lyrics.lines[0].text, "City lights are slowly fading");
        assert_eq!(lyrics.lines[2].voice, Voice::Other);
        assert_eq!(lyrics.lines[5].voice, Voice::Group);
        assert_eq!(lyrics.lines[4].backing.as_ref().unwrap().text, "one by one");
        assert_eq!(lyrics.active(start(3) + 100), vec![2, 3], "the second voice is still singing when the next line starts");
    }
}
