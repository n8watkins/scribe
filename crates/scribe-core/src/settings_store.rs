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
use tauri::{AppHandle, Manager};

use crate::{
    backend::BackendState,
    core::Core,
    error::CommandError,
    events::CoreEvent,
    host::{no_main_window, Host},
    hotkeys,
    settings::AppSettings,
};

/// Emitted with the full stored settings after every settings write.
pub const SETTINGS_CHANGED_EVENT: &str = "scribe:settings-changed";

/// Takes the settings write lock. Held from reading the stored settings to
/// saving the new ones (and announcing them), so no write is lost and the
/// events go out in the order the writes happened. The guarded value is `()`,
/// so a lock poisoned by a panic carries no broken state and is recovered.
pub fn lock(writes: &Mutex<()>) -> MutexGuard<'_, ()> {
    writes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Tells every window, and the tray menu, the stored settings changed.
pub fn emit_changed_with(host: &dyn Host, settings: &AppSettings) {
    host.emit(CoreEvent::SettingsChanged(settings.clone()));
    // The tray menu's "Launch at startup" check mark shows the stored value.
    host.sync_settings(settings);
}

/// [`emit_changed_with`] for callers that still hold an `AppHandle`. Needs
/// only the host, which the shell installs before anything else.
pub fn emit_changed(_app: &AppHandle, settings: &AppSettings) {
    match crate::shell::host() {
        Some(host) => emit_changed_with(host.as_ref(), settings),
        None => log::warn!("Could not announce the settings change: no host yet"),
    }
}

/// The `Core`, for the wrappers below that still take an `AppHandle`.
fn core_of(app: &AppHandle) -> Result<Core, CommandError> {
    match app.try_state::<Core>() {
        Some(core) => Ok(core.inner().clone()),
        None => {
            log::warn!("Could not reach the settings: the engine is not running yet");
            Err(no_main_window())
        }
    }
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
///
/// Still takes the `AppHandle` for the hotkeys and the startup entry (see
/// [`apply_settings_change_with`]).
pub fn patch_settings_now_with(
    app: &AppHandle,
    core: &Core,
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, CommandError> {
    let state = core.backend();
    patch_locked(
        state.settings_writes(),
        patch,
        || state.db()?.get_settings(),
        |previous, next| apply_settings_change_with(app, core, previous, next),
    )
}

/// [`patch_settings_now_with`] for callers that still hold only an
/// `AppHandle`.
pub fn patch_settings_now(
    app: &AppHandle,
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, CommandError> {
    patch_settings_now_with(app, &core_of(app)?, patch)
}

/// Shows the switch the user may have flipped in Windows' Startup apps list
/// while Scribe was running: copies it into "Launch at startup". Never writes
/// the startup entry. Holds the settings write lock from read to save, so it
/// cannot undo a flip the user is making in Scribe at the same moment.
/// Blocks; call it off the main thread.
///
/// Still takes the `AppHandle` for `autostart::follow_windows_switch`.
pub fn sync_launch_at_startup_with(
    app: &AppHandle,
    core: &Core,
) -> Result<AppSettings, CommandError> {
    follow_launch_switch(core.backend(), core.host().as_ref(), |stored| {
        crate::autostart::follow_windows_switch(app, stored)
    })
}

/// [`sync_launch_at_startup_with`] for callers that still hold only an
/// `AppHandle`.
pub fn sync_launch_at_startup(app: &AppHandle) -> Result<AppSettings, CommandError> {
    sync_launch_at_startup_with(app, &core_of(app)?)
}

/// The body of [`sync_launch_at_startup_with`]: `follow` gets the stored
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
pub fn set_launch_at_startup_with(
    app: &AppHandle,
    core: &Core,
    enabled: bool,
) -> Result<AppSettings, CommandError> {
    let mut patch = serde_json::Map::new();
    patch.insert("launchAtStartup".into(), serde_json::Value::Bool(enabled));
    patch_settings_now_with(app, core, &patch)
}

/// [`set_launch_at_startup_with`] for callers that still hold only an
/// `AppHandle`.
pub fn set_launch_at_startup(app: &AppHandle, enabled: bool) -> Result<AppSettings, CommandError> {
    set_launch_at_startup_with(app, &core_of(app)?, enabled)
}

/// Stores `settings` over `previous` and applies what changed: launch at
/// startup, hotkey registration, GitHub activity, and retention. Rolls the
/// side effects back when the save fails. The caller holds the settings write
/// lock.
///
/// Still takes the `AppHandle` for the hotkeys lane's `set_transform_enabled`,
/// `replace_hotkeys` and `emit_registration_failures`, and for
/// `autostart::apply`.
fn apply_settings_change_with(
    app: &AppHandle,
    core: &Core,
    previous: &AppSettings,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    let state = core.backend();
    settings
        .validate()
        .map_err(CommandError::invalid_settings)?;

    // Apply launch-at-startup first: when the OS registration fails the new
    // value is never saved, so the UI toggle reverts.
    if previous.launch_at_startup != settings.launch_at_startup {
        crate::autostart::apply(app, settings.launch_at_startup)?;
    }

    // Transform Selection is only bound while Local AI is on, so turning it on
    // or off re-registers the hotkey set too.
    let transform_toggled = previous.notes_analysis_enabled != settings.notes_analysis_enabled;
    let rebind = previous.hotkeys != settings.hotkeys || transform_toggled;
    let mut hotkey_failures = Vec::new();
    if rebind {
        hotkeys::set_transform_enabled(app, settings.notes_analysis_enabled);
        hotkey_failures = match hotkeys::replace_hotkeys(app, &previous.hotkeys, &settings.hotkeys)
        {
            Ok(failures) => failures,
            Err(error) => {
                hotkeys::set_transform_enabled(app, previous.notes_analysis_enabled);
                return Err(error);
            }
        };
    }

    if let Err(error) = save_settings_change(state, previous, &settings) {
        if rebind {
            hotkeys::set_transform_enabled(app, previous.notes_analysis_enabled);
            let _ = hotkeys::replace_hotkeys(app, &settings.hotkeys, &previous.hotkeys);
        }
        if previous.launch_at_startup != settings.launch_at_startup {
            let _ = crate::autostart::apply(app, previous.launch_at_startup);
        }

        return Err(error);
    }

    // Every window updates from this event, so it goes out as soon as the
    // new settings are stored.
    emit_changed_with(core.host().as_ref(), &settings);
    // Load, reload or unload the speech model to match (model, language,
    // GPU and "Keep the speech model loaded").
    core.warm_transcriber().sync_with_settings(&settings);
    hotkeys::emit_registration_failures(app, &hotkey_failures);

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
    use crate::host::{Flavor, RecordingHost};
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
    fn an_unchanged_startup_switch_saves_and_announces_nothing() {
        let root = tempfile::tempdir().unwrap();
        let host = host_in(root.path());
        let state = state_in(root.path());

        let settings = follow_launch_switch(&state, &host, |_| None).unwrap();
        assert_eq!(settings, AppSettings::default());
        assert!(!state.db().unwrap().has_saved_settings().unwrap());
        assert!(host.events().is_empty());
    }

    #[test]
    fn a_settings_change_is_saved() {
        let root = tempfile::tempdir().unwrap();
        let state = state_in(root.path());
        let previous = AppSettings::default();
        for next in [
            AppSettings {
                pill_x: Some(3),
                ..previous.clone()
            },
            AppSettings {
                github_repo: "someone/notes".into(),
                ..previous.clone()
            },
        ] {
            save_settings_change(&state, &previous, &next).unwrap();
            assert_eq!(state.db().unwrap().get_settings().unwrap(), next);
            enforce_retention_change(&state, &previous, &next).unwrap();
        }
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
