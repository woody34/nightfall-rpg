#!/usr/bin/env python3
"""Phase3 fixture guards and orchestration: only command stubs, no API/Compose."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('phase2_fixture_tests', SCRIPTS / 'test/phase2-fixture.test.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
fixture = base.fixture
SEEDER = fixture.REPO / 'infra/scripts/seed-phase3-fixture.py'


class Phase3Tests(unittest.TestCase):
    setUp = base.FixtureTests.setUp
    fake = base.FixtureTests.fake
    scenario = base.FixtureTests.scenario
    run_wrapper = base.FixtureTests.run_wrapper
    lines = base.FixtureTests.lines

    def test_all_packs_before_admission_exact_inputs_and_cleanup(self):
        for pack in sorted(fixture.PHASE3):
            paired = pack in fixture.phase3.PAIRED
            a = self.scenario(pack + ('-a' if paired else ''), pack)
            batch = [a, self.scenario(pack + '-b', pack)] if paired else [a]
            result, artifacts = self.run_wrapper(batch, multi=paired, SIM_PHASE3_SEEDER=str(SEEDER))
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            folder = artifacts / a.stem / 'fixture'
            seed = json.loads((folder / 'seed-manifest.json').read_text())
            manifest = json.loads((folder / 'manifest.json').read_text())
            state = json.loads((folder / 'state.json').read_text())
            self.assertEqual(seed['schema_status'], 'reconciled-000008-0a37fb3')
            self.assertEqual(seed['data_sha256'], manifest['data_sha256'])
            self.assertEqual(manifest['overlay_data_sha256'], fixture.phase3.tree_hashes(folder / 'data'))
            self.assertEqual(seed['roles']['owner']['sp'], manifest['seed_inputs']['owner']['sp'])
            self.assertIn('nf_phase3_fixture_', manifest['database'])
            self.assertTrue(manifest['project'].startswith('nightfall-phase3-harness-'))
            self.assertEqual(state['env']['NIGHTFALL_PHASE3_FIXTURE'], '1')
            self.assertNotIn('NIGHTFALL_PHASE2_FIXTURE', state['env'])
            self.assertTrue(json.loads((folder / 'cleanup-status.json').read_text())['completed'])
            sql = (self.root / 'psql-stdin').read_text()
            self.assertIn('before API startup and admission', sql)
            self.assertIn('character_skill_reuse', sql)
            self.assertIn('SELECT id,0,current_class_id,level,xp,sp', sql)
            roles = list(manifest['roles'].values())
            self.assertEqual(len({r['account_id'] for r in roles}), len(batch))
            for role in roles:
                self.assertEqual(Path(role['token_file']).stat().st_mode & 0o777, 0o600)
            if pack == 'phase3-trainer':
                self.assertEqual(seed['roles']['owner']['learned_ranks']['1177'], 1)
                self.assertEqual(seed['roles']['owner']['position'], [126, 128])
            if pack == 'phase3-resurrect':
                self.assertEqual(seed['roles']['owner']['class_id'], 15)
                self.assertEqual(seed['roles']['owner']['learned_ranks']['1016'], 1)

    def test_seeder_dry_run_guards_reject_default_external_and_ambiguous_databases(self):
        env = dict(os.environ, AUTH_DEV_TOKENS='1', NIGHTFALL_PHASE3_FIXTURE='1')
        command = [sys.executable, str(SEEDER), '--fixture', 'phase3-trainer', '--owner-account',
                   '00000000-0000-0000-0000-000000000001', '--dry-run']
        valid = 'postgres://user:secret@127.0.0.1:27432/nf_phase3_fixture_test'
        for url in (valid, valid.replace('27432', '5432'), valid.replace('127.0.0.1', 'external'),
                    valid.replace('nf_phase3_fixture_test', 'nightfall'), valid + '?options=oops'):
            result = subprocess.run(command, env=env | {'DATABASE_URL': url}, capture_output=True, text=True)
            self.assertEqual(result.returncode == 0, url == valid, result.stderr)
        result = subprocess.run(command, env=env | {'DATABASE_URL': valid, 'NIGHTFALL_PHASE3_FIXTURE': '0'}, capture_output=True)
        self.assertNotEqual(result.returncode, 0)

    def test_pair_and_fresh_mode_refusals(self):
        a = self.scenario('pvp-a', 'phase3-pvp')
        with self.assertRaises(ValueError): fixture.fixture([a])
        b = self.scenario('pvp-b', 'phase3-resurrect')
        with self.assertRaises(ValueError): fixture.fixture([a, b])
        result, _ = self.run_wrapper([self.scenario('trainer', 'phase3-trainer')], mode='attach')
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.events.exists())

    def test_failed_database_seed_stops_admission_and_cleans_owned_stack(self):
        self.fake('docker', f'''echo "docker $*" >> "{self.events}"
if [[ "$*" == *" exec "* ]]; then cat >/dev/null; exit 1; fi
''')
        scenario = self.scenario('trainer', 'phase3-trainer')
        result, artifacts = self.run_wrapper([scenario], SIM_PHASE3_SEEDER=str(SEEDER))
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('api', self.lines())
        self.assertFalse(any(line.startswith('bot ') for line in self.lines()))
        folder = artifacts / scenario.stem / 'fixture'
        self.assertFalse((folder / 'seed-complete').exists())
        self.assertTrue(json.loads((folder / 'cleanup-status.json').read_text())['completed'])

    def test_overlay_preserves_source_rewards_and_skill_rules(self):
        target = self.root / 'overlay'
        fixture.phase3.create_overlay(target, fixture.REPO)
        shipped = tomllib.loads((fixture.REPO / 'packages/data/npcs/keltir.toml').read_text())
        overlaid = tomllib.loads((target / 'npcs/keltir.toml').read_text())
        for key in ('xp_reward', 'sp_reward'):
            self.assertEqual(shipped.get(key), overlaid.get(key))
        for path in (fixture.REPO / 'packages/data').rglob('*'):
            if path.is_file() and 'skills' in path.parts:
                self.assertEqual(path.read_bytes(), (target / path.relative_to(fixture.REPO / 'packages/data')).read_bytes())
        zone = tomllib.loads((target / 'zones/test_zone.toml').read_text())
        self.assertEqual(sum(slot['count'] for slot in zone['spawn_slots'] if slot['template'] == 'keltir'), 1)


if __name__ == '__main__':
    unittest.main()
