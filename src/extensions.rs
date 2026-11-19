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
    pub description: String,
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

/// Turn one extension on or off in the settings (`extensions.turned_off`); a running Valerie follows the file.
pub fn set_on(config: &crate::config::ExtensionsConfig, name: &str, on: bool) -> anyhow::Result<()> {
    let mut off: Vec<&str> = config.turned_off.iter().map(String::as_str).filter(|n| *n != name).collect();
    if !on {
        off.push(name);
    }
    crate::config::set_setting(&["extensions", "turned_off"], &serde_json::to_string(&off)?)
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
            let description = m["description"].as_str().filter(|t| !t.starts_with("__MSG_")).unwrap_or("").to_string();
            Some(Found { name, dir, title, description, version: m["version"].as_str().unwrap_or("").to_string(), wants, print: crate::blocking::hash(&text), button, options })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Something an extension asks Valerie to do with its tabs.
#[derive(Debug, Clone)]
pub enum Ask {
    Activate(u64),
    Close(u64),
    /// Open a tab; `reply` (when not 0) is answered with `opened(reply, tab)`.
    Open { url: String, active: bool, reply: u64 },
    FocusWindow,
    /// Show this extension's popup (by folder name).
    Popup(String),
}

/// The extension an address belongs to (its host), for `webkit-extension://` addresses.
pub fn extension_host(url: &str) -> Option<String> {
    let rest = url.get(..19).filter(|p| p.eq_ignore_ascii_case("webkit-extension://")).map(|_| &url[19..])?;
    Some(rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase())
}

/// A tab as extensions see it.
pub struct Seen<'a> {
    pub id: u64,
    pub url: &'a str,
    pub title: &'a str,
    pub loading: bool,
    /// Its WKWebView, when the page is loaded.
    pub view: Option<*mut std::ffi::c_void>,
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
                // Its own storage per data folder, so a test copy never shares the real one's extension data.
                let id = uuid(&format!("controller:{}", crate::config::data_dir().display()));
                let made: Option<Retained<AnyObject>> = unsafe {
                    let config: *mut AnyObject = msg_send![objc2::class!(WKWebExtensionControllerConfiguration), configurationWithIdentifier: &*id];
                    // Extensions tell browsers apart by user agent; without Safari's they can't (Bitwarden won't start).
                    let ua = crate::platform::safari_user_agent();
                    let name = NSString::from_str(ua.split("(KHTML, like Gecko) ").nth(1).unwrap_or("Safari/605.1.15"));
                    let pages: *mut AnyObject = msg_send![config, webViewConfiguration];
                    let _: () = msg_send![pages, setApplicationNameForUserAgent: &*name];
                    let _: () = msg_send![config, setWebViewConfiguration: pages];
                    let this: objc2::rc::Allocated<AnyObject> = msg_send![cls, alloc];
                    msg_send![this, initWithConfiguration: config]
                };
                if let Some(ctrl) = &made {
                    let d = bridge::delegate();
                    unsafe {
                        let _: () = msg_send![&**ctrl, setDelegate: &*d];
                    }
                }
                *c = made;
            }
            c.clone()
        })
    }

    thread_local! {
        /// How many of each extension's errors are in the log already.
        static LOGGED: RefCell<std::collections::HashMap<String, usize>> = RefCell::new(std::collections::HashMap::new());
    }

    /// Put errors WebKit noted for extensions (bad manifest entries, failing scripts) in the log, once each.
    pub fn log_errors() {
        let all: Vec<(String, Retained<AnyObject>)> = LOADED.with(|l| l.borrow().clone());
        for (name, ctx) in all {
            unsafe {
                let errors: *mut AnyObject = msg_send![&*ctx, errors];
                let n: usize = if errors.is_null() { 0 } else { msg_send![errors, count] };
                let seen = LOGGED.with_borrow(|l| l.get(&name).copied().unwrap_or(0));
                for i in seen.min(n)..n {
                    let e: *mut AnyObject = msg_send![errors, objectAtIndex: i];
                    crate::log::line("extensions", format!("{name}: {}", describe(e)));
                }
                LOGGED.with_borrow_mut(|l| l.insert(name, n));
            }
        }
    }

    /// The same NSUUID every time for `seed`.
    fn uuid(seed: &str) -> Retained<AnyObject> {
        use sha2::Digest;
        let bytes: [u8; 16] = sha2::Sha256::digest(seed.as_bytes())[..16].try_into().unwrap_or_default();
        unsafe {
            let this: *mut AnyObject = msg_send![objc2::class!(NSUUID), alloc];
            Retained::from_raw(msg_send![this, initWithUUIDBytes: bytes.as_ptr()]).expect("NSUUID")
        }
    }

    /// The folder name of a loaded extension's context.
    fn name_of(ctx: *mut AnyObject) -> Option<String> {
        LOADED.with(|l| l.borrow().iter().find(|(_, c)| std::ptr::eq(Retained::as_ptr(c), ctx)).map(|(n, _)| n.clone()))
    }

    pub use bridge::{connect, opened, sync};

    /// Valerie's one window and its tabs, as WebKit's extension engine asks about them: without this, extensions can't
    /// tell which page you're on (`tabs.query`), message it, or open and close tabs.
    mod bridge {
        use super::{name_of, CONTROLLER};
        use crate::extensions::{Ask, Seen};
        use block2::Block;
        use objc2::rc::{Retained, Weak};
        use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, NSObject, NSObjectProtocol};
        use objc2::{define_class, msg_send, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
        use objc2_foundation::{NSArray, NSString};
        use std::cell::{Cell, RefCell};
        use std::collections::HashMap;

        pub struct TabIvars {
            id: u64,
            url: RefCell<String>,
            title: RefCell<String>,
            loading: Cell<bool>,
            view: RefCell<Weak<AnyObject>>,
        }

        type Done = Block<dyn Fn(*mut AnyObject)>;
        type DoneWith = Block<dyn Fn(*mut AnyObject, *mut AnyObject)>;

        thread_local! {
            static SEND: RefCell<Option<Box<dyn Fn(Ask)>>> = const { RefCell::new(None) };
            static WINDOW: RefCell<Option<Retained<ExtWindow>>> = const { RefCell::new(None) };
            static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
            static TABS: RefCell<Vec<Retained<ExtTab>>> = const { RefCell::new(Vec::new()) };
            static ACTIVE: Cell<Option<u64>> = const { Cell::new(None) };
            /// New-tab requests waiting for the tab (`Ask::Open`'s `reply`).
            static WAITING: RefCell<HashMap<u64, block2::RcBlock<dyn Fn(*mut AnyObject, *mut AnyObject)>>> = RefCell::new(HashMap::new());
            static NEXT: Cell<u64> = const { Cell::new(1) };
        }

        fn ask(a: Ask) {
            SEND.with_borrow(|s| match s {
                Some(f) => f(a),
                None => crate::log::line("extensions", format!("nobody to ask: {a:?}")),
            });
        }

        fn active_tab() -> Option<Retained<ExtTab>> {
            let id = ACTIVE.get()?;
            TABS.with_borrow(|t| t.iter().find(|x| x.ivars().id == id).cloned())
        }

        fn window() -> Retained<ExtWindow> {
            WINDOW.with_borrow_mut(|w| {
                w.get_or_insert_with(|| {
                    let mtm = MainThreadMarker::new().expect("extensions run on the main thread");
                    let this = ExtWindow::alloc(mtm).set_ivars(());
                    unsafe { msg_send![super(this), init] }
                })
                .clone()
            })
        }

        /// Tell the engine it can rely on these objects being what WebKit's protocols describe.
        fn conform(cls: &AnyClass, protocol: &std::ffi::CStr) {
            if let Some(p) = AnyProtocol::get(protocol) {
                unsafe { objc2::ffi::class_addProtocol(cls as *const AnyClass as *mut AnyClass, p) };
            }
        }

        pub(super) fn delegate() -> Retained<Delegate> {
            DELEGATE.with_borrow_mut(|d| {
                d.get_or_insert_with(|| {
                    let mtm = MainThreadMarker::new().expect("extensions run on the main thread");
                    conform(ExtTab::class(), c"WKWebExtensionTab");
                    conform(ExtWindow::class(), c"WKWebExtensionWindow");
                    conform(Delegate::class(), c"WKWebExtensionControllerDelegate");
                    let this = Delegate::alloc(mtm).set_ivars(());
                    unsafe { msg_send![super(this), init] }
                })
                .clone()
            })
        }

        /// Where requests from extensions go (Valerie's event loop).
        pub fn connect(send: impl Fn(Ask) + 'static) {
            SEND.set(Some(Box::new(send)));
        }

        /// A tab asked for with `Ask::Open` is open (`tab` 0: it couldn't be).
        pub fn opened(reply: u64, tab: u64) {
            let Some(done) = WAITING.with_borrow_mut(|w| w.remove(&reply)) else { return };
            let t = TABS.with_borrow(|t| t.iter().find(|x| x.ivars().id == tab).cloned());
            let ptr = t.as_ref().map_or(std::ptr::null_mut(), |t| Retained::as_ptr(t) as *mut AnyObject);
            done.call((ptr, std::ptr::null_mut()));
        }

        /// Bring the engine up to date with Valerie's tabs (cheap when nothing changed). Does nothing until an
        /// extension is loaded.
        pub fn sync(seen: &[Seen], active: Option<u64>) {
            let Some(ctrl) = CONTROLLER.with_borrow(|c| c.clone()) else { return };
            let mtm = MainThreadMarker::new().expect("extensions run on the main thread");
            let win = window();
            let old: Vec<Retained<ExtTab>> = TABS.with_borrow(|t| t.clone());
            let mut now = Vec::with_capacity(seen.len());
            let (mut added, mut changed) = (Vec::new(), Vec::new());
            for s in seen {
                let view: Option<Retained<AnyObject>> = s.view.and_then(|p| unsafe { Retained::retain(p.cast::<AnyObject>()) });
                match old.iter().find(|t| t.ivars().id == s.id) {
                    Some(t) => {
                        let iv = t.ivars();
                        let mut what = 0usize;
                        if *iv.url.borrow() != s.url {
                            iv.url.replace(s.url.to_string());
                            what |= 1 << 8;
                        }
                        if *iv.title.borrow() != s.title {
                            iv.title.replace(s.title.to_string());
                            what |= 1 << 7;
                        }
                        let same_view = iv.view.borrow().load().map(|v| Retained::as_ptr(&v)) == view.as_ref().map(Retained::as_ptr);
                        if !same_view {
                            iv.view.replace(view.as_ref().map(|v| Weak::from_retained(v)).unwrap_or_default());
                            what |= 1 << 1 | 1 << 8;
                        }
                        if iv.loading.get() != s.loading {
                            iv.loading.set(s.loading);
                            what |= 1 << 1;
                        }
                        if what != 0 {
                            changed.push((t.clone(), what));
                        }
                        now.push(t.clone());
                    }
                    None => {
                        let iv = TabIvars {
                            id: s.id,
                            url: RefCell::new(s.url.to_string()),
                            title: RefCell::new(s.title.to_string()),
                            loading: Cell::new(s.loading),
                            view: RefCell::new(view.as_ref().map(|v| Weak::from_retained(v)).unwrap_or_default()),
                        };
                        let this = ExtTab::alloc(mtm).set_ivars(iv);
                        let t: Retained<ExtTab> = unsafe { msg_send![super(this), init] };
                        added.push(t.clone());
                        now.push(t);
                    }
                }
            }
            let gone: Vec<Retained<ExtTab>> = old.iter().filter(|t| !now.iter().any(|n| n.ivars().id == t.ivars().id)).cloned().collect();
            let before = active_tab();
            TABS.set(now);
            let first = ACTIVE.get().is_none() && old.is_empty();
            ACTIVE.set(active);
            unsafe {
                if first {
                    let _: () = msg_send![&*ctrl, didOpenWindow: &*win];
                    let _: () = msg_send![&*ctrl, didFocusWindow: &*win];
                }
                for t in &gone {
                    let _: () = msg_send![&*ctrl, didCloseTab: &**t, windowIsClosing: false];
                }
                for t in &added {
                    let _: () = msg_send![&*ctrl, didOpenTab: &**t];
                }
                for (t, what) in &changed {
                    let _: () = msg_send![&*ctrl, didChangeTabProperties: *what, forTab: &**t];
                }
                let after = active_tab();
                if after.as_ref().map(|t| t.ivars().id) != before.as_ref().map(|t| t.ivars().id) {
                    if let Some(a) = &after {
                        let prev = before.as_ref().filter(|b| !gone.iter().any(|g| g.ivars().id == b.ivars().id));
                        let prev: *const AnyObject = prev.map_or(std::ptr::null(), |p| Retained::as_ptr(p).cast());
                        let _: () = msg_send![&*ctrl, didActivateTab: &**a, previousActiveTab: prev];
                    }
                }
            }
        }

        /// The tab object WebKit holds for `tab` (to tie a popup to the page it's about).
        pub fn tab_object(tab: Option<u64>) -> *mut AnyObject {
            let t = tab.and_then(|id| TABS.with_borrow(|t| t.iter().find(|x| x.ivars().id == id).cloned()));
            t.map_or(std::ptr::null_mut(), |t| Retained::as_ptr(&t) as *mut AnyObject)
        }

        fn url_string(url: *mut AnyObject) -> Option<String> {
            if url.is_null() {
                return None;
            }
            let s: *mut NSString = unsafe { msg_send![url, absoluteString] };
            (!s.is_null()).then(|| unsafe { (*s).to_string() })
        }

        define_class!(
            #[unsafe(super(NSObject))]
            #[thread_kind = MainThreadOnly]
            #[name = "ValerieExtensionTab"]
            #[ivars = TabIvars]
            pub struct ExtTab;

            impl ExtTab {
                #[unsafe(method_id(windowForWebExtensionContext:))]
                fn window_for(&self, _ctx: *mut AnyObject) -> Option<Retained<AnyObject>> {
                    Some(window().into_super().into())
                }

                #[unsafe(method_id(webViewForWebExtensionContext:))]
                fn web_view(&self, _ctx: *mut AnyObject) -> Option<Retained<AnyObject>> {
                    self.ivars().view.borrow().load()
                }

                #[unsafe(method_id(titleForWebExtensionContext:))]
                fn title(&self, _ctx: *mut AnyObject) -> Option<Retained<NSString>> {
                    Some(NSString::from_str(&self.ivars().title.borrow()))
                }

                #[unsafe(method_id(urlForWebExtensionContext:))]
                fn url(&self, _ctx: *mut AnyObject) -> Option<Retained<AnyObject>> {
                    let s = NSString::from_str(&self.ivars().url.borrow());
                    unsafe { msg_send![objc2::class!(NSURL), URLWithString: &*s] }
                }

                #[unsafe(method(isLoadingCompleteForWebExtensionContext:))]
                fn loaded(&self, _ctx: *mut AnyObject) -> bool {
                    !self.ivars().loading.get()
                }

                #[unsafe(method(isSelectedForWebExtensionContext:))]
                fn selected(&self, _ctx: *mut AnyObject) -> bool {
                    ACTIVE.get() == Some(self.ivars().id)
                }

                #[unsafe(method(activateForWebExtensionContext:completionHandler:))]
                fn activate(&self, _ctx: *mut AnyObject, done: &Done) {
                    ask(Ask::Activate(self.ivars().id));
                    done.call((std::ptr::null_mut(),));
                }

                #[unsafe(method(closeForWebExtensionContext:completionHandler:))]
                fn close(&self, _ctx: *mut AnyObject, done: &Done) {
                    ask(Ask::Close(self.ivars().id));
                    done.call((std::ptr::null_mut(),));
                }
            }

            unsafe impl NSObjectProtocol for ExtTab {}
        );

        define_class!(
            #[unsafe(super(NSObject))]
            #[thread_kind = MainThreadOnly]
            #[name = "ValerieExtensionWindow"]
            pub struct ExtWindow;

            impl ExtWindow {
                #[unsafe(method_id(tabsForWebExtensionContext:))]
                fn tabs(&self, _ctx: *mut AnyObject) -> Retained<NSArray<ExtTab>> {
                    TABS.with_borrow(|t| NSArray::from_retained_slice(t))
                }

                #[unsafe(method_id(activeTabForWebExtensionContext:))]
                fn active(&self, _ctx: *mut AnyObject) -> Option<Retained<ExtTab>> {
                    active_tab()
                }

                #[unsafe(method(windowTypeForWebExtensionContext:))]
                fn kind(&self, _ctx: *mut AnyObject) -> isize {
                    0
                }

                #[unsafe(method(windowStateForWebExtensionContext:))]
                fn state(&self, _ctx: *mut AnyObject) -> isize {
                    0
                }

                #[unsafe(method(isPrivateForWebExtensionContext:))]
                fn private(&self, _ctx: *mut AnyObject) -> bool {
                    false
                }

                #[unsafe(method(focusForWebExtensionContext:completionHandler:))]
                fn focus(&self, _ctx: *mut AnyObject, done: &Done) {
                    ask(Ask::FocusWindow);
                    done.call((std::ptr::null_mut(),));
                }
            }

            unsafe impl NSObjectProtocol for ExtWindow {}
        );

        define_class!(
            #[unsafe(super(NSObject))]
            #[thread_kind = MainThreadOnly]
            #[name = "ValerieExtensionDelegate"]
            pub struct Delegate;

            impl Delegate {
                #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
                fn windows(&self, _c: *mut AnyObject, _ctx: *mut AnyObject) -> Retained<NSArray<ExtWindow>> {
                    NSArray::from_retained_slice(&[window()])
                }

                #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
                fn focused(&self, _c: *mut AnyObject, _ctx: *mut AnyObject) -> Option<Retained<ExtWindow>> {
                    Some(window())
                }

                #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
                fn new_tab(&self, _c: *mut AnyObject, config: *mut AnyObject, _ctx: *mut AnyObject, done: &DoneWith) {
                    let url: *mut AnyObject = unsafe { msg_send![config, url] };
                    let active: bool = unsafe { msg_send![config, shouldBeActive] };
                    let reply = NEXT.get();
                    NEXT.set(reply + 1);
                    WAITING.with_borrow_mut(|w| w.insert(reply, done.copy()));
                    ask(Ask::Open { url: url_string(url).unwrap_or_else(|| "about:blank".into()), active, reply });
                }

                // A "window" of its own becomes tabs in Valerie's window.
                #[unsafe(method(webExtensionController:openNewWindowUsingConfiguration:forExtensionContext:completionHandler:))]
                fn new_window(&self, _c: *mut AnyObject, config: *mut AnyObject, _ctx: *mut AnyObject, done: &DoneWith) {
                    let urls: *mut AnyObject = unsafe { msg_send![config, tabURLs] };
                    let n: usize = if urls.is_null() { 0 } else { unsafe { msg_send![urls, count] } };
                    for i in 0..n {
                        let u: *mut AnyObject = unsafe { msg_send![urls, objectAtIndex: i] };
                        if let Some(url) = url_string(u) {
                            ask(Ask::Open { url, active: i == 0, reply: 0 });
                        }
                    }
                    let w = window();
                    done.call((Retained::as_ptr(&w) as *mut AnyObject, std::ptr::null_mut()));
                }

                #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
                fn popup(&self, _c: *mut AnyObject, _action: *mut AnyObject, ctx: *mut AnyObject, done: &Done) {
                    match name_of(ctx) {
                        Some(name) => ask(Ask::Popup(name)),
                        None => crate::log::line("extensions", "a popup from an extension that isn't loaded"),
                    }
                    done.call((std::ptr::null_mut(),));
                }
            }

            unsafe impl NSObjectProtocol for Delegate {}
        );
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
                // WebKit keeps an extension's storage under its identifier, which is random unless set: the same one every
                // launch keeps what it saved (a password manager's sign-in, say).
                let id: Retained<NSString> = msg_send![&*uuid(&format!("extension:{name}")), UUIDString];
                let _: () = msg_send![&*ctx, setUniqueIdentifier: &*id];
                let base = NSString::from_str(&format!("webkit-extension://{id}/"));
                let base: *mut AnyObject = msg_send![objc2::class!(NSURL), URLWithString: &*base];
                let _: () = msg_send![&*ctx, setBaseURL: base];
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

    /// Stop a loaded extension. Pages built while it ran need rebuilding to lose what it already put in them.
    pub fn unload(name: &str) -> Result<(), String> {
        let Some(i) = LOADED.with(|l| l.borrow().iter().position(|(n, _)| n == name)) else { return Ok(()) };
        let ctrl = controller().ok_or("this macOS can't run browser extensions")?;
        let ctx = LOADED.with(|l| l.borrow()[i].1.clone());
        let mut error: *mut AnyObject = std::ptr::null_mut();
        unsafe {
            let ok: bool = msg_send![&*ctrl, unloadExtensionContext: &*ctx, error: &mut error];
            if !ok {
                return Err(describe(error));
            }
        }
        LOADED.with(|l| l.borrow_mut().remove(i));
        Ok(())
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

    /// The configuration a page of a loaded extension must be built with (WebKit refuses extension pages in any other).
    pub fn page_configuration_for(url: &str) -> Option<Retained<AnyObject>> {
        let host = super::extension_host(url)?;
        let ctx = LOADED.with(|l| {
            l.borrow().iter().find(|(_, c)| unsafe {
                let id: *mut NSString = msg_send![&**c, uniqueIdentifier];
                !id.is_null() && (*id).to_string().eq_ignore_ascii_case(&host)
            }).map(|(_, c)| c.clone())
        })?;
        unsafe { msg_send![&*ctx, webViewConfiguration] }
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

    /// Show the extension's popup for the `active` tab next to `x, y` in the view (or click its button if it has no
    /// popup). `toggle`: close it when it's already showing.
    pub fn show_action(name: &str, active: Option<u64>, view: *mut AnyObject, x: f64, y: f64, toggle: bool) -> Result<(), String> {
        let ctx = context(name).ok_or("that extension isn't loaded")?;
        let tab = bridge::tab_object(active);
        unsafe {
            let action: *mut AnyObject = msg_send![&*ctx, actionForTab: tab];
            if action.is_null() {
                return Err("it has no button".into());
            }
            let popup: bool = msg_send![action, presentsPopup];
            if !popup {
                let _: () = msg_send![&*ctx, performActionForTab: tab];
                return Ok(());
            }
            let popover: *mut AnyObject = msg_send![action, popupPopover];
            if popover.is_null() {
                return Err("its popup couldn't be opened".into());
            }
            let shown: bool = msg_send![popover, isShown];
            if shown {
                if toggle {
                    let _: () = msg_send![popover, close];
                }
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
    pub fn unload(_name: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn options_url(_name: &str) -> Option<String> {
        None
    }
    pub fn connect(_send: impl Fn(super::Ask) + 'static) {}
    pub fn opened(_reply: u64, _tab: u64) {}
    pub fn sync(_seen: &[super::Seen], _active: Option<u64>) {}
    pub fn log_errors() {}
}

#[cfg(not(target_os = "macos"))]
pub use other::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_on_edits_the_list_and_keeps_the_rest_of_the_file() {
        let _guard = crate::config::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("valerie-ext-switch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("VALERIE_CONFIG_DIR", &dir);
        let file = dir.join("config.yaml");
        std::fs::write(&file, "# mine\nextensions:\n  enabled: true\n").unwrap();
        let read = || -> crate::config::ExtensionsConfig { crate::config::Config::load().unwrap().extensions };

        set_on(&read(), "a", false).unwrap();
        set_on(&read(), "b", false).unwrap();
        set_on(&read(), "b", false).unwrap();
        assert_eq!(read().turned_off, ["a", "b"]);
        set_on(&read(), "a", true).unwrap();
        assert_eq!(read().turned_off, ["b"]);
        assert!(std::fs::read_to_string(&file).unwrap().starts_with("# mine\n"));

        std::env::remove_var("VALERIE_CONFIG_DIR");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
