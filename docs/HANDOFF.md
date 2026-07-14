# Scribe - Session Handoff

> **Archived handoff notice, 2026-07-14:** This document records the 0.5.x and 0.6.0 work and must not be used as current operating instructions.
> Current builds use optional private GitHub backup rather than Google Drive, check for updates every six hours rather than every minute, and do not include the removed local-LLM dictation cleanup path.
> The Authenticode and SignPath plan was deliberately dropped; only updater artifacts are cryptographically signed.
> Multilingual transcription and Vulkan acceleration have shipped.
> The release workflow now validates exact versions, dependencies, checksums, SBOM output, and both Windows installers before publishing.
> See the [root README](../README.md), [app README](../app/README.md), and active workflow files for current instructions.

> **Current as of 2026-06-21, version 0.6.0.** For the running feature log read
> [`CHANGELOG.md`](../CHANGELOG.md); for the **active roadmap** (phased, checkbox
> tracker) read [`docs/UX_REFINEMENT_BACKLOG.md`](UX_REFINEMENT_BACKLOG.md); for
> competitive positioning read [`docs/COMPETITIVE-ANALYSIS.md`](COMPETITIVE-ANALYSIS.md);
> for longer history read [`docs/STATUS_AND_NEXT_STEPS.md`](STATUS_AND_NEXT_STEPS.md).
> The Project summary / release process / gotchas below stay accurate; the
> 0.5.22-era "State" and "Next steps" sections further down are **historical** —
> the current state and next steps are immediately below.

## Current state — v0.6.0 (shipped 2026-06-21)

**GPU acceleration via Vulkan shipped, plus a Phase 0 cleanup pass.** All on
`main`, released as **v0.6.0** (CI green, installer published). This session's
commits: `git log v0.5.24..HEAD` (newest `38d7f5d` … oldest `e0a5eee`).

- **GPU (Vulkan).** whisper.cpp built from source with `-DGGML_VULKAN=ON` in
  `release.yml` (pinned **v1.9.1**), `ggml-vulkan.dll` bundled. **~16× faster
  end-to-end / ~88× encode** on the RX 7800 XT (`large-v3-turbo`), measured. On
  by default (`gpu_acceleration` = Auto), automatic CPU fallback, device pin via
  `GGML_VK_VISIBLE_DEVICES`, GPU→CPU retry on failure. UI: **Audio → GPU
  acceleration** (probe-backed status + toggle + multi-GPU device picker).
  Installer download grew only ~7 MB.
- **Removed local-LLM dictation cleanup** — duplicated filler suppression and
  added a per-dictation round-trip (the lag). The LLM now powers only **Notes
  analysis + Selection transform** (both on-demand, never in the dictation path).
- **Silent recordings return empty** (no Whisper hallucination):
  `audio::wav_has_speech` gates the full-clip path; fails open.
- **Cheap GPU probe** — cached + async (`spawn_blocking`) + smallest downloaded
  Whisper model (`gpu.rs`).
- **Catalog**: full fp16 `large-v3-turbo`; **English/Multilingual filter** in the
  model browser.

Verified: **216** backend lib tests, `tsc` + frontend build clean, CI Windows
`cargo check --all-targets` green, release build green + published, GPU
benchmarked on the actual 7800 XT.

## Next: Phase 1 — Integrations + managed local LLM

**Read [`docs/UX_REFINEMENT_BACKLOG.md`](UX_REFINEMENT_BACKLOG.md) first** — it's
the phased, checkbox roadmap (Phase 0 is checked off there). Phase 1:

1. **Integrations sidebar** (backlog §A) — new "Integrations" nav section; move
   **Sync (GitHub)** and **Local LLM** under it; enabling an integration opens
   its settings + shows the features it unlocks. Icons: GitHub + LM Studio logos
   + a generic integrations icon.
2. **Scribe-managed local LLM** (backlog §B) — the "inside the app" goal: a
   **`llama-server`** (llama.cpp, same ggml family as whisper.cpp) that Scribe
   spawns/manages itself like `whisper_server.rs`, **GPU-accelerated via the
   Vulkan backend shipped in 0.6.0**, OpenAI-compatible so `note_analysis` /
   `selection_transform` clients don't change; download-on-enable GGUF (e.g.
   Gemma 3 4B) mirroring the Whisper model catalog. **NOTE:** Ollama *and* LM
   Studio are both **external** servers — Ollama already works as BYO
   (`http://localhost:11434/v1`); neither is "inside the app." Only managed
   llama-server is. **Spike first** (prove llama-server runs GPU-accelerated on
   the 7800 XT, mirroring `docs/GPU_VULKAN_SPIKE.md`) before wiring it in.

Phase 2 (per-view UX) and Phase 3 (Transcribe refactor; Transform-selection
expansion) follow — all itemized in the backlog. **History + Notes are deferred.**

## Project summary

Scribe is a free, open-source (**MIT**), Windows-only desktop app for private,
on-device dictation. Stack: **Tauri v2** — Rust backend in `app/src-tauri/src`,
React + TypeScript + Vite frontend in `app/src`. Transcription is fully local via
whisper.cpp (a resident `whisper-server.exe`, with a `whisper-cli` fallback).

Distribution: **signed GitHub Releases** (NSIS `setup.exe` + MSI + a signed
updater `latest.json`), built by **GitHub Actions** on every `v*` tag. Free CI
(public repo, no minute cap).

Core loop: global hotkey → record → local Whisper → text inserted at the cursor.
On top: quick notes (`~`+Q) with local-LLM analysis, AI dictation cleanup,
selected-text transform (voice + typed), searchable history + retention,
multilingual transcription + translate-to-English, 6 color themes, Google Drive
sync + export, a floating status pill, and a branded auto-updater.

- **Repo (source of truth):** `/home/natkins/n8builds/public/scribe` (WSL) →
  `https://github.com/n8watkins/scribe` (public, MIT).
- **Build platform:** GitHub Actions `windows-latest`. The app is Windows-only;
  most input/audio code is `#[cfg(windows)]`.
- **Installed binary is `app.exe`** (the Cargo crate is named `app`; product name
  is "Scribe") at `%LOCALAPPDATA%\Scribe\app.exe`; data in
  `%APPDATA%\com.natkins.scribe\`. The running process is `app.exe` (+ child
  `whisper-server.exe`), *not* `Scribe.exe`.

## State (this session: 0.5.11 → 0.5.22 — all committed AND pushed)

HEAD `1dc0c89` on `main`; latest release **v0.5.22** (CI green, published).
Highlights (full detail in `CHANGELOG.md`):

- **0.5.12** AI dictation cleanup; selected-text transform (typed v1); Sync
  all-transcripts Drive backup + Markdown/CSV/JSON export; notes auto-pruning.
- **0.5.13** OS-notification update alerts + faster checks; `FAQ.md`.
- **0.5.14** custom window title bar (`decorations: false`).
- **0.5.15–0.5.18** update-UX iteration → a single versioned chip + About as the
  update hub + an "auto-check" toggle.
- **0.5.19** seamless **branded auto-update** (auto-install on launch) + **6 color
  themes**.
- **0.5.20** code-review fixes for auto-update + themes.
- **0.5.21** **multilingual** transcription (+ multilingual models, language
  picker, Translate→English) + a **readable light theme** ("Daylight").
- **0.5.22** **update-check 403 fix** — detection moved off the GitHub REST API to
  the updater's `latest.json` (release CDN).
- post-0.5.22 (`d7e9160`, `1dc0c89`): **MIT** license metadata + a **SignPath
  Foundation** code-signing credit in the README (for the signing application).

Verified: **171 backend tests pass** (`cargo test`); `tsc --noEmit` + `npm run
build` clean. The maintainer is running 0.5.22 (installed via WSL→Windows).

## Key decisions & gotchas (do NOT re-ask, do NOT "fix")

1. **Update polling is intentionally at a 1-minute cadence** (`app/src/App.tsx`,
   the update-check effect: `setInterval(() => runCheck(false), 60 * 1000)`).
   It's a deliberate **test** cadence. **Do NOT change it to the production ~6h
   until the maintainer explicitly says "done testing the updater."**
2. **Update detection = `app/src/lib/updates.ts` `detectUpdate()`**, which calls
   the updater plugin's `check()` (fetches `latest.json` from the release CDN) —
   **not** the GitHub REST API. The old `update_check.rs` / `checkForUpdate`
   command is now unused by the frontend (left in place). Reason: the REST API
   caps at ~60 req/hr unauthenticated, and 1-min polling tripped a **403** that
   silently disabled detection. This is the 0.5.22 fix — don't revert it.
3. **Auto-install is launch-only**, gated on settings actually being loaded; a
   code review confirmed there is **no** mid-session install/close path. See
   `runAutoInstall` / `maybeRunLaunchInstall` in `App.tsx`. Failures fall back to
   manual install and never block the app.
4. **`google_secrets.rs` is gitignored** (real Google OAuth id/secret).
   `build.rs` recreates a placeholder from `google_secrets.example.rs`; CI injects
   the real values from GitHub Actions secrets. **Never commit the real secrets.**
5. **Multilingual models ship without a checksum** (`expected_sha1: None`) —
   upstream no longer publishes a plain-content SHA1 that matches the verifier; a
   fabricated one breaks downloads. Intentional; downloads are HTTPS.
6. **`#[cfg(windows)]` code only compiles on CI's `windows-latest` runner.**
   `cargo test` on Linux skips those branches, so a feature's Windows code is
   first compiled by CI — always `gh run watch <id> --exit-status` after a tag.
7. The **UI reorg is HELD** (the maintainer took it back). The big themes/
   multilingual/auto-update features were built via **parallel worktree agents**.

## Release process (the flow that works)

1. Bump the version in **3 files**: `app/src-tauri/Cargo.toml`,
   `app/src-tauri/tauri.conf.json`, `app/package.json`.
2. `cd app/src-tauri && cargo check` (refreshes `Cargo.lock`'s app version).
3. Add a dated section to `CHANGELOG.md`.
4. `git commit` (with the `Co-Authored-By: Claude ...` trailer), `git tag vX.Y.Z`,
   `git push origin main && git push origin vX.Y.Z`.
5. CI builds the signed release (~14–16 min): `gh run watch <run-id>
   --exit-status`, then confirm `gh release view vX.Y.Z`.

## Verification & parallel-work commands

- **Backend:** `cd app/src-tauri && cargo test` (171 tests).
- **Frontend:** `cd app && npx tsc --noEmit && npm run build`.
- **Parallel feature work:** `git worktree add -b feat/X ../scribe-wt-X HEAD` then
  `ln -sfn /home/natkins/n8builds/public/scribe/app/node_modules
  ../scribe-wt-X/app/node_modules`; run one agent per worktree; then `git merge`
  sequentially (additive conflicts land in `settings.rs` / `backend.ts` /
  `lib.rs` / `App.tsx` — keep both sides). The Agent tool's `isolation:"worktree"`
  is broken in this environment — use manual worktrees.

## Next steps (ordered)

1. **Code signing via SignPath Foundation (in progress).** Repo is MIT and the
   README credits SignPath. The maintainer is filling the SignPath Foundation OSS
   application. **BLOCKER:** the repo is days old with 0 stars/forks, and SignPath
   requires "widely used or trusted" — so the application will likely be deferred
   until the project has traction. Options: apply anyway (likely "come back
   later"); wait for traction; or **Azure Trusted Signing (~$10/mo, no reputation
   gate)** to sign now. Once approved → add the **SignPath GitHub Action** to
   `.github/workflows/release.yml` (needs a SignPath API token as a GH secret +
   org/project/signing-policy slugs from the maintainer) to Authenticode-sign the
   NSIS `setup.exe`. Acceptance: signed installer, no SmartScreen warning.
2. **Flip updater polling 1-min → ~6h** (the `App.tsx` interval) — ONLY when the
   maintainer says "done testing the updater."
3. **Competitive gaps** (see `COMPETITIVE-ANALYSIS.md`; multilingual is now DONE):
   custom/arbitrary local model selection (point Scribe at any `ggml` `.bin`);
   spoken punctuation / voice commands / voice editing; real-time streaming
   insertion; a first-run onboarding wizard.
4. **UI reorg — HELD.** A ~25-item IA reorg (sidebar reorder, Settings/Audio/
   Themes split, "Backup" rename, History/Notes layout) was captured but pulled.
   Don't start without re-confirming with the maintainer.
5. **Light-theme polish:** move the floating-pill colors into the Themes view.

## File map (current-work pointers)

- `app/src/App.tsx` — update-check effect (poll cadence + launch auto-install),
  theme apply (`data-theme`), custom title bar, nav, `renderView`.
- `app/src/lib/updates.ts` — `detectUpdate()` (latest.json detection; 0.5.22 fix).
- `app/src/components/UpdateOverlay.tsx` — branded auto-update screen.
- `app/src/views/About.tsx` — Updates hub (Install / Check / View releases +
  auto-check & auto-install toggles).
- `app/src/views/Themes.tsx` + `app/src/App.css` — themes (`--scribe-*` vars +
  `[data-theme]` presets; `midnight` is the default = historical look).
- `app/src/views/Settings.tsx` — language picker + Translate→English; dictation
  cleanup; dictionary; notes-LLM analysis.
- `app/src-tauri/src/models.rs` — model catalog (English + multilingual, with a
  `multilingual` flag).
- `app/src-tauri/src/settings.rs` — `AppSettings` (`Language` is now an ISO-code
  string; `theme`, `auto_install_updates`, `auto_update_check_enabled`,
  `translate_to_english`, retention fields, etc.).
- `app/src-tauri/src/dictation.rs` / `whisper.rs` / `whisper_server.rs` —
  transcription pipeline, `--language`/`--translate` args, dictation-cleanup hook,
  selection-transform routing.
- `app/src-tauri/src/selection_transform.rs` — selected-text transform engine.
- `app/src-tauri/src/google_*.rs` / `note_sync.rs` — Drive OAuth / sync / export.
- `.github/workflows/release.yml` — CI signed-release build (where SignPath
  signing will be wired in).
- `docs/COMPETITIVE-ANALYSIS.md` — competitor matrix + prioritized roadmap.
