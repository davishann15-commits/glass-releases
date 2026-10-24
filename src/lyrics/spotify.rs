//! Spotify's lyrics, as a fallback for songs LRCLIB doesn't have: the lyrics
//! service Spotify's own apps use, read through Glass's Spotify session with
//! librespot's `SpClient::get_lyrics` (Spotify songs only). Line-synced or
//! plain, and sometimes with translations, which are kept for display.

use super::{Line, Lyrics, Match, Outcome, ProviderId, Source, Timing, TrackRef};
use crate::ai::spotify::Source as SpotifySource;
use serde_json::Value;
use std::sync::Arc;

/// Why Spotify can't be asked for this song.
pub const NOT_CONNECTED: &str = "Glass isn't connected to Spotify right now, so Spotify's lyrics can't be fetched.";
pub const NOT_SPOTIFY: &str = "Spotify has lyrics only for songs played from Spotify, not local files.";

fn ms(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}

/// Spotify's answer, as Glass's lyrics.
pub fn parse(v: &Value, t: &TrackRef) -> Option<Lyrics> {
    let l = &v["lyrics"];
    let raw = l["lines"].as_array()?;
    let lines: Vec<Line> = raw
        .iter()
        .map(|x| {
            let text = x["words"].as_str().unwrap_or("").trim();
            Line { start_ms: ms(&x["startTimeMs"]), end_ms: Some(ms(&x["endTimeMs"])).filter(|e| *e > 0), text: if text == "♪" { String::new() } else { text.to_string() }, tokens: Vec::new() }
        })
        .collect();
    if lines.iter().all(|l| l.text.is_empty()) {
        return None;
    }
    let timing = match l["syncType"].as_str() {
        Some("LINE_SYNCED" | "SYLLABLE_SYNCED") => Timing::Line,
        _ => Timing::Plain,
    };
    // Translations, when Spotify has them: one entry per line, by language.
    let translations = l["alternatives"]
        .as_array()
        .map(|alts| {
            alts.iter()
                .filter_map(|a| {
                    let lang = a["language"].as_str()?.to_string();
                    let tl: Vec<String> = a["lines"].as_array()?.iter().map(|x| x.as_str().or_else(|| x["words"].as_str()).unwrap_or("").trim().to_string()).collect();
                    (tl.len() == lines.len()).then_some((lang, tl))
                })
                .collect()
        })
        .unwrap_or_default();
    let id = l["providerLyricsId"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| t.key.rsplit(':').next().unwrap_or("").to_string());
    let source = Source { provider: ProviderId::Spotify, id, title: t.title.clone(), artist: t.artist().to_string(), album: t.album.clone(), duration_ms: t.duration_ms };
    Some(Lyrics { source, language: l["language"].as_str().filter(|s| !s.is_empty()).map(str::to_string), timing, lines, translations })
}

/// Spotify's lyrics for `t`, as an outcome.
pub async fn fetch(src: Option<Arc<dyn SpotifySource>>, t: &TrackRef) -> Outcome {
    let p = ProviderId::Spotify;
    let Some(id) = t.key.strip_prefix("spotify:track:") else { return Outcome::Unavailable { provider: p, reason: NOT_SPOTIFY.into() } };
    let Some(src) = src else { return Outcome::Unavailable { provider: p, reason: NOT_CONNECTED.into() } };
    match src.get(format!("/glass/lyrics/{id}"), None).await {
        Ok(v) => match parse(&v, t) {
            Some(l) => Outcome::Found { lyrics: Arc::new(l), how: Match::Exact },
            None => Outcome::NotFound { provider: p },
        },
        Err(e) if e.contains("404") => Outcome::NotFound { provider: p },
        Err(e) => Outcome::Failed { provider: p, error: e },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn track() -> TrackRef {
        TrackRef { key: "spotify:track:abc".into(), title: "Mayonaka no Door".into(), artists: vec!["Miki Matsubara".into()], album: "Pocket Park".into(), duration_ms: 297_000 }
    }

    #[test]
    fn spotify_lyrics_and_their_translations() {
        let v = json!({ "lyrics": {
            "syncType": "LINE_SYNCED", "language": "ja", "providerLyricsId": "123",
            "lines": [{ "startTimeMs": "1000", "words": "真夜中のドアを叩き", "endTimeMs": "0" }, { "startTimeMs": "5000", "words": "♪", "endTimeMs": "0" }],
            "alternatives": [{ "language": "en", "lines": ["Knocking on the midnight door", ""] }, { "language": "es", "lines": ["only one"] }]
        }});
        let l = parse(&v, &track()).unwrap();
        assert_eq!((l.timing, l.lines[0].start_ms, l.lines[1].text.as_str()), (Timing::Line, 1000, ""));
        assert_eq!(l.language.as_deref(), Some("ja"));
        assert_eq!(l.translations, vec![("en".to_string(), vec!["Knocking on the midnight door".to_string(), String::new()])], "a translation that doesn't line up is dropped");
        assert_eq!(l.source.provider, ProviderId::Spotify);
        assert!(parse(&json!({ "lyrics": { "lines": [] } }), &track()).is_none());
    }

    #[tokio::test]
    async fn only_spotify_songs_and_only_when_connected() {
        let mut local = track();
        local.key = "local:/m/a.mp3".into();
        assert!(matches!(fetch(None, &local).await, Outcome::Unavailable { .. }));
        assert!(matches!(fetch(None, &track()).await, Outcome::Unavailable { reason, .. } if reason == NOT_CONNECTED));
    }
}
