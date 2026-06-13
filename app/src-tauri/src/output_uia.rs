//! EXPERIMENT: clipboard-free, keystroke-free text insertion via Windows UI
//! Automation (UIA).
//!
//! This is a prototype for the "holy grail" insert: atomic like a paste, but it
//! never touches the clipboard and never synthesizes keystrokes. It drives the
//! focused control directly through the UIA client API.
//!
//! It is intentionally ADDITIVE — it does not modify or replace the existing
//! `output.rs` paste flow. The single entry point is [`insert_focused`], wired
//! to the `experimental_uia_insert` Tauri command so the owner can A/B test it.
//!
//! ## What UIA can and cannot do (the honest version)
//!
//! UIA is primarily an *accessibility inspection* API. Its writable surface for
//! text is far thinner than people hope:
//!
//! * **`ValuePattern::SetValue`** — the only true atomic text write. It sets the
//!   ENTIRE value of a control in one shot (no keystrokes, no clipboard). The
//!   catch: it *replaces* the whole control contents, it does NOT insert at the
//!   caret. For a single-line field that is empty (or that you mean to
//!   overwrite) this is perfect. For a partially-filled field or a multi-line
//!   document it would clobber existing text, so we only use it when it is safe.
//!
//! * **`TextPattern` / `TextPattern2`** — RICH but **read-only** from the
//!   client side. `IUIAutomationTextRange` exposes `GetText`, `Select`, `Move`,
//!   … but there is *no* `InsertText`/`SetText`/`ReplaceText` in the client
//!   interface (confirmed against `windows` 0.58). So TextPattern cannot perform
//!   the insert; we only use it to *probe* whether the element is a real text
//!   control and to read its current contents for the safety check above.
//!
//! The upshot: a pure-UIA atomic insert is reliable ONLY for value-backed
//! controls (classic Win32/WinForms/WPF edit boxes) and only when overwriting
//! the whole value is acceptable. Browsers, Electron, and many modern editors
//! either expose no `ValuePattern` or expose it read-only — there UIA must fall
//! back to keystrokes. `insert_focused` returns a typed, descriptive error in
//! every unsupported case so the caller can fall back cleanly.

#[cfg(windows)]
pub use platform::insert_focused;

#[cfg(not(windows))]
pub use stub::insert_focused;

/// How the focused element was driven (or why it could not be).
///
/// Returned to the UI so the experiment can be evaluated per app without
/// reading logs. `Skipped` means UIA bowed out and a keystroke fallback should
/// run; it is deliberately NOT an `Err` so a caller can branch on it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiaInsertOutcome {
    /// True when the text was actually inserted via UIA.
    pub inserted: bool,
    /// Stable machine code: `value_pattern`, `unsupported_no_value_pattern`,
    /// `unsupported_read_only`, `unsupported_unsafe_overwrite`, `no_focus`, …
    pub method: String,
    /// Best-effort UIA control type name of the focused element (e.g. `Edit`,
    /// `Document`, `Custom`) for the test matrix.
    pub control_type: Option<String>,
    /// Best-effort UIA Name of the focused element.
    pub element_name: Option<String>,
    /// Human-readable explanation, surfaced in the UI.
    pub message: String,
}

#[cfg(windows)]
mod platform {
    use super::UiaInsertOutcome;
    use crate::error::CommandError;
    use windows::core::BSTR;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern2,
        IUIAutomationValuePattern, UIA_TextPattern2Id, UIA_ValuePatternId,
    };

    /// RAII guard around COM apartment init so every early-return path still
    /// uninitializes. `CoInitializeEx` is refcounted, so pairing it per-call is
    /// safe even if the host thread already initialized COM elsewhere.
    struct ComGuard {
        owned: bool,
    }

    impl ComGuard {
        fn enter() -> Self {
            // STA is the correct apartment for UIA on a UI-adjacent thread.
            // S_FALSE means COM was already initialized on this thread — still a
            // success, and still balanced by a CoUninitialize.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            ComGuard {
                owned: hr.is_ok(),
            }
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.owned {
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Map a UIA control-type id to a short human label for the report. Covers
    /// the handful that matter for text entry; everything else shows its raw id.
    fn control_type_label(id: i32) -> String {
        // Values from UIA_CONTROLTYPE_ID (uiautomationcore).
        match id {
            50004 => "Edit".to_string(),
            50030 => "Document".to_string(),
            50000 => "Button".to_string(),
            50025 => "Pane".to_string(),
            50033 => "Custom".to_string(),
            50032 => "Group".to_string(),
            50026 => "Header".to_string(),
            other => format!("ControlType({other})"),
        }
    }

    /// Read best-effort diagnostics off the focused element. Never fails the
    /// insert — diagnostics are advisory.
    fn describe(element: &IUIAutomationElement) -> (Option<String>, Option<String>) {
        let control_type = unsafe { element.CurrentControlType() }
            .ok()
            .map(|ct| control_type_label(ct.0));
        let name = unsafe { element.CurrentName() }
            .ok()
            .map(|bstr| bstr.to_string())
            .filter(|s| !s.is_empty());
        (control_type, name)
    }

    /// Insert `text` into the currently focused control via UIA, with no
    /// clipboard use and no synthesized keystrokes.
    ///
    /// Returns `Ok(outcome)` whether or not UIA performed the insert: callers
    /// inspect `outcome.inserted` and fall back to keystrokes when it is false.
    /// `Err` is reserved for hard failures (COM/UIA could not be initialized,
    /// no element has focus) where there is nothing meaningful to fall back
    /// *from* yet — though even those are recoverable by the caller.
    pub fn insert_focused(text: &str) -> Result<UiaInsertOutcome, CommandError> {
        let _com = ComGuard::enter();

        // Create the UI Automation client object.
        let automation: IUIAutomation = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|e| {
                CommandError::new(
                    "uia_init_failed",
                    format!("Could not create the UI Automation client. {e}"),
                )
            })?
        };

        // The focused element is the control with keyboard focus *anywhere* on
        // the desktop — i.e. the foreign app the user was typing in, provided
        // focus has already been handed back to it (see the focus-handoff note
        // in the module docs / experiment report). UIA reads global focus, so
        // it does not need an HWND.
        let focused: IUIAutomationElement = unsafe {
            automation.GetFocusedElement().map_err(|e| {
                CommandError::new(
                    "uia_no_focus",
                    format!("UI Automation could not find a focused element. {e}"),
                )
            })?
        };

        let (control_type, element_name) = describe(&focused);

        // --- Probe TextPattern2 purely for diagnostics + the safety check. ---
        // TextPattern is read-only for clients, so it can never perform the
        // insert; we use it to learn whether this is a genuine text control and
        // (when ValuePattern is present) whether overwriting is safe.
        let text_pattern: Option<IUIAutomationTextPattern2> = unsafe {
            focused
                .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
                .ok()
        };

        // --- The only real write path: ValuePattern::SetValue. ---
        let value_pattern: Option<IUIAutomationValuePattern> = unsafe {
            focused
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()
        };

        let Some(value_pattern) = value_pattern else {
            // No ValuePattern => no atomic UIA write is possible. This is the
            // common case for browsers, Electron, terminals, and rich editors.
            return Ok(UiaInsertOutcome {
                inserted: false,
                method: "unsupported_no_value_pattern".to_string(),
                control_type,
                element_name,
                message: "Focused control exposes no UIA ValuePattern, so it \
                          cannot be written atomically via UI Automation. Fall \
                          back to keystrokes."
                    .to_string(),
            });
        };

        // A read-only value control must not be touched.
        if let Ok(read_only) = unsafe { value_pattern.CurrentIsReadOnly() } {
            if read_only.as_bool() {
                return Ok(UiaInsertOutcome {
                    inserted: false,
                    method: "unsupported_read_only".to_string(),
                    control_type,
                    element_name,
                    message: "Focused control's UIA ValuePattern is read-only. \
                              Fall back to keystrokes."
                        .to_string(),
                });
            }
        }

        // SAFETY CHECK: SetValue replaces the *entire* value. Only proceed when
        // doing so won't destroy text the user already had. We treat it as safe
        // when the current value is empty. If we cannot read the current value
        // (no TextPattern and ValuePattern.CurrentValue empty/unavailable), we
        // conservatively allow it only when the read returned an empty string.
        if let Some(existing) = current_value(&value_pattern, text_pattern.as_ref()) {
            if !existing.is_empty() {
                return Ok(UiaInsertOutcome {
                    inserted: false,
                    method: "unsupported_unsafe_overwrite".to_string(),
                    control_type,
                    element_name,
                    message: format!(
                        "Focused control already contains {} character(s); a UIA \
                         SetValue would replace the whole field, not insert at the \
                         caret. Skipped to avoid clobbering. Fall back to \
                         keystrokes (which insert at the caret).",
                        existing.chars().count()
                    ),
                });
            }
        }

        // Perform the atomic, clipboard-free, keystroke-free write. SetValue
        // takes a BSTR (Param<BSTR>); BSTR::from(&str) handles the UTF-16
        // conversion and length-prefix. `&value` satisfies the Param bound.
        let value = BSTR::from(text);
        unsafe {
            value_pattern.SetValue(&value).map_err(|e| {
                CommandError::new(
                    "uia_set_value_failed",
                    format!("UIA SetValue failed on the focused control. {e}"),
                )
            })?;
        }

        Ok(UiaInsertOutcome {
            inserted: true,
            method: "value_pattern".to_string(),
            control_type,
            element_name,
            message: "Inserted via UIA ValuePattern::SetValue (no clipboard, no \
                      keystrokes)."
                .to_string(),
        })
    }

    /// Best-effort read of the focused control's current text, used for the
    /// overwrite-safety check. Prefers TextPattern's document range (works even
    /// when ValuePattern.CurrentValue is unreliable), then ValuePattern.
    /// Returns `None` only when nothing could be read at all.
    fn current_value(
        value_pattern: &IUIAutomationValuePattern,
        text_pattern: Option<&IUIAutomationTextPattern2>,
    ) -> Option<String> {
        if let Some(tp) = text_pattern {
            if let Ok(range) = unsafe { tp.DocumentRange() } {
                // -1 = no length cap.
                if let Ok(bstr) = unsafe { range.GetText(-1) } {
                    return Some(bstr.to_string());
                }
            }
        }
        unsafe { value_pattern.CurrentValue() }
            .ok()
            .map(|bstr| bstr.to_string())
    }
}

#[cfg(not(windows))]
mod stub {
    use super::UiaInsertOutcome;
    use crate::error::CommandError;

    /// Non-Windows stub: UIA is a Windows-only API.
    pub fn insert_focused(_text: &str) -> Result<UiaInsertOutcome, CommandError> {
        Err(CommandError::new(
            "uia_insert_unsupported",
            "UI Automation insertion is implemented for Windows only.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::UiaInsertOutcome;

    #[test]
    fn outcome_serializes_camel_case() {
        let outcome = UiaInsertOutcome {
            inserted: true,
            method: "value_pattern".to_string(),
            control_type: Some("Edit".to_string()),
            element_name: None,
            message: "ok".to_string(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("\"controlType\":\"Edit\""));
        assert!(json.contains("\"inserted\":true"));
        assert!(json.contains("\"elementName\":null"));
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_stub_errors() {
        let err = super::insert_focused("hi").unwrap_err();
        assert_eq!(err.code, "uia_insert_unsupported");
    }
}
