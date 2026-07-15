# Scribe

Private, local-first dictation for Windows.
Hold a hotkey, talk, and release to transcribe on your own machine with [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and insert the result wherever your cursor is.
Your audio never leaves your PC, and no account is required.
Optional GitHub backup is off by default and uploads text only to a private repository you control: notes by default and, if enabled, every transcript, but never audio.
Scribe is free and open source under the MIT license.

## What it does

**Dictate anywhere**

- **Push-to-talk:** hold `Ctrl+Win`, speak, release — text appears at your cursor.
- **Toggle mode:** tap `` ` `` (tilde) to start/stop hands-free.
- **Live transcription:** phrases are transcribed in the background while you're still talking, so text is ready the moment you stop — watch it accumulate on the pill.
- **Floating pill:** a small always-on-top status pill shows recording/transcribing state even while the app is hidden in the tray, with a live waveform. Drag it anywhere; it remembers its spot. Click it to stop recording. Colors and background are configurable.
- **Paste last transcript:** a hotkey re-inserts your most recent dictation anywhere, instantly.
- **Instant clipboard paste with restore:** insertion borrows the clipboard for a single paste and restores it exactly as it was — text, images, and files — so dictating never clobbers what you had copied. (A keystroke "type it out" fallback is available for apps that block paste.)

**Make the text better**

- **Selected-text transform:** highlight text in any app, tap a hotkey, and speak (or type) an instruction — "make this concise", "translate to Spanish", "fix grammar" — and Scribe rewrites the selection in place. An inline AI editor driven by your local LLM.
- **Dictionary:** a **context hint** that primes Whisper toward your jargon/names (better recognition), plus a deterministic **replacements** table ("say X → get Y", e.g. "my email" → your address, fix "clawed" → "Claude") applied to every transcript.

**Quick notes**

- **Notes:** hold the toggle key and tap `Q` to dictate a **note** (blue pill) that's saved to your Notes list instead of pasted — capture a thought without disturbing the focused app.
- **On-demand analysis:** run a local LLM over a note (summarize, extract action items, or your own prompt).

**Keep and find your words**

- **History & stats:** searchable transcript history in a local SQLite database — search by text and **date range**, sort newest / oldest / longest, expand entries inline, open in an external editor, and **multi-select → combine** into one merged entry.
- **Separate retention for transcripts and notes:** transcripts auto-prune on a window you choose; notes default to **keep forever** (they're deliberate saves) and are never auto-deleted.

**Sync, back up, export**

- **GitHub backup (optional, your account):** push dictated notes as dated Markdown to a private repository you control, and optionally back up every transcript.
- **Export:** export transcripts to **Markdown / CSV / JSON** locally — no account needed.

**Stays out of your way**

- **All hotkeys rebindable** from the Hotkeys tab, with per-bind **press / release** triggers and inline conflict reporting.
- **Model manager:** download and switch between curated Whisper models (tiny → large-v3-turbo) from the Models tab; they run entirely offline after the one-time download.
- **Auto-updates:** Scribe checks GitHub for new releases shortly after launch, on refocus, and every six hours, fires an OS notification when one is found, and installs it in-app. The updater artifact is cryptographically signed. Automatic checks can be turned off.
- **Tray app:** closing the window minimizes to the tray; dictation hotkeys keep working. Quit from the tray icon.

Windows 10/11 x64 only, by design. There are no plans to over-build this for other platforms.

## Install (for users)

1. Download the `Scribe_<version>_x64-setup.exe` installer from the [latest release](https://github.com/n8watkins/scribe/releases/latest) and run it.
   - The binary is not code-signed (no Authenticode certificate), so Windows SmartScreen will warn you. Click **More info → Run anyway**.
   - The installer bootstraps Microsoft WebView2 automatically if you don't have it.
2. Launch Scribe. Open the **Models** tab and download a model — `base.en` (~140 MB) is a good start; `small.en` is more accurate and still fast on modern CPUs (the default is `small.en-q5_1`, a quantized balance). Models download once from [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp) and run entirely offline afterward.
3. Open the **Audio** tab, pick your microphone, and use **Record test** / **Play test** to confirm it hears you.
4. Put your cursor in any app, hold `Ctrl+Win`, and talk.

To use selected-text transform or note analysis, run a local OpenAI-compatible LLM server ([LM Studio](https://lmstudio.ai/) or [Ollama](https://ollama.com/)) and point Scribe at it in Settings (default `http://127.0.0.1:1234/v1`).
These features are optional and off by default; core dictation needs no LLM.

### Default hotkeys

| Action | Default |
| --- | --- |
| Hold to talk | `Ctrl+Win` (hold) |
| Toggle dictation | `` ` `` (tilde) |
| Dictate a note | hold the toggle key + tap `Q` |
| Paste last transcript | `Ctrl+Alt+V` |
| Transform selection | `Ctrl+Alt+R` |
| Discard / cancel recording | `Ctrl+Alt+X` |
| Open dashboard | `Ctrl+Alt+F` |

If a hotkey conflicts with another app, rebind it in the **Hotkeys** tab — conflicts are reported inline. Each non-hold bind can fire on key **press** or **release**.

### Privacy

Audio is captured to a temp folder, transcribed locally, and the temp audio is deleted.
Transcripts live in a local SQLite database under `%APPDATA%\com.natkins.scribe\` (the **Data & Privacy** tab can open the folder, clear history, or disable history entirely).
Core dictation does not upload your audio or transcripts.
Network access is used for model downloads you trigger, update checks against GitHub, the GitHub authorization flow, and optional GitHub backup when you enable it.
Optional LLM features send their prompts to the OpenAI-compatible server URL you configure, which defaults to a local LM Studio endpoint.

## Building from source

Prerequisites: Windows 10/11 x64, [Rust](https://rustup.rs/) (stable, MSVC toolchain), Node.js 20+, the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) (Visual Studio Build Tools with the Desktop C++ workload), CMake, Ninja, and the [Vulkan SDK](https://vulkan.lunarg.com/sdk/home#windows) version 1.4.350.0.

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

For development, `npm run tauri dev` gives hot reload. `cargo test` in `app/src-tauri/` runs the backend tests. Note that most of the audio/hotkey/paste code is `#[cfg(windows)]`-gated — compiling on non-Windows hosts proves little about the Windows build.

The [documentation index](docs/README.md) separates current operating guidance from historical plans and reports.

## Roadmap / known gaps

- **Custom models** — you download from a curated catalog (English-only and multilingual Whisper models); pointing Scribe at an arbitrary local `ggml` `.bin` isn't supported in the UI yet.
- Transcript search uses SQL `LIKE`; fine for thousands of entries, not millions.

Contributions and issue reports are welcome.

## License

[MIT](LICENSE)

## Release integrity

Scribe's Windows executables and installers are intentionally not Authenticode-signed, so Windows SmartScreen can show an unrecognized-app warning on first install.
Authenticode code signing is not on the roadmap.
This is separate from Tauri updater signing: update metadata and updater artifacts are cryptographically signed so the installed app can reject tampered updates.
Release automation also publishes SHA-256 checksums and a software bill of materials, then smoke-tests both the NSIS and MSI installers before publication.
