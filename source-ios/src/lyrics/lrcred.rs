//! lrc.red (<https://lrc.red/api>): a free lyrics catalog filed by ISRC that
//! serves TTML timed word by word or syllable by syllable, with the singers
//! of a duet and the backing vocals. Its terms ask two things, both done here:
//! a User-Agent naming the app and how to reach whoever runs it, and credit
//! with a link back to lrc.red wherever its lyrics show.
//!
//! A song is looked up by its ISRC (the exact recording) when Glass knows it,
//! else by a search on title and artist whose hits must match the recording
//! exactly, length included.

use super::http::{Error, Http};
use super::matching::{self, Info};
use super::{Credit, Lyrics, Match, Outcome, ProviderId, Source, TrackRef, clean_isrc, ttml};
use serde_json::Value;
use std::sync::Arc;

pub const BASE: &str = "https://lrc.red";
const P: ProviderId = ProviderId::LrcRed;

/// The lyrics filed under `isrc`: `Ok(None)` when there are none.
async fn ttml_for(http: &Http, base: &str, isrc: &str) -> Result<Option<ttml::Doc>, Error> {
    let r = http.get(&format!("{base}/s/{isrc}.ttml"), &[], &[]).await?;
    match r.status {
        200 => ttml::parse(&r.text()?).map(Some).map_err(Error::Malformed),
        404 | 410 => Ok(None),
        code => Err(Error::Http(code)),
    }
}

/// A search hit: (isrc, title, artist, album, length in ms).
pub fn hits(v: &Value) -> Vec<(String, String, String, String, u64)> {
    v["hits"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|h| {
                    let isrc = clean_isrc(h["isrc"].as_str()?)?;
                    let secs = h["duration"].as_f64().filter(|d| d.is_finite() && *d > 0.0).unwrap_or(0.0);
                    let s = |k: &str| h[k].as_str().unwrap_or("").to_string();
                    Some((isrc, s("title"), s("artist"), s("album"), (secs * 1000.0).round() as u64))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The document as Glass's lyrics, if it's for a recording of this length.
fn lyrics(doc: ttml::Doc, t: &TrackRef, isrc: &str, title: &str, artist: &str, album: &str, how: Match) -> Option<Outcome> {
    // The TTML's own length is the recording it was timed to.
    if let Some(d) = doc.duration_ms
        && t.duration_ms > 0
        && (d as f32 - t.duration_ms as f32).abs() / 1000.0 > matching::SAME_RECORDING_S
    {
        log::debug!("lrc.red {isrc}: timed to {} ms, the song is {} ms", d, t.duration_ms);
        return None;
    }
    let source = Source { provider: P, id: isrc.to_string(), title: title.to_string(), artist: artist.to_string(), album: album.to_string(), duration_ms: doc.duration_ms.unwrap_or(t.duration_ms) };
    let mut l = Lyrics::new(source, doc.timing, doc.lines);
    l.language = doc.language.map(|s| s.split('-').next().unwrap_or("").to_string()).filter(|s| (2..=3).contains(&s.len()));
    l.translations = doc.translations;
    l.credits = vec![Credit { text: "lrc.red".into(), url: Some(format!("{BASE}/s/{isrc}")) }];
    Some(Outcome::Found { lyrics: Arc::new(l), how })
}

pub async fn fetch(http: &Http, base: &str, t: &TrackRef) -> Outcome {
    match lookup(http, base, t).await {
        Ok(o) => o,
        Err(e) => Outcome::Failed { provider: P, error: format!("lrc.red {e}") },
    }
}

async fn lookup(http: &Http, base: &str, t: &TrackRef) -> Result<Outcome, Error> {
    // The exact recording, by ISRC.
    if let Some(isrc) = clean_isrc(&t.isrc)
        && let Some(doc) = ttml_for(http, base, &isrc).await?
        && let Some(o) = lyrics(doc, t, &isrc, &t.title, t.artist(), &t.album, Match::Exact)
    {
        return Ok(o);
    }
    if t.title.trim().is_empty() || t.artist().trim().is_empty() {
        return Ok(Outcome::NotFound { provider: P });
    }
    // Otherwise a search, matched strictly: word timing is only right for the very recording.
    let q = format!("{} {}", matching::search_title(&t.title), t.artist());
    let r = http.get(&format!("{base}/search.json"), &[("q", q)], &[]).await?;
    if r.status != 200 {
        return Ok(Outcome::NotFound { provider: P });
    }
    let found = hits(&r.json()?);
    let Some(i) = matching::best_strict(t, found.iter().map(|(_, title, artist, album, ms)| Info { title, artist, album, duration_ms: *ms, ..Default::default() })) else {
        return Ok(Outcome::NotFound { provider: P });
    };
    let (isrc, title, artist, album, _) = &found[i];
    if clean_isrc(&t.isrc).as_deref() == Some(isrc.as_str()) {
        // Already asked for it by ISRC.
        return Ok(Outcome::NotFound { provider: P });
    }
    Ok(match ttml_for(http, base, isrc).await? {
        Some(doc) => lyrics(doc, t, isrc, title, artist, album, Match::Search).unwrap_or(Outcome::NotFound { provider: P }),
        None => Outcome::NotFound { provider: P },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Timing;
    use crate::lyrics::http::tests::{reply, serve};

    fn track(isrc: &str) -> TrackRef {
        TrackRef { key: "spotify:track:x".into(), title: "Song Title".into(), artists: vec!["Some Artist".into()], album: "Album".into(), duration_ms: 200_000, isrc: isrc.into() }
    }

    const DOC: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en"><body dur="3:20.046"><div><p begin="1.0" end="2.0"><span begin="1.0" end="1.5">one</span> <span begin="1.5" end="2.0">two</span></p></div></body></tt>"#;

    #[tokio::test]
    async fn by_isrc_first() {
        let (base, server) = serve(vec![reply("200 OK", "application/ttml+xml", "", DOC)]).await;
        let h = Http::new("lrc.red", "", "", None);
        let Outcome::Found { lyrics, how } = fetch(&h, &base, &track("USUG11904206")).await else { panic!() };
        assert_eq!((how, lyrics.timing, lyrics.source.id.as_str()), (Match::Exact, Timing::Word, "USUG11904206"));
        assert_eq!(lyrics.credits[0].url.as_deref(), Some("https://lrc.red/s/USUG11904206"), "lrc.red is credited with a link");
        assert!(server.await.unwrap()[0].starts_with("GET /s/USUG11904206.ttml "));
    }

    #[tokio::test]
    async fn search_only_takes_the_very_recording() {
        let search = r#"{"hits":[{"isrc":"USAAA0000001","title":"Song Title (Remix)","artist":"Some Artist","duration":200.0},{"isrc":"USAAA0000002","title":"Song Title","artist":"Some Artist","duration":230.0},{"isrc":"USAAA0000003","title":"Song Title","artist":"Some Artist, Guest","album":"Album","duration":200.4}],"next":null}"#;
        let (base, server) = serve(vec![reply("404 Not Found", "text/plain", "", ""), reply("200 OK", "application/json", "", search), reply("200 OK", "application/ttml+xml", "", DOC)]).await;
        let h = Http::new("lrc.red", "", "", None);
        let Outcome::Found { lyrics, how } = fetch(&h, &base, &track("USZZZ9999999")).await else { panic!() };
        assert_eq!((how, lyrics.source.id.as_str()), (Match::Search, "USAAA0000003"), "not the remix, not the 30 s longer cut");
        let seen = server.await.unwrap();
        assert!(seen[1].starts_with("GET /search.json?q=song+title+Some+Artist ") || seen[1].contains("q=song%20title%20Some%20Artist"), "{}", seen[1]);
    }

    #[tokio::test]
    async fn a_document_timed_to_another_length_is_refused() {
        let other = DOC.replace("3:20.046", "4:01.000");
        let (base, _s) = serve(vec![reply("200 OK", "application/ttml+xml", "", &other), reply("200 OK", "application/json", "", r#"{"hits":[]}"#)]).await;
        let h = Http::new("lrc.red", "", "", None);
        assert_eq!(fetch(&h, &base, &track("USUG11904206")).await, Outcome::NotFound { provider: P });
    }

    /// Real lookups on lrc.red (opt-in, needs the network):
    /// `cargo test live_lrcred -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_lrcred() {
        let h = Http::new("lrc.red", "", "", None);
        let t = |title: &str, artist: &str, album: &str, ms: u64, isrc: &str| TrackRef {
            key: format!("live:{title}"),
            title: title.into(),
            artists: vec![artist.into()],
            album: album.into(),
            duration_ms: ms,
            isrc: isrc.into(),
        };
        for (name, track) in [
            ("by ISRC", t("Blinding Lights", "The Weeknd", "After Hours", 200_040, "USUG11904206")),
            ("duet, by search", t("Shallow", "Lady Gaga", "A Star Is Born Soundtrack", 215_733, "")),
            ("backing vocals", t("Espresso", "Sabrina Carpenter", "Short n' Sweet", 175_459, "")),
        ] {
            match fetch(&h, BASE, &track).await {
                Outcome::Found { lyrics, how } => {
                    let backing = lyrics.lines.iter().filter(|l| l.backing.is_some()).count();
                    let other = lyrics.lines.iter().filter(|l| l.voice == crate::lyrics::Voice::Other).count();
                    println!("{name}: {} via {how:?}, {} lines ({backing} with backing vocals, {other} by a second voice), isrc {}", lyrics.timing.label(), lyrics.lines.len(), lyrics.source.id);
                    assert!(lyrics.timing >= Timing::Word, "{name}");
                }
                o => panic!("{name}: {o:?}"),
            }
        }
    }
}
