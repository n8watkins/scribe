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

use tauri::{AppHandle, Manager};

use crate::{
    core::Core,
    hotkeys::{self, ActionTarget, HotkeyAction},
};

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

    /// Acts as the matching hotkey would. Ignored until the `Core` exists
    /// (the shell's single-instance handler already drops it before then).
    pub fn run(self, app: &AppHandle) {
        match app.try_state::<Core>() {
            Some(core) => self.run_with(core.inner()),
            None => {
                log::warn!("Ignored {self:?} from the command line: the engine is not ready yet")
            }
        }
    }

    /// [`RemoteCommand::run`] on the engine handle.
    pub fn run_with(self, core: &Core) {
        self.run_on(core);
    }

    /// [`RemoteCommand::run_with`] on what the hotkey edges act through, so
    /// tests can run it without a `Core`.
    fn run_on(self, target: &(impl ActionTarget + ?Sized)) {
        log::info!("Running {self:?} from the command line");
        match self {
            Self::Toggle => hotkeys::run_action_with(target, HotkeyAction::ToggleDictation),
            Self::HoldStart => hotkeys::handle_press_edge_with(target, HotkeyAction::HoldToTalk),
            Self::HoldStop => hotkeys::handle_release_edge_with(target, HotkeyAction::HoldToTalk),
            Self::PasteLast => hotkeys::run_action_with(target, HotkeyAction::PasteLastTranscript),
            Self::Cancel => hotkeys::run_action_with(target, HotkeyAction::DiscardDictation),
            Self::Dashboard => {
                if let Err(error) = target.host().open_dashboard(None) {
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

    /// Each command acts as its hotkey: the same `scribe:hotkey-action`
    /// event (none for hold-stop), then the same queued intent.
    #[test]
    fn each_command_acts_as_its_hotkey() {
        use crate::{dictation_worker::Intent, hotkeys::test_target::Recorder};

        let cases: [(RemoteCommand, &[&str], Intent); 5] = [
            (RemoteCommand::Toggle, &["toggle_dictation"], Intent::Toggle),
            (
                RemoteCommand::HoldStart,
                &["hold_to_talk"],
                Intent::StartHold,
            ),
            (RemoteCommand::HoldStop, &[], Intent::Stop),
            (
                RemoteCommand::PasteLast,
                &["paste_last_transcript"],
                Intent::PasteLast,
            ),
            (
                RemoteCommand::Cancel,
                &["discard_dictation"],
                Intent::Cancel,
            ),
        ];
        for (command, events, intent) in cases {
            let target = Recorder::new();
            command.run_on(&target);
            assert_eq!(target.event_names(), events, "{command:?}");
            assert_eq!(target.submitted(), [(intent, events.len())], "{command:?}");
            // No host call at all, so no window call either.
            assert!(target.host.calls().is_empty());
        }
    }

    /// `dashboard` only opens the main window, exactly once, on its default
    /// route (it never toggles): no event, no dictation work, and a missing
    /// window is only logged.
    #[test]
    fn the_dashboard_command_only_opens_the_window() {
        use crate::{host::HostCall, hotkeys::test_target::Recorder};

        let target = Recorder::new();
        RemoteCommand::Dashboard.run_on(&target);
        // Exactly one call, the open (a toggle would be `ToggleDashboard`).
        assert_eq!(target.host.calls(), [HostCall::OpenDashboard(None)]);
        assert!(target.host.events().is_empty());
        assert!(target.submitted().is_empty());
    }
}
