#!/bin/bash
# Turn the power-on splash on or off: `init=/usr/lib/nibos/splash` on the kernel command
# line (with `vt.global_cursor_default=0`), or not.
#
#   rootfs/splash/boot.sh on|off|status [user@host]
#
# The command line lives in two places, both written by kernel/deploy.sh: the Falcon FIT on
# the board's Falcon partition (what SPL boots) and /boot/extlinux/extlinux.conf (the fallback U-Boot, A held
# at power-on). Only the FIT gets the splash: the fallback stays the plain command line, so a
# splash that breaks the boot can't take the way back with it. extlinux.conf records the
# choice in a "# splash on" line, which deploy.sh keeps. The FIT is rebuilt from the kernel and
# DTB in /boot, which deploy.sh put there with it, so out/ (perhaps a newer build) isn't
# involved. The boot partition (the stock U-Boot's image) keeps its command line. Restart the
# device for it to take effect.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../tools/lib.sh
. "$here/../../tools/lib.sh"
want=${1:-status}
target=${2:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=5)
# With the splash, no console cursor: kernel/deploy.sh says why.
init="init=/usr/lib/nibos/splash vt.global_cursor_default=0"

conf=$(ssh "${ssh_opts[@]}" "$target" cat /boot/extlinux/extlinux.conf)
base=$(printf '%s\n' "$conf" | sed -n 's/^ *append //p')
[ -n "$base" ] || die "no append line in the device's extlinux.conf"
now=off
printf '%s\n' "$conf" | grep -qx '# splash on' && now=on
case "$want" in
on) bootargs="$base $init" ;;
off) bootargs="$base" ;;
status)
    echo "splash: $now"
    booted=$(ssh "${ssh_opts[@]}" "$target" cat /proc/cmdline)
    case " $booted " in *" $init "*) echo "(this boot had it)" ;; *) echo "(this boot didn't)" ;; esac
    exit 0
    ;;
*) usage ;;
esac
if [ "$want" = "$now" ]; then
    echo "splash: already $want"
    exit 0
fi
echo "command line: $bootargs"

out="$here/out"
mkdir -p "$out"
dtb=$BOARD_DTB.dtb
scp -q "${ssh_opts[@]}" "$target:/boot/Image.gz" "$target:/boot/$dtb" "$out/"
in_vm_sh "cd '$out' && gzip -dc Image.gz > Image && python3 '$here/../../uboot/mkfalcon.py' \"\$RKBIN/\$SOC_RKBIN_BL31\" Image $dtb '$bootargs' falcon.itb --memory \$BOARD_MEMORY --description \"nibos Falcon boot for \$BOARD_NAME\" && rm Image Image.gz"
scp -q "${ssh_opts[@]}" "$out/falcon.itb" "$target:/tmp/"
ssh "${ssh_opts[@]}" "$target" "SPLASH='$want' FALCON='$BOARD_FALCON_PARTNAME' FALCON_SECTOR='$BOARD_FALCON_SECTOR' sh -s" <<'EOF'
set -e
part=$(for p in /sys/class/block/mmcblk*p*; do grep -q "^PARTNAME=$FALCON$" "$p/uevent" && basename "$p"; done | head -1)
[ "$(cat "/sys/class/block/$part/start" 2>/dev/null)" = "$FALCON_SECTOR" ] || { echo "the $FALCON partition is not at sector $FALCON_SECTOR (SPL's Falcon FIT)"; exit 1; }
sudo dd if=/tmp/falcon.itb of="/dev/$part" bs=1M conv=fsync status=none
sudo sed -i '/^# splash on$/d' /boot/extlinux/extlinux.conf
[ "$SPLASH" = off ] || echo "# splash on" | sudo tee -a /boot/extlinux/extlinux.conf >/dev/null
rm -f /tmp/falcon.itb
sync
EOF
rm -f "$out/falcon.itb" "$out/$dtb"
echo "splash: $want from the next boot"
