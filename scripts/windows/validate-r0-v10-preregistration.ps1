[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$oldLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v9.json"
$activeLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v10.json"
$old = Get-Content $oldLockPath -Raw | ConvertFrom-Json
$active = Get-Content $activeLockPath -Raw | ConvertFrom-Json

if ($old.freeze_id -ne "r0-v9" -or $active.freeze_id -ne "r0-v10") {
    throw "Unexpected preregistration lineage."
}
if ($active.measurement_state_at_freeze -ne "not_started") {
    throw "The active lock was not frozen before the replacement A0 and Gate C runs."
}
if (@($old.files).Count -ne 16 -or @($active.files).Count -ne 18) {
    throw "The r0-v10 lock must preserve 16 inherited paths and add exactly two renderer source paths."
}

$oldByPath = @{}
foreach ($entry in @($old.files)) { $oldByPath[[string]$entry.path] = [string]$entry.sha256 }
$added = @()
foreach ($entry in @($active.files)) {
    $relative = [string]$entry.path
    $full = [IO.Path]::GetFullPath((Join-Path $repoRoot $relative))
    $actual = (Get-FileHash $full -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne [string]$entry.sha256) { throw "Active locked hash mismatch: $relative" }
    if ($oldByPath.ContainsKey($relative)) {
        if ([string]$entry.sha256 -ne $oldByPath[$relative]) {
            throw "Inherited r0-v9 path changed: $relative"
        }
    } else {
        $added += $relative
    }
}
$expectedAdded = @(
    "crates/ketchup-app/src/bin/ketchup-gate-c-nav.rs",
    "crates/ketchup-app/src/lib.rs"
)
if ((@($added | Sort-Object) -join "|") -ne ($expectedAdded -join "|")) {
    throw "Unexpected r0-v10 source additions: $($added -join ', ')"
}

$appSource = Get-Content (Join-Path $repoRoot "crates\ketchup-app\src\lib.rs") -Raw
if (-not $appSource.Contains("renderer: eframe::Renderer::Wgpu") -or
    -not $appSource.Contains("vsync: false")) {
    throw "Gate C replacement must preserve wgpu and explicitly disable vertical synchronization."
}
$appManifest = Get-Content (Join-Path $repoRoot "crates\ketchup-app\Cargo.toml") -Raw
$requiredBackend = 'wgpu = { version = "25.0.2", default-features = false, features = ["dx12"] }'
if (-not $appManifest.Contains($requiredBackend)) {
    throw "Gate C must enable only the pinned Direct3D 12 wgpu backend."
}

$failed = Get-Content (Join-Path $repoRoot "artifacts\gate-c\hp-dev-01-nav-series-1.json") -Raw | ConvertFrom-Json
if ($failed.profile_id -ne "HP-DEV-01" -or $failed.series -ne 1 -or $failed.runs -ne 30 -or
    $failed.r0_lock_sha256 -ne "da0dbcd3b3daf845a83f6a708a528c7cdcbf8e0155d1d93bfbb9637c539a7b25" -or
    $failed.frame_p95_ms -ne 17.3552 -or $failed.frame_p95_ms -le 16.7) {
    throw "The immutable r0-v9 navigation failure evidence is missing or changed."
}

$thresholds = Get-Content (Join-Path $repoRoot "thresholds\r0.yaml") -Raw | ConvertFrom-Json
$nav = @($thresholds.query_classes | Where-Object id -eq "QC-C-NAV-01")[0]
if ($nav.metrics.frame_p95_ms_max -ne 16.7 -or $nav.metrics.frame_p99_ms_max -ne 33.3 -or
    $nav.metrics.input_to_preview_p95_ms_max -ne 50 -or
    $thresholds.gates.C.metrics.preview_commit_action_digest_match_percent_min -ne 100) {
    throw "Gate C thresholds changed."
}

$denyOutput = & cargo deny --manifest-path (Join-Path $repoRoot "Cargo.toml") check licenses sources 2>&1
if ($LASTEXITCODE -ne 0) { throw "Gate C dependency audit failed: $($denyOutput -join [Environment]::NewLine)" }

$report = Get-Content (Join-Path $repoRoot "docs\gates\R0_V10_REPORT.md") -Raw
$activeHash = (Get-FileHash $activeLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not $report.Contains("**Decision: GO**") -or -not $report.Contains($activeHash)) {
    throw "R0 v10 report does not authorize this exact lock."
}

Write-Output "R0 v10 validation passed: the r0-v9 failure remains immutable, non-vsync Direct3D 12 presentation is frozen, and all inherited thresholds, corpora, hardware profiles, and consequences remain unchanged."
