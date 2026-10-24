//! Signs Glass releases. The signing key lives in the OS credential store
//! (macOS Keychain), never in the repository; the app carries only the
//! public key and installs nothing that isn't signed with this one.
//!
//!   glass-release keygen                       make the key (once)
//!   glass-release pubkey                       the public key, for src/update.rs
//!   glass-release sign <version> <target> <file>
//!                                              prints `sha256 signature` for the manifest

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

const SERVICE: &str = "Glass release signing";
const ACCOUNT: &str = "ed25519";

fn entry() -> keyring::Entry {
    keyring::Entry::new(SERVICE, ACCOUNT).expect("credential store")
}

fn key() -> SigningKey {
    let secret = match entry().get_password() {
        Ok(s) => s,
        Err(keyring::Error::NoEntry) => fail("no signing key yet: run `glass-release keygen`"),
        // A locked Keychain or a denied prompt isn't a missing key: don't suggest making one.
        Err(e) => fail(&format!("couldn't read the signing key from the Keychain: {e}")),
    };
    let bytes: [u8; 32] = B64.decode(secret.trim()).ok().and_then(|b| b.try_into().ok()).unwrap_or_else(|| fail("the stored signing key is damaged"));
    SigningKey::from_bytes(&bytes)
}

fn fail(msg: &str) -> ! {
    eprintln!("glass-release: {msg}");
    std::process::exit(1)
}

/// What a signature covers: the version and target are part of it, so an
/// old release can't be passed off as a newer one.
fn message(version: &str, target: &str, sha256: &str) -> String {
    format!("glass-update v1\n{version}\n{target}\n{sha256}")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen"] => {
            // Only when there's certainly none: replacing the key would stop installed copies
            // from updating.
            match entry().get_password() {
                Err(keyring::Error::NoEntry) => {}
                Ok(_) => fail("a signing key already exists (replacing it would stop installed copies from updating)"),
                Err(e) => fail(&format!("couldn't check the Keychain for a signing key ({e}); nothing was changed")),
            }
            let k = SigningKey::generate(&mut rand::rngs::OsRng);
            entry().set_password(&B64.encode(k.to_bytes())).unwrap_or_else(|e| fail(&format!("couldn't store the key: {e}")));
            println!("Signing key saved in the Keychain. Its public key (put it in PUBLIC_KEY in src/update.rs):");
            println!("{}", B64.encode(k.verifying_key().to_bytes()));
        }
        ["pubkey"] => println!("{}", B64.encode(key().verifying_key().to_bytes())),
        ["sign", version, target, file] => {
            let data = std::fs::read(file).unwrap_or_else(|e| fail(&format!("{file}: {e}")));
            let sha: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
            let sig = key().sign(message(version, target, &sha).as_bytes());
            println!("{sha} {}", B64.encode(sig.to_bytes()));
        }
        _ => fail("usage: glass-release keygen | pubkey | sign <version> <target> <file>"),
    }
}
