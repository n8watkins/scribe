//! Global hotkeys on Wayland through the XDG GlobalShortcuts portal (KDE
//! Plasma, GNOME 48 and later, Hyprland and others).
//!
//! Wayland doesn't let an app grab keys. Instead Scribe hands the desktop a
//! list of actions with suggested keys; the desktop asks the user once,
//! remembers the assignment for Scribe's app id, and tells Scribe when a
//! shortcut goes down (`Activated`) and up (`Deactivated`), so hold-to-talk
//! works too. Which keys trigger what is the desktop's to change (its
//! keyboard settings, or `ConfigureShortcuts` on portals that have it).
//!
//! On the next start Scribe lists what the desktop already has for it and
//! only binds when something is missing, so the dialog shows once. A
//! cancelled dialog is remembered and not shown again on its own.

use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use ashpd::desktop::{
    global_shortcuts::{GlobalShortcuts, NewShortcut, Shortcut},
    Session,
};
use futures_util::StreamExt;
use serde::Serialize;

use super::{
    grant_store::{FileGrantStore, GrantStore, MemoryGrantStore},
    portal::{failure_from, with_timeout, PortalFailure},
};

const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const DIALOG_TIMEOUT: Duration = Duration::from_secs(300);

/// One action Scribe offers the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    pub id: String,
    pub description: String,
    /// The suggested keys in the XDG shortcuts format (`CTRL+ALT+space`).
    pub preferred_trigger: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PortalShortcutsState {
    /// Not started, or still waiting for the desktop.
    Starting,
    /// The desktop has no GlobalShortcuts portal.
    Unavailable,
    /// The user cancelled the desktop's dialog.
    Declined,
    /// The desktop delivers Scribe's shortcuts.
    Active,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalShortcutsStatus {
    pub state: PortalShortcutsState,
    /// What the desktop says triggers each action, by action id. Empty for
    /// an action the user left unassigned.
    pub triggers: HashMap<String, String>,
    /// The portal can open the desktop's own shortcut settings for Scribe.
    pub configurable: bool,
}

impl Default for PortalShortcutsStatus {
    fn default() -> Self {
        Self {
            state: PortalShortcutsState::Starting,
            triggers: HashMap::new(),
            configurable: false,
        }
    }
}

pub type ShortcutHandler = Arc<dyn Fn(&str, bool) + Send + Sync>;

struct Portal {
    proxy: GlobalShortcuts,
    session: Mutex<Option<Arc<Session<GlobalShortcuts>>>>,
}

struct Shared {
    specs: Vec<ShortcutSpec>,
    store: Box<dyn GrantStore + Sync>,
    portal: Mutex<Option<Arc<Portal>>>,
    status: Mutex<PortalShortcutsStatus>,
}

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
/// The shortcuts held down right now (between Activated and Deactivated).
static HELD: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Starts the portal client on its own thread and waits (a few seconds at
/// most) to learn whether the desktop has the portal. Returns true when it
/// does: from then on the portal owns Scribe's hotkeys, including while its
/// one-time dialog is still open. Later calls return the first answer.
pub fn start(specs: Vec<ShortcutSpec>, handler: ShortcutHandler) -> bool {
    if let Some(shared) = SHARED.get() {
        return shared.status().state != PortalShortcutsState::Unavailable;
    }
    let store: Box<dyn GrantStore + Sync> = match super::portal::state_dir() {
        Some(dir) => Box::new(FileGrantStore::in_dir(&dir)),
        None => Box::new(MemoryGrantStore::default()),
    };
    let shared = Arc::new(Shared {
        specs,
        store,
        portal: Mutex::new(None),
        status: Mutex::new(PortalShortcutsStatus::default()),
    });
    let shared = SHARED.get_or_init(|| shared).clone();

    let (ready_tx, ready_rx) = mpsc::channel::<bool>();
    let thread_shared = shared.clone();
    let spawned = thread::Builder::new()
        .name("scribe-wayland-shortcuts".to_string())
        .spawn(move || {
            async_io::block_on(run(thread_shared, handler, ready_tx));
        });
    if let Err(error) = spawned {
        log::error!("Could not start the Wayland shortcuts thread: {error}");
        shared.set_state(PortalShortcutsState::Unavailable);
        return false;
    }
    ready_rx.recv_timeout(CALL_TIMEOUT).unwrap_or_else(|_| {
        log::warn!("The GlobalShortcuts portal didn't answer in time");
        false
    })
}

/// The portal's state, or `None` when it was never started (X11 sessions).
pub fn status() -> Option<PortalShortcutsStatus> {
    SHARED.get().map(|shared| shared.status())
}

/// Shows the desktop's dialog again, after the user cancelled it (the
/// Hotkeys page's "Set up shortcuts"). Opens a new session, since a session
/// binds only once.
pub fn bind_again() -> Result<PortalShortcutsStatus, String> {
    let shared = SHARED.get().ok_or("The shortcuts portal is not in use.")?;
    let portal = shared
        .portal
        .lock()
        .ok()
        .and_then(|portal| portal.clone())
        .ok_or("The shortcuts portal is not available.")?;
    async_io::block_on(async {
        if let Some(old) = portal.session.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = with_timeout(CALL_TIMEOUT, old.close()).await;
        }
        let session = with_timeout(
            CALL_TIMEOUT,
            portal.proxy.create_session(Default::default()),
        )
        .await
        .ok_or("Timed out talking to the shortcuts portal.")?
        .map_err(|error| error.to_string())?;
        let session = Arc::new(session);
        if let Ok(mut slot) = portal.session.lock() {
            *slot = Some(session.clone());
        }
        shared.bind(&portal, &session, true).await;
        Ok(shared.status())
    })
}

/// Opens the desktop's settings for Scribe's shortcuts (portal version 2).
pub fn configure() -> Result<(), String> {
    let shared = SHARED.get().ok_or("The shortcuts portal is not in use.")?;
    let portal = shared
        .portal
        .lock()
        .ok()
        .and_then(|portal| portal.clone())
        .ok_or("The shortcuts portal is not available.")?;
    let session = portal
        .session
        .lock()
        .ok()
        .and_then(|session| session.clone())
        .ok_or("Scribe's shortcuts aren't set up yet.")?;
    async_io::block_on(with_timeout(
        CALL_TIMEOUT,
        portal
            .proxy
            .configure_shortcuts(&session, None, Default::default()),
    ))
    .ok_or_else(|| "Timed out opening the shortcut settings.".to_string())?
    .map_err(|error| error.to_string())
}

/// Waits until no portal shortcut is held, or `limit` passes. Keys a paste
/// sends would otherwise combine with the still-held hotkey (Ctrl+Alt+V
/// plus Shift+Insert). Wayland doesn't let Scribe read the keyboard, but
/// the portal says when its shortcuts come up.
pub fn wait_for_release(limit: Duration) {
    let deadline = Instant::now() + limit;
    while held_count() > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(15));
    }
}

fn held_count() -> usize {
    HELD.lock()
        .ok()
        .and_then(|held| held.as_ref().map(HashSet::len))
        .unwrap_or(0)
}

fn set_held(id: &str, down: bool) {
    if let Ok(mut held) = HELD.lock() {
        let held = held.get_or_insert_with(HashSet::new);
        if down {
            held.insert(id.to_string());
        } else {
            held.remove(id);
        }
    }
}

impl Shared {
    fn status(&self) -> PortalShortcutsStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    fn set_state(&self, state: PortalShortcutsState) {
        if let Ok(mut status) = self.status.lock() {
            status.state = state;
        }
    }

    fn set_triggers(&self, shortcuts: &[Shortcut]) {
        if let Ok(mut status) = self.status.lock() {
            for shortcut in shortcuts {
                status.triggers.insert(
                    shortcut.id().to_string(),
                    shortcut.trigger_description().to_string(),
                );
            }
        }
    }

    /// Makes sure the desktop has every action bound for this session,
    /// asking only when something is missing.
    async fn bind(&self, portal: &Portal, session: &Session<GlobalShortcuts>, explicit: bool) {
        let listed = match with_timeout(
            CALL_TIMEOUT,
            portal.proxy.list_shortcuts(session, Default::default()),
        )
        .await
        {
            Some(Ok(request)) => request.response().ok(),
            _ => None,
        };
        let known: HashSet<String> = listed
            .as_ref()
            .map(|listed| {
                listed
                    .shortcuts()
                    .iter()
                    .map(|shortcut| shortcut.id().to_string())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(listed) = &listed {
            self.set_triggers(listed.shortcuts());
        }
        let missing = missing_ids(&self.specs, &known);
        if missing.is_empty() {
            log::info!("The desktop already has Scribe's shortcuts");
            self.set_state(PortalShortcutsState::Active);
            return;
        }

        let declined = self.store.load().shortcuts_declined;
        if declined && !explicit {
            log::info!("Scribe's shortcuts were declined before; not asking again");
            self.set_state(PortalShortcutsState::Declined);
            return;
        }

        let shortcuts: Vec<NewShortcut> = self
            .specs
            .iter()
            .map(|spec| {
                NewShortcut::new(spec.id.clone(), spec.description.clone())
                    .preferred_trigger(spec.preferred_trigger.as_deref())
            })
            .collect();
        let bound = match with_timeout(
            DIALOG_TIMEOUT,
            portal
                .proxy
                .bind_shortcuts(session, &shortcuts, None, Default::default()),
        )
        .await
        {
            Some(Ok(request)) => request.response().map_err(failure_from),
            Some(Err(error)) => Err(failure_from(error)),
            None => Err(PortalFailure::Failed("timed out".to_string())),
        };
        match bound {
            Ok(bound) => {
                log::info!(
                    "Bound {} shortcuts through the portal",
                    bound.shortcuts().len()
                );
                self.set_triggers(bound.shortcuts());
                self.set_state(PortalShortcutsState::Active);
                let _ = self
                    .store
                    .update(&mut |grants| grants.shortcuts_declined = false);
            }
            Err(PortalFailure::Declined) => {
                log::info!("The shortcuts dialog was cancelled");
                self.set_state(PortalShortcutsState::Declined);
                let _ = self
                    .store
                    .update(&mut |grants| grants.shortcuts_declined = true);
            }
            Err(failure) => {
                log::warn!("Binding shortcuts through the portal failed: {failure}");
                // Keep listening: a desktop that bound some of them still
                // delivers those.
                self.set_state(if known.is_empty() {
                    PortalShortcutsState::Declined
                } else {
                    PortalShortcutsState::Active
                });
            }
        }
    }
}

/// The spec ids the desktop doesn't have yet.
fn missing_ids(specs: &[ShortcutSpec], known: &HashSet<String>) -> Vec<String> {
    specs
        .iter()
        .filter(|spec| !known.contains(&spec.id))
        .map(|spec| spec.id.clone())
        .collect()
}

enum Event {
    Down(String),
    Up(String),
    Changed(Vec<Shortcut>),
}

async fn run(shared: Arc<Shared>, handler: ShortcutHandler, ready: mpsc::Sender<bool>) {
    super::host::ensure_registered();
    let proxy = match with_timeout(CALL_TIMEOUT, GlobalShortcuts::new()).await {
        Some(Ok(proxy)) => proxy,
        Some(Err(error)) => {
            log::info!("No GlobalShortcuts portal: {}", failure_from(error));
            shared.set_state(PortalShortcutsState::Unavailable);
            let _ = ready.send(false);
            return;
        }
        None => {
            log::warn!("Timed out connecting to the GlobalShortcuts portal");
            shared.set_state(PortalShortcutsState::Unavailable);
            let _ = ready.send(false);
            return;
        }
    };
    log::info!("GlobalShortcuts portal version {}", proxy.version());
    if let Ok(mut status) = shared.status.lock() {
        status.configurable = proxy.version() >= 2;
    }

    // Listen before binding, so nothing pressed right after is lost.
    let streams = async {
        Ok::<_, ashpd::Error>((
            proxy.receive_activated().await?,
            proxy.receive_deactivated().await?,
            proxy.receive_shortcuts_changed().await?,
        ))
    };
    let (activated, deactivated, changed) = match with_timeout(CALL_TIMEOUT, streams).await {
        Some(Ok(streams)) => streams,
        other => {
            log::warn!(
                "Could not listen to the GlobalShortcuts portal: {}",
                describe(other)
            );
            shared.set_state(PortalShortcutsState::Unavailable);
            let _ = ready.send(false);
            return;
        }
    };
    let created = with_timeout(CALL_TIMEOUT, proxy.create_session(Default::default())).await;
    let session = match created {
        Some(Ok(session)) => Arc::new(session),
        other => {
            log::warn!(
                "Could not open a GlobalShortcuts session: {}",
                describe(other)
            );
            shared.set_state(PortalShortcutsState::Unavailable);
            let _ = ready.send(false);
            return;
        }
    };
    let portal = Arc::new(Portal {
        proxy,
        session: Mutex::new(Some(session.clone())),
    });
    if let Ok(mut slot) = shared.portal.lock() {
        *slot = Some(portal.clone());
    }
    let _ = ready.send(true);

    shared.bind(&portal, &session, false).await;

    let mut events = futures_util::stream::select(
        futures_util::stream::select(
            activated.map(|signal| Event::Down(signal.shortcut_id().to_string())),
            deactivated.map(|signal| Event::Up(signal.shortcut_id().to_string())),
        ),
        changed.map(|signal| Event::Changed(signal.shortcuts().to_vec())),
    );
    while let Some(event) = events.next().await {
        match event {
            Event::Down(id) => {
                set_held(&id, true);
                handler(&id, true);
            }
            Event::Up(id) => {
                set_held(&id, false);
                handler(&id, false);
            }
            Event::Changed(shortcuts) => shared.set_triggers(&shortcuts),
        }
    }
    log::warn!("The GlobalShortcuts portal stopped sending events");
}

/// A failed or timed-out portal call, for the log.
fn describe<T>(outcome: Option<Result<T, ashpd::Error>>) -> String {
    match outcome {
        Some(Ok(_)) => "no error".to_string(),
        Some(Err(error)) => failure_from(error).to_string(),
        None => "timed out".to_string(),
    }
}

/// Converts a Scribe shortcut ("Ctrl+Alt+Space") to the XDG shortcuts
/// format the portal suggests to the user ("CTRL+ALT+space"). `None` for a
/// shortcut without a key, which the desktop couldn't use.
pub fn xdg_trigger(shortcut: &str) -> Option<String> {
    let mut modifiers: Vec<&str> = Vec::new();
    let mut key: Option<String> = None;
    for token in shortcut
        .split('+')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let lower = token.to_ascii_lowercase();
        let modifier = match lower.as_str() {
            "ctrl" | "control" => Some("CTRL"),
            "alt" | "option" => Some("ALT"),
            "shift" => Some("SHIFT"),
            "super" | "win" | "windows" | "meta" | "cmd" | "command" => Some("LOGO"),
            _ => None,
        };
        if let Some(modifier) = modifier {
            if !modifiers.contains(&modifier) {
                modifiers.push(modifier);
            }
            continue;
        }
        if key.is_some() {
            return None;
        }
        key = Some(xdg_key_name(token)?);
    }
    let key = key?;
    let order = ["CTRL", "ALT", "SHIFT", "LOGO"];
    let mut parts: Vec<String> = order
        .iter()
        .filter(|modifier| modifiers.contains(modifier))
        .map(|modifier| modifier.to_string())
        .collect();
    parts.push(key);
    Some(parts.join("+"))
}

/// The xkb keysym name for a W3C key code or a plain key name.
fn xdg_key_name(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    let named = match lower.as_str() {
        "`" | "~" | "backquote" | "tilde" => "grave",
        "space" => "space",
        "enter" | "return" => "Return",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "arrowup" | "up" => "Up",
        "arrowdown" | "down" => "Down",
        "arrowleft" | "left" => "Left",
        "arrowright" | "right" => "Right",
        "minus" | "-" => "minus",
        "equal" | "=" => "equal",
        "comma" | "," => "comma",
        "period" | "." => "period",
        "slash" | "/" => "slash",
        "backslash" | "\\" => "backslash",
        "semicolon" | ";" => "semicolon",
        "quote" | "'" => "apostrophe",
        "bracketleft" | "[" => "bracketleft",
        "bracketright" | "]" => "bracketright",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    let bare = lower
        .strip_prefix("key")
        .filter(|rest| rest.len() == 1)
        .or_else(|| lower.strip_prefix("digit").filter(|rest| rest.len() == 1))
        .unwrap_or(&lower);
    if bare.len() == 1 && bare.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(bare.to_string());
    }
    if let Some(number) = bare.strip_prefix('f') {
        if number.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) {
            return Some(format!("F{number}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scribe_shortcuts_become_xdg_triggers() {
        assert_eq!(
            xdg_trigger("Ctrl+Alt+Space").as_deref(),
            Some("CTRL+ALT+space")
        );
        assert_eq!(
            xdg_trigger("Ctrl+Alt+Backquote").as_deref(),
            Some("CTRL+ALT+grave")
        );
        assert_eq!(xdg_trigger("Alt+Ctrl+V").as_deref(), Some("CTRL+ALT+v"));
        assert_eq!(xdg_trigger("Ctrl+Alt+KeyF").as_deref(), Some("CTRL+ALT+f"));
        assert_eq!(
            xdg_trigger("Super+Shift+Digit5").as_deref(),
            Some("SHIFT+LOGO+5")
        );
        assert_eq!(xdg_trigger("Win+F9").as_deref(), Some("LOGO+F9"));
        assert_eq!(
            xdg_trigger("Ctrl+Alt+Shift+Backquote").as_deref(),
            Some("CTRL+ALT+SHIFT+grave")
        );
    }

    #[test]
    fn shortcuts_without_one_key_have_no_trigger() {
        assert_eq!(xdg_trigger("Ctrl+Win"), None);
        assert_eq!(xdg_trigger(""), None);
        assert_eq!(xdg_trigger("Ctrl+A+B"), None);
        assert_eq!(xdg_trigger("Ctrl+F99"), None);
        assert_eq!(xdg_trigger("Ctrl+MediaPlayPause"), None);
    }

    #[test]
    fn only_missing_shortcuts_need_binding() {
        let spec = |id: &str| ShortcutSpec {
            id: id.to_string(),
            description: id.to_string(),
            preferred_trigger: None,
        };
        let specs = vec![spec("hold_to_talk"), spec("toggle_dictation")];
        let known: HashSet<String> = ["hold_to_talk".to_string()].into();
        assert_eq!(
            missing_ids(&specs, &known),
            vec!["toggle_dictation".to_string()]
        );
        let all: HashSet<String> =
            ["hold_to_talk".to_string(), "toggle_dictation".to_string()].into();
        assert!(missing_ids(&specs, &all).is_empty());
    }

    #[test]
    fn held_shortcuts_delay_a_paste_until_released() {
        set_held("paste_last_transcript", true);
        let started = Instant::now();
        wait_for_release(Duration::from_millis(60));
        assert!(started.elapsed() >= Duration::from_millis(60));
        set_held("paste_last_transcript", false);
        let started = Instant::now();
        wait_for_release(Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
