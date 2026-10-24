#!/bin/bash
# Boot-time benchmark for the three network modes (hw/README.md), for when the
# device can't be reached over SSH while it measures itself.
#
#   bench/network.sh boot|deferred|demand [runs]
#
# Sets `nib net mode`, arms a one-shot recorder, reboots. The recorder waits
# for the boot to finish, writes arch-counter-based figures to
# /var/lib/nibos-bench/result, then brings the network up (mode demand: it
# asks for it, and times it) so this script can read the file over SSH.
# Same arithmetic as bench/boot.sh. Leaves the device in the mode it found.
#
# Take the device lock first (tools/device-lock.sh).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
mode=${1:-}
[ -n "$mode" ] || usage
runs=${2:-3}
target=${TARGET:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=3)
sshd() { ssh "${ssh_opts[@]}" "$target" "$@"; }

original=$(sshd 'nib net status' | sed -n 's/^mode: //p')

scp -q "${ssh_opts[@]}" "$(device_tool counter)" "$(device_tool memdump)" "$target:/tmp/"
sshd 'sudo sh -s' <<'REMOTE'
install -d /var/lib/nibos-bench
install -m755 /tmp/counter /tmp/memdump /var/lib/nibos-bench/
cat > /var/lib/nibos-bench/record.sh <<'RECORD'
#!/bin/sh
d=/var/lib/nibos-bench
rm -f $d/arm $d/result
while [ "$(systemctl is-system-running)" = starting ] || [ "$(systemctl is-system-running)" = initializing ]; do sleep 0.2; done
kernel=$($d/counter | awk '{ print $NF }')
sdkernel=$(systemd-analyze | sed -nE "s/.* in ([0-9.]+)s \(kernel\).*/\1/p")
user=$(systemd-analyze | sed -nE "s/.*\+ ([0-9.]+)s \(userspace\).*/\1/p")
login=$(systemctl show -p ActiveEnterTimestampMonotonic getty@tty1.service --value)
if systemctl -q is-active sshd.service; then unit=sshd.service; else unit=sshd.socket; fi
ssh=$(systemctl show -p ActiveEnterTimestampMonotonic $unit --value)
net=$(journalctl -b -u iwd -o short-monotonic --no-pager 2>/dev/null | sed -nE 's/^\[ *([0-9.]+)\].*new: connected$/\1/p' | head -1)
mode=$(nib net status | sed -n 's/^mode: //p')
now() { cut -d' ' -f1 /proc/uptime; }
done_at=$(now)
asked=
if [ "$mode" = demand ]; then asked=$(now); nib net up; fi
n=0
until ip -4 -o addr show wlan0 2>/dev/null | grep -q inet || [ $n -ge 600 ]; do sleep 0.1; n=$((n+1)); done
addr=$(now)
echo "$kernel $sdkernel $user $login $ssh $net $done_at ${asked:-0} $addr" > $d/result
RECORD
chmod 755 /var/lib/nibos-bench/record.sh
cat > /etc/systemd/system/nibos-bench-record.service <<'UNIT'
[Unit]
Description=Record boot times (hw/bench-network.sh)
After=multi-user.target
ConditionPathExists=/var/lib/nibos-bench/arm

[Service]
Type=simple
ExecStart=/var/lib/nibos-bench/record.sh

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable nibos-bench-record.service >/dev/null 2>&1
REMOTE

for run in $(seq 1 "$runs"); do
    sshd "sudo nib net mode $mode; sudo touch /var/lib/nibos-bench/arm; sudo systemctl reboot" >/dev/null 2>&1 || true
    sleep 8
    until sshd 'test -s /var/lib/nibos-bench/result' 2>/dev/null; do sleep 2; done
    spl=$(spl_time "$target" /var/lib/nibos-bench/memdump)
    sshd 'cat /var/lib/nibos-bench/result' | awk -v spl="${spl:-0}" -v mode="$mode" -v run="$run" '{
        k = $1 + spl
        printf "run %d (%s): kernel %.2f  kernel->init %.2f  userspace %.2f | console login %.2f  booted %.2f", run, mode, k, $2, $3, k + $4 / 1e6, k + $2 + $3
        if (mode == "boot") { t = $5 / 1e6; if ($6 > t) t = $6; printf "  ssh reachable %.2f", k + t }
        if (mode == "deferred") printf "  wlan0 has an address %.2f (%.2f after login)", k + $9, $9 - $4 / 1e6
        if (mode == "demand") printf "  network asked for %.2f, address after %.2f s", k + $8, $9 - $8
        printf "\n" }'
done

sshd "sudo nib net mode ${original:-boot}; sudo systemctl disable nibos-bench-record.service >/dev/null 2>&1; sudo rm -f /etc/systemd/system/nibos-bench-record.service; sudo systemctl daemon-reload" >/dev/null
