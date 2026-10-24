#!/usr/bin/env python3
"""nibOS's own tuning file for the RG DS+'s two AW88166 speaker amplifiers.

The amps' mainline driver (sound/soc/codecs/aw88166.c) won't start them without a tuning file,
and Awinic's, from the stock firmware, isn't ours to redistribute. This writes one from scratch:
the container format is the one the driver's GPL parser reads (aw88395/aw88395_lib.c,
aw88395_data_type.h, aw88395_lib.h), and every register value below is chosen here from Awinic's
public datasheet (AW88166, Nov. 2023, V1.3). Nothing is taken from Awinic's file.

The DSP is bypassed (SYSCTRL.DSPBY), so the amps run without Awinic's DSP firmware: no speaker
excursion or voice-coil temperature protection, no EQ. The chip's own protections (over-
temperature, over-current, under- and over-voltage) stay. The values are conservative: the
lowest gain, the datasheet's default boost voltage and a lower boost current limit.

    aw88166-tuning.py OUT.bin          write the file
    aw88166-tuning.py --check FILE     check a file as the driver does (sizes, CRC8, sums)
"""
import struct
import sys

# --- What the amps are set to (the datasheet's register map; defaults in brackets) -----------

# SYSCTRL (0x04) [0xD307]: SET_GAIN 001 = 4.2 AV (the lowest) [101 = 6.75 AV]; DC cancelling on
# [on]; I2S interface on [off]; PLL from BCK [BCK]; DSP bypassed [bypassed]. HMUTE, AMPPD and
# PWDN are set here as at reset; the driver mutes, powers and unmutes the amp itself.
SYSCTRL = (1 << 15) | (0b001 << 12) | (1 << 9) | (1 << 8) | (1 << 6) | (1 << 2) | (1 << 1) | (1 << 0)
# SYSCTRL2 (0x05) [0x8000]: multi-stage power mode on [on]; VOL 0 dB [0 dB]: the gain and
# the boost limit the level, and with -6 dB here the loudest was too quiet (the listening tests
# in docs/engineering/audio.md). The driver takes VOL as its starting volume and adds ALSA's
# control to it.
SYSCTRL2 = (1 << 15) | 0
# I2SCTRL1 (0x06) [0x04E8]: Philips I2S [Philips], 32-bit data [32], BCK 64*fs [64], 48 kHz
# [48], as the RK3568's I2S3 sends it (rockchip-i2s, mclk 256*fs, PipeWire at 48 kHz); the
# channel per amp: left on 0x34, right on 0x35 (the device tree's awinic,audio-channel).
I2SCTRL1_BASE = (0b00 << 8) | (0b11 << 6) | (0b10 << 4) | 0b1000
CHSEL_LEFT, CHSEL_RIGHT = 0b01 << 10, 0b10 << 10
# BSTCTRL2 (0x61) [0x5B6C]: peak current limit 1.60 A [2.00 A]; small-signal deglitch 32 ms
# [32 ms]; Smart Boost 2 [Smart Boost 2]; boost voltage 4.5 V [the datasheet's stated
# default], never above.
BSTCTRL2 = (0b0011 << 12) | (0b1011 << 8) | (0b11 << 5) | 0b01011

AMPS = [  # (I2C bus, address, channel index, I2SCTRL1)
    (2, 0x34, 0, I2SCTRL1_BASE | CHSEL_LEFT),
    (2, 0x35, 1, I2SCTRL1_BASE | CHSEL_RIGHT),
]


def registers(i2sctrl1):
    # Written in this order by aw_dev_update_reg_container().
    return [(0x06, i2sctrl1), (0x61, BSTCTRL2), (0x05, SYSCTRL2), (0x04, SYSCTRL)]


# --- The container (aw88395_lib.c) ----------------------------------------------------------

ACF_FILE_ID = 0x0A15F908
CFG_HDR_VER = 0x00000001          # version 0: aw_cfg_hdr + aw_cfg_dde
HDR_FMT = "<I24s16s4sIIIII12s"    # struct aw_cfg_hdr: 80 bytes
DDE_FMT = "<I16sHHHHIIII20s"      # struct aw_cfg_dde: 64 bytes
DEV_TYPE_ID = 0
SEC_DSP_CFG, SEC_DSP_FW, SEC_HDR_REG = 2, 3, 4
PROFILE_MUSIC = 0
BIN_HEADER_V1 = 0x01000000
BIN_TYPE_REGISTER = 0
BIN_HEADER_LEN = 60
# Placeholders for the DSP's firmware and configuration: the driver insists on both, but with
# the DSP bypassed it only writes them to the DSP's RAM and never runs them. Even lengths
# (the driver swaps 16-bit words).
DSP_PLACEHOLDER = bytes(4)


def crc8(data):
    """The kernel's crc8() with crc8_populate_lsb(0x8C) and initial value 0."""
    crc = 0
    for b in data:
        crc ^= b
        for _ in range(8):
            crc = (crc >> 1) ^ 0x8C if crc & 1 else crc >> 1
    return crc


def register_bin(regs, addr):
    """A register list with its bin header: the ACF_SEC_TYPE_HDR_REG section."""
    body = struct.pack("<I", len(regs)) + b"".join(struct.pack("<HH", a, v) for a, v in regs)
    header = struct.pack("<IIIII", 0, BIN_HEADER_V1, BIN_TYPE_REGISTER, 1, len(body))
    header += struct.pack("<I8sIII", 0, b"AW88166", 2, 2, addr)  # ui_ver, chip, reg/data bytes
    header += bytes(BIN_HEADER_LEN - len(header))
    whole = header + body
    check_sum = sum(whole[4:]) & 0xFFFFFFFF
    return struct.pack("<I", check_sum) + whole[4:]


def build():
    sections = []  # (bus, addr, channel, data type, data)
    for bus, addr, ch, i2s in AMPS:
        sections.append((bus, addr, ch, SEC_DSP_CFG, DSP_PLACEHOLDER))
        sections.append((bus, addr, ch, SEC_DSP_FW, DSP_PLACEHOLDER))
        # Last for each amp: the driver takes the last section's type as the profile's.
        sections.append((bus, addr, ch, SEC_HDR_REG, register_bin(registers(i2s), addr)))
    hdr_len = struct.calcsize(HDR_FMT)
    ddt_size = len(sections) * struct.calcsize(DDE_FMT)
    offset = hdr_len + ddt_size
    ddes, data = b"", b""
    for bus, addr, ch, kind, payload in sections:
        ddes += struct.pack(DDE_FMT, DEV_TYPE_ID, b"aw88166", ch, bus, addr, PROFILE_MUSIC,
                            kind, len(payload), offset + len(data), crc8(payload), bytes(20))
        data += payload
    hdr = struct.pack(HDR_FMT, ACF_FILE_ID, b"nibOS RG DS+", b"nibOS", b"1.0", 0,
                      ddt_size, len(sections), hdr_len, CFG_HDR_VER, bytes(12))
    return hdr + ddes + data


def check(blob):
    """What aw88395_dev_load_acf_check() and the version 0 load path check; raises on a fault."""
    hdr = struct.unpack_from(HDR_FMT, blob)
    fid, _, _, _, _, ddt_size, ddt_num, hdr_offset, hdr_version, _ = hdr
    assert fid == ACF_FILE_ID, "not an acf file"
    assert hdr_version == CFG_HDR_VER, "not version 0"
    assert hdr_offset + ddt_size <= len(blob)
    ddes = [struct.unpack_from(DDE_FMT, blob, hdr_offset + i * struct.calcsize(DDE_FMT))
            for i in range(ddt_num)]
    assert hdr_offset + ddt_size + sum(d[7] for d in ddes) == len(blob), "sizes don't add up"
    for bus, addr, _, _ in AMPS:
        mine = [d for d in ddes if d[3] == bus and d[4] == addr and d[0] == DEV_TYPE_ID]
        assert mine, f"no sections for {bus}-{addr:04x}"
        assert mine[-1][6] == SEC_HDR_REG, "the last section must be the register list"
        kinds = {d[6] for d in mine}
        assert kinds == {SEC_DSP_CFG, SEC_DSP_FW, SEC_HDR_REG}, "reg, DSP config and firmware"
        for d in mine:
            _, _, _, _, _, prof, kind, size, off, crc, _ = d
            payload = blob[off:off + size]
            assert off + size <= len(blob) and crc8(payload) == crc, "CRC8"
            assert prof == PROFILE_MUSIC
            if kind in (SEC_DSP_CFG, SEC_DSP_FW):
                assert size and size % 2 == 0, "DSP data must be even"
            else:
                cs, ver, btype, dver, blen = struct.unpack_from("<IIIII", payload)
                assert ver == BIN_HEADER_V1 and btype == BIN_TYPE_REGISTER and dver == 1
                assert cs == sum(payload[4:BIN_HEADER_LEN + blen]) & 0xFFFFFFFF, "bin checksum"
                reg_b, dat_b = struct.unpack_from("<II", payload, 32)
                (count,) = struct.unpack_from("<I", payload, BIN_HEADER_LEN)
                assert count == (blen - 4) // (reg_b + dat_b), "register count"
                regs = (blen - 4) // 2
                assert regs % 2 == 0, "register data must be address/value pairs"
    return True


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--check":
        check(open(sys.argv[2], "rb").read())
        print("ok")
    elif len(sys.argv) == 2:
        blob = build()
        check(blob)
        open(sys.argv[1], "wb").write(blob)
        print(f"{sys.argv[1]}: {len(blob)} bytes")
    else:
        sys.exit(__doc__)
