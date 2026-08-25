[CmdletBinding()]
param(
    [string]$MappingPath,
    [string]$CanonicalTasksPath,
    [string]$OutputDir,
    [switch]$ValidateOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($MappingPath)) {
    $MappingPath = Join-Path $PSScriptRoot "canonical-task-replay-map.json"
}
if ([string]::IsNullOrWhiteSpace($CanonicalTasksPath)) {
    $CanonicalTasksPath = Join-Path $repoRoot "corpora\canonical-tasks.yaml"
}
$MappingPath = [IO.Path]::GetFullPath($MappingPath)
$CanonicalTasksPath = [IO.Path]::GetFullPath($CanonicalTasksPath)
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $runId = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssZ")
    $OutputDir = Join-Path $repoRoot "artifacts\m19\canonical-task-runs\$runId"
}
$OutputDir = [IO.Path]::GetFullPath($OutputDir)

function Read-BoundedJson([string]$Path, [string]$Label) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "$Label does not exist: $Path" }
    $length = (Get-Item $Path).Length
    if ($length -le 0 -or $length -gt 1048576) {
        throw "$Label must contain between 1 and 1048576 bytes, got $length."
    }
    try {
        return [IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) | ConvertFrom-Json
    } catch {
        throw "$Label is not valid bounded JSON: $($_.Exception.Message)"
    }
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-StringArray($Value, [string]$Label) {
    if ($null -eq $Value) { throw "$Label is missing." }
    $result = @($Value | ForEach-Object { [string]$_ })
    if (@($result | Where-Object { [string]::IsNullOrWhiteSpace($_) }).Count -ne 0) {
        throw "$Label contains an empty value."
    }
    return $result
}

function Assert-ExactStrings([string[]]$Actual, [string[]]$Expected, [string]$Label) {
    $actualSorted = @($Actual | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    if ($actualSorted.Count -ne $expectedSorted.Count) {
        throw "$Label count mismatch: expected $($expectedSorted.Count), got $($actualSorted.Count)."
    }
    for ($index = 0; $index -lt $expectedSorted.Count; $index++) {
        if ($actualSorted[$index] -cne $expectedSorted[$index]) {
            throw "$Label mismatch at ${index}: expected '$($expectedSorted[$index])', got '$($actualSorted[$index])'."
        }
    }
}

function Assert-Unique([string[]]$Values, [string]$Label) {
    $duplicates = @($Values | Group-Object | Where-Object Count -gt 1 | ForEach-Object Name)
    if ($duplicates.Count -ne 0) {
        throw "$Label contains duplicates: $([string]::Join(', ', $duplicates))."
    }
}

$lock = Read-BoundedJson $CanonicalTasksPath "canonical task lock"
$mapping = Read-BoundedJson $MappingPath "canonical task replay map"
if ([int]$lock.schema_version -ne 1 -or [string]$lock.freeze_id -cne "r0-v1" -or
    [string]$lock.status -cne "frozen-before-a0") {
    throw "Canonical task lock identity is not the accepted frozen r0-v1 schema."
}
if ([int]$mapping.schema_version -ne 1 -or [string]$mapping.freeze_id -cne [string]$lock.freeze_id -or
    [string]$mapping.authority -cne "serial-headless-egui-kittest-accesskit") {
    throw "Replay map identity or authority does not match the frozen lock."
}

$lockedTasks = @($lock.tasks)
$mappedTasks = @($mapping.tasks)
if ($lockedTasks.Count -ne 20) { throw "Frozen lock must contain exactly 20 tasks, got $($lockedTasks.Count)." }
if ($mappedTasks.Count -ne 20) { throw "Replay map must contain exactly 20 tasks, got $($mappedTasks.Count)." }
$expectedIds = @(1..20 | ForEach-Object { "T{0:D2}" -f $_ })
$lockedIds = @($lockedTasks | ForEach-Object { [string]$_.id })
$mappedIds = @($mappedTasks | ForEach-Object { [string]$_.id })
Assert-Unique $lockedIds "canonical task lock IDs"
Assert-Unique $mappedIds "replay map IDs"
Assert-ExactStrings $lockedIds $expectedIds "canonical task lock IDs"
Assert-ExactStrings $mappedIds $expectedIds "replay map IDs"

$validatedTasks = [Collections.Generic.List[object]]::new()
foreach ($taskId in $expectedIds) {
    $locked = @($lockedTasks | Where-Object { [string]$_.id -ceq $taskId })
    $mapped = @($mappedTasks | Where-Object { [string]$_.id -ceq $taskId })
    if ($locked.Count -ne 1 -or $mapped.Count -ne 1) {
        throw "$taskId must have exactly one lock record and exactly one replay mapping."
    }
    $locked = $locked[0]
    $mapped = $mapped[0]
    if ([string]$mapped.expected_intent -cne [string]$locked.expected_intent) {
        throw "$taskId expected_intent does not match the frozen lock."
    }
    Assert-ExactStrings `
        (Get-StringArray $mapped.command_batch_shape "$taskId mapped command shape") `
        (Get-StringArray $locked.command_batch_shape "$taskId locked command shape") `
        "$taskId command shape"

    $stateOracles = @(Get-StringArray $mapped.oracles.state "$taskId state oracles")
    $diffOracles = @(Get-StringArray $mapped.oracles.diff "$taskId diff oracles")
    $lossOracles = @(Get-StringArray $mapped.oracles.loss "$taskId loss oracles")
    $allOracles = @($stateOracles + $diffOracles + $lossOracles)
    Assert-Unique $allOracles "$taskId mapped oracles"
    Assert-ExactStrings `
        $allOracles `
        (Get-StringArray $locked.invariants "$taskId locked invariants") `
        "$taskId invariant coverage"

    $tests = @($mapped.tests)
    if ($tests.Count -eq 0) { throw "$taskId has no headless product test mapping." }
    $selectors = [Collections.Generic.List[string]]::new()
    foreach ($test in $tests) {
        $target = [string]$test.target
        $testName = [string]$test.test
        if ($target -cnotmatch '^[a-z0-9_]+$' -or $testName -cnotmatch '^[a-z0-9_]+$') {
            throw "$taskId contains an unsafe Cargo test selector."
        }
        $selector = "${target}::$testName"
        $selectors.Add($selector)
        $sourcePath = Join-Path $repoRoot "crates\ketchup-app\tests\$target.rs"
        if (-not (Test-Path $sourcePath -PathType Leaf)) {
            throw "$taskId maps to missing integration-test source: $sourcePath"
        }
        $source = [IO.File]::ReadAllText($sourcePath, [Text.Encoding]::UTF8)
        $escapedName = [regex]::Escape($testName)
        if ($source -cnotmatch "(?s)#\[test\]\s*fn\s+$escapedName\s*\(") {
            throw "$taskId maps to a missing #[test] function: $selector"
        }
    }
    Assert-Unique $selectors.ToArray() "$taskId test selectors"
    $validatedTasks.Add([ordered]@{
        id = $taskId
        name = [string]$locked.name
        expected_intent = [string]$locked.expected_intent
        command_batch_shape = @(Get-StringArray $locked.command_batch_shape "$taskId locked command shape")
        tests = @($tests)
        oracles = [ordered]@{
            state = @($stateOracles)
            diff = @($diffOracles)
            loss = @($lossOracles)
        }
    })
}

if ($ValidateOnly) {
    Write-Host "PASS: frozen r0-v1 T01-T20 replay map is complete, unique, source-resolved, and oracle-complete."
    exit 0
}
if (Test-Path $OutputDir) { throw "OutputDir already exists; refusing to overwrite evidence: $OutputDir" }

$cargo = (Get-Command cargo -CommandType Application -ErrorAction Stop).Source
$parentDir = Split-Path $OutputDir -Parent
[void](New-Item $parentDir -ItemType Directory -Force)
$staging = "$OutputDir.staging-$([Guid]::NewGuid().ToString('N'))"
[void](New-Item $staging -ItemType Directory)
$completed = $false
try {
    $logsDir = Join-Path $staging "logs"
    [void](New-Item $logsDir -ItemType Directory)
    $results = [Collections.Generic.List[object]]::new()
    foreach ($task in $validatedTasks) {
        $testResults = [Collections.Generic.List[object]]::new()
        foreach ($test in @($task.tests)) {
            $target = [string]$test.target
            $testName = [string]$test.test
            $selector = "${target}::$testName"
            $stem = "$($task.id)-$target-$testName"
            $stdoutPath = Join-Path $logsDir "$stem.stdout.txt"
            $stderrPath = Join-Path $logsDir "$stem.stderr.txt"
            $arguments = @(
                "test", "-p", "ketchup-app", "--test", $target, $testName,
                "--", "--exact", "--test-threads=1"
            )
            $started = [DateTime]::UtcNow
            Write-Host "[$($task.id)] cargo $([string]::Join(' ', $arguments))"
            $previousPreference = $ErrorActionPreference
            try {
                $ErrorActionPreference = "Continue"
                Push-Location $repoRoot
                try {
                    $transcriptLines = @(& $cargo @arguments 2>&1)
                    $exitCode = $LASTEXITCODE
                } finally {
                    Pop-Location
                }
            } finally {
                $ErrorActionPreference = $previousPreference
            }
            $finished = [DateTime]::UtcNow
            $stdout = [string]::Join(
                [Environment]::NewLine,
                @($transcriptLines | ForEach-Object { $_.ToString() })
            ) + [Environment]::NewLine
            $stderr = ""
            [IO.File]::WriteAllText($stdoutPath, $stdout, [Text.UTF8Encoding]::new($false))
            [IO.File]::WriteAllText($stderrPath, $stderr, [Text.UTF8Encoding]::new($false))
            $transcript = "$stdout`n$stderr"
            if ($exitCode -ne 0) {
                throw "$($task.id) $selector failed with exit code $exitCode."
            }
            if ($transcript -cnotmatch 'test result: ok\.\s+1 passed;\s+0 failed;') {
                throw "$($task.id) $selector did not prove exactly one passing test; zero-test and filtered-out success are rejected."
            }
            $testResults.Add([ordered]@{
                selector = $selector
                status = "PASS"
                exit_code = $exitCode
                started_utc = $started.ToString("o")
                finished_utc = $finished.ToString("o")
                duration_ms = [int64]($finished - $started).TotalMilliseconds
                stdout = "logs/$stem.stdout.txt"
                stdout_sha256 = Get-Sha256 $stdoutPath
                stderr = "logs/$stem.stderr.txt"
                stderr_sha256 = Get-Sha256 $stderrPath
            })
        }
        $results.Add([ordered]@{
            id = $task.id
            name = $task.name
            status = "PASS"
            expected_intent = $task.expected_intent
            command_batch_shape = @($task.command_batch_shape)
            tests = @($testResults)
            state = [ordered]@{
                status = if (@($task.oracles.state).Count -eq 0) { "NOT_APPLICABLE" } else { "PASS" }
                oracles = @($task.oracles.state)
            }
            diff = [ordered]@{
                status = if (@($task.oracles.diff).Count -eq 0) { "NOT_APPLICABLE" } else { "PASS" }
                oracles = @($task.oracles.diff)
            }
            loss = [ordered]@{
                status = if (@($task.oracles.loss).Count -eq 0) { "NOT_APPLICABLE" } else { "PASS" }
                oracles = @($task.oracles.loss)
            }
        })
    }

    $sourcePaths = [Collections.Generic.List[string]]::new()
    foreach ($path in @(& git -C $repoRoot ls-files -- Cargo.toml Cargo.lock crates locales scripts corpora thresholds)) {
        if (-not [string]::IsNullOrWhiteSpace($path)) { $sourcePaths.Add(([string]$path).Replace('\', '/')) }
    }
    if ($LASTEXITCODE -ne 0) { throw "git ls-files failed while binding current-tree evidence." }
    foreach ($path in @(& git -C $repoRoot ls-files --others --exclude-standard -- Cargo.toml Cargo.lock crates locales scripts corpora thresholds)) {
        if (-not [string]::IsNullOrWhiteSpace($path)) { $sourcePaths.Add(([string]$path).Replace('\', '/')) }
    }
    if ($LASTEXITCODE -ne 0) { throw "git untracked source enumeration failed while binding current-tree evidence." }
    $sourcePaths = @($sourcePaths | Sort-Object -Unique)
    $sourceRegistry = [Collections.Generic.List[object]]::new()
    foreach ($relative in $sourcePaths) {
        $absolute = Join-Path $repoRoot $relative
        if (-not (Test-Path $absolute -PathType Leaf)) { throw "Current-tree source disappeared during certification: $relative" }
        $sourceRegistry.Add([ordered]@{
            path = $relative
            size_bytes = (Get-Item $absolute).Length
            sha256 = Get-Sha256 $absolute
        })
    }
    $head = (& git -C $repoRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $head -cnotmatch '^[0-9a-f]{40}$') { throw "Could not bind certification to git HEAD." }
    $cargoVersion = (& $cargo --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw "cargo --version failed." }
    $rustcVersion = (& rustc --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw "rustc --version failed." }

    $manifest = [ordered]@{
        schema_version = 1
        kind = "g19-03-canonical-task-replay"
        status = "PASS"
        result = "20/20"
        freeze_id = [string]$lock.freeze_id
        platform = "windows-x86_64"
        execution = "serial-headless-egui-kittest-accesskit"
        physical_desktop_input_used = $false
        captured_utc = [DateTime]::UtcNow.ToString("o")
        canonical_tasks_path = "corpora/canonical-tasks.yaml"
        canonical_tasks_sha256 = Get-Sha256 $CanonicalTasksPath
        mapping_path = "scripts/windows/canonical-task-replay-map.json"
        mapping_sha256 = Get-Sha256 $MappingPath
        runner_path = "scripts/windows/run-canonical-task-replay.ps1"
        runner_sha256 = Get-Sha256 $PSCommandPath
        git_head = $head
        cargo_version = $cargoVersion
        rustc_version = $rustcVersion
        source_registry = @($sourceRegistry)
        tasks = @($results)
        release_eligible = $false
        remaining_release_blockers = @("G19-04-current-tree-hardware-certification")
    }
    $manifestPath = Join-Path $staging "evidence-manifest.json"
    [IO.File]::WriteAllText(
        $manifestPath,
        (($manifest | ConvertTo-Json -Depth 16) + "`n"),
        [Text.UTF8Encoding]::new($false)
    )
    Move-Item $staging $OutputDir
    $completed = $true
    Write-Host "PASS: G19-03 current-tree canonical task replay completed 20/20."
    Write-Host "Evidence: $(Join-Path $OutputDir 'evidence-manifest.json')"
} finally {
    if (-not $completed -and (Test-Path $staging)) {
        Remove-Item $staging -Recurse -Force
    }
}
