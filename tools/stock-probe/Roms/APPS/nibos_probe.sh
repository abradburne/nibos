#!/bin/bash
# nibos_probe: run from Apps on the stock RG DS+ menu. Checks the hardware facts nibos
# needs (RAM, speaker amp, touch, lid, USB-C, screens, sleep), asking you a few things on
# screen. The report goes to Roms/APPS/nibos/probe-<time>.txt.
exec bash "$(cd "$(dirname "$0")" && pwd)/nibos/probe.sh" --screen
