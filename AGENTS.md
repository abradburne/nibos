# AGENTS.md: building for nibOS

You are building an app (or changing nibOS itself) for **nibOS on the Anbernic RG DS+**. Read this
first; it is short on purpose and every number in it was measured on the device or read from this
repository. Deeper documents are linked. Start new apps from [`sdk/template/`](sdk/template/),
which runs on the device as it is.

## The device

| | |
|---|---|
| SoC | Rockchip RK3568: 4× Cortex-A55, 408 MHz–1.992 GHz (`scaling_available_frequencies`: 408, 600, 816, 1104, 1416, 1608, 1800, 1992 MHz) |
| GPU | Mali-G52 (Panfrost, id 0x7402), 200–800 MHz, GLES 3.1 through Mesa 26.2.4 (`mesa-nibos`: Panfrost only, no LLVM) |
| RAM | 984 MB total; ~560 MB available with the boot UI running. Compressed swap in RAM (zram) of 984 MB, priority 100 |
| Screens | Two 1024×768 MIPI-DSI panels at **59.94 Hz**, one CRTC each, vblanks 8.2 ms apart. **SDL display 0 = bottom panel** (DSI-1, CRTC 0, **touch**); **SDL display 1 = top panel** (DSI-2, CRTC 1). Both are upright in software; no rotation needed |
| Touch | Bottom panel only: `Goodix Capacitive TouchScreen`. SDL finger events arrive normalized 0–1 over the bottom window |
| Controls | Face buttons, D-pad, L/R, L2/R2, Select, Start, Menu, one analog stick, volume up/down, Back, power, lid |
| Sound | Speakers (two AW88166 amps) and a headphone jack (RK817), switched automatically |
| Storage | Two microSD slots. `/mnt/roms`: the card's ROMS partition (FAT, automounted, owned by the login user). `/mnt/ds`: the second card (exFAT, label `DS`, automounted). The root file system is ext4 on the first card |
| Battery | `rk817-battery`, 4247 mAh design; `rk817-charger` |
| Software | Arch Linux ARM, systemd 262, SDL3 3.4.18 with sdl2-compat 2.32.74, PipeWire 1.6.9, Rust 1.99 in the build machine |

Details: [boards/rg-ds-plus/README.md](boards/rg-ds-plus/README.md), [docs/architecture.md](docs/architecture.md).

## Graphics

- **Use SDL on KMSDRM, one fullscreen window per screen, with an accelerated renderer or a GLES
  context, and vsync.** Run with `SDL_VIDEODRIVER=kmsdrm SDL_KMSDRM_ATOMIC=0` (SDL3's atomic path
  has no plane for the second panel and crashes tearing its window down). Open window *i* at
  `display_bounds(i)` with `fullscreen_desktop()`. See [`sdk/template/src/main.rs`](sdk/template/src/main.rs).
- **Do not drive KMS page flips yourself** (no own DRM master, no direct `drmModePageFlip`): an
  app doing that hung the device in suspend. Present through SDL's GL path.
- **The GPU is small.** Budget for fill rate and memory bandwidth: avoid full-screen overdraw and
  many full-screen passes, keep render targets at 1024×768 or below, batch draws. Panfrost runs
  late-Z when a shader writes `gl_FragDepth`, so don't, if you want early-Z.
- **Screen ownership today:** one program owns both screens at a time. DSlot (the boot UI) owns
  them, lets go before it starts a game or app, and takes them back when that exits. This will
  change ([docs/design/app-model.md](docs/design/app-model.md)).
- **Handing the screens over:** a program that hands the screens to another and takes them back
  keeps the DRM card open across the handover (the last close of the card restores the text
  console, which then flashes between the two), and keeps a picture of its own on the outputs
  (dumb buffers, set while it is master and no one else is). DSlot and the boot splash do;
  measured, the console was on both panels ~630 ms before a game and ~470 ms after it until
  DSlot did (DSlot's docs/nibos.md has the details).

## Input

- **Use SDL's game controller API.** The buttons and stick are one gamepad, **"Anbernic RG DS+
  Gamepad"**, made by `nibos-input`. SDL names buttons **by position**: SDL `A` is the bottom face
  button (labelled **B** on the device), `B` the right one (labelled **A**), `X` the left, `Y` the
  top. `Back` = Select, `Start` = Start, `Guide` = **Menu**. Stick axes: ±32767, 0 at rest, a 2048
  flat zone (use a dead zone of ~4000). The D-pad is a hat: SDL gives `DPadUp`… buttons.
- **Menu reaches you as a tap on release** (press and release together): `nibos-input` holds its
  press back in case it's a shortcut. Menu held with another button arrives normally, Menu first.
- **Reserved, never delivered to apps:** Menu + Volume up/down (both screens' brightness). The
  volume keys themselves change the volume outside a desktop; don't rely on them.
- Back is a key (KEY_BACK), not a gamepad button. Through sdl2-compat it arrives as SDL2's
  "Sleep" keycode; bind both "Sleep" and "AC Back".
- USB keyboards (in the host-only USB-C port, not the charging one) and Bluetooth keyboards and pads work as normal SDL devices.

Details: [docs/engineering/input.md](docs/engineering/input.md).

## Audio

- PipeWire runs at **48 kHz** (quantum 1024 by default, 32–2048 allowed). Open SDL audio at 48000 Hz,
  stereo, s16 or f32; 512–1024 sample buffers. SDL's audio goes through PipeWire.
- Outputs switch by themselves (headphones while plugged in, speakers otherwise). A silent
  keep-alive stream always plays to the speakers so their amps don't clip the start of a sound;
  don't fight it.
- Run as the login user, with `XDG_RUNTIME_DIR=/run/user/<uid>` set (it is under a login or DSlot).

## Device settings

Change device settings through nibOS's names, without root
([docs/engineering/device-interface.md](docs/engineering/device-interface.md)):

- **Brightness per screen:** write `/run/nibos/backlight/top/brightness` or `.../bottom/brightness`
  (group `video`). Percent → level: `floor + (max - floor) * p / 100`, with `max` from
  `max_brightness` (1666) and `floor = max * NIBOS_BACKLIGHT_MIN_PERCENT / 100` (2% → 33; in
  `/usr/lib/nibos/board.conf`). Never write below the floor. Or run `nib brightness top 40`
  (sets and saves). In Rust: `nibkit::backlight::{get, set, save}`.
- **Brightness has one source of truth, the kernel's level.** The Menu + volume shortcut, `nib`
  and every app write the same file, and nibOS saves it; don't keep a copy in your own settings.
  To show it, read it when you show it, and **notice changes made elsewhere** (the shortcut
  pressed over your settings screen): `nibkit::backlight::watch()` gives a `Watch` whose
  `changed()` (once a frame, never waits) says when to read the levels again; it also has a
  descriptor for a poll loop. Without nibkit: inotify `IN_MODIFY` on each screen's
  `brightness` and `bl_power` (the kernel raises it for every write, yours included).
  `cargo run --example brightness` in nibkit shows it.
- **Screen off:** write `4` to `/run/nibos/backlight/<screen>/bl_power` to power a screen's
  backlight down (touch keeps working), `0` to light it (`nibkit::backlight::set_off`); **turn it
  back on before you exit**. Any
  brightness change through nibOS (Menu + volume too) also lights it.
- **Volume:** `wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 40%` as the login user (the volume keys
  step onto multiples of 5%, as brightness does); `nibkit::device` has it, and
  `device::watch_volume` for changes made elsewhere.
- **Network:** `nib wifi on|off|status` is the Wi-Fi switch (the radio alone; Bluetooth stays
  as it is; the wheel group may). `nib net up|down|status` starts or stops both radios, for the
  network modes (`nib net mode` needs root).
- **Lid:** `nib lid sleep|ignore` (kept across restarts), `nib lid` to read it.
- **Battery:** `nibkit::battery::read()` gives the charge (the RK817's coulomb count, the same
  `/run/nibos/battery/capacity` nibOS shows), charging or not, and an estimated time to full;
  `battery::protect()` and `set_protect(on)` the **battery protect** switch (charge to 4.1 V,
  about 80-85%), or `nib battery protect on|off`. Without nibkit: `constant_charge_voltage` on
  `/run/nibos/battery`, below `constant_charge_voltage_max` when on (wheel may write it).
  [docs/engineering/battery.md](docs/engineering/battery.md).
- Device names come from `/usr/lib/nibos/board.conf`; never hard-code `backlight1`, `event7` or
  input device names.

## Power, scheduling, heat

- **Sleep:** closing the lid or a tap of power suspends to RAM; opening or power wakes. Apps get
  no special message: timers jump. Save state often; SDL may report no event at all.
- **SIGTERM means "save and quit"** (nibOS's quit-and-resume model): save your resume state and
  exit 0, promptly. Holding power for 1 s opens the power menu: DSlot sends the game it started
  SIGTERM, waits up to **10 s**, then SIGKILL; Cancel or waking from Sleep starts it again, and it
  should resume where it was. systemd sends the same at shutdown. Handle SIGTERM in every app,
  DSlot or not.
- **The power key is the system's** (tap: sleep, hold: the power menu); apps never see it as
  input they should act on.
- **Real-time threads:** apps started by DSlot inherit `LimitRTPRIO=10`. PipeWire's client
  library sets **`RLIMIT_RTTIME` to 200 ms** in any process that uses it: a real-time thread that
  runs 200 ms without sleeping is **killed (SIGKILL)**. Do heavy work (loading a ROM or a level,
  decompressing) on a normal thread, never on a SCHED_RR/FIFO one; drop to SCHED_OTHER around
  long bursts.
- **CPU governor:** performance while booting, then **schedutil**. Don't busy-wait; sleep or block
  (vsync, `poll`), or the cores never clock down and the battery drains.
- **A minimum clock for your own threads (uclamp):** schedutil picks the lowest clock that keeps
  a core ~80% busy, so a game that sleeps until just before vblank gets a clock at which its
  frame only just fits (a 3 ms frame at 1.99 GHz runs ~12 ms at ~600 MHz). Ask for a floor on
  the thread that runs the frame, not the whole process: `sched_setattr(0, &attr, 0)` with
  `sched_flags = SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_UTIL_CLAMP_MIN`
  and `sched_util_min` out of 1024 (512 ≈ half the top clock), no root needed. It holds
  only while that thread runs; the core still idles between frames. Set it back to 0 for menus
  and long idle stretches. Measured on the RG DS+: a 60 Hz loop doing ~3 ms of work ran at
  1104 MHz without a floor, at 1992 MHz with 768 or more, its work ~30% shorter.
- **Heat:** the CPU and GPU throttle from **75 °C** and are held under 85 °C (95 °C shuts down).
  Sustained full load reaches it in minutes; budget below the top clocks for long sessions.

## Performance on the A55

- Build with `-C target-cpu=cortex-a55` (Rust; set in the template's `.cargo/config.toml`) or
  `-mcpu=cortex-a55` (C/C++). NEON is always there.
- Profile-guided optimisation pays for hot loops (the emulator ships a PGO build); train it on the
  device.
- Memory: assume ~400 MB for an app; zram makes more possible but slow.
- Measure on the device, never guess: boot and wake-up tools in [bench/](bench/README.md); `perf` is
  installable with pacman.

## Building and shipping

- **Build in the build machine**, an Arch Linux ARM (aarch64) VM (OrbStack on a Mac;
  `BUILD_MACHINE` in `config.sh`): native aarch64, the same distribution as the device, so binaries
  run on the device as built. `orb -m "$BUILD_MACHINE" bash -lc 'cd <dir> && cargo build --release'`.
- **The device lock:** [`tools/device-lock.sh`](tools/device-lock.sh) is a lock for several
  programs or agents sharing one device. Take it before anything that changes the device or needs
  it quiet: installing, stopping or starting DSlot, games, benchmarks, deploys, reboots, suspend.
  Reading (journalctl, /proc) needs none. `tools/device-lock.sh take "WHO: WHAT"`, then `drop`
  when done, also on failure (a `trap … EXIT`). It is a directory on the device
  (`~/.cache/nibos/device.lock`, kept across reboots, so a test that reboots keeps it; `show`
  gives its age), so anything that can reach the device sees it. If `take` fails, someone holds
  it: wait and retry, and never remove another's lock (if it looks stale, check with whoever
  holds it). Then:
  - `tools/device-lock.sh game` before touching DSlot: if a game runs, someone is playing, so
    stop. The check must abort the run (`pgrep … && deploy` deploys anyway).
  - Bound every run with a timeout, start DSlot again in the same trap, and kill only by PID
    (`pkill -f PATTERN` over SSH can match its own shell).
  - Check the battery first (`nib battery`); no long runs on a low charge, stop below ~15%.
  - Check with the device's owner before playing sound, anything that could need a card swap,
    or a change to charging (which also needs the unplug tests in
    [docs/engineering/battery.md](docs/engineering/battery.md)).
- **The dev loop:** `sdk/run.sh [APP_DIR]` builds the app there, copies it to the device, stops DSlot,
  runs the app on the screens with its output in your terminal, and starts DSlot again when it
  exits. It refuses while a game is being played.
- **Package as an Arch package** (`PKGBUILD`; see [`hw/PKGBUILD`](hw/PKGBUILD) for the conventions:
  build from local sources copied in `prepare()`, `cargo fetch --locked` then `--frozen`, tests in
  `check()`). Install to `/usr/bin`; data to `/usr/share/<app>`; per-user state to
  `~/.local/share/<app>` or next to the game on `/mnt/roms`.
- **Saves:** games keep battery saves and states in `/mnt/roms/Saves/<SYSTEM>/<ROM name>/`.
  That folder is the **game's, not the emulator's**: every emulator (and every libretro core)
  that can play the system uses the same one, and the player may switch a game between them. So
  never overwrite another emulator's files: name resume states so they can't collide, or move
  another emulator's aside and bring yours back (inkdot keeps `.auto-dmg.state` /
  `.auto-cgb.state`, nibcase `.auto-<core>.state`); and load a battery save made elsewhere as
  far as it fits, keeping the original (`<name>.srm.<size>.bak`) before the first write. An
  emulator must never write battery RAM it hasn't loaded yet: that replaces the player's save
  with a blank one.
- **Permissions:** run as the login user (groups `video`, `input`, `audio`); never as root. Ask for root
  only through a package's install hooks.
- **Logging:** write to stderr; under systemd it goes to the journal (`journalctl -u <unit>` or
  `journalctl -t <name>`).
- **A background service** (not something that draws):

  ```ini
  # <app>: what it is for, in a sentence.
  [Unit]
  Description=What it does
  After=multi-user.target

  [Service]
  ExecStart=/usr/bin/<app>
  Restart=on-failure
  RestartSec=2

  [Install]
  WantedBy=multi-user.target
  ```

- **Starting from DSlot (optional).** A nibOS app doesn't need DSlot: it is a program that takes
  the screens, started however you like (`sdk/run.sh`, a unit, a shell). DSlot is the boot UI
  most devices run, and an emulator can appear on its shelf: an emulator describes itself with a file in
  `/usr/share/dslot/emulators/<id>.toml` (installed by its package): `name`, `priority`, and under
  `[systems]` a command (a list of words) for each system it plays (`NDS`, `GBA`, `GB`, `GBC`), with
  `{rom}`, `{name}`, `{system}`, `{saves}`, `{options}` filled in; optional `requires`, `[env]`,
  `[[seed]]`, `quit_on_output`. DSlot closes SDL (both panels and the audio device are free), runs
  the command as its own user inside `dslot.service` (with `SDL_VIDEODRIVER=kmsdrm`,
  `SDL_KMSDRM_ATOMIC=0`, `XDG_RUNTIME_DIR`, `LimitRTPRIO=10`), waits for it to exit, and takes the
  screens back. Take both panels yourself, exit to return; Menu is the conventional way back.
  Full contract: DSlot's [docs/emulators.md](https://github.com/abradburne/dslot/blob/main/docs/emulators.md).
  Any other app can appear on DSlot's **Apps tab** (shown when at least one app is installed, from
  either card): a folder `Roms/APPS/<folder>/` with the program and an `app.toml` (`name`,
  `command` as a list of words run from that folder, optional `description`, `icon` (default
  `icon.png`), `runs_on` (default `["nibos"]`), `[env]`). DSlot starts it the way it starts an
  emulator, and a held power key asks it to save and quit (SIGTERM) before the power menu; Cancel
  starts it again. `sdk/run.sh --install [APP_DIR]` builds the app and puts it, its `app.toml` and
  `icon.png` there; see [sdk/template](sdk/template/README.md). Full contract: DSlot's
  [docs/apps.md](https://github.com/abradburne/dslot/blob/main/docs/apps.md).

## Test checklist (on the device)

`tools/hwtest.sh` checks the hardware itself in about 30 s (`--suspend 3` for sleep, `--sound`
only with the device owner's OK); the list below is for your app.

- [ ] Both screens draw, at 59.94 Hz, nothing on the wrong panel; touch lands under the finger.
- [ ] Every control does what it should; Menu tap and Menu + another button behave; Menu + volume
      changes brightness and doesn't reach the app.
- [ ] Sound on the speakers and on headphones, no clipped starts.
- [ ] Suspend in the middle (close the lid), wake: the app carries on or recovers; Wi-Fi returns.
- [ ] A cold boot to the app (or to DSlot and into it); quitting returns to DSlot.
- [ ] No busy-wait: `top` shows the app idle when nothing changes.
- [ ] `journalctl` clean of your app's errors.

## Do and don't

- **Do** read `/usr/lib/nibos/board.conf` for device names; use `/run/nibos/...` for settings; present
  through SDL with vsync; keep heavy work off real-time threads; test on the device; say in your
  commit what you ran.
- **Don't** hard-code `/dev/input/eventN`, `backlightN` or `card0` connector numbers; drive KMS
  yourself; run as root; spin a core; write below the brightness floor; claim a test you didn't run.

## Shared code: nibkit

Use [nibkit](https://github.com/abradburne/nibkit) (Rust, MIT; nibkit-menu-c for C and C++)
instead of re-deriving the facts above: `screens::place` (which SDL display is which screen),
`vblank`, `input::printed` / `sdl_name` (labels vs SDL's positional names), `touch`, `backlight`
(percent with the floor), `device` (volume, Wi-Fi), `paths::AppDirs`, layered `settings`. Its
DSlot types sit behind the optional `dslot` feature, off by default: an app that doesn't use
DSlot doesn't need them.
`nibkit = { git = "https://github.com/abradburne/nibkit", tag = "v0.1.0" }`; the template shows
it in use.

## Changing nibOS itself

- **Crashes:** a boot UI (DSlot, or a frontend later) exits 0 when the player quits; anything
  else is a crash, which systemd restarts and, repeated, answers with the safe mode
  ([rootfs/safe-mode](rootfs/safe-mode/README.md)).
- **The boot path:** the Falcon partition holds the A/B slots and their boot control block;
  change them with `nibos-bootctl`, never by writing the partition
  ([design/ab-rollback.md](docs/design/ab-rollback.md)); `kernel/deploy.sh` writes slot A.
- **Check the hardware** with `tools/hwtest.sh` after a kernel or bootloader change.

`make check` must pass (shellcheck, board description, rustfmt/clippy/tests, Python, links). Read
[CONTRIBUTING.md](CONTRIBUTING.md): layout, names, style, patches.

## Contributing

AI-assisted contributions are welcome under these rules ([CONTRIBUTING.md](CONTRIBUTING.md#ai-assisted-contributions)):

- A human submits and is accountable: they read the change, ran it, and stand behind it.
- **Never add `Signed-off-by:` yourself.** The human adds it when submitting (`git commit -s`, the
  [DCO](https://developercertificate.org/)). Record your part with a `Co-Authored-By:` trailer.
- One logical change per pull request, a few hundred changed lines at most; no unrelated
  refactors, reformatting or renames. Features, new dependencies, and anything touching the
  bootloader, partitions or boot chain: open an issue first.
- Say what, why (with evidence), how, alternatives, risk, and how it was tested: `make check`, plus
  the board, build, commands and results on the device; before/after numbers for performance.
  Never claim a test or a device run that didn't happen; state uncertainty plainly.
- New behaviour comes with tests where it can be tested; a bug fix with a regression test.
- Match the surrounding code; comments say why; no dead code, debug leftovers, personal paths,
  hostnames or secrets; no vendored binaries or firmware; docs and CHANGELOG in the same change.
- A change that could stop a device booting says so at the top, with the way back.
