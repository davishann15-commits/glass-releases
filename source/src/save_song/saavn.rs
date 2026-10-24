//! JioSaavn: AAC in MP4, up to 320 kbps.

use super::matching::{SaavnHit, rank_saavn};
use super::{Song, TIMEOUT};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use ecb::cipher::{BlockDecryptMut, KeyInit, block_padding::Pkcs7};
use serde_json::Value;

const API: &str = "https://www.jiosaavn.com/api.php?_format=json&_marker=0&cc=in";

/// The best match's media URL and its bitrate.
pub async fn find(http: &reqwest::Client, song: &Song) -> Result<(String, u16)> {
    let url = format!("{API}&__call=autocomplete.get&includeMetaTags=1&query={}", enc(&song.title));
    let data: Value = http.get(url).timeout(TIMEOUT).send().await?.error_for_status()?.json().await?;
    let hits: Vec<SaavnHit> = data["songs"]["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|v| {
                    let more = &v["more_info"];
                    let artists = format!("{}, {}", text(&more["primary_artists"]), text(&more["singers"]));
                    SaavnHit { id: text(&v["id"]), title: text(&v["title"]), artists }
                })
                .collect()
        })
        .unwrap_or_default();
    let id = rank_saavn(&hits, song).into_iter().next().map(|(id, _)| id).context("no match on JioSaavn")?;

    let url = format!("{API}&__call=song.getDetails&pids={}", enc(&id));
    let data: Value = http.get(url).timeout(TIMEOUT).send().await?.error_for_status()?.json().await?;
    let details = if data[&id].is_object() { &data[&id] } else { &data["songs"][0] };
    let encrypted = details["encrypted_media_url"].as_str().context("JioSaavn gave no media URL")?;
    let is_320 = matches!(&details["320kbps"], Value::Bool(true)) || details["320kbps"].as_str() == Some("true");
    let kbps = if is_320 { 320 } else { 160 };
    // The URL ends in "_96.mp4"; the same file exists at the higher bitrates.
    let url = decrypt(encrypted)?;
    let base = url.rsplit_once('_').map(|(b, _)| b).unwrap_or(&url);
    Ok((format!("{base}_{kbps}.mp4").replace("http:", "https:"), kbps))
}

/// The web player's static key: DES-ECB("38346591") over base64.
fn decrypt(encrypted: &str) -> Result<String> {
    let mut buf = base64::engine::general_purpose::STANDARD.decode(encrypted.trim())?;
    let plain =
        ecb::Decryptor::<des::Des>::new_from_slice(b"38346591").map_err(|e| anyhow!("{e}"))?.decrypt_padded_mut::<Pkcs7>(&mut buf).map_err(|e| anyhow!("decrypting JioSaavn's media URL: {e}"))?;
    Ok(String::from_utf8(plain.to_vec())?)
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// JioSaavn HTML-escapes its text fields.
fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().replace("&quot;", "\"").replace("&#039;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    #[test]
    fn decrypts_media_urls() {
        let url = super::decrypt("ID2ieOjCrwfgWvL5sXl4B1ImC5QfbsDyryhkSYK5IH2E7FCO52VR6yhNbcEbes5iCcja4+W8xhE0SwtCJToN4Bw7tS9a8Gtq").unwrap();
        assert_eq!(url, "https://aac.saavncdn.com/871/c2febd353f3a076a406fa37510f31f9f_96.mp4");
    }
}
