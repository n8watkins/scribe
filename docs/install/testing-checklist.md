# Testing checklist for the macOS and Linux betas

For anyone trying Scribe on a real Mac or Linux machine.
It takes about 30 minutes.
CI already proves each build installs, starts and transcribes a WAV file; what only a person can check is everything below: real permissions, a real microphone, real apps and real desktops.

Install guides: [macOS](macos.md) · [Linux](linux.md).

## Before you start

- Note your system: Mac model and macOS version (Apple menu → About This Mac), or Linux distribution, version and desktop (GNOME, KDE, ...).
- On Linux, note the session type: `echo $XDG_SESSION_TYPE` (`x11` or `wayland`).
  If you can, run the Linux list twice: once in an X11 session and once in a Wayland session.
- Have a text editor and a browser open, plus a terminal.
- Copy an image to the clipboard (for example a screenshot), so you can check later that Scribe puts it back.

## The checks

Mark each one **pass**, **fail** or **skip**, and add a note for anything that surprised you.

### Install and first open

1. Download the file from the latest release and install it as the guide says.
   - Mac: which Gatekeeper message did you get, and which step opened it (Open Anyway, Control-click → Open, or `xattr`)?
   - Linux: `.deb`, AppImage or both? Did the AppImage need `libfuse2`?
2. Scribe's window opens, and its icon appears in the menu bar (Mac) or tray (Linux).
3. About shows the version you downloaded.

### Permissions (Mac)

4. The first recording asks for the **Microphone**; allow it.
5. The **Dictation** page shows the **Accessibility permission** row; **Open System Settings** opens the right list; after turning Scribe on, the row goes away.
6. Deny something on purpose once (or turn it off later) and note what Scribe says.

### Microphone and model

7. On **Microphone**, your microphone is listed; **Record test** then **Play test** plays your voice back.
8. On **Models & language**, download a model; it completes and can be selected.

### Dictation into other apps

Use the default hotkeys from the guide (Mac `Control+Option+…`, Linux `Ctrl+Alt+…`).

9. **Hold to talk:** hold the key in the text editor, say a sentence, release; the text appears at the cursor.
10. **Toggle:** press the toggle hotkey, talk, press it again; the text appears.
11. **Clipboard comes back:** after 9 or 10, paste with `Cmd+V`/`Ctrl+V`: you get the image you copied earlier, not the dictation.
12. **Other apps:** dictate into a browser text field and into a terminal (Mac: Terminal or iTerm; Linux: your terminal).
13. **App switch:** start dictating, switch to another app before the text arrives; Scribe should not paste into the wrong app and should say the text is on the clipboard.
14. **Paste last:** press the paste-last hotkey in another app; the last dictation is inserted again.
15. **Type it out:** set **Insert method → Type it out** on the **Dictation** page and repeat 9.
16. **Discard:** start a recording and press the discard hotkey; nothing is inserted.
17. **Transform** (optional, needs a local AI server set up on **Local AI**): select a sentence in the editor, press the transform hotkey, speak an instruction; the selection is rewritten.

### Wayland only (Linux)

The longer walk-through, with what to expect on GNOME and KDE, is in [Testing Scribe on Wayland](linux-wayland-testing.md).

18. **Hotkeys:** on first start, did your desktop show a dialog listing Scribe's shortcuts? With a native Wayland app in front (GNOME Text Editor, Kate, Firefox), do hold to talk and toggle fire? What does the **Hotkeys** page say at the top right ("Managed by your desktop" or "Global shortcuts")?
19. **Paste:** dictate into a native Wayland app. Did the desktop ask once to allow remote control? After allowing it, did the text appear? Dictate again, and again after restarting Scribe: no second dialog?
20. **X11 apps:** with an X11 app in front (for example `xterm`, or an app started with `GDK_BACKEND=x11`), do the hotkeys and paste work?

### App behaviour

21. Close the window: Scribe keeps running in the menu bar or tray, and the hotkeys still work.
22. **Transcribe a file:** transcribe an MP3 or WAV.
23. **Launch at startup** (in **General**): turn it on, log out and back in; does Scribe start?
24. **About → Check for updates:** what does it say?
25. **GitHub backup** (optional): connect, quit and restart Scribe; still connected? (Mac: did the Keychain ask for your password? Linux: which keyring?)
26. Look at the floating pill while recording: is its background see-through or a solid rectangle?

### Updating (if a newer version comes out while you test)

27. Install the new version over the old one.
    Mac: did it need the first-open step again? Did paste still work, or did Accessibility need toggling? Did the Microphone prompt come back?

## What to send back

Open an [issue](https://github.com/n8watkins/scribe/issues/new) titled "Beta test: macOS 15.1 on M2" (or your system), with:

- Your system details from "Before you start".
- The list of check numbers with pass, fail or skip, and notes.
- **About → Copy diagnostics**, pasted in.
- Screenshots of anything that looked wrong, especially the pill, the toast and any error message.
- For a crash or a failure: the newest log file (Mac: `~/Library/Logs/com.natkins.scribe`; Linux: `~/.local/share/com.natkins.scribe/logs`), and on a Mac any `Scribe` crash report from Console → Crash Reports.
