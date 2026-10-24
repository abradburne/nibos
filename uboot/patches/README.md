# U-Boot patches: generic

The patches every nibOS U-Boot gets, whatever the board, applied by
[`../build.sh`](../build.sh) on top of mainline U-Boot `UBOOT_TAG` (`config.sh`; v2026.10),
before the SoC's (`socs/<soc>/uboot/patches/`) and the board's
(`boards/<board>/uboot/patches/`). GPL-2.0+, like U-Boot.

| # | Patch | Upstream status |
|---|---|---|
| 0001 | `bootstage mark NAME` command | Candidate |
| 0002 | SPL boots Linux through TF-A (Falcon mode) | Candidate |
| 0003 | The partition scan stops at the first unused GPT entry | Board-specific hack |
| 0004 | Kernel decompressors built with `-O3 -mno-strict-align` | Board-specific hack |

`FALLBACK=1` builds apply only 0001 and 0002 and nothing else: the fallback U-Boot stays the
first one that booted. Two sets are for investigation builds only, never installed for normal
use: `measure/` (`MEASURE=1`) and `falcon-debug/` (`FALCON=1 FALCON_DEBUG=1`).

Why each one exists, as measured, next to the SoC's and the board's, and how to change one:
[docs/engineering/patches.md](../../docs/engineering/patches.md).
