use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

use crate::{
    app_state::AppStatus,
    audio::{self, RecordingResultStatus},
    commands::BackendState,
    dictation,
    error::CommandError,
    output,
};

const TRAY_ID: &str = "localdictate-main-tray";
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
    let dashboard = MenuItem::with_id(
        app,
        MENU_OPEN_DASHBOARD,
        "Open Dashboard",
        true,
        None::<&str>,
    )?;
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
        .tooltip("LocalDictate - Idle")
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
            MENU_START_DICTATION => {
                // Tray start behaves like the toggle hotkey (the user is not
                // holding any key), so silence auto-stop applies.
                let _ = start_dictation(app, true);
            }
            MENU_STOP_DICTATION => {
                let _ = stop_dictation(app);
            }
            MENU_PASTE_LAST_TRANSCRIPT => {
                let _ = paste_last_transcript(app);
            }
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

/// Starts dictation. `allow_auto_stop` controls whether silence auto-stop may
/// end this recording: true for toggle-style starts (toggle hotkey, tray
/// menu), false for hold-to-talk where the user is still holding the key.
pub fn start_dictation(app: &AppHandle, allow_auto_stop: bool) -> Result<(), CommandError> {
    let state = app.state::<BackendState>();
    let snapshot = state.app_state()?.snapshot();

    if matches!(
        snapshot.status,
        AppStatus::Idle | AppStatus::Ready | AppStatus::Error | AppStatus::Recording
    ) {
        let _ = audio::start_recording_for_app(app, None, allow_auto_stop)?;
        let status = state.app_state()?.status().clone();
        update_tray_status(app, status);
    }

    Ok(())
}

pub fn stop_dictation(app: &AppHandle) -> Result<(), CommandError> {
    let state = app.state::<BackendState>();
    let snapshot = state.app_state()?.snapshot();

    if snapshot.status != AppStatus::Recording {
        return Ok(());
    }

    let result = audio::stop_recording_for_app(app)?;
    let status = state.app_state()?.status().clone();
    update_tray_status(app, status);

    if result.status == RecordingResultStatus::TooShort {
        // A quick tap of the toggle hotkey is benign, not an error: the
        // state machine already returned to Idle (AudioTooShort), so just
        // tell the frontend nothing was heard.
        dictation::emit_dictation_empty(app);
        return Ok(());
    }

    dictation::transcribe_recording_for_app(app, result)?;
    let status = state.app_state()?.status().clone();
    update_tray_status(app, status);
    Ok(())
}

pub fn paste_last_transcript(app: &AppHandle) -> Result<(), CommandError> {
    output::paste_last_transcript(app)?;
    let state = app.state::<BackendState>();
    let status = state.app_state()?.status().clone();
    update_tray_status(app, status);
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
        "localdictate:navigate",
        NavigationPayload {
            route: route.to_string(),
        },
    );

    Ok(())
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

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(format!("LocalDictate - {}", label)));
        let _ = tray.set_title(Some(label));
    }
}
