#!/bin/bash
# Take from Anbernic's stock firmware image (STOCK_IMAGE) the files nibOS needs
# but may not redistribute, into STOCK_DIR (boards/rg-ds-plus/stock/, not in
# git):
#
#   boot.img          partition 3, the stock U-Boot's FIT: kernel/mkboot.py
#                     builds boot images in its layout, with its resource block
#   aw88166_acf.bin   the speaker amps' tuning file (Awinic's), from the stock
#                     root file system: make-arch.sh and hw/install-firmware.sh
#                     install it (never a package)
#   stock.dtb, .dts   the stock device tree and its decompiled source (nodes
#                     sorted), which dts/rk3568-anbernic-rg-ds-plus.dts was written from
#
#   boards/rg-ds-plus/extract-stock.sh        (on the Mac or in the build machine)
#
# Needs debugfs (e2fsprogs) and dtc: run in the build machine, which it does by
# itself when started on the Mac. Reads the image only.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../tools/lib.sh
. "$here/../../tools/lib.sh"
if [ "$(uname -s)" != Linux ]; then
    in_vm bash "$here/extract-stock.sh"
    exit
fi
[ -f "$STOCK_IMAGE" ] || die "no stock image at $STOCK_IMAGE (STOCK_IMAGE in config.sh)"
for tool in debugfs dtc; do command -v "$tool" >/dev/null || die "needs $tool"; done
mkdir -p "$STOCK_DIR"

# The stock card's layout (sectors of 512 bytes): partition 3 "boot" at 32768,
# 64 MiB; partition 8 "rootfs" (ext4) at 491520.
dd if="$STOCK_IMAGE" of="$STOCK_DIR/boot.img" bs=512 skip=32768 count=131072 status=none
debugfs -R "dump /lib/firmware/aw883xx_acf.bin $STOCK_DIR/aw88166_acf.bin" \
    "$STOCK_IMAGE?offset=$((491520 * 512))" 2>/dev/null
[ -s "$STOCK_DIR/aw88166_acf.bin" ] || die "no amp tuning file in the stock root file system"

# The device tree is the FIT's "fdt" image (read with mkboot.py's FIT reader).
cd "$NIBOS_ROOT/kernel"
PYTHONDONTWRITEBYTECODE=1 python3 - "$STOCK_DIR/boot.img" "$STOCK_DIR/stock.dtb" <<'PY'
import sys
import mkboot
fit = open(sys.argv[1], "rb").read()
open(sys.argv[2], "wb").write(mkboot.fit_image(fit, mkboot.fdt_props(fit), "fdt"))
PY
dtc -q -s -I dtb -O dts -o "$STOCK_DIR/stock.dts" "$STOCK_DIR/stock.dtb"

ls -la "$STOCK_DIR"
