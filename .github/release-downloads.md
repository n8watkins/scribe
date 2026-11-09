## Downloads

| System | File |
|---|---|
| Windows 10 and 11 | `Scribe_<version>_x64-setup.exe` (recommended) or `Scribe_<version>_x64_en-US.msi` |
| macOS 13.3 or later, Apple Silicon and Intel (beta) | `Scribe-macos-universal.dmg` |
| Linux x86_64 (beta) | `Scribe-linux-amd64.deb` for Ubuntu 22.04+ and Debian 12+, or `Scribe-linux-x86_64.AppImage` |

`SHA256SUMS.txt` lists a checksum for every file.
Install guides: [Windows](https://github.com/n8watkins/scribe/blob/main/docs/install/windows.md), [macOS](https://github.com/n8watkins/scribe/blob/main/docs/install/macos.md), [Linux](https://github.com/n8watkins/scribe/blob/main/docs/install/linux.md).

**The macOS and Linux builds are betas.**
They pass the same automated install, launch and transcription checks as Windows, but have not yet been tried on many real Macs and Linux desktops.
Please report what works and what doesn't in [Issues](https://github.com/n8watkins/scribe/issues); the [testing checklist](https://github.com/n8watkins/scribe/blob/main/docs/install/testing-checklist.md) says what to check.

**macOS: the app is not notarized.**
Scribe has no Apple Developer ID, so the app is ad-hoc signed and macOS blocks it the first time you open it.
Drag Scribe to Applications, try to open it once, then open System Settings, Privacy & Security, and click **Open Anyway** next to the Scribe message.
On macOS 13 and 14 you can instead Control-click (right-click) Scribe in Applications and choose **Open**.
If macOS says the app "is damaged and can't be opened", run `xattr -dr com.apple.quarantine /Applications/Scribe.app` in Terminal and open it again.

**Linux:** install the .deb with `sudo apt install ./Scribe-linux-amd64.deb`.
The AppImage needs FUSE 2 (`sudo apt install libfuse2t64` on Ubuntu 24.04, `libfuse2` on 22.04); then `chmod +x Scribe-linux-x86_64.AppImage` and run it.
Global hotkeys and pasting work on X11; on a Wayland session they only reach X11 (XWayland) apps for now, and elsewhere Scribe copies the text for you to paste.

**Updates:** Windows installs update themselves.
macOS and Linux installs don't yet: About tells you when a new version is out; download it from the [latest release](https://github.com/n8watkins/scribe/releases/latest) and install it over the old one.
