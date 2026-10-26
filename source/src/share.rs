//! Local files between your devices running Glass, on the same network.
//!
//! On unless turned off in Settings, with nothing to set up: each Glass
//! announces itself over mDNS (Bonjour) as `_glass-files._tcp`, finds the
//! others, and shows the ones signed in to the same Spotify account. Any of
//! them can list another's Local Files and get the songs picked, send songs
//! over, or ask for a playlist's local songs that aren't here.
//!
//! What keeps others out without a code to compare: the account. Every device
//! signed in to it knows its username and email address (`account_secret`),
//! and nothing else on the network does. The announcement carries a tag made
//! with it (an HMAC of a salt that's new each run, so the tag can't follow a
//! device around), and a transfer runs over a Noise (XX) encrypted connection
//! where, before anything else, each side proves it knows the secret: an HMAC
//! of the connection's handshake hash, so a proof fits only that one connection
//! (it can't be replayed or relayed). The secret itself is never sent. A
//! connection that hasn't proved itself yet is read only a small hello, and at
//! most eight connections are served at once.
//!
//! Requests: `List` (your Local Files, please), `Get` (these of them), `Send`
//! (here are songs for you) and `Want` (do you have these `spotify:local:`
//! songs?, for a playlist's missing local songs). Received songs are saved in
//! one folder, which joins Local Files so they play; songs already here are
//! left out.

use crate::local::{self, LocalTrack};
use anyhow::{Context, Result, anyhow, bail};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const SERVICE: &str = "_glass-files._tcp.local.";
/// The port transfers use when it's free (any port otherwise), so a firewall
/// can let them through: TCP 47321, plus mDNS (UDP 5353).
pub const PORT: u16 = 47321;
const NOISE: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
/// 2: pairing, and the salted account tag. 3: a random value from each side
/// in the pairing code. 4: no pairing; the account secret proves it; `List`
/// and `Get`.
pub const VERSION: u32 = 4;
/// Plaintext per encrypted message (Noise messages are at most 64 KiB).
const CHUNK: usize = 60 * 1024;
const MAX_FILE: u64 = 4 << 30;
const MAX_FILES: usize = 20_000;
/// Give up on a connection that says nothing for this long.
const QUIET: Duration = Duration::from_secs(30);

/// Another Glass on this network, signed in to the same account.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    /// Its announcement's random ID (new each run).
    pub id: String,
    pub name: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
    /// The sharing protocol it speaks. Only this one's (`VERSION`) can share; a
    /// Glass that's older or newer is listed to say which one needs updating.
    pub version: u32,
}

impl Peer {
    /// It speaks this Glass's protocol: songs can go both ways.
    pub fn ready(&self) -> bool {
        self.version == VERSION
    }

    /// Why it can't share, when it can't: which Glass needs updating.
    pub fn update_note(&self) -> Option<String> {
        match self.version.cmp(&VERSION) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Less => Some(format!("{} has an older Glass. Update Glass there to share with it.", self.name)),
            std::cmp::Ordering::Greater => Some(format!("{} has a newer Glass. Update Glass here to share with it.", self.name)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Running,
    Done,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Transfer {
    pub id: u64,
    /// The other device's name.
    pub peer: String,
    pub sending: bool,
    pub files: usize,
    pub done: usize,
    pub bytes: u64,
    pub total: u64,
    pub state: State,
}

pub enum Event {
    /// The devices on the network, now.
    Peers(Vec<Peer>),
    Transfer(Transfer),
    /// Songs saved here (Local Files should look again), also when a transfer
    /// broke off after some of them.
    Received {
        from: String,
        paths: Vec<PathBuf>,
    },
    /// Another device's Local Files (the answer to `Share::browse`).
    Library {
        peer: String,
        songs: Result<Vec<FileMeta>, String>,
    },
    /// iOS: the user hasn't allowed Glass on the local network, so it can't look
    /// for other devices.
    #[cfg_attr(not(glass_mobile), allow(dead_code))]
    NoLocalNetwork,
}

/// Another device's Local Files, being looked at to get some of its songs.
pub struct Browsed {
    /// Its `Peer::id`.
    pub peer: String,
    pub name: String,
    /// `None` while they're asked for; the error in plain words.
    pub songs: Option<Result<Vec<FileMeta>, String>>,
    /// Which of them are here already (not fetched again), by index.
    pub here: Vec<bool>,
}

impl Browsed {
    pub fn new(peer: &Peer) -> Self {
        Self { peer: peer.id.clone(), name: peer.name.clone(), songs: None, here: Vec::new() }
    }

    /// Takes `Event::Library`'s answer (if it's this device's).
    pub fn answered(&mut self, peer: &str, songs: Result<Vec<FileMeta>, String>, mine: &[LocalTrack]) {
        if peer != self.peer {
            return;
        }
        self.songs = Some(songs.map_err(|_| format!("Couldn't see the songs on {}. Check that Glass is open on it and that it's on the same Wi-Fi.", self.name)));
        self.mark_here(mine);
    }

    /// Marks which of its songs this device has (after a list arrives, and as songs
    /// arrive here). By title first: there can be thousands on each side.
    pub fn mark_here(&mut self, mine: &[LocalTrack]) {
        let Some(Ok(songs)) = &self.songs else { return };
        let norm = crate::lyrics::matching::normalize;
        let mut by_title: HashMap<String, Vec<LocalTrack>> = HashMap::new();
        for t in mine {
            by_title.entry(norm(&t.title)).or_default().push(t.clone());
        }
        self.here = songs.iter().map(|f| by_title.get(&norm(&f.title)).is_some_and(|same| local::find(same, &f.spotify_uri()).is_some())).collect();
    }

    /// The listed songs (with whether each is here), once they've come.
    pub fn list(&self) -> impl Iterator<Item = (&FileMeta, bool)> {
        let songs = match &self.songs {
            Some(Ok(s)) => s.as_slice(),
            _ => &[],
        };
        songs.iter().enumerate().map(|(i, f)| (f, self.here.get(i).copied().unwrap_or(false)))
    }
}

/// Who this Glass is.
#[derive(Clone)]
pub struct Me {
    /// Random, new each run: names the announcement.
    pub instance: String,
    /// This device's name, as other devices show it.
    pub name: String,
    /// What only the account's devices know (`account_secret`). Never sent: only
    /// tags and proofs made with it.
    pub account: String,
    /// The account's username: Glass before protocol 4 tagged its announcement with
    /// it (recognised only to say that Glass needs updating).
    pub username: String,
}

/// What a connection needs to know.
#[derive(Clone)]
struct Ctx {
    me: Me,
    /// `account_tag` for this run.
    tag: String,
    key: Arc<snow::Keypair>,
    /// Local Files as last scanned (to answer `List`, `Get` and `Want`, and to
    /// skip songs already here).
    library: Library,
    /// Where received songs go.
    inbox: PathBuf,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
    next_id: Arc<AtomicU64>,
    /// Connections and transfers under way, stopped when sharing is turned off.
    running: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
    /// The devices found, and which of them answer.
    peers: Arc<Mutex<Found>>,
}

/// The devices announced on the network, by announcement ID, and the ones that
/// answered when last asked (only those are listed).
#[derive(Default)]
struct Found {
    all: HashMap<String, Peer>,
    answering: HashSet<String>,
}

impl Ctx {
    /// Keeps `task` to be stopped with the rest when sharing stops.
    fn track(&self, task: tokio::task::JoinHandle<()>) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|t| !t.is_finished());
        running.push(task.abort_handle());
    }

    /// Tells the app the devices listed now.
    fn peers_changed(&self) {
        let mut list: Vec<Peer> = {
            let f = self.peers.lock().unwrap();
            f.all.values().filter(|p| f.answering.contains(&p.id)).cloned().collect()
        };
        list.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        (self.on_event)(Event::Peers(list));
    }

    /// A device announced (or announced again): listed once it answers. A Glass that
    /// quit without saying so (killed, crashed, its computer asleep) leaves its
    /// announcement behind for a while, and isn't listed.
    fn found(&self, peer: Peer) {
        if self.peers.lock().unwrap().all.insert(peer.id.clone(), peer.clone()).as_ref() == Some(&peer) {
            return;
        }
        let ctx = self.clone();
        self.track(tokio::spawn(async move { ctx.check(peer).await }));
    }

    /// Lists `peer` if it answers, and takes it off the list if it doesn't.
    async fn check(&self, peer: Peer) {
        let up = answers(&peer).await;
        let changed = {
            let mut f = self.peers.lock().unwrap();
            if !f.all.contains_key(&peer.id) {
                return;
            }
            if up { f.answering.insert(peer.id.clone()) } else { f.answering.remove(&peer.id) }
        };
        if changed {
            if !up {
                log::info!("local files sharing: {} ({}) doesn't answer", peer.name, peer.id);
            }
            self.peers_changed();
        }
    }

    /// Asks every device found again: one that stopped answering leaves the list, one
    /// that answers again comes back.
    async fn recheck(&self) {
        let all: Vec<Peer> = self.peers.lock().unwrap().all.values().cloned().collect();
        for p in all {
            self.check(p).await;
        }
    }

    /// A device's announcement is gone.
    fn gone(&self, id: &str) {
        let listed = {
            let mut f = self.peers.lock().unwrap();
            f.all.remove(id);
            f.answering.remove(id)
        };
        if listed {
            self.peers_changed();
        }
    }
}

/// Local Files as the app last scanned them: the same list, not a copy.
pub type Library = Arc<RwLock<Arc<Vec<LocalTrack>>>>;

/// The library right now (the lock is held only to take it).
fn library(ctx: &Ctx) -> Arc<Vec<LocalTrack>> {
    ctx.library.read().map(|l| l.clone()).unwrap_or_default()
}

/// What every device signed in to the account knows and nothing else on the
/// network does: its username and email address (Spotify shows the email only
/// to the account itself), hashed.
pub fn account_secret(username: &str, email: &str) -> String {
    to_hex(&Sha256::new().chain_update(b"glass-lan-v4\0").chain_update(username.as_bytes()).chain_update(b"\0").chain_update(email.trim().to_lowercase().as_bytes()).finalize())
}

/// Tags an account without naming it: `salt.mac`, the mac keyed with the
/// account secret. With a new salt each run, a tag doesn't single out a
/// device from one network or day to the next.
pub fn account_tag(secret: &str, salt: &str) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("any key length");
    m.update(b"glass-lan-v2");
    m.update(salt.as_bytes());
    format!("{salt}.{}", to_hex(&m.finalize().into_bytes()[..8]))
}

/// Whether `tag` was made for this account.
fn same_account(secret: &str, tag: &str) -> bool {
    tag.split_once('.').is_some_and(|(salt, _)| !salt.is_empty() && salt.len() <= 64 && account_tag(secret, salt) == tag)
}

/// The protocol of an announcement (its `v` and `acct`) from a Glass signed in to
/// this account, or `None` for another account's (or a broken one). Protocols 2
/// and 3 keyed the tag with the username alone.
fn announced(me: &Me, v: Option<&str>, tag: Option<&str>) -> Option<u32> {
    let (v, tag) = (v?.parse::<u32>().ok()?, tag?);
    let key = if v < 4 { &me.username } else { &me.account };
    (v >= 2 && !key.is_empty() && same_account(key, tag)).then_some(v)
}

/// A device's addresses in the order to try them: its network addresses (IPv4
/// first), then link-local ones, loopback last (that's only ever this device itself,
/// when it's on the same computer).
fn by_reach(mut addrs: Vec<IpAddr>) -> Vec<IpAddr> {
    let rank = |a: &IpAddr| match a {
        IpAddr::V4(v) if v.is_loopback() => 4,
        IpAddr::V6(v) if v.is_loopback() => 5,
        IpAddr::V4(v) if v.is_link_local() => 2,
        IpAddr::V4(_) => 0,
        IpAddr::V6(v) if v.is_unicast_link_local() => 3,
        IpAddr::V6(_) => 1,
    };
    addrs.sort_by_key(|a| (rank(a), *a));
    addrs.dedup();
    addrs
}

/// Whether anything takes a connection at `peer`'s addresses (tried at once, for a
/// few seconds).
async fn answers(peer: &Peer) -> bool {
    let mut tries = tokio::task::JoinSet::new();
    for a in peer.addrs.iter().copied() {
        let port = peer.port;
        tries.spawn(async move { matches!(tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(SocketAddr::new(a, port))).await, Ok(Ok(_))) });
    }
    while let Some(r) = tries.join_next().await {
        if r.unwrap_or(false) {
            return true;
        }
    }
    false
}

/// This device's name ("Davis's MacBook Pro"), for other devices to show.
pub fn computer_name() -> String {
    // iOS: the name the app gives (iOS doesn't tell apps the device's own name).
    #[cfg(glass_mobile)]
    if let Some(n) = crate::platform::share_name() {
        return n;
    }
    #[cfg(target_os = "macos")]
    if let Ok(o) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output()
        && let Ok(s) = String::from_utf8(o.stdout)
        && !s.trim().is_empty()
    {
        return s.trim().to_string();
    }
    for f in ["/etc/hostname", "/proc/sys/kernel/hostname"] {
        if let Ok(s) = std::fs::read_to_string(f)
            && !s.trim().is_empty()
        {
            return s.trim().to_string();
        }
    }
    "This computer".to_string()
}

/// A name another device gave itself, kept to something a list can show.
fn shown_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// Where received songs are saved: Music/Glass.
pub fn inbox() -> PathBuf {
    crate::save_song::default_dir()
}

// ------------------------------------------------------------------ proofs

/// That this side knows the account secret, for this one connection.
fn proof(secret: &str, handshake: &[u8], initiator: bool) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("any key length");
    m.update(b"glass-lan-v4");
    m.update(&[initiator as u8]);
    m.update(handshake);
    m.finalize().into_bytes().to_vec()
}

/// Whether `hello` comes from a device of this account, on this connection.
fn verify(hello: &Hello, handshake: &[u8], initiator: bool, me: &Me) -> bool {
    if hello.v != VERSION || !same_account(&me.account, &hello.account) {
        return false;
    }
    let Some(claimed) = hex(&hello.proof) else { return false };
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(me.account.as_bytes()).expect("any key length");
    m.update(b"glass-lan-v4");
    m.update(&[initiator as u8]);
    m.update(handshake);
    m.verify_slice(&claimed).is_ok()
}

fn hex(s: &str) -> Option<Vec<u8>> {
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ------------------------------------------------------------------ wire

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Hello {
    v: u32,
    name: String,
    account: String,
    proof: String,
}

/// A song on offer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileMeta {
    /// Which of the sender's songs this is, for `Get` (a hash of where it is there).
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub size: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
}

impl FileMeta {
    fn of(t: &LocalTrack) -> Option<Self> {
        let size = std::fs::metadata(&t.path).ok()?.len();
        let name = t.path.file_name()?.to_string_lossy().into_owned();
        Some(Self { id: file_id(&t.path), name, size, title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone(), duration_ms: t.duration_ms })
    }

    /// The song as a playlist would name it (to spot one that's already here).
    pub fn spotify_uri(&self) -> String {
        local::spotify_uri(&LocalTrack { title: self.title.clone(), artist: self.artist.clone(), album: self.album.clone(), duration_ms: self.duration_ms, ..Default::default() })
    }
}

/// A song's `FileMeta::id`: a hash of its path, so the path itself isn't sent.
fn file_id(path: &Path) -> String {
    to_hex(&Sha256::digest(path.as_os_str().as_encoded_bytes())[..10])
}

#[derive(Serialize, Deserialize, Debug)]
enum Msg {
    /// "Here are songs for you."
    Send(Vec<FileMeta>),
    /// "Do you have these `spotify:local:` songs?"
    Want(Vec<String>),
    /// The songs the sender has to offer (an answer to `Want` or `Get`).
    Offer(Vec<FileMeta>),
    /// Which offered songs to send, in order.
    Accept(Vec<bool>),
    /// How many songs were saved.
    Saved(usize),
    Refused(String),
    /// "What's in your Local Files?"
    List,
    /// The answer to `List`.
    Library(Vec<FileMeta>),
    /// "Send me these of your songs" (`FileMeta::id`s).
    Get(Vec<String>),
}

/// Message kinds: the last (or only) piece of a JSON message, a piece with
/// more to follow, file data.
const JSON: u8 = 0;
const JSON_PART: u8 = 2;
const DATA: u8 = 1;
/// The longest JSON message accepted (a long list of songs), and the longest
/// hello from a connection that hasn't proved anything yet.
const MAX_JSON: usize = 32 << 20;
const MAX_HELLO: usize = 4 << 10;
/// Incoming connections served at once (more are dropped).
const INCOMING: usize = 8;

/// An encrypted, verified connection.
struct Conn {
    stream: TcpStream,
    noise: snow::TransportState,
    buf: Vec<u8>,
}

/// One frame: its length, then its bytes, in a single write.
async fn write_frame(s: &mut TcpStream, b: &[u8]) -> Result<()> {
    let mut frame = Vec::with_capacity(b.len() + 2);
    frame.extend_from_slice(&(b.len() as u16).to_be_bytes());
    frame.extend_from_slice(b);
    s.write_all(&frame).await?;
    Ok(())
}

/// The next frame, if it starts within `wait`.
async fn read_frame(s: &mut TcpStream, wait: Duration) -> Result<Vec<u8>> {
    let mut len = [0u8; 2];
    tokio::time::timeout(wait, s.read_exact(&mut len)).await.context("timed out")??;
    let mut b = vec![0u8; u16::from_be_bytes(len) as usize];
    tokio::time::timeout(QUIET, s.read_exact(&mut b)).await.context("timed out")??;
    Ok(b)
}

impl Conn {
    /// The handshake. Returns the connection and its handshake hash.
    async fn open(mut stream: TcpStream, key: &snow::Keypair, initiator: bool) -> Result<(Self, Vec<u8>)> {
        let params: snow::params::NoiseParams = NOISE.parse()?;
        let b = snow::Builder::new(params).local_private_key(&key.private);
        let mut hs = if initiator { b.build_initiator()? } else { b.build_responder()? };
        let mut buf = vec![0u8; 65535];
        // XX: -> e; <- e, ee, s, es; -> s, se
        for step in 0..3 {
            if (step % 2 == 0) == initiator {
                let n = hs.write_message(&[], &mut buf)?;
                write_frame(&mut stream, &buf[..n]).await?;
            } else {
                let m = read_frame(&mut stream, QUIET).await?;
                hs.read_message(&m, &mut buf)?;
            }
        }
        let hash = hs.get_handshake_hash().to_vec();
        Ok((Self { stream, noise: hs.into_transport_mode()?, buf }, hash))
    }

    async fn send(&mut self, kind: u8, data: &[u8]) -> Result<()> {
        let mut plain = Vec::with_capacity(data.len() + 1);
        plain.push(kind);
        plain.extend_from_slice(data);
        let n = self.noise.write_message(&plain, &mut self.buf)?;
        write_frame(&mut self.stream, &self.buf[..n]).await
    }

    async fn recv(&mut self) -> Result<(u8, Vec<u8>)> {
        let m = read_frame(&mut self.stream, QUIET).await?;
        let n = self.noise.read_message(&m, &mut self.buf)?;
        let (kind, rest) = self.buf[..n].split_first().ok_or_else(|| anyhow!("empty message"))?;
        Ok((*kind, rest.to_vec()))
    }

    async fn send_json<T: Serialize>(&mut self, v: &T) -> Result<()> {
        let b = serde_json::to_vec(v)?;
        let mut pieces = b.chunks(CHUNK).peekable();
        while let Some(p) = pieces.next() {
            self.send(if pieces.peek().is_some() { JSON_PART } else { JSON }, p).await?;
        }
        if b.is_empty() {
            self.send(JSON, &[]).await?;
        }
        Ok(())
    }

    async fn recv_json<T: DeserializeOwned>(&mut self) -> Result<T> {
        self.recv_json_up_to(MAX_JSON).await
    }

    /// A message of at most `max` bytes.
    async fn recv_json_up_to<T: DeserializeOwned>(&mut self, max: usize) -> Result<T> {
        let mut all = Vec::new();
        loop {
            let (kind, b) = self.recv().await?;
            if all.len() + b.len() > max {
                bail!("message too long");
            }
            all.extend(b);
            match kind {
                JSON_PART => {}
                JSON => return Ok(serde_json::from_slice(&all)?),
                _ => bail!("unexpected data"),
            }
        }
    }

    async fn recv_msg(&mut self) -> Result<Msg> {
        match self.recv_json::<Msg>().await? {
            Msg::Refused(why) => bail!("{why}"),
            m => Ok(m),
        }
    }
}

/// Handshake, then both sides prove they're signed in to the same account.
/// Returns the verified connection and the other device's name.
async fn connect(stream: TcpStream, ctx: &Ctx, initiator: bool) -> Result<(Conn, String)> {
    let (mut c, hash) = Conn::open(stream, &ctx.key, initiator).await?;
    let mine = Hello { v: VERSION, name: ctx.me.name.clone(), account: ctx.tag.clone(), proof: to_hex(&proof(&ctx.me.account, &hash, initiator)) };
    let name = if initiator {
        c.send_json(&mine).await?;
        let theirs: Hello = match c.recv_json_up_to::<Hello>(MAX_HELLO).await {
            Ok(h) => h,
            Err(_) => bail!("it didn't answer (it may need updating, or be signed in to another account)"),
        };
        if !verify(&theirs, &hash, !initiator, &ctx.me) {
            bail!("it isn't signed in to your Spotify account");
        }
        theirs.name
    } else {
        let theirs: Hello = c.recv_json_up_to(MAX_HELLO).await?;
        if !verify(&theirs, &hash, !initiator, &ctx.me) {
            bail!("a device that isn't signed in to this account");
        }
        c.send_json(&mine).await?;
        theirs.name
    };
    Ok((c, shown_name(&name)))
}

// ------------------------------------------------------------------ files

/// Where an offered song would be saved, or `None` to skip it (not audio,
/// a bad name, too big, or already here).
fn destination(ctx: &Ctx, f: &FileMeta) -> Option<PathBuf> {
    let name = Path::new(&f.name).file_name()?.to_str()?;
    if name.starts_with('.') || name != f.name || !local::is_audio(Path::new(name)) || f.size == 0 || f.size > MAX_FILE {
        return None;
    }
    if local::find(&library(ctx), &f.spotify_uri()).is_some() {
        return None;
    }
    let path = ctx.inbox.join(name);
    match std::fs::metadata(&path) {
        Ok(m) if m.len() == f.size => None,
        Ok(_) => {
            let p = Path::new(name);
            let (stem, ext) = (p.file_stem()?.to_string_lossy(), p.extension()?.to_string_lossy());
            (2..100).map(|i| ctx.inbox.join(format!("{stem} ({i}).{ext}"))).find(|p| !p.exists())
        }
        Err(_) => Some(path),
    }
}

fn report(ctx: &Ctx, t: &Transfer) {
    (ctx.on_event)(Event::Transfer(t.clone()));
}

/// Sends `files` (the accepted ones) over `c`.
async fn stream_files(c: &mut Conn, ctx: &Ctx, t: &mut Transfer, files: &[(PathBuf, u64)]) -> Result<()> {
    let mut last = Instant::now();
    let mut buf = vec![0u8; CHUNK];
    for (path, size) in files {
        let mut f = tokio::fs::File::open(path).await?;
        let mut left = *size;
        while left > 0 {
            let n = f.read(&mut buf[..CHUNK.min(left as usize)]).await?;
            if n == 0 {
                bail!("{} got shorter while sending", path.display());
            }
            c.send(DATA, &buf[..n]).await?;
            left -= n as u64;
            t.bytes += n as u64;
            if last.elapsed() > Duration::from_millis(250) {
                last = Instant::now();
                report(ctx, t);
            }
        }
        t.done += 1;
        report(ctx, t);
    }
    Ok(())
}

/// Receives the accepted songs into their destinations. Returns the ones saved,
/// also when the transfer broke off part way.
async fn receive_files(c: &mut Conn, ctx: &Ctx, t: &mut Transfer, files: &[(PathBuf, u64)]) -> (Vec<PathBuf>, Result<()>) {
    let mut saved = Vec::new();
    if let Err(e) = std::fs::create_dir_all(&ctx.inbox) {
        return (saved, Err(e.into()));
    }
    let mut last = Instant::now();
    for (path, size) in files {
        // Hidden while it arrives; one left by quitting mid-way is swept later
        // (`save_song::sweep_work_files`).
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let part = path.with_file_name(format!(".{name}.glass-part"));
        let mut out = match tokio::fs::File::create(&part).await {
            Ok(f) => f,
            Err(e) => return (saved, Err(e.into())),
        };
        let mut got = 0u64;
        let r: Result<()> = async {
            while got < *size {
                let (kind, data) = c.recv().await?;
                if kind != DATA || got + data.len() as u64 > *size {
                    bail!("the transfer went wrong");
                }
                out.write_all(&data).await?;
                got += data.len() as u64;
                t.bytes += data.len() as u64;
                if last.elapsed() > Duration::from_millis(250) {
                    last = Instant::now();
                    report(ctx, t);
                }
            }
            out.flush().await?;
            Ok(())
        }
        .await;
        drop(out);
        if let Err(e) = r.and_then(|_| std::fs::rename(&part, path).map_err(Into::into)) {
            let _ = std::fs::remove_file(&part);
            return (saved, Err(e));
        }
        saved.push(path.clone());
        t.done += 1;
        report(ctx, t);
    }
    (saved, Ok(()))
}

/// The receiving side: picks what to take from `offer`, then saves it. Returns
/// the songs saved (even if it then failed) and how it went.
async fn take(c: &mut Conn, ctx: &Ctx, t: &mut Transfer, offer: &[FileMeta]) -> (Vec<PathBuf>, Result<()>) {
    let mut seen = std::collections::HashSet::new();
    let picks: Vec<Option<PathBuf>> = offer.iter().map(|f| destination(ctx, f).filter(|p| seen.insert(p.clone()))).collect();
    if let Err(e) = c.send_json(&Msg::Accept(picks.iter().map(Option::is_some).collect())).await {
        return (Vec::new(), Err(e));
    }
    let files: Vec<(PathBuf, u64)> = picks.into_iter().zip(offer).filter_map(|(p, f)| Some((p?, f.size))).collect();
    t.files = files.len();
    t.total = files.iter().map(|f| f.1).sum();
    report(ctx, t);
    let (saved, r) = receive_files(c, ctx, t, &files).await;
    let r = match r {
        Ok(()) => c.send_json(&Msg::Saved(saved.len())).await,
        Err(e) => Err(e),
    };
    (saved, r)
}

/// The sending side: offers `tracks`, then sends what the other side takes.
async fn give(c: &mut Conn, ctx: &Ctx, t: &mut Transfer, tracks: &[(LocalTrack, FileMeta)]) -> Result<usize> {
    let Msg::Accept(yes) = c.recv_msg().await? else { bail!("unexpected answer") };
    let files: Vec<(PathBuf, u64)> = tracks.iter().zip(yes).filter(|(_, y)| *y).map(|((tr, m), _)| (tr.path.clone(), m.size)).collect();
    t.files = files.len();
    t.total = files.iter().map(|f| f.1).sum();
    report(ctx, t);
    stream_files(c, ctx, t, &files).await?;
    match c.recv_msg().await? {
        Msg::Saved(n) => Ok(n),
        _ => bail!("unexpected answer"),
    }
}

fn new_transfer(ctx: &Ctx, peer: &str, sending: bool) -> Transfer {
    Transfer { id: ctx.next_id.fetch_add(1, Ordering::Relaxed), peer: peer.to_string(), sending, files: 0, done: 0, bytes: 0, total: 0, state: State::Running }
}

/// One incoming connection.
async fn serve(stream: TcpStream, ctx: Ctx) -> Result<()> {
    let (mut c, peer) = connect(stream, &ctx, false).await?;
    match c.recv_msg().await? {
        Msg::Send(offer) if offer.len() > MAX_FILES => too_many(&mut c).await,
        Msg::Want(uris) if uris.len() > MAX_FILES => too_many(&mut c).await,
        Msg::Get(ids) if ids.len() > MAX_FILES => too_many(&mut c).await,
        Msg::Send(offer) => {
            let mut t = new_transfer(&ctx, &peer, false);
            let (saved, r) = take(&mut c, &ctx, &mut t, &offer).await;
            finish(&ctx, t, &peer, saved, r)
        }
        Msg::List => {
            // Sizes come from the disk: off the async threads.
            let lib = library(&ctx);
            let songs = tokio::task::spawn_blocking(move || lib.iter().take(MAX_FILES).filter_map(FileMeta::of).collect::<Vec<_>>()).await?;
            c.send_json(&Msg::Library(songs)).await
        }
        Msg::Want(uris) => {
            // Answer from this device's Local Files.
            let lib = library(&ctx);
            let have: Vec<LocalTrack> = uris.iter().filter_map(|u| local::find(&lib, u).map(|i| lib[i].clone())).collect();
            offer(&mut c, &ctx, &peer, have).await
        }
        Msg::Get(ids) => {
            let lib = library(&ctx);
            let ids: HashSet<String> = ids.into_iter().collect();
            let have: Vec<LocalTrack> = lib.iter().filter(|t| ids.contains(&file_id(&t.path))).cloned().collect();
            offer(&mut c, &ctx, &peer, have).await
        }
        _ => {
            c.send_json(&Msg::Refused("not understood".into())).await?;
            Ok(())
        }
    }
}

/// Offers `tracks` (the ones asked for) and sends what the other side takes.
async fn offer(c: &mut Conn, ctx: &Ctx, peer: &str, tracks: Vec<LocalTrack>) -> Result<()> {
    let have: Vec<(LocalTrack, FileMeta)> = tracks.into_iter().filter_map(|t| FileMeta::of(&t).map(|m| (t, m))).collect();
    c.send_json(&Msg::Offer(have.iter().map(|h| h.1.clone()).collect())).await?;
    if have.is_empty() {
        return Ok(());
    }
    let mut t = new_transfer(ctx, peer, true);
    let r = give(c, ctx, &mut t, &have).await;
    finish(ctx, t, peer, Vec::new(), r.map(|_| ()))
}

async fn too_many(c: &mut Conn) -> Result<()> {
    c.send_json(&Msg::Refused(format!("that's more than {MAX_FILES} songs at once"))).await
}

/// Reports how a transfer ended. Songs that arrived are passed on either way.
fn finish(ctx: &Ctx, mut t: Transfer, peer: &str, saved: Vec<PathBuf>, r: Result<()>) -> Result<()> {
    if !saved.is_empty() {
        (ctx.on_event)(Event::Received { from: peer.to_string(), paths: saved });
    }
    t.state = match &r {
        Ok(()) => State::Done,
        Err(e) => State::Failed(e.to_string()),
    };
    report(ctx, &t);
    r
}

async fn dial(peer: &Peer) -> Result<TcpStream> {
    let mut last = anyhow!("{} isn't reachable", peer.name);
    for a in by_reach(peer.addrs.clone()) {
        match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(SocketAddr::new(a, peer.port))).await {
            Ok(Ok(s)) => {
                // Small messages go out at once (no waiting to batch them).
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Ok(Err(e)) => last = e.into(),
            Err(_) => last = anyhow!("{} didn't answer", peer.name),
        }
    }
    Err(last)
}

/// Sends songs to `peer`.
async fn send_to(ctx: Ctx, peer: Peer, tracks: Vec<LocalTrack>) -> Result<()> {
    let items: Vec<(LocalTrack, FileMeta)> = tracks.into_iter().filter_map(|t| FileMeta::of(&t).map(|m| (t, m))).collect();
    let mut t = new_transfer(&ctx, &peer.name, true);
    report(&ctx, &t);
    let r: Result<usize> = async {
        if items.len() > MAX_FILES {
            bail!("that's more than {MAX_FILES} songs at once: send them in parts");
        }
        let (mut c, _) = connect(dial(&peer).await?, &ctx, true).await?;
        c.send_json(&Msg::Send(items.iter().map(|i| i.1.clone()).collect())).await?;
        give(&mut c, &ctx, &mut t, &items).await
    }
    .await;
    finish(&ctx, t, &peer.name, Vec::new(), r.map(|_| ()))
}

/// Asks `peer` for the `spotify:local:` songs in `uris`; returns those saved.
async fn want_from(ctx: &Ctx, peer: &Peer, uris: Vec<String>) -> Result<Vec<PathBuf>> {
    request(ctx, peer, Msg::Want(uris)).await
}

/// Gets the songs with these `FileMeta::id`s from `peer`; returns those saved.
async fn get_from(ctx: &Ctx, peer: &Peer, ids: Vec<String>) -> Result<Vec<PathBuf>> {
    request(ctx, peer, Msg::Get(ids)).await
}

/// Asks `peer` for songs (`Want` or `Get`) and takes what it offers.
async fn request(ctx: &Ctx, peer: &Peer, ask: Msg) -> Result<Vec<PathBuf>> {
    let (mut c, _) = connect(dial(peer).await?, ctx, true).await?;
    c.send_json(&ask).await?;
    let Msg::Offer(offer) = c.recv_msg().await? else { bail!("unexpected answer") };
    if offer.is_empty() || offer.len() > MAX_FILES {
        return Ok(Vec::new());
    }
    let mut t = new_transfer(ctx, &peer.name, false);
    let (saved, r) = take(&mut c, ctx, &mut t, &offer).await;
    finish(ctx, t, &peer.name, saved.clone(), r)?;
    Ok(saved)
}

/// `peer`'s Local Files.
async fn list_from(ctx: &Ctx, peer: &Peer) -> Result<Vec<FileMeta>> {
    let (mut c, _) = connect(dial(peer).await?, ctx, true).await?;
    c.send_json(&Msg::List).await?;
    match c.recv_msg().await? {
        Msg::Library(songs) => Ok(songs),
        _ => bail!("unexpected answer"),
    }
}

// ------------------------------------------------------------------ service

/// Sharing, while it's on: announces this Glass, finds the others, accepts
/// their transfers. Stops when dropped.
pub struct Share {
    ctx: Ctx,
    #[cfg(not(glass_mobile))]
    mdns: Option<mdns_sd::ServiceDaemon>,
    /// iOS: Bonjour through the system's DNS-SD (see ios/core/src/bonjour.rs).
    #[cfg(glass_mobile)]
    mdns: Option<crate::bonjour::Bonjour>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    rt: tokio::runtime::Handle,
}

impl Share {
    pub fn start(rt: tokio::runtime::Handle, me: Me, library: Library, inbox: PathBuf, on_event: impl Fn(Event) + Send + Sync + 'static) -> Result<Self> {
        // A new key each run: nothing remembers it (the account proves who's who). The
        // kept key and paired list of protocol 3 aren't needed any more.
        let key = Arc::new(snow::Builder::new(NOISE.parse()?).generate_keypair()?);
        for old in ["share_key", "share_paired.json"] {
            let _ = std::fs::remove_file(crate::config::config_dir().join(old));
        }
        let tag = account_tag(&me.account, &me.instance);
        let ctx = Ctx { me, tag, key, library, inbox, on_event: Arc::new(on_event), next_id: Arc::default(), running: Arc::default(), peers: Arc::default() };
        let std_listener = std::net::TcpListener::bind(("0.0.0.0", PORT)).or_else(|_| std::net::TcpListener::bind("0.0.0.0:0"))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let listener = {
            let _in_runtime = rt.enter();
            TcpListener::from_std(std_listener)?
        };
        let mut tasks = Vec::new();
        let c = ctx.clone();
        let slots = Arc::new(tokio::sync::Semaphore::new(INCOMING));
        tasks.push(rt.spawn(async move {
            loop {
                let stream = match listener.accept().await {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        // Out of file handles, say: wait rather than spin.
                        log::warn!("local files sharing: {e}");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                };
                // More than a few at once are dropped: real transfers come one or two at a time.
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
                let _ = stream.set_nodelay(true);
                let task = c.clone();
                c.track(tokio::spawn(async move {
                    let _slot = slot;
                    if let Err(e) = serve(stream, task).await {
                        log::info!("local files sharing: {e:#}");
                    }
                }));
            }
        }));
        let mdns = Self::announce(&rt, &ctx, port, &mut tasks).map_err(|e| log::warn!("local files sharing can't use the network: {e:#}")).ok();
        let c = ctx.clone();
        tasks.push(rt.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                c.recheck().await;
            }
        }));
        Ok(Self { ctx, mdns, tasks, rt })
    }

    /// iOS: the same announcement and the same peers, through DNS-SD.
    #[cfg(glass_mobile)]
    fn announce(rt: &tokio::runtime::Handle, ctx: &Ctx, port: u16, tasks: &mut Vec<tokio::task::JoinHandle<()>>) -> Result<crate::bonjour::Bonjour> {
        let me = &ctx.me;
        let props = [("v", VERSION.to_string()), ("acct", ctx.tag.clone()), ("name", me.name.clone())];
        let (bonjour, mut events) = crate::bonjour::Bonjour::start(SERVICE.trim_end_matches(".local."), &me.instance, port, &props)?;
        let ctx = ctx.clone();
        tasks.push(rt.spawn(async move {
            while let Some(ev) = events.recv().await {
                match ev {
                    crate::bonjour::Found::Service { instance, txt, port, addrs } => {
                        // This account's (in any version: one that can't share is listed to say so).
                        if let Some(version) = announced(&ctx.me, txt.get("v").map(String::as_str), txt.get("acct").map(String::as_str))
                            && instance != ctx.me.instance
                            && !instance.is_empty()
                        {
                            let name = shown_name(txt.get("name").map(String::as_str).unwrap_or("Glass"));
                            ctx.found(Peer { id: instance, name, addrs, port, version });
                        }
                    }
                    crate::bonjour::Found::Gone { instance } => ctx.gone(&instance),
                    crate::bonjour::Found::Denied => (ctx.on_event)(Event::NoLocalNetwork),
                }
            }
        }));
        Ok(bonjour)
    }

    #[cfg(not(glass_mobile))]
    fn announce(rt: &tokio::runtime::Handle, ctx: &Ctx, port: u16, tasks: &mut Vec<tokio::task::JoinHandle<()>>) -> Result<mdns_sd::ServiceDaemon> {
        let mdns = mdns_sd::ServiceDaemon::new()?;
        // Networks don't come and go every few seconds (the default check).
        let _ = mdns.set_ip_check_interval(30);
        let me = &ctx.me;
        let props = [("v", VERSION.to_string()), ("acct", ctx.tag.clone()), ("name", me.name.clone())];
        let info = mdns_sd::ServiceInfo::new(SERVICE, &me.instance, &format!("glass-{}.local.", me.instance), (), port, &props[..])?.enable_addr_auto();
        mdns.register(info)?;
        let events = mdns.browse(SERVICE)?;
        let ctx = ctx.clone();
        tasks.push(rt.spawn(async move {
            while let Ok(ev) = events.recv_async().await {
                match ev {
                    mdns_sd::ServiceEvent::ServiceResolved(info) => {
                        // This account's (in any version: one that can't share is listed to say so).
                        let id = info.get_fullname().split('.').next().unwrap_or("").to_string();
                        if let Some(version) = announced(&ctx.me, info.get_property_val_str("v"), info.get_property_val_str("acct"))
                            && id != ctx.me.instance
                            && !id.is_empty()
                        {
                            let name = shown_name(info.get_property_val_str("name").unwrap_or("Glass"));
                            let mut addrs: Vec<IpAddr> = info.get_addresses().iter().cloned().collect();
                            addrs.sort();
                            ctx.found(Peer { id, name, addrs, port: info.get_port(), version });
                        }
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => ctx.gone(fullname.split('.').next().unwrap_or("")),
                    _ => {}
                }
            }
        }));
        Ok(mdns)
    }

    /// The devices songs can go to and from now (the ones that speak this protocol).
    pub fn peers(&self) -> Vec<Peer> {
        let mut list: Vec<Peer> = {
            let f = self.ctx.peers.lock().unwrap();
            f.all.values().filter(|p| p.ready() && f.answering.contains(&p.id)).cloned().collect()
        };
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Sends songs to another device (in the background).
    pub fn send(&self, peer: Peer, tracks: Vec<LocalTrack>) {
        let ctx = self.ctx.clone();
        self.ctx.track(self.rt.spawn(async move {
            if let Err(e) = send_to(ctx, peer, tracks).await {
                log::info!("sending local files: {e:#}");
            }
        }));
    }

    /// Gets the `spotify:local:` songs in `uris` from whichever devices have
    /// them (in the background).
    pub fn fetch(&self, uris: Vec<String>) {
        let (ctx, peers) = (self.ctx.clone(), self.peers());
        self.ctx.track(self.rt.spawn(async move {
            let mut left = uris;
            // The most one request may ask for.
            left.truncate(MAX_FILES);
            for p in peers {
                if left.is_empty() {
                    break;
                }
                match want_from(&ctx, &p, left.clone()).await {
                    Ok(saved) if !saved.is_empty() => {
                        let lib: Vec<LocalTrack> = local::read_paths(&saved);
                        left.retain(|u| local::find(&lib, u).is_none());
                    }
                    Ok(_) => {}
                    Err(e) => log::info!("getting local files from {}: {e:#}", p.name),
                }
            }
        }));
    }
    /// Asks `peer` for its Local Files (the answer comes as `Event::Library`).
    pub fn browse(&self, peer: Peer) {
        let ctx = self.ctx.clone();
        self.ctx.track(self.rt.spawn(async move {
            let songs = list_from(&ctx, &peer).await.map_err(|e| {
                log::info!("listing {}'s local files: {e:#}", peer.name);
                format!("{e:#}")
            });
            (ctx.on_event)(Event::Library { peer: peer.id, songs });
        }));
    }

    /// Gets the songs with these `FileMeta::id`s from `peer` (in the background).
    pub fn get(&self, peer: Peer, ids: Vec<String>) {
        let ctx = self.ctx.clone();
        self.ctx.track(self.rt.spawn(async move {
            let mut ids = ids;
            ids.truncate(MAX_FILES);
            if let Err(e) = get_from(&ctx, &peer, ids).await {
                log::info!("getting local files from {}: {e:#}", peer.name);
            }
        }));
    }
}

impl Drop for Share {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        for t in self.ctx.running.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            t.abort();
        }
        #[cfg(not(glass_mobile))]
        if let Some(m) = self.mdns.take() {
            let _ = m.unregister(&format!("{}.{SERVICE}", self.ctx.me.instance));
            let _ = m.shutdown();
        }
        #[cfg(glass_mobile)]
        drop(self.mdns.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(device: &str, account: &str) -> Me {
        Me { instance: format!("i-{device}"), name: format!("Mac {device}"), account: account_secret(account, &format!("{account}@example.com")), username: account.into() }
    }

    /// A device signed in to `account`.
    fn ctx(device: &str, account: &str, library: Vec<LocalTrack>, inbox: PathBuf) -> (Ctx, Arc<Mutex<Vec<Event>>>) {
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let ev = events.clone();
        let params: snow::params::NoiseParams = NOISE.parse().unwrap();
        let me = me(device, account);
        let c = Ctx {
            tag: account_tag(&me.account, &me.instance),
            me,
            key: Arc::new(snow::Builder::new(params).generate_keypair().unwrap()),
            library: Arc::new(RwLock::new(Arc::new(library))),
            inbox,
            on_event: Arc::new(move |e| ev.lock().unwrap().push(e)),
            next_id: Arc::default(),
            running: Arc::default(),
            peers: Arc::default(),
        };
        (c, events)
    }

    fn song(dir: &Path, file: &str, title: &str, bytes: usize) -> LocalTrack {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(file);
        std::fs::write(&path, (0..bytes).map(|i| (i % 251) as u8).collect::<Vec<u8>>()).unwrap();
        LocalTrack {
            path,
            title: title.into(),
            artist: "Anri".into(),
            album: "Timely!!".into(),
            duration_ms: 250_000,
            added: std::time::SystemTime::UNIX_EPOCH,
            year: None,
            genre: String::new(),
            has_cover: false,
            ..Default::default()
        }
    }

    /// B listens; returns its address.
    async fn listen(b: Ctx) -> Peer {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((s, _)) = l.accept().await {
                let b = b.clone();
                tokio::spawn(async move {
                    let _ = serve(s, b).await;
                });
            }
        });
        Peer { id: "b".into(), name: "Mac B".into(), addrs: vec!["127.0.0.1".parse().unwrap()], port, version: VERSION }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = crate::config::test_dir(&format!("share-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn proofs_only_fit_their_connection_and_account() {
        let a = me("a", "davis");
        let hash = [7u8; 32];
        let hello = Hello { v: VERSION, name: "A".into(), account: account_tag(&a.account, &a.instance), proof: to_hex(&proof(&a.account, &hash, true)) };
        let b = me("b", "davis");
        assert!(verify(&hello, &hash, true, &b));
        assert!(!verify(&hello, &[8u8; 32], true, &b), "relayed into another connection");
        assert!(!verify(&hello, &hash, false, &b), "reflected back with the other role");
        assert!(!verify(&hello, &hash, true, &me("x", "someone-else")), "another account");
        // Knowing the username alone isn't enough: the email is part of the secret.
        let guess = Me { account: account_secret("davis", "guess@example.com"), ..b.clone() };
        let forged = Hello { account: account_tag(&guess.account, "salt"), proof: to_hex(&proof(&guess.account, &hash, true)), ..hello.clone() };
        assert!(!verify(&forged, &hash, true, &b));
        assert!(!verify(&Hello { v: 3, ..hello }, &hash, true, &b), "an older Glass");
    }

    #[test]
    fn the_secret_ignores_how_the_email_is_written() {
        assert_eq!(account_secret("davis", " Davis@Example.com "), account_secret("davis", "davis@example.com"));
        assert_ne!(account_secret("davis", "davis@example.com"), account_secret("davis2", "davis@example.com"));
    }

    #[tokio::test]
    async fn songs_go_across_and_only_to_the_same_account() {
        let (src, dst) = (tmp("src"), tmp("dst"));
        let a_songs = vec![song(&src, "Remember Summer Days.mp3", "Remember Summer Days", 150_000), song(&src, "Last Summer Whisper.flac", "Last Summer Whisper", 70_000)];
        // B already has the second song (another copy), so only the first comes over.
        let b_lib = vec![LocalTrack { path: dst.join("elsewhere.flac"), ..a_songs[1].clone() }];
        let (a, _) = ctx("a", "davis", a_songs.clone(), tmp("a-inbox"));
        let (b, b_events) = ctx("b", "davis", b_lib, dst.clone());
        let b_library = b.library.clone();
        let peer = listen(b).await;
        send_to(a.clone(), peer.clone(), a_songs.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let got = std::fs::read(dst.join("Remember Summer Days.mp3")).unwrap();
        assert_eq!(got, std::fs::read(&a_songs[0].path).unwrap(), "the song arrives intact");
        assert!(!dst.join("Last Summer Whisper.flac").exists(), "a song B already has isn't sent");
        assert!(b_events.lock().unwrap().iter().any(|e| matches!(e, Event::Received { from, paths } if from == "Mac a" && paths.len() == 1)));

        // A device signed in to another account is turned away, both ways.
        let (stranger, _) = ctx("x", "someone-else", a_songs.clone(), tmp("x-inbox"));
        let e = send_to(stranger.clone(), peer.clone(), a_songs.clone()).await.unwrap_err();
        assert!(e.to_string().contains("didn't answer"), "{e:#}");
        assert!(list_from(&stranger, &peer).await.is_err());

        // Pull: another device asks B for a playlist's local song, which B now
        // has (once Local Files has looked again).
        *b_library.write().unwrap() = Arc::new(
            local::read_paths(std::slice::from_ref(&dst))
                .into_iter()
                .map(|t| LocalTrack { title: a_songs[0].title.clone(), artist: "Anri".into(), album: "Timely!!".into(), duration_ms: 250_000, ..t })
                .collect(),
        );
        let (want, _) = ctx("a", "davis", vec![], tmp("pull-inbox"));
        let uri = local::spotify_uri(&a_songs[0]);
        let saved = want_from(&want, &peer, vec![uri, "spotify:local:Nobody::Nothing:100".into()]).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(std::fs::read(&saved[0]).unwrap().len(), 150_000);
        for d in [src, dst, tmp("a-inbox"), tmp("x-inbox"), tmp("pull-inbox")] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[tokio::test]
    async fn a_device_lists_its_songs_and_sends_the_ones_picked() {
        let src = tmp("list-src");
        let songs = vec![song(&src, "One.mp3", "One", 5000), song(&src, "Two.flac", "Two", 7000), song(&src, "Three.m4a", "Three", 9000)];
        let (b, b_events) = ctx("b", "davis", songs.clone(), tmp("list-b"));
        let peer = listen(b).await;
        let inbox = tmp("list-inbox");
        let (a, a_events) = ctx("a", "davis", vec![], inbox.clone());
        let listed = list_from(&a, &peer).await.unwrap();
        assert_eq!(listed.iter().map(|f| f.title.as_str()).collect::<Vec<_>>(), ["One", "Two", "Three"]);
        assert!(listed.iter().all(|f| f.id.len() == 20 && !f.id.contains('/')), "ids, not paths");
        assert_eq!(listed[1].size, 7000);

        let picked = vec![listed[0].id.clone(), listed[2].id.clone(), "not-a-song".into()];
        let saved = get_from(&a, &peer, picked).await.unwrap();
        assert_eq!(saved, [inbox.join("One.mp3"), inbox.join("Three.m4a")]);
        assert_eq!(std::fs::read(inbox.join("Three.m4a")).unwrap(), std::fs::read(&songs[2].path).unwrap());
        assert!(!inbox.join("Two.flac").exists());
        assert!(a_events.lock().unwrap().iter().any(|e| matches!(e, Event::Received { from, paths } if from == "Mac B" && paths.len() == 2)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(b_events.lock().unwrap().iter().any(|e| matches!(e, Event::Transfer(t) if t.sending && t.state == State::Done && t.done == 2)));
        for d in [src, inbox, tmp("list-b")] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// Plays a second Glass against the one running on this computer: found over
    /// the network, signed in to the same account (from the running Glass's saved
    /// login), and asked for one of your local songs (read only; the copy goes to a
    /// temporary folder). Needs Glass running and signed in, with sharing on:
    /// `cargo test --release live_get_a_song -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_get_a_song_from_running_glass() {
        use crate::config::{Settings, config_dir};
        use librespot_core::cache::Cache;
        let real = Settings::load();
        let creds = Cache::new(Some(config_dir()), None, None, None).unwrap().credentials().expect("sign in to Glass first");
        let lib = local::scan(&real.local_dirs);
        let song = lib.iter().find(|t| t.duration_ms > 0 && !t.artist.is_empty() && std::fs::metadata(&t.path).is_ok_and(|m| m.len() < 20 << 20)).expect("a tagged local song").clone();
        println!("asking for {}", song.title);

        // A second Glass, with its own config (sharing the running Glass's would
        // take over its connection to Spotify).
        let home = tmp("live-home");
        std::fs::create_dir_all(&home).unwrap();
        let real_home = std::env::var_os("HOME");
        // SAFETY: an ignored test run on its own.
        unsafe { std::env::set_var("HOME", &home) };
        let sp = crate::sp::Sp::connect_cached(creds).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let me_json = sp.handle(&http::Method::GET, "/me", None, None).await.unwrap();
        let username = me_json["id"].as_str().unwrap().to_string();
        let account = account_secret(&username, me_json["email"].as_str().unwrap_or(""));
        let (tx, rx) = std::sync::mpsc::channel();
        let inbox = tmp("live-inbox");
        let who = Me { instance: "livetest0000".into(), name: "Sharing test".into(), account, username };
        let share = Share::start(tokio::runtime::Handle::current(), who, Arc::default(), inbox.clone(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let t0 = Instant::now();
        while share.peers().is_empty() && t0.elapsed() < Duration::from_secs(20) {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let peers = share.peers();
        println!("found on the network: {:?}", peers.iter().map(|p| &p.name).collect::<Vec<_>>());
        assert!(!peers.is_empty(), "the running Glass announces itself");
        let listed = list_from(&share.ctx, &peers[0]).await.unwrap_or_else(|e| panic!("listing {}: {e:#}", peers[0].name));
        println!("{} lists {} songs", peers[0].name, listed.len());
        let want = listed.iter().find(|f| f.title == song.title && f.size == std::fs::metadata(&song.path).unwrap().len()).expect("the song is listed");
        let paths = get_from(&share.ctx, &peers[0], vec![want.id.clone()]).await.unwrap_or_else(|e| panic!("asking {}: {e:#}", peers[0].name));
        assert_eq!(paths.len(), 1, "the song arrived");
        let from = match rx.try_iter().find(|e| matches!(e, Event::Received { .. })) {
            Some(Event::Received { from, .. }) => from,
            _ => String::new(),
        };
        assert_eq!(std::fs::metadata(&paths[0]).unwrap().len(), std::fs::metadata(&song.path).unwrap().len());
        println!("ok  got {} from {from} in {:.1} s", paths[0].display(), t0.elapsed().as_secs_f32());
        drop(share);
        if let Some(h) = real_home {
            unsafe { std::env::set_var("HOME", h) };
        }
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(inbox);
    }

    #[test]
    fn glass_of_this_account_in_another_version_is_recognised_to_say_so() {
        let me = me("a", "davis");
        let ours = |v: &str, key: &str| announced(&me, Some(v), Some(&account_tag(key, "run1")));
        assert_eq!(ours("4", &me.account), Some(4));
        // Released 1.0.0-beta.22 (protocol 3) keys its tag with the username alone.
        assert_eq!(ours("3", "davis"), Some(3));
        assert_eq!(ours("3", &me.account), None);
        assert_eq!(ours("5", &me.account), Some(5), "a newer Glass, if it keeps the tag");
        assert_eq!(ours("3", "someone-else"), None);
        assert_eq!(ours("4", &account_secret("someone-else", "x@example.com")), None);
        assert_eq!(announced(&me, Some("four"), Some("run1.00")), None);
        assert_eq!(announced(&me, None, Some(&account_tag(&me.account, "run1"))), None);

        let peer = |version| Peer { id: "p".into(), name: "Davis's MacBook".into(), addrs: vec![], port: 1, version };
        assert!(peer(VERSION).ready() && peer(VERSION).update_note().is_none());
        assert!(!peer(3).ready());
        assert_eq!(peer(3).update_note().unwrap(), "Davis's MacBook has an older Glass. Update Glass there to share with it.");
        assert!(peer(VERSION + 1).update_note().unwrap().contains("Update Glass here"));
    }

    #[tokio::test]
    async fn only_devices_that_answer_are_listed() {
        let (c, events) = ctx("a", "davis", vec![], tmp("answer"));
        let live = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live_port = live.local_addr().unwrap().port();
        let dead_port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let peer = |id: &str, port| Peer { id: id.into(), name: "Davis's MacBook Pro".into(), addrs: vec!["127.0.0.1".parse().unwrap()], port, version: VERSION };
        let listed = || events.lock().unwrap().iter().rev().find_map(|e| if let Event::Peers(p) = e { Some(p.iter().map(|p| p.id.clone()).collect::<Vec<_>>()) } else { None });
        // Glass on the Mac restarted: its old announcement lingers, a new one comes.
        c.found(peer("old", dead_port));
        c.found(peer("new", live_port));
        let t0 = Instant::now();
        while listed().is_none() && t0.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        c.recheck().await;
        assert_eq!(listed(), Some(vec!["new".to_string()]), "the one that doesn't answer isn't listed");
        // It quits without a word: off the list at the next check.
        drop(live);
        c.recheck().await;
        assert_eq!(listed(), Some(vec![]));
        // Back (on the same port): listed again.
        let _again = TcpListener::bind(("127.0.0.1", live_port)).await.unwrap();
        c.recheck().await;
        assert_eq!(listed(), Some(vec!["new".to_string()]));
        c.gone("new");
        assert_eq!(listed(), Some(vec![]));
        let _ = std::fs::remove_dir_all(tmp("answer"));
    }

    #[test]
    fn addresses_are_tried_network_first_loopback_last() {
        let a: Vec<IpAddr> = ["::1", "fe80::1", "127.0.0.1", "169.254.24.119", "fd00::5", "192.168.1.20", "10.0.0.7"].iter().map(|s| s.parse().unwrap()).collect();
        let order: Vec<String> = by_reach(a).iter().map(|a| a.to_string()).collect();
        assert_eq!(order, ["10.0.0.7", "192.168.1.20", "fd00::5", "169.254.24.119", "fe80::1", "127.0.0.1", "::1"]);
    }

    #[test]
    fn a_browsed_device_marks_the_songs_already_here() {
        let peer = Peer { id: "p".into(), name: "Davis's iPhone".into(), addrs: vec![], port: 1, version: VERSION };
        let mut b = Browsed::new(&peer);
        let f = |id: &str, title: &str| FileMeta { id: id.into(), name: format!("{title}.mp3"), size: 10, title: title.into(), artist: "Anri".into(), album: "Timely!!".into(), duration_ms: 250_000 };
        let mine = vec![LocalTrack { path: "/m/Windy Summer.flac".into(), title: "Windy Summer".into(), artist: "Anri".into(), album: "Timely!!".into(), duration_ms: 250_000, ..Default::default() }];
        b.answered("someone else", Ok(vec![f("1", "Windy Summer")]), &mine);
        assert!(b.songs.is_none(), "another device's answer");
        b.answered("p", Ok(vec![f("1", "Windy Summer"), f("2", "Remember Summer Days")]), &mine);
        assert_eq!(b.list().map(|(f, here)| (f.id.as_str(), here)).collect::<Vec<_>>(), [("1", true), ("2", false)]);
        b.answered("p", Err("timed out".into()), &mine);
        let e = b.songs.as_ref().unwrap().as_ref().unwrap_err();
        assert!(e.contains("Davis's iPhone") && !e.contains("timed out"), "{e}");
    }

    /// The computer's side of ios/scripts/live-share.sh: signed in as the demo account
    /// (as `GLASS_DEMO_SHARE` runs), it finds the iPhone app in the simulator (open with
    /// `-demo -demoShare`), lists its songs, gets "Phone Song" and sends it the song at
    /// `GLASS_LIVE_SONG`: connections made to the iPhone, over the network.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_demo_computer() {
        let mine = PathBuf::from(std::env::var_os("GLASS_LIVE_SONG").expect("GLASS_LIVE_SONG: a song to send"));
        let track = local::read_paths(std::slice::from_ref(&mine)).pop().expect("the song to send");
        let inbox = tmp("live-demo-inbox");
        let (tx, rx) = std::sync::mpsc::channel();
        let who = Me { instance: "livedemo0000".into(), name: "Live test computer".into(), account: account_secret("me", "you@example.com"), username: "me".into() };
        let share = Share::start(tokio::runtime::Handle::current(), who, Arc::new(RwLock::new(Arc::new(vec![track.clone()]))), inbox.clone(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let t0 = Instant::now();
        while share.peers().is_empty() && t0.elapsed() < Duration::from_secs(30) {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let phone = share.peers().first().cloned().expect("the iPhone app is found on the network");
        println!("found {} ({:?}, port {})", phone.name, phone.addrs, phone.port);
        let listed = list_from(&share.ctx, &phone).await.unwrap_or_else(|e| panic!("listing {}: {e:#}", phone.name));
        println!("{} lists {} songs", phone.name, listed.len());
        let want = listed.iter().find(|f| f.title == "Phone Song").expect("its Phone Song is listed");
        let got = get_from(&share.ctx, &phone, vec![want.id.clone()]).await.unwrap_or_else(|e| panic!("getting from {}: {e:#}", phone.name));
        assert_eq!(got.len(), 1);
        assert_eq!(std::fs::metadata(&got[0]).unwrap().len(), want.size, "the song arrives whole");
        send_to(share.ctx.clone(), phone.clone(), vec![track]).await.unwrap_or_else(|e| panic!("sending to {}: {e:#}", phone.name));
        let sent = rx.try_iter().any(|e| matches!(e, Event::Transfer(t) if t.sending && t.state == State::Done && t.done == 1));
        assert!(sent, "{} saved the song sent to it", phone.name);
        println!("ok  got Phone Song from {} and sent it {}", phone.name, mine.display());
        drop(share);
        let _ = std::fs::remove_dir_all(inbox);
    }

    #[test]
    fn account_tags_are_salted_and_only_fit_their_account() {
        let (a, b) = (account_tag("secret", "run1"), account_tag("secret", "run2"));
        assert_ne!(a, b, "a new salt each run: the tag doesn't follow the device");
        assert!(same_account("secret", &a) && same_account("secret", &b));
        assert!(!same_account("another", &a));
        assert!(!same_account("secret", "run1.0000000000000000") && !same_account("secret", "nodot") && !same_account("secret", ".abc"));
    }

    #[test]
    fn only_plain_audio_file_names_are_saved() {
        let dir = tmp("names");
        let (c, _) = ctx("b", "davis", vec![], dir.clone());
        let f = |name: &str| FileMeta { id: String::new(), name: name.into(), size: 10, title: name.into(), artist: String::new(), album: String::new(), duration_ms: 1000 };
        assert_eq!(destination(&c, &f("Song.mp3")), Some(dir.join("Song.mp3")));
        for bad in ["../evil.mp3", "/etc/passwd", "a/b.mp3", ".hidden.mp3", "notes.txt", "run.sh"] {
            assert_eq!(destination(&c, &f(bad)), None, "{bad}");
        }
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Song.mp3"), b"different").unwrap();
        assert_eq!(destination(&c, &f("Song.mp3")), Some(dir.join("Song (2).mp3")), "a different file of the same name isn't overwritten");
        let _ = std::fs::remove_dir_all(dir);
    }
}
