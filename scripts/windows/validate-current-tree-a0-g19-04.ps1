[CmdletBinding()]
param(
    [string]$FreezeId = "current-tree-a0-g19-04-v1",
    [switch]$EmitJson
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$repoPrefix = $repoRoot.TrimEnd("\") + "\"
$freezeId = $FreezeId
if ($freezeId -notin @("current-tree-a0-g19-04-v1", "current-tree-a0-g19-04-v2", "current-tree-a0-g19-04-v3", "current-tree-a0-g19-04-v4", "current-tree-a0-g19-04-v5", "current-tree-a0-g19-04-v6")) {
    throw "FreezeId must be an approved current-tree G19-04 A0 namespace."
}
$artifactRoot = Join-Path $repoRoot "artifacts\gate-a0"
$preregistrationPath = Join-Path $artifactRoot "$freezeId-preregistration.json"
$lockPath = Join-Path $artifactRoot "$freezeId-lock.json"
$pythonPath = "C:\Python311\python.exe"
$expectedPythonSha256 = "5f7b89a612c9b8af1d6456cdfcd1dbe5ca630849e79aebced9bee9a6694952ec"

function Get-Sha256([string]$Path) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "Missing current-tree A0 input: $Path" }
    return (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-TextSha256([string]$Text) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
        return [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

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

function Get-SourceRegistry {
    $paths = [Collections.Generic.List[string]]::new()
    foreach ($path in @(& git -C $repoRoot ls-files -- Cargo.toml Cargo.lock rust-toolchain.toml crates locales scripts corpora thresholds governance)) {
        if (-not [string]::IsNullOrWhiteSpace([string]$path)) { $paths.Add(([string]$path).Replace("\", "/")) }
    }
    if ($LASTEXITCODE -ne 0) { throw "git ls-files failed during current-tree A0 validation." }
    foreach ($path in @(& git -C $repoRoot ls-files --others --exclude-standard -- Cargo.toml Cargo.lock rust-toolchain.toml crates locales scripts corpora thresholds governance)) {
        if (-not [string]::IsNullOrWhiteSpace([string]$path)) { $paths.Add(([string]$path).Replace("\", "/")) }
    }
    if ($LASTEXITCODE -ne 0) { throw "git untracked source enumeration failed." }
    $records = @($paths | Sort-Object -Unique | ForEach-Object {
        $absolute = Join-Path $repoRoot $_
        if (-not (Test-Path $absolute -PathType Leaf)) { throw "Source disappeared during validation: $_" }
        [ordered]@{ path = $_; size_bytes = (Get-Item $absolute).Length; sha256 = Get-Sha256 $absolute }
    })
    $canonical = (@($records | ForEach-Object { "$($_.path)|$($_.size_bytes)|$($_.sha256)" }) -join "`n") + "`n"
    return [ordered]@{ file_count = $records.Count; tree_sha256 = Get-TextSha256 $canonical }
}

if (-not (Test-Path $pythonPath -PathType Leaf) -or (Get-Sha256 $pythonPath) -ne $expectedPythonSha256) {
    throw "Missing or changed frozen Python tree-hash tool."
}
$preregistration = [IO.File]::ReadAllText($preregistrationPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
$lock = [IO.File]::ReadAllText($lockPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
if ([string]$preregistration.gate_id -ne $freezeId -or
    [string]$preregistration.status -ne "preregistered_before_observation" -or
    [string]$preregistration.measurement_state_at_preregistration -ne "not_started" -or
    [string]$lock.freeze_id -ne $freezeId -or
    [string]$lock.measurement_state_at_freeze -ne "not_started") {
    throw "Current-tree A0 was not frozen before observation."
}

$thresholdsPath = Join-Path $repoRoot ([string]$preregistration.inherited_contract.thresholds_path)
$thresholds = [IO.File]::ReadAllText($thresholdsPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
$thresholdsSha256 = Get-Sha256 $thresholdsPath
if ([int]$thresholds.schema_version -ne 1 -or [string]$thresholds.freeze_id -ne "r0-v1" -or
    [int]$thresholds.gates.A0.metrics.ffi_fuzz_call_count_min -ne 10000 -or
    [string]$preregistration.inherited_contract.threshold_change -ne "none" -or
    [string]$preregistration.inherited_contract.operation_envelope_change -ne "none" -or
    [string]$preregistration.inherited_contract.consequence_change -ne "none" -or
    [string]$preregistration.inherited_contract.thresholds_sha256 -ne $thresholdsSha256 -or
    [int]$preregistration.inherited_contract.required_counts_per_backend.fixed_expected_valid -ne 4 -or
    [int]$preregistration.inherited_contract.required_counts_per_backend.ffi_fuzz_calls_min -ne 10000 -or
    [int]$preregistration.inherited_contract.required_counts_per_backend.adversarial_expected_valid -ne 10 -or
    [int]$preregistration.inherited_contract.required_counts_per_backend.typed_rejections -ne 6 -or
    [int]$preregistration.inherited_contract.required_counts_per_backend.guaranteed_mutation_outcomes -ne 24 -or
    [int]$preregistration.inherited_contract.required_counts_per_backend.step_fixtures -ne 3 -or
    [int]$preregistration.inherited_contract.required_backend_suite_passes -ne 2) {
    throw "Current-tree A0 inherited envelope or frozen thresholds changed."
}

$requiredCombinations = @("prior-to-prior", "current-to-current", "prior-to-current", "current-to-prior")
$actualCombinations = @($preregistration.backend_matrix.required_combinations)
$requiredSameBuild = @("prior-to-prior", "current-to-current")
$actualSameBuild = @($preregistration.backend_matrix.same_build_go_required_combinations)
if ((($requiredCombinations | Sort-Object) -join "|") -ne (($actualCombinations | Sort-Object) -join "|") -or
    (($requiredSameBuild | Sort-Object) -join "|") -ne (($actualSameBuild | Sort-Object) -join "|") -or
    [int]$preregistration.backend_matrix.full_go_required_pass_count -ne 4 -or
    [string]$preregistration.backend_matrix.negative_control_same_build -ne "Lost" -or
    [string]$preregistration.backend_matrix.negative_control_cross_build -ne "QuarantinedMigration" -or
    ((@($preregistration.backend_matrix.forbidden_negative_control_results) | Sort-Object) -join "|") -ne "Ambiguous|Resolved" -or
    [string]$preregistration.decision_rule.pf0 -ne "inactive") {
    throw "Current-tree A0 matrix, negative control, or disposition changed."
}

foreach ($anchor in $preregistration.historical_anchors.PSObject.Properties.Value) {
    $full = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$anchor.path)))
    if (-not $full.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase) -or (Get-Sha256 $full) -ne [string]$anchor.sha256) {
        throw "Historical A0 anchor changed or escaped repository: $($anchor.path)"
    }
}

$expectedPaths = @(
    "artifacts/gate-a0/$freezeId-preregistration.json",
    "artifacts/gate-a0/strengthened-a0-v2-r3-lock.json",
    "artifacts/gate-a0/runs/strengthened-v2-r3-run-001/seal.json",
    "artifacts/r0/occt-build-manifest.json",
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
    "scripts/windows/freeze-current-tree-a0-g19-04.ps1",
    "scripts/windows/validate-current-tree-a0-g19-04.ps1",
    "scripts/windows/run-strengthened-a0-v2.ps1"
)
$priorCurrentVersion = if ($freezeId -eq "current-tree-a0-g19-04-v2") { "v1" } elseif ($freezeId -eq "current-tree-a0-g19-04-v3") { "v2" } elseif ($freezeId -eq "current-tree-a0-g19-04-v4") { "v3" } elseif ($freezeId -eq "current-tree-a0-g19-04-v5") { "v4" } elseif ($freezeId -eq "current-tree-a0-g19-04-v6") { "v5" } else { $null }
if ($null -ne $priorCurrentVersion) {
    $expectedPaths += @(
        "artifacts/gate-a0/current-tree-a0-g19-04-$priorCurrentVersion-lock.json",
        "artifacts/gate-a0/runs/current-tree-a0-g19-04-$priorCurrentVersion-run-001/seal.json"
    )
}
$actualPaths = @($lock.files | ForEach-Object { [string]$_.path })
if ($actualPaths.Count -ne $expectedPaths.Count -or (($actualPaths | Sort-Object) -join "|") -ne (($expectedPaths | Sort-Object) -join "|")) {
    throw "Current-tree A0 lock input set changed."
}
foreach ($entry in @($lock.files)) {
    $full = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$entry.path)))
    if (-not $full.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        (Get-Sha256 $full) -ne [string]$entry.sha256 -or (Get-Item $full).Length -ne [int64]$entry.size_bytes) {
        throw "Current-tree A0 locked input changed or escaped repository: $($entry.path)"
    }
}

$source = Get-SourceRegistry
if ([string]$source.tree_sha256 -ne [string]$preregistration.current_tree.tree_sha256 -or
    [string]$source.tree_sha256 -ne [string]$lock.current_tree_sha256 -or
    [int]$source.file_count -ne [int]$preregistration.current_tree.file_count) {
    throw "Current-tree source changed after A0 freeze."
}
$rustcVersion = (@(& rustc -Vv) -join "`n").Trim()
if ($LASTEXITCODE -ne 0) { throw "rustc identity validation failed." }
$cargoVersion = (& cargo -V).Trim()
if ($LASTEXITCODE -ne 0) { throw "cargo identity validation failed." }
if ((Get-Sha256 (Join-Path $repoRoot "rust-toolchain.toml")) -ne [string]$preregistration.toolchain.rust_toolchain_sha256 -or
    (Get-Sha256 (Join-Path $repoRoot "Cargo.lock")) -ne [string]$preregistration.toolchain.cargo_lock_sha256 -or
    $rustcVersion -ne [string]$preregistration.toolchain.rustc_version -or
    $cargoVersion -ne [string]$preregistration.toolchain.cargo_version) {
    throw "Current-tree A0 toolchain identity changed after freeze."
}

$nativeEntry = @($lock.files | Where-Object path -eq "crates/ketchup-exact/src/native.cc")[0]
$thresholdEntry = @($lock.files | Where-Object path -eq "thresholds/r0.yaml")[0]
if ($null -eq $nativeEntry -or $null -eq $thresholdEntry -or
    [string]$lock.evaluator_source_sha256 -ne [string]$nativeEntry.sha256 -or
    [string]$lock.tolerance_profile_sha256 -ne [string]$thresholdEntry.sha256) {
    throw "Current-tree A0 evaluator or tolerance identity is not derived from locked source."
}

$backends = @($lock.backends)
if ($backends.Count -ne 2 -or [string]$backends[0].alias -ne "prior" -or [string]$backends[1].alias -ne "current") {
    throw "Current-tree A0 lock must retain the exact prior/current backend pair."
}
$backendResults = @()
foreach ($backend in $backends) {
    $root = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$backend.install_path)))
    if (-not $root.StartsWith((Join-Path $repoRoot "third_party").TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "A0 backend escapes third_party: $($backend.install_path)"
    }
    $tree = Get-TreeFingerprint $root
    if ([int]$tree.file_count -ne [int]$backend.file_count -or [string]$tree.sha256 -ne [string]$backend.tree_sha256) {
        throw "Current-tree A0 backend changed: $($backend.id)"
    }
    foreach ($library in @($backend.representative_libraries)) {
        if ((Get-Sha256 (Join-Path $root ([string]$library.path))) -ne [string]$library.sha256) {
            throw "Current-tree A0 backend library changed: $($backend.id)/$($library.path)"
        }
    }
    $backendResults += [ordered]@{
        alias = [string]$backend.alias
        id = [string]$backend.id
        install_path = $root
        tree_sha256 = [string]$backend.tree_sha256
        fingerprint = [string]$backend.fingerprint
        tkernel_sha256 = [string](@($backend.representative_libraries | Where-Object path -eq "win64/vc14/bin/TKernel.dll")[0].sha256)
        representative_libraries = @($backend.representative_libraries)
    }
}
if ($backendResults[0].tree_sha256 -eq $backendResults[1].tree_sha256 -or
    $backendResults[0].fingerprint -eq $backendResults[1].fingerprint) {
    throw "Current-tree A0 requires two distinct real backend builds."
}

$result = [ordered]@{
    freeze_id = $freezeId
    lock_sha256 = Get-Sha256 $lockPath
    current_tree_sha256 = [string]$lock.current_tree_sha256
    evaluator_source_sha256 = [string]$lock.evaluator_source_sha256
    tolerance_profile_sha256 = [string]$lock.tolerance_profile_sha256
    backends = $backendResults
}
if ($EmitJson) {
    $result | ConvertTo-Json -Depth 6 -Compress
} else {
    Write-Host "PASS: validated immutable $freezeId against current source, toolchain, thresholds, and two OCCT backends."
}
