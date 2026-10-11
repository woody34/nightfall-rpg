#!/usr/bin/env python3
"""Audit actual verified token replay inputs and derive a bounded V5 death wait."""
from fractions import Fraction as F
import importlib.util
import json
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('skill_oracle', REPO / 'packages/data/scripts/skill_oracle.py')
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


def death_budget(player_speed, sentinel_speed, movement_speed):
    # Fixture policy: allow 64 player rolls and 3 sentinel rolls, with 3 seconds
    # delivery grace; round the result up to a 30-second harness budget quantum.
    # These are test allowances, not L2J constants or a guarantee for arbitrary RNG.
    # home separation6: token-fixtures.py overlay_zone_content/phase3-fixtures.py.
    # Drift per axis300/32 tiles: zone/ai.rs:24-30; inherited pinned MAX_DRIFT_RANGE.
    # Double direct Manhattan travel allowance includes a moving target's wander.
    travel = 2 * (F(6) + 2 * F(300, 32)) / F(movement_speed)
    _, player_interval = oracle.calc_auto_timing_ticks(player_speed)
    _, npc_interval = oracle.calc_auto_timing_ticks(sentinel_speed)
    seconds = travel + F(64 * player_interval + 3 * npc_interval, 10) + 3
    return 30 * oracle.ceil(seconds / 30)


def verify(rows):
    if not rows or rows[0]['kind'] != 'manifest' or rows[-1]['kind'] != 'complete':
        raise ValueError('verified complete native evidence required')
    attacks = []
    budgets = []
    for row in rows:
        if row['kind'] != 'auto_attack_resolution':
            continue
        data = row['data']
        source = data['inputs']['source']['entity']
        target = data['inputs']['target']
        results = [e for e in data['events'] if e['type'] == 'attack_result' and e['attacker'] == source['id']]
        if not results:
            continue  # Cancelled swings have no hit roll.
        if len(results) != 1:
            raise ValueError('ambiguous auto result')
        result = results[0]
        a, b = source['combat']['sheet'], target['combat']['sheet']
        draws = {d['callsite']: d['value'] for d in data['rng_draws']}
        chance = oracle.calc_auto_hit_chance(oracle.rounded(F(a['accuracy'], 1000000)),
                                            oracle.rounded(F(b['evasion'], 1000000)))
        hit = draws['AutoHit'] <= chance
        if hit != (result['outcome'] != 'miss'):
            raise ValueError('auto hit differs from pinned chance')
        if hit and (draws['AutoCritical'] < a['crit_permille']) != (result['outcome'] == 'crit'):
            raise ValueError('auto critical differs from pinned chance')
        sentinel = source['combat']['role'].get('template') == 'token_sentinel'
        if sentinel:
            budgets.append(death_budget(oracle.rounded(F(b['attack_speed'], 1000000)),
                oracle.rounded(F(a['attack_speed'], 1000000)), F(target['speed'], 100)))
        if sentinel and hit:
            spread = F(100 + draws['Spread'] - a['random_damage'], 100)
            rolled = oracle.floor(oracle.calc_auto_damage(F(a['p_atk'], 1000000),
                F(b['p_def'], 1000000), result['outcome'] == 'crit', spread))
            hp = target['combat']['hp']
            if (result['damage'] != min(hp, rolled) or result['target_hp_after'] != max(0, hp - rolled)
                    or result['phase3']['cp_absorbed'] != 0):
                raise ValueError('NPC HP damage or CP bypass differs from oracle')
            if rolled < b['max_hp']:
                raise ValueError('fixture sentinel no longer lethal at full HP')
        attacks.append({'tick': row['tick'], 'source': 'sentinel' if sentinel else 'player',
                        'chance': chance, 'roll': draws['AutoHit'], 'outcome': result['outcome'],
                        'hp_removed': result['damage'], 'hp_after': result['target_hp_after']})
    if not any(a['source'] == 'sentinel' and a['hp_after'] == 0 for a in attacks):
        raise ValueError('missing lethal sentinel evidence')
    return {'attacks': attacks, 'death_wait_seconds': max(budgets),
            'player_64_miss_probability': str(F(799, 1000) ** 64)}


if __name__ == '__main__':
    print(json.dumps(verify([json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]), indent=2))
