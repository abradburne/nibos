#!/usr/bin/env python3
"""Every crate a GPL-3.0-or-later part of nibOS builds in has a licence compatible with it.

    tools/check-licences.py CRATE_DIR...     (in the build machine: cargo, the crates fetched)

Reads `cargo metadata` over each crate's whole resolved graph (Cargo.lock) and checks every
package's SPDX expression: an alternative (OR) is fine when all its parts (AND) are licences
GPL-3.0 can include. OFL-1.1 is allowed as font data built into a program (nibkit's fonts), not as
code. Prints the licences found and their counts; exits 1 naming any package that fails, or that
says no licence at all.
"""
import json
import re
import subprocess
import sys
from collections import Counter

# Licences GPL-3.0 can include (the FSF's list of GPL-compatible licences; Apache-2.0 is
# compatible with v3, not v2).
COMPATIBLE = {
    "MIT", "MIT-0", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD",
    "Unlicense", "CC0-1.0", "BSL-1.0", "MPL-2.0", "Unicode-3.0", "Unicode-DFS-2016",
    "LGPL-2.1-or-later", "LGPL-3.0-or-later", "LGPL-3.0-only", "GPL-2.0-or-later",
    "GPL-3.0-only", "GPL-3.0-or-later",
    "OFL-1.1",  # font data only (nibkit's fonts)
}


def ok(expression):
    expr = expression.replace("/", " OR ")  # the old cargo form, "MIT/Apache-2.0"
    for alternative in re.split(r"\s+OR\s+", expr.replace("(", " ").replace(")", " ")):
        parts = [re.sub(r"\s+WITH\s+.*", "", p).strip() for p in re.split(r"\s+AND\s+", alternative)]
        if parts and all(p in COMPATIBLE for p in parts if p):
            return True
    return False


def main(dirs):
    seen, bad = Counter(), []
    for d in dirs:
        meta = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--locked"], cwd=d))
        for p in meta["packages"]:
            lic = p.get("license") or ""
            if p.get("license_file") and not lic:
                lic = f"(file: {p['license_file']})"
            seen[lic or "(none)"] += 1
            if not lic or not ok(lic):
                bad.append(f"{d}: {p['name']} {p['version']}: {lic or 'no licence'}")
    for lic, n in sorted(seen.items(), key=lambda x: -x[1]):
        print(f"{n:4}  {lic}")
    if bad:
        print("not compatible with GPL-3.0-or-later, or no licence:", *bad, sep="\n  ")
        return 1
    print(f"licences: {sum(seen.values())} packages in {len(dirs)} graphs, all GPL-3.0 compatible")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
