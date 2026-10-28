# Suspend and Wi-Fi: two bugs that looked like one

Closing the lid puts the RG DS+ in deep sleep (suspend to RAM); opening it wakes it, and Wi-Fi
is back about 1.9 s later. The symptom (the device never comes back) has two independent
causes, each hiding the other, plus a separate fault in the USB host controller. This page
covers all three and how to test suspend.

## The symptom

On the stock boot chain (Rockchip's SPL, BL31 v1.46, OP-TEE, U-Boot), deep suspend works. With
mainline U-Boot in Falcon mode and an unpatched kernel, closing the lid suspends the device and
opening it doesn't wake it: a forced power-off, and a journal ending at
`PM: suspend entry (deep)`. It can look like the running game (its KMS page flips, its
real-time threads) or the lid; with `pm_test` some runs pass and some hang, and verbose console
output makes the failures go away by changing the timing.

## Bug 1: the PMU's 32 kHz clock, gated under Falcon

**What:** the RK3568's PMU low-power clock, `clk_rtc_32k`, runs from `clk_rtc32k_frac` (24 MHz
divided to 32.768 kHz). Only U-Boot proper's clock driver opens that gate (PMUCRU
`PMUGATE_CON0` bit 1). In Falcon mode there is no U-Boot proper: SPL starts TF-A and Linux
directly, the gate stays closed, and TF-A's core and cluster power-down, which both s2idle and
deep suspend go through, never completes. Every suspend hung, game or not.

**How it was found:** s2idle hanging as well as deep suspend pointed below the suspend type, at
firmware or something both paths share. The difference between the boot that worked and the
one that didn't was the bootloader, so the PMU, CRU and GRF registers of a boot through U-Boot
proper were diffed against a Falcon boot's ([`tools/device/memdump.c`](../../tools/device/memdump.c));
one gate differed. Opening it by hand on a Falcon boot
([`tools/device/memwrite.c`](../../tools/device/memwrite.c)): 10 of 10 RTC-woken suspend cycles.

**Fix:** SPL opens the gate (U-Boot `rk3568/0004`), and Linux marks the clock critical so it is
never closed whatever booted it (kernel `rk3568/0004`).

## Bug 2: the RTL8821CS, cold and then kept powered

With bug 1 fixed, suspend still hangs, reliably. Bisecting on the device rules out the
kernel patches, the configuration and the device tree, and points at `rtw88_8821cs` as first
probed at boot, which always logs SDIO timeouts. Reloaded once, the driver suspends and
resumes cleanly, and a sleep hook that unloads the driver before sleep and loads it after
works around it (Wi-Fi back ~4.6 s after waking). The cause is two things:

**A cold chip's first probe.** In SDIO 3.0 (UHS) mode rtw88 reaches the MAC registers through
the chip's indirect access engine. An RTL8821CS fresh from power-up has its SDIO host interface
suspended (`REG_SDIO_HSUS_CTRL` = 0x19, `HCI_SUS_REQ` set), and the engine doesn't answer then
(170 ms of polling, nothing). rtw88 lifted the suspend only in `rtw_mac_pre_system_cfg()`,
after the chip-version read (`REG_SYS_CFG1` read as 0 instead of `0x00494d31`, so the chip was
set up as an A-cut instead of an E-cut) and its own first write. Once rtw88 has powered the chip
up the state survives a WL_REG_ON reset, which is why a module reload was always clean. Direct
CMD52 reaches the system registers below 0x100 before power-on but not the MAC above, so it is
no alternative. Not the cause: BT_REG_ON, idle time after power-on, the 200 ms post-power-on
delay. **Fix:** kernel `generic/0002` lifts `HCI_SUS_REQ` at probe.

**Resume with the card kept powered.** With that fixed the chip was set up right and suspend
woke every time, but Wi-Fi didn't come back: rtw88 asks the MMC core to keep the card powered
(`MMC_PM_KEEP_POWER`), so it isn't reinitialised after resume, and after deep sleep the chip no
longer answered even CMD52 (`Timeout sending command`, `mac power on failed`, mac80211's
"Hardware became unavailable upon resume"). Without WoWLAN mac80211 stops and restarts the chip
around suspend anyway, so keeping it powered buys nothing. **Fix:** kernel `generic/0003`: no
keep-power without WoWLAN, so the core powers the card off through the power sequence and
initialises it again on resume; rtw88's resume restores what the core doesn't (the function
enable, the 512-byte block size, without which every CMD53 transfer times out, the host
interface resume, and the SDIO interrupt, released in suspend).

**Result** (kernel #35, 2026-10-18): a clean first probe, 4 of 4 RTC-woken cycles from a fresh
boot, Wi-Fi associated ~1.9 s after `PM: suspend exit`, the same address each time; no sleep
hook needed. Rechecked the same day with the hardware layer's sleep hooks (the USB host
controller unbound before sleep and bound again after): 2 of 2 RTC-woken cycles, iwd
`connected` 2.5 s after `PM: suspend exit`, the same address.

## The third fault: the USB host controller

The host-only USB-C port's controller (OHCI `fd840000.usb`, with its EHCI companion
`fd800000.usb`) hangs the kernel when it is suspended, even with nothing plugged in. The cause
is not known. The hardware layer unbinds both from their drivers before sleep and binds them
again after waking (`NIBOS_SLEEP_UNBIND` in the board's
[`board.conf`](../../boards/rg-ds-plus/hw/files/usr/lib/nibos/board.conf), run by
[`system-sleep/nibos-unbind`](../../hw/files/usr/lib/systemd/system-sleep/nibos-unbind));
plugged-in devices re-enumerate.

## Red herrings, for the record

- **An emulator's own KMS output.** With a game drawing through the emulator's own KMS page
  flips, `pm_test=processors` hung; on SDL's GL renderer it passed. The GL renderer is the
  better choice anyway (the GPU scales, the CPU is left to the 3D threads), but the passes were
  luck: suspend also hangs with no game at all.
- **The lid.** A software lid close (writing `SW_LID` into `gpio-keys-hall`, so logind and
  the emulator both saw it) hung too, with or without a game.
- **Verbose console output** hid the failures by changing the timing.

## How to test suspend

- Use default printk levels, and suspend from a `systemd-run` unit with an RTC alarm, so the
  test doesn't depend on the SSH session that is about to lose Wi-Fi:
  `sudo systemd-run sh -c 'rtcwake -m no -s 12; systemctl suspend'`.
- A test that "hangs" over SSH may only have lost Wi-Fi: check `journalctl -b -1` after a power
  cycle.
- `systemd-sleep` writes `/sys/power/mem_sleep` itself; to try s2idle, use a sleep.conf drop-in
  with `MemorySleepMode=s2idle` rather than echoing into sysfs.
- `/sys/power/pm_test` (`freezer`, `devices`, `platform`, `processors`) narrows down where a
  hang is, through `systemctl suspend` so the sleep hooks run.
- **A cold Wi-Fi chip without rebooting:** unload rtw88 and hci_uart, unbind `fe000000.mmc` from
  `dwmmc_rockchip`, drive GPIO0_A0 (`vcc_wifi`) low for 2 s through `/dev/mem` (GPIO0 at
  `0xfdd60000`, masked writes to DR_L/DDR_L; `tools/device/memwrite.c`), bind again, load
  rtw88.

## Asleep while Wi-Fi connects

Wi-Fi switched off and on, then the lid closed a second later: after waking, iwd stays
"connecting" until restarted (seen 2026-10-21). Suspending just as iwd starts to connect aborts the
association in the kernel (`aborting association ... by local choice`) while iwd is frozen, and
it never moves on. Reproduced on the dev card (`nib net down; nib net up`, suspend as iwd
starts autoconnecting: stuck in 2 of 3 tries). nibos-hw's sleep hook
(`system-sleep/nibos-wifi-wake`) starts `/usr/lib/nibos/wifi-after-wake` in the background at
every wake: if iwd is still "connecting" 20 s later, it restarts iwd, which connects again
(25 s after waking in the tests). `tools/hwtest.sh --wifi 3 radios` toggles Wi-Fi three times
and then sleeps while it connects: all passed with the hook.

## Dead ends: don't repeat

Tried and measured, or found not to work; the source says more. Add to it when something doesn't pay.

### Radios

- Wi-Fi power save on: the RTL8821CS drops off; a one-shot `iw` setting doesn't stick (iwd re-applies it); iwd's `PowerSaveDisable`.
- Unloading rtw88 around suspend: worked (~4.6 s back) but a workaround; the kernel patches (~1.9 s).
- Direct CMD52 on a cold chip reaches only registers below 0x100; BT_REG_ON, idle time and the 200 ms delay aren't the cold-probe cause.
- `MMC_PM_KEEP_POWER` through deep sleep: the chip stops answering; useless without WoWLAN.
- Not restoring the 512-byte SDIO block size on resume: every CMD53 times out.
- iwd without `UseDefaultInterface`: wlan1, wlan2... across reloads.

### Power and suspend

- The emulator's KMS output, the lid and console verbosity: red herrings for the suspend hang (pm_test passes were luck).
- Echoing into `/sys/power/mem_sleep`: systemd-sleep overwrites it; a sleep.conf drop-in.
- `clk_rtc32k_frac` gated under Falcon: every suspend hangs in TF-A.
- OHCI/EHCI suspended: hangs; unbound around sleep.
- udev LED rules on "change": loop forever.
- Mainline thermal trips at 70/75 °C: 4.3% of frames late; stock's 75/85.
- The stock firmware's TF-A sleep configuration (`rockchip,sleep-mode-config` 0x5ec over the
  vendor SIP call, 2026-10-27): the device hangs at suspend entry, nothing wakes it; its logic-off
  resume needs the vendor loader. Don't send CENTER_OFF or ARMOFF_LOGOFF without a resume path
  in SPL (docs/design/deep-sleep.md, "Where the drain goes").
- Wi-Fi and Bluetooth blocked before deep sleep (2026-10-29): the drain didn't move (48.7 to 46.1
  mA, inside the noise); the RTL8821CS isn't where the sleep current goes.
- schedutil's `rate_limit_us` raised from 256 µs to 10 ms (2026-10-21): frequency changes 98/s to 52/s and the governor's wakeups with them, but the battery current *rose*, 488 mA to 502-529 mA on the idle shelf (coulomb counter, 90 s windows). Keep the default.
