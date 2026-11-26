# Stages what the release SBOM describes: the lock files of what Scribe ships.
# The app's npm packages come from app/package-lock.json and the Rust crates
# from the workspace Cargo.lock at the repository root (the Tauri shell and
# crates/scribe-core). The CLI, the MCP server, the spikes and the release
# tooling are not shipped, so their lock files stay out.
# Used by release.yml and the CI job that proves the SBOM on pull requests.
param(
  [Parameter(Mandatory)] [string] $Out
)
$ErrorActionPreference = "Stop"
$repo = Resolve-Path (Join-Path $PSScriptRoot "../..")
foreach ($file in @("app/package.json", "app/package-lock.json", "Cargo.lock")) {
  $target = Join-Path $Out $file
  New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
  Copy-Item (Join-Path $repo $file) $target
}
Write-Host "Staged the SBOM source in $Out"
