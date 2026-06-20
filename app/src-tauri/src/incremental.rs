//! Incremental transcription: while the user is still talking, finished
//! phrases are cut at natural pauses, written out as 16 kHz WAV segments, and
//! transcribed in the background through the shared [`WarmTranscriber`]. On
//! stop only the tail phrase remains to transcribe, so stop-to-text latency
//! stays near-instant regardless of recording length.
//!
//! Architecture per recording session:
//! - The recording worker owns a [`WorkerLink`] (segmenter + sender). It cuts
//!   segments and hands their WAV paths to the coordinator in order.
//! - A coordinator thread receives segment paths, transcribes each one,
//!   accumulates the texts in order, and emits
//!   `scribe:partial-transcript` events as it goes.
//! - The dictation stop path looks the session up in the [`Registry`] and
//!   waits (bounded) for the assembled text. Any failure anywhere degrades
//!   gracefully to the existing full-clip transcription path.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use crossbeam_channel::{bounded, unbounded, Receiver, RecvTimeoutError, Sender};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    audio::{self, AUTO_STOP_SPEECH_RMS, SILENCE_RMS_THRESHOLD},
    commands::BackendState,
    model_manager,
    whisper::WhisperRequest,
    whisper_server::WarmTranscriber,
};

// The pause threshold and length cap are now per-recording settings
// (`segment_pause_*` / `segment_max_ms`), passed into `Segmenter::new` from
// `start_session`. Why they are the dominant lever on "too much punctuation
// when I pause": each finalized segment is transcribed as a standalone clip, so
// Whisper closes it with sentence-final punctuation, and `join_segments` then
// concatenates them — planting a period/comma exactly where the cut happened. A
// short pause threshold (the old 350 ms) cut on nearly every mid-sentence
// hesitation, manufacturing sentence breaks all over normal speech; a longer
// one (default 3 s) lines segments up with deliberate sentence-ending pauses.
// Cutting stays pause-based (never a fixed time slice, which would cut mid-word
// and hand Whisper a truncated fragment); the length cap is only a safety net
// for speech with no qualifying pause, bounded to Whisper's ~30 s window so a
// segment can never be truncated.

/// When pause-based cutting is disabled, how much trailing audio to keep while
/// bounding the buffer at the length cap during speechless input. Independent
/// of the user's pause threshold.
const SILENCE_KEEP_MS: u64 = 1_000;
/// Trailing silence appended to each segment WAV. Whisper tends to drop the
/// final word when the audio ends abruptly mid-/right-after speech, which the
/// tail segment otherwise does.
const SEGMENT_TAIL_PAD_MS: u64 = 300;
/// How much accumulated transcript text is appended to the vocabulary prompt
/// for cross-segment continuity.
const PROMPT_CONTEXT_CHARS: usize = 200;
/// How long the dictation stop path waits for the coordinator's assembled
/// text before falling back to full-clip transcription.
pub const RESULT_TIMEOUT: Duration = Duration::from_secs(15);

const PARTIAL_TRANSCRIPT_EVENT: &str = "scribe:partial-transcript";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PartialTranscriptEvent {
    session_id: String,
    text: String,
    segments: u32,
    finalized: bool,
}

/// Emits a `scribe:partial-transcript` event. The dictation stop path
/// uses this for the final `finalized: true` event; the coordinator emits the
/// in-progress ones.
pub fn emit_partial_transcript(
    app: &AppHandle,
    session_id: &str,
    text: &str,
    segments: u32,
    finalized: bool,
) {
    let _ = app.emit(
        PARTIAL_TRANSCRIPT_EVENT,
        PartialTranscriptEvent {
            session_id: session_id.to_string(),
            text: text.to_string(),
            segments,
            finalized,
        },
    );
}

/// Cuts a stream of audio chunks into transcribable segments. A segment is
/// finalized once it contains speech (any chunk RMS at or above
/// [`AUTO_STOP_SPEECH_RMS`]) and ends in `pause_ms` of continuous silence (when
/// pause-based cutting is enabled), or once it reaches `max_ms`. Silence-only
/// audio between segments is never accumulated.
struct Segmenter {
    sample_rate: u32,
    samples: Vec<f32>,
    speech_detected: bool,
    trailing_silence_samples: usize,
    /// Pause length that finalizes a segment, or `None` to disable pause-based
    /// cutting (segments then end only at `max_ms`).
    pause_ms: Option<u64>,
    /// Hard cap on segment length (Whisper-safe; the caller clamps it).
    max_ms: u64,
}

impl Segmenter {
    fn new(sample_rate: u32, pause_ms: Option<u64>, max_ms: u64) -> Self {
        Self {
            sample_rate,
            samples: Vec::new(),
            speech_detected: false,
            trailing_silence_samples: 0,
            pause_ms,
            max_ms,
        }
    }

    /// Feeds one chunk of mono samples at the source rate. Returns the
    /// finalized segment's samples when this chunk completed one.
    fn push_chunk(&mut self, chunk: &[f32]) -> Option<Vec<f32>> {
        if chunk.is_empty() || self.sample_rate == 0 {
            return None;
        }

        let rms = audio::chunk_rms(chunk);
        // Silence between segments never starts a new segment.
        if self.samples.is_empty() && rms < SILENCE_RMS_THRESHOLD {
            return None;
        }

        self.samples.extend_from_slice(chunk);
        if rms >= AUTO_STOP_SPEECH_RMS {
            self.speech_detected = true;
            self.trailing_silence_samples = 0;
        } else if rms >= SILENCE_RMS_THRESHOLD {
            // Not speech, but not silent either: the silence run is broken.
            self.trailing_silence_samples = 0;
        } else {
            self.trailing_silence_samples += chunk.len();
        }

        if let Some(pause_ms) = self.pause_ms {
            if self.speech_detected
                && audio::duration_ms(self.trailing_silence_samples, self.sample_rate) >= pause_ms
            {
                return Some(self.take_segment());
            }
        }

        if audio::duration_ms(self.samples.len(), self.sample_rate) >= self.max_ms {
            if self.speech_detected {
                return Some(self.take_segment());
            }
            // Speech never showed up: keep only a short tail so the buffer
            // stays bounded. Everything dropped is below the speech
            // threshold; the full recording still captures it.
            let keep = ((self.sample_rate as u64 * SILENCE_KEEP_MS) / 1_000) as usize;
            let drop = self.samples.len().saturating_sub(keep);
            self.samples.drain(..drop);
            self.trailing_silence_samples = self.trailing_silence_samples.min(self.samples.len());
        }

        None
    }

    /// Final flush at stop: whatever is left, with no trailing-silence
    /// requirement — but only when it actually contains speech.
    fn take_tail(&mut self) -> Option<Vec<f32>> {
        if self.speech_detected {
            Some(self.take_segment())
        } else {
            self.samples.clear();
            self.trailing_silence_samples = 0;
            None
        }
    }

    fn take_segment(&mut self) -> Vec<f32> {
        self.speech_detected = false;
        self.trailing_silence_samples = 0;
        std::mem::take(&mut self.samples)
    }
}

/// Joins per-segment transcripts with single spaces, dropping empty ones.
fn join_segments(texts: &[String]) -> String {
    texts
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Per-segment prompt: the vocabulary prompt plus the last
/// [`PROMPT_CONTEXT_CHARS`] characters of the accumulated transcript, for
/// cross-segment continuity.
fn prompt_with_context(vocabulary_prompt: &str, accumulated: &str) -> String {
    let vocabulary = vocabulary_prompt.trim();
    let context = text_tail(accumulated.trim(), PROMPT_CONTEXT_CHARS);

    match (vocabulary.is_empty(), context.is_empty()) {
        (true, true) => String::new(),
        (false, true) => vocabulary.to_string(),
        (true, false) => context.to_string(),
        (false, false) => format!("{} {}", vocabulary, context),
    }
}

/// Last `max_chars` characters of `text`, on a char boundary.
fn text_tail(text: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }

    match text.char_indices().rev().nth(max_chars - 1) {
        Some((index, _)) => &text[index..],
        None => text,
    }
}

#[derive(Debug)]
enum Msg {
    /// An ordered, finished segment WAV (16 kHz mono PCM16) to transcribe.
    Segment(PathBuf),
    /// The worker could not produce a segment; the session must fall back.
    Fail,
    /// End of stream: all segments (including the tail) have been sent.
    Finish,
    /// Discard all pending work and exit.
    Cancel,
}

/// The coordinator's assembled output for one session.
#[derive(Debug, Clone)]
pub struct AssembledTranscript {
    pub text: String,
    pub segments: u32,
}

/// Handle the dictation stop path uses to collect (or cancel) a session.
pub struct SessionHandle {
    msg_tx: Sender<Msg>,
    result_rx: Receiver<Result<AssembledTranscript, ()>>,
}

impl SessionHandle {
    /// Waits for the coordinator's assembled transcript. Errors describe why
    /// the caller should fall back to full-clip transcription.
    pub fn wait(&self, timeout: Duration) -> Result<AssembledTranscript, String> {
        match self.result_rx.recv_timeout(timeout) {
            Ok(Ok(assembled)) => Ok(assembled),
            Ok(Err(())) => Err("a segment transcription failed".to_string()),
            Err(RecvTimeoutError::Timeout) => {
                Err(format!("timed out after {} s", timeout.as_secs()))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err("the coordinator exited without a result".to_string())
            }
        }
    }

    /// Tells the coordinator to discard all pending work. Safe to call after
    /// the coordinator has already exited.
    pub fn cancel(&self) {
        let _ = self.msg_tx.send(Msg::Cancel);
    }
}

/// Maps active session ids to their coordinator handles so the dictation stop
/// path (which only sees a serialized `RecordingResult`) can find them.
#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<String, SessionHandle>>,
}

impl Registry {
    fn register(&self, session_id: String, handle: SessionHandle) {
        self.lock().insert(session_id, handle);
    }

    /// Removes and returns the session's handle. Each session is consumed at
    /// most once.
    pub fn take(&self, session_id: &str) -> Option<SessionHandle> {
        self.lock().remove(session_id)
    }

    /// Removes the session and cancels any pending coordinator work. Used
    /// when a recording ends without transcription (cancelled / too short).
    pub fn discard(&self, session_id: &str) {
        if let Some(handle) = self.take(session_id) {
            handle.cancel();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, SessionHandle>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The recording worker's side of a session: segments the live audio and
/// hands finished segment WAVs to the coordinator.
pub struct WorkerLink {
    session_id: String,
    temp_dir: PathBuf,
    msg_tx: Sender<Msg>,
    segmenter: Segmenter,
    next_segment: u32,
    failed: bool,
}

impl WorkerLink {
    /// Feeds one chunk of mono samples at the source rate.
    pub fn push_chunk(&mut self, chunk: &[f32]) {
        if self.failed {
            return;
        }
        if let Some(segment) = self.segmenter.push_chunk(chunk) {
            self.send_segment(segment);
        }
    }

    /// Flushes the tail segment (if it contains speech) and signals
    /// end-of-stream so the coordinator assembles its result.
    pub fn finish(mut self) {
        if !self.failed {
            if let Some(tail) = self.segmenter.take_tail() {
                self.send_segment(tail);
            }
        }
        let _ = self.msg_tx.send(Msg::Finish);
    }

    /// Discards the session (recording cancelled or too short).
    pub fn cancel(self) {
        let _ = self.msg_tx.send(Msg::Cancel);
    }

    fn send_segment(&mut self, samples: Vec<f32>) {
        let mut normalized =
            audio::normalize_to_whisper_wav_samples(&samples, self.segmenter.sample_rate);
        let pad_samples = (audio::TARGET_SAMPLE_RATE as u64 * SEGMENT_TAIL_PAD_MS / 1_000) as usize;
        normalized.extend(std::iter::repeat(0.0).take(pad_samples));
        let path = self.temp_dir.join(format!(
            "{}-segment-{}.wav",
            self.session_id, self.next_segment
        ));
        self.next_segment += 1;

        match audio::write_wav(&path, &normalized) {
            Ok(()) => {
                let _ = self.msg_tx.send(Msg::Segment(path));
            }
            Err(error) => {
                log::warn!(
                    "Could not write incremental segment WAV for session {}; falling back to full-clip transcription. {}",
                    self.session_id,
                    error.message
                );
                self.failed = true;
                let _ = self.msg_tx.send(Msg::Fail);
            }
        }
    }
}

/// Starts the per-session coordinator thread and registers its handle.
/// Returns the worker's link, or None when the coordinator could not start
/// (the caller then behaves exactly as if incremental were disabled).
pub fn start_session(
    app: &AppHandle,
    session_id: &str,
    temp_dir: PathBuf,
    source_sample_rate: u32,
    pause_ms: Option<u64>,
    max_ms: u64,
) -> Option<WorkerLink> {
    let (msg_tx, msg_rx) = unbounded::<Msg>();
    let (result_tx, result_rx) = bounded::<Result<AssembledTranscript, ()>>(1);

    let registry_state = app.state::<BackendState>();
    registry_state.incremental().register(
        session_id.to_string(),
        SessionHandle {
            msg_tx: msg_tx.clone(),
            result_rx,
        },
    );

    let coordinator_app = app.clone();
    let coordinator_session = session_id.to_string();
    let spawned = std::thread::Builder::new()
        .name("incremental-transcribe".to_string())
        .spawn(move || run_coordinator(coordinator_app, coordinator_session, msg_rx, result_tx));

    if let Err(error) = spawned {
        log::warn!(
            "Could not start incremental transcription coordinator for session {}; falling back to full-clip transcription. {}",
            session_id,
            error
        );
        registry_state.incremental().take(session_id);
        return None;
    }

    log::info!(
        "Incremental transcription active for session {} (segment pause {}, cap {} ms)",
        session_id,
        match pause_ms {
            Some(ms) => format!("{} ms", ms),
            None => "disabled".to_string(),
        },
        max_ms
    );
    Some(WorkerLink {
        session_id: session_id.to_string(),
        temp_dir,
        msg_tx,
        segmenter: Segmenter::new(source_sample_rate, pause_ms, max_ms),
        next_segment: 0,
        failed: false,
    })
}

/// Settings and model resolved once per session, at the first segment.
struct SegmentContext {
    model_path: PathBuf,
    language: String,
    translate: bool,
    vocabulary_prompt: String,
}

fn run_coordinator(
    app: AppHandle,
    session_id: String,
    msg_rx: Receiver<Msg>,
    result_tx: Sender<Result<AssembledTranscript, ()>>,
) {
    let mut texts: Vec<String> = Vec::new();
    let mut segments: u32 = 0;
    let mut failed = false;
    let mut context: Option<SegmentContext> = None;

    loop {
        match msg_rx.recv() {
            Ok(Msg::Segment(path)) => {
                if !failed {
                    let accumulated = join_segments(&texts);
                    match transcribe_segment(&app, &mut context, &path, &accumulated) {
                        Ok(text) => {
                            segments += 1;
                            if !text.is_empty() {
                                texts.push(text);
                            }
                            emit_partial_transcript(
                                &app,
                                &session_id,
                                &join_segments(&texts),
                                segments,
                                false,
                            );
                        }
                        Err(error) => {
                            log::warn!(
                                "Incremental segment transcription failed for session {}; falling back to full-clip transcription. {}",
                                session_id,
                                error
                            );
                            failed = true;
                        }
                    }
                }
                let _ = fs::remove_file(&path);
            }
            Ok(Msg::Fail) => failed = true,
            Ok(Msg::Finish) => {
                let result = if failed {
                    Err(())
                } else {
                    Ok(AssembledTranscript {
                        text: join_segments(&texts),
                        segments,
                    })
                };
                let _ = result_tx.send(result);
                break;
            }
            Ok(Msg::Cancel) | Err(_) => break,
        }
    }

    // Session over (finish, cancel, or all senders gone): delete any segment
    // WAVs still queued so nothing is left behind in the temp dir.
    while let Ok(message) = msg_rx.try_recv() {
        if let Msg::Segment(path) = message {
            let _ = fs::remove_file(&path);
        }
    }
}

fn transcribe_segment(
    app: &AppHandle,
    context: &mut Option<SegmentContext>,
    wav_path: &Path,
    accumulated: &str,
) -> Result<String, String> {
    if context.is_none() {
        *context = Some(segment_context(app)?);
    }
    let context = context.as_ref().expect("segment context resolved above");

    let transcription = app
        .state::<WarmTranscriber>()
        .transcribe(
            app,
            WhisperRequest {
                model_path: context.model_path.clone(),
                wav_path: wav_path.to_path_buf(),
                language: context.language.clone(),
                translate: context.translate,
                vocabulary_prompt: prompt_with_context(&context.vocabulary_prompt, accumulated),
            },
        )
        .map_err(|error| error.message)?;

    Ok(transcription.text)
}

fn segment_context(app: &AppHandle) -> Result<SegmentContext, String> {
    let state = app.state::<BackendState>();
    let db = state.db().map_err(|error| error.message)?;
    let settings = db.get_settings().map_err(|error| error.message)?;
    let (_, model_path) =
        model_manager::selected_model_path(app, &db).map_err(|error| error.message)?;

    Ok(SegmentContext {
        model_path,
        language: crate::dictation::whisper_language(&settings.language),
        translate: settings.translate_to_english,
        vocabulary_prompt: settings.vocabulary_prompt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;
    /// 20 ms chunks at 16 kHz, matching the trim window granularity.
    const CHUNK: usize = 320;
    /// Pause/cap the tests construct segmenters with, independent of the
    /// shipped defaults so the assertions don't drift if those change.
    const TEST_PAUSE_MS: u64 = 1_000;
    const TEST_MAX_MS: u64 = 25_000;

    /// A segmenter with pause-based cutting on, at the test pause/cap.
    fn new_segmenter() -> Segmenter {
        Segmenter::new(RATE, Some(TEST_PAUSE_MS), TEST_MAX_MS)
    }

    fn speech(ms: usize) -> Vec<f32> {
        vec![0.5; RATE as usize * ms / 1_000]
    }

    fn silence(ms: usize) -> Vec<f32> {
        vec![0.0; RATE as usize * ms / 1_000]
    }

    /// Above the silence threshold but below the speech threshold.
    fn quiet(ms: usize) -> Vec<f32> {
        vec![0.02; RATE as usize * ms / 1_000]
    }

    /// Trailing silence included in a finalized segment: the pause threshold
    /// rounded up to the 20 ms chunk that crossed it.
    fn silence_marker_len() -> usize {
        ((RATE as u64 * TEST_PAUSE_MS / 1_000) as usize).div_ceil(CHUNK) * CHUNK
    }

    /// Feeds samples in 20 ms chunks and collects every finalized segment.
    fn feed(segmenter: &mut Segmenter, samples: &[f32]) -> Vec<Vec<f32>> {
        samples
            .chunks(CHUNK)
            .filter_map(|chunk| segmenter.push_chunk(chunk))
            .collect()
    }

    #[test]
    fn finalizes_on_silence_after_speech() {
        let mut segmenter = new_segmenter();
        let mut audio = speech(1_000);
        // A pause comfortably past the finalize threshold.
        audio.extend(silence(TEST_PAUSE_MS as usize + 400));

        let segments = feed(&mut segmenter, &audio);

        // 1 s of speech plus the trailing-silence marker, rounded up to the
        // chunk that crossed the threshold.
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].len(), 16_000 + silence_marker_len());
        assert!(segmenter.take_tail().is_none());
    }

    #[test]
    fn brief_pause_below_threshold_does_not_split_a_segment() {
        // The punctuation-on-pauses fix: a short hesitation (well under the
        // pause threshold) must NOT finalize a segment, so Whisper sees one
        // continuous phrase and does not punctuate the gap. With the old 350 ms
        // threshold this pause would have cut a segment and planted a sentence
        // break here.
        let mut segmenter = new_segmenter();
        let pause = silence(TEST_PAUSE_MS as usize / 2);
        let mut audio = speech(1_000);
        audio.extend_from_slice(&pause);
        audio.extend(speech(1_000));

        let segments = feed(&mut segmenter, &audio);

        assert!(
            segments.is_empty(),
            "a sub-threshold hesitation must not split the segment"
        );
        // It all flushes as one segment at stop: both speech runs plus the
        // hesitation between them, transcribed together.
        let tail = segmenter.take_tail().expect("tail contains speech");
        assert_eq!(tail.len(), RATE as usize * 2 + pause.len());
    }

    #[test]
    fn pause_disabled_never_cuts_on_silence() {
        // With pause-based cutting off (the user's "disable the pause" setting),
        // even a long silence must not finalize a segment — only the length cap
        // (large here) can.
        let mut segmenter = Segmenter::new(RATE, None, TEST_MAX_MS);
        let mut audio = speech(1_000);
        audio.extend(silence(3_000));

        let segments = feed(&mut segmenter, &audio);

        assert!(segments.is_empty(), "pause-disabled must not cut on silence");
        let tail = segmenter.take_tail().expect("tail contains speech");
        // The whole thing — speech and the long pause — stays one segment.
        assert_eq!(tail.len(), RATE as usize * 4);
    }

    #[test]
    fn length_cap_still_bounds_segments_when_pause_disabled() {
        // The safety cap is independent of pause cutting: a 2 s cap finalizes a
        // 2 s segment out of 3 s of continuous speech.
        let mut segmenter = Segmenter::new(RATE, None, 2_000);

        let segments = feed(&mut segmenter, &speech(3_000));

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].len(), RATE as usize * 2);
        let tail = segmenter.take_tail().expect("tail contains speech");
        assert_eq!(tail.len(), RATE as usize);
    }

    #[test]
    fn does_not_finalize_without_speech() {
        let mut segmenter = new_segmenter();
        let mut audio = quiet(2_000);
        audio.extend(silence(1_000));

        let segments = feed(&mut segmenter, &audio);

        assert!(segments.is_empty());
        assert!(segmenter.take_tail().is_none());
    }

    #[test]
    fn finalizes_at_hard_cap_during_continuous_speech() {
        let mut segmenter = new_segmenter();

        let segments = feed(&mut segmenter, &speech(27_000));

        assert_eq!(segments.len(), 1);
        // Finalized at the 25 s cap (the first chunk that crosses it).
        assert_eq!(segments[0].len(), RATE as usize * 25);
        // The rest stays buffered as the next segment's start.
        let tail = segmenter.take_tail().expect("tail contains speech");
        assert_eq!(tail.len(), RATE as usize * 2);
    }

    #[test]
    fn hard_cap_without_speech_keeps_buffer_bounded_and_discards() {
        let mut segmenter = new_segmenter();

        let segments = feed(&mut segmenter, &quiet(60_000));

        assert!(segments.is_empty());
        assert!(segmenter.samples.len() <= RATE as usize * 25 + CHUNK);
        assert!(segmenter.take_tail().is_none());
    }

    #[test]
    fn leading_silence_is_never_accumulated() {
        let mut segmenter = new_segmenter();
        let mut audio = silence(1_000);
        audio.extend(speech(1_000));
        audio.extend(silence(TEST_PAUSE_MS as usize + 400));

        let segments = feed(&mut segmenter, &audio);

        // The leading second of silence is skipped entirely: the segment is
        // just the speech plus the trailing-silence marker.
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].len(), 16_000 + silence_marker_len());
    }

    #[test]
    fn tail_flushes_speech_without_silence_requirement() {
        let mut segmenter = new_segmenter();

        let segments = feed(&mut segmenter, &speech(300));

        assert!(segments.is_empty());
        let tail = segmenter.take_tail().expect("tail contains speech");
        assert_eq!(tail.len(), 4_800);
        // The tail consumed the buffer; a second flush yields nothing.
        assert!(segmenter.take_tail().is_none());
    }

    #[test]
    fn joins_segments_with_single_spaces_dropping_empties() {
        let texts = vec![
            " Hello there. ".to_string(),
            String::new(),
            "   ".to_string(),
            "Second phrase.".to_string(),
        ];

        assert_eq!(join_segments(&texts), "Hello there. Second phrase.");
        assert_eq!(join_segments(&[]), "");
    }

    #[test]
    fn prompt_combines_vocabulary_and_context_tail() {
        assert_eq!(prompt_with_context("", ""), "");
        assert_eq!(
            prompt_with_context(" Scribe, Tauri ", ""),
            "Scribe, Tauri"
        );
        assert_eq!(prompt_with_context("", "previous text"), "previous text");
        assert_eq!(
            prompt_with_context("Scribe", "previous text"),
            "Scribe previous text"
        );
    }

    #[test]
    fn prompt_context_is_limited_to_the_last_200_chars() {
        let accumulated = "x".repeat(300);

        let prompt = prompt_with_context("vocab", &accumulated);

        assert_eq!(prompt, format!("vocab {}", "x".repeat(200)));
    }

    #[test]
    fn prompt_context_truncation_respects_char_boundaries() {
        let accumulated = "é".repeat(300);

        let prompt = prompt_with_context("", &accumulated);

        assert_eq!(prompt.chars().count(), 200);
        assert!(prompt.chars().all(|character| character == 'é'));
    }

    #[test]
    fn text_tail_returns_short_text_unchanged() {
        assert_eq!(text_tail("short", 200), "short");
        assert_eq!(text_tail("anything", 0), "");
    }
}
