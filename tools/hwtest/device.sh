#!/bin/bash
# The device side of tools/hwtest.sh: runs the checks and prints one line each,
# "AREA|CHECK|PASS|detail" (or FAIL, SKIP), then nothing else. Runs as root (sudo), with the
# board's board.conf and the hwtest profile (HWTEST_PROFILE). Every check is bounded.
#
#   HWTEST_PROFILE=... HWTEST_AREAS="boot display ..." HWTEST_SOUND=0|1 HWTEST_SUSPEND=0|N
#   HWTEST_WIFI_TOGGLE=0|N device.sh
# The checks are small sh -c scripts, expanded on the device: single quotes on purpose.
# shellcheck disable=SC2016
set -u
# shellcheck disable=SC1091
. /usr/lib/nibos/board.conf
# shellcheck disable=SC1090
. "${HWTEST_PROFILE:?}"
areas=" ${HWTEST_AREAS:-boot display gpu audio input storage radios power suspend charging} "
user=$(id -nu 1000 2>/dev/null || echo nibos)

r() { printf '%s|%s|%s|%s\n' "$1" "$2" "$3" "${4:-}"; }
# check AREA NAME COMMAND...: PASS when the command succeeds within 10 s, its last line the
# detail; FAIL otherwise.
check() {
    local area=$1 name=$2 out
    shift 2
    if out=$(timeout 10 "$@" 2>&1); then r "$area" "$name" PASS "$(tail -1 <<<"$out")"
    else r "$area" "$name" FAIL "$(tail -1 <<<"$out")"; fi
}
want() { case "$areas" in *" $1 "*) return 0 ;; esac; return 1; }

if want boot; then
    check boot "nibOS kernel" sh -c 'uname -r | grep nibos'
    check boot "no failed units" sh -c 'f=$(systemctl --failed --no-legend --plain | awk "{print \$1}" | tr "\n" " "); [ -z "$f" ] && echo none || { echo "failed: $f"; false; }'
    check boot "boot UI up" sh -c 'if systemctl -q is-enabled dslot.service; then systemctl -q is-active dslot && journalctl -b -u dslot -o cat | grep -m1 "first frame shown"; else systemctl -q is-active getty@tty1 && echo "console"; fi'
    check boot "boot picture" sh -c 'if ! grep -q "init=/usr/lib/nibos/splash" /proc/cmdline; then echo "splash off"; elif journalctl -k -b -t nibos-splash -o cat | grep -m1 "picture on\|charges asleep"; then :; else echo "init= set, no picture logged"; false; fi'
    check boot "kernel warnings" sh -c 'n=$(journalctl -k -b -o cat | grep -c "cut here"); echo "$n WARNINGs (journalctl -k -b)"; [ "$n" -eq 0 ]'
    check boot "kernel errors" sh -c 'n=$(journalctl -k -b -p err -o cat | grep -vc "aw88166\|check pll\|iis signal\|start failed\|pll check\|mode2 check"); echo "$n besides the amps start retries"; [ "$n" -le 5 ]'
fi

if want display; then
    for c in $HWTEST_CONNECTORS; do
        check display "$c" sh -c "f=\$(ls -d /sys/class/drm/card*-$c | head -1); [ \"\$(cat \$f/status)\" = connected ] && head -1 \$f/modes | grep -x '$HWTEST_MODE'"
    done
    check display "KMS driver" sh -c "readlink /sys/class/drm/card0/device/driver | grep -o '$HWTEST_KMS_DRIVER\$'"
    for pair in $NIBOS_BACKLIGHTS; do
        b=${pair#*:}
        check display "backlight ${pair%%:*}" sh -c "v=\$(cat /sys/class/backlight/$b/brightness); m=\$(cat /sys/class/backlight/$b/max_brightness); echo \"$b \$v/\$m\"; [ \"\$v\" -gt 0 ]"
    done
fi

if want gpu; then
    check gpu "render node" sh -c "readlink /sys/class/drm/renderD128/device/driver | grep -o '$HWTEST_GPU_DRIVER\$'"
    check gpu "devfreq" sh -c 'd=$(ls -d /sys/class/devfreq/*gpu* | head -1); echo "$(cat $d/governor) $(($(cat $d/cur_freq)/1000000)) MHz"'
    check gpu "no GPU faults" sh -c 'n=$(journalctl -k -b -o cat | grep -ci "panfrost.*\(fault\|timeout\|reset\)"); echo "$n faults"; [ "$n" -eq 0 ]'
fi

if want audio; then
    for c in $HWTEST_SOUND_CARDS; do
        check audio "card $c" sh -c "grep -q '\[$c' /proc/asound/cards && echo present"
    done
    check audio "PipeWire sinks" sh -c "s=\$(as_user() { sudo -u $user XDG_RUNTIME_DIR=/run/user/\$(id -u $user) \"\$@\"; }; as_user pactl list short sinks | awk '{print \$2}' | tr '\n' ' '); echo \"\$s\"; echo \"\$s\" | grep -q '${NIBOS_SPEAKERS_SINK:-.}'"
    for a in $HWTEST_AMPS; do
        check audio "amp $a running" sh -c "c=\$(awk '\$1==\"04:\"{print \$2}' /sys/kernel/debug/regmap/$a/registers); st=\$(awk '\$1==\"01:\"{print \$2}' /sys/kernel/debug/regmap/$a/registers); echo \"SYSCTRL \$c SYSST \$st\"; [ \$((0x\$c & 0x0102)) -eq 0 ] && [ \$((0x\$st & 0x0300)) -ne 0 ]"
    done
    check audio "jack switch" sh -c "[ -n \"\$(grep -lx '$NIBOS_JACK_DEVICE' /sys/class/input/event*/device/name)\" ] && echo '$NIBOS_JACK_DEVICE'"
    if [ "${HWTEST_SOUND:-0}" = 1 ]; then
        check audio "a tone on the speakers" sh -c "as_user() { sudo -u $user XDG_RUNTIME_DIR=/run/user/\$(id -u $user) \"\$@\"; }; as_user pw-play --volume=0.2 /usr/share/sounds/freedesktop/stereo/bell.oga 2>/dev/null || as_user speaker-test -t sine -f 440 -l 1 -p 2 >/dev/null; echo played"
    else
        r audio "a tone on the speakers" SKIP "--sound to play one (ask first)"
    fi
fi

if want input; then
    for d in "$NIBOS_BUTTONS_DEVICE" "${NIBOS_STICK_DEVICE:-}" "${NIBOS_VOLUME_DEVICE:-}" "${NIBOS_POWER_DEVICE:-}" "${NIBOS_LID_DEVICE:-}" "$HWTEST_TOUCH" "$NIBOS_PAD_NAME" "nibOS volume keys"; do
        [ -n "$d" ] || continue
        check input "$d" sh -c "e=\$(grep -lx '$d' /sys/class/input/event*/device/name | head -1); [ -n \"\$e\" ] && echo \"\$(basename \$(dirname \$(dirname \$e)))\""
    done
    check input "nibos-input" systemctl is-active nibos-input
fi

if want storage; then
    check storage "root" sh -c 'findmnt -no SOURCE,FSTYPE,OPTIONS / | tee /dev/stderr | grep -q "ext4 rw"'
    check storage "SD card speed mode" sh -c "grep -h 'timing spec' /sys/kernel/debug/mmc*/ios | grep -m1 '$HWTEST_SD_TIMING'"
    check storage "read 64 MiB" sh -c 'd=$(findmnt -no SOURCE / | sed "s/p[0-9]*$//"); sync; echo 1 > /proc/sys/vm/drop_caches; t0=$(date +%s.%N); dd if=$d of=/dev/null bs=4M count=16 skip=200 status=none; t1=$(date +%s.%N); awk -v a=$t0 -v b=$t1 "BEGIN{r=64/(b-a); printf \"%.0f MB/s\n\", r; exit !(r > 20)}"'
    check storage "space on root" sh -c 'df -h --output=avail / | tail -1 | tr -d " "'
fi

if want radios; then
    if [ -e "/sys/class/net/$HWTEST_WIFI" ]; then
        check radios "Wi-Fi" sh -c "iwctl station $HWTEST_WIFI show | awk '/State/{print \$2}' | grep -x connected"
    else
        r radios "Wi-Fi" SKIP "no $HWTEST_WIFI (network on demand: nib net)"
    fi
    if [ -e "/sys/class/bluetooth/$HWTEST_BLUETOOTH" ]; then check radios "Bluetooth" sh -c "echo $HWTEST_BLUETOOTH"
    else r radios "Bluetooth" SKIP "no $HWTEST_BLUETOOTH (radios on demand)"; fi
    n=${HWTEST_WIFI_TOGGLE:-0}
    if [ "$n" -gt 0 ] && [ -e "/sys/class/net/$HWTEST_WIFI" ]; then
        state() { iwctl station "$HWTEST_WIFI" show 2>/dev/null | awk '/ State /{ print $2 }'; }
        # Seconds until connected, or nothing after LIMIT seconds.
        connected_in() { for i in $(seq "$1"); do [ "$(state)" = connected ] && { echo "$i"; return 0; }; sleep 1; done; return 1; }
        for i in $(seq "$n"); do
            nib net down; sleep 2; nib net up
            if t=$(connected_in 40); then r radios "Wi-Fi off and on ($i)" PASS "connected in ${t} s"
            else r radios "Wi-Fi off and on ($i)" FAIL "not connected after 40 s ($(state))"; fi
        done
        # The Wi-Fi switch alone (nib wifi): Bluetooth stays as it was.
        bt() { bluetoothctl show 2>/dev/null | awk '/Powered:/{ print $2 }'; }
        was=$(bt); bluetoothctl power on >/dev/null 2>&1; sleep 2
        nib wifi off; sleep 3
        if [ "$(bt)" = yes ] && [ "$(cat "/sys/class/net/$HWTEST_WIFI/operstate" 2>/dev/null)" != up ]; then
            r radios "nib wifi off leaves Bluetooth on" PASS "wlan0 down, Bluetooth powered"
        else r radios "nib wifi off leaves Bluetooth on" FAIL "Bluetooth powered: $(bt)"; fi
        nib wifi on
        if t=$(connected_in 40) && [ "$(bt)" = yes ]; then r radios "nib wifi on, Bluetooth still on" PASS "connected in ${t} s"
        else r radios "nib wifi on, Bluetooth still on" FAIL "Wi-Fi $(state), Bluetooth powered: $(bt)"; nib wifi on; fi
        [ "$was" = yes ] || bluetoothctl power off >/dev/null 2>&1
        # Asleep while connecting (2026-10-21: iwd stayed "connecting" until a restart).
        nib net down; sleep 2; since=$(date +%s); nib net up
        for _ in $(seq 150); do journalctl -u iwd --since "@$since" -o cat | grep -q "new: connecting" && break; sleep 0.1; done
        echo 0 > /sys/class/rtc/rtc0/wakealarm; echo +8 > /sys/class/rtc/rtc0/wakealarm
        systemctl suspend
        for _ in $(seq 40); do journalctl -k --since "@$since" -o cat | grep -q "PM: suspend exit" && break; sleep 1; done
        if t=$(connected_in 60); then r radios "Wi-Fi after sleeping while connecting" PASS "connected ${t} s after waking"
        else r radios "Wi-Fi after sleeping while connecting" FAIL "not connected 60 s after waking ($(state))"; nib net up; systemctl restart iwd; fi
    elif [ "$n" -gt 0 ]; then
        r radios "Wi-Fi off and on" SKIP "no $HWTEST_WIFI"
    fi
fi

if want power; then
    check power "battery" sh -c "c=\$(cat /sys/class/power_supply/$NIBOS_BATTERY/capacity); s=\$(cat /sys/class/power_supply/$NIBOS_BATTERY/status); echo \"\$c% \$s\"; [ \"\$c\" -ge 0 ] && [ \"\$c\" -le 100 ]"
    check power "charger" sh -c "o=\$(cat /sys/class/power_supply/$NIBOS_CHARGER/online); echo \"online \$o\""
    for z in $HWTEST_THERMAL; do
        check power "temperature $z" sh -c "for t in /sys/class/thermal/thermal_zone*; do [ \"\$(cat \$t/type)\" = $z ] && { v=\$((\$(cat \$t/temp)/1000)); echo \"\$v C\"; [ \$v -lt 80 ]; exit; }; done; false"
    done
    check power "watchdog" sh -c "readlink /sys/class/watchdog/watchdog0/device/driver | grep -q '$HWTEST_WATCHDOG\$' && systemctl show -p RuntimeWatchdogUSec --value | grep -v '^0$'"
    check power "panic reboots" sh -c 'p=$(cat /proc/sys/kernel/panic); echo "panic=$p"; [ "$p" -gt 0 ]'
    check power "cpufreq" sh -c 'echo "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor) $(($(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq)/1000)) MHz"'
fi

if want suspend; then
    n=${HWTEST_SUSPEND:-0}
    if [ "$n" -gt 0 ]; then
        ok=0
        for _ in $(seq "$n"); do
            since=$(date +%s)
            echo 0 > /sys/class/rtc/rtc0/wakealarm; echo +8 > /sys/class/rtc/rtc0/wakealarm
            systemctl suspend
            for _ in $(seq 40); do journalctl -k --since "@$since" -o cat | grep -q "PM: suspend exit" && break; sleep 1; done
            journalctl -k --since "@$since" -o cat | grep -q "PM: suspend exit" && ok=$((ok + 1))
            sleep 4
        done
        if [ "$ok" = "$n" ]; then r suspend "RTC-woken suspends" PASS "$ok of $n"
        else r suspend "RTC-woken suspends" FAIL "$ok of $n"; fi
        for a in $HWTEST_AMPS; do
            check suspend "amp $a after waking" sh -c "c=\$(awk '\$1==\"04:\"{print \$2}' /sys/kernel/debug/regmap/$a/registers); [ \$((0x\$c & 0x0102)) -eq 0 ] && echo \"SYSCTRL \$c\""
        done
        if [ -e "/sys/class/net/$HWTEST_WIFI" ]; then
            check suspend "Wi-Fi back" sh -c "for _ in \$(seq 9); do iwctl station $HWTEST_WIFI show | grep -q 'State.*connected' && { echo connected; exit 0; }; sleep 1; done; false"
        fi
    else
        r suspend "RTC-woken suspends" SKIP "--suspend N to run N"
    fi
fi

if want charging; then
    r charging "unplug awake, unplug asleep" SKIP "needs hands: docs/engineering/battery.md, 'Testing a charging change'"
    r charging "charge-sleep (plug in while off)" SKIP "needs hands: hw/README.md, 'Charging while off'"
fi
