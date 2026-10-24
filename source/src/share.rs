//! Sending local files between your computers running Glass, on the same network.
//!
//! Off unless turned on in Settings. Each Glass that shares announces itself
//! over mDNS (Bonjour) as `_glass-files._tcp`, with a tag that copies signed in
//! to the same Spotify account recognize (an HMAC, keyed with the account's
//! username, of a salt that's new each run, so the tag can't follow a computer
//! around). The username isn't a secret, so this only sorts announcements; the
//! proofs and pairing below are what keep others out.
//!
//! A transfer runs over a Noise (XX) encrypted connection, and before anything
//! else each side proves it's one of the account's computers in Spotify
//! Connect: it sends an HMAC of the connection's handshake hash keyed with its
//! own device ID, which the other side checks against the computers Spotify
//! lists for the account. The proof only fits that one connection (it can't be
//! relayed), and the device ID isn't sent.
//!
//! A device ID isn't a secret, though (Spotify shows it to every app the
//! account uses), so that alone keeps out strangers, not someone who has one
//! of the account's IDs. So the first time two computers connect, both show the
//! same six-digit code and nothing is shared until each user confirms it. The
//! code comes from the handshake hash (which differs at each end if anyone is
//! in between) and a random value from each side, swapped inside the encrypted
//! connection: the dialing side sends a hash of its value first and shows the
//! value only after the other side's has arrived. So neither side can pick its
//! value to steer the code, and a computer in between gets the same code at
//! both ends only by chance (one in a million).
//! Each Glass keeps one Noise key for good, and remembers the keys of the
//! computers it paired with; those connect without asking again. A connection
//! that hasn't proved itself yet is read only a small hello, and at most eight
//! connections are served at once.
//!
//! Two requests: `Send` (here are songs for you) and `Want` (do you have these
//! `spotify:local:` songs?, for a playlist's missing local songs). Received
//! songs are saved in one folder, which joins Local Files so they play.

use crate::local::{self, LocalTrack};
use anyhow::{Context, Result, anyhow, bail};
use futures_util::future::BoxFuture;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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
/// in the pairing code.
const VERSION: u32 = 3;
/// Plaintext per encrypted message (Noise messages are at most 64 KiB).
const CHUNK: usize = 60 * 1024;
const MAX_FILE: u64 = 4 << 30;
const MAX_FILES: usize = 20_000;
/// Give up on a connection that says nothing for this long.
const QUIET: Duration = Duration::from_secs(30);
/// How long a pairing waits for the user's answer.
const PAIR_WAIT: Duration = Duration::from_secs(120);

/// Another Glass on this network, signed in to the same account.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    /// Its announcement's random ID (new each run).
    pub id: String,
    pub name: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
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
    /// A computer this one hasn't paired with: pair only if it shows the same `code`
    /// (answer with `Share::answer`). `incoming`: it asked, not this computer.
    Pair {
        id: u64,
        name: String,
        code: String,
        incoming: bool,
    },
    /// That pairing is over (answered, turned down on the other computer, timed
    /// out, or the other computer left).
    PairDone {
        id: u64,
    },
}

/// Who this Glass is.
#[derive(Clone)]
pub struct Me {
    /// Random, new each run: names the announcement.
    pub instance: String,
    /// This computer's name, as other devices show it.
    pub name: String,
    /// The signed-in Spotify account's username. Never sent: only tags made with it.
    pub username: String,
    /// This Glass's Spotify Connect device ID. Never sent: only proofs made
    /// with it are.
    pub device_id: String,
}

/// The account's Spotify Connect device IDs, fresh from Spotify.
pub type Devices = Arc<dyn Fn() -> BoxFuture<'static, Vec<String>> + Send + Sync>;

/// What a connection needs to know.
#[derive(Clone)]
struct Ctx {
    me: Me,
    /// `account_tag` for this run.
    tag: String,
    key: Arc<snow::Keypair>,
    trust: Arc<Trust>,
    devices: Devices,
    /// Local Files as last scanned (to answer `Want`, and to skip songs already here).
    library: Library,
    /// Where received songs go.
    inbox: PathBuf,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
    next_id: Arc<AtomicU64>,
    /// The account's computers, as Spotify last listed them, and when.
    known: Arc<Mutex<Option<Known>>>,
    /// Connections and transfers under way, stopped when sharing is turned off.
    running: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl Ctx {
    /// Keeps `task` to be stopped with the rest when sharing stops.
    fn track(&self, task: tokio::task::JoinHandle<()>) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|t| !t.is_finished());
        running.push(task.abort_handle());
    }
}

/// When the account's computers were listed, and their device IDs.
type Known = (Instant, Vec<String>);

/// Local Files as the app last scanned them: the same list, not a copy.
pub type Library = Arc<RwLock<Arc<Vec<LocalTrack>>>>;

/// The library right now (the lock is held only to take it).
fn library(ctx: &Ctx) -> Arc<Vec<LocalTrack>> {
    ctx.library.read().map(|l| l.clone()).unwrap_or_default()
}

/// Tags an account without naming it: `salt.mac`, the mac keyed with the
/// account's username. The username isn't a secret, so this only sorts
/// announcements; the proofs and pairing are what keep others out. With a new
/// salt each run, a tag doesn't single out a computer from one network or day
/// to the next.
pub fn account_tag(username: &str, salt: &str) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(username.as_bytes()).expect("any key length");
    m.update(b"glass-lan-v2");
    m.update(salt.as_bytes());
    format!("{salt}.{}", to_hex(&m.finalize().into_bytes()[..8]))
}

/// Whether `tag` was made for `username`'s account.
fn same_account(username: &str, tag: &str) -> bool {
    tag.split_once('.').is_some_and(|(salt, _)| !salt.is_empty() && salt.len() <= 64 && account_tag(username, salt) == tag)
}

/// The six digits both computers show while pairing, from the handshake hash
/// (which covers both computers' keys, so a computer in between makes it
/// differ) and each side's random value. Each side adds its value only after
/// the other has committed to its own (see `pairing_code`), so neither can
/// steer the code.
fn code(handshake: &[u8], dialing: &[u8], answering: &[u8]) -> String {
    let h = Sha256::new().chain_update(handshake).chain_update(dialing).chain_update(answering).finalize();
    let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

/// What the dialing side sends before its random value: a hash of it, so the
/// value can't change once the other side's is known.
fn commitment(handshake: &[u8], value: &[u8]) -> Vec<u8> {
    Sha256::new().chain_update(handshake).chain_update(value).finalize().to_vec()
}

/// This computer's name ("Davis's MacBook Pro"), for other devices to show.
pub fn computer_name() -> String {
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

/// A name another computer gave itself, kept to something a list or dialog can show.
fn shown_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// Where received songs are saved: Music/Glass.
pub fn inbox() -> PathBuf {
    crate::save_song::default_dir()
}

// ------------------------------------------------------------------ proofs

fn proof(device_id: &str, handshake: &[u8], initiator: bool) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(device_id.as_bytes()).expect("any key length");
    m.update(b"glass-lan-v1");
    m.update(&[initiator as u8]);
    m.update(handshake);
    m.finalize().into_bytes().to_vec()
}

/// Which of the account's devices made `hello`'s proof (never this one).
fn verify(hello: &Hello, handshake: &[u8], initiator: bool, me: &Me, devices: &[String]) -> Option<String> {
    if hello.v != VERSION || !same_account(&me.username, &hello.account) {
        return None;
    }
    let claimed = hex(&hello.proof)?;
    devices
        .iter()
        .filter(|d| **d != me.device_id)
        .find(|d| {
            let mut m = <Hmac<Sha256> as Mac>::new_from_slice(d.as_bytes()).expect("any key length");
            m.update(b"glass-lan-v1");
            m.update(&[initiator as u8]);
            m.update(handshake);
            m.verify_slice(&claimed).is_ok()
        })
        .cloned()
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
        Some(Self { name, size, title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone(), duration_ms: t.duration_ms })
    }

    /// The song as a playlist would name it (to spot one that's already here).
    fn spotify_uri(&self) -> String {
        local::spotify_uri(&LocalTrack { title: self.title.clone(), artist: self.artist.clone(), album: self.album.clone(), duration_ms: self.duration_ms, ..Default::default() })
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum Msg {
    /// "Here are songs for you."
    Send(Vec<FileMeta>),
    /// "Do you have these `spotify:local:` songs?"
    Want(Vec<String>),
    /// The songs the sender has to offer (an answer to `Want`).
    Offer(Vec<FileMeta>),
    /// Which offered songs to send, in order.
    Accept(Vec<bool>),
    /// How many songs were saved.
    Saved(usize),
    Refused(String),
    /// Pairing, first: (this side knows the other's key, it would pair now).
    Known(bool, bool),
    /// Pairing, before the code: the dialing side's `commitment` (hex).
    Commit(String),
    /// Pairing, before the code: a side's random value (hex).
    Random(String),
    /// Pairing, then: this side's user confirmed the code.
    Paired(bool),
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
    /// The other computer's Noise key (what pairing remembers).
    remote: Vec<u8>,
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
        let remote = hs.get_remote_static().context("no key from the other side")?.to_vec();
        Ok((Self { stream, noise: hs.into_transport_mode()?, buf, remote }, hash))
    }

    async fn send(&mut self, kind: u8, data: &[u8]) -> Result<()> {
        let mut plain = Vec::with_capacity(data.len() + 1);
        plain.push(kind);
        plain.extend_from_slice(data);
        let n = self.noise.write_message(&plain, &mut self.buf)?;
        write_frame(&mut self.stream, &self.buf[..n]).await
    }

    async fn recv(&mut self) -> Result<(u8, Vec<u8>)> {
        self.recv_within(QUIET).await
    }

    async fn recv_within(&mut self, wait: Duration) -> Result<(u8, Vec<u8>)> {
        let m = read_frame(&mut self.stream, wait).await?;
        let n = self.noise.read_message(&m, &mut self.buf)?;
        let (kind, rest) = self.buf[..n].split_first().ok_or_else(|| anyhow!("empty message"))?;
        Ok((*kind, rest.to_vec()))
    }

    /// Waits until the other side sends something or closes, without reading it
    /// (so it can be dropped for something else at any point).
    async fn arrived(&self) {
        let _ = self.stream.peek(&mut [0u8; 1]).await;
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

    async fn recv_json_up_to<T: DeserializeOwned>(&mut self, max: usize) -> Result<T> {
        self.recv_json_within(max, QUIET).await
    }

    /// A message of at most `max` bytes, starting within `wait`.
    async fn recv_json_within<T: DeserializeOwned>(&mut self, max: usize, wait: Duration) -> Result<T> {
        let mut all = Vec::new();
        loop {
            let (kind, b) = self.recv_within(wait).await?;
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

/// Handshake, then both sides prove they belong to the account, then (the first
/// time) both users confirm they see the same code. Returns the verified
/// connection and the other device's name.
async fn connect(stream: TcpStream, ctx: &Ctx, initiator: bool) -> Result<(Conn, String)> {
    let (mut c, hash) = Conn::open(stream, &ctx.key, initiator).await?;
    let mine = Hello { v: VERSION, name: ctx.me.name.clone(), account: ctx.tag.clone(), proof: to_hex(&proof(&ctx.me.device_id, &hash, initiator)) };
    let name = if initiator {
        c.send_json(&mine).await?;
        let theirs: Hello = match c.recv_json_up_to::<Hello>(MAX_HELLO).await {
            Ok(h) => h,
            Err(_) => bail!("the other computer turned the connection down or didn't answer"),
        };
        if !proven(ctx, &theirs, &hash, !initiator).await {
            bail!("couldn't confirm it's signed in to your Spotify account");
        }
        theirs.name
    } else {
        let theirs: Hello = c.recv_json_up_to(MAX_HELLO).await?;
        if !proven(ctx, &theirs, &hash, !initiator).await {
            bail!("a computer that couldn't prove it's yours");
        }
        c.send_json(&mine).await?;
        theirs.name
    };
    let name = shown_name(&name);
    pair(&mut c, ctx, &name, &hash, !initiator).await?;
    Ok((c, name))
}

/// Unless both computers already know each other's key, both users compare the
/// code before anything is shared; the other's key is then remembered.
async fn pair(c: &mut Conn, ctx: &Ctx, name: &str, hash: &[u8], incoming: bool) -> Result<()> {
    let key = c.remote.clone();
    let known = ctx.trust.knows(&key);
    let open = ctx.trust.open_to(&key);
    // Both send, then both read (each message is small).
    c.send_json(&Msg::Known(known, open)).await?;
    let Msg::Known(they_know, they_open) = c.recv_json_up_to(MAX_HELLO).await? else { bail!("unexpected answer") };
    if known && they_know {
        return Ok(());
    }
    if !open {
        bail!("this computer can't pair with {name} right now (another pairing is waiting here, or {name} was turned down since Glass started)");
    }
    if !they_open {
        bail!("{name} can't pair right now (another pairing is waiting there, or this computer was turned down there since Glass started)");
    }
    let code = pairing_code(c, hash, !incoming).await?;
    let asking = ask(ctx, &key, name, &code, incoming);
    tokio::pin!(asking);
    // Their answer can come first: a yes is kept for after this user's, a no (or
    // the connection closing) ends the wait here, which closes the dialog.
    let mut they = false;
    let yes = loop {
        tokio::select! {
            yes = &mut asking => break yes,
            _ = c.arrived() => {
                if they || !matches!(c.recv_json_up_to(MAX_HELLO).await, Ok(Msg::Paired(true))) {
                    bail!("{name} didn't pair with this computer");
                }
                they = true;
            }
        }
    };
    c.send_json(&Msg::Paired(yes)).await?;
    // Their user may still be deciding.
    let they = they || matches!(c.recv_json_within(MAX_HELLO, PAIR_WAIT + QUIET).await, Ok(Msg::Paired(true)));
    if !yes {
        bail!("not paired with {name}");
    }
    if !they {
        bail!("{name} didn't pair with this computer");
    }
    if !known {
        ctx.trust.add(&key, name);
    }
    Ok(())
}

/// The code both users compare, from a random value of each side's, swapped
/// inside the encrypted connection: the dialing side sends its `commitment`,
/// the other side answers with its value, then the first sends its own, which
/// has to match what it committed to.
async fn pairing_code(c: &mut Conn, hash: &[u8], initiator: bool) -> Result<String> {
    let mine: [u8; 16] = rand::random();
    if initiator {
        c.send_json(&Msg::Commit(to_hex(&commitment(hash, &mine)))).await?;
        let theirs = random_value(c.recv_json_up_to(MAX_HELLO).await?)?;
        c.send_json(&Msg::Random(to_hex(&mine))).await?;
        Ok(code(hash, &mine, &theirs))
    } else {
        let Msg::Commit(commit) = c.recv_json_up_to(MAX_HELLO).await? else { bail!("unexpected answer") };
        c.send_json(&Msg::Random(to_hex(&mine))).await?;
        let theirs = random_value(c.recv_json_up_to(MAX_HELLO).await?)?;
        if hex(&commit) != Some(commitment(hash, &theirs)) {
            bail!("the pairing code didn't check out (another computer may be in between)");
        }
        Ok(code(hash, &theirs, &mine))
    }
}

/// The other side's random value, from its `Random` message.
fn random_value(m: Msg) -> Result<Vec<u8>> {
    match m {
        Msg::Random(v) => hex(&v).filter(|v| v.len() == 16).context("unexpected answer"),
        _ => bail!("unexpected answer"),
    }
}

/// Asks this computer's user to pair with `name`; false if they don't (or don't
/// answer in time, or another pairing is waiting).
async fn ask(ctx: &Ctx, key: &[u8], name: &str, code: &str, incoming: bool) -> bool {
    let Some((id, answer)) = ctx.trust.start_asking() else { return false };
    /// Closes the dialog however the wait ends (`pair` also drops it when the
    /// other computer turns the pairing down or leaves).
    struct Done<'a>(&'a Ctx, u64);
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            self.0.trust.stop_asking(self.1);
            (self.0.on_event)(Event::PairDone { id: self.1 });
        }
    }
    let _done = Done(ctx, id);
    (ctx.on_event)(Event::Pair { id, name: name.to_string(), code: code.to_string(), incoming });
    let yes = tokio::time::timeout(PAIR_WAIT, answer).await.ok().and_then(Result::ok).unwrap_or(false);
    if !yes {
        ctx.trust.decline(key);
    }
    yes
}

/// Whether `hello` proves the other side is one of the account's computers. The
/// device list is asked of Spotify at most every few seconds, however many
/// connections arrive, and only for a hello from this account's tag.
async fn proven(ctx: &Ctx, hello: &Hello, hash: &[u8], initiator: bool) -> bool {
    if hello.v != VERSION || !same_account(&ctx.me.username, &hello.account) {
        return false;
    }
    let cached = ctx.known.lock().ok().and_then(|k| k.clone());
    if let Some((_, devices)) = &cached
        && verify(hello, hash, initiator, &ctx.me, devices).is_some()
    {
        return true;
    }
    // A device not seen yet (a Glass that just started): ask again, unless that was just done.
    if cached.as_ref().is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(5)) {
        return false;
    }
    let devices = (ctx.devices)().await;
    let ok = verify(hello, hash, initiator, &ctx.me, &devices).is_some();
    if let Ok(mut k) = ctx.known.lock() {
        *k = Some((Instant::now(), devices));
    }
    ok
}

// ------------------------------------------------------------------ pairing

/// A computer paired with this one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Paired {
    /// Its Noise key, in hex.
    pub key: String,
    /// Its name when it was paired.
    pub name: String,
}

/// The computers paired with this one, and the pairing waiting for an answer.
pub struct Trust {
    /// Where the list is kept (`None`: in memory only, for tests).
    path: Option<PathBuf>,
    paired: Mutex<Vec<Paired>>,
    /// Keys whose pairing was turned down this run: not asked about again until restart.
    declined: Mutex<std::collections::HashSet<Vec<u8>>>,
    /// One pairing at a time: its ID and where its answer goes.
    asking: Mutex<Option<(u64, tokio::sync::oneshot::Sender<bool>)>>,
    next: AtomicU64,
}

impl Trust {
    pub fn path() -> PathBuf {
        crate::config::config_dir().join("share_paired.json")
    }

    fn load(path: Option<PathBuf>) -> Self {
        let paired = path.as_deref().map(crate::config::read_json).unwrap_or_default();
        Self { path, paired: Mutex::new(paired), declined: Mutex::default(), asking: Mutex::default(), next: AtomicU64::new(1) }
    }

    fn knows(&self, key: &[u8]) -> bool {
        let hex = to_hex(key);
        self.paired.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|p| p.key == hex)
    }

    /// Whether a pairing with `key` could be asked about now.
    fn open_to(&self, key: &[u8]) -> bool {
        !self.declined.lock().unwrap_or_else(|e| e.into_inner()).contains(key) && self.asking.lock().unwrap_or_else(|e| e.into_inner()).is_none()
    }

    fn add(&self, key: &[u8], name: &str) {
        let mut list = self.paired.lock().unwrap_or_else(|e| e.into_inner());
        let hex = to_hex(key);
        list.retain(|p| p.key != hex);
        list.push(Paired { key: hex, name: name.to_string() });
        self.save(&list);
    }

    fn save(&self, list: &[Paired]) {
        let Some(path) = &self.path else { return };
        let r = serde_json::to_vec_pretty(list).map_err(std::io::Error::other).and_then(|b| crate::config::write_private(path, &b));
        if let Err(e) = r {
            log::warn!("paired computers couldn't be saved: {e}");
        }
    }

    fn decline(&self, key: &[u8]) {
        self.declined.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_vec());
    }

    fn start_asking(&self) -> Option<(u64, tokio::sync::oneshot::Receiver<bool>)> {
        let mut asking = self.asking.lock().unwrap_or_else(|e| e.into_inner());
        if asking.is_some() {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        *asking = Some((id, tx));
        Some((id, rx))
    }

    fn stop_asking(&self, id: u64) {
        let mut asking = self.asking.lock().unwrap_or_else(|e| e.into_inner());
        if asking.as_ref().is_some_and(|(i, _)| *i == id) {
            *asking = None;
        }
    }

    fn answer(&self, id: u64, yes: bool) {
        let mut asking = self.asking.lock().unwrap_or_else(|e| e.into_inner());
        if asking.as_ref().is_some_and(|(i, _)| *i == id)
            && let Some((_, tx)) = asking.take()
        {
            let _ = tx.send(yes);
        }
    }

    fn list(&self) -> Vec<Paired> {
        self.paired.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn forget(&self, key: &str) {
        let mut list = self.paired.lock().unwrap_or_else(|e| e.into_inner());
        list.retain(|p| p.key != key);
        self.save(&list);
    }
}

/// This Glass's own Noise key, made once and kept (readable only by this user):
/// the key other computers pair with.
fn identity(path: &Path) -> Result<snow::Keypair> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let mut lines = text.lines();
        if let (Some(private), Some(public)) = (lines.next().and_then(hex), lines.next().and_then(hex))
            && private.len() == 32
            && public.len() == 32
        {
            return Ok(snow::Keypair { private, public });
        }
        log::warn!("{} isn't a key; making a new one (computers paired before ask again)", path.display());
    }
    let params: snow::params::NoiseParams = NOISE.parse()?;
    let key = snow::Builder::new(params).generate_keypair()?;
    crate::config::write_private(path, format!("{}\n{}\n", to_hex(&key.private), to_hex(&key.public)).as_bytes())?;
    Ok(key)
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
        Msg::Send(offer) => {
            let mut t = new_transfer(&ctx, &peer, false);
            let (saved, r) = take(&mut c, &ctx, &mut t, &offer).await;
            finish(&ctx, t, &peer, saved, r)
        }
        Msg::Want(uris) => {
            // Answer from this computer's Local Files.
            let lib = library(&ctx);
            let have: Vec<(LocalTrack, FileMeta)> = uris.iter().filter_map(|u| local::find(&lib, u).map(|i| lib[i].clone())).filter_map(|t| FileMeta::of(&t).map(|m| (t, m))).collect();
            c.send_json(&Msg::Offer(have.iter().map(|h| h.1.clone()).collect())).await?;
            if have.is_empty() {
                return Ok(());
            }
            let mut t = new_transfer(&ctx, &peer, true);
            let r = give(&mut c, &ctx, &mut t, &have).await;
            finish(&ctx, t, &peer, Vec::new(), r.map(|_| ()))
        }
        _ => {
            c.send_json(&Msg::Refused("not understood".into())).await?;
            Ok(())
        }
    }
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
    let mut addrs: Vec<IpAddr> = peer.addrs.clone();
    addrs.sort_by_key(|a| !a.is_ipv4());
    for a in addrs {
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
    let (mut c, _) = connect(dial(peer).await?, ctx, true).await?;
    c.send_json(&Msg::Want(uris)).await?;
    let Msg::Offer(offer) = c.recv_msg().await? else { bail!("unexpected answer") };
    if offer.is_empty() || offer.len() > MAX_FILES {
        return Ok(Vec::new());
    }
    let mut t = new_transfer(ctx, &peer.name, false);
    let (saved, r) = take(&mut c, ctx, &mut t, &offer).await;
    finish(ctx, t, &peer.name, saved.clone(), r)?;
    Ok(saved)
}

// ------------------------------------------------------------------ service

/// Sharing, while it's on: announces this Glass, finds the others, accepts
/// their transfers. Stops when dropped.
pub struct Share {
    ctx: Ctx,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    mdns: Option<mdns_sd::ServiceDaemon>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    rt: tokio::runtime::Handle,
}

impl Share {
    pub fn start(rt: tokio::runtime::Handle, me: Me, devices: Devices, library: Library, inbox: PathBuf, on_event: impl Fn(Event) + Send + Sync + 'static) -> Result<Self> {
        let key = Arc::new(identity(&crate::config::config_dir().join("share_key"))?);
        let trust = Arc::new(Trust::load(Some(Trust::path())));
        let tag = account_tag(&me.username, &me.instance);
        let ctx = Ctx { me, tag, key, trust, devices, library, inbox, on_event: Arc::new(on_event), next_id: Arc::default(), known: Arc::default(), running: Arc::default() };
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
        let peers: Arc<Mutex<HashMap<String, Peer>>> = Arc::default();
        let mdns = Self::announce(&rt, &ctx, port, &peers, &mut tasks).map_err(|e| log::warn!("local files sharing can't use the network: {e:#}")).ok();
        Ok(Self { ctx, peers, mdns, tasks, rt })
    }

    fn announce(rt: &tokio::runtime::Handle, ctx: &Ctx, port: u16, peers: &Arc<Mutex<HashMap<String, Peer>>>, tasks: &mut Vec<tokio::task::JoinHandle<()>>) -> Result<mdns_sd::ServiceDaemon> {
        let mdns = mdns_sd::ServiceDaemon::new()?;
        // Networks don't come and go every few seconds (the default check).
        let _ = mdns.set_ip_check_interval(30);
        let me = &ctx.me;
        let props = [("v", VERSION.to_string()), ("acct", ctx.tag.clone()), ("name", me.name.clone())];
        let info = mdns_sd::ServiceInfo::new(SERVICE, &me.instance, &format!("glass-{}.local.", me.instance), (), port, &props[..])?.enable_addr_auto();
        mdns.register(info)?;
        let events = mdns.browse(SERVICE)?;
        let (peers, ctx) = (peers.clone(), ctx.clone());
        tasks.push(rt.spawn(async move {
            while let Ok(ev) = events.recv_async().await {
                let changed = match ev {
                    mdns_sd::ServiceEvent::ServiceResolved(info) => {
                        // This account's, and a Glass that speaks this version (an older one would only fail).
                        let ours = info.get_property_val_str("acct").is_some_and(|t| same_account(&ctx.me.username, t)) && info.get_property_val_str("v") == Some(VERSION.to_string().as_str());
                        let id = info.get_fullname().split('.').next().unwrap_or("").to_string();
                        if !ours || id == ctx.me.instance || id.is_empty() {
                            false
                        } else {
                            let name = shown_name(info.get_property_val_str("name").unwrap_or("Glass"));
                            let mut addrs: Vec<IpAddr> = info.get_addresses().iter().cloned().collect();
                            addrs.sort();
                            let peer = Peer { id: id.clone(), name, addrs, port: info.get_port() };
                            peers.lock().unwrap().insert(id, peer.clone()) != Some(peer)
                        }
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => peers.lock().unwrap().remove(fullname.split('.').next().unwrap_or("")).is_some(),
                    _ => false,
                };
                if changed {
                    let mut list: Vec<Peer> = peers.lock().unwrap().values().cloned().collect();
                    list.sort_by(|a, b| a.name.cmp(&b.name));
                    (ctx.on_event)(Event::Peers(list));
                }
            }
        }));
        Ok(mdns)
    }

    /// The user's answer to `Event::Pair`.
    pub fn answer(&self, id: u64, yes: bool) {
        self.ctx.trust.answer(id, yes);
    }

    /// The computers paired with this one.
    pub fn paired(&self) -> Vec<Paired> {
        self.ctx.trust.list()
    }

    /// Unpairs a computer: it has to be confirmed again next time.
    pub fn forget(&self, key: &str) {
        self.ctx.trust.forget(key);
    }

    pub fn peers(&self) -> Vec<Peer> {
        let mut list: Vec<Peer> = self.peers.lock().unwrap().values().cloned().collect();
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
}

impl Drop for Share {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        for t in self.ctx.running.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            t.abort();
        }
        if let Some(m) = self.mdns.take() {
            let _ = m.unregister(&format!("{}.{SERVICE}", self.ctx.me.instance));
            let _ = m.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(device: &str) -> Me {
        Me { instance: format!("i-{device}"), name: format!("Mac {device}"), username: "davis".into(), device_id: device.into() }
    }

    fn ctx(device: &str, account_devices: Vec<String>, library: Vec<LocalTrack>, inbox: PathBuf) -> (Ctx, Arc<Mutex<Vec<Event>>>) {
        ctx_answering(device, account_devices, library, inbox, Some(true))
    }

    /// A device whose user answers every pairing with `pair` (`None`: never answers).
    fn ctx_answering(device: &str, account_devices: Vec<String>, library: Vec<LocalTrack>, inbox: PathBuf, pair: Option<bool>) -> (Ctx, Arc<Mutex<Vec<Event>>>) {
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let ev = events.clone();
        let params: snow::params::NoiseParams = NOISE.parse().unwrap();
        let devices: Devices = Arc::new(move || {
            let d = account_devices.clone();
            Box::pin(async move { d })
        });
        let trust = Arc::new(Trust::load(None));
        let answers = trust.clone();
        let me = me(device);
        let c = Ctx {
            tag: account_tag(&me.username, &me.instance),
            me,
            trust,
            key: Arc::new(snow::Builder::new(params).generate_keypair().unwrap()),
            devices,
            library: Arc::new(RwLock::new(Arc::new(library))),
            inbox,
            on_event: Arc::new(move |e| {
                if let (Event::Pair { id, .. }, Some(yes)) = (&e, pair) {
                    answers.answer(*id, yes);
                }
                ev.lock().unwrap().push(e)
            }),
            next_id: Arc::default(),
            known: Arc::default(),
            running: Arc::default(),
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
        Peer { id: "b".into(), name: "Mac B".into(), addrs: vec!["127.0.0.1".parse().unwrap()], port }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = crate::config::test_dir(&format!("share-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn proofs_only_fit_their_connection_and_account() {
        let me_a = me("dev-a");
        let hash = [7u8; 32];
        let hello = Hello { v: VERSION, name: "A".into(), account: account_tag(&me_a.username, &me_a.instance), proof: to_hex(&proof("dev-a", &hash, true)) };
        let me_b = me("dev-b");
        let devices = vec!["dev-a".to_string(), "dev-b".to_string()];
        assert_eq!(verify(&hello, &hash, true, &me_b, &devices).as_deref(), Some("dev-a"));
        assert!(verify(&hello, &[8u8; 32], true, &me_b, &devices).is_none(), "relayed into another connection");
        assert!(verify(&hello, &hash, false, &me_b, &devices).is_none(), "reflected back with the other role");
        assert!(verify(&hello, &hash, true, &me_b, &["dev-b".to_string(), "dev-x".to_string()]).is_none(), "not one of the account's devices");
        let other = Me { username: "someone-else".into(), ..me_b.clone() };
        assert!(verify(&hello, &hash, true, &other, &devices).is_none(), "another account");
        assert!(verify(&hello, &hash, true, &me("dev-a"), &devices).is_none(), "can't claim to be this device");
    }

    #[tokio::test]
    async fn songs_go_across_and_only_to_the_same_account() {
        let (src, dst) = (tmp("src"), tmp("dst"));
        let devices = vec!["dev-a".to_string(), "dev-b".to_string()];
        let a_songs = vec![song(&src, "Remember Summer Days.mp3", "Remember Summer Days", 150_000), song(&src, "Last Summer Whisper.flac", "Last Summer Whisper", 70_000)];
        // B already has the second song (another copy), so only the first comes over.
        let b_lib = vec![LocalTrack { path: dst.join("elsewhere.flac"), ..a_songs[1].clone() }];
        let (a, _) = ctx("dev-a", devices.clone(), a_songs.clone(), tmp("a-inbox"));
        let (b, b_events) = ctx("dev-b", devices.clone(), b_lib, dst.clone());
        let b_library = b.library.clone();
        let peer = listen(b).await;
        send_to(a.clone(), peer.clone(), a_songs.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let got = std::fs::read(dst.join("Remember Summer Days.mp3")).unwrap();
        assert_eq!(got, std::fs::read(&a_songs[0].path).unwrap(), "the song arrives intact");
        assert!(!dst.join("Last Summer Whisper.flac").exists(), "a song B already has isn't sent");
        assert!(b_events.lock().unwrap().iter().any(|e| matches!(e, Event::Received { from, paths } if from == "Mac dev-a" && paths.len() == 1)));

        // A device that isn't on the account is turned away.
        let (stranger, _) = ctx("dev-x", devices.clone(), a_songs.clone(), tmp("x-inbox"));
        let e = send_to(stranger, peer.clone(), a_songs.clone()).await.unwrap_err();
        assert!(e.to_string().contains("turned the connection down"), "{e:#}");

        // Pull: another device asks B for a playlist's local song, which B now
        // has (once Local Files has looked again).
        *b_library.write().unwrap() = Arc::new(
            local::read_paths(std::slice::from_ref(&dst))
                .into_iter()
                .map(|t| LocalTrack { title: a_songs[0].title.clone(), artist: "Anri".into(), album: "Timely!!".into(), duration_ms: 250_000, ..t })
                .collect(),
        );
        let (want, _) = ctx("dev-a", devices.clone(), vec![], tmp("pull-inbox"));
        let uri = local::spotify_uri(&a_songs[0]);
        let saved = want_from(&want, &peer, vec![uri, "spotify:local:Nobody::Nothing:100".into()]).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(std::fs::read(&saved[0]).unwrap().len(), 150_000);
        for d in [src, dst, tmp("a-inbox"), tmp("x-inbox"), tmp("pull-inbox")] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// Plays a second Glass against the one running on this computer: its own
    /// Spotify Connect device (a temporary config), found over the network,
    /// verified against the account's real device list, and asked for one of
    /// your local songs (read only; the copy goes to a temporary folder).
    /// Needs Glass running and signed in, with sharing on:
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
        let uri = local::spotify_uri(&song);
        println!("asking for {uri}");

        // This test is a second Glass, with its own config and so its own
        // device IDs (sharing the running Glass's would take over its
        // connection to Spotify's player state).
        let home = tmp("live-home");
        std::fs::create_dir_all(&home).unwrap();
        let real_home = std::env::var_os("HOME");
        // SAFETY: an ignored test run on its own.
        unsafe { std::env::set_var("HOME", &home) };
        let sp = crate::sp::Sp::connect_cached(creds.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let me_json = sp.handle(&http::Method::GET, "/me", None, None).await.unwrap();
        let username = me_json["id"].as_str().unwrap().to_string();
        let dsp = Arc::new(crate::dsp::DspParams::new(crate::dsp::DspState { eq_enabled: false, gains: [0.0; 6], mono: false }));
        let settings = Settings { volume: 0.0, device_name: "Glass (sharing test)".into(), ..Default::default() };
        let engine = crate::engine::Engine::start_with(&settings, creds, dsp, |_| {}).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let s2 = sp.clone();
        let devices: Devices = Arc::new(move || {
            let sp = s2.clone();
            Box::pin(async move {
                sp.handle(&http::Method::GET, "/me/player/devices", None, None)
                    .await
                    .ok()
                    .and_then(|v| v["devices"].as_array().map(|d| d.iter().filter_map(|x| x["id"].as_str().map(str::to_string)).collect()))
                    .unwrap_or_default()
            })
        });
        let listed = devices().await;
        assert!(listed.contains(&engine.device_id), "Spotify lists the test device");
        let (tx, rx) = std::sync::mpsc::channel();
        let inbox = tmp("live-inbox");
        let who = Me { instance: "livetest0000".into(), name: "Sharing test".into(), username: username.clone(), device_id: engine.device_id.clone() };
        let share = Share::start(tokio::runtime::Handle::current(), who, devices, Arc::default(), inbox.clone(), move |e| {
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
        let paths = want_from(&share.ctx, &peers[0], vec![uri]).await.unwrap_or_else(|e| panic!("asking {}: {e:#}", peers[0].name));
        assert_eq!(paths.len(), 1, "the song arrived");
        let from = match rx.try_iter().find(|e| matches!(e, Event::Received { .. })) {
            Some(Event::Received { from, .. }) => from,
            _ => String::new(),
        };
        assert_eq!(std::fs::metadata(&paths[0]).unwrap().len(), std::fs::metadata(&song.path).unwrap().len());
        println!("ok  got {} from {from} in {:.1} s", paths[0].display(), t0.elapsed().as_secs_f32());
        drop(share);
        drop(engine);
        if let Some(h) = real_home {
            unsafe { std::env::set_var("HOME", h) };
        }
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(inbox);
    }

    #[test]
    fn account_tags_are_salted_and_only_fit_their_account() {
        let (a, b) = (account_tag("davis", "run1"), account_tag("davis", "run2"));
        assert_ne!(a, b, "a new salt each run: the tag doesn't follow the computer");
        assert!(same_account("davis", &a) && same_account("davis", &b));
        assert!(!same_account("someone-else", &a));
        assert!(!same_account("davis", "run1.0000000000000000") && !same_account("davis", "nodot") && !same_account("davis", ".abc"));
    }

    /// Both ends of a new encrypted connection over loopback, and its handshake hash.
    async fn connection() -> (Conn, Conn, Vec<u8>) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let key = || snow::Builder::new(NOISE.parse().unwrap()).generate_keypair().unwrap();
        let (ka, kb) = (key(), key());
        let dial = async { Conn::open(TcpStream::connect(addr).await.unwrap(), &ka, true).await.unwrap() };
        let answer = async { Conn::open(l.accept().await.unwrap().0, &kb, false).await.unwrap() };
        let ((a, hash), (b, _)) = tokio::join!(dial, answer);
        (a, b, hash)
    }

    #[tokio::test]
    async fn pairing_codes_match_and_a_changed_value_is_caught() {
        assert_eq!(code(&[7; 32], &[1; 16], &[2; 16]), "943 512");
        assert_eq!(code(&[7; 32], &[1; 16], &[3; 16]), "192 555", "either side's value changes the code");
        let (mut a, mut b, hash) = connection().await;
        let (x, y) = tokio::join!(pairing_code(&mut a, &hash, true), pairing_code(&mut b, &hash, false));
        assert_eq!(x.unwrap(), y.unwrap(), "both ends show the same code");

        // A dialing side that shows another value than the one it committed to.
        let (mut a, mut b, hash) = connection().await;
        let cheat = async {
            a.send_json(&Msg::Commit(to_hex(&commitment(&hash, &[1; 16])))).await.unwrap();
            let _: Msg = a.recv_json_up_to(MAX_HELLO).await.unwrap();
            a.send_json(&Msg::Random(to_hex(&[2; 16]))).await.unwrap();
        };
        let (_, r) = tokio::join!(cheat, pairing_code(&mut b, &hash, false));
        assert!(r.is_err(), "the pairing fails");
    }

    #[tokio::test]
    async fn computers_pair_once_with_a_code_and_a_no_stops_it() {
        let (src, dst) = (tmp("pair-src"), tmp("pair-dst"));
        let devices = vec!["dev-a".to_string(), "dev-b".to_string(), "dev-c".to_string()];
        let songs = vec![song(&src, "One.mp3", "One", 1000), song(&src, "Two.mp3", "Two", 1000)];
        let (a, a_events) = ctx("dev-a", devices.clone(), songs.clone(), tmp("pair-a"));
        let (b, b_events) = ctx("dev-b", devices.clone(), vec![], dst.clone());
        let (a_trust, b_trust) = (a.trust.clone(), b.trust.clone());
        let peer = listen(b).await;
        send_to(a.clone(), peer.clone(), songs[..1].to_vec()).await.unwrap();
        let asked = |ev: &Arc<Mutex<Vec<Event>>>| ev.lock().unwrap().iter().filter(|e| matches!(e, Event::Pair { .. })).count();
        let codes = |ev: &Arc<Mutex<Vec<Event>>>| ev.lock().unwrap().iter().find_map(|e| if let Event::Pair { code, .. } = e { Some(code.clone()) } else { None });
        assert_eq!((asked(&a_events), asked(&b_events)), (1, 1), "both users are asked the first time");
        assert_eq!(codes(&a_events), codes(&b_events), "and both see the same code");
        assert_eq!((a_trust.list().len(), b_trust.list().len()), (1, 1));
        // Paired: the next transfer doesn't ask.
        send_to(a.clone(), peer.clone(), songs[1..].to_vec()).await.unwrap();
        assert_eq!((asked(&a_events), asked(&b_events)), (1, 1));
        assert!(dst.join("Two.mp3").exists());

        // A computer whose user says no gets nothing, and isn't asked about again this run.
        let (c, _) = ctx_answering("dev-c", devices.clone(), vec![], tmp("pair-c"), Some(false));
        let other = Peer { id: "c".into(), ..peer.clone() };
        let picky = listen(c).await;
        let e = send_to(a.clone(), Peer { port: picky.port, ..other }, songs.clone()).await.unwrap_err();
        assert!(e.to_string().contains("didn't pair") || e.to_string().contains("can't pair"), "{e:#}");
        for d in [src, dst, tmp("pair-a"), tmp("pair-c")] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[tokio::test]
    async fn a_no_on_the_other_computer_closes_the_dialog_here() {
        let src = tmp("pair-wait-src");
        let devices = vec!["dev-a".to_string(), "dev-c".to_string()];
        let songs = vec![song(&src, "One.mp3", "One", 1000)];
        // A's user hasn't answered yet when C's says no.
        let (a, a_events) = ctx_answering("dev-a", devices.clone(), songs.clone(), tmp("pair-wait-a"), None);
        let (c, _) = ctx_answering("dev-c", devices, vec![], tmp("pair-wait-c"), Some(false));
        let peer = listen(c).await;
        let r = tokio::time::timeout(Duration::from_secs(10), send_to(a, peer, songs)).await.expect("A stops waiting for its user");
        assert!(r.unwrap_err().to_string().contains("didn't pair"));
        assert!(a_events.lock().unwrap().iter().any(|e| matches!(e, Event::PairDone { .. })), "and A's dialog closes");
        let _ = std::fs::remove_dir_all(src);
    }

    #[test]
    fn the_key_is_kept() {
        let dir = tmp("key");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("share_key");
        let first = identity(&path).unwrap();
        let again = identity(&path).unwrap();
        assert_eq!((first.private.clone(), first.public.clone()), (again.private, again.public));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "only this user can read it");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_plain_audio_file_names_are_saved() {
        let dir = tmp("names");
        let (c, _) = ctx("dev-b", vec![], vec![], dir.clone());
        let f = |name: &str| FileMeta { name: name.into(), size: 10, title: name.into(), artist: String::new(), album: String::new(), duration_ms: 1000 };
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
