#!/usr/bin/env python3
"""Synthetic consumer negatives; these do not attest a native gameplay recording."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
spec = importlib.util.spec_from_file_location('evidence', SCRIPTS / 'phase3-evidence.py')
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)
ENTITY = '01970000-0000-7000-8000-000000000030'
PEER = '01970000-0000-7000-8000-000000000031'


class ReplayEvidence(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.recording = self.root / 'unit.nfr'
        self.recording.write_bytes(b'synthetic recording identity')
        self.data = {'schema_version': 1, 'recording_sha256': hashlib.sha256(self.recording.read_bytes()).hexdigest(),
                     'zone': 1, 'epoch': 1, 'facts': []}

    def row(self, kind, inputs, actual, tick=1, skill=0, phase=''):
        return {'kind': kind, 'entity': ENTITY, 'tick': tick, 'skill_id': skill,
                'phase': phase, 'inputs': inputs, 'actual': actual}

    def check(self, data, declaration):
        path = self.root / 'scenario.nfs'
        path.write_text('# replay-gate: ' + declaration + '\n')
        return evidence.evaluate(data, self.recording, ENTITY, evidence.requirements(path))

    def test_exact_twelve_declared_facts_and_zero_unavailable_client_predicates(self):
        kinds = set()
        for path in (SCRIPTS.parent / 'Scenarios').rglob('3-*.nfs'):
            kinds.update(gate['kind'] for gate in evidence.requirements(path))
            for line in path.read_text().splitlines():
                if line.startswith(('nf.Expect ', 'nf.WaitFor ')):
                    self.assertNotIn(line.split()[1], evidence.FACTS, path.name)
        self.assertEqual(kinds, evidence.FACTS)

    def test_pulses_are_entity_tick_bound_nonvacuous_and_never_guessed_from_vitals(self):
        kind = 'regen_pulses'
        self.data['facts'] = [self.row(kind, {'posture': 'standing'}, {'pulse_count': 1}, tick=i)
                              for i in (30, 60, 90)]
        self.assertTrue(self.check(self.data, kind + ' >= 3')['passed'])
        for mutation in (lambda d: d['facts'].pop(),
                         lambda d: d['facts'][0].update(entity=PEER),
                         lambda d: d['facts'][0]['inputs'].update(posture='moving'),
                         lambda d: d['facts'][0].update(actual={'pulse_count': True})):
            data = copy.deepcopy(self.data)
            mutation(data)
            self.assertFalse(self.check(data, kind + ' >= 3')['passed'])
        data = copy.deepcopy(self.data)
        data['facts'].append(data['facts'][0])
        with self.assertRaises(ValueError): self.check(data, kind + ' >= 3')

    def test_all_numeric_oracles_compare_exact_inputs_and_actual_outcome(self):
        source = REPO / 'packages/data/fixtures/skill-oracle.json'
        oracle = json.loads(source.read_text())
        for kind, formula in (('skill_damage_matches_oracle', 'calcMagicDam'),
                              ('skill_effect_matches_oracle', 'calcEffectSuccess'),
                              ('stat_matches_oracle', 'FuncMAtkMod')):
            vector = oracle['formulas'][formula][0]
            inputs = dict(formula=formula, vector_index=0, formula_inputs=vector['inputs'],
                          oracle_sha256=hashlib.sha256(source.read_bytes()).hexdigest())
            if kind == 'skill_effect_matches_oracle':
                inputs['effect_roll'] = None
                actual = {'landed': True, 'chance_q': vector['q_result']}
            elif kind == 'skill_damage_matches_oracle':
                actual = {'rolled_damage': vector['rounded_result'], 'outcome': 'hit'}
            else:
                actual = {'value_q': vector['q_result']}
            self.data['facts'] = [self.row(kind, inputs, actual, skill=1177, phase='active')]
            declaration = kind + (' active' if kind == 'stat_matches_oracle' else ' 1177')
            self.assertTrue(self.check(self.data, declaration)['passed'])
            self.data['facts'][0]['actual'] = {}
            self.assertFalse(self.check(self.data, declaration)['passed'])
            self.data['facts'][0]['actual'] = actual
            self.data['facts'][0]['inputs']['formula_inputs'] = {}
            self.assertFalse(self.check(self.data, declaration)['passed'])

    def test_consistency_and_fraction_checks_have_independent_expected_values(self):
        death = {'incarnation': 1, 'pre_death_xp': 1000, 'recoverable_death_xp_loss': 75}
        cases = [
            ('death_record_matches_fixture', {'incarnation': 1, 'pre_death_xp': 1000,
                 'level_span': 1000, 'loss_percent': '7.5'}, death),
            ('death_record_matches_before_disconnect', {'before': death, 'after_incarnation': 2}, death | {'incarnation': 2}),
            ('restore_did_not_reapply_hooks', {'retained_modifiers': [{'kind': 'death_penalty'}],
                 'retained_periodic_phase': []}, {'modifiers': [{'kind': 'death_penalty'}], 'periodic_phase': [],
                                               'one_time_hook_invocations': 0, 'resource_hook_credit_q': 0}),
            ('resurrect_refund_matches_oracle', {'base_restore_percent': 20, 'wit_bonus': '201/100',
                 'recoverable_loss': 100, 'xp_before': 900}, {'xp_refund': 60, 'xp_after': 960,
                                               'remaining_recoverable_loss': 0, 'refund_count': 1}),
            ('skill_hate_outcome_coherent', {'ledger_before': {}, 'attacker': ENTITY, 'effect_point': 10,
                 'rolled_damage': 8, 'npc_level': 1, 'hate_cap': 999999999, 'eligible': [ENTITY],
                 'current_target': None}, {'ledger': {ENTITY: 101}, 'most_hated': ENTITY}),
        ]
        resource = dict(base='2', level_mod='1', stat_bonus='1', functions=[],
                        character_multiplier='1', posture_multiplier='11/10',
                        before=1, maximum=10, fraction_q=900000)
        resources = {key: resource for key in ('hp', 'mp', 'cp')}
        for kind, posture, multiplier, rate, after, fraction in (
                ('regen_matches_oracle', 'standing', '11/10', 2200000, 4, 100000),
                ('moving_regen_matches_oracle', 'moving', '7/10', 1400000, 3, 300000)):
            inputs = dict(posture=posture, alive=True,
                          resources={key: value | {'posture_multiplier': multiplier} for key, value in resources.items()})
            actual = {key: dict(after=after, fraction_q=fraction, rate_q=rate) for key in resources}
            cases.append((kind, inputs, actual))
        for kind, inputs, actual in cases:
            self.data['facts'] = [self.row(kind, inputs, actual)]
            self.assertTrue(self.check(self.data, kind)['passed'], kind)
            self.data['facts'][0]['actual'] = {}
            self.assertFalse(self.check(self.data, kind)['passed'], kind)

    def test_missing_facts_wrong_recording_or_producer_pass_flag_cannot_pass(self):
        for kind in evidence.FACTS:
            declaration = kind + (' >= 1' if kind.endswith('pulses') else ' 1177' if kind.startswith('skill_') and kind != 'skill_hate_outcome_coherent' else ' active' if kind == 'stat_matches_oracle' else '')
            self.assertFalse(self.check(self.data, declaration)['passed'], kind)
        data = self.data | {'recording_sha256': 'a' * 64}
        with self.assertRaises(ValueError): self.check(data, 'regen_pulses >= 1')
        data = self.data | {'facts': [{'kind': 'regen_pulses', 'passed': True}]}
        with self.assertRaises(ValueError): self.check(data, 'regen_pulses >= 1')
        with self.assertRaises(ValueError): evidence.unique_object([('kind', 1), ('kind', 2)])


if __name__ == '__main__':
    unittest.main()
