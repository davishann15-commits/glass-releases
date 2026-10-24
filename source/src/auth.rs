//! Sign-in. Glass uses Spotify's normal OAuth (PKCE) page in your browser.
//! The refresh token is stored in `session.json`; librespot keeps its own
//! reusable login in `credentials.json` (see `engine`). Both are readable only
//! by you, in a folder only you can open. Access tokens stay in memory and are
//! refreshed about an hour later.

use crate::config::config_dir;
use anyhow::{Context, Result, anyhow};
use librespot_oauth::{OAuthClient, OAuthClientBuilder, OAuthToken};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::Mutex;

pub const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";

/// Every permission Glass asks for, and why.
pub const SCOPES: &[&str] = &[
    "streaming",                   // play audio on this computer
    "user-read-playback-state",    // see what's playing and on which device
    "user-modify-playback-state",  // play, pause, skip, queue, transfer
    "user-read-currently-playing", // now-playing bar
    "user-read-recently-played",   // Recents and Home
    "user-read-playback-position", // resume podcasts
    "user-top-read",               // Home and Profile
    "user-library-read",           // Your Library
    "user-library-modify",         // like, save, remove
    "user-follow-read",            // followed artists
    "user-follow-modify",          // follow and unfollow
    "playlist-read-private",       // your playlists
    "playlist-read-collaborative", // collaborative playlists
    "playlist-modify-private",     // edit your playlists
    "playlist-modify-public",      // edit your public playlists
    "ugc-image-upload",            // playlist cover images
    "user-read-private",           // country, product (Premium) and explicit filter
    "user-read-email",             // asked for as Spotify's own apps do; Glass doesn't show it
];

const DONE_PAGE: &str = "<html><body style=\"font-family:-apple-system;background:#111;color:#eee;\
display:flex;align-items:center;justify-content:center;height:100vh\"><div>\
<h2>Signed in to Glass</h2><p>You can close this tab.</p></div></body></html>";

#[derive(Serialize, Deserialize)]
struct Stored {
    refresh_token: String,
}

fn session_path() -> PathBuf {
    config_dir().join("session.json")
}

fn client(client_id: &str) -> Result<OAuthClient> {
    OAuthClientBuilder::new(client_id, REDIRECT_URI, SCOPES.to_vec()).open_in_browser().with_custom_message(DONE_PAGE).build().map_err(|e| anyhow!("{e}"))
}

/// Unblocks a sign-in that is still waiting for the browser, so the redirect
/// port is free for the next attempt.
pub fn cancel_sign_in() {
    use std::io::Write;
    let host = REDIRECT_URI.trim_start_matches("http://").split('/').next().unwrap_or_default();
    let Ok(addr) = host.parse() else { return };
    if let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)) {
        let _ = s.write_all(b"GET /login?error=cancelled HTTP/1.1\r\n\r\n");
    }
}

pub fn has_saved_session() -> bool {
    session_path().exists()
}

pub fn forget_session() {
    let _ = std::fs::remove_file(session_path());
}

/// Writes the session, readable only by this user.
fn save_refresh(token: &str) -> Result<()> {
    crate::config::write_private(&session_path(), serde_json::to_string(&Stored { refresh_token: token.into() })?.as_bytes())?;
    Ok(())
}

fn load_refresh() -> Option<String> {
    let s = std::fs::read_to_string(session_path()).ok()?;
    serde_json::from_str::<Stored>(&s).ok().map(|s| s.refresh_token)
}

/// Holds the current access token and refreshes it when it expires.
pub struct Auth {
    client_id: String,
    token: Mutex<OAuthToken>,
}

impl Auth {
    /// Opens the browser for sign-in. Blocks until the redirect arrives.
    pub async fn sign_in(client_id: &str) -> Result<Self> {
        let id = client_id.to_string();
        let token = tokio::task::spawn_blocking(move || client(&id)?.get_access_token().map_err(|e| anyhow!("{e}"))).await??;
        save_refresh(&token.refresh_token)?;
        Ok(Self { client_id: client_id.into(), token: Mutex::new(token) })
    }

    /// Uses the saved refresh token from a previous run.
    pub async fn resume(client_id: &str) -> Result<Self> {
        let refresh = load_refresh().context("no saved session")?;
        let mut token = client(client_id)?.refresh_token_async(&refresh).await.map_err(|e| anyhow!("{e}"))?;
        if token.refresh_token.is_empty() {
            token.refresh_token = refresh;
        }
        save_refresh(&token.refresh_token)?;
        Ok(Self { client_id: client_id.into(), token: Mutex::new(token) })
    }

    /// A valid bearer token, refreshing first if it expires within a minute.
    pub async fn bearer(&self) -> Result<String> {
        let mut t = self.token.lock().await;
        if t.expires_at <= Instant::now() + std::time::Duration::from_secs(60) {
            self.refresh_locked(&mut t).await?;
        }
        Ok(t.access_token.clone())
    }

    async fn refresh_locked(&self, t: &mut OAuthToken) -> Result<()> {
        let (mut fresh, used) = match client(&self.client_id)?.refresh_token_async(&t.refresh_token).await {
            Ok(f) => (f, t.refresh_token.clone()),
            // Another Glass on this computer (another build taking over) may
            // have refreshed since and been given a new token: the saved one.
            Err(e) => match load_refresh().filter(|saved| *saved != t.refresh_token) {
                Some(saved) => (client(&self.client_id)?.refresh_token_async(&saved).await.map_err(|e| anyhow!("{e}"))?, saved),
                None => return Err(anyhow!("{e}")),
            },
        };
        if fresh.refresh_token.is_empty() {
            fresh.refresh_token = used;
        }
        save_refresh(&fresh.refresh_token)?;
        *t = fresh;
        Ok(())
    }
}
