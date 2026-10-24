#!/usr/bin/env python3
"""Writes packaging/THIRD-PARTY-NOTICES.txt: every crate compiled into Glass on
macOS and Linux, its license, and the license texts its authors ship, which
their licenses ask to go along with the app.

Run from the repository after changing dependencies:
    python3 scripts/third-party-notices.py
"""
import hashlib
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "packaging", "THIRD-PARTY-NOTICES.txt")
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
LICENSE_NAMES = ("license", "licence", "copying", "notice", "ofl", "ufl", "unlicense")


def metadata():
    out = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def shipped(target):
    """(name, version) of every crate compiled into Glass for `target`: normal
    dependencies only, as a release build resolves them (no dev or build ones), and
    no proc-macro crates (they run while compiling and aren't in the app)."""
    out = subprocess.run(
        ["cargo", "tree", "-e", "normal,no-proc-macro", "--target", target, "--prefix", "none", "-f", "{p}", "--locked"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    found = set()
    for line in out.splitlines():
        # "name v1.2.3", "name v1.2.3 (/path)" for path dependencies, "(*)" for repeats.
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            found.add((parts[0], parts[1][1:]))
    found.discard(("glass", read_version()))
    return found


def read_version():
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as h:
        for line in h:
            if line.startswith("version"):
                return line.split('"')[1]
    return ""


def license_files(p):
    """The license texts in a package's folder (and a license-file it names)."""
    folder = os.path.dirname(p["manifest_path"])
    found = []
    if p.get("license_file"):
        found.append(os.path.join(folder, p["license_file"]))
    for dirpath, dirnames, filenames in os.walk(folder):
        depth = dirpath[len(folder):].count(os.sep)
        # The crate root, and one level down (fonts/, a bundled C library).
        if depth > 1:
            dirnames[:] = []
            continue
        dirnames[:] = [d for d in dirnames if d not in ("target", "tests", "benches", "examples", ".git")]
        for f in filenames:
            if f.lower().startswith(LICENSE_NAMES) and not f.endswith((".rs", ".toml", ".json", ".py")):
                found.append(os.path.join(dirpath, f))
    return sorted(set(found))


STANDARD = {
    # SPDX id -> how to recognize its full text among the files crates ship
    "Apache-2.0": "apache license\nversion 2.0, january 2004",
    "MIT": "permission is hereby granted, free of charge",
    "MPL-2.0": "mozilla public license version 2.0",
    "BSD-3-Clause": "neither the name of",
    "ISC": "permission to use, copy, modify, and/or distribute this software for any",
    "Zlib": "this software is provided 'as-is', without any express or implied",
}


def without_copyright(text):
    """A license text as a standard one: the project's own copyright lines left out."""
    lines = text.splitlines()
    while lines and (not lines[0].strip() or lines[0].strip().lower().startswith(("copyright", "(c)", "©"))):
        lines.pop(0)
    return "\n".join(lines).strip()


def spdx_ids(expr):
    for t in (expr or "").replace("(", " ").replace(")", " ").replace("/", " ").split():
        if t not in ("OR", "AND", "WITH"):
            yield t


def main():
    meta = {(p["name"], p["version"]): p for p in metadata()["packages"]}
    wanted = set()
    for target in TARGETS:
        wanted |= shipped(target)
    packages = sorted((meta[k] for k in wanted if k in meta), key=lambda p: (p["name"].lower(), p["version"]))
    texts = {}  # normalized sha -> (text, [package names])
    standard = {}  # SPDX id -> a full text of it
    missing = []
    for p in packages:
        files = license_files(p)
        if not files:
            missing.append(p)
        for f in files:
            try:
                with open(f, encoding="utf-8", errors="replace") as h:
                    text = h.read().strip()
            except OSError:
                continue
            if not text:
                continue
            flat = " ".join(text.split()).lower()
            for spdx, mark in STANDARD.items():
                if " ".join(mark.split()) in flat and spdx not in standard and len(list(spdx_ids(p.get("license")))) <= 2:
                    standard[spdx] = without_copyright(text)
            # The same text but for spacing is one text. So is the Apache License
            # itself, whatever is filled in its appendix (NOTICE files stay apart).
            apache = flat.find("apache license version 2.0, january 2004")
            if 0 <= apache < 200 and "end of terms and conditions" in flat and flat.count("apache license version 2.0") == 1:
                key = "apache-2.0"
            else:
                key = hashlib.sha256(flat.encode()).hexdigest()
            texts.setdefault(key, (text, []))[1].append(f'{p["name"]} {p["version"]}')
    def read(rel):
        with open(os.path.join(ROOT, rel), encoding="utf-8") as h:
            return h.read().strip()

    lines = [
        "Glass is under the MIT License:",
        "",
        read("LICENSE"),
        "",
        "=" * 78,
        "",
        "Glass includes the open-source software listed below. Each is listed with",
        "its license; the license texts its authors provide follow the list.",
        "",
        "The Rust crates are used as published, with their source at",
        "https://crates.io/crates/<name>/<version>, except librespot-playback and",
        "librespot-connect (MIT), which Glass changes.",
        "",
        "LAME 3.100, the MP3 encoder, is a separate library under the GNU Library",
        "General Public License, version 2 (the LGPL; its text is at the end): on macOS",
        "Glass.app/Contents/Frameworks/libmp3lame.0.dylib, built from LAME 3.100's",
        "source (with GNU's 2024 config.guess and config.sub in place of its 2015",
        "ones); on Linux, the system's libmp3lame. You may replace it with your own",
        "build of LAME. Its source:",
        "https://github.com/davishann15-commits/glass-releases/releases/tag/lame-3.100",
        "(LAME's home is https://lame.sourceforge.io).",
        "",
        "=" * 78,
        "",
    ]
    for p in packages:
        lines.append(f'{p["name"]} {p["version"]}: {p.get("license") or "see its license text"}')
    lines += ["", "=" * 78, ""]
    if missing:
        lines.append("These ship no license file of their own; their authors and licenses are")
        lines.append("below, and each license's standard text is under \"Standard license texts\".")
        lines.append("")
        for p in missing:
            authors = ", ".join(a.split(" <")[0] for a in p.get("authors") or []) or f'the {p["name"]} authors'
            lines.append(f'{p["name"]} {p["version"]} ({p.get("license")}): Copyright (c) {authors}')
        lines += ["", "=" * 78, ""]
    for text, names in sorted(texts.values(), key=lambda t: t[1][0].lower()):
        lines.append("License text for: " + ", ".join(sorted(set(names))))
        lines.append("-" * 78)
        lines.append(text)
        lines += ["", "=" * 78, ""]
    def wanted(expr):
        ids = list(spdx_ids(expr))
        # "A OR B": one of them is enough; else every one named.
        if " OR " in (expr or "") and " AND " not in expr:
            have = [i for i in ids if i in standard]
            return have[:1] or ids[:1]
        return ids

    needed = sorted({i for p in missing for i in wanted(p.get("license"))})
    lines.append("Standard license texts")
    lines += ["", "=" * 78, ""]
    for spdx in needed:
        if spdx in standard:
            lines.append(f"{spdx}")
            lines.append("-" * 78)
            lines.append(standard[spdx])
            lines += ["", "=" * 78, ""]
        else:
            print(f"warning: no standard text found for {spdx}", file=sys.stderr)
    lines.append("LAME 3.100: GNU Library General Public License, version 2")
    lines.append("-" * 78)
    lines.append(read("vendor/lame-3.100/COPYING"))
    lines += ["", "=" * 78, ""]
    with open(OUT, "w", encoding="utf-8") as h:
        h.write("\n".join(lines).rstrip() + "\n")
    print(f"{len(packages)} packages, {len(texts)} distinct license texts, {len(missing)} without their own -> {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    sys.exit(main())
