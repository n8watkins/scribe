# Scribe: full assessment (plan for a fresh session)

Written 2026-10-27.
Start a new Claude Code session in `~/projects/tools/scribe/scribe-app`, run `git fetch && git switch main && git pull`, and read this file there (`docs/ASSESSMENT_PLAN_2026-10.md`).

## Why

Scribe was built months ago with an older model.
It is a full desktop app now (about 21,000 lines of Rust in 35 modules, about 11,000 lines of React/TypeScript, 263 Rust tests).
The owner wants a top-to-bottom assessment with a newer model, to take it to the next level: code, security, UI, and above all UX.
The assessment comes first; changes come only after the owner has read the report and picked what to do.

## Read first

1. This file.
2. `README.md`, `SECURITY.md`, `docs/HANDOFF.md`, `docs/STATUS_AND_NEXT_STEPS.md`.
3. Earlier UX thinking, so the assessment builds on it instead of repeating it: `docs/UX_REFINEMENT_BACKLOG.md` (2026-06-21: an Integrations view for GitHub and LM Studio, and more), `docs/FRONTEND_UX_REFINEMENT_PLAN.md`, `docs/COMPETITIVE-ANALYSIS.md`, `docs/PRD.md`.
4. The landing page's own notes: `~/projects/tools/scribe/scribe-web/docs/scribe-review-and-videos.md`.

## State of the repo today (check, don't assume)

- Checked out on `feat/rust-storage-profiles` (last commit `80b3cff`), not `main`; a worktree `validation/scribe-rust-storage-windows` exists.
  Find out what those branches are and whether they're merged before reviewing "the code".
- Dependabot: of the five PRs open when this plan was written (#20, #32, #37, #38, #39, the oldest from July), only #38 is still open; #40 superseded the rest on 2026-10-27.
- 15 commits sit on `main` since v0.8.0, including the security fixes from #40 (the owner hasn't said yet whether to cut a release).
- CI: `.github/workflows/ci.yml`, `release.yml`, `gpu-spike.yml`; builds run on `windows-latest` because the app is Windows-only.
- The installed app is at `C:\Users\natha\AppData\Local\Scribe\Scribe.exe` (there is also a "Scribe Dev" data folder).

## What the owner asked for

In their words, condensed:

- A full analysis of Scribe's actual code, by an agent dedicated to it.
- A security analysis.
- An overview: how it's built, what's solid, what's fragile.
- A UI analysis and, more importantly, a UX analysis: do the settings and their locations make sense?
- Screenshots of every screen.
- The window has a fixed size where a lot of scrolling is needed, in Settings especially; is that the right experience?
- Some things are awkward to use; Replacements is the example given. Find the rest of those.
- "Let me know what we can do to improve this."

## How to run it

Use dedicated agents in parallel, one per lane, each read-only, each writing its findings to its own file under `docs/assessment-2026-10/` on a new branch `docs/assessment-2026-10`.
Then merge them into one report.
At most 4 heavy jobs at once (the machine's limit for builds and tests).

### Lane 1: architecture and code quality

- Map the modules (`app/src-tauri/src/*.rs`, `app/src/views`, `app/src/components`, `cli/`, `integrations/mcp-server/`) and how they talk (Tauri commands, events, the local state server).
- Look for: dead code and half-built features (`recording_mode` is inert; the Developer panel is hidden), duplicated logic, oversized modules, error handling that swallows failures, blocking work on the UI thread, threads and locks in dictation and hotkeys, and settings migrations.
- Run the checks and record the results: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `npm run lint`, `npm test`, `npx tsc --noEmit`.
  Builds of the Windows app happen in CI; don't try to build the Tauri bundle in WSL.
- Test coverage: which areas have none (the front end has only 2 test files).

### Lane 2: security

- Tauri: capabilities and permissions (`src-tauri/capabilities`), CSP in `tauri.conf.json`, which commands the front end can call, and whether any accept paths or URLs from the page without checks.
- The local HTTP state server (`state_server.rs`): bind address, auth, what it exposes, and whether a web page in a browser could reach it (DNS rebinding, CORS).
- GitHub OAuth and the backup (`github_oauth.rs`, `github_backup.rs`, `note_sync.rs`): token storage (Windows Credential Manager or a plain file?), scopes, the private-repo check, and what is uploaded.
- AI calls (`note_analysis.rs`, `selection_transform.rs`): the endpoint is user-set; check for SSRF-style surprises, timeouts, and what text is sent.
  Note that Ctrl+Alt+R (transform selection) is bound even when "Analyze notes" is off.
- Model downloads (`model_manager.rs`): sizes are checked; are hashes? HTTPS only? Partial files cleaned up?
- Import (`import.rs`, SHA-256 checked) and export (`export.rs`): path handling.
- The updater: signature verification, the `latest.json` source, and downgrade behavior.
- File transcription (`file_transcribe.rs`) and the ffmpeg call: argument injection.
- Clipboard handling (it saves and restores the user's clipboard, including images and files): anything kept longer than needed?
- Dependencies: `cargo audit`, `npm audit`, and the open Dependabot PRs.
- Privacy claims: compare what the app does with what the landing page and README say.
  Known already: audio clips are kept by default ("Save raw audio clips" is on; clips are deleted with their transcript after 30 days), so "the recording is deleted" was wrong on the landing page.

### Lane 3: UI and UX, with screenshots

- Screenshots, two ways:
  1. The real app on Windows: launch the installed `Scribe.exe` and capture each view from WSL with `powershell.exe` (a small script using `System.Windows.Forms`/`System.Drawing` to grab the Scribe window).
     Ask the owner before driving the app if anything would change their real settings or data; prefer the "Scribe Dev" profile if it is separate.
  2. Reproducible captures of the React UI in a browser: run `npm run dev` in `app/` and mock the Tauri IPC (`@tauri-apps/api/mocks`, `mockIPC`) in a Playwright script with realistic data, so every view can be shot at several window sizes (the default size, smaller, larger, and the light theme).
- Every view: Dashboard, History, Transcribe, Models, Hotkeys, Audio, Settings, Themes, Data & Privacy, Sync, Stats, About, Developer.
  Plus the floating pill in its three modes, the first run, the update screen, the tray menu, and error states (no mic, no model, AI server down, GitHub disconnected).
- Questions to answer:
  - Information architecture: is each setting where a user would look for it?
    The June backlog proposed an Integrations view (GitHub and LM Studio); say whether that's still right.
  - Window size and scrolling: what the default window shows above the fold in each view; whether the layout should change (wider two-column settings, sections, a search box for settings) or the default size should.
  - Replacements, the dictionary, hotkey rebinding, model choice, and retention: walk through each as a new user and note every bit of friction.
  - First run: how long from install to the first dictation, and what can go wrong.
  - Consistency: spacing, type, colors across themes, icons, empty states, wording (plain, no jargon, no em dashes).
  - Accessibility: keyboard navigation, focus, contrast in every theme, screen-reader labels.
- Compare with the best of the category (Wispr Flow, SuperWhisper, MacWhisper, Windows' own voice typing) using `docs/COMPETITIVE-ANALYSIS.md` as the start.

### Lane 4: features, performance and reliability

- Start from the feature inventory below and confirm each item works as described.
- Measure what can be measured from the code and logs: model load time, key-release-to-text latency (the app records it in Stats), memory, installer size (13 MB), and startup time.
- Reliability: what happens on sleep/wake, mic unplugged, a model file deleted, the AI server down, GitHub rate limits, and a crash mid-dictation (the last 20 failed recordings are kept for retry).

### Lane 5: release notes

- Today the app's `CHANGELOG.md` stops at 0.6.0 while releases go up to v0.8.0, and the GitHub release notes are auto-generated lists of PR titles (written for developers, not users).
- The website (scribe-web) has no release notes at all, and the two aren't connected; n8builds.dev's build log reads Scribe's GitHub releases and commits.
- Recommend one setup, for example the owner's own release-kit (`~/n8builds/release-kit`, already used by n8builds-web): user-facing notes per release, a generated changelog (don't hand-edit `CHANGELOG.md` once it's generated), and the latest notes shown on scribe.n8builds.dev and in the app's About screen.

## For the website (do during the assessment)

The landing page needs real images of the app, which can only be taken on Windows:

- every theme (Midnight, Ocean, Slate, Emerald, Violet, Pewter, a Custom one), same screen each time;
- the Models screen, the pill in its three modes, History, Notes with an AI summary, Settings, and the transform-selection flow;
- at 2x, saved under `docs/assessment-2026-10/screens/` and offered to scribe-web's `public/`.

## Deliverable

- One report as a hosted Artifact page (the owner prefers long reports as hosted HTML), with a short summary in the terminal and the link.
- Sections: summary; top 10 improvements ranked by value; security findings by severity; UX findings with screenshots; code health; performance and reliability; a proposed roadmap (quick wins, then bigger changes).
- Each finding: what, where (file and line or screen), why it matters, the fix, and the effort.
- Also commit the per-lane files on `docs/assessment-2026-10` and open a PR so they're kept.
- Then stop and let the owner choose; don't start fixing until they do.

## Working rules

- Conventional Commits, no Co-Authored-By trailer, no em dashes anywhere, one sentence per line in long Markdown.
- Branch and PR per change.
- For taste or sweeping UI changes later: show a desktop preview, then mobile or smaller sizes, and get approval before polishing.

## Appendix: what Scribe does today (from the source, 2026-10-27)

Defaults in brackets.

- **Dictation:** hold Ctrl+Win to talk; tap the backquote key (`~`) for hands-free; tap Q while dictating to save a quick note instead of pasting; Ctrl+Alt+X discards; Ctrl+Alt+V pastes the last one again; Ctrl+Alt+F opens the dashboard.
  Every hotkey can be rebound, on press or on release, with conflicts flagged.
- **Live transcription** while you talk, with split-on-pause settings [on]; auto-stop after 60 s of silence for hands-free [on]; silence trim [on]; recording length 0.3 s to 10 min.
- **Output:** auto-paste [default], copy, copy and paste, or save only; clipboard paste that restores the clipboard exactly [default] or type-it-out for apps that block paste.
- **Floating pill:** dot, waveform, or waveform with text [default]; draggable; colours can be changed; start and stop sounds [on].
- **Cleaner text:** a dictionary hint for names and jargon; "say X, get Y" replacements; filler removal for pause-bracketed "um" and "uh" [off].
- **Transform selection:** highlight text anywhere, Ctrl+Alt+R, say an instruction, and a local AI rewrites it in place.
- **Models:** 11 Whisper models from 78 MB to 1.6 GB (English Tiny, Base, Small, Small Q5_1 [default], Medium; multilingual Tiny, Base, Small, Medium; Large v3 Turbo Q5_0 and full), from Hugging Face, size-checked; kept loaded for speed; GPU (Vulkan) with CPU fallback and a GPU picker [auto].
- **Languages:** auto-detect plus 29 in the picker; translate any language to English [off; needs a multilingual model].
- **Transcribe a file:** WAV, MP3, FLAC, OGG; MP4, MKV, M4A and more with ffmpeg; save as text.
- **Audio:** microphone picker, record and play a test; the last 20 failed recordings kept for retry; audio clips saved for playback [on].
- **History:** search, date filters, sort, expand, copy, open in an editor, play, delete, combine several; kept 30 days [default], or forever, or off.
- **Notes:** their own view, kept forever [default]; AI summary and action items with an editable prompt via LM Studio, Ollama or any OpenAI-compatible server [off].
- **Stats:** words and dictations today, average WPM, latency, recording length, most-used model, total words.
- **Backup and export:** GitHub backup of notes (and optionally all transcripts) to a private repo, checked private, with sync history [off]; export to Markdown, CSV or JSON; restore from a JSON export, SHA-256 checked.
- **Look and feel:** seven themes (Midnight [default], Ocean, Slate, Emerald, Violet, Pewter light, Custom from three colours); default window size setting.
- **App:** lives in the tray with a menu; launch at startup [off]; notifications [on]; updates checked at launch, on refocus and every 6 hours, installed signed at launch only, both switchable in About.
- **For developers:** a local HTTP state server, a `scribe` CLI, and an MCP server reading dictation state; a hidden Developer panel.
- **Half-built or hidden:** `recording_mode` setting is inert; no custom model files yet; the Developer panel is opt-in; Google integration is a plan only (`docs/GOOGLE_INTEGRATION_PLAN.md`), although the 30-second promo video claims a Google Drive backup.
