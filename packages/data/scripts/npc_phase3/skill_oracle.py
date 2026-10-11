#!/usr/bin/env python3
"""Separate Fraction oracle; preserve all historical skill_oracle bytes/hashes."""
import argparse
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import sys
import tomllib

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from skill_oracle import binary_nearest, rounded  # noqa: E402

ROOT = SCRIPTS.parent
OUT = ROOT / 'fixtures/skill-oracle-npc-phase3.json'


def generate():
    penalty_path = 'tables-phase3/npc_level_penalty.toml'
    hit_path = 'tables-phase3/hit_condition_bonus.toml'
    penalty = tomllib.loads((ROOT / penalty_path).read_text())
    hit = tomllib.loads((ROOT / hit_path).read_text())
    config = penalty['config']
    threshold = int(config['MinNPCLevelForDmgPenalty']['values'][0])
    formulas = {key: [] for key in ('calcPhysDam_npcLevelPenalty',
        'calcSkillPhysDam_npcLevelPenalty', 'calcMagicDam_npcLevelPenalty',
        'calcMagicSuccess_npcChancePenalty', 'getConditionBonus')}
    cases = [(85, 85 - gap, True, False, False, True) for gap in range(-1, 13)]
    cases += [(77, 70, True, False, False, True), (78, 76, True, False, False, True),
              (78, 75, True, False, False, True), (85, 80, False, False, False, True),
              (85, 80, True, True, False, True), (85, 80, True, False, True, True),
              (85, 80, True, False, False, False)]
    def lookup(key, index):
        values = config[key]['values']
        return F(values[min(index, len(values) - 1)])
    for target, player, attackable, raid, minion, acting in cases:
        gap = target - player
        base_gate = attackable and not raid and not minion and acting and target >= threshold
        inputs = {'target_level': target, 'acting_player_level': player,
                  'is_attackable': attackable, 'is_l2attackable': attackable,
                  'is_raid': raid, 'is_raid_minion': minion, 'has_acting_player': acting,
                  'rolls': []}
        for crit in (False, True):
            gate = base_gate and gap >= 2
            index = gap - 1
            normal = lookup('CritDmgPenaltyForLvLDifferences' if crit else 'DmgPenaltyForLvLDifferences', index) if gate else F(1)
            skill = lookup('SkillDmgPenaltyForLvLDifferences', index) if gate else F(1)
            for key, factor, lines in (
                ('calcPhysDam_npcLevelPenalty', normal, '713-731'),
                ('calcSkillPhysDam_npcLevelPenalty', normal * skill, '776-800'),
                ('calcMagicDam_npcLevelPenalty', skill, '887-898')):
                amount = F(1001) * factor
                formulas[key].append({'inputs': dict(inputs, critical=crit, incoming_damage=1001),
                    'source_lines': ['model/stats/Formulas.java:' + lines, 'config/npc.properties:54,61-63'],
                    'eligible': gate, 'lookup_index': index if gate else -1,
                    'exact_multiplier': str(factor), 'multiplier_q': int(factor * 1000000),
                    'exact_rational': str(amount), 'whole_damage': amount.numerator // amount.denominator,
                    'rounding_rule': 'Exact penalty-stage Fraction; whole truncation evidence only. No earlier-stage rounding or Java binary64 equivalence claim.'})
        gate = base_gate and gap >= 3
        index = gap - 2
        factor = lookup('SkillChancePenaltyForLvLDifferences', index) if gate else F(1)
        # skill magic level fixed at target level: isolates this targetModifier.
        raw = F(13, 10) ** 0 * factor * F(1)
        rate = 100 - rounded(binary_nearest(raw, 24))
        formulas['calcMagicSuccess_npcChancePenalty'].append({
            'inputs': dict(inputs, skill_magic_level=target, resist_modifier='1', roll=rate - 1),
            'source_lines': ['model/stats/Formulas.java:1286-1314', 'config/npc.properties:54,70,74'],
            'eligible': gate, 'lookup_index': index if gate else -1,
            'exact_multiplier': str(factor), 'multiplier_q': int(factor * 1000000),
            'exact_rational': str(raw), 'success_rate': rate,
            'lands_below_rate': True, 'lands_at_rate': False,
            'rounding_rule': 'Math.round binary32 failure subtrahend; Rnd.get(100) < rate. magicLvl isolates targetModifier; separate magic threshold unused.'})
    bonuses = {r['condition']: r['percent'] for r in hit['rows']}
    for height in (-51, -50, 0, 50, 51):
        for night in (False, True):
            for position in ('front', 'side', 'back'):
                for rain in (False, True):
                    mod = 100 + bonuses[position] + (bonuses['dark'] if night else 0)
                    mod += bonuses['high'] if height > 50 else bonuses['low'] if height < -50 else 0
                    result = max(F(mod, 100), F(0))
                    formulas['getConditionBonus'].append({
                        'inputs': {'z_difference': height, 'night': night, 'position': position, 'rain': rain},
                        'source_lines': ['data/xml/impl/HitConditionBonusData.java:92-118', 'stats/hitConditionBonus.xml:3-9'],
                        'exact_rational': str(result), 'multiplier_q': int(result * 1000000),
                        'rounding_rule': 'Exact percentage sum divided by 100, lower bound zero; rain ignored by source.'})
    paths = [penalty_path, hit_path, 'scripts/npc_phase3/skill_oracle.py', 'scripts/skill_oracle.py']
    golden = {'schema': 1, 'source_revisions': {'l2j_game': penalty['source']['revision'],
              'l2j_datapack': hit['source']['revision']},
              'inputs_sha256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sorted(paths)},
              'formulas': formulas,
              'vector_set_sha256': hashlib.sha256(json.dumps(formulas, sort_keys=True, separators=(',', ':')).encode()).hexdigest()}
    return golden, json.dumps(golden, indent=2) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    golden, content = generate()
    if args.check:
        if not OUT.exists() or OUT.read_text() != content:
            raise SystemExit(f'Stale fixture: {OUT}')
    else:
        OUT.write_text(content)
    print('npc_phase3/skill_oracle: ' + str({k: len(v) for k, v in golden['formulas'].items()}))


if __name__ == '__main__':
    main()
