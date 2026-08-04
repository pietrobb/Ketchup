[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$expectedFiles = [ordered]@{
    "artifacts/r0/preregistration-lock-r0-v12.json" = "01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176"
    "scripts/windows/run-gate-c-hp-igpu-01.ps1" = "cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164"
    "scripts/windows/validate-r0-v12-preregistration.ps1" = "2efd7ab90ff199c2cd9669fbb603af6ba1db58b1ef264e4d126baed5564c0c56"
    "scripts/windows/write-gate-c-report.ps1" = "21d8c7e3dd820925ff7f0d264e511890c5fb31f2317aeb88b5b8a914f034a4b3"
    "artifacts/gate-c/hp-dev-01-portable-core-r0-v12-provenance.json" = "d24a34e50cfe910aa30f702344ecb951ad96dcdf578917ed3b58e83b3a50d090"
    "artifacts/gate-c/hp-dev-01-portable-nav-r0-v12-provenance.json" = "51de8f7bfdfb9697a66de1edec65d7bb0c447c42ed4846a9834ccabefae983da"
}
$mustBeAbsent = @(
    "artifacts/gate-c/hp-igpu-01-fingerprint-r0-v12.json",
    "artifacts/gate-c/hp-igpu-01-r0-v12-attempt-claim.json",
    "artifacts/gate-c/hp-igpu-01-r0-v12-run-manifest.json",
    "artifacts/gate-c/hp-igpu-01-r0-v12-build.stdout.log",
    "artifacts/gate-c/hp-igpu-01-r0-v12-build.stderr.log",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-1.json",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-2.json",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-3.json",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-1.json",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-2.json",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-3.json",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-1.stdout.log",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-2.stdout.log",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-3.stdout.log",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-1.stderr.log",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-2.stderr.log",
    "artifacts/gate-c/hp-igpu-01-core-r0-v12-series-3.stderr.log",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-1.stdout.log",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-2.stdout.log",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-3.stdout.log",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-1.stderr.log",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-2.stderr.log",
    "artifacts/gate-c/hp-igpu-01-nav-r0-v12-series-3.stderr.log",
    "target/gate-c-r0-v12-hp-igpu-01",
    "artifacts/gate-c/report.md",
    "artifacts/gate-c/report-no-go.md",
    "artifacts/gate-c/report-infrastructure-invalid.md"
)

foreach ($entry in $expectedFiles.GetEnumerator()) {
    $path = Join-Path $repoRoot $entry.Key
    if (-not (Test-Path $path -PathType Leaf)) {
        throw "Gate C transfer is incomplete: missing $($entry.Key)."
    }
    $actual = (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $entry.Value) {
        throw "Gate C transfer hash mismatch for $($entry.Key): expected $($entry.Value), found $actual."
    }
}

foreach ($relative in $mustBeAbsent) {
    if (Test-Path (Join-Path $repoRoot $relative)) {
        throw "Gate C transfer is not pre-observation: unexpected $relative."
    }
}

$reportWriterPath = Join-Path $repoRoot "scripts\windows\write-gate-c-report.ps1"
$tokens = $null
$parseErrors = $null
[void][Management.Automation.Language.Parser]::ParseFile($reportWriterPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) {
    throw "Gate C report writer does not parse: $($parseErrors[0].Message)"
}

$validatorPath = Join-Path $repoRoot "scripts\windows\validate-r0-v12-preregistration.ps1"
$validationOutput = & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $validatorPath 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "R0 v12 validation failed after transfer: $($validationOutput -join [Environment]::NewLine)"
}

Write-Output "Gate C transfer preflight passed: frozen inputs match, the report writer parses, the portable build-provenance self-test passed, and the HP-IGPU-01 evidence and clean-build namespaces are unused."
