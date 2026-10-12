#!/usr/bin/env python3
"""Exercise offline selection and credential isolation through a fake editor."""
from pathlib import Path
import json
import os
import shutil
import subprocess
import tempfile
import unittest

RUNNER = Path(__file__).resolve().parents[1] / 'run-tests.sh'


class OfflineRunnerTests(unittest.TestCase):
    def test_offline_selection_and_no_credentials(self):
        for spelling in ('offline', '--offline'):
            with self.subTest(spelling=spelling), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                scripts = root / 'apps/client-unreal/Scripts'
                tests = scripts.parent / 'Source/Nightfall/Tests'
                scripts.mkdir(parents=True)
                tests.mkdir(parents=True)
                shutil.copyfile(RUNNER, scripts / 'run-tests.sh')
                tests.joinpath('Fixture.cpp').write_text('\n'.join(
                    f'IMPLEMENT_SIMPLE_AUTOMATION_TEST(T, "{name}", Flags);' for name in (
                        'Nightfall.Item.Reducer', 'Nightfall.Skill.StateFences',
                        'Nightfall.Content.Wiring', 'Nightfall.Content.VendorArt',
                        'Nightfall.Combat.EndToEnd', 'Nightfall.Skill.LiveAdmission',
                        'Nightfall.Net.SessionClient.Ping')))
                engine = root / 'engine/Engine/Binaries/Linux'
                engine.mkdir(parents=True)
                editor = engine / 'UnrealEditor'
                editor.write_text('#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\n')
                editor.chmod(0o755)
                env = dict(os.environ, UE_ROOT=str(root / 'engine'), CI='false',
                           NIGHTFALL_DEV_TOKEN='must-not-reach-editor', AUTH_DEV_TOKENS='1')
                env.pop('RUN_TESTS_EXTRA_ARGS', None)
                result = subprocess.run(['bash', str(scripts / 'run-tests.sh'), spelling],
                                        env=env, text=True, capture_output=True, check=True)
                args = json.loads(result.stdout)
                command = next(arg for arg in args if arg.startswith('-ExecCmds='))
                self.assertIn('^Nightfall.Item.Reducer$', command)
                self.assertIn('^Nightfall.Skill.StateFences$', command)
                self.assertIn('^Nightfall.Content.Wiring$', command)
                for excluded in ('VendorArt', 'EndToEnd', 'LiveAdmission', 'SessionClient.Ping'):
                    self.assertNotIn(excluded, command)
                self.assertFalse(any('DevToken' in arg or 'must-not-reach-editor' in arg for arg in args))
                filtered = subprocess.run(['bash', str(scripts / 'run-tests.sh'), spelling, 'Nightfall.Item'],
                                          env=env, text=True, capture_output=True, check=True)
                self.assertIn('Automation RunTests Nightfall.Item; Quit', filtered.stdout)
                conflict = subprocess.run(['bash', str(scripts / 'run-tests.sh'), spelling, '--require-live-api'],
                                          env=env, text=True, capture_output=True)
                self.assertEqual(conflict.returncode, 2)
                self.assertIn('modes conflict', conflict.stderr)


if __name__ == '__main__':
    unittest.main()
