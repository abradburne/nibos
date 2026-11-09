#!/usr/bin/env python3
"""What's new in a release, as the plain text the menus show (nibkit's `guide`), from CHANGELOG.md.

    tools/whats-new.py CHANGELOG.md VERSION > whats-new.txt

tools/make-image.sh writes it into every image as /usr/share/nibos/whats-new.txt (through
rootfs/make-arch.sh's DOCS), so a release's notes are its changelog's section and nothing is
copied by hand. VERSION is the build's version, with or without its "v": a release's
(0.1.0-beta3) takes the "## 0.1.0-beta3" section; a build after a tag (git describe's
0.1.0-beta3-4-gabc1234) takes that tag's section, and says it is a later build. With no such
section it writes nothing and exits 1.

The first line is the title ("What's new in nibOS 0.1.0-beta3"); then the section, with
Markdown's marks taken out: **bold** and `code` become plain words, [links](...) their words,
and the list items stay "- " items with their lines indented under them, as nibkit's guide
reads them.
"""
import re
import sys


def sections(text):
    """The changelog's "## VERSION" sections, in order: (version, lines)."""
    out = []
    for line in text.splitlines():
        m = re.match(r"^## (\S+)\s*$", line)
        if m:
            out.append((m.group(1), []))
        elif out:
            out[-1][1].append(line)
    return out


def plain(line):
    """A line of Markdown as plain words."""
    line = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", line)
    line = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", line)
    line = re.sub(r"\*\*(.+?)\*\*", r"\1", line)
    line = re.sub(r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])", r"\1", line)
    line = re.sub(r"`([^`]+)`", r"\1", line)
    return line


def find(all_sections, version):
    """The section for `version`, and whether the build is a later one than its tag."""
    for v, lines in all_sections:
        if v == version:
            return v, lines, False
    # git describe: TAG-N-gHASH (and -dirty).
    m = re.match(r"^(.+?)-\d+-g[0-9a-f]+(-dirty)?$", version)
    if m:
        for v, lines in all_sections:
            if v == m.group(1):
                return v, lines, True
    return None, None, False


def render(all_sections, version):
    """The file's text, or None if the changelog has no section for `version`."""
    version = version[1:] if version.startswith("v") else version
    v, lines, later = find(all_sections, version)
    if v is None:
        return None
    body = [plain(l.rstrip()) for l in lines]
    while body and not body[0]:
        body.pop(0)
    while body and not body[-1]:
        body.pop()
    title = f"What's new in nibOS {v}"
    if later:
        body = [f"This is a later build, {version}: these are {v}'s notes.", ""] + body
    return "\n".join([title, ""] + body) + "\n"


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as f:
        text = render(sections(f.read()), argv[2])
    if text is None:
        print(f"whats-new: {argv[1]} has no section for {argv[2]}", file=sys.stderr)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
