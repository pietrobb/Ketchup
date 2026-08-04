[CmdletBinding()]
param(
    [string]$BaseRef,
    [switch]$ArchitectureOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))

function Invoke-NativeStep([string]$Name, [scriptblock]$Command) {
    Write-Output "==> $Name"
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE." }
}

Push-Location $repoRoot
try {
    $guardArguments = @{}
    if (-not [string]::IsNullOrWhiteSpace($BaseRef)) { $guardArguments.BaseRef = $BaseRef }
    & (Join-Path $PSScriptRoot "test-architecture-guards.ps1") @guardArguments
    & (Join-Path $PSScriptRoot "test-ci-guard-red-paths.ps1")
    & (Join-Path $PSScriptRoot "validate-r0-transition-classifications.ps1")

    if ($ArchitectureOnly) {
        Write-Output "CI architecture-only governance block passed."
        exit 0
    }

    $a0Disposition = Get-Content (Join-Path $repoRoot "artifacts\gate-a0\strengthened-metrics.json") -Raw | ConvertFrom-Json
    if ($a0Disposition.freeze_id -ne "strengthened-a0-v1" -or
        $a0Disposition.lock_sha256 -ne "5ae34bdd0eb7cad4719c11154e57e5ec8d955d51313e7ffb14ff5f96809a7ff0" -or
        $a0Disposition.decision -ne "NO-GO" -or
        $a0Disposition.failure_class -ne "substantive_topology_or_reference" -or
        -not ([string]$a0Disposition.applied_consequence).StartsWith("Halt M1/M2/M3")) {
        throw "Strengthened A0 disposition or required halt changed."
    }

    Invoke-NativeStep "rustfmt" { & cargo fmt --all --check }
    Invoke-NativeStep "clippy" { & cargo clippy --locked --workspace --all-targets -- -D warnings }
    Invoke-NativeStep "portable workspace tests with sealed A0 NO-GO assertions excluded" {
        & cargo test --locked --workspace --all-targets -- --skip gate_a0 --skip fixed_extrusion_is_valid_and_carries_guaranteed_history
    }
    Invoke-NativeStep "product build" { & cargo build --locked -p ketchup-app --bin ketchup-app }
    Invoke-NativeStep "dependency advisories, licenses, and sources" {
        & cargo deny --config (Join-Path $repoRoot "governance\deny-ci.toml") check advisories licenses sources
    }

    Write-Output "==> strengthened A0 frozen-input and backend validation"
    & (Join-Path $PSScriptRoot "validate-strengthened-a0-v1.ps1")

    Write-Output "==> historical R0 v13/Gate C evidence validation"
    & (Join-Path $PSScriptRoot "validate-r0-v13-historical-evidence.ps1")

    Write-Output "CI governance block passed: quality, workspace tests, product build, architecture guards, frozen gate inputs, and dependency policy are green."
} finally {
    Pop-Location
}
