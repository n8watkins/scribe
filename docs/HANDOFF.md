# Scribe handoff

## Current state and open work (2026-10-27): start here

This section is the current entry point for an agent starting in this repository.
Everything under "Historical record" further down describes the 0.5.x and 0.6.0 work and is not current; where it disagrees with this section, this section wins.

### What Scribe is and where it lives

Scribe is a free, open-source (MIT), Windows-only desktop app for private, on-device dictation, built with Tauri v2 (Rust backend in `app/src-tauri/src`, React and TypeScript front end in `app/src`), with local Whisper transcription.

- Repository: https://github.com/n8watkins/scribe, cloned at `~/projects/tools/scribe/scribe-app`.
- Releases: GitHub Releases (NSIS and MSI installers plus a signed updater `latest.json`); installed copies auto-update from the latest release.
- Website: https://scribe.n8builds.dev, from the separate `n8watkins/scribe-web` repository (`~/projects/tools/scribe/scribe-web`).

Current product and operations docs are listed in the [documentation index](README.md); the current release state is in [STATUS_AND_NEXT_STEPS.md](STATUS_AND_NEXT_STEPS.md), and the assessment plan is [ASSESSMENT_PLAN_2026-10.md](ASSESSMENT_PLAN_2026-10.md).

### Status today

- Latest release: **v0.8.0**, tagged 2026-07-14.
  No release since; `main` has 15 commits on top of it.
- Merged on `main` but **not released**, so no user has them yet:
  - #40, the security fixes: 67 of 68 Dependabot alerts cleared (npm lockfiles, and `cargo update` for the RustSec advisories).
  - #41, the assessment plan.
  - #42, release-kit 2.0.0 for versions, tags, changelog and release notes (see "Release process" below).
- One Dependabot alert stays open: `glib` 0.18 (RUSTSEC-2024-0429, GHSA-wrw7-89jp-8q8g), pulled in only by Tauri's Linux GTK stack.
  It does not affect the Windows app and waits on Tauri moving to `glib` 0.20.
- `.cargo/audit.toml` ignores RUSTSEC-2026-0367, 0368 and 0369 (`zip` 4.x, used only by `tauri-plugin-updater` 2.x, which verifies the update's signature before extracting it).
  Remove those ignores once `tauri-plugin-updater` 3 is stable and moves to `zip` 9.
- One Dependabot PR is open: #38 (GitHub Actions versions, opened 2026-10-13, all checks green).
  The older ones from July to October (#20, #32, #37, #39) were closed on 2026-10-27, superseded by #40.
- The website went live on 2026-10-27 with drawn app previews; it waits on real Windows screenshots and recordings from the assessment.
- The full assessment (code, security, UI and UX) has not been run yet.
- Code signing: only the updater artifacts are signed (minisign); the installers have no Authenticode signature, so Windows SmartScreen warns on first install.

### Open problems

1. **Security fixes are unreleased.**
   Users on v0.8.0 still run the vulnerable dependency versions until a new release ships.
   A release auto-updates every installed copy, so cutting one needs the owner's go-ahead.
2. **The "type it out" paste fallback has no setting** (to confirm in the assessment).
   The paste method (`PasteMethod` in `app/src-tauri/src/settings.rs`, applied in `app/src-tauri/src/output.rs`, including `DirectInsert`, which types the text) appears to have no control in the Settings view.
   Reproduce: open Settings in the installed app and look for a way to choose between pasting and typing.
3. **Ctrl+Alt+R (rewrite selection) is bound even when the AI tools are off** (to confirm in the assessment).
   The default chord is in `app/src-tauri/src/settings.rs`; registration is in `app/src-tauri/src/hotkeys.rs` (`TransformSelection`).
   Reproduce: turn the AI features off, select text in another app and press Ctrl+Alt+R.
4. **Dependabot PRs sat open for months.**
   #38 is the one left; merge it, and decide how Dependabot PRs get handled so they don't pile up again.
5. **Unmerged local branch `feat/rust-storage-profiles`.**
   The local checkout is on it; it holds 5 commits that are not on `main` (smaller Rust build artifacts, an NSIS smoke-test uninstall fix, frontend lint dependency updates).
   Its PR #18 was closed without merging on 2026-07-27, and the branch no longer exists on GitHub.
   A Windows worktree for validating it exists at `/mnt/c/Users/natha/Projects/Tools/scribe-rust-storage-validation` (branch `validation/scribe-rust-storage-windows`).
   Decide whether to revive it (rebase onto `main`, open a new PR) or drop it; don't delete the Windows worktree without the owner.

### Blocked on the owner

- **Go-ahead for the next release** (v0.8.1 or v0.9.0), because releases auto-update installed copies.
  The main reason to release is the security fixes in #40.
- **A code-signing certificate** for Authenticode-signing the installers (SignPath was dropped; Azure Trusted Signing or a bought certificate are the options).
- Choosing which assessment findings to act on, once the report exists.

### Next steps, in order

1. Run the full assessment per [ASSESSMENT_PLAN_2026-10.md](ASSESSMENT_PLAN_2026-10.md), including the real Windows screenshots and recordings the website needs, and confirm or rule out open problems 2 and 3.
2. Present the release decision to the owner, with the security fixes (#40) as the main reason to release; once approved, cut it with `npm run release -- --push` (see "Release process" below).
3. Merge Dependabot #38 before the release so it ships with up-to-date Actions.
4. Hand the screenshots and recordings to the `scribe-web` repository to replace the drawn previews.
5. Act on the assessment findings the owner picks.
6. When `tauri-plugin-updater` 3 is stable, upgrade and remove the `.cargo/audit.toml` ignores; when Tauri moves to `glib` 0.20, the last alert closes.
7. Code signing once the owner has a certificate.

## Release process (release-kit)

Scribe follows the n8 release standard ([release-kit](https://github.com/n8watkins/release-kit); see `RELEASING.md`).
release-kit owns the version, the tag, `CHANGELOG.md` and the release notes; `.github/workflows/release.yml` still owns the build, signing, smoke installs, `latest.json`, checksums and the upload.

1. Commit with Conventional Commits; `feat`, `fix`, `perf` and `security` subjects become the release notes, so write them for a user.
2. From a clean, up-to-date `main`: `npm run release -- --dry-run` to preview the version and notes.
3. `npm run release -- --push`: runs `release:check` (the desktop frontend's lint, tests and build), bumps every file in `.release.json` (root and app `package.json` and lock, `tauri.conf.json`, `Cargo.toml`, `Cargo.lock`), writes the `CHANGELOG.md` section, commits `chore(release): vX.Y.Z`, tags it, and pushes the branch and tag together.
4. The tag starts the Release workflow (~15 min): `gh run watch <run-id> --exit-status`, then `gh release view vX.Y.Z`.
   It checks the tag against every version file and publishes the notes from the commits, not GitHub's pull request list.

## Historical record (0.5.x and 0.6.0, June 2026; not current)

Everything below is kept for history.
Its version numbers, paths, update cadence, Google Drive sync, local-LLM cleanup and SignPath plans are out of date.

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
> In June 2026 the Project summary and gotchas below were current; the
> 0.5.22-era "State" and "Next steps" sections were already historical.
> For today's state and next steps, see the start-here section at the top.

### Current state - v0.6.0 (shipped 2026-06-21)

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
- **Removed local-LLM dictation cleanup** - duplicated filler suppression and
  added a per-dictation round-trip (the lag). The LLM now powers only **Notes
  analysis + Selection transform** (both on-demand, never in the dictation path).
- **Silent recordings return empty** (no Whisper hallucination):
  `audio::wav_has_speech` gates the full-clip path; fails open.
- **Cheap GPU probe** - cached + async (`spawn_blocking`) + smallest downloaded
  Whisper model (`gpu.rs`).
- **Catalog**: full fp16 `large-v3-turbo`; **English/Multilingual filter** in the
  model browser.

Verified: **216** backend lib tests, `tsc` + frontend build clean, CI Windows
`cargo check --all-targets` green, release build green + published, GPU
benchmarked on the actual 7800 XT.

### Next: Phase 1 - Integrations + managed local LLM

**Read [`docs/UX_REFINEMENT_BACKLOG.md`](UX_REFINEMENT_BACKLOG.md) first** - it's
the phased, checkbox roadmap (Phase 0 is checked off there). Phase 1:

1. **Integrations sidebar** (backlog §A) - new "Integrations" nav section; move
   **Sync (GitHub)** and **Local LLM** under it; enabling an integration opens
   its settings + shows the features it unlocks. Icons: GitHub + LM Studio logos
   + a generic integrations icon.
2. **Scribe-managed local LLM** (backlog §B) - the "inside the app" goal: a
   **`llama-server`** (llama.cpp, same ggml family as whisper.cpp) that Scribe
   spawns/manages itself like `whisper_server.rs`, **GPU-accelerated via the
   Vulkan backend shipped in 0.6.0**, OpenAI-compatible so `note_analysis` /
   `selection_transform` clients don't change; download-on-enable GGUF (e.g.
   Gemma 3 4B) mirroring the Whisper model catalog. **NOTE:** Ollama *and* LM
   Studio are both **external** servers - Ollama already works as BYO
   (`http://localhost:11434/v1`); neither is "inside the app." Only managed
   llama-server is. **Spike first** (prove llama-server runs GPU-accelerated on
   the 7800 XT, mirroring `docs/GPU_VULKAN_SPIKE.md`) before wiring it in.

Phase 2 (per-view UX) and Phase 3 (Transcribe refactor; Transform-selection
expansion) follow - all itemized in the backlog. **History + Notes are deferred.**

### Project summary

Scribe is a free, open-source (**MIT**), Windows-only desktop app for private,
on-device dictation. Stack: **Tauri v2** - Rust backend in `app/src-tauri/src`,
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

### State (this session: 0.5.11 → 0.5.22 - all committed AND pushed)

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
- **0.5.22** **update-check 403 fix** - detection moved off the GitHub REST API to
  the updater's `latest.json` (release CDN).
- post-0.5.22 (`d7e9160`, `1dc0c89`): **MIT** license metadata + a **SignPath
  Foundation** code-signing credit in the README (for the signing application).

Verified: **171 backend tests pass** (`cargo test`); `tsc --noEmit` + `npm run
build` clean. The maintainer is running 0.5.22 (installed via WSL→Windows).

### Key decisions & gotchas (do NOT re-ask, do NOT "fix")

1. **Update polling is intentionally at a 1-minute cadence** (`app/src/App.tsx`,
   the update-check effect: `setInterval(() => runCheck(false), 60 * 1000)`).
   It's a deliberate **test** cadence. **Do NOT change it to the production ~6h
   until the maintainer explicitly says "done testing the updater."**
2. **Update detection = `app/src/lib/updates.ts` `detectUpdate()`**, which calls
   the updater plugin's `check()` (fetches `latest.json` from the release CDN) -
   **not** the GitHub REST API. The old `update_check.rs` / `checkForUpdate`
   command is now unused by the frontend (left in place). Reason: the REST API
   caps at ~60 req/hr unauthenticated, and 1-min polling tripped a **403** that
   silently disabled detection. This is the 0.5.22 fix - don't revert it.
3. **Auto-install is launch-only**, gated on settings actually being loaded; a
   code review confirmed there is **no** mid-session install/close path. See
   `runAutoInstall` / `maybeRunLaunchInstall` in `App.tsx`. Failures fall back to
   manual install and never block the app.
4. **`google_secrets.rs` is gitignored** (real Google OAuth id/secret).
   `build.rs` recreates a placeholder from `google_secrets.example.rs`; CI injects
   the real values from GitHub Actions secrets. **Never commit the real secrets.**
5. **Multilingual models ship without a checksum** (`expected_sha1: None`) -
   upstream no longer publishes a plain-content SHA1 that matches the verifier; a
   fabricated one breaks downloads. Intentional; downloads are HTTPS.
6. **`#[cfg(windows)]` code only compiles on CI's `windows-latest` runner.**
   `cargo test` on Linux skips those branches, so a feature's Windows code is
   first compiled by CI - always `gh run watch <id> --exit-status` after a tag.
7. The **UI reorg is HELD** (the maintainer took it back). The big themes/
   multilingual/auto-update features were built via **parallel worktree agents**.

### Verification & parallel-work commands

- **Backend:** `cd app/src-tauri && cargo test` (171 tests).
- **Frontend:** `cd app && npx tsc --noEmit && npm run build`.
- **Parallel feature work:** `git worktree add -b feat/X ../scribe-wt-X HEAD` then
  `ln -sfn /home/natkins/n8builds/public/scribe/app/node_modules
  ../scribe-wt-X/app/node_modules`; run one agent per worktree; then `git merge`
  sequentially (additive conflicts land in `settings.rs` / `backend.ts` /
  `lib.rs` / `App.tsx` - keep both sides). The Agent tool's `isolation:"worktree"`
  is broken in this environment - use manual worktrees.

### Next steps (ordered)

1. **Code signing via SignPath Foundation (in progress).** Repo is MIT and the
   README credits SignPath. The maintainer is filling the SignPath Foundation OSS
   application. **BLOCKER:** the repo is days old with 0 stars/forks, and SignPath
   requires "widely used or trusted" - so the application will likely be deferred
   until the project has traction. Options: apply anyway (likely "come back
   later"); wait for traction; or **Azure Trusted Signing (~$10/mo, no reputation
   gate)** to sign now. Once approved → add the **SignPath GitHub Action** to
   `.github/workflows/release.yml` (needs a SignPath API token as a GH secret +
   org/project/signing-policy slugs from the maintainer) to Authenticode-sign the
   NSIS `setup.exe`. Acceptance: signed installer, no SmartScreen warning.
2. **Flip updater polling 1-min → ~6h** (the `App.tsx` interval) - ONLY when the
   maintainer says "done testing the updater."
3. **Competitive gaps** (see `COMPETITIVE-ANALYSIS.md`; multilingual is now DONE):
   custom/arbitrary local model selection (point Scribe at any `ggml` `.bin`);
   spoken punctuation / voice commands / voice editing; real-time streaming
   insertion; a first-run onboarding wizard.
4. **UI reorg - HELD.** A ~25-item IA reorg (sidebar reorder, Settings/Audio/
   Themes split, "Backup" rename, History/Notes layout) was captured but pulled.
   Don't start without re-confirming with the maintainer.
5. **Light-theme polish:** move the floating-pill colors into the Themes view.

### File map (current-work pointers)

- `app/src/App.tsx` - update-check effect (poll cadence + launch auto-install),
  theme apply (`data-theme`), custom title bar, nav, `renderView`.
- `app/src/lib/updates.ts` - `detectUpdate()` (latest.json detection; 0.5.22 fix).
- `app/src/components/UpdateOverlay.tsx` - branded auto-update screen.
- `app/src/views/About.tsx` - Updates hub (Install / Check / View releases +
  auto-check & auto-install toggles).
- `app/src/views/Themes.tsx` + `app/src/App.css` - themes (`--scribe-*` vars +
  `[data-theme]` presets; `midnight` is the default = historical look).
- `app/src/views/Settings.tsx` - language picker + Translate→English; dictation
  cleanup; dictionary; notes-LLM analysis.
- `app/src-tauri/src/models.rs` - model catalog (English + multilingual, with a
  `multilingual` flag).
- `app/src-tauri/src/settings.rs` - `AppSettings` (`Language` is now an ISO-code
  string; `theme`, `auto_install_updates`, `auto_update_check_enabled`,
  `translate_to_english`, retention fields, etc.).
- `app/src-tauri/src/dictation.rs` / `whisper.rs` / `whisper_server.rs` -
  transcription pipeline, `--language`/`--translate` args, dictation-cleanup hook,
  selection-transform routing.
- `app/src-tauri/src/selection_transform.rs` - selected-text transform engine.
- `app/src-tauri/src/google_*.rs` / `note_sync.rs` - Drive OAuth / sync / export.
- `.github/workflows/release.yml` - CI signed-release build (where SignPath
  signing will be wired in).
- `docs/COMPETITIVE-ANALYSIS.md` - competitor matrix + prioritized roadmap.
