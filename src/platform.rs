//! Small OS-specific touches that wry/tao don't expose.
use wry::WebView;

/// Stable per-profile id; on macOS it names an isolated WKWebsiteDataStore.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn profile_id(name: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (half, seed) in [(0usize, 0xcbf29ce484222325u64), (8, 0x84222325cbf29ce4)] {
        let mut h = seed;
        for b in name.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        out[half..half + 8].copy_from_slice(&h.to_le_bytes());
    }
    out[6] = (out[6] & 0x0f) | 0x40; // RFC 4122 version 4 / variant bits so it is a well-formed UUID
    out[8] = (out[8] & 0x3f) | 0x80;
    out
}

/// Open a URL or file with the OS default handler.
pub fn open_external(target: &str) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(target).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", target]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(target).spawn();
    if let Err(e) = r {
        crate::log::line("open", format!("could not open {target}: {e}"));
    }
}

/// Terminal apps a terminal editor can open in (the `terminal` setting).
pub const TERMINALS: &[&str] = &["Terminal", "iTerm", "Ghostty", "WezTerm", "kitty", "Alacritty"];
const TERMINAL_EDITORS: &[&str] = &["vim", "vi", "nvim", "nano", "emacs", "hx", "helix", "micro", "kak", "pico", "ed"];

/// `editor` as the command to run in a terminal, if it's a terminal editor.
fn terminal_command(editor: &str) -> Option<&str> {
    let e = editor.trim();
    if let Some(cmd) = e.strip_prefix("terminal:") {
        return Some(cmd.trim());
    }
    TERMINAL_EDITORS.contains(&e.split_whitespace().next().unwrap_or("")).then_some(e)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Open a text file the way the `editor` setting says: the default app, an app by name, or a terminal editor
/// running in the `terminal` app.
pub fn open_in_editor(path: &std::path::Path, editor: &str, terminal: &str) -> Result<(), String> {
    use std::process::Command;
    let file = path.to_string_lossy().to_string();
    let ran = |r: std::io::Result<std::process::ExitStatus>| match r {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err("it isn't installed".to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Some(cmd) = terminal_command(editor) {
        let line = format!("{cmd} {}", shell_quote(&file));
        #[cfg(target_os = "macos")]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let osa = |script: String| ran(Command::new("osascript").args(["-e", &script]).status());
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            return match terminal.to_lowercase().as_str() {
                "iterm" | "iterm2" => osa(format!(
                    "tell application \"iTerm\"\nactivate\ncreate window with default profile command \"{}\"\nend tell",
                    esc(&format!("{shell} -lc {}", shell_quote(&line)))
                )),
                "ghostty" => ran(Command::new("open").args(["-na", "Ghostty", "--args", "-e", &shell, "-lc", &line]).status()),
                "wezterm" => ran(Command::new("open").args(["-na", "WezTerm", "--args", "start", "--", &shell, "-lc", &line]).status()),
                "kitty" => ran(Command::new("open").args(["-na", "kitty", "--args", &shell, "-lc", &line]).status()),
                "alacritty" => ran(Command::new("open").args(["-na", "Alacritty", "--args", "-e", &shell, "-lc", &line]).status()),
                _ => osa(format!("tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell", esc(&line))),
            };
        }
        #[cfg(target_os = "windows")]
        return ran(Command::new("cmd").args(["/C", "start", "", "cmd", "/K", &line]).status());
        #[cfg(all(unix, not(target_os = "macos")))]
        return ran(Command::new("x-terminal-emulator").args(["-e", "sh", "-c", &line]).status());
    }
    let _ = terminal;
    match editor.trim().to_lowercase().as_str() {
        "default" | "" => {
            #[cfg(target_os = "macos")]
            {
                // No app set for .yaml files: fall back to the plain text editor.
                if !Command::new("open").arg(path).status().is_ok_and(|s| s.success()) {
                    return ran(Command::new("open").arg("-t").arg(path).status());
                }
                Ok(())
            }
            #[cfg(not(target_os = "macos"))]
            {
                open_external(&file);
                Ok(())
            }
        }
        "textedit" if cfg!(target_os = "macos") => ran(Command::new("open").args(["-a", "TextEdit"]).arg(path).status()),
        _ => {
            #[cfg(target_os = "macos")]
            return ran(Command::new("open").args(["-a", editor.trim()]).arg(path).status());
            #[cfg(not(target_os = "macos"))]
            {
                let mut words = editor.split_whitespace();
                let prog = words.next().unwrap_or_default();
                Command::new(prog).args(words).arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use wry::{WebView, WebViewExtMacOS};

    fn ptr(wv: &WebView) -> *mut AnyObject {
        let v = wv.webview();
        objc2::rc::Retained::as_ptr(&v) as *mut AnyObject
    }

    /// Re-adding a subview moves it to the top of its siblings.
    pub fn bring_to_front(wv: &WebView) {
        let v = ptr(wv);
        unsafe {
            let sup: *mut AnyObject = msg_send![v, superview];
            if !sup.is_null() {
                let _: () = msg_send![sup, addSubview: v, positioned: 1isize, relativeTo: std::ptr::null_mut::<AnyObject>()];
            }
        }
    }

    pub fn round_corners(wv: &WebView, radius: f64, corners: super::Corners) {
        // CACornerMask bits: MinXMinY=1, MaxXMinY=2, MinXMaxY=4, MaxXMaxY=8.
        let v = ptr(wv);
        unsafe {
            let _: () = msg_send![v, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![v, layer];
            if layer.is_null() {
                return;
            }
            let flipped: bool = msg_send![layer, isGeometryFlipped];
            let top = if flipped { 1usize | 2 } else { 4usize | 8 };
            let mask = match corners {
                super::Corners::None => 0,
                super::Corners::All => 15,
                super::Corners::Left => 1 | 4,
                super::Corners::Right => 2 | 8,
                super::Corners::Top => top,
            };
            let r = if mask == 0 { 0.0 } else { radius };
            let _: () = msg_send![layer, setCornerRadius: r];
            let _: () = msg_send![layer, setMaskedCorners: mask];
            let _: () = msg_send![layer, setMasksToBounds: r > 0.0];
        }
    }

    /// Run `f` inside an NSAnimationContext group so `animator` property changes ease over `secs`.
    unsafe fn animate(secs: f64, f: impl FnOnce()) {
        use objc2::class;
        let ctx_cls = class!(NSAnimationContext);
        let _: () = msg_send![ctx_cls, beginGrouping];
        let ctx: *mut AnyObject = msg_send![ctx_cls, currentContext];
        let _: () = msg_send![ctx, setDuration: secs];
        let name = objc2_foundation::NSString::from_str("easeInEaseOut");
        let tf: *mut AnyObject = msg_send![class!(CAMediaTimingFunction), functionWithName: &*name];
        let _: () = msg_send![ctx, setTimingFunction: tf];
        f();
        let _: () = msg_send![ctx_cls, endGrouping];
    }

    /// Same coordinate maths as wry's set_bounds, but through the view's animator.
    pub fn animate_bounds(wv: &WebView, x: f64, y: f64, w: f64, h: f64, secs: f64) -> bool {
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        let v = ptr(wv);
        unsafe {
            let sup: *mut AnyObject = msg_send![v, superview];
            if sup.is_null() {
                return false;
            }
            let flipped: bool = msg_send![sup, isFlipped];
            let frame: NSRect = msg_send![sup, frame];
            let oy = if flipped { y } else { frame.size.height - y - h };
            let target = NSRect::new(NSPoint::new(x, oy), NSSize::new(w.round(), h.round()));
            animate(secs, || {
                let animator: *mut AnyObject = msg_send![v, animator];
                let _: () = msg_send![animator, setFrame: target];
            });
        }
        true
    }

    pub fn set_alpha(wv: &WebView, alpha: f64, secs: f64) {
        let v = ptr(wv);
        unsafe {
            animate(secs, || {
                let animator: *mut AnyObject = msg_send![v, animator];
                let _: () = msg_send![animator, setAlphaValue: alpha];
            });
        }
    }

    pub fn fade_in(wv: &WebView, secs: f64) {
        let v = ptr(wv);
        unsafe {
            let _: () = msg_send![v, setAlphaValue: 0.0f64];
            animate(secs, || {
                let animator: *mut AnyObject = msg_send![v, animator];
                let _: () = msg_send![animator, setAlphaValue: 1.0f64];
            });
        }
    }

    pub fn snapshot(wv: &WebView, width: f64, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
        use objc2::class;
        let v = ptr(wv);
        let done = std::cell::RefCell::new(Some(done));
        let block = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
            let Some(done) = done.borrow_mut().take() else { return };
            if image.is_null() {
                return done(Err("the page could not be captured (is the tab loaded?)".into()));
            }
            unsafe {
                let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
                let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
                let props: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
                // NSBitmapImageFileTypePNG = 4
                let png: *mut AnyObject = msg_send![rep, representationUsingType: 4usize, properties: props];
                if png.is_null() {
                    return done(Err("PNG encoding failed".into()));
                }
                let len: usize = msg_send![png, length];
                let bytes: *const u8 = msg_send![png, bytes];
                done(Ok(std::slice::from_raw_parts(bytes, len).to_vec()))
            }
        });
        unsafe {
            let cfg: *mut AnyObject = msg_send![class!(WKSnapshotConfiguration), new];
            let n: *mut AnyObject = msg_send![class!(NSNumber), numberWithDouble: width];
            let _: () = msg_send![cfg, setSnapshotWidth: n];
            let _: () = msg_send![v, takeSnapshotWithConfiguration: cfg, completionHandler: &*block];
            let _: () = msg_send![cfg, release];
        }
    }

    /// Show alert / confirm / prompt as proper dialogs. WebKit sends them to the UI delegate, and wry's delegate
    /// doesn't answer them, so `alert()` silently vanished and `confirm()` always said Cancel. The methods are added
    /// to wry's delegate class once, after the first webview exists.
    pub fn install_page_dialogs(wv: &WebView) {
        use objc2::runtime::{AnyClass, Bool, Imp, Sel};
        use objc2::sel;
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            // wry's delegate class (its runtime name isn't fixed, so take it from a live webview).
            let delegate: *mut AnyObject = msg_send![ptr(wv), UIDelegate];
            if delegate.is_null() {
                return;
            }
            let cls = (*delegate).class();
            let cls = cls as *const AnyClass as *mut AnyClass;
            type Done0 = block2::Block<dyn Fn()>;
            type DoneBool = block2::Block<dyn Fn(Bool)>;
            type DoneText = block2::Block<dyn Fn(*mut objc2_foundation::NSString)>;
            unsafe extern "C-unwind" fn alert(_: *mut AnyObject, _: Sel, wv: *mut AnyObject, msg: *mut objc2_foundation::NSString, frame: *mut AnyObject, done: &Done0) {
                let done = done.copy();
                sheet(wv, msg, frame, &["OK"], None, move |_, _| done.call(()));
            }
            unsafe extern "C-unwind" fn confirm(_: *mut AnyObject, _: Sel, wv: *mut AnyObject, msg: *mut objc2_foundation::NSString, frame: *mut AnyObject, done: &DoneBool) {
                let done = done.copy();
                sheet(wv, msg, frame, &["OK", "Cancel"], None, move |r, _| done.call((Bool::new(r == 1000),)));
            }
            unsafe extern "C-unwind" fn prompt(
                _: *mut AnyObject,
                _: Sel,
                wv: *mut AnyObject,
                msg: *mut objc2_foundation::NSString,
                default: *mut objc2_foundation::NSString,
                frame: *mut AnyObject,
                done: &DoneText,
            ) {
                let d = if default.is_null() { String::new() } else { (*default).to_string() };
                let done = done.copy();
                sheet(wv, msg, frame, &["OK", "Cancel"], Some(&d), move |r, text| match text.filter(|_| r == 1000) {
                    Some(t) => done.call((objc2::rc::Retained::autorelease_ptr(objc2_foundation::NSString::from_str(&t)),)),
                    None => done.call((std::ptr::null_mut(),)),
                });
            }
            let add = |name: Sel, imp: Imp, types: &std::ffi::CStr| {
                objc2::ffi::class_addMethod(cls, name, imp, types.as_ptr());
            };
            add(sel!(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:), std::mem::transmute::<*const (), Imp>(alert as *const ()), c"v@:@@@@?");
            add(sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:), std::mem::transmute::<*const (), Imp>(confirm as *const ()), c"v@:@@@@?");
            add(
                sel!(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:),
                std::mem::transmute::<*const (), Imp>(prompt as *const ()),
                c"v@:@@@@@?",
            );
        });
    }

    /// An NSAlert sheet on the page's window, titled with the page's site (a sheet doesn't freeze the rest of
    /// Valerie the way a modal alert would). `reply` gets the button (1000 = first) and any typed text.
    unsafe fn sheet(
        webview: *mut AnyObject,
        msg: *mut objc2_foundation::NSString,
        frame: *mut AnyObject,
        buttons: &[&str],
        input: Option<&str>,
        reply: impl FnOnce(isize, Option<String>) + 'static,
    ) {
        sheet_titled(webview, msg, frame, None, buttons, input, reply)
    }

    unsafe fn sheet_titled(
        webview: *mut AnyObject,
        msg: *mut objc2_foundation::NSString,
        frame: *mut AnyObject,
        heading: Option<&str>,
        buttons: &[&str],
        input: Option<&str>,
        reply: impl FnOnce(isize, Option<String>) + 'static,
    ) {
        use objc2::class;
        use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
        let host: Option<String> = (|| {
            let origin: *mut AnyObject = if frame.is_null() { std::ptr::null_mut() } else { msg_send![frame, securityOrigin] };
            let h: *mut NSString = if origin.is_null() { std::ptr::null_mut() } else { msg_send![origin, host] };
            (!h.is_null()).then(|| (*h).to_string()).filter(|s| !s.is_empty())
        })();
        let alert: *mut AnyObject = msg_send![class!(NSAlert), new];
        let title = heading.map(str::to_string).unwrap_or_else(|| host.map_or("This page says".to_string(), |h| format!("{} says", h.trim_start_matches("www."))));
        let _: () = msg_send![alert, setMessageText: &*NSString::from_str(&title)];
        let body = if msg.is_null() { String::new() } else { (*msg).to_string() };
        let _: () = msg_send![alert, setInformativeText: &*NSString::from_str(&body)];
        for b in buttons {
            let _: *mut AnyObject = msg_send![alert, addButtonWithTitle: &*NSString::from_str(b)];
        }
        let field: *mut AnyObject = match input {
            Some(d) => {
                let f: *mut AnyObject = msg_send![class!(NSTextField), alloc];
                let f: *mut AnyObject = msg_send![f, initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(280.0, 24.0))];
                let _: () = msg_send![f, setStringValue: &*NSString::from_str(d)];
                let _: () = msg_send![alert, setAccessoryView: f];
                let win: *mut AnyObject = msg_send![alert, window];
                let _: () = msg_send![win, setInitialFirstResponder: f];
                f
            }
            None => std::ptr::null_mut(),
        };
        let reply = std::cell::RefCell::new(Some(reply));
        let finish = move |code: isize| {
            let text = (!field.is_null()).then(|| {
                let s: *mut NSString = msg_send![field, stringValue];
                if s.is_null() { String::new() } else { (*s).to_string() }
            });
            if let Some(r) = reply.borrow_mut().take() {
                r(code, text);
            }
            let _: () = msg_send![alert, release];
        };
        let window: *mut AnyObject = if webview.is_null() { std::ptr::null_mut() } else { msg_send![webview, window] };
        if window.is_null() {
            let code: isize = msg_send![alert, runModal];
            finish(code);
        } else {
            let block = block2::RcBlock::new(move |code: isize| finish(code));
            let _: () = msg_send![alert, beginSheetModalForWindow: window, completionHandler: &*block];
        }
    }

    pub fn print(wv: &WebView) {
        use objc2::class;
        unsafe {
            let v = ptr(wv);
            let info: *mut AnyObject = msg_send![class!(NSPrintInfo), sharedPrintInfo];
            let op: *mut AnyObject = msg_send![v, printOperationWithPrintInfo: info];
            let win: *mut AnyObject = msg_send![v, window];
            if op.is_null() || win.is_null() {
                return;
            }
            // WebKit's print operation needs a frame to lay the page out into.
            let view: *mut AnyObject = msg_send![op, view];
            let bounds: objc2_foundation::NSRect = msg_send![v, bounds];
            let _: () = msg_send![view, setFrame: bounds];
            let _: () = msg_send![op, runOperationModalForWindow: win, delegate: std::ptr::null_mut::<AnyObject>(), didRunSelector: None::<objc2::runtime::Sel>, contextInfo: std::ptr::null_mut::<std::ffi::c_void>()];
        }
    }

    // ---------------------------------------------------------------- notifications (UserNotifications.framework)

    #[link(name = "UserNotifications", kind = "framework")]
    extern "C" {}

    static CLICKED: std::sync::OnceLock<Box<dyn Fn(String) + Send + Sync>> = std::sync::OnceLock::new();
    static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// Only an app bundle (with a bundle id) may use the notification center; a bare binary would crash in it.
    fn bundled() -> bool {
        use objc2::class;
        unsafe {
            let b: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
            let id: *mut objc2_foundation::NSString = msg_send![b, bundleIdentifier];
            !id.is_null()
        }
    }

    pub fn notifications_init(on_click: Box<dyn Fn(String) + Send + Sync>) {
        use objc2::runtime::{AnyClass, Bool, Imp, Sel};
        use objc2::{class, sel};
        let _ = CLICKED.set(on_click);
        if !bundled() {
            return;
        }
        type Done0 = block2::Block<dyn Fn()>;
        type DoneOpts = block2::Block<dyn Fn(usize)>;
        // Clicked: hand the notification's action back to Valerie.
        unsafe extern "C-unwind" fn did_receive(_: *mut AnyObject, _: Sel, _: *mut AnyObject, response: *mut AnyObject, done: &Done0) {
            let note: *mut AnyObject = msg_send![response, notification];
            let req: *mut AnyObject = msg_send![note, request];
            let content: *mut AnyObject = msg_send![req, content];
            let info: *mut AnyObject = msg_send![content, userInfo];
            let key = objc2_foundation::NSString::from_str("action");
            let v: *mut objc2_foundation::NSString = if info.is_null() { std::ptr::null_mut() } else { msg_send![info, objectForKey: &*key] };
            if !v.is_null() {
                if let Some(f) = CLICKED.get() {
                    f((*v).to_string());
                }
            }
            done.call(());
        }
        // Show banners even while Valerie is the front app (macOS hides them by default).
        unsafe extern "C-unwind" fn will_present(_: *mut AnyObject, _: Sel, _: *mut AnyObject, _: *mut AnyObject, done: &DoneOpts) {
            done.call((0b11010,)); // banner | list | sound
        }
        unsafe {
            let name = c"ValerieNotificationDelegate";
            let cls: *const AnyClass = match AnyClass::get(name) {
                Some(c) => c,
                None => {
                    let c = objc2::ffi::objc_allocateClassPair(class!(NSObject), name.as_ptr(), 0);
                    if c.is_null() {
                        return;
                    }
                    let add = |s: Sel, f: *const (), types: &std::ffi::CStr| {
                        objc2::ffi::class_addMethod(c, s, std::mem::transmute::<*const (), Imp>(f), types.as_ptr());
                    };
                    add(sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:), did_receive as *const (), c"v@:@@@?");
                    add(sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:), will_present as *const (), c"v@:@@@?");
                    objc2::ffi::objc_registerClassPair(c);
                    c
                }
            };
            let center: *mut AnyObject = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
            let delegate: *mut AnyObject = msg_send![cls, new]; // the center holds it weakly, so it lives for the app's life
            let _: () = msg_send![center, setDelegate: delegate];
            let asked = block2::RcBlock::new(|granted: Bool, err: *mut AnyObject| {
                READY.store(granted.as_bool(), std::sync::atomic::Ordering::Relaxed);
                if !granted.as_bool() {
                    let why: *mut objc2_foundation::NSString = if err.is_null() { std::ptr::null_mut() } else { msg_send![err, localizedDescription] };
                    let why = if why.is_null() { "turned off in System Settings > Notifications".to_string() } else { (*why).to_string() };
                    crate::log::line("notifications", format!("notifications aren't allowed ({why}); using the fallback"));
                }
            });
            // alert | sound
            let _: () = msg_send![center, requestAuthorizationWithOptions: 6usize, completionHandler: &*asked];
        }
    }

    /// The older notification API, for when macOS refuses the current one (it can remember a refusal for an app's ID
    /// for good). It still shows as Valerie, with its icon; the AppleScript fallback shows Script Editor's.
    pub fn notify_legacy(n: &super::Note) -> bool {
        use objc2::class;
        use objc2::runtime::{AnyClass, Bool, Imp, Sel};
        use objc2::sel;
        use objc2_foundation::NSString;
        if !bundled() || AnyClass::get(c"NSUserNotificationCenter").is_none() {
            return false;
        }
        // Clicked: hand the notification's action back to Valerie. Shown even while Valerie is in front.
        unsafe extern "C-unwind" fn activated(_: *mut AnyObject, _: Sel, _: *mut AnyObject, note: *mut AnyObject) {
            let info: *mut AnyObject = msg_send![note, userInfo];
            let key = objc2_foundation::NSString::from_str("action");
            let v: *mut objc2_foundation::NSString = if info.is_null() { std::ptr::null_mut() } else { msg_send![info, objectForKey: &*key] };
            if !v.is_null() {
                if let Some(f) = CLICKED.get() {
                    f((*v).to_string());
                }
            }
        }
        unsafe extern "C-unwind" fn present(_: *mut AnyObject, _: Sel, _: *mut AnyObject, _: *mut AnyObject) -> Bool {
            Bool::YES
        }
        unsafe {
            let center: *mut AnyObject = msg_send![class!(NSUserNotificationCenter), defaultUserNotificationCenter];
            if center.is_null() {
                return false;
            }
            let has_delegate: *mut AnyObject = msg_send![center, delegate];
            if has_delegate.is_null() {
                let name = c"ValerieLegacyNotificationDelegate";
                let cls: Option<&AnyClass> = AnyClass::get(name).or_else(|| {
                    let c = objc2::ffi::objc_allocateClassPair(class!(NSObject), name.as_ptr(), 0);
                    if c.is_null() {
                        return None;
                    }
                    let add = |s: Sel, f: *const (), types: &std::ffi::CStr| {
                        objc2::ffi::class_addMethod(c, s, std::mem::transmute::<*const (), Imp>(f), types.as_ptr());
                    };
                    add(sel!(userNotificationCenter:didActivateNotification:), activated as *const (), c"v@:@@");
                    add(sel!(userNotificationCenter:shouldPresentNotification:), present as *const (), c"c@:@@");
                    objc2::ffi::objc_registerClassPair(c);
                    AnyClass::get(name)
                });
                if let Some(cls) = cls {
                    let delegate: *mut AnyObject = msg_send![cls, new]; // held weakly by the center: lives for the app's life
                    let _: () = msg_send![center, setDelegate: delegate];
                }
            }
            let note: *mut AnyObject = msg_send![class!(NSUserNotification), new];
            let _: () = msg_send![note, setTitle: &*NSString::from_str(n.title)];
            if !n.subtitle.is_empty() {
                let _: () = msg_send![note, setSubtitle: &*NSString::from_str(n.subtitle)];
            }
            let _: () = msg_send![note, setInformativeText: &*NSString::from_str(n.body)];
            if n.sound {
                let _: () = msg_send![note, setSoundName: &*NSString::from_str("DefaultSoundName")];
            }
            if let Some(a) = &n.action {
                let (k, v) = (NSString::from_str("action"), NSString::from_str(a));
                let info: *mut AnyObject = msg_send![class!(NSDictionary), dictionaryWithObject: &*v, forKey: &*k];
                let _: () = msg_send![note, setUserInfo: info];
            }
            let _: () = msg_send![center, deliverNotification: note];
            let _: () = msg_send![note, release];
        }
        true
    }

    /// Post through the notification center; false when that isn't available (not an app, or not allowed).
    pub fn notify_native(n: &super::Note) -> bool {
        use objc2::class;
        use objc2_foundation::NSString;
        if !bundled() || !READY.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        unsafe {
            let content: *mut AnyObject = msg_send![class!(UNMutableNotificationContent), new];
            let _: () = msg_send![content, setTitle: &*NSString::from_str(n.title)];
            if !n.subtitle.is_empty() {
                let _: () = msg_send![content, setSubtitle: &*NSString::from_str(n.subtitle)];
            }
            let _: () = msg_send![content, setBody: &*NSString::from_str(n.body)];
            if n.sound {
                let s: *mut AnyObject = msg_send![class!(UNNotificationSound), defaultSound];
                let _: () = msg_send![content, setSound: s];
            }
            if let Some(a) = &n.action {
                let (k, v) = (NSString::from_str("action"), NSString::from_str(a));
                let info: *mut AnyObject = msg_send![class!(NSDictionary), dictionaryWithObject: &*v, forKey: &*k];
                let _: () = msg_send![content, setUserInfo: info];
            }
            let id = NSString::from_str(&crate::mcp::random_hex());
            let req: *mut AnyObject = msg_send![class!(UNNotificationRequest), requestWithIdentifier: &*id, content: content, trigger: std::ptr::null_mut::<AnyObject>()];
            let center: *mut AnyObject = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
            let _: () = msg_send![center, addNotificationRequest: req, withCompletionHandler: std::ptr::null::<block2::Block<dyn Fn(*mut AnyObject)>>()];
            let _: () = msg_send![content, release];
        }
        true
    }

    /// A yes/no question as a sheet on the webview's window; `done(true)` for the first button.
    pub fn ask(wv: &WebView, title: &str, body: &str, yes: &str, no: &str, done: impl FnOnce(bool) + 'static) {
        let msg = objc2_foundation::NSString::from_str(body);
        unsafe {
            let m = objc2::rc::Retained::as_ptr(&msg) as *mut objc2_foundation::NSString;
            sheet_titled(ptr(wv), m, std::ptr::null_mut(), Some(title), &[yes, no], None, move |r, _| done(r == 1000));
        }
    }

    /// A question with several answers, as a sheet; `done` gets the index of the button (the first is the default).
    pub fn choose(wv: &WebView, title: &str, body: &str, buttons: &[&str], done: impl FnOnce(usize) + 'static) {
        let msg = objc2_foundation::NSString::from_str(body);
        unsafe {
            let m = objc2::rc::Retained::as_ptr(&msg) as *mut objc2_foundation::NSString;
            sheet_titled(ptr(wv), m, std::ptr::null_mut(), Some(title), buttons, None, move |r, _| done((r - 1000).max(0) as usize));
        }
    }

    pub fn has_entitlement(name: &str) -> bool {
        use std::ffi::{c_char, c_void, CString};
        #[link(name = "Security", kind = "framework")]
        extern "C" {
            fn SecTaskCreateFromSelf(allocator: *const c_void) -> *const c_void;
            fn SecTaskCopyValueForEntitlement(task: *const c_void, entitlement: *const c_void, error: *mut *const c_void) -> *const c_void;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> *const c_void;
            fn CFRelease(cf: *const c_void);
            static kCFBooleanTrue: *const c_void;
        }
        let Ok(name) = CString::new(name) else { return false };
        unsafe {
            let task = SecTaskCreateFromSelf(std::ptr::null());
            if task.is_null() {
                return false;
            }
            let key = CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), 0x0800_0100);
            let value = SecTaskCopyValueForEntitlement(task, key, std::ptr::null_mut());
            let yes = !value.is_null() && value == kCFBooleanTrue;
            if !value.is_null() {
                CFRelease(value);
            }
            CFRelease(key);
            CFRelease(task);
            yes
        }
    }

    pub fn merge_bare_logins(marker: &std::path::Path, profiles: &[String]) {
        use objc2::class;
        use objc2_foundation::NSString;
        if marker.exists() || !bundled() {
            return;
        }
        let Some(lib) = dirs::home_dir().map(|h| h.join("Library/WebKit")) else { return };
        let (old, new) = (lib.join("valerie/WebsiteDataStore"), lib.join("dev.valerie.browser/WebsiteDataStore"));
        if !old.is_dir() {
            let _ = std::fs::write(marker, "nothing to bring over\n");
            return;
        }
        // The bare binary must not be running, or its files may be half-written.
        // (Matched on each process's program path, not its arguments.)
        let bare_running = std::process::Command::new("ps").args(["-Ao", "comm="]).output().is_ok_and(|o| {
            String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim_end().ends_with("/target/release/valerie") || l.trim_end().ends_with("/target/debug/valerie"))
        });
        if bare_running {
            return;
        }
        let mut report = String::new();
        // Only the profiles in the user's settings.
        let wanted: Vec<[u8; 16]> = profiles.iter().map(|p| super::profile_id(p)).collect();
        for entry in std::fs::read_dir(&old).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(uuid) = uuid_bytes(&name) else { continue };
            if !wanted.contains(&uuid) {
                continue;
            }
            let dest = new.join(&name);
            if !dest.exists() {
                let ok = std::process::Command::new("ditto").arg(entry.path()).arg(&dest).status().is_ok_and(|s| s.success());
                report.push_str(&format!("{name}: copied ({})\n", if ok { "ok" } else { "failed" }));
                continue;
            }
            let cookies = super::read_binary_cookies(&std::fs::read(entry.path().join("Cookies/Cookies.binarycookies")).unwrap_or_default());
            let now = crate::state::now() as f64;
            let cookies: Vec<super::Cookie> = cookies.into_iter().filter(|c| c.expires > now).collect();
            report.push_str(&format!("{name}: merging {} cookies\n", cookies.len()));
            if cookies.is_empty() {
                continue;
            }
            unsafe {
                let id: *mut AnyObject = msg_send![class!(NSUUID), alloc];
                let id: *mut AnyObject = msg_send![id, initWithUUIDBytes: uuid.as_ptr()];
                let store: *mut AnyObject = msg_send![class!(WKWebsiteDataStore), dataStoreForIdentifier: id];
                if store.is_null() {
                    continue;
                }
                let _: *mut AnyObject = msg_send![store, retain]; // lives until the async merge is done (once per launch)
                let jar: *mut AnyObject = msg_send![store, httpCookieStore];
                let pending = std::cell::RefCell::new(Some(cookies));
                let merge = block2::RcBlock::new(move |existing: *mut AnyObject| {
                    let Some(cookies) = pending.borrow_mut().take() else { return };
                    let mut have: std::collections::HashMap<(String, String, String), f64> = Default::default();
                    let n: usize = if existing.is_null() { 0 } else { msg_send![existing, count] };
                    let s = |o: *mut AnyObject, sel: &str| -> String {
                        let v: *mut NSString = match sel {
                            "domain" => msg_send![o, domain],
                            "name" => msg_send![o, name],
                            _ => msg_send![o, path],
                        };
                        if v.is_null() { String::new() } else { (*v).to_string() }
                    };
                    for i in 0..n {
                        let c: *mut AnyObject = msg_send![existing, objectAtIndex: i];
                        let date: *mut AnyObject = msg_send![c, expiresDate];
                        let exp: f64 = if date.is_null() { f64::MAX } else { msg_send![date, timeIntervalSince1970] };
                        have.insert((s(c, "domain"), s(c, "name"), s(c, "path")), exp);
                    }
                    for c in cookies {
                        if have.get(&(c.domain.clone(), c.name.clone(), c.path.clone())).is_some_and(|e| *e >= c.expires) {
                            continue;
                        }
                        let cookie = ns_cookie(&c);
                        if !cookie.is_null() {
                            let _: () = msg_send![jar, setCookie: cookie, completionHandler: std::ptr::null::<block2::Block<dyn Fn()>>()];
                        }
                    }
                });
                let _: () = msg_send![jar, getAllCookies: &*merge];
            }
        }
        let _ = std::fs::write(marker, report);
    }

    /// An autoreleased NSHTTPCookie (null if Foundation rejects it).
    unsafe fn ns_cookie(c: &super::Cookie) -> *mut AnyObject {
        use objc2::class;
        use objc2_foundation::NSString;
        let props: *mut AnyObject = msg_send![class!(NSMutableDictionary), new];
        let put = |k: &str, v: *mut AnyObject| {
            let _: () = msg_send![props, setObject: v, forKey: &*NSString::from_str(k)];
        };
        let st = |v: &str| objc2::rc::Retained::autorelease_ptr(NSString::from_str(v)) as *mut AnyObject;
        put("Domain", st(&c.domain));
        put("Name", st(&c.name));
        put("Path", st(if c.path.is_empty() { "/" } else { &c.path }));
        put("Value", st(&c.value));
        let date: *mut AnyObject = msg_send![class!(NSDate), dateWithTimeIntervalSince1970: c.expires];
        put("Expires", date);
        if c.secure {
            put("Secure", st("TRUE"));
        }
        if c.http_only {
            put("HttpOnly", st("TRUE"));
        }
        let cookie: *mut AnyObject = msg_send![class!(NSHTTPCookie), cookieWithProperties: props];
        let _: () = msg_send![props, release];
        cookie
    }

    /// Put cookies into the web view's own cookie jar (its profile's), then call `done` with how many were set.
    pub fn set_cookies(wv: &WebView, cookies: Vec<super::Cookie>, done: impl FnOnce(usize) + 'static) {
        unsafe {
            let config: *mut AnyObject = msg_send![ptr(wv), configuration];
            let store: *mut AnyObject = msg_send![config, websiteDataStore];
            let jar: *mut AnyObject = msg_send![store, httpCookieStore];
            let made: Vec<*mut AnyObject> = cookies.iter().map(|c| ns_cookie(c)).filter(|c| !c.is_null()).collect();
            let n = made.len();
            let done = std::cell::RefCell::new(Some(done));
            // The jar runs requests in order, so the last one finishing means they all have.
            let finished = block2::RcBlock::new(move || {
                if let Some(d) = done.borrow_mut().take() {
                    d(n);
                }
            });
            if made.is_empty() {
                finished.call(());
                return;
            }
            for (i, c) in made.iter().enumerate() {
                if i + 1 == n {
                    let _: () = msg_send![jar, setCookie: *c, completionHandler: &*finished];
                } else {
                    let _: () = msg_send![jar, setCookie: *c, completionHandler: std::ptr::null::<block2::Block<dyn Fn()>>()];
                }
            }
        }
    }

    /// "b1cc7f6a-8f82-…" → its 16 bytes.
    fn uuid_bytes(s: &str) -> Result<[u8; 16], ()> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 {
            return Err(());
        }
        let mut out = [0u8; 16];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| ())?;
        }
        Ok(out)
    }

    pub fn copy_png(png: &[u8]) -> bool {
        use objc2::class;
        unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            if pb.is_null() {
                return false;
            }
            let _: isize = msg_send![pb, clearContents];
            let data: *mut AnyObject = msg_send![class!(NSData), dataWithBytes: png.as_ptr().cast::<std::ffi::c_void>(), length: png.len()];
            let ty = objc2_foundation::NSString::from_str("public.png");
            let ok: bool = msg_send![pb, setData: data, forType: &*ty];
            ok
        }
    }

    /// Remove cookies, caches and storage for one site from this webview's profile. Calls `done` with how many
    /// site records were cleared.
    pub fn clear_site_data(wv: &WebView, host: &str, done: impl FnOnce(usize) + 'static) {
        use objc2::class;
        let v = ptr(wv);
        let host = host.to_ascii_lowercase();
        unsafe {
            let config: *mut AnyObject = msg_send![v, configuration];
            let store: *mut AnyObject = msg_send![config, websiteDataStore];
            let _: *mut AnyObject = msg_send![store, retain];
            let types: *mut AnyObject = msg_send![class!(WKWebsiteDataStore), allWebsiteDataTypes];
            let _: *mut AnyObject = msg_send![types, retain];
            let done = std::cell::RefCell::new(Some(done));
            let fetched = block2::RcBlock::new(move |records: *mut AnyObject| {
                let Some(done) = done.borrow_mut().take() else { return };
                let matching: *mut AnyObject = msg_send![class!(NSMutableArray), array];
                let n: usize = msg_send![records, count];
                for i in 0..n {
                    let rec: *mut AnyObject = msg_send![records, objectAtIndex: i];
                    let name: *mut objc2_foundation::NSString = msg_send![rec, displayName];
                    // Records are per registrable domain ("google.com"), which covers its subdomains.
                    let name = if name.is_null() { String::new() } else { (*name).to_string().to_ascii_lowercase() };
                    if !name.is_empty() && (host == name || host.ends_with(&format!(".{name}"))) {
                        let _: () = msg_send![matching, addObject: rec];
                    }
                }
                let count: usize = msg_send![matching, count];
                let done = std::cell::RefCell::new(Some(done));
                let finished = block2::RcBlock::new(move || {
                    if let Some(d) = done.borrow_mut().take() {
                        d(count)
                    }
                });
                let _: () = msg_send![store, removeDataOfTypes: types, forDataRecords: matching, completionHandler: &*finished];
                let _: () = msg_send![types, release];
                let _: () = msg_send![store, release];
            });
            let _: () = msg_send![store, fetchDataRecordsOfTypes: types, completionHandler: &*fetched];
        }
    }

    /// Opaque CoreGraphics colour, declared so objc2 can check the `^{CGColor=}` encoding.
    #[repr(C)]
    pub struct CGColor {
        _private: [u8; 0],
    }
    unsafe impl objc2::encode::RefEncode for CGColor {
        const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("CGColor", &[]));
    }

    /// A border drawn by the view's layer (follows its rounded corners); width 0 removes it.
    pub fn set_border(wv: &WebView, width: f64, rgba: [f64; 4]) {
        use objc2::class;
        let v = ptr(wv);
        unsafe {
            let _: () = msg_send![v, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![v, layer];
            if layer.is_null() {
                return;
            }
            if width > 0.0 {
                let color: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: rgba[0], green: rgba[1], blue: rgba[2], alpha: rgba[3]];
                let cg: *const CGColor = msg_send![color, CGColor];
                let _: () = msg_send![layer, setBorderColor: cg];
            }
            let _: () = msg_send![layer, setBorderWidth: width];
        }
    }

    pub fn set_traffic_lights_hidden(window: &tao::window::Window, hidden: bool) {
        use tao::platform::macos::WindowExtMacOS;
        let w = window.ns_window() as *mut AnyObject;
        unsafe {
            for b in 0usize..3 {
                let btn: *mut AnyObject = msg_send![w, standardWindowButton: b];
                if !btn.is_null() {
                    let _: () = msg_send![btn, setHidden: hidden];
                }
            }
        }
    }

    pub fn allow_magnification(wv: &WebView, on: bool) {
        unsafe {
            let _: () = msg_send![ptr(wv), setAllowsMagnification: on];
        }
    }

    pub fn set_window_opacity(window: &tao::window::Window, alpha: f64) {
        use tao::platform::macos::WindowExtMacOS;
        let w = window.ns_window() as *mut AnyObject;
        unsafe {
            let _: () = msg_send![w, setAlphaValue: alpha];
        }
    }

    thread_local! {
        static EFFECT: std::cell::Cell<*mut AnyObject> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    }

    /// Frosted glass behind the sidebar view (`material`: 0 = none). The sidebar's webview stops drawing its own
    /// background so the glass shows through wherever its page is see-through.
    pub fn set_vibrancy(window: &tao::window::Window, sidebar: &WebView, material: isize) {
        use objc2::class;
        use tao::platform::macos::WindowExtMacOS;
        let content = window.ns_view() as *mut AnyObject;
        unsafe {
            let old = EFFECT.with(|e| e.replace(std::ptr::null_mut()));
            if !old.is_null() {
                let _: () = msg_send![old, removeFromSuperview];
                let _: () = msg_send![old, release];
            }
            let no_bg = objc2_foundation::NSNumber::numberWithBool(material == 0);
            let _: () = msg_send![ptr(sidebar), setValue: &*no_bg, forKey: &*objc2_foundation::NSString::from_str("drawsBackground")];
            if material == 0 || content.is_null() {
                return;
            }
            let frame: objc2_foundation::NSRect = msg_send![content, bounds];
            let v: *mut AnyObject = msg_send![class!(NSVisualEffectView), alloc];
            let v: *mut AnyObject = msg_send![v, initWithFrame: frame];
            let _: () = msg_send![v, setMaterial: material];
            let _: () = msg_send![v, setBlendingMode: 0isize]; // behind the window
            let _: () = msg_send![v, setState: 1isize]; // always active
            let _: () = msg_send![v, setAutoresizingMask: 18usize]; // width and height follow the window
            let _: () = msg_send![content, addSubview: v, positioned: -1isize, relativeTo: std::ptr::null_mut::<AnyObject>()];
            EFFECT.with(|e| e.set(v));
        }
    }

    pub fn set_app_icon(path: Option<&std::path::Path>) {
        use objc2::class;
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let img: *mut AnyObject = match path {
                Some(p) => {
                    let a: *mut AnyObject = msg_send![class!(NSImage), alloc];
                    let s = objc2_foundation::NSString::from_str(&p.to_string_lossy());
                    msg_send![a, initWithContentsOfFile: &*s]
                }
                None => std::ptr::null_mut(),
            };
            let _: () = msg_send![app, setApplicationIconImage: img];
        }
    }

    thread_local! {
        static DOCK_MENU: std::cell::Cell<*mut AnyObject> = const { std::cell::Cell::new(std::ptr::null_mut()) };
        static STATUS_ITEM: std::cell::Cell<*mut AnyObject> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    }

    /// The Dock icon's right-click menu (an NSMenu, or null for none).
    pub fn set_dock_menu(menu: *mut AnyObject) {
        use objc2::runtime::{AnyClass, Imp, Sel};
        use objc2::{class, sel};
        static ONCE: std::sync::Once = std::sync::Once::new();
        DOCK_MENU.with(|d| d.set(menu));
        ONCE.call_once(|| unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let delegate: *mut AnyObject = msg_send![app, delegate];
            if delegate.is_null() {
                return;
            }
            unsafe extern "C-unwind" fn dock_menu(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> *mut AnyObject {
                DOCK_MENU.with(|d| d.get())
            }
            let cls = (*delegate).class() as *const AnyClass as *mut AnyClass;
            objc2::ffi::class_addMethod(cls, sel!(applicationDockMenu:), std::mem::transmute::<*const (), Imp>(dock_menu as *const ()), c"@@:@".as_ptr());
        });
    }

    /// An icon at the right of the menu bar showing `title` and opening `menu` (empty title removes it).
    pub fn set_status_item(title: &str, menu: *mut AnyObject) {
        use objc2::class;
        unsafe {
            let bar: *mut AnyObject = msg_send![class!(NSStatusBar), systemStatusBar];
            let mut item = STATUS_ITEM.with(|s| s.get());
            if title.is_empty() {
                if !item.is_null() {
                    let _: () = msg_send![bar, removeStatusItem: item];
                    let _: () = msg_send![item, release];
                    STATUS_ITEM.with(|s| s.set(std::ptr::null_mut()));
                }
                return;
            }
            if item.is_null() {
                item = msg_send![bar, statusItemWithLength: -1.0f64];
                let _: *mut AnyObject = msg_send![item, retain];
                STATUS_ITEM.with(|s| s.set(item));
            }
            let button: *mut AnyObject = msg_send![item, button];
            if !button.is_null() {
                let _: () = msg_send![button, setTitle: &*objc2_foundation::NSString::from_str(title)];
            }
            let _: () = msg_send![item, setMenu: menu];
        }
    }

    /// Ask where to save a file (a save panel); None when cancelled.
    pub fn save_panel(name: &str, folder: &std::path::Path) -> Option<std::path::PathBuf> {
        use objc2::class;
        unsafe {
            let panel: *mut AnyObject = msg_send![class!(NSSavePanel), savePanel];
            let _: () = msg_send![panel, setNameFieldStringValue: &*objc2_foundation::NSString::from_str(name)];
            let dir: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: &*objc2_foundation::NSString::from_str(&folder.to_string_lossy()), isDirectory: true];
            let _: () = msg_send![panel, setDirectoryURL: dir];
            let r: isize = msg_send![panel, runModal];
            if r != 1 {
                return None;
            }
            let url: *mut AnyObject = msg_send![panel, URL];
            let path: *mut objc2_foundation::NSString = if url.is_null() { std::ptr::null_mut() } else { msg_send![url, path] };
            (!path.is_null()).then(|| std::path::PathBuf::from((*path).to_string()))
        }
    }

    // ---------- shortcuts that work in every app (Carbon hot keys: no special permission needed) ----------
    #[repr(C)]
    struct EventTypeSpec {
        class: u32,
        kind: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct HotKeyId {
        signature: u32,
        id: u32,
    }
    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn GetApplicationEventTarget() -> *mut std::ffi::c_void;
        fn InstallEventHandler(
            target: *mut std::ffi::c_void,
            handler: extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void) -> i32,
            count: u32,
            types: *const EventTypeSpec,
            data: *mut std::ffi::c_void,
            out: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn RegisterEventHotKey(code: u32, modifiers: u32, id: HotKeyId, target: *mut std::ffi::c_void, options: u32, out: *mut *mut std::ffi::c_void) -> i32;
        fn UnregisterEventHotKey(key: *mut std::ffi::c_void) -> i32;
        fn GetEventParameter(event: *mut std::ffi::c_void, name: u32, kind: u32, out_kind: *mut u32, size: usize, out_size: *mut usize, data: *mut std::ffi::c_void) -> i32;
    }
    static HOTKEY_FN: std::sync::Mutex<Option<Box<dyn Fn(u32) + Send>>> = std::sync::Mutex::new(None);
    thread_local! {
        static HOTKEYS: std::cell::RefCell<Vec<*mut std::ffi::c_void>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    const fn four(s: &[u8; 4]) -> u32 {
        (s[0] as u32) << 24 | (s[1] as u32) << 16 | (s[2] as u32) << 8 | s[3] as u32
    }
    extern "C" fn on_hotkey(_: *mut std::ffi::c_void, event: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
        let mut id = HotKeyId { signature: 0, id: 0 };
        let ok = unsafe { GetEventParameter(event, four(b"----"), four(b"hkid"), std::ptr::null_mut(), std::mem::size_of::<HotKeyId>(), std::ptr::null_mut(), &mut id as *mut _ as *mut _) };
        if ok == 0 {
            if let Some(f) = HOTKEY_FN.lock().ok().as_ref().and_then(|g| g.as_ref()) {
                f(id.id);
            }
        }
        0
    }

    /// Register global shortcuts as (macOS key code, Carbon modifiers); `on` gets the index of the one pressed.
    /// Returns the ones that couldn't be registered (taken by another app or the system).
    pub fn set_global_hotkeys(keys: &[(u32, u32)], on: impl Fn(u32) + Send + 'static) -> Vec<usize> {
        static ONCE: std::sync::Once = std::sync::Once::new();
        if let Ok(mut f) = HOTKEY_FN.lock() {
            *f = Some(Box::new(on));
        }
        let mut failed = Vec::new();
        unsafe {
            ONCE.call_once(|| {
                let spec = EventTypeSpec { class: four(b"keyb"), kind: 5 };
                let mut r = std::ptr::null_mut();
                InstallEventHandler(GetApplicationEventTarget(), on_hotkey, 1, &spec, std::ptr::null_mut(), &mut r);
            });
            HOTKEYS.with(|h| {
                for k in h.borrow_mut().drain(..) {
                    UnregisterEventHotKey(k);
                }
                for (i, (code, mods)) in keys.iter().enumerate() {
                    let mut r = std::ptr::null_mut();
                    let id = HotKeyId { signature: four(b"VALR"), id: i as u32 };
                    if RegisterEventHotKey(*code, *mods, id, GetApplicationEventTarget(), 0, &mut r) == 0 {
                        h.borrow_mut().push(r);
                    } else {
                        failed.push(i);
                    }
                }
            });
        }
        failed
    }
}

/// Seconds since the keyboard or mouse was last used (0 where that can't be known).
pub fn idle_seconds() -> f64 {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
        }
        // kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType
        unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) }
    }
    #[cfg(not(target_os = "macos"))]
    0.0
}

/// Bring Valerie in front of other apps.
pub fn activate_app() {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::{class, msg_send};
        let app: *mut objc2::runtime::AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
    }
}

/// Pinch to zoom a page in and out (gestures.pinch_to_zoom).
pub fn allow_magnification(wv: &WebView, on: bool) {
    #[cfg(target_os = "macos")]
    mac::allow_magnification(wv, on);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, on);
}

/// How solid the whole window is (window.opacity).
pub fn set_window_opacity(window: &tao::window::Window, alpha: f64) {
    #[cfg(target_os = "macos")]
    mac::set_window_opacity(window, alpha.clamp(0.3, 1.0));
    #[cfg(not(target_os = "macos"))]
    let _ = (window, alpha);
}

/// Frosted glass behind the sidebar (window.vibrancy): "none" | "sidebar" | "window".
pub fn set_vibrancy(window: &tao::window::Window, sidebar: &WebView, kind: &str) {
    // NSVisualEffectMaterial: sidebar = 7, under-window background = 21.
    let material = match kind {
        "sidebar" => 7,
        "window" => 21,
        _ => 0,
    };
    #[cfg(target_os = "macos")]
    mac::set_vibrancy(window, sidebar, material);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, sidebar, material);
}

/// Valerie's Dock icon from a picture file (None = the usual icon).
pub fn set_app_icon(path: Option<&std::path::Path>) {
    #[cfg(target_os = "macos")]
    mac::set_app_icon(path);
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// The Dock icon's right-click menu.
pub fn set_dock_menu(menu: Option<&muda::Menu>) {
    #[cfg(target_os = "macos")]
    {
        use muda::ContextMenu;
        mac::set_dock_menu(menu.map_or(std::ptr::null_mut(), |m| m.ns_menu() as *mut objc2::runtime::AnyObject));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = menu;
}

/// An icon in the menu bar (empty title = none) that opens `menu`.
pub fn set_status_item(title: &str, menu: Option<&muda::Menu>) {
    #[cfg(target_os = "macos")]
    {
        use muda::ContextMenu;
        mac::set_status_item(title, menu.map_or(std::ptr::null_mut(), |m| m.ns_menu() as *mut objc2::runtime::AnyObject));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (title, menu);
}

/// Ask where to save a download; None = cancelled (or not possible here: then the usual folder is used).
pub fn save_panel(name: &str, folder: &std::path::Path) -> Option<Option<std::path::PathBuf>> {
    #[cfg(target_os = "macos")]
    return Some(mac::save_panel(name, folder));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (name, folder);
        None
    }
}

/// macOS key code and Carbon modifier bits for a shortcut like "Ctrl+Alt+Space" (None if it can't be a global one).
#[allow(deprecated)]
pub fn hotkey_code(accel: &str) -> Option<(u32, u32)> {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    let a: Accelerator = accel.parse().ok()?;
    let m = a.modifiers();
    let mut mods = 0u32;
    if m.contains(Modifiers::SUPER) || m.contains(Modifiers::META) {
        mods |= 1 << 8;
    }
    if m.contains(Modifiers::SHIFT) {
        mods |= 1 << 9;
    }
    if m.contains(Modifiers::ALT) {
        mods |= 1 << 11;
    }
    if m.contains(Modifiers::CONTROL) {
        mods |= 1 << 12;
    }
    let code = match a.key() {
        Code::KeyA => 0x00, Code::KeyS => 0x01, Code::KeyD => 0x02, Code::KeyF => 0x03, Code::KeyH => 0x04, Code::KeyG => 0x05,
        Code::KeyZ => 0x06, Code::KeyX => 0x07, Code::KeyC => 0x08, Code::KeyV => 0x09, Code::KeyB => 0x0B, Code::KeyQ => 0x0C,
        Code::KeyW => 0x0D, Code::KeyE => 0x0E, Code::KeyR => 0x0F, Code::KeyY => 0x10, Code::KeyT => 0x11, Code::Digit1 => 0x12,
        Code::Digit2 => 0x13, Code::Digit3 => 0x14, Code::Digit4 => 0x15, Code::Digit6 => 0x16, Code::Digit5 => 0x17,
        Code::Equal => 0x18, Code::Digit9 => 0x19, Code::Digit7 => 0x1A, Code::Minus => 0x1B, Code::Digit8 => 0x1C,
        Code::Digit0 => 0x1D, Code::BracketRight => 0x1E, Code::KeyO => 0x1F, Code::KeyU => 0x20, Code::BracketLeft => 0x21,
        Code::KeyI => 0x22, Code::KeyP => 0x23, Code::Enter => 0x24, Code::KeyL => 0x25, Code::KeyJ => 0x26, Code::Quote => 0x27,
        Code::KeyK => 0x28, Code::Semicolon => 0x29, Code::Backslash => 0x2A, Code::Comma => 0x2B, Code::Slash => 0x2C,
        Code::KeyN => 0x2D, Code::KeyM => 0x2E, Code::Period => 0x2F, Code::Tab => 0x30, Code::Space => 0x31, Code::Backquote => 0x32,
        Code::Backspace => 0x33, Code::Escape => 0x35, Code::F1 => 0x7A, Code::F2 => 0x78, Code::F3 => 0x63, Code::F4 => 0x76,
        Code::F5 => 0x60, Code::F6 => 0x61, Code::F7 => 0x62, Code::F8 => 0x64, Code::F9 => 0x65, Code::F10 => 0x6D, Code::F11 => 0x67,
        Code::F12 => 0x6F, Code::ArrowLeft => 0x7B, Code::ArrowRight => 0x7C, Code::ArrowDown => 0x7D, Code::ArrowUp => 0x7E,
        _ => return None,
    };
    Some((code, mods))
}

/// Register shortcuts that work in every app; `on` gets the index of the one pressed. Returns the indexes that
/// couldn't be registered.
pub fn set_global_hotkeys(keys: &[(u32, u32)], on: impl Fn(u32) + Send + 'static) -> Vec<usize> {
    #[cfg(target_os = "macos")]
    return mac::set_global_hotkeys(keys, on);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = on;
        (0..keys.len()).collect()
    }
}

pub fn bring_to_front(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::bring_to_front(wv);
    #[cfg(not(target_os = "macos"))]
    let _ = wv;
}

/// Which corners of a page to round: the ones that meet the browser's own UI, not the window edge.
#[derive(Clone, Copy, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum Corners {
    None,
    All,
    Left,
    Right,
    Top,
}

pub fn round_corners(wv: &WebView, radius: f64, corners: Corners) {
    #[cfg(target_os = "macos")]
    mac::round_corners(wv, radius, corners);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, radius, corners);
}

pub fn set_traffic_lights_hidden(window: &tao::window::Window, hidden: bool) {
    #[cfg(target_os = "macos")]
    mac::set_traffic_lights_hidden(window, hidden);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, hidden);
}

/// Animate a child webview to new bounds. Returns false when the platform can't, so the caller sets them directly.
pub fn animate_bounds(wv: &WebView, x: f64, y: f64, w: f64, h: f64, secs: f64) -> bool {
    #[cfg(target_os = "macos")]
    return mac::animate_bounds(wv, x, y, w, h, secs);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (wv, x, y, w, h, secs);
        false
    }
}

pub fn set_alpha(wv: &WebView, alpha: f64, secs: f64) {
    #[cfg(target_os = "macos")]
    mac::set_alpha(wv, alpha, secs);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, alpha, secs);
}

pub fn fade_in(wv: &WebView, secs: f64) {
    #[cfg(target_os = "macos")]
    mac::fade_in(wv, secs);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, secs);
}

/// Safari's user agent for this OS version, so sites serve their full modern pages.
pub fn safari_user_agent() -> String {
    let version = safari_version().unwrap_or_else(|| "18.0".into());
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{version} Safari/605.1.15"
    )
}

fn safari_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let v = plist::Value::from_file("/Applications/Safari.app/Contents/Info.plist").ok()?;
        let s = v.as_dictionary()?.get("CFBundleShortVersionString")?.as_string()?.to_string();
        let mut parts = s.split('.');
        let major = parts.next()?;
        Some(format!("{major}.{}", parts.next().unwrap_or("0")))
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// PNG of the visible page, `width` points wide. Calls `done` later on the main thread.
pub fn snapshot(wv: &WebView, width: f64, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
    #[cfg(target_os = "macos")]
    mac::snapshot(wv, width, done);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (wv, width);
        done(Err("screenshots are only implemented on macOS so far".into()));
    }
}

/// Remove one site's cookies, caches and storage from the webview's profile; `done` gets the number of records removed.
pub fn clear_site_data(wv: &WebView, host: &str, done: impl FnOnce(usize) + 'static) {
    #[cfg(target_os = "macos")]
    mac::clear_site_data(wv, host, done);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (wv, host);
        done(0);
    }
}

/// Ask macOS to make Valerie the default browser (macOS shows its own confirmation). Needs the bundled Valerie.app.
pub fn set_default_browser() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};
        #[link(name = "CoreServices", kind = "framework")]
        extern "C" {
            fn LSSetDefaultHandlerForURLScheme(scheme: *const AnyObject, bundle_id: *const AnyObject) -> i32;
        }
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let id: *mut AnyObject = msg_send![bundle, bundleIdentifier];
        if id.is_null() {
            return Err("run Valerie.app (scripts/bundle-macos.sh) to make it the default browser".into());
        }
        for scheme in ["http", "https"] {
            let s = objc2_foundation::NSString::from_str(scheme);
            let status = LSSetDefaultHandlerForURLScheme(&*s as *const _ as *const AnyObject, id);
            if status != 0 {
                return Err(format!("macOS refused ({status})"));
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("set Valerie as the default browser in your system settings".into())
}

/// Outline a webview (the focused split-view pane); width 0 removes the outline.
pub fn set_border(wv: &WebView, width: f64, rgba: [f64; 4]) {
    #[cfg(target_os = "macos")]
    mac::set_border(wv, width, rgba);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, width, rgba);
}

/// Put a PNG image on the clipboard. Returns false where that isn't supported.
pub fn copy_png(png: &[u8]) -> bool {
    #[cfg(target_os = "macos")]
    return mac::copy_png(png);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = png;
        false
    }
}

/// Local date and time for file names, like "2026-10-15 at 14.03.22".
pub fn local_timestamp() -> String {
    match local_time(crate::state::now()) {
        Some(t) => format!("{:04}-{:02}-{:02} at {:02}.{:02}.{:02}", t.year, t.month, t.day, t.hour, t.min, t.sec),
        None => crate::state::now().to_string(),
    }
}

/// Make `alert()`, `confirm()` and `prompt()` in web pages show real dialogs (pass any webview Valerie made).
pub fn install_page_dialogs(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::install_page_dialogs(wv);
    #[cfg(not(target_os = "macos"))]
    let _ = wv;
}

/// Open the print dialog for a page.
pub fn print(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::print(wv);
    #[cfg(not(target_os = "macos"))]
    let _ = wv.evaluate_script("window.print()");
}

/// One notification. `action` is JSON Valerie gets back when it's clicked.
pub struct Note<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub body: &'a str,
    pub action: Option<String>,
    pub sound: bool,
}

/// Get ready to send notifications; `on_click` receives a clicked notification's action.
pub fn notifications_init(on_click: impl Fn(String) + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    mac::notifications_init(Box::new(on_click));
    #[cfg(not(target_os = "macos"))]
    let _ = on_click;
}

/// Show a system notification: macOS's own notification system when Valerie runs as an app, otherwise the best the
/// platform offers (a script on a bare macOS build, notify-send on Linux).
pub fn notify(n: &Note) {
    #[cfg(target_os = "macos")]
    if mac::notify_native(n) || mac::notify_legacy(n) {
        return;
    }
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!("display notification \"{}\" with title \"{}\" subtitle \"{}\"", esc(n.body), esc(n.title), esc(n.subtitle)))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("notify-send").args([n.title, &format!("{}\n{}", n.subtitle, n.body)]).spawn();
    #[cfg(windows)]
    let _ = esc;
}

/// Today's local date, weekday and time: ("2026-10-16", "Thu", "Thursday 16 October", "08:02").
pub fn local_day() -> (String, String, String, String) {
    match local_time(crate::state::now()) {
        Some(t) => (
            t.date(),
            WEEKDAYS[t.weekday][..3].to_string(),
            format!("{} {} {}", WEEKDAYS[t.weekday], t.day, MONTHS[t.month as usize - 1]),
            format!("{:02}:{:02}", t.hour, t.min),
        ),
        None => (format!("day {}", crate::state::now() / 86400), String::new(), String::new(), String::new()),
    }
}

/// A question with several answers on a webview's window; `done(Some(i))` for the i-th button. Elsewhere `None`.
pub fn choose(wv: &WebView, title: &str, body: &str, buttons: &[&str], done: impl FnOnce(Option<usize>) + 'static) {
    #[cfg(target_os = "macos")]
    mac::choose(wv, title, body, buttons, move |i| done(Some(i)));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (wv, title, body, buttons);
        done(None);
    }
}

/// Ask a yes/no question on a webview's window (a sheet); `done(true)` for yes. Elsewhere it answers no.
pub fn ask(wv: &WebView, title: &str, body: &str, yes: &str, no: &str, done: impl FnOnce(bool) + 'static) {
    #[cfg(target_os = "macos")]
    mac::ask(wv, title, body, yes, no, done);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (wv, title, body, yes, no);
        done(false);
    }
}

/// A moment (seconds) as local "Thu 09:00" (or "09:00" if it's today).
pub fn local_time_of(secs: u64) -> String {
    let (Some(t), Some(today)) = (local_time(secs), local_time(crate::state::now())) else { return format!("{secs}") };
    if t.date() == today.date() {
        format!("{:02}:{:02}", t.hour, t.min)
    } else {
        format!("{} {:02}:{:02}", &WEEKDAYS[t.weekday][..3], t.hour, t.min)
    }
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// A moment on the computer's clock, in its time zone.
struct LocalTime {
    year: i32,
    month: u32,
    day: u32,
    weekday: usize,
    hour: u32,
    min: u32,
    sec: u32,
}

impl LocalTime {
    fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Read straight from the C library (this runs every tick; starting `date` each time was a process per call).
fn local_time(secs: u64) -> Option<LocalTime> {
    #[cfg(unix)]
    {
        use std::ffi::{c_char, c_int, c_long};
        #[repr(C)]
        struct Tm {
            sec: c_int,
            min: c_int,
            hour: c_int,
            mday: c_int,
            mon: c_int,
            year: c_int,
            wday: c_int,
            yday: c_int,
            isdst: c_int,
            gmtoff: c_long,
            zone: *const c_char,
        }
        extern "C" {
            fn tzset();
            fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
        }
        let t = secs as i64;
        let mut tm: Tm = unsafe { std::mem::zeroed() };
        // tzset picks up a time zone change (localtime_r alone needn't).
        if unsafe { tzset(); localtime_r(&t, &mut tm) }.is_null() {
            return None;
        }
        Some(LocalTime {
            year: tm.year + 1900,
            month: (tm.mon + 1) as u32,
            day: tm.mday as u32,
            weekday: tm.wday.clamp(0, 6) as usize,
            hour: tm.hour as u32,
            min: tm.min as u32,
            sec: tm.sec as u32,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = secs;
        None
    }
}

/// A saved cookie (from WebKit's Cookies.binarycookies).
#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub domain: String,
    pub name: String,
    pub path: String,
    pub value: String,
    /// Seconds since 1970.
    pub expires: f64,
    pub secure: bool,
    pub http_only: bool,
}

/// Read WebKit's Cookies.binarycookies: "cook", page count and sizes (big-endian), then pages of cookie records
/// (little-endian) with null-terminated strings and dates counted from 2001.
pub fn read_binary_cookies(data: &[u8]) -> Vec<Cookie> {
    let be = |b: &[u8], i: usize| b.get(i..i + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap()) as usize);
    let le = |b: &[u8], i: usize| b.get(i..i + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()) as usize);
    let f64le = |b: &[u8], i: usize| b.get(i..i + 8).map(|s| f64::from_le_bytes(s.try_into().unwrap()));
    let text = |b: &[u8], i: usize| b.get(i..).map(|s| String::from_utf8_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]).into_owned());
    let mut out = Vec::new();
    if data.get(..4) != Some(b"cook") {
        return out;
    }
    let Some(pages) = be(data, 4) else { return out };
    let sizes: Vec<usize> = (0..pages).filter_map(|i| be(data, 8 + i * 4)).collect();
    let mut at = 8 + pages * 4;
    for size in sizes {
        let Some(page) = data.get(at..at + size) else { break };
        at += size;
        let n = le(page, 4).unwrap_or(0);
        for k in 0..n {
            let Some(off) = le(page, 8 + k * 4) else { continue };
            let Some(c) = page.get(off..) else { continue };
            let (Some(flags), Some(u), Some(nm), Some(p), Some(v), Some(exp)) = (le(c, 8), le(c, 16), le(c, 20), le(c, 24), le(c, 28), f64le(c, 40)) else { continue };
            let (Some(domain), Some(name), Some(path), Some(value)) = (text(c, u), text(c, nm), text(c, p), text(c, v)) else { continue };
            out.push(Cookie { domain, name, path, value, expires: exp + 978_307_200.0, secure: flags & 1 != 0, http_only: flags & 4 != 0 });
        }
    }
    out
}

/// Whether passkeys can work in Valerie's pages. On Mac WebKit only does them for a browser Apple has granted
/// `com.apple.developer.web-browser.public-key-credential`; Windows hands them to Windows Hello.
pub fn passkeys_work() -> bool {
    #[cfg(target_os = "macos")]
    {
        static WORKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *WORKS.get_or_init(|| mac::has_entitlement("com.apple.developer.web-browser.public-key-credential"))
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// Safari's saved cookies (it keeps them in its sandbox, which needs Full Disk Access to read).
/// `VALERIE_SAFARI_COOKIES` points at another file, for tests.
pub fn safari_cookies() -> Result<Vec<Cookie>, std::io::ErrorKind> {
    let path = match std::env::var_os("VALERIE_SAFARI_COOKIES") {
        Some(p) => std::path::PathBuf::from(p),
        None => dirs::home_dir().ok_or(std::io::ErrorKind::NotFound)?.join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies"),
    };
    std::fs::read(path).map(|d| read_binary_cookies(&d)).map_err(|e| e.kind())
}

/// Put cookies into a web view's profile; `done` gets how many were set.
pub fn set_cookies(wv: &WebView, cookies: Vec<Cookie>, done: impl FnOnce(usize) + 'static) {
    #[cfg(target_os = "macos")]
    mac::set_cookies(wv, cookies, done);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (wv, cookies);
        done(0);
    }
}

/// Bring logins from runs of the bare `valerie` binary (WebKit keeps those apart, under ~/Library/WebKit/valerie)
/// into the app's own (~/Library/WebKit/dev.valerie.browser), once. A profile only the bare binary had is copied
/// whole; one both had gets the other's cookies where they're newer. Call before any page view exists.
pub fn merge_bare_logins(marker: &std::path::Path, profiles: &[String]) {
    #[cfg(target_os = "macos")]
    mac::merge_bare_logins(marker, profiles);
    #[cfg(not(target_os = "macos"))]
    let _ = (marker, profiles);
}

#[cfg(test)]
mod cookie_tests {
    /// One page holding one cookie, laid out like WebKit writes it.
    fn blob(domain: &str, name: &str, path: &str, value: &str, expires_2001: f64, flags: u32) -> Vec<u8> {
        let mut c = vec![0u8; 56];
        let mut strings = Vec::new();
        let mut offs = [0u32; 4];
        for (i, s) in [domain, name, path, value].iter().enumerate() {
            offs[i] = (56 + strings.len()) as u32;
            strings.extend_from_slice(s.as_bytes());
            strings.push(0);
        }
        c[8..12].copy_from_slice(&flags.to_le_bytes());
        for (i, o) in offs.iter().enumerate() {
            c[16 + i * 4..20 + i * 4].copy_from_slice(&o.to_le_bytes());
        }
        c[40..48].copy_from_slice(&expires_2001.to_le_bytes());
        c.extend(strings);
        let size = c.len() as u32;
        c[0..4].copy_from_slice(&size.to_le_bytes());
        let mut page = vec![0, 0, 1, 0];
        page.extend(1u32.to_le_bytes());
        page.extend(16u32.to_le_bytes()); // the cookie starts after header, count, one offset, footer
        page.extend([0u8; 4]);
        page.extend(c);
        let mut out = b"cook".to_vec();
        out.extend(1u32.to_be_bytes());
        out.extend((page.len() as u32).to_be_bytes());
        out.extend(page);
        out
    }

    #[cfg(unix)]
    #[test]
    fn local_time_matches_date() {
        let out = std::process::Command::new("date").arg("+%Y-%m-%d|%a|%A %-d %B|%H:%M").output().unwrap();
        let want = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let (a, b, c, d) = super::local_day();
        assert_eq!(format!("{a}|{b}|{c}|{d}"), want);
        assert_eq!(super::local_time_of(crate::state::now()), d);
        assert!(super::local_time_of(crate::state::now() - 3 * 86400).starts_with(&super::WEEKDAYS[(super::local_time(crate::state::now() - 3 * 86400).unwrap().weekday)][..3]));
    }

    #[test]
    fn picks_terminal_editors() {
        assert_eq!(super::terminal_command("vim"), Some("vim"));
        assert_eq!(super::terminal_command("nvim -O"), Some("nvim -O"));
        assert_eq!(super::terminal_command("terminal: less -N"), Some("less -N"));
        assert_eq!(super::terminal_command("Visual Studio Code"), None);
        assert_eq!(super::terminal_command("default"), None);
        assert_eq!(super::shell_quote("/a b/it's.yaml"), "'/a b/it'\\''s.yaml'");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_own_entitlements() {
        assert!(!super::passkeys_work());
        if let Ok(e) = std::env::var("VALERIE_EXPECT_ENTITLEMENT") {
            assert!(super::mac::has_entitlement(&e), "{e}");
        }
    }

    #[test]
    fn reads_webkit_cookies() {
        let got = super::read_binary_cookies(&blob(".example.com", "sid", "/", "abc", 1_000_000.0, 5));
        assert_eq!(got.len(), 1);
        let c = &got[0];
        assert_eq!((c.domain.as_str(), c.name.as_str(), c.path.as_str(), c.value.as_str()), (".example.com", "sid", "/", "abc"));
        assert_eq!(c.expires, 979_307_200.0);
        assert!(c.secure && c.http_only);
        assert!(super::read_binary_cookies(b"nope").is_empty());
    }

    /// `VALERIE_COOKIES=<path to Cookies.binarycookies> cargo test real_cookie_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_cookie_file() {
        let path = std::env::var("VALERIE_COOKIES").expect("set VALERIE_COOKIES");
        let got = super::read_binary_cookies(&std::fs::read(path).unwrap());
        let mut sites: Vec<&str> = got.iter().map(|c| c.domain.trim_start_matches('.')).collect();
        sites.sort();
        sites.dedup();
        println!("{} cookies from {} sites, e.g. {:?}", got.len(), sites.len(), &sites[..sites.len().min(8)]);
        assert!(!got.is_empty());
    }
}
