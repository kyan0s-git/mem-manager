#!/usr/bin/env bash
# Assembles MemManager.app from the SwiftPM build (docs/architecture/macos.md §2).
# Usage: macos/scripts/bundle.sh [debug|release]   (run on macOS)
# Signing/notarization are added in the release workflow; this script only lays out the bundle.
set -euo pipefail

config="${1:-release}"
here="$(cd "$(dirname "$0")/.." && pwd)"
cd "$here"

swift build -c "$config" --arch arm64 --arch x86_64
bin="$(swift build -c "$config" --arch arm64 --arch x86_64 --show-bin-path)"

app="$here/build/MemManager.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Library/LaunchDaemons"
cp "$bin/MemManager" "$bin/MemManagerHelper" "$app/Contents/MacOS/"
cp Resources/Info.plist "$app/Contents/Info.plist"
cp Resources/dev.memmanager.helper.plist "$app/Contents/Library/LaunchDaemons/"
echo "Bundled $app"
