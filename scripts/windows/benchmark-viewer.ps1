# Measure the Kečup Viewer on this PC: runs `ketchup-viewer --benchmark` on
# each package and adds the process's peak memory, which the Viewer cannot
# read about itself.
#
#   scripts\windows\benchmark-viewer.ps1 viewer\samples\kitchen.ketchup-view
#   scripts\windows\benchmark-viewer.ps1 -Viewer path\to\ketchup-viewer.exe a.ketchup-view b.ketchup-view
[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$Viewer = "",
    [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
    [string[]]$Packages
)
$ErrorActionPreference = "Stop"

if (-not $Viewer) {
    $repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo "target" }
    $Viewer = Join-Path $target "release\ketchup-viewer.exe"
}
if (-not (Test-Path $Viewer)) {
    throw "No Viewer at $Viewer; build it with: cargo build --release --locked -p ketchup-viewer"
}
$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
$ram = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
Write-Output "PC: $env:COMPUTERNAME, $cpu, $ram GB RAM"

foreach ($package in $Packages) {
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = (Resolve-Path $Viewer).Path
    $info.Arguments = "--benchmark `"$((Resolve-Path $package).Path)`""
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::Start($info)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    # The peak only grows, so its last reading before exit is the peak.
    $peak = 0
    while (-not $process.HasExited) {
        try {
            $process.Refresh()
            $peak = [math]::Max($peak, $process.PeakWorkingSet64)
        } catch [System.InvalidOperationException] {
            break
        }
        Start-Sleep -Milliseconds 20
    }
    $process.WaitForExit()
    Write-Output ""
    Write-Output "== $package"
    Write-Output $stdout.Result.TrimEnd()
    if ($process.ExitCode -ne 0) {
        Write-Output $stderr.Result.TrimEnd()
        throw "Viewer benchmark of $package failed with exit code $($process.ExitCode)"
    }
    Write-Output ("peak memory: {0:N0} MB" -f ($peak / 1MB))
}
