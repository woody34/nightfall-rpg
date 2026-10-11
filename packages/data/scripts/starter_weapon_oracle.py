#!/usr/bin/env python3
"""Independent Fraction oracle for equipped Phase 3 starter-weapon M.Atk."""
import argparse
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]
KEYS = ('human_fighter', 'human_mystic', 'elven_mystic', 'dark_fighter',
        'orc_mystic', 'dwarven_fighter')
LEVELS = (1, 20, 40, 85)
OUT = ROOT / 'fixtures/skill-oracle-starter-weapon-phase3.json'


def generate() -> tuple[dict, str]:
    weapon_rel = 'tables-phase3/starter_weapon.toml'
    weapon = tomllib.loads((ROOT / weapon_rel).read_text())
    func = next(f for f in weapon['functions'] if f['stat'] == 'mAtk')
    if func['operation'] != 'SET' or func['order'] != 0:
        raise ValueError('expected order-0 weapon M.Atk SET')
    bonus_table = tomllib.loads((ROOT / 'tables/stat_bonus.toml').read_text())
    rows = []
    paths = [weapon_rel, 'tables/stat_bonus.toml', 'scripts/starter_weapon_oracle.py']
    for key in KEYS:
        rel = f'classes-phase3/{key}.toml'
        paths.append(rel)
        profile = tomllib.loads((ROOT / rel).read_text())
        for level in LEVELS:
            # FuncSet.java:36-40 returns getValue(), ignoring template initVal.
            base = F(func['value'])
            stat = profile['base_stats']['int']
            bonus = F(bonus_table['int']['bonus'][stat])
            level_mod = F(level + 89, 100)  # L2Character.java:5007-5008
            exact = base * bonus * bonus * level_mod * level_mod
            rows.append({
                'description': f"{profile['l2_ref']} level {level}: equipped Squire's Sword",
                'source_lines': ['model/actor/stat/CharStat.java:237-246',
                                 'model/stats/functions/FuncSet.java:36-40',
                                 'model/stats/functions/formulas/FuncMAtkMod.java:37-46',
                                 'model/actor/L2Character.java:5007-5008'],
                'template_source': profile['source'], 'weapon_source': func['source'],
                'inputs': {'class_id': profile['id'], 'class_key': key, 'level': level,
                           'template_file': rel, 'is_player': True,
                           'equipment': 'squires_sword_empty_jewelry', 'weapon_file': weapon_rel,
                           'item_id': weapon['item_id'], 'weapon_type': weapon['weapon_type'],
                           'template_base': profile['magical']['base_m_atk'],
                           'base': int(base), 'stat_name': 'INT', 'stat_value': stat,
                           'modifiers': [], 'rolls': []},
                'intermediate': {'after_order_0_set': str(base),
                                 'weapon_order': func['order'], 'builtin_order': 1,
                                 'stat_bonus': str(bonus), 'level_mod': str(level_mod),
                                 'unarmed_exact': str(F(profile['magical']['base_m_atk']) * bonus**2 * level_mod**2),
                                 'incorrect_additive_exact': str((F(profile['magical']['base_m_atk']) + base) * bonus**2 * level_mod**2)},
                'exact_rational': str(exact),
                'q_result': exact.numerator * 1000000 // exact.denominator,
                'rounded_result': exact.numerator // exact.denominator,
                'rounding_rule': 'SET replaces template base at order 0; FuncMAtkMod multiplies at order 1. Exact Fraction to Q once; whole floor for display only.',
            })
    formulas = {'FuncMAtkMod': rows}
    golden = {
        'schema': 1,
        'source_revisions': {'l2j_game': weapon['ordering']['set_source']['revision'],
                             'l2j_datapack': weapon['source']['revision']},
        'policy': 'Exact Fraction formula evidence and Q adaptation; not a Java binary64 equivalence claim.',
        'coverage': {'class_keys': list(KEYS), 'levels': list(LEVELS), 'item_id': weapon['item_id']},
        'inputs_sha256': {rel: hashlib.sha256((ROOT / rel).read_bytes()).hexdigest() for rel in sorted(paths)},
        'formulas': formulas,
        'vector_set_sha256': hashlib.sha256(json.dumps(formulas, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
    }
    return golden, json.dumps(golden, indent=2) + '\n'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    _, content = generate()
    if args.check:
        if not OUT.exists() or OUT.read_text() != content:
            raise SystemExit(f'Stale fixture: {OUT}')
    else:
        OUT.write_text(content)
    print('starter_weapon_oracle: 24 equipped M.Atk vectors; ' + ('check passed' if args.check else 'generated'))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
