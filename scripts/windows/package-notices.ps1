# Third-party notices every Kecup package carries (LGPL OCCT, MPL acadrust,
# modified Apache starlark, MIT CORE-MATH): the file name in the package and its source.

function Get-PackageNoticeSources([string]$RepoRoot, [string]$OcctRoot) {
    return [ordered]@{
        "THIRD_PARTY_NOTICES.txt" = Join-Path $RepoRoot "THIRD_PARTY_NOTICES.txt"
        "LICENSE-OCCT-LGPL-2.1.txt" = Join-Path $OcctRoot "LICENSE_LGPL_21.txt"
        "LICENSE-OCCT-LGPL-EXCEPTION.txt" = Join-Path $OcctRoot "OCCT_LGPL_EXCEPTION.txt"
        "LICENSE-Starlark-Apache-2.0.txt" = Join-Path $RepoRoot "third_party\starlark\LICENSE"
        "LICENSE-CORE-MATH-MIT.txt" = Join-Path $RepoRoot "third_party\crmath\core-math\LICENSE"
    }
}

function Copy-PackageNotices([string]$RepoRoot, [string]$OcctRoot, [string]$PayloadDir) {
    $sources = Get-PackageNoticeSources $RepoRoot $OcctRoot
    foreach ($name in $sources.Keys) {
        $source = $sources[$name]
        if (-not (Test-Path $source -PathType Leaf) -or (Get-Item $source).Length -eq 0) {
            throw "Missing third-party notice source: $source"
        }
        Copy-Item $source (Join-Path $PayloadDir $name)
    }
}

# $Location is a package folder or a .zip archive; every notice must be there and non-empty.
function Assert-PackageNotices([string]$Location, [string]$Label) {
    $names = @((Get-PackageNoticeSources "." ".").Keys)
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
