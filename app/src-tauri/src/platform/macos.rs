//! macOS implementation of the platform layer: a stub until the macOS port
//! lands. Pastes and selection copies report that they are unsupported and
//! the clipboard is never borrowed.

use std::collections::HashMap;

use super::{unsupported, PasteTarget, WindowId};
use crate::error::CommandError;

/// Nothing is captured on macOS yet.
#[derive(Debug, Default)]
pub struct ClipboardSnapshot;

pub fn foreground_window() -> WindowId {
    0
}

pub fn foreground_target() -> Option<PasteTarget> {
    None
}

pub fn is_own_window(_window: WindowId) -> bool {
    false
}

pub fn is_console_window(_window: WindowId) -> bool {
    false
}

pub fn runs_elevated_above_us(_process_id: u32) -> bool {
    false
}

/// Without a way to read or change focus, the paste goes to whatever is in
/// front.
pub fn ensure_foreign_focus() -> Result<(), CommandError> {
    Ok(())
}

pub fn type_text(_text: &str) -> Result<(), CommandError> {
    Err(unsupported(
        "direct_insert_unsupported",
        "Typing text into other apps",
    ))
}

pub fn send_paste_shortcut() -> Result<(), CommandError> {
    Err(unsupported("paste_unsupported", "Pasting into other apps"))
}

pub fn send_copy_shortcut(_console: bool) -> Result<(), CommandError> {
    Err(unsupported(
        "selection_copy_unsupported",
        "Copying the selection from other apps",
    ))
}

/// Synthetic keystrokes aren't implemented on macOS yet.
pub fn paste_unavailable() -> Option<&'static str> {
    Some("Scribe can't paste into other apps on macOS yet.")
}

pub fn open_clipboard() -> Result<arboard::Clipboard, arboard::Error> {
    arboard::Clipboard::new()
}

/// Selection capture needs synthetic keystrokes, which macOS lacks so far.
pub fn can_capture_selection() -> bool {
    false
}

pub fn clipboard_sequence_number() -> u32 {
    0
}

pub fn save_clipboard_snapshot() -> ClipboardSnapshot {
    ClipboardSnapshot
}

/// Nothing was snapshotted, so there is nothing to restore.
pub fn restore_clipboard_snapshot(_snapshot: &ClipboardSnapshot) -> Result<(), CommandError> {
    Ok(())
}

pub fn set_private_clipboard_text(
    clipboard: &mut arboard::Clipboard,
    text: &str,
) -> Result<(), arboard::Error> {
    clipboard.set_text(text.to_string())
}

/// No system endpoint ids on macOS: microphones are identified by their cpal
/// names.
pub fn audio_endpoint_ids_by_name() -> HashMap<String, Vec<String>> {
    HashMap::new()
}

/// Core Audio lists only real inputs, shown under their own names.
pub fn microphone_label(device_name: &str) -> Option<String> {
    Some(device_name.to_string())
}

pub fn microphone_id(_device_name: &str) -> Option<String> {
    None
}
