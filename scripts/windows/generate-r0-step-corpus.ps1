[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$installDir = Join-Path $repoRoot "third_party\occt-install-r0-v1"
$outputDir = Join-Path $repoRoot "corpora\r0\step"
$buildDir = Join-Path $repoRoot ("target\r0-step-corpus-" + [Guid]::NewGuid().ToString("N"))
$cmake = (Get-Command cmake -CommandType Application).Source
$vsInstance = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
$rc = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\rc.exe"
$mt = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\mt.exe"

if (-not (Test-Path $installDir)) { throw "Missing frozen OCCT install tree: $installDir" }
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
New-Item -ItemType Directory -Path $buildDir | Out-Null

try {
    & $cmake -S (Join-Path $repoRoot "tests\native") -B $buildDir -G "Visual Studio 17 2022" -A x64 -T "version=14.35.32215" "-DCMAKE_GENERATOR_INSTANCE=$($vsInstance.Replace('\', '/'))" "-DCMAKE_SYSTEM_VERSION=10.0.22000.0" "-DCMAKE_RC_COMPILER=$($rc.Replace('\', '/'))" "-DCMAKE_MT=$($mt.Replace('\', '/'))" "-DOCCT_ROOT=$($installDir.Replace('\', '/'))"
    if ($LASTEXITCODE -ne 0) { throw "R0 STEP corpus configuration failed." }

    & $cmake --build $buildDir --config Release --target r0-step-corpus --parallel 4
    if ($LASTEXITCODE -ne 0) { throw "R0 STEP corpus build failed." }

    $releaseDir = Join-Path $buildDir "Release"
    Get-ChildItem (Join-Path $installDir "win64\vc14\bin") -Filter "*.dll" -File | Copy-Item -Destination $releaseDir
    & (Join-Path $releaseDir "r0-step-corpus.exe") $outputDir
    if ($LASTEXITCODE -ne 0) { throw "R0 STEP corpus generation failed." }

    $fixedTimestamp = "2026-08-01T00:00:00"
    $expectedFiles = @(
        "self-authored-box.step",
        "self-authored-through-cut.step",
        "self-authored-l-bracket.step"
    )
    foreach ($name in $expectedFiles) {
        $path = Join-Path $outputDir $name
        if (-not (Test-Path $path)) { throw "Missing generated fixture: $name" }
        $text = [IO.File]::ReadAllText($path)
        $normalized = [Regex]::Replace($text, '\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}', $fixedTimestamp)
        $normalized = $normalized.Replace("`r`n", "`n").Replace("`r", "`n")
        [IO.File]::WriteAllText($path, $normalized, [Text.UTF8Encoding]::new($false))
        $hash = (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()
        Write-Output "$name $hash"
    }
} finally {
    if (Test-Path $buildDir) { Remove-Item $buildDir -Recurse -Force }
}
