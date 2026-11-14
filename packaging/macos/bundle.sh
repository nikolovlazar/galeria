#!/bin/sh
# Build Galeria.app from a release build.
#
#   packaging/macos/bundle.sh            → target/bundle/Galeria.app
#   GALERIA_SIGN="Developer ID Application: …" packaging/macos/bundle.sh
#
# It only assembles and signs the bundle. It does not install or start it:
# copy it to /Applications yourself when you want to use it.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

if [ "${GALERIA_SKIP_BUILD:-}" != 1 ]; then
  cargo build --release
fi
bin="${CARGO_TARGET_DIR:-target}/release/galeria"
[ -x "$bin" ] || { echo "no release binary at $bin" >&2; exit 1; }

app="target/bundle/Galeria.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/galeria"
cp assets/icons/icon.icns "$app/Contents/Resources/icon.icns"

plist="$app/Contents/Info.plist"
cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Galeria</string>
  <key>CFBundleDisplayName</key><string>Galeria</string>
  <key>CFBundleIdentifier</key><string>com.nikolovlazar.galeria</string>
  <key>CFBundleExecutable</key><string>galeria</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.lifestyle</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>© Lazar Nikolov</string>
</dict>
</plist>
PLIST
# Whatever else the app declares (menu bar app, …) lives in packaging/macos/Info.plist.
/usr/libexec/PlistBuddy -c "Merge packaging/macos/Info.plist" "$plist" >/dev/null
plutil -lint "$plist" >/dev/null

# Ad hoc unless an identity is given. macOS treats each ad hoc build as a new app,
# so it asks again for anything it had granted the previous one.
codesign --force --sign "${GALERIA_SIGN:--}" "$app"
codesign --verify --strict "$app"

echo "$app ($version, $(du -sh "$app" | cut -f1))"
