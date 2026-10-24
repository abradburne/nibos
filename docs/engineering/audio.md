# Audio

The RG DS+ has two sound cards, and nothing in the stock ALSA or PipeWire setup knows that they
are one device's headphones and speakers. The hardware layer
([`nibos-audio`](../../hw/README.md) and its WirePlumber rules) makes them behave as one
output that follows the headphone jack.

## The hardware

| Card | ALSA name | What |
|---|---|---|
| RK817 codec | `sound` | The headphone jack and the microphone, with UCM profiles; the jack switch is the input device `rk817_ext Headphones` |
| Two AW88166 smart amps | `sound-speakers` | The speakers: I2C 0x34/0x35 on i2c2, I2S3 (m1), resets on GPIO4_A7 and GPIO4_B2 |

The stock device tree calls the amps `awinic,aw883xx_smartpa`; the stock-OS probe read their
ID register (0x00 = 0x2066): AW88166, which mainline has (`snd-soc-aw88166`). Its driver has no
OF match table, so udev never loads it: the board's `modules-load.d` file does. The amps won't
play without a tuning file (below).

## The speaker amps' tuning

The driver loads `aw88166_acf.bin` when the amps start, and without it they stay off. Awinic's
own, the stock firmware's `aw883xx_acf.bin`, carries their DSP's firmware (speaker excursion
and voice-coil temperature protection, EQ) and can't be redistributed. So nibOS ships its own,
and imports Awinic's from the device's stock firmware on request.

**nibOS's tuning** (`boards/rg-ds-plus/hw/aw88166-tuning.py`, built into `nibos-hw` as
`/usr/lib/nibos/aw88166-nibos.bin`, 640 bytes) is written from scratch. The container is the one
the driver's GPL parser reads (`aw88395_lib.c`: an ACF header, version 0; per amp, matched by
I2C bus and address, a register list with its bin header and checksum, and the DSP
configuration and firmware the driver insists on, here 4-byte placeholders that the DSP,
bypassed, never runs; each section's CRC8). Every value is chosen from Awinic's public datasheet
(AW88166, Nov. 2023, V1.3), none from Awinic's file:

| Register | nibOS | Datasheet default | Why |
|---|---|---|---|
| SYSCTRL (0x04) | 0x9347 | 0xD307 | Gain 4.2 AV (the lowest, against 6.75), the DSP bypassed, DC cancelling and the I2S interface on |
| SYSCTRL2 (0x05) | 0x8000 | 0x8000 | The driver's starting volume, 0 dB: the gain and the boost limit the level (a 6 dB ceiling made the loudest too quiet) |
| I2SCTRL1 (0x06) | 0x04E8 / 0x08E8 | 0x04E8 | Philips I2S, 32-bit, BCK 64*fs, 48 kHz; the left channel on 0x34, the right on 0x35 |
| BSTCTRL2 (0x61) | 0x3B6B | 0x5B6C | Boost peak current 1.6 A (against 2.0 A), Smart Boost 2, at most 4.5 V |

With the DSP bypassed, only the chip's own protections remain (over-temperature,
over-current, under- and over-voltage): the low gain and the boost limits are there instead of
the DSP's speaker model.

**The official tuning** comes from the device's own stock firmware: `nib audio import-tuning`
finds a stock card in TF2 or a stock image (`.img`, `.img.gz`, `.img.xz`) on the games partition
(or takes a device or file given), reads the file out of its root file system as
`extract-stock.sh` does, checks it's an ACF file, and keeps it as
`/var/lib/nibos/firmware/aw88166-official.bin`. Either one is copied to
`/var/lib/nibos/firmware/aw88166_acf.bin`, which the kernel reads before `/usr/lib/firmware`
(`firmware_class.path` on the command line); `nib audio tuning nibos|official` switches, `nib
audio` says which, and the amps take it at the next start. DSlot's System page uses the same
through `nibkit::audio` (`tuning()`, `import_tuning()`, `set_tuning()`). A dev card made from
the stock image starts with the official one; a downloadable image with nibOS's.

**Listening tests** (2026-10-20, the RG DS+, by ear; a 440 Hz tone to the speakers only, the
speakers muted between steps): both amps ran on nibOS's tuning
exactly as written (SYSCTRL 0x9244, the status register 0x0311: PLL locked, clocks stable,
boost started, output switching; no fault bits), the stream S32_LE at 48 kHz as the file sets.

| Step | Level | Official | nibOS |
|---|---|---|---|
| 1-2 | 20-50 %, -30/-20 dBFS | at the threshold of hearing | not heard |
| 3 | 70 %, -12 dBFS | quiet but clear | clean, both speakers, quieter |
| 4 | 100 %, -6 dBFS | | quiet and clear |
| 5 | 70 %, -12 dBFS, no ALSA cut | | clear, clean, not too loud |
| 5 | 100 %, -6 dBFS, no ALSA cut | | clean and clear, louder, but still a bit quiet as a maximum |
| 6 | 100 %, -6 dBFS, no ALSA cut, the ceiling removed (VOL 0 dB) | | clear, loud enough as a maximum |

With a 6 dB ceiling in SYSCTRL2, nibOS's tuning started the amps ~5 dB lower than the official
one (the SYSCTRL2 volume register under each: 0x40 against 0xD3 with the same ALSA control);
without the ceiling (step 6, the shipped value) it is ~1 dB above. The amps' temperature register stayed
at 0x21 (0x22 once); no fault and no driver error during playback. Steps 1-4 ran with the amps'
`PCM Playback Volume` controls at 200 (about -19 dB), a value saved in the device's ALSA state
(`/var/lib/alsa/asound.state`, restored by `alsa-restore`), not nibOS's: the driver's default
is 0, which is what a new image starts with, and step 5 played at that.

## Headphones or speakers

PipeWire switches ports within a card but not between cards. `nibos-audio` (a user service next
to PipeWire) watches the jack switch (`SW_HEADPHONE_INSERT`) and makes the matching sink the
default with `pactl`: the headphones (`alsa_output.platform-sound.*`) while they are plugged
in, the speakers (`alsa_output.platform-sound-speakers.*`) otherwise. The sinks are told apart
by the start of their node names, which the board's
[`board.conf`](../../boards/rg-ds-plus/hw/files/usr/lib/nibos/board.conf) gives. Plugging and
unplugging headphones is not yet tested on the device; the default at start, the speakers, is.

## The speakers' keep-alive

The amps take a few hundred milliseconds to lock to the I2S clock whenever it starts, and that
cut off the start of every sound: a click in DSlot's menu was silent, a game's first notes were
missing. PipeWire only runs the speaker output while something is connected to it, so
`nibos-audio` keeps something connected: a silent stream (`pw-cat` from `/dev/zero` at volume
0, node `nibos-audio-keepalive`), restarted if PipeWire goes away. The WirePlumber rules also
never suspend the speaker sink (`session.suspend-timeout-seconds = 0`), name the outputs
("Speakers", "Headphones", "Microphone") and hide the amps' I2S input side, which is not a
microphone.

## The amps after suspend

The driver starts the amps from a DAPM event, which can come before the I2S clock runs: the
amp's PLL check fails. At boot a later try succeeds. At resume it didn't: the widgets power up
before PipeWire restarts the stream, all five tries failed at once, and the speakers stayed
silent until the stream was closed for longer than ASoC's 5 s power-down delay (stopping
PipeWire for 8 s brings them back on an unpatched kernel). nibOS's tuning makes it certain: with
no DSP firmware to rewrite between tries, they come straight after each other; the official file's
144 KB rewrite gives the clock time. Kernel patch generic/0005 starts the amps again from the
DAI's trigger, once the stream runs. A running amp reads SYSCTRL (0x04) with AMPPD (bit 1) and
HMUTE (bit 8) clear (0x9244 with nibOS's tuning) and SYSST (0x01) 0x0311 (PLL locked), in
`/sys/kernel/debug/regmap/2-0034/registers` and `2-0035`; a silent one reads 0x9346 and
0x0011. [`amp-resume-test.sh`](../../boards/rg-ds-plus/hw/amp-resume-test.sh) runs suspend
cycles woken by the RTC and checks both after each: 10/10 with either tuning on the patched
kernel, 0/1 before. Checked by ear (2026-10-20, kernel 917b746): with nibOS's tuning, the
speakers play after a tap of power to sleep and a tap to wake.

## Volume keys

Outside a desktop, `nibos-audio` turns the volume keys (their own input device,
`gpio-keys-volume`) into 5% steps on the default sink with `wpctl`, capped at 100%; under a
Wayland compositor (a socket in `XDG_RUNTIME_DIR`) it leaves them to the desktop's own bindings.

## The boot UI and games

DSlot runs from a system unit with no login session, so it finds the image account's PipeWire
through `XDG_RUNTIME_DIR=/run/user/<uid>` in nibOS's drop-in for its unit
([user.conf](../../rootfs/dslot/user.conf)); the account lingers, so its user manager (and
PipeWire) runs from boot. The games it starts run in
the same unit and reach PipeWire the same way.

## Bluetooth headphones

A connected Bluetooth sink (`bluez_output.*`) becomes the default as soon as it appears, and the
jack decides again when it goes: `nibos-audio` routes on PipeWire's sinks coming and going
(`pactl subscribe`) as well as on the jack, Bluetooth first. Without that, its own
`set-default-sink` would keep the speakers configured and WirePlumber wouldn't switch. The
volume keys step `@DEFAULT_AUDIO_SINK@`, so they follow; the speakers' keep-alive targets the
speakers by name, never Bluetooth. WirePlumber's BlueZ monitor runs with seat monitoring off
(DSlot has no logind seat). Not yet tried with real headphones.

The codecs PipeWire offers (its BlueZ endpoints, read after a restart on 2026-10-22): SBC (every
headset has it; GPL-2.0-or-later / LGPL-2.1-or-later), aptX, aptX HD and aptX Low Latency
(libfreeaptx, LGPL-2.1-or-later), LDAC to send (libldac, Apache-2.0), FastStream, Opus
(BSD-3-Clause), and for calls mSBC and LC3 (liblc3, Apache-2.0). **No AAC**: Arch's
pipewire-audio depends on Fraunhofer's FDK AAC library for that one plugin, whose licence grants
no patent rights, so nibOS leaves it out (`rootfs/no-fdk-aac`: a package that stands in for
libfdk-aac, and pacman's `NoExtract` for the AAC plugin). AAC headphones use SBC. aptX and LDAC
are patent-encumbered formats too, with free implementations, as most distributions ship them.

## Open

- Three kernel WARNINGs at every boot from `audio-graph-card2` parsing the speakers' `multi`
  node (`bad of_node_put()`, refcount underflow and saturation, in `graph_parse_node`): seen
  on kernels since at least 2026-10-21, harmless so far, a real bug in the card's device tree or the
  driver. `tools/hwtest.sh boot` fails on it until it's fixed.

## Dead ends: don't repeat

Tried and measured, or found not to work; the source says more. Add to it when something doesn't pay.

- Retrying the amps' start ("check pll lock fail") against the clipped first sound: the keep-alive stream fixed it.
- A 6 dB volume ceiling in nibOS's tuning: the loudest was too quiet; removed.
- At resume the amps' DAPM start runs before the I2S clock: fixed by patch generic/0005, not by retries.
- Awinic's tuning file in the package: not redistributable.
- The upgrade hook using `/var/lib/nibos/firmware` as scratch (wiped the tuning); `cmp` (not installed): fixed.
- The AW88166 driver has no OF match table: udev never loads it; modules-load.d does.
- DSlot's clicks without `XDG_RUNTIME_DIR`: pacat can't connect.
- Volume steps relative to the level (37% to 42%): steps onto multiples of 5%.
- PipeWire's 200 ms `RLIMIT_RTTIME` killed a real-time emulator thread.
