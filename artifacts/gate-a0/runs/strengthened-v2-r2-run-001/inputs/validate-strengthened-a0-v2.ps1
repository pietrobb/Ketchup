[CmdletBinding()]
param([switch]$EmitJson)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$lockPath = Join-Path $repoRoot "artifacts\gate-a0\strengthened-a0-v2-r2-lock.json"
$preregistrationPath = Join-Path $repoRoot "artifacts\gate-a0\strengthened-a0-v2-r2-preregistration.json"
$pythonPath = "C:\Python311\python.exe"
$expectedPythonSha256 = "5f7b89a612c9b8af1d6456cdfcd1dbe5ca630849e79aebced9bee9a6694952ec"

function Get-TreeFingerprint([string]$Root) {
    $code = @'
import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1])
files = sorted((p for p in root.rglob('*') if p.is_file()), key=lambda p: p.relative_to(root).as_posix().lower())
lines = []
for path in files:
    relative = path.relative_to(root).as_posix()
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    lines.append(f'{relative}|{path.stat().st_size}|{digest}\n')
print(json.dumps({'file_count': len(files), 'sha256': hashlib.sha256(''.join(lines).encode('utf-8')).hexdigest()}))
'@
    $raw = & $pythonPath -c $code ([IO.Path]::GetFullPath($Root))
    if ($LASTEXITCODE -ne 0) { throw "Frozen Python tree-hash tool failed for $Root" }
    return $raw | ConvertFrom-Json
}

if (-not (Test-Path $pythonPath -PathType Leaf) -or
    (Get-FileHash $pythonPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedPythonSha256) {
    throw "Missing or changed frozen Python tree-hash tool."
}
if (-not (Test-Path $lockPath -PathType Leaf)) { throw "Missing strengthened A0 v2 lock: $lockPath" }
if (-not (Test-Path $preregistrationPath -PathType Leaf)) { throw "Missing strengthened A0 v2 preregistration: $preregistrationPath" }

$lock = Get-Content $lockPath -Raw | ConvertFrom-Json
$preregistration = Get-Content $preregistrationPath -Raw | ConvertFrom-Json
if ($lock.freeze_id -ne "strengthened-a0-v2-r2" -or $lock.measurement_state_at_freeze -ne "not_started") {
    throw "Unexpected strengthened A0 v2 freeze identity or measurement state."
}
if ($preregistration.gate_id -ne "strengthened-a0-v2-r2" -or
    $preregistration.status -ne "preregistered_before_observation" -or
    $preregistration.measurement_state_at_preregistration -ne "not_started") {
    throw "Strengthened A0 v2 was not preregistered before observation."
}
$thresholdsPath = Join-Path $repoRoot ([string]$preregistration.inherited_contract.thresholds_path)
$thresholdsSha256 = (Get-FileHash $thresholdsPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($preregistration.inherited_contract.operation_envelope_change -ne "none" -or
    $preregistration.inherited_contract.threshold_change -ne "none" -or
    $preregistration.inherited_contract.consequence_change -ne "none" -or
    $thresholdsSha256 -ne [string]$preregistration.inherited_contract.thresholds_sha256 -or
    $preregistration.inherited_contract.required_counts_per_backend.fixed_expected_valid -ne 4 -or
    $preregistration.inherited_contract.required_counts_per_backend.ffi_fuzz_calls_min -ne 10000 -or
    $preregistration.inherited_contract.required_counts_per_backend.adversarial_expected_valid -ne 10 -or
    $preregistration.inherited_contract.required_counts_per_backend.typed_rejections -ne 6 -or
    $preregistration.inherited_contract.required_counts_per_backend.guaranteed_mutation_outcomes -ne 24 -or
    $preregistration.inherited_contract.required_counts_per_backend.step_fixtures -ne 3 -or
    $preregistration.inherited_contract.required_backend_suite_passes -ne 2 -or
    $preregistration.inherited_contract.measurement_build -notmatch "^Release") {
    throw "A0 v2 inherited contract or frozen threshold identity changed."
}
$requiredCombinations = @("prior-to-prior", "current-to-current", "prior-to-current", "current-to-prior")
$actualCombinations = @($preregistration.backend_matrix.required_combinations)
$requiredSameBuildCombinations = @("prior-to-prior", "current-to-current")
$actualSameBuildCombinations = @($preregistration.backend_matrix.same_build_go_required_combinations)
if ($preregistration.backend_matrix.full_go_required_pass_count -ne 4 -or
    (($actualCombinations | Sort-Object) -join "|") -ne (($requiredCombinations | Sort-Object) -join "|") -or
    (($actualSameBuildCombinations | Sort-Object) -join "|") -ne (($requiredSameBuildCombinations | Sort-Object) -join "|") -or
    $preregistration.backend_matrix.intentional_invalid_reference_negative_control.same_build_expected -ne "Lost" -or
    $preregistration.backend_matrix.intentional_invalid_reference_negative_control.cross_build_expected -ne "QuarantinedMigration" -or
    @($preregistration.backend_matrix.intentional_invalid_reference_negative_control.forbidden).Count -ne 2) {
    throw "A0 v2 matrix or negative-control oracle changed."
}
if ($preregistration.decision_rule.operator_disposition -notmatch "FULL_GO withdraws L-01/L-02" -or
    $preregistration.decision_rule.operator_disposition -notmatch "SAME_BUILD_GO retains L-01/L-02" -or
    $preregistration.decision_rule.operator_disposition -notmatch "Both release the M3 halt" -or
    $preregistration.decision_rule.full_go_effect -notmatch "Withdraw L-01/L-02" -or
    $preregistration.decision_rule.same_build_go_effect -notmatch "Retain L-01/L-02") {
    throw "A0 v2 success dispositions changed after owner direction."
}

foreach ($anchor in $preregistration.historical_anchors.PSObject.Properties.Value) {
    $path = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$anchor.path)))
    if (-not $path.StartsWith($repoRoot.TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "Historical anchor escapes the repository: $($anchor.path)"
    }
    if (-not (Test-Path $path -PathType Leaf)) { throw "Missing historical anchor: $($anchor.path)" }
    if ((Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne [string]$anchor.sha256) {
        throw "Historical anchor changed: $($anchor.path)"
    }
}
$diagnosticSealPath = Join-Path $repoRoot "artifacts\gate-a0\diagnostics\a0d-run-002\seal.json"
$diagnosticRoot = Split-Path $diagnosticSealPath -Parent
$diagnosticSeal = Get-Content $diagnosticSealPath -Raw | ConvertFrom-Json
foreach ($entry in @($diagnosticSeal.files)) {
    $sealedPath = [IO.Path]::GetFullPath((Join-Path $diagnosticRoot ([string]$entry.path)))
    if (-not $sealedPath.StartsWith($diagnosticRoot.TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path $sealedPath -PathType Leaf) -or
        (Get-FileHash $sealedPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne [string]$entry.sha256) {
        throw "A0-D sealed evidence changed or escaped its run directory: $($entry.path)"
    }
}
$failedRunSealPath = Join-Path $repoRoot "artifacts\gate-a0\runs\strengthened-v2-run-001\seal.json"
$failedRunRoot = Split-Path $failedRunSealPath -Parent
$failedRunSeal = Get-Content $failedRunSealPath -Raw | ConvertFrom-Json
foreach ($entry in @($failedRunSeal.files)) {
    $sealedPath = [IO.Path]::GetFullPath((Join-Path $failedRunRoot ([string]$entry.path)))
    if (-not $sealedPath.StartsWith($failedRunRoot.TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path $sealedPath -PathType Leaf) -or
        (Get-FileHash $sealedPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne [string]$entry.sha256) {
        throw "A0 v2 run-001 sealed evidence changed or escaped its run directory: $($entry.path)"
    }
}

$requiredFiles = @(
    "artifacts/gate-a0/strengthened-a0-v2-r2-preregistration.json",
    "artifacts/gate-a0/strengthened-a0-v2-lock.json",
    "artifacts/gate-a0/runs/strengthened-v2-run-001/seal.json",
    "artifacts/gate-a0/strengthened-a0-v1-lock.json",
    "artifacts/gate-a0/runs/strengthened-run-001/metrics.json",
    "artifacts/gate-a0/runs/strengthened-run-001/report.md",
    "artifacts/gate-a0/diagnostics/a0d-run-002/seal.json",
    "docs/adr/0004-v4-p15-sequence-and-a0-disposition.md",
    "docs/adr/0005-no-go-diagnostic-hold.md",
    "docs/architecture/EXECUTION_CONTRACT.md",
    "thresholds/r0.yaml",
    "corpora/manifest.yaml",
    "corpora/r0/step/self-authored-box.step",
    "corpora/r0/step/self-authored-through-cut.step",
    "corpora/r0/step/self-authored-l-bracket.step",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "crates/ketchup-exact/Cargo.toml",
    "crates/ketchup-exact/build.rs",
    "crates/ketchup-exact/include/ketchup_exact.hxx",
    "crates/ketchup-exact/src/lib.rs",
    "crates/ketchup-exact/src/native.cc",
    "crates/ketchup-exact/src/bin/ketchup-a0-diagnostic-probe.rs",
    "crates/ketchup-exact/tests/gate_a0_v2.rs",
    "scripts/windows/validate-strengthened-a0-v2.ps1",
    "scripts/windows/run-strengthened-a0-v2.ps1",
    "artifacts/r0/occt-build-manifest.json"
)
$lockedPaths = @($lock.files | ForEach-Object { [string]$_.path })
if ($lockedPaths.Count -ne $requiredFiles.Count -or
    (($lockedPaths | Sort-Object) -join "|") -ne (($requiredFiles | Sort-Object) -join "|")) {
    throw "Strengthened A0 v2 lock does not contain the exact preregistered input set."
}
foreach ($entry in @($lock.files)) {
    $fullPath = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$entry.path)))
    if (-not $fullPath.StartsWith($repoRoot.TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "Locked input escapes the repository: $($entry.path)"
    }
    if (-not (Test-Path $fullPath -PathType Leaf)) { throw "Missing locked input: $($entry.path)" }
    $actual = (Get-FileHash $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne [string]$entry.sha256) { throw "Strengthened A0 v2 locked hash mismatch: $($entry.path)" }
}
$nativeEntry = @($lock.files | Where-Object { $_.path -eq "crates/ketchup-exact/src/native.cc" })[0]
$thresholdEntry = @($lock.files | Where-Object { $_.path -eq "thresholds/r0.yaml" })[0]
if ($null -eq $nativeEntry -or $null -eq $thresholdEntry -or
    [string]$lock.evaluator_source_sha256 -ne [string]$nativeEntry.sha256 -or
    [string]$lock.tolerance_profile_sha256 -ne [string]$thresholdEntry.sha256 -or
    [string]$thresholdEntry.sha256 -ne $thresholdsSha256) {
    throw "Evaluator or tolerance identity is not derived from its locked source."
}

$backends = @($lock.backends)
if ($backends.Count -ne 2 -or
    [string]$backends[0].alias -ne "prior" -or
    [string]$backends[0].id -ne "prior-real-build" -or
    [string]$backends[0].install_path -ne "third_party/occt-install" -or
    [int]$backends[0].file_count -ne 7400 -or
    [string]$backends[1].alias -ne "current" -or
    [string]$backends[1].id -ne "current-r0-v1-build" -or
    [string]$backends[1].install_path -ne "third_party/occt-install-r0-v1" -or
    [int]$backends[1].file_count -ne 7400) {
    throw "Strengthened A0 v2 lock does not name the exact two frozen backend builds."
}
$backendResults = @()
foreach ($backend in $backends) {
    $root = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$backend.install_path)))
    if (-not $root.StartsWith((Join-Path $repoRoot "third_party").TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "Backend install escapes third_party: $($backend.install_path)"
    }
    $tree = Get-TreeFingerprint $root
    if ($tree.file_count -ne [int]$backend.file_count -or $tree.sha256 -ne [string]$backend.tree_sha256) {
        throw "Backend install tree mismatch: $($backend.id)"
    }
    $libraries = @($backend.representative_libraries)
    $requiredLibraries = @("win64/vc14/bin/TKernel.dll", "win64/vc14/bin/TKBRep.dll", "win64/vc14/bin/TKPrim.dll")
    if ((@($libraries | ForEach-Object { [string]$_.path } | Sort-Object) -join "|") -ne (($requiredLibraries | Sort-Object) -join "|")) {
        throw "Backend representative-library set changed: $($backend.id)"
    }
    foreach ($library in $libraries) {
        $libraryPath = Join-Path $root ([string]$library.path)
        if ((Get-FileHash $libraryPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne [string]$library.sha256) {
            throw "Backend library mismatch: $($backend.id)/$($library.path)"
        }
    }
    $backendResults += [ordered]@{
        alias = [string]$backend.alias
        id = [string]$backend.id
        install_path = $root
        tree_sha256 = [string]$backend.tree_sha256
        fingerprint = [string]$backend.fingerprint
        tkernel_sha256 = [string](@($libraries | Where-Object { $_.path -eq "win64/vc14/bin/TKernel.dll" })[0].sha256)
        representative_libraries = $libraries
    }
}
if ($backendResults[0].tree_sha256 -eq $backendResults[1].tree_sha256 -or
    $backendResults[0].tkernel_sha256 -eq $backendResults[1].tkernel_sha256 -or
    $backendResults[0].fingerprint -eq $backendResults[1].fingerprint) {
    throw "Strengthened A0 v2 requires two distinct real backend builds."
}

$result = [ordered]@{
    freeze_id = "strengthened-a0-v2-r2"
    lock_sha256 = (Get-FileHash $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
    evaluator_source_sha256 = [string]$lock.evaluator_source_sha256
    tolerance_profile_sha256 = [string]$lock.tolerance_profile_sha256
    backends = $backendResults
}
if ($EmitJson) {
    $result | ConvertTo-Json -Depth 5 -Compress
} else {
    Write-Output "Strengthened A0 v2 preflight passed for lock $($result.lock_sha256), repaired evaluator $($result.evaluator_source_sha256), and two frozen OCCT builds."
}
