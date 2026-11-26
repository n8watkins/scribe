# Splitting the engine from the shell

Scribe is moving from a Tauri app to a native Slint app.
Phase 1 separates the engine (`crates/scribe-core`) from the interface inside today's shipping Tauri app, without changing what the app does.

- P1-1 moved the engine into `crates/scribe-core`.
- P1-2 added the seams: `Paths`, `CoreEvent`, `Host`, `Core`, and split the Tauri commands into one shell file per domain.
- P1-3 ran seven lanes in parallel, one per domain, each converting its engine modules from `AppHandle` to `&Core` and `Host`.
  All seven are merged: `status` (#145), `sync` (#146), `output` (#147), `models` (#148), `data` (#150), `hotkeys` (#151) and `audio` (#155), plus an engine fix found in review (#154) and an end-to-end test of the default Windows hotkeys (#153).
- P1-4 (next) removes what the lanes left behind (the `AppHandle` wrappers and parameters, the individually managed state, the `shell` free functions) and makes the engine free of Tauri.
  Its work list is the last section of this file, and it describes main as of 75614fc.

Behavior never changes along the way: same commands and names, same event names and payloads, same data folders, same installers.

The sections from "The pattern for a lane" to "Definition of done for a lane" are the P1-3 rulebook, corrected where the lanes and their reviews showed it was wrong or silent.
Where a rule changed, the text says what main does now.

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
It is not the folder the user can choose in Settings (`dataDir`, resolved by `data_hygiene::effective_data_dir_with`), and the two must never be swapped for each other.

### Which folder value to use

There are two values for the app's folders, and they are not interchangeable:

- `core.backend().dirs()` is `DataDirs` (`data_hygiene.rs`), built in `lib.rs` setup from Tauri's own `app_data_dir()` and `app_cache_dir()` and handed to `BackendState::new`.
  It names the folders the engine writes user data into and deletes from: clips, failed recordings, temp recordings, editor copies, and the file-transcription work folder.
  Dictation saves clips and quarantines failed recordings there (`dictation.rs`), and "Clear history", retention and the startup clean-up delete from the same value (`data_hygiene.rs`), so what is written and what is deleted can never point at different folders.
- `core.paths()` (or `host.paths()`) is `Paths`, which the engine resolves itself and the shell only checks against Tauri (the startup log line "The engine's folder differs from Tauri's").

New code that writes or deletes user data takes the folder from `core.backend().dirs()`, never from `core.paths()`.
The `Paths` uses on main that stay legitimate are reads and opens: the logs folder (`data_hygiene::logs_dir_with`, `diagnostics.rs`; `DataDirs` has no log folder), the home folder that diagnostics masks, and the resource folder the engine binaries are found in (`whisper::resolve_bundled_executable_with`).
Three merged writers also use `Paths`, and they are known exceptions rather than the model: the models folder (`model_manager::models_dir_with`, `<Paths::data_dir>/models`, where downloads land and deletes happen), the status file (`status_file::publish_with`, `Paths::cache_dir`), and the dictation-state discovery file (`state_server::control_file_path`, `~/.scribe` under `Paths::home_dir`).
They are correct only while `Paths` agrees with Tauri, which is why the startup mismatch log matters (P1-4 adds a check for it).

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
The only engine code that runs before the `Core` exists is setup's own steps 1 to 4 (and the threads they start: the folder clean-up, the speech engine watchdog and its `speech-engine-status` thread, the note-sync and dictation workers, the replacements watcher, the dictation-state server).
Three of those find what they need without the `Core`: the folder clean-up looks up the managed `BackendState`, `WarmTranscriber::new` takes the installed host and the managed `BackendState`, and the note-sync and dictation workers poll `try_state::<Core>()` until it is managed.

At exit the shell calls `Core::shutdown()`, which writes an idle status file, stops the dictation-state server and stops the speech engine, in that order.
Stopping the speech engine (`WarmTranscriber::shutdown`, #154) first sets a `shutting_down` flag, then stops every speech helper process it started (whisper-server, scribe-parakeet) by its recorded identity, without taking the engine lock, and tidies the engine slot only if the lock is free.
From then on nothing may start a load: `prewarm`, the background load, `run_unloaded`, `load` and the whisper-cli fallback all refuse (with the error `speech_engine_stopped` where they return one).
Any new code path that can load a model must check the same flag.
Nothing in the app may rely on drop to stop anything: the handles are shared and Tauri never drops its managed state.
(`WarmTranscriber` has a safety net that stops the engine when its last handle drops, for tests and for a shell that drops its `Core`; the app never reaches it.)
The native shell must call `Core::shutdown()` on its way out too.

Tests in `app/src-tauri/src/lib.rs` pin the startup order, the guards, the number of `app.manage(` calls (8) and the 79 command names; `app/src-tauri/tests/tauri_host.rs` checks `Paths` against Tauri's resolver for both flavors and the emitted payloads against the engine's own `app.emit` calls; `app/src-tauri/tests/models.rs` does the same for the models folder and the two models events.

## The pattern for a lane

A lane converts the engine modules it owns, one function at a time.

1. Find what ties the function to Tauri: `app.state::<X>()`, `app.try_state::<X>()`, `app.emit(...)`, `app.path()`, `is_dev_flavor(app)`, `display_name(app)`, `app.config().identifier`, `app.package_info().version`, `app.exit(...)`, `crate::shell::...(app, ...)`.
2. Write the engine version, named after the old function with `_with` added.
   Replace `app.state::<BackendState>()` with `core.backend()`, the other parts with their `Core` accessors, `app.emit(name, payload)` with `core.emit(CoreEvent::Variant(payload))`, `is_dev_flavor(app)` with `core.flavor().is_dev()`, `display_name(app)` with `core.flavor().display_name()`, `app.config().identifier` with `core.flavor().identifier()`, and `crate::shell::open_dashboard(app, route)` with `core.host().open_dashboard(route)`.
   A folder the function writes into or deletes from comes from `core.backend().dirs()` (`DataDirs`), not from `core.paths()`; only reads and opens of the logs, home and resource folders use `core.paths()` (see "Which folder value to use").
   Never replace `app.package_info().version` with the engine crate's `env!("CARGO_PKG_VERSION")`.
   Dev builds (`release.yml` gives each one a pre-release version such as `0.16.4-57`) and the installer-upgrade test build (`installer-upgrade.yml`, `--config` with a raised version) override the app's version, so the engine crate's version is wrong in exactly the builds that test upgrades.
   The test `the_engine_carries_the_app_version` in `app/src-tauri/src/lib.rs` compares only the base `tauri.conf.json` with the crate versions and does not cover those builds.
   Until the `Host` can answer the app's version (a P1-4 task), a function that needs it takes it as an argument from a caller that still has the app, as `diagnostics::collect_with`, `release_notes::pending_version_with` and `self_test::run` do.
3. Pick the signature by what the body still needs:
   - nothing from Tauri: `fn name_with(core: &Core, ...)`, or `fn name_with(host: &dyn Host, ...)` when it needs nothing but the host;
     a function that starts a thread which outlives the call takes an owned handle instead (`Arc<dyn Host>`, or a `Core` clone), as `state_server::start_with` does;
     a part the `Core` holds cannot be handed the `Core` it is part of, so a part that needs engine functions takes the host (and the other parts it reads) instead: `WarmTranscriber::new_with(host, backend, path)`, and the `models` functions it calls take `&dyn Host`;
   - still an `AppHandle`, because it calls another lane's function that takes one, a function of a module no lane owns that takes one (`autostart`), or a Tauri plugin or window API: the transitional `fn name_with(app: &AppHandle, core: &Core, ...)`, with everything else converted to `core`.
     A function that needs the app only to pass it on to such a function takes the transitional signature too.
     Every such function is listed by name in the "still takes AppHandle" table of the lane's pull request, for P1-4.
     The current list is in "Functions that still need an AppHandle" below.
4. Keep the old function, with its old name and signature, as a thin wrapper that finds the `Core` and calls the engine version.
   A wrapper uses `app.try_state::<Core>()`, never `app.state::<Core>()` (which panics, and a panic on the dictation worker kills the one thread every dictation runs on).
   A wrapper whose engine version needs nothing but the host uses `crate::shell::host()` instead of the `Core`, because the host is installed first thing in setup and some of these wrappers run before the `Core` exists (`status_file::publish`, `settings_store::emit_changed`, `output::emit_output_failed`, `audio::emit_recording_error`, `incremental::emit_partial_transcript`, `hotkeys::emit_registration_failures`, `data_hygiene::logs_dir`, `test_audio::activate`, `state_server::start`).
   When the `Core` (or the host) is missing, the wrapper logs a warning and returns what the function returns with no main window: the error `host::no_main_window()` for a function that returns a `Result<_, CommandError>`, `None` or nothing otherwise.
   A function that returns a plain struct with an error field (for example `SyncAttempt`) returns that struct carrying `host::no_main_window()` in the field, with everything else empty.
   A function that returns a plain struct with no error field returns its "nothing happened" value (`gpu::probe` returns a not-probed `GpuProbe`).
   A `pub(crate)` function whose callers are all in your lane is replaced by its engine version, not wrapped (`audio::finish_timed_out_recording`, `audio::finish_disconnected_recording`, `dictation::emit_dictation_empty`, `whisper::resolve_bundled_executable`): a wrapper nobody calls is dead code.
   A thread that starts before the `Core` is managed and looks it up later (the note-sync worker, the dictation worker) waits for it: it polls `try_state::<Core>()` every 50 ms and stops only when its channel closes.
   It never drops the work it was woken for.
   Callers in other lanes keep compiling against the wrapper, untouched.
5. Switch the callers inside your own lane to the `_with` version.
   A call into another lane may go straight to that lane's `_with` version once it is merged and takes no `AppHandle` (the `audio` lane calls `output::emit_output_failed_with`, `status_file::publish_with` and the `models` lane's `_with` functions; the `data` lane calls `hotkeys::set_transform_enabled_with`).
   This leaves the other lane's wrapper with fewer callers, often none; list every wrapper you leave without a caller in your pull request, because P1-4 deletes them (the current list is under "Wrappers to delete" in the P1-4 section).
6. Convert your commands in `app/src-tauri/src/commands/<domain>.rs` to take `core: tauri::State<'_, Core>` instead of `app: tauri::AppHandle` where they only needed the app for state.
   An `async` command runs its blocking work with `off_main_core(core.inner().clone(), |core| ...)`.
   A command that took the app only to run off the main thread (`off_main(app, ...)`) and reads no state takes the `Core` only for `off_main_core` and ignores it (`get_paste_permission`, `request_paste_permission`, `configure_portal_shortcuts`); a shell command that uses neither stays as it is (`check_for_update`).
   A synchronous command stays synchronous: only its arguments change, never `fn` to `async fn`, because that moves it off the main thread.
   The synchronous commands today are `get_logs_dir`, `get_failed_recordings_dir`, `whats_new_pending`, `dismiss_whats_new` and `collect_diagnostics` (`data`), `github_device_cancel` (`sync`), and `open_dashboard` and `open_release_page` (`shell`, no lane).
   The windows pass no argument for `State`, so the command's interface does not change.
7. Test the engine version.
   A function that takes `&dyn Host` is tested with `RecordingHost` and `Paths::from_parts` on a temporary folder: it records every event sent, so a test can assert the event name and payload.
   A function that takes `&Core` cannot be tested directly yet: no test builds a `Core`.
   `WarmTranscriber::new_with` and the test-only `DictationWorker::spawn_with` need no app, so the one part still tied to the Tauri `AppHandle` is `NoteSyncWorker::spawn` (a mock app's handle has a different type).
   So keep the `_with(core: &Core)` function a thin shell, move its body into a private function over the parts it really uses (`&BackendState`, `&dyn Host`, a store, plain values), and unit-test that function.
   The models are the sync lane's `collect_notes`, `collect_transcripts` and `record_attempt` in `note_sync.rs`, the data lane's `apply_settings_change_in` with a recording `SettingsEffects`, and the audio lane's `dictation_state::Parts` with its `test_parts::Fixture`.
   The integration test file `app/src-tauri/tests/<lane>.rs` is optional: add it only when it tests something a unit test cannot.
   A way to build a `Core` in tests is on the P1-4 list.

   The Windows end-to-end job is not proof of the event contract.
   It drives real dictations, the default hotkeys and their watchers (#153) and the paste, and it fails when a dictation does not reach its target, but it does not check event names, payloads, their order, how many times an event is sent, or the error paths.
   For example, after the `output` and `audio` lanes a failed paste after a dictation sent `scribe:output-failed` twice (once in `output.rs`, once more in `dictation::transcribe_recording_inner`), and every check passed; #156 fixed it (`dictation::settle_output`).
   So the unit tests with `RecordingHost` must pin, for every function that sends events:
   - the event name and the exact JSON payload (`CoreEvent::name()` and the serialized event, compared with what the old `app.emit(name, payload)` sent);
   - the order of the events relative to each other and to the other side effects (for a new state: the `scribe:app-state` event, then the status file, then the dictation-state server; for the speech engine: the event, then the tray line; for settings: one `scribe:settings-changed`, after the save);
   - how many times each event is sent, including "exactly once" on failure paths and "none" where nothing may be sent;
   - the error paths: what the function returns and sends when the `Core` or the host is missing, when the database fails, and when a step in the middle fails (a rollback, a quarantined recording).

Rules that keep behavior the same and the lanes apart:

- A lane never adds anything to `Host`, `Core`, `CoreEvent` or `Paths`, and never converts or changes another lane's function.
  It calls the other lane's function: the merged `_with` version when that takes no `AppHandle`, otherwise the wrapper, and lists the call in its "still takes AppHandle" table.
- Send the same event, with the `CoreEvent` variant whose `name()` is the name you replace; do not change a payload type (the `events.rs` tests pin every payload).
- Calls that need a Tauri plugin or a window (the opener, notifications, the global-shortcut plugin, the autostart plugin, webview windows, exiting the app) keep their `AppHandle`; P1-4 adds what the `Host` needs for them.
- Never call the host (an event, the tray, a window, a sound) while holding an engine lock (the speech engine's lock, the dictation state machine's lock, the database lock).
  On Tauri the tray calls wait for the main thread, and at exit the main thread waits for the engine lock in `Core::shutdown`: that is the quit deadlock #148 found and fixed by sending the speech engine's status to the host from its own `speech-engine-status` thread, in order.
  Read what you need, drop the lock, then call the host (as `dictation_worker::refresh_tray` does).
- The one lock held across host calls on purpose is the settings write lock (`BackendState::settings_writes`, taken through `settings_store::lock`): it is held from reading the settings to announcing them, so the events go out in the order the writes happened, and the announcement (`emit_changed_with`) updates the tray menu, and a settings change registers hotkeys through the global-shortcut plugin while holding it.
  So that lock must never be taken on the main thread; every caller today runs off it (the async commands through `off_main` or `off_main_core`, the tray and window-focus handlers on their own `launch-at-startup` threads).
  P1-4 adds a debug assertion for this.

## Functions that still need an AppHandle

This is every function in `crates/scribe-core/src` and `app/src-tauri/src/commands` that still takes an `AppHandle` on main (75614fc), apart from the old-signature wrappers, which are listed in the P1-4 section.
It was rebuilt from a scan of every function signature that names `AppHandle`, outside tests, and reconciled with the "still takes AppHandle" tables of #145, #146, #147, #148, #150, #151 and #155.

The P1-2 version of this list undercounted.
It named only the functions that call across a lane boundary or use a plugin themselves, and missed every function that takes the app only to pass it on to one of those (the hotkeys lane alone found 11 of them), the `autostart` module, `self_test` (exiting the app, the app's version), the version-reading commands, the dialog commands (`on_dialog_thread` takes the app), and the setup-time constructors that find their parts through the app.
Some of its entries also went away: `dictation_state::emit_state_snapshot` and the models calls of `audio` no longer need the app.

Each row says why the app is still there:

- **Tauri API**: the body itself uses a plugin, a window or an app call; P1-4 gives the `Host` a method for it.
- **Pass-through**: the body only passes the app on to another function in this list.
- **Before the Core**: built during setup before the `Core` exists, so it finds what it needs through the app.

### audio

| Function | Why |
|----------|-----|
| `dictation.rs` `transcribe_recording_with`, `transcribe_recording_checked`, `transcribe_recording_inner` | Pass-through to `output::handle_transcription_output_with` (output) |
| `dictation.rs` `finish_selection_transform` | Pass-through to `selection_transform::apply_result_with` (output) |
| `dictation_worker.rs` `AppExecutor` (holds `app`), `execute_with` | Pass-through to `output::paste_last_transcript_with` (output), `stop_and_transcribe`, `start_transform`, `audio::finish_*_recording_with` |
| `dictation_worker.rs` `stop_and_transcribe` | Pass-through to `dictation::transcribe_recording_with` |
| `dictation_worker.rs` `start_transform` | Pass-through to the wrapper `hotkeys::start_transform` (hotkeys) |
| `dictation_worker.rs` `DictationWorker::spawn` | Before the Core: its thread polls `app.try_state::<Core>()` for the `Core` |
| `audio.rs` `finish_timed_out_recording_with`, `finish_disconnected_recording_with` | Pass-through to `dictation::transcribe_recording_with` |
| `file_transcribe.rs` `transcribe_file_with` | Pass-through to the wrapper `whisper::transcribe_with_output_prefix` (models), which has a host-only `_with` version it could call today |
| `self_test.rs` `spawn_with` | Tauri API: `app.exit` and `app.package_info().version` |
| `self_test.rs` `run` | Pass-through to `file_transcribe::transcribe_file_with` |
| `commands/audio.rs` `transcribe_file` | Pass-through to `file_transcribe::transcribe_file_with` |

`dictation_worker::request_with` takes no app but runs on `tauri::async_runtime::spawn_blocking`, which ties the engine to Tauri too.

### output

| Function | Why |
|----------|-----|
| `output.rs` `notify` | Tauri API: the notification plugin |
| `output.rs` `emit_output_completed` | Pass-through to `notify` |
| `output.rs` `paste_transcript_text`, `copy_and_paste_transcript_text` | Pass-through to `emit_output_completed`, and the `PasteStateGuard` closures call the wrapper `dictation_state::emit_state_snapshot(app, ..)` (audio), whose `_with(core)` version they could call today |
| `output.rs` `handle_transcription_output_with`, `paste_text_with`, `paste_last_transcript_with`, `paste_transcript_with` | Pass-through to the paste paths above |
| `output.rs` `core_for` | Wrapper helper: finds the `Core` through the app |
| `selection_transform.rs` `apply_result_with`, `transform_selection_blocking_with` | Pass-through to `output::paste_text_with` |
| `commands/output.rs` `paste_transcript`, `transform_selection` | Pass-through to the `_with` functions above |

### models

| Function | Why |
|----------|-----|
| `whisper_server.rs` `WarmTranscriber::new` | Before the Core: finds the managed `BackendState` through the app (`new_with(host, backend, path)` is the engine version) |
| `commands/models.rs` `open_models_folder` | Pass-through to `data_hygiene::open_folder` (data: the opener plugin) |
| `commands/models.rs` `delete_model`, `select_model` | Pass-through to the wrapper `settings_store::emit_changed` (data), whose host-only `_with` version they could call today |

### hotkeys

| Function | Why |
|----------|-----|
| `hotkeys.rs` `register_hotkey_set_with`, `unregister_hotkey_set`, `unregister_shortcuts`, `status_with`, `arm_note_chord_with`, `disarm_note_chord_with` | Tauri API: the global-shortcut plugin |
| `hotkeys.rs` `handle_press_edge_with`, `handle_release_edge_with`, `on_toggle_key_press_with`, `on_toggle_key_up_with`, `handle_note_chord_with` | Pass-through to the wrapper `dictation_worker::submit` (audio), whose `submit_with(core)` they could call today |
| `hotkeys.rs` `run_action_with` | Pass-through to the wrappers `dictation_worker::submit` and `audio::emit_recording_error` (audio), both with core-only or host-only versions |
| `hotkeys.rs` `start_transform_with` | Pass-through to the wrapper `audio::start_recording_for_app` (audio), whose `start_recording_with(core)` it could call today |
| `hotkeys.rs` `setup_with`, `replace_hotkeys_with` | Pass-through to `register_hotkey_set_with` and `unregister_hotkey_set` (plugin) |
| `hotkeys.rs` `handle_shortcut_with`, `on_toggle_key_down_with`, `configure_toggle_watcher_with`, `run_toggle_watcher`, `configure_chord_watcher_with`, `dispatch_chord_pressed_with`, `dispatch_chord_released_with`, `windows_chord::run_watcher`, `windows_chord::step`, `wayland::start_portal`, `wayland::on_portal_event` | Pass-through to the functions above; the two watcher threads own an `AppHandle` and a `Core` |
| `remote_command.rs` `RemoteCommand::run_with` | Pass-through to the edge functions above |
| `commands/hotkeys.rs` `get_hotkey_status`, `bind_portal_shortcuts` | Pass-through to `status_with` (plugin) |
| `commands/hotkeys.rs` `rebind_hotkey`, `set_hotkey_trigger`, `reset_hotkeys_to_defaults`, `load_production_hotkey_defaults` | Pass-through to `replace_hotkeys_with` and `status_with` (plugin), and to the wrapper `settings_store::emit_changed` (data) |

### data

| Function | Why |
|----------|-----|
| `settings_store.rs` `apply_settings_change_with` and its `AppEffects` (holds `app`) | Pass-through to `hotkeys::replace_hotkeys_with` (hotkeys: plugin) and `autostart::apply` (no lane) |
| `settings_store.rs` `patch_settings_now_with`, `set_launch_at_startup_with` | Pass-through to `apply_settings_change_with` |
| `settings_store.rs` `sync_launch_at_startup_with` | Pass-through to `autostart::follow_windows_switch` (no lane) |
| `settings_store.rs` `core_of` | Wrapper helper: finds the `Core` through the app |
| `data_hygiene.rs` `open_folder` | Wrapper of `open_folder_with(host, dir)` since P1-4 step 1 |
| `data_hygiene.rs` `run_startup_maintenance` | Before the Core: finds the managed `BackendState` through the app (`run_startup_maintenance_with(&BackendState, ..)` is the engine version) |
| `commands/data.rs` `open_transcript_externally`, `open_replacements_file` | Tauri API: the opener plugin |
| `commands/data.rs` `open_data_folder`, `open_logs_folder`, `open_failed_recordings_folder` | Pass-through to `data_hygiene::open_folder` |
| `commands/data.rs` `patch_settings` | Pass-through to `patch_settings_now_with` |
| `commands/data.rs` `whats_new_pending`, `collect_diagnostics` | Tauri API: `app.package_info().version` |

### sync

| Function | Why |
|----------|-----|
| `note_sync.rs` `NoteSyncWorker::spawn` | Before the Core: its thread polls `app.try_state::<Core>()` before each sync |
| `commands/sync.rs` `github_device_start` | Tauri API: the opener plugin |
| `commands/sync.rs` `github_device_poll`, `github_disconnect` | Pass-through to the wrapper `settings_store::emit_changed` (data), whose host-only `_with` version they could call today |

### status

None apart from the wrappers (`state_server::start`, `status_file::publish`, `status_file::publish_idle`).

### No lane

| Function | Why |
|----------|-----|
| `autostart.rs` `apply`, `reconcile_at_launch`, `follow_windows_switch` | Since P1-4 step 1, wrappers of their host-only `_with` versions (through `shell::host()`), kept for their callers: `settings_store.rs` (data) and `lib.rs` setup |
| `lib.rs` (scribe-core) `is_dev_flavor`, `display_name` | Tauri API: `app.config().identifier`; `Flavor` covers both |
| `shell.rs` `update_tray_status`, `update_engine_summary`, `sync_settings`, `open_dashboard`, `toggle_dashboard`, `main_window_logical_size` | The old free functions; they ignore the app and forward to the installed host |
| `commands/mod.rs` `off_main` | Runs a closure that receives the app (`on_dialog_thread` takes the `Core` since P1-4 step 1) |
| `commands/shell.rs` `open_dashboard`, `save_window_size`, `open_release_page` | The shell's own commands (the opener plugin, the window); they stay in the shell |

The Tauri shell's own files (`app/src-tauri/src/lib.rs`, `tray.rs`, `window_size.rs`, `tauri_host.rs`) keep using the `AppHandle`: they are the shell.

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

`state_server.rs` was left for the `status` lane, which converted it in #145 (`start_with` takes an `Arc<dyn Host>`; `start` is the host-only wrapper setup still calls).
Since the `audio` lane (#155), `dictation_state` calls `status_file::publish_with` directly, so the `publish` wrapper above has no caller left; `publish_idle` is still called by the shell's setup and by its exit path for a setup that failed before the `Core` existed.

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
- Every cross-lane call in the "still needs an AppHandle" tables above goes through the callee's wrapper or its transitional `_with(app, core, ..)` version, and the callee keeps that name and signature until P1-4 changes caller and callee together.
- Pasting on Linux goes through the Wayland portal code in `platform/linux/` (`hotkeys`), which `output` calls through `platform/mod.rs`.
  The `output` lane must not change `platform/linux/`; the `hotkeys` lane must keep the functions `platform/mod.rs` calls.

The rule that keeps all of these working: a lane converts the inside of its functions and keeps every public name and signature, adding `_with` versions beside them.

## Definition of done for a lane

A lane's pull request is ready when:

- it changes only the lane's files listed above;
- locally `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings` (with and without `--features e2e-audio`) and `cargo test --locked --workspace` pass, run through `devguard slot`;
- it reports the test counts of every test binary against main's, and no test stopped running;
- every check on GitHub is green (the three backends, the bundles, install-and-launch, Windows end-to-end, Release SBOM);
- its unit tests pin what the "Test the engine version" step lists (event names, payloads, order, counts and error paths) for every function that sends events, since the end-to-end job does not;
- its description has two tables: the functions converted (old name, `_with` version, what it now takes) and the functions that still take an `AppHandle` (name and why, from the lists above plus anything new it found), and it names every wrapper of another lane that it left without a caller.

## For P1-4

P1-4 removes the `AppHandle` wrappers and parameters, manages only `Core`, and drops the `tauri` dependency from `crates/scribe-core`.
It is done when no function in `crates/scribe-core/src` takes an `AppHandle`, `lib.rs` setup calls `app.manage(` exactly once (for the `Core`), `crates/scribe-core/Cargo.toml` names no `tauri` crate, behavior is unchanged (same commands and arguments, same events and payloads, same folders, same installers), and every check on GitHub is green.
Everything in this section was checked against main at 75614fc; file:line references are to that commit.
Since then #156 (main 8587a69) changed only `crates/scribe-core/src/dictation.rs`: it removed the second `scribe:output-failed` after a failed paste (new helper `settle_output`), and it did not change `dictation_state::app_version()`, which is still P1-4 work below.
If main has moved further when you start, check the references you rely on before you change code.

Heavy commands (builds, clippy, tests) run through `devguard slot -- <command>`, which waits for a free slot of the machine's shared build and test budget before running the command.

### What step 1 built

Step 1 (`seams`) added these, and step 2 lanes build on these names:

- `Host` (`host.rs`) gained `app_version()`, `exit(code)`, `open_path(&Path)`, `open_url(&str)`, `notify(title, body)`, `register_shortcut(HotKey)`, `unregister_shortcut(HotKey)`, `is_shortcut_registered(HotKey)`, `launch_at_login()` and `set_launch_at_login(bool)`.
  The fallible ones return `Result<_, String>`, the plugin's error text, so the engine keeps building today's `CommandError` codes and messages around it.
  `HotKey` is `global_hotkey::hotkey::HotKey`, the type the plugin calls `Shortcut`.
- `TauriHost` (`tauri_host.rs`) answers them with the same plugin calls the engine makes today; `tauri_host::app_version(app)` is its version, and `tauri_host::autostart_plugin()` is the plugin the shell registers (`lib.rs:358`).
- `RecordingHost` records every call that acts (`calls()`, a list of `HostCall`), answers `RECORDING_HOST_VERSION` (or `with_app_version(..)`), holds shortcuts and the startup entry the way the plugins do (a second registration of a held shortcut fails), and `fail(Failing::..)` makes one kind of call fail with `Failing::message()`.
  Queries (`app_version`, `is_shortcut_registered`, `launch_at_login`) are not recorded.
- `CoreCell` (`core.rs`): `CoreCell::new()`, `cell.fill(&core)` right after `Core::new` (whose signature did not change), and `cell.get()`.
- `data_hygiene::open_folder_with(host, dir)`; `open_folder` is its host-only wrapper.
- `autostart::apply_with`, `reconcile_at_launch_with` and `follow_windows_switch_with` take `&dyn Host`; `apply`, `reconcile_at_launch` and `follow_windows_switch` are their host-only wrappers.
  The Windows registry entries are named after `Flavor::product_name()`.
- `commands/mod.rs` `on_dialog_thread(core, |core| ..)`; `pick_data_dir`, `export_transcripts` and `preview_transcript_import` no longer take the app.
- Every `impl Host` must implement the new methods: besides the two hosts above, the test hosts `MainThreadHost` and `PanicsOnceHost` in `whisper_server.rs` do.

### What the Host must gain

Add these to `trait Host` (`crates/scribe-core/src/host.rs`), to `TauriHost` (`app/src-tauri/src/tauri_host.rs`, using the same plugin calls the engine makes today) and to `RecordingHost` (which records each call, so tests can assert it):

- **The app's version** (`app.package_info().version` in `TauriHost`), for `self_test::spawn_with` (`self_test.rs:77`), the `whats_new_pending` and `collect_diagnostics` commands (`commands/data.rs:670`, `:690`), and `dictation_state::app_version()` (`dictation_state.rs:37`, today the engine crate's `CARGO_PKG_VERSION`, as it was before the split; `docs/TODO.md` lists the Dev build's wrong `appVersion`).
  `app_version()` feeds three `appVersion` fields: the stream snapshot (`DictationSnapshot::from_state`, `dictation_state.rs:95`), the status file (`status_file.rs:64`) and the dictation-state discovery file (`state_server.rs:483`).
  Switching them to the host's answer is an intended fix and a visible change: in Dev and installer-upgrade-test builds, `appVersion` in `status.json`, the event stream and `control.json` changes from the base version to the real one; say so in the pull request.
  Never answer with `env!("CARGO_PKG_VERSION")`: Dev builds and the installer-upgrade build override the app's version.
  Both the version and exiting the app must be callable from any thread, not only the main thread: the self-test exits from its own thread (`self_test.rs:87`).
  `update_check.rs` (no lane) also compares the engine crate's version with the release manifest, and the HTTP user agents use it; leave them alone in P1-4 and list them in the pull request as a known difference in Dev builds.
- **Exiting the app with a code**, for the self-test (`self_test.rs:65`, `:87`, `:91`).
- **Opening a path and a URL**, for `data_hygiene::open_folder` (`data_hygiene.rs:467`), `open_transcript_externally` and `open_replacements_file` (`commands/data.rs:252`, `:651`) and `github_device_start` (`commands/sync.rs:122`).
  The shell's own `open_release_page` (`commands/shell.rs:47`) may keep the plugin.
- **A system notification**, for `output::notify` (`output.rs:785`).
- **Global shortcuts**: register, unregister and "is it registered", for `hotkeys.rs` (`register_hotkey_set_with`, `unregister_hotkey_set`, `unregister_shortcuts`, `status_with`, `arm_note_chord_with`, `disarm_note_chord_with`).
  The engine names the plugin's `Shortcut`, `Code`, `ShortcutEvent` and `ShortcutState`, which the plugin re-exports from the `global-hotkey` crate (already in `Cargo.lock`), so the engine can name those types from `global-hotkey` and keep its runtime state without Tauri.
  The plugin's handler in `lib.rs` (`:346`) stays in the shell and calls the engine.
- **The startup entry**: what `autostart::apply`, `reconcile_at_launch` and `follow_windows_switch` do (`autostart.rs:168`, `:204`/`:241`, `:263`).
  They use the autostart plugin and `app.package_info().name` for the Windows registry entries (`autostart.rs:343`, `:371`, `:379`); the product name is `Flavor::product_name()`.
  `autostart::plugin()` (`autostart.rs:44`) moves to the shell; the app crate does not depend on `tauri-plugin-autostart` today (only `crates/scribe-core/Cargo.toml:25` does), so step 1 adds it there.

Not a `Host` method: `on_dialog_thread` (`commands/mod.rs:66`) uses the app only to get off the main thread (the dialogs are `rfd`), so it takes the `Core` (or nothing), like `off_main_core`.

### How the workers receive the Core

`DictationWorker` and `NoteSyncWorker` are built before the `Core` and look it up through the app today (`dictation_worker.rs:243`-`:250`, `note_sync.rs:362`-`:377`).
Both move to one mechanism, defined once in `core.rs` in step 1, so no lane invents its own:

- `CoreCell`: a cheap `Clone` handle onto `Arc<OnceLock<Weak<Inner>>>`, where `Inner` is the `Core`'s private inner value.
- `CoreCell::new()` makes an empty cell; `Core::new(host, parts, &cell)` (or `cell.fill(&core)` right after `Core::new`) stores a weak reference to the new `Core`; filling twice is a no-op with a warning.
- `CoreCell::get() -> Option<Core>` upgrades the weak reference: `None` before the `Core` exists and again once every `Core` handle has been dropped.
- A worker takes a `CoreCell` clone instead of the `AppHandle`, calls `get()` once per job (the dictation worker) or once per sync (the note-sync worker), and drops the `Core` it got when that job ends; it never stores a strong `Core`.
  While `get()` answers `None` before the first fill, the worker keeps today's behavior: it polls every 50 ms, queues jobs in arrival order, and drops none.
- Because the workers hold only weak references, dropping the last `Core` drops the worker senders it holds, the channels close, and both worker threads end; that breaks the cycle described under "Hazards".

Step 1 adds `CoreCell` with unit tests (empty, filled, dropped); the `audio` and `sync` lanes give their worker a constructor that takes a `CoreCell` in step 2, leaving `spawn(app)` unchanged because the shell still calls it; step 3 builds the cell in `lib.rs`, passes it to both workers, and fills it after `Core::new`.

### Hazards

- **The dictation worker finds the `Core` by polling.**
  `DictationWorker::spawn(app)` runs in `lib.rs` setup (`:508`) before the `Core` exists, and its thread polls `app.try_state::<Core>()` every 50 ms when the first job arrives, queueing jobs in order meanwhile (`dictation_worker.rs:243`, `:338`-`:395`).
  With only `Core` managed and no `AppHandle` in the engine, it receives the `Core` through the `CoreCell` above, and it must keep both promises its tests pin: no job is dropped, and jobs run in arrival order.
  `NoteSyncWorker::spawn(app)` (`note_sync.rs:362`-`:374`) has the same problem: it looks the `Core` up before each sync, and it uses the same `CoreCell`.
- **The `Core` and its workers keep each other alive.**
  The `Core` holds the dictation worker's sender; the worker thread's `AppExecutor` holds a `Core` clone once it has found one (`dictation_worker.rs:486`); and the thread ends only when every sender is gone (`:342`).
  The note-sync worker is the same shape: the `Core` holds its sender, and its thread ends only when every sender is gone (`note_sync.rs:413`); today it drops the `Core` after each sync, and it must keep doing so (the `CoreCell` rule: no stored strong `Core`).
  Other long-lived owners: the sound player thread holds a `BackendState` clone for the life of the process (its handle sits in a process-wide static, `sounds.rs:325`-`:346`); the Windows toggle-key and chord watcher threads own a `Core` clone until their stop flag is set (`hotkeys.rs:665`, `:1550`); the recording timeout thread and the audio stream's error callback hold `DictationWorker` clones.
  Tauri never drops its state, so none of this matters in the Tauri shell.
  A native shell that expects dropping the `Core` to stop these threads needs the `CoreCell` rule for the workers, and `Core::shutdown` setting the watchers' stop flags, and it must still call `Core::shutdown()`.
- **The startup clean-up would be skipped silently.**
  `lib.rs` (`:460`-`:473`) starts the `data-cleanup` thread with `data_hygiene::run_startup_maintenance(&handle, ..)`, which looks up the individually managed `BackendState` (`data_hygiene.rs:267`-`:276`).
  Once only `Core` is managed, that lookup finds nothing and the clean-up of old temp recordings, editor copies, failed recordings past retention and orphaned clips does not run, with only a warning in the log.
  Pass the thread a clone of the backend and call `run_startup_maintenance_with`, and make the startup-order test pin the clean-up thread after the backend is created (#150 describes this).
- **The replacements watcher bypasses the host.**
  The watcher set up in `lib.rs` (`:513`-`:531`) sends `scribe:replacements-changed` with `watch_app.emit(..)` and finds the note-sync worker with `try_state::<NoteSyncWorker>()`.
  Send `CoreEvent::ReplacementsChanged` through the host and give the closure the `NoteSyncWorker` clone (created earlier, `:504`); the state lookup stops working once only `Core` is managed.
- **`emit_state_snapshot` cannot become host-only.**
  `dictation_state::emit_state_snapshot_with` needs the dictation-state server, which only the `Core` holds (`dictation_state.rs:127`, `:187`-`:197`), so its callers keep a `Core` (or `dictation_state::Parts`).
  `output.rs` still calls the `AppHandle` wrapper from its paste guards (`:375`, `:498`).
- **Other state lookups in the shell.**
  The exit path (`lib.rs:754`-`:774`) falls back to `try_state::<DictationServer>()` and `try_state::<WarmTranscriber>()` when setup failed before the `Core` existed; the window-focus and close handlers (`lib.rs:627`, `:644`) and the tray (`tray.rs:165`, `:180`) look up `BackendState`.
  Each must move to the `Core`, and the failed-setup exit path must still stop whatever parts did start.
- **Runtime failures the compiler does not catch once only `Core` is managed.**
  Each of these compiles and then fails at run time:
  - `app.state::<whisper_server::WarmTranscriber>()` at the end of setup (`lib.rs:606`) panics on every launch;
  - `save_window_size` reads `app.state::<BackendState>()` (`commands/shell.rs:21`) and panics when the user saves the window size;
  - `missing_command_state` (`lib.rs:222`-`:250`) lists `BackendState`, `WarmTranscriber`, `ReplacementStore`, `DictationWorker` and `NoteSyncWorker` as missing, and `create_config_windows` (`lib.rs:262`-`:269`) then returns an error instead of creating the windows, so setup fails;
  - `WarmTranscriber::new(app, ..)` (`whisper_server.rs:388`-`:403`) looks up the managed `BackendState`; without it a release build logs a warning and builds a speech engine with no backend (only debug builds assert), so step 3 must call `WarmTranscriber::new_with(host, backend, path)` instead.
  So step 3 must also be proven by launching the app: the install-and-launch jobs and the Windows end-to-end job must pass on its pull request, and `grep -rn "state::<" app/src-tauri/src` must show no lookup of a part that is no longer managed.
- **Windows-only code.**
  Large parts of the code this work changes compile only on Windows: the toggle-key watcher and its key handlers (`hotkeys.rs:664`-`:736`), the note chord (`:1316`-`:1367`), the chord watcher (`:1491`-`:1700`), the startup-entry registry code (`autostart.rs:204`, `:339`, and the rest of its `#[cfg(windows)]` items) and the title-bar styling in setup (`lib.rs:581`).
  Local clippy and tests on Linux or WSL never compile them, so only the Windows CI jobs (and macOS CI for the macOS code) prove them.
  See "Checking Windows code locally" below for what works on this machine.
- **Locks.**
  Keep the lock rules of the rulebook: never call the host while holding an engine lock, and never take the settings write lock on the main thread.
  `Core::shutdown()` must stay free of the engine lock, and no new code path may start a model load once `shutting_down` is set (#154).
- **Tauri outside `AppHandle`.**
  `dictation_worker::request_with` uses `tauri::async_runtime::spawn_blocking` (`dictation_worker.rs:478`), `output::core_for` returns a `tauri::State` (`output.rs:157`), and `hotkeys.rs` imports the plugin's types (`:13`-`:14`).
- **The version test.**
  `the_engine_carries_the_app_version` (`app/src-tauri/src/lib.rs:829`) explains the engine's own `CARGO_PKG_VERSION` uses; update its comment when the engine stops reporting that version as the app's.

### Wrappers to delete

These `AppHandle` wrappers have no caller outside tests on main; delete them (and the helpers only they use, such as `model_manager::host_of`):

- `audio`: `audio::list_microphones_for_app`, `stop_recording_for_app`, `cancel_recording_for_app`, `record_test_clip_for_app`, `get_test_clip_audio_for_app`; `dictation::transcribe_recording_for_app`; `incremental::emit_partial_transcript`, `start_session`; `sounds::play`; `dictation_worker::dispatch`, `request`; `file_transcribe::transcribe_file`.
- `output`: `output::handle_transcription_output`, `paste_text`, `paste_last_transcript`, `copy_last_transcript`, `paste_transcript`, `copy_transcript`, `emit_output_failed`; `selection_transform::apply_result`, `transform_selection_blocking`.
- `models`: `model_manager::list_models`, `download_model`, `retry_model_download`, `delete_model`, `select_model`, `selected_model_path`, `model_path_for_id`, `session_model_path`, `models_dir`, `smallest_downloaded_model_path`; `whisper::transcribe`; `gpu::probe`; `ParakeetProcess::launch`; `WarmTranscriber::transcribe`.
- `hotkeys`: `hotkeys::emit_registration_failures`, `set_transform_enabled`, `replace_hotkeys`, `status`.
- `data`: `data_hygiene::effective_data_dir`, `logs_dir`, `failed_recordings_dir`; `diagnostics::collect`; `replacements_file::notify_backup`; `settings_store::patch_settings_now`.
- `sync`: `note_sync::collect_and_sync`, `collect_and_sync_all_transcripts`, `collect_sync_attempt`, `record_sync_attempt`.
- `status`: `status_file::publish`.
- `shell.rs`: `update_tray_status`, `update_engine_summary`, `sync_settings`, `toggle_dashboard`.

These wrappers still have callers; switch each caller to the engine version, then delete the wrapper:

- One caller in another lane: `hotkeys::start_transform` (`dictation_worker.rs:596`), `whisper::transcribe_with_output_prefix` (`file_transcribe.rs:94`), `audio::start_recording_for_app` (`hotkeys.rs:1413`), `audio::emit_recording_error` (`hotkeys.rs:1310`), `dictation_state::emit_state_snapshot` (`output.rs:375`, `:498`).
- `dictation_worker::submit`: `hotkeys.rs` (`:717`, `:730`, `:1230`, `:1244`, `:1268`, `:1272`, `:1291`, `:1303`, `:1367`) and `tray.rs` (`:118`-`:120`).
- `settings_store::emit_changed`: `commands/models.rs:96`, `:116`; `commands/hotkeys.rs:125`, `:167`, `:203`, `:237`; `commands/sync.rs:187`, `:229`; `commands/shell.rs:32`.
- `data_hygiene::open_folder` (also a Tauri API user): `commands/models.rs:23`; `commands/data.rs:320`, `:378`, `:401`.
- `settings_store::set_launch_at_startup`, `sync_launch_at_startup` (through the `commands.rs` re-export): `tray.rs:162`, `:187`; `lib.rs:631`.
- `autostart::apply` and `follow_windows_switch` (step 1 made them wrappers): `settings_store.rs:247`, `:156` (the `data` lane switches them to the `_with` versions); `autostart::reconcile_at_launch`: `lib.rs:444` (step 3).
  Step 4 deletes the three wrappers.
- Setup and the shell's handlers in `lib.rs`: `RemoteCommand::run` (`:304`), `hotkeys::handle_shortcut` (`:354`), `is_dev_flavor` (`:417`, `:576`), `data_hygiene::run_startup_maintenance` (`:467`), `WarmTranscriber::new` (`:484`), `state_server::start` (`:492`), `NoteSyncWorker::spawn` (`:504`), `DictationWorker::spawn` (`:508`), `hotkeys::setup` (`:560`), `test_audio::activate` (`:589`), `status_file::publish_idle` (`:594`, `:763`), `self_test::spawn` (`:601`).
- `display_name` (`tray.rs:92`, `:256`), and `shell::open_dashboard` and `shell::main_window_logical_size` (`commands/shell.rs:12`, `:22`).

The transitional `_with(app, core, ..)` functions to strip of their `app` are the ones in "Functions that still need an AppHandle" above.

Who deletes what:

- A step-2 lane deletes only the wrappers listed for it under "Wrappers to delete" (in its own files), never a wrapper that went dead after main 75614fc, even when its own change is what left it without callers; it lists that wrapper in its pull request instead, for step 4.
- Wrappers whose last caller sits in another lane's file stay until step 4, which deletes them: `whisper::transcribe_with_output_prefix` (`whisper.rs:128`) and its helper `whisper::host_of` (`:80`) once `audio` switches `file_transcribe.rs:94`; `audio::start_recording_for_app` (`audio.rs:437`) and `audio::emit_recording_error` once `hotkeys` switches its calls; `dictation_state::emit_state_snapshot` once `output` switches; `hotkeys::start_transform` (`hotkeys.rs:1375`), whose caller `dictation_worker.rs:596` keeps calling it until step 4 because `start_transform_with` still takes the app; `output::core_for` (`output.rs:157`), which the remaining `output` and `audio` wrappers use.
- `whisper::transcribe` (`whisper.rs:90`) has no caller on main, so the `models` lane deletes it in step 2 (but keeps `host_of`, which `transcribe_with_output_prefix` still uses).

### Work list and lane split

The lanes below never edit the same file at the same time.
Lanes in one step run in parallel; a step starts when the step before it is merged.
Each lane follows the rulebook above (unit tests with `RecordingHost` for every event it touches, test counts against main, "Definition of done"), and each lists in its pull request every wrapper or parameter it left for a later step.

| Step | Lane | Owns | Work |
|------|------|------|------|
| 1 | `seams` | `host.rs`, `tauri_host.rs`, `core.rs`, `autostart.rs`, `commands/mod.rs` and the three `on_dialog_thread` calls in `commands/data.rs` (`:346`, `:429`, `:534`), `data_hygiene::open_folder` only, `app/src-tauri/src/lib.rs:358` only (the autostart plugin), both `Cargo.toml` files and `Cargo.lock` | The `Host` methods above in all three hosts, with tests; `CoreCell` with tests; `open_folder_with(host, dir)` beside `open_folder`; `on_dialog_thread` changed in place to take the `Core`, with its three call sites updated; the autostart plugin calls behind the `Host` and `autostart::plugin()` built in the shell; add `global-hotkey` 0.8 to `crates/scribe-core/Cargo.toml` and `tauri-plugin-autostart` to `app/src-tauri/Cargo.toml`. |
| 1 | `e2e-log-check` | `.github/workflows/e2e-windows.yml`, `app/e2e/windows/run.py` | Fail the Windows end-to-end job when the app's log contains "The engine's folder differs from Tauri's" (`lib.rs:372`). |
| 2 | `sync` | the P1-3 `sync` files | `github_device_start` opens through the host; `github_device_poll` and `github_disconnect` call `emit_changed_with`; delete the four dead `note_sync` wrappers; a `NoteSyncWorker` constructor that takes a `CoreCell` (which tests can also use). |
| 2 | `models` | the P1-3 `models` files | Delete the `models` wrappers listed above, including `whisper::transcribe` but not `transcribe_with_output_prefix` or `whisper::host_of`; `open_models_folder` uses `open_folder_with`; `delete_model` and `select_model` call `emit_changed_with`. |
| 2 | `data` | the P1-3 `data` files | Opener, version and autostart through the host; delete the dead `data` wrappers; add a debug assertion in `settings_store::lock` that it is not taken on the main thread (with a test). |
| 2 | `hotkeys` | the P1-3 `hotkeys` files | Global shortcuts through the host, with the shortcut types from `global-hotkey`; call `dictation_worker::submit_with`, `audio::start_recording_with` and `audio::emit_recording_error_with`; the four `emit_changed` calls in `commands/hotkeys.rs` (`:125`, `:167`, `:203`, `:237`) become `emit_changed_with`; delete the dead `hotkeys` wrappers; drop `app` from every function whose callers are all in the lane. |
| 2 | `output` | the P1-3 `output` files | `notify` through the host; the paste guards call `dictation_state::emit_state_snapshot_with`; delete the dead `output` wrappers; drop `app` from every function whose callers are all in the lane. |
| 2 | `audio` | the P1-3 `audio` files, plus the `app_version()` uses in `status_file.rs:64` and `state_server.rs:483` | The self-test exits and reads the version through the host; `dictation_state::app_version()` replaced by the host's version in all three `appVersion` fields (the visible change above); `file_transcribe` calls `whisper::transcribe_with_output_prefix_with`; `request_with` off `tauri::async_runtime`; a `DictationWorker` constructor that takes a `CoreCell`, keeping its no-drop, in-order tests; delete the dead `audio` wrappers. |
| 3 | `wiring` | `app/src-tauri/src/lib.rs`, `tray.rs`, `commands/shell.rs`, `core.rs`, `shell.rs` | Build the `CoreCell`, give it to both workers and fill it after `Core::new`; `WarmTranscriber::new_with`; `run_startup_maintenance_with` with a backend clone; the replacements watcher through the host; `state_server::start_with`, `status_file::publish_idle_with`, `test_audio::activate_with`, `self_test::spawn_with`, `hotkeys::setup_with`, `handle_shortcut_with`, `RemoteCommand::run_with`, `settings_store::set_launch_at_startup_with` and `sync_launch_at_startup_with`; every `is_dev_flavor` and `display_name` caller on `Flavor` (the re-exports at app `lib.rs:12`-`:16`, `lib.rs:417`, `:576`, `tray.rs:92`, `:256`); the exit path, the window handlers, `save_window_size`, `missing_command_state` and the tray on the `Core`; manage only `Core` (and update the startup-order test and its `app.manage(` count); a test builder for `Core`; delete the `shell.rs` free functions. |
| 4 | `sweep` | every file in `crates/scribe-core/src` and `app/src-tauri/src` that still names `AppHandle` or imports through `crate::commands` | Remove the remaining `app` parameters (callers and callees together); delete the remaining wrappers, `output::core_for` and `settings_store::core_of`; delete `crates/scribe-core/src/commands.rs` and update its importers (`data_hygiene.rs:25`, `model_manager.rs:19`, `note_sync.rs:11`, `diagnostics.rs:22`, `whisper_server.rs:46`, core `lib.rs:15`, app `commands/mod.rs:21`, `lib.rs:24`, `commands/shell.rs:8`, `tray.rs:165`, `:180`); give the "engine not ready" answer a clear message. |
| 5 | `drop-tauri` | `crates/scribe-core/Cargo.toml`, `Cargo.lock`, `crates/scribe-core/src/lib.rs` | Delete `is_dev_flavor` and `display_name` (step 3 left them without callers); remove every `tauri` and `tauri-plugin-*` dependency from `scribe-core`. |
| 6 | `rename` | every engine file | Drop the `_with` suffix (see "Renaming" below). |

Step 4 deletes the `commands.rs` re-exports, not step 3, because five engine files import `BackendState` through it and step 4 already edits every engine file and runs alone; step 3 would have to take over those files from no one in particular.
"Manage only `Core`" means the engine: the tray still manages its own `LaunchAtStartupItem` (`tray.rs:88`), which is shell state, so the startup-order test counts the `app.manage(` calls in setup only.

The answer for a missing engine (`host::no_main_window()`, `host.rs:97`) keeps its error code `window_not_found`, because commands return it and a caller may match on it (nothing in `app/src` or the CLI does today); step 4 may change only its message ("Could not find the main window.") to say the engine is not ready yet, and lists that as a visible text change.

The merge order inside step 2 is free: with the deletion rules above, no step-2 lane depends on another.
Every lane merges main into its branch before it merges.
Step 2 lanes may stop using `app` inside a function that another lane calls, but must keep its parameter (renamed `_app`) until step 4, so no lane edits another lane's call sites.

#### Done when, per step

- Step 1: the new `Host` methods and `CoreCell` exist with tests; `RecordingHost` records the new calls; `on_dialog_thread` takes the `Core`; `autostart::plugin()` is built in the shell; no engine behavior changed (same test counts plus the new tests); every check is green.
- `e2e-log-check`: the Windows end-to-end job fails on a run whose log contains the mismatch line (shown once by a deliberate test, then reverted) and passes on main.
- Step 2, each lane: no function in your files takes an `AppHandle` except those this doc leaves for step 4 (the cross-lane ones, the wrappers the shell still calls, and the `_app` parameters other lanes pass); none of the wrappers listed for you under "Wrappers to delete" is left; your pull request lists every wrapper that your change left without callers; every check is green.
- Step 3: setup calls `app.manage(` once (the `Core`); `grep -rn "state::<" app/src-tauri/src` finds no lookup of an engine part other than the `Core`; the install-and-launch jobs and the Windows end-to-end job pass.
- Step 4: `grep -rn AppHandle crates/scribe-core/src` finds nothing; `crates/scribe-core/src/commands.rs` is gone; every check is green.
- Step 5: `crates/scribe-core/Cargo.toml` names no `tauri` crate and `cargo tree -p scribe-core | grep tauri` prints nothing; every check is green.
- Step 6: no engine version has a `_with` twin left; every check is green.

#### Renaming

Step 6 drops the suffix only from the P1-3 engine versions whose old wrapper is gone, one name at a time, never with a blanket search and replace of `_with`.
Names that must not be renamed, or need care:

- `DictationWorker::spawn_with` (`dictation_worker.rs:254`) is a test-only constructor that takes an executor, not an engine version; renaming it would clash with `spawn`, so leave it or call it `spawn_with_executor`.
- `Dispatcher::start_with` in `sounds.rs:271` is a private helper of the sound player, not an engine version; leave it.
- `whisper::transcribe_with_output_prefix_with` becomes `transcribe_with_output_prefix`; only the trailing `_with` goes.
- `stop_recording_with_reason` (`audio.rs`), the test helper `restore_with` (`output.rs:1256`) and test names that end in `_with` (for example `a_dictation_keeps_the_model_it_started_with`) are not engine versions; leave them.
- `gpu::probe_with` (`gpu.rs:209`) becomes `probe`, with a different signature from the old wrapper (`host, Option<&WarmTranscriber>`).
- `status_with`, `run_with`, `publish_with`, `transcribe_with`, `start_with` (`state_server.rs:617`) and `new_with` share their new name with other functions in other types or modules (there are several other `status`, `publish`, `run`, `start` and `new` functions); that compiles, but check every `use` that imports by name.

#### Checking Windows code locally

On this machine (WSL) a Windows-target clippy does not work yet: `cargo clippy --locked -p scribe-core --target x86_64-pc-windows-gnu --all-targets` (the `x86_64-pc-windows-gnu` Rust target is installed) stops in the build script of `ring`, which needs the MinGW C compiler `x86_64-w64-mingw32-gcc`, and that is not installed (checked on 2026-11-26).
So until someone installs MinGW-w64 here and that command passes, the Windows-only code is proven only by the Windows CI jobs (the Windows desktop backend check and the Windows end-to-end job), and a lane that changes it must wait for them before it reports done.
Once the command works, every lane that touches Windows-only code runs it, through `devguard slot`, before pushing.

Still open after P1-4: when the engine's folders move onto `Paths`, Tauri's answer wins wherever the two disagree until the switch, and the startup log (which reports every disagreement) is checked on Windows, macOS and Linux installs first.
