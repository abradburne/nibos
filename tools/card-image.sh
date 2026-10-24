#!/bin/bash
# Save a whole device card to one compressed image file, or write one back, on a Mac.
#
#   tools/card-image.sh save [name]        card in the reader -> backups/<name>.img.zst
#   tools/card-image.sh restore FILE       FILE -> the card in the reader (erases it)
#
# Everything on the card is kept: the bootloader area (SPL, Falcon FIT, fallback
# U-Boot), the partition table, the stock partitions and the Arch root. Unused
# space compresses to almost nothing. The image is the full card size (128 GB),
# so restoring needs a card at least that big. Asks for your password (raw disk
# access); type the disk name back before a restore.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"
backups="$(cd "$here/../.." && pwd)/backups"

case "${1:-}" in
save)
    disk=$(card_disk)
    commit=$(git -C "$here/.." rev-parse --short HEAD 2>/dev/null || echo nogit)
    name=${2:-rgds-dev-$(date +%Y%m%d)-$commit}
    out="$backups/$name.img.zst"
    mkdir -p "$backups"
    [ ! -e "$out" ] || die "$out exists"
    size=$(diskutil info "$disk" | awk -F'[(]' '/Disk Size/ { split($2, a, " "); print a[1] }')
    echo "Saving $disk ($((size / 1000000000)) GB) to $out; this reads the whole card, about 25 minutes."
    diskutil unmountDisk "$disk"
    sudo dd if="/dev/r${disk#/dev/}" bs=4m status=progress | zstd -T0 -3 -q -o "$out"
    shasum -a 256 "$out" | cut -d' ' -f1 > "$out.sha256"
    printf 'card: %s bytes\nfrom: %s\ncommit: %s\nsaved: %s\n' "$size" "$disk" "$commit" "$(date)" > "$out.txt"
    diskutil eject "$disk"
    ls -la "$out"
    ;;
restore)
    file=${2:-}
    [ -n "$file" ] || usage
    [ -f "$file" ] || die "no $file"
    [ "$(shasum -a 256 "$file" | cut -d' ' -f1)" = "$(cat "$file.sha256")" ] || die "$file does not match its checksum"
    disk=$(card_disk)
    need=$(awk '/^card:/ { print $2 }' "$file.txt")
    size=$(diskutil info "$disk" | awk -F'[(]' '/Disk Size/ { split($2, a, " "); print a[1] }')
    [ "$size" -ge "$need" ] || die "$disk is smaller than the saved card"
    diskutil list "$disk"
    printf 'This erases %s. Type %s to go on: ' "$disk" "${disk#/dev/}"
    read -r answer
    [ "$answer" = "${disk#/dev/}" ] || die "Nothing written."
    diskutil unmountDisk "$disk"
    zstd -dc "$file" | sudo dd of="/dev/r${disk#/dev/}" bs=4m status=progress
    sync
    diskutil eject "$disk"
    echo "Done."
    ;;
*)
    usage ;;
esac
