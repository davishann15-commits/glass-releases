//! The two OpenAI calls the assistant makes: list models, and stream a
//! Responses API request. For ChatGPT plan usage both use the OAuth access
//! token; with the user's own API key they use that key instead. Nothing is
//! ever retried on a different billing path.

use crate::api::{LogEntry, NetLog};
use serde_json::{Value, json};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingKind {
    ChatGptPlan,
    ApiKey,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub slug: String,
    pub name: String,
}

pub const PREFERRED_MODEL: &str = "gpt-6-luna";

/// What went wrong, mapped to what the user can do about it.
#[derive(Clone, Debug, PartialEq)]
pub enum ApiError {
    /// Plan usage isn't available for this user/workspace/policy. Don't retry.
    NotEligible,
    /// The plan's (or this app's) usage limit was reached. Pause; link to usage.
    UsageLimit,
    /// Usage couldn't be checked, or user info is temporarily unavailable. Retry later.
    TryLater(String),
    /// A request feature isn't supported here (`error.param`).
    Unsupported(String),
    /// Signed identity or permission not accepted.
    Unauthorized(String),
    /// Policy (e.g. region) or permission check stopped the request.
    Forbidden(String),
    /// Too many requests at once.
    RateLimited(String),
    /// The API account is out of credits or over its spending limit.
    NoCredit,
    Http {
        status: u16,
        code: String,
        message: String,
        request_id: String,
    },
    Network(String),
    /// The model stopped early (`response.incomplete`).
    Incomplete(String),
    /// The stream ended without `response.completed`.
    Interrupted,
}

impl ApiError {
    /// Worth one more try after a pause (never on another billing path).
    pub fn transient(&self) -> bool {
        matches!(self, ApiError::TryLater(_) | ApiError::Network(_) | ApiError::Http { status: 500..=599, .. })
    }

    pub fn message(&self, kind: BillingKind) -> String {
        match self {
            ApiError::NotEligible => "ChatGPT plan usage isn't available for this account or workspace. Eligible ChatGPT Plus and Pro plans can use it.".into(),
            ApiError::UsageLimit => "Usage limit reached — your ChatGPT plan's or this app's limit. Review it in ChatGPT settings.".into(),
            ApiError::TryLater(m) => format!("OpenAI couldn't take the request right now ({m}). Try again in a moment."),
            ApiError::Unsupported(p) => format!("This model or account doesn't support part of the request ({p})."),
            ApiError::Unauthorized(m) if kind == BillingKind::ChatGptPlan => format!("OpenAI didn't accept the ChatGPT sign-in ({m}). Check the account in Settings → Assistant, or sign in again."),
            ApiError::Unauthorized(m) => format!("OpenAI didn't accept the API key ({m})."),
            ApiError::Forbidden(m) => format!("OpenAI declined the request: {m}"),
            ApiError::RateLimited(m) => format!("Rate limited by OpenAI ({m}). Try again in a moment."),
            ApiError::NoCredit => "Your OpenAI API account is out of credits or over its spending limit. Check billing on platform.openai.com.".into(),
            ApiError::Http { status, code, message, request_id } => {
                let code = if code.is_empty() { String::new() } else { format!(" {code}") };
                let id = if request_id.is_empty() { String::new() } else { format!(" · request {request_id}") };
                format!("OpenAI answered {status}{code}: {message}{id}")
            }
            ApiError::Network(e) => format!("Couldn't reach OpenAI ({e}). The assistant needs a connection."),
            ApiError::Incomplete(why) => format!("The reply stopped early ({why})."),
            ApiError::Interrupted => "The connection dropped before the reply finished.".into(),
        }
    }
}

/// Maps an HTTP error before a stream opens: a Responses error object
/// (`error.code`) or a direct-admission body (`{"detail": ...}`).
pub fn classify(status: u16, body: &str, request_id: &str) -> ApiError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v["error"]["code"].as_str().unwrap_or("").to_string();
    let param = v["error"]["param"].as_str().unwrap_or("").to_string();
    let message = v["error"]["message"].as_str().or_else(|| v["detail"].as_str()).map(str::to_string).unwrap_or_else(|| body.chars().take(160).collect());
    match code.as_str() {
        "subscription_sharing_user_not_eligible" => return ApiError::NotEligible,
        "subscription_sharing_usage_limit_exceeded" => return ApiError::UsageLimit,
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => return ApiError::TryLater(code),
        "subscription_sharing_unsupported_capability" => return ApiError::Unsupported(if param.is_empty() { message } else { param }),
        "subscription_sharing_route_not_supported" | "chatpass_v2_scope_not_authorized" | "chatpass_v2_invalid_authorization_context" => return ApiError::Forbidden(code),
        "subscription_sharing_invalid_user" => return ApiError::Unauthorized(code),
        "rate_limit_exceeded" => return ApiError::RateLimited(code),
        "insufficient_quota" => return ApiError::NoCredit,
        _ => {}
    }
    match status {
        401 => ApiError::Unauthorized(message),
        403 => ApiError::Forbidden(message),
        429 => ApiError::RateLimited(message),
        503 => ApiError::TryLater(message),
        _ => ApiError::Http { status, code, message, request_id: request_id.to_string() },
    }
}

/// Token counts reported for one request.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

/// A finished request: its output items (to send back with tool results),
/// the text it wrote, and its usage.
#[derive(Clone, Debug, Default)]
pub struct Completed {
    pub items: Vec<Value>,
    pub text: String,
    pub usage: Usage,
    /// Text was cut off at the app's length limit.
    pub truncated: bool,
}

/// The app's cap on a chat reply's text (the plan route doesn't accept
/// `max_output_tokens`). Translations get more room: see `with_max_text`.
pub const MAX_TEXT: usize = 6_000;

pub struct Client {
    http: reqwest::Client,
    base: String,
    log: Option<NetLog>,
    max_text: usize,
}

impl Client {
    pub fn new(base: &str, log: Option<NetLog>) -> Self {
        // One connection pool for every request (a client per message would set up TLS each
        // time). A stream that goes quiet for two minutes has stalled: the model sends
        // progress events meanwhile.
        static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let http = HTTP.get_or_init(|| crate::config::http().connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(120)).build().unwrap_or_default()).clone();
        Self { http, base: base.trim_end_matches('/').to_string(), log, max_text: MAX_TEXT }
    }

    /// Allows replies up to `bytes` of text (a whole song's translated lyrics).
    pub fn with_max_text(mut self, bytes: usize) -> Self {
        self.max_text = bytes;
        self
    }

    fn note(&self, method: &str, path: &str, status: String, started: Instant) {
        if let Some(log) = &self.log {
            // Host and path only — never headers, tokens or content.
            log.push(LogEntry { at: SystemTime::now(), method: method.into(), path: format!("api.openai.com{path}"), status, millis: started.elapsed().as_millis() });
        }
    }

    /// The models this account (or key) can use, in the server's order (API keys:
    /// newest names first), keeping those meant for display that can call tools
    /// (API keys: GPT text models, by name).
    pub async fn models(&self, bearer: &str, kind: BillingKind) -> Result<Vec<Model>, ApiError> {
        let started = Instant::now();
        let resp = self.http.get(format!("{}/models", self.base)).bearer_auth(bearer).timeout(Duration::from_secs(20)).send().await.map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        let rid = resp.headers().get("x-request-id").and_then(|h| h.to_str().ok()).unwrap_or("").to_string();
        let body = resp.text().await.map_err(|e| ApiError::Network(e.to_string()))?;
        self.note("GET", "/v1/models", status.to_string(), started);
        if status != 200 {
            return Err(classify(status, &body, &rid));
        }
        Ok(parse_models(&serde_json::from_str(&body).unwrap_or(Value::Null), kind))
    }

    /// Streams one Responses request. `on_text` gets text as it arrives.
    /// Success only on `response.completed`.
    pub async fn stream(&self, bearer: &str, body: &Value, mut on_text: impl FnMut(&str)) -> Result<Completed, ApiError> {
        let started = Instant::now();
        let resp = self.http.post(format!("{}/responses", self.base)).bearer_auth(bearer).json(body).send().await.map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        let rid = resp.headers().get("x-request-id").and_then(|h| h.to_str().ok()).unwrap_or("").to_string();
        if status != 200 {
            let text = resp.text().await.unwrap_or_default();
            self.note("POST", "/v1/responses", status.to_string(), started);
            return Err(classify(status, &text, &rid));
        }
        let mut sse = Sse::default();
        let mut out = Completed::default();
        let mut resp = resp;
        let result = loop {
            let chunk = match resp.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break Err(ApiError::Interrupted),
                Err(e) => break Err(ApiError::Network(e.without_url().to_string())),
            };
            let mut done = None;
            for ev in sse.push(&chunk) {
                match handle_event(&ev, &mut out, self.max_text, &mut on_text) {
                    Step::Continue => {}
                    Step::Done(r) => {
                        done = Some(r);
                        break;
                    }
                }
            }
            if let Some(r) = done {
                break r;
            }
        };
        self.note("POST", "/v1/responses", if result.is_ok() { "200".into() } else { "200 (failed)".into() }, started);
        result.map(|()| out)
    }
}

pub fn parse_models(v: &Value, kind: BillingKind) -> Vec<Model> {
    // The ChatGPT-plan catalog: `models[]` with slug, display_name, visibility.
    if let Some(list) = v["models"].as_array() {
        return list
            .iter()
            .filter(|m| m["visibility"].as_str() == Some("list"))
            // Drop models that say they can't call functions (when they say so).
            .filter(|m| m["supports_function_calling"].as_bool() != Some(false) && m["supported_tools"].as_array().is_none_or(|t| t.iter().any(|x| x.as_str() == Some("function"))))
            .filter_map(|m| {
                let slug = m["slug"].as_str()?.to_string();
                Some(Model { name: m["display_name"].as_str().unwrap_or(&slug).to_string(), slug })
            })
            .collect();
    }
    // The API catalog: `data[].id`. Keep GPT text models (by name; the API list doesn't say which call tools).
    let skip = ["audio", "realtime", "transcribe", "tts", "image", "search", "embedding", "moderation", "instruct", "codex", "computer", "dall", "whisper"];
    let mut out: Vec<Model> = v["data"]
        .as_array()
        .map(|l| l.iter().filter_map(|m| m["id"].as_str()).filter(|id| id.starts_with("gpt-") && !skip.iter().any(|s| id.contains(s))).map(|id| Model { slug: id.into(), name: id.into() }).collect())
        .unwrap_or_default();
    if kind == BillingKind::ApiKey {
        out.sort_by(|a, b| b.slug.cmp(&a.slug));
    }
    out
}

/// The model to use: the user's saved choice if still offered (none if it isn't);
/// with nothing saved, GPT-6 Luna if offered; otherwise none (the user picks —
/// never a silent switch to something pricier).
pub fn choose_model(models: &[Model], saved: &str) -> Option<String> {
    if !saved.is_empty() {
        return models.iter().any(|m| m.slug == saved).then(|| saved.to_string());
    }
    models.iter().find(|m| m.slug == PREFERRED_MODEL).map(|m| m.slug.clone())
}

/// An estimated cost in US dollars for API-key usage, when the price is
/// known (from developers.openai.com model pages). Always labelled an estimate.
pub fn estimate_usd(model: &str, u: Usage) -> Option<f64> {
    let (inp, out) = match model {
        "gpt-6-luna" => (0.10, 0.50),
        _ => return None,
    };
    Some(u.input as f64 / 1e6 * inp + u.output as f64 / 1e6 * out)
}

// --------------------------------------------------------------------- SSE

#[derive(Default)]
struct Sse {
    buf: Vec<u8>,
}

impl Sse {
    /// Complete `data:` payloads in this chunk (events are separated by a blank line).
    /// The buffer is searched as bytes, so a stray invalid byte can't shift the cut.
    fn push(&mut self, chunk: &[u8]) -> Vec<Value> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        let find = |buf: &[u8], sep: &[u8]| buf.windows(sep.len()).position(|w| w == sep);
        loop {
            let end = match (find(&self.buf, b"\n\n"), find(&self.buf, b"\r\n\r\n")) {
                (Some(a), Some(b)) if b < a => (b, 4),
                (Some(a), _) => (a, 2),
                (None, Some(b)) => (b, 4),
                (None, None) => break,
            };
            let block = String::from_utf8_lossy(&self.buf[..end.0]);
            let data: String = block.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.trim_start()).collect::<Vec<_>>().join("\n");
            if !data.is_empty() && data != "[DONE]" {
                match serde_json::from_str(&data) {
                    Ok(v) => out.push(v),
                    Err(_) => out.push(json!({ "type": "glass.malformed_event" })),
                }
            }
            self.buf.drain(..end.0 + end.1);
        }
        out
    }
}

enum Step {
    Continue,
    Done(Result<(), ApiError>),
}

fn handle_event(ev: &Value, out: &mut Completed, max_text: usize, on_text: &mut impl FnMut(&str)) -> Step {
    match ev["type"].as_str().unwrap_or("") {
        "response.output_text.delta" => {
            let d = ev["delta"].as_str().unwrap_or("");
            if out.text.len() < max_text {
                let room = max_text - out.text.len();
                // Whole characters only, up to `room` bytes.
                let cut = d.char_indices().map(|(i, c)| i + c.len_utf8()).take_while(|&end| end <= room).last().unwrap_or(0);
                let take = &d[..cut];
                out.text.push_str(take);
                on_text(take);
                if take.len() < d.len() {
                    out.truncated = true;
                }
            } else {
                out.truncated = true;
            }
            Step::Continue
        }
        "response.output_item.done" => {
            if let Some(item) = ev.get("item") {
                out.items.push(item.clone());
            }
            Step::Continue
        }
        "response.completed" => {
            let r = &ev["response"];
            // The completed response's own output list is authoritative when present.
            if let Some(items) = r["output"].as_array().filter(|a| !a.is_empty()) {
                out.items = items.clone();
            }
            out.usage = Usage { input: r["usage"]["input_tokens"].as_u64().unwrap_or(0), output: r["usage"]["output_tokens"].as_u64().unwrap_or(0) };
            Step::Done(Ok(()))
        }
        "response.failed" => {
            let e = &ev["response"]["error"];
            let body = json!({ "error": e });
            Step::Done(Err(classify(400, &body.to_string(), "")))
        }
        "response.incomplete" => Step::Done(Err(ApiError::Incomplete(ev["response"]["incomplete_details"]["reason"].as_str().unwrap_or("incomplete").to_string()))),
        "error" => {
            let body = json!({ "error": { "code": ev["code"].as_str().or_else(|| ev["error"]["code"].as_str()), "message": ev["message"].as_str().or_else(|| ev["error"]["message"].as_str()) } });
            Step::Done(Err(classify(400, &body.to_string(), "")))
        }
        _ => Step::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_recovery() {
        assert_eq!(classify(429, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"x"}}"#, ""), ApiError::UsageLimit);
        assert_eq!(classify(403, r#"{"error":{"code":"subscription_sharing_user_not_eligible"}}"#, ""), ApiError::NotEligible);
        assert!(classify(503, r#"{"error":{"code":"subscription_sharing_usage_unavailable"}}"#, "").transient());
        assert_eq!(classify(400, r#"{"error":{"code":"subscription_sharing_unsupported_capability","param":"tools[0]"}}"#, ""), ApiError::Unsupported("tools[0]".into()));
        // Direct admission bodies aren't standard error objects.
        assert_eq!(classify(401, r#"{"detail":"Unauthorized"}"#, ""), ApiError::Unauthorized("Unauthorized".into()));
        assert_eq!(classify(403, r#"{"detail":"region"}"#, ""), ApiError::Forbidden("region".into()));
        assert!(classify(503, r#"{"detail":"not enabled"}"#, "").transient());
        assert!(matches!(classify(418, "teapot", "req_1"), ApiError::Http { status: 418, request_id, .. } if request_id == "req_1"));
        assert!(!ApiError::UsageLimit.transient() && !ApiError::NotEligible.transient());
    }

    #[test]
    fn model_lists() {
        let plan = json!({ "models": [
            { "slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list" },
            { "slug": "gpt-6-luna", "display_name": "GPT-6 Luna", "visibility": "list" },
            { "slug": "internal", "display_name": "Hidden", "visibility": "hide" },
            { "slug": "no-tools", "display_name": "No tools", "visibility": "list", "supports_function_calling": false },
        ] });
        let m = parse_models(&plan, BillingKind::ChatGptPlan);
        assert_eq!(m.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(), vec!["gpt-6.1-sol", "gpt-6-luna"], "server order, listed and tool-capable only");
        assert_eq!(choose_model(&m, ""), Some("gpt-6-luna".into()), "Luna preferred");
        assert_eq!(choose_model(&m, "gpt-6.1-sol"), Some("gpt-6.1-sol".into()), "the user's choice is kept");
        assert_eq!(choose_model(&m, "gpt-5-gone"), None, "a vanished choice isn't silently replaced");
        assert_eq!(choose_model(&[Model { slug: "gpt-6.1-sol".into(), name: "Sol".into() }], ""), None, "no silent pick of a pricier model");
        let api = json!({ "data": [{ "id": "gpt-6-luna" }, { "id": "gpt-4o-audio-preview" }, { "id": "text-embedding-3-small" }, { "id": "gpt-6-astra" }] });
        assert_eq!(parse_models(&api, BillingKind::ApiKey).iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(), vec!["gpt-6-luna", "gpt-6-astra"]);
    }

    fn feed(chunks: &[&str]) -> (Completed, Result<(), ApiError>, String) {
        let mut sse = Sse::default();
        let mut out = Completed::default();
        let mut seen = String::new();
        for c in chunks {
            for ev in sse.push(c.as_bytes()) {
                if let Step::Done(r) = handle_event(&ev, &mut out, MAX_TEXT, &mut |t: &str| seen.push_str(t)) {
                    return (out, r, seen);
                }
            }
        }
        (out, Err(ApiError::Interrupted), seen)
    }

    #[test]
    fn streams_text_tool_calls_and_usage() {
        let (out, r, seen) = feed(&[
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"search_library\",\"arguments\":\"{}\"}}\n",
            "\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[],\"usage\":{\"input_tokens\":120,\"output_tokens\":7}}}\n\n",
        ]);
        assert_eq!(r, Ok(()));
        assert_eq!((out.text.as_str(), seen.as_str()), ("Hello", "Hello"));
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.usage, Usage { input: 120, output: 7 });
    }

    #[test]
    fn failures_inside_the_stream() {
        let (_, r, _) = feed(&[
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n\n",
        ]);
        assert_eq!(r, Err(ApiError::UsageLimit), "a limit can arrive after streaming began");
        let (_, r, _) = feed(&["data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n"]);
        assert_eq!(r, Err(ApiError::Incomplete("max_output_tokens".into())));
        let (_, r, _) = feed(&["data: {\"type\":\"response.output_text.delta\",\"delta\":\"cut\"}\n\n"]);
        assert_eq!(r, Err(ApiError::Interrupted), "no response.completed = not a success");
        let (out, r, _) = feed(&["data: {not json\n\n", "data: {\"type\":\"response.completed\",\"response\":{}}\n\n"]);
        assert!(r.is_ok() && out.text.is_empty(), "a malformed event is skipped");
    }

    #[test]
    fn reply_text_is_capped() {
        let long = "x".repeat(MAX_TEXT + 500);
        let ev = format!("data: {}\n\n", json!({ "type": "response.output_text.delta", "delta": long }));
        let (out, _, _) = feed(&[&ev, "data: {\"type\":\"response.completed\",\"response\":{}}\n\n"]);
        assert!(out.text.len() == MAX_TEXT && out.truncated);
    }

    #[test]
    fn estimates_are_only_for_known_prices() {
        assert!((estimate_usd("gpt-6-luna", Usage { input: 1_000_000, output: 1_000_000 }).unwrap() - 0.60).abs() < 1e-9);
        assert_eq!(estimate_usd("gpt-6.1-sol", Usage::default()), None);
    }
}
