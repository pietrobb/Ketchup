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

function Write-Utf8Json([string]$Path, [object]$Value) {
    $json = $Value | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
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
New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null

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
}
$expectedRustToolHashes = [ordered]@{
    git = "5ecc74f73bcb2ed9ca3c35e7fa287018147fa53c5f8f402517af675a14afbb1a"
    rustup = "86478e53f769379d7f0ebfa7c9aa97cb76ca92233f79aa2cc0dbee2efaac73c7"
    rustc = "6d1c5543ed3a45cfbc1c1332d42d6550d883c14d3c2e323427e631c331cebeeb"
    cargo = "3cd119fe81dfedb9dce4573696bf65058f16b57c9e5babe415b71624315cbb7d"
    rustfmt = "607162816ec4f330fee34d6954e20631a76657c4264d75ae80c14183f8dc1b18"
    clippy_driver = "4ff747c4e2a55d05bfd42b6ea1849c558d4533e44ef44e009706b2be473c5450"
    cargo_fmt = "3eb9b94afb841b7dd95687a35fe815d592bc130961147213d4f2b5dd579597bd"
    cargo_clippy = "d2e4a82ae44b78ab9b218d23ad9a9284f3e71efde5573edaf3076cbbf2b9213e"
    cargo_deny = "0dec180815d2c88b8d695b0ba188fca592cd06826befa37d28bb6c84f7db1849"
}
foreach ($entry in $toolPaths.GetEnumerator()) {
    if (-not (Test-Path $entry.Value)) { throw "Missing frozen tool: $($entry.Value)" }
    $hash = (Get-FileHash $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $expectedRustToolHashes[$entry.Key]) { throw "Frozen tool hash mismatch: $($entry.Key)" }
}

$expectedCommit = "b8f597c677811d1f9f4d8a97f5ae2825c0353a42"
$expectedOrigin = "https://github.com/Open-Cascade-SAS/OCCT.git"
$git = $toolPaths.git
$sourceCommit = (& $git -C $SourceDir rev-parse HEAD).Trim()
$sourceOrigin = (& $git -C $SourceDir remote get-url origin).Trim()
$sourceStatus = @(& $git -C $SourceDir status --porcelain --untracked-files=all)
$sourceModified = $sourceStatus.Count -ne 0
if ($sourceCommit -ne $expectedCommit -or $sourceOrigin -ne $expectedOrigin -or $sourceModified) {
    throw "OCCT source provenance or cleanliness does not match the frozen baseline."
}

$cachePath = Join-Path $BuildDir "CMakeCache.txt"
if (-not (Test-Path $cachePath)) { throw "Missing OCCT CMake cache at $cachePath." }
$cache = @{}
foreach ($line in Get-Content $cachePath) {
    if ($line -match '^([^#/][^:]*):[^=]*=(.*)$') { $cache[$Matches[1]] = $Matches[2] }
}
function Assert-Cache([string]$Name, [string]$Expected) {
    if (-not $cache.ContainsKey($Name) -or $cache[$Name] -ne $Expected) {
        $actual = if ($cache.ContainsKey($Name)) { $cache[$Name] } else { "<missing>" }
        throw "CMake cache mismatch for ${Name}: expected '$Expected', found '$actual'."
    }
}

$expectedCache = [ordered]@{
    BUILD_CPP_STANDARD = "C++17"
    BUILD_LIBRARY_TYPE = "Shared"
    BUILD_MODULE_ApplicationFramework = "OFF"
    BUILD_MODULE_DataExchange = "ON"
    BUILD_MODULE_Draw = "OFF"
    BUILD_MODULE_FoundationClasses = "ON"
    BUILD_MODULE_ModelingAlgorithms = "ON"
    BUILD_MODULE_ModelingData = "ON"
    BUILD_MODULE_Visualization = "OFF"
    BUILD_RELEASE_DISABLE_EXCEPTIONS = "OFF"
    BUILD_USE_PCH = "OFF"
    BUILD_USE_VCPKG = "OFF"
    BUILD_WITH_DEBUG = "OFF"
    BUILD_YACCLEX = "OFF"
    CMAKE_GENERATOR = "Visual Studio 17 2022"
    CMAKE_GENERATOR_PLATFORM = "x64"
    CMAKE_GENERATOR_TOOLSET = "version=14.35.32215"
    USE_D3D = "OFF"
    USE_DRACO = "OFF"
    USE_FFMPEG = "OFF"
    USE_FREEIMAGE = "OFF"
    USE_FREETYPE = "OFF"
    USE_OPENVR = "OFF"
    USE_RAPIDJSON = "OFF"
    USE_TBB = "OFF"
}
foreach ($entry in $expectedCache.GetEnumerator()) { Assert-Cache $entry.Key $entry.Value }

$systemFile = Get-ChildItem (Join-Path $BuildDir "CMakeFiles") -Filter "CMakeSystem.cmake" -File -Recurse | Select-Object -First 1
$compilerFile = Get-ChildItem (Join-Path $BuildDir "CMakeFiles") -Filter "CMakeCXXCompiler.cmake" -File -Recurse | Select-Object -First 1
if (-not $systemFile -or -not $compilerFile) { throw "Missing generated CMake compiler metadata." }
$systemText = Get-Content $systemFile.FullName -Raw
$compilerText = Get-Content $compilerFile.FullName -Raw
if ($systemText -notmatch 'set\(CMAKE_SYSTEM_VERSION "([^"]+)"\)') { throw "Cannot read selected Windows SDK." }
$sdkVersion = $Matches[1]
if ($sdkVersion -ne "10.0.22000.0") { throw "Unexpected Windows SDK: $sdkVersion" }
if ($compilerText -notmatch 'set\(CMAKE_CXX_COMPILER "([^"]+)"\)') { throw "Cannot read C++ compiler path." }
$cl = $Matches[1].Replace("/", "\")
if ($compilerText -notmatch 'set\(CMAKE_CXX_COMPILER_VERSION "([^"]+)"\)') { throw "Cannot read C++ compiler version." }
$compilerVersion = $Matches[1]
if ($compilerVersion -ne "19.35.32217.1") { throw "Unexpected C++ compiler version: $compilerVersion" }

Assert-Cache "CMAKE_RC_COMPILER" "C:/Program Files (x86)/Windows Kits/10/bin/10.0.22000.0/x64/rc.exe"
Assert-Cache "CMAKE_MT" "C:/Program Files (x86)/Windows Kits/10/bin/10.0.22000.0/x64/mt.exe"
$cmake = $cache["CMAKE_COMMAND"].Replace("/", "\")
$generatorInstance = $cache["CMAKE_GENERATOR_INSTANCE"].Replace("/", "\")
$link = Join-Path (Split-Path $cl) "link.exe"
$rc = $cache["CMAKE_RC_COMPILER"].Replace("/", "\")
$mt = $cache["CMAKE_MT"].Replace("/", "\")
$nativeToolPaths = [ordered]@{
    cmake = $cmake
    cl = $cl
    link = $link
    rc = $rc
    mt = $mt
}
$expectedNativeToolHashes = [ordered]@{
    cmake = "56a4d1e9407238ab004abc6a0bb960aa10a8a77b0c52023e10cdf880fe16346f"
    cl = "2ca2e80391d33c76dd2ad17a79b997e5f667aa6d8bedaa8bb28eb0e87a083e3f"
    link = "6fac48476c6009b5f0d66ec957789264aca73d5b8852c3d51c2ef0a93913a467"
    rc = "54bca8318dcb7583b11956671be5695836fcb7984bc8242d4931c947a6959324"
    mt = "8a96db1ff35ddf168dc5308e26882567b9a2a31cae9f26e221d9c1c4ef5fc52a"
}
foreach ($entry in $nativeToolPaths.GetEnumerator()) {
    if (-not (Test-Path $entry.Value)) { throw "Recorded tool is missing: $($entry.Value)" }
    $hash = (Get-FileHash $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $expectedNativeToolHashes[$entry.Key]) { throw "Frozen tool hash mismatch: $($entry.Key)" }
}

$installRoot = (Resolve-Path $InstallDir).Path.TrimEnd("\")
$dlls = @(Get-ChildItem $InstallDir -Filter "*.dll" -File -Recurse | Sort-Object FullName)
if ($dlls.Count -eq 0) { throw "No installed OCCT shared libraries were found." }
if (@($dlls | Where-Object { $_.Name -notmatch '^TK[A-Za-z0-9]+\.dll$' }).Count -ne 0) {
    throw "Unexpected non-OCCT DLL found in the clean install tree."
}
if (@($dlls | Where-Object { $_.Name -eq "TKOpenGl.dll" }).Count -ne 0) {
    throw "The disabled OCCT OpenGL renderer was built unexpectedly."
}

$libraryRecords = @($dlls | ForEach-Object {
    [ordered]@{
        path = $_.FullName.Substring($installRoot.Length).TrimStart("\").Replace("\", "/")
        size_bytes = $_.Length
        sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
})
$installTree = Get-TreeFingerprint $InstallDir

Push-Location $repoRoot
try {
    $rustcVersion = (& $toolPaths.rustc --version).Trim()
    $cargoVersion = (& $toolPaths.cargo --version).Trim()
    $cargoDenyVersion = (& $toolPaths.cargo_deny --version).Trim()
} finally {
    Pop-Location
}

$normalizedConfig = [ordered]@{
    schema_version = 1
    generator = $cache["CMAKE_GENERATOR"]
    generator_platform = $cache["CMAKE_GENERATOR_PLATFORM"]
    generator_toolset = $cache["CMAKE_GENERATOR_TOOLSET"]
    visual_studio_version = "17.5.33530.505"
    msvc_compiler_version = $compilerVersion
    msvc_tools_version = "14.35.32215"
    windows_sdk_version = $sdkVersion
    configuration = "Release"
    library_type = $cache["BUILD_LIBRARY_TYPE"]
    cpp_standard = $cache["BUILD_CPP_STANDARD"]
    enabled_modules = @("FoundationClasses", "ModelingData", "ModelingAlgorithms", "DataExchange")
    disabled_modules = @("Visualization", "ApplicationFramework", "Draw")
    release_exceptions_enabled = ($cache["BUILD_RELEASE_DISABLE_EXCEPTIONS"] -eq "OFF")
    optional_integrations = @()
    cmake_version = (& $cmake --version | Select-Object -First 1).Trim()
    tool_sha256 = [ordered]@{
        git = (Get-FileHash $toolPaths.git -Algorithm SHA256).Hash.ToLowerInvariant()
        rustup = (Get-FileHash $toolPaths.rustup -Algorithm SHA256).Hash.ToLowerInvariant()
        rustc = (Get-FileHash $toolPaths.rustc -Algorithm SHA256).Hash.ToLowerInvariant()
        cargo = (Get-FileHash $toolPaths.cargo -Algorithm SHA256).Hash.ToLowerInvariant()
        rustfmt = (Get-FileHash $toolPaths.rustfmt -Algorithm SHA256).Hash.ToLowerInvariant()
        clippy_driver = (Get-FileHash $toolPaths.clippy_driver -Algorithm SHA256).Hash.ToLowerInvariant()
        cargo_fmt = (Get-FileHash $toolPaths.cargo_fmt -Algorithm SHA256).Hash.ToLowerInvariant()
        cargo_clippy = (Get-FileHash $toolPaths.cargo_clippy -Algorithm SHA256).Hash.ToLowerInvariant()
        cargo_deny = (Get-FileHash $toolPaths.cargo_deny -Algorithm SHA256).Hash.ToLowerInvariant()
        cmake = (Get-FileHash $nativeToolPaths.cmake -Algorithm SHA256).Hash.ToLowerInvariant()
        cl = (Get-FileHash $nativeToolPaths.cl -Algorithm SHA256).Hash.ToLowerInvariant()
        link = (Get-FileHash $nativeToolPaths.link -Algorithm SHA256).Hash.ToLowerInvariant()
        rc = (Get-FileHash $nativeToolPaths.rc -Algorithm SHA256).Hash.ToLowerInvariant()
        mt = (Get-FileHash $nativeToolPaths.mt -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$configPath = Join-Path $artifactDir "occt-cmake-config.json"
Write-Utf8Json $configPath $normalizedConfig

$manifest = [ordered]@{
    schema_version = 1
    status = "built-and-fingerprinted"
    captured_utc = [DateTime]::UtcNow.ToString("o")
    source = [ordered]@{
        repository = "https://github.com/Open-Cascade-SAS/OCCT"
        release = "8.0.1"
        tag = "V8.0.1"
        commit = $sourceCommit
        clean = $true
    }
    build = [ordered]@{
        platform = "windows-x86_64"
        configuration = "Release"
        library_type = $cache["BUILD_LIBRARY_TYPE"]
        cpp_standard = $cache["BUILD_CPP_STANDARD"]
        modules = $normalizedConfig.enabled_modules
        disabled_modules = $normalizedConfig.disabled_modules
        release_exceptions_enabled = $normalizedConfig.release_exceptions_enabled
        optional_integrations = $normalizedConfig.optional_integrations
        normalized_config_artifact = "occt-cmake-config.json"
        normalized_config_sha256 = (Get-FileHash $configPath -Algorithm SHA256).Hash.ToLowerInvariant()
        raw_cmake_cache_sha256 = (Get-FileHash $cachePath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    install_tree = $installTree
    toolchain = [ordered]@{
        rustc = $rustcVersion
        cargo = $cargoVersion
        cargo_deny = $cargoDenyVersion
        cmake = $normalizedConfig.cmake_version
        visual_studio = "17.5.33530.505"
        msvc_tools = "14.35.32215"
        cl_file_version = (Get-Item $cl).VersionInfo.FileVersion
        windows_sdk = $sdkVersion
        tool_sha256 = $normalizedConfig.tool_sha256
    }
    shared_libraries = $libraryRecords
    license = [ordered]@{
        expression = "LGPL-2.1-only WITH OCCT-exception-1.0"
        distribution_model = "replaceable shared libraries"
        source_modifications = $sourceModified
    }
}

$outputPath = Join-Path $artifactDir "occt-build-manifest.json"
Write-Utf8Json $outputPath $manifest
Write-Output "Wrote $outputPath with $($libraryRecords.Count) OCCT DLL fingerprints."
