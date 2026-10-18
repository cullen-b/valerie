//! Password vaults in the macOS Keychain: one vault per profile plus shared vaults declared in config.
//! Each credential is a generic-password item: service "Valerie — <vault>", account "<origin>\n<username>".
use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub vault: String,
    pub origin: String,
    pub username: String,
}

fn service(vault: &str) -> String {
    format!("Valerie — {vault}")
}

/// "https://accounts.example.com" for any URL on that origin.
pub fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    matches!(u.scheme(), "https" | "http").then(|| u.origin().ascii_serialization())
}

/// Minimal CSV reader (quoted fields, doubled quotes, newlines inside quotes) for browser password exports.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            ('\r', false) => {}
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Import a Chrome/Arc/Safari/1Password-style CSV export (needs url, username and password columns).
pub fn import_csv(path: &std::path::Path, vault: &str) -> Result<usize> {
    let rows = parse_csv(&std::fs::read_to_string(path)?);
    let Some(header) = rows.first() else { bail!("empty CSV") };
    let col = |names: &[&str]| header.iter().position(|h| names.contains(&h.trim().to_lowercase().as_str()));
    let (Some(u), Some(n), Some(p)) = (col(&["url", "website", "login_uri"]), col(&["username", "login_username", "user"]), col(&["password", "login_password"])) else {
        bail!("CSV needs url, username and password columns");
    };
    let mut count = 0;
    for r in rows.iter().skip(1) {
        let (Some(url), Some(user), Some(pw)) = (r.get(u), r.get(n), r.get(p)) else { continue };
        let Some(origin) = origin_of(url) else { continue };
        if pw.is_empty() {
            continue;
        }
        save(vault, &origin, user, pw)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(target_os = "macos")]
pub(crate) mod keychain {
    use anyhow::{bail, Result};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        static kSecClass: *const AnyObject;
        static kSecClassGenericPassword: *const AnyObject;
        static kSecAttrService: *const AnyObject;
        static kSecAttrAccount: *const AnyObject;
        static kSecAttrLabel: *const AnyObject;
        static kSecValueData: *const AnyObject;
        static kSecReturnData: *const AnyObject;
        static kSecReturnAttributes: *const AnyObject;
        static kSecMatchLimit: *const AnyObject;
        static kSecMatchLimitAll: *const AnyObject;
        static kSecMatchLimitOne: *const AnyObject;
        fn SecItemAdd(attributes: *const AnyObject, result: *mut *const AnyObject) -> i32;
        fn SecItemCopyMatching(query: *const AnyObject, result: *mut *const AnyObject) -> i32;
        fn SecItemUpdate(query: *const AnyObject, attributes: *const AnyObject) -> i32;
        fn SecItemDelete(query: *const AnyObject) -> i32;
    }

    const NOT_FOUND: i32 = -25300;
    const DUPLICATE: i32 = -25299;

    /// An autoreleased NSMutableDictionary built from (key, value) object pairs.
    unsafe fn dict(pairs: &[(*const AnyObject, *const AnyObject)]) -> *mut AnyObject {
        let d: *mut AnyObject = msg_send![class!(NSMutableDictionary), dictionary];
        for (k, v) in pairs {
            let _: () = msg_send![d, setObject: *v, forKey: *k];
        }
        d
    }

    fn ns(s: &str) -> objc2::rc::Retained<NSString> {
        NSString::from_str(s)
    }

    unsafe fn yes() -> *const AnyObject {
        msg_send![class!(NSNumber), numberWithBool: true]
    }

    unsafe fn data(bytes: &[u8]) -> *const AnyObject {
        msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() as *const std::ffi::c_void, length: bytes.len()]
    }

    unsafe fn to_string(obj: *const AnyObject) -> Option<String> {
        if obj.is_null() {
            return None;
        }
        Some((*(obj as *const NSString)).to_string())
    }

    pub fn save(service: &str, account: &str, label: &str, password: &str) -> Result<()> {
        let (s, a, l) = (ns(service), ns(account), ns(label));
        unsafe {
            let sp = &*s as *const NSString as *const AnyObject;
            let ap = &*a as *const NSString as *const AnyObject;
            let lp = &*l as *const NSString as *const AnyObject;
            let pw = data(password.as_bytes());
            let attrs = dict(&[(kSecClass, kSecClassGenericPassword), (kSecAttrService, sp), (kSecAttrAccount, ap), (kSecAttrLabel, lp), (kSecValueData, pw)]);
            let status = SecItemAdd(attrs, std::ptr::null_mut());
            if status == DUPLICATE {
                let query = dict(&[(kSecClass, kSecClassGenericPassword), (kSecAttrService, sp), (kSecAttrAccount, ap)]);
                let update = dict(&[(kSecValueData, pw), (kSecAttrLabel, lp)]);
                let status = SecItemUpdate(query, update);
                if status != 0 {
                    bail!("keychain update failed ({status})");
                }
            } else if status != 0 {
                bail!("keychain add failed ({status})");
            }
        }
        Ok(())
    }

    pub fn get(service: &str, account: &str) -> Result<Option<String>> {
        let (s, a) = (ns(service), ns(account));
        unsafe {
            let query = dict(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, &*s as *const NSString as *const AnyObject),
                (kSecAttrAccount, &*a as *const NSString as *const AnyObject),
                (kSecReturnData, yes()),
                (kSecMatchLimit, kSecMatchLimitOne),
            ]);
            let mut out: *const AnyObject = std::ptr::null();
            match SecItemCopyMatching(query, &mut out) {
                0 => {}
                NOT_FOUND => return Ok(None),
                status => bail!("keychain read failed ({status})"),
            }
            let len: usize = msg_send![out, length];
            let bytes: *const u8 = msg_send![out, bytes];
            let pw = String::from_utf8_lossy(std::slice::from_raw_parts(bytes, len)).into_owned();
            let _: () = msg_send![out, release]; // CopyMatching returns +1
            Ok(Some(pw))
        }
    }

    /// Account names stored under a service.
    pub fn accounts(service: &str) -> Result<Vec<String>> {
        let s = ns(service);
        unsafe {
            let query = dict(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, &*s as *const NSString as *const AnyObject),
                (kSecReturnAttributes, yes()),
                (kSecMatchLimit, kSecMatchLimitAll),
            ]);
            let mut out: *const AnyObject = std::ptr::null();
            match SecItemCopyMatching(query, &mut out) {
                0 => {}
                NOT_FOUND => return Ok(vec![]),
                status => bail!("keychain list failed ({status})"),
            }
            let n: usize = msg_send![out, count];
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let item: *const AnyObject = msg_send![out, objectAtIndex: i];
                let acct: *const AnyObject = msg_send![item, objectForKey: kSecAttrAccount];
                v.extend(to_string(acct));
            }
            let _: () = msg_send![out, release];
            Ok(v)
        }
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let (s, a) = (ns(service), ns(account));
        unsafe {
            let query = dict(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, &*s as *const NSString as *const AnyObject),
                (kSecAttrAccount, &*a as *const NSString as *const AnyObject),
            ]);
            match SecItemDelete(query) {
                0 | NOT_FOUND => Ok(()),
                status => bail!("keychain delete failed ({status})"),
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) mod keychain {
    use anyhow::{bail, Result};
    pub fn save(_: &str, _: &str, _: &str, _: &str) -> Result<()> {
        bail!("password storage is only implemented on macOS so far")
    }
    pub fn get(_: &str, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    pub fn accounts(_: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    pub fn delete(_: &str, _: &str) -> Result<()> {
        Ok(())
    }
}

fn account(origin: &str, username: &str) -> String {
    format!("{origin}\n{username}")
}

pub fn save(vault: &str, origin: &str, username: &str, password: &str) -> Result<()> {
    store(vault, origin, username, password)?;
    stamp(vault, origin, username, crate::state::now(), false);
    Ok(())
}

fn store(vault: &str, origin: &str, username: &str, password: &str) -> Result<()> {
    let host = url::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    keychain::save(&service(vault), &account(origin, username), &format!("Valerie: {host} ({username})"), password)
}

pub fn password(e: &Entry) -> Result<Option<String>> {
    keychain::get(&service(&e.vault), &account(&e.origin, &e.username))
}

pub fn delete(e: &Entry) -> Result<()> {
    keychain::delete(&service(&e.vault), &account(&e.origin, &e.username))?;
    stamp(&e.vault, &e.origin, &e.username, crate::state::now(), true);
    Ok(())
}

/// Every entry in the given vaults (usernames only; passwords stay in the keychain until filled).
pub fn list(vaults: &[String]) -> Vec<Entry> {
    vaults
        .iter()
        .flat_map(|v| {
            keychain::accounts(&service(v)).unwrap_or_default().into_iter().filter_map(move |a| {
                let (origin, username) = a.split_once('\n')?;
                Some(Entry { vault: v.clone(), origin: origin.to_string(), username: username.to_string() })
            })
        })
        .collect()
}

// ---------------------------------------------------------------- sync (sync.rs encrypts these before they leave)

/// One saved password as it travels between machines; `deleted` marks one removed (so it goes everywhere).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Synced {
    pub vault: String,
    pub origin: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
    pub modified: u64,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Stamp {
    modified: u64,
    #[serde(default)]
    deleted: bool,
}

fn stamps_path() -> std::path::PathBuf {
    crate::config::data_dir().join("password-changes.json")
}

/// When each password was last saved or deleted on this machine (the Keychain doesn't say in a way that travels).
fn stamps() -> std::collections::BTreeMap<String, Stamp> {
    std::fs::read_to_string(stamps_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn stamp(vault: &str, origin: &str, username: &str, modified: u64, deleted: bool) {
    let mut all = stamps();
    all.insert(format!("{vault}\n{origin}\n{username}"), Stamp { modified, deleted });
    let p = stamps_path();
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
    if let Ok(s) = serde_json::to_string(&all) {
        let _ = std::fs::write(p, s);
    }
}

/// Something that changes whenever a password is added, changed or removed here, without reading any secret
/// (reading every password on each sync would be slow, and could make the Keychain ask).
pub fn signature(vaults: &[String]) -> String {
    let mut names: Vec<String> = list(vaults).iter().map(|e| format!("{}\n{}\n{}", e.vault, e.origin, e.username)).collect();
    names.sort();
    format!("{}|{}", names.join("\t"), std::fs::read_to_string(stamps_path()).unwrap_or_default())
}

/// Every password in these vaults, plus the ones deleted here, for the other machines.
pub fn export(vaults: &[String]) -> Vec<Synced> {
    let st = stamps();
    let mut out: Vec<Synced> = list(vaults)
        .into_iter()
        .filter_map(|e| {
            let pw = password(&e).ok().flatten()?;
            let modified = st.get(&format!("{}\n{}\n{}", e.vault, e.origin, e.username)).map_or(0, |s| s.modified);
            Some(Synced { vault: e.vault, origin: e.origin, username: e.username, password: pw, modified, deleted: false })
        })
        .collect();
    for (k, s) in st.iter().filter(|(_, s)| s.deleted) {
        let mut parts = k.splitn(3, '\n');
        if let (Some(v), Some(o), Some(u)) = (parts.next(), parts.next(), parts.next()) {
            if vaults.iter().any(|x| x == v) {
                out.push(Synced { vault: v.into(), origin: o.into(), username: u.into(), password: String::new(), modified: s.modified, deleted: true });
            }
        }
    }
    out
}

/// The newest of each password across `lists` (other machines' exports).
pub fn newest(lists: Vec<Vec<Synced>>) -> Vec<Synced> {
    let mut best: std::collections::BTreeMap<String, Synced> = Default::default();
    for e in lists.into_iter().flatten() {
        let k = format!("{}\n{}\n{}", e.vault, e.origin, e.username);
        if best.get(&k).is_none_or(|b| e.modified > b.modified) {
            best.insert(k, e);
        }
    }
    best.into_values().collect()
}

/// Bring in other machines' changes that are newer than this machine's. Returns how many changed here.
pub fn import(remote: &[Synced], vaults: &[String]) -> usize {
    let st = stamps();
    let here: std::collections::BTreeSet<String> = list(vaults).iter().map(|e| format!("{}\n{}\n{}", e.vault, e.origin, e.username)).collect();
    let mut n = 0;
    for e in remote.iter().filter(|e| vaults.contains(&e.vault)) {
        let k = format!("{}\n{}\n{}", e.vault, e.origin, e.username);
        let mine = st.get(&k).copied();
        let have = here.contains(&k);
        let newer = match mine {
            Some(m) => e.modified > m.modified,
            None => !have || e.modified > 0,
        };
        if !newer {
            continue;
        }
        let (ok, changed) = if e.deleted {
            (!have || keychain::delete(&service(&e.vault), &account(&e.origin, &e.username)).is_ok(), have)
        } else {
            let same = have && password(&Entry { vault: e.vault.clone(), origin: e.origin.clone(), username: e.username.clone() }).ok().flatten().as_deref() == Some(e.password.as_str());
            (same || store(&e.vault, &e.origin, &e.username, &e.password).is_ok(), !same)
        };
        if ok {
            stamp(&e.vault, &e.origin, &e.username, e.modified, e.deleted);
            n += usize::from(changed);
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_change_wins() {
        let e = |pw: &str, modified: u64, deleted: bool| Synced { vault: "Me".into(), origin: "https://a.com".into(), username: "u".into(), password: pw.into(), modified, deleted };
        let n = newest(vec![vec![e("old", 5, false)], vec![e("new", 9, false)], vec![e("", 7, true)]]);
        assert_eq!(n, vec![e("new", 9, false)]);
        let n = newest(vec![vec![e("old", 5, false)], vec![e("", 7, true)]]);
        assert!(n[0].deleted);
    }

    #[test]
    fn csv_handles_quotes_and_newlines() {
        let rows = parse_csv("name,url,username,password\n\"a, b\",https://x.com,me,\"p\"\"w\nd\"\n");
        assert_eq!(rows[1], vec!["a, b", "https://x.com", "me", "p\"w\nd"]);
    }

    /// Touches the real login keychain, so it only runs on request: `cargo test -- --ignored keychain`.
    #[test]
    #[ignore]
    fn keychain_roundtrip() {
        let vault = format!("Test {}", crate::mcp::random_hex());
        let e = Entry { vault: vault.clone(), origin: "https://example.test".into(), username: "me@example.test".into() };
        save(&vault, &e.origin, &e.username, "first").unwrap();
        save(&vault, &e.origin, &e.username, "second").unwrap(); // update, not duplicate
        assert_eq!(list(&[vault.clone()]), vec![e.clone()]);
        assert_eq!(password(&e).unwrap().as_deref(), Some("second"));
        delete(&e).unwrap();
        assert!(list(&[vault]).is_empty());
        assert_eq!(password(&e).unwrap(), None);
    }

    #[test]
    fn origins() {
        assert_eq!(origin_of("https://accounts.example.com/login?x=1").as_deref(), Some("https://accounts.example.com"));
        assert_eq!(origin_of("http://localhost:3000/a").as_deref(), Some("http://localhost:3000"));
        assert_eq!(origin_of("file:///etc/passwd"), None);
    }
}
