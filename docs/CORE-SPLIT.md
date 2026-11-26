# Splitting the engine from the shell

Scribe is moving from a Tauri app to a native Slint app.
Phase 1 separates the engine (`crates/scribe-core`) from the interface inside today's shipping Tauri app, without changing what the app does.

- P1-1 moved the engine into `crates/scribe-core`.
- P1-2 (this step) added the seams: `Paths`, `CoreEvent`, `Host`, `Core`, and split the Tauri commands into one shell file per domain.
- P1-3 runs seven lanes in parallel, one per domain, each converting its engine modules from `AppHandle` to `&Core` and `Host`.
- P1-4 removes what the lanes leave behind (the `AppHandle` wrappers and parameters, the individually managed state, the `shell` free functions) and makes the engine free of Tauri.

Behaviour never changes along the way: same commands and names, same event names and payloads, same data folders, same installers.

## The seams

| File | What it is |
|------|------------|
| `crates/scribe-core/src/paths.rs` | `Paths`: every folder the engine uses (data, cache, log, resource, home), resolved exactly as Tauri resolves them, without Tauri. |
| `crates/scribe-core/src/events.rs` | `CoreEvent`: one variant per event the engine sends (22), carrying today's payload. `name()` is today's event name and the event serializes as its bare payload. |
| `crates/scribe-core/src/host.rs` | `trait Host`: what the engine needs from its shell (`emit`, `paths`, `flavor`, the tray and main-window calls). `Flavor` (Stable or Dev). `RecordingHost`, a host for tests. |
| `crates/scribe-core/src/core.rs` | `Core`: a cheap, cloneable handle owning the engine's parts (`backend()`, `warm_transcriber()`, `dictation_server()`, `note_sync()`, `dictation_worker()`, `hotkey_runtime()`, `replacements()`) and its `host()`, with `emit()`, `paths()` and `flavor()` shortcuts, and `shutdown()`. |
| `crates/scribe-core/src/backend.rs` | `BackendState` (was in `commands.rs`): the database, the state machine, the audio service and the shared locks. |
| `crates/scribe-core/src/shell.rs` | The installed `Host` (`shell::host()`), and the old tray and window free functions, which now forward to it. |
| `app/src-tauri/src/tauri_host.rs` | `TauriHost`: `Host` on Tauri. `emit` sends `app.emit(event.name(), &event)`. |
| `app/src-tauri/src/commands/<domain>.rs` | The `#[tauri::command]` functions, one file per domain. `commands/mod.rs` has `off_main`, `off_main_core` and `on_dialog_thread`. |

`Paths::data_dir()` is the framework's app data folder (Tauri's `app_data_dir()`: the database, clips, models, replacements.txt).
It is not the folder the user can choose in Settings (`dataDir`, resolved by `data_hygiene::effective_data_dir`), and the two must never be swapped for each other.

## Startup and shutdown

Setup runs in this order, and a test in `app/src-tauri/src/lib.rs` pins it:

1. resolve `Paths` and log any folder where they disagree with Tauri (every build; debug builds also assert);
2. install the `TauriHost` (`shell::install`);
3. open the database, manage `BackendState`, start the folder clean-up thread, stop stale speech processes;
4. manage the `WarmTranscriber`, the `DictationServer` (when it starts), the `NoteSyncWorker`, the `DictationWorker`, the replacements store, and an empty `HotkeyRuntimeState`;
5. build the `Core` from those same parts and manage it;
6. register the hotkeys (`hotkeys::setup`, which fills in the hotkey runtime), build the tray, create the windows.

Each part is a cheap handle onto shared state, so `app.state::<BackendState>()` and `core.backend()` are the same instance.

Engine work can also start from outside setup's own code: a queued dictation job, a hotkey press, a tray click, a window's command, or a remote command from a second launch (`Scribe toggle`).
All of these can start only once the `Core` is managed: the hotkeys, the tray and the windows come after it, and the single-instance handler and the global-shortcut handler both drop their event with a log line while `try_state::<Core>()` is `None`.
The only engine code that runs before the `Core` exists is setup's own steps 1 to 4 (and the threads they start: the folder clean-up, the speech engine watchdog, the note-sync and dictation workers, the replacements watcher, the dictation-state server).

At exit the shell calls `Core::shutdown()`, which writes an idle status file, stops the dictation-state server and stops the speech engine, in that order.
Nothing in the engine stops on drop: the handles are shared and Tauri never drops its managed state.
The native shell must call `Core::shutdown()` on its way out too.

Tests in `app/src-tauri/src/lib.rs` pin the startup order, the guards and the 79 command names; `app/src-tauri/tests/tauri_host.rs` checks `Paths` against Tauri's resolver for both flavours and the emitted payloads against the engine's own `app.emit` calls.

## The pattern for a lane

A lane converts the engine modules it owns, one function at a time.

1. Find what ties the function to Tauri: `app.state::<X>()`, `app.try_state::<X>()`, `app.emit(...)`, `app.path()`, `is_dev_flavor(app)`, `display_name(app)`, `app.config().identifier`, `app.package_info().version`, `crate::shell::...(app, ...)`.
2. Write the engine version, named after the old function with `_with` added.
   Replace `app.state::<BackendState>()` with `core.backend()`, the other parts with their `Core` accessors, `app.emit(name, payload)` with `core.emit(CoreEvent::Variant(payload))`, `app.path().app_cache_dir()` and friends with `core.paths().cache_dir()` and friends, `is_dev_flavor(app)` with `core.flavor().is_dev()`, `display_name(app)` with `core.flavor().display_name()`, `app.config().identifier` with `core.flavor().identifier()`, `app.package_info().version` with `env!("CARGO_PKG_VERSION")` (a test keeps the engine's version equal to the app's), and `crate::shell::open_dashboard(app, route)` with `core.host().open_dashboard(route)`.
3. Pick the signature by what the body still needs:
   - nothing from Tauri: `fn name_with(core: &Core, ...)`, or `fn name_with(host: &dyn Host, ...)` when it needs nothing but the host;
   - still an `AppHandle`, because it calls another lane's function that takes one, a function of a module no lane owns that takes one (`autostart`), or a Tauri plugin or window API: the transitional `fn name_with(app: &AppHandle, core: &Core, ...)`, with everything else converted to `core`.
     Every such function is listed by name in the "still takes AppHandle" table of the lane's pull request, for P1-4.
     The full list as of P1-2 is below, so no lane has to find it.
4. Keep the old function, with its old name and signature, as a thin wrapper that finds the `Core` and calls the engine version.
   A wrapper uses `app.try_state::<Core>()`, never `app.state::<Core>()` (which panics, and a panic on the dictation worker kills the one thread every dictation runs on).
   When the `Core` is missing, the wrapper logs a warning and returns what the function returns with no main window: the error `host::no_main_window()` for a function that returns a `Result<_, CommandError>`, `None` or nothing otherwise.
   Callers in other lanes keep compiling against the wrapper, untouched.
5. Switch the callers inside your own lane to the `_with` version.
6. Convert your commands in `app/src-tauri/src/commands/<domain>.rs` to take `core: tauri::State<'_, Core>` instead of `app: tauri::AppHandle` where they only needed the app for state.
   An `async` command runs its blocking work with `off_main_core(core.inner().clone(), |core| ...)`.
   A synchronous command stays synchronous: only its arguments change, never `fn` to `async fn`, because that moves it off the main thread.
   The synchronous commands today are `get_logs_dir`, `get_failed_recordings_dir`, `whats_new_pending`, `dismiss_whats_new` and `collect_diagnostics` (`data`), `github_device_cancel` (`sync`), and `open_dashboard` and `open_release_page` (`shell`, no lane).
   The windows pass no argument for `State`, so the command's interface does not change.
7. Test the engine version with `RecordingHost` and `Paths::from_parts` on a temporary folder: it records every event sent, so a test can assert the event name and payload.

Rules that keep behaviour the same and the lanes apart:

- A lane never adds anything to `Host`, `Core`, `CoreEvent` or `Paths`, and never converts or changes another lane's function: it calls the other lane's existing function (its wrapper) and lists the call in its "still takes AppHandle" table.
- Send the same event, with the `CoreEvent` variant whose `name()` is the name you replace; do not change a payload type (the `events.rs` tests pin every payload).
- Calls that need a Tauri plugin or a window (the opener, notifications, the global-shortcut plugin, the autostart plugin, webview windows) keep their `AppHandle`; P1-4 adds what the `Host` needs for them.

## Functions that still need an AppHandle

These are every function in the seven lanes whose body, once converted, still needs an `AppHandle`: it calls another lane's function with the app, calls `autostart` (no lane), or uses a Tauri plugin.
They take the transitional `(app: &AppHandle, core: &Core, ...)` signature and go in the lane's "still takes AppHandle" table.
The list came from a scan of every call made with the app across lane boundaries and every plugin use, outside tests.
Calls inside one lane, calls into `shell` (the `Host` covers them) and `is_dev_flavor`, `display_name`, the identifier and the version (`Core` covers them) are not on it.

### audio

| Function | Still needs the app for |
|----------|-------------------------|
| `dictation.rs` `transcribe_recording_inner` | `model_manager::session_model_path` (models), `output::handle_transcription_output`, `output::emit_output_failed` (output) |
| `dictation.rs` `finish_selection_transform` | `selection_transform::apply_result` (output) |
| `dictation_state.rs` `emit_state_snapshot` | `status_file::publish` (status) |
| `dictation_worker.rs` `execute` | `output::paste_last_transcript` (output) |
| `dictation_worker.rs` `start_transform` | `hotkeys::start_transform` (hotkeys) |
| `file_transcribe.rs` `transcribe_file` | `model_manager::selected_model_path`, `whisper::transcribe_with_output_prefix` (models) |
| `incremental.rs` `segment_context` | `model_manager::session_model_path` (models) |
| `self_test.rs` `ensure_selected_model` | `model_manager::download_model`, `model_manager::selected_model_path` (models) |

### output

| Function | Still needs the app for |
|----------|-------------------------|
| `output.rs` `notify` | the notification plugin |
| `output.rs` `PasteStateGuard::start` and its `Drop` | `dictation_state::emit_state_snapshot` (audio) |

### models

None: every call the models lane makes with the app stays inside the lane.

### hotkeys

| Function | Still needs the app for |
|----------|-------------------------|
| `hotkeys.rs` `register_hotkey_set`, `unregister_shortcuts`, `status`, `arm_note_chord`, `disarm_note_chord` | the global-shortcut plugin |
| `hotkeys.rs` `handle_press_edge`, `handle_release_edge`, `on_toggle_key_press`, `on_toggle_key_up`, `handle_note_chord` | `dictation_worker::submit` (audio) |
| `hotkeys.rs` `run_action` | `dictation_worker::submit`, `audio::emit_recording_error` (audio) |
| `hotkeys.rs` `start_transform` | `audio::start_recording_for_app` (audio) |

### data

| Function | Still needs the app for |
|----------|-------------------------|
| `settings_store.rs` `apply_settings_change` | `hotkeys::set_transform_enabled`, `hotkeys::replace_hotkeys`, `hotkeys::emit_registration_failures` (hotkeys), `autostart::apply` (no lane) |
| `settings_store.rs` `sync_launch_at_startup` | `autostart::follow_windows_switch` (no lane) |
| `data_hygiene.rs` `open_folder` | the opener plugin |
| `commands/data.rs` `open_transcript_externally`, `open_replacements_file` | the opener plugin |

### sync

| Function | Still needs the app for |
|----------|-------------------------|
| `commands/sync.rs` `github_device_start` | the opener plugin |
| `commands/sync.rs` `github_device_poll`, `github_disconnect` | `settings_store::emit_changed` (data) |

### status

None.

### Commands of other lanes that call `data`

`settings_store::emit_changed` and `data_hygiene::open_folder` (`data`) are called with the app from `commands/models.rs` (`delete_model`, `select_model`, `open_models_folder`) and `commands/hotkeys.rs` (`rebind_hotkey`, `set_hotkey_trigger`, `reset_hotkeys_to_defaults`, `load_production_hotkey_defaults`).
Those commands keep their `app` argument; they are listed in the `models` and `hotkeys` tables of their pull requests.

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
They need only the host, which is installed before anything else, so they use it rather than the `Core`:

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

Each lane changes only its own engine modules (in `crates/scribe-core/src`), its one command file (in `app/src-tauri/src/commands`) and its one new integration test file (`app/src-tauri/tests/<lane>.rs`, for tests that need a Tauri mock app; unit tests go in the lane's own modules).

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
- the shell: `tray.rs`, `window_size.rs`, `main.rs`, `build.rs`, the Tauri configs, the installer hooks;
- `app/src-tauri/tests/tauri_host.rs`, `tests/windows-test-manifest.xml` and another lane's test file;
- the engine modules no lane owns: `app_state.rs`, `autostart.rs`, `child_guard.rs`, `error.rs`, `filler.rs`, `models.rs`, `process_info.rs`, `prompt_budget.rs`, `settings.rs`, `transcript.rs`, `update_check.rs`;
- every `Cargo.toml` and `Cargo.lock` (a lane needing a new dependency stops and lists it);
- another lane's files.

A test that builds a Tauri mock app must live in `app/src-tauri/tests/`: `build.rs` embeds the Windows app manifest into integration tests only, and a mock app in a unit test fails to start on Windows (`STATUS_ENTRYPOINT_NOT_FOUND`).

## Where two lanes meet

These are the places a careless change in one lane breaks another; keep the names, signatures and types below as they are.

- `core.rs` names a type from five lanes: `DictationWorker` (`audio`), `WarmTranscriber` (`models`), `HotkeyRuntimeState` (`hotkeys`), `BackendState` and `ReplacementStore` (`data`, through `backend.rs` and `replacements_file.rs`), `NoteSyncWorker` (`sync`), `DictationServer` (`status`).
  Each must keep its name and stay a cheap `Clone` handle; `DictationServer::shutdown`, `WarmTranscriber::shutdown` and `status_file::publish_idle_with` must keep their signatures (`Core::shutdown` calls them).
- `lib.rs` calls `hotkeys::setup(app, &runtime, &hotkeys, transform_enabled)`; the `hotkeys` lane keeps that signature.
- `events.rs` names the payload types, which live in lane files: `AudioLevelEvent`, `RecordingSessionInfo`, `RecordingResult`, `RecordingErrorEvent` (`audio.rs`); `DictationResult`, `DictationSaveFailedPayload`, `DictationEmptyPayload` (`dictation.rs`); `PartialTranscriptEvent` (`incremental.rs`); `OutputResult`, `OutputStartedPayload`, `OutputFailedPayload` (`output.rs`); `ModelDownloadProgress` (`model_manager.rs`); `EngineStatus` (`speech_engine.rs`); `HotkeyRegistrationFailure` and `RegistrationFailedEvent` (`hotkeys.rs`); `ReplacementsInfo` (`replacements_file.rs`); and the event-name constants in `settings_store.rs`, `whisper_server.rs`, `incremental.rs` and `replacements_file.rs`.
  Do not rename, move or change them.
- Every call in the "still needs an AppHandle" tables above crosses two lanes: the caller keeps calling the callee's wrapper, and the callee keeps that wrapper's name and signature.
- Pasting on Linux goes through the Wayland portal code in `platform/linux/` (`hotkeys`), which `output` calls through `platform/mod.rs`.
  The `output` lane must not change `platform/linux/`; the `hotkeys` lane must keep the functions `platform/mod.rs` calls.

The rule that keeps all of these working: a lane converts the inside of its functions and keeps every public name and signature, adding `_with` versions beside them.

## Definition of done for a lane

A lane's pull request is ready when:

- it changes only the lane's files listed above;
- locally `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings` (with and without `--features e2e-audio`) and `cargo test --locked --workspace` pass, run through `devguard slot`;
- it reports the test counts of every test binary against main's, and no test stopped running;
- every check on GitHub is green (the three backends, the bundles, install-and-launch, Windows end-to-end, Release SBOM);
- its description has two tables: the functions converted (old name, `_with` version, what it now takes) and the functions that still take an `AppHandle` (name and why, from the lists above plus anything new it found).

## For P1-4

- Give the `Host` what the "still takes AppHandle" tables need (expected: opening a path or URL, a system notification, registering global shortcuts, the autostart entry), then remove the `app` parameter from every transitional `_with(app, core, ...)` function.
- Remove the `AppHandle` wrappers once no caller is left, and drop the `_with` suffix.
- Manage only `Core`; remove the individual `app.manage(...)` calls and the `Clone` handles that existed only for them.
- Remove the free functions in `shell.rs` and the `commands.rs` re-exports.
- When the engine's folders move onto `Paths`, Tauri's answer wins wherever the two disagree until the switch, and the startup log (which reports every disagreement) is checked on Windows, macOS and Linux installs first.
- Drop the `tauri` dependency from `scribe-core`.
