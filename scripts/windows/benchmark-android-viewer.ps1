# Measure the installed Android Kečup Viewer on the single connected device or
# emulator, as scripts\windows\benchmark-viewer.ps1 does on Windows: each
# package is pushed as benchmark.ketchup-view, the Viewer is restarted and
# measures it on its window's GPU, and benchmark.txt comes back with the peak
# memory. Install the Viewer first (build-android-viewer.ps1 -Release
# -InstallWithModel ...).
#
#   scripts\windows\benchmark-android-viewer.ps1 viewer\samples\kitchen.ketchup-view
[CmdletBinding(PositionalBinding = $false)]
param(
    [int]$TimeoutSeconds = 600,
    [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
    [string[]]$Packages
)
$ErrorActionPreference = "Stop"

$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA "Android\Sdk" }
$adb = Join-Path $sdk "platform-tools\adb.exe"
$app = "sk.ketchup.viewer"
$dir = "/sdcard/Android/data/$app/files"
$model = (& $adb shell getprop ro.product.model).Trim()
$avd = (& $adb shell getprop ro.boot.qemu.avd_name).Trim()
$abi = (& $adb shell getprop ro.product.cpu.abi).Trim()
$release = (& $adb shell getprop ro.build.version.release).Trim()
$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
$device = if ($avd) { "emulator $avd ($model, $abi, Android $release) on $env:COMPUTERNAME, $cpu" } else { "$model ($abi, Android $release)" }
Write-Output "Device: $device"

foreach ($package in $Packages) {
    & $adb shell am force-stop $app
    & $adb shell rm -f "$dir/benchmark.txt"
    # adb reports push progress on stderr, which Stop would treat as a failure.
    $ErrorActionPreference = "Continue"
    & $adb push (Resolve-Path $package).Path "$dir/benchmark.ketchup-view" 2>&1 | Out-Null
    $pushed = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    if ($pushed -ne 0) { throw "adb push of $package failed" }
    & $adb shell am start -n "$app/.ViewerActivity" | Out-Null
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $report = $null
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 2
        $text = (& $adb shell "cat $dir/benchmark.txt 2>/dev/null") -join "`n"
        if ($text.Trim()) { $report = $text; break }
    }
    & $adb shell am force-stop $app
    if (-not $report) { throw "No benchmark report for $package within $TimeoutSeconds s; see adb logcat -s KetchupViewer" }
    Write-Output ""
    Write-Output "== $package"
    Write-Output $report.TrimEnd()
}
