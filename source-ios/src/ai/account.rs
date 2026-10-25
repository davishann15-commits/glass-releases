//! ChatGPT accounts registered on this computer, and fresh access tokens.
//!
//! Non-secret registration facts (issued client ID, verified subject, email,
//! whether plan usage was granted) and this installation's host ID live in
//! `<app data>/assistant/`. Tokens live only in the OS credential store.

use super::oauth::{self, Attempt, AuthError, Client, Listener};
use super::secrets::{self, SecretStore, TokenSet};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Registration {
    pub client_id: String,
    /// The verified ID-token subject (None until the first sign-in completes).
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    /// Whether `chatgpt.tokens.use.direct` was granted at the last sign-in.
    #[serde(default)]
    pub plan_enabled: bool,
}

impl Registration {
    pub fn label(&self) -> String {
        self.email.clone().unwrap_or_else(|| format!("ChatGPT account ({}…)", self.client_id.chars().take(12).collect::<String>()))
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
struct File {
    #[serde(default)]
    registrations: Vec<Registration>,
    #[serde(default)]
    active: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct HostFile {
    ext_agent_host_id: String,
}

/// The registrations file and this installation's host ID.
#[derive(Clone, Debug, PartialEq)]
pub struct Accounts {
    dir: PathBuf,
    pub host_id: String,
    file: File,
}

impl Accounts {
    pub fn dir() -> PathBuf {
        crate::config::config_dir().join("assistant")
    }

    /// Loads the registrations and the host ID, creating and saving the host
    /// ID the first time (before any sign-in, as required).
    pub fn load(dir: PathBuf) -> Self {
        let host_path = dir.join("host.json");
        let host_id =
            std::fs::read_to_string(&host_path).ok().and_then(|s| serde_json::from_str::<HostFile>(&s).ok()).map(|h| h.ext_agent_host_id).filter(|h| h.starts_with("urn:uuid:")).unwrap_or_else(|| {
                let id = oauth::new_host_id();
                if let Err(e) = crate::config::write_private(&host_path, serde_json::to_string_pretty(&HostFile { ext_agent_host_id: id.clone() }).unwrap_or_default().as_bytes()) {
                    log::warn!("couldn't save the assistant host ID: {e}");
                }
                id
            });
        let file = crate::config::read_json(&dir.join("accounts.json"));
        Self { dir, host_id, file }
    }

    fn save(&self) {
        if let Err(e) = crate::config::write_private(&self.dir.join("accounts.json"), serde_json::to_string_pretty(&self.file).unwrap_or_default().as_bytes()) {
            log::warn!("couldn't save assistant accounts: {e}");
        }
    }

    pub fn registrations(&self) -> &[Registration] {
        &self.file.registrations
    }

    pub fn active(&self) -> Option<&Registration> {
        let id = self.file.active.as_ref()?;
        self.file.registrations.iter().find(|r| &r.client_id == id)
    }

    /// Adds or updates a registration (matched by issued client ID).
    pub fn upsert(&mut self, r: Registration) {
        match self.file.registrations.iter_mut().find(|x| x.client_id == r.client_id) {
            Some(x) => *x = r,
            None => self.file.registrations.push(r),
        }
        self.save();
    }

    pub fn set_active(&mut self, client_id: Option<String>) {
        self.file.active = client_id;
        self.save();
    }
}

/// Where the sign-in and API requests go (the documented endpoints; tests
/// point them at a local mock).
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    pub discovery: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self { authorize: oauth::AUTHORIZE.into(), token: oauth::TOKEN.into(), discovery: oauth::DISCOVERY.into(), api: oauth::RESOURCE.into() }
    }
}

/// What a completed sign-in produced.
#[derive(Clone, Debug)]
pub struct SignedIn {
    pub registration: Registration,
    pub plan_enabled: bool,
}

/// Signs in, refreshes and signs out. Holds no tokens itself.
pub struct Auth {
    pub store: Arc<dyn SecretStore>,
    pub endpoints: Endpoints,
    http: reqwest::Client,
    /// Refreshes for the same session are serialized (rotating refresh tokens).
    refreshing: tokio::sync::Mutex<()>,
}

impl Auth {
    pub fn new(store: Arc<dyn SecretStore>, endpoints: Endpoints) -> Self {
        let http = crate::config::http().timeout(Duration::from_secs(30)).build().unwrap_or_default();
        Self { store, endpoints, http, refreshing: tokio::sync::Mutex::new(()) }
    }

    /// Runs the whole browser sign-in. `client` is `New` for a first sign-in
    /// or the selected registration; `on_registered` is called with an issued
    /// client ID as soon as the callback provides it (so a later attempt can
    /// reuse it even if the code exchange fails).
    pub async fn sign_in(
        &self,
        client: Client,
        expect_subject: Option<String>,
        host_id: String,
        reconsent: bool,
        open_browser: impl FnOnce(&str),
        on_registered: impl FnOnce(&str),
    ) -> Result<SignedIn, AuthError> {
        let listener = Listener::start().await.map_err(|e| AuthError::Network(format!("couldn't open the sign-in callback: {e}")))?;
        let attempt = Attempt::new(client.clone(), listener.redirect_uri.clone(), host_id, reconsent);
        open_browser(&attempt.authorize_url(&self.endpoints.authorize));
        let cb = listener.wait(&attempt.state, Duration::from_secs(600)).await?;
        let (code, client_id) = attempt.check(&cb)?;
        if client == Client::New {
            on_registered(&client_id);
        }
        let tokens = oauth::exchange_code(&self.http, &self.endpoints.token, &client_id, &code, &attempt.verifier, &attempt.redirect_uri).await?;
        let id_token = tokens.id_token.clone().ok_or(AuthError::IdToken("no ID token in the response".into()))?;
        let discovery = oauth::discover(&self.http, &self.endpoints.discovery).await?;
        if discovery.issuer != oauth::ISSUER {
            return Err(AuthError::IdToken("unexpected issuer in OpenAI's OpenID configuration".into()));
        }
        let keys = oauth::jwks(&self.http, &discovery.jwks_uri).await?;
        let identity = oauth::verify_id_token(&id_token, &keys, &client_id, &attempt.nonce)?;
        if let Some(expected) = expect_subject
            && expected != identity.subject
        {
            return Err(AuthError::WrongAccount);
        }
        let scopes = tokens.scopes();
        let set = TokenSet {
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone().unwrap_or_default(),
            id_token,
            expires_at: oauth::now() + tokens.expires_in.unwrap_or(3600),
            earliest_refresh_at: tokens.earliest_refresh(),
            scopes,
        };
        let plan_enabled = set.plan_enabled();
        // Under the refresh lock: a refresh of the old tokens running now can't save
        // them over these (or delete them when the old refresh token is refused).
        let _guard = self.refreshing.lock().await;
        secrets::save_tokens(self.store.as_ref(), &client_id, &set).map_err(|e| AuthError::Keyring(e.to_string()))?;
        Ok(SignedIn { registration: Registration { client_id, subject: Some(identity.subject), email: identity.email, plan_enabled }, plan_enabled })
    }

    /// The retained ID token, for `id_token_hint` on the next sign-in.
    pub fn id_token_hint(&self, client_id: &str) -> Option<String> {
        secrets::load_tokens(self.store.as_ref(), client_id).ok().flatten().map(|t| t.id_token).filter(|t| !t.is_empty())
    }

    /// A usable access token for `client_id`, refreshed near expiry.
    pub async fn access_token(&self, client_id: &str) -> Result<String, AuthError> {
        let _guard = self.refreshing.lock().await;
        // A keychain that doesn't answer (locked, say) isn't a sign-in that ended.
        let tokens = secrets::load_tokens(self.store.as_ref(), client_id).map_err(AuthError::Keyring)?.ok_or(AuthError::SignInAgain("not signed in".into()))?;
        if !tokens.plan_enabled() {
            return Err(AuthError::Declined);
        }
        let now = oauth::now();
        let fresh_enough = tokens.expires_at > now + 120;
        if fresh_enough || (now < tokens.earliest_refresh_at && tokens.expires_at > now) {
            return Ok(tokens.access_token);
        }
        if tokens.refresh_token.is_empty() {
            return Err(AuthError::SignInAgain("no refresh token".into()));
        }
        match oauth::refresh(&self.http, &self.endpoints.token, client_id, &tokens.refresh_token).await {
            Ok(r) => {
                // Replace access token, expiry, scopes and the rotating refresh token together.
                let next = TokenSet {
                    access_token: r.access_token.clone(),
                    refresh_token: r.refresh_token.clone().unwrap_or(tokens.refresh_token),
                    id_token: r.id_token.clone().unwrap_or(tokens.id_token),
                    expires_at: now + r.expires_in.unwrap_or(3600),
                    earliest_refresh_at: r.earliest_refresh(),
                    scopes: if r.scope.is_some() { r.scopes() } else { tokens.scopes },
                };
                // The old refresh token is spent: if the new one can't be kept, this access
                // token still works for now (the next sign-in will be needed sooner).
                if let Err(e) = secrets::save_tokens(self.store.as_ref(), client_id, &next) {
                    log::warn!("assistant sign-in couldn't be saved: {e}");
                }
                if !next.plan_enabled() {
                    return Err(AuthError::Declined);
                }
                Ok(next.access_token)
            }
            Err(e @ AuthError::SignInAgain(_)) => {
                // Unusable refresh token: clear it so the next attempt signs in again.
                let _ = self.store.delete(&secrets::tokens_key(client_id));
                Err(e)
            }
            // A network or server failure: keep the tokens and report it.
            Err(e) => Err(e),
        }
    }

    /// Signs out: revoke the renewable session (with a couple of retries),
    /// then clear the tokens. Returns whether revocation was confirmed.
    pub async fn sign_out(&self, client_id: &str) -> bool {
        let _guard = self.refreshing.lock().await;
        let loaded = secrets::load_tokens(self.store.as_ref(), client_id);
        if let Err(e) = &loaded {
            log::warn!("couldn't read the ChatGPT sign-in to revoke it: {e}");
        }
        // Nothing saved: nothing to revoke. A store that didn't answer may still hold a session.
        let mut confirmed = matches!(loaded, Ok(None));
        if let Some(t) = loaded.ok().flatten().filter(|t| !t.refresh_token.is_empty())
            && let Ok(d) = oauth::discover(&self.http, &self.endpoints.discovery).await
        {
            for wait in [0u64, 1, 3] {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                match oauth::revoke(&self.http, &d.revocation_endpoint, client_id, &t.refresh_token).await {
                    Ok(()) => {
                        confirmed = true;
                        break;
                    }
                    Err(AuthError::Network(_)) | Err(AuthError::Http(500..=599, _)) => continue,
                    Err(_) => break,
                }
            }
        }
        if let Err(e) = self.store.delete(&secrets::tokens_key(client_id)) {
            log::warn!("couldn't clear the ChatGPT sign-in: {e}");
            confirmed = false;
        }
        confirmed
    }
}

#[cfg(test)]
mod tests {
    use super::super::secrets::MemoryStore;
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = crate::config::test_dir(&format!("ai-accounts-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_host_id_is_made_once_and_kept() {
        let dir = temp("host");
        let a = Accounts::load(dir.clone());
        assert!(a.host_id.starts_with("urn:uuid:"));
        assert_eq!(Accounts::load(dir.clone()).host_id, a.host_id, "persisted before any sign-in");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("host.json")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registrations_stay_separate() {
        let dir = temp("regs");
        let mut a = Accounts::load(dir.clone());
        a.upsert(Registration { client_id: "oaiapp_1".into(), subject: Some("s1".into()), email: Some("same@example.com".into()), plan_enabled: true });
        a.upsert(Registration { client_id: "oaiapp_2".into(), subject: Some("s2".into()), email: Some("same@example.com".into()), plan_enabled: false });
        a.set_active(Some("oaiapp_2".into()));
        let b = Accounts::load(dir.clone());
        assert_eq!(b.registrations().len(), 2, "same email, two registrations");
        assert_eq!(b.active().unwrap().subject.as_deref(), Some("s2"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A scripted token endpoint on localhost.
    async fn token_server(replies: Vec<(u16, String)>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            for (status, body) in replies {
                let (mut s, _) = l.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap();
                seen.push(String::from_utf8_lossy(&buf[..n]).to_string());
                let resp = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                s.write_all(resp.as_bytes()).await.unwrap();
            }
            seen
        });
        (format!("http://{addr}"), task)
    }

    fn tokens(expires_at: u64) -> TokenSet {
        TokenSet {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            id_token: "id".into(),
            expires_at,
            earliest_refresh_at: 0,
            scopes: vec![oauth::PLAN_SCOPE.into(), "openid".into()],
        }
    }

    #[tokio::test]
    async fn refresh_rotates_tokens_and_clears_unusable_ones() {
        let store = Arc::new(MemoryStore::default());
        secrets::save_tokens(store.as_ref(), "oaiapp_1", &tokens(oauth::now() + 30)).unwrap();
        let ok = r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600,"scope":"chatgpt.tokens.use.direct openid"}"#.to_string();
        let (base, server) = token_server(vec![(200, ok), (400, r#"{"error":"refresh_token_reused"}"#.into())]).await;
        let auth = Auth::new(store.clone(), Endpoints { token: format!("{base}/token"), ..Endpoints::default() });
        assert_eq!(auth.access_token("oaiapp_1").await.unwrap(), "new-access", "near expiry: refreshed");
        let saved = secrets::load_tokens(store.as_ref(), "oaiapp_1").unwrap().unwrap();
        assert_eq!((saved.refresh_token.as_str(), saved.expires_at > oauth::now() + 3000), ("new-refresh", true), "replaced together");
        assert_eq!(auth.access_token("oaiapp_1").await.unwrap(), "new-access", "fresh: no request");
        // Force another refresh that fails terminally.
        secrets::save_tokens(store.as_ref(), "oaiapp_1", &tokens(oauth::now())).unwrap();
        assert!(matches!(auth.access_token("oaiapp_1").await, Err(AuthError::SignInAgain(_))));
        assert_eq!(secrets::load_tokens(store.as_ref(), "oaiapp_1").unwrap(), None, "unusable tokens are cleared");
        let seen = server.await.unwrap();
        assert!(seen[0].contains("grant_type=refresh_token") && seen[0].contains("client_id=oaiapp_1") && seen[0].contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"));
        assert!(!seen[0].contains("scope="), "scope omitted to keep the grant");
    }

    #[tokio::test]
    async fn a_network_failure_keeps_the_tokens() {
        let store = Arc::new(MemoryStore::default());
        secrets::save_tokens(store.as_ref(), "oaiapp_1", &tokens(oauth::now())).unwrap();
        let auth = Auth::new(store.clone(), Endpoints { token: "http://127.0.0.1:9/token".into(), ..Endpoints::default() });
        assert!(matches!(auth.access_token("oaiapp_1").await, Err(AuthError::Network(_))));
        assert!(secrets::load_tokens(store.as_ref(), "oaiapp_1").unwrap().is_some(), "not erased for a temporary failure");
    }

    #[tokio::test]
    async fn without_plan_permission_there_is_no_token() {
        let store = Arc::new(MemoryStore::default());
        let mut t = tokens(oauth::now() + 3600);
        t.scopes = vec!["openid".into(), "email".into()];
        secrets::save_tokens(store.as_ref(), "oaiapp_1", &t).unwrap();
        let auth = Auth::new(store, Endpoints::default());
        assert_eq!(auth.access_token("oaiapp_1").await, Err(AuthError::Declined), "a valid sign-in alone doesn't authorize plan usage");
    }

    #[tokio::test]
    async fn sign_out_revokes_then_clears() {
        let store = Arc::new(MemoryStore::default());
        secrets::save_tokens(store.as_ref(), "oaiapp_1", &tokens(oauth::now() + 3600)).unwrap();
        let (base, server) = token_server(vec![]).await;
        drop(server);
        // Discovery unreachable: revocation can't be confirmed, tokens are still cleared.
        let auth = Auth::new(store.clone(), Endpoints { discovery: format!("{base}/nope"), ..Endpoints::default() });
        assert!(!auth.sign_out("oaiapp_1").await);
        assert_eq!(secrets::load_tokens(store.as_ref(), "oaiapp_1").unwrap(), None);
    }

    /// A keychain that doesn't answer (locked, say).
    struct Locked;

    impl SecretStore for Locked {
        fn get(&self, _: &str) -> Result<Option<String>, String> {
            Err("locked".into())
        }
        fn set(&self, _: &str, _: &str) -> Result<(), String> {
            Err("locked".into())
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Err("locked".into())
        }
        fn describe(&self) -> &'static str {
            "a locked keychain"
        }
    }

    #[tokio::test]
    async fn an_unreadable_keychain_isnt_a_revoked_sign_in() {
        let auth = Auth::new(Arc::new(Locked), Endpoints::default());
        assert!(!auth.sign_out("oaiapp_1").await, "the session may still be there");
    }
}
