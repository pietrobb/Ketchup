[CmdletBinding()]
param(
    # The release folder; SHA256SUMS.txt is written into it.
    [Parameter(Mandatory = $true)][string]$OutputDir,
    # The files of the release, each directly in OutputDir.
    [Parameter(Mandatory = $true)][string[]]$Assets
)

# Writes SHA256SUMS.txt for the files a user downloads from the release, by the
# names they download them under, so `sha256sum -c SHA256SUMS.txt` in the folder
# of the downloads checks each one. A file not directly in the release folder is
# no file of the release and is refused: the 0.3.2 sums named
# payload/package-manifest.json, which no one could download (review 2026-11-08
# REL-251).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$folder = [IO.Path]::GetFullPath($OutputDir).TrimEnd('\', '/')
$lines = foreach ($asset in $Assets) {
    $path = [IO.Path]::GetFullPath($asset)
    if ([IO.Path]::GetDirectoryName($path).TrimEnd('\', '/') -ne $folder) {
        throw "$path is not a file of the release in $folder, so SHA256SUMS.txt cannot name it."
    }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "The release file $path does not exist."
    }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($path))"
}
[IO.File]::WriteAllText(
    (Join-Path $folder "SHA256SUMS.txt"),
    (($lines -join "`r`n") + "`r`n"),
    [Text.UTF8Encoding]::new($false))
