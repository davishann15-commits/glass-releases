#!/bin/bash
# Builds the Rust core (ios/core) for iOS and hands it to Xcode:
#   ios/build/headers/   the C header and module map Swift imports (glass_coreFFI)
#   ios/Glass/Generated/glass_core.swift   UniFFI's Swift bindings (generated; not committed)
#   $BUILT_PRODUCTS_DIR/libglass_core.a   when Xcode runs it: the library for exactly the
#       platform and configuration being built, which the app target links
#
# Usage: ios/scripts/build-core.sh [sim|device|all] [debug|release]
# Xcode's "Build Rust core" phase calls it with the platform and configuration
# being built, so a normal Xcode build (or xcodebuild) is all that's needed.
# (The app used to link an XCFramework; Xcode reads one before any build phase runs,
# so switching between simulator and device could find only the other platform's slice.)
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"   # ios/
WHAT="${1:-sim}"
PROFILE="${2:-debug}"

# Xcode runs build phases with a minimal PATH.
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
# The macOS deployment flags Xcode exports must not reach the iOS cross-builds
# (cc-rs would pass -mmacosx-version-min to clang for C dependencies).
unset MACOSX_DEPLOYMENT_TARGET SDKROOT
export IPHONEOS_DEPLOYMENT_TARGET=26.0

case "$WHAT" in
  sim) TARGETS=(aarch64-apple-ios-sim) ;;
  device) TARGETS=(aarch64-apple-ios) ;;
  all) TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim) ;;
  *) echo "usage: $0 [sim|device|all] [debug|release]" >&2; exit 2 ;;
esac
case "$PROFILE" in
  debug) CARGO_PROFILE=dev; OUT=debug ;;
  release) CARGO_PROFILE=release; OUT=release ;;
  *) echo "usage: $0 [sim|device|all] [debug|release]" >&2; exit 2 ;;
esac

export CARGO_TARGET_DIR="$HERE/target"
cd "$HERE/core"
for t in "${TARGETS[@]}"; do
  echo "== cargo build --target $t --profile $CARGO_PROFILE"
  cargo build --locked --target "$t" --profile "$CARGO_PROFILE"
done

# The binding generator (host tool), built once.
BINDGEN="$CARGO_TARGET_DIR/release/uniffi-bindgen"
if [ ! -x "$BINDGEN" ]; then
  (cd "$HERE/bindgen" && cargo build --locked --release)
fi

GEN="$HERE/build/gen"
rm -rf "$GEN" && mkdir -p "$GEN"
LIB="$CARGO_TARGET_DIR/${TARGETS[0]}/$OUT/libglass_core.a"
"$BINDGEN" generate --library "$LIB" --language swift --out-dir "$GEN" >/dev/null

HEADERS="$HERE/build/headers"
rm -rf "$HEADERS" && mkdir -p "$HEADERS"
cp "$GEN/glass_coreFFI.h" "$HEADERS/"
cp "$GEN/glass_coreFFI.modulemap" "$HEADERS/module.modulemap"

# Swift bindings go into the app's sources (only rewritten when they change,
# so Xcode doesn't recompile them every build).
mkdir -p "$HERE/Glass/Generated"
if ! cmp -s "$GEN/glass_core.swift" "$HERE/Glass/Generated/glass_core.swift"; then
  cp "$GEN/glass_core.swift" "$HERE/Glass/Generated/glass_core.swift"
fi

# In Xcode: the library for the build being made, where the target links it from
# (only replaced when it changed, so an unchanged core doesn't relink the app).
if [ -n "${BUILT_PRODUCTS_DIR:-}" ]; then
  mkdir -p "$BUILT_PRODUCTS_DIR"
  if ! cmp -s "$LIB" "$BUILT_PRODUCTS_DIR/libglass_core.a"; then
    cp "$LIB" "$BUILT_PRODUCTS_DIR/libglass_core.a"
  fi
  echo "== $BUILT_PRODUCTS_DIR/libglass_core.a"
else
  for t in "${TARGETS[@]}"; do echo "== $CARGO_TARGET_DIR/$t/$OUT/libglass_core.a"; done
fi
