#!/usr/bin/env python3
"""Fail-closed C11 replay gates. Raw entity/tick facts, never producer PASS flags.

Producer handoff: nightfall-replay trace --evidence-out JSON. Until that producer
exists, affected units fail with missing evidence; a green DSL audit is not acceptance.
"""
import argparse
from fractions import Fraction
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import uuid
import xml.etree.ElementTree as ET

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parents[2]
FACTS = frozenset(('regen_pulses', 'moving_regen_pulses', 'regen_matches_oracle',
    'moving_regen_matches_oracle', 'death_record_matches_fixture',
    'death_record_matches_before_disconnect', 'restore_did_not_reapply_hooks',
    'skill_hate_outcome_coherent', 'skill_damage_matches_oracle',
    'skill_effect_matches_oracle', 'stat_matches_oracle', 'resurrect_refund_matches_oracle'))
ORACLE_FORMULAS = {
    'skill_damage_matches_oracle': {'calcSkillPhysDam', 'calcMagicDam'},
    'skill_effect_matches_oracle': {'calcEffectSuccess', 'effect_duration'},
    'stat_matches_oracle': {'FuncMAtkMod', 'FuncMDefMod', 'FuncMAtkSpeed', 'FuncAtkEvasion'},
}


def integer(value, minimum=0):
    if type(value) is not int or value < minimum:
        raise ValueError('nonnegative integer evidence required')
    return value


def ratio(value):
    if type(value) is not int and not isinstance(value, str):
        raise ValueError('integer or exact rational string required')
    return Fraction(value)


def rounded(value):
    # Math.round at L2PcInstance.java:4702; exact positive fraction arithmetic.
    return (value + Fraction(1, 2)).numerator // (value + Fraction(1, 2)).denominator


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate evidence JSON key {key}')
        result[key] = value
    return result


def requirements(path):
    result = []
    for line, text in enumerate(path.read_text().splitlines(), 1):
        if not text.startswith('# replay-gate:'):
            continue
        tokens = shlex.split(text.split(':', 1)[1])
        if not tokens or tokens[0] not in FACTS:
            raise ValueError(f'{path}:{line}: unknown replay gate')
        kind, args = tokens[0], tokens[1:]
        if kind in ('regen_pulses', 'moving_regen_pulses'):
            if len(args) != 2 or args[0] != '>=' or not args[1].isdigit() or int(args[1]) <= 0:
                raise ValueError('pulse gate requires >= positive count')
        elif kind in ('skill_damage_matches_oracle', 'skill_effect_matches_oracle'):
            if len(args) != 1 or not args[0].isdigit() or not 0 < int(args[0]) < 2**32:
                raise ValueError('skill gate requires positive skill id')
        elif kind == 'stat_matches_oracle':
            if args not in (['active'], ['removed']):
                raise ValueError('stat gate requires active or removed phase')
        elif args:
            raise ValueError(f'{kind} takes no arguments')
        result.append({'kind': kind, 'args': args, 'line': line})
    return result


def oracle_value(kind, inputs):
    """Exact inputs must match an independent committed source vector, including rolls.

    Unsupported live input vectors refuse acceptance; no inferred/default expectation.
    """
    formula = inputs['formula']
    if formula not in ORACLE_FORMULAS[kind]:
        raise ValueError('wrong independent formula for gate')
    oracle = json.loads((REPO / 'packages/data/fixtures/skill-oracle.json').read_text())
    if inputs['oracle_sha256'] != hashlib.sha256(
            (REPO / 'packages/data/fixtures/skill-oracle.json').read_bytes()).hexdigest():
        raise ValueError('oracle source hash mismatch')
    vector = oracle['formulas'][formula][integer(inputs['vector_index'])]
    if inputs['formula_inputs'] != vector['inputs']:
        raise ValueError('live inputs differ from independent oracle vector')
    if kind == 'skill_damage_matches_oracle':
        source = vector['inputs']
        if formula == 'calcMagicDam' and source['magic_failure']:
            outcome = 'half' if source['second_success'] and source['lvl_diff'] <= 9 else 'resist'
        else:
            outcome = 'crit' if source.get('mcrit', source.get('crit', False)) else 'hit'
        return {'rolled_damage': vector['rounded_result'], 'outcome': outcome}
    if kind == 'skill_effect_matches_oracle':
        if formula == 'effect_duration':
            return {'duration_ticks': vector['ticks_result']}
        source = vector['inputs']
        unconditional = source['activate_rate'] == -1 or source['basic_property'] == 'NONE'
        roll = inputs['effect_roll']
        if unconditional:
            if roll is not None:
                raise ValueError('unconditional effect must not consume a success draw')
            landed = True
        else:
            integer(roll)
            if roll >= 100:
                raise ValueError('effect roll outside source bound')
            landed = ratio(vector['exact_rational']) > roll
        return {'landed': landed, 'chance_q': vector['q_result']}
    if formula == 'FuncAtkEvasion':
        return {'whole': vector['rounded_result']['evasion_with_intermediate_cast']}
    if formula == 'FuncMAtkSpeed':
        return {'whole': vector['rounded_result']}
    return {'value_q': vector['q_result']}


def expected(row):
    kind, data = row['kind'], row['inputs']
    if kind in ORACLE_FORMULAS:
        return oracle_value(kind, data)
    if kind in ('regen_matches_oracle', 'moving_regen_matches_oracle'):
        # Formulas.java:263-450; D-R12 Q accumulation and clamp. The producer supplies
        # source profile/bonus/function-fold inputs at the actual regen phase boundary.
        posture = 'moving' if kind.startswith('moving_') else 'standing'
        if data['posture'] != posture or data['alive'] is not True:
            raise ValueError('wrong regen phase/life')
        result = {}
        for resource, item in data['resources'].items():
            if resource not in ('hp', 'mp', 'cp'):
                raise ValueError('unknown regeneration resource')
            initial = max(Fraction(1), ratio(item['base']) * ratio(item['level_mod']) * ratio(item['stat_bonus']))
            if item['functions']:
                raise ValueError('regen modifier oracle vector required')
            initial = Fraction(int(initial * 1_000_000), 1_000_000)
            credit = initial * ratio(item['character_multiplier']) * ratio(item['posture_multiplier'])
            rate_q = int(credit * 1_000_000)
            current, maximum, remainder = (integer(item[k]) for k in ('before', 'maximum', 'fraction_q'))
            if remainder >= 1_000_000 or current > maximum:
                raise ValueError('invalid resource baseline')
            total = remainder + rate_q
            after = min(maximum, current + total // 1_000_000)
            result[resource] = {'after': after, 'fraction_q': 0 if after == maximum else total % 1_000_000,
                                'rate_q': rate_q}
        if set(result) != {'hp', 'mp', 'cp'}:
            raise ValueError('all owner resources required')
        return result
    if kind == 'death_record_matches_fixture':
        # Existing loss table; L2PcInstance.java:4860; loss fraction comes from the recorded rules.
        loss = min(integer(data['pre_death_xp']), rounded(
            integer(data['level_span']) * ratio(data['loss_percent']) / 100))
        return {'incarnation': integer(data['incarnation'], 1),
                'pre_death_xp': data['pre_death_xp'], 'recoverable_death_xp_loss': loss}
    if kind == 'death_record_matches_before_disconnect':
        if not data['before'] or data['after_incarnation'] <= data['before']['incarnation']:
            raise ValueError('retained death/life transition required')
        return dict(data['before'], incarnation=data['after_incarnation'])
    if kind == 'restore_did_not_reapply_hooks':
        if not data['retained_modifiers']:
            raise ValueError('nonempty retained modifier proof required')
        return {'modifiers': data['retained_modifiers'], 'periodic_phase': data['retained_periodic_phase'],
                'one_time_hook_invocations': 0, 'resource_hook_credit_q': 0}
    if kind == 'skill_hate_outcome_coherent':
        # L2Attackable.java:571,606-621; L2Character.java:4603-4606 and L2Attackable.java:666 effectPoint.
        ledger = {str(uuid.UUID(key)): integer(value) for key, value in data['ledger_before'].items()}
        attacker = str(uuid.UUID(data['attacker']))
        point = integer(abs(data['effect_point']))
        if data['effect_point'] > 0 and attacker not in ledger:
            point = 0
        added = point + 1 + max(1, integer(data['rolled_damage']) * 100 // (integer(data['npc_level'], 1) + 7))
        ledger[attacker] = min(integer(data['hate_cap']), ledger.get(attacker, 0) + added)
        eligible = {str(uuid.UUID(key)) for key in data['eligible']}
        candidates = {key: value for key, value in ledger.items() if key in eligible}
        if not candidates:
            raise ValueError('eligible hate target proof required')
        most = max(candidates.values())
        tied = sorted(key for key, value in candidates.items() if value == most)
        selected = data['current_target'] if data['current_target'] in tied else tied[0]
        return {'ledger': ledger, 'most_hated': selected}
    if kind == 'resurrect_refund_matches_oracle':
        # Formulas.java:1372-1386 source cap quirk; L2PcInstance.java:4702 refund.
        base = ratio(data['base_restore_percent'])
        result = base
        if base not in (0, 100):
            result *= ratio(data['wit_bonus'])
            if result - base > 20:
                result += 20
            result = min(Fraction(90), max(base, result))
        refund = rounded(integer(data['recoverable_loss']) * result / 100)
        return {'xp_refund': refund, 'xp_after': integer(data['xp_before']) + refund,
                'remaining_recoverable_loss': 0, 'refund_count': 1}
    raise ValueError(f'unsupported raw fact {kind}')


def evaluate(data, recording, entity, gates):
    digest = hashlib.sha256(recording.read_bytes()).hexdigest()
    if (set(data) != {'schema_version', 'recording_sha256', 'zone', 'epoch', 'facts'}
            or type(data['schema_version']) is not int or data['schema_version'] != 1 or data['recording_sha256'] != digest
            or not isinstance(data['facts'], list)):
        raise ValueError('invalid/unbound replay evidence envelope')
    integer(data['zone'], 1)
    integer(data['epoch'], 1)
    seen, rows = set(), []
    for row in data['facts']:
        if set(row) != {'kind', 'entity', 'tick', 'skill_id', 'phase', 'inputs', 'actual'} or row['kind'] not in FACTS:
            raise ValueError('unknown/missing raw evidence fields; producer PASS flags forbidden')
        row_entity = str(uuid.UUID(row['entity']))
        key = (row['kind'], row_entity, integer(row['tick']), integer(row['skill_id']), row['phase'])
        if key in seen:
            raise ValueError('duplicate entity/tick evidence fact')
        seen.add(key)
        if row_entity == entity:
            rows.append(row)
    results = []
    for gate in gates:
        kind, args = gate['kind'], gate['args']
        matched = [row for row in rows if row['kind'] == kind]
        if kind in ('skill_damage_matches_oracle', 'skill_effect_matches_oracle'):
            matched = [row for row in matched if row['skill_id'] == int(args[0])]
        if kind == 'stat_matches_oracle':
            matched = [row for row in matched if row['phase'] == args[0]]
        errors = []
        if kind in ('regen_pulses', 'moving_regen_pulses'):
            if len(matched) < int(args[1]):
                errors.append(f'expected {args[1]} pulses, received {len(matched)}')
            for row in matched:
                posture = 'moving' if kind.startswith('moving_') else 'standing'
                if row['inputs'].get('posture') != posture or json.dumps(row['actual'], sort_keys=True) != json.dumps({'pulse_count': 1}, sort_keys=True):
                    errors.append(f"tick {row['tick']}: invalid pulse fact")
        else:
            if not matched:
                errors.append('missing entity/tick raw replay fact')
            for row in matched:
                try:
                    if json.dumps(row['actual'], sort_keys=True) != json.dumps(expected(row), sort_keys=True):
                        errors.append(f"tick {row['tick']}: source expectation mismatch")
                except (KeyError, ValueError, TypeError, IndexError, ZeroDivisionError) as error:
                    errors.append(f"tick {row['tick']}: incomplete oracle inputs: {error}")
        results.append(gate | {'ticks': [row['tick'] for row in matched], 'errors': errors})
    return {'schema_version': 1, 'recording_sha256': digest, 'entity': entity, 'gates': results,
            'passed': bool(results) and not any(row['errors'] for row in results)}


def run(scenario, recording, report, out, replay):
    gates = requirements(scenario)
    if not gates:
        return 0
    root = ET.parse(report).getroot()
    identities = root.findall(".//property[@name='own_entity_id']")
    if len(identities) != 1:
        raise ValueError('exactly one current report owner required')
    entity = str(uuid.UUID(identities[0].get('value')))
    raw = out.with_suffix('.raw.json')
    if raw.exists() or out.exists():
        raise ValueError('replay evidence destinations must be fresh')
    # Coverage/trace and replay comparison all use the same unit recording, never a
    # manually supplied green sidecar. Producer arguments are the cross-owner C11 hook.
    command = shlex.split(replay)
    log = out.with_suffix('.log')
    with log.open('w') as stream:
        for args in (['check', '--file', str(recording), '--session', entity],
                     ['trace', '--file', str(recording), '--session', entity,
                      '--out', str(out.with_suffix('.trace.html')), '--evidence-out', str(raw)]):
            subprocess.run(command + args, cwd=REPO, stdout=stream, stderr=subprocess.STDOUT, check=True)
    data = json.loads(raw.read_text(), object_pairs_hook=unique_object)
    result = evaluate(data, recording, entity, gates)
    out.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    return int(not result['passed'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--list', type=Path, help='validate declarations without execution')
    parser.add_argument('--count', type=Path)
    parser.add_argument('--scenario', type=Path)
    parser.add_argument('--recording', type=Path)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--out', type=Path)
    parser.add_argument('--replay', default='cargo run --quiet --bin nightfall-replay --')
    args = parser.parse_args()
    try:
        if args.list or args.count:
            gates = requirements(args.list or args.count)
            if args.count:
                print(len(gates))
            return 0
        if not all((args.scenario, args.recording, args.report, args.out)):
            raise ValueError('scenario/recording/report/out required')
        return run(args.scenario, args.recording, args.report, args.out, args.replay)
    except (OSError, ValueError, KeyError, TypeError, ET.ParseError, subprocess.SubprocessError) as error:
        print(f'Phase3 replay evidence gate failed: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
