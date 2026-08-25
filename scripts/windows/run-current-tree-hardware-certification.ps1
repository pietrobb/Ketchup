[CmdletBinding()]
param(
    [string]$OutputDir,
    [switch]$ValidateOnly,
    [switch]$IncludeFormalA0,
    [switch]$IncludeNavigation
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$thresholdsPath = Join-Path $repoRoot "thresholds\r0.yaml"
$r0Validator = Join-Path $PSScriptRoot "validate-r0-transition-classifications.ps1"
$a0Validator = Join-Path $PSScriptRoot "validate-current-tree-a0-g19-04.ps1"
$a0Runner = Join-Path $PSScriptRoot "run-strengthened-a0-v2.ps1"
$a0FreezeId = "current-tree-a0-g19-04-v6"
$a0PreregistrationPath = Join-Path $repoRoot "artifacts\gate-a0\$a0FreezeId-preregistration.json"
$a0LockPath = Join-Path $repoRoot "artifacts\gate-a0\$a0FreezeId-lock.json"
$historicalLockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v13.json"
$occtManifestPath = Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json"
$occtRoot = Join-Path $repoRoot "third_party\occt-install-r0-v1"
$runtimeRoot = Join-Path $occtRoot "win64\vc14\bin"
$expectedHistoricalLockSha256 = "b1cf0c769cb46d0c678c1bc579e241356cc85663582a0df72093e2e54086cb01"
$expectedGpu = "AMD Radeon RX 6800 XT"
$runId = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssZ")
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "artifacts\m19\hardware-certification-runs\$runId"
}
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$repoPrefix = $repoRoot.TrimEnd("\") + "\"
if (-not $OutputDir.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "OutputDir must remain inside the repository."
}

function Get-Sha256([string]$Path) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "Missing evidence input: $Path" }
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

function Get-RelativePath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Evidence path escapes the repository: $full"
    }
    return $full.Substring($repoPrefix.Length).Replace("\", "/")
}

function Get-SourceRegistry {
    $paths = [Collections.Generic.List[string]]::new()
    foreach ($path in @(& git -C $repoRoot ls-files -- Cargo.toml Cargo.lock rust-toolchain.toml crates locales scripts corpora thresholds governance)) {
        if (-not [string]::IsNullOrWhiteSpace([string]$path)) { $paths.Add(([string]$path).Replace("\", "/")) }
    }
    if ($LASTEXITCODE -ne 0) { throw "git ls-files failed while capturing current-tree inputs." }
    foreach ($path in @(& git -C $repoRoot ls-files --others --exclude-standard -- Cargo.toml Cargo.lock rust-toolchain.toml crates locales scripts corpora thresholds governance)) {
        if (-not [string]::IsNullOrWhiteSpace([string]$path)) { $paths.Add(([string]$path).Replace("\", "/")) }
    }
    if ($LASTEXITCODE -ne 0) { throw "git untracked source enumeration failed." }
    $records = [Collections.Generic.List[object]]::new()
    foreach ($relative in @($paths | Sort-Object -Unique)) {
        $absolute = Join-Path $repoRoot $relative
        if (-not (Test-Path $absolute -PathType Leaf)) { throw "Source disappeared during capture: $relative" }
        $records.Add([ordered]@{
            path = $relative
            size_bytes = (Get-Item $absolute).Length
            sha256 = Get-Sha256 $absolute
        })
    }
    $canonical = (@($records | ForEach-Object { "$($_.path)|$($_.size_bytes)|$($_.sha256)" }) -join "`n") + "`n"
    return [ordered]@{
        file_count = $records.Count
        tree_sha256 = Get-TextSha256 $canonical
        files = @($records)
    }
}

function Get-MachineFingerprint {
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $os = Get-CimInstance Win32_OperatingSystem
    $gpus = @(Get-CimInstance Win32_VideoController | Where-Object Status -eq "OK" | Sort-Object Name)
    return [ordered]@{
        profile_id = "HP-DEV-01"
        cpu = ([string]$cpu.Name).Trim()
        physical_cores = [int]$cpu.NumberOfCores
        logical_processors = [int]$cpu.NumberOfLogicalProcessors
        ram_gib = [math]::Round([double]$os.TotalVisibleMemorySize / 1MB, 1)
        os = [string]$os.Caption
        os_version = [string]$os.Version
        operational_gpus = @($gpus | ForEach-Object {
            [ordered]@{ name = [string]$_.Name; driver_version = [string]$_.DriverVersion }
        })
    }
}

function Assert-Contract([object]$Thresholds, [object]$Machine) {
    if ([int]$Thresholds.schema_version -ne 1 -or [string]$Thresholds.freeze_id -ne "r0-v1") {
        throw "Unsupported R0 threshold identity."
    }
    if ((Get-Sha256 $historicalLockPath) -ne $expectedHistoricalLockSha256) {
        throw "Historical r0-v13 lock changed."
    }
    $profiles = @($Thresholds.hardware_profiles)
    if ($profiles.Count -ne 2 -or @($profiles | ForEach-Object { [string]$_.id } | Sort-Object) -join "," -ne "HP-DEV-01,HP-IGPU-01") {
        throw "R0 must retain exactly HP-DEV-01 and HP-IGPU-01."
    }
    $queries = @($Thresholds.query_classes)
    $expectedQueries = @("QC-B-READER-01", "QC-B-TRANSPORT-01", "QC-C-NAV-01", "QC-C-EDIT-01", "QC-C-PICK-01", "QC-C-LONG-01")
    $actualQueries = @($queries | ForEach-Object { [string]$_.id } | Sort-Object)
    if (($actualQueries -join ",") -ne (($expectedQueries | Sort-Object) -join ",")) {
        throw "R0 query-class coverage changed."
    }
    if ([int]$Thresholds.gates.A0.metrics.ffi_fuzz_call_count_min -ne 10000 -or
        [int]$Thresholds.gates.A1.metrics.canonical_changes_after_100_save_load_cycles_max -ne 0 -or
        [int]$Thresholds.gates.B.metrics.schedule_permutations_min -ne 10000 -or
        [double](@($queries | Where-Object id -eq "QC-C-NAV-01")[0].metrics.frame_p95_ms_max) -ne 16.7 -or
        [double](@($queries | Where-Object id -eq "QC-C-NAV-01")[0].metrics.frame_p99_ms_max) -ne 33.3 -or
        [double](@($queries | Where-Object id -eq "QC-C-NAV-01")[0].metrics.input_to_preview_p95_ms_max) -ne 50 -or
        [double](@($queries | Where-Object id -eq "QC-C-EDIT-01")[0].metrics.exact_result_p95_ms_max) -ne 100 -or
        [double](@($queries | Where-Object id -eq "QC-C-PICK-01")[0].metrics.pick_snap_p95_ms_max) -ne 50 -or
        [double](@($queries | Where-Object id -eq "QC-C-LONG-01")[0].metrics.cancel_p95_ms_max) -ne 250) {
        throw "A0/A1/B/C frozen thresholds changed."
    }
    if ($Machine.cpu -ne "AMD Ryzen 9 5900X 12-Core Processor" -or
        $Machine.physical_cores -ne 12 -or $Machine.logical_processors -ne 24 -or
        $Machine.os -ne "Microsoft Windows 10 Pro" -or $Machine.os_version -ne "10.0.19045" -or
        @($Machine.operational_gpus).Count -ne 1 -or [string]$Machine.operational_gpus[0].name -ne $expectedGpu) {
        throw "This machine does not match frozen HP-DEV-01."
    }
    foreach ($path in @($r0Validator, $a0Validator, $a0Runner, $a0PreregistrationPath, $a0LockPath, $occtManifestPath, $runtimeRoot)) {
        if (-not (Test-Path $path)) { throw "Required certification input is missing: $path" }
    }
}

function Write-Utf8Json([string]$Path, [object]$Value) {
    [IO.File]::WriteAllText($Path, (($Value | ConvertTo-Json -Depth 20) + "`n"), [Text.UTF8Encoding]::new($false))
}

function Invoke-CapturedStage(
    [string]$Id,
    [string]$Executable,
    [string[]]$Arguments,
    [hashtable]$Environment,
    [string]$LogsDir
) {
    $stdoutPath = Join-Path $LogsDir "$Id.stdout.txt"
    $stderrPath = Join-Path $LogsDir "$Id.stderr.txt"
    $started = [DateTime]::UtcNow
    $oldEnvironment = @{}
    foreach ($name in $Environment.Keys) {
        $oldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
        [Environment]::SetEnvironmentVariable($name, [string]$Environment[$name], "Process")
    }
    $lines = @()
    $exitCode = $null
    $launchError = $null
    try {
        Push-Location $repoRoot
        try {
            $priorPreference = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $lines = @(& $Executable @Arguments 2>&1)
                $exitCode = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $priorPreference
            }
        } finally {
            Pop-Location
        }
    } catch {
        $launchError = $_.Exception.Message
    } finally {
        foreach ($name in $Environment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $oldEnvironment[$name], "Process")
        }
    }
    $stdout = ([string]::Join([Environment]::NewLine, @($lines | ForEach-Object { $_.ToString() }))) + [Environment]::NewLine
    [IO.File]::WriteAllText($stdoutPath, $stdout, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($stderrPath, $(if ($null -eq $launchError) { "" } else { $launchError + [Environment]::NewLine }), [Text.UTF8Encoding]::new($false))
    $success = $null -eq $launchError -and $exitCode -eq 0
    return [ordered]@{
        id = $Id
        status = if ($success) { "PASS" } else { "FAIL" }
        command = $Executable
        arguments = @($Arguments)
        exit_code = $exitCode
        launch_error = $launchError
        started_utc = $started.ToString("o")
        completed_utc = [DateTime]::UtcNow.ToString("o")
        duration_ms = [int64]([DateTime]::UtcNow - $started).TotalMilliseconds
        stdout = "logs/$Id.stdout.txt"
        stdout_sha256 = Get-Sha256 $stdoutPath
        stderr = "logs/$Id.stderr.txt"
        stderr_sha256 = Get-Sha256 $stderrPath
    }
}

function Assert-StagePassed([object]$Stage) {
    if ($Stage.status -ne "PASS") { throw "Certification stage failed: $($Stage.id)" }
}

function Get-NearestRank([object[]]$Samples, [double]$Percentile) {
    if ($Samples.Count -eq 0) { throw "Cannot compute a percentile from zero samples." }
    $ordered = @($Samples | ForEach-Object { [double]$_ } | Sort-Object)
    $rank = [math]::Ceiling($ordered.Count * $Percentile)
    return [double]$ordered[[math]::Max(0, $rank - 1)]
}

function Assert-NavigationMetric([object]$Metric, [int]$Series, [string]$CurrentTreeSha256, [object]$Threshold) {
    $frameCounts = @($Metric.per_run_frame_sample_counts)
    $previewCounts = @($Metric.per_run_input_to_preview_sample_counts)
    $frameSamples = @($Metric.frame_ms)
    $previewSamples = @($Metric.input_to_preview_ms)
    $frameCount = [int64]($frameCounts | Measure-Object -Sum).Sum
    $previewCount = [int64]($previewCounts | Measure-Object -Sum).Sum
    $computedFrameP95 = Get-NearestRank $frameSamples 0.95
    $computedFrameP99 = Get-NearestRank $frameSamples 0.99
    $computedPreviewP95 = Get-NearestRank $previewSamples 0.95
    if ([int]$Metric.schema_version -ne 1 -or [string]$Metric.query_class -ne "QC-C-NAV-01" -or
        [string]$Metric.profile_id -ne "HP-DEV-01" -or [int]$Metric.series -ne $Series -or
        [string]$Metric.r0_lock_sha256 -ne $CurrentTreeSha256 -or [int]$Metric.runs -ne 30 -or
        [int]$Metric.warmup_seconds_per_run -ne 10 -or [int]$Metric.measurement_seconds_per_run -ne 30 -or
        [int]$Metric.occurrences -ne 10000 -or [int]$Metric.visible_tessellated_triangles -gt 500000 -or
        [int]$Metric.shared_authoritative_geometry -ne 1 -or
        [string]$Metric.selected_adapter.name -ne $expectedGpu -or
        [string]$Metric.selected_adapter.backend -ne "dx12" -or
        [string]$Metric.selected_adapter.device_type -ne "discrete-gpu" -or
        $frameCounts.Count -ne 30 -or $previewCounts.Count -ne 30 -or
        @($frameCounts | Where-Object { [int]$_ -le 0 }).Count -ne 0 -or
        @($previewCounts | Where-Object { [int]$_ -le 0 }).Count -ne 0 -or
        $frameSamples.Count -ne $frameCount -or $previewSamples.Count -ne $previewCount -or
        [math]::Abs([double]$Metric.frame_p95_ms - $computedFrameP95) -gt 0.000001 -or
        [math]::Abs([double]$Metric.frame_p99_ms - $computedFrameP99) -gt 0.000001 -or
        [math]::Abs([double]$Metric.input_to_preview_p95_ms - $computedPreviewP95) -gt 0.000001 -or
        [double]$Metric.frame_p95_ms -gt [double]$Threshold.metrics.frame_p95_ms_max -or
        [double]$Metric.frame_p99_ms -gt [double]$Threshold.metrics.frame_p99_ms_max -or
        [double]$Metric.input_to_preview_p95_ms -gt [double]$Threshold.metrics.input_to_preview_p95_ms_max) {
        throw "Gate C navigation series $Series failed frozen schema, cardinality, adapter, percentile, or threshold verification."
    }
}

$thresholds = [IO.File]::ReadAllText($thresholdsPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
$machine = Get-MachineFingerprint
Assert-Contract $thresholds $machine
$sourceBefore = Get-SourceRegistry
if ($ValidateOnly) {
    Write-Host "PASS: G19-04 current-tree contract is complete, frozen thresholds resolve, and this host matches HP-DEV-01."
    exit 0
}
if (Test-Path $OutputDir) { throw "OutputDir already exists; refusing to overwrite evidence: $OutputDir" }

$parentDir = Split-Path $OutputDir -Parent
[void](New-Item $parentDir -ItemType Directory -Force)
$staging = "$OutputDir.staging-$([Guid]::NewGuid().ToString('N'))"
$workRoot = Join-Path $env:TEMP "ketchup-g19-04-$runId-$([Guid]::NewGuid().ToString('N'))"
[void](New-Item $staging -ItemType Directory)
[void](New-Item $workRoot -ItemType Directory)
$completed = $false
try {
    $logsDir = Join-Path $staging "logs"
    $metricsDir = Join-Path $staging "metrics"
    [void](New-Item $logsDir -ItemType Directory)
    [void](New-Item $metricsDir -ItemType Directory)
    $cargo = (Get-Command cargo -CommandType Application -ErrorAction Stop).Source
    $powershell = (Get-Command powershell.exe -CommandType Application -ErrorAction Stop).Source
    $targetDir = Join-Path $workRoot "target"
    $commonEnvironment = @{
        CARGO_TARGET_DIR = $targetDir
        KETCHUP_OCCT_ROOT = $occtRoot
        PATH = $runtimeRoot + ";" + $env:PATH
    }
    $stages = [Collections.Generic.List[object]]::new()

    $r0 = Invoke-CapturedStage "r0-transition-governance" $powershell @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $r0Validator
    ) @{} $logsDir
    Assert-StagePassed $r0
    $stages.Add($r0)

    $a0Integrity = Invoke-CapturedStage "a0-current-tree-frozen-input-integrity" $powershell @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $a0Validator, "-FreezeId", $a0FreezeId, "-EmitJson"
    ) @{} $logsDir
    Assert-StagePassed $a0Integrity
    $stages.Add($a0Integrity)

    $formalA0 = $null
    if ($IncludeFormalA0) {
        $existingRunIds = @(Get-ChildItem (Join-Path $repoRoot "artifacts\gate-a0\runs") -Directory -ErrorAction SilentlyContinue |
            Where-Object Name -match ('^' + [regex]::Escape($a0FreezeId) + '-run-[0-9]{3}$') | ForEach-Object Name)
        $nextNumber = 1
        while ($existingRunIds -contains ("$a0FreezeId-run-{0:D3}" -f $nextNumber)) { $nextNumber++ }
        if ($nextNumber -gt 999) { throw "No fresh current-tree A0 run namespace remains." }
        $formalRunId = "$a0FreezeId-run-{0:D3}" -f $nextNumber
        $formalA0 = Invoke-CapturedStage "a0-formal-current-exact-inputs" $powershell @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $a0Runner,
            "-RunId", $formalRunId, "-FreezeId", $a0FreezeId, "-ValidatorPath", $a0Validator,
            "-PreregistrationPath", $a0PreregistrationPath, "-LockPath", $a0LockPath
        ) @{} $logsDir
        Assert-StagePassed $formalA0
        $stages.Add($formalA0)
    }

    $a1 = Invoke-CapturedStage "a1-release" $cargo @(
        "test", "--locked", "--release", "-p", "ketchup-core", "--test", "gate_a1", "--", "--test-threads=1"
    ) $commonEnvironment $logsDir
    Assert-StagePassed $a1
    $stages.Add($a1)

    $gateBMetrics = Join-Path $metricsDir "gate-b.json"
    $gateBEnvironment = @{}
    foreach ($name in $commonEnvironment.Keys) { $gateBEnvironment[$name] = $commonEnvironment[$name] }
    $gateBEnvironment["KETCHUP_GATE_B_METRICS_PATH"] = $gateBMetrics
    $gateB = Invoke-CapturedStage "b-release" $cargo @(
        "test", "--locked", "--release", "-p", "ketchup-scheduler", "--test", "gate_b", "formal_gate_b", "--", "--exact", "--test-threads=1"
    ) $gateBEnvironment $logsDir
    Assert-StagePassed $gateB
    if (-not (Test-Path $gateBMetrics -PathType Leaf)) { throw "Gate B passed without fresh metrics." }
    $stages.Add($gateB)

    $buildC = Invoke-CapturedStage "c-core-release-build" $cargo @(
        "build", "--locked", "--release", "-p", "ketchup-scheduler", "--bin", "ketchup-gate-c-core", "--bin", "ketchup-exact-worker"
    ) $commonEnvironment $logsDir
    Assert-StagePassed $buildC
    $stages.Add($buildC)
    $releaseDir = Join-Path $targetDir "release"
    $coreExe = Join-Path $releaseDir "ketchup-gate-c-core.exe"
    $workerExe = Join-Path $releaseDir "ketchup-exact-worker.exe"
    foreach ($path in @($coreExe, $workerExe)) {
        if (-not (Test-Path $path -PathType Leaf)) { throw "Gate C build omitted $path" }
    }

    $currentLockSha256 = $sourceBefore.tree_sha256
    for ($series = 1; $series -le 3; $series++) {
        $metricPath = Join-Path $metricsDir "gate-c-core-series-$series.json"
        $stage = Invoke-CapturedStage "c-core-series-$series" $coreExe @(
            "HP-DEV-01", "$series", $currentLockSha256, $workerExe, $metricPath
        ) $commonEnvironment $logsDir
        Assert-StagePassed $stage
        if (-not (Test-Path $metricPath -PathType Leaf)) { throw "Gate C core series $series omitted metrics." }
        $metric = [IO.File]::ReadAllText($metricPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
        if ($metric.profile_id -ne "HP-DEV-01" -or [int]$metric.series -ne $series -or
            $metric.r0_lock_sha256 -ne $currentLockSha256 -or [double]$metric.edit_p95_ms -gt 100 -or
            [double]$metric.action_digest_match_percent -ne 100 -or [double]$metric.pick_snap_p95_ms -gt 50 -or
            [int]$metric.wrong_identity_count -ne 0 -or [double]$metric.navigation_block_max_ms -gt 100 -or
            [double]$metric.cancel_p95_ms -gt 250 -or [int]$metric.committed_data_loss_count -ne 0) {
            throw "Gate C core series $series failed frozen threshold verification."
        }
        $stages.Add($stage)
    }

    $navExe = $null
    if ($IncludeNavigation) {
        $buildNav = Invoke-CapturedStage "c-navigation-release-build" $cargo @(
            "build", "--locked", "--release", "-p", "ketchup-app", "--bin", "ketchup-gate-c-nav"
        ) $commonEnvironment $logsDir
        Assert-StagePassed $buildNav
        $stages.Add($buildNav)
        $navExe = Join-Path $releaseDir "ketchup-gate-c-nav.exe"
        if (-not (Test-Path $navExe -PathType Leaf)) { throw "Gate C navigation build omitted $navExe" }
        $navEnvironment = @{}
        foreach ($name in $commonEnvironment.Keys) { $navEnvironment[$name] = $commonEnvironment[$name] }
        $navEnvironment["WGPU_BACKEND"] = "dx12"
        $navThreshold = @($thresholds.query_classes | Where-Object id -eq "QC-C-NAV-01")[0]
        for ($series = 1; $series -le 3; $series++) {
            $metricPath = Join-Path $metricsDir "gate-c-navigation-series-$series.json"
            $stage = Invoke-CapturedStage "c-navigation-series-$series" $navExe @(
                "HP-DEV-01", "$series", $currentLockSha256, $expectedGpu, $metricPath
            ) $navEnvironment $logsDir
            Assert-StagePassed $stage
            if (-not (Test-Path $metricPath -PathType Leaf)) { throw "Gate C navigation series $series omitted metrics." }
            $metric = [IO.File]::ReadAllText($metricPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
            Assert-NavigationMetric $metric $series $currentLockSha256 $navThreshold
            $stages.Add($stage)
        }
    }

    $sourceAfter = Get-SourceRegistry
    if ($sourceAfter.tree_sha256 -ne $sourceBefore.tree_sha256) {
        throw "Current-tree source changed during hardware certification."
    }
    $head = (& git -C $repoRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $head -notmatch '^[0-9a-f]{40}$') { throw "Could not bind evidence to git HEAD." }
    $metricRecords = @(Get-ChildItem $metricsDir -File | Sort-Object Name | ForEach-Object {
        [ordered]@{ path = "metrics/$($_.Name)"; sha256 = Get-Sha256 $_.FullName; size_bytes = $_.Length }
    })
    $remaining = [Collections.Generic.List[string]]::new()
    if (-not $IncludeFormalA0) {
        $remaining.Add("fresh-formal-A0-observation")
    }
    if (-not $IncludeNavigation) { $remaining.Add("HP-DEV-01-three-series-navigation") }
    $remaining.Add("HP-IGPU-01-three-series-core-and-navigation")
    $completedScope = [Collections.Generic.List[string]]::new()
    foreach ($scope in @("R0-governance", "A0-current-tree-freeze", "A1-release", "B-release", "C-core-HP-DEV-01-three-series")) { $completedScope.Add($scope) }
    if ($IncludeFormalA0) { $completedScope.Add("A0-current-tree-formal-observation") }
    if ($IncludeNavigation) { $completedScope.Add("C-navigation-HP-DEV-01-three-series") }
    $blockedScope = [Collections.Generic.List[string]]::new()
    if (-not $IncludeFormalA0) { $blockedScope.Add("A0-formal-observation") }
    if (-not $IncludeNavigation) { $blockedScope.Add("C-navigation-HP-DEV-01") }
    $blockedScope.Add("HP-IGPU-01")
    $manifest = [ordered]@{
        schema_version = 1
        kind = "g19-04-current-tree-hardware-certification"
        status = "PARTIAL_PASS"
        release_eligible = $false
        captured_utc = [DateTime]::UtcNow.ToString("o")
        git_head = $head
        current_tree_sha256 = $sourceBefore.tree_sha256
        current_tree = $sourceBefore
        thresholds = [ordered]@{ path = "thresholds/r0.yaml"; sha256 = Get-Sha256 $thresholdsPath; freeze_id = [string]$thresholds.freeze_id }
        historical_r0_v13_lock = [ordered]@{ path = "artifacts/r0/preregistration-lock-r0-v13.json"; sha256 = Get-Sha256 $historicalLockPath }
        current_tree_a0_freeze = [ordered]@{
            freeze_id = $a0FreezeId
            preregistration_path = "artifacts/gate-a0/$a0FreezeId-preregistration.json"
            preregistration_sha256 = Get-Sha256 $a0PreregistrationPath
            lock_path = "artifacts/gate-a0/$a0FreezeId-lock.json"
            lock_sha256 = Get-Sha256 $a0LockPath
            formal_run_id = if ($null -ne $formalA0) { $formalRunId } else { $null }
        }
        runner = [ordered]@{ path = "scripts/windows/run-current-tree-hardware-certification.ps1"; sha256 = Get-Sha256 $PSCommandPath }
        executables = [ordered]@{
            gate_c_core_sha256 = Get-Sha256 $coreExe
            exact_worker_sha256 = Get-Sha256 $workerExe
            gate_c_navigation_sha256 = if ($null -ne $navExe) { Get-Sha256 $navExe } else { $null }
        }
        machine = $machine
        physical_desktop_input_used = $false
        completed_scope = @($completedScope)
        blocked_scope = @($blockedScope)
        stages = @($stages)
        metrics = $metricRecords
        remaining_required_scope = @($remaining)
    }
    Write-Utf8Json (Join-Path $staging "evidence-manifest.json") $manifest
    Move-Item $staging $OutputDir
    $completed = $true
    if ($IncludeNavigation) {
        Write-Host "PARTIAL PASS: current-tree R0/A0/A1/B and HP-DEV-01 Gate C core/navigation passed; HP-IGPU-01 remains."
    } else {
        Write-Host "PARTIAL PASS: current-tree R0/A0/A1/B and HP-DEV-01 Gate C core passed; navigation and HP-IGPU-01 remain."
    }
    Write-Host "Evidence: $(Join-Path $OutputDir 'evidence-manifest.json')"
} finally {
    if (Test-Path $workRoot) { Remove-Item $workRoot -Recurse -Force }
    if (-not $completed -and (Test-Path $staging)) { Remove-Item $staging -Recurse -Force }
}
