#!/bin/bash
# Restart the DS+ and print U-Boot's boot-stage timings (needs a U-Boot built
# with bootstage stash, which build.sh does) and when the kernel started. Needs
# the kernel DTB that reserves 0x0f000000-0x0f01ffff.
#   uboot/measure.sh [user@host]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
target=${1:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=3)
mkdir -p "$here/out"
ssh "${ssh_opts[@]}" "$target" 'sudo systemctl reboot' >/dev/null 2>&1 || true
sleep 10
until ssh "${ssh_opts[@]}" "$target" true 2>/dev/null; do sleep 2; done
scp -q "${ssh_opts[@]}" "$(device_tool memdump)" "$target:/tmp/memdump"
ssh "${ssh_opts[@]}" "$target" 'sudo /tmp/memdump 0x0f000000 0x4000' > "$here/out/bootstage.bin"
python3 "$here/bootstage.py" "$here/out/bootstage.bin"
# A MEASURE build also leaves its console log at 0x0f004000.
ssh "${ssh_opts[@]}" "$target" 'sudo /tmp/memdump 0x0f004000 0x1c000' | tr -d '\0' > "$here/out/console.log" || true
[ -s "$here/out/console.log" ] && echo "U-Boot console log: $here/out/console.log"
# When the kernel started, on the arch counter (counter minus CLOCK_BOOTTIME).
# Not journald's kernel stamps: dmesg starts at 0 under mainline U-Boot and
# journald's "monotonic" for early kernel lines is not from reset. The counter
# restarts at about U-Boot proper's start (TF-A), so add TPL+SPL ("end phase"
# above, ~0.36 s) for time from reset.
scp -q "${ssh_opts[@]}" "$(device_tool counter)" "$target:/tmp/counter"
ssh "${ssh_opts[@]}" "$target" /tmp/counter | awk '{ printf "kernel started at %.3f s on the counter\n", $NF }'
