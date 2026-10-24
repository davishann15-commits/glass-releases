//! Lyrics translation on `App`: when translations are wanted and lyrics are
//! on screen, find one — the provider's own, a saved one, Apple's on-device
//! engine (macOS) or the assistant's model — in the background, once per
//! song and language.

use crate::app::{App, Msg};
use crate::lyrics::service::Status;
use crate::lyrics::translate::{self, Engine, Fix, Mode, Translation};
use crate::lyrics::{Lyrics, Outcome};
use serde_json::json;
use std::sync::Arc;

/// The translation for the lyrics on screen (`result` is `None` while working).
#[derive(Default)]
pub struct TranslationState {
    pub key: String,
    pub result: Option<Arc<Translation>>,
    /// The translation being made, stopped when another song's starts (the
    /// assistant would otherwise go on spending tokens on a song that's gone).
    pub task: Option<tokio::task::AbortHandle>,
}

/// What a translation depends on. The translator setting and whether the assistant can
/// answer are part of it, so turning the assistant on replaces "turn on the assistant".
fn key(lyrics: &Lyrics, target: &str, engine: Engine, assistant: bool) -> String {
    format!("{}|{}|{}|{target}|{}|{assistant}", lyrics.source.provider.name(), lyrics.source.id, lyrics.lines.len(), engine.name())
}

/// The part of a key a saved translation is filed under (the lyrics and the language).
fn stored(key: &str) -> &str {
    key.rsplitn(3, '|').nth(2).unwrap_or(key)
}

fn same_language(a: &str, b: &str) -> bool {
    translate::base_code(a) == translate::base_code(b)
}

impl App {
    /// The translation for `lyrics`, if translations are on and it's known.
    /// `Some(None)` while it's being worked out.
    pub fn lyrics_translation(&self, lyrics: &Lyrics) -> Option<Option<Arc<Translation>>> {
        let cfg = self.config.lyrics();
        if cfg.translation == Mode::Off {
            return None;
        }
        (self.translation.key == key(lyrics, &translate::target(&cfg.translate_to), cfg.translator, self.ai_ready().is_ok())).then(|| self.translation.result.clone())
    }

    /// Once per frame: start translating the lyrics on screen when wanted.
    pub(crate) fn sync_translation(&mut self) {
        let cfg = self.config.lyrics();
        if cfg.translation == Mode::Off || !self.lyrics_visible() {
            return;
        }
        let Status::Done(res) = &self.lyrics.status else { return };
        let Outcome::Found { lyrics, .. } = &res.outcome else { return };
        let target = translate::target(&cfg.translate_to);
        let k = key(lyrics, &target, cfg.translator, self.ai_ready().is_ok());
        if self.translation.key == k {
            return;
        }
        let (cfg, lyrics) = (cfg.clone(), lyrics.clone());
        // The assistant may translate, but its model list isn't loaded yet
        // (the panel hasn't been open): load it and come back next frame.
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
        if let Some(t) = self.translation.task.take() {
            t.abort();
        }
        self.translation = crate::translate_app::TranslationState { key: k.clone(), result: None, task: None };
        let lines: Vec<String> = lyrics.lines.iter().map(|l| l.text.clone()).collect();
        // The provider's own translation (Spotify has them for some songs).
        if cfg.translator == Engine::Auto
            && let Some((lang, tl)) = lyrics.translations.iter().find(|(l, _)| same_language(l, &target))
        {
            self.translation.result =
                Some(Arc::new(Translation::Ready { from: lyrics.language.clone().unwrap_or_default(), to: lang.clone(), lines: tl.clone(), by: lyrics.source.provider.label().to_string() }));
            return;
        }
        // Already in that language?
        let hint = lyrics.language.clone().or_else(|| translate::script_language(&lines).map(str::to_string));
        if let Some(h) = &hint
            && same_language(h, &target)
        {
            self.translation.result = Some(Arc::new(Translation::Same { lang: h.clone() }));
            return;
        }
        if let Some(t) = self.translation_store.get(stored(&k)) {
            self.translation.result = Some(Arc::new(t));
            return;
        }
        // Translate: Apple on this Mac first (in auto), then the assistant.
        let apple = matches!(cfg.translator, Engine::Auto | Engine::Apple) && translate::apple_available();
        let assistant = if matches!(cfg.translator, Engine::Auto | Engine::Assistant) && self.ai_ready().is_ok() {
            match (self.ai_model(), self.ai_bearer_for_translation()) {
                // A whole song, translated (and Japanese or Chinese take three bytes a character).
                (Some(model), Some(bearer)) => Some((self.ai_client_for_translation().with_max_text(64_000), model, self.ai_billing().kind(), bearer)),
                _ => None,
            }
        } else {
            None
        };
        let fallback = match cfg.translator {
            Engine::Apple if !translate::apple_available() => Translation::Unavailable {
                why: if cfg!(target_os = "macos") { "On-device translation isn't part of this build of Glass." } else { "Apple's on-device translation is only on macOS." }.into(),
                fix: None,
            },
            Engine::Assistant | Engine::Auto if assistant.is_none() && !apple => Translation::Unavailable {
                // Shown also when the assistant is on but not signed in, or has no model yet.
                why: if translate::apple_available() {
                    "Set up the assistant (Settings → Assistant) to translate lyrics, or choose Apple's on-device translation.".into()
                } else {
                    "Lyrics are translated by the assistant here: set it up in Settings → Assistant.".into()
                },
                fix: Some(Fix::TurnOnAssistant),
            },
            _ => Translation::Failed("no translator answered".into()),
        };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let task = self.rt.spawn(async move {
            let mut result: Option<Translation> = None;
            if apple {
                let (l, h, t) = (lines.clone(), hint.clone(), target.clone());
                let r = tokio::task::spawn_blocking(move || translate::apple(&l, h.as_deref(), &t)).await.unwrap_or_else(|e| Translation::Failed(e.to_string()));
                result = Some(r);
            }
            let done = matches!(result, Some(Translation::Ready { .. } | Translation::Same { .. }));
            if !done && let Some((client, model, kind, bearer)) = assistant {
                let mut body = json!({
                    "model": model,
                    "instructions": translate::assistant_instructions(&target),
                    "input": [{ "role": "user", "content": translate::assistant_input(&lines) }],
                    "store": false,
                    "stream": true,
                });
                if kind == crate::ai::openai::BillingKind::ApiKey {
                    body["max_output_tokens"] = json!(8000);
                }
                let r = match bearer().await {
                    Ok(token) => match client.stream(&token, &body, |_| {}).await {
                        Ok(done) if done.truncated => Translation::Failed("the translation was too long to finish".into()),
                        Ok(done) => translate::parse_assistant(&done.text, lines.len(), &target),
                        Err(e) => Translation::Failed(e.message(kind)),
                    },
                    Err(e) => Translation::Failed(e.to_string()),
                };
                // Apple's "needs a download" stays the answer if the assistant also fails.
                if matches!(r, Translation::Ready { .. } | Translation::Same { .. }) || result.is_none() {
                    result = Some(r);
                }
            }
            let _ = tx.send(Msg::LyricsTranslated { key: k, result: result.unwrap_or(fallback) });
            ctx.request_repaint();
        });
        self.translation.task = Some(task.abort_handle());
    }

    pub(crate) fn lyrics_translated(&mut self, key: String, result: Translation) {
        // One that finished just as the song changed is still kept for next time
        // (written off the UI thread).
        if result.keep() {
            let (store, k, t) = (self.translation_store.clone(), stored(&key).to_string(), result.clone());
            std::thread::spawn(move || store.put(&k, &t));
        }
        if self.translation.key != key {
            return;
        }
        self.translation.task = None;
        self.translation.result = Some(Arc::new(result));
    }
}
