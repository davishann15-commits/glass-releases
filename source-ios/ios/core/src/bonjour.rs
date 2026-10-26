//! Bonjour for sharing on iOS, through Apple's DNS-SD API (dns_sd.h, part of
//! libSystem): announce this Glass, browse for the others, resolve their TXT
//! records, ports and addresses. The desktop uses mdns-sd, which opens its own
//! multicast sockets; an iOS app may not (that needs Apple's multicast entitlement),
//! but DNS-SD goes through the system's mDNSResponder, which only needs the
//! service type declared (NSBonjourServices) and the user's local-network permission.
//!
//! Callbacks run on a private serial dispatch queue; every DNSServiceRef is created,
//! served and deallocated there or under its lock. What's found goes out on a channel.

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_void};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

type DNSServiceRef = *mut c_void;
type Queue = *mut c_void;

type RegisterReply = extern "C" fn(DNSServiceRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type BrowseReply = extern "C" fn(DNSServiceRef, u32, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type ResolveReply = extern "C" fn(DNSServiceRef, u32, u32, i32, *const c_char, *const c_char, u16, u16, *const u8, *mut c_void);
type AddrReply = extern "C" fn(DNSServiceRef, u32, u32, i32, *const c_char, *const libc_sockaddr, u32, *mut c_void);

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct libc_sockaddr {
    sa_len: u8,
    sa_family: u8,
    sa_data: [u8; 14],
}

const FLAG_MORE_COMING: u32 = 0x1;
const FLAG_ADD: u32 = 0x2;
const PROTO_V4: u32 = 0x1;
const PROTO_V6: u32 = 0x2;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 30;
/// kDNSServiceErr_PolicyDenied: the user didn't allow Glass on the local network.
const POLICY_DENIED: i32 = -65570;

unsafe extern "C" {
    fn DNSServiceRegister(sd: *mut DNSServiceRef, flags: u32, iface: u32, name: *const c_char, regtype: *const c_char, domain: *const c_char, host: *const c_char, port: u16, txt_len: u16, txt: *const c_void, cb: RegisterReply, ctx: *mut c_void) -> i32;
    fn DNSServiceBrowse(sd: *mut DNSServiceRef, flags: u32, iface: u32, regtype: *const c_char, domain: *const c_char, cb: BrowseReply, ctx: *mut c_void) -> i32;
    fn DNSServiceResolve(sd: *mut DNSServiceRef, flags: u32, iface: u32, name: *const c_char, regtype: *const c_char, domain: *const c_char, cb: ResolveReply, ctx: *mut c_void) -> i32;
    fn DNSServiceGetAddrInfo(sd: *mut DNSServiceRef, flags: u32, iface: u32, protocol: u32, host: *const c_char, cb: AddrReply, ctx: *mut c_void) -> i32;
    fn DNSServiceSetDispatchQueue(sd: DNSServiceRef, q: Queue) -> i32;
    fn DNSServiceRefDeallocate(sd: DNSServiceRef);
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> Queue;
    fn dispatch_sync_f(q: Queue, ctx: *mut c_void, f: extern "C" fn(*mut c_void));
    fn dispatch_async_f(q: Queue, ctx: *mut c_void, f: extern "C" fn(*mut c_void));
    fn dispatch_release(o: *mut c_void);
}

/// What browsing found.
#[derive(Debug, Clone)]
pub enum Found {
    Service { instance: String, txt: HashMap<String, String>, port: u16, addrs: Vec<IpAddr> },
    Gone { instance: String },
    /// Local network access isn't allowed (Settings › Privacy & Security › Local Network).
    Denied,
}

/// A service being resolved: its TXT and port once resolved, then its addresses.
#[derive(Default)]
struct Pending {
    /// The lookups' shared context box (`Ctx`), freed with them.
    ctx: usize,
    /// The network interfaces it was found on (it's gone when it's left all of them).
    ifaces: HashSet<u32>,
    resolve: usize,
    addr: usize,
    txt: HashMap<String, String>,
    port: u16,
    addrs: Vec<IpAddr>,
}

struct Inner {
    queue: Queue,
    tx: tokio::sync::mpsc::UnboundedSender<Found>,
    regtype: CString,
    /// Refs as usize (raw pointers aren't Send); touched on the queue only.
    register: Mutex<usize>,
    browse: Mutex<usize>,
    pending: Mutex<HashMap<String, Pending>>,
    closed: std::sync::atomic::AtomicBool,
}

unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

/// Context for one service's resolve and address lookups.
struct Ctx {
    inner: Arc<Inner>,
    instance: String,
}

pub struct Bonjour {
    inner: Arc<Inner>,
    /// The browse callback's context (freed after the refs are gone).
    browse_ctx: *mut Arc<Inner>,
}

unsafe impl Send for Bonjour {}
unsafe impl Sync for Bonjour {}

/// "key=value" strings, each with a length byte.
fn txt_record(props: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in props {
        let s = format!("{k}={v}");
        let b = &s.as_bytes()[..s.len().min(255)];
        out.push(b.len() as u8);
        out.extend_from_slice(b);
    }
    out
}

fn parse_txt(bytes: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut i = 0;
    while i < bytes.len() {
        let n = bytes[i] as usize;
        i += 1;
        let Some(entry) = bytes.get(i..i + n) else { break };
        i += n;
        let s = String::from_utf8_lossy(entry);
        match s.split_once('=') {
            Some((k, v)) => map.insert(k.to_string(), v.to_string()),
            None => map.insert(s.to_string(), String::new()),
        };
    }
    map
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

extern "C" fn on_register(_: DNSServiceRef, _: u32, err: i32, _: *const c_char, _: *const c_char, _: *const c_char, _: *mut c_void) {
    if err != 0 {
        log::warn!("Bonjour: announcing failed ({err})");
    }
}

extern "C" fn on_browse(_: DNSServiceRef, flags: u32, iface: u32, err: i32, name: *const c_char, regtype: *const c_char, domain: *const c_char, ctx: *mut c_void) {
    // SAFETY: the browse context lives until the browse ref is deallocated.
    let inner = unsafe { &*(ctx as *const Arc<Inner>) }.clone();
    if err != 0 {
        log::warn!("Bonjour: browsing failed ({err})");
        if err == POLICY_DENIED {
            let _ = inner.tx.send(Found::Denied);
        }
        return;
    }
    let instance = cstr(name);
    if flags & FLAG_ADD == 0 {
        // Gone from this interface (Wi-Fi, say), maybe not from the others.
        let left = inner.pending.lock().unwrap().get_mut(&instance).is_some_and(|p| {
            p.ifaces.remove(&iface);
            !p.ifaces.is_empty()
        });
        if !left {
            stop_lookups(&inner, &instance);
            let _ = inner.tx.send(Found::Gone { instance });
        }
        return;
    }
    if let Some(p) = inner.pending.lock().unwrap().get_mut(&instance) {
        p.ifaces.insert(iface);
        return;
    }
    let c = Box::into_raw(Box::new(Ctx { inner: inner.clone(), instance: instance.clone() }));
    let mut sd: DNSServiceRef = std::ptr::null_mut();
    // Resolved on any interface, not just the one it was first seen on: that can be
    // one its addresses never come from (a VPN's, say), and nothing would be found.
    // SAFETY: valid C strings from the browse reply; the context box outlives the ref.
    let e = unsafe { DNSServiceResolve(&mut sd, 0, 0, name, regtype, domain, on_resolve, c as *mut c_void) };
    if e != 0 || sd.is_null() {
        // SAFETY: not handed to any ref.
        drop(unsafe { Box::from_raw(c) });
        return;
    }
    unsafe { DNSServiceSetDispatchQueue(sd, inner.queue) };
    inner.pending.lock().unwrap().insert(instance, Pending { ctx: c as usize, ifaces: HashSet::from([iface]), resolve: sd as usize, ..Default::default() });
}

extern "C" fn on_resolve(sd: DNSServiceRef, _: u32, _: u32, err: i32, _: *const c_char, host: *const c_char, port: u16, txt_len: u16, txt: *const u8, ctx: *mut c_void) {
    // SAFETY: the context box lives until its refs are deallocated (stop_lookups).
    let c = unsafe { &*(ctx as *const Ctx) };
    if err != 0 {
        return;
    }
    let txt = if txt.is_null() { HashMap::new() } else { parse_txt(unsafe { std::slice::from_raw_parts(txt, txt_len as usize) }) };
    let mut addr: DNSServiceRef = std::ptr::null_mut();
    // SAFETY: host is a valid C string; same context.
    let e = unsafe { DNSServiceGetAddrInfo(&mut addr, 0, 0, PROTO_V4 | PROTO_V6, host, on_addr, ctx) };
    let mut pending = c.inner.pending.lock().unwrap();
    if let Some(p) = pending.get_mut(&c.instance) {
        p.txt = txt;
        p.port = u16::from_be(port);
        if e == 0 && !addr.is_null() {
            unsafe { DNSServiceSetDispatchQueue(addr, c.inner.queue) };
            p.addr = addr as usize;
        }
        // The resolve is done; deallocating a ref inside its own callback is allowed.
        if p.resolve == sd as usize {
            unsafe { DNSServiceRefDeallocate(sd) };
            p.resolve = 0;
        }
    }
}

extern "C" fn on_addr(_: DNSServiceRef, flags: u32, _: u32, err: i32, _: *const c_char, addr: *const libc_sockaddr, _: u32, ctx: *mut c_void) {
    // SAFETY: as above.
    let c = unsafe { &*(ctx as *const Ctx) };
    if err != 0 || addr.is_null() {
        return;
    }
    let ip = unsafe {
        match (*addr).sa_family {
            AF_INET => {
                let a = addr as *const u8;
                let b = std::slice::from_raw_parts(a.add(4), 4);
                Some(IpAddr::from([b[0], b[1], b[2], b[3]]))
            }
            AF_INET6 => {
                let a = addr as *const u8;
                let b: [u8; 16] = std::slice::from_raw_parts(a.add(8), 16).try_into().unwrap_or([0; 16]);
                Some(IpAddr::from(b))
            }
            _ => None,
        }
    };
    let mut pending = c.inner.pending.lock().unwrap();
    let Some(p) = pending.get_mut(&c.instance) else { return };
    if let Some(ip) = ip
        && flags & FLAG_ADD != 0
        && !p.addrs.contains(&ip)
    {
        p.addrs.push(ip);
    }
    if flags & FLAG_MORE_COMING == 0 && !p.addrs.is_empty() {
        let mut addrs = p.addrs.clone();
        addrs.sort();
        let _ = c.inner.tx.send(Found::Service { instance: c.instance.clone(), txt: p.txt.clone(), port: p.port, addrs });
    }
}

/// Stops a service's lookups (on the queue) and frees their context.
fn stop_lookups(inner: &Arc<Inner>, instance: &str) {
    let Some(p) = inner.pending.lock().unwrap().remove(instance) else { return };
    free_pending(p);
}

/// Deallocates a service's lookups, then their shared context box.
fn free_pending(p: Pending) {
    for r in [p.resolve, p.addr] {
        if r != 0 {
            unsafe { DNSServiceRefDeallocate(r as DNSServiceRef) };
        }
    }
    if p.ctx != 0 {
        // SAFETY: no ref uses it any more.
        drop(unsafe { Box::from_raw(p.ctx as *mut Ctx) });
    }
}

impl Bonjour {
    /// Announces `instance` of `regtype` ("_glass-files._tcp") on `port` with `props` as
    /// its TXT record, and browses for the others.
    pub fn start(regtype: &str, instance: &str, port: u16, props: &[(&str, String)]) -> anyhow::Result<(Self, tokio::sync::mpsc::UnboundedReceiver<Found>)> {
        Self::open(regtype, Some((instance, port, props)))
    }

    /// Only browses for `regtype` (announcing nothing).
    pub fn browse(regtype: &str) -> anyhow::Result<(Self, tokio::sync::mpsc::UnboundedReceiver<Found>)> {
        Self::open(regtype, None)
    }

    fn open(regtype: &str, announce: Option<(&str, u16, &[(&str, String)])>) -> anyhow::Result<(Self, tokio::sync::mpsc::UnboundedReceiver<Found>)> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let label = CString::new("app.glass.bonjour")?;
        // SAFETY: a serial queue (null attributes), released in Drop.
        let queue = unsafe { dispatch_queue_create(label.as_ptr(), std::ptr::null_mut()) };
        let inner = Arc::new(Inner {
            queue,
            tx,
            regtype: CString::new(regtype)?,
            register: Mutex::new(0),
            browse: Mutex::new(0),
            pending: Mutex::new(HashMap::new()),
            closed: Default::default(),
        });
        if let Some((instance, port, props)) = announce {
            Self::announce(&inner, instance, port, props)?;
        }
        let browse_ctx = Box::into_raw(Box::new(inner.clone()));
        let mut br: DNSServiceRef = std::ptr::null_mut();
        // SAFETY: the context box is freed in Drop after the browse ref.
        let e = unsafe { DNSServiceBrowse(&mut br, 0, 0, inner.regtype.as_ptr(), std::ptr::null(), on_browse, browse_ctx as *mut c_void) };
        if e != 0 || br.is_null() {
            let b = Self { inner, browse_ctx };
            drop(b);
            anyhow::bail!("couldn't look for other devices (DNS-SD error {e})");
        }
        unsafe { DNSServiceSetDispatchQueue(br, queue) };
        *inner.browse.lock().unwrap() = br as usize;
        Ok((Self { inner, browse_ctx }, rx))
    }

    fn announce(inner: &Arc<Inner>, instance: &str, port: u16, props: &[(&str, String)]) -> anyhow::Result<()> {
        let queue = inner.queue;
        let name = CString::new(instance)?;
        let txt = txt_record(props);
        let mut reg: DNSServiceRef = std::ptr::null_mut();
        // SAFETY: valid strings and TXT bytes for the call; the register reply needs no context.
        let e = unsafe { DNSServiceRegister(&mut reg, 0, 0, name.as_ptr(), inner.regtype.as_ptr(), std::ptr::null(), std::ptr::null(), port.to_be(), txt.len() as u16, txt.as_ptr() as *const c_void, on_register, std::ptr::null_mut()) };
        if e != 0 || reg.is_null() {
            unsafe { dispatch_release(queue) };
            anyhow::bail!("couldn't announce on the local network (DNS-SD error {e}); is local network access allowed for Glass?");
        }
        unsafe { DNSServiceSetDispatchQueue(reg, queue) };
        *inner.register.lock().unwrap() = reg as usize;
        Ok(())
    }
}

extern "C" fn teardown(ctx: *mut c_void) {
    // SAFETY: `ctx` is the Bonjour's Inner, alive for this synchronous call.
    let inner = unsafe { &*(ctx as *const Inner) };
    inner.closed.store(true, std::sync::atomic::Ordering::Relaxed);
    for r in [&inner.register, &inner.browse] {
        let mut g = r.lock().unwrap();
        if *g != 0 {
            unsafe { DNSServiceRefDeallocate(*g as DNSServiceRef) };
            *g = 0;
        }
    }
    let all: Vec<Pending> = inner.pending.lock().unwrap().drain().map(|(_, p)| p).collect();
    for p in all {
        free_pending(p);
    }
}

extern "C" fn release_queue(ctx: *mut c_void) {
    unsafe { dispatch_release(ctx) };
}

impl Drop for Bonjour {
    fn drop(&mut self) {
        // Every ref is torn down on the queue (no callback can be running meanwhile).
        unsafe { dispatch_sync_f(self.inner.queue, Arc::as_ptr(&self.inner) as *mut c_void, teardown) };
        // SAFETY: the browse ref is gone; nothing uses its context now.
        drop(unsafe { Box::from_raw(self.browse_ctx) });
        unsafe { dispatch_async_f(self.inner.queue, self.inner.queue, release_queue) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_records_round_trip() {
        let t = txt_record(&[("v", "3".into()), ("acct", "abc.def".into()), ("name", "Davis's iPhone".into())]);
        let m = parse_txt(&t);
        assert_eq!(m["v"], "3");
        assert_eq!(m["acct"], "abc.def");
        assert_eq!(m["name"], "Davis's iPhone");
    }
}
