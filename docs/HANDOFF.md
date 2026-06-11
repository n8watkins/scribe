# LocalDictate — Session Handoff

Last updated: 2026-06-11 (QA + live-feedback session, v0.2.0)
Read this first, then `docs/STATUS_AND_NEXT_STEPS.md` for deeper project history.

## Project summary

LocalDictate is a Windows-only Tauri app (Rust backend + React/Vite frontend) for
local push-to-talk dictation via whisper.cpp. Hold `Ctrl+Win` (or tap `` ` ``),
talk, and text is typed at the cursor by a locally running model. The owner
(Nathan) uses it daily on Windows 11; it is his personal tool and he is the only
stakeholder. Source of truth is the WSL repo
(`/home/natkins/personal/tools/localdictate`); a Windows clone at
`C:\Users\natha\Projects\Tools\localdictate` exists solely to build/test with the
Windows toolchain. GitHub: `https://github.com/n8watkins/localdictate`.

## State after this session

**v0.2.0 is installed on the owner's machine** (silent NSIS upgrade from the
clone's `target/release/bundle/nsis/`). The owner QA'd the previous session's
work live and dictated feedback mid-session; everything below shipped in
response. 91/91 tests on the Windows toolchain, `npx tsc` + `npm run build`
clean. **Local only — not pushed** (now 15 unpushed commits across both
sessions).

| Commit | What |
|---|---|
| `3c1fa3b` | **The QA find:** pill capability lacked `core:window:allow-set-size`/`allow-scale-factor`; the redesign's `setSize()` threw inside a swallow-all catch before `show()`, so the pill never appeared during recording. Tests/`tsc` can't catch this class of bug — only a real run. |
| `1331c95` | Text-mode pill starts compact (320×80), grows upward with the live transcript, caps at 150 logical px; background more transparent (0.55/0.72 hover). |
| `287926f` | Live transcript streams sooner: `SEGMENT_SILENCE_MS` 500→350 ms; segmenter test assertions derive from the constant. |
| `85ee41a` | Dynamic-growth measurement reads an inner block div — `justify-content: flex-end` clip boxes don't report top-edge overflow in `scrollHeight` (confirmation text was clipped). |
| `437e057` | All pill window mutations serialized through one promise chain (concurrent ops compounded bottom-anchor shifts and walked the pill up the screen); text layout persists through Stopping/Transcribing/Pasting so the pill doesn't bounce through bar size. |
| `92367b6` | Owner-feedback batch: `Ctrl+Alt+F` hides the dashboard when focused (new `dashboardHotkeyToggles` setting, default on); paste waits up to 1.5 s for physical hotkey modifiers to release before `SendInput` (held Ctrl+Alt turned typed "," into Ctrl+Alt+"," — Windows Terminal opens its settings JSON); confirmation pill 5→8 s; visualizer full-scale at 0.07 RMS (was 0.12). |
| `8344e5e` | v0.2.0 everywhere (About had hardcoded "0.1.0"); About shows the runtime version; new `update_check.rs` compares against the latest GitHub release tag — Check-for-updates button in About + one quiet auto-check 5 s after launch that toasts. |

### Visual QA results (on real hardware, real DB)

- Migration 002 (`audio_path`) and defaults v4 ran cleanly on the live DB; a
  pre-migration backup sits at
  `%APPDATA%\Roaming\com.natkins.localdictate\localdictate.sqlite3.bak-pre-qa-20260611`.
- Timeout-transcribe: recording stopped at the cap with reason `Timeout`,
  transcribed, returned to Idle. Freeze bug confirmed dead.
- Focus guard: with the dashboard foregrounded, paste refocused the next
  real app and never typed into LocalDictate.
- Clips: saved, valid 16 kHz mono WAVs of the trimmed audio; History shows
  Play only on clip-backed rows. (Owner uses pill-confirmation Copy live.)
- Transcribe-a-file: WAV → exact text, Save writes `<source>.txt`,
  MP4 without ffmpeg → friendly `ffmpeg_required` message.
- ffmpeg is now installed for real (winget `Gyan.FFmpeg`, on user PATH); the
  running instance was relaunched with the fresh PATH. **Video transcription
  not yet exercised by the owner** — first MP4 through the card confirms it.
- Incremental streaming needs chunk RMS ≥ 0.03 to segment; quiet audio (e.g.
  speaker playback picked up by the mic) silently falls back to full-clip
  transcription. The owner's real voice segments fine (0 ms stop-to-text).

## Next steps (priority order)

1. **Owner verifies the feel** of the v0.2.0 install: pill growth/anchoring,
   `Ctrl+Alt+F` toggle, `Ctrl+Alt+V` into a terminal (was opening Windows
   Terminal's settings JSON), one MP4 through Transcribe-a-file.
2. ~~Push + release~~ **Done 2026-06-11**: repo is public, all commits pushed,
   `v0.2.0` tag pushed and the Release workflow ran. The update-check API
   verified working against the public repo.
3. **Notes feature** — designed but NOT approved in detail; confirm specifics
   with the owner before building big pieces. Decided so far: dedicated
   note-taking hotkey (owner suggested a `~`+Q-style chord; exact bind TBD),
   pill turns **blue** while taking a note, notes save as flagged transcripts,
   a Notes section in the dashboard, optional local-LLM analysis with a
   user-editable prompt, Google Drive sync via a new Integrations tab.
   Architecture decision already made: **plain Google Drive REST + OAuth, not MCP**.
4. ~~OpenWhispr fallback~~ **Done 2026-06-11**: fallback removed from
   `model_manager.rs` (`LOCALDICTATE_MODEL_DIR` kept), 6.2 GB cache deleted.
   The owner KEEPS OpenWhispr installed — an empty locked `qdrant-data` dir
   remains under `~/.cache/openwhispr` because the app was running.
5. Carry-overs: real auto-updater (`tauri-plugin-updater`, needs signing
   keys), code signing.
6. Housekeeping: owner may want to delete the QA "quick brown fox" transcripts
   from History; QA scratch files live in `C:\Users\natha\AppData\Local\Temp\ld-qa\`
   (safe to delete wholesale).

## Conventions & gotchas

- **Windows verification is mandatory** for Rust changes: almost everything is
  `#[cfg(windows)]`; green WSL `cargo check` proves nothing. Sync the clone
  without pushing: from `/mnt/c/Users/natha/Projects/Tools/localdictate`,
  `git fetch /home/natkins/personal/tools/localdictate main && git merge --ff-only FETCH_HEAD`,
  then `cd /mnt/c && cmd.exe /c "cd /d C:\Users\natha\Projects\Tools\localdictate\app\src-tauri && cargo check 2>&1"`
  (likewise `cargo test`).
- **The owner is usually AT the machine during sessions** — no input
  injection, no audio playback, no focus stealing while he works. Verify via
  logs (`%LOCALAPPDATA%\com.natkins.localdictate\logs\LocalDictate.log` — note
  LOCALAPPDATA, not APPDATA) and direct SQLite edits (settings JSON in the
  `settings` table; the backend re-reads per recording, the pill re-reads per
  state change). Ship to him by building the NSIS bundle and upgrading
  silently (`LocalDictate_*_setup.exe /S`) — he launches the installed app,
  and single-instance means dev-exe launches just focus it.
- Frontend webview bugs (permissions, window management) only surface in a
  real run — the pill capability bug survived 88 green tests.
- Commit per logical change with a `Co-Authored-By: Claude ...` trailer. Push
  only when the owner asks.
- Shipped-default changes go through `CURRENT_DEFAULTS_VERSION` +
  `migrate_defaults` in `settings.rs`. Brand-new settings fields just need
  `#[serde(default = ...)]` (see `dashboard_hotkey_toggles`).
- DB schema changes: numbered SQL files in `app/src-tauri/migrations/`,
  applied in `db.rs::apply_migrations`.
- Frontend check: `npx tsc --noEmit -p tsconfig.json` then `npm run build` in `app/`.
- The owner dictates long, stream-of-consciousness requests; transcription
  garbles words — ask one targeted question when a requirement is ambiguous,
  otherwise pick the sensible reading and say so.
- reqwest is compiled with `default-features = false` — no `json()` on
  responses; use `text()` + `serde_json`.

## File map

- `app/src-tauri/src/audio.rs` — capture, silence auto-stop, timeout thread, stop-grace capture
- `app/src-tauri/src/incremental.rs` — live phrase segmentation (350 ms silence, tail padding)
- `app/src-tauri/src/file_transcribe.rs` — transcribe-a-file backend (whisper-cli + ffmpeg fallback)
- `app/src-tauri/src/dictation.rs` — transcribe pipeline; `save_audio_clip`
- `app/src-tauri/src/output.rs` — paste/insert, focus guard, `wait_for_modifier_release`
- `app/src-tauri/src/update_check.rs` — GitHub latest-release version check
- `app/src-tauri/src/settings.rs` — AppSettings, defaults v4, `dashboard_hotkey_toggles`
- `app/src-tauri/src/tray.rs` — `toggle_dashboard` / `open_dashboard`
- `app/src-tauri/src/db.rs` — SQLite, migrations, clip-file cleanup
- `app/src-tauri/src/commands.rs`, `lib.rs` — Tauri command surface
- `app/src-tauri/capabilities/pill.json` — pill window ACL (set-size/scale-factor matter)
- `app/src/App.tsx` — dashboard (AboutView has the update check)
- `app/src/PillApp.tsx`, `app/src/pill.css` — pill modes, dynamic text growth, serialized window ops
- `app/src/backend.ts` — TS command wrappers and types
