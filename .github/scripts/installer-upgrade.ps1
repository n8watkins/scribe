# Upgrade and uninstall scenarios for the NSIS installer (see
# .github/workflows/installer-upgrade.yml). Runs on a throwaway CI runner and
# writes to HKCU and the user profile, so it is not meant for a real PC.
param(
  [Parameter(Mandatory)] [string] $Candidate,
  [Parameter(Mandatory)] [string] $OldDir,
  [Parameter(Mandatory)] [string] $Version
)

$ErrorActionPreference = "Stop"
$failures = New-Object System.Collections.Generic.List[string]
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Scribe"
$oldKey = "HKCU:\Software\natkins\Scribe"
$newKey = "HKCU:\Software\n8builds.dev\Scribe"
$defaultDir = Join-Path $env:LOCALAPPDATA "Scribe"

function Check([string] $name, [bool] $ok, [string] $detail = "") {
  if ($ok) { Write-Host "  PASS  $name" }
  else { Write-Host "  FAIL  $name $detail"; $failures.Add($name) }
}

function Default-Value([string] $key) {
  if (-not (Test-Path -LiteralPath $key)) { return $null }
  return (Get-Item -LiteralPath $key).GetValue("")
}

function Same-Folder($a, $b) {
  return $a -and $b -and ($a.TrimEnd('\') -ieq $b.TrimEnd('\'))
}

function Old-Installer([string] $tag) {
  return (Get-ChildItem (Join-Path $OldDir "old-$tag") -Filter "*setup.exe" | Select-Object -First 1).FullName
}

# NSIS reads /D= as the rest of the command line, so it goes last, unquoted.
function Run-Installer([string] $installer, [string] $arguments) {
  Write-Host "  run: $(Split-Path $installer -Leaf) $arguments"
  Get-Process -Name Scribe -ErrorAction SilentlyContinue | Stop-Process -Force
  $process = Start-Process $installer -ArgumentList $arguments -Wait -PassThru
  Check "installer exit code is 0 ($(Split-Path $installer -Leaf))" ($process.ExitCode -eq 0) "(was $($process.ExitCode))"
}

# `_?=` makes the NSIS uninstaller run in place and wait, instead of copying
# itself to %TEMP% and returning at once.
function Run-Uninstaller([string] $folder) {
  Get-Process -Name Scribe -ErrorAction SilentlyContinue | Stop-Process -Force
  $uninstaller = Join-Path $folder "uninstall.exe"
  Check "uninstaller exists in $folder" (Test-Path $uninstaller)
  if (Test-Path $uninstaller) {
    $process = Start-Process $uninstaller -ArgumentList "/S _?=$folder" -Wait -PassThru
    Check "uninstaller exit code is 0" ($process.ExitCode -eq 0) "(was $($process.ExitCode))"
  }
}

function Reset-Machine {
  foreach ($key in $oldKey, $newKey, $uninstallKey) {
    Remove-Item -LiteralPath $key -Recurse -Force -ErrorAction SilentlyContinue
  }
  foreach ($dir in $defaultDir, "C:\ScribeCustom") {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
  }
}

function Check-Upgraded([string] $custom) {
  $exe = Join-Path $custom "Scribe.exe"
  Check "Scribe.exe is in the custom folder" (Test-Path $exe)
  if (Test-Path $exe) {
    $info = (Get-Item $exe).VersionInfo
    Check "Scribe.exe is the new version" ($info.ProductVersion -like "$Version*") "(ProductVersion $($info.ProductVersion))"
    Check "Scribe.exe CompanyName is n8builds.dev" ($info.CompanyName -eq "n8builds.dev") "(was '$($info.CompanyName)')"
  }
  Check "no copy in the default folder" (-not (Test-Path (Join-Path $defaultDir "Scribe.exe"))) "($defaultDir exists)"
  $entry = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
  Check "Uninstall entry Publisher is n8builds.dev" ($entry.Publisher -eq "n8builds.dev") "(was '$($entry.Publisher)')"
  Check "Uninstall entry points into the custom folder" ($entry.UninstallString -like "*$custom*") "(was '$($entry.UninstallString)')"
  Check "the natkins key is gone" (-not (Test-Path -LiteralPath $oldKey))
  Check "the n8builds.dev key records the custom folder" (Same-Folder (Default-Value $newKey) $custom) "(was '$(Default-Value $newKey)')"
}

# ---- Scenario A: v0.16.1 in a custom folder -> this branch, as the updater ----
Write-Host "Scenario A: v0.16.1 in a custom folder, updated with /S /UPDATE"
Reset-Machine
$custom = "C:\ScribeCustom\Scribe"
Run-Installer (Old-Installer "v0.16.1") "/S /D=$custom"
Check "v0.16.1 installed into the custom folder" (Test-Path (Join-Path $custom "Scribe.exe"))
Check "v0.16.1 recorded the folder under natkins" (Same-Folder (Default-Value $oldKey) $custom) "(was '$(Default-Value $oldKey)')"
Check "v0.16.1 wrote no n8builds.dev key" (-not (Test-Path -LiteralPath $newKey))
Run-Installer $Candidate "/S /UPDATE"
Get-Content "$env:TEMP\scribe-hook-debug.log" -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "  DEBUG $_" }
Check-Upgraded $custom
Run-Uninstaller $custom

# ---- Scenario B: v0.16.2 in a custom folder, key as the app writes it ----
Write-Host "Scenario B: v0.16.2 in a custom folder with the n8builds.dev key, updated with /S /UPDATE"
Reset-Machine
Run-Installer (Old-Installer "v0.16.2") "/S /D=$custom"
Check "v0.16.2 installed into the custom folder" (Test-Path (Join-Path $custom "Scribe.exe"))
# What v0.16.2 writes at each launch of an installed copy.
New-Item -Path $newKey -Force | Out-Null
Set-Item -LiteralPath $newKey -Value $custom
Run-Installer $Candidate "/S /UPDATE"
Check-Upgraded $custom
Run-Uninstaller $custom

# ---- Scenario C: fresh install to the default folder, then uninstall ----
Write-Host "Scenario C: fresh install, then uninstall"
Reset-Machine
Run-Installer $Candidate "/S"
Check "Scribe.exe is in the default folder" (Test-Path (Join-Path $defaultDir "Scribe.exe"))
$entry = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
Check "Uninstall entry Publisher is n8builds.dev" ($entry.Publisher -eq "n8builds.dev") "(was '$($entry.Publisher)')"
Check "the n8builds.dev key records the default folder" (Same-Folder (Default-Value $newKey) $defaultDir) "(was '$(Default-Value $newKey)')"
Check "no natkins key" (-not (Test-Path -LiteralPath $oldKey))
Run-Uninstaller $defaultDir
Check "Scribe.exe is removed" (-not (Test-Path (Join-Path $defaultDir "Scribe.exe")))
Check "the n8builds.dev key is gone after uninstall" (-not (Test-Path -LiteralPath $newKey))
Check "the natkins key is gone after uninstall" (-not (Test-Path -LiteralPath $oldKey))
Check "the Uninstall entry is gone" (-not (Test-Path -LiteralPath $uninstallKey))

if ($failures.Count -gt 0) {
  throw "$($failures.Count) check(s) failed: $($failures -join '; ')"
}
Write-Host "All installer upgrade checks passed."
