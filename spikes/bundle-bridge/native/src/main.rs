//! Stand-in Scribe.exe for the bundle bridge spike.
//!
//! On launch it appends one line to the marker log (which exe ran, with which
//! arguments, and whether it would start hidden in the tray), then stays
//! resident like the real tray app until it is stopped, or for ten minutes at
//! most so a forgotten copy cannot outlive a CI job.

#![windows_subsystem = "windows"]

use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The flag the Windows sign-in entry (HKCU Run value `Scribe`) passes.
const SIGN_IN_FLAG: &str = "--minimized";

/// A fixed path, so the marker is found whichever user token or environment
/// the installer relaunched the app with.
const MARKER_LOG: &str = r"C:\ScribeBridgeSpike\launches.log";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let minimized = args.iter().any(|arg| arg == SIGN_IN_FLAG);
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();

    let line = format!(
        "time={now}\tpid={}\tversion={}\tminimized={minimized}\texe={exe}\targs={}\n",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        args.join(" ")
    );
    if let Some(dir) = std::path::Path::new(MARKER_LOG).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(MARKER_LOG)
    {
        let _ = file.write_all(line.as_bytes());
    }

    // `--minimized` means "start in the tray": the stub has no window either
    // way; the log line above records which start it was.
    std::thread::sleep(Duration::from_secs(600));
}
