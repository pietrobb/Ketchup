[CmdletBinding()]
param([string]$FreezeId = "current-tree-a0-g19-04-v1")

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$repoPrefix = $repoRoot.TrimEnd("\") + "\"
$artifactRoot = Join-Path $repoRoot "artifacts\gate-a0"
$freezeId = $FreezeId
if ($freezeId -notin @("current-tree-a0-g19-04-v1", "current-tree-a0-g19-04-v2", "current-tree-a0-g19-04-v3", "current-tree-a0-g19-04-v4", "current-tree-a0-g19-04-v5", "current-tree-a0-g19-04-v6")) {
    throw "FreezeId must be an approved current-tree G19-04 A0 namespace."
}
$preregistrationPath = Join-Path $artifactRoot "$freezeId-preregistration.json"
$lockPath = Join-Path $artifactRoot "$freezeId-lock.json"
$historicalLockPath = Join-Path $artifactRoot "strengthened-a0-v2-r3-lock.json"
$historicalSealPath = Join-Path $artifactRoot "runs\strengthened-v2-r3-run-001\seal.json"
$priorCurrentVersion = if ($freezeId -eq "current-tree-a0-g19-04-v2") { "v1" } elseif ($freezeId -eq "current-tree-a0-g19-04-v3") { "v2" } elseif ($freezeId -eq "current-tree-a0-g19-04-v4") { "v3" } elseif ($freezeId -eq "current-tree-a0-g19-04-v5") { "v4" } elseif ($freezeId -eq "current-tree-a0-g19-04-v6") { "v5" } else { $null }
$priorCurrentLockPath = if ($null -ne $priorCurrentVersion) { Join-Path $artifactRoot "current-tree-a0-g19-04-$priorCurrentVersion-lock.json" } else { $null }
$priorCurrentSealPath = if ($null -ne $priorCurrentVersion) { Join-Path $artifactRoot "runs\current-tree-a0-g19-04-$priorCurrentVersion-run-001\seal.json" } else { $null }
$pythonPath = "C:\Python311\python.exe"
$expectedPythonSha256 = "5f7b89a612c9b8af1d6456cdfcd1dbe5ca630849e79aebced9bee9a6694952ec"
$thresholdsPath = Join-Path $repoRoot "thresholds\r0.yaml"

function Get-Sha256([string]$Path) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "Missing freeze input: $Path" }
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

function Write-Utf8Json([string]$Path, [object]$Value) {
    [IO.File]::WriteAllText($Path, (($Value | ConvertTo-Json -Depth 20) + "`n"), [Text.UTF8Encoding]::new($false))
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
    if ($LASTEXITCODE -ne 0) { throw "git ls-files failed while freezing current-tree inputs." }
    foreach ($path in @(& git -C $repoRoot ls-files --others --exclude-standard -- Cargo.toml Cargo.lock rust-toolchain.toml crates locales scripts corpora thresholds governance)) {
        if (-not [string]::IsNullOrWhiteSpace([string]$path)) { $paths.Add(([string]$path).Replace("\", "/")) }
    }
    if ($LASTEXITCODE -ne 0) { throw "git untracked source enumeration failed." }
    $records = @($paths | Sort-Object -Unique | ForEach-Object {
        $absolute = Join-Path $repoRoot $_
        if (-not (Test-Path $absolute -PathType Leaf)) { throw "Source disappeared during freeze: $_" }
        [ordered]@{ path = $_; size_bytes = (Get-Item $absolute).Length; sha256 = Get-Sha256 $absolute }
    })
    $canonical = (@($records | ForEach-Object { "$($_.path)|$($_.size_bytes)|$($_.sha256)" }) -join "`n") + "`n"
    return [ordered]@{ file_count = $records.Count; tree_sha256 = Get-TextSha256 $canonical }
}

if ((Test-Path $preregistrationPath -PathType Leaf) -or (Test-Path $lockPath -PathType Leaf)) {
    throw "Current-tree A0 freeze already exists; refusing to overwrite immutable evidence."
}
if (-not (Test-Path $pythonPath -PathType Leaf) -or (Get-Sha256 $pythonPath) -ne $expectedPythonSha256) {
    throw "Missing or changed frozen Python tree-hash tool."
}

$thresholds = [IO.File]::ReadAllText($thresholdsPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
if ([int]$thresholds.schema_version -ne 1 -or [string]$thresholds.freeze_id -ne "r0-v1" -or
    [int]$thresholds.gates.A0.metrics.ffi_fuzz_call_count_min -ne 10000) {
    throw "Frozen A0 threshold contract changed."
}

$historicalLock = [IO.File]::ReadAllText($historicalLockPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
$backends = @($historicalLock.backends)
if ($backends.Count -ne 2) { throw "Historical A0 lock does not contain two backend identities." }
foreach ($backend in $backends) {
    $root = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$backend.install_path)))
    $tree = Get-TreeFingerprint $root
    if ([int]$tree.file_count -ne [int]$backend.file_count -or [string]$tree.sha256 -ne [string]$backend.tree_sha256) {
        throw "Frozen OCCT backend changed: $($backend.id)"
    }
    foreach ($library in @($backend.representative_libraries)) {
        if ((Get-Sha256 (Join-Path $root ([string]$library.path))) -ne [string]$library.sha256) {
            throw "Frozen OCCT library changed: $($backend.id)/$($library.path)"
        }
    }
}

$lockedPaths = @(
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
if ($null -ne $priorCurrentVersion) {
    $lockedPaths += @(
        "artifacts/gate-a0/current-tree-a0-g19-04-$priorCurrentVersion-lock.json",
        "artifacts/gate-a0/runs/current-tree-a0-g19-04-$priorCurrentVersion-run-001/seal.json"
    )
}
foreach ($relative in $lockedPaths) {
    $full = [IO.Path]::GetFullPath((Join-Path $repoRoot $relative))
    if (-not $full.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Freeze input escapes repository: $relative" }
    [void](Get-Sha256 $full)
}

$source = Get-SourceRegistry
$rustcVersion = (@(& rustc -Vv) -join "`n").Trim()
if ($LASTEXITCODE -ne 0) { throw "rustc identity capture failed." }
$cargoVersion = (& cargo -V).Trim()
if ($LASTEXITCODE -ne 0) { throw "cargo identity capture failed." }
$thresholdsSha256 = Get-Sha256 $thresholdsPath
$historicalAnchors = [ordered]@{
    strengthened_a0_v2_r3_lock = [ordered]@{ path = "artifacts/gate-a0/strengthened-a0-v2-r3-lock.json"; sha256 = Get-Sha256 $historicalLockPath }
    strengthened_a0_v2_r3_seal = [ordered]@{ path = "artifacts/gate-a0/runs/strengthened-v2-r3-run-001/seal.json"; sha256 = Get-Sha256 $historicalSealPath }
}
if ($null -ne $priorCurrentVersion) {
    $historicalAnchors["prior_current_tree_a0_lock"] = [ordered]@{ path = "artifacts/gate-a0/current-tree-a0-g19-04-$priorCurrentVersion-lock.json"; sha256 = Get-Sha256 $priorCurrentLockPath }
    $historicalAnchors["prior_current_tree_a0_seal"] = [ordered]@{ path = "artifacts/gate-a0/runs/current-tree-a0-g19-04-$priorCurrentVersion-run-001/seal.json"; sha256 = Get-Sha256 $priorCurrentSealPath }
}
$preregistration = [ordered]@{
    schema_version = 1
    gate_id = $freezeId
    status = "preregistered_before_observation"
    preregistered_utc = [DateTime]::UtcNow.ToString("o")
    measurement_state_at_preregistration = "not_started"
    purpose = "Current-tree G19-04 A0 observation using the unchanged frozen A0 envelope, thresholds, matrix, and dispositions."
    prior_failed_observation = if ($freezeId -eq "current-tree-a0-g19-04-v2") {
        "v1 was sealed NO-GO after both inherited suites invoked the historical r3 validator; its native producer/consumer matrix passed 4/4. v2 changes only validator/namespace injection in the owned harness."
    } elseif ($freezeId -eq "current-tree-a0-g19-04-v3") {
        "v2 was sealed provenance-only NO-GO before native observation because the parent preflight omitted FreezeId and selected v1. v3 adds only that missing preflight argument."
    } elseif ($freezeId -eq "current-tree-a0-g19-04-v4") {
        "v3 was sealed provenance-only NO-GO before native observation because metrics serialization retained one removed historical freeze constant. v4 reads the already-sealed FreezeId environment value instead."
    } elseif ($freezeId -eq "current-tree-a0-g19-04-v5") {
        "v4 completed FULL_GO. v5 changes only the top-level G19-04 integrity-stage invocation so the successful A0 evidence can be attached to a current-tree hardware manifest."
    } elseif ($freezeId -eq "current-tree-a0-g19-04-v6") {
        "v5 completed FULL_GO. v6 changes only the G19-04 orchestration needed to build, execute, validate, and seal the already-frozen QC-C-NAV-01 harness on HP-DEV-01."
    } else { $null }
    current_tree = $source
    toolchain = [ordered]@{
        rust_toolchain_path = "rust-toolchain.toml"
        rust_toolchain_sha256 = Get-Sha256 (Join-Path $repoRoot "rust-toolchain.toml")
        cargo_lock_sha256 = Get-Sha256 (Join-Path $repoRoot "Cargo.lock")
        rustc_version = $rustcVersion
        cargo_version = $cargoVersion
        python_path = $pythonPath
        python_sha256 = $expectedPythonSha256
    }
    historical_anchors = $historicalAnchors
    inherited_contract = [ordered]@{
        thresholds_path = "thresholds/r0.yaml"
        thresholds_sha256 = $thresholdsSha256
        threshold_change = "none"
        operation_envelope_change = "none"
        consequence_change = "none"
        measurement_build = "Release; debugger and profiler detached"
        required_counts_per_backend = [ordered]@{
            fixed_expected_valid = 4
            ffi_fuzz_calls_min = 10000
            adversarial_expected_valid = 10
            typed_rejections = 6
            guaranteed_mutation_outcomes = 24
            step_fixtures = 3
        }
        required_backend_suite_passes = 2
    }
    backend_matrix = [ordered]@{
        required_combinations = @("prior-to-prior", "current-to-current", "prior-to-current", "current-to-prior")
        full_go_required_pass_count = 4
        same_build_go_required_combinations = @("prior-to-prior", "current-to-current")
        negative_control_same_build = "Lost"
        negative_control_cross_build = "QuarantinedMigration"
        forbidden_negative_control_results = @("Resolved", "Ambiguous")
    }
    decision_rule = [ordered]@{
        full_go_effect = "Withdraw L-01/L-02, leave L-03/L-04 unadopted, and release M3."
        same_build_go_effect = "Retain L-01/L-02 without a version change, quarantine changed identities, and release M3 on the unchanged passing identity."
        no_go_effect = "Apply diagnostic hold; do not weaken thresholds, guarantees, or the operation envelope."
        pf0 = "inactive"
    }
    physical_desktop_input_required = $false
}
Write-Utf8Json $preregistrationPath $preregistration

$files = @($lockedPaths + "artifacts/gate-a0/$freezeId-preregistration.json" | Sort-Object -Unique | ForEach-Object {
    $full = Join-Path $repoRoot $_
    [ordered]@{ path = $_; sha256 = Get-Sha256 $full; size_bytes = (Get-Item $full).Length }
})
$lock = [ordered]@{
    schema_version = 1
    freeze_id = $freezeId
    frozen_utc = [DateTime]::UtcNow.ToString("o")
    measurement_state_at_freeze = "not_started"
    current_tree_sha256 = $source.tree_sha256
    evaluator_source_sha256 = Get-Sha256 (Join-Path $repoRoot "crates\ketchup-exact\src\native.cc")
    tolerance_profile_sha256 = $thresholdsSha256
    files = $files
    backends = $backends
}
Write-Utf8Json $lockPath $lock
Write-Host "PASS: froze $freezeId before observation at $lockPath"
