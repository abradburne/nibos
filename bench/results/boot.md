# Boot time

Seconds from reset, measured with [`bench/boot.sh`](../boot.sh): the device restarts over SSH,
3 boots per row unless it says otherwise, and the range is given. "Kernel starts" is the arch
counter minus `CLOCK_BOOTTIME` plus DDR init and SPL from U-Boot's bootstage records; systemd's
stamps count from the kernel's start. "Console" is when the shell on tty1 starts (the prompt
you can type at); "SSH" is when a client can connect (sshd listening and Wi-Fi associated);
"fully booted" is systemd's.

Hardware for every row: Anbernic RG DS+ (RK3568, 1 GB), the development card in TF1 (a copy
of the stock image plus nibOS's root partition), Wi-Fi to the same access point.

## Reference: 2026-10-16, kernel v7.3-rc7, U-Boot v2026.10 in Falcon mode

The baseline for later changes. Build of 2026-10-16: kernel v7.3-rc7 with the current generic,
RK3568 and RG DS+ kernel patches except the rtw88 and 32 kHz clock fixes (kernel
`generic/0002`, `generic/0003`, `rk3568/0004`); U-Boot v2026.10 in Falcon mode; network mode
`deferred`; boot UI off.

| | Stock firmware | nibOS (tty1 shell) |
|---|---|---|
| Kernel starts | ~2.6 | **0.65** |
| Kernel → init | 1.92–1.94 (from the kernel's start) | 0.89–0.90 |
| getty | | 3.43–3.62 |
| Console | 7.29–7.40 (login prompt) | **4.05–4.58** |
| SSH | 9.60–9.73 | 10.00–10.51 (network after start-up, by design of `deferred`) |
| Fully booted | 9.90–9.99 (+ ~0.3 s DDR init and SPL) | 5.21–5.49 |

The stock firmware's column is counted from TF-A (its DDR init and SPL, ~0.3 s, are not in
its records).

## How it got there

From the first boot of mainline Linux (2026-10-15) to the reference, step by step; seconds
from reset unless marked. What each step changed is in [docs/boot-time.md](../../docs/boot-time.md).

| Step | Kernel starts | Console | Fully booted |
|---|---|---|---|
| Start: arm64 defconfig, stock Arch setup, stock bootloader (from TF-A) | 4.6 | 11.9 | 12.45 |
| Userspace trimming (quiet, no root fstab entry, Rockchip-only kernel, video drivers and zram after boot, generators masked) | 3.45 | 6.6–6.8 | 9.2–9.4 |
| Mainline U-Boot with its boot-time patches | 1.32 | 6.20–6.44 | 8.73–9.00 |
| More userspace trimming ([`rootfs/tune-boot.sh`](../../rootfs/tune-boot.sh)) | 1.32 | 4.99–5.07 | 7.32–7.42 |
| Kernel: `Image.gz` 11.75 → 7.57 MB, panel async probe | 1.04 | 4.12–4.18 | 6.43–6.58 |
| Kernel: panels in parallel, SD controllers early | 1.05 | 3.69–4.34 | 5.94–6.54 |
| U-Boot at UHS SDR104 | 0.89 | | 5.62–5.87 |
| Userspace: lingering user, `user-light` tty sessions | 1.04 | 4.44–4.78 | 5.91–6.18 |
| Falcon mode, SPL at UHS SDR104 (the reference) | **0.65** | **4.05–4.58** | **5.21–5.49** |
