#!/bin/bash
# Boot-time benchmark: restart the DS+ over SSH and report where the time went.
#
#   bench/boot.sh [user@host] [runs]
#
# Times come from the arch counter, which TF-A (BL31, the same v1.46 under the
# stock and the mainline bootloader) restarts: "kernel" is when Linux started
# on that counter (counter minus CLOCK_BOOTTIME), and systemd's CLOCK_MONOTONIC
# stamps count from Linux's start. Under mainline U-Boot the time before TF-A
# (DDR init and SPL, ~0.32 s) is in the bootstage stash, and "from reset" adds
# it; under the stock bootloader that part is unknown and left out: run with
# STOCK=1 (memory keeps an earlier mainline boot's stash across restarts).
#
# Not journald's kernel stamps: lines logged before journald starts get made-up
# times (dmesg 0.12 s shows as 3.47 s).
#
# "console" is when the autologin shell on tty1 started (its start time in
# /proc/PID/stat), the prompt you can type at; "getty" is when systemd started
# getty@tty1, which can be seconds earlier (login waits for logind and the user
# manager). "ssh" is when a client can connect: sshd listening (sshd.socket, or
# sshd.service if that's what runs) and iwd connected with an address.
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
    # TPL+SPL from the stash ("end phase"), when mainline U-Boot left one.
    spl=
    [ "${STOCK:-0}" = 1 ] || spl=$(spl_time "$target")
    ssh "${ssh_opts[@]}" "$target" "SPL='${spl:-}' sh -s" <<'EOF'
until systemctl is-system-running --quiet 2>/dev/null || [ "$(systemctl is-system-running)" = degraded ]; do sleep 0.5; done
kernel=$(/tmp/counter | awk '{ print $NF }')
# systemd-analyze prints times under a second in ms ("656ms (kernel)").
secs() { awk '{ print $2 == "ms" ? $1 / 1000 : $1 }'; }
sdkernel=$(systemd-analyze 2>/dev/null | sed -nE "s/.* in ([0-9.]+)(m?s) \(kernel\).*/\1 \2/p" | secs)
user=$(systemd-analyze 2>/dev/null | sed -nE "s/.*\+ ([0-9.]+)(m?s) \(userspace\).*/\1 \2/p" | secs)
login=$(systemctl show -p ActiveEnterTimestampMonotonic getty@tty1.service --value)
n=0
until shell=$(pgrep -o -t tty1 -x bash) || [ $n -ge 60 ]; do sleep 0.5; n=$((n+1)); done
shell=$([ -n "$shell" ] && awk -v hz="$(getconf CLK_TCK)" '{ print $22 / hz }' "/proc/$shell/stat")
if systemctl -q is-active sshd.service; then unit=sshd.service; else unit=sshd.socket; fi
ssh=$(systemctl show -p ActiveEnterTimestampMonotonic $unit --value)
net=$(journalctl -b -u iwd -o short-monotonic --no-pager 2>/dev/null | sed -nE 's/^\[ *([0-9.]+)\].*new: connected$/\1/p' | head -1)
awk -v k="$kernel" -v sk="$sdkernel" -v u="$user" -v l="$login" -v sh="$shell" -v s="$ssh" -v net="$net" -v spl="$SPL" 'BEGIN {
    pre = spl == "" ? 0 : spl
    s = s / 1e6; if (net == "") s = -1; else if (net > s) s = net
    printf "%s kernel %5.2f  kernel->init %5.2f  userspace %5.2f  | getty %5.2f  console %s  ssh %s  booted %5.2f\n",
        spl == "" ? "since TF-A:" : sprintf("from reset (TPL+SPL %.2f):", spl),
        k + pre, sk, u, k + pre + l / 1e6,
        sh == "" ? "  n/a" : sprintf("%5.2f", k + pre + sh),
        s < 0 ? "  n/a" : sprintf("%5.2f", k + pre + s), k + pre + sk + u }'
EOF
}

for run in $(seq 1 "$runs"); do
    ssh "${ssh_opts[@]}" "$target" 'sudo systemctl reboot' >/dev/null 2>&1 || true
    sleep 8
    until ssh "${ssh_opts[@]}" "$target" true 2>/dev/null; do sleep 1; done
    printf 'run %d: ' "$run"
    measure
done
