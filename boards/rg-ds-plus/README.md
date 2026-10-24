# Anbernic RG DS+

**Status: works.** Everything on the board works under nibOS, including deep sleep with Wi-Fi back
on wake; the known issues are listed at the end.

A clamshell handheld with two screens: Rockchip RK3568 (4× Cortex-A55 at up to 1.99 GHz,
Mali-G52; [socs/rk3568/](../../socs/rk3568/README.md)), 1 GB LPDDR3, two 1024×768 MIPI-DSI
panels (the bottom one with touch), two microSD slots and no eMMC, the RK817 PMIC, two AW88166
speaker amps, an RTL8821CS for Wi-Fi and Bluetooth, buttons, a D-pad, one analog stick, a lid
sensor, three LEDs, a USB-C port for charging and OTG and a second, host-only USB-C port.

| Path | What |
|---|---|
| [`board.sh`](board.sh) | The board as the scripts see it: device tree, U-Boot defconfig, the card's layout, console size, RAM |
| [`dts/rk3568-anbernic-rg-ds-plus.dts`](dts/rk3568-anbernic-rg-ds-plus.dts) | The device tree: mainline's RG DS tree plus what the DS+'s stock tree says is different |
| [`kernel/`](kernel/) | Three panel patches ([why each](../../docs/engineering/patches.md)), the kernel config fragment, the smoke test's drivers and firmware |
| [`uboot/`](uboot/) | Four U-Boot patches (the A-held fallback, the CPU clock while unpacking, UHS in SPL, the A/B slot choice) and the config fragments |
| [`hw/files/`](hw/files/) | The hardware layer's board files: the daemons' device names (`usr/lib/nibos/board.conf`), udev, logind, WirePlumber, thermal limits, iwd, modules |
| [`rootfs/files/`](rootfs/files/) | The root image's board file: the stock partitions kept out of systemd |
| [`hw/aw88166-tuning.py`](hw/aw88166-tuning.py) | nibOS's own tuning file for the speaker amps (the DSP bypassed, conservative gain), built into `nibos-hw` |
| [`extract-stock.sh`](extract-stock.sh) | Takes the stock boot partition, the amps' official tuning file and the stock device tree from Anbernic's image, for a development card |

## The files that aren't in git

A development card uses three things from Anbernic's firmware image (V1.0, 2026-09-15,
`RG-DS-PLUS-EN16GB-V1.0-20260915.IMG`, 15.5 GB) that are not ours to redistribute; a
downloadable image uses none of them. `extract-stock.sh` takes them into `stock/` (git
ignores it):

| File | From | Used for |
|---|---|---|
| `boot.img` | Partition 3 of the image | The template `kernel/mkboot.py` builds boot images for the stock U-Boot from (its resource block) |
| `aw88166_acf.bin` | `/lib/firmware/aw883xx_acf.bin` in the stock root (partition 8) | The speaker amps' official tuning file (Awinic's), put in a development card's root by `rootfs/make-arch.sh` (or `hw/install-firmware.sh`); on a device, `nib audio import-tuning` takes the same file from a stock card or image |
| `stock.dtb`, `stock.dts` | The FIT in partition 3, and `dtc -s` of it | The reference the device tree here was written from |

## How the DS+ differs from the RG DS

The device tree starts from mainline's `rk3568-anbernic-rg-ds.dts`. These are the differences,
read from the DS+ stock device tree (`stock/stock.dts`). Everything else (buttons, stick, lid
sensor, panel power and reset, Wi-Fi/BT, USB, vibrator) is on the same pins.

| | RG DS (mainline) | RG DS+ |
|---|---|---|
| Panels | JD9365, 640×480, 4-lane | JD9165, 1024×768, 2-lane, 62.8 MHz; top panel flipped with MADCTL 0x03 |
| Backlight PWM | pwm12 / pwm13 | pwm1 (GPIO0_B5) / pwm2 (GPIO0_B6) |
| Touch | two GT911 (i2c5, i2c3) | one GT9xx on i2c5, reset GPIO0_A6, irq GPIO0_C6, 1024×768 |
| Speakers | AW87391 analog amps on the rk817 output | two AW88166 smart amps on i2c2 0x34/0x35, I2S3 m1; reset GPIO4_A7 / GPIO4_B2 |
| Headphone detect | GPIO4_C2 | GPIO3_B5 |
| LEDs | pwm5/6/7 | GPIO0_C4 (power), GPIO0_C5 (charging), GPIO3_B6 (low battery) |
| Storage | eMMC + one microSD | no eMMC; two microSD: TF1 sdmmc0, TF2 sdmmc1 (power GPIO0_B7, detect GPIO0_C0) |
| Battery | 4000 mAh | 4247 mAh; cw2015 profile from the DS+ stock tree |

The tree also makes three boot-time changes: the SD card's 3.3 V switches lose the RG DS
tree's `vin-supply` (it is the 1.8 V signalling rail, and waiting for it deferred the SD
controllers to late initcalls), SDR104 tuning tries 90 phases instead of 360, and
`0x0f000000`–`0x0f01ffff` is reserved for U-Boot's bootstage records, which the benchmarks
read. Under mainline the card in TF1 is `mmcblk0` and TF2 `mmcblk1`. Each panel has its own
CRTC: `DSI-1` is the bottom (touch) panel, `DSI-2` the top.

### Panel refresh rate: VPLL at 1188 MHz

The panels' pixel clocks (`dclk_vop0/1`) come from VPLL and can't change its rate
(`CLK_SET_RATE_NO_REPARENT`, no `SET_RATE_PARENT`), so the refresh rate is whatever VPLL divides
to. The RG DS tree's VPLL of 292.5 MHz gave 292.5/5 = 58.5 MHz instead of the mode's 62.8 MHz
(stock timings: 1304×800 total, 60.2 Hz): **56.08 Hz**, so everything synced to the display ran
at 94% speed. The tree sets VPLL to **1188 MHz**, a rate in the CRU's PLL table, and the divider
rounds down to 1188/19 = 62.526 MHz: **59.937 Hz** on both CRTCs (measured with
[`tools/device/vblank.c`](../../tools/device/vblank.c)), next to the DS's 59.83 Hz. Nothing else hangs off VPLL,
and the DSI lane rate comes from the mode, not the pixel clock, so it is unchanged. Other
table rates near 60 Hz: 1440/23 (60.02 Hz), 1000/16 (59.91 Hz). The two panels' vblanks are
8.2 ms apart. More in [docs/engineering/display.md](../../docs/engineering/display.md).

### Wi-Fi and Bluetooth: RTL8821CS

The chip sits on sdmmc2 in SDIO 3.0 mode (UHS SDR50). `vcc_wifi` (GPIO0_A0) is always on; the
SDIO power sequence drives only WL_REG_ON (GPIO4_A2, active-low reset) and the rk817's 32 kHz
clkout2. BT_REG_ON is GPIO0_D5 (hci_uart's `enable`). Two generic rtw88 patches make it
reliable at boot and across deep sleep; iwd reconnects about 1.9 s after waking. The story of
finding them: [docs/engineering/suspend-and-wifi.md](../../docs/engineering/suspend-and-wifi.md).

## Measured on the stock OS

Before any of nibOS was written, a probe app on the stock firmware
([tools/stock-probe/](../../tools/stock-probe/README.md), 2026-10-15) answered what the stock
device tree couldn't:

| Question | Answer |
|---|---|
| RAM | 1 GB LPDDR3 (953 MB usable, 48 MB CMA). CPU max 1.99 GHz |
| Speaker amps | Two AW88166 (ID register 0x00 = 0x2066) at 0x34/0x35 on i2c-2: mainline's `snd-soc-aw88166` |
| Touch | One GT9xx at i2c-5 0x14 |
| Which screen is which | `backlight1` and `dsi@fe070000` are the top screen |
| Lid sensor | A hall switch on GPIO0_C3 (the stock OS reports it as keys on the PMIC's power key device): a `gpio-keys` `SW_LID` in mainline |
| USB-C host | The second port is host-only (EHCI/OHCI at `fd840000`); a USB keyboard enumerated |
| Suspend | Deep suspend works on the stock boot chain |
| Awake power | ~0.58 A (rk817) / ~0.89 A (cw2015) at ~3.8 V, both screens at backlight 30/255: 2.2–3.4 W |

The stock system: Buildroot 2024.02 on Rockchip's 6.1 BSP kernel; boot chain DDR init v1.25,
SPL v1.14, BL31 v1.46, OP-TEE v2.16, U-Boot `evb_rk3568`.

## Status

Works: both panels, touch, buttons, stick, lid, volume, battery and charger, LEDs, both SD
slots, Wi-Fi, Bluetooth, speakers and headphones, Panfrost (GLES 3.1), cpufreq and thermal
limits, USB host, and deep suspend with Wi-Fi back on wake. Boot: the kernel starts 0.65 s
after reset, a picture is on both screens at 1.5 s, DSlot's shelf at about 5.2 s
([bench/results/](../../bench/results/README.md)).

Known issues:

- **Warm restarts can leave the top panel striped** (alternate lines, even on the text
  console); a cold power-on is clean. The JD9165 init on a panel that kept its power.
- **The USB host controller hangs suspend** (OHCI `fd840000.usb`), even with nothing plugged
  in. The hardware layer unbinds it around sleep (`NIBOS_SLEEP_UNBIND`); the cause is not
  known.
- The ICM42607 IMU driver probes but finds nothing; the part may not be fitted on the DS+.
- audio-graph-card2 logs a bad `of_node_put` warning at boot (an upstream bug, harmless so far).
- Not tested yet: headphone plug and unplug switching (needs headphones), rumble through the
  merged gamepad, battery-low shutdown, sleep drain over hours.
