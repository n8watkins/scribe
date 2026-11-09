"""Scribe's Windows end-to-end test.

Drives a Scribe Dev build made with the `e2e-audio` feature the way a person
would: global hotkeys pressed with injected keystrokes, dictation into stand-in
app windows (target.ps1), the clipboard, and app switches. Hosted runners have
no microphone, so the app records a WAV file in place of one (see
app/src-tauri/src/test_audio.rs); hotkeys, recording, Whisper and output all
run for real.

Every check reads real state: the target window's text, the clipboard,
status.json, the app's database and its log. Waits poll for a state, never a
fixed sleep.

Python standard library only. Usage:
    python run.py --exe "path\\Scribe Dev.exe" --fixtures DIR --out DIR
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import re
import shutil
import sqlite3
import struct
import subprocess
import sys
import threading
import time
import traceback
import zlib
from ctypes import wintypes
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
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
}
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


# --------------------------------------------------------------------------
# The app
# --------------------------------------------------------------------------


class Scribe:
    def __init__(self, exe: Path, clip_path: Path, out: Path) -> None:
        self.exe = exe
        self.clip_path = clip_path
        self.out = out
        self.process: subprocess.Popen | None = None

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
        self.app = Scribe(Path(args.exe), self.clip_path, self.out)
        self.clipboard = Clipboard()
        self.ai = FakeLocalAi()
        self.results: list[tuple[str, bool, str, float]] = []
        self.targets: dict[str, Target] = {}
        self.clips: dict[str, Path] = {"jfk": self.fixtures / "jfk.wav"}

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

    # -- driver ------------------------------------------------------------

    def scenario(self, name: str, body) -> None:
        started = time.monotonic()
        try:
            body()
            self.results.append((name, True, "", time.monotonic() - started))
            print(f"PASS {name} ({time.monotonic() - started:.1f} s)", flush=True)
        except Exception as error:
            detail = f"{type(error).__name__}: {error}"
            self.results.append((name, False, detail, time.monotonic() - started))
            print(f"FAIL {name}: {detail}", flush=True)
            traceback.print_exc()
            self.shot(f"failed-{re.sub(r'[^a-z0-9]+', '-', name.lower())}")

    def run(self) -> int:
        try:
            self.scenario("Setup: self-test, test settings, launch", self.setup)
            if self.results[-1][1]:
                for name, body in [
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
        finally:
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
    return Run(parser.parse_args()).run()


if __name__ == "__main__":
    sys.exit(main())
