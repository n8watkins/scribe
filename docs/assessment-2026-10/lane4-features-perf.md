# Lane 4: features, performance and reliability

Assessed 2026-10-27 on `main` at `1d88fe9` (v0.8.0 plus 15 unreleased commits).
Method: code reading of `app/src-tauri/src` and `app/src`, plus read-only evidence from the owner's installed v0.8.0 (a copy of `scribe.sqlite3` and `Scribe.log` under `%APPDATA%\com.natkins.scribe` and `%LOCALAPPDATA%\com.natkins.scribe`).
Only aggregate numbers were taken from that data; no transcript text was read into this report.
No builds or tests were run in this lane (Lane 1 owns them).

## Summary

The core loop is fast and well engineered: incremental transcription keeps stop-to-text latency flat regardless of recording length (median 432 ms, p90 749 ms over 1,273 logged dictations, with no growth from 3 s to 10 min clips).
The perceived key-release-to-text time is about 1 second, and roughly 40% of it is a fixed 400 ms "stop grace" sleep plus a 60 ms clipboard settle, not Whisper.
The app's own "latency" stat leaves both out, so it under-reports what users feel by about half.

The most serious problems are not speed but correctness and storage:

- Transform Selection (Ctrl+Alt+R) reads the clipboard after a synthetic Ctrl+C without checking that anything was copied, so with nothing selected it rewrites whatever was already on the clipboard and pastes it at the cursor; in a terminal the synthetic Ctrl+C also interrupts the running process.
  It is bound even when the AI features are off (open problem 3 in HANDOFF.md: confirmed).
- The default toggle key (Backquote, no modifiers) is a system-wide `RegisterHotKey`, so the backtick is swallowed in every app, and on non-US layouts the same virtual key is a letter (for example Ö on German).
  The Ctrl+Alt defaults collide with AltGr characters on many European layouts.
- Audio clips are stored as uncompressed 16 kHz WAV: the owner's data folder holds 1.2 GB of clips, 505 MB of which are orphans left by the LocalDictate rebrand migration that nothing will ever delete.
  At the owner's real usage (372 minutes of audio in five days) the default 30-day retention holds about 4 GB.
- A transcript is pasted only after it is saved; if the save fails (disk full, DB error) the text is neither saved nor pasted.
- Paste into an elevated (admin) window fails silently under UIPI, the clipboard is then restored, and the app reports success.
- A selected microphone that is unplugged makes every dictation fail with an error instead of falling back to the default device.
- "Paste method: type it out" has no control in the UI (open problem 2: confirmed), and `recording_mode` is still inert.

## Feature verification

Status values: Works (code path complete and wired to UI), Partial, Broken, Not wired.
"Works" here means verified by code reading; behaviors that need a real Windows run are marked as such.

| Feature (from the plan's inventory) | Status | Evidence | Notes |
| --- | --- | --- | --- |
| Hold Ctrl+Win to talk | Works | `settings.rs:578`, chord watcher `hotkeys.rs:1258-1330` | Polled with `GetAsyncKeyState` every 30 ms with a 150 ms arming delay; audio starts after arming, so the first ~200-400 ms of speech can be clipped. |
| Tap Backquote for hands-free | Works, with a serious side effect | `settings.rs:579`, watcher `hotkeys.rs:529-554`, VK map `hotkeys.rs:598` | Registered as a bare global hotkey, so the backtick key stops typing system-wide; layout issue below. |
| Tap Q while toggle is held for a quick note | Works | `hotkeys.rs:1013-1075` | Q is grabbed only while the toggle key is down; log shows one "could not grab Q" conflict. |
| Ctrl+Alt+X discard, Ctrl+Alt+V paste last, Ctrl+Alt+F dashboard | Works | `settings.rs:526,580-581` | Log shows registration conflicts for Ctrl+Alt+V, Ctrl+Alt+F and Ctrl+Alt+R on the owner's machine; the user is notified via `emit_registration_failures` (`hotkeys.rs:187`). |
| Every hotkey rebindable, press or release, conflicts flagged | Works | `hotkeys.rs:690-790`, `validate_hotkeys` `hotkeys.rs:665` | |
| Live transcription with split-on-pause | Works | `incremental.rs:1-64`, defaults pause 3000 ms, cap 25000 ms | Partials are emitted as `scribe:partial-transcript`; insertion still happens once at the end (`dictation.rs:380`). |
| Auto-stop after 60 s silence (hands-free) | Works | `audio.rs:310-314`, `AUTO_STOP_SPEECH_RMS` `audio.rs:46` | Hold-to-talk is never auto-stopped, by design. |
| Silence trim | Works | `audio.rs:50-60` | |
| Recording length 0.3 s to 10 min | Works | `audio.rs:286-289`, `audio.rs:790-815` | 7 of 1,409 logged recordings hit the 10 min cap and were cut (`Timeout`). |
| Output: auto-paste, copy, copy and paste, save only | Works | `output.rs` | |
| Clipboard paste that restores the clipboard exactly | Works, with caveats | `output.rs:342-400` | Restores all formats, but every transcript also lands in Windows clipboard history (Win+V) and cloud clipboard because the exclusion formats are not set. |
| Type-it-out paste method | Not wired in UI | `PasteMethod` in `backend.ts:81`, used only in `lib/format.ts:57`; no control in any view | Open problem 2 confirmed. The owner's DB holds 5 transcripts with `direct_insert`, so it was reachable in an earlier build. |
| Floating pill (dot, waveform, waveform with text), draggable, colors, sounds | Works (code) | settings `pillDisplayMode`, `pillX/Y`, `pillColor*` | Needs a Windows visual check (Lane 3). |
| Dictionary hint | Works | `vocabulary_prompt` passed per request, `whisper_server.rs:10-12` | |
| "Say X, get Y" replacements | Works | `text_replace.rs`, applied at `dictation.rs:281` | Not applied to file transcription (intentional, `dictation.rs:278`). |
| Filler removal [off] | Works | `filler.rs`, `dictation.rs:247` | Runs on the warm server path too. |
| Transform selection (Ctrl+Alt+R) | Broken in two cases | `selection_transform.rs:98-121`, `hotkeys.rs:1100-1140` | Stale clipboard is used as the "selection"; Ctrl+C is sent into terminals; bound with AI off. Details in findings F1 and F2. |
| 11 Whisper models, SHA-256 checked | Works | `models.rs:4` (default `small.en-q5_1`), `model_manager.rs:500-590` | Hash is verified on download and on select, not per dictation (good). Select saves the setting before verifying the hash (`model_manager.rs:249-253`), so a failed check still leaves the model selected. |
| Model kept loaded for speed | Partial | `whisper_server.rs:40` | Only for 10 minutes; then the server is stopped and the next dictation pays a cold start. No pre-warm when recording starts. |
| GPU (Vulkan) with CPU fallback and picker | Works | `whisper_server.rs:680`, `gpu.rs` | The owner runs with GPU off. |
| Auto-detect plus 29 languages, translate to English | Works (code) | `dictation.rs:520`, `settings.rs` | Language and translate are launch arguments, so changing either restarts the server (`whisper_server.rs:669`). |
| Transcribe a file (WAV, MP3, FLAC, OGG; MP4 etc. with ffmpeg) | Partial | `file_transcribe.rs:62-160` | ffmpeg is not bundled; it must be on PATH (`Command::new("ffmpeg")`, `file_transcribe.rs:139`), with a NotFound message at `:149`. |
| Microphone picker, test clip | Works | `audio.rs:1270-1360`, `audio.rs:591-614` | Selected mic missing is a hard error (F7). |
| Last 20 failed recordings kept for retry | Partial | `dictation.rs:116-197` | Kept on disk under `failed/`; there is no in-app retry command among the registered commands (`lib.rs:418-480`), only "open folder". |
| Audio clips saved for playback [on] | Works | `dictation.rs:318-330` | Uncompressed WAV; storage section below. |
| History: search, filters, sort, copy, open, play, delete, combine | Works | `db.rs:167-320`, commands in `lib.rs:421-436` | Search is a `LIKE` scan; fine at current sizes. |
| Retention 30 days / forever / off | Works for rows | `db.rs:810-860`, run on every save (`db.rs:445`) and at startup (`lib.rs:330`) | Clip files are deleted by the path stored in the row, which is why rebrand-copied clips were orphaned (F4). |
| Notes view, kept forever, AI summary via local LLM | Works | `note_analysis.rs`, `commands.rs:801` | 180 s request timeout (`note_analysis.rs:14`). |
| Stats | Partial | `db.rs:594-640` | "Today" is computed in UTC (`db.rs:595,612`), so it resets at 4-5 pm in US Pacific; "Total words" sums the current history and shrinks as retention prunes; the `app_stats_daily` table is never written (0 rows in the owner's DB). |
| GitHub backup, export, SHA-256-checked restore | Works (code) | `github_backup.rs`, `note_sync.rs`, `export.rs`, `import.rs` | No rate-limit (403/429) handling or retry; see reliability. |
| Seven themes, default window size | Works (code) | `tauri.conf.json` default 960x680, `save_window_size` | Visual check is Lane 3. |
| Tray, launch at startup, notifications | Works | `tray.rs`, autostart reconcile `lib.rs:334-357` | |
| Updates at launch, on refocus, every 6 h; install at launch only | Works | `App.tsx:582-591` | 169 update-check failures in the log, almost all on 2026-06-23/24 (offline or the earlier 1-minute cadence). |
| State server, CLI, MCP | Works | `state_server.rs`, `status_file.rs`; per-flavor discovery file `state_server.rs:449-461` | |
| `recording_mode` | Not wired | `settings.rs:75,363`; only typed in `backend.ts:138` | Inert, as the plan said. |
| Developer panel | Hidden | | Opt-in; not reviewed further here. |

## Hot path latency: key release to text in the target app

Measured from the owner's install (model `base.en`, CPU, GPU off):

- Stop-to-text as logged by the incremental path: median 432 ms, p90 749 ms, max 4,934 ms (n = 1,273, `Scribe.log`).
- `transcription_latency_ms` stored per transcript: median 448 ms, p90 647 ms, max 3,584 ms (n = 224, current history).
- Latency does not grow with length: medians of 456, 443, 514 and 418 ms for clips under 3 s, 3-10 s, 10-30 s and over 30 s.
  Incremental transcription is doing its job.
- whisper-server cold start (spawn plus model load): median 541 ms, max 2,176 ms (n = 142) for `base.en`; larger models will be several seconds.

Step by step, with the constants that set each cost:

| Step | Cost | Where |
| --- | --- | --- |
| Release detected (Ctrl+Win chord) | 0-30 ms poll | `POLL_INTERVAL` `hotkeys.rs:1258` |
| Release detected (toggle key) | 0-10 ms poll | `hotkeys.rs:544` |
| Stop grace: keep recording to catch the last word | 400 ms fixed sleep | `STOP_GRACE_MS` `audio.rs:58`, slept at `audio.rs:425` |
| Stop stream, join worker, finish WAV and trim | ~10-50 ms (estimate) | `audio.rs:427-433` |
| Wait for the tail segment and assemble | median 432 ms, p90 749 ms (measured) | `incremental.rs:64` (15 s cap), `dictation.rs:460` |
| Replacements, clip move, DB insert, retention sweep | ~5-30 ms (estimate) | `dictation.rs:281-345`, `db.rs:430-450` |
| Clipboard snapshot and set | ~5-20 ms (estimate; larger with an image on the clipboard) | `output.rs:365-369` |
| Pre-paste settle | 60 ms fixed sleep | `output.rs:371` |
| Wait for modifiers to be released | 0 ms normally, up to 1,500 ms if still held | `output.rs:671` |
| Send Ctrl+V | ~1 ms | `output.rs:375` |
| **Text appears** | **about 0.95 s median, about 1.3 s p90** | |
| Post-paste settle, then clipboard restore | 180 ms (after the text is visible) | `output.rs:377` |
| Cold server after 10 min idle | +0.5-2.2 s for `base.en`, more for bigger models | `IDLE_SHUTDOWN` `whisper_server.rs:40` |

Press side (time before audio is captured): 150 ms arming plus up to 30 ms polling for the Ctrl+Win chord (`hotkeys.rs:1259`), then a fresh WASAPI host, full device enumeration and endpoint-ID lookup on every start (`audio.rs:1270-1320`), then stream open.
Estimated 250-400 ms during which speech is lost if the user starts talking as they press; there is no pre-roll buffer.

What the "latency" stat measures: `stopped` is taken in `transcribe_recording_for_app` (`dictation.rs:62`), after the 400 ms grace and worker join, and it ends before output.
So Stats shows about 450 ms while users wait about 950 ms.

Best levers, in order:

1. Replace the fixed 400 ms grace with an adaptive one: stop immediately when the last 100-150 ms before release is already below `SILENCE_RMS_THRESHOLD` (the common case), and only wait the full grace when speech is still in progress.
   Saves up to 400 ms on most dictations.
2. Pre-warm whisper-server on recording start (call `ensure_server` from the press edge on a background thread) and make the idle shutdown configurable or longer (for example 60 min, or "while plugged in").
   Removes the 0.5-2+ s cold start on the first dictation after a break.
3. Keep the input stream open (or open it on press without re-enumerating devices) and keep a 300-500 ms pre-roll ring buffer, so speech at the moment of the press is not clipped.
4. Paste before saving: run output first, then save and clip-move (which also fixes F6).
5. Report true end-to-end latency (release to Ctrl+V sent) in Stats, alongside model time.

## Startup and memory

Startup (`lib.rs:228-410`) runs synchronously in Tauri's `setup` before the app is usable:

- Rebrand migration check: cheap once the `.rebrand-migrated` marker exists (`lib.rs:65-68`).
- Open SQLite and run migrations (`db.rs:33-37`, `db.rs:900-922`).
  Migrations are "run the ALTER and ignore duplicate column" rather than a versioned `user_version`; fine today, fragile for the next non-additive change.
- Retention sweeps (`lib.rs:330-331`), which delete clip files from disk on the startup path.
  With thousands of expired clips (first launch after a long break) this is disk work before the UI is ready.
- Autostart registry reconcile, state server bind, note-sync worker spawn, hotkey registration and watcher threads, tray.
- Nothing heavy blocks first paint: whisper-server is lazy and the GPU probe is cached and async (per HANDOFF).
- The front end shows a loading panel until the dashboard data arrives (`App.tsx:1029`); update check runs 5 s after launch (`App.tsx:582`).

Memory:

- whisper-server holds the model while warm: about 150 MB for `base.en`, about 0.6-1.6 GB for medium and large on CPU (or in VRAM with Vulkan), released after 10 idle minutes.
- Recording buffers: audio chunks go over an unbounded channel to the worker (`audio.rs:303`); a 10 min recording at 16 kHz mono is about 19 MB of samples plus the WAV on disk, which is fine.
- The incremental coordinator writes each segment as a separate WAV in the temp folder and deletes it afterwards.
- whisper-server is a child process with no Windows Job Object, so if Scribe crashes or is killed the server is orphaned with the model in memory and a port bound until reboot (the clean-exit path at `lib.rs:516-524` does not run on a crash).

## Storage and how it scales

Format: one SQLite file (`scribe.sqlite3`), tables `transcripts`, `settings` (all settings as one JSON row), `models`, and an unused `app_stats_daily`.
Journal mode is the default rollback journal (not WAL), and there is one connection behind a `Mutex` (`commands.rs:83`).
The owner's DB is 3.4 MB with 263 rows and 88% free pages (737 of 835); nothing ever runs `VACUUM`.

Rows scale fine: even 100,000 transcripts is tens of MB, and the `created_at` index covers the list views.
`LIKE` search is a full scan, which is still fast at that size; FTS5 would be the upgrade if search grows.

Audio clips do not scale well:

- Clips are 16 kHz 16-bit mono WAV, about 1.9 MB per minute.
- The owner recorded 372 minutes in five days (2026-10-23 to 28), which is 695 MB of referenced clips.
  At that pace the default 30-day retention keeps about 4 GB on disk.
- 784 of the 1,047 files in `clips/` (505 MB, all from June) are referenced by no row.
  Cause: the rebrand migration copied clips from `com.natkins.localdictate` (`lib.rs:120-132`), but the copied DB rows still point to the old folder, so retention deleted the old-path files and left the copies forever.
- The same migration duplicated the models: rows still point at `com.natkins.localdictate\models\` while copies sit in `com.natkins.scribe\models\` (about 220 MB wasted), and the old data folders (`com.natkins.localdictate`, `.dev`, about 430 MB together) are never removed.
- Stale segment and recording WAVs from June and October sit in `%LOCALAPPDATA%\com.natkins.scribe\recordings`; nothing sweeps the temp folder at startup.

Fixes: store clips as Opus (about 10x smaller at speech quality) or at least FLAC (about 2x); a startup sweep that deletes clip files no row references and temp WAVs older than a day; repoint migrated paths; show clip storage used in Data & Privacy with a "delete all clips" action; `PRAGMA user_version` migrations; WAL mode and an occasional `VACUUM`.

## Reliability matrix

| Scenario | Current behavior | Where | Risk | Fix | Effort |
| --- | --- | --- | --- | --- | --- |
| Sleep and wake | No power-event handling; the device list is re-read on every start, so a new default device after wake is picked up. A recording that spans sleep relies on cpal's error callback. GPU device loss after wake falls back GPU to CPU per request. | `audio.rs:1270`, `whisper_server.rs:200-220` | Low | Listen for `WM_POWERBROADCAST` resume: stop any active recording, restart whisper-server. | S |
| Mic unplugged mid-recording | cpal error callback stops the session, saves what was captured to History, never pastes it, toasts. | `audio.rs:1151-1165`, `audio.rs:668-700`, `dictation.rs:388` | Low | Good as is. A device that goes silent without an error is not detected (no "no samples" watchdog). | S |
| Selected mic missing at start | Hard error "Selected microphone is not available"; no fallback to the default device. | `audio.rs:1329-1340` | Medium: every dictation fails after unplugging a headset. | Fall back to the default device with a one-time toast. | S |
| Default device changes | Picked up on next start (no cached device). | `audio.rs:1270-1320` | Low | | |
| Model file deleted | Clear "Selected Whisper model is missing" error. | `model_manager.rs:272-295` | Low | Offer a re-download button in the error toast. | S |
| Model file corrupt | Server exits during startup, falls back to whisper-cli, which also fails; after 3 failures the server path is disabled for the session. Hash is only checked on select. | `whisper_server.rs:582-608`, `whisper_server.rs:93-104` | Medium | On a load failure, re-verify the hash and tell the user to re-download. | S |
| Large model slower to load than 15 s | Health check gives up at 15 s, kills the server, then whisper-cli loads the model again from scratch; three of those disable the server for the session, so every later dictation is a cold CLI run. | `HEALTH_CHECK_TIMEOUT` `whisper_server.rs:44` | Medium on slow disks or CPUs with large models | Scale the timeout with model size; don't count a slow first load as a failure. | S |
| whisper-server crash | Detected (`has_exited`) and restarted on the next request; CLI fallback for the current one. | `whisper_server.rs:478-484` | Low | | |
| whisper-server hang | The request blocks for up to 300 s (`INFERENCE_TIMEOUT`) while holding the transcriber lock; the incremental wait gives up at 15 s and the full-clip fallback then waits on the same lock, so the app sits in Transcribing for up to 5 minutes and the Discard hotkey does not apply. | `whisper_server.rs:47,338`, `incremental.rs:64` | Medium | Inference timeout proportional to audio length (for example 10x realtime plus 10 s), kill the server on timeout, allow cancel while transcribing. | M |
| Crash or kill mid-dictation | The temp WAV stays in `recordings/` and is never recovered or cleaned; whisper-server is orphaned (no Job Object). The log shows 5 `tao` "cannot move state from Destroyed" panics and 2 "state() called before manage()" panics on the main thread between June and August. | `lib.rs:516-524`, `Scribe.log` | Medium | Put whisper-server in a Job Object with kill-on-close; on startup offer to transcribe leftover recordings; investigate both panics (the first looks like a `tao` event-loop teardown bug, the second a call to `app.state::<BackendState>()` that runs before `setup` finishes). | M |
| Transcript save fails (disk full, DB error) | Error is returned before output runs, so the text is neither saved nor pasted; the WAV is quarantined to `failed/`. | `dictation.rs:332-345` then `dictation.rs:388` | High for data loss in a rare case | Paste first, then save; on a save failure keep the text in the Last Transcript Buffer in memory. | S |
| Disk full while recording | WAV and segment writes fail; the session errors. | `audio.rs` worker | Low | | |
| AI server down | Fast connection error, toast. | `note_analysis.rs:60-75` | Low | | |
| AI server slow (model still loading) | Transform Selection waits up to 180 s with the pill in Transcribing and no cancel; the user's selection and focus may have moved by then, and the paste lands wherever focus is. | `note_analysis.rs:14`, `selection_transform.rs:76` | Medium | Shorter timeout for transforms (30 s), cancel via Discard, and re-check the foreground window before pasting. | S |
| GitHub rate limit or 5xx | Treated as a generic HTTP error; no backoff, no `Retry-After`; the next attempt happens only at the next note save. 401 clears the token. | `github_backup.rs:251-270`, `note_sync.rs:82,246-290` | Low (each sync rebuilds the day's file from the DB, so nothing is lost) | Honor `Retry-After`, retry with backoff, show "last synced" age. | S |
| GitHub token expiry | Refreshed 5 minutes before expiry using the refresh token stored in the keychain. | `github_oauth.rs:42,291-320` | Low | | |
| Two instances | Single-instance plugin focuses the existing window; Dev and stable use separate identifiers, status files and discovery files. | `lib.rs:230-238`, `state_server.rs:449-461` | Low | | |
| Hotkey taken by another app | Registration failure is logged and surfaced to the UI. | `hotkeys.rs:187`, `hotkeys.rs:726` | Low | | |
| Default toggle key steals a character | Backquote with no modifiers is a global hotkey, so the backtick cannot be typed anywhere while Scribe runs. | `settings.rs:579`, `hotkeys.rs:598` | High for developers and Markdown users | Default to a key that types nothing (Right Ctrl, Caps Lock, F-key, or a mouse button) or pass the key through on a quick tap. | M |
| Non-US keyboard layouts | The toggle key is VK `0xC0` (`hotkeys.rs:598`), which is a letter on some layouts (Ö on German). Ctrl+Alt+V/X/R/F defaults are AltGr chords on many European layouts (for example AltGr+V types @ in Hungarian, AltGr+X types ź in Polish), so Scribe swallows those characters. Text injection itself is layout-safe (`KEYEVENTF_UNICODE`, `output.rs:724`). | as cited | High for non-US users | Detect AltGr layouts and pick other defaults at first run; avoid bare keys as defaults. | M |
| Target window is elevated (admin terminal, installer, Task Manager) | `SendInput` is silently blocked by UIPI; Scribe still reports "Pasted" and then restores the clipboard, so the user cannot even paste by hand. | `output.rs:960-975`, `output.rs:377-380` | Medium | Check the foreground window's integrity level before pasting; if it is higher, leave the transcript on the clipboard and toast "press Ctrl+V". | S |
| Remote Desktop | No special handling; works in a local session; mic and paste inside an RDP client window depend on redirection. | | Low | Document it. | XS |
| Terminal as the target for Transform Selection | The synthetic Ctrl+C is SIGINT in most terminals when nothing is selected. | `selection_transform.rs:109`, `selection_transform.rs:212-222` | High | See F1. | S |
| Clipboard history and cloud clipboard | Every dictation and every captured selection is added to Win+V history and synced to the cloud clipboard if the user enabled it, even though the clipboard is "restored". | `output.rs:365-380`; no `CanIncludeInClipboardHistory` or `ExcludeClipboardContentFromMonitorProcessing` anywhere | Medium (privacy) | Add the exclusion formats when Scribe writes the clipboard. | S |

## Missing features vs. the category, ranked by value

Built on `docs/COMPETITIVE-ANALYSIS.md` (sections 5-6); multilingual and GPU are done since it was written.

1. **Per-app context and modes** (Wispr Flow, SuperWhisper): pick style, replacements, language or an AI rewrite per foreground app (email vs. chat vs. code editor). Scribe already knows the foreground window for the paste guard, so detection is cheap.
2. **Voice commands and spoken punctuation** ("new line", "period", "scratch that", "all caps"). Deterministic, local, and the biggest daily-use gap against Windows voice typing.
3. **Optional AI cleanup pass with a strict latency budget** (Wispr Flow's core feature): it was removed in 0.6.0 because it added lag; it could return as an opt-in per-app mode now that local LLMs are faster.
4. **First-run onboarding**: mic check, model pick with a size and speed hint, and a practice dictation. Also the place to pick layout-safe hotkeys.
5. **Custom vocabulary learning**: suggest dictionary and replacement entries from the user's own corrections in History.
6. **Streaming insertion of stable segments** while still talking (insert each finished phrase). Incremental segments already exist, so this is mostly an output change, and it removes perceived latency for long dictations.
7. **Custom model files** (point at any ggml `.bin`, including distil and fine-tuned models).
8. **Mouse-button and foot-pedal triggers** and a push-to-talk key that types nothing; this also solves the backtick problem.
9. **Per-dictation language switching** (a second hotkey or auto-detect per segment) for bilingual users.
10. **Accessibility users**: voice-only operation (start, stop and confirm without a keyboard), screen-reader announcements of state changes, and larger pill text.
11. **Settings and vocabulary sync** across machines (could ride on the existing GitHub backup).
12. **Speaker labels and timestamps for file transcription** (MacWhisper's main strength).

## Findings

Severity: High, Medium, Low. Effort: XS (under an hour), S (a day), M (a few days), L (a week or more).

**F1. High. Transform Selection uses a stale clipboard and sends Ctrl+C into terminals.**
`capture_selection` sends Ctrl+C, sleeps 120 ms and reads whatever text is on the clipboard (`selection_transform.rs:98-121`).
It never checks `GetClipboardSequenceNumber`, so with nothing selected it takes the previous clipboard contents as the selection, rewrites them, and pastes the result at the cursor; the empty-selection guard at `hotkeys.rs:1124` never fires when the clipboard had text.
In a terminal, the synthetic Ctrl+C with no selection interrupts the running process.
Fix: record the clipboard sequence number before Ctrl+C and treat "unchanged" as no selection; skip or use Ctrl+Shift+C for console windows (class `ConsoleWindowClass` or `CASCADIA_HOSTING_WINDOW_CLASS`); consider UI Automation `TextPattern` to read the selection without the clipboard (`docs/EXPERIMENT_UIA_INSERT.md`).
Effort: S.

**F2. High. Ctrl+Alt+R is registered and active with the AI features off.**
`toggle_transform` (`hotkeys.rs:1100`) never checks `notes_analysis_enabled`; only the Notes analyze command does (`commands.rs:801`).
The chord is held system-wide (taking it from other apps), sends Ctrl+C, records, then fails at the LLM call.
Fix: register the hotkey only when the AI integration is enabled.
Effort: XS.

**F3. High. The default toggle key swallows a character system-wide; Ctrl+Alt defaults collide with AltGr.**
Backquote without modifiers (`settings.rs:579`) is a global hotkey, so the backtick cannot be typed; on German layouts the same VK is Ö.
Ctrl+Alt+V/X/R/F are AltGr characters on several European layouts.
Fix: change defaults (or choose them per layout at first run), and pass a quick tap through as the original key.
Effort: M.

**F4. High (storage). Audio clips grow to gigabytes, and the rebrand migration orphaned 505 MB.**
WAV at 1.9 MB per minute; about 4 GB per 30 days at the owner's real usage.
784 June clips are unreferenced copies made by `migrate_pre_rebrand_data` (`lib.rs:120-132`), whose rows pointed at the old folder; duplicated models and the old `com.natkins.localdictate*` folders add about 650 MB more.
Fix: Opus or FLAC clips, an orphan sweep at startup, repoint migrated paths, show clip storage in Data & Privacy.
Effort: M.

**F5. Medium. The fixed 400 ms stop grace is the largest avoidable latency, and Stats hides it.**
`STOP_GRACE_MS` (`audio.rs:58,425`) plus the 60 ms paste settle (`output.rs:371`) are about half of the roughly 0.95 s release-to-text time; the stored latency starts after them (`dictation.rs:62`).
Fix: adaptive grace (skip when the tail is already silent), and record end-to-end latency.
Effort: S.

**F6. Medium. Text is pasted only after it is saved; a save failure loses the dictation from the user's view.**
`dictation.rs:332-345` returns on a DB error before output at `dictation.rs:388`.
Fix: paste first, then save, and keep the text available for Paste Last even when the save fails.
Effort: S.

**F7. Medium. A missing selected microphone fails every dictation.**
`select_input_device` (`audio.rs:1323-1340`) errors instead of falling back to the default.
Fix: fall back with a toast.
Effort: XS.

**F8. Medium. Paste into elevated windows fails silently and the clipboard is restored anyway.**
UIPI blocks `SendInput` without an error (`output.rs:960-975`), so the result is "Pasted" and `restore_clipboard_snapshot` (`output.rs:380`) removes the transcript from the clipboard.
Fix: integrity-level check before pasting, fall back to copy with a toast.
Effort: S.

**F9. Medium. First dictation after 10 idle minutes pays a cold model load; slow loads can disable the warm server for the session.**
`IDLE_SHUTDOWN` 10 min (`whisper_server.rs:40`), no pre-warm, 15 s health timeout (`:44`), 3 strikes disable (`:49`).
Fix: pre-warm on press, configurable idle window, model-size-aware timeout.
Effort: S.

**F10. Medium. Speech at the start of a dictation can be clipped.**
150 ms chord arming (`hotkeys.rs:1259`) plus per-start device enumeration and stream open (`audio.rs:1270-1320`), with no pre-roll.
Fix: keep a warm input stream (or open it faster) with a 300-500 ms ring buffer prepended to the recording.
Effort: M.

**F11. Medium (privacy). Transcripts and captured selections go into Windows clipboard history and cloud clipboard.**
No clipboard-history exclusion formats are set anywhere in `output.rs`.
Fix: set `CanIncludeInClipboardHistory = 0`, `CanUploadToCloudClipboard = 0` and `ExcludeClipboardContentFromMonitorProcessing` on Scribe's clipboard writes.
Effort: S.

**F12. Medium. A hung whisper-server leaves the app in Transcribing for up to 5 minutes with no cancel.**
`INFERENCE_TIMEOUT` 300 s (`whisper_server.rs:47`) under the transcriber lock.
Fix: timeout proportional to audio length, kill on timeout, cancel while transcribing.
Effort: M.

**F13. Medium. Main-thread panics in the field and an orphanable whisper-server.**
5 `tao` "cannot move state from Destroyed" and 2 "state() called before manage()" panics in the owner's log (June to August); no Job Object for the child process.
Fix: Job Object with kill-on-close; reproduce the panics (likely shutdown or session end, and an early event before `setup` finishes) and guard with `try_state`.
Effort: M.

**F14. Low. Stats are wrong in two ways.**
"Today" uses the UTC date (`db.rs:595,612`); "Total words" shrinks with retention because it sums the current history (`db.rs:618`); `app_stats_daily` exists and is never written.
Fix: local-date grouping and write daily aggregates to `app_stats_daily` so totals survive retention.
Effort: S.

**F15. Low. Type-it-out paste method has no UI; `recording_mode` is inert.**
`PasteMethod::DirectInsert` exists in the backend and is honored (`output.rs`), but no view sets it; `recording_mode` is only typed (`backend.ts:138`).
Fix: add "Paste method" under Output (with a hint for apps that block paste), and remove `recording_mode`.
Effort: XS.

**F16. Low. Failed recordings are kept but cannot be retried from the app.**
Only "open folder" exists (`lib.rs` command list).
Fix: a Failed list in Audio or History with Retry and Delete.
Effort: S.

**F17. Low. Model select saves the setting before verifying the hash.**
`model_manager.rs:249-253`.
Fix: verify first.
Effort: XS.

**F18. Low. Long AI waits and no rate-limit handling.**
Transform waits up to 180 s (`note_analysis.rs:14`) and then pastes into whatever has focus; GitHub sync has no backoff or `Retry-After`.
Fix: 30 s transform timeout with cancel and a foreground-window check before pasting; backoff for sync.
Effort: S.

**F19. Low. ffmpeg is required on PATH for video and M4A file transcription.**
`file_transcribe.rs:139`.
Fix: bundle a minimal ffmpeg, or decode common formats with Symphonia in-process.
Effort: M.

**F20. Low. Storage hygiene.**
No temp-folder sweep (stale WAVs in `recordings/`), no `VACUUM` (88% free pages), migrations without `user_version`, DB in rollback-journal mode.
Fix: startup sweep, periodic `VACUUM`, versioned migrations, WAL.
Effort: S.
