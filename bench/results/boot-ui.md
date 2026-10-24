# The boot UI

Seconds from reset with DSlot as the boot UI, measured with [`bench/boot-ui.sh`](../boot-ui.sh)
(3 boots per row unless marked, the range). Hardware: Anbernic RG DS+, the development card in
TF1, ROMs on the ROMS partition. Every build: kernel v7.3-rc7 and U-Boot v2026.10 in Falcon mode.

## 2026-10-18: splash and skeleton

Build: kernel v7.3-rc7 (`0fcb05b7`) and U-Boot v2026.10 with this repository's patches, `nibos-hw`
0.1.0-16, `mesa-nibos` 1:26.2.4-2 (Panfrost only, [rootfs/mesa/](../../rootfs/mesa/README.md)).
DSlot as the boot UI, splash off unless marked, no read-ahead, network mode `boot`.

| | Kernel starts | Kernel → init | Skeleton / splash | DSlot's unit | Shelf on GL, both screens |
|---|---|---|---|---|---|
| Skeleton | 0.65 | 0.87–0.91 | skeleton 3.41–3.62 | 2.78–2.82 | **5.11–5.49** |
| Splash on (2 boots) | 0.65 | 0.91–0.93 | splash **1.50–1.52** | 2.84–2.88 | 5.10–5.28 |

## 2026-10-16: splash and shelf

Build: network mode `deferred`, the Panfrost-only Mesa.

| | Picture on both screens | DSlot's unit starts | Shelf on GL, both screens |
|---|---|---|---|
| The splash (`init=`), drawn by a child while PID 1 execs systemd | **1.52–1.53** | 2.82–2.86 | **5.20–5.24** |
| The splash's logo instead of DSlot's last frame | 1.51–1.55 | | 5.2–5.4 (buttons answer at 6.3) |

## 2026-10-16: what each change to the shelf is worth

| How both panels are driven | Shelf on both screens |
|---|---|
| SDL KMSDRM + GLES (Panfrost), Arch's Mesa | 6.77 |
| Direct KMS, software rendering (13.6 fps) | 4.62–4.75 |
| Skeleton in software first, then GL | skeleton 3.35–3.44, GL 6.34–6.44 |
| Skeleton, Mesa without LLVM | skeleton 3.31–3.61, GL 5.39–5.56 |
| Mesa loaded on a thread from DSlot's first instruction, games listed on another | skeleton 3.31–3.42, GL 5.07–5.24 |
| Plus read-ahead of the first frame's 41 MB, idle I/O priority | skeleton 3.32–3.34, GL **4.74–4.85** |
| Splash instead of the skeleton | splash 1.52–1.53, GL 5.20–5.24 |

The last two are different builds of DSlot (the splash build had no read-ahead); with both,
the shelf is at about 5.2 s. What was tried and not kept, with numbers:
[docs/boot-time.md](../../docs/boot-time.md#the-boot-ui-a-picture-at-15-s-the-shelf-at-52-s).
