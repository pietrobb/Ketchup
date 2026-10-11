#!/bin/bash
# Builds "Kečup Viewer.app" (universal arm64 + x86_64) and a .dmg with it,
# registered for .ketchup-view and .ketchup-comments, and proves the bundled
# Viewer opens the sample package.
#
#   scripts/macos/build-viewer-app.sh [--output DIR] [--sign IDENTITY]
#
# --sign defaults to "-" (ad-hoc, for this Mac). Pass a "Developer ID
# Application: …" identity to sign for other Macs; notarization is a separate step.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml")"
output="$repo/artifacts/macos-viewer/$version"
identity="-"
while [ $# -gt 0 ]; do
    case "$1" in
        --output) output="$2"; shift 2 ;;
        --sign) identity="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

name="Kečup Viewer"
app="$output/$name.app"
dmg="$output/Ketchup-Viewer-$version-macOS.dmg"
targets=(aarch64-apple-darwin x86_64-apple-darwin)

cd "$repo"
rustup target add "${targets[@]}" >/dev/null
binaries=()
for target in "${targets[@]}"; do
    cargo build --locked --release -p ketchup-viewer-macos --target "$target"
    binaries+=("target/$target/release/ketchup-viewer-macos")
done

rm -rf "$app" "$dmg"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create "${binaries[@]}" -output "$app/Contents/MacOS/ketchup-viewer"

# The app icon is the bottle from the splash picture.
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
    <key>CFBundleIdentifier</key><string>sk.ketchup.viewer</string>
    <key>CFBundleExecutable</key><string>ketchup-viewer</string>
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
            <key>CFBundleTypeName</key><string>Kečup Viewer Package</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Owner</string>
            <key>LSItemContentTypes</key><array><string>sk.ketchup.view-package</string></array>
        </dict>
        <dict>
            <key>CFBundleTypeName</key><string>Kečup Viewer Comments</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Owner</string>
            <key>LSItemContentTypes</key><array><string>sk.ketchup.view-comments</string></array>
        </dict>
    </array>
    <key>UTExportedTypeDeclarations</key>
    <array>
        <dict>
            <key>UTTypeIdentifier</key><string>sk.ketchup.view-package</string>
            <key>UTTypeDescription</key><string>Kečup Viewer Package</string>
            <key>UTTypeConformsTo</key><array><string>public.data</string><string>public.content</string></array>
            <key>UTTypeTagSpecification</key>
            <dict><key>public.filename-extension</key><array><string>ketchup-view</string></array></dict>
        </dict>
        <dict>
            <key>UTTypeIdentifier</key><string>sk.ketchup.view-comments</string>
            <key>UTTypeDescription</key><string>Kečup Viewer Comments</string>
            <key>UTTypeConformsTo</key><array><string>public.data</string><string>public.content</string></array>
            <key>UTTypeTagSpecification</key>
            <dict><key>public.filename-extension</key><array><string>ketchup-comments</string></array></dict>
        </dict>
    </array>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

if [ "$identity" = "-" ]; then
    codesign --force --sign - "$app"
else
    codesign --force --options runtime --timestamp --sign "$identity" "$app"
fi
codesign --verify --strict "$app"

lipo -info "$app/Contents/MacOS/ketchup-viewer"
"$app/Contents/MacOS/ketchup-viewer" --verify-open viewer/samples/kitchen.ketchup-view

mkdir "$work/dmg"
cp -R "$app" "$work/dmg/"
ln -s /Applications "$work/dmg/Applications"
hdiutil create -quiet -volname "$name" -srcfolder "$work/dmg" -format UDZO "$dmg"
if [ "$identity" != "-" ]; then
    codesign --force --timestamp --sign "$identity" "$dmg"
fi
echo "Built $app"
echo "Built $dmg"
