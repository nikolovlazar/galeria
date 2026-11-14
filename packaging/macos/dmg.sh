#!/bin/sh
# Build the disk image people download: a gallery wall with Galeria.app in one
# gilt frame and the Applications folder in the other, to drag across.
#
#   packaging/macos/dmg.sh            → target/bundle/Galeria-<version>.dmg
#   GALERIA_SIGN="Developer ID Application: …" packaging/macos/dmg.sh
#
# Builds the app first (see bundle.sh). The window's background is drawn by
# Galeria itself (src/ui/installer.rs) and laid out with dmgbuild, which is
# installed into target/dmg-venv the first time (needs python3 and the network).
# Without a Developer ID the app is signed ad hoc and not notarized, so macOS
# asks people to approve it on first launch.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

packaging/macos/bundle.sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
dmg="target/bundle/Galeria-$version.dmg"
work=$(mktemp -d "${TMPDIR:-/tmp}/galeria-dmg.XXXXXX")
trap 'rm -rf "$work"' EXIT

# The background, at 1x and 2x in one file, so it is sharp on any display.
bin="${CARGO_TARGET_DIR:-target}/release/galeria"
for scale in 1 2; do
  GALERIA_HOME="$work/home" "$bin" ui --window installer --scale "$scale" \
    --snap "$work/background-$scale.png" >/dev/null 2>&1
  [ -s "$work/background-$scale.png" ] || { echo "could not draw the background" >&2; exit 1; }
done
tiffutil -cathidpicheck "$work/background-1.png" "$work/background-2.png" \
  -out "$work/background.tiff" >/dev/null 2>&1

venv="target/dmg-venv"
if [ ! -x "$venv/bin/dmgbuild" ]; then
  python3 -m venv "$venv"
  "$venv/bin/pip" -q install --disable-pip-version-check dmgbuild
fi

rm -f "$dmg"
"$venv/bin/dmgbuild" -s packaging/macos/dmg_settings.py \
  -D app="$root/target/bundle/Galeria.app" \
  -D background="$work/background.tiff" \
  "Galeria" "$dmg" >/dev/null
hdiutil verify -quiet "$dmg"

if [ -n "${GALERIA_SIGN:-}" ]; then
  codesign --force --sign "$GALERIA_SIGN" "$dmg"
fi

echo "$dmg ($(du -h "$dmg" | cut -f1))"
