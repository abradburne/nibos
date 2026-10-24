# The device interface: settings programs may change

Programs on a nibOS handheld (the boot UI, a game, a settings screen, a script) need to change
the device: a screen's brightness, the sound output, the network. They run as an ordinary
account, with no root, no password prompt and, for the boot UI, no logind session. This page is how nibOS lets
them, so each new setting is done the same way instead of with one-off rules.

## Two kinds of setting

**A plain knob** is a value the kernel already exposes, where any value in range is safe and
nothing has to be decided but the value: a backlight's level, an LED. For these:

1. **A name, not a kernel path.** The hardware layer links the kernel's file or directory under
   `/run/nibos/<setting>/<name>`, named for what it is (`backlight/top`, not
   `backlight1`). The board's `board.conf` says which device is which; udev makes the link
   when the device appears. Programs use only the `/run/nibos` name, so the same program works
   on another board.
2. **A group, not root.** The same udev rule opens the value to the group that owns that kind of
   hardware (`video` for screens, `input` for LEDs on controls, `audio` for sound). The account
   the root image creates and the boot UI's unit are in them.
3. **nibOS keeps it.** If the setting should survive a restart, the hardware layer saves it
   (`/var/lib/nibos/<setting>/`) and sets it back at boot, as early as it matters; systemd's
   own saving for the same thing is turned off, so nothing restores twice.
4. **Limits in one place.** What a program may not do by accident (a dark screen) is the
   hardware layer's floor or ceiling, applied by everything nibOS sets and restores, and by the
   `nib` command.
5. **A unit people understand.** Programs speak in percent or names; the mapping to raw values
   (perceptual brightness) lives in the hardware layer's Rust library, used by its daemons and
   by `nib`, and is written down so a UI can do the same.
6. **`nib <setting>`** for people and scripts, the same rules applied.

A program may write the `/run/nibos` file directly (that is the interface; `nib` is a
convenience), or run `nib <setting>`.

**A setting with policy** needs a decision the kernel can't make: which sink is the default,
whether the network may come up, a CPU performance mode that trades battery, anything
where two programs could disagree or a bad value has consequences beyond the setting. These
belong in a small nibos-hw service (Rust, like the daemons) that programs ask, over a socket or
D-Bus, and that applies the policy and tells everyone else what changed. It doesn't exist
yet; the first setting that needs it will add it. Until then such settings stay root-only
behind `nib` (as `nib net` is).

## Settings

| Setting | Kind | Interface | Status |
|---|---|---|---|
| Brightness, per screen | Plain knob | `/run/nibos/backlight/<screen>/brightness` (video group), `nib brightness`, Menu + volume for both; changes seen through inotify (`nibkit::backlight::watch`) | **Done** ([hw/README.md](../../hw/README.md), "Brightness") |
| Screen off, per screen | Plain knob | `/run/nibos/backlight/<screen>/bl_power` (video group): `4` powers the backlight down, `0` on (`nibkit::backlight::set_off`). The level isn't touched, so nothing dark is saved; any brightness nibOS sets (`nib brightness`, Menu + volume) turns it back on; a program that turns a screen off turns it on when it exits | **Done** |
| Lid closing sleeps or not | Plain knob, applied as root | `nib lid sleep|ignore` (a logind drop-in, kept across restarts); a non-root user goes through `nibos-lid@<setting>.service`, which polkit allows wheel to start | **Done** |
| Battery protect (charge to about 80%) | Plain knob | `/run/nibos/battery/constant_charge_voltage` (wheel; below `constant_charge_voltage_max`: on), `nib battery protect on|off` (kept across restarts), `nibkit::battery` ([battery.md](battery.md)) | **Done** |
| Volume | Plain knob, through PipeWire | The volume keys (`nibos-audio`), `wpctl` as the account | Works; no `nib volume` yet |
| Sound output (speakers or headphones) | Policy | `nibos-audio` follows the jack; a program can't choose yet | Planned: the service |
| LEDs | Plain knob | Kernel LED triggers set by udev; not open to programs | Planned: `/run/nibos/led/<name>` for the ones nibOS doesn't drive |
| CPU and GPU performance mode | Policy | schedutil after boot (`nibos-cpufreq`); a program may ask for a minimum clock for its own threads with uclamp (`sched_util_min`, no root) | Works per thread; planned: the service, with battery and thermal limits (it can cap clamps with `sched_util_clamp_min` and a unit's `cpu.uclamp.*`) |
| Network on or off | Policy | `nib net up|down|status`: the image's account (wheel, through nibOS's polkit rule) may start and stop the network target; `nib net mode` needs root | Works for the image's account; planned: the service, when wheel loses its blanket rule |
| Rumble | Plain knob | The kernel's force-feedback on the merged gamepad (not passed through yet) | Planned: in `nibos-input` |

When one of these is done, it follows the pattern above and its row here moves to Done.

## Noticing changes

A plain knob's value lives in one place, the kernel, and anything may change it: a shortcut, `nib`,
another program. A program that shows a setting reads it when it shows it and keeps no copy of
its own; to follow changes made while it is on screen, it watches the `/run/nibos` file with
inotify (`IN_MODIFY`, which the kernel raises for every write to a sysfs value, through the
link or not). No service is involved, so a program in any language can. `nibkit` wraps it per
setting: `nibkit::backlight::watch()` for brightness and screen power. Checked on the RG DS+
(2026-10-19): Menu + volume, `nib brightness` and a program's own write each raise it on the
screens they change. Settings with policy will announce changes from their service instead.

## The power menu and "save and quit"

The one request nibOS sends programs, rather than a knob they set. Holding power (1 s) asks the
boot UI for its power menu with **SIGUSR1** to its main process; the boot UI asks the game it
started to **save and quit with SIGTERM**, waits up to 10 s (then SIGKILL), and shows Sleep /
Restart / Power off / Cancel; Cancel starts the game again, which resumes. Every nibOS program
handles SIGTERM as "save your resume state and exit 0": it is also what systemd sends at
shutdown, and what quit-and-resume app switching will send ([design/app-model.md](../design/app-model.md)).
`nibkit::power` has the signal and a watcher for a program that shows the menu itself.
