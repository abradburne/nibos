# SoCs

What every board on one SoC family shares, for every layer: kernel and U-Boot patches and
config fragments, the hardware layer's files, and the scripts' SoC settings. A board names its
SoC in `board.sh` (`BOARD_SOC`), and [`config.sh`](../config.sh) reads that SoC's `soc.sh`, so
the scripts see `SOC_*` names instead of literal values. The levels (generic, SoC, board) are in
[docs/architecture.md](../docs/architecture.md#generic-soc-board).

| SoC | Boards |
|---|---|
| [Rockchip RK3568](rk3568/README.md) | [Anbernic RG DS+](../boards/rg-ds-plus/README.md) |

## What a SoC provides

| Path | What | Used by |
|---|---|---|
| `soc.sh` | The SoC as the scripts see it: the kernel platform (`SOC_KERNEL_PLATFORM`), the device tree directory, TF-A from rkbin (`SOC_RKBIN_BL31`) | Every host script, through `config.sh` |
| `README.md` | The SoC: what nibOS adds to mainline, things worth knowing | People |
| `kernel/patches/` | Kernel patches the SoC needs (`git format-patch` output) | [`kernel/build.sh`](../kernel/build.sh), after the generic patches and before the board's |
| `kernel/<soc>.config` | The SoC's kernel config fragment: its blocks built in, other SoCs' drivers off | `kernel/build.sh`, before the board's fragment |
| `uboot/patches/` | U-Boot patches for the SoC | [`uboot/build.sh`](../uboot/build.sh) |
| `hw/files/` | The hardware layer's SoC files: `usr/lib/nibos/soc.conf`, how it sleeps, the drivers kept out of start-up | [`hw/`](../hw/README.md) (the `nibos-hw` package) |
