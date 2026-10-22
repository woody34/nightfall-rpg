#!/usr/bin/env python3
"""phase4-contract-smoke.py with a fake nightfall-replay: the gate catches producer/consumer drift offline."""
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
FIXTURES = SCRIPTS / 'test/fixtures'


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_').replace('.py', ''), SCRIPTS / name)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


smoke = module('phase4-contract-smoke.py')

FAKE = '''#!/usr/bin/env bash
# fake nightfall-replay: check/coverage/export driven by FAKE_* and the fixture name
set -euo pipefail
cmd="$1"; shift
file=""; out=""
while (($#)); do case "$1" in --file) file="$2"; shift 2 ;; --out|--items-evidence-out) out="$2"; shift 2 ;; *) shift ;; esac; done
name="$(basename "$file" .nfr)"
case "$cmd" in
  check) [[ "${FAKE_DIVERGE:-}" != "$name" ]] || { echo "zone 1 epoch 1: divergence at tick 7"; exit 1; }; echo "zone 1 epoch 67: match."; ;;
  coverage) python3 - "$out" "${FAKE_COVERAGE}" "${FAKE_LABEL:-}" <<'PYC'
import json, sys
data = json.load(open(sys.argv[2]))
if sys.argv[3]: data['item_operations'][sys.argv[3]] = 1
json.dump(data, open(sys.argv[1], 'w'))
PYC
  ;;
  export) if [[ "$name" == t11-* || "$name" == t19-* ]]; then
            [[ "${FAKE_LIVE_EXPORTS:-0}" != 1 ]] || { echo ok > "$out"; exit 0; }
            echo "error: item evidence refuses partial/live captures" >&2; exit 2; fi
          src="${FAKE_EXPORTS}/$name.jsonl"; [[ -f "$src" ]] || { echo "error: no export" >&2; exit 2; }
          if [[ -n "${FAKE_HASH_SWAP:-}" && "$name" == "$FAKE_HASH_SWAP" ]]; then
            python3 - "$src" "$out" <<'PYS'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1])]
rows[0]['phase4_rules_hash'] = 'f' * 64
open(sys.argv[2], 'w').write(''.join(json.dumps(r, separators=(',', ':')) + '\\n' for r in rows))
PYS
          else cp "$src" "$out"; fi ;;
  *) exit 2 ;;
esac
'''


class ContractSmokeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.repo = root / 'repo'
        for folder in ('items', 'sessions', 'phase3-v6'):
            (self.repo / 'apps/api/fixtures' / folder).mkdir(parents=True)
        # The committed items fixtures and their pinned exports (phase4-t10) agree by recording digest.
        for name in ('draw-free-grant', 'offline-grant-only'):
            shutil.copy(REPO / f'apps/api/fixtures/items/{name}.nfr', self.repo / f'apps/api/fixtures/items/{name}.nfr')
        (self.repo / 'apps/api/fixtures/sessions/t11-v6-fake.nfr').write_bytes(b'live capture')
        self.binary = root / 'nightfall-replay'
        self.binary.write_text(FAKE)
        self.binary.chmod(0o755)
        # The sibling nightfall-api attests the embedded bundle (both hashes) and exits, as the real one does.
        self.api = root / 'nightfall-api'
        # derive_rules_hash starts the API with a clean environment, so the fake is steered by files beside it.
        self.api.write_text('#!/usr/bin/env bash\nhere="$(dirname "$0")"\n[[ ! -f "$here/api-silent" ]] || exit 1\n'
                            'hash="$(cat "$here/api-hash" 2>/dev/null || echo bfee2e7d9fb73f53706c7a4afc19f5d095f4d630ca90e9a5747895c0e5472aad)"\n'
                            'echo "{\\"timestamp\\":\\"2026-10-22T00:00:00Z\\",\\"level\\":\\"INFO\\",\\"fields\\":{\\"message\\":\\"Phase3 bootstrap bundle validated with the Phase4 (V6) item extension\\",'
                            '\\"phase4_item_rules_hash\\":\\"sha256:$hash\\",\\"phase4_skill_rules_hash\\":\\"sha256:5c2378c9eab24f26fdef161c4965dc3567fc05eefe04186cdf095d84cad7f6d1\\"}}"\nexit 1\n')
        self.api.chmod(0o755)
        self.out = root / 'out'
        self.env = {**os.environ, 'FAKE_COVERAGE': str(FIXTURES / 'phase4-contract-smoke/coverage.schema4.json'),
                    'FAKE_EXPORTS': str(FIXTURES / 'phase4-t10'), 'PYTHONDONTWRITEBYTECODE': '1'}

    def run_smoke(self, *extra, **env):
        return subprocess.run(['python3', str(SCRIPTS / 'phase4-contract-smoke.py'), '--replay', str(self.binary),
                               '--repo', str(self.repo), '--out', str(self.out), *extra],
                              env={**self.env, **env}, text=True, capture_output=True, timeout=120)

    def report(self):
        return json.loads((self.out / 'contract-smoke.json').read_text())

    def test_agreeing_producer_passes_and_records_every_fixture(self):
        result = self.run_smoke()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = self.report()
        self.assertTrue(report['passed'])
        self.assertEqual([(Path(r['fixture']).name, r['kind']) for r in report['fixtures']],
                         [('draw-free-grant.nfr', 'closed'), ('offline-grant-only.nfr', 'closed'), ('t11-v6-fake.nfr', 'live')])
        self.assertEqual(report['rules_hash'], 'bfee2e7d9fb73f53706c7a4afc19f5d095f4d630ca90e9a5747895c0e5472aad')
        self.assertEqual(report['fixtures'][0]['labels']['item_operations'], ['grant'])
        self.assertEqual(result.stdout.count('PASS '), 3)
        self.assertTrue((self.out / 'draw-free-grant.phase4.raw.jsonl').is_file())
        self.assertTrue((self.out / 'rules-hash.api.log').is_file())
        self.assertFalse(self.run_smoke('--rules-hash', 'a' * 64).returncode == 0)
        self.assertEqual(self.run_smoke('--rules-hash', report['rules_hash']).returncode, 0)

    def test_pinned_hash_comes_from_the_api_attestation_or_the_flag_never_the_export(self):
        (self.api.parent / 'api-hash').write_text('e' * 64 + '\n')  # a stale embedded bundle fails every closed fixture
        result = self.run_smoke()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertTrue(all(any('pinned hash' in p for p in r['problems']) for r in self.report()['fixtures'] if r['kind'] == 'closed'))
        (self.api.parent / 'api-hash').unlink()
        (self.api.parent / 'api-silent').touch()
        self.assertEqual(self.run_smoke().returncode, 2)  # no attestation, no --rules-hash
        (self.api.parent / 'api-silent').unlink()
        self.assertEqual(self.run_smoke('--api', str(self.out / 'missing-api')).returncode, 2)
        self.assertEqual(self.run_smoke('--rules-hash', 'xyz').returncode, 2)
        self.assertEqual(self.run_smoke('--api', str(self.out / 'missing-api'), '--rules-hash',
                                        'bfee2e7d9fb73f53706c7a4afc19f5d095f4d630ca90e9a5747895c0e5472aad').returncode, 0)

    def test_divergence_unknown_label_and_live_export_fail_the_gate(self):
        result = self.run_smoke(FAKE_DIVERGE='offline-grant-only')
        self.assertEqual(result.returncode, 1)
        self.assertIn('check exit 1', result.stdout)
        self.assertEqual([bool(r['problems']) for r in self.report()['fixtures']], [False, True, False])
        result = self.run_smoke(FAKE_LABEL='teleport')
        self.assertEqual(result.returncode, 1)
        self.assertTrue(all(any('coverage' in p for p in r['problems']) for r in self.report()['fixtures']))
        result = self.run_smoke(FAKE_LIVE_EXPORTS='1')
        self.assertEqual(result.returncode, 1)
        self.assertIn('expected refusal', result.stdout)

    def test_rules_hash_disagreement_and_corrupt_export_fail(self):
        result = self.run_smoke(FAKE_HASH_SWAP='draw-free-grant')
        self.assertEqual(result.returncode, 1)
        self.assertTrue(any('pinned hash' in p for p in self.report()['fixtures'][0]['problems']))
        (self.repo / 'apps/api/fixtures/items/offline-grant-only.nfr').write_bytes(b'changed recording')
        result = self.run_smoke()
        self.assertEqual(result.returncode, 1)
        self.assertTrue(any('items evidence contract' in p for p in self.report()['fixtures'][1]['problems']))

    def test_no_closed_fixtures_is_a_usage_error(self):
        for path in (self.repo / 'apps/api/fixtures/items').glob('*.nfr'):
            path.unlink()
        self.assertEqual(self.run_smoke().returncode, 2)


if __name__ == '__main__':
    unittest.main()
