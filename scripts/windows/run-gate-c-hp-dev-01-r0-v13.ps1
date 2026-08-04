[CmdletBinding()]
param(
    [ValidateRange(2, 3)]
    [int]$Attempt = 2
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$artifactDir = Join-Path $repoRoot "artifacts\gate-c"
$attemptSuffix = "attempt-$Attempt"
$targetDir = Join-Path $repoRoot "target\gate-c-r0-v13-hp-dev-01-$attemptSuffix"
$lockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v13.json"
$runnerPath = Join-Path $repoRoot "scripts\windows\run-gate-c-hp-igpu-01.ps1"
$validatorPath = Join-Path $repoRoot "scripts\windows\validate-r0-v13-preregistration.ps1"
$statePath = Join-Path $artifactDir "hp-dev-01-r0-v13-$attemptSuffix-state.json"
$coreProvenancePath = Join-Path $artifactDir "hp-dev-01-portable-core-r0-v13-provenance.json"
$navProvenancePath = Join-Path $artifactDir "hp-dev-01-portable-nav-r0-v13-provenance.json"
$buildStdoutPath = Join-Path $artifactDir "hp-dev-01-r0-v13-$attemptSuffix-build.stdout.log"
$buildStderrPath = Join-Path $artifactDir "hp-dev-01-r0-v13-$attemptSuffix-build.stderr.log"
$lockSha256 = "b1cf0c769cb46d0c678c1bc579e241356cc85663582a0df72093e2e54086cb01"
$buildInputTreeSha256 = "de8592b10b5ed88d2ae7cf8394c127d3d7ca1ea8b22830911cc28a8fbdca84bb"
$expectedAdapterName = "AMD Radeon RX 6800 XT"

$coreMetrics = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-dev-01-core-r0-v13-$attemptSuffix-series-$_.json" })
$navMetrics = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-dev-01-nav-r0-v13-$attemptSuffix-series-$_.json" })
$coreStdout = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-dev-01-core-r0-v13-$attemptSuffix-series-$_.stdout.log" })
$coreStderr = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-dev-01-core-r0-v13-$attemptSuffix-series-$_.stderr.log" })
$navStdout = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-dev-01-nav-r0-v13-$attemptSuffix-series-$_.stdout.log" })
$navStderr = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-dev-01-nav-r0-v13-$attemptSuffix-series-$_.stderr.log" })

function Get-LowerSha256([string]$Path) {
    return (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-Utf8Json([string]$Path, [object]$Value) {
    $json = ($Value | ConvertTo-Json -Depth 12) + [Environment]::NewLine
    [IO.File]::WriteAllText($Path, $json, [Text.UTF8Encoding]::new($false))
}

function Write-Utf8JsonExclusive([string]$Path, [object]$Value) {
    $json = (($Value | ConvertTo-Json -Depth 12) + [Environment]::NewLine)
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json)
    $stream = [IO.FileStream]::new($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
}

function Get-RelativePath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $prefix = $repoRoot.TrimEnd("\") + "\"
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Evidence path escapes the repository: $full"
    }
    return $full.Substring($prefix.Length).Replace("\", "/")
}

function Get-ArtifactRecord([string]$Path) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "Missing evidence artifact: $Path" }
    return [ordered]@{ path = Get-RelativePath $Path; sha256 = Get-LowerSha256 $Path }
}

function Set-State([string]$Status, [string]$Stage, [int]$CompletedCore, [int]$CompletedNav, [string]$Message) {
    Write-Utf8Json $statePath ([ordered]@{
        schema_version = 1
        profile_id = "HP-DEV-01"
        freeze_id = "r0-v13"
        r0_lock_sha256 = $lockSha256
        attempt = $Attempt
        status = $Status
        stage = $Stage
        completed_core_series = $CompletedCore
        completed_nav_series = $CompletedNav
        message = $Message
        updated_utc = [DateTime]::UtcNow.ToString("o")
    })
}

function Assert-CoreMetric([string]$Path, [int]$Series) {
    $metric = Get-Content $Path -Raw | ConvertFrom-Json
    if ($metric.profile_id -ne "HP-DEV-01" -or [int]$metric.series -ne $Series -or
        $metric.r0_lock_sha256 -ne $lockSha256 -or [int]$metric.occurrences -ne 10000 -or
        [double]$metric.edit_p95_ms -gt 100 -or [double]$metric.action_digest_match_percent -ne 100 -or
        [double]$metric.pick_snap_p95_ms -gt 50 -or [int]$metric.wrong_identity_count -ne 0 -or
        [double]$metric.navigation_block_max_ms -gt 100 -or [double]$metric.cancel_p95_ms -gt 250 -or
        [int]$metric.committed_data_loss_count -ne 0) {
        throw "Core series $Series failed the frozen schema or thresholds."
    }
    return $metric
}

function Assert-NavMetric([string]$Path, [int]$Series) {
    $metric = Get-Content $Path -Raw | ConvertFrom-Json
    if ($metric.profile_id -ne "HP-DEV-01" -or [int]$metric.series -ne $Series -or
        $metric.r0_lock_sha256 -ne $lockSha256 -or [int]$metric.runs -ne 30 -or
        [int]$metric.warmup_seconds_per_run -ne 10 -or [int]$metric.measurement_seconds_per_run -ne 30 -or
        [int]$metric.occurrences -ne 10000 -or [int]$metric.visible_tessellated_triangles -ne 20000 -or
        [int]$metric.shared_authoritative_geometry -ne 1 -or
        [string]$metric.selected_adapter.name -ne $expectedAdapterName -or
        [string]$metric.selected_adapter.backend -ne "dx12" -or
        [string]$metric.selected_adapter.device_type -ne "discrete-gpu" -or
        [double]$metric.frame_p95_ms -gt 16.7 -or [double]$metric.frame_p99_ms -gt 33.3 -or
        [double]$metric.input_to_preview_p95_ms -gt 50) {
        throw "Navigation series $Series failed the frozen schema, adapter binding, or thresholds."
    }
    return $metric
}

trap {
    if (Test-Path $statePath -PathType Leaf) {
        $prior = Get-Content $statePath -Raw | ConvertFrom-Json
        Set-State "FAILED" "terminal" ([int]$prior.completed_core_series) ([int]$prior.completed_nav_series) $_.Exception.Message
    }
    Write-Error $_
    exit 1
}

$allReserved = @($statePath, $coreProvenancePath, $navProvenancePath, $buildStdoutPath, $buildStderrPath) +
    $coreMetrics + $navMetrics + $coreStdout + $coreStderr + $navStdout + $navStderr
foreach ($path in $allReserved) {
    if (Test-Path $path) { throw "R0 v13 HP-DEV-01 evidence namespace is not fresh: $path" }
}
if (Test-Path $targetDir) { throw "R0 v13 HP-DEV-01 clean-build target already exists: $targetDir" }
if ((Get-LowerSha256 $lockPath) -ne $lockSha256) { throw "R0 v13 lock hash mismatch." }

& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $validatorPath
if ($LASTEXITCODE -ne 0) { throw "R0 v13 preregistration validation failed." }

$gpu = @(Get-CimInstance Win32_VideoController | Where-Object Status -eq "OK")
if ($gpu.Count -ne 1 -or [string]$gpu[0].Name -ne $expectedAdapterName) {
    throw "HP-DEV-01 requires exactly one operational $expectedAdapterName adapter."
}

$startedUtc = [DateTime]::UtcNow.ToString("o")
Set-State "RUNNING" "release-build" 0 0 "Creating the single clean R0 v13 reference build."
$rustBin = Join-Path $env:USERPROFILE ".rustup\toolchains\1.97.0-x86_64-pc-windows-msvc\bin"
$msvcBin = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.35.32215\bin\HostX64\x64"
$cargo = Join-Path $rustBin "cargo.exe"
$previousTarget = $env:CARGO_TARGET_DIR
$previousRustc = $env:RUSTC
$previousPath = $env:PATH
try {
    $env:CARGO_TARGET_DIR = $targetDir
    $env:RUSTC = Join-Path $rustBin "rustc.exe"
    $env:PATH = $rustBin + ";" + $msvcBin + ";" + $previousPath
    $build = Start-Process -FilePath $cargo -ArgumentList @(
        "build", "--locked", "--release", "--bin", "ketchup-gate-c-core",
        "--bin", "ketchup-exact-worker", "--bin", "ketchup-gate-c-nav"
    ) -WorkingDirectory $repoRoot -RedirectStandardOutput $buildStdoutPath `
        -RedirectStandardError $buildStderrPath -Wait -PassThru
    if ($build.ExitCode -ne 0) { throw "The clean R0 v13 reference build failed with exit code $($build.ExitCode)." }
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    $env:RUSTC = $previousRustc
    $env:PATH = $previousPath
}

$releaseDir = Join-Path $targetDir "release"
$coreExe = Join-Path $releaseDir "ketchup-gate-c-core.exe"
$workerExe = Join-Path $releaseDir "ketchup-exact-worker.exe"
$navExe = Join-Path $releaseDir "ketchup-gate-c-nav.exe"
foreach ($path in @($coreExe, $workerExe, $navExe)) {
    if (-not (Test-Path $path -PathType Leaf)) { throw "Clean build omitted executable: $path" }
}
$executables = [ordered]@{
    "ketchup-gate-c-core.exe" = Get-LowerSha256 $coreExe
    "ketchup-exact-worker.exe" = Get-LowerSha256 $workerExe
    "ketchup-gate-c-nav.exe" = Get-LowerSha256 $navExe
}

$coreResults = @()
for ($series = 1; $series -le 3; $series++) {
    Set-State "RUNNING" "core-series-$series" ($series - 1) 0 "Running core reference series $series."
    $coreRun = Start-Process -FilePath $coreExe -ArgumentList @(
        "HP-DEV-01", "$series", $lockSha256, $workerExe, $coreMetrics[$series - 1]
    ) -WorkingDirectory $repoRoot -RedirectStandardOutput $coreStdout[$series - 1] `
        -RedirectStandardError $coreStderr[$series - 1] -Wait -PassThru
    if ($coreRun.ExitCode -ne 0 -or -not (Test-Path $coreMetrics[$series - 1] -PathType Leaf)) {
        throw "Core reference series $series failed with exit code $($coreRun.ExitCode)."
    }
    $coreResults += Assert-CoreMetric $coreMetrics[$series - 1] $series
}
$coreFiles = @($coreMetrics + $coreStdout + $coreStderr | ForEach-Object { Get-ArtifactRecord $_ })
Write-Utf8JsonExclusive $coreProvenancePath ([ordered]@{
    schema_version = 1
    profile_id = "HP-DEV-01"
    freeze_id = "r0-v13"
    attempt = $Attempt
    contract = "portable-build-provenance-v1"
    r0_lock_sha256 = $lockSha256
    build_input_tree_sha256 = $buildInputTreeSha256
    controller_script_sha256 = Get-LowerSha256 $PSCommandPath
    runner_script_sha256 = Get-LowerSha256 $runnerPath
    validator_script_sha256 = Get-LowerSha256 $validatorPath
    clean_build = $true
    decision = "PASS"
    started_utc = $startedUtc
    completed_utc = [DateTime]::UtcNow.ToString("o")
    executable_sha256 = $executables
    metrics = @($coreResults | ForEach-Object {
        [ordered]@{
            series = [int]$_.series
            edit_p95_ms = [double]$_.edit_p95_ms
            pick_snap_p95_ms = [double]$_.pick_snap_p95_ms
            navigation_block_max_ms = [double]$_.navigation_block_max_ms
            cancel_p95_ms = [double]$_.cancel_p95_ms
            action_digest_match_percent = [double]$_.action_digest_match_percent
            wrong_identity_count = [int]$_.wrong_identity_count
            committed_data_loss_count = [int]$_.committed_data_loss_count
        }
    })
    files = $coreFiles
})

$env:WGPU_BACKEND = "dx12"
$navResults = @()
for ($series = 1; $series -le 3; $series++) {
    Set-State "RUNNING" "navigation-series-$series" 3 ($series - 1) "Running the immutable 1,200-second navigation reference series $series."
    $navRun = Start-Process -FilePath $navExe -ArgumentList @(
        "HP-DEV-01", "$series", $lockSha256, "`"$expectedAdapterName`"", $navMetrics[$series - 1]
    ) -WorkingDirectory $repoRoot -RedirectStandardOutput $navStdout[$series - 1] `
        -RedirectStandardError $navStderr[$series - 1] -Wait -PassThru
    if ($navRun.ExitCode -ne 0 -or -not (Test-Path $navMetrics[$series - 1] -PathType Leaf)) {
        throw "Navigation reference series $series failed with exit code $($navRun.ExitCode)."
    }
    $navResults += Assert-NavMetric $navMetrics[$series - 1] $series
}
$navFiles = @($navMetrics + $navStdout + $navStderr | ForEach-Object { Get-ArtifactRecord $_ })
Write-Utf8JsonExclusive $navProvenancePath ([ordered]@{
    schema_version = 1
    profile_id = "HP-DEV-01"
    freeze_id = "r0-v13"
    attempt = $Attempt
    contract = "portable-build-provenance-v1"
    r0_lock_sha256 = $lockSha256
    build_input_tree_sha256 = $buildInputTreeSha256
    controller_script_sha256 = Get-LowerSha256 $PSCommandPath
    runner_script_sha256 = Get-LowerSha256 $runnerPath
    validator_script_sha256 = Get-LowerSha256 $validatorPath
    clean_build = $true
    decision = "PASS"
    started_utc = $startedUtc
    completed_utc = [DateTime]::UtcNow.ToString("o")
    executable_sha256 = $executables["ketchup-gate-c-nav.exe"]
    expected_adapter_name = $expectedAdapterName
    backend = "dx12"
    metrics = @($navResults | ForEach-Object {
        [ordered]@{
            series = [int]$_.series
            frame_p95_ms = [double]$_.frame_p95_ms
            frame_p99_ms = [double]$_.frame_p99_ms
            input_to_preview_p95_ms = [double]$_.input_to_preview_p95_ms
            frame_sample_count = @($_.frame_ms).Count
            input_to_preview_sample_count = @($_.input_to_preview_ms).Count
            decision = "PASS"
        }
    })
    files = $navFiles
})
Set-State "PASS" "complete" 3 3 "All R0 v13 HP-DEV-01 reference series passed and provenance was sealed."
Write-Output "R0 v13 HP-DEV-01 reference controller PASS."
