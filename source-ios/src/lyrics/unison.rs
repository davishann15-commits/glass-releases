//! Unison (<https://unison.boidu.dev>), Better Lyrics' community catalog:
//! hand-made syncs, mostly TTML timed word by word, looked up by song and
//! artist. Free and keyless; reads are rate limited per IP (120 a minute).
//!
//! Its syncs are made against music videos, which can run longer than the
//! album recording, so a sync is only taken when its length matches the song.

use super::http::{Error, Http};
use super::matching::{self, Info};
use super::{Credit, Lyrics, Match, Outcome, ProviderId, Source, TrackRef, parse, ttml};
use serde_json::Value;
use std::sync::Arc;

pub const BASE: &str = "https://unison.boidu.dev";
const P: ProviderId = ProviderId::Unison;

pub async fn fetch(http: &Http, base: &str, t: &TrackRef) -> Outcome {
    if t.title.trim().is_empty() || t.artist().trim().is_empty() || t.duration_ms == 0 {
        return Outcome::NotFound { provider: P };
    }
    let mut q = vec![("song", matching::plain_title(&t.title)), ("artist", t.artist().to_string()), ("duration", (t.duration_ms / 1000).to_string())];
    if !t.album.is_empty() {
        q.push(("album", t.album.clone()));
    }
    let r = match http.get(&format!("{base}/lyrics"), &q, &[]).await {
        Ok(r) => r,
        Err(e) => return Outcome::Failed { provider: P, error: format!("Unison {e}") },
    };
    match r.status {
        200 => {}
        404 => return Outcome::NotFound { provider: P },
        code => return Outcome::Failed { provider: P, error: format!("Unison {}", Error::Http(code)) },
    }
    match r.json() {
        Ok(v) => answer(&v, t).unwrap_or(Outcome::NotFound { provider: P }),
        Err(e) => Outcome::Failed { provider: P, error: format!("Unison {e}") },
    }
}

/// `{ success, data: { id, song, artist, album, duration, format, syncType, lyrics, … } }`.
pub fn answer(v: &Value, t: &TrackRef) -> Option<Outcome> {
    if v["success"] != Value::Bool(true) || v["data"]["hidden"] == Value::Bool(true) {
        return None;
    }
    let d = &v["data"];
    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
    let body = d["lyrics"].as_str()?;
    let (lines, timing, doc_ms, language) = match d["format"].as_str()? {
        "ttml" => {
            let doc = ttml::parse(body).map_err(|e| log::debug!("unison {}: {e}", s("id"))).ok()?;
            (doc.lines, doc.timing, doc.duration_ms, doc.language)
        }
        "lrc" => {
            let (lines, timing) = parse::lrc(body);
            (lines, timing, None, None)
        }
        // Plain text carries nothing LRCLIB doesn't.
        _ => return None,
    };
    if lines.is_empty() {
        return None;
    }
    // The length it was synced to: the answer's own, else the TTML's.
    let ms = d["duration"].as_f64().filter(|x| x.is_finite() && *x > 0.0).map(|x| (x * 1000.0).round() as u64).or(doc_ms).unwrap_or(0);
    let (title, artist, album) = (s("song"), s("artist"), s("album"));
    matching::strict(t, &Info { title: &title, artist: &artist, album: &album, duration_ms: ms, ..Default::default() })?;
    let id = d["id"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| s("id"));
    let source = Source { provider: P, id, title, artist, album, duration_ms: ms };
    let mut l = Lyrics::new(source, timing, lines);
    l.language = d["language"].as_str().or(language.as_deref()).map(|s| s.to_lowercase()).filter(|s| (2..=3).contains(&s.len()));
    l.credits = vec![Credit { text: "Unison (Better Lyrics)".into(), url: Some(BASE.to_string()) }];
    Some(Outcome::Found { lyrics: Arc::new(l), how: Match::Search })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Timing;
    use serde_json::json;

    fn track(secs: u64) -> TrackRef {
        TrackRef { key: "spotify:track:x".into(), title: "The Search".into(), artists: vec!["NF".into()], album: "The Search".into(), duration_ms: secs * 1000, ..Default::default() }
    }

    /// Shaped like a real answer (fields as Unison sends them; the lyrics are placeholders).
    fn answer_json(dur: &str) -> Value {
        json!({ "success": true, "data": {
            "id": 10, "videoId": "Zh9p-6JfrSo", "song": "The Search", "artist": "NF", "album": "The Search",
            "lyrics": format!(r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" itunes:timing="Word"><head><metadata><ttm:agent type="person" xml:id="v1"/></metadata></head><body dur="{dur}"><div begin="00:37.097" end="04:08.033"><p begin="00:37.097" end="00:40.503" ttm:agent="v1" itunes:key="L1"><span begin="00:37.097" end="00:37.325">alpha</span> <span begin="00:37.325" end="00:37.539">bravo</span></p></div></body></tt>"#),
            "format": "ttml", "language": "en", "syncType": "richsync", "score": 17, "voteCount": 21, "confidence": "medium", "hidden": false
        }})
    }

    #[test]
    fn a_sync_for_this_recording() {
        let Some(Outcome::Found { lyrics, .. }) = answer(&answer_json("04:08.033"), &track(248)) else { panic!() };
        assert_eq!((lyrics.timing, lyrics.lines[0].text.as_str(), lyrics.source.id.as_str()), (Timing::Word, "alpha bravo", "10"));
        assert_eq!(lyrics.language.as_deref(), Some("en"));
    }

    #[test]
    fn a_sync_made_against_a_longer_video_is_refused() {
        assert!(answer(&answer_json("04:30.000"), &track(248)).is_none());
        assert!(answer(&json!({ "success": false, "error": "Lyrics not found", "code": "NOT_FOUND" }), &track(248)).is_none());
        let mut plain = answer_json("04:08.033");
        plain["data"]["format"] = json!("plain");
        assert!(answer(&plain, &track(248)).is_none());
    }

    /// `cargo test live_unison -- --ignored --nocapture` (needs the network).
    #[tokio::test]
    #[ignore]
    async fn live_unison() {
        let h = Http::new("unison", "", "", None);
        match fetch(&h, BASE, &track(248)).await {
            Outcome::Found { lyrics, .. } => println!("unison: {} lines, {}", lyrics.lines.len(), lyrics.timing.label()),
            o => panic!("{o:?}"),
        }
        let missing = TrackRef { title: "Glass Test Track That Does Not Exist".into(), ..track(123) };
        assert_eq!(fetch(&h, BASE, &missing).await, Outcome::NotFound { provider: P });
    }
}
