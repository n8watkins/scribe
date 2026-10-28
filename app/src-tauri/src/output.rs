use std::{thread, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

pub use crate::platform::PasteTarget;
use crate::{
    app_state::{AppEvent, AppStatus},
    commands::BackendState,
    dictation_state::emit_state_snapshot,
    error::CommandError,
    platform,
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
    /// The transcript was pasted, but the user's previous clipboard text could
    /// not be put back afterwards (the borrow-and-restore default). The paste
    /// itself succeeded; only the restore step failed.
    ClipboardRestoreFailed,
    /// Scribe did not paste, because the window in front was not the one the
    /// dictation started in, or Windows would have blocked the paste. The
    /// text was left on the clipboard for the user to paste themselves.
    PasteSkipped,
}

/// Why Scribe chose to leave the text on the clipboard instead of pasting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteBlock {
    /// The user moved to another app while the dictation was transcribing.
    FocusChanged,
    /// The app in front runs as administrator. Windows silently drops input
    /// sent to it from a normal process (UIPI), so a paste would vanish.
    Elevated,
    /// The system can't send keystrokes to the app in front (a Wayland app
    /// on Linux). Holds the one-sentence reason.
    Unavailable(&'static str),
}

impl PasteBlock {
    fn message(self) -> String {
        match self {
            PasteBlock::FocusChanged => {
                "You switched windows, so Scribe didn't paste. Copied. Press Ctrl+V to paste."
                    .to_string()
            }
            PasteBlock::Elevated => {
                "That window runs as administrator, which blocks Scribe from pasting. Copied. Press Ctrl+V to paste."
                    .to_string()
            }
            PasteBlock::Unavailable(reason) => {
                format!("{reason} Copied. Press Ctrl+V to paste.")
            }
        }
    }
}

/// Decides whether pasting into the window in front is safe. `pinned` is the
/// window the dictation started in (`None` when Scribe itself had focus, so
/// there is nothing to hold to), `current` is the window in front now
/// (`None` when it can't be read), and `current_elevated` says whether it runs
/// at a higher integrity level than Scribe. Windows of the same app count as
/// the same target, so switching tabs or panes inside one app still pastes.
pub fn paste_block_reason(
    pinned: Option<PasteTarget>,
    current: Option<PasteTarget>,
    current_elevated: bool,
) -> Option<PasteBlock> {
    if current_elevated {
        return Some(PasteBlock::Elevated);
    }
    match (pinned, current) {
        (None, _) => None,
        // A process id of 0 means unknown and never matches.
        (Some(pinned), Some(current))
            if pinned.window == current.window
                || (pinned.process_id != 0 && pinned.process_id == current.process_id) =>
        {
            None
        }
        (Some(_), _) => Some(PasteBlock::FocusChanged),
    }
}

/// The window in front right now, or `None` when it is one of Scribe's own
/// windows or there is none. Recorded when a dictation starts.
pub fn current_paste_target() -> Option<PasteTarget> {
    platform::foreground_target()
}

/// Whether to skip the paste, checked after the focus guard so it sees the
/// window the keystrokes would really go to.
fn paste_block(pinned: Option<PasteTarget>) -> Option<PasteBlock> {
    if let Some(reason) = platform::paste_unavailable() {
        return Some(PasteBlock::Unavailable(reason));
    }
    let current = platform::foreground_target();
    let elevated = current
        .map(|target| platform::runs_elevated_above_us(target.process_id))
        .unwrap_or(false);
    paste_block_reason(pinned, current, elevated)
}

/// Honest description of what each output path did to the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardPreservation {
    /// The system clipboard was never read or written (the opt-in
    /// keystroke insert).
    Untouched,
    /// The transcript was briefly placed on the clipboard to send Ctrl+V, then
    /// the user's previous clipboard text was restored (the default paste).
    RestoredAfterPaste,
    /// The transcript was placed on the clipboard to paste, but the previous
    /// clipboard text could not be restored afterwards, so the transcript is
    /// still on the clipboard.
    RestoreFailed,
    /// The transcript was placed on the system clipboard and left there on
    /// purpose (the Copy and Copy + Paste output modes).
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
    // Taken whatever the output mode, so a stale target never carries over
    // to a later dictation.
    let pinned = app.state::<BackendState>().take_paste_target();
    let result = match settings.output_mode {
        OutputMode::SaveOnly => return Ok(None),
        OutputMode::AutoPaste => paste_transcript_text(
            app,
            transcript,
            OutputMode::AutoPaste,
            settings.paste_method.clone(),
            pinned,
            true,
        ),
        OutputMode::CopyClipboard => {
            copy_transcript_text(app, transcript, OutputMode::CopyClipboard, true)
        }
        OutputMode::CopyAndPaste => copy_and_paste_transcript_text(app, transcript, pinned, true),
    };

    result.map(Some)
}

/// Pastes arbitrary `text` into the focused app, replacing any active
/// selection. Reuses the full dictation paste path (foreign-focus guard,
/// borrow-the-clipboard, modifier release, atomic Ctrl+V, then restore), so the
/// text lands the same way a dictation would. Used by the selected-text
/// transform to drop the rewritten text over the original selection.
///
/// The text is wrapped in an ephemeral, never-persisted `Transcript` purely to
/// reuse `paste_transcript_text`; nothing is saved to history. Output events
/// (`scribe:output-*`) are suppressed because this is not a dictation.
pub fn paste_text(app: &AppHandle, text: &str) -> Result<OutputResult, CommandError> {
    let transcript = Transcript::new_last_buffer(text, None, None, None).ok_or_else(|| {
        CommandError::new(
            "transform_result_empty",
            "The transform produced no text to paste.",
        )
    })?;
    let paste_method = app
        .state::<BackendState>()
        .db()
        .and_then(|db| db.get_settings())
        .map(|settings| settings.paste_method)
        .unwrap_or(PasteMethod::ClipboardPaste);

    paste_transcript_text(
        app,
        &transcript,
        OutputMode::AutoPaste,
        paste_method,
        None,
        false,
    )
}

pub fn paste_last_transcript(app: &AppHandle) -> Result<OutputResult, CommandError> {
    let state = app.state::<BackendState>();
    let transcript = state.last_transcript()?.ok_or_else(no_last_transcript)?;
    let paste_method = state.db()?.get_settings()?.paste_method;

    paste_transcript_text(
        app,
        &transcript,
        OutputMode::AutoPaste,
        paste_method,
        None,
        true,
    )
}

pub fn copy_last_transcript(app: &AppHandle) -> Result<OutputResult, CommandError> {
    let state = app.state::<BackendState>();
    let transcript = state.last_transcript()?.ok_or_else(no_last_transcript)?;

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

    paste_transcript_text(
        app,
        &transcript,
        OutputMode::AutoPaste,
        paste_method,
        None,
        true,
    )
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
    pinned: Option<PasteTarget>,
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

        if let Some(block) = paste_block(pinned) {
            set_private_clipboard_text(&transcript.text)?;
            return Ok(paste_skipped_result(
                transcript,
                output_mode,
                paste_method.clone(),
                block,
            ));
        }

        match paste_method {
            PasteMethod::DirectInsert => {
                platform::type_text(&transcript.text)?;
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
                    message: "Inserted via keystrokes; clipboard untouched.".to_string(),
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
    pinned: Option<PasteTarget>,
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
        if let Some(block) = paste_block(pinned) {
            // The text is already on the clipboard, where this mode keeps it.
            return Ok(paste_skipped_result(
                transcript,
                output_mode,
                PasteMethod::ClipboardPaste,
                block,
            ));
        }
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

/// Default "Paste (keeps your clipboard)": borrow the clipboard to send a real
/// Ctrl+V, then put the user's previous clipboard contents back. This makes the
/// insert land as a single atomic paste (unlike the keystroke path, which can
/// visibly stream characters in chat/browser/Electron apps) without
/// permanently consuming the clipboard.
///
/// Sequence: snapshot ALL current clipboard formats (raw Win32) -> set the
/// clipboard to the transcript text -> send Ctrl+V (which first waits for the
/// paste-hotkey modifiers to release, so the chord can't combine with the
/// injected V) -> small delay to let the target app finish reading the
/// clipboard -> restore every snapshotted format.
///
/// FIDELITY: unlike the old text-only restore, this captures the full clipboard
/// as raw bytes per format, so a previously-copied IMAGE or set of FILES
/// survives the paste, not just text. See `save_clipboard_snapshot` for the
/// formats that are captured vs. skipped (the skipped ones — raw GDI bitmap /
/// metafile handles and delayed-render formats — virtually always co-occur with
/// a GlobalAlloc-backed variant such as CF_DIB/CF_DIBV5/CF_HDROP that IS
/// captured, so images and files still restore correctly).
fn clipboard_paste(
    transcript: &Transcript,
    output_mode: OutputMode,
) -> Result<OutputResult, CommandError> {
    // Borrow: snapshot every clipboard format the user currently has as raw
    // bytes so the full payload (text, image, files, ...) can be restored after
    // the paste. A snapshot failure (e.g. another app is holding the clipboard)
    // is treated as "nothing to restore" so the paste still proceeds.
    let snapshot = platform::save_clipboard_snapshot();

    // The transcript is only borrowing the clipboard, so keep it out of
    // Clipboard History, the cloud clipboard and clipboard managers.
    set_private_clipboard_text(&transcript.text)?;
    // Let the clipboard write settle before the paste reads it.
    // NOTE: the 60ms (pre-paste settle) and 180ms (post-paste settle below) are
    // timing knobs; slow targets may need them raised.
    thread::sleep(Duration::from_millis(60));
    // `send_paste_shortcut` waits for held modifiers to release before injecting
    // Ctrl+V, so a still-held paste chord can't scramble the keystroke.
    platform::send_paste_shortcut()?;
    // Give the target app time to read the clipboard before we overwrite it
    // again with the restore, otherwise it may paste the restored data instead.
    thread::sleep(Duration::from_millis(180));

    // Restore: put the user's previous clipboard contents back. When the
    // snapshot was empty (nothing was there, or it couldn't be captured) this
    // clears the clipboard so the transcript isn't left behind.
    let restore = platform::restore_clipboard_snapshot(&snapshot);

    match restore {
        Ok(()) => Ok(OutputResult {
            transcript_id: transcript.id.clone(),
            action: OutputAction::Paste,
            status: OutputStatus::Completed,
            output_mode,
            paste_method: Some(PasteMethod::ClipboardPaste),
            // We touched the clipboard transiently but put it back, so from the
            // user's perspective nothing was copied and nothing is left behind.
            copied: false,
            pasted: true,
            clipboard_restored: Some(true),
            clipboard_preservation: ClipboardPreservation::RestoredAfterPaste,
            clipboard_restore_error: None,
            message: "Pasted transcript and restored your clipboard.".to_string(),
        }),
        // The paste already succeeded; only the restore failed. Report the
        // partial success honestly rather than erroring the whole operation.
        Err(error) => Ok(OutputResult {
            transcript_id: transcript.id.clone(),
            action: OutputAction::Paste,
            status: OutputStatus::ClipboardRestoreFailed,
            output_mode,
            paste_method: Some(PasteMethod::ClipboardPaste),
            // Restore failed, so the transcript is still sitting on the
            // clipboard.
            copied: true,
            pasted: true,
            clipboard_restored: Some(false),
            clipboard_preservation: ClipboardPreservation::RestoreFailed,
            clipboard_restore_error: Some(error.message),
            message: "Pasted transcript, but couldn't restore your previous clipboard.".to_string(),
        }),
    }
}

/// Text Scribe puts on the clipboard only to paste on the user's behalf. It
/// is marked so Windows keeps it out of Clipboard History (Win+V) and the
/// cloud clipboard, and clipboard managers skip it. Public so the selected-text
/// transform can make the same private writes.
pub fn set_private_clipboard_text(text: &str) -> Result<(), CommandError> {
    let mut clipboard = open_clipboard()?;
    platform::set_private_clipboard_text(&mut clipboard, text).map_err(clipboard_write_failed)
}

/// Text the user asked to keep on the clipboard (Copy, Copy only, Paste and
/// leave it): an ordinary write that Clipboard History may keep.
fn set_clipboard_text(text: &str) -> Result<(), CommandError> {
    open_clipboard()?
        .set_text(text.to_string())
        .map_err(clipboard_write_failed)
}

fn open_clipboard() -> Result<arboard::Clipboard, CommandError> {
    platform::open_clipboard().map_err(|error| {
        CommandError::new(
            "clipboard_unavailable",
            format!("Could not access the clipboard. {}", error),
        )
    })
}

fn clipboard_write_failed(error: arboard::Error) -> CommandError {
    CommandError::new(
        "clipboard_write_failed",
        format!(
            "Could not write transcript text to the clipboard. {}",
            error
        ),
    )
}

/// The honest result when Scribe left the text on the clipboard instead of
/// pasting it.
fn paste_skipped_result(
    transcript: &Transcript,
    output_mode: OutputMode,
    paste_method: PasteMethod,
    block: PasteBlock,
) -> OutputResult {
    log::info!(
        "Paste skipped for transcript {} ({:?}); left on the clipboard",
        transcript.id,
        block
    );
    OutputResult {
        transcript_id: transcript.id.clone(),
        action: OutputAction::CopyClipboard,
        status: OutputStatus::PasteSkipped,
        output_mode,
        paste_method: Some(paste_method),
        copied: true,
        pasted: false,
        clipboard_restored: None,
        clipboard_preservation: ClipboardPreservation::ReplacedWithTranscript,
        clipboard_restore_error: None,
        message: block.message(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const EDITOR: PasteTarget = PasteTarget {
        window: 10,
        process_id: 100,
    };
    const EDITOR_OTHER_WINDOW: PasteTarget = PasteTarget {
        window: 11,
        process_id: 100,
    };
    const CHAT: PasteTarget = PasteTarget {
        window: 20,
        process_id: 200,
    };

    #[test]
    fn pastes_into_the_window_the_dictation_started_in() {
        assert_eq!(paste_block_reason(Some(EDITOR), Some(EDITOR), false), None);
        // Another window or tab of the same app still counts.
        assert_eq!(
            paste_block_reason(Some(EDITOR), Some(EDITOR_OTHER_WINDOW), false),
            None
        );
    }

    #[test]
    fn copies_instead_when_the_user_switched_apps() {
        assert_eq!(
            paste_block_reason(Some(EDITOR), Some(CHAT), false),
            Some(PasteBlock::FocusChanged)
        );
        // An unreadable foreground window is treated as a switch, not a match.
        assert_eq!(
            paste_block_reason(Some(EDITOR), None, false),
            Some(PasteBlock::FocusChanged)
        );
    }

    #[test]
    fn unknown_process_ids_never_match() {
        let unknown = |window| PasteTarget {
            window,
            process_id: 0,
        };
        assert_eq!(
            paste_block_reason(Some(unknown(10)), Some(unknown(20)), false),
            Some(PasteBlock::FocusChanged)
        );
        assert_eq!(
            paste_block_reason(Some(unknown(10)), Some(unknown(10)), false),
            None
        );
    }

    #[test]
    fn without_a_pinned_window_any_target_is_fine() {
        assert_eq!(paste_block_reason(None, Some(CHAT), false), None);
        assert_eq!(paste_block_reason(None, None, false), None);
    }

    #[test]
    fn never_pastes_into_an_elevated_window() {
        assert_eq!(
            paste_block_reason(Some(EDITOR), Some(EDITOR), true),
            Some(PasteBlock::Elevated)
        );
        assert_eq!(
            paste_block_reason(None, Some(CHAT), true),
            Some(PasteBlock::Elevated)
        );
    }

    #[test]
    fn skipped_paste_messages_tell_the_user_what_to_do() {
        for block in [
            PasteBlock::FocusChanged,
            PasteBlock::Elevated,
            PasteBlock::Unavailable("Scribe can't paste into Wayland apps yet."),
        ] {
            assert!(block.message().ends_with("Copied. Press Ctrl+V to paste."));
        }
    }

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
            message: "Inserted via keystrokes; clipboard untouched.".to_string(),
        };

        // The opt-in keystroke path must never report touching the clipboard.
        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::Untouched
        );
        assert!(!result.copied);
        assert!(result.pasted);
    }

    // NOTE: `clipboard_paste` itself can't be exercised here — it needs a real
    // system clipboard (arboard) and a foreground window to paste into, neither
    // of which exists in headless CI. These tests assert the shape of the
    // `OutputResult` it builds on the restore-success and restore-failure paths
    // so the honest-messaging contract is locked down at the unit level.

    #[test]
    fn clipboard_paste_success_result_reports_restored_clipboard() {
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();

        // Mirrors the borrow-and-restore success path: paste happened and the
        // previous clipboard text was put back.
        let result = OutputResult {
            transcript_id: transcript.id,
            action: OutputAction::Paste,
            status: OutputStatus::Completed,
            output_mode: OutputMode::AutoPaste,
            paste_method: Some(PasteMethod::ClipboardPaste),
            copied: false,
            pasted: true,
            clipboard_restored: Some(true),
            clipboard_preservation: ClipboardPreservation::RestoredAfterPaste,
            clipboard_restore_error: None,
            message: "Pasted transcript and restored your clipboard.".to_string(),
        };

        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::RestoredAfterPaste
        );
        assert_eq!(result.status, OutputStatus::Completed);
        // The clipboard was put back, so from the user's view nothing was kept.
        assert!(!result.copied);
        assert!(result.pasted);
        assert_eq!(result.clipboard_restored, Some(true));
        assert!(result.clipboard_restore_error.is_none());
    }

    #[test]
    fn clipboard_paste_restore_failure_result_is_honest() {
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();

        // Mirrors the borrow-and-restore failure path: paste succeeded but the
        // previous clipboard could not be restored, so the transcript is still
        // on the clipboard. This must be reported honestly, not as a hard error.
        let result = OutputResult {
            transcript_id: transcript.id,
            action: OutputAction::Paste,
            status: OutputStatus::ClipboardRestoreFailed,
            output_mode: OutputMode::AutoPaste,
            paste_method: Some(PasteMethod::ClipboardPaste),
            copied: true,
            pasted: true,
            clipboard_restored: Some(false),
            clipboard_preservation: ClipboardPreservation::RestoreFailed,
            clipboard_restore_error: Some("clipboard write failed".to_string()),
            message: "Pasted transcript, but couldn't restore your previous clipboard.".to_string(),
        };

        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::RestoreFailed
        );
        assert_eq!(result.status, OutputStatus::ClipboardRestoreFailed);
        // The paste still landed even though restore failed.
        assert!(result.pasted);
        assert_eq!(result.clipboard_restored, Some(false));
        assert!(result.clipboard_restore_error.is_some());
    }
}
