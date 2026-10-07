#!/usr/bin/env bash
# Assembles MemManager.app from the SwiftPM build (docs/architecture/macos.md §2).
# Usage: macos/scripts/bundle.sh [debug|release]   (run on macOS)
#
# Produces an ad-hoc signed bundle. For distribution, re-sign with a
# Developer ID (hardened runtime) and notarize; the privileged helper can
# only be registered by a properly signed app.
set -euo pipefail

config="${1:-release}"
here="$(cd "$(dirname "$0")/.." && pwd)"
cd "$here"

swift build -c "$config" --arch arm64 --arch x86_64
bin="$(swift build -c "$config" --arch arm64 --arch x86_64 --show-bin-path)"

app="$here/build/MemManager.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Library/LaunchDaemons" "$app/Contents/Resources"
cp "$bin/MemManager" "$bin/MemManagerHelper" "$app/Contents/MacOS/"
cp Resources/Info.plist "$app/Contents/Info.plist"
cp Resources/dev.memmanager.helper.plist "$app/Contents/Library/LaunchDaemons/"

codesign --force --options runtime --identifier dev.memmanager.helper -s - "$app/Contents/MacOS/MemManagerHelper"
codesign --force --options runtime -s - "$app"
codesign --verify --deep --strict "$app"
echo "Bundled $app"
