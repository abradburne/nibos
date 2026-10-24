#!/bin/bash
# Put the kernel just built (out/) on a running device over SSH, and restart it:
# the uncompressed kernel, DTB and bootargs as a Falcon FIT to the board's
# Falcon partition (what SPL boots by default: ../uboot/install-falcon.sh),
# kernel and DTB to /boot with an extlinux.conf (what mainline U-Boot boots, and
# the fallback U-Boot when A is held at power-on), the same as a FIT boot image
# to the boot partition (what the stock U-Boot boots), and modules to
# /usr/lib/modules.
#
#   kernel/deploy.sh [user@host]        (default $DEVICE from config.sh)
#
# Builds out/boot-arch.img and out/falcon.itb from out/Image.gz and the DTB
# first. The card's original Falcon ("recovery") partition is kept once on
# ROMS/nibos/recovery-stock.img; flash-boot.sh keeps the boot partition.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
target=${1:-$DEVICE}
# systemd.tty.*.console: the VT size (board.sh). Without them PID 1 asks the
# console its size with an xterm query the VT never answers and waits 333 ms
# for the reply before starting anything.
# SYSTEMD_DEFAULT_MOUNT_RATE_LIMIT_BURST (passed to PID 1 as environment): the
# early API mounts, the ROMS automount and the root remount are more than the
# default 5 mount-table changes a second, and PID 1 then holds every mount job
# (so local-fs.target and everything after it) ~0.7 s until the limit lifts.
# rootfstype=ext4: without it the kernel tries ext3 and ext2 (which ext4 also
# registers) on the root partition first.
# firmware_class.path: firmware nibOS chooses at run time (the speaker amps' tuning: nib audio)
# comes from /var/lib/nibos/firmware before /usr/lib/firmware.
bootargs="root=PARTLABEL=$BOARD_ROOT_PARTLABEL rootfstype=ext4 rootwait rw firmware_class.path=/var/lib/nibos/firmware console=tty1 fbcon=font:TER16x32 systemd.tty.rows.console=$BOARD_CONSOLE_ROWS systemd.tty.columns.console=$BOARD_CONSOLE_COLUMNS SYSTEMD_DEFAULT_MOUNT_RATE_LIMIT_BURST=20 quiet loglevel=3"
# The power-on splash (init= puts DSlot's logo up, then runs systemd) stays on or off as the device
# has it (../rootfs/splash/boot.sh on|off): in the Falcon FIT only, so the fallback through
# extlinux keeps the plain command line. extlinux.conf records it in a "# splash on" line.
# With it, the console's cursor is off (vt.global_cursor_default=0): the console shows for a
# moment whenever no program holds the screens (the splash handing over to DSlot, DSlot
# stopping before the goodbye picture or restarting), and a blinking cursor there flashed. A
# shell on a VT turns it back on (/etc/profile.d/nibos-cursor.sh).
falconargs=$bootargs splash=off
if ssh "${ssh_opts[@]}" "$target" "grep -qx '# splash on' /boot/extlinux/extlinux.conf" 2>/dev/null; then
    falconargs="$bootargs init=/usr/lib/nibos/splash vt.global_cursor_default=0" splash=on
fi

dtb=$BOARD_DTB.dtb
[ -f "$STOCK_DIR/boot.img" ] || die "no $STOCK_DIR/boot.img: run boards/$NIBOS_BOARD/extract-stock.sh"
in_vm_sh "cd kernel && python3 mkboot.py \"\$STOCK_DIR/boot.img\" out/Image.gz out/$dtb out/boot-arch.img --bootargs '$bootargs' --description \"nibos kernel for \$BOARD_NAME\""
# Falcon: SPL reads the kernel uncompressed (it can't unpack it quickly).
in_vm_sh "cd kernel && gzip -dc out/Image.gz > out/Image && python3 ../uboot/mkfalcon.py \"\$RKBIN/\$SOC_RKBIN_BL31\" out/Image out/$dtb '$falconargs' out/falcon.itb --memory \$BOARD_MEMORY --description \"nibos Falcon boot for \$BOARD_NAME\" && rm out/Image"

scp "${ssh_opts[@]}" "$here/out/boot-arch.img" "$here/out/modules.tar.zst" "$here/out/Image.gz" \
    "$here/out/$dtb" "$here/out/falcon.itb" "$target:/tmp/"
ssh "${ssh_opts[@]}" "$target" "BOOTARGS='$bootargs' SPLASH='$splash' DTB='$dtb' BOOT='$BOARD_BOOT_PARTNAME' FALCON='$BOARD_FALCON_PARTNAME' FALCON_SECTOR='$BOARD_FALCON_SECTOR' sh -s" <<'EOF'
set -e
sudo install -d /boot/extlinux
sudo install -m644 /tmp/Image.gz "/tmp/$DTB" /boot/
printf "default nibos\nprompt 0\ntimeout 1\n\nlabel nibos\n  kernel /boot/Image.gz\n  fdt /boot/%s\n  append %s\n" "$DTB" "$BOOTARGS" |
    sudo tee /boot/extlinux/extlinux.conf >/dev/null
[ "$SPLASH" = off ] || echo "# splash on" | sudo tee -a /boot/extlinux/extlinux.conf >/dev/null
part=$(for p in /sys/class/block/mmcblk*p*; do grep -q "^PARTNAME=$BOOT$" "$p/uevent" && basename "$p"; done | head -1)
[ -n "$part" ] || { echo "no boot partition found"; exit 1; }
echo "writing /dev/$part"
sudo dd if=/tmp/boot-arch.img of="/dev/$part" bs=1M conv=fsync status=none
part=$(for p in /sys/class/block/mmcblk*p*; do grep -q "^PARTNAME=$FALCON$" "$p/uevent" && basename "$p"; done | head -1)
[ "$(cat "/sys/class/block/$part/start" 2>/dev/null)" = "$FALCON_SECTOR" ] || { echo "the $FALCON partition is not at sector $FALCON_SECTOR (SPL's Falcon FIT)"; exit 1; }
keep=/mnt/roms/nibos/recovery-stock.img
if [ ! -f "$keep" ]; then
    echo "keeping the stock recovery partition in $keep"
    sudo dd if="/dev/$part" of="$keep.part" bs=1M status=none
    sudo mv "$keep.part" "$keep"
    sha256sum "$keep" | cut -d" " -f1 | sudo tee "$keep.sha256" >/dev/null
fi
echo "writing the Falcon FIT to /dev/$part"
sudo dd if=/tmp/falcon.itb of="/dev/$part" bs=1M conv=fsync status=none
sudo tar -C /usr --no-same-owner --no-overwrite-dir --zstd -xf /tmp/modules.tar.zst
sudo depmod -a "$(tar --zstd -tf /tmp/modules.tar.zst | sed -n "s|^lib/modules/\([^/]*\)/$|\1|p" | head -1)"
rm -f /tmp/boot-arch.img /tmp/modules.tar.zst /tmp/Image.gz "/tmp/$DTB" /tmp/falcon.itb
sync
echo "restarting"
# In a second, so this ssh session ends cleanly before the connection drops.
sudo systemd-run --quiet --on-active=1 systemctl reboot
EOF
echo "Deployed. It's restarting; ssh back in in about 30 s."
