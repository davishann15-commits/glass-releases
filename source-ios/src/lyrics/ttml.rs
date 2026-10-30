//! TTML, the timed XML Apple Music keeps its lyrics in, and the shape lrc.red
//! and Unison serve:
//!
//! ```text
//! <p begin="2:19.610" end="2:23.809" itunes:key="L27" ttm:agent="v1">
//!   <span begin="2:19.610" end="2:19.899">I'm</span> <span …>just</span>
//!   <span ttm:role="x-bg"><span begin="2:21.720" end="2:22.154">(Back</span> …</span>
//! </p>
//! ```
//!
//! A `<span>` is a word or a syllable; whitespace between two spans is the only
//! sign that they're separate words ("some" + "thing," sit flush). `ttm:agent`
//! names the voice (a duet's second singer is drawn on the other side), and a
//! span with the `x-bg` role holds the backing vocals sung under the line.
//! Lines of different voices can overlap in time. A line timed only as a whole
//! has its text straight in the `<p>`. Translations come in the head, keyed by
//! the line's `itunes:key` (`lrc:key` in lrc.red's copies), or inline as
//! `x-translation` spans. Prefixes vary between sources, so names are matched
//! by their local part.

use super::{Backing, Line, Timing, Token, Voice};
use quick_xml::events::{BytesStart, Event};
use std::collections::HashMap;

/// Anything bigger isn't one song's lyrics.
const MAX_DOC: usize = 2 << 20;

/// A TTML document, normalized.
#[derive(Debug)]
pub struct Doc {
    pub lines: Vec<Line>,
    pub timing: Timing,
    /// `xml:lang` of the document.
    pub language: Option<String>,
    /// `<body dur>`: the length of the recording the lyrics were timed to.
    pub duration_ms: Option<u64>,
    /// The head's (or inline) translations: (language, one entry per line).
    pub translations: Vec<(String, Vec<String>)>,
}

/// A begin, end or dur: seconds ("27.395"), minutes ("2:19.610"), hours
/// ("1:02:03.456"), or a number with a unit ("1.5s", "500ms").
pub fn clock(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, scale) = if let Some(v) = s.strip_suffix("ms") {
        (v, 1.0)
    } else if let Some(v) = s.strip_suffix('s') {
        (v, 1000.0)
    } else {
        (s, 1000.0)
    };
    let parts: Vec<&str> = num.split(':').collect();
    if num.is_empty() || parts.len() > 3 || (parts.len() > 1 && scale != 1000.0) {
        return None;
    }
    let mut total = 0.0f64;
    for (i, p) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let ok = !p.is_empty() && p.chars().all(|c| c.is_ascii_digit() || (last && c == '.')) && p.chars().filter(|&c| c == '.').count() <= 1;
        if !ok {
            return None;
        }
        let v: f64 = p.parse().ok()?;
        // Minutes and seconds after the first part stay under 60.
        if i > 0 && v >= 60.0 {
            return None;
        }
        total = total * 60.0 + v;
    }
    let ms = (total * scale).round();
    // Under 100 hours: anything longer is a broken file, not a song.
    (ms.is_finite() && (0.0..360_000_000.0).contains(&ms)).then_some(ms as u64)
}

/// An attribute by its local name ("agent" matches `ttm:agent`), unescaped.
fn attr(e: &BytesStart, local: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| {
            let k = a.key.as_ref();
            let k = k.rsplit(|&b| b == b':').next().unwrap_or(k);
            k == local.as_bytes()
        })
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

fn local(name: &[u8]) -> &[u8] {
    name.rsplit(|&b| b == b':').next().unwrap_or(name)
}

/// The words of one voice inside a line: the line's own, or its backing vocals.
#[derive(Default)]
struct Words {
    tokens: Vec<Token>,
    /// Everything said, for a line with no timed words.
    plain: String,
}

impl Words {
    /// Whitespace went by: the next word is a new one.
    fn space(&mut self) {
        if let Some(t) = self.tokens.last_mut()
            && !t.text.ends_with(' ')
        {
            t.text.push(' ');
        }
    }
    fn word(&mut self, raw: &str, start: u64, end: Option<u64>) {
        let text = raw.trim();
        if raw.starts_with(char::is_whitespace) {
            self.space();
        }
        if !text.is_empty() {
            self.tokens.push(Token { text: collapse(text), start_ms: start, end_ms: end.filter(|&e| e >= start) });
        }
        if raw.ends_with(char::is_whitespace) {
            self.space();
        }
    }
    /// Text in the line that no span timed (punctuation after a word): it
    /// belongs to the word before it.
    fn loose(&mut self, raw: &str) {
        let text = raw.trim();
        match self.tokens.last_mut() {
            Some(t) if !text.is_empty() && !raw.starts_with(char::is_whitespace) && !t.text.ends_with(' ') => t.text.push_str(&collapse(text)),
            _ => {}
        }
    }
    fn finish(mut self) -> (String, Vec<Token>) {
        if let Some(t) = self.tokens.last_mut() {
            t.text = t.text.trim_end().to_string();
        }
        let text = if self.tokens.is_empty() { collapse(self.plain.trim()) } else { self.tokens.iter().map(|t| t.text.as_str()).collect() };
        (text, self.tokens)
    }
}

/// Runs of whitespace (and line breaks from pretty-printing) as one space.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One `<p>` being read.
#[derive(Default)]
struct Para {
    begin: Option<u64>,
    end: Option<u64>,
    agent: Option<String>,
    key: Option<String>,
    main: Words,
    backing: Option<Words>,
    /// Depth (in spans) of the open x-bg span.
    bg_depth: Option<usize>,
    /// The span being read as a word: (text, begin, end, depth).
    word: Option<(String, u64, Option<u64>, usize)>,
    /// An inline translation span: (language, text, depth).
    inline: Option<(String, String, usize)>,
    translated: Vec<(String, String)>,
}

impl Para {
    fn words(&mut self) -> &mut Words {
        match self.backing.as_mut() {
            Some(b) if self.bg_depth.is_some() => b,
            _ => &mut self.main,
        }
    }
}

/// A `<text for=…>` of a head translation being read.
struct HeadText {
    lang: String,
    key: String,
    text: String,
}

/// Parses a TTML document. Malformed timing is dropped rather than guessed;
/// a document with no lines is an error.
pub fn parse(src: &str) -> Result<Doc, String> {
    if src.len() > MAX_DOC {
        return Err("TTML too large".into());
    }
    let mut reader = quick_xml::Reader::from_str(src);
    reader.config_mut().check_end_names = false;
    let mut language = None;
    let mut duration_ms = None;
    let mut agents: HashMap<String, String> = HashMap::new();
    // The agent inherited from <body> and <div>s, innermost last.
    let mut inherited: Vec<Option<String>> = Vec::new();
    let mut paras: Vec<Para> = Vec::new();
    let mut para: Option<Para> = None;
    let mut spans = 0usize;
    // Head translations: language → key → text.
    let mut head: Vec<(String, HashMap<String, String>)> = Vec::new();
    let mut translation_lang: Option<String> = None;
    let mut head_text: Option<HeadText> = None;

    loop {
        let ev = reader.read_event().map_err(|e| format!("bad TTML: {e}"))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                let name = e.name();
                match local(name.as_ref()) {
                    b"tt" => language = attr(e, "lang").map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()),
                    b"agent" if para.is_none() => {
                        if let Some(id) = attr(e, "id") {
                            agents.insert(id, attr(e, "type").unwrap_or_default());
                        }
                    }
                    b"body" | b"div" => {
                        if local(name.as_ref()) == b"body" {
                            duration_ms = attr(e, "dur").and_then(|d| clock(&d));
                        }
                        if !empty {
                            let a = attr(e, "agent").or_else(|| inherited.last().cloned().flatten());
                            inherited.push(a);
                        }
                    }
                    b"p" if para.is_none() => {
                        let p = Para {
                            begin: attr(e, "begin").and_then(|t| clock(&t)),
                            end: attr(e, "end").and_then(|t| clock(&t)),
                            agent: attr(e, "agent").or_else(|| inherited.last().cloned().flatten()),
                            key: attr(e, "key"),
                            ..Default::default()
                        };
                        if empty {
                            paras.push(p);
                        } else {
                            para = Some(p);
                            spans = 0;
                        }
                    }
                    b"span" if para.is_some() && !empty => {
                        spans += 1;
                        let p = para.as_mut().unwrap();
                        let role = attr(e, "role").unwrap_or_default();
                        if p.inline.is_some() || p.word.is_some() {
                            // Ruby or the like inside a word: its text is part of the word.
                        } else if role == "x-bg" && p.bg_depth.is_none() {
                            p.bg_depth = Some(spans);
                            p.backing.get_or_insert_with(Words::default).space();
                        } else if role == "x-translation" || role.starts_with("x-roman") {
                            let lang = if role == "x-translation" { attr(e, "lang").unwrap_or_default() } else { String::new() };
                            p.inline = Some((lang, String::new(), spans));
                        } else if let Some(begin) = attr(e, "begin").and_then(|t| clock(&t)) {
                            p.word = Some((String::new(), begin, attr(e, "end").and_then(|t| clock(&t)), spans));
                        }
                    }
                    b"translation" if para.is_none() && !empty => translation_lang = Some(attr(e, "lang").unwrap_or_default()),
                    b"text" if para.is_none() && !empty => {
                        if let (Some(lang), Some(key)) = (&translation_lang, attr(e, "for")) {
                            head_text = Some(HeadText { lang: lang.clone(), key, text: String::new() });
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref e) => {
                let name = e.name();
                match local(name.as_ref()) {
                    b"body" | b"div" => {
                        inherited.pop();
                    }
                    b"p" => {
                        if let Some(p) = para.take() {
                            paras.push(p);
                        }
                    }
                    b"span" if para.is_some() => {
                        let p = para.as_mut().unwrap();
                        if p.word.as_ref().is_some_and(|w| w.3 == spans) {
                            let (text, begin, end, _) = p.word.take().unwrap();
                            p.words().word(&text, begin, end);
                        } else if p.inline.as_ref().is_some_and(|w| w.2 == spans) {
                            let (lang, text, _) = p.inline.take().unwrap();
                            if !lang.is_empty() && !text.trim().is_empty() {
                                p.translated.push((lang, collapse(&text)));
                            }
                        } else if p.bg_depth == Some(spans) {
                            p.bg_depth = None;
                            p.main.space();
                        }
                        spans = spans.saturating_sub(1);
                    }
                    b"translation" => translation_lang = None,
                    b"text" => {
                        if let Some(t) = head_text.take() {
                            let i = match head.iter().position(|(l, _)| *l == t.lang) {
                                Some(i) => i,
                                None => {
                                    head.push((t.lang.clone(), HashMap::new()));
                                    head.len() - 1
                                }
                            };
                            head[i].1.insert(t.key, collapse(&t.text));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref t) => {
                let s = t.decode().map_err(|e| format!("bad TTML text: {e}"))?;
                text(&mut para, &mut head_text, &s);
            }
            Event::CData(ref t) => {
                let s = String::from_utf8_lossy(t.as_ref()).into_owned();
                text(&mut para, &mut head_text, &s);
            }
            Event::GeneralRef(ref r) => {
                let c = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => {
                        let n = String::from_utf8_lossy(r.as_ref()).into_owned();
                        quick_xml::escape::resolve_predefined_entity(&n).unwrap_or("").to_string()
                    }
                };
                text(&mut para, &mut head_text, &c);
            }
            Event::Eof => break,
            _ => {}
        }
    }

    // Voices: the first person to sing leads; any other person is the duet's
    // other side; a group (or "other") agent is everyone together.
    let mut people: Vec<String> = Vec::new();
    for p in &paras {
        if let Some(a) = &p.agent
            && agents.get(a).is_none_or(|t| t == "person")
            && !people.contains(a)
        {
            people.push(a.clone());
        }
    }
    let voice = |a: &Option<String>| match a {
        Some(a) if agents.get(a).is_some_and(|t| !t.is_empty() && t != "person") => Voice::Group,
        Some(a) if people.len() > 1 && people.first() != Some(a) => Voice::Other,
        _ => Voice::Lead,
    };

    // Each line with its key (for the head's translations) and its inline translations.
    type Keyed = (Line, Option<String>, Vec<(String, String)>);
    let mut keyed: Vec<Keyed> = Vec::new();
    for p in paras {
        let v = voice(&p.agent);
        let (text, tokens) = p.main.finish();
        let backing = p
            .backing
            .map(|b| {
                let (text, mut tokens) = b.finish();
                if tokens.is_empty() {
                    let t = text.trim();
                    let t = t.strip_prefix('(').and_then(|t| t.strip_suffix(')')).unwrap_or(t);
                    return Backing { text: t.trim().to_string(), tokens };
                }
                unwrap_parens(&mut tokens);
                Backing { text: tokens.iter().map(|t| t.text.as_str()).collect::<String>().trim().to_string(), tokens }
            })
            .filter(|b| !b.text.is_empty());
        let Some(start) = p.begin.or_else(|| tokens.first().map(|t| t.start_ms)) else { continue };
        if text.is_empty() && backing.is_none() {
            continue;
        }
        let line = Line { start_ms: start, end_ms: p.end.filter(|&e| e > start), text, tokens, voice: v, backing: backing.map(Box::new) };
        keyed.push((line, p.key, p.translated));
    }
    if keyed.is_empty() {
        return Err("no lyrics lines in the TTML".into());
    }
    keyed.sort_by_key(|(l, _, _)| l.start_ms);

    // Translations: the head's by key, else the inline ones.
    let mut translations: Vec<(String, Vec<String>)> = Vec::new();
    for (lang, by_key) in &head {
        if lang.is_empty() || by_key.is_empty() {
            continue;
        }
        let lines: Vec<String> = keyed.iter().map(|(_, k, _)| k.as_ref().and_then(|k| by_key.get(k)).cloned().unwrap_or_default()).collect();
        if lines.iter().any(|l| !l.is_empty()) {
            translations.push((lang.to_lowercase(), lines));
        }
    }
    let inline_langs: Vec<String> = keyed.iter().flat_map(|(_, _, t)| t.iter().map(|(l, _)| l.to_lowercase())).fold(Vec::new(), |mut v, l| {
        if !v.contains(&l) {
            v.push(l);
        }
        v
    });
    for lang in inline_langs {
        if translations.iter().any(|(l, _)| *l == lang) {
            continue;
        }
        translations.push((lang.clone(), keyed.iter().map(|(_, _, t)| t.iter().find(|(l, _)| l.to_lowercase() == lang).map(|(_, s)| s.clone()).unwrap_or_default()).collect()));
    }

    let mut lines: Vec<Line> = keyed.into_iter().map(|(l, _, _)| l).collect();
    let timing = super::parse::settle_tokens(&mut lines);
    Ok(Doc { lines, timing, language, duration_ms, translations })
}

/// Text inside the document: a word, whitespace between words, or a translation.
fn text(para: &mut Option<Para>, head_text: &mut Option<HeadText>, s: &str) {
    if let Some(p) = para.as_mut() {
        if let Some((_, t, _)) = p.inline.as_mut() {
            t.push_str(s);
            return;
        }
        if let Some((t, ..)) = p.word.as_mut() {
            t.push_str(s);
            return;
        }
        let w = p.words();
        w.plain.push_str(s);
        if s.trim().is_empty() {
            if !s.is_empty() {
                w.space();
            }
        } else {
            w.loose(s);
        }
    } else if let Some(h) = head_text.as_mut() {
        h.text.push_str(s);
    }
}

/// Backing vocals are written in parentheses ("(Back", "know)"); they're shown
/// as backing vocals instead, so the parentheses go.
fn unwrap_parens(tokens: &mut [Token]) {
    let n = tokens.len();
    if n == 0 || !tokens[0].text.starts_with('(') || !tokens[n - 1].text.trim_end().ends_with(')') {
        return;
    }
    tokens[0].text.remove(0);
    let last = &mut tokens[n - 1].text;
    let trimmed = last.trim_end().len();
    last.truncate(trimmed);
    last.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like lrc.red's documents (namespaces, `lrc:key`, agents in the
    /// head, a duet, syllables sitting flush, backing vocals, overlapping
    /// lines); the words are placeholders, not a song.
    const DUET: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:lrc="http://lrc.red/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" lrc:timing="Word" xml:lang="en"><head><metadata><ttm:agent type="person" xml:id="v1"/><ttm:agent type="person" xml:id="v2"/><ttm:agent type="group" xml:id="v3"/><sourceMetadata xmlns="http://lrc.red/lyric-ttml-internal" leadingSilence="0"><translations/><songwriters><songwriter>A Writer</songwriter></songwriters></sourceMetadata></metadata></head><body dur="3:35.737" ttm:agent="v2"><div begin="15.572" end="34.457" lrc:songPart="Verse" ttm:agent="v1"><p begin="15.572" end="18.258" lrc:key="L1" ttm:agent="v1"><span begin="15.572" end="16.023">Tell</span> <span begin="16.023" end="16.265">me</span> <span begin="16.265" end="16.594">some</span><span begin="16.594" end="17.192">thing,</span> <span begin="17.192" end="18.258">friend</span></p><p begin="17.900" end="20.000" lrc:key="L2" ttm:agent="v2"><span begin="17.900" end="18.400">I'm</span> <span begin="18.400" end="19.100">here</span> <span ttm:role="x-bg"><span begin="19.000" end="19.400">(Right</span> <span begin="19.400" end="20.000">here)</span></span></p></div><div begin="25.0" end="30.0"><p begin="25.000" end="27.500" lrc:key="L3" ttm:agent="v3"><span begin="25.000" end="26.000">All</span> <span begin="26.000" end="27.500">together</span></p></div></body></tt>"#;

    #[test]
    fn words_syllables_voices_and_backing_vocals() {
        let d = parse(DUET).unwrap();
        assert_eq!(d.language.as_deref(), Some("en"));
        assert_eq!(d.duration_ms, Some(215_737));
        assert_eq!(d.lines.len(), 3);
        let l = &d.lines[0];
        assert_eq!(l.text, "Tell me something, friend", "flush spans make one word");
        let w: Vec<(&str, u64, Option<u64>)> = l.tokens.iter().map(|t| (t.text.as_str(), t.start_ms, t.end_ms)).collect();
        assert_eq!(w, vec![("Tell ", 15_572, Some(16_023)), ("me ", 16_023, Some(16_265)), ("some", 16_265, Some(16_594)), ("thing, ", 16_594, Some(17_192)), ("friend", 17_192, Some(18_258))]);
        assert_eq!(d.timing, Timing::Syllable, "a word split in two is syllable timing");
        assert_eq!((l.voice, d.lines[1].voice, d.lines[2].voice), (Voice::Lead, Voice::Other, Voice::Group));
        let b = d.lines[1].backing.as_deref().unwrap();
        assert_eq!(b.text, "Right here", "backing vocals lose their parentheses");
        assert_eq!(b.tokens.iter().map(|t| (t.text.as_str(), t.start_ms)).collect::<Vec<_>>(), vec![("Right ", 19_000), ("here", 19_400)]);
        assert_eq!(d.lines[1].text, "I'm here", "the backing vocals aren't part of the line");
        // The second voice comes in before the first has finished: both keep their times.
        assert_eq!((d.lines[0].end_ms, d.lines[1].start_ms), (Some(18_258), 17_900));
        assert!(!d.lines[0].tokens.is_empty() && !d.lines[1].tokens.is_empty(), "overlap doesn't cost the words their timing");
    }

    #[test]
    fn line_timed_documents_and_head_translations() {
        let doc = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" itunes:timing="Line" xml:lang="ja"><head><metadata><iTunesMetadata xmlns="http://music.apple.com/lyric-ttml-internal"><translations><translation type="subtitle" xml:lang="en"><text for="L1">The night &amp; the door</text><text for="L2">Knocking</text></translation></translations><transliterations><transliteration xml:lang="ja-Latn"><text for="L1">mayonaka</text></transliteration></transliterations></iTunesMetadata></metadata></head><body dur="297.0"><div><p begin="00:01.000" end="00:04.500" itunes:key="L1">真夜中の
        ドア</p><p begin="00:05.000" end="00:08.000" itunes:key="L2">叩き &lt;3</p></div></body></tt>"#;
        let d = parse(doc).unwrap();
        assert_eq!(d.timing, Timing::Line);
        assert_eq!(d.lines.iter().map(|l| (l.start_ms, l.text.as_str())).collect::<Vec<_>>(), vec![(1000, "真夜中の ドア"), (5000, "叩き <3")]);
        assert!(d.lines.iter().all(|l| l.tokens.is_empty()), "no words are invented");
        assert_eq!(d.translations, vec![("en".to_string(), vec!["The night & the door".to_string(), "Knocking".to_string()])], "transliterations aren't translations");
        assert_eq!(d.duration_ms, Some(297_000));
    }

    #[test]
    fn inline_translations_and_loose_punctuation() {
        let doc = r#"<tt><body><div><p begin="1" end="3"><span begin="1" end="2">Hola</span><span begin="2" end="3">mundo</span>! <span ttm:role="x-translation" xml:lang="en">Hello world!</span></p></div></body></tt>"#;
        let d = parse(doc).unwrap();
        assert_eq!(d.lines[0].text, "Holamundo!", "flush spans, untimed punctuation joins the word before it");
        assert_eq!(d.translations, vec![("en".to_string(), vec!["Hello world!".to_string()])]);
    }

    #[test]
    fn clocks() {
        assert_eq!(clock("27.395"), Some(27_395));
        assert_eq!(clock("2:19.610"), Some(139_610));
        assert_eq!(clock("00:37.097"), Some(37_097));
        assert_eq!(clock("1:02:03.456"), Some(3_723_456));
        assert_eq!(clock("1.5s"), Some(1_500));
        assert_eq!(clock("500ms"), Some(500));
        assert_eq!(clock(""), None);
        assert_eq!(clock("-1"), None);
        assert_eq!(clock("1:75.0"), None);
        assert_eq!(clock("soon"), None);
        assert_eq!(clock("1:2:3:4"), None);
    }

    #[test]
    fn broken_documents() {
        assert!(parse("<tt><body></body></tt>").is_err(), "no lines");
        assert!(parse("not xml at all").is_err());
        assert!(parse(&"<tt>".repeat(MAX_DOC)).is_err());
        // Words out of order lose their timing; the line keeps its own.
        let d = parse(r#"<tt><body><p begin="1" end="3"><span begin="2.5">b</span> <span begin="1">a</span></p></body></tt>"#).unwrap();
        assert!(d.lines[0].tokens.is_empty());
        assert_eq!((d.lines[0].text.as_str(), d.timing), ("b a", Timing::Line));
        // A span without a begin is text, not a timed word.
        let d = parse(r#"<tt><body><p begin="1"><span>loose</span> <span begin="2">word</span></p></body></tt>"#).unwrap();
        assert_eq!(d.lines[0].start_ms, 1000);
    }
}
