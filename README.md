# Scribe

Private, local-first dictation for Windows, with beta builds for macOS and Linux.
Hold a hotkey, talk, and release to transcribe on your own machine with [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and insert the result wherever your cursor is.
Your audio never leaves your PC, and no account is required.
Optional GitHub backup is off by default and uploads text only to a private repository you control: notes by default and, if enabled, every transcript, but never audio.
Scribe is free and open source under the MIT license.

## What it does

**Dictate anywhere**

- **Push-to-talk:** hold `Ctrl+Win`, speak, and release; the text appears at your cursor.
- **Toggle mode:** tap the `` ` `` key (left of `1`) to start and stop hands-free.
- **Live transcription:** phrases are transcribed in the background while you're still talking, so text is ready the moment you stop.
  You can watch it build up on the pill.
- **Floating pill:** a small always-on-top status pill shows recording/transcribing state even while the app is hidden in the tray, with a live waveform. Drag it anywhere; it remembers its spot. Click it to stop recording. Colors and background are configurable.
- **Paste last transcript:** a hotkey re-inserts your most recent dictation anywhere, instantly.
- **Instant clipboard paste with restore:** insertion borrows the clipboard for a single paste and restores it exactly as it was (text, images, and files), so dictating never clobbers what you had copied.
  For apps that block paste, choose **Type it out** in Settings and Scribe types the text as keystrokes instead.

**Make the text better**

- **Selected-text transform:** with the AI tools turned on, highlight text in any app, tap a hotkey, and speak (or type) an instruction such as "make this concise", "translate to Spanish", or "fix grammar".
  Scribe rewrites the selection in place using your local AI server.
- **Dictionary:** a **context hint** that helps Whisper recognize your names and jargon, plus a **replacements** table ("say X → get Y", e.g. "my email" → your address, fix "clawed" → "Claude") applied to every transcript.

**Quick notes**

- **Notes:** hold the toggle key and tap `Q` to dictate a **note** (blue pill) that's saved to your Notes list instead of pasted, so you can capture a thought without disturbing the focused app.
- **On-demand analysis:** with the AI tools turned on, run your local AI server over a note (summarize, extract action items, or your own prompt).

**Keep and find your words**

- **History & stats:** searchable transcript history in a local SQLite database: search by text and **date range**, sort newest / oldest / longest, expand entries inline, open in an external editor, and **multi-select → combine** into one merged entry.
- **Separate retention for transcripts and notes:** transcripts auto-prune on a window you choose; notes default to **keep forever** (they're deliberate saves) and are never auto-deleted.

**Sync, back up, export**

- **GitHub backup (optional, your account):** push dictated notes as dated Markdown to a private repository you control, and optionally back up every transcript.
- **Export:** export transcripts to **Markdown / CSV / JSON** locally, with no account needed.

**Stays out of your way**

- **All hotkeys rebindable** from the **Hotkeys** page, with per-bind **press / release** triggers and inline conflict reporting.
- **Model manager:** download and switch between curated Whisper models (tiny to large-v3-turbo) on the **Models & language** page; they run entirely offline after the one-time download.
- **Auto-updates (Windows):** Scribe checks GitHub for new releases shortly after launch, on refocus, and every six hours, fires an OS notification when one is found, and installs it in-app. The updater artifact is cryptographically signed. Automatic checks can be turned off.
  The macOS and Linux betas don't update themselves yet: install the new version from the latest release.
- **Tray app:** closing the window minimizes to the tray; dictation hotkeys keep working. Quit from the tray icon.

Scribe is built for Windows 10/11 x64 first.
The macOS and Linux builds are betas: every release builds, installs, launches and test-transcribes them automatically, but they have had little hands-on use on real Macs and Linux desktops yet, so please report what you find in [Issues](https://github.com/n8watkins/scribe/issues).

## Install (for users)

Every file is on the [latest release](https://github.com/n8watkins/scribe/releases/latest), and `SHA256SUMS.txt` there lists a checksum for each one.
Each platform has a full guide: install, update and uninstall, permissions, what works there, where your data lives, known limits and troubleshooting.

| Platform | Download | Guide |
|---|---|---|
| Windows 10/11 x64 | `Scribe_<version>_x64-setup.exe` (or the `.msi`) | [Windows](docs/install/windows.md) |
| macOS 13.3+, Apple Silicon and Intel (beta) | [`Scribe-macos-universal.dmg`](https://github.com/n8watkins/scribe/releases/latest/download/Scribe-macos-universal.dmg) | [macOS](docs/install/macos.md) |
| Linux x86_64 (beta) | [`Scribe-linux-amd64.deb`](https://github.com/n8watkins/scribe/releases/latest/download/Scribe-linux-amd64.deb) or [`Scribe-linux-x86_64.AppImage`](https://github.com/n8watkins/scribe/releases/latest/download/Scribe-linux-x86_64.AppImage) | [Linux](docs/install/linux.md) |

- **Windows:** Scribe is not code-signed, so SmartScreen warns on first install: click **More info → Run anyway**.
- **macOS:** Scribe has no Apple Developer ID, so the app is ad-hoc signed and **not notarized**, and macOS blocks the first open.
  On macOS 15 and later, open **System Settings → Privacy & Security** and click **Open Anyway**; on macOS 13 and 14, Control-click Scribe in Applications and choose **Open**.
  If macOS says the app "is damaged", run `xattr -dr com.apple.quarantine /Applications/Scribe.app`.
  Then allow the **Microphone** and turn on **Accessibility** so Scribe can paste.
- **Linux:** `sudo apt install ./Scribe-linux-amd64.deb`, or make the AppImage executable (it needs FUSE 2: `libfuse2t64` on Ubuntu 24.04).
  Hotkeys and pasting work on X11 and on Wayland, where the desktop asks once for permission ([details](docs/install/linux.md#x11-and-wayland)).
- Trying a beta on a real Mac or Linux machine? The [testing checklist](docs/install/testing-checklist.md) says what to check and what to send back.

### First run (all platforms)

1. Launch Scribe. Open **Models & language** and download a model; `base.en` (~140 MB) is a good start; `small.en` is more accurate and still fast on modern CPUs (the default is `small.en-q5_1`, a quantized balance). Models download once from [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp) and run entirely offline afterward.
2. Open **Microphone**, pick your microphone, and use **Record test** / **Play test** to confirm it hears you.
3. Put your cursor in any app, hold the hold-to-talk hotkey (`Ctrl+Win` on Windows), and talk.

To use selected-text transform or note analysis, run a local OpenAI-compatible LLM server ([LM Studio](https://lmstudio.ai/) or [Ollama](https://ollama.com/)) and point Scribe at it on the **Local AI** page (default `http://127.0.0.1:1234/v1`).
These features are optional and off by default; core dictation needs no LLM.

### Default hotkeys

These are the Windows defaults; macOS uses `Control+Option` and Linux `Ctrl+Alt` combinations instead (see the [macOS](docs/install/macos.md#default-hotkeys) and [Linux](docs/install/linux.md#default-hotkeys) guides).

| Action | Default |
| --- | --- |
| Hold to talk | `Ctrl+Win` (hold) |
| Toggle dictation | `` ` `` (left of `1`) |
| Dictate a note | hold the toggle key + tap `Q` |
| Paste last transcript | `Ctrl+Alt+V` |
| Transform selection | `Ctrl+Alt+R` |
| Discard / cancel recording | `Ctrl+Alt+X` |
| Open dashboard | `Ctrl+Alt+F` |

If a hotkey conflicts with another app, rebind it on the **Hotkeys** page, where conflicts are reported inline. Each non-hold bind can fire on key **press** or **release**.

### Privacy

Your audio is recorded and transcribed on your PC and is never uploaded.
By default Scribe keeps each recording with its transcript so you can play it back, and deletes both automatically after 30 days.
Notes keep their text until you delete them, but their recordings are deleted after the same 30 days.
Recordings that fail to transcribe are kept (up to 20) so you can retry them, and are deleted on the same schedule.
The **Your data** page sets how long history is kept, and turning off **Save audio clips** stops Scribe keeping new recordings once they are transcribed.
Transcripts live in a local SQLite database under `%APPDATA%\com.natkins.scribe\`.
**Data & Privacy** can open the folder, turn history off, or clear history, which deletes every dictation with its audio, failed recordings and copies opened in an editor, and keeps your notes.

Dictated text reaches the app you're typing in through the clipboard.
Scribe restores what you had copied afterwards, and marks the text it borrows the clipboard for so Windows clipboard history and cloud clipboard sync skip it.
Copy buttons and the **Copy only** setting still make a normal copy.

The AI tools (selected-text transform and note analysis) are off by default.
When you turn them on, the selected text or note is sent to the server address you set, which defaults to a local LM Studio server on this PC.
While they are off, Scribe sends nothing to an AI server and the transform hotkey does nothing.

Scribe goes online only to download the models you choose, to check GitHub for updates, and, if you turn them on, for GitHub backup and an AI server that isn't on this PC.
There is no telemetry and no account.

## Building from source

Prerequisites: Windows 10/11 x64, [Rust](https://rustup.rs/) (stable, MSVC toolchain), Node.js 24, the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) (Visual Studio Build Tools with the Desktop C++ workload), CMake, Ninja, and the [Vulkan SDK](https://vulkan.lunarg.com/sdk/home#windows) version 1.4.350.0.

1. **Build the reviewed whisper.cpp runtime** from the repository root in a Developer PowerShell for Visual Studio:

   ```powershell
   $src = Join-Path $env:TEMP "scribe-whisper-cpp"
   git clone --depth 1 --branch v1.9.1 https://github.com/ggml-org/whisper.cpp $src
   $actual = (git -C $src rev-parse HEAD).Trim()
   if ($actual -ne "f049fff95a089aa9969deb009cdd4892b3e74916") {
     throw "whisper.cpp commit $actual is not the reviewed commit"
   }
   cmake -S $src -B "$src/build" -G Ninja `
     -DCMAKE_BUILD_TYPE=Release `
     -DGGML_VULKAN=ON `
     -DGGML_NATIVE=OFF `
     -DBUILD_SHARED_LIBS=ON `
     -DWHISPER_BUILD_EXAMPLES=ON `
     -DWHISPER_BUILD_TESTS=OFF `
     -DWHISPER_BUILD_SERVER=ON
   cmake --build "$src/build" -j

   $cli = Get-ChildItem "$src/build" -Recurse -Filter whisper-cli.exe | Select-Object -First 1
   if (-not $cli) { throw "whisper-cli.exe was not produced" }
   $bin = Split-Path $cli.FullName
   $dest = "app/src-tauri/resources/bin/windows"
   $needed = @(
     "whisper-cli.exe", "whisper-server.exe", "whisper.dll", "ggml.dll",
     "ggml-base.dll", "ggml-cpu.dll", "ggml-vulkan.dll"
   )
   foreach ($name in $needed) {
     $file = Get-ChildItem $bin -Recurse -Filter $name | Select-Object -First 1
     if (-not $file) { throw "Missing $name in the Vulkan build output" }
     Copy-Item $file.FullName $dest
   }
   ```

   This matches the pinned source, Vulkan SDK, CMake options, and runtime file validation in the release workflow.
   It stages exactly these required files:

   ```text
   whisper-cli.exe
   whisper-server.exe
   whisper.dll
   ggml.dll
   ggml-base.dll
   ggml-cpu.dll
   ggml-vulkan.dll
   ```

   `whisper-server.exe` powers the warm transcriber, and `whisper-cli.exe` is the fallback path.
   `ggml-vulkan.dll` enables GPU acceleration; CPU fallback remains available through `ggml-cpu.dll`.

2. Build:

   ```powershell
   cd app
   npm install
   npm run tauri build
   ```

   Installers land in `app/src-tauri/target/release/bundle/nsis/` (and `bundle/msi/`).

For development, `npm run tauri dev` gives hot reload. `cargo test` in `app/src-tauri/` runs the backend tests. Most of the audio, hotkey and paste code is `#[cfg(windows)]`-gated, so compiling on other hosts proves little about the Windows build.

The [documentation index](docs/README.md) separates current operating guidance from historical plans and reports.

## Roadmap / known gaps

- **Custom models:** you download from a curated catalog (English-only and multilingual Whisper models); pointing Scribe at an arbitrary local `ggml` `.bin` isn't supported in the UI yet.
- Transcript search uses SQL `LIKE`; fine for thousands of entries, not millions.

Contributions and issue reports are welcome.

## License

[MIT](LICENSE)

## Release integrity

Scribe's Windows executables and installers are intentionally not Authenticode-signed, so Windows SmartScreen can show an unrecognized-app warning on first install.
Authenticode code signing is not on the roadmap.
This is separate from Tauri updater signing: update metadata and updater artifacts are cryptographically signed so the installed app can reject tampered updates.
The macOS app and its bundled whisper.cpp binaries are ad-hoc signed but not notarized, because Scribe has no Apple Developer ID; see the [macOS guide](docs/install/macos.md) for how to open it.
The Linux AppImage and `.deb` are not signed.
Release automation also publishes SHA-256 checksums for every file and a software bill of materials.
Before anything is published it installs every build on a fresh runner, launches it and transcribes a speech clip with the bundled whisper.cpp: the NSIS and MSI installers on Windows, the `.dmg` on macOS (Apple Silicon, and Intel under Rosetta), and the `.deb` and AppImage on Linux.
Updates that install themselves are Windows only for now.
