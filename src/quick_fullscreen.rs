//! Quick full screen on the Mac: WebKit's own element full screen, without the system's slide to a separate Space.
//! The page can't tell (`:fullscreen`, `document.fullscreenElement` and its events are all real); only the window differs.
//! WebKit's window calls `enterFullScreenMode:` and then waits for delegate callbacks, so that's all this replaces.
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{class, sel};
use objc2_foundation::NSRect;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

const ENTER_SECS: f64 = 0.25;
const EXIT_SECS: f64 = 0.2;

/// What a quick full screen changed, to put back when it ends. Pointers, kept as numbers to live in a static.
struct Active {
    window: usize,
    backdrop: usize,
    style: usize,
    options: usize,
}
static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

/// Private names: if WebKit renames them, nothing here runs and full screen stays the system's.
pub fn install() {
    let Some(cls) = AnyClass::get(c"WebCoreFullScreenWindow") else {
        crate::log::line("fullscreen", "WebKit's full screen window class is missing: quick full screen is off");
        return;
    };
    let window = class!(NSWindow);
    if window.instance_method(sel!(enterFullScreenMode:)).is_none() || window.instance_method(sel!(exitFullScreenMode:)).is_none() {
        crate::log::line("fullscreen", "NSWindow's full screen methods are missing: quick full screen is off");
        return;
    }
    let cls = cls as *const AnyClass as *mut AnyClass;
    unsafe {
        let add = |name: Sel, imp: Imp| objc2::ffi::class_addMethod(cls, name, imp, c"v@:@".as_ptr());
        add(sel!(enterFullScreenMode:), std::mem::transmute::<*const (), Imp>(enter as *const ()));
        add(sel!(exitFullScreenMode:), std::mem::transmute::<*const (), Imp>(exit as *const ()));
        add(sel!(toggleFullScreen:), std::mem::transmute::<*const (), Imp>(toggle as *const ()));
    }
}

unsafe fn call_nswindow(this: *mut AnyObject, sel: Sel, sender: *mut AnyObject) {
    if let Some(method) = class!(NSWindow).instance_method(sel) {
        let inherited: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = std::mem::transmute(method.implementation());
        inherited(this, sel, sender);
    }
}

fn active() -> std::sync::MutexGuard<'static, Option<Active>> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "C-unwind" fn enter(this: *mut AnyObject, sel: Sel, sender: *mut AnyObject) {
    let delegate: *mut AnyObject = msg_send![this, delegate];
    let screen: *mut AnyObject = msg_send![this, screen];
    let can_animate: bool = !delegate.is_null() && msg_send![delegate, respondsToSelector: sel!(window:startCustomAnimationToEnterFullScreenWithDuration:)];
    if !crate::platform::QUICK_FULLSCREEN.load(Ordering::Relaxed) || !can_animate || screen.is_null() {
        return call_nswindow(this, sel, sender);
    }
    finish();
    // The whole screen goes black behind the page, including the strip beside a notch that WebKit's window leaves out.
    let frame: NSRect = msg_send![screen, frame];
    let backdrop: *mut AnyObject = msg_send![class!(NSWindow), alloc];
    let backdrop: *mut AnyObject = msg_send![backdrop, initWithContentRect: frame, styleMask: 0usize, backing: objc2_app_kit::NSBackingStoreType::Buffered, defer: false];
    let _: () = msg_send![backdrop, setReleasedWhenClosed: false];
    let black: *mut AnyObject = msg_send![class!(NSColor), blackColor];
    let _: () = msg_send![backdrop, setBackgroundColor: black];
    let _: () = msg_send![backdrop, setIgnoresMouseEvents: true];
    let _: () = msg_send![backdrop, setHasShadow: false];
    let _: () = msg_send![backdrop, setAnimationBehavior: 2isize];
    let _: () = msg_send![backdrop, setAlphaValue: 0.0f64];
    // Not a full screen window of its own (bit 7), and allowed to share a Space that already is one (bit 8).
    let behaviour: usize = msg_send![this, collectionBehavior];
    let behaviour = (behaviour & !(1usize << 7)) | 1 << 8;
    let _: () = msg_send![this, setCollectionBehavior: behaviour];
    let _: () = msg_send![backdrop, setCollectionBehavior: behaviour];
    let number: isize = msg_send![this, windowNumber];
    let _: () = msg_send![backdrop, orderWindow: -1isize, relativeTo: number];
    let style: usize = msg_send![this, styleMask];
    let _: () = msg_send![this, setStyleMask: 0usize];
    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
    let options: usize = msg_send![app, presentationOptions];
    // Auto-hide the dock (1) and menu bar (4): the mouse at the edge still brings them back, as in system full screen.
    let _: () = msg_send![app, setPresentationOptions: options | 1 | 4];
    *active() = Some(Active { window: this as usize, backdrop: backdrop as usize, style, options });
    fade(backdrop, 1.0, ENTER_SECS);
    let _: () = msg_send![delegate, window: this, startCustomAnimationToEnterFullScreenWithDuration: ENTER_SECS];
    let _: () = msg_send![delegate, performSelector: sel!(windowDidEnterFullScreen:), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: ENTER_SECS];
    crate::log::line("fullscreen", "quick full screen");
}

unsafe extern "C-unwind" fn exit(this: *mut AnyObject, sel: Sel, sender: *mut AnyObject) {
    let now = active().as_ref().filter(|a| a.window == this as usize).map(|a| (a.backdrop, a.options));
    let Some((backdrop, options)) = now else {
        return call_nswindow(this, sel, sender);
    };
    let delegate: *mut AnyObject = msg_send![this, delegate];
    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
    let _: () = msg_send![app, setPresentationOptions: options];
    fade(backdrop as *mut AnyObject, 0.0, EXIT_SECS);
    let _: () = msg_send![delegate, window: this, startCustomAnimationToExitFullScreenWithDuration: EXIT_SECS];
    let _: () = msg_send![delegate, performSelector: sel!(windowDidExitFullScreen:), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: EXIT_SECS];
}

/// The system's full screen menu item and shortcut: in a quick full screen, they mean leave it.
unsafe extern "C-unwind" fn toggle(this: *mut AnyObject, sel: Sel, sender: *mut AnyObject) {
    if active().as_ref().is_some_and(|a| a.window == this as usize) {
        let _: () = msg_send![this, cancelOperation: sender];
        return;
    }
    call_nswindow(this, sel, sender);
}

unsafe fn fade(window: *mut AnyObject, alpha: f64, secs: f64) {
    let context = class!(NSAnimationContext);
    let _: () = msg_send![context, beginGrouping];
    let current: *mut AnyObject = msg_send![context, currentContext];
    let _: () = msg_send![current, setDuration: secs];
    let animator: *mut AnyObject = msg_send![window, animator];
    let _: () = msg_send![animator, setAlphaValue: alpha];
    let _: () = msg_send![context, endGrouping];
}

/// Puts everything back. Also runs when a page closes or crashes in full screen, when WebKit never asks to leave it.
pub fn finish() {
    let Some(a) = active().take() else {
        return;
    };
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setPresentationOptions: a.options];
        let backdrop = a.backdrop as *mut AnyObject;
        let _: () = msg_send![backdrop, orderOut: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![backdrop, release];
        let _: () = msg_send![a.window as *mut AnyObject, setStyleMask: a.style];
    }
}
