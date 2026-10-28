# Installing nibOS on an RG DS+

Two ways onto an Anbernic RG DS+:

- **An image** (most people): write a downloadable image to a card, fill in `provision.toml`,
  boot. Nothing from the stock firmware is needed. Below, "Writing an image".
- **A development card, from source**: built on a Mac, the card made from Anbernic's own
  firmware image, with your SSH key baked in. For changing nibOS; below, "A development card".
  [building.md](building.md) is the same path with more detail on each step.

Both give one of two setups:

- **nibOS**: the operating system alone. The device boots in about 5 s to a shell on the top
  screen (tty1), with SSH and Wi-Fi.
- **nibOS + DSlot**: DSlot as the boot UI (the game shelf on both screens), the emulators, the
  power-on splash and read-ahead.

## Writing an image

1. **Get the image**: `nibos-<version>-rgds.img.xz` (nibOS) or
   `nibos-<version>-rgds-dslot.img.xz` (nibOS + DSlot), and `SHA256SUMS`, from a release, or
   build one with `tools/make-image.sh [--dslot]`. What each contains:
   [design/images.md](design/images.md). *Check:* `sha256sum -c SHA256SUMS --ignore-missing`
   (`shasum -a 256 -c` on a Mac).
2. **Write it** to a microSD card of 16 GB or more (it erases the card): with Raspberry Pi
   Imager (Use custom, the `.img.xz` as it is) or `xzcat <image> | sudo dd of=<card> bs=4M`.
   balenaEtcher 2.1.6 (on macOS) writes a `.img.xz` wrongly, and the card doesn't start;
   unpack it first and give it the `.img`. If the computer calls the card unreadable or
   offers to format it, say no (Ignore or Eject, never Initialise). Writing a newer image over a card that was set up by nibOS 0.1.0-alpha1 or later keeps its
   NIBOS-DATA partition, games and saves included, and from 0.1.0-alpha6 your settings, play
   time and favourites too (kept in NIBOS-DATA's `nibos/settings/`; sign in to
   RetroAchievements again); a card set up by anything older is erased once. Copying your saves
   off first is still wise.
3. **Set it up.** On the card's `NIBOS` partition, copy `provision.toml.example` to
   `provision.toml` and fill in what you want: a Wi-Fi network, an SSH public key, a password
   (or none), the time zone. Every part is optional.
4. **Boot.** The card in the slot marked TF1, power on. The first start makes the data
   partition (NIBOS-DATA) over the rest of the card; `provision.toml` is applied and erased
   (a file with a mistake is renamed `provision.rejected-N.toml`, with `provision.error.txt`
   saying why, and nothing in it is applied). About a minute after the first start, with the
   shelf already up, nibOS reads the card once and checks that the system is exactly what the
   image shipped. A card an imaging tool wrote wrongly often boots and then fails in odd ways (one
   emulator quits at once while another runs); DSlot then says so on the shelf, and the way out
   is to write the image again with Raspberry Pi Imager or `dd` ([recovery.md](recovery.md#a-card-that-wasnt-written-right)).

Then:

- **Logging in.** The user is `nibos`. Without a password, tty1 logs in by itself; SSH is on
  only once `provision.toml` gave a key: `ssh nibos@nibos.local`.
- **Games** go on NIBOS-DATA in `Roms/NDS`, `Roms/GBA`, `Roms/GB`, `Roms/GBC` and the other
  systems' folders (or on a second card in TF2, exFAT or FAT32).
- **Wi-Fi and Bluetooth** start off on a fresh card unless `provision.toml` named a network:
  DSlot's switches, `nib wifi on` or `nib bluetooth on` turn them on, and the choice is kept.
- **The speakers** use nibOS's own tuning for the amps. `sudo nib audio import-tuning` imports
  the official one from your own stock card or image ([using.md](using.md)).

## A development card

### What this setup is

- **A Mac is needed.** The build runs in an OrbStack Arch Linux ARM machine (native aarch64),
  and the scripts that write the card use macOS's `diskutil`. A Linux computer could do the
  same work, but the card scripts haven't been ported (`ROMS_VOLUME` in `config.sh` is the
  first step).
- **You need the stock firmware image** (`RG-DS-PLUS-EN16GB-V1.0-20260915.IMG`, 15.5 GB,
  Anbernic's download). The card starts as a stock card: its bootloader, partitions and the
  stock OS stay on it, and nibOS is added after them. A few files come only from that image
  and can't be shipped with nibOS (`extract-stock.sh` takes them from your copy).
- **It is a development image.** Your SSH key is authorised for the user and for root; there
  are **no passwords** (the user logs in on tty1 by itself; `sudo` and polkit ask for nothing);
  SSH is on; the network comes up at boot. Fine on your own device on your own network; set a
  password with `passwd` before taking it anywhere that matters.
- **Nothing is updated automatically.** Updates are rebuilding and redeploying (below).
- **The card needs more than 16 GB.** The stock image fills 16 GB; nibOS's root partition takes
  the rest. A 32 GB card or larger.
- **Time:** about an hour the first time, most of it the kernel build and writing the card.

Commands run from this repository's root in one of three places, as in building.md: **Mac**
(the host), **VM** (`orb -m rgbuild …`, the build machine) or **device** (over SSH).

### Before you start

1. **Back up the card you will use**, if it has anything on it. With it in the Mac's reader:
   `tools/card-image.sh save stock-before-nibos` (Mac) keeps every byte in `../backups/`.
   *Check:* `ls -lh ../backups/`.
2. **The folders.** Next to this repository, the stock image, and for nibOS + DSlot the other
   projects:

   ```
   parent/
     nibos/                                 this repository
     RG-DS-PLUS-EN16GB-V1.0-20260915.IMG    the stock firmware image
     dslot/                                 DSlot (DSLOT_SRC)
     DSperate/                              the DS and GBA emulator
     inkdot/                                the GB and GBC emulator
   ```

3. **Your settings.** `cp config.local.sh.example config.local.sh` and set what differs from
   the defaults: `DEVICE` (default `$NIBOS_USER@$NIBOS_HOSTNAME.local`; the user is `nibos`
   and the host name the board's, `rgds` on the RG DS+) and, if yours isn't `~/.ssh/id_ed25519`, `SSH_KEY`.
   *Check:* `bash -c '. tools/lib.sh; echo $DEVICE $SSH_KEY'`.
4. **The Mac's tools and the build machine:** [building.md, "One-time setup"](building.md#0-one-time-setup)
   (OrbStack, `brew install gptfdisk shellcheck`, the `rgbuild` machine with its packages, the
   kernel source at the pinned commit). *Check:* `orb -m rgbuild uname -m` says `aarch64`, and
   `orb -m rgbuild bash -c '. tools/lib.sh; git -C "$KSRC" rev-parse HEAD'` is `KERNEL_COMMIT`.
5. **The files from the stock image** (Mac; reads the image only):

   ```bash
   boards/rg-ds-plus/extract-stock.sh
   ```

   *Check:* `ls boards/rg-ds-plus/stock/` lists `boot.img`, `aw88166_acf.bin` (the speaker
   amps' official tuning file) and `stock.dtb`.

### A1. Build (VM)

```bash
orb -m rgbuild bash kernel/build.sh
orb -m rgbuild bash -c 'cd hw && PKGDEST="$PWD/out" BUILDDIR="$HOME/makepkg-build" makepkg -f --nodeps'
orb -m rgbuild sudo bash rootfs/make-arch.sh
orb -m rgbuild bash -c '. tools/lib.sh && cd kernel && python3 mkboot.py "$STOCK_DIR/boot.img" out/Image.gz "out/$BOARD_DTB.dtb" out/boot-arch.img --bootargs "root=PARTLABEL=$BOARD_ROOT_PARTLABEL rootfstype=ext4 rootwait rw console=tty1 fbcon=font:TER16x32 quiet loglevel=3" --description "nibos kernel for $BOARD_NAME"'
orb -m rgbuild bash uboot/build.sh
orb -m rgbuild bash -c 'FALLBACK=1 bash uboot/build.sh'
orb -m rgbuild bash -c 'FALCON=1 bash uboot/build.sh'
```

*Check:* `kernel/out/` has `Image.gz`, `rk3568-anbernic-rg-ds-plus.dtb`, `modules.tar.zst` and
`boot-arch.img`; `hw/out/` a `nibos-hw-*.pkg.tar.xz`; `rootfs/out/archroot.img`;
`uboot/out/` `u-boot-rockchip.bin`, `u-boot-fallback.itb` and `idbloader-falcon.img`.
`make-arch.sh` puts the amps' official tuning file in the image from your `stock/` (no package
carries it); it says so if the file is missing.

### A2. The card (Mac; **erases the card**)

With the card in the Mac's reader:

```bash
kernel/make-dev-card.sh /dev/diskN kernel/out/boot-arch.img    # diskutil list shows N; asks you to type it back
rootfs/install-arch.sh
```

The first writes the stock image to the card (about 20 minutes) and nibOS's boot image into its
boot partition; the second adds the root partition over the free space and writes nibOS onto
it. Both ask for your password. Then put your Wi-Fi network on the card's ROMS volume as
`ROMS/nibos/wifi/<network>.psk` ([using.md](using.md#wi-fi)) and eject it.

*Check:* the card in the DS+, power on. The stock bootloader starts nibOS's kernel: the top
screen shows the shell, logged in (about 10 s this first time; the root file system grows to
fill the card). From the Mac, `ssh $DEVICE` gets in (`$NIBOS_HOSTNAME.local` through mDNS, once
Wi-Fi is up). `ssh $DEVICE nib battery` answers.

### A3. Mainline U-Boot and Falcon mode (device, over SSH, in this order)

These write the bootloader at the start of the card, so a mistake here can stop the card
booting: the way back is [recovery.md](recovery.md), and each script keeps the stock
bootloader's copy on the ROMS volume before it writes.

```bash
kernel/deploy.sh                 # the kernel everywhere the bootloaders read it; restarts
uboot/install.sh                 # keeps the stock bootloader on ROMS/nibos/, mainline U-Boot after it
uboot/install.sh --fallback      # the known-good U-Boot that boots with A held at power-on
uboot/install-falcon.sh          # SPL boots the kernel directly (Falcon mode); restarts
```

*Check:* after each restart `ssh $DEVICE uname -v` answers. After the last, power-on to the
shell takes about 5 s. Hold **A** while powering on: the fallback boots the same kernel with
the console on the screens (slower, no splash). That is nibOS done.

### B. nibOS + DSlot

Do A first. Then, in this order:

#### B1. Mesa without LLVM (VM, then device)

```bash
orb -m rgbuild bash -c 'mkdir -p ~/mesa-build/nibos && cp rootfs/mesa/PKGBUILD ~/mesa-build/nibos/ && cd ~/mesa-build/nibos && PKGDEST="$OLDPWD/rootfs/mesa/out" makepkg -s'
scp rootfs/mesa/out/mesa-nibos-*.pkg.tar.xz $DEVICE:/tmp/ && ssh $DEVICE 'sudo pacman -U /tmp/mesa-nibos-*.pkg.tar.xz'
```

Answer `y` to replacing Arch's `mesa`. *Check:* `ssh $DEVICE pacman -Q mesa-nibos`. (`makepkg`
refuses to build a package that is already in `out/`: add `-f` to build it again.)

#### B2. DSlot, the boot UI

```bash
rootfs/dslot/install.sh               # builds the dslot package from ../dslot in the VM, installs it
ssh $DEVICE sudo nib boot-ui on       # DSlot at power-on instead of the shell
```

*Check:* `ssh $DEVICE sudo reboot`; the shelf comes up on both screens. Select + Start leaves
it for the shell.

#### B3. The emulators (VM, then device)

DSperate (DS and GBA), from your checkout. With `_nibkit=` it gets the pause menu DSlot and the
other emulators share, from [nibkit](https://github.com/abradburne/nibkit) at `NIBKIT_TAG`
(`config.sh`); without it, DSperate's own. `_nibkit` names a `nibkit-menu-c` directory with a
`nibkit` directory (with its `LICENSE`) beside it, the layout `tools/make-image.sh` makes from
`NIBKIT_REPO` at `NIBKIT_TAG`:

```bash
orb -m rgbuild bash -c 'cd ../DSperate/packaging/arch && _src="$(cd ../.. && pwd)" _nibkit=/path/to/nibkit-menu-c PKGDEST="$PWD/out" BUILDDIR="$HOME/makepkg-build" makepkg -f --nodeps'
```

`makepkg` clones the checkout into its source directory; one left by an older DSperate build
may lack your checkout's commit ("unable to read tree"): give it a fresh `BUILDDIR` and
`SRCDEST`.

inkdot (GB and GBC), which fetches nibkit at the tag its `Cargo.toml` names:

```bash
orb -m rgbuild bash -c 'cd ../inkdot/packaging/nibos && PKGDEST="$PWD/out" BUILDDIR="$HOME/makepkg-build" makepkg -f --nodeps'
```

Then install both:

```bash
scp ../DSperate/packaging/arch/out/dsperate-*.pkg.tar.* ../inkdot/packaging/nibos/out/inkdot-*.pkg.tar.* $DEVICE:/tmp/
ssh $DEVICE 'sudo pacman -U /tmp/dsperate-*.pkg.tar.* /tmp/inkdot-*.pkg.tar.*'
```

*Check:* `ssh $DEVICE ls /usr/share/dslot/emulators/` lists `dsperate.toml` and `inkdot.toml`.
Put games on the card's ROMS volume in `Roms/NDS`, `Roms/GBA`, `Roms/GB`, `Roms/GBC` (or on a
second card in TF2, exFAT or FAT32); DSlot shows them, and A starts one.

#### B4. The splash and read-ahead

```bash
rootfs/splash/install.sh && rootfs/splash/boot.sh on     # the logo from 1.5 s, goodbye at power-off
rootfs/readahead/install.sh                              # DSlot's and Mesa's files read ahead
```

*Check:* restart: the logo appears early, then the shelf, about 5 s from power-on.

### Keeping it up to date

A change is built and deployed the way it was installed: the kernel with `kernel/build.sh` then
`kernel/deploy.sh`; a package by rebuilding it and `sudo pacman -U` on the device. Upgrading
`nibos-hw` from 0.1.0-22 or older keeps the amps' official tuning file (older packages carried
it); on a device that lacks it, `hw/install-firmware.sh` puts your copy there.

This path was checked on a Mac with OrbStack and an RG DS+ already running nibOS: every build
step in A1 and B3 as written; on the device, the `nibos-hw` 0.1.0-22 → 0.1.0-23 upgrade (the
tuning file kept, unchanged, owned by no package; the speakers present after a restart),
`hw/install-firmware.sh`, and B3's installs. A2 and A3 are the commands every development card
is made with.

### Back to stock

From the least to the most drastic ([recovery.md](recovery.md) has each in full):

1. **nibOS won't start:** hold A at power-on (the fallback), or read
   `ROMS/nibos/log/boot-*.txt` on a computer.
2. **Undo Falcon and mainline U-Boot** (Mac, card in the reader): `uboot/restore-stock.sh`
   writes the stock bootloader back from the copy on the card. The stock bootloader then
   starts nibOS from the boot partition, as after A2.
3. **The stock OS back** (Mac, after 2): `kernel/flash-boot.sh --restore` writes the stock boot
   partition back. The card is a stock card again, with nibOS's root partition left unused at
   the end (delete it with any partition tool, or leave it).
4. **The card exactly as it was:** `tools/card-image.sh restore ../backups/stock-before-nibos.img.zst`
   (Mac), or write Anbernic's image to it again.

## Not yet

The [roadmap](roadmap.md) has each with what "done" means:

- **updates for a development card**: a kernel package and a signed package repository, instead
  of rebuilding and redeploying (an image card updates itself: `nib update`,
  [design/updates.md](design/updates.md); [roadmap 1](roadmap.md#1-updates));
- **published packages** of DSlot and the emulators for a development card, instead of
  building each from a checkout;
- **card tools that don't need a Mac.**
