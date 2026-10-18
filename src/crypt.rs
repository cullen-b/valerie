//! The sync passphrase. Tabs, chats and passwords are encrypted before they go into the sync repo; only machines
//! that know the passphrase can read them. The repo holds `sync-key.json` (salt, key-derivation settings and a check
//! value, no secrets); each machine keeps the derived key in its Keychain, so the passphrase is typed once per machine.
use anyhow::{bail, Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const KEY_FILE: &str = "sync-key.json";
const MAGIC: &[u8; 4] = b"VLR1";
const CHECK: &[u8] = b"valerie sync key";
#[cfg_attr(test, allow(dead_code))]
const KEYCHAIN: &str = "Valerie sync key";

pub type Key = [u8; 32];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFile {
    pub version: u32,
    pub kdf: String,
    /// Argon2id memory in KiB, passes and lanes.
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
    pub salt: String,
    /// CHECK sealed with the key: tells a wrong passphrase from a right one.
    pub check: String,
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("the system's random number generator");
    b
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("bad hex");
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("bad hex")).collect()
}

fn derive(passphrase: &str, salt: &[u8], m_kib: u32, t: u32, p: u32) -> Result<Key> {
    let params = argon2::Params::new(m_kib, t, p, Some(32)).map_err(|e| anyhow::anyhow!("{e}"))?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    a.hash_password_into(passphrase.trim().as_bytes(), salt, &mut key).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(key)
}

/// A new key file for `passphrase`, and the key it unlocks.
pub fn new_key_file(passphrase: &str) -> Result<(KeyFile, Key)> {
    if passphrase.trim().chars().count() < 12 {
        bail!("use at least 12 characters (a few words is easiest)");
    }
    let salt = random::<16>();
    let (m_kib, t, p) = (64 * 1024, 3, 1);
    let key = derive(passphrase, &salt, m_kib, t, p)?;
    let kf = KeyFile { version: 1, kdf: "argon2id".into(), m_kib, t, p, salt: hex(&salt), check: hex(&seal(&key, CHECK)) };
    Ok((kf, key))
}

/// The key for `passphrase`, or an error when it's the wrong one.
pub fn unlock(kf: &KeyFile, passphrase: &str) -> Result<Key> {
    if kf.version != 1 || kf.kdf != "argon2id" {
        bail!("this sync repo was set up by a newer Valerie: update Valerie on this computer first");
    }
    let key = derive(passphrase, &unhex(&kf.salt)?, kf.m_kib, kf.t, kf.p)?;
    match open(&key, &unhex(&kf.check)?) {
        Ok(v) if v == CHECK => Ok(key),
        _ => bail!("that isn't the sync passphrase"),
    }
}

/// MAGIC, a random nonce, then the ciphertext (which includes its authentication tag).
pub fn seal(key: &Key, plain: &[u8]) -> Vec<u8> {
    let nonce = random::<24>();
    let ct = XChaCha20Poly1305::new(key.into()).encrypt(XNonce::from_slice(&nonce), plain).expect("encrypting in memory");
    [&MAGIC[..], &nonce, &ct].concat()
}

pub fn open(key: &Key, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 4 + 24 + 16 || &data[..4] != MAGIC {
        bail!("not an encrypted Valerie file");
    }
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(&data[4..28]), &data[28..])
        .map_err(|_| anyhow::anyhow!("couldn't decrypt (wrong passphrase, or the file was changed)"))
}

pub fn read_key_file(repo_dir: &Path) -> Option<KeyFile> {
    serde_json::from_str(&std::fs::read_to_string(repo_dir.join(KEY_FILE)).ok()?).ok()
}

fn key_path() -> std::path::PathBuf {
    crate::config::data_dir().join("sync-key")
}

/// This machine's key for the repo whose key file has this salt (a new passphrase means a new salt, so an old key
/// is never used by mistake).
pub fn stored_key(kf: &KeyFile) -> Option<Key> {
    let hexed = keychain_get(&kf.salt).or_else(|| std::fs::read_to_string(key_path()).ok().and_then(|s| s.trim().strip_prefix(&format!("{}:", kf.salt)).map(str::to_string)))?;
    let key: Key = unhex(&hexed).ok()?.try_into().ok()?;
    // A key that doesn't open the check value belongs to another passphrase.
    matches!(open(&key, &unhex(&kf.check).ok()?), Ok(v) if v == CHECK).then_some(key)
}

// Tests keep keys in their own data folder, never in the real Keychain.
#[cfg(not(test))]
fn keychain_get(account: &str) -> Option<String> {
    crate::passwords::keychain::get(KEYCHAIN, account).ok().flatten()
}
#[cfg(test)]
fn keychain_get(_: &str) -> Option<String> {
    None
}

pub fn store_key(kf: &KeyFile, key: &Key) -> Result<()> {
    #[cfg(not(test))]
    if crate::passwords::keychain::save(KEYCHAIN, &kf.salt, "Valerie sync key", &hex(key)).is_ok() {
        return Ok(());
    }
    // No Keychain on this system yet: a file only this user can read.
    let path = key_path();
    std::fs::create_dir_all(path.parent().context("data dir")?)?;
    std::fs::write(&path, format!("{}:{}", kf.salt, hex(key)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// A strong passphrase that's easy to type: five made-up words.
pub fn suggest_passphrase() -> String {
    const C: &[u8] = b"bdfgklmnprstvz";
    const V: &[u8] = b"aeiou";
    let r = random::<20>();
    (0..5)
        .map(|w| {
            let b = &r[w * 4..w * 4 + 4];
            [C[b[0] as usize % C.len()], V[b[1] as usize % V.len()], C[b[2] as usize % C.len()], V[b[3] as usize % V.len()], C[(b[0] ^ b[3]) as usize % C.len()]]
                .iter()
                .map(|&c| c as char)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seals_and_checks_the_passphrase() {
        let (kf, key) = new_key_file("correct horse battery staple").unwrap();
        assert_eq!(unlock(&kf, "correct horse battery staple").unwrap(), key);
        assert!(unlock(&kf, "wrong passphrase entirely").is_err());
        let sealed = seal(&key, b"tabs and chats");
        assert_eq!(open(&key, &sealed).unwrap(), b"tabs and chats");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&key, &tampered).is_err());
        assert!(new_key_file("short").is_err());
        let s = suggest_passphrase();
        assert_eq!(s.split('-').count(), 5);
        assert!(s.len() >= 12);
    }
}
