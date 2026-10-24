# SDK

What you need to build an app for nibOS, beyond [AGENTS.md](../AGENTS.md), which is the guide (for
people and AI agents alike).

| Path | What |
|---|---|
| [`template/`](template/README.md) | A Rust + SDL2 app to start from: both screens, the gamepad, touch and sound, tested on the RG DS+ |
| [`run.sh`](run.sh) | The dev loop: build in the build machine, run on the device's screens with DSlot out of the way, DSlot back after |

[nibkit](https://github.com/abradburne/nibkit) (MIT, its own repository; tag `v0.1.0`) is the
shared library for nibOS programs: where the screens are and their vblank, the buttons by their
printed labels, touch, brightness in percent, volume, Wi-Fi, app folders, layered settings, a
shared menu (with nibkit-menu-c for C and C++). DSlot's description types sit behind an optional
`dslot` feature, off by default. The template, DSlot and the inkdot emulator use it, by git tag
(`NIBKIT_REPO`, `NIBKIT_TAG` in [config.sh](../config.sh)).

The app platform is young: how apps share the screens, switch, and get packaged is being designed
in [docs/design/app-model.md](../docs/design/app-model.md); what comes after is in
[docs/roadmap.md](../docs/roadmap.md).
