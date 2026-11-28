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
    collections::VecDeque,
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{channel, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppStatus,
    audio::{self, RecordingResultStatus, RecordingSessionInfo, StartRecordingRequest},
    core::{Core, CoreCell, CoreLookup},
    dictation,
    error::CommandError,
    output::{self, OutputResult},
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
/// before the worker existed. `start_pending` says a start is queued but has
/// not run yet: the user already asked to record, so a quick second toggle
/// means stop, not a second start.
pub fn resolve(
    status: &AppStatus,
    start_pending: bool,
    intent: Intent,
) -> Option<DictationCommand> {
    let idle = matches!(
        status,
        AppStatus::Idle | AppStatus::Ready | AppStatus::Error
    );
    let startable = idle && !start_pending;
    let recording = *status == AppStatus::Recording || (idle && start_pending);
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

type JobResult = Result<Outcome, CommandError>;

/// Where a job's result goes: a caller blocked in [`DictationWorker::call`],
/// or one awaiting [`DictationWorker::call_async`]. Dropping a reply unsent
/// (the worker is gone) tells either caller so.
enum Reply {
    Blocking(Sender<JobResult>),
    Async(AsyncReply),
}

impl Reply {
    fn send(self, result: JobResult) {
        match self {
            Reply::Blocking(sender) => {
                let _ = sender.send(result);
            }
            Reply::Async(reply) => reply.send(result),
        }
    }
}

struct Job {
    command: DictationCommand,
    reply: Option<Reply>,
}

/// The state an [`AsyncReply`] and its [`ReplyFuture`] share.
#[derive(Default)]
struct ReplySlot {
    result: Option<JobResult>,
    /// The reply was sent or dropped: nothing more will arrive.
    closed: bool,
    waker: Option<Waker>,
}

fn lock_slot(slot: &Mutex<ReplySlot>) -> MutexGuard<'_, ReplySlot> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The worker's half of an async reply. Sending stores the result; dropping
/// it, sent or not, wakes the waiting task, which then finds the result or,
/// when none came (the job was dropped with the worker), [`worker_gone`].
struct AsyncReply(Arc<Mutex<ReplySlot>>);

impl AsyncReply {
    fn send(self, result: JobResult) {
        lock_slot(&self.0).result = Some(result);
    }
}

impl Drop for AsyncReply {
    fn drop(&mut self) {
        let waker = {
            let mut slot = lock_slot(&self.0);
            slot.closed = true;
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// The caller's half of an async reply: resolves to the job's result. Needs
/// no particular async runtime and holds no thread while it waits.
struct ReplyFuture(Arc<Mutex<ReplySlot>>);

impl Future for ReplyFuture {
    type Output = JobResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<JobResult> {
        let mut slot = lock_slot(&self.0);
        if let Some(result) = slot.result.take() {
            return Poll::Ready(result);
        }
        if slot.closed {
            return Poll::Ready(Err(worker_gone()));
        }
        slot.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

fn async_reply() -> (AsyncReply, ReplyFuture) {
    let slot = Arc::new(Mutex::new(ReplySlot::default()));
    (AsyncReply(slot.clone()), ReplyFuture(slot))
}

/// Handle to the worker thread. Clones share the one worker; the `Core`
/// holds one.
#[derive(Clone)]
pub struct DictationWorker {
    sender: Sender<Job>,
    /// Start commands queued but not yet run (see [`resolve`]).
    pending_starts: Arc<AtomicUsize>,
}

fn is_start(command: &DictationCommand) -> bool {
    matches!(
        command,
        DictationCommand::Start { .. }
            | DictationCommand::StartNote
            | DictationCommand::StartTransform
    )
}

impl DictationWorker {
    /// Starts the worker for a shell that hands it the `Core` through `cell`,
    /// which the shell fills right after `Core::new`.
    ///
    /// The worker never keeps the `Core`: it calls [`CoreCell::lookup`] once
    /// per job and drops the `Core` when that job ends. So the `Core`, which
    /// holds this worker's sender, can be dropped while the worker exists,
    /// and the worker thread then ends with the last sender. Before the cell
    /// is filled, jobs wait in arrival order and none is dropped. Once every
    /// `Core` handle is gone, whether or not a job ever ran, the waiting job
    /// and every later one are answered `dictation_worker_unavailable`.
    pub fn spawn_on(cell: CoreCell) -> std::io::Result<Self> {
        Self::spawn_finding(move || cell.lookup().map(|core| AppExecutor { core }))
    }

    /// [`DictationWorker::spawn_on`] for tests: finds the `Core` through
    /// `cell` the same way and runs each job on the executor `executor_for`
    /// builds from it, so no job reaches the microphone or the clipboard.
    #[cfg(test)]
    pub(crate) fn spawn_on_for_tests<E: Executor>(
        cell: CoreCell,
        executor_for: impl Fn(Core) -> E + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_finding(move || cell.lookup().map(&executor_for))
    }

    /// Starts the worker with an executor that is ready now.
    #[cfg(test)]
    pub(crate) fn spawn_with<E: Executor + Clone>(executor: E) -> std::io::Result<Self> {
        Self::spawn_finding(move || CoreLookup::Ready(executor.clone()))
    }

    /// Starts the worker thread. `find_executor` is asked for an executor
    /// once per job, which runs the job and is dropped with it, so the
    /// worker keeps no executor (and no engine) between jobs. While it
    /// answers `NotFilled`, it is asked again every [`EXECUTOR_POLL`] while
    /// jobs queue up in order; `Gone` answers the job
    /// `dictation_worker_unavailable`.
    fn spawn_finding<E: Executor>(
        find_executor: impl FnMut() -> CoreLookup<E> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = channel::<Job>();
        let pending_starts = Arc::new(AtomicUsize::new(0));
        let pending = Arc::clone(&pending_starts);
        thread::Builder::new()
            .name("scribe-dictation".to_string())
            .spawn(move || worker_loop(find_executor, &receiver, &pending))?;
        Ok(Self {
            sender,
            pending_starts,
        })
    }

    /// Resolves `intent` against `status` (and any start still queued) and
    /// queues the resulting command without waiting for it.
    pub(crate) fn submit_intent(
        &self,
        intent: Intent,
        status: impl FnOnce() -> Result<AppStatus, CommandError>,
    ) -> Result<(), CommandError> {
        // Read the pending count before the status: a start that finishes in
        // between has already moved the status to Recording.
        let start_pending = self.pending_starts.load(Ordering::SeqCst) > 0;
        let status = status()?;
        match resolve(&status, start_pending, intent) {
            Some(command) => self.send(command),
            None => log::debug!("Ignored {:?} while {:?}", intent, status),
        }
        Ok(())
    }

    /// Queues a command without waiting. Its error, if any, is reported as a
    /// recording error, the same way hotkey errors always were.
    pub fn send(&self, command: DictationCommand) {
        self.enqueue(command, None);
    }

    /// Queues a command and blocks until it has run. Never call this on the
    /// main thread; async callers use [`DictationWorker::call_async`].
    pub fn call(&self, command: DictationCommand) -> Result<Outcome, CommandError> {
        let (reply, receiver) = channel();
        self.enqueue(command, Some(Reply::Blocking(reply)));
        receiver.recv().unwrap_or_else(|_| Err(worker_gone()))
    }

    /// Queues a command now and returns a future of its result, the same
    /// result [`DictationWorker::call`] returns, without holding any thread
    /// while it waits. Queueing never blocks, so this is safe on any thread.
    pub fn call_async(
        &self,
        command: DictationCommand,
    ) -> impl Future<Output = Result<Outcome, CommandError>> + Send + 'static {
        let (reply, future) = async_reply();
        self.enqueue(command, Some(Reply::Async(reply)));
        future
    }

    fn enqueue(&self, command: DictationCommand, reply: Option<Reply>) {
        let start = is_start(&command);
        if start {
            self.pending_starts.fetch_add(1, Ordering::SeqCst);
        }
        if let Err(error) = self.sender.send(Job { command, reply }) {
            if start {
                self.pending_starts.fetch_sub(1, Ordering::SeqCst);
            }
            // The worker thread is gone; tell a waiting caller instead of
            // leaving it blocked.
            log::error!(
                "Dictation worker is not running; dropped {:?}",
                error.0.command
            );
            if let Some(reply) = error.0.reply {
                reply.send(Err(worker_gone()));
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

/// How often the worker looks for its executor (the engine) while the shell
/// has not built it yet.
const EXECUTOR_POLL: Duration = Duration::from_millis(50);

/// The worker thread: runs every job in arrival order. Ends when every sender
/// is gone, which is app shutdown (or the last `Core` dropped).
fn worker_loop<E: Executor>(
    mut find_executor: impl FnMut() -> CoreLookup<E>,
    receiver: &Receiver<Job>,
    pending_starts: &AtomicUsize,
) {
    // Jobs that arrived while the worker waited for its executor, in order.
    let mut backlog = VecDeque::new();
    loop {
        let job = match backlog.pop_front() {
            Some(job) => job,
            None => match receiver.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
        };
        let start = is_start(&job.command);
        let finished = || {
            if start {
                pending_starts.fetch_sub(1, Ordering::SeqCst);
            }
        };
        match wait_for_executor(&mut find_executor, receiver, &mut backlog) {
            // The executor, and the engine it holds, is dropped with its job.
            Found::Ready(executor) => run_job(&executor, job, finished),
            Found::Gone => engine_gone(job, finished),
            // The channel closed while waiting: the app is shutting down.
            // Dropping the waiting jobs tells their callers the worker is
            // gone.
            Found::Closed => return,
        }
    }
}

/// What [`wait_for_executor`] found.
enum Found<E> {
    Ready(E),
    /// The engine is gone for good (every `Core` handle was dropped).
    Gone,
    /// Every sender is gone before the engine came.
    Closed,
}

/// A job that arrived after the engine went away (every `Core` handle was
/// dropped, whether or not a job ever ran on it): it cannot run. A waiting
/// caller hears that the worker is gone; a fire-and-forget job is logged, as
/// when the worker thread itself is gone.
fn engine_gone(job: Job, finished: impl FnOnce()) {
    finished();
    log::error!(
        "Dictation worker has no engine any more; dropped {:?}",
        job.command
    );
    if let Some(reply) = job.reply {
        reply.send(Err(worker_gone()));
    }
}

/// Finds the executor for one job: asks once, and while the shell has not
/// built the engine yet (`NotFilled`) waits for it, asking again every
/// [`EXECUTOR_POLL`]. Jobs that arrive meanwhile join `backlog`, in order,
/// so none is dropped whatever the startup timing.
fn wait_for_executor<E>(
    find_executor: &mut impl FnMut() -> CoreLookup<E>,
    receiver: &Receiver<Job>,
    backlog: &mut VecDeque<Job>,
) -> Found<E> {
    loop {
        match find_executor() {
            CoreLookup::Ready(executor) => return Found::Ready(executor),
            CoreLookup::Gone => return Found::Gone,
            CoreLookup::NotFilled => {}
        }
        match receiver.recv_timeout(EXECUTOR_POLL) {
            Ok(job) => backlog.push_back(job),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Found::Closed,
        }
    }
}

/// Runs one job. `finished` runs once the command is done (or skipped), before
/// a waiting caller hears back.
fn run_job<E: Executor>(executor: &E, job: Job, finished: impl FnOnce()) {
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
    finished();

    match reply {
        Some(reply) => reply.send(result),
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
    match app.try_state::<Core>() {
        Some(core) => submit_with(&core, intent),
        None => log::error!(
            "Dictation worker is not reachable yet: the engine is not ready; dropped {:?}",
            intent
        ),
    }
}

/// [`submit`] on the engine.
pub fn submit_with(core: &Core, intent: Intent) {
    let status = || Ok(core.backend().app_state()?.status().clone());
    if let Err(error) = core.dictation_worker().submit_intent(intent, status) {
        audio::emit_recording_error_with(core.host().as_ref(), error);
    }
}

/// Runs `command` on the worker and waits for it without holding the calling
/// async task's thread, on any async runtime: the worker wakes the task when
/// the command has run. The task keeps no `Core` while it waits.
pub async fn request_with(core: Core, command: DictationCommand) -> Result<Outcome, CommandError> {
    let result = core.dictation_worker().call_async(command);
    drop(core);
    result.await
}

/// Production side effects: the real audio, transcription, and output paths.
struct AppExecutor {
    core: Core,
}

impl Executor for AppExecutor {
    fn status(&self) -> Result<AppStatus, CommandError> {
        Ok(self.core.backend().app_state()?.status().clone())
    }

    fn execute(&self, command: DictationCommand) -> Result<Outcome, CommandError> {
        execute_with(&self.core, command)
    }

    fn report(&self, error: CommandError) {
        refresh_tray(&self.core);
        audio::emit_recording_error_with(self.core.host().as_ref(), error);
    }
}

/// Runs one admitted command.
fn execute_with(core: &Core, command: DictationCommand) -> Result<Outcome, CommandError> {
    let outcome = match command {
        DictationCommand::Start {
            request,
            allow_auto_stop,
        } => Outcome::Started(audio::start_recording_with(core, request, allow_auto_stop)?),
        DictationCommand::StartNote => {
            let request = StartRecordingRequest {
                is_note: true,
                ..Default::default()
            };
            Outcome::Started(audio::start_recording_with(core, Some(request), true)?)
        }
        DictationCommand::StartTransform => {
            start_transform(core)?;
            Outcome::Done
        }
        DictationCommand::Stop => {
            stop_and_transcribe(core)?;
            Outcome::Done
        }
        DictationCommand::Cancel => {
            audio::cancel_recording_with(core)?;
            Outcome::Done
        }
        DictationCommand::PasteLast => {
            Outcome::Output(output::paste_last_transcript_with(core)?)
        }
        DictationCommand::FinishTimedOut { session_id } => {
            audio::finish_timed_out_recording_with(core, &session_id)?;
            Outcome::Done
        }
        DictationCommand::FinishDisconnected { session_id } => {
            // The disconnect already told the user what happened; a
            // failure to salvage the audio is only logged.
            if let Err(error) = audio::finish_disconnected_recording_with(core, &session_id) {
                log::warn!(
                    "Could not recover from a mid-recording audio stream error: {}",
                    error.message
                );
            }
            Outcome::Done
        }
    };
    refresh_tray(core);
    Ok(outcome)
}

fn refresh_tray(core: &Core) {
    if let Ok(machine) = core.backend().app_state() {
        let status = machine.status().clone();
        drop(machine);
        core.host().update_tray_status(status);
    }
}

/// Stops the active recording and finishes it: a too-short tap is reported as
/// "nothing heard"; anything else is transcribed and output.
fn stop_and_transcribe(core: &Core) -> Result<(), CommandError> {
    let result = audio::stop_recording_with(core)?;
    refresh_tray(core);

    if result.status == RecordingResultStatus::TooShort {
        // A quick tap of the toggle hotkey is benign, not an error: the state
        // machine already returned to Idle (AudioTooShort), so just tell the
        // frontend nothing was heard.
        dictation::emit_dictation_empty_with(core.host().as_ref());
        return Ok(());
    }

    dictation::transcribe_recording_with(core, result)?;
    Ok(())
}

/// Where the system can't copy the selection out of another app, the
/// typed-instruction panel on the dashboard stands in for it.
fn start_transform(core: &Core) -> Result<(), CommandError> {
    if crate::platform::can_capture_selection() {
        crate::hotkeys::start_transform_with(core)
    } else {
        core.host().open_dashboard(Some("dashboard"))
    }
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
        resolve(&status, false, intent).as_ref().map(label)
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
    fn a_toggle_behind_a_queued_start_stops_but_a_busy_press_is_still_ignored() {
        for intent in [Intent::Toggle, Intent::ToggleNote, Intent::ToggleTransform] {
            let command = resolve(&AppStatus::Idle, true, intent);
            assert_eq!(command.as_ref().map(label), Some("stop"));
            assert_eq!(
                resolve(&AppStatus::Transcribing, true, intent).map(|c| label(&c)),
                None
            );
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
        /// Time each start takes to open the mic.
        start_delay: Duration,
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
                | DictationCommand::StartTransform => {
                    thread::sleep(self.start_delay);
                    self.set(AppStatus::Recording);
                }
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
    fn a_fast_double_toggle_starts_then_stops() {
        let fake = Fake {
            start_delay: Duration::from_millis(50),
            ..Fake::default()
        };
        let worker = DictationWorker::spawn_with(fake.clone()).unwrap();

        // The second press lands while the first start is still opening the
        // mic, so the status the user's key sees is still Idle.
        worker
            .submit_intent(Intent::Toggle, || Ok(fake.current()))
            .unwrap();
        worker
            .submit_intent(Intent::Toggle, || Ok(fake.current()))
            .unwrap();
        // Queued last, so it returns once both toggles have run.
        worker
            .call(DictationCommand::FinishTimedOut {
                session_id: String::new(),
            })
            .unwrap();

        assert_eq!(fake.ran(), vec!["start", "stop", "timed-out"]);
        assert_eq!(fake.current(), AppStatus::Idle);
        assert_eq!(worker.pending_starts.load(Ordering::SeqCst), 0);
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

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn jobs_sent_before_the_engine_exists_wait_for_it_and_run_in_order() {
        use std::sync::atomic::AtomicBool;

        let fake = Fake::default();
        let ready = Arc::new(AtomicBool::new(false));
        let asked = Arc::new(AtomicUsize::new(0));
        let worker = {
            let (fake, ready, asked) = (fake.clone(), ready.clone(), asked.clone());
            DictationWorker::spawn_finding(move || {
                asked.fetch_add(1, Ordering::SeqCst);
                if ready.load(Ordering::SeqCst) {
                    CoreLookup::Ready(fake.clone())
                } else {
                    CoreLookup::NotFilled
                }
            })
            .unwrap()
        };

        // A hotkey start and stop land before the shell has built the engine.
        worker.send(start());
        worker.send(DictationCommand::Stop);
        // The worker keeps looking for the engine without running or
        // dropping anything.
        wait_until("the worker looks for the engine again", || {
            asked.load(Ordering::SeqCst) >= 3
        });
        assert!(fake.ran().is_empty());

        ready.store(true, Ordering::SeqCst);
        // Queued last, so it returns once both earlier jobs have run.
        let outcome = worker.call(DictationCommand::Stop).unwrap();
        assert!(matches!(outcome, Outcome::Skipped));
        assert_eq!(fake.ran(), vec!["start", "stop"]);
        assert_eq!(worker.pending_starts.load(Ordering::SeqCst), 0);

        // Once found, the engine is looked up once per job (it is not kept
        // between jobs), with no more polling.
        let asked_once_found = asked.load(Ordering::SeqCst);
        worker.call(start()).unwrap();
        worker.call(DictationCommand::Stop).unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), asked_once_found + 2);
        assert_eq!(fake.ran(), vec!["start", "stop", "start", "stop"]);
    }

    /// Lets a test hold the engine as an `Arc` and hand the worker only a
    /// weak reference to it, the way `CoreCell` does with the `Core`.
    impl Executor for Arc<Fake> {
        fn status(&self) -> Result<AppStatus, CommandError> {
            self.as_ref().status()
        }

        fn execute(&self, command: DictationCommand) -> Result<Outcome, CommandError> {
            self.as_ref().execute(command)
        }

        fn report(&self, error: CommandError) {
            self.as_ref().report(error)
        }
    }

    /// Sets its flag when dropped: the worker thread dropped its lookup,
    /// which it does only when it ends.
    struct SetOnDrop(Arc<std::sync::atomic::AtomicBool>);

    impl Drop for SetOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// The `CoreCell` contract, on a stand-in engine behind a weak
    /// reference: the worker keeps no strong reference between jobs, so
    /// the engine can be dropped while the worker exists; jobs after that
    /// are answered at once, not left polling; and the worker thread ends
    /// once its last sender is gone. The same on a real `Core`:
    /// `a_core_dropped_before_any_job_answers_every_job_unavailable`.
    #[test]
    fn the_engine_can_be_dropped_while_the_worker_exists() {
        use std::sync::atomic::AtomicBool;

        let engine = Arc::new(Fake::default());
        let watch = Arc::downgrade(&engine);
        let ended = Arc::new(AtomicBool::new(false));
        let worker = {
            let cell = Arc::downgrade(&engine);
            let guard = SetOnDrop(ended.clone());
            DictationWorker::spawn_finding(move || {
                let _guard = &guard;
                match cell.upgrade() {
                    Some(engine) => CoreLookup::Ready(engine),
                    None => CoreLookup::Gone,
                }
            })
            .unwrap()
        };

        // Every wait has a deadline, so a regression fails instead of hanging.
        call_within_deadline(&worker, start()).unwrap();
        call_within_deadline(&worker, DictationCommand::Stop).unwrap();
        assert_eq!(engine.ran(), vec!["start", "stop"]);
        // Each job's handle is dropped right after the job.
        wait_until("the worker lets go of the engine", || {
            Arc::strong_count(&engine) == 1
        });

        drop(engine);
        assert!(watch.upgrade().is_none(), "nothing else holds the engine");

        // A waiting caller hears the engine is gone instead of waiting.
        let error = call_within_deadline(&worker, start()).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
        let error = block_on(worker.call_async(DictationCommand::Stop)).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
        // A fire-and-forget start is dropped without leaving a start pending.
        worker.send(start());
        call_within_deadline(&worker, DictationCommand::Cancel).unwrap_err();
        assert_eq!(worker.pending_starts.load(Ordering::SeqCst), 0);
        assert!(!ended.load(Ordering::SeqCst));

        // The helper threads' worker clones are gone once they replied, so
        // dropping this last handle closes the channel.
        drop(worker);
        wait_until("the worker thread ends", || ended.load(Ordering::SeqCst));
    }

    /// Polls `future` on this thread until it is ready, parking in between:
    /// the worker's wake unparks it.
    fn block_on<F: Future>(future: F) -> F::Output {
        struct Unpark(thread::Thread);
        impl std::task::Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        // A reply that never comes fails the test instead of hanging it.
        let deadline = std::time::Instant::now() + TEST_DEADLINE;
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "the reply never came");
            thread::park_timeout(left);
        }
    }

    /// How long a test waits for the worker before it fails.
    const TEST_DEADLINE: Duration = Duration::from_secs(10);

    /// [`DictationWorker::call`] on a helper thread, failing the test when
    /// no reply comes within [`TEST_DEADLINE`] instead of hanging it.
    fn call_within_deadline(worker: &DictationWorker, command: DictationCommand) -> JobResult {
        let worker = worker.clone();
        let (done, result) = channel();
        thread::spawn(move || {
            let _ = done.send(worker.call(command));
        });
        result
            .recv_timeout(TEST_DEADLINE)
            .expect("the reply never came")
    }

    #[test]
    fn an_async_caller_gets_what_a_blocking_caller_gets_in_order() {
        let fake = Fake {
            stop_delay: Duration::from_millis(20),
            ..Fake::default()
        };
        let worker = DictationWorker::spawn_with(fake.clone()).unwrap();

        // Queued when called, in order with every other source.
        let started = worker.call_async(start());
        worker.send(DictationCommand::Stop);
        let late_stop = worker.call_async(DictationCommand::Stop);
        assert!(matches!(block_on(started), Ok(Outcome::Done)));
        assert!(matches!(block_on(late_stop), Ok(Outcome::Skipped)));
        assert_eq!(fake.ran(), vec!["start", "stop"]);

        // An error goes to the waiting caller, not to the error event.
        let error = block_on(worker.call_async(DictationCommand::PasteLast)).unwrap_err();
        assert_eq!(error.code, "no_last_transcript");
        fake.set(AppStatus::Transcribing);
        let error = block_on(worker.call_async(start())).unwrap_err();
        assert_eq!(error.code, "recording_unavailable");
        assert!(fake.reported.lock().unwrap().is_empty());
        assert_eq!(worker.pending_starts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_async_caller_is_never_left_waiting_when_the_worker_is_gone() {
        // The worker thread is gone before the command is queued.
        let (sender, receiver) = channel::<Job>();
        drop(receiver);
        let gone = DictationWorker {
            sender,
            pending_starts: Arc::new(AtomicUsize::new(0)),
        };
        let error = block_on(gone.call_async(DictationCommand::Stop)).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");

        // The worker shuts down (its last sender dropped) while the command
        // still waits for an engine that never came.
        let waiting = DictationWorker::spawn_finding(|| CoreLookup::<Fake>::NotFilled).unwrap();
        let pending = waiting.call_async(start());
        drop(waiting);
        let error = block_on(pending).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
    }

    #[test]
    fn an_async_reply_can_cross_threads_and_outlive_the_worker_handle() {
        fn send_static<T: Send + 'static>(_: &T) {}
        let worker = DictationWorker::spawn_with(Fake::default()).unwrap();
        let reply = worker.call_async(DictationCommand::Cancel);
        send_static(&reply);
        drop(worker);
        let outcome = thread::spawn(move || block_on(reply)).join().unwrap();
        assert!(matches!(outcome, Ok(Outcome::Done)));
    }

    #[test]
    fn an_idle_worker_does_not_look_for_the_engine() {
        let asked = Arc::new(AtomicUsize::new(0));
        let worker = {
            let asked = asked.clone();
            DictationWorker::spawn_finding(move || {
                asked.fetch_add(1, Ordering::SeqCst);
                CoreLookup::Ready(Fake::default())
            })
            .unwrap()
        };
        thread::sleep(Duration::from_millis(120));
        assert_eq!(asked.load(Ordering::SeqCst), 0);
        worker.call(DictationCommand::Cancel).unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 1);
    }

    /// The regression step 3b fixes: a cell filled and emptied before any
    /// job ran used to look unfilled, so the first job polled until the
    /// channel closed, and a caller holding a worker handle (which keeps the
    /// channel open) never heard back.
    #[test]
    fn a_core_dropped_before_any_job_answers_every_job_unavailable() {
        use crate::core::test_core::TestCore;

        let TestCore {
            core,
            cell,
            ran,
            dir: _dir,
            ..
        } = TestCore::new();
        let worker = core.dictation_worker().clone();
        core.shutdown();
        drop(core);
        assert!(matches!(cell.lookup(), CoreLookup::Gone));

        let error = block_on(worker.call_async(start())).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
        let error = call_within_deadline(&worker, DictationCommand::Stop).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
        // A fire-and-forget start is dropped too, without leaving a start
        // pending (the call behind it returns only after it was handled).
        worker.send(start());
        let error = block_on(worker.call_async(DictationCommand::Cancel)).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
        assert_eq!(worker.pending_starts.load(Ordering::SeqCst), 0);
        assert!(ran.lock().unwrap().is_empty());
    }

    #[test]
    fn a_cell_driven_worker_runs_jobs_on_the_real_core_then_answers_unavailable_once_it_is_gone() {
        use crate::core::test_core::TestCore;

        let TestCore {
            core,
            cell,
            ran,
            dir: _dir,
            ..
        } = TestCore::new();
        let worker = core.dictation_worker().clone();
        let outcome = block_on(worker.call_async(DictationCommand::Cancel)).unwrap();
        assert!(matches!(outcome, Outcome::Done));
        // Not admitted while the real state machine is idle.
        let outcome = block_on(worker.call_async(DictationCommand::Stop)).unwrap();
        assert!(matches!(outcome, Outcome::Skipped));
        assert_eq!(*ran.lock().unwrap(), vec!["Cancel".to_string()]);

        // The worker may still hold the last job's `Core` for a moment after
        // replying, but it lets go before it takes the next job, so the next
        // job finds the engine gone.
        core.shutdown();
        drop(core);
        let error = block_on(worker.call_async(DictationCommand::Cancel)).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
        assert!(matches!(cell.lookup(), CoreLookup::Gone));
        assert_eq!(*ran.lock().unwrap(), vec!["Cancel".to_string()]);
    }

    #[test]
    fn jobs_waiting_for_an_engine_that_goes_away_are_all_answered_in_order() {
        #[derive(Clone, Copy)]
        enum Engine {
            NotFilled,
            Gone,
        }
        let engine = Arc::new(Mutex::new(Engine::NotFilled));
        let (asked, was_asked) = channel::<()>();
        let worker = {
            let engine = engine.clone();
            DictationWorker::spawn_finding(move || {
                let _ = asked.send(());
                match *engine.lock().unwrap() {
                    Engine::NotFilled => CoreLookup::<Fake>::NotFilled,
                    Engine::Gone => CoreLookup::Gone,
                }
            })
            .unwrap()
        };

        let first = worker.call_async(start());
        worker.send(start());
        let last = worker.call_async(DictationCommand::Stop);
        // The worker is waiting for the engine: asked, and asked again.
        for _ in 0..2 {
            was_asked
                .recv_timeout(TEST_DEADLINE)
                .expect("the worker looks for the engine");
        }

        *engine.lock().unwrap() = Engine::Gone;
        assert_eq!(
            block_on(first).unwrap_err().code,
            "dictation_worker_unavailable"
        );
        assert_eq!(
            block_on(last).unwrap_err().code,
            "dictation_worker_unavailable"
        );
        // Both starts were handled, the fire-and-forget one before `last`.
        assert_eq!(worker.pending_starts.load(Ordering::SeqCst), 0);
        // And every later job too, at once.
        let error = call_within_deadline(&worker, DictationCommand::Cancel).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
    }

    #[test]
    fn the_caller_is_never_left_waiting_when_the_worker_is_gone() {
        let (sender, receiver) = channel::<Job>();
        drop(receiver);
        let worker = DictationWorker {
            sender,
            pending_starts: Arc::new(AtomicUsize::new(0)),
        };
        let error = worker.call(DictationCommand::Stop).unwrap_err();
        assert_eq!(error.code, "dictation_worker_unavailable");
    }
}
