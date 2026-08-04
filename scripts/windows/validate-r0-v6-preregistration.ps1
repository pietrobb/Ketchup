[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$oldLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v5.json"
$activeLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v6.json"
$old = Get-Content $oldLockPath -Raw | ConvertFrom-Json
$active = Get-Content $activeLockPath -Raw | ConvertFrom-Json

if ($old.freeze_id -ne "r0-v5" -or $active.freeze_id -ne "r0-v6") {
    throw "Unexpected preregistration lineage."
}
if ($active.measurement_state_at_freeze -ne "not_started") {
    throw "The active lock was not frozen before the replacement A0 run."
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
    throw "r0-v6 may differ from r0-v5 only in Cargo.lock; observed: $($changed -join ', ')"
}

$cargoDiff = & git -C $repoRoot diff "f0a0cf3afa9df45682fe8723dacc99cb8e153058" -- Cargo.lock
if ($LASTEXITCODE -ne 0) { throw "Could not compare Cargo.lock to the r0-v5 source baseline." }
$addedLines = @($cargoDiff | Where-Object { $_ -match '^\+(?!\+)' })
$removedLines = @($cargoDiff | Where-Object { $_ -match '^-(?!--)' })
if ($removedLines.Count -ne 0) { throw "Cargo.lock removed an r0-v5 dependency line." }
$expectedAdded = @(
    '+[[package]]',
    '+name = "ketchup-scheduler"',
    '+version = "0.1.0"',
    '+dependencies = [',
    '+ "ketchup-core",',
    '+ "ketchup-exact",',
    '+]',
    '+'
)
if (($addedLines -join "`n") -ne ($expectedAdded -join "`n")) {
    throw "Cargo.lock changed beyond the expected local ketchup-scheduler package entry."
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

$report = Get-Content (Join-Path $repoRoot "docs\gates\R0_V6_REPORT.md") -Raw
$activeHash = (Get-FileHash $activeLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not $report.Contains("**Decision: GO**") -or -not $report.Contains($activeHash)) {
    throw "R0 v6 report does not authorize this exact lock."
}

Write-Output "R0 v6 validation passed: only the local Gate B scheduler package was added to Cargo.lock; all frozen thresholds, corpora, external dependencies, and toolchain evidence remain byte-identical to r0-v5."
