# LocalDictate - Status and Next Steps

Status: Windows build fixed and installers produced; ready for manual Windows QA  
Last updated: 2026-06-10  
Repository: `https://github.com/n8watkins/localdictate`  
Visibility: Private

## Windows Build Fix and Packaging (2026-06-10)

The Windows build of the prior `main` failed with 170 compile errors. Root causes, fixed in commit `721ed8a`:

- cpal's platform `Stream` wrapper is deliberately `!Send`; it was stored in `AudioService` -> `BackendState` -> `app.manage()` (which requires `Send + Sync`), cascading into every Tauri command. Fixed by using the raw `WasapiHost`/`WasapiDevice`/`WasapiStream` types, which are `Send`.
- `PKEY_Device_FriendlyName` and `IMMDevice::OpenPropertyStore` require the `Win32_UI_Shell_PropertiesSystem` feature on the `windows` crate.
- `WasapiStream` has no `Debug` impl, so the unused `derive(Debug)` on `AudioService` and `RecordingSession` was removed.

Lesson: almost all audio/hotkey/paste code is `#[cfg(windows)]`-gated, so `cargo check` in WSL never compiles it. Windows changes must be verified with the Windows toolchain, e.g. from WSL: `cmd.exe /c "cd /d C:\Users\natha\Projects\Tools\localdictate\app\src-tauri && cargo check"`.

Verified on Windows (clone at `C:\Users\natha\Projects\Tools\localdictate`): `cargo check` clean, 22/22 tests pass, and `npm run tauri build` produced:

```text
app/src-tauri/target/release/bundle/nsis/LocalDictate_0.1.0_x64-setup.exe
app/src-tauri/target/release/bundle/msi/LocalDictate_0.1.0_x64_en-US.msi
```

The MSI payload was extracted and confirmed to contain `bin/windows/whisper-cli.exe`, `whisper.dll`, and `ggml*.dll` at the resource root, matching the `$RESOURCE/bin/windows/whisper-cli.exe` path `whisper.rs` resolves. `whisper-cli.exe --help` runs standalone with only those DLLs. Extra whisper.cpp binaries (bench, server, stream, talk-llama, SDL2.dll, etc.) were moved out of `resources/` into `whisper-extras-unbundled/` at the Windows repo root so they are not bundled.

Remaining before release: run the manual checklist in [V1_WINDOWS_QA_CHECKLIST.md](./V1_WINDOWS_QA_CHECKLIST.md) (real microphone, hold-to-talk, transcription, paste targets, tray/hotkeys).

## Current State

LocalDictate is now a Tauri v2 + React + TypeScript desktop app with the main V1 feature lanes implemented and pushed to `main`.

The app includes:

- Rust backend state machine, settings, SQLite persistence, migrations, and Tauri commands.
- Command-backed React UI for dashboard, settings, history, models, audio, hotkeys, and transcript actions.
- System tray and global hotkey backend integration.
- Microphone enumeration, recording lifecycle, level events, timeout handling, and WAV normalization to 16 kHz mono PCM16.
- Whisper/model manager foundation using a bundled `whisper-cli.exe` resource path and downloadable local models.
- Last Transcript Buffer, output modes, copy/paste commands, and clipboard-preserving direct insert path.
- Searchable transcript history, edit/delete/clear actions, retention enforcement, and stats refresh.
- Final UX wiring for models, microphones, event refresh, in-app toasts, floating pill, close-to-tray behavior, and Windows packaging prep.

## What We Have Done

### Planning and docs

- Captured product requirements in [PRD.md](./PRD.md).
- Captured milestone plan in [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md).
- Captured multi-agent plan in [AGENT_ORCHESTRATION.md](./AGENT_ORCHESTRATION.md).
- Added Windows release QA checklist in [V1_WINDOWS_QA_CHECKLIST.md](./V1_WINDOWS_QA_CHECKLIST.md).

### Backend

- Added app state machine with `Idle`, `Recording`, `Stopping`, `Transcribing`, `Pasting`, `Ready`, `Error`, and `Paused`.
- Added settings defaults/schema matching the PRD baseline.
- Added SQLite foundation for settings, transcripts, models, and stats.
- Added transcript domain model and Last Transcript Buffer behavior.
- Added tray menu and global hotkey registration/rebinding.
- Added audio capture, normalized WAV output, level events, min/max duration rules, and temp lifecycle.
- Added model catalog, model download/select/delete/retry/cancel commands, and app data model storage.
- Added Whisper CLI invocation path, transcription runner, and dictation integration.
- Added output/copy/paste commands with direct insert and compatibility clipboard paste.
- Added transcript search, get, update, delete, clear history, retention, and stats commands.

### Frontend

- Replaced mock dashboard/settings/history data with Tauri command calls.
- Wired settings persistence.
- Wired Last Transcript Buffer copy/insert/clear actions.
- Wired History search, pagination, edit, delete, clear all, copy, and insert.
- Wired Models view to backend model commands and download progress.
- Wired Audio view to backend microphone enumeration and recording controls.
- Added event-driven refresh for app state, audio, dictation, output, model progress, history, and stats.
- Added in-app toasts and floating pill states.
- Kept existing visual language and compact desktop utility layout.

### Packaging prep

- Added Windows bundle/resource config for the expected Whisper CLI resource path.
- Added resource instructions at `app/src-tauri/resources/bin/windows/README.md`.
- Cleaned up Cargo metadata.
- Added close-to-tray behavior.
- Added Windows QA/release checklist.

## Current Commits

```text
e9a418f Polish UX wiring and Windows packaging
65d83b4 Implement transcript history retention and stats
ec054ea Implement output paste and clipboard commands
d8b4da1 Implement whisper transcription and model management
747ce5b Implement audio capture backend
fbd9750 Implement tray and global hotkeys
eb92546 Wire frontend to Tauri commands
8e8d59b Add backend foundation
629c847 Add LocalDictate status handoff
68b9592 Refine LocalDictate frontend UX
7d186c1 Initial LocalDictate scaffold and plans
```

## Verified Commands

Latest full hardening pass verified:

```bash
cd /home/natkins/personal/tools/localdictate/app/src-tauri
cargo fmt
cargo check
cargo test
```

Result: Rust checks passed; 22 tests passed.

Frontend build:

```bash
cd /home/natkins/personal/tools/localdictate/app
npm run build
```

Result: passed.

Tauri build on this Linux host:

```bash
cd /home/natkins/personal/tools/localdictate/app
npm run tauri build
```

Result: passed and built the Linux release app at:

```text
app/src-tauri/target/release/app
```

Important: this Linux host did not produce Windows NSIS/MSI installer artifacts. Windows packaging still needs to run on Windows.

## What Still Needs To Be Done

### Required before Windows MVP validation

1. Add the real Whisper CLI binary:

   ```text
   app/src-tauri/resources/bin/windows/whisper-cli.exe
   ```

   Do not ship a placeholder executable. Runtime transcription intentionally reports `missing_whisper_executable` until this binary exists in the packaged resources.

2. Build and smoke test on Windows:

   ```powershell
   cd app
   npm run tauri build
   ```

3. Run the Windows acceptance checklist:

   ```text
   docs/V1_WINDOWS_QA_CHECKLIST.md
   ```

4. Validate real hardware/system behavior:

   - Microphone enumeration and selection.
   - Recording and normalized WAV output.
   - Hold-to-talk press/release reliability.
   - `Ctrl+Alt+V` paste-last hotkey.
   - Direct insert into Notepad, browser input, VS Code, and elevated/admin apps.
   - Compatibility paste clipboard restore behavior.
   - Model download/select/delete/retry.
   - Whisper transcription using a downloaded model.
   - History, stats, retention, and Last Transcript Buffer behavior.

### Remaining polish/non-blocking V1 items

- Add production code signing before external distribution.
- Decide and record stable Windows installer upgrade behavior.
- Add native OS notifications if wanted; current feedback is in-app toast plus floating pill.
- Add Tauri autostart plugin wiring for `launchAtStartup`.
- Add final tray icon state variants when assets exist.
- Add open data/model folder commands; UI currently leaves these disabled.
- Consider SQLite FTS for large transcript histories later. Current search uses `LIKE`, which is acceptable for V1.

## Immediate Next Step

Start a Windows QA/resource packaging agent.

This is the correct next step because core implementation is complete and the remaining high-risk work is platform validation: bundled Whisper resource, installer output, Windows microphone behavior, Windows hotkey behavior, and Windows paste behavior.

## Fresh Context Start Prompt

Use this exact prompt in a new context:

```text
Start Windows QA and Packaging Agent for LocalDictate.

Repo path: /home/natkins/personal/tools/localdictate

Read these first:
- docs/STATUS_AND_NEXT_STEPS.md
- docs/V1_WINDOWS_QA_CHECKLIST.md
- docs/PRD.md sections 17, 18, 20, and 21
- app/src-tauri/resources/bin/windows/README.md
- app/src-tauri/tauri.conf.json
- app/src-tauri/src/whisper.rs
- app/src-tauri/src/audio.rs
- app/src-tauri/src/hotkeys.rs
- app/src-tauri/src/output.rs

Goal:
Prepare and validate the Windows MVP release path. Add the real whisper.cpp `whisper-cli.exe` resource if available, build on Windows, run installer/resource smoke tests, and execute the manual Windows QA checklist.

Do not expand product scope. Focus on packaging, resource correctness, Windows runtime validation, and clear fixes for issues found during QA.

Required checks:
- git status -sb
- cd app && npm run build
- cd app/src-tauri && cargo check
- cd app/src-tauri && cargo test
- cd app && npm run tauri build on Windows

Required manual QA:
- Follow docs/V1_WINDOWS_QA_CHECKLIST.md.
- Verify `app/src-tauri/resources/bin/windows/whisper-cli.exe` exists before packaging.
- Verify installed resources include `bin/windows/whisper-cli.exe`.
- Download/select a model from the Models view.
- Record from a real microphone.
- Transcribe locally with Whisper.
- Verify Last Transcript Buffer, History, and Stats update.
- Verify direct insert and compatibility paste in common Windows apps.
- Verify tray and global hotkeys while the main window is hidden/unfocused.

Deliverables:
- Fixes for any blocking Windows packaging/runtime bugs found.
- Updated docs/V1_WINDOWS_QA_CHECKLIST.md with pass/fail notes.
- A short release readiness summary.
- Commit and push when done.
```

## Takeover Notes

- `main` should be clean and match `origin/main` before starting.
- The app compiles and builds on Linux, but Windows installer output has not been validated.
- The expected Whisper binary path is intentionally documented but not committed with a real executable.
- Most remaining risk is Windows-specific runtime behavior, not missing application architecture.
