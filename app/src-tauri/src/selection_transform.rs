//! Selected-text transform: the user highlights text in any app, gives Scribe
//! an instruction ("make this concise", "translate to Spanish", "fix grammar"),
//! and Scribe replaces the selection in place with the local-LLM-transformed
//! text. This turns Scribe into an inline AI editor.
//!
//! The flow has three pieces, kept deliberately separable so the LLM assembly
//! is unit-testable without any OS input:
//!   1. `capture_selection` — copy the current selection out of the focused app
//!      (Ctrl+C), remembering what was on the clipboard so it can be restored.
//!   2. `transform` — send (selection, instruction) to the local LLM via the
//!      reused `note_analysis::analyze_text` client and return the rewritten
//!      text.
//!   3. `apply_result` — paste the rewritten text back over the still-selected
//!      original (Ctrl+V, via the proven `output` paste path), then restore the
//!      pre-capture clipboard.
//!
//! Selection capture/apply touch the Win32 clipboard + SendInput and are
//! Windows-only, mirroring the rest of the app's input code. The LLM assembly
//! (`build_messages`) and the round-trip (`transform`) are platform-independent.

use crate::error::CommandError;

#[cfg(windows)]
use crate::output::OutputResult;
#[cfg(windows)]
use tauri::AppHandle;

/// System prompt that pins the model to a pure text editor: output the
/// transformed text and nothing else. Kept terse on purpose — small local
/// models follow short, blunt instructions best.
const SYSTEM_PROMPT: &str = "You are a precise text editor. Apply the user's instruction to the text below. Output ONLY the transformed text — no preamble, no explanations, no quotes.";

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
    let outcome = crate::note_analysis::analyze_text(endpoint, model, &system, &user)?;
    Ok(outcome.analysis.trim().to_string())
}

/// The original selection plus the clipboard contents that were displaced to
/// capture it, so `apply_result` can put the user's clipboard back afterwards.
#[cfg(windows)]
pub struct CapturedSelection {
    pub selection: String,
    /// Raw, full-fidelity snapshot of every clipboard format that was present
    /// before we copied the selection (text, image, files, ...). Restored
    /// verbatim after the transformed text is pasted.
    prev_clipboard: Vec<(u32, Vec<u8>)>,
}

/// Copies the current selection out of the focused app and returns it together
/// with a snapshot of the previous clipboard.
///
/// Sequence: snapshot the clipboard (so it can be restored) -> synthesize
/// Ctrl+C -> wait briefly for the target app to publish the copied text ->
/// read the clipboard back as the selection.
#[cfg(windows)]
pub fn capture_selection() -> Result<CapturedSelection, CommandError> {
    use std::{thread, time::Duration};

    // Snapshot everything currently on the clipboard so the user's prior
    // clipboard survives the round-trip. An empty snapshot (capture failed /
    // nothing there) just means "nothing to restore".
    let prev_clipboard = crate::output::save_clipboard_snapshot();

    // Copy the selection. Force-release any modifiers still physically held
    // from the trigger chord first, so the synthetic Ctrl+C lands clean
    // (mirrors the paste path's modifier handling).
    platform::send_copy_shortcut()?;

    // Let the target app put the selection on the clipboard before we read it.
    thread::sleep(Duration::from_millis(120));

    let selection = read_clipboard_text().unwrap_or_default();

    Ok(CapturedSelection {
        selection,
        prev_clipboard,
    })
}

/// Pastes `text` over the still-selected original (replacing it) and then,
/// when `restore` is true, puts the pre-capture clipboard back.
///
/// The paste itself reuses `output::paste_text` — the same borrow-the-clipboard,
/// focus-guard, send-Ctrl+V machinery used for normal dictation output — so the
/// transformed text lands as a single atomic paste over the selection. The
/// clipboard restore here layers the user's *original* (pre-capture) clipboard
/// back on top, overriding `paste_text`'s own transient restore.
#[cfg(windows)]
pub fn apply_result(
    app: &AppHandle,
    text: &str,
    captured: &CapturedSelection,
    restore: bool,
) -> Result<OutputResult, CommandError> {
    // Paste the transformed text. This replaces the highlighted selection in
    // the focused app (the selection is still active because capture only sent
    // Ctrl+C). `paste_text` handles the foreign-focus guard and modifier
    // release internally.
    let result = crate::output::paste_text(app, text)?;

    if restore {
        // Put the user's ORIGINAL clipboard back. `paste_text` already restored
        // whatever it borrowed, but for the transform flow the meaningful thing
        // to preserve is what the user had before the Ctrl+C capture, so restore
        // that snapshot last. A failure here is non-fatal: the paste succeeded.
        if let Err(error) = crate::output::restore_clipboard_snapshot(&captured.prev_clipboard) {
            log::warn!(
                "Selection transform pasted, but could not restore the pre-capture clipboard: {}",
                error.message
            );
        }
    }

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
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_C, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
        VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
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

    /// Synthesizes Ctrl+C to copy the current selection. Releases held
    /// modifiers first so the chord that triggered the transform doesn't fuse
    /// with the injected C.
    pub fn send_copy_shortcut() -> Result<(), CommandError> {
        release_held_modifiers()?;

        let inputs = [
            keyboard_input(VK_CONTROL, Default::default()),
            keyboard_input(VK_C, Default::default()),
            keyboard_input(VK_C, KEYEVENTF_KEYUP),
            keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        send_inputs(&inputs, "copy shortcut")
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
