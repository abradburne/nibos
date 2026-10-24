# Benchmarks

Every measurement in nibOS's docs can be taken again with a script here or next to the code it
measures. They all read [`config.sh`](../config.sh) through `tools/lib.sh`: the
device (`DEVICE`), the SSH key, the build machine and the board. The results, with the date,
the hardware and the build each was taken on, are in [results/](results/README.md).

**Before running anything on the device**, check that nothing is in use on it: the scripts
restart the device, so check first that no game is running (`pgrep` for the emulator) and
start DSlot again after a run that stopped it.

## Boot

| Script | Measures | Needs |
|---|---|---|
| [`boot.sh [host] [runs]`](boot.sh) | Restarts the device and reports the kernel's start, kernel → init, userspace, getty, the shell on tty1, SSH reachable and fully booted, in seconds from reset | SSH; mainline U-Boot for "from reset" (`STOCK=1` for the stock bootloader: from TF-A) |
| [`boot-ui.sh [host] [runs]`](boot-ui.sh) | The same clocks with DSlot as the boot UI: the splash, the skeleton, DSlot's unit, SDL video, windows, games listed, the shelf on both screens | `nib boot-ui on` |
| [`network.sh MODE [runs]`](network.sh) | Boot time in each network mode (`boot`, `deferred`, `demand`), with a recorder on the device that writes its figures before the network comes up | SSH once booted; leaves the mode as it found it |
| [`../uboot/measure.sh`](../uboot/measure.sh) | The bootloader's own stages (DDR init, SPL, SD init, loading and unpacking the kernel) from its bootstage records | A `MEASURE=1` U-Boot installed |

How the clocks work (the arch counter, U-Boot's bootstage records in reserved RAM, why not
journald's stamps): [docs/boot-time.md](../docs/boot-time.md#measuring).

## Suspend and Wi-Fi

How to test deep sleep and Wi-Fi's return (an RTC-woken suspend from a `systemd-run` unit,
`pm_test`, a cold Wi-Fi chip without rebooting) and the results:
[docs/engineering/suspend-and-wifi.md](../docs/engineering/suspend-and-wifi.md#how-to-test-suspend).

## Cards and device tools

| Tool | What |
|---|---|
| [`../tools/card-image.sh`](../tools/card-image.sh) | Saves a whole card to one compressed image, or writes one back (Mac): before anything risky |
| [`../tools/device/`](../tools/device/) | `counter` (the arch counter's time, so the kernel's start from reset), `memdump` and `memwrite` (physical memory through `/dev/mem`: the bootstage records, register diffs), built static for the device on first use; `vblank` (the panels' real refresh rate) |
