# Scribe — UX & Architecture Rework Plan

Status: PLANNED (owner reviewing — build nothing until approved)
Created: 2026-06-13
Author: deep-dive pass over the full codebase (frontend `App.tsx`, Rust backend, pill)
Related docs: [PRD.md](./PRD.md), [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md),
[FRONTEND_UX_REFINEMENT_PLAN.md](./FRONTEND_UX_REFINEMENT_PLAN.md), [HANDOFF.md](./HANDOFF.md)

> This plan turns one long owner review (2026-06-13) into 8 workstreams. Every raw
> complaint is mapped to a workstream in the **Coverage Map** at the bottom so
> nothing is dropped. The owner chose: **write this doc first, build nothing until
> approved.**

---

## 0. Decisions locked (from owner, 2026-06-13)

1. **Insertion must not consume the system clipboard.** No "borrow the clipboard,
   paste, then restore it" as the default — the owner finds save/restore hacky.
   The transcript lives in **its own private buffer** (this already exists as the
   *Last Transcript Buffer* in SQLite) and is injected into the focused app
   **without touching the system clipboard**.
   - A **"Clipboard paste"** mode (uses the system clipboard, leaves the transcript
     on it, **no restore**) is an acceptable **opt-in for other users**.
   - **Full-fidelity clipboard save/restore** is *not* a priority; it may be
     **explored by a subagent as an experiment** only, behind the default.
2. **Pill: make color a setting.** Color picker for normal + note modes (refined
   amber/cyan defaults), plus narrower + fully-rounded shape and tapered visualizer
   edges.
3. **Sequencing: this doc first.** No code until the owner approves.

---

## 0.1 Owner approvals & refinements (2026-06-13, post-review)

**Phase 1 approved to build:** Workstream A (insertion) + the dashboard-overflow
slice of B, plus the two enablers below.

- **Insertion default = atomic-burst keystroke injection** (clipboard-free). The
  **UI Automation experiment is APPROVED** — run in parallel (subagent) toward a true
  atomic, clipboard-free insert; if reliable on the owner's daily apps it can become
  the default. Full-fidelity clipboard restore stays parked.
- **Refactor approved (Model 2):** split `App.tsx` into per-view modules FIRST so
  frontend agents parallelize without colliding.
- **Combine output = BOTH:** produce insertable text *and* allow saving a new entry.
- **Resolution readout → Developer Settings (not dev-build-gated):** add an "Enable
  developer settings" toggle in Settings; when on, a **Developer** item appears in the
  left sidebar holding the live resolution readout (window WxH + content width) as a
  toggle. Works in any build once enabled.
- **Dev/stable hotkey coexistence (Wave 2):** the **Scribe Dev** flavor
  (`com.natkins.scribe.dev`, `is_dev_flavor` in `lib.rs:32`, separate settings store)
  must seed **different default hotkeys** so dev + stable don't fight over global binds
  when both run. Add a **"Load my production defaults"** button in Developer settings
  to switch dev to the real binds when running dev alone.
- **Dictionary redesigned — see §3.1.**

**Verification in this env:** Windows targets are installed
(`x86_64-pc-windows-msvc` / `-gnu`), so `cargo check --target x86_64-pc-windows-msvc`
**type-checks the `#[cfg(windows)]` code** (can't run here). Frontend: `tsc --noEmit`
+ `vite build`. Real runtime checks happen on the owner's Windows box.

---

## 1. Guiding principles & constraints

- **Clipboard-free insertion is the product promise**, not a nice-to-have
  (`PRD.md` core promise). The architecture already embodies the owner's mental
  model — the fix is execution + clarity, not redesign.
- **Windows insertion reality (important):** to put text into *another* app's
  focused field there are exactly two general mechanisms:
  1. **Synthetic keystrokes** (`SendInput` Unicode) — **does not touch the
     clipboard**, but is "typed" (current `DirectInsert`). Instant in normal text
     fields; can look like typing and can misbehave in terminals.
  2. **System clipboard + `Ctrl+V`** — atomic, but **uses the clipboard**.

  There is **no** way to make another app's `Ctrl+V` read from a private buffer;
  the OS clipboard is the only shared channel. So "atomic paste feel" and "never
  touch the clipboard" are in tension for arbitrary apps. See Workstream A for how
  we reconcile this.
- **The frontend is one 4,278-line file** (`app/src/App.tsx`). Parallel agents
  editing it collide badly. Frontend work is **sequenced under one owner** (or we
  split the file first — see Orchestration).
- **Don't touch the notes backend** (owner). Notes are already a first-class
  DB-backed, searchable object (same `transcripts` table, `is_note` flag).
- **Windows-only** remains the target; non-Windows paths stay stubbed.

---

## 2. What's actually true today (myth-busting from the code)

- **Insertion already fires once, at the end** (`dictation.rs:239`
  `handle_transcription_output`). Live transcription only updates the *pill*; it
  never pastes partial text. The "streaming" feel is **purely** `DirectInsert`
  typing char-by-char (`output.rs:603`).
- **There is a local database.** SQLite at `%APPDATA%\com.natkins.scribe\`.
  Transcripts **and notes** are rows with full text + metadata — **not `.txt`
  files**. (The only `.txt` writing is the unrelated file-transcribe "Save"
  feature, `save_text_file`.)
- **Search already exists** (`search_transcripts`, `db.rs:91`) — but only
  `text LIKE` + newest-first + pagination. **No date filter, no sort options yet.**
- **"Custom vocabulary" is Whisper's `--prompt` / initial_prompt** (`whisper.rs:153`).
  It's a *soft* bias toward your spellings/terms/context; in live mode it also
  appends your last ~200 chars as rolling context (`incremental.rs:187`). It helps
  homophones ("no"/"know") by priming context but cannot guarantee them.
- **LM Studio is already the notes endpoint** — default `http://127.0.0.1:1234/v1`
  (`settings.rs:139`), OpenAI-compatible (`note_analysis.rs`). There's just no
  *status/health* surface yet.
- **"Cancel" (top-right) = `cancel_recording`** — aborts + discards the in-progress
  recording (nothing transcribed/saved). Only meaningful while recording.
- **The "Clipboard Preserved" pill on the buffer card is hardcoded** (`App.tsx:3463`)
  — it shows regardless of the actual setting. Misleading.

---

## 3. Workstreams

Effort key: **S** ≈ <½ day, **M** ≈ ½–1 day, **L** ≈ 1–2 days. Layer: BE = Rust, FE = React/CSS.

### A — Insertion = clean, clipboard-free single insert  ★ top priority
**Problem:** Transcript is "typed" char-by-char (looks like streaming, corrupts
terminals). Owner wants a single atomic insert that never touches the clipboard.
**Root cause:** default `PasteMethod::DirectInsert` (`settings.rs:264`) →
`direct_insert_text` synthesizes each UTF-16 unit as a keypress (`output.rs:603`).
**Fix:** see the dedicated deep-dive in **§4**. In short: keep clipboard-free
keystroke injection as the default but make it land as **one atomic burst** and
**kill the terminal modifier collision**; reframe the UI around the private buffer;
add an opt-in **"Clipboard paste"** mode (no restore); fix the misleading pill.
**Files:** `output.rs`, `settings.rs`, `dictation.rs` (confirm single-shot),
`App.tsx` (labels, buffer card, paste-method UI), `backend.ts`.
**Acceptance:**
- Default insert touches the system clipboard **0 times** (verified by setting a
  sentinel clipboard value before dictation and asserting it's unchanged after).
- A 60-word transcript inserts as a single visible action in Notepad/VS Code/a text
  field (no per-char crawl).
- Pasting into Windows Terminal with `Ctrl+Alt+V` held does **not** trigger
  shortcuts or scramble input.
- The buffer card's clipboard pill reflects the actual mode.
**Layer/Effort:** BE+FE / **M**.

### B — Dashboard rework
**Problem:** 4 status tiles overflow at the current window size (badge + "Change"
clipped, screenshot confirms); duplicated status ("Recording" badge **and** value;
redundant "Selected" model badge); mic shows "Default in…" not the real device;
output-mode wording too long; Dashboard duplicates Transcribe; buffer shows the
whole transcript (long pages).
**Root cause:** `.status-grid { grid-template-columns: repeat(4, minmax(0,1fr)) }`
(`App.css:290`) collapses to 2 only at `max-width:880px` (`App.css:1483`) — too
late, and `minmax(0,…)` lets tiles shrink below content so text clips. Cards:
`StatusCard` usages at `App.tsx:818-856`.
**Fix:**
- Tiles wrap with `repeat(auto-fit, minmax(220px, 1fr))` so 4→2→1 happens by
  available width, never clipping; raise the 4-across threshold accordingly.
- "Current status": drop the duplicated value, keep one `StatePill`; surface the
  action only when relevant.
- "Active model": remove the "Selected" badge → just `base.en` + **Manage**.
- "Active microphone": resolve and show the **real** device name from
  `listMicrophones()` (use `isSelected`/`isDefault`); selection stays in Audio.
- "Output mode": tighten label (e.g. **Auto-paste · clipboard untouched**), no
  overflow.
- **Dev-mode live resolution readout** (window WxH + content-area width) so the
  owner can hand back exact breakpoints. Gate behind dev build / a debug toggle.
- Collapse the buffer card to a snippet + **See more** (full viewer lives in C).
- De-dupe Dashboard vs Transcribe: Dashboard = status + quick buffer; Transcribe =
  the capture/output controls. Remove overlap.
**Files:** `App.tsx` (`DashboardView` 781, `StatusCard` 3573, helpers
`microphoneDisplayName` 4126, `clipboardStatus` 3950, `outputModeLabel` 3968),
`App.css` (`.status-grid`, media queries).
**Acceptance:** at the screenshot's window width all tiles fit with no clipped text;
no duplicated status words; mic tile shows the actual device; resolution readout
visible in dev.
**Layer/Effort:** FE / **M**.

### C — Transcript viewer + archive detail
**Problem:** Can't read a full transcript in-app without opening a file; archive
rows lead with the timestamp ("Transcript from June 13…") instead of the text; no
inline expand; no "open externally"; buffer dumps the whole transcript.
**Root cause:** `transcriptTitle` (`App.tsx:4262`) = timestamp; `HistoryView`/
`TranscriptRow` (1149 / 3662); buffer renders `transcript.text` raw (`App.tsx:3473`).
**Fix:**
- **Transcript-first layout:** row title = first line/snippet of the text;
  timestamp + meta (words · chars · `4.2s` audio · `base.en`) become the secondary
  line. Applies to History rows and the buffer card.
- **Inline "See more"** that expands the row in place (row grows, full text shown)
  — multiple expandable rows, no navigation.
- **Read-only viewer window** for long transcripts (a "See full" that opens a
  larger in-app text view; not the editor). Reuse the existing window plumbing the
  pill uses.
- **Open externally**: write a temp `.txt` and open the OS default editor
  (Notepad/whatever), next to **Play recording**. (Small BE command or reuse
  `save_text_file` + an `open` shell.)
- Keep `updateTranscript` available but the default detail view is **read-only**
  (owner: "viewing, not editing").
**Files:** `App.tsx` (History, TranscriptRow, LastTranscriptCard, new viewer),
`App.css`; small BE: `open_transcript_externally` (or reuse).
**Acceptance:** clicking a row reveals the full text inline; long transcripts open
a roomy read-only window; "open externally" launches the default editor; rows read
text-first.
**Layer/Effort:** FE (+ small BE) / **L**.

### D — Archive search/sort + combine
**Problem:** Can't search/sort by time; can't combine multiple transcripts.
**Root cause:** `search_transcripts` (`db.rs:91`) has no date params and always
`ORDER BY created_at DESC`; no combine command exists.
**Fix:**
- Extend `search_transcripts` with optional `from`/`to` (date range) and a `sort`
  (newest/oldest/longest). Add an index on `created_at` if missing. (FTS5 only if
  LIKE proves slow at the owner's volume — likely unnecessary.)
- Search/filter UI: text box + date range + sort, in History (and Notes view via D
  reuse).
- **Combine:** multi-select rows → merged preview (ordered, separator configurable)
  → actions: copy to private buffer / insert / **save as a new entry**. New BE
  command `combine_transcripts(ids, separator)` returning the merged text (and
  optionally persisting a new row).
**Files:** `db.rs`, `commands.rs`, `backend.ts`, `App.tsx` (History toolbar +
multi-select), `App.css`.
**Acceptance:** searching a date range returns only that range; sort options work;
selecting N rows + Combine yields one merged transcript you can insert/save.
**Layer/Effort:** BE+FE / **M**.

### E — Models view rework
**Problem:** Shows every possible model up front; "selected" appears twice; status
conflates download-state and selection; lots of wasted right-side space; whole
container scrolls.
**Root cause:** `ModelStatus` includes both download states **and** `selected`,
**and** `ModelInfo` has a separate `selected: boolean` (`backend.ts:148-177`) — the
double-selected bug. `ModelsView` (`App.tsx:2624`).
**Fix:**
- **Separate two axes:** (1) download state = *not downloaded → downloading →
  downloaded* with a single **Download/Delete** action; (2) **selected** = a
  distinct radio/check. **Only downloaded models are selectable.** Remove
  "selected" from the status enum's UI usage (keep one source of truth).
- **Default model + storage shown first**; the full catalog lives behind a
  **"Browse models" accordion** that **scrolls internally** (fixed-height list),
  so the page doesn't grow/scroll as a whole at the default window size.
- De-emphasize "not downloaded"; reclaim the wasted horizontal space (tighter rows).
**Files:** `App.tsx` (`ModelsView`), `App.css`; possibly `model_manager.rs`/
`models.rs` labels only (no behavior change).
**Acceptance:** default view shows current model + storage without scrolling at the
default window size; the catalog scrolls within its own box; you can only select
downloaded models; "selected" shows once.
**Layer/Effort:** FE / **M**.

### F — Settings / Audio reorg, renames, cross-links
**Problem:** "Custom vocabulary" is opaque; output-mode options are cramped and
under-segmented (tiny descriptions glued to the controls); "Notes/Analysis" naming;
Audio page sprawls and scrolls on small windows; no quick path from a sub-view to
its settings.
**Fix:**
- **Rename** "Custom vocabulary" → **Dictionary** (with a plain-English explainer +
  examples + maybe 2–3 preset prompts); "Notes/Analysis" → **Notes**.
- **Output-mode option cards:** each mode (Auto-paste / Save only / Copy / Copy &
  paste) in its own encapsulated card with a roomy description, clearly separated
  from the control. Re-word using A's clipboard-free framing.
- **Audio compartmentalized** into **Input** (device pick + level/test), **Audio
  processing** (silence trim/auto-stop, incremental), **Device health**; compact
  enough to avoid scrolling on the small window.
- **Cross-links:** a **Settings** button in Notes/History/Models/Audio that
  deep-links to the relevant settings section (route param → open + scroll).
**Files:** `App.tsx` (`SettingsView` 1505, `AudioView` 2903, sub-views), `App.css`.
**Acceptance:** Dictionary reads clearly to a non-expert; output modes are visually
distinct cards; Audio fits the small window in 3 labeled sections; each sub-view has
a working "Settings" deep-link.
**Layer/Effort:** FE / **L**.

### G — LM Studio (local LLM) status & setup surface
**Problem:** No indication whether the local LLM (LM Studio/Ollama) is reachable or
which model is loaded; setup is undocumented in-app.
**Root cause:** `note_analysis.rs` calls the endpoint on demand but nothing pings it
for status.
**Fix:** new BE command `llm_status()` → `GET {endpoint}/models` (reuse
`first_listed_model` shape): returns `{ reachable, models[], endpoint }`. FE: a
**connection card** in Notes/Settings showing Connected/Not running + loaded
model(s) + a short setup guide (start LM Studio, load a model, default port 1234) +
a "Test connection" button.
**Files:** `note_analysis.rs` (or new `llm.rs`), `commands.rs`, `backend.ts`,
`App.tsx`.
**Acceptance:** with LM Studio off → "Not running" + guidance; with it on → models
listed; Test connection round-trips.
**Layer/Effort:** BE+FE / **M**.

### H — Pill + visualizer polish
**Problem:** Pill not perfectly rounded, too wide; colors not configurable; the
visualizer's outer bars are as loud as the center.
**Root cause:** `.pill-shell` radius 13px (`pill.css:33`); `LAYOUT_SIZES.bar.width
= 230` (`PillApp.tsx:61`); bar color hardcoded `#fbbf24`/note `#38bdf8`
(`pill.css:122/242`); bars uniform scale (`PillApp.tsx` Visualizer).
**Fix:**
- **Color as a setting:** add `pillColorNormal` + `pillColorNote` to `AppSettings`
  (BE + FE types), color pickers in Settings, applied via CSS custom properties on
  the pill. Refined defaults (amber / cyan).
- **Shape:** bar layout radius → full pill (999px); width 230 → ~200 (keep click
  target comfortable).
- **Tapered edges:** apply a falloff envelope so the outermost bars max out lower
  (multiply target scale by a window function across the bar index), or a CSS mask
  gradient on `.pill-bars`.
**Files:** `pill.css`, `PillApp.tsx`, `settings.rs` (+ defaults/migration),
`backend.ts`, `App.tsx` (Settings color pickers).
**Acceptance:** pill reads as a clean rounded pill; colors change live from
Settings for both modes; visualizer edges visibly quieter than the center.
**Layer/Effort:** FE (+ small BE for settings) / **M**.

---

## 3.1 Dictionary redesign (replaces "Custom vocabulary") — Workstream F

Today "custom vocabulary" = Whisper `initial_prompt` (a soft bias). The owner expected
"say X → get Y". Split into **two clearly-labelled layers:**

1. **Context hint (old behavior, renamed):** a short natural-language prompt that
   primes Whisper toward your domain/spellings/proper nouns. Soft, probabilistic, not
   find-replace. *Starter prompt to ship (editable):*
   > "Casual technical dictation by Nathan. Common terms: Scribe, Tauri, Rust,
   > TypeScript, React, whisper.cpp, Claude, Anthropic, GitHub, LM Studio, SQLite,
   > YouTube, vidIQ. Prefer these spellings; transcribe spoken punctuation."
2. **Text replacements (the real, intuitive Dictionary):** a deterministic
   post-transcription table of **spoken phrase → output text**, applied after Whisper,
   before insertion. E.g. "my email" → an address; "clawed"/"cloud code" → "Claude
   Code"; "new line" → an actual newline; "arrow function" → "=>". Case-insensitive,
   longest-match-first, word-boundary aware; optional regex later.

This directly answers the owner: the new Dictionary **is** "say X → get Y" (Layer 2);
Layer 1 is only a hint. *(Workstream F / later phase — captured now so it's not lost.)*

---

## 4. Deep-dive: the insertion / clipboard fix (Workstream A)

The owner's ask, restated: *"paste with Ctrl+Alt+V and insert the last transcript
**without consuming the clipboard at all** — a private buffer that's just ours."*

**Good news: that buffer already exists.** The *Last Transcript Buffer* is stored in
SQLite and is fully independent of the system clipboard; `Ctrl+Alt+V`
(`paste_last_transcript`) already reads from it. So "our own clipboard" is **done at
the data layer**. The only open question is the *injection mechanism*, and that's
where the typing feel / terminal bug live.

### The reconciliation (recommended default)
**Keep clipboard-free keystroke injection as the default, but fix its two real
defects:**

1. **Make it land atomically.** Today `direct_insert_text` flushes every 120 chars
   (`output.rs:616`). For typical dictations that's already one `SendInput`, but we
   should send the **entire transcript in as few calls as possible** and confirm
   there's no artificial pacing, so it appears as one insert rather than a crawl.
   *Honest caveat:* in apps that process input per-keystroke (terminals, some
   Electron/React fields) injected keystrokes can still render visibly — that is
   inherent to not using the clipboard, and those users can opt into Clipboard
   paste (below).
2. **Kill the terminal corruption.** The "switching between stuff" is held
   modifiers: `Ctrl+Alt+V` is still physically down when chars inject, so the
   terminal sees `Ctrl+Alt+<char>` shortcuts. There's already a
   `wait_for_modifier_release` (`output.rs:585`) + a focus handoff
   (`ensure_foreign_focus` 510). **Verify both actually fire on the hotkey path**
   and, defensively, emit explicit key-up events for Ctrl/Alt/Shift/Win immediately
   before injecting. This is the concrete terminal fix.

### The opt-in for everyone else
Add a clearly-labeled **"Clipboard paste"** mode: set clipboard → `Ctrl+V` →
**leave the transcript on the clipboard** (no hacky restore). Honest framing in the
UI: *"Fastest and most compatible. Uses your clipboard — your previous clipboard is
replaced by the transcript."* This maps onto the existing `ClipboardRestore` variant
with the restore step removed.

### UI reframe (so the model is obvious)
- Rename paste methods to outcomes: **"Insert (clipboard untouched)"** [default] vs
  **"Clipboard paste"**.
- Fix the hardcoded buffer pill (`App.tsx:3463`) to show the real mode.
- "Cancel" → **"Discard"**, shown only while recording.
- Confirm/ą add a **transcribing spinner** state on the buffer/pill so "we're
  working, insert happens when done" is visible.

### Experiments — UIA insert APPROVED (running in Phase 1 as a subagent); restore parked
- **UI Automation atomic insert** (`TextPattern`/`ValuePattern`): a *true* atomic,
  clipboard-free, keystroke-free insert for **supported** controls (standard edits,
  many browser fields), with keystroke fallback. This is the only path that could
  give "atomic paste feel" **and** "never touch the clipboard" where it works. If a
  subagent proves it reliable across the owner's daily apps, it becomes the default.
- **Full-fidelity clipboard save/restore** (text + images + files): explore only if
  the owner later wants the Clipboard-paste mode to restore — owner currently rates
  this hacky and low-priority.

---

## 5. Agent orchestration & phasing

**Conflict reality:** `App.tsx`, `App.css`, `PillApp.tsx`, `pill.css` are shared by
B, C, E, F, H. Parallel agents on them will collide. Two viable models:

- **Model 1 (recommended): backend parallel, frontend serial.**
  - Parallel worktree agents: **A-backend**, **D-backend**, **G-backend** (separate
    Rust files — clean).
  - One **frontend owner** runs B → C → E → F → H in sequence (and folds in each
    backend's new IPC as it lands).
- **Model 2: split `App.tsx` first.** A prep agent extracts each view into its own
  module (`views/Dashboard.tsx`, `views/History.tsx`, …). Then frontend agents own
  separate files and parallelize. Higher upfront cost, better long-term + unblocks
  parallelism. Worth it given how much FE work is queued.

**Suggested phases:**
- **Phase 1 (highest impact):** A (insertion fix) + the B overflow/de-dup slice +
  fix the misleading pill. Ship + verify live. *(This is what the owner originally
  reached for.)*
- **Phase 2:** C + D (archive viewer, search/sort/combine) — the other big daily pain.
- **Phase 3:** E + F (models + settings/audio reorg + renames + cross-links).
- **Phase 4:** G + H (LM Studio surface, pill polish). G/H are independent and can
  overlap earlier phases since G is backend + a small card and H is mostly pill files.

Each workstream = its own branch/PR, verified against the Acceptance criteria, then
merged. Use `/verify` or the run skill to confirm behavior in the real app, not just
type-checks.

---

## 6. Coverage map (every raw owner ask → workstream)

| # | Owner ask | Workstream |
|---|---|---|
| 1 | Single atomic insert, not streaming/typing | A |
| 2 | Don't consume the clipboard | A |
| 3 | Spinner while transcribing, insert once done | A |
| 4 | "Insert into focused app" should inject, not type | A |
| 5 | A dedicated clipboard just for the transcript | A (already the buffer) |
| 6 | Terminal input corruption when pasting | A |
| 7 | Combine transcripts from the archive | D |
| 8 | See the entire transcription in-app | C |
| 9 | Open a transcript in an external editor | C |
| 10 | Are we saving `.txt`? address storage | (answered: SQLite) — C intro |
| 11 | Inline detail (words/lang/audio) + expand row | C |
| 12 | Multiple layers of archive navigation | C |
| 13 | Transcript text front-and-center, timestamp secondary | C / B |
| 14 | Sort + search archive by time | D |
| 15 | Efficient local-DB lookup | D (index; FTS5 if needed) |
| 16 | Buttons by "Play recording": open externally, see more | C |
| 17 | "Cancel" unclear | A (→ "Discard", recording-only) |
| 18 | Dashboard rework; tiles don't fit small view | B |
| 19 | 4-across breakpoint too early; overflow | B |
| 20 | Dev-mode resolution readout | B |
| 21 | "Clipboard preserved" + Change overflow; tighter copy | B |
| 22 | Dashboard vs Transcribe overlap | B |
| 23 | "Current status" duplicated | B |
| 24 | Active mic: show real device; pick in settings | B |
| 25 | Active model: drop "Selected"; concise + Manage | B |
| 26 | Output mode needs the most context | B / F |
| 27 | Buffer: don't show everything; "See more" | B / C |
| 28 | "See more" opens a larger viewer window | C |
| 29 | In-app read-only viewer, not `.txt` | C |
| 30 | Pill color configurable (note + default) | H |
| 31 | Pill more curved + narrower | H |
| 32 | Visualizer edges quieter (taper) | H |
| 33 | Custom vocabulary → Dictionary + explain (no/know) | F |
| 34 | Output-mode display poorly segmented | F |
| 35 | "Notes/Analysis" → Notes | F |
| 36 | Settings deep-links from sub-views | F |
| 37 | Models: default+storage first; accordion; internal scroll | E |
| 38 | Models: wasted right-side space | E |
| 39 | Models: drop "not downloaded" prominence | E |
| 40 | Models: "selected" twice; split download vs select | E |
| 41 | Audio: compartmentalize; compact on small | F |
| 42 | LM Studio status/connected/running + setup | G |
| 43 | Notes searchable, DB not `.txt` | (already DB) — D / F |
| 44 | Stats fine; dashboard boxes more concise | B |

---

## 7. Open questions for the owner

1. **Phase 1 scope:** start building with A + the B overflow slice once approved, or
   wait and tackle workstreams strictly in order?
2. **`App.tsx` split (Model 2):** worth the upfront refactor to unblock parallel
   frontend agents, or keep one frontend owner working serially?
3. **Combine output:** should "Combine" save a **new** archive entry, or just
   produce text you insert/copy without persisting? (Default assumed: offer both,
   persisting optional.)
4. **Dictionary presets:** want me to ship 2–3 starter prompts (e.g. a coding/tech
   one) or leave it blank with just the explainer?
5. **Dev resolution readout:** dev-build only, or a hidden debug toggle you can flip
   in the shipped app?
