//! Process liveness (contract sections 5.2 and 7).
//!
//! The discovery file carries Scribe's `pid`. A consumer may pid-check it; if
//! the process is dead, the file is treated as void (Scribe crashed without
//! cleaning up), so a stale file can never wedge a consumer into believing
//! dictation is still active.

/// Is a process with this pid currently alive?
///
/// - Unix: `kill(pid, 0)` - alive if it succeeds or fails with `EPERM`
///   (exists but we lack permission); dead only on `ESRCH`.
/// - Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` - alive if the
///   handle opens.
/// - Other platforms: we cannot check cheaply, so we conservatively report
///   `true` (never manufacture a false "offline").
pub fn pid_alive(pid: i64) -> bool {
    platform::pid_alive(pid)
}

#[cfg(unix)]
mod platform {
    pub fn pid_alive(pid: i64) -> bool {
        if pid <= 0 {
            return false;
        }
        // SAFETY: `kill` with signal 0 performs error checking without sending
        // a signal. It has no memory effects.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        // errno == EPERM means the process exists but we may not signal it.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    pub fn pid_alive(pid: i64) -> bool {
        if pid <= 0 {
            return false;
        }
        // SAFETY: OpenProcess returns a null handle on failure; we only close a
        // non-null handle.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
            if handle.is_null() {
                return false;
            }
            CloseHandle(handle);
            true
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub fn pid_alive(_pid: i64) -> bool {
        // Unknown platform: cannot check, so never claim the process is dead.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        let me = std::process::id() as i64;
        assert!(pid_alive(me));
    }

    #[test]
    fn nonpositive_pid_is_dead() {
        assert!(!pid_alive(0));
        assert!(!pid_alive(-1));
    }

    #[cfg(unix)]
    #[test]
    fn an_almost_certainly_dead_pid_is_dead() {
        // Very high pid unlikely to be in use; ESRCH -> dead.
        assert!(!pid_alive(2_000_000_000));
    }
}
