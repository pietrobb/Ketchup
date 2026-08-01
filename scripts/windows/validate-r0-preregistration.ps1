[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$repoPrefix = $repoRoot.TrimEnd("\") + "\"

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Read-JsonYaml([string]$RelativePath) {
    $path = Join-Path $repoRoot $RelativePath
    return Get-Content $path -Raw | ConvertFrom-Json
}

$lockPath = Join-Path $repoRoot "artifacts\r0\preregistration-lock.json"
$lock = Get-Content $lockPath -Raw | ConvertFrom-Json
Assert-True ($lock.schema_version -eq 1 -and $lock.freeze_id -eq "r0-v1") "Unexpected preregistration lock identity."
Assert-True ($lock.measurement_state_at_freeze -eq "not_started") "The lock was not created before measurements."
Assert-True (@($lock.files).Count -eq 16) "The preregistration lock file set changed."

$seen = @{}
foreach ($entry in @($lock.files)) {
    $relative = [string]$entry.path
    Assert-True (-not $seen.ContainsKey($relative)) "Duplicate locked path: $relative"
    $seen[$relative] = $true
    $full = [IO.Path]::GetFullPath((Join-Path $repoRoot $relative))
    Assert-True ($full.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) "Locked path escapes the repository: $relative"
    Assert-True (Test-Path $full -PathType Leaf) "Missing locked file: $relative"
    $actual = (Get-FileHash $full -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-True ($actual -eq [string]$entry.sha256) "Locked hash mismatch: $relative"
}

$thresholds = Read-JsonYaml "thresholds\r0.yaml"
$corpora = Read-JsonYaml "corpora\manifest.yaml"
$tasks = Read-JsonYaml "corpora\canonical-tasks.yaml"
foreach ($document in @($thresholds, $corpora, $tasks)) {
    Assert-True ($document.schema_version -eq 1 -and $document.freeze_id -eq "r0-v1") "Preregistration document identity mismatch."
    Assert-True ($document.status -eq "frozen-before-a0") "Preregistration document is not frozen before A0."
    if ($document.PSObject.Properties.Name -contains "measurement_state") {
        Assert-True ($document.measurement_state -eq "not_started") "A measurement predates the freeze."
    }
}

$taskList = @($tasks.tasks)
Assert-True ($taskList.Count -eq 20) "Exactly 20 canonical tasks are required."
$expectedTaskIds = 1..20 | ForEach-Object { "T{0:D2}" -f $_ }
$actualTaskIds = @($taskList | ForEach-Object { [string]$_.id })
Assert-True (($actualTaskIds -join "|") -eq ($expectedTaskIds -join "|")) "Canonical task IDs or order changed."
Assert-True ((@($actualTaskIds | Select-Object -Unique).Count) -eq 20) "Canonical task IDs are not unique."
foreach ($task in $taskList) {
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$task.request)) "Task $($task.id) has no frozen request."
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$task.expected_intent)) "Task $($task.id) has no expected Intent."
    Assert-True ($task.PSObject.Properties.Name -contains "command_batch_shape") "Task $($task.id) has no CommandBatch shape."
    Assert-True (@($task.invariants).Count -gt 0) "Task $($task.id) has no deterministic invariants."
}

Assert-True (@($corpora.fixed).Count -eq 4) "The fixed corpus changed."
Assert-True ($corpora.generative.case_count -eq 1000 -and @($corpora.generative.seeds).Count -eq 20) "The generative corpus changed."
Assert-True (@($corpora.mutation.cases).Count -eq 8) "The mutation corpus changed."
Assert-True (@($corpora.adversarial.expected_valid).Count -eq 10) "The expected-valid adversarial corpus changed."
Assert-True (@($corpora.adversarial.expected_rejected).Count -eq 6) "The expected-rejected adversarial corpus changed."
Assert-True (@($corpora.external_step.fixtures).Count -eq 3) "The STEP corpus changed."
Assert-True ($corpora.external_step.provenance.license -eq "Apache-2.0") "The STEP corpus lacks the frozen redistribution license."
foreach ($fixture in @($corpora.external_step.fixtures)) {
    $path = Join-Path $repoRoot ([string]$fixture.path)
    Assert-True (Test-Path $path -PathType Leaf) "Missing STEP fixture: $($fixture.id)"
    $actual = (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-True ($actual -eq [string]$fixture.sha256) "STEP fixture hash mismatch: $($fixture.id)"
}

$referenceIds = @($thresholds.guaranteed_subset.references | ForEach-Object { [string]$_.id })
$expectedReferences = @("extrusion.top", "extrusion.bottom", "extrusion.side(profile_edge=east)")
Assert-True (($referenceIds -join "|") -eq ($expectedReferences -join "|")) "The frozen Guaranteed subset changed."
$mutationIds = @($thresholds.guaranteed_subset.mutation_cases | ForEach-Object { [string]$_ })
$expectedMutationIds = 1..8 | ForEach-Object { "M{0:D2}" -f $_ }
Assert-True (($mutationIds -join "|") -eq ($expectedMutationIds -join "|")) "The Guaranteed mutation matrix changed."
Assert-True ($thresholds.gates.A0.metrics.ffi_fuzz_call_count_min -eq 10000) "A0 fuzz threshold changed."
Assert-True ($thresholds.gates.A0.metrics.silent_invalid_shape_max -eq 0) "A0 invalid-shape threshold changed."
Assert-True ($thresholds.gates.A0.metrics.guaranteed_identity_correct_percent_min -eq 100) "A0 identity threshold changed."
Assert-True ($thresholds.gates.A1.metrics.canonical_changes_after_100_save_load_cycles_max -eq 0) "A1 persistence threshold changed."
Assert-True ($thresholds.gates.B.metrics.schedule_permutations_min -eq 10000) "Gate B schedule threshold changed."
Assert-True ($thresholds.gates.C.metrics.preview_commit_action_digest_match_percent_min -eq 100) "Gate C digest threshold changed."
Assert-True ($thresholds.gates.FLP.canonical_tasks_pass_required -eq 20) "FLP task threshold changed."

$hardwareIds = @($thresholds.hardware_profiles | ForEach-Object { [string]$_.id })
Assert-True (($hardwareIds -join "|") -eq "HP-DEV-01|HP-IGPU-01") "Required hardware profiles changed."
$queryIds = @($thresholds.query_classes | ForEach-Object { [string]$_.id })
$expectedQueryIds = @("QC-B-READER-01", "QC-B-TRANSPORT-01", "QC-C-NAV-01", "QC-C-EDIT-01", "QC-C-PICK-01", "QC-C-LONG-01")
Assert-True (($queryIds -join "|") -eq ($expectedQueryIds -join "|")) "Named query classes changed."
Assert-True (@($thresholds.owners_and_deadlines).Count -eq 5) "Owners and deadlines are incomplete."

$occt = Get-Content (Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json") -Raw | ConvertFrom-Json
Assert-True ($occt.status -eq "built-and-fingerprinted") "OCCT is not built and fingerprinted."
Assert-True ($occt.source.release -eq "8.0.1" -and $occt.source.commit -eq "b8f597c677811d1f9f4d8a97f5ae2825c0353a42" -and $occt.source.clean) "OCCT source provenance changed."
Assert-True ($occt.build.library_type -eq "Shared" -and @($occt.shared_libraries).Count -eq 48) "OCCT shared-library model changed."
Assert-True ($occt.install_tree.file_count -eq 7400) "OCCT install-tree evidence changed."

$reportPath = Join-Path $repoRoot "docs\gates\R0_REPORT.md"
Assert-True (Test-Path $reportPath -PathType Leaf) "Missing R0 decision report."
$report = Get-Content $reportPath -Raw
$lockHash = (Get-FileHash $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-True ($report.Contains("**Decision: GO**")) "R0 report is not GO."
Assert-True ($report.Contains($lockHash)) "R0 report does not identify the exact preregistration lock."

Write-Output "R0 preregistration validation passed: 20 tasks, 4 fixed fixtures, 1,000 generated cases, 8 mutation cases, 16 adversarial cases, 3 STEP fixtures, 3 Guaranteed references, and 16 locked files."
