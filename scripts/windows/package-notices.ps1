# Third-party notices each Kecup package carries: THIRD_PARTY_NOTICES.txt, made by
# scripts/third_party_notices.py for the programs of that package (review 2026-11-01
# REL-100), and the license texts of the components it names (LGPL OCCT, modified
# Apache starlark, MIT CORE-MATH) beside it. The Viewer contains none of them.

function Get-PackageNoticeSources([string]$RepoRoot, [string]$OcctRoot, [string]$Package) {
    switch ($Package) {
        "ketchup" {
            return [ordered]@{
                "LICENSE-OCCT-LGPL-2.1.txt" = Join-Path $OcctRoot "LICENSE_LGPL_21.txt"
                "LICENSE-OCCT-LGPL-EXCEPTION.txt" = Join-Path $OcctRoot "OCCT_LGPL_EXCEPTION.txt"
                "LICENSE-Starlark-Apache-2.0.txt" = Join-Path $RepoRoot "third_party\starlark\LICENSE"
                "LICENSE-CORE-MATH-MIT.txt" = Join-Path $RepoRoot "third_party\crmath\core-math\LICENSE"
            }
        }
        "viewer" { return [ordered]@{} }
        default { throw "Unknown package $Package." }
    }
}

function Get-PackageNoticeNames([string]$Package) {
    return @("THIRD_PARTY_NOTICES.txt") + @((Get-PackageNoticeSources "." "." $Package).Keys)
}

function Copy-PackageNotices([string]$RepoRoot, [string]$OcctRoot, [string]$PayloadDir, [string]$Package) {
    $sources = Get-PackageNoticeSources $RepoRoot $OcctRoot $Package
    foreach ($name in $sources.Keys) {
        $source = $sources[$name]
        if (-not (Test-Path $source -PathType Leaf) -or (Get-Item $source).Length -eq 0) {
            throw "Missing third-party notice source: $source"
        }
    }
    & python (Join-Path $RepoRoot "scripts\third_party_notices.py") --package $Package --output $PayloadDir
    if ($LASTEXITCODE -ne 0) { throw "The third-party notices of $Package could not be made." }
    foreach ($name in $sources.Keys) {
        Copy-Item $sources[$name] (Join-Path $PayloadDir $name)
    }
}

# $Location is a package folder or a .zip archive; every notice must be there and non-empty.
function Assert-PackageNotices([string]$Location, [string]$Label, [string]$Package) {
    $names = Get-PackageNoticeNames $Package
    if ($Location -like "*.zip") {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [IO.Compression.ZipFile]::OpenRead($Location)
        try {
            foreach ($name in $names) {
                $entry = $archive.GetEntry($name)
                if ($null -eq $entry -or $entry.Length -eq 0) {
                    throw "$Label lacks the third-party notice $name."
                }
            }
        } finally {
            $archive.Dispose()
        }
    } else {
        foreach ($name in $names) {
            $path = Join-Path $Location $name
            if (-not (Test-Path $path -PathType Leaf) -or (Get-Item $path).Length -eq 0) {
                throw "$Label lacks the third-party notice $name."
            }
        }
    }
}
