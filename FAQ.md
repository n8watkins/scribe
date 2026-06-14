# Scribe — FAQ

Answers to questions that come up while using or building Scribe. Newest at the top.

## The taskbar / Task Manager icon is wrong or old — why?

The app icon **is** the Scribe mic logo in every build. If the **window** icon looks right but the **taskbar** or **Task Manager** icon is old or generic, that's **Windows' icon cache**, not Scribe: Windows caches an app's shell icon and does **not** refresh it when the app updates itself in place. The window icon is read live (so it's correct), while the taskbar/Task Manager pull from the stale cache.

**Fix:** do a clean install of the latest release (download the installer from the [latest release](https://github.com/n8watkins/scribe/releases/latest) and run it), or reboot — both refresh the icon cache.

## How do updates work? Can I install one from the app, or dismiss it?

Yes to both.

- **Notification:** Scribe polls GitHub for new releases shortly after launch, on a timer, and whenever you refocus the window. When one is found you get an **OS notification** (even when minimized) plus an **"Update available"** chip in the top-right.
- **Install from the app:** **About → "Check for updates"**, then click the **"Install update"** button that appears — *that* one downloads, installs, and restarts. ("Check for updates" alone only checks.)
- **Don't want it?** The top-right chip has **"View"** (opens the release notes) and a **dismiss "✕"** — dismissing hides it (you can still update later from About) and it stays dismissed for that version until a newer one ships.
- **If the in-app install ever fails:** grab the installer from the [latest release](https://github.com/n8watkins/scribe/releases/latest) and run it.

## Can I install other / custom Whisper models?

**Not from the UI today.** The **Models** view offers a fixed, curated catalog of six Whisper models you can download and switch between:

| Model | Trade-off |
|---|---|
| `tiny.en` | Fastest, least accurate |
| `base.en` | Fast |
| `small.en` | Balanced |
| `small.en-q5_1` | Balanced, quantized — **the default** |
| `medium.en` | More accurate, slower |
| `large-v3-turbo-q5_0` | Most accurate, heaviest |

Scribe only recognizes models from this built-in catalog — download, select, and delete are all keyed to it — and it does **not** scan the models folder for arbitrary files. So dropping your own `ggml-*.bin` into the models folder won't make it appear or become selectable yet.

**Want a different model?** Custom/local model support (point Scribe at any whisper.cpp-compatible `ggml` `.bin`) is a reasonable future addition and is on the backlog. The models already live in your local data folder under `models/` (see **Data & Privacy → Local data**), so the storage plumbing exists — what's missing is the UI + selection logic to use a non-catalog file, plus surfacing the language/quantization so the right Whisper flags are passed.

## Why didn't I get notified about an update?

Two reasons, both addressed in 0.5.13+:

- **Timing.** A new release only exists once CI finishes publishing it. If you were running an older build *before* the new one was published, there was correctly nothing to notify about until then. The check also used to run only every 2 hours.
- **Reach.** The old alert was an in-app topbar button + toast, which you can't see while Scribe is minimized to the tray.

From 0.5.13 the app checks ~5s after launch, every 30 minutes, **and whenever you refocus the window**, and it fires a real **OS notification** (even when minimized) the first time it sees a new version. To update: **About → Check for updates → Install update**. If the in-app installer ever fails to apply, you can always grab the installer directly from the [latest release](https://github.com/n8watkins/scribe/releases/latest).
