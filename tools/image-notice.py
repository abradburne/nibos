#!/usr/bin/env python3
"""The package part of a downloadable image's NOTICE, from the image's package list.

    tools/image-notice.py NOTICE PACKAGES_TXT       (appends to NOTICE)

PACKAGES_TXT is `pacman -Qi`'s name, version and licences, one package a line (rootfs/make-arch.sh).
Appended: the packages whose licence isn't a plain free software one (custom, LicenseRef-...,
non-commercial), each named, with what it means for whoever passes the image on where we know
(Snes9x, FDK-AAC); then every package with its version and licence.
"""
import re
import sys

# What a licence means for redistributing the image, where it needs saying.
NOTES = {
    "snes9x": "Snes9x is licensed for non-commercial use only: this image, or anything "
              "built from it that includes Snes9x, may not be sold or bundled with a product "
              "that is.",
    "libfdk-aac": "The Fraunhofer FDK AAC library (Bluetooth AAC audio) is redistributed under "
                  "Fraunhofer's licence, which grants no patent rights: AAC patent licences "
                  "are the user's affair where they apply.",
}
# Licence words that need a look: not one of the usual free software licences.
RESTRICTED = re.compile(r"custom|LicenseRef|non-?commercial|proprietary|unknown", re.I)


def main(notice, packages_txt):
    rows = []
    for line in open(packages_txt):
        parts = line.split(None, 2)
        if len(parts) >= 2:
            rows.append((parts[0], parts[1], parts[2].strip() if len(parts) > 2 else "unknown"))
    restricted = [r for r in rows if RESTRICTED.search(r[2])]
    out = ["", "Licences to know about",
           "Packages under a custom licence: most are free software licences without a standard",
           "name, a few restrict how the image may be passed on (noted). The full texts are in",
           "/usr/share/licenses/<package>/ on the device:"]
    for name, version, lic in restricted:
        out.append(f"- {name} {version}: {lic}")
        note = next((n for key, n in NOTES.items() if name == key or name.startswith(key + "-")
                     or (key == "snes9x" and "snes9x" in name)), None)
        if note:
            out.append(f"  {note}")
    if not restricted:
        out.append("- none")
    out += ["", f"Every package in the image ({len(rows)}), with its version and licence"]
    out += [f"- {n} {v}: {lic}" for n, v, lic in rows]
    with open(notice, "a") as f:
        f.write("\n".join(out) + "\n")
    print(f"{notice}: {len(rows)} packages, {len(restricted)} with licences to know about")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
