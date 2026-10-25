//! What the core needs from iOS itself, implemented in Swift (UniFFI foreign
//! traits): the Keychain. Set once when the core starts.

use crate::ai::secrets::SecretStore;
use std::sync::{Arc, OnceLock};

/// The iOS Keychain, implemented in Swift (generic passwords, kept on this
/// device only and readable after the first unlock, so a token can be
/// refreshed while the phone is locked during background playback).
#[uniffi::export(with_foreign)]
pub trait Keychain: Send + Sync {
    fn get(&self, key: String) -> Option<String>;
    /// False when the Keychain refused the write.
    fn set(&self, key: String, value: String) -> bool;
    fn delete(&self, key: String);
}

/// The Keychain as the shared code's `SecretStore` (Spotify's refresh token
/// in `auth`, the assistant's tokens and API key in `ai`).
struct KeychainStore(Arc<dyn Keychain>);

impl SecretStore for KeychainStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.get(key.to_string()))
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if self.0.set(key.to_string(), value.to_string()) { Ok(()) } else { Err("the Keychain didn't accept it".into()) }
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.delete(key.to_string());
        Ok(())
    }
    fn describe(&self) -> &'static str {
        "the iOS Keychain"
    }
}

static SECRETS: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();

/// Where secrets go: the Keychain once the app has handed it over, memory
/// before that (and in the demo, which never touches the real Keychain).
pub fn secrets() -> Arc<dyn SecretStore> {
    SECRETS.get_or_init(|| Arc::new(crate::ai::secrets::MemoryStore::default())).clone()
}

pub fn install_keychain(k: Option<Arc<dyn Keychain>>) {
    let store: Arc<dyn SecretStore> = match k {
        Some(k) => Arc::new(KeychainStore(k)),
        None => Arc::new(crate::ai::secrets::MemoryStore::default()),
    };
    let _ = SECRETS.set(store);
}

/// Apple's on-device translation (the Translation framework), implemented in
/// Swift. Takes and returns the macOS helper's JSON (tools/glass-translate):
/// in `{"lines":[…],"source":"ja"|null,"target":"en"}`, out `{"status":"ok"|
/// "same"|"needs_download"|"unsupported"|"undetermined"|"error", "source",
/// "lines", "message"}`. Called off the main thread; it may block until done.
#[uniffi::export(with_foreign)]
pub trait Translator: Send + Sync {
    fn translate(&self, request: String) -> String;
}

static TRANSLATOR: OnceLock<Arc<dyn Translator>> = OnceLock::new();
static LANGUAGE: OnceLock<String> = OnceLock::new();

pub fn translator() -> Option<Arc<dyn Translator>> {
    TRANSLATOR.get().cloned()
}

pub fn install_translator(t: Option<Arc<dyn Translator>>) {
    if let Some(t) = t {
        let _ = TRANSLATOR.set(t);
    }
}

/// The user's first preferred language ("en-US"), from iOS.
pub fn language() -> Option<String> {
    LANGUAGE.get().cloned()
}

pub fn install_language(l: &str) {
    if !l.is_empty() {
        let _ = LANGUAGE.set(l.to_string());
    }
}

static SHARE_NAME: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// How this device appears to your other Glass devices (the Connect device name).
pub fn share_name() -> Option<String> {
    SHARE_NAME.lock().unwrap().clone()
}

pub fn set_share_name(n: &str) {
    *SHARE_NAME.lock().unwrap() = (!n.trim().is_empty()).then(|| n.trim().to_string());
}
