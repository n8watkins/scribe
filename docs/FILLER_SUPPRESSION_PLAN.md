# Pause-Aware Filler Suppression — Plan (design only)

> **Status: design / not started.** How a user-configurable, pause-aware filler
> remover would work in Scribe, and what it would take. Nothing here is built. See
> the open-items list in [`STATUS_AND_NEXT_STEPS.md`](STATUS_AND_NEXT_STEPS.md).

## 1. Goal

Remove genuine **disfluency filler** the user actually said — "um", "uh", "er",
"hmm" (and optionally "like", "so", "oh") — from dictation, **without** mangling
the same words when they're meaningful ("**Oh** no", "**like** this", "**so**
good"). The discriminator is a **pause**: a filler bracketed by silence is a
hesitation and should go; a filler tight against its neighbours is fluent speech
and should stay.

This is distinct from two things that already exist:
- The **hallucination denylist** (`strip_hallucinated_sentences` in `whisper.rs`)
  removes words Whisper *invented* on silence. Filler suppression removes words
  the user *spoke*. Keep them separate.
- The optional **AI dictation cleanup** (LLM) already strips filler
  context-aware, but needs a running local LLM and adds latency. This feature is
  the deterministic, no-LLM, always-on-if-enabled alternative.

## 2. Current state (what we're changing)

- Whisper runs with **`--no-timestamps`** (`whisper.rs` `whisper_args`) and
  `--output-txt`; the warm server is asked for a **text-only** response
  (`whisper_server.rs` parses `{ text }`). **All per-word timing is discarded
  today** — the overnight audit (`OVERNIGHT_REPORT.md` §7) flagged this.
- Post-processing happens in `normalize_transcript_text` (shared by the CLI and
  server paths) — the natural home for a text-level filter, but it only has the
  string today, not timings.
- **The pattern to copy already exists:** the dictionary **replacements**
  (`text_replacements: Vec<TextReplacement>`) are a user-editable list in
  `AppSettings` with an add/remove table in the Dictionary view. The filler list
  is the same shape.
- The incremental segmenter (`incremental.rs`) already **cuts phrases on
  silence** (`segment_pause_ms`) — so a segment boundary *is* a detected pause
  (useful synergy, see §4).

## 3. Settings (user-configurable)

Three new `AppSettings` fields, each with a serde default (so they degrade per
the R3 container default):

| Setting | Type | Default | Meaning |
|---|---|---|---|
| `filler_suppression_enabled` | `bool` | `false` | Master on/off, like the other cleanup features. |
| `filler_words` | `Vec<String>` | `["um","umm","uh","uhh","er","erm","hmm"]` | The user's editable list. Lowercased/trimmed/deduped on save; clearing it disables removal. |
| `filler_pause_threshold_ms` | `u32` | `300` | How big an adjacent silence (ms) makes a filler removable. Lower = more aggressive. Validated to a sane range (e.g. 100–1500). |

UI: an editable **"Filler words"** table in the Dictionary view (mirror the
replacements table) + the toggle and a threshold slider next to it. No redesign.

## 4. The algorithm

### 4a. Get the timings (the core change)
Turn on Whisper's token timestamps and parse them:
- **CLI path:** replace `--no-timestamps`/`--output-txt` with
  `--output-json-full` (`-ojf`). whisper.cpp then emits, per token, the text and
  millisecond `offsets {from,to}`.
- **Server path:** request a verbose/timestamped `response_format` from
  `whisper-server` and parse the token/segment offsets. **Must mirror the CLI**
  (verify the exact response shape against the bundled `whisper-server.exe`, like
  the `server_args` parity we already maintain).
- Reconstruct **words** from sub-word tokens (whisper marks word starts with a
  leading space), carrying each word's `start`/`end` ms.

### 4b. The pause rule
For each word whose normalized form (lowercased, punctuation-stripped) is in
`filler_words`:
- `gap_before = word.start − prev_word.end`
- `gap_after  = next_word.start − word.end`
- Remove the word iff `max(gap_before, gap_after) ≥ filler_pause_threshold_ms`.
- **Edges:** a filler at the very start/end of a segment has no neighbour on one
  side — treat that side as a pause (a leading "Um, so…" is almost always a
  hesitation). Because segments are cut on silence, a segment edge already *is* a
  pause, so this is consistent with 4a's data.

Then re-run the existing whitespace/punctuation tidy so removed words don't leave
double spaces or orphaned commas.

### 4c. Where it runs
A new step in the transcription pipeline **after** timing-aware reconstruction,
**before** `text_replacements`/AI cleanup. It needs the timings, so it can't live
purely in `normalize_transcript_text` (string-only) — the timing parse + filler
filter run first, and hand a clean string onward.

## 5. Workstreams

- **WS1 — Capture timestamps (CLI).** Swap to `--output-json-full`, parse tokens
  + ms offsets, reconstruct the plain text identically to today (so non-filler
  output is unchanged). The riskiest change — it touches the core parse.
- **WS2 — Capture timestamps (server).** Same data from `whisper-server`; verify
  the response format against the bundled binary; keep CLI/server output
  identical.
- **WS3 — Filler filter.** The §4b gap rule over the reconstructed words. Pure,
  fully unit-testable on Linux (feed synthetic word+timing lists).
- **WS4 — Settings.** The three fields + validation + a `defaults_version`
  migration if needed; wire through to the transcription path.
- **WS5 — UI.** Filler-words table (reuse the replacements component) + toggle +
  threshold slider in the Dictionary view.
- **WS6 — Interplay.** Define ordering with AI cleanup (filler strip first, then
  optional LLM polish) and the hallucination denylist (independent).

## 6. Risks & considerations

- **Token-timestamp accuracy.** whisper.cpp's default token times are
  approximate (not DTW-aligned). Fine for "is there a ~300 ms gap?" but not
  millisecond-exact. `--dtw <model>` is more accurate but needs an alignment
  model + flag — note as a later precision upgrade, not v1.
- **Output-format blast radius.** Moving off `--output-txt` to JSON changes the
  single most important parse in the app. The plain-text result for normal
  dictation must be byte-identical to today (guard with tests) — this is the main
  correctness risk.
- **Server/CLI parity.** If the warm server can't emit the same timings, the two
  paths would behave differently (the warm path is the default). Verify early; if
  the server can't, fall back to applying the filter only on the CLI path, or
  request word timestamps a different way.
- **Performance.** Timestamped decoding is negligibly slower; the parse is cheap.
- **Latency vs. the LLM cleanup.** This is the deterministic, instant option;
  it complements (doesn't replace) the context-aware LLM cleanup.
- **Windows-gated.** The transcription path is the Windows surface — needs real
  on-device testing + the CI `cargo check`.

## 7. Acceptance criteria

- "I went **um** to the store" (um flanked by a pause) → "I went to the store".
- "**Oh** no." / "I want it **like** this." (no adjacent pause) → unchanged.
- `filler_suppression_enabled = false` → output byte-identical to today.
- Editing `filler_words` changes what's removed; clearing it removes nothing.
- Raising/lowering `filler_pause_threshold_ms` visibly changes aggressiveness.
- Normal (non-filler) transcripts are byte-identical CLI vs. server vs. today.
- The filler filter is covered by Linux unit tests over synthetic timed words.

## 8. Effort

~**2–4 days.** The bulk is WS1/WS2 (timestamp capture + parse on both paths, kept
output-identical) and the on-Windows verification; WS3 is small and testable, WS4
mirrors existing settings, WS5 mirrors the existing table UI.

## 9. Open questions

1. ~~Can the bundled `whisper-server.exe` return token/word timestamps?~~
   **RESOLVED (verified against the bundled v1.8.6 binaries):** CLI
   `--output-json-full` gives per-token `offsets` in ms; the server's
   `response_format=verbose_json` returns per-word `segments` with `start`/`end`
   (seconds) plus a `words` array. Both paths can do it.
2. v1 filler defaults: safe-only (`um/uh/er/hmm`), or include the risky
   (`oh/like/so`) on the strength of the pause guard?
3. Default threshold — is 300 ms right, or start more conservative (~400 ms)?
4. Should a removed filler that *was* a whole segment also collapse that segment
   (so "Um." alone → nothing), reusing the empty→no-paste path?

## 10. Rollback (how to remove this cleanly)

The feature is **off by default** and entirely behind `FillerConfig` — when off,
the transcription path is byte-for-byte unchanged, so the simplest "removal" is
just leaving it disabled. To delete it outright:

**Commits** (`git log --grep='feat(filler)'`):
- `4520c16` WS3 core (`filler.rs::suppress_fillers` + tests)
- `5945471` WS4 settings (3 `AppSettings` fields)
- `2aff990` WS1 parse (`whisper::parse_cli_words` + tests)
- `3722e50` WS1/WS2 pipeline wiring
- `01641f6` WS5 settings UI

**Files & touch-points** (every integration line is marked `// FILLER`):
- **New, delete entirely:** `app/src-tauri/src/filler.rs` (+ `pub mod filler;` in
  `lib.rs`).
- `whisper.rs` — `WhisperRequest.filler` field; the `word_timestamps` param +
  output-format switch in `whisper_args`; the `Some(config)` branch in
  `transcribe_with_output_prefix`; `parse_cli_words` (+ its tests).
- `whisper_server.rs` — the `server_eligible = request.filler.is_none()` guard in
  `transcribe`.
- `dictation.rs`, `incremental.rs` (`SegmentContext.filler`), `file_transcribe.rs`
  — the `filler: …` line in each `WhisperRequest`.
- `settings.rs` — `filler_suppression_enabled` / `filler_words` /
  `filler_pause_threshold_ms` (+ defaults, validation, tests).
- `backend.ts` — the three `filler*` type fields.
- `Settings.tsx` — the "Filler suppression" subsection + `parseFillerWords` +
  the `MsInput` import.

Cleanest revert: `git revert 01641f6 3722e50 2aff990 5945471 4520c16` (newest
first), or delete the marked lines + `filler.rs` by hand. No data migration is
needed — the unused settings columns are harmless if left.
