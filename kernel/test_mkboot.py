"""Tests for mkboot.py's device tree and resource block handling.

  python3 -m unittest discover -s kernel        (or: make python)
"""
import hashlib
import struct
import unittest

import mkboot

SECTOR = mkboot.SECTOR


def fdt(nodes):
    """A flattened device tree from {"/path": {"prop": bytes}}, enough for fdt_props()."""
    strings, structure = b"", b""

    def name_off(name):
        nonlocal strings
        at = strings.find(name.encode() + b"\0")
        if at < 0:
            at, strings = len(strings), strings + name.encode() + b"\0"
        return at

    def pad(b):
        return b + bytes(-len(b) % 4)

    def node(path, children):
        nonlocal structure
        structure += struct.pack(">I", 1) + pad(path.rsplit("/", 1)[-1].encode() + b"\0")
        for prop, value in nodes.get(path or "/", {}).items():
            structure += struct.pack(">III", 3, len(value), name_off(prop)) + pad(value)
        for child in children:
            node(child, [p for p in nodes if p.startswith(child + "/") and p.count("/") == child.count("/") + 1])
        structure += struct.pack(">I", 2)

    node("", [p for p in nodes if p.count("/") == 1 and p != "/"])
    structure += struct.pack(">I", 9)
    off_struct = 40
    off_strings = off_struct + len(structure)
    header = struct.pack(">10I", 0xD00DFEED, off_strings + len(strings), off_struct, off_strings,
                         40, 17, 16, 0, len(strings), len(structure))
    return header + structure + strings


def resource(entries, total_sectors):
    """A Rockchip RSCE block: a header sector, an ENTR sector per (name, sector, data)."""
    block = bytearray(total_sectors * SECTOR)
    block[:4] = b"RSCE"
    for i, (name, sector, data) in enumerate(entries):
        at = SECTOR * (1 + i)
        block[at:at + 4] = b"ENTR"
        block[at + 4:at + 4 + len(name)] = name.encode()
        struct.pack_into("<III", block, at + 256, 0, sector, len(data))
        block[sector * SECTOR:sector * SECTOR + len(data)] = data
    return bytes(block)


class FdtProps(unittest.TestCase):
    def test_reads_nested_properties(self):
        blob = fdt({"/images": {}, "/images/kernel": {"data-size": b"\0\0\0\x10", "type": b"kernel\0"}})
        props = mkboot.fdt_props(blob)
        self.assertEqual(props["/images/kernel:data-size"], b"\0\0\0\x10")
        self.assertEqual(props["/images/kernel:type"], b"kernel\0")

    def test_fit_image_slices_external_data(self):
        blob = fdt({"/images": {}, "/images/resource": {
            "data-position": struct.pack(">I", 3), "data-size": struct.pack(">I", 4)}})
        fit = b"abcdefghij"
        self.assertEqual(mkboot.fit_image(fit, mkboot.fdt_props(blob), "resource"), b"defg")


class ReplaceDtb(unittest.TestCase):
    OLD = b"old-device-tree" * 10

    def block(self):
        return resource([("logo.bmp", 4, b"BM..."), ("rk-kernel.dtb", 5, self.OLD), ("tail", 7, b"x")], 9)

    def test_puts_the_tree_in_the_old_ones_room(self):
        new = b"new tree"
        out = mkboot.replace_dtb(self.block(), new)
        entry = 2 * SECTOR
        self.assertEqual(out[5 * SECTOR:5 * SECTOR + len(new)], new)
        # The rest of its room up to the next entry's data is zeroed.
        self.assertEqual(out[5 * SECTOR + len(new):7 * SECTOR], bytes(2 * SECTOR - len(new)))
        self.assertEqual(out[entry + 224:entry + 244], hashlib.sha1(new).digest())
        self.assertEqual(struct.unpack_from("<I", out, entry + 264)[0], len(new))
        # Other entries are untouched.
        self.assertEqual(out[7 * SECTOR:7 * SECTOR + 1], b"x")
        self.assertEqual(out[4 * SECTOR:4 * SECTOR + 5], b"BM...")

    def test_refuses_a_tree_larger_than_the_room(self):
        with self.assertRaises(SystemExit):
            mkboot.replace_dtb(self.block(), bytes(2 * SECTOR + 1))

    def test_refuses_what_is_not_a_resource_block(self):
        with self.assertRaises(SystemExit):
            mkboot.replace_dtb(b"NOPE" + bytes(SECTOR), b"tree")


if __name__ == "__main__":
    unittest.main()
