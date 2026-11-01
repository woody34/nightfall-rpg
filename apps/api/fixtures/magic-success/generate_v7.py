"""Independent V7 oracle for calcMagicSuccess (Formulas.java:1286-1301 @ abfde049; design E32).

Under V7 the level modifier is Java's Math.pow(1.3, d): the captured OpenJDK 21 raw bits of
packages/data/jvm-pow-tables.txt (pow13, d = -120..120). Then, left to right in double,
fl(fl(lvlModifier * targetModifier) * resModifier), the (float) cast with ties-to-even, and
Math.round(float) with Java's signed-int saturation. Exact Fractions throughout; no runtime
power or rounding code is reused. Run from any directory to regenerate vectors_v7.json.
"""
from fractions import Fraction
import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]


def round_bits(value, mantissa_bits):
    """Round a positive Fraction to the nearest binary float with `mantissa_bits` fraction
    bits (ties to even); normal range only, which every vector stays in."""
    if not value:
        return value
    exponent = value.numerator.bit_length() - value.denominator.bit_length()
    if value < Fraction(2) ** exponent:
        exponent -= 1
    quantum = Fraction(2) ** (exponent - mantissa_bits)
    scaled = value / quantum
    whole, remainder = divmod(scaled.numerator, scaled.denominator)
    twice = remainder * 2
    whole += twice > scaled.denominator or (twice == scaled.denominator and whole % 2 == 1)
    return whole * quantum


def fl(value):
    return round_bits(value, 52)


def fl32(value):
    return round_bits(value, 23)


def pow13_table():
    table = {}
    for line in (ROOT / "packages/data/jvm-pow-tables.txt").read_text().splitlines():
        if line.startswith("pow13 d="):
            head, bits = line.split(" bits=")
            d = int(head.removeprefix("pow13 d="))
            table[d] = Fraction(struct.unpack(">d", bytes.fromhex(bits))[0])
    return table


POW13 = pow13_table()
vectors = []
for gap in range(-84, 85):
    for modifier in ("0", "0.5", "1", "2.5", "3", "3.25", "3.5"):
        target = fl(Fraction(modifier))
        value = fl32(fl(fl(POW13[gap] * target) * 1))
        rounded = min((value + Fraction(1, 2)).__floor__(), 2**31 - 1)
        vectors.append({"gap": gap, "modifier": modifier, "rate": 100 - rounded})
Path(__file__).with_name("vectors_v7.json").write_text(json.dumps(vectors, indent=2) + "\n")
