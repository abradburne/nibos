#!/usr/bin/env python3
"""Decode a U-Boot bootstage stash (BOOTSTAGE_STASH): bootstage.py stash.bin

Fetch it from the device with memdump (0x0f000000, 4096 bytes). SPL times count
from reset; U-Boot proper's restart from its own start.
"""
import struct, sys

b = open(sys.argv[1], "rb").read()
version, count, size, magic, next_id = struct.unpack_from("<5I", b, 0)
recs, off = [], 20
for _ in range(count):
    t, start, _name, flags, id_ = struct.unpack_from("<QI4xQiI", b, off)
    recs.append((t, start))
    off += 32
names = b[off:off + size].split(b"\0")[:count]
prev = None
for (t, start), name in zip(recs, names):
    if start:
        print(f"  {name.decode():18s} took {t / 1e3:8.1f} ms")
    else:
        step = f"(+{(t - prev) / 1e3:7.1f} ms)" if prev is not None and t >= prev else ""
        print(f"{t / 1e6:8.3f} s  {name.decode():18s} {step}")
        prev = t
