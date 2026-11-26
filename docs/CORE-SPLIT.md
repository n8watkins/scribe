# Splitting the engine from the shell

Scribe is moving from a Tauri app to a native Slint app.
Phase 1 separates the engine (`crates/scribe-core`) from the interface inside today's shipping Tauri app, without changing what the app does.

- P1-1 moved the engine into `crates/scribe-core`.
- P1-2 added the seams: `Paths`, `CoreEvent`, `Host`, `Core`, and split the Tauri commands into one shell file per domain.
- P1-3 converted the engine modules from `AppHandle` to `&Core` and `Host`, in seven parallel lanes (#145 to #155).
- P1-4 removes what P1-3 left behind (the `AppHandle` wrappers and parameters, the individually managed state, the `shell` free functions) and makes the engine free of Tauri, in six steps.
  Step 1 (`seams` #159, `e2e-log-check` #158) and step 2 (`output` #163, `data` #161, `audio` #165, `models` #162, `sync` #160, `hotkeys` #164) are merged.
  Steps 3 to 6 remain; each has its own section under "For P1-4", written so that it can be handed to a fresh agent with this file as its only instructions.

This file describes main as of 550f041, and file:line references are to that commit.
If main has moved when you start, check the references you rely on before you change code.

Behavior never changes along the way: same commands and names, same event names and payloads, same data folders, same installers.
The one intended visible change so far is #165's: `appVersion` in `status.json`, `control.json` and the state stream is the app's own version, which in Dev and installer-upgrade builds is no longer the base version.

## The seams

| File | What it is |
|------|------------|
| `crates/scribe-core/src/paths.rs` | `Paths`: every folder the engine uses (data, cache, log, resource, home), resolved exactly as Tauri resolves them, without Tauri. |
| `crates/scribe-core/src/events.rs` | `CoreEvent`: one variant per event the engine sends (22), carrying today's payload. `name()` is today's event name and the event serializes as its bare payload. |
| `crates/scribe-core/src/host.rs` | `trait Host`: what the engine needs from its shell (`emit`, `paths`, `flavor`, the tray and main-window calls, the app's version, exiting, opening a path or URL, notifications, global shortcuts, the startup entry). `Flavor` (Stable or Dev). `RecordingHost`, a host for tests. |
| `crates/scribe-core/src/core.rs` | `Core`: a cheap, cloneable handle owning the engine's parts (`backend()`, `warm_transcriber()`, `dictation_server()`, `note_sync()`, `dictation_worker()`, `hotkey_runtime()`, `replacements()`) and its `host()`, with `emit()`, `paths()` and `flavor()` shortcuts, and `shutdown()`. `CoreCell` hands the `Core` to the workers built before it. |
| `crates/scribe-core/src/backend.rs` | `BackendState` (was in `commands.rs`): the database, the state machine, the audio service and the shared locks. |
| `crates/scribe-core/src/shell.rs` | The installed `Host` (`shell::install`, `shell::host()`), which the remaining host-only wrappers use, and the old tray and window free functions, which forward to it. |
| `app/src-tauri/src/tauri_host.rs` | `TauriHost`: `Host` on Tauri. `emit` sends `app.emit(event.name(), &event)`. |
| `app/src-tauri/src/commands/<domain>.rs` | The `#[tauri::command]` functions, one file per domain. `commands/mod.rs` has `off_main` (takes the app), `off_main_core` and `on_dialog_thread` (take the `Core`). |

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
They are correct only while `Paths` agrees with Tauri, which is why the startup mismatch log matters: since #158 the Windows end-to-end job and the Linux and macOS install-and-launch scripts fail when the app's log contains it (`app/e2e/folder_mismatch.py`).

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
Until step 3, three of those find what they need without the `Core`: the folder clean-up looks up the managed `BackendState`, `WarmTranscriber::new` takes the installed host and the managed `BackendState`, and the note-sync and dictation workers poll `try_state::<Core>()` until it is managed.
Step 3 manages only the `Core`, hands those parts what they need directly, and gives the workers a `CoreCell` (see "Step 3").

At exit the shell calls `Core::shutdown()`, which writes an idle status file, stops the dictation-state server and stops the speech engine, in that order.
Stopping the speech engine (`WarmTranscriber::shutdown`, #154) first sets a `shutting_down` flag, then stops every speech helper process it started (whisper-server, scribe-parakeet) by its recorded identity, without taking the engine lock, and tidies the engine slot only if the lock is free.
From then on nothing may start a load: `prewarm`, the background load, `run_unloaded`, `load` and the whisper-cli fallback all refuse (with the error `speech_engine_stopped` where they return one).
Any new code path that can load a model must check the same flag.
Nothing in the app may rely on drop to stop anything: the handles are shared and Tauri never drops its managed state.
(`WarmTranscriber` has a safety net that stops the engine when its last handle drops, for tests and for a shell that drops its `Core`; the app never reaches it.)
The native shell must call `Core::shutdown()` on its way out too.

Tests in `app/src-tauri/src/lib.rs` pin the startup order, the guards, the number of `app.manage(` calls (8, `lib.rs:1118`) and the 79 command names (`:871`); `app/src-tauri/tests/tauri_host.rs` checks `Paths` against Tauri's resolver for both flavors and the emitted payloads against the engine's own `app.emit` calls; `app/src-tauri/tests/models.rs` does the same for the models folder and the two models events.

### How the engine knows the main thread

The settings write lock must never be taken on the main thread (see the lock rules below), and since #161 `settings_store::lock` debug-asserts it (`settings_store.rs:43`-`:51`).
The engine has no seam for "the main thread": `on_main_thread` (`:56`) asks whether the current thread is named "main", the name Rust's runtime gives the thread that runs `main`, and no Scribe thread takes that name.
In the Tauri shell that is the event-loop thread, because `main` calls `app_lib::run()` (`app/src-tauri/src/main.rs`) and Tauri runs its event loop on the calling thread on Windows, macOS and Linux.
A native shell must keep its event loop on the Rust main thread too: with the event loop on any other thread this assertion goes blind, and a settings change taken on the event-loop thread could deadlock against its own tray and hotkey calls unnoticed.
The tests are `the_settings_lock_is_taken_off_the_main_thread` and `taking_the_settings_lock_on_the_main_thread_panics_in_debug_builds` (`settings_store.rs:476`, `:489`).

## Rules every remaining step keeps

P1-3 and P1-4 step 2 followed a longer rulebook for converting one function at a time; it is in this file's history (`git show 550f041:docs/CORE-SPLIT.md`).
What still applies to steps 3 to 6:

- Never replace `app.package_info().version` with the engine crate's `env!("CARGO_PKG_VERSION")`: Dev builds (`release.yml` gives each one a pre-release version such as `0.16.4-57`) and the installer-upgrade test build (`installer-upgrade.yml`, `--config` with a raised version) override the app's version.
  The engine reads the app's version from `Host::app_version()`.
- A folder the engine writes into or deletes from comes from `core.backend().dirs()`, not from `core.paths()` (see "Which folder value to use").
- Send the same events: the `CoreEvent` variant whose `name()` is today's name, with today's payload type (the `events.rs` tests pin every payload).
  Do not rename or move the payload types or the event-name constants `events.rs` names.
- Code that finds the `Core` through the app uses `app.try_state::<Core>()`, never `app.state::<Core>()`, which panics (a panic on the dictation worker kills the one thread every dictation runs on).
  When the `Core` is missing it logs a warning and answers as with no main window: `host::no_main_window()`, `None` or nothing.
- A synchronous command stays synchronous: only its arguments change, never `fn` to `async fn`, because that moves it off the main thread.
  The synchronous commands are `get_logs_dir`, `get_failed_recordings_dir`, `whats_new_pending`, `dismiss_whats_new` and `collect_diagnostics` (`commands/data.rs`), `github_device_cancel` (`commands/sync.rs`), and `open_dashboard` and `open_release_page` (`commands/shell.rs`).
  An `async` command runs its blocking work with `off_main_core(core.inner().clone(), |core| ..)`.
  The windows pass no argument for `State`, so a command that swaps `app: tauri::AppHandle` for `core: tauri::State<'_, Core>` keeps its interface.
- Never call the host (an event, the tray, a window, a sound) while holding an engine lock (the speech engine's lock, the dictation state machine's lock, the database lock).
  On Tauri the tray calls wait for the main thread, and at exit the main thread waits for the engine lock in `Core::shutdown`: that is the quit deadlock #148 fixed.
  Read what you need, drop the lock, then call the host (as `dictation_worker::refresh_tray` does).
- The one lock held across host calls on purpose is the settings write lock (`BackendState::settings_writes`, taken through `settings_store::lock`): it is held from reading the settings to announcing them, so the events go out in the order the writes happened, and while it is held the announcement (`emit_changed_with`) updates the tray menu and a settings change registers hotkeys.
  So it is never taken on the main thread (see "How the engine knows the main thread"); every caller runs off it (the async commands inside `off_main` or `off_main_core`, the tray and window-focus handlers on their own `launch-at-startup` threads).
- Once the shell's event loop runs, the engine calls `Host::register_shortcut` and `unregister_shortcut` only from its own threads (Tauri's plugin waits for the main thread); only setup registers the first hotkeys on the main thread, before the event loop starts.
- A test that builds a Tauri mock app lives in `app/src-tauri/tests/`: `build.rs` embeds the Windows app manifest into integration tests only, and a mock app in a unit test fails to start on Windows (`STATUS_ENTRYPOINT_NOT_FOUND`).

### Testing the event contract

The Windows end-to-end job is not proof of the event contract.
It drives real dictations, the default hotkeys and their watchers (#153) and the paste, and it fails when a dictation does not reach its target, but it does not check event names, payloads, their order, how many times an event is sent, or the error paths.
For example, after P1-3 a failed paste sent `scribe:output-failed` twice and every check passed; #156 fixed it.
So every change to a function that sends events or calls the host keeps or adds unit tests with `RecordingHost` (on `Paths::from_parts` over a temporary folder) that pin:

- the event name and the exact JSON payload;
- the order of the events relative to each other and to the other side effects (for a new state: the `scribe:app-state` event, then the status file, then the dictation-state server; for the speech engine: the event, then the tray line; for settings: one `scribe:settings-changed`, after the save);
- how many times each event is sent, including "exactly once" on failure paths and "none" where nothing may be sent;
- the error paths: what the function returns and sends when the `Core` or the host is missing, when the database fails, and when a step in the middle fails (a rollback, a quarantined recording).

Until step 3 adds a test builder, no test can build a `Core`, so the lanes tested the body of each `_with(core)` function through a private function over the parts it uses (`&BackendState`, `&dyn Host`, a store) or through a small trait (`hotkeys::ActionTarget`, `dictation_state::Parts`).

### Definition of done for every step

A step's pull request is ready when:

- it changes only the files its section lists;
- locally `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings` (with and without `--features e2e-audio`) and `cargo test --locked --workspace` pass, each run through `devguard slot -- <command>` (which waits for a free slot of the machine's shared build and test budget);
- it reports the test counts of every test binary against main's, and no test stopped running;
- every check on GitHub is green (the three backends, the bundles, install-and-launch, Windows end-to-end, Release SBOM);
- its description lists every wrapper or parameter it left for a later step and every function its change left without a caller.

## Functions that still need an AppHandle

This is every function in `crates/scribe-core/src` and `app/src-tauri/src/commands` that still takes an `AppHandle` on main (550f041), apart from the old-signature wrappers, which "Step 4" lists with their callers.
It was rebuilt from a scan of every function signature that names `AppHandle` (multi-line signatures and `_app` parameters included) and of the two types that hold one (`AppExecutor`, `AppEffects`), and reconciled with the "left for a later step" tables of #159 to #165.
Step 2 dropped the app from every function whose callers were all in one lane, so each row is one of:

- **Unused**: an `_app` parameter nothing reads, kept because a caller in another lane's file passes it.
- **Pass-through**: the body only passes the app on, in the end to an unused parameter or to a wrapper.
- **Tauri API**: the body itself uses the app.

The `models`, `sync` and `status` files have none apart from their wrappers.
Step 3 changes `open_dashboard` and `save_window_size` in `commands/shell.rs` and every caller of `is_dev_flavor` and `display_name` (step 5 deletes those two), step 4 removes the rest with callers and callees changed together, and `open_release_page` keeps the app.

| Function | Kind | Passes the app to, or uses |
|---|---|---|
| `output.rs:171` `handle_transcription_output_with(_app, ..)` | Unused | caller `dictation.rs:372` |
| `output.rs:238` `paste_last_transcript_with(_app, ..)` | Unused | caller `dictation_worker.rs:664` |
| `selection_transform.rs:246` `apply_result_with(_app, ..)` | Unused | caller `dictation.rs:499` |
| `hotkeys.rs:825` `replace_hotkeys_with(_app, ..)` | Unused | callers `settings_store.rs:273` and `commands/hotkeys.rs:102`, `:109`, `:115`, `:153`, `:157`, `:188`, `:193`, `:222`, `:227` |
| `dictation.rs:71` `transcribe_recording_with`, `:102` `transcribe_recording_checked`, `:227` `transcribe_recording_inner` | Pass-through | `handle_transcription_output_with`, `finish_selection_transform` |
| `dictation.rs:492` `finish_selection_transform` | Pass-through | `apply_result_with` |
| `audio.rs:650` `finish_timed_out_recording_with`, `:699` `finish_disconnected_recording_with` | Pass-through | `dictation::transcribe_recording_with` |
| `dictation_worker.rs:610` `AppExecutor` (holds `app`), `:634` `execute_with`, `:697` `stop_and_transcribe`, `:718` `start_transform` | Pass-through | `paste_last_transcript_with`, `transcribe_recording_with`, `audio::finish_*_recording_with`, the wrapper `hotkeys::start_transform` |
| `dictation_worker.rs:351` `DictationWorker::spawn_on(cell, app)` | Pass-through | `AppExecutor` |
| `file_transcribe.rs:41` `transcribe_file_with` | Pass-through | the wrapper `whisper::transcribe_with_output_prefix` (`:88`) |
| `self_test.rs:79` `spawn_with(app, core, request)`, `:118` `run` | Pass-through | `transcribe_file_with` |
| `settings_store.rs:223` `apply_settings_change_with` and `:254` `AppEffects` (holds `app`) | Pass-through | `replace_hotkeys_with` (`:273`) |
| `settings_store.rs:140` `patch_settings_now_with`, `:200` `set_launch_at_startup_with` | Pass-through | `apply_settings_change_with` |
| `commands/audio.rs:80` `transcribe_file` | Pass-through | `transcribe_file_with` |
| `commands/data.rs:41` `patch_settings` | Pass-through | `patch_settings_now_with` |
| `commands/hotkeys.rs:76` `rebind_hotkey`, `:130` `set_hotkey_trigger`, `:169` `reset_hotkeys_to_defaults`, `:209` `load_production_hotkey_defaults` | Pass-through | `off_main` and `replace_hotkeys_with` |
| `commands/mod.rs:29` `off_main` | Pass-through | the closure it runs; callers are those four commands and `commands/shell.rs:20` |
| `lib.rs:70` (scribe-core) `is_dev_flavor`, `:75` `display_name` | Tauri API | `app.config().identifier`; `Flavor::is_dev()` and `Flavor::display_name()` replace them |
| `commands/shell.rs:11` `open_dashboard` | Pass-through | the free function `shell::open_dashboard` |
| `commands/shell.rs:19` `save_window_size` | Tauri API | `off_main`, `app.state::<BackendState>()` (`:21`), `shell::main_window_logical_size`, `settings_store::emit_changed` |
| `commands/shell.rs:47` `open_release_page` | Tauri API | the opener plugin; the shell's own command, it keeps the app |

Since step 2, no engine file imports a `tauri_plugin_*` crate, and `dictation_worker::request_with` no longer runs on `tauri::async_runtime` (#165).
The Tauri shell's own files (`app/src-tauri/src/lib.rs`, `tray.rs`, `window_size.rs`, `tauri_host.rs`) keep using the `AppHandle`: they are the shell.

## For P1-4

P1-4 removes the `AppHandle` wrappers and parameters, manages only `Core`, and drops the `tauri` dependency from `crates/scribe-core`.
It is done when no function in `crates/scribe-core/src` takes an `AppHandle`, `lib.rs` setup calls `app.manage(` exactly once (for the `Core`), `crates/scribe-core/Cargo.toml` names no `tauri` crate, behavior is unchanged (same commands and arguments, same events and payloads, same folders, same installers), and every check on GitHub is green.

| Step | Lane | State |
|---|---|---|
| 1 | `seams`, `e2e-log-check` | merged: #159, #158 |
| 2 | `output`, `data`, `audio`, `models`, `sync`, `hotkeys` | merged: #163, #161, #165, #162, #160, #164 (main 550f041) |
| 3 | `wiring` | next; one lane |
| 4 | `sweep` | after step 3; one lane, alone |
| 5 | `drop-tauri` | after step 4 |
| 6 | `rename` | after step 5 |

A step starts when the step before it is merged, and merges main into its branch before it merges.
Each step follows "Rules every remaining step keeps" and "Definition of done for every step", and these names stay as they are until step 6: the types `Core` holds (`DictationWorker`, `WarmTranscriber`, `HotkeyRuntimeState`, `BackendState`, `ReplacementStore`, `NoteSyncWorker`, `DictationServer`), each a cheap `Clone` handle, and the signatures `Core::shutdown` calls (`DictationServer::shutdown`, `WarmTranscriber::shutdown`, `status_file::publish_idle_with`).

### What steps 1 and 2 built

Steps 3 to 6 build on these names:

- `Host` (`host.rs`) has `app_version()` and `exit(code)` (both callable from any thread), `open_path(&Path)`, `open_url(&str)`, `notify(title, body)`, `register_shortcut`, `unregister_shortcut` and `is_shortcut_registered` (on `global_hotkey::hotkey::HotKey`), `launch_at_login()` and `set_launch_at_login(bool)`.
  The fallible ones return `Result<_, String>`, the plugin's error text, and the engine builds today's `CommandError` codes and messages around it.
  `TauriHost` answers them with the plugin calls the engine used to make; `tauri_host::autostart_plugin()` is the autostart plugin the shell registers (`lib.rs:358`).
- The engine names the global-shortcut types from `global-hotkey`, which the plugin re-exports: `HotKey`, `GlobalHotKeyEvent`, `HotKeyState`, `Code`, `Modifiers` (#164).
  The plugin's handler stays in the shell (`lib.rs:344`-`:357`) and calls the engine.
- `RecordingHost` records every event (`events()`) and, in a separate list, every call that acts (`calls()`, a list of `HostCall`: `Exit`, `OpenPath`, `OpenUrl`, `Notify`, `RegisterShortcut`, `UnregisterShortcut`, `SetLaunchAtLogin`; `host.rs:160`-`:168`).
  It answers `RECORDING_HOST_VERSION` (or `with_app_version(..)`), holds shortcuts and the startup entry the way the plugins do, and `fail(Failing::..)` makes one kind of call fail with `Failing::message()`.
  It records neither the queries (`app_version`, `is_shortcut_registered`, `launch_at_login`) nor the tray and main-window calls (`update_tray_status`, `update_engine_summary`, `sync_settings`, `open_dashboard`, `toggle_dashboard`; `host.rs:287`-`:299`), so two lanes wrote test-only wrapper hosts: `SequencedHost` (`output.rs:1096`, events and notifications in one ordered list) and `DashboardHost` (`hotkeys.rs:1880`, the main-window calls).
  `RecordingHost::unregister_shortcut` answers `Ok` for a shortcut it does not hold, where the real plugin may answer an error, so no test may rely on releasing an unheld shortcut succeeding.
- `CoreCell` (`core.rs:145`-`:166`): `CoreCell::new()`, `cell.fill(&core)` right after `Core::new(host, parts)` (whose signature did not change), and `cell.get()`.
  It holds `Arc<OnceLock<Weak<Inner>>>`; filling twice keeps the first and logs a warning; `get()` answers `None` both before the fill and after every `Core` handle is dropped.
- Worker constructors on a `CoreCell`: `DictationWorker::spawn_on(cell, app)` (`dictation_worker.rs:351`; the app only passes through, until step 4) and `NoteSyncWorker::spawn_with(cell)` (`note_sync.rs:325`).
  Both ask the cell once per job or sync and drop the `Core` after it; before the fill they poll every 50 ms, keep jobs in arrival order and drop none.
  Once a job has found the `Core` and every `Core` is gone, the dictation worker answers later jobs `dictation_worker_unavailable`.
  `DictationWorker::call_async` (`:421`) returns a runtime-free future, which `request_with` (`:601`) awaits instead of using `tauri::async_runtime`.
- Engine versions the shell can call today without the app: `WarmTranscriber::new_with(host, backend, measurements_path)` (`whisper_server.rs:409`), `data_hygiene::run_startup_maintenance_with(&BackendState, ..)` (`data_hygiene.rs:210`), `state_server::start_with(Arc<dyn Host>)` (`state_server.rs:623`), `status_file::publish_idle_with(&dyn Host)` (`status_file.rs:123`), `test_audio::activate_with(&dyn Host)` (`test_audio.rs:57`), `autostart::reconcile_at_launch_with(&dyn Host, wanted)` (`autostart.rs:221`, `:258`), `hotkeys::setup_with(core, hotkeys, transform_enabled)` (`hotkeys.rs:513`), `hotkeys::handle_shortcut_with(core, shortcut, event)` (`:556`), `RemoteCommand::run_with(core)` (`remote_command.rs:57`), `dictation_worker::submit_with(core, intent)` (`dictation_worker.rs:591`), `settings_store::sync_launch_at_startup_with(&Core)` (`settings_store.rs:159`) and `settings_store::emit_changed_with(&dyn Host, ..)` (`:61`).
  Two still take the app until step 4: `self_test::spawn_with(app, core, request)` (`self_test.rs:79`) and `settings_store::set_launch_at_startup_with(app, core, enabled)` (`settings_store.rs:200`).
- The app's version: the self-test, `whats_new_pending`, `collect_diagnostics` and the three `appVersion` fields read `Host::app_version()`; `dictation_state::app_version()` is gone (#165), and `release_notes::pending_version_with(core)` and `diagnostics::collect_with(core)` take no version argument (#161).
- `on_dialog_thread(core, |core| ..)` (`commands/mod.rs`) takes the `Core`.
- No engine file imports a `tauri_plugin_*` crate, and `crates/scribe-core/Cargo.toml` no longer names `tauri-plugin-autostart` (#159).

### Known issues after step 2

None of these blocks a step.

- On Linux a speech helper is not stopped at quit if its program file was replaced while it ran (an update): `/proc/<pid>/exe` then names a deleted file, so the identity check no longer matches the recorded one (`process_info.rs:42`-`:46`, `:185`).
- A GPU probe skipped because Scribe is quitting is logged and remembered as failed for the session: `whisper_server.rs:568`-`:569` turns the refused `run_unloaded` into `not_probed`, which `gpu.rs:240`-`:251` records as a failure.
- `update_check.rs` (`:36`, `:59`) and the HTTP user agents (`note_analysis.rs:62`, `:151`, `github_oauth.rs:543`, `github_backup.rs:71`) use the engine crate's `CARGO_PKG_VERSION`, which differs from the app's in Dev and installer-upgrade builds; P1-4 leaves them alone on purpose.
- In flight: #166 (`test/shutdown-kill-test-deadline`) makes the engine kill tests in `whisper_server.rs` and `process_info.rs` deterministic (one flaked under load during step 2); no step edits those two files until it is merged.

### Hazards

These apply to every remaining step; each step's section adds its own.

- **The `Core` and its threads keep each other alive.**
  Since #165 the dictation worker no longer keeps the `Core` it found (the note-sync worker never did): both get it once per job or sync and drop it after.
  Other long-lived owners remain: the sound player thread holds a `BackendState` clone for the life of the process (its handle sits in a process-wide static, `sounds.rs:315`-`:346`); the Windows toggle-key and chord watcher threads own a `Core` clone until their stop flag is set (`hotkeys.rs:614`, `:1011`), and their handles sit in the hotkey runtime the `Core` holds; the recording timeout thread and the audio stream's error callback hold `DictationWorker` clones; the replacements watcher thread never ends (`replacements_file.rs:316`-`:326`).
  Tauri never drops its state, so none of this matters in the Tauri shell.
  A native shell that expects dropping the `Core` to stop these threads needs `Core::shutdown` to set the watchers' stop flags, and it must still call `Core::shutdown()`.
- **Windows-only code.**
  Large parts of the code these steps change compile only on Windows: the toggle-key watcher and its key handlers (`hotkeys.rs:599`-`:736`), the note chord (`:1307`-`:1367`), the chord watcher (`:1487`-`:1734`), the startup-entry registry code (`autostart.rs:220`, `:293`, `:360` and the rest of its `#[cfg(windows)]` items) and the title-bar styling in setup (`lib.rs:581`).
  Local clippy and tests on Linux or WSL never compile them, so only the Windows CI jobs (and macOS CI for the macOS code) prove them; see "Checking Windows code locally".
- **Locks.**
  `Core::shutdown()` must stay free of the engine lock, and no new code path may start a model load once `shutting_down` is set (#154); see the lock rules under "Rules every remaining step keeps".
- **A setup that fails never reaches the exit handler.**
  Tauri 2.12.7 (`Cargo.lock`) panics with "Failed to setup app" when the setup closure returns an error (`tauri-2.12.7/src/app.rs:1443`-`:1444`), so the run callback never sees `RunEvent::Exit` without a `Core`.
  The exit path's fallback for a failed setup (`lib.rs:761`-`:773`) is therefore dead in the Tauri shell; step 3 removes it rather than port it.
- **`-D warnings` and dead code.**
  A `pub` function compiles without callers, but a `pub(crate)` or private one, an import, or a struct field that nothing reads fails clippy the moment its last use goes.
  So a step deletes those in the same commit that removes their last use; step 4 names each case.

### Step 3: `wiring`

The shell builds the engine from its parts, manages only the `Core`, and hands the workers a `CoreCell`; the deferred tests that need a real `Core` get a builder.

**Owns:**

- `app/src-tauri/src/lib.rs`, `tray.rs`, `commands/shell.rs`;
- `crates/scribe-core/src/core.rs`, `host.rs`, `shell.rs`;
- in `dictation_worker.rs` and `note_sync.rs`, the worker constructors, their loops and the comments on them: the `CoreCell` fix below changes both loops, and no other step is editing those files;
- the test code of `output.rs`, `hotkeys.rs` (including its `#[cfg(test)] pub(crate) mod test_target`), `remote_command.rs` and `settings_store.rs` (the deferred tests, and retiring `SequencedHost` and `DashboardHost`), and of any module whose exact `calls()` assertions change with the new `HostCall` variants.

**Must not touch:** `whisper_server.rs` and `process_info.rs` (#166 is changing them), any `Cargo.toml` or `Cargo.lock`, and non-test code in the other engine files: the wrappers step 3 leaves without callers stay for step 4.
Step 3 adds no `Host` method, so no `impl Host` changes.

**Work:**

| # | What | Where |
|---|---|---|
| 1 | `CoreCell` gets a three-state answer, for example `lookup() -> CoreLookup { NotFilled, Gone, Ready(Core) }`, keeping `get()`. Prefer it over a bare `is_filled()`, whose caller must read it before `get()` or misread a fill that lands between the two. | `core.rs:145`-`:166` |
| 2 | The dictation worker: on a filled cell whose `Core` is gone, answer the waiting job and every later one `dictation_worker_unavailable` instead of polling. Today its loop keeps a local `found` flag, so a cell filled and emptied before any job ran looks unfilled, and the next job polls until the channel closes: a caller holding a worker handle keeps it open, so its `call_async` never resolves. Delete `DictationWorker::spawn(app)`, which only the shell calls. | `dictation_worker.rs:326`, `:465`-`:541` |
| 3 | The note-sync worker: on `Gone`, `wait_for_core` ends the thread instead of waking every 50 ms until the last sender drops. Delete `NoteSyncWorker::spawn(app)`. | `note_sync.rs:309`, `:358`-`:368` |
| 4 | Setup builds a `CoreCell` before the workers, starts them with `NoteSyncWorker::spawn_with(cell.clone())` and `DictationWorker::spawn_on(cell.clone(), app.handle().clone())`, and calls `cell.fill(&core)` right after `Core::new`, before anything that can start engine work. | `lib.rs:504`, `:508`, `:543` |
| 5 | Setup's other constructors: `autostart::reconcile_at_launch_with(host.as_ref(), ..)`; the `data-cleanup` thread gets a backend clone and calls `run_startup_maintenance_with`; `WarmTranscriber::new_with(host.clone(), backend.clone(), ..)`; `state_server::start_with(host.clone())`; `hotkeys::setup_with(&core, ..)`; `test_audio::activate_with`; `status_file::publish_idle_with`; `self_test::spawn_with(app.handle(), &core, request)`; `core.warm_transcriber().sync_with_settings(..)` in place of `app.state::<WarmTranscriber>()`. | `lib.rs:444`, `:460`-`:473`, `:484`, `:492`, `:560`, `:589`, `:594`, `:601`, `:606` |
| 6 | The replacements watcher sends `CoreEvent::ReplacementsChanged` through the host and reaches note sync through a `CoreCell` clone (`replacements_file::notify_backup_with`), never a `NoteSyncWorker` clone: its thread never ends, so a sender it held would outlive the `Core` and keep the note-sync thread alive. Start the watch after the fill, so no edit lands before the cell can answer. | `lib.rs:513`-`:534` |
| 7 | The handlers: the single-instance handler calls `run_with(&core)` and the shortcut handler `handle_shortcut_with(&core, ..)` on the `Core` they already look up; the window-focus handler calls `sync_launch_at_startup_with(&core)`, and it and the close handler read the settings through the `Core`. | `lib.rs:300`-`:304`, `:350`-`:354`, `:627`-`:631`, `:644` |
| 8 | Every `is_dev_flavor` and `display_name` caller on `Flavor` (`host.flavor().is_dev()` in setup; the tray takes the `Flavor` from the `Core` or `Flavor::from_identifier`), and both dropped from the app's re-exports. | `lib.rs:14`-`:15`, `:417`, `:576`; `tray.rs:92`, `:256` |
| 9 | The tray: `dictation_worker::submit_with(&core, ..)`, `set_launch_at_startup_with(&app, &core, wanted)`, `sync_launch_at_startup_with(&core)`, and the `BackendState` lookups on the `Core`. | `tray.rs:118`-`:120`, `:162`, `:165`, `:180`, `:187` |
| 10 | `commands/shell.rs`: `open_dashboard` takes the `Core` and calls `core.host().open_dashboard(None)` (staying synchronous); `save_window_size` runs on `off_main_core` and uses `core.backend()`, `core.host().main_window_logical_size()` and `emit_changed_with`. | `commands/shell.rs:11`-`:36` |
| 11 | Manage only the `Core`: delete the other seven `app.manage(` calls in setup, make `missing_command_state` check only the `Core`, and drop the exit path's failed-setup fallback (see "Hazards"). The tray's own `LaunchAtStartupItem` (`tray.rs:88`) is shell state and stays. | `lib.rs:457`, `:488`, `:495`, `:505`, `:509`, `:532`, `:538`; `:224`-`:251`; `:761`-`:773` |
| 12 | The startup-order test: the steps become host, database, backend, the clean-up thread, `child_guard::init`, the speech engine, the state server, both workers, `Core::new`, the fill, `app.manage(core)`, then the hotkeys, the tray and the windows; one `app.manage(`; and the guard checks follow the renamed calls (it looks for `command.run(app)` today). | `lib.rs:1068`-`:1119` |
| 13 | `shell.rs`: delete the six free functions; keep `install` and `host()`, which the remaining host-only wrappers use until step 4. | `shell.rs:38`-`:72` |
| 14 | `RecordingHost` records the tray and main-window calls: `HostCall::SyncSettings(AppSettings)`, `OpenDashboard(Option<String>)`, `ToggleDashboard`, `UpdateTrayStatus(AppStatus)`, `UpdateEngineSummary(String)` (all `Eq`), and gains one ordered log of events and calls (for example `sequence()`), so `SequencedHost` and `DashboardHost` can go (`DashboardHost` and `DashboardCall` live in `hotkeys::test_target`, which the `remote_command.rs` tests also use); update every test whose exact `calls()` list now gains an entry. | `host.rs:160`-`:168`, `:274`-`:303`; `output.rs:1096`; `hotkeys.rs:1854`-`:2000`; `remote_command.rs:131`, `:167` |
| 15 | A test builder for `Core` (a `#[cfg(test)]` module in `core.rs`): `BackendState::new` on `Database::open` in a temporary folder, `WarmTranscriber::new_with`, no `DictationServer`, `NoteSyncWorker::spawn_with(cell)`, a `DictationWorker` on a test executor (`spawn_with`, `dictation_worker.rs:362`, because `spawn_on` still needs the app), `HotkeyRuntimeState::default()`, `ReplacementStore::open` in the folder; it fills the cell, and tests end with `core.shutdown()`. | `core.rs` |
| 16 | The tests deferred until a `Core` can be built: see below. | |
| 17 | Stale notes in files step 3 owns: the comment on `the_engine_carries_the_app_version` (the engine no longer reports its crate version as `appVersion`, #165); setup's "each part is managed on its own" (`lib.rs:453`-`:455`) and the same claim in the startup-order test (`:1098`-`:1099`); "kept in Tauri's managed state" on both workers (`dictation_worker.rs:302`, `note_sync.rs:293`); the "step 3 lets it tell them apart" note (`dictation_worker.rs:344`-`:347`); `shell.rs`'s module comment. | `lib.rs:826`-`:827` and the rest as listed |

The deferred tests (item 16):

- `CoreCell` on a real `Core`: not filled, filled, and `Gone` once every `Core` is dropped (step 1 could test only the private `WeakCell`); with it, both worker regressions of items 2 and 3, each with a deadline so a regression fails instead of hanging.
- `emit_changed_with` updates the tray after `scribe:settings-changed` (`HostCall::SyncSettings`, `settings_store.rs:61`-`:65`); no test pins it today.
- The output paths: each of the three calls `finish_output` with the `emit_events` its caller asked for and sends no second `scribe:output-failed` (`output.rs:363`, `:428`, `:483`), and the paste guards publish through `emit_state_snapshot_with` (`:325`, `:448`).
  These paths use the real clipboard and send keystrokes (`platform::`), so pin them through a seam over the platform calls or keep the test to what a headless runner can do; a unit test never sends keystrokes.
- `register_hotkey_set_with` registers the bindings, then configures the chord watcher, then the toggle watcher (`hotkeys.rs:849`-`:868`).
  The watchers start only on Windows (`:610`, `:1007`), so this is a `#[cfg(windows)]` test, proven by the Windows backend check, and it stops the watchers it started.
- The settings-lock assertion on a real command path: `sync_launch_at_startup_with(&core)` on a thread named "main" panics in debug builds (`settings_store.rs:159`, `:188`).
- Not yet possible: `AppEffects`' autostart call (`settings_store.rs:259`-`:261`) needs an `AppEffects`, which holds the app until step 4; step 4 adds that test.

**Hazards:**

- Runtime failures the compiler does not catch once only `Core` is managed: `app.state::<WarmTranscriber>()` at the end of setup (`lib.rs:606`) panics on every launch; `save_window_size` reads `app.state::<BackendState>()` (`commands/shell.rs:21`) and panics when the user saves the window size; `missing_command_state` (`lib.rs:224`-`:251`) reports the unmanaged parts and `create_config_windows` (`:261`-`:269`) then fails setup.
- `WarmTranscriber::new(app, ..)` (`whisper_server.rs:388`-`:404`) looks up the managed `BackendState`; without it a release build logs a warning and builds a speech engine with no backend (only debug builds assert), so setup must call `new_with`.
- The startup clean-up looks up the managed `BackendState` too (`data_hygiene.rs:267`-`:276`); without it the clean-up of old temp recordings, editor copies, failed recordings past retention and orphaned clips is skipped with only a warning, so the thread gets a backend clone (item 5).
- The replacements watcher sends with `watch_app.emit(..)` and finds the note-sync worker with `try_state::<NoteSyncWorker>()` (`lib.rs:526`-`:530`), which finds nothing once only `Core` is managed (item 6).
- The new `HostCall` variants change what `calls()` returns wherever a tested function announces settings, opens the dashboard or updates the tray; run the whole suite before pushing.

**Done when:** setup calls `app.manage(` once; `grep -rn "state::<" app/src-tauri/src` finds only `Core` and the tray's `LaunchAtStartupItem`; neither worker has a `spawn(app)`; `SequencedHost` and `DashboardHost` are gone; the tests above exist, and the `AppEffects` one is listed for step 4; the install-and-launch jobs and the Windows end-to-end job pass; "Definition of done for every step" holds.

### Step 4: `sweep`

Every remaining `AppHandle` parameter and wrapper goes, and with them `output::core_for`, `settings_store::core_of`, `off_main`, `crates/scribe-core/src/commands.rs` and `shell.rs`.

**Owns:** every file in `crates/scribe-core/src` and `app/src-tauri/src` that names `AppHandle`, imports through `crate::commands`, calls `shell::host()`, or holds a function listed below.
It starts after step 3 is merged and after #166 is merged (it edits `whisper_server.rs`).

**Must not touch:** any `Cargo.toml` or `Cargo.lock` (step 5), and no `_with` renaming (step 6).

**Runs alone.**
Its three parameter chains below meet in shared files (`hotkeys.rs` holds both `start_transform` and `replace_hotkeys_with`; app `lib.rs` calls both `spawn_on` and `self_test::spawn_with`), and removing `commands.rs` and `shell.rs` touches files of every chain, so parallel lanes would edit the same files.
The work is mostly deletions, ordered so each commit keeps `-D warnings` green, and fits one reviewable pull request.

**Work, in this order:**

1. The dictation chain, one commit: drop the unused `_app` of `output::handle_transcription_output_with` (`output.rs:171`), `output::paste_last_transcript_with` (`:238`) and `selection_transform::apply_result_with` (`selection_transform.rs:246`); then the `app` of `dictation::transcribe_recording_with`, `transcribe_recording_checked`, `transcribe_recording_inner` and `finish_selection_transform` (`dictation.rs:71`, `:102`, `:227`, `:492`), `audio::finish_timed_out_recording_with` and `finish_disconnected_recording_with` (`audio.rs:650`, `:699`), and `execute_with`, `stop_and_transcribe`, `start_transform` and the `app` field of `AppExecutor` (`dictation_worker.rs:634`, `:697`, `:718`, `:610`; a field nothing reads fails clippy).
   `dictation_worker.rs:720` switches to `hotkeys::start_transform_with(core)` and `hotkeys::start_transform` (`hotkeys.rs:1370`, `pub(crate)`) goes in the same commit.
   `DictationWorker::spawn_on(cell)` loses its app, with its call in app `lib.rs`.
2. The settings chain, one commit: drop the unused `_app` of `hotkeys::replace_hotkeys_with` (`hotkeys.rs:825`) and the `app` of `AppEffects`, `apply_settings_change_with`, `patch_settings_now_with` and `set_launch_at_startup_with` (`settings_store.rs:254`, `:223`, `:140`, `:200`), `patch_settings` (`commands/data.rs:41`), the four hotkeys commands (`commands/hotkeys.rs:76`, `:130`, `:169`, `:209`, which move to `off_main_core`) and the tray's call (`tray.rs:162` after step 3).
   `off_main` (`commands/mod.rs:29`, `pub(crate)`) loses its last callers there and goes in the same commit.
   Add the test step 3 could not write: `set_launch_at_startup_with(&core, true)` on a test `Core` records `HostCall::SetLaunchAtLogin(true)` through `AppEffects::apply_autostart` (`settings_store.rs:259`-`:261`).
3. The file-transcription chain, one commit: `file_transcribe.rs:88` calls `whisper::transcribe_with_output_prefix_with(core.host().as_ref(), ..)`, and `whisper::transcribe_with_output_prefix` (`whisper.rs:120`, `pub(crate)`) and its helper `whisper::host_of` (`:80`, private) go in the same commit.
   This was planned for the `audio` lane in step 2 and deferred (#165): switching the caller alone leaves the `pub(crate)` wrapper dead and fails clippy.
   Then the `app` of `transcribe_file_with` (`file_transcribe.rs:41`), `self_test::run` and `spawn_with` (`self_test.rs:118`, `:79`), the `transcribe_file` command (`commands/audio.rs:80`) and the self-test call in app `lib.rs`.
4. The wrappers, each deleted with the imports it alone used (`use tauri::{AppHandle, Manager}` lines go when their last use does).
   All are `pub` unless marked, so they compile without callers; once step 3 has switched the callers marked "(step 3)", none has a caller left.

   | Wrapper | Where | Last caller on main 550f041 |
   |---|---|---|
   | `autostart::apply`, `reconcile_at_launch`, `follow_windows_switch` | `autostart.rs:168`, `:207`, `:277` | none, `lib.rs:444` (step 3 switches it), none |
   | `data_hygiene::run_startup_maintenance`, `open_folder` | `data_hygiene.rs:267`, `:430` | `lib.rs:467` (step 3), none |
   | `dictation_state::emit_state_snapshot` | `dictation_state.rs:118` | none (#163 switched the paste guards) |
   | `audio::start_recording_for_app`, with `output::core_for` (`pub(crate)`, `output.rs:157`) and its import (`audio.rs:31`) in the same commit | `audio.rs:432` | none (#164); it is `core_for`'s only caller (`audio.rs:437`) |
   | `audio::emit_recording_error` | `audio.rs:1911` | none (#164) |
   | `dictation_worker::submit` | `dictation_worker.rs:580` | `tray.rs:118`-`:120` (step 3) |
   | `hotkeys::setup`, `handle_shortcut` | `hotkeys.rs:493`, `:547` | `lib.rs:560`, `:354` (step 3) |
   | `RemoteCommand::run` | `remote_command.rs:47` | `lib.rs:304` (step 3) |
   | `self_test::spawn` | `self_test.rs:65` | `lib.rs:601` (step 3) |
   | `settings_store::emit_changed` | `settings_store.rs:69` | `commands/shell.rs:32` (step 3) |
   | `settings_store::sync_launch_at_startup`, `set_launch_at_startup`, with `core_of` (private, `:77`) and their re-export (`commands.rs:11`) in the same commit | `settings_store.rs:165`, `:212` | `lib.rs:631`, `tray.rs:187`, `tray.rs:162` (step 3) |
   | `state_server::start` | `state_server.rs:650` | `lib.rs:492` (step 3) |
   | `status_file::publish`, `publish_idle` | `status_file.rs:134`, `:142` | none (#165), `lib.rs:594`, `:763` (step 3) |
   | `test_audio::activate` | `test_audio.rs:49` | `lib.rs:589` (step 3) |
   | `WarmTranscriber::new` | `whisper_server.rs:388` | `lib.rs:484` (step 3) |
   | `model_manager::delete_model_with` (not an `AppHandle` wrapper; `delete_model_in` stays for `delete_model_announced_with`) | `model_manager.rs:194` | none (#162) |

5. Delete `crates/scribe-core/src/commands.rs` and `pub mod commands` (core `lib.rs:15`), and point its importers at `crate::backend::BackendState`: `data_hygiene.rs:25`, `model_manager.rs:18`, `note_sync.rs:11`, `diagnostics.rs:21`, `whisper_server.rs:46`, app `commands/mod.rs:21` (`pub use scribe_core::commands::*`), app `lib.rs:24`, `commands/shell.rs:8`, and whatever step 3 left in `tray.rs` (`:165`, `:180` on main).
   Step 4 does this rather than step 3 because five engine files import through it and step 4 edits every engine file anyway.
6. Once no engine code calls `shell::host()` (after item 4 its callers are gone), delete `shell.rs` and its process-wide `HOST`, `pub mod shell` (core `lib.rs:53`, whose module comment links to it at `:8`), `shell::install` in setup and its step in the startup-order test, and `shell` from the app's re-exports (`lib.rs:18`).
7. `host::no_main_window()` (`host.rs:152`) keeps its code `window_not_found`, because commands return it and a caller may match on it (nothing in `app/src` or the CLI does today); only its message ("Could not find the main window.") changes, to say the engine is not ready yet, listed in the pull request as a visible text change.
8. Stale comments that name the app or a later step in the files it edits (for example `hotkeys.rs:823`-`:824`, `file_transcribe.rs:36`-`:39`, `self_test.rs:76`-`:78`, `dictation_worker.rs:349`-`:350`, `:607`-`:609`, `:630`-`:633`, `:713`-`:717`).

**Hazards:** the `-D warnings` cases above (each `pub(crate)` or private item, unused import or unread field goes in the commit that removes its last use); `whisper_server.rs` must not be edited while #166 is open; the Windows-only code in `hotkeys.rs` and `autostart.rs` (see "Hazards") is proven only by the Windows CI jobs.

**Done when:** `grep -rn "AppHandle\|use tauri\|tauri::" crates/scribe-core/src` finds nothing but comments; `grep -rn "AppHandle\|off_main(" app/src-tauri/src/commands` finds only `open_release_page`; `crates/scribe-core/src/commands.rs` and `shell.rs` are gone; the `AppEffects` autostart test exists; "Definition of done for every step" holds.

### Step 5: `drop-tauri`

**Owns:** `crates/scribe-core/Cargo.toml`, `Cargo.lock`, `crates/scribe-core/src/lib.rs`.

**Work:**

- Delete `is_dev_flavor` and `display_name` (`lib.rs:68`-`:81`; step 3 left them without callers) and rewrite the module comment (`lib.rs:6`-`:8` still says the engine takes an `AppHandle` and emits Tauri events).
- Remove every Tauri dependency from `crates/scribe-core/Cargo.toml`: `tauri` (with `tray-icon`), `tauri-plugin-global-shortcut`, `tauri-plugin-opener`, `tauri-plugin-single-instance`, `tauri-plugin-log`, `tauri-plugin-updater`, `tauri-plugin-process` and `tauri-plugin-notification`.
  None of the plugins is used by any engine file even on main (no `tauri_plugin_` in `crates/scribe-core/src`): `tauri-plugin-notification` has been unused since #163 (`output::notify` goes through `Host::notify`), the opener since #159 (`data_hygiene::open_folder` opens through the host), the global-shortcut plugin since #164, and the single-instance, log, updater and process plugins are registered by the shell (`app/src-tauri/src/lib.rs:291`-`:361`), whose own `Cargo.toml` names them.
  `tauri` itself is used only for `AppHandle`, `Manager` and `State`, which step 4 removes.
- Keep `global-hotkey` (the engine's shortcut types) and rewrite its comment, which explains it through "the plugin above".
- Fix the version comment (`Cargo.toml:3`-`:5`): the crate still carries the app's version, but since #165 the engine reports `CARGO_PKG_VERSION` only in the update check and the user agents, no longer in the dictation-state interface.
  `release-kit` keeps bumping it with the app, and `the_engine_carries_the_app_version` keeps checking that.
- Check the comments that explain a dependency by the Tauri tree (`objc2` "already in the tree through Tauri and arboard", `ashpd` "through the single-instance plugin"): they stay true for the workspace, because the app crate still depends on Tauri; reword them if they read as the engine's own.

**Hazards:** `Cargo.lock` should change only in `scribe-core`'s dependency list, with no package added, removed or upgraded (the app crate still pulls in every Tauri crate).

**Done when:** `crates/scribe-core/Cargo.toml` names no `tauri` crate, `cargo tree -p scribe-core -e normal | grep -i tauri` prints nothing, and "Definition of done for every step" holds.

### Step 6: `rename`

**Owns:** every engine file, and the shell files that call a renamed function.

**Work:** drop the `_with` suffix from every engine version whose old wrapper is gone, one name at a time, never with a blanket search and replace of `_with`; that includes the engine functions step 2 added with no wrapper twin (`model_manager::open_models_folder_with`, `delete_model_announced_with`, `select_model_announced_with`, `data_hygiene::open_transcript_externally_with`, `replacements_file::open_replacements_file_with`).

Names that must not be renamed, or need care:

- `DictationWorker::spawn_with` (`dictation_worker.rs:362`) is a test-only constructor that takes an executor, not an engine version; leave it or call it `spawn_with_executor`.
  `DictationWorker::spawn_on` keeps its name; `NoteSyncWorker::spawn_with(cell)` becomes `spawn` (step 3 deleted `spawn(app)`).
- `Dispatcher::start_with` (`sounds.rs:269`) is a private helper of the sound player, not an engine version; leave it.
- `whisper::transcribe_with_output_prefix_with` becomes `transcribe_with_output_prefix`; only the trailing `_with` goes.
- `stop_recording_with_reason` (`audio.rs:738`), the test helper `restore_with` (`output.rs:1408`) and test names that contain or end in `_with` (for example `start_with_publishes_the_discovery_file_under_the_hosts_home`, `a_dictation_keeps_the_model_it_started_with`) are not engine versions; leave them.
- The private bodies named `_in` (`sync_launch_at_startup_in`, `delete_model_in` and others) are not engine versions; leave them.
- `gpu::probe_with` (`gpu.rs:193`) becomes `probe`; its signature (`host, Option<&WarmTranscriber>`) differs from the deleted wrapper's.
- `status_with`, `run_with`, `publish_with`, `transcribe_with`, `start_with` (`state_server.rs:623`) and `new_with` share their new name with other functions in other types or modules (there are several other `status`, `publish`, `run`, `start` and `new` functions), and the five step-2 names above match command names in `app/src-tauri/src/commands`; that compiles, but check every `use` that imports by name.

**Done when:** no engine function has a `_with` suffix except the ones this list keeps, and "Definition of done for every step" holds.

### Checking Windows code locally

On this machine (WSL) a Windows-target clippy does not work yet: `cargo clippy --locked -p scribe-core --target x86_64-pc-windows-gnu --all-targets` (the `x86_64-pc-windows-gnu` Rust target is installed) stops in the build script of `ring`, which needs the MinGW C compiler `x86_64-w64-mingw32-gcc`, and that is not installed (checked on 2026-11-26).
So until someone installs MinGW-w64 here and that command passes, the Windows-only code is proven only by the Windows CI jobs (the Windows desktop backend check and the Windows end-to-end job), and a lane that changes it must wait for them before it reports done.
Once the command works, every lane that touches Windows-only code runs it, through `devguard slot`, before pushing.

Still open after P1-4: when the engine's folders move onto `Paths`, Tauri's answer wins wherever the two disagree until the switch, and the startup log (which reports every disagreement) is checked on Windows, macOS and Linux installs first.
