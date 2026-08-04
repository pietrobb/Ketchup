[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$lockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v13.json"
$reportPath = Join-Path $repoRoot "docs\gates\R0_V13_REPORT.md"
$provenancePath = Join-Path $repoRoot "artifacts\gate-c\hp-dev-01-portable-core-r0-v13-provenance.json"
$runnerPath = Join-Path $repoRoot "scripts\windows\run-gate-c-hp-igpu-01.ps1"
$controllerPath = Join-Path $repoRoot "scripts\windows\run-gate-c-hp-dev-01-r0-v13.ps1"
$expectedLockHash = "b1cf0c769cb46d0c678c1bc579e241356cc85663582a0df72093e2e54086cb01"
$expectedBuildTreeHash = "de8592b10b5ed88d2ae7cf8394c127d3d7ca1ea8b22830911cc28a8fbdca84bb"
$expectedRunnerHash = "8bade6a87253ebdac41f4cf9f92acc11a84ddec7e907cce9e2ad16af7cdcf564"
$expectedControllerHash = "588a6a8be01446a11ede1a06a28c08987ccd06fa7da14ec2d3b842173b9ab38c"

if ((Get-FileHash $lockPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedLockHash) {
    throw "Historical r0-v13 lock changed."
}
$lock = Get-Content $lockPath -Raw | ConvertFrom-Json
if ($lock.freeze_id -ne "r0-v13" -or $lock.measurement_state_at_freeze -ne "not_started" -or @($lock.files).Count -ne 18) {
    throw "Historical r0-v13 lock identity or preregistration state changed."
}
if ((Get-FileHash $runnerPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedRunnerHash -or
    (Get-FileHash $controllerPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedControllerHash) {
    throw "Historical r0-v13 runner or controller changed."
}

$report = Get-Content $reportPath -Raw
foreach ($required in @("**Decision: GO**", $expectedLockHash, $expectedBuildTreeHash)) {
    if (-not $report.Contains($required)) { throw "Historical r0-v13 report lost required evidence: $required" }
}

$provenance = Get-Content $provenancePath -Raw | ConvertFrom-Json
if ($provenance.schema_version -ne 1 -or
    $provenance.profile_id -ne "HP-DEV-01" -or
    $provenance.freeze_id -ne "r0-v13" -or
    $provenance.contract -ne "portable-build-provenance-v1" -or
    $provenance.r0_lock_sha256 -ne $expectedLockHash -or
    $provenance.build_input_tree_sha256 -ne $expectedBuildTreeHash -or
    $provenance.runner_script_sha256 -ne $expectedRunnerHash -or
    $provenance.controller_script_sha256 -ne $expectedControllerHash -or
    -not $provenance.clean_build -or
    $provenance.decision -ne "PASS" -or
    @($provenance.metrics).Count -ne 3) {
    throw "Historical r0-v13 provenance changed or is incomplete."
}
foreach ($metric in @($provenance.metrics)) {
    if ($metric.action_digest_match_percent -ne 100 -or
        $metric.wrong_identity_count -ne 0 -or
        $metric.committed_data_loss_count -ne 0) {
        throw "Historical r0-v13 correctness metrics changed."
    }
}

$repoPrefix = $repoRoot.TrimEnd("\") + "\"
foreach ($entry in @($provenance.files)) {
    $fullPath = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$entry.path)))
    if (-not $fullPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path $fullPath -PathType Leaf)) {
        throw "Historical evidence path is unsafe or missing: $($entry.path)"
    }
    $actualHash = (Get-FileHash $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne [string]$entry.sha256) { throw "Historical evidence changed: $($entry.path)" }
}

Write-Output "Historical r0-v13 evidence passed: immutable lock, runners, GO report, provenance, and nine evidence files remain sealed."
