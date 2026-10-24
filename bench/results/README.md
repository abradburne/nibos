# Results

Measurements on the RG DS+, as reference tables: what was measured, on what hardware, with
which build, when, and how. Each table says how many runs it is (three boots or more for boot
times) and gives the range. Newer results go on top of each file with their date; old ones
stay, so a regression shows.

| File | What |
|---|---|
| [boot.md](boot.md) | Reset to kernel, init, shell, SSH and fully booted, against the stock firmware |
| [boot-ui.md](boot-ui.md) | The splash and DSlot's shelf on both screens |
| [network.md](network.md) | The three network modes |
| [bootloader.md](bootloader.md) | The bootloader's stages and how each change moved them |
| [suspend-and-wifi](../../docs/engineering/suspend-and-wifi.md) | Deep sleep and Wi-Fi's return (kept with the fixes they test) |
