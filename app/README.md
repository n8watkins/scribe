# LocalDictate

LocalDictate is a private, local-first Windows speech-to-text tray app built with Tauri, React, TypeScript, Rust, SQLite, and whisper.cpp.

Reference docs:

- [Product requirements](../docs/PRD.md)
- [Implementation plan](../docs/IMPLEMENTATION_PLAN.md)
- [Agent orchestration](../docs/AGENT_ORCHESTRATION.md)

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
