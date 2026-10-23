# The release version has one source, [workspace.package] version in Cargo.toml:
# the programs report it (CARGO_PKG_VERSION), and the installer and the Android
# Viewer take it from there instead of stating their own.

function Get-WorkspaceVersion([string]$RepoRoot) {
    $inPackage = $false
    foreach ($line in Get-Content (Join-Path $RepoRoot "Cargo.toml")) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $inPackage = $Matches[1] -eq "workspace.package"
        } elseif ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    throw "Cargo.toml in $RepoRoot has no [workspace.package] version"
}

# Android updates an app only to a higher versionCode: major * 1000000 +
# minor * 1000 + patch of the version, so every later release is higher.
function Get-AndroidVersionCode([string]$Version) {
    if ($Version -notmatch '^(\d+)\.(\d+)\.(\d+)(-[0-9A-Za-z.-]+)?$') {
        throw "Version $Version is not major.minor.patch"
    }
    $major = [int]$Matches[1]
    $minor = [int]$Matches[2]
    $patch = [int]$Matches[3]
    if ($minor -gt 999 -or $patch -gt 999 -or $major -gt 2099) {
        throw "Version $Version does not fit an Android versionCode (minor and patch up to 999)"
    }
    return $major * 1000000 + $minor * 1000 + $patch
}
