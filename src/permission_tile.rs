//! Full Disk Access in the welcome guide. macOS has no dialog that asks for it, so a small window beside System Settings
//! holds Valerie's icon for the user to drag into the list. Only the Mac has any of this.
use std::path::Path;

/// Can Valerie read what Full Disk Access protects? `None` where that isn't a thing, or can't be told.
pub fn full_disk_access() -> Option<bool> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let home = dirs::home_dir()?;
    // Every Mac has the first; reading it is simply refused without the permission, with no prompt.
    for probe in ["Library/Application Support/com.apple.TCC/TCC.db", "Library/Safari/Bookmarks.plist"] {
        match std::fs::File::open(home.join(probe)) {
            Ok(_) => return Some(true),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Some(false),
            Err(_) => {}
        }
    }
    None
}

#[cfg(target_os = "macos")]
pub use mac::{close, follow, show};

#[cfg(not(target_os = "macos"))]
pub fn show(_app: &Path) -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub fn follow() {}
#[cfg(not(target_os = "macos"))]
pub fn close() {}

#[cfg(target_os = "macos")]
mod mac {
    use super::Path;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
    use objc2::{class, define_class, msg_send, AnyThread, Encode, Encoding, MainThreadMarker, MainThreadOnly, RefEncode};
    use objc2_app_kit::{NSBackingStoreType, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent, NSPanel, NSRunningApplication, NSScreen, NSView, NSWindowCollectionBehavior, NSWindowStyleMask};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
    use std::cell::RefCell;

    const WIDTH: f64 = 300.0;
    const HEIGHT: f64 = 80.0;
    const GAP: f64 = 14.0;
    const ICON: NSRect = NSRect { origin: NSPoint { x: 14.0, y: 12.0 }, size: NSSize { width: 56.0, height: 56.0 } };

    #[repr(C)]
    struct CGColor {
        _private: [u8; 0],
    }
    unsafe impl RefEncode for CGColor {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[]));
    }
    const _: fn() = || {
        fn is_encode<T: Encode>() {}
        is_encode::<*const CGColor>();
    };

    thread_local! {
        static PANEL: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
        static APP: RefCell<String> = const { RefCell::new(String::new()) };
    }

    fn app_icon(path: &str) -> Retained<AnyObject> {
        unsafe {
            let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            msg_send![&*workspace, iconForFile: &*NSString::from_str(path)]
        }
    }

    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "ValerieDragTile"]
        struct DragTile;

        impl DragTile {
            #[unsafe(method(acceptsFirstMouse:))]
            fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
                true
            }

            #[unsafe(method(mouseDownCanMoveWindow))]
            fn mouse_down_can_move_window(&self) -> bool {
                false
            }

            // The whole tile drags, the icon and words included.
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, _point: NSPoint) -> *const NSView {
                (self as *const Self).cast()
            }

            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, _event: &NSEvent) {}

            #[unsafe(method(mouseDragged:))]
            fn mouse_dragged(&self, event: &NSEvent) {
                let path = APP.with_borrow(|a| a.clone());
                if path.is_empty() {
                    return;
                }
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
                let writer = ProtocolObject::from_ref(&*url);
                let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), writer);
                let icon = app_icon(&path);
                unsafe { item.setDraggingFrame_contents(ICON, Some(&*icon)) };
                let items = NSArray::from_retained_slice(&[item]);
                self.beginDraggingSessionWithItems_event_source(&items, event, ProtocolObject::from_ref(self));
            }
        }

        unsafe impl NSObjectProtocol for DragTile {}

        unsafe impl NSDraggingSource for DragTile {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn source_operation(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
                NSDragOperation::Copy
            }
        }
    );

    unsafe fn label(text: &str, x: f64, y: f64, size: f64, bold: bool, alpha: f64) -> Retained<AnyObject> {
        let field: Retained<AnyObject> = msg_send![class!(NSTextField), labelWithString: &*NSString::from_str(text)];
        let font: Retained<AnyObject> = if bold { msg_send![class!(NSFont), boldSystemFontOfSize: size] } else { msg_send![class!(NSFont), systemFontOfSize: size] };
        let color: Retained<AnyObject> = msg_send![class!(NSColor), colorWithWhite: 1.0f64, alpha: alpha];
        let _: () = msg_send![&*field, setFont: &*font];
        let _: () = msg_send![&*field, setTextColor: &*color];
        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(WIDTH - x - 12.0, size + 6.0));
        let _: () = msg_send![&*field, setFrame: frame];
        field
    }

    /// Build the window (out of sight until System Settings shows up). False if it's not a real Valerie.app.
    pub fn show(app: &Path) -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        APP.set(app.to_string_lossy().into_owned());
        if PANEL.with_borrow(|p| p.is_some()) {
            return true;
        }
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT));
        unsafe {
            let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
            let panel: Retained<NSPanel> = msg_send![NSPanel::alloc(mtm), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false];
            panel.setFloatingPanel(true);
            panel.setBecomesKeyOnlyIfNeeded(true);
            panel.setHidesOnDeactivate(false);
            panel.setReleasedWhenClosed(false);
            panel.setOpaque(false);
            panel.setHasShadow(true);
            panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary);
            let clear: Retained<AnyObject> = msg_send![class!(NSColor), clearColor];
            let _: () = msg_send![&*panel, setBackgroundColor: &*clear];

            let tile: Retained<DragTile> = msg_send![DragTile::alloc(mtm), initWithFrame: frame];
            tile.setWantsLayer(true);
            let layer: *mut AnyObject = msg_send![&*tile, layer];
            let color: Retained<AnyObject> = msg_send![class!(NSColor), colorWithSRGBRed: 0.086f64, green: 0.08f64, blue: 0.11f64, alpha: 0.98f64];
            let cg: *const CGColor = msg_send![&*color, CGColor];
            let _: () = msg_send![layer, setBackgroundColor: cg];
            let _: () = msg_send![layer, setCornerRadius: 14.0f64];

            let icon: Retained<AnyObject> = msg_send![class!(NSImageView), imageViewWithImage: &*app_icon(&APP.with_borrow(|a| a.clone()))];
            let _: () = msg_send![&*icon, setFrame: ICON];
            let _: () = msg_send![&*tile, addSubview: &*icon];
            let title = label("Drag Valerie into the list", 80.0, 42.0, 14.0, true, 0.96);
            let hint = label("Then turn on the switch beside it", 80.0, 21.0, 12.0, false, 0.66);
            let _: () = msg_send![&*tile, addSubview: &*title];
            let _: () = msg_send![&*tile, addSubview: &*hint];

            panel.setContentView(Some(&tile));
            PANEL.set(Some(panel));
        }
        follow();
        true
    }

    /// System Settings' front window, in screen points with the origin at the bottom left.
    fn settings_window() -> Option<NSRect> {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut NSArray<AnyObject>;
        }
        const ON_SCREEN_ONLY: u32 = 1;
        const EXCLUDE_DESKTOP: u32 = 16;
        let mtm = MainThreadMarker::new()?;
        let id = NSString::from_str("com.apple.systempreferences");
        let pid = NSRunningApplication::runningApplicationsWithBundleIdentifier(&id).firstObject()?.processIdentifier();
        let number = |dict: &AnyObject, key: &str| -> Option<f64> {
            unsafe {
                let v: Option<Retained<AnyObject>> = msg_send![dict, objectForKey: &*NSString::from_str(key)];
                let v = v?;
                Some(msg_send![&*v, doubleValue])
            }
        };
        unsafe {
            let list = Retained::from_raw(CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0))?;
            for info in list.iter() {
                if number(&info, "kCGWindowOwnerPID")? as i32 != pid || number(&info, "kCGWindowLayer")? != 0.0 {
                    continue;
                }
                let bounds: Option<Retained<AnyObject>> = msg_send![&*info, objectForKey: &*NSString::from_str("kCGWindowBounds")];
                let bounds = bounds?;
                let (x, y, w, h) = (number(&bounds, "X")?, number(&bounds, "Y")?, number(&bounds, "Width")?, number(&bounds, "Height")?);
                if w < 200.0 {
                    continue;
                }
                let top = NSScreen::screens(mtm).firstObject()?.frame().size.height;
                return Some(NSRect::new(NSPoint::new(x, top - y - h), NSSize::new(w, h)));
            }
        }
        None
    }

    /// Beside the window if there's room, else under it, so the two never cover each other.
    fn spot(window: NSRect, screen: NSRect) -> NSPoint {
        let right = window.origin.x + window.size.width + GAP;
        let left = window.origin.x - GAP - WIDTH;
        let middle = window.origin.y + (window.size.height - HEIGHT) / 2.0;
        let clamped = middle.clamp(screen.origin.y, (screen.origin.y + screen.size.height - HEIGHT).max(screen.origin.y));
        if right + WIDTH <= screen.origin.x + screen.size.width {
            NSPoint::new(right, clamped)
        } else if left >= screen.origin.x {
            NSPoint::new(left, clamped)
        } else {
            let x = window.origin.x + (window.size.width - WIDTH) / 2.0;
            NSPoint::new(x, (window.origin.y - GAP - HEIGHT).max(screen.origin.y))
        }
    }

    /// Stay beside System Settings, and away while it's closed.
    pub fn follow() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let window = settings_window();
        PANEL.with_borrow(|panel| {
            let Some(panel) = panel else { return };
            let Some(window) = window else { return panel.orderOut(None) };
            let center = NSPoint::new(window.origin.x + window.size.width / 2.0, window.origin.y + window.size.height / 2.0);
            let screens = NSScreen::screens(mtm);
            let screen = screens.iter().find(|s| {
                let f = s.frame();
                center.x >= f.origin.x && center.x < f.origin.x + f.size.width && center.y >= f.origin.y && center.y < f.origin.y + f.size.height
            });
            let Some(screen) = screen.or_else(|| screens.firstObject()) else { return };
            panel.setFrameOrigin(spot(window, screen.visibleFrame()));
            if !panel.isVisible() {
                panel.orderFrontRegardless();
            }
        });
    }

    pub fn close() {
        if let Some(panel) = PANEL.take() {
            panel.orderOut(None);
            panel.close();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
            NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
        }

        #[test]
        fn sits_beside_the_window_and_never_over_it() {
            let screen = rect(0.0, 0.0, 1600.0, 900.0);
            assert_eq!(spot(rect(100.0, 100.0, 700.0, 600.0), screen).x, 814.0);
            assert_eq!(spot(rect(1000.0, 100.0, 590.0, 600.0), screen).x, 686.0);
            let under = spot(rect(200.0, 300.0, 1200.0, 500.0), screen);
            assert_eq!((under.x, under.y), (650.0, 206.0));
        }
    }
}
