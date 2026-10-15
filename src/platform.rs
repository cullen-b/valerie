//! Small OS-specific touches that wry/tao don't expose.
use wry::WebView;

/// Stable per-profile id; on macOS it names an isolated WKWebsiteDataStore.
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

    pub fn round_corners(wv: &WebView, radius: f64) {
        let v = ptr(wv);
        unsafe {
            let _: () = msg_send![v, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![v, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setCornerRadius: radius];
                let _: () = msg_send![layer, setMasksToBounds: radius > 0.0];
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
}

pub fn bring_to_front(wv: &WebView) {
    #[cfg(target_os = "macos")]
    mac::bring_to_front(wv);
    #[cfg(not(target_os = "macos"))]
    let _ = wv;
}

pub fn round_corners(wv: &WebView, radius: f64) {
    #[cfg(target_os = "macos")]
    mac::round_corners(wv, radius);
    #[cfg(not(target_os = "macos"))]
    let _ = (wv, radius);
}

pub fn set_traffic_lights_hidden(window: &tao::window::Window, hidden: bool) {
    #[cfg(target_os = "macos")]
    mac::set_traffic_lights_hidden(window, hidden);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, hidden);
}
