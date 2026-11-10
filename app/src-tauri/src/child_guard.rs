//! Keeps speech engine processes (whisper-server, whisper-cli, the Parakeet
//! sidecar) from outliving Scribe, however Scribe ends: a normal quit, a
//! crash, a Task Manager kill or a forced shutdown.
//!
//! - Windows: every child joins one Job Object created with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Scribe holds the only handle to
//!   it, so when Scribe's process ends for any reason Windows closes the
//!   handle and kills every process in the job.
//! - Linux: each child asks the kernel for SIGKILL when its parent dies
//!   (`PR_SET_PDEATHSIG`). The kernel ties that to the thread that spawned
//!   the child, so all guarded children are spawned from one thread that
//!   lives as long as Scribe.
//! - macOS has no such kernel feature. The children Scribe keeps running
//!   there (the Parakeet sidecar) watch their stdin and their parent pid and
//!   exit when either goes away; whisper-cli runs only for one dictation.
//!
//! On top of that, the resident processes are written to a small registry
//! file. At the next start Scribe stops any that are still running, after
//! checking pid, executable and start time (never the name alone), and
//! clears the file.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{Mutex, OnceLock},
};

use crate::process_info::{self, ProcessIdentity};

/// Starts `command` so that the child dies with Scribe (see the module docs).
pub fn spawn(command: Command) -> io::Result<Child> {
    imp::spawn(command)
}

/// Starts `command` with [`spawn`] and waits for it to finish, collecting
/// stdout and stderr like `Command::output`.
pub fn output(mut command: Command) -> io::Result<std::process::Output> {
    use std::process::Stdio;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn(command)?.wait_with_output()
}

#[cfg(windows)]
mod imp {
    use std::{
        io,
        os::windows::io::AsRawHandle,
        process::{Child, Command},
        sync::OnceLock,
    };

    use windows::Win32::{
        Foundation::HANDLE,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    /// The job's handle, as an integer so it can sit in a static. Created
    /// once and never closed: closing it is exactly what kills the children,
    /// and that must happen only when Scribe's process ends.
    fn job() -> io::Result<HANDLE> {
        static JOB: OnceLock<Result<isize, String>> = OnceLock::new();
        let job = JOB.get_or_init(|| unsafe {
            let job = CreateJobObjectW(None, None).map_err(|error| error.to_string())?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .map_err(|error| error.to_string())?;
            Ok(job.0 as isize)
        });
        match job {
            Ok(raw) => Ok(HANDLE(*raw as *mut core::ffi::c_void)),
            Err(error) => Err(io::Error::other(format!(
                "Could not create the job object. {error}"
            ))),
        }
    }

    pub fn spawn(mut command: Command) -> io::Result<Child> {
        let job = job()?;
        let mut child = command.spawn()?;
        let assigned = unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) };
        if let Err(error) = assigned {
            // A child outside the job could outlive Scribe; don't keep it.
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other(format!(
                "Could not tie the process to Scribe. {error}"
            )));
        }
        Ok(child)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::{
        io,
        os::unix::process::CommandExt,
        process::{Child, Command},
        sync::{mpsc, Mutex, OnceLock},
    };

    type Job = (Command, mpsc::Sender<io::Result<Child>>);

    /// The long-lived thread every guarded child is spawned from. The parent
    /// death signal fires when the spawning *thread* exits, so spawning from
    /// a short-lived worker thread would kill the child too early.
    fn spawner() -> io::Result<mpsc::Sender<Job>> {
        static SPAWNER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
        if let Some(sender) = SPAWNER.get() {
            return Ok(sender.lock().unwrap_or_else(|p| p.into_inner()).clone());
        }
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("speech-process-spawner".into())
            .spawn(move || {
                for (mut command, reply) in receiver {
                    let _ = reply.send(command.spawn());
                }
            })?;
        let stored = SPAWNER.get_or_init(|| Mutex::new(sender));
        Ok(stored.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }

    pub fn spawn(mut command: Command) -> io::Result<Child> {
        let parent = unsafe { libc::getpid() };
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(io::Error::last_os_error());
                }
                // Scribe may have died between fork and prctl.
                if libc::getppid() != parent {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
        let (reply, result) = mpsc::channel();
        spawner()?
            .send((command, reply))
            .map_err(|_| io::Error::other("The process spawner thread stopped."))?;
        result
            .recv()
            .map_err(|_| io::Error::other("The process spawner thread stopped."))?
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use std::{
        io,
        process::{Child, Command},
    };

    pub fn spawn(mut command: Command) -> io::Result<Child> {
        command.spawn()
    }
}

/// The resident speech processes this Scribe started, kept on disk so the
/// next start can stop any that survived (see the module docs).
pub struct Registry {
    path: PathBuf,
    entries: Mutex<Vec<ProcessIdentity>>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Sets up the registry at `path` and stops the processes a previous Scribe
/// left behind. Returns how many were stopped. Runs once, at startup.
pub fn init(path: PathBuf) -> usize {
    let stopped = reap_stale(&path);
    let _ = REGISTRY.set(Registry {
        path,
        entries: Mutex::new(Vec::new()),
    });
    stopped
}

/// Notes a resident child so a later Scribe can stop it if it survives.
pub fn track(pid: u32) {
    let Some(registry) = REGISTRY.get() else {
        return;
    };
    let Some(identity) = process_info::identity(pid) else {
        return;
    };
    let mut entries = registry.entries.lock().unwrap_or_else(|p| p.into_inner());
    entries.retain(|entry| entry.pid != pid);
    entries.push(identity);
    write_entries(&registry.path, &entries);
}

/// Forgets a resident child once Scribe has stopped it.
pub fn untrack(pid: u32) {
    let Some(registry) = REGISTRY.get() else {
        return;
    };
    let mut entries = registry.entries.lock().unwrap_or_else(|p| p.into_inner());
    entries.retain(|entry| entry.pid != pid);
    write_entries(&registry.path, &entries);
}

fn write_entries(path: &Path, entries: &[ProcessIdentity]) {
    if entries.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(entries) {
        let _ = fs::write(path, json);
    }
}

/// Stops each recorded process that is still the same process (pid,
/// executable and start time all match), then clears the file.
fn reap_stale(path: &Path) -> usize {
    let Ok(bytes) = fs::read(path) else {
        return 0;
    };
    let entries: Vec<ProcessIdentity> = serde_json::from_slice(&bytes).unwrap_or_default();
    let mut stopped = 0;
    for entry in &entries {
        if process_info::terminate_if_same(entry) {
            log::warn!(
                "Stopped a speech engine process left by an earlier Scribe (pid {}, {})",
                entry.pid,
                entry.exe.display()
            );
            stopped += 1;
        }
    }
    let _ = fs::remove_file(path);
    stopped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A command that runs for a minute and does nothing.
    fn long_sleeper() -> Command {
        if cfg!(windows) {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        }
    }

    fn wait_gone(identity: &ProcessIdentity, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if !process_info::is_same_process_running(identity) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    const MIDDLE_ENV: &str = "SCRIBE_CHILD_GUARD_MIDDLE";

    /// Run as a separate test process by `a_guarded_child_dies_with_its_parent`:
    /// starts a guarded sleeper, prints its pid, then waits to be killed.
    #[test]
    fn child_guard_middle_process() {
        if std::env::var_os(MIDDLE_ENV).is_none() {
            return;
        }
        let mut child = spawn(long_sleeper()).expect("spawn guarded sleeper");
        println!("\nGUARDED_PID={}", child.id());
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        // The parent test kills this process long before the sleeper ends.
        let _ = child.wait();
    }

    /// The real guarantee: kill the parent hard (no clean-up code runs) and
    /// the guarded child is gone within a few seconds. On macOS only the
    /// sidecar's own watch provides this, so the check is skipped there.
    #[test]
    fn a_guarded_child_dies_with_its_parent() {
        if cfg!(not(any(windows, target_os = "linux"))) {
            return;
        }
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;

        let mut middle = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "child_guard::tests::child_guard_middle_process",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(MIDDLE_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the middle process");

        let stdout = middle.stdout.take().unwrap();
        let mut pid = None;
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            // libtest prints the test name on the same line.
            if let Some((_, value)) = line.split_once("GUARDED_PID=") {
                pid = Some(value.trim().parse::<u32>().unwrap());
                break;
            }
        }
        let pid = pid.expect("the middle process reported its child");
        let identity = process_info::identity(pid).expect("the guarded child is running");

        // TerminateProcess / SIGKILL: no destructor or exit handler runs.
        middle.kill().unwrap();
        middle.wait().unwrap();

        assert!(
            wait_gone(&identity, Duration::from_secs(10)),
            "the guarded child outlived its parent"
        );
    }

    #[test]
    fn output_collects_stdout() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "echo hello"]);
            command
        } else {
            let mut command = Command::new("echo");
            command.arg("hello");
            command
        };
        command.env("SCRIBE_TEST", "1");
        let out = output(command).unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("hello"));
    }

    #[test]
    fn reaps_a_recorded_survivor_and_ignores_a_reused_pid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("speech-processes.json");

        // Spawned the way Scribe spawns speech engines. A plain
        // `Command::spawn` can return before the child has exec'd on Linux
        // while other test threads fork, so its identity would still name the
        // test binary and never match the sleeper again.
        let mut survivor = spawn(long_sleeper()).unwrap();
        // Wait until the OS reports the process, then record it.
        let identity = {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(identity) = process_info::identity(survivor.id()) {
                    break identity;
                }
                assert!(Instant::now() < deadline, "sleeper never showed up");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        // A second entry with the right pid but another start time stands
        // for a pid the OS has given to some other program: never stopped.
        let mut bystander = spawn(long_sleeper()).unwrap();
        let mut bystander_identity = process_info::identity(bystander.id()).unwrap();
        bystander_identity.start_time += 1;

        write_entries(&path, &[identity.clone(), bystander_identity.clone()]);
        assert_eq!(reap_stale(&path), 1);
        assert!(wait_gone(&identity, Duration::from_secs(5)));
        assert!(!path.exists(), "the registry is cleared after the sweep");

        bystander_identity.start_time -= 1;
        assert!(process_info::is_same_process_running(&bystander_identity));
        let _ = bystander.kill();
        let _ = bystander.wait();
        let _ = survivor.wait();
    }
}
