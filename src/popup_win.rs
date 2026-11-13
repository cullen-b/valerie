//! Sign-in pop-ups on Windows as a floating pane: no title bar, the page inset on a plain pane with rounded corners,
//! and minimize / maximize / close in a strip that slides open while the pointer is at the top edge (`links.popup_look`).

use std::{cell::RefCell, collections::HashMap, num::NonZeroIsize};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, WindowCloseRequestedEventHandler};
use windows::core::{w, HSTRING, PWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

const BEZEL: f64 = 6.0;
const STRIP: f64 = 32.0;
const BUTTON: f64 = 46.0; // Windows' own caption button width
const CLASS: windows::core::PCWSTR = w!("ValeriePopup");

struct Popup {
    webview: wry::WebView,
    open: bool,
    hover: Option<usize>,
    pressed: Option<usize>,
}

thread_local! {
    static POPUPS: RefCell<HashMap<isize, Popup>> = RefCell::new(HashMap::new());
}

struct Host(HWND);
impl HasWindowHandle for Host {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let h = Win32WindowHandle::new(NonZeroIsize::new(self.0 .0 as isize).ok_or(HandleError::Unavailable)?);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(h)) })
    }
}

fn dark() -> bool {
    let mut v: u32 = 1;
    let mut n = 4u32;
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut n))
    };
    r.is_ok() && v == 0
}

fn scale(hwnd: HWND) -> f64 {
    (unsafe { GetDpiForWindow(hwnd) }.max(96) as f64) / 96.0
}

fn client(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut r) };
    r
}

/// The page's spot: inset by the bezel, lower while the strip is open.
fn page_rect(hwnd: HWND, open: bool) -> wry::Rect {
    let (s, r) = (scale(hwnd), client(hwnd));
    let b = (BEZEL * s).round() as i32;
    let top = if open { (STRIP * s).round() as i32 } else { b };
    wry::Rect {
        position: wry::dpi::PhysicalPosition::new(b, top).into(),
        size: wry::dpi::PhysicalSize::new((r.right - 2 * b).max(0) as u32, (r.bottom - top - b).max(0) as u32).into(),
    }
}

/// Minimize, maximize and close, right to left from the strip's right end, in client pixels.
fn button_rect(hwnd: HWND, i: usize) -> RECT {
    let (s, r) = (scale(hwnd), client(hwnd));
    let (w, h) = ((BUTTON * s).round() as i32, (STRIP * s).round() as i32);
    let right = r.right - (2 - i as i32) * w;
    RECT { left: right - w, top: 0, right, bottom: h }
}

fn cursor(hwnd: HWND) -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
        let _ = ScreenToClient(hwnd, &mut p);
    }
    p
}

fn inside(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

fn with<R>(hwnd: HWND, f: impl FnOnce(&mut Popup) -> R) -> Option<R> {
    POPUPS.with(|m| m.borrow_mut().get_mut(&(hwnd.0 as isize)).map(f))
}

fn set_open(hwnd: HWND, open: bool) {
    let changed = with(hwnd, |p| {
        let was = p.open;
        p.open = open;
        if !open {
            p.hover = None;
            p.pressed = None;
        }
        if was != open {
            let _ = p.webview.set_bounds(page_rect(hwnd, open));
        }
        was != open
    });
    if changed == Some(true) {
        repaint(hwnd);
    }
}

/// After every move: open the strip at the top edge, close it once the pointer leaves it, and light the button under it.
fn pointer_moved(hwnd: HWND, nonclient: bool) {
    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | if nonclient { TME_NONCLIENT } else { Default::default() }, hwndTrack: hwnd, dwHoverTime: 0 };
    let _ = unsafe { TrackMouseEvent(&mut t) };
    let p = cursor(hwnd);
    let s = scale(hwnd);
    let r = client(hwnd);
    let open = with(hwnd, |p| p.open).unwrap_or(false);
    let band = if open { STRIP } else { BEZEL };
    let want = p.x >= 0 && p.x < r.right && p.y >= 0 && (p.y as f64) < band * s;
    set_open(hwnd, want);
    let hover = want.then(|| (0..3).find(|&i| inside(&button_rect(hwnd, i), p))).flatten();
    if with(hwnd, |pp| std::mem::replace(&mut pp.hover, hover) != hover) == Some(true) {
        repaint(hwnd);
    }
}

/// Paints now: Valerie's event loop keeps the queue busy, so a queued WM_PAINT for a window it doesn't own may never come.
fn repaint(hwnd: HWND) {
    unsafe {
        let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_UPDATENOW);
    }
}

fn paint(hwnd: HWND) {
    let dark = dark();
    let (bg, fg, lit) = if dark { (0x202020, 0xFFFFFF, 0x2D2D2D) } else { (0xF3F3F3, 0x1A1A1A, 0xE5E5E5) };
    let rgb = |c: u32| COLORREF(((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF));
    let (open, hover, zoomed) = (with(hwnd, |p| p.open).unwrap_or(false), with(hwnd, |p| p.hover).flatten(), unsafe { IsZoomed(hwnd) }.as_bool());
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let r = client(hwnd);
        let brush = CreateSolidBrush(rgb(bg));
        FillRect(dc, &r, brush);
        let _ = DeleteObject(brush.into());
        if open {
            let s = scale(hwnd);
            let font = CreateFontW(-((10.0 * s).round() as i32), 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe MDL2 Assets"));
            let old = SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            // Minimize, maximize (restore while maximized), close.
            let glyphs = [0xE921u16, if zoomed { 0xE923 } else { 0xE922 }, 0xE8BB];
            for (i, g) in glyphs.iter().enumerate() {
                let mut br = button_rect(hwnd, i);
                let mut ink = fg;
                if hover == Some(i) {
                    let fill = if i == 2 { 0xC42B1C } else { lit };
                    if i == 2 {
                        ink = 0xFFFFFF;
                    }
                    let b = CreateSolidBrush(rgb(fill));
                    FillRect(dc, &br, b);
                    let _ = DeleteObject(b.into());
                }
                SetTextColor(dc, rgb(ink));
                let mut text = [*g];
                DrawTextW(dc, &mut text, &mut br, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
            SelectObject(dc, old);
            let _ = DeleteObject(font.into());
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn hit_test(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let mut p = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut p);
    }
    let (s, r) = (scale(hwnd), client(hwnd));
    let edge = (BEZEL * s).round() as i32;
    let open = with(hwnd, |p| p.open).unwrap_or(false);
    let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
    if !zoomed {
        let (l, rt, t, b) = (p.x < edge, p.x >= r.right - edge, p.y < (2.0 * s) as i32, p.y >= r.bottom - edge);
        let code = match (t, b, l, rt) {
            (true, _, true, _) => Some(HTTOPLEFT),
            (true, _, _, true) => Some(HTTOPRIGHT),
            (_, true, true, _) => Some(HTBOTTOMLEFT),
            (_, true, _, true) => Some(HTBOTTOMRIGHT),
            (true, ..) if !open => Some(HTTOP),
            (_, true, ..) => Some(HTBOTTOM),
            (_, _, true, _) => Some(HTLEFT),
            (_, _, _, true) => Some(HTRIGHT),
            _ => None,
        };
        if let Some(c) = code {
            return LRESULT(c as isize);
        }
    }
    if open && (0..3).any(|i| inside(&button_rect(hwnd, i), p)) {
        return LRESULT(HTCLIENT as isize);
    }
    if (p.y as f64) < if open { STRIP } else { BEZEL } * s {
        return LRESULT(HTCAPTION as isize);
    }
    LRESULT(HTCLIENT as isize)
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if !matches!(msg, WM_MOUSEMOVE | WM_NCMOUSEMOVE | WM_PAINT | WM_SETCURSOR | WM_NCHITTEST | WM_ERASEBKGND | 0x2a1 | 0x2a2 | 0x2a3 | WM_GETOBJECT | WM_GETICON | WM_GETMINMAXINFO | WM_TIMER) {
        crate::log::line("popup-debug", format!("msg {msg:#x} w={:#x} l={:#x}", wparam.0, lparam.0));
    }
    match msg {
        // The whole window is client area (no title bar); a maximized window keeps its frame off screen.
        WM_NCCALCSIZE if wparam.0 != 0 => {
            if IsZoomed(hwnd).as_bool() {
                let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                let dpi = GetDpiForWindow(hwnd);
                let f = GetSystemMetricsForDpi(SM_CXFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                let r = &mut params.rgrc[0];
                r.left += f;
                r.top += f;
                r.right -= f;
                r.bottom -= f;
            }
            LRESULT(0)
        }
        WM_NCHITTEST => hit_test(hwnd, lparam),
        WM_MOUSEMOVE => {
            pointer_moved(hwnd, false);
            LRESULT(0)
        }
        WM_NCMOUSEMOVE => {
            pointer_moved(hwnd, true);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSELEAVE | WM_NCMOUSELEAVE => {
            pointer_moved(hwnd, msg == WM_NCMOUSELEAVE);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let p = cursor(hwnd);
            let b = (0..3).find(|&i| inside(&button_rect(hwnd, i), p));
            with(hwnd, |pp| pp.pressed = b);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let p = cursor(hwnd);
            let b = (0..3).find(|&i| inside(&button_rect(hwnd, i), p));
            let pressed = with(hwnd, |pp| pp.pressed.take()).flatten();
            if b.is_some() && b == pressed {
                match b {
                    Some(0) => {
                        let _ = ShowWindow(hwnd, SW_MINIMIZE);
                    }
                    Some(1) => {
                        let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
                    }
                    _ => {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            }
            LRESULT(0)
        }
        WM_SIZE => {
            with(hwnd, |p| {
                let _ = p.webview.set_bounds(page_rect(hwnd, p.open));
            });
            repaint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            let on: i32 = dark().into();
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const i32 as _, 4);
            repaint(hwnd);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            // Dropping the page closes it.
            let gone = POPUPS.with(|m| m.borrow_mut().remove(&(hwnd.0 as isize)));
            drop(gone);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// A site's pop-up at the size it asked for, as a floating pane; `None` lets WebView2 open its own window instead.
pub fn popup_window(opener: &ICoreWebView2, environment: &ICoreWebView2Environment, size: (f64, f64), position: Option<(f64, f64)>) -> Option<ICoreWebView2> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&class);
        });
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), CLASS, w!(""), WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, 400, 400, None, None, Some(instance.into()), None).ok()?;
        let s = scale(hwnd);
        let (w, h) = ((size.0 * s).round() as i32, (size.1 * s).round() as i32);
        // Where the site asked, else centred over Valerie's window.
        let (x, y) = match position {
            Some((x, y)) => ((x * s).round() as i32, (y * s).round() as i32),
            None => {
                let mut r = RECT::default();
                let _ = GetWindowRect(GetForegroundWindow(), &mut r);
                (r.left + (r.right - r.left - w) / 2, r.top + (r.bottom - r.top - h) / 2)
            }
        };
        let round: i32 = 2; // DWMWCP_ROUND
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const i32 as _, 4);
        let on: i32 = dark().into();
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const i32 as _, 4);
        // Keeps DWM's shadow without a title bar.
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 });
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_FRAMECHANGED | SWP_NOZORDER);

        // The pop-up speaks as its opener does, so a site sees the same browser before and after signing in.
        let agent = opener.Settings().ok().and_then(|s| windows::core::Interface::cast::<ICoreWebView2Settings2>(&s).ok()).and_then(|s| {
            let mut p = PWSTR::null();
            s.UserAgent(&mut p).ok()?;
            let a = p.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
            a
        });
        let title_hwnd = hwnd.0 as isize;
        let mut builder = WebViewBuilder::new()
            .with_environment(environment.clone())
            .with_bounds(page_rect(hwnd, false))
            .with_focused(true)
            .with_document_title_changed_handler(move |t| {
                let _ = SetWindowTextW(HWND(title_hwnd as _), &HSTRING::from(t));
            });
        if let Some(a) = agent.as_deref() {
            builder = builder.with_user_agent(a);
        }
        let webview = match builder.build_as_child(&Host(hwnd)) {
            Ok(w) => w,
            Err(e) => {
                crate::log::line("page", format!("the pop-up window could not be made, so WebView2 opens its own: {e}"));
                let _ = DestroyWindow(hwnd);
                return None;
            }
        };
        let core = webview.webview();
        // window.close() from the page (a finished sign-in) closes the whole pane.
        let mut token = Default::default();
        let _ = core.add_WindowCloseRequested(
            &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                crate::log::line("popup-debug", "close requested by the page");
                let _ = PostMessageW(Some(HWND(title_hwnd as _)), WM_CLOSE, WPARAM(0), LPARAM(0));
                Ok(())
            })),
            &mut token,
        );
        POPUPS.with(|m| m.borrow_mut().insert(hwnd.0 as isize, Popup { webview, open: false, hover: None, pressed: None }));
        let _ = ShowWindow(hwnd, SW_SHOW);
        repaint(hwnd);
        let _ = SetForegroundWindow(hwnd);
        Some(core)
    }
}
