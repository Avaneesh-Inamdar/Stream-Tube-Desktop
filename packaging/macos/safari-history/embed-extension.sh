#!/bin/bash
set -euo pipefail

source_dir=$(cd "$(dirname "$0")" && pwd -P)
app=$1
identity=$2
bundle_id=io.github.avaneesh-inamdar.StreamTube.SafariPrototype
name="Stream Tube Safari History"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/streamtube-safari-extension.XXXXXXXX")
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/project"

xcrun safari-web-extension-converter "$source_dir/web" --project-location "$scratch/project" \
    --app-name "$name" --bundle-identifier "$bundle_id" --swift --macos-only --no-open --no-prompt >&2
project="$scratch/project/$name"
cp "$source_dir/SafariWebExtensionHandler.swift" "$project/$name Extension/"
xcodebuild -project "$project/$name.xcodeproj" -target "$name Extension" \
    -configuration Release -arch arm64 CONFIGURATION_BUILD_DIR="$scratch/products" CODE_SIGNING_ALLOWED=NO \
    > "$scratch/build.log" 2>&1 || { cat "$scratch/build.log" >&2; exit 1; }

mkdir -p "$app/Contents/PlugIns"
extension="$app/Contents/PlugIns/$name Extension.appex"
ditto "$scratch/products/$name Extension.appex" "$extension"
codesign --force --sign "$identity" --entitlements "$source_dir/entitlements.plist" "$extension" >&2

info="$app/Contents/Info.plist"
plutil -replace CFBundleIdentifier -string "$bundle_id" "$info"
plutil -replace CFBundleName -string "Stream Tube Safari Prototype" "$info"
plutil -replace CFBundleURLTypes.0.CFBundleURLSchemes.0 -string streamtube-safari-prototype "$info"
