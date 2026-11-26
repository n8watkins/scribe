//! The portal clients against a stand-in `org.freedesktop.portal.Desktop` on
//! a private session bus: the real D-Bus traffic of `AshpdRemoteDesktop` and
//! the GlobalShortcuts thread, without a desktop. The stand-in answers like
//! xdg-desktop-portal does (Request objects, Response signals, restore
//! tokens) and records what Scribe asked for.
//!
//! Needs a session bus, so it is ignored by default. CI runs it with
//! `dbus-run-session -- cargo test --lib portal_dbus -- --ignored`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use zbus::{
    message::Header,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedValue, Value},
    Connection,
};

use super::{
    grant_store::{GrantStore, MemoryGrantStore, PortalGrants},
    portal::{AshpdRemoteDesktop, PortalFailure, PortalKeyboard, PASTE_KEYS},
    shortcuts::{self, PortalShortcutsState, ShortcutSpec},
    x11::{XK_INSERT, XK_SHIFT_L},
};

const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Default)]
struct Recorded {
    persist_modes: Vec<u32>,
    device_types: Vec<u32>,
    restore_tokens_in: Vec<Option<String>>,
    tokens_issued: u32,
    /// Answer the next Start with "cancelled" (the user said no).
    cancel_next_start: bool,
    keys: Vec<(i32, u32)>,
    bound: Vec<(String, Option<String>)>,
    sessions: Vec<String>,
}

type Log = Arc<Mutex<Recorded>>;

fn sender_part(header: &Header<'_>) -> String {
    header
        .sender()
        .map(|sender| sender.trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default()
}

fn option_str(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_string)
}

fn option_u32(options: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    options.get(key).and_then(|value| u32::try_from(value).ok())
}

fn request_path(header: &Header<'_>, options: &HashMap<String, OwnedValue>) -> String {
    format!(
        "{DESKTOP_PATH}/request/{}/{}",
        sender_part(header),
        option_str(options, "handle_token").unwrap_or_default()
    )
}

async fn respond(
    connection: &Connection,
    path: &str,
    code: u32,
    results: HashMap<&str, Value<'_>>,
) -> zbus::fdo::Result<()> {
    connection
        .emit_signal(
            None::<&str>,
            path,
            "org.freedesktop.portal.Request",
            "Response",
            &(code, results),
        )
        .await
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

async fn create_session(
    connection: &Connection,
    header: &Header<'_>,
    options: &HashMap<String, OwnedValue>,
    log: &Log,
) -> zbus::fdo::Result<ObjectPath<'static>> {
    let request = request_path(header, options);
    let session = format!(
        "{DESKTOP_PATH}/session/{}/{}",
        sender_part(header),
        option_str(options, "session_handle_token").unwrap_or_default()
    );
    log.lock().unwrap().sessions.push(session.clone());
    // xdg-desktop-portal answers with the session handle as a string.
    respond(
        connection,
        &request,
        0,
        HashMap::from([("session_handle", Value::from(session.as_str()))]),
    )
    .await?;
    Ok(ObjectPath::try_from(request).unwrap().into_owned())
}

struct RemoteDesktopPortal {
    log: Log,
}

#[zbus::interface(name = "org.freedesktop.portal.RemoteDesktop")]
impl RemoteDesktopPortal {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[zbus(property, name = "AvailableDeviceTypes")]
    fn available_device_types(&self) -> u32 {
        7
    }

    async fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<ObjectPath<'static>> {
        create_session(connection, &header, &options, &self.log).await
    }

    async fn select_devices(
        &self,
        _session: ObjectPath<'_>,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<ObjectPath<'static>> {
        {
            let mut log = self.log.lock().unwrap();
            log.persist_modes
                .push(option_u32(&options, "persist_mode").unwrap_or(0));
            log.device_types
                .push(option_u32(&options, "types").unwrap_or(0));
            log.restore_tokens_in
                .push(option_str(&options, "restore_token"));
        }
        let request = request_path(&header, &options);
        respond(connection, &request, 0, HashMap::new()).await?;
        Ok(ObjectPath::try_from(request).unwrap().into_owned())
    }

    async fn start(
        &self,
        _session: ObjectPath<'_>,
        _parent_window: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<ObjectPath<'static>> {
        let request = request_path(&header, &options);
        let outcome = {
            let mut log = self.log.lock().unwrap();
            if std::mem::take(&mut log.cancel_next_start) {
                None
            } else {
                log.tokens_issued += 1;
                Some(format!("token-{}", log.tokens_issued))
            }
        };
        match outcome {
            Some(token) => {
                respond(
                    connection,
                    &request,
                    0,
                    HashMap::from([
                        ("devices", Value::from(1u32)),
                        ("restore_token", Value::from(token.as_str())),
                    ]),
                )
                .await?
            }
            None => respond(connection, &request, 1, HashMap::new()).await?,
        }
        Ok(ObjectPath::try_from(request).unwrap().into_owned())
    }

    async fn notify_keyboard_keysym(
        &self,
        _session: ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        keysym: i32,
        state: u32,
    ) {
        self.log.lock().unwrap().keys.push((keysym, state));
    }
}

struct GlobalShortcutsPortal {
    log: Log,
}

type ShortcutEntry<'a> = (String, HashMap<&'a str, Value<'a>>);

#[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl GlobalShortcutsPortal {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<ObjectPath<'static>> {
        create_session(connection, &header, &options, &self.log).await
    }

    async fn list_shortcuts(
        &self,
        _session: ObjectPath<'_>,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<ObjectPath<'static>> {
        let request = request_path(&header, &options);
        let listed: Vec<ShortcutEntry<'_>> = Vec::new();
        respond(
            connection,
            &request,
            0,
            HashMap::from([("shortcuts", Value::from(listed))]),
        )
        .await?;
        Ok(ObjectPath::try_from(request).unwrap().into_owned())
    }

    async fn bind_shortcuts(
        &self,
        _session: ObjectPath<'_>,
        shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
        _parent_window: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<ObjectPath<'static>> {
        let request = request_path(&header, &options);
        let mut bound: Vec<(String, HashMap<&str, Value<'_>>)> = Vec::new();
        {
            let mut log = self.log.lock().unwrap();
            for (id, info) in &shortcuts {
                let trigger = option_str(info, "preferred_trigger");
                log.bound.push((id.clone(), trigger.clone()));
                let description = option_str(info, "description").unwrap_or_default();
                bound.push((
                    id.clone(),
                    HashMap::from([
                        ("description", Value::from(description)),
                        (
                            "trigger_description",
                            Value::from(trigger.unwrap_or_default()),
                        ),
                    ]),
                ));
            }
        }
        respond(
            connection,
            &request,
            0,
            HashMap::from([("shortcuts", Value::from(bound))]),
        )
        .await?;
        Ok(ObjectPath::try_from(request).unwrap().into_owned())
    }

    #[zbus(signal)]
    async fn activated(
        emitter: &SignalEmitter<'_>,
        session: ObjectPath<'_>,
        shortcut_id: &str,
        timestamp: u64,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn deactivated(
        emitter: &SignalEmitter<'_>,
        session: ObjectPath<'_>,
        shortcut_id: &str,
        timestamp: u64,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;
}

async fn serve(log: Log) -> Connection {
    zbus::connection::Builder::session()
        .unwrap()
        .name("org.freedesktop.portal.Desktop")
        .unwrap()
        .serve_at(DESKTOP_PATH, RemoteDesktopPortal { log: log.clone() })
        .unwrap()
        .serve_at(DESKTOP_PATH, GlobalShortcutsPortal { log })
        .unwrap()
        .build()
        .await
        .expect("a session bus (run under dbus-run-session)")
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

#[derive(Default)]
struct SharedStore(Arc<MemoryGrantStore>);

impl GrantStore for SharedStore {
    fn load(&self) -> PortalGrants {
        self.0.load()
    }
    fn save(&self, grants: &PortalGrants) -> Result<(), String> {
        self.0.save(grants)
    }
}

#[test]
#[ignore = "needs a D-Bus session bus; CI runs it under dbus-run-session"]
fn portal_dbus_round_trip() {
    let log: Log = Arc::default();
    let server = async_io::block_on(serve(log.clone()));
    super::host::set_app_id("com.natkins.scribe.test");

    // RemoteDesktop: the first paste asks (no token), saves the token the
    // portal returns, and sends Shift+Insert as keysyms.
    let memory = Arc::new(MemoryGrantStore::default());
    let mut keyboard =
        PortalKeyboard::new(AshpdRemoteDesktop::default(), SharedStore(memory.clone()));
    keyboard
        .send(&PASTE_KEYS)
        .expect("paste through the portal");
    {
        let log = log.lock().unwrap();
        assert_eq!(log.persist_modes, vec![2], "asks to be remembered");
        assert_eq!(log.device_types, vec![1], "keyboard only");
        assert_eq!(log.restore_tokens_in, vec![None]);
        assert_eq!(
            log.keys,
            vec![
                (XK_SHIFT_L as i32, 1),
                (XK_INSERT as i32, 1),
                (XK_INSERT as i32, 0),
                (XK_SHIFT_L as i32, 0),
            ]
        );
    }
    assert_eq!(
        memory.load().remote_desktop_token.as_deref(),
        Some("token-1")
    );

    // The next session restores with the saved token and keeps the new one.
    keyboard.close_if_idle();
    keyboard.send(&PASTE_KEYS).unwrap();
    assert_eq!(
        log.lock().unwrap().restore_tokens_in,
        vec![None, Some("token-1".to_string())]
    );
    assert_eq!(
        memory.load().remote_desktop_token.as_deref(),
        Some("token-2")
    );

    // A revoked grant: the portal asks again, the user says no, Scribe
    // remembers that and stops asking.
    keyboard.close_if_idle();
    log.lock().unwrap().cancel_next_start = true;
    assert_eq!(keyboard.send(&PASTE_KEYS), Err(PortalFailure::Declined));
    let grants = memory.load();
    assert!(grants.remote_desktop_declined);
    assert_eq!(grants.remote_desktop_token, None);
    let starts = log.lock().unwrap().restore_tokens_in.len();
    assert_eq!(keyboard.send(&PASTE_KEYS), Err(PortalFailure::Declined));
    assert_eq!(log.lock().unwrap().restore_tokens_in.len(), starts);

    // GlobalShortcuts: binds every action with its suggested keys, then
    // delivers presses and releases.
    let events: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let seen = events.clone();
    let specs = vec![
        ShortcutSpec {
            id: "hold_to_talk".to_string(),
            description: "Hold to talk".to_string(),
            preferred_trigger: Some("CTRL+ALT+space".to_string()),
        },
        ShortcutSpec {
            id: "toggle_dictation".to_string(),
            description: "Toggle".to_string(),
            preferred_trigger: Some("CTRL+ALT+grave".to_string()),
        },
    ];
    assert!(shortcuts::start(
        specs,
        Arc::new(move |id: &str, down: bool| seen.lock().unwrap().push((id.to_string(), down))),
    ));
    assert!(wait_until(Duration::from_secs(5), || {
        shortcuts::status().is_some_and(|status| status.state == PortalShortcutsState::Active)
    }));
    let status = shortcuts::status().unwrap();
    assert_eq!(
        status.triggers.get("hold_to_talk").map(String::as_str),
        Some("CTRL+ALT+space")
    );
    assert_eq!(
        log.lock().unwrap().bound,
        vec![
            (
                "hold_to_talk".to_string(),
                Some("CTRL+ALT+space".to_string())
            ),
            (
                "toggle_dictation".to_string(),
                Some("CTRL+ALT+grave".to_string())
            ),
        ]
    );

    let session = log.lock().unwrap().sessions.last().cloned().unwrap();
    async_io::block_on(async {
        let emitter = SignalEmitter::new(&server, DESKTOP_PATH).unwrap();
        let session = ObjectPath::try_from(session.as_str()).unwrap();
        GlobalShortcutsPortal::activated(
            &emitter,
            session.clone(),
            "hold_to_talk",
            1,
            HashMap::new(),
        )
        .await
        .unwrap();
        GlobalShortcutsPortal::deactivated(&emitter, session, "hold_to_talk", 2, HashMap::new())
            .await
            .unwrap();
    });
    assert!(wait_until(Duration::from_secs(5), || events
        .lock()
        .unwrap()
        .len()
        == 2));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            ("hold_to_talk".to_string(), true),
            ("hold_to_talk".to_string(), false)
        ]
    );
}
