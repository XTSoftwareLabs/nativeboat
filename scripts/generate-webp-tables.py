"""Generates crates/nativeboat-win32/src/webp/tables.rs from libwebp's sources.

usage: python generate-webp-tables.py LIBWEBP_SRC_DEC_DIR OUT_RS
"""
import re
import sys

src_dir, out_path = sys.argv[1], sys.argv[2]
tree = open(f"{src_dir}/tree_dec.c", encoding="utf-8").read()
quant = open(f"{src_dir}/quant_dec.c", encoding="utf-8").read()


def numbers(text, name):
    """The numbers of the C array initializer that follows `name`."""
    start = text.index(name)
    start = text.index("=", start)
    depth = 0
    body = []
    for i in range(start, len(text)):
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                body = text[start:i]
                break
    body = re.sub(r"//[^\n]*", "", body)
    return [int(n) for n in re.findall(r"-?\d+", body)]


def nested(values, dims):
    if len(dims) == 1:
        assert len(values) == dims[0], (len(values), dims)
        return "[" + ", ".join(str(v) for v in values) + "]"
    size = len(values) // dims[0]
    assert size * dims[0] == len(values)
    return "[" + ", ".join(nested(values[i * size:(i + 1) * size], dims[1:]) for i in range(dims[0])) + "]"


proba0 = numbers(tree, "CoeffsProba0[NUM_TYPES]")
update = numbers(tree, "CoeffsUpdateProba[NUM_TYPES]")
bmodes = numbers(tree, "kBModesProba[NUM_BMODES]")
dc = numbers(quant, "kDcTable[128]")
ac = numbers(quant, "kAcTable[128]")
assert len(proba0) == len(update) == 4 * 8 * 3 * 11
assert len(bmodes) == 10 * 10 * 9
assert len(dc) == len(ac) == 128

out = []
out.append("//! The constant tables of the VP8 format (RFC 6386), as libwebp has them.")
out.append("")
out.append("/// The default probabilities of coefficient tokens, by plane type, band,")
out.append("/// context and branch (RFC 6386, 13.5).")
out.append(f"pub(super) const COEFFS_PROBA: [[[[u8; 11]; 3]; 8]; 4] = {nested(proba0, [4, 8, 3, 11])};")
out.append("")
out.append("/// The probabilities that a frame updates each coefficient probability")
out.append("/// (RFC 6386, 13.4).")
out.append(f"pub(super) const COEFFS_UPDATE_PROBA: [[[[u8; 11]; 3]; 8]; 4] = {nested(update, [4, 8, 3, 11])};")
out.append("")
out.append("/// The probabilities of 4x4 luma prediction modes, by the modes above and")
out.append("/// to the left (RFC 6386, 11.5).")
out.append(f"pub(super) const BMODES_PROBA: [[[u8; 9]; 10]; 10] = {nested(bmodes, [10, 10, 9])};")
out.append("")
out.append("/// The DC dequantization factors by quantizer index (RFC 6386, 14.1).")
out.append(f"pub(super) const DC_TABLE: [u8; 128] = {nested(dc, [128])};")
out.append("")
out.append("/// The AC dequantization factors by quantizer index (RFC 6386, 14.1).")
out.append(f"pub(super) const AC_TABLE: [u16; 128] = {nested(ac, [128])};")
out.append("")
open(out_path, "w", encoding="utf-8", newline="\n").write("\n".join(out))
print("ok", len(proba0), len(update), len(bmodes), dc[-1], ac[-1])
