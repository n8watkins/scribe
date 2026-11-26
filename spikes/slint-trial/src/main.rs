//! Scribe Slint trial: a standalone go or no-go build. The tray icon, the pill
//! and a main window made on demand, drawn by Slint on the CPU, with fixture
//! data and no Scribe backend.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod fixtures;
mod pill;
mod place;
mod probe;
mod status;
mod wave;
#[cfg(windows)]
mod win;

slint::include_modules!();

use std::cell::Cell;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Pill,
    Main { activate: bool },
}

thread_local! {
    static NEXT: Cell<WindowKind> = const { Cell::new(WindowKind::Main { activate: true }) };
}

/// Says which window the next component creates, for the attributes hook.
pub fn next_window(kind: WindowKind) {
    NEXT.with(|n| n.set(kind));
}

pub fn next_kind() -> WindowKind {
    NEXT.with(Cell::get)
}

#[cfg(windows)]
pub fn hwnd_of(w: &slint::Window) -> Option<win::Hwnd> {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    w.with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(win::Hwnd(h.hwnd.get())),
        _ => None,
    })
    .flatten()
}

pub fn set_clipboard(text: &str) -> bool {
    #[cfg(windows)]
    {
        win::set_clipboard(text)
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        false
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
