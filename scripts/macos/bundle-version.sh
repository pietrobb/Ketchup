# Sourced by the Mac app builds. A bundle's CFBundleShortVersionString and
# CFBundleVersion are period-separated integers, so 0.3.2-dev is neither (review
# 2026-11-04 P3 platform). The short version is the release the build leads to; the
# bundle version is the number Android gives the Viewer (WorkspaceVersion.ps1), so a
# pre-release is below its release and every later release is higher.

# Prints "SHORT BUILD" for a workspace version, or fails for one that is not
# major.minor.patch with an optional -dev[.N] or -rc[.N] (review 2026-11-04 r19 P3:
# -rc.1, -rc.2 and -dev shared one bundle version).
bundle_versions() {
    local version="$1" pattern='^([0-9]+)\.([0-9]+)\.([0-9]+)(-(dev|rc)(\.([0-9]+))?)?$'
    if [[ ! "$version" =~ $pattern ]]; then
        echo "version $version is not major.minor.patch with an optional -dev[.N] or -rc[.N]" >&2
        return 1
    fi
    local major=$((10#${BASH_REMATCH[1]})) minor=$((10#${BASH_REMATCH[2]})) patch=$((10#${BASH_REMATCH[3]}))
    local kind="${BASH_REMATCH[5]}" number="${BASH_REMATCH[7]}" step=99 most=99
    if [ "$kind" = dev ]; then
        step=0 most=49
        [ -n "$number" ] && step=$((1 + 10#$number))
    elif [ "$kind" = rc ]; then
        step=50 most=98
        [ -n "$number" ] && step=$((51 + 10#$number))
    fi
    if [ "$major" -gt 20 ] || [ "$minor" -gt 999 ] || [ "$patch" -gt 999 ] || [ "$step" -gt "$most" ]; then
        echo "version $version does not fit a bundle version (major up to 20, minor and patch up to 999, -dev.N up to 48, -rc.N up to 47)" >&2
        return 1
    fi
    echo "$major.$minor.$patch $(((major * 1000000 + minor * 1000 + patch) * 100 + step))"
}
