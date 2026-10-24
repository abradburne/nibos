#!/bin/bash
# probe.sh — report a stock RG DS+'s hardware (RAM, amps, touch, lid, backlights, USB-C
# host, suspend) from its stock firmware.
#
# From the stock menu: copy tools/stock-probe/Roms/APPS/* into the card's Roms/APPS, then
# choose "nibos_probe" in Apps. Instructions appear on both screens; answer
# with the shoulder buttons. The report is saved beside this script, in
# Roms/APPS/nibos/probe-<time>.txt.
#
# Over SSH also works:  ssh -t root@<device> bash probe.sh [--batch] [--suspend SECONDS]
#
# Everything is read-only except: each backlight is turned off for 2 s and
# restored (to tell top from bottom), and the optional sleep test suspends the
# device for 5 minutes with rtcwake.

# No set -e: a probe reports what fails and goes on to the next check.
HERE="$(cd "$(dirname "$0")" && pwd)"
MODE='batch'
[ -t 0 ] && MODE='tty'
SUSPEND_SECS=0
while [ $# -gt 0 ]; do
    case "$1" in
        --screen) MODE='screen' ;;
        --batch) MODE='batch' ;;
        --suspend) SUSPEND_SECS="$2"; shift ;;
        *) echo "usage: $0 [--screen|--batch] [--suspend SECONDS]"; exit 2 ;;
    esac
    shift
done
INTERACTIVE=1
[ "$MODE" = batch ] && INTERACTIVE=0

STAMP=$(date +%Y%m%d-%H%M%S)
if [ "$MODE" = screen ]; then
    OUT="$HERE/probe-$STAMP.txt"
    SCREEN=/tmp/nibos-probe.screen
    : > "$SCREEN"
    python3 "$HERE/progress.py" "nibos probe" "$SCREEN" 2> /dev/null &
    SHOWN=$!
    trap 'kill $SHOWN 2> /dev/null' EXIT
    exec > "$OUT" 2>&1
else
    if [ -d /mnt/sdcard ] && [ -w /mnt/sdcard ]; then OUTDIR=/mnt/sdcard; else OUTDIR=/tmp; fi
    OUT="$OUTDIR/nibos-probe-$STAMP.txt"
    exec > >(tee "$OUT") 2>&1
fi

section() {
    printf '\n==== %s ====\n' "$1"
    [ "$MODE" = screen ] && echo "checking: $1" >> "$SCREEN"
}
say() { [ "$MODE" = screen ] && echo "$1" >> "$SCREEN"; }
have() { command -v "$1" >/dev/null 2>&1; }
dt_u32() { od -An -tx1 "$1" 2>/dev/null | tr -d ' \n'; }

# Button codes come from the device tree: Anbernic's codes don't follow the
# usual BTN_* layout (L1 is BTN_WEST, Start is BTN_TR, ...).
dt_code() { local h; h=$(dt_u32 "/proc/device-tree/gpio-keys-polled/$1/linux,code"); [ -n "$h" ] && echo $((16#$h)); }
KEYDEV=$(awk '/^N: Name=/{n=$0} /^H: Handlers=/{ if (n ~ /ANBERNIC-rk3568-keys/) { match($0, /event[0-9]+/); print "/dev/input/" substr($0, RSTART, RLENGTH) } }' /proc/bus/input/devices)
# Read through eval in choose().
# shellcheck disable=SC2034
BTN_L1=$(dt_code keyL1)
# shellcheck disable=SC2034
BTN_R1=$(dt_code keyR1)
# shellcheck disable=SC2034
BTN_START=$(dt_code keyStart)

# choose "question" "label:BUTTON ..." -> REPLY=label. On screen it waits for
# one of the buttons; over SSH you type the label.
choose() {
    local q="$1" spec="$2" p codes line names=""
    REPLY=""
    [ "$INTERACTIVE" = 1 ] || return 1
    for p in $spec; do names="$names ${p#*:}=${p%%:*}"; done
    if [ "$MODE" = tty ]; then
        printf '\n>>> %s [%s ] ' "$q" "$names" > /dev/tty
        read -r REPLY < /dev/tty
        return 0
    fi
    say ""; say "$q"; say "press:$names"
    if [ -z "$KEYDEV" ]; then
        say "(no button device found: carrying on in 10 s)"; sleep 10; REPLY=timeout; return 0
    fi
    codes=""
    for p in $spec; do eval "codes=\"\$codes|\$BTN_${p#*:}\""; done
    line=$(timeout 180 stdbuf -oL evtest "$KEYDEV" 2> /dev/null | grep -m1 -E "type 1 \(EV_KEY\), code (${codes#|}) .*value 1")
    REPLY=timeout
    for p in $spec; do
        eval "c=\$BTN_${p#*:}"
        case "$line" in *"code $c "*) REPLY=${p%%:*} ;; esac
    done
    say "-> $REPLY"
}

# Watch every input device for SECS seconds and print what each one reported.
watch_inputs() {
    local secs="$1" dir ev name pids=()
    dir=$(mktemp -d)
    say "watching for $secs s..."
    for ev in /dev/input/event*; do
        timeout "$secs" evtest "$ev" > "$dir/$(basename "$ev")" 2>&1 &
        pids+=($!)
    done
    # Wait for these only: a bare wait would also wait for progress.py, which never ends.
    wait "${pids[@]}"
    for f in "$dir"/event*; do
        name=$(sed -n 's/^Input device name: //p' "$f")
        if grep -q '^Event: time.*type [1-5] ' "$f"; then
            echo "  $(basename "$f") ($name):"
            grep '^Event: time.*type [1-5] ' "$f" | sed 's/^Event: time [0-9.]*, /    /' | sort | uniq -c | sort -rn | head -8
        fi
    done
    rm -rf "$dir"
}

echo "nibos probe — $(date)"
echo "report: $OUT"

section "System"
uname -a
cat /proc/device-tree/model 2>/dev/null; echo
cat /proc/device-tree/compatible 2>/dev/null | tr '\0' ' '; echo
echo "cmdline: $(cat /proc/cmdline)"
[ -f /etc/os-release ] && grep -E '^(NAME|VERSION|BUILD)' /etc/os-release

section "1. RAM"
free -m
grep -E 'MemTotal|CmaTotal|MemAvailable' /proc/meminfo
echo "CPU max freqs (kHz): $(cat /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq 2>/dev/null | sort -u | tr '\n' ' ')"
df -h 2>/dev/null | grep -E '^/dev|Filesystem'

section "2. Speaker amp chip ID (aw883xx on i2c@fe5b0000)"
AMP_BUS=""
for b in /sys/bus/i2c/devices/i2c-*; do
    case "$(readlink -f "$b/of_node" 2>/dev/null)" in *fe5b0000*) AMP_BUS=${b##*i2c-} ;; esac
done
echo "amp bus: i2c-${AMP_BUS:-not found}"
for addr in 34 35; do
    d="/sys/bus/i2c/devices/${AMP_BUS}-00$addr"
    echo "--- 0x$addr: name=$(cat "$d/name" 2>/dev/null) driver=$(basename "$(readlink "$d/driver" 2>/dev/null)" 2>/dev/null)"
    if [ -n "$AMP_BUS" ] && have i2cget; then
        # Reg 0x00 is the 16-bit chip ID on Awinic amps. -f because the vendor
        # driver owns the address; a single register read is harmless.
        # SMBus words are little-endian, so 0x4920 means ID 0x2049.
        for reg in 0x00 0x01; do
            echo "    reg $reg word: $(i2cget -f -y "$AMP_BUS" 0x$addr $reg w 2>&1)  byte: $(i2cget -f -y "$AMP_BUS" 0x$addr $reg 2>&1)"
        done
    fi
done
echo "known IDs (verify against mainline sources): aw88395=0x2049 aw88261=0x2113 aw88399=0x2183"
echo "--- kernel log:"
dmesg 2>/dev/null | grep -iE 'aw88|aw87|awinic|smartpa' | head -30
for d in /sys/bus/i2c/drivers/*[aA][wW]*; do [ -e "$d" ] && basename "$d"; done
echo "sound cards:"; cat /proc/asound/cards 2>/dev/null

section "3. Touch"
grep -A8 -iE '^N: Name=.*(goodix|gt9|touch|ts)' /proc/bus/input/devices
echo "all input devices:"
grep '^N: Name' /proc/bus/input/devices
echo "--- kernel log:"
dmesg 2>/dev/null | grep -iE 'goodix|gt9|gtp|touch' | head -20
echo "i2c devices on touch bus:"
for b in /sys/bus/i2c/devices/i2c-*; do
    case "$(readlink -f "$b/of_node" 2>/dev/null)" in
        *fe5e0000*) for c in "$b"/[0-9]*-*; do [ -e "$c" ] && echo "  ${c##*/} $(cat "$c/name")"; done ;;
    esac
done

section "4. Displays: which DSI is the top screen"
for c in /sys/class/drm/card*-*; do
    [ -f "$c/status" ] && echo "  $(basename "$c"): $(cat "$c/status") $(head -1 "$c/modes" 2>/dev/null)"
done
for dsi in dsi@fe060000 dsi@fe070000; do
    echo "  $dsi panel -> backlight phandle $(dt_u32 /proc/device-tree/$dsi/panel@0/backlight)"
done
for bl in backlight backlight1; do
    echo "  /$bl phandle $(dt_u32 /proc/device-tree/$bl/phandle)"
done
for b in /sys/class/backlight/*; do
    [ -d "$b" ] || continue
    echo "  sysfs $(basename "$b"): of_node=$(readlink -f "$b/device/of_node" 2>/dev/null) brightness=$(cat "$b/brightness")/$(cat "$b/max_brightness")"
done
if [ "$INTERACTIVE" = 1 ]; then
    for b in /sys/class/backlight/*; do
        [ -d "$b" ] || continue
        saved=$(cat "$b/brightness")
        echo 0 > "$b/brightness"; sleep 2; echo "$saved" > "$b/brightness"
        choose "Which screen just went dark for 2 s?" "top:L1 bottom:R1 neither:START"
        echo "  $(basename "$b") -> $REPLY"
    done
    choose "Next: keep touching and dragging on the touch screen for 6 s." "go:START"
    echo "  input during touch test:"; watch_inputs 6
fi

section "5. Lid sensor"
echo "devices with switch events (EV_SW):"
awk '/^N: Name/{n=$0} /^H: Handlers/{h=$0} /^B: SW=/{print "  " n " | " h " | " $0}' /proc/bus/input/devices
echo "anbernic_misc:"
for f in /sys/class/anbernic_misc/*/* /sys/class/anbernic_misc/*; do
    [ -f "$f" ] && [ -r "$f" ] && echo "  $f = $(head -c 80 "$f" 2>/dev/null | tr '\n' ' ')"
done
dmesg 2>/dev/null | grep -iE 'hall|lid' | head -10
if [ "$INTERACTIVE" = 1 ]; then
    choose "Next: close the lid for ~3 s, then open it (8 s window)." "go:START"
    before=$(for f in /sys/class/anbernic_misc/*/*; do [ -f "$f" ] && echo "$f=$(head -c 40 "$f" 2>/dev/null)"; done)
    echo "  input during lid test:"; watch_inputs 8
    after=$(for f in /sys/class/anbernic_misc/*/*; do [ -f "$f" ] && echo "$f=$(head -c 40 "$f" 2>/dev/null)"; done)
    echo "  anbernic_misc changes:"; diff <(echo "$before") <(echo "$after") | sed 's/^/    /'
    dmesg 2>/dev/null | tail -15 | sed 's/^/    dmesg: /'
fi

section "6. USB-C host mode"
for n in /proc/device-tree/usbdrd/usb@fcc00000 /proc/device-tree/usb@fcc00000; do
    [ -d "$n" ] && echo "  $n dr_mode=$(tr -d '\0' < "$n/dr_mode" 2>/dev/null) status=$(tr -d '\0' < "$n/status" 2>/dev/null)"
done
for r in /sys/class/usb_role/*; do [ -f "$r/role" ] && echo "  usb_role $(basename "$r"): $(cat "$r/role")"; done
for t in /sys/class/typec/*; do [ -e "$t" ] && echo "  typec: ${t##*/}"; done
for e in /sys/class/extcon/*; do [ -d "$e" ] && echo "  extcon $(basename "$e") $(cat "$e/name" 2>/dev/null): $(cat "$e/state" 2>/dev/null | tr '\n' ' ')"; done
for r in /sys/class/regulator/*; do
    case "$(cat "$r/name" 2>/dev/null)" in *usb*|*otg*|*5v*) echo "  regulator $(cat "$r/name"): state=$(cat "$r/state" 2>/dev/null) uV=$(cat "$r/microvolts" 2>/dev/null)";; esac
done
echo "  lsusb now:"; lsusb 2>/dev/null | sed 's/^/    /'
if [ "$INTERACTIVE" = 1 ]; then
    choose "Plug a USB keyboard or stick into a USB-C port, wait 3 s." "plugged:START skip:R1"
    echo "  answer: $REPLY"; echo "  lsusb after plugging:"; lsusb 2>/dev/null | sed 's/^/    /'
    for r in /sys/class/usb_role/*; do [ -f "$r/role" ] && echo "  usb_role $(basename "$r"): $(cat "$r/role")"; done
    dmesg 2>/dev/null | grep -iE 'usb|dwc3|otg' | tail -10 | sed 's/^/    dmesg: /'
fi

section "Wi-Fi / Bluetooth / GPU (for reference)"
lsmod 2>/dev/null
iw dev 2>/dev/null
hciconfig -a 2>/dev/null | head -8
ls /dev/dri /dev/mali* 2>/dev/null

section "7. Power and suspend"
cat /sys/power/state 2>/dev/null | sed 's/^/  state: /'
cat /sys/power/mem_sleep 2>/dev/null | sed 's/^/  mem_sleep: /'
for t in /sys/class/rtc/*; do [ -e "$t" ] && echo "  rtc: ${t##*/}"; done
power_snapshot() {
    for p in /sys/class/power_supply/*; do
        [ -f "$p/uevent" ] || continue
        echo "  [$(basename "$p")]"
        grep -E 'STATUS|CAPACITY=|VOLTAGE_NOW|CURRENT_NOW|CHARGE_NOW|CHARGE_COUNTER|ENERGY_NOW|ONLINE|PRESENT' "$p/uevent" | sed 's/^POWER_SUPPLY_/    /'
    done
}
echo "awake, screens on:"; power_snapshot
echo "wakeup sources enabled:"
for w in /sys/class/wakeup/*; do [ -f "$w/name" ] && echo "  $(cat "$w/name")"; done | head -20

if [ "$MODE" = screen ]; then
    choose "Sleep test: unplug the charger. It sleeps 5 min, then wakes by itself." "sleep:L1 skip:R1"
    [ "$REPLY" = sleep ] && SUSPEND_SECS=300
fi
if [ "$SUSPEND_SECS" -gt 0 ] 2>/dev/null; then
    if have rtcwake; then
        [ "$MODE" = tty ] && choose "Unplug the charger, then press Enter to suspend for $SUSPEND_SECS s." ""
        say "sleeping for $SUSPEND_SECS s..."
        echo "before suspend ($(date +%s)):"; power_snapshot
        sync
        rtcwake -m mem -s "$SUSPEND_SECS"; rc=$?
        echo "rtcwake exit $rc ($(date +%s))"
        echo "after resume:"; power_snapshot
        dmesg 2>/dev/null | grep -iE 'PM: |suspend|resume|wakeup' | tail -20 | sed 's/^/    dmesg: /'
    else
        echo "  rtcwake not found; suspend test skipped"
    fi
else
    echo "  (suspend test skipped; run with --suspend 300 to measure drain)"
fi

section "Done"
echo "report saved to $OUT"
sync
if [ "$MODE" = screen ]; then
    say ""; say "Done. Report: Roms/APPS/nibos/$(basename "$OUT")"
    choose "Press Start to go back to the menu." "exit:START"
fi
