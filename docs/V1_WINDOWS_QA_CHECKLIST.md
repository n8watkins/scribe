# Windows QA checklist

What gets checked before a Windows release, split by who checks it: GitHub Actions on its own, GitHub Actions once the end-to-end job exists, or a person on a real PC.
All automated testing runs on GitHub-hosted runners; nothing depends on a maintainer's machine.

## Automated today (GitHub Actions)

CI (`.github/workflows/ci.yml`) on every pull request:

- Backend formatting, clippy and tests on Windows (with and without the test-only `e2e-audio` feature); frontend lint, tests and build; CLI and MCP server tests; npm and Rust dependency audits.

Windows end-to-end (`.github/workflows/e2e-windows.yml`) on pull requests and pushes to main that change the app, nightly, and on demand:

Hosted runners have an interactive desktop but no audio devices.
The job builds Scribe Dev with the `e2e-audio` feature, which records a WAV file in place of the microphone (`app/src-tauri/src/test_audio.rs`), so hotkeys, recording, Whisper and output all run for real.
It uses a CPU-only build of the pinned whisper.cpp source and the default model, downloaded through the app's verified download path.
`app/e2e/windows/run.py` drives it with injected hotkeys (Ctrl+Alt+F7 to F12) against stand-in app windows in their own processes, and checks the window text, the clipboard, `status.json`, the database and the app log.
Screenshots (dashboard, the floating pill while recording, any failure), app logs and results are uploaded on every run.

- `--self-test-transcribe` downloads the default model on a fresh profile and transcribes `tests/fixtures/jfk.wav`.
- Closing the main window hides it to the tray when `minimizeToTray` is on, and Scribe keeps running.
- Toggle dictation with the hotkey: the transcript lands in the focused text box and the previous clipboard is restored.
- Hold-to-talk with the keys held for the whole clip.
- Each output mode: Save Only (saved to History, clipboard untouched), Copy to Clipboard, Copy + Paste, Auto Paste, and typed insertion (`direct_insert`, the clipboard sequence number never changes).
- Switching apps mid-dictation: the text is pasted into neither app and stays on the clipboard.
- Paste the last transcript, and discard a dictation (nothing pasted, saved or copied).
- Transform Selection against a stand-in OpenAI-compatible server on 127.0.0.1: the spoken instruction and the selection reach the server, the result replaces the selection, and the clipboard is restored.

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
