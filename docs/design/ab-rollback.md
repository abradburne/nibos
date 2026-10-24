# A/B slots and rollback

An update writes the slot that isn't running (its root file system and its Falcon FIT), then
restarts into it. If the new slot never reaches its boot UI, the device goes back to the old
slot by itself, after three tries, without a card reader and without anyone holding a button.
The A-held fallback stays as the last way back, untouched.

Inspecting and resetting the slots on a device: [recovery.md](../recovery.md#ab-slots).
Updates themselves are on the [roadmap](../roadmap.md#1-updates).

## What is implemented

Phase 1:

- SPL's slot choice (U-Boot patch `rg-ds-plus/0004`);
- `nibos-bootctl`, which reads and writes the boot control block (nibos-hw,
  `hw/daemons/src/ab.rs`);
- `nibos-boot-good.service`, which confirms a slot once its boot UI is up;
- the safe mode's **Go back to the previous version** on an unconfirmed slot.

This touches SPL, so a broken SPL build can need a card reader (below).

## Where things live

Everything is in the Falcon partition, which is nibOS's own in both layouts (p1 `falcon`,
64 MiB, on an image; p4 `recovery`, 128 MiB, on the dev card), both at sector 163840 where SPL
looks. A FIT is ~17 MB, so the partition has room for two and a control block:

| Offset in the Falcon partition | Sector | What |
|---|---|---|
| 0 | 163840 | Slot A's Falcon FIT: command line `root=PARTLABEL=nibos-root-a` |
| 32 MiB | 229376 | Slot B's Falcon FIT: the same, with `root=PARTLABEL=nibos-root-b` |
| 64 MiB - 1 KiB | 294910, 294911 | The boot control block, two copies |

- No new partition and no GPT parsing in SPL (it reads raw sectors), and each slot's command
  line stays inside its own FIT: SPL never edits a command line.
- `kernel/deploy.sh` and `boot.sh` write a FIT with `dd` of the file's size, so they don't
  reach the block at the end. They write the running slot's FIT (from `/proc/cmdline`'s root),
  not always A's.
- A dev card has no root B; it works as slot A only (no block, no change).

## The boot control block

One 512-byte sector, written as two copies in a row so a power cut mid-write leaves one good:

| Bytes | Field |
|---|---|
| 0-3 | Magic `NIBB` |
| 4 | Version (1) |
| 5 | Active slot: 0 = A, 1 = B |
| 6-7 | Per slot (A, B): tries left, 0-7 |
| 8-9 | Per slot: good (1 once its userspace said so) |
| 10-13 | Sequence number: every write is one higher; the copy with the higher valid one wins |
| 14-507 | Zero (room for later: a slot's version string, the last failure) |
| 508-511 | CRC-32 (zlib's, as U-Boot's `crc32(0, ...)`) of bytes 0-507 |

## What SPL does

In `board_spl_mmc_get_uboot_raw_sector`, where the A button is read:

```
if A is held:                          return the fallback U-Boot (0x5000)    # first
read both copies; take the valid one with the higher sequence
if none is valid:                      return slot A's FIT
s = active
if good[s]:                            return slot s's FIT                    # no write
if tries[s] > 0:  tries[s] -= 1; write; return slot s's FIT
o = the other slot
if good[o]:       active = o; write; return slot o's FIT                      # rolled back
                                       return the fallback U-Boot             # both failed
```

- A healthy boot reads two sectors and writes nothing: no wear and no time (the read is well
  under a millisecond next to the 17 MB FIT).
- SPL writes only while a slot is on trial. It writes the copy with the lower sequence first,
  then the other, so one copy is always whole.
- Holding A is checked before the block is read: a block, or block code, that goes wrong can
  always be stepped over.
- It needs `SPL_MMC_WRITE` (also used by `FALCON_DEBUG`) and `SPL_CRC32`.

## What userspace does

- **Good.** `nibos-boot-good.service` runs `nibos-bootctl confirm-when-up`: once the splash
  has seen both screens taken over (DSlot's first frame, or the console's login prompt in the
  base edition; it leaves `/run/nibos/screens-taken`), with at most 2 minutes' wait for a boot
  without the splash, and 30 s later DSlot is still running rather than in the safe mode, it
  sets `good[active] = 1` (and leaves `tries` alone). Nothing to do on a confirmed slot or a
  card with no block. `nibos-bootctl` writes both copies the way SPL does, the older first,
  each synced.
- **Trying a slot.** `nibos-bootctl try A|B` sets `active` to that slot, `tries = 3`,
  `good = 0`; `nibos-bootctl write-fit A|B FILE` puts a Falcon FIT in that slot's place. An
  updater writes the other slot's root and FIT, then tries it and restarts. If the new slot
  fails three times, SPL goes back to the old slot, which is still marked good.
- **The safe mode.** Landing in the safe mode on a slot that isn't confirmed good counts as a
  failed try: it offers **Go back to the previous version** (`nibos-bootctl rollback`), which
  makes the other slot active and restarts. On a good slot the safe mode stays as it is
  (DSlot crashing on a known-good system isn't the update's fault).

```
nibos-bootctl [status]          the active slot, each slot's tries left and whether it's good
nibos-bootctl unconfirmed       exit 0 if the running slot isn't confirmed good yet, else 1
nibos-bootctl init [--force]    a new block: slot A active and good (refused over a valid one)
nibos-bootctl good              the running slot is good
nibos-bootctl confirm-when-up   nibos-boot-good.service
nibos-bootctl try A|B           boot that slot next, with 3 tries, then back to the other
nibos-bootctl rollback          back to the other slot, if it's good (the safe mode)
nibos-bootctl write-fit A|B FILE    a Falcon FIT into that slot's place
```

## What counts as a failed try

A try is used up when SPL boots a slot that isn't good yet. It ends badly however the boot
doesn't reach "good":

- a kernel panic: the kernel restarts 10 s later (`panic=10`);
- a hang in userspace: the hardware watchdog's first stage panics the kernel about 22 s after
  systemd stops feeding it, and the panic restarts it (below);
- DSlot crashing into the safe mode (above);
- someone holding power to switch off: the next power-on is the next try;
- a hang before systemd (in the kernel, mounting the root, in userspace before PID 1 takes the
  watchdog over): SPL starts the hardware watchdog when it boots a slot on trial (U-Boot
  `rg-ds-plus/0005`), and the kernel feeds it until systemd opens it, for at most 60 s
  (`watchdog.open_timeout=60` on the command line). A good slot boots with it off.

### How a watchdog timeout restarts the board

The watchdog's own reset never brings the RG DS+ back. It is a CRU global reset, and after
one (first or second global reset alike) the board hangs before SPL: dark until switched off
by hand. Found on the device: a slot on trial whose init never started; then a good slot whose
watchdog was opened and left unfed. A diagnostic SPL logged no run between the reset and the
next power-on, the red LED it lit first thing stayed off, and turning the SD card off and on
in SPL changed nothing. Every normal restart goes through the RK817 PMIC instead (the RK817
is the `system-power-controller`, and its restart handler comes before PSCI's), and those
come back, as do panics.

So a watchdog timeout panics, and the panic restarts through the PMIC. The watchdog runs in
IRQ mode, two stages: the first raises the pre-timeout interrupt, which panics the kernel
(the panic pre-timeout governor, `nibos.config`), and only the second resets.

- SPL starts it in IRQ mode for a slot on trial, two stages of 11.2 s (U-Boot `rg-ds-plus/0005`);
  Linux's dw_wdt keeps the mode it finds and pings within one stage (kernel `generic/0006`).
- systemd asks for IRQ mode (`RuntimeWatchdogPreSec`, nibos-hw's `50-nibos-watchdog.conf`):
  two stages of about 11 s for its 20 s.
- Left over: a kernel so stuck that the interrupt isn't taken. The lockup detectors panic for
  most of those (`nibos.config`); one that defeats them too ends in the CRU reset and stays dark.
  U-Boot `rk3568/0005` makes that a first global reset (the pins reset too), as proposed
  upstream. The thermal shutdown (TSADC, `hw-tshut-mode` CRU) has the same weakness.

## What could need a card reader

| What goes wrong | Way back |
|---|---|
| The block parsing in SPL is wrong | Hold A (checked first). Then write a fresh block from Linux (`nibos-bootctl init --force`), or reinstall the old SPL |
| SPL itself doesn't run (a build broken before the hook) | Card reader: `uboot/restore-stock.sh` or `install-falcon.sh` from the Mac. The same risk as any SPL change |
| A wrong write lands somewhere else on the card | SPL only ever writes two fixed sectors, both checked against the partition's start (163840) at build time; `nibos-bootctl` checks the partition's name and start before writing |

## Boot-time cost

Two sector reads in SPL on every boot, and an fsync'd 1 KiB write once per boot on slots still
on trial. Kernel start stays at 0.65 s (below).

## Tested

On the dev card: kernel #51, SPL with `rg-ds-plus/0004`, nibos-hw 0.1.0-36; slot B's FITs made
by `mkfalcon.py` from the same kernel (the dev card has no root B: a working slot B mounts the
same root).

| Test | Result |
|---|---|
| No block on the card | Boots slot A as before (back in 15 s) |
| A block with A good | The block's hash unchanged after a restart: nothing written |
| Slot B broken (`root=PARTLABEL=nibos-nonexistent rootwait=3`: panic, restart 10 s later), `try B` | Three tries, then slot A by itself, 60 s after the restart; the block says A, B 0 tries |
| Slot B working, `try B` | Boots B (2, then 1 tries left), confirmed good ~35 s in |
| Slot B unconfirmed, DSlot crashed into the safe mode | "Go back to the previous version" listed; chosen: slot A, restarted |
| Boot time | Kernel start 0.65 s, as before |

Notes: `/proc/self/mounts` names the root `/dev/root`, so the root's partition is found by its
device number. Images build the same SPL; with no block on a fresh card it boots slot A, and
an updater writes the first block.

## Not yet

- **Phase 2's test on a device**: SPL arms the watchdog for a slot on trial, in IRQ mode (built,
  above). To test: a panic restarting through the PMIC (`echo c > /proc/sysrq-trigger`); a
  watchdog left unfed on a good slot restarting by itself; a slot on trial whose init never
  starts systemd (`init=/usr/bin/cat` in its FIT) restarting three times and handing back; a
  userspace hang (DSlot's unit `ExecStart=/bin/sleep infinity`, the splash never taken over)
  as a failed try.
- **Phase 3: the fallback follows the good slot.** Today holding A boots `/boot` on root A
  with the plain command line; U-Boot proper can read the block (`mmc read`) and pick the
  `extlinux.conf` on the good slot's root instead.
- **Phase 4: the updater** (one-file and network updates, `nib update status` with each slot's
  version and whether this boot is a rollback), with a read-only root to make a slot's contents
  trustworthy.
- A test build that resets after writing the first copy, to check a power cut during SPL's
  write.
