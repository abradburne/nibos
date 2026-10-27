# Crash recovery and the safe mode

When the boot UI fails, systemd starts it again; when it keeps failing, the device shows what
failed and a way out instead of dropping to the console's login prompt.

- **Restarts.** `nibos-recovery.conf`, a drop-in on `dslot.service`: `Restart=on-failure`
  after 1 s, at most 3 starts in 60 s. A clean exit still gives the console (the package's
  `OnSuccess=`), and so does a boot that skips DSlot (a charge-sleep).
- **The safe mode.** Once the restarts run out (`start-limit-hit`), `nibos-safe-mode.service`
  (`OnFailure=`; it conflicts with `getty@tty1`, and the package's own `OnFailure=getty@tty1`
  is taken out of the unit by `make-arch.sh` and `install.sh`: a drop-in can't take it away, and
  with both named, systemd started the console and dropped the safe mode after a crash) runs `/usr/lib/nibos/safe-mode`: through KMS, with no GL
  or SDL (either may be what broke), "DSlot stopped working" and how (a crash and its signal,
  an exit status, the last error in DSlot's journal) on the top screen, and on the bottom one a
  menu drawn as DSlot's own (nibkit's `menu-raster`): Restart DSlot, Restart the device, Open a
  console, Power off, with the D-pad and A. `OnFailure=` fires at every failure, also the ones
  answered by a restart: the safe mode waits 2 s and leaves if DSlot is back. Once it has taken
  the screen it forgets DSlot's failures (`systemctl reset-failed`), and `dslot.service`'s
  `ExecStartPre=` stops it: a `systemctl start dslot` from anywhere (its own menu, `nib boot-ui`,
  a test install) brings DSlot back instead of being refused as repeated too quickly, or failing
  to open the display the safe mode holds.
- **Hangs.** systemd feeds the SoC's hardware watchdog (`dw_wdt`, 22 s in IRQ mode;
  nibos-hw's `system.conf.d/50-nibos-watchdog.conf`), which its driver stops in suspend; its
  first stage panics the kernel (the watchdog's own reset never brings the RG DS+ back:
  [ab-rollback.md](../../docs/design/ab-rollback.md#how-a-watchdog-timeout-restarts-the-board)); the
  kernel reboots 10 s after a panic and panics on a soft or hard lockup
  (`kernel/config/nibos.config`).

```bash
rootfs/safe-mode/install.sh        # build in rgbuild, install the program, the unit, the drop-in
```

## Tested on the device

- 2026-10-27, alpha2 card: DSlot SIGKILLed four times 2.5 s apart with the package's
  `OnFailure=getty@tty1` still in the unit: the console came up each time and the safe mode
  never did. With it removed: the safe mode, "It crashed (signal 9)". `systemctl start dslot`
  then: accepted (no "repeated too quickly"), "Stopping Safe mode", the shelf back in 0.5 s.

- DSlot killed (SIGKILL) four times in 10 s: restarted three times (the safe mode started each
  time, saw DSlot back and left), then `start-limit-hit` and the safe mode, with "It crashed
  (signal 9)"; no login prompt. A press of A on "Restart DSlot" (injected into the gamepad's
  event device) brought DSlot back.
- The watchdog through 5 RTC-woken suspends of 20 s and one of 60 s (its timeout is 22 s): no
  reset, the same boot.
- `echo c > /proc/sysrq-trigger`: rebooted by itself, back on DSlot 36 s later.
- Boot to the shelf: 5.52-5.65 s before, 5.52-5.88 s with the new kernel and the watchdog
  (3 boots each; kernel to init 0.94 s in both): no cost above the noise.

GPL-3.0-or-later: it builds the splash's KMS code (`../splash/src/kms.rs`) in.
