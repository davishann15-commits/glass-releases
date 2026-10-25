//! Signing in to Spotify on iOS.
//!
//! The desktop opens the browser and waits on 127.0.0.1:8898 (librespot-oauth).
//! On iOS the app presents Spotify's own page in the system's web sign-in sheet
//! (ASWebAuthenticationSession), and the core waits on the same loopback address
//! — the redirect URI the client ID is registered with — while the sheet is up
//! (the app is in front then, so its listener can answer). When Spotify's redirect
//! arrives, the core checks `state`, answers with a redirect to
//! `glass-ios://spotify-signed-in` (which closes the sheet), and exchanges the
//! code itself (PKCE, no client secret).

use crate::auth::{Auth, REDIRECT_URI, SCOPES};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use librespot_oauth::OAuthToken;
use sha2::Digest;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const CALLBACK_SCHEME: &str = "glass-ios";
const DONE_URL: &str = "glass-ios://spotify-signed-in";
/// Longer than anyone takes to sign in; the sheet's Cancel ends it sooner.
const WAIT: Duration = Duration::from_secs(600);

pub struct Pending {
    pub url: String,
    verifier: String,
    state: String,
    listener: tokio::net::TcpListener,
}

fn random_string(n: usize) -> String {
    use rand::Rng;
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..n).map(|_| SET[rng.gen_range(0..SET.len())] as char).collect()
}

/// Opens the listener and makes the authorize URL. The listener is open before
/// the URL is handed out, so the redirect can't arrive first.
pub async fn begin(client_id: &str) -> Result<Pending> {
    let addr = REDIRECT_URI.trim_start_matches("http://").split('/').next().unwrap_or("127.0.0.1:8898").to_string();
    let listener = tokio::net::TcpListener::bind(&addr).await.with_context(|| format!("couldn't listen on {addr} for Spotify's sign-in"))?;
    let verifier = random_string(64);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    let state = random_string(24);
    let enc = crate::api::enc;
    let url = format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}&scope={}",
        enc(client_id),
        enc(REDIRECT_URI),
        enc(&challenge),
        enc(&state),
        enc(&SCOPES.join(" "))
    );
    Ok(Pending { url, verifier, state, listener })
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| url::form_urlencoded::parse(format!("v={v}").as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default())
    })
}

/// Waits for Spotify's redirect, checks it, and trades the code for tokens.
pub async fn finish(p: Pending, client_id: &str) -> Result<Auth> {
    let code = tokio::time::timeout(WAIT, wait_for_code(&p)).await.map_err(|_| anyhow!("the sign-in took too long"))??;
    let token = exchange(client_id, &code, &p.verifier).await?;
    Auth::from_token(client_id, token)
}

async fn wait_for_code(p: &Pending) -> Result<String> {
    loop {
        let (mut sock, _) = p.listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let mut n = 0;
        // The request line is all that's needed.
        while n < buf.len() {
            let r = sock.read(&mut buf[n..]).await?;
            if r == 0 {
                break;
            }
            n += r;
            if buf[..n].windows(2).any(|w| w == b"\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf[..n]);
        let Some(target) = text.lines().next().and_then(|l| l.split_whitespace().nth(1)) else { continue };
        let Some((path, query)) = target.split_once('?') else {
            // A favicon or a stray request: not the redirect.
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        };
        if path != "/login" {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        // Closes the sheet (it watches for the glass-ios scheme) whatever the outcome.
        let reply = format!("HTTP/1.1 302 Found\r\nLocation: {DONE_URL}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = sock.write_all(reply.as_bytes()).await;
        let _ = sock.shutdown().await;
        if let Some(e) = query_value(query, "error") {
            if e == "cancelled" {
                bail!("cancelled");
            }
            bail!("Spotify didn't sign you in ({e})");
        }
        // A redirect that isn't the answer to this sign-in (an old tab, or a forged
        // request from another app) is refused.
        if query_value(query, "state").as_deref() != Some(p.state.as_str()) {
            bail!("the sign-in answer didn't match this sign-in; try again");
        }
        return query_value(query, "code").filter(|c| !c.is_empty()).ok_or_else(|| anyhow!("Spotify's answer had no code"));
    }
}

async fn exchange(client_id: &str, code: &str, verifier: &str) -> Result<OAuthToken> {
    let form = [("grant_type", "authorization_code"), ("code", code), ("redirect_uri", REDIRECT_URI), ("client_id", client_id), ("code_verifier", verifier)];
    let r = crate::config::http().timeout(Duration::from_secs(30)).build()?.post("https://accounts.spotify.com/api/token").form(&form).send().await?;
    let status = r.status();
    let v: serde_json::Value = r.json().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Spotify refused the sign-in ({}: {})", status.as_u16(), v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no reason given"));
    }
    let access = v["access_token"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| anyhow!("Spotify's answer had no access token"))?;
    Ok(OAuthToken {
        access_token: access.to_string(),
        refresh_token: v["refresh_token"].as_str().unwrap_or_default().to_string(),
        expires_at: Instant::now() + Duration::from_secs(v["expires_in"].as_u64().unwrap_or(3600)),
        token_type: v["token_type"].as_str().unwrap_or("Bearer").to_string(),
        scopes: v["scope"].as_str().unwrap_or_default().split_whitespace().map(str::to_string).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_query_values() {
        assert_eq!(query_value("code=a%2Bb&state=xyz", "code").as_deref(), Some("a+b"));
        assert_eq!(query_value("code=abc&state=xyz", "state").as_deref(), Some("xyz"));
        assert_eq!(query_value("error=access_denied", "code"), None);
    }

    #[tokio::test]
    async fn refuses_a_redirect_for_another_sign_in() {
        // A free port: the test doesn't need the real redirect address.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let p = Pending { url: String::new(), verifier: "v".into(), state: "right".into(), listener };
        let wait = tokio::spawn(async move { wait_for_code(&p).await });
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /login?code=c&state=wrong HTTP/1.1\r\n\r\n").await.unwrap();
        let mut reply = String::new();
        s.read_to_string(&mut reply).await.unwrap();
        assert!(reply.starts_with("HTTP/1.1 302") && reply.contains(DONE_URL), "the sheet still closes");
        assert!(wait.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn accepts_the_matching_redirect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let p = Pending { url: String::new(), verifier: "v".into(), state: "s1".into(), listener };
        let wait = tokio::spawn(async move { wait_for_code(&p).await });
        // A stray request first (a favicon), then the redirect.
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").await.unwrap();
        let mut ignored = String::new();
        s.read_to_string(&mut ignored).await.unwrap();
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /login?code=the-code&state=s1 HTTP/1.1\r\n\r\n").await.unwrap();
        assert_eq!(wait.await.unwrap().unwrap(), "the-code");
    }
}
