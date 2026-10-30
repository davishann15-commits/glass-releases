//! `~/.config/glass/config.toml`: settings that belong in a file you can edit
//! (`[lyrics]` and `[assistant]`). Works like theme.toml: optional, watched for
//! changes, validated, and edited in place so comments and other keys survive.

use crate::lyrics::ProviderId;
use crate::lyrics::service::{DEFAULT_SOURCES, ProviderSetting};
use crate::lyrics::translate::{Engine, Mode};
use crate::theme_config::{config_dir, file_stamp, write_atomic};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use toml_edit::{DocumentMut, Item, Value};

/// Written out (with its comments) the first time Glass edits the file.
pub const DEFAULT_FILE: &str = include_str!("../config.example.toml");

pub const OFFSET_LIMIT: i64 = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub struct LyricsConfig {
    pub provider: ProviderSetting,
    pub fallback: bool,
    /// The word timing services asked in auto, in order of preference.
    pub sources: Vec<ProviderId>,
    pub offset_ms: i64,
    /// Sent to LRCLIB in the User-Agent, if set.
    pub contact: String,
    /// Original, original + translation, or translated.
    pub translation: Mode,
    /// "system" or a language code.
    pub translate_to: String,
    pub translator: Engine,
}

impl Default for LyricsConfig {
    fn default() -> Self {
        Self {
            provider: ProviderSetting::Auto,
            fallback: true,
            sources: DEFAULT_SOURCES.to_vec(),
            offset_ms: 0,
            contact: String::new(),
            translation: Mode::Off,
            translate_to: "system".into(),
            translator: Engine::Auto,
        }
    }
}

/// The assistant's preferences. Secrets (tokens, API key) are never here.
#[derive(Clone, Debug, PartialEq)]
pub struct AssistantConfig {
    /// On by default; it does nothing until the user signs in to OpenAI or adds a key.
    pub enabled: bool,
    /// "chatgpt" (ChatGPT plan) or "api_key" (the user's own key, billed separately).
    pub billing: String,
    /// The chosen model's slug; empty until chosen.
    pub model: String,
    /// The one-time "You're using your ChatGPT plan" message was shown.
    pub welcomed: bool,
    /// The assistant may use the user's Spotify library, history, search and
    /// playback (once the assistant itself is on).
    pub spotify: bool,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self { enabled: true, billing: "chatgpt".into(), model: String::new(), welcomed: false, spotify: true }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct UserConfig {
    pub lyrics: LyricsConfig,
    pub assistant: AssistantConfig,
}

const KEYS: &[&str] = &[
    "lyrics.provider",
    "lyrics.fallback",
    "lyrics.sources",
    "lyrics.offset_ms",
    "lyrics.contact",
    "lyrics.translation",
    "lyrics.translate_to",
    "lyrics.translator",
    "assistant.enabled",
    "assistant.billing",
    "assistant.model",
    "assistant.welcomed",
    "assistant.spotify",
];

fn check(key: &str, item: &Item) -> Result<(), String> {
    match key {
        "lyrics.provider" => item
            .as_str()
            .and_then(ProviderSetting::parse)
            .map(|_| ())
            .ok_or_else(|| format!("{key}: expected \"auto\" or one of \"lrclib\", \"spotify\", \"lrcred\", \"unison\", \"netease\", \"spicy\", \"musixmatch\"")),
        "lyrics.sources" => sources(item).map(|_| ()).ok_or_else(|| format!("{key}: expected a list of \"lrcred\", \"qobuzify\", \"unison\", \"netease\", \"lrcmux\", \"spicy\" or \"musixmatch\"")),
        "lyrics.fallback" => item.as_bool().map(|_| ()).ok_or_else(|| format!("{key}: expected true or false")),
        "lyrics.offset_ms" => match item.as_integer() {
            Some(n) if n.abs() <= OFFSET_LIMIT => Ok(()),
            _ => Err(format!("{key}: expected a whole number of milliseconds between -{OFFSET_LIMIT} and {OFFSET_LIMIT}")),
        },
        "lyrics.contact" => match item.as_str() {
            Some(s) if s.chars().count() <= 200 && !s.chars().any(char::is_control) => Ok(()),
            _ => Err(format!("{key}: expected text on one line, at most 200 characters")),
        },
        "lyrics.translation" => item.as_str().and_then(Mode::parse).map(|_| ()).ok_or_else(|| format!("{key}: expected \"off\", \"below\" or \"only\"")),
        "lyrics.translator" => item.as_str().and_then(Engine::parse).map(|_| ()).ok_or_else(|| format!("{key}: expected \"auto\", \"apple\" or \"assistant\"")),
        "lyrics.translate_to" => match item.as_str().map(str::trim) {
            Some(s) if s == "system" || (2..=3).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphabetic()) => Ok(()),
            _ => Err(format!("{key}: expected \"system\" or a language code like \"en\"")),
        },
        "assistant.enabled" | "assistant.welcomed" | "assistant.spotify" => item.as_bool().map(|_| ()).ok_or_else(|| format!("{key}: expected true or false")),
        "assistant.billing" => match item.as_str() {
            Some("chatgpt" | "api_key") => Ok(()),
            _ => Err(format!("{key}: expected \"chatgpt\" or \"api_key\"")),
        },
        "assistant.model" => match item.as_str() {
            Some(s) if s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-:".contains(c)) => Ok(()),
            _ => Err(format!("{key}: expected a model name such as \"gpt-6-luna\"")),
        },
        _ => Ok(()),
    }
}

/// `lyrics.sources`: word timing services, each once (LRCLIB and Spotify are
/// the line-timed chain, not sources).
fn sources(item: &Item) -> Option<Vec<ProviderId>> {
    let mut out = Vec::new();
    for v in item.as_array()? {
        let p = ProviderId::parse(v.as_str()?).filter(|p| ProviderId::WORD.contains(p))?;
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Some(out)
}

/// `lyrics.sources` as config.toml writes it.
pub fn sources_value(list: &[ProviderId]) -> Value {
    Value::Array(list.iter().map(|p| p.key()).collect())
}

/// Reads the file. Every bad value is reported; unknown keys are warnings.
pub fn parse(text: &str) -> Result<(UserConfig, Vec<String>), String> {
    let doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| format!("config.toml isn't valid TOML: {}", e.to_string().trim()))?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut c = UserConfig::default();
    for (table, item) in doc.iter() {
        let Some(t) = item.as_table_like() else {
            warnings.push(format!("unknown key {table} (ignored)"));
            continue;
        };
        for (field, v) in t.iter() {
            let key = format!("{table}.{field}");
            if !KEYS.contains(&key.as_str()) {
                warnings.push(format!("unknown key {key} (ignored)"));
                continue;
            }
            if let Err(e) = check(&key, v) {
                errors.push(e);
                continue;
            }
            match key.as_str() {
                "lyrics.provider" => c.lyrics.provider = v.as_str().and_then(ProviderSetting::parse).unwrap_or(ProviderSetting::Auto),
                "lyrics.fallback" => c.lyrics.fallback = v.as_bool().unwrap_or(true),
                "lyrics.sources" => c.lyrics.sources = sources(v).unwrap_or_default(),
                "lyrics.offset_ms" => c.lyrics.offset_ms = v.as_integer().unwrap_or(0),
                "lyrics.contact" => c.lyrics.contact = v.as_str().unwrap_or("").trim().to_string(),
                "lyrics.translation" => c.lyrics.translation = v.as_str().and_then(Mode::parse).unwrap_or_default(),
                "lyrics.translate_to" => c.lyrics.translate_to = v.as_str().unwrap_or("system").trim().to_string(),
                "lyrics.translator" => c.lyrics.translator = v.as_str().and_then(Engine::parse).unwrap_or_default(),
                "assistant.enabled" => c.assistant.enabled = v.as_bool().unwrap_or(true),
                "assistant.welcomed" => c.assistant.welcomed = v.as_bool().unwrap_or(false),
                "assistant.spotify" => c.assistant.spotify = v.as_bool().unwrap_or(true),
                "assistant.billing" => c.assistant.billing = v.as_str().unwrap_or("chatgpt").to_string(),
                "assistant.model" => c.assistant.model = v.as_str().unwrap_or("").to_string(),
                _ => {}
            }
        }
    }
    if errors.is_empty() { Ok((c, warnings)) } else { Err(errors.join("\n")) }
}

const POLL: Duration = Duration::from_millis(500);

pub struct ConfigStore {
    path: PathBuf,
    config: UserConfig,
    error: Option<String>,
    warnings: Vec<String>,
    stamp: Option<(SystemTime, u64)>,
    /// The file was found empty: as in theme.toml, it's taken as empty only if
    /// it's still that way at the next check (editors empty it first to save).
    empty: Option<(SystemTime, u64)>,
    checked: Instant,
}

impl ConfigStore {
    pub fn load() -> Self {
        Self::at(config_dir().join("config.toml"))
    }

    pub fn at(path: PathBuf) -> Self {
        let mut s = Self { path, config: UserConfig::default(), error: None, warnings: Vec::new(), stamp: None, empty: None, checked: Instant::now() };
        s.reload();
        s
    }

    pub fn assistant(&self) -> &AssistantConfig {
        &self.config.assistant
    }
    pub fn lyrics(&self) -> &LyricsConfig {
        &self.config.lyrics
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Re-reads the file if it changed on disk (checked at most twice a
    /// second). True when the settings in use changed.
    pub fn poll(&mut self) -> bool {
        if self.checked.elapsed() < POLL {
            return false;
        }
        self.checked = Instant::now();
        let stamp = file_stamp(&self.path);
        if stamp == self.stamp {
            return false;
        }
        if let Some(s) = stamp.filter(|s| s.1 == 0)
            && self.empty != Some(s)
        {
            self.empty = Some(s);
            return false;
        }
        self.empty = None;
        self.reload()
    }

    /// Reads the file now; a missing file means the defaults. On an error the
    /// last good settings stay in use.
    pub fn reload(&mut self) -> bool {
        self.stamp = file_stamp(&self.path);
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                self.error = Some(format!("{}: {e}", self.path.display()));
                return false;
            }
        };
        match parse(&text) {
            Ok((c, warnings)) => {
                self.error = None;
                self.warnings = warnings;
                let changed = c != self.config;
                self.config = c;
                changed
            }
            Err(e) => {
                log::warn!("config.toml: {e}");
                self.error = Some(e);
                false
            }
        }
    }

    /// Writes the commented default file if there is none yet.
    pub fn ensure_file(&self) -> Result<(), String> {
        if self.path.exists() { Ok(()) } else { write_atomic(&self.path, DEFAULT_FILE) }
    }

    /// Sets one key (e.g. `set("lyrics.provider", "lrclib".into())`), keeping
    /// comments and every other key. Refuses to touch a file that doesn't
    /// parse, and never writes a bad value.
    pub fn set(&mut self, key: &str, v: Value) -> Result<(), String> {
        if !KEYS.contains(&key) {
            return Err(format!("unknown key {key}"));
        }
        check(key, &Item::Value(v.clone()))?;
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_FILE.to_string(),
            Err(e) => return Err(format!("couldn't read {}: {e}", self.path.display())),
        };
        let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| format!("fix config.toml first — it isn't valid TOML: {}", e.to_string().trim()))?;
        let (table, field) = key.split_once('.').expect("keys are table.field");
        if !doc.contains_table(table) {
            doc[table] = toml_edit::table();
        }
        let mut v = v;
        if let Some(old) = doc[table].get(field).and_then(Item::as_value) {
            *v.decor_mut() = old.decor().clone();
        }
        doc[table][field] = Item::Value(v);
        let out = doc.to_string();
        parse(&out)?;
        write_atomic(&self.path, &out)?;
        self.reload();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (ConfigStore, PathBuf) {
        let dir = crate::config::test_dir(&format!("config-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        (ConfigStore::at(path.clone()), path)
    }

    #[test]
    fn defaults_and_the_example_file_agree() {
        let (c, w) = parse(DEFAULT_FILE).unwrap();
        assert_eq!(c, UserConfig::default());
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(parse("").unwrap().0, UserConfig::default(), "missing file = defaults");
    }

    #[test]
    fn values_are_read() {
        let (c, _) =
            parse("[lyrics]\nprovider = \"LRCLIB\"\nfallback = false\nsources = [\"netease\", \"lrcred\", \"netease\"]\noffset_ms = -250\ncontact = \" me@example.com \"\ntranslation = \"below\"\ntranslate_to = \"ja\"\ntranslator = \"assistant\"\n")
                .unwrap();
        assert_eq!(
            c.lyrics,
            LyricsConfig {
                provider: ProviderSetting::Only(ProviderId::Lrclib),
                fallback: false,
                sources: vec![ProviderId::NetEase, ProviderId::LrcRed],
                offset_ms: -250,
                contact: "me@example.com".into(),
                translation: crate::lyrics::translate::Mode::Below,
                translate_to: "ja".into(),
                translator: crate::lyrics::translate::Engine::Assistant
            }
        );
        assert!(parse("[lyrics]\ntranslation = \"sideways\"\ntranslate_to = \"klingon!\"\n").is_err());
        assert!(parse("[lyrics]\nsources = [\"lrclib\"]\n").is_err(), "LRCLIB is the line chain, not a word source");
        assert!(parse("[lyrics]\nsources = \"lrcred\"\n").is_err());
        assert_eq!(parse("[lyrics]\nsources = []\n").unwrap().0.lyrics.sources, vec![], "no word services at all");
    }

    #[test]
    fn invalid_values_are_all_reported() {
        let e = parse("[lyrics]\nprovider = \"genius\"\nfallback = \"yes\"\noffset_ms = 99999\ncontact = \"a\\nb\"\n").unwrap_err();
        for k in ["lyrics.provider", "lyrics.fallback", "lyrics.offset_ms", "lyrics.contact"] {
            assert!(e.contains(k), "{k} missing from: {e}");
        }
        assert!(parse("[lyrics\n").is_err());
        let (_, w) = parse("[lyrics]\nsource = \"x\"\n[other]\na = 1\n").unwrap();
        assert_eq!(w, vec!["unknown key lyrics.source (ignored)", "unknown key other.a (ignored)"]);
    }

    #[test]
    fn a_bad_edit_keeps_the_last_good_settings() {
        let (mut s, path) = store("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[lyrics]\nprovider = \"lrclib\"\n").unwrap();
        s.reload();
        assert_eq!(s.lyrics().provider, ProviderSetting::Only(ProviderId::Lrclib));
        std::fs::write(&path, "[lyrics]\nprovider = \"nope\"\n").unwrap();
        s.reload();
        assert_eq!(s.lyrics().provider, ProviderSetting::Only(ProviderId::Lrclib), "last good value stays");
        assert!(s.error().unwrap().contains("lyrics.provider"));
        // An in-app change refuses to overwrite the broken file.
        std::fs::write(&path, "[lyrics\n").unwrap();
        assert!(s.set("lyrics.offset_ms", 100.into()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[lyrics\n");
    }

    #[test]
    fn edits_keep_comments_and_other_settings() {
        let (mut s, path) = store("edit");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# mine\n[lyrics]\nprovider = \"auto\" # keep this note\nfuture_key = 1\n\n[other]\nx = \"y\"\n").unwrap();
        s.reload();
        s.set("lyrics.provider", "spotify".into()).unwrap();
        s.set("lyrics.offset_ms", 300.into()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# mine") && text.contains("provider = \"spotify\" # keep this note") && text.contains("future_key = 1") && text.contains("x = \"y\""), "{text}");
        assert_eq!((s.lyrics().provider, s.lyrics().offset_ms), (ProviderSetting::Only(ProviderId::Spotify), 300));
        s.set("lyrics.sources", sources_value(&[ProviderId::Unison])).unwrap();
        assert_eq!(s.lyrics().sources, vec![ProviderId::Unison]);
        assert!(std::fs::read_to_string(&path).unwrap().contains("sources = [\"unison\"]"));
        assert!(s.set("lyrics.offset_ms", 50_000.into()).is_err(), "out of range");
        assert!(s.set("lyrics.provider", "genius".into()).is_err());
        assert_eq!(s.lyrics().offset_ms, 300);
    }

    #[test]
    fn first_edit_writes_the_commented_file() {
        let (mut s, path) = store("first");
        s.set("lyrics.fallback", false.into()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Where lyrics come from") && text.contains("fallback = false"), "{text}");
    }

    #[test]
    fn poll_notices_changes() {
        let (mut s, path) = store("poll");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[lyrics]\noffset_ms = 10\n").unwrap();
        s.checked = Instant::now() - POLL;
        assert!(s.poll());
        assert_eq!(s.lyrics().offset_ms, 10);
        s.checked = Instant::now() - POLL;
        assert!(!s.poll(), "unchanged");
    }

    #[test]
    fn a_file_emptied_mid_save_is_waited_out() {
        let (mut s, path) = store("empty");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[lyrics]\noffset_ms = 10\n").unwrap();
        s.reload();
        std::fs::write(&path, "").unwrap();
        s.checked = Instant::now() - POLL;
        assert!(!s.poll(), "an empty file is given one more check");
        assert_eq!(s.lyrics().offset_ms, 10);
        s.checked = Instant::now() - POLL;
        assert!(s.poll(), "still empty: now it's the defaults");
        assert_eq!(s.lyrics().offset_ms, 0);
    }
}
