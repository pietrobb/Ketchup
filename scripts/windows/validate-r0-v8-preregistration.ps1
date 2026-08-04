[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$oldLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v7.json"
$activeLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v8.json"
$old = Get-Content $oldLockPath -Raw | ConvertFrom-Json
$active = Get-Content $activeLockPath -Raw | ConvertFrom-Json

if ($old.freeze_id -ne "r0-v7" -or $active.freeze_id -ne "r0-v8") {
    throw "Unexpected preregistration lineage."
}
if ($active.measurement_state_at_freeze -ne "not_started") {
    throw "The active lock was not frozen before the replacement A0 and Gate C runs."
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
    throw "r0-v8 may differ from r0-v7 only in Cargo.lock; observed: $($changed -join ', ')"
}

$appManifest = Get-Content (Join-Path $repoRoot "crates\ketchup-app\Cargo.toml") -Raw
$requiredBackend = 'wgpu = { version = "25.0.2", default-features = false, features = ["dx12"] }'
if (-not $appManifest.Contains($requiredBackend)) {
    throw "Gate C must enable only the pinned Direct3D 12 wgpu backend."
}
$cargoLock = Get-Content (Join-Path $repoRoot "Cargo.lock") -Raw
foreach ($required in @(
    'name = "ketchup-app"',
    'name = "ketchup-interaction"',
    "name = `"eframe`"`nversion = `"0.32.3`"",
    "name = `"egui`"`nversion = `"0.32.3`"",
    "name = `"wgpu`"`nversion = `"25.0.2`"",
    'name = "gpu-allocator"'
)) {
    if (-not $cargoLock.Contains($required)) { throw "Cargo.lock is missing required Gate C package: $required" }
}

$denyOutput = & cargo deny --manifest-path (Join-Path $repoRoot "Cargo.toml") check licenses sources 2>&1
if ($LASTEXITCODE -ne 0) { throw "Gate C dependency audit failed: $($denyOutput -join [Environment]::NewLine)" }

$thresholds = Get-Content (Join-Path $repoRoot "thresholds\r0.yaml") -Raw | ConvertFrom-Json
$corpora = Get-Content (Join-Path $repoRoot "corpora\manifest.yaml") -Raw | ConvertFrom-Json
if ($thresholds.gates.A0.metrics.ffi_fuzz_call_count_min -ne 10000 -or
    $thresholds.gates.A0.metrics.silent_invalid_shape_max -ne 0 -or
    $thresholds.gates.A0.metrics.guaranteed_identity_correct_percent_min -ne 100 -or
    $thresholds.gates.C.metrics.preview_commit_action_digest_match_percent_min -ne 100) {
    throw "A0 or Gate C thresholds changed."
}
if (@($corpora.fixed).Count -ne 4 -or $corpora.generative.case_count -ne 1000 -or
    @($corpora.mutation.cases).Count -ne 8 -or @($corpora.adversarial.expected_valid).Count -ne 10 -or
    @($corpora.adversarial.expected_rejected).Count -ne 6 -or @($corpora.external_step.fixtures).Count -ne 3) {
    throw "A0 corpus cardinality changed."
}

$report = Get-Content (Join-Path $repoRoot "docs\gates\R0_V8_REPORT.md") -Raw
$activeHash = (Get-FileHash $activeLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not $report.Contains("**Decision: GO**") -or -not $report.Contains($activeHash)) {
    throw "R0 v8 report does not authorize this exact lock."
}

Write-Output "R0 v8 validation passed: the pinned Direct3D 12 wgpu backend is enabled; all frozen thresholds, corpora, licenses, OCCT inputs, hardware profiles, and consequences remain byte-identical to r0-v7."
