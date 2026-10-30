//! What the user does with the assistant's recommendations (`tools::Picks`),
//! shared by both apps: hear a song's 30-second preview, and add songs to a
//! playlist of theirs without adding what it already has.

use super::spotify::{Catalog, Source, playlist_songs};
use reqwest::Method;
use std::time::{Duration, Instant};

/// A preview clip is about 350 KB (30 s of 96 kbps MP3).
const MAX_CLIP: usize = 2 << 20;

/// A song's preview playing (or loading) from a recommendation card.
pub struct Preview {
    pub uri: String,
    pub generation: u64,
    /// `None` while the clip loads.
    pub voice: Option<crate::dj::Voice>,
    pub started: Option<Instant>,
    /// Glass paused what was playing for the preview; it plays again after.
    pub resume: bool,
}

impl Preview {
    /// Seconds heard so far, for the card's countdown.
    pub fn elapsed(&self) -> Duration {
        self.started.map(|s| s.elapsed()).unwrap_or_default()
    }
    pub fn done(&self) -> bool {
        self.voice.as_ref().is_some_and(|v| v.done())
    }
}

/// The preview clip of `uri`: where the assistant's answers said it is, else
/// from the song's details. `Ok(None)`: Spotify has no preview for it.
pub async fn clip(src: &dyn Source, http: &reqwest::Client, catalog: &Catalog, uri: &str) -> Result<Option<Vec<u8>>, String> {
    let url = match catalog.get(uri).and_then(|i| i.preview) {
        Some(u) => Some(u),
        None => {
            let id = crate::model::id_of(uri);
            let t = src.get(format!("/tracks/{id}"), None).await.map_err(|e| format!("couldn't look the song up ({e})"))?;
            t["preview_url"].as_str().filter(|u| u.starts_with("https://")).map(str::to_string)
        }
    };
    let Some(url) = url else { return Ok(None) };
    let r = http.get(&url).timeout(Duration::from_secs(15)).send().await.map_err(|e| format!("couldn't load the preview ({e})"))?;
    if !r.status().is_success() {
        return Err(format!("couldn't load the preview ({})", r.status()));
    }
    let bytes = r.bytes().await.map_err(|e| format!("couldn't load the preview ({e})"))?;
    if bytes.len() > MAX_CLIP {
        return Err("the preview was too big".into());
    }
    Ok(Some(bytes.to_vec()))
}

/// What adding came to.
#[derive(Debug, PartialEq)]
pub struct Added {
    /// Added now.
    pub uris: Vec<String>,
    /// Already in the playlist (this recording or another release of the song).
    pub already: Vec<String>,
}

/// Adds songs ((uri, title, artists)) to playlist `pid` (a bare ID), leaving
/// out those it already has.
pub async fn add(src: &dyn Source, pid: &str, songs: &[(String, String, String)]) -> Result<Added, String> {
    let has = playlist_songs(src, pid).await.map_err(|e| format!("Couldn't read the playlist: {e}"))?;
    let (mut uris, mut already) = (Vec::new(), Vec::new());
    for (uri, title, artists) in songs {
        if has.has(uri, title, artists) || uris.contains(uri) {
            already.push(uri.clone());
        } else {
            uris.push(uri.clone());
        }
    }
    for chunk in uris.chunks(100) {
        let b = serde_json::json!({ "uris": chunk });
        let mut r = src.send(Method::POST, format!("/playlists/{pid}/tracks"), Some(b.clone())).await;
        if r.as_ref().is_err_and(|e| e.starts_with("404")) {
            r = src.send(Method::POST, format!("/playlists/{pid}/items"), Some(b)).await;
        }
        r?;
    }
    Ok(Added { uris, already })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::spotify::tests::{Fake, fake, t};
    use serde_json::json;

    fn song(id: &str, title: &str) -> (String, String, String) {
        (format!("spotify:track:{id}"), title.to_string(), "Anri".to_string())
    }

    #[tokio::test]
    async fn adding_leaves_out_what_the_playlist_has() {
        let mut f: Fake = fake();
        f.data.insert(
            "/playlists/mine/tracks".into(),
            json!({ "items": [{ "track": t("cat", "Cat's Eye", "Anri", "Timely!!", 230) }, { "track": t("windy2018", "Windy Summer (2018 Remaster)", "Anri", "Timely!!", 270) }] }),
        );
        let r = add(&f, "mine", &[song("cat", "Cat's Eye"), song("windy", "Windy Summer"), song("shyness", "Shyness Boy")]).await.unwrap();
        assert_eq!(r.uris, vec!["spotify:track:shyness"], "the same recording, and the same song on another release, are already there");
        assert_eq!(r.already.len(), 2);
        let sent = f.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].0.as_str(), sent[0].1.as_str()), ("POST", "/playlists/mine/tracks"));
        assert_eq!(sent[0].2.as_ref().unwrap()["uris"], json!(["spotify:track:shyness"]));
        // Nothing new: nothing sent.
        let r = add(&f, "mine", &[song("cat", "Cat's Eye")]).await.unwrap();
        assert!(r.uris.is_empty());
        assert_eq!(f.sent.lock().unwrap().len(), 1);
    }

    /// A real preview, fetched and played at volume 0 (needs the network and an
    /// audio output): `cargo test live_preview -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_preview() {
        let catalog = Catalog::default();
        let uri = "spotify:track:0VjIjW4GlUZAMYd2vXMi3b";
        catalog.add(crate::ai::spotify::Item { uri: uri.into(), preview: Some("https://p.scdn.co/mp3-preview/51c08d92815cce4ac2de94a7335a430b81234624".into()), ..Default::default() });
        let started = Instant::now();
        let bytes = clip(&fake(), &reqwest::Client::new(), &catalog, uri).await.unwrap().expect("a preview");
        println!("{} KB in {} ms", bytes.len() / 1024, started.elapsed().as_millis());
        let voice = tokio::task::spawn_blocking(move || crate::dj::Voice::play(&[std::sync::Arc::new(bytes)], 0.0)).await.unwrap().expect("plays");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!voice.done(), "30 seconds of it are still playing");
    }

    #[tokio::test]
    async fn a_songs_preview_comes_from_its_details_when_not_known() {
        let mut f = fake();
        f.data.insert("/tracks/plain".into(), json!({ "uri": "spotify:track:plain", "name": "No Clip" }));
        let http = reqwest::Client::new();
        assert_eq!(clip(&f, &http, &Catalog::default(), "spotify:track:plain").await, Ok(None), "no preview: said so, nothing fetched");
        assert!(clip(&f, &http, &Catalog::default(), "spotify:track:missing").await.is_err());
    }
}
