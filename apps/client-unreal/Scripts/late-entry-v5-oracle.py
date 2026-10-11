#!/usr/bin/env python3
"""V5 late-entry regression consumer and exact harness geometry/budget.

Pinned game abfde049: Formulas.java:263-351 (NPC regen), :665-738 (auto
attack), :1054-1063 (hit chance); dp3ca488dd stats/npcs/20500-20599.xml:3659
supplies hpRegen2. The production fixture keeps its custom physical stats.
C11/C15: phase3-evidence.regen calls the independent Fraction oracle using
actual recorded inputs, including pulses on the first impact's own tick.

The old r9 observer arrived at tick596: durable HP256/256, two dead statics.
First wound613: damage17, HP239. Fixed sleep12 assumed legacy 5tile/s and
shorter kills. New handshake observes the wound, exits at(62,90), returns to
(70,90); A stops until B leaves/re-enters and remains until B acknowledges by
leaving again. No RNG or fixed sleep decides when the wound is ready.

AOI uses32-tile cells (domain/zone/aoi.rs:13): x62 cell1 excludes meadow
x>=96 cell3; x70 cell2 includes it. Spawn homes100/104 and speed1 milli-tile
per tick are explicit custom fixture inputs (phase1a-late-entry/zones and
npcs). Handshake8 tiles/3.95tile/s; 3x plus3s delivery <15s. Human Fighter
speed395 comes from the source FuncMoveSpeed.java:43 via the committed
V5 Fraction calculation (Scenarios/2-racial-traits.nfs:2-5), not legacy Speed::DEFAULT.
All movement budgets use3x Manhattan distance/speed plus3s delivery, rounded
up to5s; Within covers the sum of WaitFor bounds plus30s exit allowance.
The wound wait90 and receipt15/30 are explicit harness allowances; no
finite timeout promises success for every possible random miss stream.
"""
import copy
from fractions import Fraction as F
import importlib.util
import json
from pathlib import Path
import sys
import shlex

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('late_evidence', SCRIPTS / 'phase3-evidence.py')
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)


def budget(distance):
    # Source FuncMoveSpeed.java:43; packages/data/tables/stat_bonus.toml:47 DEX30=1.10;
    # packages/data/professions/human_fighter.toml:22 run115.
    speed = (F(115) * F('1.10') * 1000 // 320) / F(100)
    return 5 * evidence.oracle.ceil((F(distance) / speed * 3 + 3) / 5)


def check_scenarios():
    expected = {
        '1-late-entry-a': {'40 40': budget(80), '80 85': budget(85), '97 100': budget(32)},
        '1-late-entry-b': {'20 60': budget(80), '70 90': budget(80), '62 90': budget(8)},
    }
    for name, paths in expected.items():
        total, within = 30, None  # harness exit/delivery allowance
        for line in (SCRIPTS.parent / 'Scenarios' / f'{name}.nfs').read_text().splitlines():
            words = shlex.split(line.split('#', 1)[0])
            if not words:
                continue
            if words[0] == 'nf.WaitFor':
                total += int(words[-1])
                if words[1] == 'own_at':
                    path = ' '.join(words[2:4])
                    # The second70,90 leg is the8-tile return handshake.
                    minimum = budget(8) if path == '70 90' and int(words[-1]) == 15 else paths[path]
                    if int(words[-1]) < minimum:
                        raise ValueError(f'{name}: movement budget below exact derivation')
            if words[0] == 'nf.Within':
                within = int(words[1])
        if within is None or within < total:
            raise ValueError(f'{name}: Within must cover {total} seconds of allowances')


def verify(recording, raw, observer):
    check_scenarios()
    gates = [{'kind': 'late_entry_hp_continuity', 'args': ['keltir_durable'], 'line': 1}]
    rows = evidence.read_jsonl(raw)
    result = evidence.evaluate(rows, recording, observer, gates)
    if not result['passed']:
        raise ValueError(result)
    # Negative control: a forged wounded spawn must fail the same actual-input gate.
    altered = copy.deepcopy(rows)
    for row in altered[1:-1]:
        if row['kind'] != 'observer_combat' or row['entity'] != observer:
            continue
        for event in row['data']['events']:
            if event['type'] == 'entity_spawn' and event['combat']['hp'] < event['combat']['max_hp'] and not event['combat']['dead']:
                event['combat']['hp'] -= 1
    if evidence.evaluate(altered, recording, observer, gates)['passed']:
        raise ValueError('corrupted baseline accepted')
    return result


if __name__ == '__main__':
    print(json.dumps(verify(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]), indent=2))
