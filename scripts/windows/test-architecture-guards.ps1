[CmdletBinding()]
param(
    [string]$RepoRoot,
    [string]$BaseRef,
    [string[]]$ChangedPaths,
    [string]$ChangeManifestPath,
    [string]$FrozenLockPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
} else {
    $RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
}
if ([string]::IsNullOrWhiteSpace($ChangeManifestPath)) {
    $ChangeManifestPath = Join-Path $RepoRoot "governance\contract-changes.json"
}
function Fail-Guard([string]$Id, [string]$Message) {
    throw "[guard:$Id] $Message"
}

function Get-BracedBlock([string]$Source, [string]$Marker, [string]$GuardId) {
    $start = $Source.IndexOf($Marker, [StringComparison]::Ordinal)
    if ($start -lt 0) { Fail-Guard $GuardId "Missing authority marker: $Marker" }
    $open = $Source.IndexOf("{", $start)
    if ($open -lt 0) { Fail-Guard $GuardId "Authority marker has no body: $Marker" }
    $depth = 0
    for ($index = $open; $index -lt $Source.Length; $index++) {
        if ($Source[$index] -eq "{") { $depth++ }
        if ($Source[$index] -eq "}") {
            $depth--
            if ($depth -eq 0) { return $Source.Substring($start, $index - $start + 1) }
        }
    }
    Fail-Guard $GuardId "Unbalanced authority body: $Marker"
}

function Normalize-Path([string]$Path) {
    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized.TrimStart([char[]]@("/"))
}

function Get-JsonLeaves([object]$Value, [string]$Prefix = "") {
    $result = @{}
    if ($null -eq $Value) {
        $result[$Prefix] = $null
        return $result
    }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        foreach ($property in $Value.PSObject.Properties) {
            $childPrefix = if ($Prefix) { "$Prefix.$($property.Name)" } else { $property.Name }
            $child = Get-JsonLeaves $property.Value $childPrefix
            foreach ($key in $child.Keys) { $result[$key] = $child[$key] }
        }
        return $result
    }
    if ($Value -is [Array]) {
        for ($index = 0; $index -lt $Value.Count; $index++) {
            $child = Get-JsonLeaves $Value[$index] "$Prefix[$index]"
            foreach ($key in $child.Keys) { $result[$key] = $child[$key] }
        }
        return $result
    }
    $result[$Prefix] = $Value
    return $result
}

function Get-ThresholdDirection([string]$Base, [string]$Root) {
    $oldText = @(& git -C $Root show "$Base`:thresholds/r0.yaml" 2>&1)
    if ($LASTEXITCODE -ne 0) { Fail-Guard "anti-loosening" "Cannot read base thresholds from $Base." }
    $oldLeaves = Get-JsonLeaves (($oldText -join "`n") | ConvertFrom-Json)
    $newLeaves = Get-JsonLeaves (Get-Content (Join-Path $Root "thresholds\r0.yaml") -Raw | ConvertFrom-Json)
    $directions = @()
    foreach ($key in @(@($oldLeaves.Keys) + @($newLeaves.Keys) | Sort-Object -Unique)) {
        if (-not $oldLeaves.ContainsKey($key) -or -not $newLeaves.ContainsKey($key)) {
            $directions += "unknown"
            continue
        }
        $oldValue = $oldLeaves[$key]
        $newValue = $newLeaves[$key]
        if ([string]$oldValue -eq [string]$newValue) { continue }
        if ($key.StartsWith("operation_envelope.", [StringComparison]::Ordinal)) {
            $directions += "unknown"
        } elseif ($oldValue -is [ValueType] -and $newValue -is [ValueType] -and $key -match '_max$') {
            $directions += $(if ([double]$newValue -gt [double]$oldValue) { "loosen" } else { "tighten" })
        } elseif ($oldValue -is [ValueType] -and $newValue -is [ValueType] -and $key -match '_min$') {
            $directions += $(if ([double]$newValue -lt [double]$oldValue) { "loosen" } else { "tighten" })
        } else {
            $directions += "unknown"
        }
    }
    if ($directions -contains "loosen") { return "loosen" }
    if ($directions -contains "unknown") { return "unknown" }
    if ($directions -contains "tighten") { return "tighten" }
    return "neutral"
}

$d08Path = Join-Path $RepoRoot "governance\d08-lifecycle-exceptions.json"
if (-not (Test-Path $d08Path -PathType Leaf)) {
    Fail-Guard "sole-mutation" "Missing D-08 lifecycle exception register."
}
$d08Hash = (Get-FileHash $d08Path -Algorithm SHA256).Hash.ToLowerInvariant()
if ($d08Hash -ne "a845a5bfc99fce5cd7ebd5850b90cf8dd9316cb2b9f25e2b496a38732b0b9f7c") {
    Fail-Guard "sole-mutation" "D-08 lifecycle exception semantics changed without a reviewed register version."
}
$d08 = Get-Content $d08Path -Raw | ConvertFrom-Json
if ($d08.schema_version -ne 2 -or $d08.decision -ne "D-08" -or
    $d08.invariant -ne "Only a validated canonical command batch mutates canonical model state.") {
    Fail-Guard "sole-mutation" "D-08 lifecycle exception contract is unsupported or weakened."
}
$expectedLifecycle = @{
    "undo" = "DocumentStore::undo|cursor_only"
    "redo" = "DocumentStore::redo|cursor_only"
    "discard-history-before-current" = "DocumentStore::discard_history_before_current|retention_only"
    "new-document" = "KetchupApp::new_document|validated_fresh_store_swap"
    "open-document" = "KetchupApp::open_document_from|validated_candidate_swap"
}
$lifecycleExceptions = @($d08.lifecycle_exceptions)
if ($lifecycleExceptions.Count -ne $expectedLifecycle.Count) {
    Fail-Guard "sole-mutation" "D-08 must declare exactly the five reviewed lifecycle exceptions."
}
foreach ($exception in $lifecycleExceptions) {
    $id = [string]$exception.id
    $shape = "$($exception.scope)|$($exception.kind)"
    if (-not $expectedLifecycle.ContainsKey($id) -or $expectedLifecycle[$id] -ne $shape -or
        @($exception.allowed_state_changes).Count -eq 0 -or @($exception.forbidden_effects).Count -eq 0 -or
        @($exception.evidence).Count -eq 0) {
        Fail-Guard "sole-mutation" "Invalid or unreviewed D-08 lifecycle exception: $id"
    }
    foreach ($evidence in @($exception.evidence)) {
        $evidencePath = [IO.Path]::GetFullPath((Join-Path $RepoRoot ([string]$evidence)))
        if (-not $evidencePath.StartsWith($RepoRoot.TrimEnd("\") + "\", [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path $evidencePath -PathType Leaf)) {
            Fail-Guard "sole-mutation" "D-08 exception $id has unsafe or missing evidence: $evidence"
        }
    }
}
$expectedCanonicalDelegates = @(
    "DocumentStore::commit_proposal",
    "DocumentStore::commit_verified_proposal",
    "DocumentStore::convert_group_to_component",
    "DocumentStore::make_unique"
) | Sort-Object
$actualCanonicalDelegates = @($d08.delegated_gateways | ForEach-Object {
    $expectedTarget = if ($_.scope -eq "DocumentStore::commit_proposal") {
        "DocumentStore::commit_verified_proposal"
    } else {
        "DocumentStore::apply_batch"
    }
    if ($_.delegates_to -ne $expectedTarget) {
        Fail-Guard "sole-mutation" "Canonical delegate has an unexpected target: $($_.scope)"
    }
    [string]$_.scope
}) | Sort-Object
$expectedDerivedDelegates = @(
    "DocumentStore::register_evaluation",
    "DocumentStore::register_exact_reference_evidence"
) | Sort-Object
$actualDerivedDelegates = @($d08.derived_result_delegates | ForEach-Object {
    if ($_.delegates_to -ne "DocumentStore::register_derived_result") {
        Fail-Guard "sole-mutation" "Derived-result delegate does not target the P07 gateway: $($_.scope)"
    }
    [string]$_.scope
}) | Sort-Object
if ($d08.ordinary_gateway.scope -ne "DocumentStore::apply_batch" -or
    (Compare-Object $expectedCanonicalDelegates $actualCanonicalDelegates) -or
    $d08.derived_result_gateway.scope -ne "DocumentStore::register_derived_result" -or
    $d08.derived_result_gateway.kind -ne "noncanonical_derived_result" -or
    @($d08.derived_result_gateway.required_envelope).Count -ne 3 -or
    @($d08.derived_result_gateway.allowed_payloads).Count -ne 2 -or
    @($d08.derived_result_gateway.forbidden_effects).Count -ne 4 -or
    (Compare-Object $expectedDerivedDelegates $actualDerivedDelegates) -or
    @($d08.dry_run_forbidden_effects).Count -ne 4) {
    Fail-Guard "sole-mutation" "D-08 canonical or P07 gateway contract changed without review."
}

$documentPath = Join-Path $RepoRoot "crates\ketchup-core\src\document.rs"
if (-not (Test-Path $documentPath -PathType Leaf)) {
    Fail-Guard "sole-mutation" "Missing canonical DocumentStore authority."
}
$documentSource = Get-Content $documentPath -Raw
$storeStruct = Get-BracedBlock $documentSource "pub struct DocumentStore" "sole-mutation"
if ($storeStruct -match '(?m)^\s*pub(?:\([^)]*\))?\s+[A-Za-z_][A-Za-z0-9_]*\s*:') {
    Fail-Guard "sole-mutation" "DocumentStore exposes mutable authority fields."
}
foreach ($requiredField in @(
    'revisions:\s*Vec<Arc<Revision>>',
    'cursor:\s*usize',
    'next_revision_id:\s*u64',
    'evaluation_registry:\s*BTreeMap<DerivedResultKey,\s*DerivedResultEvent>'
)) {
    if ($storeStruct -notmatch $requiredField) {
        Fail-Guard "sole-mutation" "DocumentStore authority fields changed shape or gained interior mutability."
    }
}
$inherentImplMarkers = [regex]::Matches($documentSource, '(?m)^\s*impl\s+DocumentStore\s*\{')
if ($inherentImplMarkers.Count -ne 1) {
    Fail-Guard "sole-mutation" "DocumentStore must have exactly one inherent implementation block."
}
$traitImpls = [regex]::Matches($documentSource, '(?m)^\s*impl\s+([^\r\n{]+)\s+for\s+DocumentStore\s*\{')
foreach ($traitImpl in $traitImpls) {
    if ($traitImpl.Groups[1].Value.Trim() -ne "Default") {
        Fail-Guard "sole-mutation" "Unexpected DocumentStore trait implementation: $($traitImpl.Groups[1].Value.Trim())"
    }
}
$storeImpl = Get-BracedBlock $documentSource "impl DocumentStore" "sole-mutation"
$publicMethodSignatures = [regex]::Matches(
    $storeImpl,
    '(?ms)^\s*pub(?:\([^)]*\))?\s+(?:(?:async|const|unsafe)\s+)*fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\((.*?)\)\s*(?:where\s+.*?)?(?:->[^\{]+)?\{'
)
foreach ($signature in $publicMethodSignatures) {
    $parameters = $signature.Groups[2].Value
    if ($parameters -cmatch "&(?:'[A-Za-z_][A-Za-z0-9_]*\s+)?mut\s+(?!self\b)[A-Za-z_][A-Za-z0-9_:<>]*") {
        Fail-Guard "sole-mutation" "Public associated mutator can bypass the canonical gateway: $($signature.Groups[1].Value)"
    }
}
$allowedMutableMethods = @(
    ([string]$d08.ordinary_gateway.scope).Split("::")[-1]
    @($d08.delegated_gateways | ForEach-Object { ([string]$_.scope).Split("::")[-1] })
    @($d08.derived_result_delegates | ForEach-Object { ([string]$_.scope).Split("::")[-1] })
    @($lifecycleExceptions | Where-Object { $_.scope -like "DocumentStore::*" } |
        ForEach-Object { ([string]$_.scope).Split("::")[-1] })
) | Sort-Object -Unique
$mutableMethods = [regex]::Matches(
    $storeImpl,
    '(?m)^\s*pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^)]*&mut\s+self'
) | ForEach-Object { $_.Groups[1].Value }
foreach ($method in $mutableMethods) {
    if ($method -notin $allowedMutableMethods) {
        Fail-Guard "sole-mutation" "Unexpected public DocumentStore mutator: $method"
    }
}
foreach ($required in $allowedMutableMethods) {
    if ($required -notin $mutableMethods) {
        Fail-Guard "sole-mutation" "Required canonical gateway or lifecycle exception is missing: $required"
    }
}
foreach ($delegate in $actualCanonicalDelegates) {
    $method = ([string]$delegate).Split("::")[-1]
    $declaration = @($d08.delegated_gateways | Where-Object { $_.scope -eq $delegate })[0]
    $targetMethod = ([string]$declaration.delegates_to).Split("::")[-1]
    $delegateBlock = Get-BracedBlock $storeImpl "pub fn $method" "sole-mutation"
    if (-not $delegateBlock.Contains(".$targetMethod(")) {
        Fail-Guard "sole-mutation" "Canonical delegate no longer calls its reviewed target: $delegate"
    }
}
foreach ($delegate in $actualDerivedDelegates) {
    $method = ([string]$delegate).Split("::")[-1]
    $delegateBlock = Get-BracedBlock $storeImpl "pub fn $method" "sole-mutation"
    if (-not $delegateBlock.Contains(".register_derived_result(")) {
        Fail-Guard "sole-mutation" "Derived-result delegate bypasses the P07 gateway: $delegate"
    }
}
$applyBlock = Get-BracedBlock $storeImpl "pub fn apply_batch" "sole-mutation"
$verifiedProposalBlock = Get-BracedBlock $storeImpl "pub fn commit_verified_proposal" "sole-mutation"
$derivedResultBlock = Get-BracedBlock $storeImpl "fn register_derived_result" "sole-mutation"
$graphValidation = $applyBlock.LastIndexOf("validate_graph(", [StringComparison]::Ordinal)
$productValidation = $applyBlock.LastIndexOf("validate_product(", [StringComparison]::Ordinal)
$revisionAppend = $applyBlock.IndexOf("self.revisions.push(", [StringComparison]::Ordinal)
if ($graphValidation -lt 0 -or $productValidation -lt 0 -or $revisionAppend -lt 0 -or
    $graphValidation -gt $revisionAppend -or $productValidation -gt $revisionAppend) {
    Fail-Guard "sole-mutation" "apply_batch must validate the complete candidate before revision append."
}
$validationBoundary = [Math]::Max($graphValidation, $productValidation)
$validatedTail = $applyBlock.Substring($validationBoundary, $revisionAppend - $validationBoundary)
if ($validatedTail -match '(?m)(?:&mut\s+(?:nodes|product)\b|\b(?:nodes|product)\s*(?:=|\.\s*(?:clear|insert|remove|append|extend|retain|entry|get_mut)\b))') {
    Fail-Guard "sole-mutation" "apply_batch mutates candidate canonical state after validation and before revision append."
}
foreach ($requiredRollbackLine in @(
    "let previous_revisions = self.revisions.clone()",
    "let previous_cursor = self.cursor",
    "let previous_next_revision_id = self.next_revision_id",
    "let previous_registry = self.evaluation_registry.clone()",
    ".apply_batch(&proposal.batch)",
    "self.revisions = previous_revisions",
    "self.cursor = previous_cursor",
    "self.next_revision_id = previous_next_revision_id",
    "self.evaluation_registry = previous_registry"
)) {
    if (-not $verifiedProposalBlock.Contains($requiredRollbackLine)) {
        Fail-Guard "sole-mutation" "Verified Proposal rollback no longer restores the complete pre-commit authority."
    }
}
if (-not $derivedResultBlock.Contains("event.document_id != current.snapshot.document_id()") -or
    -not $derivedResultBlock.Contains("event.revision_id != current.snapshot.revision_id()") -or
    -not $derivedResultBlock.Contains("event.canonical_digest != current.snapshot.canonical_digest()") -or
    -not $derivedResultBlock.Contains("key.document_id != event.document_id") -or
    -not $derivedResultBlock.Contains("key.revision_id != event.revision_id") -or
    -not $derivedResultBlock.Contains("reference.document_id != event.document_id") -or
    -not $derivedResultBlock.Contains("return false;") -or
    -not $derivedResultBlock.Contains("DerivedResultPayload::Evaluation") -or
    -not $derivedResultBlock.Contains("DerivedResultPayload::ExactReference") -or
    $derivedResultBlock -match 'next_revision_id|revisions\.push|self\.cursor\s*(?:=|\+=|-=)') {
    Fail-Guard "sole-mutation" "P07 derived-result gateway must remain envelope-bound, non-revisioned, and non-Undoable."
}
$fromProductBlock = Get-BracedBlock $storeImpl "pub(crate) fn from_product" "sole-mutation"
if ($storeImpl -match '(?m)^\s*pub\s+fn\s+from_product\b' -or
    $fromProductBlock.IndexOf("validate_graph(", [StringComparison]::Ordinal) -lt 0 -or
    $fromProductBlock.IndexOf("validate_product(", [StringComparison]::Ordinal) -lt 0 -or
    $fromProductBlock.IndexOf("Ok(Self", [StringComparison]::Ordinal) -lt 0 -or
    $fromProductBlock.IndexOf("validate_graph(", [StringComparison]::Ordinal) -gt $fromProductBlock.IndexOf("Ok(Self", [StringComparison]::Ordinal) -or
    $fromProductBlock.IndexOf("validate_product(", [StringComparison]::Ordinal) -gt $fromProductBlock.IndexOf("Ok(Self", [StringComparison]::Ordinal)) {
    Fail-Guard "sole-mutation" "from_product must remain crate-private and validate the complete candidate before construction."
}
$undoBlock = Get-BracedBlock $storeImpl "pub fn undo" "sole-mutation"
$redoBlock = Get-BracedBlock $storeImpl "pub fn redo" "sole-mutation"
$retentionBlock = Get-BracedBlock $storeImpl "pub fn discard_history_before_current" "sole-mutation"
$forbiddenLifecycleMutation = 'next_revision_id|\bnodes\b|\bproduct\b|apply_batch|revisions\.(?:truncate|insert|remove)'
if (-not $undoBlock.Contains("self.cursor -= 1") -or $undoBlock -match $forbiddenLifecycleMutation -or
    -not $redoBlock.Contains("self.cursor += 1") -or $redoBlock -match $forbiddenLifecycleMutation) {
    Fail-Guard "sole-mutation" "Undo/Redo must remain cursor-only selection of immutable revisions."
}
if (-not $retentionBlock.Contains("Arc::clone(&self.revisions[self.cursor])") -or
    -not $retentionBlock.Contains("self.revisions.clear()") -or
    -not $retentionBlock.Contains("self.revisions.push(current)") -or
    -not $retentionBlock.Contains("self.cursor = 0") -or
    $retentionBlock -match 'next_revision_id|\bnodes\b|\bproduct\b|apply_batch') {
    Fail-Guard "sole-mutation" "History discard must remain retention-only and preserve current entity content."
}
$unguardedStoreImpl = $storeImpl
foreach ($authorizedBlock in @(
    $applyBlock,
    $verifiedProposalBlock,
    $derivedResultBlock,
    $fromProductBlock,
    $undoBlock,
    $redoBlock,
    $retentionBlock
)) {
    $unguardedStoreImpl = $unguardedStoreImpl.Replace($authorizedBlock, "")
}
$authorityMutationPattern = '(?ms)(?:&mut\s+self\.(?:revisions|cursor|next_revision_id|evaluation_registry)\b|self\.(?:revisions|cursor|next_revision_id|evaluation_registry)(?:\s*\[[^\]]+\])?\s*(?:=|\+=|-=|\.\s*(?:clear|push|insert|remove|append|extend|retain|entry|get_mut)\s*\())'
if ($unguardedStoreImpl -match $authorityMutationPattern) {
    Fail-Guard "sole-mutation" "DocumentStore authority is mutated outside apply_batch, the P07 gateway, construction, or a reviewed lifecycle operation."
}

$sourceRoots = @(
    (Join-Path $RepoRoot "crates"),
    (Join-Path $RepoRoot "tests")
) | Where-Object { Test-Path $_ -PathType Container }
$rustFiles = @($sourceRoots | ForEach-Object { Get-ChildItem $_ -Filter "*.rs" -File -Recurse })
$legacyPattern = '\b(SceneBox|SceneHistoryEntry)\b|Vec\s*<\s*SceneBox\s*>'
foreach ($file in $rustFiles) {
    $source = Get-Content $file.FullName -Raw
    if ($source -match $legacyPattern) {
        $relative = Normalize-Path $file.FullName.Substring($RepoRoot.Length)
        Fail-Guard "legacy-absence" "Forbidden duplicate scene/history authority in $relative."
    }
}

$appPath = Join-Path $RepoRoot "crates\ketchup-app\src\lib.rs"
$appSource = if (Test-Path $appPath -PathType Leaf) { Get-Content $appPath -Raw } else { "" }
$newDocumentBlock = Get-BracedBlock $appSource "fn new_document" "sole-mutation"
$openDocumentBlock = Get-BracedBlock $appSource "fn open_document_from" "sole-mutation"
$loadCandidate = $openDocumentBlock.IndexOf("ketchup_core::persistence::load_file(path)", [StringComparison]::Ordinal)
$successBranch = $openDocumentBlock.IndexOf("Ok(outcome)", [StringComparison]::Ordinal)
$editableCandidate = $openDocumentBlock.IndexOf("outcome.into_editable()", [StringComparison]::Ordinal)
$historyBaseline = $openDocumentBlock.IndexOf("document.discard_history_before_current()", [StringComparison]::Ordinal)
$storeSwap = $openDocumentBlock.IndexOf("self.document = document", [StringComparison]::Ordinal)
$failureBranch = $openDocumentBlock.IndexOf("Err(error)", [StringComparison]::Ordinal)
if (-not $newDocumentBlock.Contains("*self = Self::new().with_dialogs(dialogs)") -or
    $loadCandidate -lt 0 -or $successBranch -lt $loadCandidate -or
    $editableCandidate -lt $successBranch -or $historyBaseline -lt $editableCandidate -or
    $storeSwap -lt $historyBaseline -or $failureBranch -lt $storeSwap -or
    ([regex]::Matches($openDocumentBlock, 'self\.document\s*=')).Count -ne 1) {
    Fail-Guard "sole-mutation" "New/Open must replace the active store only with a fresh or fully validated candidate; failed Open must not mutate it."
}
if ($appSource -match 'InteractionScene::new|\.add_occurrence\s*\(' -or
    $appSource -notmatch 'CanonicalInteractionProjection::from_snapshot') {
    Fail-Guard "projection-authority" "Production app scene construction must use the canonical interaction projection service exclusively."
}
$exactProjectionPath = Join-Path $RepoRoot "crates\ketchup-interaction\src\exact_projection.rs"
$exactProjectionSource = if (Test-Path $exactProjectionPath -PathType Leaf) {
    Get-Content $exactProjectionPath -Raw
} else {
    ""
}
$viewportBlock = Get-BracedBlock $appSource "fn viewport(" "exact-body-authority"
$viewportBoxesBlock = Get-BracedBlock $appSource "fn viewport_boxes" "exact-body-authority"
$exactExportBlock = Get-BracedBlock $appSource "pub fn export_exact_occurrence_mesh_to" "exact-body-authority"
if ($exactProjectionSource -notmatch 'package\.is_current\(snapshot\)' -or
    $exactProjectionSource -notmatch 'ray_triangle_distance' -or
    -not $viewportBoxesBlock.Contains('!exact_projection.contains_occurrence(&item.instance_path)') -or
    -not $viewportBlock.Contains('exact_projection.contains_occurrence(&occurrence.instance_path)') -or
    -not $viewportBlock.Contains('for triangle in package.triangles()') -or
    -not $exactExportBlock.Contains('.filter(|package| package.is_current(&snapshot))') -or
    -not $exactExportBlock.Contains('package.mesh_export(occurrence.transform)')) {
    Fail-Guard "exact-body-authority" "Current exact bodies must suppress box proxies and share triangle render, pick, and transformed export authority."
}
$stateViewPath = Join-Path $RepoRoot "crates\ketchup-core\src\state_view.rs"
$completeFixture = Join-Path $RepoRoot "crates\ketchup-core\tests\fixtures\state-view\complete-v1.txt"
$agentFixture = Join-Path $RepoRoot "crates\ketchup-core\tests\fixtures\state-view\agent-v1.txt"
if (-not (Test-Path $stateViewPath -PathType Leaf) -or
    -not (Test-Path $completeFixture -PathType Leaf) -or
    -not (Test-Path $agentFixture -PathType Leaf)) {
    Fail-Guard "state-view" "StateView encoder or independently versioned golden fixtures are missing."
}
$stateViewSource = Get-Content $stateViewPath -Raw
if ($stateViewSource -notmatch 'COMPLETE_STATE_VIEW_V1' -or
    $stateViewSource -notmatch 'AGENT_STATE_VIEW_V1' -or
    $stateViewSource -notmatch 'encode_semantic_state' -or
    (Get-Item $completeFixture).Length -eq 0 -or
    (Get-Item $agentFixture).Length -eq 0) {
    Fail-Guard "state-view" "Complete and agent StateView v1 must share one encoder and retain non-empty separate golden fixtures."
}

$exactManifestSource = Get-Content (Join-Path $RepoRoot "crates\ketchup-exact\Cargo.toml") -Raw
foreach ($target in @("gate_a0", "gate_a0_v2")) {
    $targetPattern = "(?ms)\[\[test\]\]\s*name\s*=\s*`"$target`"\s*required-features\s*=\s*\[`"a0-certification`"\]"
    if ($exactManifestSource -notmatch $targetPattern) {
        Fail-Guard "a0-separation" "Sealed target $target must require the explicit a0-certification feature."
    }
}
$ciGovernanceSource = Get-Content (Join-Path $RepoRoot "scripts\windows\invoke-ci-governance.ps1") -Raw
if ($ciGovernanceSource -notmatch 'cargo test --locked --workspace --all-targets' -or
    $ciGovernanceSource -match '(?m)--skip\s+gate_a0|validate-strengthened-a0-v[12]\.ps1') {
    Fail-Guard "a0-separation" "Daily governance must run the unfiltered product workspace and must not invoke sealed A0 validation."
}
foreach ($runner in @("run-strengthened-a0-v1.ps1", "run-strengthened-a0-v2.ps1")) {
    $runnerSource = Get-Content (Join-Path $RepoRoot "scripts\windows\$runner") -Raw
    if ($runnerSource -notmatch '(?:--features\s+a0-certification|"--features",\s*"a0-certification")') {
        Fail-Guard "a0-separation" "Explicit sealed runner $runner must enable a0-certification."
    }
}

$repoPrefix = $RepoRoot.TrimEnd("\") + "\"
if (-not [string]::IsNullOrWhiteSpace($FrozenLockPath)) {
    if (-not (Test-Path $FrozenLockPath -PathType Leaf)) {
        Fail-Guard "frozen-input" "Missing frozen-input lock: $FrozenLockPath"
    }
    $frozenLock = Get-Content $FrozenLockPath -Raw | ConvertFrom-Json
    if (-not ($frozenLock.PSObject.Properties.Name -contains "files") -or @($frozenLock.files).Count -eq 0) {
        Fail-Guard "frozen-input" "Frozen-input lock has no files."
    }
    foreach ($entry in @($frozenLock.files)) {
        $relative = Normalize-Path ([string]$entry.path)
        $fullPath = [IO.Path]::GetFullPath((Join-Path $RepoRoot $relative))
        if (-not $fullPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            Fail-Guard "frozen-input" "Locked path escapes the repository: $relative"
        }
        if (-not (Test-Path $fullPath -PathType Leaf)) {
            Fail-Guard "frozen-input" "Locked input is missing: $relative"
        }
        $item = Get-Item $fullPath -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            Fail-Guard "frozen-input" "Locked path traverses a reparse point: $relative"
        }
        $cursorPath = Split-Path $fullPath -Parent
        while (-not [string]::IsNullOrWhiteSpace($cursorPath) -and
            $cursorPath.StartsWith($RepoRoot, [StringComparison]::OrdinalIgnoreCase)) {
            $cursor = Get-Item $cursorPath -Force
            if ($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                Fail-Guard "frozen-input" "Locked path traverses a reparse point: $relative"
            }
            $parentPath = Split-Path $cursorPath -Parent
            if ($parentPath -eq $cursorPath) { break }
            $cursorPath = $parentPath
        }
        $actualHash = (Get-FileHash $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -ne ([string]$entry.sha256).ToLowerInvariant()) {
            Fail-Guard "frozen-input" "Locked input changed: $relative"
        }
    }
}

if (-not (Test-Path $ChangeManifestPath -PathType Leaf)) {
    Fail-Guard "anti-loosening" "Missing contract-change register."
}
$changeManifest = Get-Content $ChangeManifestPath -Raw | ConvertFrom-Json
if ($changeManifest.schema_version -ne 1 -or -not ($changeManifest.PSObject.Properties.Name -contains "changes")) {
    Fail-Guard "anti-loosening" "Unsupported contract-change register schema."
}

$effectiveChangedPaths = @()
if ($PSBoundParameters.ContainsKey("ChangedPaths")) {
    $effectiveChangedPaths = @($ChangedPaths | ForEach-Object { Normalize-Path $_ } | Sort-Object -Unique)
} elseif (-not [string]::IsNullOrWhiteSpace($BaseRef)) {
    $gitOutput = @(& git -C $RepoRoot diff --name-status --find-renames "$BaseRef...HEAD" 2>&1)
    if ($LASTEXITCODE -ne 0) {
        Fail-Guard "anti-loosening" "Cannot compare protected contracts with base ref $BaseRef`: $($gitOutput -join ' ')"
    }
    $changed = @()
    foreach ($line in $gitOutput) {
        $parts = ([string]$line) -split "`t"
        if ($parts.Count -lt 2) { continue }
        $changed += Normalize-Path $parts[1]
        if ($parts[0] -match '^[RC]' -and $parts.Count -ge 3) { $changed += Normalize-Path $parts[2] }
    }
    $effectiveChangedPaths = @($changed | Sort-Object -Unique)
}

$bootstrapMode = $false
if (-not [string]::IsNullOrWhiteSpace($BaseRef) -and
    ($changeManifest.PSObject.Properties.Name -contains "bootstrap_base_commit")) {
    $resolvedBase = @(& git -C $RepoRoot rev-parse $BaseRef 2>&1)
    if ($LASTEXITCODE -ne 0) { Fail-Guard "anti-loosening" "Cannot resolve bootstrap base ref $BaseRef." }
    $baseGovernancePaths = @(& git -C $RepoRoot ls-tree -r --name-only $BaseRef -- "governance/contract-changes.json")
    if ($LASTEXITCODE -ne 0) { Fail-Guard "anti-loosening" "Cannot inspect bootstrap base tree $BaseRef." }
    $baseHasGovernance = $baseGovernancePaths.Count -ne 0
    $bootstrapMode = -not $baseHasGovernance -and
        ([string]$resolvedBase[0]).Trim() -eq [string]$changeManifest.bootstrap_base_commit
}
$unfilteredChangedPaths = @($effectiveChangedPaths)
if ($bootstrapMode) {
    $bootstrapPaths = @($changeManifest.changes | ForEach-Object { Normalize-Path ([string]$_.path) } | Sort-Object -Unique)
    $effectiveChangedPaths = @($effectiveChangedPaths | Where-Object { $_ -in $bootstrapPaths })
}

$protectedPatterns = @(
    '^thresholds/',
    '^corpora/',
    '^crates/ketchup-exact/tests/upper_envelope\.rs$',
    '^artifacts/.+(?:lock|preregistration).*\.json$',
    '^artifacts/(?:gate-a0|gate-a1|gate-b|gate-c)/runs/',
    '^docs/design/EXECUTION_CONTRACT\.md$',
    '^scripts/windows/(?:run|validate|verify|write)-(?:gate|strengthened|r0)',
    '^scripts/windows/(?:test-architecture-guards|test-ci-guard-red-paths|invoke-ci-governance)\.ps1$',
    '^governance/',
    '^\.github/workflows/ci\.yml$',
    '^\.github/CODEOWNERS$'
)
$unfilteredProtectedPaths = @($unfilteredChangedPaths | Where-Object {
    $candidate = $_
    @($protectedPatterns | Where-Object { $candidate -match $_ }).Count -gt 0
})
if ($bootstrapMode) {
    $unclassifiedBootstrapPaths = @($unfilteredProtectedPaths | Where-Object { $_ -notin $bootstrapPaths })
    if ($unclassifiedBootstrapPaths.Count -gt 0) {
        Fail-Guard "anti-loosening" "Bootstrap contains unclassified protected changes: $($unclassifiedBootstrapPaths -join ', ')"
    }
}
$protectedChangedPaths = @($effectiveChangedPaths | Where-Object {
    $candidate = $_
    @($protectedPatterns | Where-Object { $candidate -match $_ }).Count -gt 0
})
$records = @($changeManifest.changes)
foreach ($path in $protectedChangedPaths) {
    $matching = @($records | Where-Object { (Normalize-Path ([string]$_.path)) -eq $path })
    if ($matching.Count -ne 1) {
        Fail-Guard "anti-loosening" "Protected contract change needs exactly one classification record: $path"
    }
    $record = $matching[0]
    $direction = ([string]$record.direction).ToLowerInvariant()
    if ($direction -notin @("tighten", "loosen", "neutral")) {
        Fail-Guard "anti-loosening" "Direction must be tighten, loosen, or neutral for $path; unknown is fail-closed."
    }
    foreach ($field in @("old_freeze_id", "new_freeze_id", "evidence")) {
        if (-not ($record.PSObject.Properties.Name -contains $field) -or [string]::IsNullOrWhiteSpace([string]$record.$field)) {
            Fail-Guard "anti-loosening" "Missing $field for $path."
        }
    }
    if ([string]$record.old_freeze_id -eq [string]$record.new_freeze_id) {
        Fail-Guard "anti-loosening" "A protected change must issue a new freeze ID: $path"
    }
    if ($path -ne "governance/contract-changes.json") {
        if (-not ($record.PSObject.Properties.Name -contains "new_sha256") -or
            ([string]$record.new_sha256).ToLowerInvariant() -notmatch '^[0-9a-f]{64}$') {
            Fail-Guard "anti-loosening" "Protected change must bind the exact reviewed file hash: $path"
        }
        $reviewedPath = [IO.Path]::GetFullPath((Join-Path $RepoRoot $path))
        if (-not $reviewedPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path $reviewedPath -PathType Leaf) -or
            (Get-FileHash $reviewedPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne ([string]$record.new_sha256).ToLowerInvariant()) {
            Fail-Guard "anti-loosening" "Protected file no longer matches its reviewed classification hash: $path"
        }
    } elseif (-not $bootstrapMode -and -not [string]::IsNullOrWhiteSpace($BaseRef)) {
        $baseManifestText = @(& git -C $RepoRoot show "$BaseRef`:governance/contract-changes.json" 2>&1)
        if ($LASTEXITCODE -ne 0) { Fail-Guard "anti-loosening" "Cannot read the base contract-change register." }
        $baseManifest = ($baseManifestText -join "`n") | ConvertFrom-Json
        $baseRecord = @($baseManifest.changes | Where-Object { (Normalize-Path ([string]$_.path)) -eq $path })
        if ($baseRecord.Count -eq 1 -and
            [string]$baseRecord[0].new_freeze_id -eq [string]$record.new_freeze_id -and
            [string]$baseRecord[0].evidence -eq [string]$record.evidence) {
            Fail-Guard "anti-loosening" "The contract-change register changed without issuing its own new reviewed freeze."
        }
    }
    $evidencePath = [IO.Path]::GetFullPath((Join-Path $RepoRoot (Normalize-Path ([string]$record.evidence))))
    if (-not $evidencePath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path $evidencePath -PathType Leaf)) {
        Fail-Guard "anti-loosening" "Evidence must name an existing repository file: $path"
    }
    if ($path -eq "thresholds/r0.yaml" -and -not [string]::IsNullOrWhiteSpace($BaseRef)) {
        $computedDirection = Get-ThresholdDirection $BaseRef $RepoRoot
        if ($computedDirection -eq "unknown") {
            Fail-Guard "anti-loosening" "Threshold or operation-envelope semantics are not mechanically classifiable; fail closed."
        }
        if ($computedDirection -ne $direction) {
            Fail-Guard "anti-loosening" "Declared direction $direction disagrees with computed direction $computedDirection for $path."
        }
    }
    if ($direction -eq "loosen") {
        foreach ($field in @("approval", "upper_envelope_evidence")) {
            if (-not ($record.PSObject.Properties.Name -contains $field) -or [string]::IsNullOrWhiteSpace([string]$record.$field)) {
                Fail-Guard "anti-loosening" "Loosening $path requires explicit $field."
            }
        }
        if ([string]$record.approval -ne "project-owner") {
            Fail-Guard "anti-loosening" "Loosening $path requires project-owner approval."
        }
        $upperEvidencePath = [IO.Path]::GetFullPath((Join-Path $RepoRoot (Normalize-Path ([string]$record.upper_envelope_evidence))))
        if (-not $upperEvidencePath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path $upperEvidencePath -PathType Leaf)) {
            Fail-Guard "anti-loosening" "Loosening $path requires existing upper-envelope evidence."
        }
    }
}

Write-Output "Architecture guards passed: sole mutation, legacy absence, projection authority, StateView fixtures, sealed A0 separation, optional frozen inputs, and anti-loosening governance."
