#!/usr/bin/env python3
"""Validate the guarded seeder's own contract; no live API or progression mutation."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib
import unittest
from urllib.parse import urlparse, urlunparse
from uuid import uuid4

ROOT = Path(__file__).resolve().parents[3]
SEEDER = ROOT / 'infra/scripts/seed-phase2-transfer-fixture.py'
ACCOUNT = '01970000-0000-7000-8000-000000000001'
OTHER = '01970000-0000-7000-8000-000000000002'
XP = {r['level']: r['xp'] for r in tomllib.loads(
    (ROOT / 'packages/data/tables/experience.toml').read_text())['to_level']}


class SeederContractTests(unittest.TestCase):
    def invoke(self, fixture='phase2-token-level20', extra=(), **overrides):
        env = dict(os.environ, AUTH_DEV_TOKENS='1', NIGHTFALL_PHASE2_FIXTURE='1',
                   DATABASE_URL='postgres://fixture:unused@127.0.0.1:26432/nf_phase2_fixture_unit')
        env.update(overrides)
        return subprocess.run([sys.executable, str(SEEDER), '--fixture', fixture,
                               '--owner-account', ACCOUNT, '--dry-run', *extra],
                              env=env, text=True, capture_output=True, timeout=5)

    def manifest(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout.splitlines()[-1])

    def test_true_crossing_profiles_use_authoritative_xp_and_sparse_typed_balances(self):
        for fixture, level, tokens, mask, threshold_level, target, raw, award in [
            ('phase2-token-level20', 19, [0, 0], 0, 20, 20, 10235, 10746),
            ('phase2-token-level40', 39, [1, 0], 1, 40, 40, 62751, 65888),
            ('phase2-token-jump19-40', 19, [0, 0], 0, 20, 40, 13892446, 14587068),
        ]:
            with self.subTest(fixture=fixture):
                result = self.invoke(fixture)
                seed = self.manifest(result)
                self.assertEqual(seed['owner'], {
                    'account_id': ACCOUNT,
                    'character_id': '01970000-0000-7000-8000-000000000020',
                    'level': level, 'class_id': 0, 'sex': 'female',
                    'tokens': tokens, 'milestone_claimed_mask': mask,
                    'race': 'human', 'xp': XP[threshold_level] - 1, 'hp': None, 'mp': None,
                    'cp': 0, 'sp': 0, 'position': [126, 126],
                })
                self.assertEqual(seed['crossing'], {
                    'level': target, 'threshold_xp': XP[target], 'xp_margin': 1,
                    'seed_threshold_xp': XP[threshold_level], 'npc_raw_xp': raw,
                    'kill_xp': award, 'expected_level': target,
                    'expected_xp': XP[threshold_level] - 1 + award})
                self.assertLess(seed['owner']['xp'], XP[target])
                self.assertGreaterEqual(seed['owner']['xp'] + award, XP[target])
                self.assertLess(seed['owner']['xp'] + award, XP[target + 1])
                self.assertIn(f'126,126,{XP[threshold_level] - 1},NULL,NULL', result.stdout)
                self.assertEqual(seed['transfers'], [])

    def test_backfill_profile_has_no_pre_granted_tokens(self):
        result = self.invoke('phase2-token-backfill')
        seed = self.manifest(result)
        self.assertEqual((seed['owner']['level'], seed['owner']['xp']), (40, XP[40]))
        self.assertEqual(seed['owner']['tokens'], [0, 0])
        self.assertEqual(seed['owner']['milestone_claimed_mask'], 0)
        self.assertNotIn('crossing', seed)
        self.assertIn(f'126,126,{XP[40]},NULL,NULL', result.stdout)

    def test_existing_positive_and_claimed_zero_contracts_remain_unchanged(self):
        for fixture, level, tokens, mask, transfers in [
            ('phase2-transfer', 40, [1, 1], 3, [1, 2]),
            ('phase2-transfer-observer', 40, [1, 1], 3, [1, 2]),
            ('phase2-transfer-missing-token', 20, [0, 0], 1, []),
        ]:
            extra = ('--observer-account', OTHER) if fixture.endswith('observer') else ()
            seed = self.manifest(self.invoke(fixture, extra))
            self.assertEqual(seed['owner'], {
                'account_id': ACCOUNT,
                'character_id': '01970000-0000-7000-8000-000000000020',
                'level': level, 'class_id': 0, 'sex': 'female',
                'tokens': tokens, 'milestone_claimed_mask': mask,
            })
            self.assertEqual(seed['transfers'], transfers)
            self.assertEqual('observer' in seed, fixture.endswith('observer'))

    def test_guards_refuse_unsafe_endpoints_and_missing_explicit_opt_in(self):
        for override in [
            {'AUTH_DEV_TOKENS': '0'}, {'NIGHTFALL_PHASE2_FIXTURE': '0'},
            {'DATABASE_URL': 'postgres://user:secret@localhost:5432/nf_phase2_fixture_unit'},
            {'DATABASE_URL': 'postgres://user:secret@localhost/nf_phase2_fixture_unit'},
            {'DATABASE_URL': 'postgres://user:secret@localhost:26432/nightfall'},
            {'DATABASE_URL': 'postgres://user:secret@localhost:26432/nf_phase2_fixture_unit?options=bad'},
            {'DATABASE_URL': 'postgres://user:secret@localhost:26432/nf_phase2_fixture_unit#fragment'},
            {'DATABASE_URL': 'mysql://user:secret@localhost:26432/nf_phase2_fixture_unit'},
        ]:
            with self.subTest(override=list(override)):
                result = self.invoke(**override)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('BEGIN;', result.stdout)
                self.assertNotIn('secret', result.stdout + result.stderr)

    def test_observer_identity_and_pre_admission_guards_are_retained(self):
        self.assertNotEqual(self.invoke('phase2-transfer-observer').returncode, 0)
        self.assertNotEqual(self.invoke('phase2-transfer-observer',
                                       ('--observer-account', ACCOUNT)).returncode, 0)
        result = self.invoke()
        self.manifest(result)
        for table in ['characters', 'account_sessions', 'play_tickets', 'zone_epochs',
                      'zone_snapshots', 'idempotency_keys']:
            self.assertIn(f'EXISTS (SELECT 1 FROM {table})', result.stdout)
        self.assertIn('pg_stat_activity', result.stdout)
        self.assertIn('ACCESS EXCLUSIVE MODE', result.stdout)
        self.assertIn('ON_ERROR_STOP=1', SEEDER.read_text())
        self.assertIn('INSERT INTO character_class_slots', result.stdout)
        self.assertIn("'racial.adaptable',1", result.stdout)


@unittest.skipUnless(os.environ.get('NIGHTFALL_PHASE2_SEEDER_PG_TESTS') == '1',
                     'requires explicit owned real Postgres opt-in')
class SeederPostgresTests(unittest.TestCase):
    def setUp(self):
        self.base = os.environ['DATABASE_URL']
        parsed = urlparse(self.base)
        self.assertEqual(parsed.hostname, '127.0.0.1')
        self.assertNotIn(parsed.port, (None, 5432))
        self.assertTrue(parsed.path.startswith('/nf_phase2_'))
        self.database = 'nf_phase2_fixture_test_' + uuid4().hex
        self.url = urlunparse(parsed._replace(path='/' + self.database))
        self.psql('CREATE DATABASE ' + self.database, self.base)
        self.addCleanup(self.psql, 'DROP DATABASE ' + self.database + ' WITH (FORCE)', self.base)
        migrations = ROOT / 'apps/api/src/infrastructure/postgres/migrations'
        for migration in sorted(migrations.glob('*.sql')):
            self.psql(migration.read_text())

    def psql(self, sql, url=None, check=True):
        return subprocess.run(['psql', '-X', '--set=ON_ERROR_STOP=1', '--quiet', '-tA'],
                              input=sql, text=True, capture_output=True,
                              env=dict(os.environ, PGDATABASE=url or self.url),
                              check=check, timeout=10)

    def seed(self, fixture='phase2-token-level20'):
        args = [sys.executable, str(SEEDER), '--fixture', fixture,
                '--owner-account', ACCOUNT]
        if fixture == 'phase2-transfer-observer':
            args += ['--observer-account', OTHER]
        return subprocess.run(args, env=dict(os.environ, AUTH_DEV_TOKENS='1',
                              NIGHTFALL_PHASE2_FIXTURE='1', DATABASE_URL=self.url),
                              text=True, capture_output=True, timeout=10)

    def test_real_postgres_profiles_persist_exact_manifest_and_retry_refuses(self):
        for profile, level, xp, tokens, mask in [
            ('phase2-token-level20', 19, XP[20]-1, [0, 0], 0),
            ('phase2-token-level40', 39, XP[40]-1, [1, 0], 1),
            ('phase2-token-backfill', 40, XP[40], [0, 0], 0),
            ('phase2-token-jump19-40', 19, XP[20]-1, [0, 0], 0),
            ('phase2-transfer-missing-token', 20, XP[20], [0, 0], 1),
            ('phase2-transfer-observer', 40, XP[40], [1, 1], 3),
        ]:
            with self.subTest(profile=profile):
                result = self.seed(profile)
                self.assertEqual(result.returncode, 0, result.stderr)
                seed = json.loads(result.stdout)
                row = json.loads(self.psql("""SELECT json_build_object(
                    'race',race,'level',level,'xp',xp,'hp',hp,'mp',mp,'cp',cp,'sp',sp,
                    'class_id',current_class_id,'base_class_id',base_class_id,
                    'sex',sex,'mask',milestone_claimed_mask,'tokens',
                    json_build_array(token_tier_1_count,token_tier_2_count),
                    'position',json_build_array(pos_x,pos_y))
                    FROM characters WHERE name='ClassOwner'""").stdout)
                self.assertEqual(row, {'race':'human','level':level,'xp':xp,'hp':None,
                    'mp':None,'cp':0,'sp':0,'class_id':0,'base_class_id':0,'sex':'female',
                    'mask':mask,'tokens':tokens,'position':[126,126]})
                self.assertEqual(seed['owner']['tokens'], row['tokens'])
                slots = self.psql("SELECT level||','||exp||','||sp FROM character_class_slots WHERE character_id='01970000-0000-7000-8000-000000000020'").stdout.strip()
                self.assertEqual(slots, f'{level},{xp},0')
                self.assertEqual(self.psql('SELECT count(*) FROM idempotency_keys').stdout.strip(), '0')
                self.assertEqual(self.psql('SELECT count(*) FROM zone_epochs').stdout.strip(), '0')
                retry = self.seed(profile)
                self.assertNotEqual(retry.returncode, 0)
                self.assertIn('fresh empty database', retry.stderr)
                self.assertEqual(self.psql('SELECT count(*) FROM characters').stdout.strip(),
                                 '2' if profile.endswith('observer') else '1')
                self.psql('TRUNCATE characters CASCADE; DELETE FROM accounts;')

    def test_real_postgres_old_session_or_other_active_connection_refuses_without_write(self):
        self.psql(f"INSERT INTO accounts(id) VALUES ('{ACCOUNT}'); INSERT INTO account_sessions(account_id,generation) VALUES ('{ACCOUNT}',1)")
        result = self.seed()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('fresh empty database', result.stderr)
        self.assertEqual(self.psql('SELECT count(*) FROM characters').stdout.strip(), '0')
        self.psql('DELETE FROM account_sessions; DELETE FROM accounts;')
        # A second session simulates API startup before admission. Wait for the
        # pg_sleep query to be visible instead of guessing a startup sleep.
        busy = subprocess.Popen(['psql','-X','--quiet'], stdin=subprocess.PIPE,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                text=True, env=dict(os.environ, PGDATABASE=self.url))
        try:
            busy.stdin.write('SELECT pg_sleep(10);')
            busy.stdin.close()
            for _ in range(100):
                active = self.psql("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()").stdout.strip()
                if active != '0':
                    break
            self.assertNotEqual(active, '0')
            result = self.seed()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('fresh empty database', result.stderr)
            self.assertEqual(self.psql('SELECT count(*) FROM characters').stdout.strip(), '0')
        finally:
            self.psql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()")
            busy.wait(timeout=5)


if __name__ == '__main__':
    unittest.main()
