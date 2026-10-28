# Root file system

Arch Linux ARM for the board, built as an ext4 image in the build machine: written to a
development card's root partition (partition 12 on the RG DS+), or, with `IMAGE=1`, the root
of a downloadable image (`tools/make-image.sh`, [docs/design/images.md](../docs/design/images.md)).
It is the dev system and the base the boot UI goes on.

| Path | What |
|---|---|
| [`make-arch.sh`](make-arch.sh) | Builds `out/archroot.img`: pacstrap, the kernel's modules, `nibos-hw`, the user, SSH, network, then the boot-time trimming |
| [`tune-boot.sh`](tune-boot.sh) | The boot-time trimming, each change with why ([docs/boot-time.md](../docs/boot-time.md)) |
| [`install-arch.sh`](install-arch.sh) | Mac: adds the root partition over the card's free space and writes the image to it |
| [`files/`](files/) | nibOS's own units and scripts in the image: growing the root file system, the Wi-Fi import, the boot logs, polkit for wheel, mDNS (and the board's, from `boards/<board>/rootfs/files/`) |
| [`dslot/`](dslot/README.md) | DSlot as the boot UI, and the games' launcher and settings |
| [`splash/`](splash/README.md) | The power-on splash (`init=`) |
| [`readahead/`](readahead/README.md) | Read-ahead of the boot UI's files from a systemd generator |
| [`mesa/`](mesa/README.md) | Mesa with only Panfrost and no LLVM |
| [`safe-mode/`](safe-mode/README.md) | DSlot's restarts and the safe mode when it keeps failing |
| [`portmaster/`](portmaster/README.md) | PortMaster as a nibOS platform, downloaded on request |
| [`no-fdk-aac/`](no-fdk-aac/README.md) | A stand-in for `libfdk-aac`, so the images don't carry it |
| [`image/`](image/) | What only a downloadable image has: `provision.toml`'s example and service, the data partition's first boot, the state partition's links, SSH by key from `provision.toml`, the `NOTICE` template |
| [`roms-early/`](roms-early/nibos-roms-early.service) | An early ROMS mount, measured and not used |

## Build and install

```bash
orb -m rgbuild bash kernel/build.sh                      # first: the kernel's modules go in
orb -m rgbuild bash -c 'cd hw && PKGDEST="$PWD/out" BUILDDIR="$HOME/makepkg-build" makepkg -f --nodeps'
orb -m rgbuild sudo bash rootfs/make-arch.sh             # out/archroot.img
rootfs/install-arch.sh                                   # Mac, dev card in the reader
```

`make-arch.sh` takes `SIZE` (default 4G; the file system grows to fill the partition on first
boot) and `NETWORK_MODE=boot|deferred|demand`. The user, host name, time
zone and SSH key come from [`config.sh`](../config.sh). The full order, with the card and the
bootloader, is in [docs/building.md](../docs/building.md).

## What's in it

- Arch base with systemd, `sudo`, `vim`, `htop`, `evtest`, ALSA utilities.
- No kernel package: the kernel is in the bootloader's places (`kernel/deploy.sh`), and its
  modules are unpacked from `kernel/out/modules.tar.zst`.
- [`nibos-hw`](../hw/README.md), the hardware layer, and `linux-firmware-realtek`.
- iwd (Wi-Fi and DHCP), systemd-resolved with mDNS (the device is `$NIBOS_HOSTNAME.local`),
  OpenSSH (per connection, from a socket), BlueZ, PipeWire with WirePlumber, Mesa (Panfrost).
- One user, `$NIBOS_USER` (uid `$NIBOS_UID`, 1000; wheel, audio, video, input), and root.
  On a development card both are reachable by SSH key only, with no passwords anywhere: `sudo`
  and polkit let wheel through without one, and tty1 logs in by itself. An image's user is
  `nibos` on host `nibos`, with no keys, no password and SSH off until `provision.toml` sets
  them; root has no SSH login.
- `/mnt/roms`: the card's ROMS partition (NIBOS-DATA on an image), mounted on first use (an
  automount, so it isn't on the boot path).
- Three small units of its own ([`files/`](files/)): `nibos-wifi-import` copies Wi-Fi networks from
  `ROMS/nibos/wifi/` into iwd at every start; `nibos-bootlog` leaves each start's kernel
  log and journal on `ROMS/nibos/log/` (the last 10); `nibos-growfs` grows the file system
  to its partition at every start (off DSlot's path; a no-op once it fills it), so the small
  file system an update writes is grown too.
- No fstab entry for the root (the kernel mounts it; an entry would add a device wait, fsck and
  growfs to every boot), and the console font is the kernel's Terminus 16x32.

Using it (Wi-Fi, logging in, the network modes): [docs/using.md](../docs/using.md).
