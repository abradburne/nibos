# The kernel and U-Boot patches

nibOS runs mainline Linux and mainline U-Boot with 21 patches, in three sets each: generic
(any board), SoC (the RK3568) and board (the RG DS+). The build applies them in that order.
Each patch is `git format-patch` output whose message says why it exists, what it was measured
to do and whether it could go upstream; this page puts those reasons side by side.

A patch is named by its tree, set and number: kernel `rk3568/0004` is
[`socs/rk3568/kernel/patches/0004-…`](../../socs/rk3568/kernel/patches/).

"Board-specific hack" means a change that buys boot time on this hardware in a way the upstream
project wouldn't take as it is; each such message says what a general fix would look like.

## Linux (at `KERNEL_COMMIT`, v7.3-rc7)

| Patch | What | Why, as measured | Upstream |
|---|---|---|---|
| [generic/0001](../../kernel/patches/0001-init-open-the-console-after-mounting-root.patch) | `init/main.c`: open the console after mounting root | fbcon holds the console lock while its modeset powers the panels on (~0.33 s), and the root mount waited for it; now the mount (~85 ms) overlaps the panels and init starts 0.66–0.72 s into the kernel | Board-specific hack |
| [generic/0002](../../kernel/patches/0002-wifi-rtw88-sdio-resume-the-host-interface-before-the.patch) | rtw88 SDIO: resume the host interface before the first MAC access | A cold RTL8821CS has its SDIO host interface suspended, so the chip version read as 0 (set up as the wrong cut), every boot logged `-110` errors, and any later suspend never woke | Candidate |
| [generic/0003](../../kernel/patches/0003-wifi-rtw88-sdio-power-the-card-off-in-suspend-withou.patch) | rtw88 SDIO: power the card off in suspend without WoWLAN | Kept powered through deep sleep, the chip stopped answering; powered off and set up again, Wi-Fi is back ~1.9 s after waking (4/4 cycles) | Candidate |
| [generic/0004](../../kernel/patches/0004-power-supply-rk817-let-userspace-lower-the-charge-vo.patch) | rk817 charger: writable `constant_charge_voltage` (4.1 V up) | Battery protect: charged to 4.1 V the cell stops at about 80-85% with the battery connected; turning charging off instead disconnected it and unplugging cut the power ([battery.md](battery.md)) | Candidate |
| [generic/0005](../../kernel/patches/0005-ASoC-codecs-aw88166-start-the-amp-once-the-stream-ru.patch) | aw88166: start the amp from the DAI trigger too | The DAPM start runs before the I2S clock; at resume every try failed and the speakers stayed silent until the stream was closed. 10/10 suspend cycles with each tuning ([audio.md](audio.md#the-amps-after-suspend)) | Candidate |
| [rk3568/0001](../../socs/rk3568/kernel/patches/0001-i2c-mfd-regulator-rk3x-rk8xx-register-at-subsys_init.patch) | rk3x I2C, rk8xx and its regulators at `subsys_initcall` | The SD controllers need the RK817 for their 1.8 V rail and deferred to late initcalls: the root card started at 1.29 s into the kernel, now at 0.26 s | Board-specific hack |
| [rk3568/0002](../../socs/rk3568/kernel/patches/0002-mmc-dw_mmc-rockchip-use-a-short-data-timeout-while-t.patch) | dw_mmc-rockchip: a 10 ms data timeout while tuning | A tuning phase whose block never arrives waited 112 ms; SDR104 tuning took 0.03–0.51 s, now 24–100 ms | Candidate |
| [rk3568/0003](../../socs/rk3568/kernel/patches/0003-drm-rockchip-set-up-the-fbdev-client-from-a-work-ite.patch) | rockchip-drm: set up the fbdev client from a work item | The first modeset (the better part of a second of panel sleeps) ran inside a component bind, holding every probe the root mount waits for | Candidate (RFC) |
| [rk3568/0004](../../socs/rk3568/kernel/patches/0004-clk-rockchip-rk3568-keep-clk_rtc32k_frac-running.patch) | RK3568: keep `clk_rtc32k_frac` running | With the PMU's 32 kHz clock gated, TF-A's power-down never completes and every suspend hangs ([suspend-and-wifi.md](suspend-and-wifi.md)) | Candidate, for discussion |
| [rg-ds-plus/0001](../../boards/rg-ds-plus/kernel/patches/0001-drm-panel-jadard-jd9365da-h3-add-Anbernic-RG-DS-pane.patch) | Jadard driver: the RG DS+'s JD9165 panels | The panels, from the stock device tree's init sequences; the top one flips its scan with MADCTL | Candidate (with a DT binding) |
| [rg-ds-plus/0002](../../boards/rg-ds-plus/kernel/patches/0002-drm-panel-jadard-jd9365da-h3-probe-asynchronously.patch) | Jadard driver: probe asynchronously | Both panels' power-on ran inside the driver's initcall and blocked every initcall after it: 0.81 s of a 2.03 s kernel start | Candidate |
| [rg-ds-plus/0003](../../boards/rg-ds-plus/kernel/patches/0003-drm-panel-jadard-jd9365da-h3-RG-DS-reset-in-probe-sl.patch) | RG DS+ panels: reset in probe, the sleep-out wait in enable | The panels powered on one after the other; now the second initialises while the first waits: on at 0.55 and 0.61 s instead of 0.70 and 0.84 s | Board-specific as written |

## U-Boot (at `UBOOT_TAG`, v2026.10)

| Patch | What | Why, as measured | Upstream |
|---|---|---|---|
| [generic/0001](../../uboot/patches/0001-cmd-bootstage-add-bootstage-mark-name.patch) | `bootstage mark NAME` | Lets the measuring build time each step of its boot command | Candidate |
| [generic/0002](../../uboot/patches/0002-spl-atf-boot-Linux-through-TF-A-Falcon-mode.patch) | SPL boots Linux through TF-A (Falcon mode) | No U-Boot proper: the kernel starts 0.65 s after reset instead of 0.89 s | Candidate |
| [generic/0003](../../uboot/patches/0003-blk-stop-the-partition-scan-at-the-first-unused-entr.patch) | The partition scan stops at the first unused GPT entry | 116 empty entries, each a re-read of the GPT: 0.15 s of every boot | Board-specific hack |
| [rk3568/0001](../../socs/rk3568/uboot/patches/0001-board_f-turn-the-data-cache-on-before-relocation-on-.patch) | The data cache on before relocation | With it off, U-Boot spent 0.92 s before relocation; afterwards 0.14 s to its boot command | Board-specific hack |
| [rk3568/0002](../../socs/rk3568/uboot/patches/0002-clk-rk3568-double-the-SD-card-clock-the-controller-d.patch) | The SD clock: ask the CRU for double (the controller halves it) | "50 MHz" was 25 MHz: the 11.75 MB kernel read in 0.98 s instead of 0.50 s | Candidate |
| [rk3568/0003](../../socs/rk3568/uboot/patches/0003-mmc-rockchip_dw_mmc-UHS-SDR104-for-the-RK3568-SD-car.patch) | dw_mmc: UHS SDR104 for the RK3568's SD card | 1.8 V signalling, IO-domain order, CRU phases and tuning: the kernel loads in 107 ms instead of 0.33 s, and Falcon mode pays | Candidate (as a series to split) |
| [rk3568/0004](../../socs/rk3568/uboot/patches/0004-rockchip-rk3568-ungate-clk_rtc32k_frac-in-SPL.patch) | RK3568: ungate `clk_rtc32k_frac` in SPL | Only U-Boot proper opened the gate; under Falcon every suspend hung. 10/10 suspend cycles with it | Candidate |
| [rg-ds-plus/0001](../../boards/rg-ds-plus/uboot/patches/0001-rockchip-rk3568-RG-DS-hold-A-at-power-on-to-boot-the.patch) | A held at power-on boots the fallback U-Boot | A kernel or bootloader under test that hangs costs a restart, not a card reader | Board-specific (nibOS's recovery) |
| [rg-ds-plus/0002](../../boards/rg-ds-plus/uboot/patches/0002-rockchip-rk3568-RG-DS-run-the-CPU-at-1608-MHz-to-unp.patch) | The CPU at 1608 MHz to unpack the kernel | gunzip of the kernel took 0.54 s at 816 MHz, 0.28 s at 1608 | Board-specific |
| [rg-ds-plus/0003](../../boards/rg-ds-plus/uboot/patches/0003-rockchip-rk3568-RG-DS-UHS-in-SPL-but-not-while-A-is-.patch) | UHS in SPL, but not while A is held | Falcon's 17.5 MB read: 0.73 s at High Speed, 0.25 s at SDR104; a hanging UHS init can't take the escape hatch with it | Board-specific |

Two more sets are for investigation builds only and never installed for normal use:
`uboot/patches/measure/` (`MEASURE=1`: boot-stage marks for initcalls, probes, SD init and
SPL loading, and commands to stash the console log and time decompression, read back by
[`uboot/measure.sh`](../../uboot/measure.sh)) and `uboot/patches/falcon-debug/`
(`FALCON=1 FALCON_DEBUG=1`: SPL leaves breadcrumbs on the SD card at sector 24000, the only
way to see into a Falcon boot without a serial console).

## Changing a patch

Apply the sets in order to a tree at the pinned version (`KERNEL_COMMIT` or `UBOOT_TAG` in
`config.sh`), change or add a commit, and regenerate each set from its range:

```bash
# in a kernel tree: generic, then rk3568, then rg-ds-plus, as one series of commits
git am kernel/patches/*.patch socs/rk3568/kernel/patches/*.patch boards/rg-ds-plus/kernel/patches/*.patch
# ... change ...
git format-patch --zero-commit --no-signature --base=$KERNEL_COMMIT -o kernel/patches $KERNEL_COMMIT..HEAD~7
git format-patch --zero-commit --no-signature --base=HEAD~7 -o socs/rk3568/kernel/patches HEAD~7..HEAD~3
git format-patch --zero-commit --no-signature --base=HEAD~3 -o boards/rg-ds-plus/kernel/patches HEAD~3..HEAD
```

(with the counts adjusted to the sets' sizes; the same for U-Boot with `UBOOT_TAG`). The
patches carry no `Signed-off-by:`: that line is the author's own certification under the
[Developer's Certificate of Origin](https://docs.kernel.org/process/submitting-patches.html#sign-your-work-the-developer-s-certificate-of-origin),
added by the author when a patch is sent upstream.
