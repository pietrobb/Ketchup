# The installer and the portable archive carry the third-party notices; a package
# missing one is refused. Needs the pinned OCCT install for its license texts.
[CmdletBinding()]
param([string]$OcctRoot)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($OcctRoot)) {
    $OcctRoot = Join-Path $repoRoot "third_party\occt-install-r0-v1"
}
. (Join-Path $PSScriptRoot "package-notices.ps1")

function Assert-Refused([scriptblock]$Check, [string]$Case) {
    $refused = $false
    try { & $Check } catch { $refused = $true }
    if (-not $refused) { throw "FAIL: $Case was accepted." }
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-notices-" + [Guid]::NewGuid().ToString("N"))
try {
    $payload = Join-Path $root "payload"
    [void](New-Item $payload -ItemType Directory)
    Copy-PackageNotices $repoRoot $OcctRoot $payload
    $zip = Join-Path $root "portable.zip"
    Compress-Archive -Path (Join-Path $payload "*") -DestinationPath $zip
    Assert-PackageNotices $payload "The payload"
    Assert-PackageNotices $zip "The archive"

    $notices = Get-Content (Join-Path $payload "THIRD_PARTY_NOTICES.txt") -Raw
    foreach ($needle in @("OCCT", "LGPL", "acadrust", "MPL", "https://github.com/hakanaktt/acadrust",
            "starlark", "Apache License 2.0", "PATCH.md", "CORE-MATH", "LICENSE-CORE-MATH-MIT.txt",
            "81a899d67953a619d703660fd9630f0554673524")) {
        if (-not $notices.Contains($needle)) { throw "FAIL: THIRD_PARTY_NOTICES.txt does not name $needle." }
    }

    Remove-Item (Join-Path $payload "LICENSE-OCCT-LGPL-EXCEPTION.txt")
    Assert-Refused { Assert-PackageNotices $payload "The payload" } "a payload without the OCCT exception"
    $partial = Join-Path $root "partial.zip"
    Compress-Archive -Path (Join-Path $payload "*") -DestinationPath $partial
    Assert-Refused { Assert-PackageNotices $partial "The archive" } "an archive without the OCCT exception"
    Assert-Refused { Copy-PackageNotices $repoRoot (Join-Path $root "no-occt") $payload } "a missing OCCT license source"
} finally {
    if (Test-Path $root) { Remove-Item $root -Recurse -Force }
}
Write-Host "PASS: payload and portable archive carry the third-party notices; a package without one is refused."
