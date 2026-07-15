# Experiment: clipboard-free, keystroke-free insert via Windows UI Automation

> **Historical experiment:** This prototype is retained for research context and is not the current insertion implementation.
> Current behavior is documented in the [root README](../README.md).

**Status:** prototype, opt-in only. Not wired into the normal output flow.
**Verdict (short version):** **Partial. Keep as opt-in; do NOT make it the
default.** A pure-UIA atomic insert is reliable only for a narrow class of
controls and, even there, it *replaces the whole field* rather than inserting at
the caret. It cannot replace keystroke insertion as the general path. Its honest
role is a **first-tier fast path with a mandatory keystroke fallback**, and even
that is only worth shipping if the owner's manual matrix shows it winning in apps
they actually dictate into.

---

## 1. Goal

Prototype the "holy grail" insert: atomic like a paste, but it **never touches
the clipboard** and **never synthesizes keystrokes**. Drive the focused control
directly via the Windows UI Automation (UIA) client API, and deliver an honest
reliability assessment.

## 2. What was built (additive, low-conflict)

Nothing in the existing paste/output flow was changed. New, isolated surface:

| File | Change |
| --- | --- |
| `app/src-tauri/src/output_uia.rs` | **New module.** `#[cfg(windows)]` UIA insert + `#[cfg(not(windows))]` stub. Single entry point `insert_focused(&str) -> Result<UiaInsertOutcome>`. |
| `app/src-tauri/Cargo.toml` | Added one `windows` feature: **`Win32_UI_Accessibility`** (the UIA namespace). No new crates. |
| `app/src-tauri/src/output.rs` | Added a thin `pub fn ensure_foreign_focus()` wrapper over the existing private `platform::ensure_foreign_focus`, so the experiment reuses the exact same focus-handoff. No existing function changed. |
| `app/src-tauri/src/commands.rs` | New Tauri command `experimental_uia_insert(text: String)`. Hands focus back to the foreign app, then calls `output_uia::insert_focused`. |
| `app/src-tauri/src/lib.rs` | Registered `pub mod output_uia;` and the command in `invoke_handler`. |
| `app/src/backend.ts` | Exported `UiaInsertOutcome` type + `experimentalUiaInsert(text)` invoke wrapper. |

### `windows` crate features / deps added
- Crate: `windows` **0.58** (already in use; no version change).
- **Feature added:** `Win32_UI_Accessibility`.
- COM (`Win32_System_Com`) and `Win32_Foundation` were already enabled.
- **No new third-party dependencies.**

## 3. Approach and the UIA patterns/interfaces used

Flow inside `insert_focused`:

1. **COM init** — `CoInitializeEx(COINIT_APARTMENTTHREADED)`, wrapped in an RAII
   `ComGuard` so every early return still calls `CoUninitialize`. STA is the
   correct apartment for UIA on a UI-adjacent thread; `S_FALSE` (COM already
   initialized) is treated as success and still balanced.
2. **Create the client** — `CoCreateInstance(CUIAutomation)` → `IUIAutomation`.
3. **Find the target** — `IUIAutomation::GetFocusedElement()` →
   `IUIAutomationElement`. UIA reads **global keyboard focus**, so it needs no
   HWND; this is why the focus-handoff (below) must run first.
4. **Diagnose** — read `CurrentControlType` and `CurrentName` for the report /
   test matrix (advisory; never fails the insert).
5. **Probe `TextPattern2`** (`UIA_TextPattern2Id` →
   `IUIAutomationTextPattern2`) — used **only** to read existing contents via
   `DocumentRange().GetText(-1)` for the safety check. See the limitation below:
   TextPattern cannot write.
6. **Write via `ValuePattern`** (`UIA_ValuePatternId` →
   `IUIAutomationValuePattern`):
   - If the element exposes no `ValuePattern` → return
     `unsupported_no_value_pattern` (caller falls back to keystrokes).
   - If `CurrentIsReadOnly()` is true → `unsupported_read_only`.
   - **Overwrite-safety gate:** if the control already has non-empty text →
     `unsupported_unsafe_overwrite` (because `SetValue` replaces the *entire*
     value, it would clobber existing text and the user's caret position).
   - Otherwise call `SetValue(&BSTR::from(text))` — the atomic, clipboard-free,
     keystroke-free write — and return `value_pattern { inserted: true }`.

The function returns `Ok(outcome)` with `inserted: false` (not `Err`) for every
"UIA can't safely do this" case, so a caller can branch to a keystroke fallback
without treating it as a hard failure. `Err` is reserved for COM/UIA init
failure and no-focus.

### Focus-handoff strategy (targeting the foreign window)

The insert must land in the app the user was typing in **before** Scribe took
focus — never in a Scribe window. We reuse the existing, battle-tested handoff
from `output.rs`:

- `output::ensure_foreign_focus()` checks whether the foreground window belongs
  to **our** process. If so, it walks **down the Z-order** (`GetWindow` /
  `GW_HWNDNEXT`) to find the first real foreign top-level window (visible, not a
  tool window, has a title, not DWM-cloaked) and calls `SetForegroundWindow` on
  it, then sleeps ~100 ms so it actually takes keyboard focus.
- Only **after** focus is restored does `GetFocusedElement` resolve to the
  foreign control. The command runs the handoff first, exactly like the paste
  path, so behavior is identical to today's "paste into the previous app."

## 4. The critical limitations discovered (the honest part)

These shaped the verdict and were verified directly against the `windows` 0.58
bindings, not assumed.

1. **UIA `TextPattern` is read-only from the client.** The client-side
   `IUIAutomationTextRange` exposes `GetText`, `Select`, `Move`,
   `ExpandToEnclosingUnit`, … but there is **no** `InsertText` / `SetText` /
   `ReplaceText` anywhere in the client API. So the rich, caret-aware
   TextPattern **cannot perform the insert at all**. (Inserting via text ranges
   only exists on the *provider* side, i.e. inside the target app's own UIA
   implementation — not available to an external client like us.)

2. **`ValuePattern::SetValue` replaces the entire control value.** It is the
   only true atomic UIA text write available to a client, but it is
   **whole-value replace, not caret insert**. For an empty field this is exactly
   what we want. For a partially-typed field or any multi-line document, it
   would wipe existing content and ignore the caret — unacceptable for a
   dictation tool. Our overwrite-safety gate refuses that case and defers to
   keystrokes. This is the single biggest reason UIA cannot be the general
   default.

3. **`ValuePattern` coverage is narrow and inconsistent.** Classic Win32 /
   WinForms / WPF single-line edits usually expose a writable `ValuePattern`.
   Multi-line documents, browsers' `contenteditable`/`<textarea>`, Electron
   apps, and terminals frequently expose **no** `ValuePattern`, or expose it
   **read-only**, or expose it but ignore/limit `SetValue`. There is no reliable
   way to know in advance without trying.

4. **`SetValue` may not fire the events apps expect.** Some apps update their
   model only on keystroke/`WM_CHAR`/input events; a programmatic `SetValue` can
   land visually but leave the app's internal state (validation, change
   handlers, autosave) unaware. This is app-specific and only the manual matrix
   can reveal it.

Net: the genuinely-atomic, clipboard-free, keystroke-free UIA insert works for
**empty value-backed edit controls** and degrades to "skip, fall back to
keystrokes" everywhere else.

## 5. Windows manual-test matrix (for the owner to run)

Run each row with the experimental command (e.g. from the dev console:
`window.__TAURI__` is not exposed; instead temporarily call
`experimentalUiaInsert("UIA insert test 123")` from app code, or invoke the
command directly). For each app: **(a)** put the caret in an EMPTY field and run
it, then **(b)** repeat with the field already containing some text. Record
`inserted`, `method`, and `controlType` from the returned `UiaInsertOutcome`.

| # | App / target | Expectation (empty field) | Why |
| --- | --- | --- | --- |
| 1 | **Notepad** (Win11 edit area) | **Works** (`value_pattern`, Document/Edit) | Classic edit control; modern Notepad exposes a writable ValuePattern. Confirm whether multi-line existing text triggers `unsupported_unsafe_overwrite`. |
| 2 | **WordPad** | **Partial** | Rich edit; may expose ValuePattern read-only or not at all. Likely `unsupported_*` → keystroke fallback. |
| 3 | **VS Code** | **Unsupported** | Electron/Monaco editor; the editor surface is a custom canvas with no writable ValuePattern. Expect `unsupported_no_value_pattern`. (The Quick Open / find boxes may differ — worth a side test.) |
| 4 | **Windows Terminal** | **Unsupported** | Terminal text area is a custom XAML surface; no ValuePattern for the buffer. Expect `unsupported_no_value_pattern`. Keystrokes are the only sane path here anyway. |
| 5 | **Browser `<textarea>`** (Chrome/Edge) | **Partial → likely unsupported** | Some Chromium text inputs surface a ValuePattern via the accessibility tree, but it is often read-only to external clients or absent for `contenteditable`. Test both a plain `<textarea>` and a Gmail-style `contenteditable`. Expect mostly fallback. |
| 6 | **Electron app** (Slack / Discord) | **Unsupported** | `contenteditable` message box inside Chromium; expect `unsupported_no_value_pattern`. |
| 7 | **MS Word** | **Partial / unsupported** | Word's document surface exposes TextPattern (read-only to us) but typically **no writable ValuePattern** on the body. Expect fallback. Small dialog fields inside Word may accept it. |

Suggested extra columns to fill in: actual `inserted` (Y/N), actual `method`,
whether the app's own change handlers reacted (e.g. did autosave/validation
fire), and whether anything looked wrong (caret jumped, selection lost).

**Interpretation guide:** the value of this experiment is entirely in rows 1, 5,
and 7 for the empty-field case. If even Notepad's multi-line case forces a
fallback (it will, by design), that confirms UIA can't be the general path.

## 6. Recommendation

**Keep UIA as an opt-in fast path with a mandatory keystroke fallback. Do not
make it the default, and do not ship it at all unless the owner's matrix shows a
real win in apps they dictate into.**

Rationale:
- The clipboard-free + keystroke-free atomic insert is real, but only for
  **empty, value-backed controls** — and dictation frequently targets
  partially-filled fields, editors, browsers, and Electron apps where UIA
  either can't write or would clobber content.
- "Whole-value replace, not caret insert" is a fundamental UIA limitation, not
  an implementation gap. It cannot be engineered away from the client side.
- Today's `DirectInsert` (Unicode keystroke burst) already inserts at the caret
  in essentially every app and is the correct general default. UIA's only
  advantage is *atomicity* (one operation vs. N keystrokes) and *zero key
  events*, which matters mainly for apps that choke on synthetic keystrokes or
  for very long text where a burst is slow/visible.

### Proposed fallback strategy (if pursued)

A tiered insert, behind an opt-in setting (e.g. `pasteMethod: "uia_then_keys"`):

1. **Tier 1 — UIA atomic:** `output::ensure_foreign_focus()` →
   `output_uia::insert_focused(text)`.
   - If `outcome.inserted == true` → done. No clipboard, no keystrokes.
   - If `inserted == false` (any `unsupported_*` / no-focus) → Tier 2.
   - If `Err` (COM/UIA init failed) → Tier 2.
2. **Tier 2 — keystroke burst:** the existing
   `platform::direct_insert_text(text)` (Unicode `SendInput`), which inserts at
   the caret and works almost everywhere.
3. **Tier 3 — clipboard-restore paste:** the existing
   `ClipboardRestore` path, only if the user explicitly prefers it for very
   large pastes.

Crucially, Tier 1 must keep the **overwrite-safety gate**: never `SetValue` over
a non-empty control. Without that gate, UIA-as-default would silently destroy
user text — which is why default-on is off the table.

### If not pursued

Leave the module in place behind the `experimental_uia_insert` command for
future evaluation, or delete it. It is fully isolated; removing it touches only
the six wiring points listed in §2.

## 7. Build / verification notes

- This cannot build a full Windows binary on the Linux dev box: the project's C
  dependencies (`ring` via `reqwest` rustls-tls, and `lib.exe`/`cl.exe` for the
  MSVC target) require a Windows or MinGW cross-toolchain that isn't installed,
  and passwordless install wasn't available.
  - `cargo check --target x86_64-pc-windows-msvc` → fails in `cc-rs`
    (`lib.exe` not found) **before** compiling any of our Rust.
  - `cargo check --target x86_64-pc-windows-gnu` → fails building `ring`
    (`x86_64-w64-mingw32-gcc` not found), again a C-toolchain gap, not a Rust
    error.
- **What was verified instead:**
  - `cargo check` (host Linux) — **passes**; validates all wiring, the shared
    `UiaInsertOutcome` type, and the `#[cfg(not(windows))]` stub.
  - `cargo test --lib output_uia` — **2 tests pass** (serde shape; non-Windows
    stub error).
  - The exact `#[cfg(windows)]` UIA code was copied verbatim into a throwaway
    crate depending only on `windows` 0.58 (no C deps) and
    `cargo check --target x86_64-pc-windows-gnu` on it **passes** — this
    type-checks every UIA API call against the real 0.58 signatures. (This is
    how the original `SetValue(PCWSTR)` mistake was caught: `SetValue` actually
    takes `Param<BSTR>`, now `SetValue(&BSTR::from(text))`.)
- **Still requires a real Windows run** to validate runtime behavior and to fill
  in the matrix in §5.
