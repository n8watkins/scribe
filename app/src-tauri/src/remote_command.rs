//! Control commands from the command line, for desktops whose global
//! hotkeys Scribe can't register itself (Wayland without the GlobalShortcuts
//! portal). The user binds `Scribe toggle` (or the AppImage path plus
//! `toggle`) in the desktop's keyboard settings; that launch finds the
//! running Scribe through the single-instance plugin, hands it the
//! arguments and exits, and the running Scribe acts as if its own hotkey had
//! fired.
//!
//! `hold-start` and `hold-stop` drive hold to talk from desktops that can
//! run a command on key release too (sway and Hyprland `bindr`).

use tauri::AppHandle;

use crate::hotkeys::{self, HotkeyAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteCommand {
    Toggle,
    HoldStart,
    HoldStop,
    PasteLast,
    Cancel,
    Dashboard,
}

impl RemoteCommand {
    /// The command in `args` (the program name first, as `std::env::args`
    /// gives them), if there is one. Accepts `toggle` and `--toggle`.
    pub fn parse<S: AsRef<str>>(args: &[S]) -> Option<Self> {
        let command = args.get(1)?.as_ref().trim().trim_start_matches("--");
        match command {
            "toggle" => Some(Self::Toggle),
            "hold-start" => Some(Self::HoldStart),
            "hold-stop" => Some(Self::HoldStop),
            "paste-last" => Some(Self::PasteLast),
            "cancel" => Some(Self::Cancel),
            "dashboard" | "show" => Some(Self::Dashboard),
            _ => None,
        }
    }

    pub fn run(self, app: &AppHandle) {
        log::info!("Running {self:?} from the command line");
        match self {
            Self::Toggle => hotkeys::run_action(app, HotkeyAction::ToggleDictation),
            Self::HoldStart => hotkeys::handle_press_edge(app, HotkeyAction::HoldToTalk),
            Self::HoldStop => hotkeys::handle_release_edge(app, HotkeyAction::HoldToTalk),
            Self::PasteLast => hotkeys::run_action(app, HotkeyAction::PasteLastTranscript),
            Self::Cancel => hotkeys::run_action(app, HotkeyAction::DiscardDictation),
            Self::Dashboard => {
                if let Err(error) = crate::tray::open_dashboard(app, None) {
                    log::warn!("Could not open the dashboard: {}", error.message);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_command_after_the_program_name() {
        assert_eq!(
            RemoteCommand::parse(&["Scribe", "toggle"]),
            Some(RemoteCommand::Toggle)
        );
        assert_eq!(
            RemoteCommand::parse(&["/usr/bin/Scribe", "--toggle"]),
            Some(RemoteCommand::Toggle)
        );
        assert_eq!(
            RemoteCommand::parse(&["Scribe", "hold-start"]),
            Some(RemoteCommand::HoldStart)
        );
        assert_eq!(
            RemoteCommand::parse(&["Scribe", "hold-stop"]),
            Some(RemoteCommand::HoldStop)
        );
        assert_eq!(
            RemoteCommand::parse(&["Scribe", "paste-last"]),
            Some(RemoteCommand::PasteLast)
        );
        assert_eq!(
            RemoteCommand::parse(&["Scribe", "cancel"]),
            Some(RemoteCommand::Cancel)
        );
        assert_eq!(
            RemoteCommand::parse(&["Scribe", "show"]),
            Some(RemoteCommand::Dashboard)
        );
    }

    #[test]
    fn plain_launches_and_other_flags_are_not_commands() {
        assert_eq!(RemoteCommand::parse(&["Scribe"]), None);
        assert_eq!(RemoteCommand::parse(&["Scribe", "--autostart"]), None);
        assert_eq!(
            RemoteCommand::parse(&["Scribe", "--self-test-transcribe", "a.wav"]),
            None
        );
        assert_eq!(RemoteCommand::parse::<&str>(&[]), None);
    }
}
