# shellcheck shell=sh
# nibOS settings: which board, who the device's user is, where the device and the build
# machine are, and where the sources come from. Every host-side script reads
# this file through tools/lib.sh.
#
# Don't edit it for your own setup. Put your values in config.local.sh next to
# it (not in git; config.local.sh.example shows the format), or set them in the
# environment for one run:
#
#   DEVICE=me@192.168.1.20 kernel/deploy.sh
#
# Precedence: environment, then config.local.sh, then the defaults below. For
# that, config.local.sh sets values as the defaults here do, `: "${NAME:=value}"`
# (a plain NAME=value there would override the environment too).
# Plain POSIX sh, so both bash and sh scripts can source it.

# shellcheck disable=SC1091 # optional, and its path is only known at run time
[ -n "${NIBOS_ROOT:-}" ] && [ -f "$NIBOS_ROOT/config.local.sh" ] && . "$NIBOS_ROOT/config.local.sh"

# --- The board --------------------------------------------------------------

# Which handheld to build for: a directory under boards/ (boards/README.md).
# Its board.sh, and its SoC's socs/<soc>/soc.sh, describe the hardware to the
# build and install scripts; they are read at the end of this file.
: "${NIBOS_BOARD:=rg-ds-plus}"

# --- The device -------------------------------------------------------------

# The one login user made by rootfs/make-arch.sh: tty1 autologin, SSH, sudo
# without a password, owner of the ROMS volume, the user DSlot runs as.
: "${NIBOS_USER:=nibos}"
: "${NIBOS_UID:=1000}"
# Host name of the image; with mDNS (systemd-resolved) the device answers as
# $NIBOS_HOSTNAME.local. The default is the board's (BOARD_HOSTNAME).
: "${NIBOS_TIMEZONE:=UTC}"

# The SSH key the scripts log in with. Its .pub is what make-arch.sh authorises
# on the device (a development image: see docs/install.md). The default is the
# Mac's ~/.ssh/id_ed25519, which the OrbStack machine sees at the same path,
# also under sudo there; elsewhere (a Linux host) it is $HOME/.ssh/id_ed25519.
if [ -z "${SSH_KEY:-}" ]; then
    SSH_KEY=/Users/${SUDO_USER:-${USER:-$(id -un)}}/.ssh/id_ed25519
    [ -e "$SSH_KEY" ] || SSH_KEY=$HOME/.ssh/id_ed25519
fi
# SSH_KEY names the private key; an old habit of giving make-arch.sh the .pub
# still works.
case "$SSH_KEY" in *.pub) SSH_KEY=${SSH_KEY%.pub} ;; esac
: "${SSH_PUBKEY:=$SSH_KEY.pub}"

# Where the computer mounts the card's ROMS volume (FAT): the card scripts
# (kernel/make-dev-card.sh, kernel/flash-boot.sh, rootfs/install-arch.sh,
# uboot/restore-stock.sh, tools/card-image.sh) find the card by it. They use
# macOS's diskutil and run on a Mac today.
: "${ROMS_VOLUME:=/Volumes/ROMS}"

# --- The build machine ------------------------------------------------------

# The OrbStack Arch Linux ARM machine the kernel, U-Boot, packages and images
# are built in (docs/building.md). It sees the Mac's files at the same paths.
: "${BUILD_MACHINE:=rgbuild}"

# --- Sources ----------------------------------------------------------------

# Paths inside the build machine ($HOME is its home there): scripts that run on
# the Mac source tools/lib.sh again inside the machine to read them.
#
# Mainline Linux, inside the build machine (not on the case-insensitive Mac
# volume). kernel/build.sh refuses another commit unless KERNEL_COMMIT is
# changed too: the patches are made against this one.
: "${KSRC:=$HOME/linux}"
: "${KERNEL_COMMIT:=0fcb05b72344f40806cbe810de8872f1698bf053}"   # v7.3-rc7, torvalds master

# Mainline U-Boot and Rockchip's DDR init and TF-A binaries, also inside the
# build machine; uboot/build.sh clones them at these versions if missing.
: "${USRC:=$HOME/u-boot}"
: "${UBOOT_TAG:=v2026.10}"
: "${RKBIN:=$HOME/rkbin}"
: "${RKBIN_COMMIT:=3e288fe814e059dd06833495f845cab04ac20a5c}"   # rockchip-linux/rkbin, 2026-06-26

# DSlot, the boot UI (rootfs/dslot, rootfs/splash): a separate repository,
# next to this one (docs/building.md, "One-time setup").
: "${DSLOT_SRC:=$(cd "${NIBOS_ROOT:-.}/.." 2>/dev/null && pwd)/dslot}"

# --- What the board says ----------------------------------------------------

# shellcheck source=boards/rg-ds-plus/board.sh
. "${NIBOS_ROOT:-.}/boards/$NIBOS_BOARD/board.sh"
# shellcheck source=socs/rk3568/soc.sh
. "${NIBOS_ROOT:-.}/socs/$BOARD_SOC/soc.sh"

: "${NIBOS_HOSTNAME:=$BOARD_HOSTNAME}"
# Where the deploy, install and benchmark scripts reach the device (ssh target).
: "${DEVICE:=$NIBOS_USER@$NIBOS_HOSTNAME.local}"
# The vendor's firmware image for the board, next to this repository: the dev
# card starts as a copy of it, and boards/<board>/extract-stock.sh takes the
# files nibOS needs from it but may not redistribute.
: "${STOCK_IMAGE:=$(cd "${NIBOS_ROOT:-.}/.." 2>/dev/null && pwd)/$BOARD_STOCK_IMAGE}"
# Where extract-stock.sh puts them (not in git).
: "${STOCK_DIR:=${NIBOS_ROOT:-.}/boards/$NIBOS_BOARD/stock}"

# nibkit, the SDK library nibOS's programs and the emulators build on: its repository and the
# tag they use (tools/make-image.sh fetches it for the packages built from other repositories).
: "${NIBKIT_REPO:=https://github.com/abradburne/nibkit}"
: "${NIBKIT_TAG:=v0.1.0}"

export NIBKIT_REPO NIBKIT_TAG NIBOS_BOARD NIBOS_USER NIBOS_UID NIBOS_HOSTNAME NIBOS_TIMEZONE DEVICE SSH_KEY SSH_PUBKEY ROMS_VOLUME \
    BUILD_MACHINE KSRC KERNEL_COMMIT USRC UBOOT_TAG RKBIN RKBIN_COMMIT DSLOT_SRC STOCK_IMAGE STOCK_DIR
