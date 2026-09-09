[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$runner = Join-Path $PSScriptRoot "invoke-pr-ci-group.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-pr-ci-red-" + [Guid]::NewGuid().ToString("N"))
$fakeCargo = Join-Path $tempRoot "cargo.cmd"

New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
try {
    [IO.File]::WriteAllText(
        $fakeCargo,
        "@echo off`r`nexit /b 17`r`n",
        [Text.ASCIIEncoding]::new()
    )

    foreach ($group in @("quality-build", "foundations", "scheduler", "app")) {
        $output = @()
        $exitCode = 0
        try {
            $output = @(& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $runner -Group $group -CargoCommand $fakeCargo 2>&1)
            $exitCode = $LASTEXITCODE
        } catch {
            $output += $_.Exception.Message
            $exitCode = 1
        }
        if ($exitCode -eq 0) {
            throw "Expected PR CI group '$group' to propagate a failing Cargo step."
        }
        if (($output -join "`n") -notmatch "failed with exit code 17") {
            throw "PR CI group '$group' failed for the wrong reason: $($output -join ' ')"
        }
        Write-Output "RED confirmed: $group propagates Cargo failure"
    }

    Write-Output "All PR CI groups propagated their deliberate-red Cargo failure."
} finally {
    if (Test-Path $tempRoot) { Remove-Item $tempRoot -Recurse -Force }
}
