//! Selected-text transform: the user highlights text in any app, gives Scribe
//! an instruction ("make this concise", "translate to Spanish", "fix grammar"),
//! and Scribe replaces the selection in place with the local-LLM-transformed
//! text. This turns Scribe into an inline AI editor.
//!
//! The flow has three pieces, kept deliberately separable so the LLM assembly
//! is unit-testable without any OS input:
//!   1. `capture_selection`: copy the current selection out of the focused app
//!      (Ctrl+C, or Ctrl+Shift+C in a console), remembering what was on the
//!      clipboard so it can be restored. The clipboard sequence number tells a
//!      real copy apart from "nothing was selected".
//!   2. `transform`: send (selection, instruction) to the local LLM via the
//!      reused `note_analysis::analyze_text` client and return the rewritten
//!      text.
//!   3. `apply_result`: paste the rewritten text back over the still-selected
//!      original (Ctrl+V, via the proven `output` paste path), then restore the
//!      pre-capture clipboard.
//!
//! Selection capture/apply touch the Win32 clipboard + SendInput and are
//! Windows-only, mirroring the rest of the app's input code. The LLM assembly
//! (`build_messages`) and the round-trip (`transform`) are platform-independent.

use std::time::Duration;

use crate::error::CommandError;
use crate::settings::AppSettings;

#[cfg(windows)]
use crate::output::OutputResult;
#[cfg(windows)]
use tauri::AppHandle;

/// A transform is an interactive edit, so a stalled local server must not hold
/// the user's selection hostage for minutes. Notes analysis keeps its longer
/// budget in `note_analysis`.
pub const TRANSFORM_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the target app gets to publish the copied selection, and how often
/// the clipboard sequence number is checked while waiting.
#[cfg(windows)]
const COPY_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(windows)]
const COPY_POLL: Duration = Duration::from_millis(15);
/// Some apps publish the copy in several steps (text first, then rich
/// formats); a short settle after the first change lets them finish.
#[cfg(windows)]
const COPY_SETTLE: Duration = Duration::from_millis(30);

/// Window classes of console hosts, where a bare Ctrl+C with nothing selected
/// interrupts the running program instead of copying. Those get Ctrl+Shift+C.
const CONSOLE_WINDOW_CLASSES: [&str; 3] = [
    "ConsoleWindowClass",
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "mintty",
];

/// Transform Selection rides on the Local AI connection; with that switched
/// off, no selection may be copied or sent anywhere.
pub fn ensure_enabled(settings: &AppSettings) -> Result<(), CommandError> {
    if settings.notes_analysis_enabled {
        Ok(())
    } else {
        Err(CommandError::new(
            "local_ai_off",
            "Turn on Local AI in Settings to use Transform selection.",
        ))
    }
}

/// True when the window class belongs to a console host (see
/// `CONSOLE_WINDOW_CLASSES`).
pub fn is_console_window_class(class_name: &str) -> bool {
    CONSOLE_WINDOW_CLASSES.contains(&class_name)
}

/// Waits for the clipboard sequence number to move past `before`, checking
/// every `poll` for at most `timeout`. Returns false when it never changed,
/// which means the copy put nothing on the clipboard (nothing was selected).
/// `read` and `sleep` are injected so the logic is testable without Win32.
pub fn wait_for_sequence_change(
    mut read: impl FnMut() -> u32,
    before: u32,
    timeout: Duration,
    poll: Duration,
    mut sleep: impl FnMut(Duration),
) -> bool {
    let attempts = (timeout.as_millis() / poll.as_millis().max(1)).max(1);
    for _ in 0..attempts {
        sleep(poll);
        if read() != before {
            return true;
        }
    }
    false
}

/// Whether the transformed text may be pasted into the current foreground
/// window. The window the selection came from is fine; so is one of Scribe's
/// own windows (the user clicked the pill to stop), because the paste focus
/// guard then returns focus to the app underneath. Any other app means the
/// user moved on, and pasting there would drop text into the wrong place.
pub fn may_paste_into(foreground: isize, source: isize, foreground_is_own: bool) -> bool {
    foreground == source || foreground_is_own
}

/// System prompt that pins the model to a pure text editor: output the
/// transformed text and nothing else. Kept terse on purpose: small local
/// models follow short, blunt instructions best.
const SYSTEM_PROMPT: &str = "You are a precise text editor. Apply the user's instruction to the text below. Output ONLY the transformed text, with no preamble, no explanations, no quotes.";

/// Assembles the (system, user) chat messages for a transform. Pure and
/// unit-tested: the instruction and the selection are laid out so the model
/// can't confuse the two (explicit `Instruction:` / `Text:` labels with a
/// separator), which matters most for tiny local models.
pub fn build_messages(selection: &str, instruction: &str) -> (String, String) {
    let system = SYSTEM_PROMPT.to_string();
    let user = format!(
        "Instruction: {instruction}\n\n---\nText:\n{selection}",
        instruction = instruction.trim(),
        selection = selection
    );
    (system, user)
}

/// Runs one transform against the local LLM and returns the trimmed result.
///
/// Reuses `note_analysis::analyze_text` (the same OpenAI-compatible client the
/// Notes feature uses) so there is a single LLM code path and no new
/// endpoint/model settings: the caller passes the existing
/// `notes_analysis_endpoint` / `notes_analysis_model`.
///
/// Errors when the selection or instruction is empty (nothing to do), or when
/// the LLM call itself fails.
pub fn transform(
    selection: &str,
    instruction: &str,
    endpoint: &str,
    model: &str,
) -> Result<String, CommandError> {
    if selection.trim().is_empty() {
        return Err(CommandError::new(
            "selection_empty",
            "No text was selected. Highlight some text first, then transform it.",
        ));
    }
    if instruction.trim().is_empty() {
        return Err(CommandError::new(
            "instruction_empty",
            "No instruction was given. Say or type what to do with the selected text.",
        ));
    }

    let (system, user) = build_messages(selection, instruction);
    let outcome = crate::note_analysis::analyze_text_with_timeout(
        endpoint,
        model,
        &system,
        &user,
        TRANSFORM_TIMEOUT,
    )?;
    Ok(outcome.analysis.trim().to_string())
}

/// The original selection plus the clipboard contents that were displaced to
/// capture it. Dropping it without `settle` puts the user's clipboard back, so
/// every path that abandons a transform (LLM failure, discard, a too-short
/// instruction, an empty selection) restores the clipboard without having to
/// remember to.
#[cfg(windows)]
pub struct CapturedSelection {
    pub selection: String,
    /// Raw, full-fidelity snapshot of every clipboard format that was present
    /// before we copied the selection (text, image, files, ...).
    prev_clipboard: Vec<(u32, Vec<u8>)>,
    /// The foreground window the selection was copied from (an HWND value).
    source_window: isize,
    restore_on_drop: bool,
}

#[cfg(windows)]
impl CapturedSelection {
    /// Drops the capture without touching the clipboard.
    pub fn forget(mut self) {
        self.restore_on_drop = false;
    }

    /// Puts the pre-capture clipboard back now and disarms the drop restore.
    fn restore_now(&mut self) {
        self.restore_on_drop = false;
        if let Err(error) = crate::output::restore_clipboard_snapshot(&self.prev_clipboard) {
            log::warn!(
                "Could not restore the clipboard after a selection transform: {}",
                error.message
            );
        }
    }
}

#[cfg(windows)]
impl Drop for CapturedSelection {
    fn drop(&mut self) {
        if self.restore_on_drop {
            self.restore_now();
        }
    }
}

/// Copies the current selection out of the focused app and returns it together
/// with a snapshot of the previous clipboard. Returns None when nothing was
/// selected: the copy left the clipboard sequence number unchanged, so the
/// clipboard was never touched and there is nothing to restore.
///
/// Sequence: snapshot the clipboard -> synthesize the copy shortcut (Ctrl+C,
/// or Ctrl+Shift+C in a console) -> wait for the clipboard sequence number to
/// change -> read the clipboard back as the selection.
#[cfg(windows)]
pub fn capture_selection() -> Result<Option<CapturedSelection>, CommandError> {
    use std::thread;

    let source_window = platform::foreground_window();
    let console = platform::is_console_window(source_window);

    // Snapshot everything currently on the clipboard so the user's prior
    // clipboard survives the round-trip.
    let prev_clipboard = crate::output::save_clipboard_snapshot();
    let before = platform::clipboard_sequence_number();

    platform::send_copy_shortcut(console)?;

    let copied = wait_for_sequence_change(
        platform::clipboard_sequence_number,
        before,
        COPY_TIMEOUT,
        COPY_POLL,
        thread::sleep,
    );
    if !copied {
        log::info!("Transform selection: the copy changed nothing, so no text is selected.");
        return Ok(None);
    }
    thread::sleep(COPY_SETTLE);

    Ok(Some(CapturedSelection {
        selection: read_clipboard_text().unwrap_or_default(),
        prev_clipboard,
        source_window,
        restore_on_drop: true,
    }))
}

/// Brings back the app the user was in before Scribe's own window took focus,
/// so the typed Dashboard panel copies from that app instead of from Scribe.
#[cfg(windows)]
pub fn focus_previous_app() -> Result<(), CommandError> {
    crate::output::focus_previous_app()
}

/// Pastes `text` over the still-selected original and then puts the
/// pre-capture clipboard back.
///
/// When the user has moved to another app while the model was working, the
/// text is left on the clipboard instead (the error says so), because pasting
/// into whatever is in front now would drop it in the wrong place.
#[cfg(windows)]
pub fn apply_result(
    app: &AppHandle,
    text: &str,
    mut captured: CapturedSelection,
) -> Result<OutputResult, CommandError> {
    let foreground = platform::foreground_window();
    if !may_paste_into(
        foreground,
        captured.source_window,
        platform::is_own_window(foreground),
    ) {
        // Private like every clipboard write Scribe makes on the user's
        // behalf: kept out of Clipboard History and the cloud clipboard.
        crate::output::set_private_clipboard_text(text)?;
        // The rewritten text must stay on the clipboard for the user to paste.
        captured.restore_on_drop = false;
        return Err(CommandError::new(
            "transform_target_changed",
            "You switched windows while Scribe was rewriting. The result is on your clipboard: press Ctrl+V to paste it.",
        ));
    }

    // Paste the transformed text over the highlighted selection (capture only
    // sent a copy, so the selection is still active). `paste_text` handles the
    // focus guard and modifier release. On failure, dropping `captured`
    // restores the clipboard.
    let result = crate::output::paste_text(app, text)?;
    if result.status == crate::output::OutputStatus::PasteSkipped {
        // Windows would have blocked the paste (an administrator window), so
        // the result was left on the clipboard. Keep it there and say so.
        captured.restore_on_drop = false;
        return Err(CommandError::new("transform_paste_blocked", result.message));
    }

    // `paste_text` restored whatever it borrowed; for the transform flow the
    // meaningful thing to preserve is the clipboard from before the capture.
    captured.restore_now();

    Ok(result)
}

/// Reads the current clipboard as text (arboard). Returns None when the
/// clipboard holds no text or could not be read.
#[cfg(windows)]
fn read_clipboard_text() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok()
}

#[cfg(windows)]
mod platform {
    use crate::error::CommandError;
    use std::mem::size_of;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_C, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
        VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
    };

    /// Same modifier set the paste path force-releases: every key whose held
    /// state would otherwise combine with the injected Ctrl+C.
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

    /// Force-release any physically-held modifier so the synthetic Ctrl+C
    /// can't be scrambled by a still-held trigger chord (e.g. Ctrl+Alt+R).
    /// Mirrors `output::release_held_modifiers`.
    fn release_held_modifiers() -> Result<(), CommandError> {
        let releases: Vec<INPUT> = MODIFIER_KEYS
            .iter()
            .filter(|vk| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16) & 0x8000 != 0)
            .map(|vk| keyboard_input(*vk, KEYEVENTF_KEYUP))
            .collect();

        if releases.is_empty() {
            return Ok(());
        }
        send_inputs(&releases, "modifier release")
    }

    /// Synthesizes the copy shortcut for the current selection: Ctrl+C, or
    /// Ctrl+Shift+C in a console, where a bare Ctrl+C interrupts the running
    /// program when nothing is selected. Releases held modifiers first so the
    /// chord that triggered the transform doesn't fuse with the injected C.
    pub fn send_copy_shortcut(console: bool) -> Result<(), CommandError> {
        release_held_modifiers()?;

        let inputs: Vec<INPUT> = if console {
            vec![
                keyboard_input(VK_CONTROL, Default::default()),
                keyboard_input(VK_SHIFT, Default::default()),
                keyboard_input(VK_C, Default::default()),
                keyboard_input(VK_C, KEYEVENTF_KEYUP),
                keyboard_input(VK_SHIFT, KEYEVENTF_KEYUP),
                keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
            ]
        } else {
            vec![
                keyboard_input(VK_CONTROL, Default::default()),
                keyboard_input(VK_C, Default::default()),
                keyboard_input(VK_C, KEYEVENTF_KEYUP),
                keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
            ]
        };
        send_inputs(&inputs, "copy shortcut")
    }

    /// The clipboard's change counter; it moves on every write by any app.
    pub fn clipboard_sequence_number() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    /// The current foreground window as a plain HWND value.
    pub fn foreground_window() -> isize {
        unsafe { GetForegroundWindow() }.0 as isize
    }

    /// True when the window belongs to this process (the main window or pill).
    pub fn is_own_window(hwnd: isize) -> bool {
        if hwnd == 0 {
            return false;
        }
        let mut process_id = 0u32;
        unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut process_id)) };
        process_id == unsafe { GetCurrentProcessId() }
    }

    /// True when the window is a console host (see `CONSOLE_WINDOW_CLASSES`).
    pub fn is_console_window(hwnd: isize) -> bool {
        if hwnd == 0 {
            return false;
        }
        let mut buffer = [0u16; 128];
        let length = unsafe { GetClassNameW(HWND(hwnd as *mut _), &mut buffer) };
        if length <= 0 {
            return false;
        }
        let class_name = String::from_utf16_lossy(&buffer[..length as usize]);
        super::is_console_window_class(&class_name)
    }

    fn keyboard_input(
        virtual_key: VIRTUAL_KEY,
        flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
    ) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    wScan: 0,
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
                "selection_copy_failed",
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn build_messages_labels_instruction_and_text() {
        let (system, user) = build_messages("Hello world", "make it uppercase");

        // The system prompt pins the model to "output only the transformed text".
        assert!(system.contains("precise text editor"));
        assert!(system.contains("ONLY the transformed text"));

        // The user message keeps the instruction and the text clearly separated.
        assert_eq!(
            user,
            "Instruction: make it uppercase\n\n---\nText:\nHello world"
        );
    }

    #[test]
    fn build_messages_trims_the_instruction_but_preserves_the_selection() {
        // Leading/trailing whitespace on a spoken/typed instruction is noise;
        // the selection is preserved verbatim (its own whitespace can matter).
        let (_system, user) = build_messages("  keep my spaces  ", "  fix grammar  ");

        assert!(user.starts_with("Instruction: fix grammar\n\n"));
        assert!(user.ends_with("Text:\n  keep my spaces  "));
    }

    #[test]
    fn ensure_enabled_requires_local_ai() {
        let mut settings = AppSettings {
            notes_analysis_enabled: false,
            ..Default::default()
        };
        let error = ensure_enabled(&settings).unwrap_err();
        assert_eq!(error.code, "local_ai_off");
        assert!(error.message.contains("Turn on Local AI"));

        settings.notes_analysis_enabled = true;
        assert!(ensure_enabled(&settings).is_ok());
    }

    #[test]
    fn console_hosts_are_recognized_by_window_class() {
        assert!(is_console_window_class("ConsoleWindowClass"));
        assert!(is_console_window_class("CASCADIA_HOSTING_WINDOW_CLASS"));
        assert!(is_console_window_class("mintty"));
        assert!(!is_console_window_class("Chrome_WidgetWin_1"));
        assert!(!is_console_window_class("Notepad"));
        assert!(!is_console_window_class(""));
    }

    #[test]
    fn sequence_change_is_detected_on_a_later_poll() {
        let mut reads = vec![7, 7, 8].into_iter();
        let mut slept = Vec::new();
        let changed = wait_for_sequence_change(
            || reads.next().unwrap_or(8),
            7,
            Duration::from_millis(500),
            Duration::from_millis(15),
            |delay| slept.push(delay),
        );
        assert!(changed);
        assert_eq!(slept.len(), 3);
    }

    #[test]
    fn unchanged_sequence_means_nothing_was_selected() {
        let mut polls = 0;
        let changed = wait_for_sequence_change(
            || 42,
            42,
            Duration::from_millis(500),
            Duration::from_millis(15),
            |_| polls += 1,
        );
        assert!(!changed);
        // 500 ms at 15 ms per poll: the wait is bounded, not endless.
        assert_eq!(polls, 33);
    }

    #[test]
    fn sequence_wait_polls_at_least_once_for_tiny_timeouts() {
        let mut polls = 0;
        let changed = wait_for_sequence_change(
            || 2,
            1,
            Duration::from_millis(1),
            Duration::from_millis(15),
            |_| polls += 1,
        );
        assert!(changed);
        assert_eq!(polls, 1);
    }

    #[test]
    fn paste_goes_to_the_source_window_or_through_scribe() {
        // The window the selection came from.
        assert!(may_paste_into(100, 100, false));
        // One of Scribe's own windows (the pill was clicked to stop).
        assert!(may_paste_into(200, 100, true));
        // The user moved to another app: keep the result on the clipboard.
        assert!(!may_paste_into(300, 100, false));
    }

    #[test]
    fn transform_rejects_empty_selection() {
        let error = transform("   ", "fix grammar", "http://127.0.0.1:1/v1", "m").unwrap_err();
        assert_eq!(error.code, "selection_empty");
    }

    #[test]
    fn transform_rejects_empty_instruction() {
        let error = transform("some text", "  ", "http://127.0.0.1:1/v1", "m").unwrap_err();
        assert_eq!(error.code, "instruction_empty");
    }

    /// One-shot OpenAI-compatible mock, mirroring the helper in
    /// `note_analysis::tests`: answers each connection in order and captures the
    /// request line + body so the assembled prompt can be asserted.
    fn mock_server(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());

        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 65536];
                let mut request = Vec::new();
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(headers_end) = text.find("\r\n\r\n") {
                        let content_length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= headers_end + 4 + content_length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let payload = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                stream.write_all(payload.as_bytes()).unwrap();
            }
            requests
        });

        (endpoint, handle)
    }

    fn completion_response(content: &str) -> String {
        serde_json::json!({
            "model": "test-model",
            "choices": [{ "message": { "role": "assistant", "content": content } }],
        })
        .to_string()
    }

    #[test]
    fn transform_round_trips_selection_and_instruction_and_trims() {
        let (endpoint, handle) = mock_server(vec![completion_response("  HELLO WORLD  \n")]);

        let result = transform(
            "hello world",
            "make it uppercase",
            &format!("{}/", endpoint), // trailing slash must be tolerated
            "my-model",
        )
        .unwrap();

        // The result is trimmed of the model's surrounding whitespace.
        assert_eq!(result, "HELLO WORLD");

        // The request carried the system prompt, the instruction, and the
        // selection text in the assembled message.
        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("POST /v1/chat/completions"));
        assert!(requests[0].contains("precise text editor"));
        assert!(requests[0].contains("make it uppercase"));
        assert!(requests[0].contains("hello world"));
    }

    #[test]
    fn transform_surfaces_llm_errors() {
        // A port from the dynamic range with nothing listening: the underlying
        // analyze_text error (mentioning the endpoint) propagates out.
        let error = transform("text", "instruction", "http://127.0.0.1:59998/v1", "m").unwrap_err();
        assert!(error.to_string().contains("127.0.0.1:59998"));
    }
}
