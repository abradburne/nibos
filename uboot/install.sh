#!/bin/bash
# Put mainline U-Boot (out/u-boot-rockchip.bin) on a running device's card
# (TF1 on the RG DS+), over SSH, after keeping the stock bootloader:
#
#   uboot/install.sh [user@host]           install mainline U-Boot
#   uboot/install.sh --stock [user@host]   put the stock bootloader back
#   uboot/install.sh --fallback [user@host]  put out/u-boot-fallback.itb (a
#       known-good U-Boot, FALLBACK=1 build.sh) at BOARD_FALLBACK_SECTOR: SPL
#       loads it instead of the one at 0x4000 when A is held at power-on
#
# The bootloader area is BOARD_BOOTLOADER_START to _END (sectors 64-24575 on the
# RG DS+): the stock DDR init/SPL loader at 64 and the stock U-Boot (the "uboot"
# partition) at 16384. The first run copies it
# to ROMS/nibos/bootloader-stock.img on the card and to out/ here, and never
# overwrites that copy. If the device won't start, put the card in a Mac and run
# restore-stock.sh. The partition table (sectors 0-33) is never touched.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
mode=install
case "${1:-}" in
--stock) mode=stock; shift ;;
--fallback) mode=fallback; shift ;;
esac
target=${1:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=10)
first=$BOARD_BOOTLOADER_START
count=$((BOARD_BOOTLOADER_END - BOARD_BOOTLOADER_START))

# Keep the stock bootloader, on the card and here.
ssh "${ssh_opts[@]}" "$target" "FIRST=$first COUNT=$count sh -s" <<'EOF2'
set -e
keep=/mnt/roms/nibos/bootloader-stock.img
ls /mnt/roms/nibos >/dev/null
if [ ! -f "$keep" ]; then
    echo "keeping the stock bootloader in $keep"
    sudo dd if=/dev/mmcblk0 of="$keep.part" bs=512 skip="$FIRST" count="$COUNT" status=none
    sudo mv "$keep.part" "$keep"
    sha256sum "$keep" | cut -d" " -f1 | sudo tee "$keep.sha256" >/dev/null
fi
[ "$(sha256sum "$keep" | cut -d" " -f1)" = "$(cat "$keep.sha256")" ] || { echo "$keep does not match its checksum"; exit 1; }
EOF2
mkdir -p "$here/out"
[ -f "$here/out/bootloader-stock.img" ] ||
    scp -q "${ssh_opts[@]}" "$target:/mnt/roms/nibos/bootloader-stock.img" "$target:/mnt/roms/nibos/bootloader-stock.img.sha256" "$here/out/"

if [ "$mode" = stock ]; then
    image=/mnt/roms/nibos/bootloader-stock.img
elif [ "$mode" = fallback ]; then
    image=/tmp/u-boot-fallback.itb
    first=$BOARD_FALLBACK_SECTOR
    size=$(file_size "$here/out/u-boot-fallback.itb")
    [ "$size" -le $(((BOARD_BOOTLOADER_END - first) * 512)) ] || die "u-boot-fallback.itb is larger than the room at sector $first"
    scp -q "${ssh_opts[@]}" "$here/out/u-boot-fallback.itb" "$target:$image"
else
    image=/tmp/u-boot-rockchip.bin
    size=$(file_size "$here/out/u-boot-rockchip.bin")
    [ "$size" -le $((count * 512)) ] || die "u-boot-rockchip.bin is larger than the bootloader area"
    scp -q "${ssh_opts[@]}" "$here/out/u-boot-rockchip.bin" "$target:$image"
fi
ssh "${ssh_opts[@]}" "$target" "sudo dd if=$image of=/dev/mmcblk0 bs=512 seek=$first conv=fsync,notrunc status=none && sync && echo 'written $(basename "$image") at sector $first'"
echo "Restart the device to use it."
