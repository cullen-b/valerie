//! Password vaults in the system's own store (macOS Keychain, Windows Credential Manager, the Linux desktop's keyring):
//! one vault per profile plus shared vaults declared in config. Each credential is an item: service "Valerie — <vault>", account "<origin>\n<username>".
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

#[cfg(windows)]
pub(crate) mod keychain {
    use anyhow::{bail, Result};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{CredDeleteW, CredEnumerateW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};

    /// Credential Manager's limit for one secret.
    const MOST_BYTES: usize = 2560;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn target(service: &str, account: &str) -> Vec<u16> {
        wide(&format!("{service}/{account}"))
    }

    fn missing(e: &windows::core::Error) -> bool {
        e.code() == ERROR_NOT_FOUND.to_hresult()
    }

    pub fn save(service: &str, account: &str, label: &str, password: &str) -> Result<()> {
        if password.len() > MOST_BYTES {
            bail!("that password is too long for Windows Credential Manager");
        }
        let (mut name, mut comment, mut secret) = (target(service, account), wide(label), password.as_bytes().to_vec());
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(name.as_mut_ptr()),
            Comment: PWSTR(comment.as_mut_ptr()),
            CredentialBlobSize: secret.len() as u32,
            CredentialBlob: secret.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        unsafe { CredWriteW(&credential, 0) }.map_err(|e| anyhow::anyhow!("Windows Credential Manager refused it: {}", e.message()))
    }

    pub fn get(service: &str, account: &str) -> Result<Option<String>> {
        let name = target(service, account);
        let mut found: *mut CREDENTIALW = std::ptr::null_mut();
        match unsafe { CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut found) } {
            Ok(()) => {}
            Err(e) if missing(&e) => return Ok(None),
            Err(e) => bail!("couldn't read Windows Credential Manager: {}", e.message()),
        }
        unsafe {
            let bytes = std::slice::from_raw_parts((*found).CredentialBlob, (*found).CredentialBlobSize as usize);
            let password = String::from_utf8_lossy(bytes).into_owned();
            CredFree(found as *const _);
            Ok(Some(password))
        }
    }

    /// Account names stored under a service.
    pub fn accounts(service: &str) -> Result<Vec<String>> {
        let prefix = format!("{service}/");
        let filter = wide(&format!("{prefix}*"));
        let (mut count, mut found) = (0u32, std::ptr::null_mut());
        match unsafe { CredEnumerateW(PCWSTR(filter.as_ptr()), None, &mut count, &mut found) } {
            Ok(()) => {}
            Err(e) if missing(&e) => return Ok(vec![]),
            Err(e) => bail!("couldn't read Windows Credential Manager: {}", e.message()),
        }
        unsafe {
            let all = std::slice::from_raw_parts(found, count as usize);
            let names = all.iter().filter_map(|c| (**c).TargetName.to_string().ok()?.strip_prefix(&prefix).map(str::to_string)).collect();
            CredFree(found as *const _);
            Ok(names)
        }
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let name = target(service, account);
        match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(()),
            Err(e) if missing(&e) => Ok(()),
            Err(e) => bail!("couldn't delete from Windows Credential Manager: {}", e.message()),
        }
    }
}

/// The desktop's keyring (GNOME Keyring, KWallet, ...) through the freedesktop Secret Service.
#[cfg(target_os = "linux")]
pub(crate) mod keychain {
    use anyhow::{anyhow, bail, Context, Result};
    use gtk::gio;
    use gtk::glib::variant::ObjectPath;
    use gtk::glib::{ToVariant, Variant};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    const ROOT: &str = "/org/freedesktop/secrets";
    const SERVICE: &str = "org.freedesktop.Secret.Service";
    const COLLECTION: &str = "org.freedesktop.Secret.Collection";
    const ITEM: &str = "org.freedesktop.Secret.Item";
    const NO_STORE: &str = "this computer has no keyring running to keep passwords in (GNOME Keyring or KWallet, say)";
    /// How long to wait for someone to answer the keyring's own window.
    const PROMPT_WAIT: Duration = Duration::from_secs(90);

    struct Keyring {
        bus: gio::DBusConnection,
        session: ObjectPath,
    }

    impl Drop for Keyring {
        fn drop(&mut self) {
            let _ = self.call(&self.session.to_string(), "org.freedesktop.Secret.Session", "Close", ().to_variant());
        }
    }

    fn path(v: Variant) -> Result<ObjectPath> {
        v.get().context("the keyring sent something unexpected")
    }

    impl Keyring {
        fn open() -> Result<Self> {
            let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).context(NO_STORE)?;
            let mut keyring = Self { bus, session: ObjectPath::try_from("/").map_err(|_| anyhow!("bad path"))? };
            // "plain": the secret crosses the desktop's own private bus, which only this user can reach.
            let opened = keyring.call(ROOT, SERVICE, "OpenSession", ("plain", "".to_variant()).to_variant()).context(NO_STORE)?;
            keyring.session = path(opened.child_value(1))?;
            Ok(keyring)
        }

        fn call(&self, object: &str, interface: &str, method: &str, args: Variant) -> Result<Variant> {
            self.bus
                .call_sync(Some("org.freedesktop.secrets"), object, interface, method, Some(&args), None, gio::DBusCallFlags::NONE, 10_000, gio::Cancellable::NONE)
                .map_err(|e| anyhow!("the keyring said: {}", e.message()))
        }

        fn property(&self, object: &str, interface: &str, name: &str) -> Result<Variant> {
            let reply = self.call(object, "org.freedesktop.DBus.Properties", "Get", (interface, name).to_variant())?;
            reply.child_value(0).as_variant().context("the keyring sent something unexpected")
        }

        /// The keyring draws its own window for this; there's nothing to wait on without a main loop, so `done` is polled.
        fn answer(&self, prompt: &ObjectPath, done: impl Fn() -> bool) -> Result<()> {
            if prompt.as_str() != "/" {
                self.call(prompt, "org.freedesktop.Secret.Prompt", "Prompt", ("",).to_variant())?;
            }
            let start = Instant::now();
            while !done() {
                if start.elapsed() > PROMPT_WAIT {
                    bail!("the keyring is still locked, so try again and unlock it when it asks");
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(())
        }

        fn default_alias(&self) -> Result<ObjectPath> {
            path(self.call(ROOT, SERVICE, "ReadAlias", ("default",).to_variant())?.child_value(0))
        }

        /// The default keyring, unlocked; a new account may not have one yet, and this makes it.
        fn unlocked(&self) -> Result<String> {
            let mut collection = self.default_alias()?;
            if collection.as_str() == "/" {
                let label = HashMap::from([(format!("{COLLECTION}.Label"), "Login".to_variant())]);
                let made = self.call(ROOT, SERVICE, "CreateCollection", (label, "default").to_variant())?;
                self.answer(&path(made.child_value(1))?, || self.default_alias().is_ok_and(|a| a.as_str() != "/"))?;
                collection = self.default_alias()?;
            }
            let locked = || self.property(&collection, COLLECTION, "Locked").ok().and_then(|v| v.get::<bool>()).unwrap_or(true);
            if locked() {
                let reply = self.call(ROOT, SERVICE, "Unlock", (vec![collection.clone()],).to_variant())?;
                self.answer(&path(reply.child_value(1))?, || !locked())?;
            }
            Ok(collection.to_string())
        }

        /// Matching items, whether or not their keyring is locked (their names can be read either way).
        fn find(&self, attributes: HashMap<String, String>) -> Result<Vec<ObjectPath>> {
            let reply = self.call(ROOT, SERVICE, "SearchItems", (attributes,).to_variant())?;
            let list = |i| reply.child_value(i).get::<Vec<ObjectPath>>().unwrap_or_default();
            Ok(list(0).into_iter().chain(list(1)).collect())
        }
    }

    fn named(service: &str, account: Option<&str>) -> HashMap<String, String> {
        let mut attributes = HashMap::from([("service".to_string(), service.to_string())]);
        attributes.extend(account.map(|a| ("account".to_string(), a.to_string())));
        attributes
    }

    pub fn save(service: &str, account: &str, label: &str, password: &str) -> Result<()> {
        let keyring = Keyring::open()?;
        let collection = keyring.unlocked()?;
        let properties = HashMap::from([(format!("{ITEM}.Label"), label.to_variant()), (format!("{ITEM}.Attributes"), named(service, Some(account)).to_variant())]);
        let secret = (keyring.session.clone(), Vec::<u8>::new(), password.as_bytes().to_vec(), "text/plain");
        keyring.call(&collection, COLLECTION, "CreateItem", (properties, secret, true).to_variant())?;
        Ok(())
    }

    pub fn get(service: &str, account: &str) -> Result<Option<String>> {
        let keyring = Keyring::open()?;
        let Some(item) = keyring.find(named(service, Some(account)))?.into_iter().next() else { return Ok(None) };
        keyring.unlocked()?;
        let reply = keyring.call(&item, ITEM, "GetSecret", (keyring.session.clone(),).to_variant())?;
        let value = reply.child_value(0).child_value(2).get::<Vec<u8>>().context("the keyring sent something unexpected")?;
        Ok(Some(String::from_utf8_lossy(&value).into_owned()))
    }

    /// Account names stored under a service.
    pub fn accounts(service: &str) -> Result<Vec<String>> {
        let keyring = Keyring::open()?;
        let items = keyring.find(named(service, None))?;
        Ok(items.iter().filter_map(|i| keyring.property(i, ITEM, "Attributes").ok()?.get::<HashMap<String, String>>()?.remove("account")).collect())
    }

    pub fn delete(service: &str, account: &str) -> Result<()> {
        let keyring = Keyring::open()?;
        let items = keyring.find(named(service, Some(account)))?;
        if !items.is_empty() {
            keyring.unlocked()?;
        }
        for item in items {
            keyring.call(&item, ITEM, "Delete", ().to_variant())?;
        }
        Ok(())
    }
}

fn account(origin: &str, username: &str) -> String {
    format!("{origin}\n{username}")
}

/// Runs `work`, calling `slow` if it's still going after `after`: the Mac's Keychain window holds a call open until it's answered.
pub fn noting_a_wait<T>(after: std::time::Duration, slow: impl FnOnce() + Send + 'static, work: impl FnOnce() -> T) -> T {
    let (finished, wait) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if matches!(wait.recv_timeout(after), Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
            slow();
        }
    });
    let out = work();
    drop(finished);
    out
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

    #[test]
    fn a_wait_is_noted_only_when_the_work_runs_long() {
        use std::sync::mpsc::channel;
        use std::time::Duration;
        let (tx, rx) = channel();
        assert_eq!(noting_a_wait(Duration::from_millis(300), move || tx.send(()).unwrap(), || 1), 1);
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
        let (tx, rx) = channel();
        noting_a_wait(Duration::from_millis(10), move || tx.send(()).unwrap(), || std::thread::sleep(Duration::from_millis(200)));
        assert!(rx.try_recv().is_ok());
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
