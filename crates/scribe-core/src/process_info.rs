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
///
/// The check and the stop act on one process, not twice on a pid: on
/// Windows through one handle (opened for both), on Linux through a pidfd
/// opened before the check. So if the process ends and its pid is reused in
/// between (another thread may be reaping it right now), the newcomer is
/// never stopped. macOS has no such handle and checks, then stops, by pid.
pub fn terminate_if_same(recorded: &ProcessIdentity) -> bool {
    imp::terminate_if(recorded.pid, &|current| same_process(recorded, current))
}

/// Physical memory (RAM) the process uses, in bytes: the working set on
/// Windows (close to Task Manager's "Memory" column), anonymous resident
/// memory on Linux, and the physical footprint Activity Monitor shows on
/// macOS. This is the RAM unloading the model gives back. (Windows' private
/// commit is larger: ggml reserves buffers it never fills, so it would
/// overstate the saving.)
pub fn resident_memory_bytes(pid: u32) -> Option<u64> {
    imp::resident_memory_bytes(pid)
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
        read_identity(&handle)
    }

    /// Opens the process once with both rights, checks who it is on that
    /// handle and stops that same handle: the handle keeps naming the process
    /// it was opened on, even if its pid is reused meanwhile.
    pub fn terminate_if(pid: u32, check: &dyn Fn(&super::ProcessIdentity) -> bool) -> bool {
        let Some(handle) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE) else {
            return false;
        };
        let Some((exe, start_time)) = read_identity(&handle) else {
            return false;
        };
        let current = super::ProcessIdentity {
            pid,
            exe,
            start_time,
        };
        check(&current) && unsafe { TerminateProcess(handle.0, 1).is_ok() }
    }

    fn read_identity(handle: &OwnedHandle) -> Option<(PathBuf, u64)> {
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

    pub fn resident_memory_bytes(pid: u32) -> Option<u64> {
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
        Some(counters.WorkingSetSize as u64)
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

    /// Opens a pidfd first, then checks who has the pid, then signals the
    /// pidfd. The pidfd names the process that had the pid when it was
    /// opened; if the check (after it) still finds the recorded process, that
    /// is the same process, so the signal can only reach it. Kernels older
    /// than 5.3 have no pidfds: there it checks, then signals the pid.
    pub fn terminate_if(pid: u32, check: &dyn Fn(&super::ProcessIdentity) -> bool) -> bool {
        let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        let same = || super::identity(pid).is_some_and(|current| check(&current));
        if pidfd < 0 {
            return same() && unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) == 0 };
        }
        let pidfd = pidfd as libc::c_int;
        let stopped = same()
            && unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    pidfd,
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                ) == 0
            };
        unsafe {
            libc::close(pidfd);
        }
        stopped
    }

    pub fn resident_memory_bytes(pid: u32) -> Option<u64> {
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

    /// macOS has no process handle to hold across the check, so this checks,
    /// then signals the pid.
    pub fn terminate_if(pid: u32, check: &dyn Fn(&super::ProcessIdentity) -> bool) -> bool {
        super::identity(pid).is_some_and(|current| check(&current))
            && unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) == 0 }
    }

    pub fn resident_memory_bytes(pid: u32) -> Option<u64> {
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

    /// The process is checked and stopped through one handle (a pidfd on
    /// Linux): a match stops it, a mismatch leaves it running.
    #[test]
    fn stops_only_the_recorded_process() {
        let mut command = if cfg!(windows) {
            let mut command = std::process::Command::new("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = std::process::Command::new("sleep");
            command.arg("30");
            command
        };
        let mut child = command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let recorded = identity(child.id()).expect("the child is readable");

        let started_later = ProcessIdentity {
            start_time: recorded.start_time + 1,
            ..recorded.clone()
        };
        assert!(!terminate_if_same(&started_later));
        assert!(child.try_wait().unwrap().is_none(), "a mismatch stopped it");

        assert!(terminate_if_same(&recorded));
        let stopped = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(stopped.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Gone (and reaped): nothing to stop any more.
        assert!(!terminate_if_same(&recorded));
    }

    #[test]
    fn reads_own_resident_memory() {
        let bytes = resident_memory_bytes(std::process::id()).expect("own memory is readable");
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
