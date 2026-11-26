# Windows QA checklist

What gets checked before a Windows release, split by who checks it: GitHub Actions on its own, GitHub Actions once the end-to-end job exists, or a person on a real PC.
All automated testing runs on GitHub-hosted runners; nothing depends on a maintainer's machine.

## Automated today (GitHub Actions)

CI (`.github/workflows/ci.yml`) on every pull request:

- Backend formatting, clippy and tests on Windows (with and without the test-only `e2e-audio` feature); frontend lint, tests and build; CLI and MCP server tests; npm and Rust dependency audits.

Windows end-to-end (`.github/workflows/e2e-windows.yml`) on pull requests and pushes to main that change the app, nightly, and on demand:

Hosted runners have an interactive desktop but no audio devices.
The job builds Scribe Dev with the `e2e-audio` feature, which records a WAV file in place of the microphone (`crates/scribe-core/src/test_audio.rs`), so hotkeys, recording, Whisper and output all run for real.
It uses a CPU-only build of the pinned whisper.cpp source and the default model, downloaded through the app's verified download path.
`app/e2e/windows/run.py` drives it with injected hotkeys against stand-in app windows in their own processes, and checks the window text, the clipboard, `status.json`, the state server's event stream, the database and the app log.
Its last scenario fails the run when the app log contains "The engine's folder differs from Tauri's" (the engine and Tauri resolved different data folders at startup), and the Linux and macOS install-and-launch jobs make the same check on their logs (`app/e2e/folder_mismatch.py`, tested by `app/e2e/test_folder_mismatch.py`).
Screenshots (dashboard, the floating pill while recording, any failure), app logs and results are uploaded on every run.

- `--self-test-transcribe` downloads the default model on a fresh profile and transcribes `tests/fixtures/jfk.wav`.
- Closing the main window hides it to the tray when `minimizeToTray` is on, and Scribe keeps running.
- Toggle dictation with the hotkey: the transcript lands in the focused text box and the previous clipboard is restored.
- Hold-to-talk with the keys held for the whole clip.
- Each output mode: Save Only (saved to History, clipboard untouched), Copy to Clipboard, Copy + Paste, Auto Paste, and typed insertion (`direct_insert`, the clipboard sequence number never changes).
- Switching apps mid-dictation: the text is pasted into neither app and stays on the clipboard.
- Paste the last transcript, and discard a dictation (nothing pasted, saved or copied).
- Transform Selection against a stand-in OpenAI-compatible server on 127.0.0.1: the spoken instruction and the selection reach the server, the result replaces the selection, and the clipboard is restored.

The scenarios above bind every action to Ctrl+Alt+F7 to F12, which the global-shortcut plugin handles.
A second group drives the bindings Windows users get by default.
Two of Scribe's own watchers in `crates/scribe-core/src/hotkeys.rs` handle those bindings: the chord watcher for a hold to talk made of modifiers only, and the toggle-key watcher for a toggle bound to a single key.
The group sets the bindings through the app's own hotkey commands (`reset_hotkeys_to_defaults`, `load_production_hotkey_defaults`, `rebind_hotkey`, `set_hotkey_trigger`), called inside the dashboard's WebView2.
The test build opens WebView2's remote debugging port for this (`app/e2e/windows/webview_debug_config.py`).
So the Dev and shipped defaults both come from the build under test, and rebinding happens while the watchers run.
Keys are sent with `SendInput` and scan codes, which is what the watchers' `GetAsyncKeyState` polling and `RegisterHotKey` see from a real keyboard.
Waiting for a recording to start or for Scribe to go idle polls `status.json`, as the other scenarios do.
The checks that nothing started (a short tap, an old or former toggle key, typing Q, a toggle key held down on Release) also read the state server's event stream, so a recording that lasts a moment is still seen.
Whether Scribe holds a key is checked by trying to register the same hotkey from the runner.
The runner refuses to run outside GitHub Actions without `--overwrite-scribe-dev-profile`, because it rewrites the Scribe Dev profile of the machine it runs on.

Through the chord watcher:

- The Dev build's own hold to talk (Ctrl+Shift+Win) and the shipped one (Ctrl+Win): a tap shorter than the 150 ms arming delay starts nothing; held, it records until the keys are let go, the dictation is pasted, and the app keeps the focus.

Through the toggle-key watcher (each step waits for the watcher's own log line):

- Loading the shipped defaults starts the watcher for Backquote on Release, and Backquote is held as a global hotkey.
- Bare toggle key on Release: nothing starts while the key is still down; tap, speak, tap; the dictation is pasted, the key's own character never reaches the app, and Q is let go afterwards.
- The note chord: hold the toggle key and tap Q (Q is grabbed through the global-shortcut plugin only while the toggle key is down); a note starts instead of a dictation, neither key types, the Q grab ends with the hold, the same chord ends the note, it is saved as a note and not pasted, and Q types normally again.
- The toggle on the Press trigger: the watcher restarts on Press, each press logs "Toggle key pressed (trigger Press)", the recording starts while the key is still down, and Q is not grabbed; switching back to Release restarts the watcher again.
- Rebinding the toggle to another bare key (K): the watcher restarts for K, Backquote is no longer held, types normally and starts nothing, and K toggles a dictation.

Through the global-shortcut plugin:

- The Dev build's own toggle (Ctrl+Shift+Backquote) starts a recording and its discard hotkey (Ctrl+Alt+Shift+X) drops it. This does not check which key edge the toggle fires on.
- Rebinding the toggle to Ctrl+Alt+Shift+K: no watcher starts, neither K nor Backquote stays held, K types normally and starts nothing, and the combination starts a recording that the shipped discard hotkey (Ctrl+Alt+X) drops.
- Open Dashboard (shipped Ctrl+Alt+F) opens and focuses the dashboard from the tray, and hides it again.

Afterwards the test bindings are put back through `patch_settings`, and the watcher's log lines are saved as `hotkey-watcher-log.txt` in the uploaded results.

Release (`.github/workflows/release.yml`) before anything is published:

- The tag matches every version file, and the release metadata validates.
- The pinned whisper.cpp source builds with Vulkan, and `whisper-cli.exe`, `whisper-server.exe`, `whisper.dll`, `ggml.dll`, `ggml-base.dll`, `ggml-cpu.dll` and `ggml-vulkan.dll` are bundled.
- No shipped executable contains the end-to-end test input.
- The updater artifact is signed, `SHA256SUMS` covers every asset, and the SBOM (with the bundled whisper.cpp) is attached.
- Smoke install, separately for NSIS and MSI on a clean runner: checksums verify, the installer runs silently, the uninstall entry exists, the bundled binaries are in place, the app launches and stays up, `--self-test-transcribe` downloads the default model through the verified download path and transcribes `tests/fixtures/jfk.wav`, and the uninstaller removes it.
- The published release's assets are verified after upload.

## Not automated yet

- Transcript history, stats, search, edit and delete refreshing without a restart: this is dashboard UI inside WebView2, which the end-to-end job doesn't drive yet (it would need WebView2 automation through its remote debugging port).
- Screenshots of the floating pill in its transcribing, ready and error states: these states last a fraction of a second on a runner, so only the recording state is captured reliably.
- MSI upgrade from the previous release keeping settings and history: it needs the previous release's installer and a second install pass, and is left for a release-time job.

## Manual before a release (a person on a real PC)

These need real hardware, a client edition of Windows, or human judgement, which hosted runners can't provide.

- Install and launch on Windows 10 and Windows 11 (runners are Windows Server).
- Download the installer in a browser and confirm the expected SmartScreen warning (the installers are intentionally unsigned).
- Pick a microphone in the Audio view and dictate in your own voice, including a noisy room and a quiet one.
- GPU (Vulkan) transcription speed on real graphics hardware (runners have no GPU), and the first dictation after 10 idle minutes.
- Dictate into real apps: a browser text field, Slack or Discord, Word or Outlook, a terminal, and an app running as administrator.
- The floating pill, tray icon and notifications look right, including on a second monitor and at 125% and 150% display scaling.
- Download, select, delete and retry a Whisper model from the Models view on a normal home connection.
- Sleep and resume, and a long session with many dictations.

## Historical V1 polish notes

The items below describe the original V1 state and are retained only as historical acceptance context.
They are not current gaps.

- Native OS notifications are not wired in V1; current feedback is in-app toast plus floating pill.
- Launch-at-startup requires adding and wiring the Tauri autostart plugin.
- Tray icon state variants need final art assets before replacing the default icon.
- Open data/model folder commands are still disabled in the frontend.
