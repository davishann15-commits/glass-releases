//! Local files with Glass on your other devices (src/share.rs: Noise XX, the
//! account's secret as proof, protocol 4), driven as the desktop does (src/app.rs
//! `keep_sharing`, `share_event`). On by default, nothing to set up; runs while the
//! app is in front (iOS suspends a backgrounded app's listener, so transfers happen
//! with Glass open). Discovery is Bonjour through DNS-SD (see bonjour.rs). Another
//! device's songs can be listed and the ones picked fetched (`share_browse`,
//! `share_get`).

use crate::app::{MobileApp, Msg};
use crate::bridge::CoreEvent;
use crate::model::s;
use crate::share::{Event, FileMeta, State};
use std::collections::HashMap;

#[derive(Default)]
pub struct ShareState {
    pub share: Option<crate::share::Share>,
    failed: Option<String>,
    pub peers: Vec<crate::share::Peer>,
    pub transfers: Vec<crate::share::Transfer>,
    pub library: crate::share::Library,
    /// A playlist whose missing local songs were asked for before its songs loaded.
    pub missing_for: Option<String>,
    /// The other device whose songs are on screen.
    browsing: Option<Browsing>,
    published: Option<ShareModel>,
    published_library: Option<DeviceLibraryModel>,
}

struct Browsing {
    peer: String,
    name: String,
    /// `None` while they're asked for.
    songs: Option<Result<Vec<FileMeta>, String>>,
    /// Which of them are on this device already.
    here: Vec<bool>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct PeerModel {
    pub id: String,
    pub name: String,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct TransferModel {
    /// "Sending 3 of 12 to Arch Desktop", "Got 12 songs from Arch Desktop"…
    pub text: String,
    /// 0–1 while running.
    pub progress: Option<f32>,
    pub failed: bool,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct ShareModel {
    pub on: bool,
    pub running: bool,
    /// Why it couldn't start (local network access refused, say).
    pub error: Option<String>,
    pub peers: Vec<PeerModel>,
    /// The last few, newest first.
    pub transfers: Vec<TransferModel>,
    pub inbox: String,
}

/// Another device's Local Files, as listed by it.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct DeviceLibraryModel {
    pub peer: String,
    pub name: String,
    pub loading: bool,
    /// Why they couldn't be listed (in plain words).
    pub error: Option<String>,
    pub songs: Vec<DeviceSongModel>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct DeviceSongModel {
    /// What `share_get` takes.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub size: u64,
    /// On this device already (it isn't fetched again).
    pub here: bool,
}

fn transfer_text(t: &crate::share::Transfer) -> TransferModel {
    let what = |n: usize| if n == 1 { "a song".to_string() } else { crate::model::songs(n as u64) };
    let at = t.done.min(t.files.saturating_sub(1)) + 1;
    let text = match (&t.state, t.sending) {
        (State::Running, true) if t.files == 1 => format!("Sending a song to {}", t.peer),
        (State::Running, false) if t.files == 1 => format!("Receiving a song from {}", t.peer),
        (State::Running, true) => format!("Sending {at} of {} to {}", t.files, t.peer),
        (State::Running, false) => format!("Receiving {at} of {} from {}", t.files, t.peer),
        (State::Done, true) => format!("Sent {} to {}", what(t.done), t.peer),
        (State::Done, false) => format!("Got {} from {}", what(t.done), t.peer),
        (State::Failed(_), true) => format!("Sending to {} stopped", t.peer),
        (State::Failed(_), false) => format!("Getting songs from {} stopped", t.peer),
    };
    TransferModel { text, progress: (t.state == State::Running && t.total > 0).then(|| t.bytes as f32 / t.total as f32), failed: matches!(t.state, State::Failed(_)) }
}

impl MobileApp {
    /// Desktop `keep_sharing`: starts or stops sharing to match the setting, once
    /// signed in (the account is what proves a device is yours).
    pub fn keep_sharing(&mut self) {
        let want = self.settings.share_local_files && !self.demo && self.foreground;
        if !want {
            if self.shares.share.take().is_some() {
                self.shares.peers.clear();
            }
            if !self.settings.share_local_files {
                self.shares.failed = None;
            }
            return;
        }
        if self.shares.share.is_some() || self.shares.failed.is_some() {
            return;
        }
        let Some(me) = self.me.as_ref() else { return };
        let username = s(me, "id");
        if username.is_empty() {
            return;
        }
        crate::platform::set_share_name(&self.settings.device_name);
        let instance = crate::model::random_hex(12);
        let who = crate::share::Me { instance, name: crate::share::computer_name(), account: crate::share::account_secret(username, s(me, "email")) };
        if let Ok(mut lib) = self.shares.library.write() {
            *lib = self.local_tracks.clone().unwrap_or_default();
        }
        let tx = self.tx.clone();
        let inbox = crate::share::inbox();
        let _ = std::fs::create_dir_all(&inbox);
        match crate::share::Share::start(self.rt.clone(), who, self.shares.library.clone(), inbox, move |e| tx.send(Msg::Share(e))) {
            Ok(share) => self.shares.share = Some(share),
            Err(e) => {
                log::warn!("local files sharing didn't start: {e:#}");
                self.shares.failed = Some("Glass couldn't use the network here. Check that Local Network is on for Glass in Settings › Privacy & Security.".into());
            }
        }
    }

    /// Desktop `share_event`.
    pub fn share_event(&mut self, e: Event) {
        match e {
            Event::Peers(p) => self.shares.peers = p,
            Event::Library { peer, songs } => {
                if let Some(b) = self.shares.browsing.as_mut().filter(|b| b.peer == peer) {
                    b.songs = Some(songs.map_err(|_| format!("Couldn't see the songs on {}. Check that it's on, has Glass open and is on the same Wi-Fi.", b.name)));
                }
                self.mark_browsed_here();
            }
            Event::Transfer(t) => {
                if matches!(t.state, State::Failed(_)) {
                    // What went wrong is in the log; the user needs to know only that it stopped.
                    self.toast(if t.sending { format!("Sending to {} stopped. Try again with both devices on the same Wi-Fi.", t.peer) } else { format!("Getting songs from {} stopped. Try again with both devices on the same Wi-Fi.", t.peer) });
                } else if t.state == State::Done && t.sending && t.files > 0 {
                    self.toast(format!("Sent {} {} to {}", t.files, if t.files == 1 { "song" } else { "songs" }, t.peer));
                } else if t.state == State::Done && t.sending && t.files == 0 {
                    self.toast(format!("{} already has those songs", t.peer));
                }
                match self.shares.transfers.iter_mut().find(|x| x.id == t.id && x.sending == t.sending && x.peer == t.peer) {
                    Some(x) => *x = t,
                    None => {
                        self.shares.transfers.push(t);
                        if self.shares.transfers.len() > 8 {
                            self.shares.transfers.remove(0);
                        }
                    }
                }
            }
            Event::Received { from, paths } => {
                let n = paths.len();
                // The inbox is Documents, which Local Files reads: look again.
                if crate::local::add_source(&mut self.settings.local_dirs, crate::share::inbox()) {
                    self.mark_settings_dirty();
                }
                self.apply(crate::app::Action::RescanLocal);
                self.toast(if n == 1 { format!("Got a song from {from}") } else { format!("Got {n} songs from {from}") });
            }
        }
    }

    /// Shows `peer`'s songs (asked for now; `DeviceLibrary` events follow).
    pub fn share_browse(&mut self, peer: String) {
        let Some(p) = self.shares.peers.iter().find(|p| p.id == peer).cloned() else {
            self.shares.browsing = Some(Browsing { name: "That device".into(), peer, songs: Some(Err("That device isn't on this network any more.".into())), here: Vec::new() });
            return;
        };
        self.shares.browsing = Some(Browsing { peer: p.id.clone(), name: p.name.clone(), songs: None, here: Vec::new() });
        if self.demo {
            self.share_event(Event::Library { peer: p.id, songs: Ok(crate::demo::device_songs()) });
        } else if let Some(share) = &self.shares.share {
            share.browse(p);
        } else if let Some(b) = self.shares.browsing.as_mut() {
            b.songs = Some(Err("Turn on Share with your other devices in Settings › Your Library.".into()));
        }
    }

    /// Gets the picked songs (`DeviceSongModel::id`s) from the device being browsed.
    pub fn share_get(&mut self, peer: String, ids: Vec<String>) {
        if self.demo {
            let n = ids.len();
            return self.toast(if n == 1 { "Getting a song from Arch Desktop…".to_string() } else { format!("Getting {n} songs from Arch Desktop…") });
        }
        let (Some(share), Some(p)) = (self.shares.share.as_ref(), self.shares.peers.iter().find(|p| p.id == peer).cloned()) else {
            self.toast("That device isn't on this network any more");
            return;
        };
        let n = ids.len();
        share.get(p.clone(), ids);
        self.toast(if n == 1 { format!("Getting a song from {}…", p.name) } else { format!("Getting {n} songs from {}…", p.name) });
    }

    /// Marks which of the browsed device's songs are on this device (after a list
    /// arrives, and as songs arrive here). By title first: there are thousands.
    fn mark_browsed_here(&mut self) {
        let Some(b) = self.shares.browsing.as_mut() else { return };
        let Some(Ok(songs)) = &b.songs else { return };
        let mine = self.local_tracks.clone().unwrap_or_default();
        let norm = crate::lyrics::matching::normalize;
        let mut by_title: HashMap<String, Vec<crate::local::LocalTrack>> = HashMap::new();
        for t in mine.iter() {
            by_title.entry(norm(&t.title)).or_default().push(t.clone());
        }
        b.here = songs.iter().map(|f| by_title.get(&norm(&f.title)).is_some_and(|same| crate::local::find(same, &f.spotify_uri()).is_some())).collect();
    }

    pub fn send_to_device(&mut self, peer: String, paths: Vec<String>) {
        let (Some(share), Some(p)) = (self.shares.share.as_ref(), self.shares.peers.iter().find(|p| p.id == peer).cloned()) else {
            self.toast("That device isn't on this network any more");
            return;
        };
        let all = self.local_tracks.clone().unwrap_or_default();
        let wanted: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        let tracks: Vec<crate::local::LocalTrack> = all.iter().filter(|t| wanted.contains(t.path.to_string_lossy().as_ref())).cloned().collect();
        let n = tracks.len();
        share.send(p.clone(), tracks);
        self.toast(if n == 1 { format!("Sending to {}…", p.name) } else { format!("Sending {n} songs to {}…", p.name) });
    }

    pub fn get_from_devices(&mut self, uris: Vec<String>) {
        match self.shares.share.as_ref() {
            Some(share) if !self.shares.peers.is_empty() => {
                let n = uris.len();
                share.fetch(uris);
                self.toast(if n == 1 { "Getting it from your other devices…".to_string() } else { format!("Getting {n} songs from your other devices…") });
            }
            _ => self.toast("None of your other devices with Glass are on this Wi-Fi"),
        }
    }

    /// A playlist's `spotify:local:` songs that aren't on this device (desktop `missing_local`).
    pub fn missing_local(&mut self, uris: &[String]) -> Vec<String> {
        uris.iter().filter(|u| u.starts_with("spotify:local:") && matches!(self.local_link(u), Some(None))).cloned().collect()
    }

    /// The songs of the playlist in `missing_for`, once loaded: asks the other devices
    /// for the local ones that aren't here.
    pub fn get_missing_now(&mut self) {
        let Some(id) = self.shares.missing_for.clone() else { return };
        let Some(v) = self.want_all(&format!("/playlists/{id}/tracks?limit=100"), 10_000) else { return };
        self.shares.missing_for = None;
        let uris: Vec<String> = crate::model::items(&v).iter().map(|i| s(crate::model::playlist_entry(i), "uri").to_string()).filter(|u| !u.is_empty()).collect();
        let missing = self.missing_local(&uris);
        if missing.is_empty() {
            self.toast("This playlist's local songs are all on this device");
        } else {
            self.get_from_devices(missing);
        }
    }

    /// Added files are read: put them where they were meant to go (desktop `files_read`).
    pub fn files_read(&mut self, target: crate::types::FileTarget, paths: Vec<std::path::PathBuf>, tracks: Vec<crate::local::LocalTrack>) {
        use crate::types::FileTarget;
        // As Local Files will list them (iOS hands over /private/var/… for /var/…).
        let paths: Vec<std::path::PathBuf> = paths.iter().map(|p| crate::paths::rebase(p)).collect();
        let tracks: Vec<crate::local::LocalTrack> = tracks
            .into_iter()
            .map(|mut t| {
                t.path = crate::paths::rebase(&t.path);
                t
            })
            .collect();
        // Each file plays from Local Files: its folder becomes a source.
        let new = crate::local::missing_sources(&paths, &self.settings.local_dirs);
        if !new.is_empty() {
            self.settings.local_dirs.extend(new);
            self.settings.local_dirs = crate::local::tidy_sources(&self.settings.local_dirs);
            self.mark_settings_dirty();
        }
        self.apply(crate::app::Action::RescanLocal);
        if tracks.is_empty() {
            self.toast("Nothing there Glass can play (it takes MP3, M4A, AAC, FLAC, Ogg, WAV and AIFF files)");
            return;
        }
        let n = tracks.len();
        match target {
            FileTarget::Spotify { id, .. } => self.apply(crate::app::Action::AddToPlaylist { playlist: id, uris: tracks.iter().map(crate::local::spotify_uri).collect() }),
            FileTarget::Local { id, .. } => self.apply(crate::app::Action::AddToLocalPlaylist { id, paths: tracks.into_iter().map(|t| t.path.to_string_lossy().into_owned()).collect() }),
            FileTarget::LocalFiles => self.toast(if n == 1 { "Added to Local Files".to_string() } else { format!("Added {n} songs to Local Files") }),
        }
    }

    /// Keeps sharing's view of Local Files current (it answers "do you have…?" from it).
    pub fn share_library_changed(&mut self) {
        if let Ok(mut lib) = self.shares.library.write() {
            *lib = self.local_tracks.clone().unwrap_or_default();
        }
        self.mark_browsed_here();
    }

    pub fn publish_share(&mut self, events: &mut Vec<CoreEvent>) {
        let m = ShareModel {
            on: self.settings.share_local_files,
            running: self.shares.share.is_some(),
            error: self.shares.failed.clone(),
            peers: self.shares.peers.iter().map(|p| PeerModel { id: p.id.clone(), name: p.name.clone() }).collect(),
            transfers: self.shares.transfers.iter().rev().take(3).map(transfer_text).collect(),
            inbox: crate::share::inbox().to_string_lossy().into_owned(),
        };
        if self.shares.published.as_ref() != Some(&m) {
            self.shares.published = Some(m.clone());
            events.push(CoreEvent::Share { share: m });
        }
        // The browsed device's songs, only when they change (there can be thousands).
        let lib = self.shares.browsing.as_ref().map(|b| DeviceLibraryModel {
            peer: b.peer.clone(),
            name: b.name.clone(),
            loading: b.songs.is_none(),
            error: b.songs.as_ref().and_then(|r| r.as_ref().err().cloned()),
            songs: match &b.songs {
                Some(Ok(songs)) => songs
                    .iter()
                    .enumerate()
                    .map(|(i, f)| DeviceSongModel { id: f.id.clone(), title: f.title.clone(), artist: f.artist.clone(), album: f.album.clone(), duration_ms: f.duration_ms, size: f.size, here: b.here.get(i).copied().unwrap_or(false) })
                    .collect(),
                _ => Vec::new(),
            },
        });
        if let Some(lib) = lib
            && self.shares.published_library.as_ref() != Some(&lib)
        {
            self.shares.published_library = Some(lib.clone());
            events.push(CoreEvent::DeviceLibrary { library: lib });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::{State, Transfer};

    fn t(sending: bool, files: usize, done: usize, state: State) -> Transfer {
        Transfer { id: 1, peer: "Arch Desktop".into(), sending, files, done, bytes: 50, total: 100, state }
    }

    #[test]
    fn transfers_in_words() {
        let m = transfer_text(&t(true, 12, 2, State::Running));
        assert_eq!(m.text, "Sending 3 of 12 to Arch Desktop");
        assert_eq!(m.progress, Some(0.5));
        assert_eq!(transfer_text(&t(false, 1, 0, State::Running)).text, "Receiving a song from Arch Desktop");
        assert_eq!(transfer_text(&t(false, 12, 12, State::Done)).text, "Got 12 songs from Arch Desktop");
        let f = transfer_text(&t(true, 3, 1, State::Failed("the other side closed".into())));
        assert!(f.failed && f.progress.is_none());
        assert_eq!(f.text, "Sending to Arch Desktop stopped", "no error text from the network");
    }
}
