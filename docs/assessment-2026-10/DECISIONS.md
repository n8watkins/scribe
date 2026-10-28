# Owner decisions on the assessment (2026-10-28)

The owner accepted the assessment's suggestions overall and decided the following.

## Decided

- **Releases:** ship in steps, one version per batch of changes, starting with the Phase 0 fixes.
  release-kit picks the number from the commits (`feat` bumps the minor version, `fix` the patch), so each phase gets its own version.
- **Hands-free key:** change the default for new installs only.
  Anyone who already has a binding saved keeps it until they change it; no migration rewrites existing hotkeys.
- **Audio clips:** keep "Save raw audio clips" on by default, delete clips automatically over time, and keep the switch to turn it off.
- **Navigation regroup:** preview the new menu layout (desktop, then the smaller window) before building it.
- **Code signing:** no paid signing.
  Drop Azure Trusted Signing from the roadmap; use the free trust measures instead (published checksums, a clear SmartScreen note on the download page, and submitting each installer to Microsoft for analysis).
- **`feat/rust-storage-profiles`:** don't merge the branch as it is.
  Move its useful parts into a fresh PR on `main`: `crate-type = ["rlib"]`, the smaller debug-info profiles, and the sturdier NSIS uninstall step in the smoke test.
  Leave out its ESLint 10 bump, which would conflict with the lockfile #40 refreshed and belongs in its own PR.
  Then delete the branch, and remove the Windows validation worktree once the owner confirms.

## Found while answering

- The old LocalDictate folders (`%APPDATA%\com.natkins.localdictate` and `.dev`, about 434 MB) still hold 873 transcripts from 2026-06-11 to 06-13 and their clips.
  The rename migration copied them into Scribe and never removed the originals.
  Together with the 505 MB of orphaned clips and about 220 MB of duplicate models, this is about 1.2 GB of stale data on the owner's machine.
- The real install is behaving as designed: all of its rows are from the last 30 days except 5 notes from June, which are kept because notes are kept forever.
- The Dev install (v0.5.23) holds 900 of its own test dictations from June and 1.4 GB on disk (598 MB of clips, 763 MB of models).
  It does not copy the real install's transcripts (one shared text out of 900).
  Retention never removed them because the Dev app has not run since June, and retention only runs while the app is running.

## Still open

- Whether a note's audio should expire after 30 days like dictations, while the note's text stays.
- Permission to clean up the stale data above on the owner's machine, and to reset the Dev profile for the real-app screenshots.
