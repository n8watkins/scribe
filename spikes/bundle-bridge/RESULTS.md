# Bundle bridge spike: results

Workflow: `.github/workflows/bundle-bridge-spike.yml`, run on GitHub's `windows-latest` runners from pull request #140.
Results below are from run https://github.com/n8watkins/scribe/actions/runs/45753456488 (commit 08e6f63).
The per-check log of every run is in the job summary and in the `bundle-bridge-results` artifact.

## Verdict

**Go**, by route A (`tauri bundle`), with one installer change required for the switch release.

A non-Tauri `Scribe.exe` packaged by `tauri bundle` with the real `tauri.conf.json` identity and the real hooks file updates installed 0.16.1, 0.16.2 and 0.16.3 copies in place with every argument form the updater uses.
Same folder, one Installed apps entry at 0.99.0 from n8builds.dev, the Run value and shortcuts still valid, data untouched, and `/R` relaunches the new exe with the old arguments.

The one gap: the Tauri build's sidecars (`bin\windows\*`, ten files) stay in the install folder after the update and after an uninstall, because the native installer does not list them.
A two-command POSTINSTALL addition to the hooks (`RMDir /r "$INSTDIR\bin\windows"`, `RMDir "$INSTDIR\bin"`) closes it, and the variant built with it passes every check.
The run stays red on purpose: the scenarios with the unchanged hooks report that gap as failures.

## Packaging route

Route A, `tauri bundle --bundles nsis,msi --no-sign` (Tauri CLI 2.12.4, tauri-bundler 2.10.4), works on a plain Rust binary.
It needs only a `tauri.conf.json` next to a `Cargo.toml` whose binary is `target/release/Scribe.exe`; it never checks for the `tauri` crate.
The config is derived at run time from the real one (`scripts/make-config.ps1`): version 0.99.0, `webviewInstallMode` skip, no `resources`, no updater artifacts, absolute icon and hooks paths, everything else as is.
The only complaint is a warning: `__TAURI_BUNDLE_TYPE variable not found in binary`.
That marker tells a Tauri app's updater which installer it came from; the native updater will not have it, so the native app must decide NSIS or MSI another way (it only matters if `latest.json` ever lists both).
The fallback route (Tauri's NSIS template driven by makensis directly) was not needed and was not built.

## NSIS: assertions

Each scenario installs a published setup with `/S` (or `/S /D=<folder>`), seeds what a real install has (Run value `Scribe` = `"<exe>" --minimized`, files in `%APPDATA%\com.natkins.scribe` and `%LOCALAPPDATA%\com.natkins.scribe`), then runs the native setup.
Argument forms, from tauri-plugin-updater 2.14.1 with `installMode` quiet:

- `/S /UPDATE` (restart_after_install off).
- `/S /UPDATE /R /ARGS` (the default, app opened from the Start menu).
- `/S /UPDATE /R /ARGS --minimized` (the default, app started at sign-in).

Scenarios: each of 0.16.1, 0.16.2 and 0.16.3 with each form in the default folder (nine), 0.16.1 and 0.16.3 in a custom folder `C:\ScribeCustom\Scribe`, and the cleanup variant for 0.16.1, 0.16.2, 0.16.3 (default folder) and 0.16.1 (custom folder).

515 checks passed and 11 failed; the 11 failures are one assertion, in each of the 11 scenarios with the unchanged hooks.

| Assertion | Real hooks (11 scenarios) | Hooks + sidecar cleanup (4 scenarios) |
| --- | --- | --- |
| Old setup and native setup exit 0 | pass | pass |
| Scribe.exe stays in the same folder (default or custom), no second copy in the default folder | pass | pass |
| Scribe.exe is the native build: ProductVersion 0.99.0, CompanyName n8builds.dev, hash differs from the old exe | pass | pass |
| Exactly one Installed apps entry, version 0.99.0, publisher n8builds.dev, uninstaller in the same folder | pass | pass |
| `Software\natkins\Scribe` gone, `Software\n8builds.dev\Scribe` records the folder (0.16.1 custom-folder adoption included) | pass | pass |
| Run value unchanged, points at an existing exe, and that exe is the native one | pass | pass |
| Start menu shortcut `Scribe\Scribe.lnk` (and the desktop shortcut) present and pointing at the native exe | pass | pass |
| Both data folders byte for byte unchanged | pass | pass |
| `/S /UPDATE`: no relaunch | pass | not run |
| `/R /ARGS`: relaunches the native exe from the install folder, with `--minimized` exactly when the old app had it | pass | pass |
| The native exe starts from the Run value command with the Tauri leftovers present | pass | pass (no leftovers) |
| Tauri files left after the update | ten files in `bin\windows` (whisper and parakeet sidecars, ggml DLLs, README.md) | none |
| Uninstall exits 0, Scribe.exe removed | pass | pass |
| Uninstall leaves no files in the install folder | **fail**: the ten `bin\windows` files stay | pass |
| Uninstall removes the Installed apps entry, both hook-managed keys (`n8builds.dev` and `natkins`), the Run value and both shortcuts | pass | pass |

The 0.16.x installers put no WebView2 loader DLL or other Tauri runtime file next to the exe (their file list is `Scribe.exe`, `uninstall.exe` and `bin\windows\*`), so the only leftovers are the sidecars from `resources`.
The installer ran with the runner elevated and one explorer.exe present.

## MSI: assertions

All 32 checks passed.
The update was run as the plugin runs an MSI update: `msiexec.exe /i <msi> /quiet /promptrestart AUTOLAUNCHAPP=True LAUNCHAPPARGS="--minimized"`.

| Assertion | Outcome |
| --- | --- |
| The 0.16.3 UpgradeCode equals the v5 UUID derived below, and the native MSI has the same one | pass |
| The native MSI has a new ProductCode and Manufacturer n8builds.dev | pass |
| 0.16.3 MSI installs to `C:\Program Files\Scribe` with its Start menu shortcut | pass |
| Native MSI exits 0 and upgrades in place: one Installed apps entry, 0.99.0, n8builds.dev | pass |
| Scribe.exe replaced in the same folder; the major upgrade removed every 0.16.3 file (only Scribe.exe and the template's `Uninstall Scribe.lnk` remain) | pass |
| Run value unchanged and valid; Start menu shortcut points at the native exe; data folders unchanged | pass |
| `AUTOLAUNCHAPP` relaunches the native exe with `--minimized` | pass |
| Uninstall removes the folder, the entry, the shortcut and the MSI's `InstallDir` value | pass |

So the MSI upgrades in place, not side by side.

### How Tauri derives the UpgradeCode

tauri-bundler 2.10.4 (`src/bundle/windows/msi/mod.rs`) uses `bundle.windows.wix.upgradeCode` when set, and otherwise `Uuid::new_v5(&Uuid::NAMESPACE_DNS, "<productName>.exe.app.x64")`.
For Scribe that is `{ADADDFC5-2E78-5900-BB3A-8E41A1CFDD54}`, which is the UpgradeCode read from the published 0.16.3 MSI and from the native MSI.
It depends on `productName` only: not on the identifier, the publisher or `mainBinaryName`.
The ProductCode is new on every build (`Id="*"`), so the native MSI is a major upgrade (`MajorUpgrade Schedule="afterInstallInitialize"`) and replaces 0.16.3 in place.
The `Path` component GUID (the exe) is a v5 UUID of the identifier `com.natkins.scribe` in a tauri-bundler namespace, so it is stable too.

## Signature

All 11 checks passed.
A throwaway key pair is made in the job with `tauri signer generate`; the real key is never read.
`verify/` reimplements the plugin's check line for line (tauri-plugin-updater 2.14.1 `verify_signature` and `verify_signed_version`, minisign-verify 0.3).

| Assertion | Outcome |
| --- | --- |
| `tauri signer sign <setup>` with the key in `TAURI_SIGNING_PRIVATE_KEY`, as `release.yml` runs it, writes `<setup>.sig` | pass |
| The plugin check accepts that signature, taken from a `latest.json` built the way `release.yml` builds it, with `requireSignedVersion` off (Scribe's setting) | pass |
| The release-form trusted comment is `timestamp:<n>`, `file:<name>`, with no version, so the same check with `requireSignedVersion` on refuses it | pass (refused as expected) |
| `tauri signer sign --app-version 0.99.0` is accepted with `requireSignedVersion` on, and refused when the manifest announces 0.99.1 | pass |
| A setup.exe with one byte changed is refused | pass |
| The real public key (the one installed copies trust) refuses a setup signed by any other key | pass |
| Control: the published 0.16.3 setup.exe and its published .sig pass the same check against the real public key | pass |

So the release pipeline's signing step fits the native setup unchanged.

## Uninstall and a clean runner

With the sidecar cleanup, an NSIS uninstall leaves the install folder empty and removes the Installed apps entry, `HKCU\Software\n8builds.dev\Scribe`, `HKCU\Software\natkins\Scribe`, the Run value and both shortcuts.
Without it, everything else is removed but the ten `bin\windows` files keep the folder alive.
After the last NSIS scenario the runner has no Scribe entry, neither key, no Run value and no Scribe process; after the MSI scenarios there is no entry and no Program Files folder.

## What could not be determined here

- The real updater flow end to end: the spike runs the installer with the plugin's argument strings, but no 0.16.x app downloaded the update from a feed (that would need a feed signed with the real key, or a rebuilt 0.16.x trusting a throwaway key).
- A running old app at update time: the plugin calls `std::process::exit(0)` right after starting the installer, and the installer's own kill step (`CheckIfAppIsRunning`, plus the hooks' whisper-server kill) was not exercised against a live 0.16.x process.
- `/R` relaunch was proven on a runner where the job is elevated and explorer.exe runs; `nsis_tauri_utils::RunAsUser` takes a different path for a non-elevated user (the usual case on a PC) and that path was not exercised.
- The MSI was installed and upgraded elevated (the runner is admin); on a PC a per-machine MSI update from inside the app needs a UAC prompt.
- SmartScreen, Authenticode and antivirus behaviour for an unsigned native exe.

## What must stay byte-compatible in the real switch release

- `productName` `Scribe`: the NSIS Uninstall key name, the default folder `%LOCALAPPDATA%\Scribe`, the Start menu shortcut name, and the MSI UpgradeCode (or pin `bundle.windows.wix.upgradeCode` to `ADADDFC5-2E78-5900-BB3A-8E41A1CFDD54` before anything renames the product).
- `mainBinaryName` `Scribe`: the exe path the Run value and shortcuts point at; a different name makes the template delete the old exe and leaves the Run value broken.
- `identifier` `com.natkins.scribe`: the data folders the uninstaller and the app use, the shortcuts' AppUserModelID, the MSI exe component GUID.
- `publisher` `n8builds.dev`: the `Software\n8builds.dev\Scribe` folder key, the Installed apps publisher, and how the NSIS installer finds an MSI install (DisplayName plus Publisher).
- `bundle.windows.nsis`: `installMode` currentUser, `startMenuFolder` Scribe, and `installerHooks` (the folder adoption from the natkins key and the whisper-server kill are still needed for 0.16.1 copies), plus the sidecar cleanup above.
- A version above every published one, and `allowDowngrades` false.
- The updater feed: `latest.json` at the same URL with the `windows-x86_64` entry pointing at the NSIS setup, signed by the real key whose public half is in `plugins.updater.pubkey`; the native app must embed that same public key to verify its own later updates.
- The Run value name `Scribe` and the `--minimized` flag: the native app must read the flag and keep writing the same value (and repair one that points at another copy, as `autostart::reconcile_at_launch` does today).

## Findings outside the bridge itself

- An MSI install updated through the feed gets the NSIS setup (the only installer in `latest.json`), which installs a second copy in `%LOCALAPPDATA%\Scribe` beside the MSI copy in Program Files: two Installed apps entries, and the Run value still starts the old MSI copy.
  The NSIS template's MSI migration only runs on its interactive reinstall page, which `/S` skips.
  This is how 0.16.x behaves today and is not caused by the native build, but the switch is a good moment to fix it (for example a hook that finds the MSI product by UpgradeCode and removes it, or an MSI entry in `latest.json`).
- The WiX template never removes the Run value on uninstall; the NSIS uninstaller does.
- Silent uninstall keeps both data folders (the delete-data checkbox defaults off), which matches the current app.
- `release.yml` signs without `--app-version`, so the trusted comment carries no version and `requireSignedVersion` cannot be turned on; adding `--app-version` to the signing step would allow it.
