[CmdletBinding()]
param([string]$RunId = "strengthened-run-001")

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$artifactRoot = Join-Path $repoRoot "artifacts\gate-a0"
$runDir = Join-Path $artifactRoot ("runs\" + $RunId)
$validator = Join-Path $PSScriptRoot "validate-strengthened-a0-v1.ps1"

function Write-FailedDisposition([string]$FailureClass, [string]$Detail, [string]$LockSha256) {
    if (Test-Path $runDir) { throw "Strengthened A0 run evidence already exists: $RunId" }
    New-Item -ItemType Directory -Path $runDir | Out-Null
    $consequence = if ($FailureClass -eq "hash_or_provenance_only") {
        "No geometry conclusion. Repair provenance and issue a new preregistration version before another formal observation."
    } else {
        "Halt M1/M2/M3 until an explicit planar fallback or backend/reference redesign disposition is approved."
    }
    $metrics = [ordered]@{
        schema_version = 2
        freeze_id = "strengthened-a0-v1"
        lock_sha256 = $LockSha256
        decision = "NO-GO"
        failure_class = $FailureClass
        evidence_scope = if ($FailureClass -eq "hash_or_provenance_only") { "none; geometry was not validly observed" } else { "strengthened A0 execution reached native geometry" }
        detail = $Detail
        applied_consequence = $consequence
    }
    $json = $metrics | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $runDir "metrics.json"), $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    $report = "# Strengthened Gate A0 v1 Report`n`n- Run: ``$RunId```n- Freeze: ``strengthened-a0-v1```n- Lock SHA-256: ``$LockSha256```n- Failure class: ``$FailureClass```n- **Decision: NO-GO**`n`n## Detail`n`n$Detail`n`n## Applied consequence`n`n$consequence`n"
    [IO.File]::WriteAllText((Join-Path $runDir "report.md"), $report, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $artifactRoot "strengthened-metrics.json"), $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $artifactRoot "strengthened-report.md"), $report, [Text.UTF8Encoding]::new($false))
}

function Assert-RuntimeLibraries([string]$RuntimeRoot, [object]$Backend) {
    foreach ($library in @($Backend.representative_libraries)) {
        $name = Split-Path ([string]$library.path) -Leaf
        $runtimePath = Join-Path $RuntimeRoot $name
        if (-not (Test-Path $runtimePath -PathType Leaf)) { throw "Missing staged runtime library: $runtimePath" }
        $actual = (Get-FileHash $runtimePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne [string]$library.sha256) { throw "Staged runtime library mismatch: $runtimePath" }
    }
}

if ($RunId -notmatch '^strengthened-run-[0-9]{3}$') { throw "RunId must match strengthened-run-NNN." }
if (Test-Path $runDir) { throw "Strengthened A0 run evidence already exists: $RunId" }

$lockSha256 = "unavailable"
try {
    $preflightRaw = & $validator -EmitJson
    $preflight = $preflightRaw | ConvertFrom-Json
    $lockSha256 = [string]$preflight.lock_sha256
} catch {
    Write-FailedDisposition "hash_or_provenance_only" $_.Exception.Message $lockSha256
    throw
}

$workRoot = Join-Path $env:TEMP ("ketchup-strengthened-a0-" + $lockSha256.Substring(0, 16))
$producerTarget = Join-Path $workRoot "producer-target"
$consumerTarget = Join-Path $workRoot "consumer-target"
$fixturePath = Join-Path $workRoot ("prior-references-" + $RunId + ".tsv")
New-Item -ItemType Directory -Force -Path $workRoot | Out-Null
if (Test-Path $fixturePath) {
    Write-FailedDisposition "hash_or_provenance_only" "Prior-reference fixture path already exists; formal evidence would not be exclusive." $lockSha256
    throw "Prior-reference fixture path already exists: $fixturePath"
}

$env:KETCHUP_OCCT_ROOT = [string]$preflight.producer.install_path
$env:KETCHUP_OCCT_BUILD_FINGERPRINT = [string]$preflight.producer.fingerprint
& cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") -p ketchup-exact --bin ketchup-a0-reference-producer --target-dir $producerTarget
if ($LASTEXITCODE -ne 0) {
    Write-FailedDisposition "hash_or_provenance_only" "The prior-build producer did not compile; no native geometry was observed." $lockSha256
    throw "Prior-build producer compilation failed."
}
try {
    Assert-RuntimeLibraries (Join-Path $producerTarget "debug") $preflight.producer
} catch {
    Write-FailedDisposition "hash_or_provenance_only" $_.Exception.Message $lockSha256
    throw
}
$producerExe = Join-Path $producerTarget "debug\ketchup-a0-reference-producer.exe"
& $producerExe $fixturePath
if ($LASTEXITCODE -ne 0) {
    Write-FailedDisposition "substantive_topology_or_reference" "The validated prior-build producer reached native geometry but failed to emit complete Guaranteed references." $lockSha256
    throw "Prior-build producer execution failed."
}

$env:KETCHUP_OCCT_ROOT = [string]$preflight.consumer.install_path
$env:KETCHUP_OCCT_BUILD_FINGERPRINT = [string]$preflight.consumer.fingerprint
$env:KETCHUP_A0_PRIOR_REFERENCE_FIXTURE = $fixturePath
$env:KETCHUP_A0_PRODUCER_FINGERPRINT = [string]$preflight.producer.fingerprint
$env:KETCHUP_A0_RUN_ID = $RunId
$env:KETCHUP_A0_LOCK_SHA256 = $lockSha256
& cargo test --manifest-path (Join-Path $repoRoot "Cargo.toml") -p ketchup-exact --features a0-certification --test gate_a0 --no-run --target-dir $consumerTarget
if ($LASTEXITCODE -ne 0) {
    Write-FailedDisposition "hash_or_provenance_only" "The current-build consumer did not compile; no consumer geometry was observed." $lockSha256
    throw "Current-build consumer compilation failed."
}
try {
    Assert-RuntimeLibraries (Join-Path $consumerTarget "debug\deps") $preflight.consumer
} catch {
    Write-FailedDisposition "hash_or_provenance_only" $_.Exception.Message $lockSha256
    throw
}
& cargo test --manifest-path (Join-Path $repoRoot "Cargo.toml") -p ketchup-exact --features a0-certification --test gate_a0 --target-dir $consumerTarget
if ($LASTEXITCODE -ne 0) {
    if (-not (Test-Path $runDir)) {
        Write-FailedDisposition "substantive_topology_or_reference" "The current-build gate reached native geometry but terminated before writing its detailed metrics." $lockSha256
    }
    throw "Strengthened A0 returned NO-GO."
}
if (-not (Test-Path (Join-Path $runDir "metrics.json")) -or -not (Test-Path (Join-Path $runDir "report.md"))) {
    throw "Strengthened A0 passed without sealed run evidence."
}
Write-Output "Strengthened A0 v1 completed for $RunId under lock $lockSha256."
