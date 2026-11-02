# The installer, the portable archive and the Viewer archive carry the third-party
# notices of what they contain (review 2026-11-01 REL-100); a package missing one is
# refused. Needs the pinned OCCT install for its license texts and cargo-about.
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

function Assert-Names([string]$Notices, [string[]]$Needles, [string]$Label) {
    foreach ($needle in $Needles) {
        if (-not $Notices.Contains($needle)) { throw "FAIL: $Label does not name $needle." }
    }
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-notices-" + [Guid]::NewGuid().ToString("N"))
try {
    $payload = Join-Path $root "payload"
    [void](New-Item $payload -ItemType Directory)
    Copy-PackageNotices $repoRoot $OcctRoot $payload "ketchup"
    $zip = Join-Path $root "portable.zip"
    Compress-Archive -Path (Join-Path $payload "*") -DestinationPath $zip
    Assert-PackageNotices $payload "The payload" "ketchup"
    Assert-PackageNotices $zip "The archive" "ketchup"

    $notices = Get-Content (Join-Path $payload "THIRD_PARTY_NOTICES.txt") -Raw
    # The components with their own conditions, then each crate with its copyright
    # and license text (the MIT permission notice, not only the word MIT).
    Assert-Names $notices @("OCCT", "LGPL", "acadrust", "MPL", "https://github.com/hakanaktt/acadrust",
        "starlark", "Apache License 2.0", "PATCH.md", "CORE-MATH", "LICENSE-CORE-MATH-MIT.txt",
        "81a899d67953a619d703660fd9630f0554673524", "Rust crates", "egui 0.32.3", "Copyright",
        "Permission is hereby granted, free of charge") "THIRD_PARTY_NOTICES.txt"

    $viewer = Join-Path $root "viewer"
    [void](New-Item $viewer -ItemType Directory)
    Copy-PackageNotices $repoRoot $OcctRoot $viewer "viewer"
    Assert-PackageNotices $viewer "The Viewer payload" "viewer"
    $viewerNotices = Get-Content (Join-Path $viewer "THIRD_PARTY_NOTICES.txt") -Raw
    Assert-Names $viewerNotices @("Kecup Viewer", "egui 0.32.3", "Copyright",
        "Permission is hereby granted, free of charge") "the Viewer's THIRD_PARTY_NOTICES.txt"
    # The Viewer contains neither OCCT, CORE-MATH, starlark nor acadrust.
    foreach ($absent in @("OCCT", "CORE-MATH", "starlark", "acadrust")) {
        if ($viewerNotices.Contains($absent)) { throw "FAIL: the Viewer's notices name $absent, which it does not contain." }
    }
    if (@(Get-ChildItem $viewer -Filter "LICENSE-*").Count -ne 0) {
        throw "FAIL: the Viewer payload carries license texts of components it does not contain."
    }

    Remove-Item (Join-Path $payload "LICENSE-OCCT-LGPL-EXCEPTION.txt")
    Assert-Refused { Assert-PackageNotices $payload "The payload" "ketchup" } "a payload without the OCCT exception"
    $partial = Join-Path $root "partial.zip"
    Compress-Archive -Path (Join-Path $payload "*") -DestinationPath $partial
    Assert-Refused { Assert-PackageNotices $partial "The archive" "ketchup" } "an archive without the OCCT exception"
    Assert-Refused { Copy-PackageNotices $repoRoot (Join-Path $root "no-occt") $payload "ketchup" } "a missing OCCT license source"
    Remove-Item (Join-Path $viewer "THIRD_PARTY_NOTICES.txt")
    Assert-Refused { Assert-PackageNotices $viewer "The Viewer payload" "viewer" } "a Viewer payload without its notices"
} finally {
    if (Test-Path $root) { Remove-Item $root -Recurse -Force }
}
Write-Host "PASS: the payload, the portable archive and the Viewer carry the notices of what they contain; a package without one is refused."
