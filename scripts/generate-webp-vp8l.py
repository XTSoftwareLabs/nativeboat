"""Writes a lossless WebP with what libwebp's encoder does not write for
small images: meta prefix codes with an unused group, the subtract-green
transform, every predictor mode (including the two unused ones), code
lengths with repeats and a symbol limit, and simple codes. The coded
pixels are random; Pillow (libwebp) decodes the file to give the reference.

usage: python generate-webp-vp8l.py OUT_WEBP
"""
import io
import random
import struct
import sys

from PIL import Image

out_path = sys.argv[1]
rng = random.Random(1030)


class Bits:
    def __init__(self):
        self.bits = []

    def put(self, value, count):
        for i in range(count):
            self.bits.append((value >> i) & 1)

    def code(self, code, length):
        # Canonical codes are stored first bit first.
        for i in reversed(range(length)):
            self.bits.append((code >> i) & 1)

    def bytes(self):
        out = bytearray((len(self.bits) + 7) // 8)
        for i, bit in enumerate(self.bits):
            out[i // 8] |= bit << (i % 8)
        return bytes(out)


def canonical(lengths):
    """Codes by symbol for lengths by symbol."""
    codes = {}
    code = 0
    for length in range(1, 16):
        for symbol, l in enumerate(lengths):
            if l == length:
                codes[symbol] = (code, length)
                code += 1
        code <<= 1
    return codes


CODE_LENGTH_ORDER = [17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
# The code length code: twelve symbols of three and four bits.
CL_SYMBOLS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 16, 17, 18]
CL_LENGTHS = [0] * 19
for s, l in zip(CL_SYMBOLS, [3, 4, 4, 3, 4, 4, 4, 4, 3, 4, 4, 3]):
    CL_LENGTHS[s] = l
CL_CODES = canonical(CL_LENGTHS)


class Code:
    """A prefix code over some symbols, and how to write it."""

    def __init__(self, alphabet, symbols, style="normal"):
        self.alphabet = alphabet
        self.symbols = list(symbols)
        self.style = style
        if style == "normal":
            # A complete code: lengths from a random full binary split.
            n = len(self.symbols)
            assert n >= 2
            lengths = {s: 0 for s in self.symbols}
            # Start with all at the same length when n is a power of two,
            # else split some leaves deeper.
            depth = 1
            while (1 << depth) < n:
                depth += 1
            leaves = [depth] * (1 << depth)
            # Merge pairs of leaves until there are n.
            while len(leaves) > n:
                deepest = max(leaves)
                leaves.remove(deepest)
                leaves.remove(deepest)
                leaves.append(deepest - 1)
            rng.shuffle(leaves)
            for s, l in zip(self.symbols, leaves):
                lengths[s] = l
            self.lengths = [lengths.get(s, 0) for s in range(alphabet)]
            self.codes = canonical(self.lengths)
        else:
            self.codes = None

    def write_header(self, w, limit=False):
        if self.style == "simple":
            w.put(1, 1)
            w.put(len(self.symbols) - 1, 1)
            first = self.symbols[0]
            if first < 2:
                w.put(0, 1)
                w.put(first, 1)
            else:
                w.put(1, 1)
                w.put(first, 8)
            if len(self.symbols) == 2:
                w.put(self.symbols[1], 8)
            return
        w.put(0, 1)
        count = max(i for i, s in enumerate(CODE_LENGTH_ORDER) if CL_LENGTHS[s]) + 1
        w.put(count - 4, 4)
        for s in CODE_LENGTH_ORDER[:count]:
            w.put(CL_LENGTHS[s], 3)
        tokens = self.tokens(limit)
        if limit:
            # Only as many tokens as needed; the rest of the lengths are 0.
            w.put(1, 1)
            n = len(tokens) - 2
            length_bits = 2
            while (1 << length_bits) <= n:
                length_bits += 2
            w.put((length_bits - 2) // 2, 3)
            w.put(n, length_bits)
        else:
            w.put(0, 1)
        for symbol, extra, extra_bits in tokens:
            w.code(*CL_CODES[symbol])
            if extra_bits:
                w.put(extra, extra_bits)

    def tokens(self, limit):
        lengths = list(self.lengths)
        if limit:
            while lengths and lengths[-1] == 0:
                lengths.pop()
        out = []
        i = 0
        previous = 8
        while i < len(lengths):
            l = lengths[i]
            run = 1
            while i + run < len(lengths) and lengths[i + run] == l:
                run += 1
            if l == 0 and run >= 11:
                n = min(run, 138)
                out.append((18, n - 11, 7))
                i += n
            elif l == 0 and run >= 3:
                n = min(run, 10)
                out.append((17, n - 3, 3))
                i += n
            elif l != 0 and l == previous and run >= 3:
                n = min(run, 6)
                out.append((16, n - 3, 2))
                i += n
            else:
                assert l in CL_SYMBOLS, l
                out.append((l, 0, 0))
                if l:
                    previous = l
                i += 1
        return out

    def write(self, w, symbol):
        if self.style == "simple":
            if len(self.symbols) == 2:
                w.put(sorted(self.symbols).index(symbol), 1)
            return
        w.code(*self.codes[symbol])


def literal_set(count, extra=()):
    return sorted(set(rng.sample(range(256), count)) | set(extra))


class Group:
    """The five codes of a group; `styles` picks how red and alpha are coded."""

    def __init__(self, alphabet_green, green_symbols, styles):
        green = Code(alphabet_green, green_symbols, styles[0])
        if styles[1] == "normal":
            red = Code(256, literal_set(8), "normal")
        elif styles[1] == "simple1":
            red = Code(256, [rng.randrange(256)], "simple")
        else:
            red = Code(256, literal_set(2), "simple")
        blue = Code(256, literal_set(16), "normal")
        if styles[3] == "simple1":
            alpha = Code(256, [255], "simple")
        else:
            alpha = Code(256, literal_set(4, [255]), "normal")
        self.codes = [green, red, blue, alpha, Code(40, [0, 1, 3, 5, 9, 14, 18, 25], "normal")]

    def write_header(self, w):
        for i, code in enumerate(self.codes):
            code.write_header(w, limit=(i == 4 or i == 0 and rng.random() < 0.5))


def sub_image(w, values):
    """A sub-image of literal pixels: no cache, single-symbol codes but for
    the channel `values` sets."""
    w.put(0, 1)  # no color cache
    present = sorted(set(values))
    channels = []
    for channel in range(4):  # green, red, blue, alpha
        if channel == 0:
            symbols = present
        else:
            symbols = [0] if channel != 3 else [255]
        if len(symbols) == 1:
            code = Code(256, symbols, "simple")
        elif len(symbols) == 2:
            code = Code(256, symbols, "simple")
        else:
            code = Code(280, symbols, "normal")
        channels.append(code)
    channels.append(Code(40, [0], "simple"))
    for code in channels:
        code.write_header(w)
    for v in values:
        channels[0].write(w, v)


def sub_image_rgb(w, values):
    """A sub-image of literal pixels with several values in red, green and
    blue, as cross-color multipliers."""
    w.put(0, 1)
    greens = sorted({v[1] for v in values})
    reds = sorted({v[0] for v in values})
    blues = sorted({v[2] for v in values})
    codes = [
        Code(280, greens, "normal"),
        Code(256, reds, "normal"),
        Code(256, blues, "normal"),
        Code(256, [255], "simple"),
        Code(40, [0], "simple"),
    ]
    for code in codes:
        code.write_header(w)
    for r, g, b in values:
        codes[0].write(w, g)
        codes[1].write(w, r)
        codes[2].write(w, b)


CODE_TO_PLANE = [
    0x18, 0x07, 0x17, 0x19, 0x28, 0x06, 0x27, 0x29, 0x16, 0x1a, 0x26, 0x2a, 0x38, 0x05, 0x37, 0x39,
    0x15, 0x1b, 0x36, 0x3a, 0x25, 0x2b, 0x48, 0x04, 0x47, 0x49, 0x14, 0x1c, 0x35, 0x3b, 0x46, 0x4a,
    0x24, 0x2c, 0x58, 0x45, 0x4b, 0x34, 0x3c, 0x03, 0x57, 0x59, 0x13, 0x1d, 0x56, 0x5a, 0x23, 0x2d,
    0x44, 0x4c, 0x55, 0x5b, 0x33, 0x3d, 0x68, 0x02, 0x67, 0x69, 0x12, 0x1e, 0x66, 0x6a, 0x22, 0x2e,
    0x54, 0x5c, 0x43, 0x4d, 0x65, 0x6b, 0x32, 0x3e, 0x78, 0x01, 0x77, 0x79, 0x53, 0x5d, 0x11, 0x1f,
    0x64, 0x6c, 0x42, 0x4e, 0x76, 0x7a, 0x21, 0x2f, 0x75, 0x7b, 0x31, 0x3f, 0x63, 0x6d, 0x52, 0x5e,
    0x00, 0x74, 0x7c, 0x41, 0x4f, 0x10, 0x20, 0x62, 0x6e, 0x30, 0x73, 0x7d, 0x51, 0x5f, 0x40, 0x72,
    0x7e, 0x61, 0x6f, 0x50, 0x71, 0x7f, 0x60, 0x70,
]


def prefix_range(symbol):
    if symbol < 4:
        return symbol + 1, 0
    extra = (symbol - 2) >> 1
    offset = (2 + (symbol & 1)) << extra
    return offset + 1, extra


def plane_distance(width, code):
    if code > 120:
        return code - 120
    plane = CODE_TO_PLANE[code - 1]
    return max(1, (plane >> 4) * width + 8 - (plane & 0xF))


width, height = 52, 37
w = Bits()
w.put(0x2F, 8)
w.put(width - 1, 14)
w.put(height - 1, 14)
w.put(1, 1)  # alpha
w.put(0, 3)
# Subtract green.
w.put(1, 1)
w.put(2, 2)
# The predictor, every mode in turn over 4x4 tiles.
w.put(1, 1)
w.put(0, 2)
w.put(0, 3)
tiles = ((width + 3) // 4) * ((height + 3) // 4)
sub_image(w, [i % 16 for i in range(tiles)])
# Cross color over 8x8 tiles.
w.put(1, 1)
w.put(1, 2)
w.put(1, 3)
tiles = ((width + 7) // 8) * ((height + 7) // 8)
sub_image_rgb(w, [(rng.randrange(256), rng.randrange(256), rng.randrange(256)) for _ in range(tiles)])
w.put(0, 1)  # no more transforms
cache_bits = 4
w.put(1, 1)
w.put(cache_bits, 4)
# Meta codes over 16x16 tiles: groups 0, 1 and 3; group 2 is read but unused.
across, down = (width + 15) // 16, (height + 15) // 16
w.put(1, 1)
w.put(2, 3)
groups_of_tiles = [[0, 1, 3][(i * 7) % 3] for i in range(across * down)]
sub_image(w, groups_of_tiles)
alphabet = 256 + 24 + (1 << cache_bits)
lengths_used = [256, 257, 259, 262, 265]
cache_used = [280, 283, 290, 295]
groups = []
for g in range(4):
    styles = [
        ("normal", "normal", "normal", "normal"),
        ("normal", "simple1", "normal", "simple1"),
        ("normal", "simple2", "normal", "normal"),
        ("normal", "normal", "normal", "simple1"),
    ][g]
    green = literal_set(21) + lengths_used + cache_used
    groups.append(Group(alphabet, green, styles))
for group in groups:
    group.write_header(w)

position = 0
total = width * height
while position < total:
    x, y = position % width, position // width
    group = groups[groups_of_tiles[(y >> 4) * across + (x >> 4)]]
    green, red, blue, alpha, dist = group.codes
    choice = rng.random()
    if choice < 0.2 and position > 0:
        # A backward reference.
        length_symbol = rng.choice(lengths_used) - 256
        base, extra = prefix_range(length_symbol)
        length = base + (rng.randrange(1 << extra) if extra else 0)
        distance_symbol = rng.choice(dist.symbols)
        dbase, dextra = prefix_range(distance_symbol)
        dvalue = dbase + (rng.randrange(1 << dextra) if dextra else 0)
        distance = plane_distance(width, dvalue)
        if distance <= position and length <= total - position:
            green.write(w, 256 + length_symbol)
            if extra:
                w.put(length - base, extra)
            dist.write(w, distance_symbol)
            if dextra:
                w.put(dvalue - dbase, dextra)
            position += length
            continue
    if choice < 0.35:
        green.write(w, rng.choice(cache_used))
        position += 1
        continue
    green.write(w, rng.choice([s for s in green.symbols if s < 256]))
    red.write(w, rng.choice(red.symbols))
    blue.write(w, rng.choice(blue.symbols))
    alpha.write(w, rng.choice(alpha.symbols))
    position += 1

data = w.bytes()
chunk = b"VP8L" + struct.pack("<I", len(data)) + data + (b"\0" if len(data) & 1 else b"")
riff = b"RIFF" + struct.pack("<I", 4 + len(chunk)) + b"WEBP" + chunk
open(out_path, "wb").write(riff)


def fnv(data):
    h = 0xCBF29CE484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


rgba = Image.open(io.BytesIO(riff)).convert("RGBA")
print(f"{rgba.size[0]}x{rgba.size[1]} 0x{fnv(rgba.tobytes()):016x} bytes={len(riff)}")
