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
        eprintln!("valerie: could not open {target}: {e}");
    }
}

/// Open a text file in the user's editor.
pub fn open_text_file(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-t").arg(path).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    open_external(&path.to_string_lossy());
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
