#!/usr/bin/env python3
"""Pack tools/collect-sources.py's output into release assets, each under a size limit.

    tools/pack-sources.py SOURCES NAME [--max-gb 1.9] [--notes SOURCES.md]

Writes SOURCES/NAME-partN.tar.zst: archives from SOURCES/files/, grouped largest first into as
few parts as fit, each part with manifest.tsv (every package, the archive its source is in,
its SHA-256 and the part it is in) and the notes. The archives are compressed already, so the
parts are stored with light zstd. Also SOURCES/NAME-manifest.tsv, the same manifest on its own.
"""

import argparse
import csv
import os
import shutil
import subprocess


def main():
    p = argparse.ArgumentParser()
    p.add_argument("sources")
    p.add_argument("name")
    p.add_argument("--max-gb", type=float, default=1.9)
    p.add_argument("--notes")
    a = p.parse_args()
    files = os.path.join(a.sources, "files")
    limit = int(a.max_gb * 1e9) - 50_000_000  # room for the tar headers, manifest and notes
    sizes = {f: os.path.getsize(os.path.join(files, f)) for f in os.listdir(files)}
    big = [f for f, s in sizes.items() if s > limit]
    if big:
        raise SystemExit(f"pack-sources: larger than a part on their own: {big}")
    parts = []  # [total, [names]]
    for f in sorted(sizes, key=sizes.get, reverse=True):
        for part in parts:
            if part[0] + sizes[f] <= limit:
                part[0] += sizes[f]
                part[1].append(f)
                break
        else:
            parts.append([sizes[f], [f]])
    part_of = {f: i + 1 for i, (_, names) in enumerate(parts) for f in names}

    with open(os.path.join(a.sources, "manifest.tsv")) as f:
        rows = list(csv.reader(f, delimiter="\t"))
    header, body = rows[0], rows[1:]
    manifest = os.path.join(a.sources, f"{a.name}-manifest.tsv")
    with open(manifest, "w", newline="") as f:
        w = csv.writer(f, delimiter="\t", lineterminator="\n")
        w.writerow(header + ["part"])
        for r in body:
            w.writerow(r + [f"{a.name}-part{part_of[r[4]]}.tar.zst"])

    for i, (_, names) in enumerate(parts, 1):
        stage = os.path.join(a.sources, f"stage-{i}")
        shutil.rmtree(stage, ignore_errors=True)
        os.makedirs(os.path.join(stage, a.name, "files"))
        for n in names:
            os.link(os.path.join(files, n), os.path.join(stage, a.name, "files", n))
        shutil.copy(manifest, os.path.join(stage, a.name, "manifest.tsv"))
        if a.notes:
            shutil.copy(a.notes, os.path.join(stage, a.name, "SOURCES.md"))
        out = os.path.join(a.sources, f"{a.name}-part{i}.tar.zst")
        tar = subprocess.Popen(["tar", "-C", stage, "-cf", "-", a.name], stdout=subprocess.PIPE)
        with open(out, "wb") as f:
            subprocess.run(["zstd", "-q", "-1", "-T0"], stdin=tar.stdout, stdout=f, check=True)
        if tar.wait():
            raise SystemExit(f"pack-sources: tar of part {i}")
        shutil.rmtree(stage)
        print(f"{os.path.basename(out)}: {len(names)} archives, {os.path.getsize(out) / 1e9:.2f} GB")


if __name__ == "__main__":
    main()
