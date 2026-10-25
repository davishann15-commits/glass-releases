#!/bin/sh
# Publishes a Glass release. Installed copies download it by themselves:
#   ./scripts/release-macos.sh "What's new in this version"
# Uses the version in Cargo.toml. Builds the universal app, signs it with the
# release key (in this Mac's Keychain; tools/glass-release), and publishes the
# DMG, the update zip and glass-update.json to the public releases repository.
set -e
cd "$(dirname "$0")/.."
REPO=davishann15-commits/glass-releases
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NOTES="${1:-Glass $VERSION}"
if [ -n "$(git status --porcelain)" ]; then
  echo "Commit your changes first (new files too): a release is built from what's committed." >&2
  exit 1
fi
# The licenses shipped with the app must match its dependencies.
python3 scripts/third-party-notices.py >/dev/null
if ! git diff --quiet packaging/THIRD-PARTY-NOTICES.txt; then
  echo "packaging/THIRD-PARTY-NOTICES.txt changed with the dependencies: commit it first." >&2
  exit 1
fi
if gh release view "v$VERSION" --repo "$REPO" >/dev/null 2>&1; then
  echo "v$VERSION is already released: raise the version in Cargo.toml." >&2
  exit 1
fi

SIGNER=tools/glass-release/target/release/glass-release
# Rebuilt when its source changed, so it signs what this version of src/update.rs checks.
cargo build --release --quiet --manifest-path tools/glass-release/Cargo.toml
# Installed copies accept only updates signed with the key whose public half they carry.
PUB=$("$SIGNER" pubkey) || { echo "Couldn't read the release key from the Keychain." >&2; exit 1; }
if [ -z "$PUB" ] || ! grep -qF "\"$PUB\"" src/update.rs; then
  echo "The release key in this Keychain isn't the one in src/update.rs: installed copies would refuse this update." >&2
  exit 1
fi

./scripts/package-macos.sh
ZIP="dist/Glass-$VERSION-macos.zip"
DMG="dist/Glass-$VERSION.dmg"

SIGNED=$("$SIGNER" sign "$VERSION" macos "$ZIP")
SHA=${SIGNED%% *}
SIG=${SIGNED#* }
SIZE=$(stat -f%z "$ZIP")
python3 - "$VERSION" "$NOTES" "$REPO" "$ZIP" "$SIZE" "$SHA" "$SIG" > dist/glass-update.json <<'PY'
import json, os, sys
version, notes, repo, zip_path, size, sha, sig = sys.argv[1:]
url = f"https://github.com/{repo}/releases/download/v{version}/{os.path.basename(zip_path)}"
print(json.dumps({"version": version, "notes": notes, "assets": {"macos": {"url": url, "size": int(size), "sha256": sha, "signature": sig}}}, indent=2))
PY

# Installed copies read glass-update.json from the release marked latest.
gh release create "v$VERSION" --repo "$REPO" --latest --title "Glass $VERSION" --notes "$NOTES" "$DMG" "$ZIP" dist/glass-update.json
echo "Released Glass $VERSION: https://github.com/$REPO/releases/tag/v$VERSION"
