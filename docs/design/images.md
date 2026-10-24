# Downloadable images

An image anyone can download and write to a card, with nothing in it that isn't ours to
redistribute: no stock partitions, no stock boot image, no credentials. Built by
`tools/make-image.sh` and checked without a device by `tools/test-image.sh`. Writing one and
setting it up: [install.md](../install.md).

## Two editions

| File | Edition |
|---|---|
| `nibos-<version>-rgds.img.xz` | **nibOS**, booting to a shell on tty1 |
| `nibos-<version>-rgds-dslot.img.xz` | **nibOS + DSlot**, ready to play: DSlot as the boot UI, DSperate (DS, GBA), inkdot (GB, GBC), nibcase with its libretro cores (SNES, NES, Mega Drive, PC Engine, Neo Geo Pocket, WonderSwan, Game Gear, Lynx, Game & Watch, PlayStation) and inktick (Game & Watch, a preview), Mesa without LLVM, the safe mode, read-ahead, PortMaster's launcher |

`<version>` is `git describe --tags --always --dirty` of this repository; `rgds` is the board's
host name (`BOARD_HOSTNAME`). Both editions have the splash (the boot picture at 1.5 s and the
goodbye at power-off).

```bash
tools/make-image.sh            # out/nibos-<version>-rgds.img.xz
tools/make-image.sh --dslot    # out/nibos-<version>-rgds-dslot.img.xz
```

It runs on a Mac or a Linux host, the work in the build machine, and writes the image's
sha256 to `out/SHA256SUMS`. The DSlot edition builds DSlot, DSperate, inkdot, nibcase and
inktick from their checkouts beside this repository (`DSLOT_SRC`, `DSPERATE_SRC`, `INKDOT_SRC`,
`NIBCASE_SRC`, `INKTICK_SRC`), and fetches [nibkit](https://github.com/abradburne/nibkit) at
`NIBKIT_TAG` (`NIBKIT_REPO` in [config.sh](../../config.sh)) for them. `SKIP_BUILD=1` reuses
`kernel/out` and `hw/out`; `HWPKG=FILE` picks the nibos-hw package; `SOURCE_URL` is where the
release's source is (in the NOTICE).

## The card

GPT. The bootloader area uses the same sectors as a dev card, which SPL is built for (SPL at 64
leaves the GPT in sectors 1-33 alone):

| Where | What | Size |
|---|---|---|
| Sectors 64-16383 | TPL (rkbin's DDR init) + SPL in Falcon mode | ≤ 8 MiB |
| Sector 20480 | The fallback U-Boot (A held at power-on), built with p3 as its root | ≤ 2 MiB |
| p1 `falcon` at sector 163840 (raw) | The Falcon FIT (TF-A, kernel, DTB, command line); room for slot B's and the boot control block ([ab-rollback.md](ab-rollback.md)) | 64 MiB |
| p2 `NIBOS` (FAT32) | `README.txt`, `provision.toml.example`, `NOTICE`, `packages.txt`, `licenses/` | 128 MiB |
| p3 `nibos-root-a` (ext4) | The root, with `/boot` (the fallback's `extlinux.conf`, kernel, DTB); grown to its partition on the first boot | 3 GiB |
| p4 `nibos-root-b` | Empty, for A/B updates | 3 GiB |
| p5 `nibos-state` (ext4) | What `provision.toml` sets: Wi-Fi networks, SSH keys, the data card, the first boot's marker; mounted at `/var/lib/nibos/state` | 512 MiB |
| p6 `NIBOS-DATA` (exFAT) | `Roms/`, `BIOS/`, `Saves/`, `APPS/` as DSlot expects, and `nibos/log/` (the boot logs); made on the first boot over the rest of the card, mounted at `/mnt/roms` | the rest |

- The image is about 7 GB before compression and much less compressed (p4 is empty and p6
  isn't in it). It fits a 16 GB card.
- SPL boots the Falcon FIT, and the fallback U-Boot boots `extlinux.conf` from p3 without
  scanning, so neither the stock U-Boot's boot partition nor `mkboot.py`'s stock template is
  needed.
- The root is read-write; the layout allows a read-only root with the state partition for what
  changes, the step that makes A/B updates safe.
- The image has no machine ID and no SSH host keys: systemd makes the machine ID and sshd its
  host keys on the first boot. iwd's networks and the authorised keys are on the state
  partition (`/var/lib/iwd` links there; sshd reads `AuthorizedKeysFile` there).
- A games card in TF2 works as on a dev card (`/mnt/ds`); `provision.toml`'s `data_card` can
  make it the data card instead of p6.

## First boot and provisioning

1. **The data partition.** `nibos-firstboot-data` (`/usr/lib/nibos/firstboot-data`, before
   anything uses `/mnt/roms`) runs while the marker on p5 isn't `done`: it moves the backup GPT
   to the card's end and adds p6 (`nibos-data`) over the rest of the card, then formats it
   exFAT, labelled `NIBOS-DATA`. The marker goes `factory`, `creating`, `formatting`, `done`,
   written before each step, so a power cut resumes where it stopped. It never formats a
   partition holding a file system it doesn't recognise as its own. The partition starts 8 MiB
   after the state partition, past the end of the image file (which runs 1 MiB past it, for its
   backup GPT), so writing a newer image over the card doesn't touch it: the first boot makes
   the partition again in the same place, finds the exFAT labelled `NIBOS-DATA` there, and keeps
   it, games and saves included. Either edition can be written over the other. Cards set up
   before nibOS 0.1.0-alpha1 had the partition right after the state partition, where an image
   overwrites its start, and are erased once. `tools/test-image.sh IMAGE [OTHER]` checks this
   by writing the image (and the other edition's) over a card with a save on it.
2. **`provision.toml`.** `nibos-provision` (at every boot, before the network, SSH and the
   console) reads `NIBOS/provision.toml` if it exists. It checks it strictly (unknown keys,
   empty or over-long values, newlines and NULs, Wi-Fi passphrase 8-63 bytes, SSH key types),
   applies it, stores the results on p5, overwrites the file and deletes it. A bad file is
   renamed `provision.rejected-N.toml` with `provision.error.txt` beside it (the reason, never
   the values), and the device boots without it. Every part is optional:

   | Field | What |
   |---|---|
   | `timezone` | e.g. `"Europe/London"` |
   | `data_card` | `auto` (a card in TF2 if there is one), `tf1`, `tf2` |
   | `[wifi]` `ssid`, `password` | A network; no password for an open one |
   | `[user]` `password` | The user `nibos`'s password, or none |
   | `[ssh]` `authorized_keys` | SSH public keys for `nibos` |

   The user is always `nibos` (DSlot's unit names it), the host name `nibos`
   (`nibos.local` through mDNS).
3. **Access.** No passwords and no keys in the image. Without a password, tty1 logs `nibos`
   in, and sudo and polkit ask for none; with one, a login prompt, and sudo and polkit ask for
   it. SSH is off until a key is given, keys only, no root login. The DSlot edition goes to
   the shelf either way.
4. **Radios.** Wi-Fi and Bluetooth start off on a fresh card until each is switched on (DSlot's
   switches, `nib wifi on`, `nib bluetooth on`) or, for Wi-Fi, `provision.toml` names a
   network; after that systemd-rfkill keeps each choice.

## The speaker amps' tuning file

The speaker amps (AW88166) don't play without a tuning file, and Awinic's isn't ours to ship.

- **nibOS's own file**, in both editions: written from scratch by
  `boards/rg-ds-plus/hw/aw88166-tuning.py` in the format the mainline driver's GPL parser reads
  (a magic number, sizes, CRC8 and byte sums; nothing signed), with register values chosen
  from Awinic's public datasheet: the DSP **bypassed** (no Awinic firmware), the lowest gain,
  the datasheet's default boost voltage and a lower boost current limit. Bypass loses the DSP's
  speaker protection (excursion, voice-coil temperature, anti-clip, EQ); the chip's own
  protections (over-temperature, over-current, UVLO, OVP) stay.
- **The official file, imported** from your own stock firmware: `sudo nib audio import-tuning`
  finds a stock card in TF2, or takes a partition or a stock image file (`.img`, `.img.gz`,
  `.img.xz`). It is kept in `/var/lib/nibos/firmware/` and becomes the active file;
  `nib audio tuning nibos|official` switches between the two (restart to apply).
  [engineering/audio.md](../engineering/audio.md).

## Licences in the image

`NOTICE`, on p2 and at `/usr/share/doc/nibos/NOTICE`, made from `rootfs/image/NOTICE.in` and
`tools/image-notice.py` from the image's package list:

- every component with its version or commit and licence: nibOS's own code GPL-3.0-or-later,
  nibkit MIT, the kernel and U-Boot GPL-2.0, and in the DSlot edition DSlot, the emulators and
  nibcase's cores;
- every package with its licence, the ones under custom licences named, with notes where they
  restrict passing the image on (Snes9x's non-commercial terms; FDK-AAC isn't in the images,
  [rootfs/no-fdk-aac](../../rootfs/no-fdk-aac/README.md));
- the non-free blobs named as such: rkbin's DDR init and BL31 at the pinned commit, Realtek's
  Wi-Fi and Bluetooth firmware;
- the written source offer, and that nibOS isn't affiliated with Arch Linux, Arch Linux ARM or
  Anbernic.

Licence texts are in `/usr/share/licenses/` (Arch's packages' own, plus rkbin's,
linux-firmware's Realtek licence and its WHENCE, the kernel's and U-Boot's GPL-2.0, the fonts'
OFL), the package list with licences in `packages.txt`, and the same `licenses/` on p2. A
release publishes the corresponding source beside each image (this repository at the tag, and
the source packages of every GPL/LGPL package in it). Upstream TF-A supports the RK3568 and
could replace rkbin's BL31 (one blob fewer) once suspend is tested with it; the DDR init has no
open replacement.

## Testing an image

Each image, written to a spare card: boot time, both screens, input, audio (headphones; the
speakers with each tuning), sleep and wake, the unplug tests
([battery.md](../engineering/battery.md)), Wi-Fi and SSH from `provision.toml`, a bad
`provision.toml`, the data partition after a power cut during the first boot, and on the DSlot
edition the shelf and a game from each emulator.
