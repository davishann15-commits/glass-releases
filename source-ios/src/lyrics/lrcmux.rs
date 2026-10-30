//! lrcmux (<https://lrcmux.dev>): an open lyrics service that asks several
//! catalogs at once. Glass asks it only for word timing (`level=word`,
//! `strict`), which its public server gets from KuGou: wide coverage,
//! Spanish and other languages the other free catalogs often time only by
//! the line, though KuGou masks some words. Keyless, 60 requests a minute;
//! it asks clients to name themselves in the User-Agent.
//!
//! The answer says which recording it is (title, artist, album, length,
//! ISRC), and it's only taken when that matches the song exactly.

use super::http::{Error, Http};
use super::matching::{self, Info};
use super::{Credit, Line, Lyrics, Match, Outcome, ProviderId, Source, Token, TrackRef, clean_isrc, parse};
use serde_json::Value;
use std::sync::Arc;

pub const BASE: &str = "https://api.lrcmux.dev";
const P: ProviderId = ProviderId::Lrcmux;

fn ms(v: &Value) -> Option<u64> {
    v.as_i64().filter(|&n| n >= 0).map(|n| n as u64)
}

fn lines(v: &Value) -> Vec<Line> {
    let mut out: Vec<Line> = v["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let tokens: Vec<Token> = l["words"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| {
                    let start = ms(&w["start"])?;
                    let text = w["text"].as_str().filter(|t| !t.is_empty())?;
                    Some(Token { text: text.to_string(), start_ms: start, end_ms: ms(&w["end"]).filter(|&e| e >= start) })
                })
                .collect();
            let start = ms(&l["start"]).or(tokens.first().map(|t| t.start_ms))?;
            let text = if tokens.is_empty() { l["text"].as_str().unwrap_or("").trim().to_string() } else { tokens.iter().map(|t| t.text.as_str()).collect::<String>().trim().to_string() };
            Some(Line { start_ms: start, end_ms: ms(&l["end"]).filter(|&e| e > start), text, tokens, ..Default::default() })
        })
        .collect();
    for l in &mut out {
        if let Some(t) = l.tokens.last_mut() {
            t.text = t.text.trim_end().to_string();
        }
    }
    out.sort_by_key(|l| l.start_ms);
    out
}

/// `{ track: { title, artist, album, duration (s), isrc }, meta: { level, source }, lines }`,
/// if it's the very recording `t`.
pub fn answer(v: &Value, t: &TrackRef) -> Option<Outcome> {
    let track = &v["track"];
    let s = |k: &str| track[k].as_str().unwrap_or("").to_string();
    let (title, artist, album) = (s("title"), s("artist"), s("album"));
    let duration_ms = track["duration"].as_u64().unwrap_or(0) * 1000;
    let same_isrc = clean_isrc(&t.isrc).is_some() && clean_isrc(&t.isrc) == clean_isrc(&s("isrc"));
    if !same_isrc {
        matching::strict(t, &Info { title: &title, artist: &artist, album: &album, duration_ms, ..Default::default() })?;
    }
    let mut lines = lines(v);
    if lines.iter().all(|l| l.text.is_empty()) {
        return None;
    }
    let timing = parse::settle_tokens(&mut lines);
    let via = v["meta"]["source"]["name"].as_str().filter(|n| !n.is_empty());
    // The recording's ISRC, else its title and artist: saved translations are kept by this ID.
    let song = clean_isrc(&s("isrc")).unwrap_or_else(|| format!("{title}|{artist}"));
    let source = Source { provider: P, id: format!("{}:{song}", v["meta"]["source"]["id"].as_str().unwrap_or("")), title, artist, album, duration_ms };
    let mut l = Lyrics::new(source, timing, lines);
    l.credits = vec![Credit { text: via.map(|n| format!("lrcmux · {n}")).unwrap_or_else(|| "lrcmux".into()), url: Some("https://lrcmux.dev".into()) }];
    Some(Outcome::Found { lyrics: Arc::new(l), how: if same_isrc { Match::Exact } else { Match::Search } })
}

pub async fn fetch(http: &Http, base: &str, t: &TrackRef) -> Outcome {
    if t.title.trim().is_empty() || t.artist().trim().is_empty() || t.duration_ms == 0 {
        return Outcome::NotFound { provider: P };
    }
    let mut q =
        vec![("artist", t.artist().to_string()), ("title", matching::plain_title(&t.title)), ("duration", (t.duration_ms / 1000).to_string()), ("level", "word".into()), ("strict", "true".into())];
    if !t.album.is_empty() {
        q.push(("album", t.album.clone()));
    }
    if let Some(isrc) = clean_isrc(&t.isrc) {
        q.push(("isrc", isrc));
    }
    let r = match http.get(&format!("{base}/get"), &q, &[("Accept", "application/json".into())]).await {
        Ok(r) => r,
        Err(e) => return Outcome::Failed { provider: P, error: format!("lrcmux {e}") },
    };
    match r.status {
        200 => {}
        // No word timing anywhere (or no lyrics at all).
        400 | 404 => return Outcome::NotFound { provider: P },
        code => return Outcome::Failed { provider: P, error: format!("lrcmux {}", Error::Http(code)) },
    }
    match r.json() {
        Ok(v) => answer(&v, t).unwrap_or(Outcome::NotFound { provider: P }),
        Err(e) => Outcome::Failed { provider: P, error: format!("lrcmux {e}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Timing;
    use crate::lyrics::http::tests::{reply, serve};
    use serde_json::json;

    fn song() -> TrackRef {
        TrackRef { key: "spotify:track:x".into(), title: "Canción".into(), artists: vec!["Cantante".into()], album: "Disco".into(), duration_ms: 244_973, ..Default::default() }
    }

    /// The documented JSON (api.lrcmux.dev/openapi.json); placeholder words.
    fn found(duration: u64, album: &str) -> Value {
        json!({ "track": { "title": "Canción", "artist": "Cantante", "album": album, "duration": duration, "isrc": "USXX00000001", "cover": {} },
            "meta": { "level": "word", "source": { "id": "kugou", "name": "KuGou", "url": "https://kugou.com" } },
            "lines": [
                { "start": 28196, "end": 30200, "text": "Y ponerte a dormir", "words": [
                    { "text": "Y ", "start": 28196, "end": 28427 }, { "text": "ponerte ", "start": 28427, "end": 29216 },
                    { "text": "a ", "start": 29216, "end": 29809 }, { "text": "dormir", "start": 29809, "end": 30200 } ] },
                { "start": 32716, "end": 35901, "text": "Es alfa", "words": [ { "text": "Es ", "start": 32716, "end": 33326 }, { "text": "alfa", "start": 33326, "end": 35901 } ] }
            ] })
    }

    #[test]
    fn words_of_the_very_recording() {
        let Some(Outcome::Found { lyrics, how }) = answer(&found(244, "Disco"), &song()) else { panic!() };
        assert_eq!((how, lyrics.timing, lyrics.unconfirmed), (Match::Search, Timing::Word, false));
        assert_eq!(lyrics.lines[0].text, "Y ponerte a dormir");
        assert_eq!(lyrics.lines[0].tokens[3], Token { text: "dormir".into(), start_ms: 29_809, end_ms: Some(30_200) });
        assert_eq!(lyrics.credits[0].text, "lrcmux · KuGou");
        assert_eq!(lyrics.source.id, "kugou:USXX00000001");
        let mut no_isrc = found(244, "Disco");
        no_isrc["track"]["isrc"] = json!("");
        let Some(Outcome::Found { lyrics, .. }) = answer(&no_isrc, &song()) else { panic!() };
        assert_eq!(lyrics.source.id, "kugou:Canción|Cantante", "another song without one has another ID");
        assert!(answer(&found(297, "Live Sessions"), &song()).is_none(), "another recording, 52 s longer");
        let by_isrc = TrackRef { isrc: "USXX00000001".into(), ..song() };
        assert!(matches!(answer(&found(244, "Greatest Hits"), &by_isrc), Some(Outcome::Found { how: Match::Exact, .. })), "the same ISRC on a compilation");
    }

    #[tokio::test]
    async fn asks_only_for_word_timing() {
        let (base, server) = serve(vec![reply("404 Not Found", "application/problem+json", "", r#"{"title":"Not Found","status":404,"detail":"no lyrics found"}"#)]).await;
        let h = Http::new("lrcmux", "", "", None);
        assert_eq!(fetch(&h, &base, &song()).await, Outcome::NotFound { provider: P });
        let seen = server.await.unwrap();
        assert!(seen[0].starts_with("GET /get?") && seen[0].contains("level=word") && seen[0].contains("strict=true") && seen[0].contains("duration=244"), "{}", seen[0]);
    }

    /// `cargo test live_lrcmux -- --ignored --nocapture` (needs the network).
    #[tokio::test]
    #[ignore]
    async fn live_lrcmux() {
        let h = Http::new("api.lrcmux.dev", "", "", None);
        let t = TrackRef { key: "live:primaveral".into(), title: "Primaveral".into(), artists: vec!["Mon Laferte".into()], album: "La Trenza".into(), duration_ms: 231_653, ..Default::default() };
        match fetch(&h, BASE, &t).await {
            Outcome::Found { lyrics, .. } => {
                println!("lrcmux: {} lines, {}, {:?}, e.g. {:?}", lyrics.lines.len(), lyrics.timing.label(), lyrics.credits[0].text, lyrics.lines.get(3).map(|l| &l.text));
                assert!(lyrics.timing >= Timing::Word);
            }
            o => panic!("{o:?}"),
        }
    }
}
