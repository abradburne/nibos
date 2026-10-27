# nibos-hw: the hardware layer

Everything that makes a board behave as a device rather than a board: one gamepad, sound that
follows the headphone jack, sleep on the lid and the power key, LEDs, network on demand,
thermal limits. Above the kernel, below any desktop or shell. One Arch package, `nibos-hw`,
built for the board in `NIBOS_BOARD` from three levels of files:

| Path | What |
|---|---|
| [`daemons/`](daemons/) | Rust: `nibos-input`, `nibos-audio`, `nibos-backlight` and `nibos-keys` (the console's on-screen keyboard, with getty@tty1), and the tools `nibos-charge-boot`, `nibos-bootctl` (the A/B slots), `nibos-provision` (an image's `provision.toml`) and `nibos-update` (system updates, [design](../docs/design/updates.md)), with their logic in a tested library (`src/board.rs`, `src/gamepad.rs`, `src/controls.rs`, `src/backlight.rs`, `src/audio.rs`, `src/keys.rs`, `src/ab.rs`, `src/provision.rs`, `src/update.rs`, ...) |
| [`files/`](files/) | What every board gets: the daemons' units, the network target and its units, `nib`, `nib net` and `nib brightness`, the backlights' names, access and saved levels, zram, cpufreq, the sleep hook, the iwd and bluetooth drop-ins |
| [`socs/<soc>/hw/files/`](../socs/rk3568/hw/files/) | The SoC's: `usr/lib/nibos/soc.conf`, how it sleeps, the drivers kept out of start-up |
| [`boards/<board>/hw/files/`](../boards/rg-ds-plus/hw/files/) | The board's: `usr/lib/nibos/board.conf`, udev rules, logind, WirePlumber, thermal limits, iwd's settings, modules |
| [`install.sh`](install.sh) | Installs the three levels, the daemons and the board's firmware into a root; refuses if two levels have the same file |
| [`PKGBUILD`](PKGBUILD), [`nibos-hw.install`](nibos-hw.install) | The package, and its install hooks (which enable the units) |

## The board's description

The daemons and units don't name any device. They read `/usr/lib/nibos/board.conf` (and
`soc.conf`), `NAME=value` lines that systemd's `EnvironmentFile=`, a shell and
[`daemons/src/board.rs`](daemons/src/board.rs) all read. The RG DS+'s,
[`board.conf`](../boards/rg-ds-plus/hw/files/usr/lib/nibos/board.conf):

| Name | RG DS+ | Used by |
|---|---|---|
| `NIBOS_PAD_NAME` | `Anbernic RG DS+ Gamepad` | `nibos-input` (the merged gamepad), `nibos-keys`, the board's udev rules |
| `NIBOS_BUTTONS_DEVICE`, `NIBOS_STICK_DEVICE` | `gpio-keys-control`, `adc-joystick` | `nibos-input` |
| `NIBOS_VOLUME_DEVICE` | `gpio-keys-volume` | `nibos-input` (grabs it, passes the keys on as "nibOS volume keys", which `nibos-audio` reads) |
| `NIBOS_JACK_DEVICE` | `rk817_ext Headphones` | `nibos-audio` |
| `NIBOS_HEADPHONES_SINK`, `NIBOS_SPEAKERS_SINK`, `NIBOS_SPEAKERS_KEEPALIVE` | the two cards' node names; `1` | `nibos-audio` |
| `NIBOS_NETWORK_MODULES` | `rtw88_8821cs hci_uart` | `nibos-network-modules.service` |
| `NIBOS_SLEEP_UNBIND` | the USB host controller | `system-sleep/nibos-unbind` |
| `NIBOS_POWER_DEVICE`, `NIBOS_POWER_LONG_PRESS_MS` | `rk805 pwrkey`; `1000` | `nibos-input` (tap: sleep; hold: the power menu, or power off with no boot UI) |
| `NIBOS_BACKLIGHTS`, `NIBOS_BACKLIGHT_MIN_PERCENT` | `top:backlight1 bottom:backlight0`; `2` | `/usr/lib/nibos/backlight`, `nib brightness`, the splash |
| `NIBOS_LATE_MODULES` (`soc.conf`) | `hantro_vpu rockchip_rga` | `nibos-late-modules.service` |

A board without a stick, a volume device or a second sound card leaves those names out, and
the daemons do without. An administrator changes single names in `/etc/nibos/board.conf`, read
after the board's.

## What it does

**Controls.** `nibos-input` (system service) grabs the button and stick devices and re-emits
them as one standard gamepad: buttons by position (BTN_SOUTH is the bottom face button), Menu
as BTN_MODE, the D-pad as the ABS_HAT0 hat, the stick re-centred on its resting value and
scaled to ±32767 (left and up are negative). udev hides the raw devices from games and
libinput. It also grabs the volume keys and passes them on through its own device, "nibOS
volume keys", except Menu + volume (brightness). Back, power and the lid stay on their own
devices for the desktop or shell. [docs/engineering/input.md](../docs/engineering/input.md).

**Audio.** `nibos-audio` (user service) makes the headphones the default output while they're
plugged in and the speakers otherwise, keeps a silent stream playing to the speakers where the
amps need it, and turns the volume keys into steps onto the next multiple of 5% (a press, and every fifth
repeat while held) outside a desktop. WirePlumber rules name
the outputs, never suspend the speakers and hide the amps' input side.
[docs/engineering/audio.md](../docs/engineering/audio.md).

**Lid and power.** Closing the lid sleeps (deep, suspend to RAM); opening the lid or the power key
wakes. logind's post-wake hold-off is 5 s. `nib lid ignore` keeps the device running with the lid
closed (`nib lid sleep` undoes it, `nib lid` says which), as a logind drop-in kept across
restarts; a user who isn't root applies it through `nibos-lid@<setting>.service`, which polkit
lets wheel start.

The **power key** is `nibos-input`'s, not logind's (logind acts on a press the moment it goes
down, so it can't tell a tap from a hold; the board's logind drop-in tells it to ignore the key):

- **a tap** (released within `NIBOS_POWER_LONG_PRESS_MS`, 1000 ms; an administrator can set
  another in `/etc/nibos/board.conf`) sleeps;
- **a hold** to that time opens the **power menu**: `nibos-input` sends the boot UI's main
  process SIGUSR1 (`systemctl kill --kill-whom=main --signal=SIGUSR1 dslot.service`). DSlot puts
  away a running game (SIGTERM: save and quit, up to 10 s) and shows Sleep / Restart / Power off
  / Cancel; Cancel resumes the game. With no boot UI running (the base image's console, a program started over SSH), a
  hold powers off cleanly (`systemctl poweroff`, with the goodbye picture);
- the press that wakes the device is ignored (`system-sleep/nibos-resumed` marks the time in
  `/run/nibos/resumed`, and power events within 3 s of it don't count);
- the RK817 PMIC also has a long press of its own that cuts the power in hardware, whatever the
  software does: the way out of a hang. Its time is not in mainline's register definitions and
  hasn't been measured yet (the PMIC's system registers 0xF0–0xF5 read `82 8c a0 40 20 02` on
  2026-10-19); Rockchip's PMICs take several seconds, so the 1 s hold is well clear of it, but
  time it with a stopwatch before relying on a number.

**Asleep too long on battery, it powers off.** Deep sleep still draws ~49 mA on the RG DS+
(s2idle 123 mA; a full battery is gone in 3.6 days), and this kernel has nothing deeper with
the memory kept. So `system-sleep/nibos-sleep-off` sets an RTC alarm `nib sleep off-after`
minutes ahead (default 60; `never` turns it off; `/var/lib/nibos/state/sleep-off-after`, kept
across updates) when the device sleeps on battery, and if that alarm is what woke it and the
charger is still out, powers off cleanly (DSlot and the emulators save the game's resume state at
shutdown). On the charger it sleeps on. [docs/design/deep-sleep.md](../docs/design/deep-sleep.md).

`system-sleep/nibos-unbind` unbinds the devices in `NIBOS_SLEEP_UNBIND` before sleep and binds
them again after; on the RG DS+, the USB host controller, which hangs the kernel when it is
suspended. Wi-Fi needs no hook: two kernel patches power the card off in suspend and set it up
again on resume, and iwd reconnects ~1.9 s after waking.
[docs/engineering/suspend-and-wifi.md](../docs/engineering/suspend-and-wifi.md).

**LEDs.** On the RG DS+: green on while running, amber while the charger is plugged in
(charging or full), asleep too (the device tree keeps the amber LED's state in suspend;
plugging in or out wakes the device, which sets it again). The udev rules match only `add` events: setting a trigger raises a
`change` event, and matching those loops forever (the LEDs flicker and two udev workers eat
40% CPU).

**Charging while off.** Plugging a charger into a switched-off device switches it on (the
PMIC does that by itself). Rather than boot to the shelf because a cable went in, nibOS charges
asleep: `dslot.service`'s `ExecCondition` (`nibos-charge-boot --check`) reads what switched the
device on from the PMIC (`NIBOS_POWER_ON_SOURCE`; the RK817's ON_SOURCE register) and, when it
was the charger alone and the charger is still in, skips the boot UI. `nibos-charge-boot.service`
then turns the backlights off and suspends, with the amber LED lit. The power key or opening
the lid carries on with the normal boot (the boot UI starts); unplugging powers it off; any
other wake sleeps again. Any error reading the PMIC boots normally.

**Crash recovery.** systemd feeds the hardware watchdog (`system.conf.d/50-nibos-watchdog.conf`:
`RuntimeWatchdogSec=20s` in IRQ mode, `RuntimeWatchdogPreSec`: about 22 s after the last ping
its first stage panics the kernel, and the panic restarts the device through the PMIC, as the
watchdog's own reset can't on the RG DS+; stopped in suspend by its driver). DSlot's restarts
and the safe mode are in [rootfs/safe-mode](../rootfs/safe-mode/README.md).

**Battery.** udev links the board's battery and charger (`NIBOS_BATTERY`, `NIBOS_CHARGER`) as
`/run/nibos/battery` and `/run/nibos/charger`. Charging is the stock firmware's: 1.5 A from USB,
at most 2 A into the cell, to 4.35 V (the device tree). **Battery protect**, for a device left
on its charger, lowers the charge voltage to 4.1 V (`NIBOS_BATTERY_PROTECT_UV`), at which the
cell stops at about 80-85% and the battery stays connected: `nib battery protect on|off` (the
wheel group may; kept in `/var/lib/nibos/battery/protect` and set again by udev at boot),
`nib battery` for the charge and the setting. It is the kernel's `constant_charge_voltage`
(the rk817 patch), which the wheel group may also write. The amber LED stays on while plugged
in.
[docs/engineering/battery.md](../docs/engineering/battery.md).

**Brightness.** Each screen has its own backlight, and nibOS offers them to programs as two
named, unprivileged knobs (the first case of nibOS's [device interface](../docs/engineering/device-interface.md)):

- **Names.** The board's `board.conf` says which backlight lights which screen
  (`NIBOS_BACKLIGHTS="top:backlight1 bottom:backlight0"` on the RG DS+, from the device tree's
  panel nodes), and udev links each as `/run/nibos/backlight/<screen>`, the kernel's backlight
  directory (`brightness`, `max_brightness`, `scale`). Programs use those names, never
  `backlightN`.
- **Who may set them.** The same udev rule gives the video group write access to
  `brightness`; the boot UI (`dslot.service`, `SupplementaryGroups=video`) and `$NIBOS_USER`, made by
  the root image, are in it. No root, polkit or logind session needed.
- **Kept per screen.** `nibos-backlight.service` saves each backlight's level to
  `/var/lib/nibos/backlight/<backlightN>` at shutdown, and `nib brightness` and the shortcut
  save after each change. udev sets the saved levels back when the backlights appear (~2.5 s
  into a boot), and the splash sets them before its picture goes up (1.5 s), so the logo
  already comes up at the saved level. Suspend keeps the levels as they are. systemd's own
  restore (`systemd-backlight@`) is turned off for them, so nothing restores twice.
- **A floor.** Nothing nibOS sets or restores goes below `NIBOS_BACKLIGHT_MIN_PERCENT` of the
  maximum (2% on the RG DS+, level 33 of 1666), so a screen can't be left dark by a slider, a
  shortcut or a bad saved value. Writing a lower value to `brightness` directly still works;
  UIs shouldn't.
- **Perceptual percent.** Programs speak in percent: 0 is the floor, 100 the maximum, and
  equal steps look equal. The RG DS+'s backlights are PWM with 1666 levels, and the kernel's
  pwm-backlight driver already spreads them on a perceptual curve (`scale` = `non-linear`), so
  percent is linear between floor and maximum: `level = floor + (max - floor) × p / 100`. On a
  backlight whose `scale` is `linear`, nibOS squares it: `floor + (max - floor) × (p/100)²`.
  [`daemons/src/backlight.rs`](daemons/src/backlight.rs) has the mapping and its tests;
  `nibos-backlight` and `nib brightness` use it:

  ```bash
  nib brightness                 # top      49%  (833/1666), bottom ...
  nib brightness top 30          # the top screen at 30%, kept
  nib brightness all +10         # both, 10 percent brighter
  ```

- **Screen off.** A program that wants a screen dark (an emulator saving battery on the bottom
  screen) writes `4` to `/run/nibos/backlight/<screen>/bl_power` (video group, like `brightness`),
  and `0` to light it again; the level stays as it was, so nothing dark is ever saved. Any
  brightness nibOS sets turns the screen back on, so Menu + volume wakes a dark screen from
  anywhere; the kernel starts with both on.
- **Menu + volume up / down** changes both screens together, each onto the next multiple of 5%
  a press and a step every fifth key repeat while held (`hw/daemons/src/steps.rs`), from anywhere: the shelf, a game, the console. `nibos-input` handles it, so it
  never reaches the game or the volume ([docs/engineering/input.md](../docs/engineering/input.md#menu--volume-brightness)).

**Thermal limits.** The RG DS+'s `nibos-thermal.conf` (tmpfiles.d) sets the CPU and GPU zones'
passive trips to the stock firmware's 75 / 85 °C instead of mainline's 70 / 75 (the 95 °C
shutdown stays): from 70 °C the CPU dropped to 1.42 GHz minutes into a heavy game, and 4.3% of
frames ran late in Golden Sun at 2× hi-res 3D; with these, 0.6%.

**Wi-Fi.** iwd manages wlan0 (no NetworkManager). With power save on, the RTL8821CS stops
answering while awake and the device drops off the network; the board's `/etc/iwd/main.conf`
sets `[DriverQuirks] PowerSaveDisable=*`, since iwd re-applies power save on every connect and
a one-shot `iw ... set power_save off` wouldn't stick (check with `iw dev wlan0 get
power_save`). `UseDefaultInterface=true` keeps iwd on the kernel's wlan0 across driver
reloads.

**Network modes.** Wi-Fi and Bluetooth are not part of every boot. `nibos-network.target` loads
the drivers (`nibos-network-modules.service`; the drivers are blacklisted in modprobe.d so udev
leaves them alone) and starts iwd and bluetooth, which are `PartOf` it. `nib net up|down|status`
starts and stops it, and a unit can ask for it with `Wants=nibos-network.target`. How the
system starts it is one switch, `nib net mode` (or `NETWORK_MODE` in `rootfs/make-arch.sh`):

| Mode | For | When the network comes up |
|---|---|---|
| `boot` | the dev system | with the system |
| `deferred` | a productivity setup | once start-up has finished, at nice 19 and CPUWeight 1 (a `Type=simple` unit that waits for `is-system-running`), so it's never on the boot path |
| `demand` | a game player | when something starts the target (RetroAchievements, an update, an SSH session wanted): `nib net up` |

sshd listens on all addresses from a socket and isn't ordered behind the network. What each
mode costs at boot: [bench/results/network.md](../bench/results/network.md).

**The Wi-Fi switch.** `nib wifi on|off|status` turns the Wi-Fi radio alone on or off (rfkill),
leaving Bluetooth as it is (`nib net down` stops both, Bluetooth headphones included). Off saves as much as stopping the whole network (464 to 371 mA idle), systemd keeps it
across restarts, and the wheel group may use it (`72-nibos-rfkill.rules`). On starts the
network first if a mode left it down.
`nib bluetooth on|off|status` is the same for Bluetooth. Both choices last: systemd-rfkill
keeps each radio's block and puts it back when the drivers load, and in the demand and deferred
modes `nibos-radios-restore` brings the network up after start-up when either radio was left on
(otherwise nothing would load the drivers, and a radio switched on would come back off).

**Start-up.** The kernel boots with the performance governor; `nibos-cpufreq` hands the CPU
to schedutil once booted. `nibos-late-modules` loads the SoC's video drivers after start-up,
and `nibos-zram` sets up compressed swap as large as RAM, also after start-up (as a swap.target
member it held up `local-fs.target`).

**`nib`.** `/usr/bin/nib` runs `/usr/lib/nibos/nib-<command>`: this package brings `nib net`,
`nib wifi`, `nib bluetooth`, `nib brightness`, `nib lid`, `nib battery`, `nib audio`,
`nib update`, `nib bug-report`, `nib sleep` (power off after this long asleep on battery) and `nib clock` (the time, zone and network time, for DSlot's
Clock page: `sudoers.d/nibos-clock` lets wheel run it), the boot UI brings `nib boot-ui`, and
`nib help` lists what is installed.

**Firmware.** The package builds the board's own tuning files (`boards/<board>/hw/*-tuning.py`:
the RG DS+'s AW88166 speaker amps, with the DSP bypassed and conservative gain) into
`/usr/lib/nibos/`. The board's firmware from its stock image (`BOARD_FIRMWARE` in `board.sh`;
Awinic's official tuning file) isn't packaged, as it isn't ours to redistribute: on a
development card `rootfs/make-arch.sh` or [`install-firmware.sh`](install-firmware.sh) puts the
copy `boards/<board>/extract-stock.sh` took there, and on any device `nib audio import-tuning`
imports it from a stock card or image. `nib audio tuning nibos|official` picks the active one,
`/var/lib/nibos/firmware/aw88166_acf.bin`, which the kernel reads first
(`firmware_class.path`). [docs/engineering/audio.md](../docs/engineering/audio.md).

## Build and install

In the build machine:

```bash
orb -m rgbuild bash -c 'cd hw && PKGDEST="$PWD/out" BUILDDIR="$HOME/makepkg-build" makepkg -f --nodeps'
```

The build copies the sources into makepkg's own directory, fetches the crates `Cargo.lock`
pins and builds offline; `check()` runs the unit tests. On a running device:
`scp hw/out/nibos-hw-*.pkg.tar.* $DEVICE:/tmp/` then `sudo pacman -U /tmp/nibos-hw-*.pkg.tar.*`.
New root images ([`../rootfs/make-arch.sh`](../rootfs/make-arch.sh)) include it.

For the daemons alone, in the build machine: `cd hw/daemons && cargo test && cargo clippy`
(or `make rust` from the Mac).

## Tested on the device

Every button, D-pad and stick direction on the merged gamepad; speakers in stereo with no
clipped starts; speakers chosen as default without headphones; volume keys; LEDs steady;
charging detected (+1.17 A from a wall charger); lid and power-key sleep and wake, with Wi-Fi
back on its own.

Not yet: headphone plug/unplug switching (needs headphones), rumble through the merged
gamepad, battery-low shutdown, sleep drain over hours, the cause of the OHCI suspend hang.
