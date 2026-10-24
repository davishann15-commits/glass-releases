//! Small read-only helpers over Web API JSON, and a few shared string helpers.
//! Nothing here does I/O, except asking `date` for the UTC offset (for the
//! greeting and the clock).

use serde_json::Value;

pub fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn u(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

pub fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn items(v: &Value) -> &[Value] {
    v.get("items").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

pub fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Pick an image URL near `want` pixels wide from an `images` array.
pub fn image(v: &Value, want: u64) -> Option<String> {
    let imgs = v.get("images").and_then(Value::as_array).or_else(|| v.get("album").and_then(|a| a.get("images")).and_then(Value::as_array))?;
    let mut best: Option<(&Value, u64)> = None;
    for img in imgs {
        let w = img["width"].as_u64().unwrap_or(640);
        let score = w.abs_diff(want) + if w < want { 1000 } else { 0 };
        if best.map(|(_, s)| score < s).unwrap_or(true) {
            best = Some((img, score));
        }
    }
    best.and_then(|(i, _)| i["url"].as_str().map(str::to_string)).or_else(|| imgs.first().and_then(|i| i["url"].as_str().map(str::to_string)))
}

pub fn artists_line(v: &Value) -> String {
    let list = arr(v, "artists");
    if list.is_empty() {
        // Episodes and chapters have a show/audiobook instead.
        if let Some(show) = v.get("show") {
            return s(show, "name").into();
        }
        if let Some(authors) = v.get("authors").and_then(Value::as_array) {
            return authors.iter().map(|a| s(a, "name")).collect::<Vec<_>>().join(", ");
        }
        return s(v, "publisher").into();
    }
    list.iter().map(|a| s(a, "name")).collect::<Vec<_>>().join(", ")
}

/// FNV-1a: a short, stable hash of a string (for file names and IDs that must
/// stay the same from one run to the next, which `std`'s hashers don't promise).
pub fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// `n` random hex digits (device IDs, local folder IDs).
pub fn random_hex(n: usize) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..n).map(|_| char::from_digit(rng.gen_range(0..16), 16).unwrap_or('0')).collect()
}

pub fn id_of(uri: &str) -> &str {
    uri.rsplit(':').next().unwrap_or(uri)
}

pub fn kind_of(uri: &str) -> &str {
    let mut parts = uri.split(':');
    parts.next();
    match parts.next() {
        Some("user") => {
            // spotify:user:<id>:collection or spotify:user:<id>:playlist:<id>
            let rest: Vec<_> = parts.collect();
            if rest.len() >= 3 {
                rest[1]
            } else if rest.get(1) == Some(&"collection") {
                "collection"
            } else {
                "user"
            }
        }
        Some(k) => k,
        None => "",
    }
}

/// The open.spotify.com page for a URI. Local files have none.
pub fn web_link(uri: &str) -> Option<String> {
    if uri.starts_with("spotify:local:") {
        return None;
    }
    if uri.ends_with(":collection") {
        return Some("https://open.spotify.com/collection/tracks".into());
    }
    Some(format!("https://open.spotify.com/{}/{}", kind_of(uri), id_of(uri)))
}

pub fn fmt_ms(ms: u64) -> String {
    let secs = ms / 1000;
    if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60) } else { format!("{}:{:02}", secs / 60, secs % 60) }
}

pub fn fmt_long(ms: u64) -> String {
    let mins = ms / 60_000;
    if mins >= 60 { format!("{} hr {} min", mins / 60, mins % 60) } else { format!("{} min {} sec", mins, (ms / 1000) % 60) }
}

/// "1 song", "1,204 songs".
pub fn songs(n: u64) -> String {
    if n == 1 { "1 song".into() } else { format!("{} songs", fmt_count(n)) }
}

pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "2024-05-01T12:00:00Z" -> "May 1, 2024"; "2019" stays "2019".
pub fn fmt_date(d: &str) -> String {
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let date = d.split('T').next().unwrap_or(d);
    let p: Vec<&str> = date.split('-').collect();
    match p.as_slice() {
        [y, m, day] => {
            let mi = m.parse::<usize>().unwrap_or(1).clamp(1, 12) - 1;
            format!("{} {}, {}", M[mi], day.trim_start_matches('0'), y)
        }
        [y, m] => {
            let mi = m.parse::<usize>().unwrap_or(1).clamp(1, 12) - 1;
            format!("{} {}", M[mi], y)
        }
        _ => date.to_string(),
    }
}

pub fn year(d: &str) -> &str {
    d.get(..4).unwrap_or(d)
}

/// Strip the small set of HTML tags Spotify puts in descriptions.
pub fn plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    crate::sp::conv::unescape(&out)
}

/// A playlist item may be `{track: {...}}` (classic) or `{item: {...}}` (newer).
pub fn playlist_entry(item: &Value) -> &Value {
    item.get("track").filter(|t| !t.is_null()).or_else(|| item.get("item")).unwrap_or(item)
}

pub fn greeting() -> &'static str {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // The local hour, from the system's UTC offset; UTC if that can't be read.
    let hour = local_hour().unwrap_or(((secs / 3600) % 24) as u32);
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// The system's UTC offset ("+0200" from `date`), looked up again every hour so
/// a daylight-saving change or a trip across time zones shows up.
fn local_offset_secs() -> Option<i64> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static OFFSET: Mutex<Option<(Instant, Option<i64>)>> = Mutex::new(None);
    let mut cached = OFFSET.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, offset)) = *cached
        && at.elapsed() < Duration::from_secs(3600)
    {
        return offset;
    }
    let read = || -> Option<i64> {
        let out = std::process::Command::new("date").arg("+%z").output().ok()?;
        let s = String::from_utf8(out.stdout).ok()?;
        let s = s.trim();
        let sign = if s.starts_with('-') { -1 } else { 1 };
        let h: i64 = s.get(1..3)?.parse().ok()?;
        let m: i64 = s.get(3..5)?.parse().ok()?;
        Some(sign * (h * 3600 + m * 60))
    };
    let offset = read();
    *cached = Some((Instant::now(), offset));
    offset
}

fn local_hour() -> Option<u32> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some((((now + local_offset_secs()?) / 3600).rem_euclid(24)) as u32)
}

/// Local wall-clock "HH:MM:SS" for a timestamp.
pub fn clock(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) + local_offset_secs().unwrap_or(0);
    let d = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", d / 3600, (d / 60) % 60, d % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uris() {
        assert_eq!(kind_of("spotify:track:abc"), "track");
        assert_eq!(id_of("spotify:track:abc"), "abc");
        assert_eq!(kind_of("spotify:user:bob:collection"), "collection");
        assert_eq!(kind_of("spotify:user:bob:playlist:xyz"), "playlist");
        assert_eq!(web_link("spotify:album:42").as_deref(), Some("https://open.spotify.com/album/42"));
        assert_eq!(web_link("spotify:local:Anri:Timely:Remember+Summer+Days:250"), None);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_ms(0), "0:00");
        assert_eq!(fmt_ms(61_000), "1:01");
        assert_eq!(fmt_ms(3_723_000), "1:02:03");
        assert_eq!(fmt_count(1234567), "1,234,567");
        assert_eq!(fmt_date("2024-05-01T12:00:00Z"), "May 1, 2024");
        assert_eq!(fmt_date("2019"), "2019");
        assert_eq!(plain("<a href=\"x\">Hi</a> &amp; bye"), "Hi & bye");
    }

    #[test]
    fn images_pick_closest_not_smaller() {
        let v = json!({ "images": [
            { "url": "big", "width": 640 }, { "url": "mid", "width": 300 }, { "url": "small", "width": 64 }
        ] });
        assert_eq!(image(&v, 64).as_deref(), Some("small"));
        assert_eq!(image(&v, 200).as_deref(), Some("mid"));
        assert_eq!(image(&json!({}), 64), None);
    }

    #[test]
    fn playlist_entries_old_and_new_shape() {
        assert_eq!(s(playlist_entry(&json!({ "track": { "name": "a" } })), "name"), "a");
        assert_eq!(s(playlist_entry(&json!({ "item": { "name": "b" } })), "name"), "b");
    }
}
