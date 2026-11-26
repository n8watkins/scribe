# NSIS bridge: published Tauri installs (0.16.1, 0.16.2, 0.16.3) updated by
# the native build's setup.exe, run with the exact argument forms
# tauri-plugin-updater 2.14.1 uses (installMode "quiet"):
#   restart_after_install = false        -> /S /UPDATE
#   restart_after_install = true         -> /S /UPDATE /R /ARGS <current args>
# Scribe's frontend calls downloadAndInstall with the plugin's default
# (restart_after_install = true), so the /R forms are what users get; the
# current args are empty for a copy opened from the Start menu and
# "--minimized" for one started at sign-in.
param(
  [Parameter(Mandatory)] [string] $Candidate,
  [Parameter(Mandatory)] [string] $OldDir,
  [Parameter(Mandatory)] [string] $Version,
  [Parameter(Mandatory)] [string] $ResultsPath,
  # Optional: the same build packaged with the cleanup hooks variant.
  [string] $CleanupCandidate = ""
)

. (Join-Path $PSScriptRoot "common.ps1")

$forms = [ordered]@{
  "plain"              = @{ Args = "/S /UPDATE"; Relaunch = $false; Minimized = $false }
  "relaunch"           = @{ Args = "/S /UPDATE /R /ARGS"; Relaunch = $true; Minimized = $false }
  "relaunch-minimized" = @{ Args = "/S /UPDATE /R /ARGS --minimized"; Relaunch = $true; Minimized = $true }
}

function Old-Installer([string] $tag) {
  return (Get-ChildItem (Join-Path $OldDir "old-$tag") -Filter "*setup.exe" | Select-Object -First 1).FullName
}

function Bridge([string] $tag, [string] $formName, [string] $folder, [string] $setup = $Candidate, [string] $label = "real hooks") {
  $form = $forms[$formName]
  Scenario "NSIS $tag -> native $Version ($label), $formName ($($form.Args)), folder $folder"
  Reset-Machine
  $exe = Join-Path $folder "Scribe.exe"
  $isDefault = Same-Path $folder $defaultDir
  $installArgs = if ($isDefault) { "/S" } else { "/S /D=$folder" }
  $code = Run-Wait (Old-Installer $tag) $installArgs
  Check "$tag installer exit code is 0" ($code -eq 0) "(was $code)"
  Check "$tag installed Scribe.exe" (Test-Path $exe)
  if (-not (Test-Path $exe)) { return }
  if (-not $isDefault -and $tag -ne "v0.16.1") {
    # What v0.16.2 and later write at each launch of an installed copy.
    New-Item -Path $newKey -Force | Out-Null
    Set-Item -LiteralPath $newKey -Value $folder
  }
  Seed-RealInstall $exe
  $oldHash = (Get-FileHash $exe -Algorithm SHA256).Hash
  $oldVersion = (Get-Item $exe).VersionInfo.ProductVersion
  $oldFiles = @(Get-ChildItem $folder -Recurse -File | ForEach-Object { $_.FullName.Substring($folder.Length + 1) })
  Note "$tag Scribe.exe ProductVersion $oldVersion; files: $($oldFiles -join ', ')"
  $startLnk = Join-Path $userStartMenu "Scribe\Scribe.lnk"
  $desktopLnk = Join-Path ([Environment]::GetFolderPath("Desktop")) "Scribe.lnk"
  Check "$tag created the Start menu shortcut" (Test-Path -LiteralPath $startLnk)
  $runBefore = Run-Value
  $dataBefore = Data-Fingerprint
  $markersBefore = (Marker-Lines).Count

  $code = Run-Wait $setup $form.Args
  Check "native setup exit code is 0" ($code -eq 0) "(was $code)"

  # Same folder, old exe replaced.
  Check "Scribe.exe is still in $folder" (Test-Path $exe)
  if (Test-Path $exe) {
    $info = (Get-Item $exe).VersionInfo
    Check "Scribe.exe is the native build ($Version)" ($info.ProductVersion -like "$Version*") "(ProductVersion $($info.ProductVersion))"
    Check "Scribe.exe CompanyName is n8builds.dev" ($info.CompanyName -eq "n8builds.dev") "(was '$($info.CompanyName)')"
    Check "the old Scribe.exe was replaced" ((Get-FileHash $exe -Algorithm SHA256).Hash -ne $oldHash)
  }
  if (-not $isDefault) {
    Check "no second copy in the default folder" (-not (Test-Path (Join-Path $defaultDir "Scribe.exe")))
  }

  # Installed apps.
  $entries = Installed-Entries
  Check "exactly one Scribe entry in Installed apps" ($entries.Count -eq 1) "(found $($entries.Count): $(($entries | ForEach-Object { "$($_.Key) $($_.DisplayVersion)" }) -join '; '))"
  if ($entries.Count -ge 1) {
    $entry = $entries[0]
    Check "Installed apps version is $Version" ($entry.DisplayVersion -eq $Version) "(was '$($entry.DisplayVersion)')"
    Check "Installed apps publisher is n8builds.dev" ($entry.Publisher -eq "n8builds.dev") "(was '$($entry.Publisher)')"
    Check "Installed apps entry points into $folder" ($entry.UninstallString -like "*$folder*") "(was '$($entry.UninstallString)')"
  }
  Check "the natkins key is gone" (-not (Test-Path -LiteralPath $oldKey))
  Check "the n8builds.dev key records $folder" (Same-Path (Default-Value $newKey) $folder) "(was '$(Default-Value $newKey)')"

  # Sign-in entry, shortcuts, data.
  $runAfter = Run-Value
  Check "the Run value is unchanged" ($runAfter -eq $runBefore) "(was '$runAfter')"
  $runExe = Run-Value-Exe $runAfter
  Check "the Run value points at an existing exe" ($runExe -and (Test-Path -LiteralPath $runExe)) "('$runExe')"
  Check "the Run value points at the native exe" (Same-Path $runExe $exe) "('$runExe')"
  Check "the Start menu shortcut is present" (Test-Path -LiteralPath $startLnk)
  Check "the Start menu shortcut points at the native exe" (Same-Path (Shortcut-Target $startLnk) $exe) "('$(Shortcut-Target $startLnk)')"
  if (Test-Path -LiteralPath $desktopLnk) {
    Check "the desktop shortcut points at the native exe" (Same-Path (Shortcut-Target $desktopLnk) $exe) "('$(Shortcut-Target $desktopLnk)')"
  }
  Check "the data folders are untouched" ((Data-Fingerprint) -eq $dataBefore)

  # Relaunch.
  if ($form.Relaunch) {
    $line = Wait-Launch $markersBefore 30
    Check "/R relaunched the app" ($null -ne $line)
    if ($line) {
      Check "the relaunched exe is the native exe in $folder" (Same-Path (Field $line "exe") $exe) "('$(Field $line "exe")')"
      Check "the relaunch carried minimized=$($form.Minimized)" ((Field $line "minimized") -eq "$($form.Minimized)".ToLower()) "('$line')"
    }
  } else {
    $line = Wait-Launch $markersBefore 5
    Check "no relaunch without /R" ($null -eq $line) "('$line')"
  }

  # Leftovers of the Tauri build next to the native exe.
  $leftovers = @(Get-ChildItem $folder -Recurse -File | ForEach-Object { $_.FullName.Substring($folder.Length + 1) } |
      Where-Object { $_ -notin @("Scribe.exe", "uninstall.exe") })
  Note "files left from the Tauri build after the update: $(if ($leftovers.Count) { $leftovers -join ', ' } else { 'none' })"
  Stop-Scribe
  $markersBefore = (Marker-Lines).Count
  Start-Process $exe -ArgumentList "--minimized" | Out-Null
  $line = Wait-Launch $markersBefore 15
  Check "the native exe starts from the Run value command with the leftovers present" ($null -ne $line)

  # Uninstall: in place (`_?=`), so it waits; it cannot delete itself then.
  Stop-Scribe
  $uninstaller = Join-Path $folder "uninstall.exe"
  Check "uninstall.exe exists" (Test-Path $uninstaller)
  if (Test-Path $uninstaller) {
    $code = Run-Wait $uninstaller "/S _?=$folder"
    Check "uninstaller exit code is 0" ($code -eq 0) "(was $code)"
    Remove-Item -LiteralPath $uninstaller -Force -ErrorAction SilentlyContinue
  }
  Check "Scribe.exe is removed" (-not (Test-Path $exe))
  $remaining = @()
  if (Test-Path $folder) { $remaining = @(Get-ChildItem $folder -Recurse -File | ForEach-Object { $_.FullName.Substring($folder.Length + 1) }) }
  Check "uninstall leaves no files in the install folder" ($remaining.Count -eq 0) "(left: $($remaining -join ', '))"
  Check "no Scribe entry in Installed apps" ((Installed-Entries).Count -eq 0)
  Check "the n8builds.dev key is gone" (-not (Test-Path -LiteralPath $newKey))
  Check "the natkins key is gone" (-not (Test-Path -LiteralPath $oldKey))
  Check "the Run value is gone" ($null -eq (Run-Value))
  Check "the Start menu shortcut is gone" (-not (Test-Path -LiteralPath $startLnk))
  Check "the desktop shortcut is gone" (-not (Test-Path -LiteralPath $desktopLnk))
  $kept = @($dataDirs | Where-Object { Test-Path $_ })
  Note "data folders after a silent uninstall: $(if ($kept.Count) { 'kept (' + ($kept -join ', ') + ')' } else { 'removed' })"
}

$explorer = @(Get-Process -Name explorer -ErrorAction SilentlyContinue).Count
$elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Scenario "Runner"
Note "explorer.exe processes: $explorer; running elevated: $elevated (the /R relaunch goes through nsis_tauri_utils::RunAsUser)"

foreach ($tag in "v0.16.1", "v0.16.2", "v0.16.3") {
  foreach ($formName in $forms.Keys) {
    Bridge $tag $formName $defaultDir
  }
}
# A pre-0.16.2 copy in a custom folder: the hooks must adopt it.
Bridge "v0.16.1" "relaunch-minimized" "C:\ScribeCustom\Scribe"
Bridge "v0.16.3" "relaunch-minimized" "C:\ScribeCustom\Scribe"

# The same bridge with the cleanup variant of the hooks (make-config.ps1),
# which removes the Tauri build's sidecars after the update.
if ($CleanupCandidate) {
  foreach ($tag in "v0.16.1", "v0.16.2", "v0.16.3") {
    Bridge $tag "relaunch-minimized" $defaultDir $CleanupCandidate "hooks + sidecar cleanup"
  }
  Bridge "v0.16.1" "relaunch" "C:\ScribeCustom\Scribe" $CleanupCandidate "hooks + sidecar cleanup"
}

Scenario "Runner clean after the last NSIS uninstall"
Check "no Scribe entry in Installed apps" ((Installed-Entries).Count -eq 0)
Check "no n8builds.dev or natkins key" (-not (Test-Path -LiteralPath $newKey) -and -not (Test-Path -LiteralPath $oldKey))
Check "no Run value" ($null -eq (Run-Value))
Check "no Scribe process" (@(Get-Process -Name Scribe -ErrorAction SilentlyContinue).Count -eq 0)
Reset-Machine

Write-Results $ResultsPath "NSIS bridge"
if ($script:failures.Count -gt 0) {
  throw "$($script:failures.Count) NSIS check(s) failed: $($script:failures -join '; ')"
}
Write-Host "All NSIS bridge checks passed."
