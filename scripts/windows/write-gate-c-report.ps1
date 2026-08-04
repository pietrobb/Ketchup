[CmdletBinding()]
param(
    [switch]$WriteReport,
    [string]$EvidenceDirectory = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$referenceDir = Join-Path $repoRoot "artifacts\gate-c"
$artifactDir = if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $referenceDir
} else {
    [IO.Path]::GetFullPath($EvidenceDirectory)
}
$lockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v12.json"
$runnerPath = Join-Path $repoRoot "scripts\windows\run-gate-c-hp-igpu-01.ps1"
$validatorPath = Join-Path $repoRoot "scripts\windows\validate-r0-v12-preregistration.ps1"
$fingerprintPath = Join-Path $artifactDir "hp-igpu-01-fingerprint-r0-v12.json"
$attemptClaimPath = Join-Path $artifactDir "hp-igpu-01-r0-v12-attempt-claim.json"
$runManifestPath = Join-Path $artifactDir "hp-igpu-01-r0-v12-run-manifest.json"
$buildStdoutPath = Join-Path $artifactDir "hp-igpu-01-r0-v12-build.stdout.log"
$buildStderrPath = Join-Path $artifactDir "hp-igpu-01-r0-v12-build.stderr.log"
$coreMetricPaths = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-core-r0-v12-series-$_.json" })
$navMetricPaths = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-nav-r0-v12-series-$_.json" })
$coreStdoutPaths = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-core-r0-v12-series-$_.stdout.log" })
$coreStderrPaths = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-core-r0-v12-series-$_.stderr.log" })
$navStdoutPaths = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-nav-r0-v12-series-$_.stdout.log" })
$navStderrPaths = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-nav-r0-v12-series-$_.stderr.log" })
$coreReferencePath = Join-Path $referenceDir "hp-dev-01-portable-core-r0-v12-provenance.json"
$navReferencePath = Join-Path $referenceDir "hp-dev-01-portable-nav-r0-v12-provenance.json"

$expectedLockSha256 = "01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176"
$expectedRunnerSha256 = "cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164"
$expectedValidatorSha256 = "2efd7ab90ff199c2cd9669fbb603af6ba1db58b1ef264e4d126baed5564c0c56"
$expectedCoreReferenceSha256 = "d24a34e50cfe910aa30f702344ecb951ad96dcdf578917ed3b58e83b3a50d090"
$expectedNavReferenceSha256 = "51de8f7bfdfb9697a66de1edec65d7bb0c447c42ed4846a9834ccabefae983da"
$expectedBuildInputTreeSha256 = "6dc2be8e1cfe992247d2946853c77977915ba249930437b6797f0b053d65b3b6"

function Assert-Contract([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Get-LowerSha256([string]$Path) {
    Assert-Contract (Test-Path $Path -PathType Leaf) "Missing required evidence: $Path"
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

function Read-Json([string]$Path) {
    Assert-Contract (Test-Path $Path -PathType Leaf) "Missing required evidence: $Path"
    try {
        return Get-Content $Path -Raw | ConvertFrom-Json
    } catch {
        throw "Invalid JSON evidence at ${Path}: $($_.Exception.Message)"
    }
}

function Resolve-EvidencePath([string]$RelativePath) {
    $fullPath = [IO.Path]::GetFullPath((Join-Path $repoRoot $RelativePath))
    $rootPrefix = $repoRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    Assert-Contract ($fullPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) `
        "Evidence path escapes the repository: $RelativePath"
    return $fullPath
}

function Assert-FileRecords([object[]]$Records, [string]$Owner) {
    foreach ($record in @($Records)) {
        $path = Resolve-EvidencePath ([string]$record.path)
        $actual = Get-LowerSha256 $path
        Assert-Contract ($actual -eq [string]$record.sha256) `
            "$Owner hash mismatch for $($record.path): expected $($record.sha256), found $actual"
    }
}

function Get-EvidenceRelativePath([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $rootPrefix = $repoRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    Assert-Contract ($fullPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) `
        "Expected evidence path escapes the repository: $Path"
    return $fullPath.Substring($rootPrefix.Length).Replace("\", "/")
}

function Assert-ExactFileRecords([object[]]$Records, [string[]]$ExpectedPaths, [string]$Owner) {
    $actualPaths = @($Records | ForEach-Object { [string]$_.path } | Sort-Object)
    $expectedRelativePaths = @($ExpectedPaths | ForEach-Object { Get-EvidenceRelativePath $_ } | Sort-Object)
    Assert-Contract ($actualPaths.Count -eq $expectedRelativePaths.Count) `
        "$Owner file-record count mismatch: expected $($expectedRelativePaths.Count), found $($actualPaths.Count)."
    for ($index = 0; $index -lt $expectedRelativePaths.Count; $index++) {
        Assert-Contract ($actualPaths[$index] -eq $expectedRelativePaths[$index]) `
            "$Owner file-record membership mismatch at index ${index}: expected $($expectedRelativePaths[$index]), found $($actualPaths[$index])."
    }
    Assert-FileRecords $Records $Owner
}

function Assert-ArtifactRecord([object]$Record, [string]$ExpectedPath, [string]$Owner) {
    Assert-Contract ($null -ne $Record) "$Owner record is missing."
    $expectedRelativePath = Get-EvidenceRelativePath $ExpectedPath
    Assert-Contract ([string]$Record.path -eq $expectedRelativePath) `
        "$Owner path mismatch: expected $expectedRelativePath, found $($Record.path)."
    $actual = Get-LowerSha256 $ExpectedPath
    Assert-Contract ([string]$Record.sha256 -eq $actual) `
        "$Owner hash mismatch: expected $($Record.sha256), found $actual."
}

function Get-NearestRank([object[]]$Samples, [double]$Percentile) {
    Assert-Contract ($Samples.Count -gt 0) "Cannot compute a percentile from an empty sample set."
    $values = [double[]]$Samples
    [Array]::Sort($values)
    $rank = [Math]::Ceiling($Percentile * $values.Length)
    return $values[$rank - 1]
}

function Assert-SummaryValue([double]$Recorded, [double]$Computed, [string]$Name) {
    Assert-Contract ([Math]::Abs($Recorded - $Computed) -le 0.0000005) `
        "$Name does not match the independently recomputed raw-sample value: recorded $Recorded, computed $Computed."
}

function Assert-QualifiedFingerprint([object]$Fingerprint) {
    $snapshot = $Fingerprint.machine
    $attestation = $Fingerprint.operator_attestation
    $portableChassis = @($snapshot.enclosure.chassis_types | Where-Object { $_ -in @(8, 9, 10, 14) })
    Assert-Contract ($snapshot.computer_system.pc_system_type -eq 2 -and
        @($snapshot.batteries).Count -gt 0 -and $portableChassis.Count -gt 0) `
        "HP-IGPU-01 fingerprint is not an objectively identified physical notebook."
    Assert-Contract ([int]$attestation.release_year -ge 2023 -and [int]$attestation.release_year -le 2026 -and
        -not [string]::IsNullOrWhiteSpace([string]$attestation.retail_model_evidence)) `
        "HP-IGPU-01 fingerprint lacks valid 2023-2026 retail model evidence."
    Assert-Contract ([string]$snapshot.os.caption -like "*Windows 11*" -and
        [int]$snapshot.os.build_number -ge 22631 -and [bool]$attestation.fully_patched_confirmed) `
        "HP-IGPU-01 fingerprint does not satisfy the frozen Windows 11 requirement."
    Assert-Contract ([int]$snapshot.cpu.architecture -eq 9 -and [int]$snapshot.cpu.physical_cores -ge 4 -and
        [int]$attestation.nominal_cpu_power_w -ge 15 -and [int]$attestation.nominal_cpu_power_w -le 30) `
        "HP-IGPU-01 fingerprint does not satisfy the frozen mobile CPU requirement."

    $selectedGpu = @($snapshot.gpus | Where-Object { $_.name -eq [string]$attestation.integrated_gpu_name })
    $operationalGpus = @($snapshot.gpus | Where-Object { $_.status -eq "OK" })
    Assert-Contract (-not [string]::IsNullOrWhiteSpace([string]$attestation.integrated_gpu_name) -and
        $selectedGpu.Count -eq 1 -and $selectedGpu[0].status -eq "OK" -and
        $operationalGpus.Count -eq 1 -and $operationalGpus[0].name -eq [string]$attestation.integrated_gpu_name -and
        -not [string]::IsNullOrWhiteSpace([string]$selectedGpu[0].driver_version) -and
        [bool]$attestation.direct3d_12_confirmed -and [bool]$attestation.discrete_gpu_disabled_confirmed -and
        [bool]$attestation.production_driver_confirmed) `
        "HP-IGPU-01 fingerprint does not satisfy the frozen integrated-GPU requirement."

    $ramGiB = [double]$snapshot.computer_system.total_physical_memory_bytes / 1GB
    Assert-Contract ($ramGiB -ge 15.5 -and $ramGiB -le 16.5 -and
        [double]$attestation.shared_gpu_budget_gib -gt 0 -and [double]$attestation.shared_gpu_budget_gib -le 4) `
        "HP-IGPU-01 fingerprint does not satisfy the frozen memory requirement."
    $matchingModes = @($snapshot.display.modes | Where-Object {
        $_.gpu_name -eq [string]$attestation.integrated_gpu_name -and [int]$_.width -eq 1920 -and
        [int]$_.height -eq 1080 -and [int]$_.refresh_rate_hz -eq 60
    })
    Assert-Contract ($matchingModes.Count -gt 0 -and [int]$snapshot.display.applied_dpi -eq 96) `
        "HP-IGPU-01 fingerprint does not satisfy the frozen display requirement."
    Assert-Contract ([bool]$snapshot.gate_state.ac_power -and
        [bool]$attestation.vendor_balanced_profile_confirmed -and [bool]$attestation.background_state_confirmed) `
        "HP-IGPU-01 fingerprint does not satisfy the frozen power and background-state requirement."

    $batteryIdentity = @($snapshot.batteries | ForEach-Object {
        [ordered]@{
            device_id = $_.device_id
            pnp_device_id = $_.pnp_device_id
            design_capacity_mwh = $_.design_capacity_mwh
        }
    })
    $configuration = [ordered]@{
        machine = [ordered]@{
            computer_system = $snapshot.computer_system
            system_product = $snapshot.system_product
            bios = $snapshot.bios
            baseboard = $snapshot.baseboard
            enclosure = $snapshot.enclosure
            cpu = $snapshot.cpu
            memory_modules = $snapshot.memory_modules
            gpus = $snapshot.gpus
            os = $snapshot.os
            battery_identity = $batteryIdentity
            display = $snapshot.display
            gate_state = [ordered]@{
                ac_power = $snapshot.gate_state.ac_power
                active_power_scheme_guid = $snapshot.gate_state.active_power_scheme_guid
            }
        }
        operator_attestation = $attestation
    }
    $configurationJson = $configuration | ConvertTo-Json -Depth 12 -Compress
    Assert-Contract ((Get-TextSha256 $configurationJson) -eq [string]$Fingerprint.machine_configuration_sha256) `
        "HP-IGPU-01 fingerprint machine-configuration digest mismatch."
}

function Assert-ReferenceProvenance([object]$Reference, [string]$Kind) {
    Assert-Contract ($Reference.schema_version -eq 1) "$Kind reference schema mismatch."
    Assert-Contract ($Reference.profile_id -eq "HP-DEV-01") "$Kind reference profile mismatch."
    Assert-Contract ($Reference.freeze_id -eq "r0-v12") "$Kind reference freeze mismatch."
    Assert-Contract ($Reference.r0_lock_sha256 -eq $expectedLockSha256) "$Kind reference lock mismatch."
    Assert-Contract ($Reference.contract -eq "portable-build-provenance-v1") "$Kind reference provenance contract mismatch."
    Assert-Contract ($Reference.build_input_tree_sha256 -eq $expectedBuildInputTreeSha256) "$Kind reference build inputs mismatch."
    Assert-Contract ($Reference.runner_script_sha256 -eq $expectedRunnerSha256) "$Kind reference runner mismatch."
    Assert-Contract ($Reference.validator_script_sha256 -eq $expectedValidatorSha256) "$Kind reference validator mismatch."
    Assert-Contract ([bool]$Reference.clean_build) "$Kind reference was not a clean build."
    Assert-Contract ($Reference.decision -eq "PASS") "$Kind reference did not pass."
    Assert-Contract (@($Reference.metrics).Count -eq 3) "$Kind reference must contain three series."
    Assert-FileRecords @($Reference.files) "$Kind reference"
}

function Assert-CoreMetric([object]$Metric, [int]$Series) {
    Assert-Contract ($Metric.schema_version -eq 1) "Core series $Series schema mismatch."
    Assert-Contract ($Metric.profile_id -eq "HP-IGPU-01") "Core series $Series profile mismatch."
    Assert-Contract ([int]$Metric.series -eq $Series) "Core series number mismatch."
    Assert-Contract ($Metric.r0_lock_sha256 -eq $expectedLockSha256) "Core series $Series lock mismatch."
    Assert-Contract ([int]$Metric.occurrences -eq 10000) "Core series $Series occurrence count mismatch."
    Assert-Contract ([int]$Metric.edit_warmup -eq 100 -and [int]$Metric.edit_samples -eq 1000) `
        "Core series $Series edit sample contract mismatch."
    Assert-Contract ([int]$Metric.pick_warmup -eq 200 -and [int]$Metric.pick_samples -eq 2000) `
        "Core series $Series pick sample contract mismatch."
    Assert-Contract ([int]$Metric.long_samples -eq 100) "Core series $Series long-job sample contract mismatch."
    Assert-Contract (@($Metric.edit_ms).Count -eq 1000) "Core series $Series edit sample cardinality mismatch."
    Assert-Contract (@($Metric.pick_snap_ms).Count -eq 2000) "Core series $Series pick sample cardinality mismatch."
    Assert-Contract (@($Metric.navigation_block_ms).Count -eq 100) "Core series $Series navigation sample cardinality mismatch."
    Assert-Contract (@($Metric.cancellation_ms).Count -eq 100) "Core series $Series cancellation sample cardinality mismatch."
    foreach ($class in @("face", "edge", "endpoint", "midpoint", "intersection")) {
        Assert-Contract ([int]$Metric.pick_class_counts.$class -eq 400) `
            "Core series $Series pick class $class cardinality mismatch."
    }
    Assert-SummaryValue ([double]$Metric.edit_p95_ms) (Get-NearestRank @($Metric.edit_ms) 0.95) "Core series $Series edit p95"
    Assert-SummaryValue ([double]$Metric.pick_snap_p95_ms) (Get-NearestRank @($Metric.pick_snap_ms) 0.95) "Core series $Series pick/snap p95"
    Assert-SummaryValue ([double]$Metric.navigation_block_max_ms) `
        ([double](@($Metric.navigation_block_ms) | Measure-Object -Maximum).Maximum) "Core series $Series navigation maximum"
    Assert-SummaryValue ([double]$Metric.cancel_p95_ms) (Get-NearestRank @($Metric.cancellation_ms) 0.95) "Core series $Series cancellation p95"
    Assert-Contract ([double]$Metric.edit_p95_ms -le 100.0) "Core series $Series failed QC-C-EDIT-01 latency."
    Assert-Contract ([double]$Metric.action_digest_match_percent -eq 100.0) "Core series $Series failed action digest identity."
    Assert-Contract ([double]$Metric.pick_snap_p95_ms -le 50.0) "Core series $Series failed QC-C-PICK-01 latency."
    Assert-Contract ([int]$Metric.wrong_identity_count -eq 0) "Core series $Series produced a wrong identity."
    Assert-Contract ([double]$Metric.navigation_block_max_ms -le 100.0) "Core series $Series blocked navigation."
    Assert-Contract ([double]$Metric.cancel_p95_ms -le 250.0) "Core series $Series failed cancellation latency."
    Assert-Contract ([int]$Metric.committed_data_loss_count -eq 0) "Core series $Series lost committed data."
}

function Assert-TerminalStageSequence([object]$Manifest, [string]$TerminalDecision) {
    $expectedStages = @("release-build", "core-series-1", "core-series-2", "core-series-3", "navigation-series-1", "navigation-series-2", "navigation-series-3")
    $stages = @($Manifest.stages)
    Assert-Contract ($stages.Count -ge 1 -and $stages.Count -le $expectedStages.Count) `
        "A terminal non-PASS manifest must contain a non-empty canonical stage prefix."
    for ($index = 0; $index -lt $stages.Count; $index++) {
        $stage = $stages[$index]
        Assert-Contract ([string]$stage.stage_id -eq $expectedStages[$index]) `
            "Terminal run stage order mismatch at index $index."
        Assert-Contract ([string]$stage.decision -in @("PASS", "FAIL", "INFRASTRUCTURE_INVALID")) `
            "Run stage $($stage.stage_id) has an unknown decision."
        if ($index -lt $stages.Count - 1) {
            Assert-Contract ([string]$stage.decision -eq "PASS" -and [int]$stage.exit_code -eq 0) `
                "A stage before the terminal stage did not pass: $($stage.stage_id)."
        }
    }
    Assert-Contract (-not [string]::IsNullOrWhiteSpace([string]$Manifest.failure_message)) `
        "A terminal non-PASS manifest must contain a failure message."

    $lastStage = $stages[-1]
    if ([string]$Manifest.failing_stage -eq "executable-verification") {
        Assert-Contract ($TerminalDecision -eq "INFRASTRUCTURE_INVALID" -and $stages.Count -eq 1 -and
            [string]$lastStage.stage_id -eq "release-build" -and [string]$lastStage.decision -eq "PASS" -and
            [int]$lastStage.exit_code -eq 0) `
            "Executable-verification failure does not match the frozen runner transition."
        return
    }

    Assert-Contract ([string]$Manifest.failing_stage -eq [string]$lastStage.stage_id) `
        "Terminal failing-stage identity does not match the final recorded stage."
    Assert-Contract ([string]$lastStage.decision -eq $TerminalDecision) `
        "Terminal decision does not match the final recorded stage decision."
    if ($TerminalDecision -eq "FAIL") {
        Assert-Contract ([string]::IsNullOrWhiteSpace([string]$lastStage.launch_error)) `
            "A measured FAIL stage unexpectedly contains an infrastructure launch error."
        Assert-Contract ($null -ne $lastStage.exit_code -and [int]$lastStage.exit_code -ne 0) `
            "A measured FAIL stage must contain the runner's nonzero process exit code."
    } elseif (-not [string]::IsNullOrWhiteSpace([string]$lastStage.launch_error)) {
        Assert-Contract ($null -eq $lastStage.exit_code) `
            "An infrastructure launch error unexpectedly contains a process exit code."
    } else {
        Assert-Contract ($null -ne $lastStage.exit_code) `
            "An infrastructure-invalid recorded stage lacks both a launch error and a process exit code."
        if ([string]$lastStage.stage_id -eq "release-build") {
            Assert-Contract ([int]$lastStage.exit_code -ne 0) `
                "A release build with exit code zero is not a runner-realizable infrastructure failure."
        }
    }
}

function Assert-NavMetric([object]$Metric, [int]$Series, [string]$ExpectedAdapterName) {
    Assert-Contract ($Metric.schema_version -eq 1) "NAV series $Series schema mismatch."
    Assert-Contract ($Metric.query_class -eq "QC-C-NAV-01") "NAV series $Series query class mismatch."
    Assert-Contract ($Metric.profile_id -eq "HP-IGPU-01") "NAV series $Series profile mismatch."
    Assert-Contract ([int]$Metric.series -eq $Series) "NAV series number mismatch."
    Assert-Contract ($Metric.r0_lock_sha256 -eq $expectedLockSha256) "NAV series $Series lock mismatch."
    Assert-Contract ($Metric.selected_adapter.name -eq $ExpectedAdapterName) "NAV series $Series adapter identity mismatch."
    Assert-Contract ($Metric.selected_adapter.device_type -eq "integrated-gpu") "NAV series $Series did not use an integrated GPU."
    Assert-Contract ($Metric.selected_adapter.backend -eq "dx12") "NAV series $Series did not use Direct3D 12."
    Assert-Contract ([int]$Metric.runs -eq 30) "NAV series $Series run count mismatch."
    Assert-Contract ([int]$Metric.warmup_seconds_per_run -eq 10) "NAV series $Series warm-up mismatch."
    Assert-Contract ([int]$Metric.measurement_seconds_per_run -eq 30) "NAV series $Series measurement duration mismatch."
    Assert-Contract ([int]$Metric.occurrences -eq 10000) "NAV series $Series occurrence count mismatch."
    Assert-Contract ([int]$Metric.visible_tessellated_triangles -le 500000) "NAV series $Series exceeded the triangle envelope."
    Assert-Contract ([int]$Metric.shared_authoritative_geometry -eq 1) "NAV series $Series did not share authoritative geometry."
    Assert-Contract (@($Metric.per_run_frame_sample_counts).Count -eq 30) "NAV series $Series frame run cardinality mismatch."
    Assert-Contract (@($Metric.per_run_input_to_preview_sample_counts).Count -eq 30) `
        "NAV series $Series preview run cardinality mismatch."
    $frameCount = (@($Metric.per_run_frame_sample_counts) | Measure-Object -Sum).Sum
    $previewCount = (@($Metric.per_run_input_to_preview_sample_counts) | Measure-Object -Sum).Sum
    Assert-Contract (@($Metric.frame_ms).Count -eq $frameCount) "NAV series $Series frame sample cardinality mismatch."
    Assert-Contract (@($Metric.input_to_preview_ms).Count -eq $previewCount) "NAV series $Series preview sample cardinality mismatch."
    Assert-SummaryValue ([double]$Metric.frame_p95_ms) (Get-NearestRank @($Metric.frame_ms) 0.95) "NAV series $Series frame p95"
    Assert-SummaryValue ([double]$Metric.frame_p99_ms) (Get-NearestRank @($Metric.frame_ms) 0.99) "NAV series $Series frame p99"
    Assert-SummaryValue ([double]$Metric.input_to_preview_p95_ms) `
        (Get-NearestRank @($Metric.input_to_preview_ms) 0.95) "NAV series $Series input-to-preview p95"
    Assert-Contract ([double]$Metric.frame_p95_ms -le 16.7) "NAV series $Series failed frame p95."
    Assert-Contract ([double]$Metric.frame_p99_ms -le 33.3) "NAV series $Series failed frame p99."
    Assert-Contract ([double]$Metric.input_to_preview_p95_ms -le 50.0) "NAV series $Series failed input-to-preview p95."
}

Assert-Contract ((Get-LowerSha256 $lockPath) -eq $expectedLockSha256) "R0 v12 lock hash mismatch."
Assert-Contract ((Get-LowerSha256 $runnerPath) -eq $expectedRunnerSha256) "Frozen HP-IGPU-01 runner hash mismatch."
Assert-Contract ((Get-LowerSha256 $validatorPath) -eq $expectedValidatorSha256) "R0 v12 validator hash mismatch."
Assert-Contract ((Get-LowerSha256 $coreReferencePath) -eq $expectedCoreReferenceSha256) "HP-DEV-01 core reference provenance hash mismatch."
Assert-Contract ((Get-LowerSha256 $navReferencePath) -eq $expectedNavReferenceSha256) "HP-DEV-01 NAV reference provenance hash mismatch."

$coreReference = Read-Json $coreReferencePath
$navReference = Read-Json $navReferencePath
Assert-ReferenceProvenance $coreReference "Core"
Assert-ReferenceProvenance $navReference "NAV"

if (-not (Test-Path $fingerprintPath -PathType Leaf) -or
    -not (Test-Path $attemptClaimPath -PathType Leaf) -or
    -not (Test-Path $runManifestPath -PathType Leaf)) {
    throw "Gate C closure evidence is incomplete: the immutable HP-IGPU-01 fingerprint, attempt claim, and run manifest are all required."
}

$fingerprint = Read-Json $fingerprintPath
$attemptClaim = Read-Json $attemptClaimPath
$runManifest = Read-Json $runManifestPath
$fingerprintSha256 = Get-LowerSha256 $fingerprintPath

Assert-Contract ($fingerprint.schema_version -eq 1) "HP-IGPU-01 fingerprint schema mismatch."
Assert-Contract ($fingerprint.profile_id -eq "HP-IGPU-01") "HP-IGPU-01 fingerprint profile mismatch."
Assert-Contract ($fingerprint.freeze_id -eq "r0-v12") "HP-IGPU-01 fingerprint freeze mismatch."
Assert-Contract ($fingerprint.r0_lock_sha256 -eq $expectedLockSha256) "HP-IGPU-01 fingerprint lock mismatch."
Assert-Contract ($fingerprint.qualification_decision -eq "PASS") "HP-IGPU-01 fingerprint did not pass qualification."
Assert-Contract ($fingerprint.runner_script_sha256 -eq $expectedRunnerSha256) "HP-IGPU-01 fingerprint runner mismatch."
Assert-QualifiedFingerprint $fingerprint
Assert-Contract ($fingerprint.build_provenance.build_inputs.tree_sha256 -eq $expectedBuildInputTreeSha256) `
    "HP-IGPU-01 fingerprint build-input tree mismatch."

Assert-Contract ($attemptClaim.schema_version -eq 1) "Attempt claim schema mismatch."
Assert-Contract ($attemptClaim.profile_id -eq "HP-IGPU-01") "Attempt claim profile mismatch."
Assert-Contract ($attemptClaim.freeze_id -eq "r0-v12") "Attempt claim freeze mismatch."
Assert-Contract ($attemptClaim.r0_lock_sha256 -eq $expectedLockSha256) "Attempt claim lock mismatch."
Assert-Contract ($attemptClaim.fingerprint_sha256 -eq $fingerprintSha256) "Attempt claim fingerprint mismatch."
Assert-Contract ($attemptClaim.runner_script_sha256 -eq $expectedRunnerSha256) "Attempt claim runner mismatch."
Assert-Contract ($attemptClaim.build_input_tree_sha256 -eq $expectedBuildInputTreeSha256) "Attempt claim build inputs mismatch."

Assert-Contract ($runManifest.schema_version -eq 2) "Run manifest schema mismatch."
Assert-Contract ($runManifest.profile_id -eq "HP-IGPU-01") "Run manifest profile mismatch."
Assert-Contract ($runManifest.freeze_id -eq "r0-v12") "Run manifest freeze mismatch."
Assert-Contract ($runManifest.r0_lock_sha256 -eq $expectedLockSha256) "Run manifest lock mismatch."
Assert-Contract ($runManifest.fingerprint_sha256 -eq $fingerprintSha256) "Run manifest fingerprint mismatch."
Assert-Contract ($runManifest.runner_script_sha256 -eq $expectedRunnerSha256) "Run manifest runner mismatch."
Assert-Contract ($runManifest.build_provenance.build_inputs.tree_sha256 -eq $expectedBuildInputTreeSha256) `
    "Run manifest build inputs mismatch."
Assert-Contract ($runManifest.build_provenance.occt_install_tree.sha256 -eq $attemptClaim.occt_install_tree_sha256) `
    "Run manifest OCCT tree differs from the attempt claim."
Assert-Contract ([string]$attemptClaim.started_utc -eq [string]$runManifest.started_utc) `
    "Attempt claim start time differs from the run manifest."
Assert-Contract ([string]$runManifest.fingerprint_path -eq (Get-EvidenceRelativePath $fingerprintPath)) `
    "Run manifest fingerprint path mismatch."

$terminalDecision = [string]$runManifest.decision
Assert-Contract ($terminalDecision -in @("PASS", "FAIL", "INFRASTRUCTURE_INVALID")) "Unknown terminal Gate C decision: $terminalDecision"
if ($terminalDecision -eq "PASS") {
    $expectedEvidencePaths = @($attemptClaimPath, $buildStdoutPath, $buildStderrPath) +
        @($coreMetricPaths) + @($navMetricPaths) + @($coreStdoutPaths) + @($coreStderrPaths) +
        @($navStdoutPaths) + @($navStderrPaths)
} else {
    Assert-TerminalStageSequence $runManifest $terminalDecision
    $expectedEvidencePaths = [Collections.Generic.List[string]]::new()
    $expectedEvidencePaths.Add($attemptClaimPath)
    foreach ($stage in @($runManifest.stages)) {
        $stageId = [string]$stage.stage_id
        $expectedMetricPath = $null
        if ($stageId -eq "release-build") {
            $expectedStdoutPath = $buildStdoutPath
            $expectedStderrPath = $buildStderrPath
        } elseif ($stageId -match '^core-series-([1-3])$') {
            $seriesIndex = [int]$Matches[1] - 1
            $expectedStdoutPath = $coreStdoutPaths[$seriesIndex]
            $expectedStderrPath = $coreStderrPaths[$seriesIndex]
            $expectedMetricPath = $coreMetricPaths[$seriesIndex]
        } elseif ($stageId -match '^navigation-series-([1-3])$') {
            $seriesIndex = [int]$Matches[1] - 1
            $expectedStdoutPath = $navStdoutPaths[$seriesIndex]
            $expectedStderrPath = $navStderrPaths[$seriesIndex]
            $expectedMetricPath = $navMetricPaths[$seriesIndex]
        } else {
            throw "Unknown canonical Gate C stage: $stageId"
        }
        Assert-ArtifactRecord $stage.stdout $expectedStdoutPath "$stageId stdout"
        Assert-ArtifactRecord $stage.stderr $expectedStderrPath "$stageId stderr"
        $expectedEvidencePaths.Add($expectedStdoutPath)
        $expectedEvidencePaths.Add($expectedStderrPath)
        if ($null -eq $expectedMetricPath) {
            Assert-Contract ($null -eq $stage.metric_artifact) "$stageId unexpectedly contains a metric artifact."
        } elseif ($null -ne $stage.metric_artifact) {
            Assert-ArtifactRecord $stage.metric_artifact $expectedMetricPath "$stageId metric"
            $expectedEvidencePaths.Add($expectedMetricPath)
        } else {
            Assert-Contract ([string]$stage.decision -eq "INFRASTRUCTURE_INVALID") `
                "$stageId passed or failed without its canonical metric artifact."
        }
    }
}
Assert-ExactFileRecords @($runManifest.evidence) @($expectedEvidencePaths) "HP-IGPU-01 run manifest"
$reportDecision = if ($terminalDecision -eq "PASS") { "GO" } elseif ($terminalDecision -eq "FAIL") { "NO-GO" } else { "INFRASTRUCTURE_INVALID" }
$reportFileName = if ($terminalDecision -eq "PASS") { "report.md" } elseif ($terminalDecision -eq "FAIL") { "report-no-go.md" } else { "report-infrastructure-invalid.md" }
$reportPath = Join-Path $artifactDir $reportFileName
$coreMetrics = @()
$navMetrics = @()

if ($terminalDecision -eq "PASS") {
    $expectedStages = @("release-build", "core-series-1", "core-series-2", "core-series-3", "navigation-series-1", "navigation-series-2", "navigation-series-3")
    Assert-Contract (@($runManifest.stages).Count -eq $expectedStages.Count) "A passing run manifest must contain exactly seven stages."
    for ($index = 0; $index -lt $expectedStages.Count; $index++) {
        $stage = @($runManifest.stages)[$index]
        Assert-Contract ($stage.stage_id -eq $expectedStages[$index]) "Run stage order mismatch at index $index."
        Assert-Contract ($stage.decision -eq "PASS" -and [int]$stage.exit_code -eq 0) "Run stage $($stage.stage_id) did not pass."
    }
    $buildStage = @($runManifest.stages)[0]
    Assert-ArtifactRecord $buildStage.stdout $buildStdoutPath "Release-build stdout"
    Assert-ArtifactRecord $buildStage.stderr $buildStderrPath "Release-build stderr"
    Assert-Contract ($null -eq $buildStage.metric_artifact) "Release-build stage unexpectedly contains a metric artifact."
    Assert-Contract ($null -eq $runManifest.failing_stage -and $null -eq $runManifest.failure_message) `
        "A passing run manifest contains terminal failure metadata."
    foreach ($series in 1..3) {
        $corePath = $coreMetricPaths[$series - 1]
        $navPath = $navMetricPaths[$series - 1]
        $coreMetric = Read-Json $corePath
        $navMetric = Read-Json $navPath
        Assert-CoreMetric $coreMetric $series
        Assert-NavMetric $navMetric $series ([string]$fingerprint.operator_attestation.integrated_gpu_name)
        $coreStage = @($runManifest.stages)[$series]
        $navStage = @($runManifest.stages)[$series + 3]
        Assert-ArtifactRecord $coreStage.stdout $coreStdoutPaths[$series - 1] "Core series $series stdout"
        Assert-ArtifactRecord $coreStage.stderr $coreStderrPaths[$series - 1] "Core series $series stderr"
        Assert-ArtifactRecord $coreStage.metric_artifact $corePath "Core series $series metric"
        Assert-ArtifactRecord $navStage.stdout $navStdoutPaths[$series - 1] "NAV series $series stdout"
        Assert-ArtifactRecord $navStage.stderr $navStderrPaths[$series - 1] "NAV series $series stderr"
        Assert-ArtifactRecord $navStage.metric_artifact $navPath "NAV series $series metric"
        $coreMetrics += $coreMetric
        $navMetrics += $navMetric
    }
    $resultFingerprints = @($coreMetrics | ForEach-Object { [string]$_.result_fingerprint } | Sort-Object -Unique)
    Assert-Contract ($resultFingerprints.Count -eq 1 -and -not [string]::IsNullOrWhiteSpace($resultFingerprints[0])) `
        "The three core series do not share one deterministic result fingerprint."
}

$reportLines = [Collections.Generic.List[string]]::new()
$reportLines.Add("# Gate C Decision Report")
$reportLines.Add("")
$reportLines.Add("**Decision: $reportDecision**")
$reportLines.Add("")
$reportLines.Add("## Evidence boundary")
$reportLines.Add("")
$reportLines.Add("This report was generated from the frozen r0-v12 contract, sealed HP-DEV-01 references, and the immutable HP-IGPU-01 fingerprint and run manifest. No threshold, hardware profile, query class, corpus, or historical observation was changed after measurement.")
$reportLines.Add("")
$reportLines.Add("- R0 lock SHA-256: ``$expectedLockSha256``")
$reportLines.Add("- HP-IGPU-01 runner SHA-256: ``$expectedRunnerSha256``")
$reportLines.Add("- HP-DEV-01 core provenance SHA-256: ``$expectedCoreReferenceSha256``")
$reportLines.Add("- HP-DEV-01 NAV provenance SHA-256: ``$expectedNavReferenceSha256``")
$reportLines.Add("- HP-IGPU-01 fingerprint SHA-256: ``$fingerprintSha256``")
$reportLines.Add("- HP-IGPU-01 run manifest SHA-256: ``$(Get-LowerSha256 $runManifestPath)``")
$reportLines.Add("- Report validator SHA-256: ``$(Get-LowerSha256 $PSCommandPath)``")
$reportLines.Add("")

if ($terminalDecision -eq "PASS") {
    $reportLines.Add("## HP-IGPU-01 frozen-threshold results")
    $reportLines.Add("")
    $reportLines.Add("| Series | Edit p95 ms | Digest match % | Pick/snap p95 ms | Wrong IDs | Navigation block max ms | Cancel p95 ms | Data loss |")
    $reportLines.Add("|---:|---:|---:|---:|---:|---:|---:|---:|")
    foreach ($metric in $coreMetrics) {
        $reportLines.Add("| $($metric.series) | $($metric.edit_p95_ms) | $($metric.action_digest_match_percent) | $($metric.pick_snap_p95_ms) | $($metric.wrong_identity_count) | $($metric.navigation_block_max_ms) | $($metric.cancel_p95_ms) | $($metric.committed_data_loss_count) |")
    }
    $reportLines.Add("")
    $reportLines.Add("| Series | Frame p95 ms | Frame p99 ms | Input-to-preview p95 ms | Adapter | Backend |")
    $reportLines.Add("|---:|---:|---:|---:|---|---|")
    foreach ($metric in $navMetrics) {
        $reportLines.Add("| $($metric.series) | $($metric.frame_p95_ms) | $($metric.frame_p99_ms) | $($metric.input_to_preview_p95_ms) | $($metric.selected_adapter.name) | $($metric.selected_adapter.backend) |")
    }
    $reportLines.Add("")
    $reportLines.Add("All three consecutive complete series passed every frozen Gate C correctness and latency threshold on both required hardware profiles. The HP-IGPU-01 NAV series used the fingerprinted integrated Direct3D 12 adapter, and all raw sample cardinalities matched their per-run counts.")
} else {
    $reportLines.Add("## Terminal evidence")
    $reportLines.Add("")
    $reportLines.Add("- Terminal run decision: ``$terminalDecision``")
    $reportLines.Add("- Failing stage: ``$($runManifest.failing_stage)``")
    $reportLines.Add("- Failure message: $($runManifest.failure_message)")
    $reportLines.Add("")
    if ($terminalDecision -eq "FAIL") {
        $reportLines.Add("The sealed physical-notebook attempt contains a valid measured failure. Gate C is NO-GO and blocks AI tool-surface expansion; the failed run is not weakened or rewritten.")
    } else {
        $reportLines.Add("The sealed physical-notebook attempt is infrastructure-invalid. It is neither a Gate C pass nor a measured product failure; preserve the attempt and issue a new preregistration before any replacement observation.")
    }
}
$reportLines.Add("")
$reportLines.Add("## Verification")
$reportLines.Add("")
$reportLines.Add("- Every provenance and run-manifest file record was rehashed and matched.")
$reportLines.Add("- The fingerprint, attempt claim, run manifest, build inputs, runner, and R0 lock formed one identity chain.")
$reportLines.Add("- The sealed HP-DEV-01 core and NAV reference sets both contained three passing series.")
if ($terminalDecision -eq "PASS") {
    $reportLines.Add("- All six HP-IGPU-01 metrics matched the frozen schemas, sample counts, adapter contract, and thresholds.")
}
$reportLines.Add("")
$reportLines.Add("## Consequence")
$reportLines.Add("")
if ($terminalDecision -eq "PASS") {
    $reportLines.Add("Gate C is GO. The localization-ready viewport, exact picking and snapping, cancellable preview, and Smart Push/Pull interaction contract may proceed to the 20-task FLP evaluation without expanding the renderer or AI command surface beyond the frozen scope.")
} elseif ($terminalDecision -eq "FAIL") {
    $reportLines.Add("Gate C is NO-GO. Apply the frozen failure consequence to the measured class and do not advance to FLP validation.")
} else {
    $reportLines.Add("Gate C remains open. Do not advance to FLP validation from this infrastructure-invalid attempt.")
}

if ($WriteReport) {
    Assert-Contract (-not (Test-Path $reportPath)) "Gate C report already exists and will not be overwritten: $reportPath"
    $text = ($reportLines -join [Environment]::NewLine) + [Environment]::NewLine
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text)
    $stream = [IO.FileStream]::new($reportPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
    Write-Output "Gate C $reportDecision report written immutably to $reportPath"
} else {
    Write-Output "Gate C evidence validated with terminal decision $terminalDecision; use -WriteReport to create the immutable $reportDecision report."
}
