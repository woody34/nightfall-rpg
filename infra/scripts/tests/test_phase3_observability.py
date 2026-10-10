"""Negative fixtures for Phase3 source/query/finite-label observability validation."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('observability', ROOT / 'scripts/validate_class_observability.py')
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


class Phase3Observability(unittest.TestCase):
    def setUp(self):
        self.data = json.loads((ROOT / 'infra/grafana/dashboards/nightfall-api.json').read_text())

    def test_exact_source_contract_passes(self):
        self.assertEqual(validator.validate_phase3_dashboard(self.data), [])

    def test_missing_panels_wrong_metrics_unbounded_labels_and_zero_fallback_fail(self):
        data = copy.deepcopy(self.data)
        data['panels'] = [p for p in data['panels'] if p['id'] != 28]
        self.assertTrue(validator.validate_phase3_dashboard(data))
        for query in ['sum(rate(nightfall_unknown_total[5m]))',
                      'sum by (skill_id) (rate(nightfall_skill_casts_total[5m]))',
                      validator.PHASE3_QUERIES[28] + ' or vector(0)']:
            with self.subTest(query=query):
                data = copy.deepcopy(self.data)
                next(p for p in data['panels'] if p['id'] == 28)['targets'][0]['expr'] = query
                self.assertTrue(validator.validate_phase3_dashboard(data))

    def test_descriptions_distinguish_rollout_rpc_replay_and_committed_purchase(self):
        for pid in (28, 29, 30, 33, 34):
            with self.subTest(panel=pid):
                data = copy.deepcopy(self.data)
                next(p for p in data['panels'] if p['id'] == pid)['description'] = 'All mutations are ready'
                self.assertTrue(validator.validate_phase3_dashboard(data))

    def test_label_instrument_and_recorder_drift_fail(self):
        mutations = [
            ('metrics.rs', 'nightfall_skill_casts', 'nightfall_skill_castz'),
            ('phase3.rs', 'Started => "started"', 'Started => "unbounded"'),
            ('phase3.rs', 'outcome: SkillCastOutcome', 'outcome: String'),
        ]
        for filename, old, new in mutations:
            with self.subTest(file=filename, change=old), tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / 'apps/api/src/infrastructure/telemetry'
                path.mkdir(parents=True)
                for file in ('metrics.rs', 'phase3.rs'):
                    source = (ROOT / 'apps/api/src/infrastructure/telemetry' / file).read_text()
                    if file == filename:
                        self.assertIn(old, source)
                        source = source.replace(old, new)
                    (path / file).write_text(source)
                with patch.object(validator, 'ROOT', Path(tmp)):
                    self.assertTrue(validator.validate_phase3_dashboard(self.data))


if __name__ == '__main__':
    unittest.main()
