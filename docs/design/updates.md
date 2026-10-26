# System updates

Status: **being built.** A/B phase 2 (SPL arms the watchdog for a slot on trial, so a hang
before systemd counts as a failed try too) is built and is tested on a device before updates go
further than a development card. How a nibOS device updates itself: a whole system slot,
written to the slot that isn't running, tried once, confirmed or rolled back by the A/B slots
that are already there ([ab-rollback.md](ab-rollback.md)).

## In short

- **One update for everything**: a release is a whole slot, the root file system and its
  Falcon FIT, for one board and one edition (base or DSlot). Packages are never installed on the
  running root to update it.
- **Written to the inactive slot** while the device is in use, checked against a signed
  manifest as it is written and read back after, then **tried**: `nibos-bootctl try` with three
  tries. The boot UI's 30 s confirmation (`nibos-boot-good`) makes it good; three failed tries,
  or landing in the safe mode, bring the device back to the slot it came from.
- **The running slot is never written.** A power cut, a lost connection or a bad file at any
  point leaves the device booting what it booted before.
- **Signed** with Ed25519 (minisign's format); the public key is in the image. An unsigned,
  tampered, wrong-board or wrong-edition release is refused before anything is written.
- **One backend**, `nibos-update`, in nibos-hw: `nib update` on the command line, DSlot's
  System → Update page, the same program behind both.

## What a device has to work with

| | |
|---|---|
| Slots | Root A `p3 nibos-root-a`, root B `p4 nibos-root-b`, 3 GiB each; FIT A at the Falcon partition's start, FIT B 32 MiB in; the boot control block at its end ([ab-rollback.md](ab-rollback.md)) |
| Already built | SPL's slot choice and tries, `nibos-bootctl try/good/rollback/write-fit`, `nibos-boot-good`, the safe mode's "Go back to the previous version" |
| Not yet | the A-held fallback booting the good slot's `/boot` (phase 3) |
| Kept outside the slots | NIBOS-DATA (games, saves), the state partition `p5` (provisioning's results, Wi-Fi networks, SSH keys) |

Updates are for image cards. A development card (no root B) keeps `kernel/deploy.sh` and the
package installs.

## The release

### Files

Two channels: **stable** (what devices follow unless told otherwise) and **beta** (alpha and
beta builds). One GitHub release per version, on the nibOS repository, holding for each
edition:

| File | What |
|---|---|
| `nibos-<version>-<board>[-dslot].manifest.json` | What the release is and every byte of it (below) |
| `….manifest.json.minisig` | Its minisign signature (Ed25519) |
| `nibos-<version>-<board>[-dslot].slot` | The payload: the root file system and both FITs, as independently compressed chunks, back to back |
| `nibos-<version>-<board>[-dslot].img.xz` | The image for writing a card, as today |

Releases may also be read from a file: a URL given to `nib update`, or a manifest and payload on
NIBOS-DATA (`nibos/updates/`), for testing and for devices without Wi-Fi.

### The manifest

```json
{
  "format": 1,
  "version": "0.1.0-alpha2",
  "build": "78a2cf6",
  "board": "rg-ds-plus",
  "edition": "dslot",
  "channel": "beta",
  "min_from": "0.1.0-alpha1",
  "notes_url": "https://github.com/…/releases/tag/v0.1.0-alpha2",
  "notes": "What's new, a few lines, shown on the device",
  "components": { "linux": "7.3-rc7 0fcb05b", "u-boot": "v2026.10", "nibos-hw": "0.1.0-44", "dslot": "557fad9" },
  "payload": { "file": "nibos-0.1.0-alpha2-rgds-dslot.slot", "size": 262144000, "sha256": "…" },
  "root": { "size": 1918935040, "sha256": "…", "chunk": 16777216,
            "chunks": [ { "offset": 0, "size": 9123456, "sha256": "…", "raw_sha256": "…" }, … ] },
  "fit": { "a": { "offset": …, "size": …, "sha256": "…" }, "b": { "offset": …, "size": …, "sha256": "…" } },
  "migrate": 1
}
```

- **The root** is the root file system image (ext4, the size of its contents), cut into 16 MiB
  chunks, each compressed on its own (zstd). Each chunk has its compressed hash (checked as it
  arrives) and its raw hash (checked after it is written, by reading the slot back). Because
  chunks are independent, an interrupted download or write resumes at the first chunk that
  doesn't verify, with HTTP range requests into the one payload file.
- **Both FITs**: slot A's (`root=PARTLABEL=nibos-root-a`) and slot B's, built from the same
  kernel and DTB. The device writes the one for the slot it fills. The FIT is the only thing
  the slot letter changes.
- **`min_from`**: the oldest version that can take this update directly (a format change in
  the state partition, for example). Older devices are told to update to an intermediate
  release first, or to write the image.
- **`migrate`**: the state format's number; see "Migrating state".

### Signing

- [minisign](https://jedisct1.github.io/minisign/) signatures (Ed25519), verified on the device
  by `minisign-verify` (MIT, no dependencies). The manifest is signed; it carries the payload's
  and every chunk's hash, so nothing else needs its own signature.
- The public key is in nibos-hw (`/usr/lib/nibos/update.pub`), so it is in every slot. A new key
  comes as a release signed with the current one that installs the new public key; old keys
  are then dropped.
- The secret key is never in the repository or on a build machine's disk unencrypted: a
  password-protected minisign key, kept in a password manager with an offline backup, given to
  `make-release.sh` (its path and passphrase) only when a release is signed.

## On the device

### The backend: `nibos-update`

A root program in nibos-hw, run one step at a time as `nibos-update@<step>.service` (`check`,
`download`, `apply`, `rollback`, `ack`, `channel-stable`, `channel-beta`), which the wheel group
may start and stop without a password (polkit, `50-nibos-update.rules`). Each step writes
`/run/nibos/update/status.json`: the phase (`idle`, `checking`, `available`, `downloading` with
chunks done and the total, `paused`, `finishing`, `ready`, `restarting`, `trying`, `error`,
`current`), the step that wrote it (`step`), the release on offer with its notes and size, the
version in the other slot and whether going back to it is possible (`previous`, `rollback_ok`),
and `updated_to` or `failed` after an update. `nib update` and DSlot start the steps and read that file. The steps run at low
priority (`Nice=19`, `IOSchedulingClass=idle`, `CPUWeight=1`), so a game keeps its frame time
while a download writes, and stopping `nibos-update@download` pauses it.

With `error` and `current` comes `code`, which stays the same from release to release, and
`detail`, a sentence for a person. A front end words the codes it knows its own way, and
falls back to `detail` for one it doesn't. What curl itself said goes to the journal only
(`journalctl -u 'nibos-update@*'`), never into the status.

| `code` | Phase | Meaning |
|---|---|---|
| `none_published` | `current` | No release for this device on its channel, or the releases aren't public (the repository answers 404): "no updates yet", not a failure |
| `up_to_date` | `current` | The newest release is the one running |
| `older` | `current` | The release on offer is older than the one running |
| `needs_intermediate` | `current` | The release needs a newer version first (`min_from`) |
| `not_for_device` | `current` | The release is for another board or edition |
| `no_network` | `error` | No connection: no DNS, refused, timed out, dropped |
| `tls_error` | `error` | A secure connection failed, often a clock that isn't set yet |
| `not_found` | `error` | Nothing at the address (HTTP 404/410): a release file that went away |
| `rate_limited` | `error` | GitHub's rate limit (HTTP 429, or 403 from its API): try later |
| `server_error` | `error` | HTTP 5xx: try later |
| `http_error`, `network_error` | `error` | Another HTTP status, another curl failure |
| `bad_signature` | `error` | The manifest isn't signed by nibOS's key |
| `bad_manifest` | `error` | The manifest can't be read |
| `damaged` | `error` | A chunk or FIT doesn't match the manifest (downloading again fixes it) |
| `card_error` | `error` | A chunk read back from the card differs: the card may be failing |
| `low_battery` | `error` | Under 30% and unplugged |
| `not_confirmed` | `error` | The running version isn't confirmed good yet |
| `no_slot` | `error` | A card without slot B (a development card) |
| `no_space` | `error` | The release's root doesn't fit the slot |
| `nothing_pending`, `not_written` | `error` | A step out of order: download before a check, apply before the download is done |
| `no_rollback` | `error` | Going back asked for, but the other slot isn't good |
| `no_update_source`, `no_update_key` | `error` | An image built without an update repository or key |
| `io_error` | `error` | Anything else on the device (writing, mounting); `detail` says what |

1. **Check**: fetch the manifest and its signature for this board and edition (from
   `/usr/lib/nibos/release`, below), verify the signature, compare versions: newer, and
   `min_from` satisfied. Reports the version, the notes and the size. A same-or-older version is
   refused unless asked for explicitly (`--allow-downgrade`).
2. **Preconditions**, checked before writing and again before applying: the battery at 30% or
   more, or the charger in; the running slot confirmed good (no update on top of an update
   still on trial); the inactive slot's partition exists and is the slot's size; the release's
   board and edition are this device's. `nib update download` and `apply` also refuse on a card
   whose root doesn't match the image it was written from (`nibos-root-verify`,
   [images.md](images.md#first-boot-and-provisioning)): write the image again first.
3. **Download and write**, chunk by chunk: fetch (curl with range requests), check the
   compressed hash, decompress, write at the chunk's offset in the inactive root partition
   (`O_DIRECT`), sync, read back, check the raw hash. The verified chunks are listed in
   `/var/lib/nibos/state/update/progress` (on the state partition), so a restart, a sleep, a
   lost network or a power cut resumes there. Sleep pauses it (systemd's sleep hook), waking
   resumes it.
4. **Finish the slot**: write the slot's FIT with `nibos-bootctl write-fit` and read it back;
   grow nothing (the new root grows to its partition on its first boot, as a new image does);
   copy the state that lives on the root across (below).
5. **Ready**: "Restart to update". Applying is `nibos-bootctl try <other>` and a restart. Until
   then the slot is written but not bootable: the block still says the running slot.

### After the restart

- The new slot boots on trial (3 tries). `nibos-boot-good` confirms it 30 s after the boot UI
  is up, as for any boot.
- The first boot of a new version runs the migration hooks (below), then
  `nibos-update` notices the version changed (it keeps the last version it saw on the state
  partition) and DSlot shows "Updated to X" once.
- A failed try, or the safe mode, brings back the old slot, which is still marked good; the
  device then says the update didn't take ("Update to X failed; back on Y").

### Going back by hand

`nib update rollback`, and DSlot's "Go back to the previous version": `nibos-bootctl rollback`
(the other slot, if it is good) and a restart. The safe mode's entry is the same call.

### Migrating state

Most of what a device keeps is already outside the slots (NIBOS-DATA, the state partition).
What a running root keeps itself is copied into the new root before it is tried, from a list
in nibos-hw (`/usr/lib/nibos/update-keep`):

- `/etc/machine-id`, `/etc/ssh/ssh_host_*` (the device's identity);
- `/home/<user>` (DSlot's and the emulators' settings);
- `/var/lib/nibos` (brightness, lid, battery protect, an imported speaker tuning);
- `/var/lib/systemd/rfkill` (the radios as they were left), `/var/lib/bluetooth` (pairings).

And the user's password: the `nibos` line's hash in `/etc/shadow` (from provision.toml's `[user]`
or `passwd`), copied into the new root's file, whose other accounts stay the release's own. A
release whose updater didn't do that (0.1.0-alpha1) is made good on the new version's first
boot: `nibos-update booted` takes the hash from the previous slot, still there until the next
update.

The release's `migrate` number says which state format the new slot expects. Scripts in the new
slot's `/usr/lib/nibos/migrate.d/` run once on its first boot, before the boot UI, given the old
format's number. Later, moving what's on the list to the state partition would make the copy
unnecessary.

### `nib update`

```
nib update                     # what runs, and an update checked, downloading or ready
nib update check [FILE|URL]    # the newest release on the channel, or the manifest given
nib update download            # write it to the inactive slot (in the background; resumable)
nib update pause               # stop the download; `download` resumes it
nib update apply               # restart into it
nib update rollback            # back to the other slot, if it is good
nib update ack                 # forget "updated to" / "failed" once shown
nib update channel stable|beta
nib update --allow-downgrade check ...
```

### DSlot

System → Update: Check for updates → What's new (the notes) → Download (in the background,
with progress; the shelf stays usable) → Restart to update. After the restart, "Updated to X";
"Go back to the previous version" beside it while the old slot is still good. DSlot starts
`nibos-update@<step>.service` and reads `/run/nibos/update/status.json`; nothing in DSlot writes
a partition.

### What a release identifies itself by

Every image and slot gets `/usr/lib/nibos/release` (written by `tools/make-image.sh`):

```
NIBOS_VERSION=0.1.0-alpha2
NIBOS_BUILD=78a2cf6
NIBOS_BOARD=rg-ds-plus
NIBOS_EDITION=dslot
NIBOS_CHANNEL=stable
```

## Making a release: `tools/make-release.sh`

Builds both editions (`tools/make-image.sh`, at a clean tagged commit) and makes, for each:

- the payload: the root image cut into 16 MiB zstd chunks, and FIT A and FIT B (the second made
  by `uboot/mkfalcon.py` with `root=PARTLABEL=nibos-root-b`);
- the manifest, and its minisign signature (`minisign -S`, with the secret key given on the
  command line, so it is never stored in the tree);
- the `.img.xz` and `SHA256SUMS` as today.

It writes them to `out/release-<version>/`, ready to attach to a GitHub release. It uploads
nothing.

## Tests

| Test | Where |
|---|---|
| A release from a file written to slot B of a card image (a loop device): every chunk verified, FIT B in place, the block unchanged until `apply` | the build machine (`tools/test-update.sh`, beside `test-image.sh`) |
| The same, interrupted at random chunks (killed, and the loop device detached), then resumed | the build machine |
| A tampered chunk, a bad signature, another board, another edition, an older version, `min_from` not met | the build machine |
| Each status carries its `code`: no server (`no_network`), nothing at the address (`not_found`, from a local HTTP server), and no curl text in `detail` | the build machine |
| A real A → B update from a local file, confirmed good; then `rollback`; then a broken update (a FIT that panics) rolling back by itself | the device, an image card |
| Battery below 30% unplugged: refused; plugged in: allowed | the device |
| Sleep in the middle of a download: resumes after waking | the device |
| The kept state (Wi-Fi, SSH, DSlot settings, pairings, brightness) all there after an update | the device |

## Not in the first version

- Downloads over anything but curl (already in both images).
- Delta updates (only the chunks that changed: the chunk hashes already allow it, by keeping
  the running slot's chunks that match).
- Updates without a network other than a file on NIBOS-DATA.
- The fallback following the good slot (A/B phase 3), which keeps the A-held way back on the
  new version.
