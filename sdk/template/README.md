# nibos-template

A nibOS app to start from, in one Rust file ([`src/main.rs`](src/main.rs)) on SDL2 and
[nibkit](https://github.com/abradburne/nibkit) (by git tag, in `Cargo.toml`):

- a fullscreen window on each screen, placed by `nibkit::screens::place` (on nibOS SDL display 1 is
  the top panel, 0 the bottom touch panel), each with an accelerated renderer that waits for vsync;
- the gamepad through SDL's game controller API: the stick or the D-pad move a square on the top
  screen, the button labelled A beeps, B changes its colour (`nibkit::input` maps labels to SDL's
  positional names), Select + Start quits;
- touch on the bottom screen: a dot under each finger;
- sound: a short beep queued to SDL's audio at 48 kHz (PipeWire's rate).

```bash
sdk/run.sh                    # build in the build machine, run on the device, DSlot back after
sdk/run.sh path/to/your-app   # the same for your copy
```

## Showing it in DSlot (optional)

[`app.toml`](app.toml) and [`icon.png`](icon.png) put the app on DSlot's Apps tab (DSlot's
[docs/apps.md](https://github.com/abradburne/dslot/blob/main/docs/apps.md)); a test parses
`app.toml` with `nibkit`'s `dslot` feature, as DSlot does. Without DSlot they do nothing.

```bash
sdk/run.sh --install                    # build, copy to Roms/APPS/nibos-template/, restart DSlot
sdk/run.sh --install path/to/your-app   # the same for your copy (give it its own app.toml)
```

Like every nibOS program, the template exits on SIGTERM (`bye`): DSlot sends it when power is
held, shows the power menu, and starts the app again on Cancel.

To start your own: copy this directory, rename the package in `Cargo.toml`, keep
`.cargo/config.toml` (Cortex-A55 code), and follow [AGENTS.md](../../AGENTS.md).

Tested on the RG DS+ on 2026-10-19 through `sdk/run.sh`: both windows open at 1024×768, the
gamepad is found, an injected D-pad press moved the square on the top screen and an injected touch
put a dot at the right place on the bottom one (both checked by capturing the screens'
framebuffers), Select + Start quit, and DSlot came back.

Tested from DSlot's Apps tab on 2026-10-19 (DSlot 648b326): `sdk/run.sh --install` listed it,
A started it, holding power (an injected 1.3 s press) made it exit with `bye` and showed the power
menu, and Cancel started it again.
