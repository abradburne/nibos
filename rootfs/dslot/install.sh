#!/bin/bash
# Put DSlot on the device as nibOS's boot UI: build the dslot package from $DSLOT_SRC (DSlot's
# own repository, next to this one; its packaging/nibos/PKGBUILD) in the build machine, install
# it, and add what nibOS provides around it: the user it runs as (a drop-in for its unit), the
# polkit rule that lets that user power off, `nib boot-ui`, expedited RCU, and the TF2 card.
#
#   rootfs/dslot/install.sh [user@host]
#
# Installs sdl2-compat (SDL2 on SDL3) on the device if it is missing. The TF2 card goes to
# /mnt/ds through the kernel's exFAT driver (exfatprogs replaces fuse-exfat's exfat-utils, whose
# mount helper would take over `mount -t exfat`). Emulators are separate packages, which put
# their descriptions in /usr/share/dslot/emulators/ (DSlot's docs/emulators.md). Leaves the boot
# switch as it is: `ssh DEVICE sudo nib boot-ui on` to start DSlot at power-on. Needs nibos-hw on
# the device (it brings the `nib` command).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../tools/lib.sh
. "$here/../../tools/lib.sh"
[ -f "$DSLOT_SRC/packaging/nibos/PKGBUILD" ] || die "no DSlot package source at $DSLOT_SRC (DSLOT_SRC in config.sh)"
dslot="$(cd "$DSLOT_SRC" && pwd)"
target=${1:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=5)
out="$here/out"
mkdir -p "$out"

rm -f "$out"/dslot-*.pkg.tar.*
in_vm bash -lc "pacman -Q sdl2-compat >/dev/null 2>&1 || sudo pacman -S --noconfirm sdl2-compat
cd '$dslot/packaging/nibos' && BUILDDIR=\$HOME/makepkg-build PKGDEST='$out' makepkg -f --nodeps >/dev/null"
pkg=$(find "$out" -name 'dslot-*.pkg.tar.*' | head -1)
[ -n "$pkg" ] || die "no dslot package in $out"

# The drop-in and the polkit rule name the user DSlot runs as.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
for f in user.conf 50-dslot-power.rules; do
    sed -e "s/@NIBOS_USER@/$NIBOS_USER/g" -e "s/@NIBOS_UID@/$NIBOS_UID/g" "$here/$f" > "$stage/$f"
done
scp -q "${ssh_opts[@]}" "$pkg" "$stage/user.conf" "$stage/50-dslot-power.rules" "$here/nib-boot-ui" \
    "$here/nibos-rcu.conf" "$target:/tmp/"
ssh "${ssh_opts[@]}" "$target" "UID_=$NIBOS_UID PKG='${pkg##*/}' sh -s" <<'EOF2'
set -e
command -v nib >/dev/null || { echo "no nib command: install nibos-hw first"; exit 1; }
pacman -Q sdl2-compat >/dev/null 2>&1 || sudo pacman -S --noconfirm --needed sdl2-compat
sudo pacman -U --noconfirm "/tmp/$PKG" >/dev/null
# A unit of DSlot's own that nibOS used to install would hide the package's.
sudo rm -f /etc/systemd/system/dslot.service
sudo install -Dm644 /tmp/user.conf /etc/systemd/system/dslot.service.d/user.conf
sudo install -Dm644 /tmp/50-dslot-power.rules /etc/polkit-1/rules.d/50-dslot-power.rules
sudo install -Dm755 /tmp/nib-boot-ui /usr/lib/nibos/nib-boot-ui
sudo install -Dm644 /tmp/nibos-rcu.conf /etc/tmpfiles.d/nibos-rcu.conf
sudo systemd-tmpfiles --create /etc/tmpfiles.d/nibos-rcu.conf
# The TF2 card: kernel exFAT, mounted on first use like the ROMS partition.
[ ! -e /usr/bin/mount.exfat-fuse ] || sudo pacman -Rdd --noconfirm exfat-utils >/dev/null
sudo pacman -S --noconfirm --needed exfatprogs >/dev/null
sudo install -d /mnt/ds
grep -q "^LABEL=DS " /etc/fstab ||
    echo "LABEL=DS    /mnt/ds    exfat rw,noatime,nofail,x-systemd.automount,x-systemd.device-timeout=5,uid=$UID_,gid=$UID_,fmask=0022,dmask=0022  0 0" |
    sudo tee -a /etc/fstab >/dev/null
rm -f "/tmp/$PKG" /tmp/user.conf /tmp/50-dslot-power.rules /tmp/nib-boot-ui /tmp/nibos-rcu.conf
sudo systemctl daemon-reload
ls /usr/share/dslot/emulators/*.toml >/dev/null 2>&1 || echo "no emulator descriptions in /usr/share/dslot/emulators yet: install an emulator to start games"
nib boot-ui status
EOF2
