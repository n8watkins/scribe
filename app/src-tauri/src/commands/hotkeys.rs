//! Commands of the `hotkeys` domain (hotkeys, remote_command,
//! platform/linux).

use tauri::Manager;

use super::off_main;
use crate::{
    commands::BackendState,
    error::CommandError,
    hotkeys::{self, HotkeyStatus},
    settings::AppSettings,
    settings_store,
};

/// Shows the desktop's GlobalShortcuts dialog again after it was cancelled
/// (Wayland), then reports the hotkeys.
#[tauri::command]
pub async fn bind_portal_shortcuts(app: tauri::AppHandle) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        #[cfg(target_os = "linux")]
        {
            crate::platform::shortcuts::bind_again()
                .map_err(|message| CommandError::new("portal_shortcuts_failed", message))?;
            let settings = app.state::<BackendState>().inner().db()?.get_settings()?;
            hotkeys::status(app, &settings.hotkeys)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = app;
            Err(CommandError::new(
                "portal_shortcuts_unsupported",
                "Desktop-managed shortcuts exist only on Linux.",
            ))
        }
    })
    .await
}

/// Opens the desktop's own settings for Scribe's shortcuts (Wayland, on
/// portals that offer it).
#[tauri::command]
pub async fn configure_portal_shortcuts(app: tauri::AppHandle) -> Result<(), CommandError> {
    off_main(app, |_| {
        #[cfg(target_os = "linux")]
        {
            crate::platform::shortcuts::configure()
                .map_err(|message| CommandError::new("portal_shortcuts_failed", message))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(CommandError::new(
                "portal_shortcuts_unsupported",
                "Desktop-managed shortcuts exist only on Linux.",
            ))
        }
    })
    .await
}

#[tauri::command]
pub async fn get_hotkey_status(app: tauri::AppHandle) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let settings = state.db()?.get_settings()?;
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

#[tauri::command]
pub async fn rebind_hotkey(
    app: tauri::AppHandle,
    action: String,
    shortcut: String,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let action = hotkeys::HotkeyAction::parse(&action)?;
        // An X11 grab needs a key and takes a bare key from every app.
        #[cfg(target_os = "linux")]
        if crate::settings::unusable_on_linux(&shortcut) {
            return Err(CommandError::new(
                "invalid_hotkey",
                "Linux needs a key with Ctrl, Alt or Super, such as Ctrl+Alt+Space. Function keys such as F9 also work alone.",
            ));
        }
        let mut settings = state.db()?.get_settings()?;
        let previous_hotkeys = settings.hotkeys.clone();
        let mut next_hotkeys = previous_hotkeys.clone();

        action.set_shortcut(&mut next_hotkeys, shortcut);
        hotkeys::validate_hotkeys(&next_hotkeys)?;
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        // When the binding being changed is the one that failed, restore the
        // previous set (other bindings stay registered either way) and surface
        // the error inline to the rebind UI.
        if let Some(failure) = failures.iter().find(|failure| failure.action == action) {
            let message = failure.message.clone();
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(CommandError::new("hotkey_registration_failed", message));
        }

        settings.hotkeys = next_hotkeys.clone();
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

/// Sets whether a single-shot bind (Toggle, Paste, Open Dashboard) acts on key
/// press or release. Rejected for Hold-to-Talk, which is push-to-talk. Re-runs
/// registration so the toggle watcher picks up the new edge, then persists.
#[tauri::command]
pub async fn set_hotkey_trigger(
    app: tauri::AppHandle,
    action: String,
    trigger: crate::settings::TriggerEdge,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let action = hotkeys::HotkeyAction::parse(&action)?;
        let mut settings = state.db()?.get_settings()?;
        let previous_hotkeys = settings.hotkeys.clone();
        let mut next_hotkeys = previous_hotkeys.clone();

        if !action.set_trigger(&mut next_hotkeys, trigger) {
            return Err(CommandError::new(
                "invalid_hotkey_trigger",
                "Hold-to-Talk is push-to-talk; it has no press/release option.",
            ));
        }

        hotkeys::validate_hotkeys(&next_hotkeys)?;
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        settings.hotkeys = next_hotkeys.clone();
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

#[tauri::command]
pub async fn reset_hotkeys_to_defaults(
    app: tauri::AppHandle,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        let previous_hotkeys = settings.hotkeys.clone();
        // "Defaults" are flavor-specific: the Dev flavor resets to its
        // non-conflicting binds, stable resets to the production binds.
        let next_hotkeys = if crate::is_dev_flavor(app) {
            crate::settings::HotkeySettings::dev_defaults()
        } else {
            AppSettings::default().hotkeys
        };

        hotkeys::validate_hotkeys(&next_hotkeys)?;
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        settings.hotkeys = next_hotkeys.clone();
        settings.dev_hotkeys_seeded = true;
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}

/// Loads the production (stable-flavor) hotkey defaults. Exposed for the Dev
/// flavor's Developer panel so you can switch Dev back to your real binds when
/// running it alone. Sets `dev_hotkeys_seeded` so the one-shot dev seeding
/// never re-applies the Dev binds on the next launch.
#[tauri::command]
pub async fn load_production_hotkey_defaults(
    app: tauri::AppHandle,
) -> Result<HotkeyStatus, CommandError> {
    off_main(app, move |app| {
        let state = app.state::<BackendState>().inner();
        let _writes = settings_store::lock(state.settings_writes());
        let mut settings = state.db()?.get_settings()?;
        let previous_hotkeys = settings.hotkeys.clone();
        let next_hotkeys = AppSettings::default().hotkeys;

        hotkeys::validate_hotkeys(&next_hotkeys)?;
        let failures = hotkeys::replace_hotkeys(app, &previous_hotkeys, &next_hotkeys)?;

        settings.hotkeys = next_hotkeys.clone();
        settings.dev_hotkeys_seeded = true;
        if let Err(error) = state.db()?.save_settings(&settings) {
            let _ = hotkeys::replace_hotkeys(app, &next_hotkeys, &previous_hotkeys);
            return Err(error);
        }

        settings_store::emit_changed(app, &settings);
        hotkeys::emit_registration_failures(app, &failures);
        hotkeys::status(app, &settings.hotkeys)
    })
    .await
}
