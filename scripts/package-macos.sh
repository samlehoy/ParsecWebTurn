#!/bin/sh
# Builds ParsecWebTurn.app for the host Mac and zips it into dist/.
# Ad-hoc signed only: other Macs need "Open Anyway" in Privacy & Security
# until a Developer ID signature and notarization are added.
set -eu
cd "$(dirname "$0")/.."

version=$(tr -d '[:space:]' < VERSION)
manifest_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' src-native/Cargo.toml | head -n 1)
[ "$manifest_version" = "$version" ] || { echo "Native version mismatch" >&2; exit 1; }

wasm=src-native/vendor/parsecd.wasm
if [ ! -f "$wasm" ]; then
    curl -fsSL -o "$wasm.download" https://web.parsec.app/parsecd
    echo "d663dd96df477c65479fb93eb88756c7fcafff581cc93be563625cd195a4b4a6  $wasm.download" | shasum -a 256 -c - >/dev/null \
        || { rm -f "$wasm.download"; echo "The public Parsec WASM changed. Audit the new ABI before updating the source pin." >&2; exit 1; }
    mv "$wasm.download" "$wasm"
fi

cargo build --locked --release --manifest-path src-native/Cargo.toml
binary=src-native/target/release/parsec-web-turn
arch=$(lipo -archs "$binary")
for marker in video-hardware-probe login-audit synthetic-session-value private-password-token-url PARSEC_BUD_DIAGNOSTICS; do
    if grep -qa "$marker" "$binary"; then echo "Diagnostic marker in normal executable: $marker" >&2; exit 1; fi
done

staging=$(mktemp -d "${TMPDIR:-/tmp}/parsecwebturn-package.XXXXXX")
trap 'rm -rf "$staging"' EXIT
package_name="ParsecWebTurn-v$version-macos-$arch"
package="$staging/$package_name"
app="$package/ParsecWebTurn.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/ParsecWebTurn"

iconset="$staging/AppIcon.iconset"
mkdir "$iconset"
for size in 16 32 128 256; do
    sips -s format png -z "$size" "$size" assets/parsec.ico --out "$iconset/icon_${size}x${size}.png" >/dev/null
done
for size in 16 32 128; do
    cp "$iconset/icon_$((size * 2))x$((size * 2)).png" "$iconset/icon_${size}x${size}@2x.png" 2>/dev/null || true
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleExecutable</key><string>ParsecWebTurn</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIdentifier</key><string>io.github.parsecwebturn</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>ParsecWebTurn</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict "$app"

cp README.md LICENSE CHANGELOG.md "$package/"
cp src-native/vendor/webrtc-0.14.0/LICENSE-MIT "$package/WEBRTC-LICENSE-MIT.txt"
cp src-native/vendor/webrtc-0.14.0/LICENSE-APACHE "$package/WEBRTC-LICENSE-APACHE.txt"
cp src-native/vendor/webrtc-0.14.0/LOCAL-CHANGES.md "$package/WEBRTC-LOCAL-CHANGES.md"
cp src-native/vendor/webrtc-ice-0.14.0/LOCAL-CHANGES.md "$package/ICE-LOCAL-CHANGES.md"
cp src-native/vendor/turn-0.11.0/LOCAL-CHANGES.md "$package/TURN-LOCAL-CHANGES.md"
cp src-native/crates/bud/vendor/lowlat-client/LICENSE "$package/LOWLAT-LICENSE.txt"
cp src-native/crates/bud/vendor/lowlat-client/PARSECWEBTURN.md "$package/BUD-LOCAL-CHANGES.md"
cp src-native/crates/bud/vendor/lowlat-net/PARSECWEBTURN.md "$package/BUD-NET-LOCAL-CHANGES.md"
opus=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/libopus_sys-0.3.3 | head -n 1)
[ -d "$opus" ] || { echo "Bundled Opus license directory missing" >&2; exit 1; }
cp "$opus/LICENSE" "$package/LIBOPUS-SYS-LICENSE.txt"
cp "$opus/opus/COPYING" "$package/OPUS-LICENSE.txt"

mkdir -p dist
zip="dist/$package_name.zip"
rm -f "$zip"
ditto -c -k --keepParent "$package" "$zip"
shasum -a 256 "$zip" | sed 's| dist/| |' > "dist/SHA256SUMS-macos.txt"
echo "$zip"
