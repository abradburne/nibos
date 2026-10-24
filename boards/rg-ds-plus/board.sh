# shellcheck shell=sh
# The Anbernic RG DS+, as the build and install scripts see it. Read by
# config.sh when NIBOS_BOARD=rg-ds-plus; README.md next to this file describes
# the hardware. Every board under boards/ sets the same names
# (boards/README.md).
# shellcheck disable=SC2034 # read by the scripts that source config.sh

BOARD_NAME="Anbernic RG DS+"
BOARD_SOC=rk3568
BOARD_HOSTNAME=rgds
# Anbernic's firmware image (V1.0, 2026-09-15). extract-stock.sh takes from it
# what nibOS needs and may not redistribute, into stock/ (STOCK_DIR).
BOARD_STOCK_IMAGE=RG-DS-PLUS-EN16GB-V1.0-20260915.IMG
# Of those, the firmware files the device needs: the speaker amps' tuning file.
# Never packaged; rootfs/make-arch.sh and hw/install-firmware.sh copy the user's
# own into /usr/lib/firmware.
BOARD_FIRMWARE=aw88166_acf.bin

# --- Kernel -----------------------------------------------------------------

# The device tree (dts/), built as arch/arm64/boot/dts/$SOC_DTS_DIR/$BOARD_DTB.dtb.
BOARD_DTB=rk3568-anbernic-rg-ds-plus
# The VT size systemd is told on the command line: Terminus 16x32 on a
# 1024x768 panel. Without it PID 1 asks the console with an xterm query the VT
# never answers and waits 333 ms for the reply.
BOARD_CONSOLE_ROWS=24
BOARD_CONSOLE_COLUMNS=64

# RAM as mainline U-Boot reports it, for the Falcon FIT's /memory node (no
# U-Boot proper fills it in): 1 GB, the first 2 MB left to TF-A.
BOARD_MEMORY="0x00200000 0x3fe00000"

# --- U-Boot -----------------------------------------------------------------

# Mainline U-Boot's RG DS board; the DS+ differs only where U-Boot doesn't look.
BOARD_UBOOT_DEFCONFIG=anbernic-rg-ds-rk3568_defconfig
# Rockchip's DDR init from rkbin: 1056 MHz covers the DS+'s LPDDR3.
BOARD_RKBIN_DDR=bin/rk35/rk3568_ddr_1056MHz_v1.26.bin

# --- The card ---------------------------------------------------------------

# The development card is a copy of the stock firmware's card, so its layout is
# Anbernic's (docs/architecture.md, "The SD card"). TF1 is mmc 1 in U-Boot and
# mmcblk0 in Linux.
BOARD_UBOOT_MMC=1
# The bootloader area: TPL + SPL at sector 64, up to the first partition.
BOARD_BOOTLOADER_START=64
BOARD_BOOTLOADER_END=24576
# Where SPL loads the fallback U-Boot from when A is held at power-on.
BOARD_FALLBACK_SECTOR=20480
# Partition 3, "boot": the stock U-Boot's FIT. 64 MiB.
BOARD_BOOT_PART=3
BOARD_BOOT_PARTNAME=boot
# Partition 4, "recovery": the Falcon FIT, which SPL reads by sector.
BOARD_FALCON_PART=4
BOARD_FALCON_PARTNAME=recovery
BOARD_FALCON_SECTOR=163840
# Partition 11, "ROMS" (FAT): games, saves, and nibOS's files for the user
# (ROMS/nibos/).
BOARD_ROMS_PART=11
# Partition 12, "archroot": the root file system, added over the free space.
BOARD_ROOT_PART=12
BOARD_ROOT_PARTLABEL=archroot
