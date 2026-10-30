//! The assistant's actions on `App`: sending requests, carrying out what
//! comes back (only after validation) and reporting what really happened,
//! approvals, saving with undo, sign-in and models. Kept apart from playback
//! so the player never waits on the AI. Actions happen in the background:
//! nothing navigates or opens on screen because the assistant did something.

use crate::ai::agent::{self, Event, TurnError, TurnResult};
use crate::ai::oauth::{AuthError, Client as OAuthClient, MANAGE_USAGE_URL};
use crate::ai::openai::{self, ApiError, Model};
use crate::ai::spotify::{Cached, Item, Source};
use crate::ai::tools::{self, AppOp, ApprovalKind, Control, Dest, Draft, DraftStatus, QueueMode, Snapshot};
use crate::ai::{Billing, Line, Models, Running, secrets};
use crate::api::enc;
use crate::app::{App, Load, Modal, Msg, RightPanel, Route};
use crate::local::LocalTrack;
use crate::model::s;
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// A pending credential (OAuth access token or API key).
type BearerFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, AuthError>> + Send>>;

/// What a request reports back to the UI thread.
pub enum AiEvent {
    Turn(Event),
    Done(Result<TurnResult, TurnError>),
}

/// Edits the user makes to a preview.
#[derive(Clone, Debug)]
pub enum DraftEdit {
    Remove(usize),
    Move(usize, isize),
    Rename(String),
}

/// The last change the assistant made that can be undone.
pub enum AiUndo {
    Queue(Vec<PathBuf>),
    Playlist,
    /// A Spotify playlist it made (`created`), or the songs it added to one.
    SpotifyPlaylist {
        id: String,
        name: String,
        created: bool,
        added: Vec<String>,
    },
    /// Songs it added to Liked Songs (only ones that weren't liked before).
    Liked(Vec<String>),
}

/// The app's answer to an action the model asked for.
type Reply = tokio::sync::oneshot::Sender<Result<Value, String>>;

/// What a change came to, reported back to the UI thread.
pub struct Wrote {
    /// Shown in the panel (and answered to the model).
    pub result: Result<String, String>,
    /// No note in the panel (playback controls: the reply says it).
    pub quiet: bool,
    /// Tell the model next time (the change happened between requests).
    pub tell: bool,
    /// Replaces the undoable change (`Some(None)` clears it).
    pub undo: Option<Option<AiUndo>>,
    pub invalidate: Vec<String>,
    /// Liked Songs hearts to update.
    pub saved: Vec<(String, bool)>,
    pub refresh_player: bool,
    /// A Spotify preview: saved as this playlist, or (None) back to editing.
    pub draft: Option<(String, Option<String>)>,
    /// Recommendations now in a playlist: (card, songs, playlist), and ones
    /// taken back out by an undo: (songs, playlist).
    pub picked: Option<(String, Vec<String>, String)>,
    pub unpicked: Option<(Vec<String>, String)>,
}

impl Wrote {
    fn ok(text: impl Into<String>) -> Self {
        Self { result: Ok(text.into()), quiet: false, tell: false, undo: None, invalidate: Vec::new(), saved: Vec::new(), refresh_player: false, draft: None, picked: None, unpicked: None }
    }
    fn err(text: impl Into<String>) -> Self {
        Self { result: Err(text.into()), ..Self::ok("") }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Spotify's refusals, in words.
fn player_error(e: &str) -> String {
    if e.contains("403") {
        "Spotify refused (controlling playback needs Premium)".into()
    } else if e.contains("404") && e.contains("player") {
        "nothing is playing on Spotify and no device is active".into()
    } else {
        e.to_string()
    }
}

impl App {
    pub fn ai_enabled(&self) -> bool {
        self.config.assistant().enabled
    }

    pub fn ai_billing(&self) -> Billing {
        Billing::parse(&self.config.assistant().billing).unwrap_or(Billing::ChatGptPlan)
    }

    /// Whether requests can be made right now, or why not.
    pub fn ai_ready(&self) -> Result<(), String> {
        if !self.ai_enabled() {
            return Err("The assistant is off. Turn it on in Settings → Assistant.".into());
        }
        match self.ai_billing() {
            Billing::ChatGptPlan => match self.ai.accounts.active() {
                None => Err("Sign in with ChatGPT in Settings → Assistant to use your ChatGPT plan.".into()),
                Some(r) if !r.plan_enabled => Err("You're signed in, but permission to use your ChatGPT plan wasn't granted. Enable it in Settings → Assistant, or use your own API key.".into()),
                Some(_) if self.ai.limit_reached => Err("Usage limit reached for your ChatGPT plan or this app.".into()),
                Some(_) => Ok(()),
            },
            Billing::ApiKey if !self.ai.has_api_key() => Err("Add your OpenAI API key in Settings → Assistant.".into()),
            Billing::ApiKey => Ok(()),
        }
    }

    /// Which credentials the model list belongs to.
    fn ai_models_key(&self) -> Option<String> {
        match self.ai_billing() {
            Billing::ChatGptPlan => self.ai.accounts.active().filter(|r| r.plan_enabled).map(|r| format!("chatgpt:{}", r.client_id)),
            Billing::ApiKey => self.ai.has_api_key().then(|| "api_key".to_string()),
        }
    }

    /// The model requests will use: the saved choice if this account offers it;
    /// with none saved, GPT-6 Luna if offered; otherwise none (the user picks).
    pub fn ai_model(&self) -> Option<String> {
        match &self.ai.models {
            Models::Ready(list) => openai::choose_model(list, &self.config.assistant().model),
            _ => None,
        }
    }

    /// A closure giving a fresh credential for each request: the plan's OAuth
    /// token (refreshed as needed) or the user's API key. Never both.
    fn ai_bearer(&self) -> Result<impl Fn() -> BearerFuture + Send + Sync + Clone + 'static, String> {
        let auth = self.ai.auth.clone();
        // `None`: the API key, read from the keychain with each request, off the UI thread.
        let client_id = match self.ai_billing() {
            Billing::ChatGptPlan => Some(self.ai.accounts.active().map(|r| r.client_id.clone()).ok_or("Not signed in with ChatGPT.")?),
            Billing::ApiKey if self.ai.has_api_key() => None,
            Billing::ApiKey => return Err("No API key saved.".into()),
        };
        Ok(move || {
            let (auth, client_id) = (auth.clone(), client_id.clone());
            Box::pin(async move {
                match client_id {
                    Some(id) => auth.access_token(&id).await,
                    None => {
                        let store = auth.store.clone();
                        let key = tokio::task::spawn_blocking(move || store.get(secrets::API_KEY_KEY)).await.map_err(|e| AuthError::Keyring(format!("Reading the API key failed ({e}).")))?;
                        key.map_err(AuthError::Keyring)?.filter(|k| !k.is_empty()).ok_or(AuthError::NoApiKey)
                    }
                }
            }) as BearerFuture
        })
    }

    /// A credential source for translating lyrics with the assistant's model.
    pub(crate) fn ai_bearer_for_translation(&self) -> Option<impl Fn() -> BearerFuture + Send + Sync + Clone + 'static> {
        self.ai_bearer().ok()
    }

    pub(crate) fn ai_client_for_translation(&self) -> openai::Client {
        self.ai_client()
    }

    fn ai_client(&self) -> openai::Client {
        openai::Client::new(&self.ai.auth.endpoints.api, Some(self.log.clone()))
    }

    /// Once per frame: keep the model list current while the assistant's panel or its
    /// settings are on screen; with the panel open, read the user's playlists ahead of
    /// the first question (it takes a moment).
    pub fn ai_tick(&mut self, panel: bool, settings: bool) {
        if !(panel || settings) || !self.ai_enabled() {
            return;
        }
        if panel
            && self.ai.library.should_prefetch()
            && let Some(src) = self.ai_source()
        {
            let lib = self.ai.library.clone();
            self.rt.spawn(async move {
                if let Err(e) = lib.get(src.as_ref()).await {
                    log::warn!("assistant library index: {e}");
                }
            });
        }
        let Some(key) = self.ai_models_key() else { return };
        if self.ai.models_for != key && self.ai.models != Models::Loading {
            self.ai_fetch_models();
        }
    }

    pub fn ai_fetch_models(&mut self) {
        let Some(key) = self.ai_models_key() else { return };
        let bearer = match self.ai_bearer() {
            Ok(b) => b,
            Err(e) => {
                self.ai.models = Models::Failed(e);
                self.ai.models_for = key;
                return;
            }
        };
        self.ai.models = Models::Loading;
        self.ai.models_for = key.clone();
        let (client, kind, tx, ctx) = (self.ai_client(), self.ai_billing().kind(), self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = match bearer().await {
                Ok(t) => client.models(&t, kind).await.map_err(|e| e.message(kind)),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(Msg::AiModels { key, result });
            ctx.request_repaint();
        });
    }

    pub fn ai_models_arrived(&mut self, key: String, result: Result<Vec<Model>, String>) {
        if Some(&key) != self.ai_models_key().as_ref() {
            return;
        }
        self.ai.models = match result {
            Ok(list) => Models::Ready(list),
            Err(e) => Models::Failed(e),
        };
        if let Some(text) = self.ai.pending.take() {
            self.ai_send(text);
        }
    }

    /// What the tools see of the library, made by the returned closure: it copies
    /// every local song, so it runs on the request's task, not the UI thread.
    fn ai_snapshot(&self) -> impl FnOnce() -> Snapshot + Send + 'static {
        let local = self.local_tracks.clone().unwrap_or_default();
        let queue = self.local.as_ref().map(|l| l.upcoming()).unwrap_or_default();
        let (playlists, drafts, next_id) = (self.playlists.list.clone(), self.ai.drafts.clone(), self.ai.next_id);
        let (catalog, library, me) = (self.ai.catalog.clone(), self.ai.library.clone(), self.user_id());
        move || {
            let mut s = Snapshot::new(&local, &playlists, &queue, drafts, next_id);
            s.catalog = catalog;
            s.library = library;
            s.me = me;
            s
        }
    }

    /// Sends a message. Any request still running is cancelled first.
    pub fn ai_send(&mut self, text: String) {
        let text = text.trim().chars().take(2_000).collect::<String>();
        if text.is_empty() {
            return;
        }
        if let Err(why) = self.ai_ready() {
            self.ai_refuse(why);
            return;
        }
        // Asked before the model list arrived (e.g. from the command bar with
        // the panel closed): send once it's here.
        if self.ai_models_key().is_some_and(|k| k != self.ai.models_for) || matches!(self.ai.models, Models::Unknown | Models::Loading) {
            if self.ai.models != Models::Loading {
                self.ai_fetch_models();
            }
            if self.ai.models == Models::Loading {
                self.ai.pending = Some(text);
                return;
            }
        }
        let Some(model) = self.ai_model() else {
            let why: String = match &self.ai.models {
                Models::Ready(_) if !self.config.assistant().model.is_empty() => "The model you chose isn't offered for this account any more. Pick one below.".into(),
                Models::Ready(_) => "Choose a model below first.".into(),
                Models::Failed(e) => format!("Couldn't list models: {e}"),
                _ => "Still loading the model list…".into(),
            };
            self.ai_refuse(why);
            return;
        };
        let local = !self.settings.local_dirs.is_empty();
        if local && self.local_tracks.is_none() {
            self.scan_local();
        }
        let spotify = self.ai_source();
        if spotify.is_none() && !local {
            self.ai_refuse(if self.config.assistant().spotify {
                "Sign in to Spotify, or add a folder of music under Local Files, so the assistant has music to work with.".into()
            } else {
                "Spotify data is off for the assistant and there's no local music folder. Turn Spotify data on in Settings → Assistant, or add a folder under Local Files.".into()
            });
            return;
        }
        let bearer = match self.ai_bearer() {
            Ok(b) => b,
            Err(e) => {
                self.ai_refuse(e);
                return;
            }
        };
        self.ai_cancel_quiet();
        // Tell the model about previews the user edited by hand.
        let edited: Vec<String> = self.ai.edited.drain().collect();
        for id in edited {
            if let Some(d) = self.ai.draft(&id).cloned() {
                self.ai.tell_model(format!("The user edited preview {} (now named “{}”). Its songs, in order: {}", d.id, d.name, d.tracks.join(", ")));
            }
        }
        self.ai.transcript.push(Line::User(text.clone()));
        self.ai.transcript.push(Line::Assistant(String::new()));
        self.ai.last_request = Some(text.clone());
        self.ai.generation += 1;
        let generation = self.ai.generation;
        let context = self.ai_context(spotify.is_some());
        let (kind, history, snapshot) = (self.ai_billing().kind(), self.ai.history.clone(), self.ai_snapshot());
        let (client, tx, ctx) = (self.ai_client(), self.tx.clone(), self.ctx.clone());
        let task = self.rt.spawn(async move {
            let req = agent::Request { model, kind, history, user: text, snapshot: snapshot(), spotify, local, context };
            let send = |e: AiEvent| {
                let _ = tx.send(Msg::Ai { generation, event: e });
                ctx.request_repaint();
            };
            let result = agent::run(&client, bearer, req, |e| send(AiEvent::Turn(e))).await;
            send(AiEvent::Done(result));
        });
        self.ai.running = Some(Running { task: task.abort_handle(), status: "thinking…".into() });
    }

    /// A request that can't be sent: why, in the panel (and as a toast if it's hidden,
    /// as when asked from the command bar).
    fn ai_refuse(&mut self, why: String) {
        if !self.ai_panel_visible() {
            self.toast(format!("Assistant: {why}"));
        }
        self.ai.transcript.push(Line::Error(why));
    }

    fn ai_cancel_quiet(&mut self) {
        if let Some(r) = self.ai.running.take() {
            r.task.abort();
            self.ai.generation += 1;
        }
    }

    /// Stops the request; its late answers are ignored.
    pub fn ai_cancel(&mut self) {
        self.ai.pending = None;
        if self.ai.running.is_some() {
            self.ai_cancel_quiet();
            if let Some(Line::Assistant(t)) = self.ai.transcript.last()
                && t.is_empty()
            {
                self.ai.transcript.pop();
            }
            self.ai.note("Stopped.");
        }
    }

    pub fn ai_retry(&mut self) {
        let Some(text) = self.ai.last_request.clone() else { return };
        // Drop the failed exchange from view; saved playlists are protected by their draft IDs.
        while matches!(self.ai.transcript.last(), Some(Line::Error(_)) | Some(Line::Assistant(_)) | Some(Line::Note(_))) {
            self.ai.transcript.pop();
        }
        if matches!(self.ai.transcript.last(), Some(Line::User(u)) if *u == text) {
            self.ai.transcript.pop();
        }
        self.ai_send(text);
    }

    /// An event from request `generation`. Stale ones are dropped (and any
    /// action they asked for is refused).
    pub fn ai_event(&mut self, generation: u64, event: AiEvent) {
        if generation != self.ai.generation {
            match event {
                AiEvent::Turn(Event::Queue { reply, .. }) => {
                    let _ = reply.send(Err("cancelled".into()));
                }
                AiEvent::Turn(Event::Act { reply, .. }) => {
                    let _ = reply.send(Err("cancelled".into()));
                }
                _ => {}
            }
            return;
        }
        match event {
            AiEvent::Turn(Event::Text(t)) => match self.ai.transcript.last_mut() {
                Some(Line::Assistant(s)) => s.push_str(&t),
                _ => self.ai.transcript.push(Line::Assistant(t)),
            },
            AiEvent::Turn(Event::Retrying(why)) => {
                if let Some(Line::Assistant(s)) = self.ai.transcript.last_mut() {
                    s.clear();
                }
                if let Some(r) = self.ai.running.as_mut() {
                    r.status = if why.is_empty() { "retrying…".into() } else { format!("retrying — {why}") };
                }
            }
            AiEvent::Turn(Event::Tool(name)) => {
                if let Some(r) = self.ai.running.as_mut() {
                    r.status = match name.rsplit('.').next().unwrap_or(&name) {
                        "search_library" => "searching your library…",
                        "library_overview" => "looking over your library…",
                        "list_playlists" | "get_playlist" | "my_spotify_playlists" | "spotify_playlist" => "reading your playlists…",
                        "propose_playlist" | "propose_spotify_playlist" | "revise_draft" => "building a preview…",
                        "queue_tracks" | "get_queue" | "spotify_queue" => "checking the queue…",
                        "queue_spotify" => "queueing…",
                        "play_spotify" => "starting playback…",
                        "like_songs" => "liking…",
                        "search_spotify" => "searching Spotify…",
                        "liked_songs" => "looking through your Liked Songs…",
                        "spotify_artist" | "spotify_album" => "looking through the catalog…",
                        "listening_history" | "check_heard" | "my_taste" => "checking what you listen to…",
                        "unheard_songs" => "looking for songs you don't have…",
                        "recommend_songs" => "picking songs…",
                        "now_playing" | "player" => "working the player…",
                        _ => "working…",
                    }
                    .into();
                }
            }
            AiEvent::Turn(Event::Draft(d)) => {
                self.ai.next_id = self.ai.next_id.max(d.id.trim_start_matches('d').parse().unwrap_or(0));
                self.ai.upsert_draft(d);
            }
            AiEvent::Turn(Event::Picks(p)) => {
                self.ai.next_id = self.ai.next_id.max(p.id.trim_start_matches('r').parse().unwrap_or(0));
                self.ai.picks.push(p);
            }
            AiEvent::Turn(Event::Approval(a)) => {
                self.ai.next_id = self.ai.next_id.max(a.id.trim_start_matches('a').parse().unwrap_or(0));
                if let ApprovalKind::SaveDraft { draft_id, .. } = &a.kind
                    && let Some(d) = self.ai.drafts.iter_mut().find(|d| &d.id == draft_id)
                {
                    // Saved since the turn began: nothing left to approve.
                    if matches!(d.status, DraftStatus::Saved(_) | DraftStatus::Saving) {
                        return;
                    }
                    d.status = DraftStatus::AwaitingApproval;
                }
                self.ai.approvals.retain(|x| x.kind != a.kind);
                self.ai.approvals.push(a);
            }
            AiEvent::Turn(Event::Queue { paths, mode, reply }) => {
                let r = self.ai_queue(paths, mode);
                let _ = reply.send(r);
            }
            AiEvent::Turn(Event::Act { op, reply }) => self.ai_act(op, Some(reply)),
            AiEvent::Done(Ok(r)) => {
                self.ai.running = None;
                self.ai.next_id = self.ai.next_id.max(r.snapshot.next_id);
                self.ai.history.extend(r.keep);
                self.ai.trim_history();
                self.ai.usage.input += r.usage.input;
                self.ai.usage.output += r.usage.output;
                self.ai.last_usage = r.usage;
                if let Some(Line::Assistant(s)) = self.ai.transcript.last_mut() {
                    // The final text is authoritative (retries may have reset the stream).
                    *s = r.text;
                }
                if r.truncated {
                    self.ai.note("The reply was cut off at the length limit.");
                }
                // Asked from the command bar with the panel closed: the answer as a toast.
                if !self.ai_panel_visible() {
                    let t = self.ai.transcript.iter().rev().find_map(|l| if let Line::Assistant(t) = l { Some(t.clone()) } else { None }).unwrap_or_default();
                    if !t.is_empty() {
                        self.toast(if t.chars().count() > 180 { format!("{}… ({} for more)", t.chars().take(180).collect::<String>(), crate::ui::widgets::keys("⌘ J")) } else { t });
                    }
                }
            }
            AiEvent::Done(Err(e)) => {
                self.ai.running = None;
                if let Some(Line::Assistant(s)) = self.ai.transcript.last()
                    && s.is_empty()
                {
                    self.ai.transcript.pop();
                }
                let kind = self.ai_billing().kind();
                let msg = match e {
                    TurnError::Api(ApiError::UsageLimit) => {
                        self.ai.limit_reached = true;
                        ApiError::UsageLimit.message(kind)
                    }
                    TurnError::Api(e) => e.message(kind),
                    TurnError::Auth(AuthError::SignInAgain(why)) => {
                        self.ai.account_note = Some(format!("Your ChatGPT sign-in ended ({why}). Sign in again in Settings → Assistant."));
                        if let Some(r) = self.ai.accounts.active().cloned() {
                            self.ai.accounts.upsert(crate::ai::account::Registration { plan_enabled: false, ..r });
                        }
                        format!("Your ChatGPT sign-in ended ({why}). Sign in again in Settings → Assistant.")
                    }
                    TurnError::Auth(AuthError::Declined) => "Permission to use your ChatGPT plan isn't granted. Enable it in Settings → Assistant.".into(),
                    TurnError::Auth(e) => e.to_string(),
                    TurnError::TooManySteps => "The assistant took too many steps without finishing, so Glass stopped it. Try a more specific request.".into(),
                };
                if !self.ai_panel_visible() {
                    self.toast(format!("Assistant: {msg}"));
                }
                self.ai.transcript.push(Line::Error(msg));
            }
        }
    }

    /// Track IDs → files (from the current library).
    fn ai_paths(&self, ids: &[String]) -> (Vec<PathBuf>, usize) {
        let all = self.local_tracks.as_deref().map(Vec::as_slice).unwrap_or_default();
        let map: HashMap<String, &PathBuf> = tools::track_ids(all).into_iter().zip(all.iter().map(|t| &t.path)).collect();
        let paths: Vec<PathBuf> = ids.iter().filter_map(|i| map.get(i).map(|p| (*p).clone())).collect();
        let missing = ids.len() - paths.len();
        (paths, missing)
    }

    fn tracks_for(&self, paths: &[PathBuf]) -> Vec<LocalTrack> {
        let all = self.local_tracks.as_deref().map(Vec::as_slice).unwrap_or_default();
        paths.iter().filter_map(|p| all.iter().find(|t| &t.path == p).cloned()).collect()
    }

    /// Queues (or plays) validated files; the result is reported truthfully.
    fn ai_queue(&mut self, paths: Vec<PathBuf>, mode: QueueMode) -> Result<String, String> {
        let tracks = self.tracks_for(&paths);
        if tracks.is_empty() {
            return Err("those files aren't in the library any more".into());
        }
        let n = tracks.len();
        let s = if n == 1 { "" } else { "s" };
        match mode {
            QueueMode::PlayNow => {
                if !self.play_local_list(tracks, 0) {
                    return Err("no audio output is available".into());
                }
                self.ai_undo = None;
                self.ai.note(format!("Playing {n} song{s}."));
                Ok(format!("now playing {n} song{s}"))
            }
            QueueMode::Next | QueueMode::End => {
                if !self.local_file_active() {
                    return Err("no local file is playing, so there's no queue to add to".into());
                }
                let next = mode == QueueMode::Next;
                let added = self.local.as_mut().map(|l| l.enqueue(tracks, next)).unwrap_or(0);
                self.ai_undo = Some(AiUndo::Queue(paths));
                self.ai.note(format!("Queued {added} song{s} {}.", if next { "next" } else { "at the end" }));
                Ok(format!("queued {added} song{s} {}", if next { "next" } else { "at the end" }))
            }
        }
    }

    pub fn ai_approve(&mut self, approval_id: &str) {
        let Some(i) = self.ai.approvals.iter().position(|a| a.id == approval_id) else { return };
        let a = self.ai.approvals.remove(i);
        match a.kind {
            ApprovalKind::SaveDraft { draft_id, replace } => self.ai_save_draft(&draft_id, replace),
            ApprovalKind::QueueMany { ids, mode } if ids.first().is_some_and(|i| i.starts_with("spotify:")) => match mode {
                QueueMode::PlayNow => self.ai_play_spotify(ids, None, None, None),
                _ => self.ai_queue_spotify(ids, None),
            },
            ApprovalKind::Like { ids } => self.ai_like(ids, None),
            ApprovalKind::QueueMany { ids, mode } => {
                let (paths, _) = self.ai_paths(&ids);
                match self.ai_queue(paths, mode) {
                    Ok(done) => self.ai.tell_model(format!("The user approved; {done}.")),
                    Err(e) => {
                        self.ai.transcript.push(Line::Error(format!("Couldn't queue: {e}")));
                        self.ai.tell_model(format!("The user approved queueing, but it failed: {e}."));
                    }
                }
            }
        }
    }

    pub fn ai_decline(&mut self, approval_id: &str) {
        let Some(i) = self.ai.approvals.iter().position(|a| a.id == approval_id) else { return };
        let a = self.ai.approvals.remove(i);
        let what = match &a.kind {
            ApprovalKind::SaveDraft { draft_id, .. } => {
                if let Some(d) = self.ai.drafts.iter_mut().find(|d| &d.id == draft_id) {
                    d.status = DraftStatus::Editing;
                }
                format!("saving preview {draft_id}")
            }
            ApprovalKind::QueueMany { ids, .. } => format!("queueing {} songs", ids.len()),
            ApprovalKind::Like { ids } => format!("liking {} songs", ids.len()),
        };
        self.ai.note("Declined.");
        self.ai.tell_model(format!("The user declined {what}. Nothing was changed."));
    }

    /// Saves a preview: a new playlist, or replacing `replace`. This is the
    /// user's approval. A second save of the same preview makes no copy.
    pub fn ai_save_draft(&mut self, draft_id: &str, replace: Option<String>) {
        let Some(d) = self.ai.draft(draft_id).cloned() else { return };
        if d.tracks.is_empty() {
            self.ai.transcript.push(Line::Error("The preview has no songs left to save.".into()));
            return;
        }
        if d.dest == Dest::Spotify {
            match &d.status {
                DraftStatus::Saved(_) => self.ai.note("Already saved to Spotify."),
                DraftStatus::Saving => {}
                _ => self.ai_save_spotify_draft(d, replace),
            }
            return;
        }
        if let DraftStatus::Saved(pid) = &d.status {
            self.ai.note(format!("Already saved as “{}”.", self.playlists.get(pid).map(|p| p.name.as_str()).unwrap_or("a playlist")));
            return;
        }
        let (paths, missing) = self.ai_paths(&d.tracks);
        if paths.is_empty() {
            self.ai.transcript.push(Line::Error("None of the preview's songs are in Local Files any more.".into()));
            return;
        }
        let result = match &replace {
            Some(pid) => self.playlists.replace(pid, paths.clone(), Some(&d.name), Some(&d.id)).map(|()| (pid.clone(), false)),
            None => self.playlists.create(&d.name, &d.description, paths.clone(), Some(&d.id)),
        };
        self.ai.approvals.retain(|a| !matches!(&a.kind, ApprovalKind::SaveDraft { draft_id: x, .. } if x == draft_id));
        match result {
            Ok((pid, _)) => {
                if let Some(x) = self.ai.drafts.iter_mut().find(|x| x.id == d.id) {
                    x.status = DraftStatus::Saved(pid.clone());
                }
                let total = crate::model::fmt_ms(self.tracks_for(&paths).iter().map(|t| t.duration_ms).sum());
                let verb = if replace.is_some() { "Replaced" } else { "Saved" };
                let extra = if missing > 0 { format!(" (left out {} no longer in the library)", crate::model::songs(missing as u64)) } else { String::new() };
                self.ai.note(format!("{verb} “{}” — {} · {total}{extra}. Undo is available.", d.name, crate::model::songs(paths.len() as u64)));
                self.ai.tell_model(format!("The user approved. {verb} local playlist “{}” (id {pid}) with {} songs.", d.name, paths.len()));
                self.ai_undo = Some(AiUndo::Playlist);
            }
            Err(e) => {
                self.ai.transcript.push(Line::Error(format!("Couldn't save: {e}")));
                self.ai.tell_model(format!("Saving preview {} failed: {e}. Nothing was saved.", d.id));
            }
        }
    }

    pub fn ai_edit_draft(&mut self, draft_id: &str, edit: DraftEdit) {
        if self.ai.busy() {
            return;
        }
        let Some(d) = self.ai.drafts.iter_mut().find(|d| d.id == draft_id) else { return };
        if matches!(d.status, DraftStatus::Saved(_) | DraftStatus::Saving) {
            return;
        }
        match edit {
            DraftEdit::Remove(i) if i < d.tracks.len() => {
                d.tracks.remove(i);
            }
            DraftEdit::Move(i, by) => {
                let j = i as isize + by;
                if i < d.tracks.len() && j >= 0 && (j as usize) < d.tracks.len() {
                    d.tracks.swap(i, j as usize);
                }
            }
            DraftEdit::Rename(n) if !n.trim().is_empty() => d.name = n.trim().chars().take(120).collect(),
            _ => return,
        }
        d.status = DraftStatus::Editing;
        let id = d.id.clone();
        self.ai.approvals.retain(|a| !matches!(&a.kind, ApprovalKind::SaveDraft { draft_id, .. } if *draft_id == id));
        self.ai.edited.insert(id);
    }

    pub fn ai_discard_draft(&mut self, draft_id: &str) {
        self.ai.drafts.retain(|d| d.id != draft_id);
        self.ai.approvals.retain(|a| !matches!(&a.kind, ApprovalKind::SaveDraft { draft_id: x, .. } if x == draft_id));
        self.ai.tell_model(format!("The user discarded preview {draft_id}."));
    }

    pub fn ai_undo_last(&mut self) {
        match self.ai_undo.take() {
            Some(AiUndo::Queue(paths)) => {
                let n = self.local.as_mut().map(|l| l.dequeue(&paths)).unwrap_or(0);
                self.ai.note(format!("Undone: removed {n} queued song{}.", plural(n)));
                self.ai.tell_model("The user undid the last queue change.");
            }
            Some(AiUndo::Playlist) => match self.playlists.undo() {
                Ok(Some(what)) => {
                    // A restored or removed playlist: its preview is a preview again.
                    for d in &mut self.ai.drafts {
                        if let DraftStatus::Saved(pid) = &d.status
                            && self.playlists.get(pid).is_none_or(|p| p.from_draft.as_deref() != Some(d.id.as_str()))
                        {
                            d.status = DraftStatus::Editing;
                        }
                    }
                    self.ai.note(format!("Undone: {what}."));
                    self.ai.tell_model(format!("The user undid the last playlist change ({what})."));
                }
                Ok(None) => {}
                Err(e) => self.ai.transcript.push(Line::Error(e)),
            },
            Some(AiUndo::SpotifyPlaylist { id, name, created, added }) => {
                let draft = self.ai.drafts.iter().find(|d| d.status == DraftStatus::Saved(id.clone())).map(|d| d.id.clone());
                self.ai_spotify_job(None, move |src| async move {
                    let r = if created {
                        src.send(Method::DELETE, format!("/playlists/{id}/followers"), None).await.map(|_| format!("Undone: removed “{name}” from your Spotify library."))
                    } else {
                        let tracks: Vec<Value> = added.iter().map(|u| json!({ "uri": u })).collect();
                        src.send(Method::DELETE, format!("/playlists/{id}/tracks"), Some(json!({ "tracks": tracks })))
                            .await
                            .map(|_| format!("Undone: took the {} added song{} back out of “{name}”.", added.len(), plural(added.len())))
                    };
                    let unpicked = (!created).then(|| (added.clone(), id.clone()));
                    match r {
                        Ok(t) => Wrote { tell: true, invalidate: vec![id.clone(), "/me/playlists".into()], draft: draft.map(|d| (d, None)), unpicked, ..Wrote::ok(t) },
                        Err(e) => Wrote { tell: true, ..Wrote::err(format!("Couldn't undo: {e}")) },
                    }
                });
            }
            Some(AiUndo::Liked(uris)) => {
                self.ai_spotify_job(None, move |src| async move {
                    let n = uris.len();
                    let mut done = 0;
                    for chunk in uris.chunks(50) {
                        let ids: Vec<&str> = chunk.iter().map(|u| crate::model::id_of(u)).collect();
                        if let Err(e) = src.send(Method::DELETE, format!("/me/tracks?ids={}", ids.join(",")), None).await {
                            if done == 0 {
                                return Wrote { tell: true, ..Wrote::err(format!("Couldn't undo the likes: {e}")) };
                            }
                            let saved = uris[..done].iter().map(|u| (u.clone(), false)).collect();
                            return Wrote { tell: true, invalidate: vec!["/me/tracks".into()], saved, ..Wrote::err(format!("Unliked {done} of {n} songs, then Spotify refused: {e}.")) };
                        }
                        done += chunk.len();
                    }
                    Wrote { tell: true, invalidate: vec!["/me/tracks".into()], saved: uris.into_iter().map(|u| (u, false)).collect(), ..Wrote::ok(format!("Undone: unliked {n} song{}.", plural(n))) }
                });
            }
            None => {}
        }
    }

    // ------------------------------------------------------------ Spotify and the player

    /// The assistant may use the user's Spotify data (its own setting, on by default).
    pub fn ai_spotify_on(&self) -> bool {
        self.config.assistant().spotify
    }

    /// The assistant panel is on screen.
    pub fn ai_panel_visible(&self) -> bool {
        self.settings.right_open && self.right == RightPanel::Assistant && self.route != Route::NowPlaying
    }

    /// Spotify for the assistant, if allowed: the live connection; in tests
    /// a fake; in demo mode what's already loaded (read-only).
    pub(crate) fn ai_source(&self) -> Option<Arc<dyn Source>> {
        if !self.ai_spotify_on() {
            return None;
        }
        self.spotify_source()
    }

    /// Glass's Spotify connection as a data source (lyrics, the assistant).
    pub(crate) fn spotify_source(&self) -> Option<Arc<dyn Source>> {
        if let Some(o) = &self.ai_source_override {
            return Some(o.clone());
        }
        if let Some(api) = &self.api {
            return Some(Arc::new(api.clone()));
        }
        self.me.as_ref()?;
        let data = self.data.iter().filter_map(|(k, v)| if let Load::Ready(v) = v { Some((k.clone(), v.clone())) } else { None }).collect();
        Some(Arc::new(Cached(data)))
    }

    fn ai_cached(&self, key: &str) -> Option<Arc<Value>> {
        match self.data.get(key) {
            Some(Load::Ready(v)) => Some(v.clone()),
            _ => None,
        }
    }

    /// Lets the model use a Spotify song (and its album and artists); returns how to name it.
    fn ai_known_track(&self, t: &Value) -> String {
        let c = &self.ai.catalog;
        let (uri, name, album) = (s(t, "uri"), s(t, "name"), &t["album"]);
        let by = t["artists"].as_array().map(|a| a.iter().map(|x| s(x, "name")).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        c.add(Item {
            uri: uri.into(),
            name: name.into(),
            by: by.clone(),
            album: s(album, "name").into(),
            duration_ms: t["duration_ms"].as_u64().unwrap_or(0),
            preview: t["preview_url"].as_str().map(str::to_string),
        });
        if !s(album, "uri").is_empty() {
            c.add(Item { uri: s(album, "uri").into(), name: s(album, "name").into(), by: by.clone(), ..Default::default() });
        }
        for a in t["artists"].as_array().into_iter().flatten() {
            c.add(Item { uri: s(a, "uri").into(), name: s(a, "name").into(), ..Default::default() });
        }
        if s(album, "uri").is_empty() { format!("“{name}” by {by} ({uri})") } else { format!("“{name}” by {by} ({uri}; album “{}” {})", s(album, "name"), s(album, "uri")) }
    }

    /// Lets the model use a playlist, album or artist; returns how to name it.
    fn ai_known_context(&self, uri: &str) -> String {
        let mut parts = uri.split(':').skip(1);
        let (kind, id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let name = self.ai.catalog.get(uri).map(|i| i.name).filter(|n| !n.is_empty()).or_else(|| self.ai_cached(&format!("/{kind}s/{id}")).map(|v| s(&v, "name").to_string())).unwrap_or_default();
        if !matches!(kind, "playlist" | "album" | "artist") {
            return format!("({uri})");
        }
        if !self.ai.catalog.contains(uri) {
            self.ai.catalog.add(Item { uri: uri.into(), name: name.clone(), ..Default::default() });
        }
        if name.is_empty() { format!("{kind} ({uri})") } else { format!("{kind} “{name}” ({uri})") }
    }

    /// One short line for this request: what's playing, what's open, how
    /// big the local library is. Spotify details only with Spotify data on.
    fn ai_context(&self, spotify: bool) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if self.local_file_active() {
            if let Some(l) = self.local.as_ref()
                && let Some(t) = l.current()
            {
                let state = if l.is_playing() { "playing" } else { "paused" };
                let id = self.local_tracks.as_deref().and_then(|all| tools::id_in(all, &t.path)).unwrap_or_else(|| tools::track_id(&t.path));
                parts.push(format!("Now {state} (local file): “{}” by {} (id {id}).", t.title, t.artist));
            }
        } else if let Some(it) = self.pb.item.as_ref().filter(|i| !s(i, "uri").is_empty()) {
            if spotify {
                let song = self.ai_known_track(it);
                let state = if self.pb.is_playing { "playing" } else { "paused" };
                let device = self.pb.device.as_ref().map(|d| s(d, "name").to_string()).filter(|n| !n.is_empty()).map(|n| format!(" on {n}")).unwrap_or_default();
                let ctx = &self.pb.context_uri;
                let from = if ctx.ends_with(":collection") {
                    ", from Liked Songs".to_string()
                } else if ctx.is_empty() {
                    String::new()
                } else {
                    format!(", from {}", self.ai_known_context(ctx))
                };
                parts.push(format!("Now {state} on Spotify{device}: {song}{from}."));
            } else {
                parts.push("Something is playing on Spotify (Spotify data is off for the assistant, so no details).".into());
            }
        }
        let page = match &self.route {
            Route::Playlist(id) if spotify => Some(self.ai_known_context(&format!("spotify:playlist:{id}"))),
            Route::Album(id) if spotify => Some(self.ai_known_context(&format!("spotify:album:{id}"))),
            Route::Artist(id) if spotify => Some(self.ai_known_context(&format!("spotify:artist:{id}"))),
            Route::LikedSongs if spotify => Some("Liked Songs".into()),
            Route::LocalPlaylist(id) => self.playlists.get(id).map(|p| format!("local playlist “{}” ({})", p.name, p.id)),
            Route::LocalFiles => Some("Local Files".into()),
            _ => None,
        };
        if let Some(p) = page {
            parts.push(format!("Open in Glass: {p}."));
        }
        if !self.settings.local_dirs.is_empty() {
            match &self.local_tracks {
                Some(t) => parts.push(format!("Local library: {} songs.", t.len())),
                None => parts.push("Local library: still being read.".into()),
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// What's playing, for `app.now_playing`.
    fn ai_now_playing(&self, spotify: bool) -> Value {
        if self.local_file_active()
            && let Some(l) = self.local.as_ref()
            && let Some(t) = l.current()
        {
            let id = self.local_tracks.as_deref().and_then(|all| tools::id_in(all, &t.path)).unwrap_or_else(|| tools::track_id(&t.path));
            return json!({
                "source": "local file", "id": id, "title": t.title, "artist": t.artist, "album": t.album,
                "playing": l.is_playing(), "position": format!("{} / {}", crate::model::fmt_ms(l.position_ms()), crate::model::fmt_ms(t.duration_ms)),
                "volume_percent": (self.pb.volume * 100.0).round(),
            });
        }
        let Some(it) = self.pb.item.as_ref().filter(|i| !s(i, "uri").is_empty()) else { return json!({ "playing": null, "note": "Nothing is playing." }) };
        if !spotify {
            return json!({ "source": "spotify", "note": "Spotify data is off for the assistant, so it can't see what's playing." });
        }
        self.ai_known_track(it);
        let ctx = &self.pb.context_uri;
        json!({
            "source": "spotify",
            "song": { "id": s(it, "uri"), "title": s(it, "name"), "artist": it["artists"].as_array().map(|a| a.iter().map(|x| s(x, "name")).collect::<Vec<_>>().join(", ")), "album": s(&it["album"], "name"), "album_id": s(&it["album"], "uri") },
            "playing": self.pb.is_playing,
            "position": format!("{} / {}", crate::model::fmt_ms(self.pb.position()), crate::model::fmt_ms(it["duration_ms"].as_u64().unwrap_or(0))),
            "shuffle": self.pb.shuffle,
            "repeat": self.pb.repeat,
            "volume_percent": (self.pb.volume * 100.0).round(),
            "device": self.pb.device.as_ref().map(|d| s(d, "name").to_string()),
            "playing_from": if ctx.is_empty() { Value::Null } else if ctx.ends_with(":collection") { json!("liked_songs") } else { self.ai_known_context(ctx); json!(ctx) },
        })
    }

    /// Carries out an action the model asked for, and answers truthfully.
    fn ai_act(&mut self, op: AppOp, reply: Option<Reply>) {
        match op {
            AppOp::NowPlaying => {
                let v = self.ai_now_playing(self.ai_source().is_some());
                if let Some(r) = reply {
                    let _ = r.send(Ok(v));
                }
            }
            AppOp::Control(c) => self.ai_control(c, reply),
            AppOp::QueueSpotify(uris) => self.ai_queue_spotify(uris, reply),
            AppOp::PlaySpotify { uris, context, shuffle } => self.ai_play_spotify(uris, context, shuffle, reply),
            AppOp::Like(uris) => self.ai_like(uris, reply),
        }
    }

    /// Reports a finished change: to the model now (during a request) or
    /// next time (approved between requests), and to the panel.
    fn ai_done(&mut self, mut w: Wrote, reply: Option<Reply>) {
        w.tell |= reply.is_none();
        if let Some(r) = reply {
            let _ = r.send(w.result.clone().map(Value::String));
        }
        self.ai_wrote(w);
    }

    /// Runs a Spotify change off the UI thread; the answer comes back as `Msg::AiWrote`.
    fn ai_spotify_job<F, Fut>(&mut self, reply: Option<Reply>, job: F)
    where
        F: FnOnce(Arc<dyn Source>) -> Fut,
        Fut: std::future::Future<Output = Wrote> + Send + 'static,
    {
        let Some(src) = self.ai_source() else {
            return self.ai_done(Wrote::err("Spotify isn't connected, so nothing changed."), reply);
        };
        let tell = reply.is_none();
        let job = job(src);
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let mut w = job.await;
            w.tell |= tell;
            if let Some(r) = reply {
                let _ = r.send(w.result.clone().map(Value::String));
            }
            let _ = tx.send(Msg::AiWrote(w));
            ctx.request_repaint();
        });
    }

    pub fn ai_wrote(&mut self, w: Wrote) {
        if let Some((did, pid)) = &w.draft
            && let Some(d) = self.ai.drafts.iter_mut().find(|d| &d.id == did)
        {
            d.status = match pid {
                Some(p) => {
                    // The new playlist can be played or added to later in the chat.
                    self.ai.catalog.add(Item { uri: format!("spotify:playlist:{p}"), name: d.name.clone(), by: "you".into(), ..Default::default() });
                    DraftStatus::Saved(p.clone())
                }
                None => DraftStatus::Editing,
            };
        }
        match &w.result {
            Ok(t) if !t.is_empty() => {
                if !w.quiet {
                    self.ai.note(t.clone());
                }
                if w.tell {
                    let id = w.draft.as_ref().and_then(|(_, p)| p.as_ref()).map(|p| format!(" Playlist ID: spotify:playlist:{p}.")).unwrap_or_default();
                    self.ai.tell_model(format!("{t}{id}"));
                }
            }
            Ok(_) => {}
            Err(e) => {
                if w.tell {
                    self.ai.transcript.push(Line::Error(e.clone()));
                    self.ai.tell_model(format!("{e} Nothing else changed."));
                } else {
                    self.ai.note(e.clone());
                }
            }
        }
        if let Some((card, uris, playlist)) = &w.picked
            && let Some(p) = self.ai.picks.iter_mut().find(|p| &p.id == card)
        {
            for s in p.songs.iter_mut().filter(|s| uris.contains(&s.uri) && !s.added_to.contains(playlist)) {
                s.added_to.push(playlist.clone());
            }
        }
        if let Some((uris, playlist)) = &w.unpicked {
            for s in self.ai.picks.iter_mut().flat_map(|p| p.songs.iter_mut()).filter(|s| uris.contains(&s.uri)) {
                s.added_to.retain(|x| x != playlist);
            }
        }
        if let Some(u) = w.undo {
            self.ai_undo = u;
        }
        if w.invalidate.iter().any(|k| k.starts_with("/me/tracks") || k.starts_with("/me/playlists")) {
            // Liked or saved something: what counts as "theirs" changed.
            self.ai.library.invalidate();
        }
        for k in &w.invalidate {
            self.invalidate(k);
        }
        for (u, on) in w.saved {
            self.saved.insert(u, on);
        }
        if w.refresh_player {
            self.poll_soon();
        }
    }

    /// Adds validated songs to the Spotify queue, one by one, and says how many made it.
    fn ai_queue_spotify(&mut self, uris: Vec<String>, reply: Option<Reply>) {
        let dev = self.device_param(false);
        let n = uris.len();
        self.ai_spotify_job(reply, move |src| async move {
            for (i, u) in uris.iter().enumerate() {
                if let Err(e) = src.send(Method::POST, format!("/me/player/queue?uri={}{dev}", enc(u)), None).await {
                    let why = player_error(&e);
                    return if i == 0 {
                        Wrote::err(format!("Couldn't queue: {why}."))
                    } else {
                        Wrote { refresh_player: true, undo: Some(None), ..Wrote::err(format!("Queued {i} of {n} songs, then Spotify refused: {why}.")) }
                    };
                }
            }
            // Spotify's queue can't be undone from here.
            Wrote { refresh_player: true, undo: Some(None), ..Wrote::ok(format!("Queued {n} song{} on Spotify.", plural(n))) }
        });
    }

    /// Plays validated songs or a context — here, or on the device that's playing.
    fn ai_play_spotify(&mut self, uris: Vec<String>, context: Option<String>, shuffle: Option<bool>, reply: Option<Reply>) {
        if self.ai_source().is_none() {
            return self.ai_done(Wrote::err("Spotify isn't connected, so nothing was played."), reply);
        }
        let what = match &context {
            Some(c) if c.ends_with(":collection") => "your Liked Songs".to_string(),
            Some(c) => self.ai.catalog.get(c).map(|i| format!("“{}”", i.name)).unwrap_or_else(|| "it".into()),
            None if uris.len() == 1 => self.ai.catalog.get(&uris[0]).map(|i| format!("“{}”", i.name)).unwrap_or_else(|| "the song".into()),
            None => format!("{} songs", uris.len()),
        };
        let how = if shuffle == Some(true) { " on shuffle" } else { "" };
        let body = match &context {
            Some(c) => json!({ "context_uri": c }),
            None => json!({ "uris": uris }),
        };
        // No jump to Now Playing: the assistant works in the background.
        self.dj_cancel();
        self.stop_local();
        if self.pb.device.is_none() && self.engine_starting() {
            self.pending_play = Some(crate::app::PendingPlay { body, shuffle, quiet: true });
            self.pb.is_playing = true;
            return self.ai_done(Wrote::ok(format!("Playing {what}{how} on this computer as soon as the player is ready.")), reply);
        }
        if self.is_local()
            && let Some(e) = self.engine.as_ref()
        {
            let r = e.load(&body, self.play_mode(&body, shuffle)).map_err(|e| e.to_string());
            self.local_touched = Some(std::time::Instant::now());
            self.poll_soon();
            let w = match r {
                Ok(()) => {
                    self.pb.is_playing = true;
                    if let Some(on) = shuffle {
                        self.pb.shuffle = on;
                    }
                    Wrote::ok(format!("Playing {what}{how}."))
                }
                Err(e) => Wrote::err(format!("Couldn't play {what}: {e}.")),
            };
            return self.ai_done(w, reply);
        }
        let (play, dev) = (format!("/me/player/play{}", self.device_param(true)), self.device_param(false));
        self.ai_spotify_job(reply, move |src| async move {
            if let Err(e) = src.send(Method::PUT, play, Some(body)).await {
                return Wrote::err(format!("Couldn't play {what}: {}.", player_error(&e)));
            }
            if let Some(on) = shuffle {
                let _ = src.send(Method::PUT, format!("/me/player/shuffle?state={on}{dev}"), None).await;
            }
            Wrote { refresh_player: true, ..Wrote::ok(format!("Playing {what}{how}.")) }
        });
    }

    /// Resume, pause, skip, shuffle, repeat, volume — for whatever is playing.
    fn ai_control(&mut self, c: Control, reply: Option<Reply>) {
        let said = match c {
            Control::Resume => "Resumed.".to_string(),
            Control::Pause => "Paused.".to_string(),
            Control::Next => "Skipped to the next song.".to_string(),
            Control::Previous => "Went back a song.".to_string(),
            Control::Shuffle(on) => format!("Shuffle {}.", if on { "on" } else { "off" }),
            Control::Repeat(m) => format!(
                "Repeat {}.",
                match m {
                    "track" => "one",
                    "context" => "all",
                    _ => "off",
                }
            ),
            Control::Volume(v) => format!("Volume {v}%."),
        };
        let quiet = |r: Result<(), String>| Wrote { quiet: true, ..if let Err(e) = r { Wrote::err(e) } else { Wrote::ok(said.clone()) } };
        if let Control::Volume(v) = c
            && (self.local_file_active() || self.is_local())
        {
            self.set_volume(v as f32 / 100.0, true);
            return self.ai_done(quiet(Ok(())), reply);
        }
        if self.local_file_active() {
            let Some(l) = self.local.as_mut() else { return };
            let r = match c {
                Control::Resume | Control::Pause => {
                    if l.is_playing() == (c == Control::Pause) {
                        l.toggle();
                    }
                    Ok(())
                }
                Control::Next => {
                    l.next();
                    Ok(())
                }
                Control::Previous => {
                    l.prev();
                    Ok(())
                }
                _ => Err("Shuffle and repeat are for Spotify playback; a local file is playing.".to_string()),
            };
            return self.ai_done(quiet(r), reply);
        }
        if self.pb.item.is_none() && !matches!(c, Control::Volume(_)) {
            return self.ai_done(quiet(Err("Nothing is playing.".into())), reply);
        }
        // A skip ends what the DJ was saying about the song, as the player's own buttons do.
        if matches!(c, Control::Next | Control::Previous) {
            self.dj_cancel();
        }
        if self.is_local()
            && let Some(e) = self.engine.as_ref()
        {
            let r = match c {
                Control::Resume => e.spirc.play(),
                Control::Pause => e.spirc.pause(),
                Control::Next => e.spirc.next(),
                Control::Previous => e.spirc.prev(),
                Control::Shuffle(on) => e.spirc.shuffle(on),
                Control::Repeat(m) => e.spirc.repeat(m != "off").and_then(|()| e.spirc.repeat_track(m == "track")),
                Control::Volume(_) => Ok(()),
            }
            .map_err(|e| e.to_string());
            if r.is_ok() {
                self.ai_mirror(c);
            }
            self.local_touched = Some(std::time::Instant::now());
            self.poll_soon();
            return self.ai_done(quiet(r), reply);
        }
        let (method, path) = match c {
            Control::Resume => (Method::PUT, "/me/player/play".to_string()),
            Control::Pause => (Method::PUT, "/me/player/pause".to_string()),
            Control::Next => (Method::POST, "/me/player/next".to_string()),
            Control::Previous => (Method::POST, "/me/player/previous".to_string()),
            Control::Shuffle(on) => (Method::PUT, format!("/me/player/shuffle?state={on}")),
            Control::Repeat(m) => (Method::PUT, format!("/me/player/repeat?state={m}")),
            Control::Volume(v) => (Method::PUT, format!("/me/player/volume?volume_percent={v}")),
        };
        let path = format!("{path}{}", self.device_param(!path.contains('?')));
        self.ai_mirror(c);
        self.ai_spotify_job(reply, move |src| async move {
            match src.send(method, path, None).await {
                Ok(_) => Wrote { quiet: true, refresh_player: true, ..Wrote::ok(said) },
                Err(e) => Wrote { quiet: true, refresh_player: true, ..Wrote::err(format!("Spotify didn't take it: {}.", player_error(&e))) },
            }
        });
    }

    /// Shows a control's effect right away (the next poll confirms it).
    fn ai_mirror(&mut self, c: Control) {
        match c {
            Control::Resume | Control::Pause => {
                self.pb.progress_ms = self.pb.position();
                self.pb.progress_at = Some(std::time::Instant::now());
                self.pb.is_playing = c == Control::Resume;
            }
            Control::Shuffle(on) => self.pb.shuffle = on,
            Control::Repeat(m) => self.pb.repeat = m.into(),
            Control::Volume(v) => self.pb.volume = v as f32 / 100.0,
            Control::Next | Control::Previous => {}
        }
    }

    /// Likes validated songs. Only ones not liked yet are added, so undo
    /// unlikes exactly those.
    fn ai_like(&mut self, uris: Vec<String>, reply: Option<Reply>) {
        self.ai_spotify_job(reply, move |src| async move {
            let mut fresh = Vec::new();
            let mut sure = true;
            for chunk in uris.chunks(50) {
                let ids: Vec<&str> = chunk.iter().map(|u| crate::model::id_of(u)).collect();
                match src.get(format!("/me/tracks/contains?ids={}", ids.join(",")), None).await {
                    Ok(Value::Array(flags)) if flags.len() == chunk.len() => fresh.extend(chunk.iter().zip(flags).filter(|(_, f)| *f != Value::Bool(true)).map(|(u, _)| u.clone())),
                    _ => {
                        sure = false;
                        fresh.extend(chunk.iter().cloned());
                    }
                }
            }
            let mut done = 0;
            for chunk in fresh.chunks(50) {
                let ids: Vec<&str> = chunk.iter().map(|u| crate::model::id_of(u)).collect();
                if let Err(e) = src.send(Method::PUT, format!("/me/tracks?ids={}", ids.join(",")), None).await {
                    if done == 0 {
                        return Wrote::err(format!("Couldn't like those songs: {e}."));
                    }
                    let liked = fresh[..done].to_vec();
                    return Wrote {
                        undo: Some(sure.then(|| AiUndo::Liked(liked.clone()))),
                        invalidate: vec!["/me/tracks".into()],
                        saved: liked.into_iter().map(|u| (u, true)).collect(),
                        ..Wrote::err(format!("Liked {done} of {} songs, then Spotify refused: {e}.", fresh.len()))
                    };
                }
                done += chunk.len();
            }
            let (n, already) = (fresh.len(), uris.len() - fresh.len());
            let text = match (n, already) {
                (0, _) => "Those songs were already in Liked Songs.".to_string(),
                (n, 0) => format!("Liked {n} song{}.", plural(n)),
                (n, a) => format!("Liked {n} song{} ({a} already liked).", plural(n)),
            };
            Wrote { undo: (n > 0).then(|| sure.then(|| AiUndo::Liked(fresh.clone()))), invalidate: vec!["/me/tracks".into()], saved: fresh.into_iter().map(|u| (u, true)).collect(), ..Wrote::ok(text) }
        });
    }

    /// Saves a Spotify preview (the user's approval): a new private
    /// playlist, or the songs not already there added to an editable one.
    fn ai_save_spotify_draft(&mut self, d: Draft, add_to: Option<String>) {
        if self.ai_source().is_none() {
            self.ai.transcript.push(Line::Error("Spotify isn't connected, so the playlist wasn't saved.".into()));
            return;
        }
        let target = add_to.as_ref().map(|pid| self.ai.catalog.get(&format!("spotify:playlist:{pid}")).map(|i| i.name).filter(|n| !n.is_empty()).unwrap_or_else(|| "your playlist".into()));
        let length = crate::model::fmt_ms(d.tracks.iter().filter_map(|u| self.ai.catalog.get(u)).map(|i| i.duration_ms).sum());
        if let Some(x) = self.ai.drafts.iter_mut().find(|x| x.id == d.id) {
            x.status = DraftStatus::Saving;
        }
        self.ai.approvals.retain(|a| !matches!(&a.kind, ApprovalKind::SaveDraft { draft_id, .. } if *draft_id == d.id));
        let me = self.user_id();
        self.ai_spotify_job(None, move |src| async move {
            let fail = |e: String| Wrote { draft: Some((d.id.clone(), None)), ..Wrote::err(e) };
            let (pid, created, songs) = match &add_to {
                Some(pid) => {
                    let existing: HashSet<String> = match src.get(format!("/playlists/{pid}/tracks?limit=100"), Some(10_000)).await {
                        Ok(v) => v["items"].as_array().map(|a| a.iter().filter_map(|r| r["track"]["uri"].as_str().map(str::to_string)).collect()).unwrap_or_default(),
                        Err(e) => return fail(format!("Couldn't read the playlist: {e}.")),
                    };
                    (pid.clone(), false, d.tracks.iter().filter(|u| !existing.contains(*u)).cloned().collect::<Vec<_>>())
                }
                None => {
                    let body = json!({ "name": d.name, "public": false, "description": d.description });
                    // The other endpoint only when this one isn't there: after any other failure
                    // the playlist may exist already (see `App::mutate`).
                    let made = match src.send(Method::POST, "/me/playlists".into(), Some(body.clone())).await {
                        Err(e) if e.starts_with("404") && !me.is_empty() => src.send(Method::POST, format!("/users/{}/playlists", enc(&me)), Some(body)).await,
                        r => r,
                    };
                    match made.map(|v| s(&v, "id").to_string()) {
                        Ok(id) if !id.is_empty() => (id, true, d.tracks.clone()),
                        Ok(_) => return fail("Spotify didn't return the new playlist, so nothing was saved.".into()),
                        Err(e) => return fail(format!("Couldn't create the playlist: {e}.")),
                    }
                }
            };
            let name = target.clone().unwrap_or_else(|| d.name.clone());
            let mut added = 0;
            for chunk in songs.chunks(100) {
                let b = json!({ "uris": chunk });
                let mut r = src.send(Method::POST, format!("/playlists/{pid}/tracks"), Some(b.clone())).await;
                if r.as_ref().is_err_and(|e| e.starts_with("404")) {
                    r = src.send(Method::POST, format!("/playlists/{pid}/items"), Some(b)).await;
                }
                if let Err(e) = r {
                    let undo = AiUndo::SpotifyPlaylist { id: pid.clone(), name, created, added: songs[..added].to_vec() };
                    return Wrote {
                        undo: Some(Some(undo)),
                        invalidate: vec!["/me/playlists".into(), pid.clone()],
                        ..fail(format!("Saved only {added} of {} songs: {e}. Undo removes what was saved.", songs.len()))
                    };
                }
                added += chunk.len();
            }
            let n = songs.len();
            let text = if created {
                format!("Saved “{}” to Spotify — {n} song{} · {length}. Undo is available.", d.name, plural(n))
            } else if n == 0 {
                format!("All of these songs are already in “{name}”.")
            } else {
                let skipped = d.tracks.len() - n;
                let skipped = if skipped > 0 { format!(" ({skipped} already there)") } else { String::new() };
                format!("Added {n} song{} to “{name}” on Spotify{skipped}. Undo is available.", plural(n))
            };
            Wrote {
                undo: (created || n > 0).then(|| Some(AiUndo::SpotifyPlaylist { id: pid.clone(), name, created, added: songs })),
                invalidate: vec!["/me/playlists".into(), pid.clone()],
                draft: Some((d.id.clone(), Some(pid))),
                ..Wrote::ok(text)
            }
        });
    }

    // ------------------------------------------------------------ recommendations

    /// ▶ on a recommendation: its 30-second preview (▶ again stops it).
    /// What's playing pauses for it, and plays again when it ends.
    pub fn ai_preview(&mut self, uri: String) {
        let was = self.ai_preview.take();
        if was.as_ref().is_some_and(|p| p.uri == uri) {
            return self.ai_preview_end(was, true);
        }
        // From one preview to another: what the first paused still comes back after.
        let mut resume = was.as_ref().is_some_and(|p| p.resume);
        drop(was);
        let Some(src) = self.ai_source() else {
            self.ai.note("Spotify isn't connected, so there's no preview to play.");
            return;
        };
        if !resume && (self.pb.is_playing || self.local.as_ref().is_some_and(|l| l.is_playing())) {
            self.apply(crate::app::Action::TogglePlay);
            resume = true;
        }
        self.ai_preview_gen += 1;
        let generation = self.ai_preview_gen;
        self.ai_preview = Some(crate::ai::picks::Preview { uri: uri.clone(), generation, voice: None, started: None, resume });
        let (catalog, tx, ctx, volume, http) = (self.ai.catalog.clone(), self.tx.clone(), self.ctx.clone(), self.pb.volume, self.dj_http.clone());
        self.rt.spawn(async move {
            let r = match crate::ai::picks::clip(src.as_ref(), &http, &catalog, &uri).await {
                // Opening the output takes a moment: not on the UI thread.
                Ok(Some(clip)) => tokio::task::spawn_blocking(move || crate::dj::Voice::play(&[Arc::new(clip)], volume)).await.map_err(|e| e.to_string()),
                Ok(None) => Err("Spotify has no preview of that song".to_string()),
                Err(e) => Err(e),
            };
            let _ = tx.send(Msg::AiPreview(generation, r));
            ctx.request_repaint();
        });
    }

    pub fn ai_preview_ready(&mut self, generation: u64, r: Result<Option<crate::dj::Voice>, String>) {
        // A preview stopped (or replaced) while loading: dropping its voice stops it.
        let Some(p) = self.ai_preview.as_mut().filter(|p| p.generation == generation) else { return };
        match r {
            Ok(Some(v)) => {
                p.voice = Some(v);
                p.started = Some(std::time::Instant::now());
            }
            Ok(None) | Err(_) => {
                if let Err(e) = r {
                    self.ai.note(format!("{e}."));
                }
                let p = self.ai_preview.take();
                self.ai_preview_end(p, true);
            }
        }
    }

    /// A preview stops; with `resume`, what it paused plays again.
    pub(crate) fn ai_preview_end(&mut self, p: Option<crate::ai::picks::Preview>, resume: bool) {
        let Some(p) = p else { return };
        drop(p.voice);
        if resume && p.resume && !(self.pb.is_playing || self.local.as_ref().is_some_and(|l| l.is_playing())) {
            self.apply(crate::app::Action::TogglePlay);
        }
    }

    /// A preview that played to its end stops.
    pub fn ai_preview_tick(&mut self) {
        if self.ai_preview.as_ref().is_some_and(|p| p.done()) {
            let p = self.ai_preview.take();
            self.ai_preview_end(p, true);
        }
    }

    /// The name of one of the user's playlists (bare ID), as the assistant or the library knows it.
    pub fn ai_playlist_name(&mut self, id: &str) -> String {
        self.ai
            .catalog
            .get(&format!("spotify:playlist:{id}"))
            .map(|i| i.name)
            .filter(|n| !n.is_empty())
            .or_else(|| crate::ui::menus::writable_playlists(self).iter().find(|p| s(p, "id") == id).map(|p| s(p, "name").to_string()))
            .unwrap_or_else(|| "your playlist".into())
    }

    /// "+ add" on a recommendation (`uri` None: "add all"): into `playlist`
    /// (a bare ID), leaving out what it already has. The assistant hears of it.
    pub fn ai_add_picks(&mut self, card: &str, uri: Option<String>, playlist: String) {
        let Some(p) = self.ai.picks.iter().find(|p| p.id == card) else { return };
        let songs: Vec<(String, String, String)> = p
            .songs
            .iter()
            .filter(|x| uri.as_ref().is_none_or(|u| &x.uri == u) && !x.added_to.contains(&playlist))
            .map(|x| {
                let it = self.ai.catalog.get(&x.uri).unwrap_or_default();
                (x.uri.clone(), it.name, it.by)
            })
            .collect();
        if songs.is_empty() {
            return;
        }
        let name = self.ai_playlist_name(&playlist);
        let card = card.to_string();
        self.ai_spotify_job(None, move |src| async move {
            match crate::ai::picks::add(src.as_ref(), &playlist, &songs).await {
                Ok(a) => {
                    let title = |u: &String| songs.iter().find(|x| &x.0 == u).map(|x| format!("“{}”", x.1)).unwrap_or_else(|| "That song".into());
                    let text = match (a.uris.len(), a.already.len()) {
                        (0, 1) => format!("{} is already in “{name}”.", title(&a.already[0])),
                        (0, _) => format!("Those songs are already in “{name}”."),
                        (1, 0) => format!("Added {} to “{name}”.", title(&a.uris[0])),
                        (n, 0) => format!("Added {n} songs to “{name}”."),
                        (n, k) => format!("Added {n} song{} to “{name}” ({k} already there).", plural(n)),
                    };
                    let picked = Some((card, a.uris.iter().chain(&a.already).cloned().collect(), playlist.clone()));
                    let undo = (!a.uris.is_empty()).then(|| Some(AiUndo::SpotifyPlaylist { id: playlist.clone(), name, created: false, added: a.uris }));
                    Wrote { undo, invalidate: vec!["/me/playlists".into(), playlist], picked, ..Wrote::ok(text) }
                }
                Err(e) => Wrote::err(format!("Couldn't add to “{name}”: {e}.")),
            }
        });
    }

    pub fn ai_dismiss_picks(&mut self, card: &str) {
        let Some(i) = self.ai.picks.iter().position(|p| p.id == card) else { return };
        let p = self.ai.picks.remove(i);
        if self.ai_preview.as_ref().is_some_and(|x| p.songs.iter().any(|s| s.uri == x.uri)) {
            let x = self.ai_preview.take();
            self.ai_preview_end(x, true);
        }
    }

    // ------------------------------------------------------------ account

    pub fn ai_sign_in(&mut self, reconsent: bool, new_account: bool) {
        if self.ai.signing_in.is_some() {
            return;
        }
        let active = self.ai.accounts.active().cloned();
        let (client, expect) = match active.filter(|_| !new_account) {
            Some(r) => (OAuthClient::Existing { client_id: r.client_id.clone(), id_token_hint: self.ai.auth.id_token_hint(&r.client_id), login_hint: r.email.clone() }, r.subject.clone()),
            None => (OAuthClient::New, None),
        };
        let (auth, host, tx, tx2, ctx) = (self.ai.auth.clone(), self.ai.accounts.host_id.clone(), self.tx.clone(), self.tx.clone(), self.ctx.clone());
        let task = self.rt.spawn(async move {
            let r = auth
                .sign_in(
                    client,
                    expect,
                    host,
                    reconsent,
                    |url| {
                        // The system browser; the URL is never logged.
                        let _ = open::that(url);
                    },
                    move |client_id| {
                        let _ = tx2.send(Msg::AiRegistered(client_id.to_string()));
                    },
                )
                .await;
            let _ = tx.send(Msg::AiSignedIn(r));
            ctx.request_repaint();
        });
        self.ai.signing_in = Some(task.abort_handle());
        self.ai.account_note = Some("Finish signing in with your browser…".into());
    }

    pub fn ai_cancel_sign_in(&mut self) {
        if let Some(t) = self.ai.signing_in.take() {
            t.abort();
            self.ai.account_note = Some("Sign-in cancelled.".into());
        }
    }

    pub fn ai_registered(&mut self, client_id: String) {
        // Saved for reuse even if the rest fails, but not made active until verified.
        if !self.ai.accounts.registrations().iter().any(|r| r.client_id == client_id) {
            self.ai.accounts.upsert(crate::ai::account::Registration { client_id, subject: None, email: None, plan_enabled: false });
        }
    }

    pub fn ai_signed_in(&mut self, r: Result<crate::ai::account::SignedIn, AuthError>) {
        self.ai.signing_in = None;
        match r {
            Ok(s) => {
                let cid = s.registration.client_id.clone();
                self.ai.accounts.upsert(s.registration);
                self.ai.accounts.set_active(Some(cid));
                self.ai.models = Models::Unknown;
                self.ai.models_for.clear();
                self.ai.limit_reached = false;
                if s.plan_enabled {
                    self.ai.account_note = None;
                    if !self.config.assistant().welcomed {
                        // The one-time confirmation; never shown again.
                        self.modal = Some(Modal::AiWelcome);
                        self.set_config("assistant.welcomed", true.into());
                    }
                } else {
                    self.ai.account_note =
                        Some("Signed in, but permission to use your ChatGPT plan wasn't granted, so the assistant can't use it. You can enable it, or use your own API key instead.".into());
                }
            }
            Err(AuthError::Declined) => self.ai.account_note = Some("You declined in the browser, so nothing changed. You can enable ChatGPT plan use later.".into()),
            Err(e) => self.ai.account_note = Some(e.to_string()),
        }
    }

    pub fn ai_select_account(&mut self, client_id: String) {
        if self.ai.accounts.registrations().iter().any(|r| r.client_id == client_id) {
            self.ai_cancel_quiet();
            self.ai.accounts.set_active(Some(client_id));
            self.ai.models = Models::Unknown;
            self.ai.models_for.clear();
            self.ai.limit_reached = false;
        }
    }

    pub fn ai_sign_out(&mut self) {
        let Some(r) = self.ai.accounts.active().cloned() else { return };
        self.ai_cancel_quiet();
        let (auth, tx, ctx) = (self.ai.auth.clone(), self.tx.clone(), self.ctx.clone());
        let cid = r.client_id.clone();
        self.rt.spawn(async move {
            let confirmed = auth.sign_out(&cid).await;
            let _ = tx.send(Msg::AiSignedOut { confirmed });
            ctx.request_repaint();
        });
        // Keep the registration (client ID) for a later sign-in; it just isn't usable now.
        self.ai.accounts.upsert(crate::ai::account::Registration { plan_enabled: false, ..r });
        self.ai.accounts.set_active(None);
        self.ai.models = Models::Unknown;
        self.ai.models_for.clear();
        self.ai.account_note = Some("Signing out…".into());
    }

    pub fn ai_signed_out(&mut self, confirmed: bool) {
        self.ai.account_note = Some(if confirmed {
            "Signed out, and access was revoked.".into()
        } else {
            "Signed out in Glass, but revoking access wasn't confirmed. To be sure, disconnect Glass in ChatGPT settings.".into()
        });
    }

    pub fn ai_save_api_key(&mut self) {
        let key = std::mem::take(&mut self.ai.key_field);
        let key = key.trim();
        if key.is_empty() {
            return;
        }
        match self.ai.auth.store.set(secrets::API_KEY_KEY, key) {
            Ok(()) => {
                self.ai.set_has_api_key(true);
                self.ai.models = Models::Unknown;
                self.ai.models_for.clear();
                self.toast(format!("API key saved in the {}", if cfg!(target_os = "macos") { "Keychain" } else { "system keyring" }));
            }
            Err(e) => self.toast(format!("The key wasn't saved. {e}")),
        }
    }

    pub fn ai_forget_api_key(&mut self) {
        let _ = self.ai.auth.store.delete(secrets::API_KEY_KEY);
        self.ai.set_has_api_key(false);
        if self.ai_billing() == Billing::ApiKey {
            self.ai.models = Models::Unknown;
            self.ai.models_for.clear();
        }
        self.toast("API key removed");
    }

    pub fn open_manage_usage(&self) {
        let _ = open::that(MANAGE_USAGE_URL);
    }

    /// The memory-only secret store and mock endpoints used by tests.
    #[cfg(test)]
    pub fn ai_use_test_backend(&mut self, store: std::sync::Arc<crate::ai::secrets::MemoryStore>, endpoints: crate::ai::account::Endpoints) {
        let dir = crate::ai::account::Accounts::dir();
        self.ai = crate::ai::Assistant::new(store, endpoints, dir);
    }
}
