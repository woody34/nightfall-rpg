#!/usr/bin/env python3
"""Assert variant behavior declarations and evidence rejection; no gameplay pass."""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
SCENARIOS = SCRIPTS.parent / 'Scenarios'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


variants = load('variants', SCRIPTS / 'phase3-variants.py')
packs = load('variant_packs_test', SCRIPTS / 'phase3-fixtures.py')
seed = load('variant_seed', REPO / 'infra/scripts/seed-phase3-fixture.py')
ci = load('variant_ci', SCRIPTS / 'run-sim-ci.py')


class VariantContracts(unittest.TestCase):
    def test_each_runtime_case_has_specific_assertions_fixture_and_complete_roles(self):
        data = variants.read_contract()
        self.assertEqual(len(data['runtime_cases']), 19)
        paths = list((SCRIPTS.parent / 'Scenarios/variants').glob('3-*.nfs'))
        for batch in ci.units(paths):
            ci.fixture(batch)
        for name, field, value in (
            ('phase3-trainer-low-sp', 'sp', 0),
            ('phase3-trainer-low-level', 'level', 4),
            ('phase3-trainer-low-mp', 'mp', 0),
            ('phase3-penalty-cap', 'death_penalty_level', 5),
            ('phase3-sp-near-cap', 'sp', 9223372036854775806),
        ):
            role = packs.profiles(name, REPO)['owner']
            self.assertEqual(role[field], value)
            role['account_id'] = '01970000-0000-7000-8000-000000000099'
            sql = seed.seed_sql({'owner': role})
            if field == 'death_penalty_level':
                self.assertIn("'female',0,0,0,0,5,0,1)", sql)
            if field == 'sp':
                self.assertIn(str(value) + ",'female'", sql)
        self.assertIn('rglob("*.nfs")', (SCRIPTS / 'run-sim-ci.py').read_text())
        with self.assertRaises(ValueError):
            ci.units([SCENARIOS / '3-cast-skill.nfs', SCENARIOS / 'variants/3-cast-skill.nfs'])

    def test_header_only_or_wrong_fixture_or_missing_assertions_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'Scenarios'
            shutil.copytree(SCRIPTS.parent / 'Scenarios', root)
            path = root / 'variants/3-learn-low-sp.nfs'
            original = path.read_text()
            for source in (original.split('nf.Login')[0],
                           original.replace('# fixture: phase3-trainer-low-sp', '# fixture: phase3-trainer'),
                           original.replace('nf.Expect learned_rank 1177 == 1', 'nf.Expect learned_rank 1177 >= 0')):
                path.write_text(source)
                with self.assertRaises(ValueError): variants.read_contract(root)
            path.write_text(original)
            self.assertEqual(variants.read_contract(root)['schema_version'], 1)

    def test_precision_matrix_has_deterministic_rolls_and_required_layers(self):
        data = variants.read_contract()
        cases = {c['id']: c for c in data['precision_cases']}
        for name in ('debuff-land', 'debuff-resist', 'death-penalty-roll-success', 'death-penalty-roll-failure'):
            c = cases[name]
            self.assertIsNone(c['evidence'])
            self.assertTrue(c['deterministic_roll_fixture']['require_full_ordered_draw_trace'])
        self.assertEqual(cases['debuff-land']['deterministic_roll_fixture']['value'], 0)
        self.assertEqual(cases['debuff-resist']['deterministic_roll_fixture']['value'], 99)
        self.assertEqual(cases['death-penalty-roll-success']['deterministic_roll_fixture']['value'], 0)
        self.assertEqual(cases['death-penalty-roll-failure']['deterministic_roll_fixture']['value'], 99)
        self.assertEqual(cases['death-penalty-roll-success']['deterministic_roll_fixture']['source_draw']['value'], 1)
        self.assertEqual(cases['death-penalty-roll-failure']['deterministic_roll_fixture']['source_draw']['value'], 100)
        for name in ('cast-status-guards', 'cast-real-weapon', 'cast-completion-resources',
                     'learn-engagement-guards', 'sp-competing-keltir', 'buff-higher-lower-overflow',
                     'physical-skill', 'dot-nonlethal-break', 'reconnect-reuse-50-51',
                     'reconnect-effects-100-101', 'resurrect-competing-offers', 'resurrect-town-race',
                     'hate-effectpoint-addhate', 'hate-heal-overheal', 'hate-tie-getagro'):
            self.assertIn(name, cases)

    def test_missing_forged_or_wrong_fixture_evidence_fails(self):
        data = variants.read_contract()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.assertEqual(len(variants.evidence_errors(data, root)), len(data['precision_cases']))
            case = next(c for c in data['precision_cases'] if c['id'] == 'debuff-land')
            report = dict(schema_version=1, case_id=case['id'], case_sha256=variants.case_hash(case),
                          passed=True, test_name=case['requested_test'],
                          assertions={a: True for a in case['assertions']}, artifacts={})
            # A green declaration with no retained output/draw trace is still missing evidence.
            (root / (case['id'] + '.json')).write_text(json.dumps(report))
            self.assertTrue(any(e.startswith('debuff-land:') for e in variants.evidence_errors(data, root)))
            changed = copy.deepcopy(case)
            changed['deterministic_roll_fixture']['value'] = 99
            self.assertNotEqual(variants.case_hash(case), variants.case_hash(changed))


if __name__ == '__main__':
    unittest.main()
