# Lane 2: security and privacy

Assessed on 2026-10-27 against `main` at `1d88fe9` (v0.8.0 plus 15 unreleased commits).
Every finding below was checked in the code; each is marked CONFIRMED (the code does this) or PLAUSIBLE (the code allows it, but exploitation depends on conditions outside the code).

## Summary

Scribe's security engineering is above average for a hobby desktop app.
The dictation-state server is loopback-only and token-protected, the GitHub token sits in Windows Credential Manager with a narrowly scoped GitHub App, model downloads are pinned to a commit and SHA-256 checked, updates are minisign-verified, imports are size-capped and stripped of audio paths, CSV export neutralizes formulas, and the release pipeline pins every action to a SHA and verifies the whisper.cpp commit.

The weak spots are mostly privacy leaks around the edges of the core promise, not classic exploits:

- Every dictation passes through the Windows clipboard without the "exclude from history" markers, so it lands in Clipboard History and, if the user turned it on, Microsoft's cloud clipboard.
- The bundled `whisper-server.exe` listens on loopback with no auth and `Access-Control-Allow-Origin: *`, and exposes a `/load` route that takes a file path.
- The Ctrl+Alt+R transform ignores the "AI tools off" switch and sends selected text to the configured endpoint.
- Several copies of private data survive "Clear history" and retention (failed recordings, "open in editor" temp files, the last-transcript buffer).
- The updater signing key is exposed to the whole build, including npm install scripts, and a stolen key would push a silent update to every install.

No critical issues were found.

### Threat model

| Attacker | Reach | What they could get today |
| --- | --- | --- |
| A web page in the user's browser | Loopback HTTP, if the browser allows local-network requests | Cannot read the state server (token). Can talk to `whisper-server` (no auth, CORS `*`): crash it, keep it busy, or make it load a local file as a model (finding M1). |
| Another process running as the same user | Everything the user can touch | Out of scope by design (it can read the DB, the keychain entry and the control file directly). Noted only where Scribe makes it easier. |
| Another Windows user on the same PC | Files with default ACLs | Blocked from `%APPDATA%` and `%LOCALAPPDATA%` by default profile ACLs; the control file gets an explicit owner-only DACL. A custom data folder gets no extra ACL (I3). |
| The network | Outbound HTTPS to GitHub and Hugging Face; user-set AI endpoint | HTTPS with rustls everywhere except the AI endpoint, which may be plain `http://` to any host (L5). |
| A malicious update | The updater | Needs the minisign key; the key's exposure in CI is the real risk (M3). |
| A malicious model or media file | whisper.cpp and ffmpeg parsers | Models are hash-pinned. User-chosen media goes through the user's own ffmpeg (I6). |
| Microsoft or OEM clipboard sync | Clipboard History and cloud clipboard | Gets every clipboard-pasted dictation and every transformed selection (H1). |

## Findings by severity

### Critical

None.

### High

#### H1. Dictations and transformed selections go into Windows Clipboard History and the cloud clipboard

- **Status:** CONFIRMED (absence of the exclusion formats in code; the Windows behavior is documented by Microsoft).
- **What:** The default paste method writes the transcript to the clipboard with `arboard` (`app/src-tauri/src/output.rs:420`), sends Ctrl+V, then restores the old clipboard.
  It never registers the `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` (0) or `CanUploadToCloudClipboard` (0) formats (no match for any of them anywhere in `src/`).
  `arboard` only sets these when asked through its Windows `exclude_from_history()` extension, which Scribe does not call.
  The selection transform does the same twice: it copies the user's selection with Ctrl+C (`selection_transform.rs:98`) and pastes the rewritten text through the clipboard.
- **Exploit scenario:** A user with Clipboard History on (Win+V) has every dictation saved there, past Scribe's own retention and "Clear history".
  A user with "Sync across your devices" on uploads every dictation and every transformed selection to their Microsoft account.
  Clipboard managers (Ditto, 1Password's clipboard, Raycast-style tools) also log them.
  The restore step does not remove the entry from history; it only adds another.
- **Why it matters:** This is the one path where dictated text routinely leaves the PC, and it contradicts "your voice never leaves it" in spirit for the text it becomes.
- **Fix:** Before `set_text`, also set the three marker formats on the same clipboard open (register them with `RegisterClipboardFormatW`, write a zero DWORD for the two `Can*` formats and an empty payload for `ExcludeClipboardContentFromMonitorProcessing`).
  Do the same for the Ctrl+C capture result when Scribe re-writes the clipboard, and for "Copy" actions in the app (optional, since a user who clicks Copy may want history).
  Mention it in the README privacy section.
- **Effort:** S.

### Medium

#### M1. The bundled whisper-server is reachable by any web page: no auth, CORS `*`, and a `/load` route that takes a local file path

- **Status:** CONFIRMED in code; exploitation PLAUSIBLE (depends on the browser's local-network protections).
- **What:** `whisper_server.rs:629-650` starts `whisper-server.exe` with `--host 127.0.0.1 --port <ephemeral>` and nothing else.
  At the pinned whisper.cpp commit (`f049fff`, `examples/server/server.cpp:729-731`) the server sends `Access-Control-Allow-Origin: *` on every response and has no API key.
  It exposes `POST /inference` and `POST /load`; `/load` frees the current model, loads whatever file path the multipart `model` field names, and calls `exit(1)` if that load fails (`server.cpp:1163-1199`).
  Multipart POSTs are CORS "simple requests", so no preflight is needed.
- **Exploit scenario:** A malicious page scans the Windows ephemeral range (49152-65535) on 127.0.0.1, finds the server, and:
  posts `/load` with `C:\Windows\notepad.exe` to crash it on demand (dictation fails until Scribe respawns it);
  or posts a path to a file it got onto disk earlier (for example a drive-by download into `Downloads`) so whisper.cpp parses an attacker-controlled GGML file, which is a memory-safety attack surface;
  or floods `/inference` to hold the server's mutex and stall the user's dictations.
  It cannot read the user's own audio or transcripts through this server.
  Chrome and Edge (142 and later) now put loopback requests from public sites behind a Local Network Access permission prompt, which lowers but does not remove the risk (users click Allow; other browsers and localhost-served pages are not covered).
- **Fix:** Start the server with `--request-path /<random 128-bit token>` (supported by the pinned build, `server.cpp:254`) so every route needs an unguessable prefix; Scribe already owns the port and can build the URL.
  Longer term, patch out `/load` and the CORS header in the vendored build, or move to the whisper.cpp library in-process.
- **Effort:** S for the request-path token, M for patching the server.

#### M2. Ctrl+Alt+R (transform selection) runs and sends selected text even when the AI tools are off

- **Status:** CONFIRMED (confirms HANDOFF open problem 3).
- **What:** The hotkey is registered unconditionally from the default `HotkeySettings` (`settings.rs:554-583`) and dispatched to `toggle_transform` (`hotkeys.rs:998`), which copies the selection with a synthetic Ctrl+C (`hotkeys.rs:1122-1143`).
  `finish_selection_transform` (`dictation.rs:404-430`) and the typed `transform_selection` command (`commands.rs:1366-1408`) post the selection to `notes_analysis_endpoint` without checking `notes_analysis_enabled`.
  Endpoint validation is skipped while the feature is off (`settings.rs:836`), so a stale or remote endpoint stays in use.
- **Exploit scenario:** A user once pointed Scribe at a remote OpenAI-compatible server, then turned "Analyze notes" off to stop sending data out.
  Pressing Ctrl+Alt+R (a chord some apps and keyboard layouts also use) still copies whatever is selected and sends it there.
  With the default endpoint and nothing listening, the only side effects are a stray Ctrl+C into the focused app and a failure notice.
- **Fix:** Gate both transform paths on `notes_analysis_enabled` (fail fast with "Turn on AI tools in Settings"), and only register the chord while the AI tools are on.
- **Effort:** S.

#### M3. The updater signing key is in the environment of the entire build, including npm install scripts

- **Status:** CONFIRMED in workflow; exploitation PLAUSIBLE (needs a compromised dependency).
- **What:** `release.yml:146` runs `npm ci` in `app/` with install scripts enabled.
  `release.yml:193-202` then runs `npm run tauri build` with `TAURI_SIGNING_PRIVATE_KEY` and its password in the environment, so every Cargo build script, proc macro, Vite plugin and npm package in the build can read them.
  No GitHub Environment with required reviewers guards the secrets, and `workflow_dispatch` (`release.yml:7`) lets any branch run the workflow.
- **Exploit scenario:** A compromised transitive npm or crates.io package reads the key during the build and exfiltrates it.
  The attacker then publishes a signed `latest.json` (needs GitHub release write too, or a man-in-the-middle on a future endpoint change), and every install updates silently at next launch (`installMode: quiet`, auto-install on launch).
- **Fix:** Build without the key, then sign the updater artifacts in a separate job or step that only runs `tauri signer sign` (or minisign) with the key.
  Use `npm ci --ignore-scripts` in `app/` (check that nothing needs install scripts; esbuild's binary is the usual exception and can be handled with an allowlist).
  Put the signing secrets in a `release` Environment limited to tags `v*` with a required reviewer.
- **Effort:** M.

#### M4. Installers are not Authenticode-signed

- **Status:** CONFIRMED (known; HANDOFF "Code signing").
- **What:** Only updater artifacts are signed; the NSIS and MSI installers are not, and the site tells users to click "More info, Run anyway".
- **Exploit scenario:** A look-alike site or a poisoned search result hosts a trojaned `Scribe_x64-setup.exe`; users already trained to bypass SmartScreen run it.
  `SHA256SUMS.txt` lives on the same release page, so it does not help a user who was sent elsewhere.
- **Fix:** Azure Trusted Signing (about $10 a month, no reputation gate) wired into `release.yml`.
- **Effort:** M (owner needs the account).

### Low

#### L1. Paste goes to whatever window is in front when transcription finishes, not where dictation started

- **Status:** CONFIRMED in code; harm PLAUSIBLE.
- **What:** `output.rs:595-626` pastes into the current foreground window, or, if Scribe itself is in front, the next eligible window in Z-order.
  Nothing records the target window at recording start.
  Transcription can take seconds (large models on CPU) and the inference timeout is 300 s (`whisper_server.rs:47`).
- **Scenario:** A user dictates a private note, switches to a Teams or Slack chat while it transcribes, and the note is pasted into the chat box.
- **Fix:** Capture the foreground HWND (and its process id) when recording starts; at paste time, if it changed, fall back to "copied, press Ctrl+V" with a notice instead of pasting.
- **Effort:** S to M.

#### L2. "Open in editor" leaves plain-text copies of transcripts that nothing deletes

- **Status:** CONFIRMED.
- **What:** `open_transcript_externally` (`commands.rs:340-394`) writes `<cache>\scribe-export\<id>.txt` and never removes it.
  Deleting the transcript, retention, "Clear history" and turning history off all leave these files.
- **Fix:** Clear the folder on startup and on Clear history, and delete a transcript's file when the transcript is deleted.
- **Effort:** S.

#### L3. Failed recordings sit outside retention, "Clear history" and "Save raw audio clips"

- **Status:** CONFIRMED.
- **What:** When transcription fails, the WAV moves to `<data>\failed\` and up to 20 are kept (`dictation.rs:114-170`).
  Neither retention, `clear_transcript_history` (`db.rs:783`), nor turning "Save raw audio clips" off touches them.
  The last-transcript buffer also survives "Clear history" (`db.rs:783-794` only deletes rows; the buffer lives in the settings table).
- **Fix:** Age failed recordings out on the same retention window, clear them (and the buffer) from Clear history, and say in Data & Privacy that they exist.
- **Effort:** S.

#### L4. An enterprise "internal" repository passes the private-repo check

- **Status:** CONFIRMED against GitHub API semantics.
- **What:** `github_backup.rs:154` only checks `"private": true`.
  GitHub returns `private: true` with `visibility: "internal"` for internal repos, which every member of the enterprise can read.
- **Fix:** Require `visibility == "private"` (fall back to `private` only when `visibility` is absent).
- **Effort:** S.

#### L5. The AI endpoint accepts plain HTTP to any host, follows redirects, and has no warning

- **Status:** CONFIRMED.
- **What:** Validation only checks the `http://` or `https://` prefix (`settings.rs:836-839`).
  `note_analysis.rs:147-150` builds a default `reqwest` client, which follows up to 10 redirects (307 and 308 keep the POST body).
  The UI does not distinguish "local" from "remote".
- **Scenario:** A user enters a LAN or internet `http://` address; notes and selections cross the network unencrypted.
- **Fix:** Treat loopback as local; for anything else show "This sends your text to <host>" and require `https://` unless the user confirms; set `redirect::Policy::none()`.
- **Effort:** S.

#### L6. No CSP, a broad command surface, and an unused opener permission

- **Status:** CONFIRMED (defense in depth; no injection sink was found).
- **What:** `tauri.conf.json:41` sets `"csp": null`.
  All 68 app commands are callable from both windows, including the pill (no `build.rs` app manifest restricts them), and some take raw paths: `save_text_file` writes `<any path>.txt` and overwrites silently (`file_transcribe.rs:98-108`), `transcribe_file` reads any path, `update_settings` can move `data_dir` anywhere.
  `capabilities/default.json:15` grants `opener:default` although the front end never imports the opener plugin.
  I found no `dangerouslySetInnerHTML`, `innerHTML` or `eval` in `app/src`, and LLM output is rendered as text, so there is no known XSS today.
- **Fix:** Set a strict CSP (`default-src 'self'; img-src 'self' data: blob:; media-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; connect-src ipc: http://ipc.localhost`), drop `opener:default`, and use an app manifest so the pill window only gets the few commands it needs.
  Have `save_text_file` refuse to overwrite and only accept the path of the file just transcribed.
- **Effort:** S to M.

### Info

- **I1.** The state-server token comparison is not constant-time (`state_server.rs:313`); negligible over loopback, but `subtle::ConstantTimeEq` costs nothing.
- **I2.** The whisper-server port is picked by binding and releasing a port (`whisper_server.rs:610`), and readiness is "any HTTP response" (`whisper_server.rs:597`); a same-user process that grabs the port first would receive audio.
  Same-user attackers are out of scope, but the M1 request-path token also closes this.
- **I3.** Transcripts, notes and audio are stored unencrypted, which is normal for the category.
  A custom data folder (`settings.rs:217`) gets no ACL tightening, and choosing a OneDrive or shared folder silently syncs everything; the folder picker could warn.
- **I4.** SQLite does not use `secure_delete` or `VACUUM` after deletes, so cleared text stays in free pages until overwritten.
- **I5.** The device-flow `verification_uri` from GitHub is opened as given (`commands.rs:919`); pinning it to `https://github.com/` is cheap hardening.
- **I6.** File transcription runs the user's `ffmpeg` from `PATH` with the path as a single argument after `-i` (`file_transcribe.rs:139-145`), and `is_file()` is checked first, so argument and protocol injection are not possible; the remaining risk is ffmpeg's own parsers on untrusted media.
- **I7.** `commits.yml:16` uses the release-kit reusable workflow by tag (`@v2.0.0`) instead of a SHA; it has `contents: read` only.
- **I8.** The Dev flavor (`tauri.dev-flavor.conf.json`) shares the stable updater endpoint and key, so a Dev install would be offered the stable installer; a reliability issue rather than security (for Lane 4).
- **I9.** Models are verified by SHA-256 when selected (`model_manager.rs:253`), not at every load; fine given the threat model.
- **I10.** Logs (`lib.rs:242-259`, Debug for `app_lib`, 5 x 5 MB) contain no transcript text, instructions or tokens that I could find; `whisper-server` output goes to null.

## Privacy claims versus behavior

| Claim | Where | What the app does | Status |
| --- | --- | --- | --- |
| "Audio is captured to a temp folder, transcribed locally, and the temp audio is deleted." | `README.md:77` | "Save raw audio clips" is on by default (`settings.rs:675`); clips are kept with their transcript, 30 days for dictations and forever for notes. Failed recordings are kept (up to 20) regardless. | Wrong |
| "Each recording is ... deleted with its transcript (after 30 days by default), or right away if you turn off Save raw audio clips." | scribe-web `app/privacy/page.tsx:57-59`, `lib/site.ts:83` | True for ordinary dictations. Note clips follow notes' "keep forever" default. Failed recordings ignore both the setting and retention (L3). | Partly wrong |
| "Your voice never leaves it" / "never uploaded". | `README.md:5`, `components/Hero.tsx:22`, `app/layout.tsx:12` | True for audio. The text goes through the clipboard into Clipboard History and, if enabled, Microsoft's cloud clipboard (H1). | Misleading for text |
| "Online AI: only if you point the AI tools at one" / "AI tools if you turn them on". | `components/Explainers.tsx:78`, `app/privacy/page.tsx:60` | Ctrl+Alt+R works and sends the selection with the AI tools off (M2). | Wrong |
| "A keystroke type-it-out fallback is available for apps that block paste." | `README.md:18` | There is no control for it in the UI, and a settings migration resets `DirectInsert` to clipboard paste (`settings.rs:768-769`). Confirms HANDOFF open problem 2. | Wrong |
| "Transcripts live in a local SQLite database under `%APPDATA%\com.natkins.scribe\`" and Data & Privacy "can clear history". | `README.md:78` | Also in `scribe-export\*.txt` temp copies (L2), `failed\` WAVs (L3), the last-transcript buffer, and SQLite free pages (I4); none are removed by Clear history. | Incomplete |
| "The app only connects out to check for updates, to download a model you pick, and to the optional GitHub backup and AI tools." | `app/privacy/page.tsx:60-61`, README:80 | Matches the code (only GitHub, Hugging Face and the AI endpoint appear). The installer may also download the WebView2 bootstrapper from Microsoft. | Correct, minor omission |
| "Checked against its known SHA-256 hash." | `components/Explainers.tsx:75` | True: pinned commit URLs plus SHA-256 for all 11 models (`models.rs`). The old handoff note that multilingual models ship without a checksum is out of date. | Correct |
| "It checks the repo is private." | `lib/site.ts:109` | True, except enterprise internal repos pass (L4). | Mostly correct |
| "No account, no telemetry." | README, site | No telemetry or analytics endpoints exist in the code. | Correct |

## Dependency audit (2026-10-27)

- `cargo audit --file app/src-tauri/Cargo.lock` (as CI runs it, with the root `.cargo/audit.toml`): 0 vulnerabilities, 5 allowed warnings.
  The warnings are `glib` 0.18.5 unsound (RUSTSEC-2024-0429, Linux GTK only), `proc-macro-error` unmaintained, and `zbus_names`, `zvariant`, `zvariant_derive` unmaintained (RUSTSEC-2026-0370 to 0372, new on 2026-10-16, Linux-only D-Bus stack).
  Run from `app/src-tauri/` instead, the root ignore file is not picked up and the three `zip` advisories (RUSTSEC-2026-0367 to 0369) show; they are ignored on purpose because only `tauri-plugin-updater` uses `zip`, after signature checks, and the Windows NSIS updater artifact is not a zip.
- `cargo audit --file cli/Cargo.lock`: clean.
- `npm audit` in `app/`, `integrations/mcp-server/` and the repo root: 0 vulnerabilities.
- Open Dependabot PR: #38 (GitHub Actions versions).
- CI still uses Node 20, end of life since April 2026 (`ci.yml:51,109`, `release.yml:97`).

## What is done well

- The dictation-state server binds 127.0.0.1 only, requires a random token on every route, sends no CORS headers, caps connections at 32, and writes its discovery file atomically with an owner-only protected DACL (`state_server.rs:255-560`); it exposes status only, never text.
- The MCP server and CLI are read-only clients of that server.
- GitHub uses a GitHub App device flow (no client secret, no classic `repo` scope, `Ov` OAuth ids rejected), tokens and refresh tokens live only in Windows Credential Manager with a redacted `Debug`, and backups refuse public repos.
- Model downloads use commit-pinned Hugging Face URLs, exact size checks during streaming, SHA-256 verification, `.part` files and cleanup on failure.
- Import is capped (100 MB, 100,000 records, 5 MB per text), fingerprint-confirmed, and strips `audio_path` so a crafted backup cannot point playback at arbitrary files.
- CSV export neutralizes formula-leading cells.
- The updater uses a pinned minisign key, a single GitHub HTTPS endpoint, and `allowDowngrades: false`; auto-install runs only at launch.
- The release workflow pins every third-party action to a SHA, verifies the whisper.cpp tag resolves to the reviewed commit, generates an SBOM, smoke-installs both installers, and verifies checksums after upload; workflow permissions default to `contents: read`.
- Child processes run with null stdio and `CREATE_NO_WINDOW`; logs contain no transcript text; `whisper-server` is not started with `--convert`, which would shell out to ffmpeg.
- The NSIS pre-install hook kills only Scribe's own `whisper-server.exe` by full path.

## Suggested order

1. H1 clipboard markers (S), M2 transform gate (S), M1 request-path token (S): one small PR, then release with the #40 fixes.
2. L2, L3, L4, L5 and the README and site wording fixes (S each).
3. M3 split signing and a protected Environment (M), then M4 code signing when the owner has the account.
4. L1 paste-target pinning and L6 CSP and capability tightening (S to M).
