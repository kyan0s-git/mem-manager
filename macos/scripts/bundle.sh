#!/usr/bin/env bash
# Assembles MemManager.app from the SwiftPM build (docs/architecture/macos.md §2).
# Usage: VERSION=0.1.0 macos/scripts/bundle.sh [debug|release]   (run on macOS)
#
# Produces an ad-hoc signed bundle unless SIGN_IDENTITY is set to a
# "Developer ID Application: …" identity, in which case it signs with the
# hardened runtime for notarization. The privileged helper can only be
# registered by a Developer-ID-signed app.
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

if [[ -n "${VERSION:-}" ]]; then
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$app/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${BUILD_NUMBER:-$VERSION}" "$app/Contents/Info.plist"
fi

identity="${SIGN_IDENTITY:--}"
codesign --force --options runtime --timestamp=none --identifier dev.memmanager.helper -s "$identity" \
  "$app/Contents/MacOS/MemManagerHelper"
if [[ "$identity" == "-" ]]; then
  codesign --force --options runtime -s - "$app"
else
  codesign --force --options runtime --timestamp -s "$identity" "$app"
fi
codesign --verify --deep --strict "$app"
echo "Bundled $app"
