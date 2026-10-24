//! Sign in with ChatGPT for an open-source, locally run app, following
//! developers.openai.com/siwc/token-sharing-open-source exactly:
//!
//! - a persisted, opaque `ext_agent_host_id` for this installation;
//! - first sign-in registers through `client_id=dynamic_agent_client` and
//!   returns an issued `client_id` that later sign-ins reuse;
//! - PKCE (S256), fresh `state` and `nonce` per attempt, the system browser,
//!   and a loopback callback on `http://127.0.0.1:<port>/auth/callback`;
//! - the ID token is verified against OpenAI's published JWKS (issuer,
//!   audience = issued client ID, expiry, nonce), and ChatGPT plan usage is
//!   enabled only when `chatgpt.tokens.use.direct` was actually granted.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const ISSUER: &str = "https://auth.openai.com";
pub const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
pub const RESOURCE: &str = "https://api.openai.com/v1";
/// The first-time registration entrypoint — never saved or used for exchange.
pub const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
pub const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// The app's actual name, used consistently as `agent_name_hint`.
pub const AGENT_NAME: &str = "Glass";
/// ChatGPT Settings → Usage: review usage and manage this app's access.
pub const MANAGE_USAGE_URL: &str = "https://chatgpt.com/settings/usage";
pub const CALLBACK_PATH: &str = "/auth/callback";
pub const PREFERRED_PORT: u16 = 1455;

#[derive(Debug, Clone, PartialEq)]
pub enum AuthError {
    /// The user declined (or the result lacks) permission.
    Declined,
    StateMismatch,
    /// The callback named a different client than the one being reauthorized.
    ClientMismatch,
    /// A new registration came back without an issued client ID.
    RegistrationIncomplete,
    /// The code expired or was used; start a fresh authorization.
    InvalidGrant,
    InvalidClient,
    /// The refresh token can't be used any more; sign in again.
    SignInAgain(String),
    /// The ID token didn't verify.
    IdToken(String),
    /// A different ChatGPT account signed in than the one selected.
    WrongAccount,
    Http(u16, String),
    Network(String),
    /// The system keychain (or Secret Service) wouldn't store or give back the tokens.
    Keyring(String),
    TimedOut,
    /// Paying with an API key, but none is saved (it was removed from the keychain).
    NoApiKey,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Declined => write!(f, "Permission wasn't granted in the browser."),
            AuthError::StateMismatch => write!(f, "The sign-in response didn't match this attempt; try again."),
            AuthError::ClientMismatch => write!(f, "The sign-in came back for a different app registration; nothing was changed."),
            AuthError::RegistrationIncomplete => write!(f, "OpenAI didn't finish registering Glass; try signing in again."),
            AuthError::InvalidGrant => write!(f, "The sign-in code expired; try again."),
            AuthError::InvalidClient => write!(f, "OpenAI doesn't recognize this app registration (invalid_client)."),
            AuthError::SignInAgain(why) => write!(f, "Your ChatGPT sign-in ended ({why}); sign in again."),
            AuthError::IdToken(why) => write!(f, "OpenAI's identity token didn't verify: {why}"),
            AuthError::WrongAccount => write!(f, "A different ChatGPT account signed in than the one selected; nothing was changed."),
            AuthError::Http(code, msg) => write!(f, "OpenAI sign-in answered {code}: {msg}"),
            AuthError::Network(e) => write!(f, "Couldn't reach OpenAI ({e})."),
            // Whole sentences, from the store (`secrets::friendly`) or whoever asked it.
            AuthError::Keyring(e) => write!(f, "{e}"),
            AuthError::TimedOut => write!(f, "Sign-in timed out waiting for the browser."),
            AuthError::NoApiKey => write!(f, "No API key saved. Add one in Settings → Assistant."),
        }
    }
}

pub fn random_token(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

/// PKCE's S256 challenge for `verifier`.
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A new installation's host ID: `urn:uuid:<v4>` (one of the documented formats).
pub fn new_host_id() -> String {
    format!("urn:uuid:{}", uuid::Uuid::new_v4().hyphenated())
}

/// Which registration an authorization is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Client {
    /// First sign-in for a ChatGPT account: register through `dynamic_agent_client`.
    New,
    /// A saved registration: its issued client ID and retained hints.
    Existing { client_id: String, id_token_hint: Option<String>, login_hint: Option<String> },
}

/// One authorization attempt's secrets and exact callback URI, kept until the callback.
pub struct Attempt {
    pub client: Client,
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub host_id: String,
    /// Ask for consent again (the user chose to enable plan usage after declining).
    pub reconsent: bool,
}

impl Attempt {
    pub fn new(client: Client, redirect_uri: String, host_id: String, reconsent: bool) -> Self {
        Self { client, state: random_token(24), nonce: random_token(24), verifier: random_token(48), redirect_uri, host_id, reconsent }
    }

    fn challenge(&self) -> String {
        challenge(&self.verifier)
    }

    /// The URL to open in the system browser. It may contain an ID token
    /// hint, so it is never logged.
    pub fn authorize_url(&self, authorize: &str) -> String {
        let mut u = url::Url::parse(authorize).expect("authorize endpoint is a URL");
        {
            let mut q = u.query_pairs_mut();
            match &self.client {
                Client::New => {
                    q.append_pair("client_id", DYNAMIC_CLIENT);
                    q.append_pair("agent_name_hint", AGENT_NAME);
                }
                Client::Existing { client_id, id_token_hint, login_hint } => {
                    q.append_pair("client_id", client_id);
                    if let Some(h) = id_token_hint {
                        q.append_pair("id_token_hint", h);
                    }
                    if let Some(h) = login_hint {
                        q.append_pair("login_hint", h);
                    }
                }
            }
            q.append_pair("ext_agent_host_id", &self.host_id);
            q.append_pair("response_type", "code");
            q.append_pair("redirect_uri", &self.redirect_uri);
            q.append_pair("scope", SCOPES);
            q.append_pair("resource", RESOURCE);
            q.append_pair("state", &self.state);
            q.append_pair("nonce", &self.nonce);
            q.append_pair("code_challenge_method", "S256");
            q.append_pair("code_challenge", &self.challenge());
            if self.reconsent {
                // The documented mechanism until `force_reconsent` is confirmed for this integration.
                q.append_pair("prompt", "consent");
            }
        }
        u.into()
    }

    /// Validates the browser's callback and returns (code, client ID to exchange with).
    pub fn check(&self, cb: &Callback) -> Result<(String, String), AuthError> {
        if cb.state.as_deref() != Some(self.state.as_str()) {
            return Err(AuthError::StateMismatch);
        }
        if let Some(e) = &cb.error {
            return Err(if e == "access_denied" { AuthError::Declined } else { AuthError::Http(400, e.clone()) });
        }
        let code = cb.code.clone().filter(|c| !c.is_empty()).ok_or(AuthError::Http(400, "no authorization code".into()))?;
        let client_id = match &self.client {
            Client::New => match &cb.client_id {
                Some(id) if !id.is_empty() && id != DYNAMIC_CLIENT => id.clone(),
                _ => return Err(AuthError::RegistrationIncomplete),
            },
            Client::Existing { client_id, .. } => match &cb.client_id {
                Some(id) if id != client_id => return Err(AuthError::ClientMismatch),
                _ => client_id.clone(),
            },
        };
        Ok((code, client_id))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub client_id: Option<String>,
}

pub fn parse_callback(query: &str) -> Callback {
    let mut cb = Callback::default();
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => cb.code = Some(v.into_owned()),
            "state" => cb.state = Some(v.into_owned()),
            "error" => cb.error = Some(v.into_owned()),
            "client_id" => cb.client_id = Some(v.into_owned()),
            _ => {}
        }
    }
    cb
}

// ------------------------------------------------------------------ listener

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Glass</title><body style=\"font-family:-apple-system,system-ui;background:#171a1f;color:#e9e4dc;display:flex;align-items:center;justify-content:center;height:100vh;margin:0\"><div><h2 style=\"font-weight:500\">You can return to Glass</h2><p style=\"opacity:.7\">This tab can be closed.</p></div>";
const STALE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Glass</title><body style=\"font-family:-apple-system,system-ui;background:#171a1f;color:#e9e4dc;display:flex;align-items:center;justify-content:center;height:100vh;margin:0\"><div><h2 style=\"font-weight:500\">This sign-in link is out of date.</h2><p style=\"opacity:.7\">This tab can be closed.</p></div>";

/// A page for the browser tab.
fn page(status: &str, body: &str) -> String {
    format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

/// The loopback listener, started before the browser opens.
pub struct Listener {
    listener: tokio::net::TcpListener,
    pub redirect_uri: String,
}

impl Listener {
    /// 127.0.0.1:1455 if free, otherwise any free port (only the port may vary).
    pub async fn start() -> std::io::Result<Self> {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PREFERRED_PORT)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let port = listener.local_addr()?.port();
        Ok(Self { listener, redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}") })
    }

    /// Waits for the browser to arrive at the callback path with this attempt's
    /// `state`. Other requests (favicon…) get a 404, callbacks with another state
    /// (a stale tab, another page) a 400, and waiting continues.
    pub async fn wait(&self, state: &str, timeout: Duration) -> Result<Callback, AuthError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let accept = async {
            loop {
                let Ok((mut sock, _)) = self.listener.accept().await else {
                    // Out of file handles, say: wait rather than spin.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                // A browser may open a connection it never uses: don't let it hold up the real one.
                let mut buf = vec![0u8; 8192];
                let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
                let (path, query) = target.split_once('?').unwrap_or((target, ""));
                if path != CALLBACK_PATH {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                }
                let cb = parse_callback(query);
                // Only this attempt's answer ends the wait.
                let fresh = cb.state.as_deref() == Some(state);
                let answer = if fresh { page("200 OK", DONE_PAGE) } else { page("400 Bad Request", STALE_PAGE) };
                let _ = sock.write_all(answer.as_bytes()).await;
                let _ = sock.shutdown().await;
                if fresh {
                    return cb;
                }
            }
        };
        tokio::time::timeout(timeout, accept).await.map_err(|_| AuthError::TimedOut)
    }
}

// --------------------------------------------------------------- token calls

/// The token endpoint's answer. Tokens are moved into a `TokenSet` at once.
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub earliest_refresh_at: Option<Value>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse").field("expires_in", &self.expires_in).field("scope", &self.scope).finish_non_exhaustive()
    }
}

impl TokenResponse {
    pub fn scopes(&self) -> Vec<String> {
        self.scope.as_deref().unwrap_or("").split_whitespace().map(str::to_string).collect()
    }
    /// `earliest_refresh_at` as Unix seconds (0 when it's missing or not a number).
    pub fn earliest_refresh(&self) -> u64 {
        match &self.earliest_refresh_at {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
            _ => 0,
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Maps an OAuth error response. Refresh-token failures that can't recover
/// become `SignInAgain`.
fn token_error(status: u16, body: &str, refreshing: bool) -> AuthError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v["error"].as_str().or_else(|| v["error"]["code"].as_str()).unwrap_or("").to_string();
    let terminal = ["invalid_refresh_token", "token_expired", "refresh_token_expired", "refresh_token_invalidated", "refresh_token_reused"];
    match code.as_str() {
        "invalid_grant" if refreshing => AuthError::SignInAgain(code),
        "invalid_grant" => AuthError::InvalidGrant,
        "invalid_client" => AuthError::InvalidClient,
        c if terminal.contains(&c) => AuthError::SignInAgain(code),
        _ => AuthError::Http(status, if code.is_empty() { body.chars().take(200).collect() } else { code }),
    }
}

async fn post_form(http: &reqwest::Client, url: &str, form: &[(&str, &str)], refreshing: bool) -> Result<TokenResponse, AuthError> {
    let resp = http.post(url).form(form).send().await.map_err(|e| AuthError::Network(e.without_url().to_string()))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| AuthError::Network(e.without_url().to_string()))?;
    if status != 200 {
        return Err(token_error(status, &body, refreshing));
    }
    serde_json::from_str(&body).map_err(|_| AuthError::Http(status, "unreadable token response".into()))
}

pub async fn exchange_code(http: &reqwest::Client, token_url: &str, client_id: &str, code: &str, verifier: &str, redirect_uri: &str) -> Result<TokenResponse, AuthError> {
    post_form(
        http,
        token_url,
        &[("grant_type", "authorization_code"), ("client_id", client_id), ("code", code), ("code_verifier", verifier), ("redirect_uri", redirect_uri), ("resource", RESOURCE)],
        false,
    )
    .await
}

/// Standard refresh; `scope` omitted to keep the grant.
pub async fn refresh(http: &reqwest::Client, token_url: &str, client_id: &str, refresh_token: &str) -> Result<TokenResponse, AuthError> {
    post_form(http, token_url, &[("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", refresh_token), ("resource", RESOURCE)], true).await
}

/// Ends the renewable session. An empty 200 is success (even for an already-invalid token).
pub async fn revoke(http: &reqwest::Client, revocation_url: &str, client_id: &str, refresh_token: &str) -> Result<(), AuthError> {
    let form = [("token", refresh_token), ("token_type_hint", "refresh_token"), ("client_id", client_id)];
    let resp = http.post(revocation_url).form(&form).send().await.map_err(|e| AuthError::Network(e.without_url().to_string()))?;
    match resp.status().as_u16() {
        200 => Ok(()),
        s => Err(AuthError::Http(s, "revocation failed".into())),
    }
}

#[derive(Deserialize, Clone)]
pub struct Discovery {
    pub issuer: String,
    pub jwks_uri: String,
    pub revocation_endpoint: String,
}

pub async fn discover(http: &reqwest::Client, url: &str) -> Result<Discovery, AuthError> {
    let resp = http.get(url).send().await.map_err(|e| AuthError::Network(e.without_url().to_string()))?;
    let s = resp.status().as_u16();
    if s != 200 {
        return Err(AuthError::Http(s, "OpenID configuration".into()));
    }
    resp.json().await.map_err(|e| AuthError::Network(e.to_string()))
}

pub async fn jwks(http: &reqwest::Client, url: &str) -> Result<jsonwebtoken::jwk::JwkSet, AuthError> {
    let resp = http.get(url).send().await.map_err(|e| AuthError::Network(e.without_url().to_string()))?;
    let s = resp.status().as_u16();
    if s != 200 {
        return Err(AuthError::Http(s, "JWKS".into()));
    }
    resp.json().await.map_err(|e| AuthError::Network(e.to_string()))
}

/// Who signed in, from a verified ID token.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    pub subject: String,
    pub email: Option<String>,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
}

/// Verifies signature (published JWKS, RS256), issuer, audience (the issued
/// client ID), expiry and the attempt's nonce.
pub fn verify_id_token(id_token: &str, keys: &jsonwebtoken::jwk::JwkSet, client_id: &str, nonce: &str) -> Result<Identity, AuthError> {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
    let bad = |e: &dyn std::fmt::Display| AuthError::IdToken(e.to_string());
    let header = decode_header(id_token).map_err(|e| bad(&e))?;
    if header.alg != Algorithm::RS256 {
        return Err(AuthError::IdToken("unexpected signing algorithm".into()));
    }
    let kid = header.kid.ok_or(AuthError::IdToken("no key id".into()))?;
    let jwk = keys.find(&kid).ok_or(AuthError::IdToken("signed with an unknown key".into()))?;
    let key = DecodingKey::from_jwk(jwk).map_err(|e| bad(&e))?;
    let mut v = Validation::new(Algorithm::RS256);
    v.set_issuer(&[ISSUER]);
    v.set_audience(&[client_id]);
    v.leeway = 60;
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let data = decode::<IdClaims>(id_token, &key, &v).map_err(|e| bad(&e))?;
    if data.claims.nonce.as_deref() != Some(nonce) {
        return Err(AuthError::IdToken("nonce doesn't match this sign-in".into()));
    }
    Ok(Identity { subject: data.claims.sub, email: data.claims.email })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::traits::PublicKeyParts;

    /// A test key pair and a JWKS with its public half.
    pub fn test_keys() -> (EncodingKey, jsonwebtoken::jwk::JwkSet) {
        let mut rng = rand::thread_rng();
        let private = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = private.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let enc = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let public = private.to_public_key();
        let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_value(serde_json::json!({ "keys": [{
            "kty": "RSA", "alg": "RS256", "use": "sig", "kid": "test-key",
            "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()), "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        }] }))
        .unwrap();
        (enc, jwks)
    }

    pub fn id_token(enc: &EncodingKey, client_id: &str, nonce: &str, exp_in: i64, iss: &str) -> String {
        let mut h = Header::new(jsonwebtoken::Algorithm::RS256);
        h.kid = Some("test-key".into());
        let exp = (now() as i64 + exp_in) as u64;
        jsonwebtoken::encode(&h, &serde_json::json!({ "iss": iss, "aud": client_id, "sub": "user-123", "email": "me@example.com", "nonce": nonce, "exp": exp, "iat": now() }), enc).unwrap()
    }

    fn attempt(client: Client) -> Attempt {
        Attempt::new(client, "http://127.0.0.1:1455/auth/callback".into(), "urn:uuid:00000000-0000-4000-8000-000000000000".into(), false)
    }

    fn params(url: &str) -> std::collections::HashMap<String, String> {
        url::Url::parse(url).unwrap().query_pairs().into_owned().collect()
    }

    #[test]
    fn first_sign_in_registers_through_the_dynamic_client() {
        let a = attempt(Client::New);
        let p = params(&a.authorize_url(AUTHORIZE));
        assert_eq!(p["client_id"], "dynamic_agent_client");
        assert_eq!(p["agent_name_hint"], "Glass");
        assert_eq!(p["ext_agent_host_id"], "urn:uuid:00000000-0000-4000-8000-000000000000");
        assert_eq!(p["response_type"], "code");
        assert_eq!(p["redirect_uri"], "http://127.0.0.1:1455/auth/callback");
        assert_eq!(p["scope"], "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct");
        assert_eq!(p["resource"], "https://api.openai.com/v1");
        assert_eq!(p["code_challenge_method"], "S256");
        assert_eq!(p["code_challenge"], URL_SAFE_NO_PAD.encode(Sha256::digest(a.verifier.as_bytes())));
        assert!(!p.contains_key("id_token_hint") && !p.contains_key("prompt"));
        assert!(a.authorize_url(AUTHORIZE).starts_with("https://auth.openai.com/api/accounts/authorize?"));
    }

    #[test]
    fn reauthorization_reuses_the_issued_client() {
        let a = attempt(Client::Existing { client_id: "oaiapp_abc".into(), id_token_hint: Some("old.id.token".into()), login_hint: Some("me@example.com".into()) });
        let p = params(&a.authorize_url(AUTHORIZE));
        assert_eq!(p["client_id"], "oaiapp_abc");
        assert!(!p.contains_key("agent_name_hint"), "only on first registration");
        assert_eq!((p["id_token_hint"].as_str(), p["login_hint"].as_str()), ("old.id.token", "me@example.com"));
        assert_eq!(p["ext_agent_host_id"], "urn:uuid:00000000-0000-4000-8000-000000000000");
        let mut again = attempt(Client::Existing { client_id: "oaiapp_abc".into(), id_token_hint: None, login_hint: None });
        again.reconsent = true;
        assert_eq!(params(&again.authorize_url(AUTHORIZE))["prompt"], "consent");
        // Every attempt is fresh.
        assert_ne!(a.state, again.state);
        assert_ne!(a.verifier, again.verifier);
    }

    #[test]
    fn callbacks_are_checked() {
        let a = attempt(Client::New);
        let ok = parse_callback(&format!("code=abc&scope=chatgpt.tokens.use.direct+email&state={}&client_id=oaiapp_new", a.state));
        assert_eq!(a.check(&ok), Ok(("abc".into(), "oaiapp_new".into())));
        assert_eq!(a.check(&parse_callback("code=abc&state=forged&client_id=oaiapp_new")), Err(AuthError::StateMismatch));
        assert_eq!(a.check(&parse_callback(&format!("error=access_denied&state={}", a.state))), Err(AuthError::Declined));
        assert_eq!(a.check(&parse_callback(&format!("code=abc&state={}", a.state))), Err(AuthError::RegistrationIncomplete), "a new registration needs its issued ID");
        assert_eq!(a.check(&parse_callback(&format!("code=abc&state={}&client_id=dynamic_agent_client", a.state))), Err(AuthError::RegistrationIncomplete));
        let re = attempt(Client::Existing { client_id: "oaiapp_abc".into(), id_token_hint: None, login_hint: None });
        assert_eq!(re.check(&parse_callback(&format!("code=c&state={}", re.state))), Ok(("c".into(), "oaiapp_abc".into())), "reauthorization may omit client_id");
        assert_eq!(re.check(&parse_callback(&format!("code=c&state={}&client_id=oaiapp_other", re.state))), Err(AuthError::ClientMismatch));
    }

    #[test]
    fn id_tokens_are_verified() {
        let (enc, keys) = test_keys();
        let good = id_token(&enc, "oaiapp_abc", "n1", 600, ISSUER);
        assert_eq!(verify_id_token(&good, &keys, "oaiapp_abc", "n1").unwrap(), Identity { subject: "user-123".into(), email: Some("me@example.com".into()) });
        assert!(matches!(verify_id_token(&good, &keys, "oaiapp_abc", "other-nonce"), Err(AuthError::IdToken(_))), "nonce");
        assert!(matches!(verify_id_token(&good, &keys, "oaiapp_other", "n1"), Err(AuthError::IdToken(_))), "audience");
        assert!(verify_id_token(&id_token(&enc, "oaiapp_abc", "n1", -3600, ISSUER), &keys, "oaiapp_abc", "n1").is_err(), "expired");
        assert!(verify_id_token(&id_token(&enc, "oaiapp_abc", "n1", 600, "https://evil.example"), &keys, "oaiapp_abc", "n1").is_err(), "issuer");
        let (other, _) = test_keys();
        assert!(verify_id_token(&id_token(&other, "oaiapp_abc", "n1", 600, ISSUER), &keys, "oaiapp_abc", "n1").is_err(), "signature");
    }

    #[test]
    fn token_errors_map_to_recovery() {
        assert_eq!(token_error(400, r#"{"error":"invalid_grant"}"#, false), AuthError::InvalidGrant);
        assert_eq!(token_error(400, r#"{"error":"invalid_grant"}"#, true), AuthError::SignInAgain("invalid_grant".into()));
        assert_eq!(token_error(400, r#"{"error":"refresh_token_reused"}"#, true), AuthError::SignInAgain("refresh_token_reused".into()));
        assert_eq!(token_error(401, r#"{"error":"invalid_client"}"#, true), AuthError::InvalidClient);
        assert!(matches!(token_error(502, "<html>", true), AuthError::Http(502, _)));
    }

    #[test]
    fn host_ids_and_pkce() {
        let h = new_host_id();
        assert!(h.starts_with("urn:uuid:") && h.len() == 45, "{h}");
        let a = Attempt::new(Client::New, "http://127.0.0.1:1/auth/callback".into(), h, false);
        let c = a.challenge();
        // RFC 7636: a 43–128 character verifier; the challenge is unpadded base64url.
        assert!(a.verifier.len() >= 43 && c.len() == 43 && !c.contains(['=', '+', '/']), "{c}");
        // The RFC's own example.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[tokio::test]
    async fn the_listener_answers_only_its_callback_path() {
        let l = Listener::start().await.unwrap();
        assert!(l.redirect_uri.starts_with("http://127.0.0.1:") && l.redirect_uri.ends_with("/auth/callback"));
        let port: u16 = l.redirect_uri.trim_start_matches("http://127.0.0.1:").split('/').next().unwrap().parse().unwrap();
        let get = |path: String| async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out).await;
            out
        };
        let waiting = tokio::spawn(async move { l.wait("s", Duration::from_secs(5)).await });
        assert!(get("/favicon.ico".into()).await.starts_with("HTTP/1.1 404"));
        assert!(get("/auth/callback?code=old&state=stale".into()).await.starts_with("HTTP/1.1 400"), "a stale tab doesn't end the sign-in");
        assert!(get("/auth/callback?code=x&state=s&client_id=oaiapp_1".into()).await.starts_with("HTTP/1.1 200"));
        let cb = waiting.await.unwrap().unwrap();
        assert_eq!((cb.code.as_deref(), cb.client_id.as_deref()), (Some("x"), Some("oaiapp_1")));
    }
}
