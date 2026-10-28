//! Linux implementation of the platform layer. Focus, window identity and
//! synthetic keystrokes go through X11 (`x11`), which covers X11 sessions
//! and, on Wayland sessions, X11 apps running under XWayland. A native
//! Wayland app can't be pasted into yet: the paste falls back to copying
//! (`paste_unavailable`). The clipboard goes through arboard.

mod x11;

use std::{collections::HashMap, sync::Mutex, thread, time::Duration};

use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

use super::{unsupported, PasteTarget, WindowId};
use crate::error::CommandError;

/// WM_CLASS names (instance or class, lowercase) of terminal emulators, where
/// Ctrl+C interrupts the running program and Ctrl+V is not paste.
const TERMINAL_CLASSES: [&str; 22] = [
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "gnome-terminal",
    "gnome-terminal-server",
    "kgx",
    "ptyxis",
    "konsole",
    "xfce4-terminal",
    "mate-terminal",
    "lxterminal",
    "qterminal",
    "terminator",
    "tilix",
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "st",
    "st-256color",
    "terminology",
];

fn is_terminal_class(instance: &str, class: &str) -> bool {
    let instance = instance.to_ascii_lowercase();
    let class = class.to_ascii_lowercase();
    TERMINAL_CLASSES
        .iter()
        .any(|name| *name == instance || *name == class)
        || class.ends_with(".wezterm")
        || class == "org.wezfurlong.wezterm"
}

/// True on a Wayland desktop, where only X11 (XWayland) apps can be reached.
fn wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty())
}

/// What a Wayland session needs to say when the app in front can't be
/// reached. One line; the caller adds "Copied. Press Ctrl+V to paste."
const WAYLAND_PASTE: &str = "Scribe can't paste into Wayland apps yet.";

fn input_failed(code: &str, what: &str, error: String) -> CommandError {
    CommandError::new(
        code,
        format!("Could not send {what} to the focused app. {error}"),
    )
}

fn display_or(code: &str, what: &str) -> Result<x11::Display, CommandError> {
    x11::Display::open().ok_or_else(|| unsupported(code, what))
}

pub fn foreground_window() -> WindowId {
    x11::Display::open()
        .and_then(|display| display.active_window())
        .map(|window| window as WindowId)
        .unwrap_or(0)
}

pub fn foreground_target() -> Option<PasteTarget> {
    let display = x11::Display::open()?;
    let window = display.active_window()?;
    let process_id = display.window_pid(window);
    if process_id == std::process::id() {
        return None;
    }
    Some(PasteTarget {
        window: window as WindowId,
        process_id,
    })
}

pub fn is_own_window(window: WindowId) -> bool {
    let Ok(window) = u32::try_from(window) else {
        return false;
    };
    window != 0
        && x11::Display::open()
            .is_some_and(|display| display.window_pid(window) == std::process::id())
}

pub fn is_console_window(window: WindowId) -> bool {
    let Ok(window) = u32::try_from(window) else {
        return false;
    };
    window != 0
        && x11::Display::open()
            .and_then(|display| display.window_class(window))
            .is_some_and(|(instance, class)| is_terminal_class(&instance, &class))
}

/// Linux has no integrity levels that silently drop synthetic input.
pub fn runs_elevated_above_us(_process_id: u32) -> bool {
    false
}

/// Why Scribe can't paste into the app in front, if it can't: no X server
/// at all, or a Wayland session whose focused app isn't an X11 one.
pub fn paste_unavailable() -> Option<&'static str> {
    let Some(display) = x11::Display::open() else {
        return Some(WAYLAND_PASTE);
    };
    if wayland_session() && display.active_window().is_none() {
        return Some(WAYLAND_PASTE);
    }
    None
}

/// When one of Scribe's own windows is in front, activates the app window
/// below it, so a paste from the dashboard lands in the app the user was in.
pub fn ensure_foreign_focus() -> Result<(), CommandError> {
    let Some(display) = x11::Display::open() else {
        return Ok(());
    };
    let own = std::process::id();
    let Some(active) = display.active_window() else {
        return Ok(());
    };
    if display.window_pid(active) != own {
        return Ok(());
    }
    let target = display
        .windows_top_down()
        .into_iter()
        .find(|window| *window != active && display.window_pid(*window) != own);
    match target {
        Some(window) => {
            display.activate(window);
            // The target needs time to take keyboard focus before the keys.
            thread::sleep(Duration::from_millis(100));
            Ok(())
        }
        None => {
            log::warn!("Focus guard found no other app window; skipping paste.");
            Err(CommandError::new(
                "paste_target_unavailable",
                "Could not find another app window to paste into. Click into the target app and try again.",
            ))
        }
    }
}

pub fn type_text(text: &str) -> Result<(), CommandError> {
    let display = display_or("direct_insert_unsupported", "Typing text into other apps")?;
    display
        .type_text(text)
        .map_err(|error| input_failed("paste_failed", "typed text", error))
}

/// Ctrl+V, or Shift+Insert in a terminal (where Ctrl+V is a literal-next
/// key). Terminals such as xterm paste the PRIMARY selection on Shift+Insert,
/// so the clipboard text is put there too; the clipboard snapshot puts the
/// previous PRIMARY text back.
pub fn send_paste_shortcut() -> Result<(), CommandError> {
    let display = display_or("paste_unsupported", "Pasting into other apps")?;
    let terminal = display
        .active_window()
        .and_then(|window| display.window_class(window))
        .is_some_and(|(instance, class)| is_terminal_class(&instance, &class));
    let chord: &[u32] = if terminal {
        copy_clipboard_to_primary();
        &[x11::XK_SHIFT_L, x11::XK_INSERT]
    } else {
        &[x11::XK_CONTROL_L, x11::XK_V]
    };
    display
        .send_chord(chord)
        .map_err(|error| input_failed("paste_failed", "the paste shortcut", error))
}

/// Ctrl+C, or Ctrl+Shift+C in a terminal.
pub fn send_copy_shortcut(console: bool) -> Result<(), CommandError> {
    let display = display_or(
        "selection_copy_unsupported",
        "Copying the selection from other apps",
    )?;
    let chord: &[u32] = if console {
        &[x11::XK_CONTROL_L, x11::XK_SHIFT_L, x11::XK_C]
    } else {
        &[x11::XK_CONTROL_L, x11::XK_C]
    };
    display
        .send_chord(chord)
        .map_err(|error| input_failed("selection_copy_failed", "the copy shortcut", error))
}

/// Selection capture needs an X11 app in front to send Ctrl+C to.
pub fn can_capture_selection() -> bool {
    paste_unavailable().is_none()
}

pub fn clipboard_sequence_number() -> u32 {
    x11::clipboard_change_count()
}

/// arboard serves X11 clipboard contents from a background thread that stops,
/// taking the contents with it unless a clipboard manager copies them, when
/// the last `Clipboard` is dropped. One instance is kept for the life of the
/// app so what Scribe copies stays pasteable.
static CLIPBOARD_KEEPALIVE: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

pub fn open_clipboard() -> Result<arboard::Clipboard, arboard::Error> {
    if let Ok(mut keepalive) = CLIPBOARD_KEEPALIVE.lock() {
        if keepalive.is_none() {
            *keepalive = Some(arboard::Clipboard::new()?);
        }
    }
    arboard::Clipboard::new()
}

fn copy_clipboard_to_primary() {
    let Ok(mut clipboard) = open_clipboard() else {
        return;
    };
    if let Ok(text) = clipboard.get_text() {
        let _ = clipboard
            .set()
            .clipboard(LinuxClipboardKind::Primary)
            .exclude_from_history()
            .text(text);
    }
}

/// What was on the clipboard (text or an image) and the PRIMARY selection's
/// text, so a borrowed clipboard can be put back.
#[derive(Default)]
pub struct ClipboardSnapshot {
    clipboard: Option<ClipboardContent>,
    primary: Option<String>,
}

impl std::fmt::Debug for ClipboardSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardSnapshot")
            .field("clipboard", &self.clipboard.is_some())
            .field("primary", &self.primary.is_some())
            .finish()
    }
}

enum ClipboardContent {
    Text(String),
    Image(arboard::ImageData<'static>),
}

pub fn save_clipboard_snapshot() -> ClipboardSnapshot {
    let Ok(mut clipboard) = open_clipboard() else {
        return ClipboardSnapshot::default();
    };
    let content = match clipboard.get_text() {
        Ok(text) => Some(ClipboardContent::Text(text)),
        Err(_) => clipboard.get_image().ok().map(ClipboardContent::Image),
    };
    let primary = clipboard
        .get()
        .clipboard(LinuxClipboardKind::Primary)
        .text()
        .ok();
    ClipboardSnapshot {
        clipboard: content,
        primary,
    }
}

pub fn restore_clipboard_snapshot(snapshot: &ClipboardSnapshot) -> Result<(), CommandError> {
    let mut clipboard = open_clipboard().map_err(|error| {
        CommandError::new(
            "clipboard_unavailable",
            format!("Could not access the clipboard. {error}"),
        )
    })?;
    let restored = match &snapshot.clipboard {
        Some(ClipboardContent::Text(text)) => {
            clipboard.set().exclude_from_history().text(text.clone())
        }
        Some(ClipboardContent::Image(image)) => {
            clipboard.set().exclude_from_history().image(image.clone())
        }
        None => clipboard.clear(),
    };
    if let Some(primary) = &snapshot.primary {
        let _ = clipboard
            .set()
            .clipboard(LinuxClipboardKind::Primary)
            .exclude_from_history()
            .text(primary.clone());
    }
    restored.map_err(|error| {
        CommandError::new(
            "clipboard_set_failed",
            format!("Could not restore the clipboard. {error}"),
        )
    })
}

/// Marked so clipboard managers (KDE Klipper and others honoring
/// x-kde-passwordManagerHint) don't keep it.
pub fn set_private_clipboard_text(
    clipboard: &mut arboard::Clipboard,
    text: &str,
) -> Result<(), arboard::Error> {
    clipboard
        .set()
        .exclude_from_history()
        .text(text.to_string())
}

/// No system endpoint ids on Linux: microphones are identified by their cpal
/// names.
pub fn audio_endpoint_ids_by_name() -> HashMap<String, Vec<String>> {
    HashMap::new()
}

/// The microphones worth offering out of everything ALSA lists, and what to
/// call them. ALSA also lists resamplers, mixers, surround layouts and each
/// card several times over (`hw:`, `plughw:`, `front:`, `dsnoop:`). Kept are
/// the system default, the sound servers (which follow the input chosen in
/// the desktop's sound settings) and each card once, by its `sysdefault`
/// entry.
pub fn microphone_label(device_name: &str) -> Option<String> {
    match device_name {
        "default" => Some("System default".to_string()),
        "pipewire" => Some("PipeWire".to_string()),
        "pulse" => Some("PulseAudio".to_string()),
        _ => device_name
            .strip_prefix("sysdefault:CARD=")
            .filter(|card| !card.is_empty())
            .map(str::to_string),
    }
}

/// The ALSA device name is stable across restarts, unlike its position in
/// the list.
pub fn microphone_id(device_name: &str) -> Option<String> {
    Some(format!("alsa:{device_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_terminals_by_window_class() {
        assert!(is_terminal_class("xterm", "XTerm"));
        assert!(is_terminal_class("gnome-terminal-server", "Gnome-terminal"));
        assert!(is_terminal_class("konsole", "konsole"));
        assert!(is_terminal_class("Alacritty", "Alacritty"));
        assert!(is_terminal_class(
            "org.wezfurlong.wezterm",
            "org.wezfurlong.wezterm"
        ));
        assert!(!is_terminal_class("gedit", "Gedit"));
        assert!(!is_terminal_class("Navigator", "firefox"));
        assert!(!is_terminal_class("", ""));
    }

    #[test]
    fn offers_the_default_the_sound_servers_and_each_card_once() {
        assert_eq!(
            microphone_label("default").as_deref(),
            Some("System default")
        );
        assert_eq!(microphone_label("pipewire").as_deref(), Some("PipeWire"));
        assert_eq!(microphone_label("pulse").as_deref(), Some("PulseAudio"));
        assert_eq!(
            microphone_label("sysdefault:CARD=C920").as_deref(),
            Some("C920")
        );
        for plumbing in [
            "null",
            "samplerate",
            "speexrate",
            "lavrate",
            "jack",
            "oss",
            "upmix",
            "vdownmix",
            "hw:CARD=PCH,DEV=0",
            "plughw:CARD=PCH,DEV=0",
            "front:CARD=PCH,DEV=0",
            "dsnoop:CARD=PCH,DEV=0",
            "surround51:CARD=PCH,DEV=0",
            "sysdefault:CARD=",
        ] {
            assert_eq!(microphone_label(plumbing), None, "{plumbing}");
        }
    }

    #[test]
    fn microphone_ids_are_the_alsa_names() {
        assert_eq!(microphone_id("pulse").as_deref(), Some("alsa:pulse"));
    }
}
