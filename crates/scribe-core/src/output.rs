use std::{thread, time::Duration};

use serde::{Deserialize, Serialize};

pub use crate::platform::PasteTarget;
use crate::{
    app_state::{AppEvent, AppStateSnapshot, AppStatus},
    backend::BackendState,
    core::Core,
    dictation_state::emit_state_snapshot_with,
    error::CommandError,
    events::CoreEvent,
    host::Host,
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
        let reason = match self {
            PasteBlock::FocusChanged => "You switched windows, so Scribe didn't paste.",
            PasteBlock::Elevated => {
                "That window runs as administrator, which blocks Scribe from pasting."
            }
            PasteBlock::Unavailable(reason) => reason,
        };
        format!(
            "{reason} Copied. Press {} to paste.",
            platform::PASTE_SHORTCUT
        )
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

/// What the output paths do to the system: the focus guard, the paste
/// check, keystrokes, and the clipboard. The paths take it as a parameter so
/// a test can run them without typing into, pasting into or overwriting the
/// clipboard of the machine it runs on; the app always passes
/// [`SystemOutput`]. It holds no lock, and the paths never call it while
/// holding an engine lock.
trait OutputPlatform {
    /// The user's clipboard, saved before a paste borrows it.
    type Snapshot;
    /// The transcript lent to the clipboard for one paste.
    type Loan;

    fn ensure_foreign_focus(&self) -> Result<(), CommandError>;
    fn paste_block(&self, pinned: Option<PasteTarget>) -> Option<PasteBlock>;
    fn type_text(&self, text: &str) -> Result<(), CommandError>;
    fn set_private_clipboard_text(&self, text: &str) -> Result<(), CommandError>;
    fn set_clipboard_text(&self, text: &str) -> Result<(), CommandError>;
    fn send_paste_shortcut(&self) -> Result<(), CommandError>;
    fn save_clipboard_snapshot(&self) -> Self::Snapshot;
    fn lend_clipboard_text(&self, text: &str) -> Result<Self::Loan, CommandError>;
    fn wait_for_read(&self, loan: Self::Loan, timeout: Duration) -> platform::LoanOutcome;
    fn restore_clipboard_snapshot(&self, snapshot: &Self::Snapshot) -> Result<(), CommandError>;
}

/// The real system: each call is the `platform` call the output paths made
/// before the seam.
struct SystemOutput;

impl OutputPlatform for SystemOutput {
    type Snapshot = platform::ClipboardSnapshot;
    type Loan = platform::ClipboardLoan;

    fn ensure_foreign_focus(&self) -> Result<(), CommandError> {
        platform::ensure_foreign_focus()
    }

    fn paste_block(&self, pinned: Option<PasteTarget>) -> Option<PasteBlock> {
        paste_block(pinned)
    }

    fn type_text(&self, text: &str) -> Result<(), CommandError> {
        platform::type_text(text)
    }

    fn set_private_clipboard_text(&self, text: &str) -> Result<(), CommandError> {
        set_private_clipboard_text(text)
    }

    fn set_clipboard_text(&self, text: &str) -> Result<(), CommandError> {
        set_clipboard_text(text)
    }

    fn send_paste_shortcut(&self) -> Result<(), CommandError> {
        platform::send_paste_shortcut()
    }

    fn save_clipboard_snapshot(&self) -> Self::Snapshot {
        platform::save_clipboard_snapshot()
    }

    fn lend_clipboard_text(&self, text: &str) -> Result<Self::Loan, CommandError> {
        platform::lend_clipboard_text(text)
    }

    fn wait_for_read(&self, loan: Self::Loan, timeout: Duration) -> platform::LoanOutcome {
        loan.wait_for_read(timeout)
    }

    fn restore_clipboard_snapshot(&self, snapshot: &Self::Snapshot) -> Result<(), CommandError> {
        platform::restore_clipboard_snapshot(snapshot)
    }
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

/// Outputs a finished dictation the way the settings say (save only, paste,
/// copy, or copy and paste).
pub fn handle_transcription_output_with(
    core: &Core,
    transcript: &Transcript,
    settings: &AppSettings,
) -> Result<Option<OutputResult>, CommandError> {
    handle_transcription_output_in(core, &SystemOutput, transcript, settings)
}

fn handle_transcription_output_in(
    core: &Core,
    system: &impl OutputPlatform,
    transcript: &Transcript,
    settings: &AppSettings,
) -> Result<Option<OutputResult>, CommandError> {
    // Taken whatever the output mode, so a stale target never carries over
    // to a later dictation.
    let pinned = core.backend().take_paste_target();
    let result = match settings.output_mode {
        OutputMode::SaveOnly => return Ok(None),
        OutputMode::AutoPaste => paste_transcript_text(
            core,
            system,
            transcript,
            OutputMode::AutoPaste,
            settings.paste_method.clone(),
            pinned,
            true,
        ),
        OutputMode::CopyClipboard => copy_transcript_text(
            core.host().as_ref(),
            system,
            transcript,
            OutputMode::CopyClipboard,
            true,
        ),
        OutputMode::CopyAndPaste => {
            copy_and_paste_transcript_text(core, system, transcript, pinned, true)
        }
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
pub fn paste_text_with(core: &Core, text: &str) -> Result<OutputResult, CommandError> {
    paste_text_in(core, &SystemOutput, text)
}

fn paste_text_in(
    core: &Core,
    system: &impl OutputPlatform,
    text: &str,
) -> Result<OutputResult, CommandError> {
    let transcript = Transcript::new_last_buffer(text, None, None, None).ok_or_else(|| {
        CommandError::new(
            "transform_result_empty",
            "The transform produced no text to paste.",
        )
    })?;
    let paste_method = core
        .backend()
        .db()
        .and_then(|db| db.get_settings())
        .map(|settings| settings.paste_method)
        .unwrap_or(PasteMethod::ClipboardPaste);

    paste_transcript_text(
        core,
        system,
        &transcript,
        OutputMode::AutoPaste,
        paste_method,
        None,
        false,
    )
}

/// Pastes the Last Transcript Buffer.
pub fn paste_last_transcript_with(core: &Core) -> Result<OutputResult, CommandError> {
    paste_last_in(core, &SystemOutput)
}

fn paste_last_in(core: &Core, system: &impl OutputPlatform) -> Result<OutputResult, CommandError> {
    let state = core.backend();
    let transcript = state.last_transcript()?.ok_or_else(no_last_transcript)?;
    let paste_method = state.db()?.get_settings()?.paste_method;

    paste_transcript_text(
        core,
        system,
        &transcript,
        OutputMode::AutoPaste,
        paste_method,
        None,
        true,
    )
}

/// Copies the Last Transcript Buffer to the clipboard.
pub fn copy_last_transcript_with(core: &Core) -> Result<OutputResult, CommandError> {
    copy_last(core.backend(), core.host().as_ref(), &SystemOutput)
}

fn copy_last(
    state: &BackendState,
    host: &dyn Host,
    system: &impl OutputPlatform,
) -> Result<OutputResult, CommandError> {
    let transcript = state.last_transcript()?.ok_or_else(no_last_transcript)?;

    copy_transcript_text(host, system, &transcript, OutputMode::CopyClipboard, true)
}

/// Pastes a saved transcript.
pub fn paste_transcript_with(core: &Core, id: &str) -> Result<OutputResult, CommandError> {
    paste_saved_in(core, &SystemOutput, id)
}

fn paste_saved_in(
    core: &Core,
    system: &impl OutputPlatform,
    id: &str,
) -> Result<OutputResult, CommandError> {
    let state = core.backend();
    let (transcript, paste_method) = {
        let db = state.db()?;
        let transcript = db
            .get_transcript_by_id(id)?
            .ok_or_else(|| transcript_not_found(id))?;
        let settings = db.get_settings()?;
        (transcript, settings.paste_method)
    };

    paste_transcript_text(
        core,
        system,
        &transcript,
        OutputMode::AutoPaste,
        paste_method,
        None,
        true,
    )
}

/// Copies a saved transcript to the clipboard.
pub fn copy_transcript_with(core: &Core, id: &str) -> Result<OutputResult, CommandError> {
    copy_saved(core.backend(), core.host().as_ref(), &SystemOutput, id)
}

fn copy_saved(
    state: &BackendState,
    host: &dyn Host,
    system: &impl OutputPlatform,
    id: &str,
) -> Result<OutputResult, CommandError> {
    let transcript = state
        .db()?
        .get_transcript_by_id(id)?
        .ok_or_else(|| transcript_not_found(id))?;

    copy_transcript_text(host, system, &transcript, OutputMode::CopyClipboard, true)
}

fn paste_transcript_text(
    core: &Core,
    system: &impl OutputPlatform,
    transcript: &Transcript,
    output_mode: OutputMode,
    paste_method: PasteMethod,
    pinned: Option<PasteTarget>,
    emit_events: bool,
) -> Result<OutputResult, CommandError> {
    let host = core.host().as_ref();
    emit_output_started(
        host,
        transcript,
        OutputAction::Paste,
        &output_mode,
        Some(&paste_method),
    );
    let result = (|| {
        let _state_guard = PasteStateGuard::start(core.backend(), |snapshot| {
            emit_state_snapshot_with(core, snapshot)
        });

        // The paste hotkey can fire while one of our own windows is focused;
        // the transcript must never be typed into Scribe itself.
        system.ensure_foreign_focus()?;

        if let Some(block) = system.paste_block(pinned) {
            system.set_private_clipboard_text(&transcript.text)?;
            return Ok(paste_skipped_result(
                transcript,
                output_mode,
                paste_method.clone(),
                block,
            ));
        }

        match paste_method {
            PasteMethod::DirectInsert => {
                system.type_text(&transcript.text)?;
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
            PasteMethod::ClipboardPaste => clipboard_paste(system, transcript, output_mode),
        }
    })();

    finish_output(host, &transcript.id, result, emit_events)
}

/// Announces how an output ended, when `emit_events` is set (a dictation or
/// a transcript the user picked; the selected-text transform sends nothing):
/// `scribe:output-completed` and, for a paste Scribe held back, the system
/// notification after it; or `scribe:output-failed`, exactly once. The
/// dictation relies on this being the only `scribe:output-failed` for its
/// output (`dictation::settle_output`, #156). Returns `result` unchanged.
fn finish_output(
    host: &dyn Host,
    transcript_id: &str,
    result: Result<OutputResult, CommandError>,
    emit_events: bool,
) -> Result<OutputResult, CommandError> {
    match result {
        Ok(result) => {
            if emit_events {
                emit_output_completed(host, &result);
            }
            Ok(result)
        }
        Err(error) => {
            if emit_events {
                emit_output_failed_with(host, transcript_id.to_string(), &error);
            }
            Err(error)
        }
    }
}

/// Copies the transcript to the clipboard and leaves it there. A copy never
/// skips a paste, so it never raises the "didn't paste" notification.
fn copy_transcript_text(
    host: &dyn Host,
    system: &impl OutputPlatform,
    transcript: &Transcript,
    output_mode: OutputMode,
    emit_events: bool,
) -> Result<OutputResult, CommandError> {
    emit_output_started(
        host,
        transcript,
        OutputAction::CopyClipboard,
        &output_mode,
        None,
    );
    let result = (|| {
        system.set_clipboard_text(&transcript.text)?;

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

    // Always `Completed` on success, so no notification is raised.
    finish_output(host, &transcript.id, result, emit_events)
}

fn copy_and_paste_transcript_text(
    core: &Core,
    system: &impl OutputPlatform,
    transcript: &Transcript,
    pinned: Option<PasteTarget>,
    emit_events: bool,
) -> Result<OutputResult, CommandError> {
    let host = core.host().as_ref();
    let output_mode = OutputMode::CopyAndPaste;
    emit_output_started(
        host,
        transcript,
        OutputAction::CopyAndPaste,
        &output_mode,
        None,
    );
    let result = (|| {
        let _state_guard = PasteStateGuard::start(core.backend(), |snapshot| {
            emit_state_snapshot_with(core, snapshot)
        });

        system.set_clipboard_text(&transcript.text)?;
        thread::sleep(Duration::from_millis(60));
        // The paste hotkey can fire while one of our own windows is focused;
        // the transcript must never be typed into Scribe itself. The
        // clipboard copy above is kept either way so a manual paste still works.
        system.ensure_foreign_focus()?;
        if let Some(block) = system.paste_block(pinned) {
            // The text is already on the clipboard, where this mode keeps it.
            return Ok(paste_skipped_result(
                transcript,
                output_mode,
                PasteMethod::ClipboardPaste,
                block,
            ));
        }
        system.send_paste_shortcut()?;

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

    finish_output(host, &transcript.id, result, emit_events)
}

/// Longest wait for the target app to read the pasted text before the user's
/// clipboard is put back anyway. Generous, because restoring early pastes the
/// user's old clipboard in place of the dictation (issue #105); an app that
/// reads normally ends the wait as soon as it does.
const PASTE_READ_TIMEOUT: Duration = Duration::from_secs(3);

/// Default "Paste (keeps your clipboard)": borrow the clipboard to send a real
/// Ctrl+V, then put the user's previous clipboard contents back. This makes the
/// insert land as a single atomic paste (unlike the keystroke path, which can
/// visibly stream characters in chat/browser/Electron apps) without
/// permanently consuming the clipboard.
///
/// Sequence: snapshot ALL current clipboard formats (raw Win32) -> lend the
/// clipboard to the transcript text -> send Ctrl+V (which first waits for the
/// paste-hotkey modifiers to release, so the chord can't combine with the
/// injected V) -> wait until the target app has read the text (on Windows
/// this is observed, see `platform::lend_clipboard_text`; elsewhere a short
/// fixed wait) -> restore every snapshotted format.
///
/// FIDELITY: unlike the old text-only restore, this captures the full clipboard
/// as raw bytes per format, so a previously-copied IMAGE or set of FILES
/// survives the paste, not just text. See `save_clipboard_snapshot` for the
/// formats that are captured vs. skipped (the skipped ones — raw GDI bitmap /
/// metafile handles and delayed-render formats — virtually always co-occur with
/// a GlobalAlloc-backed variant such as CF_DIB/CF_DIBV5/CF_HDROP that IS
/// captured, so images and files still restore correctly).
fn clipboard_paste(
    system: &impl OutputPlatform,
    transcript: &Transcript,
    output_mode: OutputMode,
) -> Result<OutputResult, CommandError> {
    // Borrow: snapshot every clipboard format the user currently has as raw
    // bytes so the full payload (text, image, files, ...) can be restored after
    // the paste. A snapshot failure (e.g. another app is holding the clipboard)
    // is treated as "nothing to restore" so the paste still proceeds.
    let snapshot = system.save_clipboard_snapshot();

    // The transcript is only borrowing the clipboard, so it is kept out of
    // Clipboard History, the cloud clipboard and clipboard managers.
    let loan = system.lend_clipboard_text(&transcript.text)?;
    // Let the clipboard write settle before the paste reads it.
    thread::sleep(Duration::from_millis(60));
    // `send_paste_shortcut` waits for held modifiers to release before injecting
    // Ctrl+V, so a still-held paste chord can't scramble the keystroke.
    if let Err(error) = system.send_paste_shortcut() {
        // Nothing was pasted; don't leave the transcript in place of the
        // user's clipboard.
        if let Err(restore_error) = system.restore_clipboard_snapshot(&snapshot) {
            log::warn!(
                "Could not restore the clipboard after a failed paste: {}",
                restore_error.message
            );
        }
        return Err(error);
    }
    // Never put the old clipboard back before the target has read the text,
    // or it pastes the old clipboard instead of the dictation.
    let outcome = system.wait_for_read(loan, PASTE_READ_TIMEOUT);
    if outcome == platform::LoanOutcome::NotRead {
        log::warn!(
            "The app in front didn't read the pasted text within {} s; restoring the clipboard anyway.",
            PASTE_READ_TIMEOUT.as_secs()
        );
    }

    Ok(restore_result(transcript, output_mode, outcome, || {
        system.restore_clipboard_snapshot(&snapshot)
    }))
}

/// Puts the user's clipboard back after a paste (unless they copied
/// something newer meanwhile) and builds the honest result for it.
fn restore_result(
    transcript: &Transcript,
    output_mode: OutputMode,
    outcome: platform::LoanOutcome,
    restore: impl FnOnce() -> Result<(), CommandError>,
) -> OutputResult {
    // Something newer than the transcript is on the clipboard: the user's
    // own clipboard is already theirs again, and a restore would wipe it.
    // When the snapshot was empty (nothing was there, or it couldn't be
    // captured) the restore clears the clipboard so the transcript isn't
    // left behind.
    let restore = if outcome == platform::LoanOutcome::Replaced {
        log::info!("The clipboard changed during the paste; keeping the newer contents.");
        Ok(())
    } else {
        restore()
    };

    match restore {
        Ok(()) => OutputResult {
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
        },
        // The paste already succeeded; only the restore failed. Report the
        // partial success honestly rather than erroring the whole operation.
        Err(error) => OutputResult {
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
        },
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
    host: &dyn Host,
    transcript: &Transcript,
    action: OutputAction,
    output_mode: &OutputMode,
    paste_method: Option<&PasteMethod>,
) {
    host.emit(CoreEvent::OutputStarted(OutputStartedPayload {
        transcript_id: transcript.id.clone(),
        action,
        output_mode: output_mode.clone(),
        paste_method: paste_method.cloned(),
    }));
}

/// Logs and sends `scribe:output-completed`, then raises the system
/// notification for a paste Scribe held back.
fn emit_output_completed(host: &dyn Host, result: &OutputResult) {
    send_output_completed(host, result);
    if let Some((title, body)) = paste_skipped_notification(result) {
        notify(host, title, body);
    }
}

/// Logs and sends `scribe:output-completed`, without the notification.
fn send_output_completed(host: &dyn Host, result: &OutputResult) {
    log::info!(
        "Output {:?} {:?} for transcript {}: {}",
        result.action,
        result.status,
        result.transcript_id,
        result.message
    );
    host.emit(CoreEvent::OutputCompleted(result.clone()));
}

/// The system notification for a paste Scribe held back. The text did not
/// land where the user expected and they are in another window, so this is
/// always shown, whatever the in-app notifications setting says.
fn paste_skipped_notification(result: &OutputResult) -> Option<(&'static str, &str)> {
    (result.status == OutputStatus::PasteSkipped)
        .then_some(("Scribe didn't paste", result.message.as_str()))
}

/// Best-effort system notification, shown from the backend so it works with
/// no window open. Showing does not block; failures are only logged.
fn notify(host: &dyn Host, title: &str, body: &str) {
    if let Err(error) = host.notify(title, body) {
        log::warn!("Could not show the \"{}\" notification: {}", title, error);
    }
}

/// Logs the failure and sends `scribe:output-failed`.
pub fn emit_output_failed_with(host: &dyn Host, transcript_id: String, error: &CommandError) {
    log::error!(
        "Output failed for transcript {} ({}): {}",
        transcript_id,
        error.code,
        error.message
    );
    host.emit(CoreEvent::OutputFailed(OutputFailedPayload {
        transcript_id,
        code: error.code.clone(),
        message: error.message.clone(),
    }));
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
pub struct OutputStartedPayload {
    pub transcript_id: String,
    pub action: OutputAction,
    pub output_mode: OutputMode,
    pub paste_method: Option<PasteMethod>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFailedPayload {
    pub transcript_id: String,
    pub code: String,
    pub message: String,
}

/// Shows Pasting while a paste runs, when the dictation state was Ready, and
/// goes back to Ready when dropped. `publish` sends each new state out; the
/// paste paths pass `dictation_state::emit_state_snapshot_with` (the `audio`
/// lane's).
struct PasteStateGuard<'a, F: Fn(&AppStateSnapshot)> {
    state: &'a BackendState,
    publish: F,
    active: bool,
}

impl<'a, F: Fn(&AppStateSnapshot)> PasteStateGuard<'a, F> {
    fn start(state: &'a BackendState, publish: F) -> Self {
        let active = state
            .app_state()
            .map(|state| state.snapshot().status == AppStatus::Ready)
            .unwrap_or(false);

        if active {
            if let Ok(snapshot) = state.transition_app_state(AppEvent::StartPasting) {
                publish(&snapshot);
            }
        }

        Self {
            state,
            publish,
            active,
        }
    }
}

impl<F: Fn(&AppStateSnapshot)> Drop for PasteStateGuard<'_, F> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        if let Ok(snapshot) = self.state.transition_app_state(AppEvent::PasteCompleted) {
            (self.publish)(&snapshot);
        }
    }
}

/// Whether pasting into other apps needs a permission, whether it has been
/// given, and how dictations reach other apps right now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PastePermission {
    pub required: bool,
    pub granted: bool,
    /// Which permission: "accessibility" (macOS, granted in System
    /// Settings), "waylandPortal" (Linux on Wayland, granted once in a
    /// desktop dialog) or "none".
    pub kind: &'static str,
    /// The user turned the Wayland request down; Scribe copies instead and
    /// doesn't ask again on its own.
    pub declined: bool,
    /// The insert method in effect (the Dashboard shows it).
    pub method: crate::platform::InsertMethod,
}

/// The paste permission on this system right now.
pub fn paste_permission() -> PastePermission {
    let method = crate::platform::insert_method();
    #[cfg(target_os = "macos")]
    {
        PastePermission {
            required: true,
            granted: crate::platform::accessibility_granted(),
            kind: "accessibility",
            declined: false,
            method,
        }
    }
    #[cfg(target_os = "linux")]
    {
        use crate::platform::PortalPaste;
        match crate::platform::wayland_paste_permission() {
            Some((state, granted)) => PastePermission {
                required: true,
                granted,
                kind: "waylandPortal",
                declined: state == PortalPaste::Declined,
                method,
            },
            // X11 lets any app send keystrokes.
            None => PastePermission {
                required: false,
                granted: true,
                kind: "none",
                declined: false,
                method,
            },
        }
    }
    // Windows lets any app send keystrokes.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        PastePermission {
            required: false,
            granted: true,
            kind: "none",
            declined: false,
            method,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_core::TestCore;
    use crate::host::{HostCall, Recorded, RecordingHost};
    use std::sync::{Arc, Mutex};

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
            assert!(block.message().ends_with(&format!(
                "Copied. Press {} to paste.",
                platform::PASTE_SHORTCUT
            )));
        }
    }

    #[test]
    fn only_a_skipped_paste_raises_a_system_notification() {
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();
        let skipped = paste_skipped_result(
            &transcript,
            OutputMode::AutoPaste,
            PasteMethod::ClipboardPaste,
            PasteBlock::FocusChanged,
        );
        assert_eq!(
            paste_skipped_notification(&skipped),
            Some(("Scribe didn't paste", skipped.message.as_str()))
        );

        let pasted = OutputResult {
            status: OutputStatus::Completed,
            pasted: true,
            ..skipped.clone()
        };
        assert_eq!(paste_skipped_notification(&pasted), None);
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

    fn recording_host() -> crate::host::RecordingHost {
        crate::host::RecordingHost::new(
            crate::paths::Paths::from_parts("/d".into(), "/c".into(), "/l".into(), None, None),
            crate::host::Flavor::Stable,
        )
    }

    /// The (name, JSON payload) of every event the host was sent.
    fn sent(host: &crate::host::RecordingHost) -> Vec<(&'static str, serde_json::Value)> {
        host.events()
            .iter()
            .map(|event| (event.name(), serde_json::to_value(event).unwrap()))
            .collect()
    }

    #[test]
    fn output_events_go_to_the_host_with_todays_names_and_payloads() {
        let host = recording_host();
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();

        emit_output_started(
            &host,
            &transcript,
            OutputAction::Paste,
            &OutputMode::AutoPaste,
            Some(&PasteMethod::ClipboardPaste),
        );
        let completed = paste_skipped_result(
            &transcript,
            OutputMode::AutoPaste,
            PasteMethod::ClipboardPaste,
            PasteBlock::FocusChanged,
        );
        send_output_completed(&host, &completed);
        emit_output_failed_with(
            &host,
            transcript.id.clone(),
            &CommandError::new("clipboard_unavailable", "Could not access the clipboard."),
        );

        assert_eq!(
            sent(&host),
            [
                (
                    "scribe:output-started",
                    serde_json::json!({
                        "transcriptId": transcript.id,
                        "action": "paste",
                        "outputMode": "auto_paste",
                        "pasteMethod": "clipboard_paste",
                    })
                ),
                (
                    "scribe:output-completed",
                    serde_json::to_value(&completed).unwrap()
                ),
                (
                    "scribe:output-failed",
                    serde_json::json!({
                        "transcriptId": transcript.id,
                        "code": "clipboard_unavailable",
                        "message": "Could not access the clipboard.",
                    })
                ),
            ]
        );
    }

    fn skipped_paste(transcript: &Transcript) -> OutputResult {
        paste_skipped_result(
            transcript,
            OutputMode::AutoPaste,
            PasteMethod::ClipboardPaste,
            PasteBlock::FocusChanged,
        )
    }

    fn completed_paste(transcript: &Transcript) -> OutputResult {
        restore_result(
            transcript,
            OutputMode::AutoPaste,
            platform::LoanOutcome::Read,
            || Ok(()),
        )
    }

    fn not_pasted() -> CommandError {
        CommandError::new("paste_failed", "Could not send the paste shortcut.")
    }

    #[test]
    fn a_skipped_paste_is_announced_then_notified() {
        let host = recording_host();
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();
        let skipped = skipped_paste(&transcript);

        let result = finish_output(&host, &transcript.id, Ok(skipped.clone()), true).unwrap();

        assert_eq!(result.status, OutputStatus::PasteSkipped);
        assert_eq!(
            host.sequence(),
            [
                Recorded::Event(
                    "scribe:output-completed",
                    serde_json::to_value(&skipped).unwrap()
                ),
                Recorded::Call(HostCall::Notify {
                    title: "Scribe didn't paste".to_string(),
                    body: skipped.message.clone(),
                }),
            ]
        );
        assert_eq!(
            host.calls(),
            [HostCall::Notify {
                title: "Scribe didn't paste".to_string(),
                body: skipped.message,
            }]
        );
    }

    #[test]
    fn a_failed_notification_is_only_logged() {
        let host = recording_host();
        host.fail(crate::host::Failing::Notify);
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();
        let skipped = skipped_paste(&transcript);

        // The paste was still held back and announced: the result and the
        // event stand, and no failure is announced for the notification.
        let result = finish_output(&host, &transcript.id, Ok(skipped.clone()), true).unwrap();

        assert_eq!(result.status, OutputStatus::PasteSkipped);
        assert_eq!(
            host.sequence(),
            [
                Recorded::Event(
                    "scribe:output-completed",
                    serde_json::to_value(&skipped).unwrap()
                ),
                Recorded::Call(HostCall::Notify {
                    title: "Scribe didn't paste".to_string(),
                    body: skipped.message.clone(),
                }),
            ]
        );
        assert_eq!(host.calls().len(), 1);
    }

    #[test]
    fn a_completed_output_raises_no_notification() {
        let host = recording_host();
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();
        let completed = completed_paste(&transcript);

        finish_output(&host, &transcript.id, Ok(completed.clone()), true).unwrap();

        assert_eq!(
            host.sequence(),
            [Recorded::Event(
                "scribe:output-completed",
                serde_json::to_value(&completed).unwrap()
            )]
        );
        assert!(host.calls().is_empty());
    }

    #[test]
    fn a_failed_output_is_announced_exactly_once() {
        let host = recording_host();
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();

        let error = finish_output(&host, &transcript.id, Err(not_pasted()), true).unwrap_err();

        assert_eq!(error.code, "paste_failed");
        assert_eq!(
            host.sequence(),
            [Recorded::Event(
                "scribe:output-failed",
                serde_json::json!({
                    "transcriptId": transcript.id,
                    "code": "paste_failed",
                    "message": "Could not send the paste shortcut.",
                })
            )]
        );
        assert!(host.calls().is_empty());
    }

    #[test]
    fn an_output_without_events_sends_nothing() {
        // The selected-text transform pastes with events off: neither the
        // outcome nor the "didn't paste" notification reaches the host.
        let host = recording_host();
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();

        let result =
            finish_output(&host, &transcript.id, Ok(skipped_paste(&transcript)), false).unwrap();
        assert_eq!(result.status, OutputStatus::PasteSkipped);
        let error = finish_output(&host, &transcript.id, Err(not_pasted()), false).unwrap_err();
        assert_eq!(error.code, "paste_failed");

        assert!(host.sequence().is_empty());
        assert!(host.events().is_empty());
        assert!(host.calls().is_empty());
    }

    fn backend() -> (BackendState, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "scribe-output-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let backend = BackendState::new(
            crate::db::Database::in_memory().unwrap(),
            root.join("recordings"),
            crate::data_hygiene::DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        );
        (backend, root)
    }

    fn to_ready(state: &BackendState) {
        for event in [
            AppEvent::StartRecording,
            AppEvent::StopRecording,
            AppEvent::ValidAudio,
            AppEvent::TranscriptionSucceeded,
        ] {
            state.transition_app_state(event).unwrap();
        }
    }

    fn status(state: &BackendState) -> AppStatus {
        state.app_state().unwrap().snapshot().status
    }

    #[test]
    fn a_paste_after_a_dictation_shows_pasting_then_ready() {
        let (state, root) = backend();
        to_ready(&state);
        let published = std::cell::RefCell::new(Vec::new());
        {
            let _guard = PasteStateGuard::start(&state, |snapshot: &AppStateSnapshot| {
                published.borrow_mut().push(snapshot.status.clone())
            });
            assert_eq!(status(&state), AppStatus::Pasting);
        }
        assert_eq!(status(&state), AppStatus::Ready);
        assert_eq!(
            published.into_inner(),
            [AppStatus::Pasting, AppStatus::Ready]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_paste_outside_ready_leaves_the_state_alone() {
        let (state, root) = backend();
        let published = std::cell::RefCell::new(Vec::new());
        {
            let _guard = PasteStateGuard::start(&state, |snapshot: &AppStateSnapshot| {
                published.borrow_mut().push(snapshot.status.clone())
            });
            assert_eq!(status(&state), AppStatus::Idle);
        }
        assert_eq!(status(&state), AppStatus::Idle);
        assert!(published.into_inner().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn copying_with_nothing_to_copy_fails_before_any_event() {
        let (state, root) = backend();
        let host = Arc::new(recording_host());
        let system = FakeSystem::new(&host);

        let error = copy_last(&state, host.as_ref(), &system).unwrap_err();
        assert_eq!(error.code, "last_transcript_missing");
        let error = copy_saved(&state, host.as_ref(), &system, "no-such-id").unwrap_err();
        assert_eq!(error.code, "transcript_not_found");
        assert!(error.message.contains("no-such-id"));

        assert!(host.events().is_empty());
        assert!(system.calls().is_empty(), "nothing reached the system");
        let _ = std::fs::remove_dir_all(root);
    }

    // The real clipboard paste needs a real clipboard and a window to paste
    // into; the Windows end-to-end test drives it (including an app that
    // reads the clipboard late, issue #105). These tests cover how the
    // wait's outcome and the restore turn into the result the user sees; the
    // tests on `FakeSystem` below cover the order of its steps.

    use crate::platform::LoanOutcome;

    fn restore_with(outcome: LoanOutcome, works: bool) -> (OutputResult, bool) {
        let transcript = Transcript::new_last_buffer("hello", Some(100), None, None).unwrap();
        let mut restored = false;
        let result = restore_result(&transcript, OutputMode::AutoPaste, outcome, || {
            restored = true;
            if works {
                Ok(())
            } else {
                Err(CommandError::new(
                    "clipboard_set_failed",
                    "clipboard write failed",
                ))
            }
        });
        (result, restored)
    }

    #[test]
    fn restores_the_clipboard_once_the_paste_was_read() {
        for outcome in [
            LoanOutcome::Read,
            LoanOutcome::NotRead,
            LoanOutcome::Unobserved,
        ] {
            let (result, restored) = restore_with(outcome, true);
            assert!(restored, "{outcome:?} must restore the clipboard");
            assert_eq!(result.status, OutputStatus::Completed);
            assert_eq!(
                result.clipboard_preservation,
                ClipboardPreservation::RestoredAfterPaste
            );
            // The clipboard was put back, so from the user's view nothing was kept.
            assert!(!result.copied);
            assert!(result.pasted);
            assert_eq!(result.clipboard_restored, Some(true));
            assert!(result.clipboard_restore_error.is_none());
        }
    }

    #[test]
    fn never_overwrites_something_copied_during_the_paste() {
        let (result, restored) = restore_with(LoanOutcome::Replaced, false);
        assert!(
            !restored,
            "a newer copy must not be replaced by the old clipboard"
        );
        assert_eq!(result.status, OutputStatus::Completed);
        assert!(!result.copied);
    }

    #[test]
    fn a_failed_restore_is_reported_honestly() {
        let (result, restored) = restore_with(LoanOutcome::Read, false);
        assert!(restored);
        // The paste still landed; only the restore failed, so the transcript
        // is still on the clipboard.
        assert_eq!(result.status, OutputStatus::ClipboardRestoreFailed);
        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::RestoreFailed
        );
        assert!(result.pasted);
        assert!(result.copied);
        assert_eq!(result.clipboard_restored, Some(false));
        assert_eq!(
            result.clipboard_restore_error.as_deref(),
            Some("clipboard write failed")
        );
    }

    // The output paths on a `FakeSystem`: nothing here types, pastes or
    // touches the clipboard of the machine the tests run on.

    /// A call an output path made to the system.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Sys {
        EnsureForeignFocus,
        PasteBlock,
        TypeText(String),
        SetPrivateClipboardText(String),
        SetClipboardText(String),
        SendPasteShortcut,
        SaveClipboardSnapshot,
        LendClipboardText(String),
        WaitForRead,
        RestoreClipboardSnapshot,
    }

    /// Records each call with how many entries the host's ordered log held
    /// when it came, so [`trace`] can place it among the events. It can fail
    /// one kind of call and hold a paste back.
    struct FakeSystem {
        host: Arc<RecordingHost>,
        calls: Mutex<Vec<(Sys, usize)>>,
        failing: Option<std::mem::Discriminant<Sys>>,
        block: Option<PasteBlock>,
    }

    impl FakeSystem {
        fn new(host: &Arc<RecordingHost>) -> Self {
            Self {
                host: host.clone(),
                calls: Mutex::new(Vec::new()),
                failing: None,
                block: None,
            }
        }

        /// Fails every call of the kind `call` names (its text is ignored).
        fn failing(mut self, call: Sys) -> Self {
            self.failing = Some(std::mem::discriminant(&call));
            self
        }

        fn blocking(mut self, block: PasteBlock) -> Self {
            self.block = Some(block);
            self
        }

        fn calls(&self) -> Vec<Sys> {
            let calls = self.calls.lock().unwrap();
            calls.iter().map(|(call, _)| call.clone()).collect()
        }

        fn record(&self, call: Sys) -> Result<(), CommandError> {
            let at = self.host.sequence().len();
            let fails = self.failing == Some(std::mem::discriminant(&call));
            let error = fails.then(|| failure_of(&call));
            self.calls.lock().unwrap().push((call, at));
            error.map_or(Ok(()), Err)
        }
    }

    fn failure_of(call: &Sys) -> CommandError {
        CommandError::new("system_failed", format!("{call:?} failed."))
    }

    impl OutputPlatform for FakeSystem {
        type Snapshot = ();
        type Loan = ();

        fn ensure_foreign_focus(&self) -> Result<(), CommandError> {
            self.record(Sys::EnsureForeignFocus)
        }

        fn paste_block(&self, _pinned: Option<PasteTarget>) -> Option<PasteBlock> {
            let _ = self.record(Sys::PasteBlock);
            self.block
        }

        fn type_text(&self, text: &str) -> Result<(), CommandError> {
            self.record(Sys::TypeText(text.to_string()))
        }

        fn set_private_clipboard_text(&self, text: &str) -> Result<(), CommandError> {
            self.record(Sys::SetPrivateClipboardText(text.to_string()))
        }

        fn set_clipboard_text(&self, text: &str) -> Result<(), CommandError> {
            self.record(Sys::SetClipboardText(text.to_string()))
        }

        fn send_paste_shortcut(&self) -> Result<(), CommandError> {
            self.record(Sys::SendPasteShortcut)
        }

        fn save_clipboard_snapshot(&self) {
            let _ = self.record(Sys::SaveClipboardSnapshot);
        }

        fn lend_clipboard_text(&self, text: &str) -> Result<(), CommandError> {
            self.record(Sys::LendClipboardText(text.to_string()))
        }

        fn wait_for_read(&self, _loan: (), _timeout: Duration) -> platform::LoanOutcome {
            let _ = self.record(Sys::WaitForRead);
            platform::LoanOutcome::Read
        }

        fn restore_clipboard_snapshot(&self, _snapshot: &()) -> Result<(), CommandError> {
            self.record(Sys::RestoreClipboardSnapshot)
        }
    }

    /// One ordered log of the host's events and calls and the system's
    /// calls: an app-state event shows its status, a system call its
    /// `Debug` text.
    fn trace(host: &RecordingHost, system: &FakeSystem) -> Vec<String> {
        let sequence = host.sequence();
        let calls = system.calls.lock().unwrap().clone();
        let mut calls = calls.into_iter().peekable();
        let mut out = Vec::new();
        for (index, recorded) in sequence.iter().enumerate() {
            while let Some((call, _)) = calls.next_if(|(_, at)| *at <= index) {
                out.push(format!("{call:?}"));
            }
            out.push(match recorded {
                Recorded::Event("scribe:app-state", payload) => {
                    format!("scribe:app-state {}", payload["status"].as_str().unwrap())
                }
                Recorded::Event(name, _) => name.to_string(),
                Recorded::Call(HostCall::Notify { title, .. }) => format!("notify {title}"),
                Recorded::Call(call) => format!("{call:?}"),
            });
        }
        out.extend(calls.map(|(call, _)| format!("{call:?}")));
        out
    }

    fn settings(output_mode: OutputMode, paste_method: PasteMethod) -> AppSettings {
        AppSettings {
            output_mode,
            paste_method,
            ..AppSettings::default()
        }
    }

    fn hello() -> Transcript {
        Transcript::new_last_buffer("hello", Some(100), None, None).unwrap()
    }

    /// The default paste after a dictation: the state shows Pasting around
    /// every step of the borrow (published through `emit_state_snapshot_with`),
    /// then Ready, then the outcome is announced.
    #[test]
    fn a_clipboard_paste_runs_between_pasting_and_ready() {
        let test = TestCore::new();
        to_ready(test.core.backend());
        let system = FakeSystem::new(&test.host);

        let result = handle_transcription_output_in(
            &test.core,
            &system,
            &hello(),
            &settings(OutputMode::AutoPaste, PasteMethod::ClipboardPaste),
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.status, OutputStatus::Completed);
        assert_eq!(
            result.clipboard_preservation,
            ClipboardPreservation::RestoredAfterPaste
        );
        assert_eq!(
            trace(&test.host, &system),
            [
                "scribe:output-started",
                "scribe:app-state Pasting",
                "EnsureForeignFocus",
                "PasteBlock",
                "SaveClipboardSnapshot",
                "LendClipboardText(\"hello\")",
                "SendPasteShortcut",
                "WaitForRead",
                "RestoreClipboardSnapshot",
                "scribe:app-state Ready",
                "scribe:output-completed",
            ]
        );
        assert_eq!(status(test.core.backend()), AppStatus::Ready);
        // Each state went to the status file as well as the event.
        let status_file =
            std::fs::read_to_string(test.core.paths().cache_dir().join("status.json")).unwrap();
        assert!(status_file.contains("\"Ready\""), "{status_file}");
        test.core.shutdown();
    }

    /// Every way a dictation's output can fail, on each of the three paths:
    /// the failure is announced exactly once, after the state is back to
    /// Ready, and the steps after the failed one never run.
    #[test]
    fn each_output_path_announces_a_failure_exactly_once() {
        let cases = [
            (
                settings(OutputMode::AutoPaste, PasteMethod::DirectInsert),
                Sys::TypeText(String::new()),
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "EnsureForeignFocus",
                    "PasteBlock",
                    "TypeText(\"hello\")",
                    "scribe:app-state Ready",
                    "scribe:output-failed",
                ],
            ),
            (
                settings(OutputMode::AutoPaste, PasteMethod::DirectInsert),
                Sys::EnsureForeignFocus,
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "EnsureForeignFocus",
                    "scribe:app-state Ready",
                    "scribe:output-failed",
                ],
            ),
            (
                // A paste that could not be sent puts the user's clipboard
                // back before it fails.
                settings(OutputMode::AutoPaste, PasteMethod::ClipboardPaste),
                Sys::SendPasteShortcut,
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "EnsureForeignFocus",
                    "PasteBlock",
                    "SaveClipboardSnapshot",
                    "LendClipboardText(\"hello\")",
                    "SendPasteShortcut",
                    "RestoreClipboardSnapshot",
                    "scribe:app-state Ready",
                    "scribe:output-failed",
                ],
            ),
            (
                settings(OutputMode::AutoPaste, PasteMethod::ClipboardPaste),
                Sys::LendClipboardText(String::new()),
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "EnsureForeignFocus",
                    "PasteBlock",
                    "SaveClipboardSnapshot",
                    "LendClipboardText(\"hello\")",
                    "scribe:app-state Ready",
                    "scribe:output-failed",
                ],
            ),
            (
                // A copy has no paste, so the state is left alone.
                settings(OutputMode::CopyClipboard, PasteMethod::ClipboardPaste),
                Sys::SetClipboardText(String::new()),
                vec![
                    "scribe:output-started",
                    "SetClipboardText(\"hello\")",
                    "scribe:output-failed",
                ],
            ),
            (
                settings(OutputMode::CopyAndPaste, PasteMethod::ClipboardPaste),
                Sys::SendPasteShortcut,
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "SetClipboardText(\"hello\")",
                    "EnsureForeignFocus",
                    "PasteBlock",
                    "SendPasteShortcut",
                    "scribe:app-state Ready",
                    "scribe:output-failed",
                ],
            ),
            (
                settings(OutputMode::CopyAndPaste, PasteMethod::ClipboardPaste),
                Sys::SetClipboardText(String::new()),
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "SetClipboardText(\"hello\")",
                    "scribe:app-state Ready",
                    "scribe:output-failed",
                ],
            ),
        ];

        for (settings, failing, expected) in cases {
            let test = TestCore::new();
            to_ready(test.core.backend());
            let system = FakeSystem::new(&test.host).failing(failing);
            let transcript = hello();

            let error = handle_transcription_output_in(&test.core, &system, &transcript, &settings)
                .unwrap_err();

            let failed_call = system.calls().last().cloned().unwrap();
            assert_eq!(error.code, "system_failed", "{settings:?}");
            assert_eq!(
                trace(&test.host, &system),
                expected,
                "{:?} {:?}",
                settings.output_mode,
                settings.paste_method
            );
            let failed = test
                .host
                .events()
                .into_iter()
                .filter(|event| event.name() == "scribe:output-failed")
                .map(|event| event.payload())
                .collect::<Vec<_>>();
            let reported = if failed_call == Sys::RestoreClipboardSnapshot {
                // The restore after a failed paste succeeded; the paste's
                // own error is the one reported.
                failure_of(&Sys::SendPasteShortcut)
            } else {
                failure_of(&failed_call)
            };
            assert_eq!(
                failed,
                [serde_json::json!({
                    "transcriptId": transcript.id,
                    "code": "system_failed",
                    "message": reported.message,
                })]
            );
            assert_eq!(status(test.core.backend()), AppStatus::Ready);
            test.core.shutdown();
        }
    }

    /// A paste Scribe held back leaves the text on the clipboard (privately
    /// for a paste, as asked for Copy + Paste) and is announced, then
    /// notified; nothing is typed or pasted.
    #[test]
    fn a_held_back_paste_is_copied_then_announced_then_notified() {
        let cases = [
            (
                settings(OutputMode::AutoPaste, PasteMethod::ClipboardPaste),
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "EnsureForeignFocus",
                    "PasteBlock",
                    "SetPrivateClipboardText(\"hello\")",
                    "scribe:app-state Ready",
                    "scribe:output-completed",
                    "notify Scribe didn't paste",
                ],
            ),
            (
                settings(OutputMode::CopyAndPaste, PasteMethod::ClipboardPaste),
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "SetClipboardText(\"hello\")",
                    "EnsureForeignFocus",
                    "PasteBlock",
                    "scribe:app-state Ready",
                    "scribe:output-completed",
                    "notify Scribe didn't paste",
                ],
            ),
        ];

        for (settings, expected) in cases {
            let test = TestCore::new();
            to_ready(test.core.backend());
            let system = FakeSystem::new(&test.host).blocking(PasteBlock::FocusChanged);

            let result = handle_transcription_output_in(&test.core, &system, &hello(), &settings)
                .unwrap()
                .unwrap();

            assert_eq!(result.status, OutputStatus::PasteSkipped);
            assert!(!result.pasted);
            assert_eq!(trace(&test.host, &system), expected, "{settings:?}");
            test.core.shutdown();
        }
    }

    /// The selected-text transform pastes with events off: whether the paste
    /// lands or fails, no outcome and no notification is sent. (Today it
    /// still sends `scribe:output-started`, which this pins.)
    #[test]
    fn the_transform_paste_announces_no_outcome() {
        for (failing, expected) in [
            (
                None,
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "EnsureForeignFocus",
                    "PasteBlock",
                    "SaveClipboardSnapshot",
                    "LendClipboardText(\"rewritten\")",
                    "SendPasteShortcut",
                    "WaitForRead",
                    "RestoreClipboardSnapshot",
                    "scribe:app-state Ready",
                ],
            ),
            (
                Some(Sys::EnsureForeignFocus),
                vec![
                    "scribe:output-started",
                    "scribe:app-state Pasting",
                    "EnsureForeignFocus",
                    "scribe:app-state Ready",
                ],
            ),
        ] {
            let test = TestCore::new();
            to_ready(test.core.backend());
            let system = FakeSystem::new(&test.host);
            let system = match failing.clone() {
                Some(call) => system.failing(call),
                None => system,
            };

            let result = paste_text_in(&test.core, &system, "rewritten");

            assert_eq!(result.is_err(), failing.is_some());
            assert_eq!(trace(&test.host, &system), expected, "{failing:?}");
            assert!(test.host.calls().is_empty(), "no notification");
            test.core.shutdown();
        }

        // Even a paste held back raises no notification.
        let test = TestCore::new();
        let system = FakeSystem::new(&test.host).blocking(PasteBlock::Elevated);
        let result = paste_text_in(&test.core, &system, "rewritten").unwrap();
        assert_eq!(result.status, OutputStatus::PasteSkipped);
        assert_eq!(
            trace(&test.host, &system),
            [
                "scribe:output-started",
                "EnsureForeignFocus",
                "PasteBlock",
                "SetPrivateClipboardText(\"rewritten\")",
            ]
        );
        test.core.shutdown();
    }

    /// Pasting or copying a transcript the user picked announces its
    /// outcome, and outside Ready the state is left alone.
    #[test]
    fn a_picked_transcript_is_pasted_or_copied_and_announced() {
        let test = TestCore::new();
        let transcript = hello();
        {
            let db = test.core.backend().db().unwrap();
            db.save_settings(&settings(OutputMode::AutoPaste, PasteMethod::DirectInsert))
                .unwrap();
            db.save_last_transcript(&transcript).unwrap();
        }
        let host = test.host.as_ref();
        let system = FakeSystem::new(&test.host);

        let pasted = paste_last_in(&test.core, &system).unwrap();
        let pasted_again = paste_saved_in(&test.core, &system, &transcript.id).unwrap();
        let copied = copy_last(test.core.backend(), host, &system).unwrap();
        let copied_again = copy_saved(test.core.backend(), host, &system, &transcript.id).unwrap();

        for result in [&pasted, &pasted_again] {
            assert_eq!(result.paste_method, Some(PasteMethod::DirectInsert));
            assert_eq!(
                result.clipboard_preservation,
                ClipboardPreservation::Untouched
            );
        }
        for result in [&copied, &copied_again] {
            assert_eq!(result.action, OutputAction::CopyClipboard);
            assert_eq!(
                result.clipboard_preservation,
                ClipboardPreservation::ReplacedWithTranscript
            );
        }
        let paste = [
            "scribe:output-started",
            "EnsureForeignFocus",
            "PasteBlock",
            "TypeText(\"hello\")",
            "scribe:output-completed",
        ];
        let copy = [
            "scribe:output-started",
            "SetClipboardText(\"hello\")",
            "scribe:output-completed",
        ];
        assert_eq!(
            trace(host, &system),
            [&paste[..], &paste[..], &copy[..], &copy[..]].concat()
        );
        assert_eq!(status(test.core.backend()), AppStatus::Idle);
        test.core.shutdown();
    }
}
