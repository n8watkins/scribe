# Scribe - Status and Next Steps

> This doc keeps the longer project history. The sections below the "Where
> things stand" summary are a historical record of the early V1 push
> (2026-06-10 → 06-11) and are intentionally left as-shipped; for the
> day-to-day feature record see [`CHANGELOG.md`](../CHANGELOG.md), and for a
> competitive review + prioritized gap list see
> [`docs/COMPETITIVE-ANALYSIS.md`](COMPETITIVE-ANALYSIS.md).

Status: Shipping (public, MIT open-source) — current version **0.5.22**  
Last updated: 2026-06-15  
Repository: `https://github.com/n8watkins/scribe` (public)  
Releases: signed installers published per tag via CI (see the GitHub Releases page)

## Where things stand

Scribe is a mature local dictation app the owner uses daily on Windows 11. Core
loop: hold `Ctrl+Win` (or tap `` ` ``), talk, and text is inserted at your
cursor by a locally running whisper.cpp model (warm `whisper-server` with a
`whisper-cli` fallback), with live/incremental transcription so stop-to-text is
near-instant. On top of dictation it now ships **quick notes** (`~`+Q) with
on-demand local-LLM analysis, **AI dictation cleanup**, **selected-text
transform** (an inline AI editor), a **dictionary** (context hint +
replacements), **searchable history** with date-range search and combine,
**separate transcript/notes retention**, **Google Drive sync + Markdown/CSV/JSON
export**, a configurable **floating pill**, fully **rebindable hotkeys** with
per-bind press/release triggers, a **model manager**, and a **signed
auto-updater** with OS notifications. The backend test suite (~160 tests) and
the frontend both build clean; the audio/hotkey/paste paths are Windows-gated
and verified on the Windows toolchain.

The "What was done" sections below are the original V1 launch log (2026-06-10 →
06-11) and predate almost all of the above — read them as history, not current
state.

## What was done (2026-06-10 → 06-11)

### Made it compile and ship
- Fixed the Windows build (170 errors from three roots: cpal's `!Send` stream wrapper in Tauri managed state, a missing `windows`-crate feature, stray `Debug` derives). Established the rule that matters: **Windows-gated code must be verified with the Windows toolchain** — `cargo check` in WSL never compiles it. Workflow: WSL repo is source of truth; the clone at `C:\Users\natha\Projects\Tools\localdictate` builds/tests via `cmd.exe` interop.
- Produced the first NSIS/MSI installers; pruned the whisper.cpp resource drop to exactly the needed binaries (everything in `resources/` gets bundled); verified the MSI payload.

### Made hotkeys real
- Replaced the unusable defaults (`Ctrl+Win+D` is "new virtual desktop"…) with `Ctrl+Shift` hold-to-talk, `~` toggle, `Ctrl+Alt+V` paste, `Ctrl+Alt+D` dashboard — with one-time migrations for existing installs.
- Modifier-only chords (e.g. bare `Ctrl+Shift`) are unsupported by the global-shortcut plugin, so there's a native Windows watcher: `GetAsyncKeyState` polling with a 150 ms arming delay that suppresses ordinary `Ctrl+Shift+<key>` shortcuts.
- Real rebind UI (press-to-capture, inline conflict errors, reset to defaults). Registration is per-binding best-effort with failures surfaced as toasts — and the recording-mode gate that silently discarded hold-to-talk presses in toggle mode is gone.

### Made it pleasant
- Pill overlay is a real always-on-top frameless window (label `pill`): visible while the main window is hidden, draggable, position persisted (`pillX`/`pillY`), click-to-stop.
- UI restructured and densified twice: Stats and Data & Privacy views, History owns recents, icon-only actions with tooltips, friendly mic names (never endpoint GUIDs), audio start/stop cues, stop controls in topbar/pill, test-clip playback, open data/models folder commands, 940×600 default window.

### Made it fast and smart (waves 1–2)
- **Auto-paste is the default output mode** (versioned migration via `defaultsVersion`).
- **Warm transcriber** (`src/whisper_server.rs`): resident `whisper-server.exe` holds the model in RAM across dictations; per-request vocabulary prompt (verified empirically); 10-minute idle shutdown; auto-fallback to `whisper-cli.exe`; killed on exit. `transcribe()` is a stateless serialized primitive, deliberately segment-shaped.
- **Auto-stop on silence** for toggle/manual recordings (arms after speech ≥ 0.03 RMS, fires after `silenceAutoStopMs` below 0.015 RMS); real silence trimming replaced the placeholder.
- **Custom vocabulary** setting → whisper `--prompt`.
- Single-instance plugin; file logging via `tauri-plugin-log` (LogDir) — release builds are no longer silent.

### Made it open-sourceable
- Root README (user install + build-from-source incl. the required whisper.cpp binaries), MIT LICENSE, v0.1.0 GitHub release with installer.

## What to do next (priority order)

Shipped since the original V1 list: incremental/live transcription, tag-triggered
CI release workflow, launch at startup, Notes (`~`+Q) + on-demand analysis,
**signed auto-updater** with OS notifications, AI dictation cleanup, selected-text
transform, the dictionary (context hint + replacements), history date-range
search + combine, separate transcript/notes retention, Google Drive sync +
Markdown/CSV/JSON export, configurable pill, repo flipped public. The
auto-updater carry-over is done; the items below remain.

For a researched, prioritized "build next to be competitive" list (with rough
effort), see [`docs/COMPETITIVE-ANALYSIS.md`](COMPETITIVE-ANALYSIS.md). Short
version of the open gaps:

1. ~~Authenticode code signing~~ — **DROPPED (2026-06-20).** Decided not to
   pursue. We're staying unsigned, so the SmartScreen first-run warning remains.
   The free **SignPath Foundation** OSS route existed but wasn't worth the
   dependency on repo reputation; paid Azure Trusted Signing (~$10/mo) is out of
   scope. (Unrelated to the updater's minisign artifact signing, which already
   ships.)
2. ~~Multilingual transcription + Whisper translate~~ — **DONE in 0.5.21**
   (multilingual `ggml` models + a ~29-language picker + Translate→English).
3. **Custom / arbitrary local model selection** — let users point Scribe at any
   whisper.cpp-compatible `ggml` `.bin` they already have on disk, instead of
   only the curated download catalog in the Models tab.
4. **Spoken punctuation / voice editing + real-time streaming insertion** —
   _partially addressed._ The "too much punctuation when I pause" complaint was
   fixed (tunable `segment_pause_ms`, default **3 s** — Audio → Live
   transcription). Still open: **literal spoken punctuation** ("period" → `.`) /
   voice-command editing, and **real-time streaming insertion** (text still lands
   only after you stop, not while you talk).
5. **GPU acceleration (Vulkan)** — speed up the large/most-accurate models on
   any modern GPU (NVIDIA/AMD/Intel). Designed but not started; see
   [`docs/GPU_VULKAN_BUILD_PLAN.md`](GPU_VULKAN_BUILD_PLAN.md). The open question
   that picks the approach: does a single Vulkan build still run on a no-GPU
   machine (one build, auto-fallback) or do we ship it as an optional download?
6. **Pause-aware filler suppression** — a user-editable filler-word list
   (`um`/`uh`/…) removed only when bracketed by a pause, so fluent uses ("oh no",
   "like this") survive. Designed but not started; see
   [`docs/FILLER_SUPPRESSION_PLAN.md`](FILLER_SUPPRESSION_PLAN.md). Gating
   question: can the bundled `whisper-server` return token timestamps (needs them
   off the warm path, not just the CLI)?
7. Parked: a first-run onboarding wizard; FTS5 search if histories grow.

## Working notes for the next session

- Verify Rust changes on Windows: `cd /mnt/c && cmd.exe /c "cd /d C:\Users\natha\Projects\Tools\localdictate\app\src-tauri && cargo check 2>&1"` (likewise `cargo test`, `npm run tauri build` from `app\`).
- The Windows clone's `resources/bin/windows/` binaries are untracked — `git reset --hard` keeps them. `whisper-extras-unbundled/` at the clone root holds the unused whisper.cpp extras.
- Installed-app data (settings DB, models, logs): `%APPDATA%\com.natkins.localdictate\` — readable from WSL for debugging; reading the settings JSON there found the toggle-mode bug.
