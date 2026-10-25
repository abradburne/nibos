# nibOS

A Linux system of its own for small handhelds: mainline Linux and U-Boot, a small hardware
layer, Arch Linux ARM, and a boot UI that starts the games the devices were made for.
It treats a handheld as a pocket Linux computer that happens to have game controls, rather
than a games console that happens to run Linux.

The first board is the **Anbernic RG DS+**, a clamshell with two screens. On it, nibOS boots
from power-on to a picture on both screens in 1.5 s and to its game shelf in about 5 s, and
everything on the board works, including deep sleep with Wi-Fi back on wake.

| | |
|---|---|
| Board | Anbernic RG DS+ (RK3568). Other handhelds are a goal for later; the layout is ready for them ([boards/](boards/README.md)) |
| Bootloader | Mainline U-Boot v2026.10, SPL in Falcon mode: the kernel starts **0.65 s** after reset |
| Kernel | Mainline Linux 7.3-rc7, the board's device tree, thirteen patches (panels, boot time, Wi-Fi suspend, charging, the speakers after a wake, the watchdog) |
| Userland | Arch Linux ARM with systemd, trimmed until init to a shell takes 2.5–3 s |
| Boot UI | DSlot's game shelf on both screens; the emulators it starts are separate projects |
| Recovery | DSlot restarted after a crash, then a safe mode; the hardware watchdog; A/B slots that fall back from a bad update by themselves; hold A for the fallback ([recovery.md](docs/recovery.md)) |
| Updates | Signed releases written to the slot that isn't running and kept only once they come up: DSlot's System → Update page, or `nib update` ([design/updates.md](docs/design/updates.md)) |

Everything here is written to be read: each script, patch and unit says what it does and why,
and the measurements behind it are kept, with how to take them again.

## Why nibOS?

Handheld firmware usually means the vendor's kernel and bootloader, frozen at the version the
device shipped with, with a launcher on top. nibOS is for people who want more of the device:

- **Mainline Linux and U-Boot**, not a vendor BSP. Fixes keep arriving, our own patches are
  written to go upstream, and the device stays supported after the vendor stops.
- **Open graphics**: Panfrost and Mesa (GLES 3.1, with PanVK as an optional Vulkan package),
  which is what makes higher-resolution rendering and latency work possible.
- **The RG DS+ fixed where stock gets it wrong**: deep sleep with Wi-Fi back on wake, the
  battery level from the PMIC's coulomb counter (stock shows 100% at about 70%), battery protect
  and safe charging ([battery.md](docs/engineering/battery.md)), a clean hand-over from the
  splash to the boot UI, and a speaker tuning of our own that can be redistributed.
- **A real computer**: Arch Linux ARM packages, SSH, a keyboard, development tools; not only a
  host for a launcher.
- **A platform to build on**: [nibkit](https://github.com/abradburne/nibkit), [AGENTS.md](AGENTS.md),
  the template app, one interface for device control, a shared menu.
- **Images you can redistribute**: nothing in them comes from the stock firmware.
- **Fast anyway**: a picture at 1.5 s and the game shelf at about 5 s from reset, with a full
  system behind it.

### Why not BaseOS (if it supported the DS+)?

[BaseOS](https://github.com/pvaibhav/BaseOS) is a fast, minimal BusyBox base for Anbernic's
H700 handhelds, on Anbernic's own bootloader and kernel, and frontend-agnostic. It does well
what it sets out to do: it is smaller and simpler than nibOS, supports many devices, and runs
NextUI, Slot and spruceOS. If what you want is a lean base for an existing launcher, it is a
good choice.

nibOS aims at something else: the mainline stack, open graphics, a full Linux userland and a
platform for new apps, on one device at a time, done thoroughly. It doesn't want to be
DSlot-only: a frontend contract, so that any launcher can be the one nibOS boots, is on the
[roadmap](docs/roadmap.md).

## How it is made

nibOS is built largely with AI coding agents, and every change is tested on a real RG DS+ by a
person; nothing here is claimed to work that wasn't run on the device. AI-assisted
contributions are welcome under the rules in [CONTRIBUTING.md](CONTRIBUTING.md#ai-assisted-contributions).

## Building for nibOS (and for AI agents)

[AGENTS.md](AGENTS.md) is the guide to writing an app for nibOS, for people and AI agents alike:
the device's facts, graphics on two screens, input, audio, device settings, power and real-time
rules, building and the dev loop, a test checklist. [`sdk/template/`](sdk/template/README.md) is a
Rust + SDL2 app to start from (both screens, the gamepad, touch, sound) that runs on the device as
it is, and `sdk/run.sh` runs yours there. Where nibOS goes next: [docs/roadmap.md](docs/roadmap.md).

## Documentation

| | |
|---|---|
| [Architecture](docs/architecture.md) | The layers, generic / SoC / board, the boot chain, the SD card, what is where on the device |
| [Installing](docs/install.md) | From a stock card to nibOS alone or the whole package, step by step, as it works today, and back to stock |
| [Building](docs/building.md) | From an empty Mac to a booting device, in order |
| [Using](docs/using.md) | Wi-Fi, logging in, the boot UI, the network modes, `nib` |
| [Recovery](docs/recovery.md) | When it doesn't start: the A-held fallback, the logs on the card, back to stock |
| [How it boots fast](docs/boot-time.md) | Each phase, what made it fast, what didn't pay |
| [Engineering notes](docs/README.md#engineering) | The patches and why each exists, suspend and Wi-Fi, display, audio, input |
| [Benchmarks](bench/README.md) | Every measurement, how to take it again, and the [results](bench/results/README.md) |
| [Boards](boards/README.md) | What a board provides, what works, how to add one |
| [Building apps](AGENTS.md) | The guide for app developers and AI agents, with the [template app](sdk/template/README.md) |
| [Roadmap](docs/roadmap.md) | What nibOS needs next to be a good base for building on the RG DS+ |
| [Contributing](CONTRIBUTING.md) | Layout, style, checks, names, how to change a patch, the rules for AI-assisted contributions |
| [Changelog](CHANGELOG.md) | What each release brings |

## Repository layout

| Directory | What | Read |
|---|---|---|
| `boards/` | One directory per board: device tree, patches, config fragments, hardware files, the card's layout | [boards/README.md](boards/README.md) |
| `socs/` | What the boards on one SoC share: patches, config, hardware files | [socs/README.md](socs/README.md) |
| `uboot/` | Mainline U-Boot: build, Falcon FIT, install and recovery, boot-stage tools, generic patches | [uboot/README.md](uboot/README.md) |
| `kernel/` | The kernel: build, deploy, boot images, generic patches and config, smoke test | [kernel/README.md](kernel/README.md) |
| `hw/` | `nibos-hw`, the hardware layer: Rust daemons and the files that make a board a device | [hw/README.md](hw/README.md) |
| `rootfs/` | The Arch Linux ARM image and its boot-time trimming; DSlot as boot UI, splash, read-ahead, Mesa | [rootfs/README.md](rootfs/README.md) |
| `bench/` | The boot and network benchmarks, and the reference results of all of them | [bench/README.md](bench/README.md) |
| `sdk/` | The template app and the dev loop for building apps; the SDK library, nibkit, is [its own repository](https://github.com/abradburne/nibkit) | [sdk/README.md](sdk/README.md) |
| `tools/` | The shared script library, the checks, the image build, card backups, device tools | [tools/README.md](tools/README.md) |
| `docs/` | The documents above | [docs/README.md](docs/README.md) |

Settings for your own machine and device go in `config.local.sh` (see [config.sh](config.sh)).
`make check` runs every check the repository has (shellcheck, board descriptions, rustfmt,
clippy, unit tests, Python, documentation links).

## License

nibOS's own code is free software under the **GNU General Public License, version 3 or later**
([LICENSE](LICENSE)): the hardware layer (`hw/`, `nibos-hw`, `nib`), the tools, the image build,
the root file system's scripts and units, the splash and the safe mode. Anyone may use, study,
change and share it; whoever ships it, changed or not, ships its source too. The parts that keep
another licence:

| Part | Licence | Why |
|---|---|---|
| `sdk/template/` | MIT | The code an app starts from, so it doesn't decide the app's licence |
| Kernel patches (`kernel/`, `socs/*/kernel/`, `boards/*/kernel/patches/`) | GPL-2.0 | Like the kernel they patch |
| U-Boot patches (`uboot/`, `socs/*/uboot/`, `boards/*/uboot/patches/`) | GPL-2.0-or-later | Like U-Boot |
| `boards/rg-ds-plus/dts/` | GPL-2.0-or-later OR MIT | As its SPDX line says: it derives from mainline's RG DS tree |
| `rootfs/portmaster/` profile files | MIT | PortMaster's licence: they derive from its platform files and are meant for it upstream |

Not in the repository or any package, and not ours to redistribute: the files a board's
`extract-stock.sh` takes from your own stock image (on the RG DS+: Anbernic's device tree,
Awinic's amplifier tuning file, the stock boot partition), Rockchip's DDR init and TF-A
binaries (fetched from rkbin at a pinned commit), and the Wi-Fi and Bluetooth firmware (from
`linux-firmware`). DSlot (the boot UI) and the emulators are separate projects with their own
repositories, packages and licences; nibOS starts them but doesn't contain them.
[nibkit](https://github.com/abradburne/nibkit), the SDK library (with nibkit-menu-c for C and
C++), is MIT, its fonts under the SIL Open Font License 1.1; nibOS's crates depend on it by git
tag (`NIBKIT_REPO`, `NIBKIT_TAG` in [config.sh](config.sh)), and `tools/make-image.sh` fetches
it for the packages built from other repositories. A downloadable
image lists everything in it with its licence in `NOTICE` ([design/images.md](docs/design/images.md)).
