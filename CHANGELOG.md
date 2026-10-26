# Changelog

## 0.1.0-alpha2

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
