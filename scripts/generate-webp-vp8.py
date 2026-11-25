"""Writes a lossy WebP whose VP8 frame uses features libwebp's encoder never
writes: the simple loop filter, filter-level deltas, relative segment
values, several token partitions and quantizer deltas. Pillow (libwebp)
decodes it to give the reference digest.

usage: python generate-webp-vp8.py TABLES_RS OUT_WEBP simple|normal|plain
"""
import io
import random
import re
import struct
import sys

from PIL import Image

tables_rs, out_path = sys.argv[1], sys.argv[2]
variant = sys.argv[3] if len(sys.argv) > 3 else "simple"
text = open(tables_rs, encoding="utf-8").read()


def table(name, count):
    start = text.index(name)
    start = text.index("=", start)
    end = text.index(";", start)
    values = [int(n) for n in re.findall(r"\d+", text[start:end])]
    assert len(values) == count, (name, len(values))
    return values


proba = table("COEFFS_PROBA:", 4 * 8 * 3 * 11)
update = table("COEFFS_UPDATE_PROBA:", 4 * 8 * 3 * 11)
bmodes = table("BMODES_PROBA:", 10 * 10 * 9)


def coeff_p(values, t, b, c):
    at = ((t * 8 + b) * 3 + c) * 11
    return values[at:at + 11]


BANDS = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 0]
CATEGORIES = [
    [173, 148, 140],
    [176, 155, 140, 135],
    [180, 157, 141, 134, 130],
    [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129],
]


class Writer:
    """RFC 6386's boolean encoder."""

    def __init__(self):
        self.out = bytearray()
        self.range = 255
        self.bottom = 0
        self.count = 24

    def carry(self):
        i = len(self.out) - 1
        while i >= 0 and self.out[i] == 255:
            self.out[i] = 0
            i -= 1
        self.out[i] += 1

    def bit(self, value, probability):
        split = 1 + (((self.range - 1) * probability) >> 8)
        if value:
            self.bottom += split
            self.range -= split
        else:
            self.range = split
        while self.range < 128:
            self.range <<= 1
            if self.bottom & (1 << 31):
                self.carry()
            self.bottom = (self.bottom << 1) & 0xFFFFFFFF
            self.count -= 1
            if self.count == 0:
                self.out.append((self.bottom >> 24) & 0xFF)
                self.bottom &= (1 << 24) - 1
                self.count = 8

    def flag(self, value):
        self.bit(1 if value else 0, 128)

    def value(self, value, count):
        for bit in reversed(range(count)):
            self.flag((value >> bit) & 1)

    def signed(self, value, count):
        self.value(abs(value), count)
        self.flag(value < 0)

    def finish(self):
        for _ in range(32):
            self.bit(0, 128)
        return bytes(self.out)


rng = random.Random(1029)
simple = variant == "simple"
plain = variant == "plain"
width, height = {"simple": (70, 66), "normal": (40, 72), "plain": (35, 20)}[variant]
mb_w, mb_h = (width + 15) // 16, (height + 15) // 16
partition_bits = {"simple": 2, "normal": 3, "plain": 1}[variant]
partitions = [Writer() for _ in range(1 << partition_bits)]
header = Writer()
header.flag(0)  # color space
header.flag(0)  # clamping
# "simple" has segments with a map and values relative to the frame's;
# "plain" has segments without either, so every macroblock is in the first;
# "normal" has none.
segmented = variant != "normal"
update_map = simple
header.flag(segmented)
if segmented:
    header.flag(update_map)
    header.flag(simple)  # update the segments' values
if simple:
    header.flag(0)  # relative to the frame's
    for q in [0, -6, 9, 0]:
        header.flag(q != 0)
        if q:
            header.signed(q, 7)
    for f in [0, 5, -4, 20]:
        header.flag(f != 0)
        if f:
            header.signed(f, 6)
segment_probabilities = [120, 90, 200]
if update_map:
    for p in segment_probabilities:
        header.flag(1)
        header.value(p, 8)
header.flag(simple)
header.value(0 if plain else 18 if simple else 30, 6)  # level
header.value(3 if simple else 6, 3)  # sharpness
header.flag(1)  # filter deltas
header.flag(1)  # update them
for delta in [2, 0, 0, 0, -5 if simple else 7, 0, 0, 3]:
    header.flag(delta != 0)
    if delta:
        header.signed(delta, 6)
header.value(partition_bits, 2)
header.value(36 if simple else 20, 7)  # base quantizer
for delta in [-3, 2, 0, 4, -2]:
    header.flag(delta != 0)
    if delta:
        header.signed(delta, 4)
header.flag(0)  # refresh entropy probabilities
bands = list(proba)
for i, p in enumerate(update):
    change = rng.random() < 0.02
    header.bit(change, p)
    if change:
        bands[i] = rng.randrange(1, 256)
        header.value(bands[i], 8)
skip_probability = 60
header.flag(1)
header.value(skip_probability, 8)


def write_large(w, p, v):
    if v <= 4:
        w.bit(0, p[3])
        if v == 2:
            w.bit(0, p[4])
        else:
            w.bit(1, p[4])
            w.bit(v - 3, p[5])
    elif v <= 10:
        w.bit(1, p[3])
        w.bit(0, p[6])
        if v <= 6:
            w.bit(0, p[7])
            w.bit(v - 5, 159)
        else:
            w.bit(1, p[7])
            x = v - 7
            w.bit(x >> 1, 165)
            w.bit(x & 1, 145)
    else:
        w.bit(1, p[3])
        w.bit(1, p[6])
        category = 0
        while category < 3 and v >= 3 + (16 << category):
            category += 1
        high = category >> 1
        w.bit(high, p[8])
        w.bit(category & 1, p[9 + high])
        extra = v - (3 + (8 << category))
        bits = CATEGORIES[category]
        for i, probability in enumerate(bits):
            w.bit((extra >> (len(bits) - 1 - i)) & 1, probability)


def write_block(w, t, context, first, values):
    """Writes coefficients by scan position; returns whether any past
    `first` is not zero."""
    last = max((n for n in range(16) if values[n]), default=-1)
    n = first
    p = coeff_p(bands, t, BANDS[n], context)
    while n < 16:
        if n > last:
            w.bit(0, p[0])
            return last >= first
        w.bit(1, p[0])
        while values[n] == 0:
            w.bit(0, p[1])
            n += 1
            p = coeff_p(bands, t, BANDS[n], 0)
        w.bit(1, p[1])
        v = abs(values[n])
        if v == 1:
            w.bit(0, p[2])
            p = coeff_p(bands, t, BANDS[n + 1], 1)
        else:
            w.bit(1, p[2])
            write_large(w, p, v)
            p = coeff_p(bands, t, BANDS[n + 1], 2)
        w.flag(values[n] < 0)
        n += 1
    return last >= first


def random_values(first, density, large):
    values = [0] * 16
    for n in range(first, 16):
        if rng.random() < density / (1 + n):
            v = rng.choice([1, 1, 1, 2, 3, 4, 5, 6, 7, 9, 10])
            if rng.random() < large:
                v = rng.choice([11, 18, 19, 34, 35, 66, 67, 150, 400])
            values[n] = v if rng.random() < 0.5 else -v
    return values


top_modes = [0] * (4 * mb_w)
top_nz = [[0] * 9 for _ in range(mb_w)]  # 4 luma, 2 u, 2 v, y2
for mb_y in range(mb_h):
    left_modes = [0] * 4
    left_nz = [0] * 9
    tokens = partitions[mb_y & ((1 << partition_bits) - 1)]
    for mb_x in range(mb_w):
        if update_map:
            segment = rng.randrange(4)
            header.bit(segment >= 2, segment_probabilities[0])
            header.bit(segment & 1, segment_probabilities[2 if segment >= 2 else 1])
        skip = rng.random() < 0.2
        header.bit(skip, skip_probability)
        # Skipped whole-block macroblocks have no inner edges to filter.
        i4x4 = rng.random() < 0.4 and not (skip and simple)
        header.bit(not i4x4, 145)
        if not i4x4:
            mode = (mb_y * mb_w + mb_x) % 4  # DC, TM, V, H
            header.bit(mode in (1, 3), 156)
            if mode in (1, 3):
                header.bit(mode == 1, 128)
            else:
                header.bit(mode == 2, 163)
            top_modes[4 * mb_x:4 * mb_x + 4] = [mode] * 4
            left_modes = [mode] * 4
        else:
            for y in range(4):
                for x in range(4):
                    top = top_modes[4 * mb_x + x]
                    left = left_modes[y]
                    p = bmodes[(top * 10 + left) * 9:(top * 10 + left) * 9 + 9]
                    mode = rng.randrange(10)
                    # The tree, as the decoder walks it.
                    path = {
                        0: [(0, 0)],
                        1: [(1, 0), (0, 1)],
                        2: [(1, 0), (1, 1), (0, 2)],
                        3: [(1, 0), (1, 1), (1, 2), (0, 3), (0, 4)],
                        4: [(1, 0), (1, 1), (1, 2), (0, 3), (1, 4), (0, 5)],
                        5: [(1, 0), (1, 1), (1, 2), (0, 3), (1, 4), (1, 5)],
                        6: [(1, 0), (1, 1), (1, 2), (1, 3), (0, 6)],
                        7: [(1, 0), (1, 1), (1, 2), (1, 3), (1, 6), (0, 7)],
                        8: [(1, 0), (1, 1), (1, 2), (1, 3), (1, 6), (1, 7), (0, 8)],
                        9: [(1, 0), (1, 1), (1, 2), (1, 3), (1, 6), (1, 7), (1, 8)],
                    }[mode]
                    for bit, branch in path:
                        header.bit(bit, p[branch])
                    top_modes[4 * mb_x + x] = mode
                    left_modes[y] = mode
        uv = rng.randrange(4)
        header.bit(uv != 0, 142)
        if uv:
            header.bit(uv != 2, 114)
            if uv != 2:
                header.bit(uv == 1, 183)
        tnz, lnz = top_nz[mb_x], left_nz
        if skip:
            for i in range(8):
                tnz[i] = lnz[i] = 0
            if not i4x4:
                tnz[8] = lnz[8] = 0
            continue
        first = 0
        if not i4x4:
            dc = random_values(0, 3.0 if rng.random() < 0.6 else 0.6, 0.2)
            nz = write_block(tokens, 1, tnz[8] + lnz[8], 0, dc)
            tnz[8] = lnz[8] = int(nz)
            first = 1
        t = 3 if i4x4 else 0
        for y in range(4):
            for x in range(4):
                values = random_values(first, 1.2, 0.05) if rng.random() < 0.7 else [0] * 16
                nz = write_block(tokens, t, tnz[x] + lnz[y], first, values)
                tnz[x] = lnz[y] = int(nz)
        for channel in (4, 6):
            for y in range(2):
                for x in range(2):
                    values = random_values(0, 1.0, 0.05) if rng.random() < 0.6 else [0] * 16
                    nz = write_block(tokens, 2, tnz[channel + x] + lnz[channel + y], 0, values)
                    tnz[channel + x] = lnz[channel + y] = int(nz)

first_partition = header.finish()
token_data = [w.finish() for w in partitions]
frame = bytearray()
tag = (len(first_partition) << 5) | (1 << 4)
frame += struct.pack("<I", tag)[:3]
frame += b"\x9d\x01\x2a"
frame += struct.pack("<HH", width, height)
frame += first_partition
for data in token_data[:-1]:
    frame += struct.pack("<I", len(data))[:3]
for data in token_data:
    frame += data
chunk = b"VP8 " + struct.pack("<I", len(frame)) + frame + (b"\0" if len(frame) & 1 else b"")
riff = b"RIFF" + struct.pack("<I", 4 + len(chunk)) + b"WEBP" + chunk
open(out_path, "wb").write(riff)


def fnv(data):
    h = 0xCBF29CE484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


decoded = Image.open(io.BytesIO(riff))
rgba = decoded.convert("RGBA")
print(f"{variant}: {rgba.size[0]}x{rgba.size[1]} 0x{fnv(rgba.tobytes()):016x} bytes={len(riff)}")
