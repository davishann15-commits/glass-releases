#!/bin/sh
# Type-checks and lints the Linux build from a Mac:
#   ./scripts/check-linux.sh
# Nothing is linked, so the C libraries only have to look installed: clang
# compiles ring's and OpenSSL's C for Linux against macOS's C headers and
# Homebrew's OpenSSL headers, and a stand-in pkg-config says ALSA and D-Bus are
# there. Needs the x86_64-unknown-linux-gnu target (rustup target add) and
# Homebrew's openssl@3. It checks that the code compiles, not that it runs.
set -e
cd "$(dirname "$0")/.."
OPENSSL=$(brew --prefix openssl@3 2>/dev/null) || { echo "Needs Homebrew's openssl@3." >&2; exit 1; }
SDK=$(xcrun --show-sdk-path)
FAKE=$(mktemp -d)
trap 'rm -rf "${FAKE:?}"' EXIT
cat > "$FAKE/pkg-config" <<'EOF'
#!/bin/sh
for a in "$@"; do
  case "$a" in
    --modversion) echo "1.2.0"; exit 0 ;;
    --version) echo "0.29.2"; exit 0 ;;
  esac
done
echo ""
EOF
chmod +x "$FAKE/pkg-config"
export PKG_CONFIG="$FAKE/pkg-config" PKG_CONFIG_ALLOW_CROSS=1
export X86_64_UNKNOWN_LINUX_GNU_OPENSSL_DIR="$OPENSSL" X86_64_UNKNOWN_LINUX_GNU_OPENSSL_INCLUDE_DIR="$OPENSSL/include" X86_64_UNKNOWN_LINUX_GNU_OPENSSL_LIB_DIR="$OPENSSL/lib"
export CC_x86_64_unknown_linux_gnu=clang AR_x86_64_unknown_linux_gnu=ar CFLAGS_x86_64_unknown_linux_gnu="--target=x86_64-linux-gnu -isystem $SDK/usr/include"
cargo clippy --all-targets --target x86_64-unknown-linux-gnu --target-dir target/linux-check -- -D warnings
echo "The Linux build type-checks, with no warnings."
