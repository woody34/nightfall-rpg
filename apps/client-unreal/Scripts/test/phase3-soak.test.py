#!/usr/bin/env python3
"""Casting preparation/acceptance with synthetic reports; no native/live evidence."""
import copy
import hashlib
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fixtures = load('soak_fixtures', SCRIPTS / 'soak-fixtures.py')
reporting = load('soak_report', SCRIPTS / 'soak-report.py')
gates = load('soak_gates', SCRIPTS / 'sim-gates.py')
isolated = load('fixture_budget', SCRIPTS / 'phase2-fixture.py')
seed = load('soak_seed', REPO / 'infra/scripts/seed-phase3-soak.py')


class CastingSoakTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name)

    def test_eight_canonical_seeded_roles_cast_and_tokens_are_private(self):
        fixtures.prepare(REPO, self.out, 8, casting=True)
        manifest = json.loads((self.out / 'roles.json').read_text())
        roles = seed.validated_roles(manifest)
        self.assertEqual(len(roles), 8)
        self.assertEqual(len({r['account_id'] for r in roles.values()}), 8)
        self.assertEqual(len({r['character_id'] for r in roles.values()}), 8)
        for role in manifest['roles']:
            path = Path(role['scenario'])
            text = path.read_text()
            self.assertTrue(text.startswith('# covers: 3 '))
            self.assertIn('nf.Cast 1177', text)
            self.assertIn('nf.Cast 1068 self', text)
            self.assertIn('nf.WaitFor in_world 15\nnf.WaitFor learned_rank', text)
            self.assertNotIn('nf.Attack', text)
            self.assertNotIn('damage_numbers', text)
            self.assertNotIn('own_mp_cost_matches', text)
            waits = sum(int(line.split()[-1]) for line in text.splitlines() if line.startswith('nf.WaitFor '))
            self.assertGreaterEqual(isolated.scenario_budget([path]), waits + 60)
            token = Path(role['token_file'])
            self.assertEqual(token.stat().st_mode & 0o777, 0o600)
            self.assertEqual(token.read_text().strip(), 'test:' + role['seed']['account_id'])
            self.assertEqual(role['seed']['level'], 7)
        sql = seed.seed.seed_sql(roles)
        self.assertEqual(sql.count('BEGIN;'), 1)
        self.assertIn('before API startup and admission', sql)
        self.assertEqual(sql.count("'l2.skill.1068'"), 8)
        self.assertEqual(sql.count("'l2.skill.1177'"), 8)
        for mutate in (
            lambda d: d['roles'][0]['seed'].update(sp=999),
            lambda d: d['roles'][1]['seed'].update(account_id=d['roles'][0]['seed']['account_id']),
            lambda d: d['roles'].reverse(),
            lambda d: d.update(casting=False),
        ):
            changed = copy.deepcopy(manifest)
            mutate(changed)
            with self.assertRaises(ValueError): seed.validated_roles(changed)

    def test_database_guards_and_libpq_overrides_cannot_redirect_seeding(self):
        valid = 'postgres://user:pass@127.0.0.1:27432/nf_phase3_fixture_soak_test'
        seed.seed.guarded_database(valid)
        for url in (valid.replace('27432', '5432'), valid.replace('127.0.0.1', 'remote'),
                    valid + '?host=remote', valid.replace('nf_phase3_fixture_soak_test', 'nightfall')):
            with self.assertRaises(ValueError): seed.seed.guarded_database(url)
        with patch.dict(os.environ, {'PGHOSTADDR': 'remote', 'PGSERVICE': 'user-service', 'PGOPTIONS': '-c role=admin'}):
            env = seed.seed.psql_env(valid)
            self.assertEqual(env['PGHOST'], '127.0.0.1')
            self.assertEqual(env['PGPORT'], '27432')
            self.assertEqual(env['PGDATABASE'], 'nf_phase3_fixture_soak_test')
            for key in ('PGHOSTADDR', 'PGSERVICE', 'PGOPTIONS'): self.assertNotIn(key, env)

    def test_seeder_uses_owned_container_without_host_psql_and_propagates_failure(self):
        fixtures.prepare(REPO, self.out / 'fixtures', 1, casting=True)
        tools = self.out / 'bin'
        tools.mkdir()
        docker = tools / 'docker'
        docker.write_text(f'#!{sys.executable}\n'
                          'import json,os,sys\nfrom pathlib import Path\n'
                          f'out=Path({str(self.out)!r})\n'
                          '(out/"docker-call.json").write_text(json.dumps({"args":sys.argv[1:],"env":dict(os.environ)}))\n'
                          '(out/"sql").write_text(sys.stdin.read())\n'
                          'sys.exit(int(os.environ.get("FAKE_DOCKER_EXIT","0")))\n')
        docker.chmod(0o700)
        compose = self.out / 'compose.json'
        config = {'services': {'postgres': {'environment': {
            'POSTGRES_DB': 'nf_phase3_fixture_soak_test', 'POSTGRES_USER': 'user',
            'POSTGRES_PASSWORD': 'p@ss'}, 'ports': [{'target': 5432, 'published': '27432'}]}}}
        compose.write_text(json.dumps(config))
        (self.out / 'empty.env').touch()
        project = 'nightfall-phase3-harness-soak-123-456'
        command = [sys.executable, str(REPO / 'infra/scripts/seed-phase3-soak.py'),
                   '--manifest', str(self.out / 'fixtures/roles.json'),
                   '--compose-project', project, '--compose-file', str(compose)]
        env = dict(os.environ, PATH=str(tools), AUTH_DEV_TOKENS='1', NIGHTFALL_PHASE3_FIXTURE='1',
                   DATABASE_URL='postgres://user:p%40ss@localhost:27432/nf_phase3_fixture_soak_test',
                   PGHOSTADDR='remote', PGSERVICE='wrong', PGOPTIONS='-c role=admin')
        self.assertIsNone(shutil.which('psql', path=env['PATH']))
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['fixture_enabled'])
        call = json.loads((self.out / 'docker-call.json').read_text())
        expected = ['compose', '--env-file', str(self.out / 'empty.env'), '-p', project,
                    '-f', str(compose), 'exec', '-T']
        connection = {'PGHOST': '127.0.0.1', 'PGPORT': '5432',
                      'PGDATABASE': 'nf_phase3_fixture_soak_test', 'PGUSER': 'user', 'PGPASSWORD': 'p@ss',
                      'PGCONNECT_TIMEOUT': '3', 'PGAPPNAME': 'phase3_fixture_seed'}
        for key, value in connection.items():
            expected += ['--env', key]
            self.assertEqual(call['env'][key], value)
        self.assertEqual(call['args'], expected + ['postgres', 'psql', '-X', '--set=ON_ERROR_STOP=1', '--quiet'])
        for key in ('PGHOSTADDR', 'PGSERVICE', 'PGOPTIONS'):
            self.assertNotIn(key, call['env'])
        roles = seed.validated_roles(json.loads((self.out / 'fixtures/roles.json').read_text()))
        self.assertEqual((self.out / 'sql').read_text(), seed.seed.seed_sql(roles))
        failed = subprocess.run(command, env=env | {'FAKE_DOCKER_EXIT': '1'},
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(failed.returncode, 0)
        self.assertNotIn('fixture_enabled', failed.stdout)
        (self.out / 'docker-call.json').unlink()
        for bad in (command[:-4], command[:-3] + ['nightfall-development', '--compose-file', str(compose)]):
            rejected = subprocess.run(bad, env=env, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((self.out / 'docker-call.json').exists())
        for mutate in (
            lambda d: d['services']['postgres']['environment'].update(POSTGRES_DB='other'),
            lambda d: d['services']['postgres']['environment'].update(POSTGRES_PASSWORD='other'),
            lambda d: d['services']['postgres']['ports'][0].update(published='27433'),
        ):
            changed = copy.deepcopy(config)
            mutate(changed)
            compose.write_text(json.dumps(changed))
            rejected = subprocess.run(command, env=env, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((self.out / 'docker-call.json').exists())
        compose.write_text(json.dumps(config))
        rejected = subprocess.run(command, env=env | {'DATABASE_URL': env['DATABASE_URL'].replace('p%40ss', 'p%0Ass')},
                                  capture_output=True, text=True, timeout=10)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn('control characters', rejected.stderr)
        self.assertFalse((self.out / 'docker-call.json').exists())
        runner = (SCRIPTS / 'run-soak.sh').read_text()
        self.assertIn('--compose-project "$COMPOSE_PROJECT" --compose-file "$OUT/compose.json"', runner)

    def test_two_iterations_include_persisted_endpoint_and_arrival_tolerance(self):
        fixtures.prepare(REPO, self.out, 8, casting=True)
        manifest = json.loads((self.out / 'roles.json').read_text())
        for role in manifest['roles']:
            source = Path(role['scenario']).read_text()
            route = [line.split()[2:] for line in source.splitlines()
                     if line.startswith('nf.WaitFor own_at ')]
            position = (0, 0)
            for iteration in range(2):
                for x, y, tolerance, budget in route:
                    destination = (int(x), int(y))
                    # Worst start includes the previous own_at acceptance radius.
                    distance = math.dist(position, destination) + (1 if iteration or position != (0, 0) else 0)
                    self.assertGreaterEqual(int(budget), distance / 3.75 * 3)
                    position = destination
            waits = sum(int(line.split()[-1]) for line in source.splitlines()
                        if line.startswith('nf.WaitFor '))
            self.assertEqual(waits, 303)
            self.assertIn('nf.Within 360', source)
            self.assertGreaterEqual(360, waits + 30)
        self.assertIn('-SoakScenarioSeconds=360', (SCRIPTS / 'run-soak.sh').read_text())

    def synthetic_reports(self):
        fixture_dir = self.out / 'fixtures'
        fixtures.prepare(REPO, fixture_dir, 1, casting=True)
        role = self.out / 'client-01'
        role.mkdir()
        (role / '1-kill-one-monster.xml').write_text('''<testsuites><testsuite time="1201"><properties>
          <property name="loop_iterations" value="8"/><property name="exit_code" value="0"/>
          </properties><testcase name="cast"/></testsuite></testsuites>''')
        contract = json.loads((SCRIPTS.parent / 'Scenarios/coverage.contract.baseline.json').read_text())
        contract['counts']['intents']['cast_skill'] = '1'
        contract['counts']['events']['skill_cast_result'] = '2'
        contract['counts']['events']['effect_applied'] = '1'
        (role / '1-kill-one-monster.coverage.contract.json').write_text(json.dumps(contract))
        coverage = json.loads((SCRIPTS / 'test/fixtures/phase2-native-reconnect.transitions.json').read_text())
        coverage['schema_version'] = 3
        for table, (states, reachable) in gates.V3_REACHABLE.items():
            coverage[table] = [{'from': a, 'to': b, 'reachable': (a, b) in reachable,
                                'count': int((a, b) in reachable)} for a in sorted(states) for b in sorted(states)]
        for key in gates.V3_COUNTERS: coverage[key] = {}
        for key in gates.V3_SCALARS: coverage[key] = 0
        (self.out / 'soak.nfr').write_bytes(b'synthetic report-link fixture, never native evidence')
        coverage['recording_sha256'] = hashlib.sha256((self.out / 'soak.nfr').read_bytes()).hexdigest()
        (self.out / 'coverage.transitions.json').write_text(json.dumps(coverage))
        (self.out / 'replay-check.log').write_text('match. 100 ticks replayed (100 recorded)')
        return contract, coverage

    def run_report(self, require=True):
        responses = [io.BytesIO(json.dumps({'data': {'result': [{'value': [1300, '0.012']}]}}).encode()),
                     io.BytesIO(json.dumps({'data': {'result': [{'metric': {
                         '__name__': 'nightfall_combat_tick_duration_seconds_count'}, 'value': [1300, '100']}]}}).encode())]
        with patch('urllib.request.urlopen', side_effect=responses), patch('sys.stdout', new=io.StringIO()):
            return reporting.report(self.out, 1, 1200, 1, 1300, 0, require_casting=require)

    def test_casting_report_requires_each_role_and_matching_schema3_recording(self):
        contract, coverage = self.synthetic_reports()
        self.assertEqual(self.run_report(), 0)
        self.assertFalse(json.loads((self.out / 'report.json').read_text())['phase3_acceptance'])
        contract_path = self.out / 'client-01/1-kill-one-monster.coverage.contract.json'
        for group, name in [('intents', 'cast_skill'), ('events', 'effect_applied'), ('events', 'skill_cast_result')]:
            bad = copy.deepcopy(contract)
            bad['counts'][group][name] = '0'
            contract_path.write_text(json.dumps(bad))
            self.assertEqual(self.run_report(), 1)
        contract_path.write_text(json.dumps(contract))
        transition_path = self.out / 'coverage.transitions.json'
        transition_path.write_text(json.dumps(coverage | {'recording_sha256': '0' * 64}))
        self.assertEqual(self.run_report(), 1)
        transition_path.write_text((SCRIPTS / 'test/fixtures/phase2-native-reconnect.transitions.json').read_text())
        self.assertEqual(self.run_report(), 1)
        (self.out / 'fixtures/roles.json').unlink()
        self.assertEqual(self.run_report(), 1)
        self.assertEqual(self.run_report(require=False), 0)
        result = json.loads((self.out / 'report.json').read_text())
        self.assertEqual(result['workload'], 'legacy-attacks')
        self.assertFalse(result['phase3_acceptance'])

    def test_soak_hooks_need_only_documented_commands_without_ripgrep(self):
        project = self.out / 'project'
        scripts = project / 'Scripts'
        scripts.mkdir(parents=True)
        client = project / 'Build/Scripts'
        client.mkdir(parents=True)
        for name in ('run-soak.sh', 'sim-lib.sh'):
            shutil.copy(SCRIPTS / name, scripts / name)
        (client / 'NightfallSoak.cs').write_text('// no role hook')
        tools = self.out / 'bin'
        tools.mkdir()
        for name in ('bash', 'dirname', 'mkdir', 'find', 'grep'):
            (tools / name).symlink_to(shutil.which(name))
        env = dict(os.environ, PATH=str(tools))
        result = subprocess.run([str(tools / 'bash'), str(scripts / 'run-soak.sh'),
                                 '--artifacts', str(self.out / 'reports')],
                                env=env, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn('SoakRoleManifest support', result.stderr)
        self.assertNotIn('command not found', result.stderr)

    def test_missing_client_hook_fails_before_any_build_or_infrastructure(self):
        scripts = self.out / 'repo/apps/client-unreal/Scripts'
        scripts.mkdir(parents=True)
        for name in ('run-soak.sh', 'sim-lib.sh'): shutil.copyfile(SCRIPTS / name, scripts / name)
        client = scripts.parent / 'Build/Scripts'
        client.mkdir(parents=True)
        (client / 'NightfallSoak.cs').write_text('// No role support yet')
        result = subprocess.run(['bash', str(scripts / 'run-soak.sh'), '--artifacts', str(self.out / 'reports')],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn('SoakRoleManifest support', result.stderr)
        self.assertEqual(list((self.out / 'reports').iterdir()), [])
        (client / 'NightfallSoak.cs').write_text('// SoakRoleManifest implemented; old deadlines remain')
        result = subprocess.run(['bash', str(scripts / 'run-soak.sh'), '--artifacts', str(self.out / 'reports')],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn('SoakScenarioSeconds budget support', result.stderr)
        self.assertEqual(list((self.out / 'reports').iterdir()), [])


if __name__ == '__main__':
    unittest.main()
