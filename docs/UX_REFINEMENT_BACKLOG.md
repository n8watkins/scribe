# UX Refinement Backlog (captured 2026-06-21)

> Source: Nathan's spoken brain-dump. This is the **verbatim-intent capture** —
> every item, nothing dropped. Status is design/outline only; nothing here is
> built. Sequencing recommendation at the bottom. Separate from the in-flight
> 0.6.0 GPU release.

## A. Integrations (new architecture)

- **A1.** Add an **"Integrations"** item to the sidebar.
- **A2.** Treat **GitHub** (Sync — connects to GitHub, uploads notes) and the
  **Local LLM** (LM Studio/Ollama) as the two main integrations.
- **A3.** Inventory check — are there other integrations? (Answer: just those two;
  Google Drive was replaced by GitHub; the auto-updater isn't an integration.)
- **A4.** UX model: the Integrations view **lists your integrations**; enabling one
  **opens a tab showing (a) that integration's settings and (b) the features it
  enables**.
- **A5.** Move **Sync (GitHub)** under Integrations.
- **A6.** Move **Local LLM (LM Studio)** settings under Integrations.
- **A7.** Overall intent: reduce the "half-baked" feel — each feature should have
  real settings, grouped by the integration that powers it. Make a plan for the
  per-feature settings.
- **A8.** Icons: use the **LM Studio logo** and the **GitHub logo** to mark those
  integrations, and a **generic "integrations" icon** for the sidebar entry.

## B. Local-LLM hosting (key decision)

- **B1.** Want: can Scribe **run a local LLM itself** so LM Studio doesn't have to
  stay open? What are the options?
- **B2.** Idea: Scribe bundles/manages a small model (he named **Gemma 3 4B**) for
  basic tasks instead of depending on LM Studio.
- **B3.** Decide: external (LM Studio/Ollama) vs Scribe-managed. (Recommendation
  in §"Answers".)

## C. Dictation cleanup — REMOVE (decided, emphatic)

- **C1.** Remove **"Clean up dictation with a local LLM"** from Settings.
- **C2.** Remove the **"Cleanup style"** selector (Standard / Email / Chat / Code /
  Custom) + the custom prompt field.
- **C3.** Rationale: pointless; doesn't want two layers of cleanup; rely on the
  transcription model + filler suppression instead.
- **C4.** **Performance:** enabling it made the app **too slow** — that was the lag
  he noticed (NOT the GPU; see D1).
- **C5.** Keep the LLM for **Notes analysis** + **Selection transform**; only
  dictation cleanup goes.
- **C6.** (He asked what the styles meant — see §"Answers".)
- **C7.** Wording note (avoid conflation): mid-stream he said "rely on the local
  LLM to do that," then **corrected himself** — "I don't want to use a local
  model… rely on my transcription model… do the suppression as opposed to two
  layers of cleanup." So the intent is **transcription + filler suppression, NOT
  an LLM**, for dictation. He also asked "am I wrong?" — he isn't: filler
  suppression is deterministic, on-device, and free; LLM cleanup duplicates it and
  adds a round-trip.

## D. GPU / performance clarification

- **D1.** The lag while *dictating* was caused by **dictation cleanup** (a
  per-dictation LLM round-trip), **not** GPU acceleration. GPU is fine; removing
  cleanup fixes it.
- **D2.** SEPARATE concern (don't conflate with D1): he also felt **enabling/
  disabling the GPU toggle was slow**. The **GPU device probe** added this session
  runs `whisper-cli` (loads a model) when the **Audio tab opens** — a real, new
  source of settings-panel lag independent of dictation cleanup. Action: make the
  probe cheap — **cache** the result, run it **async/non-blocking**, and load the
  **smallest downloaded *Whisper* model** (e.g. tiny.en, 75 MB) for it. NOTE: the
  probe loads a **Whisper** model (not the LLM), purely so ggml-vulkan prints its
  device list; today it uses the *selected* model, which may be the 1.6 GB turbo.
- **D3.** GPU verification: GPU was measured **~16× faster end-to-end / ~88× on
  encode** (large-v3-turbo, jfk.wav, identical transcript) — it's genuinely
  engaged. Idea worth adding: a built-in **"Test GPU speed"** action so the user
  can re-confirm in-app anytime.

## E. Transform selection — expand + own settings

- **E0. Status: ON THE DOCKET, back burner.** The exact modes/output are an
  un-finalized backend feature decision — capture now, design/build later.
- **E1.** Should apply to **any text you copy OR highlight** (not just highlight).
- **E2.** Problem: highlighted text is often in a **read-only** spot — can't paste
  back in place.
- **E3.** Add a mode: put the transformed result on the **clipboard** (consume the
  clipboard with the transformation) so you can paste it anywhere.
- **E4.** **Auto-paste** option.
- **E5.** Treat the transform result like the **Last Transcript buffer** —
  retrievable / re-pasteable.
- **E6.** Give Transform selection its **own dedicated settings** (niche but
  genuinely useful).

## F. Translate / language gating

- **F1.** Don't allow selecting a non-English language **unless a multilingual
  model is installed AND selected**.
- **F2.** Don't allow enabling **Translate → English** under the same condition.
- **F3.** Keep the controls **visible but disabled**, with a note: *"You must
  enable a multilingual model."*
- **F4.** English selected by default; language/translate become editable only once
  a multilingual model is installed + selected.

## G. Floating pill / startup

- **G1.** Floating pill currently **not visible** — investigate (possible startup
  bug; may just need a restart).
- **G2.** **Launch at startup** should default **ON**.

## H. Data & Privacy — Local data UI redesign

- **H1.** Remove the **"Folders on this device"** heading; keep one heading
  ("Local data").
- **H2.** Small description under "Local data": *"where Scribe keeps your data."*
- **H3.** Accordion on the **right** side; clicking the header toggles it.
- **H4.** **Path-column width must be consistent** across all rows — today it
  cascades narrower as the left descriptions get longer.
- **H5.** Allow **2-line** wrapping (for the description AND for very long paths —
  he noted paths can be long enough to need two lines) so the fixed-width path
  column still fits everything.
- **H6.** **Hover a path → tooltip with the full path.**
- **H7.** **Copy-path** button per row.
- **H8.** Stop repeating the word **"folder"** — name rows "Local data", "Local
  models", "Local logs", "Failed recordings", etc.
- **H9.** Description column can be a bit **shorter** (they're all folders).

## I. Settings reorg — window options → Developer

- **I1.** Move **"Default window size"** to Developer settings.
- **I2.** Move **"Reopen Scribe at the window's current size"** to Developer
  settings (it's currently in Data & Privacy).

## J. Hotkeys UI density

- **J1.** Make hotkey rows **less tall** so they all fit **in one scroll**.
- **J2.** Keep the **"click to change" container a fixed size**; shrink the text
  inside to fit if it's too long, so the layout doesn't shift.

## K. Models view

- **K1.** Make the **Active model / base** display match the **Dashboard** style
  (preferred over the current Models layout).
- **K2.** Use **icons** to indicate what's being shown.

## L. Audio view — tabs

- **L1.** Split Audio into **tabs** (like Settings' App / Output / Dictionary /
  Notes).
- **L2.** Tab 1: **Input + Device health** (the main pane, with a header).
- **L3.** Tab 2: **Audio processing** (+ **GPU acceleration** can live here).
- **L4.** **Record test** should be **disabled while actively dictating** — detect
  an in-progress recording instead of allowing a test mid-dictation.

## M. Themes

- **M1.** Themes are good as long as they're **saved**.
- **M2.** **Edit / duplicate a preset** to make a custom (e.g., Violet → Edit →
  creates a custom copy).
- **M3.** Browse **multiple custom** themes.
- **M4.** Make the theme list **thinner** — likely drop the per-theme descriptions.
- **M5.** Custom-theme controls visible **without scrolling**.

## N. Setup view

- **N1.** Surface **errors** (e.g., a failed hotkey assignment shows in Setup).
- **N2.** **Guide** missing config: set hotkeys, choose microphone (show current),
  download/choose a model (show current).
- **N3.** Visual indication for missing/incomplete steps.

## O. Transcribe view

- **O1.** Make **Output behavior** its own option/tab (move output settings into an
  "Output behavior" tab).
- **O2.** Orient toward **multiple tabs** vs everything on the first view (Record /
  Output behavior / Transcribe a file).
- **O3.** File transcription: **show the file path / where it transcribes**; allow
  changing it.
- **O4.** **Drag-and-drop** a file to transcribe.
- **O5.** (Idea) Transcribe-a-file could be a **settable keybind**.
- **O6.** The view looks **sparse** — flesh it out.

## P. Deferred for now (NOT being refactored this pass)

- **P1. History** — not refactoring now.
- **P2. Notes** — not refactoring now.
- **Correction (supersedes pass 2):** **Transcribe IS in scope.** He clarified —
  "transcribe as an option is not the same thing as history and notes; we're
  factoring transcribe, we're not refactoring history and notes for now." So **§O
  (Transcribe) is ACTIVE**; the deferred pair is **History + Notes**. (Pass 2
  wrongly deferred Transcribe — fixed.)

## Q. Silence should produce empty, not invented text (bug)

- **Q1.** Toggling record on/off with **no speech** returns Whisper text (a silence
  hallucination) instead of empty.
- **Q2.** Want: if nothing was heard, **return empty** — and ideally **don't run
  the model at all** on effectively-silent audio.
- **Cause:** Whisper invents plausible text from silence/near-silence. The sub-300
  ms min-duration gate + the `[BLANK_AUDIO]` strip don't catch a slightly-longer
  near-silent clip. **Fix:** gate on speech presence before transcribing — if the
  (trimmed) audio has no samples above the speech-RMS threshold, skip the model and
  return empty. (Reuses the RMS thresholds the auto-stop logic already computes.)

## Cross-cutting themes
- Recurring **"multiple tabs"** pattern (Audio, Transcribe) to declutter first
  views.
- Every feature should feel finished: real settings, clear what it does, no
  half-baked surfaces.

## Execution plan (multi-step — check off as we go)

### Phase 0 — fold into the 0.6.0 GPU release (small, decided)
- [ ] **C** Remove dictation cleanup (toggle + Cleanup style + custom prompt; keep
  LLM for Notes/Transform).
- [ ] **D2** Make the GPU device probe cheap (cache + async + smallest Whisper
  model) so Audio doesn't hitch.
- [ ] **Q** Silence → empty: skip the model on effectively-silent audio.
- [ ] Ship **0.6.0** (GPU + the above).

### Phase 1 — Integrations architecture
- [ ] **A1–A6** "Integrations" sidebar section; move Sync (GitHub) + Local LLM (LM
  Studio) under it; enabling one opens its settings + the features it enables.
- [ ] **A8** Integration icons (LM Studio, GitHub) + generic sidebar icon.
- [ ] **B/C** Decide + (if chosen) build Scribe-managed local LLM (llama-server +
  downloadable GGUF + reuse Vulkan GPU); keep LM Studio/Ollama as BYO-server.

### Phase 2 — per-view UX pass (mostly frontend)
- [ ] **L** Audio → two tabs (Input+Device health / Audio processing+GPU); **L4**
  disable Record-test while actively dictating.
- [ ] **H** Data & Privacy local-data redesign (one heading, consistent path
  column, hover/copy path, drop "folder" repetition, 2-line wrap).
- [ ] **I** Move both window options → Developer settings.
- [ ] **J** Hotkeys: shorter rows (fit one scroll), fixed change-box with
  shrinking text.
- [ ] **K** Models: match Dashboard's active-model display + icons.
- [ ] **M** Themes: edit/duplicate preset → custom, browse customs, thinner list,
  no-scroll custom controls.
- [ ] **N** Setup: surface errors + guide missing mic/model/hotkeys (show current).
- [ ] **F** Translate/language gating (visible-but-disabled until a multilingual
  model is installed + selected).
- [ ] **G** Floating-pill-not-visible bug + launch-at-startup default ON.
- [ ] **D3** (optional) "Test GPU speed" action.

### Phase 3 — bigger feature work (on the docket)
- [ ] **O** Transcribe refactor (Output-behavior tab, multi-tab, file path +
  drag-and-drop, optional keybind). *(Transcribe is in scope; History/Notes are
  not.)*
- [ ] **E** Transform selection expansion (clipboard mode, auto-paste, buffer, own
  settings) — back burner, design first.

### Deferred (captured, not this cycle)
- History (P1), Notes (P2).
