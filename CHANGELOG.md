# Changelog

## Unreleased

- **An updated system has its whole partition again.** An update writes the new system with the
  image's small root file system (1.7 GB), and the step that grows it to fill its 3 GB partition
  ran only on a card's first start: its "done" mark is carried over by updates, so every updated
  device ran with 1.3 GB unused and, with PortMaster's ports in the home folder, could fill up
  (DSlot's settings then can't be saved, nor anything installed). It now grows at every start
  (`nibos-growfs`; a no-op taking 0.2 s once the file system fills its partition), so the first
  start of an update gives the space back.

## 0.1.0-alpha6

- **Which systems earn achievements**, on DSlot 0.4.1's Achievements page: DS, Game Boy and Game
  Boy Color for now, more coming. And DSlot hears the buttons again when nibos-input makes its
  gamepad anew (an update of nibos-hw, or after a crash), rather than only the power key.
- **Writing a new image keeps your settings.** It already kept NIBOS-DATA (games and saves), but
  the settings live in the home folder, which a new image replaces: DSlot's and each emulator's
  settings (a console's screen shape, say), play time and favourites went. nibOS now keeps a
  copy of them in NIBOS-DATA's `nibos/settings/` (every 10 minutes, before sleep, at power-off)
  and puts it back on the first start after an image is written, before DSlot. The
  RetroAchievements sign-in isn't copied (the drive reads on any computer): sign in again.
  Delete `nibos/settings/` on the drive to start with defaults.
- **Wi-Fi left on comes back as soon as usual after an update.** On the first start of a new
  version the 30 s check that confirms it held the start-up open, and the radios, which wait for
  the start-up to finish, came back half a minute late, with DSlot's switch showing Wi-Fi off
  meanwhile; the check no longer holds it (`nibos-boot-good` is a simple unit, like the bootlog's).
- **Wi-Fi left on reads as on while it starts.** Until the driver loads, a few seconds after
  start-up, `nib wifi status` (and so DSlot's switch) said off; it now gives the switch as it was
  left ("on", then "starting"). Switching it off in those seconds loads the driver first and
  blocks it, rather than being undone when the driver appears with the switch it kept.

## 0.1.0-alpha5

- **RetroAchievements.** DSlot 0.4.0 signs in from the menu's new **Achievements** page (the
  password is typed once and never kept; DSlot keeps the token the service answers with) and
  shows each DS, GBA, GB and GBC game's set on the shelf: how many are earned, the points, a bar,
  and the full list with badges from the Game page. DSperate (DS) and inkdot (GB, GBC) earn them
  while you play, signed in through DSlot: a notice with the badge when one unlocks, the
  achievement in progress and any challenge under way in the bottom-left corner, progress kept
  in save states and the resume point, and unlocks earned offline sent at the next start.
  Achievements are Casual (softcore) for now; Hardcore stays off until RetroAchievements approves
  our emulators.
- **Notices look the same everywhere.** DSperate's and inkdot's notices are nibkit's (0.1.4),
  drawn as DSlot draws its own, one after another rather than over each other.
- **DSperate no longer dies starting a game with a big achievement set.** Signed in, a game whose
  set is large (Bubble Bobble Double Shot's is 446 KB) was killed by the RT watchdog a second in:
  the set was read on the emulation thread as a real-time task. It is read as an ordinary one now.
- **Any card in TF2 is read.** The games card had to be exFAT and named `DS`; one named anything
  else was silently ignored. nibOS now mounts whatever card is in the TF2 slot, exFAT or FAT32,
  whatever its name (a udev rule matches the slot, `mmcblk1`, and links its volume to
  `/dev/nibos-games-card` for fstab; on a card with several volumes one named `DS` still wins).
  FAT file names are UTF-8 by default (`CONFIG_FAT_DEFAULT_UTF8`), so a FAT32 card's Japanese
  titles read right. PortMaster puts ports on the TF2 card whenever it is mounted.
- **Asleep too long on battery, the device powers off.** Deep sleep, already the deepest state
  this kernel offers with the memory kept, still draws about 49 mA on the RG DS+ (s2idle 123 mA):
  a full battery in 3.6 days. When it sleeps on battery, a sleep hook now sets the RTC's alarm
  `nib sleep off-after` minutes ahead (default 60, `never` to turn it off, kept across updates),
  and if that alarm is what wakes it and the charger is still out, it powers off cleanly; the
  emulators save the game's resume state at shutdown as for the power menu, so after the ~6 s
  boot the game is a Resume away. On the charger it sleeps on. The measurements, the states the
  board has and what hibernation would cost: docs/design/deep-sleep.md.

## 0.1.0-alpha4

- **The kernel names what it kills.** `kernel.print-fatal-signals` is on (hw's sysctl.d), so a
  program that dies of a signal it did not handle leaves a line with its name in the kernel log,
  and so in a bug report. Found through one: the RT watchdog's kill of DSperate at start-up
  ("RT Watchdog Timeout (hard)"), which DSlot could only report as "signal 9". DSperate's
  start-up now runs as an ordinary task until emulation begins (it reached SDL as a real-time
  task under the 200 ms budget PipeWire leaves in DSlot, and the kernel killed it when that
  stretch took longer than 200 ms of CPU, as on a slow clock).

## 0.1.0-alpha3

- **A bug report carries the kernel's first lines too.** The report `nib bug-report` makes (and
  DSlot's Y: send a bug report) adds the first 150 lines of the boot log to the last 3000: the
  kernel's account of the board, its memory and its command line, which a long session's tail
  left out, and which a crash report needs to make sense of an out-of-memory kill.
- **A DSlot started by hand gets past the safe mode.** Once the safe mode is up it forgets
  DSlot's failures, and a start of `dslot.service` (its menu, `nib boot-ui`, `systemctl start`)
  stops the safe mode first. Until now the start was refused as repeated too quickly, or DSlot
  died at once because the safe mode held the display, and the safe mode came straight back:
  what three quick restarts of a test build did on 27 October. And the safe mode now comes up
  after a crash at all: the package unit's own `OnFailure=getty@tty1` was named beside it and
  won, so a DSlot that kept crashing left a login prompt on the top screen instead.
- **The clock can be seen and set.** `nib clock` shows the time, the zone and whether the time
  came from the network; `nib clock sync` brings Wi-Fi up and waits for the time servers,
  `nib clock zone` sets the zone and keeps it on the state partition so an update doesn't lose
  it (`nibos-provision` puts it back at boot; `provision.toml`'s still wins), `nib clock set`
  sets the time by hand with network time off until `nib clock ntp on`, and the hardware clock
  is written after each. DSlot's System page has it as **Date and time**
  ([using.md](docs/using.md#date-and-time)). Until now a DSlot card showed UTC, and the time
  was hours out until Wi-Fi came up.
- DSlot 0.3.2: **System info** and **Date and time** on the System page, **Network info** on
  the Wi-Fi page ([DSlot's changelog](https://github.com/abradburne/DSlot/blob/main/CHANGELOG.md)).
- `nib update` refuses to update a card whose root check failed, as 0.1.0-alpha2's notes said
  it would: that build's `nibos-hw` was made before the check went in.

## 0.1.0-alpha2

- **A card that wasn't written right says so.** Once per write of the card, about a minute
  after the first start and at idle priority, `nibos-root-verify` checks every file the image
  shipped under `/usr` and `/boot` against the SHA-256 list the build wrote. A mismatch is in
  the journal and `/var/lib/nibos/state/root-verified`, DSlot shows "This card was not written
  correctly" on the shelf, and `nib update` refuses to update over it
  ([recovery.md](docs/recovery.md#a-card-that-wasnt-written-right)). The usual cause is an
  imaging tool writing a `.img.xz` wrongly: one emulator quits at once while another runs.
- **A crash says where it died.** DSperate (DS and GBA) writes one line from its crash handler,
  `dsperate: crashed (SIGBUS) after "displays open", loading X.nds`, names the ROM it was given
  in its first log lines, and its "cannot start" notice is no longer dismissed by a button still
  held from the launch; DSlot's notice quotes that line.


- **Starting without a card in TF2 no longer takes minutes.** `/mnt/ds` was an automount
  whose 5 s device timeout every touch waited out, again and again, with the slot empty; the
  card is now mounted by udev the moment it appears and unmounted when it goes
  (`70-nibos-games-card.rules`), and nibkit 0.1.1 leaves its folders out with no card in.
  Measured: the shelf up 0.6 s after DSlot starts with TF2 out; a hot-plugged card mounted in
  0.25 s.
- **`nib bug-report`**: a copy of this start's log with the private parts taken out (Wi-Fi
  names and passphrases, MAC and IP addresses, the hostname), saved as
  `nibos/bug-report-DATE.txt` on the data partition; `nib bug-report send` also posts it to
  nibOS's bug tracker and prints a code to quote. DSlot 0.3.1 offers it from the notice it
  now shows when a game stops at once ([recovery.md](docs/recovery.md)).
- DSlot 0.3.1: a game that comes straight back says why, on the shelf and in the log.

## 0.1.0-alpha1

The first public release: nibOS on the Anbernic RG DS+ (RK3568), mainline Linux and U-Boot,
Arch Linux ARM, the `nibos-hw` hardware layer, and DSlot as the boot UI.

- **Fast boot**: mainline U-Boot with SPL in Falcon mode starts the kernel ~0.65 s after reset;
  a picture on both screens at 1.5 s, DSlot's game shelf at ~5 s ([boot-time.md](docs/boot-time.md)).
- **The whole board works**: both panels at 59.94 Hz, touch, one gamepad, speakers and
  headphones switched automatically, Wi-Fi, Bluetooth, both microSD slots, USB host, LEDs.
- **Deep sleep** on the lid or a tap of power, with Wi-Fi back after waking
  ([suspend-and-wifi.md](docs/engineering/suspend-and-wifi.md)).
- **Battery**: the level from the PMIC's coulomb counter, battery protect (charge to 4.1 V,
  about 80-85%), charging while switched off or asleep ([battery.md](docs/engineering/battery.md)).
- **Speakers**: nibOS's own redistributable tuning for the AW88166 amps, and
  `nib audio import-tuning` for the official one from your stock firmware
  ([audio.md](docs/engineering/audio.md)).
- **Recovery**: A/B slots with automatic rollback from an unconfirmed slot, DSlot restarted
  after a crash and then a safe mode, the hardware watchdog, panics that restart, and a
  known-good fallback when A is held at power-on ([recovery.md](docs/recovery.md)).
- **System updates**: signed releases written to the slot that isn't running, tried, and kept
  only once the new version comes up; from DSlot's System → Update page or `nib update`
  ([design/updates.md](docs/design/updates.md)). A hang restarts the device by itself (the
  watchdog panics, and the panic restarts through the PMIC).
- **Two image editions**, base (a shell) and DSlot, set up at first boot from
  `provision.toml` with no credentials baked in; the host answers as `nibos.local`, user
  `nibos` ([install.md](docs/install.md), [design/images.md](docs/design/images.md)).
- **Writing a newer image keeps your games and saves**: the data partition (NIBOS-DATA) sits
  past the end of any image, so a card written again keeps it, with either edition.
- **Wi-Fi and Bluetooth off on a fresh card**, then kept as you leave them; `nib wifi`,
  `nib net` and three network modes ([using.md](docs/using.md)).
- **A console with an on-screen keyboard**: the console on the top screen, a touch keyboard on
  the bottom one.
- **PortMaster**, downloaded when you ask for it.
- **Boot logs on the card** (`nibos/log/` on the data partition), written 20 s into each start,
  at power-off and when DSlot fails, readable from a computer when the device can't be reached.
- **`nib`**, one command for device settings (brightness, volume, Wi-Fi, lid, battery, boot UI),
  and a device interface programs use without root
  ([device-interface.md](docs/engineering/device-interface.md)).
- **For developers**: [AGENTS.md](AGENTS.md), a Rust + SDL2 template app, `sdk/run.sh` for the
  dev loop, and [nibkit](https://github.com/abradburne/nibkit) (MIT) as the SDK library.
- **Images you can redistribute**: nothing from the stock firmware; a `NOTICE` lists every
  package with its licence. nibOS's own code is GPL-3.0-or-later ([README](README.md#license)).
