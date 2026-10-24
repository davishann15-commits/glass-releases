//! Spotify's DJ ("X"). The engine plays the DJ's songs (see
//! vendor/librespot-connect/GLASS.md); this is the voice: each DJ song can
//! carry a line before it (`narration.intro.*`) or after it
//! (`narration.outro.*`), as SSML plus the voice to speak it in. Spotify's
//! clients have that spoken by Spotify's text-to-speech service
//! (`/client-tts/v1/fulfill`), which returns the clip as MP3.

use anyhow::{Context, Result, bail};
use librespot_core::Session;
use std::collections::HashMap;

/// The DJ, as a playlist URI.
pub const URI: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";

pub fn is_dj(uri: &str) -> bool {
    uri == URI
}

/// The DJ's cover, from Spotify's own DJ assets.
pub const COVER: &str = "https://lexicon-assets.spotifycdn.com/Your-DJ-Cover-Art-300.png";

/// Shared with the engine's hook: whether X speaks, and the line the song
/// playing ends with (said only if it plays to its end).
pub struct Shared {
    pub voice_on: std::sync::atomic::AtomicBool,
    pub pending_outro: std::sync::Mutex<Option<Line>>,
}

pub static SHARED: Shared = Shared { voice_on: std::sync::atomic::AtomicBool::new(true), pending_outro: std::sync::Mutex::new(None) };

/// A song was skipped or something else was chosen: its closing line isn't said.
pub fn forget_outro() {
    if let Ok(mut o) = SHARED.pending_outro.lock() {
        *o = None;
    }
}

/// Decides, as Connect starts a song, what X says first. Returns the lines
/// to say now (the last song's outro, this one's intro) and the ones to get
/// ready (this song's outro, the next song's intro).
pub fn on_track_start(t: &librespot_connect::glass::TrackStart) -> (Vec<Line>, Vec<Line>) {
    let intro = Line::from_metadata(t.metadata, "intro");
    let outro = Line::from_metadata(t.metadata, "outro");
    // The last song's closing line, only if it played to its end (a skip from
    // another Spotify app reaches Connect directly, without Glass's `forget_outro`).
    let before = SHARED.pending_outro.lock().ok().and_then(|mut o| std::mem::replace(&mut *o, outro.clone())).filter(|_| t.after_end);
    if !SHARED.voice_on.load(std::sync::atomic::Ordering::Relaxed) {
        return (Vec::new(), Vec::new());
    }
    let next_intro = t.next.and_then(|(_, m)| Line::from_metadata(m, "intro"));
    ([before, intro].into_iter().flatten().collect(), [outro, next_intro].into_iter().flatten().collect())
}

/// One of the DJ's lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub ssml: String,
    pub voice: u32,
    pub provider: u32,
    pub sample_rate: u32,
}

impl Line {
    /// The line said `when` ("intro" or "outro") for a song, from its metadata.
    pub fn from_metadata(m: &HashMap<String, String>, when: &str) -> Option<Self> {
        let get = |k: &str| m.get(&format!("narration.{when}.{k}")).map(String::as_str);
        let ssml = get("ssml").filter(|s| !s.trim().is_empty())?.to_string();
        // VOICE1 … VOICE40; the official client says VOICE1.
        let voice = get("voice").and_then(|v| v.strip_prefix("VOICE")).and_then(|n| n.parse().ok()).unwrap_or(1);
        let provider = match get("tts_provider").unwrap_or("SONANTIC_FAST") {
            "CLOUD_TTS" => 1,
            "READSPEAKER" => 2,
            "POLLY" => 3,
            "WELL_SAID" => 4,
            "SONANTIC_DEPRECATED" => 5,
            _ => 6,
        };
        let sample_rate = get("sample_rate").and_then(|r| r.parse().ok()).unwrap_or(44_100);
        Some(Self { ssml, voice, provider, sample_rate })
    }

    /// What X says, as plain text (for captions).
    pub fn text(&self) -> String {
        ssml_text(&self.ssml)
    }
}

/// SSML without its markup ("<entity …>Lynyrd Skynyrd</entity>" → "Lynyrd Skynyrd").
pub fn ssml_text(ssml: &str) -> String {
    let mut out = String::with_capacity(ssml.len());
    let mut in_tag = false;
    for c in ssml.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let unescaped = out.replace("&amp;", "&").replace("&apos;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">");
    let mut s = String::with_capacity(unescaped.len());
    for w in unescaped.split_whitespace() {
        // "<entity>Drake</entity>'s": the possessive joins the name again (a
        // quote opening after a space stays apart).
        let possessive = w.strip_prefix("'s").is_some_and(|r| !r.starts_with(char::is_alphanumeric));
        if !s.is_empty() && !possessive {
            s.push(' ');
        }
        s.push_str(w);
    }
    for p in [" ,", " .", " !", " ?", " ;", " :"] {
        s = s.replace(p, &p[1..]);
    }
    s
}

// --------------------------------------------------------------- the voice

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// `spotify.narration.proto.ResolveRequest` (tts-resolve.proto, which
/// librespot ships but doesn't compile): MP3 of `line`.
fn request(line: &Line) -> Vec<u8> {
    let mut b = Vec::with_capacity(line.ssml.len() + 16);
    // 2: ssml (the prompt)
    b.push(0x12);
    varint(&mut b, line.ssml.len() as u64);
    b.extend_from_slice(line.ssml.as_bytes());
    // 3: audio_format = MP3
    b.extend_from_slice(&[0x18, 5]);
    // 5: tts_voice, 6: tts_provider, 7: sample_rate_hz
    b.push(0x28);
    varint(&mut b, line.voice as u64);
    b.push(0x30);
    varint(&mut b, line.provider as u64);
    b.push(0x38);
    varint(&mut b, line.sample_rate as u64);
    b
}

/// The `url` of a `ResolveResponse` (field 1), if that's what came back.
fn response_url(b: &[u8]) -> Option<String> {
    let (&tag, rest) = b.split_first()?;
    if tag != 0x0a {
        return None;
    }
    let (mut len, mut shift, mut i) = (0usize, 0, 0);
    loop {
        let byte = *rest.get(i)?;
        // A length longer than any URL is a broken answer, not something to shift past 64 bits.
        if shift > 28 {
            return None;
        }
        len |= ((byte & 0x7f) as usize) << shift;
        i += 1;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let url = std::str::from_utf8(rest.get(i..i.checked_add(len)?)?).ok()?;
    url.starts_with("http").then(|| url.to_string())
}

fn is_audio(b: &[u8]) -> bool {
    b.starts_with(b"ID3") || (b.len() > 1 && b[0] == 0xff && b[1] & 0xe0 == 0xe0) || b.starts_with(b"RIFF") || b.starts_with(b"OggS")
}

/// X saying `line`, as MP3 bytes. The service answers with a redirect to the
/// clip, which librespot's client won't follow, so this asks with the
/// session's tokens itself. reqwest follows the redirect to Spotify's clip host
/// without the access token (the client token, which names the app install,
/// does go along).
pub async fn speak(session: &Session, http: &reqwest::Client, line: &Line) -> Result<Vec<u8>> {
    let e = |e: librespot_core::Error| anyhow::anyhow!("{e}");
    let base = session.spclient().base_url().await.map_err(e)?;
    let token = session.login5().auth_token().await.map_err(e)?;
    let mut req = http
        .post(format!("{}/client-tts/v1/fulfill", base.trim_end_matches('/')))
        .header("authorization", format!("{} {}", token.token_type, token.access_token))
        .header("content-type", "application/x-protobuf")
        .body(request(line));
    if let Ok(ct) = session.spclient().client_token().await {
        req = req.header("client-token", ct);
    }
    let clean = |e: reqwest::Error| anyhow::anyhow!("{}", e.without_url());
    let res = req.send().await.map_err(clean)?.error_for_status().map_err(clean)?.bytes().await.map_err(clean)?;
    if is_audio(&res) {
        return Ok(res.to_vec());
    }
    // Or a link to the clip.
    let url = response_url(&res).context("the DJ's voice didn't come back")?;
    let clip = http.get(&url).send().await.map_err(|e| anyhow::anyhow!("{}", e.without_url()))?.error_for_status().map_err(|e| anyhow::anyhow!("{}", e.without_url()))?.bytes().await?;
    if !is_audio(&clip) {
        bail!("the DJ's voice wasn't audio");
    }
    Ok(clip.to_vec())
}

/// Clips already spoken, by their SSML (the next lines are fetched ahead).
pub type Clips = std::sync::Arc<std::sync::Mutex<HashMap<String, std::sync::Arc<Vec<u8>>>>>;

/// The clips for `lines`, in order, from `cache` or freshly spoken.
pub async fn clips(session: &Session, http: &reqwest::Client, cache: &Clips, lines: &[Line]) -> Result<Vec<std::sync::Arc<Vec<u8>>>> {
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let cached = cache.lock().ok().and_then(|c| c.get(&l.ssml).cloned());
        let clip = match cached {
            Some(c) => c,
            None => {
                let c = std::sync::Arc::new(speak(session, http, l).await?);
                if let Ok(mut m) = cache.lock() {
                    if m.len() > 24 {
                        m.clear();
                    }
                    m.insert(l.ssml.clone(), c.clone());
                }
                c
            }
        };
        out.push(clip);
    }
    Ok(out)
}

/// X speaking: the clips, one after another, on the system output.
pub struct Voice {
    sink: rodio::Sink,
    /// The output stays open on a thread of its own until the voice is dropped
    /// (a stream can't move between threads; the voice can).
    _output: std::sync::mpsc::Sender<()>,
}

impl Voice {
    /// `volume` as Glass's volume slider has it (0–1). Opening the output takes a
    /// moment: call it off the UI thread.
    pub fn play(clips: &[std::sync::Arc<Vec<u8>>], volume: f32) -> Option<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (output, closed) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("dj-voice".into())
            .spawn(move || {
                let Ok(mut stream) = rodio::OutputStreamBuilder::open_default_stream() else {
                    let _ = ready_tx.send(None);
                    return;
                };
                stream.log_on_drop(false);
                let _ = ready_tx.send(Some(rodio::Sink::connect_new(stream.mixer())));
                // Until the Voice (the other end) goes away.
                let _ = closed.recv();
            })
            .ok()?;
        let sink = ready_rx.recv().ok()??;
        sink.set_volume(volume * volume);
        for c in clips {
            if let Ok(d) = rodio::Decoder::new(std::io::Cursor::new(c.to_vec())) {
                // The visualizer moves to X's voice too.
                sink.append(librespot_playback::tap::tapped(d));
            }
        }
        (!sink.empty()).then(|| {
            sink.play();
            Self { sink, _output: output }
        })
    }

    pub fn done(&self) -> bool {
        self.sink.empty()
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    pub fn toggle(&self) {
        if self.sink.is_paused() { self.sink.play() } else { self.sink.pause() }
    }

    pub fn set_volume(&self, v: f32) {
        self.sink.set_volume(v * v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_come_from_a_songs_metadata() {
        let m: HashMap<String, String> = [
            ("narration.intro.ssml", "<speak xml:lang=\"en-US\">Next up, I got some driving music. Starting with <entity type=\"artist\" uri=\"4MVy\">Lynyrd Skynyrd</entity>.</speak>"),
            ("narration.intro.voice", "VOICE1"),
            ("narration.intro.tts_provider", "SONANTIC_FAST"),
            ("narration.intro.sample_rate", "44100"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let line = Line::from_metadata(&m, "intro").unwrap();
        assert_eq!(line.text(), "Next up, I got some driving music. Starting with Lynyrd Skynyrd.");
        assert_eq!((line.voice, line.provider, line.sample_rate), (1, 6, 44_100));
        assert!(Line::from_metadata(&m, "outro").is_none());
    }

    #[test]
    fn captions_join_possessives_but_not_quotes() {
        assert_eq!(ssml_text("<speak>One from <entity type=\"artist\" uri=\"x\">Drake</entity>'s new album.</speak>"), "One from Drake's new album.");
        assert_eq!(ssml_text("<speak>Here&apos;s &apos;Free Bird&apos;.</speak>"), "Here's 'Free Bird'.");
        assert_eq!(ssml_text("<speak>Next, &apos;sweet&apos; songs.</speak>"), "Next, 'sweet' songs.");
    }

    #[test]
    fn a_closing_line_is_said_only_after_the_song_ends() {
        let says = |n: &str| -> HashMap<String, String> {
            [("narration.outro.ssml", format!("<speak>That was {n}.</speak>")), ("narration.outro.voice", "VOICE1".into())].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        };
        let (a, b, c) = (says("A"), says("B"), says("C"));
        let start = |m, after_end| on_track_start(&librespot_connect::glass::TrackStart { uri: "spotify:track:x", playing: true, metadata: m, next: None, after_end }).0;
        forget_outro();
        start(&a, false);
        // Skipped from another app: A's closing line isn't said.
        assert!(start(&b, false).is_empty());
        // B played to its end: it is.
        assert_eq!(start(&c, true).iter().map(Line::text).collect::<Vec<_>>(), ["That was B."]);
        forget_outro();
    }

    #[test]
    fn the_request_is_a_resolve_request() {
        let line = Line { ssml: "<speak>Hi</speak>".into(), voice: 1, provider: 6, sample_rate: 44_100 };
        let b = request(&line);
        assert_eq!(&b[..2], &[0x12, 17]);
        assert_eq!(&b[2..19], b"<speak>Hi</speak>");
        assert_eq!(&b[19..], &[0x18, 5, 0x28, 1, 0x30, 6, 0x38, 0xc4, 0xd8, 0x02]);
        let mut resp = vec![0x0a, 21];
        resp.extend_from_slice(b"https://tts.example/x");
        assert_eq!(response_url(&resp).as_deref(), Some("https://tts.example/x"));
        assert!(is_audio(b"ID3\\x04") && !is_audio(&resp));
    }
}
