# Kernel patches: generic

The patches every nibOS kernel gets, whatever the board, applied by
[`../build.sh`](../build.sh) on top of mainline Linux at `KERNEL_COMMIT` (`config.sh`;
v7.3-rc7), before the SoC's (`socs/<soc>/kernel/patches/`) and the board's
(`boards/<board>/kernel/patches/`). GPL-2.0, like the kernel.

| # | Patch | Upstream status |
|---|---|---|
| 0001 | `init/main.c`: open the console after mounting root | Board-specific hack |
| 0002 | rtw88 SDIO: resume the host interface before the first MAC access | Candidate |
| 0003 | rtw88 SDIO: power the card off in suspend without WoWLAN | Candidate |
| 0004 | rk817 charger: a writable charge voltage (battery protect) | Candidate |
| 0005 | aw88166: start the amp once the stream runs (speakers after resume) | Candidate |

Why each one exists, as measured, next to the SoC's and the board's, and how to change one:
[docs/engineering/patches.md](../../docs/engineering/patches.md).
