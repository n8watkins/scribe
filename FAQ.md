# Scribe FAQ

Answers to questions that come up while using or building Scribe. Newest at the top.

## How long can one recording be? Will Scribe cut me off mid-sentence?

Scribe never cuts you off while you are talking at a normal length.
A recording ends when you stop it (release the hold key, or tap the toggle key again), or on its own in one of these ways:

- **Silence auto-stop:** a toggle (hands-free) recording ends after 60 seconds of silence once you have said something.
- **Long recordings:** past 10 minutes, the same 60-second silence also ends hold-to-talk recordings, even if nothing was said, so a stuck key or a forgotten open mic still stops.
- **Hard stop:** at 30 minutes a recording stops even mid-sentence, as a safety limit. Nothing is lost: what you said is still transcribed and saved to History.

All three are in **Microphone → Advanced timing** (*Auto-stop pause*, *Silence stops any recording after*, *Hard stop*).
The hard stop goes up to 60 minutes, because a recording is held in memory until it finishes.
Turning **Auto-stop on silence** off leaves only the hard stop.

## Is there a Mac or Linux version?

Yes, as betas: a universal macOS app (Apple Silicon and Intel, macOS 13.3 or later) and a Linux `.deb` and AppImage (x86_64), on every release next to the Windows installers.
Each platform has a guide with install steps, permissions, what works there and known limits: [Windows](docs/install/windows.md), [macOS](docs/install/macos.md), [Linux](docs/install/linux.md).

- **macOS:** the app is not notarized (Scribe has no Apple Developer ID), so the first open needs **Open Anyway** in System Settings → Privacy & Security; the [macOS guide](docs/install/macos.md#the-first-open-scribe-is-not-notarized) has every step.
- **Linux:** hotkeys and pasting work on X11 and on Wayland; on Wayland your desktop asks once for permission, and desktops without the portals get a copy fallback or a command to bind ([details](docs/install/linux.md#x11-and-wayland)).
- **Updates:** the Mac and Linux betas don't update themselves yet; About tells you when a new version is out, and you install it from the latest release.
- Testing a beta? Follow the [testing checklist](docs/install/testing-checklist.md) and send back what you find.

## Why is the taskbar or Task Manager icon wrong or old?

The app icon **is** the Scribe mic logo in every build. If the **window** icon looks right but the **taskbar** or **Task Manager** icon is old or generic, that's **Windows' icon cache**, not Scribe: Windows caches an app's shell icon and does **not** refresh it when the app updates itself in place. The window icon is read live (so it's correct), while the taskbar/Task Manager pull from the stale cache.

**Fix:** do a clean install of the latest release (download the installer from the [latest release](https://github.com/n8watkins/scribe/releases/latest) and run it), or reboot; both refresh the icon cache.

## How do updates work? Can I install one from the app, or dismiss it?

Yes to both, on Windows.
The macOS and Linux betas only tell you about a new version for now; install it from the [latest release](https://github.com/n8watkins/scribe/releases/latest).

- **Notification:** Scribe checks for new releases shortly after launch, every six hours, and whenever you refocus the window. When one is found you get an **OS notification** (even when minimized) plus an **"Update available"** chip in the top-right.
- **Install from the app:** **About → "Check for updates"**, then click the **"Install update"** button that appears; *that* one downloads, installs, and restarts. ("Check for updates" alone only checks.)
- **Don't want it?** The top-right chip has **"View"** (opens the release notes) and a **dismiss "✕"**; dismissing hides it (you can still update later from About) and it stays dismissed for that version until a newer one ships.
- **If the in-app install ever fails:** grab the installer from the [latest release](https://github.com/n8watkins/scribe/releases/latest) and run it.

## Can I install other / custom Whisper models?

**Not from the UI today.** The **Models** view offers a fixed, curated catalog of 11 Whisper models you can download and switch between:

| Model | Trade-off |
|---|---|
| `tiny.en`, `base.en` | Fastest, least accurate (English) |
| `small.en`, `small.en-q5_1` | Balanced (English); `small.en-q5_1` is quantized and **the default** |
| `medium.en` | More accurate, slower (English) |
| `tiny`, `base`, `small`, `medium` | The same sizes for many languages, with translate to English |
| `large-v3-turbo-q5_0`, `large-v3-turbo` | Most accurate, heaviest (many languages) |

Scribe only recognizes models from this built-in catalog (download, select, and delete are all keyed to it), and it does **not** scan the models folder for arbitrary files. So dropping your own `ggml-*.bin` into the models folder won't make it appear or become selectable yet.

**Want a different model?** Custom/local model support (point Scribe at any whisper.cpp-compatible `ggml` `.bin`) is a reasonable future addition and is on the backlog. The models already live in your local data folder under `models/` (see **Data & Privacy → Local data**), so the storage plumbing exists; what's missing is the UI + selection logic to use a non-catalog file, plus surfacing the language/quantization so the right Whisper flags are passed.

## Why didn't I get notified about an update?

Two reasons, both addressed in 0.5.13+:

- **Timing.** A new release only exists once CI finishes publishing it. If you were running an older build *before* the new one was published, there was correctly nothing to notify about until then. The check also used to run only every 2 hours.
- **Reach.** The old alert was an in-app topbar button + toast, which you can't see while Scribe is minimized to the tray.

Current builds check about five seconds after launch, every six hours, **and whenever you refocus the window**, and they fire a real **OS notification** (even when minimized) the first time they see a new version.
To update, use **About → Check for updates → Install update**.
If the in-app installer ever fails to apply, you can always grab the installer directly from the [latest release](https://github.com/n8watkins/scribe/releases/latest).
