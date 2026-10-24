#!/bin/bash
# Build mainline U-Boot for the board (NIBOS_BOARD): out/u-boot-rockchip.bin.
#
#   orb -m rgbuild bash uboot/build.sh         (from the repository, on the Mac)
#
# Runs in the build machine. U-Boot at UBOOT_TAG with the generic, SoC and board
# patches, from the board's defconfig (BOARD_UBOOT_DEFCONFIG; for the RG DS+
# mainline's RG DS board: U-Boot only needs the PMIC, the SD slot and RAM). DDR
# init and TF-A (BL31) are Rockchip's binaries from rkbin at RKBIN_COMMIT, as in
# Armbian's RG DS build (BOARD_RKBIN_DDR, SOC_RKBIN_BL31). No OP-TEE, no boot
# logo.
#
# u-boot-rockchip.bin goes at sector 64 of the card: the SPL loader there, U-Boot
# itself at 8 MiB (sector 16384, the stock "uboot" partition). See install.sh.
#
# ROOT_PART (default BOARD_ROOT_PART) is the partition U-Boot boots extlinux.conf
# from, and OUT (default uboot/out) where the result goes: tools/make-image.sh
# builds the downloadable image's fallback U-Boot with its own layout's.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
src=$USRC
rkbin=$RKBIN
tag=$UBOOT_TAG
ddr=$rkbin/$BOARD_RKBIN_DDR
bl31=$rkbin/$SOC_RKBIN_BL31
board=$NIBOS_ROOT/boards/$NIBOS_BOARD
patches=("$here"/patches/0*.patch "$NIBOS_ROOT/socs/$BOARD_SOC"/uboot/patches/*.patch "$board"/uboot/patches/*.patch)

# Both sources at the versions config.sh pins: the patches are made against
# UBOOT_TAG, and RKBIN_COMMIT fixes the DDR init and TF-A binaries.
if [ ! -d "$src" ]; then
    git clone -q --depth 1 -b "$tag" https://source.denx.de/u-boot/u-boot.git "$src"
fi
[ "$(git -C "$src" rev-parse HEAD)" = "$(git -C "$src" rev-parse "$tag^{commit}" 2>/dev/null)" ] ||
    die "$src is not at $tag (UBOOT_TAG in config.sh): check out $tag there"
if [ ! -d "$rkbin" ]; then
    git init -q "$rkbin"
    git -C "$rkbin" fetch -q --depth 1 https://github.com/rockchip-linux/rkbin.git "$RKBIN_COMMIT"
    git -C "$rkbin" checkout -q FETCH_HEAD
fi
[ "$(git -C "$rkbin" rev-parse HEAD)" = "$RKBIN_COMMIT" ] ||
    die "$rkbin is not at $RKBIN_COMMIT (RKBIN_COMMIT in config.sh)"
[ -f "$ddr" ] && [ -f "$bl31" ] || die "rkbin has no $(basename "$ddr") or $(basename "$bl31")"

cd "$src"
git checkout -q -- . && git clean -qfd
# FALLBACK=1 builds the U-Boot that went to the card first and booted (the
# first two generic patches, bootstage marks and Falcon support, and plain
# sysboot), as out/u-boot-fallback.itb: install.sh --fallback puts it at
# BOARD_FALLBACK_SECTOR, where SPL loads it when A is held.
[ "${FALLBACK:-0}" = 1 ] && patches=("$here"/patches/000[12]-*.patch)
for p in "${patches[@]}"; do
    [ -e "$p" ] || continue
    echo "patch: ${p#"$NIBOS_ROOT"/}"
    git apply "$p"
done
make -s mrproper
make -s "$BOARD_UBOOT_DEFCONFIG"
# Boot straight from the root partition's extlinux.conf (mmc $BOARD_UBOOT_MMC,
# partition $BOARD_ROOT_PART, which U-Boot numbers in hex), no two-second key
# wait. If that fails, try again with the card at 25 MHz (U-Boot rk3568/0002
# doubled the SD clock to the 50 MHz it always claimed), then a full scan of
# every device. MEASURE=1 boots with each step marked instead (bootstage,
# stashed at 0x0f000000 for Linux to read: see measure.sh).
# \${...} below are U-Boot's own variables, expanded by U-Boot at boot.
mmc=$BOARD_UBOOT_MMC
root=$(printf 'mmc %d:%x' "$BOARD_UBOOT_MMC" "${ROOT_PART:-$BOARD_ROOT_PART}")
sysboot="sysboot $root any \${scriptaddr} /boot/extlinux/extlinux.conf"
bootcmd="$sysboot; mmc dev $mmc 0 0; $sysboot; bootflow scan -lb"
[ "${FALLBACK:-0}" = 1 ] && bootcmd="$sysboot; bootflow scan -lb"
if [ "${MEASURE:-0}" = 1 ]; then
    # Times SD init (mmc info shows the bus mode), loading /boot/Image.gz and
    # unpacking it, then boots through extlinux as a normal boot does (the
    # kernel is read again); the console log goes to 0x0f004000 for measure.sh.
    # The CPU PLL registers go to the log.
    m='bootstage mark'
    # shellcheck disable=SC2016
    c='${kernel_comp_addr_r}' k='${kernel_addr_r}'
    bootcmd="$m bootcmd; mmc dev $mmc; $m mmc_dev; mmc info; $m mmc_info"
    # MEASURE_RETRY=1: then re-init the card as the boot command's retry does
    # (High Speed, 3.3 V, after a UHS init) and load the kernel that way.
    [ "${MEASURE_RETRY:-0}" = 1 ] && bootcmd+="; mmc dev $mmc 0 0; $m mmc_retry; mmc info"
    bootcmd+="; load $root \${scriptaddr} /boot/extlinux/extlinux.conf; $m load_extlinux"
    bootcmd+="; md.l 0xfdd20000 3; md.l 0xfdd200c0 1; md.l 0xfdd20100 3"
    bootcmd+="; load $root $c /boot/Image.gz; $m load_gz"
    bootcmd+="; bootstage decomp gzip $c \${filesize} $k; $m decomp_gz"
    bootcmd+="; bootstage conlog 0x0f004000 0x1c000; $sysboot"
    bootcmd+="; mmc dev $mmc 0 0; $sysboot"
    for p in "$here"/patches/measure/*.patch; do git apply "$p"; done
fi
# The board's own settings (boards/<board>/uboot/<board>.config), merged before
# the ones below and settled by the one olddefconfig at the end.
merge() { scripts/kconfig/merge_config.sh -m -O . .config "$@" >/dev/null; }
merge "$board/uboot/$NIBOS_BOARD.config"
scripts/config \
    --set-val BOOTDELAY 0 \
    -e CMD_SYSBOOT -e MMC_SPEED_MODE_SET \
    --set-str BOOTCOMMAND "$bootcmd" \
    -e BOOTSTAGE -e BOOTSTAGE_REPORT -e CMD_BOOTSTAGE \
    -e SPL_BOOTSTAGE -e BOOTSTAGE_STASH --set-val BOOTSTAGE_STASH_ADDR 0x0f000000 \
    --set-val BOOTSTAGE_STASH_SIZE 0x4000
# No UEFI: we boot with sysboot/booti, and making EFI disk objects for the SD
# card's partitions took 0.22 s of every boot. zstd for the kernel, full size.
# SPL doesn't hash-check U-Boot and TF-A (no secure boot here; it cost most of
# SPL's 0.12 s load). 8 MiB of malloc instead of 32: U-Boot zeroes it at start.
# UHS SDR104 for the SD card (U-Boot rk3568/0003): 1.8 V signalling, 150 MHz,
# tuned.
[ "${FALLBACK:-0}" = 1 ] || scripts/config -d EFI_LOADER -e ZSTD -d ZSTD_LIB_MINIFY \
    -d SPL_FIT_SIGNATURE --set-val SYS_MALLOC_LEN 0x800000 \
    -e MMC_IO_VOLTAGE -e MMC_UHS_SUPPORT
[ "${MEASURE:-0}" = 1 ] &&
    scripts/config -e CONSOLE_RECORD --set-val CONSOLE_RECORD_OUT_SIZE 0x10000 \
        --set-val BOOTSTAGE_RECORD_COUNT 250 --set-val SPL_BOOTSTAGE_RECORD_COUNT 60 \
        --set-val SPL_SYS_MALLOC_F_LEN 0x10000 --set-val SYS_MALLOC_F_LEN 0x40000
# FALCON=1: SPL starts Linux itself, through TF-A (U-Boot generic/0002), from a
# FIT made by mkfalcon.py at the start of the board's Falcon partition
# (BOARD_FALCON_SECTOR). No U-Boot proper. SPL doesn't check hashes: SHA-256
# over the kernel costs more than it's worth here.
if [ "${FALCON:-0}" = 1 ]; then
    # SPL reads the 17 MB kernel at UHS SDR104 (U-Boot rk3568/0003, rg-ds-plus/0003).
    scripts/config --set-val SYS_MMCSD_RAW_MODE_U_BOOT_SECTOR "$(printf '%#x' "$BOARD_FALCON_SECTOR")" -d SPL_FIT_SIGNATURE \
        -e SPL_MMC_IO_VOLTAGE -e SPL_MMC_UHS_SUPPORT -e SPL_DRIVERS_MISC -e SPL_ROCKCHIP_IODOMAIN
    # The board's way to 1.8 V in SPL (boards/<board>/uboot/falcon.config).
    merge "$board/uboot/falcon.config"
    # FALCON_DEBUG=1: SPL writes what it hands TF-A to SD sector 24000.
    if [ "${FALCON_DEBUG:-0}" = 1 ]; then
        for p in "$here"/patches/falcon-debug/*.patch; do git apply "$p"; done
        scripts/config -e SPL_MMC_WRITE
    fi
fi
make -s olddefconfig
make -j"$(nproc)" BL31="$bl31" ROCKCHIP_TPL="$ddr"

out=${OUT:-$here/out}
mkdir -p "$out"
if [ "${FALLBACK:-0}" = 1 ]; then
    cp u-boot.itb "$out/u-boot-fallback.itb"
    ls -la "$out/u-boot-fallback.itb"
elif [ "${FALCON:-0}" = 1 ]; then
    cp idbloader.img "$out/idbloader-falcon.img"
    ls -la "$out/idbloader-falcon.img"
else
    cp u-boot-rockchip.bin "$out/"
    ls -la "$out/u-boot-rockchip.bin"
fi
grep -E "^CONFIG_(BOOTDELAY|BOOTCOMMAND|CMD_SYSBOOT)=" .config
