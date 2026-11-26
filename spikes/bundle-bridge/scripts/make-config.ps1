# Writes native/tauri.conf.json: the real app/src-tauri/tauri.conf.json with
# only what a packaging-only run of a non-Tauri exe needs changed. Everything
# that decides the installer identity (productName, mainBinaryName,
# identifier, publisher, NSIS installMode, startMenuFolder, the hooks file,
# WiX settings) is read from the real file and kept as is.
param(
  [Parameter(Mandatory)] [string] $Repo,
  [Parameter(Mandatory)] [string] $Version
)

$ErrorActionPreference = "Stop"
$tauriDir = Join-Path $Repo "app/src-tauri"
$real = Get-Content (Join-Path $tauriDir "tauri.conf.json") -Raw | ConvertFrom-Json

$config = $real
$config.version = $Version
# No frontend to build: `tauri bundle` packages an exe that already exists.
$config.build = [pscustomobject]@{}
# The native build has no webview, so the installer must not fetch WebView2.
$config.bundle.windows.webviewInstallMode = [pscustomobject]@{ type = "skip" }
# The real resources are the whisper sidecars; the stub ships none, which is
# also how this spike sees what an update leaves behind of the old files.
$config.bundle.PSObject.Properties.Remove("resources")
# Signing happens on its own afterwards with `tauri signer sign`, as in
# release.yml, so the bundle step never needs a key.
$config.bundle.createUpdaterArtifacts = $false

# Paths in the real file are relative to app/src-tauri; make them absolute.
$abs = { param($p) (Resolve-Path (Join-Path $tauriDir $p)).Path }
$config.bundle.icon = @($config.bundle.icon | ForEach-Object { & $abs $_ })
$nsis = $config.bundle.windows.nsis
$nsis.installerIcon = & $abs $nsis.installerIcon
$nsis.uninstallerIcon = & $abs $nsis.uninstallerIcon
$nsis.installerHooks = & $abs $nsis.installerHooks

$out = Join-Path $PSScriptRoot "../native/tauri.conf.json"
$config | ConvertTo-Json -Depth 20 | Set-Content $out -Encoding utf8
Write-Host "Wrote $out"
Get-Content $out
