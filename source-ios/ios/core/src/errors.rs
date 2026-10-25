//! What the user is told when something fails: a few plain sentences, or nothing
//! when Glass recovers by itself. The error's own text (HTTP statuses, request
//! paths, librespot's and the OS's wording) goes to the log, never to the screen.

/// An error from Spotify's Web API, the network or the player, for a toast.
/// `None`: nothing to say (Glass resyncs or reconnects by itself).
pub fn plain(e: &str) -> Option<String> {
    log::warn!("{e}");
    let l = e.to_lowercase();
    // A player that's gone (replaced by a new one), or Spotify still catching up
    // (the player state is polled again at once), or a sign-in being renewed.
    if l.contains("channel closed") || l.contains("internal error") || l.starts_with("503") || l.contains("isn't ready") || l.starts_with("401") {
        return None;
    }
    let player = l.contains("/me/player");
    Some(
        if l.starts_with("403") && player {
            "That needs Spotify Premium."
        } else if l.starts_with("404") && player {
            "Nothing's playing. Choose a device in Devices."
        } else if l.starts_with("429") || l.contains("too many requests") {
            "Spotify is busy right now. Try again in a moment."
        } else if offline(&l) {
            "Can't reach Spotify. Check your connection."
        } else if l.starts_with("403") {
            "Spotify didn't allow that."
        } else if l.starts_with("404") {
            "That isn't available any more."
        } else {
            "That didn't work. Try again."
        }
        .to_string(),
    )
}

/// A page that couldn't load, under "couldn't load this" (it's retried by itself).
pub fn page(e: &str) -> String {
    // Glass's own explanations, already in plain words.
    if e.contains("isn't available in Glass") || e.contains("Spotify changed how") {
        return e.to_string();
    }
    let l = e.to_lowercase();
    if offline(&l) {
        "Check your connection; Glass tries again by itself.".into()
    } else if l.starts_with("404") {
        "This isn't available.".into()
    } else {
        "Glass tries again by itself, or pull to try now.".into()
    }
}

/// The network's at fault (no connection, a timeout, a server that didn't answer).
pub fn offline(l: &str) -> bool {
    ["timeout", "timed out", "connect", "error sending request", "dns", "network", "offline", "unreachable", "connection", "no route", "os error 50", "os error 51", "os error 60", "os error 65"].iter().any(|w| l.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_become_plain_words_or_nothing() {
        assert_eq!(plain("Internal error { channel closed }"), None);
        assert_eq!(plain("503 player state isn't ready yet"), None);
        assert_eq!(plain("403 Forbidden: /me/player/next?device_id=abc").as_deref(), Some("That needs Spotify Premium."));
        assert_eq!(plain("404 Not Found: /me/player/play").as_deref(), Some("Nothing's playing. Choose a device in Devices."));
        assert_eq!(plain("timeout /me/player/next?device_id=abc").as_deref(), Some("Can't reach Spotify. Check your connection."));
        assert_eq!(plain("error sending request for url (https://api.spotify.com/v1/me)").as_deref(), Some("Can't reach Spotify. Check your connection."));
        assert_eq!(plain("403 Forbidden: /playlists/x/tracks").as_deref(), Some("Spotify didn't allow that."));
        let other = plain("500 Internal Server Error: {\"error\":1}").unwrap();
        assert!(!other.contains("500") && !other.contains('{'), "{other}");
    }

    #[test]
    fn pages_keep_glass_own_words_only() {
        assert_eq!(page("This kind of page isn't available in Glass yet"), "This kind of page isn't available in Glass yet");
        assert!(!page("502 Bad Gateway: /v1/playlists/37i9").contains("502"));
        assert!(page("timeout connecting to Spotify").contains("connection"));
    }
}
