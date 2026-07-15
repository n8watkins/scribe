# Scribe

Scribe is a private, local-first Windows dictation app built with Tauri, React, TypeScript, Rust, SQLite, and whisper.cpp.
Beyond push-to-talk transcription it adds quick notes, optional local-LLM note analysis and selected-text transform, a dictionary, searchable history with export, optional private GitHub backups, and an integrity-checked auto-updater.

This is the app subproject. For the full user-facing overview and install/build instructions, see the [root README](../README.md).

Reference docs:

- [Full feature list & changelog](../CHANGELOG.md)
- [Competitive analysis & gap review](../docs/COMPETITIVE-ANALYSIS.md)
- [Product requirements (original)](../docs/PRD.md)
- [Implementation plan (original)](../docs/IMPLEMENTATION_PLAN.md)

## Development

Install dependencies:

```bash
npm install
```

Run the frontend:

```bash
npm run dev
```

Run the Tauri desktop app:

```bash
npm run tauri dev
```

Rust and the Tauri OS prerequisites are required for desktop builds.

## GitHub App configuration

GitHub backup uses a GitHub App device flow and does not embed a client secret.
Set `SCRIBE_GITHUB_APP_CLIENT_ID` while compiling the Tauri backend to the app's public client ID, which starts with `Iv`.
Builds reject OAuth App client IDs that start with `Ov` so they cannot request the classic account-wide `repo` scope.
Release builds read this value from the GitHub Actions repository variable named `SCRIBE_GITHUB_APP_CLIENT_ID` and fail before compiling when it is missing or invalid.

The GitHub App must have Device Flow enabled.
Set its homepage URL to `https://scribe.n8builds.dev`.
It should request only repository Contents read and write access.
Repository access is controlled by the app installation, not OAuth scopes.
Users create a private backup repository themselves, install the GitHub App on that repository, and enter its `owner/name` in Scribe.
Scribe deliberately does not request repository-administration permission or create repositories automatically.

If expiring user access tokens are enabled, Scribe stores both tokens in the OS keychain and rotates the refresh token before the access token expires.
Existing raw OAuth access-token credentials remain readable during migration, but new connections require a configured GitHub App client ID.
