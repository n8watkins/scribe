# Scribe on macOS (beta)

macOS 13.3 (Ventura) or later.
One universal app runs natively on Apple Silicon and on Intel Macs.

**Beta:** every release installs this build on a GitHub-hosted Mac, launches it and transcribes a speech clip on both Apple Silicon and Intel (under Rosetta), but it has had little hands-on use on real Macs.
Please report what you find in [Issues](https://github.com/n8watkins/scribe/issues), or follow the [testing checklist](testing-checklist.md).

[Windows](windows.md) · [Linux (beta)](linux.md) · [Testing checklist](testing-checklist.md)

## Install

1. Download [`Scribe-macos-universal.dmg`](https://github.com/n8watkins/scribe/releases/latest/download/Scribe-macos-universal.dmg) from the latest release.
   The same file is also there under its versioned name, `Scribe_<version>_universal.dmg`.
2. Open the `.dmg` and drag **Scribe** into **Applications**.
3. Open Scribe from Applications.

### The first open: Scribe is not notarized

Scribe has no Apple Developer ID, so the app is ad-hoc signed and **not notarized** by Apple.
macOS therefore blocks it the first time you open it, with a message that Apple could not verify Scribe is free of malware.
You only have to get past this once per installed version.

- **macOS 15 (Sequoia) and later:** click **Done** on the message, open **System Settings → Privacy & Security**, scroll down to the line about Scribe and click **Open Anyway**, then confirm with **Open Anyway** and your password.
- **macOS 13 and 14:** Control-click (or right-click) Scribe in Applications, choose **Open**, then click **Open** in the dialog.
  The **Open Anyway** button in System Settings → Privacy & Security works on these versions too.
- **If macOS says Scribe "is damaged and can't be opened"**, remove the download quarantine in Terminal, then open Scribe again:

  ```sh
  xattr -dr com.apple.quarantine /Applications/Scribe.app
  ```

To check your download first, compare `shasum -a 256 Scribe-macos-universal.dmg` with the line in the release's `SHA256SUMS.txt`.

## First-run permissions

Scribe needs two permissions on macOS.

| Permission | Why | When you're asked | If you said no |
|---|---|---|---|
| **Microphone** | Recording your voice | The first time you dictate or test the microphone | System Settings → Privacy & Security → Microphone → turn on **Scribe** |
| **Accessibility** | Pasting, typing out text and copying a selection for Transform, which all send keystrokes to other apps | The **Dictation** page shows an **Accessibility permission** row with **Open System Settings** until it's on | System Settings → Privacy & Security → Accessibility → turn on **Scribe** (add it with **+** from Applications if it isn't listed) |

Without Accessibility, dictation still works: Scribe copies the text and tells you to press `Cmd+V`.
macOS may also ask for your login password the first time Scribe reads its GitHub backup token from the Keychain.

**After installing a new version**, macOS may treat it as a different app, because each build has its own ad-hoc signature.
If paste stops working after an update, turn **Scribe** off and on again in the Accessibility list, or remove it with **−** and add it again.

## Update

The macOS beta doesn't install updates itself yet.
**About → Check for updates** tells you when a new version is out, and **View releases** opens the download.
To update, quit Scribe from its menu bar icon, download the new `.dmg`, drag Scribe into Applications again and choose **Replace**.
Your settings, history and models stay.
Each new version needs the first-open step above again, and maybe the Accessibility toggle.

## Uninstall

1. Quit Scribe from its menu bar icon, and turn off **Launch at startup** in **General** first if you turned it on.
2. Drag **Scribe** from Applications to the Trash.
3. For a clean removal, also delete the folders in [Where your data lives](#where-your-data-lives), and reset the permissions:

   ```sh
   tccutil reset All com.natkins.scribe
   ```

   If you connected GitHub backup, delete the `com.natkins.scribe` item in Keychain Access too.

## What works

| Feature | macOS |
|---|---|
| Hold-to-talk hotkey | Yes. Default `Control+Option+Z`; start on press, stop on release. |
| Toggle hotkey | Yes. Default ``Control+Option+` ``, acting on press. |
| Modifier-only chords (such as `Control+Option` alone) | No: macOS global shortcuts need a key. |
| Dictate a note (hold the toggle key, tap `Q`) | No, Windows only |
| Paste into the app in front | Yes, with `Cmd+V`, once Accessibility is on. Terminal and iTerm paste with `Cmd+V` too. |
| Type it out (Direct Insert) | Yes, once Accessibility is on |
| Clipboard restored after a paste | Yes: everything on the clipboard, including rich text, images and Finder files |
| Transform selected text | Yes, once Accessibility is on (Scribe copies the selection with `Cmd+C`) |
| GPU transcription | Apple Silicon: Metal, on by default (**Use GPU acceleration** on **Models & language**). Intel Macs: CPU only. The GPU device list is Windows only. |
| File transcription | WAV, MP3, FLAC and OGG. Other formats (video, M4A) need `ffmpeg`, which Scribe opened from Finder usually can't find (see Known limits). |
| Menu bar icon, close to menu bar | Yes |
| Launch at startup | Yes, through a Login Item (LaunchAgent); not yet confirmed on a real Mac |
| GitHub backup | Yes; the token is kept in your login Keychain |
| In-app updates | Not yet: About tells you about a new version and links to it |

## Default hotkeys

| Action | Default |
|---|---|
| Hold to talk | `Control+Option+Z` |
| Toggle dictation | ``Control+Option+` `` |
| Paste last transcript | `Control+Option+V` |
| Transform selection | `Control+Option+R` |
| Discard recording | `Control+Option+X` |
| Open dashboard | `Control+Option+F` |

Rebind any of them on the **Hotkeys** page.
`Control+Option+Space` can't be used: macOS keeps it for switching input sources.

## Where your data lives

| What | Where |
|---|---|
| Settings, history and notes (`scribe.sqlite3`), audio clips (`clips`) and models (`models`) | `~/Library/Application Support/com.natkins.scribe` |
| Logs | `~/Library/Logs/com.natkins.scribe` |
| Local control file for integrations | `~/.scribe/control.json` |
| GitHub backup token | Login Keychain, item `com.natkins.scribe` |

**About → Copy diagnostics** collects the version, settings summary and recent log lines for a bug report.

## Known limits

- **Not notarized:** the first open of every version needs the steps above.
- **Permissions after updates:** see [First-run permissions](#first-run-permissions); the Accessibility grant may need toggling after each update.
- **Dvorak:** the paste and copy shortcuts are sent as the keys in the V and C positions of a US keyboard, so on a Dvorak layout they press other keys.
  Use **Insert method → Type it out** on the **Dictation** page.
- **ffmpeg from Finder:** apps opened from Finder or the Dock don't get your shell's `PATH`, so Scribe can't find a Homebrew `ffmpeg` for video and M4A files.
  Convert the file to WAV or MP3 first, or start Scribe from Terminal with `open -a Scribe` after `ffmpeg` is on your `PATH`.
- **Floating pill:** its see-through background needs a Tauri option that isn't on yet, so it may draw as a solid rectangle.
- **Note chord** and **modifier-only** hotkeys are Windows only.

## Troubleshooting

- **"Scribe can't be opened" or "Apple could not verify":** see [The first open](#the-first-open-scribe-is-not-notarized).
- **The text is copied but not pasted, with a message about Accessibility:** turn on Scribe in System Settings → Privacy & Security → Accessibility; after an update, turn it off and on again.
- **Nothing is recorded:** check System Settings → Privacy & Security → Microphone, then pick the right device on the **Microphone** page and use **Record test**.
- **A hotkey does nothing:** another app may own it; rebind it on the **Hotkeys** page.
- **Anything else:** About → Copy diagnostics, and open an [issue](https://github.com/n8watkins/scribe/issues) with it and your macOS version and Mac model.
