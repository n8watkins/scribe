# LocalDictate - Product Requirements Document

Status: Reference PRD  
Date recorded: 2026-06-10  
Target platform: Windows 10/11 desktop  
Stack target: Tauri v2, React, TypeScript, Rust, SQLite, whisper.cpp

## 1. Product Summary

LocalDictate is a private, local-first Windows speech-to-text desktop app built with Tauri. The app lets users press a global hotkey, speak, transcribe locally using Whisper, and either paste the transcript immediately, copy it to the clipboard, or save it into an internal "Last Transcript Buffer" that can be inserted later with a separate hotkey.

Core product promise:

> A fast, private, local Windows dictation utility that lets users speak text into any app without being forced to overwrite their clipboard.

This is not intended to be a full WhisperType competitor. The target is a polished daily-driver utility for personal productivity.

## 2. Primary Goals

### Must achieve

- Run as a lightweight Windows tray app.
- Use local Whisper transcription.
- Support push-to-talk dictation.
- Support toggle start/stop dictation.
- Allow user to select microphone.
- Allow user to select/download local Whisper models.
- Store the most recent transcript in an internal Last Transcript Buffer.
- Allow user to paste the Last Transcript Buffer with a dedicated hotkey.
- Allow pasting without permanently overwriting the system clipboard.
- Provide simple transcript history.
- Provide basic stats.
- Provide a polished, premium dark-mode dashboard.

### Must not become in V1

- A real-time streaming transcription app.
- A cloud transcription app.
- A team/collaboration tool.
- A wake-word assistant.
- A full AI writing assistant.
- A command-mode automation app.

## 3. Target Platform

### V1 platform

- Windows 10/11 desktop.
- Tauri desktop app.
- Local-first, offline-capable after model download.

### Later platforms

- macOS and Linux may be considered later, but V1 should optimize for Windows reliability.

## 4. Core User Flow

### Main dictation flow

1. User presses or holds the configured dictation hotkey.
2. App enters `Recording` state.
3. User speaks.
4. User releases hotkey or presses toggle hotkey again.
5. App stops recording.
6. App trims beginning/end silence.
7. App ignores accidental recordings under 300 ms.
8. App saves temporary audio file.
9. App transcribes audio locally using Whisper.
10. App stores the transcript in the Last Transcript Buffer.
11. App optionally saves transcript to history.
12. App performs the selected output behavior:
    - Save Only
    - Auto Paste
    - Copy to Clipboard
    - Copy + Paste

### Paste-last flow

1. User has a previous transcript stored in the Last Transcript Buffer.
2. User presses the configured Paste Last Transcript hotkey.
3. App inserts the transcript into the active focused app.
4. Clipboard remains untouched by default.
5. If direct insertion fails or compatibility mode is enabled, app may use temporary clipboard paste and restore previous clipboard afterward.

## 5. Key Product Concept: Last Transcript Buffer

The Last Transcript Buffer is a core primitive, separate from clipboard and history.

### Definition

The Last Transcript Buffer is an internal app state slot containing the most recent completed transcription.

```ts
lastTranscript = {
  id: "tx_123",
  text: "This is the most recent dictated text.",
  createdAt: "2026-06-10T10:42:00",
  durationMs: 8400,
  wordCount: 42,
  charCount: 231,
  modelId: "small.en-q5_1",
  language: "en"
}
```

### Important distinction

- Clipboard: OS-level user clipboard.
- Last Transcript Buffer: current app memory slot.
- History: longer-term archive of prior transcripts.

### Required behavior

After every successful transcription:

1. Update Last Transcript Buffer.
2. Optionally save to history.
3. Trigger selected output behavior.

The user must be able to press a separate hotkey to insert the Last Transcript Buffer without needing to copy it to the clipboard.

## 6. Output Modes

### Save Only

The transcript is saved to the Last Transcript Buffer and optionally to history. No paste occurs. Clipboard is untouched.

Use case: user wants to review, edit, or paste later.

### Auto Paste

The transcript is inserted into the active focused app immediately after transcription completes.

User setting determines insertion method:

- Direct Insert
- Compatibility Clipboard Paste

### Copy to Clipboard

The transcript is copied to the OS clipboard.

Use case: user explicitly wants the transcript on clipboard.

### Copy + Paste

Optional advanced mode. Transcript is copied to clipboard and pasted. This should not be the default.

## 7. Paste Methods

### Direct Insert Mode

Default preferred paste method. The app injects text into the active focused window without modifying the clipboard.

Pros:

- Preserves clipboard.
- Matches core product promise.
- Feels clean and controlled.

Cons:

- May be less reliable in some apps.
- Can be slower for long text.
- May fail in elevated/admin apps depending on Windows permissions.

### Compatibility Clipboard Paste Mode

Fallback method.

Flow:

1. Read current clipboard contents.
2. Store clipboard contents temporarily.
3. Set clipboard to transcript text.
4. Send paste command.
5. Restore previous clipboard contents.

Pros:

- More reliable across apps.
- Good fallback for problematic apps.

Cons:

- Technically touches clipboard briefly.
- Needs careful timing.
- Clipboard restore may fail in edge cases.

### Required setting

```text
Paste Method:
- Direct Insert, preserve clipboard
- Compatibility Paste, restore clipboard afterward
```

## 8. App State Machine

The app should use an explicit state machine to prevent duplicate recordings, stuck mic states, or repeated pastes.

### States

```text
Idle
Recording
Stopping
Transcribing
Pasting
Ready
Error
Paused
```

### Behavior

- `Idle` -> hotkey down -> `Recording`
- `Recording` -> hotkey up / toggle stop -> `Stopping`
- `Stopping` -> valid audio -> `Transcribing`
- `Stopping` -> audio too short -> `Idle`
- `Transcribing` -> success -> update buffer -> selected output behavior
- `Transcribing` -> failure -> `Error`
- `Pasting` -> complete -> `Ready`
- `Ready` -> timeout -> `Idle`
- `Error` -> reset action -> `Idle`

### Rules

- Ignore new recording hotkey while `Transcribing`.
- Allow cancel during `Recording`.
- Show visible feedback for all major states.
- Prevent duplicate paste after a single transcription.
- Never overwrite Last Transcript Buffer with empty or failed transcript.

## 9. MVP Feature Requirements

### 9.1 Tray App

The app must run in the Windows system tray.

Tray menu items:

- Start Dictation
- Stop Dictation
- Paste Last Transcript
- Open Dashboard
- Open History
- Settings
- Quit

Tray states:

- Idle
- Recording
- Transcribing
- Ready
- Error

The tray icon should visually reflect the current state.

### 9.2 Global Hotkeys

Required default hotkeys:

```text
Hold-to-Talk: Ctrl + Win + Space
Toggle Dictation: Ctrl + Win + D
Paste Last Transcript: Ctrl + Alt + V
Open Dashboard: Ctrl + Win + H
```

Hotkeys must be rebindable.

Hotkey settings should:

- Capture new shortcut.
- Validate shortcut.
- Detect registration failure.
- Show conflict message if shortcut cannot be registered.
- Preserve previous shortcut if registration fails.

If true hold/release behavior is unreliable through the global shortcut plugin alone, implement a lower-level Windows keyboard hook for hold-to-talk behavior.

### 9.3 Recording Modes

Support two recording modes.

#### Hold-to-talk

- Press and hold hotkey to record.
- Release to stop and transcribe.

#### Toggle mode

- Press hotkey once to start recording.
- Press again to stop and transcribe.

User setting:

```text
Recording Mode:
- Hold-to-talk
- Toggle start/stop
- Both enabled
```

### 9.4 Audio Recording

Requirements:

- Microphone selector.
- Input level meter.
- Test recording button.
- Optional playback of test recording.
- Recording timeout.
- Silence trim.
- Ignore accidental recordings under 300 ms.

Recommended defaults:

```text
Minimum recording duration: 300 ms
Max recording duration: 3 minutes
Sample rate target: 16 kHz
Channels: mono
Format for Whisper: WAV / PCM 16-bit
```

If recording device outputs a different format, normalize internally before passing to Whisper.

### 9.5 Whisper Transcription Engine

Recommended V1 engine:

```text
whisper.cpp
```

Implementation options:

- Bundle whisper.cpp executable and call it from Rust.
- Later migrate to direct Rust/C++ binding if needed.

V1 recommended flow:

1. Record audio.
2. Convert/normalize to expected WAV format.
3. Run local whisper.cpp command.
4. Capture output.
5. Parse transcript.
6. Store transcript in Last Transcript Buffer.
7. Save metadata.

Example conceptual command:

```bash
whisper-cli -m models/ggml-small.en-q5_1.bin -f temp/input.wav --language en --output-txt
```

The actual command should be adapted to the bundled whisper.cpp binary and app paths.

### 9.6 Model Manager

The app should provide an in-app model manager.

Model states:

- Not Downloaded
- Downloading
- Downloaded
- Selected
- Loaded
- Failed
- Update Available

Model manager features:

- Download model.
- Delete model.
- Select default model.
- Show model size.
- Show model location.
- Show download progress.
- Cancel download.
- Verify file checksum/hash if available.
- Retry failed download.

Recommended initial model list:

```text
tiny.en
base.en
small.en
small.en quantized
medium.en
large-v3-turbo quantized
```

Recommended default:

```text
small.en quantized
```

Model storage path:

```text
%APPDATA%/LocalDictate/models/
```

### 9.7 Last Transcript Buffer UI

The dashboard must prominently show the Last Transcript Buffer.

Card content:

- Transcript text.
- Word count.
- Character count.
- Time created.
- Duration.
- Model used.
- Clipboard status.

Required actions:

- Insert
- Edit
- Copy
- Clear

Required status labels:

```text
Clipboard Untouched
Clipboard Preserved
Copied to Clipboard
```

When clipboard mode is active, the label should clearly change to `Copied to Clipboard`.

### 9.8 Transcript History

History should be local and optional.

Defaults:

- Save transcript history: enabled.
- Save raw audio: disabled.

History item fields:

- id
- text
- created_at
- duration_ms
- word_count
- character_count
- model_id
- language
- output_mode
- paste_method
- transcription_latency_ms

History actions:

- View
- Search
- Copy
- Insert
- Edit
- Delete
- Clear all history

Retention settings:

- 7 days
- 30 days
- 90 days
- Forever
- Manual only

### 9.9 Basic Stats

Dashboard should show:

- Words today.
- Dictations today.
- Average WPM.
- Average transcription latency.
- Average recording duration.
- Most used model.
- Total words transcribed.

Stats should be computed locally from history records.

### 9.10 Notifications and Feedback

The app should provide lightweight feedback for key events.

Examples:

- Recording started.
- Recording stopped.
- Transcribing.
- Transcript ready.
- Pasted.
- Saved to Last Transcript Buffer.
- Model downloaded.
- Error occurred.

Feedback methods:

- Tray icon state.
- Floating pill overlay.
- Optional native notification.
- Optional start/stop sound.

User should be able to disable sounds and notifications.

## 10. Dashboard UX Requirements

The dashboard should use the structure of a clean Windows utility with the visual palette of a premium dark AI developer tool.

### Layout

Use a left sidebar and main dashboard card layout.

Sidebar items:

- Dashboard
- Transcribe
- History
- Settings
- Hotkeys
- Models
- Audio
- About

Main dashboard sections:

1. Current Status
2. Active Microphone
3. Active Whisper Model
4. Output Mode
5. Last Transcript Buffer
6. Hotkeys
7. Recent Transcripts
8. Basic Stats

### Information hierarchy

The Last Transcript Buffer should be one of the most important cards.

Status, mic, model, output behavior, and hotkeys should be immediately visible.

The dashboard should answer:

```text
Am I recording?
What mic am I using?
What model am I using?
Where did my last transcript go?
Will this touch my clipboard?
What hotkeys do I press?
```

## 11. Visual Design Specification

### Design direction

Combine:

- Structure of a clean Windows 11 utility.
- Visual palette of a premium dark AI developer dashboard.

### Style keywords

```text
Premium
Local
Private
Fast
Calm
Dark-mode
Developer-friendly
Trustworthy
Daily-driver utility
```

### Palette

```text
Background: #070B14
Surface 1: #0D1320
Surface 2: #111827
Surface 3: #172033
Border: rgba(148, 163, 184, 0.18)

Primary Cyan: #22D3EE
Primary Blue: #3B82F6
Accent Violet: #8B5CF6
Success Green: #22C55E
Warning Amber: #F59E0B
Danger Red: #EF4444

Text Primary: #F8FAFC
Text Secondary: #CBD5E1
Text Muted: #64748B
```

### UI styling

- Dark navy / near-black background.
- Glassy or softly frosted cards.
- Rounded corners.
- Thin borders.
- Soft shadows.
- Subtle cyan/violet glow around selected or active items.
- Minimal icons.
- Crisp typography.
- Compact spacing, but not cramped.
- Tiny waveform/audio visualizations where useful.

### Avoid

- Cartoon style.
- Fake brand logos.
- Oversized marketing hero sections.
- Heavy gradients.
- Excessive neon.
- Dense settings walls.
- Visual clutter.

### Typography

Recommended fonts:

- Inter
- Segoe UI Variable
- Geist
- system-ui fallback

Readable sizes:

- Page title: 24-28 px.
- Card title: 14-16 px.
- Body text: 14-16 px.
- Metadata: 12-13 px.
- Button text: 13-14 px.

### Component language

- Cards
- Pills
- Segmented controls
- Keycap hotkey badges
- Status dots
- Subtle waveform bars
- Toggle switches
- Select dropdowns
- Compact list rows

## 12. Floating Recording Pill

A small floating pill should appear during recording/transcribing.

States:

- Idle
- Recording
- Transcribing
- Ready
- Error

Recording state:

```text
* Recording...
```

Transcribing state:

```text
Transcribing...
```

Ready state:

```text
Transcript ready
Ctrl+Alt+V to insert
```

Save-only mode ready state:

```text
Saved to Last Transcript
Clipboard preserved
```

The floating pill should be subtle, centered near the top of the screen, and disappear after a short timeout when no longer needed.

## 13. Technical Architecture

### Frontend

Recommended:

- React
- TypeScript
- Vite
- Tailwind CSS
- shadcn/ui or custom component system
- Zustand or TanStack Store for UI state
- TanStack Query if async command/state syncing becomes complex

Frontend responsibilities:

- Dashboard UI.
- Settings UI.
- History UI.
- Model manager UI.
- Hotkey editor.
- Status display.
- Calling Tauri commands.
- Rendering transcript buffer and stats.

### Backend

Recommended:

- Tauri v2
- Rust command handlers
- whisper.cpp integration
- SQLite persistence
- Windows input/paste integration
- Audio capture/normalization pipeline
- Model download manager

Backend responsibilities:

- Register global hotkeys.
- Manage tray.
- Capture audio.
- Normalize audio.
- Run Whisper transcription.
- Manage model files.
- Store transcript history.
- Store settings.
- Insert text into active app.
- Preserve clipboard when compatibility paste is used.

## 14. Suggested Tauri Plugins / Capabilities

Recommended Tauri capabilities:

- Global Shortcut plugin for global hotkeys.
- System tray support for tray utility behavior.
- SQL plugin with SQLite for local transcript history/settings.
- Notification plugin for optional native notifications.
- Autostart plugin for launch at startup.
- Updater plugin for future app updates.
- Shell/process command support for calling bundled whisper.cpp binary if using external executable approach.
- File system/path APIs for app data directories and model storage.

## 15. Local Data Storage

### SQLite

Use SQLite for:

- Transcript history.
- Stats.
- Settings.
- Model metadata.
- Hotkey preferences.

### Suggested tables

#### transcripts

```sql
CREATE TABLE transcripts (
  id TEXT PRIMARY KEY,
  text TEXT NOT NULL,
  created_at TEXT NOT NULL,
  duration_ms INTEGER,
  word_count INTEGER,
  character_count INTEGER,
  model_id TEXT,
  language TEXT,
  output_mode TEXT,
  paste_method TEXT,
  transcription_latency_ms INTEGER
);
```

#### settings

```sql
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

#### models

```sql
CREATE TABLE models (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  filename TEXT NOT NULL,
  local_path TEXT,
  size_bytes INTEGER,
  status TEXT NOT NULL,
  checksum TEXT,
  selected INTEGER DEFAULT 0,
  downloaded_at TEXT
);
```

#### app_stats_daily

```sql
CREATE TABLE app_stats_daily (
  date TEXT PRIMARY KEY,
  dictation_count INTEGER DEFAULT 0,
  word_count INTEGER DEFAULT 0,
  total_recording_ms INTEGER DEFAULT 0,
  total_transcription_latency_ms INTEGER DEFAULT 0
);
```

## 16. Settings Schema

```ts
type AppSettings = {
  launchAtStartup: boolean;
  minimizeToTray: boolean;
  showFloatingPill: boolean;
  notificationsEnabled: boolean;
  soundsEnabled: boolean;

  recordingMode: "hold" | "toggle" | "both";
  minRecordingMs: number;
  maxRecordingMs: number;
  silenceTrimEnabled: boolean;

  selectedMicId: string | null;
  selectedModelId: string | null;
  language: "auto" | "en";

  outputMode: "save_only" | "auto_paste" | "copy_clipboard" | "copy_and_paste";
  pasteMethod: "direct_insert" | "clipboard_restore";

  historyEnabled: boolean;
  saveAudioClips: boolean;
  historyRetentionDays: 7 | 30 | 90 | 365 | null;

  hotkeys: {
    holdToTalk: string;
    toggleDictation: string;
    pasteLastTranscript: string;
    openDashboard: string;
  };
};
```

## 17. Security and Privacy Requirements

### Local-first

- Transcription should happen locally.
- Audio should not leave the device.
- Transcript history should remain local.
- Models should be downloaded only when user requests or confirms.

### Defaults

```text
Cloud transcription: disabled / not present
Save transcript history: enabled
Save raw audio: disabled
Clipboard preservation: enabled
Notifications: enabled
Telemetry: disabled
```

### User controls

- Clear transcript history.
- Clear Last Transcript Buffer.
- Delete downloaded models.
- Disable transcript history.
- Disable notifications.
- Open local data folder.
- Reset all settings.

## 18. Error Handling

Required error states:

- No microphone selected.
- Microphone permission denied.
- Microphone unavailable.
- Recording failed.
- Audio too short.
- Whisper model missing.
- Whisper transcription failed.
- Model download failed.
- Hotkey registration failed.
- Paste failed.
- Clipboard restore failed.
- App database error.

Error messages should be specific and action-oriented.

Example:

```text
Could not register Ctrl+Alt+V. Another app may already be using this shortcut. Choose a different hotkey.
```

Example:

```text
Selected Whisper model is missing. Re-download the model or choose another model.
```

## 19. V1 Build Phases

### Phase 1 - Skeleton App

- Create Tauri + React + TypeScript app.
- Add dashboard shell.
- Add sidebar.
- Add tray.
- Add settings persistence.
- Add basic app state machine.

### Phase 2 - Hotkeys and Recording

- Register global hotkeys.
- Implement hold-to-talk.
- Implement toggle recording.
- Add mic selector.
- Add recording status.
- Save test WAV file.

### Phase 3 - Whisper Integration

- Bundle or configure whisper.cpp.
- Add model path setting.
- Run transcription on recorded WAV.
- Parse transcript output.
- Update Last Transcript Buffer.

### Phase 4 - Output System

- Add output modes.
- Implement direct insert.
- Implement compatibility clipboard paste with restore.
- Add Paste Last Transcript hotkey.
- Add Last Transcript Buffer actions.

### Phase 5 - Model Manager

- Add model list.
- Add download progress.
- Add model delete/select.
- Add model status.
- Add checksum verification if available.

### Phase 6 - History and Stats

- Add SQLite transcript history.
- Add history page.
- Add recent transcripts card.
- Add daily stats.

### Phase 7 - Polish

- Floating recording pill.
- Notifications.
- Better error states.
- Theme polish.
- Installer build.
- Optional updater support.

## 20. MVP Acceptance Criteria

The app is considered V1-ready when:

- User can install and launch the app on Windows.
- App can run in the tray.
- User can select a microphone.
- User can download/select a Whisper model.
- User can hold a hotkey to record.
- User can release the hotkey to transcribe.
- Transcript appears in Last Transcript Buffer.
- User can paste last transcript using Ctrl+Alt+V.
- User can use paste-last without permanently overwriting clipboard.
- User can choose Save Only, Auto Paste, or Copy to Clipboard.
- User can view recent transcripts.
- User can see basic stats.
- User can rebind hotkeys.
- User receives clear feedback when recording/transcribing.
- App handles common failures without crashing.

## 21. Non-Goals for V1

Do not include in V1:

- Real-time streaming transcription.
- Wake word.
- Cloud fallback.
- Speaker diarization.
- AI rewriting.
- Translation mode.
- Team sync.
- Mobile app.
- Browser extension.
- Per-app profiles.
- Command mode.
- Voice macros.
- Multi-user accounts.
- Subscription/payment system.

## 22. Final Product Feel

LocalDictate should feel like:

- A serious Windows productivity utility.
- A private local AI tool.
- A fast daily-driver dictation app.
- A calm, premium desktop assistant.
- A tool that respects the user's clipboard.

The defining product idea is:

> Dictate locally, save the result into an internal Last Transcript Buffer, and insert it later with a dedicated hotkey without consuming the clipboard.

That should remain the core around which the product is designed.
