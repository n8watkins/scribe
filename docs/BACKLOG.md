# Backlog

Ideas that are written down but not planned.
Each one says when it would be worth building.

## Per-app or per-project custom words

Whisper reads about 450 characters of custom words, so a long vocabulary doesn't fit.
The idea is to keep several short sets and send the one that matches where you dictate.

Examples:

- In VS Code on the n8builds repo: n8builds, Appturnity, Tauri, worktree, pnpm, Cloudflare.
- In Slack: teammates' names, channel and project names.
- In an email: client names, product names, your own sign-off.

Detection:

- Scribe already records the focused window when a recording starts, to paste back into it.
- The app comes from that window's process name (Code.exe, slack.exe, OUTLOOK.EXE, a browser and its tab title).
- The project comes from the window title, which VS Code and most terminals set to the folder or repo name.
- A set would match on app, title text or both, with the general set as the fallback.

Why it might not generalise:

- Most people have one vocabulary, and one well-chosen set of 35 to 40 terms covers it.
- Window titles are unreliable: browsers, remote desktops and terminals often hide the project.
- It adds a settings surface most users would never fill in.

Worth it if:

- the custom words budget is regularly full for real users, not just in tests;
- and a held-out test shows that terms swapped out of the general set to make room are then missed, while a per-app set would have caught them.
