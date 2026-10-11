#!/usr/bin/env python3
"""Fail-closed C11 gates over verified C14 export JSON Lines.

Every sim unit exports its own recording; raw facts never carry acceptance flags.
Absent transitions and unsupported independent oracle inputs fail closed.
"""
import argparse
from fractions import Fraction
import hashlib
import importlib.util
import json
from pathlib import Path
import shlex
import subprocess
import sys
import uuid
import xml.etree.ElementTree as ET

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parents[2]
ORACLE_SPEC = importlib.util.spec_from_file_location(
    'phase3_skill_oracle', REPO / 'packages/data/scripts/skill_oracle.py')
oracle = importlib.util.module_from_spec(ORACLE_SPEC)
ORACLE_SPEC.loader.exec_module(oracle)
FACTS = frozenset(('late_entry_hp_continuity', 'regen_pulses', 'moving_regen_pulses', 'regen_matches_oracle',
    'moving_regen_matches_oracle', 'death_record_matches_fixture',
    'death_record_matches_before_disconnect', 'restore_did_not_reapply_hooks',
    'skill_hate_outcome_coherent', 'skill_damage_matches_oracle',
    'skill_effect_matches_oracle', 'stat_matches_oracle', 'resurrect_refund_matches_oracle', 'resurrection_vitals_match_fixture', 'karma_decay_matches_oracle'))
RAW_FIELDS = {
    'observer_combat': 'events',
    'karma_decay': 'target xp_before xp_after pre_award_level karma_before karma_after',
    'regen_pulse': 'actual amounts before cause fractions_before functions inputs max_hp max_mp rules',
    'death_record': 'after incarnation inside_peace killer level loss penalty_before penalty_roll pre_death_xp victim_classification',
    'modifier_reconstruction': 'after before functions ordered_sources word_cursor_after',
    'admission_reconstruction': 'after load',
    'resource_hook': 'after amount before fraction_after fraction_before handler rank resource skill_id whole_credit',
    'hate_selection': 'current_target eligible ledger most_hated',
    'formula_resolution': 'after continuous inputs outcomes rank rng_draws skill_id target',
    'effect_formula': 'chance_evaluation duration_inputs duration_seconds landed rank rng_draws skill_id target',
    'auto_attack_resolution': 'events inputs rng_draws',
    'rng_draw': 'bound callsite target value word_after word_before',
    'resurrection_offer': 'death definition offer skill wit_bonus',
    'resurrection_refund': 'accepted after expected_xp_refund incarnation_after offer xp_after xp_before vitals_before vitals_after',
    'learning_commit': 'account request_key result',
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
    # L2PcInstance.java:4702, Java Math.round; no host floats.
    value += Fraction(1, 2)
    return value.numerator // value.denominator


def same(actual, expected):
    # JSON distinguishes true from 1, even though Python equality does not.
    if json.dumps(actual, sort_keys=True) != json.dumps(expected, sort_keys=True):
        raise ValueError('source expectation mismatch')


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate evidence JSON key {key}')
        result[key] = value
    return result


def no_float(value):
    raise ValueError(f'non-exact JSON number {value}')


def read_jsonl(path):
    return [json.loads(line, object_pairs_hook=unique_object,
                       parse_float=no_float, parse_constant=no_float)
            for line in path.read_text().splitlines()]


def binding(value):
    if set(value) != {'entity', 'incarnation'}:
        raise ValueError('incomplete life binding')
    same(str(uuid.UUID(value['entity'])), value['entity'])
    integer(value['incarnation'], 1)


def draw_key(data):
    return json.dumps(data, sort_keys=True)


def validate(rows, recording):
    if not isinstance(rows, list) or len(rows) < 2:
        raise ValueError('manifest and complete trailer required')
    manifest, complete = rows[0], rows[-1]
    if set(manifest) != set(('kind', 'schema_version', 'recording_sha256', 'zone', 'epoch',
                            'snapshot_tick', 'watermark', 'snapshot_meta', 'rules', 'classes',
                            'phase3_rules', 'scope')):
        raise ValueError('unknown/missing manifest fields')
    same(manifest['kind'], 'manifest')
    same(manifest['schema_version'], 1)
    same(manifest['scope'], 'whole_recording')
    same(manifest['recording_sha256'], hashlib.sha256(recording.read_bytes()).hexdigest())
    integer(manifest['zone'], 1)
    integer(manifest['epoch'], 1)
    start = integer(manifest['snapshot_tick'])
    if set(complete) != {'kind', 'facts', 'ticks', 'last_tick'} or complete['kind'] != 'complete':
        raise ValueError('complete trailer required')
    same(complete['facts'], len(rows)-2)
    ticks = integer(complete['ticks'])
    last = start + ticks - 1 if ticks else None
    same(complete['last_tick'], last)
    watermark = manifest['watermark']
    if set(watermark) != {'zone', 'epoch', 'records', 'last_tick', 'reason'}:
        raise ValueError('unknown/missing watermark fields')
    for key, expected in (('zone', manifest['zone']), ('epoch', manifest['epoch']),
                          ('records', ticks), ('last_tick', last)):
        same(watermark[key], expected)
    if not isinstance(watermark['reason'], str) or not watermark['reason']:
        raise ValueError('watermark reason required')
    previous_tick, draws, cursor = start, {}, 0
    for sequence, row in enumerate(rows[1:-1]):
        if set(row) != {'kind', 'entity', 'tick', 'sequence', 'data'}:
            raise ValueError('unknown/missing raw fact fields; producer PASS flags forbidden')
        same(row['sequence'], sequence)
        if str(uuid.UUID(row['entity'])) != row['entity']:
            raise ValueError('canonical entity UUID required')
        tick = integer(row['tick'])
        if tick < previous_tick or tick < start or tick >= start + ticks:
            raise ValueError('fact outside ordered recording tick scope')
        previous_tick = tick
        data, kind = row['data'], row['kind']
        if not isinstance(data, dict):
            raise ValueError('raw fact data object required')
        if kind == 'hate_delta':
            fields = ('amount direction ledger_after ledger_before reason requested source'
                      if data.get('direction') == 'add' else 'amount direction reason removed_row source')
        elif kind == 'damage_formula':
            fields = 'critical exact_damage formula power rank rolled_damage skill_id target '
            if data.get('formula') == 'calcMagicDam':
                fields += 'm_atk_q m_def_q magic_level_difference magic_success_rate resolution spread'
            elif data.get('formula') == 'calcSkillPhysDam':
                fields += 'p_atk_q p_def_q'
            else:
                raise ValueError('unknown damage formula')
        elif kind in RAW_FIELDS:
            fields = RAW_FIELDS[kind]
        else:
            raise ValueError(f'unknown raw category {kind}')
        if set(data) != set(fields.split()):
            raise ValueError(f'unknown/missing {kind} data fields; producer PASS flags forbidden')
        if 'target' in data:
            binding(data['target'])
        if 'skill_id' in data:
            integer(data['skill_id'], 1)
            integer(data['rank'], 1)
        if kind == 'formula_resolution':
            same(data['inputs']['caster']['entity']['id'], row['entity'])
            same(data['inputs']['target']['entity']['id'], data['target']['entity'])
            same(data['inputs']['target']['entity']['combat']['incarnation'], data['target']['incarnation'])
            same(data['inputs']['rank'], data['rank'])
            same(data['inputs']['skill']['id'], data['skill_id'])
        if kind in ('resurrection_offer', 'resurrection_refund'):
            binding(data['offer']['caster'])
            integer(data['offer']['target_incarnation'], 1)
            integer(data['offer']['id'], 1)
        if kind == 'hate_selection' and data['most_hated'] is not None:
            binding(data['most_hated'])
        if kind in ('modifier_reconstruction', 'admission_reconstruction'):
            same(data['after']['entity']['id'], row['entity'])
        if kind == 'rng_draw':
            bound = integer(data['bound'], 1)
            if integer(data['value']) >= bound:
                raise ValueError('RNG value outside exclusive bound')
            same(data['target']['entity'], row['entity'])
            for name in ('word_before', 'word_after'):
                if not isinstance(data[name], str) or not data[name].isdigit():
                    raise ValueError('decimal RNG word cursor required')
            before, after = int(data['word_before']), int(data['word_after'])
            if before < cursor or after <= before or not isinstance(data['callsite'], str):
                raise ValueError('RNG consumption out of order')
            cursor = after
            key = (tick, draw_key(data))
            if key in draws:
                raise ValueError('duplicate standalone RNG consumption')
            draws[key] = sequence
        if 'rng_draws' in data:
            prior, seen = -1, set()
            if not isinstance(data['rng_draws'], list):
                raise ValueError('ordered RNG draw list required')
            for draw in data['rng_draws']:
                index = draws.get((tick, draw_key(draw)))
                if index is None or index <= prior or index in seen:
                    raise ValueError('resolution draw missing, duplicated or reordered')
                seen.add(index)
                prior = index
    return manifest, rows[1:-1]


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


def fold(initial, functions, stat):
    modifiers = []
    for function in functions:
        if function['key']['stat'] != stat:
            continue
        if function['conditions']:
            raise ValueError('unsupported independent conditional modifier inputs')
        operation = function['operation']
        if len(operation) != 1:
            raise ValueError('one modifier operation required')
        kind, value = next(iter(operation.items()))
        modifiers.append((kind, ratio(value) / (1 if kind == 'Product' else 1_000_000)))
    return oracle.fold_stat(initial, modifiers)


def regen(row, manifest, facts):
    data, inputs = row['data'], row['data']['inputs']
    same(data['cause'], 'scheduled_regeneration')
    if type(inputs['player']) is not bool or inputs['posture'] not in ('Standing', 'Moving'):
        raise ValueError('recorded player/posture required')
    same(data['rules'], manifest['phase3_rules']['regen'])
    rules = data['rules']
    result = {'fractions': {}}
    amounts = {}
    for resource, stat in (('hp', 'regHp'), ('mp', 'regMp'), ('cp', 'regCp')):
        if resource == 'cp' and not inputs['player']:
            amounts['cp_q'], result['cp'], result['cp_granted'] = None, None, 0
            result['fractions']['cp'] = 0
            continue
        # Formulas.java:263-351,357-442,442-450; min-before-Funcs, then config/posture.
        initial = oracle.calc_regen_initial(
            ratio(inputs['base_' + resource]),
            ratio(inputs['level_mod']) if inputs['player'] else Fraction(1),
            ratio(inputs['men_bonus' if resource == 'mp' else 'con_bonus']) if inputs['player'] else Fraction(1),
            ratio(rules['rules']['min_initial_regen']))
        initial = Fraction(int(initial * 1_000_000), 1_000_000)
        credit = fold(initial, data['functions'], stat)
        credit = oracle.calc_regen_credit(
            credit, ratio(rules['character_multipliers'][resource + '_regen_multiplier']),
            ratio(rules['movement_multipliers'][inputs['posture'].lower()]) if inputs['player'] else Fraction(1))
        amount = int(credit * 1_000_000)
        amounts[resource + '_q'] = integer(amount)
        if resource == 'cp':
            before, maximum = (integer(data['before']['cp'][key]) for key in ('current', 'maximum'))
        else:
            before, maximum = integer(data['before'][resource]), integer(data['max_' + resource])
        fraction = integer(data['fractions_before'][resource])
        if before > maximum or fraction >= 1_000_000:
            raise ValueError('invalid regeneration baseline')
        total = fraction + amount
        after = min(maximum, before + total // 1_000_000)
        result[resource] = after
        result[resource + '_granted'] = after - before
        result['fractions'][resource] = 0 if after == maximum else total % 1_000_000
    same(data['amounts'], amounts)
    same(data['actual'], result)


def death(row, manifest, facts):
    data = row['data']
    level = integer(data['level'], 1)
    # Existing Nightfall XP table, progression.rs:42-51; not a new loss policy.
    rules = manifest['rules']
    span = integer(rules['xp_to_level'][level]) - integer(rules['xp_to_level'][level-1])
    loss = min(integer(data['pre_death_xp']), rounded(Fraction(span * integer(rules['death_loss'][level-1]), 1_000_000)))
    same(data['loss'], loss)
    same(data['after']['last_death'], {'incarnation': integer(data['incarnation'], 1),
         'pre_death_xp': data['pre_death_xp'], 'recoverable_death_xp_loss': loss})
    # Binding A15 / design3 §3: source eligible draw, cap and peace exclusions.
    eligible = (not data['inside_peace'] and data['penalty_before'] < 5
                and data['killer'] is not None
                and (data['killer']['kind'] == 'npc' or
                     (data['killer']['kind'] == 'player' and data['victim_classification'] == 'Chaotic')))
    penalty = integer(data['penalty_before'])
    if eligible:
        roll = integer(data['penalty_roll'])
        if roll >= 100:
            raise ValueError('penalty draw outside source bound')
        penalty += int(roll < 20)
    else:
        same(data['penalty_roll'], None)
    same(data['after']['death_penalty_level'], penalty)


def preceding_mutation(row, facts, death_record=False):
    # Admission itself is a reconstruction, never the mutation it must preserve.
    prior_admissions = [r for r in facts if r['entity'] == row['entity']
                        and r['kind'] == 'admission_reconstruction' and r['sequence'] < row['sequence']]
    start = prior_admissions[-1]['sequence'] if prior_admissions else -1
    before = [r for r in facts if start < r['sequence'] < row['sequence'] and r['tick'] < row['tick']]
    if death_record:
        candidates = [r for r in before if r['entity'] == row['entity'] and r['kind'] == 'death_record'
                      and r['data']['after'] is not None]
    else:
        applications = {effect_id for r in before if r['kind'] == 'formula_resolution'
                        for outcome in r['data']['outcomes'] if outcome['target']['entity'] == row['entity']
                        for effect_id in outcome['applied_effect_ids']}
        candidates = [r for r in before if r['entity'] == row['entity'] and r['kind'] == 'modifier_reconstruction'
                      and applications.intersection(map(int, r['data']['after']['phase3']['effects']['entries']))]
    return candidates[-1] if candidates else None


def admission_death(row, manifest, facts):
    data, after = row['data'], row['data']['after']
    checkpoint = data['load']['phase3']
    retained = checkpoint['last_death']
    if not retained:
        raise ValueError('retained death record required')
    incarnation = integer(after['entity']['combat']['incarnation'], 1)
    same(after['phase3']['player']['last_death'], retained | {'incarnation': incarnation})
    same(after['phase3']['player']['death_penalty_level'], checkpoint['death_penalty_level'])
    # A checkpoint supplied by admission alone cannot attest before-disconnect equality.
    before = preceding_mutation(row, facts, True)
    if before is None:
        raise ValueError('missing recorded death before disconnect')
    same(before['data']['after']['last_death'], retained)
    same(before['data']['after']['death_penalty_level'], checkpoint['death_penalty_level'])


def reconstruction(row, manifest, facts):
    data, after = row['data'], row['data']['after']
    load, sidecar = data['load'], after['phase3']
    checkpoint = load['phase3']
    # Reconstruction with passives alone does not attest retained live hooks/effects.
    retained = checkpoint['effect_remainders']
    if not retained:
        raise ValueError('nonempty retained timed effect required')
    before = preceding_mutation(row, facts)
    if before is None:
        raise ValueError('missing recorded effect before disconnect')
    previous = before['data']['after']
    original = sorted(previous['phase3']['effects']['entries'].values(), key=lambda e: (e['applied_at'], e['id']))
    for retained_effect in retained:
        matches = [e for e in original if all(e[key] == retained_effect[key]
                   for key in ('skill', 'source', 'pool', 'abnormal_type', 'abnormal_level', 'flags'))]
        if len(matches) != 1:
            raise ValueError('retained effect not bound to unique preceding mutation')
        old = matches[0]
        if retained_effect['remaining_ticks'] > old['expires_at'] - before['tick']:
            raise ValueError('retained duration increased before disconnect')
        same([h['xml_ordinal'] for h in old['handlers']], [h['xml_ordinal'] for h in retained_effect['handlers']])
    live = sorted(sidecar['effects']['entries'].values(), key=lambda e: (e['applied_at'], e['id']))
    same(len(live), len(retained))
    for old, new in zip(retained, live):
        for key in ('skill', 'source', 'pool', 'abnormal_type', 'abnormal_level', 'flags'):
            same(new[key], old[key])
        same(new['target_incarnation'], after['entity']['combat']['incarnation'])
        same(new['expires_at'], row['tick'] + integer(old['remaining_ticks'], 101))
        same(len(new['handlers']), len(old['handlers']))
        for old_handler, handler in zip(old['handlers'], new['handlers']):
            same(handler['xml_ordinal'], old_handler['xml_ordinal'])
            periodic = old_handler['periodic']
            if periodic is None:
                same(handler['periodic'], None)
                continue
            actual = handler['periodic']
            for key in ('resource', 'power'):
                same(actual[key], periodic[key])
            same(actual['schedule'], {'anchor': row['tick'], 'interval_ms': periodic['interval_ms'],
                 'pulse_index': periodic['pulse_index'], 'next_ideal_offset_ms': periodic['next_in_ms']})
    ordered = [v for _, v in sorted(sidecar['modifier_install_order'].items(), key=lambda pair: int(pair[0]))]
    expected = []
    for source in checkpoint['modifier_install_order']:
        if source['kind'] == 'passive':
            expected.append({'kind': 'passive', 'value': source['skill']})
        elif source['kind'] == 'effect':
            expected.append({'kind': 'effect', 'value': live[integer(source['effect_ordinal'])]['id']})
        elif source['kind'] == 'death_penalty':
            expected.append({'kind': 'death_penalty'})
        else:
            raise ValueError('unknown retained source kind')
    same(ordered, expected)
    hooks = [r for r in facts if r['entity'] == row['entity'] and r['tick'] == row['tick'] and r['kind'] == 'resource_hook']
    if hooks:
        raise ValueError('resource hook executed on reconstruction tick')
    for resource in ('hp', 'mp'):
        if load[resource] is None:
            raise ValueError('explicit persisted resource baseline required')
        maximum = after['effective']['physical']['max_' + resource]
        same(after['entity']['combat'][resource], min(integer(load[resource]), integer(maximum)))
    for rebuilt in [r for r in facts if r['entity'] == row['entity'] and r['tick'] == row['tick']
                    and r['kind'] == 'modifier_reconstruction']:
        same(rebuilt['data']['before']['word_cursor'], rebuilt['data']['word_cursor_after'])


def refund(row, manifest, facts):
    data, offer = row['data'], row['data']['offer']
    candidates = [r for r in facts if r['kind'] == 'resurrection_offer' and r['entity'] == row['entity']
                  and r['sequence'] < row['sequence'] and r['data']['offer']['id'] == offer['id']]
    if len(candidates) != 1:
        raise ValueError('unique preceding offer required')
    original = candidates[0]['data']
    same(offer, original['offer'])
    definition = original['definition']
    same(definition, manifest['phase3_rules']['skills']['skills'][str(original['skill']['id'])])
    resolutions = [r for r in facts if r['kind'] == 'formula_resolution'
                   and r['entity'] == offer['caster']['entity'] and r['tick'] == candidates[0]['tick']
                   and r['data']['target']['entity'] == row['entity']
                   and r['data']['skill_id'] == original['skill']['id']
                   and r['data']['rank'] == original['skill']['rank']]
    if len(resolutions) != 1:
        raise ValueError('unique source resurrection resolution required')
    context = resolutions[0]['data']['inputs']['caster']
    current_class = context['entity']['combat']['role']['progression']['class_state']['current_class_id']
    wit = manifest['phase3_rules']['profiles']['classes'][str(current_class)]['base_stats']['wit']
    if any(f['key']['stat'] == 'WIT' for f in context['effective']['calculator']):
        raise ValueError('independent modified WIT vector required')
    # StatRules::bonus (stat_rules.rs:121-135) indexes the table by raw stat, including zero.
    same(str(ratio(original['wit_bonus'])), str(Fraction(manifest['rules']['bonus']['wit'][wit], 1_000_000)))
    powers = [effect['params']['power'] for effect in definition['effects'] if effect['name'] == 'Resurrection']
    if len(powers) != 1:
        raise ValueError('one source resurrection handler required')
    base = ratio(powers[0][integer(original['skill']['rank'], 1)-1])
    percent = base
    # Formulas.java:1372-1386 preserves the upstream >20 branch quirk.
    if base not in (0, 100):
        percent *= ratio(original['wit_bonus'])
        if percent-base > 20:
            percent += 20
        percent = min(Fraction(90), max(base, percent))
    percent_q = rounded(percent * 1_000_000)
    same(offer['restore_percent_q'], percent_q)
    same(offer['recoverable_xp'], original['death']['recoverable_death_xp_loss'])
    if type(data['accepted']) is not bool:
        raise ValueError('acceptance boolean required')
    credit = (min(offer['recoverable_xp'], rounded(Fraction(offer['recoverable_xp'] * percent_q, 100_000_000)))
              if data['accepted'] else 0)
    same(data['expected_xp_refund'], credit)  # diagnostic, never the expectation source
    same(data['xp_after'], integer(data['xp_before']) + credit)
    same(data['incarnation_after'], offer['target_incarnation'] + int(data['accepted']))
    same(data['after']['last_death'], None if data['accepted'] else original['death'])
    same(data['after']['resurrect_offer'], None)
    other = [r for r in facts if r['kind'] == 'resurrection_refund' and r['entity'] == row['entity']
             and r['data']['offer']['id'] == offer['id']]
    same(len(other), 1)


def revival_vitals(row, manifest, facts):
    refund(row, manifest, facts)
    data = row['data']
    same(data['accepted'], True)
    before, after = data['vitals_before'], data['vitals_after']
    same(before['id'], row['entity'])
    same(after['id'], row['entity'])
    same(before['combat']['incarnation'], data['offer']['target_incarnation'])
    same(after['combat']['incarnation'], data['incarnation_after'])
    old, new = before['combat'], after['combat']
    # doRevive runs at the corpse; MP/CP are retained. Actual maxima are replay inputs.
    expected = oracle.do_revive(new['sheet']['max_hp'], old['hp'], old['mp'],
                               old['role']['progression']['class_state']['cp'])
    same(new['hp'], expected['hp'])
    same(new['mp'], expected['mp'])
    same(new['role']['progression']['class_state']['cp'], expected['cp'])
    same(after['pos'], before['pos'])
    same(new['protected_until'], None)


def hate(row, manifest, facts):
    data = row['data']
    if data['direction'] != 'add':
        raise ValueError('skill addition ledger required')
    uuid.UUID(data['source'])
    before, requested = data['ledger_before']['entries'], data['requested']
    constants = manifest['rules']['constants']
    ledger = {key: dict(value) for key, value in before.items()}
    previous = ledger.get(data['source'], {'hate': 0, 'damage': 0})
    ledger[data['source']] = {
        'hate': min(integer(constants['hate_cap']), integer(previous['hate']) + integer(requested['hate'])),
        'damage': min(2**64-1, integer(previous['damage']) + integer(requested['damage']))}
    same(data['ledger_after'], {'entries': ledger})
    same(data['amount'], ledger[data['source']]['hate'] - previous['hate'])
    resolutions = [r for r in facts if r['kind'] == 'formula_resolution' and r['entity'] == data['source']
                   and r['tick'] == row['tick'] and r['data']['target']['entity'] == row['entity']]
    if len(resolutions) != 1:
        raise ValueError('unique same-tick skill resolution required for NPC ledger')
    resolution = resolutions[0]['data']
    if (resolution['inputs']['caster']['phase3']['player'] is None
            or resolution['inputs']['target']['phase3']['player'] is not None):
        raise ValueError('playable source and NPC target required for skill hate')
    point = resolution['inputs']['skill']['effect_point'][resolution['rank']-1]
    same(resolution['inputs']['skill'], manifest['phase3_rules']['skills']['skills'][str(resolution['skill_id'])])
    same(resolution['inputs']['caster']['entity']['id'], data['source'])
    same(resolution['inputs']['target']['entity']['id'], row['entity'])
    outcomes = resolution['outcomes']
    if data['reason'] == 'effect_point':
        same(requested, {'hate': abs(point), 'damage': 0})
        if point > 0 and data['source'] not in before:
            raise ValueError('positive effectPoint requires preexisting ledger row')
    elif data['reason'] == 'attacked':
        same(requested, {'hate': 1, 'damage': 0})
    elif data['reason'] == 'damage':
        if len(outcomes) != 1:
            raise ValueError('independent multi-handler hate vector required')
        # MagicalAttack.java:86-90 passes source damage, including overkill;
        # L2Attackable.java:556-606 and state_combat.rs:527-557 retain rolled hate.
        removed = integer(outcomes[0]['rolled'])
        level = integer(resolution['inputs']['target']['effective']['physical']['level'], 1)
        same(requested, {'hate': max(1, removed * constants['hate_numerator'] // (level + constants['hate_level_offset'])),
                         'damage': removed})
    else:
        raise ValueError('independent skill hate caller vector required for ' + data['reason'])
    selections = [r for r in facts if r['kind'] == 'hate_selection' and r['entity'] == row['entity']
                  and r['tick'] == row['tick'] and r['sequence'] > row['sequence']
                  and r['data']['ledger'] == data['ledger_after']]
    if not selections:
        # A lethal damage mutation can immediately clear a ledger before AI selection.
        removed = [r for r in facts if r['kind'] == 'hate_delta' and r['entity'] == row['entity']
                   and r['tick'] == row['tick'] and r['sequence'] > row['sequence']
                   and r['data']['direction'] == 'remove' and r['data']['reason'] == 'npc_died'
                   and r['data']['source'] == data['source']]
        if not removed:
            raise ValueError('missing immediate selection or death removal')
        same(removed[0]['data']['removed_row'], ledger[data['source']])
        return
    selected = selections[0]['data']
    eligible = {str(uuid.UUID(key)) for key in selected['eligible']}
    candidates = {key: value['hate'] for key, value in ledger.items() if key in eligible and value['hate'] > 0}
    if not candidates:
        same(selected['most_hated'], None)
    else:
        most = max(candidates.values())
        tied = sorted(key for key, value in candidates.items() if value == most)
        chosen = selected['current_target'] if selected['current_target'] in tied else tied[0]
        binding(selected['most_hated'])
        same(selected['most_hated']['entity'], chosen)


def one_draw(draws, callsite, bound):
    selected = [d for d in draws if d['callsite'] == callsite]
    if len(selected) != 1:
        raise ValueError('unique ' + callsite + ' draw required')
    same(selected[0]['bound'], bound)
    value = integer(selected[0]['value'])
    if value >= bound:
        raise ValueError('draw outside exclusive bound')
    return value


def exact_root(actual, offset, coefficient, radicand):
    # Compare exact algebra, including perfect-square roots and alternate scaling.
    if isinstance(actual, dict):
        same(sorted(actual), ['coefficient', 'offset', 'radicand'])
        same(str(ratio(actual['offset'])), str(offset))
        if ratio(actual['radicand']) < 0:
            raise ValueError('negative exact root radicand')
        other = ratio(actual['coefficient'])
        if (other < 0) != (coefficient < 0):
            raise ValueError('root coefficient sign mismatch')
        same(str(other**2 * ratio(actual['radicand'])), str(coefficient**2 * radicand))
    else:
        difference = ratio(actual) - offset
        if (difference < 0) != (coefficient < 0):
            raise ValueError('root coefficient sign mismatch')
        same(str(difference**2), str(coefficient**2 * radicand))


def resolution_for(row, facts):
    data = row['data']
    resolutions = [r for r in facts if r['kind'] == 'formula_resolution' and r['entity'] == row['entity']
                   and r['tick'] == row['tick'] and r['data']['skill_id'] == data['skill_id']
                   and r['data']['rank'] == data['rank'] and r['data']['target'] == data['target']
                   and r['sequence'] > row['sequence']]
    if len(resolutions) != 1:
        raise ValueError('unique enclosing skill/caster/target/rank/tick resolution required')
    resolution = resolutions[0]['data']
    inputs = resolution['inputs']
    same(inputs['rank'], data['rank'])
    same(inputs['skill']['id'], data['skill_id'])
    for context, entity in ((inputs['caster'], row['entity']), (inputs['target'], data['target']['entity'])):
        same(context['entity']['id'], entity)
    same(inputs['target']['entity']['combat']['incarnation'], data['target']['incarnation'])
    same(resolution['after']['entity']['id'], data['target']['entity'])
    return resolution


def damage(row, manifest, facts):
    data = row['data']
    resolution = resolution_for(row, facts)
    inputs = resolution['inputs']
    skill = inputs['skill']
    same(skill, manifest['phase3_rules']['skills']['skills'][str(data['skill_id'])])
    caster, target = inputs['caster']['effective'], inputs['target']['effective']
    supported = {'pAtk', 'pDef', 'mAtk', 'mDef', 'mAtkSpd', 'rEvas', 'runSpd',
                 'regHp', 'regMp', 'regCp', 'magicSuccRes'}
    changed = {f['key']['stat'] for c in (caster, target) for f in c['calculator']
               if f['key']['install_sequence'] != 0}
    if changed - supported:
        raise ValueError('unsupported independent damage modifiers')
    outcomes = [o for o in resolution['outcomes'] if o['target'] == data['target'] and o['resolution_kind'] in ('magical', 'physical')]
    if len(outcomes) != 1:
        raise ValueError('independent multi-handler damage vector required')
    outcome = outcomes[0]
    same(outcome['rolled'], data['rolled_damage'])
    # Formulas.java:867-898: use the recipient's recorded branch draws and spread.
    draws = resolution['rng_draws']
    spread = [d for d in draws if d['callsite'] == 'Spread']
    if len(spread) != 1:
        raise ValueError('unique damage spread draw required')
    radius = integer(caster['physical']['random_damage'])
    same(spread[0]['bound'], radius * 2 + 1)
    multiplier = Fraction(100 + spread[0]['value'] - radius, 100)
    handlers = [h for h in skill['effects'] if h['name'] in ('MagicalAttack', 'MagicalAttackRange', 'PhysicalAttack')]
    if len(handlers) != 1:
        raise ValueError('independent single damage handler required')
    powers = handlers[0]['params']['power']
    power = powers[data['rank']-1] if isinstance(powers, list) else powers
    same(str(ratio(data['power'])), str(ratio(power)))
    # Unsupported high-level NPC penalties are refused rather than treated as neutral.
    target_level = target['physical']['level']
    caster_level = caster['physical']['level']
    minimum = ratio(manifest['phase3_rules']['npc']['penalty']['config']['MinNPCLevelForDmgPenalty']['values'][0])
    if inputs['caster']['phase3']['player'] is not None and inputs['target']['phase3']['player'] is None and target_level >= minimum and target_level - caster_level >= 2:
        raise ValueError('unsupported independent high-level NPC penalty inputs')
    if data['formula'] == 'calcMagicDam':
        same(data['m_atk_q'], caster['m_atk_q'])
        same(data['m_def_q'], target['m_def_q'])
        same(str(ratio(data['spread'])), str(multiplier))
        magic_level = skill['magic_level'][data['rank']-1]
        difference = target_level - (magic_level if magic_level > 0 else caster_level)
        same(data['magic_level_difference'], difference)
        resistance = fold(Fraction(1), target['calculator'], 'magicSuccRes')
        rate = oracle.calc_magic_success(difference, resistance)
        same(data['magic_success_rate'], rate)
        critical = one_draw(draws, 'MagicCritical', 1000) < caster['magic_crit']
        same(data['critical'], critical)
        success = one_draw(draws, 'MagicSuccess', 100) < rate
        failure_draws = [d for d in draws if d['callsite'] == 'MagicFailure']
        player = inputs['caster']['phase3']['player'] is not None
        if not success and player:
            second = one_draw(draws, 'MagicFailure', 100) < rate
        else:
            if failure_draws:
                raise ValueError('unexpected second magic success draw')
            second = False
        branch = 'Success' if success else ('Half' if player and second and target_level - caster_level <= 9 else 'Resist')
        same(data['resolution'], branch)
        is_pvp = player and inputs['target']['phase3']['player'] is not None
        lo, hi = oracle.calc_magic_damage_bounds(
            ratio(data['power']), Fraction(data['m_atk_q'], 1_000_000),
            Fraction(data['m_def_q'], 1_000_000), is_pvp, critical, not success,
            branch == 'Half', target_level - caster_level, spread=multiplier)
        if branch == 'Resist':
            offset, coefficient, radicand = multiplier, Fraction(0), Fraction(0)
        else:
            coefficient = Fraction(91) * ratio(data['power']) / Fraction(data['m_def_q'], 1_000_000) * multiplier
            coefficient *= Fraction(1, 2) if branch == 'Half' else (Fraction(5, 2) if critical and is_pvp else 3 if critical else 1)
            offset, radicand = Fraction(0), Fraction(data['m_atk_q'], 1_000_000)
        exact_root(data['exact_damage'], offset, coefficient, radicand)
        if oracle.floor(lo) != oracle.floor(hi):
            raise ValueError('ambiguous independent damage rounding')
        expected = oracle.floor(lo)
    else:
        same(data['p_atk_q'], caster['physical']['p_atk'])
        same(data['p_def_q'], target['physical']['p_def'])
        if any(t['attack_providers'] or t['defence_providers'] or t['immunity_providers']
               for c in (caster, target) for t in c['traits'].values()):
            raise ValueError('unsupported independent physical trait inputs')
        expected_exact = oracle.calc_skill_phys_damage(
            ratio(data['power']), Fraction(data['p_atk_q'], 1_000_000),
            Fraction(data['p_def_q'], 1_000_000), crit=data['critical'], spread=multiplier)
        same(str(ratio(data['exact_damage'])), str(expected_exact))
        expected = oracle.floor(expected_exact)
    same(data['rolled_damage'], expected)


def effect(row, manifest, facts):
    data = row['data']
    resolution = resolution_for(row, facts)
    skill = resolution['inputs']['skill']
    same(skill, manifest['phase3_rules']['skills']['skills'][str(data['skill_id'])])
    inputs = resolution['inputs']
    caster, target = inputs['caster']['effective'], inputs['target']['effective']
    success = data['chance_evaluation']
    if success is None:
        if skill['activate_rate'] != -1 and skill['basic_property'] != 'NONE':
            raise ValueError('missing conditional effect chance')
        same(data['landed'], True)
        if any(d['callsite'] == 'EffectSuccess' for d in data['rng_draws']):
            raise ValueError('unconditional effect consumed success draw')
    else:
        src = success['inputs']
        same(src['magic_level'], skill['magic_level'][data['rank']-1])
        same(src['target_level'], target['physical']['level'])
        same(src['activate_rate'], skill['activate_rate'])
        same(src['level_bonus_rate'], skill['level_bonus_rate'])
        same(src['is_magic'], skill['is_magic'] == 1)
        for key in ('min_chance', 'max_chance'):
            same(src[key], skill[key])
        same(str(ratio(src['magic_attack'])), str(Fraction(caster['m_atk_q'], 1_000_000)))
        same(str(ratio(src['magic_defence'])), str(Fraction(target['m_def_q'], 1_000_000)))
        projected = {'magic_level': src['magic_level'], 'target_level': src['target_level'],
                     'activate_rate': src['activate_rate'], 'lvl_bonus_rate': src['level_bonus_rate'],
                     'target_base_stat': src['target_base_stat'], 'basic_property': skill['basic_property'],
                     'is_magic': src['is_magic'], 'm_atk': src['magic_attack'], 'm_def': src['magic_defence'],
                     'trait_mod': src['trait_modifier'], 'element_mod': src['element_modifier'],
                     'buff_debuff_mod': src['buff_debuff_modifier'], 'min_chance': src['min_chance'],
                     'max_chance': src['max_chance']}
        lo, hi = oracle.calc_effect_success_bounds(**projected)
        src_level = projected['target_level'] + 3 if projected['magic_level'] <= -1 else projected['magic_level']
        coefficient = ratio((src_level - projected['target_level'] + 3) * projected['lvl_bonus_rate']
                            + projected['activate_rate'] + 30 - projected['target_base_stat'])
        coefficient *= ratio(projected['trait_mod']) * ratio(projected['element_mod']) * ratio(projected['buff_debuff_mod'])
        radicand = Fraction(1)
        if projected['is_magic']:
            coefficient *= Fraction(11) / ratio(projected['m_def'])
            radicand = ratio(projected['m_atk'])
        if lo == hi:
            exact_root(success['chance'], lo, Fraction(0), Fraction(0))
        else:
            exact_root(success['chance'], Fraction(0), coefficient, radicand)
        roll = one_draw(data['rng_draws'], 'EffectSuccess', 100)
        if (lo > roll) != (hi > roll):
            raise ValueError('ambiguous independent effect chance comparison')
        same(data['landed'], lo > roll)
    src = data['duration_inputs']
    same(src['base_seconds'], skill['abnormal']['time_s'][data['rank']-1])
    same(src['is_debuff'], skill['abnormal']['is_debuff'])
    same(src['magic_level'], skill['magic_level'][data['rank']-1])
    same(src['caster_level'], caster['physical']['level'])
    same(src['target_level'], target['physical']['level'])
    same(src['level_bonus_rate'], skill['level_bonus_rate'])
    projected = {'abnormal_time': src['base_seconds'], 'is_debuff': src['is_debuff'],
                 'magic_level': src['magic_level'], 'attacker_level': src['caster_level'],
                 'target_level': src['target_level'], 'lvl_bonus_rate': src['level_bonus_rate'],
                 'stat_bonus': src['stat_bonus'], 'trait_mod': src['trait_modifier'],
                 'element_mod': src['element_modifier']}
    seconds = oracle.ceil(oracle.calc_effect_duration(**projected))
    same(data['duration_seconds'], seconds)
    same(resolution['continuous'], [data['landed'], data['duration_seconds']])


def stats(row, manifest, facts):
    after = row['data']['after']
    effective = after['effective']
    entity = after['entity']
    role = entity['combat']['role']
    current_class = role['progression']['class_state']['current_class_id']
    profile = manifest['phase3_rules']['profiles']['classes'][str(current_class)]
    base_stats, magical = profile['base_stats'], profile['magical']
    level = effective['physical']['level']
    contexts = [row['data']['after']]
    contexts.extend(r['data']['after'] for r in facts if r['kind'] == 'modifier_reconstruction'
                    and r['entity'] == row['entity'] and r['sequence'] < row['sequence']
                    and r['data']['after']['phase3']['effects']['entries'])
    changed = {f['key']['stat'] for c in contexts for f in c['effective']['calculator']
               if f['key']['install_sequence'] != 0}
    unsupported = changed - {'mAtk', 'mDef', 'mAtkSpd', 'rEvas', 'pAtk', 'pDef'}
    if unsupported:
        raise ValueError('unsupported independent affected stats: ' + ','.join(sorted(unsupported)))
    weapon = manifest['phase3_rules']['profiles']['starter_weapon']['functions']
    sets = [w for w in weapon if w['stat'] == 'mAtk' and w['operation'] == 'SET']
    if len(sets) != 1:
        raise ValueError('independent weapon MAtk baseline required')
    # Independent source formulas precede the retained Func fold; never trust supplied Product.
    for formula, stat, source, actual in (
        ('FuncMAtkMod', 'mAtk', {'init_val': sets[0]['value'], 'level': level, 'int_stat': base_stats['int']}, effective['m_atk_q']),
        ('FuncMDefMod', 'mDef', {'value': magical['base_m_def'], 'level': level, 'men_stat': base_stats['men']}, effective['m_def_q']),
        ('FuncMAtkSpeed', 'mAtkSpd', {'init_val': magical['base_m_atk_speed'], 'wit_stat': base_stats['wit'],
         'max_m_atk_speed': manifest['phase3_rules']['constants']['MaxMAtkSpeed']['value']}, effective['cast_speed'])):
        value = source.get('init_val', source.get('value'))
        stat_value = source.get('int_stat', source.get('men_stat', source.get('wit_stat')))
        exact = oracle.calc_stat_base(formula, ratio(value), level, stat_value,
                                      source.get('max_m_atk_speed'))
        initial = Fraction(oracle.rounded(exact * 1_000_000), 1_000_000)
        extras = [f for f in effective['calculator'] if f['key']['stat'] == stat and f['key']['install_sequence'] != 0]
        derived = fold(initial, extras, stat)
        expected = (int(min(derived, source['max_m_atk_speed'])) if formula == 'FuncMAtkSpeed'
                    else rounded(derived * 1_000_000))
        same(actual, expected)
    if any(f['key']['stat'] == 'rEvas' and f['key']['install_sequence'] != 0 for f in effective['calculator']):
        raise ValueError('independent modified evasion vector required')
    independent = oracle.calc_evasion_base(level, base_stats['dex'])
    same(effective['physical']['evasion'], independent * 1_000_000)
    # FuncPAtkMod.java:43-44; FuncPDefMod.java:45-70; weapon SET precedes builtin.
    atk_sets = [w for w in weapon if w['stat'] == 'pAtk' and w['operation'] == 'SET']
    if len(atk_sets) != 1:
        raise ValueError('independent weapon PAtk baseline required')
    physical_profile = next((p for p in manifest['rules']['classes'] if p['id'] == role['class']), None)
    if physical_profile is None:
        raise ValueError('independent physical class profile required')
    for formula, stat, value, actual in (
        ('FuncPAtk', 'pAtk', atk_sets[0]['value'], effective['physical']['p_atk']),
        ('FuncPDef', 'pDef', Fraction(physical_profile['p_def_unarmoured'], 1_000_000), effective['physical']['p_def'])):
        exact = oracle.calc_stat_base(formula, value if isinstance(value, Fraction) else ratio(value), level, base_stats['str'])
        initial = Fraction(oracle.rounded(exact * 1_000_000), 1_000_000)
        extras = [f for f in effective['calculator'] if f['key']['stat'] == stat and f['key']['install_sequence'] != 0]
        same(actual, oracle.rounded(fold(initial, extras, stat) * 1_000_000))


def karma_decay(row, manifest, facts):
    # abfde049 Formulas.java:1814-1821: actual credited XP, pre-award level,
    # binary64 at each Java division, then truncation. No finite-vector lookup.
    data = row['data']
    rules = manifest['phase3_rules']['pvp']['karma_lost']
    credited = integer(data['xp_after']) - integer(data['xp_before'])
    integer(credited, 1)
    level = integer(data['pre_award_level'], 1)
    lost = oracle.calculate_karma_lost(credited, ratio(rules['multipliers'][level-1]),
                                       ratio(rules['rate_karma_lost']))
    before = integer(data['karma_before'])
    same(integer(data['karma_after']), max(0, before-lost))
    deaths = [r for r in facts if r['kind'] == 'death_record' and r['tick'] == row['tick']
              and r['entity'] == data['target']['entity'] and r['data']['victim_classification'] is None
              and r['data']['incarnation'] == data['target']['incarnation']
              and r['data']['killer']['binding']['entity'] == row['entity']]
    if len(deaths) != 1:
        raise ValueError('karma credit must join one actual NPC death')


def late_entry_continuity(row, manifest, facts):
    """Exact first delivered hit after every wounded NPC identity baseline.

    C11: same-tick regen precedes impacts; only final vitals reach the client.
    Check each intervening pulse with the independent Fraction oracle, then join
    the actual pre-impact HP and delivered result. No tolerance or skipped mismatch.
    """
    matched = False
    for spawn in row['data']['events']:
        if spawn['type'] != 'entity_spawn':
            continue
        combat = spawn['combat']
        if spawn['kind'] != 'npc' or combat['dead'] or combat['hp'] >= combat['max_hp']:
            continue
        target, life, hp = spawn['entity'], combat['incarnation'], combat['hp']
        for later in facts:
            if later['tick'] <= row['tick']:
                continue  # identity contains FINAL state of its admission tick
            data = later['data']
            if later['kind'] == 'regen_pulse' and later['entity'] == target:
                if data['before']['hp'] != hp:
                    raise ValueError(f"regen continuity: tick {later['tick']} before {data['before']['hp']} expected {hp}")
                regen(later, manifest, facts)
                hp = data['actual']['hp']
            if later['kind'] == 'auto_attack_resolution':
                victim = data['inputs']['target']
                if victim is None:
                    continue
                victim = victim.get('entity', victim)
                if victim['id'] == target:
                    same(victim['combat']['incarnation'], life)
                    if victim['combat']['hp'] != hp:
                        raise ValueError(f"impact continuity: tick {later['tick']} before {victim['combat']['hp']} expected {hp}")
            if later['kind'] != 'observer_combat' or later['entity'] != row['entity']:
                continue
            for event in data['events']:
                if event['type'] == 'entity_despawn' and event['entity'] == target:
                    raise ValueError('wounded baseline left visibility before a continued hit')
                if event['type'] != 'attack_result' or event['target'] != target:
                    continue
                same(event['target_incarnation'], life)
                expected = max(0, hp - integer(event['damage']))
                if event['target_hp_after'] != expected:
                    raise ValueError(f"delivered hit: tick {later['tick']} HP {event['target_hp_after']} expected {expected}")
                # The delivered carrier must equal a real resolution in this tick.
                resolutions = [r for r in facts if r['kind'] == 'auto_attack_resolution'
                               and r['tick'] == later['tick']
                               and event in r['data']['events']]
                if len(resolutions) != 1:
                    raise ValueError('first delivered hit has no unique native resolution')
                matched = True
                break
            else:
                continue
            break
        else:
            raise ValueError('wounded baseline has no subsequent delivered hit')
    if not matched:
        raise ValueError('no wounded NPC baseline with a continued hit')


def evaluate(rows, recording, entity, gates):
    entity = str(uuid.UUID(entity))
    manifest, facts = validate(rows, recording)
    if manifest['snapshot_meta']['schema_version'] != 9 or manifest['snapshot_meta']['digest_version'] != 'BinaryV5' or not manifest['phase3_rules']:
        raise ValueError('C11 requires recorded V5 policy')
    owner = [r for r in facts if r['entity'] == entity]
    results = []
    for gate in gates:
        kind, args = gate['kind'], gate['args']
        check = None
        if kind == 'late_entry_hp_continuity':
            # Select the last wounded baseline per life. Earlier observation used only
            # to coordinate leaving AOI is superseded by the actual late-entry baseline.
            baselines = {}
            for row in owner:
                if row['kind'] != 'observer_combat':
                    continue
                for event in row['data']['events']:
                    if event['type'] == 'entity_spawn':
                        combat = event['combat']
                        if event['kind'] == 'npc' and not combat['dead'] and combat['hp'] < combat['max_hp']:
                            baselines[(event['entity'], combat['incarnation'])] = row
            matched = list({r['sequence']: r for r in baselines.values()}.values())
            check = late_entry_continuity
        elif kind in ('regen_pulses', 'moving_regen_pulses', 'regen_matches_oracle', 'moving_regen_matches_oracle'):
            matched = [r for r in owner if r['kind'] == 'regen_pulse'
                       and (not kind.startswith('moving_') or r['data']['inputs']['posture'] == 'Moving')]
            check = regen
        elif kind == 'death_record_matches_fixture':
            matched = [r for r in owner if r['kind'] == 'death_record' and r['data']['after'] is not None]
            check = death
        elif kind in ('death_record_matches_before_disconnect', 'restore_did_not_reapply_hooks'):
            matched = [r for r in owner if r['kind'] == 'admission_reconstruction'
                       and r['data']['load'] is not None
                       and preceding_mutation(r, facts, kind.startswith('death_')) is not None]
            check = admission_death if kind.startswith('death_') else reconstruction
        elif kind in ('resurrect_refund_matches_oracle', 'resurrection_vitals_match_fixture'):
            matched = [r for r in owner if r['kind'] == 'resurrection_refund'
                       and (kind == 'resurrect_refund_matches_oracle' or r['data']['accepted'])]
            check = refund if kind == 'resurrect_refund_matches_oracle' else revival_vitals
        elif kind == 'karma_decay_matches_oracle':
            matched = [r for r in owner if r['kind'] == 'karma_decay']
            check = karma_decay
        elif kind == 'skill_hate_outcome_coherent':
            matched = [r for r in facts if r['kind'] == 'hate_delta' and r['data']['source'] == entity and r['data']['direction'] == 'add']
            check = hate
        elif kind in ('skill_damage_matches_oracle', 'skill_effect_matches_oracle'):
            raw = 'damage_formula' if kind.startswith('skill_damage') else 'effect_formula'
            matched = [r for r in owner if r['kind'] == raw and r['data']['skill_id'] == int(args[0])]
            check = damage if raw == 'damage_formula' else effect
        elif kind == 'stat_matches_oracle':
            matched = []
            previous_effects = set()
            for row in owner:
                if row['kind'] != 'modifier_reconstruction':
                    continue
                active = set(row['data']['after']['phase3']['effects']['entries'])
                admission_tick = any(r['kind'] == 'admission_reconstruction' and r['tick'] == row['tick'] for r in owner)
                if not admission_tick and ((args[0] == 'active' and active) or (args[0] == 'removed' and previous_effects - active)):
                    matched.append(row)
                previous_effects = active
            check = stats
        else:
            raise ValueError('unknown gate')
        errors = []
        if not matched:
            errors.append('missing entity/tick raw replay fact')
        if kind.endswith('pulses') and len({r['tick'] for r in matched}) < int(args[1]):
            errors.append(f"expected {args[1]} pulses, received {len(matched)}")
        seen_ticks = set()
        for row in matched:
            try:
                if row['kind'] == 'regen_pulse':
                    if row['tick'] in seen_ticks:
                        raise ValueError('duplicate scheduled resource pulse')
                    seen_ticks.add(row['tick'])
                check(row, manifest, facts)
            except (KeyError, ValueError, TypeError, IndexError, ZeroDivisionError) as error:
                errors.append(f"tick {row['tick']} sequence {row['sequence']}: {error}")
        # The accepted branch must occur to attest one-time refunds, even if declines verify.
        if kind == 'resurrect_refund_matches_oracle' and not any(r['data']['accepted'] is True for r in matched):
            errors.append('missing accepted resurrection refund')
        if kind == 'karma_decay_matches_oracle' and not any(r['data']['karma_before'] > r['data']['karma_after'] for r in matched):
            errors.append('missing positive NPC-XP karma decay')
        results.append(gate | {'ticks': [r['tick'] for r in matched],
                              'sequences': [r['sequence'] for r in matched],
                              'raw_kinds': sorted({r['kind'] for r in matched}), 'errors': errors})
    return {'schema_version': 1, 'recording_sha256': manifest['recording_sha256'],
            'zone': manifest['zone'], 'epoch': manifest['epoch'], 'entity': entity, 'gates': results,
            'passed': bool(results) and not any(r['errors'] for r in results)}


def run(scenario, recording, report, out, replay):
    gates = requirements(scenario)
    if not gates:
        return 0
    identities = ET.parse(report).getroot().findall(".//property[@name='own_entity_id']")
    if len(identities) != 1:
        raise ValueError('exactly one current report owner required')
    entity = str(uuid.UUID(identities[0].get('value')))
    raw = out.with_suffix('.raw.jsonl')
    if raw.exists() or out.exists():
        raise ValueError('replay evidence destinations must be fresh')
    command = shlex.split(replay)
    with out.with_suffix('.log').open('w') as stream:
        for args in (['check', '--file', str(recording), '--session', entity],
                     ['export', '--file', str(recording), '--session', entity, '--evidence-out', str(raw)]):
            subprocess.run(command + args, cwd=REPO, stdout=stream, stderr=subprocess.STDOUT, check=True)
    result = evaluate(read_jsonl(raw), recording, entity, gates)
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
    except (OSError, ValueError, KeyError, TypeError, IndexError, ET.ParseError, subprocess.SubprocessError) as error:
        print(f'Phase3 replay evidence gate failed: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
