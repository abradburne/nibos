# Sleep, and powering off after a while asleep

Whether the RG DS+ can drop from its normal sleep into something deeper once it has been asleep
for a while, what each state costs, and what nibOS does about it: `nib sleep off-after`
(default 10 minutes), a sleep hook and an RTC alarm. Measured 2026-10-27 on nibOS 0.1.0-alpha4
(kernel 7.3-rc7), the dev card, DSlot on the shelf with no game, Wi-Fi up, charger out, 53% to
49% over the runs.

## What the hardware and this kernel offer

| State | How | Drain | Wakes on | Back in |
|---|---|---|---|---|
| Awake, idle shelf | | 470-520 mA | | |
| s2idle (`/sys/power/mem_sleep` `s2idle`) | CPUs idle in firmware, SoC and PMIC in their normal modes | **123 mA** | lid, power key, RTC, charger | ~1.5 s |
| deep (`mem_sleep` `deep`, the default: `sleep.conf.d`) | TF-A system suspend; the RK817 is put in sleep mode (`rk8xx_suspend` sets its SLEEP pin function), where the device tree's `regulator-state-mem` turns off `vdd_cpu`, `vdd_gpu`, the panels' supplies, the SD and Wi-Fi supplies and the DVP rails, and holds `vdd_logic` at 0.9 V | **49 mA** | lid, power key, RTC, charger | ~1.5 s to `PM: suspend exit`; Wi-Fi 2.2 s after that |
| Hibernation | `CONFIG_HIBERNATION` is off | | | |
| Off | the PMIC off; only the RK817 and its RTC run | not measured (below) | power key, charger | ~6 s to the shelf, 37 s to everything |

The drain is the RK817's coulomb counter (`charge_now`) over one RTC-woken 10 minute sleep per
state, and agrees with the gauge's own `current_avg` read right after waking (-48.7 mA and
-50.6 mA after the two deep sleeps, -119.5 mA after s2idle). The script is
[`sleepmeas.sh`](#how-it-was-measured). A full 4247 mAh battery lasts about 3.6 days in deep
sleep and 1.4 days in s2idle; stock firmware's figure wasn't measured (the card would have to be
swapped), and the user's rule is to measure our own builds.

So: **nibOS already sleeps in the deepest state this kernel offers with the memory kept.**
There is no second, lower suspend state to fall into after ten minutes. The 49 mA is what the
board draws with TF-A's default sleep configuration and the device tree's regulator states;
bringing it down is a different job (see "Open" below), and it would help every sleep, not only
long ones.

### What can wake it

Checked on the device: the lid's hall sensor (`gpio-keys-hall`, `wakeup-source`), the RK817's
interrupt (its power key, its RTC alarm, the charger plugged in or out: all one wake IRQ, 21),
and, by the sleep hooks, nothing else. The RK808 RTC (`/sys/class/rtc/rtc0`) takes a
`wakealarm`; it fired within 2 s of the set time in every run (the alarm has 1 s resolution).
After it fires the kernel clears it, so `wakealarm` reads empty; an alarm the device was woken
before still shows its time. That is how the hook below tells an alarm wake from a lid or key.

Powered off, the lid does nothing (the hall sensor is a GPIO of an unpowered SoC): only the
power key or the charger switches the PMIC on. Whether the RK817 also powers on for its RTC
alarm is untested; nothing here relies on it.

## The deeper states that exist, and what they cost

**Hibernation (suspend to disk).** Possible in principle, not worth it here. It needs
`CONFIG_HIBERNATION`, a swap area on the card (zram is RAM: it goes with the power), of the
order of 300-500 MB written to the card at every hibernation and read back at every resume
(1 GB of RAM, ~500 MB in use on the shelf), a `resume=` path that works without an initramfs on
an SD card whose probe is asynchronous (`resumewait`), and every driver's freeze/thaw/restore to
work: Panfrost, the panels, rtw88, the RK817. Resume would be a boot plus the image read: 10-20
s, longer than a cold boot to the shelf. What it saves over powering off is only that the game
needn't save its state, which the emulators already do on SIGTERM.

**Powering off.** The device draws its "off" current (the PMIC's standby and the RTC; not
measured yet, see below), and comes back by the power key in ~6 s to the shelf. At shutdown
systemd sends `dslot.service` SIGTERM; DSlot passes it to the emulator, which saves its
auto-resume state and quits (the same path as the power menu's Sleep/Power off, up to 10 s),
so after the boot the game is one Resume away. This is what the user asked for in effect: after
N minutes asleep, stop paying 49 mA.

The arithmetic for the default of 10 minutes: ten minutes asleep costs about 8 mAh (0.2% of the
battery), the wake-and-shutdown about 0.5 mAh, and the next boot a few mAh. Powering off pays
off only when the device would otherwise have slept for hours, and it costs the user a ~6 s boot
and the game's reload instead of a ~2 s lid-open. 10 minutes is the default the user asked
for; something longer (30-60 minutes) would annoy less and save nearly as much, and it is one
setting away.

## The mechanism

- **The setting:** `nib sleep` shows it, `sudo nib sleep off-after MINUTES|never` sets it
  (1-1440). It is `/var/lib/nibos/state/sleep-off-after` on the state partition, so an update
  keeps it; absent means **10 minutes**. The wheel group may set it with sudo (sudoers
  `nibos-sleep`), for DSlot's System page later.
- **The hook:** `system-sleep/nibos-sleep-off`. Before sleep, on battery and with the setting
  not `never`, it sets the RTC's `wakealarm` that far ahead and records the time in
  `/run/nibos/sleep-off-alarm` (an alarm already set for sooner, e.g. a test's `rtcwake`, is
  left alone and nothing is recorded). After waking, if its recorded alarm has been reached and
  cleared by the kernel (so the alarm woke the device, not a lid or key) and the charger is
  still out, it logs `nibos-sleep-off: asleep the set time on battery: powering off` and starts
  `systemctl poweroff` through a transient timer 2 s later, so the suspend's own job finishes
  first. Woken early, it clears the alarm so it can't fire later. On the charger it does nothing
  (the device charges asleep, and `nibos-charge-boot`'s own loop sleeps it again after any wake).
  `/run/nibos/sleep-off-dry-run` makes it log instead of powering off.
- **Risk to waking:** none added. The alarm is set only on top of a sleep that already works,
  and a fired alarm leaves the device awake until it decides; a hook failure is logged and the
  hook exits 0, as the others do. The one new state is "off", which the power key leaves. The
  hook never touches the charger, so the charging path is unchanged.
- **Interactions:** `nib lid ignore` doesn't matter (the hook runs on any suspend). The 3 s
  power-key hold-off after waking (`nibos-input`) doesn't matter: nothing presses it. logind's
  5 s lid hold-off would re-suspend a device woken with the lid closed; the power-off is queued
  2 s after the hook, so it wins.

## Tested

- Dry run on the dev card: `nib sleep off-after 2`, `/run/nibos/sleep-off-dry-run`, `systemctl
  suspend`: the device woke 2 minutes later on its own and the journal has
  `nibos-sleep-off: dry run: asleep the set time on battery, would power off`.
- The real power-off run needs someone at the device to press the power key afterwards.

## Open

- **The "off" drain**, and the RK817's power-on sources (is its RTC alarm one?): a run with
  someone at the device.
- **Why deep sleep draws 49 mA.** The RK3568 in BSP deep sleep is reported around 10-20 mA.
  Candidates: TF-A's default sleep configuration (mainline has no `rockchip-suspend` node for the
  RK3568, so the 24 MHz oscillator and the PMU PLLs may stay up), `vcc_3v3` and `vcc_1v8` kept
  on (the AW88166 amps, the touch controller, the SD card's logic), the hall sensor, `vdd_logic`
  at 0.9 V. Each is a regulator-state or firmware change with a hang risk, to be tried one at a
  time with an RTC alarm as the safety net. Lower it and the power-off timer matters less.
- **DSlot's System page**: show and set `nib sleep off-after` (needs `nibkit`), a follow-up in
  dslot.

## How it was measured

`/tmp/sleepmeas.sh deep|s2idle SECONDS LABEL`, run as a `systemd-run` unit so the SSH session's
loss doesn't end it: writes a `sleep.conf.d` drop-in in `/run` for the mode, settles 5 s, reads
`charge_now`, `voltage_avg` and `current_avg`, sets `wakealarm` SECONDS ahead, `systemctl
suspend`, waits for `/run/nibos/resumed` to be touched, settles 3 s, reads again and appends a
line to `/tmp/sleepmeas.log`. The awake seconds at each end (~3 s at ~500 mA, ~0.4 mAh) are in
the 10 minute figures and make them about 2 mA high.
