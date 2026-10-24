#!/bin/bash
# Check system updates without a device, in the build machine (as root): a release made from a
# built image (tools/release.py, signed with a throwaway key) written by the real nibos-update
# and nibos-bootctl to slot B of a copy of the image made card sized, and every way it can go
# wrong (docs/design/updates.md).
#
#   orb -m rgbuild sudo bash tools/test-update.sh out/nibos-<version>-<board>.img.xz WORK
#
# WORK is the image's tools/make-image.sh work directory (root.img, falcon.itb, falcon-b.itb,
# docs/release). The release is the same build with a newer version.
# shellcheck disable=SC2016 # each check is a string, evaluated by check()
set -euo pipefail
shopt -s inherit_errexit
[ $# -eq 2 ] || { sed -n '2,9s/^# \{0,1\}//p' "$0" >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
image=$1 work=$(cd "$2" && pwd)
fails=0
ok() { echo "ok:   $1"; }
bad() { echo "FAIL: $1"; fails=$((fails + 1)); }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

tmp=$(mktemp -d)
loop=
cleanup() {
    mountpoint -q "$tmp/m" 2>/dev/null && umount "$tmp/m"
    [ -z "$loop" ] || losetup -d "$loop" 2>/dev/null || true
    rm -rf "$tmp"
}
trap cleanup EXIT

# The programs, as nibos-hw builds them.
user=${SUDO_USER:-root}
target=/tmp/nibos-check/test-update
su "$user" -c "cd '$here/../hw/daemons' && CARGO_TARGET_DIR=$target cargo build -q --release --bin nibos-update --bin nibos-bootctl"
export PATH="$target/release:$PATH"

# The card.
img=$tmp/card.img
xz -dc "$image" > "$img"
truncate -s +4G "$img"
loop=$(losetup -fP --show "$img")
for _ in $(seq 50); do [ -b "${loop}p4" ] && break; sleep 0.1; done

# A throwaway signing key, and the release: the same build, a newer version.
minisign -G -W -p "$tmp/test.pub" -s "$tmp/test.key" -f >/dev/null
running=$(sed -n 's/^NIBOS_VERSION=//p' "$work/docs/release")
newer=99.0.0
python3 "$here/release.py" "$work" "$tmp/rel" --version "$newer" --channel beta >/dev/null
manifest=$(echo "$tmp"/rel/*.manifest.json)
minisign -S -s "$tmp/test.key" -m "$manifest" -t "test release" >/dev/null
payload=$(echo "$tmp"/rel/*.slot)

# The system as nibos-update sees it, under a test root: its release file and key, the keep
# list, the battery, and a little state to carry over.
P=$tmp/root
mkdir -p "$P/usr/lib/nibos" "$P/var/lib/nibos/state/update" "$P/run/nibos/update" \
    "$P/run/nibos/battery" "$P/run/nibos/charger" "$P/etc/ssh" "$P/home/nibos/.config/dslot"
cp "$work/docs/release" "$P/usr/lib/nibos/release"
cp "$tmp/test.pub" "$P/usr/lib/nibos/update.pub"
cp "$here/../hw/files/usr/lib/nibos/update-keep" "$P/usr/lib/nibos/update-keep"
echo 0123456789abcdef > "$P/etc/machine-id"
echo key > "$P/etc/ssh/ssh_host_ed25519_key"
echo 'theme = "x"' > "$P/home/nibos/.config/dslot/settings.toml"
echo 80 > "$P/run/nibos/battery/capacity"
echo 0 > "$P/run/nibos/charger/online"
export NIBOS_UPDATE_TEST_ROOT=$P NIBOS_UPDATE_RUNNING=A NIBOS_UPDATE_SLOT_DEV=${loop}p4 \
    NIBOS_BOOTCTL_DEVICE=${loop}p1 NIBOS_UPDATE_NO_REBOOT=1
st() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2], ""))' "$P/run/nibos/update/status.json" "$1"; }
# A fresh image's card has no boot control block (SPL boots slot A); apply makes it.

echo "== check"
check "a newer release from a file: available" 'nibos-update check --from "$manifest" >/dev/null && [ "$(st phase)" = available ] && [ "$(st available)" = "$newer" ]'
cp "$tmp/test.pub" "$tmp/good.pub"
minisign -G -W -p "$P/usr/lib/nibos/update.pub" -s "$tmp/other.key" -f >/dev/null
check "signed with another key: refused" '! nibos-update check --from "$manifest" 2>/dev/null && [ "$(st phase)" = error ]'
cp "$tmp/good.pub" "$P/usr/lib/nibos/update.pub"
sed -i 's/^NIBOS_EDITION=.*/NIBOS_EDITION=other/' "$P/usr/lib/nibos/release"
check "another edition: refused" 'nibos-update check --from "$manifest" >/dev/null && [ "$(st phase)" = current ]'
cp "$work/docs/release" "$P/usr/lib/nibos/release"
sed -i 's/^NIBOS_BOARD=.*/NIBOS_BOARD=other/' "$P/usr/lib/nibos/release"
check "another board: refused" 'nibos-update check --from "$manifest" >/dev/null && [ "$(st phase)" = current ]'
cp "$work/docs/release" "$P/usr/lib/nibos/release"
sed -i "s/^NIBOS_VERSION=.*/NIBOS_VERSION=$newer/" "$P/usr/lib/nibos/release"
check "the same version: not offered" 'nibos-update check --from "$manifest" >/dev/null && [ "$(st phase)" = current ]'
cp "$work/docs/release" "$P/usr/lib/nibos/release"
check "back to the newer release" 'nibos-update check --from "$manifest" >/dev/null && [ "$(st phase)" = available ]'

echo "== download"
echo 20 > "$P/run/nibos/battery/capacity"
check "at 20% unplugged: refused" '! nibos-update download 2>/dev/null'
echo 1 > "$P/run/nibos/charger/online"
total=$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["root"]["chunks"]))' "$manifest")
# Stopped part way (SIGTERM, as stopping its unit does), then resumed.
nibos-update download >/dev/null 2>&1 &
pid=$!
for _ in $(seq 600); do
    [ -f "$P/var/lib/nibos/state/update/progress" ] &&
        [ "$(wc -l < "$P/var/lib/nibos/state/update/progress")" -ge 4 ] && break
    sleep 0.1
done
kill -TERM "$pid" 2>/dev/null || true
wait "$pid" || true
done1=$(($(wc -l < "$P/var/lib/nibos/state/update/progress") - 1))
check "stopped after $done1 of $total chunks: paused" '[ "$(st phase)" = paused ] && [ "$done1" -lt "$total" ]'
check "resumed: written, the slot's FIT too" 'nibos-update download >/dev/null && [ "$(st phase)" = ready ]'
# Every chunk was read back and checked as it was written; since then the kept state was copied
# in, so the slot is no longer the image byte for byte: a clean file system with its files.
check "slot B's root: a clean file system with the release's files" 'e2fsck -fn "${loop}p4" >/dev/null 2>&1 && debugfs -R "cat /usr/lib/nibos/release" "${loop}p4" 2>/dev/null | cmp -s - "$work/docs/release"'
# shellcheck disable=SC2034 # used in the checks
fit=$(stat -c %s "$work/falcon-b.itb")
check "slot B's FIT in the Falcon partition, 32 MiB in" 'cmp -n "$fit" "$work/falcon-b.itb" <(dd if="${loop}p1" bs=1M skip=32 status=none)'
check "no boot control block until applied (SPL keeps booting A)" 'nibos-bootctl status | grep -q "no boot control block"'
mkdir -p "$tmp/m"
mount -o ro "${loop}p4" "$tmp/m"
check "the device's identity and the user's settings carried over" 'cmp -s "$P/etc/machine-id" "$tmp/m/etc/machine-id" && cmp -s "$P/home/nibos/.config/dslot/settings.toml" "$tmp/m/home/nibos/.config/dslot/settings.toml" && cmp -s "$P/etc/ssh/ssh_host_ed25519_key" "$tmp/m/etc/ssh/ssh_host_ed25519_key"'
umount "$tmp/m"

echo "== a damaged download"
cp "$payload" "$tmp/good.slot"
python3 - "$payload" "$manifest" <<'PY'
import json, sys
m = json.load(open(sys.argv[2]))
c = m["root"]["chunks"][2]
with open(sys.argv[1], "r+b") as f:
    f.seek(c["offset"] + c["size"] // 2)
    b = f.read(1)
    f.seek(-1, 1)
    f.write(bytes([b[0] ^ 1]))
PY
rm -f "$P/var/lib/nibos/state/update/progress" "$P/var/lib/nibos/state/update/ready"
check "a damaged chunk: refused, not ready, the failed step named" '! nibos-update download 2>/dev/null && [ "$(st phase)" = error ] && [ "$(st step)" = download ] && [ ! -e "$P/var/lib/nibos/state/update/ready" ]'
check "nothing to apply" '! nibos-update apply 2>/dev/null && nibos-bootctl status | grep -q "no boot control block"'
cp "$tmp/good.slot" "$payload"
check "the good one again: ready" 'nibos-update download >/dev/null && [ "$(st phase)" = ready ]'

echo "== apply, and after"
check "applied: slot B on trial (3 tries), the old version recorded" 'nibos-update apply >/dev/null && nibos-bootctl status | grep -q "active: B" && [ "$(st previous)" = "$running" ]'
# Back on A (as after three failed tries): the update is reported as failed.
check "back on the old version: \"failed\"" 'nibos-update booted && [ "$(st failed)" = "$newer" ] && [ ! -e "$P/var/lib/nibos/state/update/applied" ]'
check "ack forgets it" 'nibos-update ack && [ -z "$(st failed)" ]'
# Going back by hand (slot B confirmed good, then rollback): the version left behind is recorded.
nibos-bootctl try B >/dev/null && nibos-bootctl good >/dev/null 2>&1
check "rollback: back to A, the version left behind recorded" 'nibos-update rollback >/dev/null && nibos-bootctl status | grep -q "active: A" && [ "$(st previous)" = "$running" ]'
echo "($running → $newer, $total chunks)"

echo
if [ "$fails" = 0 ]; then echo "all checks passed"; else echo "$fails failed"; exit 1; fi
