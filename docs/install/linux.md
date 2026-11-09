# Scribe on Linux (beta)

x86_64 desktops with glibc 2.35 or later: Ubuntu 22.04 and later, Debian 12 and later, and other current distributions through the AppImage.
Scribe works best in an **X11 session**; on Wayland, see [X11 and Wayland](#x11-and-wayland).

**Beta:** every release installs the `.deb` with apt and runs the AppImage on a fresh Ubuntu 24.04 machine on GitHub, launches each one and transcribes a speech clip, but it has had little hands-on use on real Linux desktops.
Please report what you find in [Issues](https://github.com/n8watkins/scribe/issues), or follow the [testing checklist](testing-checklist.md).

[Windows](windows.md) · [macOS (beta)](macos.md) · [Testing checklist](testing-checklist.md)

## Install

Pick one.

| File | Use it when |
|---|---|
| [`Scribe-linux-amd64.deb`](https://github.com/n8watkins/scribe/releases/latest/download/Scribe-linux-amd64.deb) | Debian, Ubuntu and their derivatives. apt installs the libraries Scribe needs (WebKitGTK and the rest) and adds Scribe to your app menu. |
| [`Scribe-linux-x86_64.AppImage`](https://github.com/n8watkins/scribe/releases/latest/download/Scribe-linux-x86_64.AppImage) | Any other distribution, or when you'd rather not install a package. One file that carries its own libraries. |

Both files are also on the release under their versioned names (`Scribe_<version>_amd64.deb`, `Scribe_<version>_amd64.AppImage`).
To check a download, compare `sha256sum <file>` with its line in the release's `SHA256SUMS.txt`.

### .deb

```sh
sudo apt install ./Scribe-linux-amd64.deb
```

Then start **Scribe** from your app menu.

### AppImage

AppImages need FUSE 2, which newer distributions no longer install by default:

```sh
sudo apt install libfuse2t64   # Ubuntu 24.04 and later
sudo apt install libfuse2      # Ubuntu 22.04, Debian 12
sudo dnf install fuse-libs     # Fedora
```

Then make the file executable and run it:

```sh
chmod +x Scribe-linux-x86_64.AppImage
./Scribe-linux-x86_64.AppImage
```

Keep the AppImage in one place, for example `~/Applications`.
**Launch at startup** with the AppImage is not confirmed yet; if Scribe doesn't start after you log in, use the `.deb` or add the AppImage to your desktop's startup apps yourself.

## Update

The Linux beta doesn't install updates itself yet.
**About → Check for updates** tells you when a new version is out, and **View releases** opens the download.

- **.deb:** download the new `.deb` and run `sudo apt install ./Scribe-linux-amd64.deb` again.
- **AppImage:** quit Scribe, replace the old file with the new one (same name and place), and start it again.

Your settings, history and models stay.

## Uninstall

1. Quit Scribe from its tray icon, and turn off **Launch at startup** in **General** first if you turned it on.
2. Remove the app:
   - **.deb:** `sudo apt remove scribe`
   - **AppImage:** delete the file.
3. For a clean removal, also delete the folders in [Where your data lives](#where-your-data-lives).
   If you connected GitHub backup, remove the Scribe entry (`com.natkins.scribe`) from your keyring, for example with Seahorse (Passwords and Keys) on GNOME.

## First run and permissions

Linux asks for no permissions.
Pick your microphone on the **Microphone** page (**System default**, **PipeWire** or **PulseAudio** follow the input chosen in your desktop's sound settings; each sound card is listed too) and use **Record test**, download a model on **Models & language**, then hold `Ctrl+Alt+Space` in any app and talk.
Sandboxed apps (Flatpak, Snap) may not accept synthetic keystrokes or clipboard changes from outside; see Known limits.

## X11 and Wayland

Scribe sends keystrokes, reads the window in front and registers its global hotkeys through **X11**.

| | X11 session | Wayland session |
|---|---|---|
| Global hotkeys | Work everywhere | Fire only while an X11 (XWayland) app has focus; not while a native Wayland app is in front |
| Paste and Type it out | Work in every app | Work in X11 (XWayland) apps; in native Wayland apps Scribe copies the text and says "Scribe can't paste into Wayland apps yet. Copied. Press Ctrl+V to paste." |
| Transform selection | Works | Works in X11 apps; elsewhere it opens Scribe's own panel, where you type or paste the text |
| Clipboard | Works | Works |

Check which session you are in with `echo $XDG_SESSION_TYPE`.

**Workarounds on Wayland today:**

- Log in to an X11 session: on the login screen, pick **Ubuntu on Xorg** (GNOME) or **Plasma (X11)** (KDE) from the gear or session menu.
- Or start and stop dictation from the tray icon (**Start Dictation**, **Stop Dictation**, **Paste Last Transcript**) or the floating pill, and press `Ctrl+V` after each dictation.

**Coming (no date yet): native Wayland support.**
Scribe will use the desktop's portals: the RemoteDesktop portal to paste and type, and the GlobalShortcuts portal for hotkeys (GNOME 48+, KDE Plasma 5.27+).
Your desktop will ask once for permission, Scribe will save the grant so it never asks again, and where a desktop has no portal Scribe will keep copying the text for you to paste.

## What works

| Feature | Linux |
|---|---|
| Hold-to-talk hotkey | Yes. Default `Ctrl+Alt+Space`; start on press, stop on release. X11 only, see above. |
| Toggle hotkey | Yes. Default ``Ctrl+Alt+` ``, acting on press. X11 only, see above. |
| Modifier-only chords (such as `Ctrl+Super` alone) | No: X11 key grabs need a key. |
| Dictate a note (hold the toggle key, tap `Q`) | No, Windows only |
| Paste into the app in front | Yes on X11 with `Ctrl+V`; terminals (GNOME Terminal, Konsole, kitty, Alacritty, xterm and others) get `Shift+Insert` |
| Type it out (Direct Insert) | Yes on X11 |
| Clipboard restored after a paste | Text and images, plus the PRIMARY (middle-click) selection; other formats such as copied files are not put back |
| Transform selected text | Yes on X11 (Scribe copies the selection with `Ctrl+C`, or `Ctrl+Shift+C` in a terminal) |
| GPU transcription | No: the Linux build of whisper.cpp runs on the CPU |
| File transcription | WAV, MP3, FLAC and OGG directly; other formats (video, M4A) need `ffmpeg` installed (`sudo apt install ffmpeg`) |
| Tray icon, close to tray | Yes, through AppIndicator; not yet confirmed on a real desktop (see Known limits) |
| Launch at startup | Yes, through an autostart entry; not yet confirmed on a real desktop |
| GitHub backup | Yes; the token is kept in your keyring through the Secret Service (GNOME Keyring or KWallet) |
| In-app updates | Not yet: About tells you about a new version and links to it |

## Default hotkeys

| Action | Default |
|---|---|
| Hold to talk | `Ctrl+Alt+Space` |
| Toggle dictation | ``Ctrl+Alt+` `` |
| Paste last transcript | `Ctrl+Alt+V` |
| Transform selection | `Ctrl+Alt+R` |
| Discard recording | `Ctrl+Alt+X` |
| Open dashboard | `Ctrl+Alt+F` |

Rebind any of them on the **Hotkeys** page, which reports a shortcut another app has already grabbed.

## Where your data lives

| What | Where |
|---|---|
| Settings, history and notes (`scribe.sqlite3`), audio clips (`clips`) and models (`models`) | `~/.local/share/com.natkins.scribe` |
| Logs | `~/.local/share/com.natkins.scribe/logs` |
| Local control file for integrations | `~/.scribe/control.json` |
| GitHub backup token | Your keyring (Secret Service), service `com.natkins.scribe` |

**About → Copy diagnostics** collects the version, settings summary and recent log lines for a bug report.

## Known limits

- **Wayland:** see [X11 and Wayland](#x11-and-wayland).
- **Tray icon on GNOME:** stock GNOME shows no tray icons; Ubuntu's own GNOME does.
  Elsewhere install the **AppIndicator and KStatusNotifierItem Support** extension, or reopen Scribe from the app menu.
- **No compositor:** without a compositing window manager the floating pill and its result toast draw as a dark rectangle instead of see-through.
  GNOME, KDE and most desktops composite, so this rarely shows.
- **Flatpak and Snap apps** may ignore the paste; use **Insert method → Type it out** on the **Dictation** page, or paste with `Ctrl+V` yourself.
- **No GPU:** transcription runs on the CPU; pick a smaller model on **Models & language** if it's slow.
- **Note chord** and **modifier-only** hotkeys are Windows only.

## Troubleshooting

- **The AppImage doesn't start and mentions FUSE or `libfuse.so.2`:** install FUSE 2 as shown under [AppImage](#appimage).
- **Hotkeys do nothing:** check `echo $XDG_SESSION_TYPE`; on `wayland`, see [X11 and Wayland](#x11-and-wayland).
  On X11, another app may hold the shortcut: rebind it on the **Hotkeys** page.
- **"Scribe can't paste into Wayland apps yet":** the app in front is a native Wayland app; press `Ctrl+V`.
- **No microphone, or silence:** pick the device on the **Microphone** page and use **Record test**; check the input in your desktop's sound settings (PipeWire or PulseAudio) and that it isn't muted.
- **GitHub backup forgets the connection after a restart:** your keyring must be running and unlocked (GNOME Keyring or KWallet).
- **Anything else:** About → Copy diagnostics, and open an [issue](https://github.com/n8watkins/scribe/issues) with it, your distribution and version, desktop, and session type.
  Starting Scribe from a terminal also prints its log there.
