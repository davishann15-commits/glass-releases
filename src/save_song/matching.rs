//! Picking the right recording among search results: the one whose title has the
//! song's words, that credits its artists, runs about as long, and isn't another
//! version of it (a cover, a remix, a live take…) unless the song is one too.

use super::Song;

/// A YouTube Music search result.
#[derive(Clone, Debug, PartialEq)]
pub struct YtHit {
    pub video_id: String,
    pub name: String,
    /// Songs: the artist column. Videos: the channel.
    pub artist: String,
    /// "m:ss"
    pub duration: String,
    pub is_song: bool,
}

/// A JioSaavn search result.
#[derive(Clone, Debug, PartialEq)]
pub struct SaavnHit {
    pub id: String,
    pub title: String,
    /// Comma-separated.
    pub artists: String,
}

/// Search results that fit the song, best first, with their scores (about 0-1). A result
/// must have most of the title's words and credit at least one of the artists.
pub fn rank_youtube(hits: &[YtHit], song: &Song) -> Vec<(String, f32)> {
    let want = Wanted::of(song);
    let scored = hits.iter().filter_map(|hit| {
        let title = words(&hit.name);
        // A video's credits are its channel, and often the title ("Artist - Song").
        let credits = if hit.is_song { words(&hit.artist) } else { [words(&hit.name), words(&hit.artist)].concat() };
        let (t, a) = want.fits(&title, &credits)?;
        let length = match (seconds(&hit.duration), song.duration_sec) {
            (Some(secs), want) if want > 0 => {
                // Near 1 within a few seconds, a half at 6 s apart, little past 20 s.
                let apart = (secs - want as f32) / 6.0;
                1.0 / (1.0 + apart * apart)
            }
            _ => 0.3,
        };
        // An "official audio" track is a little more likely the album recording.
        let song_bonus = if hit.is_song { 0.05 } else { 0.0 };
        Some((hit.video_id.clone(), (0.45 * a + 0.35 * length + 0.2 * t + song_bonus) * want.version_penalty(&title)))
    });
    best_first(scored.collect())
}

/// Like `rank_youtube`; JioSaavn's search gives no lengths, so credits and title decide.
pub fn rank_saavn(hits: &[SaavnHit], song: &Song) -> Vec<(String, f32)> {
    let want = Wanted::of(song);
    let scored = hits.iter().filter_map(|hit| {
        let title = words(&hit.title);
        let (t, a) = want.fits(&title, &words(&hit.artists))?;
        Some((hit.id.clone(), (0.65 * a + 0.35 * t) * want.version_penalty(&title)))
    });
    best_first(scored.collect())
}

/// What a result is compared against.
struct Wanted {
    /// The title's own words: without "(feat. …)" or " - Remastered 2011" and the like.
    title: Vec<String>,
    /// Every word of the full title, to tell whether the song itself is, say, live.
    all: Vec<String>,
    /// Each artist's words.
    artists: Vec<Vec<String>>,
}

/// Words that mark another version of a song.
const VERSIONS: &[&str] = &["cover", "remix", "live", "karaoke", "instrumental", "acoustic", "sped", "slowed", "reverb", "nightcore", "8d", "lofi", "mashup", "tribute", "parody"];

impl Wanted {
    fn of(song: &Song) -> Self {
        let base = song.title.split(" - ").next().unwrap_or(&song.title);
        // Brackets hold credits and versions ("(feat. X)", "[Live]"), not the title.
        let mut plain = String::new();
        let mut depth = 0u32;
        for c in base.chars() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth = depth.saturating_sub(1),
                _ if depth == 0 => plain.push(c),
                _ => {}
            }
        }
        let title = words(&plain);
        Wanted { title: if title.is_empty() { words(&song.title) } else { title }, all: words(&song.title), artists: song.artists.iter().map(|a| words(a)).filter(|w| !w.is_empty()).collect() }
    }

    /// (title score, artist score), each 0-1, or `None` if the result isn't this song.
    fn fits(&self, title: &[String], credits: &[String]) -> Option<(f32, f32)> {
        let found = self.title.iter().filter(|w| title.iter().any(|t| close(w, t))).count();
        let t = found as f32 / self.title.len().max(1) as f32;
        // Most of the title: one word may be missing or spelled differently.
        if found == 0 || (self.title.len() > 2 && t < 0.6) || (self.title.len() <= 2 && found < self.title.len()) {
            return None;
        }
        let credited = self.artists.iter().filter(|a| credits_artist(credits, a)).count();
        if credited == 0 {
            return None;
        }
        Some((t, credited as f32 / self.artists.len() as f32))
    }

    /// 1, or less for a version (cover, remix, live…) the song itself isn't.
    fn version_penalty(&self, title: &[String]) -> f32 {
        let other = VERSIONS.iter().any(|v| title.iter().any(|w| w == v) && !self.all.iter().any(|w| w == v));
        if other { 0.5 } else { 1.0 }
    }
}

/// Whether `credits` names the artist: all of its words, or the name run together
/// ("TheWeekndVEVO" credits "The Weeknd").
fn credits_artist(credits: &[String], artist: &[String]) -> bool {
    if artist.iter().all(|w| credits.iter().any(|c| close(w, c))) {
        return true;
    }
    let joined: String = artist.concat();
    joined.chars().count() >= 4 && credits.iter().any(|c| c.contains(&joined))
}

/// Lowercase words, with accents and punctuation set aside:
/// "Beyoncé – Halo (Live)" is `beyonce`, `halo`, `live`. "Lo-fi" is one word, `lofi`.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                match fold(l) {
                    Some(plain) => word.push_str(plain),
                    None => word.push(l),
                }
            }
        } else if (c == '-' || c == '\'' || c == '’') && !word.is_empty() && chars.peek().is_some_and(|n| n.is_alphanumeric()) {
            // Inside a word ("lo-fi", "don't") it stays one word.
        } else if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// A common Latin letter without its accent (other scripts are compared as they are).
fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => "a",
        'ç' | 'č' | 'ć' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => "e",
        'ì' | 'í' | 'î' | 'ï' | 'ī' => "i",
        'ñ' | 'ń' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => "o",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => "u",
        'ý' | 'ÿ' => "y",
        'š' | 'ś' => "s",
        'ž' | 'ź' | 'ż' => "z",
        'ł' => "l",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        _ => return None,
    })
}

/// The same word, allowing for a typo or another spelling in longer words (one
/// letter in 5-7, two from 8 on; short words must match: "love" isn't "live").
fn close(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let allowed = match a.len().min(b.len()) {
        0..=4 => return false,
        5..=7 => 1,
        _ => 2,
    };
    a.len().abs_diff(b.len()) <= allowed && edits(&a, &b) <= allowed
}

/// Levenshtein distance: the fewest single-letter changes from `a` to `b`.
fn edits(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

fn seconds(duration: &str) -> Option<f32> {
    let (m, s) = duration.split_once(':')?;
    Some(m.trim().parse::<f32>().ok()? * 60.0 + s.trim().parse::<f32>().ok()?)
}

fn best_first(mut scored: Vec<(String, f32)>) -> Vec<(String, f32)> {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut seen = std::collections::HashSet::new();
    scored.retain(|(id, _)| seen.insert(id.clone()));
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, artists: &[&str], duration_sec: u32) -> Song {
        Song { title: title.into(), artists: artists.iter().map(|a| a.to_string()).collect(), duration_sec, ..Default::default() }
    }

    fn yt(id: &str, name: &str, artist: &str, duration: &str, is_song: bool) -> YtHit {
        YtHit { video_id: id.into(), name: name.into(), artist: artist.into(), duration: duration.into(), is_song }
    }

    fn ids(ranked: &[(String, f32)]) -> Vec<&str> {
        ranked.iter().map(|r| r.0.as_str()).collect()
    }

    #[test]
    fn words_set_case_accents_and_punctuation_aside() {
        assert_eq!(words("Beyoncé – Halo (Live)"), ["beyonce", "halo", "live"]);
        assert_eq!(words("Lo-Fi Beats, Don't Stop"), ["lofi", "beats", "dont", "stop"]);
        assert_eq!(words("केसरिया Kesariya"), ["केसरिया", "kesariya"]);
        assert!(close("weeknd", "weekend") && close("colour", "color") && !close("love", "live") && !close("you", "yon"));
    }

    #[test]
    fn youtube_prefers_the_right_artist_and_length() {
        let hits = [
            yt("cover", "Blinding Lights (Metal Version)", "Leo", "4:01", true),
            yt("video", "The Weeknd - Blinding Lights (Official Video)", "The Weeknd", "4:23", false),
            yt("song", "Blinding Lights", "The Weeknd", "3:22", true),
            yt("other", "Something Else", "The Weeknd", "3:20", true),
        ];
        let ranked = rank_youtube(&hits, &song("Blinding Lights", &["The Weeknd"], 200));
        assert_eq!(ids(&ranked), ["song", "video"], "another artist and another song are out");
    }

    #[test]
    fn other_versions_lose_unless_the_song_is_one() {
        let hits = [yt("live", "Halo (Live)", "Beyonce", "3:44", true), yt("studio", "Halo", "Beyoncé", "3:41", true)];
        assert_eq!(ids(&rank_youtube(&hits, &song("Halo", &["Beyoncé"], 222)))[0], "studio");
        assert_eq!(ids(&rank_youtube(&hits, &song("Halo - Live", &["Beyoncé"], 224)))[0], "live");
    }

    #[test]
    fn credits_come_in_many_shapes() {
        // Featured artists, an "&" column, and a channel named after the artist.
        let s = song("Stay (with Justin Bieber)", &["The Kid LAROI", "Justin Bieber"], 141);
        let hits = [yt("duet", "STAY", "The Kid LAROI & Justin Bieber", "2:21", true), yt("solo", "Stay", "Someone Else", "2:21", true)];
        assert_eq!(ids(&rank_youtube(&hits, &s)), ["duet"]);
        let video = [yt("vevo", "Blinding Lights (Official Audio)", "TheWeekndVEVO", "3:21", false)];
        assert_eq!(ids(&rank_youtube(&video, &song("Blinding Lights", &["The Weeknd"], 200))), ["vevo"]);
    }

    #[test]
    fn saavn_needs_a_shared_artist() {
        let hit = |id: &str, title: &str, artists: &str| SaavnHit { id: id.into(), title: title.into(), artists: artists.into() };
        let hits = [hit("lofi", "Kesariya (Lofi)", "VIBIE"), hit("real", "Kesariya", "Pritam, Arijit Singh"), hit("x", "Unrelated", "Arijit Singh")];
        let ranked = rank_saavn(&hits, &song("Kesariya", &["Arijit Singh", "Pritam"], 268));
        assert_eq!(ids(&ranked), ["real"]);
    }
}
