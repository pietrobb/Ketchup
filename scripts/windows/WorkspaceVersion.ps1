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

# Android updates an app only to a higher versionCode: a hundred times major * 1000000
# + minor * 1000 + patch, plus a step for what the build is: 99 for a release, 51 + N
# for -rc.N (50 for -rc), 1 + N for -dev.N (0 for -dev). So every later release is
# higher, a pre-release is below its release, and each pre-release of one release has
# its own code in SemVer order: -dev < -dev.1 < -rc < -rc.1 < -rc.2 (review 2026-11-04
# P3 platform; r19 P3: -rc.1, -rc.2 and -dev shared one code). Other pre-releases are
# refused rather than given a code another build has. 0.3.2 shipped as 30029 under the
# rule before, below any code of this one. scripts/macos/bundle-version.sh gives the Mac
# apps the same. Matched with case, as bash does: -RC.1 is refused, not given the code
# of -rc.1 (review 2026-11-06 P3).
function Get-AndroidVersionCode([string]$Version) {
    if ($Version -cnotmatch '^(\d+)\.(\d+)\.(\d+)(?:-(dev|rc)(?:\.(\d+))?)?$') {
        throw "Version $Version is not major.minor.patch with an optional -dev[.N] or -rc[.N]"
    }
    $major = [int]$Matches[1]
    $minor = [int]$Matches[2]
    $patch = [int]$Matches[3]
    $step = switch ($Matches[4]) {
        "dev" { if ($Matches[5]) { 1 + [int]$Matches[5] } else { 0 } }
        "rc" { if ($Matches[5]) { 51 + [int]$Matches[5] } else { 50 } }
        default { 99 }
    }
    if ($minor -gt 999 -or $patch -gt 999 -or $major -gt 20 -or ($Matches[4] -eq "dev" -and $step -gt 49) -or ($Matches[4] -eq "rc" -and $step -gt 98)) {
        throw "Version $Version does not fit an Android versionCode (major up to 20, minor and patch up to 999, -dev.N up to 48, -rc.N up to 47)"
    }
    return ($major * 1000000 + $minor * 1000 + $patch) * 100 + $step
}
