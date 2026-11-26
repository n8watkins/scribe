# Shared helpers for the bundle bridge scenarios. Dot-sourced by
# nsis-bridge.ps1 and msi-bridge.ps1. These write to HKCU, HKLM, Program
# Files and the user profile, so they are for a throwaway CI runner only.

$ErrorActionPreference = "Stop"
$script:failures = New-Object System.Collections.Generic.List[string]
$script:results = New-Object System.Collections.Generic.List[string]
$script:scenario = ""

$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Scribe"
$oldKey = "HKCU:\Software\natkins\Scribe"
$newKey = "HKCU:\Software\n8builds.dev\Scribe"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$defaultDir = Join-Path $env:LOCALAPPDATA "Scribe"
$markerDir = "C:\ScribeBridgeSpike"
$markerLog = Join-Path $markerDir "launches.log"
# Tauri's app data folders for the identifier com.natkins.scribe.
$dataDirs = @(
  (Join-Path $env:APPDATA "com.natkins.scribe"),
  (Join-Path $env:LOCALAPPDATA "com.natkins.scribe")
)
$userStartMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$machineStartMenu = Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs"

function Scenario([string] $name) {
  $script:scenario = $name
  Write-Host ""
  Write-Host "=== $name ==="
  $script:results.Add("")
  $script:results.Add("### $name")
}

function Check([string] $name, [bool] $ok, [string] $detail = "") {
  $line = if ($ok) { "PASS  $name" } else { "FAIL  $name $detail" }
  Write-Host "  $line"
  $script:results.Add("- $line")
  if (-not $ok) { $script:failures.Add("[$script:scenario] $name") }
}

function Note([string] $text) {
  Write-Host "  NOTE  $text"
  $script:results.Add("- NOTE  $text")
}

function Default-Value([string] $key) {
  if (-not (Test-Path -LiteralPath $key)) { return $null }
  return (Get-Item -LiteralPath $key).GetValue("")
}

function Same-Path($a, $b) {
  return $a -and $b -and ($a.TrimEnd('\') -ieq $b.TrimEnd('\'))
}

function Stop-Scribe {
  Get-Process -Name Scribe -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Get-Process -Name whisper-server -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
}

# Runs a program and waits at most $TimeoutSec, so a dialog that a silent
# run should never show fails the check instead of hanging the job.
function Run-Wait([string] $file, [string] $arguments, [int] $TimeoutSec = 300) {
  Write-Host "  run: $file $arguments"
  $process = Start-Process $file -ArgumentList $arguments -PassThru
  if (-not $process.WaitForExit($TimeoutSec * 1000)) {
    $process | Stop-Process -Force -ErrorAction SilentlyContinue
    return -1
  }
  return $process.ExitCode
}

# Every "Scribe" entry in Installed apps, from both hives and both views.
function Installed-Entries {
  $roots = @(
    "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"
  )
  $entries = @()
  foreach ($root in $roots) {
    if (-not (Test-Path $root)) { continue }
    foreach ($key in Get-ChildItem $root) {
      $p = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue
      if ($p.DisplayName -eq "Scribe") {
        $entries += [pscustomobject]@{
          Key = $key.Name
          DisplayVersion = $p.DisplayVersion
          Publisher = $p.Publisher
          InstallLocation = $p.InstallLocation
          UninstallString = $p.UninstallString
        }
      }
    }
  }
  return ,$entries
}

function Shortcut-Target([string] $lnk) {
  if (-not (Test-Path -LiteralPath $lnk)) { return $null }
  $shell = New-Object -ComObject WScript.Shell
  return $shell.CreateShortcut($lnk).TargetPath
}

# What a real install has besides the installer's own files: the sign-in
# entry the autostart plugin writes, and the user's data.
function Seed-RealInstall([string] $exe) {
  New-Item -Path $runKey -Force | Out-Null
  Set-ItemProperty -LiteralPath $runKey -Name "Scribe" -Value "`"$exe`" --minimized"
  foreach ($dir in $dataDirs) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Set-Content -Path (Join-Path $dir "settings.json") -Value '{"spike":"bundle-bridge","keep":true}' -Encoding utf8
    Set-Content -Path (Join-Path $dir "history.db") -Value ("x" * 4096) -Encoding ascii
  }
}

function Data-Fingerprint {
  $lines = foreach ($dir in $dataDirs) {
    if (Test-Path $dir) {
      Get-ChildItem $dir -Recurse -File | Sort-Object FullName | ForEach-Object {
        "{0} {1}" -f $_.FullName, (Get-FileHash $_.FullName -Algorithm SHA256).Hash
      }
    } else { "$dir missing" }
  }
  return ($lines -join "`n")
}

function Run-Value {
  $item = Get-ItemProperty -LiteralPath $runKey -ErrorAction SilentlyContinue
  if (-not $item) { return $null }
  return $item.Scribe
}

# The exe a Run value starts: the quoted first token.
function Run-Value-Exe($value) {
  if (-not $value) { return $null }
  if ($value -match '^"([^"]+)"') { return $Matches[1] }
  return ($value -split ' ')[0]
}

# The leading comma keeps a one-line log an array (PowerShell would unroll
# it to a string, and indexing a string gives its first character).
function Marker-Lines {
  if (-not (Test-Path $markerLog)) { return ,@() }
  return ,@(Get-Content $markerLog)
}

# Waits for a new marker line (a launch of the stub) after $before lines.
function Wait-Launch([int] $before, [int] $TimeoutSec = 30) {
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  while ((Get-Date) -lt $deadline) {
    $lines = @(Marker-Lines)
    if ($lines.Count -gt $before) { return $lines[$before] }
    Start-Sleep -Milliseconds 500
  }
  return $null
}

function Field([string] $line, [string] $name) {
  foreach ($part in $line -split "`t") {
    if ($part.StartsWith("$name=")) { return $part.Substring($name.Length + 1) }
  }
  return $null
}

function Reset-Machine {
  Stop-Scribe
  foreach ($key in $oldKey, $newKey, $uninstallKey) {
    Remove-Item -LiteralPath $key -Recurse -Force -ErrorAction SilentlyContinue
  }
  Remove-ItemProperty -LiteralPath $runKey -Name "Scribe" -ErrorAction SilentlyContinue
  foreach ($dir in @($defaultDir, "C:\ScribeCustom", $markerDir) + $dataDirs) {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
  }
  foreach ($lnk in @(
      (Join-Path $userStartMenu "Scribe"),
      (Join-Path $userStartMenu "Scribe.lnk"),
      (Join-Path ([Environment]::GetFolderPath("Desktop")) "Scribe.lnk"))) {
    Remove-Item -LiteralPath $lnk -Recurse -Force -ErrorAction SilentlyContinue
  }
}

function Write-Results([string] $path, [string] $title) {
  $out = @("## $title") + $script:results
  if ($script:failures.Count -gt 0) {
    $out += ""
    $out += "Failed: $($script:failures.Count)"
    $out += ($script:failures | ForEach-Object { "- $_" })
  } else {
    $out += ""
    $out += "All checks passed."
  }
  Set-Content -Path $path -Value $out -Encoding utf8
  if ($env:GITHUB_STEP_SUMMARY) { Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $out -Encoding utf8 }
}
