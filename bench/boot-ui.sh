#!/bin/bash
# Boot-to-shelf benchmark: restart the device over SSH and report when DSlot's shelf was on screen.
#
#   bench/boot-ui.sh [user@host] [runs]
#
# DSlot must be switched on (`nib boot-ui on`). Clocks as in bench/boot.sh: the
# kernel's start is the arch counter minus CLOCK_BOOTTIME, plus DDR init and SPL from mainline
# U-Boot's bootstage stash, which makes it seconds from reset. systemd's stamps and DSlot's
# marks ("dslot: 2.345 s first frame shown", CLOCK_BOOTTIME) count from the kernel's start.
#
# Columns: kernel start; kernel->init; dslot.service started; SDL video up; both windows and
# GLES renderers made; ROMs listed; first frame presented on both screens; and, started through
# KMS (DSLOT_KMS=hybrid), the first frame after moving to GL. All but kernel->init are seconds
# from reset. With DSLOT_KMS=skeleton, "skeleton" is when the empty layout went up through KMS;
# with the power-on splash (rootfs/splash/), "splash" is when its picture went up.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
target=${1:-$DEVICE}
runs=${2:-3}
ssh_opts+=(-o ConnectTimeout=3)
tools=("$(device_tool counter)" "$(device_tool memdump)")

measure() {
    scp -q "${ssh_opts[@]}" "${tools[@]}" "$target:/tmp/"
    spl=$(spl_time "$target")
    ssh "${ssh_opts[@]}" "$target" "SPL='${spl:-0}' sh -s" <<'EOS'
n=0
until journalctl -b -u dslot -o cat --no-pager | grep -q "first frame shown" || [ $n -ge 60 ]; do sleep 0.5; n=$((n+1)); done
kernel=$(/tmp/counter | awk '{ print $NF }')
secs() { awk '{ print $2 == "ms" ? $1 / 1000 : $1 }'; }
sdkernel=$(systemd-analyze 2>/dev/null | sed -nE "s/.* in ([0-9.]+)(m?s) \(kernel\).*/\1 \2/p" | secs)
[ -n "$sdkernel" ] || sdkernel=$(systemctl show -p UserspaceTimestampMonotonic --value | awk '{ print $1 / 1e6 }')
unit=$(systemctl show -p ExecMainStartTimestampMonotonic dslot.service --value)
mark() { journalctl -b -u dslot -o cat --no-pager | sed -nE "s/^dslot: ([0-9.]+) s $1\$/\1/p" | head -1; }
# Started through KMS (DSLOT_KMS=hybrid), the shelf is shown again once it moves to GL.
gl=$(journalctl -b -u dslot -o cat --no-pager | sed -nE "s/^dslot: ([0-9.]+) s first frame shown\$/\1/p" | sed -n 2p)
awk -v k="$kernel" -v pre="$SPL" -v sk="$sdkernel" -v u="$unit" -v v="$(mark 'video ready')" \
    -v w="$(mark 'window and renderer ready')" -v g="$(mark 'games scanned')" -v f="$(mark 'first frame shown')" -v gl="$gl" -v skel="$(mark 'skeleton shown')" -v splash="$(cat /run/nibos/splash 2>/dev/null)" 'BEGIN {
    t = k + pre
    if (splash != "") printf "splash %5.2f  ", t + splash
    f = f == "" ? "  n/a" : sprintf("%5.2f", t + f)
    if (skel != "") printf "skeleton %5.2f  ", t + skel
    printf "kernel %5.2f  kernel->init %5.2f  unit %5.2f  video %5.2f  windows %5.2f  games %5.2f  | shelf %s  on GL %s\n",
        t, sk, t + u / 1e6, t + v, t + w, t + g, f, gl == "" ? "  -" : sprintf("%5.2f", t + gl) }'
EOS
}

for run in $(seq 1 "$runs"); do
    ssh "${ssh_opts[@]}" "$target" 'sudo systemctl reboot' >/dev/null 2>&1 || true
    sleep 8
    until ssh "${ssh_opts[@]}" "$target" true 2>/dev/null; do sleep 1; done
    printf 'run %d: ' "$run"
    measure
done
