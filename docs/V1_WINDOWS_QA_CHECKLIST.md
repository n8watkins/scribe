# Windows QA checklist

What gets checked before a Windows release, split by who checks it: GitHub Actions on its own, GitHub Actions once the end-to-end job exists, or a person on a real PC.
All automated testing runs on GitHub-hosted runners; nothing depends on a maintainer's machine.

## Automated today (GitHub Actions)

CI (`.github/workflows/ci.yml`) on every pull request:

- Backend formatting, clippy and tests on Windows; frontend lint, tests and build; CLI and MCP server tests; npm and Rust dependency audits.

Release (`.github/workflows/release.yml`) before anything is published:

- The tag matches every version file, and the release metadata validates.
- The pinned whisper.cpp source builds with Vulkan, and `whisper-cli.exe`, `whisper-server.exe`, `whisper.dll`, `ggml.dll`, `ggml-base.dll`, `ggml-cpu.dll` and `ggml-vulkan.dll` are bundled.
- The updater artifact is signed, `SHA256SUMS` covers every asset, and the SBOM (with the bundled whisper.cpp) is attached.
- Smoke install, separately for NSIS and MSI on a clean runner: checksums verify, the installer runs silently, the uninstall entry exists, the bundled binaries are in place, the app launches and stays up, `--self-test-transcribe` downloads the default model through the verified download path and transcribes `tests/fixtures/jfk.wav`, and the uninstaller removes it.
- The published release's assets are verified after upload.

## Can be automated next (issue #93)

A spike on 2026-11-04 confirmed that hosted `windows-2022` and `windows-2025` runners have an interactive desktop where a test can take focus, receive `RegisterHotKey` hotkeys from injected input, see held keys through `GetAsyncKeyState`, read and write the clipboard, paste with Ctrl+V, type with Unicode `SendInput`, switch to another app and back, and take screenshots.
They have no audio devices, so the job needs a test-only way to feed a WAV file into the dictation pipeline in place of the microphone, like `--self-test-transcribe` does for transcription.
With that, an end-to-end job can cover:

- Toggle dictation with the hotkey: the transcript lands in the focused text box and the previous clipboard is restored.
- Hold-to-talk with the keys held for the whole clip.
- Each output mode: Save Only, Auto Paste, Copy to Clipboard, Copy + Paste, and typed insertion (`direct_insert`, clipboard never touched).
- Switching apps mid-dictation: the text is not pasted into the wrong app and stays on the clipboard.
- Paste the last transcript (`Ctrl+Alt+V`) and discard a dictation.
- Transform Selection against a stand-in OpenAI-compatible server on 127.0.0.1.
- Transcript history, stats, search, edit and delete refresh without a restart.
- Closing the main window hides it to the tray when `minimizeToTray` is on.
- MSI upgrade from the previous release keeps settings and history.
- Screenshots of the floating pill in its recording, transcribing, ready and error states, uploaded for review.

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
