# LocalDictate — Session Handoff

Last updated: 2026-06-12 (early AM, end of the marathon QA + Notes session)
Read this first, then `docs/STATUS_AND_NEXT_STEPS.md` for older project history.

## Project summary

LocalDictate is a Windows-only Tauri 2 app (Rust backend + React/Vite
frontend) for local push-to-talk dictation via whisper.cpp. Hold `Ctrl+Win`
or tap `` ` `` (acts on release), talk, and text is typed at the cursor by a
locally running model. Holding `` ` `` and tapping `Q` dictates a **note**
instead (blue pill, saved to the Notes view, never pasted). The owner
(Nathan) uses it daily on Windows 11; he is the only stakeholder and is
usually AT the machine during agent sessions.

- WSL repo (source of truth): `/home/natkins/n8builds/tools/localdictate`
- Windows clone (build/test only): `C:\Users\natha\Projects\Tools\localdictate`
- GitHub: `https://github.com/n8watkins/localdictate` (public; releases via
  tag-triggered CI; latest release tag `v0.2.0`)
- Installed apps on the owner's machine:
  - **LocalDictate** (stable, `com.natkins.localdictate`) — his daily tool;
    currently runs commit `b78e498`. Only upgrade it when he asks: full
    `npm run tauri build`, then silent NSIS `/S` upgrade, relaunch from
    `C:\Users\natha\AppData\Local\LocalDictate\app.exe`.
  - **LocalDictate Dev** (`com.natkins.localdictate.dev`) — the agent's test
    app, own data dir/DB, runs side-by-side with stable.

## State (all pushed through `b78e498`)

Everything below is shipped to the stable install and owner-verified unless
noted. Session commits, newest first:

| Commit | What |
|---|---|
| `b78e498` | On-brand cyan scrollbars; native title bar colored to the app bg via DwmSetWindowAttribute (`lib.rs::style_native_titlebar`) |
| `6ba2a25` | Whisper noise annotations (`[BLANK_AUDIO]`, `(silence)`, …) stripped in `whisper.rs::normalize_transcript_text`; annotation-only transcripts become empty |
| `732cfe4` | LocalDictate Dev build flavor (`tauri.dev-flavor.conf.json`, `npm run tauri:dev-flavor`) |
| `67a1dd9`/`eebf692`/`9df6188`/`11750e6` | The hotkey saga (see Gotchas): toggle key driven by a native GetAsyncKeyState watcher, Q grabbed from worker threads |
| `c986c68` | Notes v1: tilde-release toggle, tilde+Q note chord, `is_note` (migration 003), Notes view, archive pages 10→25 |
| `65d1605` | OpenWhispr model-cache fallback removed (cache deleted; owner KEEPS the OpenWhispr app) |
| `8344e5e` | v0.2.0, runtime version in About, GitHub update check (button + launch toast via `update_check.rs`) |
| `92367b6` | Ctrl+Alt+F dashboard toggle (`dashboardHotkeyToggles` setting); paste waits for hotkey-modifier release (fixed Ctrl+Alt+V opening Windows Terminal's settings JSON); confirm pill 8 s; visualizer full-scale at 0.07 RMS |
| `437e057`/`85ee41a`/`287926f`/`1331c95`/`3c1fa3b` | Pill: missing window ACL perms (the pill never showed at all before), compact-start + grow-upward text mode (cap 150), serialized window ops, 350 ms segment streaming |

Verified on hardware by the owner: tilde toggle on release, tilde+Q blue-pill
note, pill growth/anchoring, Ctrl+Alt+F toggle, blank-audio fix, seamless
title bar. ffmpeg is installed (winget Gyan.FFmpeg, user PATH) so
Transcribe-a-file handles video; **owner hasn't personally run an MP4 yet**,
and hasn't yet dictated a non-silent note (empty notes are never saved).

## Next steps (priority order — owner-confirmed)

1. **Local-LLM analysis of notes with a user-editable prompt.** Decided so
   far: the analysis prompt lives in settings and is user-editable; analysis
   applies to note transcripts (the `is_note` rows). NOT yet decided (ask or
   propose cheaply): which local LLM runtime (nothing is integrated today —
   whisper.cpp only does STT), what "analysis" produces (summary? action
   items? tags?), where results show in the Notes view, and whether analysis
   runs automatically per note or on demand. Recommend starting with a
   design + a thin on-demand pipeline behind a settings field
   (`notesAnalysisPrompt`), defaulting OFF.
2. **Auto-updater** (`tauri-plugin-updater`): generate updater signing keys,
   add the plugin + updater artifacts to the CI release workflow, wire
   "Install update" into the existing update check UI (`update_check.rs`,
   AboutView in `App.tsx`). The check+toast already exists; this adds
   one-click install. Owner asked for this explicitly.
3. Then: Google Drive sync of notes via a new Integrations tab (decided:
   plain Drive REST + OAuth, NOT MCP); code signing (revisit when the owner
   wants money spent — explained to him 2026-06-12); optional v0.3.0 tag so
   the GitHub release matches shipped code.

## Conventions & gotchas (hard-won — do not relearn these)

- **Windows verification is mandatory** for Rust changes: nearly everything
  is `#[cfg(windows)]`. Sync the clone:
  `cd /mnt/c/Users/natha/Projects/Tools/localdictate && git fetch /home/natkins/n8builds/tools/localdictate main && git merge --ff-only FETCH_HEAD`
  then `cd /mnt/c && cmd.exe /c "cd /d C:\Users\natha\Projects\Tools\localdictate\app\src-tauri && cargo test 2>&1"`.
  Frontend: `npx tsc --noEmit -p tsconfig.json && npm run build` in `app/`.
- **Test on the Dev flavor, ship to stable only on owner request.**
  `npm run tauri:dev-flavor` builds `target/release/app.exe` with the dev
  identifier (the flavor is baked at build time — always rebuild the right
  flavor before launching). Dev has its own data dir
  (`%APPDATA%\Roaming\com.natkins.localdictate.dev`, models already copied).
  Global hotkeys belong to whichever instance registered first.
- **The owner is AT the machine**: no input injection, no audio playback,
  no focus stealing. Verify via the log
  (`%LOCALAPPDATA%\com.natkins.localdictate\logs\LocalDictate.log`) and
  direct SQLite settings edits (backend re-reads per recording; the pill
  re-reads per state change). Wait for dictation-idle in the log before
  killing/upgrading the app. **Synthetic keypresses are invisible** to both
  the hotkey plugin and GetAsyncKeyState while a hotkey is registered —
  hotkey verification needs the owner's physical keys (he responds fast).
- **Hotkey plumbing landmines** (cost hours): tauri-plugin-global-shortcut
  `register`/`unregister` deadlock when called on the main thread (inside a
  handler OR via run_on_main_thread) but work from worker threads; an
  unmodified `Backquote` shortcut has id 0 and its events were unreliable;
  RegisterHotKey keeps suppressing the keystroke even when events are
  dropped. Hence: the toggle key is owned by `hotkeys.rs::run_toggle_watcher`
  (GetAsyncKeyState polling, both edges), plugin events for ToggleDictation
  are ignored when the watcher is active, and the Q grab is armed/disarmed
  from `std::thread::spawn`.
- Webview/window bugs (ACL permissions, window management) only surface in
  real runs — the pill capability bug survived 88 green tests. Pill window
  perms live in `app/src-tauri/capabilities/pill.json`.
- Settings: new fields need `#[serde(default = ...)]` only; changed shipped
  defaults go through `CURRENT_DEFAULTS_VERSION` + `migrate_defaults`. DB
  schema: numbered SQL in `app/src-tauri/migrations/` (003 = `is_note`),
  applied in `db.rs::apply_migrations` ("duplicate column" = already ran).
- reqwest has `default-features = false`: no `.json()`, use `.text()` +
  serde_json (see `update_check.rs`).
- Notes semantics: `is_note` transcripts save to history only (never the
  Last Transcript Buffer, never auto-pasted, saved even with history off).
- Commit per logical change with `Co-Authored-By: Claude ...`; push only
  when the owner asks (he has been saying push — everything is pushed).
- Owner dictates stream-of-consciousness; transcription garbles words. Pick
  the sensible reading, say so, ask at most one targeted question.

## File map (for the next steps)

- `app/src-tauri/src/settings.rs` — AppSettings; add `notesAnalysisPrompt` here
- `app/src-tauri/src/db.rs` — transcripts + `search_transcripts(notes_only)`
- `app/src-tauri/src/transcript.rs` — Transcript (`is_note`)
- `app/src-tauri/src/update_check.rs` — version check; extend for updater UX
- `app/src-tauri/src/lib.rs` — plugin/command registration, titlebar styling
- `app/src-tauri/src/hotkeys.rs` — toggle watcher + note chord (don't touch lightly)
- `app/src/App.tsx` — Notes view (HistoryView notesOnly), AboutView updates row, Settings rows
- `app/src/backend.ts` — TS command wrappers/types
- `.github/workflows/` — tag-triggered Release workflow (updater artifacts go here)
