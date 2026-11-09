# Testing Scribe on Wayland

For anyone helping test Scribe's Wayland support on a real Linux desktop.
It takes about 30 minutes per desktop.
Automated tests already cover how Scribe talks to the desktop portals, against a stand-in portal; what only a real desktop can show is whether GNOME and KDE behave the way Scribe expects.

Do as many of the three runs as you can: **GNOME on Wayland**, **KDE Plasma on Wayland**, and **any X11 session**.
Install Scribe first with the [Linux guide](linux.md), download a model on **Models & language**, and check that **Record test** works on **Microphone**.

## Before each run

1. Log in to the session you are testing.
   On the login screen, the gear or session menu picks it: **GNOME** or **Ubuntu** (Wayland), **Ubuntu on Xorg** or **GNOME on Xorg** (X11), **Plasma (Wayland)** or **Plasma (X11)**.
2. In a terminal, note these:

   ```sh
   echo $XDG_SESSION_TYPE
   gnome-shell --version 2>/dev/null; plasmashell --version 2>/dev/null
   /usr/libexec/xdg-desktop-portal --version 2>/dev/null || /usr/lib/xdg-desktop-portal --version
   ```

3. Start fresh, so the one-time dialogs show: quit Scribe from its tray icon, then

   ```sh
   rm -f ~/.local/share/com.natkins.scribe/wayland-portal.json
   ```

   On KDE, also remove any old Scribe entry under System Settings, Keyboard, Shortcuts.
4. Have open: a native Wayland text app (**GNOME Text Editor** or **Kate**), **Firefox** or **Chrome**, a terminal (**GNOME Terminal**, **Ptyxis** or **Konsole**), and `xterm` if you have it (an X11 app).
5. Copy an image to the clipboard (a screenshot), so you can check that Scribe puts it back.

## The checks

Mark each one **pass**, **fail** or **skip**, and note anything that surprised you.

### Hotkeys

1. Start Scribe.
   - **Expected on KDE, and GNOME 48 or newer:** a dialog from the desktop lists Scribe's actions (Hold to talk, Start or stop dictation, ...) with suggested keys such as `Ctrl+Alt+Space`.
     Confirm it.
   - **Expected on GNOME 47 or older:** no dialog.
   - Screenshot the dialog if one appears.
2. Open **Hotkeys** in Scribe and note the text at the top right ("Managed by your desktop" or "Global shortcuts") and any message under the list.
   Screenshot the page.
3. With **GNOME Text Editor** or **Kate** in front, hold `Ctrl+Alt+Space`, say a sentence, release.
   Does the pill appear while you hold, and does it stop when you release?
4. Press ``Ctrl+Alt+` `` (the key left of 1), talk, press it again.
   Does it start and stop?
5. Quit Scribe and start it again.
   Did the hotkey dialog appear a second time? (It shouldn't.)
6. **Only where Hotkeys says your desktop has no shortcuts portal:** open **Use a shortcut from your desktop's keyboard settings** on the **Hotkeys** page, copy the "Start or stop dictation" command, add it as a custom shortcut in your desktop's keyboard settings (the page says where), and try it with GNOME Text Editor in front.

### Pasting

7. Dictate into GNOME Text Editor or Kate (hold to talk or toggle).
   - **Expected:** the first time, the desktop asks whether Scribe may control your keyboard ("Remote desktop" or "Remote control").
     Allow it, and tick any "remember" or "allow restoring" option it offers.
   - Does the text appear after you allow it?
   - Screenshot the dialog.
8. Dictate again into the same app: the text appears with no dialog.
9. Look at the top bar while and just after Scribe pastes: is there a remote-control or screen-sharing indicator?
   Does it go away within a minute?
10. Paste with `Ctrl+V`: you get the image you copied, not the dictation.
11. Dictate into Firefox or Chrome (a search box or a text area), and into the terminal at a shell prompt.
    Does the text land in each? In the terminal, is it pasted once, not twice?
12. Quit and restart Scribe, then dictate into GNOME Text Editor again.
    Did the remote-control dialog come back? (It shouldn't.)
13. On the **Home** page, what does the **After dictation** card say (Portal paste, X11 paste or Copy only)?
14. **Type it out:** on **Dictation**, set **Insert method** to **Type it out** and dictate a sentence with punctuation into the editor.
    Is it typed correctly, capitals and punctuation included? Set it back to **Paste, keeps your clipboard** afterwards.

### Saying no

15. Quit Scribe, delete the file again (`rm -f ~/.local/share/com.natkins.scribe/wayland-portal.json`), start Scribe and dictate into the editor; this time **deny** the remote-control dialog.
    - **Expected:** a message ending "Copied. Press Ctrl+V to paste.", and `Ctrl+V` pastes your words.
16. Dictate again: no dialog this time, the text is copied again.
17. On **Dictation**, the **Paste into Wayland apps** row offers **Ask again**.
    Click it, allow the dialog, and dictate: the text is pasted.

### X11 session (log out and in to an X11 session)

18. Hotkeys and paste work in GNOME Text Editor or Kate, Firefox and the terminal, with no dialogs at all.
19. **After dictation** on Home says **X11 paste**.

## What to send back

Open an [issue](https://github.com/n8watkins/scribe/issues/new) titled "Wayland test: GNOME 47 on Ubuntu 24.10" (or your desktop), with:

- The versions from "Before each run" (session type, desktop version, portal version), and your distribution.
- The check numbers with pass, fail or skip, and notes.
- The screenshots: the hotkey dialog, the remote-control dialog, the **Hotkeys** page and anything that looked wrong.
- **About → Copy diagnostics**, pasted in.
- The newest log from `~/.local/share/com.natkins.scribe/logs`; the lines with "portal", "Wayland" or "RemoteDesktop" are the interesting ones.
