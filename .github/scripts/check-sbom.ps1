# Fails when the release SBOM has lost its Rust crates or npm packages, which
# a scan of the wrong folder does silently. Compares against the SBOM of the
# latest published release when one is there: the Rust crate count may not
# fall below 90% of it. Needs GH_TOKEN for the download.
# Used by release.yml and the CI job that proves the SBOM on pull requests.
param(
  [Parameter(Mandatory)] [string] $Sbom,
  [Parameter(Mandatory)] [string] $Repository
)
$ErrorActionPreference = "Stop"

function Get-Counts([string] $Path) {
  $components = @((Get-Content $Path -Raw | ConvertFrom-Json).components)
  $purls = @($components | ForEach-Object { [string]$_.purl })
  [pscustomobject]@{
    Cargo = @($purls | Where-Object { $_.StartsWith("pkg:cargo/") }).Count
    Npm = @($purls | Where-Object { $_.StartsWith("pkg:npm/") }).Count
    All = $components.Count
  }
}

$counts = Get-Counts $Sbom
Write-Host "SBOM $Sbom`: $($counts.Cargo) Rust crates (pkg:cargo), $($counts.Npm) npm packages, $($counts.All) components"
if ($counts.Cargo -eq 0) { throw "The SBOM has no Rust crates: the scan missed Cargo.lock" }
if ($counts.Npm -eq 0) { throw "The SBOM has no npm packages: the scan missed app/package-lock.json" }

$previousDir = Join-Path ([System.IO.Path]::GetTempPath()) "scribe-previous-sbom"
Remove-Item -Recurse -Force $previousDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $previousDir | Out-Null
$tag = gh release view --repo $Repository --json tagName --jq .tagName
if ($LASTEXITCODE -ne 0 -or -not $tag) {
  Write-Host "::warning::No published release found to compare the SBOM with"
  exit 0
}
gh release download $tag --repo $Repository --pattern "*.cdx.json" --dir $previousDir
if ($LASTEXITCODE -ne 0) {
  Write-Host "::warning::Release $tag has no SBOM to compare with"
  exit 0
}
$previousFile = Get-ChildItem $previousDir -Filter "*.cdx.json" | Select-Object -First 1
$previous = Get-Counts $previousFile.FullName
Write-Host "Previous SBOM ($tag): $($previous.Cargo) Rust crates, $($previous.Npm) npm packages, $($previous.All) components"
if ($counts.Cargo -lt [math]::Floor($previous.Cargo * 0.9)) {
  throw "The SBOM lists $($counts.Cargo) Rust crates, far below the $($previous.Cargo) of $tag"
}
exit 0
