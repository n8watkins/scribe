use serde::Serialize;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

use crate::{
    app_state::AppStatus,
    dictation_worker::{self, Intent},
    error::CommandError,
    host::Flavor,
    settings::AppSettings,
    settings_store, Core,
};

const TRAY_ID: &str = "scribe-main-tray";
const MENU_START_DICTATION: &str = "start_dictation";
const MENU_STOP_DICTATION: &str = "stop_dictation";
const MENU_PASTE_LAST_TRANSCRIPT: &str = "paste_last_transcript";
const MENU_OPEN_DASHBOARD: &str = "open_dashboard";
const MENU_OPEN_HISTORY: &str = "open_history";
const MENU_SETTINGS: &str = "settings";
const MENU_LAUNCH_AT_STARTUP: &str = "launch_at_startup";
const MENU_QUIT: &str = "quit";

/// The same words as the switch in Settings > General.
pub const LAUNCH_AT_STARTUP_LABEL: &str = "Launch at startup";

/// The tray menu's "Launch at startup" item, kept so its check mark can follow
/// the setting.
struct LaunchAtStartupItem(CheckMenuItem<tauri::Wry>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NavigationPayload {
    route: String,
}

pub fn setup(app: &AppHandle, settings: &AppSettings) -> tauri::Result<()> {
    let start = MenuItem::with_id(
        app,
        MENU_START_DICTATION,
        "Start Dictation",
        true,
        None::<&str>,
    )?;
    let stop = MenuItem::with_id(
        app,
        MENU_STOP_DICTATION,
        "Stop Dictation",
        true,
        None::<&str>,
    )?;
    let paste = MenuItem::with_id(
        app,
        MENU_PASTE_LAST_TRANSCRIPT,
        "Paste Last Transcript",
        true,
        None::<&str>,
    )?;
    let dashboard = MenuItem::with_id(app, MENU_OPEN_DASHBOARD, "Open Scribe", true, None::<&str>)?;
    let history = MenuItem::with_id(app, MENU_OPEN_HISTORY, "Open History", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, None::<&str>)?;
    let launch_at_startup = CheckMenuItem::with_id(
        app,
        MENU_LAUNCH_AT_STARTUP,
        LAUNCH_AT_STARTUP_LABEL,
        true,
        settings.launch_at_startup,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &start,
            &stop,
            &paste,
            &dashboard,
            &history,
            &settings_item,
            &PredefinedMenuItem::separator(app)?,
            &launch_at_startup,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    app.manage(LaunchAtStartupItem(launch_at_startup));

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(format!("{} - Idle", display_name(app)))
        // Left-click opens the dashboard (handled below); the menu stays on
        // right-click only.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => {
                let _ = open_dashboard(tray.app_handle(), None);
            }
            // Before the menu opens (on hover, and on the right button going
            // down; the menu opens when it comes up), refresh the "Launch at
            // startup" tick from Windows' Startup apps switch.
            TrayIconEvent::Enter { .. }
            | TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Down,
                ..
            } => refresh_launch_at_startup(tray.app_handle()),
            _ => {}
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            // Dictation actions run on the dictation worker, never on this
            // (main) thread, so a stop that transcribes cannot freeze the UI.
            MENU_START_DICTATION => submit(app, Intent::StartFromTray),
            MENU_STOP_DICTATION => submit(app, Intent::Stop),
            MENU_PASTE_LAST_TRANSCRIPT => submit(app, Intent::PasteLast),
            MENU_OPEN_DASHBOARD => {
                let _ = open_dashboard(app, None);
            }
            MENU_OPEN_HISTORY => {
                let _ = open_dashboard(app, Some("history"));
            }
            MENU_SETTINGS => {
                let _ = open_dashboard(app, Some("settings"));
            }
            MENU_LAUNCH_AT_STARTUP => {
                // The menu has already flipped the tick, so its state is what
                // the user asked for.
                let wanted = app
                    .try_state::<LaunchAtStartupItem>()
                    .and_then(|item| item.0.is_checked().ok());
                if let Some(wanted) = wanted {
                    set_launch_at_startup(app, wanted);
                }
            }
            MENU_QUIT => app.exit(0),
            _ => {}
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    update_tray_status(app, AppStatus::Idle);

    Ok(())
}

/// "Scribe" or "Scribe Dev", from the app's identifier, the same way the
/// host tells the flavours apart.
fn display_name(app: &AppHandle) -> &'static str {
    Flavor::from_identifier(&app.config().identifier).display_name()
}

/// The engine, once setup has managed it (the tray is built after that, so
/// a tray event always finds it).
fn core_of(app: &AppHandle) -> Option<Core> {
    app.try_state::<Core>().map(|core| core.inner().clone())
}

/// Queues a dictation action from the menu on the dictation worker.
fn submit(app: &AppHandle, intent: Intent) {
    match core_of(app) {
        Some(core) => dictation_worker::submit_with(&core, intent),
        None => log::error!(
            "Dictation worker is not reachable yet: the engine is not ready; dropped {:?}",
            intent
        ),
    }
}

/// Sets "Launch at startup" from the tray to the state the user ticked, the
/// same way the switch in Settings does. Runs off the main thread: it writes
/// the registry and the database.
fn set_launch_at_startup(app: &AppHandle, wanted: bool) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("launch-at-startup".into())
        .spawn(move || {
            let Some(core) = core_of(&app) else {
                log::warn!("Could not reach the settings: the engine is not running yet");
                return;
            };
            if let Err(error) = settings_store::set_launch_at_startup_with(&app, &core, wanted) {
                log::warn!("Could not change launch at startup. {}", error.message);
                // Put the tick back to the stored value.
                if let Ok(settings) = core.backend().db().and_then(|db| db.get_settings()) {
                    sync_settings(&app, &settings);
                }
            }
        });
    if let Err(error) = spawned {
        log::warn!("Could not change launch at startup. {}", error);
    }
}

/// Copies Windows' Startup apps switch into "Launch at startup" (and so the
/// tick). Never writes the startup entry. Runs off the main thread.
fn refresh_launch_at_startup(app: &AppHandle) {
    if !cfg!(windows) {
        return;
    }
    let Some(core) = core_of(app) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("launch-at-startup".into())
        .spawn(move || {
            if let Err(error) = settings_store::sync_launch_at_startup_with(&core) {
                log::warn!(
                    "Could not follow Windows' Startup apps switch. {}",
                    error.message
                );
            }
        });
}

/// Shows the stored "Launch at startup" value in the tray menu.
pub(crate) fn sync_settings(app: &AppHandle, settings: &AppSettings) {
    if let Some(item) = app.try_state::<LaunchAtStartupItem>() {
        let _ = item.0.set_checked(settings.launch_at_startup);
    }
}

/// Dashboard hotkey behavior: when the window is already visible and focused,
/// hide it again; otherwise show and focus it. The caller gates this on the
/// dashboardHotkeyToggles setting.
pub fn toggle_dashboard(app: &AppHandle) -> Result<(), CommandError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| CommandError::new("window_not_found", "Could not find the main window."))?;

    if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
        return window
            .hide()
            .map_err(|error| CommandError::new("window_hide_failed", error.to_string()));
    }

    open_dashboard(app, None)
}

pub fn open_dashboard(app: &AppHandle, route: Option<&str>) -> Result<(), CommandError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| CommandError::new("window_not_found", "Could not find the main window."))?;

    window
        .show()
        .map_err(|error| CommandError::new("window_show_failed", error.to_string()))?;
    window
        .set_focus()
        .map_err(|error| CommandError::new("window_focus_failed", error.to_string()))?;

    let route = route.unwrap_or("dashboard");
    let _ = app.emit(
        "scribe:navigate",
        NavigationPayload {
            route: route.to_string(),
        },
    );

    Ok(())
}

/// The tooltip's second line: the speech model and whether it is loaded.
static ENGINE_SUMMARY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// The tooltip's first line's status word.
static STATUS_LABEL: std::sync::Mutex<&'static str> = std::sync::Mutex::new("Idle");

fn set_tooltip(app: &AppHandle) {
    let label = *STATUS_LABEL.lock().unwrap_or_else(|p| p.into_inner());
    let engine = ENGINE_SUMMARY
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip_text(
            display_name(app),
            label,
            engine.as_deref(),
        )));
    }
}

/// "Scribe - Idle" plus, when known, the model line. Windows cuts tooltips
/// at 127 characters, so the model line is trimmed to fit.
fn tooltip_text(app_name: &str, label: &str, engine: Option<&str>) -> String {
    let first = format!("{app_name} - {label}");
    match engine {
        Some(engine) => {
            let room = 127usize.saturating_sub(first.chars().count() + 1);
            let engine: String = engine.chars().take(room).collect();
            format!("{first}\n{engine}")
        }
        None => first,
    }
}

/// Shows which speech model is active and whether it is loaded.
pub(crate) fn update_engine_summary(app: &AppHandle, summary: String) {
    *ENGINE_SUMMARY.lock().unwrap_or_else(|p| p.into_inner()) = Some(summary);
    set_tooltip(app);
}

pub(crate) fn update_tray_status(app: &AppHandle, status: AppStatus) {
    let label = match status {
        AppStatus::Idle => "Idle",
        AppStatus::Recording => "Recording",
        AppStatus::Stopping => "Stopping",
        AppStatus::Transcribing => "Transcribing",
        AppStatus::Pasting => "Pasting",
        AppStatus::Ready => "Ready",
        AppStatus::Error => "Error",
        AppStatus::Paused => "Paused",
    };

    *STATUS_LABEL.lock().unwrap_or_else(|p| p.into_inner()) = label;
    set_tooltip(app);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_title(Some(label));
    }
}

#[cfg(test)]
mod tests {
    use super::tooltip_text;

    #[test]
    fn tooltip_shows_status_then_model() {
        assert_eq!(tooltip_text("Scribe", "Idle", None), "Scribe - Idle");
        assert_eq!(
            tooltip_text("Scribe", "Idle", Some("Parakeet v3 · loaded · 0.9 GB")),
            "Scribe - Idle\nParakeet v3 · loaded · 0.9 GB"
        );
    }

    #[test]
    fn tooltip_fits_the_windows_limit() {
        let long = "x".repeat(300);
        assert_eq!(
            tooltip_text("Scribe Dev", "Transcribing", Some(&long))
                .chars()
                .count(),
            127
        );
    }
}
