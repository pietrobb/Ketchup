[CmdletBinding()]
param(
    [string]$BinaryDir
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($BinaryDir)) { $BinaryDir = Join-Path $repoRoot "target\debug" }
$BinaryDir = [IO.Path]::GetFullPath($BinaryDir)
$packager = Join-Path $PSScriptRoot "build-release-candidate.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-m19-package-" + [Guid]::NewGuid().ToString("N"))
$packageDir = Join-Path $tempRoot "windows-x86_64"
$foreignWorkingDir = Join-Path $tempRoot "foreign-working-directory"
$appProcess = $null

function Invoke-VerifyExpectingFailure([string]$Reason) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "SilentlyContinue"
        & powershell -NoProfile -ExecutionPolicy Bypass -File $packager -VerifyOnly -OutputDir $packageDir *> $null
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -eq 0) { throw "Package verification accepted $Reason." }
}

try {
    [void](New-Item $tempRoot -ItemType Directory)
    & $packager -SkipBuild -BinaryDir $BinaryDir -OutputDir $packageDir

    $substitutedOcctManifest = Join-Path $tempRoot "substituted-occt-build-manifest.json"
    $occtManifest = Get-Content (Join-Path $repoRoot "artifacts\r0\occt-build-manifest.json") -Raw | ConvertFrom-Json
    $occtManifest.captured_utc = "2026-08-09T00:00:00Z"
    [IO.File]::WriteAllText($substitutedOcctManifest, (($occtManifest | ConvertTo-Json -Depth 12) + "`n"), [Text.UTF8Encoding]::new($false))
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "SilentlyContinue"
        & powershell -NoProfile -ExecutionPolicy Bypass -File $packager -VerifyOnly -OutputDir $packageDir -OcctManifestPath $substitutedOcctManifest *> $null
        $substitutedManifestExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($substitutedManifestExitCode -eq 0) {
        throw "Package verification accepted a byte-substituted R0 OCCT manifest with otherwise unchanged runtime records."
    }

    $worker = Join-Path $packageDir "ketchup-exact-worker.exe"
    $response = ("PING" | & $worker).Trim()
    if ($LASTEXITCODE -ne 0 -or $response -ne "PONG") {
        throw "The co-located packaged exact worker failed its process-boundary PING."
    }

    [void](New-Item $foreignWorkingDir -ItemType Directory)
    $package = Get-Content (Join-Path $packageDir "package-manifest.json") -Raw | ConvertFrom-Json
    $appProcess = Start-Process `
        -FilePath (Join-Path $packageDir "ketchup-app.exe") `
        -WorkingDirectory $foreignWorkingDir `
        -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 250
        $appProcess.Refresh()
    } while (-not $appProcess.HasExited -and
        $appProcess.MainWindowHandle -eq [IntPtr]::Zero -and
        [DateTime]::UtcNow -lt $deadline)
    if ($appProcess.HasExited) {
        throw "The packaged desktop application exited during foreign-working-directory startup: $($appProcess.ExitCode)"
    }
    if ($appProcess.MainWindowHandle -eq [IntPtr]::Zero) {
        throw "The packaged desktop application did not expose its product window within 30 seconds."
    }
    $loadedPinnedDlls = @{}
    foreach ($module in @($appProcess.Modules)) {
        $moduleName = [string]$module.ModuleName
        if ($moduleName -cnotmatch '^TK[A-Za-z0-9]+\.dll$') { continue }
        $records = @($package.files | Where-Object {
            [string]$_.name -ceq $moduleName -and [string]$_.role -ceq "pinned-occt-runtime"
        })
        if ($records.Count -ne 1) {
            throw "The packaged app loaded OCCT module outside the exact pinned package registry: $moduleName"
        }
        $record = $records[0]
        $expectedPath = [IO.Path]::GetFullPath((Join-Path $packageDir $moduleName))
        $actualPath = [IO.Path]::GetFullPath($module.FileName)
        if (-not $actualPath.Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
            throw "The packaged app resolved pinned OCCT module $moduleName outside its exact package root: $($module.FileName)"
        }
        if ((Get-Item $actualPath).Length -ne [int64]$record.size_bytes -or
            (Get-FileHash $actualPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$record.sha256) {
            throw "The packaged app loaded a changed pinned OCCT module: $moduleName"
        }
        $loadedPinnedDlls[$moduleName] = $true
    }
    if (-not $loadedPinnedDlls.ContainsKey("TKernel.dll")) {
        throw "The packaged app did not load the foundational pinned OCCT runtime from its package."
    }
    Write-Host "Observed $($loadedPinnedDlls.Count) pin-listed OCCT modules loaded from the package by the live GUI process."
    Stop-Process -Id $appProcess.Id -Force
    $appProcess.WaitForExit()
    $appProcess = $null

    $manifestPath = Join-Path $packageDir "package-manifest.json"
    $originalManifest = [IO.File]::ReadAllBytes($manifestPath)
    $unexpected = Join-Path $packageDir "unrecorded.dll"
    [IO.File]::WriteAllBytes($unexpected, [byte[]](1, 2, 3))
    Invoke-VerifyExpectingFailure "an unrecorded runtime DLL"
    Remove-Item $unexpected -Force
    $unexpectedDirectory = Join-Path $packageDir "runtime-shadow"
    [void](New-Item $unexpectedDirectory -ItemType Directory)
    Invoke-VerifyExpectingFailure "an unrecorded package directory"
    Remove-Item $unexpectedDirectory -Force

    $recordedPayload = Join-Path $packageDir "recorded-payload.bin"
    [IO.File]::WriteAllBytes($recordedPayload, [byte[]](4, 5, 6))
    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    $forgedManifest.files = @($forgedManifest.files) + @([ordered]@{
        name = "recorded-payload.bin"
        role = "support-payload"
        size_bytes = (Get-Item $recordedPayload).Length
        sha256 = (Get-FileHash $recordedPayload -Algorithm SHA256).Hash.ToLowerInvariant()
    })
    [IO.File]::WriteAllText($manifestPath, (($forgedManifest | ConvertTo-Json -Depth 8) + "`n"), [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "a self-consistent extra package payload and manifest record"
    Remove-Item $recordedPayload -Force
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    @($forgedManifest.files | Where-Object { [string]$_.name -ceq "ketchup-app.exe" })[0].role = "Desktop-Application"
    [IO.File]::WriteAllText($manifestPath, (($forgedManifest | ConvertTo-Json -Depth 8) + "`n"), [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "a case-variant desktop application role"
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    $originalWorker = [IO.File]::ReadAllBytes($worker)
    $workerPeOffset = [BitConverter]::ToInt32($originalWorker, 0x3c)
    $x86Worker = [byte[]]$originalWorker.Clone()
    [Array]::Copy([BitConverter]::GetBytes([uint16]0x014c), 0, $x86Worker, $workerPeOffset + 4, 2)
    [IO.File]::WriteAllBytes($worker, $x86Worker)
    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    @($forgedManifest.files | Where-Object { [string]$_.name -ceq "ketchup-exact-worker.exe" })[0].sha256 = (Get-FileHash $worker -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($manifestPath, (($forgedManifest | ConvertTo-Json -Depth 8) + "`n"), [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "a self-consistent x86 exact worker in a windows-x86_64 package"
    [IO.File]::WriteAllBytes($worker, $originalWorker)
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    $pe32Worker = [byte[]]$originalWorker.Clone()
    [Array]::Copy([BitConverter]::GetBytes([uint16]0x010b), 0, $pe32Worker, $workerPeOffset + 24, 2)
    [IO.File]::WriteAllBytes($worker, $pe32Worker)
    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    @($forgedManifest.files | Where-Object { [string]$_.name -ceq "ketchup-exact-worker.exe" })[0].sha256 = (Get-FileHash $worker -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($manifestPath, (($forgedManifest | ConvertTo-Json -Depth 8) + "`n"), [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "a self-consistent PE32 exact worker in a windows-x86_64 package"
    [IO.File]::WriteAllBytes($worker, $originalWorker)
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    $forgedManifest.occt.manifest_sha256 = (("0" * 64) -join "")
    [IO.File]::WriteAllText($manifestPath, (($forgedManifest | ConvertTo-Json -Depth 8) + "`n"), [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "an OCCT provenance declaration detached from the immutable R0 manifest"
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    $runtime = Join-Path $packageDir "TKernel.dll"
    $original = [IO.File]::ReadAllBytes($runtime)
    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    $forgedManifest.platform_decision_record = "docs/adr/forged-platform-decision.md"
    $forgedJson = $forgedManifest | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($manifestPath, $forgedJson + "`n", [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "a forged Windows-first platform-decision record"
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    $forgedManifest.platform_decision_record_sha256 = "0" * 64
    [IO.File]::WriteAllText($manifestPath, (($forgedManifest | ConvertTo-Json -Depth 8) + "`n"), [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "a Windows-first decision declaration detached from the accepted ADR bytes"
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    $tampered = [byte[]]$original.Clone()
    $tampered[0] = $tampered[0] -bxor 1
    [IO.File]::WriteAllBytes($runtime, $tampered)
    $forgedManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    $forgedRecord = @($forgedManifest.files | Where-Object { $_.name -eq "TKernel.dll" })
    $forgedRecord[0].sha256 = (Get-FileHash $runtime -Algorithm SHA256).Hash.ToLowerInvariant()
    $forgedJson = $forgedManifest | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($manifestPath, $forgedJson + "`n", [Text.UTF8Encoding]::new($false))
    Invoke-VerifyExpectingFailure "a modified pinned DLL with a matching forged package manifest"
    [IO.File]::WriteAllBytes($runtime, $original)
    [IO.File]::WriteAllBytes($manifestPath, $originalManifest)

    & $packager -VerifyOnly -OutputDir $packageDir
    Write-Host "PASS: Windows-first packaged app/worker/DLL exact registry, AMD64 PE32+ architecture, byte-bound decision record, immutable R0 OCCT-manifest binding, foreign-CWD app launch, co-located OCCT discovery, and fail-closed extra-payload/claim/tamper checks."
} finally {
    if ($null -ne $appProcess -and -not $appProcess.HasExited) {
        Stop-Process -Id $appProcess.Id -Force
        $appProcess.WaitForExit()
    }
    if (Test-Path $tempRoot) { Remove-Item $tempRoot -Recurse -Force }
}
