"""Independent unbounded Fraction oracle, Formulas.java:1286-1301 @ abfde049.

All admitted level gaps; source binary32 ties-to-even, then Math.round(float)
with Java's signed-int saturation. No bounded runtime power algorithm is reused.
Run from any directory to regenerate vectors.json. packages/data stays read-only.
"""
from fractions import Fraction
import json
from pathlib import Path


def binary32(value):
    if not value:
        return value
    exponent = value.numerator.bit_length() - value.denominator.bit_length()
    power = Fraction(2) ** exponent
    if value < power:
        exponent -= 1
    quantum = Fraction(2) ** (exponent - 23)
    scaled = value / quantum
    whole, remainder = divmod(scaled.numerator, scaled.denominator)
    twice = remainder * 2
    whole += twice > scaled.denominator or (
        twice == scaled.denominator and whole % 2 == 1
    )
    return whole * quantum


vectors = []
for gap in range(-84, 85):
    for modifier in ("0", "0.5", "1", "2.5", "3", "3.25", "3.5"):
        value = binary32(Fraction(13, 10) ** gap * Fraction(modifier))
        rounded = min((value + Fraction(1, 2)).__floor__(), 2**31 - 1)
        vectors.append({"gap": gap, "modifier": modifier, "rate": 100 - rounded})
Path(__file__).with_name("vectors.json").write_text(json.dumps(vectors, indent=2) + "\n")
