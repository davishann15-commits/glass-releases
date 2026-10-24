//! Updates. Glass checks a public feed (the latest release of
//! github.com/davishann15-commits/glass-releases), downloads a newer version
//! in the background, makes sure it was signed with the release key (kept in
//! the developer's Keychain; see tools/glass-release), and installs it when
//! Glass quits, or at once from "Restart to update".
//!
//! Only builds made for release update themselves (scripts/package-macos.sh
//! sets `GLASS_RELEASE`, as the Arch package does). Linux packages are updated
//! by building the new package; Glass only says when one is out. Development
//! builds don't check by themselves; "Check now" says when a new version is out.

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FEED: &str = "https://github.com/davishann15-commits/glass-releases/releases/latest/download/glass-update.json";
/// The release key's public half (tools/glass-release pubkey).
const PUBLIC_KEY: &str = "1SbgbPIx4TS7FBRV7fm1hytnlLuulL8vNszq9gE2Q+c=";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// A build made for release, which installs its own updates.
pub const RELEASE: bool = option_env!("GLASS_RELEASE").is_some();
/// Which download in a release is for this computer.
pub const TARGET: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_arch = "aarch64") {
    "linux-aarch64"
} else {
    "linux-x86_64"
};
/// The largest update accepted.
const MAX_SIZE: u64 = 512 << 20;

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub assets: HashMap<String, Asset>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Asset {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub signature: String,
}

/// Whether `candidate` is a later version than `current` (1.0.0-beta.10
/// comes after 1.0.0-beta.2, and 1.0.0 after both).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (semver::Version::parse(candidate.trim_start_matches('v')), semver::Version::parse(current)) {
        (Ok(c), Ok(n)) => c > n,
        _ => false,
    }
}

/// What a release signature covers (the same as tools/glass-release).
fn message(version: &str, target: &str, sha256: &str) -> String {
    format!("glass-update v1\n{version}\n{target}\n{sha256}")
}

fn verify_with(key: &str, version: &str, target: &str, sha256: &str, signature: &str) -> bool {
    let Ok(Ok(k)) = B64.decode(key).map(|b| <[u8; 32]>::try_from(b.as_slice()).map_err(|_| ())) else { return false };
    let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&k) else { return false };
    let Ok(Ok(sig)) = B64.decode(signature).map(|b| ed25519_dalek::Signature::from_slice(&b)) else { return false };
    vk.verify_strict(message(version, target, sha256).as_bytes(), &sig).is_ok()
}

/// The feed: `GLASS_UPDATE_FEED` to try out a release before publishing it
/// (whatever it points to still has to be signed with the release key).
pub fn feed() -> String {
    std::env::var("GLASS_UPDATE_FEED").unwrap_or_else(|_| FEED.to_string())
}

/// Set while the app is being replaced, so nothing exits halfway through.
pub static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A newer version for this computer, if there is one.
pub async fn check(http: &reqwest::Client) -> Result<Option<Manifest>> {
    let clean = |e: reqwest::Error| anyhow!("{}", e.without_url());
    let m: Manifest = http.get(feed()).timeout(std::time::Duration::from_secs(30)).send().await.map_err(clean)?.error_for_status().map_err(clean)?.json().await.map_err(clean)?;
    // macOS installs the update itself, so it needs this computer's file; elsewhere
    // Glass only says a new version is out.
    let usable = m.assets.contains_key(TARGET) || !cfg!(target_os = "macos");
    Ok((is_newer(&m.version, VERSION) && usable).then_some(m))
}

/// Downloads this computer's update into `dir`, checking its size, hash and
/// signature on the way. Returns the file.
pub async fn download(http: &reqwest::Client, m: &Manifest, dir: &Path) -> Result<PathBuf> {
    download_with(http, m, dir, PUBLIC_KEY).await
}

async fn download_with(http: &reqwest::Client, m: &Manifest, dir: &Path, key: &str) -> Result<PathBuf> {
    let a = m.assets.get(TARGET).context("no download for this computer")?;
    // The signature first: nothing unsigned is even downloaded.
    if !verify_with(key, &m.version, TARGET, &a.sha256, &a.signature) {
        bail!("the update isn't signed with Glass's release key");
    }
    if a.size == 0 || a.size > MAX_SIZE {
        bail!("the update's size is wrong");
    }
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(format!("Glass-{}-{TARGET}.zip", m.version));
    if file_sha256(&dest).is_some_and(|h| h == a.sha256) {
        return Ok(dest);
    }
    // Older downloads (and broken ones) go: only this version is wanted now.
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("Glass-") && (name.ends_with(".zip") || name.ends_with(".part")) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let part = dest.with_extension("part");
    let r = fetch_into(http, &a.url, &part, a.size).await;
    match r {
        Ok(sha256) if sha256 == a.sha256 => {
            std::fs::rename(&part, &dest)?;
            Ok(dest)
        }
        other => {
            let _ = std::fs::remove_file(&part);
            other.and_then(|_| Err(anyhow!("the update didn't download intact")))
        }
    }
}

/// Downloads `url` into `path` (exactly `size` bytes); returns its SHA-256.
async fn fetch_into(http: &reqwest::Client, url: &str, path: &Path, size: u64) -> Result<String> {
    let clean = |e: reqwest::Error| anyhow!("{}", e.without_url());
    let mut res = http.get(url).send().await.map_err(clean)?.error_for_status().map_err(clean)?;
    let mut file = std::fs::File::create(path)?;
    let mut hash = Sha256::new();
    let mut n = 0u64;
    while let Some(chunk) = res.chunk().await.map_err(clean)? {
        n += chunk.len() as u64;
        if n > size {
            bail!("the update is bigger than it should be");
        }
        hash.update(&chunk);
        file.write_all(&chunk)?;
    }
    if n != size {
        bail!("the update didn't download intact");
    }
    Ok(hex(&hash.finalize()))
}

/// A file's SHA-256, read in pieces.
fn file_sha256(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    std::io::copy(&mut f, &mut hash).ok()?;
    Some(hex(&hash.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The .app this Glass runs from (macOS).
pub fn current_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?.to_path_buf();
    (bundle.extension().is_some_and(|e| e == "app")).then_some(bundle)
}

/// Whether Glass can replace itself where it is (it needs to write there).
pub fn can_install() -> bool {
    current_bundle().and_then(|b| b.parent().map(Path::to_path_buf)).is_some_and(|d| {
        let probe = d.join(".glass-write-test");
        let ok = std::fs::write(&probe, b"").is_ok();
        let _ = std::fs::remove_file(probe);
        ok
    })
}

fn plist_value(app: &Path, key: &str) -> Option<String> {
    let o = std::process::Command::new("plutil").args(["-extract", key, "raw", "-o", "-"]).arg(app.join("Contents/Info.plist")).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Puts the downloaded `version` in place of the running app. Returns the
/// app's path. The running Glass carries on until it quits.
pub fn install(zip: &Path, version: &str) -> Result<PathBuf> {
    let bundle = current_bundle().context("Glass isn't running from an app")?;
    install_into(zip, version, &bundle)?;
    Ok(bundle)
}

fn install_into(zip: &Path, version: &str, bundle: &Path) -> Result<()> {
    INSTALLING.store(true, std::sync::atomic::Ordering::Release);
    let r = replace(zip, version, bundle);
    INSTALLING.store(false, std::sync::atomic::Ordering::Release);
    r
}

fn replace(zip: &Path, version: &str, bundle: &Path) -> Result<()> {
    let dir = bundle.parent().context("no folder")?;
    let staging = dir.join(".Glass-update");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).context("Glass can't update itself in this folder (move it to Applications)")?;
    let ok = std::process::Command::new("ditto").args(["-x", "-k"]).arg(zip).arg(&staging).status().is_ok_and(|s| s.success());
    let fresh = staging.join("Glass.app");
    let right = ok && plist_value(&fresh, "CFBundleIdentifier").as_deref() == Some("app.glass.player") && plist_value(&fresh, "CFBundleShortVersionString").as_deref() == Some(version);
    if !right {
        let _ = std::fs::remove_dir_all(&staging);
        bail!("the update isn't the Glass {version} it should be");
    }
    // One atomic swap where the file system can (APFS): there's never a moment without Glass.app.
    #[cfg(target_os = "macos")]
    if swap(&fresh, bundle).is_ok() {
        // The staging folder now holds the old app.
        let _ = std::fs::remove_dir_all(&staging);
        return Ok(());
    }
    // Otherwise two renames, putting the old one back if anything goes wrong.
    let old = dir.join(".Glass-old.app");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(bundle, &old).context("Glass can't update itself in this folder (move it to Applications)")?;
    if let Err(e) = std::fs::rename(&fresh, bundle) {
        let _ = std::fs::rename(&old, bundle);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// Exchanges two paths in one step (`renamex_np` with `RENAME_SWAP`).
#[cfg(target_os = "macos")]
fn swap(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::ffi::{CString, c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }
    const RENAME_SWAP: c_uint = 0x2;
    let (a, b) = (CString::new(a.as_os_str().as_bytes())?, CString::new(b.as_os_str().as_bytes())?);
    // SAFETY: two valid, NUL-terminated paths that outlive the call; it only reads them.
    if unsafe { renamex_np(a.as_ptr(), b.as_ptr(), RENAME_SWAP) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Opens the updated app; it takes over from this one (see instance.rs).
pub fn relaunch(bundle: &Path) {
    let _ = std::process::Command::new("open").arg("-n").arg(bundle).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    fn signed(version: &str, target: &str, data: &[u8]) -> (String, Asset) {
        let k = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let sha = hex(&Sha256::digest(data));
        let sig = B64.encode(k.sign(message(version, target, &sha).as_bytes()).to_bytes());
        (B64.encode(k.verifying_key().to_bytes()), Asset { url: String::new(), size: data.len() as u64, sha256: sha, signature: sig })
    }

    #[test]
    fn versions_compare_like_releases() {
        assert!(is_newer("1.0.0-beta.3", "1.0.0-beta.2"));
        assert!(is_newer("1.0.0-beta.10", "1.0.0-beta.9"), "numbers, not letters");
        assert!(is_newer("1.0.0", "1.0.0-beta.10"));
        assert!(is_newer("v1.1.0", "1.0.0"));
        assert!(!is_newer("1.0.0-beta.2", "1.0.0-beta.2"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    #[test]
    fn only_signed_updates_for_their_version_pass() {
        let (key, a) = signed("1.0.0-beta.3", "macos", b"glass");
        assert!(verify_with(&key, "1.0.0-beta.3", "macos", &a.sha256, &a.signature));
        assert!(!verify_with(&key, "1.0.0-beta.9", "macos", &a.sha256, &a.signature), "an old build passed off as a new one");
        assert!(!verify_with(&key, "1.0.0-beta.3", "linux-x86_64", &a.sha256, &a.signature), "another platform's");
        assert!(!verify_with(&key, "1.0.0-beta.3", "macos", &hex(&Sha256::digest(b"evil")), &a.signature), "other contents");
        assert!(!verify_with(PUBLIC_KEY, "1.0.0-beta.3", "macos", &a.sha256, &a.signature), "someone else's key");
    }

    #[tokio::test]
    async fn downloads_are_checked_before_they_count() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let data = b"PK fake zip of Glass".to_vec();
        let serve = |body: Vec<u8>| async move {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = l.local_addr().unwrap();
            tokio::spawn(async move {
                let (mut s, _) = l.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                r.extend(body);
                let _ = s.write_all(&r).await;
            });
            format!("http://{addr}/u.zip")
        };
        let dir = crate::config::test_dir("update");
        let _ = std::fs::remove_dir_all(&dir);
        let http = reqwest::Client::new();
        let (key, mut a) = signed("9.0.0", TARGET, &data);
        a.url = serve(data.clone()).await;
        let m = Manifest { version: "9.0.0".into(), notes: String::new(), assets: [(TARGET.to_string(), a.clone())].into() };
        let got = download_with(&http, &m, &dir, &key).await.unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), data);
        // Same signature, different bytes served: refused, and nothing kept.
        let _ = std::fs::remove_dir_all(&dir);
        let mut b = a.clone();
        b.url = serve(b"PK something else!!!".to_vec()).await;
        let m2 = Manifest { assets: [(TARGET.to_string(), b)].into(), ..m.clone() };
        assert!(download_with(&http, &m2, &dir, &key).await.is_err());
        assert!(std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) == 0);
        // Unsigned: not even downloaded.
        let m3 = Manifest { assets: [(TARGET.to_string(), Asset { signature: B64.encode([0u8; 64]), ..a })].into(), ..m };
        assert!(download_with(&http, &m3, &dir, &key).await.unwrap_err().to_string().contains("signed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_update_replaces_the_app_and_only_the_right_one() {
        let dir = crate::config::test_dir("install");
        let _ = std::fs::remove_dir_all(&dir);
        let make = |root: &Path, version: &str, id: &str| {
            let app = root.join("Glass.app/Contents");
            std::fs::create_dir_all(app.join("MacOS")).unwrap();
            std::fs::write(app.join("MacOS/glass"), version).unwrap();
            std::fs::write(app.join("Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"#)).unwrap();
        };
        let zip_of = |root: &Path, name: &str| {
            let z = dir.join(name);
            assert!(std::process::Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(root.join("Glass.app")).arg(&z).status().unwrap().success());
            z
        };
        let installed = dir.join("Applications");
        make(&installed, "1.0.0", "app.glass.player");
        let new = dir.join("new");
        make(&new, "1.1.0", "app.glass.player");
        let zip = zip_of(&new, "u.zip");
        let bundle = installed.join("Glass.app");
        assert!(install_into(&zip, "1.2.0", &bundle).is_err(), "not the version it claims");
        assert_eq!(std::fs::read_to_string(bundle.join("Contents/MacOS/glass")).unwrap(), "1.0.0", "untouched");
        install_into(&zip, "1.1.0", &bundle).unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("Contents/MacOS/glass")).unwrap(), "1.1.0");
        assert!(!installed.join(".Glass-old.app").exists() && !installed.join(".Glass-update").exists(), "tidy afterwards");
        let other = dir.join("other");
        make(&other, "1.3.0", "com.example.notglass");
        assert!(install_into(&zip_of(&other, "o.zip"), "1.3.0", &bundle).is_err(), "another app");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
