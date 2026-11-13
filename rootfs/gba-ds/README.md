# GBA DS

[GBA DS](https://github.com/kyannon/gba-ds) plays Game Boy Advance games with the game on one
screen and its map, race HUD or status screen on the other. On nibOS its games are on DSlot's
**GBA DS** page, after the GBA page: the games in `Roms/GBA` that GBA DS has a second screen
for (DSlot's `docs/gba-ds.md`).

| Where | What |
|---|---|
| `/usr/lib/gba-ds/gba-ds` | GBA DS 1.5 built from nibOS's fork, with nibOS's pause menu ([PKGBUILD](PKGBUILD): the commit and SHA-256) |
| [`/usr/lib/gba-ds/run-game`](files/usr/lib/gba-ds/run-game) `[options] <game> <saves folder>` | Starts one game as nibOS needs: what DSlot runs |
| [`/usr/share/dslot/emulators/gba-ds.toml`](files/usr/share/dslot/emulators/gba-ds.toml) | DSlot's description: the `GBADS` system, three options, "Clear resume state" |
| `Saves/GBADS/<game>/` | The game's battery save (`.sav`), resume point (`.auto.state`) and save states |
| `~/.local/share/gba-ds/` | GBA DS's other files: settings from its menus, cheats, patches, screenshots |

## What run-game does differently from GBA DS's launch.sh

`launch.sh` is written for ROCKNIX, BananaOS and the stock OS. On nibOS (GBA DS 1.5, tried on
the RG DS+ on 12 Nov 2026):

- **Both panels:** SDL's KMSDRM driver with a window on each panel, and SDL's atomic KMS path
  off (`SDL_KMSDRM_ATOMIC=0`). With it on, SDL3 sets up a plane only for the first window's
  display, and GBA DS crashed creating the second window. DSlot and DSperate set the same.
- **Menu:** `nibos-input` sends Menu as `BTN_MODE`, raw button 10, which GBA DS doesn't take
  as HOME unless told: `--home-button=10`. run-game also asks for nibOS's own pause menu on
  Back (`--menu=nibkit --back-button=menu`), Menu to save the resume point and quit
  (`--home-button-action=quit`) and no swipe-up quick bar (`--quick-bar=0`). Those options are
  in nibOS's fork (the `nibos` branch of abradburne/gba-ds), which the package builds; they are
  offered upstream.
- **Touch:** read straight from the touchscreen's device, as the bottom panel's own.
- **No bezels** (`--overlays=off`) and **no update check**: the package is how it is updated.
- **SIGTERM** (DSlot's power menu, shutdown): GBA DS saves the resume point and exits.

## Licences and source

GBA DS is GPL-3.0-only. It links libmgba (MPL-2.0), rcheevos (MIT) and nibkit's menu (MIT, its
face Roboto Condensed OFL-1.1) statically and draws its own text in Jost (OFL-1.1); their
licences are in `/usr/share/licenses/gba-ds/`. The PKGBUILD builds from the fork's commit and
the mGBA commit GBA DS's `scripts/mgba-options.sh` pins; `tools/collect-sources.py` archives
them with the rest of a release's source.
