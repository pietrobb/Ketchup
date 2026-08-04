[CmdletBinding()]
param([string]$RunId = "strengthened-v2-run-001")

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$artifactRoot = Join-Path $repoRoot "artifacts\gate-a0"
$runDir = Join-Path $artifactRoot ("runs\" + $RunId)
$logDir = Join-Path $runDir "process-logs"
$fixtureDir = Join-Path $runDir "fixtures"
$metricsDir = Join-Path $runDir "inherited-suites"
$inputDir = Join-Path $runDir "inputs"
$validator = Join-Path $PSScriptRoot "validate-strengthened-a0-v2.ps1"
$preregistrationPath = Join-Path $artifactRoot "strengthened-a0-v2-preregistration.json"
$lockPath = Join-Path $artifactRoot "strengthened-a0-v2-lock.json"
$probeSource = Join-Path $repoRoot "crates\ketchup-exact\src\bin\ketchup-a0-diagnostic-probe.rs"
$testSource = Join-Path $repoRoot "crates\ketchup-exact\tests\gate_a0_v2.rs"
$runnerSource = $PSCommandPath
$script:processRecords = @()
$script:nativeObservationReached = $false
$script:provenanceFailure = $false
$script:substantiveFailureObserved = $false
$script:cleanupBarrierPassed = $true
$suites = @()
$matrix = @()
$preflight = $null
$decision = "NO-GO"
$failureClass = "hash_or_provenance_only"
$disposition = "No geometry conclusion. Repair provenance or harness inputs and issue a new preregistration before another formal observation."
$fatalDetail = $null

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}

function Write-Json([string]$Path, [object]$Value) {
    Write-Utf8 $Path (($Value | ConvertTo-Json -Depth 16) + [Environment]::NewLine)
}

function Get-RunRelativePath([string]$Path) {
    return ([IO.Path]::GetFullPath($Path)).Substring($runDir.TrimEnd("\").Length + 1).Replace("\", "/")
}

function ConvertTo-QuotedArgument([string]$Value) {
    return '"' + $Value.Replace('\', '\').Replace('"', '\"') + '"'
}

function Initialize-KillOnCloseJobType {
    if (([System.Management.Automation.PSTypeName]"KetchupKillOnCloseJob").Type) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;

public sealed class KetchupKillOnCloseJob : IDisposable
{
    private const UInt32 JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
    private const Int32 JobObjectBasicAccountingInformation = 1;
    private const Int32 JobObjectExtendedLimitInformation = 9;
    private IntPtr handle;

    [StructLayout(LayoutKind.Sequential)]
    private struct JOBOBJECT_BASIC_LIMIT_INFORMATION
    {
        public Int64 PerProcessUserTimeLimit;
        public Int64 PerJobUserTimeLimit;
        public UInt32 LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public UInt32 ActiveProcessLimit;
        public UIntPtr Affinity;
        public UInt32 PriorityClass;
        public UInt32 SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
    {
        public Int64 TotalUserTime;
        public Int64 TotalKernelTime;
        public Int64 ThisPeriodTotalUserTime;
        public Int64 ThisPeriodTotalKernelTime;
        public UInt32 TotalPageFaultCount;
        public UInt32 TotalProcesses;
        public UInt32 ActiveProcesses;
        public UInt32 TotalTerminatedProcesses;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct IO_COUNTERS
    {
        public UInt64 ReadOperationCount;
        public UInt64 WriteOperationCount;
        public UInt64 OtherOperationCount;
        public UInt64 ReadTransferCount;
        public UInt64 WriteTransferCount;
        public UInt64 OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
    {
        public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
        public IO_COUNTERS IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateJobObject(IntPtr securityAttributes, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool SetInformationJobObject(IntPtr job, Int32 infoClass, IntPtr info, UInt32 length);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool QueryInformationJobObject(IntPtr job, Int32 infoClass, IntPtr info, UInt32 length, out UInt32 returnedLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    public KetchupKillOnCloseJob()
    {
        handle = CreateJobObject(IntPtr.Zero, null);
        if (handle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        var limits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        int length = Marshal.SizeOf(limits);
        IntPtr buffer = Marshal.AllocHGlobal(length);
        try
        {
            Marshal.StructureToPtr(limits, buffer, false);
            if (!SetInformationJobObject(handle, JobObjectExtendedLimitInformation, buffer, (UInt32)length))
                throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        catch
        {
            CloseHandle(handle);
            handle = IntPtr.Zero;
            throw;
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
    }

    public void Assign(Process process)
    {
        if (!AssignProcessToJobObject(handle, process.Handle))
            throw new Win32Exception(Marshal.GetLastWin32Error());
    }

    public bool WaitForEmpty(Int32 timeoutMilliseconds)
    {
        var stopwatch = Stopwatch.StartNew();
        int length = Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION));
        IntPtr buffer = Marshal.AllocHGlobal(length);
        try
        {
            while (true)
            {
                UInt32 returnedLength;
                if (!QueryInformationJobObject(handle, JobObjectBasicAccountingInformation, buffer, (UInt32)length, out returnedLength))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                var accounting = (JOBOBJECT_BASIC_ACCOUNTING_INFORMATION)Marshal.PtrToStructure(buffer, typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION));
                if (accounting.ActiveProcesses == 0) return true;
                if (stopwatch.ElapsedMilliseconds >= timeoutMilliseconds) return false;
                System.Threading.Thread.Sleep(25);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
    }

    public void Dispose()
    {
        if (handle != IntPtr.Zero)
        {
            if (!CloseHandle(handle)) throw new Win32Exception(Marshal.GetLastWin32Error());
            handle = IntPtr.Zero;
        }
    }
}
'@
}

function Invoke-CapturedProcess(
    [string]$Id,
    [string]$Stage,
    [string]$FilePath,
    [string[]]$Arguments,
    [hashtable]$Environment,
    [object]$BuildIdentity,
    [bool]$NativeObservation,
    [string]$RuntimeRoot,
    [int]$TimeoutSeconds
) {
    $stdoutPath = Join-Path $logDir ($Id + ".stdout.txt")
    $stderrPath = Join-Path $logDir ($Id + ".stderr.txt")
    $startedUtc = [DateTime]::UtcNow.ToString("o")
    $stdout = ""
    $stderr = ""
    $exitCode = $null
    $state = "failed_to_start"
    $process = $null
    $job = $null
    $jobAssigned = $false
    $stdoutTask = $null
    $stderrTask = $null
    $runtimeLibraries = @()
    $executableSha256 = if (Test-Path $FilePath -PathType Leaf) {
        (Get-FileHash $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    } else { $null }
    try {
        if ($NativeObservation) {
            if ([string]::IsNullOrWhiteSpace($RuntimeRoot)) { throw "Native launch lacks a runtime-library root." }
            $sourceRuntimeRoot = Join-Path ([string]$BuildIdentity.install_path) "win64\vc14\bin"
            $sourceDlls = @(Get-ChildItem $sourceRuntimeRoot -Filter "*.dll" -File | Sort-Object Name)
            $stagedDlls = @(Get-ChildItem $RuntimeRoot -Filter "*.dll" -File | Sort-Object Name)
            if ($sourceDlls.Count -eq 0 -or $stagedDlls.Count -ne $sourceDlls.Count -or
                (($stagedDlls.Name | Sort-Object) -join "|") -ne (($sourceDlls.Name | Sort-Object) -join "|")) {
                throw "Staged runtime DLL set does not exactly match the frozen backend."
            }
            foreach ($sourceDll in $sourceDlls) {
                $runtimePath = Join-Path $RuntimeRoot $sourceDll.Name
                $expected = (Get-FileHash $sourceDll.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                $actual = (Get-FileHash $runtimePath -Algorithm SHA256).Hash.ToLowerInvariant()
                $runtimeLibraries += [ordered]@{ name = $sourceDll.Name; path = $runtimePath; source_path = $sourceDll.FullName; expected_sha256 = $expected; actual_sha256 = $actual }
                if ($actual -ne $expected) { throw "Staged runtime library changed immediately before launch: $runtimePath" }
            }
        }
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $FilePath
        $startInfo.Arguments = (@($Arguments | ForEach-Object { ConvertTo-QuotedArgument $_ }) -join " ")
        $startInfo.WorkingDirectory = $repoRoot
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        foreach ($name in $Environment.Keys) {
            $startInfo.EnvironmentVariables[$name] = [string]$Environment[$name]
        }
        $job = [KetchupKillOnCloseJob]::new()
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw "Process did not start." }
        $state = "ran"
        $job.Assign($process)
        $jobAssigned = $true
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            throw "Process timed out after $TimeoutSeconds seconds."
        }
        $stdout = $stdoutTask.Result
        $stderr = $stderrTask.Result
        $exitCode = $process.ExitCode
    } catch {
        if ($NativeObservation -and $state -ne "ran") { $script:provenanceFailure = $true }
        $stderr = $_.Exception.ToString()
    } finally {
        try {
            if ($state -eq "ran" -and -not $jobAssigned) { $script:cleanupBarrierPassed = $false }
            if ($null -ne $process) {
                if (-not $process.HasExited) {
                    if ($jobAssigned -and $null -ne $job) {
                        $job.Dispose()
                        $job = $null
                    } else {
                        $process.Kill()
                    }
                    if (-not $process.WaitForExit(10000)) { $script:cleanupBarrierPassed = $false }
                }
                if (-not $process.HasExited) { $script:cleanupBarrierPassed = $false }
                if ($jobAssigned -and $null -ne $job -and -not $job.WaitForEmpty(10000)) { $script:cleanupBarrierPassed = $false }
                if ($null -ne $stdoutTask -and -not $stdoutTask.Wait(10000)) { $script:cleanupBarrierPassed = $false }
                if ($null -ne $stderrTask -and -not $stderrTask.Wait(10000)) { $script:cleanupBarrierPassed = $false }
                if ($null -ne $stdoutTask -and $stdoutTask.IsCompleted) { $stdout = $stdoutTask.Result }
                if ($null -ne $stderrTask -and $stderrTask.IsCompleted) {
                    $capturedStderr = $stderrTask.Result
                    if (-not [string]::IsNullOrEmpty($capturedStderr)) {
                        $stderr = if ([string]::IsNullOrEmpty($stderr)) { $capturedStderr } else { $capturedStderr + [Environment]::NewLine + $stderr }
                    }
                }
                if ($state -eq "ran" -and $process.HasExited) { $exitCode = $process.ExitCode }
            }
            if ($null -ne $job) { $job.Dispose() }
        } catch {
            $script:cleanupBarrierPassed = $false
            $stderr = $stderr + [Environment]::NewLine + $_.Exception.ToString()
        } finally {
            if ($null -ne $process) { $process.Dispose() }
        }
    }
    Write-Utf8 $stdoutPath $stdout
    Write-Utf8 $stderrPath $stderr
    $nativeObservationObserved = $NativeObservation -and $stdout -match "(?m)^native_observation=entered\s*$"
    if ($NativeObservation) {
        if ($nativeObservationObserved) {
            $script:nativeObservationReached = $true
            if ($exitCode -ne 0) { $script:substantiveFailureObserved = $true }
        } elseif ($state -eq "ran") {
            $script:provenanceFailure = $true
        }
    }
    $record = [ordered]@{
        id = $Id
        stage = $Stage
        state = $state
        started_utc = $startedUtc
        ended_utc = [DateTime]::UtcNow.ToString("o")
        command = $FilePath
        arguments = @($Arguments)
        build_identity = $BuildIdentity
        executable_sha256 = $executableSha256
        runtime_root = $RuntimeRoot
        runtime_libraries = $runtimeLibraries
        native_observation = $nativeObservationObserved
        exit_code = $exitCode
        success = ($state -eq "ran" -and $exitCode -eq 0)
        panic_detected = ($stderr -match "(?im)panicked at|thread '.+' panicked|fatal runtime error")
        stdout_path = Get-RunRelativePath $stdoutPath
        stderr_path = Get-RunRelativePath $stderrPath
        stdout_sha256 = (Get-FileHash $stdoutPath -Algorithm SHA256).Hash.ToLowerInvariant()
        stderr_sha256 = (Get-FileHash $stderrPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $script:processRecords += $record
    return $record
}

function Add-NotRunProcess(
    [string]$Id,
    [string]$Stage,
    [string]$Reason,
    [object]$BuildIdentity
) {
    $stdoutPath = Join-Path $logDir ($Id + ".stdout.txt")
    $stderrPath = Join-Path $logDir ($Id + ".stderr.txt")
    Write-Utf8 $stdoutPath ""
    Write-Utf8 $stderrPath ""
    $record = [ordered]@{
        id = $Id
        stage = $Stage
        state = "not_run"
        reason = $Reason
        started_utc = $null
        ended_utc = $null
        command = $null
        arguments = @()
        build_identity = $BuildIdentity
        executable_sha256 = $null
        native_observation = $false
        exit_code = $null
        success = $false
        panic_detected = $false
        stdout_path = Get-RunRelativePath $stdoutPath
        stderr_path = Get-RunRelativePath $stderrPath
        stdout_sha256 = (Get-FileHash $stdoutPath -Algorithm SHA256).Hash.ToLowerInvariant()
        stderr_sha256 = (Get-FileHash $stderrPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $script:processRecords += $record
    return $record
}

function Test-RuntimeLibraries([string]$RuntimeRoot, [object]$Backend) {
    foreach ($library in @($Backend.representative_libraries)) {
        $runtimePath = Join-Path $RuntimeRoot (Split-Path ([string]$library.path) -Leaf)
        if (-not (Test-Path $runtimePath -PathType Leaf)) {
            return "missing staged runtime library: $runtimePath"
        }
        $actual = (Get-FileHash $runtimePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne [string]$library.sha256) {
            return "staged runtime library mismatch: $runtimePath"
        }
    }
    return $null
}

function Get-NegativeControlResult([string]$Stdout) {
    $match = [regex]::Match($Stdout, "(?m)^retired_reference=(lost|quarantined)\s*$")
    if (-not $match.Success) { return $null }
    return [string]$match.Groups[1].Value
}

function Seal-Run {
    $sealedFiles = @(Get-ChildItem $runDir -Recurse -File |
        Where-Object { $_.Name -notin @("seal.json", "seal.sha256") } |
        Sort-Object FullName |
        ForEach-Object {
            [ordered]@{
                path = Get-RunRelativePath $_.FullName
                size = $_.Length
                sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        })
    $seal = [ordered]@{
        schema_version = 1
        freeze_id = "strengthened-a0-v2"
        run_id = $RunId
        sealed_utc = [DateTime]::UtcNow.ToString("o")
        hash_algorithm = "SHA-256"
        file_count = $sealedFiles.Count
        files = $sealedFiles
    }
    $sealPath = Join-Path $runDir "seal.json"
    Write-Json $sealPath $seal
    $sealSha256 = (Get-FileHash $sealPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8 (Join-Path $runDir "seal.sha256") ($sealSha256 + "  seal.json" + [Environment]::NewLine)
    foreach ($entry in $sealedFiles) {
        $path = Join-Path $runDir ([string]$entry.path)
        if ((Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne [string]$entry.sha256) {
            throw "Sealed file hash mismatch: $($entry.path)"
        }
    }
}

if ($RunId -notmatch '^strengthened-v2-run-[0-9]{3}$') { throw "RunId must match strengthened-v2-run-NNN." }
if (Test-Path $runDir) { throw "Strengthened A0 v2 evidence already exists: $RunId" }
New-Item -ItemType Directory -Path $runDir | Out-Null

try {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
    New-Item -ItemType Directory -Path $fixtureDir -Force | Out-Null
    New-Item -ItemType Directory -Path $metricsDir -Force | Out-Null
    New-Item -ItemType Directory -Path $inputDir -Force | Out-Null
    foreach ($source in @($runnerSource, $validator, $preregistrationPath, $lockPath, $probeSource, $testSource)) {
        Copy-Item $source (Join-Path $inputDir (Split-Path $source -Leaf))
    }
    Initialize-KillOnCloseJobType
    $powerShell = (Get-Command powershell.exe -ErrorAction Stop).Source
    $cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
    $preflightRecord = Invoke-CapturedProcess `
        "preflight-validator" `
        "preflight" `
        $powerShell `
        @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $validator, "-EmitJson") `
        @{} `
        ([ordered]@{ freeze_id = "strengthened-a0-v2" }) `
        $false `
        $null `
        180
    if (-not $preflightRecord.success) { throw "Strengthened A0 v2 preflight failed." }
    $preflightStdout = Get-Content (Join-Path $runDir $preflightRecord.stdout_path) -Raw
    $preflight = $preflightStdout | ConvertFrom-Json
    $backendByAlias = @{}
    foreach ($backend in @($preflight.backends)) { $backendByAlias[[string]$backend.alias] = $backend }

    $workRoot = Join-Path $env:TEMP ("ketchup-a0-v2-" + $RunId)
    if (Test-Path $workRoot) { throw "Exclusive A0 v2 work path already exists: $workRoot" }
    New-Item -ItemType Directory -Path $workRoot | Out-Null
    $builds = @{}
    foreach ($alias in @("prior", "current")) {
        $backend = $backendByAlias[$alias]
        $identity = [ordered]@{
            alias = $alias
            id = [string]$backend.id
            install_path = [string]$backend.install_path
            tree_sha256 = [string]$backend.tree_sha256
            fingerprint = [string]$backend.fingerprint
            tkernel_sha256 = [string]$backend.tkernel_sha256
            evaluator_source_sha256 = [string]$preflight.evaluator_source_sha256
            tolerance_profile_sha256 = [string]$preflight.tolerance_profile_sha256
            representative_libraries = @($backend.representative_libraries)
        }
        $targetDir = Join-Path $workRoot ("target-" + $alias)
        $environment = @{
            KETCHUP_OCCT_ROOT = [string]$backend.install_path
            KETCHUP_OCCT_BUILD_FINGERPRINT = [string]$backend.fingerprint
        }
        $suiteBuild = Invoke-CapturedProcess `
            ("build-suite-" + $alias) `
            "build" `
            $cargo `
            @("test", "--locked", "--release", "--manifest-path", (Join-Path $repoRoot "Cargo.toml"), "-p", "ketchup-exact", "--test", "gate_a0_v2", "--no-run", "--target-dir", $targetDir) `
            $environment `
            $identity `
            $false `
            $null `
            1800
        $probeBuild = if ($suiteBuild.success) {
            Invoke-CapturedProcess `
                ("build-probe-" + $alias) `
                "build" `
                $cargo `
                @("build", "--locked", "--release", "--manifest-path", (Join-Path $repoRoot "Cargo.toml"), "-p", "ketchup-exact", "--bin", "ketchup-a0-diagnostic-probe", "--target-dir", $targetDir) `
                $environment `
                $identity `
                $false `
                $null `
                1800
        } else {
            Add-NotRunProcess ("build-probe-" + $alias) "build" "inherited-suite build failed" $identity
        }
        $testExecutables = if ($suiteBuild.success) {
            @(Get-ChildItem (Join-Path $targetDir "release\deps") -Filter "gate_a0_v2-*.exe" -File)
        } else { @() }
        $testExe = if ($testExecutables.Count -eq 1) { $testExecutables[0].FullName } else { $null }
        $probeExe = Join-Path $targetDir "release\ketchup-a0-diagnostic-probe.exe"
        $suiteRuntimeError = if ($null -ne $testExe) { Test-RuntimeLibraries (Join-Path $targetDir "release\deps") $backend } else { "expected exactly one gate_a0_v2 test executable" }
        $probeRuntimeError = if ($probeBuild.success -and (Test-Path $probeExe -PathType Leaf)) { Test-RuntimeLibraries (Join-Path $targetDir "release") $backend } else { "probe build or executable unavailable" }
        $builds[$alias] = [ordered]@{
            identity = $identity
            backend = $backend
            target_dir = $targetDir
            suite_available = ($suiteBuild.success -and $null -ne $testExe -and $null -eq $suiteRuntimeError)
            suite_unavailable_reason = $suiteRuntimeError
            suite_executable = $testExe
            probe_available = ($probeBuild.success -and (Test-Path $probeExe -PathType Leaf) -and $null -eq $probeRuntimeError)
            probe_unavailable_reason = $probeRuntimeError
            probe_executable = $probeExe
        }
        if (-not $builds[$alias].suite_available -or -not $builds[$alias].probe_available) {
            $script:provenanceFailure = $true
        }
    }

    foreach ($alias in @("prior", "current")) {
        $build = $builds[$alias]
        $metricsPath = Join-Path $metricsDir ($alias + ".json")
        if (-not $build.suite_available) {
            $record = Add-NotRunProcess ("suite-" + $alias) "inherited-suite" ([string]$build.suite_unavailable_reason) $build.identity
        } else {
            $record = Invoke-CapturedProcess `
                ("suite-" + $alias) `
                "inherited-suite" `
                $build.suite_executable `
                @("--exact", "gate_a0_v2", "--nocapture") `
                @{
                    KETCHUP_A0_V2_RUN_ID = $RunId
                    KETCHUP_A0_V2_LOCK_SHA256 = [string]$preflight.lock_sha256
                    KETCHUP_A0_V2_METRICS_PATH = $metricsPath
                } `
                $build.identity `
                $true `
                (Join-Path $build.target_dir "release\deps") `
                900
        }
        $metrics = if (Test-Path $metricsPath -PathType Leaf) { Get-Content $metricsPath -Raw | ConvertFrom-Json } else { $null }
        $suitePass = $record.success -and $null -ne $metrics -and [string]$metrics.decision -eq "GO"
        if ([string]$record.state -eq "ran" -and -not $suitePass) { $script:substantiveFailureObserved = $true }
        $suites += [ordered]@{
            alias = $alias
            process_id = [string]$record.id
            state = [string]$record.state
            exit_code = $record.exit_code
            metrics_path = if ($null -ne $metrics) { Get-RunRelativePath $metricsPath } else { $null }
            metrics_decision = if ($null -ne $metrics) { [string]$metrics.decision } else { $null }
            pass = $suitePass
        }
    }

    $combinations = @(
        @("prior", "prior"),
        @("current", "current"),
        @("prior", "current"),
        @("current", "prior")
    )
    foreach ($combination in $combinations) {
        $producerAlias = $combination[0]
        $consumerAlias = $combination[1]
        $id = $producerAlias + "-to-" + $consumerAlias
        $producerBuild = $builds[$producerAlias]
        $consumerBuild = $builds[$consumerAlias]
        $fixturePath = Join-Path $fixtureDir ($id + ".tsv")
        if (-not $producerBuild.probe_available) {
            $producerRecord = Add-NotRunProcess ("producer-" + $id) "producer" ([string]$producerBuild.probe_unavailable_reason) $producerBuild.identity
        } else {
            $producerRecord = Invoke-CapturedProcess `
                ("producer-" + $id) `
                "producer" `
                $producerBuild.probe_executable `
                @("produce", $fixturePath) `
                @{} `
                $producerBuild.identity `
                $true `
                (Join-Path $producerBuild.target_dir "release") `
                120
        }
        if (-not $producerRecord.success) {
            $consumerRecord = Add-NotRunProcess ("consumer-" + $id) "consumer" "producer did not complete successfully" $consumerBuild.identity
        } elseif (-not $consumerBuild.probe_available) {
            $consumerRecord = Add-NotRunProcess ("consumer-" + $id) "consumer" ([string]$consumerBuild.probe_unavailable_reason) $consumerBuild.identity
        } else {
            $consumerRecord = Invoke-CapturedProcess `
                ("consumer-" + $id) `
                "consumer" `
                $consumerBuild.probe_executable `
                @("consume", $fixturePath) `
                @{} `
                $consumerBuild.identity `
                $true `
                (Join-Path $consumerBuild.target_dir "release") `
                120
        }
        $sameIdentity = [string]$producerBuild.identity.fingerprint -eq [string]$consumerBuild.identity.fingerprint
        $consumerStdout = Get-Content (Join-Path $runDir $consumerRecord.stdout_path) -Raw
        $resolvedCount = ([regex]::Matches($consumerStdout, "(?m)^resolved=")).Count
        $wrongIdentityCount = ([regex]::Matches($consumerStdout, "(?m)^active_reference_wrong=")).Count
        $ambiguousCount = ([regex]::Matches($consumerStdout, "(?m)^active_reference_failure=.*Ambiguous")).Count
        $negativeResult = Get-NegativeControlResult $consumerStdout
        $expectedNegative = if ($sameIdentity) { "lost" } else { "quarantined" }
        $negativePass = $negativeResult -eq $expectedNegative
        $combinationPass = $producerRecord.success -and $consumerRecord.success -and $resolvedCount -eq 3 -and $wrongIdentityCount -eq 0 -and $ambiguousCount -eq 0 -and $negativePass
        if (([string]$producerRecord.state -eq "ran" -or [string]$consumerRecord.state -eq "ran") -and -not $combinationPass) {
            $script:substantiveFailureObserved = $true
        }
        $matrix += [ordered]@{
            id = $id
            producer_build = $producerBuild.identity
            consumer_build = $consumerBuild.identity
            same_build_identity = $sameIdentity
            producer_process_id = [string]$producerRecord.id
            producer_state = [string]$producerRecord.state
            producer_exit_code = $producerRecord.exit_code
            consumer_process_id = [string]$consumerRecord.id
            consumer_state = [string]$consumerRecord.state
            consumer_not_run = ([string]$consumerRecord.state -eq "not_run")
            consumer_not_run_reason = if ([string]$consumerRecord.state -eq "not_run") { [string]$consumerRecord.reason } else { $null }
            consumer_exit_code = $consumerRecord.exit_code
            active_references_resolved = $resolvedCount
            active_reference_wrong_count = $wrongIdentityCount
            active_reference_ambiguous_count = $ambiguousCount
            negative_control_expected = $expectedNegative
            negative_control_observed = $negativeResult
            negative_control_pass = $negativePass
            pass = $combinationPass
        }
    }

    $suitePassCount = @($suites | Where-Object { $_.pass }).Count
    $matrixPassCount = @($matrix | Where-Object { $_.pass }).Count
    $sameBuildPassCount = @($matrix | Where-Object { $_.same_build_identity -and $_.pass }).Count
    $negativePassCount = @($matrix | Where-Object { $_.negative_control_pass }).Count
    $unsafeActiveCount = @($matrix | Where-Object { $_.active_reference_wrong_count -ne 0 -or $_.active_reference_ambiguous_count -ne 0 }).Count
    if ($script:provenanceFailure -or -not $script:cleanupBarrierPassed) {
        $decision = "NO-GO"
        $failureClass = "hash_or_provenance_only"
        $disposition = "No complete geometry conclusion. Repair the failed build/runtime provenance or cleanup path and issue a new preregistration before another formal observation."
    } elseif ($suitePassCount -eq 2 -and $matrixPassCount -eq 4 -and $negativePassCount -eq 4) {
        $decision = "FULL_GO"
        $failureClass = "none"
        $disposition = "A0 v2 passes both frozen backends and all four same/cross-build directions. Release M3, withdraw L-01/L-02 from ADR 0004, leave L-03/L-04 unadopted, and keep PF0 inactive."
    } elseif ($suitePassCount -eq 2 -and $sameBuildPassCount -eq 2 -and $negativePassCount -eq 4 -and $unsafeActiveCount -eq 0) {
        $decision = "SAME_BUILD_GO"
        $failureClass = "cross_build_transfer_only"
        $disposition = "Both same-build paths pass while changed-identity transfer does not fully pass. Retain L-01/L-02 and the current version, leave L-03/L-04 unadopted, quarantine changed identities, release M3 on the unchanged passing identity, and keep PF0 inactive."
    } elseif ($script:substantiveFailureObserved) {
        $decision = "NO-GO"
        $failureClass = "substantive_topology_or_reference"
        $disposition = "Apply the safe halt and diagnostic_hold. Do not authorize redesign, PF0, envelope narrowing, or any loosen until a minimal reproducer localizes the cause to a concrete source path or named external boundary."
    } else {
        $decision = "NO-GO"
        $failureClass = "hash_or_provenance_only"
        $disposition = "No complete geometry conclusion. Repair the failed build/runtime provenance path and issue a new preregistration before another formal observation."
    }
} catch {
    $fatalDetail = $_.Exception.ToString()
    if ($script:substantiveFailureObserved -or ($script:nativeObservationReached -and -not $script:provenanceFailure)) {
        $failureClass = "substantive_topology_or_reference"
        $disposition = "Apply the safe halt and diagnostic_hold. The run reached native observation but did not complete; preserve this evidence and localize the cause before any architectural disposition."
    } else {
        $failureClass = "hash_or_provenance_only"
        $disposition = "No complete geometry conclusion. Repair the failed setup/build/runtime provenance path and issue a new preregistration before another formal observation."
    }
} finally {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
    New-Item -ItemType Directory -Path $fixtureDir -Force | Out-Null
    New-Item -ItemType Directory -Path $metricsDir -Force | Out-Null
    New-Item -ItemType Directory -Path $inputDir -Force | Out-Null
    Write-Json (Join-Path $runDir "processes.json") $script:processRecords
    Write-Json (Join-Path $runDir "inherited-suites.json") $suites
    $matrixDocument = [ordered]@{
        schema_version = 1
        freeze_id = "strengthened-a0-v2"
        run_id = $RunId
        required_combinations = 4
        observed_combinations = $matrix.Count
        passed_combinations = @($matrix | Where-Object { $_.pass }).Count
        negative_control = "intentional retired north reference: Lost for same identity, QuarantinedMigration for changed identity"
        combinations = $matrix
    }
    Write-Json (Join-Path $runDir "matrix.json") $matrixDocument
    $summary = [ordered]@{
        schema_version = 1
        freeze_id = "strengthened-a0-v2"
        run_id = $RunId
        lock_sha256 = if ($null -ne $preflight) { [string]$preflight.lock_sha256 } else { "unavailable" }
        native_observation_reached = $script:nativeObservationReached
        provenance_failure = $script:provenanceFailure
        cleanup_barrier_passed = $script:cleanupBarrierPassed
        substantive_failure_observed = $script:substantiveFailureObserved
        inherited_suites_passed = @($suites | Where-Object { $_.pass }).Count
        inherited_suites_required = 2
        matrix_combinations_passed = @($matrix | Where-Object { $_.pass }).Count
        matrix_combinations_required = 4
        decision = $decision
        failure_class = $failureClass
        fatal_detail = $fatalDetail
        applied_disposition = $disposition
    }
    Write-Json (Join-Path $runDir "summary.json") $summary
    $rows = @($matrix | ForEach-Object {
        "| $($_.id) | $($_.producer_state) / $($_.producer_exit_code) | $($_.consumer_state) / $($_.consumer_exit_code) | $($_.active_references_resolved)/3 | $($_.negative_control_observed) | $($_.pass) |"
    }) -join [Environment]::NewLine
    $report = @"
# Strengthened Gate A0 v2 Report

- Run: ``$RunId``
- Native observation reached: $($script:nativeObservationReached)
- Inherited backend suites: $(@($suites | Where-Object { $_.pass }).Count)/2
- Matrix combinations: $(@($matrix | Where-Object { $_.pass }).Count)/4
- Failure class: ``$failureClass``
- **Decision: $decision**

| Combination | Producer state / exit | Consumer state / exit | Active resolved | Negative control | Pass |
|---|---:|---:|---:|---|---:|
$rows

## Disposition

$disposition

The negative control is a real north-face fingerprint under an intentionally absent semantic role. ``Resolved`` or ``Ambiguous`` is forbidden. Historical v1 and A0-D artifacts were not modified.
"@
    Write-Utf8 (Join-Path $runDir "report.md") ($report + [Environment]::NewLine)
    Seal-Run
}

if ($decision -notin @("FULL_GO", "SAME_BUILD_GO")) { throw "Strengthened A0 v2 returned $decision. See $runDir" }
Write-Output "Strengthened A0 v2 completed $decision and sealed evidence at $runDir."
