use std::{thread, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    app_state::{AppEvent, AppStateSnapshot, AppStatus},
    commands::BackendState,
    error::CommandError,
    settings::{AppSettings, OutputMode, PasteMethod},
    transcript::Transcript,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputAction {
    SaveOnly,
    CopyClipboard,
    Paste,
    CopyAndPaste,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStatus {
    Completed,
    /// Kept for serde back-compat with previously emitted/persisted payloads.
    /// No output path produces it anymore: clipboard paste deliberately leaves
    /// the transcript on the clipboard, so there is no restore step to fail.
    ClipboardRestoreFailed,
}

/// Honest description of what each output path did to the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardPreservation {
    /// The system clipboard was never read or written (the default
    /// clipboard-free insert).
    Untouched,
    /// The transcript was placed on the system clipboard and left there
    /// (clipboard paste, copy, and copy-and-paste).
    ReplacedWithTranscript,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputResult {
    pub transcript_id: String,
    pub action: OutputAction,
    pub status: OutputStatus,
    pub output_mode: OutputMode,
    pub paste_method: Option<PasteMethod>,
    pub copied: bool,
    pub pasted: bool,
    pub clipboard_restored: Option<bool>,
    pub clipboard_preservation: ClipboardPreservation,
    pub clipboard_restore_error: Option<String>,
    pub message: String,
}

pub fn handle_transcription_output(
    app: &AppHandle,
    transcript: &Transcript,
    settings: &AppSettings,
) -> Result<Option<OutputResult>, CommandError> {
    let result = match settings.output_mode {
        OutputMode::SaveOnly => return Ok(None),
        OutputMode::AutoPaste => paste_transcript_text(
            app,
            transcript,
            OutputMode::AutoPaste,
            settings.paste_method.clone(),
            true,
        ),
        OutputMode::CopyClipboard => {
            copy_transcript_text(app, transcript, OutputMode::CopyClipboard, true)
        }
        OutputMode::CopyAndPaste => copy_and_paste_transcript_text(app, transcript, true),
    };

    result.map(Some)
}

pub fn paste_last_transcript(app: &AppHandle) -> Result<OutputResult, CommandError> {
    let state = app.state::<BackendState>();
    let (transcript, paste_method) = {
        let db = state.db()?;
        let transcript = db.get_last_transcript()?.ok_or_else(no_last_transcript)?;
        let settings = db.get_settings()?;
        (transcript, settings.paste_method)
    };

    paste_transcript_text(app, &transcript, OutputMode::AutoPaste, paste_method, true)
}

pub fn copy_last_transcript(app: &AppHandle) -> Result<OutputResult, CommandError> {
    let state = app.state::<BackendState>();
    let transcript = state
        .db()?
        .get_last_transcript()?
        .ok_or_else(no_last_transcript)?;

    copy_transcript_text(app, &transcript, OutputMode::CopyClipboard, true)
}

pub fn paste_transcript(app: &AppHandle, id: &str) -> Result<OutputResult, CommandError> {
    let state = app.state::<BackendState>();
    let (transcript, paste_method) = {
        let db = state.db()?;
        let transcript = db
            .get_transcript_by_id(id)?
            .ok_or_else(|| transcript_not_found(id))?;
        let settings = db.get_settings()?;
        (transcript, settings.paste_method)
    };

    paste_transcript_text(app, &transcript, OutputMode::AutoPaste, paste_method, true)
}

pub fn copy_transcript(app: &AppHandle, id: &str) -> Result<OutputResult, CommandError> {
    let state = app.state::<BackendState>();
    let transcript = state
        .db()?
        .get_transcript_by_id(id)?
        .ok_or_else(|| transcript_not_found(id))?;

    copy_transcript_text(app, &transcript, OutputMode::CopyClipboard, true)
}

fn paste_transcript_text(
    app: &AppHandle,
    transcript: &Transcript,
    output_mode: OutputMode,
    paste_method: PasteMethod,
    emit_events: bool,
) -> Result<OutputResult, CommandError> {
    emit_output_started(
        app,
        transcript,
        OutputAction::Paste,
        &output_mode,
        Some(&paste_method),
    );
    let result = (|| {
        let _state_guard = PasteStateGuard::start(app);

        // The paste hotkey can fire while one of our own windows is focused;
        // the transcript must never be typed into Scribe itself.
        platform::ensure_foreign_focus()?;

        match paste_method {
            PasteMethod::DirectInsert => {
                platform::direct_insert_text(&transcript.text)?;
                Ok(OutputResult {
                    transcript_id: transcript.id.clone(),
                    action: OutputAction::Paste,
                    status: OutputStatus::Completed,
                    output_mode,
                    paste_method: Some(PasteMethod::DirectInsert),
                    copied: false,
                    pasted: true,
                    clipboard_restored: None,
                    clipboard_preservation: ClipboardPreservation::Untouched,
                    clipboard_restore_error: None,
                    message: "Inserted transcript. Clipboard untouched.".to_string(),
                })
            }
            PasteMethod::ClipboardPaste => clipboard_paste(transcript, output_mode),
        }
    })();

    match result {
        Ok(result) => {
            if emit_events {
                emit_output_completed(app, &result);
            }
            Ok(result)
        }
        Err(error) => {
            if emit_events {
                emit_output_failed(app, transcript.id.clone(), &error);
            }
            Err(error)
        }
    }
}

fn copy_transcript_text(
    app: &AppHandle,
    transcript: &Transcript,
    output_mode: OutputMode,
    emit_events: bool,
) -> Result<OutputResult, CommandError> {
    emit_output_started(
        app,
        transcript,
        OutputAction::CopyClipboard,
        &output_mode,
        None,
    );
    let result = (|| {
        set_clipboard_text(&transcript.text)?;

        Ok(OutputResult {
            transcript_id: transcript.id.clone(),
            action: OutputAction::CopyClipboard,
            status: OutputStatus::Completed,
            output_mode,
            paste_method: None,
            copied: true,
            pasted: false,
            clipboard_restored: None,
            clipboard_preservation: ClipboardPreservation::ReplacedWithTranscript,
            clipboard_restore_error: None,
            message: "Copied transcript to the clipboard.".to_string(),
        })
    })();

    match result {
        Ok(result) => {
            if emit_events {
                emit_output_completed(app, &result);
            }
            Ok(result)
        }
        Err(error) => {
            if emit_events {
                emit_output_failed(app, transcript.id.clone(), &error);
            }
            Err(error)
        }
    }
}

fn copy_and_paste_transcript_text(
    app: &AppHandle,
    transcript: &Transcript,
    emit_events: bool,
) -> Result<OutputResult, CommandError> {
    let output_mode = OutputMode::CopyAndPaste;
    emit_output_started(
        app,
        transcript,
        OutputAction::CopyAndPaste,
        &output_mode,
        None,
    );
    let result = (|| {
        let _state_guard = PasteStateGuard::start(app);

        set_clipboard_text(&transcript.text)?;
        thread::sleep(Duration::from_millis(60));
        // The paste hotkey can fire while one of our own windows is focused;
        // the transcript must never be typed into Scribe itself. The
        // clipboard copy above is kept either way so a manual paste still works.
        platform::ensure_foreign_focus()?;
        platform::send_paste_shortcut()?;

        Ok(OutputResult {
            transcript_id: transcript.id.clone(),
            action: OutputAction::CopyAndPaste,
            status: OutputStatus::Completed,
            output_mode,
            paste_method: Some(PasteMethod::ClipboardPaste),
            copied: true,
            pasted: true,
            clipboard_restored: None,
            clipboard_preservation: ClipboardPreservation::ReplacedWithTranscript,
            clipboard_restore_error: None,
            message: "Copied transcript to the clipboard and pasted it.".to_string(),
        })
    })();

    match result {
        Ok(result) => {
            if emit_events {
                emit_output_completed(app, &result);
            }
            Ok(result)
        }
        Err(error) => {
            if emit_events {
                emit_output_failed(app, transcript.id.clone(), &error);
            }
            Err(error)
        }
    }
}

/// Opt-in "Clipboard paste": put the transcript on the system clipboard and
/// send Ctrl+V. The transcript is deliberately left on the clipboard — there
/// is no save/restore (the owner finds borrow-and-restore hacky). The honest
/// trade-off is that the user's previous clipboard contents are replaced.
fn clipboard_paste(
    transcript: &Transcript,
    output_mode: OutputMode,
) -> Result<OutputResult, CommandError> {
    set_clipboard_text(&transcript.text)?;
    thread::sleep(Duration::from_millis(60));
    platform::send_paste_shortcut()?;

    Ok(OutputResult {
        transcript_id: transcript.id.clone(),
        action: OutputAction::Paste,
        status: OutputStatus::Completed,
        output_mode,
        paste_method: Some(PasteMethod::ClipboardPaste),
        copied: true,
        pasted: true,
        clipboard_restored: None,
        clipboard_preservation: ClipboardPreservation::ReplacedWithTranscript,
        clipboard_restore_error: None,
        message: "Pasted transcript. Transcript left on the clipboard.".to_string(),
    })
}

fn set_clipboard_text(text: &str) -> Result<(), CommandError> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| {
        CommandError::new(
            "clipboard_unavailable",
            format!("Could not access the clipboard. {}", error),
        )
    })?;

    clipboard.set_text(text.to_string()).map_err(|error| {
        CommandError::new(
            "clipboard_write_failed",
            format!(
                "Could not write transcript text to the clipboard. {}",
                error
            ),
        )
    })
}

fn emit_output_started(
    app: &AppHandle,
    transcript: &Transcript,
    action: OutputAction,
    output_mode: &OutputMode,
    paste_method: Option<&PasteMethod>,
) {
    let _ = app.emit(
        "scribe:output-started",
        OutputStartedPayload {
            transcript_id: transcript.id.clone(),
            action,
            output_mode: output_mode.clone(),
            paste_method: paste_method.cloned(),
        },
    );
}

fn emit_output_completed(app: &AppHandle, result: &OutputResult) {
    log::info!(
        "Output {:?} {:?} for transcript {}: {}",
        result.action,
        result.status,
        result.transcript_id,
        result.message
    );
    let _ = app.emit("scribe:output-completed", result);
}

pub fn emit_output_failed(app: &AppHandle, transcript_id: String, error: &CommandError) {
    log::error!(
        "Output failed for transcript {} ({}): {}",
        transcript_id,
        error.code,
        error.message
    );
    let _ = app.emit(
        "scribe:output-failed",
        OutputFailedPayload {
            transcript_id,
            code: error.code.clone(),
            message: error.message.clone(),
        },
    );
}

fn emit_state_snapshot(app: &AppHandle, snapshot: &AppStateSnapshot) {
    let _ = app.emit("scribe:app-state", snapshot);
}

fn no_last_transcript() -> CommandError {
    CommandError::new(
        "last_transcript_missing",
        "No transcript is available in the Last Transcript Buffer.",
    )
}

fn transcript_not_found(id: &str) -> CommandError {
    CommandError::new(
        "transcript_not_found",
        format!("Could not find transcript '{}'.", id),
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutputStartedPayload {
    transcript_id: String,
    action: OutputAction,
    output_mode: OutputMode,
    paste_method: Option<PasteMethod>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutputFailedPayload {
    transcript_id: String,
    code: String,
    message: String,
}

struct PasteStateGuard<'a> {
    app: &'a AppHandle,
    active: bool,
}

impl<'a> PasteStateGuard<'a> {
    fn start(app: &'a AppHandle) -> Self {
        let state = app.state::<BackendState>();
        let active = state
            .app_state()
            .map(|state| state.snapshot().status == AppStatus::Ready)
            .unwrap_or(false);

        if active {
            if let Ok(snapshot) = state.transition_app_state(AppEvent::StartPasting) {
                emit_state_snapshot(app, &snapshot);
            }
        }

        Self { app, active }
    }
}

impl Drop for PasteStateGuard<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let state = self.app.state::<BackendState>();
        if let Ok(snapshot) = state.transition_app_state(AppEvent::PasteCompleted) {
            emit_state_snapshot(self.app, &snapshot);
        }
    }
}

#[cfg(windows)]
mod platform {
    use crate::error::CommandError;
    use std::{mem::size_of, thread, time::Duration};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
        VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V,
    };

    /// Every modifier whose held state would otherwise combine with injected
    /// characters. Includes the generic (`VK_CONTROL`) and side-specific
    /// (`VK_LCONTROL`/`VK_RCONTROL`) virtual keys so an explicit key-up is
    /// emitted for whichever one Windows is actually tracking.
    const MODIFIER_KEYS: [VIRTUAL_KEY; 11] = [
        VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU, VK_SHIFT, VK_LSHIFT,
        VK_RSHIFT, VK_LWIN, VK_RWIN,
    ];
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW,
        GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow, GWL_EXSTYLE, GW_HWNDNEXT,
        WS_EX_TOOLWINDOW,
    };

    pub fn ensure_foreign_focus() -> Result<(), CommandError> {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_invalid() || !is_own_window(foreground) {
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

    fn is_own_window(hwnd: HWND) -> bool {
        let mut process_id = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
        process_id == unsafe { GetCurrentProcessId() }
    }

    fn is_paste_target(hwnd: HWND) -> bool {
        if is_own_window(hwnd) || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
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
    fn release_held_modifiers() -> Result<(), CommandError> {
        let releases: Vec<INPUT> = MODIFIER_KEYS
            .iter()
            .filter(|vk| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16) & 0x8000 != 0)
            .map(|vk| keyboard_input(*vk, 0, KEYEVENTF_KEYUP))
            .collect();

        if releases.is_empty() {
            return Ok(());
        }

        send_inputs(&releases, "modifier release")
    }

    /// Maximum INPUT events per `SendInput` call. The whole transcript is sent
    /// in a single call so it lands as one atomic insert (not a visible crawl);
    /// only pathologically long text is split, and then into large chunks so it
    /// still appears as at most a couple of bursts. Each UTF-16 unit is two
    /// INPUTs (key-down + key-up), so this caps a chunk at ~2000 characters.
    const MAX_INPUTS_PER_BURST: usize = 4000;

    pub fn direct_insert_text(text: &str) -> Result<(), CommandError> {
        wait_for_modifier_release();
        // Backstop: force-release anything still held so injected characters
        // cannot combine into shortcuts in the target app.
        release_held_modifiers()?;

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
            send_inputs(chunk, "direct insert")?;
        }

        Ok(())
    }

    pub fn send_paste_shortcut() -> Result<(), CommandError> {
        wait_for_modifier_release();

        let inputs = [
            keyboard_input(VK_CONTROL, 0, Default::default()),
            keyboard_input(VK_V, 0, Default::default()),
            keyboard_input(VK_V, 0, KEYEVENTF_KEYUP),
            keyboard_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
        ];

        send_inputs(&inputs, "paste shortcut")
    }

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

    fn send_inputs(inputs: &[INPUT], context: &str) -> Result<(), CommandError> {
        let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };

        if sent != inputs.len() as u32 {
            return Err(CommandError::new(
                "paste_failed",
                format!(
                    "Could not send {} input to the focused app. Sent {} of {} events.",
                    context,
                    sent,
                    inputs.len()
                ),
            ));
        }

        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::error::CommandError;

    pub fn ensure_foreign_focus() -> Result<(), CommandError> {
        Ok(())
    }

    pub fn direct_insert_text(_text: &str) -> Result<(), CommandError> {
        Err(CommandError::new(
            "direct_insert_unsupported",
            "Direct Insert is currently implemented for Windows only.",
        ))
    }

    pub fn send_paste_shortcut() -> Result<(), CommandError> {
        Err(CommandError::new(
            "paste_unsupported",
            "Programmatic paste is currently implemented for Windows only.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_insert_result_reports_clipboard_untouched() {
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();

        let result = OutputResult {
            transcript_id: transcript.id,
            action: OutputAction::Paste,
            status: OutputStatus::Completed,
            output_mode: OutputMode::AutoPaste,
            paste_method: Some(PasteMethod::DirectInsert),
            copied: false,
            pasted: true,
            clipboard_restored: None,
            clipboard_preservation: ClipboardPreservation::Untouched,
            clipboard_restore_error: None,
            message: "Inserted transcript. Clipboard untouched.".to_string(),
        };

        // The default insert path must never report touching the clipboard.
        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::Untouched
        );
        assert!(!result.copied);
        assert!(result.pasted);
    }

    #[test]
    fn clipboard_paste_result_reports_transcript_left_on_clipboard() {
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();

        // Mirrors what `clipboard_paste` produces: the transcript is copied and
        // deliberately left on the clipboard (no restore).
        let result = OutputResult {
            transcript_id: transcript.id,
            action: OutputAction::Paste,
            status: OutputStatus::Completed,
            output_mode: OutputMode::AutoPaste,
            paste_method: Some(PasteMethod::ClipboardPaste),
            copied: true,
            pasted: true,
            clipboard_restored: None,
            clipboard_preservation: ClipboardPreservation::ReplacedWithTranscript,
            clipboard_restore_error: None,
            message: "Pasted transcript. Transcript left on the clipboard.".to_string(),
        };

        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::ReplacedWithTranscript
        );
        assert!(result.copied);
        assert!(result.pasted);
        assert!(result.clipboard_restored.is_none());
        assert!(result.clipboard_restore_error.is_none());
    }
}
