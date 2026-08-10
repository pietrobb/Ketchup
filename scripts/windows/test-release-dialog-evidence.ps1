[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$runner = Join-Path $PSScriptRoot "run-release-dialog-evidence.ps1"
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$lockPath = Join-Path $repoRoot "Cargo.lock"
$platformDecisionRecordPath = Join-Path $repoRoot "docs\adr\0007-windows-x86-64-first-release.md"
$occtManifestPath = Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json"
$occtRoot = Join-Path $repoRoot "third_party\occt-install-r0-v1"
$runnerSource = Get-Content $runner -Raw
if ($runnerSource -match 'Type READY only while the requested native file dialog remains open' -or
    $runnerSource -match '(?m)^\s*Observe-NativeFileDialog[^\r\n]*return here' -or
    [regex]::Matches($runnerSource, [regex]::Escape('keep it foreground until this console reports OBSERVED.')).Count -ne 8 -or
    $runnerSource -notmatch [regex]::Escape('Write-Host "OBSERVED: stable foreground native dialog captured for $Id."') -or
    $runnerSource -notmatch [regex]::Escape('The runner will observe the requested foreground dialog automatically; do not return focus to this console.') -or
    $runnerSource -match [regex]::Escape('$dialogObservationDeadline = [DateTime]::UtcNow.AddMinutes(2)') -or
    $runnerSource -match [regex]::Escape('$deadline = [DateTime]::UtcNow.AddSeconds(30)') -or
    $runnerSource -notmatch [regex]::Escape('$mainWindowObservationTimer = [Diagnostics.Stopwatch]::StartNew()') -or
    $runnerSource -notmatch [regex]::Escape('$mainWindowObservationTimeout = [TimeSpan]::FromSeconds(30)') -or
    $runnerSource -notmatch [regex]::Escape('$recheckedMainWindowHandle -ne $observedMainWindowHandle') -or
    $runnerSource -notmatch [regex]::Escape('$desktopProcessEvidence.main_window_same_handle_stable = $true') -or
    $runnerSource -notmatch [regex]::Escape('$dialogObservationTimer = [Diagnostics.Stopwatch]::StartNew()') -or
    $runnerSource -notmatch [regex]::Escape('$dialogObservationTimeout = [TimeSpan]::FromMinutes(15)') -or
    $runnerSource -notmatch [regex]::Escape('} while ($dialogObservationTimer.Elapsed -lt $dialogObservationTimeout)') -or
    $runnerSource -notmatch [regex]::Escape('$candidateObservedUtc = [DateTime]::UtcNow') -or
    $runnerSource -notmatch [regex]::Escape('$stabilityRecheckedUtc = [DateTime]::UtcNow.ToString("o")') -or
    $runnerSource -notmatch [regex]::Escape('$recheckedOwnerProcessId = [KetchupNativeWindowProbe]::OwningProcessId($recheckedOwnerWindow)') -or
    $runnerSource -notmatch [regex]::Escape('$recheckedOwnerTitle = [KetchupNativeWindowProbe]::WindowTitle($recheckedOwnerWindow)') -or
    $runnerSource -notmatch [regex]::Escape('[KetchupNativeWindowProbe]::IsExistingWindow($recheckedOwnerWindow) -and') -or
    $runnerSource -notmatch [regex]::Escape('[KetchupNativeWindowProbe]::IsVisibleWindow($recheckedOwnerWindow) -and') -or
    $runnerSource -notmatch [regex]::Escape('$dialogClosedUtc = [DateTime]::UtcNow') -or
    $runnerSource -notmatch [regex]::Escape('$dialog.closure_rechecked_utc = [DateTime]::UtcNow.ToString("o")') -or
    $runnerSource -notmatch [regex]::Escape('$stepWindowObservedUtc = [DateTime]::UtcNow') -or
    $runnerSource -notmatch [regex]::Escape('$recheckedMainWindowProcessId = [KetchupNativeWindowProbe]::OwningProcessId($recheckedMainWindowHandle)') -or
    $runnerSource -notmatch [regex]::Escape('product_main_window_same_handle_stable = $true') -or
    $runnerSource -match [regex]::Escape('$creation.file_identity = [KetchupNativeWindowProbe]::FileIdentity($Path)') -or
    $runnerSource -match [regex]::Escape('$creation.sha256 = Get-Sha256 $Path') -or
    $runnerSource -notmatch [regex]::Escape('$fileObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($stream.SafeFileHandle)') -or
    $runnerSource -notmatch [regex]::Escape('$creation.sha256 = Get-StreamSha256 $stream') -or
    $runnerSource -notmatch [regex]::Escape('$creation.after_observed_utc = [DateTime]::UtcNow.ToString("o")') -or
    $runnerSource -match [regex]::Escape('$beforeSaveItem = Get-Item $baseline') -or
    $runnerSource -match [regex]::Escape('$afterSaveHash = Get-Sha256 $baseline') -or
    $runnerSource -notmatch [regex]::Escape('$beforeSaveObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($beforeSaveStream.SafeFileHandle)') -or
    $runnerSource -notmatch [regex]::Escape('$beforeSaveHash = Get-StreamSha256 $beforeSaveStream') -or
    $runnerSource -notmatch [regex]::Escape('$afterSaveObservation = [KetchupNativeWindowProbe]::FileObservationForHandle($afterSaveStream.SafeFileHandle)') -or
    $runnerSource -notmatch [regex]::Escape('$afterSaveHash = Get-StreamSha256 $afterSaveStream') -or
    $runnerSource -match [regex]::Escape('$item = Get-Item $Path') -or
    $runnerSource -match [regex]::Escape('$beforeSha256 = if ($ExclusiveLockHeld) { Get-StreamSha256 $lockStream } else { Get-Sha256 $Path }') -or
    $runnerSource -match [regex]::Escape('$afterSha256 = if ($observation.exclusive_lock_held -eq $true) { Get-StreamSha256 $lockStream } else { Get-Sha256 $Path }') -or
    $runnerSource -match [regex]::Escape('[KetchupNativeWindowProbe]::FileIdentity($Path)') -or
    $runnerSource -notmatch [regex]::Escape('$failureInputGuardStreams[$Id] = $stream') -or
    $runnerSource -notmatch [regex]::Escape('$stream = $failureInputGuardStreams[$Id]') -or
    $runnerSource -notmatch [regex]::Escape('observation_handle_guarded = $true') -or
    $runnerSource -notmatch [regex]::Escape('Copy-GuardedFailureInputToBundle ([string]$observation.id)') -or
    $runnerSource -notmatch [regex]::Escape('$stream.CopyTo($destination)') -or
    $runnerSource -match [regex]::Escape('[IO.File]::ReadAllBytes($lockedTarget)') -or
    $runnerSource -notmatch [regex]::Escape('foreach ($stream in $failureInputGuardStreams.Values) { $stream.Dispose() }') -or
    $runnerSource -notmatch [regex]::Escape('$saveExistingRewrite.before_file_identity -cne [string]$saveOutputCreations[0].file_identity') -or
    $runnerSource -notmatch [regex]::Escape('$saveOutputFileIdentities.ContainsKey([string]$saveExistingRewrite.after_file_identity)') -or
    $runnerSource -notmatch [regex]::Escape('Copy-GuardedSaveOutputToBundle `') -or
    $runnerSource -notmatch [regex]::Escape('$creation.bundle_copy_method = "guarded-source-stream"') -or
    $runnerSource -notmatch [regex]::Escape('$observation.bundle_copy_method = "guarded-source-stream"') -or
    $runnerSource -notmatch [regex]::Escape('[IO.FileAccess]::ReadWrite, [IO.FileShare]::None') -or
    $runnerSource -notmatch [regex]::Escape('$bundleDestinationSha256 = Get-StreamSha256 $destination') -or
    $runnerSource -notmatch [regex]::Escape('$creation.bundle_destination_verified = $true') -or
    $runnerSource -notmatch [regex]::Escape('$observation.bundle_destination_verified = $true') -or
    $runnerSource -notmatch [regex]::Escape('if ($stream.Length -lt 16 -or $stream.Length -gt (64 * 1024 * 1024))') -or
    $runnerSource -notmatch [regex]::Escape('$bytes = [byte[]]::new([int]$stream.Length)') -or
    $runnerSource -match [regex]::Escape('$bytes = [IO.File]::ReadAllBytes($Path)') -or
    $runnerSource -notmatch [regex]::Escape('$manifest = Read-BoundedJson $manifestPath $maxEvidenceManifestBytes "release-dialog evidence manifest" ([ref]$manifestHash)') -or
    $runnerSource -notmatch [regex]::Escape('foreach ($name in @($artifactNames + "evidence-manifest.json"))') -or
    $runnerSource -notmatch [regex]::Escape('$evidenceGuardStreams[$name] = $stream') -or
    $runnerSource -notmatch [regex]::Escape('(Get-StreamSha256 $evidenceGuardStreams["evidence-manifest.json"]) -cne $manifestHash') -or
    $runnerSource -notmatch [regex]::Escape('foreach ($stream in $evidenceGuardStreams.Values) { $stream.Dispose() }') -or
    $runnerSource -notmatch [regex]::Escape('$verificationPackageGuardStreams[$name] = [IO.File]::Open(') -or
    $runnerSource -notmatch [regex]::Escape('Assert-ExactNames @($verificationPackageChildren | ForEach-Object { $_.Name }) $expectedVerificationPackageNames "Verification package contents under guard"') -or
    $runnerSource -notmatch [regex]::Escape('(Get-StreamSha256 $verificationPackageGuardStreams["package-manifest.json"]) -cne $packageManifestHash') -or
    $runnerSource -notmatch [regex]::Escape('foreach ($stream in $verificationPackageGuardStreams.Values) { $stream.Dispose() }') -or
    $runnerSource -notmatch [regex]::Escape('$artifactStream = $evidenceGuardStreams[$name]') -or
    $runnerSource -notmatch [regex]::Escape('$canonicalHashes[$name] = Assert-NativeKetchupContainer $evidenceGuardStreams[$name]') -or
    $runnerSource -notmatch [regex]::Escape('$packageManifest = Read-BoundedJsonStream $evidenceGuardStreams["package-manifest.json"] $maxProvenanceManifestBytes "captured package manifest" ([ref]$packageManifestHash)') -or
    $runnerSource -match [regex]::Escape('$packageManifestHash = Get-Sha256 $packageManifestPath') -or
    $runnerSource -match [regex]::Escape('Get-Sha256 $verificationPackageManifest') -or
    $runnerSource -match [regex]::Escape('Get-Sha256 $verificationApp') -or
    $runnerSource -match [regex]::Escape('Get-Sha256 $verificationWorker') -or
    $runnerSource -notmatch [regex]::Escape('(Get-StreamSha256 $verificationPackageGuardStreams["ketchup-app.exe"])') -or
    $runnerSource -notmatch [regex]::Escape('(Get-StreamSha256 $verificationPackageGuardStreams["ketchup-exact-worker.exe"])') -or
    $runnerSource -match [regex]::Escape('Get-Sha256 (Join-Path $Path') -or
    $runnerSource -match [regex]::Escape('[IO.File]::ReadAllBytes((Join-Path $Path') -or
    $runnerSource -notmatch [regex]::Escape('$pinnedOcct = Read-BoundedJson $occtManifestPath $maxProvenanceManifestBytes "pinned R0 OCCT manifest" ([ref]$pinnedOcctHash)') -or
    $runnerSource -match [regex]::Escape('(Get-Sha256 $occtManifestPath)') -or
    $runnerSource -match [regex]::Escape('Copy-Item (Join-Path $work $name) (Join-Path $staging $name)') -or
    $runnerSource -notmatch [regex]::Escape('$creation.bundle_copy_guarded = $true')) {
    throw "Physical runner must use monotonic startup and dialog limits, require a two-sample-stable live Ketchup main window, bind both time-separated live samples of all eight foreground native dialogs and all six physical-step confirmations to that exact live owner, recheck stable dialog destruction without asking the operator to return focus before OBSERVED, atomically capture each successful Save output and both sides of the existing-document rewrite through write/delete-blocking handles, preserve an exact non-aliased baseline identity chain across atomic Save, guard each final source object through immutable-bundle copy, verify destination size/SHA-256 through the still-exclusive destination handle, and bind every offline bundle fingerprint, captured package-manifest parse, native document parse, and terminal continuity check to retained immutable-bundle streams while making retained guarded package streams the sole byte authority for allowlist revalidation, captured executable comparison, worker probe, and packaged-app reinspection."
}
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-m19-dialog-contract-" + [Guid]::NewGuid().ToString("N"))
$verificationPackageDir = $tempRoot + "-verification-package"
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
$stepIds = @(
    "save-as-success",
    "save-existing-success",
    "new-save-success",
    "open-roundtrip-success",
    "failed-open-continuity",
    "failed-save-continuity"
)
$stepAttestations = @(
    "Created a visible Rectangle-to-Push/Pull model and saved it through the observed native Save As dialog.",
    "Saved the existing baseline path while preserving its exact canonical bytes.",
    "Created a fresh New document and saved it through the observed native Save dialog.",
    "Opened the baseline through the observed native Open dialog and saved a byte-identical round-trip.",
    "Selected the frozen malformed Open input, observed its rejection, and preserved the active baseline document.",
    "Selected the exclusively locked Save target, observed the Save failure, and preserved the active baseline document."
)
$nativeDialogIds = @(
    "baseline-save-as",
    "new-document-save",
    "baseline-open",
    "roundtrip-save-as",
    "malformed-open",
    "post-failed-open-save-as",
    "locked-target-save-as",
    "post-failed-save-save-as"
)
$nativeDialogTargetStepIndexes = @(0, 2, 3, 3, 4, 4, 5, 5)
$saveOutputIds = @(
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
$saveOutputDialogIndexes = @(0, 1, 3, 5, 7)
$saveOutputStepIndexes = @(0, 2, 3, 4, 5)

function Get-Sha256([string]$Path) {
    return (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-Utf8([string]$Path, [string]$Content) {
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
}

function Add-Bytes([Collections.Generic.List[byte]]$Target, [byte[]]$Bytes) {
    foreach ($value in $Bytes) { $Target.Add($value) }
}

function Get-HashBytes([byte[]]$Bytes) {
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return $hasher.ComputeHash($Bytes) } finally { $hasher.Dispose() }
}

function Get-CanonicalDigest([string]$PayloadText) {
    $hash = Get-HashBytes ([Text.Encoding]::UTF8.GetBytes($PayloadText))
    return -join ($hash | ForEach-Object { $_.ToString("x2") })
}

function Add-U16([Collections.Generic.List[byte]]$Target, [uint16]$Value) {
    Add-Bytes $Target ([BitConverter]::GetBytes($Value))
}

function Add-U32([Collections.Generic.List[byte]]$Target, [uint32]$Value) {
    Add-Bytes $Target ([BitConverter]::GetBytes($Value))
}

function Add-U64([Collections.Generic.List[byte]]$Target, [uint64]$Value) {
    Add-Bytes $Target ([BitConverter]::GetBytes($Value))
}

function Add-String([Collections.Generic.List[byte]]$Target, [string]$Value) {
    $encoded = [Text.Encoding]::UTF8.GetBytes($Value)
    Add-U32 $Target $encoded.Length
    Add-Bytes $Target $encoded
}

function Write-NativeKetchupFixture([string]$Path, [string]$PayloadText) {
    $payload = [Text.Encoding]::UTF8.GetBytes($PayloadText)
    $manifest = [Collections.Generic.List[byte]]::new()
    Add-U64 $manifest $payload.Length
    Add-Bytes $manifest (Get-HashBytes $payload)
    Add-String $manifest "ketchup.graph.schema.v1"
    Add-String $manifest "ketchup.evaluator.numeric.v1"
    Add-String $manifest "ketchup.tolerance.r0-v1"

    $document = [Collections.Generic.List[byte]]::new()
    Add-Bytes $document ([Text.Encoding]::ASCII.GetBytes("KETCHUPDOC"))
    Add-U16 $document 17
    Add-U32 $document $manifest.Count
    Add-Bytes $document $manifest.ToArray()
    Add-Bytes $document $payload

    $container = [Collections.Generic.List[byte]]::new()
    Add-Bytes $container ([Text.Encoding]::ASCII.GetBytes("KETCHUPCTR"))
    Add-U16 $container 1
    Add-U32 $container 1
    Add-String $container "document.bin"
    $container.Add(1)
    Add-U64 $container $document.Count
    Add-Bytes $container (Get-HashBytes $document.ToArray())
    Add-Bytes $container $document.ToArray()
    [IO.File]::WriteAllBytes($Path, $container.ToArray())
}

function Change-CanonicalPayloadAndRepairContainerHash([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $pathLength = [BitConverter]::ToUInt32($bytes, 16)
    $containerPayloadLengthOffset = 16 + 4 + [int]$pathLength + 1
    $containerHashOffset = $containerPayloadLengthOffset + 8
    $documentOffset = $containerHashOffset + 32
    $documentLength = [BitConverter]::ToUInt64($bytes, $containerPayloadLengthOffset)
    $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 1
    $document = [byte[]]::new([int]$documentLength)
    [Array]::Copy($bytes, $documentOffset, $document, 0, [int]$documentLength)
    $repairedContainerHash = Get-HashBytes $document
    [Array]::Copy($repairedContainerHash, 0, $bytes, $containerHashOffset, 32)
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function Change-ManifestIdentifierAndRepairContainerHash([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $pathLength = [BitConverter]::ToUInt32($bytes, 16)
    $containerPayloadLengthOffset = 16 + 4 + [int]$pathLength + 1
    $containerHashOffset = $containerPayloadLengthOffset + 8
    $documentOffset = $containerHashOffset + 32
    $documentLength = [BitConverter]::ToUInt64($bytes, $containerPayloadLengthOffset)
    $graphIdOffset = $documentOffset + 56 + 4
    $bytes[$graphIdOffset] = [byte][char]'x'
    $document = [byte[]]::new([int]$documentLength)
    [Array]::Copy($bytes, $documentOffset, $document, 0, [int]$documentLength)
    $repairedContainerHash = Get-HashBytes $document
    [Array]::Copy($repairedContainerHash, 0, $bytes, $containerHashOffset, 32)
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function Add-OptionalSidecarEntryPreservingCanonicalPayload([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $entryCount = [BitConverter]::ToUInt32($bytes, 12)
    [Array]::Copy([BitConverter]::GetBytes([uint32]($entryCount + 1)), 0, $bytes, 12, 4)
    $payload = [Text.Encoding]::UTF8.GetBytes("opaque sidecar bytes`n")
    $container = [Collections.Generic.List[byte]]::new()
    Add-Bytes $container $bytes
    Add-String $container "extensions/contract/opaque.bin"
    $container.Add(0)
    Add-U64 $container $payload.Length
    Add-Bytes $container (Get-HashBytes $payload)
    Add-Bytes $container $payload
    [IO.File]::WriteAllBytes($Path, $container.ToArray())
}

function Write-CSharpFixtureExecutable([string]$Path, [string]$Source) {
    $provider = [Microsoft.CSharp.CSharpCodeProvider]::new()
    $parameters = [CodeDom.Compiler.CompilerParameters]::new()
    $parameters.GenerateExecutable = $true
    $parameters.GenerateInMemory = $false
    $parameters.OutputAssembly = $Path
    $parameters.CompilerOptions = "/platform:x64 /optimize+"
    [void]$parameters.ReferencedAssemblies.Add("System.dll")
    try {
        $result = $provider.CompileAssemblyFromSource($parameters, $Source)
    } finally {
        $provider.Dispose()
    }
    if ($result.Errors.HasErrors) {
        throw "Could not compile AMD64 contract fixture: $([string]::Join('; ', @($result.Errors | ForEach-Object { $_.ToString() })))"
    }
}

function Write-InspectorFixtureExecutable([string]$Path) {
    $source = @'
using System;
using System.Globalization;
using System.IO;
using System.Security.Cryptography;
using System.Threading;

public static class ContractDocumentInspector
{
    private static uint ReadU32(byte[] bytes, int offset)
    {
        return BitConverter.ToUInt32(bytes, offset);
    }

    private static string Sha256Hex(byte[] bytes, int offset, int length)
    {
        byte[] value = new byte[length];
        Buffer.BlockCopy(bytes, offset, value, 0, length);
        using (SHA256 hasher = SHA256.Create())
        {
            return BitConverter.ToString(hasher.ComputeHash(value)).Replace("-", "").ToLowerInvariant();
        }
    }

    public static int Main(string[] args)
    {
        if (args.Length != 2 || args[0] != "--inspect-native-document")
        {
            return 2;
        }
        string marker = Environment.GetEnvironmentVariable("KETCHUP_CONTRACT_INSPECT_MARKER");
        if (!String.IsNullOrEmpty(marker))
        {
            File.WriteAllText(marker, "inspection-started");
        }
        int delayMilliseconds;
        if (Int32.TryParse(Environment.GetEnvironmentVariable("KETCHUP_CONTRACT_INSPECT_DELAY_MS"), out delayMilliseconds) && delayMilliseconds > 0)
        {
            Thread.Sleep(delayMilliseconds);
        }
        byte[] container = File.ReadAllBytes(args[1]);
        int pathLength = checked((int)ReadU32(container, 16));
        int documentLengthOffset = 20 + pathLength + 1;
        int documentOffset = documentLengthOffset + 8 + 32;
        int documentLength = checked((int)BitConverter.ToUInt64(container, documentLengthOffset));
        int manifestLength = checked((int)ReadU32(container, documentOffset + 12));
        int canonicalOffset = documentOffset + 16 + manifestLength;
        int canonicalLength = checked(documentLength - 16 - manifestLength);
        string canonicalDigest = Sha256Hex(container, canonicalOffset, canonicalLength);
        string containerDigest = Sha256Hex(container, 0, container.Length);
        bool isNew = String.Equals(Path.GetFileName(args[1]), "new-document.ketchup", StringComparison.Ordinal);
        ulong documentId = isNew ? 102UL : 101UL;
        ulong revision = isNew ? 1UL : 3UL;
        int count = isNew ? 1 : 2;
        Console.WriteLine(
            "{\"schema_version\":17,\"document_id\":" + documentId.ToString(CultureInfo.InvariantCulture) +
            ",\"revision\":" + revision.ToString(CultureInfo.InvariantCulture) +
            ",\"canonical_digest\":\"" + canonicalDigest + "\",\"container_sha256\":\"" + containerDigest +
            "\",\"definitions\":" + count.ToString(CultureInfo.InvariantCulture) +
            ",\"root_occurrences\":" + count.ToString(CultureInfo.InvariantCulture) +
            ",\"profiles\":" + count.ToString(CultureInfo.InvariantCulture) +
            ",\"extrusions\":" + count.ToString(CultureInfo.InvariantCulture) +
            ",\"profile_extrusion_definitions\":" + count.ToString(CultureInfo.InvariantCulture) +
            ",\"visible_profile_extrusion_root_occurrences\":" + count.ToString(CultureInfo.InvariantCulture) + "}");
        return 0;
    }
}
'@
    Write-CSharpFixtureExecutable $Path $source
}

function Write-WorkerFixtureExecutable([string]$Path) {
    $source = @'
using System;

public static class ContractExactWorker
{
    public static int Main()
    {
        string request = Console.ReadLine();
        if (!String.Equals(request, "PING", StringComparison.Ordinal))
        {
            return 2;
        }
        Console.WriteLine("PONG");
        return 0;
    }
}
'@
    Write-CSharpFixtureExecutable $Path $source
}

function Write-PackageManifest {
    $pinnedOcct = Get-Content $occtManifestPath -Raw | ConvertFrom-Json
    $files = [Collections.Generic.List[object]]::new()
    foreach ($binary in @(
        [ordered]@{ name = "ketchup-app.exe"; role = "desktop-application" },
        [ordered]@{ name = "ketchup-exact-worker.exe"; role = "exact-worker" }
    )) {
        $path = Join-Path $tempRoot $binary.name
        $files.Add([ordered]@{
            name = $binary.name
            role = $binary.role
            size_bytes = (Get-Item $path).Length
            sha256 = Get-Sha256 $path
        })
    }
    foreach ($record in @($pinnedOcct.shared_libraries)) {
        $files.Add([ordered]@{
            name = [IO.Path]::GetFileName([string]$record.path)
            role = "pinned-occt-runtime"
            size_bytes = [int64]$record.size_bytes
            sha256 = [string]$record.sha256
        })
    }
    $package = [ordered]@{
        schema_version = 1
        kind = "technical-release-candidate"
        platform = "windows-x86_64"
        platform_decision = "windows-x86_64-first-release"
        platform_decision_record = "docs/adr/0007-windows-x86-64-first-release.md"
        platform_decision_record_sha256 = Get-Sha256 $platformDecisionRecordPath
        release_eligible = $false
        release_blockers = @(
            "G19-02-physical-dialog-workflow",
            "G19-03-canonical-tasks",
            "G19-04-current-tree-hardware-certification"
        )
        cargo_lock_sha256 = Get-Sha256 $lockPath
        occt = [ordered]@{
            version = [string]$pinnedOcct.source.release
            source_commit = [string]$pinnedOcct.source.commit
            manifest_sha256 = Get-Sha256 $occtManifestPath
            build_fingerprint = "occt-8.0.1:b8f597c677811d1f9f4d8a97f5ae2825c0353a42:r0-v1"
            runtime_dll_count = @($pinnedOcct.shared_libraries).Count
        }
        files = @($files | Sort-Object name)
    }
    Write-Utf8 (Join-Path $tempRoot "package-manifest.json") (($package | ConvertTo-Json -Depth 8) + "`n")
}

function Write-Manifest([bool]$ReleaseEligible) {
    $records = @($artifactNames | ForEach-Object {
        $path = Join-Path $tempRoot $_
        [ordered]@{ name = $_; size_bytes = (Get-Item $path).Length; sha256 = Get-Sha256 $path }
    })
    $stepStart = [DateTimeOffset]::Parse("2026-08-09T00:00:00Z")
    $nativeDialogOffsets = @(1, 21, 31, 32, 41, 42, 51, 52)
    $package = Get-Content (Join-Path $tempRoot "package-manifest.json") -Raw | ConvertFrom-Json
    $desktopApp = @($package.files | Where-Object { [string]$_.name -ceq "ketchup-app.exe" })[0]
    $tKernel = @($package.files | Where-Object { [string]$_.name -ceq "TKernel.dll" })[0]
    $manifest = [ordered]@{
        schema_version = 1
        kind = "physical-release-dialog-evidence"
        status = "PASS"
        platform = "windows-x86_64"
        capture_mode = "interactive-physical-operator"
        run_id = "contract-self-test"
        captured_utc = $stepStart.AddSeconds(62).UtcDateTime.ToString("o")
        physical_hardware_evidence_complete = $true
        platform_decision = "windows-x86_64-first-release"
        platform_decision_record = "docs/adr/0007-windows-x86-64-first-release.md"
        platform_decision_record_sha256 = Get-Sha256 $platformDecisionRecordPath
        release_eligible = $ReleaseEligible
        release_blockers = @(
            "G19-03-canonical-tasks",
            "G19-04-current-tree-hardware-certification"
        )
        operator = [ordered]@{
            name = "contract-self-test"
            windows_account = "CONTRACT\operator"
            windows_sid = "S-1-5-21-1000-1001-1002-1003"
            session_id = 3
            attested_physical_interaction = $true
        }
        machine = [ordered]@{
            manufacturer = "Contract Hardware"
            model = "Verifier Fixture"
            total_physical_memory_bytes = 17179869184
            os = "Windows Contract Fixture"
            os_build = "26100"
            gpus = @([ordered]@{ name = "Contract GPU"; driver_version = "1.2.3"; status = "OK" })
        }
        package_manifest_sha256 = Get-Sha256 (Join-Path $tempRoot "package-manifest.json")
        runner_sha256 = Get-Sha256 $runner
        runner_observation = [ordered]@{
            before_sha256 = Get-Sha256 $runner
            before_observed_utc = $stepStart.AddSeconds(-3).UtcDateTime.ToString("o")
            after_sha256 = Get-Sha256 $runner
            after_observed_utc = $stepStart.AddMilliseconds(61500).UtcDateTime.ToString("o")
        }
        package_snapshot = [ordered]@{
            isolated_copy = $true
            guard_mode = "read-handles-deny-write-delete"
            guarded_files = @(
                @("package-manifest.json") + @($package.files | ForEach-Object { [string]$_.name }) |
                    Sort-Object
            )
            guards_acquired_utc = $stepStart.AddMilliseconds(-1750).UtcDateTime.ToString("o")
            guards_released_utc = $stepStart.AddMilliseconds(61250).UtcDateTime.ToString("o")
            verified_before_launch_utc = $stepStart.AddSeconds(-2).UtcDateTime.ToString("o")
            verified_under_guard_utc = $stepStart.AddMilliseconds(-1600).UtcDateTime.ToString("o")
            runtime_modules_verified_before_workflow_utc = $stepStart.AddMilliseconds(-800).UtcDateTime.ToString("o")
            runtime_modules_verified_after_workflow_utc = $stepStart.AddSeconds(60).UtcDateTime.ToString("o")
            verified_after_exit_utc = $stepStart.AddSeconds(61).UtcDateTime.ToString("o")
        }
        exact_worker_probe = [ordered]@{
            request = "PING"
            response = "PONG"
            exit_code = 0
            observed_utc = $stepStart.AddMilliseconds(-1500).UtcDateTime.ToString("o")
        }
        desktop_process = [ordered]@{
            package_relative_executable = "ketchup-app.exe"
            process_id = 4242
            session_id = 3
            load_origin = "verified-package-root"
            working_directory_origin = "fresh-outside-package"
            image_path_matches_verified_package = $true
            image_size_bytes = [int64]$desktopApp.size_bytes
            image_sha256 = [string]$desktopApp.sha256
            process_machine = 0
            native_machine = 0x8664
            native_amd64_execution = $true
            main_window_observation_started_utc = $stepStart.AddMilliseconds(-1200).UtcDateTime.ToString("o")
            main_window_wait_elapsed_ms = 250
            main_window_observed_utc = $stepStart.AddMilliseconds(-1050).UtcDateTime.ToString("o")
            main_window_stability_rechecked_utc = $stepStart.AddMilliseconds(-900).UtcDateTime.ToString("o")
            main_window_same_handle_stable = $true
            main_window_exists = $true
            main_window_visible = $true
            main_window_process_id = 4242
            main_window_observed = $true
            main_window_title = "Ketchup"
            main_window_handle = 1001
            observed_started_utc = $stepStart.AddMilliseconds(-1250).UtcDateTime.ToString("o")
            observed_exited_utc = $stepStart.AddMilliseconds(60500).UtcDateTime.ToString("o")
        }
        native_file_dialogs = @(for ($index = 0; $index -lt $nativeDialogIds.Count; $index++) {
            [ordered]@{
                id = $nativeDialogIds[$index]
                window_handle = 2000 + $index
                owner_window_handle = 1001
                owner_window_enabled = $false
                owner_window_exists = $true
                owner_window_visible = $true
                owner_window_title = "Ketchup"
                owner_window_process_id = 4242
                top_level_class = "#32770"
                owning_process_id = 4242
                visible = $true
                foreground_window = $true
                direct_ui_child_observed = $true
                observation_started_utc = $stepStart.AddSeconds($nativeDialogOffsets[$index]).AddMilliseconds(-250).UtcDateTime.ToString("o")
                observation_wait_elapsed_ms = 250
                observed_utc = $stepStart.AddSeconds($nativeDialogOffsets[$index]).UtcDateTime.ToString("o")
                stability_rechecked_utc = $stepStart.AddSeconds($nativeDialogOffsets[$index]).AddMilliseconds(250).UtcDateTime.ToString("o")
                same_window_stable = $true
                closed_utc = $stepStart.AddSeconds($nativeDialogOffsets[$index]).AddMilliseconds(500).UtcDateTime.ToString("o")
                closure_rechecked_utc = $stepStart.AddSeconds($nativeDialogOffsets[$index]).AddMilliseconds(650).UtcDateTime.ToString("o")
                closure_stable = $true
                window_exists_after_close = $false
                visible_after_close = $false
                owner_window_enabled_after_close = $true
                owner_window_exists_after_close = $true
                owner_window_visible_after_close = $true
                owner_window_title_after_close = "Ketchup"
                owner_window_process_id_after_close = 4242
            }
        })
        save_output_creations = @(for ($index = 0; $index -lt $saveOutputIds.Count; $index++) {
            $artifact = $saveOutputArtifacts[$index]
            $dialogIndex = $saveOutputDialogIndexes[$index]
            $stepIndex = $saveOutputStepIndexes[$index]
            [ordered]@{
                id = $saveOutputIds[$index]
                artifact = $artifact
                existed_before = $false
                before_observed_utc = $stepStart.AddSeconds($nativeDialogOffsets[$dialogIndex]).AddMilliseconds(-500).UtcDateTime.ToString("o")
                created_utc = $stepStart.AddSeconds($nativeDialogOffsets[$dialogIndex]).AddMilliseconds(300).UtcDateTime.ToString("o")
                last_write_utc = $stepStart.AddSeconds($nativeDialogOffsets[$dialogIndex]).AddMilliseconds(400).UtcDateTime.ToString("o")
                after_observed_utc = $stepStart.AddSeconds(9 + 10 * $stepIndex).AddMilliseconds(500).UtcDateTime.ToString("o")
                file_identity = ("12345678:{0:x16}" -f (256 + $index))
                size_bytes = (Get-Item (Join-Path $tempRoot $artifact)).Length
                sha256 = Get-Sha256 (Join-Path $tempRoot $artifact)
                bundle_copy_started_utc = $stepStart.AddMilliseconds(61600).UtcDateTime.ToString("o")
                bundle_source_file_identity = if ($index -eq 0) { "12345678:0000000000000002" } else { ("12345678:{0:x16}" -f (256 + $index)) }
                bundle_source_size_bytes = (Get-Item (Join-Path $tempRoot $artifact)).Length
                bundle_source_sha256 = Get-Sha256 (Join-Path $tempRoot $artifact)
                bundle_copy_guarded = $true
                bundle_copy_method = "guarded-source-stream"
                bundle_destination_verified = $true
                bundle_destination_size_bytes = (Get-Item (Join-Path $tempRoot $artifact)).Length
                bundle_destination_sha256 = Get-Sha256 (Join-Path $tempRoot $artifact)
                bundle_copy_completed_utc = $stepStart.AddMilliseconds(61750).UtcDateTime.ToString("o")
            }
        })
        failure_input_observations = @(
            [ordered]@{
                id = "malformed-open-input"
                artifact = "corrupt.ketchup"
                dialog_id = "malformed-open"
                exclusive_lock_held = $false
                observation_handle_guarded = $true
                before_observed_utc = $stepStart.AddSeconds(40).AddMilliseconds(500).UtcDateTime.ToString("o")
                before_size_bytes = (Get-Item (Join-Path $tempRoot "corrupt.ketchup")).Length
                before_sha256 = Get-Sha256 (Join-Path $tempRoot "corrupt.ketchup")
                before_file_identity = "12345678:0000000000000010"
                after_observed_utc = $stepStart.AddSeconds(41).AddMilliseconds(750).UtcDateTime.ToString("o")
                after_size_bytes = (Get-Item (Join-Path $tempRoot "corrupt.ketchup")).Length
                after_sha256 = Get-Sha256 (Join-Path $tempRoot "corrupt.ketchup")
                after_file_identity = "12345678:0000000000000010"
                bundle_copy_started_utc = $stepStart.AddMilliseconds(61600).UtcDateTime.ToString("o")
                bundle_source_file_identity = "12345678:0000000000000010"
                bundle_source_size_bytes = (Get-Item (Join-Path $tempRoot "corrupt.ketchup")).Length
                bundle_source_sha256 = Get-Sha256 (Join-Path $tempRoot "corrupt.ketchup")
                bundle_copy_guarded = $true
                bundle_copy_method = "guarded-source-stream"
                bundle_destination_verified = $true
                bundle_destination_size_bytes = (Get-Item (Join-Path $tempRoot "corrupt.ketchup")).Length
                bundle_destination_sha256 = Get-Sha256 (Join-Path $tempRoot "corrupt.ketchup")
                bundle_copy_completed_utc = $stepStart.AddMilliseconds(61750).UtcDateTime.ToString("o")
            },
            [ordered]@{
                id = "locked-save-input"
                artifact = "locked-target.ketchup"
                dialog_id = "locked-target-save-as"
                exclusive_lock_held = $true
                observation_handle_guarded = $true
                before_observed_utc = $stepStart.AddSeconds(50).AddMilliseconds(500).UtcDateTime.ToString("o")
                before_size_bytes = (Get-Item (Join-Path $tempRoot "locked-target.ketchup")).Length
                before_sha256 = Get-Sha256 (Join-Path $tempRoot "locked-target.ketchup")
                before_file_identity = "12345678:0000000000000020"
                after_observed_utc = $stepStart.AddSeconds(51).AddMilliseconds(750).UtcDateTime.ToString("o")
                after_size_bytes = (Get-Item (Join-Path $tempRoot "locked-target.ketchup")).Length
                after_sha256 = Get-Sha256 (Join-Path $tempRoot "locked-target.ketchup")
                after_file_identity = "12345678:0000000000000020"
                bundle_copy_started_utc = $stepStart.AddMilliseconds(61600).UtcDateTime.ToString("o")
                bundle_source_file_identity = "12345678:0000000000000020"
                bundle_source_size_bytes = (Get-Item (Join-Path $tempRoot "locked-target.ketchup")).Length
                bundle_source_sha256 = Get-Sha256 (Join-Path $tempRoot "locked-target.ketchup")
                bundle_copy_guarded = $true
                bundle_copy_method = "guarded-source-stream"
                bundle_destination_verified = $true
                bundle_destination_size_bytes = (Get-Item (Join-Path $tempRoot "locked-target.ketchup")).Length
                bundle_destination_sha256 = Get-Sha256 (Join-Path $tempRoot "locked-target.ketchup")
                bundle_copy_completed_utc = $stepStart.AddMilliseconds(61750).UtcDateTime.ToString("o")
            }
        )
        loaded_pinned_occt_modules = @([ordered]@{
            name = [string]$tKernel.name
            package_relative_path = [string]$tKernel.name
            load_origin = "verified-package-root"
            size_bytes = [int64]$tKernel.size_bytes
            sha256 = [string]$tKernel.sha256
            observation_phases = @("before-workflow", "after-workflow")
        })
        document_semantics = [ordered]@{
            baseline = [ordered]@{
                schema_version = 17
                document_id = 101
                revision = 3
                canonical_digest = Get-CanonicalDigest "deterministic canonical document payload`n"
                container_sha256 = Get-Sha256 (Join-Path $tempRoot "baseline.ketchup")
                definitions = 2
                root_occurrences = 2
                profiles = 2
                extrusions = 2
                profile_extrusion_definitions = 2
                visible_profile_extrusion_root_occurrences = 2
            }
            new_document = [ordered]@{
                schema_version = 17
                document_id = 102
                revision = 1
                canonical_digest = Get-CanonicalDigest "new document payload`n"
                container_sha256 = Get-Sha256 (Join-Path $tempRoot "new-document.ketchup")
                definitions = 1
                root_occurrences = 1
                profiles = 1
                extrusions = 1
                profile_extrusion_definitions = 1
                visible_profile_extrusion_root_occurrences = 1
            }
        }
        save_existing_rewrite = [ordered]@{
            artifact = "baseline.ketchup"
            before_observed_utc = $stepStart.AddSeconds(10).UtcDateTime.ToString("o")
            before_last_write_utc = $stepStart.AddSeconds(8).UtcDateTime.ToString("o")
            before_size_bytes = (Get-Item (Join-Path $tempRoot "baseline.ketchup")).Length
            before_sha256 = Get-Sha256 (Join-Path $tempRoot "baseline.ketchup")
            before_file_identity = "12345678:0000000000000100"
            after_last_write_utc = $stepStart.AddSeconds(18).UtcDateTime.ToString("o")
            after_size_bytes = (Get-Item (Join-Path $tempRoot "baseline.ketchup")).Length
            after_sha256 = Get-Sha256 (Join-Path $tempRoot "baseline.ketchup")
            after_file_identity = "12345678:0000000000000002"
            after_observed_utc = $stepStart.AddSeconds(20).UtcDateTime.ToString("o")
        }
        steps = @(for ($index = 0; $index -lt $stepIds.Count; $index++) {
            [ordered]@{
                id = $stepIds[$index]
                result = "PASS"
                physical_operator_confirmed = $true
                operator_attestation = $stepAttestations[$index]
                product_process_alive = $true
                product_main_window_handle = 1001
                product_main_window_title = "Ketchup"
                product_main_window_visible = $true
                product_main_window_process_id = 4242
                product_main_window_observed_utc = $stepStart.AddSeconds(9 + 10 * $index).AddMilliseconds(-300).UtcDateTime.ToString("o")
                product_main_window_stability_rechecked_utc = $stepStart.AddSeconds(9 + 10 * $index).AddMilliseconds(-150).UtcDateTime.ToString("o")
                product_main_window_same_handle_stable = $true
                confirmed_utc = $stepStart.AddSeconds(9 + 10 * $index).UtcDateTime.ToString("o")
            }
        })
        artifacts = $records
    }
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($manifest | ConvertTo-Json -Depth 12) + "`n")
}

function Invoke-VerifyExpectingFailure([string]$Reason, [string]$ExpectedError = "") {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & powershell -NoProfile -ExecutionPolicy Bypass -File $runner -VerifyOnly -EvidenceDir $tempRoot -PackageDir $verificationPackageDir *>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -eq 0) { throw "Evidence verifier accepted $Reason." }
    if ($ExpectedError.Length -gt 0 -and ($output | Out-String) -notlike "*$ExpectedError*") {
        throw "Evidence verifier rejected $Reason for the wrong reason. Expected: $ExpectedError Actual: $(($output | Out-String).Trim())"
    }
}

try {
    [void](New-Item $tempRoot -ItemType Directory)
    Write-InspectorFixtureExecutable (Join-Path $tempRoot "ketchup-app.exe")
    $inspectorFixtureBytes = [IO.File]::ReadAllBytes((Join-Path $tempRoot "ketchup-app.exe"))
    Write-WorkerFixtureExecutable (Join-Path $tempRoot "ketchup-exact-worker.exe")
    Write-PackageManifest
    [void](New-Item $verificationPackageDir -ItemType Directory)
    foreach ($name in @("ketchup-app.exe", "ketchup-exact-worker.exe", "package-manifest.json")) {
        Copy-Item (Join-Path $tempRoot $name) (Join-Path $verificationPackageDir $name)
    }
    $pinnedOcct = Get-Content $occtManifestPath -Raw | ConvertFrom-Json
    foreach ($record in @($pinnedOcct.shared_libraries)) {
        Copy-Item `
            (Join-Path $occtRoot ([string]$record.path)) `
            (Join-Path $verificationPackageDir ([IO.Path]::GetFileName([string]$record.path)))
    }
    $canonical = "deterministic canonical document payload`n"
    foreach ($name in @("baseline.ketchup", "roundtrip.ketchup", "after-failed-open.ketchup", "after-failed-save.ketchup")) {
        Write-NativeKetchupFixture (Join-Path $tempRoot $name) $canonical
    }
    Write-NativeKetchupFixture (Join-Path $tempRoot "new-document.ketchup") "new document payload`n"
    Write-Utf8 (Join-Path $tempRoot "corrupt.ketchup") "not a ketchup document`n"
    Write-Utf8 (Join-Path $tempRoot "locked-target.ketchup") "KETCHUP_LOCKED_TARGET_SENTINEL`n"
    Write-Manifest $false

    & $runner -VerifyOnly -EvidenceDir $tempRoot -PackageDir $verificationPackageDir

    $guardMarkerPath = $tempRoot + "-guard-marker"
    $guardStdoutPath = $tempRoot + "-guard-stdout.log"
    $guardStderrPath = $tempRoot + "-guard-stderr.log"
    $verificationProcess = $null
    $previousInspectMarker = $env:KETCHUP_CONTRACT_INSPECT_MARKER
    $previousInspectDelay = $env:KETCHUP_CONTRACT_INSPECT_DELAY_MS
    try {
        $env:KETCHUP_CONTRACT_INSPECT_MARKER = $guardMarkerPath
        $env:KETCHUP_CONTRACT_INSPECT_DELAY_MS = "2000"
        $verificationProcess = Start-Process `
            -FilePath "powershell" `
            -ArgumentList @(
                "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runner,
                "-VerifyOnly", "-EvidenceDir", $tempRoot, "-PackageDir", $verificationPackageDir
            ) `
            -RedirectStandardOutput $guardStdoutPath `
            -RedirectStandardError $guardStderrPath `
            -PassThru
        $guardDeadline = [DateTime]::UtcNow.AddSeconds(30)
        while (-not (Test-Path $guardMarkerPath) -and
               -not $verificationProcess.HasExited -and
               [DateTime]::UtcNow -lt $guardDeadline) {
            Start-Sleep -Milliseconds 25
        }
        if (-not (Test-Path $guardMarkerPath)) {
            throw "Evidence verifier did not reach guarded packaged-app inspection."
        }
        $writeStream = $null
        $writeBlocked = $false
        try {
            $writeStream = [IO.File]::Open(
                (Join-Path $tempRoot "baseline.ketchup"),
                [IO.FileMode]::Open,
                [IO.FileAccess]::Write,
                [IO.FileShare]::ReadWrite
            )
        } catch [IO.IOException] {
            $writeBlocked = $true
        } finally {
            if ($null -ne $writeStream) { $writeStream.Dispose() }
        }
        if (-not $writeBlocked) {
            throw "Evidence verifier did not deny concurrent mutation of a guarded bundle artifact."
        }
        $packageWriteStream = $null
        $packageWriteBlocked = $false
        try {
            $packageWriteStream = [IO.File]::Open(
                (Join-Path $verificationPackageDir "TKernel.dll"),
                [IO.FileMode]::Open,
                [IO.FileAccess]::Write,
                [IO.FileShare]::ReadWrite
            )
        } catch [IO.IOException] {
            $packageWriteBlocked = $true
        } finally {
            if ($null -ne $packageWriteStream) { $packageWriteStream.Dispose() }
        }
        if (-not $packageWriteBlocked) {
            throw "Evidence verifier did not deny concurrent mutation of the guarded verification package."
        }
        $verificationProcess.WaitForExit()
        $guardStdout = [IO.File]::ReadAllText($guardStdoutPath)
        $guardStderr = [IO.File]::ReadAllText($guardStderrPath)
        if ($guardStderr.Length -ne 0 -or
            $guardStdout -notlike "*Verified immutable Windows-first physical release-dialog evidence; G19-03/G19-04 remain.*") {
            throw "Guarded evidence verifier failed: stdout=$guardStdout stderr=$guardStderr"
        }
    } finally {
        $env:KETCHUP_CONTRACT_INSPECT_MARKER = $previousInspectMarker
        $env:KETCHUP_CONTRACT_INSPECT_DELAY_MS = $previousInspectDelay
        if ($null -ne $verificationProcess -and -not $verificationProcess.HasExited) {
            $verificationProcess.Kill()
            $verificationProcess.WaitForExit()
        }
        Remove-Item $guardMarkerPath, $guardStdoutPath, $guardStderrPath -Force -ErrorAction SilentlyContinue
    }

    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") ("{" + (" " * (1024 * 1024)) + "}")
    Invoke-VerifyExpectingFailure "an oversized release-dialog evidence manifest" "release-dialog evidence manifest exceeds its 1048576-byte limit."
    Write-Manifest $false

    [IO.File]::WriteAllBytes((Join-Path $tempRoot "evidence-manifest.json"), [byte[]]@(0x7b, 0x22, 0xff, 0x22, 0x7d))
    Invoke-VerifyExpectingFailure "a release-dialog evidence manifest containing invalid UTF-8" "release-dialog evidence manifest is not valid UTF-8."
    Write-Manifest $false

    $oversizedNativePath = Join-Path $tempRoot "baseline.ketchup"
    $oversizedNativeStream = [IO.File]::Open($oversizedNativePath, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $oversizedNativeStream.SetLength((64 * 1024 * 1024) + 1) } finally { $oversizedNativeStream.Dispose() }
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "an oversized native document artifact" "Captured native document baseline.ketchup is outside the native container size envelope."
    Write-NativeKetchupFixture $oversizedNativePath $canonical
    Write-Manifest $false

    $changedRunnerEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $changedRunnerEvidence.runner_observation.after_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($changedRunnerEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "runner bytes changed after the physical workflow" "Evidence runner bytes do not match both workflow-bound observations and the current verifier."
    Write-Manifest $false

    $lateRunnerObservationEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $lateRunnerObservationEvidence.runner_observation.before_observed_utc = $lateRunnerObservationEvidence.package_snapshot.verified_before_launch_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($lateRunnerObservationEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "runner bytes observed only after package verification began" "Evidence runner byte observations do not enclose the complete physical workflow."
    Write-Manifest $false

    $detachedStepSurfaceEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $detachedStepSurfaceEvidence.steps[1].product_main_window_handle = 9998
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($detachedStepSurfaceEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a physical workflow step detached from the exact live packaged Ketchup main window" `
        "Physical workflow step is not bound to the live packaged Ketchup product surface"
    Write-Manifest $false

    $unstableStepSurfaceEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unstableStepSurfaceEvidence.steps[1].product_main_window_same_handle_stable = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unstableStepSurfaceEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a physical workflow step bound to only one live packaged Ketchup main-window sample" `
        "Physical workflow step is not bound to the live packaged Ketchup product surface"
    Write-Manifest $false

    $earlyStepSurfaceRecheckEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $stepSurfaceObservedUtc = [DateTimeOffset]::Parse([string]$earlyStepSurfaceRecheckEvidence.steps[1].product_main_window_observed_utc)
    $earlyStepSurfaceRecheckEvidence.steps[1].product_main_window_stability_rechecked_utc = $stepSurfaceObservedUtc.AddMilliseconds(50).UtcDateTime.ToString("o")
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($earlyStepSurfaceRecheckEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a physical workflow step whose product surface was rechecked before the required 100-millisecond stability interval" `
        "Physical workflow step product surface was not stable across two time-separated live samples"
    Write-Manifest $false

    $foreignStepSurfaceEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $foreignStepSurfaceEvidence.steps[1].product_main_window_process_id = 9999
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($foreignStepSurfaceEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a physical workflow step whose matching main-window HWND belonged to another process" `
        "Physical workflow step is not bound to the live packaged Ketchup product surface"
    Write-Manifest $false

    $forgedFailureAttestationEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedFailureAttestationEvidence.steps[4].operator_attestation = "Canceled the malformed Open dialog without selecting its input."
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedFailureAttestationEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a canceled malformed Open dialog presented as an observed product rejection" `
        "Physical workflow step is incomplete: failed-open-continuity"
    Write-Manifest $false

    $forgedLockedSaveAttestationEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedLockedSaveAttestationEvidence.steps[5].operator_attestation = "Canceled the locked-target Save As dialog before attempting the write."
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedLockedSaveAttestationEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a canceled locked-target Save As dialog presented as an observed Save failure" `
        "Physical workflow step is incomplete: failed-save-continuity"
    Write-Manifest $false

    $preExistingSaveTargetEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $preExistingSaveTargetEvidence.save_output_creations[0].existed_before = $true
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($preExistingSaveTargetEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a successful Save As result whose target already existed before its native dialog" "Successful Save output was not created at its exact previously absent target"
    Write-Manifest $false

    $aliasedSaveOutputEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $aliasedSaveOutputEvidence.save_output_creations[1].file_identity = $aliasedSaveOutputEvidence.save_output_creations[0].file_identity
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($aliasedSaveOutputEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "two successful Save outputs aliased to the same filesystem object" "Successful Save outputs alias the same filesystem object"
    Write-Manifest $false

    $detachedBaselineIdentityEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $detachedBaselineIdentityEvidence.save_existing_rewrite.before_file_identity = "12345678:0000000000000099"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($detachedBaselineIdentityEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an existing-document Save rewrite detached from its baseline Save As object" "Existing-document Save rewrite is not chained to the exact baseline Save As filesystem object."
    Write-Manifest $false

    $postRewriteAliasEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $postRewriteAliasEvidence.save_existing_rewrite.after_file_identity = $postRewriteAliasEvidence.save_output_creations[2].file_identity
    $postRewriteAliasEvidence.save_output_creations[0].bundle_source_file_identity = $postRewriteAliasEvidence.save_existing_rewrite.after_file_identity
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($postRewriteAliasEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a post-rewrite baseline hard-linked to a later successful Save As output" "Post-rewrite baseline aliases a successful Save output filesystem object."
    Write-Manifest $false

    $replacedBeforeBundleCopyEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $replacedBeforeBundleCopyEvidence.save_output_creations[1].bundle_source_file_identity = "12345678:0000000000000099"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($replacedBeforeBundleCopyEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a successful Save source replaced before immutable-bundle copy" "Successful Save source changed before immutable-bundle copy"
    Write-Manifest $false

    $changedBeforeBundleCopyEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $changedBeforeBundleCopyEvidence.save_output_creations[1].bundle_source_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($changedBeforeBundleCopyEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "successful Save bytes changed before immutable-bundle copy" "Successful Save source changed before immutable-bundle copy"
    Write-Manifest $false

    $pathCopiedSaveEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $pathCopiedSaveEvidence.save_output_creations[1].bundle_copy_method = "path-copy"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($pathCopiedSaveEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a successful Save artifact copied through a new path lookup" "Successful Save source changed before immutable-bundle copy"
    Write-Manifest $false

    $forgedSaveDestinationEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedSaveDestinationEvidence.save_output_creations[1].bundle_destination_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedSaveDestinationEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a successful Save bundle destination detached from its guarded source" "Successful Save immutable-bundle destination differs from its guarded source"
    Write-Manifest $false

    $preDialogSaveCreationEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $preDialogSaveCreationEvidence.save_output_creations[0].created_utc = $preDialogSaveCreationEvidence.native_file_dialogs[0].observed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($preDialogSaveCreationEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a successful Save output whose filesystem object existed before its native dialog was observed" "Successful Save output creation chronology is invalid"
    Write-Manifest $false

    $lateSaveTargetCheckEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $lateSaveTargetCheckEvidence.save_output_creations[0].before_observed_utc = $lateSaveTargetCheckEvidence.native_file_dialogs[0].observed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($lateSaveTargetCheckEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a Save As target absence check performed only after the native dialog opened" "Successful Save output creation chronology is invalid"
    Write-Manifest $false

    $unguardedFailureInputEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unguardedFailureInputEvidence.failure_input_observations[0].observation_handle_guarded = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unguardedFailureInputEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a malformed-Open input observed through independent unguarded path probes" "Failed-operation input fingerprint is invalid"
    Write-Manifest $false

    $forgedFailureInputHashEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedFailureInputHashEvidence.failure_input_observations[0].before_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedFailureInputHashEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a malformed-Open input with a forged pre-dialog fingerprint" "Failed-operation input fingerprint is invalid"
    Write-Manifest $false

    $replacedFailureInputEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $replacedFailureInputEvidence.failure_input_observations[0].after_file_identity = "12345678:0000000000000011"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($replacedFailureInputEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a malformed-Open input replaced after its native dialog" "Failed-operation input was replaced during its dialog interaction"
    Write-Manifest $false

    $replacedFailureInputBeforeBundleEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $replacedFailureInputBeforeBundleEvidence.failure_input_observations[0].bundle_source_file_identity = "12345678:0000000000000099"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($replacedFailureInputBeforeBundleEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a malformed-Open input replaced before immutable-bundle copy" "Failed-operation input changed before immutable-bundle copy"
    Write-Manifest $false

    $changedFailureInputBeforeBundleEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $changedFailureInputBeforeBundleEvidence.failure_input_observations[1].bundle_source_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($changedFailureInputBeforeBundleEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "locked-Save input bytes changed before immutable-bundle copy" "Failed-operation input changed before immutable-bundle copy"
    Write-Manifest $false

    $pathCopiedFailureInputEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $pathCopiedFailureInputEvidence.failure_input_observations[0].bundle_copy_method = "path-copy"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($pathCopiedFailureInputEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a malformed-Open input copied through a new path lookup" "Failed-operation input changed before immutable-bundle copy"
    Write-Manifest $false

    $forgedFailureDestinationEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedFailureDestinationEvidence.failure_input_observations[1].bundle_destination_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedFailureDestinationEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a failed-operation bundle destination detached from its guarded source" "Failed-operation immutable-bundle destination differs from its guarded source"
    Write-Manifest $false

    $unlockedFailureInputEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unlockedFailureInputEvidence.failure_input_observations[1].exclusive_lock_held = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unlockedFailureInputEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a locked-Save input observed without the required exclusive lock" "Failed-operation input fingerprint is invalid"
    Write-Manifest $false

    $lateFailureInputEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $lateFailureInputEvidence.failure_input_observations[1].after_observed_utc = $lateFailureInputEvidence.native_file_dialogs[7].observed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($lateFailureInputEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a locked-Save input checked only after the continuity dialog opened" "Failed-operation input observation chronology is invalid"
    Write-Manifest $false

    $nonIncreasingRewriteEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $nonIncreasingRewriteEvidence.save_existing_rewrite.after_last_write_utc = $nonIncreasingRewriteEvidence.save_existing_rewrite.before_last_write_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($nonIncreasingRewriteEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a claimed existing-document Save without a later atomic rewrite" "Existing-document Save rewrite chronology is invalid."
    Write-Manifest $false

    $forgedRewriteSizeEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedRewriteSizeEvidence.save_existing_rewrite.after_size_bytes = [int64]$forgedRewriteSizeEvidence.save_existing_rewrite.after_size_bytes + 1
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedRewriteSizeEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an existing-document Save rewrite with a forged single-handle size observation" "Existing-document Save rewrite fingerprint does not match the captured baseline artifact."
    Write-Manifest $false

    $forgedRewriteHashEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedRewriteHashEvidence.save_existing_rewrite.before_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedRewriteHashEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a forged existing-document Save pre-rewrite fingerprint" "Existing-document Save rewrite fingerprint does not match the captured baseline artifact."
    Write-Manifest $false

    $sameFileIdentityEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $sameFileIdentityEvidence.save_existing_rewrite.after_file_identity = $sameFileIdentityEvidence.save_existing_rewrite.before_file_identity
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($sameFileIdentityEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an in-place existing-document Save presented as an atomic replacement" "Existing-document Save did not replace the exact filesystem object."
    Write-Manifest $false

    $verificationRuntime = Join-Path $verificationPackageDir "TKernel.dll"
    $verificationRuntimeBytes = [IO.File]::ReadAllBytes($verificationRuntime)
    $changedVerificationRuntimeBytes = [byte[]]$verificationRuntimeBytes.Clone()
    $changedVerificationRuntimeBytes[0] = $changedVerificationRuntimeBytes[0] -bxor 1
    [IO.File]::WriteAllBytes($verificationRuntime, $changedVerificationRuntimeBytes)
    Invoke-VerifyExpectingFailure `
        "a verification package whose pinned OCCT runtime differs from its exact allowlist" `
        "Packaged runtime fingerprint mismatch: TKernel.dll"
    [IO.File]::WriteAllBytes($verificationRuntime, $verificationRuntimeBytes)

    $forgedWorkerProbeEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedWorkerProbeEvidence.exact_worker_probe.response = "PONG-FORGED"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedWorkerProbeEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a forged exact-worker process probe" "Evidence lacks a successful isolated exact-worker PING/PONG probe."
    Write-Manifest $false

    $forgedDesktopProcessEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedDesktopProcessEvidence.desktop_process.load_origin = "external-runtime"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedDesktopProcessEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a desktop process attributed outside the verified package root" `
        "exact packaged desktop process"
    Write-Manifest $false

    $foreignProcessImageEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $foreignProcessImageEvidence.desktop_process.image_path_matches_verified_package = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($foreignProcessImageEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a desktop process whose observed image path differs from the verified package snapshot" `
        "exact packaged desktop process"
    Write-Manifest $false

    $emulatedProcessEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $emulatedProcessEvidence.desktop_process.process_machine = 0x8664
    $emulatedProcessEvidence.desktop_process.native_machine = 0xAA64
    $emulatedProcessEvidence.desktop_process.native_amd64_execution = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($emulatedProcessEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "an AMD64 desktop image executing through ARM64 emulation instead of native release hardware" `
        "exact packaged desktop process"
    Write-Manifest $false

    $unboundedMainWindowWaitEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unboundedMainWindowWaitEvidence.desktop_process.main_window_wait_elapsed_ms = 30001
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unboundedMainWindowWaitEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a packaged desktop main window observed after exceeding its monotonic 30-second startup wait" `
        "exact packaged desktop process"
    Write-Manifest $false

    $forgedProcessImageEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedProcessImageEvidence.desktop_process.image_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedProcessImageEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a desktop process with a forged image fingerprint" `
        "Observed desktop process image differs from the verified technical package."
    Write-Manifest $false

    $genericMainWindowEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $genericMainWindowEvidence.desktop_process.main_window_title = "Unrelated Window"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($genericMainWindowEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a generic main window presented as the packaged Ketchup product surface" `
        "exact packaged desktop process"
    Write-Manifest $false

    $unstableMainWindowEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unstableMainWindowEvidence.desktop_process.main_window_same_handle_stable = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unstableMainWindowEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a transient packaged main-window HWND presented without the required time-separated stability recheck" `
        "exact packaged desktop process"
    Write-Manifest $false

    $singleSampleMainWindowEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $mainWindowObservedUtc = [DateTimeOffset]::Parse([string]$singleSampleMainWindowEvidence.desktop_process.main_window_observed_utc)
    $singleSampleMainWindowEvidence.desktop_process.main_window_stability_rechecked_utc = $mainWindowObservedUtc.AddMilliseconds(50).UtcDateTime.ToString("o")
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($singleSampleMainWindowEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a packaged main window rechecked before the required 100-millisecond stability interval" `
        "exact packaged desktop process"
    Write-Manifest $false

    $missingNativeDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $missingNativeDialogEvidence.native_file_dialogs = @($missingNativeDialogEvidence.native_file_dialogs | Select-Object -Skip 1)
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($missingNativeDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "physical workflow evidence missing the baseline native Save As observation" "Observed native file dialogs differs from its exact allowlist."
    Write-Manifest $false

    $foreignNativeDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $foreignNativeDialogEvidence.native_file_dialogs[0].owning_process_id = 9999
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($foreignNativeDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a native file dialog owned by a process other than the packaged desktop application" "Native file-dialog observation is not bound to the packaged desktop process"
    Write-Manifest $false

    $foreignOwnerDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $foreignOwnerDialogEvidence.native_file_dialogs[0].owner_window_handle = 9998
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($foreignOwnerDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a same-process native file dialog not owned by the exact packaged Ketchup main window" "Native file-dialog observation is not bound to the packaged desktop process"
    Write-Manifest $false

    $modelessDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $modelessDialogEvidence.native_file_dialogs[0].owner_window_enabled = $true
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($modelessDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a process-owned common-item dialog that did not modally disable its Ketchup owner window" "Native file-dialog observation did not disable its packaged Ketchup owner window"
    Write-Manifest $false

    $foreignLiveOwnerDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $foreignLiveOwnerDialogEvidence.native_file_dialogs[0].owner_window_process_id = 9999
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($foreignLiveOwnerDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a native dialog whose matching owner HWND belonged to another live process during observation" "live exact packaged Ketchup owner window"
    Write-Manifest $false

    $stillOpenDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $stillOpenDialogEvidence.native_file_dialogs[0].visible_after_close = $true
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($stillOpenDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a native file dialog replayed as a later completed interaction while still visible" "Native file-dialog observation lacks stable time-separated destruction and owner reactivation"
    Write-Manifest $false

    $hiddenLiveDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $hiddenLiveDialogEvidence.native_file_dialogs[0].window_exists_after_close = $true
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($hiddenLiveDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a hidden but still-existing native file-dialog HWND replayed as completed" "stable time-separated destruction and owner reactivation"
    Write-Manifest $false

    $unstableClosureEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unstableClosureEvidence.native_file_dialogs[0].closure_stable = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unstableClosureEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a single-sample dialog closure presented without the required time-separated destruction recheck" "stable time-separated destruction and owner reactivation"
    Write-Manifest $false

    $disabledAfterCloseEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $disabledAfterCloseEvidence.native_file_dialogs[0].owner_window_enabled_after_close = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($disabledAfterCloseEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a closed native file dialog whose exact Ketchup owner remained disabled" "Native file-dialog observation lacks stable time-separated destruction and owner reactivation"
    Write-Manifest $false

    $foreignOwnerAfterCloseEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $foreignOwnerAfterCloseEvidence.native_file_dialogs[0].owner_window_process_id_after_close = 9999
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($foreignOwnerAfterCloseEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a native dialog closure whose surviving owner window belongs to another process" "Native file-dialog closure is not bound to the still-live exact packaged Ketchup owner window"
    Write-Manifest $false

    $unstableDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unstableDialogEvidence.native_file_dialogs[0].same_window_stable = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unstableDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a transient common-item HWND presented without the required time-separated stability recheck" "Native file-dialog observation was not stable across two time-separated live samples"
    Write-Manifest $false

    $unboundedDialogWaitEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $unboundedDialogWaitEvidence.native_file_dialogs[0].observation_wait_elapsed_ms = 900001
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($unboundedDialogWaitEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a native-dialog poll presented after exceeding its monotonic fifteen-minute wait" "Native file-dialog observation was not bounded by its monotonic fifteen-minute wait"
    Write-Manifest $false

    $backgroundDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $backgroundDialogEvidence.native_file_dialogs[0].foreground_window = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($backgroundDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a process-owned native file dialog observed behind another foreground window" "Native file-dialog observation was not the active foreground window"
    Write-Manifest $false

    $genericDialogEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $genericDialogEvidence.native_file_dialogs[0].direct_ui_child_observed = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($genericDialogEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a generic #32770 window presented as a native file dialog" "Native file-dialog observation lacks the Windows common item dialog marker"
    Write-Manifest $false

    $forgedDigestEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedDigestEvidence.document_semantics.baseline.canonical_digest = "c" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedDigestEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a forged packaged-app canonical digest" "Packaged-app canonical digest does not match the captured native document."
    Write-Manifest $false

    Copy-Item (Join-Path $tempRoot "baseline.ketchup") (Join-Path $tempRoot "new-document.ketchup") -Force
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a New workflow that retained the physically modeled baseline"
    Write-NativeKetchupFixture (Join-Path $tempRoot "new-document.ketchup") "new document payload`n"
    Write-Manifest $false

    Copy-Item (Join-Path $tempRoot "baseline.ketchup") (Join-Path $tempRoot "new-document.ketchup") -Force
    Add-OptionalSidecarEntryPreservingCanonicalPayload (Join-Path $tempRoot "new-document.ketchup")
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a New document distinguished only by an optional sidecar while retaining the modeled baseline canonical payload"
    Write-NativeKetchupFixture (Join-Path $tempRoot "new-document.ketchup") "new document payload`n"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.document_semantics.baseline.definitions = 1
    $invalidEvidence.document_semantics.baseline.root_occurrences = 1
    $invalidEvidence.document_semantics.baseline.profiles = 1
    $invalidEvidence.document_semantics.baseline.extrusions = 1
    $invalidEvidence.document_semantics.baseline.profile_extrusion_definitions = 1
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a baseline whose packaged-app semantics contain only the initial model"
    Write-Manifest $false

    $hiddenModeledEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $hiddenModeledEvidence.document_semantics.baseline.visible_profile_extrusion_root_occurrences = 1
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($hiddenModeledEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a self-consistent baseline whose additional modeled root occurrence is hidden"
    Write-Manifest $false

    $forgedSemanticEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedSemanticEvidence.document_semantics.baseline.definitions = 3
    $forgedSemanticEvidence.document_semantics.baseline.root_occurrences = 3
    $forgedSemanticEvidence.document_semantics.baseline.profiles = 3
    $forgedSemanticEvidence.document_semantics.baseline.extrusions = 3
    $forgedSemanticEvidence.document_semantics.baseline.profile_extrusion_definitions = 3
    $forgedSemanticEvidence.document_semantics.baseline.visible_profile_extrusion_root_occurrences = 3
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedSemanticEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "plausible forged baseline counts that satisfy every declared semantic threshold" "Captured packaged-app reinspection differs from recorded document semantics: baseline."
    Write-Manifest $false

    Write-Utf8 (Join-Path $tempRoot "ketchup-app.exe") "substituted desktop application bytes`n"
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a substituted executable with self-consistent evidence hashes"
    [IO.File]::WriteAllBytes((Join-Path $tempRoot "ketchup-app.exe"), $inspectorFixtureBytes)
    Write-Manifest $false

    Write-Utf8 (Join-Path $tempRoot "baseline.ketchup") "tampered`n"
    Invoke-VerifyExpectingFailure "a changed canonical artifact"
    Write-NativeKetchupFixture (Join-Path $tempRoot "baseline.ketchup") $canonical
    Write-Manifest $false

    Write-Utf8 (Join-Path $tempRoot "baseline.ketchup") $canonical
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a self-consistent non-native canonical artifact"
    Write-NativeKetchupFixture (Join-Path $tempRoot "baseline.ketchup") $canonical
    Write-Manifest $false

    Change-CanonicalPayloadAndRepairContainerHash (Join-Path $tempRoot "baseline.ketchup")
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a changed canonical payload hidden behind repaired outer hashes"
    Write-NativeKetchupFixture (Join-Path $tempRoot "baseline.ketchup") $canonical
    Write-Manifest $false

    Change-ManifestIdentifierAndRepairContainerHash (Join-Path $tempRoot "baseline.ketchup")
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a self-consistent native document with a substituted graph-schema identifier"
    Write-NativeKetchupFixture (Join-Path $tempRoot "baseline.ketchup") $canonical
    Write-Manifest $false

    Write-NativeKetchupFixture (Join-Path $tempRoot "after-failed-save.ketchup") "silently mutated continuity payload`n"
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a failed-Save continuity artifact that silently diverged from the physically modeled baseline" "Canonical file continuity failed: after-failed-save.ketchup differs from baseline.ketchup."
    Write-NativeKetchupFixture (Join-Path $tempRoot "after-failed-save.ketchup") $canonical
    Write-Manifest $false

    Write-Utf8 (Join-Path $tempRoot "unexpected.txt") "unexpected`n"
    Invoke-VerifyExpectingFailure "an unrecorded artifact"
    Remove-Item (Join-Path $tempRoot "unexpected.txt") -Force

    Write-Manifest $true
    Invoke-VerifyExpectingFailure "a forged release-eligible claim"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.platform_decision = "parallel-desktop-first-release"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a physical-evidence manifest contradicting the accepted Windows-first decision"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.platform_decision_record_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "physical evidence detached from the accepted ADR bytes" "Evidence does not match the accepted Windows-first decision and remaining M19 release blockers."
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.run_id = "unsafe/run"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an unsafe physical run identifier"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.operator.windows_sid = "not-a-windows-sid"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a named physical operator without a valid captured Windows SID" `
        "Windows-session-bound named physical operator attestation"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.desktop_process.session_id = 4
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a packaged desktop process detached from the attesting operator's Windows session" `
        "packaged desktop process"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.machine.model = ""
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an incomplete physical machine identity"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.machine.gpus = @()
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a missing physical GPU inventory"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.captured_utc = "not-a-timestamp"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an invalid evidence capture timestamp"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.steps[0].confirmed_utc = "2026-08-09T00:00:00+02:00"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a non-UTC physical-step timestamp"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.steps[0] | Add-Member -NotePropertyName "automated" -NotePropertyValue $true
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an unknown physical-step field smuggling an automation claim"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.package_snapshot.isolated_copy = $false
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a physical run outside an isolated package snapshot"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.package_snapshot.guarded_files = @($invalidEvidence.package_snapshot.guarded_files | Where-Object { $_ -cne "TKernel.dll" })
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a physical workflow whose package guard omitted one allowlisted runtime" "Write/delete-guarded package files differs from its exact allowlist."
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.package_snapshot.verified_under_guard_utc = $invalidEvidence.package_snapshot.guards_acquired_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a package snapshot not fully verified after all write/delete guards were acquired" `
        "Physical workflow is not enclosed by the write/delete-guarded isolated package"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.package_snapshot.verified_before_launch_utc = $invalidEvidence.steps[0].confirmed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a package snapshot not verified before the physical workflow"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.exact_worker_probe.observed_utc = $invalidEvidence.package_snapshot.runtime_modules_verified_before_workflow_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "an exact-worker probe not completed before the GUI workflow runtime observation"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.desktop_process.observed_exited_utc = $invalidEvidence.package_snapshot.runtime_modules_verified_after_workflow_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "a packaged desktop process not observed alive through the final runtime scan" `
        "Physical workflow is not enclosed by the write/delete-guarded isolated package"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.package_snapshot.verified_after_exit_utc = $invalidEvidence.steps[-1].confirmed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a package snapshot not verified after the physical workflow"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.package_snapshot.guards_released_utc = $invalidEvidence.package_snapshot.verified_after_exit_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure `
        "package write/delete guards released before final snapshot verification completed" `
        "Physical workflow is not enclosed by the write/delete-guarded isolated package"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.package_snapshot.runtime_modules_verified_after_workflow_utc = $invalidEvidence.steps[-1].confirmed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a final runtime-module scan not performed after the complete physical workflow"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.loaded_pinned_occt_modules = @()
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a missing live co-located TKernel observation"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.loaded_pinned_occt_modules[0].observation_phases = @("before-workflow")
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a foundational OCCT runtime not observed after the complete workflow"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.loaded_pinned_occt_modules[0].load_origin = "external-runtime"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a pin-listed OCCT module loaded outside the verified package root"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $tKernelRecord = $invalidEvidence.loaded_pinned_occt_modules[0]
    $shadowModule = [ordered]@{
        name = "TKShadow.dll"
        package_relative_path = "TKShadow.dll"
        load_origin = "verified-package-root"
        size_bytes = [int64]$tKernelRecord.size_bytes
        sha256 = [string]$tKernelRecord.sha256
        observation_phases = @("before-workflow")
    }
    $invalidEvidence.loaded_pinned_occt_modules = @($invalidEvidence.loaded_pinned_occt_modules) + @($shadowModule)
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a live OCCT module absent from the exact pinned package registry"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.loaded_pinned_occt_modules[0].package_relative_path = "runtime/TKernel.dll"
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a pin-listed OCCT module attributed to a nested package path"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $firstStep = $invalidEvidence.steps[0]
    $invalidEvidence.steps[0] = $invalidEvidence.steps[1]
    $invalidEvidence.steps[1] = $firstStep
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "physical workflow steps recorded out of order"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.steps[1].confirmed_utc = $invalidEvidence.steps[0].confirmed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "non-increasing physical workflow timestamps"
    Write-Manifest $false

    $invalidEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $invalidEvidence.captured_utc = $invalidEvidence.steps[0].confirmed_utc
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($invalidEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a physical workflow step timestamp after bundle capture"
    Write-Manifest $false

    $forgedPackage = Get-Content (Join-Path $tempRoot "package-manifest.json") -Raw | ConvertFrom-Json
    $forgedPackage.platform_decision_record = "docs/adr/forged-platform-decision.md"
    Write-Utf8 (Join-Path $tempRoot "package-manifest.json") (($forgedPackage | ConvertTo-Json -Depth 8) + "`n")
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a package manifest with a forged Windows-first decision record"
    Write-PackageManifest
    Write-Manifest $false

    $forgedPackage = Get-Content (Join-Path $tempRoot "package-manifest.json") -Raw | ConvertFrom-Json
    $forgedPackage.platform_decision_record_sha256 = "0" * 64
    Write-Utf8 (Join-Path $tempRoot "package-manifest.json") (($forgedPackage | ConvertTo-Json -Depth 8) + "`n")
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a package decision declaration detached from the accepted ADR bytes"
    Write-PackageManifest
    Write-Manifest $false

    $forgedPackage = Get-Content (Join-Path $tempRoot "package-manifest.json") -Raw | ConvertFrom-Json
    $forgedPackage.occt.manifest_sha256 = (("0" * 64) -join "")
    Write-Utf8 (Join-Path $tempRoot "package-manifest.json") (($forgedPackage | ConvertTo-Json -Depth 8) + "`n")
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "an OCCT provenance declaration detached from the immutable R0 manifest"
    Write-PackageManifest
    Write-Manifest $false

    $forgedPackage = Get-Content (Join-Path $tempRoot "package-manifest.json") -Raw | ConvertFrom-Json
    $forgedPackage.cargo_lock_sha256 = (("0" * 64) -join "")
    Write-Utf8 (Join-Path $tempRoot "package-manifest.json") (($forgedPackage | ConvertTo-Json -Depth 8) + "`n")
    Write-Manifest $false
    Invoke-VerifyExpectingFailure "a package manifest detached from the current Cargo.lock"
    Write-PackageManifest
    Write-Manifest $false

    $forgedEvidence = Get-Content (Join-Path $tempRoot "evidence-manifest.json") -Raw | ConvertFrom-Json
    $forgedEvidence.package_manifest_sha256 = (("0" * 64) -join "")
    Write-Utf8 (Join-Path $tempRoot "evidence-manifest.json") (($forgedEvidence | ConvertTo-Json -Depth 12) + "`n")
    Invoke-VerifyExpectingFailure "a forged package-manifest provenance hash" "Evidence provenance hash does not match the captured package manifest."

    Write-Host "PASS: Windows-first physical release-dialog evidence accepts the Windows-account/SID/session-bound named operator, exact step-specific operator attestations, two-sample-stable PID-bound exact live Ketchup product-surface continuity after every step, workflow-enclosing immutable runner bytes, complete write/delete-guarded package registry, exact byte-bound ADR-backed platform decision, isolated exact-worker PING/PONG probe, exact native-AMD64 Ketchup process and two-sample-stable live main-window identity, five previously absent byte-bound successful Save targets with single-handle in-dialog filesystem identities, creation/write times, sizes, and SHA-256 values plus an exact non-aliased atomic baseline identity chain, immutable identity-bound malformed-Open and exclusively locked-Save inputs, byte-bound existing-document Save rewrite, eight automatically captured within monotonic fifteen-minute waits without console focus theft, two-sample-stable modal main-window-owned process-bound foreground native Windows common-item-dialog observations with time-separated stable HWND destruction, owner reactivation, and still-live exact-owner process continuity, and complete retained-guard verification package plus re-inspected modeled-baseline/New native schema-17 bundle while rejecting pre-existing, aliased, pre-dialog-created, or late-observed Save targets, missing or forged Save rewrites, generic main windows and missing, still-open, hidden-but-live, owner-disabled-after-close, background, generic #32770, foreign-owner, or foreign-process dialogs, forged or misordered worker probes, forged packaged-app canonical digests or plausible semantic counts, an initial-model-only or hidden-modeled baseline, unchanged or sidecar-only New evidence, self-consistent failed-Save continuity divergence, decision/provenance forgery, non-native or nested-payload substitution, unpinned modules, TOCTOU, extras, invalid chronology, oversized or non-UTF-8 release manifests, oversized native documents, and inferred release eligibility."
} finally {
    if (Test-Path $tempRoot) { Remove-Item $tempRoot -Recurse -Force }
    if (Test-Path $verificationPackageDir) { Remove-Item $verificationPackageDir -Recurse -Force }
}
