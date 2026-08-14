[CmdletBinding()]
param(
    [string]$RepoRoot,
    [string]$LedgerPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
} else {
    $RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
}
if ([string]::IsNullOrWhiteSpace($LedgerPath)) {
    $LedgerPath = Join-Path $RepoRoot "governance\r0-transitions-v1-v13.json"
}
$repoPrefix = $RepoRoot.TrimEnd("\") + "\"

function Resolve-RepositoryFile([string]$RelativePath) {
    $fullPath = [IO.Path]::GetFullPath((Join-Path $RepoRoot $RelativePath.Replace("/", "\")))
    if (-not $fullPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path $fullPath -PathType Leaf)) {
        throw "Unsafe or missing transition evidence: $RelativePath"
    }
    return $fullPath
}

if (-not (Test-Path $LedgerPath -PathType Leaf)) {
    throw "Missing R0 transition classification ledger: $LedgerPath"
}
$ledger = Get-Content $LedgerPath -Raw | ConvertFrom-Json
if ($ledger.schema_version -ne 1 -or
    $ledger.ledger_id -ne "r0-v1-v13-transition-classification-v1" -or
    -not $ledger.classification_rule.unknown_is_fail_closed -or
    $ledger.classification_rule.post_failure_envelope_narrowing_is -ne "loosen") {
    throw "Unsupported or weakened R0 transition classification contract."
}

$commonThresholdHash = ([string]$ledger.common_threshold_sha256).ToLowerInvariant()
if ($commonThresholdHash -notmatch '^[0-9a-f]{64}$') {
    throw "The common R0 threshold hash is invalid."
}
$activeThresholdHash = (Get-FileHash (Resolve-RepositoryFile "thresholds/r0.yaml") -Algorithm SHA256).Hash.ToLowerInvariant()
if ($activeThresholdHash -ne $commonThresholdHash) {
    throw "Active thresholds no longer match the classified R0 V1-V13 contract."
}

$previousLockPath = Resolve-RepositoryFile "artifacts/r0/preregistration-lock.json"
$previousLock = Get-Content $previousLockPath -Raw | ConvertFrom-Json
if ($previousLock.freeze_id -ne "r0-v1") { throw "The transition chain must begin at r0-v1." }
$v1Thresholds = @($previousLock.files | Where-Object { $_.path -eq "thresholds/r0.yaml" })
if ($v1Thresholds.Count -ne 1 -or ([string]$v1Thresholds[0].sha256).ToLowerInvariant() -ne $commonThresholdHash) {
    throw "r0-v1 does not carry the classified common threshold hash."
}
$previousFiles = @{}
foreach ($entry in @($previousLock.files)) { $previousFiles[[string]$entry.path] = ([string]$entry.sha256).ToLowerInvariant() }

$transitions = @($ledger.transitions)
if ($transitions.Count -ne 12) { throw "R0 V1-V13 requires exactly 12 classified transitions." }
$timingValues = @(
    "before_affected_observation",
    "after_affected_observation",
    "after_previous_before_affected_observation",
    "after_affected_failure"
)
for ($index = 0; $index -lt $transitions.Count; $index++) {
    $transition = $transitions[$index]
    $expectedFrom = "r0-v$($index + 1)"
    $expectedTo = "r0-v$($index + 2)"
    if ($transition.from -ne $expectedFrom -or $transition.to -ne $expectedTo) {
        throw "Transition $index breaks the contiguous R0 V1-V13 chain."
    }
    $direction = ([string]$transition.direction).ToLowerInvariant()
    if ($direction -notin @("tighten", "loosen", "neutral") -or $direction -eq "unknown") {
        throw "$expectedFrom->$expectedTo has an unknown or invalid direction."
    }
    if ([string]$transition.observation_timing -notin $timingValues) {
        throw "$expectedFrom->$expectedTo lacks a valid affected-observation timing classification."
    }
    if (@($transition.changed_inputs).Count -eq 0 -or [string]::IsNullOrWhiteSpace([string]$transition.reason)) {
        throw "$expectedFrom->$expectedTo lacks changed-input or rationale evidence."
    }
    if ($transition.observation_timing -eq "after_affected_failure") {
        if (-not ($transition.PSObject.Properties.Name -contains "post_failure_envelope_narrowing")) {
            throw "$expectedFrom->$expectedTo must state whether post-failure envelope narrowing occurred."
        }
        if ($transition.post_failure_envelope_narrowing -and $direction -ne "loosen") {
            throw "$expectedFrom->$expectedTo narrows an envelope after failure and must be classified loosen."
        }
    }

    $lockPath = Resolve-RepositoryFile ([string]$transition.lock)
    $reportPath = Resolve-RepositoryFile ([string]$transition.report)
    $lock = Get-Content $lockPath -Raw | ConvertFrom-Json
    if ($lock.freeze_id -ne $expectedTo -or $lock.supersedes.freeze_id -ne $expectedFrom) {
        throw "$expectedFrom->$expectedTo lock identity or supersession is invalid."
    }
    $currentFiles = @{}
    foreach ($entry in @($lock.files)) { $currentFiles[[string]$entry.path] = ([string]$entry.sha256).ToLowerInvariant() }
    $actualChangedInputs = @(@($previousFiles.Keys) + @($currentFiles.Keys) | Sort-Object -Unique | Where-Object {
        -not $previousFiles.ContainsKey($_) -or -not $currentFiles.ContainsKey($_) -or $previousFiles[$_] -ne $currentFiles[$_]
    })
    $declaredChangedInputs = @($transition.changed_inputs | ForEach-Object { [string]$_ } | Sort-Object -Unique)
    if (($actualChangedInputs -join "`n") -ne ($declaredChangedInputs -join "`n")) {
        throw "$expectedFrom->$expectedTo changed_inputs does not match adjacent lock contents. Actual: $($actualChangedInputs -join ', ')"
    }
    $previousHash = (Get-FileHash $previousLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if (([string]$lock.supersedes.lock_sha256).ToLowerInvariant() -ne $previousHash) {
        throw "$expectedFrom->$expectedTo does not bind the exact preceding lock."
    }
    $thresholds = @($lock.files | Where-Object { $_.path -eq "thresholds/r0.yaml" })
    if ($thresholds.Count -ne 1 -or ([string]$thresholds[0].sha256).ToLowerInvariant() -ne $commonThresholdHash) {
        throw "$expectedTo changed thresholds or the operation envelope despite classification $direction."
    }
    $report = Get-Content $reportPath -Raw
    if (-not $report.Contains("``$expectedTo``")) {
        throw "$expectedFrom->$expectedTo report does not identify its target freeze."
    }
    $previousLockPath = $lockPath
    $previousFiles = $currentFiles
}

$coverage = $ledger.upper_envelope_coverage
if ($coverage.freeze_id -ne "m0-upper-envelope-v2" -or
    $coverage.policy_change -ne "none" -or
    $coverage.direction -ne "tighten") {
    throw "Upper-envelope coverage must tighten evidence without changing frozen policy."
}
$testPath = Resolve-RepositoryFile ([string]$coverage.test_path)
$testHash = (Get-FileHash $testPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($testHash -ne ([string]$coverage.test_sha256).ToLowerInvariant()) {
    throw "Upper-envelope executable evidence changed without a new classified freeze."
}
$testSource = Get-Content $testPath -Raw
$cases = @($coverage.cases)
if ($cases.Count -ne 5) { throw "Upper-envelope coverage must retain all five classified cases." }
foreach ($case in $cases) {
    if ($testSource -notmatch "(?m)^#\[test\]\r?\nfn\s+$([regex]::Escape([string]$case))\s*\(") {
        throw "Upper-envelope evidence is missing case: $case"
    }
}
foreach ($required in @(
    "1_000_000.0",
    "GeometryErrorCode::InvalidParameter",
    "GeometryErrorCode::NonFiniteParameter",
    "CutMode::ThroughAll"
)) {
    if (-not $testSource.Contains($required)) { throw "Upper-envelope evidence lost required assertion token: $required" }
}

Write-Output "R0 transition governance passed: 12 V1-V13 transitions are classified, all 13 locks retain one threshold hash, and five upper-envelope cases are frozen."
