//! Lending the clipboard to one paste, and knowing when the paste target has
//! actually read it (issue #105).
//!
//! Scribe pastes by putting the dictation on the clipboard, sending Ctrl+V and
//! then putting the user's own clipboard back. The restore used to follow a
//! fixed 180 ms timer, so an app that handled the keystroke later (a loaded
//! PC, a large Electron app, a remote desktop) read the clipboard after the
//! restore and pasted the user's old text instead of the dictation.
//!
//! The clipboard sequence number can't tell when that read happens: it only
//! moves on writes. So the dictation is published with delayed rendering:
//! the clipboard announces `CF_UNICODETEXT` but holds no bytes yet, and the
//! first app to ask for them makes Windows send the clipboard owner
//! `WM_RENDERFORMAT`. Scribe hands the text over right there and treats that
//! request as the paste being consumed. Only then (or after a generous
//! timeout, for an app that never reads) is the user's clipboard put back.
//!
//! Any reader counts, not only the window in front: browsers and WebView2
//! apps read from a helper process, Store apps from their own process under
//! ApplicationFrameHost, and remote desktops through rdpclip. Clipboard
//! managers that honor the privacy formats set below never read it.
//!
//! The owner is a message-only window on its own thread, because
//! `WM_RENDERFORMAT` arrives synchronously while the reader waits inside
//! `GetClipboardData`; the thread that sends Ctrl+V and then waits for the
//! read can't also be the one answering it.

use super::mark_clipboard_private;
use crate::error::CommandError;
use crate::platform::LoanOutcome;
use std::{
    ffi::c_void,
    mem::size_of,
    sync::{mpsc, Condvar, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};
use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardOwner, GetOpenClipboardWindow,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowThreadProcessId,
    RegisterClassW, SendMessageW, TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WNDCLASSW,
};

const CF_UNICODETEXT: u32 = 13;
/// Sent to the owner window to publish the pending text from its own thread.
const WM_LEND: u32 = WM_APP + 1;

/// The text currently lent to the clipboard, shared with the owner thread.
struct Loan {
    /// UTF-16, NUL-terminated, rendered on the first request.
    text: Vec<u16>,
    state: LoanState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoanState {
    Lent,
    Read,
    Replaced,
}

#[derive(Default)]
struct Shared {
    /// Text waiting for `WM_LEND` to publish it.
    pending: Option<Vec<u16>>,
    loan: Option<Loan>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    pending: None,
    loan: None,
});
static CHANGED: Condvar = Condvar::new();
/// The owner window's handle (as an integer, since `HWND` isn't `Send`), or
/// why it couldn't be created.
static OWNER: OnceLock<Result<isize, String>> = OnceLock::new();

/// A dictation lent to the clipboard for one paste. Call `wait_for_read`
/// after sending the paste keystroke, then restore the user's clipboard.
pub struct ClipboardLoan {
    watched: bool,
}

/// Puts `text` on the clipboard for one paste, privately (kept out of
/// Clipboard History, the cloud clipboard and clipboard managers), in a way
/// that lets `wait_for_read` see the paste target read it.
pub fn lend_clipboard_text(text: &str) -> Result<ClipboardLoan, CommandError> {
    match owner_window() {
        Ok(owner) => {
            lock_shared().pending = Some(text.encode_utf16().chain(Some(0)).collect());
            let published = unsafe { SendMessageW(owner, WM_LEND, WPARAM(0), LPARAM(0)) };
            if published.0 == 1 {
                return Ok(ClipboardLoan { watched: true });
            }
            lock_shared().pending = None;
            log::warn!("Could not lend the clipboard with delayed rendering; using a timed paste");
        }
        Err(error) => {
            log::warn!("No clipboard owner window ({error}); using a timed paste");
        }
    }
    crate::output::set_private_clipboard_text(text)?;
    Ok(ClipboardLoan { watched: false })
}

impl ClipboardLoan {
    /// Blocks until an app has read the lent text, someone else replaced
    /// it, or `timeout` passes. Afterwards the text is no longer served.
    pub fn wait_for_read(self, timeout: Duration) -> LoanOutcome {
        if !self.watched {
            thread::sleep(UNOBSERVED_WAIT);
            return LoanOutcome::Unobserved;
        }
        let started = Instant::now();
        let shared = lock_shared();
        let (mut shared, _) = CHANGED
            .wait_timeout_while(
                shared,
                timeout,
                |shared| matches!(&shared.loan, Some(loan) if loan.state == LoanState::Lent),
            )
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let outcome = match shared.loan.take().map(|loan| loan.state) {
            Some(LoanState::Read) => LoanOutcome::Read,
            Some(LoanState::Lent) => LoanOutcome::NotRead,
            Some(LoanState::Replaced) | None => LoanOutcome::Replaced,
        };
        drop(shared);
        log::info!(
            "Paste clipboard: {:?} after {} ms",
            outcome,
            started.elapsed().as_millis()
        );
        if outcome == LoanOutcome::Read {
            // The reader still holds the clipboard open while it copies the
            // text, and some apps read in more than one pass; the restore's
            // own open retries cover the rest.
            thread::sleep(AFTER_READ_SETTLE);
        }
        outcome
    }
}

/// Stand-in wait when the read can't be observed (the old fixed delay).
const UNOBSERVED_WAIT: Duration = Duration::from_millis(180);
/// Grace after the read before the clipboard is reclaimed.
const AFTER_READ_SETTLE: Duration = Duration::from_millis(50);

fn lock_shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn owner_window() -> Result<HWND, String> {
    OWNER
        .get_or_init(|| {
            let (ready, receive) = mpsc::channel();
            thread::Builder::new()
                .name("clipboard-owner".into())
                .spawn(move || run_owner(ready))
                .map_err(|error| error.to_string())?;
            receive
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| "the clipboard owner thread did not start".to_string())?
        })
        .clone()
        .map(|handle| HWND(handle as *mut c_void))
}

/// Creates the message-only owner window and pumps its messages for the
/// life of the app.
fn run_owner(ready: mpsc::Sender<Result<isize, String>>) {
    let created = (|| -> Result<HWND, String> {
        let module = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
        let instance = HINSTANCE(module.0);
        let class_name = w!("ScribeClipboardOwner");
        let class = WNDCLASSW {
            lpfnWndProc: Some(owner_proc),
            hInstance: instance,
            lpszClassName: class_name,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err("could not register the clipboard owner window class".into());
        }
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                w!("Scribe clipboard"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                None,
                instance,
                None,
            )
        }
        .map_err(|error| error.to_string())
    })();

    let hwnd = match created {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(hwnd.0 as isize));

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

extern "system" fn owner_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_LEND => LRESULT(publish(hwnd) as isize),
        WM_RENDERFORMAT => {
            if wparam.0 as u32 == CF_UNICODETEXT {
                render(true);
            }
            LRESULT(0)
        }
        // The owner window is going away while the text is still announced
        // (it never is destroyed before the app exits): hand it over so the
        // clipboard isn't left with an empty promise.
        WM_RENDERALLFORMATS => {
            if unsafe { OpenClipboard(hwnd) }.is_ok() {
                if unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == hwnd) {
                    render(false);
                }
                let _ = unsafe { CloseClipboard() };
            }
            LRESULT(0)
        }
        // Someone emptied the clipboard: another app copied something, or
        // Scribe's own restore (by then the loan has ended).
        WM_DESTROYCLIPBOARD => {
            let mut shared = lock_shared();
            if let Some(loan) = shared.loan.as_mut() {
                if loan.state == LoanState::Lent {
                    loan.state = LoanState::Replaced;
                    CHANGED.notify_all();
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Runs on the owner thread: takes the clipboard and announces the pending
/// text for delayed rendering. Returns whether it worked.
fn publish(hwnd: HWND) -> bool {
    let Some(text) = lock_shared().pending.take() else {
        return false;
    };
    if !open_clipboard_with_retry(hwnd) {
        return false;
    }
    // Emptying first: the previous owner (possibly this window, from an
    // earlier paste) gets WM_DESTROYCLIPBOARD now, before the new loan exists.
    let published = unsafe { EmptyClipboard() }.is_ok() && {
        lock_shared().loan = Some(Loan {
            text,
            state: LoanState::Lent,
        });
        // A null handle announces the format without data. The binding
        // reports the null return as an error, so check the format instead.
        let _ = unsafe { SetClipboardData(CF_UNICODETEXT, HANDLE::default()) };
        unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_ok()
    };
    if published {
        mark_clipboard_private();
    } else {
        lock_shared().loan = None;
    }
    let _ = unsafe { CloseClipboard() };
    published
}

/// Runs on the owner thread inside `WM_RENDERFORMAT` (the clipboard is
/// already open by the reader) or `WM_RENDERALLFORMATS`: hands the lent text
/// over. `is_read` marks the loan as consumed.
fn render(is_read: bool) {
    let mut shared = lock_shared();
    let Some(loan) = shared.loan.as_mut() else {
        return;
    };
    let bytes = loan.text.len() * size_of::<u16>();
    let Ok(hglobal) = (unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }) else {
        log::warn!("Could not allocate memory for the pasted text");
        return;
    };
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        let _ = unsafe { GlobalFree(hglobal) };
        return;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(loan.text.as_ptr(), ptr as *mut u16, loan.text.len());
        let _ = GlobalUnlock(hglobal);
    }
    if unsafe { SetClipboardData(CF_UNICODETEXT, HANDLE(hglobal.0)) }.is_err() {
        let _ = unsafe { GlobalFree(hglobal) };
        log::warn!("Could not hand the pasted text to the clipboard");
        return;
    }
    if is_read && loan.state == LoanState::Lent {
        loan.state = LoanState::Read;
        log::info!("Paste clipboard: read by process {}", reader_process());
        CHANGED.notify_all();
    }
}

/// The process that has the clipboard open (the reader), for the log.
fn reader_process() -> u32 {
    let Ok(window) = (unsafe { GetOpenClipboardWindow() }) else {
        return 0;
    };
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    process_id
}

fn open_clipboard_with_retry(hwnd: HWND) -> bool {
    for attempt in 0..10 {
        if unsafe { OpenClipboard(hwnd) }.is_ok() {
            return true;
        }
        if attempt < 9 {
            thread::sleep(Duration::from_millis(20));
        }
    }
    false
}
