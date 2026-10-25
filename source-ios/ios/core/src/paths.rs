//! The folders iOS gives the app, set once at start (see `GlassCore::new`).
//! `config::config_dir()` and `cache_dir()` already resolve inside the app's
//! container on iOS (Library/Application Support/Glass, Library/Caches/Glass).

use std::path::PathBuf;
use std::sync::OnceLock;

static DOCUMENTS: OnceLock<PathBuf> = OnceLock::new();

pub fn install(c: &crate::bridge::CoreConfig) {
    let _ = DOCUMENTS.set(PathBuf::from(&c.documents_dir));
    let _ = std::fs::create_dir_all(&c.data_dir);
    let _ = std::fs::create_dir_all(&c.cache_dir);
    move_old_config();
}

/// theme.toml and config.toml used to be in ~/.config/glass, which an iPhone doesn't
/// let the app write (the simulator does): any made there move to their place now.
fn move_old_config() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let old = PathBuf::from(home).join(".config/glass");
    let new = crate::theme_config::config_dir();
    if old == new || !old.is_dir() {
        return;
    }
    let _ = std::fs::create_dir_all(&new);
    for name in ["theme.toml", "config.toml"] {
        let (from, to) = (old.join(name), new.join(name));
        if from.is_file() && !to.exists() && std::fs::copy(&from, &to).is_ok() {
            let _ = std::fs::remove_file(&from);
        }
    }
    // Only if empty: anything else someone put there stays.
    let _ = std::fs::remove_dir(&old);
    if let Some(parent) = old.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

/// Documents: "On My iPhone › Glass" in the Files app.
pub fn documents() -> PathBuf {
    DOCUMENTS.get().cloned().unwrap_or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents")).unwrap_or_else(|| PathBuf::from("Documents")))
}

/// A path in the app's Documents as it is this run. iOS moves the app's container when
/// the app is installed again or updated (the folder's name changes), and gives the
/// same folder as /var/… or /private/var/… depending on who asks; paths kept from
/// another run, or in the other spelling, would match no file. Anything else is
/// returned as it is.
pub fn rebase(p: &std::path::Path) -> PathBuf {
    rebase_onto(p, &documents())
}

fn rebase_onto(p: &std::path::Path, documents: &std::path::Path) -> PathBuf {
    const APP: &str = "/Containers/Data/Application/";
    let s = p.to_string_lossy();
    let Some(at) = s.find(APP).map(|i| i + APP.len()) else { return p.to_path_buf() };
    // "<container ID>/Documents", then the rest.
    let Some(end) = s[at..].find('/').map(|i| at + i) else { return p.to_path_buf() };
    let rest = &s[end..];
    match rest.strip_prefix("/Documents") {
        Some(inside) if inside.is_empty() || inside.starts_with('/') => PathBuf::from(format!("{}{inside}", documents.display())),
        _ => p.to_path_buf(),
    }
}

/// A path inside the app's container as "~/…" (the container is the app's home).
pub fn tilde(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn paths_follow_the_container() {
        let docs = Path::new("/var/mobile/Containers/Data/Application/NEW-ID/Documents");
        let old = Path::new("/var/mobile/Containers/Data/Application/OLD-ID/Documents/Yacht Rock/Styx - Babe.m4a");
        assert_eq!(rebase_onto(old, docs), docs.join("Yacht Rock/Styx - Babe.m4a"), "an earlier install's container");
        let private = Path::new("/private/var/mobile/Containers/Data/Application/NEW-ID/Documents/Yacht Rock/Styx - Babe.m4a");
        assert_eq!(rebase_onto(private, docs), docs.join("Yacht Rock/Styx - Babe.m4a"), "the other spelling");
        assert_eq!(rebase_onto(Path::new("/private/var/mobile/Containers/Data/Application/OLD-ID/Documents"), docs), docs);
        for other in ["/private/var/mobile/Library/Mobile Documents/com~apple~CloudDocs/Music", "/var/mobile/Containers/Data/Application/ID/Library/Caches/x", "/var/mobile/Containers/Data/Application/ID/DocumentsX/y", "/Users/me/Music"] {
            assert_eq!(rebase_onto(Path::new(other), docs), Path::new(other), "{other}");
        }
        let sim = Path::new("/Users/me/Library/Developer/CoreSimulator/Devices/D/data/Containers/Data/Application/OLD/Documents/a.m4a");
        assert_eq!(rebase_onto(sim, docs), docs.join("a.m4a"));
    }
}
