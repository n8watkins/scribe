//! Linux implementation of the platform layer.
//!
//! Which way keystrokes go is picked per paste, from the session type
//! (`session`):
//!
//! - X11 sessions, and X11 (XWayland) apps on Wayland: XTEST keystrokes, with
//!   focus and window identity read over X11 (`x11`).
//! - Native Wayland apps: the XDG RemoteDesktop portal (`portal`), which the
//!   user allows once; the grant is saved and reused (`grant_store`).
//! - Where neither works (no portal, permission declined, Scribe's own
//!   window in front), the text is copied and the user told to paste.
//!
//! Global hotkeys on Wayland go through the GlobalShortcuts portal
//! (`shortcuts`). The clipboard goes through arboard, natively on Wayland
//! desktops with the data-control protocol and through XWayland elsewhere.

mod grant_store;
mod host;
mod portal;
#[cfg(test)]
mod portal_dbus_tests;
mod session;
pub mod shortcuts;
mod x11;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
    thread,
    time::Duration,
};

use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

pub use self::session::{InsertPath, PortalPaste, SessionKind};
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

/// Sets up the portal clients: Scribe's app id (so the desktop can store
/// its grants) and the folder the grants are saved in. Called once at
/// startup, before any paste or hotkey.
pub fn init(state_dir: PathBuf, app_id: &str) {
    host::set_app_id(app_id);
    portal::set_state_dir(state_dir);
}

pub fn session_kind() -> SessionKind {
    SessionKind::detect()
}

/// Scribe's own windows that have the keyboard focus, by label. On Wayland
/// Scribe's windows are native Wayland ones that X11 can't see, so the focus
/// guard asks this instead.
static OWN_FOCUS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Records a focus change of one of Scribe's windows.
pub fn note_window_focus(label: &str, focused: bool) {
    if let Ok(mut windows) = OWN_FOCUS.lock() {
        let windows = windows.get_or_insert_with(HashSet::new);
        if focused {
            windows.insert(label.to_string());
        } else {
            windows.remove(label);
        }
    }
}

fn own_window_focused() -> bool {
    OWN_FOCUS
        .lock()
        .ok()
        .and_then(|windows| windows.as_ref().map(|windows| !windows.is_empty()))
        .unwrap_or(false)
}

/// Why a paste can't reach the app in front, one sentence each; the caller
/// adds "Copied. Press Ctrl+V to paste."
const NO_DISPLAY: &str = "Scribe can't reach your desktop to paste.";
const NO_PORTAL: &str = "Your desktop doesn't let apps paste into Wayland apps.";
const PORTAL_DECLINED: &str =
    "Pasting into Wayland apps is off. You can allow it on the Dictation page.";
const PORTAL_FAILED: &str = "Scribe couldn't get your desktop's OK to paste.";
const OWN_WINDOW: &str = "Scribe's own window was in front.";

/// The insert path for the app in front right now, without asking the user
/// anything. `Err` holds the reason when nothing can paste.
fn current_path() -> Result<InsertPath, &'static str> {
    let session = session_kind();
    let display = x11::Display::open();
    if session == SessionKind::X11 {
        return display.map(|_| InsertPath::X11).ok_or(NO_DISPLAY);
    }
    let x11_front = display
        .as_ref()
        .is_some_and(|display| display.active_window().is_some());
    if session == SessionKind::Wayland && !x11_front && own_window_focused() {
        return Err(OWN_WINDOW);
    }
    let portal = if session == SessionKind::Wayland && !x11_front {
        portal::paste_state().0
    } else {
        PortalPaste::Unavailable
    };
    match session::insert_path(session, x11_front, portal) {
        InsertPath::CopyOnly => Err(match (session, portal) {
            (SessionKind::Wayland, PortalPaste::Declined) => PORTAL_DECLINED,
            (SessionKind::Wayland, _) => NO_PORTAL,
            _ => NO_DISPLAY,
        }),
        path => Ok(path),
    }
}

/// The method the Dashboard shows: how a dictation reaches native apps in
/// this session, whatever is in front right now.
pub fn insert_method() -> InsertPath {
    let session = session_kind();
    let portal = if session == SessionKind::Wayland {
        portal::paste_state().0
    } else {
        PortalPaste::Unavailable
    };
    session::session_insert_path(session, x11::Display::open().is_some(), portal)
}

/// Whether pasting into Wayland apps needs the portal permission, and
/// whether the user gave it. `None` outside Wayland or without the portal.
pub fn wayland_paste_permission() -> Option<(PortalPaste, bool)> {
    if session_kind() != SessionKind::Wayland {
        return None;
    }
    let (state, granted) = portal::paste_state();
    (state != PortalPaste::Unavailable).then_some((state, granted))
}

/// Shows the desktop's permission dialog now (the Dictation page's
/// button), even if it was turned down before.
pub fn request_wayland_paste_permission() -> Result<(), CommandError> {
    portal::prepare(true).map_err(|failure| match failure {
        portal::PortalFailure::Declined => CommandError::new(
            "paste_permission_declined",
            "Pasting stays off. Scribe will copy your words for you to paste.",
        ),
        other => CommandError::new(
            "paste_permission_failed",
            format!("Could not ask your desktop for the paste permission: {other}."),
        ),
    })
}

/// The command a desktop keyboard shortcut runs to control Scribe, without
/// the action: the AppImage when Scribe runs from one, otherwise this
/// binary. Quoted when the path has spaces.
pub fn control_command() -> Option<String> {
    let path = std::env::var_os("APPIMAGE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())?;
    let path = path.to_string_lossy().into_owned();
    Some(if path.contains(char::is_whitespace) {
        format!("\"{path}\"")
    } else {
        path
    })
}

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

/// Why Scribe can't paste into the app in front, if it can't. On Wayland
/// this is also where the RemoteDesktop session opens, so the one-time
/// permission dialog shows before Scribe borrows the clipboard (and an
/// answer of no leaves the text copied instead).
pub fn paste_unavailable() -> Option<&'static str> {
    match current_path() {
        Ok(InsertPath::Portal) => match portal::prepare(false) {
            Ok(()) => None,
            Err(portal::PortalFailure::Declined) => Some(PORTAL_DECLINED),
            Err(portal::PortalFailure::Unavailable(_)) => Some(NO_PORTAL),
            Err(failure) => {
                log::warn!("RemoteDesktop portal: {failure}");
                Some(PORTAL_FAILED)
            }
        },
        Ok(_) => None,
        Err(reason) => Some(reason),
    }
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
    if current_path() == Ok(InsertPath::Portal) {
        shortcuts::wait_for_release(HOTKEY_RELEASE_WAIT);
        return portal::send_keys(portal::typing_keys(text))
            .map_err(|failure| input_failed("paste_failed", "typed text", failure.to_string()));
    }
    let display = display_or("direct_insert_unsupported", "Typing text into other apps")?;
    display
        .type_text(text)
        .map_err(|error| input_failed("paste_failed", "typed text", error))
}

/// How long a paste waits for a held portal hotkey to come up.
const HOTKEY_RELEASE_WAIT: Duration = Duration::from_millis(1_500);

/// Ctrl+V, or Shift+Insert in a terminal (where Ctrl+V is a literal-next
/// key). Terminals such as xterm paste the PRIMARY selection on Shift+Insert,
/// so the clipboard text is put there too; the clipboard snapshot puts the
/// previous PRIMARY text back. Native Wayland apps get Shift+Insert through
/// the portal, since Wayland can't say whether the app in front is a
/// terminal.
pub fn send_paste_shortcut() -> Result<(), CommandError> {
    if current_path() == Ok(InsertPath::Portal) {
        copy_clipboard_to_primary();
        shortcuts::wait_for_release(HOTKEY_RELEASE_WAIT);
        return portal::send_keys(portal::PASTE_KEYS.to_vec()).map_err(|failure| {
            input_failed("paste_failed", "the paste shortcut", failure.to_string())
        });
    }
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

/// Selection capture needs an X11 app in front to send Ctrl+C to. Native
/// Wayland apps are left out: Scribe can't tell a terminal there, where
/// Ctrl+C would stop the running program.
pub fn can_capture_selection() -> bool {
    current_path() == Ok(InsertPath::X11)
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
