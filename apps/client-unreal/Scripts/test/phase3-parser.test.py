#!/usr/bin/env python3
"""Exercise the fail-closed source audit; this does not attest scenario compatibility."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('parser_check', SCRIPTS / 'sim-parser-check.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class ParserAudit(unittest.TestCase):
    def test_registration_extraction_tracks_merged_client_and_ignores_comments(self):
        names, commands = checker.registry()
        for name in ('learned_rank', 'own_sp', 'own_karma', 'own_pk', 'own_pvp',
                     'cast_state', 'rejected', 'cast_damage_results', 'entity_effect_active'):
            self.assertIn(name, names)
        self.assertNotIn('last_reject', names)
        self.assertIn('nf.learn', commands)
        self.assertNotIn('invented', checker.clean_cpp('// RegisterFlag(TEXT("invented"))'))
        self.assertIn('"kept"', checker.clean_cpp('/* removed */ TEXT("kept")'))

    def test_each_scenario_is_audited_without_allowlisting_unknown_names(self):
        paths = sorted((SCRIPTS.parent / 'Scenarios').rglob('3-*.nfs'))
        for path in paths:
            expected = checker.validate([path])
            result = subprocess.run([sys.executable, str(SCRIPTS / 'sim-parser-check.py'), '--json', str(path)],
                                    capture_output=True, text=True, check=False)
            audit = json.loads(result.stdout)
            self.assertEqual(audit['failures'], expected, path.name)
            self.assertEqual(result.returncode, int(bool(expected)), path.name)
            self.assertEqual(audit['compatible'], not expected)
        # No list of expected missing names can turn an incompatibility into PASS.
        # Running the same CLI over all scenarios is the strict integration gate.

    def test_unknown_names_bad_command_args_and_unknown_projection_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'parser.nfs'
            for source in ('nf.Expect invented', 'nf.Invented', 'nf.Learn 1177',
                           'nf.Cast 1177 corpse', 'nf.AcceptResurrect latest maybe',
                           'nf.Expect own_mp_cost_matches 1177', 'nf.WaitFor connected 0'):
                path.write_text(source)
                self.assertTrue(checker.validate([path]), source)
            path.write_text('nf.Learn 1177 2 01970000-0000-7000-8000-000000000032\n'
                            'nf.WaitFor learned_rank 1177 == 2 15 # source allowance\n'
                            'nf.Expect own_sp == 9760\n')
            self.assertEqual(checker.validate([path]), [])

    def test_wrappers_refuse_unknown_predicates_before_build_or_api(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for multi in (False, True):
                paths = [root / name for name in (('blocked-a.nfs', 'blocked-b.nfs') if multi else ('blocked.nfs',))]
                for path in paths:
                    path.write_text('# fixture: phase3-trainer\nnf.Expect invented\nnf.Within 30\n')
                script = SCRIPTS / ('run-sim-multi.sh' if multi else 'run-sim.sh')
                result = subprocess.run(['bash', str(script), '--api', 'start', '--artifacts', str(root / str(multi)),
                                         *map(str, paths)], capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn('unregistered predicate invented', result.stderr)
                self.assertIn('registry mismatch', result.stderr)


if __name__ == '__main__':
    unittest.main()
