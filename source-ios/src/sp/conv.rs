//! Converts Spotify's internal formats (metadata protobufs, GraphQL JSON)
//! into the Web API's JSON shapes, which is what the UI reads.

use librespot_core::SpotifyId;
use librespot_protocol::metadata::{self, Image, ImageGroup};
use serde_json::{Value, json};

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn image_url(file_id: &[u8]) -> String {
    format!("https://i.scdn.co/image/{}", hex(file_id))
}

fn base62(gid: &[u8]) -> String {
    SpotifyId::from_raw(gid).ok().and_then(|id| id.to_base62().ok()).unwrap_or_default()
}

fn uri(kind: &str, gid: &[u8]) -> (String, String) {
    let id = base62(gid);
    (format!("spotify:{kind}:{id}"), id)
}

/// Spotify stores some names HTML-escaped ("Rock &amp; Roll").
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&#x27;", "'").replace("&#x2F;", "/").replace("&amp;", "&")
}

fn images(list: &[Image]) -> Vec<Value> {
    list.iter()
        .map(|i| {
            let w = match i.size() {
                metadata::image::Size::SMALL => 64,
                metadata::image::Size::LARGE => 640,
                metadata::image::Size::XLARGE => 1000,
                _ => 300,
            };
            json!({ "url": image_url(i.file_id()), "width": w, "height": w })
        })
        .collect()
}

fn group_or(g: &ImageGroup, fallback: &[Image]) -> Vec<Value> {
    if g.image.is_empty() { images(fallback) } else { images(&g.image) }
}

fn date(d: &metadata::Date) -> (String, &'static str) {
    match (d.year(), d.month(), d.day()) {
        (0, _, _) => (String::new(), "year"),
        (y, 0, _) => (format!("{y:04}"), "year"),
        (y, m, 0) => (format!("{y:04}-{m:02}"), "month"),
        (y, m, day) => (format!("{y:04}-{m:02}-{day:02}"), "day"),
    }
}

/// Milliseconds since the epoch -> "2024-05-01T12:00:00Z".
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem / 60) % 60, rem % 60)
}

fn artist_ref(a: &metadata::Artist) -> Value {
    let (uri, id) = uri("artist", a.gid());
    json!({ "id": id, "uri": uri, "name": a.name(), "type": "artist" })
}

fn album_type(a: &metadata::Album) -> &'static str {
    use metadata::album::Type;
    match a.type_() {
        Type::SINGLE | Type::EP => "single",
        Type::COMPILATION => "compilation",
        _ => "album",
    }
}

fn album_ref(a: &metadata::Album) -> Value {
    let (uri, id) = uri("album", a.gid());
    let (release, precision) = date(&a.date);
    json!({
        "id": id, "uri": uri, "name": a.name(), "type": "album", "album_type": album_type(a),
        "images": group_or(&a.cover_group, &a.cover),
        "artists": a.artist.iter().map(artist_ref).collect::<Vec<_>>(),
        "release_date": release, "release_date_precision": precision,
    })
}

pub fn track(t: &metadata::Track) -> Value {
    let (uri, id) = uri("track", t.gid());
    json!({
        "id": id, "uri": uri, "name": t.name(), "type": "track",
        "duration_ms": t.duration().max(0), "explicit": t.explicit(), "popularity": t.popularity(),
        "track_number": t.number(), "disc_number": t.disc_number().max(1),
        "is_local": false, "is_playable": true,
        "artists": t.artist.iter().map(artist_ref).collect::<Vec<_>>(),
        "album": album_ref(&t.album),
        // The recording's ISRC: lyrics are looked up by it.
        "external_ids": external_ids(&t.external_id),
        "preview_url": preview_url(t),
    })
}

/// The song's 30-second preview clip (an MP3 on Spotify's CDN), when it has
/// one; a song relinked in the user's country has it on an alternative.
fn preview_url(t: &metadata::Track) -> Option<String> {
    let id = t.preview.iter().chain(t.alternative.iter().flat_map(|a| a.preview.iter())).find_map(|f| f.file_id.as_deref().filter(|id| !id.is_empty()))?;
    Some(format!("https://p.scdn.co/mp3-preview/{}", id.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

fn external_ids(ids: &[metadata::ExternalId]) -> Value {
    let mut out = serde_json::Map::new();
    for e in ids {
        if !e.type_().is_empty() && !e.id().is_empty() {
            out.entry(e.type_().to_ascii_lowercase()).or_insert_with(|| json!(e.id()));
        }
    }
    Value::Object(out)
}

/// A full album. `tracks` are the converted disc tracks, in order.
pub fn album(a: &metadata::Album, tracks: Vec<Value>) -> Value {
    let mut v = album_ref(a);
    let copyrights: Vec<Value> = a.copyright.iter().map(|c| json!({ "text": c.text(), "type": if c.type_() == metadata::copyright::Type::P { "P" } else { "C" } })).collect();
    v["label"] = json!(a.label());
    v["popularity"] = json!(a.popularity());
    v["copyrights"] = json!(copyrights);
    v["total_tracks"] = json!(tracks.len());
    v["tracks"] = json!({ "items": tracks, "total": tracks.len() });
    v
}

pub fn album_track_uris(a: &metadata::Album) -> Vec<String> {
    a.disc.iter().flat_map(|d| d.track.iter()).map(|t| uri("track", t.gid()).0).collect()
}

pub fn artist(a: &metadata::Artist) -> Value {
    let (uri, id) = uri("artist", a.gid());
    json!({ "id": id, "uri": uri, "name": a.name(), "type": "artist", "popularity": a.popularity(), "images": group_or(&a.portrait_group, &a.portrait), "followers": { "total": 0 }, "genres": [] })
}

/// Every release URI on an artist's page, by group.
pub fn artist_release_uris(a: &metadata::Artist) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (groups, group) in [(&a.album_group, "album"), (&a.single_group, "single"), (&a.compilation_group, "compilation"), (&a.appears_on_group, "appears_on")] {
        for g in groups.iter() {
            // Each group lists regional editions of one release; the first is enough.
            if let Some(al) = g.album.first() {
                out.push((uri("album", al.gid()).0, group));
            }
        }
    }
    out
}

fn show_ref(s: &metadata::Show) -> Value {
    let (uri, id) = uri("show", s.gid());
    json!({ "id": id, "uri": uri, "name": s.name(), "publisher": s.publisher(), "type": "show", "images": images(&s.cover_image.image) })
}

pub fn show(s: &metadata::Show) -> Value {
    let mut v = show_ref(s);
    v["description"] = json!(s.description());
    v["total_episodes"] = json!(s.episode.len());
    v["explicit"] = json!(s.explicit());
    if s.is_audiobook() {
        v["type"] = json!("audiobook");
        v["authors"] = json!([{ "name": s.publisher() }]);
    }
    v
}

pub fn show_episode_uris(s: &metadata::Show) -> Vec<String> {
    s.episode.iter().map(|e| uri("episode", e.gid()).0).collect()
}

pub fn episode(e: &metadata::Episode) -> Value {
    let (uri, id) = uri("episode", e.gid());
    let (release, precision) = date(&e.publish_time);
    let mut imgs = images(&e.cover_image.image);
    if imgs.is_empty() {
        imgs = images(&e.show.cover_image.image);
    }
    json!({
        "id": id, "uri": uri, "name": e.name(), "type": "episode",
        "description": e.description(), "duration_ms": e.duration().max(0), "explicit": e.explicit(),
        "release_date": release, "release_date_precision": precision, "images": imgs,
        "show": show_ref(&e.show), "resume_point": {},
    })
}

/// `spotify:local:artist:album:title:seconds`, each part URL-encoded with '+' for spaces.
pub fn local_track(uri: &str) -> Value {
    let parts: Vec<String> = uri.split(':').skip(2).map(form_decode).collect();
    let get = |i: usize| parts.get(i).cloned().unwrap_or_default();
    let secs: u64 = get(3).parse().unwrap_or(0);
    let artist = get(0);
    json!({
        "uri": uri, "id": "", "type": "track", "is_local": true, "name": get(2), "duration_ms": secs * 1000,
        "artists": if artist.is_empty() { json!([]) } else { json!([{ "name": artist, "type": "artist" }]) },
        "album": { "name": get(1), "images": [] },
    })
}

/// A form-encoded value decoded: `+` is a space, `%XX` a byte.
pub(crate) fn form_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A stand-in for an entity whose metadata Spotify didn't return.
pub fn placeholder(uri: &str) -> Value {
    let kind = uri.split(':').nth(1).unwrap_or("track");
    json!({ "uri": uri, "id": uri.rsplit(':').next().unwrap_or(""), "type": kind, "name": "Unavailable", "artists": [], "album": { "name": "", "images": [] }, "duration_ms": 0, "is_playable": false })
}

// ------------------------------------------------------------------ GraphQL

fn gimages(v: &Value) -> Value {
    let sources = v.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
    json!(sources.iter().map(|s| json!({ "url": s["url"], "width": s["width"].as_u64().or(s["maxWidth"].as_u64()), "height": s["height"].as_u64().or(s["maxHeight"].as_u64()) })).collect::<Vec<_>>())
}

fn id_of(uri: &str) -> String {
    uri.rsplit(':').next().unwrap_or("").to_string()
}

fn gartists(v: &Value) -> Value {
    json!(
        v["items"]
            .as_array()
            .map(|a| a
                .iter()
                .map(|x| {
                    let uri = x["uri"].as_str().unwrap_or("");
                    json!({ "uri": uri, "id": id_of(uri), "name": x["profile"]["name"], "type": "artist" })
                })
                .collect::<Vec<_>>())
            .unwrap_or_default()
    )
}

fn gdate(d: &Value) -> String {
    match (d["year"].as_u64(), d["month"].as_u64(), d["day"].as_u64()) {
        (Some(y), Some(m), Some(day)) => format!("{y:04}-{m:02}-{day:02}"),
        (Some(y), Some(m), None) => format!("{y:04}-{m:02}"),
        (Some(y), _, _) => format!("{y:04}"),
        _ => d["isoString"].as_str().unwrap_or("").to_string(),
    }
}

/// Any GraphQL entity (or `*ResponseWrapper` around one) -> Web API JSON.
pub fn gentity(v: &Value) -> Option<Value> {
    let d = if v["__typename"].as_str().is_some_and(|t| t.ends_with("ResponseWrapper")) || v.get("data").is_some_and(|d| d.get("__typename").is_some()) {
        &v["data"]
    } else if let Some(item) = v.get("item") {
        return gentity(item);
    } else {
        v
    };
    let uri = d["uri"].as_str().unwrap_or("").to_string();
    if uri.is_empty() {
        return None;
    }
    let id = id_of(&uri);
    Some(match d["__typename"].as_str().unwrap_or("") {
        "Track" => {
            let al = &d["albumOfTrack"];
            let al_uri = al["uri"].as_str().unwrap_or("");
            json!({
                "uri": uri, "id": id, "type": "track", "name": d["name"],
                "duration_ms": d["duration"]["totalMilliseconds"].as_u64().unwrap_or(0),
                "explicit": d["contentRating"]["label"].as_str() == Some("EXPLICIT"),
                "artists": gartists(&d["artists"]),
                "album": { "uri": al_uri, "id": id_of(al_uri), "name": al["name"], "type": "album", "images": gimages(&al["coverArt"]) },
                "is_playable": d["playability"]["playable"].as_bool().unwrap_or(true),
            })
        }
        "Album" | "Prerelease" => json!({
            "uri": uri, "id": id, "type": "album", "name": d["name"],
            "album_type": d["type"].as_str().unwrap_or("album").to_lowercase().replace("ep", "single"),
            "images": gimages(&d["coverArt"]), "artists": gartists(&d["artists"]),
            "release_date": gdate(&d["date"]),
        }),
        "Artist" => json!({
            "uri": uri, "id": id, "type": "artist", "name": d["profile"]["name"],
            "images": gimages(&d["visuals"]["avatarImage"]),
            "followers": { "total": d["stats"]["followers"].as_u64().unwrap_or(0) },
        }),
        "Playlist" => {
            let owner = &d["ownerV2"]["data"];
            json!({
                "uri": uri, "id": id, "type": "playlist", "name": unescape(d["name"].as_str().unwrap_or("")),
                "description": d["description"],
                "images": d["images"]["items"].as_array().and_then(|a| a.first()).map(gimages).unwrap_or(json!([])),
                "owner": { "id": owner["username"], "display_name": owner["name"], "uri": owner["uri"], "type": "user" },
            })
        }
        "Podcast" => json!({
            "uri": uri, "id": id, "type": "show", "name": d["name"], "publisher": d["publisher"]["name"],
            "images": gimages(&d["coverArt"]), "description": d["description"],
        }),
        "Audiobook" => json!({
            "uri": uri, "id": id, "type": "audiobook", "name": d["name"], "images": gimages(&d["coverArt"]),
            "authors": d["authorsV2"].as_array().map(|a| a.iter().map(|x| json!({ "name": x["name"] })).collect::<Vec<_>>()).unwrap_or_default(),
            "description": d["description"],
        }),
        "Episode" | "Chapter" => {
            let show = &d["podcastV2"]["data"];
            let played = &d["playedState"];
            json!({
                "uri": uri, "id": id, "type": "episode", "name": d["name"], "description": d["description"],
                "duration_ms": d["duration"]["totalMilliseconds"].as_u64().unwrap_or(0),
                "release_date": d["releaseDate"]["isoString"], "images": gimages(&d["coverArt"]),
                "show": { "uri": show["uri"], "id": id_of(show["uri"].as_str().unwrap_or("")), "name": show["name"], "publisher": show["publisher"]["name"], "type": "show" },
                "resume_point": { "resume_position_ms": played["playPositionMilliseconds"].as_u64().unwrap_or(0), "fully_played": played["state"].as_str() == Some("COMPLETED") },
            })
        }
        "User" => json!({
            "uri": uri, "id": d["username"].as_str().map(str::to_string).unwrap_or(id), "type": "user",
            "display_name": d["displayName"].as_str().or(d["name"].as_str()).unwrap_or(""), "images": gimages(&d["avatar"]),
        }),
        _ => return None,
    })
}

/// `{"items": [...]}` of GraphQL entities -> Web API entities.
pub fn glist(v: &Value) -> Vec<Value> {
    v["items"].as_array().map(|a| a.iter().filter_map(gentity).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_791_272_416_000), "2026-10-06T07:40:16Z");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn local_uris() {
        let v = local_track("spotify:local:Tatsuro+Yamashita:Big+Wave:Magic+Ways:279");
        assert_eq!(v["name"], "Magic Ways");
        assert_eq!(v["artists"][0]["name"], "Tatsuro Yamashita");
        assert_eq!(v["album"]["name"], "Big Wave");
        assert_eq!(v["duration_ms"], 279_000);
        let v = local_track("spotify:local:::%E5%B1%B1%E4%B8%8B:165");
        assert_eq!(v["name"], "山下");
        assert_eq!(v["artists"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn html_entities() {
        assert_eq!(unescape("50s &amp; 60s Rock &amp; Blues"), "50s & 60s Rock & Blues");
        assert_eq!(unescape("plain"), "plain");
    }

    #[test]
    fn graphql_entities() {
        let t = json!({ "item": { "__typename": "TrackResponseWrapper", "data": { "__typename": "Track", "uri": "spotify:track:abc", "name": "Song",
            "duration": { "totalMilliseconds": 1000 }, "artists": { "items": [{ "uri": "spotify:artist:x", "profile": { "name": "A" } }] },
            "albumOfTrack": { "uri": "spotify:album:y", "name": "Al", "coverArt": { "sources": [{ "url": "u", "width": 64, "height": 64 }] } } } } });
        let v = gentity(&t).unwrap();
        assert_eq!(v["type"], "track");
        assert_eq!(v["artists"][0]["name"], "A");
        assert_eq!(v["album"]["id"], "y");
        assert_eq!(v["album"]["images"][0]["url"], "u");
        let a = json!({ "__typename": "Artist", "uri": "spotify:artist:z", "profile": { "name": "Z" }, "visuals": { "avatarImage": { "sources": [] } } });
        assert_eq!(gentity(&a).unwrap()["name"], "Z");
        assert!(gentity(&json!({ "__typename": "Genre", "uri": "spotify:genre:1" })).is_none());
    }
}
