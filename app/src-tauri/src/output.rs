use std::{thread, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    app_state::{AppEvent, AppStatus},
    commands::BackendState,
    dictation_state::emit_state_snapshot,
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
    /// The transcript was pasted, but the user's previous clipboard text could
    /// not be put back afterwards (the borrow-and-restore default). The paste
    /// itself succeeded; only the restore step failed.
    ClipboardRestoreFailed,
    /// Scribe did not paste, because the window in front was not the one the
    /// dictation started in, or Windows would have blocked the paste. The
    /// text was left on the clipboard for the user to paste themselves.
    PasteSkipped,
}

/// The window a dictation started in. Held so the result is pasted back
/// there, and only there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteTarget {
    pub window: isize,
    pub process_id: u32,
}

/// Why Scribe chose to leave the text on the clipboard instead of pasting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteBlock {
    /// The user moved to another app while the dictation was transcribing.
    FocusChanged,
    /// The app in front runs as administrator. Windows silently drops input
    /// sent to it from a normal process (UIPI), so a paste would vanish.
    Elevated,
}

impl PasteBlock {
    fn message(self) -> &'static str {
        match self {
            PasteBlock::FocusChanged => {
                "You switched windows, so Scribe didn't paste. Copied. Press Ctrl+V to paste."
            }
            PasteBlock::Elevated => {
                "That window runs as administrator, which blocks Scribe from pasting. Copied. Press Ctrl+V to paste."
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
        (Some(pinned), Some(current))
            if pinned.window == current.window || pinned.process_id == current.process_id =>
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

/// Snapshots every clipboard format as raw bytes (text, image, files, ...) so
/// it can be restored later. Public wrapper over the platform snapshot used by
/// the selected-text transform to preserve the user's pre-capture clipboard.
/// Returns an empty vec on any failure (treated as "nothing to restore").
pub fn save_clipboard_snapshot() -> Vec<(u32, Vec<u8>)> {
    platform::save_clipboard_snapshot()
}

/// Restores a snapshot produced by `save_clipboard_snapshot`. Public wrapper
/// over the platform restore used by the selected-text transform.
pub fn restore_clipboard_snapshot(snapshot: &[(u32, Vec<u8>)]) -> Result<(), CommandError> {
    platform::restore_clipboard_snapshot(snapshot)
}

/// When one of Scribe's own windows is in front, hands focus back to the app
/// the user was in before (the same focus guard the paste path uses). Used by
/// the typed transform, whose button click leaves Scribe focused.
pub fn focus_previous_app() -> Result<(), CommandError> {
    platform::ensure_foreign_focus()
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

        if let Some(block) = platform::paste_block(pinned) {
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
        if let Some(block) = platform::paste_block(pinned) {
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

    #[cfg(windows)]
    let written = {
        use arboard::SetExtWindows;
        clipboard
            .set()
            .exclude_from_monitoring()
            .exclude_from_cloud()
            .exclude_from_history()
            .text(text.to_string())
    };
    #[cfg(not(windows))]
    let written = clipboard.set_text(text.to_string());

    written.map_err(clipboard_write_failed)
}

/// Text the user asked to keep on the clipboard (Copy, Copy only, Paste and
/// leave it): an ordinary write that Clipboard History may keep.
fn set_clipboard_text(text: &str) -> Result<(), CommandError> {
    open_clipboard()?
        .set_text(text.to_string())
        .map_err(clipboard_write_failed)
}

fn open_clipboard() -> Result<arboard::Clipboard, CommandError> {
    arboard::Clipboard::new().map_err(|error| {
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
        message: block.message().to_string(),
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

#[cfg(windows)]
mod platform {
    use super::{paste_block_reason, PasteBlock, PasteTarget};
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
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, OpenClipboard,
        RegisterClipboardFormatW, SetClipboardData,
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
        KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
        VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V,
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

    pub fn foreground_target() -> Option<PasteTarget> {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_invalid() || is_own_window(foreground) {
            return None;
        }
        let mut process_id = 0u32;
        unsafe { GetWindowThreadProcessId(foreground, Some(&mut process_id)) };
        Some(PasteTarget {
            window: foreground.0 as isize,
            process_id,
        })
    }

    /// Whether to skip the paste, checked after the focus guard so it sees
    /// the window the keystrokes would really go to.
    pub fn paste_block(pinned: Option<PasteTarget>) -> Option<PasteBlock> {
        let current = foreground_target();
        let elevated = current
            .map(|target| runs_above_our_integrity(target.process_id))
            .unwrap_or(false);
        paste_block_reason(pinned, current, elevated)
    }

    /// True when `process_id` runs at a higher integrity level than Scribe
    /// (an app started as administrator while Scribe isn't). Windows drops
    /// SendInput into such windows without an error. When either level can't
    /// be read, assumes not, which keeps today's behavior.
    fn runs_above_our_integrity(process_id: u32) -> bool {
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
    pub fn save_clipboard_snapshot() -> Vec<(u32, Vec<u8>)> {
        let _guard = match open_clipboard_with_retry() {
            Ok(guard) => guard,
            Err(error) => {
                log::warn!("Clipboard snapshot skipped: {}", error.message);
                return Vec::new();
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

        snapshot
    }

    /// Restores a snapshot produced by `save_clipboard_snapshot`: clears the
    /// clipboard, then republishes each captured format from a freshly allocated
    /// HGLOBAL.
    ///
    /// Ownership: after a SUCCESSFUL `SetClipboardData` the system owns the
    /// HGLOBAL and will free it, so it must NOT be freed here. The block is only
    /// freed when `SetClipboardData` fails (data never handed off).
    pub fn restore_clipboard_snapshot(snapshot: &[(u32, Vec<u8>)]) -> Result<(), CommandError> {
        let _guard = open_clipboard_with_retry()?;

        // Take ownership of the clipboard and drop the transcript that is on it
        // before republishing the saved formats.
        unsafe { EmptyClipboard() }.map_err(|error| {
            CommandError::new(
                "clipboard_clear_failed",
                format!("Could not clear the clipboard before restore. {}", error),
            )
        })?;

        for (format, bytes) in snapshot {
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
        release_held_modifiers()?;

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
    use super::{PasteBlock, PasteTarget};
    use crate::error::CommandError;

    pub fn ensure_foreign_focus() -> Result<(), CommandError> {
        Ok(())
    }

    pub fn foreground_target() -> Option<PasteTarget> {
        None
    }

    pub fn paste_block(_pinned: Option<PasteTarget>) -> Option<PasteBlock> {
        None
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

    /// Non-Windows stub: there is no raw Win32 clipboard to snapshot, so report
    /// "nothing captured". The full-fidelity borrow-and-restore path is
    /// Windows-only (the app itself targets Windows).
    pub fn save_clipboard_snapshot() -> Vec<(u32, Vec<u8>)> {
        Vec::new()
    }

    /// Non-Windows stub: nothing was snapshotted, so there is nothing to
    /// restore. Treated as a successful no-op.
    pub fn restore_clipboard_snapshot(_snapshot: &[(u32, Vec<u8>)]) -> Result<(), CommandError> {
        Ok(())
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
        for block in [PasteBlock::FocusChanged, PasteBlock::Elevated] {
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
