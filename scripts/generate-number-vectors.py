"""Generates crates/nativeboat-native/src/number_builtin/vectors.txt: reference
results of Number.prototype.toFixed, toExponential and toPrecision for random
doubles, computed from their exact decimal values with Python's `decimal`
module under the specification's rule (ties choose the larger magnitude).

    python scripts/generate-number-vectors.py
"""

import decimal
import os
import random
import struct

decimal.getcontext().prec = 2000
OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "nativeboat-native", "src", "number_builtin", "vectors.txt")


def exact(x):
    return decimal.Decimal(x)


def fixed(x, digits):
    if abs(x) >= 1e21:
        return None
    value = exact(abs(x)).quantize(decimal.Decimal(1).scaleb(-digits), rounding=decimal.ROUND_HALF_UP)
    text = format(value, "f")
    sign = "-" if x < 0 and text.strip("0.") != "" else ("-" if x < 0 else "")
    return sign + text


def significant(x, count):
    """Digits and exponent of |x| rounded to `count` significant digits."""
    value = exact(abs(x))
    exponent = value.adjusted()
    rounded = value.scaleb(-exponent).quantize(decimal.Decimal(1).scaleb(-(count - 1)), rounding=decimal.ROUND_HALF_UP)
    if rounded >= 10:
        exponent += 1
        rounded = value.scaleb(-exponent).quantize(decimal.Decimal(1).scaleb(-(count - 1)), rounding=decimal.ROUND_HALF_UP)
    digits = format(rounded, "f").replace(".", "")
    return digits, exponent


def exponential(x, digits):
    if x == 0:
        mantissa, exponent = "0" * (digits + 1), 0
    else:
        mantissa, exponent = significant(x, digits + 1)
    body = mantissa[0] + ("." + mantissa[1:] if len(mantissa) > 1 else "")
    sign = "+" if exponent >= 0 else "-"
    return ("-" if x < 0 else "") + f"{body}e{sign}{abs(exponent)}"


def precision(x, count):
    if x == 0:
        mantissa, exponent = "0" * count, 0
    else:
        mantissa, exponent = significant(x, count)
    sign = "-" if x < 0 else ""
    if exponent < -6 or exponent >= count:
        body = mantissa[0] + ("." + mantissa[1:] if count > 1 else "")
        return f"{sign}{body}e{'+' if exponent >= 0 else '-'}{abs(exponent)}"
    if exponent == count - 1:
        return sign + mantissa
    if exponent >= 0:
        return sign + mantissa[: exponent + 1] + "." + mantissa[exponent + 1 :]
    return sign + "0." + "0" * (-(exponent + 1)) + mantissa


def main():
    random.seed(7)
    values = [0.5, 1.5, 2.5, 1.005, 1.45, 8.345, 0.000001, 1e-7, 123.456, 1e20, 9.995, 0.1, 0.3, 99.99, 5e-324, 1.7976931348623157e308]
    for _ in range(1500):
        bits = random.getrandbits(64)
        value = struct.unpack("<d", struct.pack("<Q", bits))[0]
        if value != value or value in (float("inf"), float("-inf")):
            continue
        values.append(value)
        values.append(round(random.uniform(-1000, 1000), random.randint(0, 6)))
    lines = []
    for value in values:
        digits = random.randint(0, 20)
        count = random.randint(1, 21)
        result = fixed(value, digits)
        lines.append("\t".join([
            f"{struct.unpack('<Q', struct.pack('<d', value))[0]:016X}",
            str(digits),
            result if result is not None else "-",
            exponential(value, digits),
            str(count),
            precision(value, count),
        ]))
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as file:
        file.write("\n".join(lines) + "\n")
    print(f"{len(lines)} vectors")


main()
