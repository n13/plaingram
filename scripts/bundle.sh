#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/release/plaingram.app"
ICONSET="$ROOT/target/AppIcon.iconset"
VERSION="$(cargo metadata --manifest-path "$ROOT/Cargo.toml" --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "tg-ui") | .version')"

cargo build --manifest-path "$ROOT/Cargo.toml" --release -p tg-ui
rm -rf "$APP" "$ICONSET"
cargo run --manifest-path "$ROOT/Cargo.toml" -q -p tg-ui --example icon -- "$ICONSET"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
cp "$ROOT/target/release/plaingram" "$APP/Contents/MacOS/plaingram"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>plaingram</string>
  <key>CFBundleIdentifier</key><string>local.plaingram</string>
  <key>CFBundleExecutable</key><string>plaingram</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
IDENTITY="$(security find-identity -v -p codesigning | awk '/"Apple Development: / {print $2}' | sort | head -1)"
if [ -z "$IDENTITY" ]; then
  echo "warning: no Apple Development certificate; signing ad-hoc, so macOS asks for Keychain access again after every rebuild" >&2
  IDENTITY=-
fi
codesign --force --sign "$IDENTITY" "$APP"
echo "$APP"
