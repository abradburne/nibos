# Display: two panels

The RG DS+ has two 1024×768 MIPI-DSI panels, each on its own DSI controller and its own VOP2
video port, so each is a separate CRTC: `DSI-1` is the bottom (touch) panel, `DSI-2` the top.
There is no external display output on the board.

## The panels

Each panel is driven over two lanes by a Jadard JD9165 controller. Mainline has no driver for
it; the JD9165 shares the JD9365's power and reset sequence, so it is added to mainline's Jadard
driver with its own init function (kernel `rg-ds-plus/0001`): it switches register pages with
0x30 and is unlocked with 0xf7. The init commands, the timings (1304×800 total, 62.8 MHz,
60.2 Hz) and the lane setup are the stock firmware's, decoded from its device tree's
`panel-init-sequence` properties, where they were generic long writes. The top panel is mounted
upside down and flips its scan direction with MADCTL (0x03); the driver tells the two apart by
their compatibles (`anbernic,rg-ds-plus-display-top`, `-bottom`). Backlights: pwm1 (bottom) and
pwm2 (top).

## Bringing them up fast

Each panel's power-on is ~0.4 s of sleeps: reset (5 + 10 + 130 ms), ~80 ms of init commands at
LP speed, sleep-out (120 ms), display-on (20 ms). With the driver built in and unpatched, both
run one after the other inside the driver's initcall, then inside DRM's first modeset, and
everything after waits: 0.81 s of a 2.03 s kernel start. Three changes take them off the path:

- the driver probes asynchronously (kernel `rg-ds-plus/0002`);
- both panels reset together in the probe, and prepare stops after sending sleep-out: the rest
  of its 120 ms is waited out in a new enable hook, so the second panel is initialised while
  the first one waits (kernel `rg-ds-plus/0003`). The panels come on at 0.55 and 0.61 s into
  the kernel instead of 0.70 and 0.84 s;
- the first modeset runs from a work item instead of inside the display driver's bind (kernel
  `rk3568/0003`), and the console is opened after the root mount (kernel `generic/0001`),
  since fbcon holds the console lock while it powers the panels on.

To check the panels after changing their bring-up, read each panel's DCS power mode back after
display-on: 0x14 means sleep out, display on.

## Refresh rate: 59.94 Hz

The panels' pixel clocks (`dclk_vop0/1`) come from VPLL and can't change its rate, so the
refresh rate is whatever VPLL divides to. The RG DS tree's 292.5 MHz gave 58.5 MHz instead of
62.8 MHz: **56.08 Hz**, and everything synced to the display (DS games among it) ran at 94%
speed. The DS+ tree sets VPLL to 1188 MHz, which divides to 62.526 MHz: **59.937 Hz** on both
CRTCs, measured with [`tools/device/vblank.c`](../../tools/device/vblank.c), next to the DS's 59.83 Hz.
[boards/rg-ds-plus/README.md](../../boards/rg-ds-plus/README.md#panel-refresh-rate-vpll-at-1188-mhz)
has the arithmetic and the other candidate rates.

The two panels' vblanks are **8.2 ms apart** (the top panel is DRM pipe 1, the bottom pipe 0).
An emulator that times its frames to one panel's vblank gets each frame to that panel sooner;
for DS games the top panel is the one to follow, since most games answer a button there.

## Who draws on them

At power-on the kernel's console is on both panels (blank under `quiet`); with the boot UI,
[the splash](../../rootfs/splash/README.md) puts the DSlot logo up through KMS dumb buffers at
1.5 s from reset and hands the CRTCs to DSlot's first GL frame. DSlot, and the emulators it
starts, use SDL's KMSDRM backend with one window per panel on GLES (Panfrost). SDL3's atomic
KMSDRM path has no plane for the second panel and crashes tearing its window down, so programs
that open both panels set `SDL_KMSDRM_ATOMIC=0`; the legacy path drives both.

## Brightness

Each panel has its own PWM backlight (`backlight0` on pwm1 for the bottom panel, `backlight1`
on pwm2 for the top, enabled through GPIO4_A4 and GPIO4_A3), 1666 levels which pwm-backlight
spreads on a perceptual curve (`scale` = `non-linear`), both at half when the kernel starts. The hardware layer names them by screen, opens them to the
video group, keeps each one's level across restarts with a floor, and gives UIs a perceptual
percent ([hw/README.md](../../hw/README.md), "Brightness").

## Known issue

**A warm restart can leave the top panel striped** (alternate lines, even on the text
console); a cold power-on is clean. It is the JD9165's init on a panel that kept its power
across the restart. Not fixed.

## Dead ends: don't repeat

Tried and measured, or found not to work; the source says more. Add to it when something doesn't pay.

- The RG DS tree's VPLL (292.5 MHz): 56.08 Hz and DS games at 94%; VPLL 1188 MHz.
- SDL3's atomic KMSDRM path with two panels: no plane for the second, segfaults; `SDL_KMSDRM_ATOMIC=0`.
- Closing the DRM card during a handover: the text console comes back for ~0.5 s; keep it open.
- PanVK on the G52: not conformant, ~5x slower than GL; not installed by default.
- An emulator driving its own KMS page flips was blamed for the suspend hang; it wasn't the cause.
- Open: a warm restart leaves the top panel striped.

## Ideas not yet done

### An on-screen indicator for brightness and volume

Menu + volume changes both screens' brightness and the volume keys change the volume from
anywhere, but nothing on the screens shows the new level: in a game the light changes and the
volume shows not at all. An indicator (a bar for a second) has to draw over whatever program
owns the screens, which nibOS's daemons don't. Two routes:

- **A KMS overlay plane** from the daemon that made the change (`nibos-input`, `nibos-audio`),
  composed by the display hardware over the program's own plane. Needs a free overlay plane on
  each VOP2 video port that the program doesn't use, DRM master or a lease for the daemon while
  the program holds the screens (one DRM master at a time, so likely a lease the program
  grants, or the daemon as master handing the primary planes out), and care that SDL's legacy
  KMSDRM path doesn't take the planes over. First step: measure which VOP2 planes are free
  while DSlot and an emulator run.
- **A small protocol programs opt into**: the daemon announces "brightness 45%" or "volume 20%"
  (a socket or D-Bus signal, through the planned device service in
  [device-interface.md](device-interface.md)), and the boot UI and the emulator draw it
  themselves. Simple and safe, but every program has to do it.

### Mesa: keep cached buffer objects mapped

A GL client that renders in several passes a frame cycles transient buffer objects through
Panfrost's BO cache with mmap/munmap and a SET_LABEL ioctl each time: ~7 × 8 syscalls a present,
measured in a 3D emulator's hi-res renderer. A small patch to nibOS's Mesa build
([rootfs/mesa/](../../rootfs/mesa/README.md)) that keeps cached BOs mapped and labels them once
would cut that for every GL client. Worth doing when a GL client's profile shows it.
