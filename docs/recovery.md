# Recovery

What to do when a nibOS device doesn't start, or starts wrong, from the least to the most
drastic. Everything here is on the one SD card, and nothing touches the partition table. 0-2
apply to an image and a development card alike; 3-4 are for a development card, made from the
stock image, whose stock partitions are never changed. The details are the RG DS+'s.

## 0. What recovers by itself

- **DSlot crashes**: systemd starts it again; after three failures in a minute, the safe mode
  ([rootfs/safe-mode](../rootfs/safe-mode/README.md)).
- **A hang**: systemd feeds the hardware watchdog; ~22 s after userspace stops, the watchdog
  panics the kernel, which restarts it 10 s later. A kernel panic, or a CPU stuck in the kernel,
  restarts it after 10 s ([how](design/ab-rollback.md#how-a-watchdog-timeout-restarts-the-board)).
- **A bad update**: an unconfirmed slot gets three tries, then SPL boots the other one (below).

## 1. Hold A while powering on

SPL then skips UHS and loads the fallback U-Boot from sector 0x5000 (the first mainline U-Boot
that ever booted, `FALLBACK=1 uboot/build.sh`), which boots the kernel in `/boot` through
`extlinux.conf` with the plain command line: no Falcon FIT, no splash, the console on the
screens. A kernel, splash or bootloader under test that hangs costs a restart, not a trip to a
card reader.

`/boot` and `extlinux.conf` are written by every `kernel/deploy.sh`, so the fallback boots the
same kernel as the normal path. If that kernel is what's broken, deploy a good one from the
fallback boot.

## A/B slots

Both slots' kernels (Falcon FITs) and the boot control block that picks one live in the Falcon
partition ([design/ab-rollback.md](design/ab-rollback.md)): p1 on an image, p4 on a development
card. From the device, as root:

```bash
nibos-bootctl status        # the active slot; each slot good or not, and its tries left
nibos-bootctl rollback      # back to the other slot, if it's good; restart to use it
nibos-bootctl try A|B       # boot that slot next, with three tries
nibos-bootctl init --force  # start again: slot A active and good
```

A card with no block boots slot A. Holding A at power-on skips the slots altogether (1).

## 2. Read the logs on the card

Every start leaves its kernel log and journal on the card's data partition, in
`nibos/log/boot-*.txt` (the last 10; `nibos-bootlog.service`): the ROMS volume on a development
card, NIBOS-DATA on an image. They are readable on any computer when the device can't be
reached, and written again at power-off and when DSlot fails. A start that hangs before the root file system is up
leaves nothing: try the fallback (1).

A test that "hangs" over SSH may only have lost Wi-Fi: check the log or `journalctl -b -1`
before assuming the worst.

For a bug report, `nib bug-report` writes a copy of this start's log with the private parts
taken out (Wi-Fi names and passphrases, MAC and IP addresses, the hostname) to
`nibos/bug-report-DATE.txt` on the same partition; `nib bug-report send` also posts it to
nibOS's bug tracker (`/etc/nibos/bug-report.conf`) and prints a code to quote. A tracker that
is down costs a minute and a message; the file is on the card either way.

## A card that wasn't written right

An image written wrongly (balenaEtcher 2.1.6 on macOS and a `.img.xz`, a card reader that
drops bytes, a card going bad) often still boots: the kernel and most of the system are whole,
and one program isn't. The usual sign is one emulator quitting the moment a game starts while
another runs. Once per write of the card, about a minute after the first start and at idle
priority, `nibos-root-verify` reads every file the image shipped under `/usr` and `/boot` and
checks it against the SHA-256 list the build wrote (`/usr/lib/nibos/root-files.sha256`). The
verdict is in `/var/lib/nibos/state/root-verified` (`result=ok` or `result=bad`, with the first
files that differ), and in the journal:

```bash
cat /var/lib/nibos/state/root-verified
journalctl -u nibos-root-verify
```

On a bad verdict DSlot shows "This card was not written correctly" on the shelf at every start,
and `nib update download` and `nib update apply` refuse: an update written from a damaged
system can't be trusted either. The fix is to write the image again
([install.md](install.md#writing-an-image)), with Raspberry Pi Imager or `dd`; NIBOS-DATA
(games, saves) is kept. A card that fails the check twice is probably failing itself.

## 3. Back to the stock bootloader (development card, Mac)

If the card doesn't boot even with A held, put it in a Mac and run:

```bash
uboot/restore-stock.sh
```

It writes back the stock bootloader that `uboot/install.sh` copied to
`ROMS/nibos/bootloader-stock.img` (with a checksum) before it first wrote anything. The stock
U-Boot then boots whatever kernel is in the boot partition: nibOS's, written there by
`kernel/deploy.sh` in the stock layout, or the stock one (4).

## 4. Back to the stock OS (development card)

With the stock bootloader back (3), `kernel/flash-boot.sh --restore` (Mac) writes the stock
boot partition back from `ROMS/nibos/boot-backup.img`. The stock recovery partition, which the
Falcon FIT replaced, is kept as `ROMS/nibos/recovery-stock.img` (write it back to partition 4
with `dd` if you need it). The card is then a stock card with an extra partition at the end.

## 5. A whole card from a backup

`tools/card-image.sh save [name]` (Mac) keeps every byte of a card in `../backups/` (about
25 minutes, compressed); `tools/card-image.sh restore FILE` writes one back. Take one before
anything risky. An image card can also simply be written again with an image: NIBOS-DATA
(games, saves) is kept on a card set up by nibOS 0.1.0-alpha1 or later
([design/images.md](design/images.md#first-boot-and-provisioning)).

## The copies the scripts keep (development card)

| On the card | Kept by | Before |
|---|---|---|
| `ROMS/nibos/bootloader-stock.img` | `uboot/install.sh`, once | The first mainline U-Boot install |
| `ROMS/nibos/recovery-stock.img` | `kernel/deploy.sh` or `uboot/install-falcon.sh`, once | The first Falcon FIT |
| `ROMS/nibos/boot-backup.img` | `kernel/flash-boot.sh`, once | The first boot image from a Mac |

Each has a `.sha256` next to it, which the restore scripts check. None is ever overwritten.
