//! Lyrics and their translation on the iOS core: the desktop's lyrics flow
//! (src/app.rs `lyrics_*`, `sync_lyrics`, `start_lyrics`, `lyrics_search` and
//! src/translate_app.rs) on `MobileApp`, and the `LyricsModel` SwiftUI draws.
//! Lookups run only while the lyrics are on screen, once per song and setting;
//! each has a generation, so one song's answer can never land on another.

use crate::app::{Action, MobileApp, Msg};
use crate::bridge::CoreEvent;
use crate::lyrics::cache::Cache as LyricsCache;
use crate::lyrics::service::{self as lyrics_service, LyricsState, ProviderSetting, Request as LyricsRequest, Resolved, Sources, Status};
use crate::lyrics::translate::{self, Engine, Fix, Mode, Translation};
use crate::lyrics::{Candidate, Lyrics, Match, Outcome, ProviderId, Timing, TrackRef, Voice};
use crate::model;
use std::sync::Arc;

/// The lyrics side of the core's state.
pub struct LyricsCore {
    pub state: LyricsState,
    pub cache: Arc<LyricsCache>,
    pub choices: crate::lyrics::choices::Choices,
    /// Every lyrics service's client and the user's keys.
    pub sources: Arc<Sources>,
    client_for: (String, String),
    pub nonce: u64,
    pub picker: Option<Picker>,
    search_seq: u64,
    /// The playing song's own offset, by lyrics key.
    song_offset: Option<(String, i64)>,
    /// Lyrics are on screen (Now Playing's lyrics pane).
    pub visible: bool,
    pub translation: TranslationState,
    pub translation_store: translate::Store,
    published: Option<LyricsModel>,
}

/// The manual picker: an LRCLIB search and its results (desktop `LyricsPicker`).
pub struct Picker {
    pub query: String,
    pub results: Option<Result<Arc<Vec<Candidate>>, String>>,
    pub generation: u64,
}

#[derive(Default)]
pub struct TranslationState {
    pub key: String,
    pub result: Option<Arc<Translation>>,
    pub task: Option<tokio::task::AbortHandle>,
}

impl LyricsCore {
    pub fn new(contact: &str, proxy: &str, log: crate::api::NetLog, demo: bool) -> Self {
        let (cache, sources) = if demo {
            // The demo never contacts a lyrics service: answers come from memory, the clients point nowhere.
            let cache = Arc::new(LyricsCache::in_memory());
            (cache.clone(), Sources::offline(cache))
        } else {
            let cache = Arc::new(LyricsCache::new(LyricsCache::dir()));
            (cache.clone(), Sources::new(contact, proxy, Some(log), cache, Some(crate::platform::secrets())))
        };
        Self {
            state: LyricsState::default(),
            cache,
            choices: crate::lyrics::choices::Choices::load(crate::lyrics::choices::Choices::path()),
            sources,
            client_for: (contact.to_string(), proxy.to_string()),
            nonce: 0,
            picker: None,
            search_seq: 0,
            song_offset: None,
            visible: false,
            translation: TranslationState::default(),
            translation_store: translate::Store::new(translate::Store::dir()),
            published: None,
        }
    }
}

/// The translation key (desktop translate_app `key`) and the part saved translations are filed under.
fn tkey(lyrics: &Lyrics, target: &str, engine: Engine, assistant: bool) -> String {
    format!("{}|{}|{}|{target}|{}|{assistant}", lyrics.source.provider.name(), lyrics.source.id, lyrics.lines.len(), engine.name())
}

fn stored(key: &str) -> &str {
    key.rsplitn(3, '|').nth(2).unwrap_or(key)
}

fn same_language(a: &str, b: &str) -> bool {
    translate::base_code(a) == translate::base_code(b)
}

impl MobileApp {
    // ------------------------------------------------------------ the song

    /// Desktop `lyrics_track`: the playing song as lyrics providers need it (podcasts have none).
    pub fn lyrics_track(&self) -> Option<TrackRef> {
        if self.local_file_active() {
            let t = self.local.as_ref()?.current()?;
            return Some(TrackRef {
                key: format!("local:{}", t.path.display()),
                title: t.title.clone(),
                artists: vec![t.artist.clone()],
                album: t.album.clone(),
                duration_ms: t.duration_ms,
                isrc: String::new(),
            });
        }
        crate::lyrics::track_ref(self.pb.item.as_deref()?)
    }

    fn lyrics_key(&self) -> Option<String> {
        if self.local_file_active() {
            let t = self.local.as_ref()?.current()?;
            return Some(format!("local:{}", t.path.display()));
        }
        let it = self.pb.item.as_ref()?;
        (model::s(it, "type") == "track" && !model::s(it, "uri").is_empty()).then(|| model::s(it, "uri").to_string())
    }

    pub fn lyrics_song_offset(&mut self) -> i64 {
        let Some(key) = self.lyrics_key() else { return 0 };
        if let Some((k, offset)) = &self.lyrics.song_offset
            && *k == key
        {
            return *offset;
        }
        let offset = self.lyrics_track().map(|t| self.lyrics.choices.offset(&t)).unwrap_or(0);
        self.lyrics.song_offset = Some((key, offset));
        offset
    }

    fn lyrics_request(&self, track: TrackRef) -> LyricsRequest {
        let c = self.config.lyrics();
        LyricsRequest { track, setting: c.provider, fallback: c.fallback, sources: c.sources.clone(), nonce: self.lyrics.nonce }
    }

    /// Desktop `sync_lyrics`, each tick: drop a stale lookup when the song changes,
    /// start one when lyrics are on screen and nothing has been asked yet.
    pub fn sync_lyrics(&mut self) {
        let key = self.lyrics_key();
        if self.lyrics.state.request.is_some() && self.lyrics.state.track_key() != key.as_deref() {
            self.lyrics.state.reset();
            self.lyrics.picker = None;
        }
        if key.is_none() || !self.lyrics.visible {
            return;
        }
        let c = self.config.lyrics();
        if let Some(r) = &self.lyrics.state.request
            && (r.setting, r.fallback, r.nonce) == (c.provider, c.fallback, self.lyrics.nonce)
            && r.sources == c.sources
        {
            return;
        }
        let Some(track) = self.lyrics_track() else { return };
        let req = self.lyrics_request(track);
        if self.lyrics.state.wants(&req) {
            self.start_lyrics(req);
        }
    }

    fn start_lyrics(&mut self, req: LyricsRequest) {
        if let Some(choice) = self.lyrics.choices.pick(&req.track) {
            let generation = self.lyrics.state.begin(req);
            self.lyrics.state.finish(generation, lyrics_service::chosen(choice.as_ref()));
            return;
        }
        let generation = self.lyrics.state.begin(req.clone());
        let spotify: Option<Arc<dyn crate::ai::spotify::Source>> = self.api.clone().map(|a| Arc::new(a) as Arc<dyn crate::ai::spotify::Source>);
        let (sources, tx) = (self.lyrics.sources.clone(), self.tx.clone());
        let task = self.rt.spawn(lyrics_service::run(sources, req.setting, req.fallback, req.sources, req.track, None, spotify, move |resolved| {
            tx.send(Msg::Lyrics { generation, resolved });
        }));
        self.lyrics.state.running(task.abort_handle());
    }

    pub fn refresh_lyrics_client(&mut self) {
        if self.demo {
            return;
        }
        let want = (self.config.lyrics().contact.clone(), self.settings.proxy.clone());
        if want != self.lyrics.client_for {
            self.lyrics.sources = Sources::new(&want.0, &want.1, Some(self.log.clone()), self.lyrics.cache.clone(), Some(crate::platform::secrets()));
            self.lyrics.client_for = want;
        }
    }

    pub fn set_config(&mut self, key: &str, v: toml_edit::Value) {
        if let Err(e) = self.config.set(key, v) {
            log::warn!("config {key}: {e}");
            self.toast("Couldn't change that setting. Try again.");
        }
    }

    fn lyrics_search(&mut self, q: String) {
        let Some(track) = self.lyrics_track() else { return };
        self.lyrics.search_seq += 1;
        let generation = self.lyrics.search_seq;
        self.lyrics.picker = Some(Picker { query: q.clone(), results: None, generation });
        let (client, tx) = (self.lyrics.sources.lrclib.clone(), self.tx.clone());
        self.rt.spawn(async move {
            let result = client.search_text(&q).await.map_err(|e| e.to_string()).map(|mut found| {
                found.sort_by(|a, b| {
                    let (sa, sb) = (crate::lyrics::matching::score(&track, a), crate::lyrics::matching::score(&track, b));
                    sb.unwrap_or(f32::MIN).total_cmp(&sa.unwrap_or(f32::MIN)).then(a.duration_ms.abs_diff(track.duration_ms).cmp(&b.duration_ms.abs_diff(track.duration_ms)))
                });
                found.iter().map(|r| r.candidate()).collect()
            });
            tx.send(Msg::LyricsSearch { generation, result });
        });
    }

    // ------------------------------------------------------------ actions

    pub fn lyrics_action(&mut self, a: LyricsAction) {
        match a {
            LyricsAction::Visible(v) => self.lyrics.visible = v,
            LyricsAction::Retry => {
                if let Some(k) = self.lyrics.state.track_key().map(str::to_string) {
                    self.lyrics.cache.forget(&k);
                }
                if let Some(t) = self.lyrics_track() {
                    self.lyrics.choices.forget_pick(&t);
                    self.lyrics.song_offset = None;
                }
                self.lyrics.nonce += 1;
            }
            LyricsAction::Pick { index } => {
                let Some(track) = self.lyrics_track() else { return };
                let Some(c) = self.picker_candidates().get(index).cloned() else { return };
                self.lyrics.picker = None;
                self.lyrics.nonce += 1;
                self.lyrics.choices.set_pick(&track, Some(&c.record));
                self.lyrics.song_offset = None;
                let resolved = lyrics_service::chosen(Some(&c.record));
                let generation = self.lyrics.state.begin(self.lyrics_request(track));
                self.lyrics.state.finish(generation, resolved);
            }
            LyricsAction::NoneFit => {
                if let Some(track) = self.lyrics_track() {
                    self.lyrics.choices.set_pick(&track, None);
                    self.lyrics.song_offset = None;
                }
                self.lyrics.picker = None;
                self.lyrics.nonce += 1;
            }
            LyricsAction::OpenPicker => {
                let Some(track) = self.lyrics_track() else { return };
                if let Status::Done(res) = &self.lyrics.state.status
                    && let Outcome::Ambiguous { candidates, .. } = &res.outcome
                {
                    let candidates = Arc::new(candidates.clone());
                    self.lyrics.picker = Some(Picker { query: format!("{} {}", track.title, track.artist()), results: Some(Ok(candidates)), generation: 0 });
                } else {
                    self.lyrics_search(format!("{} {}", crate::lyrics::matching::search_title(&track.title), track.artist()));
                }
            }
            LyricsAction::Search(q) => {
                if !q.trim().is_empty() {
                    self.lyrics_search(q);
                }
            }
            LyricsAction::ClosePicker => self.lyrics.picker = None,
            LyricsAction::Provider(p) => self.set_config("lyrics.provider", p.into()),
            LyricsAction::Sources(list) => {
                let list: Vec<ProviderId> = list.iter().filter_map(|s| ProviderId::parse(s)).filter(|p| ProviderId::WORD.contains(p)).collect();
                self.set_config("lyrics.sources", crate::user_config::sources_value(&list));
            }
            LyricsAction::Key { provider, key } => {
                let Some(p) = ProviderId::parse(&provider).filter(|p| p.needs_key()) else { return };
                let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
                match self.lyrics.sources.set_key(p, key.as_deref()) {
                    Ok(()) => {
                        // A key added puts its service first in line; a key removed takes it out.
                        let mut list = self.config.lyrics().sources.clone();
                        list.retain(|&x| x != p);
                        if key.is_some() {
                            list.insert(0, p);
                        }
                        self.set_config("lyrics.sources", crate::user_config::sources_value(&list));
                        self.toast(if key.is_some() { format!("Saved your {} key", p.label()) } else { format!("Removed your {} key", p.label()) });
                        self.lyrics.nonce += 1;
                    }
                    Err(e) => {
                        log::warn!("lyrics key: {e}");
                        self.toast("Couldn't keep the key. Try again.");
                    }
                }
            }
            LyricsAction::Fallback(on) => self.set_config("lyrics.fallback", on.into()),
            LyricsAction::Offset(ms) => {
                let ms = ms.clamp(-crate::user_config::OFFSET_LIMIT, crate::user_config::OFFSET_LIMIT);
                self.set_config("lyrics.offset_ms", ms.into());
            }
            LyricsAction::SongOffset(ms) => {
                if let Some(t) = self.lyrics_track() {
                    let ms = ms.clamp(-crate::user_config::OFFSET_LIMIT, crate::user_config::OFFSET_LIMIT);
                    self.lyrics.choices.set_offset(&t, ms);
                    self.lyrics.song_offset = None;
                }
            }
            LyricsAction::Translation(m) => self.set_config("lyrics.translation", m.into()),
            LyricsAction::TranslateTo(t) => self.set_config("lyrics.translate_to", t.trim().to_string().into()),
            LyricsAction::Translator(e) => self.set_config("lyrics.translator", e.into()),
            LyricsAction::RetryTranslation => self.lyrics.translation.key.clear(),
            LyricsAction::ClearCache => {
                match self.lyrics.cache.clear() {
                    Ok(()) => {}
                    Err(e) => {
                        log::warn!("clearing saved lyrics: {e}");
                        self.toast("Couldn't clear saved lyrics. Try again.")
                    }
                }
                self.lyrics.nonce += 1;
            }
        }
    }

    fn picker_candidates(&self) -> Vec<Candidate> {
        match &self.lyrics.picker {
            Some(Picker { results: Some(Ok(c)), .. }) => (**c).clone(),
            _ => match &self.lyrics.state.status {
                Status::Done(res) => match &res.outcome {
                    Outcome::Ambiguous { candidates, .. } => candidates.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            },
        }
    }

    pub fn lyrics_answered(&mut self, generation: u64, resolved: Resolved) {
        self.lyrics.state.finish(generation, resolved);
    }

    pub fn lyrics_search_answered(&mut self, generation: u64, result: Result<Vec<Candidate>, String>) {
        if let Some(p) = self.lyrics.picker.as_mut().filter(|p| p.generation == generation) {
            p.results = Some(result.map(Arc::new));
        }
    }

    // ------------------------------------------------------------ translation

    /// Desktop `sync_translation` (translate_app.rs): the provider's own, a saved one,
    /// Apple's on-device engine (the Translation framework here), then the assistant's model.
    pub fn sync_translation(&mut self) {
        let cfg = self.config.lyrics().clone();
        if cfg.translation == Mode::Off || !self.lyrics.visible {
            return;
        }
        let Status::Done(res) = &self.lyrics.state.status else { return };
        let Outcome::Found { lyrics, .. } = &res.outcome else { return };
        let lyrics = lyrics.clone();
        let target = translate::target(&cfg.translate_to);
        let k = tkey(&lyrics, &target, cfg.translator, self.ai_ready().is_ok());
        if self.lyrics.translation.key == k {
            return;
        }
        // The assistant may translate, but its model list isn't loaded yet
        // (the panel hasn't been open): load it and come back next tick.
        let wants_assistant = matches!(cfg.translator, Engine::Auto | Engine::Assistant) && !(cfg.translator == Engine::Auto && translate::apple_available());
        if wants_assistant && self.ai_ready().is_ok() && self.ai_model().is_none() && lyrics.translations.is_empty() {
            match self.ai.models {
                crate::ai::Models::Unknown => self.ai_fetch_models(),
                crate::ai::Models::Loading => {}
                _ => self.ai.models_for.clear(),
            }
            if matches!(self.ai.models, crate::ai::Models::Loading) {
                return;
            }
        }
        if let Some(t) = self.lyrics.translation.task.take() {
            t.abort();
        }
        self.lyrics.translation = TranslationState { key: k.clone(), result: None, task: None };
        let lines: Vec<String> = lyrics.lines.iter().map(|l| l.text.clone()).collect();
        // The provider's own translation (Spotify has them for some songs).
        if cfg.translator == Engine::Auto
            && let Some((lang, tl)) = lyrics.translations.iter().find(|(l, _)| same_language(l, &target))
        {
            self.lyrics.translation.result =
                Some(Arc::new(Translation::Ready { from: lyrics.language.clone().unwrap_or_default(), to: lang.clone(), lines: tl.clone(), by: lyrics.source.provider.label().to_string() }));
            return;
        }
        // Already in that language?
        let hint = translate::source_language(lyrics.language.as_deref(), &lines);
        if let Some(h) = &hint
            && same_language(h, &target)
        {
            self.lyrics.translation.result = Some(Arc::new(Translation::Same { lang: h.clone() }));
            return;
        }
        if let Some(t) = self.lyrics.translation_store.get(stored(&k)).filter(|t| !t.contradicted_by(&lines)) {
            self.lyrics.translation.result = Some(Arc::new(t));
            return;
        }
        // Translate: Apple on this device first (in auto), then the assistant.
        let apple = matches!(cfg.translator, Engine::Auto | Engine::Apple) && translate::apple_available();
        let assistant = if matches!(cfg.translator, Engine::Auto | Engine::Assistant) && self.ai_ready().is_ok() {
            match (self.ai_model(), self.ai_bearer_for_translation()) {
                // A whole song, translated (and Japanese or Chinese take three bytes a character).
                (Some(model), Some(bearer)) => Some((self.ai_client().with_max_text(64_000), model, self.ai_billing().kind(), bearer)),
                _ => None,
            }
        } else {
            None
        };
        let fallback = match cfg.translator {
            Engine::Apple if !translate::apple_available() => Translation::Unavailable { why: "On-device translation isn't available on this device.".into(), fix: None },
            Engine::Assistant | Engine::Auto if assistant.is_none() && !apple => Translation::Unavailable {
                // Shown also when the assistant is on but not signed in, or has no model yet.
                why: if translate::apple_available() {
                    "Set up the assistant (Settings › Assistant) to translate lyrics, or choose Apple's on-device translation.".into()
                } else {
                    "Lyrics are translated by the assistant here: set it up in Settings › Assistant.".into()
                },
                fix: Some(Fix::TurnOnAssistant),
            },
            _ => Translation::Failed("no translator answered".into()),
        };
        let tx = self.tx.clone();
        let task = self.rt.spawn(async move {
            let mut result: Option<Translation> = None;
            if apple {
                let (l, h, t) = (lines.clone(), hint.clone(), target.clone());
                let r = tokio::task::spawn_blocking(move || translate::apple(&l, h.as_deref(), &t)).await.unwrap_or_else(|e| Translation::Failed(e.to_string()));
                result = Some(r);
            }
            let done = matches!(result, Some(Translation::Ready { .. } | Translation::Same { .. }));
            if !done && let Some((client, model, kind, bearer)) = assistant {
                let mut body = serde_json::json!({
                    "model": model,
                    "instructions": translate::assistant_instructions(&target),
                    "input": [{ "role": "user", "content": translate::assistant_input(&lines) }],
                    "store": false,
                    "stream": true,
                });
                if kind == crate::ai::openai::BillingKind::ApiKey {
                    body["max_output_tokens"] = serde_json::json!(8000);
                }
                let r = match bearer().await {
                    Ok(token) => match client.stream(&token, &body, |_| {}).await {
                        Ok(done) if done.truncated => Translation::Failed("the translation was too long to finish".into()),
                        Ok(done) => translate::parse_assistant(&done.text, &lines, &target),
                        Err(e) => Translation::Failed(e.message(kind)),
                    },
                    Err(e) => Translation::Failed(e.to_string()),
                };
                // Apple's "needs a download" stays the answer if the assistant also fails.
                if matches!(r, Translation::Ready { .. } | Translation::Same { .. }) || result.is_none() {
                    result = Some(r);
                }
            }
            tx.send(Msg::LyricsTranslated { key: k, result: result.unwrap_or(fallback) });
        });
        self.lyrics.translation.task = Some(task.abort_handle());
    }

    pub fn lyrics_translated(&mut self, key: String, result: Translation) {
        if result.keep() {
            let (store, k, t) = (self.lyrics.translation_store.clone(), stored(&key).to_string(), result.clone());
            std::thread::spawn(move || store.put(&k, &t));
        }
        if self.lyrics.translation.key != key {
            return;
        }
        self.lyrics.translation.task = None;
        self.lyrics.translation.result = Some(Arc::new(result));
    }

    // ------------------------------------------------------------ the model

    pub fn publish_lyrics(&mut self, events: &mut Vec<CoreEvent>) {
        let m = self.lyrics_model();
        if self.lyrics.published.as_ref() != Some(&m) {
            self.lyrics.published = Some(m.clone());
            events.push(CoreEvent::Lyrics { lyrics: m });
        }
    }

    fn lyrics_model(&mut self) -> LyricsModel {
        let cfg = self.config.lyrics().clone();
        let global = cfg.offset_ms;
        let song = self.lyrics_song_offset();
        let track = self.lyrics_track();
        let mut m = LyricsModel {
            key: track.as_ref().map(|t| t.key.clone()).unwrap_or_default(),
            title: track.as_ref().map(|t| t.title.clone()).unwrap_or_default(),
            artists: track.as_ref().map(|t| t.artists.join(", ")).unwrap_or_default(),
            duration_ms: track.as_ref().map(|t| t.duration_ms).unwrap_or(0),
            state: LyricsPhase::NothingPlaying,
            header: "lyrics".into(),
            timing: LyricsTiming::Plain,
            lines: Vec::new(),
            provenance: String::new(),
            detail: String::new(),
            candidates: Vec::new(),
            picker: None,
            global_offset_ms: global,
            song_offset_ms: song,
            translation_mode: match cfg.translation {
                Mode::Off => TranslationMode::Off,
                Mode::Below => TranslationMode::Below,
                Mode::Only => TranslationMode::Only,
            },
            translation: None,
            provider_setting: cfg.provider.name().to_string(),
            credits: Vec::new(),
            pending: false,
        };
        if track.is_none() {
            return m;
        }
        m.picker = self.lyrics.picker.as_ref().map(|p| PickerModel {
            query: p.query.clone(),
            loading: p.results.is_none(),
            error: p.results.as_ref().and_then(|r| r.as_ref().err().map(|e| plain_lyrics_error(e))),
            results: p.results.as_ref().and_then(|r| r.as_ref().ok()).map(|c| c.iter().map(|c| candidate_model(c, m.duration_ms)).collect()).unwrap_or_default(),
        });
        let res = match &self.lyrics.state.status {
            Status::Idle | Status::Loading => {
                let provider = match cfg.provider {
                    ProviderSetting::Only(p) => p.label(),
                    ProviderSetting::Auto => "LRCLIB",
                };
                m.state = LyricsPhase::Loading { provider: provider.into() };
                return m;
            }
            Status::Done(res) => res.clone(),
        };
        match &res.outcome {
            Outcome::Found { lyrics, how } => {
                m.header = format!("lyrics / {}", lyrics.timing.label());
                m.timing = match lyrics.timing {
                    Timing::Plain => LyricsTiming::Plain,
                    Timing::Line => LyricsTiming::Line,
                    Timing::Word => LyricsTiming::Word,
                    Timing::Syllable => LyricsTiming::Syllable,
                };
                m.lines = lines_model(lyrics);
                m.pending = res.pending && lyrics.word_coverage() < 0.8;
                // Where the lyrics are from, before the header (desktop `credits`).
                m.credits = lyrics.named_by().iter().map(|c| CreditModel { text: c.text.clone(), url: c.url.clone() }).collect();
                // Footer: "{exact match|matched by search|your pick}[ · {first} had none][ · {lang}]".
                let mut prov = Vec::new();
                prov.push(
                    match how {
                        Match::Exact => "exact match",
                        Match::Search => "matched by search",
                        Match::Manual => "your pick",
                    }
                    .to_string(),
                );
                if res.fell_back()
                    && let Some((first, _)) = res.tried.first()
                {
                    prov.push(format!("{} had none", first.name()));
                }
                if let Some(l) = &lyrics.language {
                    prov.push(crate::lyrics::translate::language_name(l));
                }
                m.provenance = prov.join(" · ");
                let src = &lyrics.source;
                m.detail = format!(
                    "Lyrics from {}: “{}” — {}{} · {}.\nLyrics belong to their rights holders and are shown here for personal use.",
                    src.provider.label(),
                    src.title,
                    src.artist,
                    if src.album.is_empty() { String::new() } else { format!(" · {}", src.album) },
                    model::fmt_ms(src.duration_ms)
                );
                m.state = LyricsPhase::Found;
                if cfg.translation != Mode::Off {
                    m.translation = Some(self.translation_model(lyrics));
                }
            }
            Outcome::Instrumental { .. } => {
                m.header = "lyrics / instrumental".into();
                m.state = LyricsPhase::Instrumental;
            }
            Outcome::NotFound { provider } => {
                let others: Vec<String> = res.tried.iter().filter(|(p, _)| p != provider).map(|(p, short)| format!("{}: {short}", p.label())).collect();
                m.state = LyricsPhase::NotFound { provider: provider.label().into(), others: others.join(" · ") };
            }
            Outcome::Ambiguous { candidates, .. } => {
                m.header = "lyrics / choose".into();
                m.candidates = candidates.iter().map(|c| candidate_model(c, m.duration_ms)).collect();
                m.state = LyricsPhase::Ambiguous;
            }
            Outcome::Unavailable { provider, reason } => {
                m.header = "lyrics / unavailable".into();
                m.state = LyricsPhase::Unavailable { provider: provider.label().into(), reason: reason.clone() };
            }
            Outcome::Failed { provider, error } => m.state = LyricsPhase::Failed { provider: provider.label().into(), error: plain_lyrics_error(error) },
        }
        m
    }

    fn translation_model(&self, lyrics: &Lyrics) -> TranslationModel {
        let cfg = self.config.lyrics();
        let target = translate::target(&cfg.translate_to);
        let key = tkey(lyrics, &target, cfg.translator, self.ai_ready().is_ok());
        let result = if self.lyrics.translation.key == key { self.lyrics.translation.result.clone() } else { None };
        let mut t = TranslationModel { working: false, lines: Vec::new(), note: String::new(), fix: None, failed: false, target: target.clone() };
        match result.as_deref() {
            None => {
                t.working = true;
                t.note = "translating…".into();
            }
            Some(Translation::Ready { from, lines, by, .. }) => {
                t.lines = lines.clone();
                t.note = if from.is_empty() { format!("translated by {by}") } else { format!("from {} · {by}", translate::language_name(from)) };
            }
            Some(Translation::Same { lang }) => t.note = format!("already in {}", translate::language_name(lang)),
            Some(Translation::Unavailable { why, fix }) => {
                t.note = why.clone();
                t.fix = fix.map(|f| match f {
                    Fix::DownloadLanguage => TranslationFix::DownloadLanguage,
                    Fix::TurnOnAssistant => TranslationFix::TurnOnAssistant,
                });
            }
            Some(Translation::Failed(e)) => {
                log::warn!("lyrics translation: {e}");
                t.note = "Couldn't translate these lyrics.".into();
                t.failed = true;
            }
        }
        t
    }
}

fn candidate_model(c: &Candidate, track_ms: u64) -> CandidateModel {
    let r = &c.record;
    let diff = r.duration_ms.abs_diff(track_ms);
    let length = if track_ms == 0 || r.duration_ms == 0 {
        String::new()
    } else if diff <= 2000 {
        "same length".into()
    } else {
        format!("{}{} s", if r.duration_ms > track_ms { "+" } else { "−" }, diff / 1000)
    };
    let mut meta = Vec::new();
    if !c.source.album.is_empty() {
        meta.push(c.source.album.clone());
    }
    let mut dur = model::fmt_ms(r.duration_ms);
    if !length.is_empty() {
        dur = format!("{dur} ({length})");
    }
    meta.push(dur);
    meta.push(if c.instrumental { "instrumental".into() } else { c.timing.label().to_string() });
    CandidateModel { title: format!("{} — {}", c.source.title, c.source.artist), meta: meta.join(" · "), preview: c.preview.clone() }
}

/// Lyrics requests from the app (desktop `Action::Lyrics*`).
#[derive(Clone, Debug)]
pub enum LyricsAction {
    Visible(bool),
    Retry,
    Pick {
        index: usize,
    },
    NoneFit,
    OpenPicker,
    Search(String),
    ClosePicker,
    Provider(String),
    /// The word timing services asked, in order (`lyrics.sources`).
    Sources(Vec<String>),
    /// Keeps (or with `None`, removes) the user's key for "spicy" or "musixmatch".
    Key {
        provider: String,
        key: Option<String>,
    },
    Fallback(bool),
    Offset(i64),
    SongOffset(i64),
    Translation(String),
    TranslateTo(String),
    Translator(String),
    RetryTranslation,
    ClearCache,
}

impl From<LyricsAction> for Action {
    fn from(a: LyricsAction) -> Self {
        Action::Lyrics(a)
    }
}

// ------------------------------------------------------------------ records

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LyricsTiming {
    Plain,
    Line,
    Word,
    Syllable,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LyricToken {
    /// Carries its own trailing space.
    pub text: String,
    pub start_ms: u64,
    /// When it's sung through (worked out here, so the view only divides).
    pub end_ms: u64,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LyricVoice {
    Lead,
    /// A duet's other singer: drawn on the other side.
    Other,
    Group,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LyricBacking {
    pub text: String,
    pub tokens: Vec<LyricToken>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    /// The line is lit from `start_ms` until this (shared `Lyrics::line_end`):
    /// lines of different voices can be lit at once.
    pub active_end_ms: u64,
    /// Empty: an instrumental gap (drawn as ♪) or a stanza break in plain lyrics.
    pub text: String,
    /// Word or syllable timing when the provider has it; empty for whole-line timing.
    pub tokens: Vec<LyricToken>,
    pub voice: LyricVoice,
    pub backing: Option<LyricBacking>,
    /// Right-to-left text.
    pub rtl: bool,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct CreditModel {
    pub text: String,
    /// Where the credit links (lrc.red asks for a link back).
    pub url: Option<String>,
}

/// The lyrics as the view draws them: every end worked out by the shared
/// timing rules, so the view's per-frame work is a comparison and a division.
pub fn lines_model(lyrics: &Lyrics) -> Vec<LyricLine> {
    let tokens = |t: &[crate::lyrics::Token], end: u64| -> Vec<LyricToken> {
        (0..t.len()).map(|k| LyricToken { text: t[k].text.clone(), start_ms: t[k].start_ms, end_ms: crate::lyrics::token_end(t, k, end) }).collect()
    };
    lyrics
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let end = lyrics.line_end(i);
            LyricLine {
                start_ms: l.start_ms,
                end_ms: l.end_ms,
                active_end_ms: end,
                text: l.text.clone(),
                tokens: tokens(&l.tokens, end),
                voice: match l.voice {
                    Voice::Lead => LyricVoice::Lead,
                    Voice::Other => LyricVoice::Other,
                    Voice::Group => LyricVoice::Group,
                },
                backing: l.backing.as_ref().map(|b| LyricBacking { text: b.text.clone(), tokens: tokens(&b.tokens, end) }),
                rtl: crate::lyrics::is_rtl(&l.text),
            }
        })
        .collect()
}

#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum LyricsPhase {
    NothingPlaying,
    Loading { provider: String },
    Found,
    Instrumental,
    NotFound { provider: String, others: String },
    Ambiguous,
    Unavailable { provider: String, reason: String },
    Failed { provider: String, error: String },
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct CandidateModel {
    /// "title — artist"
    pub title: String,
    /// "album · 3:45 (same length) · line sync"
    pub meta: String,
    pub preview: String,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct PickerModel {
    pub query: String,
    pub loading: bool,
    pub error: Option<String>,
    pub results: Vec<CandidateModel>,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TranslationMode {
    Off,
    Below,
    Only,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TranslationFix {
    /// Apple needs the language pair downloaded (the app offers the system's download sheet).
    DownloadLanguage,
    TurnOnAssistant,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct TranslationModel {
    pub working: bool,
    /// One per lyrics line (empty: none for that line), when ready.
    pub lines: Vec<String>,
    /// "translating…", "from Japanese · Apple, on this device", "already in English", why it isn't available…
    pub note: String,
    pub fix: Option<TranslationFix>,
    pub failed: bool,
    pub target: String,
}

/// What the lyrics view shows for the playing song.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LyricsModel {
    pub key: String,
    pub title: String,
    pub artists: String,
    pub duration_ms: u64,
    pub state: LyricsPhase,
    /// "lyrics / line sync", "lyrics / choose"…
    pub header: String,
    pub timing: LyricsTiming,
    pub lines: Vec<LyricLine>,
    /// "exact match · English" (the source is in `credits`)
    pub provenance: String,
    /// The record it came from, and the rights note.
    pub detail: String,
    /// Several versions match: choose one.
    pub candidates: Vec<CandidateModel>,
    /// The manual picker ("wrong lyrics?"), when open.
    pub picker: Option<PickerModel>,
    /// config.toml `lyrics.offset_ms` and this song's own; positive shows lyrics earlier.
    pub global_offset_ms: i64,
    pub song_offset_ms: i64,
    pub translation_mode: TranslationMode,
    pub translation: Option<TranslationModel>,
    /// "auto", "lrclib", "spotify" or a word timing service ("lrcred"…).
    pub provider_setting: String,
    /// Where the lyrics are from (before the header), linked where the service asks for credit.
    pub credits: Vec<CreditModel>,
    /// Line-timed lyrics are showing while the word timing services are still looking.
    pub pending: bool,
}

/// Why lyrics couldn't load, in plain words (the details go to the log).
fn plain_lyrics_error(e: &str) -> String {
    if e.starts_with("LRCLIB is busy") {
        return e.to_string();
    }
    log::warn!("lyrics: {e}");
    if e.starts_with("couldn't reach") || crate::errors::offline(&e.to_lowercase()) { "Lyrics need a connection.".into() } else { "The lyrics service isn't answering. Try again later.".into() }
}
