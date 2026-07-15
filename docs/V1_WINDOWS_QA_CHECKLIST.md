# V1 Windows QA Checklist

## Release resources

- Verify the release workflow builds the pinned whisper.cpp source with Vulkan enabled.
- Verify `whisper-cli.exe`, `whisper-server.exe`, `whisper.dll`, `ggml.dll`, `ggml-base.dll`, `ggml-cpu.dll`, and `ggml-vulkan.dll` are present under `$RESOURCE/bin/windows/`.
- Do not ship with placeholder or unreviewed executable files.
- Confirm Whisper models download to the backend-resolved app data `models` directory.

## Installer and release integrity

- Build on Windows with `cd app && npm run tauri build`.
- Verify NSIS current-user install.
- Verify MSI install and upgrade behavior.
- Confirm the unsigned NSIS and MSI installers produce the expected Windows SmartScreen warning; Authenticode code signing is intentionally out of scope.
- Verify the updater signature before publishing release metadata.
- Verify `SHA256SUMS` against every staged release asset.
- Verify the release software bill of materials is present.
- Smoke-test clean NSIS and MSI installs, launch, uninstall, and removal before publishing the release.
- Record the stable MSI upgrade code before publishing updates.

## Manual acceptance

- Install and launch on Windows 10 and Windows 11.
- Confirm the app runs from the tray and closing the main window hides it when `minimizeToTray` is enabled.
- Select a microphone from the Audio view.
- Download, select, delete, and retry a Whisper model from the Models view.
- Hold the dictation hotkey, release to transcribe, and verify the Last Transcript Buffer updates.
- Paste the last transcript with `Ctrl+Alt+V`.
- Verify Save Only, Auto Paste, Copy to Clipboard, and Copy + Paste modes.
- Confirm clipboard restore behavior in compatibility paste mode.
- Confirm transcript history, stats, search, edit, and delete refresh without restarting the app.
- Confirm the floating pill appears for recording, transcribing, ready, and error states.
- Confirm in-app notifications follow the Notifications setting.

## Historical V1 polish notes

The items below describe the original V1 state and are retained only as historical acceptance context.
They are not current gaps.

- Native OS notifications are not wired in V1; current feedback is in-app toast plus floating pill.
- Launch-at-startup requires adding and wiring the Tauri autostart plugin.
- Tray icon state variants need final art assets before replacing the default icon.
- Open data/model folder commands are still disabled in the frontend.
