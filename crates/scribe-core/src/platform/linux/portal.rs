//! Keystrokes into native Wayland apps through the XDG RemoteDesktop portal.
//!
//! Wayland gives no app a way to inject input on its own; the portal does,
//! after the user allows it once in a desktop dialog. Scribe asks for the
//! keyboard only, with `persist_mode = persistent`, and saves the restore
//! token the portal returns (see `grant_store`), so later sessions start
//! without a dialog. A token that was revoked or expired makes the portal
//! show its dialog again, which is the one time Scribe asks again; if the
//! user turns it down, Scribe stops asking on its own and copies instead.
//!
//! The logic (`PortalKeyboard`) runs against the `RemoteDesktopApi` trait so
//! it is unit-tested with a fake; `AshpdRemoteDesktop` is the real D-Bus
//! client. All portal traffic runs on one worker thread, which also closes
//! an idle session so the desktop's "remote control" indicator doesn't stay
//! up between dictations.

use std::{
    path::PathBuf,
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::Duration,
};

use ashpd::desktop::{
    remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions},
    PersistMode, ResponseError, Session,
};

use super::{
    grant_store::{FileGrantStore, GrantStore, MemoryGrantStore, PortalGrants},
    session::PortalPaste,
};

/// The RemoteDesktop version that can remember a grant (`persist_mode` and
/// restore tokens). Older portals would ask on every paste.
const PERSIST_VERSION: u32 = 2;

/// How long an unused session stays open. Dictations in a row reuse it;
/// after this the desktop's indicator goes away.
const IDLE_CLOSE: Duration = Duration::from_secs(45);

/// How long the permission dialog may stay unanswered.
const DIALOG_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a call that shows no dialog may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalFailure {
    /// No RemoteDesktop portal on this desktop (or no session bus).
    Unavailable(String),
    /// The user turned the permission down.
    Declined,
    /// Anything else: a closed session, a timeout, a D-Bus error.
    Failed(String),
}

impl std::fmt::Display for PortalFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(formatter, "portal not available ({reason})"),
            Self::Declined => write!(formatter, "permission declined"),
            Self::Failed(reason) => write!(formatter, "{reason}"),
        }
    }
}

/// The parts of the RemoteDesktop portal Scribe uses.
pub trait RemoteDesktopApi: Send {
    /// The portal's interface version; `Unavailable` when there is none.
    fn version(&mut self) -> Result<u32, PortalFailure>;
    /// Starts a keyboard-only session that asks to be remembered. With a
    /// valid `restore_token` no dialog shows. Returns the token for next
    /// time, if the portal gave one.
    fn start(&mut self, restore_token: Option<&str>) -> Result<Option<String>, PortalFailure>;
    fn keysym(&mut self, keysym: u32, pressed: bool) -> Result<(), PortalFailure>;
    fn close(&mut self);
}

/// The permission flow on top of the portal: when to ask, what to remember.
pub struct PortalKeyboard<A: RemoteDesktopApi, S: GrantStore> {
    api: A,
    store: S,
    grants: PortalGrants,
    open: bool,
    /// The open session can be resumed later without a dialog (the portal
    /// gave a restore token). Sessions that can't are kept open instead of
    /// closed when idle, so they don't ask again while Scribe runs.
    resumable: bool,
}

impl<A: RemoteDesktopApi, S: GrantStore> PortalKeyboard<A, S> {
    pub fn new(api: A, store: S) -> Self {
        let grants = store.load();
        Self {
            api,
            store,
            grants,
            open: false,
            resumable: false,
        }
    }

    pub fn paste_state(&mut self) -> PortalPaste {
        match self.api.version() {
            Ok(version) if version >= PERSIST_VERSION => {
                if self.grants.remote_desktop_declined {
                    PortalPaste::Declined
                } else {
                    PortalPaste::Usable
                }
            }
            _ => PortalPaste::Unavailable,
        }
    }

    /// Whether the user has allowed it (a session is open or a token saved).
    pub fn granted(&self) -> bool {
        self.open || self.grants.remote_desktop_token.is_some()
    }

    /// Opens a session unless one is open. `explicit` is a request the user
    /// made (a button), which may ask even after a decline; an automatic
    /// paste never asks again once declined.
    pub fn ensure_session(&mut self, explicit: bool) -> Result<(), PortalFailure> {
        if self.open {
            return Ok(());
        }
        match self.paste_state() {
            PortalPaste::Unavailable => {
                return Err(match self.api.version() {
                    Ok(version) => PortalFailure::Unavailable(format!(
                        "version {version} can't remember the permission"
                    )),
                    Err(failure) => failure,
                })
            }
            PortalPaste::Declined if !explicit => return Err(PortalFailure::Declined),
            _ => {}
        }
        let token = self.grants.remote_desktop_token.clone();
        match self.api.start(token.as_deref()) {
            Ok(next_token) => {
                self.open = true;
                self.resumable = next_token.is_some();
                if next_token.is_none() {
                    log::warn!(
                        "The RemoteDesktop portal gave no restore token; it will ask again after Scribe restarts"
                    );
                }
                // The old token is spent either way.
                self.grants.remote_desktop_token = next_token;
                self.grants.remote_desktop_declined = false;
                self.save();
                Ok(())
            }
            Err(PortalFailure::Declined) => {
                self.grants.remote_desktop_token = None;
                self.grants.remote_desktop_declined = true;
                self.save();
                Err(PortalFailure::Declined)
            }
            // A transient failure keeps the token for the next try.
            Err(failure) => Err(failure),
        }
    }

    /// Sends key presses and releases, opening a session first. A session
    /// the desktop closed (the user stopped it from the indicator) is opened
    /// once more and the keys sent again.
    pub fn send(&mut self, keys: &[(u32, bool)]) -> Result<(), PortalFailure> {
        self.ensure_session(false)?;
        match self.send_once(keys) {
            Ok(()) => Ok(()),
            Err(failure) => {
                log::warn!("RemoteDesktop session failed ({failure}); starting a new one");
                self.close();
                self.ensure_session(false)?;
                self.send_once(keys)
            }
        }
    }

    fn send_once(&mut self, keys: &[(u32, bool)]) -> Result<(), PortalFailure> {
        let mut held: Vec<u32> = Vec::new();
        for &(keysym, pressed) in keys {
            if let Err(failure) = self.api.keysym(keysym, pressed) {
                // Never leave a key down.
                for keysym in held.into_iter().rev() {
                    let _ = self.api.keysym(keysym, false);
                }
                return Err(failure);
            }
            if pressed {
                held.push(keysym);
            } else {
                held.retain(|key| *key != keysym);
            }
        }
        Ok(())
    }

    /// Closes the session when it can be resumed silently later.
    pub fn close_if_idle(&mut self) {
        if self.open && self.resumable {
            self.close();
        }
    }

    fn close(&mut self) {
        if self.open {
            self.api.close();
            self.open = false;
        }
    }

    fn save(&self) {
        let (token, declined) = (
            self.grants.remote_desktop_token.clone(),
            self.grants.remote_desktop_declined,
        );
        let saved = self.store.update(&mut |grants| {
            grants.remote_desktop_token = token.clone();
            grants.remote_desktop_declined = declined;
        });
        if let Err(error) = saved {
            log::warn!("Could not save the Wayland paste permission: {error}");
        }
    }
}

/// The shift+insert paste chord, as (keysym, pressed). Shift+Insert pastes in
/// GTK, Qt, Chromium, Firefox and Electron apps and in terminals, where
/// Ctrl+V is not paste; native Wayland can't tell Scribe which app is in
/// front, so one chord has to work everywhere. Terminals that paste the
/// PRIMARY selection on Shift+Insert get the text there too.
pub const PASTE_KEYS: [(u32, bool); 4] = [
    (super::x11::XK_SHIFT_L, true),
    (super::x11::XK_INSERT, true),
    (super::x11::XK_INSERT, false),
    (super::x11::XK_SHIFT_L, false),
];

/// The presses and releases that type `text`. The compositor finds the key
/// (and Shift level) for each keysym on the active layout.
pub fn typing_keys(text: &str) -> Vec<(u32, bool)> {
    text.chars()
        .filter(|character| *character != '\r')
        .flat_map(|character| {
            let keysym = super::x11::keysym_for_char(character);
            [(keysym, true), (keysym, false)]
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The real portal, over D-Bus.

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    async_io::block_on(future)
}

/// Runs `future`, giving up after `limit`.
pub(super) async fn with_timeout<T>(
    limit: Duration,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    use futures_util::future::{select, Either};
    let future = std::pin::pin!(future);
    let timer = std::pin::pin!(async_io::Timer::after(limit));
    match select(future, timer).await {
        Either::Left((value, _)) => Some(value),
        Either::Right(_) => None,
    }
}

pub(super) fn failure_from(error: ashpd::Error) -> PortalFailure {
    match error {
        ashpd::Error::Response(ResponseError::Cancelled) => PortalFailure::Declined,
        ashpd::Error::PortalNotFound(interface) => {
            PortalFailure::Unavailable(format!("{interface} is not provided"))
        }
        ashpd::Error::Zbus(error) => {
            let text = error.to_string();
            if text.contains("ServiceUnknown")
                || text.contains("UnknownInterface")
                || text.contains("UnknownMethod")
                || text.contains("NameHasNoOwner")
            {
                PortalFailure::Unavailable(text)
            } else {
                PortalFailure::Failed(text)
            }
        }
        other => PortalFailure::Failed(other.to_string()),
    }
}

#[derive(Default)]
pub struct AshpdRemoteDesktop {
    proxy: Option<RemoteDesktop>,
    unavailable: Option<PortalFailure>,
    session: Option<Session<RemoteDesktop>>,
}

impl AshpdRemoteDesktop {
    fn proxy(&mut self) -> Result<&RemoteDesktop, PortalFailure> {
        if let Some(failure) = &self.unavailable {
            return Err(failure.clone());
        }
        if self.proxy.is_none() {
            super::host::ensure_registered();
            let created = block_on(with_timeout(CALL_TIMEOUT, RemoteDesktop::new()))
                .unwrap_or_else(|| {
                    Err(ashpd::Error::Zbus(zbus_timeout("connecting to the portal")))
                });
            match created {
                Ok(proxy) => {
                    log::info!("RemoteDesktop portal version {}", proxy.version());
                    self.proxy = Some(proxy);
                }
                Err(error) => {
                    let failure = match failure_from(error) {
                        PortalFailure::Failed(reason) => PortalFailure::Unavailable(reason),
                        other => other,
                    };
                    log::info!("No usable RemoteDesktop portal: {failure}");
                    self.unavailable = Some(failure.clone());
                    return Err(failure);
                }
            }
        }
        self.proxy
            .as_ref()
            .ok_or_else(|| PortalFailure::Failed("no portal".to_string()))
    }
}

pub(super) fn zbus_timeout(what: &str) -> ashpd::zbus::Error {
    ashpd::zbus::Error::Failure(format!("Timed out {what}"))
}

impl RemoteDesktopApi for AshpdRemoteDesktop {
    fn version(&mut self) -> Result<u32, PortalFailure> {
        self.proxy().map(RemoteDesktop::version)
    }

    fn start(&mut self, restore_token: Option<&str>) -> Result<Option<String>, PortalFailure> {
        self.close();
        let proxy = self.proxy()?;
        let session = block_on(with_timeout(
            CALL_TIMEOUT,
            proxy.create_session(Default::default()),
        ))
        .ok_or_else(|| PortalFailure::Failed("Timed out creating a session".to_string()))?
        .map_err(failure_from)?;

        let started = block_on(async {
            proxy
                .select_devices(
                    &session,
                    SelectDevicesOptions::default()
                        .set_devices(ashpd::enumflags2::BitFlags::from(DeviceType::Keyboard))
                        .set_persist_mode(PersistMode::ExplicitlyRevoked)
                        .set_restore_token(restore_token),
                )
                .await?
                .response()?;
            // This is where the desktop shows its dialog, unless the token
            // restores the earlier grant.
            match with_timeout(
                DIALOG_TIMEOUT,
                proxy.start(&session, None, Default::default()),
            )
            .await
            {
                Some(request) => request?.response(),
                None => Err(ashpd::Error::Zbus(zbus_timeout(
                    "waiting for the permission",
                ))),
            }
        });

        match started {
            Ok(selected) if selected.devices().contains(DeviceType::Keyboard) => {
                let token = selected.restore_token().map(str::to_string);
                self.session = Some(session);
                Ok(token)
            }
            Ok(_) => {
                let _ = block_on(with_timeout(CALL_TIMEOUT, session.close()));
                Err(PortalFailure::Declined)
            }
            Err(error) => {
                let _ = block_on(with_timeout(CALL_TIMEOUT, session.close()));
                Err(failure_from(error))
            }
        }
    }

    fn keysym(&mut self, keysym: u32, pressed: bool) -> Result<(), PortalFailure> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| PortalFailure::Failed("no session".to_string()))?;
        let proxy = self
            .proxy
            .as_ref()
            .ok_or_else(|| PortalFailure::Failed("no portal".to_string()))?;
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        let keysym = i32::try_from(keysym)
            .map_err(|_| PortalFailure::Failed(format!("keysym 0x{keysym:x} out of range")))?;
        block_on(with_timeout(
            CALL_TIMEOUT,
            proxy.notify_keyboard_keysym(session, keysym, state, Default::default()),
        ))
        .ok_or_else(|| PortalFailure::Failed("Timed out sending a key".to_string()))?
        .map_err(failure_from)
    }

    fn close(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = block_on(with_timeout(CALL_TIMEOUT, session.close()));
        }
    }
}

// ---------------------------------------------------------------------------
// The worker thread that owns the session.

enum Job {
    State(mpsc::Sender<(PortalPaste, bool)>),
    Prepare {
        explicit: bool,
        reply: mpsc::Sender<Result<(), PortalFailure>>,
    },
    Keys {
        keys: Vec<(u32, bool)>,
        reply: mpsc::Sender<Result<(), PortalFailure>>,
    },
}

static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
static WORKER: Mutex<Option<mpsc::Sender<Job>>> = Mutex::new(None);

/// Where the permission is saved. Called once at startup; without it the
/// grant only lasts while Scribe runs.
pub fn set_state_dir(dir: PathBuf) {
    let _ = STATE_DIR.set(dir);
}

pub(super) fn state_dir() -> Option<PathBuf> {
    STATE_DIR.get().cloned()
}

fn worker() -> Option<mpsc::Sender<Job>> {
    let mut guard = WORKER.lock().ok()?;
    if let Some(sender) = guard.as_ref() {
        return Some(sender.clone());
    }
    let (sender, receiver) = mpsc::channel::<Job>();
    let spawned = thread::Builder::new()
        .name("scribe-wayland-paste".to_string())
        .spawn(move || {
            let store: Box<dyn GrantStore> = match STATE_DIR.get() {
                Some(dir) => Box::new(FileGrantStore::in_dir(dir)),
                None => Box::new(MemoryGrantStore::default()),
            };
            run_worker(
                PortalKeyboard::new(AshpdRemoteDesktop::default(), store),
                receiver,
            );
        });
    if let Err(error) = spawned {
        log::error!("Could not start the Wayland paste worker: {error}");
        return None;
    }
    *guard = Some(sender.clone());
    Some(sender)
}

impl GrantStore for Box<dyn GrantStore> {
    fn load(&self) -> PortalGrants {
        (**self).load()
    }

    fn save(&self, grants: &PortalGrants) -> Result<(), String> {
        (**self).save(grants)
    }
}

fn run_worker<A: RemoteDesktopApi, S: GrantStore>(
    mut keyboard: PortalKeyboard<A, S>,
    jobs: mpsc::Receiver<Job>,
) {
    loop {
        match jobs.recv_timeout(IDLE_CLOSE) {
            Ok(Job::State(reply)) => {
                let state = keyboard.paste_state();
                let _ = reply.send((state, keyboard.granted()));
            }
            Ok(Job::Prepare { explicit, reply }) => {
                let _ = reply.send(keyboard.ensure_session(explicit));
            }
            Ok(Job::Keys { keys, reply }) => {
                let _ = reply.send(keyboard.send(&keys));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => keyboard.close_if_idle(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn ask<T>(make: impl FnOnce(mpsc::Sender<T>) -> Job, fallback: T) -> T {
    let Some(worker) = worker() else {
        return fallback;
    };
    let (reply, answer) = mpsc::channel();
    if worker.send(make(reply)).is_err() {
        return fallback;
    }
    // The permission dialog can take a while; the worker has its own
    // timeouts, so this only guards against a dead worker.
    answer
        .recv_timeout(DIALOG_TIMEOUT + CALL_TIMEOUT * 4)
        .unwrap_or(fallback)
}

/// Whether pasting through the portal can work, and whether it is allowed.
pub fn paste_state() -> (PortalPaste, bool) {
    ask(Job::State, (PortalPaste::Unavailable, false))
}

/// Opens the session, showing the permission dialog if it hasn't been
/// answered (or, when `explicit`, even if it was turned down).
pub fn prepare(explicit: bool) -> Result<(), PortalFailure> {
    ask(
        |reply| Job::Prepare { explicit, reply },
        Err(PortalFailure::Failed(
            "the Wayland paste worker stopped".to_string(),
        )),
    )
}

pub fn send_keys(keys: Vec<(u32, bool)>) -> Result<(), PortalFailure> {
    ask(
        |reply| Job::Keys { keys, reply },
        Err(PortalFailure::Failed(
            "the Wayland paste worker stopped".to_string(),
        )),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A scripted portal that records what was sent.
    #[derive(Clone, Default)]
    struct FakePortal {
        log: Arc<Mutex<FakeLog>>,
    }

    #[derive(Default)]
    struct FakeLog {
        version: Option<u32>,
        /// Answers for the next `start` calls, in order.
        starts: Vec<Result<Option<String>, PortalFailure>>,
        tokens_seen: Vec<Option<String>>,
        keys: Vec<(u32, bool)>,
        fail_keys: usize,
        /// Pressing this keysym fails.
        fail_keysym: Option<u32>,
        closes: usize,
    }

    impl FakePortal {
        fn with(version: Option<u32>, starts: Vec<Result<Option<String>, PortalFailure>>) -> Self {
            let fake = Self::default();
            {
                let mut log = fake.log.lock().unwrap();
                log.version = version;
                log.starts = starts;
            }
            fake
        }
    }

    impl RemoteDesktopApi for FakePortal {
        fn version(&mut self) -> Result<u32, PortalFailure> {
            self.log
                .lock()
                .unwrap()
                .version
                .ok_or_else(|| PortalFailure::Unavailable("none".to_string()))
        }

        fn start(&mut self, token: Option<&str>) -> Result<Option<String>, PortalFailure> {
            let mut log = self.log.lock().unwrap();
            log.tokens_seen.push(token.map(str::to_string));
            if log.starts.is_empty() {
                return Err(PortalFailure::Failed("unexpected start".to_string()));
            }
            log.starts.remove(0)
        }

        fn keysym(&mut self, keysym: u32, pressed: bool) -> Result<(), PortalFailure> {
            let mut log = self.log.lock().unwrap();
            if log.fail_keys > 0 {
                log.fail_keys -= 1;
                return Err(PortalFailure::Failed("session closed".to_string()));
            }
            if pressed && log.fail_keysym == Some(keysym) {
                return Err(PortalFailure::Failed("key rejected".to_string()));
            }
            log.keys.push((keysym, pressed));
            Ok(())
        }

        fn close(&mut self) {
            self.log.lock().unwrap().closes += 1;
        }
    }

    fn keyboard(
        fake: &FakePortal,
        grants: PortalGrants,
    ) -> (
        PortalKeyboard<FakePortal, Arc<MemoryGrantStore>>,
        Arc<MemoryGrantStore>,
    ) {
        let store = Arc::new(MemoryGrantStore::default());
        store.save(&grants).unwrap();
        (PortalKeyboard::new(fake.clone(), store.clone()), store)
    }

    impl GrantStore for Arc<MemoryGrantStore> {
        fn load(&self) -> PortalGrants {
            (**self).load()
        }
        fn save(&self, grants: &PortalGrants) -> Result<(), String> {
            (**self).save(grants)
        }
    }

    #[test]
    fn the_first_paste_asks_once_and_saves_the_token() {
        let fake = FakePortal::with(Some(2), vec![Ok(Some("t1".to_string()))]);
        let (mut keyboard, store) = keyboard(&fake, PortalGrants::default());
        assert_eq!(keyboard.paste_state(), PortalPaste::Usable);
        assert!(!keyboard.granted());

        keyboard.send(&PASTE_KEYS).unwrap();
        assert!(keyboard.granted());
        assert_eq!(store.load().remote_desktop_token.as_deref(), Some("t1"));
        let log = fake.log.lock().unwrap();
        assert_eq!(log.tokens_seen, vec![None]);
        assert_eq!(log.keys, PASTE_KEYS.to_vec());
    }

    #[test]
    fn a_saved_token_is_reused_and_replaced_by_the_fresh_one() {
        let fake = FakePortal::with(
            Some(2),
            vec![Ok(Some("t2".to_string())), Ok(Some("t3".to_string()))],
        );
        let (mut keyboard, store) = keyboard(
            &fake,
            PortalGrants {
                remote_desktop_token: Some("t1".to_string()),
                ..Default::default()
            },
        );
        keyboard.send(&PASTE_KEYS).unwrap();
        assert_eq!(store.load().remote_desktop_token.as_deref(), Some("t2"));
        // An idle close, then the next paste resumes with the newest token.
        keyboard.close_if_idle();
        keyboard.send(&PASTE_KEYS).unwrap();
        assert_eq!(store.load().remote_desktop_token.as_deref(), Some("t3"));
        let log = fake.log.lock().unwrap();
        assert_eq!(
            log.tokens_seen,
            vec![Some("t1".to_string()), Some("t2".to_string())]
        );
        assert_eq!(log.closes, 1);
    }

    #[test]
    fn an_open_session_is_reused_without_starting_again() {
        let fake = FakePortal::with(Some(2), vec![Ok(Some("t1".to_string()))]);
        let (mut keyboard, _) = keyboard(&fake, PortalGrants::default());
        keyboard.send(&PASTE_KEYS).unwrap();
        keyboard.send(&typing_keys("hi")).unwrap();
        assert_eq!(fake.log.lock().unwrap().tokens_seen.len(), 1);
    }

    #[test]
    fn a_declined_dialog_is_remembered_and_not_shown_again() {
        let fake = FakePortal::with(Some(2), vec![Err(PortalFailure::Declined)]);
        let (mut keyboard, store) = keyboard(&fake, PortalGrants::default());
        assert_eq!(keyboard.send(&PASTE_KEYS), Err(PortalFailure::Declined));
        assert!(store.load().remote_desktop_declined);
        assert_eq!(keyboard.paste_state(), PortalPaste::Declined);

        // Automatic pastes don't ask again...
        assert_eq!(keyboard.send(&PASTE_KEYS), Err(PortalFailure::Declined));
        assert_eq!(fake.log.lock().unwrap().tokens_seen.len(), 1);
        assert!(fake.log.lock().unwrap().keys.is_empty());
    }

    #[test]
    fn the_settings_button_can_ask_again_after_a_decline() {
        let fake = FakePortal::with(Some(2), vec![Ok(Some("t9".to_string()))]);
        let (mut keyboard, store) = keyboard(
            &fake,
            PortalGrants {
                remote_desktop_declined: true,
                ..Default::default()
            },
        );
        assert_eq!(keyboard.ensure_session(false), Err(PortalFailure::Declined));
        keyboard.ensure_session(true).unwrap();
        let grants = store.load();
        assert!(!grants.remote_desktop_declined);
        assert_eq!(grants.remote_desktop_token.as_deref(), Some("t9"));
    }

    #[test]
    fn a_revoked_token_asks_once_and_a_refusal_clears_it() {
        // The portal shows its dialog for a token it no longer knows; the
        // user says no this time.
        let fake = FakePortal::with(Some(2), vec![Err(PortalFailure::Declined)]);
        let (mut keyboard, store) = keyboard(
            &fake,
            PortalGrants {
                remote_desktop_token: Some("revoked".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(keyboard.send(&PASTE_KEYS), Err(PortalFailure::Declined));
        let grants = store.load();
        assert_eq!(grants.remote_desktop_token, None);
        assert!(grants.remote_desktop_declined);
        assert_eq!(
            fake.log.lock().unwrap().tokens_seen,
            vec![Some("revoked".to_string())]
        );
    }

    #[test]
    fn a_transient_failure_keeps_the_token() {
        let fake = FakePortal::with(
            Some(2),
            vec![Err(PortalFailure::Failed("timeout".to_string()))],
        );
        let (mut keyboard, store) = keyboard(
            &fake,
            PortalGrants {
                remote_desktop_token: Some("t1".to_string()),
                ..Default::default()
            },
        );
        assert!(matches!(
            keyboard.send(&PASTE_KEYS),
            Err(PortalFailure::Failed(_))
        ));
        assert_eq!(store.load().remote_desktop_token.as_deref(), Some("t1"));
        assert!(!store.load().remote_desktop_declined);
    }

    #[test]
    fn a_session_closed_by_the_desktop_is_restarted_once() {
        let fake = FakePortal::with(
            Some(2),
            vec![Ok(Some("t1".to_string())), Ok(Some("t2".to_string()))],
        );
        let (mut keyboard, store) = keyboard(&fake, PortalGrants::default());
        keyboard.ensure_session(false).unwrap();
        fake.log.lock().unwrap().fail_keys = 1;
        keyboard.send(&PASTE_KEYS).unwrap();
        let log = fake.log.lock().unwrap();
        assert_eq!(log.keys, PASTE_KEYS.to_vec());
        assert_eq!(log.closes, 1);
        assert_eq!(store.load().remote_desktop_token.as_deref(), Some("t2"));
    }

    #[test]
    fn a_failed_send_releases_the_keys_it_pressed() {
        let fake = FakePortal::with(Some(2), vec![Ok(Some("t1".to_string()))]);
        let (mut keyboard, _) = keyboard(&fake, PortalGrants::default());
        keyboard.ensure_session(false).unwrap();
        // Shift goes down, then Insert fails: Shift must come back up.
        fake.log.lock().unwrap().fail_keysym = Some(super::super::x11::XK_INSERT);
        assert!(keyboard.send_once(&PASTE_KEYS).is_err());
        assert_eq!(
            fake.log.lock().unwrap().keys,
            vec![
                (super::super::x11::XK_SHIFT_L, true),
                (super::super::x11::XK_SHIFT_L, false)
            ]
        );
    }

    #[test]
    fn old_or_missing_portals_are_unavailable() {
        for version in [None, Some(1)] {
            let fake = FakePortal::with(version, Vec::new());
            let (mut keyboard, _) = keyboard(&fake, PortalGrants::default());
            assert_eq!(keyboard.paste_state(), PortalPaste::Unavailable);
            assert!(matches!(
                keyboard.send(&PASTE_KEYS),
                Err(PortalFailure::Unavailable(_))
            ));
            assert!(fake.log.lock().unwrap().tokens_seen.is_empty());
        }
    }

    #[test]
    fn sessions_without_a_token_stay_open_when_idle() {
        let fake = FakePortal::with(Some(2), vec![Ok(None)]);
        let (mut keyboard, store) = keyboard(&fake, PortalGrants::default());
        keyboard.send(&PASTE_KEYS).unwrap();
        keyboard.close_if_idle();
        assert_eq!(fake.log.lock().unwrap().closes, 0);
        assert_eq!(store.load().remote_desktop_token, None);
        assert!(keyboard.granted());
    }

    #[test]
    fn typing_keys_press_and_release_each_character() {
        assert_eq!(
            typing_keys("a\r\nB"),
            vec![
                (0x61, true),
                (0x61, false),
                (super::super::x11::XK_RETURN, true),
                (super::super::x11::XK_RETURN, false),
                (0x42, true),
                (0x42, false),
            ]
        );
    }
}
