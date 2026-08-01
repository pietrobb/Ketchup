[CmdletBinding()]
param(
    [string]$SourceDir,
    [string]$BuildDir,
    [string]$InstallDir
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$thirdPartyRoot = Join-Path $repoRoot "third_party"
$artifactDir = Join-Path $repoRoot "artifacts\r0"
if ([string]::IsNullOrWhiteSpace($SourceDir)) { $SourceDir = Join-Path $thirdPartyRoot "occt-src" }
if ([string]::IsNullOrWhiteSpace($BuildDir)) { $BuildDir = Join-Path $thirdPartyRoot "occt-build-r0-v1" }
if ([string]::IsNullOrWhiteSpace($InstallDir)) { $InstallDir = Join-Path $thirdPartyRoot "occt-install-r0-v1" }

function Assert-ChildPath([string]$Candidate, [string]$Root, [string]$Label) {
    $full = [IO.Path]::GetFullPath($Candidate)
    $rootPrefix = [IO.Path]::GetFullPath($Root).TrimEnd("\") + "\"
    if (-not $full.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must be below $rootPrefix"
    }
    $cursor = $full
    while ($cursor.Length -ge $Root.Length) {
        if ((Test-Path $cursor) -and ((Get-Item $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "$Label contains a reparse point: $cursor"
        }
        if ($cursor -eq [IO.Path]::GetFullPath($Root)) { break }
        $parent = Split-Path $cursor -Parent
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    return $full
}

function Get-TreeFingerprint([string]$Root) {
    $rootPath = (Resolve-Path $Root).Path.TrimEnd("\")
    $reparsePoints = @(Get-ChildItem $rootPath -Force -Recurse | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint })
    if ($reparsePoints.Count -ne 0) { throw "Install tree contains a reparse point: $($reparsePoints[0].FullName)" }
    $files = @(Get-ChildItem $rootPath -File -Force -Recurse | Sort-Object FullName)
    $lines = [Text.StringBuilder]::new()
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($rootPath.Length).TrimStart("\").Replace("\", "/")
        $hash = (Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        [void]$lines.Append($relative).Append("|").Append($file.Length).Append("|").Append($hash).Append("`n")
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash([Text.UTF8Encoding]::new($false).GetBytes($lines.ToString()))
    } finally {
        $sha.Dispose()
    }
    return [ordered]@{
        file_count = $files.Count
        sha256 = [BitConverter]::ToString($digest).Replace("-", "").ToLowerInvariant()
    }
}

$SourceDir = Assert-ChildPath $SourceDir $thirdPartyRoot "SourceDir"
$BuildDir = Assert-ChildPath $BuildDir $thirdPartyRoot "BuildDir"
$InstallDir = Assert-ChildPath $InstallDir $thirdPartyRoot "InstallDir"

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE ".rustup" }
$rustBin = Join-Path $rustupHome "toolchains\1.97.0-x86_64-pc-windows-msvc\bin"
$toolPaths = [ordered]@{
    git = (Get-Command git -CommandType Application).Source
    rustup = Join-Path $cargoHome "bin\rustup.exe"
    rustc = Join-Path $rustBin "rustc.exe"
    cargo = Join-Path $rustBin "cargo.exe"
    rustfmt = Join-Path $rustBin "rustfmt.exe"
    clippy_driver = Join-Path $rustBin "clippy-driver.exe"
    cargo_fmt = Join-Path $rustBin "cargo-fmt.exe"
    cargo_clippy = Join-Path $rustBin "cargo-clippy.exe"
    cargo_deny = Join-Path $cargoHome "bin\cargo-deny.exe"
    cmake = (Get-Command cmake -CommandType Application).Source
    cl = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.35.32215\bin\HostX64\x64\cl.exe"
    link = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.35.32215\bin\HostX64\x64\link.exe"
    rc = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\rc.exe"
    mt = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\mt.exe"
}
$expectedToolHashes = [ordered]@{
    git = "5ecc74f73bcb2ed9ca3c35e7fa287018147fa53c5f8f402517af675a14afbb1a"
    rustup = "86478e53f769379d7f0ebfa7c9aa97cb76ca92233f79aa2cc0dbee2efaac73c7"
    rustc = "6d1c5543ed3a45cfbc1c1332d42d6550d883c14d3c2e323427e631c331cebeeb"
    cargo = "3cd119fe81dfedb9dce4573696bf65058f16b57c9e5babe415b71624315cbb7d"
    rustfmt = "607162816ec4f330fee34d6954e20631a76657c4264d75ae80c14183f8dc1b18"
    clippy_driver = "4ff747c4e2a55d05bfd42b6ea1849c558d4533e44ef44e009706b2be473c5450"
    cargo_fmt = "3eb9b94afb841b7dd95687a35fe815d592bc130961147213d4f2b5dd579597bd"
    cargo_clippy = "d2e4a82ae44b78ab9b218d23ad9a9284f3e71efde5573edaf3076cbbf2b9213e"
    cargo_deny = "0dec180815d2c88b8d695b0ba188fca592cd06826befa37d28bb6c84f7db1849"
    cmake = "56a4d1e9407238ab004abc6a0bb960aa10a8a77b0c52023e10cdf880fe16346f"
    cl = "2ca2e80391d33c76dd2ad17a79b997e5f667aa6d8bedaa8bb28eb0e87a083e3f"
    link = "6fac48476c6009b5f0d66ec957789264aca73d5b8852c3d51c2ef0a93913a467"
    rc = "54bca8318dcb7583b11956671be5695836fcb7984bc8242d4931c947a6959324"
    mt = "8a96db1ff35ddf168dc5308e26882567b9a2a31cae9f26e221d9c1c4ef5fc52a"
}
foreach ($entry in $toolPaths.GetEnumerator()) {
    if (-not (Test-Path $entry.Value)) { throw "Missing frozen tool: $($entry.Value)" }
    $hash = (Get-FileHash $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $expectedToolHashes[$entry.Key]) { throw "Frozen tool hash mismatch before execution: $($entry.Key)" }
}

$oldPath = $env:PATH
$oldRustc = $env:RUSTC
$oldRustfmt = $env:RUSTFMT
Push-Location $repoRoot
try {
    $env:PATH = $rustBin + ";" + (Join-Path $cargoHome "bin") + ";" + $oldPath
    $env:RUSTC = $toolPaths.rustc
    $env:RUSTFMT = $toolPaths.rustfmt
    $rustcVersion = (& $toolPaths.rustc --version).Trim()
    $cargoVersion = (& $toolPaths.cargo --version).Trim()
    $cargoDenyVersion = (& $toolPaths.cargo_deny --version).Trim()
    if ($rustcVersion -notmatch '^rustc 1\.97\.0 ') { throw "Expected Rust 1.97.0, found $rustcVersion." }
    if ($cargoVersion -notmatch '^cargo 1\.97\.0 ') { throw "Expected Cargo 1.97.0, found $cargoVersion." }
    if ($cargoDenyVersion -ne "cargo-deny 0.20.2") { throw "Expected cargo-deny 0.20.2, found $cargoDenyVersion." }

    & $toolPaths.cargo fmt --all --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt failed." }
    & $toolPaths.cargo clippy --locked --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed." }
    & $toolPaths.cargo test --locked --workspace --all-targets
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed." }
    & $toolPaths.cargo_deny check
    if ($LASTEXITCODE -ne 0) { throw "cargo deny failed." }
} finally {
    Pop-Location
    $env:PATH = $oldPath
    $env:RUSTC = $oldRustc
    $env:RUSTFMT = $oldRustfmt
}

$manifestPath = Join-Path $artifactDir "occt-build-manifest.json"
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
if ($manifest.schema_version -ne 1 -or $manifest.source.commit -ne "b8f597c677811d1f9f4d8a97f5ae2825c0353a42") {
    throw "Unexpected OCCT manifest schema or source commit."
}
if (-not $manifest.source.clean -or $manifest.license.source_modifications) {
    throw "The manifest does not describe a clean unmodified OCCT source."
}
if ($manifest.license.expression -ne "LGPL-2.1-only WITH OCCT-exception-1.0") {
    throw "Unexpected OCCT license expression."
}
if ($manifest.toolchain.rustc -ne $rustcVersion -or $manifest.toolchain.cargo -ne $cargoVersion -or $manifest.toolchain.cargo_deny -ne $cargoDenyVersion) {
    throw "Live Rust tools do not match the build manifest."
}

$git = $toolPaths.git
$sourceCommit = (& $git -C $SourceDir rev-parse HEAD).Trim()
$sourceOrigin = (& $git -C $SourceDir remote get-url origin).Trim()
$sourceStatus = @(& $git -C $SourceDir status --porcelain --untracked-files=all)
if ($sourceCommit -ne $manifest.source.commit -or $sourceOrigin -ne "https://github.com/Open-Cascade-SAS/OCCT.git" -or $sourceStatus.Count -ne 0) {
    throw "Live OCCT source does not match the clean recorded source."
}

if ($manifest.build.normalized_config_artifact -ne "occt-cmake-config.json") {
    throw "Unexpected normalized CMake artifact path."
}
$configPath = Join-Path $artifactDir $manifest.build.normalized_config_artifact
$configHash = (Get-FileHash $configPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($configHash -ne $manifest.build.normalized_config_sha256) { throw "Normalized CMake configuration hash mismatch." }
$config = Get-Content $configPath -Raw | ConvertFrom-Json
if ($config.generator -ne "Visual Studio 17 2022" -or
    $config.generator_platform -ne "x64" -or
    $config.generator_toolset -ne "version=14.35.32215" -or
    $config.visual_studio_version -ne "17.5.33530.505" -or
    $config.windows_sdk_version -ne "10.0.22000.0" -or
    $config.library_type -ne "Shared" -or
    $config.cpp_standard -ne "C++17" -or
    -not $config.release_exceptions_enabled) {
    throw "Normalized CMake configuration violates the frozen baseline."
}

$cachePath = Join-Path $BuildDir "CMakeCache.txt"
$cacheHash = (Get-FileHash $cachePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($cacheHash -ne $manifest.build.raw_cmake_cache_sha256) { throw "Live CMake cache hash mismatch." }

$cmake = $toolPaths.cmake
foreach ($entry in $toolPaths.GetEnumerator()) {
    $actualHash = (Get-FileHash $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $manifest.toolchain.tool_sha256.($entry.Key) -or $actualHash -ne $config.tool_sha256.($entry.Key)) {
        throw "Frozen tool hash mismatch against artifacts: $($entry.Key)"
    }
}
if ((& $cmake --version | Select-Object -First 1).Trim() -ne $manifest.toolchain.cmake) {
    throw "Live CMake version does not match the manifest."
}

$installRoot = (Resolve-Path $InstallDir).Path.TrimEnd("\")
$installPrefix = $installRoot + "\"
$actualDlls = @(Get-ChildItem $InstallDir -Filter "*.dll" -File -Recurse | Sort-Object FullName)
$records = @($manifest.shared_libraries)
if ($actualDlls.Count -ne $records.Count) { throw "Installed DLL count does not match the manifest." }
$seenPaths = @{}
foreach ($record in $records) {
    $relativePath = [string]$record.path
    if ($relativePath -notmatch '^win64/vc14/bin/TK[A-Za-z0-9]+\.dll$' -or $seenPaths.ContainsKey($relativePath)) {
        throw "Unsafe, duplicate, or unexpected DLL path in the manifest: $relativePath"
    }
    $seenPaths[$relativePath] = $true
    $path = [IO.Path]::GetFullPath((Join-Path $InstallDir $relativePath))
    if (-not $path.StartsWith($installPrefix, [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path $path)) {
        throw "Recorded DLL escapes or is missing from the install tree: $relativePath"
    }
    $actualHash = (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $record.sha256 -or (Get-Item $path).Length -ne $record.size_bytes) {
        throw "OCCT DLL fingerprint mismatch: $relativePath"
    }
}
foreach ($dll in $actualDlls) {
    $relativePath = $dll.FullName.Substring($installRoot.Length).TrimStart("\").Replace("\", "/")
    if (-not $seenPaths.ContainsKey($relativePath)) { throw "Unrecorded DLL in install tree: $relativePath" }
}
if ($seenPaths.ContainsKey("win64/vc14/bin/TKOpenGl.dll")) { throw "The disabled OCCT OpenGL renderer is present." }
$currentInstallTree = Get-TreeFingerprint $InstallDir
if ($currentInstallTree.file_count -ne $manifest.install_tree.file_count -or $currentInstallTree.sha256 -ne $manifest.install_tree.sha256) {
    throw "The complete OCCT install tree does not match its frozen fingerprint."
}

$smokeSource = Join-Path $repoRoot "tests\native"
$smokeBuild = Join-Path $repoRoot ("target\occt-smoke-" + [Guid]::NewGuid().ToString("N"))
$vsInstance = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
$rc = $toolPaths.rc.Replace("\", "/")
$mt = $toolPaths.mt.Replace("\", "/")
New-Item -ItemType Directory -Path $smokeBuild | Out-Null
try {
    & $cmake -S $smokeSource -B $smokeBuild -G "Visual Studio 17 2022" -A x64 -T "version=14.35.32215" "-DCMAKE_GENERATOR_INSTANCE=$($vsInstance.Replace('\', '/'))" "-DCMAKE_SYSTEM_VERSION=10.0.22000.0" "-DCMAKE_RC_COMPILER=$rc" "-DCMAKE_MT=$mt" "-DOCCT_ROOT=$($InstallDir.Replace('\', '/'))"
    if ($LASTEXITCODE -ne 0) { throw "OCCT native smoke configuration failed." }
    & $cmake --build $smokeBuild --config Release --parallel 4
    if ($LASTEXITCODE -ne 0) { throw "OCCT native smoke build failed." }
    $releaseDir = Join-Path $smokeBuild "Release"
    foreach ($record in $records) {
        $sourceDll = Join-Path $InstallDir ([string]$record.path)
        $stagedDll = Join-Path $releaseDir ([IO.Path]::GetFileName([string]$record.path))
        Copy-Item $sourceDll $stagedDll
        if ((Get-FileHash $stagedDll -Algorithm SHA256).Hash.ToLowerInvariant() -ne $record.sha256) {
            throw "Staged smoke DLL hash mismatch: $($record.path)"
        }
    }
    $smokeExe = Join-Path $releaseDir "occt-smoke.exe"
    $smokeOldPath = $env:PATH
    Push-Location $releaseDir
    try {
        $env:PATH = $releaseDir + ";" + $smokeOldPath
        & $smokeExe
        if ($LASTEXITCODE -ne 0) { throw "OCCT native smoke execution failed." }
    } finally {
        Pop-Location
        $env:PATH = $smokeOldPath
    }
} finally {
    if (Test-Path $smokeBuild) { Remove-Item $smokeBuild -Recurse -Force }
}

Write-Output "Toolchain validation passed for Rust 1.97.0, the native OCCT smoke test, and $($records.Count) exact shared-library fingerprints."
