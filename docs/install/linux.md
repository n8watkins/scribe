# Scribe on Linux (beta)

x86_64 desktops with glibc 2.35 or later: Ubuntu 22.04 and later, Debian 12 and later, and other current distributions through the AppImage.
Scribe works in **X11 and Wayland sessions**; see [X11 and Wayland](#x11-and-wayland) for what your desktop gives it.

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

X11 sessions ask for no permissions.
On Wayland, your desktop asks once whether Scribe may control your keyboard (so it can paste for you), and, on desktops with the GlobalShortcuts portal, which keys to use for Scribe's hotkeys.
Scribe remembers both answers and doesn't ask again; see [X11 and Wayland](#x11-and-wayland).

Pick your microphone on the **Microphone** page (**System default**, **PipeWire** or **PulseAudio** follow the input chosen in your desktop's sound settings; each sound card is listed too) and use **Record test**, download a model on **Models & language**, then hold `Ctrl+Alt+Space` in any app and talk.
Sandboxed apps (Flatpak, Snap) may not accept synthetic keystrokes or clipboard changes from outside; see Known limits.

## X11 and Wayland

Scribe picks its method when it starts and for each paste, from your session type (`echo $XDG_SESSION_TYPE`).

| | X11 session | Wayland session |
|---|---|---|
| Global hotkeys | X11 key grabs; work everywhere | Your desktop's **GlobalShortcuts portal** where it has one (KDE Plasma 5.27+, GNOME 48+, Hyprland and others), including hold to talk; elsewhere only while an X11 app has focus, or bind Scribe's command yourself (below) |
| Paste and Type it out | XTEST keystrokes in every app | X11 (XWayland) apps: as on X11. Native Wayland apps: the **RemoteDesktop portal**, after you allow it once |
| Transform selection | Works | Works in X11 apps; elsewhere it opens Scribe's own panel, where you type or paste the text |
| Clipboard | Works | Works; natively on desktops with the data-control protocol (KDE, wlroots), through XWayland on GNOME |

The **Dashboard** shows the method in use under **After dictation**: **Portal paste**, **X11 paste** or **Copy only**.

### Pasting on Wayland

The first time Scribe pastes into a native Wayland app, your desktop asks whether Scribe may control your keyboard ("Remote desktop" or "Remote control" on GNOME, "Remote control requested" on KDE).
Scribe asks for the keyboard only, never the screen or the mouse, and asks the desktop to remember the answer.
Scribe saves the grant (a restore token, in `wayland-portal.json` in its data folder, readable only by you) and reuses it, so you are not asked again.
To ask before your first dictation instead, use **Allow pasting** on the **Dictation** page.

- **You said no:** Scribe copies each dictation and says "Copied. Press Ctrl+V to paste."
  It doesn't ask again on its own; **Ask again** on the **Dictation** page shows the dialog again.
- **You revoked it later** (for example in GNOME Settings, Apps or Privacy): the desktop asks once more on the next paste.
- **No portal, or one too old to remember the answer:** Scribe copies instead of asking on every paste.
- **Scribe's own window is in front:** Wayland doesn't let Scribe switch to the app you were in, so Scribe copies; click into your app and press `Ctrl+V`, or use the paste-last hotkey there.

Scribe pastes into native Wayland apps with `Shift+Insert`, which pastes in GTK, Qt, Chromium, Firefox and Electron apps and in terminals; Wayland doesn't tell Scribe which app is in front, so one key has to work everywhere.
While Scribe works, GNOME and KDE show their remote-control indicator in the top bar; Scribe closes the session after 45 seconds without a paste.

### Hotkeys on Wayland

On desktops with the GlobalShortcuts portal, your desktop shows a dialog the first time Scribe starts, listing Scribe's actions with Scribe's suggested keys (the defaults below).
Confirm it, or pick other keys there.
From then on the desktop delivers the hotkeys in every app, and the **Hotkeys** page shows the keys it assigned ("Managed by your desktop").
Change them in your desktop's keyboard settings (System Settings, Keyboard, Shortcuts, Scribe on KDE), or with **Change keys** on desktops that support it.
If you cancelled the dialog, **Set up shortcuts** on the **Hotkeys** page shows it again.

Without the portal (GNOME 47 and older, Xfce, Cinnamon and others on Wayland), Scribe's hotkeys only fire while an X11 app has focus.
Bind Scribe's command in your desktop's keyboard settings instead; the **Hotkeys** page shows the exact command for your install with a copy button:

| Command | Does |
|---|---|
| `Scribe toggle` | Start or stop dictation |
| `Scribe paste-last` | Paste the last dictation |
| `Scribe cancel` | Cancel the recording |
| `Scribe show` | Show Scribe |
| `Scribe hold-start`, `Scribe hold-stop` | Hold to talk, for desktops that run a command on key release too (sway and Hyprland `bindr`) |

With the AppImage, use its full path instead of `Scribe`, for example `~/Applications/Scribe-linux-x86_64.AppImage toggle`.
Scribe has to be running: the command hands the action to it and exits.

- **GNOME:** Settings, Keyboard, View and Customize Shortcuts, Custom Shortcuts, then Add Shortcut.
  Paste the command and press the keys, for example ``Ctrl+Alt+` ``.
- **KDE Plasma:** System Settings, Keyboard, Shortcuts, Add New, then Command or Script.

## What works

| Feature | Linux |
|---|---|
| Hold-to-talk hotkey | Yes. Default `Ctrl+Alt+Space`; start on press, stop on release. On Wayland through the GlobalShortcuts portal, see above. |
| Toggle hotkey | Yes. Default ``Ctrl+Alt+` ``, acting on press. On Wayland through the GlobalShortcuts portal or a desktop shortcut running `Scribe toggle`. |
| Modifier-only chords (such as `Ctrl+Super` alone) and bare keys (such as `` ` ``) | No: X11 key grabs and the portal need a key with Ctrl, Alt or Super (F1 to F24 also work alone). Installs that had one are moved to the defaults below once. |
| Dictate a note (hold the toggle key, tap `Q`) | No, Windows only |
| Paste into the app in front | Yes. X11: `Ctrl+V`, and `Shift+Insert` in terminals (GNOME Terminal, Konsole, kitty, Alacritty, xterm and others). Native Wayland apps: `Shift+Insert` through the RemoteDesktop portal, after you allow it once |
| Type it out (Direct Insert) | Yes on X11; on Wayland through the RemoteDesktop portal (characters your keyboard layout lacks may not type) |
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
| Wayland paste permission (restore token, user-only) | `~/.local/share/com.natkins.scribe/wayland-portal.json` |
| GitHub backup token | Your keyring (Secret Service), service `com.natkins.scribe` |

**About → Copy diagnostics** collects the version, settings summary and recent log lines for a bug report.

## Known limits

- **Wayland:** see [X11 and Wayland](#x11-and-wayland).
  Desktops without the RemoteDesktop portal (or with one older than version 2, which can't remember the answer) get **Copy only** in native Wayland apps.
  Transform selection works only in X11 apps on Wayland.
- **Tray icon on GNOME:** stock GNOME shows no tray icons; Ubuntu's own GNOME does.
  Elsewhere install the **AppIndicator and KStatusNotifierItem Support** extension, or reopen Scribe from the app menu.
- **No compositor:** without a compositing window manager the floating pill and its result toast draw as a dark rectangle instead of see-through.
  GNOME, KDE and most desktops composite, so this rarely shows.
- **Flatpak and Snap apps** may ignore the paste; use **Insert method → Type it out** on the **Dictation** page, or paste with `Ctrl+V` yourself.
- **No GPU:** transcription runs on the CPU; pick a smaller model on **Models & language** if it's slow.
- **Note chord** (hold the toggle key, tap `Q`) and **modifier-only** hotkeys are Windows only.

## Troubleshooting

- **The AppImage doesn't start and mentions FUSE or `libfuse.so.2`:** install FUSE 2 as shown under [AppImage](#appimage).
- **Hotkeys do nothing:** check `echo $XDG_SESSION_TYPE`; on `wayland`, open the **Hotkeys** page, which says whether your desktop delivers them and what to do if not (see [Hotkeys on Wayland](#hotkeys-on-wayland)).
  On X11, another app may hold the shortcut: rebind it on the **Hotkeys** page.
- **"Pasting into Wayland apps is off":** you turned the desktop's dialog down; use **Ask again** on the **Dictation** page.
- **"Your desktop doesn't let apps paste into Wayland apps":** your desktop has no RemoteDesktop portal; press `Ctrl+V` after each dictation, or use an X11 session.
- **No microphone, or silence:** pick the device on the **Microphone** page and use **Record test**; check the input in your desktop's sound settings (PipeWire or PulseAudio) and that it isn't muted.
- **GitHub backup forgets the connection after a restart:** your keyring must be running and unlocked (GNOME Keyring or KWallet).
- **Anything else:** About → Copy diagnostics, and open an [issue](https://github.com/n8watkins/scribe/issues) with it, your distribution and version, desktop, and session type.
  Starting Scribe from a terminal also prints its log there.
