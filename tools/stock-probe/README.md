# nibos_probe: the stock-OS hardware probe

An app for the RG DS+'s **stock** firmware that reports the hardware: how much RAM, which
speaker amp, how many touch controllers, where the lid sensor is, whether USB-C host and deep
suspend work, which backlight is which screen. Its findings are in
[the RG DS+'s README](../../boards/rg-ds-plus/README.md#measured-on-the-stock-os).

`Roms/` mirrors the card: copy it onto the stock card's ROMS volume and run **nibos_probe**
from the stock menu's Apps. It asks a few things on screen (press a button, plug something in)
and writes its report to `Roms/APPS/nibos/probe-<time>.txt`. Everything is read-only except
two seconds of each backlight off, and an optional suspend test (`--suspend SECONDS`).

Over SSH it also runs as `bash probe.sh [--batch] [--suspend SECONDS]`.
