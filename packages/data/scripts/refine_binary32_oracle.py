#!/usr/bin/env python3
"""Independent binary32 oracle for retail refine selection (D86).

AugmentationData.java:464-468/497-503/518-522/550-555/705-709/729-734 accumulate `float s = 10000` and subtract
`getAugmentChance() * 100` (a float parsed by Float.parseFloat) while `s > r`. This script
parses the pinned XML directly and emulates binary32 with struct round trips: every
intermediate is an exact double (24-bit operands, small exponents) rounded once to float.
It records, per table, each residue whose float result differs from exact decimal arithmetic.
"""
import argparse
import bisect
from fractions import Fraction
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import xml.etree.ElementTree as ET

try:
    import gen_skills
except ModuleNotFoundError:
    from packages.data.scripts import gen_skills

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'fixtures/refine-binary32-oracle.json'
XML = 'src/main/resources/data/stats/augmentation/'
FILES = (('weapon', 'retailchances.xml'), ('accessory', 'retailchances_accessory.xml'))
RESIDUES = 10000


def f32(value: float) -> float:
    return struct.unpack('<f', struct.pack('<f', value))[0]


def float_thresholds(chances: list[str]) -> list[float]:
    """`s` before each row: Java evaluates `s > r` against this value."""
    s, out = f32(10000.0), []
    for chance in chances:
        out.append(s)
        s = f32(s - f32(f32(float(chance)) * 100.0))
    return out


def exact_thresholds(chances: list[str]) -> list[Fraction]:
    s, out = Fraction(10000), []
    for chance in chances:
        out.append(s)
        s -= Fraction(chance) * 100
    return out


def choose(options: list[int], ascending: list, residue: int) -> int:
    """Once `s <= r` the loop never subtracts again, so the selection is the last row whose
    preceding accumulator exceeds the residue. `ascending` is the thresholds reversed; positive
    chances make them strictly ascending, so the visited count is those above the residue."""
    visited = len(ascending) - bisect.bisect_right(ascending, residue)
    return options[visited - 1] if visited else 0


def generate(datapack: Path) -> dict:
    revision = subprocess.run(['git', '-C', str(datapack), 'rev-parse', 'HEAD'],
                              check=True, capture_output=True, text=True).stdout.strip()
    if revision != gen_skills.DATAPACK_REVISION:
        raise SystemExit(f'datapack at {revision}, expected {gen_skills.DATAPACK_REVISION}')
    tables, sources = [], []
    for kind, name in FILES:
        path = datapack / XML / name
        sources.append({'path': XML + name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
        for weapon in ET.parse(path).getroot().findall('weapon'):
            for stone in weapon.findall('stone'):
                groups: dict[str, list[tuple[int, str]]] = {}
                for variation in stone.findall('variation'):
                    for category in variation.findall('category'):
                        key = 'option12' if variation.get('id') == '1' else category.get('probability')
                        for augment in category.findall('augment'):
                            groups.setdefault(key, []).append((int(augment.get('id')), augment.get('chance')))
                for selector, rows in groups.items():
                    options = [option for option, _ in rows]
                    chances = [chance for _, chance in rows]
                    if any(Fraction(chance) <= 0 for chance in chances):
                        raise SystemExit('non-positive retail chance')
                    floats = float_thresholds(chances)[::-1]
                    exact = exact_thresholds(chances)[::-1]
                    divergent = []
                    for residue in range(RESIDUES):
                        java = choose(options, floats, residue)
                        decimal = choose(options, exact, residue)
                        if java != decimal:
                            divergent.append([residue, java, decimal])
                    if divergent:
                        tables.append({'kind': kind, 'weapon_type': weapon.get('type'),
                                       'stone': int(stone.get('id')), 'selector': selector,
                                       'rows': len(rows), 'residues': divergent})
    return {
        'description': 'Residues where Java binary32 refine selection differs from exact decimals; '
                       'each entry is [residue, java_option, exact_decimal_option].',
        'datapack_revision': gen_skills.DATAPACK_REVISION,
        'source_lines': ['datatables/AugmentationData.java:334,409 (Float.parseFloat)',
                         'datatables/AugmentationData.java:464-468,497-503,518-522,550-555,705-709,729-734'],
        'generator': 'packages/data/scripts/refine_binary32_oracle.py',
        'sources': sources,
        'tables': tables,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--datapack', type=Path, default=gen_skills.DEFAULT_DATAPACK_PATH)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    text = json.dumps(generate(args.datapack), indent=1, sort_keys=True) + '\n'
    if args.check:
        if OUT.read_text() != text:
            raise SystemExit(f'{OUT} is stale')
        return
    OUT.write_text(text)


if __name__ == '__main__':
    main()
