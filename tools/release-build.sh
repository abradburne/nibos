#!/bin/bash
# Build everything a release needs that needs no key, the same way every time
# (docs/releasing.md): nibos-hw, both images, their tests, and the corresponding source.
#
#   tools/release-build.sh VERSION          (Mac or a Linux host; the work runs in the VM)
#
# VERSION is the release's, without the v (0.1.0-alpha4). Before it runs: this tree is clean and
# at the annotated tag vVERSION, CHANGELOG.md has a "## VERSION" section, and the projects the
# DSlot edition is built from are clean checkouts beside this repository, the PUBLIC ones for
# DSlot, nibcase and inktick: DSLOT_SRC (../dslot-public), NIBCASE_SRC (../nibcase-public),
# INKTICK_SRC (../inktick-public), DSPERATE_SRC (../DSperate), INKDOT_SRC (../inkdot). The
# environment overrides each.
#
# In order, stopping at the first failure, everything logged to out/release-build-VERSION.log:
#   1. nibos-hw rebuilt in the VM (hw/out), so a change under hw/ ships.
#   2. The two images, VERSION=... SKIP_BUILD=1 tools/make-image.sh and --dslot.
#   3. tools/test-image.sh on both, then tools/test-update.sh on each: one after another, never
#      alongside anything else in the VM (they share its loop devices).
#   4. tools/collect-sources.py from out/sources-cache into out/sources-VERSION, then
#      tools/pack-sources.py. out/sources-VERSION/SOURCES.md must be there first: the previous
#      release's with the version changed, read through.
# Then it prints the steps that need a person: signing, publishing, the OTA test, the site.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"

version=${1:-}
[ -n "$version" ] && [ "${version#v}" = "$version" ] || usage
tag=v$version
out=$NIBOS_ROOT/out
parent=$(cd "$NIBOS_ROOT/.." && pwd)
: "${DSLOT_SRC:=$parent/dslot-public}" "${NIBCASE_SRC:=$parent/nibcase-public}"
: "${INKTICK_SRC:=$parent/inktick-public}" "${DSPERATE_SRC:=$parent/DSperate}" "${INKDOT_SRC:=$parent/inkdot}"
export DSLOT_SRC NIBCASE_SRC INKTICK_SRC DSPERATE_SRC INKDOT_SRC

# --- what must be true first --------------------------------------------------------------
[ -z "$(git -C "$NIBOS_ROOT" status --porcelain --untracked-files=no)" ] || die "this tree has uncommitted changes"
git -C "$NIBOS_ROOT" rev-parse -q --verify "refs/tags/$tag" >/dev/null || die "no tag $tag (git tag -a $tag -m ...)"
[ "$(git -C "$NIBOS_ROOT" rev-parse "$tag^{commit}")" = "$(git -C "$NIBOS_ROOT" rev-parse HEAD)" ] || die "$tag is not at HEAD"
[ "$(git -C "$NIBOS_ROOT" cat-file -t "$tag")" = tag ] || die "$tag is not an annotated tag"
grep -q "^## $version\$" "$NIBOS_ROOT/CHANGELOG.md" || die "CHANGELOG.md has no \"## $version\" section"
for v in DSLOT_SRC NIBCASE_SRC INKTICK_SRC DSPERATE_SRC INKDOT_SRC; do
    d=${!v}
    [ -d "$d/.git" ] || die "$v: $d is not a git checkout"
    [ -z "$(git -C "$d" status --porcelain --untracked-files=no)" ] || die "$v: $d has uncommitted changes"
done
[ -f "$out/sources-$version/SOURCES.md" ] || die "write out/sources-$version/SOURCES.md first (the previous release's, with the version changed)"
for f in "$out/release-$version" "$out/nibos-$version-$BOARD_HOSTNAME.img.xz" "$out/nibos-$version-$BOARD_HOSTNAME-dslot.img.xz"; do
    [ ! -e "$f" ] || echo "note: $f exists and will be rebuilt" >&2
done

log=$out/release-build-$version.log
mkdir -p "$out"
exec > >(tee "$log") 2>&1
step() { echo; echo "== $* ($(date '+%H:%M:%S'))"; }
echo "release-build $version at $(git -C "$NIBOS_ROOT" rev-parse --short HEAD); log: $log"
for v in DSLOT_SRC NIBCASE_SRC INKTICK_SRC DSPERATE_SRC INKDOT_SRC; do
    echo "  $v=${!v} at $(git -C "${!v}" rev-parse --short HEAD)"
done

# --- 1. nibos-hw -------------------------------------------------------------------------
step "nibos-hw"
in_vm bash -c "cd '$NIBOS_ROOT/hw' && PKGDEST=\"\$PWD/out\" BUILDDIR=\"\$HOME/makepkg-build\" makepkg -f --nodeps" >/dev/null
# shellcheck disable=SC2012 # the newest by time is what make-image.sh would pick too
hwpkg=$(ls -t "$NIBOS_ROOT"/hw/out/nibos-hw-*.pkg.tar.* | head -1)
echo "built ${hwpkg##*/}"

# --- 2. the images -------------------------------------------------------------------------
step "image: nibOS"
VERSION=$version SKIP_BUILD=1 HWPKG=$hwpkg "$here/make-image.sh"
step "image: nibOS + DSlot"
VERSION=$version SKIP_BUILD=1 HWPKG=$hwpkg "$here/make-image.sh" --dslot
base=$out/nibos-$version-$BOARD_HOSTNAME
plain=$base.img.xz dslot=$base-dslot.img.xz
work_plain=$out/work-nibos-$version-$BOARD_HOSTNAME work_dslot=$work_plain-dslot
ls -la "$plain" "$dslot"

# --- 3. the tests, one at a time --------------------------------------------------------
step "test-image"
in_vm sudo bash "$here/test-image.sh" "$dslot" "$plain"
step "test-update: nibOS + DSlot"
in_vm sudo bash "$here/test-update.sh" "$dslot" "$work_dslot"
step "test-update: nibOS"
in_vm sudo bash "$here/test-update.sh" "$plain" "$work_plain"

# --- 4. the corresponding source -----------------------------------------------------------
step "collect-sources"
in_vm env DSLOT_SRC="$DSLOT_SRC" DSPERATE_SRC="$DSPERATE_SRC" INKDOT_SRC="$INKDOT_SRC" \
    INKTICK_SRC="$INKTICK_SRC" NIBCASE_SRC="$NIBCASE_SRC" \
    python3 "$here/collect-sources.py" "$out/sources-$version" "$work_plain" "$work_dslot" --cache "$out/sources-cache"
[ ! -s "$out/sources-$version/failed.txt" ] || die "some sources were not collected: $out/sources-$version/failed.txt"
step "pack-sources"
in_vm python3 "$here/pack-sources.py" "$out/sources-$version" "nibos-$version-sources" --notes "$out/sources-$version/SOURCES.md"
ls -la "$out/sources-$version/nibos-$version-sources-"*

step "built; what needs a person is next (docs/releasing.md)"
cat <<EOF
1. Release notes: out/release-notes-$version.md (the previous release's, rewritten).
2. Sign (asks for the key's passphrase), from this directory:
     tools/make-release.sh --key ~/.minisign/nibos-update.key --channel stable \\
       --notes out/release-notes-$version.md ${work_plain#"$NIBOS_ROOT/"} ${work_dslot#"$NIBOS_ROOT/"}
3. Publish: gh release create $tag with out/release-$version/* and the notes, then
   gh release upload $tag of out/sources-$version/nibos-$version-sources-* and SOURCES.md.
4. Push this tree and $tag, and the project trees the images were built from.
5. The OTA test on a device on the previous release, then the site.
EOF
