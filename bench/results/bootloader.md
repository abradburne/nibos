# The bootloader

From reset to the kernel's first instruction on the Anbernic RG DS+, measured with
[`bench/boot.sh`](../boot.sh) (3 boots) and U-Boot's own stage records
([`uboot/measure.sh`](../../uboot/measure.sh) with a `MEASURE=1` build). Build of
2026-10-16: U-Boot v2026.10, rkbin DDR init v1.26 and BL31 v1.46.

| Step | Kernel starts |
|---|---|
| Stock bootloader (Rockchip's TPL, SPL, TF-A, OP-TEE, U-Boot) | ~2.6 s |
| Mainline U-Boot v2026.10 as first installed | ~3.3 s |
| Data cache before relocation, a real 50 MHz SD clock, no UEFI, short partition scan, fast decompressors, CPU at 1608 MHz | 1.32 s |
| The same with a 7.57 MB kernel instead of 11.75 MB (each MB of `Image.gz` costs ~0.04 s) | 1.04 s |
| U-Boot reads the card at UHS SDR104 (67 MiB/s instead of 22.8) | 0.89 s |
| **Falcon mode, SPL at UHS SDR104** | **0.65 s** |

In Falcon mode: DDR init ~0.1 s; the card at SDR104 0.31 s after reset (the RK817 over I²C,
the 1.8 V switch, tuning; ~0.11 s more than High Speed); the 17.5 MB uncompressed kernel read in
0.25 s (69 MB/s); TF-A at ~0.60 s. At High Speed the same read took 0.73 s and Falcon lost to
U-Boot proper.

Single measurements behind the steps: U-Boot spent 0.92 s before relocation with the data
cache off (0.14 s to its boot command with it on); EFI disk objects for the card's partitions
cost 0.22 s; the partition scan's 116 empty GPT entries 0.15 s; gunzip of the 11.75 MB kernel
0.54 s at 816 MHz and 0.28 s at 1608 MHz; `Image.zst` and `Image.lz4` lost to gzip in U-Boot
proper (1.15 s and 0.35 s to unpack against 0.28 s).
