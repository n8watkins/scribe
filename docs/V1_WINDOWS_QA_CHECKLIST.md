# V1 Windows QA Checklist

## Release resources

- Place the real whisper.cpp binary at `app/src-tauri/resources/bin/windows/whisper-cli.exe`.
- Verify the bundled runtime path resolves to `$RESOURCE/bin/windows/whisper-cli.exe`.
- Do not ship with a placeholder executable.
- Confirm Whisper models download to the backend-resolved app data `models` directory.

## Installer and signing

- Build on Windows with `cd app && npm run tauri build`.
- Verify NSIS current-user install.
- Verify MSI install and upgrade behavior.
- Add production code signing before external distribution.
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

## Remaining polish

- Native OS notifications are not wired in V1; current feedback is in-app toast plus floating pill.
- Launch-at-startup requires adding and wiring the Tauri autostart plugin.
- Tray icon state variants need final art assets before replacing the default icon.
- Open data/model folder commands are still disabled in the frontend.
