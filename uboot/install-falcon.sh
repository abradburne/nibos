#!/bin/bash
# Falcon boot on a running device, over SSH: SPL starts Linux directly, no
# U-Boot proper. Writes ../kernel/out/falcon.itb (made by kernel/deploy.sh,
# which also writes it at every deploy) to the board's Falcon partition (the
# RG DS+'s partition 4, "recovery") and
# out/idbloader-falcon.img (build.sh with FALCON=1) at sector 64, over the SPL
# install.sh put there; install.sh's U-Boot proper stays at 0x4000, unused.
#
#   uboot/install-falcon.sh [user@host]
#
# Keeps, once, the stock recovery partition on ROMS/nibos/recovery-stock.img
# (install.sh keeps the stock bootloader). Back to normal mainline U-Boot:
# install.sh; to the stock bootloader: install.sh --stock, or restore-stock.sh
# from a Mac if the device won't start.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
target=${1:-$DEVICE}
itb="$here/../kernel/out/falcon.itb"
spl="$here/out/idbloader-falcon.img"
[ -f "$itb" ] && [ -f "$spl" ] || die "build first: FALCON=1 build.sh, and kernel/deploy.sh for the FIT"
[ "$(file_size "$spl")" -le $(( (16384 - BOARD_BOOTLOADER_START) * 512 )) ] || die "SPL too large"

ssh "${ssh_opts[@]}" "$target" 'test -f /mnt/roms/nibos/bootloader-stock.img' ||
    die "no stock bootloader copy yet: run install.sh once first"
scp -q "${ssh_opts[@]}" "$itb" "$spl" "$target:/tmp/"
ssh "${ssh_opts[@]}" "$target" "NAME=$BOARD_FALCON_PARTNAME PART=$BOARD_FALCON_PART SECTOR=$BOARD_FALCON_SECTOR FIRST=$BOARD_BOOTLOADER_START sh -s" <<'EOF2'
set -e
part=$(for p in /sys/class/block/mmcblk0p*; do grep -q "^PARTNAME=$NAME$" "$p/uevent" && basename "$p"; done | head -1)
[ "$part" = "mmcblk0p$PART" ] || { echo "$NAME is not partition $PART ($part)"; exit 1; }
[ "$(cat "/sys/class/block/$part/start")" = "$SECTOR" ] || { echo "partition $PART does not start at sector $SECTOR"; exit 1; }
keep=/mnt/roms/nibos/recovery-stock.img
if [ ! -f "$keep" ]; then
    echo "keeping the stock recovery partition in $keep"
    sudo dd if="/dev/$part" of="$keep.part" bs=1M status=none
    sudo mv "$keep.part" "$keep"
    sha256sum "$keep" | cut -d" " -f1 | sudo tee "$keep.sha256" >/dev/null
fi
sudo dd if=/tmp/falcon.itb of="/dev/$part" bs=1M conv=fsync status=none
sudo dd if=/tmp/idbloader-falcon.img of=/dev/mmcblk0 bs=512 seek="$FIRST" conv=fsync,notrunc status=none
rm -f /tmp/falcon.itb /tmp/idbloader-falcon.img
sync
echo "Falcon boot installed"
EOF2
echo "Restart the device to use it."
