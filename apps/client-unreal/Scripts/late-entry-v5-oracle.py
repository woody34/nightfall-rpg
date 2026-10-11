#!/usr/bin/env python3
"""V5 late-entry regression consumer and exact harness geometry/budget.

Pinned game abfde049: Formulas.java:263-351 (NPC regen), :665-738 (auto
attack), :1054-1063 (hit chance); dp3ca488dd stats/npcs/20500-20599.xml:3659
supplies hpRegen2. The production fixture keeps its custom physical stats.
C11/C15: phase3-evidence.regen calls the independent Fraction oracle using
actual recorded inputs, including pulses on the first impact's own tick.

Fresh r11 re-entry583 received a same-tick NPC swing583/593/603. The gate
correctly rejected that timing; no baseline was omitted. The native pre-check
must require SUCCESS on every generated epoch, not accept one expected failure.

The durable fixture's HARNESS speed floor(253/10)=25 yields100 impact ticks:
pinned dp3ca488dd stats/npcs/20500-20599.xml:3660 supplies253;
game abfde049 L2Character.java:4336 gives500000/speed and :887 half to impact.
The factor10 is a test scheduling choice, not a claimed source combat value.
B waits for >=95 RECEIVED impact ticks, exits x63/cell1 and returns x65/cell2
(domain/zone/aoi.rs:13). Three3s waits bound that roundtrip at90ticks, strictly
inside the pending swing; position tolerance0.05 prevents early crossing claims.
A retains the hydrated swing through its received impact before resuming hits.
The gate still demands native start < re-entry < impact and exact first-hit HP.

Human Fighter speed395 is source FuncMoveSpeed.java:43 via the independent
Fraction calculation below (DEX30=1.10, run115). Two tiles take ceil(2000/395)=6
ticks, so the short-leg3s allowances contain5x the exact travel time. Long
travel keeps3x Manhattan distance/speed plus3s delivery rounded up5s. Within
covers every WaitFor budget plus30s exit allowance. No finite wait guarantees
success under arbitrarily delayed delivery; missed windows fail closed.
"""
import copy
from fractions import Fraction as F
import importlib.util
import json
from pathlib import Path
import sys
import shlex
import tomllib

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
        '1-late-entry-b': {'20 60': budget(80), '65 90': budget(75), '63 90': 3},
    }
    fixture = tomllib.loads((SCRIPTS.parents[1] / 'api/fixtures/phase1a-late-entry/npcs/keltir_durable.toml').read_text())
    # dp3ca488dd stats/npcs/20500-20599.xml:3660; factor10 is HARNESS policy.
    if fixture['attack_speed'] != 253 // 10:
        raise ValueError('durable windup lost its documented fixture derivation')
    impact, ready = evidence.oracle.calc_auto_timing_ticks(fixture['attack_speed'])
    if (impact, ready) != (100, 200):
        raise ValueError('independent source attack timing changed')
    for name, paths in expected.items():
        text = (SCRIPTS.parent / 'Scenarios' / f'{name}.nfs').read_text()
        if name.endswith('-b'):
            if 'nf.WaitFor target_swing_impact_remaining >= 95 30' not in text:
                raise ValueError('late entry must be coordinated by a received pending swing')
            steps = [shlex.split(line.split('#', 1)[0]) for line in text.splitlines()]
            phase = next(i for i, words in enumerate(steps)
                         if words[:3] == ['nf.WaitFor', 'target_swing_impact_remaining', '>='])
            end = next(i for i in range(phase + 1, len(steps)) if steps[i][:1] == ['nf.Target'])
            crossing = [words for words in steps[phase+1:end] if words]
            moves = [words[1:] for words in crossing if words[0] == 'nf.ClickMove']
            if moves != [['63', '90'], ['65', '90']]:
                raise ValueError('AOI crossing must depart cell1 then enter cell2')
            received_margin = int(steps[phase][-2])
            roundtrip = sum(int(words[-1]) * 10 for words in crossing if words[0] == 'nf.WaitFor')
            if not 0 < roundtrip < received_margin <= impact:
                raise ValueError('AOI roundtrip budgets exceed the received windup')
        if 'nf.WaitFor target_swing_impact_remaining == 0 15' not in text:
            raise ValueError('both endpoints must retain the hydrated swing through impact')
        total, within = 30, None  # harness exit/delivery allowance
        for line in (SCRIPTS.parent / 'Scenarios' / f'{name}.nfs').read_text().splitlines():
            words = shlex.split(line.split('#', 1)[0])
            if not words:
                continue
            if words[0] == 'nf.WaitFor':
                total += int(words[-1])
                if words[1] == 'own_at':
                    path = ' '.join(words[2:4])
                    # Short boundary legs have explicit3s harness delivery allowances.
                    minimum = 3 if path == '65 90' and int(words[-1]) == 3 else paths[path]
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
    errors = result['gates'][0]['errors']
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
    corrupted = evidence.evaluate(altered, recording, observer, gates)
    if corrupted['passed'] or corrupted['gates'][0]['errors'] == errors:
        raise ValueError('corrupted baseline accepted')
    return result


if __name__ == '__main__':
    print(json.dumps(verify(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]), indent=2))
