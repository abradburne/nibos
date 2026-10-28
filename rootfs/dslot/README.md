# DSlot as the boot UI

DSlot is a separate project (`DSLOT_SRC`, the `dslot` repository next to this one) with its own
Arch package, `dslot`: the program, its unit (`dslot.service`, started from `sysinit.target`, in
place of the tty1 login shell) and the splash's picture. Its `docs/nibos.md` describes what it
does on nibOS. This directory is what nibOS adds around the package:

| File | On the device | |
|---|---|---|
| [`user.conf`](user.conf) | `/etc/systemd/system/dslot.service.d/user.conf` | the user DSlot runs as and its PipeWire (`XDG_RUNTIME_DIR`); without it the unit would run DSlot as root, which DSlot refuses |
| [`50-dslot-power.rules`](50-dslot-power.rules) | `/etc/polkit-1/rules.d/` | lets that user power off and restart from DSlot's menu, with no login session |
| [`nib-boot-ui`](nib-boot-ui) | `/usr/lib/nibos/nib-boot-ui` | `nib boot-ui on|off|status`: DSlot or the shell at power-on |
| [`nibos-rcu.conf`](nibos-rcu.conf) | `/etc/tmpfiles.d/` | expedited RCU grace periods: each `close()` of an input device waits for one, and SDL opens and closes all of them looking for controllers: ~0.3 s of every game's start and of the return to the shelf, ~0.1 s expedited |
| (in `install.sh`) | `/etc/fstab` | the TF2 card (`/dev/nibos-games-card`, exFAT or FAT) at `/mnt/ds`, `noauto` (`exfatprogs` replaces fuse-exfat's `exfat-utils`) |
| `70-nibos-games-card.rules` | `/etc/udev/rules.d/` | links the TF2 card's volume (any name) to `/dev/nibos-games-card` and mounts it when it appears and lets systemd unmount it when it goes; an automount stalled every access by its device timeout with the slot empty |

```bash
rootfs/dslot/install.sh                       # build the dslot package in rgbuild; install it and the files above
ssh $DEVICE sudo nib boot-ui on               # or off: back to the tty1 shell
bench/boot-ui.sh                              # 3 restarts, shelf on screen from reset
```

How it gets on screen fast: [docs/boot-time.md](../../docs/boot-time.md#the-boot-ui-a-picture-at-15-s-the-shelf-at-52-s).
DSlot's settings are in `/etc/default/dslot` (its `docs/nibos.md`). Leaving DSlot (Select +
Start) or DSlot failing starts the shell on tty1. The unit gives DSlot `CAP_DAC_OVERRIDE` to open
the DRM and input nodes before udev has set their groups: a known shortcut, explained in DSlot's
unit and in [docs/architecture.md](../../docs/architecture.md#known-shortcuts).

## Games

Emulators are separate packages. Each describes itself to DSlot with a file in
`/usr/share/dslot/emulators/<id>.toml`: the systems it plays and the command for each (DSlot's
`docs/emulators.md`). Choosing a game, DSlot closes SDL, which lets go of both panels and the
audio device, runs the command as its user inside `dslot.service`, waits for it to exit, and
takes the screens back. The game takes both panels itself (two KMSDRM displays; display 0, DSI-1,
is the bottom touch panel); Menu is the conventional way back to the shelf. What nibOS gives
games running there: the unit's real-time limit (`LimitRTPRIO=10`), expedited RCU, and the
second card.
