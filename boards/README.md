# Boards

nibOS runs on one board, the **Anbernic RG DS+** ([`rg-ds-plus/`](rg-ds-plus/README.md)). Other
handhelds are a goal for later, and the layout is ready for them: what is the same everywhere
(the build scripts, the hardware layer's daemons, the root file system, the boot UI) lives in
the top-level directories; what one SoC family shares lives in [`socs/<soc>/`](../socs/README.md);
what only one device has lives here, in `boards/<board>/`. `NIBOS_BOARD` in
[`config.sh`](../config.sh) picks the board every script builds and installs for.

## What a board provides

| Path | What | Used by |
|---|---|---|
| `board.sh` | The board as the scripts see it: name, SoC, device tree, U-Boot defconfig, the card's layout, console size, RAM | Every host script, through [`config.sh`](../config.sh) |
| `README.md` | The hardware: what it has, what works, known issues | People |
| `dts/<dtb>.dts` | The device tree | [`kernel/build.sh`](../kernel/build.sh) |
| `kernel/patches/` | Kernel patches only this board needs (`git format-patch` output) | `kernel/build.sh`, after the generic and SoC patches |
| `kernel/<board>.config` | The board's kernel config fragment: its parts built in, what it lacks off | `kernel/build.sh`, merged last |
| `kernel/smoke.config`, `kernel/smoke-firmware` | The smoke-test kernel's extra drivers and firmware | `SMOKE=1 kernel/build.sh` |
| `uboot/patches/`, `uboot/<board>.config`, `uboot/falcon.config` | U-Boot patches and config fragments for the board, and for its Falcon SPL | [`uboot/build.sh`](../uboot/build.sh) |
| `hw/files/` | The hardware layer's board files, installed over the shared ones: `usr/lib/nibos/board.conf` (device names for the daemons), udev rules, WirePlumber rules, thermal limits, modules | [`hw/`](../hw/README.md) (the `nibos-hw` package) |
| `rootfs/files/` | Files for the root image only this board's card needs | [`rootfs/make-arch.sh`](../rootfs/make-arch.sh) |
| `extract-stock.sh` | Takes from the vendor's firmware image what nibOS needs but may not redistribute into `stock/`, which git ignores | `kernel/deploy.sh`, `hw/PKGBUILD` |

`make boards` (part of `make check`) checks that the board's `board.sh` sets every name the
scripts read and that it has the files each layer reads from it.
