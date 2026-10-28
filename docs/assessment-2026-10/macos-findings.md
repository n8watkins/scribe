# Experimental macOS build: findings

Tested 2026-10-28 in the "macOS (experimental)" workflow on PR #60, on GitHub's `macos-latest` runner (macOS 26.6.2, Apple M1 virtual machine with 3 cores and a paravirtual GPU).
No Mac hardware was used: everything below comes from CI logs, and the last section lists what still needs a person on a real Mac.

## What builds

- whisper.cpp is built from the same reviewed source as Windows (v1.9.1, `f049fff`), checked against the pinned commit before the build.
- The two whisper.cpp binaries are statically linked and need only system libraries and frameworks (Metal, Accelerate, Foundation); the job fails if `otool -L` shows anything else.
- The Apple Silicon slice has Metal on, with the shaders embedded in the binary (`GGML_METAL_EMBED_LIBRARY`), so there is no `default.metallib` to ship or find.
- The Intel slice is CPU only (Accelerate plus AVX2, FMA and F16C), which matches upstream's own x86_64 macOS builds.
  With Metal on, the Intel slice returned an empty transcript under Rosetta on the runner's paravirtual GPU, while the CPU path transcribed correctly, and Metal on a real Intel Mac GPU cannot be checked without one.
- The two slices are joined with `lipo`, and the Tauri app is built with `--target universal-apple-darwin`, so the `.app` and the `.dmg` are universal (arm64 and x86_64).
- The minimum macOS is 13.3, set both for whisper.cpp (`MACOSX_DEPLOYMENT_TARGET`) and for the app (`minimumSystemVersion`); the job checks that every slice's `minos` matches.
  13.3 is the floor because ggml's Accelerate path calls `cblas_sgemm` from the new LAPACK interface, which older macOS lacks, and it is also upstream's floor.
  Every Intel Mac that runs macOS 13 has AVX2, so the Intel slice runs on all of them.
- The macOS bundle settings live in `app/src-tauri/tauri.macos.conf.json`, which Tauri merges only when it builds on macOS, so the Windows and Linux bundles are unchanged.
- The app finds its whisper.cpp binaries under `Scribe.app/Contents/Resources/bin/macos/`, the same layout as `bin/windows/` and `bin/linux/`.
- Sizes: `Scribe.app` is 59 MiB and `Scribe_<version>_universal.dmg` is 21 MiB.
- Timing on the runner: whisper.cpp for both slices takes about 2 minutes, the two Rust release builds about 4 minutes each, and the whole job about 14 minutes from cold.
  With the whisper.cpp and cargo caches warm the job takes about 7 minutes.

## Signing and Gatekeeper

- There is no Apple Developer ID, so everything is ad-hoc signed (`codesign -s -`), which Apple Silicon requires before it runs any code.
- The whisper.cpp binaries are signed on their own before bundling, and Tauri signs the app bundle with the `-` identity from `tauri.macos.conf.json`.
- Hardened runtime is off: it only matters for notarization, and with it on the app would also need entitlements (for example the microphone) to work.
- `codesign --verify --deep --strict` passes for the app and for each whisper.cpp binary, and `codesign --display` shows `Signature=adhoc` and `TeamIdentifier=not set`.
- `spctl --assess` rejects both the app and the `.dmg` (`source=no usable signature`), as expected without notarization; the job records this and does not fail on it.
- Tauri skips notarization with a warning because no Apple credentials are set.
- A `.dmg` downloaded in a browser gets the quarantine attribute, so Gatekeeper will block the first launch.
  On macOS 15 and later the Control-click "Open" bypass is gone: the user must try to open the app once, then choose "Open Anyway" in System Settings, Privacy & Security, or run `xattr -dr com.apple.quarantine /Applications/Scribe.app`.
  The CI test copies the app from a locally built `.dmg`, which carries no quarantine attribute, so it does not exercise this path.

## Self-test result

The job mounts the `.dmg`, copies `Scribe.app` into a temporary Applications folder, and runs `Scribe --self-test-transcribe jfk.wav result.json` under a 10 minute timeout.

- Apple Silicon (native): the app started, downloaded the default model (`small.en-q5_1`) through its verified download path, and transcribed the 11 s clip as "And so, my fellow Americans, ask not what your country can do for you. Ask what you can do for your country."
  The whole run took 23 to 38 s across runs, of which the transcription was 18 to 30 s.
- Intel (the x86_64 slice under Rosetta, `arch -x86_64`, model already present): same transcript, 20 s for the transcription and 35 s for the whole run.
- `whisper-cli` on its own, same clip and model: arm64 with Metal 5 to 11 s, arm64 CPU only about 3 s, x86_64 under Rosetta 15 to 19 s.
  The runner's paravirtual GPU (no simdgroup support) is slower than its CPU, so these numbers say nothing about Metal speed on a real Mac.
- The app logs one expected error at startup: the Hold-to-Talk default (Ctrl+Win) cannot be registered because modifier-only shortcuts are Windows-only.
  The other four default hotkeys registered without any permission prompt.

## What is missing for real Mac support

1. Microphone access: audio capture (`cpal`) is compiled for Windows only, so recording, dictation, notes and the level test do not work.
   `NSMicrophoneUsageDescription` is already in `app/src-tauri/Info.plist` (without it macOS ends the app the first time it opens the microphone), and the workflow checks that it reaches the bundle.
2. Paste, Direct Insert and selection capture: the macOS platform layer (`src/platform/macos.rs`) is a stub that reports them as unsupported.
   A real one will post key events (CGEvent), which needs the Accessibility permission, and a Hold-to-Talk key listener needs Input Monitoring.
   Neither can be granted to an ad-hoc signed app permanently: macOS ties the grant to the code signature, and an ad-hoc signature changes with every build, so users would have to grant it again after each update.
3. Hotkeys: modifier-only Hold-to-Talk is Windows-only, and the Windows-style defaults (Ctrl+Win, bare backtick) need macOS equivalents.
4. Updates: `latest.json` has only `windows-x86_64`, and the macOS build produces no updater artifacts, so a macOS install cannot update itself.
   A `darwin-aarch64` and `darwin-x86_64` (or `darwin-universal`) entry pointing at a signed `.app.tar.gz` would be needed.
5. The GitHub token store uses `keyring` with only its Windows backend, so on macOS the token would not survive a restart; the `apple-native` feature would fix that.
6. The GPU probe is Windows (Vulkan) only, so the GPU section cannot show the Metal device, even though whisper.cpp uses Metal by default on Apple Silicon.
7. The pill window asks for a transparent background, which Tauri supports on macOS only with `macOSPrivateApi` (and that blocks Mac App Store distribution).
8. UI copy still talks about Windows (for example "Private local dictation for Windows").
9. Distribution without Gatekeeper prompts needs a paid Developer ID and notarization, which the owner has decided against for now.

## What a person must check on a real Mac

- Download the `.dmg` artifact in a browser, install it, and note exactly what Gatekeeper shows and which steps open the app.
- The main window, the tray icon in the menu bar, the pill window (its transparency and position) and closing to the tray.
- File transcription from the UI, with Metal on and off, and the real speed on an Apple Silicon Mac.
- The same `.dmg` on an Intel Mac, if one is available.
- Whether registering global hotkeys triggers any permission prompt, and whether they fire while other apps are in front.
- Once the platform and audio work lands: the microphone prompt and its wording, the Accessibility and Input Monitoring prompts, and whether those grants survive installing a newer build.
