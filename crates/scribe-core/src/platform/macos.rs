//! macOS implementation of the platform layer. The app in front comes from
//! AppKit (NSWorkspace) and its windows from the window server
//! (CGWindowList), keystrokes are posted as CGEvents, and the clipboard is
//! the general NSPasteboard. Posting keystrokes needs the Accessibility
//! permission: without it `paste_unavailable` says so and the paste falls
//! back to copying.

use std::{
    collections::HashMap,
    thread,
    time::{Duration, Instant},
};

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSPasteboard, NSPasteboardItem, NSPasteboardWriting,
    NSRunningApplication, NSWorkspace,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType,
};
use objc2_core_graphics::{
    kCGNullWindowID, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerPID, CGEvent, CGEventFlags,
    CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGKeyCode, CGWindowListCopyWindowInfo,
    CGWindowListOption,
};
use objc2_foundation::{NSArray, NSData, NSString, NSURL};

use super::{PasteTarget, WindowId};
use crate::error::CommandError;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// Whether this process may post keystrokes to other apps (System
    /// Settings, Privacy & Security, Accessibility). Returns a C `Boolean`.
    fn AXIsProcessTrusted() -> u8;
    /// The same check; with `kAXTrustedCheckOptionPrompt` set it also adds
    /// the app to the Accessibility list and shows the system prompt.
    fn AXIsProcessTrustedWithOptions(options: Option<&CFDictionary>) -> u8;
    static kAXTrustedCheckOptionPrompt: &'static CFString;
}

/// The Accessibility list in System Settings (Privacy & Security).
const ACCESSIBILITY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// What the paste says when the Accessibility permission is missing. One
/// line; the caller adds "Copied. Press Cmd+V to paste."
const ACCESSIBILITY_NEEDED: &str = "Scribe needs the Accessibility permission to paste. Turn on Scribe in System Settings, Privacy & Security, Accessibility.";

/// ANSI virtual key codes (`kVK_ANSI_V`, `kVK_ANSI_C`). macOS matches
/// Command shortcuts against what the key types in the active layout, so
/// these are V and C on QWERTY, QWERTZ, AZERTY and Colemak, but not on Dvorak.
const KEY_V: CGKeyCode = 9;
const KEY_C: CGKeyCode = 8;

/// The key code typed text is posted with. Apps read the characters from
/// the event's Unicode string, not from the key.
const KEY_TEXT: CGKeyCode = 0;

/// UTF-16 units per typed event. An event can carry a longer string, but
/// many apps read only the first 20 units of each one.
const UNITS_PER_EVENT: usize = 20;

/// The modifiers whose held state could combine with typed text.
const MODIFIER_FLAGS: CGEventFlags = CGEventFlags::MaskShift
    .union(CGEventFlags::MaskControl)
    .union(CGEventFlags::MaskAlternate)
    .union(CGEventFlags::MaskCommand);

/// Error codes for failed keystroke injection, per caller.
const PASTE_FAILED: &str = "paste_failed";
const SELECTION_COPY_FAILED: &str = "selection_copy_failed";

/// Whether the user has granted Scribe the Accessibility permission, which
/// posting keystrokes to other apps needs. macOS checks it per code
/// signature, so an ad-hoc signed update may need it granted again.
pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Asks for the Accessibility permission when it is missing: macOS adds
/// Scribe to the Accessibility list and shows its prompt, and the list opens
/// in System Settings, where the user turns Scribe on.
pub fn request_accessibility() {
    // SAFETY: a constant string exported by ApplicationServices.
    let prompt = unsafe { kAXTrustedCheckOptionPrompt };
    let options = CFDictionary::from_slices(&[prompt], &[CFBoolean::new(true)]);
    // SAFETY: the options dictionary maps the prompt key to a boolean.
    if unsafe { AXIsProcessTrustedWithOptions(Some(options.as_ref())) } != 0 {
        return;
    }
    autoreleasepool(|_| {
        let opened = NSURL::URLWithString(&NSString::from_str(ACCESSIBILITY_SETTINGS))
            .is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url));
        if !opened {
            log::warn!("Could not open the Accessibility settings");
        }
    });
}

fn own_pid() -> i32 {
    std::process::id() as i32
}

/// The process id of the app that receives keystrokes now.
fn frontmost_pid() -> Option<i32> {
    autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier())
    })
}

/// One window as the window server lists it.
struct ScreenWindow {
    number: i64,
    owner: i32,
    layer: i64,
}

fn window_list(option: CGWindowListOption, relative_to: u32) -> Vec<ScreenWindow> {
    let Some(list) = CGWindowListCopyWindowInfo(option, relative_to) else {
        return Vec::new();
    };
    // SAFETY: the window list is an array of dictionaries keyed by strings.
    let list =
        unsafe { CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(list) };
    list.iter()
        .filter_map(|info| {
            // SAFETY: the keys are constant strings exported by CoreGraphics.
            let (number, owner, layer) =
                unsafe { (kCGWindowNumber, kCGWindowOwnerPID, kCGWindowLayer) };
            Some(ScreenWindow {
                number: number_value(&info, number)?,
                owner: i32::try_from(number_value(&info, owner)?).ok()?,
                layer: number_value(&info, layer).unwrap_or(0),
            })
        })
        .collect()
}

fn number_value(info: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<i64> {
    info.get(key)?.downcast::<CFNumber>().ok()?.as_i64()
}

/// The ordinary app windows on screen, front to back. Menus, the Dock, the
/// menu bar and floating panels (such as Scribe's pill) sit on other layers.
fn app_windows() -> Vec<ScreenWindow> {
    window_list(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    )
    .into_iter()
    .filter(|window| window.layer == 0)
    .collect()
}

/// The id an app's front window gets here: its window number, or the
/// negated process id when the app shows no window, so two windowless apps
/// never count as the same window.
fn front_window_id(pid: i32) -> WindowId {
    app_windows()
        .into_iter()
        .find(|window| window.owner == pid)
        .map(|window| window.number as WindowId)
        .unwrap_or(-(pid as WindowId))
}

/// The front window of the app in front (see `front_window_id`), or zero
/// when no app is in front.
pub fn foreground_window() -> WindowId {
    frontmost_pid().map(front_window_id).unwrap_or(0)
}

pub fn foreground_target() -> Option<PasteTarget> {
    let pid = frontmost_pid()?;
    if pid == own_pid() {
        return None;
    }
    Some(PasteTarget {
        window: front_window_id(pid),
        process_id: u32::try_from(pid).ok()?,
    })
}

/// True when the window (or the windowless app, for a negative id) is
/// Scribe's own.
pub fn is_own_window(window: WindowId) -> bool {
    let own = own_pid();
    if window < 0 {
        return window == -(own as WindowId);
    }
    let Ok(number) = u32::try_from(window) else {
        return false;
    };
    number != 0
        && window_list(CGWindowListOption::OptionIncludingWindow, number)
            .iter()
            .any(|info| info.number == i64::from(number) && info.owner == own)
}

/// Terminals on macOS copy with Command+C and paste with Command+V like
/// every other app, so none needs a different shortcut.
pub fn is_console_window(_window: WindowId) -> bool {
    false
}

/// macOS has no integrity levels that silently drop synthetic input.
pub fn runs_elevated_above_us(_process_id: u32) -> bool {
    false
}

/// Why Scribe can't paste into the app in front, if it can't: posting
/// keystrokes needs the Accessibility permission.
pub fn paste_unavailable() -> Option<&'static str> {
    (!accessibility_granted()).then_some(ACCESSIBILITY_NEEDED)
}

/// When Scribe is the app in front, activates the app with the frontmost
/// window below it, so a paste from the dashboard lands in the app the user
/// was in.
pub fn ensure_foreign_focus() -> Result<(), CommandError> {
    let own = own_pid();
    if frontmost_pid() != Some(own) {
        return Ok(());
    }
    let mut tried = Vec::new();
    for window in app_windows() {
        if window.owner == own || tried.contains(&window.owner) {
            continue;
        }
        tried.push(window.owner);
        if activate(window.owner) {
            return Ok(());
        }
    }
    log::warn!("Focus guard found no other app window; skipping paste.");
    Err(CommandError::new(
        "paste_target_unavailable",
        "Could not find another app window to paste into. Click into the target app and try again.",
    ))
}

/// Asks the app to come to the front and waits until it has. False when it
/// can't be activated (a background-only process) or didn't come forward.
fn activate(pid: i32) -> bool {
    let requested = autoreleasepool(|_| {
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid).is_some_and(|app| {
            // Deprecated (and ignored) from macOS 14, where the active app,
            // Scribe here, can hand activation over without it. macOS 13
            // still needs it.
            #[allow(deprecated)]
            let options = NSApplicationActivationOptions::ActivateIgnoringOtherApps;
            app.activateWithOptions(options)
        })
    });
    if !requested {
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if frontmost_pid() == Some(pid) {
            // The app needs a moment to make its window key before the keys.
            thread::sleep(Duration::from_millis(100));
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    log::warn!("The app below Scribe did not come to the front in time.");
    false
}

/// Hotkey-triggered pastes can start while the user still holds the chord.
/// Typed text carries explicit (empty) modifier flags, but some apps also
/// read the live keyboard state, so wait briefly for the keys to come up.
fn wait_for_modifier_release() {
    let deadline = Instant::now() + Duration::from_millis(1_500);
    loop {
        let held = CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState)
            .intersects(MODIFIER_FLAGS);
        if !held {
            return;
        }
        if Instant::now() >= deadline {
            log::warn!("Typing with modifier keys still held after 1.5 s");
            return;
        }
        thread::sleep(Duration::from_millis(15));
    }
}

fn input_failed(code: &str, what: &str) -> CommandError {
    CommandError::new(
        code,
        format!("Could not send {what} to the focused app. macOS refused to create the key event."),
    )
}

/// Posts a key press and release. `flags` replaces the modifier state for
/// both events, so modifiers the user still holds can't combine with them.
fn post_key(
    source: &CGEventSource,
    key: CGKeyCode,
    flags: CGEventFlags,
    text: Option<&[u16]>,
) -> bool {
    for key_down in [true, false] {
        let Some(event) = CGEvent::new_keyboard_event(Some(source), key, key_down) else {
            return false;
        };
        CGEvent::set_flags(Some(&event), flags);
        if let Some(units) = text {
            // SAFETY: the pointer and length describe `units`, which
            // outlives the call; the event copies the string.
            unsafe {
                CGEvent::keyboard_set_unicode_string(Some(&event), units.len() as _, units.as_ptr())
            };
        }
        CGEvent::post(CGEventTapLocation::SessionEventTap, Some(&event));
    }
    true
}

fn event_source() -> Option<CFRetained<CGEventSource>> {
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
}

/// Splits UTF-16 text into runs of at most `max` units without cutting a
/// surrogate pair in two.
fn utf16_chunks(units: &[u16], max: usize) -> Vec<&[u16]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let mut end = (start + max).min(units.len());
        if end < units.len() && end - start > 1 && (0xD800..0xDC00).contains(&units[end - 1]) {
            end -= 1;
        }
        chunks.push(&units[start..end]);
        start = end;
    }
    chunks
}

pub fn type_text(text: &str) -> Result<(), CommandError> {
    wait_for_modifier_release();
    let source = event_source().ok_or_else(|| input_failed(PASTE_FAILED, "typed text"))?;
    let units: Vec<u16> = text.encode_utf16().collect();
    for chunk in utf16_chunks(&units, UNITS_PER_EVENT) {
        if !post_key(&source, KEY_TEXT, CGEventFlags::empty(), Some(chunk)) {
            return Err(input_failed(PASTE_FAILED, "typed text"));
        }
        // Keeps a long transcript from flooding the target's event queue.
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// Command+V. Fires at once: the explicit flags keep a still-held hotkey
/// chord from combining with it.
pub fn send_paste_shortcut() -> Result<(), CommandError> {
    let source = event_source().ok_or_else(|| input_failed(PASTE_FAILED, "the paste shortcut"))?;
    if post_key(&source, KEY_V, CGEventFlags::MaskCommand, None) {
        Ok(())
    } else {
        Err(input_failed(PASTE_FAILED, "the paste shortcut"))
    }
}

/// Command+C, in terminals too (see `is_console_window`).
pub fn send_copy_shortcut(_console: bool) -> Result<(), CommandError> {
    let source =
        event_source().ok_or_else(|| input_failed(SELECTION_COPY_FAILED, "the copy shortcut"))?;
    if post_key(&source, KEY_C, CGEventFlags::MaskCommand, None) {
        Ok(())
    } else {
        Err(input_failed(SELECTION_COPY_FAILED, "the copy shortcut"))
    }
}

/// Selection capture sends Command+C, which needs Accessibility.
pub fn can_capture_selection() -> bool {
    accessibility_granted()
}

pub fn open_clipboard() -> Result<arboard::Clipboard, arboard::Error> {
    arboard::Clipboard::new()
}

/// The general pasteboard's change count; it moves on every write by any app.
pub fn clipboard_sequence_number() -> u32 {
    autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount() as u32)
}

/// A full copy of the general pasteboard: for each item, the data of every
/// type it offered (text, rich text, images, file URLs, ...).
#[derive(Default)]
pub struct ClipboardSnapshot(Vec<Vec<(String, Vec<u8>)>>);

impl std::fmt::Debug for ClipboardSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardSnapshot")
            .field("items", &self.0.len())
            .finish()
    }
}

/// Snapshots every item on the general pasteboard. Types whose data can't
/// be produced are skipped; an empty snapshot restores as a cleared
/// pasteboard.
pub fn save_clipboard_snapshot() -> ClipboardSnapshot {
    autoreleasepool(|_| {
        let pasteboard = NSPasteboard::generalPasteboard();
        let Some(items) = pasteboard.pasteboardItems() else {
            return ClipboardSnapshot::default();
        };
        let items = items
            .iter()
            .map(|item| {
                item.types()
                    .iter()
                    .filter_map(|kind| {
                        let data = item.dataForType(&kind)?;
                        Some((kind.to_string(), data.to_vec()))
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|item| !item.is_empty())
            .collect();
        ClipboardSnapshot(items)
    })
}

/// Clears the pasteboard and writes the snapshot's items back.
pub fn restore_clipboard_snapshot(snapshot: &ClipboardSnapshot) -> Result<(), CommandError> {
    autoreleasepool(|_| {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        if snapshot.0.is_empty() {
            return Ok(());
        }
        let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
            .0
            .iter()
            .map(|types| {
                let item = NSPasteboardItem::new();
                for (kind, bytes) in types {
                    item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(kind));
                }
                ProtocolObject::from_retained(item)
            })
            .collect();
        if pasteboard.writeObjects(&NSArray::from_retained_slice(&items)) {
            Ok(())
        } else {
            Err(CommandError::new(
                "clipboard_set_failed",
                "Could not restore the clipboard. macOS refused the write.",
            ))
        }
    })
}

/// Marked concealed (the nspasteboard.org convention) so clipboard managers
/// don't keep it.
pub fn set_private_clipboard_text(
    clipboard: &mut arboard::Clipboard,
    text: &str,
) -> Result<(), arboard::Error> {
    use arboard::SetExtApple;
    clipboard
        .set()
        .exclude_from_history()
        .text(text.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_is_split_without_breaking_surrogate_pairs() {
        let units: Vec<u16> = "ab😀cd".encode_utf16().collect();
        assert_eq!(units.len(), 6);
        let chunks = utf16_chunks(&units, 3);
        // The emoji's two units would straddle the first cut, so they move
        // to the second chunk together.
        assert_eq!(chunks, vec![&units[0..2], &units[2..5], &units[5..6]]);
        assert_eq!(chunks.concat(), units);
        assert!(utf16_chunks(&[], UNITS_PER_EVENT).is_empty());
        let long: Vec<u16> = "x".repeat(45).encode_utf16().collect();
        let sizes: Vec<usize> = utf16_chunks(&long, UNITS_PER_EVENT)
            .iter()
            .map(|chunk| chunk.len())
            .collect();
        assert_eq!(sizes, vec![20, 20, 5]);
    }

    #[test]
    fn windowless_apps_get_distinct_ids_and_ours_is_recognized() {
        // Negative ids stand for windowless apps and compare by process id.
        assert!(is_own_window(-(own_pid() as WindowId)));
        assert!(!is_own_window(-(own_pid() as WindowId) - 1));
        assert!(!is_own_window(0));
    }

    /// Reads the real window server. Run with the clipboard test below:
    /// `cargo test --lib platform::macos -- --ignored`.
    #[test]
    #[ignore]
    fn reads_the_app_in_front() {
        let front = foreground_window();
        let target = foreground_target();
        println!(
            "front window {front}, target {target:?}, accessibility {}",
            accessibility_granted()
        );
        // The test runner is not an app with windows, so whatever is in
        // front belongs to another process.
        if let Some(target) = target {
            assert_ne!(target.process_id, std::process::id());
            assert_ne!(target.window, 0);
            assert!(!is_own_window(target.window));
        }
    }

    fn textedit_pid() -> Option<i32> {
        let output = std::process::Command::new("pgrep")
            .args(["-x", "TextEdit"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()?
            .trim()
            .parse()
            .ok()
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// End to end with real keystrokes: opens a text file in TextEdit, pastes
    /// into it, types after the paste, then selects all and copies it back
    /// out through the selection capture's Command+C. Needs the Accessibility
    /// permission and quits TextEdit afterwards, so it is meant for the CI
    /// runner, which grants it.
    #[test]
    #[ignore]
    fn pastes_types_and_copies_in_textedit() {
        assert!(
            accessibility_granted(),
            "posting keystrokes needs the Accessibility permission"
        );
        let file = std::env::temp_dir().join(format!("scribe-paste-{}.txt", std::process::id()));
        std::fs::write(&file, "").unwrap();
        let opened = std::process::Command::new("open")
            .args(["-a", "TextEdit"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(opened.success());
        // The test thread has no main run loop to update NSWorkspace's
        // frontmost app, so TextEdit is found by its frontmost window.
        let mut pid = 0;
        wait_until("TextEdit to start", || {
            textedit_pid().map(|found| pid = found).is_some()
        });
        wait_until("the TextEdit window in front", || {
            app_windows()
                .first()
                .is_some_and(|window| window.owner == pid)
        });
        let window = front_window_id(pid);
        assert!(window > 0);
        assert!(!is_own_window(window));
        // Let the document window take keyboard focus.
        thread::sleep(Duration::from_millis(500));

        let original = save_clipboard_snapshot();
        let mut clipboard = open_clipboard().unwrap();
        set_private_clipboard_text(&mut clipboard, "Pasted by Scribe.").unwrap();
        send_paste_shortcut().unwrap();
        thread::sleep(Duration::from_millis(500));
        let typed = " Typed: naïve café 😀, long enough to need several events.";
        type_text(typed).unwrap();
        thread::sleep(Duration::from_millis(500));

        // Command+A (the ANSI A key is code 0), then the selection copy.
        let source = event_source().unwrap();
        assert!(post_key(&source, 0, CGEventFlags::MaskCommand, None));
        thread::sleep(Duration::from_millis(300));
        let before = clipboard_sequence_number();
        send_copy_shortcut(false).unwrap();
        wait_until("the copy", || clipboard_sequence_number() != before);
        thread::sleep(Duration::from_millis(100));
        let copied = clipboard.get_text().unwrap();

        restore_clipboard_snapshot(&original).unwrap();
        let _ = std::process::Command::new("pkill")
            .args(["-x", "TextEdit"])
            .status();
        let _ = std::fs::remove_file(&file);
        assert_eq!(copied, format!("Pasted by Scribe.{typed}"));
    }

    /// Writes to the real general pasteboard (and puts it back), so it runs
    /// only when asked.
    #[test]
    #[ignore]
    fn clipboard_snapshot_round_trips() {
        let original = save_clipboard_snapshot();
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        let item = NSPasteboardItem::new();
        item.setData_forType(
            &NSData::with_bytes(b"scribe snapshot"),
            &NSString::from_str("public.utf8-plain-text"),
        );
        item.setData_forType(
            &NSData::with_bytes(&[0x89, b'P', b'N', b'G']),
            &NSString::from_str("com.example.scribe-test"),
        );
        assert!(pasteboard.writeObjects(&NSArray::from_retained_slice(&[
            ProtocolObject::from_retained(item)
        ])));

        let before = clipboard_sequence_number();
        let snapshot = save_clipboard_snapshot();
        let mut clipboard = open_clipboard().unwrap();
        set_private_clipboard_text(&mut clipboard, "borrowed").unwrap();
        assert_ne!(clipboard_sequence_number(), before);
        assert_eq!(clipboard.get_text().unwrap(), "borrowed");

        restore_clipboard_snapshot(&snapshot).unwrap();
        assert_eq!(clipboard.get_text().unwrap(), "scribe snapshot");
        // macOS may add derived types of its own, but every saved one is back.
        let restored = save_clipboard_snapshot();
        assert_eq!(restored.0.len(), 1);
        for saved in &snapshot.0[0] {
            assert!(
                restored.0[0].contains(saved),
                "{} was not restored",
                saved.0
            );
        }

        restore_clipboard_snapshot(&original).unwrap();
    }
}
