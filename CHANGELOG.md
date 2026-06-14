# Changelog

Versions bump with each meaningful increment of progress — patch for small
changes, minor for feature sets / phases — even when the work is still in flight
and not yet perfect.

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
