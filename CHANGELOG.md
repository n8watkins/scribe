# Changelog

Versions bump with each meaningful increment of progress — patch for small
changes, minor for feature sets / phases — even when the work is still in flight
and not yet perfect.

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
