#!/bin/bash
# Turn a spare SD card into a development card for the board, on a Mac: write the stock
# firmware image to it (bootloader, partitions, stock OS), then put a nibos boot
# image in its boot partition. The stock OS stays on the card, so restoring the
# boot partition (flash-boot.sh --restore) makes it a normal stock card again.
#
#   make-dev-card.sh /dev/diskN [boot.img]
#
# ERASES the whole card. It asks you to type the disk name back before writing, and
# asks for your password.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
stock=$STOCK_IMAGE
disk=${1:-}
[ -n "$disk" ] || usage
boot=${2:-$here/out/boot-smoke.img}

[ -f "$stock" ] || die "no stock image at $stock"
[ -f "$boot" ] || die "no boot image at $boot"
info=$(diskutil info "$disk") || die "no disk $disk"
echo "$info" | grep -q 'Protocol: *Secure Digital' || die "$disk is not an SD card"
echo "$info" | grep -q 'Removable Media: *Removable' || die "$disk is not removable"
size=$(echo "$info" | awk -F'[(]' '/Disk Size/ { split($2, a, " "); print a[1] }')
need=$(stat -f %z "$stock")
[ "$size" -ge "$need" ] || die "$disk is smaller than the stock image"

echo
diskutil list "$disk"
echo "This erases everything on $disk ($((size / 1000000000)) GB), including:"
for v in $(diskutil list "$disk" | awk 'NR > 2 { print $NF }' | grep "^${disk#/dev/}s"); do
    mp=$(diskutil info "$v" | awk -F': *' '/Mount Point/ { print $2 }')
    [ -n "$mp" ] && [ -d "$mp" ] || continue
    for f in "$mp"/*; do [ -e "$f" ] && echo "  $f"; done
done
printf 'Type %s to go on: ' "${disk#/dev/}"
read -r answer
[ "$answer" = "${disk#/dev/}" ] || die "Nothing written."

diskutil unmountDisk "$disk"
echo "Writing the stock image ($((need / 1000000)) MB); this takes a few minutes."
sudo dd if="$stock" of="/dev/r${disk#/dev/}" bs=4m status=progress
sync
# The image's backup GPT sits at 16 GB, not at the end of this card. Linux reads
# the primary table and is fine with that; macOS only remounts after a re-read.
diskutil unmountDisk "$disk" 2>/dev/null || true
diskutil mountDisk "$disk" >/dev/null 2>&1 || true
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -d "$roms_volume" ] && break; sleep 1; done
[ -d "$roms_volume" ] || die "ROMS did not mount; reinsert the card and run flash-boot.sh $boot"

"$here/flash-boot.sh" "$boot"
