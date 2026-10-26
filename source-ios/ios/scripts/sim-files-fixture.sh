#!/bin/sh
# Puts a "Spotify Local" folder (two short songs, one in a subfolder) in a simulator's
# Files › On My iPhone, for LocalFoldersUITests.
# Usage: scripts/sim-files-fixture.sh <simulator UDID>
set -eu
groups="$HOME/Library/Developer/CoreSimulator/Devices/$1/data/Containers/Shared/AppGroup"
root=""
for g in "$groups"/*; do
  if [ "$(plutil -extract MCMMetadataIdentifier raw "$g/.com.apple.mobile_container_manager.metadata.plist" 2>/dev/null)" = group.com.apple.FileProvider.LocalStorage ]; then
    root="$g/File Provider Storage"
  fi
done
[ -n "$root" ] || { echo "no On My iPhone storage in simulator $1 (open the Files app once)" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# Three seconds of a tone, as AAC in M4A.
python3 -I - "$work/tone.wav" <<'PY'
import math, struct, sys, wave
with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(44100)
    w.writeframes(b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * 440 * i / 44100))) for i in range(44100 * 3)))
PY
mkdir -p "$root/Spotify Local/Harbor"
afconvert -f m4af -d aac "$work/tone.wav" "$root/Spotify Local/Night Drive.m4a"
afconvert -f m4af -d aac "$work/tone.wav" "$root/Spotify Local/Harbor/Harbor Lights.m4a"
echo "$root/Spotify Local"
