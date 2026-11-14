#!/bin/sh
# Package a Linux release build into a Debian/Ubuntu .deb and a portable .tar.gz.
#
#   packaging/linux/package.sh   → target/bundle/galeria_<version>_amd64.deb
#                                → target/bundle/Galeria-<version>-linux-x64.tar.gz
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

if [ "${GALERIA_SKIP_BUILD:-}" != 1 ]; then
  cargo build --release --locked
fi
bin="${CARGO_TARGET_DIR:-target}/release/galeria"
[ -x "$bin" ] || { echo "no release binary at $bin" >&2; exit 1; }

out="target/bundle"
mkdir -p "$out"
work=$(mktemp -d "${TMPDIR:-/tmp}/galeria-linux.XXXXXX")
trap 'rm -rf "$work"' EXIT

desktop="$work/galeria.desktop"
cat > "$desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Galeria
Comment=Museum-style painting wallpapers with info plates and slow panning
Exec=galeria
Icon=galeria
Terminal=false
Categories=Utility;Graphics;
StartupWMClass=galeria
DESKTOP

# 1. Portable .tar.gz
tar_dir="$work/Galeria-$version-linux-x64"
mkdir -p "$tar_dir"
install -m 0755 "$bin" "$tar_dir/galeria"
install -m 0644 "$desktop" "$tar_dir/galeria.desktop"
install -m 0644 assets/icons/icon.png "$tar_dir/galeria.png"
install -m 0644 LICENSE "$tar_dir/LICENSE"
tar -C "$work" -czf "$out/Galeria-$version-linux-x64.tar.gz" "Galeria-$version-linux-x64"

# 2. Debian / Ubuntu .deb
if command -v dpkg-deb >/dev/null 2>&1; then
  pkg="$work/deb"
  mkdir -p \
    "$pkg/DEBIAN" \
    "$pkg/usr/bin" \
    "$pkg/usr/share/applications" \
    "$pkg/usr/share/icons/hicolor/32x32/apps" \
    "$pkg/usr/share/icons/hicolor/128x128/apps" \
    "$pkg/usr/share/icons/hicolor/256x256/apps" \
    "$pkg/usr/share/icons/hicolor/512x512/apps"

  install -m 0755 "$bin" "$pkg/usr/bin/galeria"
  install -m 0644 "$desktop" "$pkg/usr/share/applications/galeria.desktop"
  install -m 0644 assets/icons/32x32.png "$pkg/usr/share/icons/hicolor/32x32/apps/galeria.png"
  install -m 0644 assets/icons/128x128.png "$pkg/usr/share/icons/hicolor/128x128/apps/galeria.png"
  install -m 0644 assets/icons/128x128@2x.png "$pkg/usr/share/icons/hicolor/256x256/apps/galeria.png"
  install -m 0644 assets/icons/icon.png "$pkg/usr/share/icons/hicolor/512x512/apps/galeria.png"

  installed_kb=$(du -sk "$pkg/usr" | cut -f1)
  cat > "$pkg/DEBIAN/control" <<CONTROL
Package: galeria
Version: $version
Section: graphics
Priority: optional
Architecture: amd64
Installed-Size: $installed_kb
Depends: libc6, libssl3 | libssl1.1, libxkbcommon0, libvulkan1, libfontconfig1
Maintainer: Lazar Nikolov <hello@nikolovlazar.com>
Homepage: https://github.com/nikolovlazar/galeria
Description: Museum-style painting wallpapers
 Galeria turns high-resolution painting scans from open museum collections
 into a living desktop with info plates, slow panning, and a full-screen viewer.
CONTROL

  dpkg-deb --build --root-owner-group "$pkg" "$out/galeria_${version}_amd64.deb" >/dev/null
  echo "$out/galeria_${version}_amd64.deb ($(du -h "$out/galeria_${version}_amd64.deb" | cut -f1))"
fi

echo "$out/Galeria-$version-linux-x64.tar.gz ($(du -h "$out/Galeria-$version-linux-x64.tar.gz" | cut -f1))"
