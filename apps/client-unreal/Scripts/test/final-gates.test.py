#!/usr/bin/env python3
"""Focused final-gate regressions; only Bash/Python fakes, no live services or builds."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), SCRIPTS / name)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


gates = module('sim-gates.py')
ci = module('run-sim-ci.py')
fixtures = module('test/fake-artifacts.py')


class FinalGatesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        binary = self.root / 'bin'
        binary.mkdir()
        (binary / 'curl').write_text('#!/bin/sh\nexit 0\n')
        (binary / 'curl').chmod(0o755)
        self.env = {**os.environ, 'PATH': str(binary) + ':' + os.environ['PATH'],
                    'SIM_BOT_BIN': str(SCRIPTS / 'test/fake-bot.sh'), 'SIM_SKIP_BUILD': '1',
                    'SIM_SAVED_DIR': str(self.root / 'saved'),
                    'SIM_REPLAY_CMD': 'bash ' + str(SCRIPTS / 'test/fake-replay.sh'),
                    'SIM_TRACE_CMD': 'bash ' + str(SCRIPTS / 'test/fake-trace.sh'),
                    'SIM_COVERAGE_CMD': 'bash ' + str(SCRIPTS / 'test/fake-replay.sh'),
                    'PYTHONDONTWRITEBYTECODE': '1'}
        for key in ('SIM_REQUIRE_OWNED_API', 'SIM_PIPELINE_VERDICT', 'SIM_REQUIRE_TRANSITION_COVERAGE'):
            self.env.pop(key, None)
        self.scenarios = self.root / 'scenarios'
        self.scenarios.mkdir()
        self.quarantine = self.root / 'quarantine.json'
        self.quarantine.write_text('{"schema_version":1,"scenarios":[]}')
        self.baseline = self.root / 'baseline.json'
        self.baseline.write_text(json.dumps(fixtures.contract_fixture(0)))

    def scenario(self, name, result='pass', amount=1):
        path = self.scenarios / f'{name}.nfs'
        path.write_text(f'fake result {result}\nfake count {amount}\n')
        return path

    def run_wrapper(self, name, result, **env):
        path = self.scenario(name, result)
        folder = self.root / name
        run = subprocess.run(['bash', str(SCRIPTS / 'run-sim.sh'), '--artifacts', str(folder), str(path)],
                             env={**self.env, **env}, text=True, capture_output=True, timeout=15)
        verdict = json.loads((folder / 'pipeline-verdict.json').read_text())
        return run, folder, verdict

    def run_ci(self, **env):
        run = subprocess.run(['python3', str(SCRIPTS / 'run-sim-ci.py'), '--scenarios', str(self.scenarios),
                              '--quarantine', str(self.quarantine), '--artifacts', str(self.root / 'suite'),
                              '--contract-baseline', str(self.baseline)],
                             env={**self.env, **env}, text=True, capture_output=True, timeout=30)
        summary = json.loads((self.root / 'suite/summary.json').read_text())
        return run, summary

    def allow(self, names):
        self.quarantine.write_text(json.dumps({'schema_version': 1, 'scenarios': [
            {'scenario': name, 'owner': 'team', 'reason': 'seeded', 'issue': 'NF-1', 'expires': '2099-01-01'}
            for name in names]}))

    def test_completed_bot_only_verdict_uses_actual_failure_types(self):
        for result, kind in [('fail', 'bot_assertion'), ('expectation', 'bot_expectation'), ('scenario', 'bot_scenario')]:
            run, folder, verdict = self.run_wrapper(result, result)
            self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
            self.assertTrue(verdict['completed'])
            self.assertEqual(verdict['infrastructure_failures'], [])
            self.assertEqual(verdict['scenarios'][0]['failure_kinds'], [kind])
            self.assertTrue(ci.quarantine_verdict(folder, [result], 1, [result], {result: {}}))

    def test_every_seeded_infrastructure_failure_blocks_quarantine(self):
        for result in ('crash', 'noreport', 'unreadable', 'inconsistent', 'norecording', 'diverge',
                       'ensure', 'unknown', 'greenexit', 'redreport', 'nocontract', 'nocoverage',
                       'badcoverage', 'coveragefail'):
            with self.subTest(result=result):
                run, folder, verdict = self.run_wrapper(result, result)
                self.assertNotEqual(run.returncode, 0, run.stdout + run.stderr)
                self.assertEqual(verdict['exit_code'], run.returncode)
                self.assertTrue(verdict['infrastructure_failures'])
                self.assertFalse(ci.quarantine_verdict(folder, [result], run.returncode, [result], {result: {}}))

        for name, command in (('trace-command-failed', 'false'), ('trace-output-missing', 'true')):
            run, folder, verdict = self.run_wrapper(name, 'fail', SIM_TRACE_CMD=command)
            self.assertEqual(run.returncode, 1)
            self.assertTrue(verdict['infrastructure_failures'])
            self.assertFalse(ci.quarantine_verdict(folder, [name], 1, [name], {name: {}}))

    def test_reused_single_and_multi_destinations_reject_before_bot(self):
        for script, names in [('run-sim.sh', ['solo']), ('run-sim-multi.sh', ['pair-a', 'pair-b'])]:
            folder = self.root / script
            old = folder / names[0]
            old.mkdir(parents=True)
            (old / '.stale').write_text('retained')
            paths = [self.scenario(n) for n in names]
            run = subprocess.run(['bash', str(SCRIPTS / script), '--artifacts', str(folder), *map(str, paths)],
                                 env=self.env, text=True, capture_output=True, timeout=10)
            self.assertEqual(run.returncode, 2, run.stdout + run.stderr)
            self.assertEqual((old / '.stale').read_text(), 'retained')
            self.assertFalse((self.root / 'saved').exists())
            self.assertFalse((folder / 'pipeline-verdict.json').exists())

    def test_suite_aggregate_counts_sum_once_and_summary_names_gaps(self):
        for name, amount in [('solo', 2), ('pair-a', 3), ('pair-b', 5)]:
            self.scenario(name, amount=amount)
        run, summary = self.run_ci()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        root = self.root / 'suite'
        coverage = json.loads((root / 'coverage.transitions.json').read_text())
        self.assertEqual(sum(r['count'] for r in coverage['npc_intentions']), 10)
        self.assertEqual(coverage['life_incarnations']['npc:1->2'], 10)
        self.assertEqual(coverage['deaths']['npc'], 10)
        self.assertEqual(coverage['intent_rejected']['Invalid'], 10)
        self.assertEqual(len(coverage['sources']), 2)
        contract = json.loads((root / 'coverage.contract.json').read_text())
        self.assertEqual(contract['counts']['intents']['move_to'], '10')
        self.assertEqual(contract['client_frames'], '10')
        self.assertEqual(len(contract['sources']), 3)
        self.assertTrue(all(p.endswith('.coverage.contract.json') for p in contract['sources']))
        self.assertIn('1/10 reachable pairs covered', run.stdout)
        self.assertIn('never seen:', run.stdout)
        self.assertIn('unreachable:', run.stdout)
        self.assertFalse(summary['coverage']['infrastructure_errors'])
        for unit in ('solo', 'pair'):
            self.assertTrue(json.loads((root / unit / 'pipeline-verdict.json').read_text())['completed'])

    def test_identical_group_recordings_are_counted_once(self):
        self.scenario('pair-a', amount=3)
        self.scenario('pair-b', amount=3)
        run, summary = self.run_ci()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        unit = json.loads((self.root / 'suite/pair/coverage.transitions.json').read_text())
        self.assertEqual(len(unit['sources']), 1)
        self.assertEqual(sum(r['count'] for r in unit['npc_intentions']), 3)
        self.assertEqual(summary['coverage']['transitions']['deaths']['npc'], 3)
        # Runtime contracts count observed frames from both clients, even with one shared NFR.
        self.assertEqual(summary['coverage']['contract']['client_frames'], '6')

    def test_shared_group_recording_is_covered_once_with_a_trace(self):
        self.scenario('pair-a', 'sharedrecording')
        self.scenario('pair-b', 'sharedrecording')
        run, summary = self.run_ci()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        unit = self.root / 'suite/pair'
        coverage = json.loads((unit / 'coverage.transitions.json').read_text())
        self.assertEqual(len(coverage['sources']), 2)
        self.assertEqual(coverage['deaths']['npc'], 3)
        self.assertTrue((unit / 'shared.coverage.transitions.json').is_file())
        self.assertTrue((unit / 'shared.trace.html').is_file())

    def test_missing_transition_command_or_output_is_blocking_in_ci(self):
        for command, result in [('false', 'pass'), (self.env['SIM_COVERAGE_CMD'], 'nocoverage')]:
            self.scenario('solo', result)
            run, summary = self.run_ci(SIM_COVERAGE_CMD=command)
            self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
            self.assertTrue(summary['coverage']['infrastructure_errors'])
            self.assertEqual(summary['units'][0]['status'], 'FAIL')
            import shutil
            shutil.rmtree(self.root / 'suite')

    def test_missing_baseline_blocks_even_passing_suite(self):
        self.scenario('solo')
        self.baseline.unlink()
        run, summary = self.run_ci()
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertEqual(summary['units'][0]['status'], 'PASS')
        self.assertTrue(summary['coverage']['infrastructure_errors'])
        self.assertIn('baseline.json', (self.root / 'suite/coverage.contract.log').read_text())

    def test_historical_contract_presence_loss_blocks_suite(self):
        self.scenario('solo')
        baseline = fixtures.contract_fixture(0)
        baseline['counts']['intents']['respawn'] = '1'
        self.baseline.write_text(json.dumps(baseline))
        run, summary = self.run_ci()
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertTrue(summary['coverage']['infrastructure_errors'])
        self.assertIn('contract regression: intents.respawn', run.stdout)

    def test_quarantined_pair_requires_only_failing_role_exemption(self):
        self.scenario('pair-a', 'fail')
        self.scenario('pair-b')
        self.allow(['pair-a'])
        run, summary = self.run_ci()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(summary['units'][0]['status'], 'QUARANTINED')

    def test_trace_failure_overrides_quarantined_bot_failure(self):
        self.scenario('solo', 'fail')
        self.allow(['solo'])
        run, summary = self.run_ci(SIM_TRACE_CMD='false')
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertEqual(summary['units'][0]['status'], 'FAIL')

    def test_group_merge_failure_finalizes_infrastructure_and_blocks_quarantine(self):
        self.scenario('pair-a', 'fail')
        self.scenario('pair-b')
        self.allow(['pair-a'])
        python = self.root / 'bin/python3'
        python.write_text('#!/usr/bin/env bash\n'
                          'if [[ "$1" == */sim-junit.py && "$2" == merge ]]; then exit 1; fi\n'
                          'exec "$REAL_TEST_PYTHON" "$@"\n')
        python.chmod(0o755)
        run, summary = self.run_ci(REAL_TEST_PYTHON=sys.executable)
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertEqual(summary['units'][0]['status'], 'FAIL')
        verdict = json.loads((self.root / 'suite/pair/pipeline-verdict.json').read_text())
        self.assertTrue(verdict['completed'])
        self.assertEqual(verdict['exit_code'], 1)
        self.assertIn('group_junit_merge', verdict['infrastructure_failures'])
        self.assertFalse((self.root / 'suite/pair/group.xml').exists())

    def test_coverage_attempts_later_recordings_after_an_earlier_failure(self):
        dest = self.root / 'recordings'
        dest.mkdir()
        (dest / 'a.nfr').write_text('nocoverage 1')
        (dest / 'b.nfr').write_text('pass 2')
        run = subprocess.run(['bash', '-c', '. "$1"; sim_transition_coverage "$2" 1',
                              'test', str(SCRIPTS / 'sim-lib.sh'), str(dest)],
                             env=self.env, text=True, capture_output=True, timeout=10)
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertTrue((dest / 'b.coverage.transitions.json').exists())
        self.assertFalse((dest / 'coverage.transitions.json').exists())

    def test_semantic_transition_guards_reject_corrupt_counts_and_catalogues(self):
        first = self.root / 'first.json'
        second = self.root / 'second.json'
        value = fixtures.transition_fixture(2)
        first.write_text(json.dumps(value))
        mutations = [lambda d: d['npc_intentions'][0].update(count=1),
                     lambda d: d['npc_intentions'][1].update(count=-1),
                     lambda d: d['npc_intentions'][1].update(count=True),
                     lambda d: d['npc_intentions'][1].update(reachable='true'),
                     lambda d: d['npc_intentions'].pop(),
                     lambda d: d['life_incarnations'].update({'npc:2->2': 1}),
                     lambda d: d['deaths'].pop('player'),
                     lambda d: d['npc_intentions'][1].update(reachable=False, count=0)]
        for mutate in mutations:
            current = copy.deepcopy(value)
            mutate(current)
            second.write_text(json.dumps(current))
            with self.assertRaises(ValueError):
                gates.merge_transitions([first, second])
        with self.assertRaises(OSError):
            gates.merge_transitions([first, self.root / 'missing.json'])

    def test_forged_bot_verdict_cannot_classify_unknown_failure_type(self):
        run, folder, verdict = self.run_wrapper('unknown', 'unknown')
        verdict['infrastructure_failures'] = []
        verdict['scenarios'][0]['failure_kinds'] = ['bot_assertion']
        (folder / 'pipeline-verdict.json').write_text(json.dumps(verdict))
        self.assertFalse(ci.quarantine_verdict(folder, ['unknown'], 1, ['unknown'], {'unknown': {}}))


if __name__ == '__main__':
    unittest.main()
