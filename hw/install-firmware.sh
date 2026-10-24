#!/bin/bash
# Put the board's firmware from its stock image (BOARD_FIRMWARE: the RG DS+'s speaker amps'
# tuning file) on a running device, from the user's own extraction (STOCK_DIR, made by
# boards/<board>/extract-stock.sh). It isn't ours to redistribute, so no package carries it;
# rootfs/make-arch.sh puts it in a new image, and this is for a device made otherwise, or
# one whose copy went missing. The amps read it when they start: restart the device after.
#
#   hw/install-firmware.sh [HOST]        (Mac or VM; HOST defaults to DEVICE)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
host=${1:-$DEVICE}
[ -n "$BOARD_FIRMWARE" ] || { echo "the board needs no firmware from its stock image"; exit 0; }
for fw in $BOARD_FIRMWARE; do
    [ -f "$STOCK_DIR/$fw" ] || die "no $STOCK_DIR/$fw: run boards/$NIBOS_BOARD/extract-stock.sh"
    scp -q "${ssh_opts[@]}" "$STOCK_DIR/$fw" "$host:/tmp/$fw"
    ssh "${ssh_opts[@]}" "$host" "sudo install -Dm644 /tmp/$fw /usr/lib/firmware/$fw && rm /tmp/$fw"
    echo "installed /usr/lib/firmware/$fw on $host"
done
echo "restart the device for the sound to pick it up"
