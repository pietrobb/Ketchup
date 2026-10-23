[CmdletBinding()]
param(
    # Folder for Ketchup-<version>-Examples.zip; it must not hold that zip yet.
    [Parameter(Mandatory = $true)][string]$OutputDir,
    # Folder with the examples the README links to a release download (too large
    # for the repository), by file name, e.g. tiny-house.ketchup.
    [Parameter(Mandatory = $true)][string]$DownloadedExamples,
    # Defaults to the version in Cargo.toml; any other is refused.
    [string]$Version
)

# Builds Ketchup-<version>-Examples.zip, the examples README.md promises on the
# release page: every model of its "Examples you can open right away" table and
# every rule program in examples/programs, under one Ketchup-<version>-Examples
# folder. The README table is the list; a file it links that is missing fails.
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
. (Join-Path $PSScriptRoot "WorkspaceVersion.ps1")
$workspaceVersion = Get-WorkspaceVersion $root
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = $workspaceVersion
} elseif ($Version -cne $workspaceVersion) {
    throw "-Version $Version is not $workspaceVersion, the version in Cargo.toml; change [workspace.package] version there instead."
}

$readme = Get-Content (Join-Path $root "README.md") -Encoding UTF8
$start = [Array]::IndexOf($readme, "## Examples you can open right away")
if ($start -lt 0) { throw "README.md has no 'Examples you can open right away' section." }
$documents = @()
for ($i = $start + 1; $i -lt $readme.Count -and -not $readme[$i].StartsWith("## "); $i++) {
    if ($readme[$i] -match '^\|\s*\[`([^`]+)`\]\(([^)]+)\)\s*\|') {
        $documents += [pscustomobject]@{ name = $Matches[1]; target = $Matches[2] }
    }
}
if ($documents.Count -eq 0) { throw "README.md lists no examples in its table." }

$name = "Ketchup-$Version-Examples"
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$zipPath = Join-Path $OutputDir "$name.zip"
if (Test-Path $zipPath) { throw "Refusing to overwrite $zipPath" }
$stage = Join-Path ([IO.Path]::GetTempPath()) ("ketchup-examples-" + [Guid]::NewGuid().ToString("N"))
$folder = Join-Path $stage $name
try {
    [void](New-Item (Join-Path $folder "programs") -ItemType Directory -Force)
    foreach ($document in $documents) {
        if ($document.target -match '^https://[^ ]+/releases/download/[^/]+/([^/]+)$') {
            $source = Join-Path $DownloadedExamples $Matches[1]
            if (-not (Test-Path $source -PathType Leaf)) {
                throw "$($Matches[1]) is not in $DownloadedExamples; download it from $($document.target)"
            }
        } elseif ($document.target -match '^[A-Za-z0-9_./-]+$') {
            $source = Join-Path $root $document.target
            if (-not (Test-Path $source -PathType Leaf)) { throw "README.md links $($document.target), which does not exist." }
        } else {
            throw "README.md links $($document.name) to $($document.target), neither a repository file nor a release download."
        }
        if ([IO.Path]::GetFileName($source) -cne $document.name) {
            throw "README.md names $($document.name) but links $($document.target)."
        }
        Copy-Item $source (Join-Path $folder $document.name)
    }
    Copy-Item (Join-Path $root "examples\programs\*.star") (Join-Path $folder "programs")
    [void](New-Item $OutputDir -ItemType Directory -Force)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($folder, $zipPath, [IO.Compression.CompressionLevel]::Optimal, $true)
} finally {
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
}
Write-Host "Examples: $zipPath"
