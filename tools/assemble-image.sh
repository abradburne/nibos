#!/bin/bash
# Lay out a downloadable image (docs/design/images.md) from its parts, in the build machine:
# the GPT, the bootloader at the sectors SPL is built for, and each partition's contents
# written at its offset in the file (no loop devices). tools/make-image.sh runs it.
#
#   tools/assemble-image.sh WORK OUT.img
#
# WORK holds idbloader-falcon.img, u-boot-fallback.itb, falcon.itb, root.img (ext4) and
# boot/ (what goes on the NIBOS partition). The state partition is made here, with the
# first boot's "factory" marker; the data partition isn't in the image (the first boot makes
# it over the rest of the card).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"
[ $# -eq 2 ] || usage 2
work=$1 img=$2
for f in idbloader-falcon.img u-boot-fallback.itb falcon.itb root.img; do
    [ -f "$work/$f" ] || die "no $work/$f"
done
for t in sgdisk mkfs.fat mcopy mkfs.ext4; do command -v "$t" >/dev/null || die "needs $t"; done

# The layout, in 512-byte sectors. The Falcon partition is where SPL reads the FIT
# (BOARD_FALCON_SECTOR); the rest follow, 1 MiB aligned.
mib=2048
falcon_start=$BOARD_FALCON_SECTOR
falcon_size=$((64 * mib))
boot_start=$((falcon_start + falcon_size)) boot_size=$((128 * mib))
roota_start=$((boot_start + boot_size)) root_size=$((3072 * mib))
rootb_start=$((roota_start + root_size))
state_start=$((rootb_start + root_size)) state_size=$((512 * mib))
end=$((state_start + state_size))
[ "$falcon_start" -ge "$BOARD_BOOTLOADER_END" ] || die "the Falcon partition would overlap the bootloader area"

rm -f "$img"
# 1 MiB past the last partition, for the backup GPT. The first boot puts the data partition
# 8 MiB past it (firstboot-data), so writing an image never reaches a card's games and saves.
truncate -s $(((end + mib) * 512)) "$img"
# (No -q: this sgdisk takes it and then writes nothing.)
sgdisk -Z "$img" >/dev/null
sgdisk \
    -n "1:$falcon_start:$((falcon_start + falcon_size - 1))" -t 1:8300 -c 1:falcon \
    -n "2:$boot_start:$((boot_start + boot_size - 1))" -t 2:0700 -c 2:nibos-boot \
    -n "3:$roota_start:$((roota_start + root_size - 1))" -t 3:8300 -c 3:nibos-root-a \
    -n "4:$rootb_start:$((rootb_start + root_size - 1))" -t 4:8300 -c 4:nibos-root-b \
    -n "5:$state_start:$((state_start + state_size - 1))" -t 5:8300 -c 5:nibos-state \
    "$img" >/dev/null

put() { # FILE SECTOR [MAX_SECTORS]
    local size
    size=$(stat -c %s "$1")
    [ -z "${3:-}" ] || [ "$size" -le $(($3 * 512)) ] || die "$1 is larger than its place"
    dd if="$1" of="$img" bs=512 seek="$2" conv=notrunc,sparse status=none
}
# The bootloader: TPL + SPL at sector 64, the fallback U-Boot where SPL looks with A held.
put "$work/idbloader-falcon.img" "$BOARD_BOOTLOADER_START" $((16384 - BOARD_BOOTLOADER_START))
put "$work/u-boot-fallback.itb" "$BOARD_FALLBACK_SECTOR" $((BOARD_BOOTLOADER_END - BOARD_FALLBACK_SECTOR))
put "$work/falcon.itb" "$falcon_start" "$falcon_size"

# NIBOS: FAT32, what the user edits.
fat=$(mktemp)
rm -f "$fat"     # mkfs.fat -C makes the file itself
mkfs.fat -F 32 -n NIBOS -C "$fat" $((boot_size / 2)) >/dev/null
if [ -d "$work/boot" ] && [ -n "$(ls -A "$work/boot")" ]; then
    MTOOLS_SKIP_CHECK=1 mcopy -s -i "$fat" "$work/boot/"* ::/
fi
put "$fat" "$boot_start" "$boot_size"
rm -f "$fat"

# The root (grown to its partition on the first boot).
put "$work/root.img" "$roota_start" "$root_size"

# The state partition: empty but for the first boot's marker and the places provisioning writes.
state=$(mktemp -d)
mkdir -p "$state/ssh" "$state/iwd"
chmod 700 "$state/iwd"
echo factory > "$state/data-partition"
st=$(mktemp)
truncate -s $((state_size * 512)) "$st"
mkfs.ext4 -q -L nibos-state -d "$state" "$st"
put "$st" "$state_start" "$state_size"
rm -rf "$state" "$st"

sgdisk -v "$img" | grep -q 'No problems found' || die "the partition table doesn't check out"
sgdisk -p "$img" | tail -6
