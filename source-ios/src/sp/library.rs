//! Your library: the collection sets (Liked Songs and saved albums, followed
//! artists, podcasts, Your Episodes) and playlists — read and written through
//! the same services Spotify's desktop app uses.

use super::player::now_ms;
use super::{Sp, conv, pb};
use crate::model::id_of;
use anyhow::{Context, Result};
use futures_util::{StreamExt, stream};
use http::{HeaderMap, Method};
use librespot_protocol::playlist4_external::{
    Add, CreateListReply, Delta, Item, ItemAttributes, ListAttributes, ListAttributesPartialState, ListChanges, ListUpdateRequest, Mov, Op, Rem, SelectedListContent, UpdateListAttributes, op,
};
use protobuf::{Message, MessageField};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

const COLLECTION_PROTO: &str = "application/vnd.collection-v2.spotify.proto";
const SET_TTL: Duration = Duration::from_secs(90);
const LIST_TTL: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct Saved {
    pub uri: String,
    pub added_at: i64,
}

pub struct Playlist {
    /// Web API playlist object, without its items.
    pub json: Value,
    /// (uri, added_by, added_at in ms)
    pub items: Vec<(String, String, i64)>,
}

pub struct Rootlist {
    pub playlists: Vec<Value>,
}

fn item(uri: &str, added_by: Option<&str>) -> Item {
    let mut i = Item::new();
    i.set_uri(uri.into());
    let mut a = ItemAttributes::new();
    if let Some(u) = added_by {
        a.set_added_by(u.into());
    }
    a.set_timestamp(now_ms());
    i.attributes = MessageField::some(a);
    i
}

impl Sp {
    // ------------------------------------------------------------ collection

    /// One collection set ("collection", "artist", "show", "listenlater"),
    /// newest first. Cached briefly; writes update the cache in place.
    pub(super) async fn set(&self, name: &str) -> Result<Arc<Vec<Saved>>> {
        if let Some((at, v)) = self.sets.lock().unwrap().get(name)
            && at.elapsed() < SET_TTL
        {
            return Ok(v.clone());
        }
        let v = Arc::new(self.load_set(name).await?);
        self.sets.lock().unwrap().insert(name.to_string(), (Instant::now(), v.clone()));
        Ok(v)
    }

    async fn load_set(&self, name: &str) -> Result<Vec<Saved>> {
        let mut out = Vec::new();
        let mut token = String::new();
        // 300 per page; the cap only guards against a token that never ends.
        for _ in 0..500 {
            let mut req = pb::Writer::new().string(1, &self.user).string(2, name).int(4, 300);
            if !token.is_empty() {
                req = req.string(3, &token);
            }
            let raw = self.collection_post("paging", req).await?;
            token.clear();
            for (f, v) in pb::fields(&raw) {
                match (f, &v) {
                    (1, pb::Field::Bytes(b)) => {
                        let (mut uri, mut at, mut removed) = (String::new(), 0, false);
                        for (g, w) in pb::fields(b) {
                            match g {
                                1 => uri = pb::string(&w),
                                2 => at = pb::int(&w) as i64,
                                3 => removed = pb::int(&w) != 0,
                                _ => {}
                            }
                        }
                        if !removed && !uri.is_empty() {
                            out.push(Saved { uri, added_at: at });
                        }
                    }
                    (2, _) => token = pb::string(&v),
                    _ => {}
                }
            }
            if token.is_empty() {
                break;
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.added_at));
        Ok(out)
    }

    async fn collection_post(&self, endpoint: &str, body: pb::Writer) -> Result<Vec<u8>> {
        let mut h = HeaderMap::new();
        h.insert("content-type", COLLECTION_PROTO.parse()?);
        h.insert("accept", COLLECTION_PROTO.parse()?);
        let path = format!("/collection/v2/{endpoint}");
        Ok(self.session.spclient().request(&Method::POST, &path, Some(h), Some(&body.0)).await?.to_vec())
    }

    /// Add to or remove from a collection set.
    pub(super) async fn write_set(&self, name: &str, uris: &[String], remove: bool) -> Result<()> {
        let now = now_ms() / 1000;
        let mut w = pb::Writer::new().string(1, &self.user).string(2, name);
        for u in uris {
            let mut it = pb::Writer::new().string(1, u).int(2, now as u64);
            if remove {
                it = it.int(3, 1);
            }
            w = w.message(3, it);
        }
        w = w.string(4, &format!("{:016x}", now_ms() as u64 ^ 0x5eed_9a55));
        self.collection_post("write", w).await?;
        if let Some((_, v)) = self.sets.lock().unwrap().get_mut(name) {
            let mut list = (**v).clone();
            if remove {
                list.retain(|s| !uris.contains(&s.uri));
            } else {
                for u in uris.iter().rev() {
                    if !list.iter().any(|s| &s.uri == u) {
                        list.insert(0, Saved { uri: u.clone(), added_at: now });
                    }
                }
            }
            *v = Arc::new(list);
        }
        Ok(())
    }

    /// Drops lists past their time (they're read again when next asked for).
    pub(super) fn trim_library(&self) {
        self.sets.lock().unwrap().retain(|_, (at, _)| at.elapsed() < SET_TTL);
        self.playlists.lock().unwrap().retain(|_, (at, _)| at.elapsed() < LIST_TTL);
        let mut root = self.rootlist.lock().unwrap();
        if root.as_ref().is_some_and(|(at, _)| at.elapsed() >= LIST_TTL) {
            *root = None;
        }
    }

    // ------------------------------------------------------------ playlists

    fn playlist_json(&self, uri: &str, attrs: &ListAttributes, owner: &str, length: i32) -> Value {
        // Every size Spotify offers, so small rows don't download 640 px art.
        let mut images: Vec<Value> = attrs
            .picture_size
            .iter()
            .filter_map(|p| {
                let w = match p.target_name() {
                    "small" => 60,
                    "default" => 300,
                    "large" => 640,
                    _ => return None,
                };
                Some(json!({ "url": p.url(), "width": w, "height": w }))
            })
            .collect();
        if images.is_empty()
            && let Some(u) = (!attrs.picture().is_empty()).then(|| conv::image_url(attrs.picture())).or_else(|| self.mosaics.lock().unwrap().get(uri).cloned())
        {
            if u.contains("mosaic.scdn.co/640/") {
                images.push(json!({ "url": u.replace("/640/", "/300/"), "width": 300, "height": 300 }));
            }
            images.push(json!({ "url": u, "width": 640, "height": 640 }));
        }
        let owner_name = match owner {
            o if o == self.user => self.display_name(),
            "spotify" => "Spotify".to_string(),
            o => o.to_string(),
        };
        json!({
            "id": id_of(uri), "uri": uri, "type": "playlist",
            "name": conv::unescape(attrs.name()), "description": attrs.description(),
            "collaborative": attrs.collaborative(), "public": false,
            "images": images,
            "owner": { "id": owner, "display_name": owner_name, "uri": format!("spotify:user:{owner}"), "type": "user" },
            "tracks": { "total": length.max(0) }, "followers": { "total": 0 },
        })
    }

    /// Your playlists, in your library's order.
    pub(super) async fn rootlist(&self) -> Result<Arc<Rootlist>> {
        if let Some((at, r)) = self.rootlist.lock().unwrap().as_ref()
            && at.elapsed() < LIST_TTL
        {
            return Ok(r.clone());
        }
        let list = self.fetch_rootlist().await?;
        let uris: Vec<String> = list.contents.items.iter().map(|i| i.uri().to_string()).filter(|u| u.starts_with("spotify:playlist:")).collect();
        let pictured: HashSet<String> = list
            .contents
            .items
            .iter()
            .zip(list.contents.meta_items.iter())
            .filter(|(_, m)| !m.attributes.picture().is_empty() || !m.attributes.picture_size.is_empty())
            .map(|(i, _)| i.uri().to_string())
            .collect();
        let need: Vec<String> = {
            let m = self.mosaics.lock().unwrap();
            uris.iter().filter(|u| !pictured.contains(*u) && !m.contains_key(*u)).cloned().collect()
        };
        self.fill_mosaics(need).await;
        let playlists = list
            .contents
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.uri().starts_with("spotify:playlist:"))
            .map(|(n, i)| match list.contents.meta_items.get(n) {
                Some(m) => self.playlist_json(i.uri(), &m.attributes, m.owner_username(), m.length()),
                None => self.playlist_json(i.uri(), &ListAttributes::new(), "", 0),
            })
            .collect();
        let r = Arc::new(Rootlist { playlists });
        *self.rootlist.lock().unwrap() = Some((Instant::now(), r.clone()));
        Ok(r)
    }

    async fn fetch_rootlist(&self) -> Result<SelectedListContent> {
        let path = format!("/playlist/v2/user/{}/rootlist?decorate=revision,attributes,length,owner,capabilities,status_code&from=0&length=1000", self.user);
        let raw = self.session.spclient().request(&Method::GET, &path, None, None).await?;
        Ok(SelectedListContent::parse_from_bytes(&raw)?)
    }

    /// Playlists without a picture show a 2x2 grid of their first albums,
    /// as in Spotify. Needs the first few tracks of each, then one batch of
    /// track metadata for all of them.
    async fn fill_mosaics(&self, playlists: Vec<String>) {
        if playlists.is_empty() {
            return;
        }
        let heads: Vec<(String, Vec<String>)> = stream::iter(playlists)
            .map(|uri| async move {
                let path = format!("/playlist/v2/playlist/{}?from=0&length=16", id_of(&uri));
                let items = match self.session.spclient().request(&Method::GET, &path, None, None).await {
                    Ok(raw) => SelectedListContent::parse_from_bytes(&raw)
                        .map(|l| l.contents.items.iter().map(|i| i.uri().to_string()).filter(|u| u.starts_with("spotify:track:")).take(16).collect())
                        .unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
                (uri, items)
            })
            .buffer_unordered(8)
            .collect()
            .await;
        let all: Vec<String> = heads.iter().flat_map(|(_, t)| t.iter().cloned()).collect();
        let ents = self.entities(&all).await;
        let by_uri: HashMap<&str, &Value> = all.iter().map(String::as_str).zip(ents.iter()).collect();
        let mut m = self.mosaics.lock().unwrap();
        for (uri, tracks) in &heads {
            if let Some(url) = mosaic(tracks.iter().filter_map(|t| by_uri.get(t.as_str()).copied())) {
                m.insert(uri.clone(), url);
            }
        }
    }

    pub(super) async fn playlist(&self, id: &str) -> Result<Arc<Playlist>> {
        if let Some((at, p)) = self.playlists.lock().unwrap().get(id)
            && at.elapsed() < LIST_TTL
        {
            return Ok(p.clone());
        }
        let p = Arc::new(self.fetch_playlist(id).await?);
        let mut lists = self.playlists.lock().unwrap();
        // Expired ones go, so the cache stays small.
        lists.retain(|_, (at, _)| at.elapsed() < LIST_TTL);
        lists.insert(id.to_string(), (Instant::now(), p.clone()));
        Ok(p)
    }

    async fn fetch_playlist(&self, id: &str) -> Result<Playlist> {
        let mut head: Option<SelectedListContent> = None;
        let mut items = Vec::new();
        loop {
            let path = format!("/playlist/v2/playlist/{id}?from={}&length=1000", items.len());
            let raw = self.session.spclient().request(&Method::GET, &path, None, None).await.with_context(|| format!("playlist {id}"))?;
            let page = SelectedListContent::parse_from_bytes(&raw)?;
            let got = page.contents.items.len();
            items.extend(page.contents.items.iter().map(|i| (i.uri().to_string(), i.attributes.added_by().to_string(), i.attributes.timestamp())));
            let total = page.length().max(0) as usize;
            if head.is_none() {
                head = Some(page);
            }
            if got == 0 || items.len() >= total {
                break;
            }
        }
        let head = head.context("empty playlist reply")?;
        let uri = format!("spotify:playlist:{id}");
        if head.attributes.picture().is_empty() && head.attributes.picture_size.is_empty() && !self.mosaics.lock().unwrap().contains_key(&uri) {
            let first: Vec<String> = items.iter().map(|i| i.0.clone()).filter(|u| u.starts_with("spotify:track:")).take(16).collect();
            let ents = self.entities(&first).await;
            if let Some(url) = mosaic(ents.iter()) {
                self.mosaics.lock().unwrap().insert(uri.clone(), url);
            }
        }
        let mut json = self.playlist_json(&uri, &head.attributes, head.owner_username(), head.length());
        json["snapshot_id"] = json!(conv::hex(head.revision()));
        Ok(Playlist { json, items })
    }

    /// Every song in the user's playlists (their own and ones they follow,
    /// not Spotify's editorial and made-for-you lists) and Liked Songs, as
    /// compact `[uri, title, artists]` rows for the assistant's picture of
    /// what they listen to. Names come from the metadata cache where present;
    /// the rest are fetched without filling it, so pages stay cached.
    pub(super) async fn library_songs(&self) -> Result<Value> {
        const MAX_SONGS: usize = 40_000;
        let root = self.rootlist().await?;
        let lists: Vec<String> = root.playlists.iter().filter(|p| p["owner"]["id"] != "spotify").filter_map(|p| p["id"].as_str().map(str::to_string)).take(400).collect();
        let n_lists = lists.len();
        let pages: Vec<Vec<String>> =
            stream::iter(lists).map(|id| async move { self.fetch_playlist(&id).await.map(|p| p.items.into_iter().map(|i| i.0).collect()).unwrap_or_default() }).buffer_unordered(6).collect().await;
        let liked: Vec<String> = self.set("collection").await.map(|s| s.iter().map(|x| x.uri.clone()).collect()).unwrap_or_default();
        let n_liked = liked.iter().filter(|u| u.starts_with("spotify:track:")).count();
        let mut seen = HashSet::new();
        let uris: Vec<String> = liked.into_iter().chain(pages.into_iter().flatten()).filter(|u| u.starts_with("spotify:track:") && seen.insert(u.clone())).take(MAX_SONGS).collect();
        let row = |uri: &str, v: &Value| {
            let artists = v["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            json!([uri, v["name"].as_str().unwrap_or(""), artists])
        };
        let (cached, missing) = {
            let m = self.meta.lock().unwrap();
            let mut cached = Vec::new();
            let mut missing = Vec::new();
            for u in &uris {
                match m.get(u) {
                    Some((_, meta)) => cached.push((u, meta.clone())),
                    None => missing.push(u.clone()),
                }
            }
            (cached, missing)
        };
        let mut rows = Vec::with_capacity(uris.len());
        for (u, meta) in cached {
            match meta {
                super::Meta::Json(v) => rows.push(row(u, &v)),
                super::Meta::Song(b) => {
                    if let Some(v) = super::convert(u, &b) {
                        rows.push(row(u, &v));
                    }
                }
            }
        }
        // A thousand at a time, each turned into its row as it arrives: the raw metadata
        // for tens of thousands of songs is never all in memory at once.
        for chunk in missing.chunks(1000) {
            for (uri, bytes) in self.raw_meta(chunk).await {
                if let Some(v) = super::convert(&uri, &bytes) {
                    rows.push(row(&uri, &v));
                }
            }
        }
        Ok(json!({ "songs": rows, "playlists": n_lists, "liked": n_liked }))
    }

    pub(super) async fn playlist_items_json(&self, id: &str, cap: usize) -> Result<Value> {
        let pl = self.playlist(id).await?;
        let rows: Vec<&(String, String, i64)> = pl.items.iter().take(cap).collect();
        let uris: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let ents = self.entities(&uris).await;
        let items: Vec<Value> = rows
            .iter()
            .zip(ents)
            .map(|((uri, by, at), t)| {
                json!({ "added_at": if *at > 0 { conv::iso(*at) } else { String::new() }, "added_by": { "id": by, "type": "user" }, "is_local": uri.starts_with("spotify:local:"), "track": t })
            })
            .collect();
        Ok(json!({ "items": items, "total": pl.items.len() }))
    }

    // ------------------------------------------------------------ edits

    async fn post_changes(&self, path: &str, revision: Vec<u8>, ops: Vec<Op>) -> Result<()> {
        let mut delta = Delta::new();
        delta.ops = ops;
        let mut ch = ListChanges::new();
        ch.set_base_revision(revision);
        ch.deltas.push(delta);
        let mut h = HeaderMap::new();
        h.insert("content-type", "application/x-protobuf".parse()?);
        self.session.spclient().request(&Method::POST, path, Some(h), Some(&ch.write_to_bytes()?)).await?;
        Ok(())
    }

    pub(super) async fn change_playlist(&self, id: &str, ops: Vec<Op>) -> Result<()> {
        self.playlists.lock().unwrap().remove(id);
        // Only the revision is needed: the first item's page says it, not the whole playlist.
        let raw = self.session.spclient().request(&Method::GET, &format!("/playlist/v2/playlist/{id}?from=0&length=1"), None, None).await.with_context(|| format!("playlist {id}"))?;
        let revision = SelectedListContent::parse_from_bytes(&raw)?.revision().to_vec();
        self.post_changes(&format!("/playlist/v2/playlist/{id}/changes"), revision, ops).await?;
        self.playlists.lock().unwrap().remove(id);
        *self.rootlist.lock().unwrap() = None;
        Ok(())
    }

    async fn change_rootlist(&self, ops: Vec<Op>) -> Result<()> {
        let list = self.fetch_rootlist().await?;
        self.post_changes(&format!("/playlist/v2/user/{}/rootlist/changes", self.user), list.revision().to_vec(), ops).await?;
        *self.rootlist.lock().unwrap() = None;
        Ok(())
    }

    pub(super) async fn follow_playlist(&self, uri: &str, follow: bool) -> Result<()> {
        self.change_rootlist(vec![if follow { add_op(vec![item(uri, None)], true) } else { rem_op(&[uri.to_string()]) }]).await
    }

    pub(super) async fn add_to_playlist(&self, id: &str, uris: &[String]) -> Result<()> {
        let items = uris.iter().map(|u| item(u, Some(&self.user))).collect();
        self.change_playlist(id, vec![add_op(items, false)]).await
    }

    pub(super) async fn remove_from_playlist(&self, id: &str, uris: &[String]) -> Result<()> {
        self.change_playlist(id, vec![rem_op(uris)]).await
    }

    pub(super) async fn move_in_playlist(&self, id: &str, from: i32, insert_before: i32) -> Result<()> {
        let mut m = Mov::new();
        m.set_from_index(from);
        m.set_length(1);
        m.set_to_index(insert_before);
        let mut o = Op::new();
        o.set_kind(op::Kind::MOV);
        o.mov = MessageField::some(m);
        self.change_playlist(id, vec![o]).await
    }

    pub(super) async fn edit_playlist(&self, id: &str, body: &Value) -> Result<()> {
        let mut values = ListAttributes::new();
        if let Some(n) = body["name"].as_str() {
            values.set_name(n.into());
        }
        if let Some(d) = body["description"].as_str() {
            values.set_description(d.into());
        }
        if let Some(c) = body["collaborative"].as_bool() {
            values.set_collaborative(c);
        }
        let mut state = ListAttributesPartialState::new();
        state.values = MessageField::some(values);
        let mut upd = UpdateListAttributes::new();
        upd.new_attributes = MessageField::some(state);
        let mut o = Op::new();
        o.set_kind(op::Kind::UPDATE_LIST_ATTRIBUTES);
        o.update_list_attributes = MessageField::some(upd);
        self.change_playlist(id, vec![o]).await
    }

    /// A new, empty playlist at the top of your library. Returns its URI.
    pub(super) async fn create_playlist(&self, name: &str) -> Result<String> {
        let mut attrs = ListAttributes::new();
        attrs.set_name(name.into());
        let mut req = ListUpdateRequest::new();
        req.attributes = MessageField::some(attrs);
        let mut h = HeaderMap::new();
        h.insert("content-type", "application/x-protobuf".parse()?);
        let raw = self.session.spclient().request(&Method::POST, "/playlist/v2/playlist", Some(h), Some(&req.write_to_bytes()?)).await?;
        let uri = CreateListReply::parse_from_bytes(&raw)?.uri().to_string();
        self.follow_playlist(&uri, true).await?;
        Ok(uri)
    }
}

fn add_op(items: Vec<Item>, first: bool) -> Op {
    let mut a = Add::new();
    a.items = items;
    if first {
        a.set_from_index(0);
        a.set_add_first(true);
    } else {
        a.set_add_last(true);
    }
    let mut o = Op::new();
    o.set_kind(op::Kind::ADD);
    o.add = MessageField::some(a);
    o
}

fn rem_op(uris: &[String]) -> Op {
    let mut r = Rem::new();
    r.items = uris.iter().map(|u| item(u, None)).collect();
    r.set_items_as_key(true);
    let mut o = Op::new();
    o.set_kind(op::Kind::REM);
    o.rem = MessageField::some(r);
    o
}

/// A 2x2 grid of the first four different album covers, or the single cover
/// when there are fewer.
fn mosaic<'a>(tracks: impl Iterator<Item = &'a Value>) -> Option<String> {
    let mut ids: Vec<String> = Vec::new();
    for t in tracks {
        let url = t["album"]["images"].as_array().and_then(|a| a.iter().find(|i| i["width"] == 300).or(a.first())).and_then(|i| i["url"].as_str());
        if let Some(id) = url.and_then(|u| u.rsplit('/').next())
            && !ids.iter().any(|x| x == id)
        {
            ids.push(id.to_string());
        }
        if ids.len() == 4 {
            return Some(format!("https://mosaic.scdn.co/640/{}", ids.concat()));
        }
    }
    ids.first().map(|id| format!("https://i.scdn.co/image/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mosaics() {
        let t = |id: &str| json!({ "album": { "images": [{ "url": format!("https://i.scdn.co/image/{id}"), "width": 300 }] } });
        let four = [t("a"), t("b"), t("a"), t("c"), t("d")];
        assert_eq!(mosaic(four.iter()).as_deref(), Some("https://mosaic.scdn.co/640/abcd"));
        let two = [t("a"), t("b")];
        assert_eq!(mosaic(two.iter()).as_deref(), Some("https://i.scdn.co/image/a"));
        assert_eq!(mosaic([].iter()), None);
    }
}
