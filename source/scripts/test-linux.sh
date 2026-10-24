#!/bin/sh
# Builds, tests and starts the Linux version in a Debian container:
#   ./scripts/test-linux.sh
# Needs Docker (on a Mac, Colima: `colima start --cpu 4 --memory 6`). Uses the
# committed source, as the Arch package does. Runs the test suite, opens ALSA's
# default output (plug over null, which lists formats as a desktop's does), and
# starts the demo on a virtual X display, saving target/linux-home.png.
set -e
cd "$(dirname "$0")/.."
NAME=glass-linux-test
RUST=$(rustc --version | cut -d' ' -f2)
docker rm -f "$NAME" >/dev/null 2>&1 || true
trap 'docker rm -f "$NAME" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$NAME" debian:bookworm-slim sleep infinity >/dev/null
docker exec "$NAME" sh -c "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
  curl ca-certificates build-essential pkg-config libasound2-dev libssl-dev libdbus-1-dev libmp3lame-dev \
  libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
  libgl1-mesa-dev libgl1-mesa-dri libegl1 libfontconfig1-dev fonts-dejavu-core xvfb xauth >/dev/null &&
  curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain $RUST >/dev/null"
docker exec "$NAME" mkdir -p /src
TREE=HEAD
PREFIX=$(git rev-parse --show-prefix)
[ -z "$PREFIX" ] || TREE="HEAD:${PREFIX%/}"
git -C "$(git rev-parse --show-toplevel)" archive "$TREE" | docker exec -i "$NAME" tar -x -C /src
docker exec "$NAME" sh -c 'printf "pcm.!default {\n  type plug\n  slave.pcm \"null\"\n}\n" > /etc/asound.conf'
docker exec -e CARGO_PROFILE_DEV_DEBUG=0 "$NAME" sh -c '. $HOME/.cargo/env && cd /src &&
  cargo test -q --locked &&
  cargo test -q --locked -p librespot-playback --lib -- glass &&
  cargo test -q --locked -p librespot-playback --lib glass_default_output -- --ignored &&
  cargo build -q --locked &&
  (Xvfb :99 -screen 0 1600x1000x24 >/dev/null 2>&1 &) && sleep 2 &&
  DISPLAY=:99 GLASS_DEMO=1 GLASS_ROUTE=home GLASS_WINDOW=1280x820 GLASS_SCREENSHOT=/tmp/linux-home.png timeout 90 ./target/debug/glass >/dev/null 2>&1'
docker cp "$NAME:/tmp/linux-home.png" target/linux-home.png
echo "Linux: tests pass, the default ALSA output opens, and the demo started (target/linux-home.png)."
