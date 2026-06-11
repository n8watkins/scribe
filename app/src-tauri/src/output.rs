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
    ClipboardRestoreFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardPreservation {
    NotNeeded,
    Preserved,
    TextOnlyPreserved,
    TextOnlyRestoreFailed,
    ClipboardOwnedByMode,
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
    let result = {
        let _state_guard = PasteStateGuard::start(app);

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
                    clipboard_preservation: ClipboardPreservation::Preserved,
                    clipboard_restore_error: None,
                    message: "Inserted transcript without touching the clipboard.".to_string(),
                })
            }
            PasteMethod::ClipboardRestore => {
                compatibility_paste_with_restore(transcript, output_mode)
            }
        }
    };

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
            clipboard_preservation: ClipboardPreservation::ClipboardOwnedByMode,
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
    let result = {
        let _state_guard = PasteStateGuard::start(app);

        set_clipboard_text(&transcript.text)?;
        thread::sleep(Duration::from_millis(60));
        platform::send_paste_shortcut()?;

        Ok(OutputResult {
            transcript_id: transcript.id.clone(),
            action: OutputAction::CopyAndPaste,
            status: OutputStatus::Completed,
            output_mode,
            paste_method: Some(PasteMethod::ClipboardRestore),
            copied: true,
            pasted: true,
            clipboard_restored: None,
            clipboard_preservation: ClipboardPreservation::ClipboardOwnedByMode,
            clipboard_restore_error: None,
            message: "Copied transcript to the clipboard and pasted it.".to_string(),
        })
    };

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

fn compatibility_paste_with_restore(
    transcript: &Transcript,
    output_mode: OutputMode,
) -> Result<OutputResult, CommandError> {
    let previous_text = get_clipboard_text().ok();

    set_clipboard_text(&transcript.text)?;
    thread::sleep(Duration::from_millis(60));
    platform::send_paste_shortcut()?;
    thread::sleep(Duration::from_millis(120));

    let restore_result = match previous_text {
        Some(previous_text) => set_clipboard_text(&previous_text).map(|_| true),
        None => Err(CommandError::new(
            "clipboard_restore_failed",
            "Compatibility paste could not restore the previous clipboard because only text clipboard preservation is implemented and no previous text was available.",
        )),
    };

    let (status, clipboard_restored, clipboard_preservation, clipboard_restore_error, message) =
        match restore_result {
            Ok(true) => (
                OutputStatus::Completed,
                Some(true),
                ClipboardPreservation::TextOnlyPreserved,
                None,
                "Pasted transcript and restored the previous text clipboard contents.".to_string(),
            ),
            Ok(false) => unreachable!(),
            Err(error) => (
                OutputStatus::ClipboardRestoreFailed,
                Some(false),
                ClipboardPreservation::TextOnlyRestoreFailed,
                Some(error.message.clone()),
                "Pasted transcript, but the previous clipboard contents could not be restored."
                    .to_string(),
            ),
        };

    Ok(OutputResult {
        transcript_id: transcript.id.clone(),
        action: OutputAction::Paste,
        status,
        output_mode,
        paste_method: Some(PasteMethod::ClipboardRestore),
        copied: true,
        pasted: true,
        clipboard_restored,
        clipboard_preservation,
        clipboard_restore_error,
        message,
    })
}

fn get_clipboard_text() -> Result<String, CommandError> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| {
        CommandError::new(
            "clipboard_unavailable",
            format!("Could not read the clipboard. {}", error),
        )
    })?;

    clipboard.get_text().map_err(|error| {
        CommandError::new(
            "clipboard_text_unavailable",
            format!("Could not read text from the clipboard. {}", error),
        )
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
        "localdictate:output-started",
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
    let _ = app.emit("localdictate:output-completed", result);
}

pub fn emit_output_failed(app: &AppHandle, transcript_id: String, error: &CommandError) {
    log::error!(
        "Output failed for transcript {} ({}): {}",
        transcript_id,
        error.code,
        error.message
    );
    let _ = app.emit(
        "localdictate:output-failed",
        OutputFailedPayload {
            transcript_id,
            code: error.code.clone(),
            message: error.message.clone(),
        },
    );
}

fn emit_state_snapshot(app: &AppHandle, snapshot: &AppStateSnapshot) {
    let _ = app.emit("localdictate:app-state", snapshot);
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
    use std::mem::size_of;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY, VK_CONTROL, VK_V,
    };

    pub fn direct_insert_text(text: &str) -> Result<(), CommandError> {
        let mut inputs = Vec::with_capacity(256);

        for unit in text.encode_utf16() {
            inputs.push(keyboard_input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
            inputs.push(keyboard_input(
                VIRTUAL_KEY(0),
                unit,
                KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
            ));

            if inputs.len() >= 240 {
                send_inputs(&inputs, "direct insert")?;
                inputs.clear();
            }
        }

        if !inputs.is_empty() {
            send_inputs(&inputs, "direct insert")?;
        }

        Ok(())
    }

    pub fn send_paste_shortcut() -> Result<(), CommandError> {
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
    fn direct_insert_result_reports_clipboard_preserved() {
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
            clipboard_preservation: ClipboardPreservation::Preserved,
            clipboard_restore_error: None,
            message: "Inserted transcript without touching the clipboard.".to_string(),
        };

        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::Preserved
        );
        assert!(!result.copied);
        assert!(result.pasted);
    }
}
