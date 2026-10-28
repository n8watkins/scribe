//! The operating-system layer. Everything Scribe does to other apps (reading
//! which window is in front, handing focus back, synthetic paste, copy and
//! typing, and borrowing the clipboard) goes through this module, so the rest
//! of the backend is the same on every system.
//!
//! Each system module exports the same set of functions:
//!
//! - Focus and identity: `foreground_window`, `foreground_target`,
//!   `is_own_window`, `is_console_window`, `runs_elevated_above_us`,
//!   `ensure_foreign_focus`.
//! - Keystrokes: `send_paste_shortcut`, `send_copy_shortcut`, `type_text`,
//!   `can_capture_selection`.
//! - Clipboard: `ClipboardSnapshot`, `save_clipboard_snapshot`,
//!   `restore_clipboard_snapshot`, `clipboard_sequence_number`,
//!   `set_private_clipboard_text`.
//! - Audio: `audio_endpoint_ids_by_name`, `microphone_id` and
//!   `microphone_label` (which devices to offer and how to name them;
//!   recording itself is cross-platform through cpal).
//!
//! An operation a system can't do yet returns a `CommandError` from
//! [`unsupported`], never a panic.

use crate::error::CommandError;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

#[cfg(not(any(windows, target_os = "macos")))]
mod linux;
#[cfg(not(any(windows, target_os = "macos")))]
pub use self::linux::*;

/// A top-level window, as the system identifies it (an HWND on Windows).
/// Zero means "no window".
pub type WindowId = isize;

/// The window a dictation started in. Held so the result is pasted back
/// there, and only there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteTarget {
    pub window: WindowId,
    pub process_id: u32,
}

/// The error for an operation this system doesn't support yet. `what` names
/// the feature in a few words, starting with a capital letter.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn unsupported(code: &str, what: &str) -> CommandError {
    CommandError::new(code, format!("{what} is not supported on this system yet."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_errors_say_what_is_missing() {
        let error = unsupported("paste_unsupported", "Pasting into other apps");
        assert_eq!(error.code, "paste_unsupported");
        assert_eq!(
            error.message,
            "Pasting into other apps is not supported on this system yet."
        );
    }
}
