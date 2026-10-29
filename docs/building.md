# Building nibOS

Everything needed to go from an empty Mac to a handheld booting nibOS in Falcon mode, in order,
for the board in `NIBOS_BOARD` (the RG DS+ is the one that works). Each piece's own README has
the details; this is the order and what each step needs. Commands run from the repository's
root, in one of three places:

- **Mac**: on the host.
- **VM**: in the OrbStack Arch Linux ARM machine (`orb -m rgbuild …`). It sees the Mac's files
  at the same paths and starts in the same directory.
- **Device**: over SSH, as `$DEVICE`, with passwordless sudo.

## 0. One-time setup

### Your settings

The scripts read which board, who and where from [`config.sh`](../config.sh): the board
(`NIBOS_BOARD`, default `rg-ds-plus`), the device's user (`NIBOS_USER`, default `nibos`), host
name (the board's, `rgds` for the RG DS+), time zone, the SSH key, the build machine's name,
and the source paths and pinned versions. Put your own values in `config.local.sh` (not in
git):

```bash
cp config.local.sh.example config.local.sh    # then edit it
```

### Folders

Two things live next to this repository:

```
parent/
  nibos/                                this repository
  RG-DS-PLUS-EN16GB-V1.0-20260915.IMG   the vendor's firmware image (STOCK_IMAGE; Anbernic's download, 15.5 GB)
  dslot/                                DSlot, the boot UI (DSLOT_SRC; only for rootfs/dslot)
```

### Mac tools

OrbStack (`orb`), `brew install gptfdisk shellcheck` (sgdisk; shellcheck for `make check`), and
an SSH key (`SSH_KEY`, default `~/.ssh/id_ed25519`). Its `.pub` is put on the device for
`$NIBOS_USER` and root.

### The build machine

An OrbStack Arch Linux ARM (arm64) machine, `rgbuild` (`BUILD_MACHINE`). The build is native
aarch64, with no cross compiler. Install the packages, then the kernel source at the pinned
commit:

```bash
orb create arch rgbuild
orb -m rgbuild sudo pacman -Syu --needed base-devel git bc pahole cpio flex bison openssl gnutls swig \
    python-setuptools python-pyelftools uboot-tools dtc arch-install-scripts e2fsprogs rust \
    busybox linux-firmware-realtek sdl2-compat shellcheck gptfdisk dosfstools mtools exfatprogs
orb -m rgbuild bash -c '. tools/lib.sh && git clone https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git "$KSRC" && git -C "$KSRC" checkout "$KERNEL_COMMIT"'
```

The kernel tree lives in the machine's own file system because the Mac volume is
case-insensitive. `kernel/build.sh` resets and cleans it on every run, so keep no work there.
`uboot/build.sh` clones U-Boot (`UBOOT_TAG`) to `~/u-boot` and Rockchip's rkbin
(`RKBIN_COMMIT`) to `~/rkbin` by itself; `kernel/deploy.sh` needs TF-A from rkbin too, so run
`uboot/build.sh` once before the first deploy.

### The files from the stock firmware

nibOS needs a few files from the vendor's image that are not ours to redistribute (for the
RG DS+: the stock boot partition as a template, the speaker amps' tuning file, and the stock
device tree for reference). The board's script takes them into `boards/<board>/stock/`, which
git ignores:

```bash
boards/rg-ds-plus/extract-stock.sh           # Mac or VM; runs in the VM
```

## 1. Build (VM)

```bash
orb -m rgbuild bash kernel/build.sh             # kernel/out/: Image.gz, DTB, modules.tar.zst
orb -m rgbuild bash -c 'cd hw && PKGDEST="$PWD/out" BUILDDIR="$HOME/makepkg-build" makepkg -f --nodeps'
orb -m rgbuild sudo bash rootfs/make-arch.sh    # rootfs/out/archroot.img (needs the two above)
orb -m rgbuild bash -c '. tools/lib.sh && cd kernel && python3 mkboot.py "$STOCK_DIR/boot.img" out/Image.gz "out/$BOARD_DTB.dtb" out/boot-arch.img --bootargs "root=PARTLABEL=$BOARD_ROOT_PARTLABEL rootfstype=ext4 rootwait rw console=tty1 fbcon=font:TER16x32 quiet loglevel=3" --description "nibos kernel for $BOARD_NAME"'
orb -m rgbuild bash uboot/build.sh              # uboot/out/u-boot-rockchip.bin (U-Boot proper)
orb -m rgbuild bash -c 'FALLBACK=1 bash uboot/build.sh'   # uboot/out/u-boot-fallback.itb (A held)
orb -m rgbuild bash -c 'FALCON=1 bash uboot/build.sh'     # uboot/out/idbloader-falcon.img
```

`make-arch.sh` takes `SIZE` (default 4G) and `NETWORK_MODE=boot|deferred|demand`. It runs
`rootfs/tune-boot.sh` on the new root, which holds the boot-time trimming
([boot-time.md](boot-time.md)).

## 2. The card (Mac; erases it)

```bash
kernel/make-dev-card.sh /dev/diskN kernel/out/boot-arch.img   # the stock image, then our boot image on the boot partition
rootfs/install-arch.sh                                        # the root partition over the free space, archroot.img onto it
```

Before the first boot, put a Wi-Fi network on the card as `ROMS/nibos/wifi/<SSID>.psk`
([using.md](using.md#wi-fi)). The device then boots stock SPL → stock U-Boot → our kernel from
the boot partition → Arch on the root partition, which grows to fill the card.
`kernel/flash-boot.sh IMAGE` rewrites the boot partition from the Mac, and
`kernel/flash-boot.sh --restore` puts the stock one back. The card layout is in
[architecture.md](architecture.md#the-sd-card).

## 3. Mainline U-Boot and Falcon (device over SSH, in this order)

```bash
kernel/deploy.sh                 # the kernel: /boot + extlinux, boot partition, falcon.itb, modules; restarts
uboot/install.sh                 # keeps the stock bootloader on ROMS/nibos/, U-Boot at sector 0x4000
uboot/install.sh --fallback      # the known-good U-Boot at 0x5000: what boots with A held at power-on
uboot/install-falcon.sh          # SPL in Falcon mode at sector 64: boots falcon.itb directly
```

From here on a kernel change is `kernel/build.sh`, then `kernel/deploy.sh`. **Deploy only with
`deploy.sh`**: it writes `falcon.itb`, and SPL boots whatever is on the Falcon partition. If
the card doesn't boot at all: [recovery.md](recovery.md).

## 4. On top (optional; device over SSH, in this order)

| Step | What |
|---|---|
| [rootfs/mesa/](../rootfs/mesa/README.md) | Mesa with only Panfrost and no LLVM: makepkg in the VM, `pacman -U` on the device. GL ~1 s faster cold. |
| `rootfs/dslot/install.sh` | The `dslot` package (built in the VM from `DSLOT_SRC`), the user it runs as, `nib boot-ui`. Games need emulators, which are separate packages. `sudo nib boot-ui on` on the device makes it the boot UI. |
| `rootfs/splash/install.sh` | The boot picture (init=) and the goodbye at power-off, with their pictures, in either edition. `rootfs/splash/boot.sh on\|off\|status` switches the boot picture; `deploy.sh` keeps the choice. |
| `rootfs/readahead/install.sh` | Read-ahead of DSlot's and Mesa's files at power-on (~0.3 s off the shelf). `install.sh HOST off` removes it. |

`hw/` changes after the card is made: copy `hw/out/nibos-hw-*.pkg.tar.xz` to the device and
`sudo pacman -U` it.

## 5. Measuring (not needed to build)

[bench/README.md](../bench/README.md): boot time from reset, the boot UI, the network modes, the
bootloader's own stages, with the reference results.

## 6. Saving and restoring a whole card

`tools/card-image.sh save [name]` (Mac) saves every byte of the card in the reader to
`../backups/<name>.img.zst`; `tools/card-image.sh restore FILE` writes one back.

## 7. Downloadable images

`tools/make-image.sh [--dslot]` (Mac or a Linux host; the work runs in the build machine)
builds the kernel, U-Boot, nibos-hw and the root and lays them out as
`out/nibos-<version>-rgds[-dslot].img.xz`, with nothing from the stock firmware in it.
`tools/test-image.sh` checks an image without a device. The DSlot edition builds DSlot and the
emulators from their checkouts beside this repository. DSlot, inkdot and nibcase take
[nibkit](https://github.com/abradburne/nibkit) at the tags their Cargo.lock pins; DSperate's
pause menu and inktick take it at `NIBKIT_TAG` (`NIBKIT_REPO`, `config.sh`), as do nibOS's own
crates. What an image contains and
how it sets itself up: [design/images.md](design/images.md).

## Dead ends: don't repeat

Tried and measured, or found not to work; the source says more. Add to it when something doesn't pay.

- Building from whatever `~/linux`, `~/u-boot` or rkbin is checked out: pinned (`config.sh`).
- `cargo build --locked || cargo build`: quietly newer crates; locked.
- cargo in the build machine missing edits made on the Mac a moment before: wait a second, or touch the file.
- Piping a binary out of the build machine through `orb ... cat | ssh`: arrives truncated; scp.
- tar keeping the Mac's uid 501: systemd refuses the path; `--no-same-owner`.
- sfdisk on a card in use, the new-partition lookup racing the kernel: fixed in the image scripts.
- diskutil failing under pipefail: scripts exiting silently.
- A device lock on the Mac (invisible to other machines) or in the device's /tmp (lost at a reboot): `tools/device-lock.sh` keeps it on the card.
- `deploy.sh` ending with `systemctl reboot` over SSH: exit 255; it restarts in the background.
