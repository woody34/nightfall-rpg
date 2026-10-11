#!/usr/bin/env python3
"""Scenario declarations and orchestration contracts; no gameplay pass attestation."""
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
SCENARIOS = SCRIPTS.parent / 'Scenarios'
REPO = SCRIPTS.parents[2]
spec = importlib.util.spec_from_file_location('phase3_ci', SCRIPTS / 'run-sim-ci.py')
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)

MATRIX_NAMES = ('cast-skill', 'cast-rejected', 'learn-skill', 'learn-rejected', 'buff-expires',
                'debuff-status', 'regen', 'cast-observer-a', 'cast-observer-b', 'cast-reconnect',
                'death-penalty', 'resurrect-a', 'resurrect-b', 'pvp-flag-a', 'pvp-flag-b',
                'pvp-karma-decay', 'aggro-skill-hate')


class ScenarioContracts(unittest.TestCase):
    def test_seventeen_matrix_files_nonvacuous_headers_budgets_and_real_roles(self):
        matrix = dict(re.findall(r'\| `(3-[\w-]+\.nfs)` \| `([^`]+)`',
                                (SCRIPTS / 'test/fixtures/phase-3-plan-stories.md').read_text()))
        self.assertEqual(len(matrix), 17)
        for name in MATRIX_NAMES:
            source = (SCENARIOS / f'3-{name}.nfs').read_text()
            header = source.splitlines()[0]
            self.assertRegex(header, r'^# covers: 3 E\d+\.\d+(, 3 E\d+\.\d+)*$')
            self.assertEqual(header, '# covers: ' + matrix[f'3-{name}.nfs'])
            self.assertGreaterEqual(source.count('nf.Expect '), 2, name)
            self.assertRegex(source, r'(?m)^nf.Within [1-9][0-9]*$')
            self.assertNotRegex(source, r'nf\.(SetHp|SetSp|Grant|Teleport|Gm|TestDamage)\b')
            waits = sum(int(line.split('#', 1)[0].split()[-1]) for line in source.splitlines()
                        if line.startswith('nf.WaitFor '))
            self.assertGreaterEqual(int(re.search(r'^nf.Within (\d+)', source, re.M)[1]), waits + 30)
        batches = ci.units(list(SCENARIOS.glob('3-*.nfs')))
        self.assertEqual(len(batches), 14)
        for batch in batches: ci.fixture(batch)
        for name in ('3-pvp-karma-decay', '3-aggro-skill-hate'):
            batch = next(batch for batch in batches if batch[0].stem == name)
            self.assertEqual(len(batch), 2)
            self.assertNotEqual(batch[0].stem, batch[1].stem)
            self.assertTrue(ci.phase2.phase3.observer_role(batch[1]))
        first = (SCENARIOS / '3-cast-skill.nfs').read_text()
        self.assertLess(first.index('nf.Learn 1177 2'), first.index('nf.Cast 1177'))
        self.assertNotIn('3 E1.4', (SCENARIOS / '3-regen.nfs').read_text().splitlines()[0])

    def test_pvp_branches_require_real_received_state_and_impact_evidence(self):
        owner = (SCENARIOS / '3-pvp-flag-a.nfs').read_text()
        peer = (SCENARIOS / '3-pvp-flag-b.nfs').read_text()
        for assertion in ('own_pvp == 1', 'own_pk == 1', 'own_karma > 0',
                          'player_cp_damage_total > 0', 'rejected == peace_zone',
                          'cast_cancel_reason == peace_zone', 'target_flag_expired_at_last_impact'):
            self.assertIn(assertion, owner)
        self.assertLess(peer.index('nf.WaitFor entity_casting'), peer.index('nf.WaitFor observed_cast_cancel_reason'))
        # Timed flag boundary must still use received deadlines; bounded attack retries
        # may wait between commands but cannot substitute for the expiry proof.
        boundary = owner.split('nf.WaitFor target_flag_remaining <= 10')[1].split('# Finish')[0]
        self.assertNotIn('nf.Sleep', boundary)
        self.assertNotIn('nf.Sleep', peer)

    def test_budget_handles_bot_inline_comments_and_rejects_duplicate_or_invalid_limits(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'budget.nfs'
            path.write_text('nf.Within 3600.1 # source duration\n')
            self.assertEqual(ci.phase2.scenario_budget([path]), 3631)
            for source in ('nf.Within 0', 'nf.Within nope', 'nf.Within 10\nnf.Within 20'):
                path.write_text(source)
                with self.assertRaises(ValueError): ci.phase2.scenario_budget([path])

    def test_long_buff_source_timeout_reaches_wrapper_budget(self):
        result = subprocess.run([sys.executable, str(SCRIPTS / 'phase2-fixture.py'), 'budget',
                                 str(SCENARIOS / '3-buff-expires.nfs')], capture_output=True, text=True, check=True)
        self.assertGreaterEqual(int(result.stdout), 3690 + 30)

    def test_role_manifest_missing_duplicate_and_orphan_fail_before_clients(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            a, b = root / 'flow.nfs', root / 'flow-peer.nfs'
            for path in (a, b): path.write_text('# fixture: phase3-pvp\n')
            manifest = root / 'phase3-roles.json'
            for roles in (['flow.nfs', 'missing.nfs'], ['flow.nfs', 'flow.nfs'], ['flow.nfs', '../outside.nfs']):
                manifest.write_text(json.dumps({'schema_version': 1, 'units': {'flow': roles}}))
                with self.assertRaises(ValueError): ci.units([a, b])
            manifest.write_text(json.dumps({'schema_version': 1, 'units': {'flow': [a.name, b.name]}}))
            with self.assertRaises(ValueError): ci.units([a])
            self.assertEqual(ci.units([b, a]), [[a, b]])
            self.assertEqual(ci.fixture([b, a]), 'phase3-pvp')




if __name__ == '__main__':
    unittest.main()
