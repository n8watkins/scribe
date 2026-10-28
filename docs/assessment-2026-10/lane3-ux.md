# Lane 3: UI and UX assessment

Written 2026-10-28 against `main` at `1d88fe9` (v0.8.0 plus 15 unreleased commits).
Scope: every view of the main window, the floating pill, first run, empty and error states, information architecture, window size, the friction walkthroughs the owner asked for, consistency, wording, and accessibility.

## How this was done

The React UI was run in Chromium with Vite and a mocked Tauri IPC layer that answers every command the front end calls with realistic data (12 transcripts, 5 notes with AI summaries, 11 catalog models with 3 downloaded, 2 microphones, 2 GPUs, a connected GitHub account).
Scenarios switch the mock into first run (no model, empty history), no microphone, local AI server down, GitHub disconnected or not configured, an app error, and recording.
Scroll depth was measured per view from the main scroll container, and axe-core 4.14 was run on every view in Midnight and Pewter.
Every finding below was checked against the source, and the file and line are given.

The tooling is kept next to this report so the captures can be redone after changes: `tools/mock-ipc.js` and `tools/capture.cjs` (run with the Vite dev server on port 1477; the script points at a global Playwright install and would need its `require` path adjusted on another machine).

Limits of this method, to keep in mind while reading:

- Fonts: Chromium on Linux fell back to a generic sans; on Windows the stack resolves to Segoe UI Variable, so line breaks will differ by a few characters.
- The floating pill is a separate transparent, always-on-top window that resizes itself; the browser shows it on white and cannot reproduce the text mode's growth, so pill findings come mostly from the code.
- The tray menu, the native window chrome, the update overlay's live download, SmartScreen, and real hotkey behavior cannot be captured this way.
  Those need the real-app pass on Windows that the main session is arranging.
- Items marked "confirm on Windows" are strongly suggested by the code but not observed in the running app.

## Summary

Scribe's core loop is solid and the visual language is coherent: a dark, quiet, consistent card system, clear toggles, good inline explanations, and real attention to edge cases in the copy.
The problems are structural rather than cosmetic.

1. **The navigation has grown to 13 entries (14 with Developer) organised by implementation, not by task.**
   Settings for one job are split across up to four views: audio clips live in both Audio and Data & Privacy, pill colors live in Settings while Themes points to "Audio → Pill", language lives in Settings while the model that decides whether language works lives in Models, and the window size lives in Data & Privacy.
2. **The first run does not tell the truth about readiness.**
   With no model downloaded the Dashboard still says "Ready for dictation", the Start button stays enabled, the default model is badged "Active" next to its own Download button, and the mic card shows a green dot even with zero microphones.
   The Setup checklist exists but is hidden in About → Setup and shows no status.
3. **Three of the views scroll badly at the default 960x680 window.**
   Audio is 2.75 screens tall, Settings (App & output) 2.46, Sync 1.6, Data & Privacy 1.3; History, Notes and Dashboard overflow by about 30 px, which clips the pager.
   The cause is mostly density and layout (raw millisecond fields, eight-line descriptions in a narrow column, one long tab that mixes five topics), not the window size.
4. **Several daily-use frictions are real and fixable:** the toggle key is the backquote key and is swallowed system-wide (developers cannot type a backtick while Scribe runs), Replacements cannot express a new line and give no way to test a rule, the "type it out" paste method has no control, file transcription needs a typed path, and the typed Transform panel on the Dashboard very likely copies from Scribe's own window.
5. **Some wording overpromises privacy** ("Nothing leaves this machine" next to a field that accepts any OpenAI-compatible URL; "nothing is sent to any server" in About while GitHub backup and update checks exist), and 32 user-visible strings use the em dash.

The recommended direction: regroup the navigation around jobs (about 9 entries plus a settings search), make first run a guided three-step flow with live status, then fix the friction list.
The top 10 changes are ranked at the end.

## Screenshot index

All files are in `screens/mock/`.
Default size is 960x680, the app's configured window size (`app/src-tauri/tauri.conf.json:18-19`); `--full` files show the whole scrollable view in one image.

| View or state | Files |
|---|---|
| Dashboard | `01-dashboard.png`, `01-dashboard--full.png`, `15-dashboard-recording.png` |
| Transcribe | `02-transcribe.png`, `02-transcribe-file.png` |
| History | `03-history.png`, `03-history--full.png` |
| Notes | `04-notes.png`, `04-notes--full.png` |
| Sync | `05-sync.png`, `05-sync--full.png` |
| Stats | `06-stats.png` |
| Settings | `07-settings.png`, `07-settings--full.png`, `07-settings-dictionary.png`, `07-settings-notes-llm-connected.png` |
| Data & Privacy | `08-data-privacy.png`, `08-data-privacy--full.png` |
| Hotkeys | `09-hotkeys.png`, `09-hotkeys-capturing.png` |
| Models | `10-models.png`, `10-models-browse.png`, `10-models-browse--full.png` |
| Audio | `11-audio.png`, `11-audio--full.png` |
| Themes | `12-themes.png` |
| About | `13-about.png`, `13-about-what-scribe-does.png`, `13-about-setup.png` |
| Developer | `14-developer.png` |
| Floating pill (recording) | `pill-dot-recording.png`, `pill-visualizer-recording.png`, `pill-visualizer_with_text-recording.png` |
| First run (no model, empty history) | `state-first-run-dashboard.png`, `state-first-run-models.png`, `state-first-run-models--full.png` |
| Empty states | `state-empty-history.png`, `state-empty-notes.png`, `state-empty-stats.png`, `state-empty-dictionary.png` |
| Error states | `state-no-mic-dashboard.png`, `state-no-mic-audio.png`, `state-llm-down.png`, `state-llm-down-notes.png`, `state-github-disconnected.png`, `state-github-unconfigured.png`, `state-app-error-dashboard.png` |
| Smallest window (820x540) | `size-min-820x540-dashboard.png`, `size-min-820x540-settings.png`, `size-min-820x540-hotkeys.png` |
| Large window (1440x900) | `size-large-1440x900-dashboard.png`, `size-large-1440x900-settings.png` |
| Themes on the Dashboard | `theme-pewter-dashboard.png`, `theme-ocean-dashboard.png`, `theme-emerald-dashboard.png`, `theme-custom-dashboard.png` |
| Pewter (light) across views | `theme-pewter-settings.png`, `theme-pewter-history.png`, `theme-pewter-models.png`, `theme-pewter-hotkeys.png`, `theme-pewter-sync.png`, `theme-pewter-themes.png` |
| Keyboard focus | `a11y-focus-tab3.png` |

Not captured (needs the real app): the update overlay, the tray menu, the pill's text mode while it grows, notifications, and SmartScreen.

Measured scroll depth (main area height divided by visible height; 1.00 means no scroll), from `lane3-scroll-metrics.json`:

| View | 960x680 | 820x540 | 1440x900 |
|---|---|---|---|
| Audio | 2.75 | n/a | n/a |
| Settings, App & output | 2.46 | 3.34 | 1.74 |
| Sync | 1.60 | n/a | n/a |
| Data & Privacy | 1.29 | n/a | n/a |
| History and Notes | 1.05 (pager clipped) | 1.13 | 1.00 |
| Dashboard | 1.04 | 1.74 | 1.00 |
| Hotkeys | 1.00 | 1.33 | 1.00 |
| Everything else | 1.00 | n/a | n/a |

Models and History also have inner scroll areas, so a 1.00 there can still hide a nested scrollbar (the model list scrolls inside its card).

## Information architecture

### What is wrong with the current structure

- Thirteen flat entries, ordered roughly by when they were built.
  Settings-type pages (Settings, Data & Privacy, Hotkeys, Models, Audio, Themes, Sync) make up seven of them with no grouping.
- One job is spread over several pages, and two settings appear twice:
  - "Save raw audio clips" is in Data & Privacy (`app/src/views/DataPrivacy.tsx:193`) and again as "Save raw audio" in Audio (`app/src/views/Audio.tsx:405`).
  - The data folder path is in About and in Data & Privacy.
  - Pill colors are in Settings → App & output, while Themes says "set those in Audio → Pill" (`app/src/views/Themes.tsx:98`), a page that does not exist.
  - Language and Translate are in Settings, but whether they work depends on the model chosen in Models.
  - Filler suppression is in App & output, while its sibling text-cleanup features (context hint, replacements) are in the Dictionary tab.
  - The window size is in Data & Privacy.
  - History's "Settings" button opens Settings → App & output (`app/src/views/History.tsx:479`), but history retention lives in Data & Privacy.
  - Export and Restore live in Sync, though they need no account and are about your data.
- Settings → "App & output" is five topics in one scrolling tab: output, language, filler removal, the floating pill, and app behavior.
- Two topbar buttons sit on every page: "History", which is redundant with the sidebar and shows even on the History page, and "Start", which duplicates the hotkeys and the Transcribe view.
- Stats is one card of seven numbers on its own page.
- The Setup checklist, the most useful page for a new user, is a tab inside About.

### Is the June "Integrations" view still right?

Partly.
GitHub backup and the local AI server are the only two things that connect to something outside Scribe, and grouping them is right.
But the June idea of moving all of Sync under Integrations would also move Export and Restore, which are not integrations.
And the local AI server is not a feature in itself; it powers two features (Notes analysis and Transform selection) whose on/off switches and hotkey should live with the server connection, so turning AI off also unbinds Ctrl+Alt+R.

Recommendation: an "Integrations" group with two pages, "Local AI" and "GitHub backup", plus a separate "Your data" page for retention, folders, export and restore.
Use plain names; "Local AI" reads better than "Local LLM" for most users.
The managed llama-server idea from the backlog (section B) would slot into "Local AI" as a third connection option ("Built in") later without another IA change.

### Proposed navigation

| Group | Proposed entry | What it holds | Comes from today |
|---|---|---|---|
| Use | Home | Status with honest readiness, last transcript, today's stats, quick setup checklist until complete | Dashboard, Stats, About → Setup |
| Use | History | Dictations and Notes as two tabs of one view, with day grouping | History, Notes |
| Use | Transcribe a file | File picker and drag-and-drop, progress, output | Transcribe → file tab |
| Set up | Dictation | Output (insert, clipboard, paste or type), hotkeys summary, pill on/off and style, live transcription and auto-stop basics | Settings → Output and Pill, Transcribe → Output behavior, Audio → processing basics |
| Set up | Words | Context hint, replacements, filler removal, spoken punctuation later | Settings → Dictionary, Settings → Filler |
| Set up | Microphone | Device, level test, device health, advanced timing in a disclosure | Audio |
| Set up | Models and language | Model picker with guidance, language, translate, GPU | Models, Settings → Language, Audio → GPU |
| Set up | Hotkeys | Unchanged in role, denser rows | Hotkeys |
| Integrations | Local AI | Server connection, model, prompt, and the features it powers (Notes analysis, Transform selection with its hotkey) | Settings → Notes, Dashboard Transform panel |
| Integrations | GitHub backup | Connect, repo, what to back up, status | Sync (GitHub parts) |
| App | Your data | Retention, audio clips, folders, export, restore, clear | Data & Privacy, Sync → Export and Restore |
| App | Appearance | Themes, custom theme, pill colors, window size | Themes, Settings → Pill colors, Data & Privacy → Window |
| App | General | Startup, tray, notifications, sounds, updates, developer mode | Settings → App, About → Updates |
| App | About | Version, release notes, links, licenses | About |

That is 14 rows but in four labelled groups, with the three "Use" entries on top; the flat list today has no grouping at all.
Developer stays opt-in under General.
Add a settings search box (or Ctrl+K) at the top of the sidebar that jumps to any setting by name; with about 70 settings this is the cheapest way to end "where is it".

## Findings by view

Severity: **High** blocks, misleads, or risks data or privacy; **Medium** is daily friction; **Low** is polish.
Effort: **S** is under a day, **M** is a few days, **L** is a week or more.

### Dashboard

#### D1. Readiness is not honest (High, S)
- What: with no model downloaded, the Status card reads "Ready for dictation", Start stays enabled, and the setup banner above contradicts both.
- Where: `state-first-run-dashboard.png`; `app/src/lib/format.ts:85` (`canStartRecording` ignores model and mic readiness); `app/src/lib/format.ts:297`.
- Why: the first thing a new user does is press the hotkey; it fails with an error toast instead of being prevented with a reason.
- Fix: derive one readiness state from model, mic and hotkey registration; show "Needs a model" in the Status card, disable Start with a tooltip, and make the banner the single call to action.

#### D2. The mic card's green dot is hard-coded (High, S)
- What: the "Active mic" card always shows a green success dot, even with no microphones at all.
- Where: `state-no-mic-dashboard.png`; `app/src/views/Dashboard.tsx:81`.
- Why: it tells the user the mic is fine when it is not.
- Fix: color the dot from the selected mic's `isAvailable`, and show "No microphone found" when the list is empty.

#### D3. The Active model card shows the internal id (Medium, S)
- What: "small.en-q5_1" instead of "Small English Q5_1"; Stats does the same and truncates it ("small.en-q...").
- Where: `01-dashboard.png`, `06-stats.png`; `app/src/views/Dashboard.tsx:89`; `app/src/components/layout.tsx:31`.
- Fix: look up the catalog name; Models already does this.

#### D4. "Needs attention" without saying what (Medium, S)
- What: in the Error state the Status card says "Needs attention" with no reason and no action.
- Where: `state-app-error-dashboard.png`.
- Fix: show `appState.error.message` under the status and a button that goes to the place that fixes it.

#### D5. Layout leaves an orphan card and a large empty panel (Low, S)
- What: four status cards in a three-column grid leave "Auto-insert" alone on a second row; the Last Transcript card stretches to the height of the right column with empty space below its buttons; the page overflows by 28 px at the default size.
- Where: `01-dashboard.png`, `01-dashboard--full.png`, `size-large-1440x900-dashboard.png`.
- Fix: three cards (fold Auto-insert into the Last Transcript card's header, where it belongs) or a four-column grid at this width; let the left card size to content and put today's stats under it.

#### D6. "Last transcript buffer" wording and badges (Medium, S)
- What: the card title "Ready to insert later" shows even when the text was already auto-inserted; the "Clipboard restored" badge shows even when no transcript exists; the three action buttons are icons only.
- Where: `01-dashboard.png`, `state-first-run-dashboard.png`; `app/src/components/transcript.tsx:53`; `app/src/lib/format.ts:61`.
- Fix: title it "Last dictation" with a status line ("Inserted into Slack at 11:50"), hide the badge when there is nothing, and label the buttons ("Insert again", "Copy", "Clear").

#### D7. The typed Transform panel probably transforms the wrong text (High, M, confirm on Windows)
- What: the Dashboard panel says "Highlight text in any app, describe the change"; clicking its button sends Ctrl+C to the focused window, which at that moment is Scribe itself.
- Where: `app/src/views/Dashboard.tsx:139-175`; `app/src-tauri/src/commands.rs:1398-1400` (no step re-focuses the previous app); `app/src-tauri/src/selection_transform.rs:91`.
- Why: the visible, discoverable entry point to the feature likely never works; the hotkey path is the only one that can.
- Fix: either remember the previously focused window and restore focus before copying, or remove the panel and make the hotkey open a small floating instruction box near the cursor; move the feature's settings to Local AI.
  The Hotkeys hint "Or type one in the Dashboard panel" (`app/src/views/Hotkeys.tsx:36`) should change with it.

#### D8. The panel shows even when local AI is off (Medium, S)
- What: the Transform selection panel is shown whether or not a local AI server is configured; nothing in the front end gates it, which matches the handoff's open problem about Ctrl+Alt+R being bound with AI off (the backend binding is for Lanes 1 and 4 to confirm).
- Where: `01-dashboard.png` with `notesAnalysisEnabled` off; `app/src/views/Dashboard.tsx`.
- Fix: hide both until Local AI is connected; register the hotkey only then.

#### D9. A toast for every recording start (Low, S)
- What: "Recording with Realtek." appears on every dictation and covers the Transform panel; the mic name is cut at "(R)".
- Where: `15-dashboard-recording.png`; `app/src/App.tsx:631`.
- Fix: show it only when the mic changed since last time, and use the full device name.

#### D10. No live feedback in the main window while recording (Low, M)
- What: during recording the Dashboard only changes the status text; the live transcript appears only in Transcribe and the pill.
- Where: `15-dashboard-recording.png`.
- Fix: show the live text and level in the Last dictation card while recording.

### Transcribe

#### T1. File transcription needs a typed path (High, S)
- What: a text field with placeholder `C:\Users\you\Videos\meeting.mp4` and no Browse button or drop zone.
- Where: `02-transcribe-file.png`; `app/src/views/Transcribe.tsx:378-380`.
- Why: almost nobody types a full path; this was already backlog item O3/O4.
- Fix: a file picker button plus drag-and-drop on the whole view; show progress, duration and a Save as text button.

#### T2. "Need ffmpeg on PATH" (Medium, M)
- What: MP4, MKV and M4A only work if the user installs ffmpeg and edits PATH.
- Where: `02-transcribe-file.png`.
- Fix: bundle an LGPL ffmpeg build, or offer a one-click download into the app folder; at minimum say "Install ffmpeg (link) to open video files" and detect it.

#### T3. The Record tab duplicates the Dashboard (Medium, M)
- What: the Record tab repeats the Last transcript card, a Start button (the third on screen with the topbar's), and two output toggles whose descriptions are truncated behind "See more".
- Where: `02-transcribe.png`.
- Fix: in the proposed IA, recording lives on Home and output settings in Dictation; Transcribe becomes "Transcribe a file" only.

### History and Notes

#### H1. Row actions are an unlabeled grid with Delete in the top row (Medium, S)
- What: five or six icon buttons per row in a 3x2 grid; Delete sits next to Play; the grid reflows when a row has no audio, so buttons move between rows.
- Where: `03-history.png`, `04-notes.png`; `app/src/components/transcript.tsx:197-255`.
- Fix: two visible actions (Insert, Copy) and a "More" menu for Play, Open in editor, Analyze, Delete; keep positions fixed.

#### H2. Delete uses the browser's native confirm (Medium, S)
- What: `window.confirm("Delete this transcript from local history?")` instead of the app's own modal.
- Where: `app/src/views/History.tsx:382`.
- Why: in WebView2 this is an unstyled system dialog, unlike every other confirmation in the app.
- Fix: delete immediately and offer Undo in a toast, which is faster and safer.

#### H3. The pager is clipped and shown when there is one page (Low, S)
- What: Previous and Next are cut off at the bottom edge at the default size and appear even with 7 of 7 records.
- Where: `03-history.png`, `state-empty-history.png`.
- Fix: hide the pager for a single page, or switch to infinite scroll.

#### H4. "Clear all" sits beside the search box (Medium, S)
- What: the destructive red button is the first control after search.
- Where: `03-history.png`; `app/src/views/History.tsx:470`.
- Fix: move it to Your data; it already has a confirmation modal there.

#### H5. Notes analysis renders as raw text with a shouting model id (Low, S)
- What: the summary's bullets are shown as literal "- " lines, and the header is the model id in capitals ("GEMMA-3-4B-IT").
- Where: `04-notes.png`.
- Fix: render the Markdown subset (bullets, bold), and label it "AI summary" with the model in small text.

#### H6. Empty states don't teach (Low, S)
- What: "No local transcript records match this view yet." with Select all, Clear all and a pager still shown.
- Where: `state-empty-history.png`, `state-empty-notes.png`.
- Fix: "Hold Ctrl + Win and speak to make your first dictation" (and the note chord for Notes), and hide the list controls.

#### H7. Dates are absolute only, and the toolbar is crowded (Low, S)
- What: every row says "Oct 27, 2026, 11:50 PM"; the date filters take a full row with "mm/dd/yyyy" placeholders.
- Fix: group by Today, Yesterday, This week; collapse the date range into one "Any time" dropdown.

#### H8. No way to correct a transcript into a rule (Medium, M)
- What: when Whisper mishears a name, the user must remember it, go to Settings → Dictionary, and type a rule.
- Fix: let the user edit a transcript in place (the `update_transcript` command already exists in `app/src/backend.ts`) and offer "Always replace 'clawed' with 'Claude'?" from the edit; Wispr Flow learns its dictionary this way.

### Settings: App & output

#### S1. One tab, five topics, 2.5 screens (High, M)
- What: Output, Language, Filler suppression, Floating pill and App all scroll in one tab.
- Where: `07-settings--full.png`; `app/src/views/Settings.tsx:86-396`.
- Fix: the regrouping in the IA section; each resulting page fits in one screen.

#### S2. The "type it out" paste method has no control (High, S)
- What: the open problem is confirmed: `pasteMethod` (including `direct_insert`, which types the text as keystrokes) has no control anywhere in the UI.
- Where: `rg pasteMethod app/src` finds only `app/src/lib/format.ts:57` and `app/src/backend.ts`.
- Why: apps that block paste (some terminals, remote desktops, games, password-protected fields) have no workaround.
- Fix: an "Insert method" choice: "Paste (fast, keeps your clipboard)", "Paste and leave it on the clipboard", "Type it out (for apps that block paste)"; this also replaces the two hard-to-read toggles in S3.

#### S3. Two toggles whose descriptions need careful reading (Medium, S)
- What: "Auto-insert after dictation" and "Keep my clipboard" encode a four-way output mode in two toggles with three-line, em-dash-filled descriptions.
- Where: `07-settings.png`; `app/src/views/Settings.tsx:95-130`.
- Fix: one segmented control: "Insert at cursor", "Copy only", "Save only", plus the insert method from S2.

#### S4. Language is selectable when it cannot work (Medium, S)
- What: any language and Translate can be chosen with an English-only model; a muted notice appears below.
- Where: `07-settings.png`; `app/src/views/Settings.tsx:165-171`.
- Fix: backlog item F is still right: move Language and Translate next to the model picker, disable them with "Needs a multilingual model, download one" and a direct button.

#### S5. Pill display mode select is truncated (Low, S)
- What: "Visualizer + te" is cut off at every window size; Sync's "Markdown (.md" and Audio's "Automatic (recommende" and "Default input device (Re" have the same problem.
- Where: `07-settings--full.png`, `05-sync.png`, `11-audio--full.png`.
- Fix: give selects a min-width from their longest option, or shorten the labels.

#### S6. Launch at startup is still off by default (Medium, S)
- What: backlog G2 asked for on; `app/src-tauri/src/settings.rs:641` still defaults it to false.
- Why: a dictation tool that is not running when you need it is a missed dictation; Wispr Flow and SuperWhisper start at login by default.
- Fix: ask in first run ("Start Scribe with Windows? Recommended"), default yes.

### Settings: Dictionary (Replacements walkthrough)

The owner named Replacements as awkward; walking through it as a new user:

1. Find it: Settings → second tab "Dictionary" → scroll past the context hint. Nothing in History or the Dashboard points here.
2. Add one: "Add replacement" adds an empty row and saves it immediately (an empty "from" is ignored, so empty rows pile up silently).
3. Try "new line" → line break: impossible; the "Replace with" field is one line and the text is inserted verbatim with no escapes (`app/src-tauri/src/text_replace.rs` applies `to` literally). See the empty row in `07-settings-dictionary.png`.
4. Test it: there is no way to try a rule without dictating into another app and checking.
5. Check it worked later: History shows only the final text, with no hint a rule fired.
6. Manage 30 rules: no search, no sort, no enable/disable, no import or export, no duplicate warning.
7. Case: the replacement is inserted exactly as typed, so a rule for "scribe app" → "Scribe" capitalizes correctly mid-sentence but a lowercase replacement at the start of a sentence stays lowercase.

#### R1. Replacements need a v2 (High, M)
- Fix, in order of value:
  - A one-line test box at the top: type or paste a sentence, see the result live.
  - Multi-line "Replace with" (or `\n` and `\t` escapes shown as chips) so "new line" and "new paragraph" work.
  - "Add from History" (see H8) so rules come from real mistakes.
  - Enable toggle per row, a duplicate warning, and a search when there are more than about 10 rules.
  - Import and export as CSV.
  - Optional "match sentence case" per rule.
- Where: `app/src/views/Settings.tsx:769-885`, `app/src-tauri/src/text_replace.rs`.

#### R2. The context hint's explanation is good; its placement is not (Low, S)
- What: the explanation of hint versus replacement is clear, but the em dashes and "primes Whisper" are jargon.
- Fix: "Names and words Scribe should expect" as the label; keep the example.

#### R3. Filler removal belongs with these (Low, S)
- What: filler words are a text-cleanup rule like replacements but live in App & output.
- Fix: move into the proposed "Words" page.

### Settings: Notes (local AI)

#### N1. The model field ignores what Test connection finds (Medium, S)
- What: Test connection lists the server's models as pills, but they are not clickable; the Model field is free text.
- Where: `07-settings-notes-llm-connected.png`; `app/src/views/Settings.tsx:475-487`, `629-637`.
- Fix: test automatically on open, and turn the Model field into a dropdown filled from the server, with "Use whatever is loaded" as the first option.

#### N2. The error message breaks into fragments (Low, S)
- What: when the server is down, the inline `<code>` elements render as full-width blocks, splitting the sentence into ". Start LM Studio..." and "). Connection refused (os error 10061)".
- Where: `state-llm-down.png`; `app/src/views/Settings.tsx:648-656`; `.llm-status-card code` at `app/src/App.css:1763`.
- Fix: keep `code` inline, and replace the OS error with "Nothing answered at that address".

#### N3. "Nothing leaves this machine" is not guaranteed (High, S)
- What: the toggle's description promises nothing leaves the machine, while the endpoint field accepts any URL, including a cloud API.
- Where: `app/src/views/Settings.tsx:450`.
- Fix: "Notes are sent only to the server below. With the default local address, nothing leaves this computer." and warn when the host is not localhost.

#### N4. The endpoint field is too narrow to read (Low, S)
- What: "http://127.0.0.1:1" is all that shows.
- Where: `07-settings-notes-llm-connected.png`.
- Fix: full-width input under the label.

### Data & Privacy

#### P1. Broken header layout in Local data (Medium, S)
- What: "Folders on this device" and "where Scribe keeps your data" are laid out as two wrapped centered columns.
- Where: `08-data-privacy.png`; `app/src/views/DataPrivacy.tsx:213-222`.
- Fix: backlog H1 to H3 still apply: one heading "Local data" with a one-line description.

#### P2. Paths break mid-word and the Window card collapses (Medium, S)
- What: paths wrap as "co / m.natkins.scribe"; the Window card squeezes "Default window size" into one word per line.
- Where: `08-data-privacy--full.png`.
- Fix: `overflow-wrap: anywhere` only at path separators (or truncate in the middle with a tooltip, backlog H6); move the window setting out (see W below).

#### P3. Destructive buttons crammed into the folders card (Medium, S)
- What: "Clear last transcript buffer" and "Clear transcript history" sit at the bottom of the folders accordion, each wrapping to two lines.
- Fix: a separate "Delete data" section at the end of Your data with the clear actions, each with its confirmation.

#### P4. Audio clips are kept by default without saying so in first run (High, S)
- What: "Save raw audio clips" is on by default and appears in two places; nothing in first run mentions that recordings are kept for 30 days.
- Where: `app/src-tauri/src/settings.rs:675`; `app/src/views/DataPrivacy.tsx:193`; `app/src/views/Audio.tsx:405`.
- Why: Lane 2 owns whether the default should change; for UX, the user should at least be told once and be able to choose.
- Fix: keep one control (in Your data), and add a first-run choice "Keep audio so you can replay dictations (30 days)".

### Sync

#### Y1. Status tiles are cramped and misaligned (Low, S)
- What: three narrow tiles where "LAST BACKUP ATTEMPT" wraps to three lines and the values sit at different heights.
- Where: `05-sync--full.png`.
- Fix: a two-line status: "Backed up 5 notes automatically at 11:31 PM. Notes only." with a "Back up now" button.

#### Y2. Repository field and account row are too narrow (Low, S)
- What: the repo shows "n8watkins/scribe-"; the account row wraps a one-line sentence to four lines next to a pill and a button.
- Fix: full-width rows; the repo as a link that opens it on GitHub.

#### Y3. Copy that promises or confuses (Medium, S)
- What: "More backup & export destinations are coming. For now, everything syncs to GitHub." is a promise in product UI and contradicts the "Notes only" default; "installed for Scribe Local Backup" uses the GitHub App's internal name; "This build isn't configured for GitHub sync." is developer-facing.
- Where: `app/src/views/Sync.tsx:578`, `303`; `state-github-unconfigured.png`.
- Fix: remove the promise; explain the GitHub App in one sentence; for unconfigured builds, hide the card.

### Stats

#### ST1. A page for seven numbers (Low, M)
- What: one card, two thirds of the page empty, model id truncated, no trend.
- Where: `06-stats.png`, `state-empty-stats.png`.
- Fix: fold today's numbers into Home; if Stats stays, add a 30-day words chart and time saved ("about 2.4 hours of typing saved"), which is the stat competitors lead with.

### Hotkeys (rebinding walkthrough)

Walking through rebinding Toggle Dictation:

1. Click the key box; it turns into "Press keys..." with "Esc to cancel" (`09-hotkeys-capturing.png`). Clear.
2. Press a combination; it is saved at once with no confirmation; on a conflict the row turns red with "Not registered" after the fact.
3. There is no way to clear a binding (to turn an action off), and no per-row reset.
4. The Windows key cannot be captured reliably in a WebView (Windows opens Start on release), yet the default hold key is Ctrl + Win.
5. The trigger edge segments ("On press", "On release") wrap "On release" onto two lines, a different height from "On press".

#### K1. The default toggle key swallows the backquote system-wide (High, S)
- What: Toggle dictation is bound to the bare backquote key, and the registration exists "solely so the OS suppresses the keystroke"; while Scribe runs, nobody can type a backtick in Markdown, code, Slack or a terminal.
- Where: `app/src-tauri/src/hotkeys.rs:293-296`; default at `app/src-tauri/src/settings.rs:579`; README line 14.
- Why: Scribe's audience includes developers, for whom the backtick is a daily key; on UK, German and Nordic layouts that key is something else entirely, so "tilde" is wrong there too.
- Fix: pick a default that is not a typing key (for example a double-tap of Right Ctrl, or Ctrl + Win tap versus hold), or at least let the key pass through when it was tapped without the note chord; show the key's label for the current layout instead of "~ (tilde)".

#### K2. No unbind and no per-row reset (Medium, S)
- Fix: a small "x" to clear (action off) and "Reset" per row; confirm before replacing a binding that another action uses.

#### K3. Rows are tall and labels wrap (Low, S)
- What: the list fits exactly at 960x680 and scrolls at the minimum size; "On release" wraps.
- Where: `09-hotkeys.png`, `size-min-820x540-hotkeys.png`.
- Fix: backlog J1 and J2 still apply; use "Press" and "Release" as segment labels, and a fixed-width key box.

#### K4. "Refresh" and "Global Windows shortcuts" are noise (Low, S)
- Fix: refresh automatically on focus; drop the caption.

### Models (model choice walkthrough)

1. Open Models: the page shows two summary tiles and a collapsed "Browse models" accordion; the list is hidden (`10-models.png`).
2. Expand it: 11 rows with names, sizes and file names (`ggml-small.en-q5_1.bin`), a radio and a Download button each, inside a card that scrolls on its own (`10-models-browse.png`).
3. Choose: nothing says which is recommended, how fast or accurate each is, or whether your GPU makes the large ones practical.
4. First run: the default model is marked "Active" next to its own Download button (`state-first-run-models.png`).

#### M1. No guidance for choosing (High, M)
- Fix: lead with three choices: "Recommended: Small English (181 MB), fast and accurate", "Best accuracy: Large v3 Turbo (547 MB), needs a good GPU", "Many languages: Small multilingual"; put the full list under "All models" with speed and accuracy bars, as SuperWhisper does.
  Use the GPU probe the app already has to say "Your Radeon RX 7800 XT can run Large comfortably".

#### M2. "Active" on a model that is not downloaded (Medium, S)
- Where: `state-first-run-models.png`.
- Fix: "Selected, not downloaded yet" and make Download the primary button on that row; or start the download from first run.

#### M3. Jargon on the page (Low, S)
- What: "SCRIBE_MODEL_DIR", ".bin" file names, "Q5_1", "MiB".
- Where: `app/src/views/Models.tsx:291`.
- Fix: move the environment variable to an Advanced note, show "MB", and explain quantized as "smaller, almost as accurate".

#### M4. Collapsed by default and nested scrolling (Low, S)
- Fix: show the list by default; let the page scroll, not the card.

### Audio

#### A1. 2.75 screens of settings in a narrow column (High, M)
- What: twelve settings in the right column, each description wrapping to six to eight lines; the left column holds one small card and empty space.
- Where: `11-audio--full.png`.
- Fix: put the mic picker and test across the top, then group: "Recording" (auto-stop, length limits), "Live transcription" (split on pause, phrase length) under an "Advanced" disclosure; move GPU to Models.

#### A2. Raw millisecond fields (Medium, S)
- What: 60000, 3000, 25000, 300, 600000 typed in milliseconds with a "≈ 1 min" helper.
- Fix: seconds with a slider or presets ("Short pause 1 s", "Normal 3 s", "Long 5 s").

#### A3. Duplicate "Save raw audio" (Medium, S)
- See P4; keep it once.

#### A4. Device health repeats the mic list (Low, S)
- Fix: merge into the mic picker (show availability in the dropdown).

#### A5. Record test while dictating (Low, S)
- Backlog L4 still applies.

### Themes and appearance

#### TH1. The pill color pointer is wrong (Medium, S)
- What: "The floating pill keeps its own colors (set those in Audio → Pill)", but they are in Settings → App & output.
- Where: `12-themes.png`; `app/src/views/Themes.tsx:98`.
- Fix: move the pill colors here, under the theme cards.

#### TH2. Light theme contrast holes (High, S)
- What: on Pewter, the transcript metadata chips are dark gray boxes with gray text, and disabled primary buttons are pale blue with white text.
- Where: `theme-pewter-dashboard.png`; `.metadata-row span` at `app/src/App.css:948-953` hard-codes `rgba(15, 23, 42, 0.56)`; two more hard-coded dark backgrounds at `app/src/App.css:585` and `1735`.
- Fix: replace the three hard-coded colors with theme variables; then audit Pewter view by view.

#### TH3. Custom themes can produce unreadable buttons (Medium, S)
- What: with an amber accent the primary button text is white on amber at 2.05:1 (axe).
- Where: `theme-custom-dashboard.png`; `app/src/lib/customTheme.ts`.
- Fix: pick black or white for `accent-on` by contrast, and warn in the editor when text on surface falls under 4.5:1.

#### TH4. Pewter's key is "daylight" (Low, S)
- What: the theme shown as "Pewter" is stored as `daylight` (`app/src/views/Themes.tsx:49`); harmless to users but it caused a wrong capture during this assessment and will trip future contributors.
- Fix: a comment at the definition is enough; renaming needs a settings migration.

#### TH5. Backlog M2 to M5 (edit a preset into a custom, several customs) are still open; they are nice to have, below everything above.

### About and Setup

#### AB1. The Setup checklist is buried and static (High, M)
- What: About → Setup lists three steps with jump buttons, but shows no status (done, missing, failing).
- Where: `13-about-setup.png`.
- Fix: this becomes first run (see FR below) and a checklist on Home until all three are done; backlog N1 to N3 still apply.

#### AB2. Privacy statement overclaims (High, S)
- What: "Once a model is downloaded, dictation works offline and nothing is sent to any server."
- Where: `13-about.png`; `app/src/views/About.tsx:315`.
- Fix: "Dictation is transcribed on this computer and works offline. Scribe only goes online to check for updates, and to reach GitHub or an AI server if you turn those on."

#### AB3. "What Scribe does" omits half the features (Low, S)
- What: no Transform selection, file transcription, languages, or GPU; "summarise" is British spelling where the rest of the app is American.
- Where: `13-about-what-scribe-does.png`; `app/src/views/About.tsx:55`.

#### AB4. No release notes in the app (Medium, S)
- What: About shows the version and a "View releases" link; there are no notes for what just updated.
- Fix: after an auto-update, show "What's new in 0.9" once; Lane 5 covers where the notes come from.

### Developer

#### DV1. One card (Low, S)
- What: the panel only shows window sizes; the outer size reads "Unavailable" outside Tauri.
- Where: `14-developer.png`.
- Fix: fine as an opt-in panel; add the useful diagnostics (whisper server status, last error, log tail, copy diagnostics for a bug report) or fold the window readout into General.

### Floating pill

#### PL1. Three modes, one useful at a glance (Low, S)
- What: dot, waveform, and waveform with text; the dot mode is a single 20 px circle in a 230 px capsule.
- Where: `pill-dot-recording.png`.
- Fix: shrink the window to the dot in dot mode.

#### PL2. Pill colors and mode belong in Appearance (Low, S)
- See TH1.

The pill's hidden-when-idle behavior and its text growth could not be judged in a browser; the real-app pass should capture all three modes while recording, a note, and the "Transcribed" confirmation.

### First run (time to first dictation)

From the code, a new user's path is:

1. Install; SmartScreen warns because the installer is not Authenticode-signed (Lane 2 and the handoff cover this).
2. Scribe opens on the Dashboard with a banner "Download a model to start dictating" and, below it, "Ready for dictation" (D1).
3. "Get a model" opens Models with the list expanded; the default model is the fourth of eleven rows, marked "Active" with a Download button (M2).
4. Download 181 MB (seconds to a couple of minutes).
5. Nothing prompts a mic test or a first dictation; the user must learn Ctrl + Win from the Hotkeys card.
6. The first dictation pastes into whatever app has focus.

So it is four clicks and one download, roughly two to three minutes, but with three contradictory signals on the way and no confirmation that it worked.
What can go wrong with no guidance: no mic or the wrong default mic, Ctrl + Win doing something else on that machine, the backquote key now missing (K1), an app that blocks paste (S2), or a multilingual speaker on an English-only default.

#### FR1. A three-step first run (High, M)
- Step 1: "Choose a model" with the recommended one preselected and downloading in the background as soon as the window opens.
- Step 2: "Check your microphone" with the live level meter and device picker.
- Step 3: "Try it" with a practice text box: "Hold Ctrl + Win, say 'Hello Scribe', and let go", success when text appears.
- Then: start with Windows (default yes), keep audio clips (explained), done.
- Wispr Flow and SuperWhisper both end onboarding with a practice field; it removes doubt and teaches the hotkey in one step.

## Window size recommendation

The default window is 960x680 (content 770x648 after the 190 px sidebar and the 32 px title bar), resizable down to 820x540.
The window size is not remembered unless the user finds "Save current size as default" in Data & Privacy.

Recommendation, in order:

1. **Fix density first.**
   After the IA split, the Audio, Settings and Sync pages each fit in one screen at 960x680; that alone removes almost all the scrolling the owner noticed.
2. **Remember size and position automatically** (Tauri's window-state plugin) and remove the manual "Save current size as default" button and its card.
3. **Raise the default to about 1080x740 when the screen allows it**, capped at 85 percent of the work area so it still fits a 1366x768 laptop (work area about 1366x720).
4. **Use the width when it is there:** at 1200 px and above, show settings pages as two columns of cards, and show History's selected item in a right-hand reading pane.
5. **Keep the 820x540 minimum,** but make the status grid two columns and the Dashboard single-column at that size (it already reflows, see `size-min-820x540-dashboard.png`).

A wider fixed window is not the answer on its own: the large-window capture still scrolls Settings 1.74 screens because the content is one long list.

## Consistency and wording

- **Em dashes:** 32 user-visible strings use the em dash character, U+2014 (for example `app/src/views/Settings.tsx:96`, `114`, `179`, `193`, `409`, `426`, `489`; `app/src/views/Audio.tsx:323`, `435`, `455`, `457`; `app/src/views/Hotkeys.tsx:36`, `48`, `51`, `387`; `app/src/views/About.tsx:40`, `60`, `77`, `304`, `353`; `app/src/views/History.tsx:573`; `app/src/components/UpdateOverlay.tsx:63`).
  `app/src/lib/format.ts:242` and `261` use it as an empty value placeholder, which is fine to keep as a symbol but reads better as "None".
  Fix: a lint rule (ESLint `no-restricted-syntax` on string literals containing U+2014) so they do not come back.
- **Names for the same thing differ:** "Last Transcript Buffer", "buffer", "Paste Last", "Paste last transcript", "Insert the Last Transcript Buffer"; "Toggle dictation", "hands-free", "toggle mode"; "~", "tilde", "Backquote"; "Local LLM", "local model", "LM Studio"; "Sync", "backup", "Back up".
  Fix: a short glossary (Last dictation, Hands-free, Local AI, Backup) and one pass over the strings.
- **Jargon in user-facing copy:** "Whisper's translate task", "primes Whisper", "Deterministic", "pause-bracketed", "warm transcription server", "SCRIBE_MODEL_DIR", "os error 10061", "Vulkan", "OpenAI-compatible API", "webview content size".
  Keep the technical terms in Advanced sections and the Developer panel.
- **Headings repeat:** the page title "Notes" and the list title "Notes"; "History" and "Transcript archive" for the same list.
- **Icon-only buttons** are the norm in transcript rows and the Last transcript card; they have accessible names (good) but no visible labels, so sighted users rely on hover tooltips.
- **Spelling:** "summarise" (`app/src/views/About.tsx:55`) versus "Summarize" elsewhere.
- **Spacing:** Hotkeys rows are about 75 px tall against about 55 px for settings rows, and Sync, Data & Privacy and Audio each lay out their rows in narrower two-column cards; one row component with one density would make pages read as one app.

## Accessibility

axe-core 4.14, Midnight and Pewter, every view:

- **6 unlabeled selects (critical):** Settings → Language and Pill display mode, Data & Privacy → both retention selects, Sync → Format and What to export.
  `SettingRow` renders the label as a `<strong>` with no association (`app/src/components/layout.tsx:132-141`).
  Fix: give `SettingRow` an `id` and pass `aria-labelledby` (and `aria-describedby` for the description) to its control.
- **Heading order (moderate):** Settings jumps from the page `h1` to `h3` subheads (`app/src/views/Settings.tsx:94`).
- **Region (moderate):** the custom title bar sits outside any landmark; give it `role="banner"` or put it in `<header>`.
- **Contrast is largely unmeasured:** axe could not compute contrast for most elements because of gradient backgrounds, so its "no contrast violations" result is not evidence.
  Measured or visible failures: custom amber primary button 2.05:1; Pewter metadata chips and disabled primary buttons (TH2).
  Fix: a contrast test that composites the gradient to its end color per theme (the Pewter block in `app/src/App.css` already documents WCAG ratios for semantic colors; extend that to text on cards, chips and buttons).

From the code and captures:

- **Focus is visible** on sidebar items and buttons (`a11y-focus-tab3.png`), and icon buttons have their own focus ring; inputs rely on a border change.
  Good.
- **Toggles** are buttons with `aria-pressed`; `role="switch"` with `aria-checked` is the better fit and reads as "on/off" in screen readers (`app/src/components/primitives.tsx`).
- **Tabs** in Settings have proper `role="tab"` and `aria-selected`, but arrow keys do not move between them; add the standard roving tab index.
- **Toasts** use `role="status"` or `alert` (good) and errors already stay longer than notices, but every toast still disappears on a timer; errors should stay until dismissed.
- **Hotkey capture** cannot record the Windows key reliably and gives no spoken feedback beyond the button label; offer a dropdown fallback for modifiers.
- **Motion:** no `prefers-reduced-motion` handling anywhere in `app/src`; the pill waveform, pulse and glows should calm down when Windows "Animation effects" is off.
- **Text size:** the base size is 13 px and metadata is 11 px; there is no zoom or text-size setting, and Ctrl + plus is not wired.
  At minimum make Ctrl + plus and minus zoom the webview.
- **Native confirm** for delete (H2) is announced inconsistently by screen readers compared with the app's modal.

## Earlier UX documents: what is done, still valid, or obsolete

`docs/UX_REFINEMENT_BACKLOG.md` (2026-06-21):

| Item | Status today |
|---|---|
| A Integrations | Still valid, adjusted (see IA); not started |
| B Managed local LLM | Still open; fits under Local AI later |
| C Remove dictation cleanup | Done (v0.6.0) |
| D GPU | Done; D3 "Test GPU speed" still open, low value |
| E Transform selection expansion | Still valid, and more urgent because of D7 and D8 |
| F Language gating | Still valid (S4) |
| G1 Pill not visible | Could not verify in a browser; check in the real app |
| G2 Launch at startup on | Still valid, not done (S6) |
| H Local data redesign | Partly done (copy buttons, accordion); H1, H4, H5, H8 not done and the header is broken (P1, P2) |
| I Window options to Developer | Superseded: remember size automatically and drop the setting |
| J Hotkeys density | Still valid (K3) |
| K Models view | Still valid, extended (M1 to M4) |
| L Audio tabs | Still valid in spirit; the IA split does more than tabs would |
| M Themes | M1 done; M2 to M5 open, low priority |
| N Setup view | Still valid, and should become first run (FR1) |
| O Transcribe | O2 partly done (tabs exist); O3 and O4 still open (T1) |
| P History and Notes deferred | Should no longer be deferred: H1, H2 and H8 are daily friction |
| Q Silence returns empty | Done |

`docs/FRONTEND_UX_REFINEMENT_PLAN.md` (2026-06-10) is marked archived and mostly delivered: setting rows, toggles, styled selects and visible transcript actions all exist.
Its principle "Make Dashboard an overview, not a settings surface" is still not met (the Dashboard carries a Transform editor and a hotkey list).

`docs/COMPETITIVE-ANALYSIS.md` is out of date in its matrix: multilingual and translate are now shipped, GPU is shipped, AI cleanup was removed, and sync is GitHub, not Drive.
Its priority list still holds for items 1 (code signing), 3 (onboarding), 4 (custom model files), 5 (spoken punctuation and voice commands) and 6 (streaming insertion); 2 and 7 are done.

## Compared with the category

From my knowledge as of mid-2026; details of competitors may have changed since.

- **Wispr Flow:** onboarding ends with a practice field, the dictionary learns from your corrections, settings are a small modal with a handful of sections, and a "Flow bar" sits at the bottom of the screen.
  Lessons for Scribe: FR1, H8, and the IA consolidation.
- **SuperWhisper:** model picker with speed and accuracy indicators, and per-app "modes".
  Lesson: M1; modes are a bigger feature, not needed for this pass.
- **MacWhisper:** file transcription is drag-and-drop on the main window with batch support.
  Lesson: T1 and T2.
- **Windows voice typing (Win + H):** zero setup, spoken punctuation ("comma", "new line"), and an auto-punctuation toggle.
  Lesson: spoken punctuation is a baseline expectation; R1's multi-line replacements are a cheap first step toward "new line" and "new paragraph".

Where Scribe already leads: local and free, history with search and retention, notes with AI summaries, selected-text transform, GitHub backup, and real attention to clipboard preservation.
The gap is mostly polish and guidance, not features.

## Top 10 UX changes, ranked by user value

1. **Guided first run with honest readiness** (FR1, D1, D2, M2, AB1): recommended model downloading immediately, mic check, a practice dictation, and a Home status that never says "Ready" when it is not. M.
2. **Regroup the navigation around jobs, with a settings search** (IA section, S1, A1, TH1, P4 duplicates): ends the scrolling and "where is it", and removes the two duplicated settings. L.
3. **Stop swallowing the backquote key** (K1): a non-typing default for hands-free, or pass the key through when tapped alone, with layout-aware labels. S.
4. **One clear output choice including "Type it out"** (S2, S3): a segmented "Insert, Copy, Save" plus an insert method that exposes `direct_insert`. S.
5. **Replacements v2** (R1, H8): test box, multi-line output for "new line", add from History, enable per row, import and export. M.
6. **Fix and gate Transform selection** (D7, D8, N1): restore focus to the source app (or replace the panel with a floating instruction box), and bind the hotkey only when Local AI is connected. M.
7. **Model chooser with guidance** (M1, M3, M4, S4): three recommended choices with speed and accuracy, GPU-aware advice, language and translate next to the model. M.
8. **History rows that are quick and safe** (H1 to H7): two labelled actions plus a menu, delete with Undo, day grouping, rendered summaries, no clipped pager. S to M.
9. **Drag-and-drop file transcription without PATH setup** (T1, T2): picker, drop zone, progress, bundled or one-click ffmpeg. M.
10. **Truthful privacy copy, a no-em-dash and glossary pass, and the contrast and label fixes** (N3, AB2, wording, TH2, TH3, accessibility): small edits that remove the most visible rough edges and the two places where the app overpromises. S.
