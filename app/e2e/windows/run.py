"""Scribe's Windows end-to-end test.

Drives a Scribe Dev build made with the `e2e-audio` feature the way a person
would: global hotkeys pressed with injected keystrokes, dictation into stand-in
app windows (target.ps1), the clipboard, and app switches. Hosted runners have
no microphone, so the app records a WAV file in place of one (see
crates/scribe-core/src/test_audio.rs); hotkeys, recording, Whisper and output all
run for real.

Every check reads real state: the target window's text, the clipboard,
status.json, the app's database and its log. Waits poll for a state, never a
fixed sleep.

Python standard library only. Usage:
    python run.py --exe "path\\Scribe Dev.exe" --fixtures DIR --out DIR
"""

from __future__ import annotations

import argparse
import base64
import ctypes
import json
import os
import re
import shutil
import socket
import sqlite3
import struct
import subprocess
import sys
import threading
import time
import traceback
import urllib.parse
import urllib.request
import zlib
from ctypes import wintypes
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import folder_mismatch  # noqa: E402  (app/e2e/folder_mismatch.py, shared with the Linux and macOS smoke scripts)

IDENTIFIER = "com.natkins.scribe.dev"
DATA_DIR = Path(os.environ.get("APPDATA", "")) / IDENTIFIER
CACHE_DIR = Path(os.environ.get("LOCALAPPDATA", "")) / IDENTIFIER
DB_PATH = DATA_DIR / "scribe.sqlite3"
STATUS_PATH = CACHE_DIR / "status.json"
LOG_DIR = CACHE_DIR / "logs"
CLIP_FINISHED_LOG = "E2E test audio: clip finished"
TEST_AUDIO_ON_LOG = "E2E test audio is on"

# Test-only hotkeys nothing else on a runner uses.
VK_CONTROL, VK_MENU = 0x11, 0x12
HOTKEYS = {
    "holdToTalk": ("Ctrl+Alt+F7", 0x76),
    "toggleDictation": ("Ctrl+Alt+F8", 0x77),
    "pasteLastTranscript": ("Ctrl+Alt+F9", 0x78),
    "discardDictation": ("Ctrl+Alt+F10", 0x79),
    "transformSelection": ("Ctrl+Alt+F11", 0x7A),
    "openDashboard": ("Ctrl+Alt+F12", 0x7B),
}

PARAKEET_ID = "parakeet-tdt-0.6b-v3"
JFK_WORDS = ["fellow", "americans", "ask", "country", "can", "do", "for", "you"]
CLIPS = {
    "hold": "Please schedule the meeting for next Tuesday afternoon.",
    "save_only": "Remember to water the garden this evening.",
    "copy_clipboard": "The library opens at nine in the morning.",
    "copy_and_paste": "Our team finished the quarterly report early.",
    "direct_insert": "Fresh coffee tastes better with warm milk.",
    "switch": "Send the invoice to the accounting department today.",
    "discard": "This sentence should never be pasted anywhere at all.",
    "instruction": "Make this text uppercase, please.",
    "slow_app": "The busy spreadsheet finally opened after lunch.",
    "ignored_paste": "Nobody reads the clipboard in this window today.",
    # The default-style hotkey group.
    "dev_hold": "Bring a warm jacket because the evening gets cold.",
    "bare_toggle": "Please book a table for four people tonight.",
    "note_chord": "Buy fresh bread and green apples on the way home.",
    "chord_hold": "The train leaves the station at half past seven.",
    "press_trigger": "Our garden needs more sunflowers next spring.",
    "rebind_key": "Call the plumber about the leaking kitchen sink.",
}
# Log lines of the Windows toggle-key watcher and the note chord
# (crates/scribe-core/src/hotkeys.rs); the chord watcher logs nothing.
TOGGLE_WATCHER_PREFIX = "Toggle key watcher active"
TOGGLE_WATCHER_LOG = TOGGLE_WATCHER_PREFIX + " (vk 0x{vk:X}, trigger {trigger})"
TOGGLE_PRESSED_LOG = "Toggle key pressed (trigger Press)"
TOGGLE_RELEASED_LOG = "Toggle key released (note chord fired: {fired})"
NOTE_CHORD_LOG = "Note chord (Q) pressed during toggle hold"
NOTE_CHORD_UNAVAILABLE_LOG = "Note chord unavailable"
# The chord watcher's arming delay (windows_chord::ARMING_DELAY): a
# modifier-only chord released sooner starts nothing.
CHORD_ARMING_DELAY = 0.150
# When the runner probes whether Scribe holds Q (wait_q_grabbed says why).
Q_PROBE_FIRST_DELAY = 0.05
Q_PROBE_INTERVAL = 0.1
# How long a held toggle key on Release is watched for a recording that
# must not start before the key comes up: a start takes well under this.
RELEASE_HOLD_OBSERVATION = 0.5
# How long the slow stand-in app is busy before it reads the clipboard on
# Ctrl+V (target.ps1 -PasteDelayMs); well past Scribe's old fixed 180 ms wait.
SLOW_APP_PASTE_DELAY_MS = 1000
TRANSFORM_SOURCE = "scribe turns this selection into capitals"

BUSY = {"Recording", "Stopping", "Transcribing", "Pasting"}

# --------------------------------------------------------------------------
# Win32
# --------------------------------------------------------------------------

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.c_void_p]
kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.c_void_p]
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)

ULONG_PTR = ctypes.c_size_t


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [
        ("uMsg", wintypes.DWORD),
        ("wParamL", wintypes.WORD),
        ("wParamH", wintypes.WORD),
    ]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("u", _INPUTUNION)]


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("hwndActive", wintypes.HWND),
        ("hwndFocus", wintypes.HWND),
        ("hwndCapture", wintypes.HWND),
        ("hwndMenuOwner", wintypes.HWND),
        ("hwndMoveSize", wintypes.HWND),
        ("hwndCaret", wintypes.HWND),
        ("rcCaret", wintypes.RECT),
    ]


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [
        ("biSize", wintypes.DWORD),
        ("biWidth", wintypes.LONG),
        ("biHeight", wintypes.LONG),
        ("biPlanes", wintypes.WORD),
        ("biBitCount", wintypes.WORD),
        ("biCompression", wintypes.DWORD),
        ("biSizeImage", wintypes.DWORD),
        ("biXPelsPerMeter", wintypes.LONG),
        ("biYPelsPerMeter", wintypes.LONG),
        ("biClrUsed", wintypes.DWORD),
        ("biClrImportant", wintypes.DWORD),
    ]


INPUT_MOUSE, INPUT_KEYBOARD = 0, 1
KEYEVENTF_KEYUP, KEYEVENTF_UNICODE = 0x2, 0x4
MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP = 0x2, 0x4
WM_SETTEXT, WM_GETTEXT, WM_GETTEXTLENGTH, WM_CLOSE = 0x000C, 0x000D, 0x000E, 0x0010
EM_SETSEL = 0x00B1
SMTO_ABORTIFHUNG = 0x2
CF_UNICODETEXT = 13
GMEM_MOVEABLE = 0x2

WNDENUMPROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
user32.SendInput.restype = wintypes.UINT
user32.GetForegroundWindow.restype = wintypes.HWND
user32.SetForegroundWindow.argtypes = [wintypes.HWND]
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.GetWindowThreadProcessId.restype = wintypes.DWORD
user32.GetGUIThreadInfo.argtypes = [wintypes.DWORD, ctypes.POINTER(GUITHREADINFO)]
user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wintypes.HWND]
user32.IsWindow.argtypes = [wintypes.HWND]
user32.EnumWindows.argtypes = [WNDENUMPROC, wintypes.LPARAM]
user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.SendMessageTimeoutW.argtypes = [
    wintypes.HWND,
    wintypes.UINT,
    wintypes.WPARAM,
    wintypes.LPARAM,
    wintypes.UINT,
    wintypes.UINT,
    ctypes.POINTER(ULONG_PTR),
]
user32.SendMessageTimeoutW.restype = wintypes.LPARAM
user32.OpenClipboard.argtypes = [wintypes.HWND]
user32.GetClipboardData.argtypes = [wintypes.UINT]
user32.GetClipboardData.restype = wintypes.HANDLE
user32.SetClipboardData.argtypes = [wintypes.UINT, wintypes.HANDLE]
user32.SetClipboardData.restype = wintypes.HANDLE
user32.GetClipboardSequenceNumber.restype = wintypes.DWORD
user32.CreateWindowExW.argtypes = [
    wintypes.DWORD,
    wintypes.LPCWSTR,
    wintypes.LPCWSTR,
    wintypes.DWORD,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    wintypes.HWND,
    wintypes.HMENU,
    wintypes.HINSTANCE,
    wintypes.LPVOID,
]
user32.CreateWindowExW.restype = wintypes.HWND
user32.GetMessageW.argtypes = [ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT, wintypes.UINT]
user32.TranslateMessage.argtypes = [ctypes.POINTER(wintypes.MSG)]
user32.DispatchMessageW.argtypes = [ctypes.POINTER(wintypes.MSG)]
user32.GetDC.argtypes = [wintypes.HWND]
user32.GetDC.restype = wintypes.HDC
user32.ReleaseDC.argtypes = [wintypes.HWND, wintypes.HDC]
user32.GetSystemMetrics.argtypes = [ctypes.c_int]
kernel32.GlobalAlloc.argtypes = [wintypes.UINT, ctypes.c_size_t]
kernel32.GlobalAlloc.restype = wintypes.HGLOBAL
kernel32.GlobalLock.argtypes = [wintypes.HGLOBAL]
kernel32.GlobalLock.restype = wintypes.LPVOID
kernel32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
gdi32.CreateCompatibleDC.argtypes = [wintypes.HDC]
gdi32.CreateCompatibleDC.restype = wintypes.HDC
gdi32.CreateCompatibleBitmap.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = wintypes.HBITMAP
gdi32.SelectObject.argtypes = [wintypes.HDC, wintypes.HGDIOBJ]
gdi32.SelectObject.restype = wintypes.HGDIOBJ
gdi32.BitBlt.argtypes = [
    wintypes.HDC,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    wintypes.HDC,
    ctypes.c_int,
    ctypes.c_int,
    wintypes.DWORD,
]
gdi32.GetDIBits.argtypes = [
    wintypes.HDC,
    wintypes.HBITMAP,
    wintypes.UINT,
    wintypes.UINT,
    wintypes.LPVOID,
    ctypes.c_void_p,
    wintypes.UINT,
]
gdi32.DeleteObject.argtypes = [wintypes.HGDIOBJ]
gdi32.DeleteDC.argtypes = [wintypes.HDC]
user32.RegisterHotKey.argtypes = [wintypes.HWND, ctypes.c_int, wintypes.UINT, wintypes.UINT]
user32.UnregisterHotKey.argtypes = [wintypes.HWND, ctypes.c_int]
user32.MapVirtualKeyW.argtypes = [wintypes.UINT, wintypes.UINT]
user32.MapVirtualKeyW.restype = wintypes.UINT

user32.SetProcessDPIAware()


def send_inputs(inputs: list[INPUT]) -> None:
    array = (INPUT * len(inputs))(*inputs)
    sent = user32.SendInput(len(inputs), array, ctypes.sizeof(INPUT))
    if sent != len(inputs):
        raise OSError(f"SendInput sent {sent} of {len(inputs)} events: {ctypes.get_last_error()}")


def key(vk: int, up: bool) -> None:
    event = INPUT(type=INPUT_KEYBOARD)
    event.u.ki = KEYBDINPUT(wVk=vk, dwFlags=KEYEVENTF_KEYUP if up else 0)
    send_inputs([event])


def chord_down(action: str) -> None:
    for vk in (VK_CONTROL, VK_MENU, HOTKEYS[action][1]):
        key(vk, False)
        time.sleep(0.03)


def chord_up(action: str) -> None:
    for vk in (HOTKEYS[action][1], VK_MENU, VK_CONTROL):
        key(vk, True)
        time.sleep(0.03)


def press(action: str) -> None:
    """Presses and releases a Scribe hotkey, like a person tapping the chord."""
    chord_down(action)
    time.sleep(0.05)
    chord_up(action)


# Keys as a hardware keyboard sends them, for the default-style hotkeys. Scribe's
# Windows watchers poll GetAsyncKeyState (the chord watcher every 30 ms, the
# toggle-key watcher every 10 ms) and the global-shortcut plugin uses
# RegisterHotKey. SendInput with scan codes goes through the same raw input
# path as a keyboard: it updates the async key state, trips RegisterHotKey and
# low-level hooks, and the focused window types the character through its
# keyboard layout. Messages posted to a window would reach none of these.
KEYEVENTF_EXTENDEDKEY, KEYEVENTF_SCANCODE = 0x1, 0x8
MAPVK_VK_TO_VSC = 0
VK_LSHIFT, VK_LCONTROL, VK_LMENU, VK_LWIN = 0xA0, 0xA2, 0xA4, 0x5B
VK_Q, VK_OEM_3 = 0x51, 0xC0
# Keys whose scan code carries the E0 prefix.
EXTENDED_VKS = {0x5B, 0x5C, 0xA3, 0xA5, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2D, 0x2E, 0x6F}
MODIFIER_NAMES = {
    "ctrl": VK_LCONTROL,
    "control": VK_LCONTROL,
    "shift": VK_LSHIFT,
    "alt": VK_LMENU,
    "option": VK_LMENU,
    "win": VK_LWIN,
    "windows": VK_LWIN,
    "super": VK_LWIN,
    "meta": VK_LWIN,
    "cmd": VK_LWIN,
    "command": VK_LWIN,
}
# RegisterHotKey's modifier flags for each modifier key.
MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, MOD_NOREPEAT = 0x1, 0x2, 0x4, 0x8, 0x4000
HOTKEY_MOD_FLAGS = {VK_LCONTROL: MOD_CONTROL, VK_LSHIFT: MOD_SHIFT, VK_LMENU: MOD_ALT, VK_LWIN: MOD_WIN}
KEY_NAMES = {
    "backquote": VK_OEM_3,
    "`": VK_OEM_3,
    "~": VK_OEM_3,
    "tilde": VK_OEM_3,
    "space": 0x20,
    "minus": 0xBD,
    "equal": 0xBB,
    "bracketleft": 0xDB,
    "bracketright": 0xDD,
    "backslash": 0xDC,
    "semicolon": 0xBA,
    "quote": 0xDE,
    "comma": 0xBC,
    "period": 0xBE,
    "slash": 0xBF,
    **{chr(c).lower(): c for c in range(ord("A"), ord("Z") + 1)},
    **{f"key{chr(c).lower()}": c for c in range(ord("A"), ord("Z") + 1)},
    **{str(d): 0x30 + d for d in range(10)},
    **{f"digit{d}": 0x30 + d for d in range(10)},
    **{f"f{n}": 0x6F + n for n in range(1, 25)},
}


class Chord:
    """A hotkey as Scribe stores it ("Ctrl+Win", "Backquote", "Ctrl+Alt+F"),
    as the keys that press it."""

    def __init__(self, text: str) -> None:
        self.text = text
        self.modifiers: list[int] = []
        self.key: int | None = None
        for token in (part.strip().lower() for part in text.split("+")):
            if token in MODIFIER_NAMES:
                self.modifiers.append(MODIFIER_NAMES[token])
            elif token in KEY_NAMES and self.key is None:
                self.key = KEY_NAMES[token]
            else:
                raise CheckFailed(f"The runner cannot press {text!r} (no key for {token!r})")

    @property
    def modifier_only(self) -> bool:
        return self.key is None and bool(self.modifiers)

    @property
    def bare_key(self) -> bool:
        return self.key is not None and not self.modifiers

    def __repr__(self) -> str:
        return self.text


def key_input(vk: int, up: bool) -> INPUT:
    flags = KEYEVENTF_SCANCODE | (KEYEVENTF_KEYUP if up else 0)
    if vk in EXTENDED_VKS:
        flags |= KEYEVENTF_EXTENDEDKEY
    event = INPUT(type=INPUT_KEYBOARD)
    event.u.ki = KEYBDINPUT(wVk=0, wScan=user32.MapVirtualKeyW(vk, MAPVK_VK_TO_VSC), dwFlags=flags)
    return event


class Keyboard:
    """Sends keys by scan code and remembers which are down, so a failed
    scenario can let go of every key it held."""

    def __init__(self) -> None:
        self.held: list[int] = []

    def send(self, vks: list[int], up: bool) -> None:
        # A key counts as held before its down event is sent, so a SendInput
        # that fails part-way still gets every key it may have pressed let go;
        # it stops counting only once its up event went through.
        if not up:
            self.held += [vk for vk in vks if vk not in self.held]
        send_inputs([key_input(vk, up) for vk in vks])
        if up:
            self.held = [vk for vk in self.held if vk not in vks]

    def down(self, vk: int) -> None:
        self.send([vk], False)

    def up(self, vk: int) -> None:
        self.send([vk], True)

    def tap(self, vk: int) -> None:
        self.down(vk)
        time.sleep(0.03)
        self.up(vk)

    def hold(self, chord: Chord) -> None:
        """Presses a chord the way a hand does: modifiers first, a beat apart."""
        for vk in chord.modifiers + ([chord.key] if chord.key is not None else []):
            self.down(vk)
            time.sleep(0.03)

    def release(self, chord: Chord) -> None:
        for vk in ([chord.key] if chord.key is not None else []) + chord.modifiers[::-1]:
            self.up(vk)
            time.sleep(0.03)

    def press(self, chord: Chord) -> None:
        self.hold(chord)
        time.sleep(0.05)
        self.release(chord)

    def release_all(self) -> list[int]:
        held = list(self.held)
        for vk in held[::-1]:
            try:
                self.up(vk)
            except OSError as error:
                print(f"Could not release key 0x{vk:X}: {error}")
        return held


PROBE_HOTKEY_ID = 0xB0E2


def hotkey_grabbed(chord: Chord) -> bool:
    """Whether some program holds `chord` as a global hotkey (RegisterHotKey).
    The runner tries to register the same keys itself: Windows refuses with
    ERROR_HOTKEY_ALREADY_REGISTERED while Scribe holds them, and a successful
    probe is released again at once."""
    if chord.key is None:
        raise CheckFailed(f"{chord} has no key to probe")
    mods = MOD_NOREPEAT
    for vk in chord.modifiers:
        mods |= HOTKEY_MOD_FLAGS[vk]
    if user32.RegisterHotKey(None, PROBE_HOTKEY_ID, mods, chord.key):
        user32.UnregisterHotKey(None, PROBE_HOTKEY_ID)
        return False
    error = ctypes.get_last_error()
    if error == 1409:  # ERROR_HOTKEY_ALREADY_REGISTERED
        return True
    raise OSError(f"Probing {chord} with RegisterHotKey failed: {error}")


Q_KEY = Chord("Q")


def click(x: int, y: int) -> None:
    user32.SetCursorPos(x, y)
    events = []
    for flag in (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP):
        event = INPUT(type=INPUT_MOUSE)
        event.u.mi = MOUSEINPUT(dwFlags=flag)
        events.append(event)
    send_inputs(events)


def window_pid(hwnd: int) -> int:
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def focused_control(hwnd: int) -> int | None:
    """The control with keyboard focus in the thread that owns `hwnd`."""
    thread = user32.GetWindowThreadProcessId(hwnd, None)
    info = GUITHREADINFO(cbSize=ctypes.sizeof(GUITHREADINFO))
    if not user32.GetGUIThreadInfo(thread, ctypes.byref(info)):
        return None
    return info.hwndFocus


def send_message(hwnd: int, message: int, wparam: int = 0, lparam=0) -> int:
    result = ULONG_PTR()
    if not user32.SendMessageTimeoutW(
        hwnd, message, wparam, lparam, SMTO_ABORTIFHUNG, 5000, ctypes.byref(result)
    ):
        raise OSError(f"Message 0x{message:X} to window {hwnd} timed out")
    return result.value


def window_title(hwnd: int) -> str:
    buffer = ctypes.create_unicode_buffer(512)
    user32.GetWindowTextW(hwnd, buffer, 512)
    return buffer.value


def top_windows(pid: int) -> list[int]:
    found: list[int] = []

    def visit(hwnd, _):
        if window_pid(hwnd) == pid:
            found.append(hwnd)
        return True

    user32.EnumWindows(WNDENUMPROC(visit), 0)
    return found


class Clipboard:
    """The system clipboard, owned through a hidden window of our own (a null
    owner makes SetClipboardData fail). The window lives on a thread that
    pumps messages, because the next app to empty the clipboard sends its
    owner WM_DESTROYCLIPBOARD and waits for the answer."""

    def __init__(self) -> None:
        self.owner = None
        ready = threading.Event()
        threading.Thread(target=self._pump, args=(ready,), daemon=True).start()
        ready.wait(10)
        if not self.owner:
            raise OSError("Could not create the clipboard window")

    def _pump(self, ready: threading.Event) -> None:
        self.owner = user32.CreateWindowExW(0, "STATIC", "scribe-e2e-clipboard", 0, 0, 0, 0, 0, None, None, None, None)
        ready.set()
        message = wintypes.MSG()
        while user32.GetMessageW(ctypes.byref(message), None, 0, 0) > 0:
            user32.TranslateMessage(ctypes.byref(message))
            user32.DispatchMessageW(ctypes.byref(message))

    def _open(self) -> None:
        deadline = time.monotonic() + 5
        while not user32.OpenClipboard(self.owner):
            if time.monotonic() > deadline:
                raise OSError("Could not open the clipboard")
            time.sleep(0.02)

    def read(self) -> str | None:
        self._open()
        try:
            handle = user32.GetClipboardData(CF_UNICODETEXT)
            if not handle:
                return None
            pointer = kernel32.GlobalLock(handle)
            try:
                return ctypes.wstring_at(pointer)
            finally:
                kernel32.GlobalUnlock(handle)
        finally:
            user32.CloseClipboard()

    def write(self, text: str) -> None:
        data = ctypes.create_unicode_buffer(text)
        size = ctypes.sizeof(data)
        handle = kernel32.GlobalAlloc(GMEM_MOVEABLE, size)
        pointer = kernel32.GlobalLock(handle)
        ctypes.memmove(pointer, data, size)
        kernel32.GlobalUnlock(handle)
        self._open()
        try:
            user32.EmptyClipboard()
            if not user32.SetClipboardData(CF_UNICODETEXT, handle):
                raise OSError(f"SetClipboardData failed: {ctypes.get_last_error()}")
        finally:
            user32.CloseClipboard()

    @staticmethod
    def sequence() -> int:
        return user32.GetClipboardSequenceNumber()


def screenshot(path: Path) -> None:
    """Saves the whole screen as a PNG (GDI capture, no dependencies)."""
    width, height = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    screen = user32.GetDC(None)
    memory = gdi32.CreateCompatibleDC(screen)
    bitmap = gdi32.CreateCompatibleBitmap(screen, width, height)
    gdi32.SelectObject(memory, bitmap)
    gdi32.BitBlt(memory, 0, 0, width, height, screen, 0, 0, 0x00CC0020 | 0x40000000)
    header = BITMAPINFOHEADER(
        biSize=ctypes.sizeof(BITMAPINFOHEADER), biWidth=width, biHeight=-height, biPlanes=1, biBitCount=32
    )
    pixels = ctypes.create_string_buffer(width * height * 4)
    gdi32.GetDIBits(memory, bitmap, 0, height, pixels, ctypes.byref(header), 0)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(memory)
    user32.ReleaseDC(None, screen)

    bgra = pixels.raw
    rgb = bytearray(width * height * 3)
    rgb[0::3], rgb[1::3], rgb[2::3] = bgra[2::4], bgra[1::4], bgra[0::4]
    stride = width * 3
    raw = b"".join(b"\x00" + bytes(rgb[row * stride : (row + 1) * stride]) for row in range(height))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


# --------------------------------------------------------------------------
# Helpers
# --------------------------------------------------------------------------


class CheckFailed(Exception):
    pass


def wait_until(describe: str, predicate, timeout: float = 30.0, interval: float = 0.05):
    """Polls `predicate` until it returns something truthy; fails with
    `describe` and the app's status when it never does."""
    deadline = time.monotonic() + timeout
    while True:
        value = predicate()
        if value:
            return value
        if time.monotonic() > deadline:
            raise CheckFailed(f"Timed out after {timeout:.0f} s waiting for {describe} (app status: {read_status()})")
        time.sleep(interval)


def stays(describe: str, predicate, seconds: float = 1.5) -> None:
    """Checks that `predicate` holds for a short observation window. Used only
    for "nothing happened" checks, which have no event to wait for."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not predicate():
            raise CheckFailed(f"Expected {describe}")
        time.sleep(0.05)


def check(condition: bool, message: str) -> None:
    if not condition:
        raise CheckFailed(message)


def words(text: str) -> list[str]:
    return re.findall(r"[a-z0-9']+", text.lower())


def check_words(text: str, expected: list[str], what: str) -> None:
    """Transcripts are checked by key-word overlap, not exact text."""
    heard = set(words(text))
    hits = [word for word in expected if word in heard]
    check(
        len(hits) >= max(1, round(len(expected) * 0.6)),
        f"{what} {text!r} shares only {hits} with the expected words {expected}",
    )


def key_words(sentence: str) -> list[str]:
    return [word for word in words(sentence) if len(word) > 3]


def read_status() -> dict | None:
    try:
        return json.loads(STATUS_PATH.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def log_text() -> str:
    parts = []
    for path in sorted(LOG_DIR.glob("*.log")):
        try:
            parts.append(path.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            pass
    return "\n".join(parts)


def db() -> sqlite3.Connection:
    return sqlite3.connect(DB_PATH, timeout=10)


def setting_value(key_name: str) -> str | None:
    with db() as connection:
        row = connection.execute("SELECT value FROM settings WHERE key = ?", (key_name,)).fetchone()
    return row[0] if row else None


def patch_settings(patch: dict) -> None:
    """Changes stored settings the way the app's own patch does: named fields
    only. The app reads settings from the database for every dictation."""
    with db() as connection:
        row = connection.execute("SELECT value FROM settings WHERE key = 'app_settings'").fetchone()
        if not row:
            raise CheckFailed("The app saved no settings on its first run")
        settings = json.loads(row[0])
        for name, value in patch.items():
            if name not in settings:
                raise CheckFailed(f"{name} is not a setting")
            if isinstance(value, dict):
                settings[name].update(value)
            else:
                settings[name] = value
        connection.execute(
            "UPDATE settings SET value = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE key = 'app_settings'",
            (json.dumps(settings),),
        )


def last_transcript() -> dict | None:
    transcript_id = setting_value("last_transcript_id")
    if not transcript_id:
        return None
    with db() as connection:
        row = connection.execute(
            "SELECT id, text, output_mode, paste_method FROM transcripts WHERE id = ?", (transcript_id,)
        ).fetchone()
    if not row:
        return {"id": transcript_id, "text": None, "output_mode": None, "paste_method": None}
    return dict(zip(("id", "text", "output_mode", "paste_method"), row))


def transcript_count() -> int:
    with db() as connection:
        return connection.execute("SELECT COUNT(*) FROM transcripts").fetchone()[0]


def notes() -> dict[str, str]:
    """Saved notes (dictations made with the note chord), by id."""
    with db() as connection:
        rows = connection.execute("SELECT id, text FROM transcripts WHERE is_note = 1").fetchall()
    return dict(rows)


def log_lines(*markers: str) -> list[str]:
    return [line for line in log_text().splitlines() if any(marker in line for marker in markers)]


# --------------------------------------------------------------------------
# Stand-in apps and the local AI server
# --------------------------------------------------------------------------


class Target:
    """A stand-in app window (target.ps1) in its own process."""

    def __init__(self, name: str, work: Path, left: int, top: int = 40, paste_delay_ms: int = 0) -> None:
        self.name = name
        self.title = f"Scribe E2E target {name}"
        info = work / f"target-{name}.json"
        info.unlink(missing_ok=True)
        self.process = subprocess.Popen(
            [
                "powershell.exe",
                "-NoProfile",
                "-STA",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(HERE / "target.ps1"),
                "-Title",
                self.title,
                "-InfoPath",
                str(info),
                "-Left",
                str(left),
                "-Top",
                str(top),
                "-PasteDelayMs",
                str(paste_delay_ms),
            ]
        )
        data = json.loads(wait_until(f"target window {name} to open", lambda: info.exists() and info.read_text(), 60))
        self.form, self.box = int(data["form"]), int(data["box"])

    @property
    def text(self) -> str:
        length = send_message(self.box, WM_GETTEXTLENGTH)
        buffer = ctypes.create_unicode_buffer(length + 1)
        send_message(self.box, WM_GETTEXT, length + 1, ctypes.addressof(buffer))
        return buffer.value

    def set_text(self, text: str) -> None:
        buffer = ctypes.create_unicode_buffer(text)
        send_message(self.box, WM_SETTEXT, 0, ctypes.addressof(buffer))
        check(self.text == text, f"Could not set the text of target {self.name}")

    def select_all(self) -> None:
        send_message(self.box, EM_SETSEL, 0, -1)

    def has_focus(self) -> bool:
        return user32.GetForegroundWindow() == self.form and focused_control(self.form) == self.box

    def focus(self) -> None:
        """Brings the window to the front with its text box focused, by
        SetForegroundWindow or else a click into the box (never a lone Alt)."""
        if self.has_focus():
            return
        user32.SetForegroundWindow(self.form)
        try:
            wait_until("", self.has_focus, timeout=1)
            return
        except CheckFailed:
            pass
        rect = wintypes.RECT()
        user32.GetWindowRect(self.box, ctypes.byref(rect))
        click((rect.left + rect.right) // 2, (rect.top + rect.bottom) // 2)
        wait_until(f"target {self.name} to take focus", self.has_focus, timeout=5)

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.kill()


class FakeLocalAi:
    """A stand-in OpenAI-compatible server for Transform Selection: it
    upper-cases the text after the prompt's `---\\nText:\\n` separator."""

    def __init__(self) -> None:
        self.requests: list[dict] = []
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def _reply(self, body: dict) -> None:
                data = json.dumps(body).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                self._reply({"data": [{"id": "e2e-upper"}]})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
                outer.requests.append(body)
                user = next(m["content"] for m in body["messages"] if m["role"] == "user")
                instruction = user.split("\n\n---\nText:\n", 1)[0]
                selection = user.split("---\nText:\n", 1)[1]
                outer.last = {"instruction": instruction, "selection": selection}
                self._reply(
                    {
                        "model": body.get("model", "e2e-upper"),
                        "choices": [{"message": {"role": "assistant", "content": selection.upper()}}],
                    }
                )

        self.last: dict | None = None
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.endpoint = f"http://127.0.0.1:{self.server.server_address[1]}/v1"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()


def webview2_command_lines() -> str:
    """The running WebView2 browser processes' command lines, for a failure."""
    try:
        return subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='msedgewebview2.exe'\" | "
                "Where-Object { $_.CommandLine -notmatch '--type=' } | ForEach-Object { $_.CommandLine }",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError) as error:
        return f"(could not list them: {error})"


class AppCommands:
    """Calls the app's own commands (the ones the Hotkeys page calls) inside
    its main WebView2 window, through WebView2's remote debugging port and the
    Chrome DevTools Protocol. The port is opened by the test build's config
    (webview_debug_config.py). A minimal WebSocket client, standard library
    only."""

    def __init__(self, port: int) -> None:
        self.port = port
        try:
            pages = wait_until(f"the dashboard's WebView2 debugging port {port}", self._pages, 30, 0.25)
        except CheckFailed as error:
            raise CheckFailed(f"{error}. WebView2 browser processes: {webview2_command_lines()}") from None
        errors = []
        for page in pages:
            try:
                self._open(page["webSocketDebuggerUrl"])
                label = self.evaluate("window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label ?? null")
                if label == "main":
                    print(f"App commands through WebView2 page {page.get('url')}")
                    return
                errors.append(f"{page.get('url')}: window {label!r}")
            except (OSError, CheckFailed) as error:
                errors.append(f"{page.get('url')}: {error}")
            self.close()
        raise CheckFailed(f"No WebView2 page is Scribe's main window: {errors}")

    def _pages(self) -> list[dict] | None:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/json/list", timeout=2) as response:
                pages = json.loads(response.read())
        except (OSError, ValueError):
            return None
        return [page for page in pages if page.get("type") == "page" and page.get("webSocketDebuggerUrl")] or None

    def _open(self, url: str) -> None:
        parts = urllib.parse.urlparse(url)
        self.sock = socket.create_connection((parts.hostname, parts.port), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            (
                f"GET {parts.path} HTTP/1.1\r\nHost: {parts.hostname}:{parts.port}\r\n"
                f"Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                "Sec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        reply = b""
        while b"\r\n\r\n" not in reply:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise OSError("The debugging port closed the WebSocket handshake")
            reply += chunk
        head, self.buffer = reply.split(b"\r\n\r\n", 1)
        if b" 101 " not in head.split(b"\r\n", 1)[0]:
            raise OSError(f"WebSocket handshake refused: {head[:200]!r}")
        self.next_id = 0

    def close(self) -> None:
        sock = getattr(self, "sock", None)
        if sock:
            sock.close()
            self.sock = None

    def _send(self, text: str) -> None:
        data = text.encode()
        mask = os.urandom(4)
        if len(data) < 126:
            head = bytes([0x81, 0x80 | len(data)])
        elif len(data) < 65536:
            head = bytes([0x81, 0x80 | 126]) + struct.pack(">H", len(data))
        else:
            head = bytes([0x81, 0x80 | 127]) + struct.pack(">Q", len(data))
        self.sock.sendall(head + mask + bytes(byte ^ mask[i % 4] for i, byte in enumerate(data)))

    def _read(self, size: int) -> bytes:
        while len(self.buffer) < size:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise OSError("The WebView2 debugging connection closed")
            self.buffer += chunk
        data, self.buffer = self.buffer[:size], self.buffer[size:]
        return data

    def _message(self) -> str:
        parts = []
        while True:
            first, second = self._read(2)
            size = second & 0x7F
            if size == 126:
                size = struct.unpack(">H", self._read(2))[0]
            elif size == 127:
                size = struct.unpack(">Q", self._read(8))[0]
            payload = self._read(size)
            opcode = first & 0x0F
            if opcode == 0x8:
                raise OSError("The WebView2 debugging connection was closed")
            if opcode == 0x9:  # ping: answer with a masked pong
                mask = os.urandom(4)
                self.sock.sendall(bytes([0x8A, 0x80 | len(payload)]) + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))
                continue
            if opcode in (0x0, 0x1):
                parts.append(payload)
                if first & 0x80:
                    return b"".join(parts).decode()

    def evaluate(self, expression: str, timeout: float = 60):
        self.next_id += 1
        call_id = self.next_id
        self._send(
            json.dumps(
                {
                    "id": call_id,
                    "method": "Runtime.evaluate",
                    "params": {"expression": expression, "awaitPromise": True, "returnByValue": True},
                }
            )
        )
        self.sock.settimeout(timeout)
        while True:
            message = json.loads(self._message())
            if message.get("id") != call_id:
                continue
            if "error" in message:
                raise CheckFailed(f"DevTools refused the call: {message['error']}")
            result = message["result"]
            if "exceptionDetails" in result:
                raise CheckFailed(f"The page threw: {result['exceptionDetails']}")
            return result["result"].get("value")

    def invoke(self, command: str, args: dict | None = None):
        """Runs a Tauri command as the dashboard does; a command error fails the check."""
        expression = (
            "(async () => { try { return { ok: true, value: await window.__TAURI_INTERNALS__.invoke("
            f"{json.dumps(command)}, {json.dumps(args or {})}) }}; }} catch (error) {{ return {{ ok: false, "
            "error: typeof error === 'string' ? error : JSON.stringify(error) }; } })()"
        )
        reply = self.evaluate(expression)
        if not reply or not reply.get("ok"):
            raise CheckFailed(f"The app's {command} command failed: {(reply or {}).get('error')}")
        return reply.get("value")


class StateStream:
    """Every dictation state Scribe publishes, in order, from its state server
    (`/v1/events`, docs/integrations/dictation-state-contract.md). Unlike
    polling status.json it cannot miss a short-lived state, so "nothing
    started" is checked against the full sequence."""

    def __init__(self, pid: int) -> None:
        self.events: list[tuple[float, str]] = []
        self.error: str | None = None
        discovery = Path(os.environ.get("USERPROFILE", "")) / ".scribe" / "control.dev.json"

        def current():
            try:
                data = json.loads(discovery.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                return None
            return data if data.get("pid") == pid else None

        info = wait_until(f"the state server's discovery file {discovery}", current, 30, 0.2)
        parts = urllib.parse.urlparse(info["baseUrl"])
        self.sock = socket.create_connection((parts.hostname, parts.port), timeout=10)
        self.sock.sendall(
            (
                f"GET {info['endpoints']['events']} HTTP/1.1\r\nHost: {parts.netloc}\r\n"
                f"Authorization: Bearer {info['readToken']}\r\nAccept: text/event-stream\r\n\r\n"
            ).encode()
        )
        self.sock.settimeout(None)
        threading.Thread(target=self._read, daemon=True).start()
        wait_until("the state stream's first snapshot", lambda: self.events or self.error, 10)
        check(not self.error, f"The state stream failed: {self.error}")

    def _read(self) -> None:
        buffer = b""
        event = None
        try:
            while True:
                chunk = self.sock.recv(4096)
                if not chunk:
                    self.error = "the state server closed the stream"
                    return
                buffer += chunk
                while b"\n" in buffer:
                    raw, buffer = buffer.split(b"\n", 1)
                    line = raw.decode("utf-8", "replace").rstrip("\r")
                    if line.startswith("HTTP/") and " 200 " not in line + " ":
                        self.error = line
                        return
                    if line.startswith("event:"):
                        event = line[6:].strip()
                    elif line.startswith("data:") and event == "state":
                        self.events.append((time.monotonic(), json.loads(line[5:])["status"]))
                    elif not line:
                        event = None
        except OSError as error:
            if self.sock is not None:
                self.error = str(error)

    def mark(self) -> int:
        return len(self.events)

    def since(self, mark: int) -> list[str]:
        return [state for _, state in self.events[mark:]]

    def close(self) -> None:
        sock, self.sock = self.sock, None
        if sock:
            sock.close()


# --------------------------------------------------------------------------
# The app
# --------------------------------------------------------------------------


class Scribe:
    def __init__(self, exe: Path, clip_path: Path, out: Path, debug_port: int) -> None:
        self.exe = exe
        self.clip_path = clip_path
        self.out = out
        self.process: subprocess.Popen | None = None
        self.monitor: SpeechProcessMonitor | None = None
        self.debug_port = debug_port

    def self_test(self, clip: Path) -> dict:
        """The release smoke test's mode on a fresh profile: downloads the
        default model through the verified path and transcribes a clip."""
        result_path = self.out / "self-test.json"
        process = subprocess.run([str(self.exe), "--self-test-transcribe", str(clip), str(result_path)], timeout=900)
        check(result_path.exists(), f"The self-test exited with {process.returncode} and wrote no result")
        result = json.loads(result_path.read_text(encoding="utf-8"))
        print(f"Self-test: model {result.get('modelId')}, {result.get('latencyMs')} ms: {result.get('text')}")
        check(result.get("ok") and process.returncode == 0, f"The self-test failed: {result.get('error')}")
        check_words(result["text"], JFK_WORDS, "The self-test transcript")
        return result

    def launch(self) -> None:
        env = dict(os.environ, SCRIBE_E2E_AUDIO_WAV=str(self.clip_path))
        self.process = subprocess.Popen([str(self.exe)], env=env)
        pid = self.process.pid
        if self.monitor is not None:
            self.monitor.parent = pid
        wait_until("Scribe to start", lambda: (read_status() or {}).get("pid") == pid, 120)
        wait_until("Scribe to finish setup", lambda: TEST_AUDIO_ON_LOG in log_text(), 60)

    def main_window(self) -> int | None:
        for hwnd in top_windows(self.process.pid):
            if window_title(hwnd) == "Scribe Dev":
                return hwnd
        return None

    def alive(self) -> bool:
        return self.process is not None and self.process.poll() is None

    def stop(self) -> None:
        if self.alive():
            self.process.kill()
            self.process.wait(30)


def status() -> str | None:
    return (read_status() or {}).get("status")


# Processes that hold a speech model: the resident engines and whisper-cli.
SPEECH_ENGINES = ("whisper-server.exe", "scribe-parakeet.exe", "whisper-cli.exe")


class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD),
        ("th32DefaultHeapID", ULONG_PTR),
        ("th32ModuleID", wintypes.DWORD),
        ("cntThreads", wintypes.DWORD),
        ("th32ParentProcessID", wintypes.DWORD),
        ("pcPriClassBase", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
        ("szExeFile", wintypes.WCHAR * 260),
    ]


def speech_processes() -> list[dict]:
    """Every running speech engine process on the machine, with its parent
    (a Toolhelp snapshot: fast enough to poll)."""
    snapshot = kernel32.CreateToolhelp32Snapshot(0x2, 0)  # TH32CS_SNAPPROCESS
    if snapshot in (None, 0, -1, ctypes.c_void_p(-1).value):
        return []
    found = []
    try:
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(PROCESSENTRY32W)
        ok = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while ok:
            if entry.szExeFile.lower() in SPEECH_ENGINES:
                found.append(
                    {
                        "ProcessId": entry.th32ProcessID,
                        "ParentProcessId": entry.th32ParentProcessID,
                        "Name": entry.szExeFile,
                    }
                )
            ok = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return found


class SpeechProcessMonitor:
    """Samples the speech engine processes Scribe owns for the whole run and
    remembers the most that were ever alive at once: two would mean two
    models in memory together."""

    def __init__(self) -> None:
        self.parent: int | None = None
        self.peak: list[dict] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._sample, daemon=True)
        self._thread.start()

    def _sample(self) -> None:
        while not self._stop.is_set():
            if self.parent is not None:
                owned = [p for p in speech_processes() if p["ParentProcessId"] == self.parent]
                if len(owned) > len(self.peak):
                    self.peak = owned
            self._stop.wait(0.1)

    def stop(self) -> None:
        self._stop.set()
        self._thread.join(5)


def wait_idle(timeout: float = 60) -> None:
    wait_until("Scribe to be idle", lambda: status() in ("Idle", "Ready"), timeout)


def wait_recording() -> None:
    wait_until("the recording to start", lambda: status() == "Recording", 15)


def clips_finished() -> int:
    return log_text().count(CLIP_FINISHED_LOG)


def wait_clip_played(before: int) -> None:
    """Waits until the test input has played the whole clip into the recording."""
    wait_until("the clip to finish playing", lambda: clips_finished() > before, 60, 0.1)


def wait_new_transcript(previous: dict | None) -> dict:
    """The transcript is saved after its output step, so a new one in the
    database means the dictation is done."""
    previous_id = (previous or {}).get("id")

    def saved():
        current = last_transcript()
        return current if current and current["id"] != previous_id and current["text"] is not None else None

    transcript = wait_until("the transcript to be saved", saved, 120, 0.1)
    wait_idle()
    return transcript


# --------------------------------------------------------------------------
# Scenarios
# --------------------------------------------------------------------------


class Run:
    def __init__(self, args) -> None:
        self.out = Path(args.out)
        self.out.mkdir(parents=True, exist_ok=True)
        self.work = self.out / "work"
        self.work.mkdir(exist_ok=True)
        self.fixtures = Path(args.fixtures)
        self.clip_path = self.work / "current.wav"
        self.app = Scribe(Path(args.exe), self.clip_path, self.out, args.webview_debug_port)
        self.clipboard = Clipboard()
        self.ai = FakeLocalAi()
        self.results: list[tuple[str, bool, str, float]] = []
        self.targets: dict[str, Target] = {}
        self.clips: dict[str, Path] = {"jfk": self.fixtures / "jfk.wav"}
        self.monitor = SpeechProcessMonitor()
        self.app.monitor = self.monitor
        # The default-style hotkey group.
        self.keys = Keyboard()
        self.commands: AppCommands | None = None
        self.states: StateStream | None = None
        self.hotkey_status: dict = {}
        self.bindings: dict[str, dict] = {}
        self.test_hotkeys: dict | None = None
        self.state_mark = 0

    # -- setup -------------------------------------------------------------

    def make_clips(self) -> None:
        manifest = []
        for name, text in CLIPS.items():
            path = self.work / f"{name}.wav"
            self.clips[name] = path
            manifest.append({"text": text, "path": str(path)})
        manifest_path = self.work / "clips.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(HERE / "speak.ps1"),
                "-Manifest",
                str(manifest_path),
            ],
            check=True,
            timeout=300,
        )

    def use_clip(self, name: str) -> None:
        shutil.copyfile(self.clips[name], self.clip_path)

    def shot(self, name: str) -> None:
        try:
            screenshot(self.out / "screenshots" / f"{name}.png")
        except Exception as error:  # a screenshot must never fail a check
            print(f"Screenshot {name} failed: {error}")

    def setup(self) -> None:
        self.make_clips()
        self.use_clip("jfk")
        self.app.self_test(self.clips["jfk"])
        patch_settings(
            {
                "hotkeys": {action: chord for action, (chord, _) in HOTKEYS.items()},
                "autoUpdateCheckEnabled": False,
                "autoInstallUpdates": False,
                "launchAtStartup": False,
                "notificationsEnabled": False,
                "soundsEnabled": False,
                "minimizeToTray": True,
                "showFloatingPill": True,
                "silenceAutoStopEnabled": False,
                "outputMode": "auto_paste",
                "pasteMethod": "clipboard_paste",
                "notesAnalysisEnabled": True,
                "notesAnalysisEndpoint": self.ai.endpoint,
                "notesAnalysisModel": "e2e-upper",
            }
        )
        self.app.launch()
        failures = re.findall(r"Hotkey registration failed.*", log_text())
        check(not failures, f"Test hotkeys failed to register: {failures}")
        self.dashboard = wait_until("the dashboard window", self.app.main_window, 30)
        wait_until("the dashboard to show", lambda: user32.IsWindowVisible(self.dashboard), 30)
        self.shot("dashboard")
        self.targets["A"] = Target("A", self.work, 20)
        self.targets["B"] = Target("B", self.work, 520)
        self.targets["Slow"] = Target("Slow", self.work, 20, top=300, paste_delay_ms=SLOW_APP_PASTE_DELAY_MS)
        self.targets["Deaf"] = Target("Deaf", self.work, 520, top=300, paste_delay_ms=-1)

    # -- building blocks ---------------------------------------------------

    def prepare(self, target: Target, sentinel: str, text: str = "") -> int:
        """Focuses `target` with `text` in its box and `sentinel` on the
        clipboard; returns the clipboard sequence number afterwards."""
        if status() in BUSY:
            press("discardDictation")
        wait_idle()
        target.focus()
        target.set_text(text)
        self.clipboard.write(sentinel)
        check(self.clipboard.read() == sentinel, "Could not set the clipboard")
        return Clipboard.sequence()

    def toggle_dictation(self, clip: str, while_recording=None) -> dict:
        self.use_clip(clip)
        previous, played = last_transcript(), clips_finished()
        press("toggleDictation")
        wait_recording()
        if while_recording:
            while_recording()
        wait_clip_played(played)
        press("toggleDictation")
        return wait_new_transcript(previous)

    # -- scenarios ---------------------------------------------------------

    def close_to_tray(self) -> None:
        window = self.dashboard
        user32.PostMessageW(window, WM_CLOSE, 0, 0)
        wait_until("the dashboard to hide", lambda: not user32.IsWindowVisible(window), 10)
        stays("Scribe to keep running in the tray", self.app.alive, 2)
        check(user32.IsWindow(window), "Closing the dashboard destroyed it instead of hiding it")

    def toggle_paste_restores_clipboard(self) -> None:
        target = self.targets["A"]
        sentinel = "clipboard before toggle"
        self.prepare(target, sentinel)
        transcript = self.toggle_dictation("jfk", lambda: self.shot("pill-recording"))
        self.shot("after-toggle")
        check_words(transcript["text"], JFK_WORDS, "The transcript")
        wait_until("the transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 5)
        wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 5)
        check(transcript["output_mode"] == "auto_paste", f"Saved with output mode {transcript['output_mode']}")

    def hold_to_talk(self) -> None:
        target = self.targets["A"]
        sentinel = "clipboard before hold"
        self.prepare(target, sentinel)
        self.use_clip("hold")
        previous, played = last_transcript(), clips_finished()
        chord_down("holdToTalk")
        try:
            wait_recording()
            wait_clip_played(played)
            check(status() == "Recording", "Recording stopped while the keys were still held")
        finally:
            chord_up("holdToTalk")
        transcript = wait_new_transcript(previous)
        check_words(transcript["text"], key_words(CLIPS["hold"]), "The transcript")
        wait_until("the transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 5)
        wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 5)

    def slow_app_paste(self) -> None:
        """Issue #105: an app that reads the clipboard late must still get the
        dictation, never the clipboard Scribe puts back afterwards."""
        target = self.targets["Slow"]
        sentinel = "clipboard before the slow app"
        self.prepare(target, sentinel)
        transcript = self.toggle_dictation("slow_app")
        check_words(transcript["text"], key_words(CLIPS["slow_app"]), "The transcript")
        pasted = wait_until("the slow app to paste something", lambda: target.text.strip(), 15)
        check(pasted != sentinel, "The slow app pasted the old clipboard instead of the dictation")
        check(pasted == transcript["text"].strip(), f"The slow app pasted {pasted!r}")
        wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 10)
        check("Paste clipboard: Read" in log_text(), "Scribe did not see the slow app read the clipboard")

    def ignored_paste(self) -> None:
        """An app that never reads the clipboard on Ctrl+V must still get the
        user's clipboard back once Scribe stops waiting for the read."""
        target = self.targets["Deaf"]
        sentinel = "clipboard before an ignored paste"
        self.prepare(target, sentinel)
        transcript = self.toggle_dictation("ignored_paste")
        check_words(transcript["text"], key_words(CLIPS["ignored_paste"]), "The transcript")
        # The dictation is saved after its paste and restore, so both are done.
        check(self.clipboard.read() == sentinel, "The clipboard was not restored after the ignored paste")
        check("restoring the clipboard anyway" in log_text(), "Scribe did not wait for the ignored paste to time out")
        stays("nothing pasted", lambda: target.text == "")

    def save_only(self) -> None:
        patch_settings({"outputMode": "save_only"})
        target = self.targets["A"]
        sentinel = "clipboard before save only"
        sequence = self.prepare(target, sentinel)
        count = transcript_count()
        transcript = self.toggle_dictation("save_only")
        check_words(transcript["text"], key_words(CLIPS["save_only"]), "The transcript")
        check(transcript["output_mode"] == "save_only", f"Saved with output mode {transcript['output_mode']}")
        check(transcript_count() == count + 1, "The dictation was not added to History")
        stays("nothing pasted", lambda: target.text == "")
        check(self.clipboard.read() == sentinel, "Save Only changed the clipboard")
        check(Clipboard.sequence() == sequence, "Save Only touched the clipboard")

    def copy_clipboard(self) -> None:
        patch_settings({"outputMode": "copy_clipboard"})
        target = self.targets["A"]
        self.prepare(target, "clipboard before copy")
        transcript = self.toggle_dictation("copy_clipboard")
        check_words(transcript["text"], key_words(CLIPS["copy_clipboard"]), "The transcript")
        wait_until("the transcript on the clipboard", lambda: self.clipboard.read() == transcript["text"], 5)
        stays("nothing pasted", lambda: target.text == "")

    def copy_and_paste(self) -> None:
        patch_settings({"outputMode": "copy_and_paste"})
        target = self.targets["A"]
        self.prepare(target, "clipboard before copy and paste")
        transcript = self.toggle_dictation("copy_and_paste")
        check_words(transcript["text"], key_words(CLIPS["copy_and_paste"]), "The transcript")
        wait_until("the transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 5)
        stays("the transcript to stay on the clipboard", lambda: self.clipboard.read() == transcript["text"])

    def direct_insert(self) -> None:
        patch_settings({"outputMode": "auto_paste", "pasteMethod": "direct_insert"})
        target = self.targets["A"]
        sentinel = "clipboard before typing"
        sequence = self.prepare(target, sentinel)
        transcript = self.toggle_dictation("direct_insert")
        check_words(transcript["text"], key_words(CLIPS["direct_insert"]), "The transcript")
        check(transcript["paste_method"] == "direct_insert", f"Saved with paste method {transcript['paste_method']}")
        wait_until("the typed transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 10)
        check(self.clipboard.read() == sentinel, "Typing the transcript changed the clipboard")
        check(Clipboard.sequence() == sequence, "Typing the transcript touched the clipboard")
        patch_settings({"pasteMethod": "clipboard_paste"})

    def switch_apps(self) -> None:
        start, other = self.targets["A"], self.targets["B"]
        other.focus()
        other.set_text("")
        self.prepare(start, "clipboard before switching")
        transcript = self.toggle_dictation("switch", other.focus)
        check_words(transcript["text"], key_words(CLIPS["switch"]), "The transcript")
        wait_until("the transcript on the clipboard", lambda: self.clipboard.read() == transcript["text"], 5)
        stays(
            "nothing pasted into either app",
            lambda: start.text == "" and other.text == "",
        )
        self.last_switch = transcript

    def paste_last(self) -> None:
        target = self.targets["B"]
        sentinel = "clipboard before paste last"
        self.prepare(target, sentinel)
        expected = last_transcript()
        check(expected and expected["text"], "There is no last transcript to paste")
        press("pasteLastTranscript")
        wait_until("the last transcript in the focused box", lambda: target.text.strip() == expected["text"].strip(), 15)
        wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 5)

    def discard(self) -> None:
        target = self.targets["A"]
        sentinel = "clipboard before discard"
        self.prepare(target, sentinel)
        self.use_clip("discard")
        previous, count = last_transcript(), transcript_count()
        press("toggleDictation")
        wait_recording()
        press("discardDictation")
        wait_until("the recording to be discarded", lambda: status() == "Idle", 10)
        stays(
            "nothing pasted, saved or copied",
            lambda: target.text == ""
            and transcript_count() == count
            and (last_transcript() or {}).get("id") == (previous or {}).get("id")
            and self.clipboard.read() == sentinel,
        )

    def transform_selection(self) -> None:
        target = self.targets["A"]
        sentinel = "clipboard before transform"
        self.prepare(target, sentinel, TRANSFORM_SOURCE)
        target.select_all()
        self.use_clip("instruction")
        self.ai.last = None
        played = clips_finished()
        press("transformSelection")
        wait_recording()
        wait_clip_played(played)
        press("transformSelection")
        expected = TRANSFORM_SOURCE.upper()
        wait_until("the transformed text in place of the selection", lambda: target.text == expected, 120)
        wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 10)
        request = self.ai.last
        check(request is not None, "The local AI server got no request")
        check(request["selection"] == TRANSFORM_SOURCE, f"The selection sent was {request['selection']!r}")
        check_words(request["instruction"], ["make", "text", "uppercase"], "The spoken instruction")
        wait_idle()

    def dashboard_shot(self, name: str) -> None:
        """Brings the dashboard to the front and saves a screenshot of it, for
        the model status line. A screenshot never fails a check."""
        targets = [target.form for target in self.targets.values()]
        try:
            for form in targets:
                user32.ShowWindow(form, 6)  # SW_MINIMIZE: the stand-in apps sit on top
            hwnd = wait_until("the dashboard window", self.app.main_window, 30)
            user32.ShowWindow(hwnd, 9)  # SW_RESTORE
            user32.SetWindowPos(hwnd, -1, 0, 0, 0, 0, 0x0003)  # topmost, keep size and place
            time.sleep(1.5)
            self.shot(name)
            user32.SetWindowPos(hwnd, -2, 0, 0, 0, 0, 0x0003)  # not topmost
        except Exception as error:
            print(f"Dashboard screenshot {name} failed: {error}")
        finally:
            for form in targets:
                user32.ShowWindow(form, 9)  # SW_RESTORE

    def owned_engines(self) -> list[dict]:
        pid = self.app.process.pid
        return [p for p in speech_processes() if p["ParentProcessId"] == pid and p["Name"] != "whisper-cli.exe"]

    def model_loads_at_startup(self) -> None:
        """Keep the speech model loaded is on by default: the model is loaded
        right after launch, before anyone dictates."""
        engines = wait_until("the speech model to load at startup", self.owned_engines, 90, 0.2)
        print(f"Loaded at startup: {engines}")

    def never_two_models(self) -> None:
        peak = self.monitor.peak
        print(f"Most speech engine processes alive at once: {len(peak)} {peak}")
        check(len(peak) <= 1, f"Two speech models were in memory at once: {peak}")

    def keep_loaded_off(self) -> None:
        """With the switch off, nothing is loaded until a dictation starts,
        the dictation still works, and the model is unloaded a minute later."""
        self.app.stop()
        patch_settings({"keepWhisperLoaded": False})
        self.app.launch()
        stays("no model loaded after launch", lambda: not self.owned_engines(), 5)
        self.dashboard_shot("keep-loaded-off-home")
        target = self.targets["A"]
        self.prepare(target, "clipboard before keep-loaded off")
        transcript = self.toggle_dictation("jfk")
        check_words(transcript["text"], JFK_WORDS, "The transcript with the model loaded on demand")
        finished = time.monotonic()
        # A quick follow-up dictation stays instant: the model waits a minute.
        stays("the model loaded for most of the idle minute", lambda: bool(self.owned_engines()), 45)
        wait_until("the model to unload after a minute idle", lambda: not self.owned_engines(), 60, 1)
        print(f"Unloaded {time.monotonic() - finished:.0f} s after the dictation finished")
        # Back to the default for the scenarios after this one.
        self.app.stop()
        patch_settings({"keepWhisperLoaded": True})
        self.app.launch()
        wait_until("the model to load at startup again", self.owned_engines, 90, 0.2)

    def parakeet_engine(self) -> None:
        """Switching to Parakeet v3: Whisper's server is gone, the Parakeet
        sidecar loads at startup, and a dictation transcribes through it.
        Parakeet stays loaded for the hard-kill scenario after this one."""
        self.app.stop()
        patch_settings({"selectedModelId": PARAKEET_ID})
        self.app.launch()
        engines = wait_until(
            "the Parakeet sidecar to load at startup",
            lambda: [p for p in self.owned_engines() if p["Name"].lower() == "scribe-parakeet.exe"],
            120,
            0.2,
        )
        names = sorted(p["Name"].lower() for p in self.owned_engines())
        check(names == ["scribe-parakeet.exe"], f"Expected only the Parakeet sidecar, found {names}")
        print(f"Parakeet loaded: {engines}")
        self.dashboard_shot("parakeet-loaded-home")
        target = self.targets["A"]
        self.prepare(target, "clipboard before Parakeet")
        transcript = self.toggle_dictation("jfk")
        check_words(transcript["text"], JFK_WORDS, "The Parakeet transcript")
        print(f"Parakeet transcript: {transcript['text']}")
        names = sorted(p["Name"].lower() for p in self.owned_engines())
        check(names == ["scribe-parakeet.exe"], f"After the dictation expected only Parakeet, found {names}")

    def hard_kill_stops_speech_engine(self) -> None:
        """Ends Scribe the way a crash or Task Manager does (TerminateProcess:
        no exit handler runs) and checks Windows takes the loaded speech
        model down with it."""
        pid = self.app.process.pid
        engines = wait_until(
            "a speech engine process owned by Scribe",
            lambda: [p for p in speech_processes() if p["ParentProcessId"] == pid],
            60,
            0.5,
        )
        print(f"Speech engine before the kill: {engines}")
        self.app.process.kill()
        self.app.process.wait(30)
        killed_at = time.monotonic()
        engine_pids = {p["ProcessId"] for p in engines}
        wait_until(
            "the speech engine to exit with Scribe",
            lambda: not engine_pids & {p["ProcessId"] for p in speech_processes()},
            5,
            0.2,
        )
        print(f"Speech engine gone {time.monotonic() - killed_at:.1f} s after the kill")

    # -- default-style hotkeys: the Windows watcher paths ------------------
    #
    # The scenarios above bind every action to Ctrl+Alt+F7..F12, which the
    # global-shortcut plugin handles. The bindings Windows users get by default
    # go through three other mechanisms (crates/scribe-core/src/hotkeys.rs):
    # the chord watcher for a modifier-only hold to talk, the toggle-key
    # watcher for a bare toggle key, and the note chord it arms. This group
    # loads the build's own default bindings through the app's commands and
    # drives those paths with scan-code input, then puts the test bindings back.

    def hotkeys_connect(self) -> None:
        """Reaches the app's hotkey commands (as the Hotkeys page calls them)
        and its state stream, and keeps the test bindings to put back later."""
        self.test_hotkeys = json.loads(setting_value("app_settings"))["hotkeys"]
        self.commands = AppCommands(self.app.debug_port)
        self.states = StateStream(self.app.process.pid)
        self.use_bindings("get_hotkey_status")
        for action, (chord, _) in HOTKEYS.items():
            check(
                self.bindings[action]["shortcut"] == chord,
                f"get_hotkey_status reports {action} as {self.bindings[action]['shortcut']}, the runner set {chord}",
            )
        print(f"Hotkey backend: {self.hotkey_status.get('backend')}; state stream: {self.states.since(0)}")

    def use_bindings(self, command: str, args: dict | None = None) -> dict:
        """Runs a hotkey command that answers with the hotkey status, and
        checks every binding it reports is registered."""
        status = self.commands.invoke(command, args)
        self.hotkey_status = status
        self.bindings = {binding["action"]: binding for binding in status["bindings"]}
        summary = ", ".join(
            f"{b['action']}={b['shortcut']}" + (f" on {b['trigger']}" if b.get("trigger") else "")
            for b in status["bindings"]
        )
        print(f"{command}: {summary}; note chord active: {status.get('noteChordActive')}")
        missing = [
            f"{b['action']} {b['shortcut']}: {b.get('error')}"
            for b in status["bindings"]
            if not b["registered"] and not b.get("disabledReason")
        ]
        check(not missing, f"After {command} these hotkeys are not registered: {missing}")
        return status

    def chord(self, action: str) -> Chord:
        return Chord(self.bindings[action]["shortcut"])

    def ready(self, target: Target, sentinel: str, text: str = "") -> None:
        """prepare() for bindings that change: waits for idle instead of
        pressing the discard hotkey, and marks the state stream."""
        wait_idle()
        target.focus()
        target.set_text(text)
        self.clipboard.write(sentinel)
        check(self.clipboard.read() == sentinel, "Could not set the clipboard")
        self.state_mark = self.states.mark()

    def states_seen(self) -> list[str]:
        return self.states.since(self.state_mark) if self.states else []

    @staticmethod
    def watcher_starts() -> int:
        return log_text().count(TOGGLE_WATCHER_PREFIX)

    def wait_toggle_watcher(self, before: int, toggle: Chord, trigger: str) -> None:
        """Waits for the toggle-key watcher to (re)start after a hotkey command:
        a new start line after the `before` count, the newest naming `toggle`
        and `trigger`. A line left from an earlier start never counts."""
        line = TOGGLE_WATCHER_LOG.format(vk=toggle.key, trigger=trigger)
        wait_until(f"a new toggle-key watcher start ({line!r})", lambda: self.watcher_starts() > before, 10, 0.1)
        newest = log_lines(TOGGLE_WATCHER_PREFIX)[-1]
        check(line in newest, f"The toggle-key watcher restarted as {newest!r}, expected {line!r}")

    def wait_q_grabbed(self) -> None:
        """Waits until Scribe holds Q, which the toggle-key watcher grabs for
        the note chord as soon as it sees the toggle key go down (Release
        trigger). This is how the runner knows the 10 ms watcher saw the press.

        The probe registers Q itself for a few microseconds when Q is free,
        and Scribe's own registration would fail if it landed in that window
        ("Note chord unavailable"). So the first probe waits until Scribe has
        normally long finished (a few milliseconds after the key goes down),
        and later ones are 100 ms apart: Scribe is then almost always already
        holding Q, the probe fails to register and holds nothing. A collision
        is still reported as one, not as a product failure."""
        unavailable = log_text().count(NOTE_CHORD_UNAVAILABLE_LOG)
        free_probes = 0

        def grabbed() -> bool:
            nonlocal free_probes
            if hotkey_grabbed(Q_KEY):
                return True
            free_probes += 1
            return False

        time.sleep(Q_PROBE_FIRST_DELAY)
        try:
            wait_until("Scribe to grab Q while the toggle key is down", grabbed, 5, Q_PROBE_INTERVAL)
        except CheckFailed as error:
            if log_text().count(NOTE_CHORD_UNAVAILABLE_LOG) > unavailable:
                cause = (
                    f" The runner's own probe found Q free {free_probes} times and may have held it at the moment"
                    " Scribe registered it."
                    if free_probes
                    else ""
                )
                raise CheckFailed(f"{error}. Scribe logged: {log_lines(NOTE_CHORD_UNAVAILABLE_LOG)[-1]}.{cause}") from None
            raise CheckFailed(f"{error}. The toggle-key watcher never armed the note chord") from None

    def tap_watched_toggle(self, toggle: Chord, starts: bool = False) -> None:
        """Taps a bare toggle key bound on Release, as a person does: down,
        held until the watcher has seen it, up (the release toggles). With
        `starts`, also checks that nothing starts while the key is still down:
        on Release the toggle must wait for the key to come up."""
        released = log_text().count("Toggle key released")
        mark = self.states.mark()
        self.keys.down(toggle.key)
        try:
            self.wait_q_grabbed()
            if starts:
                stays(
                    "no recording while the toggle key is still down (Release trigger)",
                    lambda: "Recording" not in self.states.since(mark) and status() != "Recording",
                    RELEASE_HOLD_OBSERVATION,
                )
        finally:
            self.keys.up(toggle.key)
        wait_until(
            "the toggle-key watcher to see the key released",
            lambda: log_text().count("Toggle key released") > released,
            5,
            0.05,
        )

    def press_note_chord(self, toggle: Chord) -> None:
        """Holds the toggle key, taps Q, lets go of the toggle key."""
        fired = log_text().count(NOTE_CHORD_LOG)
        skipped = log_text().count(TOGGLE_RELEASED_LOG.format(fired="true"))
        self.keys.down(toggle.key)
        try:
            self.wait_q_grabbed()
            self.keys.tap(VK_Q)
            wait_until("Scribe to see Q during the toggle hold", lambda: log_text().count(NOTE_CHORD_LOG) > fired, 5, 0.05)
        finally:
            self.keys.release_all()
        wait_until(
            "the toggle key's release to skip the toggle after the note chord",
            lambda: log_text().count(TOGGLE_RELEASED_LOG.format(fired="true")) > skipped,
            5,
            0.05,
        )

    def wait_q_free(self) -> None:
        wait_until("Scribe to let go of Q (it is grabbed only while the toggle key is down)", lambda: not hotkey_grabbed(Q_KEY), 5, 0.05)

    def nothing_started(self, describe: str, count: int, seconds: float = 1.0) -> None:
        try:
            stays(
                f"{describe} to start nothing",
                lambda: "Recording" not in self.states_seen() and transcript_count() == count,
                seconds,
            )
        except CheckFailed as error:
            raise CheckFailed(f"{error}; states seen: {self.states_seen()}") from None

    def short_chord_tap(self, chord: Chord) -> None:
        """A tap of a modifier-only chord shorter than the chord watcher's
        150 ms arming delay starts nothing. The keys go down and up as one
        batch each, and the tap is measured: one the runner itself stretched
        past the delay is tried again instead of being judged."""
        for attempt in range(1, 4):
            # Read per attempt: a stretched tap may have dictated something.
            count = transcript_count()
            self.state_mark = self.states.mark()
            self.keys.send(chord.modifiers, False)
            pressed = time.monotonic()
            time.sleep(CHORD_ARMING_DELAY / 3)
            self.keys.send(chord.modifiers[::-1], True)
            held = time.monotonic() - pressed
            if held < CHORD_ARMING_DELAY * 0.7:
                break
            print(f"Tap {attempt} of {chord} took {held * 1000:.0f} ms on this runner; trying again")
            wait_idle()
        else:
            raise CheckFailed(f"The runner could not tap {chord} in under {CHORD_ARMING_DELAY * 700:.0f} ms")
        self.nothing_started(f"a {held * 1000:.0f} ms tap of {chord}", count)
        print(f"A {held * 1000:.0f} ms tap of {chord} started nothing")

    def chord_hold_dictation(self, hold: Chord, clip: str) -> None:
        """Hold to talk on a modifier-only chord (the chord watcher): a short
        tap starts nothing; held past the arming delay it records until the
        keys are let go, and the dictation is pasted where the user was."""
        check(hold.modifier_only, f"Expected a modifier-only hold to talk, the binding is {hold}")
        target = self.targets["A"]
        sentinel = f"clipboard before holding {hold}"
        self.ready(target, sentinel)
        self.short_chord_tap(hold)
        self.use_clip(clip)
        previous, played = last_transcript(), clips_finished()
        self.state_mark = self.states.mark()
        self.keys.hold(hold)
        try:
            wait_recording()
            wait_clip_played(played)
            check(status() == "Recording", f"Recording stopped while {hold} was still held")
        finally:
            self.keys.release(hold)
        transcript = wait_new_transcript(previous)
        check_words(transcript["text"], key_words(CLIPS[clip]), "The transcript")
        wait_until("the transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 5)
        wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 5)
        stays(f"the target to keep the focus after {hold} is let go", target.has_focus, 1.0)
        print(f"Hold {hold}: states {self.states_seen()}")

    def dev_default_hotkeys(self) -> None:
        """The build's own defaults, as the Hotkeys page's Reset sets them
        (Scribe Dev: a modifier-only hold to talk and a toggle with modifiers):
        hold to talk through the chord watcher, the toggle through the plugin
        on its trigger edge, discarded with the default discard hotkey."""
        self.use_bindings("reset_hotkeys_to_defaults")
        self.chord_hold_dictation(self.chord("holdToTalk"), "dev_hold")
        toggle, discard = self.chord("toggleDictation"), self.chord("discardDictation")
        target = self.targets["A"]
        sentinel = "clipboard before the default toggle"
        self.ready(target, sentinel)
        self.use_clip("discard")
        previous, count = last_transcript(), transcript_count()
        if toggle.bare_key and self.bindings["toggleDictation"]["trigger"] == "release":
            self.tap_watched_toggle(toggle)
        else:
            self.keys.press(toggle)
        wait_recording()
        self.keys.press(discard)
        wait_until("the recording to be discarded", lambda: status() == "Idle", 10)
        stays(
            "nothing typed, pasted, saved or copied",
            lambda: target.text == ""
            and transcript_count() == count
            and (last_transcript() or {}).get("id") == (previous or {}).get("id")
            and self.clipboard.read() == sentinel,
        )
        print(f"Toggle {toggle} then discard {discard}: states {self.states_seen()}")

    def production_default_hotkeys(self) -> None:
        """Loads the shipped Windows defaults (Hotkeys page, Developer panel:
        "Load my production defaults"): a modifier-only hold to talk and a
        bare toggle key on Release, which starts the toggle-key watcher."""
        starts = self.watcher_starts()
        self.use_bindings("load_production_hotkey_defaults")
        hold, toggle = self.chord("holdToTalk"), self.chord("toggleDictation")
        trigger = self.bindings["toggleDictation"]["trigger"]
        check(hold.modifier_only, f"The shipped hold to talk {hold} is not modifier-only")
        check(toggle.bare_key and trigger == "release", f"The shipped toggle is {toggle} on {trigger}, not a bare key on release")
        check(self.hotkey_status.get("noteChordActive"), "The note chord is not active with the shipped defaults")
        self.wait_toggle_watcher(starts, toggle, "Release")
        check(hotkey_grabbed(toggle), f"Nothing holds {toggle} as a global hotkey, so it would type into apps")
        check(not hotkey_grabbed(Q_KEY), "Q is grabbed while the toggle key is up")

    def bare_key_toggle(self) -> None:
        """Toggle on a bare key, Release trigger (the toggle-key watcher): tap
        to start, tap to stop, the dictation is pasted and the key's own
        character never reaches the app."""
        toggle = self.chord("toggleDictation")
        target = self.targets["A"]
        sentinel = "clipboard before the bare toggle key"
        self.ready(target, sentinel)
        self.use_clip("bare_toggle")
        previous, played = last_transcript(), clips_finished()
        self.tap_watched_toggle(toggle, starts=True)
        wait_recording()
        check(target.text == "", f"The toggle key typed {target.text!r} into the app")
        wait_clip_played(played)
        self.tap_watched_toggle(toggle)
        transcript = wait_new_transcript(previous)
        check_words(transcript["text"], key_words(CLIPS["bare_toggle"]), "The transcript")
        wait_until("the transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 5)
        wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 5)
        self.wait_q_free()
        print(f"Bare toggle {toggle}: states {self.states_seen()}")

    def note_chord(self) -> None:
        """Hold the toggle key and tap Q: a note starts instead of a dictation,
        neither key types, Q is let go with the toggle key and types again,
        and the same chord ends the note, which is saved as a note and never
        pasted."""
        toggle = self.chord("toggleDictation")
        target = self.targets["A"]
        sentinel = "clipboard before the note chord"
        self.ready(target, sentinel)
        self.use_clip("note_chord")
        previous, played, before = last_transcript(), clips_finished(), notes()
        self.press_note_chord(toggle)
        wait_recording()
        check(target.text == "", f"The note chord typed {target.text!r} into the app")
        self.wait_q_free()
        wait_clip_played(played)
        self.press_note_chord(toggle)

        def saved_note():
            added = {key: text for key, text in notes().items() if key not in before}
            return next(iter(added.values()), None)

        note = wait_until("the note to be saved", saved_note, 120, 0.1)
        wait_idle()
        check_words(note, key_words(CLIPS["note_chord"]), "The note")
        check(
            (last_transcript() or {}).get("id") == (previous or {}).get("id"),
            "The note was also saved as the last dictation",
        )
        check(target.text == "", f"The note was typed or pasted into the app: {target.text!r}")
        check(self.clipboard.read() == sentinel, "The note changed the clipboard")
        self.wait_q_free()
        count = transcript_count()
        self.state_mark = self.states.mark()
        self.keys.tap(VK_Q)
        wait_until("Q to type normally after the note chord", lambda: target.text == "q", 5)
        self.nothing_started("typing Q", count)
        print(f"Note chord: note {note!r}")

    def production_chord_hold(self) -> None:
        self.chord_hold_dictation(self.chord("holdToTalk"), "chord_hold")

    def press_trigger_toggle(self) -> None:
        """The toggle set to act on Press: the watcher restarts on the press
        edge, the recording starts while the key is still down, and no Q grab
        is made (the note chord needs Release). Each press must show up as the
        watcher's own "Toggle key pressed" line, so the toggle is proven to come
        from the watcher's press edge and not from some other path."""
        starts = self.watcher_starts()
        self.use_bindings("set_hotkey_trigger", {"action": "toggleDictation", "trigger": "press"})
        try:
            toggle = self.chord("toggleDictation")
            check(toggle.bare_key, f"The toggle is {toggle}, not a bare key")
            check(not self.hotkey_status.get("noteChordActive"), "The note chord is still active on Press")
            self.wait_toggle_watcher(starts, toggle, "Press")
            target = self.targets["A"]
            sentinel = "clipboard before the press trigger"
            self.ready(target, sentinel)
            self.use_clip("press_trigger")
            previous, played = last_transcript(), clips_finished()
            presses = log_text().count(TOGGLE_PRESSED_LOG)
            self.keys.down(toggle.key)
            try:
                wait_until(
                    f"the watcher's {TOGGLE_PRESSED_LOG!r} line while the key is down",
                    lambda: log_text().count(TOGGLE_PRESSED_LOG) > presses,
                    5,
                    0.05,
                )
                wait_recording()
                check(not hotkey_grabbed(Q_KEY), "Q was grabbed with the toggle on Press")
            finally:
                self.keys.up(toggle.key)
            check(target.text == "", f"The toggle key typed {target.text!r} into the app")
            wait_clip_played(played)
            presses = log_text().count(TOGGLE_PRESSED_LOG)
            self.keys.down(toggle.key)
            try:
                wait_until(
                    f"the watcher's {TOGGLE_PRESSED_LOG!r} line for the second press",
                    lambda: log_text().count(TOGGLE_PRESSED_LOG) > presses,
                    5,
                    0.05,
                )
                wait_until("the press to stop the recording", lambda: status() != "Recording", 10)
            finally:
                self.keys.up(toggle.key)
            transcript = wait_new_transcript(previous)
            check_words(transcript["text"], key_words(CLIPS["press_trigger"]), "The transcript")
            wait_until("the transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 5)
            wait_until("the clipboard to be restored", lambda: self.clipboard.read() == sentinel, 5)
            print(f"Press trigger: states {self.states_seen()}; {log_lines(TOGGLE_PRESSED_LOG)[-2:]}")
        finally:
            starts = self.watcher_starts()
            self.use_bindings("set_hotkey_trigger", {"action": "toggleDictation", "trigger": "release"})
        self.wait_toggle_watcher(starts, self.chord("toggleDictation"), "Release")

    def rebind_while_watching(self) -> None:
        """Rebinding the toggle while its watcher runs: to another bare key
        (the old key types again and starts nothing, the new key toggles),
        then to a combination with modifiers (no watcher, the new bare key
        types again, the plugin toggles)."""
        old = self.chord("toggleDictation")
        check(old.bare_key, f"The toggle is {old}, not a bare key")
        new_text = "K" if old.key != KEY_NAMES["k"] else "J"
        starts = self.watcher_starts()
        self.use_bindings("rebind_hotkey", {"action": "toggleDictation", "shortcut": new_text})
        new = self.chord("toggleDictation")
        self.wait_toggle_watcher(starts, new, "Release")
        check(not hotkey_grabbed(old), f"{old} is still grabbed after rebinding the toggle to {new}")
        target = self.targets["A"]
        self.ready(target, "clipboard before rebinding")
        count = transcript_count()
        self.keys.tap(old.key)
        wait_until(f"the old toggle key {old} to type normally", lambda: target.text != "", 5)
        check(target.text == "`" or old.key != VK_OEM_3, f"The old toggle key typed {target.text!r}")
        self.nothing_started(f"the old toggle key {old}", count)

        sentinel = "clipboard before the new toggle key"
        self.ready(target, sentinel)
        self.use_clip("rebind_key")
        previous, played = last_transcript(), clips_finished()
        self.tap_watched_toggle(new)
        wait_recording()
        check(target.text == "", f"The new toggle key typed {target.text!r} into the app")
        wait_clip_played(played)
        self.tap_watched_toggle(new)
        transcript = wait_new_transcript(previous)
        check_words(transcript["text"], key_words(CLIPS["rebind_key"]), "The transcript")
        wait_until("the transcript in the focused box", lambda: target.text.strip() == transcript["text"].strip(), 5)
        self.wait_q_free()

        combo_text = f"Ctrl+Alt+Shift+{new_text}"
        watchers = self.watcher_starts()
        self.use_bindings("rebind_hotkey", {"action": "toggleDictation", "shortcut": combo_text})
        combo = self.chord("toggleDictation")
        check(not self.hotkey_status.get("noteChordActive"), f"The note chord is still active with {combo}")
        check(hotkey_grabbed(combo), f"The plugin does not hold {combo}")
        check(not hotkey_grabbed(new), f"{new} is still grabbed after rebinding the toggle to {combo}")
        check(not hotkey_grabbed(old), f"{old} is grabbed again after rebinding the toggle to {combo}")
        stays(
            "no toggle-key watcher started for a combination",
            lambda: self.watcher_starts() == watchers,
            0.5,
        )
        self.ready(target, "clipboard before the combination")
        count = transcript_count()
        self.keys.tap(new.key)
        wait_until(f"the former toggle key {new} to type normally", lambda: target.text == new_text.lower(), 5)
        self.nothing_started(f"the former toggle key {new}", count)

        sentinel = "clipboard before the combination toggle"
        self.ready(target, sentinel)
        self.use_clip("discard")
        previous, count = last_transcript(), transcript_count()
        self.keys.press(combo)
        wait_recording()
        self.keys.press(self.chord("discardDictation"))
        wait_until("the recording to be discarded", lambda: status() == "Idle", 10)
        stays(
            "nothing typed, pasted or saved",
            lambda: target.text == "" and transcript_count() == count and self.clipboard.read() == sentinel,
        )
        check(not hotkey_grabbed(Q_KEY), "Q is grabbed after the combination toggle")
        print(f"Combination toggle {combo}: states {self.states_seen()}")

    def open_dashboard_hotkey(self) -> None:
        """The Open Dashboard hotkey opens the dashboard hidden in the tray and
        focuses it; pressed again it hides it (dashboardHotkeyToggles)."""
        chord = self.chord("openDashboard")
        hwnd = self.dashboard
        if user32.IsWindowVisible(hwnd):
            self.close_to_tray()
        self.keys.press(chord)
        wait_until(
            f"{chord} to open and focus the dashboard",
            lambda: user32.IsWindowVisible(hwnd) and user32.GetForegroundWindow() == hwnd,
            10,
        )
        self.shot("dashboard-hotkey-open")
        toggles = json.loads(setting_value("app_settings")).get("dashboardHotkeyToggles", True)
        self.keys.press(chord)
        if toggles:
            wait_until(f"{chord} to hide the focused dashboard again", lambda: not user32.IsWindowVisible(hwnd), 10)
        else:
            stays("the dashboard to stay open", lambda: user32.IsWindowVisible(hwnd))
            self.close_to_tray()

    def restore_test_hotkeys(self) -> None:
        """Puts the test bindings back through the settings path, so the
        scenarios after this group drive the plugin path as before."""
        check(self.test_hotkeys is not None, "The test bindings were never read")
        self.keys.release_all()
        wait_idle()
        self.commands.invoke("patch_settings", {"patch": {"hotkeys": self.test_hotkeys}})
        self.use_bindings("get_hotkey_status")
        for action, (chord, _) in HOTKEYS.items():
            check(self.bindings[action]["shortcut"] == chord, f"{action} is {self.bindings[action]['shortcut']}, not {chord}")
        check(not hotkey_grabbed(Q_KEY), "Q is still grabbed")
        evidence = log_lines("Toggle key watcher", "Toggle key pressed", "Toggle key released", "Note chord")
        (self.out / "hotkey-watcher-log.txt").write_text("\n".join(evidence) + "\n", encoding="utf-8")
        print(f"Watcher log lines ({len(evidence)}), also in hotkey-watcher-log.txt:")
        for line in evidence:
            print(f"  {line}")

    def no_folder_mismatch(self) -> None:
        """The engine and Tauri resolved the same folders at startup: the app
        logs "The engine's folder differs from Tauri's" otherwise (user data
        could then be written to one folder and deleted from another)."""
        folder_mismatch.check_log(log_text())

    def hotkey_evidence(self, held: list[int]) -> str:
        """What a failed hotkey scenario saw, for its report."""
        lines = [f"Keys held at the failure (now released): {[f'0x{vk:X}' for vk in held]}"]
        for name, target in self.targets.items():
            try:
                lines.append(f"Target {name} text: {target.text!r}")
            except OSError as error:
                lines.append(f"Target {name} text unreadable: {error}")
        lines.append(f"States since the scenario's last mark: {self.states_seen()}")
        lines.append(f"Status: {read_status()}")
        lines.append("Log tail:")
        lines += [f"  {line}" for line in log_text().splitlines()[-25:]]
        return "\n".join(lines)

    # -- driver ------------------------------------------------------------

    def scenario(self, name: str, body, evidence: bool = False) -> bool:
        started = time.monotonic()
        try:
            body()
            self.results.append((name, True, "", time.monotonic() - started))
            print(f"PASS {name} ({time.monotonic() - started:.1f} s)", flush=True)
            return True
        except Exception as error:
            held = self.keys.release_all()
            detail = f"{type(error).__name__}: {error}"
            self.results.append((name, False, detail, time.monotonic() - started))
            print(f"FAIL {name}: {detail}", flush=True)
            traceback.print_exc()
            if evidence:
                print(self.hotkey_evidence(held), flush=True)
            self.shot(f"failed-{re.sub(r'[^a-z0-9]+', '-', name.lower())}")
            return False
        finally:
            self.keys.release_all()

    def default_hotkey_group(self) -> None:
        if not self.scenario("Default hotkeys: the app's hotkey commands and state stream", self.hotkeys_connect, True):
            return
        for name, body in [
            ("Default hotkeys (this build's own): modifier-only hold to talk, toggle and discard", self.dev_default_hotkeys),
            ("Default hotkeys (shipped): load them, the toggle-key watcher starts", self.production_default_hotkeys),
            ("Default hotkeys: bare toggle key on Release types nothing and pastes the dictation", self.bare_key_toggle),
            ("Default hotkeys: hold the toggle key and tap Q for a note", self.note_chord),
            ("Default hotkeys: modifier-only hold to talk, a short tap starts nothing", self.production_chord_hold),
            ("Default hotkeys: toggle on the Press trigger", self.press_trigger_toggle),
            ("Default hotkeys: rebind the watched toggle to a bare key, then a combination", self.rebind_while_watching),
            ("Default hotkeys: Open Dashboard opens and hides the dashboard", self.open_dashboard_hotkey),
        ]:
            self.scenario(name, body, True)
        self.scenario("Default hotkeys: put the test bindings back", self.restore_test_hotkeys, True)

    def run(self) -> int:
        try:
            self.scenario("Setup: self-test, test settings, launch", self.setup)
            if self.results[-1][1]:
                for name, body in [
                    ("Keep the speech model loaded: it loads at startup", self.model_loads_at_startup),
                    ("Closing the dashboard hides it to the tray", self.close_to_tray),
                    ("Toggle dictation pastes and restores the clipboard", self.toggle_paste_restores_clipboard),
                    ("Hold to talk", self.hold_to_talk),
                    ("A slow app still pastes the dictation", self.slow_app_paste),
                    ("An app that ignores the paste gets its clipboard back", self.ignored_paste),
                    ("Output mode: Save Only", self.save_only),
                    ("Output mode: Copy to Clipboard", self.copy_clipboard),
                    ("Output mode: Copy + Paste", self.copy_and_paste),
                    ("Typed insertion leaves the clipboard untouched", self.direct_insert),
                    ("Switching apps mid-dictation pastes nowhere", self.switch_apps),
                    ("Paste last transcript", self.paste_last),
                    ("Discard a dictation", self.discard),
                    ("Transform Selection with a local AI server", self.transform_selection),
                ]:
                    self.scenario(name, body)
                self.default_hotkey_group()
                for name, body in [
                    ("Keep the speech model loaded off: loads on dictation, unloads after a minute", self.keep_loaded_off),
                    ("Parakeet v3: switch engines, dictate, never both loaded", self.parakeet_engine),
                    ("Never two speech models in memory at once", self.never_two_models),
                    # Last: it ends the app.
                    ("A hard kill takes the speech model down with Scribe", self.hard_kill_stops_speech_engine),
                ]:
                    self.scenario(name, body)
                # After the hard kill: the log it read is complete, and the
                # app no longer needs to be running.
                self.scenario("The engine's folders match Tauri's (app log)", self.no_folder_mismatch)
        finally:
            self.keys.release_all()
            if self.states:
                self.states.close()
            if self.commands:
                self.commands.close()
            self.monitor.stop()
            self.app.stop()
            for target in self.targets.values():
                target.close()
            (self.out / "llm-requests.json").write_text(json.dumps(self.ai.requests, indent=2), encoding="utf-8")

        lines = ["## Windows end-to-end", "", "| Scenario | Result | Time | Details |", "|---|---|---|---|"]
        for name, ok, detail, seconds in self.results:
            lines.append(f"| {name} | {'PASS' if ok else 'FAIL'} | {seconds:.1f} s | {detail.replace('|', '/')} |")
        failed = sum(1 for result in self.results if not result[1])
        lines += ["", f"{failed} of {len(self.results)} failed."]
        report = "\n".join(lines)
        print(report)
        (self.out / "results.md").write_text(report + "\n", encoding="utf-8")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a", encoding="utf-8") as handle:
                handle.write(report + "\n")
        return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", required=True, help="the Scribe Dev executable built with --features e2e-audio")
    parser.add_argument("--fixtures", required=True, help="app/src-tauri/tests/fixtures")
    parser.add_argument("--out", required=True, help="where results, screenshots and work files go")
    parser.add_argument(
        "--webview-debug-port",
        type=int,
        default=9222,
        help="the WebView2 remote debugging port the test build opens (webview_debug_config.py)",
    )
    parser.add_argument(
        "--overwrite-scribe-dev-profile",
        action="store_true",
        help="run outside GitHub Actions, accepting that this machine's Scribe Dev settings, history and hotkeys are overwritten",
    )
    args = parser.parse_args()
    if not os.environ.get("GITHUB_ACTIONS") and not args.overwrite_scribe_dev_profile:
        print(
            "Refusing to run: this test uses the real Scribe Dev profile on this machine "
            f"({DATA_DIR} and {CACHE_DIR}). It rewrites its settings and hotkeys, adds dictations and notes "
            "to its history, and kills Scribe Dev. It is meant for a disposable GitHub Actions runner. "
            "Pass --overwrite-scribe-dev-profile to run it here anyway.",
            file=sys.stderr,
        )
        return 2
    return Run(args).run()


if __name__ == "__main__":
    sys.exit(main())
