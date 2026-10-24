#!/usr/bin/env python3
"""Make the Falcon-mode FIT that SPL boots Linux from (see build.sh, FALCON=1).

  mkfalcon.py BL31.elf Image board.dtb "BOOTARGS" out.itb [--memory START SIZE]
              [--description TEXT]

Images: TF-A's loadable segments (atf-1 is the entry), the uncompressed kernel
and the device tree. With no U-Boot proper to fix the device tree up, it gets
its /memory node (--memory, the board's BOARD_MEMORY) and /chosen/bootargs
here. Runs where mkimage, fdtput and pyelftools are (the rgbuild machine).
"""
import argparse
import os
import subprocess
import sys
import tempfile

from elftools.elf.elffile import ELFFile

KERNEL_LOAD = 0x04000000
FDT_LOAD = 0x0A000000

parser = argparse.ArgumentParser(usage=__doc__.split("\n\n")[1])
parser.add_argument("bl31")
parser.add_argument("kernel")
parser.add_argument("dtb")
parser.add_argument("bootargs")
parser.add_argument("out")
# RAM as mainline U-Boot reports it on the RG DS+: 1 GB, the first 2 MB left to TF-A.
parser.add_argument("--memory", nargs=2, type=lambda v: int(v, 0), default=(0x00200000, 0x3FE00000))
parser.add_argument("--description", default="nibos Falcon boot")
args = parser.parse_args()
bl31, kernel, dtb, bootargs, out = args.bl31, args.kernel, args.dtb, args.bootargs, args.out
MEMORY = tuple(args.memory)
with tempfile.TemporaryDirectory() as work:
    atf = []
    with open(bl31, "rb") as f:
        elf = ELFFile(f)
        entry = elf.header.e_entry
        for seg in elf.iter_segments():
            if seg.header.p_type == "PT_LOAD" and seg.header.p_filesz:
                name = f"atf-{len(atf) + 1}"
                open(os.path.join(work, name), "wb").write(seg.data())
                atf.append((name, seg.header.p_paddr))
    assert atf[0][1] == entry, "TF-A's entry point is not its first segment"

    tree = os.path.join(work, "fdt")
    open(tree, "wb").write(open(dtb, "rb").read())
    fdtput = lambda *a: subprocess.run(["fdtput", "-p", *a], check=True)
    fdtput("-t", "s", tree, "/memory", "device_type", "memory")
    fdtput("-t", "x", tree, "/memory", "reg", "0", hex(MEMORY[0]), "0", hex(MEMORY[1]))
    fdtput("-t", "s", tree, "/chosen", "bootargs", bootargs)
    os.symlink(os.path.abspath(kernel), os.path.join(work, "Image"))

    images = [f"""
		kernel {{
			data = /incbin/("Image");
			type = "kernel";
			os = "linux";
			arch = "arm64";
			compression = "none";
			load = <{KERNEL_LOAD:#x}>;
			entry = <{KERNEL_LOAD:#x}>;
		}};"""]
    for name, addr in atf:
        images.append(f"""
		{name} {{
			data = /incbin/("{name}");
			type = "firmware";
			os = "arm-trusted-firmware";
			arch = "arm64";
			compression = "none";
			load = <{addr:#x}>;
			{"entry = <%#x>;" % entry if name == "atf-1" else ""}
		}};""")
    images.append(f"""
		fdt {{
			data = /incbin/("fdt");
			type = "flat_dt";
			arch = "arm64";
			compression = "none";
			load = <{FDT_LOAD:#x}>;
		}};""")
    loadables = ", ".join(['"kernel"'] + [f'"{n}"' for n, _ in atf[1:]])
    its = f"""/dts-v1/;
/ {{
	description = "{args.description}";
	#address-cells = <1>;
	images {{{''.join(images)}
	}};
	configurations {{
		default = "conf";
		conf {{
			/* SPL's fit_find_config_node() rejects a config without one */
			description = "{os.path.basename(dtb)}";
			firmware = "atf-1";
			loadables = {loadables};
			fdt = "fdt";
		}};
	}};
}};
"""
    open(os.path.join(work, "falcon.its"), "w").write(its)
    # External data, each image on a 512-byte boundary so SPL reads whole sectors.
    subprocess.run(["mkimage", "-E", "-B", "0x200", "-f", "falcon.its", "falcon.itb"],
                   cwd=work, check=True, stdout=subprocess.DEVNULL)
    data = open(os.path.join(work, "falcon.itb"), "rb").read()
data += bytes(-len(data) % 512)
open(out, "wb").write(data)
print(f"{out}: {len(data)} bytes, TF-A in {len(atf)} segments, kernel at {KERNEL_LOAD:#x}, fdt at {FDT_LOAD:#x}")
