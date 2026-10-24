#!/usr/bin/env python3
"""Make an RG DS+ boot partition image that boots our kernel through the stock U-Boot.

  mkboot.py STOCK_BOOT_IMG Image.gz board.dtb out.img [--bootargs ARGS] [--description TEXT]

The stock boot partition holds one U-Boot FIT with three images: the device tree, the kernel
and Rockchip's "resource" block (RSCE), which carries the boot pictures and a second copy of
the device tree (rk-kernel.dtb). The stock U-Boot does not enforce the FIT signature (see
dsboot). This keeps the stock layout: same image names and placeholder load addresses, our
device tree in both places, our kernel gzipped. --bootargs sets /chosen/bootargs, which the
stock U-Boot appends to its own arguments. Runs where mkimage and fdtput are installed (the
rgbuild machine).
"""

import hashlib
import os
import struct
import subprocess
import sys
import tempfile

SECTOR = 512


def fdt_props(blob):
    """{"/path:prop": bytes} for a flattened device tree (the FIT header is one)."""
    off_struct, off_strings = struct.unpack_from(">II", blob, 8)
    props, at, path = {}, off_struct, []
    while True:
        (token,) = struct.unpack_from(">I", blob, at)
        at += 4
        if token == 1:
            end = blob.index(b"\0", at)
            path.append(blob[at:end].decode())
            at = (end + 4) & ~3
        elif token == 2:
            path.pop()
        elif token == 3:
            size, name = struct.unpack_from(">II", blob, at)
            at += 8
            key = "/".join(path) + ":" + blob[off_strings + name:blob.index(b"\0", off_strings + name)].decode()
            props[key] = blob[at:at + size]
            at = (at + size + 3) & ~3
        elif token == 9:
            return props


def fit_image(fit, props, name):
    num = lambda k: int.from_bytes(props[f"/images/{name}:{k}"], "big")
    at, size = num("data-position"), num("data-size")
    return fit[at:at + size]


def replace_dtb(resource, dtb):
    """Put dtb in place of the resource block's rk-kernel.dtb, in the room the stock one had."""
    resource = bytearray(resource)
    if resource[:4] != b"RSCE":
        sys.exit("mkboot: resource block is not RSCE")
    entries, at = [], SECTOR
    while resource[at:at + 4] == b"ENTR":
        name = resource[at + 4:at + 224].split(b"\0")[0].decode()
        _, offset, size = struct.unpack_from("<III", resource, at + 256)
        entries.append((name, at, offset * SECTOR, size))
        at += SECTOR
    for name, at, offset, size in entries:
        if name != "rk-kernel.dtb":
            continue
        room = min([o for _, _, o, _ in entries if o > offset] + [len(resource)]) - offset
        if len(dtb) > room:
            sys.exit(f"mkboot: device tree is {len(dtb)} bytes, room for {room}")
        resource[offset:offset + room] = dtb + bytes(room - len(dtb))
        resource[at + 224:at + 244] = hashlib.sha1(dtb).digest()
        struct.pack_into("<I", resource, at + 264, len(dtb))
        return bytes(resource)
    sys.exit("mkboot: no rk-kernel.dtb in the resource block")


ITS = """/dts-v1/;
/ {
	description = "@DESCRIPTION@";
	#address-cells = <1>;
	images {
		fdt {
			data = /incbin/("fdt");
			type = "flat_dt";
			arch = "arm64";
			compression = "none";
			load = <0xffffff00>;
			hash { algo = "sha256"; };
		};
		kernel {
			data = /incbin/("kernel");
			type = "kernel";
			arch = "arm64";
			os = "linux";
			compression = "gzip";
			entry = <0xffffff01>;
			load = <0xffffff01>;
			hash { algo = "sha256"; };
		};
		resource {
			data = /incbin/("resource");
			type = "multi";
			arch = "arm64";
			compression = "none";
			hash { algo = "sha256"; };
		};
	};
	configurations {
		default = "conf";
		conf {
			rollback-index = <0x00>;
			fdt = "fdt";
			kernel = "kernel";
			multi = "resource";
		};
	};
};
"""


def main():
    args = sys.argv[1:]
    options = {"--bootargs": None, "--description": "nibos kernel"}
    for name in options:
        if name in args:
            at = args.index(name)
            options[name] = args[at + 1]
            del args[at:at + 2]
    bootargs = options["--bootargs"]
    if len(args) != 4:
        sys.exit(__doc__)
    stock, kernel, dtb, out = args
    with tempfile.TemporaryDirectory() as work:
        tree = os.path.join(work, "fdt")
        open(tree, "wb").write(open(dtb, "rb").read())
        if bootargs:
            subprocess.run(["fdtput", "-p", "-t", "s", tree, "/chosen", "bootargs", bootargs], check=True)
        tree = open(tree, "rb").read()
        fit = open(stock, "rb").read()
        props = fdt_props(fit)
        resource = replace_dtb(fit_image(fit, props, "resource"), tree)
        for name, data in (("fdt", tree), ("kernel", open(kernel, "rb").read()),
                           ("resource", resource)):
            open(os.path.join(work, name), "wb").write(data)
        open(os.path.join(work, "boot.its"), "w").write(ITS.replace("@DESCRIPTION@", options["--description"]))
        subprocess.run(["mkimage", "-E", "-p", "0x800", "-f", "boot.its", "boot.img"],
                       cwd=work, check=True, stdout=subprocess.DEVNULL)
        image = open(os.path.join(work, "boot.img"), "rb").read()
    # Whole sectors: raw disk writes (dd to /dev/rdiskN on macOS) need them.
    image += bytes(-len(image) % SECTOR)
    if len(image) > 64 << 20:
        sys.exit("mkboot: image does not fit the 64 MB boot partition")
    open(out, "wb").write(image)
    print(f"{out}: {len(image)} bytes, sha256 {hashlib.sha256(image).hexdigest()}")


if __name__ == "__main__":
    main()
