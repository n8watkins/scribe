# Signs the native setup.exe with a throwaway key the way release.yml signs
# the real one (`tauri signer sign` with the key in TAURI_SIGNING_PRIVATE_KEY),
# builds a latest.json entry the way release.yml does, and verifies the
# result the way tauri-plugin-updater 2.14.1 does (verify/src/main.rs).
# The real signing key is never used; the key made here lives only in
# RUNNER_TEMP for this job.
param(
  [Parameter(Mandatory)] [string] $Repo,
  [Parameter(Mandatory)] [string] $Setup,
  [Parameter(Mandatory)] [string] $Version,
  [Parameter(Mandatory)] [string] $Verifier,
  [Parameter(Mandatory)] [string] $OldSetup,
  [Parameter(Mandatory)] [string] $ResultsPath
)

. (Join-Path $PSScriptRoot "common.ps1")

$tauri = Join-Path $Repo "app/node_modules/@tauri-apps/cli/tauri.js"
$work = Join-Path $env:RUNNER_TEMP "signature"
New-Item -ItemType Directory -Force -Path $work | Out-Null

function Verify([string] $file, [string] $sigFile, [string] $pubFile, [string] $announced, [string] $require) {
  $out = & $Verifier $file $sigFile $pubFile $announced $require
  Write-Host "    verifier: $out"
  return @{ Accepted = ($LASTEXITCODE -eq 0); Output = "$out" }
}

Scenario "Updater signature"
# A throwaway key with a password made in this job.
$password = [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(24))
$keyPath = Join-Path $work "throwaway.key"
& node $tauri signer generate --ci -w $keyPath -p $password -f | Out-Host
Check "tauri signer generate wrote a key pair" ((Test-Path $keyPath) -and (Test-Path "$keyPath.pub"))
$pubFile = "$keyPath.pub"

# 1. As release.yml signs it: key content in the environment, no --app-version.
$releaseCopy = Join-Path $work (Split-Path $Setup -Leaf)
Copy-Item $Setup $releaseCopy -Force
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $keyPath -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $password
& node $tauri signer sign $releaseCopy | Out-Host
$signedOk = $LASTEXITCODE -eq 0
Check "tauri signer sign (release.yml form) exit code is 0" $signedOk
Check "the signer wrote $(Split-Path $releaseCopy -Leaf).sig" (Test-Path "$releaseCopy.sig")

# 2. The same with the version bound into the trusted comment.
$boundDir = Join-Path $work "bound"
New-Item -ItemType Directory -Force -Path $boundDir | Out-Null
$boundCopy = Join-Path $boundDir (Split-Path $Setup -Leaf)
Copy-Item $Setup $boundCopy -Force
& node $tauri signer sign --app-version $Version $boundCopy | Out-Host
Check "tauri signer sign --app-version exit code is 0" ($LASTEXITCODE -eq 0)
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD

# latest.json as release.yml builds it (never published).
$signature = (Get-Content "$releaseCopy.sig" -Raw).Trim()
$manifest = [ordered]@{
  version = $Version
  pub_date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  platforms = @{ "windows-x86_64" = @{ signature = $signature; url = "https://example.invalid/$(Split-Path $Setup -Leaf)" } }
}
$manifestPath = Join-Path $work "latest.json"
$manifest | ConvertTo-Json -Depth 4 | Set-Content $manifestPath -Encoding utf8
$parsed = Get-Content $manifestPath -Raw | ConvertFrom-Json
$manifestSig = Join-Path $work "manifest.sig"
Set-Content $manifestSig $parsed.platforms.'windows-x86_64'.signature -Encoding ascii -NoNewline
$decoded = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($signature))
Note "trusted comment (release form): $((($decoded -split "`n") | Where-Object { $_ -like 'trusted comment:*' }) -replace "`t", ' | ')"

$r = Verify $releaseCopy $manifestSig $pubFile $Version "false"
Check "plugin check accepts the release-form signature taken from latest.json (requireSignedVersion off, Scribe's setting)" $r.Accepted $r.Output
$r = Verify $releaseCopy $manifestSig $pubFile $Version "true"
Check "with requireSignedVersion on, the release-form signature is refused (no version in the trusted comment)" (-not $r.Accepted) $r.Output
$r = Verify $boundCopy "$boundCopy.sig" $pubFile $Version "true"
Check "plugin check accepts the --app-version signature with requireSignedVersion on" $r.Accepted $r.Output
$r = Verify $boundCopy "$boundCopy.sig" $pubFile "0.99.1" "true"
Check "a manifest announcing another version is refused for the --app-version signature" (-not $r.Accepted) $r.Output

$tampered = Join-Path $work "tampered-setup.exe"
Copy-Item $releaseCopy $tampered -Force
$bytes = [IO.File]::ReadAllBytes($tampered); $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 0xff; [IO.File]::WriteAllBytes($tampered, $bytes)
$r = Verify $tampered $manifestSig $pubFile $Version "false"
Check "a modified setup.exe is refused" (-not $r.Accepted) $r.Output

# The pubkey installed copies trust (public; from tauri.conf.json).
$realPub = Join-Path $work "real.pub"
$real = Get-Content (Join-Path $Repo "app/src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
Set-Content $realPub $real.plugins.updater.pubkey -Encoding ascii -NoNewline
$r = Verify $releaseCopy $manifestSig $realPub $Version "false"
Check "installed copies refuse a setup signed by any key but the real one" (-not $r.Accepted) $r.Output
# Control: the verifier accepts the published 0.16.3 setup with its published signature.
$r = Verify $OldSetup "$OldSetup.sig" $realPub "0.16.3" "false"
Check "control: the published 0.16.3 setup.exe and .sig pass the same check against the real pubkey" $r.Accepted $r.Output

Write-Results $ResultsPath "Updater signature"
if ($script:failures.Count -gt 0) {
  throw "$($script:failures.Count) signature check(s) failed: $($script:failures -join '; ')"
}
Write-Host "All signature checks passed."
