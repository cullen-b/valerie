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

/// A program Valerie starts for itself. On Windows it would otherwise open a console window of its own.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Tell the user Valerie can't start: a Windows app started from the Start menu has no terminal to print to.
#[cfg(windows)]
pub fn startup_error(text: &str) {
    use windows::{core::{w, HSTRING}, Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR}};
    unsafe { MessageBoxW(None, &HSTRING::from(text), w!("Valerie"), MB_ICONERROR) };
}

/// Open a URL or file with the OS default handler.
pub fn open_external(target: &str) {
    #[cfg(target_os = "macos")]
    let r = crate::platform::command("open").arg(target).spawn();
    #[cfg(target_os = "windows")]
    let r = crate::platform::command("cmd").args(["/C", "start", "", target]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = crate::platform::command("xdg-open").arg(target).spawn();
    if let Err(e) = r {
        crate::log::line("open", format!("could not open {target}: {e}"));
    }
}

/// Show a file in its folder (Finder, Explorer), selected where the system can.
pub fn reveal(path: &str) {
    #[cfg(target_os = "macos")]
    let r = crate::platform::command("open").args(["-R", path]).spawn();
    #[cfg(target_os = "windows")]
    let r = crate::platform::command("explorer").arg(format!("/select,{path}")).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = crate::platform::command("xdg-open").arg(std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("/"))).spawn();
    if let Err(e) = r {
        crate::log::line("open", format!("could not show {path}: {e}"));
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

/// Whether a program can be started by name here.
fn on_path(program: &str) -> bool {
    std::env::split_paths(&crate::ai::search_path()).any(|dir| {
        ["", ".exe", ".cmd", ".bat"].iter().any(|ext| dir.join(format!("{program}{ext}")).is_file())
    })
}

#[cfg(target_os = "windows")]
fn exit_ok() -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Open a text file the way the `editor` setting says: the default app, an app by name, or a terminal editor
/// running in the `terminal` app.
pub fn open_in_editor(path: &std::path::Path, editor: &str, terminal: &str) -> Result<(), String> {
    let file = path.to_string_lossy().to_string();
    let ran = |r: std::io::Result<std::process::ExitStatus>| match r {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err("it isn't installed".to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Some(cmd) = terminal_command(editor) {
        // Settings sync, and the editor picked on one computer may not be on this one.
        let program = cmd.split_whitespace().next().unwrap_or(cmd);
        if !on_path(program) {
            crate::log::line("editor", format!("{program} isn't on this computer, so the file opens in the usual app"));
            #[cfg(target_os = "windows")]
            return ran(crate::platform::command("notepad").arg(path).spawn().map(|_| exit_ok()));
            #[cfg(not(target_os = "windows"))]
            return open_in_editor(path, "default", terminal);
        }
        #[cfg(not(target_os = "windows"))]
        let line = format!("{cmd} {}", shell_quote(&file));
        #[cfg(target_os = "windows")]
        let line = format!("{cmd} \"{file}\"");
        #[cfg(target_os = "macos")]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let osa = |script: String| ran(crate::platform::command("osascript").args(["-e", &script]).status());
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            return match terminal.to_lowercase().as_str() {
                "iterm" | "iterm2" => osa(format!(
                    "tell application \"iTerm\"\nactivate\ncreate window with default profile command \"{}\"\nend tell",
                    esc(&format!("{shell} -lc {}", shell_quote(&line)))
                )),
                "ghostty" => ran(crate::platform::command("open").args(["-na", "Ghostty", "--args", "-e", &shell, "-lc", &line]).status()),
                "wezterm" => ran(crate::platform::command("open").args(["-na", "WezTerm", "--args", "start", "--", &shell, "-lc", &line]).status()),
                "kitty" => ran(crate::platform::command("open").args(["-na", "kitty", "--args", &shell, "-lc", &line]).status()),
                "alacritty" => ran(crate::platform::command("open").args(["-na", "Alacritty", "--args", "-e", &shell, "-lc", &line]).status()),
                _ => osa(format!("tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell", esc(&line))),
            };
        }
        #[cfg(target_os = "windows")]
        return ran(crate::platform::command("cmd").args(["/C", "start", "", "cmd", "/K", &line]).status());
        #[cfg(all(unix, not(target_os = "macos")))]
        return ran(crate::platform::command("x-terminal-emulator").args(["-e", "sh", "-c", &line]).status());
    }
    let _ = terminal;
    match editor.trim().to_lowercase().as_str() {
        "default" | "" => {
            #[cfg(target_os = "macos")]
            {
                // No app set for .yaml files: fall back to the plain text editor.
                if !crate::platform::command("open").arg(path).status().is_ok_and(|s| s.success()) {
                    return ran(crate::platform::command("open").arg("-t").arg(path).status());
                }
                Ok(())
            }
            #[cfg(not(target_os = "macos"))]
            {
                open_external(&file);
                Ok(())
            }
        }
        "textedit" if cfg!(target_os = "macos") => ran(crate::platform::command("open").args(["-a", "TextEdit"]).arg(path).status()),
        _ => {
            #[cfg(target_os = "macos")]
            return ran(crate::platform::command("open").args(["-a", editor.trim()]).arg(path).status());
            #[cfg(not(target_os = "macos"))]
            {
                let mut words = editor.split_whitespace();
                let prog = words.next().unwrap_or_default();
                crate::platform::command(prog).args(words).arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
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

    /// Whether the page has an address: a view whose only load became a download has none.
    pub fn has_url(wv: &WebView) -> bool {
        let url: *mut AnyObject = unsafe { msg_send![ptr(wv), URL] };
        !url.is_null()
    }

    /// A real mouse click at (x, y) in the page's CSS pixels, sent straight to the web view: the page sees it as the
    /// user's own (isTrusted), and the user's own cursor doesn't move. Needs the view to be in a window.
    pub fn real_click(wv: &WebView, x: f64, y: f64, right: bool, clicks: isize) -> Result<(), String> {
        use objc2::class;
        use objc2_foundation::{NSPoint, NSRect};
        let v = ptr(wv);
        unsafe {
            let window: *mut AnyObject = msg_send![v, window];
            if window.is_null() {
                return Err("the tab isn't in a window".into());
            }
            let zoom: f64 = msg_send![v, pageZoom];
            let bounds: NSRect = msg_send![v, bounds];
            let flipped: bool = msg_send![v, isFlipped];
            let (px, py) = (x * zoom, y * zoom);
            if px < 0.0 || py < 0.0 || px > bounds.size.width || py > bounds.size.height {
                return Err("the point is outside the tab's visible area".into());
            }
            let local = NSPoint::new(px, if flipped { py } else { bounds.size.height - py });
            let at: NSPoint = msg_send![v, convertPoint: local, toView: std::ptr::null_mut::<AnyObject>()];
            let number: isize = msg_send![window, windowNumber];
            let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
            let now: f64 = msg_send![info, systemUptime];
            // NSEventType: 1/2 left down/up, 3/4 right down/up, 5 moved.
            let event = |kind: usize, count: isize| -> *mut AnyObject {
                msg_send![class!(NSEvent), mouseEventWithType: kind, location: at, modifierFlags: 0usize, timestamp: now,
                    windowNumber: number, context: std::ptr::null_mut::<AnyObject>(), eventNumber: 0isize, clickCount: count, pressure: 1.0f32]
            };
            let moved = event(5, 0);
            let _: () = msg_send![v, mouseMoved: moved];
            for n in 1..=clicks.max(1) {
                let (down, up) = if right { (event(3, n), event(4, n)) } else { (event(1, n), event(2, n)) };
                if down.is_null() || up.is_null() {
                    return Err("couldn't make the mouse event".into());
                }
                if right {
                    let _: () = msg_send![v, rightMouseDown: down];
                    let _: () = msg_send![v, rightMouseUp: up];
                } else {
                    let _: () = msg_send![v, mouseDown: down];
                    let _: () = msg_send![v, mouseUp: up];
                }
            }
        }
        Ok(())
    }

    /// Reaches the page ahead of the close, so sound stops before the site's unload code (which can run for 100s of ms).
    pub fn silence(wv: &WebView) {
        use objc2::sel;
        let v = ptr(wv);
        unsafe {
            if msg_send![v, respondsToSelector: sel!(_setPageMuted:)] {
                let _: () = msg_send![v, _setPageMuted: 1usize];
            }
        }
    }

    /// A configuration whose webview shares `with`'s web process (WebKit's related view).
    pub fn sharing_process(with: &WebView) -> objc2::rc::Retained<AnyObject> {
        let config: objc2::rc::Retained<AnyObject> = unsafe { msg_send![objc2::class!(WKWebViewConfiguration), new] };
        relate(&config, with);
        config
    }

    pub fn relate(config: &AnyObject, with: &WebView) {
        unsafe {
            if msg_send![config, respondsToSelector: objc2::sel!(_setRelatedWebView:)] {
                // WebKit throws (macOS 26 aborted Valerie at launch) unless both views use the same process pool and data store.
                let theirs: *mut AnyObject = msg_send![ptr(with), configuration];
                if theirs.is_null() {
                    return;
                }
                let pool: *mut AnyObject = msg_send![theirs, processPool];
                let store: *mut AnyObject = msg_send![theirs, websiteDataStore];
                if !pool.is_null() {
                    let _: () = msg_send![config, setProcessPool: pool];
                }
                if !store.is_null() {
                    let _: () = msg_send![config, setWebsiteDataStore: store];
                }
                let _: () = msg_send![config, _setRelatedWebView: ptr(with)];
            }
        }
    }

    /// A configuration with a profile's storage (what wry makes from a data store identifier).
    pub fn profile_configuration(id: [u8; 16]) -> objc2::rc::Retained<AnyObject> {
        unsafe {
            let config: objc2::rc::Retained<AnyObject> = msg_send![objc2::class!(WKWebViewConfiguration), new];
            let uuid: *mut AnyObject = msg_send![objc2::class!(NSUUID), alloc];
            let uuid: *mut AnyObject = msg_send![uuid, initWithUUIDBytes: id.as_ptr()];
            let store: *mut AnyObject = msg_send![objc2::class!(WKWebsiteDataStore), dataStoreForIdentifier: uuid];
            let _: () = msg_send![uuid, release];
            let _: () = msg_send![&*config, setWebsiteDataStore: store];
            config
        }
    }

    /// Safari's way with hidden tabs: their timers fire less and less often the longer they stay hidden.
    pub fn slow_hidden_timers(config: &AnyObject) {
        unsafe {
            let prefs: *mut AnyObject = msg_send![config, preferences];
            if prefs.is_null() {
                return;
            }
            if msg_send![prefs, respondsToSelector: objc2::sel!(_setHiddenPageDOMTimerThrottlingEnabled:)] {
                let _: () = msg_send![prefs, _setHiddenPageDOMTimerThrottlingEnabled: true];
            }
            if msg_send![prefs, respondsToSelector: objc2::sel!(_setHiddenPageDOMTimerThrottlingAutoIncreases:)] {
                let _: () = msg_send![prefs, _setHiddenPageDOMTimerThrottlingAutoIncreases: true];
            }
            if msg_send![prefs, respondsToSelector: objc2::sel!(_setPageVisibilityBasedProcessSuppressionEnabled:)] {
                let _: () = msg_send![prefs, _setPageVisibilityBasedProcessSuppressionEnabled: true];
            }
        }
    }

    /// For scripts/perf-bench.py (VALERIE_BENCH=1): a test window sits behind the owner's, and an occluded page counts
    /// as hidden, so it would never draw; this makes it draw as if in view (Ardia's bench does the same for itself).
    pub fn bench_in_view(wv: &WebView) {
        use objc2::sel;
        let v = ptr(wv);
        unsafe {
            if msg_send![v, respondsToSelector: sel!(_setWindowOcclusionDetectionEnabled:)] {
                let _: () = msg_send![v, _setWindowOcclusionDetectionEnabled: false];
            }
        }
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

    /// Back under every other view, but above the frosted glass when there is some.
    pub fn send_to_back(wv: &WebView) {
        let v = ptr(wv);
        unsafe {
            let sup: *mut AnyObject = msg_send![v, superview];
            if sup.is_null() {
                return;
            }
            let glass = EFFECT.with(|e| e.get());
            let glass_here = !glass.is_null() && {
                let gs: *mut AnyObject = msg_send![glass, superview];
                gs == sup
            };
            if glass_here {
                let _: () = msg_send![sup, addSubview: v, positioned: 1isize, relativeTo: glass];
            } else {
                let _: () = msg_send![sup, addSubview: v, positioned: -1isize, relativeTo: std::ptr::null_mut::<AnyObject>()];
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

    pub fn follow_window(wv: &WebView, follow: super::Follow) {
        use super::Follow;
        let v = ptr(wv);
        unsafe {
            let sup: *mut AnyObject = msg_send![v, superview];
            if sup.is_null() {
                return;
            }
            // Which margins and sizes stretch: (left, width, right, top, height, bottom).
            let (l, w, r, t, h, b) = match follow {
                Follow::Fill => (false, true, false, false, true, false),
                Follow::Pane { first, last } => (!first, true, !last, false, true, false),
                Follow::Top => (false, true, false, false, false, true),
                Follow::Bottom => (false, true, false, true, false, false),
                Follow::TopRight => (true, false, false, false, false, true),
                Follow::Side { right } => (right, false, !right, false, true, false),
            };
            let flipped: bool = msg_send![sup, isFlipped];
            let (min_y, max_y) = if flipped { (t, b) } else { (b, t) };
            // NSViewMinXMargin=1, WidthSizable=2, MaxXMargin=4, MinYMargin=8, HeightSizable=16, MaxYMargin=32.
            let mask = [l, w, r, min_y, h, max_y].iter().enumerate().fold(0usize, |m, (i, on)| if *on { m | 1 << i } else { m });
            let _: () = msg_send![v, setAutoresizingMask: mask];
        }
    }

    /// Run `f` inside an NSAnimationContext group so `animator` property changes ease over `secs`.
    unsafe fn animate(secs: f64, f: impl FnOnce()) {
        animate_curve(secs, "easeInEaseOut", f)
    }

    /// `animate` with a named CAMediaTimingFunction curve.
    unsafe fn animate_curve(secs: f64, curve: &str, f: impl FnOnce()) {
        use objc2::class;
        let ctx_cls = class!(NSAnimationContext);
        let _: () = msg_send![ctx_cls, beginGrouping];
        let ctx: *mut AnyObject = msg_send![ctx_cls, currentContext];
        let _: () = msg_send![ctx, setDuration: secs];
        let name = objc2_foundation::NSString::from_str(curve);
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
            // Ease-out: a press mid-slide turns around at full speed instead of stalling at the start of an ease-in.
            animate_curve(secs, "easeOut", || {
                let animator: *mut AnyObject = msg_send![v, animator];
                let _: () = msg_send![animator, setFrame: target];
            });
        }
        true
    }

    pub fn set_alpha(wv: &WebView, alpha: f64, secs: f64) {
        let v = ptr(wv);
        unsafe {
            // layout runs on every resize event; don't start an animation that changes nothing.
            let now: f64 = msg_send![v, alphaValue];
            if (now - alpha).abs() < 0.001 {
                return;
            }
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

    /// The whole page, top to bottom, as a PNG: WebKit makes a PDF of all of it, which is drawn at up to 2× (fewer for very
    /// long pages, to stay near 30 million pixels).
    pub fn snapshot_full(wv: &WebView, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
        use objc2::class;
        use objc2_foundation::{NSRect, NSSize};
        let v = ptr(wv);
        let done = std::cell::RefCell::new(Some(done));
        let block = block2::RcBlock::new(move |pdf: *mut AnyObject, _error: *mut AnyObject| {
            let Some(done) = done.borrow_mut().take() else { return };
            if pdf.is_null() {
                return done(Err("the page could not be captured (is the tab loaded?)".into()));
            }
            unsafe {
                let rep: *mut AnyObject = msg_send![class!(NSPDFImageRep), imageRepWithData: pdf];
                if rep.is_null() {
                    return done(Err("WebKit's picture of the page couldn't be read".into()));
                }
                let bounds: NSRect = msg_send![rep, bounds];
                let (w, h) = (bounds.size.width.max(1.0), bounds.size.height.max(1.0));
                let scale = (30_000_000.0 / (w * h)).sqrt().clamp(0.25, 2.0);
                let image: *mut AnyObject = msg_send![class!(NSImage), alloc];
                let image: *mut AnyObject = msg_send![image, initWithData: pdf];
                // In points: macOS draws them at the screen's scale (2 pixels a point on a Retina screen).
                let screen: *mut AnyObject = msg_send![class!(NSScreen), mainScreen];
                let backing: f64 = if screen.is_null() { 1.0 } else { msg_send![screen, backingScaleFactor] };
                let size = NSSize::new((w * scale / backing.max(1.0)).round(), (h * scale / backing.max(1.0)).round());
                let _: () = msg_send![image, setSize: size];
                // Drawn at that many pixels (TIFFRepresentation would draw one pixel per point).
                let mut rect = NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), size);
                let null: *mut AnyObject = std::ptr::null_mut();
                let cg: *mut AnyObject = msg_send![image, CGImageForProposedRect: &mut rect as *mut NSRect, context: null, hints: null];
                let bitmap: *mut AnyObject = if cg.is_null() { std::ptr::null_mut() } else {
                    let b: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
                    msg_send![b, initWithCGImage: cg]
                };
                let props: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
                // NSBitmapImageFileTypePNG = 4
                let png: *mut AnyObject = if bitmap.is_null() { std::ptr::null_mut() } else { msg_send![bitmap, representationUsingType: 4usize, properties: props] };
                let out = if png.is_null() {
                    Err("PNG encoding failed".to_string())
                } else {
                    let len: usize = msg_send![png, length];
                    let bytes: *const u8 = msg_send![png, bytes];
                    Ok(std::slice::from_raw_parts(bytes, len).to_vec())
                };
                if !bitmap.is_null() {
                    let _: () = msg_send![bitmap, release];
                }
                let _: () = msg_send![image, release];
                done(out)
            }
        });
        unsafe {
            let cfg: *mut AnyObject = msg_send![class!(WKPDFConfiguration), new];
            let _: () = msg_send![v, createPDFWithConfiguration: cfg, completionHandler: &*block];
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
                if out_of_sight(wv, msg) {
                    return done.call(());
                }
                let done = done.copy();
                sheet(wv, msg, frame, &["OK"], None, move |_, _| done.call(()));
            }
            unsafe extern "C-unwind" fn confirm(_: *mut AnyObject, _: Sel, wv: *mut AnyObject, msg: *mut objc2_foundation::NSString, frame: *mut AnyObject, done: &DoneBool) {
                if out_of_sight(wv, msg) {
                    return done.call((Bool::new(false),));
                }
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
                if out_of_sight(wv, msg) {
                    return done.call((std::ptr::null_mut(),));
                }
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
            unsafe extern "C-unwind" fn did_close(_: *mut AnyObject, _: Sel, wv: *mut AnyObject) {
                let window: *mut AnyObject = msg_send![wv, window];
                if window.is_null() {
                    return;
                }
                // Only a pop-up's window closes (a floating one holds the page inset on a pane, not as its content); a tab
                // asking to close must not close Valerie's.
                let content: *mut AnyObject = msg_send![window, contentView];
                let id: *mut objc2_foundation::NSString = msg_send![window, identifier];
                let popup = !id.is_null() && (*id).to_string() == POPUP_ID;
                if content == wv || popup {
                    crate::log::line("page", "a pop-up window closed itself");
                    // Later: closing frees this webview, which WebKit is still calling.
                    let _: () = msg_send![window, performSelector: sel!(close), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: 0.0f64];
                }
            }
            add(sel!(webViewDidClose:), std::mem::transmute::<*const (), Imp>(did_close as *const ()), c"v@:@");
        });
    }

    /// Two things WebKit leaves to the browser, added to wry's navigation delegate class once: a page that says why a
    /// site didn't load (WebKit shows nothing: the tab stays blank, or on the page before), and the name and password
    /// box for sites that sign in over HTTP (without one they only ever answer "401 Unauthorized").
    pub fn install_load_failures(wv: &WebView) {
        use objc2::runtime::{AnyClass, Imp, Sel};
        use objc2::{class, sel};
        use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
        use std::sync::atomic::Ordering::Relaxed;
        type Done = block2::Block<dyn Fn(isize, *mut AnyObject)>;
        unsafe fn text(s: *mut NSString) -> String {
            if s.is_null() { String::new() } else { (*s).to_string() }
        }
        unsafe extern "C-unwind" fn failed(_: *mut AnyObject, _: Sel, wv: *mut AnyObject, _nav: *mut AnyObject, error: *mut AnyObject) {
            if error.is_null() || !super::ERROR_PAGE.load(Relaxed) {
                return;
            }
            let code: isize = msg_send![error, code];
            let domain = text(msg_send![error, domain]);
            let info: *mut AnyObject = msg_send![error, userInfo];
            let url: *mut AnyObject = if info.is_null() { std::ptr::null_mut() } else { msg_send![info, objectForKey: &*NSString::from_str("NSErrorFailingURLKey")] };
            let address = if url.is_null() { String::new() } else { text(msg_send![url, absoluteString]) };
            // Stopped on purpose (another page asked for, a download, a link another app opens): nothing went wrong.
            if domain != "NSURLErrorDomain" || code == -999 || code == -1002 || !(address.starts_with("http://") || address.starts_with("https://")) {
                return;
            }
            let why = text(msg_send![error, localizedDescription]);
            crate::log::line("page", format!("{} didn't load ({code}): {why}", super::host_of(&address)));
            let load = sel!(_loadAlternateHTMLString:baseURL:forUnreachableURL:);
            let can: bool = msg_send![wv, respondsToSelector: load];
            if can {
                // The address stays the one that failed, so reload tries it again and Back goes where it should.
                let html = super::error_page_html(&address, code, &why);
                let _: () = msg_send![wv, _loadAlternateHTMLString: &*NSString::from_str(&html), baseURL: url, forUnreachableURL: url];
            }
        }
        unsafe extern "C-unwind" fn challenge(_: *mut AnyObject, _: Sel, wv: *mut AnyObject, ch: *mut AnyObject, done: &Done) {
            let null = std::ptr::null_mut::<AnyObject>();
            let space: *mut AnyObject = msg_send![ch, protectionSpace];
            let method = if space.is_null() { String::new() } else { text(msg_send![space, authenticationMethod]) };
            let password = matches!(method.as_str(), "NSURLAuthenticationMethodDefault" | "NSURLAuthenticationMethodHTTPBasic" | "NSURLAuthenticationMethodHTTPDigest" | "NSURLAuthenticationMethodNTLM");
            // 1 = what WebKit does by itself (certificates, a name and password the system already holds).
            if !password || !super::HTTP_SIGN_IN.load(Relaxed) {
                return done.call((1, null));
            }
            let failures: isize = msg_send![ch, previousFailureCount];
            let proposed: *mut AnyObject = msg_send![ch, proposedCredential];
            let saved = failures == 0 && !proposed.is_null() && {
                let has: bool = msg_send![proposed, hasPassword];
                has
            };
            if saved {
                return done.call((1, null));
            }
            let host = text(msg_send![space, host]);
            let secure: bool = msg_send![space, receivesCredentialSecurely];
            let done = done.copy();
            let hidden: bool = msg_send![wv, isHiddenOrHasHiddenAncestor];
            if !hidden {
                return ask(wv, host, secure, failures, done);
            }
            // A new page stays hidden until it first draws (pages.wait_for_first_draw), and this one can't draw until
            // it's answered: look once more after Valerie has shown it anyway. Still hidden then (an agent's tab, a
            // background tab): nobody can type into it, so it goes on signed out.
            let _: *mut AnyObject = msg_send![wv, retain];
            let held = std::cell::RefCell::new(Some((host, done)));
            let later = block2::RcBlock::new(move |_timer: *mut AnyObject| {
                let Some((host, done)) = held.borrow_mut().take() else { return };
                let hidden: bool = msg_send![wv, isHiddenOrHasHiddenAncestor];
                if hidden {
                    crate::log::line("page", format!("{host} asked for a name and password in a tab out of sight: left signed out"));
                    done.call((0, std::ptr::null_mut()));
                } else {
                    ask(wv, host, secure, failures, done);
                }
                let _: () = msg_send![wv, release];
            });
            let _: *mut AnyObject = msg_send![class!(NSTimer), scheduledTimerWithTimeInterval: 1.3f64, repeats: false, block: &*later];
        }
        unsafe fn ask(wv: *mut AnyObject, host: String, secure: bool, failures: isize, done: block2::RcBlock<dyn Fn(isize, *mut AnyObject)>) {
            crate::log::line("page", format!("{host} asked for a name and password (HTTP sign-in)"));
            let alert: *mut AnyObject = msg_send![class!(NSAlert), new];
            let _: () = msg_send![alert, setMessageText: &*NSString::from_str(&format!("Sign in to {}", host.trim_start_matches("www.")))];
            let mut note = Vec::new();
            if failures > 0 {
                note.push("That name or password wasn't accepted.");
            }
            if !secure {
                note.push("This connection isn't private: your password is sent without encryption.");
            }
            let _: () = msg_send![alert, setInformativeText: &*NSString::from_str(&note.join(" "))];
            for b in ["Sign In", "Cancel"] {
                let _: *mut AnyObject = msg_send![alert, addButtonWithTitle: &*NSString::from_str(b)];
            }
            let field = |cls: &AnyClass, y: f64, hint: &str| -> *mut AnyObject {
                let f: *mut AnyObject = msg_send![cls, alloc];
                let f: *mut AnyObject = msg_send![f, initWithFrame: NSRect::new(NSPoint::new(0.0, y), NSSize::new(280.0, 24.0))];
                let _: () = msg_send![f, setPlaceholderString: &*NSString::from_str(hint)];
                f
            };
            let (name, pass) = (field(class!(NSTextField), 30.0, "Name"), field(class!(NSSecureTextField), 0.0, "Password"));
            let boxed: *mut AnyObject = msg_send![class!(NSView), alloc];
            let boxed: *mut AnyObject = msg_send![boxed, initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(280.0, 54.0))];
            let _: () = msg_send![boxed, addSubview: name];
            let _: () = msg_send![boxed, addSubview: pass];
            let _: () = msg_send![name, setNextKeyView: pass];
            let _: () = msg_send![alert, setAccessoryView: boxed];
            let sheet: *mut AnyObject = msg_send![alert, window];
            let _: () = msg_send![sheet, setInitialFirstResponder: name];
            let finish = move |code: isize| {
                if code == 1000 {
                    let (n, p): (*mut NSString, *mut NSString) = (msg_send![name, stringValue], msg_send![pass, stringValue]);
                    // 1 = kept until Valerie quits, never written to disk.
                    let cred: *mut AnyObject = msg_send![class!(NSURLCredential), credentialWithUser: n, password: p, persistence: 1usize];
                    done.call((0, cred));
                } else {
                    // 0 with nothing = go on without signing in: the site's own "401" page shows.
                    done.call((0, std::ptr::null_mut()));
                }
                for v in [name, pass, boxed, alert] {
                    let _: () = msg_send![v, release];
                }
            };
            let window: *mut AnyObject = msg_send![wv, window];
            if window.is_null() {
                let code: isize = msg_send![alert, runModal];
                finish(code);
            } else {
                let block = block2::RcBlock::new(move |code: isize| finish(code));
                let _: () = msg_send![alert, beginSheetModalForWindow: window, completionHandler: &*block];
            }
        }
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            let delegate: *mut AnyObject = msg_send![ptr(wv), navigationDelegate];
            if delegate.is_null() {
                return;
            }
            let cls = (*delegate).class() as *const AnyClass as *mut AnyClass;
            let a = objc2::ffi::class_addMethod(cls, sel!(webView:didFailProvisionalNavigation:withError:), std::mem::transmute::<*const (), Imp>(failed as *const ()), c"v@:@@@".as_ptr());
            let b = objc2::ffi::class_addMethod(cls, sel!(webView:didReceiveAuthenticationChallenge:completionHandler:), std::mem::transmute::<*const (), Imp>(challenge as *const ()), c"v@:@@@?".as_ptr());
            if !a.as_bool() || !b.as_bool() {
                crate::log::line("page", "couldn't add the error page or the sign-in box to the page delegate");
            }
        });
    }

    /// Only the webview the mouse is actually over sets the cursor. AppKit sends mouse moves to every view whose area
    /// holds the mouse, covered or not, so the sidebar's full-window view under a page kept setting the arrow while
    /// the page set the text cursor, and the cursor flickered between them. A covered view gets one more move (so
    /// its hover effects end), then none until the mouse is back over it. Added to wry's webview class once.
    pub fn cursor_from_top_view_only(wv: &WebView) {
        use objc2::runtime::{AnyClass, Imp, Sel};
        use objc2::sel;
        use std::{cell::RefCell, collections::HashSet};
        static ONCE: std::sync::Once = std::sync::Once::new();
        thread_local! { static COVERED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new()); }
        unsafe extern "C-unwind" fn mouse_moved(this: *mut AnyObject, _: Sel, event: *mut AnyObject) {
            let covered = || -> bool {
                let window: *mut AnyObject = msg_send![this, window];
                let content: *mut AnyObject = if window.is_null() { std::ptr::null_mut() } else { msg_send![window, contentView] };
                if content.is_null() {
                    return false;
                }
                let frame: *mut AnyObject = msg_send![content, superview];
                let at: objc2_foundation::NSPoint = msg_send![event, locationInWindow];
                let at: objc2_foundation::NSPoint = if frame.is_null() { at } else { msg_send![frame, convertPoint: at, fromView: std::ptr::null_mut::<AnyObject>()] };
                let hit: *mut AnyObject = msg_send![content, hitTest: at];
                !hit.is_null() && !msg_send![hit, isDescendantOf: this]
            };
            let first = COVERED.with(|c| {
                let mut c = c.borrow_mut();
                if covered() {
                    c.insert(this as usize)
                } else {
                    c.remove(&(this as usize));
                    true
                }
            });
            if first {
                if let Some(sup) = AnyClass::get(c"WKWebView") {
                    let _: () = msg_send![super(&*this, sup), mouseMoved: event];
                }
            }
        }
        ONCE.call_once(|| unsafe {
            let cls = AnyClass::get(c"WryWebView").unwrap_or_else(|| (*ptr(wv)).class());
            let added = objc2::ffi::class_addMethod(
                cls as *const AnyClass as *mut AnyClass,
                sel!(mouseMoved:),
                std::mem::transmute::<*const (), Imp>(mouse_moved as *const ()),
                c"v@:@".as_ptr(),
            );
            if !added.as_bool() {
                crate::log::line("cursor", "couldn't add mouseMoved: to the webview class; the cursor may flicker over pages");
            }
        });
    }

    /// The floating video's return button. WebKit asks the UI delegate before the video comes back, on screen or not;
    /// a hidden tab can't be relied on to report it, and only the button itself calls this.
    pub fn on_video_return(wv: &WebView, f: Box<dyn Fn(usize) + Send + Sync>) {
        use objc2::runtime::{AnyClass, Imp, Sel};
        use objc2::sel;
        static RETURN: std::sync::OnceLock<Box<dyn Fn(usize) + Send + Sync>> = std::sync::OnceLock::new();
        unsafe extern "C-unwind" fn may_return(_: *mut AnyObject, _: Sel, wv: *mut AnyObject) {
            if let Some(f) = RETURN.get() {
                f(wv as usize);
            }
        }
        if RETURN.set(f).is_err() {
            return;
        }
        unsafe {
            let delegate: *mut AnyObject = msg_send![ptr(wv), UIDelegate];
            if delegate.is_null() {
                return;
            }
            let cls = (*delegate).class() as *const AnyClass as *mut AnyClass;
            objc2::ffi::class_addMethod(cls, sel!(_webViewFullscreenMayReturnToInline:), std::mem::transmute::<*const (), Imp>(may_return as *const ()), c"v@:@".as_ptr());
        }
    }

    /// WebKit moves a page into its own window for full screen. `f` hears when it's back in Valerie's window.
    pub fn on_page_fullscreen(wv: &WebView, f: Box<dyn Fn() + Send + Sync>) {
        use objc2::runtime::{AnyClass, Imp, Sel};
        use objc2::sel;
        static BACK: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
        // WebKit has just moved the page and nothing animates yet: rounded corners would stay on the video.
        unsafe extern "C-unwind" fn will_enter(_: *mut AnyObject, _: Sel, wv: *mut AnyObject) {
            crate::log::line("fullscreen", "page entering full screen");
            let layer: *mut AnyObject = msg_send![wv, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setCornerRadius: 0.0f64];
                let _: () = msg_send![layer, setMasksToBounds: false];
            }
        }
        unsafe extern "C-unwind" fn did_exit(_: *mut AnyObject, _: Sel, _wv: *mut AnyObject) {
            crate::log::line("fullscreen", "page back in the window");
            crate::quick_fullscreen::finish();
            if let Some(f) = BACK.get() {
                f();
            }
        }
        if BACK.set(f).is_err() {
            return;
        }
        crate::quick_fullscreen::install();
        unsafe {
            let view_class = (*ptr(wv)).class() as *const AnyClass as *mut AnyClass;
            objc2::ffi::class_addMethod(view_class, sel!(convertRect:toView:), std::mem::transmute::<*const (), Imp>(convert_rect as *const ()), c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{CGRect={CGPoint=dd}{CGSize=dd}}@".as_ptr());
            let delegate: *mut AnyObject = msg_send![ptr(wv), UIDelegate];
            if delegate.is_null() {
                return;
            }
            let cls = (*delegate).class() as *const AnyClass as *mut AnyClass;
            let add = |name: Sel, imp: Imp| objc2::ffi::class_addMethod(cls, name, imp, c"v@:@".as_ptr());
            add(sel!(_webViewWillEnterFullscreen:), std::mem::transmute::<*const (), Imp>(will_enter as *const ()));
            add(sel!(_webViewDidExitFullscreen:), std::mem::transmute::<*const (), Imp>(did_exit as *const ()));
        }
    }

    /// WebKit photographs the page before full screen by converting the view's frame as if it were in the view's own
    /// coordinates, which is off by the page's position: beside a sidebar the picture came out shifted, half of it blank.
    unsafe extern "C-unwind" fn convert_rect(this: *mut AnyObject, sel: objc2::runtime::Sel, rect: objc2_foundation::NSRect, to: *mut AnyObject) -> objc2_foundation::NSRect {
        use objc2::class;
        use objc2_foundation::NSRect;
        let frame: NSRect = msg_send![this, frame];
        let is_own_frame = to.is_null() && rect.origin == frame.origin && rect.size == frame.size;
        let rect = if is_own_frame { msg_send![this, bounds] } else { rect };
        let inherited = class!(NSView).instance_method(sel).expect("NSView converts rects").implementation();
        let inherited: unsafe extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel, NSRect, *mut AnyObject) -> NSRect = std::mem::transmute(inherited);
        inherited(this, sel, rect, to)
    }

    /// Every page reports its own full screen changes to the delegate `on_page_fullscreen` set up.
    pub fn watch_page_fullscreen(wv: &WebView) {
        use objc2::sel;
        unsafe {
            let v = ptr(wv);
            let delegate: *mut AnyObject = msg_send![v, UIDelegate];
            let can: bool = msg_send![v, respondsToSelector: sel!(_setFullscreenDelegate:)];
            if can && !delegate.is_null() {
                let _: () = msg_send![v, _setFullscreenDelegate: delegate];
            }
            // Otherwise WebKit keeps a title bar's height free at the top of the page until the animation ends, then the video jumps.
            if msg_send![v, respondsToSelector: sel!(_setAutomaticallyAdjustsContentInsets:)] {
                let _: () = msg_send![v, _setAutomaticallyAdjustsContentInsets: false];
            }
        }
    }

    /// True while WebKit holds this page in its full screen window, where Valerie's layout means nothing.
    pub fn in_page_fullscreen(wv: &WebView) -> bool {
        use objc2::sel;
        unsafe {
            let v = ptr(wv);
            let can: bool = msg_send![v, respondsToSelector: sel!(fullscreenState)];
            if !can {
                return false;
            }
            let state: isize = msg_send![v, fullscreenState];
            state != 0
        }
    }

    pub fn is_webview(wv: &WebView, id: usize) -> bool {
        ptr(wv) as usize == id
    }

    /// Marks the windows `popup_window` makes, so `window.close()` from their page closes them.
    const POPUP_ID: &str = "valerie.popup";

    /// A site's pop-up at the size it asked for ("Sign in with Google"). Wry's own pop-ups have no UI delegate, so
    /// `window.close()` did nothing and they stayed open after signing in; this one shares its opener's delegate.
    /// `floating` gives it Valerie's look: no title bar, the page inset on a frosted pane, and the window buttons in a
    /// strip that slides open while the pointer is at the top edge (dragging that strip moves the window).
    pub fn popup_window<T: objc2::Message>(opener: &impl objc2::Message, configuration: &impl objc2::Message, size: (f64, f64), position: Option<(f64, f64)>, floating: bool) -> Option<objc2::rc::Retained<T>> {
        use objc2::class;
        use objc2_app_kit::{NSBackingStoreType, NSWindowStyleMask};
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        unsafe {
            let opener = opener as *const _ as *mut AnyObject;
            let configuration = configuration as *const _ as *mut AnyObject;
            let parent: *mut AnyObject = msg_send![opener, window];
            if parent.is_null() {
                return None;
            }
            let screen: *mut AnyObject = msg_send![parent, screen];
            if screen.is_null() {
                return None;
            }
            let parent_frame: NSRect = msg_send![parent, frame];
            let screen_frame: NSRect = msg_send![screen, frame];
            let mut origin = match position {
                Some((x, y)) => NSPoint::new(x, screen_frame.size.height - y - size.1),
                None => parent_frame.origin,
            };
            let mut test_spot = None;
            if super::test_windows().is_some() {
                let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
                let count: usize = msg_send![screens, count];
                if count > 1 {
                    let spare: *mut AnyObject = msg_send![screens, objectAtIndex: 1usize];
                    let frame: NSRect = msg_send![spare, frame];
                    origin = NSPoint::new(frame.origin.x + 40.0, frame.origin.y + frame.size.height - size.1 - 80.0);
                    test_spot = Some(origin);
                }
            }
            let rect = NSRect::new(origin, NSSize::new(size.0, size.1));
            let mut style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
            if floating {
                style |= NSWindowStyleMask::FullSizeContentView;
            }
            let window: *mut AnyObject = msg_send![class!(NSWindow), alloc];
            let window: *mut AnyObject = msg_send![window, initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false];
            let webview: *mut AnyObject = msg_send![class!(WKWebView), alloc];
            let webview: *mut AnyObject = msg_send![webview, initWithFrame: rect, configuration: configuration];
            let delegate: *mut AnyObject = msg_send![opener, UIDelegate];
            let _: () = msg_send![webview, setUIDelegate: delegate];
            // The window frees itself, and with it the webview, when it closes.
            let _: () = msg_send![window, setReleasedWhenClosed: true];
            let _: () = msg_send![window, setIdentifier: &*objc2_foundation::NSString::from_str(POPUP_ID)];
            if floating {
                float_popup(window, webview, size);
            } else {
                let _: () = msg_send![window, setContentView: webview];
            }
            let _: () = msg_send![window, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
            // Showing can move it to the display in use.
            if let Some(spot) = test_spot {
                let _: () = msg_send![window, setFrameOrigin: spot];
            }
            objc2::rc::Retained::from_raw(webview.cast())
        }
    }

    /// Dresses a pop-up window as a floating pane (see `popup_window`). The hover watcher goes away with the window.
    unsafe fn float_popup(window: *mut AnyObject, webview: *mut AnyObject, size: (f64, f64)) {
        use objc2::class;
        use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
        use std::{cell::Cell, rc::Rc};
        const BEZEL: f64 = 6.0;
        const STRIP: f64 = 28.0; // the title bar's height, so the buttons sit centred in it
        let _: () = msg_send![window, setTitlebarAppearsTransparent: true];
        let _: () = msg_send![window, setTitleVisibility: 1isize]; // hidden
        let pane: *mut AnyObject = msg_send![class!(NSVisualEffectView), alloc];
        let pane: *mut AnyObject = msg_send![pane, initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.0, size.1))];
        let _: () = msg_send![pane, setMaterial: 7isize]; // sidebar
        let _: () = msg_send![pane, setBlendingMode: 0isize]; // behind the window
        let _: () = msg_send![pane, setState: 1isize]; // always active
        let _: () = msg_send![window, setContentView: pane];
        let _: () = msg_send![pane, release];
        let page_frame = |open: bool, (w, h): (f64, f64)| {
            let top = if open { STRIP } else { BEZEL };
            NSRect::new(NSPoint::new(BEZEL, BEZEL), NSSize::new((w - 2.0 * BEZEL).max(0.0), (h - BEZEL - top).max(0.0)))
        };
        let _: () = msg_send![webview, setFrame: page_frame(false, size)];
        let _: () = msg_send![webview, setAutoresizingMask: 18usize]; // width and height follow the window
        let _: () = msg_send![webview, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![webview, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: 10.0f64];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
        let _: () = msg_send![pane, addSubview: webview];
        let set_buttons = move |hidden: bool| {
            for b in 0usize..3 {
                let btn: *mut AnyObject = msg_send![window, standardWindowButton: b];
                if !btn.is_null() {
                    let _: () = msg_send![btn, setHidden: hidden];
                }
            }
        };
        set_buttons(true);
        // Mouse moves and exits over the whole window, even while it isn't the key window.
        let area: *mut AnyObject = msg_send![class!(NSTrackingArea), alloc];
        let area: *mut AnyObject = msg_send![area, initWithRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)), options: (0x01usize | 0x02 | 0x80 | 0x200), owner: pane, userInfo: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![pane, addTrackingArea: area];
        let _: () = msg_send![area, release];
        let open = Rc::new(Cell::new(false));
        let watch = block2::RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
            let frame: NSRect = msg_send![window, frame];
            let at: NSPoint = msg_send![class!(NSEvent), mouseLocation];
            let inside = at.x >= frame.origin.x && at.x <= frame.origin.x + frame.size.width && at.y >= frame.origin.y && at.y <= frame.origin.y + frame.size.height;
            let from_top = frame.origin.y + frame.size.height - at.y;
            // A little past the bezel opens the strip; once open, the whole strip keeps it open.
            let want = inside && from_top <= if open.get() { STRIP } else { BEZEL + 6.0 };
            if want != open.get() {
                open.set(want);
                set_buttons(!want);
                let content: *mut AnyObject = msg_send![window, contentView];
                let bounds: NSRect = msg_send![content, bounds];
                let to = page_frame(want, (bounds.size.width, bounds.size.height));
                let animator: *mut AnyObject = msg_send![webview, animator];
                let _: () = msg_send![animator, setFrame: to];
            }
            event
        });
        // Mouse moved, left-dragged, entered and exited.
        let mask: u64 = (1 << 5) | (1 << 6) | (1 << 8) | (1 << 9);
        let monitor: *mut AnyObject = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: mask, handler: &*watch];
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let token = Rc::new(Cell::new(std::ptr::null_mut::<AnyObject>()));
        let token2 = token.clone();
        let closing = block2::RcBlock::new(move |_note: *mut AnyObject| {
            let _: () = msg_send![class!(NSEvent), removeMonitor: monitor];
            let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
            let t = token2.replace(std::ptr::null_mut());
            if !t.is_null() {
                let _: () = msg_send![center, removeObserver: t];
            }
        });
        let name = NSString::from_str("NSWindowWillCloseNotification");
        let t: *mut AnyObject = msg_send![center, addObserverForName: &*name, object: window, queue: std::ptr::null_mut::<AnyObject>(), usingBlock: &*closing];
        token.set(t);
    }

    /// Nobody can answer a dialog from a tab they can't see (an agent's hidden tab, a page being read, a background
    /// tab), and the page freezes until someone does: it gets "Cancel".
    unsafe fn out_of_sight(webview: *mut AnyObject, msg: *mut objc2_foundation::NSString) -> bool {
        let hidden: bool = msg_send![webview, isHiddenOrHasHiddenAncestor];
        if hidden {
            let text = if msg.is_null() { String::new() } else { (*msg).to_string() };
            crate::log::line("page", format!("dismissed a dialog from a tab out of sight: {}", text.chars().take(120).collect::<String>()));
        }
        hidden
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

    // ---------------------------------------------------------------- location (CoreLocation.framework)

    #[link(name = "CoreLocation", kind = "framework")]
    extern "C" {}

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Coordinate {
        latitude: f64,
        longitude: f64,
    }

    unsafe impl objc2::Encode for Coordinate {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct("CLLocationCoordinate2D", &[objc2::Encoding::Double, objc2::Encoding::Double]);
    }

    thread_local! {
        static LOCATOR: std::cell::Cell<*mut AnyObject> = const { std::cell::Cell::new(std::ptr::null_mut()) };
        static LOCATED: std::cell::RefCell<Option<Box<dyn FnOnce(serde_json::Value)>>> = const { std::cell::RefCell::new(None) };
        static LOOKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    fn located(answer: serde_json::Value) {
        LOOKING.set(false);
        if let Some(done) = LOCATED.with(|d| d.borrow_mut().take()) {
            done(answer);
        }
    }

    fn location_error(code: i64, message: &str) -> serde_json::Value {
        serde_json::json!({ "error": code, "message": message })
    }

    /// Carry on with what the system allows now: look, keep waiting for its question to be answered, or say no.
    unsafe fn locate_if_allowed(manager: *mut AnyObject) {
        if LOOKING.get() || LOCATED.with(|d| d.borrow().is_none()) {
            return;
        }
        let status: i32 = msg_send![manager, authorizationStatus];
        match status {
            0 => {
                let _: () = msg_send![manager, requestWhenInUseAuthorization];
            }
            3 | 4 => {
                LOOKING.set(true);
                let _: () = msg_send![manager, requestLocation];
            }
            _ => {
                crate::log::line("location", "the system doesn't allow Valerie to use location");
                located(location_error(1, "Location is turned off for Valerie"));
            }
        }
    }

    /// Where the computer is, once; the system asks the user the first time. Call on the main thread, one at a time.
    pub fn locate(done: Box<dyn FnOnce(serde_json::Value)>) {
        use objc2::runtime::{AnyClass, Imp, Sel};
        use objc2::{class, sel};
        if !bundled() {
            return done(location_error(2, "Location needs Valerie to run as an app"));
        }
        unsafe extern "C-unwind" fn changed(_: *mut AnyObject, _: Sel, manager: *mut AnyObject) {
            locate_if_allowed(manager);
        }
        unsafe extern "C-unwind" fn updated(_: *mut AnyObject, _: Sel, _: *mut AnyObject, places: *mut AnyObject) {
            let place: *mut AnyObject = msg_send![places, lastObject];
            if place.is_null() {
                return located(location_error(2, "The computer couldn't find its location"));
            }
            let at: Coordinate = msg_send![place, coordinate];
            let accuracy: f64 = msg_send![place, horizontalAccuracy];
            let altitude: f64 = msg_send![place, altitude];
            let altitude_accuracy: f64 = msg_send![place, verticalAccuracy];
            let heading: f64 = msg_send![place, course];
            let speed: f64 = msg_send![place, speed];
            let time: *mut AnyObject = msg_send![place, timestamp];
            let seconds: f64 = msg_send![time, timeIntervalSince1970];
            // Negative means the computer doesn't know, which pages read as null.
            let known = |v: f64| (v >= 0.0).then_some(v);
            located(serde_json::json!({
                "latitude": at.latitude, "longitude": at.longitude, "accuracy": accuracy,
                "altitude": known(altitude_accuracy).map(|_| altitude), "altitudeAccuracy": known(altitude_accuracy),
                "heading": known(heading), "speed": known(speed), "timestamp": (seconds * 1000.0).round(),
            }));
        }
        unsafe extern "C-unwind" fn failed(_: *mut AnyObject, _: Sel, _: *mut AnyObject, error: *mut AnyObject) {
            let code: isize = msg_send![error, code];
            let text: *mut objc2_foundation::NSString = msg_send![error, localizedDescription];
            let text = if text.is_null() { String::new() } else { (*text).to_string() };
            crate::log::line("location", format!("the computer couldn't find its location: {text}"));
            // kCLErrorDenied is 1.
            located(if code == 1 { location_error(1, "Location is turned off for Valerie") } else { location_error(2, "The computer couldn't find its location") });
        }
        LOCATED.with(|d| *d.borrow_mut() = Some(done));
        unsafe {
            let mut manager = LOCATOR.get();
            if manager.is_null() {
                let name = c"ValerieLocationDelegate";
                let cls: *const AnyClass = match AnyClass::get(name) {
                    Some(c) => c,
                    None => {
                        let c = objc2::ffi::objc_allocateClassPair(class!(NSObject), name.as_ptr(), 0);
                        if c.is_null() {
                            return located(location_error(2, "The computer couldn't find its location"));
                        }
                        let add = |s: Sel, f: *const (), types: &std::ffi::CStr| {
                            objc2::ffi::class_addMethod(c, s, std::mem::transmute::<*const (), Imp>(f), types.as_ptr());
                        };
                        add(sel!(locationManagerDidChangeAuthorization:), changed as *const (), c"v@:@");
                        add(sel!(locationManager:didUpdateLocations:), updated as *const (), c"v@:@@");
                        add(sel!(locationManager:didFailWithError:), failed as *const (), c"v@:@@");
                        objc2::ffi::objc_registerClassPair(c);
                        c
                    }
                };
                // The manager holds its delegate weakly; both live for the app's life.
                let delegate: *mut AnyObject = msg_send![cls, new];
                manager = msg_send![class!(CLLocationManager), new];
                let _: () = msg_send![manager, setDelegate: delegate];
                let _: () = msg_send![manager, setDesiredAccuracy: 100.0f64];
                LOCATOR.set(manager);
            }
            locate_if_allowed(manager);
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

    /// A folder window (NSOpenPanel); the chosen folder, or None if cancelled.
    pub fn pick_folder(message: &str) -> Option<std::path::PathBuf> {
        use objc2::class;
        unsafe {
            let panel: *mut AnyObject = msg_send![class!(NSOpenPanel), openPanel];
            if panel.is_null() {
                return None;
            }
            let _: () = msg_send![panel, setCanChooseDirectories: true];
            let _: () = msg_send![panel, setCanChooseFiles: false];
            let _: () = msg_send![panel, setAllowsMultipleSelection: false];
            let _: () = msg_send![panel, setCanCreateDirectories: true];
            let m = objc2_foundation::NSString::from_str(message);
            let _: () = msg_send![panel, setMessage: &*m];
            let r: isize = msg_send![panel, runModal];
            if r != 1 {
                return None;
            }
            let url: *mut AnyObject = msg_send![panel, URL];
            if url.is_null() {
                return None;
            }
            let path: *mut objc2_foundation::NSString = msg_send![url, path];
            path.as_ref().map(|p| std::path::PathBuf::from(p.to_string()))
        }
    }

    /// The share menu (NSSharingServicePicker) for a page's address, pointing at the top right of the page.
    pub fn share(wv: &WebView, url: &str) {
        use objc2::class;
        unsafe {
            let v = ptr(wv);
            let s = objc2_foundation::NSString::from_str(url);
            let nsurl: *mut AnyObject = msg_send![class!(NSURL), URLWithString: &*s];
            if nsurl.is_null() {
                return;
            }
            let items: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: nsurl];
            let picker: *mut AnyObject = msg_send![class!(NSSharingServicePicker), alloc];
            let picker: *mut AnyObject = msg_send![picker, initWithItems: items];
            if picker.is_null() {
                return;
            }
            let b: objc2_foundation::NSRect = msg_send![v, bounds];
            let flipped: bool = msg_send![v, isFlipped];
            // Just under the top edge, near the right; the menu opens downward.
            let x = (b.size.width - 80.0).max(0.0);
            let (y, edge) = if flipped { (6.0, 3usize) } else { (b.size.height - 6.0, 1usize) };
            let at = objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(x, y), objc2_foundation::NSSize::new(1.0, 1.0));
            let _: () = msg_send![picker, showRelativeToRect: at, ofView: v, preferredEdge: edge];
        }
    }

    /// A file window (NSOpenPanel) for opening files; the chosen files, empty if cancelled.
    pub fn pick_files(message: &str) -> Vec<std::path::PathBuf> {
        use objc2::class;
        unsafe {
            let panel: *mut AnyObject = msg_send![class!(NSOpenPanel), openPanel];
            if panel.is_null() {
                return Vec::new();
            }
            let _: () = msg_send![panel, setCanChooseDirectories: false];
            let _: () = msg_send![panel, setCanChooseFiles: true];
            let _: () = msg_send![panel, setAllowsMultipleSelection: true];
            let m = objc2_foundation::NSString::from_str(message);
            let _: () = msg_send![panel, setMessage: &*m];
            let r: isize = msg_send![panel, runModal];
            if r != 1 {
                return Vec::new();
            }
            let urls: *mut AnyObject = msg_send![panel, URLs];
            if urls.is_null() {
                return Vec::new();
            }
            let n: usize = msg_send![urls, count];
            (0..n)
                .filter_map(|i| {
                    let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                    let path: *mut objc2_foundation::NSString = msg_send![url, path];
                    path.as_ref().map(|p| std::path::PathBuf::from(p.to_string()))
                })
                .collect()
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
        // A test instance has its own marker but shares the real app's login stores: it must never write to them.
        if marker.exists() || !bundled() || crate::update::is_test_instance() {
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
        let bare_running = crate::platform::command("ps").args(["-Ao", "comm="]).output().is_ok_and(|o| {
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
                let ok = crate::platform::command("ditto").arg(entry.path()).arg(&dest).status().is_ok_and(|s| s.success());
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

    /// Reload from the server, skipping anything cached.
    pub fn hard_reload(wv: &WebView) {
        let _: *mut AnyObject = unsafe { msg_send![ptr(wv), reloadFromOrigin] };
    }

    /// Remove every site's cache (and with `cookies`, cookies and storage too) from this webview's profile.
    pub fn clear_browsing_data(wv: &WebView, cookies: bool, done: impl FnOnce(bool) + 'static) {
        use objc2::class;
        use objc2_foundation::NSString;
        let v = ptr(wv);
        unsafe {
            let config: *mut AnyObject = msg_send![v, configuration];
            let store: *mut AnyObject = msg_send![config, websiteDataStore];
            let _: *mut AnyObject = msg_send![store, retain];
            let types: *mut AnyObject = if cookies {
                msg_send![class!(WKWebsiteDataStore), allWebsiteDataTypes]
            } else {
                // The WKWebsiteDataType… constants are these same strings.
                let set: *mut AnyObject = msg_send![class!(NSMutableSet), set];
                for t in ["WKWebsiteDataTypeDiskCache", "WKWebsiteDataTypeMemoryCache", "WKWebsiteDataTypeFetchCache", "WKWebsiteDataTypeOfflineWebApplicationCache"] {
                    let t = NSString::from_str(t);
                    let _: () = msg_send![set, addObject: &*t];
                }
                set
            };
            let since: *mut AnyObject = msg_send![class!(NSDate), distantPast];
            let done = std::cell::RefCell::new(Some(done));
            let finished = block2::RcBlock::new(move || {
                let _: () = msg_send![store, release];
                if let Some(d) = done.borrow_mut().take() {
                    d(true)
                }
            });
            let _: () = msg_send![store, removeDataOfTypes: types, modifiedSince: since, completionHandler: &*finished];
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

    /// Move the window's top edge up by `dy` points (down when negative), gliding over `secs`, the bottom staying put;
    /// false when growing would pass the top of the screen's usable area.
    pub fn grow_window_top(window: &tao::window::Window, dy: f64, secs: f64) -> bool {
        use objc2_foundation::NSRect;
        use tao::platform::macos::WindowExtMacOS;
        let w = window.ns_window() as *mut AnyObject;
        unsafe {
            let mut frame: NSRect = msg_send![w, frame];
            let screen: *mut AnyObject = msg_send![w, screen];
            if dy > 0.0 && !screen.is_null() {
                let usable: NSRect = msg_send![screen, visibleFrame];
                if frame.origin.y + frame.size.height + dy > usable.origin.y + usable.size.height + 0.5 {
                    return false;
                }
            }
            frame.size.height += dy;
            if secs > 0.0 {
                animate_curve(secs, "easeOut", || {
                    let animator: *mut AnyObject = msg_send![w, animator];
                    let _: () = msg_send![animator, setFrame: frame, display: true];
                });
            } else {
                let _: () = msg_send![w, setFrame: frame, display: true];
            }
        }
        true
    }

    /// Do what System Settings says a title-bar double-click does: zoom (also for "Fill"), minimize or nothing.
    pub fn title_bar_double_click(window: &tao::window::Window) {
        use tao::platform::macos::WindowExtMacOS;
        let w = window.ns_window() as *mut AnyObject;
        unsafe {
            let defaults: *mut AnyObject = msg_send![objc2::class!(NSUserDefaults), standardUserDefaults];
            let key = objc2_foundation::NSString::from_str("AppleActionOnDoubleClick");
            let action: Option<objc2::rc::Retained<objc2_foundation::NSString>> = msg_send![defaults, stringForKey: &*key];
            match action.map(|a| a.to_string()).as_deref() {
                Some("Minimize") => {
                    let _: () = msg_send![w, miniaturize: std::ptr::null::<AnyObject>()];
                }
                Some("None") => {}
                _ => {
                    let _: () = msg_send![w, zoom: std::ptr::null::<AnyObject>()];
                }
            }
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

    /// Moves the red / yellow / green buttons so their centres sit `centre` points below the window's top, shifting
    /// them right by as much as they move down so the gaps to the top and left edges stay equal to each other.
    pub fn place_traffic_lights(window: &tao::window::Window, centre: f64) {
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        use std::sync::OnceLock;
        use tao::platform::macos::WindowExtMacOS;
        static DEFAULT: OnceLock<(f64, f64)> = OnceLock::new(); // where macOS first put the close button: (left, top)
        let w = window.ns_window() as *mut AnyObject;
        unsafe {
            let close: *mut AnyObject = msg_send![w, standardWindowButton: 0usize];
            let mini: *mut AnyObject = msg_send![w, standardWindowButton: 1usize];
            if close.is_null() || mini.is_null() {
                return;
            }
            let bar: *mut AnyObject = msg_send![close, superview];
            let container: *mut AnyObject = if bar.is_null() { bar } else { msg_send![bar, superview] };
            if container.is_null() {
                return;
            }
            let win_frame: NSRect = msg_send![w, frame];
            let b: NSRect = msg_send![close, frame];
            let m: NSRect = msg_send![mini, frame];
            let spacing = m.origin.x - b.origin.x;
            let &(left, top) = DEFAULT.get_or_init(|| {
                let r: NSRect = msg_send![bar, convertRect: b, toView: std::ptr::null_mut::<AnyObject>()];
                (r.origin.x, win_frame.size.height - (r.origin.y + r.size.height))
            });
            let down = centre - b.size.height / 2.0 - top;
            let (left, top) = (left + down, top + down);
            // A strip as tall as the buttons plus the same gap above and below, so flipped or not they land at `top`.
            let height = top * 2.0 + b.size.height;
            let strip = NSRect::new(NSPoint::new(0.0, win_frame.size.height - height), NSSize::new(win_frame.size.width, height));
            let _: () = msg_send![container, setFrame: strip];
            for i in 0usize..3 {
                let btn: *mut AnyObject = msg_send![w, standardWindowButton: i];
                if !btn.is_null() {
                    let _: () = msg_send![btn, setFrameOrigin: NSPoint::new(left + i as f64 * spacing, top)];
                }
            }
        }
    }

    pub fn allow_magnification(wv: &WebView, on: bool) {
        unsafe {
            let _: () = msg_send![ptr(wv), setAllowsMagnification: on];
        }
    }

    /// The view's appearance decides `prefers-color-scheme`: dark asks sites for their dark look; none follows the Mac.
    pub fn prefer_dark(wv: &WebView, dark: bool) {
        unsafe {
            let look: *mut AnyObject = if dark {
                let name = objc2_foundation::NSString::from_str("NSAppearanceNameDarkAqua");
                msg_send![objc2::class!(NSAppearance), appearanceNamed: &*name]
            } else {
                std::ptr::null_mut()
            };
            let _: () = msg_send![ptr(wv), setAppearance: look];
        }
    }

    pub fn make_floating_panel(window: &tao::window::Window) {
        use objc2::runtime::{AnyClass, Bool, ClassBuilder, Sel};
        use objc2::{class, sel};
        use tao::platform::macos::WindowExtMacOS;
        static PANEL: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
        extern "C-unwind" fn yes(_: &AnyObject, _: Sel) -> Bool {
            Bool::YES
        }
        extern "C-unwind" fn no(_: &AnyObject, _: Sel) -> Bool {
            Bool::NO
        }
        let cls = *PANEL.get_or_init(|| {
            let mut b = ClassBuilder::new(c"ValerieFloatingPanel", class!(NSPanel)).expect("panel class");
            // The same field tao's window class adds, so the two are the same size (and tao can still read it).
            b.add_ivar::<Bool>(c"focusable");
            unsafe {
                b.add_method(sel!(canBecomeKeyWindow), yes as extern "C-unwind" fn(_, _) -> _);
                b.add_method(sel!(canBecomeMainWindow), no as extern "C-unwind" fn(_, _) -> _);
            }
            b.register()
        });
        let w = window.ns_window() as *mut AnyObject;
        unsafe {
            // tao made an NSWindow; a non-activating panel is what lets it take keys while another app stays in front.
            AnyObject::set_class(&*w, cls);
            let mask: usize = msg_send![w, styleMask];
            let _: () = msg_send![w, setStyleMask: mask | (1 << 7)];
            let _: () = msg_send![w, setFloatingPanel: true];
            let _: () = msg_send![w, setHidesOnDeactivate: false];
            let _: () = msg_send![w, setLevel: 25isize];
            // Every Space, and over full-screen apps.
            let _: () = msg_send![w, setCollectionBehavior: (1usize << 0) | (1 << 8)];
            let _: () = msg_send![w, setHasShadow: true];
        }
    }

    pub fn show_floating_panel(window: &tao::window::Window) {
        use tao::platform::macos::WindowExtMacOS;
        let w = window.ns_window() as *mut AnyObject;
        unsafe {
            let _: () = msg_send![w, orderFrontRegardless];
            let _: () = msg_send![w, makeKeyWindow];
            let _: () = msg_send![w, invalidateShadow];
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

/// This app's WebKit graphics process and how many IOSurfaces it holds. None off macOS, or if macOS won't say.
#[cfg(target_os = "macos")]
pub fn graphics_buffers() -> Option<(i32, u64)> {
    extern "C" {
        fn proc_listallpids(buffer: *mut std::ffi::c_void, size: i32) -> i32;
        fn proc_name(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32;
        fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    }
    const RTLD_DEFAULT: *mut std::ffi::c_void = -2isize as *mut std::ffi::c_void;
    // Private but long-standing, so looked up: if a macOS removes it, this just reports nothing.
    let symbol = unsafe { dlsym(RTLD_DEFAULT, c"responsibility_get_pid_responsible_for_pid".as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    let responsible: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(symbol) };
    let mut pids = vec![0i32; 8192];
    let found = unsafe { proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
    // The process answers to whatever started us: ourselves for an app, the terminal for a program run from one.
    let me = responsible(std::process::id() as i32);
    let pid = pids.into_iter().take(found.max(0) as usize).find(|&pid| {
        let mut name = [0u8; 64];
        let n = unsafe { proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
        n > 0 && &name[..n as usize] == b"com.apple.WebKit.GPU" && responsible(pid) == me
    })?;
    let out = command("/usr/bin/footprint").args(["-p", &pid.to_string()]).output().ok()?;
    Some((pid, surfaces_in_footprint(&String::from_utf8_lossy(&out.stdout))?))
}

#[cfg(not(target_os = "macos"))]
pub fn graphics_buffers() -> Option<(i32, u64)> {
    None
}

/// The count on `footprint`'s IOSurface row: "113 MB  0 B  2846 MB  3290  IOSurface".
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn surfaces_in_footprint(report: &str) -> Option<u64> {
    let row = report.lines().find(|l| l.trim_end().ends_with(" IOSurface"))?;
    row.split_whitespace().rev().nth(1)?.parse().ok()
}

/// Ends a helper process of ours; WebKit starts a new one by itself.
pub fn stop_process(pid: i32) {
    #[cfg(unix)]
    {
        extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        const SIGKILL: i32 = 9;
        unsafe { kill(pid, SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = pid;
}

#[cfg(test)]
mod graphics_tests {
    #[test]
    fn reads_the_surface_count() {
        let report = "   Dirty  Clean  Reclaimable  Regions  Category\n 113 MB  0 B  2846 MB  3290    IOSurface\n";
        assert_eq!(super::surfaces_in_footprint(report), Some(3290));
        assert_eq!(super::surfaces_in_footprint("no such row"), None);
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

/// The process id of the app in front (macOS), so a global shortcut can hand focus back to it afterwards.
pub fn frontmost_app() -> Option<i32> {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::{class, msg_send};
        let ws: *mut objc2::runtime::AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: *mut objc2::runtime::AnyObject = msg_send![ws, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let pid: i32 = msg_send![app, processIdentifier];
        Some(pid)
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// Make a borderless window a panel that floats over every app and takes the keyboard without bringing the rest of
/// Valerie forward (the command bar opened from another app). macOS only.
pub fn make_floating_panel(window: &tao::window::Window) {
    #[cfg(target_os = "macos")]
    mac::make_floating_panel(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Show a floating panel in front of every app and give it the keyboard, leaving the app in front as it is.
pub fn show_floating_panel(window: &tao::window::Window) {
    #[cfg(target_os = "macos")]
    mac::show_floating_panel(window);
    #[cfg(not(target_os = "macos"))]
    window.set_visible(true);
}

/// Pinch to zoom a page in and out (gestures.pinch_to_zoom).
pub fn allow_magnification(wv: &WebView, on: bool) {
    #[cfg(target_os = "macos")]
    mac::allow_magnification(wv, on);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, on);
}

/// Ask sites for their own dark look (pages.dark_mode: always), or let them follow the computer.
pub fn prefer_dark(wv: &WebView, dark: bool) {
    #[cfg(target_os = "macos")]
    mac::prefer_dark(wv, dark);
    #[cfg(target_os = "windows")]
    {
        use wry::WebViewExtWindows;
        let _ = wv.set_theme(if dark { wry::Theme::Dark } else { wry::Theme::Auto });
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (wv, dark);
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
    // macOS only so far: nothing failed, and telling people otherwise on every launch would only nag.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (keys, on);
        Vec::new()
    }
}

/// The page's address. wry's own `url()` panics on macOS when the page has none (a little window opened straight onto a
/// download), so ask this instead.
pub fn page_url(wv: &WebView) -> Option<String> {
    #[cfg(target_os = "macos")]
    if !mac::has_url(wv) {
        return None;
    }
    wv.url().ok()
}

/// Cut a page's sound right away; call just before dropping its view.
/// A builder for Valerie's own pages (command bar, chat, little window bars...): on macOS they share the sidebar's web
/// process instead of each starting one of their own.
pub fn ui_builder<'a>(sidebar: &WebView) -> wry::WebViewBuilder<'a> {
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtMacos;
        let config = mac::sharing_process(sidebar);
        wry::WebViewBuilder::new().with_webview_configuration(unsafe { objc2::rc::Retained::cast_unchecked(config) })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = sidebar;
        wry::WebViewBuilder::new()
    }
}

/// A page's configuration on macOS: the profile's storage, extensions if any, and (with `mate`) sharing that page's
/// web process.
#[cfg(target_os = "macos")]
pub fn page_configuration(id: [u8; 16], extensions: Option<objc2::rc::Retained<objc2::runtime::AnyObject>>, mate: Option<&WebView>, slow_hidden: bool) -> Option<objc2::rc::Retained<objc2::runtime::AnyObject>> {
    let config = extensions.or_else(|| (mate.is_some() || slow_hidden).then(|| mac::profile_configuration(id)))?;
    if let Some(m) = mate {
        mac::relate(&config, m);
    }
    if slow_hidden {
        mac::slow_hidden_timers(&config);
    }
    Some(config)
}

/// The system and its version, for the log ("macOS Version 26.5.2 (Build 25F84)").
pub fn os_version() -> String {
    #[cfg(target_os = "macos")]
    unsafe {
        let info: *mut objc2::runtime::AnyObject = objc2::msg_send![objc2::class!(NSProcessInfo), processInfo];
        let v: *mut objc2_foundation::NSString = objc2::msg_send![info, operatingSystemVersionString];
        if !v.is_null() {
            return format!("macOS {}", (*v).to_string());
        }
    }
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn bench_in_view(wv: &WebView) {
    #[cfg(target_os = "macos")]
    if std::env::var_os("VALERIE_BENCH").is_some() {
        mac::bench_in_view(wv);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = wv;
}

/// A real mouse click in the page at (x, y) CSS pixels, which sites trust like the user's own (macOS and Windows; Linux
/// says it can't). Calls `done` later on the main thread.
pub fn real_click(wv: &WebView, x: f64, y: f64, right: bool, clicks: isize, done: impl FnOnce(Result<(), String>) + 'static) {
    #[cfg(target_os = "macos")]
    done(mac::real_click(wv, x, y, right, clicks));
    #[cfg(target_os = "windows")]
    win::real_click(wv, x, y, right, clicks, Box::new(done));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (wv, x, y, right, clicks);
        done(Err("real clicks aren't available on this system".into()));
    }
}

pub fn silence(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::silence(wv);
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::WebViewExt;
        use wry::WebViewExtUnix;
        wv.webview().set_is_muted(true);
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
        use windows::core::Interface;
        use wry::WebViewExtWindows;
        if let Ok(page) = wv.controller().CoreWebView2().and_then(|p| p.cast::<ICoreWebView2_8>()) {
            let _ = page.SetIsMuted(true);
        }
    }
}

pub fn bring_to_front(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::bring_to_front(wv);
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        if let Some(window) = wv.webview().toplevel().and_then(|t| t.window()) {
            window.raise();
        }
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};
        use wry::WebViewExtWindows;
        let mut hwnd = HWND::default();
        if wv.controller().ParentWindow(&mut hwnd).is_ok() {
            let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

/// Put a view back under the others (the sidebar's view, after it floated over the pages).
pub fn send_to_back(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::send_to_back(wv);
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};
        use wry::WebViewExtWindows;
        let mut hwnd = HWND::default();
        if wv.controller().ParentWindow(&mut hwnd).is_ok() {
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
    #[cfg(target_os = "linux")]
    let _ = wv;
}

/// Moves a live page into another window, as it is (no reload). False where that can't be done here (Linux).
pub fn move_view_to(wv: &WebView, window: &tao::window::Window) -> bool {
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowExtMacOS;
        use wry::WebViewExtMacOS;
        return wv.reparent(window.ns_window() as _).is_ok();
    }
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        use wry::WebViewExtWindows;
        return wv.reparent(window.hwnd() as isize).is_ok();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = (wv, window);
        false
    }
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

/// How a view stretches while the window is live-resized, in the same frame, before `layout` sets its exact bounds.
#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum Follow {
    Fill,
    Pane { first: bool, last: bool },
    Top,
    Bottom,
    TopRight,
    /// Full height and fixed width, against the left or right edge.
    Side { right: bool },
}

pub fn follow_window(wv: &WebView, follow: Follow) {
    #[cfg(target_os = "macos")]
    mac::follow_window(wv, follow);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, follow);
}

/// Move a window's top edge up by `dy` logical pixels (down when negative), its bottom staying put; false when there's no
/// room on the screen above it. On macOS it glides there over `secs`; elsewhere it moves in one step.
pub fn grow_window_top(window: &tao::window::Window, dy: f64, secs: f64) -> bool {
    #[cfg(target_os = "macos")]
    return mac::grow_window_top(window, dy, secs);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = secs;
        let scale = window.scale_factor();
        let (Ok(pos), size) = (window.outer_position(), window.inner_size()) else { return false };
        let pos = pos.to_logical::<f64>(scale);
        let size = size.to_logical::<f64>(scale);
        if dy > 0.0 {
            let top = window.current_monitor().map(|m| m.position().to_logical::<f64>(m.scale_factor()).y).unwrap_or(0.0);
            if pos.y - dy < top {
                return false;
            }
        }
        window.set_outer_position(tao::dpi::LogicalPosition::new(pos.x, pos.y - dy));
        window.set_inner_size(tao::dpi::LogicalSize::new(size.width, size.height + dy));
        true
    }
}

/// The window's title bar was double-clicked: zoom or minimize as the system is set to.
pub fn title_bar_double_click(window: &tao::window::Window) {
    #[cfg(target_os = "macos")]
    mac::title_bar_double_click(window);
    #[cfg(not(target_os = "macos"))]
    window.set_maximized(!window.is_maximized());
}

pub fn set_traffic_lights_hidden(window: &tao::window::Window, hidden: bool) {
    #[cfg(target_os = "macos")]
    mac::set_traffic_lights_hidden(window, hidden);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, hidden);
}

/// Lines the window buttons up with a row whose centre is `centre` points from the top (macOS only).
pub fn place_traffic_lights(window: &tao::window::Window, centre: f64) {
    #[cfg(target_os = "macos")]
    mac::place_traffic_lights(window, centre);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, centre);
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

/// Chrome's user agent for this system, with a version that keeps pace with Chrome's four-week releases.
pub fn chrome_user_agent() -> String {
    // Chrome 142 shipped 2025-10-28.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let version = 142 + secs.saturating_sub(1_761_609_600) / (28 * 86_400);
    let system = if cfg!(target_os = "windows") {
        "Windows NT 10.0; Win64; x64"
    } else if cfg!(target_os = "linux") {
        "X11; Linux x86_64"
    } else {
        "Macintosh; Intel Mac OS X 10_15_7"
    };
    format!("Mozilla/5.0 ({system}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version}.0.0.0 Safari/537.36")
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

/// Screenshots on Windows, through WebView2's DevTools protocol (`Page.captureScreenshot`).
#[cfg(target_os = "windows")]
mod win {
    use serde_json::{json, Value};
    use std::{cell::RefCell, rc::Rc};
    use webview2_com::{CallDevToolsProtocolMethodCompletedHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2};
    use windows::core::HSTRING;
    use wry::{WebView, WebViewExtWindows};

    type Done = Box<dyn FnOnce(Result<Vec<u8>, String>)>;

    /// Run one DevTools method; `done` gets its JSON result (also when the call can't start).
    fn cdp(wv: &ICoreWebView2, method: &str, params: Value, done: impl FnOnce(Result<Value, String>) + 'static) {
        let done = Rc::new(RefCell::new(Some(done)));
        let d = done.clone();
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |hr, json| {
            if let Some(done) = d.borrow_mut().take() {
                done(hr.map_err(|e| e.message()).and_then(|_| serde_json::from_str(&json).map_err(|e| e.to_string())));
            }
            Ok(())
        }));
        let r = unsafe { wv.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params.to_string()), &handler) };
        if let Err(e) = r {
            if let Some(done) = done.borrow_mut().take() {
                done(Err(e.message()));
            }
        }
    }

    /// Measure the page, then capture the region `clip` picks from the layout metrics.
    fn capture(wv: &WebView, done: Done, clip: impl FnOnce(&Value) -> Value + 'static) {
        let w = wv.webview();
        let w2 = w.clone();
        cdp(&w, "Page.getLayoutMetrics", json!({}), move |m| {
            let m = match m {
                Ok(m) => m,
                Err(e) => return done(Err(format!("the page could not be measured: {e}"))),
            };
            cdp(&w2, "Page.captureScreenshot", clip(&m), move |r| {
                let png = r.and_then(|r| {
                    let b64 = r["data"].as_str().ok_or("no picture came back")?.to_string();
                    crate::mcp::unbase64(&b64).ok_or_else(|| "the picture came back damaged".to_string())
                });
                done(png.map_err(|e| format!("the page could not be captured: {e}")))
            });
        });
    }

    pub fn snapshot(wv: &WebView, width: f64, done: Done) {
        capture(wv, done, move |m| {
            let v = &m["cssVisualViewport"];
            let vw = v["clientWidth"].as_f64().unwrap_or(1.0).max(1.0);
            json!({ "format": "png", "clip": { "x": v["pageX"].as_f64().unwrap_or(0.0), "y": v["pageY"].as_f64().unwrap_or(0.0),
                "width": vw, "height": v["clientHeight"].as_f64().unwrap_or(1.0), "scale": (width / vw).clamp(0.1, 4.0) } })
        });
    }

    /// Moves, presses and lets go through DevTools, which the page gets as the user's own input (isTrusted).
    pub fn real_click(wv: &WebView, x: f64, y: f64, right: bool, clicks: isize, done: Box<dyn FnOnce(Result<(), String>)>) {
        let button = if right { "right" } else { "left" };
        let mut steps = vec![json!({ "type": "mouseMoved", "x": x, "y": y })];
        for n in 1..=clicks.max(1) {
            for kind in ["mousePressed", "mouseReleased"] {
                steps.push(json!({ "type": kind, "x": x, "y": y, "button": button, "buttons": if kind == "mousePressed" { if right { 2 } else { 1 } } else { 0 }, "clickCount": n }));
            }
        }
        fn send(wv: ICoreWebView2, mut steps: std::collections::VecDeque<Value>, done: Box<dyn FnOnce(Result<(), String>)>) {
            let Some(step) = steps.pop_front() else { return done(Ok(())) };
            let w = wv.clone();
            cdp(&wv, "Input.dispatchMouseEvent", step, move |r| match r {
                Ok(_) => send(w, steps, done),
                Err(e) => done(Err(format!("the click couldn't be sent: {e}"))),
            });
        }
        send(wv.webview(), steps.into(), done);
    }

    /// Drawn at up to 2 pixels a CSS pixel, fewer for very long pages, to stay near 30 million pixels (like macOS).
    pub fn snapshot_full(wv: &WebView, done: Done) {
        capture(wv, done, |m| {
            let c = &m["cssContentSize"];
            let (w, h) = (c["width"].as_f64().unwrap_or(1.0).max(1.0), c["height"].as_f64().unwrap_or(1.0).max(1.0));
            // contentSize is in device pixels, so this is the screen's scale; the capture multiplies by it.
            let dpr = (m["contentSize"]["width"].as_f64().unwrap_or(w) / w).max(1.0);
            let scale = (30_000_000.0 / (w * h)).sqrt().clamp(0.25, 2.0) / dpr;
            json!({ "format": "png", "captureBeyondViewport": true,
                "clip": { "x": 0, "y": 0, "width": w, "height": h, "scale": scale } })
        });
    }
}

/// Screenshots on Linux, through WebKitGTK's snapshot.
#[cfg(target_os = "linux")]
mod gtk_shot {
    use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
    use wry::{WebView, WebViewExtUnix};

    /// PNG of `region`, scaled down to `width` pixels wide when it's wider.
    pub fn snapshot(wv: &WebView, region: SnapshotRegion, width: Option<f64>, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
        wv.webview().snapshot(region, SnapshotOptions::NONE, None::<&webkit2gtk::gio::Cancellable>, move |r| {
            done(r.map_err(|e| format!("the page could not be captured: {e}")).and_then(|s| png(s, width)))
        });
    }

    fn png(s: cairo::Surface, width: Option<f64>) -> Result<Vec<u8>, String> {
        let img = cairo::ImageSurface::try_from(s).map_err(|_| "the picture came back in an unexpected format".to_string())?;
        let mut out = Vec::new();
        match width {
            Some(want) if want >= 1.0 && (want as i32) < img.width() => {
                let k = want / img.width() as f64;
                let h = (img.height() as f64 * k).round().max(1.0) as i32;
                let small = cairo::ImageSurface::create(cairo::Format::ARgb32, want.round() as i32, h).map_err(|e| e.to_string())?;
                let cr = cairo::Context::new(&small).map_err(|e| e.to_string())?;
                cr.scale(k, k);
                cr.set_source_surface(&img, 0.0, 0.0).map_err(|e| e.to_string())?;
                cr.paint().map_err(|e| e.to_string())?;
                drop(cr);
                small.write_to_png(&mut out)
            }
            _ => img.write_to_png(&mut out),
        }
        .map_err(|e| format!("PNG encoding failed: {e}"))?;
        Ok(out)
    }
}

/// PNG of the visible page, `width` points wide. Calls `done` later on the main thread.
pub fn snapshot(wv: &WebView, width: f64, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
    #[cfg(target_os = "macos")]
    mac::snapshot(wv, width, done);
    #[cfg(target_os = "windows")]
    win::snapshot(wv, width, Box::new(done));
    #[cfg(target_os = "linux")]
    gtk_shot::snapshot(wv, webkit2gtk::SnapshotRegion::Visible, Some(width), done);
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (wv, width);
        done(Err("screenshots aren't supported on this system".into()));
    }
}

/// PNG of the whole page, top to bottom. Calls `done` later on the main thread.
pub fn snapshot_full(wv: &WebView, done: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
    #[cfg(target_os = "macos")]
    mac::snapshot_full(wv, done);
    #[cfg(target_os = "windows")]
    win::snapshot_full(wv, Box::new(done));
    #[cfg(target_os = "linux")]
    gtk_shot::snapshot(wv, webkit2gtk::SnapshotRegion::FullDocument, None, done);
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = wv;
        done(Err("full-page screenshots aren't supported on this system".into()));
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

/// Reload the page from the server, skipping the cache (a hard reload).
pub fn hard_reload(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::hard_reload(wv);
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::WebViewExt;
        use wry::WebViewExtUnix;
        wv.webview().reload_bypass_cache();
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
        use windows::core::w;
        use wry::WebViewExtWindows;
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        let sent = wv.controller().CoreWebView2().and_then(|p| p.CallDevToolsProtocolMethod(w!("Page.reload"), w!("{\"ignoreCache\":true}"), &handler));
        if let Err(e) = sent {
            crate::log::line("reload", format!("hard reload failed, reloading normally: {e}"));
            let _ = wv.reload();
        }
    }
}

/// Clear every site's cache from the webview's profile, and with `cookies` its cookies and storage as well (signs
/// you out everywhere). `done` gets whether it worked.
pub fn clear_browsing_data(wv: &WebView, cookies: bool, done: impl FnOnce(bool) + Send + 'static) {
    #[cfg(target_os = "macos")]
    mac::clear_browsing_data(wv, cookies, done);
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::{WebViewExt, WebsiteDataManagerExtManual, WebsiteDataTypes};
        use wry::WebViewExtUnix;
        let types = if cookies {
            WebsiteDataTypes::ALL
        } else {
            WebsiteDataTypes::DISK_CACHE | WebsiteDataTypes::MEMORY_CACHE | WebsiteDataTypes::OFFLINE_APPLICATION_CACHE
        };
        match wv.webview().website_data_manager() {
            Some(m) => m.clear(types, webkit2gtk::glib::TimeSpan(0), None::<&webkit2gtk::gio::Cancellable>, move |r| done(r.is_ok())),
            None => done(false),
        }
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use webview2_com::ClearBrowsingDataCompletedHandler;
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Profile2, ICoreWebView2_13, COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE, COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE,
        };
        use windows::core::Interface;
        use wry::WebViewExtWindows;
        let done = std::sync::Arc::new(std::sync::Mutex::new(Some(done)));
        let later = done.clone();
        let handler = ClearBrowsingDataCompletedHandler::create(Box::new(move |hr| {
            if let Some(d) = later.lock().ok().and_then(|mut d| d.take()) {
                d(hr.is_ok());
            }
            Ok(())
        }));
        let profile = wv.controller().CoreWebView2().and_then(|p| p.cast::<ICoreWebView2_13>()).and_then(|p| p.Profile()).and_then(|p| p.cast::<ICoreWebView2Profile2>());
        let sent = profile.and_then(|p| {
            if cookies {
                p.ClearBrowsingDataAll(&handler)
            } else {
                p.ClearBrowsingData(COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE | COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE, &handler)
            }
        });
        if let Err(e) = sent {
            crate::log::line("privacy", format!("couldn't clear browsing data: {e}"));
            if let Some(d) = done.lock().ok().and_then(|mut d| d.take()) {
                d(false);
            }
        }
    }
}

/// Make Valerie the app that opens Markdown files. Needs the bundled Valerie.app, which says it can open them.
pub fn set_default_markdown_app() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};
        #[link(name = "CoreServices", kind = "framework")]
        extern "C" {
            fn LSSetDefaultRoleHandlerForContentType(content_type: *const AnyObject, role: u32, bundle_id: *const AnyObject) -> i32;
        }
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let id: *mut AnyObject = msg_send![bundle, bundleIdentifier];
        if id.is_null() {
            return Err("run Valerie.app (scripts/bundle-macos.sh) to open Markdown files with it".into());
        }
        let kind = objc2_foundation::NSString::from_str("net.daringfireball.markdown");
        // kLSRolesAll: viewing and editing.
        match LSSetDefaultRoleHandlerForContentType(&*kind as *const _ as *const AnyObject, 0xFFFF_FFFF, id) {
            0 => Ok(()),
            status => Err(format!("macOS refused ({status})")),
        }
    }
    #[cfg(not(target_os = "macos"))]
    Err("choose Valerie for .md files in your system settings".into())
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
        let Ok(img) = image::load_from_memory_with_format(png, image::ImageFormat::Png) else { return false };
        let img = img.to_rgba8();
        let data = arboard::ImageData { width: img.width() as usize, height: img.height() as usize, bytes: img.into_raw().into() };
        match arboard::Clipboard::new().and_then(|mut c| c.set_image(data)) {
            Ok(()) => true,
            Err(e) => {
                crate::log::line("capture", format!("copying the picture failed: {e}"));
                false
            }
        }
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
#[cfg(target_os = "macos")]
pub use mac::popup_window;

/// A test copy of Valerie keeps out of the way (`scripts/test-copy.sh`): it never takes focus, and its windows open on
/// a display other than the main one, if there is one.
pub fn test_windows() -> Option<String> {
    std::env::var("VALERIE_TEST_WINDOWS").ok().filter(|v| !v.is_empty())
}


pub fn install_page_dialogs(wv: &WebView) {
    #[cfg(target_os = "macos")]
    {
        mac::install_page_dialogs(wv);
        mac::cursor_from_top_view_only(wv);
        mac::install_load_failures(wv);
    }
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

/// Whether a page's full screen is Valerie's quick kind (setting pages.quick_full_screen).
/// `pages.error_page` and `pages.http_sign_in`, for the native code that has no way to the settings.
pub static ERROR_PAGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
pub static HTTP_SIGN_IN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// "example.com" from "https://example.com:8080/a?b" (never the path or query: they can hold tokens).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn host_of(address: &str) -> String {
    let rest = address.split_once("://").map_or(address, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    host.split(':').next().unwrap_or("").trim_start_matches("www.").to_string()
}

/// The page shown in place of a site that didn't load. `code` is the system's (NSURLError…), `why` its own words.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn error_page_html(address: &str, code: isize, why: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let host = esc(&host_of(address));
    let (title, body) = match code {
        -1009 => ("You're offline".to_string(), "Connect to the internet and this page loads by itself."),
        -1003 | -1006 => (format!("Can't find {host}"), "Check the address for a typo. If it's right, the site may be down, or your connection may be blocking it."),
        -1004 => (format!("{host} refused to connect"), "The site may be down, or nothing is answering at this address."),
        -1001 => (format!("{host} took too long to answer"), "The site may be busy, or your connection slow."),
        -1005 => ("The connection was lost".to_string(), "It dropped while the page was loading."),
        -1007 => (format!("{host} redirected too many times"), "Clearing this site's cookies often fixes it."),
        -1206..=-1200 => (format!("The connection to {host} isn't private"), "The site's security certificate couldn't be checked, so someone could be reading what you send. It wasn't opened."),
        _ => ("This page didn't load".to_string(), ""),
    };
    // Only when the secure port is shut: a site that still answers on plain http.
    let plain = match address.strip_prefix("https://") {
        Some(rest) if code == -1004 => format!(r#"<a href="http://{}">Try without https</a>"#, esc(rest)),
        _ => String::new(),
    };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta name="color-scheme" content="light dark"><title>{host}</title><style>
html{{height:100%}}body{{margin:0;min-height:100%;display:grid;place-items:center;font:15px/1.5 -apple-system,system-ui,sans-serif;background:Canvas;color:CanvasText;-webkit-font-smoothing:antialiased}}
main{{max-width:420px;padding:32px;margin-top:-8vh}}
h1{{font-size:22px;line-height:1.25;font-weight:600;letter-spacing:-.01em;margin:0 0 8px;text-wrap:balance;overflow-wrap:anywhere}}
p{{margin:0 0 22px;opacity:.7;text-wrap:pretty}}
button{{font:inherit;font-weight:500;padding:7px 16px;border-radius:8px;border:0;background:CanvasText;color:Canvas;cursor:pointer;transition:transform .12s ease-out}}
button:active{{transform:scale(.97)}}button:focus-visible,a:focus-visible{{outline:2px solid Highlight;outline-offset:2px}}
a{{color:inherit;opacity:.7;margin-left:16px}}
small{{display:block;margin-top:26px;font:12px/1.4 ui-monospace,monospace;opacity:.45;overflow-wrap:anywhere}}
@media (prefers-reduced-motion:reduce){{button{{transition:none}}}}
</style></head><body><main><h1>{title}</h1><p>{body}</p><button autofocus onclick="location.reload()">Try Again</button>{plain}<small>{why} ({code})</small></main>
<script>addEventListener("online",()=>location.reload())</script></body></html>"#,
        why = esc(why),
    )
}

pub static QUICK_FULLSCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[cfg(not(target_os = "macos"))]
static FULLSCREEN_CHANGED: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

#[cfg(not(target_os = "macos"))]
fn fullscreen_changed() {
    if let Some(f) = FULLSCREEN_CHANGED.get() {
        f();
    }
}

#[cfg(target_os = "linux")]
thread_local!(static FULL_SCREEN_PAGES: std::cell::RefCell<Vec<webkit2gtk::WebView>> = const { std::cell::RefCell::new(Vec::new()) });

/// `f` runs when Valerie has to lay out again for a page's full screen: on the Mac when the page is back in Valerie's
/// window, elsewhere when a page starts or stops filling the window.
#[allow(unused_variables)]
pub fn on_page_fullscreen(wv: &WebView, f: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    mac::on_page_fullscreen(wv, Box::new(f));
    #[cfg(not(target_os = "macos"))]
    let _ = FULLSCREEN_CHANGED.set(Box::new(f));
}

/// The engine's own word, not the page's: a page can't claim full screen without a real request.
#[allow(unused_variables)]
pub fn watch_page_fullscreen(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::watch_page_fullscreen(wv);
    #[cfg(target_os = "windows")]
    {
        use webview2_com::ContainsFullScreenElementChangedEventHandler;
        use wry::WebViewExtWindows;
        let handler = ContainsFullScreenElementChangedEventHandler::create(Box::new(|_, _| {
            fullscreen_changed();
            Ok(())
        }));
        let mut token = 0i64;
        if let Err(e) = unsafe { wv.webview().add_ContainsFullScreenElementChanged(&handler, &mut token) } {
            crate::log::line("fullscreen", format!("can't watch full screen: {e}"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::WebViewExt;
        use wry::WebViewExtUnix;
        let view = wv.webview();
        // false: WebKitGTK goes on to make the window full screen itself, and the page only enters if it does.
        view.connect_enter_fullscreen(|v| {
            FULL_SCREEN_PAGES.with(|p| p.borrow_mut().push(v.clone()));
            fullscreen_changed();
            false
        });
        view.connect_leave_fullscreen(|v| {
            FULL_SCREEN_PAGES.with(|p| p.borrow_mut().retain(|x| x != v));
            fullscreen_changed();
            false
        });
    }
}

/// A page in full screen: on the Mac in WebKit's window, where Valerie mustn't move or hide it (whatever the setting);
/// elsewhere still in its view, which Valerie stretches over the window when quick full screen is on.
#[allow(unused_variables)]
pub fn in_page_fullscreen(wv: &WebView) -> bool {
    #[cfg(target_os = "macos")]
    return mac::in_page_fullscreen(wv);
    #[cfg(not(target_os = "macos"))]
    if !QUICK_FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        use wry::WebViewExtWindows;
        let mut full = windows::core::BOOL(0);
        return unsafe { wv.webview().ContainsFullScreenElement(&mut full) }.is_ok() && full.as_bool();
    }
    #[cfg(target_os = "linux")]
    {
        use wry::WebViewExtUnix;
        let view = wv.webview();
        return FULL_SCREEN_PAGES.with(|p| p.borrow().contains(&view));
    }
}

/// `f` gets the webview whose floating video's return button was pressed (see `is_webview`).
#[allow(unused_variables)]
pub fn on_video_return(wv: &WebView, f: impl Fn(usize) + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    mac::on_video_return(wv, Box::new(f));
}

#[allow(unused_variables)]
pub fn is_webview(wv: &WebView, id: usize) -> bool {
    #[cfg(target_os = "macos")]
    return mac::is_webview(wv, id);
    #[cfg(not(target_os = "macos"))]
    false
}

/// Get ready to send notifications; `on_click` receives a clicked notification's action.
pub fn notifications_init(on_click: impl Fn(String) + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    mac::notifications_init(Box::new(on_click));
    #[cfg(not(target_os = "macos"))]
    let _ = on_click;
}

/// Show a system notification: macOS's own notification system when Valerie runs as an app, otherwise the best the
/// platform offers (a script on a bare macOS build, notify-send on Linux, a PowerShell script on Windows).
pub fn notify(n: &Note) {
    #[cfg(target_os = "macos")]
    if mac::notify_native(n) || mac::notify_legacy(n) {
        return;
    }
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    #[cfg(target_os = "macos")]
    let _ = crate::platform::command("osascript")
        .arg("-e")
        .arg(format!("display notification \"{}\" with title \"{}\" subtitle \"{}\"", esc(n.body), esc(n.title), esc(n.subtitle)))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = crate::platform::command("notify-send").args([n.title, &format!("{}\n{}", n.subtitle, n.body)]).spawn();
    // Through the system's notification service; the words go in as text, so a site's can't become script or markup.
    #[cfg(windows)]
    {
        let _ = esc;
        const SCRIPT: &str = r#"$k='HKCU:\Software\Classes\AppUserModelId\dev.valerie.browser'
if(!(Test-Path $k)){New-Item $k -Force|Out-Null;Set-ItemProperty $k DisplayName Valerie}
[void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime]
[void][Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom,ContentType=WindowsRuntime]
$x=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent(1)
$t=$x.GetElementsByTagName('text')
[void]$t.Item(0).AppendChild($x.CreateTextNode($env:VALERIE_NOTE_TITLE))
[void]$t.Item(1).AppendChild($x.CreateTextNode($env:VALERIE_NOTE_BODY))
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('dev.valerie.browser').Show([Windows.UI.Notifications.ToastNotification]::new($x))"#;
        let body = if n.subtitle.is_empty() { n.body.to_string() } else { format!("{}\n{}", n.subtitle, n.body) };
        let shown = crate::platform::command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .env("VALERIE_NOTE_TITLE", n.title)
            .env("VALERIE_NOTE_BODY", body)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(e) = shown {
            crate::log::line("notify", format!("couldn't show a notification: {e}"));
        }
    }
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

/// Pick a folder in the system's folder window. None when cancelled, and elsewhere than macOS (ask for a path instead).
pub fn pick_folder(message: &str) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    return mac::pick_folder(message);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = message;
        None
    }
}

/// The system's share menu for a page (macOS only; `share_page` is hidden elsewhere).
pub fn share(wv: &WebView, url: &str) {
    #[cfg(target_os = "macos")]
    mac::share(wv, url);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, url);
}

/// Pick files to open in the system's file window; empty when cancelled.
pub fn pick_files(message: &str) -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    return mac::pick_files(message);
    #[cfg(target_os = "windows")]
    return windows_pick_files(message);
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        let d = gtk::FileChooserDialog::with_buttons(
            Some(message),
            None::<&gtk::Window>,
            gtk::FileChooserAction::Open,
            &[("_Cancel", gtk::ResponseType::Cancel), ("_Open", gtk::ResponseType::Accept)],
        );
        d.set_select_multiple(true);
        let files = if d.run() == gtk::ResponseType::Accept { d.filenames() } else { Vec::new() };
        d.close();
        files
    }
}

/// The Windows file window (IFileOpenDialog), several files allowed.
#[cfg(target_os = "windows")]
fn windows_pick_files(message: &str) -> Vec<std::path::PathBuf> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, SIGDN_FILESYSPATH};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(d) = CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) else { return Vec::new() };
        if let Ok(o) = d.GetOptions() {
            let _ = d.SetOptions(o | FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST);
        }
        let _ = d.SetTitle(&HSTRING::from(message));
        if d.Show(None).is_err() {
            return Vec::new();
        }
        let Ok(items) = d.GetResults() else { return Vec::new() };
        let n = items.GetCount().unwrap_or(0);
        (0..n)
            .filter_map(|i| {
                let p = items.GetItemAt(i).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                let path = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as _));
                path.map(std::path::PathBuf::from)
            })
            .collect()
    }
}

/// Where the computer is, for a site's navigator.geolocation; other systems' web views have their own, so only the Mac uses this.
pub fn locate(done: impl FnOnce(serde_json::Value) + 'static) {
    #[cfg(target_os = "macos")]
    mac::locate(Box::new(done));
    #[cfg(not(target_os = "macos"))]
    done(serde_json::json!({ "error": 2, "message": "Location isn't available" }));
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

#[cfg(test)]
mod error_page_tests {
    use super::{error_page_html, host_of};

    #[test]
    fn host_never_includes_path_or_login() {
        assert_eq!(host_of("https://user:pw@www.example.com:8443/a?token=1"), "example.com");
        assert_eq!(host_of("http://localhost:3000"), "localhost");
    }

    #[test]
    fn error_page_escapes_and_offers_http_only_when_refused() {
        let refused = error_page_html("https://exa<b>mple.com/x?a=\"1\"", -1004, "Could not <connect>");
        assert!(refused.contains("exa&lt;b&gt;mple.com refused to connect") && !refused.contains("<b>mple"));
        assert!(refused.contains(r#"href="http://exa&lt;b&gt;mple.com/x?a=&quot;1&quot;""#) && refused.contains("Could not &lt;connect&gt; (-1004)"));
        let cert = error_page_html("https://example.com/", -1202, "untrusted");
        assert!(cert.contains("isn't private") && !cert.contains("Try without https"));
        assert!(error_page_html("https://example.com/", -1009, "offline").contains("You're offline"));
    }
}
