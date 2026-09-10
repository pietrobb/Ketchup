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
$resolvedCargo = Get-Command $CargoCommand -CommandType Application -ErrorAction Stop
$cargoPath = [IO.Path]::GetFullPath([string]$resolvedCargo.Source)
$repoPrefix = $repoRoot.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if ([IO.Path]::GetExtension($cargoPath) -cne ".exe" -or
    $cargoPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Cargo must resolve to a native executable outside the untrusted checkout."
}
$cargoGuard = [IO.File]::Open($cargoPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
if ($cargoGuard.Length -le 0 -or $cargoGuard.Length -gt (64 * 1024 * 1024)) {
    $cargoGuard.Dispose()
    throw "Cargo executable is outside its bounded size envelope."
}
$sha256 = [Security.Cryptography.SHA256]::Create()
try {
    $cargoSha256 = -join ($sha256.ComputeHash($cargoGuard) | ForEach-Object { $_.ToString("x2") })
    $cargoGuard.Position = 0
} finally {
    $sha256.Dispose()
}
$CargoCommand = $cargoPath
Write-Output "Using guarded Cargo executable $cargoPath (SHA-256 $cargoSha256)."

function Invoke-CargoStep([string]$Name, [string[]]$Arguments) {
    Write-Output "==> $Name"
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        & $CargoCommand @Arguments
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) { throw "$Name failed with exit code $exitCode." }
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
            Invoke-CargoStep "scheduler and exact-worker product tests" @("test", "--locked", "-p", "ketchup-scheduler", "--all-targets", "--all-features", "--no-fail-fast")
        }
        "app" {
            Invoke-CargoStep "headless app product tests" @("test", "--locked", "-p", "ketchup-app", "--all-targets", "--all-features", "--no-fail-fast")
        }
    }
} finally {
    Pop-Location
    $cargoGuard.Dispose()
}
