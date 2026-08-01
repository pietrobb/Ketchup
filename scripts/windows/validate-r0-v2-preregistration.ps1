[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$oldLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock.json"
$activeLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v2.json"
$old = Get-Content $oldLockPath -Raw | ConvertFrom-Json
$active = Get-Content $activeLockPath -Raw | ConvertFrom-Json

if ($old.freeze_id -ne "r0-v1" -or $active.freeze_id -ne "r0-v2") {
    throw "Unexpected preregistration lineage."
}
if ($active.measurement_state_at_freeze -ne "not_started") {
    throw "The active lock was not frozen before A0."
}
if (@($old.files).Count -ne 16 -or @($active.files).Count -ne 16) {
    throw "The inherited lock file set changed."
}

$oldByPath = @{}
foreach ($entry in @($old.files)) { $oldByPath[[string]$entry.path] = [string]$entry.sha256 }
$changed = @()
foreach ($entry in @($active.files)) {
    $relative = [string]$entry.path
    if (-not $oldByPath.ContainsKey($relative)) { throw "Active lock added an inherited path: $relative" }
    $full = [IO.Path]::GetFullPath((Join-Path $repoRoot $relative))
    $actual = (Get-FileHash $full -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne [string]$entry.sha256) { throw "Active locked hash mismatch: $relative" }
    if ([string]$entry.sha256 -ne $oldByPath[$relative]) { $changed += $relative }
}
if (($changed -join "|") -ne "Cargo.lock") {
    throw "r0-v2 may differ from r0-v1 only in Cargo.lock; observed: $($changed -join ', ')"
}

$thresholds = Get-Content (Join-Path $repoRoot "thresholds\r0.yaml") -Raw | ConvertFrom-Json
$corpora = Get-Content (Join-Path $repoRoot "corpora\manifest.yaml") -Raw | ConvertFrom-Json
if ($thresholds.gates.A0.metrics.ffi_fuzz_call_count_min -ne 10000 -or
    $thresholds.gates.A0.metrics.silent_invalid_shape_max -ne 0 -or
    $thresholds.gates.A0.metrics.guaranteed_identity_correct_percent_min -ne 100) {
    throw "A0 thresholds changed."
}
if (@($corpora.fixed).Count -ne 4 -or $corpora.generative.case_count -ne 1000 -or
    @($corpora.mutation.cases).Count -ne 8 -or @($corpora.adversarial.expected_valid).Count -ne 10 -or
    @($corpora.adversarial.expected_rejected).Count -ne 6 -or @($corpora.external_step.fixtures).Count -ne 3) {
    throw "A0 corpus cardinality changed."
}

$report = Get-Content (Join-Path $repoRoot "docs\gates\R0_V2_REPORT.md") -Raw
$activeHash = (Get-FileHash $activeLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not $report.Contains("**Decision: GO**") -or -not $report.Contains($activeHash)) {
    throw "R0 v2 report does not authorize this exact lock."
}

Write-Output "R0 v2 validation passed: only Cargo.lock changed before A0; all frozen thresholds and corpora remain byte-identical to r0-v1."
