# Scribe - Agent Orchestration

> **Archived plan:** This was the initial 2026-06-10 implementation assignment and is no longer active.
> Its paths, branch assumptions, and baseline status are historical.
> See the [documentation index](README.md) for current guidance.

Original status: Working assignment plan
Created: 2026-06-10  
Related docs:

- [PRD.md](./PRD.md)
- [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md)

## Current Baseline

The project has:

- Tauri v2 + React + TypeScript scaffold in `scribe/app`.
- Frontend dashboard shell with sidebar navigation.
- Views for Dashboard, Transcribe, History, Settings, Hotkeys, Models, Audio, and About.
- `lucide-react` icons installed and used in navigation/actions.
- Frontend production build passing.
- Rust/Tauri environment checks passing in WSL.

Known baseline commands:

```bash
cd /home/natkins/personal/tools/scribe/app
npm run build
npm run tauri info
cd src-tauri && cargo check
```

## Orchestration Rules

- Work in vertical slices when possible.
- Keep each agent inside its ownership lane unless a dependency requires a small shared interface.
- Any agent touching backend commands must update or add matching frontend call sites only when needed for integration.
- Do not widen V1 scope beyond the PRD non-goals.
- Clipboard preservation and Last Transcript Buffer behavior are product invariants.
- Every agent must report:
  - Files changed.
  - Commands run.
  - Known risks.
  - Handoff notes for dependent agents.

## Agent 1 - Backend Foundation

Priority: First  
Branch/scope suggestion: `backend-foundation`

### Mission

Create the Rust backend foundation: explicit app state machine, Last Transcript Buffer type, settings defaults, SQLite migrations, and Tauri commands needed by the frontend shell.

### Inputs

- PRD sections 5, 8, 15, 16, and 18.
- `scribe/app/src-tauri/src/lib.rs`
- `scribe/app/src-tauri/tauri.conf.json`

### Deliverables

- `app_state` module with states:
  - `Idle`
  - `Recording`
  - `Stopping`
  - `Transcribing`
  - `Pasting`
  - `Ready`
  - `Error`
  - `Paused`
- Last Transcript Buffer domain type with metadata.
- Settings schema and default values.
- SQLite migrations for:
  - `transcripts`
  - `settings`
  - `models`
  - `app_stats_daily`
- Tauri commands:
  - `get_app_state`
  - `get_settings`
  - `update_settings`
  - `get_last_transcript`
  - `clear_last_transcript`
  - `list_recent_transcripts`
  - `get_basic_stats`
- Unit tests for state transitions and metadata helpers.

### Constraints

- Do not implement real audio, whisper, hotkey, or paste behavior in this lane.
- Stub data is acceptable only where needed to keep commands callable.
- Never overwrite Last Transcript Buffer with empty text.

### Acceptance

```bash
cd scribe/app/src-tauri
cargo check
cargo test
```

## Agent 2 - Frontend Command Integration

Priority: Second, can start after Agent 1 command shapes are stable  
Branch/scope suggestion: `frontend-command-integration`

### Mission

Replace the current mock UI data with Tauri command calls and local loading/error states.

### Inputs

- Agent 1 command signatures.
- `scribe/app/src/App.tsx`
- `scribe/app/src/App.css`

### Deliverables

- Frontend API wrapper module for Tauri commands.
- Typed frontend models aligned with backend payloads.
- Dashboard reads real app state, settings, last transcript, recent transcripts, and stats.
- Settings controls call backend update commands.
- Clear/Copy/Insert buttons are disabled or marked pending until their backend services exist.

### Constraints

- Keep the existing visual language.
- Do not introduce a router until the app needs URL-addressable routes.
- Keep local state simple; Zustand can wait until backend event streams arrive.

### Acceptance

```bash
cd scribe/app
npm run build
```

Manual check:

- Each sidebar view renders.
- Missing backend services show specific pending states, not broken buttons.

## Agent 3 - Tray and Global Hotkeys

Priority: High, can run in parallel after Agent 1 starts  
Branch/scope suggestion: `tray-hotkeys`

### Mission

Implement system tray behavior and global hotkey registration/rebinding.

### Inputs

- PRD sections 9.1, 9.2, 9.3, and 18.
- Agent 1 settings and state interfaces.

### Deliverables

- Tray menu:
  - Start Dictation
  - Stop Dictation
  - Paste Last Transcript
  - Open Dashboard
  - Open History
  - Settings
  - Quit
- Tray state update mechanism for Idle, Recording, Transcribing, Ready, Error.
- Global hotkey defaults:
  - Hold-to-Talk: `Ctrl + Win + Space`
  - Toggle Dictation: `Ctrl + Win + D`
  - Paste Last Transcript: `Ctrl + Alt + V`
  - Open Dashboard: `Ctrl + Win + H`
- Rebind command with validation and previous-shortcut preservation.
- Spike result on whether Tauri global shortcut supports release events reliably for hold-to-talk.

### Constraints

- If release events are unreliable, document the required Windows low-level keyboard hook implementation.
- Do not implement audio recording beyond calling placeholder backend actions.

### Acceptance

```bash
cd scribe/app/src-tauri
cargo check
```

Manual Windows check:

- Hotkeys trigger while dashboard is unfocused.
- Failed registration gives an actionable message.

## Agent 4 - Audio Capture

Priority: High, parallel after Agent 1 settings exist  
Branch/scope suggestion: `audio-capture`

### Mission

Implement microphone enumeration, test recording, recording lifecycle, level meter data, WAV normalization, and recording duration rules.

### Inputs

- PRD sections 9.4 and 18.
- Agent 1 state/settings interfaces.

### Deliverables

- Microphone list command.
- Selected mic persistence integration.
- Start/stop recording backend service.
- Input level updates for UI.
- Normalized WAV output:
  - 16 kHz
  - mono
  - PCM 16-bit
- Silence trim hook or placeholder interface.
- Ignore recordings under 300 ms.
- Max recording timeout defaulting to 3 minutes.

### Constraints

- Do not call whisper in this lane.
- Keep temporary audio file lifecycle explicit.

### Acceptance

```bash
cd scribe/app/src-tauri
cargo check
cargo test
```

Manual Windows check:

- Select mic.
- Record test WAV.
- Verify WAV can be played or inspected.

## Agent 5 - Whisper and Model Manager

Priority: After Agent 1 and Agent 4 interfaces are stable  
Branch/scope suggestion: `whisper-models`

### Mission

Implement local whisper.cpp transcription and in-app model management.

### Inputs

- PRD sections 9.5, 9.6, 13, and 17.
- Agent 1 database/settings/model metadata.
- Agent 4 normalized WAV output.

### Deliverables

- Model catalog with recommended V1 models.
- Model storage path under `%APPDATA%/Scribe/models/`.
- Download, progress, cancel, retry, delete, select.
- Checksum verification when available.
- whisper.cpp executable path/config.
- Transcription command runner.
- Transcript parsing and metadata capture.
- Last Transcript Buffer update on successful transcription.
- History save when enabled.

### Constraints

- Do not add cloud fallback.
- Do not overwrite the buffer on failed/empty transcript.
- Keep model download user-initiated.

### Acceptance

```bash
cd scribe/app/src-tauri
cargo check
cargo test
```

Manual Windows check:

- Download/select a model.
- Transcribe a recorded WAV locally.
- Last Transcript Buffer updates.

## Agent 6 - Paste and Output System

Priority: After Agent 1, parallel with Agent 5 once buffer exists  
Branch/scope suggestion: `paste-output`

### Mission

Implement output modes and clipboard-preserving paste behavior.

### Inputs

- PRD sections 6, 7, 9.7, and 18.
- Agent 1 Last Transcript Buffer.
- Agent 3 Paste Last hotkey hook.

### Deliverables

- Output modes:
  - Save Only
  - Auto Paste
  - Copy to Clipboard
  - Copy + Paste
- Paste methods:
  - Direct Insert
  - Compatibility Clipboard Paste with restore
- Paste Last Transcript command.
- Clipboard restore error reporting.
- UI command wiring for Insert, Copy, Clear, and output mode changes.

### Constraints

- Direct Insert should be the default.
- Compatibility mode may touch clipboard briefly but must restore previous contents where possible.
- Do not silently fall back in a way that hides clipboard risk from the user.

### Acceptance

Manual Windows check:

- Paste into Notepad, browser input, VS Code, and at least one elevated/admin app.
- Clipboard remains preserved after paste-last in default mode.
- Compatibility mode restores previous clipboard content.

## Agent 7 - History, Stats, and Retention

Priority: After Agent 1 and Agent 5 produce transcript records  
Branch/scope suggestion: `history-stats`

### Mission

Complete local transcript history, stats aggregation, and retention behavior.

### Inputs

- PRD sections 9.8, 9.9, and 15.
- Agent 1 migrations/repositories.
- Agent 5 transcript creation.

### Deliverables

- Searchable history command.
- View, copy, insert, edit, delete, clear all actions.
- Retention policy enforcement.
- Daily stats aggregation.
- Dashboard stats from local records:
  - Words today
  - Dictations today
  - Average WPM
  - Average latency
  - Average recording duration
  - Most used model
  - Total words

### Constraints

- History is enabled by default.
- Raw audio saving remains disabled by default.

### Acceptance

```bash
cd scribe/app/src-tauri
cargo test
cd ..
npm run build
```

## Agent 8 - UX Polish, Notifications, and Packaging

Priority: Final V1 hardening  
Branch/scope suggestion: `polish-packaging`

### Mission

Finish the floating pill, notifications, error states, tray icon variants, installer build, and acceptance checklist.

### Inputs

- PRD sections 9.10, 12, 17, 18, 20, and 21.
- All prior agent outputs.

### Deliverables

- Floating pill overlay states:
  - Idle
  - Recording
  - Transcribing
  - Ready
  - Error
- Optional native notifications.
- Optional sounds setting.
- Tray icon state variants.
- Installer build configuration.
- V1 acceptance checklist report.
- Known issues and follow-up list.

### Constraints

- Keep feedback lightweight and disable-able.
- Do not add non-goal features while polishing.

### Acceptance

```bash
cd scribe/app
npm run build
npm run tauri build
```

Manual Windows check:

- Full MVP acceptance criteria from the PRD.

## Recommended Assignment Order

1. Agent 1 - Backend Foundation
2. Agent 3 - Tray and Global Hotkeys
3. Agent 4 - Audio Capture
4. Agent 2 - Frontend Command Integration
5. Agent 5 - Whisper and Model Manager
6. Agent 6 - Paste and Output System
7. Agent 7 - History, Stats, and Retention
8. Agent 8 - UX Polish, Notifications, and Packaging

## Parallel Work Plan

Start together:

- Agent 1 owns the backend contracts.
- Agent 3 can spike tray/hotkeys with temporary commands.
- Agent 4 can spike audio recording in isolation.

Start after first contracts stabilize:

- Agent 2 wires UI to Agent 1 commands.
- Agent 5 uses Agent 4 WAV output and Agent 1 model persistence.
- Agent 6 uses Agent 1 buffer state and Agent 3 hotkey path.

Finish after feature lanes merge:

- Agent 7 completes data reuse and metrics.
- Agent 8 hardens the full Windows experience.

## Integration Checkpoints

### Checkpoint A - Backend Contract Freeze

Required before frontend integration:

- State payload type.
- Settings payload type.
- Last Transcript Buffer payload type.
- Transcript history row type.
- Error payload shape.

### Checkpoint B - First Vertical Slice

Required before model manager polish:

1. Trigger recording.
2. Save normalized WAV.
3. Run whisper.cpp on that WAV.
4. Update Last Transcript Buffer.
5. Insert buffer into Notepad without permanently overwriting clipboard.

### Checkpoint C - V1 Candidate

Required before packaging:

- All PRD MVP acceptance criteria pass.
- Privacy defaults verified.
- Non-goals reviewed.
- Installer smoke-tested on Windows 10/11.
