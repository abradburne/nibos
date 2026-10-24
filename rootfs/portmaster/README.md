# PortMaster

[PortMaster](https://portmaster.games) installs and runs *ports*: native Linux builds of games
and engines, packaged for handhelds, each a bash script plus a folder. This package makes nibOS
one of PortMaster's platforms: its own app runs on the top screen with the DS+'s controls, and
installs ports over Wi-Fi into `Roms/PORTS`, which DSlot lists on its Ports page
([`portmaster.toml`](files/usr/share/dslot/emulators/portmaster.toml) says how to start them);
the app itself is on DSlot's Apps page ([`apps/portmaster`](files/usr/share/dslot/apps/portmaster/app.toml)).

## How it fits together

| Where | What |
|---|---|
| [`portmaster-get`](get/src/main.rs) | Gets PortMaster when it (or a port) is first opened: asks, turns Wi-Fi on if need be, downloads the release pinned in `release.conf` (from the [PKGBUILD](PKGBUILD)), checks its SHA-256, unpacks it. Offers the pinned release over an older copy |
| `~/.local/share/PortMaster/` | The local copy, which PortMaster updates itself and keeps its runtimes in (`libs/`). Every port's script looks here (`$XDG_DATA_HOME/PortMaster`) |
| `/usr/lib/portmaster/nibos/` | nibOS's profile: [control.txt](files/usr/lib/portmaster/nibos/control.txt) (sourced by every port), [mod_nibOS.txt](files/usr/lib/portmaster/nibos/mod_nibOS.txt), the launch hooks, the gamepad's SDL mapping |
| `Roms/PORTS/` on `/mnt/ds` (the games card), else on `/mnt/roms` | Installed ports: `<Title>.sh` and its folder (`port.json`, pictures, the game) |
| [`/usr/bin/portmaster`](files/usr/bin/portmaster) | PortMaster's app (`--sync` only puts it in place) |
| [`/usr/lib/portmaster/run-port`](files/usr/lib/portmaster/run-port) `<port>.sh` | Runs one port: what DSlot starts |
| [`/usr/lib/portmaster/bin/`](files/usr/lib/portmaster/bin/) | Ahead of `PATH` for ports: `sudo` (refuses), `mount`/`umount` (runtimes through squashfuse) |
| `/usr/lib/portmaster/compat/` | Last on ports' `LD_LIBRARY_PATH`: `libtheoradec.so.1` for PortMaster's LÖVE, which Arch ships as `.so.2` (the same functions; only LÖVE's video playback uses it) |
| [`70-portmaster-uinput.rules`](files/usr/lib/udev/rules.d/70-portmaster-uinput.rules) | `/dev/uinput` for the input group, for gptokeyb |

Before each use, `portmaster` and `run-port` put nibOS's files back over PortMaster's (its
self-update replaces `control.txt`), as ROCKNIX's and JELOS's PortMaster do from their
`/usr/config/PortMaster`.

**Why nibOS downloads PortMaster instead of shipping it:** its release bundles files nibOS may
not pass on: the splash's font is CC-BY-NC-SA (non-commercial), the patcher's music is used by
permission to PortMaster alone, the GUI's music and sounds have no licence, and gptokeyb,
gptokeyb2 and oga_controls are GPL-2.0 binaries whose sources aren't pinned, so no source offer
could be made for them. Downloaded on request, PortMaster comes from PortMaster itself, as
its own installer and every port do, under its own licences; the image holds only nibOS's files.
PortMaster's installer (`Install.PortMaster.sh`) isn't used: it writes where a firmware's paths
say and expects root, and the profile has to be in place before PortMaster first runs.

## The nibOS profile

- **Paths.** `controlfolder` is `~/.local/share/PortMaster`. Ports find their folder as
  `/$directory/ports/<folder>`; `control.txt` sets `directory` to `mnt/ds/Roms` or `mnt/roms/Roms`
  from the port script's own path, and since FAT and exFAT ignore case, `ports` is `PORTS`.
  harbourmaster gets `HM_TOOLS_DIR`, `HM_PORTS_DIR` and `HM_SCRIPTS_DIR` from
  [env.sh](files/usr/lib/portmaster/env.sh).
- **No root.** `ESUDO` is empty and `PM_CAN_MOUNT=N` (ports link instead of bind-mounting);
  gptokeyb kills a game with `pkill` as the device's user, not root. Runtimes (Godot/FRT, .NET, Mono, GameMaker,
  AGS...) are `.squashfs` files ports mount with `$ESUDO mount`; `bin/mount` mounts them with
  squashfuse instead, and run-port unmounts what is left when the port ends.
- **The name.** `device_info.txt` names the firmware from `/etc/os-release`, which says Arch
  Linux ARM. `control.txt` sets `CFW_NAME=nibOS` after it, and `env.sh` adds a test for
  `/usr/lib/nibos/board.conf` to the local copy of `device_info.txt` for ports that source it
  again (the same test is in the profile prepared for PortMaster, below).
- **Screens.** SDL display 0 is the bottom (touch) panel and a port opens one window on display 0.
  `SDL_VIDEO_DISPLAY_PRIORITY=DSI-2` (SDL3's KMSDRM driver sorts its displays by connector name,
  and sdl2-compat passes the hint on to SDL2 programs) makes the top panel display 0 for the
  port, PortMaster's app and its splash. The top panel is 1024×768 (4:3), which `device_info.txt`
  reads from DRM: ports that size themselves to the screen fill it, those with a fixed 640×480
  are scaled by SDL's fullscreen (4:3 to 4:3, no bars). DSlot leaves a picture of the controls
  on the bottom panel while a port runs. Ports built with their own SDL2 (in the port's `libs/`) don't know the hint and
  open on the bottom panel.
- **OpenGL.** Mesa (Panfrost) has desktop OpenGL through EGL, so
  [libgl_nibOS.txt](files/usr/lib/portmaster/nibos/libgl_nibOS.txt) leaves gl4es out, as the
  firmwares with a desktop GL do: gl4es's own GBM output (`LIBGL_FB=4`) put no picture up
  (Blobby Volley 2), Mesa's GL does.
- **PortMaster's app.** Its splash (LÖVE) is stopped before the app starts: a program that isn't
  root gets the screens (DRM master) only by opening them while no one has them, and the app
  would otherwise open them under the splash and never be allowed to flip. The app gets a
  mapping by the printed letters (A right, B bottom) so "A Accept" is the button printed A, as
  in DSlot ([launch_hooks.txt](files/usr/lib/portmaster/nibos/launch_hooks.txt)).
- **Controls.** nibos-input's one gamepad, given to SDL as one line
  ([gamecontrollerdb.txt](files/usr/lib/portmaster/nibos/gamecontrollerdb.txt): the whole
  database, 460 KB, is too long for `SDL_GAMECONTROLLERCONFIG`, which ports export). SDL's
  names are by position (SDL A is the bottom face button, labelled B). gptokeyb's quit combo is
  **Select + Start** or **Menu + Start**; holding power opens DSlot's power menu, which asks
  run-port to stop the port (SIGTERM to its whole process group, SIGKILL 7 s later).
  gptokeyb ends a game with `pkill` through `sh`; `$GPTOKEYB` clears `LD_LIBRARY_PATH` for it
  (as `sudo` does on other firmwares), since ports' own libraries (an old `libreadline`) break
  the system's bash. A game that ignores SIGTERM (L'Abbaye des Morts in its intro) is killed
  by gptokeyb 3 s later.
- **RetroArch.** Some ports are a libretro core started with `$raloc/retroarch -L core.so`
  (2048, for one); nibOS has no RetroArch, so they don't start (`raloc` is unset).
- **32-bit.** nibOS has no armhf libraries; PortMaster sees no `armhf` capability and doesn't
  offer ports that need them.

## Build and install

```bash
cd rootfs/portmaster
orb -m rgbuild bash -lc 'PKGDEST="$PWD/out" BUILDDIR="$HOME/makepkg-build" makepkg -f --nodeps'
# on the device:
sudo pacman -U --needed portmaster-*.pkg.tar.xz
```

To move to a newer PortMaster: set `pkgver` to the release (dots for its dash) and `_pmsize` /
`_pmsha256` to its PortMaster.zip; users who have
already updated PortMaster in its app keep their newer copy.

## Prepared for PortMaster

The same profile, in PortMaster's own layout (`PortMaster/nibos/`, `mod_nibOS.txt`, a
`platform/nibos.py` that copies `nibos/control.txt` after updates, nibOS in `device_info.txt`,
`hardware.py` and `config.detect_paths()`, with tests), is in
[upstream/](upstream/README.md) as a patch on PortMaster-GUI, not yet submitted upstream.
