#!/bin/bash
# Builds "Kečup.app" (Apple Silicon) and a .dmg with it: the desktop app, the
# exact worker beside it and the pinned OCCT libraries in Contents/Frameworks,
# registered for .ketchup documents. Needs OCCT from scripts/macos/build-occt.sh.
#
#   scripts/macos/build-app.sh [--output DIR] [--sign IDENTITY]
#
# --sign defaults to "-" (ad-hoc, for this Mac). Pass a "Developer ID
# Application: …" identity to sign for other Macs; notarization is a separate step.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml")"
output="$repo/artifacts/macos-app/$version"
identity="-"
while [ $# -gt 0 ]; do
    case "$1" in
        --output) output="$2"; shift 2 ;;
        --sign) identity="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

name="Kečup"
app="$output/$name.app"
dmg="$output/Ketchup-$version-macOS.dmg"
occt_lib="$repo/third_party/occt-install-r0-v1/lib"
[ -d "$occt_lib" ] || { echo "missing OCCT: run scripts/macos/build-occt.sh" >&2; exit 1; }

cd "$repo"
cargo build --locked --release -p ketchup-app --bin ketchup-app -p ketchup-scheduler --bin ketchup-exact-worker

rm -rf "$app" "$dmg"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
binaries=("$app/Contents/MacOS/ketchup-app" "$app/Contents/MacOS/ketchup-exact-worker")
cp target/release/ketchup-app target/release/ketchup-exact-worker "$app/Contents/MacOS/"

# The OCCT libraries the binaries load, directly or through each other.
occt_dependencies() { otool -L "$1" | awk -v lib="$occt_lib/" 'index($1, lib) == 1 { print substr($1, length(lib) + 1) }'; }
libraries=()
queue=("${binaries[@]}")
while [ ${#queue[@]} -gt 0 ]; do
    image="${queue[0]}"
    queue=("${queue[@]:1}")
    for library in $(occt_dependencies "$image"); do
        if [ ! -e "$app/Contents/Frameworks/$library" ]; then
            cp -L "$occt_lib/$library" "$app/Contents/Frameworks/$library"
            chmod u+w "$app/Contents/Frameworks/$library"
            libraries+=("$app/Contents/Frameworks/$library")
            queue+=("$app/Contents/Frameworks/$library")
        fi
    done
done

# Point every OCCT reference at the bundle instead of the build machine.
relink() {
    local image="$1" dependency
    for dependency in $(occt_dependencies "$image"); do
        install_name_tool -change "$occt_lib/$dependency" "@rpath/$dependency" "$image" 2>/dev/null
    done
}
for library in "${libraries[@]}"; do
    install_name_tool -id "@rpath/$(basename "$library")" "$library" 2>/dev/null
    relink "$library"
    install_name_tool -add_rpath @loader_path "$library" 2>/dev/null
done
for binary in "${binaries[@]}"; do
    relink "$binary"
    install_name_tool -add_rpath @executable_path/../Frameworks "$binary"
done
# Dependency lines are tab-indented; the unindented headers name the images themselves.
if otool -L "${binaries[@]}" "${libraries[@]}" | grep $'^\t' | grep -F "$repo"; then
    echo "a bundled image still references the build machine" >&2
    exit 1
fi

# The app icon is the bottle from the splash picture, as for the Viewer.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sips -c 560 560 --cropOffset 375 570 pic/splash.png --out "$work/icon.png" >/dev/null
mkdir "$work/icon.iconset"
for size in 16 32 128 256 512; do
    sips -z $size $size "$work/icon.png" --out "$work/icon.iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$work/icon.png" --out "$work/icon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$work/icon.iconset" -o "$app/Contents/Resources/AppIcon.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$name</string>
    <key>CFBundleDisplayName</key><string>$name</string>
    <key>CFBundleIdentifier</key><string>sk.ketchup.app</string>
    <key>CFBundleExecutable</key><string>ketchup-app</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Kečup Document</string>
            <key>CFBundleTypeRole</key><string>Editor</string>
            <key>LSHandlerRank</key><string>Owner</string>
            <key>LSItemContentTypes</key><array><string>sk.ketchup.document</string></array>
        </dict>
    </array>
    <key>UTExportedTypeDeclarations</key>
    <array>
        <dict>
            <key>UTTypeIdentifier</key><string>sk.ketchup.document</string>
            <key>UTTypeDescription</key><string>Kečup Document</string>
            <key>UTTypeConformsTo</key><array><string>public.data</string><string>public.content</string></array>
            <key>UTTypeTagSpecification</key>
            <dict><key>public.filename-extension</key><array><string>ketchup</string></array></dict>
        </dict>
    </array>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

# Inside out: libraries and the worker before the app that seals them.
sign() {
    if [ "$identity" = "-" ]; then
        codesign --force --sign - "$@"
    else
        codesign --force --options runtime --timestamp --sign "$identity" "$@"
    fi
}
sign "${libraries[@]}"
sign "$app/Contents/MacOS/ketchup-exact-worker"
sign "$app"
codesign --verify --strict --deep "$app"

echo "${#libraries[@]} OCCT libraries bundled"
"$app/Contents/MacOS/ketchup-app" --inspect-native-document examples/house-project.ketchup >/dev/null

mkdir "$work/dmg"
cp -R "$app" "$work/dmg/"
ln -s /Applications "$work/dmg/Applications"
hdiutil create -quiet -volname "$name" -srcfolder "$work/dmg" -format UDZO "$dmg"
if [ "$identity" != "-" ]; then
    codesign --force --timestamp --sign "$identity" "$dmg"
fi
echo "Built $app"
echo "Built $dmg"
