//! Where the assistant's secrets live: the OS credential store (macOS
//! Keychain, or the Secret Service — GNOME Keyring / KWallet — on Linux).
//! Never in config files, theme files, logs or the UI.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The OAuth tokens for one ChatGPT registration (one issued client ID).
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    /// Unix seconds; don't refresh before this (from `earliest_refresh_at`).
    #[serde(default)]
    pub earliest_refresh_at: u64,
    pub scopes: Vec<String>,
}

impl TokenSet {
    pub fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == super::oauth::PLAN_SCOPE)
    }
}

// Tokens never appear in debug output, logs or panics.
impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet").field("expires_at", &self.expires_at).field("scopes", &self.scopes).finish_non_exhaustive()
    }
}

/// A place to keep secrets. The real one is the OS keyring; tests and the demo use memory.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    /// Shown in Settings ("the macOS Keychain").
    fn describe(&self) -> &'static str;
}

const SERVICE: &str = "Glass assistant";

pub struct Keyring;

impl SecretStore for Keyring {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(friendly)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(friendly(e)),
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, key).and_then(|e| e.set_password(value)).map_err(friendly)
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(friendly(e)),
        }
    }
    fn describe(&self) -> &'static str {
        if cfg!(target_os = "macos") { "the macOS Keychain" } else { "the system keyring (Secret Service)" }
    }
}

/// The whole sentence shown for a store error (`AuthError::Keyring` shows it as is).
fn friendly(e: keyring::Error) -> String {
    match e {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) if cfg!(target_os = "linux") => {
            "No system keyring is available. Install and unlock GNOME Keyring or KWallet (Secret Service), then try again.".into()
        }
        e if cfg!(target_os = "macos") => format!("The Keychain refused: {e}"),
        e => format!("The system keyring refused: {e}"),
    }
}

/// For tests and the demo: secrets in memory.
#[derive(Default, Clone)]
pub struct MemoryStore(pub Arc<Mutex<HashMap<String, String>>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
    fn describe(&self) -> &'static str {
        "memory (gone when Glass quits)"
    }
}

/// Keychain key for a registration's tokens.
pub fn tokens_key(client_id: &str) -> String {
    format!("chatgpt-tokens:{client_id}")
}

pub const API_KEY_KEY: &str = "openai-api-key";

/// The saved tokens, or `None` when there are none (or they're unreadable, which
/// also means signing in again). An error is the store's own: it didn't answer.
pub fn load_tokens(store: &dyn SecretStore, client_id: &str) -> Result<Option<TokenSet>, String> {
    Ok(store.get(&tokens_key(client_id))?.and_then(|s| {
        let t = serde_json::from_str(&s).ok();
        if t.is_none() {
            log::warn!("the saved ChatGPT sign-in is unreadable");
        }
        t
    }))
}

pub fn save_tokens(store: &dyn SecretStore, client_id: &str, t: &TokenSet) -> Result<(), String> {
    store.set(&tokens_key(client_id), &serde_json::to_string(t).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_never_show_in_debug_output() {
        let t = TokenSet {
            access_token: "secret-access".into(),
            refresh_token: "secret-refresh".into(),
            id_token: "secret-id".into(),
            expires_at: 1,
            earliest_refresh_at: 0,
            scopes: vec!["openid".into()],
        };
        let shown = format!("{t:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[test]
    fn round_trip() {
        let s = MemoryStore::default();
        let t = TokenSet { access_token: "a".into(), refresh_token: "r".into(), id_token: "i".into(), expires_at: 9, earliest_refresh_at: 0, scopes: vec![super::super::oauth::PLAN_SCOPE.into()] };
        save_tokens(&s, "oaiapp_1", &t).unwrap();
        assert_eq!(load_tokens(&s, "oaiapp_1").unwrap(), Some(t.clone()));
        assert!(t.plan_enabled());
        s.delete(&tokens_key("oaiapp_1")).unwrap();
        assert_eq!(load_tokens(&s, "oaiapp_1").unwrap(), None);
    }
}
