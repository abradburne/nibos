# shellcheck shell=bash
# Shared start of nibOS's host-side bash scripts: the settings (config.sh,
# config.local.sh, and through them the board's board.sh and its SoC's soc.sh)
# and a few helpers. A script sources it by its path relative
# to the script itself:
#
#   here="$(cd "$(dirname "$0")" && pwd)"
#   . "$here/../tools/lib.sh"

NIBOS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export NIBOS_ROOT
# shellcheck source=../config.sh
. "$NIBOS_ROOT/config.sh"

# Options for every ssh and scp to the device. Scripts add their own
# ConnectTimeout where waiting matters.
ssh_opts=(-o IdentitiesOnly=yes -i "$SSH_KEY")

# Stop with a message on stderr.
die() {
    echo "${0##*/}: $*" >&2
    exit 1
}

# Print the script's header comment (everything from line 2 to the first line
# that isn't a comment), which says how to call it, and exit: with status 0
# when asked for (-h), 2 after a wrong call.
usage() {
    awk 'NR > 1 && !/^#/ { exit } NR > 1 { sub(/^# ?/, ""); print }' "$0" >&2
    exit "${1:-2}"
}

# Run a command in the build machine. Values from config.sh that are paths
# inside it (KSRC, USRC, RKBIN) are only right there: source tools/lib.sh in
# the command to get them.
in_vm() {
    orb -m "$BUILD_MACHINE" "$@"
}

# Run a bash snippet in the build machine with tools/lib.sh sourced and the
# repository as the working directory.
in_vm_sh() {
    in_vm bash -c "set -euo pipefail; . '$NIBOS_ROOT/tools/lib.sh'; cd '$NIBOS_ROOT'; $1"
}

# Run a command on the device.
on_device() {
    ssh "${ssh_opts[@]}" "$DEVICE" "$@"
}

# A small tool that runs on the device (tools/device/NAME.c), built static in
# the build machine the first time it is wanted. Prints its path.
device_tool() {
    local out="$NIBOS_ROOT/tools/device/out"
    if [ ! -f "$out/$1" ]; then
        mkdir -p "$out"
        in_vm gcc -O2 -static -o "$out/$1" "$NIBOS_ROOT/tools/device/$1.c" >&2
    fi
    echo "$out/$1"
}

# The time from reset to the end of SPL, in seconds, from the bootstage records
# mainline U-Boot leaves in RAM at 0x0f000000 (uboot/bootstage.py); empty when
# there are none (the stock bootloader). Needs memdump (device_tool) on the
# device, in /tmp unless the second argument says where.
#
#   spl_time [user@host] [/path/to/memdump]
spl_time() {
    local stash
    stash=$(mktemp)
    ssh "${ssh_opts[@]}" "${1:-$DEVICE}" "sudo ${2:-/tmp/memdump} 0x0f000000 0x4000" > "$stash" 2>/dev/null || true
    python3 "$NIBOS_ROOT/uboot/bootstage.py" "$stash" 2>/dev/null | awk '/end phase/ { print $1; exit }'
    rm -f "$stash"
}

# The Mac's mount point of the card's ROMS volume (FAT, ROMS_VOLUME), and the
# disk of the card in the Mac's reader (/dev/diskN), found by that volume, which
# is partition BOARD_ROMS_PART of a card made from the stock image. Stops if
# there is none, or it isn't an SD card laid out like one. macOS only (diskutil).
roms_volume=$ROMS_VOLUME
card_disk() {
    local node disk
    command -v diskutil >/dev/null ||
        die "the card scripts run on a Mac today (they use diskutil): docs/install.md"
    node=$(diskutil info "$roms_volume" 2>/dev/null | awk '/Device Node/ { print $3 }' || true)
    [ -n "$node" ] || die "no $roms_volume: is the $BOARD_NAME card in the Mac's reader?"
    [ "${node##*s}" = "$BOARD_ROMS_PART" ] || die "$node is not partition $BOARD_ROMS_PART: not a $BOARD_NAME card?"
    disk=${node%s"$BOARD_ROMS_PART"}
    diskutil info "$disk" | grep -q 'Protocol: *Secure Digital' || die "$disk is not an SD card"
    echo "$disk"
}

# The size of a file in bytes, on macOS or Linux.
file_size() {
    stat -f %z "$1" 2>/dev/null || stat -c %s "$1"
}

# Every script that sources this answers -h and --help with its header. (A
# sourced file sees the script's own arguments.)
case "${1:-}" in -h | --help) usage 0 ;; esac
