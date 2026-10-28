# Sleep, and powering off after a while asleep

Whether the RG DS+ can drop from its normal sleep into something deeper once it has been asleep
for a while, what each state costs, and what nibOS does about it: `nib sleep off-after`
(default 60 minutes), a sleep hook and an RTC alarm. Measured 2026-10-27 on nibOS 0.1.0-alpha4
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

The arithmetic for the default: ten minutes asleep costs about 8 mAh (0.2% of the battery), an
hour about 49 mAh (1.2%), the wake-and-shutdown about 0.5 mAh, and the next boot a few mAh.
Powering off pays off only when the device would otherwise have slept for hours, and it costs
the user a ~6 s boot and the game's reload instead of a ~2 s lid-open. The user first asked for
10 minutes and then chose **60**: a lid closed for a meal still opens on the game, and a device
forgotten overnight loses one hour's 49 mAh instead of a third of its battery. It is one setting
away either way.

## The mechanism

- **The setting:** `nib sleep` shows it, `sudo nib sleep off-after MINUTES|never` sets it
  (1-1440). It is `/var/lib/nibos/state/sleep-off-after` on the state partition, so an update
  keeps it; absent means **60 minutes**. The wheel group may set it with sudo (sudoers
  `nibos-sleep`), for DSlot's System page later.
- **The hook:** `system-sleep/nibos-sleep-off`. Before sleep, on battery and with the setting
  not `never`, it sets the RTC's `wakealarm` that far ahead and records the time in
  `/run/nibos/sleep-off-alarm` (an alarm already set for sooner, e.g. a test's `rtcwake`, is
  left alone and nothing is recorded). After waking, if its recorded alarm has been reached and
  cleared by the kernel (so the alarm woke the device, not a lid or key) and the charger is
  still out, it logs `nibos-sleep-off: asleep the set time on battery: powering off` and starts
  a transient service that takes a block inhibitor on sleep and the lid switch at once
  (`systemd-inhibit --what=sleep:handle-lid-switch`) and then asks for `systemctl poweroff`
  every 0.5 s until logind takes it (it refuses while the suspend that woke is still finishing:
  "Action suspend already in progress"). Woken early, it clears the alarm so it can't fire later. On the charger it does nothing
  (the device charges asleep, and `nibos-charge-boot`'s own loop sleeps it again after any wake).
  `/run/nibos/sleep-off-dry-run` makes it log instead of powering off.
- **Risk to waking:** none added. The alarm is set only on top of a sleep that already works,
  and a fired alarm leaves the device awake until it decides; a hook failure is logged and the
  hook exits 0, as the others do. The one new state is "off", which the power key leaves. The
  hook never touches the charger, so the charging path is unchanged.
- **Interactions:** `nib lid ignore` doesn't matter (the hook runs on any suspend). The 3 s
  power-key hold-off after waking (`nibos-input`) doesn't matter: nothing presses it. logind
  handles a lid that is still closed by suspending again about 2 s after the wake, its 5 s
  hold-off notwithstanding. The first version queued the power-off on a 2 s timer and lost that
  race every time: on 2026-10-28 the device, lid closed all night, woke each hour, had its
  power-off refused ("Action suspend already in progress") and slept again, from 100% to 26%.
  The inhibitor, taken before logind comes round, keeps it from suspending, and the retries
  wait out the suspend that is finishing.

## Tested

- Dry run on the dev card: `nib sleep off-after 2`, `/run/nibos/sleep-off-dry-run`, `systemctl
  suspend`: the device woke 2 minutes later on its own and the journal has
  `nibos-sleep-off: dry run: asleep the set time on battery, would power off`.
- The real run, 2026-10-27 21:45 JST, `nib sleep off-after 2`, the lid open: `suspend entry`
  21:45:08, `nibos-sleep-off: asleep the set time on battery: powering off` 21:47:09.2 (the
  alarm), `PM: suspend exit` 21:47:09.5, `poweroff requested ... (unit nibos-sleep-off.service)`
  21:47:12, DSlot stopped, the goodbye picture, `Journal stopped` 21:47:14.5: off 6.3 s after the
  wake. The power key then booted it: the shelf 5.5 s after the kernel started (`systemd-analyze`:
  6.9 s in all), `nibos-charge-boot: power-on source 0x80` (the key). The gauge's `charge_now`
  across the off time isn't usable for the off drain: at boot the driver re-estimates the count
  (18 mAh "gone" in 5.7 minutes, most of it the re-estimate under the boot's load).

## Open

- **The "off" drain** needs a meter or a long off interval with the voltage read before and after
  (the gauge re-estimates at boot); and whether the RK817's RTC alarm is a power-on source.
- **Why deep sleep draws ~45 mA.** See "Where the drain goes" below: the PMIC now really sleeps
  (4-8 mA), the radios aren't it, and what is left is the SoC under TF-A's default sleep (PLLs and
  the 24 MHz oscillator up). The stock firmware's deeper TF-A configuration needs a resume path
  through the loader that mainline SPL doesn't have.
- **DSlot's System page**: show and set `nib sleep off-after` (needs `nibkit`), a follow-up in
  dslot.

## Where the drain goes (2026-10-29)

The 49 mA was hunted one change at a time, each measured the same way (below), 10 minute
RTC-woken deep sleeps on nibOS 0.1.0-alpha6, DSlot on the shelf, charger out, battery 26% to
24%. The two columns: the coulomb counter over the sleep (the awake seconds at each end make it
~2 mA high) and the gauge's `current_avg` read right after waking (its average of the last
minutes asleep).

| Change | Coulomb counter | `current_avg` | Kept |
|---|---|---|---|
| Nothing (the alpha6 kernel, the night before) | 52.1 mA | 48.5 mA | |
| The stock firmware's TF-A sleep configuration (`rockchip,sleep-mode-config` 0x5ec, `wakeup-config` 0x10, sent over the vendor SIP call from a test module) | hung: no wake by RTC, power key or lid; a 15 s power-key hold | | no |
| GPIO0_A2 muxed to the PMU's `pmic_sleep` output (the RK817's SLEEP pin and the SYR827's VSEL) | 48.7 mA | 40.6 mA | **yes** |
| ... and Wi-Fi and Bluetooth switched off (`nib wifi off`, `nib bluetooth off`) before sleeping | 46.1 mA | 40.4 mA | no change |

**The PMIC never slept.** The device tree's `regulator-state-mem` tables (CPU, GPU, panel, SD,
DVP and OTG rails off, logic at 0.9 V) take effect only when the RK817's SLEEP pin is asserted,
which the PMU does through GPIO0_A2's `pmic_sleep` function during TF-A's suspend. Mainline's
`rk8xx` driver switches the RK817's pin function to "sleep" at every suspend, but the RG DS tree
this board's device tree derives from never muxes the SoC pin (`pinctrl` showed it "MUX
UNCLAIMED"), so the pin stayed a floating GPIO and the whole table was inert. The stock tree muxes
it in its `pmic-sleep` pinctrl state; mainline's PineNote muxes it in its default state, which is
what nibOS now does (the `pmic_sleep` node in the board's pinctrl, in the rk817's `pinctrl-0`).
Worth 4-8 mA, and the regulator table now means what it says.

**The stock TF-A configuration hangs.** The vendor kernel hands TF-A a sleep mode over
`SIP_SUSPEND_MODE` (0x82000003); the stock DTB's 0x5ec is CENTER_OFF, ARMOFF_LOGOFF, PMIC_LP,
HW_PLLS_OFF, PMUALIVE_32K, OSC_DIS and 32K_PVTM, with GPIO as the only wake source. Sent as is
(TF-A v1.46, the stock's, accepted all three calls), the first sleep never came back: with the
logic domain off, waking goes through the boot ROM and the vendor loader's resume path, which
mainline SPL under Falcon doesn't have. The remaining bits (PLLs off, oscillator off, PMU on the
PVTM 32 kHz) don't need that path and are the likely home of most of the remaining 40 mA; each is
an experiment that can need a hard power-cycle, so each needs the user's OK first.

**Not it:** the RTL8821CS (its `vcc_wifi` is `regulator-always-on`, as is the boot card's
`vcc3v3_sd`; with both radios blocked the drain didn't move), the touch controller (the goodix
driver puts it in its screen-off state), the AW88166 amps (powered down between streams), the
hall sensor (a bare GPIO input, nothing to switch), the LEDs (`leds-gpio` turns them off in
suspend; the charging LED is kept only while the charger is in, when the drain doesn't matter).
`/sys/kernel/debug/wakeup_sources` shows nothing holding the device up, and `suspend_stats` no
failed steps.

## How it was measured

`/tmp/sleepmeas.sh deep|s2idle SECONDS LABEL`, run as a `systemd-run` unit so the SSH session's
loss doesn't end it: writes a `sleep.conf.d` drop-in in `/run` for the mode, settles 5 s, reads
`charge_now`, `voltage_avg` and `current_avg`, sets `wakealarm` SECONDS ahead, `systemctl
suspend`, waits for `/run/nibos/resumed` to be touched, settles 3 s, reads again and appends a
line to `/tmp/sleepmeas.log`. The awake seconds at each end (~3 s at ~500 mA, ~0.4 mAh) are in
the 10 minute figures and make them about 2 mA high.
