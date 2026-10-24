# Rockchip RK3568

4× Cortex-A55, Mali-G52 (Bifrost, Panfrost in Mesa), VOP2 with two MIPI DSI outputs, three
DesignWare SD/SDIO controllers, usually paired with the RK817 PMIC. Mainline Linux and U-Boot
support it well; nibOS adds boot-time and suspend fixes and UHS for the SD card in U-Boot.

| Path | What |
|---|---|
| [`soc.sh`](soc.sh) | `ARCH_ROCKCHIP`, `rockchip/` device trees, TF-A (BL31) v1.46 from rkbin |
| [`kernel/patches/`](kernel/patches/) | Four: the PMIC and its I2C bus early, a short SD tuning timeout, the first modeset off the probe path, the PMU's 32 kHz clock kept running |
| [`kernel/rk3568.config`](kernel/rk3568.config) | VOP2, DSI, Panfrost, I2S, SAR ADC, thermal and USB built in; four cores; other SoCs' drivers off |
| [`uboot/patches/`](uboot/patches/) | Five: the data cache before relocation, the SD clock really at 50 MHz, UHS SDR104 for the SD card, the 32 kHz clock ungated in SPL, the watchdog and the thermal sensor triggering a first global reset (it doesn't bring the RG DS+ back; a panic through the PMIC does) |
| [`hw/files/`](hw/files/) | Deep sleep; the video decode and 2D blit drivers loaded after start-up |

Why each patch exists, what it bought and whether it could go upstream:
[docs/engineering/patches.md](../../docs/engineering/patches.md).

## Things worth knowing

- **Suspend needs the PMU's 32 kHz clock running.** `clk_rtc_32k` runs from
  `clk_rtc32k_frac`, which only U-Boot proper's clock driver ungated; under Falcon mode (no
  U-Boot proper) every suspend and s2idle hung in TF-A's power-down. SPL ungates it, and the
  kernel marks it critical. [docs/engineering/suspend-and-wifi.md](../../docs/engineering/suspend-and-wifi.md).
- **The SD controller halves the clock it is given**, so U-Boot asks the CRU for double:
  before the fix "50 MHz" was 25 MHz.
- **UHS SDR104 in U-Boot** needed 1.8 V signalling, the IO-domain switch in the safe order,
  the CRU's clock phases and tuning, none of which U-Boot had for this SoC. It is what makes
  Falcon mode pay: the uncompressed kernel reads at 69 MB/s.
- **Thermal trips**: mainline's `rk356x.dtsi` starts throttling at 70 °C; the boards set
  their own limits in their hardware layer files (the RG DS+ uses the stock firmware's 75 /
  85 °C).
- **Panel pixel clocks come from VPLL** and can't change its rate, so a board's refresh rate is
  whatever its VPLL divides to ([the RG DS+'s](../../boards/rg-ds-plus/README.md#panel-refresh-rate-vpll-at-1188-mhz)).
