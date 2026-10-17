//! Browser extensions: unpacked web extensions (a folder with a manifest.json) in `<config dir>/extensions/`, run by
//! WebKit's own extension support on macOS 15.4 and later. Each is loaded once the user has allowed it.
use serde_json::Value;
use std::path::{Path, PathBuf};

/// An extension folder and what its manifest says.
#[derive(Debug, Clone)]
pub struct Found {
    /// The folder's name (how settings and the command bar refer to it).
    pub name: String,
    pub dir: PathBuf,
    pub title: String,
    pub version: String,
    /// Sites and permissions it asks for, for the "Load this extension?" question.
    pub wants: Vec<String>,
    /// A fingerprint of its manifest: a changed manifest is asked about again.
    pub print: String,
    /// It has a toolbar button (which may open a popup), and an options page.
    pub button: bool,
    pub options: bool,
}

pub fn dir(config: &crate::config::ExtensionsConfig) -> PathBuf {
    let f = config.folder.trim();
    if f.is_empty() {
        return crate::config::config_dir().join("extensions");
    }
    match f.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(f),
    }
}

/// Extension folders (each with a manifest.json), sorted by name.
pub fn find(root: &Path) -> Vec<Found> {
    let mut out: Vec<Found> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
            let m: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            let title = m["name"].as_str().filter(|t| !t.starts_with("__MSG_")).unwrap_or(&name).to_string();
            let mut wants: Vec<String> = ["permissions", "host_permissions"]
                .iter()
                .flat_map(|k| m[*k].as_array().cloned().unwrap_or_default())
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            for cs in m["content_scripts"].as_array().into_iter().flatten() {
                wants.extend(cs["matches"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)));
            }
            wants.sort();
            wants.dedup();
            let button = ["action", "browser_action", "page_action"].iter().any(|k| m.get(*k).is_some());
            let options = m.get("options_page").is_some() || m.get("options_ui").is_some();
            Some(Found { name, dir, title, version: m["version"].as_str().unwrap_or("").to_string(), wants, print: crate::blocking::hash(&text), button, options })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    use std::cell::RefCell;

    thread_local! {
        static CONTROLLER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
        /// Loaded extensions: (folder name, its WKWebExtensionContext).
        static LOADED: RefCell<Vec<(String, Retained<AnyObject>)>> = const { RefCell::new(Vec::new()) };
    }

    /// Whether this macOS can run web extensions.
    pub fn supported() -> bool {
        AnyClass::get(c"WKWebExtensionController").is_some()
    }

    /// Extensions loaded so far (folder names).
    pub fn loaded() -> Vec<String> {
        LOADED.with(|l| l.borrow().iter().map(|(n, _)| n.clone()).collect())
    }

    fn controller() -> Option<Retained<AnyObject>> {
        let cls = AnyClass::get(c"WKWebExtensionController")?;
        CONTROLLER.with(|c| {
            let mut c = c.borrow_mut();
            if c.is_none() {
                let made: Option<Retained<AnyObject>> = unsafe { msg_send![cls, new] };
                *c = made;
            }
            c.clone()
        })
    }

    /// Load an extension folder; `done` gets an error message when it can't be.
    pub fn load(name: String, dir: &std::path::Path, done: impl FnOnce(Result<(), String>) + 'static) {
        let (Some(ctrl), Some(ext_cls)) = (controller(), AnyClass::get(c"WKWebExtension")) else {
            return done(Err("this macOS can't run browser extensions (it needs macOS 15.4 or later)".into()));
        };
        if LOADED.with(|l| l.borrow().iter().any(|(n, _)| *n == name)) {
            return done(Ok(()));
        }
        let once = RefCell::new(Some((done, name, ctrl)));
        let block = block2::RcBlock::new(move |ext: *mut AnyObject, err: *mut AnyObject| {
            let Some((done, name, ctrl)) = once.borrow_mut().take() else { return };
            if ext.is_null() {
                let why = unsafe { describe(err) };
                return done(Err(why));
            }
            unsafe {
                let ctx_cls = AnyClass::get(c"WKWebExtensionContext").unwrap();
                let ctx: Option<Retained<AnyObject>> = msg_send![ctx_cls, contextForExtension: ext];
                let Some(ctx) = ctx else { return done(Err("couldn't make a context for it".into())) };
                // Grant what it asks for: the user allowed it before it got here.
                let perms: *mut AnyObject = msg_send![ext, requestedPermissions];
                for p in set_items(perms) {
                    let _: () = msg_send![&*ctx, setPermissionStatus: 3isize, forPermission: p];
                }
                let patterns: *mut AnyObject = msg_send![ext, allRequestedMatchPatterns];
                for p in set_items(patterns) {
                    let _: () = msg_send![&*ctx, setPermissionStatus: 3isize, forMatchPattern: p];
                }
                let mut error: *mut AnyObject = std::ptr::null_mut();
                let ok: bool = msg_send![&*ctrl, loadExtensionContext: &*ctx, error: &mut error];
                if !ok {
                    return done(Err(describe(error)));
                }
                LOADED.with(|l| l.borrow_mut().push((name, ctx)));
                done(Ok(()))
            }
        });
        unsafe {
            let path = NSString::from_str(&dir.to_string_lossy());
            let url: *mut AnyObject = msg_send![objc2::class!(NSURL), fileURLWithPath: &*path, isDirectory: true];
            let _: () = msg_send![ext_cls, extensionWithResourceBaseURL: url, completionHandler: &*block];
        }
    }

    unsafe fn set_items(set: *mut AnyObject) -> Vec<*mut AnyObject> {
        if set.is_null() {
            return vec![];
        }
        let all: *mut AnyObject = msg_send![set, allObjects];
        let n: usize = msg_send![all, count];
        (0..n).map(|i| msg_send![all, objectAtIndex: i]).collect()
    }

    unsafe fn describe(err: *mut AnyObject) -> String {
        if err.is_null() {
            return "it couldn't be loaded".into();
        }
        let d: *mut NSString = msg_send![err, localizedDescription];
        if d.is_null() { "it couldn't be loaded".into() } else { (*d).to_string() }
    }

    /// A configuration for a new page that has the extensions in it, using the profile's storage. None when no
    /// extension is loaded (pages are then built the usual way).
    pub fn page_configuration(profile_id: [u8; 16]) -> Option<Retained<AnyObject>> {
        if LOADED.with(|l| l.borrow().is_empty()) {
            return None;
        }
        let ctrl = controller()?;
        unsafe {
            let config: Retained<AnyObject> = msg_send![objc2::class!(WKWebViewConfiguration), new];
            let uuid_alloc: *mut AnyObject = msg_send![objc2::class!(NSUUID), alloc];
            let uuid: *mut AnyObject = msg_send![uuid_alloc, initWithUUIDBytes: profile_id.as_ptr()];
            let store: *mut AnyObject = msg_send![objc2::class!(WKWebsiteDataStore), dataStoreForIdentifier: uuid];
            let _: () = msg_send![uuid, release];
            if store.is_null() {
                return None;
            }
            let _: () = msg_send![&*config, setWebsiteDataStore: store];
            let _: () = msg_send![&*config, setWebExtensionController: &*ctrl];
            Some(config)
        }
    }

    fn context(name: &str) -> Option<Retained<AnyObject>> {
        LOADED.with(|l| l.borrow().iter().find(|(n, _)| n == name).map(|(_, c)| c.clone()))
    }

    /// The extension's options page address, if it has one.
    pub fn options_url(name: &str) -> Option<String> {
        let ctx = context(name)?;
        unsafe {
            let url: *mut AnyObject = msg_send![&*ctx, optionsPageURL];
            if url.is_null() {
                return None;
            }
            let s: *mut NSString = msg_send![url, absoluteString];
            (!s.is_null()).then(|| (*s).to_string())
        }
    }

    /// Show the extension's popup next to `x, y` in the view (or click its button if it has no popup).
    pub fn show_action(name: &str, view: *mut AnyObject, x: f64, y: f64) -> Result<(), String> {
        let ctx = context(name).ok_or("that extension isn't loaded")?;
        unsafe {
            let action: *mut AnyObject = msg_send![&*ctx, actionForTab: std::ptr::null_mut::<AnyObject>()];
            if action.is_null() {
                return Err("it has no button".into());
            }
            let popup: bool = msg_send![action, presentsPopup];
            if !popup {
                let _: () = msg_send![&*ctx, performActionForTab: std::ptr::null_mut::<AnyObject>()];
                return Ok(());
            }
            let popover: *mut AnyObject = msg_send![action, popupPopover];
            if popover.is_null() {
                return Err("its popup couldn't be opened".into());
            }
            let shown: bool = msg_send![popover, isShown];
            if shown {
                let _: () = msg_send![popover, close];
                return Ok(());
            }
            let rect = objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(x, y), objc2_foundation::NSSize::new(1.0, 1.0));
            // NSRectEdgeMinY: below the point in a flipped view.
            let _: () = msg_send![popover, showRelativeToRect: rect, ofView: view, preferredEdge: 1usize];
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    pub fn supported() -> bool {
        false
    }
    pub fn loaded() -> Vec<String> {
        vec![]
    }
    pub fn load(_name: String, _dir: &std::path::Path, done: impl FnOnce(Result<(), String>) + 'static) {
        done(Err("browser extensions work on macOS 15.4 or later".into()))
    }
    pub fn options_url(_name: &str) -> Option<String> {
        None
    }
}

#[cfg(not(target_os = "macos"))]
pub use other::*;
