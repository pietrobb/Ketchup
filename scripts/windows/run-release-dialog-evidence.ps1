[CmdletBinding()]
param(
    [string]$PackageDir,
    [string]$EvidenceDir,
    [string]$RunId,
    [string]$OperatorName,
    [switch]$VerifyOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$packager = Join-Path $PSScriptRoot "build-release-candidate.ps1"
if ([string]::IsNullOrWhiteSpace($PackageDir)) {
    $PackageDir = Join-Path $repoRoot "artifacts\m19\release-candidate\windows-x86_64"
}
$PackageDir = [IO.Path]::GetFullPath($PackageDir)
$platformDecisionRecordPath = Join-Path $repoRoot "docs\adr\0007-windows-x86-64-first-release.md"
$expectedPlatformDecisionRecordSha256 = "cb91dbd3f8d2b96f7edb5f1f1eae01c49acf2846f85aeed5546c44c79ac5dc62"
$expectedOcctManifestSha256 = "1212a72954ed503a6b06618b2813b1d7c04f5b422329d2327594268e431ef48a"
$maxEvidenceManifestBytes = 1024 * 1024
$maxProvenanceManifestBytes = 256 * 1024
if (-not [string]::IsNullOrWhiteSpace($EvidenceDir)) {
    $EvidenceDir = [IO.Path]::GetFullPath($EvidenceDir)
}

$artifactNames = @(
    "package-manifest.json",
    "ketchup-app.exe",
    "ketchup-exact-worker.exe",
    "baseline.ketchup",
    "new-document.ketchup",
    "roundtrip.ketchup",
    "after-failed-open.ketchup",
    "after-failed-save.ketchup",
    "corrupt.ketchup",
    "locked-target.ketchup"
)
$requiredStepIds = @(
    "save-as-success",
    "save-existing-success",
    "new-save-success",
    "open-roundtrip-success",
    "failed-open-continuity",
    "failed-save-continuity"
)
$requiredStepAttestations = @(
    "Created a visible Rectangle-to-Push/Pull model and saved it through the observed native Save As dialog.",
    "Saved the existing baseline path while preserving its exact canonical bytes.",
    "Created a fresh New document and saved it through the observed native Save dialog.",
    "Opened the baseline through the observed native Open dialog and saved a byte-identical round-trip.",
    "Selected the frozen malformed Open input, observed its rejection, and preserved the active baseline document.",
    "Selected the exclusively locked Save target, observed the Save failure, and preserved the active baseline document."
)
$requiredNativeDialogIds = @(
    "baseline-save-as",
    "new-document-save",
    "baseline-open",
    "roundtrip-save-as",
    "malformed-open",
    "post-failed-open-save-as",
    "locked-target-save-as",
    "post-failed-save-save-as"
)
$nativeDialogStepIds = @(
    "save-as-success",
    "new-save-success",
    "open-roundtrip-success",
    "open-roundtrip-success",
    "failed-open-continuity",
    "failed-open-continuity",
    "failed-save-continuity",
    "failed-save-continuity"
)
$requiredSaveOutputIds = @(
    "baseline-save-as-output",
    "new-document-save-output",
    "roundtrip-save-as-output",
    "post-failed-open-save-as-output",
    "post-failed-save-save-as-output"
)
$saveOutputArtifacts = @(
    "baseline.ketchup",
    "new-document.ketchup",
    "roundtrip.ketchup",
    "after-failed-open.ketchup",
    "after-failed-save.ketchup"
)
$saveOutputDialogIds = @(
    "baseline-save-as",
    "new-document-save",
    "roundtrip-save-as",
    "post-failed-open-save-as",
    "post-failed-save-save-as"
)
$saveOutputStepIds = @(
    "save-as-success",
    "new-save-success",
    "open-roundtrip-success",
    "failed-open-continuity",
    "failed-save-continuity"
)
$requiredFailureInputIds = @(
    "malformed-open-input",
    "locked-save-input"
)
$failureInputArtifacts = @(
    "corrupt.ketchup",
    "locked-target.ketchup"
)
$failureInputDialogIds = @(
    "malformed-open",
    "locked-target-save-as"
)
$failureInputExclusiveLocks = @($false, $true)

function Get-Sha256([string]$Path) {
    return (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-StreamSha256([IO.Stream]$Stream) {
    $position = $Stream.Position
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $Stream.Position = 0
        return ([BitConverter]::ToString($sha256.ComputeHash($Stream))).Replace("-", "").ToLowerInvariant()
    } finally {
        $Stream.Position = $position
        $sha256.Dispose()
    }
}

function Get-BytesSha256([byte[]]$Bytes) {
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha256.Dispose()
    }
}

function Assert-Leaf([string]$Path, [string]$Label) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "Missing $Label`: $Path" }
}

function Read-BoundedJsonStream([IO.Stream]$Stream, [int64]$MaxBytes, [string]$Label, [object]$Sha256 = $null) {
    if ($Stream.Length -le 0 -or $Stream.Length -gt $MaxBytes) {
        throw "$Label exceeds its $MaxBytes-byte limit."
    }
    $Stream.Position = 0
    $bytes = [byte[]]::new([int]$Stream.Length)
    $offset = 0
    while ($offset -lt $bytes.Length) {
        $read = $Stream.Read($bytes, $offset, $bytes.Length - $offset)
        if ($read -eq 0) { throw "$Label ended before its declared length." }
        $offset += $read
    }
    if ($null -ne $Sha256) {
        $hasher = [Security.Cryptography.SHA256]::Create()
        try {
            $Sha256.Value = -join ($hasher.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") })
        } finally {
            $hasher.Dispose()
        }
    }
    try {
        $json = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    } catch {
        throw "$Label is not valid UTF-8."
    }
    try {
        return ($json | ConvertFrom-Json)
    } catch {
        throw "$Label is not valid JSON."
    }
}

function Read-BoundedJson([string]$Path, [int64]$MaxBytes, [string]$Label, [object]$Sha256 = $null) {
    Assert-Leaf $Path $Label
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        return Read-BoundedJsonStream $stream $MaxBytes $Label $Sha256
    } finally {
        $stream.Dispose()
    }
}

function Assert-UtcTimestamp([string]$Value, [string]$Label) {
    $parsed = [DateTimeOffset]::MinValue
    if ($Value -cnotmatch 'Z$' -or
        -not [DateTimeOffset]::TryParse(
            $Value,
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind,
            [ref]$parsed
        )) {
        throw "$Label is not a valid UTC timestamp."
    }
    return $parsed
}

function Write-Utf8JsonExclusive([string]$Path, [object]$Value) {
    $json = $Value | ConvertTo-Json -Depth 12
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json + "`n")
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Dispose() }
}

function Assert-ExactNames([string[]]$Actual, [string[]]$Expected, [string]$Label) {
    $actualSorted = @($Actual | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    if (($actualSorted -join "|") -cne ($expectedSorted -join "|")) {
        throw "$Label differs from its exact allowlist. Expected $($expectedSorted -join ', '); got $($actualSorted -join ', ')."
    }
}

function Assert-ExactProperties([object]$Value, [string[]]$Expected, [string]$Label) {
    if ($null -eq $Value) { throw "$Label is missing." }
    Assert-ExactNames @($Value.PSObject.Properties | ForEach-Object { $_.Name }) $Expected "$Label properties"
}

function Read-BoundedUtf8String([byte[]]$Bytes, [ref]$Offset, [int]$End, [string]$Label) {
    if ($Offset.Value + 4 -gt $End) { throw "$Label has a truncated string length." }
    $length = [BitConverter]::ToUInt32($Bytes, $Offset.Value)
    $Offset.Value += 4
    if ($length -gt 1024 -or $Offset.Value + [int64]$length -gt $End) {
        throw "$Label has an invalid string length."
    }
    $strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    $value = $strictUtf8.GetString($Bytes, $Offset.Value, [int]$length)
    $Offset.Value += [int]$length
    return $value
}

function Assert-PackageManifestProvenance([object]$PackageManifest) {
    Assert-ExactProperties $PackageManifest @(
        "schema_version", "kind", "platform", "platform_decision", "platform_decision_record",
        "platform_decision_record_sha256", "release_eligible", "release_blockers", "cargo_lock_sha256", "occt", "files"
    ) "Captured package manifest"
    Assert-ExactProperties $PackageManifest.occt @(
        "version", "source_commit", "manifest_sha256", "build_fingerprint", "runtime_dll_count"
    ) "Captured package OCCT provenance"
    $lockPath = Join-Path $repoRoot "Cargo.lock"
    $occtManifestPath = Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json"
    Assert-Leaf $platformDecisionRecordPath "accepted Windows-first platform-decision record"
    if ((Get-Sha256 $platformDecisionRecordPath) -cne $expectedPlatformDecisionRecordSha256) {
        throw "The accepted Windows-first platform-decision record differs from its immutable baseline."
    }
    Assert-Leaf $lockPath "Cargo lockfile"
    Assert-Leaf $occtManifestPath "pinned R0 OCCT manifest"
    $pinnedOcctHash = ""
    $pinnedOcct = Read-BoundedJson $occtManifestPath $maxProvenanceManifestBytes "pinned R0 OCCT manifest" ([ref]$pinnedOcctHash)
    if ($pinnedOcctHash -cne $expectedOcctManifestSha256) {
        throw "The current OCCT manifest differs from the immutable R0 preregistration baseline."
    }
    if ($pinnedOcct.schema_version -ne 1 -or
        $pinnedOcct.status -ne "built-and-fingerprinted" -or
        $pinnedOcct.build.platform -ne "windows-x86_64" -or
        $pinnedOcct.build.configuration -ne "Release" -or
        $pinnedOcct.source.commit -ne "b8f597c677811d1f9f4d8a97f5ae2825c0353a42" -or
        -not $pinnedOcct.source.clean) {
        throw "The current R0 OCCT manifest is not the pinned Windows Release baseline."
    }
    $pinnedRecords = @($pinnedOcct.shared_libraries)
    if ($PackageManifest.schema_version -ne 1 -or
        $PackageManifest.kind -ne "technical-release-candidate" -or
        $PackageManifest.platform -ne "windows-x86_64" -or
        $PackageManifest.platform_decision -cne "windows-x86_64-first-release" -or
        $PackageManifest.platform_decision_record -cne "docs/adr/0007-windows-x86-64-first-release.md" -or
        [string]$PackageManifest.platform_decision_record_sha256 -cne $expectedPlatformDecisionRecordSha256 -or
        $PackageManifest.release_eligible -ne $false -or
        [string]::Join("|", @($PackageManifest.release_blockers)) -cne "G19-02-physical-dialog-workflow|G19-03-canonical-tasks|G19-04-current-tree-hardware-certification" -or
        [string]$PackageManifest.cargo_lock_sha256 -cne (Get-Sha256 $lockPath) -or
        [string]$PackageManifest.occt.version -cne [string]$pinnedOcct.source.release -or
        [string]$PackageManifest.occt.source_commit -cne [string]$pinnedOcct.source.commit -or
        [string]$PackageManifest.occt.manifest_sha256 -cne $expectedOcctManifestSha256 -or
        [string]$PackageManifest.occt.build_fingerprint -cne "occt-8.0.1:b8f597c677811d1f9f4d8a97f5ae2825c0353a42:r0-v1" -or
        [int]$PackageManifest.occt.runtime_dll_count -ne $pinnedRecords.Count) {
        throw "The captured package manifest is not bound to the current Cargo.lock and pinned R0 OCCT baseline."
    }

    $expected = @{
        "ketchup-app.exe" = [ordered]@{ role = "desktop-application"; pinned = $null }
        "ketchup-exact-worker.exe" = [ordered]@{ role = "exact-worker"; pinned = $null }
    }
    foreach ($record in $pinnedRecords) {
        $name = [IO.Path]::GetFileName([string]$record.path)
        if ($expected.ContainsKey($name)) { throw "Duplicate pinned package entry: $name" }
        $expected[$name] = [ordered]@{ role = "pinned-occt-runtime"; pinned = $record }
    }
    $files = @($PackageManifest.files)
    Assert-ExactNames @($files | ForEach-Object { [string]$_.name }) @($expected.Keys) "Captured package entries"
    foreach ($record in $files) {
        Assert-ExactProperties $record @("name", "role", "size_bytes", "sha256") "Captured package file record"
        $name = [string]$record.name
        $expectedRecord = $expected[$name]
        if ($name -notmatch '^[A-Za-z0-9._-]+$' -or
            [string]$record.role -cne [string]$expectedRecord.role -or
            [int64]$record.size_bytes -le 0 -or
            [string]$record.sha256 -cnotmatch '^[0-9a-f]{64}$') {
            throw "Unsafe or invalid captured package record: $name"
        }
        if ($null -ne $expectedRecord.pinned -and
            ([int64]$record.size_bytes -ne [int64]$expectedRecord.pinned.size_bytes -or
             [string]$record.sha256 -cne [string]$expectedRecord.pinned.sha256)) {
            throw "Captured package DLL provenance differs from pinned R0: $name"
        }
    }
}

function Assert-NativeKetchupContainer([IO.Stream]$Stream, [string]$Label) {
    if ($Stream.Length -lt 16 -or $Stream.Length -gt (64 * 1024 * 1024)) {
        throw "$Label is outside the native container size envelope."
    }
    $Stream.Position = 0
    $bytes = [byte[]]::new([int]$Stream.Length)
    $offset = 0
    while ($offset -lt $bytes.Length) {
        $read = $Stream.Read($bytes, $offset, $bytes.Length - $offset)
        if ($read -eq 0) { throw "$Label ended before its declared length." }
        $offset += $read
    }
    $containerMagic = [Text.Encoding]::ASCII.GetString($bytes, 0, 10)
    $containerSchema = [BitConverter]::ToUInt16($bytes, 10)
    $entryCount = [BitConverter]::ToUInt32($bytes, 12)
    if ($containerMagic -cne "KETCHUPCTR" -or $containerSchema -ne 1 -or $entryCount -eq 0 -or $entryCount -gt 4096) {
        throw "$Label is not a supported native Ketchup container."
    }

    $offset = 16
    $paths = @{}
    $documentBytes = $null
    $strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    for ($entryIndex = 0; $entryIndex -lt $entryCount; $entryIndex++) {
        if ($offset + 4 -gt $bytes.Length) { throw "$Label has a truncated container entry path length." }
        $pathLength = [BitConverter]::ToUInt32($bytes, $offset)
        $offset += 4
        if ($pathLength -eq 0 -or $pathLength -gt 1024 -or $offset + $pathLength + 41 -gt $bytes.Length) {
            throw "$Label has an invalid container entry path."
        }
        $entryPath = $strictUtf8.GetString($bytes, $offset, $pathLength)
        $offset += $pathLength
        if ($paths.ContainsKey($entryPath)) { throw "$Label has a duplicate container entry: $entryPath" }
        $paths[$entryPath] = $true
        $required = $bytes[$offset]
        $offset += 1
        if ($required -notin @(0, 1)) { throw "$Label has an invalid required-entry flag: $entryPath" }
        $payloadLength = [BitConverter]::ToUInt64($bytes, $offset)
        $offset += 8
        $expectedHash = [byte[]]::new(32)
        [Array]::Copy($bytes, $offset, $expectedHash, 0, 32)
        $offset += 32
        if ($payloadLength -gt [int]::MaxValue -or $offset + [int64]$payloadLength -gt $bytes.Length) {
            throw "$Label has an invalid container payload length: $entryPath"
        }
        $payload = [byte[]]::new([int]$payloadLength)
        [Array]::Copy($bytes, $offset, $payload, 0, [int]$payloadLength)
        $offset += [int]$payloadLength
        $hasher = [Security.Cryptography.SHA256]::Create()
        try { $actualHash = $hasher.ComputeHash($payload) } finally { $hasher.Dispose() }
        if ([Convert]::ToBase64String($actualHash) -cne [Convert]::ToBase64String($expectedHash)) {
            throw "$Label has a changed container payload: $entryPath"
        }
        if ($entryPath -ceq "document.bin") {
            if ($required -ne 1 -or $null -ne $documentBytes) {
                throw "$Label lacks one unique required native document entry."
            }
            $documentBytes = $payload
        }
    }
    if ($offset -ne $bytes.Length -or $null -eq $documentBytes) {
        throw "$Label has trailing bytes or lacks its required native document entry."
    }
    if ($documentBytes.Length -lt 56 -or
        [Text.Encoding]::ASCII.GetString($documentBytes, 0, 10) -cne "KETCHUPDOC" -or
        [BitConverter]::ToUInt16($documentBytes, 10) -ne 17) {
        throw "$Label does not contain a current schema-17 Ketchup document."
    }
    $manifestLength = [BitConverter]::ToUInt32($documentBytes, 12)
    $expectedManifestIds = @(
        "ketchup.graph.schema.v1",
        "ketchup.evaluator.numeric.v1",
        "ketchup.tolerance.r0-v1"
    )
    $expectedManifestLength = 40
    foreach ($id in $expectedManifestIds) {
        $expectedManifestLength += 4 + [Text.Encoding]::UTF8.GetByteCount($id)
    }
    if ($manifestLength -ne $expectedManifestLength -or 16 + [int64]$manifestLength -gt $documentBytes.Length) {
        throw "$Label has an invalid native document manifest length."
    }
    $canonicalPayloadLength = [BitConverter]::ToUInt64($documentBytes, 16)
    $canonicalPayloadOffset = 16 + [int]$manifestLength
    if ($canonicalPayloadLength -gt [int]::MaxValue -or
        $canonicalPayloadOffset + [int64]$canonicalPayloadLength -ne $documentBytes.Length) {
        throw "$Label has an invalid native document payload length."
    }
    $canonicalPayload = [byte[]]::new([int]$canonicalPayloadLength)
    [Array]::Copy($documentBytes, $canonicalPayloadOffset, $canonicalPayload, 0, [int]$canonicalPayloadLength)
    $expectedCanonicalHash = [byte[]]::new(32)
    [Array]::Copy($documentBytes, 24, $expectedCanonicalHash, 0, 32)
    $canonicalHasher = [Security.Cryptography.SHA256]::Create()
    try { $actualCanonicalHash = $canonicalHasher.ComputeHash($canonicalPayload) } finally { $canonicalHasher.Dispose() }
    if ([Convert]::ToBase64String($actualCanonicalHash) -cne [Convert]::ToBase64String($expectedCanonicalHash)) {
        throw "$Label has a changed native canonical payload."
    }
    $manifestOffset = 56
    foreach ($expectedId in $expectedManifestIds) {
        $actualId = Read-BoundedUtf8String $documentBytes ([ref]$manifestOffset) $canonicalPayloadOffset $Label
        if ($actualId -cne $expectedId) {
            throw "$Label has an unexpected native document manifest identifier."
        }
    }
    if ($manifestOffset -ne $canonicalPayloadOffset) {
        throw "$Label has trailing native document manifest bytes."
    }
    return -join ($actualCanonicalHash | ForEach-Object { $_.ToString("x2") })
}

function Invoke-PackagedDocumentInspection([string]$ExecutablePath, [string]$DocumentPath, [string]$Label) {
    $output = @(& $ExecutablePath --inspect-native-document $DocumentPath 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "Packaged app could not inspect $Label`: $($output -join [Environment]::NewLine)"
    }
    try {
        return (($output -join "") | ConvertFrom-Json)
    } catch {
        throw "Packaged app returned invalid semantic inspection for $Label."
    }
}

function Invoke-ExactWorkerPing([string]$ExecutablePath, [string]$Label) {
    $output = @("PING" | & $ExecutablePath 2>&1)
    $exitCode = $LASTEXITCODE
    $response = ($output -join [Environment]::NewLine).Trim()
    if ($exitCode -ne 0 -or $response -cne "PONG") {
        throw "$Label failed its exact PING/PONG process-boundary probe."
    }
}

function Verify-Evidence([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw "-EvidenceDir is required with -VerifyOnly." }
    if (-not (Test-Path $Path -PathType Container)) { throw "Missing evidence directory: $Path" }
    $manifestPath = Join-Path $Path "evidence-manifest.json"
    Assert-Leaf $manifestPath "release-dialog evidence manifest"
    $manifestHash = ""
    $manifest = Read-BoundedJson $manifestPath $maxEvidenceManifestBytes "release-dialog evidence manifest" ([ref]$manifestHash)
    Assert-ExactProperties $manifest @(
        "schema_version", "kind", "status", "platform", "capture_mode", "run_id", "captured_utc",
        "physical_hardware_evidence_complete", "platform_decision", "platform_decision_record",
        "platform_decision_record_sha256", "release_eligible", "release_blockers", "operator", "machine", "package_manifest_sha256", "runner_sha256", "runner_observation",
        "package_snapshot", "exact_worker_probe", "desktop_process", "native_file_dialogs", "save_output_creations", "failure_input_observations", "loaded_pinned_occt_modules", "document_semantics", "save_existing_rewrite", "steps", "artifacts"
    ) "Physical release-dialog evidence manifest"
    Assert-ExactProperties $manifest.operator @(
        "name", "windows_account", "windows_sid", "session_id", "attested_physical_interaction"
    ) "Physical operator attestation"
    Assert-ExactProperties $manifest.machine @(
        "manufacturer", "model", "total_physical_memory_bytes", "os", "os_build", "gpus"
    ) "Physical machine identity"
    Assert-ExactProperties $manifest.runner_observation @(
        "before_sha256", "before_observed_utc", "after_sha256", "after_observed_utc"
    ) "Evidence runner byte observation"
    Assert-ExactProperties $manifest.package_snapshot @(
        "isolated_copy", "guard_mode", "guarded_files", "guards_acquired_utc", "guards_released_utc",
        "verified_before_launch_utc", "verified_under_guard_utc", "runtime_modules_verified_before_workflow_utc",
        "runtime_modules_verified_after_workflow_utc", "verified_after_exit_utc"
    ) "Isolated package snapshot"
    Assert-ExactProperties $manifest.exact_worker_probe @(
        "request", "response", "exit_code", "observed_utc"
    ) "Exact-worker process probe"
    Assert-ExactProperties $manifest.desktop_process @(
        "package_relative_executable", "process_id", "session_id", "load_origin", "working_directory_origin",
        "image_path_matches_verified_package", "image_size_bytes", "image_sha256",
        "process_machine", "native_machine", "native_amd64_execution",
        "main_window_observation_started_utc", "main_window_wait_elapsed_ms", "main_window_observed_utc",
        "main_window_stability_rechecked_utc", "main_window_same_handle_stable", "main_window_exists", "main_window_visible", "main_window_process_id",
        "main_window_observed", "main_window_title", "main_window_handle", "observed_started_utc", "observed_exited_utc"
    ) "Packaged desktop process"
    Assert-ExactProperties $manifest.document_semantics @("baseline", "new_document") "Packaged-app document semantics"
    Assert-ExactProperties $manifest.save_existing_rewrite @(
        "artifact", "before_observed_utc", "before_last_write_utc", "before_size_bytes", "before_sha256", "before_file_identity",
        "after_last_write_utc", "after_size_bytes", "after_sha256", "after_file_identity", "after_observed_utc"
    ) "Existing-document Save rewrite evidence"
    foreach ($name in @("baseline", "new_document")) {
        Assert-ExactProperties $manifest.document_semantics.$name @(
            "schema_version", "document_id", "revision", "canonical_digest", "container_sha256", "definitions",
            "root_occurrences", "profiles", "extrusions", "profile_extrusion_definitions",
            "visible_profile_extrusion_root_occurrences"
        ) "Packaged-app $name document semantics"
    }
    if ($manifest.schema_version -ne 1 -or
        $manifest.kind -ne "physical-release-dialog-evidence" -or
        $manifest.status -ne "PASS" -or
        $manifest.platform -ne "windows-x86_64" -or
        $manifest.capture_mode -ne "interactive-physical-operator" -or
        $manifest.physical_hardware_evidence_complete -ne $true -or
        $manifest.platform_decision -cne "windows-x86_64-first-release" -or
        $manifest.platform_decision_record -cne "docs/adr/0007-windows-x86-64-first-release.md" -or
        [string]$manifest.platform_decision_record_sha256 -cne $expectedPlatformDecisionRecordSha256 -or
        $manifest.release_eligible -ne $false -or
        [string]::Join("|", @($manifest.release_blockers)) -cne "G19-03-canonical-tasks|G19-04-current-tree-hardware-certification") {
        throw "Evidence does not match the accepted Windows-first decision and remaining M19 release blockers."
    }
    if ([string]$manifest.run_id -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]{2,63}$' -or
        [string]::IsNullOrWhiteSpace([string]$manifest.operator.name) -or
        [string]$manifest.operator.name -match '[\r\n]' -or
        [string]$manifest.operator.windows_account -cnotmatch '^[^\\\r\n]+\\[^\\\r\n]+$' -or
        [string]$manifest.operator.windows_sid -cnotmatch '^S-[0-9]+-[0-9]+(?:-[0-9]+)+$' -or
        [int]$manifest.operator.session_id -le 0 -or
        $manifest.operator.attested_physical_interaction -ne $true) {
        throw "Evidence lacks a valid run identity and Windows-session-bound named physical operator attestation."
    }
    $capturedUtc = Assert-UtcTimestamp ([string]$manifest.captured_utc) "Evidence capture time"
    $runnerBeforeObservedUtc = Assert-UtcTimestamp ([string]$manifest.runner_observation.before_observed_utc) "Evidence runner pre-workflow observation time"
    $runnerAfterObservedUtc = Assert-UtcTimestamp ([string]$manifest.runner_observation.after_observed_utc) "Evidence runner post-workflow observation time"
    $currentRunnerSha256 = Get-Sha256 $PSCommandPath
    if ([string]$manifest.runner_sha256 -cne $currentRunnerSha256 -or
        [string]$manifest.runner_observation.before_sha256 -cne $currentRunnerSha256 -or
        [string]$manifest.runner_observation.after_sha256 -cne $currentRunnerSha256) {
        throw "Evidence runner bytes do not match both workflow-bound observations and the current verifier."
    }
    if ($manifest.package_snapshot.isolated_copy -ne $true -or
        [string]$manifest.package_snapshot.guard_mode -cne "read-handles-deny-write-delete") {
        throw "Evidence was not executed from a write/delete-guarded isolated technical-package snapshot."
    }
    $packageGuardsAcquiredUtc = Assert-UtcTimestamp ([string]$manifest.package_snapshot.guards_acquired_utc) "Package write/delete guard acquisition time"
    $packageGuardsReleasedUtc = Assert-UtcTimestamp ([string]$manifest.package_snapshot.guards_released_utc) "Package write/delete guard release time"
    $packageVerifiedBeforeLaunchUtc = Assert-UtcTimestamp ([string]$manifest.package_snapshot.verified_before_launch_utc) "Pre-launch package verification time"
    $packageVerifiedUnderGuardUtc = Assert-UtcTimestamp ([string]$manifest.package_snapshot.verified_under_guard_utc) "Guarded package verification time"
    $runtimeModulesVerifiedBeforeWorkflowUtc = Assert-UtcTimestamp ([string]$manifest.package_snapshot.runtime_modules_verified_before_workflow_utc) "Pre-workflow runtime-module verification time"
    $runtimeModulesVerifiedAfterWorkflowUtc = Assert-UtcTimestamp ([string]$manifest.package_snapshot.runtime_modules_verified_after_workflow_utc) "Post-workflow runtime-module verification time"
    $packageVerifiedAfterExitUtc = Assert-UtcTimestamp ([string]$manifest.package_snapshot.verified_after_exit_utc) "Post-run package verification time"
    if ($runnerBeforeObservedUtc -ge $packageVerifiedBeforeLaunchUtc -or
        $packageVerifiedAfterExitUtc -ge $runnerAfterObservedUtc -or
        $runnerAfterObservedUtc -gt $capturedUtc) {
        throw "Evidence runner byte observations do not enclose the complete physical workflow."
    }
    $exactWorkerObservedUtc = Assert-UtcTimestamp ([string]$manifest.exact_worker_probe.observed_utc) "Exact-worker process probe time"
    $desktopObservedStartedUtc = Assert-UtcTimestamp ([string]$manifest.desktop_process.observed_started_utc) "Packaged desktop process start time"
    $mainWindowObservationStartedUtc = Assert-UtcTimestamp ([string]$manifest.desktop_process.main_window_observation_started_utc) "Packaged desktop main-window observation start time"
    $mainWindowObservedUtc = Assert-UtcTimestamp ([string]$manifest.desktop_process.main_window_observed_utc) "Packaged desktop main-window observation time"
    $mainWindowStabilityRecheckedUtc = Assert-UtcTimestamp ([string]$manifest.desktop_process.main_window_stability_rechecked_utc) "Packaged desktop main-window stability recheck time"
    $desktopObservedExitedUtc = Assert-UtcTimestamp ([string]$manifest.desktop_process.observed_exited_utc) "Packaged desktop process exit time"
    if ([string]$manifest.exact_worker_probe.request -cne "PING" -or
        [string]$manifest.exact_worker_probe.response -cne "PONG" -or
        [int]$manifest.exact_worker_probe.exit_code -ne 0) {
        throw "Evidence lacks a successful isolated exact-worker PING/PONG probe."
    }
    if ([string]$manifest.desktop_process.package_relative_executable -cne "ketchup-app.exe" -or
        [int64]$manifest.desktop_process.process_id -le 0 -or
        [int]$manifest.desktop_process.session_id -le 0 -or
        [int]$manifest.desktop_process.session_id -ne [int]$manifest.operator.session_id -or
        [string]$manifest.desktop_process.load_origin -cne "verified-package-root" -or
        [string]$manifest.desktop_process.working_directory_origin -cne "fresh-outside-package" -or
        $manifest.desktop_process.image_path_matches_verified_package -ne $true -or
        [int64]$manifest.desktop_process.image_size_bytes -le 0 -or
        [string]$manifest.desktop_process.image_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        [int]$manifest.desktop_process.process_machine -ne 0 -or
        [int]$manifest.desktop_process.native_machine -ne 0x8664 -or
        $manifest.desktop_process.native_amd64_execution -ne $true -or
        [int64]$manifest.desktop_process.main_window_wait_elapsed_ms -lt 100 -or
        [int64]$manifest.desktop_process.main_window_wait_elapsed_ms -gt 30000 -or
        $mainWindowStabilityRecheckedUtc -lt $mainWindowObservedUtc.AddMilliseconds(100) -or
        $manifest.desktop_process.main_window_same_handle_stable -ne $true -or
        $manifest.desktop_process.main_window_exists -ne $true -or
        $manifest.desktop_process.main_window_visible -ne $true -or
        [int64]$manifest.desktop_process.main_window_process_id -ne [int64]$manifest.desktop_process.process_id -or
        $manifest.desktop_process.main_window_observed -ne $true -or
        [string]$manifest.desktop_process.main_window_title -cne "Ketchup" -or
        [int64]$manifest.desktop_process.main_window_handle -le 0) {
        throw "Evidence lacks the exact packaged desktop process and foreign-working-directory product-window observation."
    }
    if ($packageVerifiedBeforeLaunchUtc -ge $packageVerifiedAfterExitUtc -or $packageVerifiedAfterExitUtc -gt $capturedUtc) {
        throw "Isolated package verification chronology is invalid."
    }
    if ([string]::IsNullOrWhiteSpace([string]$manifest.machine.manufacturer) -or
        [string]::IsNullOrWhiteSpace([string]$manifest.machine.model) -or
        [int64]$manifest.machine.total_physical_memory_bytes -le 0 -or
        [string]::IsNullOrWhiteSpace([string]$manifest.machine.os) -or
        [string]$manifest.machine.os_build -cnotmatch '^[0-9]+$') {
        throw "Evidence lacks complete physical machine identity."
    }
    $gpus = @($manifest.machine.gpus)
    if ($gpus.Count -eq 0) { throw "Evidence lacks a physical GPU inventory." }
    foreach ($gpu in $gpus) {
        Assert-ExactProperties $gpu @("name", "driver_version", "status") "Physical GPU record"
        if ([string]::IsNullOrWhiteSpace([string]$gpu.name) -or
            [string]::IsNullOrWhiteSpace([string]$gpu.driver_version) -or
            [string]::IsNullOrWhiteSpace([string]$gpu.status)) {
            throw "Evidence contains an incomplete physical GPU record."
        }
    }

    $children = @(Get-ChildItem $Path -Force)
    if (@($children | Where-Object { $_.PSIsContainer }).Count -ne 0) {
        throw "Evidence directory contains an unexpected subdirectory."
    }
    Assert-ExactNames @($children | ForEach-Object { $_.Name }) @($artifactNames + "evidence-manifest.json") "Evidence contents"

    $evidenceGuardStreams = @{}
    try {
        foreach ($name in @($artifactNames + "evidence-manifest.json")) {
            $stream = [IO.File]::Open(
                (Join-Path $Path $name),
                [IO.FileMode]::Open,
                [IO.FileAccess]::Read,
                [IO.FileShare]::Read
            )
            $evidenceGuardStreams[$name] = $stream
        }
        if ((Get-StreamSha256 $evidenceGuardStreams["evidence-manifest.json"]) -cne $manifestHash) {
            throw "Evidence manifest changed before the complete immutable-bundle guard was acquired."
        }

    $records = @($manifest.artifacts)
    Assert-ExactNames @($records | ForEach-Object { [string]$_.name }) $artifactNames "Evidence artifact records"
    foreach ($record in $records) {
        Assert-ExactProperties $record @("name", "size_bytes", "sha256") "Evidence artifact record"
        $name = [string]$record.name
        if ($name -notmatch '^[A-Za-z0-9._-]+$') { throw "Unsafe evidence artifact name: $name" }
        $artifactStream = $evidenceGuardStreams[$name]
        if ($artifactStream.Length -ne [int64]$record.size_bytes -or
            (Get-StreamSha256 $artifactStream) -ne [string]$record.sha256) {
            throw "Evidence artifact fingerprint mismatch: $name"
        }
    }
    $canonicalHashes = @{}
    foreach ($name in @(
        "baseline.ketchup", "new-document.ketchup", "roundtrip.ketchup",
        "after-failed-open.ketchup", "after-failed-save.ketchup"
    )) {
        $canonicalHashes[$name] = Assert-NativeKetchupContainer $evidenceGuardStreams[$name] "Captured native document $name"
    }

    $baselineSemantics = $manifest.document_semantics.baseline
    $newSemantics = $manifest.document_semantics.new_document
    if ([string]$baselineSemantics.canonical_digest -cne [string]$canonicalHashes["baseline.ketchup"] -or
        [string]$newSemantics.canonical_digest -cne [string]$canonicalHashes["new-document.ketchup"]) {
        throw "Packaged-app canonical digest does not match the captured native document."
    }
    foreach ($record in @($baselineSemantics, $newSemantics)) {
        if ([int]$record.schema_version -ne 17 -or
            [uint64]$record.document_id -eq 0 -or
            [uint64]$record.revision -eq 0 -or
            [string]$record.canonical_digest -cnotmatch '^[0-9a-f]{64}$' -or
            [string]$record.container_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            [int]$record.definitions -lt [int]$record.profile_extrusion_definitions -or
            [int]$record.profiles -lt [int]$record.profile_extrusion_definitions -or
            [int]$record.extrusions -lt [int]$record.profile_extrusion_definitions -or
            [int]$record.visible_profile_extrusion_root_occurrences -lt 0 -or
            [int]$record.visible_profile_extrusion_root_occurrences -gt [int]$record.root_occurrences -or
            [int]$record.visible_profile_extrusion_root_occurrences -gt [int]$record.profile_extrusion_definitions) {
            throw "Packaged-app document semantics are invalid."
        }
    }
    if ([int]$baselineSemantics.definitions -lt 2 -or
        [int]$baselineSemantics.root_occurrences -lt 2 -or
        [int]$baselineSemantics.profiles -lt 2 -or
        [int]$baselineSemantics.extrusions -lt 2 -or
        [int]$baselineSemantics.profile_extrusion_definitions -lt 2 -or
        [int]$baselineSemantics.visible_profile_extrusion_root_occurrences -lt 2 -or
        [int]$newSemantics.definitions -ne 1 -or
        [int]$newSemantics.root_occurrences -ne 1 -or
        [int]$newSemantics.profiles -ne 1 -or
        [int]$newSemantics.extrusions -ne 1 -or
        [int]$newSemantics.profile_extrusion_definitions -ne 1 -or
        [int]$newSemantics.visible_profile_extrusion_root_occurrences -ne 1 -or
        [uint64]$baselineSemantics.revision -le [uint64]$newSemantics.revision -or
        [uint64]$baselineSemantics.document_id -eq [uint64]$newSemantics.document_id -or
        [string]$baselineSemantics.canonical_digest -ceq [string]$newSemantics.canonical_digest -or
        [string]$baselineSemantics.container_sha256 -cne (Get-StreamSha256 $evidenceGuardStreams["baseline.ketchup"]) -or
        [string]$newSemantics.container_sha256 -cne (Get-StreamSha256 $evidenceGuardStreams["new-document.ketchup"])) {
        throw "Packaged-app inspection did not prove an additional Rectangle-to-Extrusion baseline followed by a fresh New document."
    }

    $steps = @($manifest.steps)
    Assert-ExactNames @($steps | ForEach-Object { [string]$_.id }) $requiredStepIds "Physical workflow steps"
    $previousStepUtc = [DateTimeOffset]::MinValue
    for ($index = 0; $index -lt $steps.Count; $index++) {
        $step = $steps[$index]
        Assert-ExactProperties $step @(
            "id", "result", "physical_operator_confirmed", "operator_attestation", "product_process_alive", "product_main_window_handle",
            "product_main_window_title", "product_main_window_visible", "product_main_window_process_id", "product_main_window_observed_utc",
            "product_main_window_stability_rechecked_utc", "product_main_window_same_handle_stable", "confirmed_utc"
        ) "Physical workflow step"
        if ([string]$step.id -cne $requiredStepIds[$index]) {
            throw "Physical workflow steps are not in the required execution order."
        }
        if ($step.result -ne "PASS" -or
            $step.physical_operator_confirmed -ne $true -or
            [string]$step.operator_attestation -cne $requiredStepAttestations[$index]) {
            throw "Physical workflow step is incomplete: $($step.id)"
        }
        if ($step.product_process_alive -ne $true -or
            [int64]$step.product_main_window_handle -ne [int64]$manifest.desktop_process.main_window_handle -or
            [string]$step.product_main_window_title -cne "Ketchup" -or
            $step.product_main_window_visible -ne $true -or
            [int64]$step.product_main_window_process_id -ne [int64]$manifest.desktop_process.process_id -or
            $step.product_main_window_same_handle_stable -ne $true) {
            throw "Physical workflow step is not bound to the live packaged Ketchup product surface: $($step.id)"
        }
        $stepUtc = Assert-UtcTimestamp ([string]$step.confirmed_utc) "Physical workflow step time for $($step.id)"
        $stepWindowObservedUtc = Assert-UtcTimestamp ([string]$step.product_main_window_observed_utc) "Physical workflow product-window observation time for $($step.id)"
        $stepWindowStabilityRecheckedUtc = Assert-UtcTimestamp ([string]$step.product_main_window_stability_rechecked_utc) "Physical workflow product-window stability recheck time for $($step.id)"
        $stepWindowLowerBoundUtc = if ($index -eq 0) { $runtimeModulesVerifiedBeforeWorkflowUtc } else { $previousStepUtc }
        if ($stepWindowObservedUtc -le $stepWindowLowerBoundUtc -or
            $stepWindowStabilityRecheckedUtc -lt $stepWindowObservedUtc.AddMilliseconds(100) -or
            $stepWindowStabilityRecheckedUtc -gt $stepUtc) {
            throw "Physical workflow step product surface was not stable across two time-separated live samples: $($step.id)"
        }
        if ($stepUtc -le $previousStepUtc -or $stepUtc -gt $capturedUtc) {
            throw "Physical workflow step chronology is invalid: $($step.id)"
        }
        $previousStepUtc = $stepUtc
    }
    $firstStepUtc = Assert-UtcTimestamp ([string]$steps[0].confirmed_utc) "First physical workflow step time"
    $saveExistingRewrite = $manifest.save_existing_rewrite
    $beforeRewriteObservedUtc = Assert-UtcTimestamp ([string]$saveExistingRewrite.before_observed_utc) "Existing-document Save pre-rewrite observation time"
    $beforeRewriteLastWriteUtc = Assert-UtcTimestamp ([string]$saveExistingRewrite.before_last_write_utc) "Existing-document Save pre-rewrite file time"
    $afterRewriteLastWriteUtc = Assert-UtcTimestamp ([string]$saveExistingRewrite.after_last_write_utc) "Existing-document Save post-rewrite file time"
    $afterRewriteObservedUtc = Assert-UtcTimestamp ([string]$saveExistingRewrite.after_observed_utc) "Existing-document Save post-rewrite observation time"
    $saveExistingStepUtc = Assert-UtcTimestamp ([string]$steps[1].confirmed_utc) "Existing-document Save physical step time"
    $nextStepUtc = Assert-UtcTimestamp ([string]$steps[2].confirmed_utc) "Post-rewrite physical step time"
    $capturedBaselineStream = $evidenceGuardStreams["baseline.ketchup"]
    $capturedBaselineSizeBytes = $capturedBaselineStream.Length
    $capturedBaselineSha256 = Get-StreamSha256 $capturedBaselineStream
    if ([string]$saveExistingRewrite.artifact -cne "baseline.ketchup" -or
        [int64]$saveExistingRewrite.before_size_bytes -ne $capturedBaselineSizeBytes -or
        [int64]$saveExistingRewrite.after_size_bytes -ne $capturedBaselineSizeBytes -or
        [string]$saveExistingRewrite.before_sha256 -cne $capturedBaselineSha256 -or
        [string]$saveExistingRewrite.after_sha256 -cne $capturedBaselineSha256) {
        throw "Existing-document Save rewrite fingerprint does not match the captured baseline artifact."
    }
    if ([string]$saveExistingRewrite.before_file_identity -cnotmatch '^[0-9a-f]{8}:[0-9a-f]{16}$' -or
        [string]$saveExistingRewrite.after_file_identity -cnotmatch '^[0-9a-f]{8}:[0-9a-f]{16}$' -or
        [string]$saveExistingRewrite.before_file_identity -ceq [string]$saveExistingRewrite.after_file_identity) {
        throw "Existing-document Save did not replace the exact filesystem object."
    }
    if ($beforeRewriteLastWriteUtc -gt $firstStepUtc -or
        $firstStepUtc -ge $beforeRewriteObservedUtc -or
        $beforeRewriteObservedUtc -ge $afterRewriteLastWriteUtc -or
        $afterRewriteLastWriteUtc -gt $saveExistingStepUtc -or
        $saveExistingStepUtc -ge $afterRewriteObservedUtc -or
        $afterRewriteObservedUtc -ge $nextStepUtc) {
        throw "Existing-document Save rewrite chronology is invalid."
    }
    $nativeDialogs = @($manifest.native_file_dialogs)
    Assert-ExactNames @($nativeDialogs | ForEach-Object { [string]$_.id }) $requiredNativeDialogIds "Observed native file dialogs"
    $previousDialogUtc = [DateTimeOffset]::MinValue
    for ($index = 0; $index -lt $nativeDialogs.Count; $index++) {
        $dialog = $nativeDialogs[$index]
        Assert-ExactProperties $dialog @(
            "id", "window_handle", "owner_window_handle", "owner_window_enabled", "owner_window_exists", "owner_window_visible", "owner_window_title", "owner_window_process_id", "top_level_class", "owning_process_id", "visible", "foreground_window", "direct_ui_child_observed", "observation_started_utc", "observation_wait_elapsed_ms", "observed_utc", "stability_rechecked_utc", "same_window_stable", "closed_utc", "closure_rechecked_utc", "closure_stable", "window_exists_after_close", "visible_after_close", "owner_window_enabled_after_close", "owner_window_exists_after_close", "owner_window_visible_after_close", "owner_window_title_after_close", "owner_window_process_id_after_close"
        ) "Observed native file dialog"
        if ([string]$dialog.id -cne $requiredNativeDialogIds[$index] -or
            [int64]$dialog.window_handle -le 0 -or
            [int64]$dialog.owner_window_handle -ne [int64]$manifest.desktop_process.main_window_handle -or
            [int64]$dialog.window_handle -eq [int64]$dialog.owner_window_handle -or
            [string]$dialog.top_level_class -cne "#32770" -or
            [int64]$dialog.owning_process_id -ne [int64]$manifest.desktop_process.process_id -or
            $dialog.visible -ne $true) {
            throw "Native file-dialog observation is not bound to the packaged desktop process: $($dialog.id)"
        }
        if ($dialog.owner_window_enabled -ne $false) {
            throw "Native file-dialog observation did not disable its packaged Ketchup owner window: $($dialog.id)"
        }
        if ($dialog.owner_window_exists -ne $true -or
            $dialog.owner_window_visible -ne $true -or
            [string]$dialog.owner_window_title -cne "Ketchup" -or
            [int64]$dialog.owner_window_process_id -ne [int64]$manifest.desktop_process.process_id) {
            throw "Native file-dialog observation is not bound to the live exact packaged Ketchup owner window: $($dialog.id)"
        }
        if ($dialog.foreground_window -ne $true) {
            throw "Native file-dialog observation was not the active foreground window: $($dialog.id)"
        }
        if ($dialog.direct_ui_child_observed -ne $true) {
            throw "Native file-dialog observation lacks the Windows common item dialog marker: $($dialog.id)"
        }
        $observationStartedUtc = Assert-UtcTimestamp ([string]$dialog.observation_started_utc) "Native file-dialog observation start time for $($dialog.id)"
        $dialogUtc = Assert-UtcTimestamp ([string]$dialog.observed_utc) "Native file-dialog observation time for $($dialog.id)"
        $stabilityRecheckedUtc = Assert-UtcTimestamp ([string]$dialog.stability_rechecked_utc) "Native file-dialog stability recheck time for $($dialog.id)"
        $closedUtc = Assert-UtcTimestamp ([string]$dialog.closed_utc) "Native file-dialog close time for $($dialog.id)"
        $closureRecheckedUtc = Assert-UtcTimestamp ([string]$dialog.closure_rechecked_utc) "Native file-dialog closure recheck time for $($dialog.id)"
        if ([int64]$dialog.observation_wait_elapsed_ms -lt 0 -or
            [int64]$dialog.observation_wait_elapsed_ms -gt 120000 -or
            $observationStartedUtc -gt $dialogUtc) {
            throw "Native file-dialog observation was not bounded by its monotonic two-minute wait: $($dialog.id)"
        }
        if ($dialog.same_window_stable -ne $true -or
            $stabilityRecheckedUtc -lt $dialogUtc.AddMilliseconds(100) -or
            $stabilityRecheckedUtc -ge $closedUtc) {
            throw "Native file-dialog observation was not stable across two time-separated live samples: $($dialog.id)"
        }
        if ($dialog.closure_stable -ne $true -or
            $closureRecheckedUtc -lt $closedUtc.AddMilliseconds(100) -or
            $dialog.window_exists_after_close -ne $false -or
            $dialog.visible_after_close -ne $false -or
            $dialog.owner_window_enabled_after_close -ne $true) {
            throw "Native file-dialog observation lacks stable time-separated destruction and owner reactivation: $($dialog.id)"
        }
        if ($dialog.owner_window_exists_after_close -ne $true -or
            $dialog.owner_window_visible_after_close -ne $true -or
            [string]$dialog.owner_window_title_after_close -cne "Ketchup" -or
            [int64]$dialog.owner_window_process_id_after_close -ne [int64]$manifest.desktop_process.process_id) {
            throw "Native file-dialog closure is not bound to the still-live exact packaged Ketchup owner window: $($dialog.id)"
        }
        $stepIndex = [Array]::IndexOf($requiredStepIds, $nativeDialogStepIds[$index])
        $lowerBoundUtc = if ($stepIndex -eq 0) {
            $runtimeModulesVerifiedBeforeWorkflowUtc
        } else {
            Assert-UtcTimestamp ([string]$steps[$stepIndex - 1].confirmed_utc) "Previous physical workflow step time"
        }
        $upperBoundUtc = Assert-UtcTimestamp ([string]$steps[$stepIndex].confirmed_utc) "Native-dialog target workflow step time"
        if ($observationStartedUtc -le $previousDialogUtc -or
            $observationStartedUtc -le $lowerBoundUtc -or
            $observationStartedUtc -gt $dialogUtc -or
            $dialogUtc -ge $closedUtc -or
            $closedUtc -ge $closureRecheckedUtc -or
            $closureRecheckedUtc -ge $upperBoundUtc) {
            throw "Native file-dialog observation chronology is invalid: $($dialog.id)"
        }
        $previousDialogUtc = $closureRecheckedUtc
    }

    $saveOutputCreations = @($manifest.save_output_creations)
    Assert-ExactNames @($saveOutputCreations | ForEach-Object { [string]$_.id }) $requiredSaveOutputIds "Successful Save output creations"
    $saveOutputFileIdentities = @{}
    for ($index = 0; $index -lt $saveOutputCreations.Count; $index++) {
        $creation = $saveOutputCreations[$index]
        Assert-ExactProperties $creation @(
            "id", "artifact", "existed_before", "before_observed_utc", "created_utc", "last_write_utc",
            "after_observed_utc", "file_identity", "size_bytes", "sha256", "bundle_copy_started_utc",
            "bundle_source_file_identity", "bundle_source_size_bytes", "bundle_source_sha256", "bundle_copy_guarded",
            "bundle_copy_method", "bundle_destination_verified", "bundle_destination_size_bytes", "bundle_destination_sha256", "bundle_copy_completed_utc"
        ) "Successful Save output creation"
        if ([string]$creation.id -cne $requiredSaveOutputIds[$index] -or
            [string]$creation.artifact -cne $saveOutputArtifacts[$index] -or
            $creation.existed_before -ne $false -or
            [string]$creation.file_identity -cnotmatch '^[0-9a-f]{8}:[0-9a-f]{16}$' -or
            [int64]$creation.size_bytes -le 0 -or
            [string]$creation.sha256 -cnotmatch '^[0-9a-f]{64}$') {
            throw "Successful Save output was not created at its exact previously absent target: $($creation.id)"
        }
        if ($saveOutputFileIdentities.ContainsKey([string]$creation.file_identity)) {
            throw "Successful Save outputs alias the same filesystem object: $($creation.id)"
        }
        $saveOutputFileIdentities[[string]$creation.file_identity] = $true
        $expectedBundleSourceIdentity = if ($index -eq 0) {
            [string]$saveExistingRewrite.after_file_identity
        } else {
            [string]$creation.file_identity
        }
        if ($creation.bundle_copy_guarded -ne $true -or
            [string]$creation.bundle_copy_method -cne "guarded-source-stream" -or
            [string]$creation.bundle_source_file_identity -cne $expectedBundleSourceIdentity -or
            [int64]$creation.bundle_source_size_bytes -ne [int64]$creation.size_bytes -or
            [string]$creation.bundle_source_sha256 -cne [string]$creation.sha256) {
            throw "Successful Save source changed before immutable-bundle copy: $($creation.id)"
        }
        if ($creation.bundle_destination_verified -ne $true -or
            [int64]$creation.bundle_destination_size_bytes -ne [int64]$creation.bundle_source_size_bytes -or
            [string]$creation.bundle_destination_sha256 -cne [string]$creation.bundle_source_sha256) {
            throw "Successful Save immutable-bundle destination differs from its guarded source: $($creation.id)"
        }
        $artifactStream = $evidenceGuardStreams[[string]$creation.artifact]
        if ($artifactStream.Length -ne [int64]$creation.bundle_destination_size_bytes -or
            (Get-StreamSha256 $artifactStream) -cne [string]$creation.bundle_destination_sha256) {
            throw "Successful Save output fingerprint differs from its immutable artifact: $($creation.id)"
        }
        $beforeObservedUtc = Assert-UtcTimestamp ([string]$creation.before_observed_utc) "Successful Save pre-existence observation time for $($creation.id)"
        $createdUtc = Assert-UtcTimestamp ([string]$creation.created_utc) "Successful Save filesystem creation time for $($creation.id)"
        $lastWriteUtc = Assert-UtcTimestamp ([string]$creation.last_write_utc) "Successful Save filesystem write time for $($creation.id)"
        $afterObservedUtc = Assert-UtcTimestamp ([string]$creation.after_observed_utc) "Successful Save creation observation time for $($creation.id)"
        $bundleCopyStartedUtc = Assert-UtcTimestamp ([string]$creation.bundle_copy_started_utc) "Successful Save bundle-copy guard start time for $($creation.id)"
        $bundleCopyCompletedUtc = Assert-UtcTimestamp ([string]$creation.bundle_copy_completed_utc) "Successful Save bundle-copy completion time for $($creation.id)"
        $dialogIndex = [Array]::IndexOf($requiredNativeDialogIds, $saveOutputDialogIds[$index])
        $stepIndex = [Array]::IndexOf($requiredStepIds, $saveOutputStepIds[$index])
        $dialogObservedUtc = Assert-UtcTimestamp ([string]$nativeDialogs[$dialogIndex].observed_utc) "Successful Save native-dialog observation time"
        $dialogClosedUtc = Assert-UtcTimestamp ([string]$nativeDialogs[$dialogIndex].closed_utc) "Successful Save native-dialog close time"
        $dialogClosureRecheckedUtc = Assert-UtcTimestamp ([string]$nativeDialogs[$dialogIndex].closure_rechecked_utc) "Successful Save native-dialog closure recheck time"
        $stepConfirmedUtc = Assert-UtcTimestamp ([string]$steps[$stepIndex].confirmed_utc) "Successful Save physical-step time"
        $lowerBoundUtc = if ($stepIndex -eq 0) {
            $runtimeModulesVerifiedBeforeWorkflowUtc
        } else {
            Assert-UtcTimestamp ([string]$steps[$stepIndex - 1].confirmed_utc) "Previous successful Save physical-step time"
        }
        $upperBoundUtc = if ($stepIndex -eq $steps.Count - 1) {
            $runtimeModulesVerifiedAfterWorkflowUtc
        } else {
            Assert-UtcTimestamp ([string]$steps[$stepIndex + 1].confirmed_utc) "Next successful Save physical-step time"
        }
        if ($beforeObservedUtc -le $lowerBoundUtc -or
            $beforeObservedUtc -ge $dialogObservedUtc -or
            $createdUtc -le $dialogObservedUtc -or
            $createdUtc -gt $lastWriteUtc -or
            $lastWriteUtc -gt $dialogClosedUtc -or
            $dialogClosedUtc -ge $dialogClosureRecheckedUtc -or
            $dialogClosureRecheckedUtc -ge $stepConfirmedUtc -or
            $stepConfirmedUtc -ge $afterObservedUtc -or
            $afterObservedUtc -ge $upperBoundUtc) {
            throw "Successful Save output creation chronology is invalid: $($creation.id)"
        }
        if ($bundleCopyStartedUtc -le $runnerAfterObservedUtc -or
            $bundleCopyCompletedUtc -lt $bundleCopyStartedUtc -or
            $bundleCopyCompletedUtc -gt $capturedUtc) {
            throw "Successful Save immutable-bundle copy chronology is invalid: $($creation.id)"
        }
    }
    if ([string]$saveExistingRewrite.before_file_identity -cne [string]$saveOutputCreations[0].file_identity) {
        throw "Existing-document Save rewrite is not chained to the exact baseline Save As filesystem object."
    }
    if ($saveOutputFileIdentities.ContainsKey([string]$saveExistingRewrite.after_file_identity)) {
        throw "Post-rewrite baseline aliases a successful Save output filesystem object."
    }

    $failureInputObservations = @($manifest.failure_input_observations)
    Assert-ExactNames @($failureInputObservations | ForEach-Object { [string]$_.id }) $requiredFailureInputIds "Failed-operation input observations"
    for ($index = 0; $index -lt $failureInputObservations.Count; $index++) {
        $observation = $failureInputObservations[$index]
        Assert-ExactProperties $observation @(
            "id", "artifact", "dialog_id", "exclusive_lock_held", "observation_handle_guarded", "before_observed_utc", "before_size_bytes",
            "before_sha256", "before_file_identity", "after_observed_utc", "after_size_bytes", "after_sha256", "after_file_identity",
            "bundle_copy_started_utc", "bundle_source_file_identity", "bundle_source_size_bytes", "bundle_source_sha256",
            "bundle_copy_guarded", "bundle_copy_method", "bundle_destination_verified", "bundle_destination_size_bytes",
            "bundle_destination_sha256", "bundle_copy_completed_utc"
        ) "Failed-operation input observation"
        if ([string]$observation.id -cne $requiredFailureInputIds[$index] -or
            [string]$observation.artifact -cne $failureInputArtifacts[$index] -or
            [string]$observation.dialog_id -cne $failureInputDialogIds[$index] -or
            $observation.exclusive_lock_held -ne $failureInputExclusiveLocks[$index] -or
            $observation.observation_handle_guarded -ne $true -or
            [int64]$observation.before_size_bytes -le 0 -or
            [int64]$observation.after_size_bytes -ne [int64]$observation.before_size_bytes -or
            [string]$observation.before_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            [string]$observation.after_sha256 -cne [string]$observation.before_sha256) {
            throw "Failed-operation input fingerprint is invalid: $($observation.id)"
        }
        if ([string]$observation.before_file_identity -cnotmatch '^[0-9a-f]{8}:[0-9a-f]{16}$' -or
            [string]$observation.after_file_identity -cne [string]$observation.before_file_identity) {
            throw "Failed-operation input was replaced during its dialog interaction: $($observation.id)"
        }
        if ($observation.bundle_copy_guarded -ne $true -or
            [string]$observation.bundle_copy_method -cne "guarded-source-stream" -or
            [string]$observation.bundle_source_file_identity -cne [string]$observation.after_file_identity -or
            [int64]$observation.bundle_source_size_bytes -ne [int64]$observation.after_size_bytes -or
            [string]$observation.bundle_source_sha256 -cne [string]$observation.after_sha256) {
            throw "Failed-operation input changed before immutable-bundle copy: $($observation.id)"
        }
        if ($observation.bundle_destination_verified -ne $true -or
            [int64]$observation.bundle_destination_size_bytes -ne [int64]$observation.bundle_source_size_bytes -or
            [string]$observation.bundle_destination_sha256 -cne [string]$observation.bundle_source_sha256) {
            throw "Failed-operation immutable-bundle destination differs from its guarded source: $($observation.id)"
        }
        $artifactStream = $evidenceGuardStreams[[string]$observation.artifact]
        if ($artifactStream.Length -ne [int64]$observation.bundle_destination_size_bytes -or
            (Get-StreamSha256 $artifactStream) -cne [string]$observation.bundle_destination_sha256) {
            throw "Failed-operation input fingerprint differs from its immutable artifact: $($observation.id)"
        }
        $dialogIndex = [Array]::IndexOf($requiredNativeDialogIds, $failureInputDialogIds[$index])
        $targetStepIndex = [Array]::IndexOf($requiredStepIds, $nativeDialogStepIds[$dialogIndex])
        $beforeObservedUtc = Assert-UtcTimestamp ([string]$observation.before_observed_utc) "Failed-operation input pre-observation time for $($observation.id)"
        $afterObservedUtc = Assert-UtcTimestamp ([string]$observation.after_observed_utc) "Failed-operation input post-observation time for $($observation.id)"
        $bundleCopyStartedUtc = Assert-UtcTimestamp ([string]$observation.bundle_copy_started_utc) "Failed-operation input bundle-copy guard start time for $($observation.id)"
        $bundleCopyCompletedUtc = Assert-UtcTimestamp ([string]$observation.bundle_copy_completed_utc) "Failed-operation input bundle-copy completion time for $($observation.id)"
        $dialogObservedUtc = Assert-UtcTimestamp ([string]$nativeDialogs[$dialogIndex].observed_utc) "Failed-operation native-dialog observation time"
        $dialogClosedUtc = Assert-UtcTimestamp ([string]$nativeDialogs[$dialogIndex].closed_utc) "Failed-operation native-dialog close time"
        $dialogClosureRecheckedUtc = Assert-UtcTimestamp ([string]$nativeDialogs[$dialogIndex].closure_rechecked_utc) "Failed-operation native-dialog closure recheck time"
        $lowerBoundUtc = Assert-UtcTimestamp ([string]$steps[$targetStepIndex - 1].confirmed_utc) "Previous failed-operation workflow step time"
        $upperBoundUtc = Assert-UtcTimestamp ([string]$nativeDialogs[$dialogIndex + 1].observed_utc) "Following continuity-dialog observation time"
        if ($beforeObservedUtc -le $lowerBoundUtc -or
            $beforeObservedUtc -ge $dialogObservedUtc -or
            $dialogClosedUtc -ge $dialogClosureRecheckedUtc -or
            $dialogClosureRecheckedUtc -ge $afterObservedUtc -or
            $afterObservedUtc -ge $upperBoundUtc -or
            $bundleCopyStartedUtc -le $runnerAfterObservedUtc -or
            $bundleCopyCompletedUtc -lt $bundleCopyStartedUtc -or
            $bundleCopyCompletedUtc -gt $capturedUtc) {
            throw "Failed-operation input observation chronology is invalid: $($observation.id)"
        }
    }
    if ($packageVerifiedBeforeLaunchUtc -ge $packageGuardsAcquiredUtc -or
        $packageGuardsAcquiredUtc -ge $packageVerifiedUnderGuardUtc -or
        $packageVerifiedUnderGuardUtc -ge $exactWorkerObservedUtc -or
        $exactWorkerObservedUtc -ge $desktopObservedStartedUtc -or
        $desktopObservedStartedUtc -gt $mainWindowObservationStartedUtc -or
        $mainWindowObservationStartedUtc -gt $mainWindowObservedUtc -or
        $mainWindowObservedUtc -ge $mainWindowStabilityRecheckedUtc -or
        $mainWindowStabilityRecheckedUtc -ge $runtimeModulesVerifiedBeforeWorkflowUtc -or
        $runtimeModulesVerifiedBeforeWorkflowUtc -ge $firstStepUtc -or
        $previousStepUtc -ge $runtimeModulesVerifiedAfterWorkflowUtc -or
        $runtimeModulesVerifiedAfterWorkflowUtc -ge $desktopObservedExitedUtc -or
        $desktopObservedExitedUtc -ge $packageVerifiedAfterExitUtc -or
        $packageVerifiedAfterExitUtc -ge $packageGuardsReleasedUtc -or
        $packageGuardsReleasedUtc -ge $runnerAfterObservedUtc) {
        throw "Physical workflow is not enclosed by the write/delete-guarded isolated package, packaged desktop process, and runtime-module verification intervals."
    }

    $packageManifestHash = ""
    $packageManifest = Read-BoundedJsonStream $evidenceGuardStreams["package-manifest.json"] $maxProvenanceManifestBytes "captured package manifest" ([ref]$packageManifestHash)
    if ([string]$manifest.package_manifest_sha256 -cne $packageManifestHash) {
        throw "Evidence provenance hash does not match the captured package manifest."
    }
    Assert-PackageManifestProvenance $packageManifest
    $expectedGuardedFiles = @("package-manifest.json") + @($packageManifest.files | ForEach-Object { [string]$_.name })
    Assert-ExactNames @($manifest.package_snapshot.guarded_files) $expectedGuardedFiles "Write/delete-guarded package files"
    $loadedModules = @($manifest.loaded_pinned_occt_modules)
    if ($loadedModules.Count -eq 0 -or
        @($loadedModules | Group-Object name | Where-Object { $_.Count -ne 1 }).Count -ne 0 -or
        @($loadedModules | Where-Object { [string]$_.name -ceq "TKernel.dll" }).Count -ne 1) {
        throw "Evidence lacks a unique foundational co-located OCCT runtime observation."
    }
    foreach ($loadedModule in $loadedModules) {
        Assert-ExactProperties $loadedModule @(
            "name", "package_relative_path", "load_origin", "size_bytes", "sha256", "observation_phases"
        ) "Loaded OCCT module record"
        $name = [string]$loadedModule.name
        $packageRecords = @($packageManifest.files | Where-Object { [string]$_.name -ceq $name })
        $observationPhases = @($loadedModule.observation_phases)
        if ($name -notmatch '^TK[A-Za-z0-9]+\.dll$' -or
            [string]$loadedModule.package_relative_path -cne $name -or
            [string]$loadedModule.load_origin -cne "verified-package-root" -or
            $observationPhases.Count -eq 0 -or
            @($observationPhases | Where-Object { $_ -cnotin @("before-workflow", "after-workflow") }).Count -ne 0 -or
            @($observationPhases | Group-Object | Where-Object { $_.Count -ne 1 }).Count -ne 0 -or
            $packageRecords.Count -ne 1 -or
            [string]$packageRecords[0].role -cne "pinned-occt-runtime" -or
            [int64]$loadedModule.size_bytes -ne [int64]$packageRecords[0].size_bytes -or
            [string]$loadedModule.sha256 -cne [string]$packageRecords[0].sha256) {
            throw "Loaded OCCT module evidence differs from the verified package: $name"
        }
        if ($name -ceq "TKernel.dll") {
            Assert-ExactNames $observationPhases @("before-workflow", "after-workflow") "Foundational OCCT runtime observation phases"
        }
    }
    foreach ($name in @("ketchup-app.exe", "ketchup-exact-worker.exe")) {
        $packageRecord = @($packageManifest.files | Where-Object { [string]$_.name -ceq $name })[0]
        $capturedBinaryStream = $evidenceGuardStreams[$name]
        if ($capturedBinaryStream.Length -ne [int64]$packageRecord.size_bytes -or
            (Get-StreamSha256 $capturedBinaryStream) -cne [string]$packageRecord.sha256) {
            throw "Captured executable differs from the verified technical package: $name"
        }
        if ($name -ceq "ketchup-app.exe" -and
            ([int64]$manifest.desktop_process.image_size_bytes -ne [int64]$packageRecord.size_bytes -or
             [string]$manifest.desktop_process.image_sha256 -cne [string]$packageRecord.sha256)) {
            throw "Observed desktop process image differs from the verified technical package."
        }
    }
    & $packager -VerifyOnly -OutputDir $PackageDir
    if ($LASTEXITCODE -ne 0) { throw "Verification technical package failed its exact package allowlist." }
    $verificationPackageGuardStreams = @{}
    try {
    $expectedVerificationPackageNames = @("package-manifest.json") + @($packageManifest.files | ForEach-Object { [string]$_.name })
    $verificationPackageChildren = @(Get-ChildItem $PackageDir -Force)
    if (@($verificationPackageChildren | Where-Object { $_.PSIsContainer }).Count -ne 0) {
        throw "Verification technical package contains an unexpected subdirectory."
    }
    Assert-ExactNames @($verificationPackageChildren | ForEach-Object { $_.Name }) $expectedVerificationPackageNames "Verification package contents under guard"
    foreach ($name in $expectedVerificationPackageNames) {
        $verificationPackageGuardStreams[$name] = [IO.File]::Open(
            (Join-Path $PackageDir $name),
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            [IO.FileShare]::Read
        )
    }
    if ((Get-StreamSha256 $verificationPackageGuardStreams["package-manifest.json"]) -cne $packageManifestHash) {
        throw "Verification package manifest changed before its complete write/delete guard was acquired."
    }
    foreach ($record in @($packageManifest.files)) {
        $stream = $verificationPackageGuardStreams[[string]$record.name]
        if ($stream.Length -ne [int64]$record.size_bytes -or
            (Get-StreamSha256 $stream) -cne [string]$record.sha256) {
            throw "Verification package changed before its complete write/delete guard was acquired: $($record.name)"
        }
    }
    $verificationApp = Join-Path $PackageDir "ketchup-app.exe"
    $verificationWorker = Join-Path $PackageDir "ketchup-exact-worker.exe"
    if ((Get-StreamSha256 $verificationPackageGuardStreams["ketchup-app.exe"]) -cne (Get-StreamSha256 $evidenceGuardStreams["ketchup-app.exe"]) -or
        (Get-StreamSha256 $verificationPackageGuardStreams["ketchup-exact-worker.exe"]) -cne (Get-StreamSha256 $evidenceGuardStreams["ketchup-exact-worker.exe"])) {
        throw "Verification package differs from the captured technical package."
    }
    Invoke-ExactWorkerPing $verificationWorker "Verification packaged exact worker"
    $semanticProperties = @(
        "schema_version", "document_id", "revision", "canonical_digest", "container_sha256", "definitions",
        "root_occurrences", "profiles", "extrusions", "profile_extrusion_definitions",
        "visible_profile_extrusion_root_occurrences"
    )
    foreach ($inspection in @(
        [ordered]@{ name = "baseline"; artifact = "baseline.ketchup"; label = "captured physically modeled baseline" },
        [ordered]@{ name = "new_document"; artifact = "new-document.ketchup"; label = "captured New document" }
    )) {
        $reinspected = Invoke-PackagedDocumentInspection `
            $verificationApp `
            (Join-Path $Path $inspection.artifact) `
            $inspection.label
        Assert-ExactProperties $reinspected $semanticProperties "Reinspected $($inspection.name) document semantics"
        $recorded = $manifest.document_semantics.($inspection.name)
        foreach ($property in $semanticProperties) {
            if ([string]$reinspected.$property -cne [string]$recorded.$property) {
                throw "Captured packaged-app reinspection differs from recorded document semantics: $($inspection.name)."
            }
        }
    }
    } finally {
        foreach ($stream in $verificationPackageGuardStreams.Values) { $stream.Dispose() }
    }
    $baselineHash = Get-StreamSha256 $evidenceGuardStreams["baseline.ketchup"]
    if ((Get-StreamSha256 $evidenceGuardStreams["new-document.ketchup"]) -eq $baselineHash -or
        $canonicalHashes["new-document.ketchup"] -ceq $canonicalHashes["baseline.ketchup"]) {
        throw "New did not produce a canonically distinct document from the physically modeled baseline."
    }
    foreach ($name in @("roundtrip.ketchup", "after-failed-open.ketchup", "after-failed-save.ketchup")) {
        if ((Get-StreamSha256 $evidenceGuardStreams[$name]) -ne $baselineHash) {
            throw "Canonical file continuity failed: $name differs from baseline.ketchup."
        }
    }
    $sentinel = [Text.Encoding]::UTF8.GetBytes("KETCHUP_LOCKED_TARGET_SENTINEL`n")
    $lockedStream = $evidenceGuardStreams["locked-target.ketchup"]
    if ($lockedStream.Length -ne $sentinel.Length -or
        (Get-StreamSha256 $lockedStream) -cne (Get-BytesSha256 $sentinel)) {
        throw "The failed Save modified its locked target."
    }
    $malformedFixture = [Text.Encoding]::UTF8.GetBytes("not a ketchup document`n")
    $corruptStream = $evidenceGuardStreams["corrupt.ketchup"]
    if ($corruptStream.Length -ne $malformedFixture.Length -or
        (Get-StreamSha256 $corruptStream) -cne (Get-BytesSha256 $malformedFixture)) {
        throw "The failed-Open input is not the frozen malformed fixture."
    }
    Write-Host "Verified immutable Windows-first physical release-dialog evidence; G19-03/G19-04 remain."
    } finally {
        foreach ($stream in $evidenceGuardStreams.Values) { $stream.Dispose() }
    }
}

if ($VerifyOnly) {
    Verify-Evidence $EvidenceDir
    exit 0
}
if ($env:OS -ne "Windows_NT") { throw "Physical release-dialog evidence requires Windows." }
if ($RunId -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{2,63}$') { throw "-RunId must be a stable 3-64 character identifier." }
if ([string]::IsNullOrWhiteSpace($OperatorName)) { throw "-OperatorName must name the human physically performing the workflow." }
$windowsIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$operatorWindowsAccount = [string]$windowsIdentity.Name
$operatorWindowsSid = [string]$windowsIdentity.User.Value
$operatorSessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
if ($operatorWindowsAccount -cnotmatch '^[^\\\r\n]+\\[^\\\r\n]+$' -or
    $operatorWindowsSid -cnotmatch '^S-[0-9]+-[0-9]+(?:-[0-9]+)+$' -or
    $operatorSessionId -le 0) {
    throw "Physical release-dialog evidence requires a named Windows account in an interactive nonzero session."
}
if ([string]::IsNullOrWhiteSpace($EvidenceDir)) {
    $EvidenceDir = Join-Path $repoRoot "artifacts\m19\release-dialog-runs\$RunId"
}
$EvidenceDir = [IO.Path]::GetFullPath($EvidenceDir)
if (Test-Path $EvidenceDir) { throw "Immutable evidence path already exists: $EvidenceDir" }
$packagePrefix = $PackageDir.TrimEnd("\") + "\"
if ($EvidenceDir.StartsWith($packagePrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "EvidenceDir may not be nested inside the technical package being snapshotted."
}
$runnerBeforeObservedUtc = [DateTime]::UtcNow.ToString("o")
$runnerBeforeSha256 = Get-Sha256 $PSCommandPath

& $packager -VerifyOnly -OutputDir $PackageDir
if ($LASTEXITCODE -ne 0) { throw "Technical release-candidate verification failed." }
$app = Join-Path $PackageDir "ketchup-app.exe"
$worker = Join-Path $PackageDir "ketchup-exact-worker.exe"
$packageManifestPath = Join-Path $PackageDir "package-manifest.json"
Assert-Leaf $app "packaged desktop application"
Assert-Leaf $worker "packaged exact worker"
Assert-Leaf $packageManifestPath "package manifest"

$parent = Split-Path $EvidenceDir -Parent
[void](New-Item $parent -ItemType Directory -Force)
$staging = Join-Path $parent ("." + [IO.Path]::GetFileName($EvidenceDir) + ".incomplete-" + [Guid]::NewGuid().ToString("N"))
$packageSnapshot = Join-Path $staging "verified-package"
$work = Join-Path $staging "work"
$foreignWorkingDir = Join-Path $staging "foreign-working-directory"
$process = $null
$lockStream = $null
$packageGuardStreams = [Collections.Generic.List[IO.FileStream]]::new()
$saveOutputGuardStreams = [Collections.Generic.List[IO.FileStream]]::new()
$failureInputGuardStreams = @{}
$packageGuardedFiles = @()
$packageGuardsAcquiredUtc = $null
$packageGuardsReleasedUtc = $null
$packageVerifiedBeforeLaunchUtc = $null
$packageVerifiedUnderGuardUtc = $null
$packageVerifiedAfterExitUtc = $null
$runtimeModulesVerifiedBeforeWorkflowUtc = $null
$runtimeModulesVerifiedAfterWorkflowUtc = $null
$exactWorkerProbe = $null
$desktopProcessEvidence = $null
$loadedRuntimeModules = @{}
$steps = [Collections.Generic.List[object]]::new()
$nativeFileDialogs = [Collections.Generic.List[object]]::new()
$saveOutputCreations = [Collections.Generic.List[object]]::new()
$failureInputObservations = [Collections.Generic.List[object]]::new()

if (-not ("KetchupNativeWindowProbe" -as [type])) {
    Add-Type @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class KetchupNativeWindowProbe
{
    private delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    private delegate bool EnumChildProc(IntPtr window, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool IsWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    private static extern bool IsWindowEnabled(IntPtr window);

    [DllImport("user32.dll")]
    private static extern IntPtr GetWindow(IntPtr window, uint command);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);

    [StructLayout(LayoutKind.Sequential)]
    private struct ByHandleFileInformation
    {
        public uint FileAttributes;
        public System.Runtime.InteropServices.ComTypes.FILETIME CreationTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastAccessTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWriteTime;
        public uint VolumeSerialNumber;
        public uint FileSizeHigh;
        public uint FileSizeLow;
        public uint NumberOfLinks;
        public uint FileIndexHigh;
        public uint FileIndexLow;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetFileInformationByHandle(
        SafeFileHandle file,
        out ByHandleFileInformation information);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool IsWow64Process2(
        IntPtr process,
        out ushort processMachine,
        out ushort nativeMachine);

    public static long[] VisibleCommonDialogsForProcess(int expectedProcessId)
    {
        List<long> windows = new List<long>();
        EnumWindows(delegate(IntPtr window, IntPtr parameter)
        {
            uint processId;
            GetWindowThreadProcessId(window, out processId);
            if (processId != expectedProcessId || !IsWindowVisible(window))
            {
                return true;
            }
            StringBuilder className = new StringBuilder(256);
            GetClassName(window, className, className.Capacity);
            if (String.Equals(className.ToString(), "#32770", StringComparison.Ordinal))
            {
                windows.Add(window.ToInt64());
            }
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }

    public static bool IsForegroundWindow(long windowHandle)
    {
        return GetForegroundWindow().ToInt64() == windowHandle;
    }

    public static bool IsVisibleWindow(long windowHandle)
    {
        return IsWindowVisible(new IntPtr(windowHandle));
    }

    public static bool IsExistingWindow(long windowHandle)
    {
        return IsWindow(new IntPtr(windowHandle));
    }

    public static long OwnerWindow(long windowHandle)
    {
        const uint Owner = 4;
        return GetWindow(new IntPtr(windowHandle), Owner).ToInt64();
    }

    public static bool IsEnabledWindow(long windowHandle)
    {
        return IsWindowEnabled(new IntPtr(windowHandle));
    }

    public static int OwningProcessId(long windowHandle)
    {
        uint processId;
        GetWindowThreadProcessId(new IntPtr(windowHandle), out processId);
        return checked((int)processId);
    }

    public static string WindowTitle(long windowHandle)
    {
        StringBuilder title = new StringBuilder(256);
        GetWindowText(new IntPtr(windowHandle), title, title.Capacity);
        return title.ToString();
    }

    public static int[] ProcessMachineTypes(IntPtr processHandle)
    {
        ushort processMachine;
        ushort nativeMachine;
        if (!IsWow64Process2(processHandle, out processMachine, out nativeMachine))
        {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }
        return new int[] { processMachine, nativeMachine };
    }

    public static string FileIdentity(string path)
    {
        using (FileStream stream = new FileStream(
            path,
            FileMode.Open,
            FileAccess.Read,
            FileShare.ReadWrite | FileShare.Delete))
        {
            return FileIdentityForHandle(stream.SafeFileHandle);
        }
    }

    public static string FileIdentityForHandle(SafeFileHandle file)
    {
        return FileObservationForHandle(file)[0];
    }

    public static string[] FileObservationForHandle(SafeFileHandle file)
    {
        ByHandleFileInformation information;
        if (!GetFileInformationByHandle(file, out information))
        {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }
        string identity = information.VolumeSerialNumber.ToString("x8") + ":" +
            information.FileIndexHigh.ToString("x8") +
            information.FileIndexLow.ToString("x8");
        long creationTime = ((long)(uint)information.CreationTime.dwHighDateTime << 32) |
            (uint)information.CreationTime.dwLowDateTime;
        long lastWriteTime = ((long)(uint)information.LastWriteTime.dwHighDateTime << 32) |
            (uint)information.LastWriteTime.dwLowDateTime;
        return new string[] {
            identity,
            DateTime.FromFileTimeUtc(creationTime).ToString("o"),
            DateTime.FromFileTimeUtc(lastWriteTime).ToString("o")
        };
    }

    public static bool HasDirectUiDescendant(long windowHandle)
    {
        bool observed = false;
        EnumChildWindows(new IntPtr(windowHandle), delegate(IntPtr child, IntPtr parameter)
        {
            StringBuilder className = new StringBuilder(256);
            GetClassName(child, className, className.Capacity);
            if (String.Equals(className.ToString(), "DirectUIHWND", StringComparison.Ordinal))
            {
                observed = true;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return observed;
    }
}
'@
}

function Assert-ProductAlive {
    $process.Refresh()
    if ($process.HasExited) { throw "The packaged product exited during physical workflow capture." }
}

function Observe-NativeFileDialog([string]$Id, [string]$Instruction) {
    Write-Host ""
    Write-Host "NATIVE DIALOG OBSERVATION [$Id]"
    Write-Host $Instruction
    Write-Host "The runner will observe the requested foreground dialog automatically; do not return focus to this console."
    $dialogObservationStartedUtc = [DateTime]::UtcNow
    $dialogObservationTimeout = [TimeSpan]::FromMinutes(2)
    $dialogObservationTimer = [Diagnostics.Stopwatch]::StartNew()
    $dialogs = @()
    $ownerWindow = [int64]0
    $ownerProcessId = 0
    $ownerTitle = ""
    $observationWaitElapsedMs = $null
    $observedUtc = $null
    $stabilityRecheckedUtc = $null
    $isExactForegroundDialog = $false
    do {
        Assert-ProductAlive
        $dialogs = @([KetchupNativeWindowProbe]::VisibleCommonDialogsForProcess($process.Id))
        if ($dialogs.Count -gt 1) {
            throw "Expected at most one visible Windows native common dialog owned by the packaged product for $Id; observed $($dialogs.Count)."
        }
        if ($dialogs.Count -eq 1) {
            $candidateWindow = [int64]$dialogs[0]
            $ownerWindow = [KetchupNativeWindowProbe]::OwnerWindow($candidateWindow)
            $candidateOwnerProcessId = [KetchupNativeWindowProbe]::OwningProcessId($ownerWindow)
            $candidateOwnerTitle = [KetchupNativeWindowProbe]::WindowTitle($ownerWindow)
            $isExactForegroundDialog =
                [KetchupNativeWindowProbe]::HasDirectUiDescendant($candidateWindow) -and
                [KetchupNativeWindowProbe]::IsForegroundWindow($candidateWindow) -and
                $ownerWindow -eq [int64]$desktopProcessEvidence.main_window_handle -and
                [KetchupNativeWindowProbe]::IsExistingWindow($ownerWindow) -and
                [KetchupNativeWindowProbe]::IsVisibleWindow($ownerWindow) -and
                $candidateOwnerProcessId -eq $process.Id -and
                $candidateOwnerTitle -ceq "Ketchup" -and
                -not [KetchupNativeWindowProbe]::IsEnabledWindow($ownerWindow)
            if ($isExactForegroundDialog) {
                $candidateObservedUtc = [DateTime]::UtcNow
                Start-Sleep -Milliseconds 100
                Assert-ProductAlive
                $recheckedDialogs = @([KetchupNativeWindowProbe]::VisibleCommonDialogsForProcess($process.Id))
                if ($recheckedDialogs.Count -gt 1) {
                    throw "Expected at most one stable Windows native common dialog owned by the packaged product for $Id; observed $($recheckedDialogs.Count)."
                }
                if ($recheckedDialogs.Count -eq 1 -and [int64]$recheckedDialogs[0] -eq $candidateWindow) {
                    $recheckedOwnerWindow = [KetchupNativeWindowProbe]::OwnerWindow($candidateWindow)
                    $recheckedOwnerProcessId = [KetchupNativeWindowProbe]::OwningProcessId($recheckedOwnerWindow)
                    $recheckedOwnerTitle = [KetchupNativeWindowProbe]::WindowTitle($recheckedOwnerWindow)
                    $isExactForegroundDialog =
                        [KetchupNativeWindowProbe]::IsExistingWindow($candidateWindow) -and
                        [KetchupNativeWindowProbe]::IsVisibleWindow($candidateWindow) -and
                        [KetchupNativeWindowProbe]::OwningProcessId($candidateWindow) -eq $process.Id -and
                        [KetchupNativeWindowProbe]::HasDirectUiDescendant($candidateWindow) -and
                        [KetchupNativeWindowProbe]::IsForegroundWindow($candidateWindow) -and
                        $recheckedOwnerWindow -eq $ownerWindow -and
                        [KetchupNativeWindowProbe]::IsExistingWindow($recheckedOwnerWindow) -and
                        [KetchupNativeWindowProbe]::IsVisibleWindow($recheckedOwnerWindow) -and
                        $recheckedOwnerProcessId -eq $process.Id -and
                        $recheckedOwnerTitle -ceq "Ketchup" -and
                        -not [KetchupNativeWindowProbe]::IsEnabledWindow($recheckedOwnerWindow)
                    if ($isExactForegroundDialog -and $dialogObservationTimer.Elapsed -le $dialogObservationTimeout) {
                        $dialogs = $recheckedDialogs
                        $ownerProcessId = $recheckedOwnerProcessId
                        $ownerTitle = $recheckedOwnerTitle
                        $observationWaitElapsedMs = [int64]$dialogObservationTimer.ElapsedMilliseconds
                        $observedUtc = $candidateObservedUtc.ToString("o")
                        $stabilityRecheckedUtc = [DateTime]::UtcNow.ToString("o")
                        break
                    }
                }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ($dialogObservationTimer.Elapsed -lt $dialogObservationTimeout)
    $dialogObservationTimer.Stop()
    if ($dialogs.Count -ne 1 -or -not $isExactForegroundDialog -or $null -eq $observationWaitElapsedMs -or $null -eq $stabilityRecheckedUtc) {
        throw "The exact foreground modal Windows common item dialog for $Id was not stable across two observations within two minutes."
    }
    Write-Host "OBSERVED: stable foreground native dialog captured for $Id."
    $nativeFileDialogs.Add([ordered]@{
        id = $Id
        window_handle = [int64]$dialogs[0]
        owner_window_handle = [int64]$ownerWindow
        owner_window_enabled = $false
        owner_window_exists = $true
        owner_window_visible = $true
        owner_window_title = $ownerTitle
        owner_window_process_id = [int64]$ownerProcessId
        top_level_class = "#32770"
        owning_process_id = [int64]$process.Id
        visible = $true
        foreground_window = $true
        direct_ui_child_observed = $true
        observation_started_utc = $dialogObservationStartedUtc.ToString("o")
        observation_wait_elapsed_ms = $observationWaitElapsedMs
        observed_utc = $observedUtc
        stability_rechecked_utc = $stabilityRecheckedUtc
        same_window_stable = $true
        closed_utc = $null
        closure_rechecked_utc = $null
        closure_stable = $null
        window_exists_after_close = $null
        visible_after_close = $null
        owner_window_enabled_after_close = $null
    })
}

function Confirm-LastNativeFileDialogClosed([string]$Instruction = "") {
    if ($nativeFileDialogs.Count -eq 0) { throw "No observed native file dialog is available for closure confirmation." }
    $dialog = $nativeFileDialogs[$nativeFileDialogs.Count - 1]
    if (-not [string]::IsNullOrWhiteSpace([string]$dialog.closed_utc)) {
        throw "Native file dialog closure was already confirmed: $($dialog.id)"
    }
    if (-not [string]::IsNullOrWhiteSpace($Instruction)) {
        Write-Host ""
        Write-Host "NATIVE DIALOG CLOSURE [$($dialog.id)]"
        Write-Host $Instruction
        $answer = Read-Host "Type CLOSED only after the observed native file dialog and any resulting message are closed"
        if ($answer -cne "CLOSED") { throw "Physical operator did not confirm native file dialog closure for $($dialog.id)." }
    }
    Assert-ProductAlive
    if ([KetchupNativeWindowProbe]::IsExistingWindow([int64]$dialog.window_handle) -or
        [KetchupNativeWindowProbe]::IsVisibleWindow([int64]$dialog.window_handle) -or
        @([KetchupNativeWindowProbe]::VisibleCommonDialogsForProcess($process.Id)).Count -ne 0) {
        throw "The observed native file dialog remained live or visible after completion: $($dialog.id)"
    }
    if (-not [KetchupNativeWindowProbe]::IsEnabledWindow([int64]$dialog.owner_window_handle)) {
        throw "The packaged Ketchup owner window was not re-enabled after native file dialog closure: $($dialog.id)"
    }
    $ownerProcessId = [KetchupNativeWindowProbe]::OwningProcessId([int64]$dialog.owner_window_handle)
    $ownerTitle = [KetchupNativeWindowProbe]::WindowTitle([int64]$dialog.owner_window_handle)
    if (-not [KetchupNativeWindowProbe]::IsExistingWindow([int64]$dialog.owner_window_handle) -or
        -not [KetchupNativeWindowProbe]::IsVisibleWindow([int64]$dialog.owner_window_handle) -or
        $ownerProcessId -ne $process.Id -or
        $ownerTitle -cne "Ketchup") {
        throw "The exact packaged Ketchup owner window did not remain live after native file dialog closure: $($dialog.id)"
    }
    $dialogClosedUtc = [DateTime]::UtcNow
    Start-Sleep -Milliseconds 100
    Assert-ProductAlive
    $ownerProcessId = [KetchupNativeWindowProbe]::OwningProcessId([int64]$dialog.owner_window_handle)
    $ownerTitle = [KetchupNativeWindowProbe]::WindowTitle([int64]$dialog.owner_window_handle)
    if ([KetchupNativeWindowProbe]::IsExistingWindow([int64]$dialog.window_handle) -or
        [KetchupNativeWindowProbe]::IsVisibleWindow([int64]$dialog.window_handle) -or
        @([KetchupNativeWindowProbe]::VisibleCommonDialogsForProcess($process.Id)).Count -ne 0 -or
        -not [KetchupNativeWindowProbe]::IsEnabledWindow([int64]$dialog.owner_window_handle) -or
        -not [KetchupNativeWindowProbe]::IsExistingWindow([int64]$dialog.owner_window_handle) -or
        -not [KetchupNativeWindowProbe]::IsVisibleWindow([int64]$dialog.owner_window_handle) -or
        $ownerProcessId -ne $process.Id -or
        $ownerTitle -cne "Ketchup") {
        throw "Native file dialog destruction and exact Ketchup owner reactivation were not stable across two observations: $($dialog.id)"
    }
    $dialog.closed_utc = $dialogClosedUtc.ToString("o")
    $dialog.closure_rechecked_utc = [DateTime]::UtcNow.ToString("o")
    $dialog.closure_stable = $true
    $dialog.window_exists_after_close = $false
    $dialog.visible_after_close = $false
    $dialog.owner_window_enabled_after_close = $true
    $dialog.owner_window_exists_after_close = $true
    $dialog.owner_window_visible_after_close = $true
    $dialog.owner_window_title_after_close = $ownerTitle
    $dialog.owner_window_process_id_after_close = [int64]$ownerProcessId
}

function Confirm-PhysicalStep([string]$Id, [string]$Instruction) {
    $stepIndex = [Array]::IndexOf($requiredStepIds, $Id)
    if ($stepIndex -lt 0) { throw "Unknown physical workflow step: $Id" }
    $operatorAttestation = $requiredStepAttestations[$stepIndex]
    Write-Host ""
    Write-Host "PHYSICAL STEP [$Id]"
    Write-Host $Instruction
    Write-Host "ATTESTATION: $operatorAttestation"
    $answer = Read-Host "Type PASS only to attest the exact statement above"
    if ($answer -cne "PASS") { throw "Physical operator did not attest PASS for $Id." }
    if ($nativeFileDialogs.Count -gt 0 -and
        [string]::IsNullOrWhiteSpace([string]$nativeFileDialogs[$nativeFileDialogs.Count - 1].closed_utc)) {
        Confirm-LastNativeFileDialogClosed
    }
    Assert-ProductAlive
    $process.Refresh()
    $observedMainWindowHandle = [int64]$process.MainWindowHandle.ToInt64()
    $observedMainWindowProcessId = [KetchupNativeWindowProbe]::OwningProcessId($observedMainWindowHandle)
    if ($observedMainWindowHandle -ne [int64]$desktopProcessEvidence.main_window_handle -or
        $process.MainWindowTitle -cne "Ketchup" -or
        -not [KetchupNativeWindowProbe]::IsExistingWindow($observedMainWindowHandle) -or
        -not [KetchupNativeWindowProbe]::IsVisibleWindow($observedMainWindowHandle) -or
        $observedMainWindowProcessId -ne $process.Id) {
        throw "The exact packaged Ketchup product surface is not live after physical step $Id."
    }
    $stepWindowObservedUtc = [DateTime]::UtcNow
    Start-Sleep -Milliseconds 100
    Assert-ProductAlive
    $process.Refresh()
    $recheckedMainWindowHandle = [int64]$process.MainWindowHandle.ToInt64()
    $recheckedMainWindowProcessId = [KetchupNativeWindowProbe]::OwningProcessId($recheckedMainWindowHandle)
    if ($recheckedMainWindowHandle -ne $observedMainWindowHandle -or
        $process.MainWindowTitle -cne "Ketchup" -or
        -not [KetchupNativeWindowProbe]::IsExistingWindow($recheckedMainWindowHandle) -or
        -not [KetchupNativeWindowProbe]::IsVisibleWindow($recheckedMainWindowHandle) -or
        $recheckedMainWindowProcessId -ne $process.Id) {
        throw "The exact packaged Ketchup product surface was not stable across two observations after physical step $Id."
    }
    $stepWindowStabilityRecheckedUtc = [DateTime]::UtcNow
    $steps.Add([ordered]@{
        id = $Id
        result = "PASS"
        physical_operator_confirmed = $true
        operator_attestation = $operatorAttestation
        product_process_alive = $true
        product_main_window_handle = $recheckedMainWindowHandle
        product_main_window_title = $process.MainWindowTitle
        product_main_window_visible = $true
        product_main_window_process_id = [int64]$recheckedMainWindowProcessId
        product_main_window_observed_utc = $stepWindowObservedUtc.ToString("o")
        product_main_window_stability_rechecked_utc = $stepWindowStabilityRecheckedUtc.ToString("o")
        product_main_window_same_handle_stable = $true
        confirmed_utc = [DateTime]::UtcNow.ToString("o")
    })
}

function Begin-SaveOutputCreation([string]$Id, [string]$Artifact, [string]$Path) {
    if (Test-Path $Path) {
        throw "Successful Save target already existed before its native dialog: $Artifact"
    }
    $saveOutputCreations.Add([ordered]@{
        id = $Id
        artifact = $Artifact
        existed_before = $false
        before_observed_utc = [DateTime]::UtcNow.ToString("o")
        created_utc = $null
        last_write_utc = $null
        after_observed_utc = $null
        file_identity = $null
        size_bytes = $null
        sha256 = $null
        bundle_copy_started_utc = $null
        bundle_source_file_identity = $null
        bundle_source_size_bytes = $null
        bundle_source_sha256 = $null
        bundle_copy_guarded = $null
        bundle_copy_method = $null
        bundle_destination_verified = $null
        bundle_destination_size_bytes = $null
        bundle_destination_sha256 = $null
        bundle_copy_completed_utc = $null
    })
}

function Complete-SaveOutputCreation([string]$Id, [string]$Path) {
    if ($saveOutputCreations.Count -eq 0) { throw "No successful Save output observation is pending." }
    $creation = $saveOutputCreations[$saveOutputCreations.Count - 1]
    if ([string]$creation.id -cne $Id -or -not [string]::IsNullOrWhiteSpace([string]$creation.after_observed_utc)) {
        throw "Successful Save output observation is missing or out of order: $Id"
    }
    Assert-Leaf $Path "successful Save output"
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $fileObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($stream.SafeFileHandle)
        $creation.file_identity = $fileObservation[0]
        $creation.created_utc = $fileObservation[1]
        $creation.last_write_utc = $fileObservation[2]
        $creation.size_bytes = [int64]$stream.Length
        $creation.sha256 = Get-StreamSha256 $stream
        $creation.after_observed_utc = [DateTime]::UtcNow.ToString("o")
    } finally {
        $stream.Dispose()
    }
}

function Copy-GuardedSaveOutputToBundle([string]$Id, [string]$Path, [string]$ExpectedFileIdentity, [string]$DestinationPath) {
    $matches = @($saveOutputCreations | Where-Object { [string]$_.id -ceq $Id })
    if ($matches.Count -ne 1) { throw "Successful Save output is missing from the bundle-copy guard set: $Id" }
    $creation = $matches[0]
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $bundleSourceFileIdentity = [KetchupNativeWindowProbe]::FileIdentityForHandle($stream.SafeFileHandle)
        $bundleSourceSizeBytes = [int64]$stream.Length
        $bundleSourceSha256 = Get-StreamSha256 $stream
        if ($bundleSourceFileIdentity -cne $ExpectedFileIdentity -or
            $bundleSourceSizeBytes -ne [int64]$creation.size_bytes -or
            $bundleSourceSha256 -cne [string]$creation.sha256) {
            throw "Successful Save source changed before immutable-bundle copy: $Id"
        }
        $creation.bundle_copy_started_utc = [DateTime]::UtcNow.ToString("o")
        $creation.bundle_source_file_identity = $bundleSourceFileIdentity
        $creation.bundle_source_size_bytes = $bundleSourceSizeBytes
        $creation.bundle_source_sha256 = $bundleSourceSha256
        $creation.bundle_copy_guarded = $true
        $creation.bundle_copy_method = "guarded-source-stream"
        $saveOutputGuardStreams.Add($stream)
        $destination = [IO.File]::Open($DestinationPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try {
            $stream.Position = 0
            $stream.CopyTo($destination)
            $destination.Flush($true)
            $bundleDestinationSizeBytes = [int64]$destination.Length
            $bundleDestinationSha256 = Get-StreamSha256 $destination
            if ($bundleDestinationSizeBytes -ne $bundleSourceSizeBytes -or
                $bundleDestinationSha256 -cne $bundleSourceSha256) {
                throw "Successful Save immutable-bundle destination differs from its guarded source: $Id"
            }
            $creation.bundle_destination_verified = $true
            $creation.bundle_destination_size_bytes = $bundleDestinationSizeBytes
            $creation.bundle_destination_sha256 = $bundleDestinationSha256
        } finally {
            $destination.Dispose()
        }
        $creation.bundle_copy_completed_utc = [DateTime]::UtcNow.ToString("o")
    } catch {
        $stream.Dispose()
        throw
    }
}

function Begin-FailureInputObservation([string]$Id, [string]$Artifact, [string]$Path, [bool]$ExclusiveLockHeld) {
    Assert-Leaf $Path "failed-operation input"
    if ($ExclusiveLockHeld -and $null -eq $lockStream) {
        throw "Failed Save input is not held under the required exclusive lock: $Artifact"
    }
    if ($failureInputGuardStreams.ContainsKey($Id)) {
        throw "Failed-operation input already has an active observation guard: $Id"
    }
    $stream = if ($ExclusiveLockHeld) {
        $lockStream
    } else {
        [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    }
    try {
        $fileObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($stream.SafeFileHandle)
        $beforeSizeBytes = [int64]$stream.Length
        $beforeSha256 = Get-StreamSha256 $stream
        $failureInputGuardStreams[$Id] = $stream
        $failureInputObservations.Add([ordered]@{
            id = $Id
            artifact = $Artifact
            dialog_id = $null
            exclusive_lock_held = $ExclusiveLockHeld
            observation_handle_guarded = $true
            before_observed_utc = [DateTime]::UtcNow.ToString("o")
            before_size_bytes = $beforeSizeBytes
            before_sha256 = $beforeSha256
            before_file_identity = $fileObservation[0]
            after_observed_utc = $null
            after_size_bytes = $null
            after_sha256 = $null
            after_file_identity = $null
            bundle_copy_started_utc = $null
            bundle_source_file_identity = $null
            bundle_source_size_bytes = $null
            bundle_source_sha256 = $null
            bundle_copy_guarded = $null
            bundle_copy_method = $null
            bundle_destination_verified = $null
            bundle_destination_size_bytes = $null
            bundle_destination_sha256 = $null
            bundle_copy_completed_utc = $null
        })
    } catch {
        $failureInputGuardStreams.Remove($Id)
        if (-not $ExclusiveLockHeld) {
            $stream.Dispose()
        }
        throw
    }
}

function Complete-FailureInputObservation([string]$Id, [string]$DialogId, [string]$Path) {
    if ($failureInputObservations.Count -eq 0) { throw "No failed-operation input observation is pending." }
    $observation = $failureInputObservations[$failureInputObservations.Count - 1]
    if ([string]$observation.id -cne $Id -or -not [string]::IsNullOrWhiteSpace([string]$observation.after_observed_utc)) {
        throw "Failed-operation input observation is missing or out of order: $Id"
    }
    $usesExclusiveLock = $observation.exclusive_lock_held -eq $true
    if ($usesExclusiveLock -and $null -eq $lockStream) {
        throw "Failed Save input lost its exclusive lock during the dialog interaction: $Id"
    }
    if (-not $failureInputGuardStreams.ContainsKey($Id)) {
        throw "Failed-operation input lost its observation guard during the dialog interaction: $Id"
    }
    Assert-Leaf $Path "failed-operation input"
    $stream = $failureInputGuardStreams[$Id]
    try {
        $fileObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($stream.SafeFileHandle)
        $afterSizeBytes = [int64]$stream.Length
        $afterSha256 = Get-StreamSha256 $stream
        $afterFileIdentity = $fileObservation[0]
        if ($afterSizeBytes -ne [int64]$observation.before_size_bytes -or
            $afterSha256 -cne [string]$observation.before_sha256 -or
            $afterFileIdentity -cne [string]$observation.before_file_identity) {
            throw "Failed-operation input changed or was replaced during its dialog interaction: $Id"
        }
        $observation.dialog_id = $DialogId
        $observation.after_observed_utc = [DateTime]::UtcNow.ToString("o")
        $observation.after_size_bytes = $afterSizeBytes
        $observation.after_sha256 = $afterSha256
        $observation.after_file_identity = $afterFileIdentity
    } catch {
        throw
    }
}

function Copy-GuardedFailureInputToBundle([string]$Id, [string]$DestinationPath) {
    $matches = @($failureInputObservations | Where-Object { [string]$_.id -ceq $Id })
    if ($matches.Count -ne 1 -or -not $failureInputGuardStreams.ContainsKey($Id)) {
        throw "Failed-operation input is missing from the bundle-copy guard set: $Id"
    }
    $observation = $matches[0]
    $stream = $failureInputGuardStreams[$Id]
    $fileObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($stream.SafeFileHandle)
    $bundleSourceSizeBytes = [int64]$stream.Length
    $bundleSourceSha256 = Get-StreamSha256 $stream
    if ($fileObservation[0] -cne [string]$observation.after_file_identity -or
        $bundleSourceSizeBytes -ne [int64]$observation.after_size_bytes -or
        $bundleSourceSha256 -cne [string]$observation.after_sha256) {
        throw "Failed-operation input changed before immutable-bundle copy: $Id"
    }
    $observation.bundle_copy_started_utc = [DateTime]::UtcNow.ToString("o")
    $observation.bundle_source_file_identity = $fileObservation[0]
    $observation.bundle_source_size_bytes = $bundleSourceSizeBytes
    $observation.bundle_source_sha256 = $bundleSourceSha256
    $observation.bundle_copy_guarded = $true
    $observation.bundle_copy_method = "guarded-source-stream"
    $destination = [IO.File]::Open($DestinationPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $stream.Position = 0
        $stream.CopyTo($destination)
        $destination.Flush($true)
        $bundleDestinationSizeBytes = [int64]$destination.Length
        $bundleDestinationSha256 = Get-StreamSha256 $destination
        if ($bundleDestinationSizeBytes -ne $bundleSourceSizeBytes -or
            $bundleDestinationSha256 -cne $bundleSourceSha256) {
            throw "Failed-operation immutable-bundle destination differs from its guarded source: $Id"
        }
        $observation.bundle_destination_verified = $true
        $observation.bundle_destination_size_bytes = $bundleDestinationSizeBytes
        $observation.bundle_destination_sha256 = $bundleDestinationSha256
    } finally {
        $destination.Dispose()
    }
    $observation.bundle_copy_completed_utc = [DateTime]::UtcNow.ToString("o")
}

function Capture-PinnedRuntimeModules([string]$Phase) {
    Assert-ProductAlive
    $process.Refresh()
    foreach ($module in @($process.Modules)) {
        $moduleName = [string]$module.ModuleName
        if ($moduleName -cnotmatch '^TK[A-Za-z0-9]+\.dll$') { continue }
        $records = @($packageManifest.files | Where-Object {
            [string]$_.name -ceq $moduleName -and [string]$_.role -ceq "pinned-occt-runtime"
        })
        if ($records.Count -ne 1) {
            throw "The physical-run app loaded OCCT module outside the exact pinned package registry: $moduleName"
        }
        $record = $records[0]
        $expectedPath = [IO.Path]::GetFullPath((Join-Path $packageSnapshot ([string]$record.name)))
        $actualPath = [IO.Path]::GetFullPath($module.FileName)
        if (-not $actualPath.Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
            throw "The physical-run app resolved pinned OCCT module $moduleName outside the verified package root."
        }
        if ((Get-Item $actualPath).Length -ne [int64]$record.size_bytes -or
            (Get-Sha256 $actualPath) -cne [string]$record.sha256) {
            throw "The physical-run app loaded a changed pinned OCCT module: $moduleName"
        }
        $name = [string]$record.name
        if (-not $loadedRuntimeModules.ContainsKey($name)) {
            $loadedRuntimeModules[$name] = [ordered]@{
                name = $name
                package_relative_path = $name
                load_origin = "verified-package-root"
                size_bytes = [int64]$record.size_bytes
                sha256 = [string]$record.sha256
                observation_phases = [Collections.Generic.List[string]]::new()
            }
        }
        $phases = $loadedRuntimeModules[$name].observation_phases
        if ($phases.Contains($Phase)) { throw "Duplicate runtime-module observation phase for $name`: $Phase" }
        $phases.Add($Phase)
    }
}

try {
    [void](New-Item $staging -ItemType Directory)
    [void](New-Item $packageSnapshot -ItemType Directory)
    foreach ($entry in @(Get-ChildItem $PackageDir -Force)) {
        Copy-Item $entry.FullName (Join-Path $packageSnapshot $entry.Name) -Recurse
    }
    & $packager -VerifyOnly -OutputDir $packageSnapshot
    if ($LASTEXITCODE -ne 0) { throw "Isolated technical-package snapshot verification failed." }
    $packageVerifiedBeforeLaunchUtc = [DateTime]::UtcNow.ToString("o")
    $app = Join-Path $packageSnapshot "ketchup-app.exe"
    $worker = Join-Path $packageSnapshot "ketchup-exact-worker.exe"
    $packageManifestPath = Join-Path $packageSnapshot "package-manifest.json"
    $packageManifest = Read-BoundedJson $packageManifestPath $maxProvenanceManifestBytes "captured package manifest"
    $packageGuardedFiles = @(
        @("package-manifest.json") + @($packageManifest.files | ForEach-Object { [string]$_.name }) |
            Sort-Object
    )
    foreach ($name in $packageGuardedFiles) {
        $guardedPath = Join-Path $packageSnapshot $name
        Assert-Leaf $guardedPath "write/delete-guarded package file"
        $packageGuardStreams.Add([IO.File]::Open(
            $guardedPath,
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            [IO.FileShare]::Read
        ))
    }
    $packageGuardsAcquiredUtc = [DateTime]::UtcNow.ToString("o")
    & $packager -VerifyOnly -OutputDir $packageSnapshot
    if ($LASTEXITCODE -ne 0) { throw "Guarded technical-package snapshot verification failed." }
    $packageVerifiedUnderGuardUtc = [DateTime]::UtcNow.ToString("o")
    Invoke-ExactWorkerPing $worker "Isolated packaged exact worker"
    $exactWorkerProbe = [ordered]@{
        request = "PING"
        response = "PONG"
        exit_code = 0
        observed_utc = [DateTime]::UtcNow.ToString("o")
    }

    [void](New-Item $work -ItemType Directory)
    [void](New-Item $foreignWorkingDir -ItemType Directory)
    $baseline = Join-Path $work "baseline.ketchup"
    $newDocument = Join-Path $work "new-document.ketchup"
    $roundtrip = Join-Path $work "roundtrip.ketchup"
    $afterFailedOpen = Join-Path $work "after-failed-open.ketchup"
    $afterFailedSave = Join-Path $work "after-failed-save.ketchup"
    $corrupt = Join-Path $work "corrupt.ketchup"
    $lockedTarget = Join-Path $work "locked-target.ketchup"
    [IO.File]::WriteAllText($corrupt, "not a ketchup document`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($lockedTarget, "KETCHUP_LOCKED_TARGET_SENTINEL`n", [Text.UTF8Encoding]::new($false))
    $lockedTargetExpectedSha256 = Get-Sha256 $lockedTarget

    $process = Start-Process -FilePath $app -WorkingDirectory $foreignWorkingDir -PassThru
    if ($process.SessionId -ne $operatorSessionId) {
        throw "The packaged desktop process did not start in the physical operator's Windows session."
    }
    $observedProcessImagePath = [IO.Path]::GetFullPath($process.MainModule.FileName)
    $expectedProcessImagePath = [IO.Path]::GetFullPath($app)
    if (-not $observedProcessImagePath.Equals($expectedProcessImagePath, [StringComparison]::OrdinalIgnoreCase)) {
        throw "The packaged desktop process image does not originate from the verified package snapshot."
    }
    $observedProcessImage = Get-Item $observedProcessImagePath
    $observedProcessImageSha256 = Get-Sha256 $observedProcessImagePath
    $packageAppRecord = @($packageManifest.files | Where-Object {
        [string]$_.name -ceq "ketchup-app.exe" -and [string]$_.role -ceq "desktop-application"
    })
    if ($packageAppRecord.Count -ne 1 -or
        [int64]$observedProcessImage.Length -ne [int64]$packageAppRecord[0].size_bytes -or
        $observedProcessImageSha256 -cne [string]$packageAppRecord[0].sha256) {
        throw "The live desktop process image differs from the verified package allowlist."
    }
    $processMachineTypes = [KetchupNativeWindowProbe]::ProcessMachineTypes($process.Handle)
    if ($processMachineTypes[0] -ne 0 -or $processMachineTypes[1] -ne 0x8664) {
        throw "The packaged desktop process is not executing natively on AMD64 Windows hardware."
    }
    $desktopProcessEvidence = [ordered]@{
        package_relative_executable = "ketchup-app.exe"
        process_id = [int64]$process.Id
        session_id = [int]$process.SessionId
        load_origin = "verified-package-root"
        working_directory_origin = "fresh-outside-package"
        image_path_matches_verified_package = $true
        image_size_bytes = [int64]$observedProcessImage.Length
        image_sha256 = $observedProcessImageSha256
        process_machine = [int]$processMachineTypes[0]
        native_machine = [int]$processMachineTypes[1]
        native_amd64_execution = $true
        main_window_observation_started_utc = $null
        main_window_wait_elapsed_ms = $null
        main_window_observed_utc = $null
        main_window_stability_rechecked_utc = $null
        main_window_same_handle_stable = $false
        main_window_exists = $false
        main_window_visible = $false
        main_window_process_id = [int64]0
        main_window_observed = $false
        main_window_title = $null
        main_window_handle = [int64]0
        observed_started_utc = [DateTime]::UtcNow.ToString("o")
        observed_exited_utc = $null
    }
    $mainWindowObservationTimeout = [TimeSpan]::FromSeconds(30)
    $desktopProcessEvidence.main_window_observation_started_utc = [DateTime]::UtcNow.ToString("o")
    $mainWindowObservationTimer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Start-Sleep -Milliseconds 250
        Assert-ProductAlive
        $process.Refresh()
    } while ($process.MainWindowHandle -eq [IntPtr]::Zero -and $mainWindowObservationTimer.Elapsed -lt $mainWindowObservationTimeout)
    if ($process.MainWindowHandle -eq [IntPtr]::Zero -or $mainWindowObservationTimer.Elapsed -gt $mainWindowObservationTimeout) {
        $mainWindowObservationTimer.Stop()
        throw "The packaged product window did not appear within the monotonic 30-second limit."
    }
    $observedMainWindowHandle = [int64]$process.MainWindowHandle.ToInt64()
    $observedMainWindowProcessId = [KetchupNativeWindowProbe]::OwningProcessId($observedMainWindowHandle)
    if ($process.MainWindowTitle -cne "Ketchup" -or
        -not [KetchupNativeWindowProbe]::IsExistingWindow($observedMainWindowHandle) -or
        -not [KetchupNativeWindowProbe]::IsVisibleWindow($observedMainWindowHandle) -or
        $observedMainWindowProcessId -ne $process.Id) {
        $mainWindowObservationTimer.Stop()
        throw "The packaged product window does not expose the exact live Ketchup identity."
    }
    $desktopProcessEvidence.main_window_observed_utc = [DateTime]::UtcNow.ToString("o")
    Start-Sleep -Milliseconds 100
    Assert-ProductAlive
    $process.Refresh()
    $recheckedMainWindowHandle = [int64]$process.MainWindowHandle.ToInt64()
    $recheckedMainWindowProcessId = [KetchupNativeWindowProbe]::OwningProcessId($recheckedMainWindowHandle)
    if ($mainWindowObservationTimer.Elapsed -gt $mainWindowObservationTimeout -or
        $recheckedMainWindowHandle -ne $observedMainWindowHandle -or
        $process.MainWindowTitle -cne "Ketchup" -or
        -not [KetchupNativeWindowProbe]::IsExistingWindow($recheckedMainWindowHandle) -or
        -not [KetchupNativeWindowProbe]::IsVisibleWindow($recheckedMainWindowHandle) -or
        $recheckedMainWindowProcessId -ne $process.Id) {
        $mainWindowObservationTimer.Stop()
        throw "The exact live Ketchup product window was not stable across two observations within 30 seconds."
    }
    $mainWindowObservationTimer.Stop()
    $desktopProcessEvidence.main_window_wait_elapsed_ms = [int64]$mainWindowObservationTimer.ElapsedMilliseconds
    $desktopProcessEvidence.main_window_stability_rechecked_utc = [DateTime]::UtcNow.ToString("o")
    $desktopProcessEvidence.main_window_same_handle_stable = $true
    $desktopProcessEvidence.main_window_exists = $true
    $desktopProcessEvidence.main_window_visible = $true
    $desktopProcessEvidence.main_window_process_id = [int64]$recheckedMainWindowProcessId
    $desktopProcessEvidence.main_window_observed = $true
    $desktopProcessEvidence.main_window_title = $process.MainWindowTitle
    $desktopProcessEvidence.main_window_handle = $recheckedMainWindowHandle
    Capture-PinnedRuntimeModules "before-workflow"
    $runtimeModulesVerifiedBeforeWorkflowUtc = [DateTime]::UtcNow.ToString("o")
    if (-not $loadedRuntimeModules.ContainsKey("TKernel.dll")) {
        throw "The physical-run app did not load foundational TKernel.dll from its verified package snapshot."
    }

    Begin-SaveOutputCreation "baseline-save-as-output" "baseline.ketchup" $baseline
    Observe-NativeFileDialog "baseline-save-as" "In Ketchup create a visible exact model using Rectangle followed by Push/Pull. Confirm it remains visible, press Ctrl+Shift+S, leave the native Save As dialog open, and keep it foreground until this console reports OBSERVED."
    Confirm-PhysicalStep "save-as-success" "Complete the open native Save As dialog by saving exactly as '$baseline', confirm the model remains visible, and return here."
    Complete-SaveOutputCreation "baseline-save-as-output" $baseline
    $baselineHash = [string]$saveOutputCreations[0].sha256

    $beforeSaveStream = [IO.File]::Open($baseline, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $beforeSaveObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($beforeSaveStream.SafeFileHandle)
        $beforeSaveFileIdentity = $beforeSaveObservation[0]
        $beforeSaveLastWriteUtc = $beforeSaveObservation[2]
        $beforeSaveLastWrite = Assert-UtcTimestamp $beforeSaveLastWriteUtc "Existing-document Save pre-rewrite file time"
        $beforeSaveSizeBytes = [int64]$beforeSaveStream.Length
        $beforeSaveHash = Get-StreamSha256 $beforeSaveStream
    } finally {
        $beforeSaveStream.Dispose()
    }
    if ($beforeSaveFileIdentity -cne [string]$saveOutputCreations[0].file_identity -or
        $beforeSaveSizeBytes -ne [int64]$saveOutputCreations[0].size_bytes -or
        $beforeSaveHash -cne $baselineHash) {
        throw "Existing-document Save baseline changed before its rewrite observation."
    }
    $saveExistingRewrite = [ordered]@{
        artifact = "baseline.ketchup"
        before_observed_utc = [DateTime]::UtcNow.ToString("o")
        before_last_write_utc = $beforeSaveLastWriteUtc
        before_size_bytes = $beforeSaveSizeBytes
        before_sha256 = $beforeSaveHash
        before_file_identity = $beforeSaveFileIdentity
        after_last_write_utc = $null
        after_size_bytes = $null
        after_sha256 = $null
        after_file_identity = $null
        after_observed_utc = $null
    }
    Confirm-PhysicalStep "save-existing-success" "In Ketchup press Ctrl+S and approve the exact overwrite confirmation. Confirm that the document stays open, then return here."
    $afterSaveStream = [IO.File]::Open($baseline, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $afterSaveObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($afterSaveStream.SafeFileHandle)
        $afterSaveFileIdentity = $afterSaveObservation[0]
        $afterSaveLastWriteUtc = $afterSaveObservation[2]
        $afterSaveLastWrite = Assert-UtcTimestamp $afterSaveLastWriteUtc "Existing-document Save post-rewrite file time"
        $afterSaveSizeBytes = [int64]$afterSaveStream.Length
        $afterSaveHash = Get-StreamSha256 $afterSaveStream
        if ($afterSaveLastWrite -le $beforeSaveLastWrite -or
            $afterSaveSizeBytes -ne $beforeSaveSizeBytes -or
            $afterSaveHash -cne $baselineHash -or
            $afterSaveFileIdentity -ceq [string]$saveExistingRewrite.before_file_identity) {
            throw "Save did not atomically replace the known path with the same canonical bytes."
        }
        $saveExistingRewrite.after_last_write_utc = $afterSaveLastWriteUtc
        $saveExistingRewrite.after_size_bytes = $afterSaveSizeBytes
        $saveExistingRewrite.after_sha256 = $afterSaveHash
        $saveExistingRewrite.after_file_identity = $afterSaveFileIdentity
        $saveExistingRewrite.after_observed_utc = [DateTime]::UtcNow.ToString("o")
    } finally {
        $afterSaveStream.Dispose()
    }

    Begin-SaveOutputCreation "new-document-save-output" "new-document.ketchup" $newDocument
    Observe-NativeFileDialog "new-document-save" "In Ketchup press Ctrl+N, observe the new document, press Ctrl+S, leave the native Save dialog open, and keep it foreground until this console reports OBSERVED."
    Confirm-PhysicalStep "new-save-success" "Complete the open native Save dialog by saving exactly as '$newDocument', confirm the new document remains active, and return here."
    Complete-SaveOutputCreation "new-document-save-output" $newDocument

    Observe-NativeFileDialog "baseline-open" "In Ketchup press Ctrl+O, leave the native Open dialog open, and keep it foreground until this console reports OBSERVED."
    Confirm-LastNativeFileDialogClosed "Complete the open dialog by opening '$baseline', confirm the model remains visible, and return here before opening another dialog."
    Begin-SaveOutputCreation "roundtrip-save-as-output" "roundtrip.ketchup" $roundtrip
    Observe-NativeFileDialog "roundtrip-save-as" "In Ketchup press Ctrl+Shift+S, leave the native Save As dialog open, and keep it foreground until this console reports OBSERVED."
    Confirm-PhysicalStep "open-roundtrip-success" "Complete the open native Save As dialog by saving exactly as '$roundtrip', then return here."
    Complete-SaveOutputCreation "roundtrip-save-as-output" $roundtrip
    if ((Get-Sha256 $roundtrip) -ne $baselineHash) { throw "Open/Save As round-trip changed canonical bytes." }

    Begin-FailureInputObservation "malformed-open-input" "corrupt.ketchup" $corrupt $false
    Observe-NativeFileDialog "malformed-open" "In Ketchup press Ctrl+O, leave the native Open dialog open before selecting malformed '$corrupt', and keep it foreground until this console reports OBSERVED."
    Confirm-LastNativeFileDialogClosed "Complete the open dialog by selecting '$corrupt', observe and dismiss the localized Open failure while the current model remains active, and return here before opening another dialog."
    Complete-FailureInputObservation "malformed-open-input" "malformed-open" $corrupt
    Begin-SaveOutputCreation "post-failed-open-save-as-output" "after-failed-open.ketchup" $afterFailedOpen
    Observe-NativeFileDialog "post-failed-open-save-as" "In Ketchup press Ctrl+Shift+S, leave the native Save As dialog open, and keep it foreground until this console reports OBSERVED."
    Confirm-PhysicalStep "failed-open-continuity" "Complete the open native Save As dialog by saving exactly as '$afterFailedOpen', then return here."
    Complete-SaveOutputCreation "post-failed-open-save-as-output" $afterFailedOpen
    if ((Get-Sha256 $afterFailedOpen) -ne $baselineHash) { throw "Failed Open replaced or changed the active document." }

    $lockStream = [IO.File]::Open($lockedTarget, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    Begin-FailureInputObservation "locked-save-input" "locked-target.ketchup" $lockedTarget $true
    Observe-NativeFileDialog "locked-target-save-as" "In Ketchup press Ctrl+Shift+S, leave the native Save As dialog open before choosing existing '$lockedTarget', and keep it foreground until this console reports OBSERVED."
    Confirm-LastNativeFileDialogClosed "Complete the open dialog by choosing '$lockedTarget'. Accept the native overwrite prompt and Ketchup high-risk confirmation, observe and dismiss the Save failure, then return here before opening another dialog."
    Complete-FailureInputObservation "locked-save-input" "locked-target-save-as" $lockedTarget
    Begin-SaveOutputCreation "post-failed-save-save-as-output" "after-failed-save.ketchup" $afterFailedSave
    Observe-NativeFileDialog "post-failed-save-save-as" "In Ketchup press Ctrl+Shift+S, leave the native Save As dialog open, and keep it foreground until this console reports OBSERVED."
    Confirm-PhysicalStep "failed-save-continuity" "Complete the open native Save As dialog by saving exactly as '$afterFailedSave', then return here."
    Complete-SaveOutputCreation "post-failed-save-save-as-output" $afterFailedSave
    if ((Get-Sha256 $afterFailedSave) -ne $baselineHash) { throw "Failed Save changed the active document." }
    if ([string]$failureInputObservations[1].after_sha256 -cne $lockedTargetExpectedSha256) {
        throw "Failed Save modified the locked destination."
    }
    Capture-PinnedRuntimeModules "after-workflow"
    $runtimeModulesVerifiedAfterWorkflowUtc = [DateTime]::UtcNow.ToString("o")
    if (-not $loadedRuntimeModules["TKernel.dll"].observation_phases.Contains("after-workflow")) {
        throw "The physical-run app did not retain foundational TKernel.dll through the complete workflow."
    }

    Stop-Process -Id $process.Id
    $process.WaitForExit()
    $desktopProcessEvidence.observed_exited_utc = [DateTime]::UtcNow.ToString("o")
    $process = $null

    $documentSemantics = [ordered]@{
        baseline = Invoke-PackagedDocumentInspection $app $baseline "physically modeled baseline"
        new_document = Invoke-PackagedDocumentInspection $app $newDocument "New document"
    }

    & $packager -VerifyOnly -OutputDir $packageSnapshot
    if ($LASTEXITCODE -ne 0) { throw "Technical-package snapshot changed during the physical workflow." }
    $packageVerifiedAfterExitUtc = [DateTime]::UtcNow.ToString("o")
    Copy-Item $packageManifestPath (Join-Path $staging "package-manifest.json")
    Copy-Item $app (Join-Path $staging "ketchup-app.exe")
    Copy-Item $worker (Join-Path $staging "ketchup-exact-worker.exe")
    foreach ($stream in $packageGuardStreams) { $stream.Dispose() }
    $packageGuardStreams.Clear()
    $packageGuardsReleasedUtc = [DateTime]::UtcNow.ToString("o")
    $runnerAfterSha256 = Get-Sha256 $PSCommandPath
    $runnerAfterObservedUtc = [DateTime]::UtcNow.ToString("o")
    if ($runnerAfterSha256 -cne $runnerBeforeSha256) {
        throw "Evidence runner bytes changed during the physical workflow."
    }
    Remove-Item $packageSnapshot -Recurse -Force
    foreach ($creation in $saveOutputCreations) {
        $expectedFileIdentity = if ([string]$creation.id -ceq "baseline-save-as-output") {
            [string]$saveExistingRewrite.after_file_identity
        } else {
            [string]$creation.file_identity
        }
        Copy-GuardedSaveOutputToBundle `
            ([string]$creation.id) `
            (Join-Path $work ([string]$creation.artifact)) `
            $expectedFileIdentity `
            (Join-Path $staging ([string]$creation.artifact))
    }
    foreach ($observation in $failureInputObservations) {
        Copy-GuardedFailureInputToBundle ([string]$observation.id) (Join-Path $staging ([string]$observation.artifact))
    }
    foreach ($stream in $saveOutputGuardStreams) { $stream.Dispose() }
    $saveOutputGuardStreams.Clear()
    foreach ($stream in $failureInputGuardStreams.Values) { $stream.Dispose() }
    $failureInputGuardStreams.Clear()
    $lockStream = $null
    Remove-Item $work -Recurse -Force
    Remove-Item $foreignWorkingDir -Recurse -Force

    $machine = Get-CimInstance Win32_ComputerSystem
    $os = Get-CimInstance Win32_OperatingSystem
    $gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
        [ordered]@{ name = $_.Name; driver_version = $_.DriverVersion; status = $_.Status }
    })
    $records = @($artifactNames | ForEach-Object {
        $path = Join-Path $staging $_
        [ordered]@{ name = $_; size_bytes = (Get-Item $path).Length; sha256 = Get-Sha256 $path }
    })
    $manifest = [ordered]@{
        schema_version = 1
        kind = "physical-release-dialog-evidence"
        status = "PASS"
        platform = "windows-x86_64"
        capture_mode = "interactive-physical-operator"
        run_id = $RunId
        captured_utc = [DateTime]::UtcNow.ToString("o")
        physical_hardware_evidence_complete = $true
        platform_decision = "windows-x86_64-first-release"
        platform_decision_record = "docs/adr/0007-windows-x86-64-first-release.md"
        platform_decision_record_sha256 = Get-Sha256 $platformDecisionRecordPath
        release_eligible = $false
        release_blockers = @(
            "G19-03-canonical-tasks",
            "G19-04-current-tree-hardware-certification"
        )
        operator = [ordered]@{
            name = $OperatorName
            windows_account = $operatorWindowsAccount
            windows_sid = $operatorWindowsSid
            session_id = [int]$operatorSessionId
            attested_physical_interaction = $true
        }
        machine = [ordered]@{
            manufacturer = $machine.Manufacturer
            model = $machine.Model
            total_physical_memory_bytes = [int64]$machine.TotalPhysicalMemory
            os = $os.Caption
            os_build = $os.BuildNumber
            gpus = $gpus
        }
        package_manifest_sha256 = Get-Sha256 (Join-Path $staging "package-manifest.json")
        runner_sha256 = $runnerAfterSha256
        runner_observation = [ordered]@{
            before_sha256 = $runnerBeforeSha256
            before_observed_utc = $runnerBeforeObservedUtc
            after_sha256 = $runnerAfterSha256
            after_observed_utc = $runnerAfterObservedUtc
        }
        package_snapshot = [ordered]@{
            isolated_copy = $true
            guard_mode = "read-handles-deny-write-delete"
            guarded_files = @($packageGuardedFiles)
            guards_acquired_utc = $packageGuardsAcquiredUtc
            guards_released_utc = $packageGuardsReleasedUtc
            verified_before_launch_utc = $packageVerifiedBeforeLaunchUtc
            verified_under_guard_utc = $packageVerifiedUnderGuardUtc
            runtime_modules_verified_before_workflow_utc = $runtimeModulesVerifiedBeforeWorkflowUtc
            runtime_modules_verified_after_workflow_utc = $runtimeModulesVerifiedAfterWorkflowUtc
            verified_after_exit_utc = $packageVerifiedAfterExitUtc
        }
        exact_worker_probe = $exactWorkerProbe
        desktop_process = $desktopProcessEvidence
        native_file_dialogs = @($nativeFileDialogs)
        save_output_creations = @($saveOutputCreations)
        failure_input_observations = @($failureInputObservations)
        loaded_pinned_occt_modules = @($loadedRuntimeModules.Values | Sort-Object name)
        document_semantics = $documentSemantics
        save_existing_rewrite = $saveExistingRewrite
        steps = @($steps)
        artifacts = $records
    }
    Write-Utf8JsonExclusive (Join-Path $staging "evidence-manifest.json") $manifest
    Verify-Evidence $staging
    Move-Item $staging $EvidenceDir
    Write-Host "PASS: immutable Windows-first physical workflow evidence written to $EvidenceDir; G19-03/G19-04 remain."
} finally {
    if ($null -ne $lockStream) { $lockStream.Dispose() }
    foreach ($stream in $packageGuardStreams) { $stream.Dispose() }
    $packageGuardStreams.Clear()
    foreach ($stream in $saveOutputGuardStreams) { $stream.Dispose() }
    $saveOutputGuardStreams.Clear()
    foreach ($stream in $failureInputGuardStreams.Values) { $stream.Dispose() }
    $failureInputGuardStreams.Clear()
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
    if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
}
