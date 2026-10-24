#!/bin/bash
# Write a boot image to the card's boot partition (the stock U-Boot's), on a Mac, or put back
# what was there before.
#
#   flash-boot.sh out/boot-smoke.img     back up the card's boot partition (once), then write the image
#   flash-boot.sh --restore              write the backup back
#
# The backup is kept on the card itself, ROMS/nibos/boot-backup.img (with
# .sha256), so it always belongs to the card it came from. It is taken only if
# it does not exist yet, so a second flash never overwrites the original. It
# asks for your password to write.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"

disk=$(card_disk)
part=${disk}s$BOARD_BOOT_PART
# The stock layout's boot partition is 64 MiB.
diskutil info "$part" | grep -q '(67108864 Bytes)' || die "$part is not 64 MiB: not a $BOARD_NAME card?"
backup="$roms_volume/nibos/boot-backup.img"

if [ "${1:-}" = --restore ]; then
    [ -f "$backup" ] || die "no backup at $backup"
    [ "$(shasum -a 256 "$backup" | cut -d' ' -f1)" = "$(cat "$backup.sha256")" ] || die "$backup does not match its checksum"
    image=$backup
else
    image=${1:-}
    [ -n "$image" ] || usage
    [ -f "$image" ] || die "no $image"
    [ "$(head -c 4 "$image" | xxd -p)" = d00dfeed ] || die "$image is not a FIT boot image"
fi

if [ "$image" != "$backup" ] && [ ! -f "$backup" ]; then
    echo "Backing up $part to $backup"
    mkdir -p "$(dirname "$backup")"
    sudo dd if="/dev/r${part#/dev/}" of="$backup.part" bs=1m
    sudo chown "$(id -u):$(id -g)" "$backup.part"
    mv "$backup.part" "$backup"
    shasum -a 256 "$backup" | cut -d' ' -f1 > "$backup.sha256"
fi
copy=$(mktemp -t nibos-boot)
cp "$image" "$copy"
# Raw disk writes on macOS must be whole sectors; pad to a whole MiB.
size=$(stat -f %z "$copy")
pad=$(( (1048576 - size % 1048576) % 1048576 ))
[ "$pad" = 0 ] || head -c "$pad" /dev/zero >> "$copy"
diskutil unmountDisk "$disk"
echo "Writing $(basename "$image") to $part"
sudo dd if="$copy" of="/dev/r${part#/dev/}" bs=1m
rm -f "$copy"
sync
diskutil eject "$disk"
echo "Done. The card can go back in the device."
