//! The optional music assistant. It does nothing until the user signs in with
//! ChatGPT or adds an OpenAI API key, and can be turned off in Settings.
//!
//! - `oauth`, `account`, `secrets`: Sign in with ChatGPT (open-source flow),
//!   tokens in the OS credential store.
//! - `openai`: model list and streamed Responses requests.
//! - `tools`, `spotify`, `control`: what the model may look at or do — local
//!   files and Glass's local playlists; the user's Spotify library, history,
//!   search and queue (only while they allow it); and playback controls.
//!   Every ID is checked, and the app carries out and confirms each action.
//! - `agent`: one bounded turn of conversation and tool calls.
//!
//! This module holds the panel's state; `App` runs the actions.

pub mod account;
pub mod agent;
pub mod control;
pub mod oauth;
pub mod openai;
pub mod picks;
pub mod secrets;
pub mod spotify;
pub mod tools;

use account::{Accounts, Auth, Endpoints};
use openai::{Model, Usage};
use secrets::SecretStore;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use tools::{Approval, Draft};

/// How assistant requests are paid for. Never switched automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Billing {
    ChatGptPlan,
    ApiKey,
}

impl Billing {
    pub fn name(self) -> &'static str {
        match self {
            Billing::ChatGptPlan => "chatgpt",
            Billing::ApiKey => "api_key",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chatgpt" => Some(Billing::ChatGptPlan),
            "api_key" => Some(Billing::ApiKey),
            _ => None,
        }
    }
    pub fn kind(self) -> openai::BillingKind {
        match self {
            Billing::ChatGptPlan => openai::BillingKind::ChatGptPlan,
            Billing::ApiKey => openai::BillingKind::ApiKey,
        }
    }
}

/// What the data boundary is, in the user's words (Settings and the panel).
pub fn data_note(spotify: bool) -> &'static str {
    if spotify {
        "When you ask the assistant something, Glass sends OpenAI your message, a one-line note of what's playing and which page is open, and only what the assistant looks up for it: matching songs, albums, artists and playlists from your Spotify library, listening history and Spotify search (names, artists, lengths, IDs), the tags of matching local files, and your Glass local playlists. It never sends audio, file paths or your Spotify login. If the assistant translates lyrics (Settings → Lyrics → Translator), it gets the lyrics of that song. Requests are sent with storage off (store: false). Names and tags are treated as data, never as instructions."
    } else {
        "When you ask the assistant something, Glass sends OpenAI your message, a one-line note of what's playing and which page is open, and only what the assistant looks up for it: the tags of matching local files (title, artist, album, genre, year, length) and your Glass local playlists. With Spotify data off it sends nothing from Spotify. It never sends audio or file paths. If the assistant translates lyrics (Settings → Lyrics → Translator), it gets the lyrics of that song. Requests are sent with storage off (store: false). Tags are treated as data, never as instructions."
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    User(String),
    Assistant(String),
    /// From Glass, not the model: saves, approvals, undo.
    Note(String),
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Models {
    Unknown,
    Loading,
    Ready(Vec<Model>),
    Failed(String),
}

pub struct Running {
    pub task: tokio::task::AbortHandle,
    pub status: String,
}

pub struct Assistant {
    pub accounts: Accounts,
    pub auth: Arc<Auth>,
    pub models: Models,
    /// Which billing/account the model list was fetched for.
    pub models_for: String,
    pub transcript: Vec<Line>,
    /// Messages sent with the next request (bounded in `agent`).
    pub history: Vec<Value>,
    pub drafts: Vec<Draft>,
    /// Recommendation cards, oldest first.
    pub picks: Vec<tools::Picks>,
    /// Spotify items the model has been shown: what it may use, and how the
    /// panel names Spotify songs in previews.
    pub catalog: spotify::Catalog,
    /// Every song in the user's playlists and Liked Songs (read once, reused).
    pub library: spotify::LibraryCache,
    /// Drafts the user edited since the model last saw them.
    pub edited: HashSet<String>,
    pub approvals: Vec<Approval>,
    pub next_id: u32,
    /// Bumped per request; answers from older requests are dropped.
    pub generation: u64,
    pub running: Option<Running>,
    pub input: String,
    pub last_request: Option<String>,
    /// A message waiting for the model list.
    pub pending: Option<String>,
    pub usage: Usage,
    pub last_usage: Usage,
    /// The ChatGPT plan's (or this app's) limit was reached: pause plan requests.
    pub limit_reached: bool,
    pub signing_in: Option<tokio::task::AbortHandle>,
    /// An account problem to show (e.g. "sign in again").
    pub account_note: Option<String>,
    /// The API key field in Settings (cleared once saved; never logged).
    pub key_field: String,
    /// Whether an API key is saved, read from the store the first time it matters:
    /// only when paying with a key (on Linux, opening the keyring can ask to unlock it).
    api_key_saved: std::cell::OnceCell<bool>,
    pub focus_input: bool,
}

impl Assistant {
    pub fn new(store: Arc<dyn SecretStore>, endpoints: Endpoints, accounts_dir: std::path::PathBuf) -> Self {
        Self {
            accounts: Accounts::load(accounts_dir),
            auth: Arc::new(Auth::new(store, endpoints)),
            models: Models::Unknown,
            models_for: String::new(),
            transcript: Vec::new(),
            history: Vec::new(),
            drafts: Vec::new(),
            picks: Vec::new(),
            catalog: spotify::Catalog::default(),
            library: spotify::LibraryCache::default(),
            edited: HashSet::new(),
            approvals: Vec::new(),
            next_id: 0,
            generation: 0,
            running: None,
            input: String::new(),
            last_request: None,
            pending: None,
            usage: Usage::default(),
            last_usage: Usage::default(),
            limit_reached: false,
            signing_in: None,
            account_note: None,
            key_field: String::new(),
            api_key_saved: std::cell::OnceCell::new(),
            focus_input: false,
        }
    }

    /// The real assistant: OS keyring, documented endpoints.
    pub fn load() -> Self {
        Self::new(Arc::new(secrets::Keyring), Endpoints::default(), Accounts::dir())
    }

    /// Glass's own line in the panel. During a request it goes above the
    /// reply that's still to come.
    pub fn note(&mut self, text: impl Into<String>) {
        let line = Line::Note(text.into());
        match self.transcript.last() {
            Some(Line::Assistant(t)) if t.is_empty() && self.running.is_some() => {
                let at = self.transcript.len() - 1;
                self.transcript.insert(at, line);
            }
            _ => self.transcript.push(line),
        }
    }

    /// Glass's own note for the model's next request (approvals, saves, edits).
    /// Marked as coming from the app so it can't be confused with file tags.
    pub fn tell_model(&mut self, text: impl Into<String>) {
        self.history.push(json!({ "role": "developer", "content": format!("[Glass app note] {}", text.into()) }));
        self.trim_history();
    }

    /// Keeps the conversation the model sees, the last `MAX_HISTORY` items (all
    /// that's ever sent), so a long session doesn't hold every turn in memory.
    pub fn trim_history(&mut self) {
        let extra = self.history.len().saturating_sub(agent::MAX_HISTORY);
        self.history.drain(..extra);
    }

    pub fn draft(&self, id: &str) -> Option<&Draft> {
        self.drafts.iter().find(|d| d.id == id)
    }

    /// A turn's change to a preview. A turn works on a copy taken when it started, so a
    /// preview saved (or being saved) since then stays as it is: it can't be saved twice.
    pub fn upsert_draft(&mut self, d: Draft) {
        use tools::DraftStatus::{AwaitingApproval, Saved, Saving};
        if self.draft(&d.id).is_some_and(|x| matches!(x.status, Saving | Saved(_)) && !matches!(d.status, Saving | Saved(_))) {
            return;
        }
        self.approvals.retain(|a| !matches!(&a.kind, tools::ApprovalKind::SaveDraft { draft_id, .. } if draft_id == &d.id && d.status != AwaitingApproval));
        match self.drafts.iter_mut().find(|x| x.id == d.id) {
            Some(x) => *x = d,
            None => self.drafts.push(d),
        }
    }

    /// Starts a fresh conversation. Unsaved previews stay, those that awaited approval back
    /// to editing (approvals are cleared); saved ones go (they stay saved).
    pub fn clear(&mut self) {
        use tools::DraftStatus::{AwaitingApproval, Editing, Saving};
        self.transcript.clear();
        self.history.clear();
        self.drafts.retain_mut(|d| {
            if d.status == AwaitingApproval {
                d.status = Editing;
            }
            matches!(d.status, Editing | Saving)
        });
        self.picks.clear();
        let keep: HashSet<String> = self.drafts.iter().flat_map(|d| d.tracks.iter().cloned()).collect();
        self.catalog.retain(&keep);
        self.approvals.clear();
        self.edited.clear();
        self.last_request = None;
    }

    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    pub fn has_api_key(&self) -> bool {
        *self.api_key_saved.get_or_init(|| self.auth.store.get(secrets::API_KEY_KEY).ok().flatten().is_some_and(|k| !k.is_empty()))
    }

    /// The key was just saved or removed.
    pub fn set_has_api_key(&mut self, saved: bool) {
        self.api_key_saved = std::cell::OnceCell::from(saved);
    }
}
