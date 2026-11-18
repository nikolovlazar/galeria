#!/bin/sh
# Create the update archive from an already signed (and, if applicable,
# notarized and stapled) app. The bundle itself is kept as the ZIP root.
# Usage: packaging/update/make_zip.sh [Galeria.app] [output.zip]
set -eu

app=${1:-target/bundle/Galeria.app}
zip=${2:-target/bundle/Galeria-macos-arm64.zip}
[ -d "$app" ] || { echo "no app bundle at $app" >&2; exit 1; }
mkdir -p "$(dirname "$zip")"
rm -f "$zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
echo "$zip"
