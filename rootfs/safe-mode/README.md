# Crash recovery and the safe mode

When the boot UI fails, systemd starts it again; when it keeps failing, the device shows what
failed and a way out instead of dropping to the console's login prompt.

- **Restarts.** `nibos-recovery.conf`, a drop-in on `dslot.service`: `Restart=on-failure`
  after 1 s, at most 3 starts in 60 s. A clean exit still gives the console (the package's
  `OnSuccess=`), and so does a boot that skips DSlot (a charge-sleep).
- **The safe mode.** Once the restarts run out (`start-limit-hit`), `nibos-safe-mode.service`
  (`OnFailure=`; it conflicts with `getty@tty1`, which the package's own `OnFailure=` also asks
  for and a drop-in can't take away) runs `/usr/lib/nibos/safe-mode`: through KMS, with no GL
  or SDL (either may be what broke), "DSlot stopped working" and how (a crash and its signal,
  an exit status, the last error in DSlot's journal) on the top screen, and on the bottom one a
  menu drawn as DSlot's own (nibkit's `menu-raster`): Restart DSlot, Restart the device, Open a
  console, Power off, with the D-pad and A. `OnFailure=` fires at every failure, also the ones
  answered by a restart: the safe mode waits 2 s and leaves if DSlot is back.
- **Hangs.** systemd feeds the SoC's hardware watchdog (`dw_wdt`, 22 s;
  nibos-hw's `system.conf.d/50-nibos-watchdog.conf`), which its driver stops in suspend; the
  kernel reboots 10 s after a panic and panics on a soft or hard lockup
  (`kernel/config/nibos.config`).

```bash
rootfs/safe-mode/install.sh        # build in rgbuild, install the program, the unit, the drop-in
```

## Tested on the device

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
