# Changelog

Versions bump with each meaningful increment of progress — patch for small
changes, minor for feature sets / phases — even when the work is still in flight
and not yet perfect.

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
