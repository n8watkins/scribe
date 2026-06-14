# Scribe

Scribe is a private, local-first Windows dictation app built with Tauri, React, TypeScript, Rust, SQLite, and whisper.cpp. Beyond push-to-talk transcription it adds quick notes, an optional local-LLM layer (dictation cleanup, note analysis, selected-text transform), a dictionary, searchable history with export, Google Drive sync, and a signed auto-updater.

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
