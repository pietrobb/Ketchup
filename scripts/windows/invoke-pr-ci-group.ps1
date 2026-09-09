[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("quality-build", "foundations", "scheduler", "app")]
    [string]$Group,
    [string]$CargoCommand = "cargo"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))

function Invoke-CargoStep([string]$Name, [string[]]$Arguments) {
    Write-Output "==> $Name"
    & $CargoCommand @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE." }
}

Push-Location $repoRoot
try {
    switch ($Group) {
        "quality-build" {
            Invoke-CargoStep "rustfmt" @("fmt", "--all", "--", "--check")
            Invoke-CargoStep "strict clippy" @("clippy", "--locked", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings")
            Invoke-CargoStep "complete workspace build" @("build", "--locked", "--workspace", "--all-targets")
        }
        "foundations" {
            Invoke-CargoStep "foundation product tests" @("test", "--locked", "--workspace", "--exclude", "ketchup-app", "--exclude", "ketchup-scheduler", "--all-targets", "--no-fail-fast")
        }
        "scheduler" {
            Invoke-CargoStep "scheduler and exact-worker product tests" @("test", "--locked", "-p", "ketchup-scheduler", "--all-targets", "--no-fail-fast")
        }
        "app" {
            Invoke-CargoStep "headless app product tests" @("test", "--locked", "-p", "ketchup-app", "--all-targets", "--no-fail-fast")
        }
    }
} finally {
    Pop-Location
}
