//! The desktop shell: builds the Tauri app (plugins, windows, tray, commands,
//! run loop) on Scribe's engine, crates/scribe-core. The engine's modules are
//! re-exported here under their old `app_lib::` paths.

pub mod commands;
pub mod tauri_host;
pub mod tray;
pub mod window_size;

#[cfg(feature = "e2e-audio")]
pub use scribe_core::test_audio;
pub use scribe_core::{
    app_state, audio, autostart, backend, child_guard, data_hygiene, db, diagnostics, dictation,
    dictation_state, dictation_worker, display_name, error, export, file_transcribe, filler,
    github_backup, github_oauth, gpu, hotkeys, import, incremental, is_dev_flavor, model_manager,
    models, note_analysis, note_sync, output, parakeet, platform, process_info, prompt_budget,
    release_notes, remote_command, replacements_file, selection_transform, self_test,
    session_model, settings, settings_store, shell, sounds, speech_engine, state_server, stats,
    status_file, sync_history, text_replace, transcript, update_check, whisper, whisper_server,
};

pub use scribe_core::{core::Core, events, host, paths};

use commands::BackendState;
use db::Database;
use tauri::Manager;

/// Carry forward a pre-rebrand install's data after the LocalDictate->Scribe
/// rename.
///
/// Tauri derives the per-app data directory from the bundle identifier
/// (`%APPDATA%\<identifier>\`). Renaming the identifier from
/// `com.natkins.localdictate` to `com.natkins.scribe` therefore points the app
/// at a brand-new, empty directory and would strand the user's existing
/// SQLite DB, recorded clips, and downloaded Whisper models in the old folder.
/// This helper copies that data across on first run.
///
/// Best-effort by design: any failure is logged and swallowed so a migration
/// hiccup can never block app startup.
fn migrate_pre_rebrand_data(app: &tauri::AppHandle, new_data_dir: &std::path::Path) {
    // Derive the old identifier from the current one. If they match, this build
    // wasn't renamed (or is the old flavor) and there's nothing to migrate.
    let Some(old_data_dir) = pre_rebrand_data_dir(app, new_data_dir) else {
        return;
    };
    if !old_data_dir.is_dir() {
        return;
    }

    // Idempotency is gated on a completion marker written ONLY after a full,
    // successful migration — not merely on the new DB existing. If migration is
    // interrupted after the DB lands but before clips/models copy, guarding on
    // the DB alone would skip those forever; the marker lets the next launch
    // resume the (skip-existing) clips/models copy instead.
    let marker = new_data_dir.join(".rebrand-migrated");
    if marker.exists() {
        return;
    }

    log::info!(
        "Migrating pre-rebrand data from {} to {}",
        old_data_dir.display(),
        new_data_dir.display()
    );

    let new_db = new_data_dir.join("scribe.sqlite3");
    // Track whether every step succeeded; the marker is only written if so, so
    // an incomplete migration retries on the next launch.
    let mut complete = true;

    // Copy the DB only when the destination has none yet. Stage to a temp file
    // and atomically rename into place, so a crash or failed copy can never
    // leave a truncated scribe.sqlite3 that the app would then try to open.
    if !new_db.exists() {
        let old_db = old_data_dir.join("localdictate.sqlite3");
        if old_db.is_file() {
            let staging = new_data_dir.join("scribe.sqlite3.migrating");
            let _ = std::fs::remove_file(&staging);
            match std::fs::copy(&old_db, &staging) {
                Ok(_) => {
                    // Sidecars first (SQLite recovers if they're stale), then
                    // the main DB is renamed in last as the atomic commit point.
                    for (old_name, new_name) in [
                        ("localdictate.sqlite3-wal", "scribe.sqlite3-wal"),
                        ("localdictate.sqlite3-shm", "scribe.sqlite3-shm"),
                    ] {
                        let src = old_data_dir.join(old_name);
                        if src.exists() {
                            let _ = std::fs::copy(&src, new_data_dir.join(new_name));
                        }
                    }
                    if let Err(error) = std::fs::rename(&staging, &new_db) {
                        log::warn!("Could not finalize migrated database: {}", error);
                        let _ = std::fs::remove_file(&staging);
                        complete = false;
                    }
                }
                Err(error) => {
                    log::warn!(
                        "Could not copy database during rebrand migration: {}",
                        error
                    );
                    let _ = std::fs::remove_file(&staging);
                    complete = false;
                }
            }
        }
    }

    // Clips and models: resumable (skip-existing), so re-running after an
    // interrupted migration just fills in whatever is missing.
    for subdir in ["clips", "models"] {
        let src = old_data_dir.join(subdir);
        if src.is_dir() {
            if let Err(error) = copy_dir_skip_existing(&src, &new_data_dir.join(subdir)) {
                log::warn!(
                    "Could not copy {} directory during rebrand migration: {}",
                    subdir,
                    error
                );
                complete = false;
            }
        }
    }

    if complete {
        let _ = std::fs::write(&marker, b"migrated\n");
        log::info!("Pre-rebrand data migration complete");
    } else {
        log::warn!("Rebrand migration incomplete; it will retry on the next launch");
    }
}

/// The pre-rename data folder (`com.natkins.localdictate`, or `.dev` for the
/// Dev flavor) next to the current one, or None when this build was never
/// renamed.
fn pre_rebrand_data_dir(
    app: &tauri::AppHandle,
    new_data_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let old_identifier = app.config().identifier.replace(".scribe", ".localdictate");
    if old_identifier == app.config().identifier {
        return None;
    }
    // The data dir lives directly under %APPDATA%; the old install used the
    // same parent with the old identifier as the folder name.
    Some(new_data_dir.parent()?.join(old_identifier))
}

/// Recursively copy `from` into `to`, creating `to` (and subdirectories) as
/// needed and skipping any destination file that already exists. std-only.
fn copy_dir_skip_existing(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_skip_existing(&src, &dst)?;
        } else if !dst.exists() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Routes panics into the app log in addition to the default stderr behavior.
///
/// Several subsystems run on spawned threads (the audio worker, the timeout
/// thread, the hotkey chord/toggle watchers, and the GitHub note-sync
/// worker). A panic on one of those threads is only observed where a `join()`
/// exists; everywhere else it dies silently with nothing in the log file,
/// which makes field diagnosis from a user's logs nearly impossible. Chaining
/// the previous hook keeps the standard stderr backtrace while also emitting an
/// `log::error!` line (which the file logger captures) with the thread name and
/// panic location.
fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        log::error!(
            "PANIC on thread '{}' at {}: {}",
            thread_name,
            location,
            message
        );
        previous(info);
    }));
}

/// Tells every webview which OS it runs on (`window.__SCRIBE_PLATFORM__`:
/// "windows", "macos" or "linux") before any page script runs, so the UI can
/// word platform-specific copy without an async round trip or a flash of the
/// Windows wording.
fn platform_script<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("scribe-platform")
        .js_init_script(format!(
            "window.__SCRIBE_PLATFORM__ = {:?};",
            std::env::consts::OS
        ))
        .build()
}

/// The state the commands read with `app.state::<T>()`, which panics when the
/// state is not managed yet. Names are for the startup error only.
fn missing_command_state(app: &tauri::App) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if app.try_state::<BackendState>().is_none() {
        missing.push("BackendState");
    }
    if app.try_state::<whisper_server::WarmTranscriber>().is_none() {
        missing.push("WarmTranscriber");
    }
    if app
        .try_state::<std::sync::Arc<replacements_file::ReplacementStore>>()
        .is_none()
    {
        missing.push("ReplacementStore");
    }
    if app
        .try_state::<dictation_worker::DictationWorker>()
        .is_none()
    {
        missing.push("DictationWorker");
    }
    if app.try_state::<note_sync::NoteSyncWorker>().is_none() {
        missing.push("NoteSyncWorker");
    }
    if app.try_state::<Core>().is_none() {
        missing.push("Core");
    }
    missing
}

/// Builds the windows declared in tauri.conf.json.
///
/// Tauri builds config windows marked `create` (the default) before it runs
/// the setup closure, so their webviews load and invoke commands while setup
/// is still opening the database; any command that reads the backend state
/// then panicked with "state() called before manage()". The config marks every
/// window `"create": false` and setup calls this last, once all state is
/// managed, so no command can ever run ahead of the state it needs.
fn create_config_windows(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let missing = missing_command_state(app);
    if !missing.is_empty() {
        return Err(format!(
            "windows created before their state was managed: {}",
            missing.join(", ")
        )
        .into());
    }
    // Started by the sign-in entry: Scribe waits in the tray, window hidden.
    let at_sign_in = autostart::launched_at_sign_in(&std::env::args().collect::<Vec<_>>());
    for window_config in app.config().app.windows.iter().filter(|w| !w.create) {
        if app.get_webview_window(&window_config.label).is_none() {
            let mut builder =
                tauri::WebviewWindowBuilder::from_config(app.handle(), window_config)?;
            if at_sign_in && window_config.label == "main" {
                builder = builder.visible(false);
            }
            builder.build()?;
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_logger();
    tauri::Builder::default()
        // The single-instance plugin must be the first plugin registered so
        // it can take over before anything else initializes.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // `Scribe toggle` and friends, bound in the desktop's keyboard
            // settings where Scribe can't register hotkeys (Wayland without
            // the GlobalShortcuts portal).
            if let Some(command) = remote_command::RemoteCommand::parse(&args) {
                // A launch that lands while setup still runs (the callback
                // can come off the main thread) must not reach state that
                // isn't managed yet.
                if app
                    .try_state::<dictation_worker::DictationWorker>()
                    .is_none()
                {
                    log::warn!("Scribe is still starting; ignored {:?}", command);
                    return;
                }
                command.run(app);
                return;
            }
            // The sign-in entry firing while Scribe already runs (started by
            // hand during sign-in) must not pop the window open.
            if autostart::launched_at_sign_in(&args) {
                log::info!("Sign-in launch while already running; staying in the tray");
                return;
            }
            log::info!("Second instance launch detected; focusing the main window");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .level_for("app_lib", log::LevelFilter::Debug)
                .level_for("scribe_core", log::LevelFilter::Debug)
                // The plugin defaults (40 KB max, KeepOne) self-delete the log
                // every ~40 KB, so under normal dictation the file churns past
                // a few sessions within minutes and nothing older survives —
                // useless when a user reports "it failed an hour ago." Keep a
                // few MB across several rotated files, and stamp timestamps in
                // the user's local time so log lines line up with the clock
                // they saw the problem on.
                .max_file_size(5_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    hotkeys::handle_shortcut(app, shortcut, event);
                })
                .build(),
        )
        .plugin(autostart::plugin())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(platform_script())
        .setup(|app| {
            // The engine resolves its folders without Tauri; they must be the
            // ones Tauri resolves (checked in debug builds and in a test).
            let paths = tauri_host::resolve_paths(app)?;
            debug_assert_eq!(
                tauri_host::paths_mismatches(app, &paths),
                Vec::<String>::new(),
                "the engine's folders differ from Tauri's"
            );
            // Before anything can run engine code that touches the tray or a
            // window, an event or a folder.
            let host: std::sync::Arc<dyn host::Host> =
                std::sync::Arc::new(tauri_host::TauriHost::new(app.handle().clone(), paths));
            shell::install(host.clone());
            let app_data_dir = app.path().app_data_dir()?;
            let audio_temp_dir = app.path().app_cache_dir()?.join("recordings");
            std::fs::create_dir_all(&app_data_dir)?;
            std::fs::create_dir_all(&audio_temp_dir)?;
            // Carry forward data from the pre-rebrand identifier so existing
            // users keep their DB/clips/models after the LocalDictate->Scribe
            // rename. Best-effort; never blocks startup.
            migrate_pre_rebrand_data(app.handle(), &app_data_dir);
            let db = Database::open(app_data_dir.join("scribe.sqlite3"))?;
            let mut settings = db.get_settings()?;
            let mut settings_migrated = false;
            // One-time migration: replace the old default hotkeys (which
            // collide with Windows shortcuts like Ctrl+Win+Space) with the
            // current defaults on installs that never customized them.
            if settings.hotkeys.migrate_legacy_defaults() {
                log::info!("Migrated legacy default hotkeys to the current defaults");
                settings_migrated = true;
            }
            // One-time migration of changed shipped defaults (e.g. the
            // default output mode moving from SaveOnly to AutoPaste).
            if settings.migrate_defaults() {
                log::info!(
                    "Migrated settings defaults to version {} (output mode now {:?})",
                    settings.defaults_version,
                    settings.output_mode
                );
                settings_migrated = true;
            }
            // The Scribe Dev flavor seeds non-conflicting hotkey defaults the
            // first time it runs, so it can run alongside stable Scribe without
            // fighting over the same global binds. Only untouched (still
            // production) binds are remapped; the flag makes this one-shot so a
            // later "Load my production defaults" sticks.
            if is_dev_flavor(app.handle()) && !settings.dev_hotkeys_seeded {
                if settings.hotkeys == crate::settings::HotkeySettings::default() {
                    settings.hotkeys = crate::settings::HotkeySettings::dev_defaults();
                    log::info!("Seeded Scribe Dev hotkey defaults");
                }
                // A test build starts only when asked: never at sign-in by
                // default, and never on the Windows test runners.
                settings.launch_at_startup = false;
                settings.dev_hotkeys_seeded = true;
                settings_migrated = true;
            }
            if settings_migrated {
                db.save_settings(&settings)?;
            }
            log::info!(
                "Settings loaded (defaults version {}, output mode {:?}, model {:?})",
                settings.defaults_version,
                settings.output_mode,
                settings.selected_model_id
            );
            db.enforce_history_retention(settings.history_retention_days)?;
            db.enforce_notes_retention(settings.notes_retention_days)?;
            // Bring the sign-in entry and "Launch at startup" into step: repair
            // an entry an older install left pointing at a missing program,
            // write a missing one, and follow the switch in Windows' Startup
            // apps list when the user changed it there (see autostart.rs).
            if let Some(value) =
                autostart::reconcile_at_launch(app.handle(), settings.launch_at_startup)
            {
                settings.launch_at_startup = value;
                db.save_settings(&settings)?;
            }
            let dirs = data_hygiene::DataDirs {
                data_dir: app_data_dir.clone(),
                cache_dir: app.path().app_cache_dir()?,
            };
            // Each part of the engine is managed on its own (the commands
            // and the engine still look them up by type) and handed to the
            // Core below, which holds the very same instance.
            let backend = BackendState::new(db, audio_temp_dir, dirs);
            app.manage(backend.clone());
            // Folder cleanup (pre-rename paths, orphaned clips, stale temp
            // files) touches the disk, so it runs off the startup path.
            {
                let handle = app.handle().clone();
                let pre_rebrand_dir = pre_rebrand_data_dir(&handle, &app_data_dir);
                let migration_complete = app_data_dir.join(".rebrand-migrated").exists();
                std::thread::Builder::new()
                    .name("data-cleanup".into())
                    .spawn(move || {
                        data_hygiene::run_startup_maintenance(
                            &handle,
                            pre_rebrand_dir,
                            migration_complete,
                        )
                    })?;
            }
            // Stop any speech engine an earlier Scribe left running (a crash
            // or a forced kill on a system without a kernel-level guard).
            let stale =
                child_guard::init(app.path().app_cache_dir()?.join("speech-processes.json"));
            if stale > 0 {
                log::warn!(
                    "Stopped {} speech engine process(es) left by an earlier run",
                    stale
                );
            }
            let warm_transcriber = whisper_server::WarmTranscriber::new(
                app.handle().clone(),
                speech_engine::measurements_path(&app_data_dir),
            );
            app.manage(warm_transcriber.clone());
            // Offer Scribe's live dictation state to external tools over an
            // in-process loopback HTTP+SSE server. Additive and best-effort: a
            // bind failure must never block the app, so log and carry on.
            let dictation_server = match state_server::start(app.handle()) {
                Ok(server) => {
                    log::info!("Dictation-state interface on {}", server.base_url());
                    app.manage(server.clone());
                    Some(server)
                }
                Err(error) => {
                    log::warn!("Could not start dictation-state server: {}", error);
                    None
                }
            };
            // Background worker that debounces note saves into GitHub auto-syncs.
            let note_sync = note_sync::NoteSyncWorker::spawn(app.handle().clone());
            app.manage(note_sync.clone());
            // Every dictation action (hotkeys, tray, pill, dashboard) runs on
            // this one worker thread, so it must exist before any of them.
            let dictation_worker = dictation_worker::DictationWorker::spawn(app.handle().clone())?;
            app.manage(dictation_worker.clone());
            // The Wayland portals need Scribe's app id and a place to keep
            // the permission they grant, before the hotkeys start.
            #[cfg(target_os = "linux")]
            platform::init(app_data_dir.clone(), &app.config().identifier);
            hotkeys::setup(
                app.handle(),
                &settings.hotkeys,
                settings.notes_analysis_enabled,
            )?;
            // hotkeys::setup manages the hotkey runtime itself.
            let hotkey_runtime = app.state::<hotkeys::HotkeyRuntimeState>().inner().clone();
            tray::setup(app.handle(), &settings)?;
            // Replacements live in replacements.txt in the data folder; the
            // first run moves the list kept in settings there. A watcher picks
            // up edits made in any editor.
            let replacements = {
                use tauri::Emitter;
                let store = std::sync::Arc::new(replacements_file::ReplacementStore::open(
                    app_data_dir.join(replacements_file::FILE_NAME),
                    &settings.text_replacements,
                ));
                let watch_app = app.handle().clone();
                store.watch(move |info| {
                    log::info!(
                        "Reloaded replacements.txt: {} replacements, {} lines skipped",
                        info.rows.len(),
                        info.skipped_lines.len()
                    );
                    let _ = watch_app.emit(commands::REPLACEMENTS_CHANGED_EVENT, info);
                    // Back the edited file up with the notes, when backup is on.
                    if let Some(worker) = watch_app.try_state::<note_sync::NoteSyncWorker>() {
                        worker.notify();
                    }
                });
                app.manage(store.clone());
                store
            };
            // The engine as one handle, holding the parts managed above.
            let core = Core::new(
                host,
                scribe_core::core::CoreParts {
                    backend,
                    warm_transcriber,
                    dictation_server,
                    note_sync,
                    dictation_worker,
                    hotkey_runtime,
                    replacements,
                },
            );
            app.manage(core);
            // Only now, with every piece of state managed, may a webview
            // exist and invoke commands (see create_config_windows).
            create_config_windows(app)?;
            // Open at the saved size (or the default), shrunk to fit the screen.
            if let Some(window) = app.get_webview_window("main") {
                window_size::restore(&window, &settings);
            }
            // The dev flavor must be tellable from stable at a glance: the
            // window title (and taskbar entry) carries the flavored name.
            if is_dev_flavor(app.handle()) {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_title("Scribe Dev");
                }
            }
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                style_native_titlebar(&window);
            }

            // Builds made for the end-to-end tests can record a WAV file in
            // place of the microphone (see test_audio for the gates).
            #[cfg(feature = "e2e-audio")]
            test_audio::activate(app.handle());

            // Seed the on-disk status file with the startup (Idle) state so a
            // second app (T-Hub) always has a file to read, even before the
            // first dictation.
            status_file::publish_idle(app.handle());

            // `--self-test-transcribe <audio> <result>` (the release smoke
            // test) transcribes one file through the normal path, writes the
            // outcome and exits. Plain launches never take this branch.
            if let Some(request) = self_test::parse_args(std::env::args()) {
                log::info!("Starting self-test run");
                self_test::spawn(app.handle(), request);
            }

            // Load the speech model now when "Keep the speech model loaded" is
            // on, so the first dictation is instant; publish its status.
            app.state::<whisper_server::WarmTranscriber>()
                .sync_with_settings(&settings);

            log::info!("Scribe setup complete");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Wayland hides which app is in front; Scribe still knows when
            // one of its own windows is, so a paste never lands in Scribe.
            #[cfg(target_os = "linux")]
            if let tauri::WindowEvent::Focused(focused) = event {
                platform::note_window_focus(window.label(), *focused);
            }
            if window.label() != "main" {
                return;
            }

            // Coming back to Scribe after switching it in Windows' Startup
            // apps list shows the new value in Settings and the tray.
            if let tauri::WindowEvent::Focused(true) = event {
                let app = window.app_handle().clone();
                if cfg!(windows) && app.try_state::<BackendState>().is_some() {
                    let _ = std::thread::Builder::new()
                        .name("launch-at-startup".into())
                        .spawn(move || {
                            if let Err(error) = commands::sync_launch_at_startup(&app) {
                                log::warn!(
                                    "Could not follow Windows' Startup apps switch. {}",
                                    error.message
                                );
                            }
                        });
                }
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray = window
                    .app_handle()
                    .try_state::<BackendState>()
                    .and_then(|state| {
                        state
                            .db()
                            .ok()
                            .and_then(|db| db.get_settings().ok())
                            .map(|settings| settings.minimize_to_tray)
                    })
                    .unwrap_or(true);

                if minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            // status
            commands::status::get_app_state,
            // audio
            commands::audio::list_microphones,
            commands::audio::start_recording,
            commands::audio::stop_dictation,
            commands::audio::cancel_recording,
            commands::audio::record_test_clip,
            commands::audio::get_test_clip_audio,
            commands::audio::transcribe_file,
            commands::audio::save_text_file,
            commands::audio::paste_last_transcript,
            // output
            commands::output::get_paste_permission,
            commands::output::request_paste_permission,
            commands::output::copy_last_transcript,
            commands::output::paste_transcript,
            commands::output::copy_transcript,
            commands::output::transform_selection,
            // models
            commands::models::open_models_folder,
            commands::models::list_models,
            commands::models::download_model,
            commands::models::cancel_model_download,
            commands::models::retry_model_download,
            commands::models::delete_model,
            commands::models::select_model,
            commands::models::get_engine_status,
            commands::models::probe_gpu_devices,
            // hotkeys
            commands::hotkeys::get_hotkey_status,
            commands::hotkeys::bind_portal_shortcuts,
            commands::hotkeys::configure_portal_shortcuts,
            commands::hotkeys::rebind_hotkey,
            commands::hotkeys::set_hotkey_trigger,
            commands::hotkeys::reset_hotkeys_to_defaults,
            commands::hotkeys::load_production_hotkey_defaults,
            // data
            commands::data::get_settings,
            commands::data::patch_settings,
            commands::data::get_last_transcript,
            commands::data::clear_last_transcript,
            commands::data::list_recent_transcripts,
            commands::data::search_transcripts,
            commands::data::get_transcript,
            commands::data::update_transcript,
            commands::data::delete_transcript,
            commands::data::clear_transcript_history,
            commands::data::clear_notes,
            commands::data::combine_transcripts,
            commands::data::save_combined_transcript,
            commands::data::open_transcript_externally,
            commands::data::get_transcript_audio,
            commands::data::get_basic_stats,
            commands::data::refresh_basic_stats,
            commands::data::open_data_folder,
            commands::data::get_data_dir,
            commands::data::pick_data_dir,
            commands::data::open_logs_folder,
            commands::data::get_logs_dir,
            commands::data::open_failed_recordings_folder,
            commands::data::get_failed_recordings_dir,
            commands::data::export_transcripts,
            commands::data::preview_transcript_import,
            commands::data::restore_transcript_import,
            commands::data::get_replacements,
            commands::data::add_replacements,
            commands::data::remove_replacement,
            commands::data::open_replacements_file,
            commands::data::whats_new_pending,
            commands::data::dismiss_whats_new,
            commands::data::collect_diagnostics,
            // sync
            commands::sync::analyze_note,
            commands::sync::llm_status,
            commands::sync::github_status,
            commands::sync::github_device_start,
            commands::sync::github_device_poll,
            commands::sync::github_device_cancel,
            commands::sync::github_disconnect,
            commands::sync::github_sync_now,
            commands::sync::github_sync_activity,
            // shell
            commands::shell::open_dashboard,
            commands::shell::save_window_size,
            commands::shell::check_for_update,
            commands::shell::open_release_page
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // Tear down the resident whisper-server process so no orphan
            // survives app quit (shutdown is safe to call multiple times).
            if let tauri::RunEvent::Exit = event {
                // Leave the status file reflecting a not-listening app so a
                // reader isn't stranded on a stale Recording/Stopping state.
                status_file::publish_idle(app);
                // Stop the dictation-state server, close live SSE streams, and
                // remove the ~/.scribe discovery file so a consumer sees Scribe
                // is gone (stream EOF and a missing control file both resolve to
                // not-dictating).
                if let Some(server) = app.try_state::<state_server::DictationServer>() {
                    server.shutdown();
                }
                if let Some(warm) = app.try_state::<whisper_server::WarmTranscriber>() {
                    warm.shutdown();
                }
            }
        });
}

/// Colors the native Windows title bar to match the app background so the
/// window top blends into the dashboard instead of sitting as a gray strip.
#[cfg(windows)]
fn style_native_titlebar(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };

    let Ok(handle) = window.hwnd() else {
        return;
    };
    let hwnd = HWND(handle.0);

    // App background #070b14 as COLORREF (0x00BBGGRR).
    let caption_color: u32 = 0x0014_0B07;
    let dark_mode: i32 = 1;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption_color as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(test)]
mod install_identity_tests {
    /// An existing install updates in place only while the app identifier
    /// (data folder), product name (install folder, uninstall entry, MSI
    /// upgrade code) and binary name stay the same. The sign-in entry is
    /// named after the product too.
    #[test]
    fn the_install_identity_is_stable() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], "com.natkins.scribe");
        assert_eq!(config["productName"], "Scribe");
        assert_eq!(config["mainBinaryName"], "Scribe");
    }

    /// The engine reports its own CARGO_PKG_VERSION (the update check, user
    /// agents, the dictation-state interface), so it must carry the app's.
    #[test]
    fn the_engine_carries_the_app_version() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            include_str!("../../../crates/scribe-core/Cargo.toml")
                .lines()
                .find_map(|line| line.strip_prefix("version = "))
                .map(|version| version.trim_matches('"')),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    /// The installer records the install folder under
    /// `HKCU\Software\<publisher>\Scribe`, and Windows shows the publisher as
    /// the exe's company and the installer's publisher. The running app writes
    /// the same key (`autostart::PUBLISHER`) and the installer hooks read
    /// and clear the old one, so all three must agree.
    #[test]
    fn the_publisher_matches_the_install_folder_key() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["bundle"]["publisher"], "n8builds.dev");
        assert_eq!(config["bundle"]["publisher"], crate::autostart::PUBLISHER);
        let hooks = include_str!("../installer-hooks.nsh");
        let old = format!(
            "!define SCRIBE_OLD_MANUFACTURER \"{}\"",
            crate::autostart::PREVIOUS_PUBLISHER
        );
        assert!(
            hooks.contains(&old),
            "installer hook must clear the old publisher key"
        );
    }
}

#[cfg(test)]
mod command_list_tests {
    /// Every command the windows invoke, by the name they invoke it with, as
    /// registered on main before the commands moved into
    /// src/commands/<domain>.rs. The windows call commands by name, so the
    /// registered set must stay exactly this.
    const COMMANDS: [&str; 79] = [
        "add_replacements",
        "analyze_note",
        "bind_portal_shortcuts",
        "cancel_model_download",
        "cancel_recording",
        "check_for_update",
        "clear_last_transcript",
        "clear_notes",
        "clear_transcript_history",
        "collect_diagnostics",
        "combine_transcripts",
        "configure_portal_shortcuts",
        "copy_last_transcript",
        "copy_transcript",
        "delete_model",
        "delete_transcript",
        "dismiss_whats_new",
        "download_model",
        "export_transcripts",
        "get_app_state",
        "get_basic_stats",
        "get_data_dir",
        "get_engine_status",
        "get_failed_recordings_dir",
        "get_hotkey_status",
        "get_last_transcript",
        "get_logs_dir",
        "get_paste_permission",
        "get_replacements",
        "get_settings",
        "get_test_clip_audio",
        "get_transcript",
        "get_transcript_audio",
        "github_device_cancel",
        "github_device_poll",
        "github_device_start",
        "github_disconnect",
        "github_status",
        "github_sync_activity",
        "github_sync_now",
        "list_microphones",
        "list_models",
        "list_recent_transcripts",
        "llm_status",
        "load_production_hotkey_defaults",
        "open_dashboard",
        "open_data_folder",
        "open_failed_recordings_folder",
        "open_logs_folder",
        "open_models_folder",
        "open_release_page",
        "open_replacements_file",
        "open_transcript_externally",
        "paste_last_transcript",
        "paste_transcript",
        "patch_settings",
        "pick_data_dir",
        "preview_transcript_import",
        "probe_gpu_devices",
        "rebind_hotkey",
        "record_test_clip",
        "refresh_basic_stats",
        "remove_replacement",
        "request_paste_permission",
        "reset_hotkeys_to_defaults",
        "restore_transcript_import",
        "retry_model_download",
        "save_combined_transcript",
        "save_text_file",
        "save_window_size",
        "search_transcripts",
        "select_model",
        "set_hotkey_trigger",
        "start_recording",
        "stop_dictation",
        "transcribe_file",
        "transform_selection",
        "update_transcript",
        "whats_new_pending",
    ];

    /// The paths listed in `generate_handler!`, in order.
    fn registered() -> Vec<String> {
        let source = include_str!("lib.rs");
        let start = source.find("generate_handler![").unwrap() + "generate_handler![".len();
        let end = start + source[start..].find("])").unwrap();
        source[start..end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//"))
            .map(|line| line.trim_end_matches(',').to_string())
            .collect()
    }

    #[test]
    fn every_command_keeps_its_name() {
        let registered = registered();
        let mut names: Vec<&str> = registered
            .iter()
            .map(|path| path.rsplit("::").next().unwrap())
            .collect();
        names.sort_unstable();
        let mut expected = COMMANDS.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    /// Each command is registered from the file of its domain
    /// (`commands::<domain>::<name>`), and each of those files exists.
    #[test]
    fn every_command_comes_from_a_domain_file() {
        let domains = [
            ("audio", include_str!("commands/audio.rs")),
            ("output", include_str!("commands/output.rs")),
            ("models", include_str!("commands/models.rs")),
            ("hotkeys", include_str!("commands/hotkeys.rs")),
            ("data", include_str!("commands/data.rs")),
            ("sync", include_str!("commands/sync.rs")),
            ("status", include_str!("commands/status.rs")),
            ("shell", include_str!("commands/shell.rs")),
        ];
        let mut defined = 0;
        for (_, source) in domains {
            defined += source.matches("#[tauri::command]").count();
        }
        assert_eq!(
            defined,
            COMMANDS.len(),
            "a command is defined twice or missing"
        );
        for path in registered() {
            let parts: Vec<&str> = path.split("::").collect();
            assert_eq!(parts.len(), 3, "{path} is not commands::<domain>::<name>");
            assert_eq!(parts[0], "commands");
            let (_, source) = domains
                .iter()
                .find(|(domain, _)| *domain == parts[1])
                .unwrap_or_else(|| panic!("{path} names no domain file"));
            assert!(
                source.contains(&format!("fn {}(", parts[2])),
                "{path} is not defined in commands/{}.rs",
                parts[1]
            );
        }
    }
}

#[cfg(test)]
mod startup_order_tests {
    /// Tauri builds every config window marked `create` (the default) before
    /// it runs the setup closure, so the webview loads and invokes commands
    /// while setup is still opening the database. Commands that read
    /// `BackendState` then panic with "state() called before manage()". Every
    /// window must therefore be left for setup to create, after the state.
    #[test]
    fn config_windows_are_left_for_setup_to_create() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = config["app"]["windows"].as_array().unwrap();
        assert!(!windows.is_empty());
        for window in windows {
            assert_eq!(
                window["create"],
                serde_json::Value::Bool(false),
                "window {} is created before setup manages the backend state",
                window["label"]
            );
        }
    }

    /// Setup creates the windows only after the last `manage()` call, so no
    /// command can run before the state it reads exists.
    #[test]
    fn setup_creates_windows_after_managing_state() {
        let source = include_str!("lib.rs");
        let start = source.find(".setup(|app| {").unwrap();
        let end = start + source[start..].find(".on_window_event(").unwrap();
        let setup = &source[start..end];
        let last_manage = setup.rfind("app.manage(").unwrap();
        let create = setup
            .find("create_config_windows(app)")
            .expect("setup must create the config windows");
        assert!(
            create > last_manage,
            "setup creates the windows before it manages all of its state"
        );
    }

    /// The engine starts in a fixed order: the host first (the engine's
    /// tray, window and event calls go through it), then each part in the
    /// order it was started before the Core existed, then the Core, which
    /// holds them all, and only then the windows. A later part may rely on an
    /// earlier one (the dictation worker before the hotkeys that queue onto
    /// it, the backend before everything that reads it).
    #[test]
    fn setup_starts_the_engine_in_order() {
        let source = include_str!("lib.rs");
        let start = source.find(".setup(|app| {").unwrap();
        let end = start + source[start..].find(".on_window_event(").unwrap();
        let setup = &source[start..end];
        let steps = [
            "shell::install(host.clone())",
            "Database::open(",
            "app.manage(backend.clone())",
            "child_guard::init(",
            "app.manage(warm_transcriber.clone())",
            "app.manage(server.clone())",
            "app.manage(note_sync.clone())",
            "app.manage(dictation_worker.clone())",
            "hotkeys::setup(",
            "tray::setup(",
            "app.manage(store.clone())",
            "app.manage(core)",
            "create_config_windows(app)",
        ];
        let mut previous = 0;
        for step in steps {
            let at = setup
                .find(step)
                .unwrap_or_else(|| panic!("setup no longer has `{step}`"));
            assert_eq!(setup.matches(step).count(), 1, "`{step}` appears twice");
            assert!(at > previous, "`{step}` runs out of order");
            previous = at;
        }
        // Every part the Core holds is managed on its own too, so the
        // existing `app.state::<X>()` lookups keep finding it.
        assert_eq!(setup.matches("app.manage(").count(), 7);
    }
}
