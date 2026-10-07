#!/usr/bin/env bash
# Builds a drag-to-Applications disk image from build/MemManager.app.
# Usage: macos/scripts/make_dmg.sh <output.dmg>   (run on macOS after bundle.sh)
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:?usage: make_dmg.sh <output.dmg>}"
app="$here/build/MemManager.app"
[[ -d "$app" ]] || { echo "missing $app — run bundle.sh first" >&2; exit 1; }

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$out"
hdiutil create -volname "MemManager" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$out" >/dev/null
echo "Created $out"
