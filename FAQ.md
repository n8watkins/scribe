# Scribe — FAQ

Answers to questions that come up while using or building Scribe. Newest at the top.

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
