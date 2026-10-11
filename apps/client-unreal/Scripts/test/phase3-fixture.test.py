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
            self.assertEqual(state['env']['PHASE3_ENABLED'], '1')
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
                self.assertEqual(seed['roles']['owner']['class_id'], 10)
                self.assertNotIn('1016', seed['roles']['owner']['learned_ranks'])

    def test_every_phase1a_template_has_pinned_profile_without_physical_changes(self):
        shipped = tomllib.loads((fixture.REPO / 'packages/data/npcs/keltir.toml').read_text())
        for selected in ('phase1a-late-entry', 'phase1a-social-aggro'):
            dest = self.root / selected
            fixture.phase3.create_phase1a_overlay(dest, fixture.REPO, selected)
            for original in sorted((fixture.REPO / 'apps/api/fixtures').glob('phase1a-*/npcs/*.toml')):
                npc = tomllib.loads((dest / 'npcs' / original.name).read_text())
                self.assertEqual(npc.pop('phase3'), shipped['phase3'])
                self.assertEqual(npc.pop('sp_reward'), shipped['sp_reward'])
                self.assertEqual(npc, tomllib.loads(original.read_text()))
            before = fixture.phase3.tree_hashes(dest)
            fixture.phase3.upgrade_npc_overlays(dest, fixture.REPO)
            self.assertEqual(before, fixture.phase3.tree_hashes(dest))

    def test_phase2_overlay_is_validated_production_policy(self):
        for pack in sorted(fixture.token_fixtures.TOKEN_CROSSING):
            dest = self.root / pack
            fixture.token_fixtures.create_overlay(dest, pack, fixture.REPO)
            fixture.phase3.upgrade_npc_overlays(dest, fixture.REPO)
            shipped = tomllib.loads((fixture.REPO / 'packages/data/npcs/keltir.toml').read_text())
            for name in ('token_oracle', 'token_sentinel'):
                npc = tomllib.loads((dest / f'npcs/{name}.toml').read_text())
                self.assertEqual(npc['phase3'], shipped['phase3'])
            self.assertEqual(tomllib.loads((dest / 'npcs/token_oracle.toml').read_text())['max_hp'], 1)
        health = fixture.compose_config('test', 'unused', [15432, 14222])['services']['postgres']['healthcheck']
        self.assertIn('-h 127.0.0.1', health['test'][1])

    def test_missing_replay_producer_fails_both_wrappers_even_with_green_client_reports(self):
        for paired in (False, True):
            a = self.scenario('gated-a' if paired else 'gated', 'phase3-trainer')
            a.write_text(a.read_text() + '# replay-gate: death_record_matches_fixture\n')
            batch = [a, self.scenario('gated-b', 'phase3-trainer')] if paired else [a]
            result, artifacts = self.run_wrapper(batch, multi=paired, SIM_PHASE3_SEEDER=str(SEEDER),
                                                  SIM_PHASE3_EVIDENCE_CMD='false')
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            verdict = json.loads((artifacts / 'pipeline-verdict.json').read_text())
            self.assertTrue(any('phase3_replay_evidence' in error for error in verdict['infrastructure_failures']))
            self.assertFalse((artifacts / a.stem / 'phase3-evidence.json').exists())

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
        for name in ('skill_sentinel', 'skill_target'):
            generated = tomllib.loads((target / f'npcs/{name}.toml').read_text())
            self.assertEqual(generated['phase3'], shipped['phase3'])
            self.assertEqual(generated['sp_reward'], shipped['sp_reward'])
        zone = tomllib.loads((target / 'zones/test_zone.toml').read_text())
        self.assertEqual(sum(slot['count'] for slot in zone['spawn_slots'] if slot['template'] == 'keltir'), 1)


if __name__ == '__main__':
    unittest.main()
