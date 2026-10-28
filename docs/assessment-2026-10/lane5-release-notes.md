# Lane 5: release notes

Assessed 2026-10-28 on `main` at `1d88fe9`.

## Where things stand

- release-kit 2.0.0 was adopted in #42, so most of what the plan asked for is already in place.
  It owns the version, the tag, `CHANGELOG.md` and the release notes, and `.github/workflows/release.yml:265-276` builds the GitHub release body from Conventional Commits with `release-kit notes`.
- `CHANGELOG.md` now runs to 0.8.0, written for users ("New", "Fixed", "Safer"); the plan's note that it stops at 0.6.0 is out of date.
- The v0.8.0 GitHub release body is still GitHub's auto-generated list of pull request titles ("Normalize NSIS smoke-test install paths"), written for developers.
  Every release from the next one on will use release-kit notes.
- The app shows no notes.
  About has a version and a "View releases" link to GitHub (`app/src/views/About.tsx:34`, `:282`), and the update overlay shows no notes either.
- The website (scribe-web) has no changelog or "What's new" page.

## Recommendations

1. **Backfill the v0.8.0 release body** with the 0.8.0 section of `CHANGELOG.md` (`gh release edit v0.8.0 --notes-file`), so the release page and the n8builds.dev build log read the same as the changelog.
   Effort: XS.
2. **Show the notes in the app.**
   The updater's `check()` result carries the release body as `update.body`; show it in the update overlay before installing, and once after an update as "What's new in X.Y" (a dismissible card on Home, remembered by version in settings).
   About gets a "Release notes" section that renders the current version's notes.
   Effort: S.
3. **A changelog page on scribe.n8builds.dev** that reads the GitHub releases at build time (the same source the n8builds.dev build log uses), with a "What's new" link in the site header.
   Effort: S.
4. **Write `feat` and `fix` subjects for users**, as `AGENTS.md` and `RELEASING.md` already require; that is the only ongoing cost of keeping all three surfaces good.
