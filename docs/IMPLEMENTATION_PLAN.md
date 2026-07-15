# Scribe - Implementation Plan

> **Archived plan:** This is the original 2026-06-10 engineering outline.
> It does not describe the current repository structure or shipped feature set.
> See the [documentation index](README.md) for current guidance.

Status: Initial engineering outline  
Source PRD: [PRD.md](./PRD.md)  
Date: 2026-06-10

## 1. Working Assumptions

- V1 is Windows-first. macOS/Linux compatibility should not drive design decisions yet.
- The app should be implemented as a Tauri v2 desktop app with React, TypeScript, Vite, Rust, and SQLite.
- whisper.cpp should be integrated first as a bundled/configured executable invoked by Rust. Direct bindings can come later.
- Clipboard preservation is a product requirement, not a polish item.
- The Last Transcript Buffer is a first-class domain object and should not be modeled as clipboard state or only as the latest history row.
- The first implementation should optimize for reliability and debuggability over maximum transcription speed.

## 2. Suggested Repository Shape

```text
scribe/
  docs/
    PRD.md
    IMPLEMENTATION_PLAN.md
  app/
    package.json
    src/
      app/
      components/
      features/
        dashboard/
        history/
        hotkeys/
        models/
        audio/
        settings/
      lib/
      styles/
    src-tauri/
      Cargo.toml
      tauri.conf.json
      capabilities/
      migrations/
      src/
        main.rs
        app_state.rs
        commands/
        db/
        hotkeys/
        audio/
        whisper/
        models/
        paste/
        tray/
        settings/
```

This keeps the current docs separate from the future app scaffold. If we scaffold immediately, use `scribe/app` as the Tauri project root.

## 3. Architecture Outline

### Frontend

- React + TypeScript + Vite.
- Tailwind CSS with local component primitives. shadcn/ui is optional, but a compact custom system may be simpler for this utility.
- Zustand for UI and cached app state unless Tauri command/query synchronization becomes complex enough to justify TanStack Query.
- Frontend state mirrors backend state but does not own recording, paste, hotkey, model, or transcription behavior.

### Backend

- Tauri v2 Rust app.
- SQLite for settings, transcripts, model metadata, and stats.
- Rust-managed app state machine with explicit transitions.
- Rust commands exposed to the UI for settings, history, models, audio devices, test recording, paste, and dictation control.
- Background services for tray, hotkeys, recording, transcription, and model download.

### Domain Boundaries

- `app_state`: app lifecycle state, Last Transcript Buffer, current error, current status.
- `hotkeys`: registration, validation, rebinding, press/release hook fallback.
- `audio`: device enumeration, recording, level meter, normalization, trim, duration validation.
- `whisper`: model path resolution, CLI invocation, transcript parsing, latency capture.
- `models`: model catalog, download, checksum, delete, select.
- `paste`: direct insert, clipboard restore paste, fallback/error handling.
- `db`: migrations, settings, history, stats.
- `tray`: tray menu and status icon updates.

## 4. Milestone Plan

### Milestone 0 - Decisions and Spikes

Goal: retire the riskiest unknowns before broad feature work.

Deliverables:

- Confirm Tauri v2 plugin set and Windows compatibility.
- Spike global shortcut hold/release behavior.
- Spike Windows direct text insertion.
- Spike audio recording to normalized 16 kHz mono PCM WAV.
- Spike whisper.cpp CLI invocation from Rust with a known local model.

Exit criteria:

- We know whether Tauri global-shortcut is enough for hold-to-talk or a Windows keyboard hook is required.
- We know the preferred Rust crates/APIs for direct insert and compatibility paste.
- We can produce a valid WAV and transcribe it locally from the backend.

### Milestone 1 - Skeleton App

Goal: runnable desktop shell with state, tray, settings, and dashboard shell.

Deliverables:

- Tauri + React + TypeScript project in `scribe/app`.
- Premium dark dashboard shell with sidebar.
- Backend state machine with initial commands.
- SQLite migrations for settings, transcripts, models, and daily stats.
- Tray menu with stub actions.
- Settings load/save with defaults.

Exit criteria:

- App launches on Windows.
- Dashboard shows mocked or initial real state.
- Tray opens dashboard and quits.
- Settings persist across restart.

### Milestone 2 - Hotkeys and Recording

Goal: user can start/stop recordings from hotkeys and produce test WAV files.

Deliverables:

- Register default global hotkeys.
- Rebind, validate, and preserve previous shortcut on registration failure.
- Implement toggle recording.
- Implement hold-to-talk using plugin or lower-level hook.
- Enumerate microphones.
- Record selected mic to temporary WAV.
- Level meter and test recording UI.
- Min/max recording duration rules.

Exit criteria:

- Hotkeys work outside the app window.
- Accidental recordings under 300 ms are ignored.
- A valid normalized WAV file is created for valid recordings.

### Milestone 3 - Whisper and Last Transcript Buffer

Goal: completed recordings become local transcripts in the buffer.

Deliverables:

- Configure/bundle whisper.cpp executable path.
- Model path setting.
- Run transcription command from Rust.
- Parse transcript output.
- Store successful transcript in Last Transcript Buffer.
- Never overwrite buffer on empty/failed transcript.
- Save transcript metadata to history when enabled.

Exit criteria:

- User records speech and sees transcript in the dashboard.
- Failure states are visible and action-oriented.
- Last Transcript Buffer metadata is complete.

### Milestone 4 - Output and Paste System

Goal: transcript output modes work with clipboard-preserving default behavior.

Deliverables:

- Save Only, Auto Paste, Copy to Clipboard, Copy + Paste modes.
- Paste Last Transcript hotkey.
- Direct Insert paste method.
- Compatibility Clipboard Paste with restore.
- Last Transcript Buffer actions: Insert, Edit, Copy, Clear.
- Clipboard status labels.

Exit criteria:

- Paste-last inserts the buffer into another app.
- Default paste path does not permanently overwrite clipboard.
- Compatibility paste restores previous clipboard data where possible.

### Milestone 5 - Model Manager

Goal: user can manage local Whisper models from the app.

Deliverables:

- Model catalog with recommended initial entries.
- Download, progress, cancel, retry.
- Delete and select default model.
- Model status transitions.
- Model storage under `%APPDATA%/Scribe/models/`.
- Checksum verification where checksums are available.

Exit criteria:

- A fresh install can download/select a model and use it for transcription.
- Missing model errors link to a clear recovery path.

### Milestone 6 - History and Stats

Goal: local archive and useful dashboard metrics.

Deliverables:

- History page with search.
- View, copy, insert, edit, delete, clear all.
- Retention settings.
- Dashboard recent transcripts.
- Daily and aggregate stats from history records.

Exit criteria:

- User can find and reuse prior transcripts.
- Stats update after each successful transcription.
- Retention policy can clean old records.

### Milestone 7 - Polish and Packaging

Goal: V1 feels like a daily-driver Windows utility.

Deliverables:

- Floating recording pill.
- Notifications and optional sounds.
- Better tray icon state variants.
- Empty states and error recovery.
- Installer build.
- Startup behavior and minimize-to-tray.
- Manual QA pass on Windows 10/11.

Exit criteria:

- MVP acceptance criteria from the PRD pass.
- App handles common failures without crashing.
- Installer launches a functional tray app.

## 5. Sub-Agent Workstreams

These are written as assignable briefs. Each agent should return code changes, test notes, risks, and follow-up tasks.

### Agent A - Tauri Scaffold and App Shell

Scope:

- Create `scribe/app` Tauri v2 + React + TypeScript + Vite project.
- Add Tailwind and base dark theme tokens from the PRD.
- Build dashboard layout with sidebar sections.
- Add frontend route/view structure for Dashboard, Transcribe, History, Settings, Hotkeys, Models, Audio, About.

Inputs:

- [PRD.md](./PRD.md) sections 10 and 11.
- This plan sections 2 and 4.

Outputs:

- Runnable app shell.
- Basic component primitives: card, pill, keycap, segmented control, toggle, compact row, status dot.
- Design notes for remaining UI agents.

Dependencies:

- None after project scaffold decision.

### Agent B - Backend State, Settings, SQLite, and Tray

Scope:

- Implement Rust app state machine.
- Add Last Transcript Buffer domain type.
- Add SQLite migrations and repositories.
- Add settings defaults and persistence.
- Add tray menu and tray state update plumbing.

Inputs:

- [PRD.md](./PRD.md) sections 5, 8, 9.1, 15, and 16.

Outputs:

- Tauri commands for app state, settings, transcript buffer, and initial history reads.
- Tray menu actions wired to backend commands where possible.
- Unit tests for state transitions and settings serialization.

Dependencies:

- Agent A scaffold.

### Agent C - Windows Hotkeys and Paste Integration

Scope:

- Register/rebind global hotkeys.
- Validate shortcuts and preserve old bindings on failure.
- Prove hold-to-talk press/release behavior.
- Implement direct text insertion.
- Implement compatibility clipboard paste with restore.
- Wire Paste Last Transcript command.

Inputs:

- [PRD.md](./PRD.md) sections 7, 8, 9.2, 9.3, and 18.

Outputs:

- Hotkey service with error reporting.
- Paste service with direct and compatibility methods.
- Manual test matrix across Notepad, browser text field, VS Code, and an elevated app if possible.

Dependencies:

- Agent B state machine and settings interfaces.

### Agent D - Audio Capture and Normalization

Scope:

- Enumerate microphones.
- Implement selected device recording.
- Add level meter stream.
- Normalize to 16 kHz mono PCM WAV.
- Add trim and min/max duration validation.
- Add test recording and optional playback support.

Inputs:

- [PRD.md](./PRD.md) sections 9.4 and 18.

Outputs:

- Audio service with backend commands.
- Temporary WAV lifecycle.
- Tests or validation scripts for WAV format.

Dependencies:

- Agent B settings interfaces.

### Agent E - Whisper and Model Manager

Scope:

- Add whisper.cpp executable configuration.
- Run local transcription from Rust.
- Parse output into transcript text.
- Implement model catalog and model storage path.
- Add download/progress/cancel/delete/select.
- Add checksum verification where available.

Inputs:

- [PRD.md](./PRD.md) sections 9.5, 9.6, 13, and 17.

Outputs:

- Whisper service.
- Model repository/service.
- Model Manager UI integration points.
- Recovery path for missing model and transcription failures.

Dependencies:

- Agent D WAV output.
- Agent B settings and model metadata persistence.

### Agent F - Dashboard, History, Stats, and UX Polish

Scope:

- Build final dashboard information hierarchy.
- Prominently render Last Transcript Buffer.
- Build History page and actions.
- Build Stats cards.
- Build floating recording pill.
- Add notification/sound settings UI.

Inputs:

- [PRD.md](./PRD.md) sections 9.7, 9.8, 9.9, 9.10, 10, 11, and 12.

Outputs:

- Polished UI wired to backend commands.
- Empty, loading, ready, recording, transcribing, and error states.
- Responsive checks for dashboard views.

Dependencies:

- Agent A shell.
- Agent B state/history/settings commands.
- Agent C paste commands.
- Agent E model state.

### Agent G - QA, Packaging, and Release Readiness

Scope:

- Create manual QA matrix.
- Add smoke tests where practical.
- Verify Windows 10/11 behavior.
- Configure installer build.
- Validate app data paths.
- Validate privacy defaults.
- Confirm non-goals are not accidentally introduced.

Inputs:

- [PRD.md](./PRD.md) sections 17, 18, 20, and 21.

Outputs:

- QA checklist.
- Packaging notes.
- Known issues list.
- V1 acceptance report.

Dependencies:

- All feature milestones.

## 6. Parallelization Map

Can start immediately:

- Agent A: scaffold and UI shell.
- Agent B: state/settings/database design after scaffold exists.
- Agent C: standalone Windows hotkey/paste spike.
- Agent D: standalone audio capture spike.
- Agent E: standalone whisper.cpp invocation spike.

Should wait:

- Final Model Manager depends on persistence and app shell.
- Final History/Stats depends on database and transcript creation.
- Final Output System depends on hotkeys, paste, and Last Transcript Buffer.
- Packaging depends on stable Tauri config and plugin set.

Recommended order:

1. Run risk spikes for hotkeys, paste, audio, and whisper.
2. Scaffold app and backend state/persistence.
3. Merge recording -> transcription -> buffer as the first vertical slice.
4. Add paste-last and output modes.
5. Add model manager, history, stats, and polish.

## 7. First Vertical Slice

The fastest meaningful slice is:

1. Launch tray app.
2. Open dashboard.
3. Select/use a fixed microphone.
4. Trigger toggle recording from a button or hotkey.
5. Save normalized WAV.
6. Run whisper.cpp against a manually configured model path.
7. Show transcript in Last Transcript Buffer.
8. Press Paste Last Transcript and insert into Notepad without permanently overwriting clipboard.

This slice proves the product promise before investing in full model management and dashboard polish.

## 8. Testing Strategy

### Unit tests

- State machine transitions.
- Settings serialization/defaults.
- Shortcut validation.
- Transcript metadata calculation.
- Stats aggregation.
- Model status transitions.

### Integration tests

- SQLite migrations and repository methods.
- Whisper command invocation with fixture WAV where possible.
- Model download state transitions with mocked downloader.
- Paste compatibility clipboard restore with controlled clipboard contents.

### Manual Windows tests

- Tray lifecycle.
- Hotkeys while app is unfocused.
- Hold-to-talk start/stop behavior.
- Toggle recording behavior.
- Paste into Notepad, browser, VS Code, Office-style app, and elevated app.
- Clipboard preservation after paste-last.
- Microphone unavailable and permission-denied states.
- Missing model and failed transcription states.
- Installer install/uninstall.

## 9. Key Risks

- Tauri global shortcut may not expose reliable key release events for true hold-to-talk.
- Direct insertion may be inconsistent across Windows apps, especially elevated/admin windows.
- Clipboard restore timing can be fragile.
- Audio capture and normalization crates may require careful device-format handling.
- whisper.cpp binary distribution and model download URLs/checksums need licensing and packaging review.
- Tauri plugin APIs may shift by version; pin versions during scaffold.

## 10. Immediate Next Actions

1. Scaffold `scribe/app` as Tauri v2 + React + TypeScript + Vite.
2. Create the Rust app state machine and SQLite migrations early.
3. Run four focused spikes: hotkey hold/release, direct insert, audio WAV generation, whisper.cpp invocation.
4. Implement the first vertical slice before widening the UI.
5. Use the sub-agent briefs above once the scaffold and boundaries exist.
