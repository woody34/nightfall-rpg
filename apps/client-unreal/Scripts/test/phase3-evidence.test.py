#!/usr/bin/env python3
"""Native C14 consumer checks plus corruption regressions; no live-client claims."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
spec = importlib.util.spec_from_file_location('evidence', SCRIPTS / 'phase3-evidence.py')
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)
FIRST = '00000000-0000-0000-0000-000000000001'
SECOND = '00000000-0000-0000-0000-000000000002'
RECORDING = REPO / 'apps/api/fixtures/sessions/phase3-gameplay-v5.nfr'


class ReplayEvidence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # A missing/unbuildable producer is a failure, never a skipped native test.
        subprocess.run(['cargo', 'build', '--offline', '-p', 'nightfall-api', '--bin', 'nightfall-replay'],
                       cwd=REPO, check=True, capture_output=True, text=True)
        metadata = subprocess.run(['cargo', 'metadata', '--offline', '--no-deps', '--format-version', '1'],
                                  cwd=REPO, check=True, capture_output=True, text=True)
        cls.binary = Path(json.loads(metadata.stdout)['target_directory']) / 'debug/nightfall-replay'
        cls.native_tmp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.native_tmp.cleanup)
        cls.raw = Path(cls.native_tmp.name) / 'native.jsonl'
        cls.export(cls.raw)
        cls.native = evidence.read_jsonl(cls.raw)

    @classmethod
    def export(cls, out):
        subprocess.run([str(cls.binary), 'export', '--file', str(RECORDING), '--evidence-out', str(out)],
                       cwd=REPO, check=True, capture_output=True, text=True)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data = copy.deepcopy(self.native)

    def check(self, declaration, entity=FIRST, data=None):
        path = self.root / 'scenario.nfs'
        path.write_text('# replay-gate: ' + declaration + '\n')
        return evidence.evaluate(self.data if data is None else data, RECORDING, entity, evidence.requirements(path))

    def fact(self, kind, entity=None):
        return next(r for r in self.data[1:-1] if r['kind'] == kind and (entity is None or r['entity'] == entity))

    def test_exact_twelve_declarations_remain_outside_client_predicates(self):
        kinds = set()
        for path in (SCRIPTS.parent / 'Scenarios').rglob('3-*.nfs'):
            kinds.update(g['kind'] for g in evidence.requirements(path))
            for line in path.read_text().splitlines():
                if line.startswith(('nf.Expect ', 'nf.WaitFor ')):
                    self.assertNotIn(line.split()[1], evidence.FACTS, path.name)
        self.assertEqual(kinds, evidence.FACTS)

    def test_actual_export_is_deterministic_and_all_categories_are_retained(self):
        second = self.root / 'second.jsonl'
        self.export(second)
        self.assertEqual(second.read_bytes(), self.raw.read_bytes())
        manifest, facts = evidence.validate(self.data, RECORDING)
        self.assertEqual(manifest['recording_sha256'], hashlib.sha256(RECORDING.read_bytes()).hexdigest())
        self.assertEqual(self.data[-1]['facts'], len(facts))
        self.assertEqual([r['sequence'] for r in facts], list(range(len(facts))))
        self.assertIn('auto_attack_resolution', {r['kind'] for r in facts})
        self.assertIn('rng_draw', {r['kind'] for r in facts})

    def test_native_regen_counts_and_fraction_oracle_are_owner_and_posture_bound(self):
        for entity in (FIRST, SECOND):
            self.assertTrue(self.check('regen_pulses >= 3', entity)['passed'])
            self.assertTrue(self.check('regen_matches_oracle', entity)['passed'])
        self.assertFalse(self.check('regen_pulses >= 26')['passed'])
        self.assertFalse(self.check('moving_regen_pulses >= 1')['passed'])
        self.assertFalse(self.check('moving_regen_matches_oracle')['passed'])
        self.assertFalse(self.check('regen_matches_oracle', '00000000-0000-0000-0000-000000000099')['passed'])
        for field in ('hp', 'mp', 'cp', 'hp_granted', 'mp_granted', 'cp_granted'):
            self.data = copy.deepcopy(self.native)
            self.fact('regen_pulse', FIRST)['data']['actual'][field] += 1
            self.assertFalse(self.check('regen_matches_oracle')['passed'], field)
        self.data = copy.deepcopy(self.native)
        self.fact('regen_pulse', FIRST)['data']['actual']['fractions']['cp'] += 1
        self.assertFalse(self.check('regen_matches_oracle')['passed'])
        self.data = copy.deepcopy(self.native)
        self.fact('regen_pulse', FIRST)['data']['amounts']['hp_q'] += 1
        self.assertFalse(self.check('regen_pulses >= 3')['passed'])

    def test_same_tick_distinct_facts_are_valid_but_duplicate_pulses_do_not_count(self):
        self.assertTrue(self.check('skill_hate_outcome_coherent')['passed'])
        pulse = self.fact('regen_pulse', FIRST)
        position = pulse['sequence'] + 1
        self.data.insert(position, copy.deepcopy(pulse))
        for sequence, row in enumerate(self.data[1:-1]):
            row['sequence'] = sequence
        self.data[-1]['facts'] += 1
        self.assertFalse(self.check('regen_pulses >= 3')['passed'])

    def test_native_death_record_uses_recorded_loss_rules(self):
        self.assertTrue(self.check('death_record_matches_fixture', SECOND)['passed'])
        self.assertFalse(self.check('death_record_matches_fixture', FIRST)['passed'])
        for name in ('loss', 'pre_death_xp', 'incarnation'):
            self.data = copy.deepcopy(self.native)
            self.fact('death_record', SECOND)['data'][name] += 1
            self.assertFalse(self.check('death_record_matches_fixture', SECOND)['passed'], name)
        self.data = copy.deepcopy(self.native)
        self.fact('death_record', SECOND)['data']['penalty_roll'] = 0
        self.assertFalse(self.check('death_record_matches_fixture', SECOND)['passed'])

    def test_native_refund_requires_offer_independent_percent_and_exact_once_consumption(self):
        self.assertTrue(self.check('resurrect_refund_matches_oracle', SECOND)['passed'])
        self.assertFalse(self.check('resurrect_refund_matches_oracle', FIRST)['passed'])
        for name in ('xp_after', 'expected_xp_refund', 'incarnation_after'):
            self.data = copy.deepcopy(self.native)
            self.fact('resurrection_refund', SECOND)['data'][name] += 1
            self.assertFalse(self.check('resurrect_refund_matches_oracle', SECOND)['passed'], name)
        self.data = copy.deepcopy(self.native)
        self.fact('resurrection_offer', SECOND)['data']['wit_bonus'] = '2/1'
        self.assertFalse(self.check('resurrect_refund_matches_oracle', SECOND)['passed'])

    def test_native_hate_uses_npc_subject_source_owner_and_selection(self):
        result = self.check('skill_hate_outcome_coherent')
        self.assertTrue(result['passed'], result)
        self.assertEqual(len(result['gates'][0]['sequences']), 3)
        self.assertFalse(self.check('skill_hate_outcome_coherent', SECOND)['passed'])
        self.fact('hate_selection')['data']['most_hated']['entity'] = SECOND
        self.assertFalse(self.check('skill_hate_outcome_coherent')['passed'])
        self.data = copy.deepcopy(self.native)
        self.fact('hate_delta')['data']['amount'] += 1
        self.assertFalse(self.check('skill_hate_outcome_coherent')['passed'])

    def test_absent_native_transitions_and_unsupported_numeric_vectors_stay_red(self):
        absent = ('death_record_matches_before_disconnect', 'restore_did_not_reapply_hooks',
                  'moving_regen_pulses >= 1', 'moving_regen_matches_oracle',
                  'skill_effect_matches_oracle 1184', 'stat_matches_oracle active', 'stat_matches_oracle removed')
        for declaration in absent:
            for entity in (FIRST, SECOND):
                result = self.check(declaration, entity)
                self.assertFalse(result['passed'], declaration)
                self.assertEqual(result['gates'][0]['ticks'], [])
        numeric = self.check('skill_damage_matches_oracle 1177')
        self.assertFalse(numeric['passed'])
        self.assertEqual(len(numeric['gates'][0]['ticks']), 5)
        self.assertTrue(all('unsupported independent oracle inputs' in e for e in numeric['gates'][0]['errors']))
        # An exact-vector refusal is not repaired by a producer's guessed oracle index.
        self.fact('damage_formula')['data']['vector_index'] = 0
        with self.assertRaises(ValueError):
            self.check('skill_damage_matches_oracle 1177')

    def test_manifest_trailer_sequence_owner_scope_and_unknown_fields_fail_closed(self):
        cases = (
            lambda d: d[0].update(recording_sha256='a'*64),
            lambda d: d[0].update(schema_version=True),
            lambda d: d[0].update(scope='selected_session'),
            lambda d: d[0]['watermark'].update(epoch=d[0]['epoch']+1),
            lambda d: d[-1].update(facts=d[-1]['facts']+1),
            lambda d: d[-1].update(ticks=True),
            lambda d: d[-1].update(last_tick=d[-1]['last_tick']+1),
            lambda d: d.pop(),
            lambda d: d[1].update(sequence=1),
            lambda d: d[1].update(tick=d[0]['snapshot_tick']-1),
            lambda d: d[1].update(entity='bad-owner'),
            lambda d: d[1].update(kind='future_raw_fact'),
            lambda d: d[1].update(passed=True),
            lambda d: d[1]['data'].update(passed=True),
        )
        for mutation in cases:
            data = copy.deepcopy(self.native)
            mutation(data)
            with self.assertRaises((ValueError, TypeError)):
                self.check('regen_pulses >= 3', data=data)
        with self.assertRaises(ValueError):
            evidence.evaluate({'schema_version': 1, 'facts': []}, RECORDING, FIRST, [])

    def test_rng_slice_copies_are_checked_without_double_consumption(self):
        resolved = next(r for r in self.data if r['kind'] == 'formula_resolution' and r['data']['rng_draws'])
        for mutate in (lambda draws: draws.reverse(), lambda draws: draws.append(copy.deepcopy(draws[-1])),
                       lambda draws: draws[0].update(value=draws[0]['value']+1)):
            self.data = copy.deepcopy(self.native)
            row = self.data[resolved['sequence']+1]
            mutate(row['data']['rng_draws'])
            with self.assertRaises(ValueError):
                self.check('regen_pulses >= 3')
        self.data = copy.deepcopy(self.native)
        draw = self.fact('rng_draw')['data']
        draw['value'] = draw['bound']
        with self.assertRaises(ValueError):
            self.check('regen_pulses >= 3')
        for kind in ('formula_resolution', 'damage_formula'):
            self.data = copy.deepcopy(self.native)
            self.fact(kind)['data']['target']['incarnation'] = True
            with self.assertRaises(ValueError):
                self.check('regen_pulses >= 3')

    def test_jsonl_duplicates_nonexact_numbers_blank_lines_and_incomplete_tail_rejected(self):
        path = self.root / 'bad.jsonl'
        for text in ('{"kind":"manifest","kind":"complete"}\n', '{"bound":1.0}\n',
                     '{"value":NaN}\n', '\n', self.raw.read_text() + '{"kind":'):
            path.write_text(text)
            with self.assertRaises(ValueError):
                evidence.read_jsonl(path)

    def test_run_uses_real_export_and_fresh_unit_artifacts(self):
        scenario = self.root / 'native.nfs'
        scenario.write_text('# replay-gate: regen_matches_oracle\n')
        report = self.root / 'native.xml'
        report.write_text(f'<testsuite><properties><property name="own_entity_id" value="{FIRST}"/></properties></testsuite>')
        out = self.root / 'gates.json'
        self.assertEqual(evidence.run(scenario, RECORDING, report, out, str(self.binary)), 0)
        self.assertTrue(json.loads(out.read_text())['passed'])
        self.assertEqual(out.with_suffix('.raw.jsonl').read_bytes(), self.raw.read_bytes())
        with self.assertRaisesRegex(ValueError, 'fresh'):
            evidence.run(scenario, RECORDING, report, out, str(self.binary))
        scenario.write_text('# replay-gate: moving_regen_matches_oracle\n')
        failed = self.root / 'missing.json'
        self.assertEqual(evidence.run(scenario, RECORDING, report, failed, str(self.binary)), 1)
        self.assertFalse(json.loads(failed.read_text())['passed'])

    def test_real_export_flows_through_both_wrappers_per_unit_including_no_replay(self):
        # Only bot/JUnit/health are stubs. Recording replay, evidence export, gate math,
        # coverage and wrapper classification are real; this is not Unreal acceptance.
        native_bot = self.root / 'native-bot.py'
        native_bot.write_text('''#!/usr/bin/env python3
import os, shutil, sys, xml.etree.ElementTree as ET
from pathlib import Path
scenario = Path(next(a.split('=',1)[1] for a in sys.argv if a.startswith('-BotScenario=')))
scripts = Path(os.environ['NATIVE_SCRIPTS'])
sys.path.insert(0, str(scripts / 'test'))
import importlib.util
spec = importlib.util.spec_from_file_location('fake', scripts / 'test/fake-artifacts.py')
fake = importlib.util.module_from_spec(spec); spec.loader.exec_module(fake)
out = Path(os.environ['SIM_SAVED_DIR']); out.mkdir(parents=True,exist_ok=True)
fake.bot(out, scenario.stem, 'pass', 0, 1)
report = out / (scenario.stem + '.xml'); tree = ET.parse(report)
tree.find('.//property[@name="own_entity_id"]').set('value', os.environ['NATIVE_SECOND'] if scenario.stem.endswith('-b') else os.environ['NATIVE_FIRST'])
tree.write(report)
shutil.copyfile(os.environ['NATIVE_RECORDING'], out / (scenario.stem + '.nfr'))
''')
        native_bot.chmod(0o755)
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'curl').write_text('#!/bin/sh\nexit 0\n')
        (bin_dir / 'curl').chmod(0o755)
        replay_check = self.root / 'check.sh'
        replay_check.write_text('#!/bin/sh\nexec "$NATIVE_REPLAY" check --file "$1"\n')
        replay_check.chmod(0o755)
        env = dict(os.environ, PATH=str(bin_dir)+':'+os.environ['PATH'], SIM_SKIP_BUILD='1',
                   SIM_BOT_BIN=str(native_bot), SIM_SAVED_DIR=str(self.root / 'saved'),
                   SIM_REPLAY_BIN=str(self.binary), SIM_REPLAY_CMD=str(replay_check),
                   SIM_COVERAGE_CMD=str(self.binary), SIM_PHASE3_EVIDENCE_CMD=str(self.binary),
                   SIM_TRACE_CMD='bash ' + str(SCRIPTS / 'test/fake-trace.sh'),
                   NATIVE_REPLAY=str(self.binary), NATIVE_RECORDING=str(RECORDING),
                   NATIVE_SCRIPTS=str(SCRIPTS), NATIVE_FIRST=FIRST, NATIVE_SECOND=SECOND,
                   PYTHONDONTWRITEBYTECODE='1')
        for key in ('SIM_REQUIRE_OWNED_API', 'SIM_PIPELINE_VERDICT', 'SIM_REQUIRE_TRANSITION_COVERAGE'):
            env.pop(key, None)
        for multi in (False, True):
            for missing in (False, True):
                paths = [self.root / ('native-a.nfs' if multi else 'native.nfs')]
                paths[0].write_text('# replay-gate: ' + ('moving_regen_matches_oracle' if missing else 'regen_matches_oracle') + '\nnf.Within 30\n')
                if multi:
                    peer = self.root / 'native-b.nfs'
                    peer.write_text('# replay-gate: death_record_matches_fixture\nnf.Within 30\n')
                    paths.append(peer)
                out = self.root / f'artifacts-{multi}-{missing}'
                script = SCRIPTS / ('run-sim-multi.sh' if multi else 'run-sim.sh')
                command = ['bash', str(script), '--no-replay', '--artifacts', str(out), *map(str, paths)]
                result = subprocess.run(command, env=env, cwd=REPO, capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, int(missing), result.stdout+result.stderr)
                verdict = json.loads((out / 'pipeline-verdict.json').read_text())
                self.assertEqual(bool(verdict['infrastructure_failures']), missing, verdict)
                for path in paths:
                    folder = out / path.stem
                    self.assertEqual((folder / 'phase3-evidence.raw.jsonl').read_bytes(), self.raw.read_bytes())
                    result = json.loads((folder / 'phase3-evidence.json').read_text())
                    self.assertEqual(result['entity'], SECOND if path.stem.endswith('-b') else FIRST)
                    self.assertEqual(result['passed'], not(missing and path == paths[0]))
                if missing:
                    self.assertTrue(any('phase3_replay_evidence' in e for e in verdict['infrastructure_failures']))


if __name__ == '__main__':
    unittest.main()
