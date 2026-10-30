//! Musixmatch's developer API (<https://developer.musixmatch.com>), with the
//! user's own API key. The recording is matched by ISRC when known, else by
//! title and artist, and checked like any other match. Word timing comes
//! from `track.richsync.get` (Musixmatch's Enterprise plan), line timing from
//! `track.subtitle.get` (Scale plan); a key without them answers "no match".
//!
//! Glass doesn't mint tokens as Musixmatch's own apps: without the user's key
//! this source is off.

use super::http::{Error, Http};
use super::matching::{self, Info};
use super::netease::respace;
use super::{Credit, Line, Lyrics, Match, Outcome, ProviderId, Source, Token, TrackRef, clean_isrc, parse};
use serde_json::Value;
use std::sync::Arc;

pub const BASE: &str = "https://api.musixmatch.com/ws/1.1";
const P: ProviderId = ProviderId::Musixmatch;
pub const NO_KEY: &str = "Add your Musixmatch API key in Settings to use it.";

fn status(v: &Value) -> u64 {
    v["message"]["header"]["status_code"].as_u64().unwrap_or(0)
}

/// richsync: `[{ "ts": 27.16, "te": 28.25, "x": "line", "l": [{ "c": "I've", "o": 0.03 }, { "c": " ", "o": 0.41 }, …] }]`,
/// `o` counted from the line's start; a space ends a word, and a word can
/// come in several pieces.
pub fn richsync(body: &str) -> Vec<Line> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let secs = |v: &Value| v.as_f64().filter(|s| s.is_finite() && *s >= 0.0).map(|s| (s * 1000.0).round() as u64);
    let mut out = Vec::new();
    for e in entries {
        let (Some(start), end) = (secs(&e["ts"]), secs(&e["te"])) else { continue };
        let mut tokens: Vec<Token> = Vec::new();
        let mut open = false;
        for piece in e["l"].as_array().into_iter().flatten() {
            let c = piece["c"].as_str().unwrap_or("");
            let at = start + secs(&piece["o"]).unwrap_or(0);
            if c.trim().is_empty() {
                if let Some(last) = tokens.last_mut() {
                    last.end_ms = Some(at.max(last.start_ms));
                    if !last.text.ends_with(' ') {
                        last.text.push(' ');
                    }
                }
                open = false;
                continue;
            }
            match tokens.last_mut() {
                Some(last) if open => last.text.push_str(c.trim()),
                _ => {
                    if let Some(last) = tokens.last_mut()
                        && last.end_ms.is_none()
                    {
                        last.end_ms = Some(at.max(last.start_ms));
                    }
                    tokens.push(Token { text: c.trim().to_string(), start_ms: at, end_ms: None });
                    open = true;
                }
            }
        }
        if let Some(last) = tokens.last_mut() {
            last.text = last.text.trim_end().to_string();
            if last.end_ms.is_none() {
                last.end_ms = end.filter(|&e| e > last.start_ms);
            }
        }
        if let Some(x) = e["x"].as_str() {
            respace(&mut tokens, x);
        }
        if tokens.is_empty() {
            continue;
        }
        let text = tokens.iter().map(|t| t.text.as_str()).collect();
        out.push(Line { start_ms: start, end_ms: end.filter(|&e| e > start), text, tokens, ..Default::default() });
    }
    out
}

pub async fn fetch(http: &Http, base: &str, key: Option<&str>, t: &TrackRef) -> Outcome {
    let Some(key) = key.filter(|k| !k.trim().is_empty()) else { return Outcome::Unavailable { provider: P, reason: NO_KEY.into() } };
    match lookup(http, base, key.trim(), t).await {
        Ok(o) => o,
        Err(e) => Outcome::Failed { provider: P, error: format!("Musixmatch {e}") },
    }
}

async fn call(http: &Http, base: &str, method: &str, key: &str, query: &[(&str, String)]) -> Result<Value, Error> {
    let mut q: Vec<(&str, String)> = vec![("format", "json".into()), ("apikey", key.to_string())];
    q.extend(query.iter().cloned());
    let r = http.get(&format!("{base}/{method}"), &q, &[]).await?;
    if r.status != 200 {
        return Err(Error::Http(r.status));
    }
    r.json()
}

async fn lookup(http: &Http, base: &str, key: &str, t: &TrackRef) -> Result<Outcome, Error> {
    let mut q = vec![("q_track", matching::plain_title(&t.title)), ("q_artist", t.artist().to_string())];
    if let Some(isrc) = clean_isrc(&t.isrc) {
        q.push(("track_isrc", isrc));
    }
    let v = call(http, base, "matcher.track.get", key, &q).await?;
    match status(&v) {
        200 => {}
        401 => return Ok(Outcome::Unavailable { provider: P, reason: "Musixmatch didn't accept the API key.".into() }),
        402 => return Ok(Outcome::Unavailable { provider: P, reason: "The Musixmatch key is over its usage limit.".into() }),
        _ => return Ok(Outcome::NotFound { provider: P }),
    }
    let tr = &v["message"]["body"]["track"];
    let s = |k: &str| tr[k].as_str().unwrap_or("").to_string();
    let (title, artist, album) = (s("track_name"), s("artist_name"), s("album_name"));
    let ms = tr["track_length"].as_u64().unwrap_or(0) * 1000;
    if matching::strict(t, &Info { title: &title, artist: &artist, album: &album, duration_ms: ms, ..Default::default() }).is_none() {
        return Ok(Outcome::NotFound { provider: P });
    }
    if tr["instrumental"].as_u64() == Some(1) {
        return Ok(Outcome::NotFound { provider: P });
    }
    let id = tr["track_id"].as_u64().unwrap_or(0).to_string();
    let source = Source { provider: P, id: id.clone(), title, artist, album, duration_ms: ms };
    let found = |lines: Vec<Line>, copyright: &str| {
        let mut lines = lines;
        let timing = parse::settle_tokens(&mut lines);
        let mut l = Lyrics::new(source.clone(), timing, lines);
        l.credits = vec![Credit { text: if copyright.trim().is_empty() { "Musixmatch".into() } else { copyright.trim().to_string() }, url: Some("https://www.musixmatch.com".into()) }];
        Outcome::Found { lyrics: Arc::new(l), how: if clean_isrc(&t.isrc).is_some() { Match::Exact } else { Match::Search } }
    };
    if tr["has_richsync"].as_u64() == Some(1) {
        let v = call(http, base, "track.richsync.get", key, &[("track_id", id.clone())]).await?;
        let rs = &v["message"]["body"]["richsync"];
        if status(&v) == 200
            && let Some(body) = rs["richsync_body"].as_str()
        {
            let lines = richsync(body);
            if !lines.is_empty() {
                return Ok(found(lines, rs["lyrics_copyright"].as_str().unwrap_or("")));
            }
        }
    }
    if tr["has_subtitles"].as_u64() == Some(1) {
        let v = call(http, base, "track.subtitle.get", key, &[("track_id", id), ("subtitle_format", "lrc".into())]).await?;
        let sub = &v["message"]["body"]["subtitle"];
        if status(&v) == 200
            && let Some(body) = sub["subtitle_body"].as_str()
        {
            let (lines, _) = parse::lrc(body);
            if !lines.is_empty() {
                return Ok(found(lines, sub["lyrics_copyright"].as_str().unwrap_or("")));
            }
        }
    }
    Ok(Outcome::NotFound { provider: P })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Timing;
    use crate::lyrics::http::tests::{reply, serve};
    use serde_json::json;

    const RICH: &str = r#"[{"ts":27.16,"te":28.25,"x":"I've been, calling","l":[{"c":"I've","o":0.03},{"c":" ","o":0.41},{"c":"been","o":0.42},{"c":",","o":0.6},{"c":" ","o":0.62},{"c":"call","o":0.7},{"c":"ing","o":0.9}]}]"#;

    #[test]
    fn richsync_words() {
        let l = richsync(RICH);
        assert_eq!(l[0].text, "I've been, calling");
        let w: Vec<(&str, u64, Option<u64>)> = l[0].tokens.iter().map(|t| (t.text.as_str(), t.start_ms, t.end_ms)).collect();
        assert_eq!(w, vec![("I've ", 27_190, Some(27_570)), ("been, ", 27_580, Some(27_780)), ("calling", 27_860, Some(28_250))]);
        let mut lines = l;
        assert_eq!(parse::settle_tokens(&mut lines), Timing::Word);
        assert!(richsync("not json").is_empty());
    }

    #[tokio::test]
    async fn needs_a_key_and_the_very_recording() {
        let h = Http::new("musixmatch", "", "", None);
        let t = TrackRef { key: "spotify:track:x".into(), title: "Song".into(), artists: vec!["Artist".into()], duration_ms: 200_000, ..Default::default() };
        assert!(matches!(fetch(&h, "http://127.0.0.1:9", None, &t).await, Outcome::Unavailable { reason, .. } if reason == NO_KEY));
        let matched = json!({ "message": { "header": { "status_code": 200 }, "body": { "track": { "track_id": 7, "track_name": "Song", "artist_name": "Artist", "track_length": 200, "has_richsync": 1, "instrumental": 0 } } } });
        let rich = json!({ "message": { "header": { "status_code": 200 }, "body": { "richsync": { "richsync_body": RICH, "lyrics_copyright": "Lyrics powered by www.musixmatch.com." } } } });
        let (base, server) = serve(vec![reply("200 OK", "application/json", "", &matched.to_string()), reply("200 OK", "application/json", "", &rich.to_string())]).await;
        let Outcome::Found { lyrics, .. } = fetch(&h, &base, Some("key123"), &t).await else { panic!() };
        assert_eq!((lyrics.timing, lyrics.credits[0].text.as_str()), (Timing::Word, "Lyrics powered by www.musixmatch.com."));
        let seen = server.await.unwrap();
        assert!(seen[0].contains("apikey=key123") && seen[0].contains("q_track=Song"));
        // A refused key says so.
        let refused = json!({ "message": { "header": { "status_code": 401 }, "body": [] } });
        let (base, _s) = serve(vec![reply("200 OK", "application/json", "", &refused.to_string())]).await;
        assert!(matches!(fetch(&h, &base, Some("bad"), &t).await, Outcome::Unavailable { .. }));
    }
}
