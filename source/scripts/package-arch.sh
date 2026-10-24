#!/bin/sh
# Makes an Arch Linux source package: dist/arch/glass-<version>.tar.gz and a
# PKGBUILD for it, then `cd dist/arch && makepkg -si` (on Arch, from a clone of
# the repository, or after copying dist/arch/ over).
set -e
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# Cargo's 1.0.0-beta.1 is pacman's 1.0.0beta1 (sorts before 1.0.0, as it should).
PKGVER=$(echo "$VERSION" | sed 's/-\([a-z]*\)\.\([0-9]*\)$/\1\2/; s/-//g')
OUT=dist/arch
STAGE="$OUT/stage/glass-$PKGVER"
rm -rf "$OUT"
mkdir -p "$OUT"
if git rev-parse --verify -q HEAD >/dev/null 2>&1; then
  # From a git checkout: exactly the committed source.
  [ -z "$(git status --porcelain)" ] || echo "Note: uncommitted changes aren't in the package (it's built from the last commit)." >&2
  TREE=HEAD
  PREFIX=$(git rev-parse --show-prefix)
  [ -z "$PREFIX" ] || TREE="HEAD:${PREFIX%/}"
  git archive --format=tar.gz --prefix="glass-$PKGVER/" -o "$OUT/glass-$PKGVER.tar.gz" "$TREE"
else
  # Otherwise the working tree, without builds, packages, local tooling or the LAME
  # source (Arch builds against its lame package; see .gitattributes).
  mkdir -p "$STAGE"
  rsync -a ./ "$STAGE/" --exclude '/target' --exclude 'tools/*/target' --exclude '/dist' --exclude '/.git' --exclude '/.claude' --exclude '/vendor/lame-3.100' --exclude '.DS_Store'
  tar -C "$OUT/stage" -czf "$OUT/glass-$PKGVER.tar.gz" "glass-$PKGVER"
  rm -rf "$OUT/stage"
fi
if command -v sha256sum >/dev/null 2>&1; then SUM=$(sha256sum "$OUT/glass-$PKGVER.tar.gz" | cut -d' ' -f1); else SUM=$(shasum -a 256 "$OUT/glass-$PKGVER.tar.gz" | cut -d' ' -f1); fi
sed "s/@PKGVER@/$PKGVER/; s/@SHA256@/$SUM/" packaging/arch/PKGBUILD.in > "$OUT/PKGBUILD"
# makepkg sources it as bash: a stray quote in a description stops it cold.
bash -n "$OUT/PKGBUILD"
echo "Built $OUT/glass-$PKGVER.tar.gz and $OUT/PKGBUILD"
echo "On Arch: cd $OUT && makepkg -si"
