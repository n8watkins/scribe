//! Consistent settings writes across windows.
//!
//! The dashboard and the pill each keep a copy of the settings. They used to
//! save the whole object, so the last writer won with whatever stale copy it
//! had: moving the pill could put back a hotkey the user had just changed.
//!
//! Now a window sends only the fields it changed ([`patch_locked`] merges them
//! over the stored settings), every settings write in the backend holds one
//! write lock from read to save, and every write is announced with
//! [`SETTINGS_CHANGED_EVENT`] so each window replaces its copy with the stored
//! settings.

use std::sync::{Mutex, MutexGuard};

use serde_json::{Map, Value};

use crate::{
    backend::BackendState,
    core::Core,
    error::CommandError,
    events::CoreEvent,
    host::Host,
    hotkeys::{self, HotkeyRegistrationFailure},
    settings::{AppSettings, HotkeySettings},
};

/// Emitted with the full stored settings after every settings write.
pub const SETTINGS_CHANGED_EVENT: &str = "scribe:settings-changed";

/// Takes the settings write lock. Held from reading the stored settings to
/// saving the new ones (and announcing them), so no write is lost and the
/// events go out in the order the writes happened. The guarded value is `()`,
/// so a lock poisoned by a panic carries no broken state and is recovered.
///
/// Never take it on the main thread. The lock is held while the host is
/// called (the announcement updates the tray menu, and a settings change
/// registers hotkeys), and a host may wait for its main thread there (Tauri
/// does), so a main-thread caller could wait on itself. Debug builds assert
/// this; every caller runs on an engine thread (the async commands through
/// `off_main_core`, the tray and window-focus handlers on
/// their own `launch-at-startup` threads).
pub fn lock(writes: &Mutex<()>) -> MutexGuard<'_, ()> {
    debug_assert!(
        !on_main_thread(),
        "the settings write lock must never be taken on the main thread"
    );
    writes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether this is the process's main thread, the one that runs `main` and
/// with it the shell's event loop. Rust's runtime names that thread "main",
/// whatever the shell, and Scribe gives no other thread that name.
fn on_main_thread() -> bool {
    std::thread::current().name() == Some("main")
}

/// Tells every window, and the tray menu, the stored settings changed.
pub fn emit_changed_with(host: &dyn Host, settings: &AppSettings) {
    host.emit(CoreEvent::SettingsChanged(settings.clone()));
    // The tray menu's "Launch at startup" check mark shows the stored value.
    host.sync_settings(settings);
}

/// Returns `current` with the fields named in `patch` replaced. Keys are the
/// camelCase names the frontend uses; a key that is not a setting is rejected
/// rather than silently dropped.
pub fn merge_patch(
    current: &AppSettings,
    patch: &Map<String, Value>,
) -> Result<AppSettings, CommandError> {
    let Value::Object(mut fields) = serde_json::to_value(current)? else {
        return Err(CommandError::invalid_settings(
            "Stored settings are not an object.",
        ));
    };
    for (key, value) in patch {
        if !fields.contains_key(key) {
            return Err(CommandError::invalid_settings(format!(
                "\"{}\" is not a setting.",
                key
            )));
        }
        fields.insert(key.clone(), value.clone());
    }
    serde_json::from_value(Value::Object(fields)).map_err(CommandError::invalid_settings)
}

/// Applies `patch` under the write lock: reads the stored settings with
/// `read`, merges the patch over them, and hands the previous and new settings
/// to `commit`, which saves them (and applies their side effects). A patch that
/// changes nothing returns the stored settings without calling `commit`.
pub fn patch_locked<R, C>(
    writes: &Mutex<()>,
    patch: &Map<String, Value>,
    read: R,
    commit: C,
) -> Result<AppSettings, CommandError>
where
    R: FnOnce() -> Result<AppSettings, CommandError>,
    C: FnOnce(&AppSettings, AppSettings) -> Result<AppSettings, CommandError>,
{
    let _guard = lock(writes);
    let previous = read()?;
    let next = merge_patch(&previous, patch)?;
    if next == previous {
        return Ok(previous);
    }
    commit(&previous, next)
}

/// Applies a settings patch from the backend (the tray, Windows' Startup apps
/// switch) exactly as a window's patch is applied. Blocks; call it off the
/// main thread.
pub fn patch_settings_now_with(
    core: &Core,
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, CommandError> {
    let state = core.backend();
    patch_locked(
        state.settings_writes(),
        patch,
        || state.db()?.get_settings(),
        |previous, next| apply_settings_change_with(core, previous, next),
    )
}

/// Shows the switch the user may have flipped in Windows' Startup apps list
/// while Scribe was running: copies it into "Launch at startup". Never writes
/// the startup entry. Holds the settings write lock from read to save, so it
/// cannot undo a flip the user is making in Scribe at the same moment.
/// Blocks; call it off the main thread.
pub fn sync_launch_at_startup_with(core: &Core) -> Result<AppSettings, CommandError> {
    sync_launch_at_startup_in(core.backend(), core.host().as_ref())
}

/// The body of [`sync_launch_at_startup_with`], over the parts it uses: the
/// host answers Windows' switch.
fn sync_launch_at_startup_in(
    state: &BackendState,
    host: &dyn Host,
) -> Result<AppSettings, CommandError> {
    follow_launch_switch(state, host, |stored| {
        crate::autostart::follow_windows_switch_with(host, stored)
    })
}

/// The body of [`sync_launch_at_startup_in`]: `follow` gets the stored
/// "Launch at startup" and answers the value to store instead, or None to
/// leave it.
fn follow_launch_switch(
    state: &BackendState,
    host: &dyn Host,
    follow: impl FnOnce(bool) -> Option<bool>,
) -> Result<AppSettings, CommandError> {
    let _guard = lock(state.settings_writes());
    let mut settings = state.db()?.get_settings()?;
    if let Some(value) = follow(settings.launch_at_startup) {
        log::info!("Launch at startup follows Windows' Startup apps switch: {value}");
        settings.launch_at_startup = value;
        state.db()?.save_settings(&settings)?;
        emit_changed_with(host, &settings);
    }
    Ok(settings)
}

/// Turns "Launch at startup" on or off, as the settings toggle does.
pub fn set_launch_at_startup_with(core: &Core, enabled: bool) -> Result<AppSettings, CommandError> {
    let mut patch = serde_json::Map::new();
    patch.insert("launchAtStartup".into(), serde_json::Value::Bool(enabled));
    patch_settings_now_with(core, &patch)
}

/// Stores `settings` over `previous` and applies what changed: launch at
/// startup, hotkey registration, GitHub activity, and retention. Rolls the
/// side effects back when the save fails. The caller holds the settings write
/// lock.
fn apply_settings_change_with(
    core: &Core,
    previous: &AppSettings,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    apply_settings_change_in(
        core.backend(),
        core.host().as_ref(),
        &AppEffects { core },
        previous,
        settings,
    )
}

/// What a settings change does outside the database and the windows: the
/// startup entry, the hotkeys and the speech engine. [`AppEffects`] is the
/// real one; tests record the calls.
trait SettingsEffects {
    fn apply_autostart(&self, enabled: bool) -> Result<(), CommandError>;
    fn set_transform_enabled(&self, enabled: bool);
    fn replace_hotkeys(
        &self,
        previous: &HotkeySettings,
        next: &HotkeySettings,
    ) -> Result<Vec<HotkeyRegistrationFailure>, CommandError>;
    fn sync_speech_engine(&self, settings: &AppSettings);
    fn report_hotkey_failures(&self, failures: &[HotkeyRegistrationFailure]);
}

/// The settings side effects in the running app.
struct AppEffects<'a> {
    core: &'a Core,
}

impl SettingsEffects for AppEffects<'_> {
    fn apply_autostart(&self, enabled: bool) -> Result<(), CommandError> {
        crate::autostart::apply_with(self.core.host().as_ref(), enabled)
    }

    fn set_transform_enabled(&self, enabled: bool) {
        hotkeys::set_transform_enabled_with(self.core, enabled);
    }

    fn replace_hotkeys(
        &self,
        previous: &HotkeySettings,
        next: &HotkeySettings,
    ) -> Result<Vec<HotkeyRegistrationFailure>, CommandError> {
        hotkeys::replace_hotkeys_with(self.core, previous, next)
    }

    fn sync_speech_engine(&self, settings: &AppSettings) {
        // Load, reload or unload the speech model to match (model, language,
        // GPU and "Keep the speech model loaded").
        self.core.warm_transcriber().sync_with_settings(settings);
    }

    fn report_hotkey_failures(&self, failures: &[HotkeyRegistrationFailure]) {
        hotkeys::emit_registration_failures_with(self.core.host().as_ref(), failures);
    }
}

/// The body of [`apply_settings_change_with`], over the parts it uses.
fn apply_settings_change_in(
    state: &BackendState,
    host: &dyn Host,
    effects: &dyn SettingsEffects,
    previous: &AppSettings,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    settings
        .validate()
        .map_err(CommandError::invalid_settings)?;

    // Apply launch-at-startup first: when the OS registration fails the new
    // value is never saved, so the UI toggle reverts.
    if previous.launch_at_startup != settings.launch_at_startup {
        effects.apply_autostart(settings.launch_at_startup)?;
    }

    // Transform Selection is only bound while Local AI is on, so turning it on
    // or off re-registers the hotkey set too.
    let transform_toggled = previous.notes_analysis_enabled != settings.notes_analysis_enabled;
    let rebind = previous.hotkeys != settings.hotkeys || transform_toggled;
    let mut hotkey_failures = Vec::new();
    if rebind {
        effects.set_transform_enabled(settings.notes_analysis_enabled);
        hotkey_failures = match effects.replace_hotkeys(&previous.hotkeys, &settings.hotkeys) {
            Ok(failures) => failures,
            Err(error) => {
                effects.set_transform_enabled(previous.notes_analysis_enabled);
                return Err(error);
            }
        };
    }

    if let Err(error) = save_settings_change(state, previous, &settings) {
        if rebind {
            effects.set_transform_enabled(previous.notes_analysis_enabled);
            let _ = effects.replace_hotkeys(&settings.hotkeys, &previous.hotkeys);
        }
        if previous.launch_at_startup != settings.launch_at_startup {
            let _ = effects.apply_autostart(previous.launch_at_startup);
        }

        return Err(error);
    }

    // Every window updates from this event, so it goes out as soon as the
    // new settings are stored.
    emit_changed_with(host, &settings);
    effects.sync_speech_engine(&settings);
    effects.report_hotkey_failures(&hotkey_failures);

    enforce_retention_change(state, previous, &settings)?;

    Ok(settings)
}

/// Saves `settings`, clearing the GitHub activity in the same transaction
/// when they name a different repository than `previous`.
fn save_settings_change(
    state: &BackendState,
    previous: &AppSettings,
    settings: &AppSettings,
) -> Result<(), CommandError> {
    if previous.github_repo != settings.github_repo {
        state
            .db()?
            .save_settings_and_clear_github_activity(settings)
    } else {
        state.db()?.save_settings(settings)
    }
}

/// Applies a changed retention window (or history being turned back on) to
/// what is already stored.
fn enforce_retention_change(
    state: &BackendState,
    previous: &AppSettings,
    settings: &AppSettings,
) -> Result<(), CommandError> {
    if previous.history_retention_days != settings.history_retention_days
        || (!previous.history_enabled && settings.history_enabled)
    {
        state
            .db()?
            .enforce_history_retention(settings.history_retention_days)?;
    }

    if previous.notes_retention_days != settings.notes_retention_days {
        state
            .db()?
            .enforce_notes_retention(settings.notes_retention_days)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_hygiene::DataDirs;
    use crate::db::Database;
    use crate::host::{Flavor, HostCall, Recorded, RecordingHost};
    use crate::paths::Paths;
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn patch(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => panic!("patch must be an object"),
        }
    }

    fn host_in(root: &Path) -> RecordingHost {
        RecordingHost::new(
            Paths::from_parts(
                root.join("data"),
                root.join("cache"),
                root.join("logs"),
                None,
                None,
            ),
            Flavor::Stable,
        )
    }

    fn state_in(root: &Path) -> BackendState {
        BackendState::new(
            Database::in_memory().unwrap(),
            root.join("cache").join("recordings"),
            DataDirs {
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            },
        )
    }

    /// The events a host was sent, as (name, payload) pairs.
    fn sent(host: &RecordingHost) -> Vec<(&'static str, Value)> {
        host.events()
            .iter()
            .map(|event| (event.name(), serde_json::to_value(event).unwrap()))
            .collect()
    }

    #[test]
    fn emit_changed_sends_one_settings_changed_event_with_the_settings() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let settings = AppSettings {
            pill_x: Some(12),
            ..AppSettings::default()
        };
        emit_changed_with(&host, &settings);
        assert_eq!(
            sent(&host),
            [(
                "scribe:settings-changed",
                serde_json::to_value(&settings).unwrap()
            )]
        );
    }

    /// The tray menu's "Launch at startup" tick follows the announcement:
    /// `scribe:settings-changed` first, then `SyncSettings` with the same
    /// settings, and nothing else.
    #[test]
    fn emit_changed_announces_the_settings_then_syncs_the_tray() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let settings = AppSettings {
            pill_x: Some(12),
            launch_at_startup: true,
            ..AppSettings::default()
        };
        emit_changed_with(&host, &settings);
        assert_eq!(
            host.sequence(),
            [
                Recorded::Event(
                    "scribe:settings-changed",
                    serde_json::to_value(&settings).unwrap()
                ),
                Recorded::Call(HostCall::SyncSettings(settings)),
            ]
        );
    }

    /// Runs `work` on a new thread called `name` and waits for it; "main"
    /// stands in for the thread the shell's event loop runs on.
    fn on_a_thread_named<T: Send + 'static>(
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> thread::Result<T> {
        thread::Builder::new()
            .name(name.into())
            .spawn(work)
            .unwrap()
            .join()
    }

    /// The panic message of a failed thread.
    #[cfg(debug_assertions)]
    fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
        panic
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default()
    }

    #[test]
    fn the_settings_lock_is_taken_off_the_main_thread() {
        assert!(!on_main_thread());
        let writes = Arc::new(Mutex::new(()));
        let shared = Arc::clone(&writes);
        on_a_thread_named("launch-at-startup", move || drop(lock(&shared))).unwrap();
        // Nothing was poisoned or left held.
        assert!(writes.try_lock().is_ok());
    }

    /// A synchronous command (or any other main-thread caller) taking the
    /// lock is a bug, caught in debug builds before the lock is taken.
    #[cfg(debug_assertions)]
    #[test]
    fn taking_the_settings_lock_on_the_main_thread_panics_in_debug_builds() {
        let writes = Arc::new(Mutex::new(()));
        let shared = Arc::clone(&writes);
        let panic = on_a_thread_named("main", move || {
            assert!(on_main_thread());
            drop(lock(&shared));
        })
        .unwrap_err();
        assert_eq!(
            panic_text(panic),
            "the settings write lock must never be taken on the main thread"
        );
        // The assertion fires before the lock is taken, so it is not poisoned.
        assert!(writes.try_lock().is_ok());

        // Every settings write takes the lock through `lock`, so each one is
        // caught too.
        let panic = on_a_thread_named("main", || {
            let writes = Mutex::new(());
            let _ = patch_locked(
                &writes,
                &patch(json!({ "pillX": 1 })),
                || Ok(AppSettings::default()),
                |_, next| Ok(next),
            );
        })
        .unwrap_err();
        assert_eq!(
            panic_text(panic),
            "the settings write lock must never be taken on the main thread"
        );
    }

    /// Outside a Windows release build there is no Windows switch to follow
    /// (`follow_windows_switch_with` answers `None` before it reads the
    /// host), so syncing keeps the stored settings, announces nothing and
    /// makes no host call, even when the startup entry differs from them.
    #[cfg(any(debug_assertions, not(windows)))]
    #[test]
    fn syncing_launch_at_startup_off_a_windows_release_build_changes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        host.set_launch_at_login_state(true);
        let state = state_in(root.path());
        let stored = state.db().unwrap().get_settings().unwrap();

        let settings = sync_launch_at_startup_in(&state, &host).unwrap();

        assert_eq!(settings, stored);
        assert_eq!(state.db().unwrap().get_settings().unwrap(), stored);
        assert!(host.events().is_empty());
        assert!(host.calls().is_empty());
    }

    /// `AppEffects::apply_autostart` reaches `autostart::apply_with`, which in
    /// a debug build (every test build) refuses with `autostart_dev_build`
    /// before it calls the host. The change stops there, before the save: the
    /// stored setting is unchanged, no `scribe:settings-changed` goes out, the
    /// tray is not synced and the startup entry is never touched.
    #[cfg(debug_assertions)]
    #[test]
    fn a_refused_startup_entry_leaves_the_setting_unchanged_and_sends_nothing() {
        use crate::core::test_core::TestCore;
        let TestCore {
            core,
            host,
            dir: _dir,
            ..
        } = TestCore::new();
        let stored = core.backend().db().unwrap().get_settings().unwrap();

        let error = set_launch_at_startup_with(&core, !stored.launch_at_startup).unwrap_err();

        assert_eq!(error.code, "autostart_dev_build");
        assert_eq!(core.backend().db().unwrap().get_settings().unwrap(), stored);
        assert!(host.events().is_empty());
        assert!(!host.calls().iter().any(|call| matches!(
            call,
            HostCall::SetLaunchAtLogin(_) | HostCall::SyncSettings(_)
        )));
        core.shutdown();
    }

    #[test]
    fn syncing_launch_at_startup_with_a_broken_database_fails_and_sends_nothing() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_in(root.path());
        let poisoner = state.clone();
        let _ = thread::spawn(move || {
            let _db = poisoner.db().unwrap();
            panic!("poison the database lock");
        })
        .join();

        let error = sync_launch_at_startup_in(&state, &host).unwrap_err();

        assert_eq!(error.code, "database_lock_poisoned");
        assert!(host.events().is_empty());
        assert!(host.calls().is_empty());
    }

    #[test]
    fn following_the_startup_switch_saves_and_announces_once() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_in(root.path());
        let stored = state.db().unwrap().get_settings().unwrap();

        let mut asked = None;
        let settings = follow_launch_switch(&state, &host, |current| {
            asked = Some(current);
            Some(!current)
        })
        .unwrap();
        assert_eq!(asked, Some(stored.launch_at_startup));
        assert_eq!(settings.launch_at_startup, !stored.launch_at_startup);
        assert_eq!(state.db().unwrap().get_settings().unwrap(), settings);
        assert_eq!(
            sent(&host),
            [(
                "scribe:settings-changed",
                serde_json::to_value(&settings).unwrap()
            )]
        );
    }

    #[test]
    fn an_unchanged_startup_switch_keeps_the_settings_and_announces_nothing() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_in(root.path());

        let stored = state.db().unwrap().get_settings().unwrap();

        let settings = follow_launch_switch(&state, &host, |_| None).unwrap();
        assert_eq!(settings, stored);
        assert_eq!(state.db().unwrap().get_settings().unwrap(), stored);
        assert!(host.events().is_empty());
    }

    /// Records every side effect a settings change asks for, in order.
    #[derive(Default)]
    struct FakeEffects {
        calls: std::cell::RefCell<Vec<String>>,
        failures: Vec<HotkeyRegistrationFailure>,
    }

    impl FakeEffects {
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        fn record(&self, call: String) {
            self.calls.borrow_mut().push(call);
        }
    }

    impl SettingsEffects for FakeEffects {
        fn apply_autostart(&self, enabled: bool) -> Result<(), CommandError> {
            self.record(format!("autostart {enabled}"));
            Ok(())
        }

        fn set_transform_enabled(&self, enabled: bool) {
            self.record(format!("transform {enabled}"));
        }

        fn replace_hotkeys(
            &self,
            previous: &HotkeySettings,
            next: &HotkeySettings,
        ) -> Result<Vec<HotkeyRegistrationFailure>, CommandError> {
            self.record(format!(
                "hotkeys {} -> {}",
                previous.paste_last_transcript, next.paste_last_transcript
            ));
            Ok(self.failures.clone())
        }

        fn sync_speech_engine(&self, settings: &AppSettings) {
            self.record(format!("speech engine {:?}", settings.selected_model_id));
        }

        fn report_hotkey_failures(&self, failures: &[HotkeyRegistrationFailure]) {
            self.record(format!("hotkey failures {}", failures.len()));
        }
    }

    /// Settings that flip launch at startup and rebind a hotkey.
    fn startup_and_hotkey_change(previous: &AppSettings) -> AppSettings {
        let mut next = previous.clone();
        next.launch_at_startup = !previous.launch_at_startup;
        next.hotkeys.paste_last_transcript = "Ctrl+Alt+P".into();
        next
    }

    #[test]
    fn a_settings_change_applies_saves_and_announces_once_in_order() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_in(root.path());
        let previous = state.db().unwrap().get_settings().unwrap();
        let next = startup_and_hotkey_change(&previous);
        let effects = FakeEffects::default();

        let saved =
            apply_settings_change_in(&state, &host, &effects, &previous, next.clone()).unwrap();

        assert_eq!(saved, next);
        assert_eq!(state.db().unwrap().get_settings().unwrap(), next);
        assert_eq!(
            effects.calls(),
            [
                format!("autostart {}", next.launch_at_startup),
                format!("transform {}", next.notes_analysis_enabled),
                format!(
                    "hotkeys {} -> Ctrl+Alt+P",
                    previous.hotkeys.paste_last_transcript
                ),
                format!("speech engine {:?}", next.selected_model_id),
                "hotkey failures 0".to_string(),
            ]
        );
        assert_eq!(
            sent(&host),
            [(
                "scribe:settings-changed",
                serde_json::to_value(&next).unwrap()
            )]
        );
    }

    /// When the save fails (here the database lock is poisoned), the
    /// hotkeys and the startup entry go back and nothing is announced.
    #[test]
    fn a_failed_save_rolls_back_the_hotkeys_and_the_startup_entry() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_in(root.path());
        let previous = state.db().unwrap().get_settings().unwrap();
        let next = startup_and_hotkey_change(&previous);
        let poisoner = state.clone();
        let _ = thread::spawn(move || {
            let _db = poisoner.db().unwrap();
            panic!("poison the database lock");
        })
        .join();
        let effects = FakeEffects::default();

        let error =
            apply_settings_change_in(&state, &host, &effects, &previous, next.clone()).unwrap_err();

        assert_eq!(error.code, "database_lock_poisoned");
        assert_eq!(
            effects.calls(),
            [
                format!("autostart {}", next.launch_at_startup),
                format!("transform {}", next.notes_analysis_enabled),
                format!(
                    "hotkeys {} -> Ctrl+Alt+P",
                    previous.hotkeys.paste_last_transcript
                ),
                format!("transform {}", previous.notes_analysis_enabled),
                format!(
                    "hotkeys Ctrl+Alt+P -> {}",
                    previous.hotkeys.paste_last_transcript
                ),
                format!("autostart {}", previous.launch_at_startup),
            ]
        );
        assert!(host.events().is_empty());
    }

    fn github_activity(repo: &str) -> crate::sync_history::GitHubSyncActivity {
        crate::sync_history::GitHubSyncActivity {
            completed_at: chrono::Utc::now(),
            outcome: crate::sync_history::SyncOutcome::Success,
            source: crate::sync_history::SyncSource::Manual,
            repo: repo.into(),
            synced_items: 1,
            files_written: 1,
            error_code: None,
            error_message: None,
        }
    }

    /// A new repository starts with no GitHub activity; any other change
    /// keeps it.
    #[test]
    fn changing_the_repository_clears_the_github_activity() {
        let root = tempfile::tempdir().unwrap();
        let state = state_in(root.path());
        let previous = AppSettings {
            github_repo: "someone/notes".into(),
            ..AppSettings::default()
        };
        state.db().unwrap().save_settings(&previous).unwrap();
        let activity = github_activity("someone/notes");
        state
            .db()
            .unwrap()
            .save_github_sync_activity(&activity)
            .unwrap();

        let moved = AppSettings {
            pill_x: Some(3),
            ..previous.clone()
        };
        save_settings_change(&state, &previous, &moved).unwrap();
        assert_eq!(state.db().unwrap().get_settings().unwrap(), moved);
        assert_eq!(
            state.db().unwrap().get_github_sync_activity().unwrap(),
            Some(activity)
        );

        let other_repo = AppSettings {
            github_repo: "someone/journal".into(),
            ..moved.clone()
        };
        save_settings_change(&state, &moved, &other_repo).unwrap();
        assert_eq!(state.db().unwrap().get_settings().unwrap(), other_repo);
        assert_eq!(
            state.db().unwrap().get_github_sync_activity().unwrap(),
            None
        );
    }

    /// Saves a transcript (a note when `is_note`) made `days` ago.
    fn transcript_made(state: &BackendState, text: &str, is_note: bool, days: i64) -> String {
        let mut transcript =
            crate::transcript::Transcript::new_last_buffer(text, None, None, None).unwrap();
        transcript.is_note = is_note;
        transcript.created_at = chrono::Utc::now() - chrono::Duration::days(days);
        state
            .db()
            .unwrap()
            .save_last_transcript(&transcript)
            .unwrap();
        transcript.id
    }

    fn exists(state: &BackendState, id: &str) -> bool {
        state
            .db()
            .unwrap()
            .get_transcript_by_id(id)
            .unwrap()
            .is_some()
    }

    /// Shortening a retention window deletes what is now past it, at once;
    /// an unchanged window deletes nothing.
    #[test]
    fn a_shorter_retention_window_deletes_what_is_past_it() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_in(root.path());
        let previous = AppSettings {
            history_retention_days: Some(30),
            notes_retention_days: None,
            ..AppSettings::default()
        };
        state.db().unwrap().save_settings(&previous).unwrap();
        let old_dictation = transcript_made(&state, "old dictation", false, 20);
        let new_dictation = transcript_made(&state, "new dictation", false, 2);
        let old_note = transcript_made(&state, "old note", true, 20);
        let new_note = transcript_made(&state, "new note", true, 2);
        let effects = FakeEffects::default();

        // A change that leaves the windows alone deletes nothing.
        let moved = AppSettings {
            pill_x: Some(3),
            ..previous.clone()
        };
        apply_settings_change_in(&state, &host, &effects, &previous, moved.clone()).unwrap();
        for id in [&old_dictation, &new_dictation, &old_note, &new_note] {
            assert!(exists(&state, id));
        }

        let shorter = AppSettings {
            history_retention_days: Some(7),
            ..moved.clone()
        };
        apply_settings_change_in(&state, &host, &effects, &moved, shorter.clone()).unwrap();
        assert!(!exists(&state, &old_dictation));
        assert!(exists(&state, &new_dictation));
        assert!(exists(&state, &old_note), "history retention keeps notes");
        assert!(exists(&state, &new_note));

        let notes_window = AppSettings {
            notes_retention_days: Some(7),
            ..shorter.clone()
        };
        apply_settings_change_in(&state, &host, &effects, &shorter, notes_window).unwrap();
        assert!(!exists(&state, &old_note));
        assert!(exists(&state, &new_note));
        assert!(exists(&state, &new_dictation));
    }

    #[test]
    fn merge_replaces_only_the_named_fields() {
        let current = AppSettings::default();
        let next =
            merge_patch(&current, &patch(json!({ "pillX": 40, "theme": "pewter" }))).unwrap();

        assert_eq!(next.pill_x, Some(40));
        assert_eq!(next.theme, "pewter");
        let mut expected = current.clone();
        expected.pill_x = Some(40);
        expected.theme = "pewter".to_string();
        assert_eq!(next, expected);
    }

    #[test]
    fn merge_replaces_a_nested_setting_as_a_whole() {
        let current = AppSettings::default();
        let mut hotkeys = serde_json::to_value(&current.hotkeys).unwrap();
        hotkeys["pasteLastTranscript"] = json!("Ctrl+Alt+P");
        let next = merge_patch(&current, &patch(json!({ "hotkeys": hotkeys }))).unwrap();

        assert_eq!(next.hotkeys.paste_last_transcript, "Ctrl+Alt+P");
        assert_eq!(
            next.hotkeys.toggle_dictation,
            current.hotkeys.toggle_dictation
        );
    }

    #[test]
    fn merge_clears_an_optional_setting_with_null() {
        let current = AppSettings {
            selected_mic_id: Some("usb-mic".to_string()),
            ..AppSettings::default()
        };
        let next = merge_patch(&current, &patch(json!({ "selectedMicId": null }))).unwrap();
        assert_eq!(next.selected_mic_id, None);
    }

    #[test]
    fn merge_rejects_unknown_keys_and_wrong_types() {
        let current = AppSettings::default();
        let unknown = merge_patch(&current, &patch(json!({ "pill_x": 4 }))).unwrap_err();
        assert_eq!(unknown.code, "invalid_settings");
        let wrong_type =
            merge_patch(&current, &patch(json!({ "soundsEnabled": "yes" }))).unwrap_err();
        assert_eq!(wrong_type.code, "invalid_settings");
    }

    #[test]
    fn a_patch_that_changes_nothing_does_not_commit() {
        let writes = Mutex::new(());
        let current = AppSettings::default();
        let result = patch_locked(
            &writes,
            &patch(json!({ "theme": current.theme.clone() })),
            || Ok(current.clone()),
            |_, _| panic!("nothing changed, so nothing is saved"),
        )
        .unwrap();
        assert_eq!(result, current);
    }

    #[test]
    fn a_failed_commit_saves_nothing() {
        let writes = Mutex::new(());
        let db = Database::in_memory().unwrap();
        let error = patch_locked(
            &writes,
            &patch(json!({ "pillX": 77 })),
            || db.get_settings(),
            |_, _| Err(CommandError::new("autostart_failed", "no")),
        )
        .unwrap_err();
        assert_eq!(error.code, "autostart_failed");
        assert_eq!(db.get_settings().unwrap(), AppSettings::default());
    }

    /// Windows patch different fields at the same moment, many times over.
    /// Under the write lock every patch lands on the settings the previous
    /// one saved, so no window's change is lost.
    #[test]
    fn concurrent_patches_of_different_fields_never_lose_a_field() {
        let writes = Arc::new(Mutex::new(()));
        let db = Arc::new(Mutex::new(Database::in_memory().unwrap()));
        let rounds = 25;
        let patches: Vec<Box<dyn Fn(i32) -> Value + Send + Sync>> = vec![
            Box::new(|round| json!({ "pillX": round })),
            Box::new(|round| json!({ "pillY": round + 1000 })),
            Box::new(|round| json!({ "windowWidth": round + 2000 })),
            Box::new(|round| json!({ "vocabularyPrompt": format!("words {round}") })),
            Box::new(|round| json!({ "soundsEnabled": round % 2 == 0 })),
        ];
        let patches = Arc::new(patches);
        let barrier = Arc::new(Barrier::new(patches.len()));

        let handles: Vec<_> = (0..patches.len())
            .map(|index| {
                let (writes, db, patches, barrier) = (
                    Arc::clone(&writes),
                    Arc::clone(&db),
                    Arc::clone(&patches),
                    Arc::clone(&barrier),
                );
                thread::spawn(move || {
                    barrier.wait();
                    for round in 1..=rounds {
                        let change = patch(patches[index](round));
                        patch_locked(
                            &writes,
                            &change,
                            || db.lock().unwrap().get_settings(),
                            |_, next| {
                                // Widen the read-to-save window so a missing
                                // lock would reliably lose updates.
                                thread::yield_now();
                                db.lock().unwrap().save_settings(&next)?;
                                Ok(next)
                            },
                        )
                        .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let stored = db.lock().unwrap().get_settings().unwrap();
        assert_eq!(stored.pill_x, Some(rounds));
        assert_eq!(stored.pill_y, Some(rounds + 1000));
        assert_eq!(stored.window_width, Some(rounds + 2000));
        assert_eq!(stored.vocabulary_prompt, format!("words {rounds}"));
        assert!(!stored.sounds_enabled, "round 25 is odd, so sounds end off");
    }
}
