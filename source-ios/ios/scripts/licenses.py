#!/usr/bin/env python3
"""Writes Glass/Resources/THIRD-PARTY.txt: every crate linked into the iOS app (the
core's normal dependencies for aarch64-apple-ios, not build or dev ones), with its
version, license and repository, then each distinct license text found in the
crates' sources, plus LAME (built from source by ios/core/build.rs).

    python3 -I ios/scripts/licenses.py
"""
import glob
import json
import os
import subprocess
import sys

here = os.path.dirname(os.path.abspath(__file__))
core = os.path.join(here, "..", "core")
out_path = os.path.join(here, "..", "Glass", "Resources", "THIRD-PARTY.txt")

meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--offline", "--filter-platform", "aarch64-apple-ios"],
    cwd=core))
pkgs = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = meta["resolve"]["root"]

# Follow normal dependencies only: what ends up in the app.
seen, todo = set(), [root]
while todo:
    nid = todo.pop()
    if nid in seen:
        continue
    seen.add(nid)
    for d in nodes[nid]["deps"]:
        if any(k["kind"] is None for k in d["dep_kinds"]):
            todo.append(d["pkg"])
seen.discard(root)

rows, texts = [], {}
for nid in sorted(seen, key=lambda i: (pkgs[i]["name"], pkgs[i]["version"])):
    p = pkgs[nid]
    lic = p.get("license") or ("see " + p["license_file"] if p.get("license_file") else "unknown")
    rows.append(f'{p["name"]} {p["version"]} — {lic}' + (f' — {p["repository"]}' if p.get("repository") else ""))
    d = os.path.dirname(p["manifest_path"])
    for f in sorted(glob.glob(os.path.join(d, "LICEN[CS]E*")) + glob.glob(os.path.join(d, "COPYING*")) + glob.glob(os.path.join(d, "NOTICE*"))):
        if os.path.isfile(f):
            try:
                t = open(f, encoding="utf-8", errors="replace").read().strip()
            except OSError:
                continue
            if t:
                texts.setdefault(t, []).append(f'{p["name"]} {p["version"]}')

lame = os.path.join(core, "..", "..", "vendor")
# Add to Apple Music (core src/apple_music) is a port of ByeTunes' Swift code; its notice.
BYETUNES = """MIT License

Copyright (c) 2026 Edualexxis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.""".split("\n")
out = [
    "Glass for iOS — open-source software",
    "",
    "Glass is built on the open-source projects below. Thank you to their authors.",
    "",
    "LAME (MP3 encoder, for Save as MP3) — LGPL-2.0-or-later — https://lame.sourceforge.io/",
    "  Built from source and linked into the app. Its source is available from the",
    "  address above; the LGPL's terms apply to it.",
    "",
    "ByeTunes (Add to Apple Music is ported from it) — MIT — https://github.com/EduAlexxis/ByeTunes",
    "",
] + BYETUNES + [
    "",
    f"Rust crates ({len(rows)}):",
    "",
] + rows + ["", "", "License texts", "=" * 13, ""]
for t, who in sorted(texts.items(), key=lambda kv: kv[1][0]):
    out += ["-" * 72, "Used by: " + ", ".join(who), "-" * 72, t, ""]
os.makedirs(os.path.dirname(out_path), exist_ok=True)
open(out_path, "w").write("\n".join(out) + "\n")
print(f"{len(rows)} crates, {len(texts)} distinct license texts -> {os.path.relpath(out_path)}")
