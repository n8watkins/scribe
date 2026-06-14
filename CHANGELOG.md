# Changelog

Versions bump with each meaningful increment of progress — patch for small
changes, minor for feature sets / phases — even when the work is still in flight
and not yet perfect.

## 0.5.18 — 2026-06-14

- **Simplified the update UI.** The topbar indicator is now a single
  **"Update v0.5.x"** button (dropped the View/dismiss chip). **About → Updates**
  is the one place to act: **Install v0.5.x**, **Check for updates**, and **View
  releases** — the last always available (every release's notes = your
  changelog). Status now reads "You're on an old version — v0.5.x is available"
  (the installed version is shown just above, so the "you have vX" is gone).
- **"Automatically check for updates" toggle** in About (on by default). When
  off, background polling stops but manual "Check for updates" still works.
- Removed the per-version dismiss (it was over-complicated).

## 0.5.17 — 2026-06-14

- **Polling back to 1 minute** while we're actively iterating on the updater, so
  detection is immediate during testing. (Production target is ~6 hours — the
  on-launch and on-focus checks already make a release feel instant.)
- **Dismiss from About, too.** The Updates section now has a **Dismiss** button
  next to Install / View, and shows a "Dismissed — you can still install it here"
  note once dismissed. Dismiss is shared with the topbar chip, so it works from
  either place.

## 0.5.16 — 2026-06-14

- **Update polling dialed back to every 5 minutes** (the 0.5.15 1-minute test
  confirmed polling works) — new releases still surface promptly via the
  on-launch and on-focus checks, without hitting GitHub's rate limit. About's
  live "last checked" line stays.
- **Topbar update chip shows the version** — now reads "Update v0.5.x" (download
  icon) instead of "Update available", so you can tell exactly which version is
  waiting, even if you're several behind.
- **About always reflects an available update.** It shows the
  background-detected update — "You're on an old version — vX is available" —
  without clicking Check, and keeps showing the Install/View options even after
  you dismiss the topbar chip, so installing later is always one stop away.
- **Larger description text** across settings and views for readability.

## 0.5.15 — 2026-06-14

- **Update polling you can watch (test cadence).** The app now polls for new
  releases **every 60 seconds** (was 30 min), and About shows a live
  "Auto-checks every minute · last checked HH:MM:SS" line so you can confirm the
  polling is actually happening. Note: GitHub's unauthenticated API caps at ~60
  checks/hour, so at this rate checks start failing after about an hour — this is
  a temporary **test** cadence we'll dial back.
- **Dismissible update indicator.** The top-right "Update available" chip now has
  a **View** action (opens the release notes) and a **dismiss ✕** — you can
  ignore an update and still install it later from About. A dismissal sticks for
  that version (persists across restarts) until a newer one ships.
- **FAQ:** added entries on the taskbar/Task-Manager **icon cache** and **how
  updates work** (install / dismiss / view).

## 0.5.14 — 2026-06-14

- **Custom window title bar.** The main window now runs without native Windows
  chrome and draws its own slim bar (drag region + minimize / maximize / close),
  so there's no second "Scribe" label in the top-left.
- **Note on the taskbar / Task Manager icon:** it's already the correct Scribe
  logo in the build — a stale icon there is Windows' **icon cache** after
  updating in place, which a clean install (or a reboot) refreshes.

## 0.5.13 — 2026-06-14

- **Update notifications you won't miss.** Scribe now re-checks for a new release
  ~5s after launch, **every 30 minutes** (was 2 hours), **and whenever you
  refocus the window** — and the first time it sees a new version it fires a real
  **OS notification** (even while minimized to the tray), not just the in-app
  topbar button. (A release can only be notified about once CI has finished
  publishing it, so a brand-new version shows up shortly after, not instantly.)
  If the in-app installer ever fails to apply, grab the installer from the latest
  GitHub release directly.
- **Added FAQ.md** — starting with "Can I install custom Whisper models?" (not
  yet — the Models list is a fixed catalog) and "Why didn't I get an update
  notification?".

## 0.5.12 — 2026-06-14

- **AI dictation cleanup.** Optional local-LLM polish of each dictation before
  it's saved/pasted — strips filler ("um/uh/like"), fixes punctuation & casing,
  light formatting — with modes (Standard / Email / Chat / Code / Custom). Off by
  default; **non-blocking with a raw-text fallback**, so a slow or offline LLM
  never stalls or blanks your dictation. Settings → App & output.
- **Selected-text transform — speak (or type) an instruction to rewrite text in
  place.** Highlight text in any app, tap the Transform Selection hotkey
  (Ctrl+Alt+R), say "make this concise" / "translate to Spanish" / "fix grammar"
  (or tap again / pause to finish), and Scribe rewrites the selection where it
  sits. A typed-instruction panel on the Dashboard is the alternative. Uses the
  same local LLM as Notes.
- **Sync: back up everything + export.** "Back up all transcripts" to Google
  Drive is now real (a dated "Scribe Transcripts" folder, separate from notes),
  and you can **export** transcripts to Markdown / CSV / JSON from the Sync view
  (no account needed).
- **Separate auto-pruning for notes.** Notes now have their own retention
  window, independent of transcripts — defaults to **Forever** (notes are
  deliberate saves; you opt in). Data & Privacy → "History & retention" shows the
  transcript and note retentions side by side.

## 0.5.11 — 2026-06-14

- **Fixed: clearing transcript history (and the retention auto-delete) was also
  wiping your notes**, despite the dialog promising notes are safe. Notes are
  now protected: "Clear transcript history" and the automatic retention sweep
  only delete dictation transcripts (+ their audio clips); notes are never
  auto-pruned. The Notes view's "Clear all" now clears notes specifically (new
  `clear_notes`), and the History/Notes "Clear all" uses the on-brand
  confirmation dialog instead of the native popup.

## 0.5.10 — 2026-06-14

- **Real app icon everywhere.** Regenerated all icons from the Scribe mic logo,
  so the installed app's window/taskbar icon is the logo (releases had stale
  icons), and the sidebar brand shows it (no more "S").
- **Google Drive promoted to a first-class "Sync" area.** It's now its own
  sidebar view (out of the Settings tab) with Connection, What to back up, and
  Organize & schedule sections, framed as a backup/export hub. ("Back up all
  transcripts" is marked coming-soon — only note sync is wired up in the backend
  today.)

## 0.5.9 — 2026-06-14

- **Google Drive sync works in released builds again.** Releases were shipping
  placeholder OAuth credentials (the real ones are gitignored); the release
  build now injects them from repo secrets, so Drive sync is configured.
- **In-app visualizer now matches the floating pill** — it uses the pill's exact
  rolling-history engine (each bar is a moment in time scrolling left) instead of
  a flat pulse, with a real **vertical** gradient on the bars (cyan→purple).
- **About:** "What Scribe does" is now a grid of feature cards (not a bullet
  list); removed the headings that repeated each tab's name.
- **Brand:** the sidebar shows the real Scribe icon (bigger) instead of an "S".
- **Updates** are re-checked every 2 hours (not just at launch), so a
  long-running session still surfaces a new release.
- Removed the giant native hover tooltip that popped the full transcript text on
  History/Notes items.

## 0.5.8 — 2026-06-14

Another UI wave (three parallel agents) + perf fixes:

- **Settings no longer re-render the whole app on every toggle.** Saving a
  setting was triggering a full dashboard refetch that re-rendered every view
  and jumped the scroll; it now updates optimistically in place.
- **Visualizer** reworked the way you asked: a symmetric **center→edge** color
  (cyan center, purple edges) with a vertical gradient per bar, smaller, and as
  reactive as the floating pill.
- **History/transcripts:** "See more / See less" now sit in a fixed spot on the
  meta line (no more overlaying the text); 3 lines collapsed, 6 + scroll
  expanded; date on top. Select-all/deselect-all is always visible; "Transcript
  archive" with its icon on the left; **"Clear all" is red**; copy/save icons;
  copying deselects. Fixed the copy scroll-jump and the first-row tooltip
  hiding behind the list.
- **About** is now tabbed (App details / What Scribe does / Setup) with redirect
  buttons; the "Local-first" pill is gone.
- **Settings:** removed the duplicate first-line headings and the "App behavior"
  tab (folded into "App & output"); dropped the experimental type-it-out insert.
- **Data & Privacy:** folder paths with small open + copy-path buttons in an
  accordion.
- **Audio:** "Ready" badge moved to the right of "Input".
- **Models:** Refresh + Open-models-folder moved to the left toolbar.
- **Bigger app brand** (icon + "Scribe") in the sidebar.

## 0.5.7 — 2026-06-14

- **Fixes the "Error opening file for writing … ggml-base.dll" update failure.**
  A whisper-server child process orphaned by a non-graceful exit could keep the
  bundled whisper DLLs locked, so the next install/update failed. The NSIS
  installer now kills any leftover `whisper-server.exe` before writing files
  (pre-install hook), so updates no longer get stuck on a locked file.
- **About:** "Install update" and "View release" are now a matched pair — both
  have icons, the same footprint, and more breathing room between them.

## 0.5.6 — 2026-06-14

UI feedback wave (built by three parallel agents in isolated worktrees):

- **Hotkeys** redesigned: click the bind itself to rebind (no more "Rebind"
  button or "Registered" pill); status now shows *only* failures with the
  reason; bigger, more readable bind + description text; Press/Release sits
  beside the bind; de-duplicated heading; Reset/Refresh moved to a quiet toolbar.
- **Visualizer** (Transcribe/Audio): wider, snappier (asymmetric attack/decay),
  and a multi-color cyan→blue→purple gradient across the bars.
- **History**: inline "…see more" at the end of the snippet, the whole collapsed
  transcript is clickable to expand, and "See less" now scrolls back to the top
  (bug fix); action buttons in two rows; a select-all / deselect-all control.
- **About**: full-width detail rows (no more cramped two-column splits), the
  real on-disk data path with an Open-folder button, a clearer privacy line, a
  feature list, and a Setup checklist.
- **Clear transcript history** now uses an on-brand confirmation dialog instead
  of the native browser popup.

## 0.5.5 — 2026-06-14

- **Audio:** the "Default input device" option (and the input label) now shows
  the *resolved* default device in parens — e.g. "Default input device (FIFINE
  Microphone)" — with the Windows "Microphone (…)" wrapper stripped.
- **Configurable pill background:** a new "Pill background" color in Settings
  controls the floating pill's background (applied with slight translucency over
  the desktop); defaults to the on-brand dark blue.

## 0.5.4 — 2026-06-13

UI declutter / redesign pass (run partly via parallel agents) plus a few new
controls:

- **Chrome declutter:** every page now shows just its name (the long per-page
  subtitles are gone); the "Scribe" wordmark is on-brand blue; removed the
  "Offline ready" sidebar panel; section-panel header icons sit left of titles.
- **Dashboard tiles** are horizontal now — bigger icon, label + value, and a
  redirect arrow; the status tile shows a single status (no duplicate pill); the
  mic name is stripped of the Windows "Microphone (…)" wrapper.
- **Reactive visualizer:** the Transcribe/Audio bars now move with your mic
  level (same `audio://level` stream the pill uses) instead of being static.
- **Settings inputs:** durations are typed millisecond fields with a live
  "≈ Ns" read-out and no spinners; narrower control column; Models "In use" →
  "Active" and its folder button sits next to Refresh.
- **Transcribe** split into "Record" / "Transcribe a file" tabs; the last-buffer
  and file-result areas are height-capped and scroll inside.
- **History/Notes:** entry actions in a 2×2 grid; "see more" sits under the
  snippet; expanded items cap-and-scroll; Notes icon moved left of its heading.
- **New "Discard / Cancel" hotkey** (default `Ctrl+Alt+X`) that cancels the
  current recording.
- **Window:** default size is now `960×680`, with a "Save current size as
  default" button (Data & Privacy) that persists and restores on launch.
- Data & Privacy shows the on-disk data folder path. (Assigning a *custom* data
  folder is groundwork-only for now — the picker is held back until the
  database/models/clips can be safely relocated and Windows-tested.)

## 0.5.3 — 2026-06-13

- **Per-bind press/release trigger.** Toggle Dictation, Paste Last Transcript,
  and Open Dashboard each get an "On press / On release" switch in Hotkeys
  settings — pick which key edge fires the action. Hold-to-Talk is unchanged
  (it is push-to-talk and inherently uses both edges). Defaults preserve today's
  behavior: Toggle on release, Paste and Open Dashboard on press, so existing
  installs see no change until they flip a switch.
- Choosing **press** for Toggle automatically disables the hold-`~`+Q note
  chord (which needs the key held — i.e. release mode). The Toggle hint explains
  this right where you flip the edge, instead of failing silently.

## 0.5.2 — 2026-06-13

- **Paste-last fires on key-press, not on release.** The clipboard paste no
  longer blocks waiting for you to let go of the whole chord (e.g.
  `Ctrl+Alt+Shift+V`) — it synthesizes the held-modifier release itself and
  pastes immediately, so you don't have to release every key. The toggle/tilde
  stays on release by design (it doubles as the hold-`~`+Q note trigger).

## 0.5.1 — 2026-06-13

Multi-agent code-review fixes:

- **Crash fix:** `text_replace` no longer panics on non-ASCII input — the
  matcher was mixing byte offsets from a lowercased copy with the original
  string (e.g. Turkish `İ`); rewritten char-safe.
- **Terminal-safe default paste:** the clipboard-paste path now force-releases
  held modifiers before the synthetic `Ctrl+V` (same backstop the keystroke path
  had), so a held `Ctrl+Alt+V` can't re-fire into terminals; longer post-paste
  settle.
- **Date search:** local-day bounds with an exclusive upper bound (`created_at <
  to`) — fixes UTC-vs-local day shift and a sub-millisecond edge.
- **Dashboard** "Auto-insert" tile now correct for the paste-and-leave mode.
- **Models** row reverts to "not downloaded" correctly after delete (stale
  progress no longer shadows it).
- **History** search no longer double-loads (undebounced) per keystroke.
- Stable Tauri listeners across the Notifications toggle; audio meter resets on
  stop + object-URL leak fixed; History playback error handling; Settings
  deep-link tab resets on exit; removed dead exports.

## 0.5.0 — 2026-06-13

- **Audio view reorganized** into Input / Audio processing / Device health
  sections (compact, no scrolling at the default window).
- **Settings deep-links** — a gear button in Notes and History jumps straight to
  the relevant Settings tab (`openSettings(tab)`); the "Notes analysis" tab is
  now just **Notes**.
- Marks completion of the full UX/architecture rework plan — all 8 workstreams
  (insertion, archive, models, pill, LM-Studio status, dictionary, audio,
  dashboard/refactor).

## 0.4.8 — 2026-06-13

- **Dictionary redesign.** The old "Custom vocabulary" tab is now **Dictionary**,
  with two clearly-separated layers: a **Context hint** (the Whisper priming
  prompt — improves recognition, not find-and-replace) and **Replacements**, a
  deterministic word-boundary, case-insensitive "say X → get Y" table applied to
  every transcript after recognition (e.g. "my email" → your address, fix
  "clawed" → "Claude").

## 0.4.7 — 2026-06-13

- **Local-LLM status.** The Notes-analysis settings tab gets a connection card
  with a **Test connection** button — shows Connected + the server's available
  models, or "Not running" with setup guidance, for the LM Studio / Ollama
  endpoint that powers notes analysis.

## 0.4.6 — 2026-06-13

- **Pill polish.** The floating pill is now a true rounded pill and a bit
  narrower; the waveform **tapers at the edges** (outer bars quieter than the
  center); and the **bar/dot colors are configurable** — normal + note-session
  color pickers in Settings (defaults amber / cyan).

## 0.4.5 — 2026-06-13

- **Models view rework.** Summary header (active model + storage used + open
  folder), a "Browse models" accordion that scrolls internally, and **select
  (downloaded-only radio) cleanly separated from download state** — no more
  duplicated "Selected".
- **Cleaner archive rows.** Dropped the duplicated bold first-line "title" — a
  History/Notes row is now just the transcript text (regular weight) with the
  timestamp/meta underneath, reclaiming the line for actual content.

## 0.4.4 — 2026-06-13

- **Phase 2 — transcript archive.** History/Notes rows now **lead with the
  transcript text** (timestamp + meta secondary) with inline **See more / See
  less**; a per-row **Open in external editor**; **search by date range + sort**
  (newest / oldest / longest); and **multi-select → Combine** → merged preview →
  **Copy** or **Save as new entry**. Backend: `search_transcripts` gained
  `from`/`to`/`sort`, plus `combine_transcripts`, `save_combined_transcript`, and
  `open_transcript_externally` commands.

## 0.4.3 — 2026-06-13

- **"Keep my clipboard" toggle** — the second insertion switch. Together,
  **Auto-insert** × **Keep my clipboard** select the behavior: paste &
  restore / buffer-only / paste & leave-on-clipboard / copy-only. Completes the
  insertion/paste design (auto-insert toggle + clipboard toggle + rebindable
  Paste-last hotkey + full-fidelity restore + Dev keystroke fallback).

## 0.4.2 — 2026-06-13

- **Simplified insertion controls.** Output behavior is now a single
  **"Auto-insert after dictation"** on/off toggle (On = paste when you stop
  talking; Off = save to the buffer, insert with the Paste-last hotkey),
  replacing the old output-mode + paste-method pickers. The keystroke
  "type it out" mode moved to **Developer → Experimental insert**. Clipboard
  status labels are now honest about borrow-and-restore.

## 0.4.1 — 2026-06-13

- **Full-fidelity clipboard restore.** The instant paste (auto-paste *and*
  `Ctrl+Alt+V`) now snapshots and restores the *entire* clipboard — images
  (CF_DIB/CF_DIBV5) and files (CF_HDROP), not just text — so borrowing the
  clipboard for one `Ctrl+V` leaves it exactly as it was. (Raw GDI
  bitmap/metafile handles and delayed-render formats are skipped, but images and
  files also publish a byte-copyable variant that is restored.)

## 0.4.0 — 2026-06-13

- **Insertion overhauled.** Auto-paste *and* Paste-last-transcript (`Ctrl+Alt+V`)
  now do a single **instant clipboard paste that restores your previous
  clipboard** ("Paste instantly"), instead of typing the transcript out
  keystroke-by-keystroke. Existing installs are auto-migrated (defaults v5);
  "Type it out (no clipboard)" remains as an opt-in keystroke fallback.
- **Terminal-safe paste:** held hotkey modifiers are released before the paste,
  so a held `Ctrl+Alt+V` can't scramble terminals.
- **Dev/stable coexistence (Wave 3):** the **Scribe Dev** flavor seeds
  non-conflicting hotkey binds (`Ctrl+Shift+…`) so it no longer fights stable
  Scribe for global shortcuts (the cause of the tilde leaking through to the
  focused app); a **"Load my production defaults"** button switches Dev back to
  your real binds.
- **Dashboard rework:** status tiles wrap without overflow, the real microphone
  name is shown, duplicated status removed, and a **Developer** panel with a
  live window-resolution readout (Settings → Enable developer settings).
- **Internal:** `App.tsx` split into per-view modules; UIA atomic-insert
  experiment evaluated (verdict: partial — kept the keystroke fallback).

## 0.3.0 — baseline

- Prior shipped state at the start of this work: Scribe rebrand, Google Drive
  notes sync, history/stats, model manager, rebindable hotkeys, floating pill.
