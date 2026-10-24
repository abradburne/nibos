# Kernel

Mainline Linux (7.3-rc7, `KERNEL_COMMIT` in `config.sh`) for the board in `NIBOS_BOARD`, with
three sets of patches (generic, the SoC's, the board's), the board's device tree, and a
configuration made from arm64's defconfig and three fragments. It is built natively in the build
machine (an OrbStack Arch Linux ARM VM), not as a distribution package.

| Path | What |
|---|---|
| [`build.sh`](build.sh) | Patches the tree, configures and builds `Image.gz`, the DTB and modules into `out/` |
| [`deploy.sh`](deploy.sh) | Puts `out/` on a running device over SSH, in every place a bootloader reads it, and restarts it |
| [`patches/`](patches/README.md) | The generic patches: the console after the root mount, two rtw88 fixes |
| [`config/nibos.config`](config/nibos.config), [`config/platform.config`](config/platform.config) | The generic configuration: what every nibOS kernel has, and what goes with the platform trim |
| [`mkboot.py`](mkboot.py), [`test_mkboot.py`](test_mkboot.py) | Wraps the kernel in the stock boot partition's FIT format (for the stock U-Boot), and its tests |
| [`make-dev-card.sh`](make-dev-card.sh), [`flash-boot.sh`](flash-boot.sh) | Make a development card from the stock image; write or restore its boot partition (Mac) |
| [`smoke/`](smoke/init) | A built-in initramfs that tests the hardware without a root file system (`SMOKE=1`) |

What the SoC and the board add: [`socs/rk3568/kernel/`](../socs/rk3568/kernel/) and
[`boards/rg-ds-plus/`](../boards/rg-ds-plus/README.md) (`dts/`, `kernel/`). Why each patch
exists: [docs/engineering/patches.md](../docs/engineering/patches.md).

## Build and deploy

```bash
orb -m rgbuild bash kernel/build.sh     # out/Image.gz, out/<board dtb>, out/modules.tar.zst, out/config
kernel/deploy.sh                        # to $DEVICE (config.sh); restarts it
```

`build.sh` resets the kernel tree (`KSRC`, `~/linux` in the build machine) and refuses to build
unless it is at `KERNEL_COMMIT`: keep no work there. **Deploy only with `deploy.sh`.** The
device's SPL boots whatever FIT is on the Falcon partition, so a kernel copied anywhere else is
not the one that boots. `deploy.sh` writes the kernel to:

| Where | Read by |
|---|---|
| The Falcon partition (the RG DS+'s partition 4, "recovery"): `falcon.itb`, the uncompressed kernel + DTB + TF-A, made by [`../uboot/mkfalcon.py`](../uboot/mkfalcon.py) | SPL in Falcon mode: every normal power-on |
| `/boot/Image.gz`, the DTB and `/boot/extlinux/extlinux.conf` on the root partition | The fallback U-Boot, when A is held at power-on |
| The boot partition (partition 3): a FIT in the stock layout (`mkboot.py`, from the board's extracted `stock/boot.img`) | The stock U-Boot, if the stock bootloader is put back |
| `/usr/lib/modules/` | The running system |

The kernel command line is in `deploy.sh`, with why each argument is there. The ones that are
not obvious are all boot-time fixes: the console's size for systemd (from `board.sh`; it
otherwise waits 333 ms for a reply the VT never sends), a higher mount rate limit for PID 1,
and `rootfstype=ext4`. With the splash on, the Falcon FIT's command line adds
`init=/usr/lib/nibos/splash`; the fallback's never does.

## How the kernel is configured

`build.sh` starts from `defconfig` and merges fragments with the kernel's own
`merge_config.sh`, in two rounds, each settled with `olddefconfig`:

1. **The platform.** Every `ARCH_*` platform but the SoC's (`SOC_KERNEL_PLATFORM`, generated
   from `Kconfig.platforms`) is turned off, with ACPI, KVM, Xen and Hyper-V
   ([`config/platform.config`](config/platform.config)). arm64's defconfig otherwise builds in
   drivers for every SoC family, which the bootloader then has to read and unpack at every
   start.
2. **The rest**, the later fragment winning:
   - [`config/nibos.config`](config/nibos.config): what every nibOS kernel has (the
     display and sound stacks, uinput, zram, Terminus 16x32 as the console font, landlock and
     the crypto iwd needs, the performance governor at boot) and what a handheld never uses
     (EFI, virtualization, NUMA, hibernation, kexec, debug info);
   - `socs/<soc>/kernel/<soc>.config`: the SoC's blocks built in, other SoCs' drivers off;
   - `boards/<board>/kernel/<board>.config`: the board's parts built in, what it doesn't have
     off.

Built in, because this kernel only runs on one board and has no initramfs, except the drivers
that load firmware (Wi-Fi, Bluetooth, the speaker amps, touch): built in, they would probe
before `/usr/lib/firmware` is mounted and find nothing. Their stacks (cfg80211, mac80211,
Bluetooth) are modules for the same reason. Every MB of `Image.gz` costs the bootloader
~0.07 s; the RG DS+'s is 7.57 MB (from 11.75 MB with defconfig).

The two rounds are how the configuration was made before it was split into fragments, with
`scripts/config` calls and `olddefconfig` between them, and they give the same `.config`
(checked on 2026-10-18, building both from the same tree). One difference was found and kept:
the old calls named two options in mixed case (`I2C_MUX_PCA954x`, `ARM_CCI5xx_PMU`), which
`scripts/config` upper-cases into names that don't exist, so they were never turned off; the
fragments turn them off as intended (the PCA954x I2C mux and the CCI-500 PMU are not on the
board).


### Measured choices

Each built with its own module version, booted from A/B slot B (so a kernel that didn't boot
would have fallen back to A by itself), and measured against kernel #51: three boots to the
shelf, idle current on the shelf under DSlot's screensaver (the RK817's coulomb counter, 2 x 90
s), and two DSperate scenes headless (`dsperate-headless`, DSperate's `bench/core/bench.sh`:
mkds, gsboot; medians in ms, frame and audio hashes).

| | Shelf (s) | Idle (mA) | mkds | gsboot | Kept |
|---|---|---|---|---|---|
| #51 (5-level page tables, 52-bit VA; no MGLRU; audit on) | 5.59-5.78 | 516-522 | 9.70 | 6.60 | baseline |
| `ARM64_VA_BITS_39` (3 levels) | 5.48-5.64 | 516 | 9.62-9.83 (3 runs) | 6.48-6.59 | yes: neutral, fewer page-table levels |
| `LRU_GEN` (MGLRU) | 5.64-5.77 | 460-529 | 9.73 | 6.57 | yes: neutral here; it pays under memory pressure, which these don't make |
| `AUDIT` off | 5.57-5.68 | 485-529 | 9.75 | 6.58 | yes: neutral, 133 KB smaller FIT |
| All three (#59, deployed) | 5.58-5.61 | 529-536 (51% battery) | 9.65 | 6.67 | yes |

Hashes were identical in every run (the emulation is exact either way). `NR_CPUS` was already
4, and NUMA, hibernation and virtualization already off. zram moved from zstd to lz4 with
`vm.page-cluster = 0` (nibos-hw): a 900 MB fill under memory pressure took 3.1-3.2 s against
4.2-6.3 s. The soft and hard lockup detectors panic ([rootfs/safe-mode](../rootfs/safe-mode/README.md)).

## The smoke test

`SMOKE=1 build.sh` builds the same kernel with [`smoke/init`](smoke/init), busybox and the
board's firmware (`boards/<board>/kernel/smoke-firmware`) as a built-in initramfs, and the
firmware-loading drivers built in ([`smoke/smoke.config`](smoke/smoke.config) and the board's
`smoke.config`). It needs no root file system: it shows what it finds on the screens, runs a
20 s input test, saves a report to `ROMS/nibos-boot/<n>/` and powers off. That was the first
thing this kernel ran on an RG DS+, and it would be the first step on a new board.

## The board's details

How the RG DS+ differs from the RG DS, its panels' refresh rate, its Wi-Fi chip and the known
issues are in [boards/rg-ds-plus/README.md](../boards/rg-ds-plus/README.md). How the kernel's
part of the boot was made fast is in [docs/boot-time.md](../docs/boot-time.md).
