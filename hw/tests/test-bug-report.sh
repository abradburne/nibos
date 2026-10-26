#!/bin/sh
# The scrub in nib bug-report (hw/files/usr/lib/nibos/nib-bug-report) on a sample of what the
# boot log holds: private parts out, the rest untouched. Runs anywhere with sed (GNU or BSD).
#
#   hw/tests/test-bug-report.sh
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
script="$here/../files/usr/lib/nibos/nib-bug-report"
fail=0
check() {
    # $1 what, $2 input line, $3 expected output
    got=$(printf '%s\n' "$2" | sh "$script" scrub)
    if [ "$got" = "$3" ]; then echo "ok: $1"; else echo "FAIL: $1"; echo "  got:  $got"; echo "  want: $3"; fail=1; fi
}
check "a MAC address" \
    '    link/ether b4:65:dc:72:e7:7d brd ff:ff:ff:ff:ff:ff' \
    '    link/ether xx:xx:xx:xx:xx:xx brd xx:xx:xx:xx:xx:xx'
check "an IPv4 address with its prefix" \
    '    inet 192.168.68.74/22 scope global dynamic noprefixroute wlan0' \
    '    inet [ip] scope global dynamic noprefixroute wlan0'
check "a link-local IPv6 address" \
    '    inet6 fe80::b665:dcff:fe72:e77d/64 scope link proto kernel_ll' \
    '    inet6 [ipv6]/64 scope link proto kernel_ll'
check "iwd's network name and access point" \
    'Oct 26 11:25:00 nibos iwd[445]: event: connect-info, ssid: Endless, bss: 78:8c:b5:7d:76:73, signal: -48, load: 0/255' \
    'Oct 26 11:25:00 nibos iwd[445]: event: connect-info, ssid: [ssid], bss: xx:xx:xx:xx:xx:xx, signal: -48, load: 0/255'
check "a passphrase" \
    'Passphrase=correct horse battery' \
    'Passphrase=[hidden] horse battery'
check "a time of day is not an address" \
    '[    7.228736] nibos dslot[257]: dslot: 7.228 s Lynx games scanned at 11:25:16' \
    '[    7.228736] nibos dslot[257]: dslot: 7.228 s Lynx games scanned at 11:25:16'
check "a kernel version is not an address" \
    'Linux nibos 7.3.0-rc7-nibos-g0fcb05b72344-dirty #4 SMP PREEMPT aarch64 GNU/Linux' \
    'Linux nibos 7.3.0-rc7-nibos-g0fcb05b72344-dirty #4 SMP PREEMPT aarch64 GNU/Linux'
check "a game's file name stays" \
    'dslot: starting /usr/bin/dsperate --saves /mnt/roms/Saves/NDS/Advance Wars - Dark Conflict [EUR] /mnt/roms/Roms/NDS/Advance Wars - Dark Conflict [EUR].nds' \
    'dslot: starting /usr/bin/dsperate --saves /mnt/roms/Saves/NDS/Advance Wars - Dark Conflict [EUR] /mnt/roms/Roms/NDS/Advance Wars - Dark Conflict [EUR].nds'
check "the hostname systemd reports" \
    '[    1.267042] systemd[1]: Hostname set to <alans-ds>.' \
    '[    1.267042] systemd[1]: Hostname set to <[host]>.'
check "a loopback address" \
    '    inet 127.0.0.1/8 scope host lo' \
    '    inet [ip] scope host lo'
check "nibOS's own hostname in the journal stays" \
    'Oct 26 11:25:00 nibos dslot[257]: dslot: 7.228 s started' \
    'Oct 26 11:25:00 nibos dslot[257]: dslot: 7.228 s started'
exit $fail
