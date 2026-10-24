#!/bin/bash
# Make a release's files from built images (docs/design/updates.md), ready to attach to a
# GitHub release. Uploads nothing.
#
#   tools/make-release.sh --key KEY [--channel stable|beta] [--notes FILE] [--notes-url URL]
#                         [--min-from VERSION] [--version VERSION] WORK...
#
# WORK: tools/make-image.sh's work directories (out/work-nibos-<version>-<board>[-dslot]), one
# per edition, from a clean tagged commit. KEY: the minisign secret key; minisign asks for its
# passphrase. The key is used where it is and never copied. Writes, for each edition, into
# out/release-<version>/: the manifest and its .minisig, the .slot payload and the .img.xz;
# then SHA256SUMS.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"

key="" version=""
opts=() works=()
while [ $# -gt 0 ]; do
    case "$1" in
    --key) key=$2; shift 2 ;;
    --channel | --notes | --notes-url | --min-from) opts+=("$1" "$2"); shift 2 ;;
    --version) version=$2; opts+=("$1" "$2"); shift 2 ;;
    -*) usage ;;
    *) works+=("$(cd "$1" && pwd)"); shift ;;
    esac
done
[ -n "$key" ] && [ -f "$key" ] && [ "${#works[@]}" -gt 0 ] || usage
command -v minisign >/dev/null || in_vm which minisign >/dev/null 2>&1 || die "needs minisign (here or in the build machine)"

[ -n "$version" ] || version=$(sed -n 's/^NIBOS_VERSION=//p' "${works[0]}/docs/release")
out="$NIBOS_ROOT/out/release-$version"
mkdir -p "$out"
for w in "${works[@]}"; do
    for f in root.img falcon.itb falcon-b.itb docs/release; do
        [ -f "$w/$f" ] || die "$w has no $f: build it with tools/make-image.sh"
    done
    in_vm python3 "$here/release.py" "$w" "$out" ${opts[@]+"${opts[@]}"}
    image=$(sed -n 's/^NIBOS_IMAGE=//p' "$w/docs/release")
    manifest="$out/nibos-$version-$image.manifest.json"
    # Signed where minisign is: on this machine if it has it (the key stays here), else in the
    # build machine, which sees the same path.
    trusted="nibOS $version $image"
    if command -v minisign >/dev/null; then
        minisign -S -s "$key" -m "$manifest" -t "$trusted"
    else
        in_vm minisign -S -s "$key" -m "$manifest" -t "$trusted"
    fi
    img="$NIBOS_ROOT/out/${w##*/work-}.img.xz"
    [ ! -f "$img" ] || cp "$img" "$out/nibos-$version-$image.img.xz"
done
(cd "$out" && shasum -a 256 nibos-* > SHA256SUMS)
ls -la "$out"
