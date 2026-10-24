//! Live player state and remote control, through Spotify Connect.
//!
//! Glass registers a hidden observer device (`hobs_…`, the way Spotify's web
//! player does). Spotify then pushes the whole Connect cluster to it: what's
//! playing and where, the queue, and every device on the account. Each push is
//! passed on (`Sp::on_change`), so the app sees a change as it happens rather
//! than at its next look. Commands go to the active device through
//! `connect-state/v1/player/command`.

use super::Sp;
use anyhow::{Context as _, Result, anyhow, bail};
use futures_util::StreamExt;
use http::{HeaderMap, Method};
use librespot_core::dealer::protocol::Message;
use librespot_protocol::connect::{Cluster, ClusterUpdate, MemberType, PutStateReason, PutStateRequest};
use librespot_protocol::devices::DeviceType;
use protobuf::Message as _;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Keeps the cluster up to date for as long as `sp` is alive. If the
/// observer can't run, `sp` is marked for reconnecting rather than leaving
/// the queue, devices and player state empty for the whole session.
pub(super) fn start(sp: &Arc<Sp>) {
    let weak = Arc::downgrade(sp);
    tokio::spawn(async move {
        let result = observe(weak.clone()).await;
        if let Some(sp) = weak.upgrade()
            && sp.is_alive()
        {
            match result {
                Err(e) => log::warn!("connect observer stopped: {e:#}"),
                Ok(()) => log::warn!("connect observer stopped"),
            }
            sp.observer_failed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

async fn observe(sp: Weak<Sp>) -> Result<()> {
    let session = sp.upgrade().context("signed out")?.session.clone();
    let mut connections = session.dealer().listen_for("hm://pusher/v1/connections/", |m: Message| Ok(m.headers.get("Spotify-Connection-Id").cloned().unwrap_or_default()))?;
    let mut updates = session.dealer().listen_for("hm://connect-state/v1/cluster", Message::from_raw::<ClusterUpdate>)?;
    session.dealer().start().await?;
    loop {
        tokio::select! {
            Some(id) = connections.next() => {
                let Some(sp) = sp.upgrade() else { break };
                match id {
                    Ok(id) if !id.is_empty() => {
                        // Without registering, no player state arrives: retry a few times.
                        let mut tries = 0;
                        while let Err(e) = sp.register(&id).await {
                            tries += 1;
                            log::warn!("couldn't register the observer device: {e:#}");
                            if tries == 3 {
                                return Err(e.context("registering the observer device"));
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(2 * tries)).await;
                        }
                    }
                    _ => {}
                }
            }
            Some(update) = updates.next() => {
                let Some(sp) = sp.upgrade() else { break };
                if let Ok(u) = update
                    && let Some(c) = u.cluster.into_option() {
                        *sp.cluster.write().unwrap() = Some(Arc::new(c));
                        if let Some(f) = &sp.on_change {
                            f();
                        }
                    }
            }
            else => break,
        }
    }
    Ok(())
}

fn device_type(t: DeviceType) -> &'static str {
    match t {
        DeviceType::COMPUTER | DeviceType::CHROMEBOOK => "Computer",
        DeviceType::SMARTPHONE => "Smartphone",
        DeviceType::TABLET => "Tablet",
        DeviceType::TV | DeviceType::STB | DeviceType::CAST_VIDEO => "TV",
        DeviceType::AVR => "AVR",
        DeviceType::GAME_CONSOLE => "GameConsole",
        DeviceType::AUTOMOBILE | DeviceType::CAR_THING => "Automobile",
        DeviceType::SMARTWATCH => "Smartwatch",
        DeviceType::AUDIO_DONGLE | DeviceType::CAST_AUDIO | DeviceType::SPEAKER => "Speaker",
        _ => "Unknown",
    }
}

impl Sp {
    /// Announces the observer on a (new) dealer connection and takes the
    /// cluster from the reply.
    async fn register(&self, connection_id: &str) -> Result<()> {
        let mut req = PutStateRequest::new();
        req.member_type = MemberType::CONNECT_STATE.into();
        req.put_state_reason = PutStateReason::NEW_DEVICE.into();
        req.client_side_timestamp = now_ms() as u64;
        let info = req.device.mut_or_insert_default().device_info.mut_or_insert_default();
        info.name = "Glass".into();
        info.device_id = self.observer_id.clone();
        info.device_type = DeviceType::COMPUTER.into();
        let caps = info.capabilities.mut_or_insert_default();
        caps.hidden = true;
        caps.is_observable = true;
        let mut h = HeaderMap::new();
        h.insert("X-Spotify-Connection-Id", connection_id.parse()?);
        h.insert("content-type", "application/x-protobuf".parse()?);
        let path = format!("/connect-state/v1/devices/{}", self.observer_id);
        let raw = self.session.spclient().request(&Method::PUT, &path, Some(h), Some(&req.write_to_bytes()?)).await?;
        *self.cluster.write().unwrap() = Some(Arc::new(Cluster::parse_from_bytes(&raw)?));
        Ok(())
    }

    fn cluster(&self) -> Option<Arc<Cluster>> {
        self.cluster.read().unwrap().clone()
    }

    pub(super) fn active_device(&self) -> Option<String> {
        self.cluster().map(|c| c.active_device_id.clone()).filter(|d| !d.is_empty())
    }

    fn device_json(c: &Cluster, id: &str) -> Option<Value> {
        let d = c.device.get(id)?;
        Some(json!({
            "id": id, "name": d.name, "type": device_type(d.device_type.enum_value_or_default()),
            "is_active": c.active_device_id == id, "is_restricted": false, "is_private_session": d.is_private_session,
            "volume_percent": (d.volume as u64 * 100 + 32_767) / 65_535, "supports_volume": true,
        }))
    }

    /// `GET /me/player`: null when nothing is active, like the Web API's 204.
    pub(super) async fn player_json(&self) -> Result<Value> {
        let Some(c) = self.cluster() else { return Ok(Value::Null) };
        if c.active_device_id.is_empty() {
            return Ok(Value::Null);
        }
        let ps = &c.player_state;
        let playing = ps.is_playing && !ps.is_paused;
        let mut pos = ps.position_as_of_timestamp;
        if playing && ps.timestamp > 0 {
            pos += (now_ms() - ps.timestamp).max(0);
        }
        if ps.duration > 0 {
            pos = pos.min(ps.duration);
        }
        let item = if ps.track.uri.is_empty() { Value::Null } else { self.entities(std::slice::from_ref(&ps.track.uri)).await.remove(0) };
        let o = &ps.options;
        Ok(json!({
            "device": Self::device_json(&c, &c.active_device_id),
            "is_playing": playing,
            "progress_ms": pos.max(0),
            "shuffle_state": o.shuffling_context,
            "repeat_state": if o.repeating_track { "track" } else if o.repeating_context { "context" } else { "off" },
            "context": if ps.context_uri.is_empty() { Value::Null } else { json!({ "uri": ps.context_uri }) },
            "item": item,
            "currently_playing_type": ps.track.uri.split(':').nth(1).unwrap_or("track"),
        }))
    }

    pub(super) async fn queue_json(&self) -> Result<Value> {
        // No player state yet is "not ready", not "the queue is empty".
        let Some(c) = self.cluster() else { bail!("503 player state isn't ready yet") };
        let ps = &c.player_state;
        let next: Vec<String> = ps.next_tracks.iter().map(|t| t.uri.clone()).filter(|u| !u.is_empty() && !u.starts_with("spotify:delimiter") && !u.starts_with("spotify:meta")).take(40).collect();
        let mut all = vec![ps.track.uri.clone()];
        all.extend(next);
        let mut items = self.entities(&all).await;
        let current = items.remove(0);
        Ok(json!({ "currently_playing": if ps.track.uri.is_empty() { Value::Null } else { current }, "queue": items }))
    }

    pub(super) fn devices_json(&self) -> Result<Value> {
        let Some(c) = self.cluster() else { bail!("503 player state isn't ready yet") };
        let mut list: Vec<Value> = c.device.iter().filter(|(id, d)| **id != self.observer_id && !d.capabilities.hidden).filter_map(|(id, _)| Self::device_json(&c, id)).collect();
        list.sort_by(|a, b| b["is_active"].as_bool().cmp(&a["is_active"].as_bool()).then(a["name"].as_str().cmp(&b["name"].as_str())));
        Ok(json!({ "devices": list }))
    }

    async fn command(&self, to: &str, mut cmd: Value) -> Result<()> {
        cmd["logging_params"] = json!({
            "command_id": format!("{:032x}", rand::random::<u128>()),
            "command_initiated_time": now_ms(),
            "interaction_ids": [], "page_instance_ids": [],
        });
        let path = format!("/connect-state/v1/player/command/from/{}/to/{to}", self.observer_id);
        self.post_json(&path, &json!({ "command": cmd })).await
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<()> {
        let mut h = HeaderMap::new();
        h.insert("content-type", "application/json".parse()?);
        self.session.spclient().request(&Method::POST, path, Some(h), Some(body.to_string().as_bytes())).await?;
        Ok(())
    }

    /// Player endpoints of the Web API, sent as Connect commands.
    pub(super) async fn player_write(&self, method: &Method, rest: &[&str], q: &super::Query, body: Option<&Value>) -> Result<Value> {
        let target = q.get("device_id").filter(|d| !d.is_empty()).cloned().or_else(|| self.active_device());
        let to = || target.clone().ok_or_else(|| anyhow!("404 player: No active device"));
        match (method, rest) {
            (&Method::PUT, []) => {
                // Transfer playback: {"device_ids": [id], "play": bool}
                let body = body.cloned().unwrap_or_default();
                let dest = body["device_ids"][0].as_str().context("no device")?.to_string();
                let from = self.active_device().unwrap_or_else(|| dest.clone());
                let paused = if body["play"].as_bool().unwrap_or(true) { "resume" } else { "pause" };
                self.post_json(&format!("/connect-state/v1/connect/transfer/from/{from}/to/{dest}"), &json!({ "transfer_options": { "restore_paused": paused } })).await?;
            }
            (&Method::PUT, ["play"]) => {
                let to = to()?;
                match body.filter(|b| !b.is_null() && b.as_object().is_some_and(|o| !o.is_empty())) {
                    None => self.command(&to, json!({ "endpoint": "resume" })).await?,
                    Some(b) => self.command(&to, play_command(b)).await?,
                }
            }
            (&Method::PUT, ["pause"]) => self.command(&to()?, json!({ "endpoint": "pause" })).await?,
            (&Method::POST, ["next"]) => self.command(&to()?, json!({ "endpoint": "skip_next" })).await?,
            (&Method::POST, ["previous"]) => self.command(&to()?, json!({ "endpoint": "skip_prev" })).await?,
            (&Method::PUT, ["seek"]) => {
                let ms: u64 = q.get("position_ms").and_then(|v| v.parse().ok()).unwrap_or(0);
                self.command(&to()?, json!({ "endpoint": "seek_to", "value": ms, "position": ms })).await?;
            }
            (&Method::PUT, ["shuffle"]) => {
                let on = q.get("state").is_some_and(|s| s == "true");
                self.command(&to()?, json!({ "endpoint": "set_shuffling_context", "value": on })).await?;
            }
            (&Method::PUT, ["repeat"]) => {
                let state = q.get("state").map(String::as_str).unwrap_or("off");
                self.command(&to()?, json!({ "endpoint": "set_options", "repeating_context": state != "off", "repeating_track": state == "track" })).await?;
            }
            (&Method::PUT, ["volume"]) => {
                let pct: u64 = q.get("volume_percent").and_then(|v| v.parse().ok()).unwrap_or(50).min(100);
                let path = format!("/connect-state/v1/connect/volume/from/{}/to/{}", self.observer_id, to()?);
                let mut h = HeaderMap::new();
                h.insert("content-type", "application/json".parse()?);
                let body = json!({ "volume": pct * 65_535 / 100 }).to_string();
                self.session.spclient().request(&Method::PUT, &path, Some(h), Some(body.as_bytes())).await?;
            }
            (&Method::POST, ["queue"]) => {
                let uri = q.get("uri").context("no uri")?;
                self.command(&to()?, json!({ "endpoint": "add_to_queue", "track": { "uri": uri, "metadata": { "is_queued": "true" }, "provider": "queue" } })).await?;
            }
            _ => bail!("404 {}: not available", rest.join("/")),
        }
        Ok(Value::Null)
    }
}

/// A Web API play body (`context_uri` / `uris`, `offset`, `position_ms`)
/// as a Connect `play` command.
fn play_command(b: &Value) -> Value {
    let (context, skip_to) = if let Some(ctx) = b["context_uri"].as_str() {
        let skip = match (&b["offset"]["uri"], &b["offset"]["position"]) {
            (Value::String(u), _) => json!({ "track_uri": u }),
            (_, Value::Number(n)) => json!({ "track_index": n }),
            _ => Value::Null,
        };
        (json!({ "uri": ctx, "url": format!("context://{ctx}"), "metadata": {} }), skip)
    } else {
        let uris: Vec<&str> = b["uris"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let first = uris.first().copied().unwrap_or("");
        let tracks: Vec<Value> = uris.iter().map(|u| json!({ "uri": u })).collect();
        let index = b["offset"]["position"].as_u64().unwrap_or(0);
        (json!({ "uri": first, "url": format!("context://{first}"), "metadata": {}, "pages": [{ "tracks": tracks }] }), json!({ "track_index": index }))
    };
    let mut options = json!({ "license": "on-demand", "player_options_override": {} });
    if !skip_to.is_null() {
        options["skip_to"] = skip_to;
    }
    if let Some(ms) = b["position_ms"].as_u64() {
        options["seek_to"] = json!(ms);
    }
    json!({ "endpoint": "play", "context": context, "play_origin": { "feature_identifier": "glass", "feature_version": env!("CARGO_PKG_VERSION") }, "options": options })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_bodies() {
        let c = play_command(&json!({ "context_uri": "spotify:album:x", "offset": { "uri": "spotify:track:y" }, "position_ms": 5 }));
        assert_eq!(c["endpoint"], "play");
        assert_eq!(c["context"]["url"], "context://spotify:album:x");
        assert_eq!(c["options"]["skip_to"]["track_uri"], "spotify:track:y");
        assert_eq!(c["options"]["seek_to"], 5);
        let u = play_command(&json!({ "uris": ["spotify:track:a", "spotify:track:b"], "offset": { "position": 1 } }));
        assert_eq!(u["context"]["pages"][0]["tracks"][1]["uri"], "spotify:track:b");
        assert_eq!(u["options"]["skip_to"]["track_index"], 1);
    }
}
