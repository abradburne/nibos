# The battery and charging

How the RG DS+ charges on nibOS, what the stock firmware does, and battery protect: nibOS's
switch that stops charging at about 80% so a device left on its charger ages slower.

> Don't hold a charge on the RK817 by turning charging off (`CHRG_EN`): that disconnects the
> battery, and unplugging then powers the device off ("Charging off cuts the battery" below).
> Battery protect lowers the charge voltage instead.

## The hardware

- **The cell:** 4247 mAh, charged to 4.35 V (the stock firmware's `rk817,battery`
  `design_capacity` and its `simple-battery` node; the cell's own rating isn't printed
  on the device or in its firmware).
- **The charger and gauge:** the RK817 PMIC charges the cell from USB-C and counts the charge
  in and out of it (a coulomb counter through a 10 mΩ sense resistor). The stock firmware also
  has a **CW2015** gauge (I²C 0x62), which estimates the charge from the voltage alone; nibOS
  leaves it off (the device tree says why) and uses the RK817's count.
- **No battery temperature.** Neither the stock device tree nor ours has a thermistor for the
  battery, the BSP charger driver reads none (its NTC thresholds are defined and unused), and
  the CW2015 has none. The only temperatures are the SoC's (`cpu-thermal`, `gpu-thermal`).
- **5 V only.** The USB-C port has no Power Delivery controller in either device tree, so the
  device draws at most what a charger offers at 5 V (3 A from a Type-C charger that
  advertises it). The kernel can't tell what the charger offers; `usb_type` always says DCP.
- **The charge LED** (amber) follows `rk817-charger-online`: on while the cord is plugged in,
  charging or not (`70-nibos-board.rules`).

## The RK817's two limits

Charging is held by whichever is lower:

| Limit | Register | Stock firmware | nibOS | Steps |
|---|---|---|---|---|
| USB input current (shared with running the device) | `CHRG_IN` (0xE5) `USB_ILIM_SEL` | 1500 mA (`max_input_current`) | 1500 mA (mainline's fixed value) | 80, 450, 850, 1500, 1750, 2000, 2500, 3000 mA (BSP) |
| Charge current into the cell | `CHRG_OUT` (0xE4) `CHRG_CUR_SEL` | 2000 mA (`max_chrg_current`) | 2000 mA (the device tree's ceiling; was 10 A, clamped to 3.5 A) | 0.5, 1, 1.5, 2, 2.5, 2.75, 3, 3.5 A |
| Charge voltage | `CHRG_OUT` `CHRG_VOL_SEL` | 4.4 V (`max_chrg_voltage`, above the cell's 4.35 V) | 4.35 V | 4.1 to 4.45 V in 50 mV |
| Charging on or off | `CHRG_OUT` bit 7 (`CHRG_EN` in the BSP) | on | on, always: off also disconnects the battery | |

At 5 V × 1.5 A, ~7.5 W comes in; what the device uses while on comes off the top, so the
2 A charge limit is rarely reached. Mainline's `RK817_CHRG_EN` (0xE6 bit 6) is the BSP's
`USB_SYS_EN`, the USB input path: cleared, the device runs from the battery (measured: −631 mA,
USB idle at 5.08 V); set again, the charger sees its input come back and starts a charge.

## Charging speed

A community write-up on charging the RG DS+ faster (r/ANBERNIC, "Notes on fast charging on the
RG DS Plus", on patched stock firmware; third-party, not measured here) reports:

| Limits (input / charge) | Gauge shows 100% | Charging ends | The author's verdict |
|---|---|---|---|
| 1500 / 2000 mA (stock) | ~3 h | ~5 h | safe |
| 3000 / 2000 mA ("Play & Charge", probably GammaOS's) | ~1 h 25 min | ~3 h 30 min | borderline for the PMIC and the battery |
| 3000 / 2500 mA | ~1 h | ~3 h | not recommended: the PMIC may burn out |

At stock limits, after an hour on the main menu, open on a desk at 23 °C, it measured the back
cover's hotspot at 45 °C, the battery at 38 °C and the PMIC's hotspot at 72 °C; Li-ion cells
should charge at 45 °C or below. And the stock "100%" is the CW2015's voltage guess: it lit
"full" while the RK817's count said ~70% and charging went on.

With no battery temperature to guard a faster charge by, nibOS keeps the stock limits. The
faster, cooler way to charge is with the device asleep (lid closed): nothing else draws from
the input, so the battery gets about 1.6 times the current (measured below).

## Battery protect

`nib battery protect on` (or `nibkit::battery::set_protect`, DSlot's System page): the charge
voltage drops from 4.35 V to 4.1 V (`NIBOS_BATTERY_PROTECT_UV` in the board's `board.conf`), the
lowest the RK817 has. The cell charges to about 80-85% (4.1 V is 82% on the OCV table, which is
the RG DS's until measured) and the charger stops, as it does at full; it starts again on its
own once the cell falls below its recharge threshold. The battery stays connected throughout,
so unplugging is as safe as at a full charge (tested below). The status reads "Not charging"
while stopped on the charger; the amber LED stays on while plugged in. Kept across restarts
(`/var/lib/nibos/battery/protect`, saved before it is applied).

In the kernel (the rk817 patch, [patches.md](patches.md)) it is the battery's writable
`constant_charge_voltage` (4.1 V up to `constant_charge_voltage_max`, the device tree's
4.35 V), set again on every plug and unplug. A charge that finishes below the full voltage
isn't reported as full and doesn't recalibrate the coulomb count to 100%.

It is a voltage, not a percentage: a laptop-style "stop at exactly 80%" would need charging
turned off, which on the RK817 disconnects the battery.

## Charging off cuts the battery

An earlier battery protect (kernel #41, withdrawn in #42) held the charge with
`charge_control_end_threshold`, turning `CHRG_EN` off at 80% and back on at 75% (with a pulse
of the USB input path, which the charger needed to restart). With protect on and the battery at
90% (held: `CHRG_OUT` 0x52), unplugging the charger while the device was on powered it off at
once (the journal ends mid-line, no shutdown); it started again only after a 10 s hold of
power. While held the battery read ~3 mA, which looks like "running from USB, battery resting"
but is the battery **disconnected from the system** by `CHRG_EN` off: unplugged, nothing
supplies the device. The BSP never turns `CHRG_EN` off. Battery protect by the charge voltage
(kernel #43 on) keeps the battery connected.

So every charging change needs the unplug tests below, awake and asleep, in each state
(charging, held, full).

## Measurements

On the RG DS+ with nibOS (kernel with the rk817 patch), 2026-10-19, one USB-C charger and
cable, at 64-73% charge, room temperature, the device open on a desk. Battery current from the
RK817 (`current_avg`; positive into the battery), charge from its coulomb counter
(`charge_now`), temperatures from the SoC's zones.

| Situation | Into the battery | USB voltage | SoC |
|---|---|---|---|
| Awake on DSlot's shelf, stock limits (1.5 A in, 2 A charge) | 811 mA (5 min, coulomb counter; `current_avg` 800-840 mA) | 4.48-4.50 V | 48-50 °C |
| Asleep (deep, `rtcwake` 300 s), stock limits | ~1.3 A (1264 mA over 331 s, ~300 s of it asleep) | | 43 °C after waking |
| Battery protect on (4.1 V), at 93%, plugged in: stopped as at full (`CHRG_STS` "finished") | 6 mA | 4.87 V | |

- **The input is the limit.** Awake, the USB voltage sits at the RK817's 4.5 V input floor
  (`USB_VLIM`): the charger and cable sag before 1.5 A, so the device and the battery share
  what comes in; held (nothing charging), the voltage rises to ~4.9 V. Asleep, the device
  draws almost nothing and the battery gets ~1.6 times as much.
- **The withdrawn `CHRG_EN` battery protect** stopped charging at 80% by the count and
  restarted it at the start threshold, the battery reading 3-5 mA while held: the battery cut
  off, as the unplug showed.
- **Time to full** (`nibkit::battery`, at the current then): ~21 min to protect's 80% from 73%,
  ~101 min to 100%; an underestimate, as the current falls in the last part.

## The unplug tests (kernel #43, battery protect by voltage)

2026-10-19, unplugging and plugging the charger by hand at each step, with a 1 s log of the
charger's state and registers on the device (synced to disk each line):

| Step | State when unplugged | Result |
|---|---|---|
| 2 | Awake, charging (~820 mA), protect off | Stayed on; battery took over; charging again on plugging in |
| 3 | Awake, protect on at 93%: stopped (`CHRG_OUT` 0x82, `CHRG_STS` "finished", 6 mA) | Stayed on; battery took over at once (−631 mA); stopped again on plugging in |
| 4 | Asleep, protect on, stopped | Woke at the unplug (the PMIC's plug interrupt wakes the device, as before); no power loss, on battery, 4.1 V kept |
| 5 | Asleep, charging, protect off | Woke at the unplug; no power loss, on battery |
| 6 | Plugged in again, protect off | Charging at 868 mA |

The device stayed up throughout (no new boot, the journal unbroken), so each unplug kept it
powered. Unplugging (and plugging in) wakes a sleeping device: the RK817's plug interrupt is a
wake-up source.

## Testing a charging change

Before a change to charging ships, on the device, with the charger plugged in:

1. Charging, awake: unplug; the device stays on (the battery takes over: `current_avg`
   negative within seconds). Plug in again: charging resumes.
2. The same asleep (lid closed): unplug, wait, wake: it wakes, and the journal has no gap.
3. In every other state the change adds (held, finished): unplug awake, then asleep, as above.
4. After each: `CHRG_OUT` (0xE4), `CHRG_IN` (0xE5), `CHRG_TERM` (0xE6) and `CHRG_STS` (0xEB)
   (`i2cget -f -y 0 0x20 <reg>`), the status, and that charging restarts on plugging in.

An unplug that powers the device off needs a 10 s hold of power to start it again; know that
before the test.

## Charging while off

Plugging a charger into a switched-off RG DS+ switches it on (the RK817 does that by itself);
nibOS then charges asleep, dark, with the amber LED lit, rather than booting to the shelf
(`nibos-charge-boot`, [hw/README.md](../../hw/README.md)). Switching it off while plugged in
leaves it off: the PMIC switches on only when the charger goes in, and then nothing is lit.

## Dead ends: don't repeat

Tried and measured, or found not to work; the source says more. Add to it when something doesn't pay.

- Battery protect by turning charging off (CHRG_EN): disconnects the battery; unplugging then powers off. By the charge voltage instead.
- Restarting a held charger by toggling the input limits or the charge voltage: only pulsing USB_SYS_EN worked.
- Stopping at exactly 80%: impossible without disconnecting the battery; protect is 4.1 V (~80-85%).
- Faster charging (3 A in, 2-2.5 A charge): borderline for the PMIC and the cell, no thermistor; charge asleep instead.
- The CW2015 gauge: stock's "100%" at ~70% by the RK817's count; off.
- The `rk817-battery-charging` LED trigger: dark at full; charger-online.
- Switching off while plugged in doesn't come back up: the PMIC switches on only when the charger goes in.
