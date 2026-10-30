//! Spicy Lyrics' developer API (<https://developers.spicylyrics.org>), with
//! a key the user creates there: one request by Spotify track ID answers with
//! the best sync it has — a community one made by hand, else Apple Music's or
//! Spotify's. Syllable syncs carry duets (`OppositeAligned`) and backing
//! vocals. Times are in seconds.
//!
//! Its terms ask that the provider always be shown, and that a community
//! sync's maker and uploader be credited and linked; `credits` does both.
//! Without a key the source is off; a refused key says why.

use super::http::{Error, Http};
use super::{Backing, Credit, Line, Lyrics, Match, Outcome, ProviderId, Source, Timing, Token, TrackRef, Voice, parse};
use serde_json::Value;
use std::sync::Arc;

pub const BASE: &str = "https://api.spicylyrics.org";
const P: ProviderId = ProviderId::Spicy;
pub const NO_KEY: &str = "Add your Spicy Lyrics key in Settings to use it.";
pub const NOT_SPOTIFY: &str = "Spicy Lyrics looks songs up by their Spotify ID, so it only has lyrics for songs played from Spotify.";

fn ms(v: &Value) -> Option<u64> {
    v.as_f64().filter(|s| s.is_finite() && *s >= 0.0).map(|s| (s * 1000.0).round() as u64)
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// A Lead or Background's syllables as pieces: a syllable that isn't part of
/// a word ends with a space.
fn syllables(part: &Value) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    for s in part["Syllables"].as_array().into_iter().flatten() {
        let t = text(&s["Text"]).trim();
        let Some(start) = ms(&s["StartTime"]) else { continue };
        if t.is_empty() {
            continue;
        }
        let joined = s["IsPartOfWord"].as_bool().unwrap_or(false);
        out.push(Token { text: if joined { t.to_string() } else { format!("{t} ") }, start_ms: start, end_ms: ms(&s["EndTime"]).filter(|&e| e >= start) });
    }
    if let Some(l) = out.last_mut() {
        l.text = l.text.trim_end().to_string();
    }
    out
}

fn voice(item: &Value) -> Voice {
    if item["OppositeAligned"].as_bool().unwrap_or(false) { Voice::Other } else { Voice::Lead }
}

/// The `Body` of an answer as lines and how they're timed.
pub fn lines(body: &Value) -> Option<(Vec<Line>, Timing)> {
    let mut lines: Vec<Line> = match body["Type"].as_str()? {
        "Syllable" => body["Content"]
            .as_array()?
            .iter()
            .filter_map(|item| {
                let lead = &item["Lead"];
                let tokens = syllables(lead);
                let first = tokens.first()?.start_ms;
                let start = ms(&lead["StartTime"]).unwrap_or(first).min(first);
                let mut backing: Vec<Token> = Vec::new();
                for group in item["Background"].as_array().into_iter().flatten() {
                    let mut t = syllables(group);
                    if let Some(l) = backing.last_mut()
                        && !t.is_empty()
                    {
                        l.text.push(' ');
                    }
                    backing.append(&mut t);
                }
                let backing = (!backing.is_empty()).then(|| Box::new(Backing { text: backing.iter().map(|t| t.text.as_str()).collect(), tokens: backing }));
                Some(Line { start_ms: start, end_ms: ms(&lead["EndTime"]).filter(|&e| e > start), text: tokens.iter().map(|t| t.text.as_str()).collect(), tokens, voice: voice(item), backing })
            })
            .collect(),
        "Line" => body["Content"]
            .as_array()?
            .iter()
            .filter_map(|item| {
                let start = ms(&item["StartTime"])?;
                let t = text(&item["Text"]).trim();
                Some(Line {
                    start_ms: start,
                    end_ms: ms(&item["EndTime"]).filter(|&e| e > start),
                    text: if t == "♪" { String::new() } else { t.to_string() },
                    voice: voice(item),
                    ..Default::default()
                })
            })
            .collect(),
        "Static" => {
            let text: Vec<&str> = body["Lines"].as_array()?.iter().map(|l| text(&l["Text"])).collect();
            return Some((parse::plain(&text.join("\n")), Timing::Plain));
        }
        _ => return None,
    };
    if lines.iter().all(|l| l.text.is_empty()) {
        return None;
    }
    lines.sort_by_key(|l| l.start_ms);
    let timing = parse::settle_tokens(&mut lines);
    Some((lines, timing))
}

/// What to show under the lyrics, as the API's terms ask.
pub fn credits(body: &Value) -> Vec<Credit> {
    let via = match body["source"].as_str() {
        Some("apple_music") => " · Apple Music",
        Some("spotify") => " · Spotify",
        _ => "",
    };
    let mut out = vec![Credit { text: format!("Spicy Lyrics{via}"), url: Some("https://spicylyrics.org".into()) }];
    if body["source"].as_str() == Some("spicy_lyrics") {
        let people = &body["UploadAttribution"];
        let maker = &people["Maker"];
        let uploader = &people["Uploader"];
        let link = |p: &Value| p["url"].as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://")).map(str::to_string);
        if let Some(name) = maker["username"].as_str().filter(|s| !s.is_empty()) {
            out.push(Credit { text: format!("synced by {name}"), url: link(maker) });
        }
        if let Some(name) = uploader["username"].as_str().filter(|s| !s.is_empty())
            && Some(name) != maker["username"].as_str()
        {
            out.push(Credit { text: format!("uploaded by {name}"), url: link(uploader) });
        }
    }
    out
}

/// What a refusal's error code means, in a few words.
fn refused(v: &Value) -> String {
    let code = v["Body"]["error"].as_str().or(v["error"].as_str()).unwrap_or("");
    match code {
        "missing_authorization" | "key_not_found" | "key_malformed" | "malformed_authorization" => "Spicy Lyrics didn't recognize the key.".into(),
        "origin_not_allowed" | "origins_not_configured" => "Spicy Lyrics refused the key: allow \"No origin header\" for it in the Spicy Lyrics dashboard.".into(),
        "key_revoked" => "That Spicy Lyrics key was revoked.".into(),
        "application_paused" => "That Spicy Lyrics application is paused.".into(),
        _ => "Spicy Lyrics refused the key.".into(),
    }
}

pub async fn fetch(http: &Http, base: &str, key: Option<&str>, t: &TrackRef) -> Outcome {
    let Some(id) = t.spotify_id() else { return Outcome::Unavailable { provider: P, reason: NOT_SPOTIFY.into() } };
    let Some(key) = key.filter(|k| !k.trim().is_empty()) else { return Outcome::Unavailable { provider: P, reason: NO_KEY.into() } };
    let r = match http.get(&format!("{base}/v1/lyrics/{id}"), &[], &[("Authorization", format!("Bearer {}", key.trim())), ("Accept", "application/json".into())]).await {
        Ok(r) => r,
        Err(e) => return Outcome::Failed { provider: P, error: format!("Spicy Lyrics {e}") },
    };
    match r.status {
        200 => {}
        400 | 404 => return Outcome::NotFound { provider: P },
        401 | 403 => return Outcome::Unavailable { provider: P, reason: r.json().map(|v| refused(&v)).unwrap_or_else(|_| "Spicy Lyrics refused the key.".into()) },
        code => return Outcome::Failed { provider: P, error: format!("Spicy Lyrics {}", Error::Http(code)) },
    }
    let v = match r.json() {
        Ok(v) => v,
        Err(e) => return Outcome::Failed { provider: P, error: format!("Spicy Lyrics {e}") },
    };
    let body = &v["Body"];
    let Some((lines, timing)) = lines(body) else { return Outcome::NotFound { provider: P } };
    let source = Source { provider: P, id: body["id"].as_str().unwrap_or(id).to_string(), title: t.title.clone(), artist: t.artist().to_string(), album: t.album.clone(), duration_ms: t.duration_ms };
    let mut l = Lyrics::new(source, timing, lines);
    l.credits = credits(body);
    Outcome::Found { lyrics: Arc::new(l), how: Match::Exact }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::http::tests::{reply, serve};
    use serde_json::json;

    /// The documented shape (developers.spicylyrics.org); placeholder words.
    fn syllable_body() -> Value {
        json!({
            "Type": "Syllable", "id": "1QV6tiMFM6fSOKOGLMHYYg", "source": "spicy_lyrics", "StartTime": 1.5, "EndTime": 11.0,
            "UploadAttribution": { "Uploader": { "username": "up", "url": "https://spicylyrics.org/uid/1" }, "Maker": { "username": "mk", "url": "https://spicylyrics.org/uid/2" } },
            "Content": [
                { "Type": "Vocal", "Lead": { "StartTime": 1.5, "EndTime": 4.25, "Syllables": [
                    { "Text": "al", "StartTime": 1.5, "EndTime": 1.9, "IsPartOfWord": true },
                    { "Text": "pha", "StartTime": 1.9, "EndTime": 2.4 },
                    { "Text": "bravo", "StartTime": 2.6, "EndTime": 4.25 } ] },
                  "Background": [ { "StartTime": 3.0, "EndTime": 4.0, "Syllables": [ { "Text": "echo", "StartTime": 3.0, "EndTime": 4.0 } ] } ] },
                { "Type": "Vocal", "OppositeAligned": true, "Lead": { "StartTime": 9.0, "EndTime": 11.0, "Syllables": [ { "Text": "charlie", "StartTime": 9.0, "EndTime": 11.0 } ] } }
            ]
        })
    }

    #[test]
    fn syllables_duets_and_backing_vocals() {
        let (l, timing) = lines(&syllable_body()).unwrap();
        assert_eq!(timing, Timing::Syllable);
        assert_eq!(l[0].text, "alpha bravo");
        assert_eq!(l[0].tokens.iter().map(|t| (t.text.as_str(), t.start_ms, t.end_ms)).collect::<Vec<_>>(), vec![("al", 1500, Some(1900)), ("pha ", 1900, Some(2400)), ("bravo", 2600, Some(4250))]);
        assert_eq!(l[0].backing.as_ref().unwrap().text, "echo");
        assert_eq!((l[0].voice, l[1].voice), (Voice::Lead, Voice::Other));
    }

    #[test]
    fn line_and_static_bodies() {
        let (l, t) = lines(&json!({ "Type": "Line", "Content": [ { "Type": "Vocal", "Text": "delta foxtrot", "StartTime": 2.0, "EndTime": 4.0 }, { "Type": "Vocal", "Text": "golf hotel", "StartTime": 4.5, "EndTime": 6.0, "OppositeAligned": true } ] })).unwrap();
        assert_eq!((t, l[1].start_ms, l[1].voice), (Timing::Line, 4500, Voice::Other));
        let (l, t) = lines(&json!({ "Type": "Static", "Lines": [{ "Text": "india" }, { "Text": "" }, { "Text": "juliett" }] })).unwrap();
        assert_eq!((t, l.len()), (Timing::Plain, 3));
        assert!(lines(&json!({ "Type": "Mystery" })).is_none());
    }

    #[test]
    fn attribution_as_the_terms_ask() {
        let c = credits(&syllable_body());
        assert_eq!(c.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["Spicy Lyrics", "synced by mk", "uploaded by up"]);
        assert_eq!(c[1].url.as_deref(), Some("https://spicylyrics.org/uid/2"));
        // A catalog's sync names the catalog, and no empty "uploaded by".
        let c = credits(&json!({ "source": "apple_music" }));
        assert_eq!(c.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["Spicy Lyrics · Apple Music"]);
    }

    #[tokio::test]
    async fn keys_spotify_ids_and_refusals() {
        let h = Http::new("spicy", "", "", None);
        let local = TrackRef { key: "local:/m/a.mp3".into(), ..Default::default() };
        assert!(matches!(fetch(&h, "http://127.0.0.1:9", Some("k"), &local).await, Outcome::Unavailable { reason, .. } if reason == NOT_SPOTIFY));
        let song = TrackRef { key: "spotify:track:0VjIjW4GlUZAMYd2vXMi3b".into(), title: "t".into(), artists: vec!["a".into()], ..Default::default() };
        assert!(matches!(fetch(&h, "http://127.0.0.1:9", None, &song).await, Outcome::Unavailable { reason, .. } if reason == NO_KEY), "no key: nothing is sent");
        let refusal = r#"{"Body":{"error":"origin_not_allowed","message":"…"},"Status":403,"Type":"object"}"#;
        let (base, server) =
            serve(vec![reply("403 Forbidden", "application/json", "", refusal), reply("200 OK", "application/json", "", &json!({ "Body": syllable_body(), "Status": 200 }).to_string())]).await;
        assert!(matches!(fetch(&h, &base, Some("sl_pk_x"), &song).await, Outcome::Unavailable { reason, .. } if reason.contains("No origin header")));
        let Outcome::Found { lyrics, how } = fetch(&h, &base, Some("sl_pk_x"), &song).await else { panic!() };
        assert_eq!((how, lyrics.timing), (Match::Exact, Timing::Syllable));
        let seen = server.await.unwrap();
        assert!(seen[0].starts_with("GET /v1/lyrics/0VjIjW4GlUZAMYd2vXMi3b ") && seen[0].contains("Bearer sl_pk_x"));
    }
}
