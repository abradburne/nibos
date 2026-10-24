#!/bin/bash
# Build a downloadable nibOS image for the board (docs/design/images.md): one file to write
# to a card with any imaging tool, then edit provision.toml on its NIBOS partition, then boot.
# Nothing in it is from the stock firmware, and no credentials are baked in.
#
#   tools/make-image.sh [--dslot]          (Mac or a Linux host; the work runs in the VM)
#
# --dslot: the edition ready to play: DSlot as the boot UI, DSperate, inkdot, nibcase with its
# libretro cores and inktick (a preview), Mesa without LLVM, the safe mode and read-ahead, built
# from DSLOT_SRC, DSPERATE_SRC, INKDOT_SRC, NIBCASE_SRC and INKTICK_SRC (default: beside this
# repository) at whatever they have checked out, and rootfs/mesa/out (built if missing).
#
# Builds the kernel, U-Boot (SPL in Falcon mode, and the fallback for this layout) and
# nibos-hw, the root (rootfs/make-arch.sh with IMAGE=1), the Falcon FIT, and lays them out
# (tools/assemble-image.sh) as out/nibos-<version>-<board>[-dslot].img.xz, with its sha256
# in out/SHA256SUMS. SKIP_BUILD=1 reuses kernel/out and hw/out as they are (HWPKG=FILE takes
# that nibos-hw package rather than the newest). SOURCE_URL is
# where the release's source will be (in the NOTICE).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"
edition=base
case "${1:-}" in
"") ;;
--dslot) edition=dslot ;;
*) usage 2 ;;
esac
version=$(git -C "$NIBOS_ROOT" describe --tags --always --dirty)
name=nibos-$version-$BOARD_HOSTNAME$([ "$edition" = dslot ] && echo -dslot || true)
out=$NIBOS_ROOT/out
work=$out/work-$name
rm -rf "$work"
mkdir -p "$work/boot/licenses" "$work/docs/licenses"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
    in_vm bash "$NIBOS_ROOT/kernel/build.sh"
    in_vm bash -c "cd '$NIBOS_ROOT/hw' && PKGDEST=\"\$PWD/out\" BUILDDIR=\"\$HOME/makepkg-build\" makepkg -f --nodeps"
fi
# SPL in Falcon mode (the same as a dev card's), and the fallback U-Boot booting this
# layout's root (partition 3) when A is held.
in_vm bash -c "FALCON=1 OUT='$work' bash '$NIBOS_ROOT/uboot/build.sh'" >/dev/null
in_vm bash -c "FALLBACK=1 ROOT_PART=3 OUT='$work' bash '$NIBOS_ROOT/uboot/build.sh'" >/dev/null

# The command line, as kernel/deploy.sh's (it says why each argument is there), on this
# layout's root.
bootargs="root=PARTLABEL=nibos-root-a rootfstype=ext4 rootwait rw firmware_class.path=/var/lib/nibos/firmware console=tty1 fbcon=font:TER16x32 systemd.tty.rows.console=$BOARD_CONSOLE_ROWS systemd.tty.columns.console=$BOARD_CONSOLE_COLUMNS SYSTEMD_DEFAULT_MOUNT_RATE_LIMIT_BURST=20 watchdog.open_timeout=60 quiet loglevel=3"
# Both editions: the splash puts the boot picture up at 1.5 s (init=), in the Falcon FIT only,
# as kernel/deploy.sh does, so the fallback keeps the plain command line; with it, no console
# cursor (vt.global_cursor_default=0, as kernel/deploy.sh says).
falconargs="$bootargs init=/usr/lib/nibos/splash vt.global_cursor_default=0"
in_vm_sh "cd kernel && gzip -dc out/Image.gz > '$work/Image' && python3 ../uboot/mkfalcon.py \"\$RKBIN/\$SOC_RKBIN_BL31\" '$work/Image' out/\$BOARD_DTB.dtb '$falconargs' '$work/falcon.itb' --memory \$BOARD_MEMORY --description 'nibOS $version for $BOARD_NAME' && rm '$work/Image'"

# The notice and the licences no package installs: rkbin's, and the kernel's and U-Boot's.
extra=
if [ "$edition" = dslot ]; then
    commit() { git -C "$1" rev-parse --short=12 HEAD; }
    parent=$(cd "$NIBOS_ROOT/.." && pwd)
    extra="- DSlot (the boot UI) at $(commit "$DSLOT_SRC"): GPL-3.0-or-later.
- DSperate (DS, GBA) at $(commit "${DSPERATE_SRC:-$parent/DSperate}"): GPL-3.0-or-later.
- inkdot (GB, GBC) at $(commit "${INKDOT_SRC:-$parent/inkdot}"): MIT OR Apache-2.0.
- nibcase (the libretro frontend) at $(commit "${NIBCASE_SRC:-$parent/nibcase}"): MIT, Apache-2.0. Its cores,
  each a package of its own: Snes9x (non-commercial, see below), Nestopia, BlastEm,
  Beetle PCE Fast, Beetle NeoPop, Beetle WonderSwan, Gearsystem, Handy, Game & Watch (gw),
  PCSX ReARMed: their licences are in the package list below.
- inktick (Game & Watch, a preview) at $(commit "${INKTICK_SRC:-$parent/inktick}"): MIT OR Apache-2.0.
- Roboto Condensed (in DSlot and the emulators' menus): SIL OFL 1.1.
- nibOS's PortMaster package: portmaster-get and the launchers (GPL-3.0-or-later) and the
  nibOS profile (MIT). PortMaster is downloaded on request from github.com/PortsMaster; not
  included. It and what it downloads come under their own licences.
"
fi
# shellcheck disable=SC2016 # expanded in the build machine
read -r rkbin_commit kernel_commit < <(in_vm_sh 'echo "$(git -C "$RKBIN" rev-parse --short=12 HEAD) $(git -C "$KSRC" rev-parse --short=12 HEAD)"')
in_vm_sh "cp \"\$RKBIN/LICENSE\" '$work/docs/licenses/rkbin.txt'"
sed -e "s|@VERSION@|$version|" -e "s|@EDITION@|$edition|" \
    -e "s|@NIBOS_COMMIT@|$(git -C "$NIBOS_ROOT" rev-parse --short=12 HEAD)|" \
    -e "s|@KERNEL_COMMIT@|$kernel_commit|" -e "s|@UBOOT_TAG@|$UBOOT_TAG|" \
    -e "s|@RKBIN_DDR@|${BOARD_RKBIN_DDR##*/}|" -e "s|@RKBIN_BL31@|${SOC_RKBIN_BL31##*/}|" \
    -e "s|@RKBIN_COMMIT@|$rkbin_commit|" \
    -e "s|@SOURCE_URL@|${SOURCE_URL:-the place this image is downloaded from}|" \
    "$NIBOS_ROOT/rootfs/image/NOTICE.in" |
    EXTRA=$extra awk '$0 == "@EXTRA_COMPONENTS@" { printf "%s", ENVIRON["EXTRA"]; next } { print }' > "$work/docs/NOTICE"
mkdir -p "$work/docs/licenses/rkbin"
mv "$work/docs/licenses/rkbin.txt" "$work/docs/licenses/rkbin/LICENSE"

# nibkit (NIBKIT_REPO at NIBKIT_TAG), for its licences and for the packages built from other
# repositories, laid out the way their PKGBUILDs look for it (NIBKIT=, _nibkit=): nibkit with a
# LICENSE, and nibkit-menu-c beside it, each standing alone.
nibkit=$work/nibkit
rm -rf "$nibkit"
git -c advice.detachedHead=false clone -q --depth 1 --branch "$NIBKIT_TAG" "$NIBKIT_REPO" "$nibkit/repo"
mkdir -p "$nibkit/nibkit"
cp -R "$nibkit/repo/"{Cargo.toml,Cargo.lock,build.rs,src,assets,examples,README.md,CHANGELOG.md} "$nibkit/nibkit/"
cp "$nibkit/repo/LICENSE-MIT" "$nibkit/nibkit/LICENSE"
cp -R "$nibkit/repo/nibkit-menu-c" "$nibkit/nibkit-menu-c"
cp "$nibkit/repo/Cargo.lock" "$nibkit/nibkit-menu-c/"
python3 - "$nibkit" <<'PY'
import re, sys
root = sys.argv[1]
# nibkit without its workspace (nibkit-menu-c isn't inside it here), nibkit-menu-c on its own.
p = f"{root}/nibkit/Cargo.toml"
s = open(p).read()
s = re.sub(r"\[workspace\]\n(?:[^\[\n].*\n|\n)*?(?=\[package\])", "", s, count=1)
open(p, "w").write(s)
p = f"{root}/nibkit-menu-c/Cargo.toml"
s = open(p).read().replace('path = ".."', 'path = "../nibkit"')
open(p, "w").write(s + "\n[workspace]\n")
PY

# The DSlot edition's packages and binaries, built from the projects' checkouts (fresh makepkg
# directories, so no source clone from another build is reused).
dslot_env=()
if [ "$edition" = dslot ]; then
    parent=$(cd "$NIBOS_ROOT/.." && pwd)
    dsperate=${DSPERATE_SRC:-$parent/DSperate} inkdot=${INKDOT_SRC:-$parent/inkdot}
    for d in "$DSLOT_SRC" "$dsperate" "$inkdot"; do [ -d "$d" ] || die "no $d"; done
    nibcase=${NIBCASE_SRC:-$parent/nibcase} inktick=${INKTICK_SRC:-$parent/inktick}
    pkgs=$work/pkgs
    mkdir -p "$pkgs"
    mk="BUILDDIR=\$HOME/nibos-image-build/build SRCDEST=\$HOME/nibos-image-build/src PKGDEST='$pkgs' makepkg -f --nodeps"
    # set -e: a package that fails to build stops the image, rather than leaving it out.
    in_vm bash -c "set -e; rm -rf \$HOME/nibos-image-build && mkdir -p \$HOME/nibos-image-build/src
        pacman -Q sdl2-compat >/dev/null 2>&1 || sudo pacman -S --noconfirm sdl2-compat
        cd '$DSLOT_SRC/packaging/nibos' && NIBKIT='$nibkit/nibkit' $mk
        cd '$dsperate/packaging/arch' && _src='$dsperate' _nibkit='$nibkit/nibkit-menu-c' $mk
        cd '$inkdot/packaging/nibos' && NIBKIT='$nibkit/nibkit' $mk
        cd '$nibcase/packaging/nibos' && NIBKIT='$nibkit/nibkit' $mk
        for c in beetle-ngp beetle-wswan gearsystem gw handy pcsx-rearmed; do (cd '$nibcase/packaging/cores/'\$c && $mk); done
        cd '$inktick/packaging/nibos' && NIBKIT='$nibkit/nibkit' $mk
        cd '$NIBOS_ROOT/rootfs/portmaster' && $mk
        cd '$NIBOS_ROOT/rootfs/readahead' && mkdir -p out && gcc -O2 -static -o out/nibos-readahead readahead.c
        cd '$NIBOS_ROOT/rootfs/safe-mode' && cargo build --release --quiet
        rm -rf \$HOME/nibos-image-build" >/dev/null
    if ! ls "$NIBOS_ROOT"/rootfs/mesa/out/mesa-nibos-*.pkg.tar.* >/dev/null 2>&1; then
        in_vm bash -c "mkdir -p ~/mesa-build/nibos && cp '$NIBOS_ROOT/rootfs/mesa/PKGBUILD' ~/mesa-build/nibos/ && cd ~/mesa-build/nibos && PKGDEST='$NIBOS_ROOT/rootfs/mesa/out' makepkg -s --noconfirm" >/dev/null
    fi
    cp "$NIBOS_ROOT"/rootfs/mesa/out/mesa-nibos-*.pkg.tar.* "$pkgs/"
    dslot_env=(DSLOT=1 SAFE_MODE="$NIBOS_ROOT/rootfs/safe-mode" PREVIEW_EMULATORS=inktick
        READAHEAD="$NIBOS_ROOT/rootfs/readahead/out/nibos-readahead"
        PACKAGES="$(find "$pkgs" -name '*.pkg.tar.*' | sort | tr '\n' ' ')")
fi

# The network: the base edition starts Wi-Fi at boot (SSH is its way in); the DSlot edition
# only when asked (DSlot's Wi-Fi switch, `nib net up`): idle, the radio costs ~0.5 W, a quarter
# of the shelf's draw (docs/engineering/idle-power.md).
network=boot wifi_note=
if [ "$edition" = dslot ]; then
    network=demand
    wifi_note="
Wi-Fi is off until you turn it on in DSlot's settings; it costs battery while on."
fi

# The package that stands in for libfdk-aac (no Bluetooth AAC: rootfs/no-fdk-aac).
nofdk_dir=$NIBOS_ROOT/rootfs/no-fdk-aac
in_vm bash -c "cd '$nofdk_dir' && PKGDEST='$nofdk_dir/out' BUILDDIR=\$HOME/makepkg-build makepkg -f --nodeps >/dev/null"
nofdk=$(find "$nofdk_dir/out" -name 'nibos-no-fdk-aac-*.pkg.tar.*' | sort | tail -1)

# The splash (init=, and the goodbye at power-off) and its pictures, in both editions.
splash=$NIBOS_ROOT/rootfs/splash
in_vm bash -c "cd '$splash' && cargo build --release --quiet && mkdir -p out &&
    target/release/nibos-pspl out/boot.pspl pictures/logo.png >/dev/null &&
    target/release/nibos-pspl out/goodbye.pspl pictures/goodbye.png >/dev/null"

# The root.
in_vm sudo IMAGE=1 HWPKG="${HWPKG:-}" NOFDK="$nofdk" SPLASH="$splash/target/release/nibos-splash" SPLASH_PICTURES="$splash/out" \
    SPLASH_UNIT="$splash/nibos-goodbye.service" NETWORK_MODE=$network BOOTARGS="$bootargs" DOCS="$work/docs" ${dslot_env[@]+"${dslot_env[@]}"} \
    bash "$NIBOS_ROOT/rootfs/make-arch.sh"
mv "$NIBOS_ROOT/rootfs/out/image-root.img" "$work/root.img"

# What the user sees on the NIBOS partition.
cp "$NIBOS_ROOT/rootfs/image/provision.toml.example" "$work/boot/"
cp "$NIBOS_ROOT/rootfs/out/image-NOTICE" "$work/boot/NOTICE"
cp "$NIBOS_ROOT/rootfs/out/image-packages.txt" "$work/boot/packages.txt"
cp "$work/docs/licenses/rkbin/LICENSE" "$work/boot/licenses/rkbin.txt"
in_vm cp /usr/share/licenses/linux-firmware-realtek/LICENCE.rtlwifi_firmware.txt \
    /usr/share/licenses/linux-firmware-whence/WHENCE \
    /usr/share/licenses/spdx/GPL-2.0-only.txt "$work/boot/licenses/"
cp "$NIBOS_ROOT/LICENSE" "$work/boot/licenses/nibos-GPL-3.0.txt"
cp "$nibkit/repo/LICENSE-MIT" "$work/boot/licenses/nibkit-MIT.txt"
cp "$nibkit/repo/assets/OFL.txt" "$work/boot/licenses/Roboto-Condensed-OFL.txt"
cat > "$work/boot/README.txt" <<EOF
nibOS $version for the $BOARD_NAME

1. Copy provision.toml.example to provision.toml here, and fill in your Wi-Fi network,
   an SSH key, a password (or none) and your time zone.
2. Put the card in the slot marked TF1 and power on. The first start makes the games
   partition (NIBOS-DATA) over the rest of the card.$wifi_note

NOTICE says what is in this image and under which licences.
EOF

in_vm sudo bash "$here/assemble-image.sh" "$work" "$out/$name.img"
in_vm bash -c "cd '$out' && xz -T0 -6 -f '$name.img' && { grep -v '  $name.img.xz\$' SHA256SUMS 2>/dev/null || true; sha256sum '$name.img.xz'; } > SHA256SUMS.new && mv SHA256SUMS.new SHA256SUMS"
ls -la "$out/$name.img.xz"
grep -F "$name.img.xz" "$out/SHA256SUMS"
