#!/usr/bin/env python3
"""A release's payload and manifest from one built image (tools/make-release.sh calls this in
the build machine; docs/design/updates.md).

    release.py WORK OUT [--channel C] [--notes FILE] [--notes-url URL] [--min-from V]
               [--version V] [--chunk BYTES]

WORK is tools/make-image.sh's work directory (root.img, falcon.itb, falcon-b.itb,
docs/release). Writes OUT/nibos-<version>-<image>.slot (the root in independently
zstd-compressed chunks, then FIT A and FIT B) and OUT/nibos-<version>-<image>.manifest.json,
unsigned. --version replaces the release file's (tools/test-update.sh makes a newer release
from the same build that way).
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys


def env(path):
    out = {}
    for line in open(path):
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            out[k] = v.strip().strip('"')
    return out


def main():
    p = argparse.ArgumentParser()
    p.add_argument("work")
    p.add_argument("out")
    p.add_argument("--channel", default="stable", choices=["stable", "beta"])
    p.add_argument("--notes", help="a file: what's new, a few lines")
    p.add_argument("--notes-url")
    p.add_argument("--min-from")
    p.add_argument("--version")
    p.add_argument("--chunk", type=int, default=16 * 1024 * 1024)
    a = p.parse_args()

    rel = env(os.path.join(a.work, "docs", "release"))
    version = a.version or rel["NIBOS_VERSION"]
    image = rel["NIBOS_IMAGE"]
    base = f"nibos-{version}-{image}"
    os.makedirs(a.out, exist_ok=True)
    slot_path = os.path.join(a.out, base + ".slot")

    root_path = os.path.join(a.work, "root.img")
    root_size = os.path.getsize(root_path)
    root_hash = hashlib.sha256()
    payload_hash = hashlib.sha256()
    chunks = []
    offset = 0
    with open(root_path, "rb") as root, open(slot_path, "wb") as slot:
        while True:
            raw = root.read(a.chunk)
            if not raw:
                break
            root_hash.update(raw)
            packed = subprocess.run(
                ["zstd", "-q", "-10", "-c"], input=raw, capture_output=True, check=True
            ).stdout
            slot.write(packed)
            payload_hash.update(packed)
            chunks.append(
                {
                    "offset": offset,
                    "size": len(packed),
                    "sha256": hashlib.sha256(packed).hexdigest(),
                    "raw_sha256": hashlib.sha256(raw).hexdigest(),
                }
            )
            offset += len(packed)
        fits = {}
        for slot_name, name in (("a", "falcon.itb"), ("b", "falcon-b.itb")):
            data = open(os.path.join(a.work, name), "rb").read()
            slot.write(data)
            payload_hash.update(data)
            fits[slot_name] = {
                "offset": offset,
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            offset += len(data)

    manifest = {
        "format": 1,
        "version": version,
        "build": rel.get("NIBOS_BUILD", ""),
        "board": rel["NIBOS_BOARD"],
        "edition": rel["NIBOS_EDITION"],
        "channel": a.channel,
        "min_from": a.min_from,
        "notes_url": a.notes_url,
        "notes": open(a.notes).read().strip() if a.notes else "",
        "components": {"nibos": rel.get("NIBOS_BUILD", "")},
        "payload": {
            "file": base + ".slot",
            "size": offset,
            "sha256": payload_hash.hexdigest(),
        },
        "root": {
            "size": root_size,
            "sha256": root_hash.hexdigest(),
            "chunk": a.chunk,
            "chunks": chunks,
        },
        "fit": fits,
        "migrate": 1,
    }
    path = os.path.join(a.out, base + ".manifest.json")
    with open(path, "w") as f:
        json.dump(manifest, f, indent=1)
        f.write("\n")
    print(f"{path}: {len(chunks)} chunks, {offset / 1e6:.0f} MB")


if __name__ == "__main__":
    sys.exit(main())
