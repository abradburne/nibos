#!/bin/bash
# Build the Arch Linux ARM root file system for the board's dev card, as an
# ext4 image: rootfs/out/archroot.img.
#
#   orb -m rgbuild sudo bash rootfs/make-arch.sh     (from the repository, on the Mac)
#
# Runs in the OrbStack Arch Linux ARM machine, which uses the same repos.
# What's in it: Arch base with systemd, our kernel's modules (kernel/out), the
# nibos-hw package (hw/out), Wi-Fi/BT firmware, iwd, OpenSSH, PipeWire, BlueZ
# and Mesa (Panfrost); nibOS's own units and scripts from files/. No kernel
# package: the kernel is in the bootloader's places (kernel/deploy.sh).
#
# The user, host name, time zone and SSH key come from config.sh (NIBOS_USER,
# NIBOS_HOSTNAME, NIBOS_TIMEZONE, SSH_PUBKEY). Logging in, without any
# passwords:
#  - SSH as $NIBOS_USER or root with that key, over Wi-Fi;
#  - on the screens, tty1 logs $NIBOS_USER in by itself (plug in a USB keyboard);
#  - Wi-Fi: put iwd network files in ROMS/nibos/wifi/ (docs/using.md), or
#    run iwctl on the device.
#
# SIZE (default 4G) is the image's size before it is shrunk to what it holds;
# NETWORK_MODE=boot|deferred|demand is how the network starts (hw/README.md).
#
# IMAGE=1 builds the root of a downloadable image instead (tools/make-image.sh;
# docs/design/images.md), as out/image-root.img: no SSH key, no passwords and no
# access baked in (provision.toml sets them at boot: nibos-provision), a state
# partition for what it sets, the data partition made on the first boot, and
# nothing that isn't ours to redistribute (no firmware from the stock image).
# PACKAGES names more package files to install (the DSlot edition's), and DSLOT=1
# sets DSlot up as the boot UI as rootfs/dslot/install.sh does on a device, with
# read-ahead (READAHEAD, the generator) and the safe mode (SAFE_MODE, rootfs/safe-mode,
# built). Both editions get the splash: SPLASH (its
# binary), SPLASH_PICTURES (a folder with boot.pspl and goodbye.pspl) and SPLASH_UNIT
# (nibos-goodbye.service). NOFDK is the nibos-no-fdk-aac package (rootfs/no-fdk-aac). BOOTARGS is
# the kernel command line for /boot/extlinux/extlinux.conf; DOCS a folder with the
# image's NOTICE and licenses/ (tools/make-image.sh).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
kernel="$here/../kernel"
out="$here/out"
user=$NIBOS_USER
uid=$NIBOS_UID
key=$SSH_PUBKEY
size=${SIZE:-4G}
root=/var/tmp/nibos-root
image=$out/archroot.img
label=$BOARD_ROOT_PARTLABEL
for_image=${IMAGE:-0}
image_pkgs=()
if [ "$for_image" = 1 ]; then
    # The first boot formats the data partition exFAT.
    image_pkgs=(exfatprogs)
    # DSlot is an SDL2 program, on SDL3.
    [ "${DSLOT:-0}" = 1 ] && image_pkgs+=(sdl2-compat)
    image=$out/image-root.img
    label=nibos-root-a
    user=nibos
    uid=1000
    NIBOS_HOSTNAME=nibos
fi

[ "$(id -u)" = 0 ] || die "run as root (sudo)"
[ -f "$kernel/out/modules.tar.zst" ] || die "build the kernel first (kernel/build.sh)"
[ "$for_image" = 1 ] || [ -f "$key" ] || die "no SSH public key at $key"
# The newest package, by modification time, unless HWPKG names one (an image that leaves out
# a newer one not yet tried on a device).
# shellcheck disable=SC2012 # the package files' own names
hwpkg=${HWPKG:-$(ls -t "$here"/../hw/out/nibos-hw-*.pkg.tar.* 2>/dev/null | head -1 || true)}
[ -n "$hwpkg" ] || die "build the hardware layer first (hw/: makepkg)"

umount -lR "$root" 2>/dev/null || true
rm -rf "$root"
mkdir -p "$root" "$out"
# pacman (and arch-chroot) want the new root to be a mount point.
mount --bind "$root" "$root"
# Lazily: gpg-agent, which pacman-key starts in the new root, may still hold it.
trap 'umount -lR "$root" 2>/dev/null || true' EXIT

pacstrap -K -c "$root" \
    base archlinuxarm-keyring sudo less vim htop man-db evtest \
    linux-firmware-realtek \
    iwd openssh \
    alsa-utils bluez bluez-utils pipewire pipewire-pulse pipewire-alsa wireplumber libpulse rtkit \
    wireless-regdb \
    mesa \
    "${image_pkgs[@]}"

# pacstrap -K sets up the keyring with the upstream Arch keys only.
chroot "$root" pacman-key --populate archlinuxarm >/dev/null

# No FDK AAC (rootfs/no-fdk-aac): the Bluetooth AAC plugin never unpacked, now or on an
# upgrade, and libfdk-aac replaced by a package that stands in for it.
if [ -n "${NOFDK:-}" ]; then
    sed -i '/^\[options\]/a NoExtract = usr/lib/spa-0.2/bluez5/libspa-codec-bluez5-aac.so' "$root/etc/pacman.conf"
    rm -f "$root/usr/lib/spa-0.2/bluez5/libspa-codec-bluez5-aac.so"
    cp "$NOFDK" "$root/var/cache/pacman/pkg/"
    arch-chroot "$root" pacman -U --noconfirm --ask=4 --disable-sandbox "/var/cache/pacman/pkg/$(basename "$NOFDK")" >/dev/null
fi

# Our kernel's modules.
tar -C "$root/usr" --no-same-owner --no-overwrite-dir --zstd -xf "$kernel/out/modules.tar.zst"
release=$(ls "$root/usr/lib/modules")
depmod -b "$root/usr" "$release" 2>/dev/null || chroot "$root" depmod "$release"
# The hardware layer for the board: gamepad, audio routing, lid and power,
# LEDs, network on demand (../hw).
cp "$hwpkg" "$root/var/cache/pacman/pkg/"
arch-chroot "$root" pacman -U --noconfirm --disable-sandbox "/var/cache/pacman/pkg/$(basename "$hwpkg")" >/dev/null
# Packages for the image's edition (the DSlot edition: dslot, the emulators, Mesa, ...).
for pkg in ${PACKAGES:-}; do
    cp "$pkg" "$root/var/cache/pacman/pkg/"
    # --ask=4: yes to replacing what a package conflicts with (mesa-nibos for Arch's mesa).
    arch-chroot "$root" pacman -U --noconfirm --ask=4 --disable-sandbox "/var/cache/pacman/pkg/$(basename "$pkg")" >/dev/null
done
# Emulators shipped as previews say so in DSlot (PREVIEW_EMULATORS: their ids).
for e in ${PREVIEW_EMULATORS:-}; do
    sed -i 's/^name = "\(.*\)"$/name = "\1 (preview)"/' "$root/usr/share/dslot/emulators/$e.toml"
done
# What a replaced package left that nothing needs now (Arch's mesa's LLVM, ~170 MB).
if [ -n "${PACKAGES:-}" ]; then
    orphans=$(chroot "$root" pacman -Qdtq || true)
    # shellcheck disable=SC2086 # one name per word
    [ -z "$orphans" ] || arch-chroot "$root" pacman -Rns --noconfirm --disable-sandbox $orphans >/dev/null
fi
# The board's firmware from the stock image (BOARD_FIRMWARE: the RG DS+'s speaker tuning
# file), the user's own copy (extract-stock.sh): not ours to redistribute, so in no package,
# and never in a downloadable image.
[ "$for_image" = 1 ] || for fw in $BOARD_FIRMWARE; do
    if [ -f "$STOCK_DIR/$fw" ]; then
        # Where the kernel looks first (firmware_class.path): the active tuning, which
        # nib audio setup keeps as the official one.
        install -Dm644 "$STOCK_DIR/$fw" "$root/var/lib/nibos/firmware/$fw"
        chroot "$root" /usr/lib/nibos/nib-audio setup >/dev/null
    else
        echo "make-arch.sh: no $STOCK_DIR/$fw (boards/$NIBOS_BOARD/extract-stock.sh): the parts that need it stay silent" >&2
    fi
done

# Identity, time, language.
echo "$NIBOS_HOSTNAME" > "$root/etc/hostname"
ln -sf "/usr/share/zoneinfo/$NIBOS_TIMEZONE" "$root/etc/localtime"
sed -i 's/^#en_US.UTF-8/en_US.UTF-8/' "$root/etc/locale.gen"
chroot "$root" locale-gen >/dev/null
echo LANG=en_US.UTF-8 > "$root/etc/locale.conf"
# The console font is the kernel's Terminus 16x32 (fbcon=font: on the command
# line), so nothing has to load a font during start-up.
printf 'KEYMAP=us\n' > "$root/etc/vconsole.conf"

# No root entry: the kernel mounts it (rw, noatime from the command line), and an
# fstab entry makes systemd wait for its device unit, fsck and growfs at boot.
# ROMS mounts on first use, not during boot.
if [ "$for_image" = 1 ]; then
    # The state partition (what provision.toml sets), and the data partition the first boot
    # makes (docs/design/images.md).
    cat > "$root/etc/fstab" <<EOF
PARTLABEL=nibos-state  /var/lib/nibos/state  ext4  rw,noatime  0 0
LABEL=NIBOS-DATA  /mnt/roms  exfat  rw,noatime,nofail,x-systemd.automount,uid=$uid,gid=$uid,fmask=0022,dmask=0022  0 0
/dev/nibos-games-card  /mnt/ds  auto  rw,noatime,nofail,noauto,x-systemd.device-bound,uid=$uid,gid=$uid,fmask=0022,dmask=0022  0 0
EOF
    mkdir -p "$root/var/lib/nibos/state" "$root/mnt/ds"
    # The TF2 card is mounted when udev sees it, not on first use: an automount would stall
    # every access for its device timeout with no card in the slot.
    install -Dm644 "$here/dslot/70-nibos-games-card.rules" "$root/etc/udev/rules.d/70-nibos-games-card.rules"
else
    cat > "$root/etc/fstab" <<EOF
LABEL=ROMS  /mnt/roms  vfat  rw,noatime,nofail,x-systemd.automount,uid=$uid,gid=$uid,fmask=0022,dmask=0022  0 0
EOF
fi
# Where `nib bug-report send` posts a scrubbed log (config.sh's NIBOS_BUGREPORT_DSN; empty:
# saved on the card only). Edit on the device to point a dev card elsewhere.
install -d "$root/etc/nibos"
printf '# nib bug-report: the bug tracker a report is sent to (a Sentry-style DSN). Empty: saved on the card only.\nDSN=%s\n' "$NIBOS_BUGREPORT_DSN" > "$root/etc/nibos/bug-report.conf"
# nibOS's own units and scripts (files/): growing the root file system on the
# first boot, the Wi-Fi import, the boot logs on ROMS, polkit for wheel, mDNS;
# then the board's (boards/<board>/rootfs/files/).
cp -R "$here/files/." "$NIBOS_ROOT/boards/$NIBOS_BOARD/rootfs/files/." "$root/"
[ "$for_image" = 1 ] && cp -R "$here/image/files/." "$root/"
# Not the Finder's .DS_Store and ._ files, should the tree have been browsed on a Mac.
find "$root/etc" "$root/usr" \( -name .DS_Store -o -name "._*" \) -delete
chmod 750 "$root/etc/polkit-1/rules.d"
mkdir -p "$root/mnt/roms"

chroot "$root" useradd -m -u "$uid" -G wheel,audio,video,input "$user"
if [ "$for_image" = 1 ]; then
    # No password, no keys, no SSH and no identity in the image: provision.toml and
    # nibos-provision set the access at boot, systemd makes the machine ID and sshd its
    # host keys on the first, and iwd keeps its networks on the state partition.
    rm -f "$root/etc/polkit-1/rules.d/49-nibos-wheel.rules"
    : > "$root/etc/machine-id"
    rm -rf "$root/var/lib/iwd"
    ln -s /var/lib/nibos/state/iwd "$root/var/lib/iwd"
else
    # Users: $user (sudo without a password) and root, both by SSH key only.
    echo "%wheel ALL=(ALL:ALL) NOPASSWD: ALL" > "$root/etc/sudoers.d/wheel"
    chmod 440 "$root/etc/sudoers.d/wheel"
    # The same for polkit (files/etc/polkit-1/rules.d/49-nibos-wheel.rules): there
    # are no passwords to type into its prompts.
    for home in "/root" "/home/$user"; do
        install -d -m700 "$root$home/.ssh"
        install -m600 "$key" "$root$home/.ssh/authorized_keys"
    done
    chroot "$root" chown -R "$user:$user" "/home/$user/.ssh"
    sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication no/' "$root/etc/ssh/sshd_config"

    # tty1 logs $user in, so a USB or BT keyboard gives a shell on the screens.
    install -d "$root/etc/systemd/system/getty@tty1.service.d"
    cat > "$root/etc/systemd/system/getty@tty1.service.d/autologin.conf" <<EOF
[Service]
ExecStart=
ExecStart=-/usr/bin/agetty --autologin $user --noclear %I \$TERM
EOF
fi

# Network: iwd does Wi-Fi and DHCP (its settings come with the board's
# hardware layer); resolved does DNS, and mDNS makes it $NIBOS_HOSTNAME.local.
ln -sf /run/systemd/resolve/stub-resolv.conf "$root/etc/resolv.conf"

# zram swap comes from nibos-zram (in nibos-hw), set up after start-up.

chroot "$root" systemctl enable systemd-resolved sshd nibos-wifi-import nibos-bootlog nibos-bootlog-shutdown nibos-growfs systemd-timesyncd
# Network (iwd, Wi-Fi/BT drivers) comes from nibos-network.target in nibos-hw:
# NETWORK_MODE=boot (the dev system), deferred (a productivity setup) or demand
# (a game player).
case "${NETWORK_MODE:-boot}" in
    # As `nib net mode` sets them; nibos-hw's install hook has enabled the boot mode already.
    boot)     chroot "$root" systemctl enable nibos-network.target nibos-network-modules.service ;;
    deferred) chroot "$root" systemctl disable nibos-network.target nibos-network-modules.service 2>/dev/null || true
              chroot "$root" systemctl enable nibos-network-deferred.service ;;
    demand)   chroot "$root" systemctl disable nibos-network.target nibos-network-modules.service \
                  nibos-network-deferred.service 2>/dev/null || true ;;
    *) echo "NETWORK_MODE must be boot, deferred or demand" >&2; exit 1 ;;
esac
chroot "$root" systemctl --global enable pipewire pipewire-pulse wireplumber >/dev/null

"$here/tune-boot.sh" "$root" "$user"
if [ "$for_image" = 1 ]; then
    # The fallback U-Boot (A held at power-on) boots /boot/extlinux/extlinux.conf with the
    # plain command line (BOOTARGS, from tools/make-image.sh), as kernel/deploy.sh writes it.
    install -d "$root/boot/extlinux"
    install -m644 "$kernel/out/Image.gz" "$kernel/out/$BOARD_DTB.dtb" "$root/boot/"
    printf "default nibos\nprompt 0\ntimeout 1\n\nlabel nibos\n  kernel /boot/Image.gz\n  fdt /boot/%s\n  append %s\n" \
        "$BOARD_DTB.dtb" "${BOOTARGS:?BOOTARGS, the kernel command line}" > "$root/boot/extlinux/extlinux.conf"
    # The notice and the licences that no package installs (DOCS, from tools/make-image.sh:
    # NOTICE, licenses/), and every package with its version and licence.
    if [ -n "${DOCS:-}" ]; then
        install -Dm644 "$DOCS/NOTICE" "$root/usr/share/doc/nibos/NOTICE"
        [ ! -f "$DOCS/release" ] || install -Dm644 "$DOCS/release" "$root/usr/lib/nibos/release"
        cp -R "$DOCS/licenses/." "$root/usr/share/licenses/"
    fi
    install -d "$root/usr/share/doc/nibos"
    chroot "$root" pacman -Qi | awk -F' *: ' '
        /^Name/ { n = $2 } /^Version/ { v = $2 } /^Licenses/ { print n, v, $2 }' \
        > "$root/usr/share/doc/nibos/packages.txt"
    cp "$root/usr/share/doc/nibos/packages.txt" "$out/image-packages.txt"
    # The NOTICE's package part: what needs knowing, then every package and its licence.
    if [ -n "${DOCS:-}" ]; then
        python3 "$here/../tools/image-notice.py" "$root/usr/share/doc/nibos/NOTICE" "$out/image-packages.txt"
        cp "$root/usr/share/doc/nibos/NOTICE" "$out/image-NOTICE"
    fi
    if [ "${DSLOT:-0}" = 1 ]; then
        # DSlot as the boot UI, as rootfs/dslot/install.sh sets it up on a device: the user
        # it runs as, its power menu's polkit rule, nib boot-ui, expedited RCU; then read-ahead
        # (rootfs/readahead).
        for f in user.conf 50-dslot-power.rules; do
            sed -e "s/@NIBOS_USER@/$user/g" -e "s/@NIBOS_UID@/$uid/g" "$here/dslot/$f" > "$root/tmp/$f"
        done
        install -Dm644 "$root/tmp/user.conf" "$root/etc/systemd/system/dslot.service.d/user.conf"
        install -Dm644 "$root/tmp/50-dslot-power.rules" "$root/etc/polkit-1/rules.d/50-dslot-power.rules"
        rm -f "$root/tmp/user.conf" "$root/tmp/50-dslot-power.rules"
        install -Dm755 "$here/dslot/nib-boot-ui" "$root/usr/lib/nibos/nib-boot-ui"
        install -Dm644 "$here/dslot/nibos-rcu.conf" "$root/etc/tmpfiles.d/nibos-rcu.conf"
        install -Dm755 "${READAHEAD:?READAHEAD, the read-ahead generator}" "$root/etc/systemd/system-generators/nibos-readahead"
        sed "s|@HOME@|/home/$user|g" "$here/readahead/readahead.list" | install -Dm644 /dev/stdin "$root/etc/nibos/readahead.list"
        # When DSlot keeps failing: restarted, then the safe mode (rootfs/safe-mode).
        install -Dm755 "${SAFE_MODE:?SAFE_MODE, rootfs/safe-mode}/target/release/nibos-safe-mode" "$root/usr/lib/nibos/safe-mode"
        install -Dm644 "$SAFE_MODE/nibos-safe-mode.service" "$root/usr/lib/systemd/system/nibos-safe-mode.service"
        install -Dm644 "$SAFE_MODE/nibos-recovery.conf" "$root/usr/lib/systemd/system/dslot.service.d/nibos-recovery.conf"
        # The package's own OnFailure=getty@tty1 goes (a drop-in can't take a dependency away):
        # named beside the safe mode in one OnFailure=, the two conflict and systemd started the
        # console and dropped the safe mode after a crash. The safe mode's own "Open a console"
        # still gives it, and so does a clean exit (OnSuccess=).
        sed -i '/^OnFailure=getty@tty1.service$/d' "$root/usr/lib/systemd/system/dslot.service"
        chroot "$root" systemctl enable dslot.service >/dev/null
        chroot "$root" systemctl disable getty@tty1.service >/dev/null 2>&1 || true
    fi
    # The splash in both editions (rootfs/splash): init=, its pictures, and the goodbye at
    # power-off.
    install -Dm755 "${SPLASH:?SPLASH, the splash binary}" "$root/usr/lib/nibos/splash"
    install -Dm644 -t "$root/usr/lib/nibos/pictures" "${SPLASH_PICTURES:?SPLASH_PICTURES}/boot.pspl" \
        "$SPLASH_PICTURES/goodbye.pspl"
    install -Dm644 "${SPLASH_UNIT:?SPLASH_UNIT}" "$root/usr/lib/systemd/system/nibos-goodbye.service"
    chroot "$root" systemctl enable nibos-goodbye.service >/dev/null
    # SSH only once provision.toml gives a key (nibos-provision enables sshd.socket).
    rm -f "$root/etc/systemd/system/sockets.target.wants/sshd.socket"
    # Every file the image ships under /usr and /boot with its SHA-256, for the one-time check
    # after the first boot that the card was written right (nibos-root-verify, image/files/).
    # Written last: nothing below changes those trees (enabling units writes /etc).
    (cd "$root" && find usr boot -type f ! -path usr/lib/nibos/root-files.sha256 -print0 |
        LC_ALL=C sort -z | xargs -0 sha256sum) > "$root/usr/lib/nibos/root-files.sha256"
    chroot "$root" systemctl enable nibos-provision nibos-firstboot-data nibos-root-verify.timer \
        nibos-settings-restore nibos-settings-save.timer nibos-settings-shutdown >/dev/null
fi

# Pack it. The image is small; the partition is grown on the device.
rm -rf "$root"/var/cache/pacman/pkg/*
rm -f "$image"
truncate -s "$size" "$image"
mkfs.ext4 -q -L "$label" -d "$root" "$image"
# Down to what it holds (plus a little), so writing it to the card is quicker.
e2fsck -fy "$image" >/dev/null
resize2fs -M "$image" >/dev/null 2>&1
blocks=$(dumpe2fs -h "$image" 2>/dev/null | awk -F: '/^Block count/ { print $2 }')
truncate -s $(( blocks * 4096 + 64 * 1024 * 1024 )) "$image"
chown "$(stat -c %u "$here")":"$(stat -c %g "$here")" "$image"
echo "kernel modules: $release"
du -sh "$root" | sed 's/^/root file system: /'
ls -la "$image"
