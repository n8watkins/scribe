//! Start Scribe when the user signs in.
//!
//! The entry itself is written and removed by the Tauri autostart plugin. On
//! Windows it is a `Scribe` value under `HKCU\...\CurrentVersion\Run`, which
//! is what Settings > Apps > Startup and Task Manager's Startup tab list, and
//! it starts Scribe with [`SIGN_IN_FLAG`] so Scribe opens to the tray.
//!
//! Windows lets the user switch the entry off and on in its own list without
//! removing it (the `StartupApproved\Run` value). Scribe follows that switch:
//!
//! - Scribe writes or switches on the entry only at launch
//!   ([`reconcile_at_launch`], and only from the installed copy) and when the
//!   user flips "Launch at startup" in Scribe ([`apply`]).
//! - It never switches on an entry that Windows has off, with one exception:
//!   an entry that points at a program that no longer exists (left by a
//!   release before v0.8.0 renamed `app.exe` to `Scribe.exe`) is rewritten
//!   when the setting is on, even when Windows has it off. Such an entry is
//!   invisible in Windows' list, so the user cannot have switched it there; on
//!   the owner's PC two such entries were switched off by a program 5 ms apart.
//! - Otherwise "Launch at startup" mirrors Windows' switch: at launch, when the
//!   window is focused and when the tray menu is about to open
//!   ([`follow_windows_switch`], which never writes the entry).
//!
//! macOS and Linux keep the behaviour they had before: no start flag, and the
//! stored setting is applied at launch.
//!
//! The entry is written, removed and read through the [`Host`]
//! (`launch_at_login`, `set_launch_at_login`); the shell registers the plugin
//! (`tauri_host::autostart_plugin` in app/src-tauri).

use crate::{
    error::CommandError,
    host::Host,
};

/// The argument the Windows sign-in entry starts Scribe with. A launch
/// carrying it keeps the main window hidden; Scribe waits in the tray.
pub const SIGN_IN_FLAG: &str = "--minimized";

/// The publishers whose `Software\<publisher>\<product>` key may record the
/// install folder: the one releases before the publisher change used (the
/// installer hooks read it as a fallback, then remove it) and the current one
/// (see [`reconcile_at_launch`]).
pub const PREVIOUS_PUBLISHER: &str = "natkins";
pub const PUBLISHER: &str = "n8builds.dev";

/// Whether this launch came from the sign-in entry (`args` with the program
/// name first, as `std::env::args` gives them).
pub fn launched_at_sign_in<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().skip(1).any(|arg| arg.as_ref() == SIGN_IN_FLAG)
}

/// What the `Run` value holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// There is no entry.
    Missing,
    /// The entry starts this copy of Scribe the way it should.
    Current,
    /// The entry starts this copy of Scribe but without [`SIGN_IN_FLAG`] (what
    /// releases before this one wrote), so it would open the window.
    Outdated,
    /// The entry starts another copy of Scribe that exists on disk.
    OtherCopy,
    /// The entry points at a program that no longer exists.
    Broken,
}

/// The entry and Windows' switch for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registration {
    pub kind: Kind,
    /// Windows' Startup apps switch: false when it is off. Read even when the
    /// entry is missing, because the switch outlives the entry.
    pub on: bool,
}

/// What to do about a [`Registration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plan {
    /// `Some(true)` writes the entry for this copy (the plugin also switches
    /// it on in Windows); `Some(false)` removes it.
    pub entry: Option<bool>,
    /// The value "Launch at startup" should take to match Windows.
    pub setting: Option<bool>,
}

/// Decides how to bring the entry and the setting (`wanted`) into step.
/// `may_write` is true only at launch of the installed copy; otherwise the
/// plan only mirrors Windows' switch into the setting.
pub fn plan(wanted: bool, registration: Registration, may_write: bool) -> Plan {
    let Registration { kind, on } = registration;
    let mirror = (on != wanted).then_some(on);
    match kind {
        Kind::Current => Plan {
            entry: None,
            setting: mirror,
        },
        // Rewriting switches the entry on, so only an entry that is on gets
        // the flag added; one switched off stays as it is.
        Kind::Outdated => Plan {
            entry: (may_write && on).then_some(true),
            setting: mirror,
        },
        // A new install (setting on by default) writes its entry, unless a
        // leftover switch from an earlier install says off: then the setting
        // follows Windows instead.
        Kind::Missing if may_write && wanted => {
            if on {
                Plan {
                    entry: Some(true),
                    setting: None,
                }
            } else {
                Plan {
                    entry: None,
                    setting: Some(false),
                }
            }
        }
        Kind::Missing => Plan::default(),
        // Another copy's entry is left alone; the user takes it over by
        // switching "Launch at startup" on in this copy.
        Kind::OtherCopy => Plan::default(),
        // The documented exception: repaired even when Windows says off.
        Kind::Broken if may_write => Plan {
            entry: Some(wanted),
            setting: None,
        },
        Kind::Broken => Plan::default(),
    }
}

/// The entry change for a "Launch at startup" flip the user just made in
/// Scribe. Turning it off removes only this copy's entry, and leaves one the
/// user already switched off in Windows in place, so Scribe stays in Windows'
/// list to be switched back on there.
pub fn entry_for_toggle(enabled: bool, registration: Option<Registration>) -> Option<bool> {
    if enabled {
        return Some(true);
    }
    match registration {
        Some(Registration {
            kind: Kind::Missing | Kind::OtherCopy,
            ..
        }) => None,
        Some(Registration {
            kind: Kind::Current | Kind::Outdated,
            on: false,
        }) => None,
        _ => Some(false),
    }
}

fn dev_build_error() -> CommandError {
    CommandError::new(
        "autostart_dev_build",
        "This is a development build, so it does not change the startup entry.",
    )
}

/// Writes or removes the entry for the setting the user just chose in Scribe
/// (the settings switch or the tray menu). Development builds refuse, so they
/// never point the entry at a build folder.
pub fn apply_with(host: &dyn Host, enabled: bool) -> Result<(), CommandError> {
    if cfg!(debug_assertions) {
        return Err(dev_build_error());
    }
    let registration = read_registration(host).ok();
    match entry_for_toggle(enabled, registration) {
        Some(on) => write_entry(host, on),
        None => Ok(()),
    }
}

fn write_entry(host: &dyn Host, enabled: bool) -> Result<(), CommandError> {
    host.set_launch_at_login(enabled).map_err(|error| {
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

/// At launch: repairs or writes the entry when this is the installed copy,
/// and returns the value "Launch at startup" should take when Windows'
/// switch differs from `wanted`. Development builds do nothing.
#[cfg(windows)]
pub fn reconcile_at_launch_with(host: &dyn Host, wanted: bool) -> Option<bool> {
    if cfg!(debug_assertions) {
        return None;
    }
    let installed = installed_folder(host);
    if let Some(folder) = &installed {
        remember_install_folder(host, folder);
    }
    let registration = match read_registration(host) {
        Ok(registration) => registration,
        Err(error) => {
            log::warn!("Could not read the launch-at-startup entry. {}", error);
            return None;
        }
    };
    let plan = plan(wanted, registration, installed.is_some());
    if plan != Plan::default() {
        log::info!(
            "Launch at startup: {:?}, setting {}, installed copy {}; {:?}",
            registration,
            wanted,
            installed.is_some(),
            plan
        );
    }
    if let Some(enabled) = plan.entry {
        if let Err(error) = write_entry(host, enabled) {
            log::warn!("{}", error.message);
            return None;
        }
    }
    plan.setting
}

/// At launch on macOS and Linux, as before this change: the stored setting is
/// applied to the login entry.
#[cfg(not(windows))]
pub fn reconcile_at_launch_with(host: &dyn Host, wanted: bool) -> Option<bool> {
    if cfg!(debug_assertions) {
        return None;
    }
    match host.launch_at_login() {
        Ok(enabled) if enabled != wanted => {
            log::info!("Applying launch at startup ({wanted}) to the login entry");
            if let Err(error) = write_entry(host, wanted) {
                log::warn!("{}", error.message);
            }
        }
        Ok(_) => {}
        Err(error) => log::warn!("Could not read launch-at-startup state. {}", error),
    }
    None
}

/// The value "Launch at startup" should take to follow Windows' switch, when
/// it differs from `wanted`. Never writes the entry. Always `None` on macOS
/// and Linux and in development builds.
pub fn follow_windows_switch_with(host: &dyn Host, wanted: bool) -> Option<bool> {
    if cfg!(debug_assertions) || !cfg!(windows) {
        return None;
    }
    let registration = read_registration(host).ok()?;
    plan(wanted, registration, false).setting
}

#[cfg(windows)]
mod windows_entry {
    use super::*;
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

    pub const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    pub const APPROVED: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    /// Reads the `Run` value called `name` and Windows' switch for it.
    pub fn read(name: &str, exe: &str) -> Registration {
        let approved = CURRENT_USER
            .open(APPROVED)
            .and_then(|key| key.get_value(name))
            .ok()
            .map(|value| value.to_vec());
        let command = CURRENT_USER
            .open(RUN)
            .and_then(|key| key.get_string(name))
            .ok();
        classify(command.as_deref(), approved.as_deref(), exe, |path| {
            std::path::Path::new(path).is_file()
        })
    }

    /// Every folder an installer recorded for `product`.
    pub fn recorded_install_folders(product: &str) -> Vec<String> {
        let mut folders = Vec::new();
        let uninstall = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{product}");
        for root in [CURRENT_USER, LOCAL_MACHINE] {
            if let Ok(folder) = root
                .open(&uninstall)
                .and_then(|key| key.get_string("InstallLocation"))
            {
                folders.push(folder);
            }
        }
        // NSIS keeps the folder as the key's default value, the MSI as
        // InstallDir.
        for publisher in [PREVIOUS_PUBLISHER, PUBLISHER] {
            if let Ok(key) = CURRENT_USER.open(format!(r"Software\{publisher}\{product}")) {
                for value in ["", "InstallDir"] {
                    if let Ok(folder) = key.get_string(value) {
                        folders.push(folder);
                    }
                }
            }
        }
        folders
    }

    /// Records `folder` where the installer (publisher
    /// [`PUBLISHER`]) looks for the previous install, so an update
    /// lands in the same folder.
    pub fn remember_install_folder(product: &str, folder: &str) -> windows_registry::Result<()> {
        let path = format!(r"Software\{PUBLISHER}\{product}");
        let recorded = CURRENT_USER
            .open(&path)
            .and_then(|key| key.get_string(""))
            .ok();
        if recorded.is_some_and(|recorded| same_folder(&recorded, folder)) {
            return Ok(());
        }
        CURRENT_USER.create(&path)?.set_string("", folder)
    }
}

#[cfg(windows)]
fn read_registration(host: &dyn Host) -> Result<Registration, String> {
    // The plugin names the entry after the product ("Scribe", "Scribe Dev").
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(windows_entry::read(
        host.flavor().product_name(),
        &exe.display().to_string(),
    ))
}

#[cfg(not(windows))]
fn read_registration(host: &dyn Host) -> Result<Registration, String> {
    match host.launch_at_login() {
        Ok(true) => Ok(Registration {
            kind: Kind::Current,
            on: true,
        }),
        Ok(false) => Ok(Registration {
            kind: Kind::Missing,
            on: true,
        }),
        Err(error) => Err(error.to_string()),
    }
}

/// This copy's folder, when an installer recorded it (not a build folder or
/// a copied exe).
#[cfg(windows)]
fn installed_folder(host: &dyn Host) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?.display().to_string();
    windows_entry::recorded_install_folders(host.flavor().product_name())
        .iter()
        .any(|recorded| same_folder(recorded, &folder))
        .then_some(folder)
}

#[cfg(windows)]
fn remember_install_folder(host: &dyn Host, folder: &str) {
    let product = host.flavor().product_name();
    if let Err(error) = windows_entry::remember_install_folder(product, folder) {
        log::warn!("Could not record the install folder for updates. {}", error);
    }
}

/// Whether two folder paths name the same folder, ignoring case, quotes and a
/// trailing separator (installers record them differently).
pub fn same_folder(a: &str, b: &str) -> bool {
    fn normalize(path: &str) -> String {
        path.trim()
            .trim_matches('"')
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    }
    normalize(a) == normalize(b)
}

/// Classifies a Windows `Run` entry: `command` is its value (`None` when
/// there is none), `approved` the matching `StartupApproved\Run` value
/// (absent when the user never switched it in Windows), `exe` this copy's
/// path as the plugin writes it.
pub fn classify(
    command: Option<&str>,
    approved: Option<&[u8]>,
    exe: &str,
    exists: impl Fn(&str) -> bool,
) -> Registration {
    let on = approved_is_on(approved);
    let Some(command) = command else {
        return Registration {
            kind: Kind::Missing,
            on,
        };
    };
    let kind = match arguments_for(command, exe) {
        Some(arguments) if arguments.eq_ignore_ascii_case(SIGN_IN_FLAG) => Kind::Current,
        Some(_) => Kind::Outdated,
        None if exists(&program_path(command)) => Kind::OtherCopy,
        None => Kind::Broken,
    };
    Registration { kind, on }
}

/// The arguments of `command` when it starts `exe` (quoted or not, any
/// case), else `None`.
fn arguments_for(command: &str, exe: &str) -> Option<String> {
    let command = command.trim();
    let (quoted, body) = match command.strip_prefix('"') {
        Some(rest) => (true, rest),
        None => (false, command),
    };
    let head = body.get(..exe.len())?;
    if !head.eq_ignore_ascii_case(exe) {
        return None;
    }
    let mut rest = &body[exe.len()..];
    if quoted {
        rest = rest.strip_prefix('"')?;
    }
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim().to_string())
}

/// The program a `Run` command for another program starts: the quoted path,
/// or else everything up to and including the first `.exe`.
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
    const OTHER: &str = r"D:\Portable\Scribe\Scribe.exe";
    const ON: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    const OFF: [u8; 12] = [3, 0, 0, 0, 177, 151, 109, 76, 252, 124, 221, 1];

    fn exists(path: &str) -> bool {
        path.eq_ignore_ascii_case(EXE) || path == OTHER
    }

    fn reg(kind: Kind, on: bool) -> Registration {
        Registration { kind, on }
    }

    fn classified(command: Option<&str>, approved: Option<&[u8]>, exe: &str) -> Registration {
        classify(command, approved, exe, exists)
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
    fn entries_are_classified() {
        let current = format!("{EXE} --minimized");
        let quoted = format!("\"{EXE}\" --minimized");
        let lower = current.to_lowercase();
        // What every release before this one wrote: the path and a space.
        let outdated = format!("{EXE} ");
        let other = format!("\"{OTHER}\" --minimized");
        // The owner's PC on 2026-11-25: the entry still named app.exe.
        let broken = r"C:\Users\me\AppData\Local\Scribe\app.exe ";
        // A folder name containing ".exe" is still this copy.
        let odd_exe = r"C:\Users\me.exe.user\Scribe\Scribe.exe";
        let odd = format!("{odd_exe} --minimized");
        // (Run value, StartupApproved value, this exe, expected)
        type Case<'a> = (Option<&'a str>, Option<&'a [u8]>, &'a str, Registration);
        let cases: &[Case] = &[
            (Some(&current), None, EXE, reg(Kind::Current, true)),
            (Some(&current), Some(&ON), EXE, reg(Kind::Current, true)),
            (Some(&current), Some(&OFF), EXE, reg(Kind::Current, false)),
            (Some(&quoted), None, EXE, reg(Kind::Current, true)),
            (Some(&lower), None, EXE, reg(Kind::Current, true)),
            (Some(&outdated), None, EXE, reg(Kind::Outdated, true)),
            (Some(&outdated), Some(&OFF), EXE, reg(Kind::Outdated, false)),
            (Some(&other), None, EXE, reg(Kind::OtherCopy, true)),
            (Some(&other), Some(&OFF), EXE, reg(Kind::OtherCopy, false)),
            (Some(broken), Some(&OFF), EXE, reg(Kind::Broken, false)),
            (Some(broken), None, EXE, reg(Kind::Broken, true)),
            (Some(&odd), None, odd_exe, reg(Kind::Current, true)),
            (None, None, EXE, reg(Kind::Missing, true)),
            // A reinstall finds the switch the user left off.
            (None, Some(&OFF), EXE, reg(Kind::Missing, false)),
            // A longer name that starts with this exe's path is not this copy
            // (and, as another copy's entry, is never written or removed).
            (
                Some(r"C:\Users\me\AppData\Local\Scribe\Scribe.exe2 --minimized"),
                None,
                EXE,
                reg(Kind::OtherCopy, true),
            ),
        ];
        for (command, approved, exe, expected) in cases {
            assert_eq!(
                classified(*command, *approved, exe),
                *expected,
                "{command:?} {approved:?}"
            );
        }
    }

    #[test]
    fn plans() {
        let write = |on| Plan {
            entry: Some(on),
            setting: None,
        };
        let setting = |on| Plan {
            entry: None,
            setting: Some(on),
        };
        let nothing = Plan::default();
        // (wanted, registration, may_write, expected)
        let cases = [
            // A new install writes its entry; a leftover "off" wins instead.
            (true, reg(Kind::Missing, true), true, write(true)),
            (true, reg(Kind::Missing, false), true, setting(false)),
            (false, reg(Kind::Missing, true), true, nothing),
            (true, reg(Kind::Missing, true), false, nothing),
            // Windows' switch is mirrored, never overridden.
            (true, reg(Kind::Current, true), true, nothing),
            (true, reg(Kind::Current, false), true, setting(false)),
            (false, reg(Kind::Current, true), true, setting(true)),
            (false, reg(Kind::Current, false), false, nothing),
            (true, reg(Kind::Current, false), false, setting(false)),
            // An outdated entry that is on gets the flag, at launch only.
            (
                true,
                reg(Kind::Outdated, true),
                true,
                Plan {
                    entry: Some(true),
                    setting: None,
                },
            ),
            (
                false,
                reg(Kind::Outdated, true),
                true,
                Plan {
                    entry: Some(true),
                    setting: Some(true),
                },
            ),
            (true, reg(Kind::Outdated, true), false, nothing),
            (true, reg(Kind::Outdated, false), true, setting(false)),
            // Another copy's entry is never written or removed.
            (true, reg(Kind::OtherCopy, true), true, nothing),
            (true, reg(Kind::OtherCopy, false), true, nothing),
            (false, reg(Kind::OtherCopy, true), true, nothing),
            // The documented exception: a broken entry is repaired even when
            // Windows says off, and only at launch of the installed copy.
            (true, reg(Kind::Broken, false), true, write(true)),
            (true, reg(Kind::Broken, true), true, write(true)),
            (false, reg(Kind::Broken, false), true, write(false)),
            (true, reg(Kind::Broken, false), false, nothing),
        ];
        for (wanted, registration, may_write, expected) in cases {
            assert_eq!(
                plan(wanted, registration, may_write),
                expected,
                "wanted {wanted}, {registration:?}, may_write {may_write}"
            );
        }
    }

    #[test]
    fn a_toggle_in_scribe_writes_only_this_copys_entry() {
        assert_eq!(entry_for_toggle(true, None), Some(true));
        assert_eq!(
            entry_for_toggle(true, Some(reg(Kind::OtherCopy, false))),
            Some(true)
        );
        assert_eq!(
            entry_for_toggle(true, Some(reg(Kind::Current, false))),
            Some(true)
        );
        assert_eq!(
            entry_for_toggle(false, Some(reg(Kind::Current, true))),
            Some(false)
        );
        assert_eq!(
            entry_for_toggle(false, Some(reg(Kind::Broken, true))),
            Some(false)
        );
        assert_eq!(entry_for_toggle(false, None), Some(false));
        assert_eq!(
            entry_for_toggle(false, Some(reg(Kind::OtherCopy, true))),
            None
        );
        assert_eq!(
            entry_for_toggle(false, Some(reg(Kind::Missing, true))),
            None
        );
        assert_eq!(
            entry_for_toggle(false, Some(reg(Kind::Current, false))),
            None
        );
        assert_eq!(
            entry_for_toggle(false, Some(reg(Kind::Outdated, false))),
            None
        );
    }

    #[test]
    fn install_folders_compare_as_installers_record_them() {
        let folder = r"C:\Users\me\AppData\Local\Scribe";
        assert!(same_folder(r#""C:\Users\me\AppData\Local\Scribe""#, folder));
        assert!(same_folder(r"c:\users\me\appdata\local\scribe\", folder));
        assert!(!same_folder(
            r"C:\Users\me\AppData\Local\Scribe Dev",
            folder
        ));
    }

    fn recording_host() -> crate::host::RecordingHost {
        crate::host::RecordingHost::new(
            crate::paths::Paths::from_parts("/d".into(), "/c".into(), "/l".into(), None, None),
            crate::host::Flavor::Stable,
        )
    }

    #[test]
    fn the_entry_is_written_and_removed_through_the_host() {
        use crate::host::{Failing, HostCall};

        let host = recording_host();
        assert!(write_entry(&host, true).is_ok());
        assert_eq!(host.launch_at_login(), Ok(true));
        assert!(write_entry(&host, false).is_ok());
        assert_eq!(host.launch_at_login(), Ok(false));
        assert_eq!(
            host.calls(),
            [
                HostCall::SetLaunchAtLogin(true),
                HostCall::SetLaunchAtLogin(false)
            ]
        );

        host.fail(Failing::LaunchAtLogin);
        let error = write_entry(&host, true).unwrap_err();
        assert_eq!(error.code, "autostart_failed");
        assert_eq!(
            error.message,
            format!(
                "Could not enable launch at startup. {}",
                Failing::LaunchAtLogin.message()
            )
        );
        let error = write_entry(&host, false).unwrap_err();
        assert_eq!(
            error.message,
            format!(
                "Could not disable launch at startup. {}",
                Failing::LaunchAtLogin.message()
            )
        );
    }

    /// Outside Windows the host's answer is the whole registration: an entry
    /// is this copy's, and there is no switch to be off.
    #[cfg(not(windows))]
    #[test]
    fn the_registration_is_read_from_the_host() {
        let host = recording_host();
        assert_eq!(read_registration(&host), Ok(reg(Kind::Missing, true)));
        host.set_launch_at_login_state(true);
        assert_eq!(read_registration(&host), Ok(reg(Kind::Current, true)));
        host.fail(crate::host::Failing::LaunchAtLogin);
        assert_eq!(
            read_registration(&host),
            Err(crate::host::Failing::LaunchAtLogin.message())
        );
        assert!(host.calls().is_empty());
    }

    /// Development builds never touch the entry: the toggle refuses, launch
    /// and focus change nothing, and the host is never asked to write.
    #[cfg(debug_assertions)]
    #[test]
    fn a_development_build_leaves_the_entry_alone() {
        let host = recording_host();
        let error = apply_with(&host, true).unwrap_err();
        assert_eq!(error.code, "autostart_dev_build");
        assert_eq!(reconcile_at_launch_with(&host, true), None);
        assert_eq!(follow_windows_switch_with(&host, false), None);
        assert!(host.calls().is_empty());
    }

    /// Writes a throwaway entry and switch under a test name, reads them back
    /// through the real registry read, and removes them.
    #[cfg(windows)]
    #[test]
    fn the_registry_read_sees_the_entry_and_windows_switch() {
        use windows_registry::{Type, CURRENT_USER};

        let name = format!("ScribeAutostartTest-{}", std::process::id());
        let run = CURRENT_USER.create(windows_entry::RUN).unwrap();
        let approved = CURRENT_USER.create(windows_entry::APPROVED).unwrap();
        run.set_string(&name, format!("{EXE} --minimized")).unwrap();
        approved.set_bytes(&name, Type::Bytes, &OFF).unwrap();

        let both = windows_entry::read(&name, EXE);
        let _ = run.remove_value(&name);
        let switch_only = windows_entry::read(&name, EXE);
        let _ = approved.remove_value(&name);
        let neither = windows_entry::read(&name, EXE);

        assert_eq!(both, reg(Kind::Current, false));
        assert_eq!(switch_only, reg(Kind::Missing, false));
        assert_eq!(neither, reg(Kind::Missing, true));
    }
}
