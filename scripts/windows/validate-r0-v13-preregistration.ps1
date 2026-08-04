[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$oldLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v12.json"
$activeLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v13.json"
$old = Get-Content $oldLockPath -Raw | ConvertFrom-Json
$active = Get-Content $activeLockPath -Raw | ConvertFrom-Json
$expectedActiveHash = "b1cf0c769cb46d0c678c1bc579e241356cc85663582a0df72093e2e54086cb01"
$runnerPath = Join-Path $repoRoot "scripts\windows\run-gate-c-hp-igpu-01.ps1"
$expectedRunnerHash = "8bade6a87253ebdac41f4cf9f92acc11a84ddec7e907cce9e2ad16af7cdcf564"
$referenceControllerPath = Join-Path $repoRoot "scripts\windows\run-gate-c-hp-dev-01-r0-v13.ps1"
$expectedReferenceControllerHash = "588a6a8be01446a11ede1a06a28c08987ccd06fa7da14ec2d3b842173b9ab38c"

if ($old.freeze_id -ne "r0-v12" -or $active.freeze_id -ne "r0-v13") {
    throw "Unexpected preregistration lineage."
}
if ($active.measurement_state_at_freeze -ne "not_started") {
    throw "The active lock was not frozen before replacement observations."
}
if (@($old.files).Count -ne 18 -or @($active.files).Count -ne 18) {
    throw "The r0-v13 lock must preserve all 18 r0-v12 paths."
}
$activeHash = (Get-FileHash $activeLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($activeHash -ne $expectedActiveHash) { throw "The r0-v13 lock hash changed." }
if ((Get-FileHash $runnerPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedRunnerHash) {
    throw "The pre-observation HP-IGPU-01 runner changed after its portable provenance repair was frozen."
}
if ((Get-FileHash $referenceControllerPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedReferenceControllerHash) {
    throw "The pre-observation HP-DEV-01 reference controller changed after it was frozen."
}
$runnerSource = Get-Content $runnerPath -Raw
foreach ($requiredRunnerContract in @(
    "Write-Utf8JsonExclusive", "Invoke-RecordedStage", "INFRASTRUCTURE_INVALID",
    "runner_script_sha256", "failing_stage", "attempt-claim", '$VerifyAttemptSealing',
    'Write-Utf8JsonExclusive $fingerprintPath', "Get-BuildInputManifest", "Get-BuildProvenance",
    "build_input_tree_sha256", "occt_install_tree_sha256", '$measurementTargetDir'
)) {
    if (-not $runnerSource.Contains($requiredRunnerContract)) {
        throw "The HP-IGPU-01 runner is missing the sealed-attempt contract: $requiredRunnerContract"
    }
}
if ($runnerSource.Contains('$expectedExecutableHashes') -or
    $runnerSource.Contains('differs from the HP-DEV-01 observed binary') -or
    -not $runnerSource.Contains('de8592b10b5ed88d2ae7cf8394c127d3d7ca1ea8b22830911cc28a8fbdca84bb')) {
    throw "The HP-IGPU-01 runner does not enforce the frozen portable build-provenance contract."
}
$attemptTestOutput = & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $runnerPath -VerifyAttemptSealing 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "The HP-IGPU-01 attempt-sealing self-test failed: $($attemptTestOutput -join [Environment]::NewLine)"
}

$oldByPath = @{}
foreach ($entry in @($old.files)) { $oldByPath[[string]$entry.path] = [string]$entry.sha256 }
$changed = @()
foreach ($entry in @($active.files)) {
    $relative = [string]$entry.path
    if (-not $oldByPath.ContainsKey($relative)) { throw "Unexpected r0-v13 path: $relative" }
    $full = [IO.Path]::GetFullPath((Join-Path $repoRoot $relative))
    $actual = (Get-FileHash $full -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne [string]$entry.sha256) { throw "Active locked hash mismatch: $relative" }
    if ([string]$entry.sha256 -ne $oldByPath[$relative]) { $changed += $relative }
}
$expectedChanges = "crates/ketchup-app/src/lib.rs"
if (($changed -join "|") -ne $expectedChanges) {
    throw "r0-v13 may change only the repaired product viewport source among the inherited R0 lock paths; observed: $($changed -join ', ')"
}

$appSource = Get-Content (Join-Path $repoRoot "crates\ketchup-app\src\lib.rs") -Raw
if (-not $appSource.Contains("native_adapter_selector = Some") -or
    -not $appSource.Contains("Backends::DX12") -or
    -not $appSource.Contains("DeviceType::Cpu | eframe::wgpu::DeviceType::VirtualGpu") -or
    -not $appSource.Contains("adapter.is_surface_supported(surface)")) {
    throw "The product renderer does not enforce the frozen Direct3D 12 physical-adapter selection contract."
}
$navSource = Get-Content (Join-Path $repoRoot "crates\ketchup-app\src\bin\ketchup-gate-c-nav.rs") -Raw
if (-not $navSource.Contains("expected-adapter-name") -or
    -not $navSource.Contains("DeviceType::IntegratedGpu") -or
    -not $navSource.Contains("selected_adapter") -or
    -not $navSource.Contains("adapter_info")) {
    throw "The NAV harness does not bind and record the selected adapter."
}
$appManifest = Get-Content (Join-Path $repoRoot "crates\ketchup-app\Cargo.toml") -Raw
$requiredBackend = 'wgpu = { version = "25.0.2", default-features = false, features = ["dx12"] }'
if (-not $appManifest.Contains($requiredBackend)) {
    throw "Gate C must enable only the pinned Direct3D 12 wgpu backend."
}

$historical = Get-Content (Join-Path $repoRoot "artifacts\gate-c\hp-dev-01-portable-nav-r0-v12-provenance.json") -Raw | ConvertFrom-Json
if ($historical.profile_id -ne "HP-DEV-01" -or $historical.freeze_id -ne "r0-v12" -or
    $historical.decision -ne "PASS" -or
    $historical.r0_lock_sha256 -ne "01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176" -or
    @($historical.metrics).Count -ne 3) {
    throw "The immutable r0-v12 navigation evidence is missing or changed."
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

$report = Get-Content (Join-Path $repoRoot "docs\gates\R0_V13_REPORT.md") -Raw
if (-not $report.Contains("**Decision: GO**") -or -not $report.Contains($activeHash)) {
    throw "R0 v13 report does not authorize this exact lock."
}

Write-Output "R0 v13 validation passed: historical evidence is immutable, the repaired viewport and build-input tree are frozen, adapter selection is bound and recorded, and all thresholds and inherited inputs remain unchanged."
