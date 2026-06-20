# Scribe — Daily-Driver Reliability Pass (Overnight Report)

**Branch:** `reliability-pass`
**Date:** 2026-06-20
**Scope:** Audit the recording / transcription / clipboard / hotkey / settings flows; implement the
reliability items that were genuinely missing or broken; review two user concerns (excess punctuation
on pauses, and local-model "grading"). Preserve existing behavior; no UI redesign.

---

## TL;DR

**The headline finding is that Scribe already implements almost everything this pass asked for, and
implements it well.** Transcript history, "Paste Last Transcript" (command **+** global hotkey **+**
tray), clipboard-preserving output, sub-300 ms rejection, transcription-failure recovery, persistent
settings, and file logging all already exist and are tested. So this turned into an **audit → verify →
fill the real gaps** pass rather than a build-from-scratch pass.

I found and fixed **5 genuine reliability holes**, added tests, and ran an adversarial multi-agent
review of the diff (no blockers; 4 low-severity findings, the worthwhile ones fixed, the rest recorded
below). For the two items flagged as "review, not code": **local-model grading** stays a write-up
([§7](#7-review-should-a-local-model-grade-while-we-dictate)); **punctuation**
([§6](#6-review--fix-too-much-punctuation-when-i-pause)) was reviewed **and then fixed** at the user's
request — the incremental segment pause threshold was raised 350 ms → 3 s and, with the length cap,
promoted to tunable user settings (commits `0a74ffc`, `5972fa2`).

**Build status:** `cargo test --lib` → **183 passed, 0 failed** on Linux/WSL (was 171; +12 new).
TypeScript `tsc --noEmit` clean. ⚠️ One important caveat: the Windows-only audio code can't be
compiled on this Linux box — see [§5](#5-verification--what-could-not-be-checked-here).

---

## 1. Method

Scribe targets Windows; the dev box is Linux/WSL, where the entire `#[cfg(windows)]` surface (WASAPI
capture, Win32 paste/clipboard, global hotkeys) compiles to no-op stubs. So before writing anything I
ran a 10-way parallel audit of every relevant flow (audio, transcription, output/clipboard, hotkeys,
settings, history, punctuation, logging/state, local-LLM infra, build/test harness), then read the
core files firsthand to confirm. That audit is what surfaced "most of this already exists" — which
changed the job from *implement* to *verify + fix the few real gaps*, exactly as the prompt's
"preserve existing behavior unless clearly broken" intended.

---

## 2. What already existed (verified, not rebuilt)

| Requested item | Status | Evidence |
|---|---|---|
| **Transcript history** | ✅ Already complete | SQLite `transcripts` table, `save_last_transcript_with_history` (`db.rs`), full search/filter/sort/paginate/delete History UI (`views/History.tsx`). Time-based retention (7/30/90/365/off). |
| **Paste Last Transcript** | ✅ Already complete (cmd **+ hotkey** + tray) | `paste_last_transcript` command, `HotkeyAction::PasteLastTranscript` (default **Ctrl+Alt+V**), tray menu item — all read the persisted "Last Transcript Buffer". |
| **Clipboard-preserving output** | ✅ Already sophisticated | Default paste snapshots **all** clipboard formats (text/image/files) as raw Win32 bytes, sends Ctrl+V, then restores; honest `ClipboardPreservation` reporting (`Untouched`/`RestoredAfterPaste`/`RestoreFailed`). DirectInsert mode never touches the clipboard. (`output.rs`) |
| **Reject sub-300 ms recordings** | ✅ Already present, exactly 300 ms | `min_recording_ms` default **300** (`settings.rs:567`), enforced in `finalize_recording` (`audio.rs:951`) → `RecordingResultStatus::TooShort` → `AudioTooShort` → Idle. Configurable; `validate()` forbids 0. |
| **Safe recovery after transcription failure** | ✅ Already present | 8-state FSM (`app_state.rs`); any transcription error → `Transcribing→Error`, which **self-heals to Idle after 5 s** and is also recoverable on the next toggle. Empty result → benign `Idle`. (`dictation.rs`) |
| **Persistent settings** | ✅ Already present | Validated JSON in a SQLite row; atomic single-statement upsert; comprehensive `#[serde(default)]` + `defaults_version` migrations (`settings.rs`, `db.rs`). |
| **Useful local logs** | ⚠️ Present but crippled | `tauri-plugin-log` → log dir + stdout… but on plugin defaults (see Fix B). |
| **Mic disconnect / reconnect** | ⚠️ Partial | Reconnect **between** recordings works (each start re-enumerates; a vanished selected mic fails cleanly). Mid-recording disconnect was **not handled** (see Fix D). |
| **State-transition tests** | ✅ Already present | `app_state.rs` had 6; whole crate had 171. |

**Conclusion:** I deliberately did **not** re-implement any of the above. Rebuilding working,
tested features would have been pure risk. The real work was the gaps below.

---

## 3. What I changed (5 fixes + 1 small UI affordance)

Each fix is a separate commit on `reliability-pass`.

### Fix A — Corrupt settings no longer make the app unlaunchable  · `caea821`
`get_settings()` returned a hard error on any deserialize failure. At startup that error propagates
through `db.get_settings()?` in `setup()` and `run()`'s `.expect()` — i.e. **one corrupt or
half-written settings value bricked the app with no recovery.** Now a deserialize failure backs the
bad value up under a fixed `app_settings_corrupt` key, logs it, and falls back to (and persists)
`AppSettings::default()` — mirroring the existing missing-row path. *File:* `db.rs`. *Test:*
`corrupt_settings_self_heal_to_defaults_and_are_backed_up`.

### Fix B — "Useful local logs" actually useful  · `0a11107`
The log plugin ran on defaults: **40 KB max, `KeepOne`** → the log self-deleted every ~40 KB and kept
no history. Under normal dictation it churned past several sessions within minutes, so a user
reporting "it failed an hour ago" had nothing. Now: **5 MB × `KeepSome(5)`**, timestamps in **local
time**. *File:* `lib.rs`.

### Fix C — Panic hook so crashes reach the log  · `0a11107`
`run()` installed no panic hook, so a panic on any spawned thread (audio worker, timeout thread,
hotkey chord/toggle watchers, Drive workers) died silently. Now a hook (chaining the previous one, so
the stderr backtrace is preserved) logs the thread name + location + payload via `log::error!`.
*File:* `lib.rs`.

### Fix D — Recover when the mic is unplugged mid-recording  · `5c88473`  *(Windows-only)*
**The one genuine functional wedge.** The cpal stream-error callback only did `log::error!`. On a
mid-recording disconnect (USB/Bluetooth mic, WASAPI endpoint dies), the worker blocked forever on the
dead chunk channel and the app stayed "Recording" until the user stopped or the **max-duration timeout
(up to ~10 min)** fired — capturing silence with no feedback. Now the error callback, **exactly once**
(`AtomicBool` guard), spawns a thread that drives `disconnect_recording_for_app`:

1. stop the session via a new `StopReason::Disconnected` (finalized like a normal completion, so
   whatever was captured **before** the drop is still transcribed — no lost words — but with no
   stop-grace sleep since the device is gone);
2. emit a `microphone_unavailable` `audio://recording-error` (the frontend already listens for this
   and has friendly copy, so the user gets a toast — no frontend change needed);
3. transcribe — reusing the **exact** path the max-duration timeout already uses, so the FSM never
   strands in Recording/Transcribing.

Also made the `StopRecording` transition tolerant of a concurrent stopper, removing a rare spurious
"invalid state transition" toast (see Risk R4). *File:* `audio.rs`. Decision: **no auto-reconnect
mid-recording** (could capture from the wrong device / race) — reconnect-for-next-recording already
works.

### Fix E — Don't delete the user's audio when transcription fails  · `15329ae`
`transcribe_recording_checked` deleted the temp WAV on **every** path including failure — so a
transient whisper failure (server + CLI both down, corrupt model) destroyed the only copy of the
user's speech. Now the WAV is **quarantined** into an app-data `failed/` folder (move, copy+delete
fallback), fully best-effort (the happy path and the returned result are never affected), with
`prune_failed_recordings` capping the folder at 20 clips. Recoverable from disk. *File:* `dictation.rs`.
*Tests:* the three `prune_failed_recordings_*` tests.

### Small UI affordance — reachable logs  · `0a11107`
Added `open_logs_folder` / `get_logs_dir` commands and **one row** ("Local logs folder") in the
existing Data & Privacy → Folders accordion, so users can actually find logs to attach to a bug
report. Not a redesign — it matches the existing data/models folder rows. *Files:* `commands.rs`,
`backend.ts`, `views/DataPrivacy.tsx`.

---

## 4. Tests added (9 new; 171 → 180)

| Test | What it locks down |
|---|---|
| `db::corrupt_settings_self_heal_to_defaults_and_are_backed_up` | Fix A: corrupt JSON → defaults + single backup row + stable on reload |
| `dictation::prune_failed_recordings_caps_the_folder_and_spares_non_wavs` | Fix E: caps at `keep`, ignores non-WAVs |
| `dictation::prune_failed_recordings_is_a_noop_under_the_cap` | Fix E: under cap = untouched |
| `dictation::prune_failed_recordings_tolerates_a_missing_folder` | Fix E: best-effort, no panic |
| `app_state::pasting_flow_round_trips_back_to_ready_then_idle` | Pasting flow coverage |
| `app_state::pause_and_resume_round_trip` | Pause/Resume + reject-while-paused |
| `app_state::disconnect_recovery_reuses_the_normal_stop_to_transcribe_sequence` | Fix D's event sequence is a legal FSM path |
| `app_state::illegal_transition_is_rejected_and_leaves_state_unchanged` | Invalid edges don't corrupt state |
| `incremental::brief_pause_below_threshold_does_not_split_a_segment` | §6 punctuation fix: a sub-threshold hesitation no longer cuts a segment |

The mic-disconnect *wiring* itself (Fix D) is `#[cfg(windows)]` and not unit-testable on Linux; the
test instead pins that the recovery's **state sequence** (`Recording → StopRecording → ValidAudio →
Transcribing → success → Ready`) is one the FSM accepts.

---

## 5. Verification — and what could NOT be checked here

- ✅ `cargo test --lib` → **179 passed, 0 failed** (Linux/WSL).
- ✅ `cargo build --lib` clean; `npx tsc --noEmit` clean.
- ✅ Adversarial multi-agent review of the full diff — no blockers, no compile errors found in the
  Windows code; the cpal closure bounds, the non-`Copy` closure moved across the sample-format match
  arms, the cpal 0.15 API, and the absence of double-stop/double-transcribe/deadlock were all
  verified by reading the APIs.
- ⚠️ **The `#[cfg(windows)]` code (all of Fix D) was never compiled.** `cargo test` on Linux builds
  only the `#[cfg(not(windows))]` arms. I tried to cross-check with
  `cargo check --target x86_64-pc-windows-msvc`, but it fails in a dependency (`ring`/rustls-tls
  needs the MSVC archiver `lib.exe`, not installable here). **Fix D must be compiled on a Windows box
  (or CI) before shipping.** I'm confident in it from review, but "compiles on Windows" is unverified
  by me. Strongly recommend adding a CI step: `cargo check --target x86_64-pc-windows-msvc` so the
  Windows arms are type-checked on every push (today they never are).

---

## 6. Review + fix: "too much punctuation when I pause"

**This is a real, explainable behavior with two compounding root causes.** Reviewed below; the
dominant cause (root cause 1) was then **fixed** at the user's request — see "Implemented".

### Root cause 1 — the incremental segmenter manufactures sentence breaks at every pause *(the big one)*
Incremental transcription is **ON by default** (`incremental_transcription_enabled`,
`settings.rs:572`). It cuts a new audio segment after just **350 ms** of silence
(`SEGMENT_SILENCE_MS`, `incremental.rs:40`), transcribes each phrase as a **standalone clip**, and
`join_segments` (`incremental.rs:175`) joins them with a space. Whisper closes each standalone phrase
with sentence-final punctuation, so a pause while you think becomes:

> "I want to go **.** To the store." instead of "I want to go to the store."

`prompt_with_context` (`incremental.rs:187`) then feeds the *punctuated* tail of the accumulated text
back as the next segment's prompt, which further biases Whisper toward that punctuated style.

### Root cause 2 — Whisper runs at default decode params
`whisper_args` (`whisper.rs:133`) and `server_args` (`whisper_server.rs:528`) pass only
model/file/language/output/threads/translate/prompt — **no** temperature, no-speech threshold, or
non-speech-token suppression. At defaults, whisper.cpp readily emits commas/periods/ellipses during
pauses and near-silent gaps. Post-processing (`normalize_transcript_text`, `whisper.rs:216`) strips
pause **ellipses** and `[BLANK_AUDIO]` markers but does nothing about stray single commas/periods.

### Options (lowest-risk → highest)
1. **Immediate, zero-code mitigation you can try tonight:** turn **off** incremental transcription
   (Settings → Audio). Full-clip mode hands Whisper the *whole* utterance at once, so it punctuates
   far less aggressively at pauses. Costs a little stop-to-text latency. This is the single most
   effective lever today and is the honest first thing to try.
2. **Segmenter tuning (deterministic, testable):** raise `SEGMENT_SILENCE_MS` from 350 → ~700–900 ms
   so brief thinking-pauses don't split a phrase; strip trailing sentence-final punctuation from
   *non-final* segments before `join_segments`; and stop feeding the punctuated tail back through
   `prompt_with_context`. Combining the last two is the cleanest deterministic fix.
3. **Whisper decode flags:** add `--no-fallback` (disables temperature fallback — removes most
   pause-time hallucinated tokens), raise the no-speech threshold (`--no-speech-thold` ≈ 0.6–0.8 so
   quiet gaps are dropped), and on recent builds `--suppress-nst`. ⚠️ Two caveats: the exact flag
   spellings differ across whisper.cpp versions and must be verified against the **bundled**
   `whisper-cli.exe`/`whisper-server.exe`; and `server_args` must mirror whatever the CLI uses or the
   warm-server path and CLI fallback will punctuate differently. Trade-off: `--no-fallback` can
   slightly raise word-error on hard audio; a higher no-speech threshold can clip genuinely quiet
   speech.
4. **Post-processing backstop:** extend `normalize_transcript_text` to collapse runs of identical
   punctuation and drop leading stray punctuation — but keep it conservative and unit-tested (must not
   eat decimals like `3.14`, list commas, or quoted ellipses). Treat as a backstop, not the primary
   fix.
5. **The proper fix already in the app, just off:** the optional local-LLM **dictation cleanup**
   (`dictation_cleanup_enabled`, default off) explicitly "fixes punctuation… merges run-ons." It's the
   highest-quality fix but adds latency and requires a local LLM server (LM Studio/Ollama).

**Implemented (commits `0a74ffc`, `5972fa2`):** option 2's core lever, then promoted to **user
settings**. The incremental segmenter's pause threshold and length cap are no longer compile-time
constants — they're per-recording `AppSettings` fields wired through `start_session` into the
`Segmenter`:
- **`segment_pause_ms`** (default **3 s**, range 0.2–10 s) — pause length that splits a phrase. The
  default moved 350 ms → 1 s → **3 s** so segments line up with deliberate sentence-ending pauses
  instead of mid-sentence hesitations (far fewer manufactured periods/commas).
- **`segment_pause_enabled`** (default on) — turn pause-based splitting **off** entirely; segments then
  end only at the length cap.
- **`segment_max_ms`** (default 25 s, range **10–25 s**) — max phrase length, clamped to Whisper's
  ~30 s window so a segment can never be truncated and lose words ("off" = 25 s, the safe max).

UI: three rows under "Live transcription" in Audio settings (no redesign). Cutting stays pause-based (a
fixed time slice would cut mid-word). Fully unit-tested on Linux (pause-disabled cases, default values,
validation ranges). Trade-off documented in-UI: a longer pause = fewer breaks, slightly more
stop-to-text latency. **Not yet done** (further options if 3 s isn't enough): strip trailing punctuation
from non-final segments / stop biasing on punctuated context (option 2 remainder), or the whisper decode
flags (option 3, needs flag-spelling verification against the bundled binary).

---

## 7. Review: "should a local model grade while we dictate?"

**Feasible, and you're closer than it sounds — but the cheap version and the expensive version are
very different.** (Review only; no code changed.)

### What exists today
- A **local-LLM integration already exists**, but it's an **external** OpenAI-compatible HTTP client
  (`note_analysis.rs`), default endpoint **LM Studio `http://127.0.0.1:1234/v1`** (Ollama-compatible),
  reused three ways: optional dictation cleanup, on-demand notes analysis, and selection transform.
  It is **not** an embedded model — it depends on the user running LM Studio/Ollama, and `llm_status`
  already health-checks reachability.
- Whisper runs locally and **already computes** per-segment confidence (`avg_logprob`,
  `no_speech_prob`, `compression_ratio`) — but the app **throws it away**: the warm server is asked
  for `response_format=json` with a `{ text }`-only response struct (`whisper_server.rs:369`), and the
  CLI uses `--output-txt`. **The cheapest possible "grade" is being discarded.**
- There is no scoring/quality field anywhere; `Transcript`/`DictationResult` carry latency but no
  confidence.

### Two realistic shapes
| | **Option 1 — Whisper's own confidence (recommended)** | **Option 2 — a second LLM "grader"** |
|---|---|---|
| How | Switch the server to `response_format=verbose_json` (or CLI `--output-json`), parse `avg_logprob`/`no_speech_prob` per segment, aggregate to a 0–100 score, threshold/flag | Send the transcript to the external LLM to rate it |
| Latency | **~free** — Whisper already produced the numbers | hundreds of ms → several seconds **per dictation** on modest hardware |
| Dependency | none (pure parse/threshold logic, unit-testable on Linux) | requires the user to be running LM Studio/Ollama |
| "While we do it" | ✅ attaches in the incremental coordinator per segment | ⚠️ per-segment multiplies cost; serializes behind the transcriber |
| Risk to the product | low | conflicts with Scribe's core value: instant stop-to-paste |

### Recommendation
If you want grading, do **Option 1**: surface Whisper's own confidence (e.g. a subtle indicator, or a
"low confidence — re-record?" hint on a bad clip) and persist an optional `confidence` field on the
transcript. It's the honest answer to "grade what we're doing while we do it," costs ~nothing, and is
fully testable. **Avoid** a real-time second-LLM grade: it's slow, depends on an external server that's
often not running, and undercuts the instant-paste experience. If a deeper LLM critique is ever wanted,
make it strictly **post-hoc, advisory, and non-blocking** (mirror the cleanup pass's "never blocks,
never blanks" contract), gated behind `llm_status` + off by default — never per incremental segment.

---

## 8. Remaining risks & known limitations

| | Risk | Severity | Disposition |
|---|---|---|---|
| **R1** | **Fix D is unverified on Windows.** The `#[cfg(windows)]` code couldn't be compiled here (`ring` needs MSVC). | Process | **Compile on Windows / add the MSVC `cargo check` CI step before shipping.** |
| **R2** | Quarantined `failed/` WAVs (Fix E) are recoverable only by browsing the folder — no in-app "retry" UI. The code/log wording was softened to say "recoverable from disk," not "retry," to avoid over-promising. | Low | As designed. Optional follow-up: an "open failed-recordings folder" affordance or a retry-through-`file_transcribe` path. |
| **R3** | Corrupt-settings self-heal (Fix A) resets **all** settings on **any** serde failure — fine for genuine corruption, but a *future* field rename/retype (or downgrading to an older binary) could trip it and silently reset customizations (the old value is kept in the backup row but not surfaced in-UI). It replaces a hard startup crash, so it's strictly better, but the blast radius is wide. | Low–Med | **Recommended follow-up:** add `#[serde(default = …)]` to the ~17 original `AppSettings` fields that lack it (each needs the *correct* default, e.g. `min_recording_ms` → 300, **not** the type default 0), so schema evolution degrades per-field instead of resetting; and emit a one-time user-visible notice on reset. |
| **R4** | Mic-disconnect adds a **third** concurrent stopper (alongside the timeout thread and manual stop) to a pre-existing benign FSM race. Worst case in a sub-millisecond unplug+timeout coincidence: the FSM lands `Idle` instead of `Ready`, **but the transcript is still produced** and the state self-heals. Fix D's tolerant `StopRecording` transition removes the spurious toast this used to cause. | Low | Mitigated; residual is cosmetic. Fully fixing the race means making the FSM transition + `active.take()` atomic (out of scope). |
| **R5** | No watchdog on `Transcribing`/`Stopping`. If whisper hangs, the app waits on the transcriber's internal `INFERENCE_TIMEOUT` (**300 s**) before failing over. Bounded (not an infinite wedge), but a long silent stall in a pathological case. | Low | Left as-is to avoid scope creep; noted. A `Transcribing`-entry watchdog (mirroring the 5 s Error self-heal) would tighten it. |
| **R6** | Sub-300 ms rejection is measured on **raw** capture length; `trim_silence` runs afterward and can shorten the saved WAV below 300 ms for a borderline clip. Harmless (whisper handles short clips; empty output is benign). | Info | Document that `min_recording_ms` gates raw capture duration, not post-trim speech length. |

---

## 9. Manual test steps (Windows — needed because Fix D isn't testable here)

> Run a Windows build (`npm run tauri build` or dev). The mic-disconnect path **must** be exercised
> on real hardware.

1. **Mic unplug mid-recording (Fix D — the important one).** Start a dictation (toggle hotkey), speak
   a sentence, then **unplug the mic** (or disable it in Sound settings) while still "recording."
   - *Expect:* within a moment the app leaves "Recording", you get a toast that the microphone
     stopped/was disconnected, and whatever you said **before** unplugging is transcribed & pasted.
   - *Expect NOT:* the app stuck on "Recording" for minutes, or no transcript.
   - Reconnect the mic and start a new dictation — it should work normally.
   - Check the log (Data & Privacy → **Local logs folder**) for `Audio stream error` + the recovery line.
2. **Reconnect between recordings.** With a USB mic selected, unplug it, then start a recording →
   expect a clean "microphone unavailable" error (not a crash). Replug, start again → works.
3. **Corrupt settings self-heal (Fix A).** Close Scribe. With a SQLite tool, set
   `settings.value` for `key='app_settings'` to `{garbage`. Relaunch.
   - *Expect:* app launches normally on default settings; a row `key='app_settings_corrupt'` holds
     your garbage; the log notes the reset. *Expect NOT:* app fails to start.
4. **Failed-transcription audio kept (Fix E).** Force a transcription failure (e.g. delete/rename the
   selected model files, or point the model path at a corrupt file), then dictate.
   - *Expect:* an error toast, the app self-heals to Idle, **and** a WAV appears in
     `%APPDATA%\…\<id>\failed\`. Restore the model; confirm normal dictation deletes its temp WAV
     (no leak).
5. **Logs retention (Fix B).** Dictate many times; confirm the log dir keeps **several** rotated files
   (not a single ~40 KB file) and timestamps are in **local** time.
6. **Panic capture (Fix C).** (If you can trigger a thread panic in a debug build) confirm a
   `PANIC on thread '…' at …` line appears in the log.
7. **Regression sweep (existing features, confirm unchanged):** Paste Last Transcript (Ctrl+Alt+V),
   clipboard restore after paste, sub-300 ms tap produces no transcript, history saves, settings
   persist across restart.

---

## 10. Recommended follow-ups (not done this pass)

1. **CI: `cargo check --target x86_64-pc-windows-msvc`** (or a Windows runner) so the `#[cfg(windows)]`
   code is type-checked on every push. Today it never is — this pass is the proof (Fix D shipped
   unverified-by-compile).
2. **Punctuation:** the 350 ms → 1 s threshold (commit `0a74ffc`) is in. If 1 s isn't enough, add the
   remaining §6 option-2 work (strip non-final-segment punctuation) and/or expose it as a settings
   slider; verify decode flags (option 3) against the bundled whisper binary.
3. **Settings robustness (R3):** add per-field `#[serde(default = …)]` to the original `AppSettings`
   fields + a user-visible "settings were reset" notice.
4. **Confidence/grading (§7):** capture Whisper's `verbose_json` confidence and surface it.
5. (Optional) an "open failed-recordings folder" affordance for R2.

---

*Generated by Claude Code (Opus 4.8, 1M context) during an overnight reliability pass. Code changes are
on the `reliability-pass` branch: `caea821` (settings self-heal), `0a11107` (logs), `5c88473` (mic
disconnect), `15329ae` (failed-recording quarantine + tests), `0a74ffc` + `5972fa2` (punctuation:
segment pause/cap as user settings). The working tree is otherwise clean. (Note: mid-session the repo
was relocated from `public/scribe` to `projects/tools/scribe/scribe-app`; all commits carried over and
a `cargo clean` cleared stale build paths.)*
