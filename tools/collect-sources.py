#!/usr/bin/env python3
"""The complete corresponding source for a release's images, for the GPL source offer that
every image's NOTICE makes (docs/design/images.md): runs in the build machine, as the
building user (reading an image's package database uses sudo).

    tools/collect-sources.py OUT WORK...

WORK: tools/make-image.sh's work directories (root.img, docs/release), one per edition. For
each package in their roots whose licence asks for source (GPL, LGPL, MPL and the like):

- Arch Linux ARM's packages: the PKGBUILD tree at the version shipped (Arch Linux ARM's own,
  archlinuxarm/PKGBUILDs, for the packages it changes; otherwise Arch's packaging repository at
  the version's tag), with every source it names: upstream tarballs (checked against the
  PKGBUILD's sums), patches, and each VCS source as a snapshot at the commit or tag it names.
- nibOS's own: this repository at the images' commit (NIBOS_BUILD), the kernel and U-Boot trees at the pinned commit and
  tag, the projects packaged from their checkouts at the commits built (DSlot, DSperate,
  nibcase, inkdot, inktick, nibkit), the sources their PKGBUILDs name, and every Rust crate
  the Rust programs link (cargo vendor).

Writes OUT/files/ (one archive a source), OUT/manifest.tsv (each image package with the
archive its source is in, and the archives' SHA-256) and OUT/failed.txt (anything not
collected). tools/pack-sources.py packs OUT into release assets.
"""

import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
NIBOS = os.path.dirname(HERE)
PARENT = os.path.dirname(NIBOS)
ALARM_REPO = "https://github.com/archlinuxarm/PKGBUILDs.git"
ARCH_GITLAB = "https://gitlab.archlinux.org/archlinux/packaging/packages/{}.git"
# Licences that oblige whoever passes the binaries on to offer the source.
OBLIGES = re.compile(r"GPL|MPL|EPL|CDDL|EUPL|CeCILL|OSL|APSL|Sleepycat", re.I)
# Packages built by nibOS's image (packager "Unknown Packager"): the source each comes from.
OWN_PACKAGES = {
    "nibos-hw": "nibos",
    "nibos-no-fdk-aac": "nibos",
    "portmaster": "portmaster",
    "mesa-nibos": "mesa-nibos",
    "dslot": "dslot",
    "dsperate": "dsperate",
    "inkdot": "inkdot",
    "inktick": "inktick",
    "nibcase": "nibcase",
    "libretro-beetle-ngp": "core:beetle-ngp",
    "libretro-beetle-wswan": "core:beetle-wswan",
    "libretro-gearsystem": "core:gearsystem",
    "libretro-gw": "core:gw",
    "libretro-handy": "core:handy",
    "libretro-pcsx-rearmed-nibos": "core:pcsx-rearmed",
}


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, text=True, **kw)


def out_of(cmd, **kw):
    return subprocess.run(cmd, check=True, text=True, capture_output=True, **kw).stdout


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def packages(work, tmp):
    """Each package in a work directory's root: name, version, pkgbase, licence, packager."""
    db = os.path.join(tmp, os.path.basename(work.rstrip("/")))
    os.makedirs(db, exist_ok=True)
    run(["sudo", "debugfs", "-R", f"rdump /var/lib/pacman/local {db}",
         os.path.join(work, "root.img")], capture_output=True)
    run(["sudo", "chown", "-R", str(os.getuid()), db])
    rows = []
    local = os.path.join(db, "local")
    for d in sorted(os.listdir(local)):
        desc = os.path.join(local, d, "desc")
        if not os.path.exists(desc):
            continue
        f, key = {}, None
        for line in open(desc):
            line = line.rstrip("\n")
            if line.startswith("%") and line.endswith("%"):
                key = line.strip("%")
                f[key] = []
            elif line and key:
                f[key].append(line)
        rows.append({
            "name": f["NAME"][0],
            "version": f["VERSION"][0],
            "base": (f.get("BASE") or f["NAME"])[0],
            "license": " AND ".join(f.get("LICENSE", ["unknown"])),
            "packager": (f.get("PACKAGER") or ["?"])[0],
        })
    return rows


def split_version(v):
    epoch, _, rest = v.rpartition(":")
    pkgver, _, pkgrel = rest.rpartition("-")
    return epoch, pkgver, pkgrel


def pkgbuild_vars(text):
    """A PKGBUILD's epoch, pkgver and pkgrel, as bash sees them."""
    with tempfile.NamedTemporaryFile("w", suffix=".PKGBUILD") as f:
        f.write(text)
        f.flush()
        r = subprocess.run(["bash", "-c", 'source "$1" >/dev/null 2>&1; '
                            'printf "%s\\n%s\\n%s\\n" "$epoch" "$pkgver" "$pkgrel"', "-", f.name],
                           capture_output=True, text=True)
    return tuple((r.stdout.split("\n") + ["", "", ""])[:3])


def alarm_tree(alarm, base, version, dest):
    """Arch Linux ARM's own PKGBUILD tree for `base` at `version`: how it was found; None if
    Arch Linux ARM doesn't change the package; False if it does but has no tree at `version`."""
    for repo in ("core", "extra", "alarm", "aur"):
        path = f"{repo}/{base}"
        if not os.path.isdir(os.path.join(alarm, path)):
            continue
        want = split_version(version)
        for commit in out_of(["git", "-C", alarm, "log", "--format=%H", "--", path]).split():
            try:
                text = out_of(["git", "-C", alarm, "show", f"{commit}:{path}/PKGBUILD"])
            except subprocess.CalledProcessError:
                continue
            if pkgbuild_vars(text) == want:
                tar = subprocess.Popen(["git", "-C", alarm, "archive", commit, path],
                                       stdout=subprocess.PIPE)
                run(["tar", "-x", "-C", dest, f"--strip-components=2"], stdin=tar.stdout)
                tar.wait()
                return f"archlinuxarm/PKGBUILDs {commit[:12]} {path}"
        return False
    return None


def arch_tree(base, version, dest):
    """Arch's packaging repository at the version's tag."""
    tag = version.replace(":", "-")
    names = [base, base.replace("++", "plusplus"), base.replace("+", "plus")]
    for name in dict.fromkeys(names):
        url = ARCH_GITLAB.format(name)
        r = subprocess.run(["git", "clone", "-q", "--depth", "1", "--branch", tag, url, dest],
                           capture_output=True, text=True)
        if r.returncode == 0:
            shutil.rmtree(os.path.join(dest, ".git"), ignore_errors=True)
            return f"Arch packaging {name} tag {tag}"
        shutil.rmtree(dest, ignore_errors=True)
    return None


def source_list(tree, env=None):
    """A PKGBUILD's sources for aarch64 (source, source_aarch64), each with its SHA-256 or
    BLAKE2b sum where the PKGBUILD gives one: [(entry, kind, sum)]."""
    script = r"""
CARCH=aarch64; source PKGBUILD >/dev/null 2>&1
emit() { local -n src=$1 sha=$2 b2=$3; local i
    for i in "${!src[@]}"; do
        if [ -n "${sha[i]:-}" ] && [ "${sha[i]}" != SKIP ]; then printf '%s\tsha256\t%s\n' "${src[i]}" "${sha[i]}"
        elif [ -n "${b2[i]:-}" ] && [ "${b2[i]}" != SKIP ]; then printf '%s\tb2\t%s\n' "${src[i]}" "${b2[i]}"
        else printf '%s\t\t\n' "${src[i]}"; fi
    done; }
emit source sha256sums b2sums
emit source_aarch64 sha256sums_aarch64 b2sums_aarch64
"""
    out = out_of(["bash", "-c", script], cwd=tree, env=dict(os.environ, **(env or {})))
    return [tuple((line.split("\t") + ["", ""])[:3]) for line in out.splitlines() if line]


def git_snapshot(url, fragment, dest_tar, prefix):
    """A git source at its commit, tag or branch, as a plain tarball (not the whole history)."""
    kind, _, ref = fragment.partition("=")
    ref = ref.split("?")[0]
    with tempfile.TemporaryDirectory() as d:
        run(["git", "init", "-q", d])
        want = {"commit": ref, "tag": f"refs/tags/{ref}", "branch": f"refs/heads/{ref}"}.get(kind, "HEAD")
        r = subprocess.run(["git", "-C", d, "fetch", "-q", "--depth", "1", url, want],
                           capture_output=True, text=True)
        if r.returncode:
            # A server that won't serve a commit by its hash: a clone without file contents,
            # then the files for this commit alone.
            shutil.rmtree(d)
            run(["git", "clone", "-q", "--bare", "--filter=blob:none", url, d], capture_output=True)
            rev = ref if kind == "commit" else (f"refs/tags/{ref}" if kind == "tag" else
                                                 (f"refs/heads/{ref}" if kind == "branch" else "HEAD"))
            run(["git", "-C", d, "archive", "--format=tar", f"--prefix={prefix}/", "-o", dest_tar, rev])
        else:
            run(["git", "-C", d, "archive", "--format=tar", f"--prefix={prefix}/", "-o", dest_tar,
                 "FETCH_HEAD"])


def collect_tree(tree, files, srcdest, name, env=None):
    """A PKGBUILD tree with every source it names for aarch64, as NAME.src.tar.zst: the tree's
    own files (PKGBUILD, patches), each downloaded source (checked against its sum), and each
    VCS source as a snapshot at the commit or tag it names (NAME.vcs.tar)."""
    path = os.path.join(files, f"{name}.src.tar.zst")
    if os.path.exists(path):
        return path
    stage = os.path.join(srcdest, "stage", name)
    shutil.rmtree(stage, ignore_errors=True)
    shutil.copytree(tree, stage, ignore=shutil.ignore_patterns(".git", "src", "pkg"))
    for entry, kind, want in source_list(tree, env):
        fname, _, url = entry.rpartition("::")
        if "://" not in url:
            continue  # a file in the tree, copied already
        if url.startswith("git+") or url.startswith("git://"):
            base, _, fragment = url.removeprefix("git+").partition("#")
            base = base.split("?")[0]  # makepkg's ?signed: a request to check tags, not the URL
            label = fname or os.path.basename(base.rstrip("/")).removesuffix(".git")
            git_snapshot(base, fragment, os.path.join(stage, f"{label}.vcs.tar"), label)
            continue
        if re.match(r"(svn|hg|bzr|fossil)\+", url):
            raise RuntimeError(f"a VCS source this doesn't fetch: {entry}")
        fname = fname or os.path.basename(url.split("#")[0].split("?")[0])
        cached = os.path.join(srcdest, fname)
        if not os.path.exists(cached):
            run(["curl", "-fsSL", "--retry", "3", "-o", cached + ".part", url], capture_output=True)
            os.replace(cached + ".part", cached)
        if kind:
            algo = "sha256" if kind == "sha256" else "blake2b"
            h = hashlib.new(algo)
            with open(cached, "rb") as f:
                for block in iter(lambda: f.read(1 << 20), b""):
                    h.update(block)
            if h.hexdigest() != want:
                os.remove(cached)
                raise RuntimeError(f"{fname}: {kind} sum doesn't match the PKGBUILD's")
        shutil.copy(cached, os.path.join(stage, fname))
    tar = subprocess.Popen(["tar", "-C", os.path.dirname(stage), "-cf", "-", name],
                           stdout=subprocess.PIPE)
    with open(path + ".part", "wb") as f:
        run(["zstd", "-q", "-3", "-T0"], stdin=tar.stdout, stdout=f)
    if tar.wait():
        raise RuntimeError(f"tar of {name}")
    os.replace(path + ".part", path)
    shutil.rmtree(stage)
    return path


def git_archive(repo, commit, name, files):
    """A tree at a commit as NAME-COMMIT.tar.zst."""
    short = out_of(["git", "-C", repo, "rev-parse", "--short=12", commit]).strip()
    path = os.path.join(files, f"{name}-{short}.tar.zst")
    if not os.path.exists(path):
        tar = subprocess.Popen(["git", "-C", repo, "archive", "--format=tar",
                                f"--prefix={name}-{short}/", commit], stdout=subprocess.PIPE)
        with open(path, "wb") as f:
            run(["zstd", "-q", "-19", "-T0"], stdin=tar.stdout, stdout=f)
        if tar.wait():
            raise RuntimeError(f"git archive {repo} {commit}")
    return path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("out")
    p.add_argument("work", nargs="+")
    p.add_argument("--only", help="comma-separated pkgbases (for a retry)")
    a = p.parse_args()
    files = os.path.join(a.out, "files")
    srcdest = os.path.join(a.out, "srcdest")
    os.makedirs(files, exist_ok=True)
    os.makedirs(srcdest, exist_ok=True)
    tmp = tempfile.mkdtemp(prefix="nibos-sources-")

    rows = {}
    editions = {}
    builds = set()
    # The commit each project's package was built from, as the image's NOTICE records it
    # ("- DSlot (the boot UI) at 84d2f35d15f4: ..."); a checkout may have moved on since.
    built_at = {}
    for w in a.work:
        notice = os.path.join(w, "docs/NOTICE")
        if os.path.exists(notice):
            for name, commit in re.findall(r"^- (\w+)[^\n:]* at ([0-9a-f]{7,40}):",
                                           open(notice).read(), re.M):
                built_at[name.lower()] = commit
        release = open(os.path.join(w, "docs/release")).read()
        image = re.search(r"NIBOS_IMAGE=(\S+)", release)[1]
        builds.add(re.search(r"NIBOS_BUILD=(\S+)", release)[1])
        for r in packages(w, tmp):
            rows.setdefault(r["name"], r)
            editions.setdefault(r["name"], set()).add(image)
    oblig = {n: r for n, r in rows.items() if OBLIGES.search(r["license"])}
    own = {n: r for n, r in rows.items() if r["packager"] == "Unknown Packager"}
    # nibcase's per-core packages (nibcase-<core>) are its split packages: a README and the
    # dependency on the core's libretro package, whose source is collected on its own.
    for n in own:
        if n.startswith("nibcase-"):
            OWN_PACKAGES[n] = "nibcase"
    unknown = sorted(set(own) - set(OWN_PACKAGES))
    if unknown:
        sys.exit(f"collect-sources: no source known for nibOS's packages {unknown}")

    archive_of = {}  # package name -> (archive path, how)
    failed = []

    # Arch Linux ARM's packages, by pkgbase.
    alarm = os.path.join(a.out, "PKGBUILDs")
    if not os.path.isdir(alarm):
        run(["git", "clone", "-q", ALARM_REPO, alarm])
    bases = {}
    for n, r in oblig.items():
        if n not in own:
            bases.setdefault((r["base"], r["version"]), []).append(n)
    only = set(a.only.split(",")) if a.only else None
    for (base, version), names in sorted(bases.items()):
        if only and base not in only:
            continue
        done = os.path.join(files, f"{base}-{version.replace(':', '-')}.src.tar.zst")
        if os.path.exists(done):  # from an earlier run into the same OUT
            for n in names:
                archive_of[n] = (done, "collected by an earlier run")
            continue
        tree = os.path.join(tmp, "trees", f"{base}-{version}")
        os.makedirs(tree, exist_ok=True)
        how = alarm_tree(alarm, base, version, tree)
        if how is False:
            failed.append(f"{base} {version}: Arch Linux ARM's tree has no commit at this version")
            continue
        how = how or arch_tree(base, version, tree)
        if not how:
            failed.append(f"{base} {version}: no PKGBUILD tree at this version")
            continue
        try:
            path = collect_tree(tree, files, srcdest, f"{base}-{version.replace(':', '-')}")
        except (subprocess.CalledProcessError, RuntimeError) as e:
            err = getattr(e, "stderr", "") or str(e)
            failed.append(f"{base} {version} ({how}): {str(err).strip()[-300:]}")
            continue
        print(f"{base} {version}: {os.path.basename(path)} ({how})", flush=True)
        for n in names:
            archive_of[n] = (path, how)

    if only:  # a retry of some of Arch Linux ARM's packages: those alone
        for f in failed:
            print("failed:", f)
        return

    # nibOS's own.
    def checkout(env, default):
        return os.environ.get(env) or os.path.join(PARENT, default)

    def project(env, default, name):
        return git_archive(checkout(env, default), built_at.get(name, "HEAD"), name, files)

    if len(builds) != 1:
        sys.exit(f"collect-sources: the images are from different nibOS commits: {builds}")
    build = builds.pop()
    nibos = git_archive(NIBOS, build, "nibos", files)
    sources = {
        "nibos": (nibos, f"nibOS at {build} (the images' build): nibos-hw, nibos-no-fdk-aac, "
                         "the kernel's and U-Boot's patches and configuration, the image"),
        "dslot": (project("DSLOT_SRC", "dslot", "dslot"), "DSlot"),
        "dsperate": (project("DSPERATE_SRC", "DSperate", "dsperate"), "DSperate"),
        "inkdot": (project("INKDOT_SRC", "inkdot", "inkdot"), "inkdot"),
        "inktick": (project("INKTICK_SRC", "inktick", "inktick"), "inktick"),
        "nibcase": (project("NIBCASE_SRC", "nibcase", "nibcase"), "nibcase"),
    }
    nibcase = checkout("NIBCASE_SRC", "nibcase")
    for name, (key) in OWN_PACKAGES.items():
        if name not in own:
            continue
        if key.startswith("core:"):
            core = key[5:]
            try:
                path = collect_tree(os.path.join(nibcase, "packaging/cores", core), files, srcdest,
                                    f"{name}-{own[name]['version']}")
                archive_of[name] = (path, f"nibcase packaging/cores/{core}")
            except (subprocess.CalledProcessError, RuntimeError) as e:
                failed.append(f"{name}: {getattr(e, 'stderr', '') or e}")
        elif key == "mesa-nibos":
            path = collect_tree(os.path.join(NIBOS, "rootfs/mesa"), files, srcdest,
                                f"mesa-nibos-{own[name]['version'].replace(':', '-')}")
            archive_of[name] = (path, "nibOS rootfs/mesa")
        elif key == "portmaster":
            path = collect_tree(os.path.join(NIBOS, "rootfs/portmaster"), files, srcdest,
                                f"portmaster-{own[name]['version']}", {"NIBOS_ROOT": NIBOS})
            archive_of[name] = (path, "nibOS rootfs/portmaster")
        else:
            archive_of[name] = sources[key]

    # The kernel and U-Boot (not packages: the image installs them).
    env = out_of(["bash", "-c", f". '{NIBOS}/config.sh'; echo $KSRC $KERNEL_COMMIT $USRC $UBOOT_TAG"],
                 env=dict(os.environ, NIBOS_ROOT=NIBOS)).split()
    ksrc, kcommit, usrc, utag = env
    extra = [
        ("linux", kcommit, git_archive(ksrc, kcommit, "linux", files),
         "GPL-2.0-only", "mainline Linux at KERNEL_COMMIT; nibOS's patches and config are in the nibos archive"),
        ("u-boot", utag, git_archive(usrc, utag, "u-boot", files),
         "GPL-2.0-or-later", "mainline U-Boot at UBOOT_TAG; nibOS's patches are in the nibos archive"),
    ]

    # Every crate the Rust programs link, vendored.
    crates = [os.path.join(NIBOS, c, "Cargo.toml") for c in
              ("hw/daemons", "rootfs/splash", "rootfs/safe-mode", "rootfs/portmaster/get")]
    crates.append(os.path.join(checkout("DSLOT_SRC", "dslot"), "Cargo.toml"))
    vendor = os.path.join(tmp, "rust-vendor")
    cmd = ["cargo", "vendor", "--locked", "--versioned-dirs", "--manifest-path", crates[0]]
    for c in crates[1:]:
        cmd += ["--sync", c]
    run(cmd + [vendor], capture_output=True)
    rust = os.path.join(files, "rust-vendor.tar.zst")
    tar = subprocess.Popen(["tar", "-C", tmp, "-cf", "-", "rust-vendor"], stdout=subprocess.PIPE)
    with open(rust, "wb") as f:
        run(["zstd", "-q", "-19", "-T0"], stdin=tar.stdout, stdout=f)
    tar.wait()

    # The manifest.
    with open(os.path.join(a.out, "manifest.tsv"), "w") as m:
        m.write("package\tversion\tlicence\timages\tarchive\tsha256\tfrom\n")
        hashes = {}
        for n in sorted(rows):
            if n not in oblig and n not in own:
                continue
            if n not in archive_of:
                continue
            path, how = archive_of[n]
            h = hashes.setdefault(path, sha256(path))
            r = rows[n]
            m.write("\t".join([n, r["version"], r["license"], ",".join(sorted(editions[n])),
                               os.path.basename(path), h, how]) + "\n")
        for name, version, path, lic, how in extra:
            m.write("\t".join([name, version, lic, "all", os.path.basename(path),
                               sha256(path), how]) + "\n")
        m.write("\t".join(["(Rust crates)", "-", "various", "all", "rust-vendor.tar.zst",
                           sha256(rust), "cargo vendor of nibos-hw, the splash, the safe mode, "
                           "portmaster-get and DSlot"]) + "\n")
    with open(os.path.join(a.out, "failed.txt"), "w") as f:
        f.write("\n".join(failed) + ("\n" if failed else ""))
    print(f"collected: {len(set(p for p, _ in archive_of.values()))} archives; "
          f"failed: {len(failed)} (failed.txt)")
    shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
