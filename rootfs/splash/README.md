# Boot and goodbye pictures

**At power-on**, the boot picture (the DSlot logo, on the top screen; the bottom one black) about
1.5 s from reset, before systemd, in both editions. The kernel runs `nibos-splash` as `init=`.
It forks and execs systemd at once; the child puts `/usr/lib/nibos/pictures/boot.pspl` up
through KMS and keeps it there until something else sets the screens: DSlot's first frame, or
the console's login prompt in the base edition. If nothing does within 30 s, the console gets
them back. When the charger switched the device on (the RK817's ON_SOURCE says so) there is no
picture: it charges asleep, dark (`nibos-charge-boot`, [hw/README.md](../../hw/README.md)).
Numbers and findings: [docs/boot-time.md](../../docs/boot-time.md#the-boot-ui-a-picture-at-15-s-the-shelf-at-52-s).

**At power-off and restart**, the goodbye picture on the top screen (the bottom one black), once
the boot UI or the console has let go of the screens, kept until the power goes:
`nibos-goodbye.service` is stopped after `dslot.service` and `getty@tty1.service` and runs
`nibos-splash --goodbye`, whose holder (`@nibos-goodbye`: out of the service's cgroup, and
named so systemd's last kill passes it over) keeps the card open. Not after a charge-sleep,
which powers off dark. Shutdown takes as long as without it (a restart's, first "Stopping" to
"Journal stopped": 1.87 s median over 4 each way, 2026-10-21).

**Over a handover** (`nibos-splash --hold PICTURE [SECONDS] [--while PID]`, as `$NIBOS_USER`): a
program's own picture in the same format, put up while no one has the screens, then let go of
them so the next program becomes their master when it opens them, and kept until the top screen
shows something else (or 30 s). With `--while`, its buffers are kept as long as process PID
runs and until the top screen changes again: the next program's SDL puts them back as it quits,
and DSlot replaces them a moment later, with no dark frame between. PortMaster's "Starting PortMaster..." while its own app starts
([rootfs/portmaster](../portmaster/README.md)).

```bash
rootfs/splash/install.sh           # build in rgbuild, install the splash, the pictures, the unit
rootfs/splash/boot.sh on           # init= in the Falcon FIT (off: plain again; status)
bench/boot-ui.sh                   # "splash" column: when the picture went up
```

The fallback boot (A held at power-on) never has the boot picture: it keeps the plain command
line, so a splash that breaks the boot can't take the way back with it.

## The pictures

[`pictures/`](pictures/): `logo.svg` (the DSlot logo, 350×160) and `goodbye.svg` (1024×768, a
sleepy pixel handheld: "SEE YOU SOON"), with their PNG renders. They come from
[dsboot](https://github.com/abradburne/dsboot) (`pictures/dslot/logo.svg`,
`pictures/goodbye.svg`), by the same author, GPL-3.0-or-later like this directory; the logo is
DSlot's own (`assets/logo.png` there is the same file). `nibos-pspl` (`src/bin/`) turns a PNG
into the splash's format at build time (`install.sh`, `tools/make-image.sh`): the picture in
the middle of the top screen, on black, the bottom screen black. To change one, edit the SVG,
render the PNG beside it, and rebuild.
