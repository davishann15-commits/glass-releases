//! NetEase Cloud Music's word timing ("yrc"), for songs the other sources
//! only time by the line. Keyless; searched by title and artist, and a
//! recording is only taken when it matches the song exactly, length included.
//!
//! yrc reads `[start,length](start,length,0)word (start,length,0)word …`, in
//! milliseconds from the start of the song. Pieces carry their own spacing,
//! though NetEase sometimes drops a space ("how" "to" → "howto"), so the
//! spacing is taken from NetEase's own line-synced copy of the same lyrics
//! where they agree. NetEase masks some words with asterisks, which is why
//! it's asked last.

use super::http::{Error, Http};
use super::matching::{self, Info};
use super::{Line, Lyrics, Match, Outcome, ProviderId, Source, Token, TrackRef, parse};
use serde_json::Value;
use std::sync::Arc;

pub const BASE: &str = "https://music.163.com";
const P: ProviderId = ProviderId::NetEase;
/// Recordings tried, best match first.
const TRIED: usize = 3;

fn headers() -> [(&'static str, String); 1] {
    [("Referer", "https://music.163.com".to_string())]
}

/// Search results: (id, title, artists, album, length in ms).
pub fn songs(v: &Value) -> Vec<(u64, String, String, String, u64)> {
    v["result"]["songs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let artists: Vec<&str> = s["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect()).unwrap_or_default();
                    Some((s["id"].as_u64()?, s["name"].as_str()?.to_string(), artists.join(", "), s["album"]["name"].as_str().unwrap_or("").to_string(), s["duration"].as_u64().unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One yrc line, or `None` for anything else (credits come as JSON lines).
fn yrc_line(row: &str) -> Option<Line> {
    let row = row.trim_end_matches(['\r', '\n']);
    let head = row.strip_prefix('[')?;
    let (times, mut rest) = head.split_once(']')?;
    let (start, len) = times.split_once(',')?;
    let (start, len): (u64, u64) = (start.trim().parse().ok()?, len.trim().parse().ok()?);
    let mut tokens: Vec<Token> = Vec::new();
    while let Some(body) = rest.strip_prefix('(') {
        let close = body.find(')')?;
        let mut nums = body[..close].split(',');
        let (Some(s), Some(l)) = (nums.next().and_then(|n| n.trim().parse::<u64>().ok()), nums.next().and_then(|n| n.trim().parse::<u64>().ok())) else { return None };
        let after = &body[close + 1..];
        let next = next_piece(after).unwrap_or(after.len());
        let text = &after[..next];
        if !text.is_empty() {
            tokens.push(Token { text: text.to_string(), start_ms: s, end_ms: Some(s + l.max(1)) });
        }
        rest = &after[next..];
    }
    if tokens.is_empty() {
        return None;
    }
    if let Some(f) = tokens.first_mut() {
        f.text = f.text.trim_start().to_string();
    }
    if let Some(l) = tokens.last_mut() {
        l.text = l.text.trim_end().to_string();
    }
    tokens.retain(|t| !t.text.is_empty());
    let text: String = tokens.iter().map(|t| t.text.as_str()).collect();
    Some(Line { start_ms: start, end_ms: Some(start + len).filter(|&e| e > start), text, tokens, ..Default::default() })
}

/// Where the next `(start,length,0)` begins in `s`; a parenthesis that isn't
/// one is text.
fn next_piece(s: &str) -> Option<usize> {
    s.match_indices('(').map(|(i, _)| i).find(|&i| {
        s[i + 1..].split_once(')').is_some_and(|(inner, _)| {
            let parts: Vec<&str> = inner.split(',').collect();
            parts.len() == 3 && parts.iter().all(|n| n.trim().trim_start_matches('-').parse::<u64>().is_ok())
        })
    })
}

/// Gives `tokens` the spacing and punctuation of `reference` when both spell
/// the same letters; otherwise leaves them as they are.
pub fn respace(tokens: &mut [Token], reference: &str) {
    if tokens.is_empty() || parse::squash(reference) != parse::squash(&tokens.iter().map(|t| t.text.as_str()).collect::<String>()) {
        return;
    }
    let chars: Vec<char> = reference.trim().chars().collect();
    let mut at = 0;
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    for (k, t) in tokens.iter().enumerate() {
        let want: Vec<char> = t.text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
        let mut s = String::new();
        // Text before the first letter of the line belongs to the first piece.
        let mut got = 0;
        while at < chars.len() && got < want.len() {
            let c = chars[at];
            if c.is_alphanumeric() {
                got += c.to_lowercase().count();
            }
            s.push(c);
            at += 1;
        }
        // Then everything up to the next letter: punctuation and the space after.
        if k + 1 < tokens.len() || at < chars.len() {
            while at < chars.len() && !chars[at].is_alphanumeric() {
                s.push(chars[at]);
                at += 1;
            }
        }
        out.push(s);
    }
    for (t, s) in tokens.iter_mut().zip(out) {
        t.text = s;
    }
    if let Some(l) = tokens.last_mut() {
        l.text = l.text.trim_end().to_string();
    }
}

/// yrc as lines, spaced like the line-synced `lrc` copy where they agree.
pub fn lines(yrc: &str, lrc: Option<&str>) -> Vec<Line> {
    let reference = lrc.map(|l| parse::lrc(l).0).unwrap_or_default();
    let mut out: Vec<Line> = yrc.lines().filter_map(yrc_line).collect();
    for l in &mut out {
        let squashed = parse::squash(&l.text);
        if let Some(r) = reference.iter().find(|r| r.start_ms.abs_diff(l.start_ms) <= 1_500 && parse::squash(&r.text) == squashed) {
            respace(&mut l.tokens, &r.text);
            // A piece that was only punctuation now lives in the word before it.
            l.tokens.retain(|t| !t.text.is_empty());
            l.text = l.tokens.iter().map(|t| t.text.as_str()).collect();
        }
    }
    out.sort_by_key(|l| l.start_ms);
    out
}

pub async fn fetch(http: &Http, base: &str, t: &TrackRef) -> Outcome {
    match lookup(http, base, t).await {
        Ok(o) => o,
        Err(e) => Outcome::Failed { provider: P, error: format!("NetEase {e}") },
    }
}

async fn lookup(http: &Http, base: &str, t: &TrackRef) -> Result<Outcome, Error> {
    if t.title.trim().is_empty() || t.artist().trim().is_empty() || t.duration_ms == 0 {
        return Ok(Outcome::NotFound { provider: P });
    }
    let q = format!("{} {}", matching::plain_title(&t.title), t.artist());
    let r = http.get(&format!("{base}/api/search/get"), &[("s", q), ("type", "1".into()), ("limit", "10".into())], &headers()).await?;
    if r.status != 200 {
        return Ok(Outcome::NotFound { provider: P });
    }
    let found = songs(&r.json()?);
    let mut ranked: Vec<(usize, f32)> =
        found.iter().enumerate().filter_map(|(i, (_, title, artist, album, ms))| matching::strict(t, &Info { title, artist, album, duration_ms: *ms, ..Default::default() }).map(|s| (i, s))).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, _) in ranked.into_iter().take(TRIED) {
        let (id, title, artist, album, ms) = &found[i];
        let r = http.get(&format!("{base}/api/song/lyric/v1"), &[("id", id.to_string()), ("lv", "1".into()), ("yv", "1".into()), ("tv", "-1".into())], &headers()).await?;
        if r.status != 200 {
            continue;
        }
        let v = r.json()?;
        let Some(yrc) = v["yrc"]["lyric"].as_str().filter(|s| !s.trim().is_empty()) else { continue };
        let mut lines = lines(yrc, v["lrc"]["lyric"].as_str());
        if lines.is_empty() {
            continue;
        }
        let timing = parse::settle_tokens(&mut lines);
        let source = Source { provider: P, id: id.to_string(), title: title.clone(), artist: artist.clone(), album: album.clone(), duration_ms: *ms };
        return Ok(Outcome::Found { lyrics: Arc::new(Lyrics::new(source, timing, lines)), how: Match::Search });
    }
    Ok(Outcome::NotFound { provider: P })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Timing;

    /// The shape of a real yrc answer (a credits line, then timed lines); the words are placeholders.
    const YRC: &str = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"},{\"tx\":\"Someone\"}]}\n[23550,120](23550,120,0)Yeah\n[27360,1290](27360,240,0)I've (27600,90,0)been (27690,360,0)trying (28050,600,0)calls\n[34230,1410](34230,390,0)how(34620,120,0)to (34740,900,0)love(35640,30,0), (35670,1950,0)maybe\n";
    const LRC: &str = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"}]}\n[00:23.500]Yeah\n[00:27.300]I've been trying calls\n[00:34.200]how to love, maybe\n";

    #[test]
    fn yrc_words_with_spacing_restored() {
        let l = lines(YRC, Some(LRC));
        assert_eq!(l.len(), 3, "the credits line isn't lyrics");
        assert_eq!(
            l[1].tokens.iter().map(|t| (t.text.as_str(), t.start_ms, t.end_ms)).collect::<Vec<_>>(),
            vec![("I've ", 27_360, Some(27_600)), ("been ", 27_600, Some(27_690)), ("trying ", 27_690, Some(28_050)), ("calls", 28_050, Some(28_650))]
        );
        assert_eq!((l[1].start_ms, l[1].end_ms), (27_360, Some(28_650)));
        assert_eq!(l[2].text, "how to love, maybe", "the dropped space comes back from the LRC copy");
        assert_eq!(l[2].tokens[0].text, "how ");
        let mut lines = l;
        assert_eq!(parse::settle_tokens(&mut lines), Timing::Word);
    }

    #[test]
    fn without_a_reference_the_text_is_kept_as_sent() {
        let l = lines(YRC, None);
        assert_eq!(l[2].text, "howto love, maybe");
        // A reference with other words doesn't touch them.
        let mut t = l[2].tokens.clone();
        respace(&mut t, "something else entirely");
        assert_eq!(t, l[2].tokens);
    }

    #[test]
    fn search_results() {
        let v: Value = serde_json::from_str(
            r#"{"result":{"songs":[{"id":1406633327,"name":"Blinding Lights","duration":200045,"artists":[{"name":"The Weeknd"}],"album":{"name":"Blinding Lights"}},{"name":"no id"}]},"code":200}"#,
        )
        .unwrap();
        assert_eq!(songs(&v), vec![(1406633327, "Blinding Lights".to_string(), "The Weeknd".to_string(), "Blinding Lights".to_string(), 200_045)]);
    }

    #[test]
    fn malformed_rows_are_skipped() {
        assert!(yrc_line("[12,5]").is_none());
        assert!(yrc_line("[x,5](1,2,0)a").is_none());
        assert!(yrc_line("plain text").is_none());
        let l = yrc_line("[1000,500](1000,200,0)a (1200,0,0)b").unwrap();
        assert_eq!(l.tokens[1].end_ms, Some(1201), "a piece of no length still shows for a moment");
        let l = yrc_line("[1000,500](1000,200,0)smile (:)(1200,300,0)ok").unwrap();
        assert_eq!(l.text, "smile (:)ok", "a parenthesis that isn't a time stays text");
    }

    /// `cargo test live_netease -- --ignored --nocapture` (needs the network).
    #[tokio::test]
    #[ignore]
    async fn live_netease() {
        let h = Http::new("music.163.com", "", "", None);
        let t = TrackRef { key: "live:bl".into(), title: "Blinding Lights".into(), artists: vec!["The Weeknd".into()], album: "After Hours".into(), duration_ms: 200_040, ..Default::default() };
        match fetch(&h, BASE, &t).await {
            Outcome::Found { lyrics, .. } => {
                println!("netease: {} lines, {}, e.g. {:?}", lyrics.lines.len(), lyrics.timing.label(), lyrics.lines.get(3).map(|l| &l.text));
                assert!(lyrics.timing >= Timing::Word);
            }
            o => panic!("{o:?}"),
        }
    }
}
