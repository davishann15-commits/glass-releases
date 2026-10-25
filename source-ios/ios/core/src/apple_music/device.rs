//! This iPhone's own device services, reached through LocalDevVPN (it routes
//! 10.7.0.1 back to the phone): ByeTunes' DeviceManager connection code on the
//! idevice crate itself instead of its C wrapper.
//!
//! - Before iOS 26.4: lockdown over TCP with a pairing file (from idevice_pair).
//! - From iOS 26.4 on, lockdown isn't reachable that way any more: a RemotePairing
//!   tunnel (an "RP pairing file") carries the same services (AFC through its
//!   `.shim.remote` names).
//!
//! A heartbeat runs while connected, as the services expect of a paired host.

use idevice::afc::AfcClient;
use idevice::afc::opcode::AfcFopenMode;
use idevice::core_device::AppServiceClient;
use idevice::heartbeat::HeartbeatClient;
use idevice::pairing_file::PairingFile;
use idevice::provider::TcpProvider;
use idevice::remote_pairing::{RemotePairingClient, RpPairingFile, RpPairingSocket};
use idevice::rsd::RsdHandshake;
use idevice::tcp::handle::AdapterHandle;
use idevice::{IdeviceError, IdeviceService, ReadWrite, RsdService};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// LocalDevVPN's address for the iPhone itself.
const HOST: Ipv4Addr = Ipv4Addr::new(10, 7, 0, 1);
const LOCKDOWN_PORT: u16 = 62078;
/// The usual RemotePairing port (Bonjour has the real one when it differs).
const RP_PORT: u16 = 49152;
const LABEL: &str = "Glass";
const CONNECT: Duration = Duration::from_secs(8);
/// How long one file operation (or one chunk of a file) may take before the connection
/// counts as lost (LocalDevVPN turned off, the phone locked…).
const STALL: Duration = Duration::from_secs(60);
const CHUNK: usize = 1 << 20;

/// A pairing file, as idevice_pair makes them.
#[derive(Clone)]
pub enum Pairing {
    /// The classic one (lockdown), for iOS before 26.4.
    Lockdown(Box<PairingFile>),
    /// An RP pairing file (RemotePairing), for iOS 26.4 and later.
    Remote(Box<RpPairingFile>),
}

impl Pairing {
    pub fn parse(bytes: &[u8]) -> Option<Pairing> {
        if let Ok(rp) = RpPairingFile::from_bytes(bytes) {
            return Some(Pairing::Remote(Box::new(rp)));
        }
        PairingFile::from_bytes(bytes).ok().map(|p| Pairing::Lockdown(Box::new(p)))
    }
}

/// iOS 26.4 and later reach these services only through RemotePairing.
pub fn needs_remote_pairing(major: u32, minor: u32) -> bool {
    major > 26 || (major == 26 && minor >= 4)
}

/// This iPhone's iOS version (26.4 → (26, 4)).
pub fn os_version() -> (u32, u32) {
    static VERSION: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    *VERSION.get_or_init(read_os_version)
}

fn read_os_version() -> (u32, u32) {
    let mut buf = [0u8; 32];
    let mut len = buf.len();
    // SAFETY: a valid name, buffer and length for sysctlbyname.
    let ok = unsafe { sysctlbyname(c"kern.osproductversion".as_ptr(), buf.as_mut_ptr() as *mut _, &mut len, std::ptr::null_mut(), 0) } == 0;
    if !ok {
        return (26, 0);
    }
    let s = String::from_utf8_lossy(&buf[..len.saturating_sub(1)]).into_owned();
    let mut parts = s.trim_end_matches('\0').split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(26), parts.next().unwrap_or(0))
}

unsafe extern "C" {
    fn sysctlbyname(name: *const std::ffi::c_char, old: *mut std::ffi::c_void, oldlen: *mut usize, new: *mut std::ffi::c_void, newlen: usize) -> i32;
}

enum Link {
    Lockdown(TcpProvider),
    Tunnel { adapter: AdapterHandle, rsd: RsdHandshake },
}

pub struct Device {
    link: Link,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Some(h) = self.heartbeat.take() {
            h.abort();
        }
    }
}

/// What's in a folder on the device: its names, or that it isn't there.
pub enum Listing {
    Files(Vec<String>),
    Missing,
}

fn plain(e: IdeviceError) -> String {
    e.to_string()
}

/// A file operation, failing when the device stops answering.
async fn stall<T>(f: impl std::future::Future<Output = Result<T, IdeviceError>>) -> Result<T, IdeviceError> {
    tokio::time::timeout(STALL, f).await.unwrap_or_else(|_| Err(IdeviceError::InternalError("the iPhone stopped answering (is LocalDevVPN still connected?)".into())))
}

async fn timed<T>(what: &str, f: impl std::future::Future<Output = Result<T, IdeviceError>>) -> Result<T, String> {
    match tokio::time::timeout(CONNECT, f).await {
        Ok(r) => r.map_err(|e| format!("{what}: {e}")),
        Err(_) => Err(format!("{what}: no answer")),
    }
}

impl Device {
    /// Connects to this iPhone and starts the heartbeat. `ports`: RemotePairing ports
    /// found by Bonjour, tried after the usual one.
    pub async fn connect(pairing: &Pairing, ports: &[u16]) -> Result<Device, String> {
        let link = match pairing {
            Pairing::Lockdown(file) => {
                // A plain TCP connect first: no VPN answers at once, where lockdown's own
                // handshake would wait.
                tokio::time::timeout(CONNECT, tokio::net::TcpStream::connect((HOST, LOCKDOWN_PORT))).await.map_err(|_| unreachable())?.map_err(|_| unreachable())?;
                Link::Lockdown(TcpProvider { addr: IpAddr::V4(HOST), scope_id: None, pairing_file: (**file).clone(), label: LABEL.into() })
            }
            Pairing::Remote(file) => tunnel(file, ports).await?,
        };
        let mut device = Device { link, heartbeat: None };
        let hb = device.heartbeat_client().await?;
        device.heartbeat = Some(tokio::spawn(beat(hb)));
        Ok(device)
    }

    async fn heartbeat_client(&mut self) -> Result<HeartbeatClient, String> {
        match &mut self.link {
            Link::Lockdown(p) => timed("heartbeat", HeartbeatClient::connect(&*p)).await,
            Link::Tunnel { adapter, rsd } => timed("heartbeat", HeartbeatClient::connect_rsd(adapter, rsd)).await,
        }
    }

    /// A new AFC connection (the media folders: /iTunes_Control).
    pub async fn afc(&mut self) -> Result<AfcClient, String> {
        match &mut self.link {
            Link::Lockdown(p) => timed("file access", AfcClient::connect(&*p)).await,
            Link::Tunnel { adapter, rsd } => {
                // Through the tunnel AFC sometimes refuses the first tries (ByeTunes retries too).
                let mut last = String::new();
                for _ in 0..8 {
                    match timed("file access", AfcClient::connect_rsd(adapter, rsd)).await {
                        Ok(c) => return Ok(c),
                        Err(e) => last = e,
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(last)
            }
        }
    }

    /// Ends the Music app if it's running, so it isn't holding the library while it's
    /// replaced (ByeTunes' `terminateMusicAppIfRunning`). Best effort.
    pub async fn quit_music(&mut self) -> bool {
        let r = match &mut self.link {
            Link::Tunnel { adapter, rsd } => app_service(adapter, rsd).await,
            Link::Lockdown(p) => quit_music_lockdown(p).await,
        };
        r.unwrap_or_else(|e| {
            log::info!("Add to Apple Music: couldn't close the Music app: {e}");
            false
        })
    }
}

fn unreachable() -> String {
    "Glass can't reach this iPhone. Open LocalDevVPN and connect it, then try again.".into()
}

async fn tunnel(file: &RpPairingFile, ports: &[u16]) -> Result<Link, String> {
    let mut tried = vec![RP_PORT];
    tried.extend(ports.iter().copied().filter(|p| *p != RP_PORT));
    let mut stream = None;
    for port in tried {
        if let Ok(Ok(s)) = tokio::time::timeout(CONNECT, tokio::net::TcpStream::connect((HOST, port))).await {
            stream = Some(s);
            break;
        }
    }
    let stream = stream.ok_or_else(unreachable)?;
    let mut rp = file.clone();
    let mut rpc = RemotePairingClient::new(RpPairingSocket::new(stream), LABEL);
    timed("pairing", rpc.attempt_pair_verify()).await?;
    // Only verify: a full pairing (a code on screen) isn't something to start from here.
    timed("pairing", rpc.validate_pairing(&mut rp))
        .await
        .map_err(|_| "This iPhone doesn't accept that pairing file. Make a new RP pairing file for it with idevice_pair and import it again.".to_string())?;
    let port = timed("tunnel", rpc.create_tcp_listener()).await?;
    let raw = tokio::time::timeout(CONNECT, tokio::net::TcpStream::connect(SocketAddr::new(IpAddr::V4(HOST), port))).await.map_err(|_| unreachable())?.map_err(|e| format!("tunnel: {e}"))?;
    let tunnel = timed("tunnel", idevice::remote_pairing::connect_tls_psk_tunnel_native(raw, rpc.encryption_key())).await?;
    let client: IpAddr = tunnel.info.client_address.parse().map_err(|e| format!("tunnel: {e}"))?;
    let server: IpAddr = tunnel.info.server_address.parse().map_err(|e| format!("tunnel: {e}"))?;
    let mtu = tunnel.info.mtu as usize;
    let rsd_port = tunnel.info.server_rsd_port;
    let mut adapter = idevice::tcp::adapter::Adapter::new(Box::new(tunnel.into_inner()), client, server);
    adapter.set_mss(mtu.min(8192) - 60);
    let mut adapter = adapter.to_async_handle();
    let stream = timed("tunnel", async { adapter.connect(rsd_port).await.map_err(IdeviceError::from) }).await?;
    let rsd = timed("tunnel", RsdHandshake::new(stream)).await?;
    Ok(Link::Tunnel { adapter, rsd })
}

/// Answers the device's heartbeat ("Marco" → "Polo") until dropped.
async fn beat(mut hb: HeartbeatClient) {
    let mut interval = 10;
    loop {
        match hb.get_marco(interval + 5).await {
            Ok(n) => {
                if n > 0 {
                    interval = n;
                }
                if hb.send_polo().await.is_err() {
                    break;
                }
            }
            Err(IdeviceError::Heartbeat(idevice::HeartbeatError::Timeout)) => continue,
            Err(_) => break,
        }
    }
}

/// The app service through the tunnel, on the adapter's own stream type: going
/// through `RsdService` (a `Box<dyn ReadWrite>`) makes rustc reject the run's future
/// as not Send (a higher-ranked lifetime it can't prove; rust-lang/rust#110338).
async fn app_service(adapter: &mut AdapterHandle, rsd: &RsdHandshake) -> Result<bool, IdeviceError> {
    let port = rsd.services.get("com.apple.coredevice.appservice").ok_or(IdeviceError::ServiceNotFound)?.port;
    let stream = adapter.connect(port).await?;
    kill_music(&mut AppServiceClient::new(stream).await?).await
}

async fn kill_music<R: ReadWrite>(apps: &mut AppServiceClient<R>) -> Result<bool, IdeviceError> {
    let mut killed = false;
    for p in apps.list_processes().await? {
        let exe = p.executable_url.map(|u| u.relative).unwrap_or_default();
        let music = exe.contains("MobileMusicPlayer") || exe.contains("/Music.app/");
        if p.pid == 0 || !music {
            continue;
        }
        killed |= apps.send_signal(p.pid, 15).await.is_ok();
        tokio::time::sleep(Duration::from_millis(250)).await;
        killed |= apps.send_signal(p.pid, 9).await.is_ok();
    }
    Ok(killed)
}

/// Before iOS 26.4 the app service is reached through lockdown's CoreDevice proxy.
async fn quit_music_lockdown(p: &TcpProvider) -> Result<bool, IdeviceError> {
    let proxy = idevice::core_device_proxy::CoreDeviceProxy::connect(p).await?;
    let rsd_port = proxy.tunnel_info().server_rsd_port;
    let mut adapter = proxy.create_software_tunnel().map_err(|e| IdeviceError::InternalError(e.to_string()))?.to_async_handle();
    let stream = adapter.connect(rsd_port).await?;
    let rsd = RsdHandshake::new(stream).await?;
    app_service(&mut adapter, &rsd).await
}

fn not_found(e: &IdeviceError) -> bool {
    matches!(e, IdeviceError::Afc(idevice::afc::errors::AfcError::ObjectNotFound))
}

/// A folder's names ("." and ".." left out), or that there's no such folder.
pub async fn list(afc: &mut AfcClient, path: &str) -> Result<Listing, String> {
    match stall(afc.list_dir(path)).await {
        Ok(names) => Ok(Listing::Files(names.into_iter().filter(|n| n != "." && n != "..").collect())),
        Err(e) if not_found(&e) => Ok(Listing::Missing),
        Err(e) => Err(format!("the music folder couldn't be read ({e})")),
    }
}

/// Copies a device file to `to`; false when there's no such file.
pub async fn download(afc: &mut AfcClient, path: &str, to: &Path) -> Result<bool, String> {
    let mut f = match stall(afc.open(path, AfcFopenMode::RdOnly)).await {
        Ok(f) => f,
        Err(e) if not_found(&e) => return Ok(false),
        Err(e) => return Err(format!("{path} couldn't be read ({e})")),
    };
    let mut out = tokio::fs::File::create(to).await.map_err(|e| e.to_string())?;
    loop {
        let bytes = stall(f.read_n(CHUNK)).await.map_err(|e| format!("{path} couldn't be read ({e})"))?;
        if bytes.is_empty() {
            break;
        }
        out.write_all(&bytes).await.map_err(|e| e.to_string())?;
    }
    out.flush().await.map_err(|e| e.to_string())?;
    stall(f.close()).await.map_err(plain)?;
    Ok(true)
}

/// Writes `from` to the device at `path` (replacing what's there); `sent` hears each chunk.
pub async fn upload(afc: &mut AfcClient, from: &Path, path: &str, mut sent: impl FnMut(u64)) -> Result<(), String> {
    let mut src = tokio::fs::File::open(from).await.map_err(|e| format!("{} couldn't be read ({e})", from.display()))?;
    let _ = stall(afc.remove(path)).await;
    let mut f = stall(afc.open(path, AfcFopenMode::WrOnly)).await.map_err(|e| format!("{path} couldn't be written ({e})"))?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = src.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        stall(f.write_entire(&buf[..n])).await.map_err(|e| format!("{path} couldn't be written ({e})"))?;
        sent(n as u64);
    }
    stall(f.close()).await.map_err(plain)
}

/// Writes bytes to the device at `path` (replacing what's there).
pub async fn put(afc: &mut AfcClient, bytes: &[u8], path: &str) -> Result<(), String> {
    let _ = stall(afc.remove(path)).await;
    let mut f = stall(afc.open(path, AfcFopenMode::WrOnly)).await.map_err(|e| format!("{path} couldn't be written ({e})"))?;
    stall(f.write_entire(bytes)).await.map_err(|e| format!("{path} couldn't be written ({e})"))?;
    stall(f.close()).await.map_err(plain)
}

/// Makes a folder and the ones above it (errors ignored: most already exist).
pub async fn make_dirs(afc: &mut AfcClient, path: &str) {
    let mut at = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        at.push('/');
        at.push_str(part);
        let _ = stall(afc.mk_dir(at.clone())).await;
    }
}

pub async fn exists(afc: &mut AfcClient, path: &str) -> bool {
    stall(afc.get_file_info(path)).await.is_ok()
}

pub async fn remove(afc: &mut AfcClient, path: &str) -> Result<(), String> {
    stall(afc.remove(path)).await.map_err(plain)
}

pub async fn rename(afc: &mut AfcClient, from: &str, to: &str) -> Result<(), String> {
    stall(afc.rename(from, to)).await.map_err(plain)
}
