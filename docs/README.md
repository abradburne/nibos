# Documentation

The reference documents describe nibOS as it is. Each directory of the repository also has a
README that is its own reference.

## Using and building

| | |
|---|---|
| [architecture.md](architecture.md) | The layers; generic, SoC and board; the boot chain; the SD card; what is where on the device |
| [install.md](install.md) | From a stock card to nibOS alone or the whole package (DSlot, the emulators), each step checked; back to stock |
| [building.md](building.md) | From an empty Mac to a booting device, in order |
| [releasing.md](releasing.md) | How a release is made, in order: the trees and the tag, `tools/release-build.sh`, signing, publishing, the OTA check, the site |
| [using.md](using.md) | Wi-Fi, logging in, controls and power, the boot UI, the network modes |
| [recovery.md](recovery.md) | When the device doesn't start, from the A-held fallback to a whole card from a backup |
| [boot-time.md](boot-time.md) | How it boots fast: each phase, what made it fast, what didn't pay, what's left |

## Plans

| | |
|---|---|
| [roadmap.md](roadmap.md) | What nibOS needs next to be a good base for building on the RG DS+, in priority order |
| [design/ab-rollback.md](design/ab-rollback.md) | A/B slots and rollback: the design, what phase 1 built, how it was tested |
| [design/app-model.md](design/app-model.md) | Screen ownership and the app model (exclusive handover, a compositor, leases) |
| [design/images.md](design/images.md) | The downloadable images: editions, first-boot provisioning, what they contain |
| [../AGENTS.md](../AGENTS.md) | The guide for building apps, for people and AI agents |

## Engineering

| | |
|---|---|
| [engineering/patches.md](engineering/patches.md) | Every kernel and U-Boot patch: what, why as measured, upstream status; how to change one |
| [engineering/battery.md](engineering/battery.md) | The battery and charging: the RK817's limits, battery protect, the unplug tests |
| [engineering/idle-power.md](engineering/idle-power.md) | What wakes the device when nothing happens, and what it draws: findings and open items |
| [../rootfs/safe-mode/README.md](../rootfs/safe-mode/README.md) | Crash recovery: DSlot's restarts, the safe mode, the watchdog, panics |
| [../tools/README.md](../tools/README.md) | The tools: the device lock, the hardware test suite, images, checks |
| [engineering/suspend-and-wifi.md](engineering/suspend-and-wifi.md) | Deep sleep and Wi-Fi: the two bugs that looked like one, the USB host controller, how to test |
| [engineering/display.md](engineering/display.md) | The two panels: the JD9165 driver, bringing them up fast, 59.94 Hz, who draws on them |
| [engineering/audio.md](engineering/audio.md) | Two sound cards as one output, the amps' keep-alive, volume keys |
| [engineering/input.md](engineering/input.md) | One gamepad from the raw devices, Menu + volume for brightness, keys off the gamepad, sleep and wake, latency |
| [engineering/device-interface.md](engineering/device-interface.md) | How programs change device settings without root: the pattern, and which settings are done or planned |
| [../uboot/README.md](../uboot/README.md) | The bootloader and Falcon mode |
| [../hw/README.md](../hw/README.md) | The hardware layer |
| [../boards/rg-ds-plus/README.md](../boards/rg-ds-plus/README.md) | The RG DS+: how it differs from the RG DS, the stock files, measured facts, status |
| [../bench/README.md](../bench/README.md) | The benchmarks and their results |
