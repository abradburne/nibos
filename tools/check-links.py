#!/usr/bin/env python3
"""Check the relative links in the repository's Markdown files.

  tools/check-links.py [FILE.md ...]      (default: every tracked .md file)

A link to a file or directory must point at something in the repository, and a
link with a #fragment into a Markdown file must name one of its headings, in
GitHub's form (lower case, punctuation dropped, spaces as hyphens). Links to
web pages are not fetched. Exits 1 and lists the broken links if there are any.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
CODE = re.compile(r"```.*?```|`[^`\n]*`", re.S)


def anchors(md: Path) -> set:
    """The fragments GitHub makes for a Markdown file's headings."""
    seen, out = {}, set()
    for line in CODE.sub("", md.read_text()).splitlines():
        m = re.match(r"#{1,6}\s+(.*?)\s*#*\s*$", line)
        if not m:
            continue
        slug = re.sub(r"[^\w\- ]", "", m.group(1).lower().replace("`", "")).replace(" ", "-")
        n = seen.get(slug, 0)
        seen[slug] = n + 1
        out.add(slug if n == 0 else f"{slug}-{n}")
    return out


def broken(md: Path):
    text = CODE.sub("", md.read_text())
    for target in LINK.findall(text):
        if re.match(r"[a-z]+:", target):
            continue
        path, _, fragment = target.partition("#")
        dest = (md.parent / path).resolve() if path else md
        if not dest.exists():
            yield target, "no such file"
        elif fragment and dest.suffix == ".md" and fragment not in anchors(dest):
            yield target, "no such heading"


def main():
    if sys.argv[1:]:
        files = [Path(f).resolve() for f in sys.argv[1:]]
    else:
        listed = subprocess.run(["git", "ls-files", "*.md"], cwd=ROOT, check=True,
                                capture_output=True, text=True).stdout.split()
        files = [ROOT / f for f in listed if (ROOT / f).exists()]
    bad = 0
    for md in files:
        for target, why in broken(md):
            print(f"{md.relative_to(ROOT)}: {target}: {why}")
            bad += 1
    if bad:
        sys.exit(1)
    print(f"links: {len(files)} files checked")


if __name__ == "__main__":
    main()
