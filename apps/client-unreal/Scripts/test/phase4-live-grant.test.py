#!/usr/bin/env python3
"""Opt-in live D112 provisioning test: owned ephemeral stack, real API binary, real GrantItem.

Runs only with NIGHTFALL_PHASE4_LIVE=1 and SIM_API_BIN/SIM_MIGRATE_BIN pointing at a
candidate build (`cargo build --release -p nightfall-api --bins`). It provisions the
phase4-equip pack (4-unequip-fists.nfs) on its own Compose project, starts the API on free
ports, grants the pack through GrantItem, and verifies the offline baseline and the
instance mapping. Always cleans up its own project.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.request

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('phase2_fixture_live', SCRIPTS / 'phase2-fixture.py')
fixture = importlib.util.module_from_spec(spec); spec.loader.exec_module(fixture)
SCENARIO = SCRIPTS.parent / 'Scenarios/4-unequip-fists.nfs'


@unittest.skipUnless(os.environ.get('NIGHTFALL_PHASE4_LIVE') == '1', 'opt-in live Phase4 provisioning (NIGHTFALL_PHASE4_LIVE=1)')
class LiveGrant(unittest.TestCase):
    def test_pack_is_granted_and_confirmed_offline(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp) / 'fixture'
            api = None
            try:
                fixture.provision(folder, [SCENARIO])
                value = fixture.state(folder)
                api = subprocess.Popen([sys.executable, str(SCRIPTS / 'phase2-fixture.py'), 'exec-api', '--folder', str(folder)],
                                       stdout=(folder / 'api.log').open('w'), stderr=subprocess.STDOUT)
                deadline = time.monotonic() + 180
                while True:
                    self.assertIsNone(api.poll(), 'API exited during startup')
                    try:
                        with urllib.request.urlopen(f"http://127.0.0.1:{value['ports']['http']}/health", timeout=2):
                            break
                    except OSError:
                        if time.monotonic() > deadline: self.fail('API did not become healthy')
                        time.sleep(1)
                fixture.grant(folder)
                manifest = json.loads((folder / 'grant-manifest.json').read_text())
                pack = fixture.phase4.profiles('phase4-equip')['owner']
                granted = manifest['granted']['owner']
                self.assertEqual(granted['confirmed'], 'GetInventory')
                self.assertEqual(set(granted['instance_map']), {str(r['instance_id']) for r in pack['inventory']})
                self.assertTrue(all(server >= 2**32 for server in granted['instance_map'].values()))
                # Rerun replays the same frozen receipts (deterministic keys), never grants twice.
                (folder / 'grant-complete').unlink(); (folder / 'grant-manifest.json').unlink()
                fixture.grant(folder)
                again = json.loads((folder / 'grant-manifest.json').read_text())['granted']['owner']
                self.assertEqual(again['instance_map'], granted['instance_map'])
                self.assertEqual(again['inventory_revision'], granted['inventory_revision'])
                copy = fixture.scenario_copy(folder, SCENARIO)
                self.assertEqual(copy.name, SCENARIO.name)
            finally:
                if api is not None:
                    api.terminate()
                    try: api.wait(timeout=20)
                    except subprocess.TimeoutExpired: api.kill()
                if (folder / 'state.json').is_file():
                    fixture.cleanup(folder)


if __name__ == '__main__':
    unittest.main()
