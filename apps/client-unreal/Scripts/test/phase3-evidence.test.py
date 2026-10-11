#!/usr/bin/env python3
"""Native C14 consumer checks plus corruption regressions; no live-client claims."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
spec = importlib.util.spec_from_file_location('evidence', SCRIPTS / 'phase3-evidence.py')
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)
FIRST = '00000000-0000-0000-0000-000000000001'
SECOND = '00000000-0000-0000-0000-000000000002'
RECORDING = REPO / 'apps/api/fixtures/sessions/phase3-gameplay-v5.nfr'


class ReplayEvidence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # A missing/unbuildable producer is a failure, never a skipped native test.
        subprocess.run(['cargo', 'build', '--offline', '-p', 'nightfall-api', '--bin', 'nightfall-replay'],
                       cwd=REPO, check=True, capture_output=True, text=True)
        metadata = subprocess.run(['cargo', 'metadata', '--offline', '--no-deps', '--format-version', '1'],
                                  cwd=REPO, check=True, capture_output=True, text=True)
        cls.binary = Path(json.loads(metadata.stdout)['target_directory']) / 'debug/nightfall-replay'
        cls.native_tmp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.native_tmp.cleanup)
        cls.raw = Path(cls.native_tmp.name) / 'native.jsonl'
        cls.export(cls.raw)
        cls.native = evidence.read_jsonl(cls.raw)
        cls.casts_recording = REPO / 'apps/api/fixtures/sessions/phase3-casts-v5.nfr'
        casts_out = Path(cls.native_tmp.name) / 'casts.jsonl'
        subprocess.run([str(cls.binary), 'export', '--file', str(cls.casts_recording),
                        '--evidence-out', str(casts_out)], cwd=REPO, check=True, capture_output=True)
        cls.casts = evidence.read_jsonl(casts_out)
        cls.generated = Path(cls.native_tmp.name) / 'scenario-inputs'
        subprocess.run(['cargo', 'test', '--offline', '-p', 'nightfall-api', '--test',
                        'phase3_fixture_overlays', 'actual_scenario_inputs_record_for_independent_evidence',
                        '--', '--exact'], cwd=REPO, check=True, capture_output=True, text=True,
                       env=dict(os.environ, NIGHTFALL_HARNESS_EVIDENCE_DIR=str(cls.generated)))

        subprocess.run(['cargo', 'test', '--offline', '-p', 'nightfall-api', '--test',
                        'phase3_fixture_overlays', 'late_entry_handshake_preserves_wounded_baseline_and_first_hit',
                        '--', '--exact'], cwd=REPO, check=True, capture_output=True, text=True,
                       env=dict(os.environ, NIGHTFALL_HARNESS_EVIDENCE_DIR=str(cls.generated)))

    @classmethod
    def export(cls, out):
        subprocess.run([str(cls.binary), 'export', '--file', str(RECORDING), '--evidence-out', str(out)],
                       cwd=REPO, check=True, capture_output=True, text=True)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data = copy.deepcopy(self.native)

    def check(self, declaration, entity=FIRST, data=None, recording=RECORDING):
        path = self.root / 'scenario.nfs'
        path.write_text('# replay-gate: ' + declaration + '\n')
        return evidence.evaluate(self.data if data is None else data, recording, entity, evidence.requirements(path))

    def fact(self, kind, entity=None):
        return next(r for r in self.data[1:-1] if r['kind'] == kind and (entity is None or r['entity'] == entity))

    def test_exact_twelve_declarations_remain_outside_client_predicates(self):
        kinds = set()
        for path in (SCRIPTS.parent / 'Scenarios').rglob('*.nfs'):
            kinds.update(g['kind'] for g in evidence.requirements(path))
            for line in path.read_text().splitlines():
                if line.startswith(('nf.Expect ', 'nf.WaitFor ')):
                    self.assertNotIn(line.split()[1], evidence.FACTS, path.name)
        self.assertEqual(kinds, evidence.FACTS)

    def test_late_entry_exact_continuity_including_same_tick_regen(self):
        observer = '00000000-0000-0000-0000-0000000003e5'
        for epoch in (1, 2, 3):
            recording = self.generated / f'late-{epoch}.nfr'
            rows = evidence.read_jsonl(self.generated / f'late-{epoch}.jsonl')
            result = self.check('late_entry_hp_continuity keltir_durable', observer, rows, recording)
            baseline = next(r for r in rows if r.get('sequence') == result['gates'][0]['sequences'][0])
            # All three retain exact HP/life continuity. Epoch1 starts on entry;
            # only epochs2/3 prove receipt of a swing already in flight.
            evidence.late_entry_hp(baseline, rows[0], rows[1:-1], 'keltir_durable')
            self.assertEqual(result['passed'], epoch != 1)
            if epoch == 1:
                self.assertEqual(len(result['gates'][0]['errors']), 1)
                self.assertTrue(result['gates'][0]['errors'][0].endswith('swing did not start before re-entry'))
            if epoch == 3:
                pulses = {(r['entity'], r['tick']) for r in rows if r['kind'] == 'regen_pulse'}
                baselines = [r for r in rows if r['kind'] == 'observer_combat' and r['entity'] == observer
                             and any(e['type'] == 'entity_spawn' and e['kind'] == 'npc'
                                     and e['combat']['hp'] < e['combat']['max_hp'] for e in r['data']['events'])]
                baseline = baselines[-1]
                hit = next(e for r in rows if r['kind'] == 'observer_combat' and r['entity'] == observer
                           and r['tick'] > baseline['tick'] for e in r['data']['events']
                           if e['type'] == 'attack_result' and e['attacker'].endswith('0030'))
                self.assertIn((hit['target'], hit['tick']), pulses)
            for category in ('baseline', 'hit', 'regen', 'missing', 'no_departure', 'full_reentry'):
                altered = copy.deepcopy(rows)
                for row in altered[1:-1]:
                    if category == 'regen' and row['kind'] == 'regen_pulse':
                        row['data']['amounts']['hp_q'] += 1
                    if row['kind'] != 'observer_combat' or row['entity'] != observer:
                        continue
                    if category == 'no_departure':
                        row['data']['events'] = [e for e in row['data']['events'] if e['type'] != 'entity_despawn']
                    for event in row['data']['events']:
                        if (category == 'full_reentry' and event['type'] == 'entity_spawn'
                                and event['combat']['template'] == 'keltir_durable'
                                and row['tick'] > 10):
                            event['combat']['hp'] = event['combat']['max_hp']
                        if event['type'] == 'entity_spawn' and event['kind'] == 'npc':
                            if category == 'baseline' and event['combat']['hp']:
                                event['combat']['hp'] -= 1
                            if category == 'missing':
                                event['combat']['hp'] = event['combat']['max_hp']
                        if category == 'hit' and event['type'] == 'attack_result':
                            event['target_hp_after'] += 1
                corrupted = self.check('late_entry_hp_continuity keltir_durable', observer, altered, recording)
                self.assertFalse(corrupted['passed'], category)
                self.assertNotEqual(corrupted['gates'][0]['errors'], result['gates'][0]['errors'], category)

    def test_late_entry_requires_received_original_swing_after_all_identities(self):
        observer = '00000000-0000-0000-0000-0000000003e5'
        for epoch in (1, 2, 3):
            recording = self.generated / f'late-{epoch}.nfr'
            rows = evidence.read_jsonl(self.generated / f'late-{epoch}.jsonl')
            altered = copy.deepcopy(rows)
            for row in altered:
                if row['kind'] == 'observer_combat' and row['entity'] == observer:
                    row['data']['events'] = [e for e in row['data']['events'] if e['type'] != 'attack_started']
            result = self.check('late_entry_hp_continuity keltir_durable', observer, altered, recording)
            self.assertFalse(result['passed'])
            self.assertTrue(result['gates'][0]['errors'][0].endswith('missing or duplicate re-entry swing hydration'))
            if epoch == 1:
                continue
            positive = self.check('late_entry_hp_continuity keltir_durable', observer, rows, recording)
            self.assertTrue(positive['passed'])
            sequence = positive['gates'][0]['sequences'][0]
            for category in ('missing', 'duplicate', 'ordinary', 'baseline_tick', 'start',
                             'start_at_entry', 'impact', 'impact_at_entry', 'ready', 'attacker_life',
                             'target_life', 'hidden_target', 'unknown_target', 'other_npc',
                             'before_attacker_identity', 'before_last_identity', 'other_tick',
                             'other_observer', 'initial_entry_only', 'identity_target_life', 'native_swing'):
                with self.subTest(epoch=epoch, category=category):
                    altered = copy.deepcopy(rows)
                    row = next(r for r in altered if r.get('sequence') == sequence)
                    events = row['data']['events']
                    event = next(e for e in events if e['type'] == 'attack_started' and e['phase3']['baseline'] is not None)
                    if category == 'missing':
                        events.remove(event)
                    elif category == 'duplicate':
                        events.append(copy.deepcopy(event))
                    elif category == 'ordinary':
                        event['phase3']['baseline'] = None
                    elif category == 'baseline_tick':
                        event['phase3']['baseline'] -= 1
                    elif category in ('start', 'impact', 'ready'):
                        event['tick' if category == 'start' else category] += 1
                    elif category == 'start_at_entry':
                        event['tick'] = row['tick']
                    elif category == 'impact_at_entry':
                        event['impact'] = row['tick']
                    elif category == 'attacker_life':
                        event['phase3']['attacker_incarnation'] += 1
                    elif category == 'target_life':
                        event['target_incarnation'] += 1
                    elif category == 'hidden_target':
                        event['phase3']['target_visible'] = False
                    elif category == 'unknown_target':
                        event['target'] = FIRST
                    elif category == 'other_npc':
                        event['attacker'] = next(e['entity'] for e in events if e['type'] == 'entity_spawn' and e['combat']['template'] == 'keltir_static')
                    elif category.startswith('before_'):
                        if category == 'before_attacker_identity':
                            events.remove(event)
                            index = next(i for i, e in enumerate(events) if e['type'] == 'entity_spawn' and e['entity'] == event['attacker'])
                            events.insert(index, event)
                        else:
                            identity = next(e for e in events if e['type'] == 'entity_spawn' and e['combat']['template'] == 'keltir_static')
                            events.remove(identity)
                            events.append(identity)  # Attacker known, identity batch still incomplete.
                    elif category in ('other_tick', 'other_observer', 'initial_entry_only'):
                        events.remove(event)
                        destination = next(r for r in altered if r['kind'] == 'observer_combat'
                                           and (r['tick'] == row['tick'] and r['entity'] != observer if category == 'other_observer'
                                                else r['entity'] == observer and (r['tick'] == 1 if category == 'initial_entry_only' else r['tick'] > row['tick'])))
                        destination['data']['events'].append(event)
                    elif category == 'identity_target_life':
                        next(e for e in events if e['type'] == 'entity_spawn' and e['entity'] == event['target'])['combat']['incarnation'] += 1
                    elif category == 'native_swing':
                        next(r for r in altered if r['kind'] == 'auto_attack_resolution' and r['entity'] == event['attacker']
                             and r['tick'] == event['impact'])['data']['inputs']['swing']['start'] += 1
                    result = self.check('late_entry_hp_continuity keltir_durable', observer, altered, recording)
                    self.assertFalse(result['passed'])
                    self.assertEqual(result['gates'][0]['sequences'], [sequence])

    def test_actual_export_is_deterministic_and_all_categories_are_retained(self):
        second = self.root / 'second.jsonl'
        self.export(second)
        self.assertEqual(second.read_bytes(), self.raw.read_bytes())
        manifest, facts = evidence.validate(self.data, RECORDING)
        self.assertEqual(manifest['recording_sha256'], hashlib.sha256(RECORDING.read_bytes()).hexdigest())
        self.assertEqual(self.data[-1]['facts'], len(facts))
        self.assertEqual([r['sequence'] for r in facts], list(range(len(facts))))
        self.assertIn('auto_attack_resolution', {r['kind'] for r in facts})
        self.assertIn('rng_draw', {r['kind'] for r in facts})

    def test_native_regen_counts_and_fraction_oracle_are_owner_and_posture_bound(self):
        for entity in (FIRST, SECOND):
            self.assertTrue(self.check('regen_pulses >= 3', entity)['passed'])
            self.assertTrue(self.check('regen_matches_oracle', entity)['passed'])
        self.assertFalse(self.check('regen_pulses >= 26')['passed'])
        self.assertFalse(self.check('moving_regen_pulses >= 1')['passed'])
        self.assertFalse(self.check('moving_regen_matches_oracle')['passed'])
        self.assertFalse(self.check('regen_matches_oracle', '00000000-0000-0000-0000-000000000099')['passed'])
        for field in ('hp', 'mp', 'cp', 'hp_granted', 'mp_granted', 'cp_granted'):
            self.data = copy.deepcopy(self.native)
            self.fact('regen_pulse', FIRST)['data']['actual'][field] += 1
            self.assertFalse(self.check('regen_matches_oracle')['passed'], field)
        self.data = copy.deepcopy(self.native)
        self.fact('regen_pulse', FIRST)['data']['actual']['fractions']['cp'] += 1
        self.assertFalse(self.check('regen_matches_oracle')['passed'])
        self.data = copy.deepcopy(self.native)
        self.fact('regen_pulse', FIRST)['data']['amounts']['hp_q'] += 1
        self.assertFalse(self.check('regen_pulses >= 3')['passed'])

    def test_generic_regen_counts_moving_without_changing_exact_oracle(self):
        row = self.fact('regen_pulse', FIRST)
        row['data']['inputs']['posture'] = 'Moving'
        # Counts include both postures; the input mutation must still fail exact math.
        report = self.check('regen_pulses >= 3')
        self.assertIn(row['sequence'], report['gates'][0]['sequences'])
        self.assertFalse(report['passed'])

    def test_actual_resurrection_vitals_are_not_limited_to_vector_table(self):
        row = next(r for r in self.data[1:-1] if r['kind'] == 'resurrection_refund' and r['data']['accepted'])
        owner = row['entity']
        self.assertTrue(self.check('resurrection_vitals_match_fixture', owner)['passed'])
        for resource in ('hp', 'mp'):
            saved = row['data']['vitals_after']['combat'][resource]
            row['data']['vitals_after']['combat'][resource] += 1
            self.assertFalse(self.check('resurrection_vitals_match_fixture', owner)['passed'])
            row['data']['vitals_after']['combat'][resource] = saved

    def test_same_tick_distinct_facts_are_valid_but_duplicate_pulses_do_not_count(self):
        self.assertTrue(self.check('skill_hate_outcome_coherent')['passed'])
        pulse = self.fact('regen_pulse', FIRST)
        position = pulse['sequence'] + 1
        self.data.insert(position, copy.deepcopy(pulse))
        for sequence, row in enumerate(self.data[1:-1]):
            row['sequence'] = sequence
        self.data[-1]['facts'] += 1
        self.assertFalse(self.check('regen_pulses >= 3')['passed'])

    def test_native_death_record_uses_recorded_loss_rules(self):
        self.assertTrue(self.check('death_record_matches_fixture', SECOND)['passed'])
        self.assertFalse(self.check('death_record_matches_fixture', FIRST)['passed'])
        for name in ('loss', 'pre_death_xp', 'incarnation'):
            self.data = copy.deepcopy(self.native)
            self.fact('death_record', SECOND)['data'][name] += 1
            self.assertFalse(self.check('death_record_matches_fixture', SECOND)['passed'], name)
        self.data = copy.deepcopy(self.native)
        self.fact('death_record', SECOND)['data']['penalty_roll'] = 0
        self.assertFalse(self.check('death_record_matches_fixture', SECOND)['passed'])

    def test_native_refund_requires_offer_independent_percent_and_exact_once_consumption(self):
        self.assertTrue(self.check('resurrect_refund_matches_oracle', SECOND)['passed'])
        self.assertFalse(self.check('resurrect_refund_matches_oracle', FIRST)['passed'])
        for name in ('xp_after', 'expected_xp_refund', 'incarnation_after'):
            self.data = copy.deepcopy(self.native)
            self.fact('resurrection_refund', SECOND)['data'][name] += 1
            self.assertFalse(self.check('resurrect_refund_matches_oracle', SECOND)['passed'], name)
        self.data = copy.deepcopy(self.native)
        self.fact('resurrection_offer', SECOND)['data']['wit_bonus'] = '2/1'
        self.assertFalse(self.check('resurrect_refund_matches_oracle', SECOND)['passed'])

    def test_native_hate_uses_npc_subject_source_owner_and_selection(self):
        result = self.check('skill_hate_outcome_coherent')
        self.assertTrue(result['passed'], result)
        self.assertEqual(len(result['gates'][0]['sequences']), 3)
        self.assertFalse(self.check('skill_hate_outcome_coherent', SECOND)['passed'])
        self.fact('hate_selection')['data']['most_hated']['entity'] = SECOND
        self.assertFalse(self.check('skill_hate_outcome_coherent')['passed'])
        self.data = copy.deepcopy(self.native)
        self.fact('hate_delta')['data']['amount'] += 1
        self.assertFalse(self.check('skill_hate_outcome_coherent')['passed'])

    def test_absent_native_transitions_stay_red_and_actual_damage_passes(self):
        absent = ('death_record_matches_before_disconnect', 'restore_did_not_reapply_hooks',
                  'moving_regen_pulses >= 1', 'moving_regen_matches_oracle',
                  'skill_effect_matches_oracle 1184', 'stat_matches_oracle active', 'stat_matches_oracle removed')
        for declaration in absent:
            for entity in (FIRST, SECOND):
                result = self.check(declaration, entity)
                self.assertFalse(result['passed'], declaration)
                self.assertEqual(result['gates'][0]['ticks'], [])
        numeric = self.check('skill_damage_matches_oracle 1177')
        self.assertTrue(numeric['passed'], numeric)
        self.assertEqual(len(numeric['gates'][0]['ticks']), 5)
        self.assertEqual(numeric['gates'][0]['errors'], [])
        # Numeric acceptance never comes from a producer's guessed oracle index.
        self.fact('damage_formula')['data']['vector_index'] = 0
        with self.assertRaises(ValueError):
            self.check('skill_damage_matches_oracle 1177')

    def generated_case(self, name):
        recording = self.generated / (name + '.nfr')
        rows = evidence.read_jsonl(self.generated / (name + '.jsonl'))
        owner = next(r['entity'] for r in rows if r['kind'] == 'admission_reconstruction')
        return recording, rows, owner

    def test_actual_scenario_powers_levels_modifiers_and_spread_pass_independent_oracle(self):
        recording, rows, owner = self.generated_case('actual-inputs')
        for declaration in ('skill_damage_matches_oracle 1177', 'skill_damage_matches_oracle 1184',
                            'skill_effect_matches_oracle 1184', 'skill_effect_matches_oracle 1068',
                            'stat_matches_oracle active', 'stat_matches_oracle removed'):
            result = self.check(declaration, owner, rows, recording)
            self.assertTrue(result['passed'], result)
        pvp_recording, pvp_rows, pvp_owner = self.generated_case('pvp-inputs')
        result = self.check('skill_damage_matches_oracle 1177', pvp_owner, pvp_rows, pvp_recording)
        self.assertTrue(result['passed'], result)
        pvp_damage = next(r for r in pvp_rows if r['kind'] == 'damage_formula')
        self.assertEqual(pvp_damage['data']['power'], '21/1')
        pvp_resolution = next(r for r in pvp_rows if r['kind'] == 'formula_resolution')
        self.assertEqual(pvp_resolution['data']['inputs']['caster']['effective']['physical']['level'], 14)
        damages = [r for r in rows if r['kind'] == 'damage_formula']
        self.assertEqual({r['data']['power'] for r in damages}, {'13/1', '8/1'})
        self.assertTrue(any(evidence.ratio(r['data']['spread']) != 1 for r in damages))
        active = next(r for r in rows if r['kind'] == 'modifier_reconstruction'
                      and r['entity'] == owner and r['data']['after']['phase3']['effects']['entries'])
        self.assertEqual(active['data']['after']['effective']['physical']['level'], 14)
        modifier = next(f for f in active['data']['functions'] if f['key']['stat'] == 'pAtk'
                        and f['key']['install_sequence'] != 0)
        self.assertEqual(modifier['operation'], {'Mul': 1080000})
        altered = copy.deepcopy(rows)
        changed = altered[active['sequence'] + 1]['data']['after']['effective']['physical']
        changed['p_atk'] += 1
        self.assertFalse(self.check('stat_matches_oracle active', owner, altered, recording)['passed'])
        for name in ('power', 'magic_success_rate', 'rolled_damage', 'm_atk_q'):
            altered = copy.deepcopy(rows)
            data = altered[damages[0]['sequence'] + 1]['data']
            data[name] = '14/1' if name == 'power' else data[name]+1
            self.assertFalse(self.check('skill_damage_matches_oracle 1177', owner, altered, recording)['passed'], name)
        altered = copy.deepcopy(rows)
        altered[active['sequence'] + 1]['data']['after']['effective']['calculator'].append(
            {'key': dict(modifier['key'], stat='unsupportedStat'), 'conditions': [], 'operation': {'Mul': 1080000}})
        self.assertFalse(self.check('stat_matches_oracle active', owner, altered, recording)['passed'])

    def test_karma_decay_uses_actual_credited_xp_and_pre_award_level(self):
        recording, rows, owner = self.generated_case('pvp-inputs')
        result = self.check('karma_decay_matches_oracle', owner, rows, recording)
        self.assertTrue(result['passed'], result)
        row = next(r for r in rows if r['kind'] == 'karma_decay')
        self.assertEqual(row['data']['pre_award_level'], 14)
        self.assertGreater(row['data']['karma_before'], row['data']['karma_after'])
        for field in ('karma_after', 'xp_after', 'pre_award_level'):
            altered = copy.deepcopy(rows)
            # One XP may fall within the same integer bucket; force the next bucket.
            altered[row['sequence']+1]['data'][field] += {'xp_after': 1000, 'pre_award_level': 70, 'karma_after': 1}[field]
            self.assertFalse(self.check('karma_decay_matches_oracle', owner, altered, recording)['passed'], field)

    def test_committed_cast_fixture_continuous_effect_uses_callable_duration_oracle(self):
        result = self.check('skill_effect_matches_oracle 1040', data=self.casts, recording=self.casts_recording)
        self.assertTrue(result['passed'], result)
        self.assertEqual(len(result['gates'][0]['ticks']), 2)
        altered = copy.deepcopy(self.casts)
        next(r for r in altered if r['kind'] == 'effect_formula')['data']['duration_seconds'] += 1
        self.assertFalse(self.check('skill_effect_matches_oracle 1040', data=altered, recording=self.casts_recording)['passed'])

    def test_reconnect_correlates_initial_admission_live_mutation_and_restored_admission(self):
        for name, gate in (('actual-inputs', 'restore_did_not_reapply_hooks'),
                           ('death-reconnect', 'death_record_matches_before_disconnect'),
                           ('death-cap', 'death_record_matches_before_disconnect')):
            recording, rows, owner = self.generated_case(name)
            result = self.check(gate, owner, rows, recording)
            self.assertTrue(result['passed'], result)
            admissions = [r for r in rows if r['kind'] == 'admission_reconstruction' and r['entity'] == owner]
            self.assertEqual(len(admissions), 2)
            self.assertEqual(result['gates'][0]['sequences'], [admissions[-1]['sequence']])
            for mode in ('no_reconnect', 'no_mutation', 'wrong_retained'):
                altered = copy.deepcopy(rows)
                if mode == 'no_reconnect':
                    altered.remove(altered[admissions[-1]['sequence'] + 1])
                elif mode == 'no_mutation':
                    if gate.startswith('death_'):
                        altered = [r for r in altered if r['kind'] != 'death_record']
                    else:
                        altered = [r for r in altered if r['kind'] != 'formula_resolution']
                else:
                    checkpoint = altered[admissions[-1]['sequence'] + 1]['data']['load']['phase3']
                    if gate.startswith('death_'):
                        checkpoint['last_death']['pre_death_xp'] += 1
                    else:
                        checkpoint['effect_remainders'][0]['abnormal_level'] += 1
                for sequence, row in enumerate(altered[1:-1]):
                    row['sequence'] = sequence
                altered[-1]['facts'] = len(altered)-2
                self.assertFalse(self.check(gate, owner, altered, recording)['passed'], (name,mode))
            initial_only = rows[:admissions[0]['sequence']+2] + [dict(rows[-1])]
            initial_only[-1]['facts'] = len(initial_only)-2
            self.assertFalse(self.check(gate, owner, initial_only, recording)['passed'])

    def test_real_modified_regeneration_requires_credit_for_canonical_stat_name(self):
        recording, rows, owner = self.generated_case('modified-regen')
        self.assertTrue(self.check('regen_matches_oracle', owner, rows, recording)['passed'])
        pulses = [r for r in rows if r['kind'] == 'regen_pulse' and r['entity'] == owner]
        base = next(r for r in pulses if not any(f['key']['stat'] == 'regHp' for f in r['data']['functions']))
        modified = next(r for r in pulses if any(f['key']['stat'] == 'regHp' for f in r['data']['functions']))
        self.assertGreater(modified['data']['amounts']['hp_q'], base['data']['amounts']['hp_q'])
        altered = copy.deepcopy(rows)
        # Keep the native installed regHp function but omit its formula credit.
        altered[modified['sequence'] + 1]['data']['amounts']['hp_q'] = base['data']['amounts']['hp_q']
        self.assertFalse(self.check('regen_matches_oracle', owner, altered, recording)['passed'])
        # Review13 repro: append a canonical regHp x2 without changing any credit.
        for stat in ('regHp', 'regMp', 'regCp'):
            altered = copy.deepcopy(self.native)
            pulse = next(r for r in altered if r['kind'] == 'regen_pulse' and r['entity'] == FIRST)
            pulse['data']['functions'].append({'key': {'stat':stat, 'priority':32, 'install_sequence':1,
                'handler_ordinal':0, 'function_ordinal':0}, 'conditions':[], 'operation':{'Mul':2000000}})
            self.assertFalse(self.check('regen_matches_oracle', data=altered)['passed'], stat)

    def test_manifest_trailer_sequence_owner_scope_and_unknown_fields_fail_closed(self):
        cases = (
            lambda d: d[0].update(recording_sha256='a'*64),
            lambda d: d[0].update(schema_version=True),
            lambda d: d[0].update(scope='selected_session'),
            lambda d: d[0]['watermark'].update(epoch=d[0]['epoch']+1),
            lambda d: d[-1].update(facts=d[-1]['facts']+1),
            lambda d: d[-1].update(ticks=True),
            lambda d: d[-1].update(last_tick=d[-1]['last_tick']+1),
            lambda d: d.pop(),
            lambda d: d[1].update(sequence=1),
            lambda d: d[1].update(tick=d[0]['snapshot_tick']-1),
            lambda d: d[1].update(entity='bad-owner'),
            lambda d: d[1].update(kind='future_raw_fact'),
            lambda d: d[1].update(passed=True),
            lambda d: d[1]['data'].update(passed=True),
        )
        for mutation in cases:
            data = copy.deepcopy(self.native)
            mutation(data)
            with self.assertRaises((ValueError, TypeError)):
                self.check('regen_pulses >= 3', data=data)
        with self.assertRaises(ValueError):
            evidence.evaluate({'schema_version': 1, 'facts': []}, RECORDING, FIRST, [])

    def test_rng_slice_copies_are_checked_without_double_consumption(self):
        resolved = next(r for r in self.data if r['kind'] == 'formula_resolution' and r['data']['rng_draws'])
        for mutate in (lambda draws: draws.reverse(), lambda draws: draws.append(copy.deepcopy(draws[-1])),
                       lambda draws: draws[0].update(value=draws[0]['value']+1)):
            self.data = copy.deepcopy(self.native)
            row = self.data[resolved['sequence']+1]
            mutate(row['data']['rng_draws'])
            with self.assertRaises(ValueError):
                self.check('regen_pulses >= 3')
        self.data = copy.deepcopy(self.native)
        draw = self.fact('rng_draw')['data']
        draw['value'] = draw['bound']
        with self.assertRaises(ValueError):
            self.check('regen_pulses >= 3')
        for kind in ('formula_resolution', 'damage_formula'):
            self.data = copy.deepcopy(self.native)
            self.fact(kind)['data']['target']['incarnation'] = True
            with self.assertRaises(ValueError):
                self.check('regen_pulses >= 3')

    def test_jsonl_duplicates_nonexact_numbers_blank_lines_and_incomplete_tail_rejected(self):
        path = self.root / 'bad.jsonl'
        for text in ('{"kind":"manifest","kind":"complete"}\n', '{"bound":1.0}\n',
                     '{"value":NaN}\n', '\n', self.raw.read_text() + '{"kind":'):
            path.write_text(text)
            with self.assertRaises(ValueError):
                evidence.read_jsonl(path)

    def test_run_uses_real_export_and_fresh_unit_artifacts(self):
        scenario = self.root / 'native.nfs'
        scenario.write_text('# replay-gate: regen_matches_oracle\n')
        report = self.root / 'native.xml'
        report.write_text(f'<testsuite><properties><property name="own_entity_id" value="{FIRST}"/></properties></testsuite>')
        out = self.root / 'gates.json'
        self.assertEqual(evidence.run(scenario, RECORDING, report, out, str(self.binary)), 0)
        self.assertTrue(json.loads(out.read_text())['passed'])
        self.assertEqual(out.with_suffix('.raw.jsonl').read_bytes(), self.raw.read_bytes())
        with self.assertRaisesRegex(ValueError, 'fresh'):
            evidence.run(scenario, RECORDING, report, out, str(self.binary))
        scenario.write_text('# replay-gate: moving_regen_matches_oracle\n')
        failed = self.root / 'missing.json'
        self.assertEqual(evidence.run(scenario, RECORDING, report, failed, str(self.binary)), 1)
        self.assertFalse(json.loads(failed.read_text())['passed'])

    def test_real_export_flows_through_both_wrappers_per_unit_including_no_replay(self):
        # Only bot/JUnit/health are stubs. Recording replay, evidence export, gate math,
        # coverage and wrapper classification are real; this is not Unreal acceptance.
        native_bot = self.root / 'native-bot.py'
        native_bot.write_text('''#!/usr/bin/env python3
import os, shutil, sys, xml.etree.ElementTree as ET
from pathlib import Path
scenario = Path(next(a.split('=',1)[1] for a in sys.argv if a.startswith('-BotScenario=')))
scripts = Path(os.environ['NATIVE_SCRIPTS'])
sys.path.insert(0, str(scripts / 'test'))
import importlib.util
spec = importlib.util.spec_from_file_location('fake', scripts / 'test/fake-artifacts.py')
fake = importlib.util.module_from_spec(spec); spec.loader.exec_module(fake)
out = Path(os.environ['SIM_SAVED_DIR']); out.mkdir(parents=True,exist_ok=True)
fake.bot(out, scenario.stem, 'pass', 0, 1)
report = out / (scenario.stem + '.xml'); tree = ET.parse(report)
tree.find('.//property[@name="own_entity_id"]').set('value', os.environ['NATIVE_SECOND'] if scenario.stem.endswith('-b') else os.environ['NATIVE_FIRST'])
tree.write(report)
shutil.copyfile(os.environ['NATIVE_RECORDING'], out / (scenario.stem + '.nfr'))
''')
        native_bot.chmod(0o755)
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'curl').write_text('#!/bin/sh\nexit 0\n')
        (bin_dir / 'curl').chmod(0o755)
        replay_check = self.root / 'check.sh'
        replay_check.write_text('#!/bin/sh\nexec "$NATIVE_REPLAY" check --file "$1"\n')
        replay_check.chmod(0o755)
        env = dict(os.environ, PATH=str(bin_dir)+':'+os.environ['PATH'], SIM_SKIP_BUILD='1',
                   SIM_BOT_BIN=str(native_bot), SIM_SAVED_DIR=str(self.root / 'saved'),
                   SIM_REPLAY_BIN=str(self.binary), SIM_REPLAY_CMD=str(replay_check),
                   SIM_COVERAGE_CMD=str(self.binary), SIM_PHASE3_EVIDENCE_CMD=str(self.binary),
                   SIM_TRACE_CMD='bash ' + str(SCRIPTS / 'test/fake-trace.sh'),
                   NATIVE_REPLAY=str(self.binary), NATIVE_RECORDING=str(RECORDING),
                   NATIVE_SCRIPTS=str(SCRIPTS), NATIVE_FIRST=FIRST, NATIVE_SECOND=SECOND,
                   PYTHONDONTWRITEBYTECODE='1')
        for key in ('SIM_REQUIRE_OWNED_API', 'SIM_PIPELINE_VERDICT', 'SIM_REQUIRE_TRANSITION_COVERAGE'):
            env.pop(key, None)
        for multi in (False, True):
            for missing in (False, True):
                paths = [self.root / ('native-a.nfs' if multi else 'native.nfs')]
                paths[0].write_text('# replay-gate: ' + ('moving_regen_matches_oracle' if missing else 'skill_damage_matches_oracle 1177') + '\nnf.Within 30\n')
                if multi:
                    peer = self.root / 'native-b.nfs'
                    peer.write_text('# replay-gate: death_record_matches_fixture\nnf.Within 30\n')
                    paths.append(peer)
                out = self.root / f'artifacts-{multi}-{missing}'
                script = SCRIPTS / ('run-sim-multi.sh' if multi else 'run-sim.sh')
                command = ['bash', str(script), '--no-replay', '--artifacts', str(out), *map(str, paths)]
                result = subprocess.run(command, env=env, cwd=REPO, capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, int(missing), result.stdout+result.stderr)
                verdict = json.loads((out / 'pipeline-verdict.json').read_text())
                self.assertEqual(bool(verdict['infrastructure_failures']), missing, verdict)
                for path in paths:
                    folder = out / path.stem
                    self.assertEqual((folder / 'phase3-evidence.raw.jsonl').read_bytes(), self.raw.read_bytes())
                    result = json.loads((folder / 'phase3-evidence.json').read_text())
                    self.assertEqual(result['entity'], SECOND if path.stem.endswith('-b') else FIRST)
                    self.assertEqual(result['passed'], not(missing and path == paths[0]))
                if missing:
                    self.assertTrue(any('phase3_replay_evidence' in e for e in verdict['infrastructure_failures']))


if __name__ == '__main__':
    unittest.main()
