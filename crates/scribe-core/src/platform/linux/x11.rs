//! The X11 side of the Linux platform layer: which window is in front, handing
//! focus back, and synthetic keystrokes through the XTEST extension. Works on
//! X11 sessions and, for X11 apps, on Wayland sessions through XWayland.
//!
//! Every call opens its own short-lived connection, so nothing here holds
//! state that a display restart could leave stale.

use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use x11rb::{
    connection::Connection,
    protocol::{
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, InputFocus, Keycode,
            Keysym, Window, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
        },
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
    CURRENT_TIME, NONE,
};

pub const XK_SHIFT_L: Keysym = 0xffe1;
pub const XK_CONTROL_L: Keysym = 0xffe3;
pub const XK_INSERT: Keysym = 0xff63;
pub const XK_RETURN: Keysym = 0xff0d;
pub const XK_TAB: Keysym = 0xff09;
pub const XK_C: Keysym = 0x0063;
pub const XK_V: Keysym = 0x0076;

/// Shift, Control, Meta, Alt, Super, Hyper (left and right) and AltGr: the keys
/// whose held state would combine with injected keystrokes.
const MODIFIER_KEYSYMS: [Keysym; 13] = [
    0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe7, 0xffe8, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xffed, 0xffee,
    0xfe03,
];

/// The keysym that types `character`: the control keys by name, Latin-1 as
/// its own code, everything else as a Unicode keysym.
pub fn keysym_for_char(character: char) -> Keysym {
    match character {
        '\n' | '\r' => XK_RETURN,
        '\t' => XK_TAB,
        ' '..='~' | '\u{a0}'..='\u{ff}' => character as u32,
        other => 0x0100_0000 | other as u32,
    }
}

pub struct Display {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

struct Atoms {
    net_active_window: u32,
    net_wm_pid: u32,
    net_client_list_stacking: u32,
    wm_state: u32,
}

impl Display {
    /// `None` when there is no X server to talk to (a Wayland session
    /// without XWayland, or no desktop at all).
    pub fn open() -> Option<Self> {
        std::env::var_os("DISPLAY")?;
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let intern = |name: &str| -> Option<u32> {
            Some(
                conn.intern_atom(false, name.as_bytes())
                    .ok()?
                    .reply()
                    .ok()?
                    .atom,
            )
        };
        let atoms = Atoms {
            net_active_window: intern("_NET_ACTIVE_WINDOW")?,
            net_wm_pid: intern("_NET_WM_PID")?,
            net_client_list_stacking: intern("_NET_CLIENT_LIST_STACKING")?,
            wm_state: intern("WM_STATE")?,
        };
        Some(Self { conn, root, atoms })
    }

    fn property_u32s(&self, window: Window, property: u32, kind: AtomEnum) -> Vec<u32> {
        self.conn
            .get_property(false, window, property, kind, 0, 4096)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(|values| values.collect()))
            .unwrap_or_default()
    }

    fn has_wm_state(&self, window: Window) -> bool {
        self.conn
            .get_property(false, window, self.atoms.wm_state, AtomEnum::ANY, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.type_ != NONE)
    }

    fn parent_and_children(&self, window: Window) -> Option<(Window, Vec<Window>)> {
        let reply = self.conn.query_tree(window).ok()?.reply().ok()?;
        Some((reply.parent, reply.children))
    }

    /// The application window that holds `window`: itself or its nearest
    /// ancestor carrying WM_STATE, or, from a window-manager frame, the
    /// client window inside it.
    fn client_window(&self, window: Window) -> Option<Window> {
        let mut current = window;
        loop {
            if self.has_wm_state(current) {
                return Some(current);
            }
            let (parent, _) = self.parent_and_children(current)?;
            if parent == self.root || parent == NONE {
                break;
            }
            current = parent;
        }
        // `current` is a top-level window without WM_STATE: a frame. Look
        // for the client inside it.
        let mut level = vec![current];
        for _ in 0..3 {
            let mut next = Vec::new();
            for window in level {
                let (_, children) = self.parent_and_children(window)?;
                for child in children {
                    if self.has_wm_state(child) {
                        return Some(child);
                    }
                    next.push(child);
                }
            }
            level = next;
        }
        // No window manager: the top-level window is the app window.
        Some(current)
    }

    /// The application window with keyboard focus, or `None` when no X11
    /// window has it (a Wayland app in front, or the desktop).
    pub fn active_window(&self) -> Option<Window> {
        // A window manager that keeps _NET_ACTIVE_WINDOW is the authority,
        // including when it says no window is active (0).
        let active = self
            .property_u32s(self.root, self.atoms.net_active_window, AtomEnum::WINDOW)
            .first()
            .copied();
        if let Some(active) = active {
            return (active != NONE).then_some(active);
        }
        // No such window manager: ask the X server who has the focus.
        let focus = self.conn.get_input_focus().ok()?.reply().ok()?.focus;
        // 0 is None and 1 is PointerRoot.
        if focus <= 1 || focus == self.root {
            return None;
        }
        self.client_window(focus)
    }

    /// The process that owns `window`, from _NET_WM_PID. Zero when the app
    /// doesn't say.
    pub fn window_pid(&self, window: Window) -> u32 {
        self.property_u32s(window, self.atoms.net_wm_pid, AtomEnum::CARDINAL)
            .first()
            .copied()
            .unwrap_or(0)
    }

    /// WM_CLASS as (instance, class).
    pub fn window_class(&self, window: Window) -> Option<(String, String)> {
        let reply = self
            .conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()?
            .reply()
            .ok()?;
        let mut parts = reply
            .value
            .split(|byte| *byte == 0)
            .map(|part| String::from_utf8_lossy(part).into_owned());
        Some((parts.next()?, parts.next().unwrap_or_default()))
    }

    fn is_viewable(&self, window: Window) -> bool {
        self.conn
            .get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|attributes| {
                attributes.map_state == x11rb::protocol::xproto::MapState::VIEWABLE
            })
    }

    /// Application windows from the top of the stack down.
    pub fn windows_top_down(&self) -> Vec<Window> {
        let stacking = self.property_u32s(
            self.root,
            self.atoms.net_client_list_stacking,
            AtomEnum::WINDOW,
        );
        let mut windows: Vec<Window> = if stacking.is_empty() {
            // No EWMH window manager: the root's children are in stacking
            // order, bottom first.
            self.parent_and_children(self.root)
                .map(|(_, children)| children)
                .unwrap_or_default()
                .into_iter()
                .filter(|window| self.is_viewable(*window))
                .filter_map(|window| self.client_window(window))
                .collect()
        } else {
            stacking
        };
        windows.reverse();
        windows
    }

    /// Asks the window manager to activate `window` (and, without one, gives
    /// it the input focus directly).
    pub fn activate(&self, window: Window) -> bool {
        let message = ClientMessageEvent::new(
            32,
            window,
            self.atoms.net_active_window,
            // Source 2: a pager or similar acting for the user.
            [2u32, CURRENT_TIME, 0, 0, 0],
        );
        let sent = self
            .conn
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                message,
            )
            .is_ok();
        let _ = self
            .conn
            .set_input_focus(InputFocus::PARENT, window, CURRENT_TIME);
        let _ = self.conn.sync();
        sent
    }

    fn keyboard_mapping(&self) -> Option<KeyboardMapping> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)
            .ok()?
            .reply()
            .ok()?;
        Some(KeyboardMapping {
            first: min,
            per_keycode: reply.keysyms_per_keycode,
            keysyms: reply.keysyms,
        })
    }

    fn fake_key(&self, keycode: Keycode, press: bool) -> Result<(), String> {
        let kind = if press {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        };
        self.conn
            .xtest_fake_input(kind, keycode, CURRENT_TIME, self.root, 0, 0, 0)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn flush(&self) -> Result<(), String> {
        self.conn.sync().map_err(|error| error.to_string())
    }

    /// The keys held right now, split into (modifiers, other keys).
    fn held_keys(&self, mapping: &KeyboardMapping) -> (Vec<Keycode>, Vec<Keycode>) {
        let Some(keys) = self
            .conn
            .query_keymap()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.keys)
        else {
            return (Vec::new(), Vec::new());
        };
        let (mut modifiers, mut others) = (Vec::new(), Vec::new());
        for (byte_index, byte) in keys.iter().enumerate() {
            for bit in 0..8 {
                if byte & (1 << bit) == 0 {
                    continue;
                }
                let Ok(keycode) = Keycode::try_from(byte_index * 8 + bit) else {
                    continue;
                };
                if mapping
                    .keysyms_of(keycode)
                    .iter()
                    .any(|keysym| MODIFIER_KEYSYMS.contains(keysym))
                {
                    modifiers.push(keycode);
                } else {
                    others.push(keycode);
                }
            }
        }
        (modifiers, others)
    }

    /// Gets the keyboard ready for injected keys, after a hotkey started
    /// the action:
    ///
    /// - Waits up to 1.5 s for the hotkey's own key (V, R, ...) to come up.
    ///   While it is down, the X server routes every key event to Scribe's
    ///   hotkey grab, so injected keys would never reach the app.
    /// - With `wait_for_modifiers`, also waits for Ctrl, Alt and the rest,
    ///   which typing needs (a held Ctrl would turn letters into shortcuts).
    /// - Releases any modifier still held, so the injected keys can't
    ///   combine with it.
    pub fn release_modifiers(&self, wait_for_modifiers: bool) -> Result<(), String> {
        let mapping = self.keyboard_mapping().ok_or("no keyboard mapping")?;
        let deadline = Instant::now() + Duration::from_millis(1_500);
        loop {
            let (modifiers, others) = self.held_keys(&mapping);
            let waiting = !others.is_empty() || (wait_for_modifiers && !modifiers.is_empty());
            if !waiting || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }
        for keycode in self.held_keys(&mapping).0 {
            self.fake_key(keycode, false)?;
        }
        self.flush()
    }

    /// Presses `keys` in order and releases them in reverse: a shortcut such
    /// as Ctrl+V.
    pub fn send_chord(&self, keys: &[Keysym]) -> Result<(), String> {
        self.release_modifiers(false)?;
        let mapping = self.keyboard_mapping().ok_or("no keyboard mapping")?;
        let codes = keys
            .iter()
            .map(|keysym| {
                mapping
                    .find(*keysym)
                    .map(|(keycode, _)| keycode)
                    .ok_or_else(|| format!("no key for keysym 0x{keysym:x}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for keycode in &codes {
            self.fake_key(*keycode, true)?;
        }
        for keycode in codes.iter().rev() {
            self.fake_key(*keycode, false)?;
        }
        self.flush()
    }

    /// Types `text` key by key. Characters the keyboard layout has no key for
    /// are typed through a spare keycode that is remapped for each one and
    /// put back afterwards.
    pub fn type_text(&self, text: &str) -> Result<(), String> {
        self.release_modifiers(true)?;
        let mapping = self.keyboard_mapping().ok_or("no keyboard mapping")?;
        let shift = mapping.find(XK_SHIFT_L).map(|(keycode, _)| keycode);
        let spare = mapping.spare_keycode();
        let per_keycode = mapping.per_keycode;

        for character in text.chars() {
            let keysym = keysym_for_char(character);
            match mapping.find(keysym) {
                Some((keycode, level)) => {
                    let shifted = level == 1;
                    if shifted {
                        self.fake_key(shift.ok_or("no Shift key")?, true)?;
                    }
                    self.fake_key(keycode, true)?;
                    self.fake_key(keycode, false)?;
                    if shifted {
                        self.fake_key(shift.ok_or("no Shift key")?, false)?;
                    }
                    self.flush()?;
                }
                None => {
                    let spare = spare.ok_or("no spare key to type this character")?;
                    let row = vec![keysym; usize::from(per_keycode)];
                    self.conn
                        .change_keyboard_mapping(1, spare, per_keycode, &row)
                        .map_err(|error| error.to_string())?;
                    self.flush()?;
                    // Apps reload the keymap when the mapping-change event
                    // arrives; give them a moment before the key.
                    thread::sleep(Duration::from_millis(20));
                    self.fake_key(spare, true)?;
                    self.fake_key(spare, false)?;
                    self.flush()?;
                    thread::sleep(Duration::from_millis(20));
                    let empty = vec![0; usize::from(per_keycode)];
                    self.conn
                        .change_keyboard_mapping(1, spare, per_keycode, &empty)
                        .map_err(|error| error.to_string())?;
                    self.flush()?;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}

struct KeyboardMapping {
    first: Keycode,
    per_keycode: u8,
    keysyms: Vec<Keysym>,
}

impl KeyboardMapping {
    fn keysyms_of(&self, keycode: Keycode) -> &[Keysym] {
        let per = usize::from(self.per_keycode);
        let Some(index) = keycode.checked_sub(self.first).map(usize::from) else {
            return &[];
        };
        self.keysyms
            .get(index * per..(index + 1) * per)
            .unwrap_or(&[])
    }

    /// The keycode for `keysym` and the shift level it sits on (0 plain,
    /// 1 shifted). Only the first group is searched.
    fn find(&self, keysym: Keysym) -> Option<(Keycode, u8)> {
        let per = usize::from(self.per_keycode).max(1);
        for level in 0..per.min(2) {
            for (index, row) in self.keysyms.chunks(per).enumerate() {
                if row.get(level) == Some(&keysym) {
                    let keycode = Keycode::try_from(usize::from(self.first) + index).ok()?;
                    return Some((keycode, level as u8));
                }
            }
        }
        None
    }

    /// A keycode with nothing mapped to it, used to type characters the
    /// layout lacks. Searched from the top, where unused codes usually are.
    fn spare_keycode(&self) -> Option<Keycode> {
        let per = usize::from(self.per_keycode).max(1);
        self.keysyms
            .chunks(per)
            .enumerate()
            .rev()
            .find(|(_, row)| row.iter().all(|keysym| *keysym == 0))
            .and_then(|(index, _)| Keycode::try_from(usize::from(self.first) + index).ok())
    }
}

static CLIPBOARD_CHANGES: AtomicU32 = AtomicU32::new(0);
static CLIPBOARD_WATCHER: OnceLock<bool> = OnceLock::new();

/// A counter that moves whenever any app takes ownership of the CLIPBOARD
/// selection (that is, copies), like Windows' clipboard sequence number.
/// Starts a watcher thread on first use and waits until it is listening.
pub fn clipboard_change_count() -> u32 {
    CLIPBOARD_WATCHER.get_or_init(start_clipboard_watcher);
    CLIPBOARD_CHANGES.load(Ordering::SeqCst)
}

fn start_clipboard_watcher() -> bool {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();
    let spawned = thread::Builder::new()
        .name("scribe-clipboard-watch".to_string())
        .spawn(move || {
            let Some(display) = Display::open() else {
                let _ = ready_tx.send(false);
                return;
            };
            let conn = &display.conn;
            let listening = (|| {
                conn.xfixes_query_version(5, 0).ok()?.reply().ok()?;
                let clipboard = conn
                    .intern_atom(false, b"CLIPBOARD")
                    .ok()?
                    .reply()
                    .ok()?
                    .atom;
                conn.xfixes_select_selection_input(
                    display.root,
                    clipboard,
                    SelectionEventMask::SET_SELECTION_OWNER
                        | SelectionEventMask::SELECTION_WINDOW_DESTROY
                        | SelectionEventMask::SELECTION_CLIENT_CLOSE,
                )
                .ok()?;
                conn.sync().ok()
            })()
            .is_some();
            let _ = ready_tx.send(listening);
            if !listening {
                return;
            }
            while conn.wait_for_event().is_ok() {
                CLIPBOARD_CHANGES.fetch_add(1, Ordering::SeqCst);
            }
        });
    if spawned.is_err() {
        return false;
    }
    ready_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> KeyboardMapping {
        // Keycodes 8..=11: a/A, Shift_L, v/V, unused.
        KeyboardMapping {
            first: 8,
            per_keycode: 2,
            keysyms: vec![0x61, 0x41, XK_SHIFT_L, 0, 0x76, 0x56, 0, 0],
        }
    }

    #[test]
    fn characters_become_keysyms() {
        assert_eq!(keysym_for_char('a'), 0x61);
        assert_eq!(keysym_for_char('~'), 0x7e);
        assert_eq!(keysym_for_char('é'), 0xe9);
        assert_eq!(keysym_for_char('\n'), XK_RETURN);
        assert_eq!(keysym_for_char('\t'), XK_TAB);
        assert_eq!(keysym_for_char('€'), 0x0100_20ac);
        assert_eq!(keysym_for_char('😀'), 0x0101_f600);
    }

    #[test]
    fn finds_keys_and_their_shift_level() {
        let mapping = mapping();
        assert_eq!(mapping.find(0x61), Some((8, 0)));
        assert_eq!(mapping.find(0x41), Some((8, 1)));
        assert_eq!(mapping.find(XK_V), Some((10, 0)));
        assert_eq!(mapping.find(0xe9), None);
        assert_eq!(mapping.keysyms_of(9), &[XK_SHIFT_L, 0]);
        assert!(mapping.keysyms_of(7).is_empty());
    }

    #[test]
    fn the_spare_keycode_is_an_unmapped_one() {
        assert_eq!(mapping().spare_keycode(), Some(11));
        let full = KeyboardMapping {
            first: 8,
            per_keycode: 1,
            keysyms: vec![0x61],
        };
        assert_eq!(full.spare_keycode(), None);
    }
}
