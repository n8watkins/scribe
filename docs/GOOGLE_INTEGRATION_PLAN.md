# Scribe - Google Drive Sync Plan (superseded)

> **Superseded on 2026-06-21.**
> Scribe now uses optional private GitHub repository backup instead of Google Drive.
> This file preserves the rejected Drive design and its historical product decisions; it is not an implementation plan or a description of current behavior.
> Current backup behavior is documented in the [root README](../README.md), and current GitHub App build configuration is documented in the [app README](../app/README.md).

Status: SUPERSEDED (not to be implemented).
Written 2026-06-12 and refined the same day with owner decisions.

## Scope right now

**BUILD: Google Drive notes sync.** **PIN: Gemini cloud analysis** — the owner
explicitly does not want to work on the Gemini BYO-key provider right now (see
"Pinned" at the bottom; it's still a good future feature). The end-of-day
"organize my notes" LLM pass uses the **existing LOCAL LLM** (the notes-analysis
provider already shipped), NOT Gemini.

## Goal / product intent

Immediately push every dictated note to Google Drive, organized by date, so the
owner can reference them from his other tools. Then, at end of day / end of
week, have the local LLM reorganize and summarize. "Awesome free tool" — runs on
the user's own Google account, text only, comfortably inside the free 15 GB.

## Storage findings (measured 2026-06-12, stable DB)

In ~1 day of heavy use: **477 transcripts**, 5 notes.
- Transcript **text**: 116 KB/day → ~**40 MB/year**. Trivial vs 15 GB free.
- Audio **clips**: 267 MB/day → ~**97 GB/year** → blows the free tier in ~8
  weeks. **DECISION: never sync audio to Drive. Text only.** Audio stays local.
- So syncing even ALL transcripts (not just notes) as text is free-tier-safe.

## Drive layout (owner-decided)

```
Scribe Voice Notes/            (root app folder, scope drive.file)
  2026-06/                           (one folder per MONTH, named YYYY-MM)
    2026-06-12.md                    (daily log — appended live on each save)
    2026-06-12-organized.md          (end-of-day LLM-reorganized version)
    2026-W24-summary.md              (weekly summary of the week's dailies)
```

- **Folder per month** (`YYYY-MM`) so we never have one giant flat folder.
- **Daily file** `YYYY-MM-DD.md`: appended to on **every note save** (auto-sync).
  Each entry holds the **raw transcript AND its summary** + a timestamp.
- **Two versions per day** (owner wants original + summary, and an organized
  view):
  - `YYYY-MM-DD.md` — chronological live log (raw + per-note summary).
  - `YYYY-MM-DD-organized.md` — produced/updated by the **end-of-day** local-LLM
    pass: arbitrary categorization, category titles, restructured to "make
    sense." This is a fresh read of the day's notes → reorganized file.
- **Weekly** `YYYY-Www-summary.md`: once a week, local LLM summarizes the week's
  organized dailies into one file.
- Endless files are fine (text is tiny); the month folders keep it tidy.

## Sync behavior (owner-decided)

- **Auto-sync on every save** (no manual-only mode needed). At ~477/day we are
  nowhere near any Drive rate limit; this is plain file appends.
- **Sync notes** to the dated files. SEPARATELY, optionally, a **full-transcript
  text backup** is cheap (~40 MB/yr) — offer it as a toggle ("back up all
  transcripts, not just notes"), default off. Still text-only, never audio.
- Append = read-modify-write the daily file (or cache its Drive file id and
  fetch-append-update). Keep the file id per day to avoid re-listing.

## End-of-day / weekly scheduling (needs design)

The reorganize + weekly passes must fire "at a certain point in the day." This
ties to the footprint discussion:
- The **local LLM must be reachable** when the pass runs. Recommend the LM Studio
  **headless service** (Settings → Developer → "Enable Local LLM Service") so the
  server is up without the GUI; JIT-load + ~5 min TTL means the model only
  occupies VRAM during the pass, then unloads.
- A scheduler inside Scribe (it's always-on in the tray): e.g. a daily
  timer at a configurable hour → run organize for that day; a weekly timer →
  weekly summary. Guard against firing mid-game (configurable hour, e.g. 3am, or
  "only when idle"). If the local server is down, skip + retry next tick.

## OAuth — how a desktop app does this

The owner asked "how do we even do OAuth for a local app?" Answer:
- **Desktop OAuth 2.0 with loopback redirect + PKCE.** Register an OAuth client
  of type "Desktop app" in Google Cloud Console under the project. Ship its
  **client ID** with the app (desktop client secrets are not really secret;
  PKCE covers it).
- Flow: app opens the system browser to Google's consent page → user signs in →
  Google redirects to `http://127.0.0.1:<random port>` which the app is
  listening on → app catches the auth code → exchanges for access + refresh
  tokens.
- **Scope: `drive.file` ONLY** (app-created files). Least privilege AND it keeps
  us out of Google's restricted-scope security assessment.
- Until the OAuth consent screen is verified by Google, users see an
  "unverified app" warning (fine for the owner + early users; verify the brand
  later for a clean experience when distributing widely).
- **Store the refresh token in the OS keychain** (Windows Credential Manager via
  the `keyring` crate), NOT plaintext SQLite.

## Implementation sketch

Backend (`app/src-tauri/src/`):
- `google_oauth.rs` — PKCE loopback flow, token refresh, keychain storage.
- `google_drive.rs` — folder ensure/create, file create/append/update, the
  daily/organized/weekly writers.
- `note_sync.rs` (or extend existing) — on note save, format + enqueue upload.
- Scheduler — daily/weekly timers (reuse the always-on app process).
- `lib.rs` — register commands; wire note-save hook to the uploader.
- Settings: `driveSyncEnabled`, `driveSyncAllTranscripts` (default off),
  `driveOrganizeHour`, signed-in email (display only). Secrets → keychain.

Frontend (`app/src/`):
- New **Integrations** tab in Settings: Sign in with Google / signed-in-as /
  sign out; enable sync; "also back up all transcripts" toggle; organize hour;
  "Sync now / backfill" button; last-sync status.
- `backend.ts` — wrappers/types.

Reuse note: the existing local notes-analysis path
(`note_analysis.rs::analyze_text`) is what powers per-note summaries and the
end-of-day organize pass — no new LLM provider needed.

## Phasing

1. **OAuth + Drive plumbing**: sign-in, ensure folder, write today's daily file
   on note save (raw + summary). Manual "sync now" first to prove it.
2. **Auto-sync on save** + month-folder organization + per-day file ids.
3. **End-of-day organize pass** (local LLM → `-organized.md`) + scheduler.
4. **Weekly summary** + optional full-transcript backup toggle.

## Decisions locked (owner)

- Drive layout: month folders (`YYYY-MM`), daily files, daily `-organized`
  version, weekly summary. Both raw + summary stored.
- Auto-sync on every save.
- Sync notes; offer optional all-transcripts text backup (default off).
- **Text only — NEVER audio** (would be ~97 GB/yr; text is ~40 MB/yr).
- Ship an OAuth Desktop client ID with the app; scope `drive.file`; refresh
  token in OS keychain.

## PINNED for later — Gemini BYO-key analysis

Not now, per owner. Future: add `notesAnalysisProvider` = local | gemini;
user pastes their own free AI Studio key (no reselling); model picker via
`GET /v1beta/models`; track `usageMetadata` tokens; key in keychain. Self-
contained, no OAuth — good standalone phase when revisited.

## Cross-cutting reminders (from HANDOFF.md)

- Windows verification mandatory for Rust; test on the **Dev flavor** first,
  ship to stable only on owner request.
- New settings need `#[serde(default)]`; changed shipped defaults go through
  `CURRENT_DEFAULTS_VERSION` + `migrate_defaults`.
- reqwest is `default-features = false`: no `.json()`, use `.text()` +
  serde_json.
- Commit per logical change with the Co-Authored-By trailer.
