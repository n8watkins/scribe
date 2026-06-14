# Scribe — Session Handoff

> **⚠️ Historical (superseded).** This handoff captures the **2026-06-12** state
> (just after the LocalDictate→Scribe rebrand and the first Google Drive notes
> sync). Many features described below as "new" or "next" have since shipped —
> Notes + analysis, AI dictation cleanup, selected-text transform, the
> dictionary, history search/combine, export, a signed auto-updater, and more.
> For **current state** read [`CHANGELOG.md`](../CHANGELOG.md) (the live feature
> record, now at **0.5.18**) and the refreshed
> [`docs/STATUS_AND_NEXT_STEPS.md`](STATUS_AND_NEXT_STEPS.md); for a competitive
> review + prioritized gaps see
> [`docs/COMPETITIVE-ANALYSIS.md`](COMPETITIVE-ANALYSIS.md). The notes below are
> kept as a point-in-time record.

Last updated: 2026-06-12 (rebrand LocalDictate→Scribe shipped as the daily app;
Google Drive notes sync Phases 1–3 built + verified).
Read this first, then **`docs/GOOGLE_INTEGRATION_PLAN.md`** (the Drive epic, all
decisions locked), then `docs/STATUS_AND_NEXT_STEPS.md` for older history.

> The app was renamed **LocalDictate → Scribe** — brand, bundle id, GitHub repo
> (`n8watkins/scribe`), and the WSL working directory all updated. Treat
> **Scribe / `com.natkins.scribe`** as the live identity. The only intentional
> `localdictate` leftovers are back-compat: the old data dir/bundle id
> `com.natkins.localdictate` the migration reads, the `LOCALDICTATE_MODEL_DIR`
> env fallback, and the `localdictate-updater.key` filename (validated by the
> embedded pubkey, not its name).

## Project summary

Scribe is a Windows-only Tauri 2 app (Rust backend + React/Vite frontend) for
local push-to-talk dictation via whisper.cpp. Hold `Ctrl+Win` or tap `` ` ``
(acts on release), talk, and text is typed at the cursor. Holding `` ` `` and
tapping `Q` dictates a **note** (blue pill, saved to Notes, never pasted). New
this session: **Google Drive notes sync** — notes push to the user's own Drive
as dated Markdown. The owner (Nathan) uses it daily on Windows 11; he's the only
stakeholder and is usually AT the machine during sessions.

- WSL repo (source of truth): `/home/natkins/n8builds/public/scribe`
- Windows clone (build/test only): `C:\Users\natha\Projects\Tools\localdictate` (Windows dir not renamed)
- GitHub: `https://github.com/n8watkins/scribe` (public)
- **Installed app:** **Scribe** (`com.natkins.scribe`) at
  `C:\Users\natha\AppData\Local\Scribe\app.exe`, data in
  `%APPDATA%\com.natkins.scribe\`. Now version **0.3.0**, the owner's daily tool.
  - **LocalDictate is uninstalled.** Its old data dir
    `%APPDATA%\com.natkins.localdictate\` is intentionally kept as a migration
    backup (the rename migration reads it).
  - **Scribe Dev** (`com.natkins.scribe.dev`) — the agent's `--no-bundle` test
    build at the clone's `target/release/app.exe`; own data dir/keychain.

## This session (2026-06-12)

Two big epics, both shipped:

**1. Rebrand LocalDictate → Scribe** (`f34887b`, refined later). Display name +
bundle identifier `com.natkins.localdictate` → `com.natkins.scribe` (dev →
`.scribe.dev`). First-run **data migration** in `lib.rs::migrate_pre_rebrand_data`
copies the old DB/clips/models across (hardened `3e42c48`: atomic DB copy via
temp+rename, and a `.rebrand-migrated` completion marker so an interrupted
migration resumes instead of stranding data). DB file is now `scribe.sqlite3`.
**Verified live:** the owner's full history (873 transcripts, 5 notes) migrated
intact, app boots clean, hotkeys register 4/4.

**2. Google Drive notes sync — Phases 1–3** (see `GOOGLE_INTEGRATION_PLAN.md`):
- **Phase 1** (`ef6e279` backend, `43cc891` frontend): desktop OAuth 2.0
  (loopback + PKCE, scope `drive.file openid email`), refresh token in the OS
  keychain (keyring), ensure "Scribe Voice Notes" folder, write daily
  `YYYY-MM/YYYY-MM-DD.md`. **Verified END-TO-END live** — a real note synced to
  the owner's Drive (folder/file confirmed via the Drive API).
- **Phase 2** (`f81a6cb`): auto-sync each note to Drive on save (debounced
  background `DriveSyncWorker`), gated by the "Sync notes to Google Drive"
  toggle. Code verified (109 tests); **owner hasn't watched it fire live yet**.
- **Phase 3** (`de44cec`): end-of-day `DriveOrganizeScheduler` runs the local
  notes-analysis LLM over the previous day's notes → `YYYY-MM-DD-organized.md`.
  Opt-in via "End-of-day auto-organize" toggle + `drive_organize_hour`. **Never
  run live yet.**

Plus: gitignored OAuth secret scaffolding (`10c6659`), Settings tabs
(`5d69cd9`), token-based signed-in detection + email scope (`220c355`), clean
notes-only daily log (`6bce6ad`), Notes-view "Sync to Drive" button (`ca4000c`),
pause-ellipsis stripping in transcripts (`0de5b72`), title-bar dedup (`697ab93`).

**Google Cloud:** OAuth Desktop client created (id/secret in the gitignored
`google_secrets.rs`); Drive API enabled; consent screen the owner reported
**published to Production** (so refresh tokens don't expire weekly — Testing
mode expires them in 7 days). Owner is signed in on production Scribe
(`drive_account_email = nathancwatkins23@gmail.com`, token in keychain).

## State

All work is **local commits on `main`, NOT pushed** (origin is behind; push only
when the owner asks). Session commits, newest first:

| Commit | What |
|---|---|
| `697ab93` | Blank native title bar on stable (no duplicate "Scribe"; dev keeps "Scribe Dev") |
| `de44cec` | **Phase 3**: end-of-day local-LLM auto-organize (scheduler + `drive_organize_now`) |
| `f81a6cb` | **Phase 2**: auto-sync notes to Drive on save (`DriveSyncWorker`, debounced) |
| `3e42c48` | Harden rebrand migration (atomic DB copy + `.rebrand-migrated` marker) + cleanups (pkg 0.3.0) |
| `9093b80` | Bump version to 0.3.0 |
| `ca4000c` | Notes view "Sync to Drive" button |
| `6bce6ad` | Clean notes-only daily Drive log (friendly date, 12h time, no empty summaries) |
| `0de5b72` | Strip pause ellipses (`...`/`…`) from transcripts |
| `220c355` | Signed-in = stored token (not email); add `openid email` scope + OIDC userinfo |
| `5d69cd9` | Settings: tabbed sections |
| `10c6659` | Load OAuth client id/secret from gitignored `google_secrets.rs` (build.rs recreates from example) |
| `f34887b` | **Rebrand LocalDictate → Scribe** (identifier + data migration) |
| `43cc891` / `ef6e279` | Google Drive sync Phase 1 frontend / backend |

Verified: Windows `cargo test` **109 passed** at each Rust change; frontend
`tsc + npm run build` clean; rebrand migration + Drive Phase-1 sync confirmed on
the owner's real machine/Drive.

## Next steps (priority order)

1. **Confirm auto-sync + auto-organize LIVE** (needs the owner; agent verifies):
   - Owner: Settings → Google Drive → turn ON **"Sync notes to Google Drive"**
     (currently OFF on production), dictate a `` ` ``+Q note, wait ~3 s.
   - Agent: confirm the note auto-appears in Drive (`Scribe Voice Notes/…`).
   - For Phase 3: turn on "End-of-day auto-organize" + "notes analysis" (LM
     Studio running), click **"Organize today now"**, confirm `…-organized.md`.
2. **Drive cleanup** (owner, manual): delete the leftover **`2026-06-12.md`**
   test file from `Scribe Voice Notes/2026-06/` (it has a seeded test note from
   the dev flavor). The agent's Drive access is read-only (search/read only).
3. **Phase 2 optimization — per-day file-id caching.** `drive_sync_now` /
   auto-sync currently re-lists folders and rebuilds ALL daily files from the DB
   each run (O(all history); idempotent but wasteful at scale). Cache
   `{day → drive_file_id}` (e.g. a small table/JSON) so a sync touches only the
   affected day. See `GOOGLE_INTEGRATION_PLAN.md` Phase 2.
4. **Phase 4** — weekly `YYYY-Www-summary.md`; and wire the currently-dead
   `drive_sync_all_transcripts` setting (a reserved stub) into a SEPARATE
   `Transcripts/` Drive backup (the daily notes file is notes-only by design).
5. **Tech debt:** de-dup the identical percent-encode helper in
   `google_oauth.rs` + `google_drive.rs`; decide whether distributed/CI builds
   should ship real OAuth creds (CI has no secret-injection step, so a CI build
   reports "not configured" and Drive sync is off — fine for owner-only).
6. Earlier backlog still open: first **GitHub release** (push + tag `v0.3.0` so
   the updater chain starts — note a LocalDictate→Scribe jump is a fresh install,
   not an auto-update); Gemini BYO-key analysis (PINNED, see plan); code signing.

## Conventions & gotchas (hard-won — do not relearn)

- **Windows verification is mandatory** for Rust (most code is `#[cfg(windows)]`).
  Sync the clone, then test:
  `cd /mnt/c/Users/natha/Projects/Tools/localdictate && git checkout -- . && git fetch /home/natkins/n8builds/public/scribe main && git merge --ff-only FETCH_HEAD`
  then `cd /mnt/c && cmd.exe /c "cd /d C:\Users\natha\Projects\Tools\localdictate\app\src-tauri && cargo test 2>&1"`.
  (The clone shows CRLF drift on `Cargo.toml`/`Cargo.lock` — `git checkout -- .`
  first.) Frontend (builds on Linux): `npx tsc --noEmit -p tsconfig.json && npm run build` in `app/`.
- **The gitignored OAuth secret won't sync via git.** `app/src-tauri/src/google_secrets.rs`
  (CLIENT_ID/CLIENT_SECRET) is gitignored; `build.rs` recreates a *placeholder*
  copy from `google_secrets.example.rs` on a fresh clone (Drive sync then reports
  "not configured"). To build a working Scribe in the clone you MUST
  `cp` the real `google_secrets.rs` from the WSL repo into the clone after each
  `git` sync. The real file lives only in the WSL working tree.
- **Bundle builds (`npx tauri build`) fail with `os error 32` (file in use) if a
  clone-path `app.exe` OR its child `whisper-server.exe` is running.** Before any
  bundle build, kill clone-path procs:
  `powershell.exe -NoProfile -Command "Get-Process app,whisper-server,whisper-cli -EA SilentlyContinue | ? { $_.Path -like '*Projects*' } | Stop-Process -Force"`.
  Force-killing a Scribe instance orphans its `whisper-server` child — kill that too.
- **Signed bundle build** (needs the updater key + password; `createUpdaterArtifacts`
  makes every bundling build require them):
  `export TAURI_SIGNING_PRIVATE_KEY='C:\Users\natha\.tauri\localdictate-updater.key' TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat ~/.tauri/localdictate-updater.password)" WSLENV=TAURI_SIGNING_PRIVATE_KEY/w:TAURI_SIGNING_PRIVATE_KEY_PASSWORD/w`
  then `cmd.exe /c "cd /d C:\...\app && npx tauri build"`. Key filename stays
  `localdictate-updater.key` (validated by embedded pubkey, not name). Dev-flavor
  builds are `--no-bundle` and need no key.
- **Install / upgrade Scribe** (silent, currentUser NSIS): close Scribe + its
  whisper-server, then `Start-Process <Scribe_0.3.0_x64-setup.exe> -ArgumentList '/S' -Wait`,
  relaunch `%LOCALAPPDATA%\Scribe\app.exe`, verify the log
  (`%LOCALAPPDATA%\com.natkins.scribe\logs\Scribe.log`): "Settings loaded",
  "Registered 4 of 4 hotkey bindings", "Scribe setup complete". Same identifier =
  in-place upgrade, data preserved.
- **Owner is AT the machine**: no input injection, no focus stealing, no audio.
  Verify via the log + direct SQLite reads (backend re-reads settings per
  recording). **Synthetic keypresses are invisible** to the hotkey watcher, so
  hotkey/dictation behavior can ONLY be verified by the owner's physical keys.
  Auto-sync/auto-organize end-to-end therefore need the owner to dictate.
- **Two Scribe instances fight over hotkeys.** A running Scribe Dev shadow-
  captures the owner's `` ` `` (its GetAsyncKeyState watcher) and the one that
  registered `Backquote` first owns suppression (so the `` ` `` doesn't type).
  Launch Dev only for a check and kill it right after. Daily single-instance use
  suppresses the tilde correctly — that's expected.
- **Drive/OAuth:** scope is `drive.file openid email`; refresh token in the OS
  keychain keyed by `app.config().identifier` (dev/stable isolated). The
  `drive.file` scope alone does NOT return the email — that's why the email scope
  + OIDC userinfo were added, and why "signed in" is based on the stored token,
  not the email. `reqwest` is `default-features=false` (no `.json()` — use
  `.text()` + serde_json). New settings need `#[serde(default)]` only.
- **The Drive `q` injection note:** folder/file names are escaped
  (`escape_query_literal`) + percent-encoded; security review found no
  Critical/High/Medium issues in the OAuth/Drive/secret code.
- Commit per logical change with `Co-Authored-By: Claude ...`; push only when the
  owner asks. Owner dictates stream-of-consciousness; pick the sensible reading.

## File map (for the next steps)

- `app/src-tauri/src/google_oauth.rs` — PKCE loopback flow, token refresh, keychain
- `app/src-tauri/src/google_drive.rs` — Drive REST: folders/files, `sync_notes`, `write_organized`, `render_daily`, mock-server tests
- `app/src-tauri/src/google_secrets.rs` — gitignored real OAuth id/secret (`.example.rs` is the committed template; `build.rs` recreates)
- `app/src-tauri/src/note_sync.rs` — `collect_and_sync`, `DriveSyncWorker` (auto-sync), `organize_day`, `DriveOrganizeScheduler`
- `app/src-tauri/src/note_analysis.rs` — local-LLM client reused by the organize pass
- `app/src-tauri/src/commands.rs` — `google_status/sign_in/sign_out`, `drive_sync_now`, `drive_organize_now`
- `app/src-tauri/src/lib.rs` — `migrate_pre_rebrand_data`, worker/scheduler spawn, command registration, title styling
- `app/src-tauri/src/settings.rs` — AppSettings incl. all `drive*` fields
- `app/src-tauri/src/dictation.rs` — note-save path; the auto-sync `worker.notify()` hook (~line 215)
- `app/src/App.tsx` — `GoogleDrivePanel` (Drive settings), Notes view + "Sync to Drive" button, Settings tabs
- `app/src/backend.ts` — TS command wrappers/types
- `docs/GOOGLE_INTEGRATION_PLAN.md` — the locked Drive plan (Phases, decisions)
