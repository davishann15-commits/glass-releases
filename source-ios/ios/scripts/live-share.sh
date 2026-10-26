#!/bin/sh
# Local files sharing for real between the iPhone app (simulator) and the desktop app,
# over this Mac's network: starts the desktop demo with sharing on and three songs,
# puts a song in the simulator app's Documents, runs SharingLiveUITests, then checks
# the song the iPhone sent arrived on the desktop. Both are signed in as the demo
# account, which only demos have, so no real account or device is involved.
# Usage (from ios/, after `cargo build` at the top): scripts/live-share.sh <simulator UDID>
set -eu
udid=$1
work=$(mktemp -d)
trap 'kill "$desktop" 2>/dev/null || true; rm -rf "$work"' EXIT
python3 -I - "$work" <<'PY'
import math, struct, sys, wave, os
for n, secs, f in [("one", 3, 440), ("two", 4, 523), ("three", 5, 659), ("phone", 6, 784)]:
    with wave.open(os.path.join(sys.argv[1], n + ".wav"), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(44100)
        w.writeframes(b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * f * i / 44100))) for i in range(44100 * secs)))
PY
mkdir -p "$work/home" "$work/music"
for n in One Two Three; do
  afconvert -f m4af -d aac "$work/$(echo $n | tr A-Z a-z).wav" "$work/music/Mac Song $n.m4a"
done
GLASS_DEMO="$work/home" GLASS_DEMO_SHARE="$work/music" GLASS_ROUTE=local ../target/debug/glass >"$work/desktop.log" 2>&1 &
desktop=$!
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b >/dev/null
xcodebuild -project Glass.xcodeproj -scheme Glass -destination "id=$udid" -derivedDataPath build/dd build-for-testing -quiet
xcrun simctl install "$udid" "$(find build/dd/Build/Products/Debug-iphonesimulator -maxdepth 1 -name Glass.app)"
docs="$(xcrun simctl get_app_container "$udid" app.glass.ios data)/Documents"
mkdir -p "$docs"
rm -f "$docs"/Mac\ Song\ *.m4a "$docs/Computer Song.m4a"
afconvert -f m4af -d aac "$work/phone.wav" "$docs/Phone Song.m4a"
TEST_RUNNER_GLASS_LIVE_SHARE="$(scutil --get ComputerName)" xcodebuild -project Glass.xcodeproj -scheme Glass -destination "id=$udid" \
  -derivedDataPath build/dd test-without-building -only-testing:GlassUITests/SharingLiveUITests -collect-test-diagnostics never
if [ -f "$work/home/Music/Glass/Phone Song.m4a" ]; then echo "ok: the iPhone's song is on the desktop"; else echo "FAIL: the iPhone's song didn't arrive on the desktop" >&2; exit 1; fi
# The other way round: a computer connects to the iPhone app (lists its songs, gets
# one, sends one). Only the iPhone app is open now, so it's the one found.
kill "$desktop"
xcrun simctl launch --terminate-running-process "$udid" app.glass.ios -demo -demoShare -demoFolders fresh -route local >/dev/null
afconvert -f m4af -d aac "$work/three.wav" "$work/Computer Song.m4a"
(cd .. && GLASS_LIVE_SONG="$work/Computer Song.m4a" cargo test --bin glass live_demo_computer -- --ignored --nocapture 2>&1 | grep -E "^(found|ok|test |thread)|lists|panicked")
# (The test run reinstalled the app: its folder has moved.)
docs="$(xcrun simctl get_app_container "$udid" app.glass.ios data)/Documents"
if [ -f "$docs/Computer Song.m4a" ]; then echo "ok: the computer's song is on the iPhone"; else echo "FAIL: the computer's song didn't arrive on the iPhone" >&2; exit 1; fi
