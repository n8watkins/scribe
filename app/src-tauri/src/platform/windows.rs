//! Windows implementation of the platform layer: Win32 foreground-window
//! identity and focus, SendInput keystrokes, and the raw clipboard.

use super::{PasteTarget, WindowId};
use crate::error::CommandError;
use std::{ffi::c_void, mem::size_of, thread, time::Duration};
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow, GWL_EXSTYLE, GW_HWNDNEXT,
    WS_EX_TOOLWINDOW,
};

/// Every modifier whose held state would otherwise combine with injected
/// characters. Includes the generic (`VK_CONTROL`) and side-specific
/// (`VK_LCONTROL`/`VK_RCONTROL`) virtual keys so an explicit key-up is
/// emitted for whichever one Windows is actually tracking.
const MODIFIER_KEYS: [VIRTUAL_KEY; 11] = [
    VK_CONTROL,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_MENU,
    VK_LMENU,
    VK_RMENU,
    VK_SHIFT,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LWIN,
    VK_RWIN,
];

/// Window classes of console hosts, where a bare Ctrl+C with nothing selected
/// interrupts the running program instead of copying. Those get Ctrl+Shift+C.
const CONSOLE_WINDOW_CLASSES: [&str; 3] = [
    "ConsoleWindowClass",
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "mintty",
];

/// A full-fidelity copy of the clipboard: every captured format as raw bytes.
#[derive(Debug, Default)]
pub struct ClipboardSnapshot(Vec<(u32, Vec<u8>)>);

pub fn ensure_foreign_focus() -> Result<(), CommandError> {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() || !is_own_hwnd(foreground) {
        return Ok(());
    }

    // Walking down the Z-order from our foreground window reaches the app
    // the user was in before focusing Scribe. Refocusing it is allowed
    // because our process currently owns the foreground window.
    let mut candidate = foreground;
    while let Ok(next) = unsafe { GetWindow(candidate, GW_HWNDNEXT) } {
        candidate = next;

        if !is_paste_target(candidate) {
            continue;
        }

        if !unsafe { SetForegroundWindow(candidate) }.as_bool() {
            break;
        }

        // The target needs time to take keyboard focus before SendInput.
        thread::sleep(Duration::from_millis(100));
        return Ok(());
    }

    log::warn!("Focus guard found no other app window; skipping paste.");
    Err(CommandError::new(
        "paste_target_unavailable",
        "Could not find another app window to paste into. Click into the target app and try again.",
    ))
}

pub fn foreground_target() -> Option<PasteTarget> {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() || is_own_hwnd(foreground) {
        return None;
    }
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut process_id)) };
    Some(PasteTarget {
        window: foreground.0 as isize,
        process_id,
    })
}

/// The current foreground window as a plain HWND value.
pub fn foreground_window() -> WindowId {
    unsafe { GetForegroundWindow() }.0 as isize
}

/// True when the window belongs to this process (the main window or pill).
pub fn is_own_window(window: WindowId) -> bool {
    if window == 0 {
        return false;
    }
    is_own_hwnd(HWND(window as *mut _))
}

/// True when the window is a console host (see `CONSOLE_WINDOW_CLASSES`).
pub fn is_console_window(window: WindowId) -> bool {
    if window == 0 {
        return false;
    }
    let mut buffer = [0u16; 128];
    let length = unsafe { GetClassNameW(HWND(window as *mut _), &mut buffer) };
    if length <= 0 {
        return false;
    }
    let class_name = String::from_utf16_lossy(&buffer[..length as usize]);
    is_console_window_class(&class_name)
}

fn is_console_window_class(class_name: &str) -> bool {
    CONSOLE_WINDOW_CLASSES.contains(&class_name)
}

/// True when `process_id` runs at a higher integrity level than Scribe
/// (an app started as administrator while Scribe isn't). Windows drops
/// SendInput into such windows without an error. When either level can't
/// be read, assumes not, which keeps today's behavior.
pub fn runs_elevated_above_us(process_id: u32) -> bool {
    let Some(ours) = integrity_level(unsafe { GetCurrentProcess() }) else {
        return false;
    };
    let Ok(process) =
        (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) })
    else {
        return false;
    };
    let theirs = integrity_level(process);
    let _ = unsafe { CloseHandle(process) };
    theirs.is_some_and(|theirs| theirs > ours)
}

/// The mandatory integrity level RID of a process (for example 0x2000 for
/// medium, 0x3000 for high).
fn integrity_level(process: HANDLE) -> Option<u32> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;

    let mut needed = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed) };
    let mut buffer = vec![0u8; needed as usize];
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr() as *mut c_void),
            needed,
            &mut needed,
        )
    };
    let _ = unsafe { CloseHandle(token) };
    if read.is_err() || buffer.len() < size_of::<TOKEN_MANDATORY_LABEL>() {
        return None;
    }

    let label = unsafe { &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL) };
    let sid = label.Label.Sid;
    unsafe {
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, u32::from(count - 1)))
    }
}

fn is_own_hwnd(hwnd: HWND) -> bool {
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    process_id == unsafe { GetCurrentProcessId() }
}

fn is_paste_target(hwnd: HWND) -> bool {
    if is_own_hwnd(hwnd) || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    // Tool windows (tray flyouts, floating palettes) never take typed input.
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }

    // Untitled top-level windows are almost always invisible helpers.
    if unsafe { GetWindowTextLengthW(hwnd) } == 0 {
        return false;
    }

    // Suspended UWP apps stay "visible" in the Z-order but are cloaked by
    // DWM; focusing one would swallow the paste. A failed query means the
    // window predates cloaking and is safe to use.
    let mut cloaked = 0u32;
    let cloak_query = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        )
    };
    !(cloak_query.is_ok() && cloaked != 0)
}

/// Hotkey-triggered pastes can start while the user still physically
/// holds the chord (e.g. Ctrl+Alt+V). Injected keystrokes then combine
/// with the held modifiers in the target app — terminals turn a typed
/// "," into Ctrl+Alt+"," and open their settings JSON — so wait for
/// every modifier to come back up before sending input.
fn wait_for_modifier_release() {
    let deadline = std::time::Instant::now() + Duration::from_millis(1_500);
    loop {
        let held = MODIFIER_KEYS
            .iter()
            .any(|vk| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16) & 0x8000 != 0);
        if !held {
            return;
        }
        if std::time::Instant::now() >= deadline {
            log::warn!("Pasting with modifier keys still held after 1.5 s");
            return;
        }
        thread::sleep(Duration::from_millis(15));
    }
}

/// Defensive backstop to `wait_for_modifier_release`: synthesize an explicit
/// key-up for every modifier still physically down. Even if the user keeps
/// holding the paste chord past the wait deadline, this guarantees the
/// modifiers are logically up before any character is injected, so a typed
/// "," can never be seen as Ctrl+Alt+"," (which scrambles Windows Terminal
/// and opens settings JSON). Sent as one burst, immediately before the text.
///
/// `error_code` and `release_context` name the failure in the caller's terms
/// (a paste or a selection copy).
fn release_held_modifiers(error_code: &str, release_context: &str) -> Result<(), CommandError> {
    let releases: Vec<INPUT> = MODIFIER_KEYS
        .iter()
        .filter(|vk| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16) & 0x8000 != 0)
        .map(|vk| keyboard_input(*vk, 0, KEYEVENTF_KEYUP))
        .collect();

    if releases.is_empty() {
        return Ok(());
    }

    send_inputs(&releases, error_code, release_context)
}

/// Maximum INPUT events per `SendInput` call. The whole transcript is sent
/// in a single call so it lands as one atomic insert (not a visible crawl);
/// only pathologically long text is split, and then into large chunks so it
/// still appears as at most a couple of bursts. Each UTF-16 unit is two
/// INPUTs (key-down + key-up), so this caps a chunk at ~2000 characters.
const MAX_INPUTS_PER_BURST: usize = 4000;

pub fn type_text(text: &str) -> Result<(), CommandError> {
    wait_for_modifier_release();
    // Backstop: force-release anything still held so injected characters
    // cannot combine into shortcuts in the target app.
    release_held_modifiers(PASTE_FAILED, "modifier release input")?;

    // Build the INPUTs for the entire transcript up front so the common case
    // is a single SendInput call (one atomic insert).
    let mut inputs: Vec<INPUT> = Vec::with_capacity(text.len().saturating_mul(2));
    for unit in text.encode_utf16() {
        inputs.push(keyboard_input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
        inputs.push(keyboard_input(
            VIRTUAL_KEY(0),
            unit,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }

    if inputs.is_empty() {
        return Ok(());
    }

    // One call for normal-length transcripts; large chunks only if huge.
    for chunk in inputs.chunks(MAX_INPUTS_PER_BURST) {
        send_inputs(chunk, PASTE_FAILED, "direct insert input")?;
    }

    Ok(())
}

/// Closes the clipboard on drop so every early return in the snapshot/
/// restore routines still releases it. The OS clipboard is a single global
/// lock — leaking it open would wedge copy/paste for the whole desktop.
struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // Best effort: nothing useful to do if the close itself fails.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Opens the clipboard, retrying a few times because another app may briefly
/// hold it (Win32 allows only one owner at a time). Returns a guard that
/// closes it on drop.
fn open_clipboard_with_retry() -> Result<ClipboardGuard, CommandError> {
    const ATTEMPTS: u32 = 6;
    for attempt in 0..ATTEMPTS {
        // HWND::default() (null) ties the clipboard to the current task
        // without associating a specific window, which is fine here.
        if unsafe { OpenClipboard(HWND::default()) }.is_ok() {
            return Ok(ClipboardGuard);
        }
        if attempt + 1 < ATTEMPTS {
            thread::sleep(Duration::from_millis(20));
        }
    }
    Err(CommandError::new(
        "clipboard_unavailable",
        "Could not open the clipboard; another app may be holding it.",
    ))
}

/// Clipboard formats backed by GDI/handle objects rather than HGLOBAL
/// memory. `GlobalLock`/`GlobalSize` are invalid on these handles, so they
/// cannot be byte-captured and are skipped. Images and files are unaffected
/// because they also publish GlobalAlloc-backed variants (CF_DIB/CF_DIBV5
/// for images, CF_HDROP for files) that ARE captured here.
fn is_non_hglobal_format(format: u32) -> bool {
    // CF_BITMAP, CF_METAFILEPICT, CF_PALETTE, CF_ENHMETAFILE, and the
    // owner-display / private display variants.
    matches!(
        format,
        2      // CF_BITMAP
            | 3  // CF_METAFILEPICT
            | 9  // CF_PALETTE
            | 14 // CF_ENHMETAFILE
            | 0x80 // CF_OWNERDISPLAY
            | 0x82 // CF_DSPBITMAP
            | 0x83 // CF_DSPMETAFILEPICT
            | 0x8E // CF_DSPENHMETAFILE
    )
}

/// Snapshots EVERY currently-available clipboard format as raw bytes so the
/// full payload (text, image, files, ...) can be restored after a borrow.
///
/// For each enumerated format: read its data handle; null handles are
/// delayed-render placeholders (the data isn't materialized) and are
/// skipped; raw GDI/handle formats (see `is_non_hglobal_format`) are skipped
/// because they aren't HGLOBAL memory; everything else is treated as HGLOBAL
/// and its bytes are copied out via GlobalLock/GlobalSize.
///
/// Returns an empty vec on any failure (e.g. the clipboard couldn't be
/// opened) so the caller can still paste; the worst case is that the
/// previous clipboard is cleared rather than restored.
pub fn save_clipboard_snapshot() -> ClipboardSnapshot {
    let _guard = match open_clipboard_with_retry() {
        Ok(guard) => guard,
        Err(error) => {
            log::warn!("Clipboard snapshot skipped: {}", error.message);
            return ClipboardSnapshot::default();
        }
    };

    let mut snapshot: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut format = unsafe { EnumClipboardFormats(0) };
    while format != 0 {
        if !is_non_hglobal_format(format) {
            // A null/invalid handle is GetClipboardData's Err case here and
            // signals delayed rendering — there is nothing to copy.
            if let Ok(handle) = unsafe { GetClipboardData(format) } {
                let hglobal = HGLOBAL(handle.0);
                let ptr = unsafe { GlobalLock(hglobal) };
                if !ptr.is_null() {
                    let size = unsafe { GlobalSize(hglobal) };
                    if size > 0 {
                        let mut bytes = vec![0u8; size];
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                ptr as *const u8,
                                bytes.as_mut_ptr(),
                                size,
                            );
                        }
                        snapshot.push((format, bytes));
                    }
                    // GlobalUnlock returns Err once the lock count hits 0,
                    // which is the normal success case here, so ignore it.
                    let _ = unsafe { GlobalUnlock(hglobal) };
                }
            }
        }

        format = unsafe { EnumClipboardFormats(format) };
    }

    ClipboardSnapshot(snapshot)
}

/// Restores a snapshot produced by `save_clipboard_snapshot`: clears the
/// clipboard, then republishes each captured format from a freshly allocated
/// HGLOBAL.
///
/// Ownership: after a SUCCESSFUL `SetClipboardData` the system owns the
/// HGLOBAL and will free it, so it must NOT be freed here. The block is only
/// freed when `SetClipboardData` fails (data never handed off).
pub fn restore_clipboard_snapshot(snapshot: &ClipboardSnapshot) -> Result<(), CommandError> {
    let _guard = open_clipboard_with_retry()?;

    // Take ownership of the clipboard and drop the transcript that is on it
    // before republishing the saved formats.
    unsafe { EmptyClipboard() }.map_err(|error| {
        CommandError::new(
            "clipboard_clear_failed",
            format!("Could not clear the clipboard before restore. {}", error),
        )
    })?;

    for (format, bytes) in &snapshot.0 {
        // Allocate at least one byte so GlobalLock is always valid, even for
        // a (rare) zero-length format; only the real bytes are copied in.
        let alloc_size = bytes.len().max(1);
        let hglobal = unsafe { GlobalAlloc(GMEM_MOVEABLE, alloc_size) }.map_err(|error| {
            CommandError::new(
                "clipboard_alloc_failed",
                format!(
                    "Could not allocate memory to restore the clipboard. {}",
                    error
                ),
            )
        })?;

        let ptr = unsafe { GlobalLock(hglobal) };
        if ptr.is_null() {
            // Hand the just-allocated (never-published) block back to the OS.
            let _ = unsafe { GlobalFree(hglobal) };
            return Err(CommandError::new(
                "clipboard_lock_failed",
                "Could not lock memory to restore the clipboard.",
            ));
        }

        if !bytes.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            }
        }
        let _ = unsafe { GlobalUnlock(hglobal) };

        // SetClipboardData takes ownership of the HGLOBAL on success.
        match unsafe { SetClipboardData(*format, HANDLE(hglobal.0)) } {
            Ok(_) => {}
            Err(error) => {
                // Data was not handed off, so we still own the block.
                let _ = unsafe { GlobalFree(hglobal) };
                return Err(CommandError::new(
                    "clipboard_set_failed",
                    format!("Could not restore a clipboard format. {}", error),
                ));
            }
        }
    }

    // Putting the user's own clipboard back must not add a second copy of
    // it to Clipboard History or upload it again.
    mark_clipboard_private();

    Ok(())
}

/// Adds the formats that keep the current clipboard item out of Clipboard
/// History, the cloud clipboard and clipboard managers. Needs the clipboard
/// open. Best effort: a failure only means Windows may keep a copy.
fn mark_clipboard_private() {
    let markers = [
        w!("ExcludeClipboardContentFromMonitorProcessing"),
        w!("CanIncludeInClipboardHistory"),
        w!("CanUploadToCloudClipboard"),
    ];
    for name in markers {
        let format = unsafe { RegisterClipboardFormatW(name) };
        if format == 0 {
            log::warn!("Could not register a clipboard privacy format");
            continue;
        }
        // Each marker is a DWORD of 0 ("no").
        let Ok(hglobal) = (unsafe { GlobalAlloc(GMEM_MOVEABLE, size_of::<u32>()) }) else {
            continue;
        };
        let ptr = unsafe { GlobalLock(hglobal) };
        if ptr.is_null() {
            let _ = unsafe { GlobalFree(hglobal) };
            continue;
        }
        unsafe { std::ptr::write_unaligned(ptr as *mut u32, 0) };
        let _ = unsafe { GlobalUnlock(hglobal) };
        if unsafe { SetClipboardData(format, HANDLE(hglobal.0)) }.is_err() {
            let _ = unsafe { GlobalFree(hglobal) };
            log::warn!("Could not mark the restored clipboard as private");
        }
    }
}

pub fn send_paste_shortcut() -> Result<(), CommandError> {
    // Paste fires on key-PRESS, not on release: rather than blocking until
    // the user lets go of the whole paste chord (which made Ctrl+Alt+Shift+V
    // feel like it only worked once every key was released), synthesize
    // key-ups for any physically-held modifier so a held Ctrl/Alt/Shift/Win
    // can't combine with the synthetic Ctrl+V below and scramble the target
    // (e.g. Windows Terminal). The OS treats the modifiers as up until the
    // held key's next auto-repeat (~250 ms away), so the Ctrl+V lands clean.
    release_held_modifiers(PASTE_FAILED, "modifier release input")?;

    let inputs = [
        keyboard_input(VK_CONTROL, 0, Default::default()),
        keyboard_input(VK_V, 0, Default::default()),
        keyboard_input(VK_V, 0, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];

    send_inputs(&inputs, PASTE_FAILED, "paste shortcut input")
}

/// Synthesizes the copy shortcut for the current selection: Ctrl+C, or
/// Ctrl+Shift+C in a console, where a bare Ctrl+C interrupts the running
/// program when nothing is selected. Releases held modifiers first so the
/// chord that triggered the transform doesn't fuse with the injected C.
pub fn send_copy_shortcut(console: bool) -> Result<(), CommandError> {
    release_held_modifiers(SELECTION_COPY_FAILED, "modifier release")?;

    let inputs: Vec<INPUT> = if console {
        vec![
            keyboard_input(VK_CONTROL, 0, Default::default()),
            keyboard_input(VK_SHIFT, 0, Default::default()),
            keyboard_input(VK_C, 0, Default::default()),
            keyboard_input(VK_C, 0, KEYEVENTF_KEYUP),
            keyboard_input(VK_SHIFT, 0, KEYEVENTF_KEYUP),
            keyboard_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
        ]
    } else {
        vec![
            keyboard_input(VK_CONTROL, 0, Default::default()),
            keyboard_input(VK_C, 0, Default::default()),
            keyboard_input(VK_C, 0, KEYEVENTF_KEYUP),
            keyboard_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
        ]
    };
    send_inputs(&inputs, SELECTION_COPY_FAILED, "copy shortcut")
}

/// Whether `send_copy_shortcut` and the clipboard snapshot work here, so the
/// Transform Selection hotkey can copy the selection out of another app.
pub fn can_capture_selection() -> bool {
    true
}

/// The clipboard's change counter; it moves on every write by any app.
pub fn clipboard_sequence_number() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Writes `text` with the formats that keep it out of Clipboard History
/// (Win+V), the cloud clipboard and clipboard managers.
pub fn set_private_clipboard_text(
    clipboard: &mut arboard::Clipboard,
    text: &str,
) -> Result<(), arboard::Error> {
    use arboard::SetExtWindows;
    clipboard
        .set()
        .exclude_from_monitoring()
        .exclude_from_cloud()
        .exclude_from_history()
        .text(text.to_string())
}

/// Error codes for failed keystroke injection, per caller.
const PASTE_FAILED: &str = "paste_failed";
const SELECTION_COPY_FAILED: &str = "selection_copy_failed";

fn keyboard_input(
    virtual_key: VIRTUAL_KEY,
    scan_code: u16,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_inputs(inputs: &[INPUT], error_code: &str, context: &str) -> Result<(), CommandError> {
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };

    if sent != inputs.len() as u32 {
        return Err(CommandError::new(
            error_code,
            format!(
                "Could not send {} to the focused app. Sent {} of {} events.",
                context,
                sent,
                inputs.len()
            ),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_hosts_are_recognized_by_window_class() {
        assert!(is_console_window_class("ConsoleWindowClass"));
        assert!(is_console_window_class("CASCADIA_HOSTING_WINDOW_CLASS"));
        assert!(is_console_window_class("mintty"));
        assert!(!is_console_window_class("Chrome_WidgetWin_1"));
        assert!(!is_console_window_class("Notepad"));
        assert!(!is_console_window_class(""));
    }
}
