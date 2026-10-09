#!/usr/bin/env python3
"""Seeded pipeline regressions, using only Python and Bash fakes."""
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sim_ci', SCRIPTS / 'run-sim-ci.py')
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class CorrectionsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.env = {**os.environ, 'PATH': f'{self.bin}:{os.environ["PATH"]}'}
        self.env.pop('SIM_REQUIRE_OWNED_API', None)

    def fake(self, name, body):
        file = self.bin / name
        file.write_text('#!/usr/bin/env bash\nset -eu\n' + body)
        file.chmod(0o755)
        return file

    def shell(self, code, **env):
        return subprocess.run(['bash', '-c', 'set -euo pipefail\n. "$1"\n' + code,
                               'test', str(SCRIPTS / 'sim-lib.sh')],
                              env={**self.env, **env}, text=True, capture_output=True, timeout=10)

    @contextlib.contextmanager
    def listener(self, healthy):
        # Fake HTTP and /proc rather than binding sockets in the restricted test sandbox.
        self.fake('curl', 'exit "${FAKE_HEALTH_EXIT:-0}"\n')
        self.proc_fixture()
        previous = dict(self.env)
        self.env.update(FAKE_HEALTH_EXIT='0' if healthy else '22', FAKE_LISTENER='1',
                        FAKE_OWNED_SOCKET='0')
        try:
            yield 'http://127.0.0.1:3000'
        finally:
            self.env = previous

    def proc_fixture(self):
        fake = self.root / 'python-fakes'
        fake.mkdir(exist_ok=True)
        (fake / 'sitecustomize.py').write_text('''import os
from pathlib import Path
import socket
original_read = Path.read_text
original_iter = Path.iterdir
original_link = Path.readlink
def read(self, *args, **kwargs):
    if str(self) in ('/proc/net/tcp', '/proc/net/tcp6'):
        row = '0: 0100007F:0BB8 00000000:0000 0A 0 0 0 0 0 12345\\n'
        return 'header\\n' + (row if os.environ.get('FAKE_LISTENER') == '1' else '')
    return original_read(self, *args, **kwargs)
def iterate(self):
    if str(self).startswith('/proc/') and self.name == 'fd':
        return iter([self / '999']) if os.environ.get('FAKE_OWNED_SOCKET') == '1' else iter([])
    return original_iter(self)
def link(self):
    if str(self).startswith('/proc/') and self.name == '999':
        return Path('socket:[12345]')
    return original_link(self)
Path.read_text = read
Path.iterdir = iterate
Path.readlink = link
socket.getaddrinfo = lambda *a, **k: [(socket.AF_INET, socket.SOCK_STREAM, 6, '', ('127.0.0.1', 3000))]
''')
        self.env['PYTHONPATH'] = str(fake)

    def test_owned_start_rejects_healthy_and_unhealthy_occupied_ports(self):
        marker = self.root / 'docker-called'
        self.fake('docker', f'touch "{marker}"\n')
        for healthy in (True, False):
            with self.listener(healthy) as url:
                result = self.shell('sim_api_prepare start', SIM_API_URL=url, SIM_REQUIRE_OWNED_API='1')
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn('occupied', result.stderr)
            self.assertFalse(marker.exists())

    def test_default_start_still_attaches_and_owned_attach_is_rejected(self):
        self.fake('docker', 'exit 99\n')
        with self.listener(True) as url:
            result = self.shell('sim_api_prepare start', SIM_API_URL=url)
            self.assertEqual(result.returncode, 0, result.stderr)
            result = self.shell('sim_api_prepare attach', SIM_API_URL=url, SIM_REQUIRE_OWNED_API='1')
            self.assertEqual(result.returncode, 2)

    def test_owned_listener_must_belong_to_launched_process_tree(self):
        with self.listener(True) as url:
            result = self.shell('SIM_STARTED_API_PID=$$; sim_api_endpoint_probe owned', SIM_API_URL=url)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('not owned', result.stderr)
        self.proc_fixture()
        self.fake('curl', 'exit 0\n')
        self.fake('docker', 'exit 0\n')
        self.fake('cargo', 'exec sleep 30\n')
        # The launched process's fd owns the seeded listener. A startup race that introduces
        # an unrelated listener is separately rejected even while the child stays alive.
        for owned in ('1', '0'):
            code = 'sim_api_endpoint_probe() { if [[ "$1" == free ]]; then return 0; fi; ' + \
                   'python3 - "$SIM_API_URL" owned "$SIM_STARTED_API_PID" < "$PROBE_SOURCE"; }; ' + \
                   'sim_api_prepare start; sim_api_cleanup'
            source = self.root / 'probe.py'
            library = (SCRIPTS / 'sim-lib.sh').read_text()
            source.write_text(library.split("<<'PY'\n", 1)[1].split('\nPY\n', 1)[0])
            result = self.shell(code, SIM_API_URL='http://127.0.0.1:3000',
                                SIM_REQUIRE_OWNED_API='1', SIM_API_WAIT='3',
                                FAKE_LISTENER='1', FAKE_OWNED_SOCKET=owned, PROBE_SOURCE=str(source))
            self.assertEqual(result.returncode, 0 if owned == '1' else 2, result.stderr)

    def ci_args(self):
        scenarios = self.root / 'scenarios'
        scenarios.mkdir()
        (scenarios / 'solo.nfs').write_text('# default fixture\n')
        quarantine = self.root / 'quarantine.json'
        quarantine.write_text('{"schema_version":1,"scenarios":[]}')
        return ['ci', '--scenarios', str(scenarios), '--quarantine', str(quarantine),
                '--artifacts', str(self.root / 'artifacts')]

    def test_seeded_stale_xml_and_recording_are_preserved_and_rejected(self):
        args = self.ci_args()
        old = self.root / 'artifacts/solo/solo'
        old.mkdir(parents=True)
        (old / 'solo.xml').write_text('<testsuite tests="1"><testcase/></testsuite>')
        (old / 'solo.nfr').write_bytes(b'old recording')
        with patch('sys.argv', args), patch.object(ci, 'run_clients') as run:
            with self.assertRaises(SystemExit) as error:
                ci.main()
            self.assertEqual(error.exception.code, 2)
            run.assert_not_called()
        self.assertEqual((old / 'solo.nfr').read_bytes(), b'old recording')

    def test_fresh_flag_overrides_inherited_opt_out(self):
        args = self.ci_args() + ['--fresh-stack']
        seen = []
        def clients(command, log, seconds, env):
            seen.append(env)
            return 2
        with patch('sys.argv', args), patch.dict(os.environ, {
                'COMPOSE_PROJECT_NAME': 'nightfall-sim-test', 'SIM_REQUIRE_OWNED_API': '0'}), \
                patch.object(ci, 'run_clients', clients), patch.object(ci.subprocess, 'run',
                    return_value=subprocess.CompletedProcess([], 0, stdout='')):
            self.assertTrue(ci.main())
        self.assertEqual(seen[0]['SIM_REQUIRE_OWNED_API'], '1')

    def verdict(self, folder, names, kinds=None, **changes):
        value = {'schema_version': 1, 'completed': True, 'exit_code': 1,
                 'infrastructure_failures': [], 'scenarios': [
                     {'scenario': name, 'failure_kinds': (kinds or {}).get(name, [])}
                     for name in names]}
        value.update(changes)
        (folder / 'pipeline-verdict.json').write_text(json.dumps(value))

    def failing_report(self, name):
        folder = self.root / name
        folder.mkdir(exist_ok=True)
        (folder / f'{name}.xml').write_text(
            '<testsuite tests="1" failures="1"><testcase name="nf.Expect">'
            '<failure type="bot_assertion"/></testcase></testsuite>')

    def test_quarantine_requires_completed_explicit_bot_only_verdict(self):
        self.failing_report('bad')
        allowed = {'bad': {}}
        check = lambda code=1: ci.quarantine_verdict(self.root, ['bad'], code, ['bad'], allowed)
        self.assertFalse(check())
        self.verdict(self.root, ['bad'], {'bad': ['bot_assertion']})
        self.assertTrue(check())
        for changes in ({'infrastructure_failures': ['group_merge']}, {'completed': False},
                        {'exit_code': 0}, {'schema_version': 99}, {'scenarios': []}):
            self.verdict(self.root, ['bad'], {'bad': ['bot_assertion']}, **changes)
            self.assertFalse(check(), changes)
        for kind in ('replay', 'trace', 'timeout', 'bot_crash', 'unrecognized'):
            self.verdict(self.root, ['bad'], {'bad': ['bot_assertion', kind]})
            self.assertFalse(check(), kind)
        self.verdict(self.root, ['bad'], {'bad': ['bot_assertion']})
        self.assertFalse(check(124))
        self.assertFalse(ci.quarantine_verdict(self.root, ['bad'], 1, ['bad'], {}))
        (self.root / 'pipeline-verdict.json').write_text('{broken')
        self.assertFalse(check())

    def test_group_merge_failure_is_blocking_despite_quarantined_role(self):
        args = self.ci_args()
        scenarios = self.root / 'scenarios'
        (scenarios / 'solo.nfs').unlink()
        for name in ('fight-a', 'fight-b'):
            (scenarios / f'{name}.nfs').write_text('# default fixture\n')
        (self.root / 'quarantine.json').write_text(json.dumps({'schema_version': 1, 'scenarios': [
            {'scenario': 'fight-a', 'owner': 'team', 'reason': 'seeded', 'issue': 'NF-1',
             'expires': '2099-10-10'}]}))
        def clients(command, log, seconds, env):
            folder = Path(env['SIM_PIPELINE_VERDICT']).parent
            for name, failure in (('fight-a', '<failure/>'), ('fight-b', '')):
                dest = folder / name
                dest.mkdir()
                (dest / f'{name}.xml').write_text(
                    f'<testsuite tests="1" failures="{int(bool(failure))}">'
                    f'<testcase name="nf.Expect">{failure}</testcase></testsuite>')
            self.verdict(folder, ['fight-a', 'fight-b'], {'fight-a': ['bot_assertion']})
            log.write('A wrapper merge operation failed with an arbitrary new message\n')
            return 1
        with patch('sys.argv', args), patch.object(ci, 'run_clients', clients):
            self.assertTrue(ci.main())
        summary = json.loads((self.root / 'artifacts/summary.json').read_text())
        self.assertEqual(summary['units'][0]['status'], 'FAIL')
        self.assertIn('group JUnit merge', summary['units'][0]['infrastructure_errors'][0])

    def test_provenance_excludes_unchanged_future_files_and_collects_rewritten_files(self):
        saved = self.root / 'saved'
        saved.mkdir()
        (saved / 'old.xml').write_text('stale')
        (saved / 'old.nfr').write_text('stale')
        (saved / 'rewritten.xml').write_text('old')
        # Both old artifacts have future mtimes and would satisfy plain find -newer.
        import time
        for name in ('old.xml', 'old.nfr'):
            os.utime(saved / name, ns=(time.time_ns() + 10**12, time.time_ns() + 10**12))
        dest = self.root / 'collected'
        result = self.shell('marker="$(sim_new_marker)"; '
                            'echo fresh > "$SIM_SAVED_DIR/rewritten.xml"; '
                            'echo new > "$SIM_SAVED_DIR/new.nfr"; '
                            'sim_collect "$marker" "$DEST"; rm "$marker"',
                            SIM_REQUIRE_FRESH_ARTIFACTS='1', SIM_SAVED_DIR=str(saved), DEST=str(dest))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual({p.name for p in dest.iterdir()}, {'rewritten.xml', 'new.nfr'})
        self.assertEqual((dest / 'rewritten.xml').read_text().strip(), 'fresh')

    def test_ci_runs_real_single_and_multi_wrappers_with_fakes(self):
        args = self.ci_args()
        scenarios = self.root / 'scenarios'
        (scenarios / 'solo.nfs').write_text('fake result pass\n')
        for name in ('pair-a', 'pair-b'):
            (scenarios / f'{name}.nfs').write_text('fake result pass\n')
        self.fake('curl', 'exit 0\n')
        fixture_spec = importlib.util.spec_from_file_location('fake_artifacts', SCRIPTS / 'test/fake-artifacts.py')
        fixtures = importlib.util.module_from_spec(fixture_spec)
        fixture_spec.loader.exec_module(fixtures)
        baseline = self.root / 'baseline.json'
        baseline.write_text(json.dumps(fixtures.contract_fixture(0)))
        args += ['--contract-baseline', str(baseline)]
        env = {**self.env, 'SIM_BOT_BIN': str(SCRIPTS / 'test/fake-bot.sh'),
               'SIM_REPLAY_CMD': 'bash ' + str(SCRIPTS / 'test/fake-replay.sh'),
               'SIM_TRACE_CMD': 'bash ' + str(SCRIPTS / 'test/fake-trace.sh'),
               'SIM_COVERAGE_CMD': 'bash ' + str(SCRIPTS / 'test/fake-replay.sh'),
               'SIM_SKIP_BUILD': '1', 'SIM_SAVED_DIR': str(self.root / 'saved')}
        result = subprocess.run(['python3', str(SCRIPTS / 'run-sim-ci.py'), *args[1:]],
                                env=env, text=True, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        summary = json.loads((self.root / 'artifacts/summary.json').read_text())
        self.assertEqual([row['status'] for row in summary['units']], ['PASS', 'PASS'])
        self.assertTrue((self.root / 'artifacts/pair/group.xml').is_file())
        self.assertTrue((self.root / 'artifacts/solo/solo/solo.nfr').is_file())

    def test_video_retry_cannot_replace_headless_failure(self):
        self.fake('curl', 'exit 0\n')
        self.fake('xvfb-run', 'shift 3; exec "$@"\n')
        self.fake('ffmpeg', 'touch "${@: -1}"\n')
        bot = self.fake('bot', '''for arg in "$@"; do
  if [[ "$arg" == -nullrhi ]]; then exec bash "$FAKE_HEADLESS" "$@"; fi
done
exec python3 - "$@" <<'INNER'
import os, pathlib, sys, time
arg = next(a for a in sys.argv if 'GameScreenshotSaveDirectory=' in a)
frames = pathlib.Path(arg.split('Path="')[1].split('"')[0])
saved = pathlib.Path(os.environ['SIM_SAVED_DIR'])
(saved / 'bad.xml').write_text('<testsuite tests="1" failures="0"><testcase name="retry"/></testsuite>')
(frames / 'MovieFrame1.png').write_bytes(b'frame')
time.sleep(.01)
INNER
''')
        scenario = self.root / 'bad.nfs'
        scenario.write_text('fake result fail\n')
        icd = self.root / 'icd'
        icd.touch()
        dest = self.root / 'artifacts'
        result = subprocess.run(['bash', str(SCRIPTS / 'run-sim.sh'), '--video',
                                 '--artifacts', str(dest), str(scenario)],
                                env={**self.env, 'SIM_VIDEO_ICD': str(icd), 'SIM_BOT_BIN': str(bot),
                                     'SIM_SKIP_BUILD': '1', 'SIM_SAVED_DIR': str(self.root / 'saved'),
                                     'SIM_REPLAY_CMD': 'bash ' + str(SCRIPTS / 'test/fake-replay.sh'),
                                     'SIM_TRACE_CMD': 'bash ' + str(SCRIPTS / 'test/fake-trace.sh'),
               'SIM_COVERAGE_CMD': 'bash ' + str(SCRIPTS / 'test/fake-replay.sh'),
                                     'SIM_REQUIRE_FRESH_ARTIFACTS': '1',
                                     'FAKE_HEADLESS': str(SCRIPTS / 'test/fake-bot.sh')},
                                text=True, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        import xml.etree.ElementTree as ET
        self.assertGreater(int(ET.parse(dest / 'bad/bad.xml').getroot().get('failures')), 0)
        self.assertEqual(ET.parse(dest / 'bad/video/bad.xml').getroot().get('failures'), '0')
        self.assertTrue((dest / 'bad/video/bad.mp4').is_file())

    def test_video_uses_wall_bounds_and_irregular_frame_mtimes(self):
        self.fake('xvfb-run', 'shift 3; exec "$@"\n')
        ffargs = self.root / 'ffargs'
        self.fake('ffmpeg', 'printf "%s\\n" "$@" > "$FAKE_FFARGS"\ntouch "${@: -1}"\n')
        bot = self.fake('bot', '''exec python3 - "$@" <<'INNER'
import pathlib, sys, time
arg = next(a for a in sys.argv if 'GameScreenshotSaveDirectory=' in a)
folder = pathlib.Path(arg.split('Path="')[1].split('"')[0])
time.sleep(.05)
(folder / 'MovieFrame0002.png').write_bytes(b'first')
time.sleep(.15)
(folder / 'MovieFrame0001.png').write_bytes(b'second')
time.sleep(.3)
(folder / 'MovieFrame0003.png').write_bytes(b'last')
time.sleep(.05)
sys.exit(1)
INNER
''')
        icd = self.root / 'icd'
        icd.touch()
        dest = self.root / "video space's"
        result = subprocess.run(['bash', str(SCRIPTS / 'sim-video.sh'), 'seeded.nfs', str(dest)],
                                env={**self.env, 'SIM_VIDEO_ICD': str(icd), 'SIM_BOT_BIN': str(bot),
                                     'SIM_SKIP_BUILD': '1', 'SIM_SAVED_DIR': str(self.root / 'saved'),
                                     'FAKE_FFARGS': str(ffargs)}, text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        timing = json.loads((dest / 'frame-timing.json').read_text())
        frames = timing['frames']
        self.assertEqual([Path(f['file']).name for f in frames],
                         ['MovieFrame0002.png', 'MovieFrame0001.png', 'MovieFrame0003.png'])
        self.assertAlmostEqual(frames[1]['duration_ns'] / 1e9, .3, delta=.06)
        self.assertEqual(sum(f['duration_ns'] for f in frames),
                         timing['capture_end_ns'] - timing['capture_start_ns'])
        self.assertGreater(sum(f['duration_ns'] for f in frames) / 1e9, .5)
        self.assertEqual((dest / 'retry-exit-code.txt').read_text().strip(), '1')
        args = ffargs.read_text().splitlines()
        self.assertNotIn('-r', args)
        self.assertIn('vfr', args)
        self.assertNotIn('0.066666667', (dest / 'frames.txt').read_text())
        # A second retry cannot consume the earlier frames or overwrite its diagnostic evidence.
        rerun = subprocess.run(['bash', str(SCRIPTS / 'sim-video.sh'), 'seeded.nfs', str(dest)],
                               env=self.env, text=True, capture_output=True, timeout=10)
        self.assertEqual(rerun.returncode, 2)
        self.assertIn('must be empty', rerun.stderr)


if __name__ == '__main__':
    unittest.main()
