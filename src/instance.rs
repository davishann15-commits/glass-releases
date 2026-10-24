//! One Glass at a time. A second copy started while Glass runs (a rebuilt
//! or newly downloaded app, or the same program started again) talks to the
//! running one over a Unix socket in Glass's cache folder:
//!
//! - the same build, or an older version: the running window comes forward
//!   and the new copy exits (an old copy left in Downloads never replaces an
//!   updated Glass);
//! - a different build of the same or a newer version: the running Glass saves
//!   and quits, and the new one takes its place (an update never leaves two
//!   Glasses side by side).
//!
//! Only the user can reach the socket; it answers just these two requests.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    Focus,
    Quit,
}

pub enum Claim {
    /// Run, serving later copies on this listener (if one could be made).
    #[cfg(unix)]
    Run(Option<std::os::unix::net::UnixListener>),
    #[cfg(not(unix))]
    Run(Option<()>),
    /// The same build, or a newer one, already runs and was brought forward.
    Exit,
}

#[cfg(unix)]
mod imp {
    use super::{Claim, Request};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant, UNIX_EPOCH};

    static IDENTITY: OnceLock<String> = OnceLock::new();

    pub fn socket() -> PathBuf {
        crate::config::cache_dir().join("glass.sock")
    }

    /// This build: its version, and the program's path, size and modification
    /// time, taken at startup (a rebuild replaces the file under a running Glass).
    fn identity() -> &'static str {
        IDENTITY.get_or_init(|| {
            let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).unwrap_or_default();
            let m = std::fs::metadata(&exe).ok();
            let modified = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            format!("{}|{}|{}|{modified}", crate::update::VERSION, exe.display(), m.map(|m| m.len()).unwrap_or(0))
        })
    }

    /// Whether the copy that sent `them` is an older version than this one.
    fn older(them: &str, me: &str) -> bool {
        let version = |id: &str| id.split('|').next().and_then(|v| semver::Version::parse(v).ok());
        matches!((version(them), version(me)), (Some(a), Some(b)) if a < b)
    }

    pub fn claim_at(path: &Path, me: &str) -> Claim {
        if let Ok(mut s) = UnixStream::connect(path) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
            let mut reply = String::new();
            if writeln!(s, "glass1 {me}").is_ok() {
                let _ = BufReader::new(&s).read_line(&mut reply);
            }
            match reply.trim() {
                "focus" => return Claim::Exit,
                "quit" => {
                    // Wait for it to save and go (it closes the socket on the way out).
                    let until = Instant::now() + Duration::from_secs(10);
                    while Instant::now() < until && UnixStream::connect(path).is_ok() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                _ => {}
            }
        }
        // Nobody answered (no Glass, or a crashed one's socket), or the old one made way: take the socket.
        let _ = std::fs::remove_file(path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let listener = UnixListener::bind(path).ok();
        if listener.is_some() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Claim::Run(listener)
    }

    pub fn claim() -> Claim {
        claim_at(&socket(), identity())
    }

    pub fn serve_as(listener: UnixListener, me: String, on: impl Fn(Request) + Send + 'static) {
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut s) = conn else { continue };
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(&s).read_line(&mut line).is_err() {
                    continue;
                }
                let Some(them) = line.trim().strip_prefix("glass1 ") else { continue };
                let req = if them == me || older(them, &me) { Request::Focus } else { Request::Quit };
                let _ = writeln!(s, "{}", if req == Request::Focus { "focus" } else { "quit" });
                drop(s);
                on(req);
                if req == Request::Quit {
                    // Should the window be unable to close (it saves on the
                    // way out), make way for the new Glass anyway; but never in
                    // the middle of installing an update.
                    std::thread::sleep(Duration::from_secs(6));
                    while crate::update::INSTALLING.load(std::sync::atomic::Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    std::process::exit(0);
                }
            }
        });
    }

    pub fn serve(listener: UnixListener, on: impl Fn(Request) + Send + 'static) {
        serve_as(listener, identity().to_string(), on);
    }
}

/// Becomes the running Glass, or says to exit because this same build (or a
/// newer one) already runs and has been brought forward. Demo mode runs alongside
/// (unless `GLASS_ONE_INSTANCE` is set, to try this out).
pub fn claim() -> Claim {
    #[cfg(unix)]
    if std::env::var_os("GLASS_DEMO").is_none() || std::env::var_os("GLASS_ONE_INSTANCE").is_some() {
        return imp::claim();
    }
    Claim::Run(None)
}

/// Answers later copies of Glass; `on` runs for each request.
#[cfg(unix)]
pub fn serve(listener: std::os::unix::net::UnixListener, on: impl Fn(Request) + Send + 'static) {
    imp::serve(listener, on);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn the_same_build_focuses_and_a_stale_socket_is_replaced() {
        let path = crate::config::test_dir("inst.sock");
        let _ = std::fs::remove_file(&path);
        let Claim::Run(Some(listener)) = imp::claim_at(&path, "build-a") else { panic!("first copy runs") };
        let (tx, rx) = mpsc::channel();
        imp::serve_as(listener, "build-a".into(), move |r| {
            let _ = tx.send(r);
        });
        assert!(matches!(imp::claim_at(&path, "build-a"), Claim::Exit), "the same build just comes forward");
        assert_eq!(rx.recv().unwrap(), Request::Focus);
        // A stale socket (a crashed Glass) doesn't stop a new one.
        let stale = crate::config::test_dir("inst-stale.sock");
        let _ = std::fs::remove_file(&stale);
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        assert!(matches!(imp::claim_at(&stale, "x"), Claim::Run(Some(_))));
        let _ = std::fs::remove_file(&stale);
        let _ = std::fs::remove_file(&path);
    }
}
