#!/usr/bin/env bash
# Packs a release build into "dist/Stream Tube.app" and a .dmg next to it.
#
#   cargo build --release && packaging/macos/build-app.sh
#
# libmpv and every library it needs (FFmpeg, ...) are copied into
# Contents/Frameworks, so the app plays video without Homebrew. That needs
# Homebrew's mpv and dylibbundler: `brew install mpv dylibbundler`.
# The app is signed ad hoc; without a Developer ID, Gatekeeper asks once
# (right click > Open, or `xattr -cr "Stream Tube.app"`).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

binary=${1:-target/release/streamtube}
[ -x "$binary" ] || { echo "no $binary: run cargo build --release first" >&2; exit 1; }
version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
arch=$(lipo -archs "$binary" | tr ' ' '-')
command -v dylibbundler >/dev/null || { echo "dylibbundler missing: brew install dylibbundler" >&2; exit 1; }
mpv_lib="${MPV_LIB:-$(brew --prefix mpv)/lib/libmpv.2.dylib}"
[ -f "$mpv_lib" ] || { echo "no $mpv_lib: brew install mpv" >&2; exit 1; }

app="dist/Stream Tube.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"

cp "$binary" "$app/Contents/MacOS/streamtube"
strip -x "$app/Contents/MacOS/streamtube"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
cp LICENSE README.md "$app/Contents/Resources/"

# icon: every size iconutil wants, scaled from the logo
iconset=$(mktemp -d)/streamtube.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/logo.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" assets/logo.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/streamtube.icns"
rm -rf "$(dirname "$iconset")"

# libmpv is loaded at runtime from Contents/Frameworks (src/player/mpv.rs);
# its dependencies go next to it and are found through @loader_path
frameworks="$app/Contents/Frameworks"
cp -L "$mpv_lib" "$frameworks/libmpv.2.dylib"
chmod u+w "$frameworks/libmpv.2.dylib"
install_name_tool -id "@loader_path/libmpv.2.dylib" "$frameworks/libmpv.2.dylib"
dylibbundler -of -b -cd -x "$frameworks/libmpv.2.dylib" -d "$frameworks" -p "@loader_path/" \
  -s "$(brew --prefix)/lib" </dev/null
# dylibbundler rewrites each old rpath to @loader_path/; dyld refuses duplicates
for lib in "$frameworks"/*.dylib; do
  while [ "$(otool -l "$lib" | grep -c 'path @loader_path/ ')" -gt 1 ]; do
    install_name_tool -delete_rpath "@loader_path/" "$lib" 2>/dev/null
  done
done

codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

dmg="dist/StreamTube-${version}-macos-${arch}.dmg"
rm -f "$dmg"
hdiutil create -volname "Stream Tube" -srcfolder "$app" -ov -format UDZO "$dmg" >/dev/null
echo "$app"
echo "$dmg"
