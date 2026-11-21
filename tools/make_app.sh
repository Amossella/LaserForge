#!/bin/bash
# Build LaserForge.app (release build, icon, Info.plist) and a zip for GitHub releases.
# Usage: tools/make_app.sh            → release/LaserForge.app and release/LaserForge-<version>-macos-<arch>.zip
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
OUT="$ROOT/release"
APP="$OUT/LaserForge.app"

cargo build --release -p laserforge-app

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/LaserForge "$APP/Contents/MacOS/LaserForge"

# icon: assets/icon.png → .icns
ICONSET="$(mktemp -d)/LaserForge.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/LaserForge.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>LaserForge</string>
    <key>CFBundleDisplayName</key><string>LaserForge</string>
    <key>CFBundleIdentifier</key><string>local.laserforge.app</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleExecutable</key><string>LaserForge</string>
    <key>CFBundleIconFile</key><string>LaserForge</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSCameraUsageDescription</key><string>LaserForge usa la camera per mostrare il materiale sull'area di lavoro.</string>
    <key>NSLocalNetworkUsageDescription</key><string>LaserForge cerca i laser in rete e offre il controllo dal telefono.</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>CFBundleDocumentTypes</key>
    <array><dict>
        <key>CFBundleTypeName</key><string>LaserForge Project</string>
        <key>CFBundleTypeExtensions</key><array><string>lfproj</string></array>
        <key>CFBundleTypeRole</key><string>Editor</string>
    </dict></array>
</dict>
</plist>
EOF

# ad-hoc signature, so macOS keeps camera / network permissions between launches
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

ZIP="$OUT/LaserForge-${VERSION}-macos-${ARCH}.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "$ZIP"
