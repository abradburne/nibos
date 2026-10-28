# Using nibOS

What a nibOS card does once it is written ([install.md](install.md)) or built
([building.md](building.md)), and how to change it. The controls and names are the RG DS+'s.
Where an image and a development card differ, both are given.

## Wi-Fi

On an image, the easiest is `provision.toml`'s `[wifi]` before the first boot
([design/images.md](design/images.md#first-boot-and-provisioning)). Wi-Fi starts off on a fresh
image unless `provision.toml` names a network; `nib wifi on|off|status` switches the radio
(`nib bluetooth` the same for Bluetooth), and the choice is kept.

On a development card, drop a file on the card's ROMS volume, named after the network, in
`ROMS/nibos/wifi/`. It is copied into iwd at every start:

`ROMS/nibos/wifi/MyNetwork.psk`:

```ini
[Security]
Passphrase=the-network-password
```

A network name with anything but letters, digits, `-` and `_` needs iwd's hex file name: `=`
followed by the name in hex, e.g. `=4d79204e6574.psk` for "My Net". Open networks are
`NAME.open`. Or plug a keyboard in and run `iwctl station wlan0 connect <network>`.

When the network comes up depends on the network mode (below). Power save is off: with it on,
the RTL8821CS drops off the network while awake.

## Logging in

- **SSH:** on an image, `ssh nibos@nibos.local` with a key from `provision.toml`'s `[ssh]`
  (SSH stays off until there is one; no root login). On a development card,
  `ssh $NIBOS_USER@$NIBOS_HOSTNAME.local` (or root), with the key from `SSH_PUBKEY`
  (`config.sh`). Password login is off on both. The device answers by its host name through
  mDNS.
- **On the screens:** without a password, tty1 logs the user in by itself and `sudo` needs no
  password (on an image with a password from `provision.toml`, a login prompt, and `sudo` asks
  for it). The console is on the top screen and a keyboard on the bottom one (`nibos-keys`):
  touch it to type, with Shift, Ctrl, Alt and Fn tapped before a key (twice to keep them on),
  and a row of shortcuts (Esc, Ctrl-C, Ctrl-D, Ctrl-Z, Ctrl-L, Home, End, Page Up and Down).
  The buttons type too: the D-pad the arrows, A Enter, B Backspace, X Tab, Y Ctrl-C, L Shift
  and R Ctrl while held. **Menu + X** hides it (the bottom screen goes black) and shows it
  again; a touch on the black screen shows it too. A USB keyboard (in the host-only USB-C port, not the charging one) or a
  Bluetooth one works beside it.

A development card has no passwords at all. Set one with `passwd` before relying on the device
anywhere that matters.

## Controls and power

| | |
|---|---|
| Close the lid, or a short press of power | Sleep (suspend to RAM) |
| Open the lid, or power | Wake; Wi-Fi reconnects about 2 s later |
| Hold power (1 s) | The power menu (Sleep, Restart, Power off) with DSlot; without it (the base image's console), a clean power-off. Holding it for about 10 s cuts the power instead, without saving anything: only for a device that doesn't answer |
| Volume keys | Steps onto multiples of 5% on the current output (outside a desktop); with Menu held, both screens' brightness |
| Headphones | Sound moves to them while they're plugged in, back to the speakers after |

**The two USB-C ports.** One charges the device (and is the port a computer would see, in a
later device mode); it is not a host, so a keyboard there does nothing. **Keyboards, pads and
other USB devices go in the other, host-only USB-C port** (with a USB-C adapter or cable).

The buttons and stick appear to games as one gamepad, "Anbernic RG DS+ Gamepad", laid out by
position (the bottom face button is BTN_SOUTH); Menu is the gamepad's guide button. LEDs: green
while running, amber while the charger is plugged in (asleep too). Plugged into a switched-off
device, the charger switches it on and it charges asleep, dark, with amber lit: the power key
starts it properly, unplugging switches it off again ([hw/README.md](../hw/README.md)).
`nib battery` shows the charge and battery protect ([engineering/battery.md](engineering/battery.md)).

## Speakers

The speaker amps need a tuning file. nibOS ships its own (the amps' DSP bypassed, conservative
gain); Awinic's official one, with the DSP's speaker protection and EQ, can be imported from
your own stock firmware:

```bash
nib audio                          # which tuning the speakers use (nibos, official, none)
sudo nib audio import-tuning       # from a stock card in TF2, or a partition or stock image file
sudo nib audio tuning nibos        # back to nibOS's (official: the imported one again)
```

Restart to apply a change ([engineering/audio.md](engineering/audio.md)).

## The boot UI: DSlot or a shell

From 1.5 s after power-on the DSlot logo is on the top screen until DSlot (or the console) takes
over, and at power-off and restart a goodbye picture stays up until the power goes
([rootfs/splash](../rootfs/splash/README.md)).

```bash
ssh $DEVICE sudo nib boot-ui on      # DSlot on both screens at power-on
ssh $DEVICE sudo nib boot-ui off     # the tty1 shell
rootfs/splash/boot.sh on|off|status  # the logo at 1.5 s (from the Mac, on a development card)
```

Leaving DSlot (Select + Start), or DSlot failing, starts the shell on tty1 either way. DSlot
lists `Roms/NDS`, `Roms/GBA`, `Roms/GB`, `Roms/GBC` and the other systems' folders on the data
partition (ROMS on a development card, NIBOS-DATA on an image) and on a card in TF2 (exFAT or
FAT32, any name). Saves and save states are kept per game in `Saves/<SYSTEM>/<ROM name>/` there.

Games are started by a launcher and emulators that are separate projects, installed as their
own packages; their controls and settings are theirs ([rootfs/dslot/README.md](../rootfs/dslot/README.md#games)).

## Brightness

Each screen has its own brightness, kept across restarts:

```bash
nib brightness                  # both screens, in percent
nib brightness top 40           # the top screen at 40% (bottom, all)
```

**Menu + Volume up / down** changes both screens at once, from anywhere (in a game too; it
doesn't reach the game or the volume). Percent is perceptual, and 0 is a dim floor, never off.
Programs in the video group can set `/run/nibos/backlight/top/brightness` and
`.../bottom/brightness` directly ([hw/README.md](../hw/README.md), "Brightness").

## Network modes

```bash
nib net status                          # up or down, and the mode
sudo nib net up | down                  # now
sudo nib net mode boot|deferred|demand  # from the next boot
nib help                                # the commands this device has
```

| Mode | For | The network comes up |
|---|---|---|
| `boot` | the dev system | with the system |
| `deferred` | a productivity setup | once start-up has finished, at the lowest priority |
| `demand` | a game player | only when something starts `nibos-network.target` |

## Date and time

The clock is set from the network by systemd-timesyncd whenever the network is up. On the
DSlot edition Wi-Fi comes up on demand, so after a power-off the time may be hours out until
something brings the network up; DSlot's **System → Date and time** page shows the time, the
zone and where the time came from, and offers **Sync now**, a zone picker and setting the time
by hand. The same from a shell:

```bash
nib clock                               # the time, the zone, set from the network or not
sudo nib clock sync                     # Wi-Fi up if it is off, then the time servers
nib clock zones                         # the zones to choose from
sudo nib clock zone Asia/Tokyo          # the zone, kept across updates
sudo nib clock set '2026-10-27 11:30'   # by hand: network time goes off
sudo nib clock ntp on                   # network time back on
nib sleep                               # asleep this long on battery, it powers off (60 min)
sudo nib sleep off-after 15             # a quarter of an hour instead; `never` sleeps until woken
```

A time set by hand turns network time off (it would be put straight back otherwise) until
`ntp on` or the page's **Use network time**. The zone is kept on the state partition
(`/var/lib/nibos/state/timezone`), so an update, which replaces the root, keeps it;
`provision.toml`'s `timezone` sets the same file. Where the board has a hardware clock
(`/dev/rtc0`, the RG DS+ has one) it is written after every set and sync, so the time survives
a power-off.

## Updates

On an image card, `nib update` updates the whole system from nibOS's releases: `nib update
check`, `nib update download` (in the background; the device stays usable), then `nib update
apply` to restart into it. The new version is written to the slot that isn't running and
tried; if it doesn't come up, the device goes back to the old one by itself, and `nib update
rollback` goes back by hand. Games, saves and settings stay. How it works:
[design/updates.md](design/updates.md).

## What to do if…

- **Something went wrong and you want to report it.** `nib bug-report send` saves a scrubbed
  copy of this start's log on the card (`nibos/bug-report-DATE.txt`) and, when the image names a
  bug tracker, sends it and prints a code to quote. Without a network it is saved all the same:
  attach the file. [recovery.md](recovery.md), "Read the logs on the card".
- **It won't start**: hold **A** while powering on; that boots the fallback whatever is wrong
  with the normal path. The boot logs are on the card. [recovery.md](recovery.md).
- **"DSlot stopped working" appears**: DSlot crashed several times in a row and the safe mode
  took over. Choose Restart DSlot; if it keeps happening, Open a console and look at
  `journalctl -u dslot -b`. Just after an update it also offers **Go back to the previous
  version** ([rootfs/safe-mode](../rootfs/safe-mode/README.md)).
- **An update rolled back** (the device is on the old version): the new one didn't reach its
  shelf three times, so the bootloader went back by itself. `sudo nibos-bootctl status` says
  which slot runs and what happened ([recovery.md](recovery.md#ab-slots)).
- **It froze**: it restarts by itself, about 30 s after a hang (the watchdog) or 10 s after a
  kernel crash. Holding power for about 10 s cuts the power in any case.
- **It's dark with the amber light on**: it's charging while "off" (above); press power to use
  it.
- **The speakers are silent**: check `nib audio` and the volume; `tools/hwtest.sh audio`
  from a computer reads the amps' state ([audio.md](engineering/audio.md)).
