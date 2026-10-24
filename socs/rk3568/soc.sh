# shellcheck shell=sh
# The Rockchip RK3568, as the build scripts see it. Read by config.sh for every
# board whose BOARD_SOC is rk3568; README.md next to this file describes what
# nibOS changes for it. Every SoC under socs/ sets the same names
# (socs/README.md).
# shellcheck disable=SC2034 # read by the scripts that source config.sh

SOC_NAME="Rockchip RK3568"
# The Kconfig platform the kernel is trimmed to (every other ARCH_* is off) and
# the device tree directory under arch/arm64/boot/dts/.
SOC_KERNEL_PLATFORM=ARCH_ROCKCHIP
SOC_DTS_DIR=rockchip
# TF-A (BL31) from rkbin: v1.46, the version the stock firmware runs. U-Boot
# packs it, and SPL hands it the kernel in Falcon mode.
SOC_RKBIN_BL31=bin/rk35/rk3568_bl31_v1.46.elf
