# Settings map

Every Scribe setting: what it connects to, which speech engine it applies to, what happens when it is off or can't apply, and how it affects other settings.
Field names are the ones in `AppSettings` (`app/src/backend.ts`, `app/src-tauri/src/settings.rs`).
"UI label" says how the app explains a setting that another one turns off or makes pointless; the tests for those labels are in `app/src/components/settingsLabels.test.tsx`.

## How a dictation flows

1. A hotkey, the tray or Start begins a recording from the microphone (`selectedMicId`).
2. The dictation pins the speech model selected at that moment (`selectedModelId`, see `src-tauri/src/session_model.rs`), and keeps it for every live segment and the final text.
3. With live transcription on, phrases are transcribed while you talk; otherwise the whole recording is transcribed when you stop.
4. Whisper gets custom words as its prompt; Parakeet gets none.
5. Filler removal runs on the engine's timed words.
6. Replacements run on the finished text.
7. The text is inserted, copied or only saved (`outputMode`, `pasteMethod`), then saved to history (`historyEnabled`).

## Speech engine and model

### Model (`selectedModelId`)

- Connects to: the engine (Whisper or Parakeet, from the model catalog), the resident engine process, Home's Model card and setup row.
- Engine: chooses it.
- Off or unavailable: with no downloaded model, dictation can't start and Home shows "Download a model to start dictating".
- Implications:
  - Changing it during a dictation applies from the next dictation; the new model loads in the background afterwards when it is kept loaded.
  - Parakeet ignores custom words, language and translate.
  - English-only Whisper models (`.en`) ignore any language other than English.
- UI label: Models says "used from your next dictation" while you dictate. Words says "Not used by Parakeet" with a Change model link.

### Language (`language`) and Translate to English (`translateToEnglish`)

- Connects to: whisper-server and whisper-cli launch arguments.
- Engine: Whisper only. Parakeet detects which of its 25 languages you speak.
- Off or unavailable: with Parakeet the Language row shows "Detected" and Translate is hidden.
- Implications: anything but English, auto-detect, and translate need a multilingual Whisper model; an English-only model ignores them.
- UI label: "Your model only understands English, so this won't take effect" under Language and Translate when the model is English-only.

### GPU acceleration (`gpuAcceleration`) and Graphics card (`gpuDeviceIndex`)

- Connects to: the engine launch (`--no-gpu` when off) and the Vulkan device it may use (`GGML_VK_VISIBLE_DEVICES`).
- Engine: both.
  - Whisper's speedup is measured: Large v3 Turbo on an RX 7800 XT went from 11 s to 0.1 to 0.7 s for an 11 s clip.
  - Parakeet's is not measured yet.
- Off or unavailable:
  - Off, everything runs on the processor.
  - With no usable card, Scribe falls back to the processor by itself.
- Implications:
  - Changing either restarts the loaded engine.
  - The graphics card choice does nothing while GPU acceleration is off.
  - Large models are too slow for live dictation without a card.
- UI label: "Off because Use GPU acceleration is off" under Graphics card. Home's setup row flags GPU Off in amber with Turn on when a card is available.

### Keep the speech model loaded (`keepWhisperLoaded`, `keepParakeetLoaded`)

- Connects to: the resident engine process and its memory.
- Engine: one switch per engine, and the Models page shows the one for the engine in use.
- Off or unavailable:
  - Off, the model loads at the start of each dictation and unloads after an idle spell.
  - Whisper can't stay loaded on macOS and Linux; it runs once per dictation there.
- Implications:
  - On, dictation starts at once and the memory stays in use.
  - Off, memory is freed and the first dictation after a pause is slower (base.en loads in about 0.5 s on the owner's PC, small.en in about 1 s).
- UI label: the Models page shows the measured memory and load time. On macOS and Linux it says Whisper loads for each dictation.

## Words

### Custom words (`vocabularyPrompt`)

- Connects to: Whisper's prompt (`--prompt` on the command line, `prompt` on the server), through `src-tauri/src/prompt_budget.rs`.
- Engine: Whisper only. Parakeet has no prompt or hotword support.
- Off or unavailable:
  - Empty, there is no prompt.
  - With Parakeet the words are kept but not used.
- Implications:
  - whisper.cpp reads at most 223 prompt tokens.
  - The UI limit is 450 characters, at most about 155 tokens on measured hints.
  - Scribe trims a longer hint by tokens and keeps its front.
  - Live transcription's context shares the same 223 tokens and gets only the room the words leave.
  - The words are carried into every 30 s window of a long recording.
  - A sentence works better than a comma list, which makes Whisper write in list style.
- UI label: "Not used by Parakeet" with a Change model link. A red counter and "Over by N characters" when too long. Home shows "Not used" with Parakeet.

### Replacements (`replacements.txt`)

- Connects to:
  - the finished text of every dictation, after filler removal;
  - the file `replacements.txt` in the data folder, which is the source of truth (`src-tauri/src/replacements_file.rs`).
- Engine: both. They are the only lever for your words with Parakeet.
- Off or unavailable:
  - An empty file changes nothing.
  - Lines that aren't `heard -> written` are skipped and reported by line number.
- Implications:
  - No limit.
  - Matching is case-insensitive on whole words, longer phrases first.
  - Only add real mishearings; spacing and case variants read fine as they are.
  - File transcription doesn't apply them.
  - Edits in any editor are picked up within about 2 s.
  - App writes are atomic and only add or remove their own lines.
  - The list that settings kept before 0.15.1 (`textReplacements`) is moved into the file once and kept as a backup; the setting is no longer read.
  - GitHub backup doesn't include the file.
- Format: `docs/REPLACEMENTS.md`, also behind the Format link in the app.
- UI label:
  - "No limit, and they work with every model".
  - "N lines skipped: ..." for unreadable lines.

### Remove filler words (`fillerSuppressionEnabled`), Filler words (`fillerWords`), Pause before removing (`fillerPauseThresholdMs`)

- Connects to: the engine's timed words (Whisper's JSON with word times, Parakeet's word list).
- Engine: both.
- Off or unavailable: off, the words list and the pause do nothing.
- Implications:
  - It runs before replacements, so a replacement never sees a removed filler.
  - A word only goes when you paused around it.
- UI label: "Off because Remove filler words is off" under the pause.

## Microphone and recording

### Microphone (`selectedMicId`)

- Connects to: the recording input.
- Engine: both.
- Off or unavailable: when the saved microphone isn't connected, Scribe records from the system default, else the first input.
- Implications: Home and the Microphone page show the device actually used.
- UI label: "Saved mic not connected" on Home's Microphone card. The Microphone page explains the fallback.

### Silence trim (`silenceTrimEnabled`)

- Connects to: the recording before transcription.
- Engine: both.
- Off or unavailable: off, leading and trailing quiet is kept.

### Auto-stop on silence (`silenceAutoStopEnabled`), Auto-stop pause (`silenceAutoStopMs`), Silence stops any recording after (`maxRecordingMs`)

- Connects to: hands-free recordings (toggle, tray, Start) and, past `maxRecordingMs`, hold-to-talk too.
- Engine: both.
- Off or unavailable: off, the pause length and the hold-to-talk limit do nothing.
- UI label: "Off because Auto-stop on silence is off" under both.

### Shortest recording (`minRecordingMs`) and Hard stop (`recordingCeilingMs`)

- Connects to: every recording.
- Engine: both.
- Implications: the hard stop keeps the text.

### Live transcription (`incrementalTranscriptionEnabled`), Split on pause (`segmentPauseEnabled`), Pause before splitting (`segmentPauseMs`), Longest phrase (`segmentMaxMs`)

- Connects to: phrase-by-phrase transcription while you talk (`src-tauri/src/incremental.rs`).
- Engine:
  - Both.
  - Whisper also gets the last 200 characters already said as context, inside the custom words budget.
  - Parakeet gets no context.
- Off or unavailable:
  - Off, the whole recording is transcribed at stop, and the three phrase settings do nothing.
  - Split on pause off makes its pause length do nothing.
- Implications:
  - Phrases are cut at 25 s at most, which keeps Whisper inside one 30 s window.
  - Each Whisper phrase costs a full window of work, so live transcription trades some processor time for text that is ready when you stop.
- UI label:
  - "Off because Live transcription is off" under the three phrase settings.
  - "Off because Split on pause is off" under its pause.
  - Home's setup row flags Live transcription Off with Turn on.

## Dictation output

### After dictation (`outputMode`) and Insert method (`pasteMethod`)

- Connects to: the app you dictate into, and the clipboard.
- Engine: both.
- Off or unavailable:
  - Insert method only applies to Insert at cursor, and is hidden otherwise.
  - Notes are never inserted.
  - On macOS and Wayland, pasting needs a permission; until it's granted the text is copied instead.
- UI label: the permission row explains it, and Home's After dictation card names the method in use.

### Show floating pill (`showFloatingPill`), Pill style (`pillDisplayMode`), pill colors

- Connects to: the pill window.
- Off or unavailable: off, the style and colors do nothing.
- UI label: "Not shown while Show floating pill is off" under Pill style.

## History and data

### Keep history (`historyEnabled`), Keep dictations for (`historyRetentionDays`), Keep notes for (`notesRetentionDays`), Save audio clips (`saveAudioClips`)

- Connects to: the local database and the clips folder.
- Engine: both.
- Off or unavailable: with Keep history off, new dictations aren't saved to history; notes and their clips still are.
- Implications: clips are deleted with their dictation, or after the dictation retention for notes.
- UI label: "Keep history is off: new dictations aren't saved, notes still are" under Keep dictations for and Save audio clips.

## Local AI

### Use local AI (`notesAnalysisEnabled`), Server address, Model, Analysis prompt

- Connects to: the OpenAI-compatible server, note analysis, the transform selection hotkey.
- Engine: neither (text only).
- Off or unavailable: off, the server, model and prompt are not used and the transform hotkey isn't registered.
- UI label:
  - "Not used while Use local AI is off" under the server, model and prompt.
  - Hotkeys gives the reason on the Transform selection row.

## GitHub backup

### Backup (`githubSyncEnabled`), Repository (`githubRepo`), Back up every transcript (`githubSyncAllTranscripts`)

- Connects to: the GitHub App sign-in and the private repository.
- Off or unavailable: without a connected account, nothing is uploaded and the backup controls say so.

## App

### Launch at startup, Minimize to tray, Open Scribe hotkey hides it (`dashboardHotkeyToggles`), Notifications, Sounds, Developer settings

- Connects to: the operating system and the window.
- Implications: Open Scribe hotkey hides it only matters when the Open Scribe hotkey is bound.

### Automatically check for updates (`autoUpdateCheckEnabled`) and Install updates automatically (`autoInstallUpdates`)

- Connects to: the GitHub release feed.
- Off or unavailable:
  - With checks off, automatic install does nothing.
  - The macOS and Linux betas don't install updates themselves.
- UI label: "Off because Automatically check for updates is off" under Install updates automatically.

### Theme, custom theme, window size (`windowWidth`, `windowHeight`, `windowSizeLogical`)

- Connects to: the main window.
- Implications: the saved size is in logical pixels and shrinks to fit the screen at launch.

## Labels still missing

- Open Scribe hotkey hides it has no note when the Open Scribe hotkey is unbound.
- Back up every transcript is not labeled while backup is off; the panel says the account isn't connected instead.
- Parakeet's GPU speedup is not measured, so the GPU row doesn't say how much it helps Parakeet.
