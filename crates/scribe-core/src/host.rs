//! What the engine needs from the shell it runs under.
//!
//! The engine never talks to a window, a tray or Tauri's event system
//! directly: it asks its [`Host`]. Today's shell (app/src-tauri) implements
//! `Host` on top of Tauri (`TauriHost`); a native shell implements it again
//! without changing the engine.
//!
//! `Host` also carries the tray and the main window, so there is one seam.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

use global_hotkey::hotkey::HotKey;

use crate::{
    app_state::AppStatus, error::CommandError, events::CoreEvent, paths::Paths,
    settings::AppSettings,
};

/// Which build of Scribe this is. Stable and Dev keep separate folders,
/// hotkeys and settings so both can run side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Stable,
    Dev,
}

impl Flavor {
    /// The flavour an app identifier belongs to: the Dev flavour's identifier
    /// ends in `.dev` (`tauri.dev-flavor.conf.json`).
    pub fn from_identifier(identifier: &str) -> Self {
        if identifier.ends_with(".dev") {
            Self::Dev
        } else {
            Self::Stable
        }
    }

    /// The app identifier, which names the data, cache and log folders.
    pub fn identifier(self) -> &'static str {
        match self {
            Self::Stable => "com.natkins.scribe",
            Self::Dev => "com.natkins.scribe.dev",
        }
    }

    /// The product name, which names the install and resources folders.
    pub fn product_name(self) -> &'static str {
        match self {
            Self::Stable => "Scribe",
            Self::Dev => "Scribe Dev",
        }
    }

    /// The name the interface shows: "Scribe" or "Scribe Dev".
    pub fn display_name(self) -> &'static str {
        self.product_name()
    }

    pub fn is_dev(self) -> bool {
        self == Self::Dev
    }
}

/// The shell the engine runs under: where events go, where the folders are,
/// which flavour this is, and the tray and main window.
pub trait Host: Send + Sync + 'static {
    /// Sends an event to every window (the shell picks how).
    fn emit(&self, event: CoreEvent);

    /// The folders the engine uses.
    fn paths(&self) -> &Paths;

    /// Stable or Dev.
    fn flavor(&self) -> Flavor;

    /// The tray's status word (tooltip and title).
    fn update_tray_status(&self, status: AppStatus);

    /// The tray tooltip's speech-model line.
    fn update_engine_summary(&self, summary: String);

    /// The tray menu's "Launch at startup" tick.
    fn sync_settings(&self, settings: &AppSettings);

    /// Shows and focuses the main window on a route ("dashboard" when None).
    fn open_dashboard(&self, route: Option<&str>) -> Result<(), CommandError>;

    /// Hides the main window when it is in front, else opens it.
    fn toggle_dashboard(&self) -> Result<(), CommandError>;

    /// The main window's inner size in logical pixels.
    fn main_window_logical_size(&self) -> Option<(u32, u32)>;

    /// The running app's version, as the shell was built with it. Dev builds
    /// and the installer-upgrade test build override the version at build
    /// time, so this is never the engine crate's `CARGO_PKG_VERSION`.
    /// Callable from any thread.
    fn app_version(&self) -> String;

    /// Exits the app with `code` through the shell's usual exit path, so
    /// `Core::shutdown` still runs. Callable from any thread: it only asks the
    /// shell to exit and returns.
    fn exit(&self, code: i32);

    /// Opens a file or folder with the system's default program.
    fn open_path(&self, path: &Path) -> Result<(), String>;

    /// Opens a URL in the default browser.
    fn open_url(&self, url: &str) -> Result<(), String>;

    /// Shows a system notification. Does not wait for the user.
    fn notify(&self, title: &str, body: &str) -> Result<(), String>;

    /// Registers a global shortcut; the shell sends its presses to the engine
    /// (`hotkeys::handle_shortcut_with`). Fails when the shortcut is already taken.
    ///
    /// The engine's rule: once the shell's event loop runs, the engine never
    /// calls this from the shell's main thread (rebinds, settings changes and
    /// the note chord register from engine threads), so a host may wait for
    /// its main thread here (Tauri's does). The one exception is the shell's
    /// own setup, which registers the first hotkeys on the main thread before
    /// the event loop starts.
    fn register_shortcut(&self, shortcut: HotKey) -> Result<(), String>;

    /// Releases a global shortcut registered with [`Host::register_shortcut`],
    /// under the same rule: never from the shell's main thread once its event
    /// loop runs.
    fn unregister_shortcut(&self, shortcut: HotKey) -> Result<(), String>;

    /// Whether this app holds `shortcut`.
    fn is_shortcut_registered(&self, shortcut: HotKey) -> bool;

    /// Whether the system starts this app at sign-in: the startup entry
    /// exists (on Windows, the `Run` value named after
    /// [`Flavor::product_name`], starting this exe).
    fn launch_at_login(&self) -> Result<bool, String>;

    /// Writes (`true`) or removes (`false`) the startup entry. On Windows
    /// writing also switches it on in Windows' Startup apps list, and the
    /// entry starts the app with `autostart::SIGN_IN_FLAG`.
    fn set_launch_at_login(&self, enabled: bool) -> Result<(), String>;
}

/// The error a host with no main window answers window calls with.
pub fn no_main_window() -> CommandError {
    CommandError::new("window_not_found", "Could not find the main window.")
}

/// A call [`RecordingHost`] recorded: every host call that acts on the
/// system. The queries (`app_version`, `is_shortcut_registered`,
/// `launch_at_login`) answer from the host's state and are not recorded.
// A test record, built and dropped within a test: `SyncSettings` carries the
// settings as they are (the spec's shape), so the size gap is not worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCall {
    Exit(i32),
    OpenPath(PathBuf),
    OpenUrl(String),
    Notify {
        title: String,
        body: String,
    },
    RegisterShortcut(HotKey),
    UnregisterShortcut(HotKey),
    SetLaunchAtLogin(bool),
    /// The tray menu's "Launch at startup" tick was synced to these settings.
    SyncSettings(AppSettings),
    /// The main window was asked to open, on this route (`None` is the
    /// default one).
    OpenDashboard(Option<String>),
    /// The main window was asked to toggle.
    ToggleDashboard,
    /// The tray's status word was updated.
    UpdateTrayStatus(AppStatus),
    /// The tray tooltip's speech-model line was updated.
    UpdateEngineSummary(String),
}

/// One entry of [`RecordingHost::sequence`]: an event or a call, in the order
/// the host received them. An event is kept as its name and its JSON payload
/// (`CoreEvent` is not comparable).
// A test record, built and dropped within a test: `SyncSettings` carries the
// settings as they are (the spec's shape), so the size gap is not worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    Event(&'static str, serde_json::Value),
    Call(HostCall),
}

/// A kind of [`RecordingHost`] call a test can make fail
/// ([`RecordingHost::fail`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Failing {
    OpenPath,
    OpenUrl,
    Notify,
    RegisterShortcut,
    UnregisterShortcut,
    /// Both reading and writing the startup entry.
    LaunchAtLogin,
}

impl Failing {
    /// The error a failing call answers with.
    pub fn message(self) -> String {
        format!("{self:?} failed on purpose")
    }
}

/// The version a [`RecordingHost`] answers unless the test sets one. It is
/// deliberately not the engine crate's version, so a test can tell the
/// host's answer from `CARGO_PKG_VERSION`.
pub const RECORDING_HOST_VERSION: &str = "0.0.0-recording-host";

/// A host with no windows and no tray that keeps every event it is sent and
/// every call it gets, for tests (in this crate and in the shell). Window
/// calls are recorded and answer as they do with no main window; nothing it
/// is asked to do reaches the system (no exit, no opened file, no shortcut,
/// no startup entry). [`RecordingHost::events`] and
/// [`RecordingHost::calls`] are the two lists; [`RecordingHost::sequence`]
/// is both in one list, in arrival order.
pub struct RecordingHost {
    paths: Paths,
    flavor: Flavor,
    app_version: String,
    events: Mutex<Vec<CoreEvent>>,
    calls: Mutex<Vec<HostCall>>,
    sequence: Mutex<Vec<Recorded>>,
    shortcuts: Mutex<HashSet<HotKey>>,
    launch_at_login: Mutex<bool>,
    failing: Mutex<HashSet<Failing>>,
}

/// Locks `mutex`, carrying on through a poisoned lock (a test that panicked
/// must not hide what was recorded).
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl RecordingHost {
    pub fn new(paths: Paths, flavor: Flavor) -> Self {
        Self {
            paths,
            flavor,
            app_version: RECORDING_HOST_VERSION.to_string(),
            events: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            sequence: Mutex::new(Vec::new()),
            shortcuts: Mutex::new(HashSet::new()),
            launch_at_login: Mutex::new(false),
            failing: Mutex::new(HashSet::new()),
        }
    }

    /// The same host, answering `version` as the app's version.
    pub fn with_app_version(mut self, version: impl Into<String>) -> Self {
        self.app_version = version.into();
        self
    }

    /// The events sent so far, oldest first.
    pub fn events(&self) -> Vec<CoreEvent> {
        lock(&self.events).clone()
    }

    /// The calls made so far, oldest first (see [`HostCall`]).
    pub fn calls(&self) -> Vec<HostCall> {
        lock(&self.calls).clone()
    }

    /// Every event and call so far in one list, oldest first, so a test can
    /// pin an event against a call (`events()` and `calls()` cannot).
    pub fn sequence(&self) -> Vec<Recorded> {
        lock(&self.sequence).clone()
    }

    /// From now on, calls of this kind answer `Err(what.message())` (they
    /// are still recorded).
    pub fn fail(&self, what: Failing) {
        lock(&self.failing).insert(what);
    }

    /// Sets whether the startup entry exists, as if the user changed it
    /// outside the app.
    pub fn set_launch_at_login_state(&self, enabled: bool) {
        *lock(&self.launch_at_login) = enabled;
    }

    /// Keeps a call in `calls` and, in the same step, in `sequence`, so the
    /// two lists never disagree about the order.
    fn record(&self, call: HostCall) {
        let mut sequence = lock(&self.sequence);
        lock(&self.calls).push(call.clone());
        sequence.push(Recorded::Call(call));
    }

    fn check(&self, what: Failing) -> Result<(), String> {
        if lock(&self.failing).contains(&what) {
            Err(what.message())
        } else {
            Ok(())
        }
    }
}

impl Host for RecordingHost {
    fn emit(&self, event: CoreEvent) {
        let mut sequence = lock(&self.sequence);
        sequence.push(Recorded::Event(
            event.name(),
            serde_json::to_value(&event).expect("an event serializes"),
        ));
        lock(&self.events).push(event);
    }

    fn paths(&self) -> &Paths {
        &self.paths
    }

    fn flavor(&self) -> Flavor {
        self.flavor
    }

    fn update_tray_status(&self, status: AppStatus) {
        self.record(HostCall::UpdateTrayStatus(status));
    }

    fn update_engine_summary(&self, summary: String) {
        self.record(HostCall::UpdateEngineSummary(summary));
    }

    fn sync_settings(&self, settings: &AppSettings) {
        self.record(HostCall::SyncSettings(settings.clone()));
    }

    fn open_dashboard(&self, route: Option<&str>) -> Result<(), CommandError> {
        self.record(HostCall::OpenDashboard(route.map(str::to_string)));
        Err(no_main_window())
    }

    fn toggle_dashboard(&self) -> Result<(), CommandError> {
        self.record(HostCall::ToggleDashboard);
        Err(no_main_window())
    }

    fn main_window_logical_size(&self) -> Option<(u32, u32)> {
        None
    }

    fn app_version(&self) -> String {
        self.app_version.clone()
    }

    fn exit(&self, code: i32) {
        self.record(HostCall::Exit(code));
    }

    fn open_path(&self, path: &Path) -> Result<(), String> {
        self.record(HostCall::OpenPath(path.to_path_buf()));
        self.check(Failing::OpenPath)
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.record(HostCall::OpenUrl(url.to_string()));
        self.check(Failing::OpenUrl)
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        self.record(HostCall::Notify {
            title: title.to_string(),
            body: body.to_string(),
        });
        self.check(Failing::Notify)
    }

    /// Like the global-shortcut plugin, refuses a shortcut it already holds.
    fn register_shortcut(&self, shortcut: HotKey) -> Result<(), String> {
        self.record(HostCall::RegisterShortcut(shortcut));
        self.check(Failing::RegisterShortcut)?;
        if lock(&self.shortcuts).insert(shortcut) {
            Ok(())
        } else {
            Err(format!("{shortcut} is already registered"))
        }
    }

    fn unregister_shortcut(&self, shortcut: HotKey) -> Result<(), String> {
        self.record(HostCall::UnregisterShortcut(shortcut));
        self.check(Failing::UnregisterShortcut)?;
        lock(&self.shortcuts).remove(&shortcut);
        Ok(())
    }

    fn is_shortcut_registered(&self, shortcut: HotKey) -> bool {
        lock(&self.shortcuts).contains(&shortcut)
    }

    fn launch_at_login(&self) -> Result<bool, String> {
        self.check(Failing::LaunchAtLogin)?;
        Ok(*lock(&self.launch_at_login))
    }

    fn set_launch_at_login(&self, enabled: bool) -> Result<(), String> {
        self.record(HostCall::SetLaunchAtLogin(enabled));
        self.check(Failing::LaunchAtLogin)?;
        *lock(&self.launch_at_login) = enabled;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flavour_follows_the_identifier() {
        assert_eq!(
            Flavor::from_identifier("com.natkins.scribe"),
            Flavor::Stable
        );
        assert_eq!(
            Flavor::from_identifier("com.natkins.scribe.dev"),
            Flavor::Dev
        );
        for flavor in [Flavor::Stable, Flavor::Dev] {
            assert_eq!(Flavor::from_identifier(flavor.identifier()), flavor);
        }
    }

    #[test]
    fn the_flavours_match_the_tauri_configs() {
        let stable: serde_json::Value =
            serde_json::from_str(include_str!("../../../app/src-tauri/tauri.conf.json")).unwrap();
        let dev: serde_json::Value = serde_json::from_str(include_str!(
            "../../../app/src-tauri/tauri.dev-flavor.conf.json"
        ))
        .unwrap();
        for (config, flavor) in [(stable, Flavor::Stable), (dev, Flavor::Dev)] {
            assert_eq!(config["identifier"], flavor.identifier());
            assert_eq!(config["productName"], flavor.product_name());
        }
    }

    #[test]
    fn a_recording_host_keeps_what_it_is_sent() {
        let host = RecordingHost::new(
            Paths::from_parts("/d".into(), "/c".into(), "/l".into(), None, None),
            Flavor::Dev,
        );
        host.emit(CoreEvent::SelectionTransformed("done".into()));
        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name(), "scribe:selection-transformed");
        assert_eq!(host.flavor(), Flavor::Dev);
        assert!(host.open_dashboard(None).is_err());
        assert_eq!(host.main_window_logical_size(), None);
    }

    #[test]
    fn a_recording_host_records_the_tray_and_window_calls() {
        let host = recording_host();
        let settings = AppSettings::default();
        host.update_tray_status(AppStatus::Recording);
        host.update_engine_summary("Whisper tiny.en".into());
        host.sync_settings(&settings);
        assert_eq!(
            host.open_dashboard(Some("settings")).unwrap_err().code,
            "window_not_found"
        );
        assert_eq!(
            host.open_dashboard(None).unwrap_err().code,
            "window_not_found"
        );
        assert_eq!(
            host.toggle_dashboard().unwrap_err().code,
            "window_not_found"
        );
        assert_eq!(
            host.calls(),
            [
                HostCall::UpdateTrayStatus(AppStatus::Recording),
                HostCall::UpdateEngineSummary("Whisper tiny.en".into()),
                HostCall::SyncSettings(settings),
                HostCall::OpenDashboard(Some("settings".into())),
                HostCall::OpenDashboard(None),
                HostCall::ToggleDashboard,
            ]
        );
        assert!(host.events().is_empty());
    }

    #[test]
    fn a_recording_host_keeps_events_and_calls_in_one_ordered_list() {
        let host = recording_host();
        host.emit(CoreEvent::SelectionTransformed("one".into()));
        host.update_engine_summary("model".into());
        host.exit(0);
        host.emit(CoreEvent::SelectionTransformed("two".into()));
        // The queries are not recorded.
        let _ = host.app_version();
        let _ = host.launch_at_login();

        assert_eq!(
            host.sequence(),
            [
                Recorded::Event("scribe:selection-transformed", serde_json::json!("one")),
                Recorded::Call(HostCall::UpdateEngineSummary("model".into())),
                Recorded::Call(HostCall::Exit(0)),
                Recorded::Event("scribe:selection-transformed", serde_json::json!("two")),
            ]
        );
        // The two lists keep their own order and agree with the sequence.
        assert_eq!(host.events().len(), 2);
        assert_eq!(
            host.calls(),
            [
                HostCall::UpdateEngineSummary("model".into()),
                HostCall::Exit(0)
            ]
        );
    }

    fn recording_host() -> RecordingHost {
        RecordingHost::new(
            Paths::from_parts("/d".into(), "/c".into(), "/l".into(), None, None),
            Flavor::Stable,
        )
    }

    #[test]
    fn a_recording_host_answers_its_version_from_any_thread() {
        let host = recording_host();
        assert_eq!(host.app_version(), RECORDING_HOST_VERSION);
        assert_ne!(host.app_version(), env!("CARGO_PKG_VERSION"));

        let host: std::sync::Arc<dyn Host> =
            std::sync::Arc::new(recording_host().with_app_version("0.16.4-57"));
        let other = host.clone();
        let answered = std::thread::spawn(move || other.app_version())
            .join()
            .unwrap();
        assert_eq!(answered, "0.16.4-57");
        assert_eq!(host.app_version(), "0.16.4-57");
    }

    #[test]
    fn a_recording_host_records_exits_from_any_thread_without_exiting() {
        let host = std::sync::Arc::new(recording_host());
        let other = host.clone();
        std::thread::spawn(move || other.exit(1)).join().unwrap();
        host.exit(0);
        assert_eq!(host.calls(), [HostCall::Exit(1), HostCall::Exit(0)]);
        assert!(host.events().is_empty());
    }

    #[test]
    fn a_recording_host_records_opens_and_notifications() {
        let host = recording_host();
        assert_eq!(host.open_path(Path::new("/d/logs")), Ok(()));
        assert_eq!(host.open_url("https://github.com/login/device"), Ok(()));
        assert_eq!(
            host.notify("Scribe didn't paste", "Copied instead."),
            Ok(())
        );

        host.fail(Failing::OpenPath);
        host.fail(Failing::OpenUrl);
        host.fail(Failing::Notify);
        assert_eq!(
            host.open_path(Path::new("/d/clips")),
            Err(Failing::OpenPath.message())
        );
        assert_eq!(
            host.open_url("https://example.com"),
            Err(Failing::OpenUrl.message())
        );
        assert_eq!(host.notify("t", "b"), Err(Failing::Notify.message()));

        assert_eq!(
            host.calls(),
            [
                HostCall::OpenPath("/d/logs".into()),
                HostCall::OpenUrl("https://github.com/login/device".into()),
                HostCall::Notify {
                    title: "Scribe didn't paste".into(),
                    body: "Copied instead.".into()
                },
                HostCall::OpenPath("/d/clips".into()),
                HostCall::OpenUrl("https://example.com".into()),
                HostCall::Notify {
                    title: "t".into(),
                    body: "b".into()
                },
            ]
        );
    }

    #[test]
    fn a_recording_host_holds_shortcuts_like_the_plugin() {
        use global_hotkey::hotkey::{Code, Modifiers};

        let host = recording_host();
        let toggle = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SUPER), Code::Space);
        let note = HotKey::new(None, Code::KeyQ);

        assert!(!host.is_shortcut_registered(toggle));
        assert_eq!(host.register_shortcut(toggle), Ok(()));
        assert!(host.is_shortcut_registered(toggle));
        assert!(!host.is_shortcut_registered(note));
        // A shortcut already held is refused, as the plugin refuses it.
        assert!(host.register_shortcut(toggle).is_err());
        assert_eq!(host.unregister_shortcut(toggle), Ok(()));
        assert!(!host.is_shortcut_registered(toggle));

        host.fail(Failing::RegisterShortcut);
        assert_eq!(
            host.register_shortcut(note),
            Err(Failing::RegisterShortcut.message())
        );
        assert!(!host.is_shortcut_registered(note));
        host.fail(Failing::UnregisterShortcut);
        assert_eq!(
            host.unregister_shortcut(note),
            Err(Failing::UnregisterShortcut.message())
        );

        assert_eq!(
            host.calls(),
            [
                HostCall::RegisterShortcut(toggle),
                HostCall::RegisterShortcut(toggle),
                HostCall::UnregisterShortcut(toggle),
                HostCall::RegisterShortcut(note),
                HostCall::UnregisterShortcut(note),
            ]
        );
    }

    #[test]
    fn a_recording_host_keeps_the_startup_entry() {
        let host = recording_host();
        assert_eq!(host.launch_at_login(), Ok(false));
        assert_eq!(host.set_launch_at_login(true), Ok(()));
        assert_eq!(host.launch_at_login(), Ok(true));
        // The user removed the entry outside the app: not a host call.
        host.set_launch_at_login_state(false);
        assert_eq!(host.launch_at_login(), Ok(false));

        host.fail(Failing::LaunchAtLogin);
        assert_eq!(
            host.launch_at_login(),
            Err(Failing::LaunchAtLogin.message())
        );
        assert_eq!(
            host.set_launch_at_login(true),
            Err(Failing::LaunchAtLogin.message())
        );

        assert_eq!(
            host.calls(),
            [
                HostCall::SetLaunchAtLogin(true),
                HostCall::SetLaunchAtLogin(true)
            ]
        );
    }
}
