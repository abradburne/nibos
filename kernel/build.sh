#!/bin/bash
# Build the nibOS kernel for the board (NIBOS_BOARD): mainline Linux at
# KERNEL_COMMIT, the generic, SoC and board patches, the board's device tree,
# and the configuration from defconfig and the three fragments.
#
#   orb -m rgbuild bash kernel/build.sh        (from the repository, on the Mac)
#
# Runs in the build machine (native aarch64, no cross compiler). The kernel
# tree (KSRC) lives in the machine's own file system, because the Mac volume is
# case-insensitive and the kernel tree is not; build.sh resets and cleans it on
# every run, so keep no work there.
#
# SMOKE=1 builds smoke/init into the kernel as its initramfs, with busybox and
# the board's firmware: a kernel that tests the hardware without any root file
# system (see smoke/init).
#
# Output in out/: Image.gz, $BOARD_DTB.dtb, modules.tar.zst, config.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
board="$NIBOS_ROOT/boards/$NIBOS_BOARD"
soc="$NIBOS_ROOT/socs/$BOARD_SOC"
out="$here/out"
dts_dir=arch/arm64/boot/dts/$SOC_DTS_DIR

cd "$KSRC"
# The patches are made against one commit; another tree may not take them, or
# may build a different kernel without saying so.
head=$(git rev-parse HEAD)
[ "$head" = "$KERNEL_COMMIT" ] ||
    die "$KSRC is at ${head:0:12}, not $KERNEL_COMMIT (KERNEL_COMMIT in config.sh): git -C $KSRC checkout $KERNEL_COMMIT"

# A clean tree each time, then the patches (generic, SoC, board, in that order)
# and the board's device tree.
git reset -q --hard
git clean -qfd -- "$dts_dir" drivers/gpu/drm/panel
for p in "$here"/patches/*.patch "$soc"/kernel/patches/*.patch "$board"/kernel/patches/*.patch; do
    [ -e "$p" ] || continue
    echo "patch: ${p#"$NIBOS_ROOT"/}"
    patch -p1 --no-backup-if-mismatch < "$p"
done
cp "$board/dts/$BOARD_DTB.dts" "$dts_dir/"
grep -q "$BOARD_DTB.dtb" "$dts_dir/Makefile" ||
    echo "dtb-\$(CONFIG_$SOC_KERNEL_PLATFORM) += $BOARD_DTB.dtb" >> "$dts_dir/Makefile"

# The configuration, in two rounds like scripts/config and olddefconfig would
# do it by hand. First only the SoC's platform: arm64 defconfig builds in
# drivers for every other SoC family too, which the bootloader then has to read
# and unpack at every start. Then the fragments, the later one winning.
make -s defconfig
platform=$(mktemp)
fragments=$(mktemp)
trap 'rm -f "$platform" "$fragments"' EXIT
grep -oE "^config ARCH_[A-Z0-9_]+" arch/arm64/Kconfig.platforms | sed "s/^config //" |
    grep -vx "$SOC_KERNEL_PLATFORM" | sed 's/.*/# CONFIG_& is not set/' > "$platform"
cat "$here/config/platform.config" >> "$platform"
merge() { scripts/kconfig/merge_config.sh -m -O . .config "$@" >/dev/null; }
merge "$platform"
make -s olddefconfig
set -- "$here/config/nibos.config" "$soc/kernel/$BOARD_SOC.config" "$board/kernel/$NIBOS_BOARD.config"
if [ "${SMOKE:-0}" = 1 ]; then
    root=$HOME/smoke-initramfs
    rm -rf "$root"
    mkdir -p "$root"/{bin,dev,proc,sys,tmp,mnt,lib/firmware}
    cp /usr/bin/busybox "$root/bin/"
    cp "$here/smoke/init" "$root/init"
    chmod 755 "$root/init" "$root/bin/busybox"
    grep -v '^#' "$board/kernel/smoke-firmware" | while read -r f; do
        case "$f" in
        stock/*) src=$STOCK_DIR/${f#stock/} dest=${f#stock/} ;;
        *) src=/usr/lib/firmware/$f dest=$f ;;
        esac
        mkdir -p "$root/lib/firmware/$(dirname "$dest")"
        # shellcheck disable=SC2086 # the list's entries may be globs
        cp $src "$root/lib/firmware/$(dirname "$dest")/"
    done
    # /dev/console has to exist before init runs; a directory source can't
    # hold a device node made without root, so it comes from a cpio list.
    printf 'dir /dev 755 0 0\nnod /dev/console 600 0 0 c 5 1\n' > "$root.list"
    printf 'CONFIG_INITRAMFS_SOURCE="%s %s"\n' "$root" "$root.list" > "$fragments"
    set -- "$@" "$here/smoke/smoke.config" "$board/kernel/smoke.config" "$fragments"
fi
merge "$@"
make -s olddefconfig

make -j"$(nproc)" Image.gz modules "$SOC_DTS_DIR/$BOARD_DTB.dtb"

mkdir -p "$out" && rm -f "$out"/{Image.gz,*.dtb,config,modules.tar.zst}
cp arch/arm64/boot/Image.gz "$out/"
cp "$dts_dir/$BOARD_DTB.dtb" "$out/"
cp .config "$out/config"
stage=$(mktemp -d)
make -s INSTALL_MOD_PATH="$stage" INSTALL_MOD_STRIP=1 modules_install
tar -C "$stage" --zstd -cf "$out/modules.tar.zst" lib
rm -rf "$stage"

echo "kernel $(make -s kernelrelease)"
ls -la "$out"
