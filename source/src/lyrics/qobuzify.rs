//! Qobuzify (<https://api.qobuzify.app>): a free lyrics API that answers in
//! Spicy Lyrics' format — syllable timing with duets and backing vocals,
//! else line timing — and covers many songs the other free catalogs only
//! time by the line (Spanish and Japanese ones especially). No key is needed
//! (15 requests a minute, plenty for one lookup per song).
//!
//! Spotify songs are looked up by their track ID; other songs by title,
//! artist, album, length and ISRC. Qobuzify doesn't say which recording its
//! lyrics were timed to (it repeats what it was asked), and some answers are
//! timed to another version of the song, seconds off. So its answers are
//! marked `unconfirmed`: their word timing is only used when it agrees with
//! the song's line-synced lyrics (see `agree`).

use super::http::{Error, Http};
use super::{Credit, Lyrics, Match, Outcome, ProviderId, Source, TrackRef, spicy};
use serde_json::Value;
use std::sync::Arc;

pub const BASE: &str = "https://api.qobuzify.app";
const P: ProviderId = ProviderId::Qobuzify;

/// The question for `t`: its Spotify ID, or what Glass knows about it.
fn query(t: &TrackRef) -> Vec<(&'static str, String)> {
    if let Some(id) = t.spotify_id() {
        return vec![("spotifyId", id.to_string())];
    }
    let mut q = vec![("name", t.title.clone()), ("artist", t.artist().to_string())];
    if !t.album.is_empty() {
        q.push(("album", t.album.clone()));
    }
    if t.duration_ms > 0 {
        q.push(("durationMs", t.duration_ms.to_string()));
    }
    if !t.isrc.is_empty() {
        q.push(("isrc", t.isrc.clone()));
    }
    q
}

/// `{ ok, track, lyrics: { available, type, source, data: { Type, Content… } } }`.
pub fn answer(v: &Value, t: &TrackRef) -> Option<Outcome> {
    let l = &v["lyrics"];
    if v["ok"] != Value::Bool(true) || l["available"] != Value::Bool(true) {
        return None;
    }
    let (lines, timing) = spicy::lines(&l["data"])?;
    let id = v["key"].as_str().or(t.spotify_id()).unwrap_or("").to_string();
    let source = Source { provider: P, id, title: t.title.clone(), artist: t.artist().to_string(), album: t.album.clone(), duration_ms: t.duration_ms };
    let mut out = Lyrics::new(source, timing, lines);
    out.unconfirmed = true;
    out.credits = vec![Credit { text: "Qobuzify".into(), url: Some(BASE.to_string()) }];
    let how = if t.spotify_id().is_some() { Match::Exact } else { Match::Search };
    Some(Outcome::Found { lyrics: Arc::new(out), how })
}

pub async fn fetch(http: &Http, base: &str, t: &TrackRef) -> Outcome {
    if t.spotify_id().is_none() && (t.title.trim().is_empty() || t.artist().trim().is_empty()) {
        return Outcome::NotFound { provider: P };
    }
    let r = match http.get(&format!("{base}/v2/track"), &query(t), &[("Accept", "application/json".into())]).await {
        Ok(r) => r,
        Err(e) => return Outcome::Failed { provider: P, error: format!("Qobuzify {e}") },
    };
    match r.status {
        200 => {}
        // A Spotify ID it can't resolve, or nothing to go on.
        400 | 404 => return Outcome::NotFound { provider: P },
        code => return Outcome::Failed { provider: P, error: format!("Qobuzify {}", Error::Http(code)) },
    }
    match r.json() {
        Ok(v) => answer(&v, t).unwrap_or(Outcome::NotFound { provider: P }),
        Err(e) => Outcome::Failed { provider: P, error: format!("Qobuzify {e}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Timing;
    use crate::lyrics::http::tests::{reply, serve};
    use serde_json::json;

    fn song() -> TrackRef {
        TrackRef { key: "spotify:track:50WkRwFR0BAo6If7GtXnA8".into(), title: "Song".into(), artists: vec!["Singer".into()], album: "Album".into(), duration_ms: 245_000, ..Default::default() }
    }

    /// The shape of a real answer; placeholder words.
    fn found() -> Value {
        json!({ "ok": true, "cached": "d1", "key": "sp:50WkRwFR0BAo6If7GtXnA8",
            "track": { "name": "Song", "artist": "Singer", "album": "Album", "durationMs": 244973, "isrc": "USXX00000001", "spotifyId": "50WkRwFR0BAo6If7GtXnA8" },
            "lyrics": { "available": true, "synced": true, "type": "Syllable", "source": "Lingua Franca", "data": { "Type": "Syllable", "StartTime": 35.17, "Content": [
                { "Type": "Vocal", "OppositeAligned": false, "Lead": { "StartTime": 35.17, "EndTime": 37.27, "Syllables": [
                    { "Text": "Es", "StartTime": 35.17, "EndTime": 35.753, "IsPartOfWord": false },
                    { "Text": "alfa", "StartTime": 35.753, "EndTime": 37.27, "IsPartOfWord": false } ] } },
                { "Type": "Vocal", "OppositeAligned": true, "Lead": { "StartTime": 38.85, "EndTime": 40.0, "Syllables": [
                    { "Text": "bra", "StartTime": 38.85, "EndTime": 39.4, "IsPartOfWord": true },
                    { "Text": "vo", "StartTime": 39.4, "EndTime": 40.0, "IsPartOfWord": false } ] } }
            ] } },
            "songwriters": { "available": false } })
    }

    #[test]
    fn syllables_from_its_answer() {
        let Some(Outcome::Found { lyrics, how }) = answer(&found(), &song()) else { panic!() };
        assert_eq!((how, lyrics.timing, lyrics.unconfirmed), (Match::Exact, Timing::Syllable, true), "the recording isn't confirmed, so the timing gets checked");
        assert_eq!(lyrics.lines[0].text, "Es alfa");
        assert_eq!(lyrics.lines[1].tokens.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["bra", "vo"]);
        assert_eq!(lyrics.lines[1].voice, crate::lyrics::Voice::Other);
        assert_eq!(lyrics.credits[0].text, "Qobuzify");
        assert!(answer(&json!({ "ok": true, "lyrics": { "available": false } }), &song()).is_none());
        assert!(answer(&json!({ "ok": false, "error": { "code": "bad_request" } }), &song()).is_none());
    }

    #[tokio::test]
    async fn spotify_songs_by_id_others_by_what_glass_knows() {
        let (base, server) = serve(vec![
            reply("200 OK", "application/json", "", &found().to_string()),
            reply("200 OK", "application/json", "", r#"{"ok":true,"lyrics":{"available":false}}"#),
            reply("400 Bad Request", "application/json", "", r#"{"ok":false}"#),
        ])
        .await;
        let h = Http::new("qobuzify", "", "", None);
        assert!(matches!(fetch(&h, &base, &song()).await, Outcome::Found { .. }));
        let local = TrackRef { key: "local:/m/a.m4a".into(), title: "SHYNESS BOY".into(), artists: vec!["Anri".into()], album: "Timely!!".into(), duration_ms: 196_666, isrc: "JPFL08310300".into() };
        assert_eq!(fetch(&h, &base, &local).await, Outcome::NotFound { provider: P });
        assert_eq!(fetch(&h, &base, &song()).await, Outcome::NotFound { provider: P }, "a refused question is no match");
        let seen = server.await.unwrap();
        assert!(seen[0].starts_with("GET /v2/track?spotifyId=50WkRwFR0BAo6If7GtXnA8 "), "{}", seen[0]);
        assert!(seen[1].contains("name=SHYNESS+BOY") && seen[1].contains("durationMs=196666") && seen[1].contains("isrc=JPFL08310300"), "{}", seen[1]);
    }

    /// `cargo test live_qobuzify -- --ignored --nocapture` (needs the network).
    #[tokio::test]
    #[ignore]
    async fn live_qobuzify() {
        let h = Http::new("api.qobuzify.app", "", "", None);
        let t = TrackRef {
            key: "spotify:track:0VjIjW4GlUZAMYd2vXMi3b".into(),
            title: "Blinding Lights".into(),
            artists: vec!["The Weeknd".into()],
            album: "After Hours".into(),
            duration_ms: 200_040,
            ..Default::default()
        };
        match fetch(&h, BASE, &t).await {
            Outcome::Found { lyrics, .. } => {
                println!("qobuzify: {} lines, {}, e.g. {:?}", lyrics.lines.len(), lyrics.timing.label(), lyrics.lines.get(3).map(|l| &l.text));
                assert!(lyrics.timing >= Timing::Word);
            }
            o => panic!("{o:?}"),
        }
    }
}
