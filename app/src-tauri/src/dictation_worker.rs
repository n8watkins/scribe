//! The single dictation worker: one thread that owns every dictation action.
//!
//! Hotkeys, the tray, the floating pill, and the dashboard all used to start
//! and stop dictations on whatever thread noticed the request. A hotkey bound
//! through the global-shortcut plugin, a tray click, and every synchronous
//! command ran on the main (UI) thread, so stopping a dictation froze every
//! window until Whisper (and, for Transform Selection, the local LLM) was done.
//!
//! Now every source sends its request here. The worker runs them one at a
//! time, in arrival order, on its own thread, so:
//!   - the UI thread never waits for transcription, paste, or the LLM,
//!   - start, stop, discard, and paste requests from different sources can
//!     never interleave (a hotkey stop and a pill stop cannot both transcribe),
//!   - the "may this run now?" rules live in one place ([`resolve`] and
//!     [`admits`]) and are unit tested.
//!
//! Requests come in two shapes. An [`Intent`] is what a key or menu item means
//! ("toggle", "start holding"); [`resolve`] turns it into a concrete
//! [`DictationCommand`] against the status the user saw when they pressed it,
//! so a press during Transcribing is ignored exactly as before. The worker then
//! re-checks each command against the status at the moment it runs
//! ([`admits`]), which keeps a queued command from acting on a state it was not
//! meant for.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::mpsc::{channel, Sender},
    thread,
};

use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppStatus,
    audio::{self, RecordingResultStatus, RecordingSessionInfo, StartRecordingRequest},
    commands::BackendState,
    dictation,
    error::CommandError,
    output::{self, OutputResult},
    tray,
};

/// What a hotkey, tray item, or button asks for, before it is checked against
/// the current status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Toggle hotkey: start when idle, stop when recording.
    Toggle,
    /// Hold-toggle-key + Q note chord: start a note when idle, stop when
    /// recording.
    ToggleNote,
    /// Transform Selection hotkey: capture the selection and record an
    /// instruction when idle, stop when recording.
    ToggleTransform,
    /// Hold-to-talk press: start without silence auto-stop.
    StartHold,
    /// Tray "Start Dictation": start with silence auto-stop.
    StartFromTray,
    /// Hold-to-talk release, tray "Stop Dictation", silence auto-stop.
    Stop,
    /// Discard hotkey: drop the recording without transcribing.
    Cancel,
    /// Paste Last hotkey or tray item.
    PasteLast,
}

/// A concrete dictation action, run on the worker thread.
#[derive(Debug, Clone)]
pub enum DictationCommand {
    /// Start a dictation. Admitted while already recording, where it is a
    /// no-op that reports the active session.
    Start {
        request: Option<StartRecordingRequest>,
        allow_auto_stop: bool,
    },
    /// Start a note dictation (saved as a note, never pasted).
    StartNote,
    /// Capture the selection and record a spoken Transform Selection
    /// instruction.
    StartTransform,
    /// Stop the recording, then transcribe and output it.
    Stop,
    /// Discard the recording without transcribing.
    Cancel,
    /// Paste the Last Transcript Buffer.
    PasteLast,
    /// The max-duration timer of `session_id` fired: stop and transcribe it if
    /// it is still the active recording.
    FinishTimedOut { session_id: String },
    /// The microphone of `session_id` stopped sending audio: stop and
    /// transcribe whatever it captured.
    FinishDisconnected { session_id: String },
}

/// What a command produced, for callers that wait for it.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// The command ran and has nothing to report.
    Done,
    /// The command was not admitted in the current status and did nothing.
    Skipped,
    /// A start command ran; the active recording session.
    Started(RecordingSessionInfo),
    /// A paste command ran.
    Output(OutputResult),
}

/// Turns an intent into a command against the status the user saw when they
/// pressed the key. `None` means the press does nothing in that status (for
/// example a toggle while Transcribing), which matches how hotkeys behaved
/// before the worker existed.
pub fn resolve(status: &AppStatus, intent: Intent) -> Option<DictationCommand> {
    let startable = matches!(
        status,
        AppStatus::Idle | AppStatus::Ready | AppStatus::Error
    );
    let recording = *status == AppStatus::Recording;
    let toggle = |start: DictationCommand| {
        if startable {
            Some(start)
        } else if recording {
            Some(DictationCommand::Stop)
        } else {
            None
        }
    };

    match intent {
        Intent::Toggle => toggle(DictationCommand::Start {
            request: None,
            allow_auto_stop: true,
        }),
        Intent::ToggleNote => toggle(DictationCommand::StartNote),
        Intent::ToggleTransform => toggle(DictationCommand::StartTransform),
        // Hold-to-talk never auto-stops on silence: the user is still holding
        // the key, and a pause mid-thought must not cut them off.
        Intent::StartHold => (startable || recording).then_some(DictationCommand::Start {
            request: None,
            allow_auto_stop: false,
        }),
        // Tray start behaves like the toggle hotkey (no key is held), so
        // silence auto-stop applies.
        Intent::StartFromTray => (startable || recording).then_some(DictationCommand::Start {
            request: None,
            allow_auto_stop: true,
        }),
        Intent::Stop => Some(DictationCommand::Stop),
        Intent::Cancel => Some(DictationCommand::Cancel),
        Intent::PasteLast => Some(DictationCommand::PasteLast),
    }
}

/// Whether `command` may run in `status`, checked on the worker right before
/// it runs. A command that is not admitted is skipped.
pub fn admits(status: &AppStatus, command: &DictationCommand) -> bool {
    let startable = matches!(
        status,
        AppStatus::Idle | AppStatus::Ready | AppStatus::Error
    );
    match command {
        DictationCommand::Start { .. } => startable || *status == AppStatus::Recording,
        DictationCommand::StartNote | DictationCommand::StartTransform => startable,
        DictationCommand::Stop => *status == AppStatus::Recording,
        // These check their own preconditions (an active session, a last
        // transcript) and no-op cleanly without one.
        DictationCommand::Cancel
        | DictationCommand::PasteLast
        | DictationCommand::FinishTimedOut { .. }
        | DictationCommand::FinishDisconnected { .. } => true,
    }
}

/// The result of a skipped command. A caller waiting on a start hears why
/// nothing started (the dashboard shows it); everything else is a quiet no-op,
/// like a stop that arrives after silence auto-stop already stopped.
fn skipped(status: &AppStatus, command: &DictationCommand) -> Result<Outcome, CommandError> {
    match command {
        DictationCommand::Start { .. }
        | DictationCommand::StartNote
        | DictationCommand::StartTransform => Err(CommandError::new(
            "recording_unavailable",
            format!("Cannot start recording while app is {:?}.", status),
        )),
        _ => Ok(Outcome::Skipped),
    }
}

/// The side effects the worker loop drives. Production uses [`AppExecutor`];
/// tests use a fake so the queueing and gating can run without a Tauri app.
pub(crate) trait Executor: Send + 'static {
    /// The current dictation status, read right before each command.
    fn status(&self) -> Result<AppStatus, CommandError>;
    /// Runs one admitted command.
    fn execute(&self, command: DictationCommand) -> Result<Outcome, CommandError>;
    /// Reports the error of a command nobody is waiting on.
    fn report(&self, error: CommandError);
}

type Reply = Sender<Result<Outcome, CommandError>>;

struct Job {
    command: DictationCommand,
    reply: Option<Reply>,
}

/// Handle to the worker thread, kept in Tauri's managed state.
pub struct DictationWorker {
    sender: Sender<Job>,
}

impl DictationWorker {
    /// Starts the worker for the running app.
    pub fn spawn(app: AppHandle) -> std::io::Result<Self> {
        Self::spawn_with(AppExecutor { app })
    }

    pub(crate) fn spawn_with<E: Executor>(executor: E) -> std::io::Result<Self> {
        let (sender, receiver) = channel::<Job>();
        thread::Builder::new()
            .name("scribe-dictation".to_string())
            .spawn(move || {
                // Ends when every sender is gone, which is app shutdown.
                for job in receiver {
                    run_job(&executor, job);
                }
            })?;
        Ok(Self { sender })
    }

    /// Queues a command without waiting. Its error, if any, is reported as a
    /// recording error, the same way hotkey errors always were.
    pub fn send(&self, command: DictationCommand) {
        self.enqueue(command, None);
    }

    /// Queues a command and blocks until it has run. Never call this on the
    /// main thread; async commands call it through `spawn_blocking`.
    pub fn call(&self, command: DictationCommand) -> Result<Outcome, CommandError> {
        let (reply, receiver) = channel();
        self.enqueue(command, Some(reply));
        receiver.recv().unwrap_or_else(|_| Err(worker_gone()))
    }

    fn enqueue(&self, command: DictationCommand, reply: Option<Reply>) {
        if let Err(error) = self.sender.send(Job { command, reply }) {
            // The worker thread is gone; tell a waiting caller instead of
            // leaving it blocked.
            log::error!(
                "Dictation worker is not running; dropped {:?}",
                error.0.command
            );
            if let Some(reply) = error.0.reply {
                let _ = reply.send(Err(worker_gone()));
            }
        }
    }
}

fn worker_gone() -> CommandError {
    CommandError::new(
        "dictation_worker_unavailable",
        "Scribe's dictation worker stopped. Restart Scribe to dictate again.",
    )
}

fn run_job<E: Executor>(executor: &E, job: Job) {
    let Job { command, reply } = job;
    let result = match executor.status() {
        Ok(status) if admits(&status, &command) => {
            // A panic in one dictation must not take the worker (and with it
            // every hotkey) down for the rest of the session. The panic hook
            // has already logged it.
            catch_unwind(AssertUnwindSafe(|| executor.execute(command))).unwrap_or_else(|_| {
                Err(CommandError::new(
                    "dictation_failed",
                    "Something went wrong with that dictation. Try again.",
                ))
            })
        }
        Ok(status) => {
            log::debug!("Skipped {:?} while {:?}", command, status);
            skipped(&status, &command)
        }
        Err(error) => Err(error),
    };

    match reply {
        Some(reply) => {
            let _ = reply.send(result);
        }
        None => {
            if let Err(error) = result {
                executor.report(error);
            }
        }
    }
}

/// Resolves `intent` against the current status and queues the resulting
/// command. Safe to call from any thread, including the main thread and the
/// audio callback thread: it never waits for the command to run.
pub fn submit(app: &AppHandle, intent: Intent) {
    let status = match app.state::<BackendState>().app_state() {
        Ok(machine) => machine.status().clone(),
        Err(error) => {
            audio::emit_recording_error(app, error);
            return;
        }
    };
    match resolve(&status, intent) {
        Some(command) => dispatch(app, command),
        None => log::debug!("Ignored {:?} while {:?}", intent, status),
    }
}

/// Queues a concrete command without waiting for it.
pub fn dispatch(app: &AppHandle, command: DictationCommand) {
    match app.try_state::<DictationWorker>() {
        Some(worker) => worker.send(command),
        None => log::error!("Dictation worker is not running; dropped {:?}", command),
    }
}

/// Runs `command` on the worker and waits for it without holding the calling
/// async task's thread.
pub async fn request(app: AppHandle, command: DictationCommand) -> Result<Outcome, CommandError> {
    tauri::async_runtime::spawn_blocking(move || match app.try_state::<DictationWorker>() {
        Some(worker) => worker.call(command),
        None => Err(worker_gone()),
    })
    .await
    .map_err(|_| worker_gone())?
}

/// Production side effects: the real audio, transcription, and output paths.
struct AppExecutor {
    app: AppHandle,
}

impl Executor for AppExecutor {
    fn status(&self) -> Result<AppStatus, CommandError> {
        Ok(self
            .app
            .state::<BackendState>()
            .app_state()?
            .status()
            .clone())
    }

    fn execute(&self, command: DictationCommand) -> Result<Outcome, CommandError> {
        let app = &self.app;
        let outcome = match command {
            DictationCommand::Start {
                request,
                allow_auto_stop,
            } => Outcome::Started(audio::start_recording_for_app(
                app,
                request,
                allow_auto_stop,
            )?),
            DictationCommand::StartNote => {
                let request = StartRecordingRequest {
                    is_note: true,
                    ..Default::default()
                };
                Outcome::Started(audio::start_recording_for_app(app, Some(request), true)?)
            }
            DictationCommand::StartTransform => {
                start_transform(app)?;
                Outcome::Done
            }
            DictationCommand::Stop => {
                stop_and_transcribe(app)?;
                Outcome::Done
            }
            DictationCommand::Cancel => {
                audio::cancel_recording_for_app(app)?;
                Outcome::Done
            }
            DictationCommand::PasteLast => Outcome::Output(output::paste_last_transcript(app)?),
            DictationCommand::FinishTimedOut { session_id } => {
                audio::finish_timed_out_recording(app, &session_id)?;
                Outcome::Done
            }
            DictationCommand::FinishDisconnected { session_id } => {
                // The disconnect already told the user what happened; a
                // failure to salvage the audio is only logged.
                if let Err(error) = audio::finish_disconnected_recording(app, &session_id) {
                    log::warn!(
                        "Could not recover from a mid-recording audio stream error: {}",
                        error.message
                    );
                }
                Outcome::Done
            }
        };
        refresh_tray(app);
        Ok(outcome)
    }

    fn report(&self, error: CommandError) {
        refresh_tray(&self.app);
        audio::emit_recording_error(&self.app, error);
    }
}

fn refresh_tray(app: &AppHandle) {
    if let Ok(machine) = app.state::<BackendState>().app_state() {
        let status = machine.status().clone();
        drop(machine);
        tray::update_tray_status(app, status);
    }
}

/// Stops the active recording and finishes it: a too-short tap is reported as
/// "nothing heard"; anything else is transcribed and output.
fn stop_and_transcribe(app: &AppHandle) -> Result<(), CommandError> {
    let result = audio::stop_recording_for_app(app)?;
    refresh_tray(app);

    if result.status == RecordingResultStatus::TooShort {
        // A quick tap of the toggle hotkey is benign, not an error: the state
        // machine already returned to Idle (AudioTooShort), so just tell the
        // frontend nothing was heard.
        dictation::emit_dictation_empty(app);
        return Ok(());
    }

    dictation::transcribe_recording_for_app(app, result)?;
    Ok(())
}

#[cfg(windows)]
fn start_transform(app: &AppHandle) -> Result<(), CommandError> {
    crate::hotkeys::start_transform(app)
}

/// Off Windows there is no selection capture (SendInput and the clipboard), so
/// the typed-instruction panel on the dashboard stands in for it.
#[cfg(not(windows))]
fn start_transform(app: &AppHandle) -> Result<(), CommandError> {
    tray::open_dashboard(app, Some("dashboard"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::Duration;

    const ALL_STATUSES: [AppStatus; 8] = [
        AppStatus::Idle,
        AppStatus::Recording,
        AppStatus::Stopping,
        AppStatus::Transcribing,
        AppStatus::Pasting,
        AppStatus::Ready,
        AppStatus::Error,
        AppStatus::Paused,
    ];

    fn label(command: &DictationCommand) -> &'static str {
        match command {
            DictationCommand::Start {
                allow_auto_stop: true,
                ..
            } => "start",
            DictationCommand::Start {
                allow_auto_stop: false,
                ..
            } => "start-hold",
            DictationCommand::StartNote => "start-note",
            DictationCommand::StartTransform => "start-transform",
            DictationCommand::Stop => "stop",
            DictationCommand::Cancel => "cancel",
            DictationCommand::PasteLast => "paste-last",
            DictationCommand::FinishTimedOut { .. } => "timed-out",
            DictationCommand::FinishDisconnected { .. } => "disconnected",
        }
    }

    fn resolved(status: AppStatus, intent: Intent) -> Option<&'static str> {
        resolve(&status, intent).as_ref().map(label)
    }

    #[test]
    fn toggles_start_when_startable_and_stop_when_recording() {
        for status in [AppStatus::Idle, AppStatus::Ready, AppStatus::Error] {
            assert_eq!(resolved(status.clone(), Intent::Toggle), Some("start"));
            assert_eq!(
                resolved(status.clone(), Intent::ToggleNote),
                Some("start-note")
            );
            assert_eq!(
                resolved(status, Intent::ToggleTransform),
                Some("start-transform")
            );
        }
        for intent in [Intent::Toggle, Intent::ToggleNote, Intent::ToggleTransform] {
            assert_eq!(resolved(AppStatus::Recording, intent), Some("stop"));
        }
    }

    #[test]
    fn presses_while_busy_or_paused_do_nothing() {
        for status in [
            AppStatus::Stopping,
            AppStatus::Transcribing,
            AppStatus::Pasting,
            AppStatus::Paused,
        ] {
            for intent in [
                Intent::Toggle,
                Intent::ToggleNote,
                Intent::ToggleTransform,
                Intent::StartHold,
                Intent::StartFromTray,
            ] {
                assert_eq!(
                    resolved(status.clone(), intent),
                    None,
                    "{intent:?} in {status:?}"
                );
            }
        }
    }

    #[test]
    fn hold_to_talk_never_auto_stops_and_tray_start_does() {
        assert_eq!(
            resolved(AppStatus::Idle, Intent::StartHold),
            Some("start-hold")
        );
        assert_eq!(
            resolved(AppStatus::Idle, Intent::StartFromTray),
            Some("start")
        );
        // Starting again while recording reports the active session.
        assert_eq!(
            resolved(AppStatus::Recording, Intent::StartHold),
            Some("start-hold")
        );
    }

    #[test]
    fn stop_cancel_and_paste_always_queue() {
        for status in ALL_STATUSES {
            assert_eq!(resolved(status.clone(), Intent::Stop), Some("stop"));
            assert_eq!(resolved(status.clone(), Intent::Cancel), Some("cancel"));
            assert_eq!(resolved(status, Intent::PasteLast), Some("paste-last"));
        }
    }

    #[test]
    fn admits_stop_only_while_recording() {
        for status in ALL_STATUSES {
            assert_eq!(
                admits(&status, &DictationCommand::Stop),
                status == AppStatus::Recording,
                "{status:?}"
            );
        }
    }

    #[test]
    fn admits_note_and_transform_starts_only_when_startable() {
        for status in ALL_STATUSES {
            let startable = matches!(
                status,
                AppStatus::Idle | AppStatus::Ready | AppStatus::Error
            );
            assert_eq!(admits(&status, &DictationCommand::StartNote), startable);
            assert_eq!(
                admits(&status, &DictationCommand::StartTransform),
                startable
            );
            let start = DictationCommand::Start {
                request: None,
                allow_auto_stop: true,
            };
            assert_eq!(
                admits(&status, &start),
                startable || status == AppStatus::Recording
            );
        }
    }

    #[test]
    fn a_skipped_start_explains_itself_and_other_skips_are_quiet() {
        let start = DictationCommand::Start {
            request: None,
            allow_auto_stop: true,
        };
        let error = skipped(&AppStatus::Transcribing, &start).unwrap_err();
        assert_eq!(error.code, "recording_unavailable");
        assert!(matches!(
            skipped(&AppStatus::Idle, &DictationCommand::Stop),
            Ok(Outcome::Skipped)
        ));
    }

    /// A fake executor with a tiny state machine of its own: starts move to
    /// Recording, stops go through Transcribing back to Idle. It records the
    /// order commands ran in and can be told to panic.
    #[derive(Clone, Default)]
    struct Fake {
        status: Arc<Mutex<Option<AppStatus>>>,
        ran: Arc<Mutex<Vec<&'static str>>>,
        reported: Arc<Mutex<Vec<String>>>,
        /// Time each stop "transcribes" for.
        stop_delay: Duration,
        panic_on_cancel: bool,
    }

    impl Fake {
        fn current(&self) -> AppStatus {
            self.status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(AppStatus::Idle)
        }

        fn set(&self, status: AppStatus) {
            *self.status.lock().unwrap() = Some(status);
        }

        fn ran(&self) -> Vec<&'static str> {
            self.ran.lock().unwrap().clone()
        }
    }

    impl Executor for Fake {
        fn status(&self) -> Result<AppStatus, CommandError> {
            Ok(self.current())
        }

        fn execute(&self, command: DictationCommand) -> Result<Outcome, CommandError> {
            self.ran.lock().unwrap().push(label(&command));
            match command {
                DictationCommand::Start { .. }
                | DictationCommand::StartNote
                | DictationCommand::StartTransform => self.set(AppStatus::Recording),
                DictationCommand::Stop => {
                    self.set(AppStatus::Transcribing);
                    thread::sleep(self.stop_delay);
                    self.set(AppStatus::Idle);
                }
                DictationCommand::Cancel if self.panic_on_cancel => panic!("cancel blew up"),
                DictationCommand::Cancel => self.set(AppStatus::Idle),
                DictationCommand::PasteLast => {
                    return Err(CommandError::new("no_last_transcript", "Nothing to paste."))
                }
                DictationCommand::FinishTimedOut { .. }
                | DictationCommand::FinishDisconnected { .. } => {}
            }
            Ok(Outcome::Done)
        }

        fn report(&self, error: CommandError) {
            self.reported.lock().unwrap().push(error.code);
        }
    }

    fn start() -> DictationCommand {
        DictationCommand::Start {
            request: None,
            allow_auto_stop: true,
        }
    }

    #[test]
    fn runs_commands_in_arrival_order_and_gates_on_the_status_when_they_run() {
        let fake = Fake::default();
        let worker = DictationWorker::spawn_with(fake.clone()).unwrap();

        worker.send(start());
        worker.send(DictationCommand::Stop);
        // Arrives after the stop has finished: nothing is recording any more,
        // so it is skipped instead of stopping a dictation it never saw.
        worker.send(DictationCommand::Stop);
        let outcome = worker.call(DictationCommand::Stop).unwrap();

        assert!(matches!(outcome, Outcome::Skipped));
        assert_eq!(fake.ran(), vec!["start", "stop"]);
        assert_eq!(fake.current(), AppStatus::Idle);
    }

    #[test]
    fn a_waiting_caller_gets_the_error_and_a_fire_and_forget_error_is_reported() {
        let fake = Fake::default();
        let worker = DictationWorker::spawn_with(fake.clone()).unwrap();

        let error = worker.call(DictationCommand::PasteLast).unwrap_err();
        assert_eq!(error.code, "no_last_transcript");
        assert!(fake.reported.lock().unwrap().is_empty());

        worker.send(DictationCommand::PasteLast);
        // A call queued behind it runs only after the send finished.
        worker.call(DictationCommand::Cancel).unwrap();
        assert_eq!(
            *fake.reported.lock().unwrap(),
            vec!["no_last_transcript".to_string()]
        );
    }

    #[test]
    fn a_skipped_start_errors_for_a_waiting_caller() {
        let fake = Fake::default();
        fake.set(AppStatus::Transcribing);
        let worker = DictationWorker::spawn_with(fake.clone()).unwrap();

        let error = worker.call(start()).unwrap_err();
        assert_eq!(error.code, "recording_unavailable");
        assert!(fake.ran().is_empty());
    }

    #[test]
    fn a_panicking_command_does_not_stop_the_worker() {
        let fake = Fake {
            panic_on_cancel: true,
            ..Fake::default()
        };
        let worker = DictationWorker::spawn_with(fake.clone()).unwrap();

        let error = worker.call(DictationCommand::Cancel).unwrap_err();
        assert_eq!(error.code, "dictation_failed");
        worker.call(start()).unwrap();
        assert_eq!(fake.ran(), vec!["cancel", "start"]);
        assert_eq!(fake.current(), AppStatus::Recording);
    }

    #[test]
    fn stops_from_many_sources_at_once_transcribe_exactly_once() {
        let fake = Fake {
            stop_delay: Duration::from_millis(20),
            ..Fake::default()
        };
        let worker = Arc::new(DictationWorker::spawn_with(fake.clone()).unwrap());
        worker.call(start()).unwrap();

        // Hotkey, pill, tray, dashboard, and silence auto-stop all ask to stop
        // the same recording at the same moment.
        let sources = 5;
        let barrier = Arc::new(Barrier::new(sources));
        let handles: Vec<_> = (0..sources)
            .map(|_| {
                let worker = Arc::clone(&worker);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    worker.call(DictationCommand::Stop).unwrap()
                })
            })
            .collect();
        let outcomes: Vec<Outcome> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let done = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::Done))
            .count();
        assert_eq!(done, 1, "exactly one stop transcribes");
        assert_eq!(fake.ran(), vec!["start", "stop"]);
    }

    #[test]
    fn the_caller_is_never_left_waiting_when_the_worker_is_gone() {
        let (sender, receiver) = channel::<Job>();
        drop(receiver);
        let worker = DictationWorker { sender };
        let error = worker.call(DictationCommand::Stop).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
    }
}
