# Roadmap

What nibOS needs next to be a good base for anyone building for the Anbernic RG DS+ (other boards
later). Roughly in priority order. Each item says what "done" means; where something exists
already, it says so.

Today's base: the kernel starts 0.65 s after reset and the shelf is up at ~5 s; every part of the
board works, including deep sleep with Wi-Fi back in ~2.5 s; the hardware layer gives programs
one gamepad, sound that follows the jack, per-screen brightness without root; a template app runs
on both screens ([AGENTS.md](../AGENTS.md)).

## 1. Updates

A device should get fixes without a card reader, and never be left unbootable by one.

- **Rollback**: A/B slots, phase 1 done ([design/ab-rollback.md](design/ab-rollback.md)): SPL
  boots the slot the boot control block says, an unconfirmed slot gets three tries and then the
  other one takes over, a slot is confirmed once its boot UI is up, and the safe mode offers to
  go back. Next: **phase 2**, SPL arms the watchdog (hangs before systemd count as failures);
  **phase 3**, the fallback U-Boot follows the good slot; **phase 4**, the updater that writes
  a slot (below).
- **One-file updates**: copy a signed update file to the card's root and restart; the OS is
  updated, games, saves and settings are kept, and no network is needed. It uses the same
  signing key and layout as the network updates, and the A/B root partitions the images already
  have (`nibos-root-b`). Done when: an update file copied from a computer updates a device
  offline, a corrupt or unsigned one is refused with a message on the screen, and the device
  still boots.
- **A nibOS package repository**, signed, with nibOS's own packages (`nibos-hw`, `mesa-nibos`, the
  kernel as a package, DSlot) and **pinned Arch Linux ARM snapshots**: the device follows a
  snapshot date that has been tested, not the moving mirror.
- **Promotion only after device checks**: a snapshot or package set goes from testing to stable
  after the [test checklist](../AGENTS.md#test-checklist-on-the-device) and the boot and suspend
  benchmarks pass on a device ([bench/](../bench/README.md)), with the results recorded.
- **`nib update`**: fetch, install, write the new kernel to the inactive slot, keep the previous
  kernel and modules.
- Done when: a device updates over Wi-Fi from one stable set to the next, and a deliberately bad
  kernel is undone by itself or with A held.

## 2. Installer and first boot

Downloadable images exist, set up from `provision.toml` ([design/images.md](design/images.md)).

- **First boot on the device**: Wi-Fi network, user name and password, time zone, an SSH key (or
  none), with the controls and touch, before the boot UI starts.
- **Hostname and mDNS options** in `provision.toml`, editable after first boot (today the file
  is applied once).
- Done when: someone with a blank card and an imaging tool reaches the shelf with Wi-Fi and
  their own password, without editing a file.

## 3. App platform and SDK

What makes nibOS a base to build on. The design is in [design/app-model.md](design/app-model.md).

- **Screen ownership for two screens**: how programs get, share and give back the two panels:
  a clean KMS handover protocol, or a minimal compositor per screen.
- **App switching by quit and resume**: an app saves its state when asked and resumes fast; one
  app alive at a time on 1 GB. A system menu (Menu held) that can interrupt any app; only the
  system draws above a game (indicators, battery warnings, the menu).
- **[nibkit](https://github.com/abradburne/nibkit)**, the SDK library, and the
  [template app](../sdk/template/README.md): exist; they grow with the app model.
- **`nib deploy`**: the dev loop as a command on the computer (today `sdk/run.sh`), and packaging
  an app as a nibOS package from the template.
- Done when: two apps from the template can be switched between from the shelf, each resuming
  where it was, and a third party can build, run and package one from the docs alone.

## 4. Frontends

nibOS boots DSlot today. It should boot whichever frontend is installed.

- **A frontend contract**: a documented way to be the program nibOS boots: a frontend binary
  or script at a known path on the data card, or a launcher registered by its package. DSlot
  is the default; the base edition falls back to the shell. What a frontend gets and must do
  (screens, the gamepad, the device interface, sleep and quit) goes in
  [AGENTS.md](../AGENTS.md). Done when: a launcher that isn't DSlot is installed by copying it to
  the card or installing its package, boots instead of DSlot, and removing it brings DSlot back.
- **Ports of existing launchers** on top of the contract, with nibOS's gamepad, device interface,
  paths, sleep and quit.

## 5. USB-C device mode

The device tree puts the RK3568's OTG controller (DWC3) in peripheral mode, and the kernel
builds the configfs gadget functions for NCM, RNDIS and mass storage; still to check is that
this is the USB-C port, and that a computer enumerates it.

- **USB mass storage of the data card**: hold a button while plugging the cable in and the data
  card appears on the computer as a drive. Done when: games copied from a computer are on the
  shelf after unplugging, and unplugging mid-copy leaves the card intact.
- **USB networking** (CDC-NCM, with RNDIS for older Windows): SSH over the cable without Wi-Fi.

## 6. Power

- **Deeper CPU idle**: the cores only ever WFI today (no cpuidle driver). PSCI idle states in
  the device tree, if TF-A takes `CPU_SUSPEND`, measured against the idle draw
  ([idle-power.md](engineering/idle-power.md)).
- **Low-battery warning and safe shutdown** before the battery cuts out.
- **Power profiles**: a policy service for CPU/GPU performance against battery, which a game can
  ask for and the system can refuse; a thermal policy that tells programs why they're slower.

## 7. Device control and status

The [device interface](engineering/device-interface.md): settings programs may change without
root. Brightness per screen, volume, Wi-Fi, lid and battery protect are done.

- **Battery and charging for programs** as a nibOS interface rather than raw sysfs.
- **Bluetooth pairing** from the device (today from a shell with `bluetoothctl`).
- **An on-screen indicator** for the brightness and volume shortcuts.
- Done when: each setting in the device interface's table is done, with a `nib` command and docs.

## 8. Recovery and diagnostics

The safe mode, the watchdog, A/B rollback and the boot logs on the card exist
([recovery.md](recovery.md)).

- **A recovery menu on the device**: a button combination at power-on that offers the previous
  slot, a settings reset and a card check.
- **Reset to defaults**: settings back to a fresh install, games and saves kept.
- **Backup and restore** of saves and settings.
- **`nib doctor`**: one report of what's installed and what's wrong, for bug reports.

## 9. Security basics

- No default passwords and SSH by key only: done.
- A password set at first boot, with `sudo` and polkit asking for it once one exists.
- Signed packages and updates (item 1); SSH off by default on a user image.

## 10. Another board

- **A porting guide**: how a new handheld becomes `boards/<board>/` (and `socs/<soc>/` if new),
  using the RG DS+ as the worked example ([boards/README.md](../boards/README.md)).
- Done when: a second board is supported by following it.

## Inspiration

Some of these items come from other projects:

- [BaseOS](https://github.com/pvaibhav/BaseOS): the frontend contract, USB storage mode,
  one-file updates, a settings file on the visible partition.
- [Ghazal](https://github.com/sharno/ghazal): first-boot setup from `provision.toml`, a
  read-only root, A/B root slots with signed updates and a watchdog-confirmed boot.
