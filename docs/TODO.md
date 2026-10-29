# Scribe to-do (as of 2026-10-29)

The working list for picking the project back up in a fresh session.
Released: v0.11.0 (Windows).
Merged on `main` since v0.11.0: Phase 4 batch 1 (#70-#73, #75, #76), all Linux/macOS work that leaves Windows behavior unchanged.

Read first: `docs/assessment-2026-10/DECISIONS.md` (owner decisions), `linux-findings.md` and `macos-findings.md` (parts are now outdated, see 6.1).
Standing rules: Conventional Commits under 100 chars, no Co-Authored-By trailer, no em dash, never edit CHANGELOG.md, one PR per change, squash-merge with an explicit subject and body after green CI, re-run CI on current `main` before merging parallel PRs, never change users' saved hotkeys, no paid code signing.

## 1. Next release (v0.12.0)

- [ ] Decide when to release the Phase 4 batch 1 work. Windows users see only small changes (platform-specific wording code paths, unchanged Windows text), so it can wait for more Windows-facing work or go out as a minor release.
- [ ] Before releasing, run the Windows harness once on the Dev build (see 7.1) to confirm paste, type-out, Transform and the badge still pass after the platform-copy and keyring changes.
- [ ] Release with `npm run release -- minor --push --summary "..."`, then verify: release assets, `SHA256SUMS.txt`, the updater signature against the pubkey in `tauri.conf.json`, `latest.json`, and the website version (refreshes hourly).

## 2. Linux (Phase 4)

### 2.0 Full dictation test in GitHub Actions

- [ ] In `linux.yml`: run the AppImage under Xvfb (X11) and under a headless Wayland compositor (weston or sway headless), with a PulseAudio/PipeWire null sink as the microphone fed from speech WAVs.
- [ ] Same flow as the macOS test (3.0): hotkey via XTEST, dictate, check the paste into a test window (a small GTK or xterm target) and the clipboard restore, type-out, paste guard with a second app, Transform with the stand-in LLM, discard.
- [ ] Wayland run: today it should show the copy fallback message; once 2.1 lands it becomes the portal paste test.
- [ ] Screenshots uploaded as artifacts.

### 2.1 Must-have for a usable Linux build
- [ ] **Wayland paste and typing.** Today paste works only through X11/XWayland (#64). Options: XDG RemoteDesktop portal (keyboard injection with a one-time user consent), `wtype` (wlroots compositors only), `ydotool` (needs uinput access). Recommendation: portal first, fall back to copy with a clear message.
- [ ] **Wayland global shortcuts.** Hotkeys are X11 grabs, so on native Wayland they fire only while an X11 window has focus. Use the XDG GlobalShortcuts portal (GNOME 48+, KDE 5.27+), and keep X11 grabs as the fallback.
- [ ] **Transform Selection on Wayland**: currently falls back to the typed panel; revisit once the portal paste path exists.
- [ ] **Updates for Linux.** Add a signed AppImage updater artifact (`createUpdaterArtifacts` is false in `tauri.linux.conf.json`) and a `linux-x86_64` entry in `latest.json` from `release.yml`. Only after 2.1 works end to end, so nobody is auto-updated onto a half-working build.
- [ ] **About page wording on Linux** says updates install on launch, which is false until the item above lands. Fix with it, or hide the auto-install toggle on Linux until then.

### 2.2 Should-have
- [ ] Hold-to-talk with a modifier-only chord (Ctrl+Win style) on Linux needs a key watcher (evdev needs input-group access; X11 XInput2 raw events work without it). Today Linux defaults to Ctrl+Alt+Space, which works.
- [ ] Hold-and-tap-Q note chord is Windows-only (`noteChordSupported`); decide whether Linux needs it.
- [ ] GPU acceleration: the Linux whisper.cpp build is CPU-only and the GPU probe is Windows-only. Vulkan on Linux would mirror Windows.
- [ ] Tray icon and close-to-tray on a real desktop (WSLg has no tray, so this was never checked).
- [ ] Pill and toast draw as an opaque rectangle without a compositor; acceptable, but consider a non-transparent fallback style.
- [ ] Launch at startup on Linux: verify the autostart `.desktop` entry is written and works for AppImage (path changes when the AppImage moves) and `.deb`.
- [ ] Check the AppImage on an older distro (built on the oldest runner for glibc) and on Fedora/Arch, not only Ubuntu.

### 2.3 Needs a real Linux desktop (owner or a VM)
- [ ] Dictate and paste into GNOME and KDE apps on X11 and on Wayland.
- [ ] GitHub backup: connect, restart, still connected (Secret Service, #73) on GNOME Keyring and KWallet.
- [ ] Microphone selection with PipeWire and PulseAudio.

## 3. macOS (Phase 4)

### 3.0 Full dictation test in GitHub Actions (do this first)

The macOS runner already has a logged-in session with Accessibility granted, which is how #72's TextEdit paste test works.
Build on that to cover most of the hands-on checklist automatically, in `macos.yml` (one PR, then grow it):

- [ ] Install a virtual audio device on the runner (BlackHole via Homebrew, free) and make it the default input, so recorded speech WAVs can be "spoken" into Scribe.
- [ ] Launch the full app (not just `--self-test-transcribe`) with a throwaway profile, `status.json` to wait on state (as the Windows harness does), and Save-only off.
- [ ] Toggle dictation with the real default hotkey (Control+Option+Backquote posted as a CGEvent), play a WAV, stop, and check the transcript lands in a TextEdit document and the previous clipboard (text, then an image, then a file URL) is restored.
- [ ] Hold-to-talk: hold Control+Option+Z while the WAV plays, release, check the paste.
- [ ] Direct Insert (type out) and Copy-only modes.
- [ ] Paste guard: switch to another app while transcribing, check nothing is pasted and the text is left on the clipboard with the Cmd+V message.
- [ ] Paste last (Control+Option+V) with the dashboard in front: the app below gets focus and the text.
- [ ] Transform Selection on a TextEdit selection, with a stand-in OpenAI-compatible server on localhost that upper-cases the text (like the Windows test's `fake_llm.py`).
- [ ] Discard (Control+Option+X) leaves no clip; silence auto-stop.
- [ ] Other targets: Terminal (and iTerm via Homebrew cask), a Safari text field if automatable.
- [ ] Screenshots (`screencapture`) of the main window, the pill (transparency), the result toast and the Accessibility row, uploaded as workflow artifacts for a visual review.
- [ ] Update path: install the previous release, update to the new build, check whether the Accessibility grant still applies. The runner's TCC setup is special, so treat the result as a hint, not proof.
- [ ] Run it on PRs that touch macOS code, the platform layer, audio, output or hotkeys, and on release builds.

What CI cannot answer (still needs a real Mac, see 3.1): Gatekeeper's first-launch flow after a browser download, the real permission prompts (runners come pre-granted), whether grants survive updates under normal TCC, real Metal speed, and apps that need a sign-in (Slack).

### 3.1 Must-have
- [ ] **Hands-on test on a real Mac** (the owner has none; options: borrow one, a cloud Mac such as MacStadium/AWS EC2 Mac (paid), or accept CI-only). Checklist is in `macos-findings.md` "What a person must check" plus: dictate into TextEdit, Safari, Slack, Terminal, iTerm with Paste, Copy+Paste and Direct Insert; clipboard (text, image, Finder file) comes back after paste; switching apps mid-dictation skips the paste with the Cmd+V message; Paste last (Control+Option+V) from the dashboard; Transform on a TextEdit selection; the Accessibility row and button (#76); the microphone prompt; hotkeys fire while other apps are in front (#71).
- [ ] **Accessibility grant after updates.** Ad-hoc signing changes the code signature each build, so the grant probably stops applying after every update. Confirm on a Mac, then decide: document it (row text already warns), detect a stale grant and explain "turn Scribe off and on again", or reconsider a free stable signing identity (a self-signed certificate kept in CI secrets gives a stable designated requirement without paying Apple; check whether TCC keeps the grant with it).
- [ ] **Keychain prompt after updates** (#70): same signature issue, macOS may ask for the login password once per update. Confirm on a Mac.
- [ ] **Updates for macOS.** `darwin-aarch64` and `darwin-x86_64` (or universal) entries in `latest.json`, with a signed `.app.tar.gz` from `macos.yml`/`release.yml`. Only after 3.1 hands-on testing passes.
- [ ] **Distribution page**: Gatekeeper steps for an unsigned app (macOS 15+: try to open, then System Settings, Privacy & Security, "Open Anyway"; or `xattr -dr com.apple.quarantine /Applications/Scribe.app`). Put this on the website download section when Mac downloads go live.

### 3.2 Should-have
- [ ] Pill transparency: needs `macOSPrivateApi` in Tauri (blocks Mac App Store, which is not planned). Turn it on in `tauri.macos.conf.json` and check the pill.
- [ ] GPU section: the probe is Vulkan/Windows-only; show the Metal device on Apple Silicon (whisper.cpp already uses Metal there).
- [ ] Paste shortcut on Dvorak: Command+V/C are sent as ANSI key codes, which is wrong on Dvorak. Map through the active keyboard layout (Text Input Sources, main thread).
- [ ] Modifier-only Hold-to-talk on macOS (CGEventTap; needs Input Monitoring permission, same signature problem as Accessibility).
- [ ] Launch at startup on macOS (tauri-plugin-autostart LaunchAgent): verify.
- [ ] Notification permission prompt on macOS: verify wording and behavior.
- [ ] Intel Macs: the x86_64 slice is CPU-only; Metal on a real Intel GPU is untested.

## 4. Cross-platform

- [ ] README, FAQ and website still describe a Windows-only app. Update when Linux or macOS downloads are offered, not before.
- [ ] Website download section: add Linux/macOS downloads (from the latest release assets) once they are in releases; `scribe-web/lib/site.ts` reads only the `x64-setup.exe` asset today.
- [ ] Website fallback release in `scribe-web/lib/site.ts` still points at 0.9.0 (used only when GitHub is unreachable). Bump it to the current release.
- [ ] `release.yml` builds Windows only. Add Linux and macOS jobs (reusing `linux.yml`/`macos.yml`) that upload to the same release, when 2.1/3.1 are done.
- [ ] Dependabot alert #86 (`glib` 0.18, Linux GTK stack, dismissed as tolerable): revisit when Tauri moves to gtk-rs 0.20.

## 5. Windows follow-ups

- [ ] Clipboard robustness ideas from the 2026-10-28 investigation (no bug found, optional hardening): own a hidden HWND as the clipboard owner instead of NULL, and skip synthesized and OLE formats when restoring.
- [ ] Dev build `status.json` reports `appVersion` "0.10.0"/base version for Dev builds; minor, make it the real Dev version.
- [ ] CI Windows paste smoke test: a GitHub Actions Windows runner job that pastes into a test window (the hands-on harness proves the approach; see 7.1).
- [ ] Defender ML flags the unsigned Dev NSIS installer (`Trojan:Win32/Bearfoos.A!ml`): owner said to ignore it. No action.
- [ ] DECISIONS.md "Still open": whether a note's audio should expire after 30 days while its text stays.
- [ ] DECISIONS.md "Found while answering": confirm the stale data cleanup (old LocalDictate folders, orphaned clips, duplicate models, the old Dev profile) was done, or do it with the owner's OK.
- [ ] Old remote branch `worktree-agent-a4954df2133cd0a2f`: keep until the owner OKs deleting it.
- [ ] Wave plan items not yet done (lanes A core, B interface, C trust/storage, W website): see the wave plan artifact https://claude.ai/artifact/53uSboHoDzAqHAofts9ek2 and the assessment report https://claude.ai/artifact/3sYEzCHfvR3pTWUDC4qPKL, and tick off what is merged.

## 6. Docs

- [ ] 6.1 Update `linux-findings.md` and `macos-findings.md`: recording, X11 paste, hotkeys, keyring and wording are done (#63-#66, #70-#76); keep the remaining gaps only.
- [ ] Write a short `docs/PLATFORMS.md`: what works on each OS, which permissions each needs, and how to test each.

## 7. Tooling and environment

- [ ] 7.1 Windows hands-on harness lives outside the repo in `C:\Users\natha\AppData\Local\Temp\scribe-test\` (backup copy with `devctl.sh`, `fake_llm.py` and the WAV clips: `~/projects/tools/scribe/windows-harness/`; the working set is `run.ps1`, `go.ps1`, `devstart.ps1`, `devshow.ps1`, `targetb.ps1`, `pw.ps1`, the rest are older experiments) (`go.ps1` watchdog, `run.ps1`, `devstart.ps1`, `targetb.ps1`); Temp can be wiped. Move it into the repo (for example `tools/windows-harness/`) with a README, so it survives and CI can reuse it. Rules: never tap Alt to take focus, click inside the harness window instead; announce start and end; save and restore the owner's clipboard; window B must be a separate process.
- [ ] The agent `isolation: "worktree"` hook fails ("hook succeeded but returned no worktree path"); create worktrees by hand with `git worktree add ../wt-<name> -b <branch> origin/main` until the hook is fixed.
- [ ] Stacked PRs: retarget them to `main` before deleting the base branch, or GitHub closes them for good (#74 became #76).
- [ ] WSL memory: 20 GB cap shared by several sessions; the `wsl-cache-trim` timer trims file cache every 2 minutes. Check `free -m` before cargo builds and use low `-j`; macOS work compiles in CI only.
