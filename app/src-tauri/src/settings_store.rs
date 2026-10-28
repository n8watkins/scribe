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
use tauri::{AppHandle, Emitter};

use crate::{error::CommandError, settings::AppSettings};

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

/// Tells every window the stored settings changed.
pub fn emit_changed(app: &AppHandle, settings: &AppSettings) {
    let _ = app.emit(SETTINGS_CHANGED_EVENT, settings);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use serde_json::json;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn patch(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => panic!("patch must be an object"),
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
