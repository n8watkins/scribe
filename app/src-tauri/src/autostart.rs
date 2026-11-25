//! Start Scribe when the user signs in.
//!
//! The entry itself is written and removed by the Tauri autostart plugin (on
//! Windows a `Scribe` value under `HKCU\...\CurrentVersion\Run`, which is what
//! Settings > Apps > Startup and Task Manager's Startup tab list). The entry
//! starts Scribe with [`SIGN_IN_FLAG`], so it opens to the tray instead of
//! showing its window.
//!
//! Windows lets the user switch the entry off and on in its own Startup apps
//! list without removing it. Scribe's "Launch at startup" setting follows that
//! switch: [`reconcile`] reads the entry at launch and whenever the window is
//! focused, and turns the setting off when Windows has the entry off (it never
//! switches it back on behind the user's back) or on when Windows has it on.
//! It also repairs an entry that no longer starts anything, such as one left
//! by an older install that pointed at `app.exe`.

use tauri::{plugin::TauriPlugin, AppHandle, Runtime};

use crate::error::CommandError;

/// The argument the sign-in entry starts Scribe with. A launch carrying it
/// keeps the main window hidden; Scribe waits in the tray.
pub const SIGN_IN_FLAG: &str = "--minimized";

/// The autostart plugin, registering Scribe with [`SIGN_IN_FLAG`].
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_autostart::Builder::new()
        .arg(SIGN_IN_FLAG)
        .build()
}

/// Whether this launch came from the sign-in entry (`args` with the program
/// name first, as `std::env::args` gives them).
pub fn launched_at_sign_in<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().skip(1).any(|arg| arg.as_ref() == SIGN_IN_FLAG)
}

/// What the operating system has for Scribe's sign-in entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    /// There is no entry.
    Missing,
    /// The entry starts this copy of Scribe the way it should.
    /// `on` is false when the user switched it off in Windows.
    Current { on: bool },
    /// The entry starts this copy of Scribe but with old arguments (it would
    /// open the window at sign-in). `on` as for `Current`.
    Outdated { on: bool },
    /// The entry starts another copy of Scribe that exists on disk.
    OtherCopy,
    /// The entry points at a program that no longer exists (an older
    /// install's path), so it starts nothing and Windows does not list it.
    Broken,
}

/// What [`reconcile`] does about a [`Registration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plan {
    /// `Some(true)` writes the entry for this copy (and switches it on in
    /// Windows); `Some(false)` removes it.
    pub entry: Option<bool>,
    /// The value "Launch at startup" should take to match the entry.
    pub setting: Option<bool>,
}

/// Decides how to bring the entry and the setting (`wanted`) into step.
/// Windows' own switch wins over the stored setting, because it is the last
/// thing the user changed when they differ; a missing, broken or outdated
/// entry is rewritten from the setting.
pub fn plan(wanted: bool, registration: Registration) -> Plan {
    use Registration::*;
    let mirror = |on: bool| (on != wanted).then_some(on);
    match registration {
        Missing => Plan {
            entry: wanted.then_some(true),
            setting: None,
        },
        Current { on } => Plan {
            entry: None,
            setting: mirror(on),
        },
        // Rewriting keeps Windows' switch on, so only rewrite an entry that
        // is on; one the user switched off stays off until they switch it on.
        Outdated { on } => Plan {
            entry: on.then_some(true),
            setting: mirror(on),
        },
        // Another copy of Scribe owns the entry: take it over only when this
        // copy should start, never remove it from here.
        OtherCopy => Plan {
            entry: wanted.then_some(true),
            setting: None,
        },
        Broken => Plan {
            entry: Some(wanted),
            setting: None,
        },
    }
}

/// Writes or removes the entry for the setting the user just chose. Turning
/// it off leaves an entry the user already switched off in Windows in place,
/// so Scribe stays in Windows' Startup apps list to be switched back on there.
pub fn apply(app: &AppHandle, enabled: bool) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;

    let autolaunch = app.autolaunch();
    let result = if enabled {
        autolaunch.enable()
    } else {
        match read_registration(app) {
            Ok(Registration::Current { on: false } | Registration::Outdated { on: false }) => {
                Ok(())
            }
            _ => autolaunch.disable(),
        }
    };

    result.map_err(|error| {
        CommandError::new(
            "autostart_failed",
            format!(
                "Could not {} launch at startup. {}",
                if enabled { "enable" } else { "disable" },
                error
            ),
        )
    })
}

/// Reads the entry, fixes it when it is missing or broken, and returns the
/// value "Launch at startup" should take when it differs from `wanted`.
/// Development builds (`tauri dev`) leave the entry alone, so they never point
/// it at a build folder.
pub fn reconcile(app: &AppHandle, wanted: bool) -> Option<bool> {
    if cfg!(debug_assertions) {
        return None;
    }
    let registration = match read_registration(app) {
        Ok(registration) => registration,
        Err(error) => {
            log::warn!("Could not read the launch-at-startup entry. {}", error);
            return None;
        }
    };
    let plan = plan(wanted, registration);
    if plan != Plan::default() {
        log::info!(
            "Launch at startup: entry {:?}, setting {}; {:?}",
            registration,
            wanted,
            plan
        );
    }
    if let Some(enabled) = plan.entry {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = app.autolaunch();
        let result = if enabled {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        if let Err(error) = result {
            log::warn!("Could not update the launch-at-startup entry. {}", error);
            return None;
        }
    }
    plan.setting
}

#[cfg(windows)]
fn read_registration(app: &AppHandle) -> Result<Registration, String> {
    use windows_registry::CURRENT_USER;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    // The plugin names the entry after the product ("Scribe", "Scribe Dev").
    let name = &app.package_info().name;
    let command = match CURRENT_USER.open(RUN).and_then(|key| key.get_string(name)) {
        Ok(command) => command,
        Err(_) => return Ok(Registration::Missing),
    };
    let approved = CURRENT_USER
        .open(APPROVED)
        .and_then(|key| key.get_value(name))
        .ok()
        .map(|value| value.to_vec());
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(classify(
        &command,
        approved.as_deref(),
        &exe.display().to_string(),
        |path| std::path::Path::new(path).is_file(),
    ))
}

#[cfg(not(windows))]
fn read_registration(app: &AppHandle) -> Result<Registration, String> {
    use tauri_plugin_autostart::ManagerExt;

    match app.autolaunch().is_enabled() {
        Ok(true) => Ok(Registration::Current { on: true }),
        Ok(false) => Ok(Registration::Missing),
        Err(error) => Err(error.to_string()),
    }
}

/// Classifies a Windows `Run` entry: `command` is its value, `approved` the
/// matching `StartupApproved\Run` value (absent when the user never switched
/// it in Windows), `exe` this copy's path, as the plugin writes it.
pub fn classify(
    command: &str,
    approved: Option<&[u8]>,
    exe: &str,
    exists: impl Fn(&str) -> bool,
) -> Registration {
    let target = program_path(command);
    if !target.eq_ignore_ascii_case(exe) {
        return if exists(&target) {
            Registration::OtherCopy
        } else {
            Registration::Broken
        };
    }
    let on = approved_is_on(approved);
    let expected = format!("{exe} {SIGN_IN_FLAG}");
    if command.trim().eq_ignore_ascii_case(&expected) {
        Registration::Current { on }
    } else {
        Registration::Outdated { on }
    }
}

/// The program a `Run` command starts: the quoted path, or else everything up
/// to and including the first `.exe`.
fn program_path(command: &str) -> String {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        return rest.split('"').next().unwrap_or_default().to_string();
    }
    match command.to_ascii_lowercase().find(".exe") {
        Some(end) => command[..end + 4].to_string(),
        None => command.split(' ').next().unwrap_or_default().to_string(),
    }
}

/// Windows' Startup apps switch: the first byte of the `StartupApproved`
/// value is even when the entry is on (02) and odd when it is off (03). No
/// value means the user never switched it, so it is on.
fn approved_is_on(approved: Option<&[u8]>) -> bool {
    match approved.and_then(|bytes| bytes.first()) {
        Some(first) => first % 2 == 0,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\me\AppData\Local\Scribe\Scribe.exe";
    const ON: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    const OFF: [u8; 12] = [3, 0, 0, 0, 177, 151, 109, 76, 252, 124, 221, 1];

    fn exists(path: &str) -> bool {
        path.eq_ignore_ascii_case(EXE) || path == r"D:\Portable\Scribe\Scribe.exe"
    }

    #[test]
    fn the_sign_in_flag_is_recognised_only_as_an_argument() {
        assert!(launched_at_sign_in(&["Scribe.exe", "--minimized"]));
        assert!(!launched_at_sign_in(&["Scribe.exe"]));
        assert!(!launched_at_sign_in(&["--minimized"]));
        assert!(!launched_at_sign_in(&["Scribe.exe", "toggle"]));
    }

    #[test]
    fn the_sign_in_flag_is_not_a_command_line_command() {
        assert_eq!(
            crate::remote_command::RemoteCommand::parse(&["Scribe.exe", SIGN_IN_FLAG]),
            None
        );
    }

    #[test]
    fn an_entry_for_this_copy_with_the_flag_is_current() {
        let command = format!("{EXE} --minimized");
        assert_eq!(
            classify(&command, None, EXE, exists),
            Registration::Current { on: true }
        );
        assert_eq!(
            classify(&command, Some(&ON), EXE, exists),
            Registration::Current { on: true }
        );
        assert_eq!(
            classify(&command, Some(&OFF), EXE, exists),
            Registration::Current { on: false }
        );
    }

    #[test]
    fn paths_compare_without_case() {
        let command = format!("{} --minimized", EXE.to_lowercase());
        assert_eq!(
            classify(&command, None, EXE, exists),
            Registration::Current { on: true }
        );
    }

    #[test]
    fn an_entry_without_the_flag_is_outdated() {
        // What every release before this one wrote: the path and a space.
        let command = format!("{EXE} ");
        assert_eq!(
            classify(&command, None, EXE, exists),
            Registration::Outdated { on: true }
        );
        assert_eq!(
            classify(&command, Some(&OFF), EXE, exists),
            Registration::Outdated { on: false }
        );
    }

    #[test]
    fn an_entry_for_a_missing_program_is_broken_whatever_windows_says() {
        // The owner's PC on 2026-11-25: the entry still named app.exe, which
        // the v0.8.0 installer replaced with Scribe.exe.
        let command = r"C:\Users\me\AppData\Local\Scribe\app.exe ";
        assert_eq!(
            classify(command, Some(&OFF), EXE, exists),
            Registration::Broken
        );
        assert_eq!(classify(command, None, EXE, exists), Registration::Broken);
    }

    #[test]
    fn an_entry_for_another_installed_copy_is_left_to_it() {
        let command = r#""D:\Portable\Scribe\Scribe.exe" --minimized"#;
        assert_eq!(
            classify(command, None, EXE, exists),
            Registration::OtherCopy
        );
    }

    #[test]
    fn quoted_paths_with_spaces_are_read_whole() {
        assert_eq!(
            program_path(r#""C:\Program Files\Scribe Dev\Scribe Dev.exe" --minimized"#),
            r"C:\Program Files\Scribe Dev\Scribe Dev.exe"
        );
        assert_eq!(
            program_path(r"C:\Program Files\Scribe Dev\Scribe Dev.exe --minimized"),
            r"C:\Program Files\Scribe Dev\Scribe Dev.exe"
        );
    }

    #[test]
    fn a_missing_entry_is_written_only_when_wanted() {
        assert_eq!(
            plan(true, Registration::Missing),
            Plan {
                entry: Some(true),
                setting: None
            }
        );
        assert_eq!(plan(false, Registration::Missing), Plan::default());
    }

    #[test]
    fn switching_scribe_off_in_windows_turns_the_setting_off_and_keeps_the_entry() {
        assert_eq!(
            plan(true, Registration::Current { on: false }),
            Plan {
                entry: None,
                setting: Some(false)
            }
        );
        assert_eq!(
            plan(true, Registration::Outdated { on: false }),
            Plan {
                entry: None,
                setting: Some(false)
            }
        );
        assert_eq!(
            plan(false, Registration::Current { on: false }),
            Plan::default()
        );
    }

    #[test]
    fn switching_scribe_on_in_windows_turns_the_setting_on() {
        assert_eq!(
            plan(false, Registration::Current { on: true }),
            Plan {
                entry: None,
                setting: Some(true)
            }
        );
        assert_eq!(
            plan(true, Registration::Current { on: true }),
            Plan::default()
        );
    }

    #[test]
    fn an_outdated_entry_that_is_on_is_rewritten_with_the_flag() {
        assert_eq!(
            plan(true, Registration::Outdated { on: true }),
            Plan {
                entry: Some(true),
                setting: None
            }
        );
    }

    #[test]
    fn a_broken_entry_is_repaired_or_removed() {
        assert_eq!(
            plan(true, Registration::Broken),
            Plan {
                entry: Some(true),
                setting: None
            }
        );
        assert_eq!(
            plan(false, Registration::Broken),
            Plan {
                entry: Some(false),
                setting: None
            }
        );
    }

    #[test]
    fn another_copys_entry_is_never_removed_from_here() {
        assert_eq!(plan(false, Registration::OtherCopy), Plan::default());
        assert_eq!(
            plan(true, Registration::OtherCopy),
            Plan {
                entry: Some(true),
                setting: None
            }
        );
    }
}
