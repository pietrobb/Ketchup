# Sourced by the Mac app builds. A bundle's CFBundleShortVersionString and
# CFBundleVersion are period-separated integers, so 0.3.2-dev is neither (review
# 2026-11-04 P3 platform). The short version is the release the build leads to; the
# bundle version is the number Android gives the Viewer (WorkspaceVersion.ps1), so a
# pre-release is below its release and every later release is higher.

# Prints "SHORT BUILD" for a workspace version, or fails for one that is not
# major.minor.patch with an optional pre-release.
bundle_versions() {
    local version="$1" pattern='^([0-9]+)\.([0-9]+)\.([0-9]+)(-[0-9A-Za-z.-]+)?$'
    if [[ ! "$version" =~ $pattern ]]; then
        echo "version $version is not major.minor.patch" >&2
        return 1
    fi
    local major=$((10#${BASH_REMATCH[1]})) minor=$((10#${BASH_REMATCH[2]})) patch=$((10#${BASH_REMATCH[3]}))
    local release=9
    [ -n "${BASH_REMATCH[4]}" ] && release=0
    if [ "$major" -gt 209 ] || [ "$minor" -gt 999 ] || [ "$patch" -gt 999 ]; then
        echo "version $version does not fit a bundle version (major up to 209, minor and patch up to 999)" >&2
        return 1
    fi
    echo "$major.$minor.$patch $(((major * 1000000 + minor * 1000 + patch) * 10 + release))"
}
