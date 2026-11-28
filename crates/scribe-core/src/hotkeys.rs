use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use global_hotkey::{
    hotkey::{Code, HotKey},
    GlobalHotKeyEvent, HotKeyState,
};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    audio,
    core::Core,
    dictation_worker::{self, Intent},
    error::CommandError,
    events::{CoreEvent, HotkeyRegistrationFailedPayload},
    host::Host,
    settings::{HotkeySettings, TriggerEdge},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyAction {
    HoldToTalk,
    ToggleDictation,
    PasteLastTranscript,
    OpenDashboard,
    DiscardDictation,
    TransformSelection,
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
            "discardDictation" | "discard_dictation" | "discard-dictation" => {
                Ok(Self::DiscardDictation)
            }
            "transformSelection" | "transform_selection" | "transform-selection" => {
                Ok(Self::TransformSelection)
            }
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
            Self::DiscardDictation => "discard_dictation",
            Self::TransformSelection => "transform_selection",
        }
    }

    pub fn shortcut(self, hotkeys: &HotkeySettings) -> &str {
        match self {
            Self::HoldToTalk => &hotkeys.hold_to_talk,
            Self::ToggleDictation => &hotkeys.toggle_dictation,
            Self::PasteLastTranscript => &hotkeys.paste_last_transcript,
            Self::OpenDashboard => &hotkeys.open_dashboard,
            Self::DiscardDictation => &hotkeys.discard_dictation,
            Self::TransformSelection => &hotkeys.transform_selection,
        }
    }

    pub fn set_shortcut(self, hotkeys: &mut HotkeySettings, shortcut: String) {
        match self {
            Self::HoldToTalk => hotkeys.hold_to_talk = shortcut,
            Self::ToggleDictation => hotkeys.toggle_dictation = shortcut,
            Self::PasteLastTranscript => hotkeys.paste_last_transcript = shortcut,
            Self::OpenDashboard => hotkeys.open_dashboard = shortcut,
            Self::DiscardDictation => hotkeys.discard_dictation = shortcut,
            Self::TransformSelection => hotkeys.transform_selection = shortcut,
        }
    }

    /// The key edge this action acts on, or None for Hold-to-Talk, which is
    /// push-to-talk and inherently uses both edges (press starts, release
    /// stops) rather than a single configurable edge.
    pub fn trigger(self, hotkeys: &HotkeySettings) -> Option<TriggerEdge> {
        match self {
            Self::HoldToTalk => None,
            Self::ToggleDictation => Some(hotkeys.toggle_dictation_trigger),
            Self::PasteLastTranscript => Some(hotkeys.paste_last_transcript_trigger),
            Self::OpenDashboard => Some(hotkeys.open_dashboard_trigger),
            Self::DiscardDictation => Some(hotkeys.discard_dictation_trigger),
            Self::TransformSelection => Some(hotkeys.transform_selection_trigger),
        }
    }

    /// Sets this action's trigger edge. Returns false for Hold-to-Talk, which
    /// is push-to-talk and has no single-edge option, without modifying anything.
    pub fn set_trigger(self, hotkeys: &mut HotkeySettings, edge: TriggerEdge) -> bool {
        match self {
            Self::HoldToTalk => false,
            Self::ToggleDictation => {
                hotkeys.toggle_dictation_trigger = edge;
                true
            }
            Self::PasteLastTranscript => {
                hotkeys.paste_last_transcript_trigger = edge;
                true
            }
            Self::OpenDashboard => {
                hotkeys.open_dashboard_trigger = edge;
                true
            }
            Self::DiscardDictation => {
                hotkeys.discard_dictation_trigger = edge;
                true
            }
            Self::TransformSelection => {
                hotkeys.transform_selection_trigger = edge;
                true
            }
        }
    }
}

const HOTKEY_ACTIONS: [HotkeyAction; 6] = [
    HotkeyAction::HoldToTalk,
    HotkeyAction::ToggleDictation,
    HotkeyAction::PasteLastTranscript,
    HotkeyAction::OpenDashboard,
    HotkeyAction::DiscardDictation,
    HotkeyAction::TransformSelection,
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    pub bindings: Vec<HotkeyBindingStatus>,
    /// True when the hold-toggle-key + tap-Q note chord is currently usable:
    /// Toggle acts on release AND its bind is a pollable single key. The UI
    /// uses this to explain why the note chord is on or off.
    pub note_chord_active: bool,
    /// Whether this system has the note chord at all (it needs the Windows
    /// key watcher), so the UI doesn't offer it where it can't work.
    pub note_chord_supported: bool,
    /// Modifier-only hold chords (Ctrl+Shift) need the native key watcher,
    /// which only Windows has; the Hotkeys page explains it there.
    pub hold_release_verification_required: bool,
    /// What delivers the hotkeys: "native" (Windows, macOS, X11 sessions),
    /// "portal" (the desktop's GlobalShortcuts portal on Wayland) or
    /// "x11OnWayland" (X11 grabs on a Wayland desktop without that portal,
    /// which only fire while an X11 app has focus).
    pub backend: &'static str,
    /// The portal's progress when `backend` is "portal".
    pub portal_state: Option<PortalState>,
    /// The portal can open the desktop's shortcut settings for Scribe.
    pub portal_configurable: bool,
    /// On Linux, the command a desktop keyboard shortcut can run to control
    /// Scribe, without the action (`<command> toggle`).
    pub control_command: Option<String>,
}

/// Where the desktop's GlobalShortcuts portal stands (Wayland).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PortalState {
    Starting,
    Unavailable,
    Declined,
    Active,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBindingStatus {
    pub action: HotkeyAction,
    pub shortcut: String,
    pub normalized_shortcut: Option<String>,
    /// Which key edge this bind acts on, or None for Hold-to-Talk (a hold).
    pub trigger: Option<TriggerEdge>,
    pub registered: bool,
    pub error: Option<String>,
    /// Set when the binding is intentionally not registered (Transform
    /// Selection while Local AI is off), so the UI shows why instead of a
    /// registration failure.
    pub disabled_reason: Option<String>,
    /// The keys the desktop assigned, when the GlobalShortcuts portal owns
    /// the hotkeys (as the desktop words them, such as "Ctrl+Alt+Space").
    pub system_trigger: Option<String>,
}

/// One hotkey binding that could not be registered. Serialized into the
/// "scribe:hotkey-registration-failed" event payload, so `action`
/// serializes as the camelCase action name (e.g. "holdToTalk").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyRegistrationFailure {
    pub action: HotkeyAction,
    pub shortcut: String,
    pub message: String,
}

/// The payload this module sent before `CoreEvent` existed; the `events.rs`
/// tests check `CoreEvent::HotkeyRegistrationFailed` against it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct RegistrationFailedEvent<'a> {
    pub(crate) failures: &'a [HotkeyRegistrationFailure],
}

/// Notifies the frontend that one or more hotkey bindings could not be
/// registered. No-op when `failures` is empty.
pub fn emit_registration_failures_with(host: &dyn Host, failures: &[HotkeyRegistrationFailure]) {
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

    host.emit(CoreEvent::HotkeyRegistrationFailed(
        HotkeyRegistrationFailedPayload {
            failures: failures.to_vec(),
        },
    ));
}

/// The `scribe:hotkey-action` event for an action's event name (or
/// `note_dictation` for the note chord).
fn hotkey_action_event(name: &str) -> CoreEvent {
    CoreEvent::HotkeyAction(name.to_string())
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
    Plugin(HotKey),
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

/// The registered hotkeys and their live state, kept in Tauri's managed
/// state. A cheap handle: clones share one [`HotkeyRuntime`] (Tauri state
/// and `core::Core` hold the same one).
#[derive(Clone, Default)]
pub struct HotkeyRuntimeState(Arc<HotkeyRuntime>);

impl std::ops::Deref for HotkeyRuntimeState {
    type Target = HotkeyRuntime;

    fn deref(&self) -> &HotkeyRuntime {
        &self.0
    }
}

#[derive(Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct HotkeyRuntime {
    actions_by_id: Mutex<HashMap<u32, HotkeyAction>>,
    pressed_actions: Mutex<HashSet<HotkeyAction>>,
    /// Per-action trigger edge, cached from settings at registration time so
    /// per-keystroke dispatch never has to read the database.
    triggers: Mutex<HashMap<HotkeyAction, TriggerEdge>>,
    chord_bindings: Mutex<Vec<(ModifierChord, HotkeyAction)>>,
    chord_watcher: Mutex<Option<ChordWatcherHandle>>,
    registration_errors: Mutex<HashMap<HotkeyAction, String>>,
    /// The note-chord key (Q), registered only while the toggle key is
    /// physically held so it never interferes with normal typing.
    note_key: Mutex<Option<HotKey>>,
    /// Whether Q fired during the current toggle hold; a fired chord
    /// suppresses the release-toggle.
    note_chord_fired: AtomicBool,
    /// Whether the toggle key is physically held right now. The async arm
    /// thread checks it after registering so a tap that ended before the
    /// registration completed never leaks the Q grab.
    toggle_held: AtomicBool,
    /// Native watcher that polls the toggle key (press AND release edges).
    /// Plugin events are unreliable for it: an unmodified Backquote hashes
    /// to shortcut id 0, which global_hotkey never delivers, and the plugin
    /// reports no Released state for it either. The plugin registration is
    /// kept solely so the OS suppresses the keystroke.
    toggle_watcher: Mutex<Option<ChordWatcherHandle>>,
    /// True while the toggle watcher owns the toggle key; plugin events for
    /// ToggleDictation are ignored then.
    toggle_watched: AtomicBool,
    /// Mirrors the Local AI switch: Transform Selection is only registered
    /// while it is on, so its chord is free for other apps otherwise.
    transform_enabled: AtomicBool,
    /// The desktop's GlobalShortcuts portal delivers the hotkeys (native
    /// Wayland); nothing is registered with the global-shortcut plugin then.
    portal_owns_hotkeys: AtomicBool,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl HotkeyRuntime {
    fn replace_bindings(&self, actions_by_id: HashMap<u32, HotkeyAction>) {
        if let Ok(mut bindings) = self.actions_by_id.lock() {
            *bindings = actions_by_id;
        }

        if let Ok(mut pressed) = self.pressed_actions.lock() {
            pressed.clear();
        }
    }

    fn action_for(&self, shortcut: &HotKey) -> Option<HotkeyAction> {
        self.actions_by_id
            .lock()
            .ok()
            .and_then(|bindings| bindings.get(&shortcut.id()).copied())
    }

    fn store_triggers(&self, triggers: HashMap<HotkeyAction, TriggerEdge>) {
        if let Ok(mut stored) = self.triggers.lock() {
            *stored = triggers;
        }
    }

    /// The cached trigger edge for `action`, defaulting to Press if absent
    /// (Hold-to-Talk has no entry and never calls this).
    fn trigger_for(&self, action: HotkeyAction) -> TriggerEdge {
        self.triggers
            .lock()
            .ok()
            .and_then(|triggers| triggers.get(&action).copied())
            .unwrap_or(TriggerEdge::Press)
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

    /// Stores the registered note key, unless the toggle hold already ended;
    /// returns false in that case so the caller unregisters it again.
    fn arm_note_key(&self, shortcut: HotKey) -> bool {
        if let Ok(mut guard) = self.note_key.lock() {
            if self.toggle_held.load(Ordering::SeqCst) {
                *guard = Some(shortcut);
                return true;
            }
        }
        false
    }

    fn disarm_note_key(&self) -> Option<HotKey> {
        self.note_key.lock().ok().and_then(|mut guard| guard.take())
    }

    fn is_note_key(&self, id: u32) -> bool {
        self.note_key
            .lock()
            .ok()
            .and_then(|guard| guard.map(|shortcut| shortcut.id() == id))
            .unwrap_or(false)
    }

    /// Marks the chord as fired; returns true only for the first firing of
    /// the current toggle hold (filters key autorepeat).
    fn mark_note_chord_fired(&self) -> bool {
        !self.note_chord_fired.swap(true, Ordering::SeqCst)
    }

    fn note_chord_fired(&self) -> bool {
        self.note_chord_fired.load(Ordering::SeqCst)
    }

    fn registration_error_for(&self, action: HotkeyAction) -> Option<String> {
        self.registration_errors
            .lock()
            .ok()
            .and_then(|errors| errors.get(&action).cloned())
    }
}

/// Registers the hotkeys. `runtime` is the hotkey runtime the shell created
/// and already manages (and handed to the `Core`), so a key press that
/// arrives the moment a hotkey is registered finds it. The shell manages the
/// `Core` before calling this; without it, nothing is registered and the
/// no-engine error comes back.
pub fn setup(
    app: &AppHandle,
    runtime: &HotkeyRuntimeState,
    hotkeys: &HotkeySettings,
    transform_enabled: bool,
) -> Result<(), CommandError> {
    let Some(core) = app.try_state::<Core>() else {
        log::warn!("Could not register the hotkeys: the engine is not ready yet");
        return Err(crate::host::no_main_window());
    };
    debug_assert!(
        Arc::ptr_eq(&runtime.0, &core.hotkey_runtime().0),
        "setup must fill in the Core's own hotkey runtime"
    );
    setup_with(core.inner(), hotkeys, transform_enabled)
}

/// [`setup`] on the engine handle; fills in `core`'s hotkey runtime and
/// registers the hotkeys through the host. The shell calls it on its main
/// thread before the event loop starts (the one registration allowed there).
pub fn setup_with(
    core: &Core,
    hotkeys: &HotkeySettings,
    transform_enabled: bool,
) -> Result<(), CommandError> {
    let runtime = core.hotkey_runtime();
    runtime
        .transform_enabled
        .store(transform_enabled, Ordering::SeqCst);
    validate_hotkeys(hotkeys)?;
    #[cfg(target_os = "linux")]
    wayland::start_portal(core, hotkeys);

    let failures = register_hotkey_set_with(core, hotkeys);
    if portal_owns_hotkeys(runtime) {
        log::info!("Hotkeys are delivered by the desktop's GlobalShortcuts portal");
        return Ok(());
    }
    let active = HOTKEY_ACTIONS
        .into_iter()
        .filter(|action| action_enabled(*action, transform_enabled))
        .count();
    log::info!(
        "Registered {} of {} hotkey bindings",
        active - failures.len(),
        active
    );
    emit_registration_failures_with(core.host().as_ref(), &failures);

    Ok(())
}

/// A global-shortcut plugin event. Ignored until the `Core` exists (the
/// shell's handler already drops events before then).
pub fn handle_shortcut(app: &AppHandle, shortcut: &HotKey, event: GlobalHotKeyEvent) {
    let Some(core) = app.try_state::<Core>() else {
        log::warn!("Ignored a hotkey: the engine is not ready yet");
        return;
    };
    handle_shortcut_with(core.inner(), shortcut, event);
}

/// [`handle_shortcut`] on the engine handle.
pub fn handle_shortcut_with(core: &Core, shortcut: &HotKey, event: GlobalHotKeyEvent) {
    let runtime = core.hotkey_runtime();

    if runtime.is_note_key(shortcut.id()) {
        #[cfg(windows)]
        if event.state == HotKeyState::Pressed && runtime.mark_note_chord_fired() {
            handle_note_chord_with(core);
        }
        return;
    }

    let Some(action) = runtime.action_for(shortcut) else {
        return;
    };

    // The native toggle watcher owns the toggle key when it could be
    // mapped to a pollable virtual key; the plugin registration only exists
    // to suppress the keystroke then.
    if action == HotkeyAction::ToggleDictation && runtime.toggle_watched.load(Ordering::SeqCst) {
        return;
    }

    match event.state {
        HotKeyState::Pressed => {
            if !runtime.mark_pressed_once(action) {
                return;
            }
            handle_press_edge_with(core, action);
        }
        HotKeyState::Released => {
            if !runtime.mark_released_once(action) {
                return;
            }
            handle_release_edge_with(core, action);
        }
    }
}

/// Stops any running toggle-key watcher and starts a fresh one when the toggle
/// binding maps to a pollable virtual key (needed in both modes — modifier-less
/// keys deliver no plugin events). In Release mode the watcher arms the Q note
/// chord on key down and runs the toggle on key up (unless Q fired). In Press
/// mode it runs the toggle on key down and skips the note chord entirely.
fn configure_toggle_watcher_with(core: &Core, vk: Option<i32>, trigger: TriggerEdge) {
    let runtime = core.hotkey_runtime();

    if let Ok(mut guard) = runtime.toggle_watcher.lock() {
        if let Some(watcher) = guard.take() {
            watcher.stop.store(true, Ordering::SeqCst);
            let _ = watcher.thread.join();
        }
    }
    runtime.toggle_watched.store(false, Ordering::SeqCst);

    #[cfg(windows)]
    if let Some(vk) = vk {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_core = core.clone();
        let spawned = std::thread::Builder::new()
            .name("scribe-toggle-watcher".to_string())
            .spawn(move || run_toggle_watcher(thread_core, vk, trigger, thread_stop));
        match spawned {
            Ok(thread) => {
                runtime.toggle_watched.store(true, Ordering::SeqCst);
                if let Ok(mut guard) = runtime.toggle_watcher.lock() {
                    *guard = Some(ChordWatcherHandle { stop, thread });
                }
                log::info!(
                    "Toggle key watcher active (vk 0x{:X}, trigger {:?})",
                    vk,
                    trigger
                );
            }
            Err(error) => {
                log::error!("Failed to start toggle key watcher: {}", error);
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (vk, trigger);
    }
}

/// Polls the toggle key. A lasting thread, so it owns its handle.
#[cfg(windows)]
fn run_toggle_watcher(core: Core, vk: i32, trigger: TriggerEdge, stop: Arc<AtomicBool>) {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    let mut was_down = false;
    while !stop.load(Ordering::SeqCst) {
        let down = (unsafe { GetAsyncKeyState(vk) } as u16) & 0x8000 != 0;
        if down && !was_down {
            match trigger {
                TriggerEdge::Release => on_toggle_key_down_with(&core),
                TriggerEdge::Press => on_toggle_key_press_with(&core),
            }
        } else if !down && was_down && trigger == TriggerEdge::Release {
            on_toggle_key_up_with(&core);
        }
        was_down = down;
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Never leave a Q grab behind when the watcher is being replaced (only the
    // Release path ever arms it).
    if was_down && trigger == TriggerEdge::Release {
        disarm_note_chord_with(&core);
    }
}

/// Release-mode key down: arm the Q note chord and signal the frontend. The
/// actual toggle waits for key up so a hold-and-tap-Q makes a note instead.
#[cfg(windows)]
fn on_toggle_key_down_with(core: &Core) {
    let runtime = core.hotkey_runtime();
    core.emit(hotkey_action_event(
        HotkeyAction::ToggleDictation.event_name(),
    ));
    runtime.toggle_held.store(true, Ordering::SeqCst);
    runtime.note_chord_fired.store(false, Ordering::SeqCst);
    let core = core.clone();
    std::thread::spawn(move || arm_note_chord_with(&core));
}

#[cfg(windows)]
fn on_toggle_key_up_with(core: &Core) {
    let fired = disarm_note_chord_with(core);
    log::info!("Toggle key released (note chord fired: {})", fired);
    if !fired {
        dictation_worker::submit_with(core, Intent::Toggle);
    }
}

/// Press-mode key down: toggle immediately. No note chord (there is no hold
/// window when the toggle fires on press).
/// Compiled everywhere so its tests run on every platform; only the Windows
/// toggle watcher calls it.
#[cfg_attr(not(windows), allow(dead_code))]
fn on_toggle_key_press_with(target: &(impl ActionTarget + ?Sized)) {
    log::info!("Toggle key pressed (trigger Press)");
    target.host().emit(hotkey_action_event(
        HotkeyAction::ToggleDictation.event_name(),
    ));
    target.submit(Intent::Toggle);
}

/// Maps a shortcut key code to the Windows virtual key the toggle watcher
/// polls. Covers the keys a toggle hotkey is realistically bound to; None
/// falls back to toggle-on-press without the note chord.
#[cfg(windows)]
fn code_to_vk(code: Code) -> Option<i32> {
    let vk = match code {
        Code::Backquote => 0xC0,
        Code::KeyA => 0x41,
        Code::KeyB => 0x42,
        Code::KeyC => 0x43,
        Code::KeyD => 0x44,
        Code::KeyE => 0x45,
        Code::KeyF => 0x46,
        Code::KeyG => 0x47,
        Code::KeyH => 0x48,
        Code::KeyI => 0x49,
        Code::KeyJ => 0x4A,
        Code::KeyK => 0x4B,
        Code::KeyL => 0x4C,
        Code::KeyM => 0x4D,
        Code::KeyN => 0x4E,
        Code::KeyO => 0x4F,
        Code::KeyP => 0x50,
        Code::KeyQ => 0x51,
        Code::KeyR => 0x52,
        Code::KeyS => 0x53,
        Code::KeyT => 0x54,
        Code::KeyU => 0x55,
        Code::KeyV => 0x56,
        Code::KeyW => 0x57,
        Code::KeyX => 0x58,
        Code::KeyY => 0x59,
        Code::KeyZ => 0x5A,
        Code::Digit0 => 0x30,
        Code::Digit1 => 0x31,
        Code::Digit2 => 0x32,
        Code::Digit3 => 0x33,
        Code::Digit4 => 0x34,
        Code::Digit5 => 0x35,
        Code::Digit6 => 0x36,
        Code::Digit7 => 0x37,
        Code::Digit8 => 0x38,
        Code::Digit9 => 0x39,
        Code::F1 => 0x70,
        Code::F2 => 0x71,
        Code::F3 => 0x72,
        Code::F4 => 0x73,
        Code::F5 => 0x74,
        Code::F6 => 0x75,
        Code::F7 => 0x76,
        Code::F8 => 0x77,
        Code::F9 => 0x78,
        Code::F10 => 0x79,
        Code::F11 => 0x7A,
        Code::F12 => 0x7B,
        Code::Space => 0x20,
        Code::Minus => 0xBD,
        Code::Equal => 0xBB,
        Code::BracketLeft => 0xDB,
        Code::BracketRight => 0xDD,
        Code::Backslash => 0xDC,
        Code::Semicolon => 0xBA,
        Code::Quote => 0xDE,
        Code::Comma => 0xBC,
        Code::Period => 0xBE,
        Code::Slash => 0xBF,
        _ => return None,
    };
    Some(vk)
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

/// Whether `action` should be registered at all. Transform Selection is only
/// bound while Local AI is on; every other action always is.
fn action_enabled(action: HotkeyAction, transform_enabled: bool) -> bool {
    action != HotkeyAction::TransformSelection || transform_enabled
}

fn transform_enabled(runtime: &HotkeyRuntime) -> bool {
    runtime.transform_enabled.load(Ordering::SeqCst)
}

/// Records whether Local AI is on, which decides whether Transform Selection
/// gets registered. Returns the previous value. Callers re-register the hotkey
/// set (`replace_hotkeys_with`) afterwards for the change to take effect.
pub fn set_transform_enabled_with(core: &Core, enabled: bool) -> bool {
    swap_transform_enabled(core.hotkey_runtime(), enabled)
}

fn swap_transform_enabled(runtime: &HotkeyRuntime, enabled: bool) -> bool {
    runtime.transform_enabled.swap(enabled, Ordering::SeqCst)
}

/// Swaps the registered hotkey set. Registration is best-effort per binding:
/// the returned vector lists the bindings in `next` that could not be
/// registered (working bindings stay registered). Returns Err only when
/// `next` fails validation, in which case nothing is changed. Runs on an
/// engine thread, never the shell's main thread (see [`Host::register_shortcut`]).
///
/// `_app` is unused: `settings_store` (the `data` lane) still passes it, and
/// P1-4 step 4 removes it with that caller.
pub fn replace_hotkeys_with(
    _app: &AppHandle,
    core: &Core,
    previous: &HotkeySettings,
    next: &HotkeySettings,
) -> Result<Vec<HotkeyRegistrationFailure>, CommandError> {
    validate_hotkeys(next)?;

    unregister_hotkey_set(core.host().as_ref(), previous);

    Ok(register_hotkey_set_with(core, next))
}

/// What [`register_bindings`] registered and what the watchers need.
struct Registration {
    failures: Vec<HotkeyRegistrationFailure>,
    chord_bindings: Vec<(ModifierChord, HotkeyAction)>,
    toggle_vk: Option<i32>,
}

/// Registers every binding in `hotkeys`: shortcuts with a key through the
/// host, modifier-only chords through the Windows chord watcher. Each binding
/// is attempted independently; failures are collected and returned while
/// every binding that did register stays registered.
fn register_hotkey_set_with(
    core: &Core,
    hotkeys: &HotkeySettings,
) -> Vec<HotkeyRegistrationFailure> {
    let registration = register_bindings(core.host().as_ref(), core.hotkey_runtime(), hotkeys);

    configure_chord_watcher_with(core, registration.chord_bindings);

    // The toggle key gets a native press/release watcher whenever its binding
    // maps to a pollable virtual key (see configure_toggle_watcher) - needed in
    // both modes, since modifier-less keys deliver no plugin events at all. The
    // watcher fires on the edge the toggle trigger selects.
    configure_toggle_watcher_with(
        core,
        registration.toggle_vk,
        hotkeys.toggle_dictation_trigger,
    );

    registration.failures
}

/// The host and runtime half of [`register_hotkey_set_with`]: registers the
/// shortcuts with a key and records the bindings, and says which chords and
/// toggle key the watchers must poll.
fn register_bindings(
    host: &dyn Host,
    runtime: &HotkeyRuntime,
    hotkeys: &HotkeySettings,
) -> Registration {
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

    if portal_owns_hotkeys(runtime) {
        // The desktop owns the keys; nothing to grab. Bindings changed here
        // only become the suggestion for the next time the desktop asks.
        // The watchers are stopped (no chords, no toggle key).
        runtime.replace_bindings(HashMap::new());
        runtime.store_registration_errors(&[]);
        return Registration {
            failures,
            chord_bindings,
            toggle_vk: None,
        };
    }

    let transform_enabled = transform_enabled(runtime);
    for action in HOTKEY_ACTIONS {
        if !action_enabled(action, transform_enabled) {
            continue;
        }
        let shortcut_text = action.shortcut(hotkeys);

        match parse_shortcut(shortcut_text) {
            Ok(ParsedShortcut::Plugin(shortcut)) => match host.register_shortcut(shortcut) {
                Ok(()) => {
                    log::debug!(
                        "Registered {:?} as shortcut id={} key={:?} mods={:?}",
                        action,
                        shortcut.id(),
                        shortcut.key,
                        shortcut.mods
                    );
                    actions_by_id.insert(shortcut.id(), action);
                }
                Err(error) => {
                    log::warn!(
                        "Registering {:?} ({}) failed: {}",
                        action,
                        shortcut_text,
                        error
                    );
                    record_failure(
                        action,
                        shortcut_text,
                        CommandError::hotkey_registration_failed(shortcut_text).message,
                    )
                }
            },
            Ok(ParsedShortcut::ModifierChord(chord)) => {
                if cfg!(windows) {
                    chord_bindings.push((chord, action));
                } else {
                    record_failure(
                        action,
                        shortcut_text,
                        format!(
                            "Could not register {}. {}",
                            shortcut_text,
                            modifier_only_unsupported()
                        ),
                    );
                }
            }
            Err(error) => record_failure(action, shortcut_text, error.message),
        }
    }

    let triggers = HOTKEY_ACTIONS
        .into_iter()
        .filter_map(|action| action.trigger(hotkeys).map(|edge| (action, edge)))
        .collect::<HashMap<_, _>>();

    runtime.replace_bindings(actions_by_id);
    runtime.store_registration_errors(&failures);
    runtime.store_triggers(triggers);

    let toggle_vk = match parse_shortcut(HotkeyAction::ToggleDictation.shortcut(hotkeys)) {
        Ok(ParsedShortcut::Plugin(shortcut)) => toggle_vk_for(shortcut),
        _ => None,
    };

    Registration {
        failures,
        chord_bindings,
        toggle_vk,
    }
}

#[cfg(windows)]
fn toggle_vk_for(shortcut: HotKey) -> Option<i32> {
    // Only modifier-less bindings can be polled as a single key.
    if !shortcut.mods.is_empty() {
        return None;
    }
    code_to_vk(shortcut.key)
}

#[cfg(not(windows))]
fn toggle_vk_for(_shortcut: HotKey) -> Option<i32> {
    None
}

/// Stops any running chord watcher thread, stores the new chord bindings (so
/// status() can report them as registered), and starts a fresh watcher thread
/// when there is at least one chord to monitor. Called on initial
/// registration and on every rebind/reset/replace.
fn configure_chord_watcher_with(core: &Core, bindings: Vec<(ModifierChord, HotkeyAction)>) {
    let runtime = core.hotkey_runtime();

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
        let thread_core = core.clone();
        let spawned = std::thread::Builder::new()
            .name("scribe-chord-watcher".to_string())
            .spawn(move || windows_chord::run_watcher(thread_core, bindings, thread_stop));

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

/// Where each hotkey stands, for the Hotkeys page.
pub fn status_with(core: &Core, hotkeys: &HotkeySettings) -> Result<HotkeyStatus, CommandError> {
    Ok(status_in(
        core.host().as_ref(),
        core.hotkey_runtime(),
        hotkeys,
    ))
}

/// [`status_with`] over the parts it reads: the portal's status when the
/// portal owns the hotkeys, else what the host holds.
fn status_in(host: &dyn Host, runtime: &HotkeyRuntime, hotkeys: &HotkeySettings) -> HotkeyStatus {
    #[cfg(target_os = "linux")]
    if portal_owns_hotkeys(runtime) {
        return wayland::status(runtime, hotkeys);
    }
    native_status(runtime, hotkeys, |shortcut| {
        host.is_shortcut_registered(shortcut)
    })
}

/// The status of hotkeys the app grabs itself (not the portal's), given
/// whether the host holds a shortcut.
fn native_status(
    runtime: &HotkeyRuntime,
    hotkeys: &HotkeySettings,
    is_registered: impl Fn(HotKey) -> bool,
) -> HotkeyStatus {
    let transform_enabled = transform_enabled(runtime);
    let mut bindings = Vec::new();

    for action in HOTKEY_ACTIONS {
        let shortcut = action.shortcut(hotkeys);
        if !action_enabled(action, transform_enabled) {
            bindings.push(HotkeyBindingStatus {
                action,
                shortcut: shortcut.to_string(),
                normalized_shortcut: match parse_shortcut(shortcut) {
                    Ok(ParsedShortcut::Plugin(parsed)) => Some(parsed.to_string()),
                    Ok(ParsedShortcut::ModifierChord(chord)) => Some(chord.label()),
                    Err(_) => None,
                },
                trigger: action.trigger(hotkeys),
                registered: false,
                error: None,
                disabled_reason: Some(
                    "Off while Local AI is off. Turn on Local AI to use it.".to_string(),
                ),
                system_trigger: None,
            });
            continue;
        }
        let registration_error = runtime.registration_error_for(action);
        let (normalized_shortcut, registered, error) = match parse_shortcut(shortcut) {
            Ok(ParsedShortcut::Plugin(parsed)) => {
                let registered = is_registered(parsed);
                let error = if registered { None } else { registration_error };

                (Some(parsed.to_string()), registered, error)
            }
            Ok(ParsedShortcut::ModifierChord(chord)) => {
                let registered = runtime.chord_is_registered(chord);
                let error = if registered {
                    None
                } else {
                    registration_error.or_else(|| {
                        (!cfg!(windows)).then(|| modifier_only_unsupported().to_string())
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
            trigger: action.trigger(hotkeys),
            registered,
            error,
            disabled_reason: None,
            system_trigger: None,
        });
    }

    HotkeyStatus {
        bindings,
        note_chord_active: note_chord_active(hotkeys),
        note_chord_supported: cfg!(windows),
        hold_release_verification_required: cfg!(windows),
        backend: grab_backend(),
        portal_state: None,
        portal_configurable: false,
        control_command: control_command(),
    }
}

/// "native", or "x11OnWayland" when X11 grabs stand in on a Wayland desktop.
fn grab_backend() -> &'static str {
    #[cfg(target_os = "linux")]
    if crate::platform::session_kind() == crate::platform::SessionKind::Wayland {
        return "x11OnWayland";
    }
    "native"
}

fn control_command() -> Option<String> {
    #[cfg(target_os = "linux")]
    return crate::platform::control_command();
    #[cfg(not(target_os = "linux"))]
    None
}

/// Why a modifier-only chord can't be registered on this system.
fn modifier_only_unsupported() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS needs a key in the shortcut, such as Ctrl+Alt+Z."
    } else {
        "Linux needs a key in the shortcut, such as Ctrl+Alt+Space."
    }
}

fn portal_owns_hotkeys(runtime: &HotkeyRuntime) -> bool {
    runtime.portal_owns_hotkeys.load(Ordering::SeqCst)
}

/// The hold-toggle-key + tap-Q note chord works only when the toggle acts on
/// release (so the key can be held) AND its bind is a pollable single key (the
/// watcher that arms Q polls a virtual key). Always false off Windows, where
/// `toggle_vk_for` returns None.
fn note_chord_active(hotkeys: &HotkeySettings) -> bool {
    hotkeys.toggle_dictation_trigger == TriggerEdge::Release
        && matches!(
            parse_shortcut(HotkeyAction::ToggleDictation.shortcut(hotkeys)),
            Ok(ParsedShortcut::Plugin(shortcut)) if toggle_vk_for(shortcut).is_some()
        )
}

/// What a hotkey edge acts through: the engine's [`Core`] in the app, and a
/// recorder in the tests (no test can build a `Core` yet).
pub(crate) trait ActionTarget {
    /// Where the events go and the main window is.
    fn host(&self) -> &dyn Host;
    /// The registered hotkeys' live state.
    fn runtime(&self) -> &HotkeyRuntime;
    /// Queues dictation work on the dictation worker and returns.
    fn submit(&self, intent: Intent);
    /// Whether the Open Dashboard hotkey hides a dashboard already in front
    /// (the setting; true when it can't be read).
    fn dashboard_hotkey_toggles(&self) -> bool;
}

impl ActionTarget for Core {
    fn host(&self) -> &dyn Host {
        Core::host(self).as_ref()
    }

    fn runtime(&self) -> &HotkeyRuntime {
        self.hotkey_runtime()
    }

    fn submit(&self, intent: Intent) {
        dictation_worker::submit_with(self, intent);
    }

    fn dashboard_hotkey_toggles(&self) -> bool {
        self.backend()
            .db()
            .and_then(|db| db.get_settings())
            .map(|settings| settings.dashboard_hotkey_toggles)
            .unwrap_or(true)
    }
}

/// Press edge of a binding. Hold-to-talk always starts here (it is push-to-talk
/// and uses both edges); single-shot actions fire only when their configured
/// trigger is Press. Reached by the plugin shortcut handler (on the main
/// thread) and the modifier-chord watcher, so dictation actions only queue
/// work on the dictation worker and return.
pub(crate) fn handle_press_edge_with(target: &(impl ActionTarget + ?Sized), action: HotkeyAction) {
    if action == HotkeyAction::HoldToTalk {
        target.host().emit(hotkey_action_event(action.event_name()));
        target.submit(Intent::StartHold);
        return;
    }

    if target.runtime().trigger_for(action) == TriggerEdge::Press {
        run_action_with(target, action);
    }
}

/// Release edge of a binding. Hold-to-talk always stops here; single-shot
/// actions fire only when their configured trigger is Release.
pub(crate) fn handle_release_edge_with(
    target: &(impl ActionTarget + ?Sized),
    action: HotkeyAction,
) {
    if action == HotkeyAction::HoldToTalk {
        target.submit(Intent::Stop);
        return;
    }

    if target.runtime().trigger_for(action) == TriggerEdge::Release {
        run_action_with(target, action);
    }
}

/// Performs a single-shot action (toggle / paste / open dashboard) and emits
/// the frontend hotkey-action event. Called on whichever edge the action's
/// trigger selects. Hold-to-talk is not a single-shot action and is ignored.
pub(crate) fn run_action_with(target: &(impl ActionTarget + ?Sized), action: HotkeyAction) {
    target.host().emit(hotkey_action_event(action.event_name()));

    let result = match action {
        HotkeyAction::ToggleDictation => {
            // Clear the pressed mark so a toggle bound to a key that delivers
            // no Released event (modifier-less keys, modifier chords) still
            // re-fires on the next press.
            target
                .runtime()
                .mark_released_once(HotkeyAction::ToggleDictation);
            target.submit(Intent::Toggle);
            Ok(())
        }
        HotkeyAction::PasteLastTranscript => {
            target.submit(Intent::PasteLast);
            Ok(())
        }
        HotkeyAction::OpenDashboard => {
            if target.dashboard_hotkey_toggles() {
                target.host().toggle_dashboard()
            } else {
                target.host().open_dashboard(None)
            }
        }
        // Discards the in-progress recording without transcribing. A no-op when
        // nothing is recording, so a stray press is harmless.
        HotkeyAction::DiscardDictation => {
            target.submit(Intent::Cancel);
            Ok(())
        }
        // Voice Transform Selection (tap to start, tap again, or let silence
        // auto-stop end it, to finish). Starting captures the focused app's
        // selection and records a spoken *instruction*; stopping flows through
        // the normal stop and transcribe path, but because the recording is
        // flagged `is_transform` the transcription is routed to the transform
        // engine (rewrite the captured selection in place) instead of being
        // saved or pasted as dictation. The typed-instruction Dashboard panel
        // remains as an alternative.
        HotkeyAction::TransformSelection => {
            target.submit(Intent::ToggleTransform);
            Ok(())
        }
        HotkeyAction::HoldToTalk => return,
    };

    if let Err(error) = result {
        audio::emit_recording_error_with(target.host(), error);
    }
}

/// The key grabbed while the toggle key is held; pressing it starts (or
/// stops) a note dictation instead of the normal toggle.
#[cfg_attr(not(windows), allow(dead_code))]
const NOTE_CHORD_CODE: Code = Code::KeyQ;

/// Grabs Q for the current toggle hold. Runs on its own thread (the host may
/// wait for the shell's main thread to register).
#[cfg(windows)]
fn arm_note_chord_with(core: &Core) {
    arm_note_chord_in(core.host().as_ref(), core.hotkey_runtime());
}

/// [`arm_note_chord_with`] over the parts it uses. Compiled everywhere so its
/// tests run on every platform; only Windows calls it.
#[cfg_attr(not(windows), allow(dead_code))]
fn arm_note_chord_in(host: &dyn Host, runtime: &HotkeyRuntime) {
    let shortcut = HotKey::new(None, NOTE_CHORD_CODE);
    match host.register_shortcut(shortcut) {
        Ok(()) => {
            log::debug!("Note chord key grabbed");
            if !runtime.arm_note_key(shortcut) {
                // The tap ended before registration finished: release the
                // grab immediately so Q keeps typing normally.
                release_note_key(host, shortcut);
            }
        }
        Err(error) => log::warn!(
            "Note chord unavailable: could not grab Q during the toggle hold: {}",
            error
        ),
    }
}

/// Releases the Q grab, logging when the host refuses.
#[cfg_attr(not(windows), allow(dead_code))]
fn release_note_key(host: &dyn Host, shortcut: HotKey) {
    if let Err(error) = host.unregister_shortcut(shortcut) {
        log::error!("Could not release the note-chord key grab: {}", error);
    }
}

/// Ends the toggle hold and releases the Q grab on a background thread
/// (plugin register/unregister deadlocks when run on the main thread, but
/// is fine from worker threads - the rebind command path proves it);
/// returns whether the note chord fired.
#[cfg(windows)]
fn disarm_note_chord_with(core: &Core) -> bool {
    let runtime = core.hotkey_runtime();
    runtime.toggle_held.store(false, Ordering::SeqCst);
    if let Some(shortcut) = runtime.disarm_note_key() {
        let host = Arc::clone(core.host());
        std::thread::spawn(move || release_note_key(host.as_ref(), shortcut));
    }
    runtime.note_chord_fired()
}

/// The note chord fired: start (or stop) a note dictation. Compiled
/// everywhere so its tests run on every platform; only Windows calls it.
#[cfg_attr(not(windows), allow(dead_code))]
fn handle_note_chord_with(target: &(impl ActionTarget + ?Sized)) {
    log::info!("Note chord (Q) pressed during toggle hold");
    target.host().emit(hotkey_action_event("note_dictation"));
    target.submit(Intent::ToggleNote);
}

/// Captures the focused app's current selection and starts a transform
/// recording for the spoken instruction. Aborts with a friendly notice when
/// Local AI is off or nothing is selected, so the user never records an
/// instruction for an empty selection. Runs on the dictation worker.
/// Without the `Core` nothing is captured and the no-engine error comes back.
pub(crate) fn start_transform(app: &AppHandle) -> Result<(), CommandError> {
    match app.try_state::<Core>() {
        Some(core) => start_transform_with(core.inner()),
        None => {
            log::warn!("Could not start Transform Selection: the engine is not ready yet");
            Err(crate::host::no_main_window())
        }
    }
}

/// [`start_transform`] on the engine handle.
pub(crate) fn start_transform_with(core: &Core) -> Result<(), CommandError> {
    // Second guard behind registration: never copy a selection while Local AI
    // is off.
    let settings = core.backend().db()?.get_settings()?;
    if let Err(error) = crate::selection_transform::ensure_enabled(&settings) {
        core.emit(CoreEvent::SelectionTransformFailed(error.message));
        return Ok(());
    }

    // An empty or non-text selection drops `captured`, which restores the
    // user's clipboard; a copy that changed nothing comes back as None.
    let captured = crate::selection_transform::capture_selection()?
        .filter(|captured| !captured.selection.trim().is_empty());
    let Some(captured) = captured else {
        core.emit(CoreEvent::SelectionTransformFailed(
            "Highlight some text first, then trigger Transform Selection.".to_string(),
        ));
        return Ok(());
    };

    let request = audio::StartRecordingRequest {
        is_transform: true,
        ..Default::default()
    };
    // If the recording can't start, `captured` drops here and the clipboard
    // is put back.
    audio::start_recording_with(core, Some(request), true)?;
    core.backend().set_pending_transform(captured);
    Ok(())
}

/// Releases every shortcut with a key in `hotkeys` that the host holds. Runs
/// on an engine thread (see [`Host::unregister_shortcut`]).
fn unregister_hotkey_set(host: &dyn Host, hotkeys: &HotkeySettings) {
    let shortcuts = HOTKEY_ACTIONS
        .into_iter()
        .filter_map(|action| match parse_shortcut(action.shortcut(hotkeys)) {
            Ok(ParsedShortcut::Plugin(shortcut)) => Some(shortcut),
            _ => None,
        })
        .collect::<Vec<_>>();

    unregister_shortcuts(host, &shortcuts);
}

/// Asks first: the plugin may refuse to release a shortcut it does not hold.
fn unregister_shortcuts(host: &dyn Host, shortcuts: &[HotKey]) {
    for shortcut in shortcuts {
        if host.is_shortcut_registered(*shortcut) {
            let _ = host.unregister_shortcut(*shortcut);
        }
    }
}

pub fn parse_shortcut(shortcut: &str) -> Result<ParsedShortcut, CommandError> {
    if let Some(chord) = parse_modifier_chord(shortcut) {
        return Ok(ParsedShortcut::ModifierChord(chord));
    }

    let normalized = normalize_shortcut(shortcut);
    HotKey::from_str(&normalized)
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
fn dispatch_chord_pressed_with(core: &Core, action: HotkeyAction) {
    if !core.hotkey_runtime().mark_pressed_once(action) {
        return;
    }
    handle_press_edge_with(core, action);
}

#[cfg(windows)]
fn dispatch_chord_released_with(core: &Core, action: HotkeyAction) {
    if !core.hotkey_runtime().mark_released_once(action) {
        return;
    }
    handle_release_edge_with(core, action);
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

    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
        VK_PACKET, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };

    use super::{
        dispatch_chord_pressed_with, dispatch_chord_released_with, HotkeyAction, ModifierChord,
    };
    use crate::core::Core;

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
        /// The press edge has fired; waiting for a modifier release.
        Active,
        /// Aborted or finished; waiting for all chord modifiers to be
        /// released before re-arming.
        Suppressed,
    }

    /// Polls the modifier chords. A lasting thread, so it owns its handle.
    pub(super) fn run_watcher(
        core: Core,
        bindings: Vec<(ModifierChord, HotkeyAction)>,
        stop: Arc<AtomicBool>,
    ) {
        let mut states = vec![ChordState::Idle; bindings.len()];

        while !stop.load(Ordering::SeqCst) {
            let snapshot = ModifierSnapshot::capture();

            for (index, (chord, action)) in bindings.iter().enumerate() {
                states[index] = step(&core, *chord, *action, states[index], snapshot);
            }

            thread::sleep(POLL_INTERVAL);
        }
    }

    fn step(
        core: &Core,
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
                    dispatch_chord_pressed_with(core, action);
                    ChordState::Active
                } else {
                    ChordState::Arming(deadline)
                }
            }
            ChordState::Active => {
                if !snapshot.holds_all(chord) {
                    dispatch_chord_released_with(core, action);
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

/// Hotkeys through the desktop's GlobalShortcuts portal on Wayland.
#[cfg(target_os = "linux")]
mod wayland {
    use std::sync::{atomic::Ordering, Arc};

    use super::{
        action_enabled, handle_press_edge_with, handle_release_edge_with, run_action_with,
        transform_enabled, HotkeyAction, HotkeyBindingStatus, HotkeyRuntime, HotkeyStatus,
        PortalState, HOTKEY_ACTIONS,
    };
    use crate::{
        core::Core,
        platform::{
            shortcuts::{self, PortalShortcutsState, ShortcutSpec},
            SessionKind,
        },
        settings::HotkeySettings,
    };

    /// What the desktop's dialog lists for each action.
    fn description(action: HotkeyAction) -> &'static str {
        match action {
            HotkeyAction::HoldToTalk => "Hold to talk (Scribe)",
            HotkeyAction::ToggleDictation => "Start or stop dictation (Scribe)",
            HotkeyAction::PasteLastTranscript => "Paste the last dictation (Scribe)",
            HotkeyAction::OpenDashboard => "Show Scribe",
            HotkeyAction::DiscardDictation => "Cancel the recording (Scribe)",
            HotkeyAction::TransformSelection => "Transform the selected text (Scribe)",
        }
    }

    pub(super) fn specs(hotkeys: &HotkeySettings) -> Vec<ShortcutSpec> {
        HOTKEY_ACTIONS
            .into_iter()
            .map(|action| ShortcutSpec {
                id: action.event_name().to_string(),
                description: description(action).to_string(),
                preferred_trigger: shortcuts::xdg_trigger(action.shortcut(hotkeys)),
            })
            .collect()
    }

    /// On a Wayland session, hands the hotkeys to the GlobalShortcuts portal
    /// when the desktop has one. Without it, the X11 grabs stay (they fire
    /// while an X11 app has focus) and the Hotkeys page explains how to bind
    /// Scribe's command in the desktop's keyboard settings. The portal keeps
    /// its event callback for good, so the callback owns its handle.
    pub(super) fn start_portal(core: &Core, hotkeys: &HotkeySettings) {
        if crate::platform::session_kind() != SessionKind::Wayland {
            return;
        }
        let callback_core = core.clone();
        let owns = shortcuts::start(
            specs(hotkeys),
            Arc::new(move |id: &str, down: bool| on_portal_event(&callback_core, id, down)),
        );
        log::info!(
            "Wayland session: hotkeys through {}",
            if owns {
                "the GlobalShortcuts portal"
            } else {
                "X11 grabs (no GlobalShortcuts portal)"
            }
        );
        core.hotkey_runtime()
            .portal_owns_hotkeys
            .store(owns, Ordering::SeqCst);
    }

    /// A portal shortcut went down or up. Hold to talk uses both edges; the
    /// other actions act on press, since the desktop, not Scribe, decides
    /// the keys there and some desktops don't report releases reliably.
    fn on_portal_event(core: &Core, id: &str, down: bool) {
        let Ok(action) = HotkeyAction::parse(id) else {
            log::debug!("Ignoring unknown portal shortcut {id}");
            return;
        };
        let runtime = core.hotkey_runtime();
        if !action_enabled(action, transform_enabled(runtime)) {
            return;
        }
        // A portal that answered after startup gave up on it: the X11 grabs
        // are in charge then, and one key press must not act twice.
        if !runtime.portal_owns_hotkeys.load(Ordering::SeqCst) {
            return;
        }
        if down {
            if !runtime.mark_pressed_once(action) {
                return;
            }
            if action == HotkeyAction::HoldToTalk {
                handle_press_edge_with(core, action);
            } else {
                run_action_with(core, action);
            }
        } else if runtime.mark_released_once(action) && action == HotkeyAction::HoldToTalk {
            handle_release_edge_with(core, action);
        }
    }

    pub(super) fn status(runtime: &HotkeyRuntime, hotkeys: &HotkeySettings) -> HotkeyStatus {
        let portal = shortcuts::status().unwrap_or_default();
        let state = match portal.state {
            PortalShortcutsState::Starting => PortalState::Starting,
            PortalShortcutsState::Unavailable => PortalState::Unavailable,
            PortalShortcutsState::Declined => PortalState::Declined,
            PortalShortcutsState::Active => PortalState::Active,
        };
        let transform_enabled = transform_enabled(runtime);
        let bindings = HOTKEY_ACTIONS
            .into_iter()
            .map(|action| {
                let trigger = portal
                    .triggers
                    .get(action.event_name())
                    .filter(|trigger| !trigger.trim().is_empty())
                    .cloned();
                let enabled = action_enabled(action, transform_enabled);
                let registered = enabled && state == PortalState::Active && trigger.is_some();
                let error = (enabled && !registered).then(|| {
                    match state {
                        PortalState::Starting => "Waiting for your desktop's shortcut dialog.",
                        PortalState::Declined => {
                            "Not set up: the desktop's shortcut dialog was cancelled."
                        }
                        _ => "No keys assigned in your desktop's shortcut settings.",
                    }
                    .to_string()
                });
                HotkeyBindingStatus {
                    action,
                    shortcut: action.shortcut(hotkeys).to_string(),
                    normalized_shortcut: None,
                    trigger: None,
                    registered,
                    error,
                    disabled_reason: (!enabled).then(|| {
                        "Off while Local AI is off. Turn on Local AI to use it.".to_string()
                    }),
                    system_trigger: trigger,
                }
            })
            .collect();
        HotkeyStatus {
            bindings,
            note_chord_active: false,
            note_chord_supported: false,
            hold_release_verification_required: false,
            backend: "portal",
            portal_state: Some(state),
            portal_configurable: portal.configurable,
            control_command: crate::platform::control_command(),
        }
    }
}

/// An [`ActionTarget`] for tests: a `RecordingHost`, a hotkey runtime, and
/// the intents queued, each with how many events the host had been sent
/// when it was queued (so a test can pin an event before its intent).
#[cfg(test)]
pub(crate) mod test_target {
    use std::sync::Mutex;

    use super::{ActionTarget, HotkeyRuntime};
    use crate::{
        dictation_worker::Intent,
        host::{Flavor, Host, RecordingHost},
        paths::Paths,
    };

    pub(crate) struct Recorder {
        pub(crate) host: RecordingHost,
        pub(crate) runtime: HotkeyRuntime,
        pub(crate) dashboard_toggles: bool,
        submitted: Mutex<Vec<(Intent, usize)>>,
    }

    impl Recorder {
        pub(crate) fn new() -> Self {
            Self {
                host: RecordingHost::new(
                    Paths::from_parts("/d".into(), "/c".into(), "/l".into(), None, None),
                    Flavor::Stable,
                ),
                runtime: HotkeyRuntime::default(),
                dashboard_toggles: true,
                submitted: Mutex::new(Vec::new()),
            }
        }

        /// The intents queued so far, oldest first, each with the number of
        /// events sent before it.
        pub(crate) fn submitted(&self) -> Vec<(Intent, usize)> {
            self.submitted.lock().unwrap().clone()
        }

        /// The `scribe:hotkey-action` payloads sent so far (and the names
        /// of any other events), oldest first.
        pub(crate) fn event_names(&self) -> Vec<String> {
            self.host
                .events()
                .iter()
                .map(|event| match event.name() {
                    "scribe:hotkey-action" => event.payload().as_str().unwrap().to_string(),
                    other => other.to_string(),
                })
                .collect()
        }
    }

    impl ActionTarget for Recorder {
        fn host(&self) -> &dyn Host {
            &self.host
        }

        fn runtime(&self) -> &HotkeyRuntime {
            &self.runtime
        }

        fn submit(&self, intent: Intent) {
            let sent = self.host.events().len();
            self.submitted.lock().unwrap().push((intent, sent));
        }

        fn dashboard_hotkey_toggles(&self) -> bool {
            self.dashboard_toggles
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Failing, HostCall, Recorded};
    use crate::settings::AppSettings;
    use test_target::Recorder;

    #[test]
    fn transform_selection_is_only_registered_with_local_ai_on() {
        assert!(!action_enabled(HotkeyAction::TransformSelection, false));
        assert!(action_enabled(HotkeyAction::TransformSelection, true));
        for action in HOTKEY_ACTIONS {
            if action != HotkeyAction::TransformSelection {
                assert!(action_enabled(action, false), "{action:?} must stay bound");
            }
        }
    }

    fn recording_host() -> crate::host::RecordingHost {
        crate::host::RecordingHost::new(
            crate::paths::Paths::from_parts("/d".into(), "/c".into(), "/l".into(), None, None),
            crate::host::Flavor::Stable,
        )
    }

    /// The failures go to the host under today's event name, with the
    /// payload this module sent through `app.emit` before.
    #[test]
    fn registration_failures_reach_the_host_as_before() {
        let host = recording_host();
        let failures = vec![HotkeyRegistrationFailure {
            action: HotkeyAction::HoldToTalk,
            shortcut: "Ctrl+Alt+Space".into(),
            message: "Taken by another app.".into(),
        }];

        emit_registration_failures_with(&host, &failures);

        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name(), "scribe:hotkey-registration-failed");
        let before = serde_json::to_value(RegistrationFailedEvent {
            failures: &failures,
        })
        .unwrap();
        assert_eq!(events[0].payload(), before);
        assert_eq!(
            events[0].payload()["failures"][0]["action"],
            serde_json::json!("holdToTalk")
        );
    }

    #[test]
    fn no_failures_send_no_event() {
        let host = recording_host();
        emit_registration_failures_with(&host, &[]);
        assert!(host.events().is_empty());
    }

    /// `scribe:hotkey-action` carries the bare action name, as the `&str`
    /// payload did before.
    #[test]
    fn the_hotkey_action_event_carries_the_bare_name() {
        for name in HOTKEY_ACTIONS
            .into_iter()
            .map(HotkeyAction::event_name)
            .chain(["note_dictation"])
        {
            let event = hotkey_action_event(name);
            assert_eq!(event.name(), "scribe:hotkey-action");
            assert_eq!(event.payload(), serde_json::to_value(name).unwrap());
        }
    }

    #[test]
    fn the_transform_switch_returns_the_previous_value() {
        let runtime = HotkeyRuntime::default();
        assert!(!transform_enabled(&runtime));
        assert!(!swap_transform_enabled(&runtime, true));
        assert!(transform_enabled(&runtime));
        assert!(swap_transform_enabled(&runtime, false));
        assert!(!transform_enabled(&runtime));
    }

    fn keyed_hotkeys() -> HotkeySettings {
        HotkeySettings {
            hold_to_talk: "Ctrl+Alt+Space".to_string(),
            toggle_dictation: "Ctrl+Alt+T".to_string(),
            paste_last_transcript: "Ctrl+Alt+V".to_string(),
            open_dashboard: "Ctrl+Alt+D".to_string(),
            discard_dictation: "Ctrl+Alt+X".to_string(),
            transform_selection: "Ctrl+Alt+R".to_string(),
            ..Default::default()
        }
    }

    fn binding(status: &HotkeyStatus, action: HotkeyAction) -> &HotkeyBindingStatus {
        status
            .bindings
            .iter()
            .find(|binding| binding.action == action)
            .unwrap()
    }

    /// The Hotkeys page reads the runtime for the Local AI switch and the
    /// stored errors, and the plugin for what is registered.
    #[test]
    fn native_status_reads_the_runtime_and_the_plugin() {
        let runtime = HotkeyRuntime::default();
        let hotkeys = keyed_hotkeys();

        let status = native_status(&runtime, &hotkeys, |_| true);
        assert_eq!(status.bindings.len(), HOTKEY_ACTIONS.len());
        let transform = binding(&status, HotkeyAction::TransformSelection);
        assert!(!transform.registered);
        assert!(transform.disabled_reason.is_some());
        let toggle = binding(&status, HotkeyAction::ToggleDictation);
        assert!(toggle.registered);
        assert_eq!(toggle.error, None);
        assert_eq!(toggle.trigger, Some(hotkeys.toggle_dictation_trigger));
        assert_eq!(binding(&status, HotkeyAction::HoldToTalk).trigger, None);

        swap_transform_enabled(&runtime, true);
        runtime.store_registration_errors(&[HotkeyRegistrationFailure {
            action: HotkeyAction::PasteLastTranscript,
            shortcut: hotkeys.paste_last_transcript.clone(),
            message: "Taken.".into(),
        }]);
        let status = native_status(&runtime, &hotkeys, |_| false);
        let transform = binding(&status, HotkeyAction::TransformSelection);
        assert_eq!(transform.disabled_reason, None);
        assert!(!transform.registered);
        let paste = binding(&status, HotkeyAction::PasteLastTranscript);
        assert!(!paste.registered);
        assert_eq!(paste.error.as_deref(), Some("Taken."));
        assert_eq!(binding(&status, HotkeyAction::ToggleDictation).error, None);
    }

    fn expect_plugin(shortcut: &str) -> HotKey {
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        };

        assert!(validate_hotkeys(&hotkeys).is_ok());
    }

    /// Windows and macOS never report a Wayland hotkey backend or a
    /// desktop-shortcut command, so the Hotkeys page never shows the Wayland
    /// guidance there.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn only_linux_reports_wayland_hotkeys() {
        assert_eq!(grab_backend(), "native");
        assert_eq!(control_command(), None);
    }

    #[test]
    fn default_hotkeys_validate() {
        let settings = AppSettings::default();

        assert!(validate_hotkeys(&settings.hotkeys).is_ok());
        #[cfg(windows)]
        assert!(matches!(
            parse_shortcut(&settings.hotkeys.hold_to_talk),
            Ok(ParsedShortcut::ModifierChord(_))
        ));
        assert!(matches!(
            parse_shortcut(&settings.hotkeys.toggle_dictation),
            Ok(ParsedShortcut::Plugin(_))
        ));
    }

    /// Linux registers global shortcuts as X11 key grabs and macOS as Carbon
    /// hot keys: a modifier-only chord can't be registered at all, and a key
    /// without modifiers steals that key from every other app. Every default
    /// (and Dev) bind must be a key with at least one modifier.
    #[cfg(not(windows))]
    #[test]
    fn default_hotkeys_register_and_leave_typing_alone() {
        for hotkeys in [HotkeySettings::default(), HotkeySettings::dev_defaults()] {
            assert!(validate_hotkeys(&hotkeys).is_ok());
            for action in HOTKEY_ACTIONS {
                let bind = action.shortcut(&hotkeys);
                match parse_shortcut(bind) {
                    Ok(ParsedShortcut::Plugin(shortcut)) => {
                        assert!(!shortcut.mods.is_empty(), "{bind} has no modifier")
                    }
                    other => panic!("{bind} is not a key chord: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn dev_hotkeys_validate_and_differ_from_production() {
        let dev = HotkeySettings::dev_defaults();
        // Each Dev bind must be individually registerable.
        assert!(validate_hotkeys(&dev).is_ok());
        #[cfg(windows)]
        assert!(matches!(
            parse_shortcut(&dev.hold_to_talk),
            Ok(ParsedShortcut::ModifierChord(_))
        ));
        // ...and differ from production on every action so the two flavors
        // never fight over the same global shortcut when both run.
        let prod = HotkeySettings::default();
        assert_ne!(prod.hold_to_talk, dev.hold_to_talk);
        assert_ne!(prod.toggle_dictation, dev.toggle_dictation);
        assert_ne!(prod.paste_last_transcript, dev.paste_last_transcript);
        assert_ne!(prod.open_dashboard, dev.open_dashboard);
        assert_ne!(prod.discard_dictation, dev.discard_dictation);
        assert_ne!(prod.transform_selection, dev.transform_selection);
    }

    // The hotkey edges, through a recorder standing in for the `Core`.

    /// Hold to talk sends its event before starting, and stops without one.
    #[test]
    fn hold_to_talk_announces_then_starts_and_stops_silently() {
        let target = Recorder::new();

        handle_press_edge_with(&target, HotkeyAction::HoldToTalk);
        assert_eq!(target.event_names(), ["hold_to_talk"]);
        assert_eq!(target.submitted(), [(Intent::StartHold, 1)]);

        handle_release_edge_with(&target, HotkeyAction::HoldToTalk);
        assert_eq!(target.event_names(), ["hold_to_talk"]);
        assert_eq!(
            target.submitted(),
            [(Intent::StartHold, 1), (Intent::Stop, 1)]
        );
        assert!(target.host.calls().is_empty());
    }

    /// A single-shot action fires on the edge its trigger selects, once.
    #[test]
    fn single_shot_actions_fire_on_their_trigger_edge_only() {
        let target = Recorder::new();
        target.runtime.store_triggers(HashMap::from([
            (HotkeyAction::PasteLastTranscript, TriggerEdge::Press),
            (HotkeyAction::DiscardDictation, TriggerEdge::Release),
        ]));

        handle_press_edge_with(&target, HotkeyAction::PasteLastTranscript);
        handle_release_edge_with(&target, HotkeyAction::PasteLastTranscript);
        assert_eq!(target.event_names(), ["paste_last_transcript"]);
        assert_eq!(target.submitted(), [(Intent::PasteLast, 1)]);

        handle_press_edge_with(&target, HotkeyAction::DiscardDictation);
        assert_eq!(target.event_names(), ["paste_last_transcript"]);
        handle_release_edge_with(&target, HotkeyAction::DiscardDictation);
        assert_eq!(
            target.event_names(),
            ["paste_last_transcript", "discard_dictation"]
        );
        assert_eq!(
            target.submitted(),
            [(Intent::PasteLast, 1), (Intent::Cancel, 2)]
        );
    }

    /// Each dictation action sends its `scribe:hotkey-action` event, then
    /// queues its intent. Hold to talk is not a single-shot action: as
    /// before, it still sends its event but queues nothing.
    #[test]
    fn each_action_sends_its_event_then_queues_its_intent() {
        let target = Recorder::new();
        let actions = [
            (HotkeyAction::ToggleDictation, Intent::Toggle),
            (HotkeyAction::PasteLastTranscript, Intent::PasteLast),
            (HotkeyAction::DiscardDictation, Intent::Cancel),
            (HotkeyAction::TransformSelection, Intent::ToggleTransform),
        ];
        for (action, _) in actions {
            run_action_with(&target, action);
        }

        let events = target.host.events();
        assert_eq!(events.len(), actions.len(), "one event per action");
        assert!(events
            .iter()
            .all(|event| event.name() == "scribe:hotkey-action"));
        for (index, (action, intent)) in actions.into_iter().enumerate() {
            assert_eq!(target.event_names()[index], action.event_name());
            assert_eq!(target.submitted()[index], (intent, index + 1));
        }
        assert_eq!(target.submitted().len(), actions.len());

        run_action_with(&target, HotkeyAction::HoldToTalk);
        assert_eq!(target.event_names().len(), actions.len() + 1);
        assert_eq!(target.event_names().last().unwrap(), "hold_to_talk");
        assert_eq!(target.submitted().len(), actions.len());
        assert!(target.host.calls().is_empty());
    }

    /// The toggle clears its pressed mark, so a key that sends no release
    /// still toggles on its next press.
    #[test]
    fn the_toggle_action_clears_its_pressed_mark() {
        let target = Recorder::new();
        assert!(target
            .runtime
            .mark_pressed_once(HotkeyAction::ToggleDictation));
        run_action_with(&target, HotkeyAction::ToggleDictation);
        assert!(target
            .runtime
            .mark_pressed_once(HotkeyAction::ToggleDictation));
    }

    /// Open Dashboard queues no dictation work; a dashboard the host can't
    /// show comes back as one `audio://recording-error`, after the action's
    /// event, whether the hotkey toggles or only opens.
    #[test]
    fn open_dashboard_reports_a_missing_window_as_a_recording_error() {
        for toggles in [true, false] {
            let mut target = Recorder::new();
            target.dashboard_toggles = toggles;

            run_action_with(&target, HotkeyAction::OpenDashboard);

            let events = target.host.events();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].name(), "scribe:hotkey-action");
            assert_eq!(events[0].payload(), serde_json::json!("open_dashboard"));
            assert_eq!(events[1].name(), "audio://recording-error");
            let missing = crate::host::no_main_window();
            assert_eq!(
                events[1].payload(),
                serde_json::to_value(crate::audio::RecordingErrorEvent {
                    code: missing.code,
                    message: missing.message,
                })
                .unwrap()
            );
            assert!(target.submitted().is_empty());
            // The setting picks the window call, made exactly once.
            let expected = if toggles {
                crate::host::HostCall::ToggleDashboard
            } else {
                crate::host::HostCall::OpenDashboard(None)
            };
            assert_eq!(target.host.calls(), std::slice::from_ref(&expected));
            // The window call comes between the action's event and the error.
            assert_eq!(
                target.host.sequence(),
                [
                    Recorded::Event("scribe:hotkey-action", events[0].payload()),
                    Recorded::Call(expected),
                    Recorded::Event("audio://recording-error", events[1].payload()),
                ]
            );
        }
    }

    /// Press-mode toggle key: its event, then the toggle.
    #[test]
    fn the_toggle_key_in_press_mode_announces_then_toggles() {
        let target = Recorder::new();
        on_toggle_key_press_with(&target);
        assert_eq!(target.event_names(), ["toggle_dictation"]);
        assert_eq!(target.submitted(), [(Intent::Toggle, 1)]);
    }

    /// The note chord: `note_dictation`, then a note toggle.
    #[test]
    fn the_note_chord_announces_then_toggles_a_note() {
        let target = Recorder::new();
        handle_note_chord_with(&target);
        assert_eq!(target.event_names(), ["note_dictation"]);
        assert_eq!(target.submitted(), [(Intent::ToggleNote, 1)]);
        assert!(target.host.calls().is_empty());
    }

    // Registering through the host.

    fn plugin_shortcut(text: &str) -> HotKey {
        expect_plugin(text)
    }

    /// Every enabled binding with a key is registered with the host, in
    /// action order, and found again by its shortcut id; nothing is sent.
    #[test]
    fn bindings_are_registered_through_the_host_in_action_order() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        let hotkeys = keyed_hotkeys();

        let registration = register_bindings(&host, &runtime, &hotkeys);

        assert!(registration.failures.is_empty());
        assert!(registration.chord_bindings.is_empty());
        assert_eq!(registration.toggle_vk, None, "Ctrl+Alt+T has modifiers");
        let enabled = HOTKEY_ACTIONS
            .into_iter()
            .filter(|action| *action != HotkeyAction::TransformSelection)
            .collect::<Vec<_>>();
        assert_eq!(
            host.calls(),
            enabled
                .iter()
                .map(|action| HostCall::RegisterShortcut(plugin_shortcut(
                    action.shortcut(&hotkeys)
                )))
                .collect::<Vec<_>>()
        );
        for action in enabled {
            let shortcut = plugin_shortcut(action.shortcut(&hotkeys));
            assert_eq!(runtime.action_for(&shortcut), Some(action));
        }
        assert_eq!(
            runtime.trigger_for(HotkeyAction::ToggleDictation),
            hotkeys.toggle_dictation_trigger
        );
        assert!(host.events().is_empty());

        // With Local AI on, Transform Selection is registered last.
        swap_transform_enabled(&runtime, true);
        let host = recording_host();
        register_bindings(&host, &runtime, &hotkeys);
        assert_eq!(
            host.calls().last(),
            Some(&HostCall::RegisterShortcut(plugin_shortcut(
                &hotkeys.transform_selection
            )))
        );
    }

    /// A host that refuses every shortcut: each binding fails with today's
    /// message (not the host's error text), the failures are kept for the
    /// Hotkeys page, and announcing them sends one event.
    #[test]
    fn a_refused_registration_fails_with_todays_message() {
        let host = recording_host();
        host.fail(Failing::RegisterShortcut);
        let runtime = HotkeyRuntime::default();
        let hotkeys = keyed_hotkeys();

        let registration = register_bindings(&host, &runtime, &hotkeys);

        let failed = registration
            .failures
            .iter()
            .map(|failure| failure.action)
            .collect::<Vec<_>>();
        assert_eq!(
            failed,
            [
                HotkeyAction::HoldToTalk,
                HotkeyAction::ToggleDictation,
                HotkeyAction::PasteLastTranscript,
                HotkeyAction::OpenDashboard,
                HotkeyAction::DiscardDictation,
            ]
        );
        for failure in &registration.failures {
            assert_eq!(failure.shortcut, failure.action.shortcut(&hotkeys));
            assert_eq!(
                failure.message,
                CommandError::hotkey_registration_failed(&failure.shortcut).message
            );
            let shortcut = plugin_shortcut(&failure.shortcut);
            assert_eq!(runtime.action_for(&shortcut), None);
        }
        assert!(host.events().is_empty(), "registering sends nothing");

        let status = status_in(&host, &runtime, &hotkeys);
        let toggle = binding(&status, HotkeyAction::ToggleDictation);
        assert!(!toggle.registered);
        assert_eq!(
            toggle.error,
            Some(CommandError::hotkey_registration_failed(&hotkeys.toggle_dictation).message)
        );

        emit_registration_failures_with(&host, &registration.failures);
        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].payload(),
            serde_json::to_value(RegistrationFailedEvent {
                failures: &registration.failures,
            })
            .unwrap()
        );
    }

    /// A shortcut another registration already holds fails alone; the other
    /// bindings stay registered.
    #[test]
    fn a_taken_shortcut_fails_alone() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        let hotkeys = keyed_hotkeys();
        let taken = plugin_shortcut(&hotkeys.paste_last_transcript);
        host.register_shortcut(taken).unwrap();

        let registration = register_bindings(&host, &runtime, &hotkeys);

        assert_eq!(registration.failures.len(), 1);
        assert_eq!(
            registration.failures[0].action,
            HotkeyAction::PasteLastTranscript
        );
        let status = status_in(&host, &runtime, &hotkeys);
        assert!(binding(&status, HotkeyAction::ToggleDictation).registered);
        assert!(binding(&status, HotkeyAction::HoldToTalk).registered);
    }

    /// A modifier-only chord never reaches the host: the Windows chord
    /// watcher takes it, and elsewhere it fails with the platform's reason.
    #[test]
    fn modifier_only_chords_never_reach_the_host() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        let hotkeys = HotkeySettings {
            hold_to_talk: "Ctrl+Shift".to_string(),
            ..keyed_hotkeys()
        };

        let registration = register_bindings(&host, &runtime, &hotkeys);

        let keyed = [
            HotkeyAction::ToggleDictation,
            HotkeyAction::PasteLastTranscript,
            HotkeyAction::OpenDashboard,
            HotkeyAction::DiscardDictation,
        ];
        assert_eq!(
            host.calls(),
            keyed
                .iter()
                .map(|action| HostCall::RegisterShortcut(plugin_shortcut(
                    action.shortcut(&hotkeys)
                )))
                .collect::<Vec<_>>()
        );
        if cfg!(windows) {
            assert!(registration.failures.is_empty());
            assert_eq!(
                registration.chord_bindings,
                [(expect_chord("Ctrl+Shift"), HotkeyAction::HoldToTalk)]
            );
        } else {
            assert!(registration.chord_bindings.is_empty());
            assert_eq!(registration.failures.len(), 1);
            assert_eq!(
                registration.failures[0].message,
                format!(
                    "Could not register Ctrl+Shift. {}",
                    modifier_only_unsupported()
                )
            );
        }
    }

    /// While the desktop's portal owns the hotkeys nothing is grabbed, the
    /// bindings are cleared, and the watchers get nothing to poll.
    #[test]
    fn a_portal_owned_set_grabs_nothing() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        let hotkeys = keyed_hotkeys();
        register_bindings(&host, &runtime, &hotkeys);
        runtime.portal_owns_hotkeys.store(true, Ordering::SeqCst);
        let host = recording_host();

        let registration = register_bindings(&host, &runtime, &hotkeys);

        assert!(host.calls().is_empty());
        assert!(registration.failures.is_empty());
        assert!(registration.chord_bindings.is_empty());
        assert_eq!(registration.toggle_vk, None);
        let toggle = plugin_shortcut(&hotkeys.toggle_dictation);
        assert_eq!(runtime.action_for(&toggle), None);
    }

    /// Replacing a set releases only what the host holds (it asks first,
    /// since the plugin may refuse an unheld shortcut), then registers the
    /// next set.
    #[test]
    fn replacing_releases_only_held_shortcuts_then_registers_the_next_set() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        let previous = keyed_hotkeys();
        let taken = plugin_shortcut(&previous.open_dashboard);
        host.register_shortcut(taken).unwrap();
        register_bindings(&host, &runtime, &previous);
        // Something else let go of Open Dashboard's keys meanwhile.
        host.unregister_shortcut(taken).unwrap();
        let next = HotkeySettings {
            toggle_dictation: "Ctrl+Alt+Y".to_string(),
            ..previous.clone()
        };
        let before = host.calls().len();

        unregister_hotkey_set(&host, &previous);
        let registration = register_bindings(&host, &runtime, &next);

        let calls = host.calls()[before..].to_vec();
        let released = [
            HotkeyAction::HoldToTalk,
            HotkeyAction::ToggleDictation,
            HotkeyAction::PasteLastTranscript,
            HotkeyAction::DiscardDictation,
        ];
        let registered = [
            HotkeyAction::HoldToTalk,
            HotkeyAction::ToggleDictation,
            HotkeyAction::PasteLastTranscript,
            HotkeyAction::OpenDashboard,
            HotkeyAction::DiscardDictation,
        ];
        let expected = released
            .iter()
            .map(|action| HostCall::UnregisterShortcut(plugin_shortcut(action.shortcut(&previous))))
            .chain(
                registered.iter().map(|action| {
                    HostCall::RegisterShortcut(plugin_shortcut(action.shortcut(&next)))
                }),
            )
            .collect::<Vec<_>>();
        assert_eq!(calls, expected);
        assert!(registration.failures.is_empty());
        assert_eq!(
            runtime.action_for(&plugin_shortcut("Ctrl+Alt+Y")),
            Some(HotkeyAction::ToggleDictation)
        );
        assert!(host.events().is_empty());
    }

    /// A release the host refuses is skipped; the others still happen.
    #[test]
    fn a_refused_release_does_not_stop_the_others() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        let hotkeys = keyed_hotkeys();
        register_bindings(&host, &runtime, &hotkeys);
        host.fail(Failing::UnregisterShortcut);
        let before = host.calls().len();

        unregister_hotkey_set(&host, &hotkeys);

        let released = host.calls()[before..]
            .iter()
            .filter(|call| matches!(call, HostCall::UnregisterShortcut(_)))
            .count();
        assert_eq!(released, 5, "every held shortcut was asked for");
    }

    /// The Hotkeys page asks the host what it holds.
    #[test]
    fn the_status_asks_the_host_what_it_holds() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        let hotkeys = keyed_hotkeys();
        let status = status_in(&host, &runtime, &hotkeys);
        assert!(!binding(&status, HotkeyAction::ToggleDictation).registered);

        register_bindings(&host, &runtime, &hotkeys);
        let status = status_in(&host, &runtime, &hotkeys);
        assert!(binding(&status, HotkeyAction::ToggleDictation).registered);
        assert_eq!(binding(&status, HotkeyAction::ToggleDictation).error, None);
        assert!(host.events().is_empty());
    }

    // The note chord's grab of Q.

    fn note_key() -> HotKey {
        HotKey::new(None, Code::KeyQ)
    }

    /// During a toggle hold Q is grabbed and kept as the note key.
    #[test]
    fn the_note_chord_grabs_q_during_a_hold() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();
        runtime.toggle_held.store(true, Ordering::SeqCst);

        arm_note_chord_in(&host, &runtime);

        assert_eq!(host.calls(), [HostCall::RegisterShortcut(note_key())]);
        assert!(runtime.is_note_key(note_key().id()));
        assert!(host.is_shortcut_registered(note_key()));
        assert!(host.events().is_empty());
    }

    /// A tap that ended before the grab finished lets Q go at once.
    #[test]
    fn the_note_chord_lets_q_go_when_the_hold_already_ended() {
        let host = recording_host();
        let runtime = HotkeyRuntime::default();

        arm_note_chord_in(&host, &runtime);

        assert_eq!(
            host.calls(),
            [
                HostCall::RegisterShortcut(note_key()),
                HostCall::UnregisterShortcut(note_key())
            ]
        );
        assert!(!runtime.is_note_key(note_key().id()));
        assert!(!host.is_shortcut_registered(note_key()));
    }

    /// Q taken by another app: the note chord is unavailable for this hold,
    /// nothing is kept and nothing is sent.
    #[test]
    fn a_refused_note_grab_leaves_no_note_key() {
        let host = recording_host();
        host.fail(Failing::RegisterShortcut);
        let runtime = HotkeyRuntime::default();
        runtime.toggle_held.store(true, Ordering::SeqCst);

        arm_note_chord_in(&host, &runtime);

        assert_eq!(host.calls(), [HostCall::RegisterShortcut(note_key())]);
        assert!(!runtime.is_note_key(note_key().id()));
        assert!(host.events().is_empty());
    }

    /// Releasing Q asks the host once, and a refusal is only logged.
    #[test]
    fn releasing_the_note_key_asks_the_host_once() {
        let host = recording_host();
        host.fail(Failing::UnregisterShortcut);
        release_note_key(&host, note_key());
        assert_eq!(host.calls(), [HostCall::UnregisterShortcut(note_key())]);
        assert!(host.events().is_empty());
    }
}
