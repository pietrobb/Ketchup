[CmdletBinding()]
param([string]$RunId = "a0d-run-001")

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$artifactRoot = Join-Path $repoRoot "artifacts\gate-a0\diagnostics"
$runDir = Join-Path $artifactRoot $RunId
$logDir = Join-Path $runDir "process-logs"
$fixtureDir = Join-Path $runDir "fixtures"
$inputDir = Join-Path $runDir "inputs"
$validator = Join-Path $PSScriptRoot "validate-strengthened-a0-v1.ps1"
$probeSource = Join-Path $repoRoot "crates\ketchup-exact\src\bin\ketchup-a0-diagnostic-probe.rs"
$runnerSource = $PSCommandPath
$script:processRecords = @()

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}

function Write-Json([string]$Path, [object]$Value) {
    Write-Utf8 $Path (($Value | ConvertTo-Json -Depth 12) + [Environment]::NewLine)
}

function Get-RunRelativePath([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    return $fullPath.Substring($runDir.TrimEnd("\").Length + 1).Replace("\", "/")
}

function Invoke-CapturedProcess(
    [string]$Id,
    [string]$Stage,
    [string]$FilePath,
    [string[]]$Arguments,
    [hashtable]$Environment,
    [object]$BuildIdentity
) {
    $stdoutPath = Join-Path $logDir ($Id + ".stdout.txt")
    $stderrPath = Join-Path $logDir ($Id + ".stderr.txt")
    $startedUtc = [DateTime]::UtcNow.ToString("o")
    $stdout = ""
    $stderr = ""
    $exitCode = -1
    try {
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $FilePath
        $startInfo.Arguments = $Arguments -join " "
        $startInfo.WorkingDirectory = $repoRoot
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        foreach ($name in $Environment.Keys) {
            $startInfo.EnvironmentVariables[$name] = [string]$Environment[$name]
        }
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw "Process did not start." }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $stdout = $stdoutTask.Result
        $stderr = $stderrTask.Result
        $exitCode = $process.ExitCode
        $process.Dispose()
    } catch {
        $stderr = $_.Exception.ToString()
    }
    Write-Utf8 $stdoutPath $stdout
    Write-Utf8 $stderrPath $stderr
    $record = [ordered]@{
        id = $Id
        stage = $Stage
        state = "ran"
        started_utc = $startedUtc
        ended_utc = [DateTime]::UtcNow.ToString("o")
        command = $FilePath
        arguments = @($Arguments)
        command_display = (@($FilePath) + $Arguments) -join " "
        build_identity = $BuildIdentity
        exit_code = $exitCode
        success = ($exitCode -eq 0)
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
        command_display = $null
        build_identity = $BuildIdentity
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

function Get-Diagnosis([object[]]$Matrix) {
    $priorSame = @($Matrix | Where-Object { $_.id -eq "prior-to-prior" })[0]
    $currentSame = @($Matrix | Where-Object { $_.id -eq "current-to-current" })[0]
    $cross = @($Matrix | Where-Object { $_.id -in @("prior-to-current", "current-to-prior") })
    if ($priorSame.pass -and $currentSame.pass -and (@($cross | Where-Object { -not $_.pass }).Count -eq 0)) {
        return "All four combinations passed. The historical strengthened-run-001 NO-GO is not reproduced; its runner/reporting path is the remaining demonstrated defect."
    }
    if (-not $priorSame.pass -or -not $currentSame.pass) {
        return "At least one same-build path failed. The evidence localizes the problem before cross-build transfer (build-specific construction, adjacency, capture, or consumer behavior); inspect the sealed process stderr."
    }
    return "Both same-build paths passed and at least one cross-build path failed. The evidence localizes the remaining defect to cross-build reference transfer or quarantine behavior."
}

if ($RunId -notmatch '^a0d-run-[0-9]{3}$') { throw "RunId must match a0d-run-NNN." }
if (Test-Path $runDir) { throw "A0 diagnostic evidence already exists: $RunId" }
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
New-Item -ItemType Directory -Path $fixtureDir -Force | Out-Null
New-Item -ItemType Directory -Path $inputDir -Force | Out-Null
Copy-Item $runnerSource (Join-Path $inputDir "run-a0-diagnostic-matrix-v1.ps1")
Copy-Item $probeSource (Join-Path $inputDir "ketchup-a0-diagnostic-probe.rs")

$powerShell = (Get-Command powershell.exe -ErrorAction Stop).Source
$cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
$preflightRecord = Invoke-CapturedProcess `
    "preflight-validator" `
    "preflight" `
    $powerShell `
    @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $validator, "-EmitJson") `
    @{} `
    ([ordered]@{ freeze_id = "strengthened-a0-v1" })
if (-not $preflightRecord.success) {
    Write-Json (Join-Path $runDir "processes.json") $script:processRecords
    throw "Strengthened A0 preflight failed; diagnostic matrix was not observed. See $($preflightRecord.stderr_path)."
}
$preflightStdout = Get-Content (Join-Path $runDir $preflightRecord.stdout_path) -Raw
$preflight = $preflightStdout | ConvertFrom-Json
$backendByAlias = [ordered]@{
    prior = $preflight.producer
    current = $preflight.consumer
}

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
    }
    $targetDir = Join-Path $env:TEMP ("ketchup-a0d-" + $RunId + "-" + $alias)
    $record = Invoke-CapturedProcess `
        ("build-" + $alias) `
        "build" `
        $cargo `
        @("build", "--manifest-path", (Join-Path $repoRoot "Cargo.toml"), "-p", "ketchup-exact", "--bin", "ketchup-a0-diagnostic-probe", "--target-dir", $targetDir) `
        @{ KETCHUP_OCCT_ROOT = [string]$backend.install_path; KETCHUP_OCCT_BUILD_FINGERPRINT = [string]$backend.fingerprint } `
        $identity
    $exePath = Join-Path $targetDir "debug\ketchup-a0-diagnostic-probe.exe"
    $runtimeError = if ($record.success) { Test-RuntimeLibraries (Join-Path $targetDir "debug") $backend } else { "build process failed" }
    $builds[$alias] = [ordered]@{
        identity = $identity
        record = $record
        available = ($record.success -and $null -eq $runtimeError -and (Test-Path $exePath -PathType Leaf))
        unavailable_reason = $runtimeError
        executable = $exePath
        executable_sha256 = if (Test-Path $exePath -PathType Leaf) { (Get-FileHash $exePath -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
    }
}

$combinations = @(
    @("prior", "prior"),
    @("current", "current"),
    @("prior", "current"),
    @("current", "prior")
)
$matrix = @()
foreach ($combination in $combinations) {
    $producerAlias = $combination[0]
    $consumerAlias = $combination[1]
    $id = $producerAlias + "-to-" + $consumerAlias
    $producerBuild = $builds[$producerAlias]
    $consumerBuild = $builds[$consumerAlias]
    $fixturePath = Join-Path $fixtureDir ($id + ".tsv")

    if (-not $producerBuild.available) {
        $producerRecord = Add-NotRunProcess ("producer-" + $id) "producer" ("producer build unavailable: " + $producerBuild.unavailable_reason) $producerBuild.identity
    } else {
        $producerRecord = Invoke-CapturedProcess `
            ("producer-" + $id) `
            "producer" `
            $producerBuild.executable `
            @("produce", $fixturePath) `
            @{} `
            $producerBuild.identity
    }

    if (-not $producerRecord.success) {
        $consumerRecord = Add-NotRunProcess ("consumer-" + $id) "consumer" "producer did not complete successfully" $consumerBuild.identity
    } elseif (-not $consumerBuild.available) {
        $consumerRecord = Add-NotRunProcess ("consumer-" + $id) "consumer" ("consumer build unavailable: " + $consumerBuild.unavailable_reason) $consumerBuild.identity
    } else {
        $consumerRecord = Invoke-CapturedProcess `
            ("consumer-" + $id) `
            "consumer" `
            $consumerBuild.executable `
            @("consume", $fixturePath) `
            @{} `
            $consumerBuild.identity
    }

    $matrix += [ordered]@{
        id = $id
        producer_build = $producerBuild.identity
        consumer_build = $consumerBuild.identity
        same_build_identity = ([string]$producerBuild.identity.fingerprint -eq [string]$consumerBuild.identity.fingerprint)
        producer_process_id = [string]$producerRecord.id
        producer_state = [string]$producerRecord.state
        producer_exit_code = $producerRecord.exit_code
        consumer_process_id = [string]$consumerRecord.id
        consumer_state = [string]$consumerRecord.state
        consumer_not_run = ([string]$consumerRecord.state -eq "not_run")
        consumer_not_run_reason = if ([string]$consumerRecord.state -eq "not_run") { [string]$consumerRecord.reason } else { $null }
        consumer_exit_code = $consumerRecord.exit_code
        pass = ($producerRecord.success -and $consumerRecord.success)
    }
}

$diagnosis = Get-Diagnosis $matrix
$matrixDocument = [ordered]@{
    schema_version = 1
    diagnostic_id = "a0-diagnostic-matrix-v1"
    run_id = $RunId
    observed_utc = [DateTime]::UtcNow.ToString("o")
    purpose = "Disambiguate harness, same-build capture/adjacency, and cross-build transfer without changing A0 thresholds or consequences."
    gate_effect = "diagnostic_only"
    threshold_or_consequence_change = "none; no loosen and no tightening"
    strengthened_a0_v1_lock_sha256 = [string]$preflight.lock_sha256
    runner_source_sha256 = (Get-FileHash $runnerSource -Algorithm SHA256).Hash.ToLowerInvariant()
    probe_source_sha256 = (Get-FileHash $probeSource -Algorithm SHA256).Hash.ToLowerInvariant()
    expected_combinations = 4
    observed_combinations = $matrix.Count
    passed_combinations = @($matrix | Where-Object { $_.pass }).Count
    diagnosis = $diagnosis
    combinations = $matrix
}
Write-Json (Join-Path $runDir "processes.json") $script:processRecords
Write-Json (Join-Path $runDir "matrix.json") $matrixDocument

$rows = @($matrix | ForEach-Object {
    "| $($_.id) | $($_.producer_state) / $($_.producer_exit_code) | $($_.consumer_state) / $($_.consumer_exit_code) | $($_.pass) |"
}) -join [Environment]::NewLine
$report = @"
# A0 Diagnostic Matrix v1

- Run: ``$RunId``
- Strengthened A0 v1 lock: ``$($preflight.lock_sha256)``
- Gate effect: diagnostic only
- Threshold/consequence change: none; no loosen and no tightening
- Combinations observed: $($matrix.Count)/4
- Combinations passed: $(@($matrix | Where-Object { $_.pass }).Count)/4

| Combination | Producer state / exit | Consumer state / exit | Pass |
|---|---:|---:|---:|
$rows

## Diagnosis

$diagnosis

Every launched process has immutable stdout/stderr files, exit code, command, build identity, and panic detection in ``processes.json``. Every skipped consumer is explicit as ``not_run`` with a reason.
"@
Write-Utf8 (Join-Path $runDir "report.md") ($report + [Environment]::NewLine)

$sealedFiles = @(Get-ChildItem $runDir -Recurse -File | Where-Object { $_.Name -ne "seal.json" } | Sort-Object FullName | ForEach-Object {
    [ordered]@{
        path = Get-RunRelativePath $_.FullName
        size = $_.Length
        sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
})
$seal = [ordered]@{
    schema_version = 1
    run_id = $RunId
    sealed_utc = [DateTime]::UtcNow.ToString("o")
    hash_algorithm = "SHA-256"
    file_count = $sealedFiles.Count
    files = $sealedFiles
}
Write-Json (Join-Path $runDir "seal.json") $seal

foreach ($entry in $seal.files) {
    $sealedPath = Join-Path $runDir ([string]$entry.path)
    if (-not (Test-Path $sealedPath -PathType Leaf)) { throw "Sealed file is missing: $($entry.path)" }
    $actual = (Get-FileHash $sealedPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne [string]$entry.sha256) { throw "Sealed file hash mismatch: $($entry.path)" }
}
if ($matrix.Count -ne 4) { throw "Diagnostic matrix did not preserve all four combinations." }
Write-Output "A0 diagnostic matrix sealed at $runDir with $(@($matrix | Where-Object { $_.pass }).Count)/4 passing combinations."
