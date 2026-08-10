[CmdletBinding()]
param(
    [string]$BinaryDir,
    [string]$OutputDir,
    [string]$OcctRoot,
    [string]$OcctManifestPath,
    [switch]$SkipBuild,
    [switch]$VerifyOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($BinaryDir)) { $BinaryDir = Join-Path $repoRoot "target\release" }
if ([string]::IsNullOrWhiteSpace($OutputDir)) { $OutputDir = Join-Path $repoRoot "artifacts\m19\release-candidate\windows-x86_64" }
if ([string]::IsNullOrWhiteSpace($OcctRoot)) { $OcctRoot = Join-Path $repoRoot "third_party\occt-install-r0-v1" }
if ([string]::IsNullOrWhiteSpace($OcctManifestPath)) { $OcctManifestPath = Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json" }
$BinaryDir = [IO.Path]::GetFullPath($BinaryDir)
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$OcctRoot = [IO.Path]::GetFullPath($OcctRoot)
$OcctManifestPath = [IO.Path]::GetFullPath($OcctManifestPath)
$packageManifestPath = Join-Path $OutputDir "package-manifest.json"
$platformDecisionRecordPath = Join-Path $repoRoot "docs\adr\0007-windows-x86-64-first-release.md"
$expectedPlatformDecisionRecordSha256 = "cb91dbd3f8d2b96f7edb5f1f1eae01c49acf2846f85aeed5546c44c79ac5dc62"
$expectedOcctManifestSha256 = "1212a72954ed503a6b06618b2813b1d7c04f5b422329d2327594268e431ef48a"

function Get-Sha256([string]$Path) {
    return (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-Leaf([string]$Path, [string]$Label) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "Missing $Label`: $Path" }
}

function Assert-PeAmd64([string]$Path, [string]$Label) {
    Assert-Leaf $Path $Label
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
        throw "$Label is not a valid PE image."
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($peOffset -lt 64 -or $peOffset + 26 -gt $bytes.Length -or
        $bytes[$peOffset] -ne 0x50 -or $bytes[$peOffset + 1] -ne 0x45 -or
        $bytes[$peOffset + 2] -ne 0 -or $bytes[$peOffset + 3] -ne 0) {
        throw "$Label has an invalid PE signature or header offset."
    }
    $machine = [BitConverter]::ToUInt16($bytes, $peOffset + 4)
    $optionalHeaderSize = [BitConverter]::ToUInt16($bytes, $peOffset + 20)
    $optionalHeaderMagic = [BitConverter]::ToUInt16($bytes, $peOffset + 24)
    if ($machine -ne 0x8664 -or $optionalHeaderSize -lt 2 -or $optionalHeaderMagic -ne 0x20b) {
        throw "$Label is not an AMD64 PE32+ image."
    }
}

function Assert-ExactProperties([object]$Value, [string[]]$Expected, [string]$Label) {
    if ($null -eq $Value) { throw "$Label is missing." }
    $actual = @($Value.PSObject.Properties | ForEach-Object { $_.Name } | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    if (($actual -join "|") -cne ($expectedSorted -join "|")) {
        throw "$Label properties differ from the exact schema. Expected $($expectedSorted -join ', '); got $($actual -join ', ')."
    }
}

function Assert-ExactNames([string[]]$Actual, [string[]]$Expected, [string]$Label) {
    $actualSorted = @($Actual | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    if (($actualSorted -join "|") -cne ($expectedSorted -join "|")) {
        throw "$Label differs from its exact allowlist. Expected $($expectedSorted -join ', '); got $($actualSorted -join ', ')."
    }
}

function Read-OcctManifest {
    Assert-Leaf $OcctManifestPath "pinned OCCT manifest"
    if ((Get-Sha256 $OcctManifestPath) -cne $expectedOcctManifestSha256) {
        throw "The OCCT manifest differs from the immutable R0 preregistration baseline."
    }
    $manifest = Get-Content $OcctManifestPath -Raw | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or
        $manifest.status -ne "built-and-fingerprinted" -or
        $manifest.build.platform -ne "windows-x86_64" -or
        $manifest.build.configuration -ne "Release" -or
        $manifest.source.commit -ne "b8f597c677811d1f9f4d8a97f5ae2825c0353a42" -or
        -not $manifest.source.clean) {
        throw "The OCCT manifest does not describe the pinned Windows Release runtime."
    }
    $records = @($manifest.shared_libraries)
    if ($records.Count -eq 0) { throw "The OCCT manifest contains no runtime DLL records." }
    $names = @{}
    foreach ($record in $records) {
        $relative = [string]$record.path
        if ($relative -notmatch '^win64/vc14/bin/(TK[A-Za-z0-9]+\.dll)$') {
            throw "Unsafe OCCT runtime path in manifest: $relative"
        }
        $name = $Matches[1]
        if ($names.ContainsKey($name)) { throw "Duplicate OCCT runtime name in manifest: $name" }
        $names[$name] = $true
        $source = [IO.Path]::GetFullPath((Join-Path $OcctRoot $relative))
        $rootPrefix = $OcctRoot.TrimEnd("\") + "\"
        if (-not $source.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "OCCT runtime path escapes its pinned root: $relative"
        }
        Assert-Leaf $source "pinned OCCT runtime"
        if ((Get-Item $source).Length -ne [int64]$record.size_bytes -or
            (Get-Sha256 $source) -ne [string]$record.sha256) {
            throw "Pinned OCCT runtime fingerprint mismatch: $name"
        }
    }
    return $manifest
}

function Verify-Package {
    Assert-Leaf $packageManifestPath "package manifest"
    $package = Get-Content $packageManifestPath -Raw | ConvertFrom-Json
    Assert-ExactProperties $package @(
        "schema_version", "kind", "platform", "platform_decision", "platform_decision_record",
        "platform_decision_record_sha256", "release_eligible", "release_blockers", "cargo_lock_sha256", "occt", "files"
    ) "Package manifest"
    Assert-ExactProperties $package.occt @(
        "version", "source_commit", "manifest_sha256", "build_fingerprint", "runtime_dll_count"
    ) "Package OCCT provenance"
    Assert-Leaf $platformDecisionRecordPath "accepted Windows-first platform-decision record"
    if ((Get-Sha256 $platformDecisionRecordPath) -cne $expectedPlatformDecisionRecordSha256) {
        throw "The accepted Windows-first platform-decision record differs from its immutable baseline."
    }
    if ($package.schema_version -ne 1 -or
        $package.kind -ne "technical-release-candidate" -or
        $package.platform -ne "windows-x86_64" -or
        $package.platform_decision -cne "windows-x86_64-first-release" -or
        $package.platform_decision_record -cne "docs/adr/0007-windows-x86-64-first-release.md" -or
        [string]$package.platform_decision_record_sha256 -cne $expectedPlatformDecisionRecordSha256 -or
        $package.release_eligible -ne $false -or
        [string]::Join("|", @($package.release_blockers)) -cne "G19-02-physical-dialog-workflow|G19-03-canonical-tasks|G19-04-current-tree-hardware-certification") {
        throw "Package manifest does not match the accepted Windows-first decision and remaining M19 release blockers."
    }
    $pinnedOcct = Read-OcctManifest
    $lockPath = Join-Path $repoRoot "Cargo.lock"
    Assert-Leaf $lockPath "Cargo lockfile"
    if ($package.cargo_lock_sha256 -ne (Get-Sha256 $lockPath) -or
        $package.occt.version -ne $pinnedOcct.source.release -or
        $package.occt.source_commit -ne $pinnedOcct.source.commit -or
        [string]$package.occt.manifest_sha256 -cne $expectedOcctManifestSha256 -or
        $package.occt.build_fingerprint -ne "occt-8.0.1:b8f597c677811d1f9f4d8a97f5ae2825c0353a42:r0-v1" -or
        $package.occt.runtime_dll_count -ne @($pinnedOcct.shared_libraries).Count) {
        throw "Package provenance does not match the current lockfile and pinned R0 OCCT manifest."
    }
    $expectedRecords = [Collections.Generic.List[object]]::new()
    $expectedRecords.Add([ordered]@{ name = "ketchup-app.exe"; role = "desktop-application" })
    $expectedRecords.Add([ordered]@{ name = "ketchup-exact-worker.exe"; role = "exact-worker" })
    foreach ($record in @($pinnedOcct.shared_libraries)) {
        $expectedRecords.Add([ordered]@{
            name = [IO.Path]::GetFileName([string]$record.path)
            role = "pinned-occt-runtime"
        })
    }
    $files = @($package.files)
    Assert-ExactNames @($files | ForEach-Object { [string]$_.name }) @($expectedRecords | ForEach-Object { [string]$_.name }) "Package entries"
    foreach ($record in $files) {
        Assert-ExactProperties $record @("name", "role", "size_bytes", "sha256") "Package file record"
        $name = [string]$record.name
        $expectedRecord = @($expectedRecords | Where-Object { [string]$_.name -ceq $name })
        if ($name -notmatch '^[A-Za-z0-9._-]+$' -or
            $expectedRecord.Count -ne 1 -or
            [string]$record.role -cne [string]$expectedRecord[0].role -or
            [int64]$record.size_bytes -le 0 -or
            [string]$record.sha256 -cnotmatch '^[0-9a-f]{64}$') {
            throw "Unsafe or invalid package entry: $name"
        }
        $path = Join-Path $OutputDir $name
        Assert-Leaf $path "packaged runtime entry"
        if ((Get-Item $path).Length -ne [int64]$record.size_bytes -or
            (Get-Sha256 $path) -cne [string]$record.sha256) {
            throw "Packaged runtime fingerprint mismatch: $name"
        }
        Assert-PeAmd64 $path "Packaged runtime entry $name"
    }
    $actual = @(Get-ChildItem $OutputDir -Force)
    Assert-ExactNames @($actual | ForEach-Object { $_.Name }) @(@($expectedRecords | ForEach-Object { [string]$_.name }) + "package-manifest.json") "Package contents"
    foreach ($entry in $actual) {
        if ($entry.PSIsContainer) { throw "Unexpected package directory: $($entry.Name)" }
    }
    $worker = Join-Path $OutputDir "ketchup-exact-worker.exe"
    $app = Join-Path $OutputDir "ketchup-app.exe"
    Assert-Leaf $worker "co-located exact worker"
    Assert-Leaf $app "desktop application"
    if (@($files | Where-Object { $_.name -eq "ketchup-app.exe" -and $_.role -eq "desktop-application" }).Count -ne 1 -or
        @($files | Where-Object { $_.name -eq "ketchup-exact-worker.exe" -and $_.role -eq "exact-worker" }).Count -ne 1) {
        throw "Package executable roles are incomplete or ambiguous."
    }
    $dllRecords = @($files | Where-Object { $_.role -eq "pinned-occt-runtime" })
    if ($dllRecords.Count -ne [int]$package.occt.runtime_dll_count -or
        @($files | Where-Object { $_.name -match '\.dll$' -and $_.role -ne "pinned-occt-runtime" }).Count -ne 0) {
        throw "Packaged OCCT DLL set does not match the pinned runtime roles."
    }
    $pinnedDlls = @{}
    foreach ($record in @($pinnedOcct.shared_libraries)) {
        $pinnedDlls[[IO.Path]::GetFileName([string]$record.path)] = $record
    }
    foreach ($record in $dllRecords) {
        $name = [string]$record.name
        if (-not $pinnedDlls.ContainsKey($name)) { throw "Unpinned OCCT runtime in package manifest: $name" }
        $pinned = $pinnedDlls[$name]
        if ([int64]$record.size_bytes -ne [int64]$pinned.size_bytes -or
            [string]$record.sha256 -ne [string]$pinned.sha256) {
            throw "Packaged OCCT provenance differs from the pinned R0 manifest: $name"
        }
    }
    Write-Host "Verified Windows-first technical release candidate: $($files.Count) pinned files; G19-02/G19-03/G19-04 remain."
}

if ($env:OS -ne "Windows_NT") { throw "This candidate packages only the current Windows x86-64 product path." }
if ($VerifyOnly) {
    Verify-Package
    exit 0
}

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        & cargo build --locked --release -p ketchup-app --bin ketchup-app -p ketchup-scheduler --bin ketchup-exact-worker
        if ($LASTEXITCODE -ne 0) { throw "Release build failed." }
    } finally {
        Pop-Location
    }
}

$appSource = Join-Path $BinaryDir "ketchup-app.exe"
$workerSource = Join-Path $BinaryDir "ketchup-exact-worker.exe"
Assert-Leaf $appSource "desktop release binary"
Assert-Leaf $workerSource "exact-worker release binary"
$occtManifest = Read-OcctManifest

if (Test-Path $OutputDir) {
    if (@(Get-ChildItem $OutputDir -Force).Count -ne 0) {
        throw "OutputDir must be absent or empty; refusing to overwrite a candidate: $OutputDir"
    }
} else {
    [void](New-Item $OutputDir -ItemType Directory)
}

$entries = [Collections.Generic.List[object]]::new()
function Add-PackageFile([string]$Source, [string]$Name, [string]$Role) {
    $destination = Join-Path $OutputDir $Name
    Copy-Item $Source $destination
    $file = Get-Item $destination
    $entries.Add([ordered]@{
        name = $Name
        role = $Role
        size_bytes = $file.Length
        sha256 = Get-Sha256 $destination
    })
}

Add-PackageFile $appSource "ketchup-app.exe" "desktop-application"
Add-PackageFile $workerSource "ketchup-exact-worker.exe" "exact-worker"
foreach ($record in @($occtManifest.shared_libraries) | Sort-Object path) {
    $name = [IO.Path]::GetFileName([string]$record.path)
    $source = Join-Path $OcctRoot ([string]$record.path)
    Add-PackageFile $source $name "pinned-occt-runtime"
}

$lockPath = Join-Path $repoRoot "Cargo.lock"
Assert-Leaf $lockPath "Cargo lockfile"
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
        version = [string]$occtManifest.source.release
        source_commit = [string]$occtManifest.source.commit
        manifest_sha256 = $expectedOcctManifestSha256
        build_fingerprint = "occt-8.0.1:b8f597c677811d1f9f4d8a97f5ae2825c0353a42:r0-v1"
        runtime_dll_count = @($occtManifest.shared_libraries).Count
    }
    files = @($entries | Sort-Object name)
}
$json = $package | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText($packageManifestPath, $json + "`n", [Text.UTF8Encoding]::new($false))
Verify-Package
