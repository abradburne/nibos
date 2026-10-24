#!/bin/bash
# Put the stock bootloader back on a device's card, on a Mac, when the device
# no longer starts: writes ROMS/nibos/bootloader-stock.img (kept by install.sh)
# to the bootloader area (sectors 64-24575 on the RG DS+). Asks for your
# password.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"

keep="$roms_volume/nibos/bootloader-stock.img"
[ -f "$keep" ] || die "no $keep: is the card in, and is its ROMS volume mounted?"
[ "$(shasum -a 256 "$keep" | cut -d' ' -f1)" = "$(cat "$keep.sha256")" ] || die "$keep does not match its checksum"
disk=$(card_disk)

copy=$(mktemp -t bootloader-stock)
cp "$keep" "$copy"
diskutil unmountDisk "$disk"
echo "Writing the stock bootloader to $disk at sector $BOARD_BOOTLOADER_START"
sudo dd if="$copy" of="/dev/r${disk#/dev/}" bs=512 seek="$BOARD_BOOTLOADER_START" conv=notrunc
rm -f "$copy"
sync
diskutil eject "$disk"
echo "Done. The card can go back in the device."
