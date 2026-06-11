# LocalDictate

Private, local-first dictation for Windows. Hold a hotkey, talk, release — your words are transcribed on your own machine by [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and inserted wherever your cursor is. No cloud, no account, no audio ever leaving your PC.

- **Push-to-talk:** hold `Ctrl+Shift`, speak, release.
- **Toggle mode:** tap `` ` `` (tilde) to start/stop hands-free.
- **Paste last transcript:** `Ctrl+Alt+V` re-inserts your most recent dictation anywhere.
- **Floating pill:** a small always-on-top status pill shows recording/transcribing state even while the app is hidden in the tray. Drag it anywhere; it remembers its spot. Click it to stop recording.
- **History & stats:** searchable transcript history with retention controls, kept in a local SQLite database.
- **All hotkeys rebindable** from the Hotkeys tab.

Windows 10/11 x64 only, by design. There are no plans to over-build this for other platforms.

## Install (for users)

1. Download `LocalDictate_x64-setup.exe` from the [latest release](../../releases/latest) and run it.
   - The binary is not code-signed, so Windows SmartScreen will warn you. Click **More info → Run anyway**.
   - The installer bootstraps Microsoft WebView2 automatically if you don't have it.
2. Launch LocalDictate. Open the **Models** tab and download a model — `base.en` (~140 MB) is a good start; `small.en` is more accurate and still fast on modern CPUs. Models download once from [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp) and run entirely offline afterward.
3. Open the **Audio** tab, pick your microphone, and use **Record test** / **Play test** to confirm it hears you.
4. Put your cursor in any app, hold `Ctrl+Shift`, and talk.

Closing the window minimizes to the tray; dictation hotkeys keep working. Quit from the tray icon.

### Default hotkeys

| Action | Default |
| --- | --- |
| Hold to talk | `Ctrl+Shift` (hold) |
| Toggle dictation | `` ` `` (tilde) |
| Paste last transcript | `Ctrl+Alt+V` |
| Open dashboard | `Ctrl+Alt+D` |

If a hotkey conflicts with another app, rebind it in the **Hotkeys** tab — conflicts are reported inline.

### Privacy

Audio is captured to a temp folder, transcribed locally, and the temp audio is deleted. Transcripts live in a local SQLite database under `%APPDATA%\com.natkins.localdictate\` (the **Data & Privacy** tab can open the folder, clear history, or disable history entirely). Nothing is uploaded anywhere; the only network access is the one-time model download you trigger yourself.

## Building from source

Prerequisites: Windows 10/11 x64, [Rust](https://rustup.rs/) (stable, MSVC toolchain), Node.js 20+, and the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) (Visual Studio Build Tools with the Desktop C++ workload).

1. **Provide the whisper.cpp binaries** (required — the build intentionally fails at runtime without them). Download the Windows x64 binary zip from a [whisper.cpp release](https://github.com/ggml-org/whisper.cpp/releases) and copy exactly these files into `app/src-tauri/resources/bin/windows/`:

   ```text
   whisper-cli.exe
   whisper-server.exe
   whisper.dll
   ggml.dll
   ggml-base.dll
   ggml-cpu.dll
   ```

   `whisper-server.exe` powers the warm transcriber (the model stays loaded in RAM between dictations); `whisper-cli.exe` is the fallback path.

   Don't copy the rest of the zip — everything in `resources/` gets bundled into the installer.

2. Build:

   ```powershell
   cd app
   npm install
   npm run tauri build
   ```

   Installers land in `app/src-tauri/target/release/bundle/nsis/` (and `bundle/msi/`).

For development, `npm run tauri dev` gives hot reload. `cargo test` in `app/src-tauri/` runs the backend tests. Note that most of the audio/hotkey/paste code is `#[cfg(windows)]`-gated — compiling on non-Windows hosts proves little about the Windows build.

Design and architecture docs live in [`docs/`](docs/), including the [PRD](docs/PRD.md) and the [Windows QA checklist](docs/V1_WINDOWS_QA_CHECKLIST.md).

## Roadmap / known gaps

- No code signing yet (SmartScreen warning on install).
- Incremental transcription (text appearing while you talk) is planned; the warm transcription service was built segment-first to support it.
- Transcript search uses SQL `LIKE`; fine for thousands of entries, not millions.
- Launch-at-startup setting is not yet wired to the OS.

Contributions and issue reports are welcome.

## License

[MIT](LICENSE)
