//! Win32 details Slint does not cover: the pill's tool-window, never-activate
//! style, placement in physical pixels, monitors, the clipboard, and posting
//! mouse messages to our own window for the no-input checks.

use crate::place::Rect;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST,
    PostMessageW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hwnd(pub isize);

impl Hwnd {
    fn raw(self) -> HWND {
        self.0 as HWND
    }
}

/// Never in the taskbar or Alt+Tab (`WS_EX_TOOLWINDOW`, no `WS_EX_APPWINDOW`),
/// never activated by a click (`WS_EX_NOACTIVATE`), always on top, and no
/// Windows 11 rounding on top of the pill's own.
pub fn style_pill(h: Hwnd) {
    unsafe {
        let style = GetWindowLongPtrW(h.raw(), GWL_EXSTYLE);
        let wanted = (style | (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST) as isize)
            & !(WS_EX_APPWINDOW as isize);
        SetWindowLongPtrW(h.raw(), GWL_EXSTYLE, wanted);
        SetWindowPos(
            h.raw(),
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        let pref: i32 = DWMWCP_DONOTROUND;
        DwmSetWindowAttribute(
            h.raw(),
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&raw const pref).cast(),
            size_of::<i32>() as u32,
        );
    }
}

pub fn ex_style(h: Hwnd) -> isize {
    unsafe { GetWindowLongPtrW(h.raw(), GWL_EXSTYLE) }
}

pub fn show_inactive(h: Hwnd) {
    unsafe {
        ShowWindow(h.raw(), SW_SHOWNOACTIVATE);
    }
}

pub fn hide(h: Hwnd) {
    unsafe {
        ShowWindow(h.raw(), SW_HIDE);
    }
}

/// Moves and sizes in one call (physical pixels), topmost, never activating.
pub fn set_bounds(h: Hwnd, r: Rect) {
    unsafe {
        SetWindowPos(h.raw(), HWND_TOPMOST, r.x, r.y, r.w, r.h, SWP_NOACTIVATE);
    }
}

pub fn move_to(h: Hwnd, x: i32, y: i32) {
    unsafe {
        SetWindowPos(
            h.raw(),
            HWND_TOPMOST,
            x,
            y,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE,
        );
    }
}

pub fn bounds(h: Hwnd) -> Rect {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetWindowRect(h.raw(), &raw mut r);
    }
    Rect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    }
}

pub fn scale(h: Hwnd) -> f32 {
    let dpi = unsafe { GetDpiForWindow(h.raw()) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&raw mut p);
    }
    (p.x, p.y)
}

pub fn foreground() -> isize {
    unsafe { GetForegroundWindow() as isize }
}

fn monitor_rect(m: HMONITOR) -> Option<Rect> {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(m, &raw mut info) } == 0 {
        return None;
    }
    let r = info.rcMonitor;
    Some(Rect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    })
}

/// Every monitor's full rectangle (what Tauri's `availableMonitors` reports).
pub fn monitors() -> Vec<Rect> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        let list = unsafe { &mut *(data as *mut Vec<Rect>) };
        if let Some(r) = monitor_rect(m) {
            list.push(r);
        }
        1
    }
    let mut list: Vec<Rect> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            (&raw mut list) as LPARAM,
        );
    }
    list
}

pub fn primary_monitor() -> Option<Rect> {
    let m = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    monitor_rect(m)
}

fn lparam(x: i32, y: i32) -> LPARAM {
    ((y as u32 as LPARAM) << 16) | (x as u16 as LPARAM)
}

/// Posts a left press, a 1 px move and a release at client point (x, y) to our
/// own window. No real input is synthesised and nothing else receives it.
pub fn post_click(h: Hwnd, x: i32, y: i32) {
    const MK_LBUTTON: WPARAM = 0x0001;
    unsafe {
        PostMessageW(h.raw(), WM_MOUSEMOVE, 0, lparam(x, y));
        PostMessageW(h.raw(), WM_LBUTTONDOWN, MK_LBUTTON, lparam(x, y));
        PostMessageW(h.raw(), WM_MOUSEMOVE, MK_LBUTTON, lparam(x + 1, y));
        PostMessageW(h.raw(), WM_LBUTTONUP, 0, lparam(x + 1, y));
    }
}

/// Puts `text` on the clipboard as Unicode text.
pub fn set_clipboard(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let bytes = wide.len() * 2;
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        let mut ok = false;
        if !mem.is_null() {
            let dst = GlobalLock(mem) as *mut u16;
            if !dst.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                GlobalUnlock(mem);
                ok = !SetClipboardData(CF_UNICODETEXT as u32, mem as _).is_null();
            }
        }
        CloseClipboard();
        ok
    }
}

/// Hands idle pages back to Windows, as Windows does for a hidden webview.
pub fn trim_working_set() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}
