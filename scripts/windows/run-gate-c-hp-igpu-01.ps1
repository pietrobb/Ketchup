[CmdletBinding()]
param(
    [int]$ReleaseYear = 0,
    [int]$NominalCpuPowerW = 0,
    [double]$SharedGpuBudgetGiB = 0,
    [string]$IntegratedGpuName = "",
    [string]$RetailModelEvidence = "",
    [switch]$Direct3D12Confirmed,
    [switch]$FullyPatchedConfirmed,
    [switch]$DiscreteGpuDisabledConfirmed,
    [switch]$VendorBalancedProfileConfirmed,
    [switch]$ProductionDriverConfirmed,
    [switch]$BackgroundStateConfirmed,
    [switch]$RunFormalMeasurements,
    [switch]$VerifyAttemptSealing
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$artifactDir = Join-Path $repoRoot "artifacts\gate-c"
$lockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock-r0-v13.json"
$fingerprintPath = Join-Path $artifactDir "hp-igpu-01-fingerprint-r0-v13.json"
$runManifestPath = Join-Path $artifactDir "hp-igpu-01-r0-v13-run-manifest.json"
$attemptClaimPath = Join-Path $artifactDir "hp-igpu-01-r0-v13-attempt-claim.json"
$occtManifestPath = Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json"
$occtInstallPath = Join-Path $repoRoot "third_party\occt-install-r0-v1"
$expectedLockSha256 = "b1cf0c769cb46d0c678c1bc579e241356cc85663582a0df72093e2e54086cb01"
$expectedBuildInputTreeSha256 = "de8592b10b5ed88d2ae7cf8394c127d3d7ca1ea8b22830911cc28a8fbdca84bb"
$expectedSourceHashes = [ordered]@{
    "crates/ketchup-app/src/lib.rs" = "69ee2729fbc371924eaa94aa45544f6257a0197438e732a04fbcb1a3b5f9d977"
    "crates/ketchup-app/src/bin/ketchup-gate-c-nav.rs" = "2dfe7857159e9cfca1b5fd5f98cb4bded3649fe96d4524947b96223548efab92"
    "crates/ketchup-scheduler/src/bin/ketchup-gate-c-core.rs" = "1dd8d666d456f8c421b498628bf9f9e9a2d48bf02eabac817300aa8c15bb4b84"
    "crates/ketchup-scheduler/src/bin/ketchup-exact-worker.rs" = "d7091e406934b992e3910e9937781030f3d05853677ed66885dc53babf4fcddc"
}

function Get-LowerSha256([string]$Path) {
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

function Get-TreeFingerprint([string]$Root) {
    $rootPath = (Resolve-Path $Root).Path.TrimEnd("\")
    $files = @(Get-ChildItem $rootPath -File -Force -Recurse | Sort-Object FullName)
    $lines = [Text.StringBuilder]::new()
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($rootPath.Length).TrimStart("\").Replace("\", "/")
        $hash = Get-LowerSha256 $file.FullName
        [void]$lines.Append($relative).Append("|").Append($file.Length).Append("|").Append($hash).Append("`n")
    }
    return [ordered]@{
        file_count = $files.Count
        sha256 = Get-TextSha256 $lines.ToString()
    }
}

function Get-BuildInputManifest([string]$Root) {
    $paths = [Collections.Generic.List[string]]::new()
    foreach ($relative in @("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")) {
        $paths.Add((Join-Path $Root $relative))
    }
    foreach ($relativeRoot in @("crates", "locales")) {
        $inputRoot = Join-Path $Root $relativeRoot
        foreach ($file in @(Get-ChildItem $inputRoot -File -Force -Recurse)) {
            $paths.Add($file.FullName)
        }
    }
    $paths.Sort([StringComparer]::Ordinal)
    $rootPrefix = [IO.Path]::GetFullPath($Root).TrimEnd("\") + "\"
    $records = @($paths | ForEach-Object {
        $fullPath = [IO.Path]::GetFullPath($_)
        if (-not $fullPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Build input escapes the repository: $fullPath"
        }
        [ordered]@{
            path = $fullPath.Substring($rootPrefix.Length).Replace("\", "/")
            sha256 = Get-LowerSha256 $fullPath
        }
    })
    $lines = ($records | ForEach-Object { "$($_.path)|$($_.sha256)" }) -join "`n"
    return [ordered]@{
        schema_version = 1
        file_count = $records.Count
        tree_sha256 = Get-TextSha256 ($lines + "`n")
        files = $records
    }
}

function Get-BuildProvenance([string]$Root, [object]$OcctManifest) {
    $buildInputs = Get-BuildInputManifest $Root
    if ($buildInputs.tree_sha256 -ne $expectedBuildInputTreeSha256) {
        throw "Frozen Gate C build-input tree hash mismatch: expected $expectedBuildInputTreeSha256, found $($buildInputs.tree_sha256)."
    }

    $rustBin = Join-Path $env:USERPROFILE ".rustup\toolchains\1.97.0-x86_64-pc-windows-msvc\bin"
    $msvcBin = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.35.32215\bin\HostX64\x64"
    $tools = [ordered]@{
        rustc = Join-Path $rustBin "rustc.exe"
        cargo = Join-Path $rustBin "cargo.exe"
        cl = Join-Path $msvcBin "cl.exe"
        link = Join-Path $msvcBin "link.exe"
    }
    $toolHashes = [ordered]@{}
    foreach ($entry in $tools.GetEnumerator()) {
        if (-not (Test-Path $entry.Value -PathType Leaf)) { throw "Missing frozen build tool: $($entry.Value)" }
        $actualHash = Get-LowerSha256 $entry.Value
        if ($actualHash -ne [string]$OcctManifest.toolchain.tool_sha256.($entry.Key)) {
            throw "Frozen build tool hash mismatch: $($entry.Key)"
        }
        $toolHashes[$entry.Key] = $actualHash
    }
    $rustcVerbose = ((& $tools.rustc --version --verbose) -join "`n").Trim()
    if ($LASTEXITCODE -ne 0 -or -not $rustcVerbose.Contains("release: 1.97.0") -or
        -not $rustcVerbose.Contains("host: x86_64-pc-windows-msvc")) {
        throw "The active Rust compiler does not satisfy the frozen Windows toolchain contract."
    }
    $cargoVerbose = ((& $tools.cargo --version --verbose) -join "`n").Trim()
    if ($LASTEXITCODE -ne 0 -or -not $cargoVerbose.Contains("release: 1.97.0") -or
        -not $cargoVerbose.Contains("host: x86_64-pc-windows-msvc")) {
        throw "The active Cargo does not satisfy the frozen Windows toolchain contract."
    }

    $installTree = Get-TreeFingerprint $occtInstallPath
    if ($installTree.file_count -ne [int]$OcctManifest.install_tree.file_count -or
        $installTree.sha256 -ne [string]$OcctManifest.install_tree.sha256) {
        throw "The complete OCCT install tree differs from its frozen fingerprint."
    }
    return [ordered]@{
        schema_version = 1
        build_inputs = $buildInputs
        rustc_verbose = $rustcVerbose
        cargo_verbose = $cargoVerbose
        tool_sha256 = $toolHashes
        occt_manifest_sha256 = Get-LowerSha256 $occtManifestPath
        occt_install_tree = $installTree
        cargo_arguments = @("build", "--locked", "--release", "--bin", "ketchup-gate-c-core", "--bin", "ketchup-exact-worker", "--bin", "ketchup-gate-c-nav")
        target_triple = "x86_64-pc-windows-msvc"
    }
}

function Write-Utf8Json([string]$Path, [object]$Value) {
    $json = $Value | ConvertTo-Json -Depth 12
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

function Write-Utf8JsonExclusive([string]$Path, [object]$Value) {
    $json = ($Value | ConvertTo-Json -Depth 12) + [Environment]::NewLine
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json)
    $stream = [IO.FileStream]::new($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
}

function Get-ArtifactRecord([string]$Path, [string]$Root) {
    if (-not (Test-Path $Path -PathType Leaf)) { return $null }
    $fullPath = [IO.Path]::GetFullPath($Path)
    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $fullPath.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Evidence path is outside the expected root: $fullPath"
    }
    return [ordered]@{
        path = $fullPath.Substring($fullRoot.Length).Replace("\", "/")
        sha256 = Get-LowerSha256 $fullPath
    }
}

function Test-MetricArtifact([string]$Path, [int]$Series, [string]$LockSha256) {
    if (-not (Test-Path $Path -PathType Leaf)) { return $false }
    try {
        $metric = Get-Content $Path -Raw | ConvertFrom-Json
        return $metric.profile_id -eq "HP-IGPU-01" -and
            [int]$metric.series -eq $Series -and
            $metric.r0_lock_sha256 -eq $LockSha256
    } catch {
        return $false
    }
}

function Invoke-RecordedStage(
    [string]$StageId,
    [string]$Executable,
    [object[]]$Arguments,
    [string]$StdoutPath,
    [string]$StderrPath,
    [string]$MetricOutputPath,
    [int]$Series,
    [string]$LockSha256,
    [string]$EvidenceRoot
) {
    $startedUtc = [DateTime]::UtcNow.ToString("o")
    $exitCode = $null
    $launchError = ""
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        & $Executable @Arguments 1> $StdoutPath 2> $StderrPath
        $exitCode = $LASTEXITCODE
    } catch {
        $launchError = $_.Exception.Message
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    foreach ($logPath in @($StdoutPath, $StderrPath)) {
        if (-not (Test-Path $logPath -PathType Leaf)) {
            $stream = [IO.File]::Open($logPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            $stream.Dispose()
        }
    }

    $metricRecord = if ([string]::IsNullOrWhiteSpace($MetricOutputPath)) {
        $null
    } else {
        Get-ArtifactRecord $MetricOutputPath $EvidenceRoot
    }
    $metricIsValid = -not [string]::IsNullOrWhiteSpace($MetricOutputPath) -and
        (Test-MetricArtifact $MetricOutputPath $Series $LockSha256)
    $decision = if (-not [string]::IsNullOrWhiteSpace($launchError)) {
        "INFRASTRUCTURE_INVALID"
    } elseif ($exitCode -eq 0 -and ([string]::IsNullOrWhiteSpace($MetricOutputPath) -or $metricIsValid)) {
        "PASS"
    } elseif ($exitCode -ne 0 -and $metricIsValid) {
        "FAIL"
    } else {
        "INFRASTRUCTURE_INVALID"
    }

    return [ordered]@{
        stage_id = $StageId
        started_utc = $startedUtc
        completed_utc = [DateTime]::UtcNow.ToString("o")
        exit_code = $exitCode
        decision = $decision
        launch_error = if ([string]::IsNullOrWhiteSpace($launchError)) { $null } else { $launchError }
        stdout = Get-ArtifactRecord $StdoutPath $EvidenceRoot
        stderr = Get-ArtifactRecord $StderrPath $EvidenceRoot
        metric_artifact = $metricRecord
    }
}

function New-AttemptManifest(
    [string]$Decision,
    [string]$FailingStage,
    [string]$FailureMessage,
    [string]$StartedUtc,
    [string]$FingerprintPath,
    [string]$FingerprintDisplayPath,
    [string]$RunnerSha256,
    [object]$BuildProvenance,
    [object]$ExecutableSha256,
    [object[]]$Stages,
    [string[]]$EvidencePaths,
    [string]$EvidenceRoot,
    [string]$LockSha256
) {
    $evidence = @($EvidencePaths | ForEach-Object { Get-ArtifactRecord $_ $EvidenceRoot } | Where-Object { $null -ne $_ })
    return [ordered]@{
        schema_version = 2
        profile_id = "HP-IGPU-01"
        freeze_id = "r0-v13"
        r0_lock_sha256 = $LockSha256
        fingerprint_path = $FingerprintDisplayPath
        fingerprint_sha256 = Get-LowerSha256 $FingerprintPath
        runner_script_sha256 = $RunnerSha256
        build_provenance = $BuildProvenance
        started_utc = $StartedUtc
        completed_utc = [DateTime]::UtcNow.ToString("o")
        decision = $Decision
        failing_stage = if ([string]::IsNullOrWhiteSpace($FailingStage)) { $null } else { $FailingStage }
        failure_message = if ([string]::IsNullOrWhiteSpace($FailureMessage)) { $null } else { $FailureMessage }
        executable_sha256 = $ExecutableSha256
        stages = @($Stages)
        evidence = $evidence
    }
}

function Invoke-AttemptSealingSelfTest {
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-gate-c-attempt-" + [Guid]::NewGuid().ToString("n"))
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    try {
        $testLock = "a" * 64
        $fingerprint = Join-Path $testRoot "fingerprint.json"
        Write-Utf8JsonExclusive $fingerprint ([ordered]@{ test = $true })
        $child = Join-Path $testRoot "simulated-failure.ps1"
        $childSource = @'
param([string]$OutputPath, [string]$LockSha256)
$metric = [ordered]@{ schema_version = 1; profile_id = "HP-IGPU-01"; series = 1; r0_lock_sha256 = $LockSha256 }
[IO.File]::WriteAllText($OutputPath, (($metric | ConvertTo-Json -Compress) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
Write-Output "simulated stdout"
[Console]::Error.WriteLine("simulated stderr")
exit 7
'@
        [IO.File]::WriteAllText($child, $childSource, [Text.UTF8Encoding]::new($false))
        $metricPath = Join-Path $testRoot "metric.json"
        $stdoutPath = Join-Path $testRoot "stage.stdout.log"
        $stderrPath = Join-Path $testRoot "stage.stderr.log"
        $hostExecutable = (Get-Process -Id $PID).Path
        $stage = Invoke-RecordedStage "simulated-core-1" $hostExecutable @(
            "-NoProfile", "-NonInteractive", "-File", $child, $metricPath, $testLock
        ) $stdoutPath $stderrPath $metricPath 1 $testLock $testRoot
        if ($stage.decision -ne "FAIL" -or $stage.exit_code -ne 7 -or
            $null -eq $stage.metric_artifact -or $null -eq $stage.stdout -or $null -eq $stage.stderr) {
            throw "Simulated threshold failure was not classified and captured correctly: decision=$($stage.decision), exit=$($stage.exit_code), metric=$($null -ne $stage.metric_artifact), stdout=$($null -ne $stage.stdout), stderr=$($null -ne $stage.stderr)."
        }

        $infrastructureChild = Join-Path $testRoot "simulated-infrastructure-failure.ps1"
        [IO.File]::WriteAllText($infrastructureChild, '[Console]::Error.WriteLine("simulated crash"); exit 9', [Text.UTF8Encoding]::new($false))
        $infrastructureStage = Invoke-RecordedStage "simulated-navigation-1" $hostExecutable @(
            "-NoProfile", "-NonInteractive", "-File", $infrastructureChild
        ) (Join-Path $testRoot "infrastructure.stdout.log") `
            (Join-Path $testRoot "infrastructure.stderr.log") `
            (Join-Path $testRoot "missing-metric.json") 1 $testLock $testRoot
        if ($infrastructureStage.decision -ne "INFRASTRUCTURE_INVALID" -or $infrastructureStage.exit_code -ne 9 -or
            $null -ne $infrastructureStage.metric_artifact) {
            throw "A simulated process crash was not classified as infrastructure-invalid."
        }

        $manifestPath = Join-Path $testRoot "attempt-manifest.json"
        $manifest = New-AttemptManifest "FAIL" $stage.stage_id "simulated threshold failure" `
            ([DateTime]::UtcNow.ToString("o")) $fingerprint "fingerprint.json" `
            (Get-LowerSha256 $PSCommandPath) ([ordered]@{ simulated = $true }) `
            ([ordered]@{ simulated = Get-LowerSha256 $child }) @($stage) @($metricPath, $stdoutPath, $stderrPath) $testRoot $testLock
        Write-Utf8JsonExclusive $manifestPath $manifest
        $sealed = Get-Content $manifestPath -Raw | ConvertFrom-Json
        if ($sealed.decision -ne "FAIL" -or $sealed.failing_stage -ne "simulated-core-1" -or
            @($sealed.stages).Count -ne 1 -or @($sealed.evidence).Count -ne 3) {
            throw "The simulated terminal attempt manifest is incomplete."
        }
        $overwriteRejected = $false
        try {
            Write-Utf8JsonExclusive $manifestPath $manifest
        } catch {
            $overwriteRejected = $true
        }
        if (-not $overwriteRejected) { throw "An immutable attempt manifest could be overwritten." }
        Write-Output "Gate C attempt-sealing self-test passed: failed child evidence was sealed and overwrite was rejected."
    } finally {
        Remove-Item $testRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Get-MachineSnapshot {
    $computer = Get-CimInstance Win32_ComputerSystem
    $product = Get-CimInstance Win32_ComputerSystemProduct
    $bios = Get-CimInstance Win32_BIOS
    $baseboard = Get-CimInstance Win32_BaseBoard
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $os = Get-CimInstance Win32_OperatingSystem
    $windowsVersion = Get-ItemProperty -Path "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
    $hotfixIds = @(Get-CimInstance Win32_QuickFixEngineering | ForEach-Object { [string]$_.HotFixID } | Sort-Object -Unique)
    $enclosure = Get-CimInstance Win32_SystemEnclosure
    $battery = @(Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue)
    $batteryPower = @(Get-CimInstance -Namespace "root\wmi" -ClassName BatteryStatus -ErrorAction SilentlyContinue)
    $memory = @(Get-CimInstance Win32_PhysicalMemory | Sort-Object BankLabel, DeviceLocator)
    $video = @(Get-CimInstance Win32_VideoController | Sort-Object PNPDeviceID, Name)
    $powerScheme = (& powercfg.exe /GetActiveScheme 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw "Cannot capture the active power scheme: $powerScheme" }
    $powerGuid = if ($powerScheme -match '([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})') {
        $Matches[1].ToLowerInvariant()
    } else {
        ""
    }
    $appliedDpi = $null
    try {
        $appliedDpi = [int](Get-ItemProperty -Path "HKCU:\Control Panel\Desktop\WindowMetrics" -Name AppliedDPI -ErrorAction Stop).AppliedDPI
    } catch {
        $appliedDpi = $null
    }

    $memoryRecords = @($memory | ForEach-Object {
        [ordered]@{
            bank_label = [string]$_.BankLabel
            device_locator = [string]$_.DeviceLocator
            capacity_bytes = [uint64]$_.Capacity
            manufacturer = [string]$_.Manufacturer
            part_number = ([string]$_.PartNumber).Trim()
            serial_number = ([string]$_.SerialNumber).Trim()
            speed_mhz = [int]$_.Speed
        }
    })
    $gpuRecords = @($video | ForEach-Object {
        [ordered]@{
            name = [string]$_.Name
            pnp_device_id = [string]$_.PNPDeviceID
            driver_version = [string]$_.DriverVersion
            adapter_ram_bytes = if ($null -eq $_.AdapterRAM) { $null } else { [uint64]$_.AdapterRAM }
            video_processor = [string]$_.VideoProcessor
            status = [string]$_.Status
            availability = if ($null -eq $_.Availability) { $null } else { [int]$_.Availability }
            current_horizontal_resolution = if ($null -eq $_.CurrentHorizontalResolution) { $null } else { [int]$_.CurrentHorizontalResolution }
            current_vertical_resolution = if ($null -eq $_.CurrentVerticalResolution) { $null } else { [int]$_.CurrentVerticalResolution }
            current_refresh_rate_hz = if ($null -eq $_.CurrentRefreshRate) { $null } else { [int]$_.CurrentRefreshRate }
        }
    })
    $batteryRecords = @($battery | ForEach-Object {
        [ordered]@{
            device_id = [string]$_.DeviceID
            pnp_device_id = [string]$_.PNPDeviceID
            status = [string]$_.Status
            battery_status = if ($null -eq $_.BatteryStatus) { $null } else { [int]$_.BatteryStatus }
            design_capacity_mwh = if ($null -eq $_.DesignCapacity) { $null } else { [uint64]$_.DesignCapacity }
        }
    })

    $batteryPowerRecords = @($batteryPower | ForEach-Object {
        [ordered]@{
            instance_name = [string]$_.InstanceName
            power_online = [bool]$_.PowerOnline
        }
    })
    $onAcPower = $batteryRecords.Count -gt 0 -and $batteryPowerRecords.Count -gt 0 -and
        @($batteryPowerRecords | Where-Object { -not $_.power_online }).Count -eq 0
    return [ordered]@{
        computer_system = [ordered]@{
            manufacturer = [string]$computer.Manufacturer
            model = [string]$computer.Model
            system_sku = [string]$computer.SystemSKUNumber
            pc_system_type = [int]$computer.PCSystemType
            total_physical_memory_bytes = [uint64]$computer.TotalPhysicalMemory
        }
        system_product = [ordered]@{
            name = [string]$product.Name
            version = [string]$product.Version
            uuid = [string]$product.UUID
            identifying_number = [string]$product.IdentifyingNumber
        }
        bios = [ordered]@{
            manufacturer = [string]$bios.Manufacturer
            name = [string]$bios.Name
            smbios_bios_version = [string]$bios.SMBIOSBIOSVersion
            serial_number = [string]$bios.SerialNumber
            release_date = if ($null -eq $bios.ReleaseDate) { $null } else { ([DateTime]$bios.ReleaseDate).ToUniversalTime().ToString("o") }
        }
        baseboard = [ordered]@{
            manufacturer = [string]$baseboard.Manufacturer
            product = [string]$baseboard.Product
            version = [string]$baseboard.Version
            serial_number = [string]$baseboard.SerialNumber
        }
        enclosure = [ordered]@{
            chassis_types = @($enclosure.ChassisTypes | ForEach-Object { [int]$_ })
            serial_number = [string]$enclosure.SerialNumber
        }
        cpu = [ordered]@{
            name = ([string]$cpu.Name).Trim()
            processor_id = [string]$cpu.ProcessorId
            architecture = [int]$cpu.Architecture
            physical_cores = [int]$cpu.NumberOfCores
            logical_processors = [int]$cpu.NumberOfLogicalProcessors
        }
        memory_modules = $memoryRecords
        gpus = $gpuRecords
        os = [ordered]@{
            caption = [string]$os.Caption
            version = [string]$os.Version
            build_number = [int]$os.BuildNumber
            update_build_revision = [int]$windowsVersion.UBR
            display_version = [string]$windowsVersion.DisplayVersion
            hotfix_ids = $hotfixIds
            os_architecture = [string]$os.OSArchitecture
            install_date = if ($null -eq $os.InstallDate) { $null } else { ([DateTime]$os.InstallDate).ToUniversalTime().ToString("o") }
        }
        batteries = $batteryRecords
        battery_power = $batteryPowerRecords
        display = [ordered]@{
            applied_dpi = $appliedDpi
            modes = @($gpuRecords | ForEach-Object {
                [ordered]@{
                    gpu_name = $_.name
                    width = $_.current_horizontal_resolution
                    height = $_.current_vertical_resolution
                    refresh_rate_hz = $_.current_refresh_rate_hz
                }
            })
        }
        gate_state = [ordered]@{
            ac_power = $onAcPower
            active_power_scheme_guid = $powerGuid
            active_power_scheme = $powerScheme
        }
    }
}

function Get-QualificationFailures([object]$Snapshot, [object]$Attestation) {
    $failures = [Collections.Generic.List[string]]::new()
    $chassis = @($Snapshot.enclosure.chassis_types)
    if ($Snapshot.computer_system.pc_system_type -ne 2 -or $Snapshot.batteries.Count -eq 0 -or
        @($chassis | Where-Object { $_ -in @(8, 9, 10, 14) }).Count -eq 0) {
        $failures.Add("The machine is not objectively identified as a notebook with a battery and portable/laptop/notebook/subnotebook chassis.")
    }
    if ($Attestation.release_year -lt 2023 -or $Attestation.release_year -gt 2026 -or [string]::IsNullOrWhiteSpace($Attestation.retail_model_evidence)) {
        $failures.Add("Retail model evidence and a release year from 2023 through 2026 are required.")
    }
    if (-not $Snapshot.os.caption.Contains("Windows 11") -or $Snapshot.os.build_number -lt 22631 -or -not $Attestation.fully_patched_confirmed) {
        $failures.Add("Windows 11 build 22631 (23H2) or later and an explicit fully-patched confirmation are required.")
    }
    if ($Snapshot.cpu.architecture -ne 9 -or $Snapshot.cpu.physical_cores -lt 4 -or
        $Attestation.nominal_cpu_power_w -lt 15 -or $Attestation.nominal_cpu_power_w -gt 30) {
        $failures.Add("An x86-64 mobile CPU with at least four physical cores and a documented 15-30 W nominal class is required.")
    }
    $selectedGpu = @($Snapshot.gpus | Where-Object { $_.name -eq $Attestation.integrated_gpu_name })
    $operationalGpus = @($Snapshot.gpus | Where-Object { $_.status -eq "OK" })
    if ([string]::IsNullOrWhiteSpace($Attestation.integrated_gpu_name) -or $selectedGpu.Count -ne 1 -or
        $selectedGpu[0].status -ne "OK" -or $operationalGpus.Count -ne 1 -or
        $operationalGpus[0].name -ne $Attestation.integrated_gpu_name -or
        [string]::IsNullOrWhiteSpace($selectedGpu[0].driver_version) -or -not $Attestation.direct3d_12_confirmed -or
        -not $Attestation.discrete_gpu_disabled_confirmed -or -not $Attestation.production_driver_confirmed) {
        $failures.Add("Select exactly one operational integrated GPU and confirm Direct3D 12 support, the production driver, and that every discrete GPU is objectively disabled rather than merely idle.")
    }
    $ramGiB = [double]$Snapshot.computer_system.total_physical_memory_bytes / 1GB
    if ($ramGiB -lt 15.5 -or $ramGiB -gt 16.5 -or
        $Attestation.shared_gpu_budget_gib -le 0 -or $Attestation.shared_gpu_budget_gib -gt 4) {
        $failures.Add("The machine must have the frozen 16 GiB system-memory configuration and an integrated-GPU shared budget of at most 4 GiB.")
    }
    $matchingModes = @($Snapshot.display.modes | Where-Object {
        $_.gpu_name -eq $Attestation.integrated_gpu_name -and $_.width -eq 1920 -and
        $_.height -eq 1080 -and $_.refresh_rate_hz -eq 60
    })
    if ($matchingModes.Count -eq 0 -or $Snapshot.display.applied_dpi -ne 96) {
        $failures.Add("The active display must be exactly 1920x1080 at 60 Hz and 100 percent scale (96 DPI).")
    }
    if (-not $Snapshot.gate_state.ac_power -or -not $Attestation.vendor_balanced_profile_confirmed) {
        $failures.Add("AC power and explicit confirmation of the vendor balanced profile are required.")
    }
    if (-not $Attestation.background_state_confirmed) {
        $failures.Add("Confirm no pending OS update, active build, debugger, profiler, or overlapping formal measurement before capture.")
    }
    return @($failures)
}

if ($VerifyAttemptSealing) {
    if (-not (Test-Path $occtManifestPath -PathType Leaf)) { throw "Missing frozen OCCT build manifest: $occtManifestPath" }
    $selfTestOcctManifest = Get-Content $occtManifestPath -Raw | ConvertFrom-Json
    $selfTestBuildProvenance = Get-BuildProvenance $repoRoot $selfTestOcctManifest
    Invoke-AttemptSealingSelfTest
    Write-Output "Gate C portable build-provenance self-test passed: $($selfTestBuildProvenance.build_inputs.tree_sha256)"
    exit 0
}

if (-not (Test-Path $lockPath -PathType Leaf)) { throw "Missing active r0-v13 lock: $lockPath" }
if ((Get-LowerSha256 $lockPath) -ne $expectedLockSha256) { throw "Active r0-v13 lock hash mismatch." }
foreach ($entry in $expectedSourceHashes.GetEnumerator()) {
    $path = Join-Path $repoRoot $entry.Key
    if (-not (Test-Path $path -PathType Leaf) -or (Get-LowerSha256 $path) -ne $entry.Value) {
        throw "Frozen Gate C measurement source hash mismatch: $($entry.Key)"
    }
}
$validator = Join-Path $repoRoot "scripts\windows\validate-r0-v13-preregistration.ps1"
& $validator
if ($LASTEXITCODE -ne 0) { throw "R0 v13 preregistration validation failed." }
if (-not (Test-Path $occtManifestPath -PathType Leaf)) { throw "Missing frozen OCCT build manifest: $occtManifestPath" }
$occtManifest = Get-Content $occtManifestPath -Raw | ConvertFrom-Json
foreach ($library in @($occtManifest.shared_libraries)) {
    $path = Join-Path $occtInstallPath ([string]$library.path)
    if (-not (Test-Path $path -PathType Leaf) -or (Get-LowerSha256 $path) -ne [string]$library.sha256) {
        throw "Frozen OCCT runtime hash mismatch: $($library.path)"
    }
}
$buildProvenance = Get-BuildProvenance $repoRoot $occtManifest

$existingFingerprint = $null
if (Test-Path $fingerprintPath -PathType Leaf) {
    $existingFingerprint = Get-Content $fingerprintPath -Raw | ConvertFrom-Json
    if ($existingFingerprint.profile_id -ne "HP-IGPU-01" -or
        $existingFingerprint.freeze_id -ne "r0-v13" -or
        $existingFingerprint.r0_lock_sha256 -ne $expectedLockSha256 -or
        $existingFingerprint.qualification_decision -ne "PASS") {
        throw "The existing HP-IGPU-01 fingerprint is not valid for r0-v13. Historical evidence will not be overwritten."
    }
    $attestation = $existingFingerprint.operator_attestation
} else {
    $attestation = [ordered]@{
        retail_model_evidence = $RetailModelEvidence
        release_year = $ReleaseYear
        nominal_cpu_power_w = $NominalCpuPowerW
        integrated_gpu_name = $IntegratedGpuName
        shared_gpu_budget_gib = $SharedGpuBudgetGiB
        direct3d_12_confirmed = [bool]$Direct3D12Confirmed
        fully_patched_confirmed = [bool]$FullyPatchedConfirmed
        discrete_gpu_disabled_confirmed = [bool]$DiscreteGpuDisabledConfirmed
        vendor_balanced_profile_confirmed = [bool]$VendorBalancedProfileConfirmed
        production_driver_confirmed = [bool]$ProductionDriverConfirmed
        background_state_confirmed = [bool]$BackgroundStateConfirmed
    }
}

$snapshot = Get-MachineSnapshot
$failures = @(Get-QualificationFailures $snapshot $attestation)
if ($failures.Count -ne 0) {
    throw ("HP-IGPU-01 qualification failed:`n- " + ($failures -join "`n- "))
}
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
        battery_identity = @($snapshot.batteries | ForEach-Object {
            [ordered]@{
                device_id = $_.device_id
                pnp_device_id = $_.pnp_device_id
                design_capacity_mwh = $_.design_capacity_mwh
            }
        })
        display = $snapshot.display
        gate_state = [ordered]@{
            ac_power = $snapshot.gate_state.ac_power
            active_power_scheme_guid = $snapshot.gate_state.active_power_scheme_guid
        }
    }
    operator_attestation = $attestation
}
$configurationJson = $configuration | ConvertTo-Json -Depth 12 -Compress
$configurationSha256 = Get-TextSha256 $configurationJson
$scriptSha256 = Get-LowerSha256 $PSCommandPath

if ($null -eq $existingFingerprint) {
    New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null
    $fingerprint = [ordered]@{
        schema_version = 1
        profile_id = "HP-IGPU-01"
        freeze_id = "r0-v13"
        r0_lock_sha256 = $expectedLockSha256
        captured_utc = [DateTime]::UtcNow.ToString("o")
        qualification_decision = "PASS"
        selection_rule = "The first available machine satisfying every frozen requirement becomes the exact HP-IGPU-01 fingerprint."
        machine_configuration_sha256 = $configurationSha256
        runner_script_sha256 = $scriptSha256
        operator_attestation = $attestation
        machine = $snapshot
        frozen_source_sha256 = $expectedSourceHashes
        build_provenance = $buildProvenance
    }
    Write-Utf8JsonExclusive $fingerprintPath $fingerprint
    Write-Output "HP-IGPU-01 qualification PASS; immutable pre-observation fingerprint written to $fingerprintPath"
} else {
    if ($existingFingerprint.machine_configuration_sha256 -ne $configurationSha256) {
        throw "The current machine or gate configuration differs from the frozen HP-IGPU-01 fingerprint. Substitution is forbidden."
    }
    if ($existingFingerprint.runner_script_sha256 -ne $scriptSha256) {
        throw "The execution script differs from the version recorded before the first observation."
    }
    if ($existingFingerprint.build_provenance.build_inputs.tree_sha256 -ne $buildProvenance.build_inputs.tree_sha256 -or
        $existingFingerprint.build_provenance.occt_install_tree.sha256 -ne $buildProvenance.occt_install_tree.sha256) {
        throw "The current portable build provenance differs from the pre-observation fingerprint."
    }
    Write-Output "Existing HP-IGPU-01 fingerprint matches the current machine and gate configuration."
}

if (-not $RunFormalMeasurements) {
    Write-Output "Qualification-only mode completed; no Gate C measurement was started."
    exit 0
}
if (-not $BackgroundStateConfirmed) {
    throw "Formal measurement requires a fresh -BackgroundStateConfirmed attestation for this invocation."
}
if (Test-Path $runManifestPath) { throw "The immutable HP-IGPU-01 run manifest already exists." }

$coreOutputs = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-core-r0-v13-series-$_.json" })
$navOutputs = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-nav-r0-v13-series-$_.json" })
$buildStdout = Join-Path $artifactDir "hp-igpu-01-r0-v13-build.stdout.log"
$buildStderr = Join-Path $artifactDir "hp-igpu-01-r0-v13-build.stderr.log"
$measurementTargetDir = Join-Path $repoRoot "target\gate-c-r0-v13-hp-igpu-01"
$coreStdout = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-core-r0-v13-series-$_.stdout.log" })
$coreStderr = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-core-r0-v13-series-$_.stderr.log" })
$navStdout = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-nav-r0-v13-series-$_.stdout.log" })
$navStderr = @(1..3 | ForEach-Object { Join-Path $artifactDir "hp-igpu-01-nav-r0-v13-series-$_.stderr.log" })
$allFormalPaths = @($runManifestPath, $attemptClaimPath, $buildStdout, $buildStderr) +
    @($coreOutputs) + @($navOutputs) + @($coreStdout) + @($coreStderr) + @($navStdout) + @($navStderr)
foreach ($path in $allFormalPaths) {
    if (Test-Path $path) { throw "Formal evidence already exists and will not be overwritten: $path" }
}
if (Test-Path $measurementTargetDir) {
    throw "The clean formal-build target already exists and will not be reused: $measurementTargetDir"
}

$attemptStartedUtc = [DateTime]::UtcNow.ToString("o")
$attemptClaim = [ordered]@{
    schema_version = 1
    profile_id = "HP-IGPU-01"
    freeze_id = "r0-v13"
    r0_lock_sha256 = $expectedLockSha256
    fingerprint_sha256 = Get-LowerSha256 $fingerprintPath
    runner_script_sha256 = $scriptSha256
    build_input_tree_sha256 = $buildProvenance.build_inputs.tree_sha256
    occt_install_tree_sha256 = $buildProvenance.occt_install_tree.sha256
    started_utc = $attemptStartedUtc
}
Write-Utf8JsonExclusive $attemptClaimPath $attemptClaim
$stageResults = [Collections.Generic.List[object]]::new()
$terminalDecision = "INFRASTRUCTURE_INVALID"
$failingStage = "runner"
$failureMessage = "The formal attempt did not reach a terminal measurement decision."
$attemptError = $null
$actualExecutableHashes = [ordered]@{
    gate_c_core = $null
    exact_worker = $null
    gate_c_nav = $null
}
$coreExe = Join-Path $measurementTargetDir "release\ketchup-gate-c-core.exe"
$workerExe = Join-Path $measurementTargetDir "release\ketchup-exact-worker.exe"
$navExe = Join-Path $measurementTargetDir "release\ketchup-gate-c-nav.exe"
$oldWgpuBackend = $env:WGPU_BACKEND
$oldCargoTargetDir = $env:CARGO_TARGET_DIR
$oldRustc = $env:RUSTC
$oldPath = $env:PATH
$rustBin = Join-Path $env:USERPROFILE ".rustup\toolchains\1.97.0-x86_64-pc-windows-msvc\bin"
$msvcBin = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.35.32215\bin\HostX64\x64"

Push-Location $repoRoot
try {
    $env:CARGO_TARGET_DIR = $measurementTargetDir
    $env:RUSTC = Join-Path $rustBin "rustc.exe"
    $env:PATH = $rustBin + ";" + $msvcBin + ";" + $oldPath
    $buildStage = Invoke-RecordedStage "release-build" (Join-Path $rustBin "cargo.exe") @(
        "build", "--locked", "--release", "--bin", "ketchup-gate-c-core", "--bin",
        "ketchup-exact-worker", "--bin", "ketchup-gate-c-nav"
    ) $buildStdout $buildStderr "" 0 $expectedLockSha256 $repoRoot
    $stageResults.Add($buildStage)
    if ($buildStage.decision -ne "PASS") {
        $terminalDecision = $buildStage.decision
        $failingStage = $buildStage.stage_id
        $failureMessage = "Release measurement build failed."
        throw $failureMessage
    }

    $failingStage = "executable-verification"
    foreach ($path in @($coreExe, $workerExe, $navExe)) {
        if (-not (Test-Path $path -PathType Leaf)) { throw "Missing release measurement executable: $path" }
        $fileName = [IO.Path]::GetFileName($path)
        $actualHash = Get-LowerSha256 $path
        switch ($fileName) {
            "ketchup-gate-c-core.exe" { $actualExecutableHashes.gate_c_core = $actualHash }
            "ketchup-exact-worker.exe" { $actualExecutableHashes.exact_worker = $actualHash }
            "ketchup-gate-c-nav.exe" { $actualExecutableHashes.gate_c_nav = $actualHash }
        }
        if ((Get-Item $path).Length -le 0) {
            throw "Release measurement executable is empty: $path"
        }
    }

    foreach ($series in 1..3) {
        $stage = Invoke-RecordedStage "core-series-$series" $coreExe @(
            "HP-IGPU-01", $series, $expectedLockSha256, $workerExe, $coreOutputs[$series - 1]
        ) $coreStdout[$series - 1] $coreStderr[$series - 1] $coreOutputs[$series - 1] `
            $series $expectedLockSha256 $repoRoot
        $stageResults.Add($stage)
        if ($stage.decision -ne "PASS") {
            $terminalDecision = $stage.decision
            $failingStage = $stage.stage_id
            $failureMessage = "HP-IGPU-01 core series $series did not pass."
            throw $failureMessage
        }
    }

    $env:WGPU_BACKEND = "dx12"
    foreach ($series in 1..3) {
        $stage = Invoke-RecordedStage "navigation-series-$series" $navExe @(
            "HP-IGPU-01", $series, $expectedLockSha256, $attestation.integrated_gpu_name,
            $navOutputs[$series - 1]
        ) $navStdout[$series - 1] $navStderr[$series - 1] $navOutputs[$series - 1] `
            $series $expectedLockSha256 $repoRoot
        $stageResults.Add($stage)
        if ($stage.decision -ne "PASS") {
            $terminalDecision = $stage.decision
            $failingStage = $stage.stage_id
            $failureMessage = "HP-IGPU-01 navigation series $series did not pass."
            throw $failureMessage
        }
    }

    $terminalDecision = "PASS"
    $failingStage = ""
    $failureMessage = ""
} catch {
    $attemptError = $_
    $failureMessage = $_.Exception.Message
} finally {
    $env:WGPU_BACKEND = $oldWgpuBackend
    $env:CARGO_TARGET_DIR = $oldCargoTargetDir
    $env:RUSTC = $oldRustc
    $env:PATH = $oldPath
    Pop-Location
    $evidencePaths = @($attemptClaimPath, $buildStdout, $buildStderr) + @($coreOutputs) + @($navOutputs) +
        @($coreStdout) + @($coreStderr) + @($navStdout) + @($navStderr)
    $runManifest = New-AttemptManifest $terminalDecision $failingStage $failureMessage `
        $attemptStartedUtc $fingerprintPath "artifacts/gate-c/hp-igpu-01-fingerprint-r0-v13.json" `
        $scriptSha256 $buildProvenance $actualExecutableHashes @($stageResults) $evidencePaths $repoRoot $expectedLockSha256
    Write-Utf8JsonExclusive $runManifestPath $runManifest
}

if ($null -ne $attemptError) {
    throw "HP-IGPU-01 formal attempt ended as $terminalDecision at $failingStage; immutable manifest written to $runManifestPath"
}
Write-Output "All three HP-IGPU-01 core and navigation series passed; immutable run manifest written to $runManifestPath"
