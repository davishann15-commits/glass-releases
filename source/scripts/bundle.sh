#!/bin/sh
# Build Glass for this computer and wrap it in target/Glass.app:
#   ./scripts/bundle.sh [local|release|debug]
# `local` (the default) is optimized but quick to rebuild; `release` is the
# slower, fully optimized build that the DMG and Arch package use.
# If Glass is running, the new build replaces it when it opens.
set -e
cd "$(dirname "$0")/.."
PROFILE="${1:-local}"
case "$PROFILE" in
  debug) DIR=debug ;;
  *) DIR="$PROFILE" ;;
esac
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="target/Glass.app"

echo "==> Building Glass $VERSION ($PROFILE)"
if [ ! -x "target/$DIR/glass" ]; then
  echo "    The first build compiles everything and takes a few minutes; later ones take well under a minute."
fi
if [ "$PROFILE" = "debug" ]; then cargo build; else cargo build --profile "$PROFILE"; fi

echo "==> Making $APP"
RUNNING=$(pgrep -x glass >/dev/null 2>&1 && echo yes || true)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "target/$DIR/glass" "$APP/Contents/MacOS/glass"
# LAME, the MP3 encoder, a library of its own (see build.rs).
LAME_DIR=$(otool -l "target/$DIR/glass" | awk '/LC_RPATH/ {getline; getline; print $2}' | grep -v '^@' | head -1)
cp "$LAME_DIR/libmp3lame.0.dylib" "$APP/Contents/Frameworks/libmp3lame.0.dylib"
cp packaging/icon/Glass.icns "$APP/Contents/Resources/Glass.icns"
cp packaging/THIRD-PARTY-NOTICES.txt "$APP/Contents/Resources/THIRD-PARTY-NOTICES.txt"
# Apple's on-device translation for lyrics (macOS 26+). Optional: without a
# Swift toolchain Glass builds as usual and translates with the assistant.
if command -v swiftc >/dev/null 2>&1; then
  if [ ! -x "target/$DIR/glass-translate" ] || [ tools/glass-translate/main.swift -nt "target/$DIR/glass-translate" ]; then
    swiftc -O -parse-as-library -target "$(uname -m)-apple-macos26.0" -o "target/$DIR/glass-translate" tools/glass-translate/main.swift 2>/dev/null ||
      echo "note: couldn't build glass-translate (needs the macOS 26 SDK); lyrics will be translated by the assistant"
  fi
  [ -x "target/$DIR/glass-translate" ] && cp "target/$DIR/glass-translate" "$APP/Contents/MacOS/glass-translate"
fi
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
  <key>CFBundleVersion</key><string>$(echo "$VERSION" | sed 's/-.*//')</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>NSLocalNetworkUsageDescription</key><string>Glass finds your other computers running Glass on this network, to send local songs between them.</string>
  <key>NSBonjourServices</key><array><string>_glass-files._tcp</string></array>
</dict>
</plist>
PLIST
codesign --force --sign - "$APP/Contents/Frameworks/libmp3lame.0.dylib" >/dev/null 2>&1 || { echo "Couldn't sign libmp3lame.0.dylib" >&2; exit 1; }
[ -f "$APP/Contents/MacOS/glass-translate" ] && codesign --force --sign - "$APP/Contents/MacOS/glass-translate" >/dev/null 2>&1
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
# An unpacked copy left by the DMG script would show up as a second Glass.
if [ -d dist/macos/Glass.app ]; then
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u dist/macos/Glass.app >/dev/null 2>&1 || true
  rm -rf dist/macos
fi

if [ -n "$RUNNING" ]; then
  echo "==> Glass is running: opening the new build (it takes over from the old one)"
  # Builds from before the hand-over existed don't answer it: ask them to quit.
  if [ ! -S "$HOME/Library/Caches/Glass/glass.sock" ]; then
    osascript -e 'tell application id "app.glass.player" to quit' >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      pgrep -x glass >/dev/null 2>&1 || break
      sleep 0.5
    done
  fi
  # -n: a new copy even though Glass is running (plain `open` would only
  # bring the old one forward); it takes over from the old one.
  open -n "$APP"
else
  echo "Built $APP. Open it with: open $APP"
fi
