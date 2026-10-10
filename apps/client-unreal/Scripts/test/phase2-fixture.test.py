#!/usr/bin/env python3
"""Phase2 isolation and normal wrapper integration using stub commands only."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / name)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


fixture = module('phase2-fixture.py')
ci = module('run-sim-ci.py')


class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.events = self.root / 'events'
        self.env = dict(os.environ, PATH=f'{self.bin}:{os.environ["PATH"]}', SIM_SKIP_BUILD='1',
                        SIM_SAVED_DIR=str(self.root / 'saved'), SIM_API_WAIT='3',
                        SIM_REPLAY_CMD=f'bash {SCRIPTS}/test/fake-replay.sh',
                        SIM_COVERAGE_CMD=f'bash {SCRIPTS}/test/fake-replay.sh',
                        SIM_TRACE_CMD=f'bash {SCRIPTS}/test/fake-trace.sh',
                        DATABASE_URL='postgres://dev:secret@localhost:5432/nightfall',
                        NATS_URL='nats://localhost:4222', COMPOSE_PROJECT_NAME='user-dev',
                        ZONE_SIM_FIXTURE='ambient-fixture', ZONE_FILE='/user/zone',
                        AUTH_DEV_TOKENS='0', NIGHTFALL_PHASE2_FIXTURE='0')
        self.env.pop('SIM_BOT_ARGS_JSON', None)
        self.env.pop('SIM_REQUIRE_OWNED_API', None)
        self.env.pop('SIM_API_URL', None)
        self.fake('docker', f'''echo "docker $*" >> "{self.events}"
if [[ "$*" == *" exec "* ]]; then
  cat > "{self.root}/psql-stdin"
  "{sys.executable}" -c 'import os,json; from pathlib import Path; Path("{self.root}/psql-env.json").write_text(json.dumps(dict(os.environ)))'
fi
[[ "$*" != *" down "* || "${{FAIL_DOWN:-0}}" != 1 ]]
''')
        # Stub ONLY the Linux listener probe. API stub has no network listeners; no live API.
        self.fake('python3', f'''if [[ "${{1:-}}" == - && "${{2:-}}" == http://* && "${{3:-}}" =~ ^(free|owned)$ ]]; then
  cat >/dev/null
  if [[ "${{FAIL_PROBE:-0}}" == 1 ]]; then echo occupied >&2; exit 1; fi
  exit 0
fi
exec "{sys.executable}" "$@"
''')
        self.fake('curl', 'exit 0\n')
        api = self.fake('api', f'''echo api >> "{self.events}"
"{sys.executable}" -c 'import os,json; from pathlib import Path; Path("{self.root}/api-env.json").write_text(json.dumps(dict(os.environ)))'
exec sleep 60
''')
        migrate = self.fake('migrate', f'''echo "migrate $*" >> "{self.events}"
"{sys.executable}" -c 'import os,json; from pathlib import Path; Path("{self.root}/migrate-env.json").write_text(json.dumps(dict(os.environ)))'
''')
        seed = self.root / 'seed.py'
        seed.write_text(f'''import argparse,json,os,subprocess
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--fixture',choices={sorted(fixture.PHASE2)!r},required=True)
p.add_argument('--owner-account',required=True)
p.add_argument('--observer-account')
a=p.parse_args()
assert os.environ['AUTH_DEV_TOKENS']=='1' and os.environ['NIGHTFALL_PHASE2_FIXTURE']=='1'
assert ('nf_phase2_fixture_' in os.environ['DATABASE_URL'])
assert bool(a.observer_account)==(a.fixture=='phase2-transfer-observer')
with Path({str(self.events)!r}).open('a') as f: f.write('seed '+json.dumps(vars(a))+'\\n')
if Path({str(self.root / 'fail-seed')!r}).exists(): raise SystemExit(1)
subprocess.run(['psql','-X','--set=ON_ERROR_STOP=1','--quiet'], input='stub seed input', text=True, env=dict(os.environ, PGDATABASE=os.environ['DATABASE_URL']),check=True)
missing=a.fixture=='phase2-transfer-missing-token'
seed={{'fixture_enabled':True,'fixture':a.fixture,'dry_run':False,
 'owner':{{'account_id':a.owner_account,'character_id':{fixture.OWNER!r},'level':20 if missing else 40,
          'class_id':0,'sex':'female','tokens':[0,0] if missing else [1,1],'milestone_claimed_mask':1 if missing else 3}},
 'position':[126,126],'transfers':[] if missing else [1,2]}}
if a.observer_account: seed['observer']={{'account_id':a.observer_account,'character_id':{fixture.OBSERVER!r},'level':1,'class_id':0,'sex':'male'}}
if Path({str(self.root / 'bad-seed-manifest')!r}).exists(): seed['owner']['level']=1
print(json.dumps(seed))
''')
        bot = self.fake('bot', f'''echo "bot $*" >> "{self.events}"
exec bash "{SCRIPTS}/test/fake-bot.sh" "$@"
''')
        self.env.update(SIM_API_BIN=str(api), SIM_MIGRATE_BIN=str(migrate),
                        SIM_PHASE2_SEEDER=str(seed), SIM_BOT_BIN=str(bot))

    def fake(self, name, body):
        path = self.bin / name
        path.write_text('#!/usr/bin/env bash\nset -eu\n' + body)
        path.chmod(0o755)
        return path

    def scenario(self, name, pack=None):
        path = self.root / (name + '.nfs')
        path.write_text((f'# fixture: {pack}\n' if pack else '# ordinary\n') + 'fake result pass\n')
        return path

    def run_wrapper(self, batch, multi=False, mode='start', **env):
        folder = self.root / ('art-' + str(len(list(self.root.glob('art-*')))))
        result = subprocess.run(['bash', str(SCRIPTS / ('run-sim-multi.sh' if multi else 'run-sim.sh')),
                                 '--api', mode, '--artifacts', str(folder), *map(str, batch)],
                                env=self.env | env, capture_output=True, text=True, timeout=30)
        return result, folder

    def lines(self):
        return self.events.read_text().splitlines() if self.events.exists() else []

    def test_headers_allowlist_roles_creation_units_and_legacy_catalogue(self):
        for pack in ('phase2-transfer', 'phase2-transfer-missing-token'):
            self.assertEqual(ci.fixture([self.scenario(pack, pack)]), pack)
        a = self.scenario('pair-a', 'phase2-transfer-observer')
        b = self.scenario('pair-b', 'phase2-transfer-observer')
        self.assertEqual(ci.fixture([b, a]), 'phase2-transfer-observer')
        creations = [self.scenario(f'creation-{n:02}') for n in (1, 2, 3)]
        self.assertEqual(len(ci.units(creations)), 3)
        for role in (a, b):
            with self.assertRaises(ValueError): ci.units([role])
            with self.assertRaises(ValueError): ci.fixture([role])
        for header in ('# fixture: nope\n', '# fixture: phase2-transfer\n# fixture: phase2-transfer\n',
                       '# fixture:phase2-transfer\n', '# fixture: phase2-transfer extra\n'):
            a.write_text(header)
            with self.assertRaises(ValueError): ci.fixture([a])
        a.write_text('# fixture: phase2-transfer-observer\n')
        b.write_text('# ordinary\n')
        with self.assertRaises(ValueError): ci.fixture([a, b])
        legacy = sorted(path for path in (ci.PROJECT / 'Scenarios').glob('*.nfs')
                        if path.name.startswith(('0b-', '1-')))

        self.assertEqual(len(legacy), 19)
        self.assertEqual(set(ci.fixture(batch) for batch in ci.units(legacy)),
                         {'default', 'phase1a-social-aggro', 'phase1a-late-entry'})

    def test_solo_order_seeder_forwarding_isolation_cleanup_and_arguments(self):
        for pack in ('phase2-transfer', 'phase2-transfer-missing-token'):
            scenario = self.scenario(pack, pack)
            result, artifacts = self.run_wrapper([scenario])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            folder = artifacts / scenario.stem / 'fixture'
            manifest = json.loads((folder / 'manifest.json').read_text())
            env = json.loads((folder / 'state.json').read_text())['env']
            self.assertTrue(manifest['database'].startswith('nf_phase2_fixture_'))
            self.assertTrue(manifest['project'].startswith('nightfall-sim-phase2-'))
            self.assertNotIn('secret', (folder / 'manifest.json').read_text())
            self.assertNotIn('user-dev', (folder / 'compose.json').read_text())
            migrate_env = json.loads((self.root / 'migrate-env.json').read_text())
            api_env = json.loads((self.root / 'api-env.json').read_text())
            for key in ('DATABASE_URL', 'NATS_URL', 'AUTH_DEV_TOKENS', 'NIGHTFALL_PHASE2_FIXTURE'):
                self.assertEqual(env[key], migrate_env[key])
                self.assertEqual(env[key], api_env[key])
            self.assertNotIn('ZONE_SIM_FIXTURE', env)
            self.assertNotIn('COMPOSE_PROJECT_NAME', env)
            self.assertEqual(env['AUTH_DEV_TOKENS'], '1')
            self.assertEqual(env['NIGHTFALL_PHASE2_FIXTURE'], '1')
            self.assertIn('127.0.0.1:', env['DATABASE_URL'])
            self.assertNotIn(':5432/', env['DATABASE_URL'])
            self.assertEqual(json.loads((folder / 'cleanup-status.json').read_text()), {'completed': True})
            config = json.loads((folder / 'compose.json').read_text())
            self.assertEqual(set(config['services']), {'postgres', 'nats'})
            for service in config['services'].values():
                self.assertTrue(all(port.startswith('127.0.0.1:') for port in service['ports']))
            for volume in config['volumes'].values(): self.assertEqual(volume, {})
            role = manifest['roles'][scenario.stem]
            self.assertEqual(role['character_id'], fixture.OWNER)
            self.assertTrue(any('-DevToken=test:' + role['account_id'] in line for line in self.lines() if line.startswith('bot ')))
            lines = self.lines()
            start = next(i for i, line in enumerate(lines) if manifest['project'] in line and ' up ' in line)
            unit = lines[start:]
            self.assertTrue(unit[0].startswith('docker '))
            self.assertTrue(unit[1].startswith('migrate up'))
            seed = json.loads(unit[2][5:])
            self.assertEqual(seed, {'fixture': pack, 'owner_account': role['account_id'], 'observer_account': None})
            self.assertIn(' exec ', unit[3])
            self.assertNotIn(env['DATABASE_URL'], unit[3])
            self.assertEqual(unit[4], 'api')
            self.assertTrue(unit[5].startswith('bot '))
            pg_env = json.loads((self.root / 'psql-env.json').read_text())
            self.assertEqual(pg_env['PGDATABASE'], manifest['database'])
            self.assertNotIn('://', pg_env['PGDATABASE'])
            self.assertEqual(pg_env['PGHOST'], '127.0.0.1')
            self.assertEqual(pg_env['PGPORT'], '5432')
            self.assertEqual(pg_env['PGUSER'], 'nightfall')
            self.assertEqual(pg_env['PGPASSWORD'], fixture.urlsplit(env['DATABASE_URL']).password)
            for component in ('PGHOST', 'PGPORT', 'PGDATABASE', 'PGUSER', 'PGPASSWORD'):
                self.assertIn('--env ' + component, unit[3])
            self.assertNotIn(pg_env['PGPASSWORD'], unit[3])
            self.assertEqual((self.root / 'psql-stdin').read_text(), 'stub seed input')
            self.assertIn(' down ', unit[-1])

    def test_observer_roles_distinct_even_reversed_input(self):
        a = self.scenario('observer-a', 'phase2-transfer-observer')
        b = self.scenario('observer-b', 'phase2-transfer-observer')
        result, artifacts = self.run_wrapper([b, a], multi=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        manifest = json.loads((artifacts / b.stem / 'fixture/manifest.json').read_text())
        roles = manifest['roles']
        self.assertNotEqual(roles[a.stem]['account_id'], roles[b.stem]['account_id'])
        self.assertEqual(roles[a.stem]['character_id'], fixture.OWNER)
        self.assertEqual(roles[b.stem]['character_id'], fixture.OBSERVER)
        seed = json.loads(next(line[5:] for line in self.lines() if line.startswith('seed ')))
        self.assertEqual(seed['owner_account'], roles[a.stem]['account_id'])
        self.assertEqual(seed['observer_account'], roles[b.stem]['account_id'])
        for name in roles:
            bot = next(line for line in self.lines() if line.startswith('bot ') and f'{name}.nfs' in line)
            self.assertIn('-DevToken=test:' + roles[name]['account_id'], bot)
            self.assertEqual(bot.count('-DevToken='), 1)

    def test_mixed_sequence_resets_phase2_and_does_not_contaminate_normal_bot(self):
        batch = [self.scenario('transfer', 'phase2-transfer'), self.scenario('ordinary'),
                 self.scenario('missing', 'phase2-transfer-missing-token')]
        result, artifacts = self.run_wrapper(batch)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        manifests = [json.loads((artifacts / s.stem / 'fixture/manifest.json').read_text()) for s in (batch[0], batch[2])]
        self.assertNotEqual(manifests[0]['project'], manifests[1]['project'])
        self.assertNotEqual(manifests[0]['roles']['transfer']['account_id'], manifests[1]['roles']['missing']['account_id'])
        lines = self.lines()
        ordinary = next(i for i, line in enumerate(lines) if line.startswith('bot ') and 'ordinary.nfs' in line)
        self.assertNotIn('-DevToken=', lines[ordinary])
        self.assertTrue(any(' down ' in line for line in lines[:ordinary]))
        self.assertTrue(any(' up ' in line for line in lines[ordinary + 1:]))

    def test_attach_conflicts_old_seeder_and_seed_failure_refuse_api(self):
        scenario = self.scenario('solo', 'phase2-transfer')
        for kwargs in ({'mode': 'attach'}, {'SIM_API_URL': 'http://127.0.0.1:31000'},
                       {'SIM_BOT_ARGS_JSON': '["-DevToken=test:foreign"]'},
                       {'SIM_BOT_ARGS_JSON': '["-NfGrpc=localhost:50051"]'},
                       {'SIM_MIGRATE_BIN': '/missing'}):
            result, _ = self.run_wrapper([scenario], **kwargs)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse(self.events.exists())
        old = self.root / 'old.py'
        old.write_text('print("pair-only seeder")\n')
        result, _ = self.run_wrapper([scenario], SIM_PHASE2_SEEDER=str(old))
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.events.exists())
        (self.root / 'fail-seed').touch()
        result, artifacts = self.run_wrapper([scenario])
        self.assertEqual(result.returncode, 2)
        self.assertNotIn('api', self.lines())
        self.assertFalse(any(line.startswith('bot ') for line in self.lines()))
        self.assertIn(' down ', self.lines()[-1])
        self.assertFalse((artifacts / scenario.stem / 'fixture/seed-complete').exists())
        (self.root / 'fail-seed').unlink()
        (self.root / 'bad-seed-manifest').touch()
        result, _ = self.run_wrapper([scenario])
        self.assertEqual(result.returncode, 2)
        self.assertNotIn('api', self.lines())

    def test_occupied_endpoint_and_cleanup_failure_are_blocking(self):
        scenario = self.scenario('solo', 'phase2-transfer')
        result, _ = self.run_wrapper([scenario], FAIL_PROBE='1')
        self.assertEqual(result.returncode, 2)
        self.assertNotIn('api', self.lines())
        self.assertIn(' down ', self.lines()[-1])
        result, artifacts = self.run_wrapper([scenario], FAIL_DOWN='1')
        # Compose runs with a clean environment. Put the failure in the stub itself.
        self.assertEqual(result.returncode, 0)
        self.fake('docker', f'echo "docker $*" >> "{self.events}"\n[[ "$*" != *" down "* ]]\n')
        result, artifacts = self.run_wrapper([scenario])
        self.assertNotEqual(result.returncode, 0)
        verdict = json.loads((artifacts / 'pipeline-verdict.json').read_text())
        self.assertIn('fixture_cleanup', verdict['infrastructure_failures'])
        status = json.loads((artifacts / scenario.stem / 'fixture/cleanup-status.json').read_text())
        self.assertFalse(status['completed'])

    def test_infra_and_migration_failure_always_clean_owned_stack(self):
        scenario = self.scenario('solo', 'phase2-transfer')
        migrate = self.fake('bad-migrate', f'echo migrate-failed >> "{self.events}"\nexit 1\n')
        result, _ = self.run_wrapper([scenario], SIM_MIGRATE_BIN=str(migrate))
        self.assertEqual(result.returncode, 2)
        self.assertNotIn('api', self.lines())
        self.assertFalse(any(line.startswith('seed ') for line in self.lines()))
        self.assertIn(' down ', self.lines()[-1])
        self.events.unlink()
        self.fake('docker', f'echo "docker $*" >> "{self.events}"\n[[ "$*" != *" up "* ]]\n')
        result, _ = self.run_wrapper([scenario])
        self.assertEqual(result.returncode, 2)
        self.assertNotIn('api', self.lines())
        self.assertEqual(len(self.lines()), 3)  # up failure, retained logs, owned down
        self.assertIn(' down ', self.lines()[-1])

    def test_default_binary_build_is_stubbed_and_psql_guard_blocks_foreign_database(self):
        target = self.root / 'target/debug'
        target.mkdir(parents=True)
        shutil.copy(self.bin / 'api', target / 'nightfall-api')
        shutil.copy(self.bin / 'migrate', target / 'nightfall-migrate')
        self.fake('cargo', f"""echo "cargo $*" >> "{self.events}"
if [[ "$1" == metadata ]]; then echo '{{"target_directory":"{self.root}/target"}}'; fi
""")
        env = self.env.copy()
        env.pop('SIM_API_BIN')
        env.pop('SIM_MIGRATE_BIN')
        with patch.dict(self.env, env, clear=True):
            result, artifacts = self.run_wrapper([self.scenario('solo', 'phase2-transfer')])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('--bin nightfall-api --bin nightfall-migrate', self.lines()[0])
        folder = artifacts / 'solo/fixture'
        with patch.dict(os.environ, {'PGDATABASE':'postgres://user:secret@localhost:5432/nightfall'}), \
                patch.object(fixture.subprocess, 'run') as run:
            with self.assertRaises(ValueError): fixture.psql(folder, ['-X'])
            run.assert_not_called()
        with patch('sys.argv', ['fixture', 'exec-api', '--folder', str(folder)]), \
                patch.object(fixture.os, 'execve') as execute:
            (folder / 'seed-complete').unlink()
            with self.assertRaisesRegex(ValueError, 'completed before-admission seed'):
                fixture.main()
            execute.assert_not_called()
            (folder / 'seed-complete').touch()
            (target / 'nightfall-api').write_text('#!/bin/sh\nexit 1\n')
            with self.assertRaisesRegex(ValueError, 'binary changed'):
                fixture.main()
            execute.assert_not_called()
        state_path = folder / 'state.json'
        value = json.loads(state_path.read_text())
        uri = f"postgres://night%66all:pass%40word@127.0.0.1:{value['ports']['postgres']}/{value['database']}"
        value['env']['DATABASE_URL'] = uri
        state_path.write_text(json.dumps(value))
        with patch.dict(os.environ, {'PGDATABASE': uri}), patch.object(fixture.subprocess, 'run') as run:
            fixture.psql(folder, ['-X'])
            actual_env = run.call_args.kwargs['env']
            self.assertEqual(actual_env['PGUSER'], 'nightfall')
            self.assertEqual(actual_env['PGPASSWORD'], 'pass@word')
            self.assertEqual(actual_env['PGDATABASE'], value['database'])
            self.assertNotIn(uri, run.call_args.args[0])
            self.assertNotIn('pass@word', run.call_args.args[0])
        for bad in (uri.replace('pass%40word', 'pass%0Aword'), uri + '\n'):
            value['env']['DATABASE_URL'] = bad
            state_path.write_text(json.dumps(value))
            with patch.dict(os.environ, {'PGDATABASE': bad}), patch.object(fixture.subprocess, 'run') as run:
                with self.assertRaises(ValueError): fixture.psql(folder, ['-X'])
                run.assert_not_called()

    def test_ci_real_wrappers_discover_phase2_and_creation_units(self):
        scenarios = self.root / 'scenarios'
        scenarios.mkdir()
        batch = [self.scenario('transfer', 'phase2-transfer'),
                 self.scenario('observer-a', 'phase2-transfer-observer'),
                 self.scenario('observer-b', 'phase2-transfer-observer'),
                 *[self.scenario(f'creation-{n:02}') for n in (1, 2, 3)]]
        for path in batch: shutil.move(str(path), scenarios / path.name)
        quarantine = self.root / 'quarantine.json'
        quarantine.write_text('{"schema_version":1,"scenarios":[]}')
        artifacts = self.root / 'ci-real'
        baseline = self.root / 'baseline.json'
        baseline.write_text(json.dumps(module('test/fake-artifacts.py').contract_fixture(0)))
        result = subprocess.run([sys.executable, str(SCRIPTS / 'run-sim-ci.py'),
                                 '--fresh-stack', '--scenarios', str(scenarios),
                                 '--quarantine', str(quarantine), '--artifacts', str(artifacts),
                                 '--contract-baseline', str(baseline)],
                                env=self.env | {'COMPOSE_PROJECT_NAME': 'nightfall-sim-stub'},
                                capture_output=True, text=True, timeout=40)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        summary = json.loads((artifacts / 'summary.json').read_text())
        self.assertEqual([row['unit'] for row in summary['units']],
                         ['creation-01', 'creation-02', 'creation-03', 'observer', 'transfer'])
        self.assertTrue(all(row['status'] == 'PASS' for row in summary['units']))
        # These commands are stubs: only legacy units request the old root Compose path.
        self.assertEqual(self.lines().count('docker compose down --volumes --remove-orphans'), 3)
        self.assertEqual(len(list(artifacts.glob('*/*/fixture/cleanup-status.json'))), 2)

    def test_ci_retries_only_interrupted_owned_cleanup(self):
        scenarios = self.root / 'scenarios'
        scenarios.mkdir()
        shutil.move(str(self.scenario('solo', 'phase2-transfer')), scenarios / 'solo.nfs')
        quarantine = self.root / 'quarantine.json'
        quarantine.write_text('{"schema_version":1,"scenarios":[]}')
        args = ['ci', '--fresh-stack', '--scenarios', str(scenarios), '--quarantine', str(quarantine),
                '--artifacts', str(self.root / 'ci-timeout')]
        folders = []
        def clients(command, log, seconds, env):
            folder = Path(env['SIM_PIPELINE_VERDICT']).parent / 'solo/fixture'
            folder.mkdir(parents=True)
            (folder / 'state.json').write_text('{}')
            folders.append(folder)
            return 124
        with patch('sys.argv', args), patch.object(ci, 'run_clients', clients), \
                patch.object(ci.phase2, 'cleanup') as cleanup, patch.object(ci.subprocess, 'run') as run, \
                patch.object(ci, 'suite_coverage', return_value={'infrastructure_errors': [], 'summary_lines': []}):
            self.assertTrue(ci.main())
            cleanup.assert_called_once_with(folders[0])
            run.assert_not_called()

    def test_ci_requires_fresh_and_never_root_compose_for_phase2(self):
        scenarios = self.root / 'scenarios'
        scenarios.mkdir()
        shutil.move(str(self.scenario('solo', 'phase2-transfer')), scenarios / 'solo.nfs')
        quarantine = self.root / 'quarantine.json'
        quarantine.write_text('{"schema_version":1,"scenarios":[]}')
        args = ['ci', '--scenarios', str(scenarios), '--quarantine', str(quarantine),
                '--artifacts', str(self.root / 'ci-art')]
        with patch('sys.argv', args), patch.object(ci, 'run_clients') as run:
            with self.assertRaises(SystemExit): ci.main()
            run.assert_not_called()
        seen = []
        def clients(command, log, seconds, env):
            seen.append((command, env))
            return 2
        with patch('sys.argv', args + ['--fresh-stack']), patch.dict(os.environ, self.env, clear=True), \
                patch.object(ci, 'run_clients', clients), patch.object(ci.subprocess, 'run') as run, \
                patch.object(ci, 'suite_coverage', return_value={'infrastructure_errors': [], 'summary_lines': []}):
            self.assertTrue(ci.main())
            run.assert_not_called()
        self.assertIn('start', seen[0][0])
        self.assertEqual(seen[0][1]['SIM_REQUIRE_OWNED_API'], '1')
        self.assertNotIn('ZONE_SIM_FIXTURE', seen[0][1])


if __name__ == '__main__':
    unittest.main()
