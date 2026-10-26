#!/bin/sh
# Builds a shareable Glass for macOS: a universal app (Apple silicon and
# Intel) in dist/Glass-<version>.dmg, ad-hoc signed (not notarized).
set -e
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
SHORT=$(echo "$VERSION" | sed 's/-.*//')
OUT=dist/macos
APP="$OUT/Glass.app"
rm -rf "$OUT" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"

# A release build: it installs its own updates (see src/update.rs).
export GLASS_RELEASE=1
# Keep this computer's paths (and user name) out of the shared binary.
export RUSTFLAGS="--remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$PWD=/glass --remap-path-prefix=$HOME=/home"
for T in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target add "$T" >/dev/null 2>&1 || true
  MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --target "$T"
done
# LAME, the MP3 encoder, is a library of its own (see build.rs): the app carries
# it in Contents/Frameworks, one universal file. Each binary's rpath to its
# build's copy (an absolute path on this Mac) is taken out of copies of them.
THIN="$OUT/thin"
mkdir -p "$THIN"
LAMES=""
for T in aarch64-apple-darwin x86_64-apple-darwin; do
  cp "target/$T/release/glass" "$THIN/glass-$T"
  DEV=$(otool -l "$THIN/glass-$T" | awk '/LC_RPATH/ {getline; getline; print $2}' | grep -v '^@')
  for R in $DEV; do
    [ -f "$R/libmp3lame.0.dylib" ] && LAMES="$LAMES $R/libmp3lame.0.dylib"
    install_name_tool -delete_rpath "$R" "$THIN/glass-$T"
  done
done
[ "$(echo $LAMES | wc -w)" -eq 2 ] || { echo "LAME wasn't built for both architectures" >&2; exit 1; }
lipo -create -output "$APP/Contents/Frameworks/libmp3lame.0.dylib" $LAMES
lipo -create -output "$APP/Contents/MacOS/glass" "$THIN/glass-aarch64-apple-darwin" "$THIN/glass-x86_64-apple-darwin"
rm -rf "$THIN"

# Apple's on-device lyrics translation (used on macOS 26 and later).
if command -v swiftc >/dev/null 2>&1; then
  mkdir -p target/translate
  for arch in arm64 x86_64; do
    swiftc -O -parse-as-library -target $arch-apple-macos26.0 -o target/translate/glass-translate-$arch tools/glass-translate/main.swift ||
      { echo "glass-translate needs the macOS 26 SDK (Xcode 26 or its Command Line Tools)." >&2; exit 1; }
  done
  lipo -create -output "$APP/Contents/MacOS/glass-translate" target/translate/glass-translate-arm64 target/translate/glass-translate-x86_64
fi

cp packaging/icon/Glass.icns "$APP/Contents/Resources/Glass.icns"
cp packaging/THIRD-PARTY-NOTICES.txt "$APP/Contents/Resources/THIRD-PARTY-NOTICES.txt"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Glass</string>
  <key>CFBundleDisplayName</key><string>Glass</string>
  <key>CFBundleIdentifier</key><string>app.glass.player</string>
  <key>CFBundleExecutable</key><string>glass</string>
  <key>CFBundleIconFile</key><string>Glass</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$SHORT</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>NSLocalNetworkUsageDescription</key><string>Glass finds Glass on your other devices on this network, to share local songs with them.</string>
  <key>NSBonjourServices</key><array><string>_glass-files._tcp</string></array>
</dict>
</plist>
PLIST
# Ad-hoc signatures (inside out): enough to run, not to skip Gatekeeper.
codesign --force --sign - "$APP/Contents/Frameworks/libmp3lame.0.dylib"
[ -f "$APP/Contents/MacOS/glass-translate" ] && codesign --force --sign - "$APP/Contents/MacOS/glass-translate"
codesign --force --sign - "$APP"

# The disk image: the app, a shortcut to Applications, and how to open it.
STAGE="$OUT/dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp packaging/macos/READ-ME-FIRST.txt "$STAGE/Read Me First.txt"
DMG="dist/Glass-$VERSION.dmg"
rm -f "$DMG"
hdiutil create -volname "Glass $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
# What installed copies download to update (scripts/release-macos.sh).
ZIP="dist/Glass-$VERSION-macos.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
# Only the disk image and the zip stay: an unpacked Glass.app here would show
# up on this Mac as a second copy of Glass.
rm -rf "$OUT"
echo "Built $DMG"
