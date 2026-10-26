#!/bin/bash
# Check a downloadable image without a device, in the build machine (as root): what's in its
# root and on its NIBOS partition, the one-time check of the root against the image (on the
# copy as written, and with its root deliberately damaged), and the first boot's data partition
# on a copy made card sized, including resuming after a power cut, leaving a foreign file system
# alone, and the card written again (with this image, and with OTHER, the other edition's, if
# given).
#   orb -m rgbuild sudo bash tools/test-image.sh out/nibos-<version>-<board>.img.xz [OTHER]
# shellcheck disable=SC2016 # each check is a string, evaluated by check()
set -euo pipefail
shopt -s inherit_errexit
[ $# -ge 1 ] && [ $# -le 2 ] || { sed -n '2,6s/^# \{0,1\}//p' "$0" >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 2; }
fails=0
ok() { echo "ok:   $1"; }
bad() { echo "FAIL: $1"; fails=$((fails + 1)); }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

tmp=$(mktemp -d)
img=$tmp/card.img
loop=
cleanup() {
    machinectl terminate "nibos-test-$$" >/dev/null 2>&1 || true
    mountpoint -q "$tmp/ov" 2>/dev/null && umount "$tmp/ov"
    for m in "$tmp"/m-*; do mountpoint -q "$m" 2>/dev/null && umount "$m"; done
    [ -z "$loop" ] || losetup -d "$loop" 2>/dev/null || true
    rm -rf "$tmp"
}
trap cleanup EXIT
xz -dc "$1" > "$img"
truncate -s +4G "$img"            # a card larger than the image
loop=$(losetup -fP --show "$img")
# The kernel makes the partitions' device nodes a moment after it finds them (there is no
# udev in the build machine to wait on): wait for them.
nodes() {
    local p
    for _ in $(seq 50); do
        for p in /sys/block/"${loop##*/}"/"${loop##*/}"p*; do
            [ ! -e "$p" ] || [ -b "/dev/${p##*/}" ] || { sleep 0.1; continue 2; }
        done
        return 0
    done
}
nodes
mnt() {
    mkdir -p "$tmp/m-$1"
    mount -o "${3:-ro}" "$2" "$tmp/m-$1" || { echo "can't mount $2" >&2; exit 1; }
    echo "$tmp/m-$1"
}

echo "== the root"
r=$(mnt root "${loop}p3")
check "no authorized_keys" '[ -z "$(find "$r/root" "$r/home" -name authorized_keys 2>/dev/null)" ]'
check "no SSH host keys" '! ls "$r"/etc/ssh/ssh_host_* >/dev/null 2>&1'
check "empty machine-id" '[ ! -s "$r/etc/machine-id" ]'
check "no password for nibos (locked)" 'grep -q "^nibos:[!*]" "$r/etc/shadow"'
check "root has no password" 'grep -q "^root:[!*]*:" "$r/etc/shadow"'
check "no sudoers or autologin baked in" '[ ! -e "$r/etc/sudoers.d/wheel" ] && [ ! -e "$r/etc/systemd/system/getty@tty1.service.d/autologin.conf" ]'
check "no polkit wheel rule baked in" '[ ! -e "$r/etc/polkit-1/rules.d/49-nibos-wheel.rules" ]'
check "sshd per connection reads the provisioned keys (no AuthorizedKeysFile of its own)" 'grep -qx "ExecStart=-/usr/bin/sshd -i" "$r/etc/systemd/system/sshd@.service.d/50-nibos.conf"'
check "sshd off until provisioned" '[ ! -e "$r/etc/systemd/system/sockets.target.wants/sshd.socket" ] && [ ! -e "$r/etc/systemd/system/multi-user.target.wants/sshd.service" ]'
check "nibos-provision on" '[ -L "$r/etc/systemd/system/sysinit.target.wants/nibos-provision.service" ]'
check "nibos-firstboot-data on" '[ -L "$r/etc/systemd/system/local-fs.target.wants/nibos-firstboot-data.service" ]'
check "the provision program" '[ -x "$r/usr/bin/nibos-provision" ]'
check "fstab: state and data" 'grep -q "PARTLABEL=nibos-state" "$r/etc/fstab" && grep -q "LABEL=NIBOS-DATA" "$r/etc/fstab"'
check "iwd keeps networks on the state partition" '[ "$(readlink "$r/var/lib/iwd")" = /var/lib/nibos/state/iwd ]'
check "iwd's state directory isn't the symlink (systemd refuses one)" 'grep -qx "StateDirectory=" "$r/etc/systemd/system/iwd.service.d/50-nibos-state.conf"'
check "Wi-Fi and Bluetooth off on a fresh card until switched on" '[ -x "$r/usr/lib/nibos/radios-first-off" ] && grep -q "nibos-radios-first-off.service" "$r/etc/udev/rules.d/73-nibos-radios-first-off.rules" && [ -s "$r/etc/systemd/system/nibos-radios-first-off.service" ]'
check "fallback extlinux on root-a" 'grep -q "root=PARTLABEL=nibos-root-a" "$r/boot/extlinux/extlinux.conf" && [ -f "$r/boot/Image.gz" ]'
check "no Awinic tuning file" '[ -z "$(find "$r/usr/lib/firmware" -name "aw88*" 2>/dev/null)" ]'
check "the speakers use nibOS's own tuning" 'cmp -s "$r/var/lib/nibos/firmware/aw88166_acf.bin" "$r/usr/lib/nibos/aw88166-nibos.bin" && [ ! -e "$r/var/lib/nibos/firmware/aw88166-official.bin" ]'
check "firmware_class.path on the command line" 'grep -q "firmware_class.path=/var/lib/nibos/firmware" "$r/boot/extlinux/extlinux.conf"'
check "NOTICE and packages.txt" '[ -s "$r/usr/share/doc/nibos/NOTICE" ] && [ -s "$r/usr/share/doc/nibos/packages.txt" ]'
check "rkbin's licence" '[ -s "$r/usr/share/licenses/rkbin/LICENSE" ]'
check "the builder's SSH key isn't anywhere" '! grep -rqs "$(cut -d" " -f2 /root/.ssh/id_ed25519.pub 2>/dev/null || echo none-here)" "$r/root" "$r/home" "$r/etc"'
check "the root's file list for the check after the first boot" '[ -s "$r/usr/lib/nibos/root-files.sha256" ] && grep -q "  usr/bin/bash$" "$r/usr/lib/nibos/root-files.sha256" && grep -q "  boot/extlinux/extlinux.conf$" "$r/usr/lib/nibos/root-files.sha256"'
check "nibos-root-verify on (its timer)" '[ -L "$r/etc/systemd/system/timers.target.wants/nibos-root-verify.timer" ] && [ -x "$r/usr/lib/nibos/root-verify" ]'
if [ -e "$r/usr/bin/dslot" ]; then
    echo "== the DSlot edition"
    # shellcheck disable=SC2034 # used in the checks
    w=$r/etc/systemd/system
    check "DSlot is the boot UI" '[ -n "$(find "$w" -path "*wants/dslot.service" 2>/dev/null)" ]'
    check "no tty1 login instead" '[ -z "$(find "$w" -path "*wants/getty@tty1.service" 2>/dev/null)" ]'
    check "DSlot runs as nibos" 'grep -q "^User=nibos" "$w/dslot.service.d/user.conf"'
    check "its power menu's polkit rule" 'grep -q "\"nibos\"" "$r/etc/polkit-1/rules.d/50-dslot-power.rules"'
    check "the splash" '[ -x "$r/usr/lib/nibos/splash" ] && [ -s "$r/var/lib/dslot/splash" ]'
    check "read-ahead" '[ -x "$r/etc/systemd/system-generators/nibos-readahead" ] && grep -q "^/home/nibos/" "$r/etc/nibos/readahead.list"'
    check "DSperate and inkdot for DSlot" '[ -s "$r/usr/share/dslot/emulators/dsperate.toml" ] && [ -s "$r/usr/share/dslot/emulators/inkdot.toml" ]'
    check "Mesa without LLVM" 'grep -q "^mesa-nibos " "$r/usr/share/doc/nibos/packages.txt" && ! grep -q "^llvm-libs " "$r/usr/share/doc/nibos/packages.txt"'
    check "the fonts' OFL" '[ -s "$r/usr/share/licenses/dslot/OFL.txt" ]'
    check "NOTICE names DSlot, DSperate, inkdot" 'grep -q "^- DSlot" "$r/usr/share/doc/nibos/NOTICE" && grep -q "^- DSperate" "$r/usr/share/doc/nibos/NOTICE" && grep -q "^- inkdot" "$r/usr/share/doc/nibos/NOTICE"'
fi
umount "$r"

echo "== the root against the image (nibos-root-verify)"
# The script from this tree against the copy's root, as written and then with one block of a
# file zeroed (as a card written wrongly has it), the marker and the notice in $tmp.
rv=$tmp/root-verify
cp "$(dirname "$0")/../rootfs/image/files/usr/lib/nibos/root-verify" "$rv"
rm -rf "$tmp/rv-state" "$tmp/rv-run"
mkdir -p "$tmp/rv-state" "$tmp/rv-run"
run_rv() { ROOT=$1 MARKER="$tmp/rv-state/root-verified" NOTICE_DIR="$tmp/rv-run/notice" sh "$rv" 2>&1; }
r=$(mnt root "${loop}p3")
check "an intact copy: ok" 'run_rv "$r" | grep -q "files match the image" && grep -qx "result=ok" "$tmp/rv-state/root-verified" && [ ! -e "$tmp/rv-run/notice/root-verify" ]'
check "a second run reads nothing" '[ "$(run_rv "$r")" = "root-verify: already checked: ok" ]'
umount "$r"
victim=/usr/share/licenses/rkbin/LICENSE
blocksize=$(dumpe2fs -h "${loop}p3" 2>/dev/null | awk -F: '/^Block size/ { gsub(/ /, "", $2); print $2 }')
block=$(debugfs -R "bmap $victim 0" "${loop}p3" 2>/dev/null | tr -d ' ')
check "found a block of $victim to damage" '[ -n "$block" ] && [ "$block" -gt 0 ] && [ -n "$blocksize" ]'
[ -z "$block" ] || dd if=/dev/zero of="${loop}p3" bs="${blocksize:-4096}" seek="$block" count=1 conv=notrunc status=none
sync
rm -f "$tmp/rv-state/root-verified"
r=$(mnt root "${loop}p3")
check "a damaged copy: bad, named in the marker" 'run_rv "$r" | grep -q "not written correctly" && grep -qx "result=bad" "$tmp/rv-state/root-verified" && grep -q "^bad: ${victim#/}: FAILED" "$tmp/rv-state/root-verified"'
check "a damaged copy: the notice for DSlot" '[ "$(head -n1 "$tmp/rv-run/notice/root-verify")" = "This card was not written correctly: write the image again with Raspberry Pi Imager or dd (docs/install.md)" ]'
rm -f "$tmp/rv-run/notice/root-verify"
check "a damaged copy: announced again without reading" '[ "$(run_rv "$r" | head -n1)" = "root-verify: already checked: bad" ] && [ -s "$tmp/rv-run/notice/root-verify" ]'
umount "$r"

echo "== NIBOS"
b=$(mnt boot "${loop}p2")
for f in provision.toml.example NOTICE README.txt packages.txt licenses/rkbin.txt licenses/LICENCE.rtlwifi_firmware.txt; do
    check "NIBOS/$f" '[ -s "$b/$f" ]'
done
check "no provision.toml shipped" '[ ! -e "$b/provision.toml" ]'
umount "$b"

echo "== the first boot's data partition"
s=$(mnt state "${loop}p5" rw)
check "the state partition's top and ssh/ open to all (sshd reads keys as the user)" '[ "$(stat -c %a "$s")" = 755 ] && [ "$(stat -c %a "$s/ssh")" = 755 ] && [ "$(stat -c %a "$s/iwd")" = 700 ]'
check "marker says factory" '[ "$(cat "$s/data-partition")" = factory ]'
fb=$tmp/firstboot-data
cp "$(dirname "$0")/../rootfs/image/files/usr/lib/nibos/firstboot-data" "$fb"
run_fb() { MARKER="$s/data-partition" DISK="$loop" sh "$fb"; }
data_part() { lsblk -lnpo NAME,PARTLABEL "$loop" | awk '$2 == "nibos-data" { print $1 }'; }
run_fb >/dev/null
nodes
p6=$(data_part)
check "made nibos-data" '[ -n "$p6" ]'
check "exFAT, labelled NIBOS-DATA" '[ "$(blkid -p -o value -s TYPE "$p6")" = exfat ] && [ "$(blkid -p -o value -s LABEL "$p6")" = NIBOS-DATA ]'
check "over the rest of the card (> 3.5 GiB)" '[ "$(lsblk -bno SIZE "$p6")" -gt $((3500 * 1024 * 1024)) ]'
d=$(mnt data "$p6")
check "Roms/NDS, BIOS, Saves, APPS" '[ -d "$d/Roms/NDS" ] && [ -d "$d/BIOS" ] && [ -d "$d/Saves" ] && [ -d "$d/APPS" ]'
umount "$d"
check "marker says done" '[ "$(cat "$s/data-partition")" = done ]'
check "a second run does nothing" 'run_fb >/dev/null && [ "$(cat "$s/data-partition")" = done ]'

# A power cut after the partition was made but before it was formatted.
wipefs -qaf "$p6"
echo formatting > "$s/data-partition"
run_fb >/dev/null
check "resumed from formatting" '[ "$(blkid -p -o value -s TYPE "$p6")" = exfat ]'

# A foreign file system where it would format: left alone.
mkfs.ext4 -q -L someone-else "$p6"
echo formatting > "$s/data-partition"
run_fb >/dev/null 2>&1 || true
check "left a foreign file system alone" '[ "$(blkid -p -o value -s TYPE "$p6")" = ext4 ]'

# A power cut before the partition existed: the next start reads the table afresh.
wipefs -qaf "$p6"
echo creating > "$s/data-partition"
umount "$s"
losetup -d "$loop"
sfdisk -q --delete "$img" 6
loop=$(losetup -fP --show "$img")
nodes
s=$(mnt state "${loop}p5" rw)
run_fb >/dev/null
nodes
check "resumed from creating" '[ "$(blkid -p -o value -s TYPE "$(data_part)")" = exfat ] && [ "$(cat "$s/data-partition")" = done ]'

# The card written again, as an imaging tool does it (every byte of the image over the card's
# start), with this image and then the other edition's if given: the first boot makes the data
# partition again where it was, and the games and saves on it must survive.
d=$(mnt data "$(data_part)" rw)
echo kept > "$d/Saves/a-save"
umount "$d"
umount "$s"
for again in "$1" ${2:+"$2"}; do
    losetup -d "$loop"
    xz -dc "$again" | dd of="$img" bs=4M conv=notrunc status=none
    loop=$(losetup -fP --show "$img")
    nodes
    s=$(mnt state "${loop}p5" rw)
    check "written again (${again##*/}): a fresh state partition" '[ "$(cat "$s/data-partition")" = factory ]'
    run_fb >/dev/null
    nodes
    d=$(mnt data "$(data_part)")
    check "written again (${again##*/}): NIBOS-DATA kept" '[ "$(cat "$d/Saves/a-save" 2>/dev/null)" = kept ] && [ "$(cat "$s/data-partition")" = done ]'
    umount "$d"
    umount "$s"
done

echo "== the first boot and the next ones, in a container"
# The root as it boots, read-only beneath an overlay that lasts across the container's restarts
# (as the root's changes last on the card), with the state partition. The container has no
# radios and no drivers to load, so this checks what starts the network, not Wi-Fi itself.
r=$(mnt root "${loop}p3")
s=$(mnt state "${loop}p5" rw)
mkdir -p "$tmp/ov-up" "$tmp/ov-work" "$tmp/ov" "$tmp/roms"
mount -t overlay overlay -o "lowerdir=$r,upperdir=$tmp/ov-up,workdir=$tmp/ov-work" "$tmp/ov"
machine=nibos-test-$$
boot() {
    systemd-nspawn -q -D "$tmp/ov" --bind="$s:/var/lib/nibos/state" --bind="$tmp/roms:/mnt/roms" -b \
        --machine="$machine" >>"$tmp/boot.log" 2>&1 &
    local state=
    for _ in $(seq 60); do
        state=$(systemctl -M "$machine" is-system-running 2>/dev/null || true)
        case "$state" in running | degraded) break ;; esac
        sleep 1
    done
    # Every check below means something only if the root really booted.
    case "$state" in running | degraded) ;; *) bad "the container didn't boot ($state)"; tail -20 "$tmp/boot.log" ;; esac
    # nibos-radios-restore, once start-up has finished.
    for _ in $(seq 40); do
        case "$(systemctl -M "$machine" is-active nibos-radios-restore.service 2>/dev/null)" in
        active | activating) sleep 1 ;;
        *) break ;;
        esac
    done
}
halt() {
    machinectl poweroff "$machine" >/dev/null 2>&1 || machinectl terminate "$machine" >/dev/null 2>&1 || true
    for _ in $(seq 30); do machinectl status "$machine" >/dev/null 2>&1 || break; sleep 1; done
}
inactive() { [ "$(systemctl -M "$machine" is-active "$1" 2>/dev/null)" != active ] && ! systemctl -M "$machine" list-jobs --no-legend 2>/dev/null | grep -q "$1"; }
# Whether this boot started the drivers (it fails in a container, but it was started).
drivers_started() { journalctl -M "$machine" -b -u nibos-network-modules.service -o cat 2>/dev/null | grep -q "^Starting"; }
in_machine() { systemd-run -M "$machine" --wait -q --pipe "$@"; }
boot_mode=0
[ -L "$r/etc/systemd/system/multi-user.target.wants/nibos-network.target" ] && boot_mode=1
boot
if [ "$boot_mode" = 0 ]; then
    check "the network isn't started at boot (its mode isn't boot)" 'inactive iwd.service && ! drivers_started'
fi
# What a menu does when it reads the radios (nibkit's net, with D-Bus's no-auto-start flag):
# it must not start iwd, which would load the Wi-Fi driver.
systemctl -M "$machine" stop iwd.service nibos-network-modules.service >/dev/null 2>&1 || true
in_machine busctl --system --auto-start=no call net.connman.iwd / \
    org.freedesktop.DBus.ObjectManager GetManagedObjects >/dev/null 2>&1 || true
sleep 1
check "reading iwd without auto-start doesn't start it" 'inactive iwd.service && inactive nibos-network-modules.service'
check "system updates: nibOS's update key" 'grep -q "^untrusted comment: minisign public key 41B02B0D6ED49D26$" "$r/usr/lib/nibos/update.pub"'
check "system updates: the release file, nibos-update, its boot step and keep list" 'grep -q "^NIBOS_VERSION=." "$r/usr/lib/nibos/release" && grep -q "^NIBOS_UPDATE_REPO=." "$r/usr/lib/nibos/release" && [ -x "$r/usr/bin/nibos-update" ] && [ -x "$r/usr/lib/nibos/nib-update" ] && [ -L "$r/etc/systemd/system/multi-user.target.wants/nibos-update-boot.service" ] && [ -s "$r/usr/lib/nibos/update-keep" ]'
# The timer fires at once in a container (OnBootSec counts from the host's start); the check
# reads the whole of /usr, so give it a while.
for _ in $(seq 180); do grep -qs "^result=" "$s/root-verified" && break; sleep 1; done
check "nibos-root-verify ran once after the start and found the root whole" '[ "$(systemctl -M "$machine" show -p Result --value nibos-root-verify.service)" = success ] && grep -qx "result=ok" "$s/root-verified"'
check "the update step at boot ran and said where it stands" '[ "$(systemctl -M "$machine" show -p Result --value nibos-update-boot.service)" = success ] && in_machine cat /run/nibos/update/status.json | grep -q "\"phase\":\"idle\""'
check "the radios' switches: nib bluetooth and the restore at boot" '[ -x "$r/usr/lib/nibos/nib-bluetooth" ] && [ -L "$r/etc/systemd/system/multi-user.target.wants/nibos-radios-restore.service" ]'
halt
check "the logs on the card at power-off" 'grep -qs "^nibOS: .*(now)" "$tmp"/roms/nibos/log/boot-*.txt'
# A radio left on (what systemd-rfkill saves for an unblocked radio) brings the network up at
# the next boot; left off, it doesn't. The radios' own state is systemd-rfkill's to put back.
rf=$tmp/ov/var/lib/systemd/rfkill   # written through the overlay, as the device would
if [ "$boot_mode" = 0 ]; then
    mkdir -p "$rf"
    echo 0 > "$rf/platform-test:wlan"
    boot
    check "Wi-Fi left on: the network comes up at the next boot" 'drivers_started'
    halt
    echo 1 > "$rf/platform-test:wlan"
    echo 0 > "$rf/platform-test:bluetooth"
    boot
    check "Bluetooth left on: the network comes up at the next boot" 'drivers_started'
    halt
    echo 1 > "$rf/platform-test:bluetooth"
    boot
    check "both left off: the network stays down" '! drivers_started'
    halt
fi
umount "$tmp/ov"
umount "$s"
umount "$r"

echo
if [ "$fails" = 0 ]; then echo "all checks passed"; else echo "$fails failed"; exit 1; fi
