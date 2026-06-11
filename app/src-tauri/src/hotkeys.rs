use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::{
    app_state::AppStatus, audio, commands::BackendState, error::CommandError,
    settings::HotkeySettings, tray,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyAction {
    HoldToTalk,
    ToggleDictation,
    PasteLastTranscript,
    OpenDashboard,
}

impl HotkeyAction {
    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "holdToTalk" | "hold_to_talk" | "hold-to-talk" => Ok(Self::HoldToTalk),
            "toggleDictation" | "toggle_dictation" | "toggle-dictation" => {
                Ok(Self::ToggleDictation)
            }
            "pasteLastTranscript" | "paste_last_transcript" | "paste-last-transcript" => {
                Ok(Self::PasteLastTranscript)
            }
            "openDashboard" | "open_dashboard" | "open-dashboard" => Ok(Self::OpenDashboard),
            _ => Err(CommandError::new(
                "invalid_hotkey_action",
                format!("Unknown hotkey action '{}'.", value),
            )),
        }
    }

    pub fn event_name(self) -> &'static str {
        match self {
            Self::HoldToTalk => "hold_to_talk",
            Self::ToggleDictation => "toggle_dictation",
            Self::PasteLastTranscript => "paste_last_transcript",
            Self::OpenDashboard => "open_dashboard",
        }
    }

    pub fn shortcut(self, hotkeys: &HotkeySettings) -> &str {
        match self {
            Self::HoldToTalk => &hotkeys.hold_to_talk,
            Self::ToggleDictation => &hotkeys.toggle_dictation,
            Self::PasteLastTranscript => &hotkeys.paste_last_transcript,
            Self::OpenDashboard => &hotkeys.open_dashboard,
        }
    }

    pub fn set_shortcut(self, hotkeys: &mut HotkeySettings, shortcut: String) {
        match self {
            Self::HoldToTalk => hotkeys.hold_to_talk = shortcut,
            Self::ToggleDictation => hotkeys.toggle_dictation = shortcut,
            Self::PasteLastTranscript => hotkeys.paste_last_transcript = shortcut,
            Self::OpenDashboard => hotkeys.open_dashboard = shortcut,
        }
    }
}

const HOTKEY_ACTIONS: [HotkeyAction; 4] = [
    HotkeyAction::HoldToTalk,
    HotkeyAction::ToggleDictation,
    HotkeyAction::PasteLastTranscript,
    HotkeyAction::OpenDashboard,
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    pub bindings: Vec<HotkeyBindingStatus>,
    pub hold_release_verification_required: bool,
    pub windows_fallback_note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBindingStatus {
    pub action: HotkeyAction,
    pub shortcut: String,
    pub normalized_shortcut: Option<String>,
    pub registered: bool,
    pub error: Option<String>,
}

/// One hotkey binding that could not be registered. Serialized into the
/// "localdictate:hotkey-registration-failed" event payload, so `action`
/// serializes as the camelCase action name (e.g. "holdToTalk").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyRegistrationFailure {
    pub action: HotkeyAction,
    pub shortcut: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
struct RegistrationFailedEvent<'a> {
    failures: &'a [HotkeyRegistrationFailure],
}

/// Notifies the frontend that one or more hotkey bindings could not be
/// registered. No-op when `failures` is empty.
pub fn emit_registration_failures(app: &AppHandle, failures: &[HotkeyRegistrationFailure]) {
    if failures.is_empty() {
        return;
    }

    for failure in failures {
        log::error!(
            "Hotkey registration failed for {:?} ({}): {}",
            failure.action,
            failure.shortcut,
            failure.message
        );
    }

    let _ = app.emit(
        "localdictate:hotkey-registration-failed",
        RegistrationFailedEvent { failures },
    );
}

/// A shortcut made up exclusively of modifier keys (e.g. "Ctrl+Shift").
/// The global-shortcut plugin cannot register these because they have no key
/// code, so on Windows a polling watcher thread drives them instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModifierChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

impl ModifierChord {
    fn is_empty(self) -> bool {
        !(self.ctrl || self.shift || self.alt || self.win)
    }

    /// Canonical user-facing label, e.g. "Ctrl+Shift" or "Ctrl+Alt+Win".
    pub fn label(self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.win {
            parts.push("Win");
        }
        parts.join("+")
    }
}

/// Either a shortcut the global-shortcut plugin can register, or a
/// modifier-only chord handled by the Windows chord watcher.
#[derive(Debug, Clone, Copy)]
pub enum ParsedShortcut {
    Plugin(Shortcut),
    ModifierChord(ModifierChord),
}

impl ParsedShortcut {
    fn binding_key(&self) -> BindingKey {
        match self {
            Self::Plugin(shortcut) => BindingKey::Plugin(shortcut.id()),
            Self::ModifierChord(chord) => BindingKey::Chord(*chord),
        }
    }
}

/// Identity used for duplicate detection across both shortcut kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BindingKey {
    Plugin(u32),
    Chord(ModifierChord),
}

struct ChordWatcherHandle {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

#[derive(Default)]
pub struct HotkeyRuntimeState {
    actions_by_id: Mutex<HashMap<u32, HotkeyAction>>,
    pressed_actions: Mutex<HashSet<HotkeyAction>>,
    chord_bindings: Mutex<Vec<(ModifierChord, HotkeyAction)>>,
    chord_watcher: Mutex<Option<ChordWatcherHandle>>,
    registration_errors: Mutex<HashMap<HotkeyAction, String>>,
}

impl HotkeyRuntimeState {
    fn replace_bindings(&self, actions_by_id: HashMap<u32, HotkeyAction>) {
        if let Ok(mut bindings) = self.actions_by_id.lock() {
            *bindings = actions_by_id;
        }

        if let Ok(mut pressed) = self.pressed_actions.lock() {
            pressed.clear();
        }
    }

    fn action_for(&self, shortcut: &Shortcut) -> Option<HotkeyAction> {
        self.actions_by_id
            .lock()
            .ok()
            .and_then(|bindings| bindings.get(&shortcut.id()).copied())
    }

    fn mark_pressed_once(&self, action: HotkeyAction) -> bool {
        self.pressed_actions
            .lock()
            .map(|mut pressed| pressed.insert(action))
            .unwrap_or(false)
    }

    fn mark_released_once(&self, action: HotkeyAction) -> bool {
        self.pressed_actions
            .lock()
            .map(|mut pressed| pressed.remove(&action))
            .unwrap_or(false)
    }

    fn chord_is_registered(&self, chord: ModifierChord) -> bool {
        self.chord_bindings
            .lock()
            .map(|bindings| bindings.iter().any(|(bound, _)| *bound == chord))
            .unwrap_or(false)
    }

    fn store_chord_bindings(&self, bindings: Vec<(ModifierChord, HotkeyAction)>) {
        if let Ok(mut stored) = self.chord_bindings.lock() {
            *stored = bindings;
        }
    }

    fn take_watcher(&self) -> Option<ChordWatcherHandle> {
        self.chord_watcher
            .lock()
            .ok()
            .and_then(|mut guard| guard.take())
    }

    fn store_registration_errors(&self, failures: &[HotkeyRegistrationFailure]) {
        if let Ok(mut errors) = self.registration_errors.lock() {
            *errors = failures
                .iter()
                .map(|failure| (failure.action, failure.message.clone()))
                .collect();
        }
    }

    fn registration_error_for(&self, action: HotkeyAction) -> Option<String> {
        self.registration_errors
            .lock()
            .ok()
            .and_then(|errors| errors.get(&action).cloned())
    }
}

pub fn setup(app: &AppHandle, hotkeys: &HotkeySettings) -> Result<(), CommandError> {
    app.manage(HotkeyRuntimeState::default());
    validate_hotkeys(hotkeys)?;

    let failures = register_hotkey_set(app, hotkeys);
    log::info!(
        "Registered {} of {} hotkey bindings",
        HOTKEY_ACTIONS.len() - failures.len(),
        HOTKEY_ACTIONS.len()
    );
    emit_registration_failures(app, &failures);

    Ok(())
}

pub fn handle_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    let Some(runtime) = app.try_state::<HotkeyRuntimeState>() else {
        return;
    };
    let Some(action) = runtime.action_for(shortcut) else {
        return;
    };

    match event.state {
        ShortcutState::Pressed => {
            if !runtime.mark_pressed_once(action) {
                return;
            }
            handle_pressed(app, action);
        }
        ShortcutState::Released => {
            if !runtime.mark_released_once(action) {
                return;
            }
            handle_released(app, action);
        }
    }
}

pub fn validate_hotkeys(hotkeys: &HotkeySettings) -> Result<(), CommandError> {
    let mut seen = HashMap::<BindingKey, HotkeyAction>::new();

    for action in HOTKEY_ACTIONS {
        let shortcut = action.shortcut(hotkeys);
        let parsed = parse_shortcut(shortcut)?;

        if let Some(previous_action) = seen.insert(parsed.binding_key(), action) {
            return Err(CommandError::new(
                "duplicate_hotkey",
                format!(
                    "{} is already assigned to {:?}. Choose a different hotkey.",
                    shortcut, previous_action
                ),
            ));
        }
    }

    Ok(())
}

/// Swaps the registered hotkey set. Registration is best-effort per binding:
/// the returned vector lists the bindings in `next` that could not be
/// registered (working bindings stay registered). Returns Err only when
/// `next` fails validation, in which case nothing is changed.
pub fn replace_hotkeys(
    app: &AppHandle,
    previous: &HotkeySettings,
    next: &HotkeySettings,
) -> Result<Vec<HotkeyRegistrationFailure>, CommandError> {
    validate_hotkeys(next)?;

    unregister_hotkey_set(app, previous);

    Ok(register_hotkey_set(app, next))
}

/// Registers every binding in `hotkeys`: plugin shortcuts through the
/// global-shortcut plugin, modifier-only chords through the Windows chord
/// watcher. Each binding is attempted independently; failures are collected
/// and returned while every binding that did register stays registered.
fn register_hotkey_set(
    app: &AppHandle,
    hotkeys: &HotkeySettings,
) -> Vec<HotkeyRegistrationFailure> {
    let mut actions_by_id = HashMap::<u32, HotkeyAction>::new();
    let mut chord_bindings = Vec::<(ModifierChord, HotkeyAction)>::new();
    let mut failures = Vec::<HotkeyRegistrationFailure>::new();
    let mut record_failure = |action: HotkeyAction, shortcut: &str, message: String| {
        failures.push(HotkeyRegistrationFailure {
            action,
            shortcut: shortcut.to_string(),
            message,
        });
    };

    for action in HOTKEY_ACTIONS {
        let shortcut_text = action.shortcut(hotkeys);

        match parse_shortcut(shortcut_text) {
            Ok(ParsedShortcut::Plugin(shortcut)) => {
                match app.global_shortcut().register(shortcut) {
                    Ok(()) => {
                        actions_by_id.insert(shortcut.id(), action);
                    }
                    Err(error) => record_failure(
                        action,
                        shortcut_text,
                        CommandError::hotkey_registration_failed(shortcut_text, error).message,
                    ),
                }
            }
            Ok(ParsedShortcut::ModifierChord(chord)) => {
                if cfg!(windows) {
                    chord_bindings.push((chord, action));
                } else {
                    record_failure(
                        action,
                        shortcut_text,
                        format!(
                            "Could not register {}. Modifier-only shortcuts are only supported on Windows.",
                            shortcut_text
                        ),
                    );
                }
            }
            Err(error) => record_failure(action, shortcut_text, error.message),
        }
    }

    if let Some(runtime) = app.try_state::<HotkeyRuntimeState>() {
        runtime.replace_bindings(actions_by_id);
        runtime.store_registration_errors(&failures);
    }

    configure_chord_watcher(app, chord_bindings);

    failures
}

/// Stops any running chord watcher thread, stores the new chord bindings (so
/// status() can report them as registered), and starts a fresh watcher thread
/// when there is at least one chord to monitor. Called on initial
/// registration and on every rebind/reset/replace.
fn configure_chord_watcher(app: &AppHandle, bindings: Vec<(ModifierChord, HotkeyAction)>) {
    let Some(runtime) = app.try_state::<HotkeyRuntimeState>() else {
        return;
    };

    if let Some(watcher) = runtime.take_watcher() {
        watcher.stop.store(true, Ordering::SeqCst);
        let _ = watcher.thread.join();
    }

    runtime.store_chord_bindings(bindings.clone());

    if bindings.is_empty() {
        return;
    }

    #[cfg(windows)]
    {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let app_handle = app.clone();
        let spawned = std::thread::Builder::new()
            .name("localdictate-chord-watcher".to_string())
            .spawn(move || windows_chord::run_watcher(app_handle, bindings, thread_stop));

        match spawned {
            Ok(thread) => {
                if let Ok(mut guard) = runtime.chord_watcher.lock() {
                    *guard = Some(ChordWatcherHandle { stop, thread });
                }
            }
            Err(error) => {
                log::error!("Failed to start chord watcher thread: {}", error);
                runtime.store_chord_bindings(Vec::new());
            }
        }
    }

    #[cfg(not(windows))]
    {
        // Modifier-only chords are rejected during registration on
        // non-Windows platforms, so this branch never sees a non-empty set.
        runtime.store_chord_bindings(Vec::new());
    }
}

pub fn status(app: &AppHandle, hotkeys: &HotkeySettings) -> Result<HotkeyStatus, CommandError> {
    let runtime = app.try_state::<HotkeyRuntimeState>();
    let mut bindings = Vec::new();

    for action in HOTKEY_ACTIONS {
        let shortcut = action.shortcut(hotkeys);
        let registration_error = runtime
            .as_ref()
            .and_then(|runtime| runtime.registration_error_for(action));
        let (normalized_shortcut, registered, error) = match parse_shortcut(shortcut) {
            Ok(ParsedShortcut::Plugin(parsed)) => {
                let registered = app.global_shortcut().is_registered(parsed);
                let error = if registered { None } else { registration_error };

                (Some(parsed.to_string()), registered, error)
            }
            Ok(ParsedShortcut::ModifierChord(chord)) => {
                let registered = runtime
                    .as_ref()
                    .map(|runtime| runtime.chord_is_registered(chord))
                    .unwrap_or(false);
                let error = if registered {
                    None
                } else {
                    registration_error.or_else(|| {
                        (!cfg!(windows)).then(|| {
                            "Modifier-only shortcuts are only supported on Windows.".to_string()
                        })
                    })
                };

                (Some(chord.label()), registered, error)
            }
            Err(error) => (None, false, Some(error.message)),
        };

        bindings.push(HotkeyBindingStatus {
            action,
            shortcut: shortcut.to_string(),
            normalized_shortcut,
            registered,
            error,
        });
    }

    Ok(HotkeyStatus {
        bindings,
        hold_release_verification_required: true,
        windows_fallback_note:
            "Manual Windows verification is still required for hold-to-talk release events. If releases are unreliable, use a Windows-only SetWindowsHookExW(WH_KEYBOARD_LL) fallback."
                .to_string(),
    })
}

fn handle_pressed(app: &AppHandle, action: HotkeyAction) {
    let _ = app.emit("localdictate:hotkey-action", action.event_name());

    // Hold-to-talk and toggle both always work, regardless of the (legacy)
    // recordingMode setting; gating either one made the other hotkey a
    // silent no-op. Hold-to-talk never auto-stops on silence: the user is
    // still holding the key, and a pause mid-thought must not cut them off.
    let result = match action {
        HotkeyAction::HoldToTalk => tray::start_dictation(app, false),
        HotkeyAction::ToggleDictation => toggle_dictation(app),
        HotkeyAction::PasteLastTranscript => tray::paste_last_transcript(app),
        HotkeyAction::OpenDashboard => {
            let toggles = app
                .state::<BackendState>()
                .db()
                .and_then(|db| db.get_settings())
                .map(|settings| settings.dashboard_hotkey_toggles)
                .unwrap_or(true);
            if toggles {
                tray::toggle_dashboard(app)
            } else {
                tray::open_dashboard(app, None)
            }
        }
    };

    if let Err(error) = result {
        audio::emit_recording_error(app, error);
    }
}

fn handle_released(app: &AppHandle, action: HotkeyAction) {
    if action == HotkeyAction::HoldToTalk {
        if let Err(error) = tray::stop_dictation(app) {
            audio::emit_recording_error(app, error);
        }
    }
}

fn toggle_dictation(app: &AppHandle) -> Result<(), CommandError> {
    let state = app.state::<BackendState>();
    let status = state.app_state()?.status().clone();

    match status {
        // From Error the start path resets the state machine to Idle first
        // (ResetError), so a wedged Error state never disables the toggle:
        // pressing it again starts a fresh dictation. Hold-to-talk gets the
        // same recovery for free because tray::start_dictation handles Error.
        AppStatus::Idle | AppStatus::Ready | AppStatus::Error => tray::start_dictation(app, true),
        AppStatus::Recording => tray::stop_dictation(app),
        _ => Ok(()),
    }
}

fn unregister_hotkey_set(app: &AppHandle, hotkeys: &HotkeySettings) {
    let shortcuts = HOTKEY_ACTIONS
        .into_iter()
        .filter_map(|action| match parse_shortcut(action.shortcut(hotkeys)) {
            Ok(ParsedShortcut::Plugin(shortcut)) => Some(shortcut),
            _ => None,
        })
        .collect::<Vec<_>>();

    unregister_shortcuts(app, &shortcuts);
}

fn unregister_shortcuts(app: &AppHandle, shortcuts: &[Shortcut]) {
    for shortcut in shortcuts {
        if app.global_shortcut().is_registered(*shortcut) {
            let _ = app.global_shortcut().unregister(*shortcut);
        }
    }
}

pub fn parse_shortcut(shortcut: &str) -> Result<ParsedShortcut, CommandError> {
    if let Some(chord) = parse_modifier_chord(shortcut) {
        return Ok(ParsedShortcut::ModifierChord(chord));
    }

    let normalized = normalize_shortcut(shortcut);
    Shortcut::from_str(&normalized)
        .map(ParsedShortcut::Plugin)
        .map_err(|error| CommandError::invalid_hotkey(shortcut, error))
}

/// Returns Some when every +-separated token is a modifier name
/// (case-insensitive): Ctrl/Control, Shift, Alt/Option,
/// Win/Windows/Super/Meta/Cmd/Command. Anything else (including empty
/// strings or trailing '+') falls through to the plugin parser.
fn parse_modifier_chord(shortcut: &str) -> Option<ModifierChord> {
    if shortcut.trim().is_empty() {
        return None;
    }

    let mut chord = ModifierChord::default();

    for token in shortcut.split('+') {
        match token.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => chord.ctrl = true,
            "shift" => chord.shift = true,
            "alt" | "option" => chord.alt = true,
            "win" | "windows" | "super" | "meta" | "cmd" | "command" => chord.win = true,
            _ => return None,
        }
    }

    if chord.is_empty() {
        None
    } else {
        Some(chord)
    }
}

fn normalize_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|token| match token.trim().to_ascii_lowercase().as_str() {
            "win" | "windows" | "meta" => "Super".to_string(),
            "control" => "Ctrl".to_string(),
            "~" | "`" | "tilde" | "backquote" => "Backquote".to_string(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(windows)]
fn dispatch_chord_pressed(app: &AppHandle, action: HotkeyAction) {
    let Some(runtime) = app.try_state::<HotkeyRuntimeState>() else {
        return;
    };
    if !runtime.mark_pressed_once(action) {
        return;
    }
    handle_pressed(app, action);
}

#[cfg(windows)]
fn dispatch_chord_released(app: &AppHandle, action: HotkeyAction) {
    let Some(runtime) = app.try_state::<HotkeyRuntimeState>() else {
        return;
    };
    if !runtime.mark_released_once(action) {
        return;
    }
    handle_released(app, action);
}

#[cfg(windows)]
mod windows_chord {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    use tauri::AppHandle;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
        VK_PACKET, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };

    use super::{dispatch_chord_pressed, dispatch_chord_released, HotkeyAction, ModifierChord};

    const POLL_INTERVAL: Duration = Duration::from_millis(30);
    const ARMING_DELAY: Duration = Duration::from_millis(150);
    /// 0x01..=0x06 are mouse buttons (VK_LBUTTON..VK_XBUTTON2); start above.
    const FIRST_SCANNED_VK: u16 = 0x08;
    const LAST_SCANNED_VK: u16 = 0xFE;

    #[derive(Debug, Clone, Copy)]
    enum ChordState {
        /// Waiting for the chord modifiers to be held down.
        Idle,
        /// Chord is down; firing at the deadline unless another key shows up.
        Arming(Instant),
        /// handle_pressed has fired; waiting for a modifier release.
        Active,
        /// Aborted or finished; waiting for all chord modifiers to be
        /// released before re-arming.
        Suppressed,
    }

    pub(super) fn run_watcher(
        app: AppHandle,
        bindings: Vec<(ModifierChord, HotkeyAction)>,
        stop: Arc<AtomicBool>,
    ) {
        let mut states = vec![ChordState::Idle; bindings.len()];

        while !stop.load(Ordering::SeqCst) {
            let snapshot = ModifierSnapshot::capture();

            for (index, (chord, action)) in bindings.iter().enumerate() {
                states[index] = step(&app, *chord, *action, states[index], snapshot);
            }

            thread::sleep(POLL_INTERVAL);
        }
    }

    fn step(
        app: &AppHandle,
        chord: ModifierChord,
        action: HotkeyAction,
        state: ChordState,
        snapshot: ModifierSnapshot,
    ) -> ChordState {
        match state {
            ChordState::Idle => {
                if snapshot.exactly(chord) {
                    ChordState::Arming(Instant::now() + ARMING_DELAY)
                } else {
                    ChordState::Idle
                }
            }
            ChordState::Arming(deadline) => {
                if !snapshot.holds_all(chord) {
                    // Chord released before the arming delay elapsed.
                    ChordState::Idle
                } else if !snapshot.exactly(chord) {
                    // An extra modifier joined; this is some other chord.
                    ChordState::Suppressed
                } else if non_modifier_key_down() {
                    // The user was typing a normal shortcut like Ctrl+Shift+T.
                    ChordState::Suppressed
                } else if Instant::now() >= deadline {
                    dispatch_chord_pressed(app, action);
                    ChordState::Active
                } else {
                    ChordState::Arming(deadline)
                }
            }
            ChordState::Active => {
                if !snapshot.holds_all(chord) {
                    dispatch_chord_released(app, action);
                    ChordState::Suppressed
                } else {
                    ChordState::Active
                }
            }
            ChordState::Suppressed => {
                if snapshot.holds_any(chord) {
                    ChordState::Suppressed
                } else {
                    ChordState::Idle
                }
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct ModifierSnapshot {
        ctrl: bool,
        shift: bool,
        alt: bool,
        win: bool,
    }

    impl ModifierSnapshot {
        fn capture() -> Self {
            Self {
                ctrl: vk_down(VK_CONTROL.0),
                shift: vk_down(VK_SHIFT.0),
                alt: vk_down(VK_MENU.0),
                win: vk_down(VK_LWIN.0) || vk_down(VK_RWIN.0),
            }
        }

        /// Exactly the chord's modifiers are down, no more and no less.
        fn exactly(self, chord: ModifierChord) -> bool {
            self.ctrl == chord.ctrl
                && self.shift == chord.shift
                && self.alt == chord.alt
                && self.win == chord.win
        }

        /// Every modifier in the chord is still down (extras allowed).
        fn holds_all(self, chord: ModifierChord) -> bool {
            (!chord.ctrl || self.ctrl)
                && (!chord.shift || self.shift)
                && (!chord.alt || self.alt)
                && (!chord.win || self.win)
        }

        /// At least one modifier in the chord is still down.
        fn holds_any(self, chord: ModifierChord) -> bool {
            (chord.ctrl && self.ctrl)
                || (chord.shift && self.shift)
                || (chord.alt && self.alt)
                || (chord.win && self.win)
        }
    }

    fn vk_down(vk: u16) -> bool {
        (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
    }

    fn is_modifier_vk(vk: u16) -> bool {
        [
            VK_SHIFT,
            VK_CONTROL,
            VK_MENU,
            VK_LWIN,
            VK_RWIN,
            VK_LSHIFT,
            VK_RSHIFT,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_LMENU,
            VK_RMENU,
        ]
        .iter()
        .any(|modifier| modifier.0 == vk)
    }

    /// Scans the virtual-key range (skipping mouse buttons and modifiers) for
    /// any currently held non-modifier key.
    fn non_modifier_key_down() -> bool {
        for vk in FIRST_SCANNED_VK..=LAST_SCANNED_VK {
            if is_modifier_vk(vk) || vk == VK_PACKET.0 {
                continue;
            }

            if vk_down(vk) {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    fn expect_plugin(shortcut: &str) -> Shortcut {
        match parse_shortcut(shortcut).unwrap() {
            ParsedShortcut::Plugin(parsed) => parsed,
            other => panic!(
                "expected plugin shortcut for {:?}, got {:?}",
                shortcut, other
            ),
        }
    }

    fn expect_chord(shortcut: &str) -> ModifierChord {
        match parse_shortcut(shortcut).unwrap() {
            ParsedShortcut::ModifierChord(chord) => chord,
            other => panic!(
                "expected modifier chord for {:?}, got {:?}",
                shortcut, other
            ),
        }
    }

    #[test]
    fn normalizes_windows_modifier_for_global_hotkey_parser() {
        let shortcut = expect_plugin("Ctrl+Win+Space");

        assert_eq!(shortcut.to_string(), "control+super+Space");
    }

    #[test]
    fn parses_modifier_only_chords() {
        let chord = expect_chord("Ctrl+Shift");
        assert!(chord.ctrl && chord.shift && !chord.alt && !chord.win);
        assert_eq!(chord.label(), "Ctrl+Shift");

        let chord = expect_chord(" control + ALT ");
        assert!(chord.ctrl && chord.alt && !chord.shift && !chord.win);
        assert_eq!(chord.label(), "Ctrl+Alt");

        let chord = expect_chord("Shift+Meta");
        assert!(chord.shift && chord.win && !chord.ctrl && !chord.alt);
        assert_eq!(chord.label(), "Shift+Win");

        let chord = expect_chord("Win");
        assert!(chord.win && !chord.ctrl && !chord.shift && !chord.alt);
        assert_eq!(chord.label(), "Win");
    }

    #[test]
    fn modifier_chord_parsing_is_case_insensitive_and_order_insensitive() {
        assert_eq!(expect_chord("CTRL+SHIFT"), expect_chord("shift+ctrl"));
        assert_eq!(expect_chord("Super+Ctrl"), expect_chord("ctrl+win"));
    }

    #[test]
    fn rejects_empty_and_garbage_shortcuts() {
        assert!(parse_shortcut("").is_err());
        assert!(parse_shortcut("   ").is_err());
        assert!(parse_shortcut("+").is_err());
        assert!(parse_shortcut("Ctrl+").is_err());
        assert!(parse_shortcut("Ctrl+NotAKey").is_err());
        assert!(parse_shortcut("Bogus").is_err());
    }

    #[test]
    fn shortcuts_with_a_key_are_not_modifier_chords() {
        let shortcut = expect_plugin("Ctrl+Shift+T");

        assert_eq!(shortcut.to_string(), "shift+control+KeyT");
    }

    #[test]
    fn normalizes_tilde_aliases_to_backquote() {
        for alias in ["~", "`", "tilde", "Tilde", "Backquote", "BACKQUOTE"] {
            let shortcut = expect_plugin(alias);
            assert_eq!(shortcut.to_string(), "Backquote", "alias {:?}", alias);
        }

        let shortcut = expect_plugin("Ctrl+~");
        assert_eq!(shortcut.to_string(), "control+Backquote");
    }

    #[test]
    fn detects_duplicate_shortcuts() {
        let hotkeys = HotkeySettings {
            hold_to_talk: "Ctrl+Win+Space".to_string(),
            toggle_dictation: "Ctrl+Win+Space".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Win+H".to_string(),
        };

        assert!(validate_hotkeys(&hotkeys).is_err());
    }

    #[test]
    fn detects_duplicate_modifier_chords_across_spellings() {
        let hotkeys = HotkeySettings {
            hold_to_talk: "Ctrl+Shift".to_string(),
            toggle_dictation: "shift+control".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Alt+D".to_string(),
        };

        assert!(validate_hotkeys(&hotkeys).is_err());
    }

    #[test]
    fn modifier_chord_does_not_collide_with_plugin_shortcut() {
        let hotkeys = HotkeySettings {
            hold_to_talk: "Ctrl+Shift".to_string(),
            toggle_dictation: "Ctrl+Shift+T".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Alt+D".to_string(),
        };

        assert!(validate_hotkeys(&hotkeys).is_ok());
    }

    #[test]
    fn default_hotkeys_validate() {
        let settings = AppSettings::default();

        assert!(validate_hotkeys(&settings.hotkeys).is_ok());
        assert!(matches!(
            parse_shortcut(&settings.hotkeys.hold_to_talk),
            Ok(ParsedShortcut::ModifierChord(_))
        ));
        assert!(matches!(
            parse_shortcut(&settings.hotkeys.toggle_dictation),
            Ok(ParsedShortcut::Plugin(_))
        ));
    }
}
