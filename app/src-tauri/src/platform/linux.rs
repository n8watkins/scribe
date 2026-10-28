//! Linux implementation of the platform layer. Nothing here talks to the
//! desktop yet: Scribe can't tell which window is in front or send keystrokes,
//! so pastes and selection copies report that they are unsupported and the
//! clipboard is never borrowed.

use std::collections::HashMap;

use super::{unsupported, PasteTarget, WindowId};
use crate::error::CommandError;

/// Nothing is captured on Linux yet.
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

/// Selection capture needs synthetic keystrokes, which Linux lacks so far.
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
