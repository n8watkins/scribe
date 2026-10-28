# Lane 1: architecture and code quality

Assessed 2026-10-27 on `main` at `1d88fe9` (branch `docs/assessment-2026-10`).
All paths are relative to the repository root; `src-tauri` means `app/src-tauri`.

## Overview

Scribe is a Tauri v2 desktop app: a Rust backend of 21,202 lines in 35 modules (`src-tauri/src`), a React 19 and TypeScript front end of 10,857 lines (`app/src`), a 1,000-line Rust CLI (`cli/`), and a 650-line TypeScript MCP server (`integrations/mcp-server/`).
The backend is synchronous by design: `std::thread`, `Mutex`, blocking `reqwest`, and `tiny_http`, with no async runtime of its own.
The code is unusually well commented, the error type is consistent (`CommandError` with a code and a user-readable message), the dictation state machine is small and tested, and the Rust side has 263 unit tests that all pass along with a clean clippy run.
The weak points are structural rather than stylistic.
Almost every Tauri command is synchronous and therefore runs on the main (UI) thread, so slow work (transcription, the selection-transform LLM call, LLM health checks, exports) can freeze every window.
Settings are written as one whole JSON object from two windows that each cache their own copy.
Several features are half built or dead (`recording_mode`, the custom data folder, the paste-method setting, the old update checker, the `app_stats_daily` table).
The front end has almost no tests (2 files, 10 tests), the Windows-only code (about 80 `cfg(windows)` blocks covering audio capture, hotkeys, clipboard, and paste) is compiled and tested only in CI, and there is no end-to-end test of any kind, although the ESLint config claims E2E coverage.

## Module map

Lines are total and non-test (code before the first `#[cfg(test)]`); "tests" counts `#[test]` functions.

| Module | Lines (code) | Tests | Responsibility | Health |
| --- | --- | --- | --- | --- |
| `audio.rs` | 1734 (1615) | 9 | Mic capture via cpal, worker and timeout threads, silence trim, resample, failed-clip handling | Large; 34 `cfg(windows)` blocks; Linux tests cover only pure helpers |
| `commands.rs` | 1482 (1482) | 0 | 68 of the 69 Tauri commands plus `BackendState`; also holds export, import and GitHub orchestration logic | Oversized, untested, mostly synchronous commands |
| `hotkeys.rs` | 1585 (1426) | 11 | Global shortcuts, a native modifier-chord watcher, toggle watcher, Q note chord, rebinding | Complex threading; Windows paths CI-only |
| `db.rs` | 1842 (1075) | 26 | SQLite schema, migrations, settings JSON, history, notes, stats, retention | Well tested; fragile migration scheme |
| `output.rs` | 1114 (1014) | 3 | Paste, copy, clipboard snapshot and restore, keystroke insert, focus guard | Thin tests; fixed timing constants |
| `settings.rs` | 1744 (899) | 38 | `AppSettings`, defaults, validation, default migrations | Good tests; carries inert fields |
| `model_manager.rs` | 768 (768) | 0 | Model catalog download, SHA-256 verify, cancel, delete, select | No tests at all |
| `whisper_server.rs` | 907 (690) | 12 | Resident `whisper-server.exe` lifecycle, health, idle shutdown, GPU to CPU retry | Solid; no OS-level child cleanup on crash |
| `dictation.rs` | 749 (689) | 3 | Stop to transcribe to output pipeline, selection-transform finish, failed-clip quarantine | Core path, lightly tested |
| `state_server.rs` | 918 (636) | 9 | Loopback HTTP and SSE dictation-state server with a token | Good; `lock().unwrap()` in the hub |
| `github_oauth.rs` | 906 (599) | 15 | GitHub device flow, token in the OS keyring | Good |
| `incremental.rs` | 799 (566) | 14 | Transcribes finished phrases while the user is still talking | Good |
| `lib.rs` | 559 (559) | 0 | Plugin setup, startup migrations, window events, command registry | Fine; one stale hard-coded caption color |
| `github_backup.rs` | 737 (449) | 9 | GitHub Contents API backup of notes and transcripts | Good |
| `whisper.rs` | 710 (444) | 15 | `whisper-cli` fallback, argument building, output parsing | Good |
| `note_sync.rs` | 320 (288) | 2 | Debounced auto-sync worker | OK |
| `note_analysis.rs` | 438 (261) | 9 | OpenAI-compatible LLM client for notes and transform | OK |
| `selection_transform.rs` | 386 (258) | 6 | Copy selection, LLM rewrite, paste back | Correctness bugs (findings 2 and 3) |
| `tray.rs` | 254 (254) | 0 | Tray menu and start/stop helpers shared with hotkeys | Untested; synchronous stop path |
| `models.rs` | 338 (248) | 6 | Static model catalog | Good |
| `gpu.rs` | 295 (225) | 6 | Vulkan device probe (async, cached) | Good |
| `file_transcribe.rs` | 274 (225) | 5 | File transcription with optional ffmpeg | OK |
| `import.rs` / `export.rs` | 289 / 245 | 6 / 7 | JSON restore with SHA-256 fingerprint; Markdown, CSV, JSON export | Good |
| `app_state.rs` | 354 (161) | 10 | Dictation state machine | Good |
| `update_check.rs` | 141 (116) | 3 | Old GitHub REST update check | Dead: no caller in the front end |
| Small modules | under 240 each | 41 total | `text_replace`, `filler`, `status_file`, `transcript`, `dictation_state`, `sync_history`, `error`, `stats`, `main` | Good |

Front end:

| Area | Lines | Notes |
| --- | --- | --- |
| `App.tsx` | 1148 | One `App` component of about 860 lines with 31 hooks, 17 event listeners, all settings writes, and `renderView` prop drilling |
| `PillApp.tsx` | 772 | Floating pill; duplicates the stop-then-transcribe flow from `App.tsx` |
| `backend.ts` | 1004 | Typed `invoke` wrappers; 5 exports have no caller |
| `views/*.tsx` | 13 views, 16 to 914 lines | `Settings.tsx` 914, `History.tsx` 725, `Sync.tsx` 710 |
| CSS | 4258 | `App.css` alone is 2647 lines |
| Tests | 2 files, 10 tests | `views/Sync.test.tsx`, `lib/syncActivity.test.ts` |

`cli/` (Rust, 40 tests) and `integrations/mcp-server/` (TypeScript, 24 tests) are thin read-only clients of the state server and are in good shape, but neither is shipped in a release (`.github/workflows/release.yml` builds only the desktop installers).

### How the parts talk

- Front end to backend: 69 Tauri commands registered in `src-tauri/src/lib.rs:435-504`, wrapped in `app/src/backend.ts`.
- Backend to front end: events such as `scribe:app-state`, `audio://level`, `scribe:dictation-transcribed`, `model://download-progress`; `App.tsx:614-735` and `PillApp.tsx:208-382` subscribe.
- Hotkeys: the global-shortcut plugin handler (`lib.rs:266`, `hotkeys.rs:438`) runs on the main thread; modifier-only chords (Ctrl+Win) and the toggle key run on dedicated watcher threads (`hotkeys.rs:806`).
- Transcription: a resident `whisper-server.exe` on an ephemeral loopback port (`whisper_server.rs:473-545`), with `whisper-cli` as a fallback.
- External tools: `state_server.rs` serves `/v1/status` and `/v1/events` on 127.0.0.1 with a read token, and writes `~/.scribe/control.json`; `status_file.rs` writes a fallback status file.

## Check results

Run on WSL (Linux), so every `cfg(windows)` branch is excluded; CI compiles and tests those on `windows-latest`.

| Check | Result |
| --- | --- |
| `cargo test` (`app/src-tauri`) | Pass: 262 passed, 0 failed, 30.4 s (one more test is Windows-only, 263 in total) |
| `cargo clippy --all-targets -- -D warnings` | Pass, no warnings (Linux target, so `cfg(windows)` code is not linted here; CI lints it on Windows) |
| `npm run lint` (`app`) | Pass, 0 warnings (`--max-warnings=0`) |
| `npm test` (`app`) | Pass: 2 files, 10 tests, 1.8 s |
| `npx tsc --noEmit` (`app`) | Pass, no errors |
| `npm test` and `npm run typecheck` (`integrations/mcp-server`) | Pass: 24 tests, no type errors |

Only one lint suppression exists in the front end (`app/src/views/Settings.tsx:526`), and `app/eslint.config.js:22-27` turns off `react-hooks/set-state-in-effect` and `react-hooks/refs` with a comment that says the patterns "are covered by E2E flows"; no E2E tests exist anywhere in the repository.

### Code that only CI compiles

About 80 `cfg(windows)` blocks are invisible to a Linux `cargo test` or `cargo clippy`: `audio.rs` (34), `hotkeys.rs` (19), `commands.rs` (8), `selection_transform.rs` (7), `gpu.rs` (2), `dictation.rs` (2), `lib.rs` (2), `output.rs` (the whole `platform` module from line 552), and `state_server.rs` (5).
These are the riskiest parts of the app (clipboard, `SendInput`, focus, mic capture, chord watchers), and their tests are mostly for pure helpers.
CI does run clippy and tests on `windows-latest` (`.github/workflows/ci.yml:17-40`), so they are compiled and linted before merge, but nothing exercises them at runtime.

## Findings

Severity is high, medium, or low; effort is S (hours), M (a day or two), or L (several days).

### 1. Slow work runs on the main thread and freezes the app (high, M)

- **What:** 60 of the 69 registered commands are synchronous Rust functions, and Tauri v2 runs non-`async` commands on the main thread.
  Among them: `transcribe_recording` (`src-tauri/src/commands.rs:1304`), `llm_status` (`commands.rs:831`, a network call with a 5 s timeout, `note_analysis.rs:17`), `record_test_clip` (`commands.rs:607`, blocks for the clip length, 3 s by default, `audio.rs:42`), `export_transcripts` and `restore_transcript_import` (`commands.rs:1061`, `commands.rs:1232`), and the paste commands, which sleep 240 ms (`output.rs:371-377`).
  The global-shortcut plugin handler also runs on the main thread, and `handle_release_edge` (`hotkeys.rs:942`) and `run_action` (`hotkeys.rs:965`) call `tray::stop_dictation` there, which transcribes synchronously (`tray.rs:184`).
  A voice Transform Selection stop runs the LLM call in that path with a 180 s timeout (`dictation.rs:421`, `note_analysis.rs:14`).
- **Where it shows:** the pill's stop button (`app/src/PillApp.tsx:625`), the dashboard's stop button (`app/src/App.tsx:870`), the tray "Stop dictation" item, any hotkey bound to a normal key combination (Ctrl+Alt+R by default), and the Settings LLM "Test" button.
- **Why it matters:** while the main thread is blocked, every webview, the pill animation, the tray, and further global hotkeys stop responding; a slow local LLM can make Scribe look hung for minutes.
  The default Ctrl+Win and backquote paths escape this only because their watchers run on their own threads.
- **Fix:** make every command that touches disk, network, audio, or Whisper `async` with `spawn_blocking` (as `transcribe_file`, `check_for_update`, and `probe_gpu_devices` already do), and run hotkey and tray actions on a single dedicated worker thread (a channel-fed "dictation actor") instead of in the plugin callback.
- **Effort:** M.

### 2. Transform Selection with nothing selected rewrites the old clipboard (high, S)

- **What:** `capture_selection` (`src-tauri/src/selection_transform.rs:98-121`) snapshots the clipboard, sends Ctrl+C, sleeps 120 ms, and reads the clipboard as "the selection".
  It never empties the clipboard first and never checks `GetClipboardSequenceNumber`, so when nothing is selected (most apps leave the clipboard untouched on an empty Ctrl+C) it reads whatever was already on the clipboard.
  The empty-selection guard at `src-tauri/src/hotkeys.rs:1124` therefore only fires when the clipboard was also empty.
- **Why it matters:** the user's earlier clipboard text (which can be a password or private text) is sent to the LLM endpoint and the rewrite is pasted at the cursor, which is surprising and a privacy leak.
  The same 120 ms race means a slow app can also return stale contents when text is selected.
- **Fix:** record the clipboard sequence number before Ctrl+C and poll (up to about 500 ms) until it changes; if it does not change, treat the selection as empty, restore the snapshot, and show the "Highlight some text first" notice.
- **Effort:** S.

### 3. Transform Selection leaves the user's clipboard overwritten on failure or cancel (medium, S)

- **What:** the snapshot taken in `capture_selection` is restored only inside `apply_result` (`selection_transform.rs:131`), which runs only after a successful LLM call (`dictation.rs:421-428`).
  If the LLM fails or times out, the instruction recording is discarded (Ctrl+Alt+X) or too short, or the selection is empty (`hotkeys.rs:1124-1129`), the clipboard keeps the copied selection and the user's original clipboard is lost.
  A discarded transform also leaves its `pending_transform` in `BackendState` (`commands.rs:62`) until the next transform.
- **Why it matters:** the app promises clipboard preservation everywhere else (`output.rs:357-419`).
- **Fix:** move the snapshot into a guard type whose `Drop` restores the clipboard unless `apply_result` consumed it, and clear `pending_transform` on cancel.
- **Effort:** S.

### 4. Ctrl+Alt+R is always registered, and transform ignores the AI switch (medium, S): open problem 3 confirmed

- **What:** `register_hotkey_set` (`src-tauri/src/hotkeys.rs:706-760`) registers every entry of `HOTKEY_ACTIONS` (`hotkeys.rs:136-143`) unconditionally, including `TransformSelection` (default `Ctrl+Alt+R`, `settings.rs:529`).
  `toggle_transform` (`hotkeys.rs:1100`) and `finish_selection_transform` (`dictation.rs:404`) never read `notes_analysis_enabled`; the only check of that flag in the backend is `analyze_note` (`commands.rs:801`).
- **Why it matters:** Scribe takes Ctrl+Alt+R away from every other app (it is a common shortcut in other tools, and on keyboard layouts with AltGr, Ctrl+Alt acts as AltGr), and with AI off a press copies the selection, records, and then fails against `http://localhost:1234` by default.
- **Fix:** skip registering `TransformSelection` while the AI features are off (and re-register on toggle in `update_settings`), and check the flag in `toggle_transform` as a second guard.
  Consider a less collision-prone default.
- **Effort:** S.

### 5. The paste method has no setting (medium, S): open problem 2 confirmed

- **What:** `AppSettings.paste_method` (`src-tauri/src/settings.rs:141`, enum at `settings.rs:471`) is honored by `output.rs:199-216`, and `DirectInsert` is documented as the fallback "for apps that block synthetic Ctrl+V" (`settings.rs:472-476`), but no view writes `pasteMethod`.
  The only front-end reference is a read in `app/src/lib/format.ts:57`.
  The v5 settings migration (`settings.rs:763-769`) moved everyone off `DirectInsert`, so no user can reach it.
- **Why it matters:** users of apps where paste fails (some terminals, remote desktops, games, admin windows) have no recovery path.
- **Fix:** add a "Paste method: Paste (recommended) / Type it out" control next to the output mode in Settings, with a one-line explanation.
- **Effort:** S.

### 6. Settings are saved as a whole object from two windows with separate caches (medium, M)

- **What:** `update_settings` (`src-tauri/src/commands.rs:139`) replaces the whole `AppSettings`.
  The main window merges a patch into its cached copy (`app/src/App.tsx:753-786`), and the pill does the same with its own cached copy when it is dragged (`app/src/PillApp.tsx:428-441`).
  The backend never broadcasts a settings-changed event (no such event exists), the pill refetches settings only on app-state changes (`PillApp.tsx:312-320`), and other commands change settings behind both caches (`rebind_hotkey`, `set_hotkey_trigger`, `select_model`, `github_disconnect`).
- **Why it matters:** last writer wins with stale data.
  Example: rebind a hotkey, then move the pill before the next dictation; the pill's stale copy is saved, `update_settings` sees `previous.hotkeys != settings.hotkeys` (`commands.rs:155-157`) and re-registers the old binding.
  The reverse (main window toggles a setting and writes back an old `pillX`/`pillY`) is likelier and silently resets the pill position.
- **Fix:** replace whole-object writes with a backend `patch_settings` command that applies only the changed fields under the DB lock, and emit `scribe:settings-changed` with the canonical settings after every write so every window stays in sync.
- **Effort:** M.

### 7. Database migrations are not versioned or transactional (medium, S)

- **What:** `apply_migrations` (`src-tauri/src/db.rs:901-922`) re-runs every migration on every launch and treats a "duplicate column name" error string as "already applied".
  `migrations/004_note_analysis.sql` holds three `ALTER TABLE` statements in one batch; `execute_batch` is not transactional, so if the first succeeds and a later one fails (crash, disk full), every later launch hits the duplicate-column error on the first statement and never adds the other two, and every transcript `SELECT` (`db.rs:150-155`) then fails.
  No `PRAGMA user_version`, `journal_mode`, or `busy_timeout` is set.
- **Why it matters:** the database holds all of a user's history and notes, and the next schema change will be harder to get right with string matching.
- **Fix:** track the schema version with `PRAGMA user_version`, run each pending migration in its own transaction, and set `journal_mode=WAL` and a `busy_timeout`.
  Add a test that opens a database at each historic version.
- **Effort:** S.

### 8. Dead and half-built code paths (medium, S)

- `recording_mode` is inert (`src-tauri/src/settings.rs:72-75`) but still serialized and typed in `app/src/backend.ts:24`, `backend.ts:138`.
- A custom data folder exists only in part: `pick_data_dir` (`commands.rs:661`) and `effective_data_dir` (`commands.rs:627`) read `settings.data_dir`, but the database, clips, and models never use it (`lib.rs:280-289`, `model_manager.rs:663`), and `pickDataDir` (`app/src/backend.ts:690`) has no caller, so the "Local data folder" shown in Data & Privacy (`app/src/views/DataPrivacy.tsx:234`) would point at the wrong place if it were ever set.
- `update_check.rs` and the `check_for_update` command (`commands.rs:843`) are unused since detection moved to the updater plugin (`app/src/lib/updates.ts`); `checkForUpdate` in `backend.ts:723` has no caller.
- The `app_stats_daily` table (`migrations/001_initial.sql:36-42`) is never read or written; stats are recomputed from `transcripts` (`db.rs:594-640`).
- Unused front-end exports: `getTranscript`, `updateTranscript`, `refreshBasicStats`, `pickDataDir`, `checkForUpdate` in `app/src/backend.ts`.
- `crate-type = ["staticlib", "cdylib", "rlib"]` in `src-tauri/Cargo.toml:14` builds two unused library artifacts per build; the unmerged `feat/rust-storage-profiles` branch fixes this.
- **Why it matters:** dead settings and commands confuse future work and enlarge the IPC surface.
- **Fix:** delete `update_check.rs`, the `check_for_update` command, the unused exports and the `app_stats_daily` table (in a migration); either finish the data-folder feature (move DB, clips, and models with a restart) or remove `data_dir` and `pick_data_dir`; mark `recording_mode` with `#[serde(default, skip_serializing)]` and drop it from the TypeScript type.
- **Effort:** S.

### 9. Stats are wrong for most time zones and shrink with retention (medium, S)

- **What:** "today" is `Utc::now().date_naive()` compared against the UTC `created_at` prefix (`src-tauri/src/db.rs:595`, `db.rs:611`).
  Total words is `SUM(word_count) FROM transcripts` (`db.rs:617-622`), and transcripts are pruned after 30 days by default and are not stored at all when history is off.
- **Why it matters:** for a user in US Pacific time, "Words today" resets at 4 or 5 pm local time; "Total words" drops every day once retention starts deleting, and is always zero with history off.
- **Fix:** compute "today" in local time (pass the local date range from the front end or use `chrono::Local`), and keep running totals in the existing `app_stats_daily` table, updated on each dictation, so stats survive retention.
- **Effort:** S.

### 10. Clipboard paste timing is hard-coded and every dictation enters clipboard history (medium, S)

- **What:** `clipboard_paste` waits 60 ms before and 180 ms after Ctrl+V before restoring (`src-tauri/src/output.rs:371-377`), and the code itself notes slow targets may need more.
  The snapshot copies every clipboard format, including large images, on every dictation (`output.rs:810-858`).
  The transcript is placed on the clipboard without the `ExcludeClipboardContentFromMonitorProcessing` or `CanIncludeInClipboardHistory` formats (no reference in the codebase), so every dictation lands in Windows clipboard history (Win+V) and, if enabled, cloud clipboard sync.
- **Why it matters:** in slow targets (remote desktop, Electron apps under load, VMs) the restore can race the paste and the old clipboard gets pasted instead of the transcript; and transcripts persist in clipboard history after the app has "restored" the clipboard, which contradicts the privacy story.
- **Fix:** register the two exclusion formats when Scribe writes the clipboard, make the post-paste delay adaptive or configurable (an "Advanced" setting), and skip snapshotting formats above a size cap.
- **Effort:** S.

### 11. A Scribe crash orphans `whisper-server.exe` (medium, S)

- **What:** the resident server is killed only by `Drop` and the `RunEvent::Exit` handler (`src-tauri/src/whisper_server.rs:132-141`, `lib.rs:510-524`); it is not placed in a Windows Job Object with kill-on-close.
- **Why it matters:** after a crash, a kill from Task Manager, or a forced update, the server keeps up to 1.6 GB of model in RAM or VRAM until reboot, and each relaunch can start another.
- **Fix:** create a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` at startup and assign every spawned child (whisper-server, whisper-cli, ffmpeg) to it.
- **Effort:** S.

### 12. Lock poisoning can brick the session (low, S)

- **What:** `BackendState::db()` and `app_state()` (`src-tauri/src/commands.rs:77-88`) turn a poisoned mutex into a permanent error, and the state-server hub uses `lock().unwrap()` (`state_server.rs:109-169`), which would panic every thread that publishes state after any panic while that lock was held.
- **Why it matters:** one panic in a worker thread can leave every later command failing with "Could not access database" until restart.
- **Fix:** recover with `unwrap_or_else(PoisonError::into_inner)` where the guarded data stays consistent (as `github_sync_lock` already does at `commands.rs:107-111`), and use `parking_lot::Mutex`, which does not poison.
- **Effort:** S.

### 13. A corrupt settings value silently resets everything (low, S)

- **What:** any `serde_json` error in `get_settings` (`src-tauri/src/db.rs:57-86`) replaces the whole settings object with defaults, backing up the raw value under one key and logging it.
  A single unknown enum value or a field type change (for example after a downgrade or a bad future migration) therefore discards hotkeys, model, GitHub backup, and retention choices.
- **Why it matters:** users lose their setup with no notice.
- **Fix:** deserialize leniently (`serde_json::Value` first, then merge field by field over defaults, dropping only invalid fields), and tell the user in the UI when anything was reset.
- **Effort:** S.

### 14. `App.tsx` and `commands.rs` are god files (low, M)

- **What:** `App` (`app/src/App.tsx:199-1060`) owns all data loading, 17 event subscriptions, settings writes, toasts, update logic, and navigation, and passes an `actions` object into every view (`renderView`, `App.tsx:1061`).
  `commands.rs` mixes thin command shims with business logic (export at `commands.rs:1061-1180`, import at `commands.rs:1182-1300`, GitHub orchestration) and has no tests.
  The stop-then-transcribe flow is duplicated in `App.tsx:862-880` and `PillApp.tsx:619-632`.
- **Why it matters:** every change touches the same files (the old handoff notes merge conflicts in `settings.rs`, `backend.ts`, `lib.rs`, and `App.tsx`), and untestable shapes keep coverage low.
- **Fix:** split `App.tsx` into a settings store hook, an events hook, and an updates hook; move command bodies into their domain modules and keep `commands.rs` as one-line shims, grouped per domain; add a single backend `stop_and_transcribe` command used by both windows.
- **Effort:** M.

### 15. Test coverage gaps (medium, M)

- **What:** the front end has 10 tests in 2 files; `commands.rs` (1482 lines), `model_manager.rs` (768 lines), `tray.rs`, and `lib.rs` have none; `output.rs` has 3 and `dictation.rs` has 3 for the two most important paths in the app.
  There is no end-to-end or smoke test of the running app, although `app/eslint.config.js:22-24` says patterns are "covered by E2E flows".
- **Why it matters:** the riskiest code (paste, clipboard, hotkeys, downloads, settings writes) is exactly the code without tests, and Windows-only code is never executed in CI.
- **Fix:** add Vitest tests for `persistSettings`, the Settings view, Hotkeys rebinding, and History using `@tauri-apps/api/mocks`; add Rust tests for `model_manager` with a local HTTP fixture (size mismatch, hash mismatch, cancel, resume); add a Windows CI job that launches the built app with a test profile and drives it through the state server, at least checking startup, a test clip, and a transcription of a bundled WAV.
  Correct the ESLint comment until then.
- **Effort:** M.

### 16. CI gaps (low, S)

- Node 20, end of life since April 2026, in `ci.yml:49`, `ci.yml:104`, and `release.yml:97` (already listed as open problem 6).
- The `rust-security` job audits only `app/src-tauri/Cargo.lock` (`ci.yml:121-131`), not `cli/Cargo.lock`.
- The front-end job runs on Linux, which is right, but there is no `tsc --noEmit` step separate from `npm run build`; this is fine today because `build` runs `tsc` first.
- No job builds or ships the CLI or MCP server, so the documented `scribe` CLI (`cli/README.md`) can only be had by building from source.
- **Fix:** move to Node 24, audit both lockfiles, and either publish the CLI and MCP server as release assets or say clearly in the README that they are source-only.
- **Effort:** S.

### 17. Small correctness and polish items (low, S)

- `style_native_titlebar` (`src-tauri/src/lib.rs:531-559`) hard-codes the Midnight background `#070b14` as the caption and border color, but the window has `decorations: false` (`tauri.conf.json:18`) and seven themes; on Windows 11 the DWM border stays dark navy under the light Pewter theme.
  Drive it from the active theme or remove it.
- `CloseRequested` reads and deserializes the whole settings JSON from SQLite on the main thread on every close click (`lib.rs:416-433`); the same happens per hotkey press for `dashboard_hotkey_toggles` (`hotkeys.rs:981-986`).
  Cache settings in memory behind an `RwLock`, refreshed on save.
- `save_last_transcript_with_history` performs four writes without a transaction (`db.rs:428-450`).
- The unmerged branch `feat/rust-storage-profiles` (5 commits) carries a real fix: the NSIS smoke-test uninstall in `release.yml` handles quoted `UninstallString` paths, plus smaller debug artifacts (`app/src-tauri/target` is 4.6 GB locally).
  Rebase and merge it rather than drop it.

## What's solid

- The state machine (`src-tauri/src/app_state.rs`) is small, explicit, and tested, and every failure path returns to Idle (`dictation.rs:543-580`, self-healing from Error after 5 s at `dictation.rs:598`).
- Errors are consistent: one `CommandError` with a stable code and a plain-English message (`src-tauri/src/error.rs`), surfaced as toasts.
- Defensive startup: corrupt settings self-heal, the pre-rebrand data migration is resumable and atomic (`lib.rs:66-183`), and panics on any thread are logged (`lib.rs:195-226`) with rotating 5 MB logs (`lib.rs:242-261`).
- Model downloads check both size and SHA-256, stream to a `.part` file, and support cancel and retry (`model_manager.rs:378-530`).
- The whisper-server integration keeps the model warm, restarts on config change, shuts down when idle, and retries on CPU when the GPU fails (`whisper_server.rs:186-420`).
- Incremental transcription is a genuine latency win and is well tested (14 tests in `incremental.rs`).
- The state-server, CLI, and MCP server share a frozen wire contract (`docs/integrations/dictation-state-contract.md`) and are each tested against it.
- Lint and type checks are strict (`--max-warnings=0`, TypeScript `strict`, clippy `-D warnings` in CI on Windows), and all pass.
- CI pins every action by SHA, uses least-privilege `permissions`, audits npm and cargo, and the release workflow validates versions, checksums, and both installers.

## What's fragile

- Main-thread blocking (finding 1): the app's responsiveness depends on which hotkey or button started the dictation.
- Clipboard choreography (findings 2, 3, 10): fixed sleeps of 60, 120, and 180 ms, no sequence-number checks, and restore paths that only run on success.
- Settings consistency (findings 6, 13): whole-object writes from multiple caches, and an all-or-nothing deserialize.
- Windows-only code (about 80 `cfg(windows)` blocks) is compiled in CI but never run by any automated test.
- Migrations by error-string matching (finding 7).
- Hotkeys: two watcher threads plus the plugin, a temporary Q grab during the toggle hold, and a documented deadlock when plugin registration runs on the main thread (`hotkeys.rs:1036-1038`), which `update_settings` and `rebind_hotkey` do anyway because they are synchronous commands.
