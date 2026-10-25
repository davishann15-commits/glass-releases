//! YouTube Music search and YouTube audio, without yt-dlp.
//!
//! Search uses YouTube Music's web API with its "Songs" and "Videos" filters. Audio comes
//! from the visionOS client of YouTube's player API (yt-dlp's default JS-less client),
//! which hands out ready-to-use AAC URLs: no signature or throttling JavaScript to run and
//! no proof-of-origin token. It does want a visitor ID, which the search response carries.
//! Files are fetched in 10 MB ranges; one request for a whole file is throttled to
//! playback speed.

use super::matching::{YtHit, rank_youtube};
use super::{Song, TIMEOUT};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

const SEARCH: &str = "https://music.youtube.com/youtubei/v1/search?alt=json&prettyPrint=false";
const MUSIC_VERSION: &str = "1.20250101.01.00";
const SONGS: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
const VIDEOS: &str = "EgWKAQIQAWoMEA4QChADEAQQCRAF";

const PLAYER: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
/// yt-dlp's `visionos` client (2026-08); bump these if YouTube stops answering it.
const VISION_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const VISION_VERSION: &str = "1.02";
const VISION_ID: &str = "101";
const CHUNK: u64 = 10 << 20;
/// AAC-LC 128 kbps in MP4.
const AAC_ITAG: u64 = 140;

pub struct Found {
    pub video_id: String,
    pub visitor: Option<String>,
}

/// The best YouTube Music match for the song.
pub async fn find(http: &reqwest::Client, song: &Song) -> Result<Found> {
    let query = format!("{} - {}", song.title, song.artists.join(","));
    let (songs, videos) = tokio::join!(search(http, &query, SONGS), search(http, &query, VIDEOS));
    let (songs, videos) = (songs?, videos?);
    let visitor = visitor_data(&songs).or_else(|| visitor_data(&videos));
    let mut hits = rows(&songs, true);
    hits.extend(rows(&videos, false));
    let video_id = rank_youtube(&hits, song).into_iter().next().map(|(id, _)| id).context("no match on YouTube Music")?;
    Ok(Found { video_id, visitor })
}

/// The raw response: it's ~1 MB of JSON, so it's scanned for result rows rather than parsed whole.
async fn search(http: &reqwest::Client, query: &str, filter: &str) -> Result<String> {
    let body = json!({
        "context": { "client": { "clientName": "WEB_REMIX", "clientVersion": MUSIC_VERSION, "hl": "en", "gl": "US" } },
        "query": query,
        "params": filter,
    });
    let resp = http.post(SEARCH).header("Referer", "https://music.youtube.com/search").json(&body).timeout(TIMEOUT).send().await?;
    Ok(resp.error_for_status()?.text().await?)
}

fn visitor_data(response: &str) -> Option<String> {
    let key = "\"visitorData\":\"";
    let start = response.find(key)? + key.len();
    let len = response[start..].find('"')?;
    Some(response[start..start + len].to_string())
}

/// Rows read `[name] [artist • album • m:ss]` (songs) or `[name] [channel • views • m:ss]` (videos).
fn rows(response: &str, is_song: bool) -> Vec<YtHit> {
    const KEY: &str = "\"musicResponsiveListItemRenderer\":";
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(i) = response[at..].find(KEY) {
        let start = at + i + KEY.len();
        // Parse just this row's object.
        let mut stream = serde_json::Deserializer::from_str(&response[start..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(row)) => {
                found.push(row);
                at = start + stream.byte_offset();
            }
            _ => at = start,
        }
    }
    found
        .iter()
        .filter_map(|row| {
            let runs = |col: &Value| -> Vec<String> {
                col["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"].as_array().map(|r| r.iter().filter_map(|x| x["text"].as_str()).map(String::from).collect()).unwrap_or_default()
            };
            let columns = row["flexColumns"].as_array()?;
            let name = runs(columns.first()?).concat();
            let mut groups = vec![String::new()];
            for run in runs(columns.get(1)?) {
                if run.trim() == "•" {
                    groups.push(String::new());
                } else {
                    groups.last_mut()?.push_str(&run);
                }
            }
            let duration = groups.last()?.trim().to_string();
            // Anything measured in hours isn't a song.
            if groups.len() < 2 || duration.split(':').count() != 2 {
                return None;
            }
            let video_id = row["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]["playNavigationEndpoint"]["watchEndpoint"]["videoId"]
                .as_str()
                .or_else(|| row["playlistItemData"]["videoId"].as_str())?;
            Some(YtHit { video_id: video_id.into(), name, artist: groups[0].trim().into(), duration, is_song })
        })
        .collect()
}

/// Downloads the video's AAC stream to `path`; `progress` gets (bytes so far, total).
///
/// Some visitor IDs land in a YouTube experiment whose stream URLs are then refused
/// (403), every time. A fresh visitor ID (`visitor_id`, under 1 KB) gets a working one.
pub async fn fetch(http: &reqwest::Client, video_id: &str, mut visitor: Option<String>, path: &std::path::Path, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let mut refused = None;
    for attempt in 0..4u64 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(300 * attempt)).await;
            visitor = fresh_visitor(http).await.ok().or(visitor);
        }
        let stream = audio(http, video_id, visitor.as_deref()).await?;
        let mut file = tokio::fs::File::create(path).await?;
        match download(http, &stream.url, stream.len, &mut file, |done| progress(done, stream.len)).await {
            Ok(()) => return Ok(()),
            Err(e) if e.downcast_ref::<reqwest::Error>().and_then(|e| e.status()) == Some(reqwest::StatusCode::FORBIDDEN) => {
                log::info!("YouTube refused the stream for {video_id}; trying with a new visitor ID");
                refused = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(refused.unwrap_or_else(|| anyhow!("YouTube refused the download")).context(format!("YouTube keeps refusing the audio of {video_id}")))
}

async fn fresh_visitor(http: &reqwest::Client) -> Result<String> {
    let body = json!({ "context": { "client": { "clientName": "WEB", "clientVersion": "2.20260708.05.00", "hl": "en", "gl": "US" } } });
    let resp: Value = http.post("https://www.youtube.com/youtubei/v1/visitor_id?prettyPrint=false").json(&body).timeout(TIMEOUT).send().await?.error_for_status()?.json().await?;
    resp["responseContext"]["visitorData"].as_str().map(String::from).context("no visitor ID")
}

struct Stream {
    url: String,
    len: u64,
}

/// The AAC stream's URL and size.
async fn audio(http: &reqwest::Client, video_id: &str, visitor: Option<&str>) -> Result<Stream> {
    let mut client = json!({
        "clientName": "VISIONOS", "clientVersion": VISION_VERSION, "deviceMake": "Apple", "deviceModel": "RealityDevice17,1",
        "userAgent": VISION_UA, "osName": "visionOS", "osVersion": "26.5.23O471", "hl": "en", "gl": "US",
    });
    let mut req =
        http.post(PLAYER).header("User-Agent", VISION_UA).header("X-YouTube-Client-Name", VISION_ID).header("X-YouTube-Client-Version", VISION_VERSION).header("Origin", "https://www.youtube.com");
    if let Some(v) = visitor {
        client["visitorData"] = json!(v);
        req = req.header("X-Goog-Visitor-Id", v);
    }
    let body = json!({ "context": { "client": client }, "videoId": video_id, "contentCheckOk": true, "racyCheckOk": true });
    let player: Value = req.json(&body).timeout(TIMEOUT).send().await?.error_for_status()?.json().await?;
    let status = &player["playabilityStatus"];
    if status["status"].as_str() != Some("OK") {
        bail!("YouTube won't play {video_id}: {}", status["reason"].as_str().unwrap_or("no reason given"));
    }
    let formats = player["streamingData"]["adaptiveFormats"].as_array().context("YouTube sent no streams")?;
    let usable = |f: &&Value| f["url"].is_string() && f["mimeType"].as_str().is_some_and(|m| m.starts_with("audio/mp4"));
    let format = formats
        .iter()
        .filter(usable)
        .find(|f| f["itag"].as_u64() == Some(AAC_ITAG))
        .or_else(|| formats.iter().filter(usable).max_by_key(|f| f["bitrate"].as_u64().unwrap_or(0)))
        .ok_or_else(|| anyhow!("YouTube sent no AAC stream for {video_id}"))?;
    let len = format["contentLength"].as_str().and_then(|l| l.parse().ok()).context("stream without a size")?;
    Ok(Stream { url: format["url"].as_str().unwrap_or_default().to_string(), len })
}

/// Fetches the stream in ranges into `file`; `progress` gets the bytes written so far.
async fn download(http: &reqwest::Client, url: &str, len: u64, file: &mut tokio::fs::File, mut progress: impl FnMut(u64)) -> Result<()> {
    let mut written = 0u64;
    while written < len {
        let end = (written + CHUNK).min(len) - 1;
        let mut tries = 0;
        let chunk_start = written;
        loop {
            let attempt = async {
                let mut resp = http.get(format!("{url}&range={written}-{end}")).header("User-Agent", VISION_UA).send().await?.error_for_status()?;
                while let Some(bytes) = resp.chunk().await? {
                    anyhow::ensure!(written + bytes.len() as u64 <= end + 1, "YouTube sent more than the range asked for");
                    file.write_all(&bytes).await?;
                    written += bytes.len() as u64;
                    progress(written);
                }
                anyhow::Ok(())
            }
            .await;
            match attempt {
                Ok(()) if written == end + 1 => break,
                // Ended early (it happens): ask for the rest of the range.
                Ok(()) if tries < 2 => {
                    tries += 1;
                    log::debug!("YouTube range {chunk_start}-{end} ended at {written}; asking for the rest");
                }
                Ok(()) => bail!("YouTube cut the download short"),
                // Refused (4xx): the same URL won't work on a second try.
                Err(e) if e.downcast_ref::<reqwest::Error>().and_then(|e| e.status()).is_some_and(|s| s.is_client_error()) => return Err(e),
                Err(e) if tries < 2 => {
                    tries += 1;
                    log::debug!("retrying YouTube range {chunk_start}-{end}: {e:#}");
                    // Start the range over.
                    file.set_len(chunk_start).await?;
                    use tokio::io::AsyncSeekExt;
                    file.seek(std::io::SeekFrom::Start(chunk_start)).await?;
                    written = chunk_start;
                }
                Err(e) => return Err(e),
            }
        }
    }
    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_filtered_search_rows() {
        let response = json!({"contents": [{"musicResponsiveListItemRenderer": {
            "flexColumns": [
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Blinding Lights"}]}}},
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                    {"text": "Matt Johnson"}, {"text": " & "}, {"text": "Jae Hall"}, {"text": " • "},
                    {"text": "Blinding Lights (Acoustic)"}, {"text": " • "}, {"text": "2:54"}]}}}
            ],
            "playlistItemData": {"videoId": "kBORugnc41M"}
        }}]});
        assert_eq!(
            rows(&response.to_string(), true),
            vec![YtHit { video_id: "kBORugnc41M".into(), name: "Blinding Lights".into(), artist: "Matt Johnson & Jae Hall".into(), duration: "2:54".into(), is_song: true }]
        );
    }
}
