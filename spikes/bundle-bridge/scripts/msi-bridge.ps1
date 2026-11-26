# MSI bridge: the published 0.16.3 MSI updated by the native build's MSI,
# run the way tauri-plugin-updater 2.14.1 runs an MSI update (installMode
# "quiet", restart_after_install = true):
#   msiexec.exe /i <msi> /quiet /promptrestart AUTOLAUNCHAPP=True LAUNCHAPPARGS="<args>"
# Also reads the UpgradeCode of both MSIs and recomputes it the way
# tauri-bundler 2.10.4 derives it (src/bundle/windows/msi/mod.rs): a v5 UUID
# in the DNS namespace of "<productName>.exe.app.x64", unless
# bundle.windows.wix.upgradeCode is set.
param(
  [Parameter(Mandatory)] [string] $Candidate,
  [Parameter(Mandatory)] [string] $CandidateSetup,
  [Parameter(Mandatory)] [string] $OldMsi,
  [Parameter(Mandatory)] [string] $Version,
  [Parameter(Mandatory)] [string] $ResultsPath
)

. (Join-Path $PSScriptRoot "common.ps1")

$msiexec = Join-Path $env:SYSTEMROOT "System32\msiexec.exe"
$programDir = Join-Path $env:ProgramFiles "Scribe"
$msiExe = Join-Path $programDir "Scribe.exe"
$msiStartLnk = Join-Path $machineStartMenu "Scribe\Scribe.lnk"

function Msi-Property([string] $path, [string] $name) {
  $installer = New-Object -ComObject WindowsInstaller.Installer
  $db = $installer.GetType().InvokeMember("OpenDatabase", "InvokeMethod", $null, $installer, @($path, 0))
  $view = $db.GetType().InvokeMember("OpenView", "InvokeMethod", $null, $db, @("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$name'"))
  $view.GetType().InvokeMember("Execute", "InvokeMethod", $null, $view, $null) | Out-Null
  $record = $view.GetType().InvokeMember("Fetch", "InvokeMethod", $null, $view, $null)
  $value = if ($record) { $record.GetType().InvokeMember("StringData", "GetProperty", $null, $record, 1) } else { $null }
  $view.GetType().InvokeMember("Close", "InvokeMethod", $null, $view, $null) | Out-Null
  foreach ($o in $record, $view, $db, $installer) {
    if ($o) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($o) }
  }
  return $value
}

# RFC 4122 v5 UUID, as uuid::Uuid::new_v5(&Uuid::NAMESPACE_DNS, name).
function Uuid-V5-Dns([string] $name) {
  $ns = [byte[]](0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8)
  $bytes = $ns + [Text.Encoding]::UTF8.GetBytes($name)
  $hash = [Security.Cryptography.SHA1]::Create().ComputeHash([byte[]]$bytes)
  $b = $hash[0..15]
  $b[6] = ($b[6] -band 0x0f) -bor 0x50
  $b[8] = ($b[8] -band 0x3f) -bor 0x80
  $hex = ($b | ForEach-Object { $_.ToString("x2") }) -join ""
  return "{0}-{1}-{2}-{3}-{4}" -f $hex.Substring(0, 8), $hex.Substring(8, 4), $hex.Substring(12, 4), $hex.Substring(16, 4), $hex.Substring(20, 12)
}

function Clean-Msi {
  Stop-Scribe
  foreach ($e in Installed-Entries) {
    if ($e.UninstallString -match '(\{[0-9A-Fa-f-]+\})') {
      Run-Wait $msiexec "/x $($Matches[1]) /quiet /norestart" | Out-Null
    } elseif ($e.UninstallString -match '"([^"]+uninstall\.exe)"') {
      $dir = Split-Path $Matches[1]
      Run-Wait $Matches[1] "/S _?=$dir" | Out-Null
    }
  }
  Remove-Item -LiteralPath $programDir -Recurse -Force -ErrorAction SilentlyContinue
  Reset-Machine
}

# ---- UpgradeCode ----
Scenario "MSI identity"
$oldUpgrade = Msi-Property $OldMsi "UpgradeCode"
$newUpgrade = Msi-Property $Candidate "UpgradeCode"
$derived = Uuid-V5-Dns "Scribe.exe.app.x64"
Note "0.16.3 MSI UpgradeCode $oldUpgrade, ProductCode $(Msi-Property $OldMsi 'ProductCode'), ProductVersion $(Msi-Property $OldMsi 'ProductVersion'), Manufacturer $(Msi-Property $OldMsi 'Manufacturer')"
Note "native MSI UpgradeCode $newUpgrade, ProductCode $(Msi-Property $Candidate 'ProductCode'), ProductVersion $(Msi-Property $Candidate 'ProductVersion'), Manufacturer $(Msi-Property $Candidate 'Manufacturer')"
Note "uuid v5 (DNS namespace) of 'Scribe.exe.app.x64' = $derived"
Check "the 0.16.3 UpgradeCode is the v5 UUID of 'Scribe.exe.app.x64'" ($oldUpgrade.Trim('{}') -ieq $derived) "($oldUpgrade)"
Check "the native MSI has the same UpgradeCode" ($newUpgrade -ieq $oldUpgrade) "($newUpgrade)"
Check "the native MSI has a new ProductCode" ((Msi-Property $Candidate "ProductCode") -ine (Msi-Property $OldMsi "ProductCode"))
Check "the native MSI Manufacturer is n8builds.dev" ((Msi-Property $Candidate "Manufacturer") -eq "n8builds.dev")

# ---- M1: 0.16.3 MSI -> native MSI, as the updater runs it ----
Scenario "MSI 0.16.3 -> native MSI $Version, msiexec /i <msi> /quiet /promptrestart AUTOLAUNCHAPP=True LAUNCHAPPARGS=`"--minimized`""
Clean-Msi
$code = Run-Wait $msiexec "/i `"$OldMsi`" /quiet /norestart /l*v `"$env:RUNNER_TEMP\msi-old.log`""
Check "0.16.3 MSI install exit code is 0" ($code -eq 0) "(was $code)"
Check "0.16.3 MSI installed $msiExe" (Test-Path $msiExe)
if (Test-Path $msiExe) {
  Seed-RealInstall $msiExe
  $oldHash = (Get-FileHash $msiExe -Algorithm SHA256).Hash
  Note "0.16.3 MSI files: $((Get-ChildItem $programDir -Recurse -File | ForEach-Object { $_.FullName.Substring($programDir.Length + 1) }) -join ', ')"
  Check "0.16.3 MSI created the Start menu shortcut" (Test-Path -LiteralPath $msiStartLnk)
  $runBefore = Run-Value
  $dataBefore = Data-Fingerprint
  $markersBefore = (Marker-Lines).Count

  # The plugin passes the path unquoted; this one has no spaces.
  $code = Run-Wait $msiexec "/i $Candidate /quiet /promptrestart AUTOLAUNCHAPP=True LAUNCHAPPARGS=`"--minimized`" /l*v $env:RUNNER_TEMP\msi-new.log"
  Check "native MSI exit code is 0" ($code -eq 0) "(was $code)"
  Check "Scribe.exe is still in $programDir" (Test-Path $msiExe)
  if (Test-Path $msiExe) {
    $info = (Get-Item $msiExe).VersionInfo
    Check "Scribe.exe is the native build ($Version)" ($info.ProductVersion -like "$Version*") "(ProductVersion $($info.ProductVersion))"
    Check "the old Scribe.exe was replaced" ((Get-FileHash $msiExe -Algorithm SHA256).Hash -ne $oldHash)
  }
  $entries = Installed-Entries
  Check "exactly one Scribe entry in Installed apps (upgraded in place, not side by side)" ($entries.Count -eq 1) "(found $($entries.Count): $(($entries | ForEach-Object { "$($_.Key) $($_.DisplayVersion)" }) -join '; '))"
  if ($entries.Count -ge 1) {
    Check "Installed apps version is $Version" ($entries[0].DisplayVersion -eq $Version) "(was '$($entries[0].DisplayVersion)')"
    Check "Installed apps publisher is n8builds.dev" ($entries[0].Publisher -eq "n8builds.dev") "(was '$($entries[0].Publisher)')"
  }
  # Both MSIs install Scribe.exe and the WiX template's "Uninstall Scribe.lnk".
  $leftovers = @(Get-ChildItem $programDir -Recurse -File | ForEach-Object { $_.FullName.Substring($programDir.Length + 1) } | Where-Object { $_ -notin @("Scribe.exe", "Uninstall Scribe.lnk") })
  Check "the major upgrade removed the old Tauri files" ($leftovers.Count -eq 0) "(left: $($leftovers -join ', '))"
  $runExe = Run-Value-Exe (Run-Value)
  Check "the Run value is unchanged" ((Run-Value) -eq $runBefore)
  Check "the Run value points at an existing exe" ($runExe -and (Test-Path -LiteralPath $runExe)) "('$runExe')"
  Check "the Start menu shortcut is present" (Test-Path -LiteralPath $msiStartLnk)
  Check "the Start menu shortcut points at the native exe" (Same-Path (Shortcut-Target $msiStartLnk) $msiExe) "('$(Shortcut-Target $msiStartLnk)')"
  Check "the data folders are untouched" ((Data-Fingerprint) -eq $dataBefore)
  $line = Wait-Launch $markersBefore 30
  Check "AUTOLAUNCHAPP relaunched the app" ($null -ne $line)
  if ($line) {
    Check "the relaunched exe is the native exe" (Same-Path (Field $line "exe") $msiExe) "('$(Field $line "exe")')"
    Check "the relaunch carried --minimized" ((Field $line "minimized") -eq "true") "('$line')"
  }

  # Uninstall.
  Stop-Scribe
  $product = Msi-Property $Candidate "ProductCode"
  $code = Run-Wait $msiexec "/x $product /quiet /norestart"
  Check "native MSI uninstall exit code is 0" ($code -eq 0) "(was $code)"
  Check "the install folder is removed" (-not (Test-Path $programDir))
  Check "no Scribe entry in Installed apps" ((Installed-Entries).Count -eq 0)
  Check "the Start menu shortcut is gone" (-not (Test-Path -LiteralPath $msiStartLnk))
  $installDirValue = (Get-ItemProperty -LiteralPath $newKey -ErrorAction SilentlyContinue).InstallDir
  Check "the MSI's InstallDir value under the n8builds.dev key is gone" (-not $installDirValue) "('$installDirValue')"
  Note "after the MSI uninstall: n8builds.dev key $(if (Test-Path -LiteralPath $newKey) { 'still exists (empty)' } else { 'gone' }); Run value $(if (Run-Value) { 'still present (the WiX template never removes it)' } else { 'gone' })"
}

# ---- M2: what MSI users actually get: latest.json names only setup.exe ----
Scenario "MSI 0.16.3 -> native NSIS setup, /S /UPDATE /R /ARGS --minimized (informational: latest.json only lists the NSIS installer)"
Clean-Msi
$code = Run-Wait $msiexec "/i `"$OldMsi`" /quiet /norestart"
Check "0.16.3 MSI install exit code is 0" ($code -eq 0) "(was $code)"
Seed-RealInstall $msiExe
$code = Run-Wait $CandidateSetup "/S /UPDATE /R /ARGS --minimized"
Check "native setup exit code is 0" ($code -eq 0) "(was $code)"
$entries = Installed-Entries
Note "Installed apps entries after it: $($entries.Count): $(($entries | ForEach-Object { "$($_.Key) v$($_.DisplayVersion) $($_.UninstallString)" }) -join '; ')"
Note "Scribe.exe in Program Files: $(if (Test-Path $msiExe) { (Get-Item $msiExe).VersionInfo.ProductVersion } else { 'none' }); in $defaultDir`: $(if (Test-Path (Join-Path $defaultDir 'Scribe.exe')) { (Get-Item (Join-Path $defaultDir 'Scribe.exe')).VersionInfo.ProductVersion } else { 'none' })"
Note "Run value now: $(Run-Value)"
Clean-Msi

Scenario "Runner clean after the MSI scenarios"
Check "no Scribe entry in Installed apps" ((Installed-Entries).Count -eq 0)
Check "no install folder in Program Files" (-not (Test-Path $programDir))

Write-Results $ResultsPath "MSI bridge"
if ($script:failures.Count -gt 0) {
  throw "$($script:failures.Count) MSI check(s) failed: $($script:failures -join '; ')"
}
Write-Host "All MSI bridge checks passed."
