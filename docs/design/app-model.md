# Screens and the app model

How programs get the RG DS+'s two screens, give them back, and quit. The app platform is young;
what comes after is on the [roadmap](../roadmap.md#3-app-platform-and-sdk).

## How apps run today

The RG DS+ has two 1024×768 panels, each its own CRTC on the RK3568's VOP2. **One program owns
both at a time**, by being DRM master: the splash at power-on, then DSlot, then the game or app
DSlot starts.

- **Handover.** Choosing a game or app, DSlot closes SDL (letting go of both panels and the
  audio device), runs the emulator's or app's command and waits for it, then takes the screens
  back when it exits ([rootfs/dslot](../../rootfs/dslot/README.md#games)). `nibos-splash --hold`
  can keep a picture up across the handover with no dark frame
  ([rootfs/splash](../../rootfs/splash/README.md)).
- **Drawing.** Apps run full-screen through SDL's KMSDRM backend, two displays (display 0,
  DSI-1, is the bottom touch panel), as [AGENTS.md](../../AGENTS.md) describes. A game gets the
  whole GPU and the shortest path to the panels.
- **Switching is quit and resume.** No two apps stay alive at once on the RG DS+'s 1 GB.
  **SIGTERM means save and quit**: an app saves its state and exits, and resumes fast the next
  time it starts. Holding power opens DSlot's power menu, which puts a running game away this
  way (SIGTERM, up to 10 s) before showing Sleep / Restart / Power off / Cancel; Cancel resumes
  the game ([hw/README.md](../../hw/README.md)).
- **DSlot is a client**, not a built-in home screen: the shelf is one program among others.
- **Only the system draws above a game.** Today nothing does: only the owner can draw.

## What any change must keep

Measured today:

- Games: frames at 59.94 Hz on both panels, button-to-screen latency no worse than now
  (~60–100 ms median in the emulator), no extra frame of delay in fullscreen.
- GPU: the Mali-G52 is fill-rate and bandwidth bound; a full-screen copy per frame per screen
  (2 × 1024×768×4 bytes at 60 Hz, ~375 MB/s) is a real cost here.
- Boot: the shelf at ~5.2 s from reset; anything that starts before DSlot adds to it.
- Memory: 984 MB in all, ~560 MB free with the shelf up.
- Suspend: nothing may hang it (an app's own KMS page flips once did).

## Later

- **A supervisor in nibOS.** The handover moves from DSlot into a small nibOS supervisor with a
  documented contract: start an app, hand it the DRM device, take it back when it exits or
  crashes, restore the screens. Part of the contract: which signal means save and quit, how
  long an app has, and what "resume fast" must mean. Apps keep SDL on KMSDRM. Today a crashing
  app leaves the screens as it left them, and the way back is DSlot's.
- **The system above an app**: an indicator for brightness and volume, battery warnings, a
  system menu on Menu held. App notifications later, routed through the system. This needs one
  of:
  - **a minimal Rust compositor** (`nibos-shell`, on smithay) owning KMS and both panels, the
    shelf and every app Wayland clients (SDL's Wayland backend, chosen by environment, not by
    code). With **direct scanout** a full-screen game's buffer goes to the panel with no copy
    and no extra frame; composition happens only while something is drawn above it. It would
    also give switching without quitting, two programs on two screens, and crash recovery.
    To measure first, with a prototype: frame times and button-to-screen latency against
    KMSDRM, whether Panfrost and VOP2 give direct scanout of a full-screen client on both
    panels, the compositor's start time and memory. It goes ahead only if a full-screen game
    costs no extra frame and the shelf stays at ~5.2 s.
  - **DRM leases**, if direct scanout doesn't work out: a supervisor keeps DRM master, leases
    the CRTCs, connectors and primary planes to the app, and keeps an overlay plane per screen
    for itself (if VOP2 has one free per video port; not measured). SDL's KMSDRM backend has no
    lease support, and switching would need every app to survive having its lease revoked.
- **Real switching** (apps kept alive in the background) stays possible later, memory allowing.
