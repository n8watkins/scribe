//! Small, OS-specific facts about another process: who it is (its executable
//! and start time, so a reused pid is never mistaken for it), how much memory
//! it holds, and a way to stop it.
//!
//! Used for the speech engine processes Scribe starts (whisper-server, the
//! Parakeet sidecar): the stale-process sweep at startup and the memory figure
//! shown next to "Keep the speech model loaded".

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What identifies one particular process. A pid alone can be reused by the
/// OS for an unrelated program, so a match needs all three fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessIdentity {
    pub pid: u32,
    pub exe: PathBuf,
    /// Process start time in an OS-specific unit (a FILETIME on Windows,
    /// clock ticks since boot on Linux, microseconds since the epoch on
    /// macOS). Only ever compared with another value from this module.
    pub start_time: u64,
}

/// Reads the identity of a running process, or None when it is gone or not
/// readable by this user.
pub fn identity(pid: u32) -> Option<ProcessIdentity> {
    let (exe, start_time) = imp::exe_and_start_time(pid)?;
    Some(ProcessIdentity {
        pid,
        exe,
        start_time,
    })
}

/// True when the process `recorded` describes is still running: same pid,
/// same executable and same start time.
pub fn is_same_process_running(recorded: &ProcessIdentity) -> bool {
    identity(recorded.pid).is_some_and(|current| same_process(recorded, &current))
}

fn same_process(recorded: &ProcessIdentity, current: &ProcessIdentity) -> bool {
    recorded.pid == current.pid
        && recorded.start_time == current.start_time
        && paths_match(&recorded.exe, &current.exe)
}

fn paths_match(a: &std::path::Path, b: &std::path::Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// Stops the process `recorded` describes, but only after checking it is
/// still that process. Returns true when it was running and was stopped.
pub fn terminate_if_same(recorded: &ProcessIdentity) -> bool {
    if !is_same_process_running(recorded) {
        return false;
    }
    imp::terminate(recorded.pid)
}

/// Memory the process holds for itself, in bytes: private commit on Windows
/// (what Task Manager calls "Commit size"), anonymous resident memory on
/// Linux, and the physical footprint Activity Monitor shows on macOS. This is
/// what unloading the model gives back.
pub fn private_memory_bytes(pid: u32) -> Option<u64> {
    imp::private_memory_bytes(pid)
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, FILETIME, HANDLE},
            System::{
                ProcessStatus::{
                    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
                },
                Threading::{
                    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
                    PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                    PROCESS_TERMINATE,
                },
            },
        },
    };

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<OwnedHandle> {
        unsafe { OpenProcess(access, false, pid).ok().map(OwnedHandle) }
    }

    pub fn exe_and_start_time(pid: u32) -> Option<(PathBuf, u64)> {
        let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle.0,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut size,
            )
            .ok()?;
        }
        let exe = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user).ok()?;
        }
        let start = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        Some((exe, start))
    }

    pub fn terminate(pid: u32) -> bool {
        let Some(handle) = open(pid, PROCESS_TERMINATE) else {
            return false;
        };
        unsafe { TerminateProcess(handle.0, 1).is_ok() }
    }

    pub fn private_memory_bytes(pid: u32) -> Option<u64> {
        let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                handle.0,
                &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
                counters.cb,
            )
            .ok()?;
        }
        Some(counters.PrivateUsage as u64)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::{fs, path::PathBuf};

    pub fn exe_and_start_time(pid: u32) -> Option<(PathBuf, u64)> {
        let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        Some((exe, start_time_from_stat(&stat)?))
    }

    /// Field 22 of /proc/<pid>/stat (starttime). The command name in field 2
    /// may hold spaces and parentheses, so fields are counted after the last
    /// closing parenthesis.
    pub(super) fn start_time_from_stat(stat: &str) -> Option<u64> {
        let rest = &stat[stat.rfind(')')? + 1..];
        // `rest` starts at field 3 (state); starttime is field 22.
        rest.split_whitespace().nth(19)?.parse().ok()
    }

    pub fn terminate(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) == 0 }
    }

    pub fn private_memory_bytes(pid: u32) -> Option<u64> {
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status.lines().find_map(|line| {
            let value = line.strip_prefix("RssAnon:")?;
            let kib: u64 = value.trim().trim_end_matches("kB").trim().parse().ok()?;
            Some(kib * 1024)
        })
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    pub fn exe_and_start_time(pid: u32) -> Option<(PathBuf, u64)> {
        let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let length = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                path.as_mut_ptr().cast(),
                path.len() as u32,
            )
        };
        if length <= 0 {
            return None;
        }
        path.truncate(length as usize);
        let exe = PathBuf::from(String::from_utf8_lossy(&path).into_owned());

        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let read = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if read != size {
            return None;
        }
        Some((
            exe,
            info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        ))
    }

    pub fn terminate(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) == 0 }
    }

    pub fn private_memory_bytes(pid: u32) -> Option<u64> {
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::proc_pid_rusage(
                pid as libc::c_int,
                libc::RUSAGE_INFO_V2,
                (&mut info as *mut libc::rusage_info_v2).cast(),
            )
        };
        (result == 0).then_some(info.ri_phys_footprint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_this_process() {
        let me = identity(std::process::id()).expect("own process is readable");
        assert_eq!(me.pid, std::process::id());
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            me.exe.file_name().map(|name| name.to_ascii_lowercase()),
            exe.file_name().map(|name| name.to_ascii_lowercase())
        );
        assert!(is_same_process_running(&me));
    }

    #[test]
    fn a_reused_pid_is_not_the_same_process() {
        let me = identity(std::process::id()).unwrap();
        let other_start = ProcessIdentity {
            start_time: me.start_time + 1,
            ..me.clone()
        };
        assert!(!is_same_process_running(&other_start));
        let other_exe = ProcessIdentity {
            exe: PathBuf::from("/not/the/same/program"),
            ..me
        };
        assert!(!is_same_process_running(&other_exe));
        // Never stops a process that only shares the pid.
        assert!(!terminate_if_same(&other_exe));
    }

    #[test]
    fn reads_own_private_memory() {
        let bytes = private_memory_bytes(std::process::id()).expect("own memory is readable");
        assert!(bytes > 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_start_time_after_a_tricky_command_name() {
        let stat =
            "1234 (a) b (c) S 1 1234 1234 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1000 10";
        assert_eq!(imp::start_time_from_stat(stat), Some(987654));
    }
}
