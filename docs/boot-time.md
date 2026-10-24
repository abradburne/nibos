# Boot time

From power-on, nibOS on the RG DS+ puts a picture on both screens at 1.5 s, the game shelf at about 5.2 s,
and a shell on the console at about 4.1 s. The stock firmware's console comes up at about 7.6 s.
This page covers each phase, what makes it fast, and what was tried and didn't pay. The
measurements, with dates and builds, are in [bench/results/](../bench/results/README.md).

| Seconds from reset | Stock firmware | nibOS (tty1 shell) | nibOS (DSlot boot UI) |
|---|---|---|---|
| Kernel starts | ~2.6 | **0.65** | 0.65 |
| Kernel starts init | ~4.5 | **1.55** | 1.55 |
| A picture on both screens | | ~1.35 (the console, blank under `quiet`) | **1.5** (the splash) |
| Shell on tty1 / the shelf on GL | 7.6 (login) | **4.05–4.58** | **5.2–5.4** |
| Fully booted (systemd) | ~10.2 | 5.21–5.49 | |

nibOS figures: mainline U-Boot in Falcon mode, network mode `deferred`, 3 boots each.

## Measuring

[`bench/boot.sh`](../bench/boot.sh) restarts the device over SSH and reports each phase. The clocks:

- **The arch counter**, which TF-A restarts, gives the kernel's start (counter minus
  `CLOCK_BOOTTIME`). Everything after that is `CLOCK_BOOTTIME` or systemd's monotonic stamps,
  which count from the kernel's start.
- **Before TF-A**, mainline U-Boot leaves its bootstage records in RAM at `0x0f000000` (the
  DTS reserves it), and `bootstage.py` reads the DDR init and SPL time from them, which makes
  every figure seconds from reset.
- **Not journald's kernel stamps.** journald makes up times for lines logged before it starts
  (dmesg's 0.12 s showed as 3.47 s), so figures anchored on them can only be compared with
  each other.

"Console" is when the tty1 shell process starts (the prompt you can type at), not when systemd
starts getty. "SSH" is when a client can actually connect: sshd listening and Wi-Fi associated.
`bench/boot-ui.sh` adds DSlot's own marks (splash, skeleton, first frame on both screens),
`uboot/measure.sh` the bootloader's stages, `bench/network.sh` the network modes
([bench/README.md](../bench/README.md)).

## Bootloader: 0.65 s

Mainline U-Boot in Falcon mode: SPL brings the card up at UHS SDR104 and loads TF-A and the
uncompressed kernel directly. Details in [uboot/README.md](../uboot/README.md). What each
change is worth:

| Step | Kernel starts |
|---|---|
| Stock bootloader (Rockchip's TPL, SPL, TF-A, OP-TEE, U-Boot) | ~2.6 s |
| Mainline U-Boot v2026.10 as first installed | ~3.3 s |
| Data cache before relocation, a real 50 MHz SD clock, no UEFI, short partition scan, fast decompressors, CPU at 1608 MHz | 1.32 s |
| The same with a 7.57 MB kernel instead of 11.75 MB (each MB of `Image.gz` costs ~0.04 s) | 1.04 s |
| U-Boot reads the card at UHS SDR104 (67 MiB/s instead of 22.8) | 0.89 s |
| Falcon mode, SPL at UHS SDR104 | **0.65 s** |

Image.zst and Image.lz4 lost to gzip in U-Boot proper (1.15 s and 0.35 s to unpack against
gzip's 0.28 s). Falcon reads the kernel uncompressed: SPL can't unpack it quickly.

## Kernel: 0.9 s to init

From the kernel's start to init takes about 0.9 s (2.0 s untuned). An `initcall_debug` boot
shows where the time went, and almost all of it was waiting:

- **The panels.** Attaching the second panel binds the display pipeline, and the first modeset
  powered both panels on one after the other, ~0.4 s of sleeps each, with every initcall after
  it waiting. The panel driver probes asynchronously (kernel `rg-ds-plus/0002`), both panels reset
  together in probe and the second one is initialised while the first waits out its sleep-out
  time (`rg-ds-plus/0003`), the first modeset runs from a work item instead of inside the display's probe
  (`rk3568/0003`), and the console is opened after the root mount, because fbcon holds the console lock
  while it powers the panels on (`generic/0001`). The root mount now overlaps the panels.
- **The SD controllers** deferred until late initcalls, waiting for the PMIC that feeds their
  1.8 V signalling. The I2C controller, PMIC and regulators now register at `subsys_initcall`
  (`rk3568/0001`), and the DTS drops a wrong supply on the card's 3.3 V switch: the root card starts at
  0.26 s instead of 1.29 s.
- **SDR104 tuning** took 0.03–0.51 s: a phase whose tuning block never arrives waited 112 ms
  each. A 10 ms timeout while tuning (`rk3568/0002`) and 90 phases instead of 360
  (`rockchip,desired-num-phases`) bring it to 24–100 ms.
- **Size.** A Rockchip-only configuration: `Image.gz` 11.75 → 7.57 MB.
- The ICM42607 IMU driver, which binds nothing on the DS+, is a module (0.1 s), and
  `rootfstype=ext4` saves trying ext3 and ext2 first.

Init starts when fbcon lets go of the console, with the root file system mounted ~0.1 s before.

## Userspace: init to a shell in 2.5–3 s

[`rootfs/tune-boot.sh`](../rootfs/tune-boot.sh) holds the trimming, each with its reason. In
order of what it saves:

| Change | Why it was slow |
|---|---|
| Lingering user; tty logins are `user-light` sessions; a trimmed `/etc/pam.d/login` | logind held the login until the user manager was up, started through a PID 1 still busy booting: getty → shell 2.3 s → 0.6–1.0 s |
| getty is `Type=simple`, not `idle` | `idle` held agetty until every other start-up job was done, 2 s here |
| Console size on the command line | PID 1 asks the console its size with a query the Linux VT never answers, and waits 333 ms |
| `SYSTEMD_DEFAULT_MOUNT_RATE_LIMIT_BURST=20` | The early mounts are more than 5 a second, and PID 1 then held every mount job ~0.7 s |
| Coldplug only devices, minus classes no rule matches | `udevadm trigger --type=all` replays 911 driver, module and bus events: ~1.35 s of udevd CPU during sysinit |
| `group: files systemd` in nsswitch.conf | Arch's merge asks userdbd and PID 1 even for groups `/etc/group` has: 122 varlink calls from tmpfiles alone |
| resolved, timesyncd, udevd, utmp, backlight and the journal flush no longer ordered before `sysinit.target` | Nothing that early needs them, and each held everything after sysinit (0.5–0.7 s each) |
| The stock OS's ten partitions lose their `systemd` udev tag | Half of the 223 device units PID 1 built, and the user manager enumerated again |
| 13 generators masked; no vconsole setup; hugepages and fusectl not mounted | Each is a program read from the card before any unit starts |
| sshd per connection from a socket | ~1 s of start-up while the console's login ran |

Still before the first unit, not fixable from the root file system: ~0.18 s verifying systemd's
socket-bind BPF programs (only `CONFIG_CGROUP_BPF` off, which costs systemd's device and IP
access control, avoids it).

**The network is not on the boot path** unless the edition wants it there. The Wi-Fi and
Bluetooth drivers are blacklisted for udev and loaded by `nibos-network.target`, which an
edition starts at boot (`boot`), once start-up is done at the lowest priority (`deferred`), or
only when something asks (`demand`). See [hw/README.md](../hw/README.md).

## The boot UI: a picture at 1.5 s, the shelf at 5.2 s

With DSlot as the boot UI ([rootfs/dslot/](../rootfs/dslot/README.md)), what you see is:

1. **1.5 s: the splash.** The kernel runs [`nibos-splash`](../rootfs/splash/README.md) as
   `init=`. It forks: the parent execs systemd at once, the child puts the DSlot logo on both
   CRTCs through KMS (dumb buffers, 20–50 ms) and stays until DSlot's first GL frame replaces
   it. Drawing before the exec held systemd back ~0.1 s; forked, it costs the shelf nothing
   measurable.
2. **~2.7 s: DSlot starts**, from `sysinit.target`, with no getty, login, PAM or user manager.
3. **5.2–5.4 s: the shelf on GL, on both screens.** What it waits for is GL's cold start:
   - Mesa built with **only Panfrost and no LLVM** ([rootfs/mesa/](../rootfs/mesa/README.md)):
     libgallium 21 MB instead of 54 MB plus libLLVM's 171 MB. The two GLES contexts take
     0.64–0.89 s cold instead of 1.8–1.9 s; the shelf comes ~1 s sooner.
   - DSlot loads Mesa on a thread from its first instruction, and starts SDL video and GL while
     another thread lists the games, so the ROMS automount (~1 s, it waits for udev's
     coldplug) is off the critical path.
   - **Read-ahead** ([rootfs/readahead/](../rootfs/readahead/README.md)) from a systemd
     generator, which runs before any unit: the 41 MB DSlot and Mesa read before the first
     frame, at idle I/O priority, in 128 KB reads. ~0.3 s off the shelf. Reading more, or at
     normal priority, took the card from systemd and cost more than it saved.
   - SDL's controller scan and audio start after the first frame: each `close()` of an evdev
     node waits for an RCU grace period, and opening all 17 to see what they are cost ~0.5 s.

Without the splash, DSlot draws a **skeleton** of the shelf in software straight to both CRTCs
before anything else (3.3–3.6 s). A hybrid that showed the real shelf through KMS at 13.6 fps
and then switched to GL looked worse (a visible switch).

Tried and not kept:

- `rcupdate.rcu_expedited=1` on the command line: nothing measurable at boot. (It is set at
  runtime instead, for game starts: `rootfs/dslot/nibos-rcu.conf`.)
- Mounting ROMS early without udev ([rootfs/roms-early/](../rootfs/roms-early/nibos-roms-early.service)):
  the games were listed 0.5 s sooner, but the mount competed with GL starting and the shelf came
  later. It would pay only if the shelf had to wait for the game list.
- A launcher started straight from a unit on tty1 without login: 1 s sooner than the getty
  login, but with no logind session it has no seat. DSlot does exactly this, with the shortcut
  described in [architecture.md](architecture.md#known-shortcuts).

## What's left

- **Bootloader:** reading 17.5 MB at 69 MB/s (0.25 s); a smaller kernel is the lever.
- **Kernel:** the panels' own power-on sequence (~0.33 s from the modeset's start), which
  init waits for through the console.
- **Userspace:** ~0.9 s between DSlot's dependencies being done and its unit starting, while
  PID 1 works through the rest of sysinit (not yet looked into); the BPF probe.
- **Boot UI:** GL's cold start (~1.5 s from DSlot's skeleton to both windows and renderers
  ready, two GLES contexts through SDL). Driving EGL/GBM directly with one context for both
  panels is the next candidate; not tried yet.

## Shutdown

A restart's shutdown (systemd's first "Stopping" to "Journal stopped") takes 1.87 s median;
the goodbye picture doesn't change it, and nor does the whole restart measured from the
command to the device answering again (3.2-4.2 s without, 3.3-3.4 s with;
[rootfs/splash](../rootfs/splash/README.md)).

## Dead ends: don't repeat

Tried and measured, or found not to work; the source says more. Add to it when something doesn't pay.

### Bootloader

- Image.zst or Image.lz4 instead of gzip: both unpack slower in U-Boot proper (1.15 s, 0.35 s against gzip's 0.28 s) ([results](../bench/results/bootloader.md)).
- Rebuilding the decompressors at -O3 alone: barely changed anything; the cause was `-mstrict-align` and the CPU at 816 MHz.
- `clk dump` in the MEASURE build: changes SPL itself; read the PLL with `md`.
- U-Boot's "50 MHz" SD clock is 25 MHz (the controller halves it): patched ([patches.md](engineering/patches.md)).
- UEFI disk objects (0.22 s) and a full GPT scan (0.15 s): off.
- Falcon mode at SD High Speed: loses to U-Boot proper with UHS; Falcon only pays with UHS in SPL.
- A FIT config without `description`, or loadables without `SPL_OS_BOOT`: SPL never reaches TF-A.
- SDR50 in U-Boot: needs a 200 MHz CRU clock the SD mux can't make.
- SHA-256 of the FIT in SPL: 55 ms for little; off.
- The escape hatch on volume down: volume down + power doesn't start the device; it's A (EAST, GPIO3_B7, not SOUTH).

### Kernel

- journald's timestamps as the boot clock: invented for early lines; use the arch counter.
- `rootflags=noatime`: ext4 refuses it and the root mount fails.
- The panel driver probing synchronously: 0.81 s of a 2.03 s kernel start; it probes asynchronously now.
- SDR104 tuning over 360 phases (0.4 s) and the 112 ms timeout per failed phase: 90 phases, short timeout.
- An rk817 `vin-supply` for the SD 3.3 V switch: defers the SD controllers to late initcall.
- The IMU built in (0.1 s probing nothing), firmware-loading drivers built in (probe before the firmware is there): modules.
- Turning `CGROUP_BPF`/`BPF_SYSCALL` off (~0.18 s): systemd's device and IP access control needs them; not done.
- Mixed-case options in `scripts/config` (`I2C_MUX_PCA954x`): silently not set.

### Userspace

- getty `Type=idle`, PID 1 asking the console its size (333 ms timeout), the default mount rate limit (~0.7 s), `udevadm trigger --type=all` (1.35 s of udevd), Arch's nsswitch group merge, services ordered before sysinit (0.5-0.7 s each), zram-generator holding `/tmp`: all fixed, don't bring back.
- The launcher straight on tty1 without a login: no seat; ordered after `user-runtime-dir@`, no faster than the lean login.
- A chgrp script for DSlot's DRM and input nodes: 0.64 s and missed late nodes; `CAP_DAC_OVERRIDE` instead.
- `rcupdate.rcu_expedited=1` on the command line: nothing at boot; set at runtime.
- Read-ahead of whole files, or at normal I/O priority: took the card from systemd, shelf later.
- Mounting ROMS early without udev: games listed sooner, shelf later.
- The splash drawn before exec'ing systemd (~0.1 s late) or showing DSlot's last frame (looked broken; GL readback 0.12 s a screen).
- The "hybrid" boot UI (shelf via KMS, then GL): 13.6 fps and a visible switch.
- Arch's Mesa with LLVM: 1.8-1.9 s of cold reads; Panfrost-only Mesa.
- Network mode `boot` puts the radios on the boot path; loading rtw88 early changes nothing.
- The ROMS automount at boot costs ~1 s to whoever touches it first.
