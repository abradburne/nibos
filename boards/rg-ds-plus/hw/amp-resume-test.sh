#!/bin/bash
# amp-resume-test.sh [CYCLES] [SLEEP_S]: on the device, as root, with nothing playing a game.
# Suspends CYCLES times (default 10), woken by the RTC after SLEEP_S seconds (default 8), and
# checks after each resume that both speaker amps run again: SYSCTRL's AMPPD and HMUTE clear,
# SYSST's PLL locked (docs/engineering/audio.md, "The amps after suspend"). Exits 1 on a failure.
#
#   scp boards/rg-ds-plus/hw/amp-resume-test.sh DEVICE: && ssh DEVICE sudo ./amp-resume-test.sh
n=${1:-10} s=${2:-8} fail=0
reg() { awk -v r="$2:" '$1 == r { print $2 }' "/sys/kernel/debug/regmap/$1/registers"; }
for i in $(seq "$n"); do
    pgrep -x "dsperate|inkdot" >/dev/null && { echo "a game is running: stop"; exit 2; }
    since=$(date +%s)
    echo 0 > /sys/class/rtc/rtc0/wakealarm
    echo "+$s" > /sys/class/rtc/rtc0/wakealarm
    systemctl suspend
    # Wait for the resume, then for the stream to come back.
    for _ in $(seq 60); do
        journalctl -k --since @"$since" --no-pager | grep -q "PM: suspend exit" && break
        sleep 1
    done
    sleep 3
    line="cycle $i:"
    for a in 2-0034 2-0035; do
        c=$(reg $a 04) st=$(reg $a 01)
        ok=yes
        (( 0x$c & 0x0102 )) && ok=no      # AMPPD (bit 1) or HMUTE (bit 8) still set
        (( 0x$st & 0x0300 )) || ok=no     # SYSST: PLL and clocks not locked
        [ $ok = yes ] || fail=$((fail + 1))
        line+=" $a SYSCTRL=$c SYSST=$st $ok;"
    done
    starts=$(journalctl -k --since @"$since" --no-pager | grep -c "device start failed")
    pcm=$(head -1 /proc/asound/speakers/pcm0p/sub0/status)
    echo "$line failed-tries=$starts pcm=[$pcm]"
done
echo "failures: $fail"
[ "$fail" = 0 ]
