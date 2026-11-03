# Scribe Dev builds

Scribe Dev (`tauri.dev-flavor.conf.json`, identifier `com.natkins.scribe.dev`) installs next to stable Scribe and updates from its own feed, `releases/download/dev/latest-dev.json`, never from the stable `latest.json`.
Each dev build is versioned as a numbered pre-release of the next patch, for example `0.9.1-57` on top of `0.9.0`, so every build is newer than the one before it.

- Actions, Release, "Run workflow" with flavor `dev` builds and smoke-tests a Dev installer and keeps it as a workflow artifact; nothing is signed or published.
- Ticking "publish_dev" on a run from `main` also signs the updater artifact and replaces the assets of the rolling `dev` pre-release, which Scribe Dev installs pick up on their next update check.
- The `dev` release is a pre-release, so it never becomes GitHub's latest release and stable installs never see it.

This lives here rather than in `RELEASING.md`, which release-kit rewrites on every `apply`.
