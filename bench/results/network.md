# Network modes

Boot time in each network mode, measured with [`bench/network.sh`](../network.sh), which arms a
recorder on the device so the figures are written before the network is up (3 boots per row,
seconds from reset). Hardware: Anbernic RG DS+, the development card in TF1, the same access
point. Build of 2026-10-16: kernel v7.3-rc7 with the 7.57 MB `Image.gz` and async panel probe
(before the panels-in-parallel and early-SD changes), mainline U-Boot proper (kernel at 1.04 s).

| Mode | For | Kernel → init | Console login | SSH reachable | Fully booted |
|---|---|---|---|---|---|
| `boot` | the dev system: drivers loaded early by the target | 1.13–1.15 | 4.22–4.28 | 5.78–5.99 | 6.11–6.28 |
| `deferred` | a productivity setup: Wi-Fi after start-up, at low priority | 1.12–1.14 | 4.14–4.22 | address at 10.14–10.25 (6.0 s after login) | **5.64–5.66** |
| `demand` | a game player: no network at boot | 1.12–1.13 | **4.18–4.19** | `nib net up`: address after 4.3–4.4 s | **5.58–5.64** |

With `deferred` the network costs the start-up nothing measurable and is there about six
seconds after the console. Later changes moved every column down (the reference in
[boot.md](boot.md) is mode `deferred` at 5.21–5.49 s fully booted).
