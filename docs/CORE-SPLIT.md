# Splitting the engine from the shell

Scribe is moving from a Tauri app to a native Slint app.
Phase 1 separates the engine (`crates/scribe-core`) from the interface inside today's shipping Tauri app, without changing what the app does.

- P1-1 moved the engine into `crates/scribe-core`.
- P1-2 (this step) added the seams: `Paths`, `CoreEvent`, `Host`, `Core`, and split the Tauri commands into one shell file per domain.
- P1-3 runs seven lanes in parallel, one per domain, each converting its engine modules from `AppHandle` to `&Core` and `Host`.
- P1-4 removes what the lanes leave behind (the `AppHandle` wrappers, the individually managed state, the `shell` free functions) and makes the engine free of Tauri.

Behaviour never changes along the way: same commands and names, same event names and payloads, same data folders, same installers.

## The seams

| File | What it is |
|------|------------|
| `crates/scribe-core/src/paths.rs` | `Paths`: every folder the engine uses (data, cache, log, resource, home), resolved exactly as Tauri resolves them, without Tauri. |
| `crates/scribe-core/src/events.rs` | `CoreEvent`: one variant per event the engine sends (22), carrying today's payload. `name()` is today's event name and the event serializes as its bare payload. |
| `crates/scribe-core/src/host.rs` | `trait Host`: what the engine needs from its shell (`emit`, `paths`, `flavor`, the tray and main-window calls). `Flavor` (Stable or Dev). `RecordingHost`, a host for tests. |
| `crates/scribe-core/src/core.rs` | `Core`: a cheap, cloneable handle owning the engine's parts (`backend()`, `warm_transcriber()`, `dictation_server()`, `note_sync()`, `dictation_worker()`, `hotkey_runtime()`, `replacements()`) and its `host()`, with `emit()`, `paths()` and `flavor()` shortcuts. |
| `crates/scribe-core/src/backend.rs` | `BackendState` (was in `commands.rs`): the database, the state machine, the audio service and the shared locks. |
| `crates/scribe-core/src/shell.rs` | The installed `Host` (`shell::host()`), and the old tray and window free functions, which now forward to it. |
| `app/src-tauri/src/tauri_host.rs` | `TauriHost`: `Host` on Tauri. `emit` sends `app.emit(event.name(), &event)`. |
| `app/src-tauri/src/commands/<domain>.rs` | The `#[tauri::command]` functions, one file per domain. `commands/mod.rs` has `off_main`, `off_main_core` and `on_dialog_thread`. |

At startup the shell resolves `Paths`, installs a `TauriHost` (first thing in setup), starts each part in the same order as before, manages each part in Tauri state as before, then builds the `Core` from those same parts and manages it, and only then creates the windows.
Each part is a cheap handle onto shared state, so `app.state::<BackendState>()` and `core.backend()` are the same instance.
Tests in `app/src-tauri/src/lib.rs` pin the startup order and the 79 command names; tests in `tauri_host.rs` check `Paths` against Tauri's resolver for both flavours.

## The pattern for a lane

A lane converts the engine modules it owns, one function at a time.

1. Find what ties the function to Tauri: `app.state::<X>()`, `app.try_state::<X>()`, `app.emit(...)`, `app.path()`, `is_dev_flavor(app)`, `display_name(app)`, `crate::shell::...(app, ...)`.
2. Write the engine version, named after the old function with `_with` added, taking `core: &Core` first (or `host: &dyn Host` when it needs nothing but the host).
   Replace `app.state::<BackendState>()` with `core.backend()`, the other parts with their `Core` accessors, `app.emit(name, payload)` with `core.emit(CoreEvent::Variant(payload))`, `app.path().app_cache_dir()` and friends with `core.paths().cache_dir()` and friends, `is_dev_flavor(app)` with `core.flavor().is_dev()`, and `crate::shell::open_dashboard(app, route)` with `core.host().open_dashboard(route)`.
3. Keep the old function, with its old name and signature, as a thin wrapper that finds the `Core` (or the host) and calls the engine version.
   Callers in other lanes keep compiling against the wrapper, untouched.
4. Switch the callers inside your own lane to the `_with` version.
5. Convert your commands in `app/src-tauri/src/commands/<domain>.rs` to take `core: tauri::State<'_, Core>` instead of `app: tauri::AppHandle` where they only needed the app for state, and run their blocking work with `off_main_core(core.inner().clone(), |core| ...)`.
   The windows pass no argument for `State`, so the command's interface does not change.
6. Test the engine version with `RecordingHost` and `Paths::from_parts` on a temporary folder: it records every event sent, so a test can assert the event name and payload.

Rules that keep behaviour the same:

- Send the same event, with the same `CoreEvent` variant whose `name()` is the name you replace; do not change a payload type (the `events.rs` tests pin every payload).
- Where a wrapper finds the `Core` with `app.state::<Core>()`, it must not run before the shell manages the `Core`, which happens at the end of setup.
  Code that can run earlier must use the host (`crate::shell::host()`, installed first thing in setup) or keep its old path: `hotkeys::setup`, `tray::setup`, `WarmTranscriber::new` and its watchdog, `state_server::start`, `NoteSyncWorker::spawn`, `DictationWorker::spawn`, `data_hygiene::run_startup_maintenance` (a thread started during setup), `autostart::reconcile_at_launch`, and a single-instance remote command (accepted as soon as the dictation worker exists).
  When in doubt, use `app.try_state::<Core>()` and fall back to the old path.
- Calls that need a Tauri plugin or a window (the opener, notifications, the global-shortcut plugin, the autostart plugin, webview windows, `app.config()`, `app.package_info()`) keep their `AppHandle` for now; list them for P1-4, which adds what the `Host` needs for them.

## Worked example: the status domain

`status_file.rs` needed the app only to find the cache folder.
Its engine versions take the host:

```rust
pub fn publish_with(host: &dyn Host, snapshot: &AppStateSnapshot) {
    let dir = host.paths().cache_dir();
    // ... unchanged: create the folder, build the payload, write it atomically
}

pub fn publish_idle_with(host: &dyn Host) { /* builds an Idle snapshot, calls publish_with */ }
```

The old functions stay as wrappers, so `dictation_state::emit_state_snapshot` (the `audio` lane) and the shell's setup and exit code keep calling `status_file::publish(app, snapshot)` and `status_file::publish_idle(app)` unchanged.
They use the installed host rather than the `Core`, because a state change can come from a remote command before setup has managed the `Core`:

```rust
pub fn publish(_app: &AppHandle, snapshot: &AppStateSnapshot) {
    match crate::shell::host() {
        Some(host) => publish_with(host.as_ref(), snapshot),
        None => log::warn!("Could not resolve cache dir for status file: no host yet"),
    }
}
```

A new test (`publish_writes_into_the_hosts_cache_folder`) runs the engine versions with a `RecordingHost` on a temporary folder, with no Tauri app.

The domain's command, `get_app_state`, takes the `Core`:

```rust
#[tauri::command]
pub async fn get_app_state(core: tauri::State<'_, Core>) -> Result<AppStateSnapshot, CommandError> {
    off_main_core(core.inner().clone(), move |core| {
        Ok(core.backend().app_state()?.snapshot())
    })
    .await
}
```

`state_server.rs` is left for the `status` lane.

## Who owns which files in P1-3

Each lane changes only its own engine modules (in `crates/scribe-core/src`) and its one command file (in `app/src-tauri/src/commands`).

| Lane | Engine modules | Command file | Commands |
|------|----------------|--------------|----------|
| `audio` | `audio.rs`, `dictation.rs`, `dictation_state.rs`, `dictation_worker.rs`, `incremental.rs`, `sounds.rs`, `file_transcribe.rs`, `self_test.rs`, `test_audio.rs` | `commands/audio.rs` | 9 |
| `output` | `output.rs`, `selection_transform.rs`, `text_replace.rs`, `platform/mod.rs`, `platform/windows.rs`, `platform/windows/`, `platform/macos.rs` | `commands/output.rs` | 6 |
| `models` | `model_manager.rs`, `whisper.rs`, `whisper_server.rs`, `parakeet.rs`, `speech_engine.rs`, `gpu.rs`, `session_model.rs` | `commands/models.rs` | 9 |
| `hotkeys` | `hotkeys.rs`, `remote_command.rs`, `platform/linux.rs`, `platform/linux/` | `commands/hotkeys.rs` | 7 |
| `data` | `db.rs`, `settings_store.rs`, `data_hygiene.rs`, `export.rs`, `import.rs`, `stats.rs`, `release_notes.rs`, `diagnostics.rs`, `replacements_file.rs` | `commands/data.rs` | 34 |
| `sync` | `note_sync.rs`, `github_backup.rs`, `github_oauth.rs`, `sync_history.rs`, `note_analysis.rs` | `commands/sync.rs` | 9 |
| `status` | `state_server.rs`, `status_file.rs` | `commands/status.rs` | 1 |

No lane edits these; anything a lane needs added to them goes on its list for P1-4:

- the shared seams: `lib.rs` (both crates), `core.rs`, `host.rs`, `events.rs`, `paths.rs`, `backend.rs`, `shell.rs`, `commands.rs` (the re-exports), `commands/mod.rs`, `commands/shell.rs`, `tauri_host.rs`;
- the shell: `tray.rs`, `window_size.rs`, `main.rs`, the Tauri configs, the installer hooks;
- the engine modules no lane owns: `app_state.rs`, `autostart.rs`, `child_guard.rs`, `error.rs`, `filler.rs`, `models.rs`, `process_info.rs`, `prompt_budget.rs`, `settings.rs`, `transcript.rs`, `update_check.rs`;
- every `Cargo.toml` and `Cargo.lock` (a lane needing a new dependency stops and lists it);
- another lane's files.

## Where two lanes meet

These are the places a careless change in one lane breaks another; keep the names, signatures and types below as they are.

- `core.rs` names a type from five lanes: `DictationWorker` (`audio`), `WarmTranscriber` (`models`), `HotkeyRuntimeState` (`hotkeys`), `BackendState` and `ReplacementStore` (`data`, through `backend.rs` and `replacements_file.rs`), `NoteSyncWorker` (`sync`), `DictationServer` (`status`).
  Each must keep its name and stay a cheap `Clone` handle.
- `events.rs` names the payload types, which live in lane files: `AudioLevelEvent`, `RecordingSessionInfo`, `RecordingResult`, `RecordingErrorEvent` (`audio.rs`); `DictationResult`, `DictationSaveFailedPayload`, `DictationEmptyPayload` (`dictation.rs`); `PartialTranscriptEvent` (`incremental.rs`); `OutputResult`, `OutputStartedPayload`, `OutputFailedPayload` (`output.rs`); `ModelDownloadProgress` (`model_manager.rs`); `EngineStatus` (`speech_engine.rs`); `HotkeyRegistrationFailure` and `RegistrationFailedEvent` (`hotkeys.rs`); `ReplacementsInfo` (`replacements_file.rs`); and the event-name constants in `settings_store.rs`, `whisper_server.rs`, `incremental.rs` and `replacements_file.rs`.
  Do not rename, move or change them.
- `dictation_state::emit_state_snapshot` (`audio`) calls `status_file::publish` and `DictationServer::publish` (`status`).
- `settings_store::apply_settings_change` (`data`) calls `hotkeys::set_transform_enabled`, `hotkeys::replace_hotkeys` and `hotkeys::emit_registration_failures` (`hotkeys`), `autostart::apply` (no lane) and `WarmTranscriber::sync_with_settings` (`models`).
- `dictation.rs` (`audio`) calls into `output`, `models` (`whisper`, `whisper_server`), `sync` (`note_sync`) and `data` (`db`); `output.rs` calls back into `dictation_state` (`audio`).
- `commands/models.rs` calls `data_hygiene::open_folder` (`data`); `commands/audio.rs` calls `output::OutputResult`; several command files call `settings_store::lock` and `emit_changed` (`data`).
- Pasting on Linux goes through the Wayland portal code in `platform/linux/` (`hotkeys`), which `output` calls through `platform/mod.rs`.
  The `output` lane must not change `platform/linux/`; the `hotkeys` lane must keep the functions `platform/mod.rs` calls.
- `hotkeys.rs` reaches `BackendState` and the dictation worker; `remote_command.rs` runs dictation commands.

The rule that keeps all of these working: a lane converts the inside of its functions and keeps every public name and signature, adding `_with` versions beside them.

## For P1-4

- Remove the `AppHandle` wrappers once no caller is left, and drop the `_with` suffix.
- Manage only `Core`; remove the individual `app.manage(...)` calls and the `Clone` handles that existed only for them.
- Remove the free functions in `shell.rs` and the `commands.rs` re-exports.
- Add to `Host` what the lanes listed (expected: opening a path or URL, a system notification, the app version, registering global shortcuts, the autostart entry).
- Drop the `tauri` dependency from `scribe-core`.
