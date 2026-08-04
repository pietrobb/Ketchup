[CmdletBinding()]
param([switch]$EmitJson)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$lockPath = Join-Path $repoRoot "artifacts\gate-a0\strengthened-a0-v1-lock.json"
$preregistrationPath = Join-Path $repoRoot "artifacts\gate-a0\strengthened-a0-v1-preregistration.json"

$pythonPath = "C:\Python311\python.exe"
$expectedPythonSha256 = "5f7b89a612c9b8af1d6456cdfcd1dbe5ca630849e79aebced9bee9a6694952ec"
if (-not (Test-Path $pythonPath -PathType Leaf) -or
    (Get-FileHash $pythonPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedPythonSha256) {
    throw "Missing or changed frozen Python tree-hash tool."
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

if (-not (Test-Path $lockPath -PathType Leaf)) { throw "Missing strengthened A0 lock: $lockPath" }
$lock = Get-Content $lockPath -Raw | ConvertFrom-Json
$preregistration = Get-Content $preregistrationPath -Raw | ConvertFrom-Json
if ($lock.freeze_id -ne "strengthened-a0-v1" -or $lock.measurement_state_at_freeze -ne "not_started") {
    throw "Unexpected strengthened A0 freeze identity or measurement state."
}
if ($preregistration.gate_id -ne "strengthened-a0-v1" -or $preregistration.measurement_state_at_preregistration -ne "not_started") {
    throw "Strengthened A0 was not preregistered before observation."
}
if ($preregistration.inherited_contract.required_counts.ffi_fuzz_calls_min -ne 10000 -or
    $preregistration.inherited_contract.required_counts.guaranteed_mutation_outcomes -ne 24 -or
    $preregistration.complete_adjacency_oracle.required_pass_count -ne 24 -or
    $preregistration.real_build_migration_oracle.expected.resolved_on_consumer_build -ne 3 -or
    $preregistration.real_build_migration_oracle.expected.quarantined_removed_reference -ne 1) {
    throw "Strengthened A0 preregistered counts or oracles changed."
}

$requiredFiles = @(
    "artifacts/gate-a0/HISTORICAL_AUDIT_ADDENDUM.md",
    "artifacts/gate-a0/strengthened-a0-v1-preregistration.json",
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
    "crates/ketchup-exact/src/bin/ketchup-a0-reference-producer.rs",
    "crates/ketchup-exact/tests/gate_a0.rs",
    "scripts/windows/validate-strengthened-a0-v1.ps1",
    "scripts/windows/run-strengthened-a0-v1.ps1",
    "artifacts/r0/occt-build-manifest.json"
)
$lockedPaths = @($lock.files | ForEach-Object { [string]$_.path })
if ($lockedPaths.Count -ne $requiredFiles.Count -or
    (($lockedPaths | Sort-Object) -join "|") -ne (($requiredFiles | Sort-Object) -join "|")) {
    throw "Strengthened A0 lock does not contain the exact preregistered input set."
}

foreach ($entry in @($lock.files)) {
    $fullPath = [IO.Path]::GetFullPath((Join-Path $repoRoot ([string]$entry.path)))
    if (-not $fullPath.StartsWith($repoRoot.TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "Locked input escapes the repository: $($entry.path)"
    }
    if (-not (Test-Path $fullPath -PathType Leaf)) { throw "Missing locked input: $($entry.path)" }
    $actual = (Get-FileHash $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne [string]$entry.sha256) { throw "Strengthened A0 locked hash mismatch: $($entry.path)" }
}

$backends = @($lock.backends)
if ($backends.Count -ne 2 -or
    [string]$backends[0].id -ne "prior-real-build" -or
    [string]$backends[0].install_path -ne "third_party/occt-install" -or
    [int]$backends[0].file_count -ne 7400 -or
    [string]$backends[1].id -ne "current-r0-v1-build" -or
    [string]$backends[1].install_path -ne "third_party/occt-install-r0-v1" -or
    [int]$backends[1].file_count -ne 7400) {
    throw "Strengthened A0 lock does not name the exact preregistered producer and consumer builds."
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
    $libraryPaths = @($libraries | ForEach-Object { [string]$_.path })
    if ($libraryPaths.Count -ne $requiredLibraries.Count -or
        (($libraryPaths | Sort-Object) -join "|") -ne (($requiredLibraries | Sort-Object) -join "|")) {
        throw "Backend representative-library set changed: $($backend.id)"
    }
    foreach ($library in $libraries) {
        $libraryPath = Join-Path $root ([string]$library.path)
        $actual = (Get-FileHash $libraryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne [string]$library.sha256) { throw "Backend library mismatch: $($backend.id)/$($library.path)" }
    }
    $backendResults += [ordered]@{
        id = [string]$backend.id
        install_path = $root
        tree_sha256 = [string]$backend.tree_sha256
        fingerprint = [string]$backend.fingerprint
        tkernel_sha256 = [string](@($libraries | Where-Object { $_.path -eq "win64/vc14/bin/TKernel.dll" })[0].sha256)
        representative_libraries = $libraries
    }
}
if ($backendResults.Count -ne 2 -or
    $backendResults[0].tree_sha256 -eq $backendResults[1].tree_sha256 -or
    $backendResults[0].tkernel_sha256 -eq $backendResults[1].tkernel_sha256 -or
    $backendResults[0].fingerprint -eq $backendResults[1].fingerprint) {
    throw "Strengthened A0 requires exactly two distinct real backend builds."
}

$lockSha256 = (Get-FileHash $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
$result = [ordered]@{
    freeze_id = "strengthened-a0-v1"
    lock_sha256 = $lockSha256
    producer = $backendResults[0]
    consumer = $backendResults[1]
}
if ($EmitJson) {
    $result | ConvertTo-Json -Depth 4 -Compress
} else {
    Write-Output "Strengthened A0 v1 preflight passed for lock $lockSha256 and two distinct frozen OCCT builds."
}
