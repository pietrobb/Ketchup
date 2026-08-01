[CmdletBinding()]
param(
    [string]$SourceDir,
    [string]$BuildDir,
    [string]$InstallDir,
    [ValidateRange(1, 64)]
    [int]$ParallelJobs = 12
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$thirdPartyRoot = Join-Path $repoRoot "third_party"
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

$SourceDir = Assert-ChildPath $SourceDir $thirdPartyRoot "SourceDir"
$BuildDir = Assert-ChildPath $BuildDir $thirdPartyRoot "BuildDir"
$InstallDir = Assert-ChildPath $InstallDir $thirdPartyRoot "InstallDir"
if ($BuildDir -eq $InstallDir -or $BuildDir -eq $SourceDir -or $InstallDir -eq $SourceDir) {
    throw "Source, build, and install directories must be distinct."
}

$expectedCommit = "b8f597c677811d1f9f4d8a97f5ae2825c0353a42"
$expectedOrigin = "https://github.com/Open-Cascade-SAS/OCCT.git"
$expectedGitHash = "5ecc74f73bcb2ed9ca3c35e7fa287018147fa53c5f8f402517af675a14afbb1a"
$git = (Get-Command git -CommandType Application).Source
if ((Get-FileHash $git -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedGitHash) {
    throw "The Git binary does not match the frozen R0 fingerprint."
}
$actualCommit = (& $git -C $SourceDir rev-parse HEAD).Trim()
$actualOrigin = (& $git -C $SourceDir remote get-url origin).Trim()
if ($LASTEXITCODE -ne 0 -or $actualCommit -ne $expectedCommit) {
    throw "OCCT source must be V8.0.1 commit $expectedCommit; found '$actualCommit'."
}
if ($actualOrigin -ne $expectedOrigin) { throw "Unexpected OCCT origin: $actualOrigin" }
if (& $git -C $SourceDir status --porcelain --untracked-files=all) {
    throw "OCCT source checkout has local modifications."
}

foreach ($directory in @($BuildDir, $InstallDir)) {
    if (Test-Path $directory) {
        if (Get-ChildItem $directory -Force | Select-Object -First 1) {
            throw "A clean build requires an empty directory: $directory"
        }
    } else {
        New-Item -ItemType Directory -Path $directory | Out-Null
    }
}

$expectedCMakeVersion = "cmake version 4.2.1"
$expectedCMakeHash = "56a4d1e9407238ab004abc6a0bb960aa10a8a77b0c52023e10cdf880fe16346f"
$cmake = (Get-Command cmake -CommandType Application).Source
$cmakeHash = (Get-FileHash $cmake -Algorithm SHA256).Hash.ToLowerInvariant()
if ($cmakeHash -ne $expectedCMakeHash) {
    throw "Expected the frozen CMake binary with SHA-256 $expectedCMakeHash."
}
$cmakeVersion = (& $cmake --version | Select-Object -First 1).Trim()
if ($cmakeVersion -ne $expectedCMakeVersion) {
    throw "Expected the frozen CMake 4.2.1 binary with SHA-256 $expectedCMakeHash."
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$vs = @(& $vswhere -products * -version "[17.5,17.6)" -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json | ConvertFrom-Json) | Select-Object -First 1
if (-not $vs -or $vs.installationVersion -ne "17.5.33530.505") {
    throw "Visual Studio Build Tools 17.5.33530.505 is required for this R0 fingerprint."
}

$msvcVersion = "14.35.32215"
$sdkVersion = "10.0.22000.0"
$cl = Join-Path $vs.installationPath "VC\Tools\MSVC\$msvcVersion\bin\HostX64\x64\cl.exe"
$link = Join-Path $vs.installationPath "VC\Tools\MSVC\$msvcVersion\bin\HostX64\x64\link.exe"
$rc = "C:\Program Files (x86)\Windows Kits\10\bin\$sdkVersion\x64\rc.exe"
$mt = "C:\Program Files (x86)\Windows Kits\10\bin\$sdkVersion\x64\mt.exe"
$expectedToolHashes = [ordered]@{
    $cl = "2ca2e80391d33c76dd2ad17a79b997e5f667aa6d8bedaa8bb28eb0e87a083e3f"
    $link = "6fac48476c6009b5f0d66ec957789264aca73d5b8852c3d51c2ef0a93913a467"
    $rc = "54bca8318dcb7583b11956671be5695836fcb7984bc8242d4931c947a6959324"
    $mt = "8a96db1ff35ddf168dc5308e26882567b9a2a31cae9f26e221d9c1c4ef5fc52a"
}
foreach ($tool in $expectedToolHashes.Keys) {
    if (-not (Test-Path $tool)) { throw "Missing frozen native tool: $tool" }
    $actualHash = (Get-FileHash $tool -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedToolHashes[$tool]) { throw "Native tool hash mismatch: $tool" }
}

$configure = @(
    "-S", $SourceDir,
    "-B", $BuildDir,
    "-G", "Visual Studio 17 2022",
    "-A", "x64",
    "-T", "version=$msvcVersion",
    "-DCMAKE_GENERATOR_INSTANCE=$($vs.installationPath.Replace('\', '/'))",
    "-DCMAKE_SYSTEM_VERSION=$sdkVersion",
    "-DCMAKE_RC_COMPILER=$($rc.Replace('\', '/'))",
    "-DCMAKE_MT=$($mt.Replace('\', '/'))",
    "-DBUILD_LIBRARY_TYPE=Shared",
    "-DBUILD_CPP_STANDARD=C++17",
    "-DBUILD_MODULE_FoundationClasses=ON",
    "-DBUILD_MODULE_ModelingData=ON",
    "-DBUILD_MODULE_ModelingAlgorithms=ON",
    "-DBUILD_MODULE_DataExchange=ON",
    "-DBUILD_MODULE_Visualization=OFF",
    "-DBUILD_MODULE_ApplicationFramework=OFF",
    "-DBUILD_MODULE_Draw=OFF",
    "-DBUILD_USE_VCPKG=OFF",
    "-DBUILD_USE_PCH=OFF",
    "-DBUILD_YACCLEX=OFF",
    "-DBUILD_GTEST=OFF",
    "-DBUILD_DOC_Overview=OFF",
    "-DBUILD_DOC_RefMan=OFF",
    "-DBUILD_RELEASE_DISABLE_EXCEPTIONS=OFF",
    "-DBUILD_WITH_DEBUG=OFF",
    "-DUSE_FREETYPE=OFF",
    "-DUSE_FREEIMAGE=OFF",
    "-DUSE_FFMPEG=OFF",
    "-DUSE_OPENVR=OFF",
    "-DUSE_OPENGL=OFF",
    "-DUSE_GLES2=OFF",
    "-DUSE_D3D=OFF",
    "-DUSE_EIGEN=OFF",
    "-DUSE_RAPIDJSON=OFF",
    "-DUSE_DRACO=OFF",
    "-DUSE_TK=OFF",
    "-DUSE_TBB=OFF",
    "-DUSE_VTK=OFF",
    "-DINSTALL_DIR_LAYOUT=Windows",
    "-DINSTALL_DIR=$($InstallDir.Replace('\', '/'))",
    "-DCMAKE_INSTALL_PREFIX=$($InstallDir.Replace('\', '/'))",
    "-DUSE_GIT_HASH=ON"
)

& $cmake @configure
if ($LASTEXITCODE -ne 0) { throw "OCCT CMake configuration failed." }
& $cmake --build $BuildDir --config Release --target INSTALL --parallel $ParallelJobs
if ($LASTEXITCODE -ne 0) { throw "OCCT Release build or install failed." }
