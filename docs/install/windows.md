# Scribe on Windows

Windows 10 and 11, x64.
This is Scribe's main platform: every feature works here, and in-app updates install themselves.

[macOS (beta)](macos.md) · [Linux (beta)](linux.md) · [Testing checklist](testing-checklist.md)

## Install

Download from the [latest release](https://github.com/n8watkins/scribe/releases/latest):

| File | Use it when |
|---|---|
| `Scribe_<version>_x64-setup.exe` | You install Scribe for yourself. Recommended. It installs for your user only, needs no administrator rights, and is what in-app updates install. |
| `Scribe_<version>_x64_en-US.msi` | You deploy Scribe with a management tool. In-app updates are published for the `.exe`, so update an `.msi` install by installing the next `.msi`. |

1. Run the installer.
   Scribe is not Authenticode-signed, so SmartScreen warns about an unrecognized app: click **More info**, then **Run anyway**.
2. The installer downloads Microsoft WebView2 if your PC doesn't have it yet.
3. `SHA256SUMS.txt` on the release lists a checksum for every file, if you want to check your download (`Get-FileHash <file>` in PowerShell).

## Update

Scribe checks for a new version about five seconds after launch, every six hours and whenever you bring its window back.
When it finds one it shows a notification and an **Update available** chip.
With **Install updates automatically** on (About), the next launch downloads the signed update, installs it behind a Scribe screen and restarts.
You can also use **About → Check for updates → Install**.
If an in-app update ever fails, run the new installer from the latest release over the old version; your settings and history stay.

## Uninstall

Settings → Apps → Installed apps → Scribe → Uninstall.
Your data stays behind (see below); delete those folders too for a clean removal.

## First run and permissions

Windows asks for no permissions.
Pick your microphone on the **Microphone** page, download a model on **Models & language** (the default is `small.en-q5_1`), then hold `Ctrl+Win` in any app and talk.
If Windows privacy settings block microphone access for desktop apps, turn it on in Settings → Privacy & security → Microphone → "Let desktop apps access your microphone".

## What works

| Feature | Windows |
|---|---|
| Hold-to-talk hotkey | Yes. Default `Ctrl+Win`, a modifier-only chord. |
| Toggle hotkey | Yes. Default `` ` `` on its own, acting on release. |
| Modifier-only chords (such as `Ctrl+Win`) | Yes |
| Dictate a note (hold the toggle key, tap `Q`) | Yes, Windows only |
| Paste into the app in front | Yes; terminals get the right paste for a console. Apps running as administrator can't receive it, so the text is copied for you instead. |
| Type it out (Direct Insert) | Yes |
| Clipboard restored after a paste | Yes: text, images and files |
| Transform selected text | Yes |
| GPU transcription | Yes, Vulkan (AMD, NVIDIA and Intel GPUs), set on the **Models & language** page |
| File transcription | Yes: WAV, MP3, FLAC and OGG directly; other formats (video, M4A) need `ffmpeg` on your PATH |
| Tray icon, close to tray | Yes |
| Launch at startup | Yes |
| GitHub backup | Yes; the token is kept in Windows Credential Manager |
| In-app updates | Yes, signed |

## Default hotkeys

| Action | Default |
|---|---|
| Hold to talk | `Ctrl+Win` |
| Toggle dictation | `` ` `` (left of `1`) |
| Dictate a note | hold `` ` `` and tap `Q` |
| Paste last transcript | `Ctrl+Alt+V` |
| Transform selection | `Ctrl+Alt+R` |
| Discard recording | `Ctrl+Alt+X` |
| Open dashboard | `Ctrl+Alt+F` |

Rebind any of them on the **Hotkeys** page, which reports conflicts.

## Where your data lives

| What | Where |
|---|---|
| Settings, history and notes (`scribe.sqlite3`), audio clips (`clips`) and models (`models`) | `%APPDATA%\com.natkins.scribe` |
| Logs | `%LOCALAPPDATA%\com.natkins.scribe\logs` |
| Local control file for integrations | `%USERPROFILE%\.scribe\control.json` |

**About → Copy diagnostics** collects the version, settings summary and recent log lines for a bug report.

## Known limits

- Scribe can't paste into apps running as administrator, because Windows blocks input from a normal app to an elevated one.
  The text is left on the clipboard: press `Ctrl+V`.
  Run Scribe as administrator too only if you really need this.
- SmartScreen warns on first install, because there is no code-signing certificate.
- After an update the taskbar icon can stay old until Windows refreshes its icon cache; see the [FAQ](../../FAQ.md).

## Troubleshooting

- **Nothing happens on the hotkey:** another app may own it; the **Hotkeys** page shows conflicts.
  Rebind it there.
- **The text arrives on the clipboard instead of in the app:** the app in front changed while Scribe was transcribing, or it runs as administrator.
  Press `Ctrl+V`.
- **An app ignores the paste:** set **Insert method** to **Type it out** on the **Dictation** page.
- **Transcription is slow:** pick a smaller model on **Models & language**, or turn on **Use GPU acceleration** on the **Models & language** page.
- **Anything else:** About → Copy diagnostics, and open an [issue](https://github.com/n8watkins/scribe/issues) with it.
