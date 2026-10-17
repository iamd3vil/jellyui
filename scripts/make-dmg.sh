#!/usr/bin/env bash
# Wraps the macOS release binary in Jellyui.app and packs it into a
# drag-to-Applications disk image. Runs on macOS (sips, iconutil, hdiutil).
#
#   scripts/make-dmg.sh <binary> <version> <output.dmg>
set -euo pipefail

[ $# -eq 3 ] || {
    echo "usage: $0 <binary> <version> <output.dmg>" >&2
    exit 2
}
bin=$1
version=${2#v}
out=$3
root=$(cd "$(dirname "$0")/.." && pwd)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
app=$work/dmg/Jellyui.app
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/jellyui"

iconset=$work/jellyui.iconset
mkdir "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$root/assets/icon/jellyui-1024.png" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$root/assets/icon/jellyui-1024.png" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/jellyui.icns"

for f in LICENSE assets/fonts/OFL.txt target/mpv-static/THIRD-PARTY-LICENSES.txt; do
    if [ -f "$root/$f" ]; then
        cp "$root/$f" "$app/Contents/Resources/"
    fi
done

cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Jellyui</string>
    <key>CFBundleDisplayName</key><string>Jellyui</string>
    <key>CFBundleIdentifier</key><string>in.saratchandra.jellyui</string>
    <key>CFBundleExecutable</key><string>jellyui</string>
    <key>CFBundleIconFile</key><string>jellyui</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

# Ad-hoc signature: Apple Silicon won't run unsigned code, and signing the
# bundle seals Info.plist and the resources. It is not notarized.
codesign --force --sign - "$app"

ln -s /Applications "$work/dmg/Applications"
mkdir -p "$(dirname "$out")"
hdiutil create -volname Jellyui -srcfolder "$work/dmg" -ov -format UDZO "$out" >/dev/null
echo "wrote $out"
