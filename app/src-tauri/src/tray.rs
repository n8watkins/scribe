use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

use crate::{
    app_state::AppStatus,
    dictation_worker::{self, Intent},
    error::CommandError,
};

const TRAY_ID: &str = "scribe-main-tray";
const MENU_START_DICTATION: &str = "start_dictation";
const MENU_STOP_DICTATION: &str = "stop_dictation";
const MENU_PASTE_LAST_TRANSCRIPT: &str = "paste_last_transcript";
const MENU_OPEN_DASHBOARD: &str = "open_dashboard";
const MENU_OPEN_HISTORY: &str = "open_history";
const MENU_SETTINGS: &str = "settings";
const MENU_QUIT: &str = "quit";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NavigationPayload {
    route: String,
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
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
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &start, &stop, &paste, &dashboard, &history, &settings, &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(format!("{} - Idle", crate::display_name(app)))
        // Left-click opens the dashboard (handled below); the menu stays on
        // right-click only.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = open_dashboard(tray.app_handle(), None);
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            // Dictation actions run on the dictation worker, never on this
            // (main) thread, so a stop that transcribes cannot freeze the UI.
            MENU_START_DICTATION => dictation_worker::submit(app, Intent::StartFromTray),
            MENU_STOP_DICTATION => dictation_worker::submit(app, Intent::Stop),
            MENU_PASTE_LAST_TRANSCRIPT => dictation_worker::submit(app, Intent::PasteLast),
            MENU_OPEN_DASHBOARD => {
                let _ = open_dashboard(app, None);
            }
            MENU_OPEN_HISTORY => {
                let _ = open_dashboard(app, Some("history"));
            }
            MENU_SETTINGS => {
                let _ = open_dashboard(app, Some("settings"));
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
            crate::display_name(app),
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
