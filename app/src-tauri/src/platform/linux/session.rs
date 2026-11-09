//! Which kind of desktop session Scribe runs in, and from that which way a
//! dictation reaches the app in front. Pure decisions over plain inputs, so
//! every combination is unit-tested without a display.

/// The display server the desktop session runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    X11,
    Wayland,
    /// No graphical session (a TTY, SSH, CI).
    Headless,
}

impl SessionKind {
    /// From `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY`. An explicit
    /// session type wins; without one (or "tty", "unspecified"), a Wayland
    /// socket means Wayland, since an X server next to it is XWayland.
    pub fn from_env_values(
        session_type: Option<&str>,
        wayland_display: Option<&str>,
        display: Option<&str>,
    ) -> Self {
        let set = |value: Option<&str>| value.is_some_and(|value| !value.trim().is_empty());
        match session_type.map(|kind| kind.trim().to_ascii_lowercase()) {
            Some(kind) if kind == "wayland" => return Self::Wayland,
            Some(kind) if kind == "x11" => return Self::X11,
            _ => {}
        }
        if set(wayland_display) {
            Self::Wayland
        } else if set(display) {
            Self::X11
        } else {
            Self::Headless
        }
    }

    pub fn detect() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self::from_env_values(
            var("XDG_SESSION_TYPE").as_deref(),
            var("WAYLAND_DISPLAY").as_deref(),
            var("DISPLAY").as_deref(),
        )
    }
}

/// How the RemoteDesktop portal stands, as far as pasting goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalPaste {
    /// Granted before (a restore token is saved), or never asked yet: the
    /// first paste asks once, and the answer is remembered.
    Usable,
    /// The user turned the request down. Scribe doesn't ask again on its
    /// own; the Dictation page has a button for it.
    Declined,
    /// No RemoteDesktop portal, or one too old to remember a grant (it would
    /// ask on every paste).
    Unavailable,
}

/// The way a paste or typed text reaches the app in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPath {
    /// XTEST keystrokes: an X11 session, or an X11 (XWayland) app in front
    /// on Wayland.
    X11,
    /// The XDG RemoteDesktop portal, for native Wayland apps.
    Portal,
    /// Nothing can send keystrokes: the text is copied for the user.
    CopyOnly,
}

/// Picks the insert path. `x11_app_in_front` is true when an X server is
/// reachable and one of its windows has the keyboard focus.
pub fn insert_path(
    session: SessionKind,
    x11_app_in_front: bool,
    portal: PortalPaste,
) -> InsertPath {
    match session {
        // Even with no window focused (the desktop) an X11 session pastes
        // through X11: the keys go nowhere harmful.
        SessionKind::X11 => InsertPath::X11,
        SessionKind::Wayland if x11_app_in_front => InsertPath::X11,
        SessionKind::Wayland if portal == PortalPaste::Usable => InsertPath::Portal,
        SessionKind::Wayland | SessionKind::Headless => InsertPath::CopyOnly,
    }
}

/// The method the Dashboard reports, independent of which app is in front.
pub fn session_insert_path(
    session: SessionKind,
    x11_reachable: bool,
    portal: PortalPaste,
) -> InsertPath {
    match session {
        SessionKind::X11 if x11_reachable => InsertPath::X11,
        SessionKind::Wayland if portal == PortalPaste::Usable => InsertPath::Portal,
        _ => InsertPath::CopyOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_type_decides_when_it_is_set() {
        assert_eq!(
            SessionKind::from_env_values(Some("wayland"), None, Some(":0")),
            SessionKind::Wayland
        );
        assert_eq!(
            SessionKind::from_env_values(Some("x11"), Some("wayland-0"), Some(":0")),
            SessionKind::X11
        );
        assert_eq!(
            SessionKind::from_env_values(Some("Wayland "), None, None),
            SessionKind::Wayland
        );
    }

    #[test]
    fn without_a_session_type_the_sockets_decide() {
        // WSLg and some display managers set both sockets and no type.
        assert_eq!(
            SessionKind::from_env_values(None, Some("wayland-0"), Some(":0")),
            SessionKind::Wayland
        );
        assert_eq!(
            SessionKind::from_env_values(Some("tty"), None, Some(":1")),
            SessionKind::X11
        );
        assert_eq!(
            SessionKind::from_env_values(Some("unspecified"), Some(""), Some("")),
            SessionKind::Headless
        );
        assert_eq!(
            SessionKind::from_env_values(None, None, None),
            SessionKind::Headless
        );
    }

    #[test]
    fn x11_sessions_always_paste_through_x11() {
        for portal in [
            PortalPaste::Usable,
            PortalPaste::Declined,
            PortalPaste::Unavailable,
        ] {
            assert_eq!(insert_path(SessionKind::X11, true, portal), InsertPath::X11);
            assert_eq!(
                insert_path(SessionKind::X11, false, portal),
                InsertPath::X11
            );
        }
    }

    #[test]
    fn wayland_uses_x11_for_xwayland_apps_and_the_portal_for_the_rest() {
        assert_eq!(
            insert_path(SessionKind::Wayland, true, PortalPaste::Unavailable),
            InsertPath::X11
        );
        assert_eq!(
            insert_path(SessionKind::Wayland, false, PortalPaste::Usable),
            InsertPath::Portal
        );
    }

    #[test]
    fn wayland_copies_when_the_portal_is_missing_or_declined() {
        assert_eq!(
            insert_path(SessionKind::Wayland, false, PortalPaste::Declined),
            InsertPath::CopyOnly
        );
        assert_eq!(
            insert_path(SessionKind::Wayland, false, PortalPaste::Unavailable),
            InsertPath::CopyOnly
        );
        assert_eq!(
            insert_path(SessionKind::Headless, false, PortalPaste::Usable),
            InsertPath::CopyOnly
        );
    }

    #[test]
    fn the_dashboard_method_follows_the_session() {
        assert_eq!(
            session_insert_path(SessionKind::X11, true, PortalPaste::Unavailable),
            InsertPath::X11
        );
        assert_eq!(
            session_insert_path(SessionKind::X11, false, PortalPaste::Usable),
            InsertPath::CopyOnly
        );
        assert_eq!(
            session_insert_path(SessionKind::Wayland, true, PortalPaste::Usable),
            InsertPath::Portal
        );
        assert_eq!(
            session_insert_path(SessionKind::Wayland, true, PortalPaste::Declined),
            InsertPath::CopyOnly
        );
    }
}
