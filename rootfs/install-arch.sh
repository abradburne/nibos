#!/bin/bash
# Put Arch Linux ARM on a dev card, on a Mac (the card made by
# kernel/make-dev-card.sh, in the Mac's reader):
#
#   rootfs/install-arch.sh [IMAGE]        (default out/archroot.img)
#
#  1. moves the card's backup GPT to the real end of the card (the stock image
#     put it at 16 GB) and adds the root partition (BOARD_ROOT_PART, 12 on the
#     RG DS+, "archroot") over all the free space;
#  2. writes the root image into it (the file system grows to fill it on first boot);
#  3. writes kernel/out/boot-arch.img to the boot partition (flash-boot.sh keeps
#     the card's original on ROMS/nibos/boot-backup.img).
#
# The stock partitions are not touched. It asks for your password.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
image=${1:-$here/out/archroot.img}
boot="$here/../kernel/out/boot-arch.img"
n=$BOARD_ROOT_PART label=$BOARD_ROOT_PARTLABEL

[ -f "$image" ] || die "no $image (run make-arch.sh)"
[ -f "$boot" ] || die "no $boot (run kernel/mkboot.py)"
command -v sgdisk >/dev/null || die "needs sgdisk: brew install gptfdisk"
disk=$(card_disk)

# Wi-Fi networks for the device go here (docs/using.md).
mkdir -p "$roms_volume/nibos/wifi"

diskutil unmountDisk "$disk"
if ! sudo sgdisk -p "$disk" | awk -v n="$n" '$1 == n' | grep -q "$label"; then
    echo "Adding partition $n ($label) over the free space of $disk"
    sudo sgdisk -e "$disk"
    sudo sgdisk -n "$n:0:0" -t "$n:8305" -c "$n:$label" "$disk"
fi
sudo sgdisk -p "$disk" | tail -3
part="${disk}s$n"
diskutil unmountDisk "$disk" 2>/dev/null || true

echo "Writing the root file system ($(( $(stat -f %z "$image") / 1000000 )) MB) to $part; a few minutes."
sudo dd if="$image" of="/dev/r${part#/dev/}" bs=4m status=progress
sync

diskutil mountDisk "$disk" >/dev/null 2>&1 || true
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -d "$roms_volume" ] && break; sleep 1; done
"$here/../kernel/flash-boot.sh" "$boot"
