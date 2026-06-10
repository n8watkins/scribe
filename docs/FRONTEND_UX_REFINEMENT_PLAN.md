# LocalDictate - Frontend UX Refinement Plan

Status: Ready for implementation  
Created: 2026-06-10  
Scope: Reduce clutter, clarify settings, improve component hierarchy, and make transcript actions obvious.

## Problem Summary

The current frontend is a useful foundation, but it still reads too much like a dense dashboard mock. The main issues to address before backend integration goes deep:

- Dashboard and secondary pages feel cluttered.
- Settings are not clearly editable; too many rows read as static status.
- Page sections and subcomponents need clearer visual discrimination.
- Recent transcript rows need obvious actions like Insert, Copy, Edit, and Delete.
- Settings, Hotkeys, Models, and Audio pages feel either too spread out or too vague.
- Dropdown/select styling does not fully match the premium dark theme.
- The UI needs to make status, settings, and actions visually distinct.

The fix should be a focused UX refinement pass, not a frontend rebuild.

## Design Principles

- Keep the current app shell, navigation, and dark visual direction.
- Make Dashboard an overview, not a settings surface.
- Make each page answer one job clearly.
- Use setting rows for configuration, transcript rows for transcript content, and cards only for major grouped surfaces.
- Prefer toggles, segmented controls, and compact action buttons over plain static text rows.
- Avoid cards inside cards.
- Use dense but calm Windows utility spacing on Settings, Hotkeys, Models, and Audio.

## Page Roles

### Dashboard

Purpose: answer current-state questions quickly.

Keep:

- Current status.
- Active microphone.
- Active model.
- Output mode / clipboard status.
- Last Transcript Buffer.
- Recent transcripts preview.
- Basic stats.

Change:

- Reduce each status card to current value, small status, and one action.
- Make Last Transcript Buffer the primary visual object.
- Show only the top 3 recent transcripts.
- Add compact actions to each recent transcript: Insert and Copy.
- Move advanced configuration to dedicated pages.

### Transcribe

Purpose: manual recording and output routing.

Keep:

- Start/stop/cancel controls.
- Output behavior.
- Paste method.
- Last Transcript Buffer reference.

Change:

- Make the record state and output choice more prominent than explanatory text.
- Use segmented controls for output mode and paste method.
- Keep manual controls compact and clearly disabled until backend wiring exists.

### History

Purpose: search, inspect, and reuse transcripts.

Change:

- Make this the full transcript management surface.
- Each row should include:
  - transcript title or timestamp
  - excerpt
  - metadata
  - Insert
  - Copy
  - Edit
  - Delete
- Add search and retention filter.
- Keep rows dense and scannable.

### Settings

Purpose: app behavior, privacy, and defaults.

Change:

- Replace static status rows with real setting rows.
- Standard setting row structure:
  - label
  - short description
  - control on the right
- Use toggles for:
  - launch at startup
  - minimize to tray
  - show floating pill
  - notifications
  - sounds
  - history enabled
  - save raw audio clips
  - silence trim
- Use selects only for retention, language, and other larger option lists.

### Hotkeys

Purpose: shortcut management.

Change:

- Use one compact list of hotkey rows.
- Each row should show:
  - action
  - current keycap
  - registration status
  - Rebind button
- Add a clear conflict/error area.
- Use segmented control for recording mode.

### Models

Purpose: local Whisper model management.

Change:

- Use a table-like list with columns:
  - model
  - size
  - status
  - action
- Actions:
  - Download
  - Cancel
  - Select
  - Delete
- Show selected model as a clear state, not just a badge.
- Keep storage path and model folder actions in a secondary compact panel.

### Audio

Purpose: microphone selection and recording quality.

Change:

- Use microphone selector, input meter, and test recording as the primary content.
- Use setting rows for:
  - silence trim
  - min duration
  - max duration
  - target format
  - save raw audio
- Keep diagnostics/status in a compact side panel.

### About

Purpose: product identity and local-first promise.

Change:

- Keep minimal.
- Show version, privacy statement, local data path, and links to docs/licenses.
- Avoid large marketing-style content.

## Component Updates

### Setting Row

Create a reusable setting row pattern:

```text
Label + description                       Control
```

Controls may be:

- Toggle
- Segmented control
- Select
- Number input
- Button

### Transcript Row

Create a reusable transcript row pattern:

```text
Transcript excerpt and metadata           Insert Copy Edit Delete
```

Dashboard may use a compact version. History should use the full version.

### Select Inputs

Restyle selects to match the dark theme:

- background: `#0D1320` or transparent dark surface
- border: `rgba(148, 163, 184, 0.18)`
- text: `#F8FAFC`
- focus border: cyan
- no light native-looking field background

### Toggles

Add a custom toggle component for boolean settings.

States:

- on: cyan/green accent
- off: muted slate
- disabled: reduced opacity

### Buttons

Keep button emphasis clear:

- Primary: one main action per section.
- Secondary: normal action.
- Ghost/icon: row-level actions.
- Danger: destructive actions only.

## Implementation Steps

1. Add small reusable UI primitives in the current frontend:
   - `SettingRow`
   - `Toggle`
   - `TranscriptRow`
   - `SectionPanel`
   - `IconButton`
2. Refactor Dashboard to reduce status-card content and emphasize Last Transcript Buffer.
3. Refactor History and Recent Transcripts to include Insert/Copy/Edit/Delete actions.
4. Refactor Settings into real editable rows with toggles and segmented controls.
5. Refactor Hotkeys, Models, and Audio into compact management pages.
6. Fix select/input styling and focus states.
7. Run `npm run build`.

## Sub-Agent Assignments

### UI Agent A - Component System Cleanup

Mission:

- Create reusable UI primitives in `src/App.tsx` or split into `src/components/` if the file becomes unwieldy.
- Implement SettingRow, Toggle, TranscriptRow, SectionPanel, and IconButton.
- Ensure existing visual style is preserved but clearer.

Acceptance:

- `npm run build` passes.
- Dashboard, Settings, and History use the new primitives.

### UI Agent B - Dashboard and Transcript Flows

Mission:

- Declutter Dashboard.
- Make Last Transcript Buffer primary.
- Add Insert/Copy actions to recent transcript preview.
- Improve History rows with Insert, Copy, Edit, Delete.

Acceptance:

- Users can tell immediately where their last transcript is and how to reuse it.
- Recent and full history rows have clear action affordances.

### UI Agent C - Settings, Hotkeys, Models, Audio

Mission:

- Make Settings a real settings page with toggles and clear control rows.
- Make Hotkeys a focused keybinding editor.
- Make Models a compact model manager.
- Make Audio a focused input/test panel.

Acceptance:

- Each page has a clear primary job.
- Controls look editable, not like static status labels.
- Select/dropdown styling is on-brand.

## Done Criteria

- Frontend still represents the actual app foundation, not a disposable mock.
- No page feels like a generic dashboard dump.
- Settings are visibly configurable.
- Recent transcripts and history items support obvious reuse actions.
- `npm run build` passes.
