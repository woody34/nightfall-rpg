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
            ('combat.rs', 'record_skill_resolution', 'unwired_resolution'),
            ('combat.rs', 'baseline.is_none()', 'baseline.is_some()'),
            ('checkpoint.rs', 'metrics.skill_learned()', 'metrics.unwired_learning()'),
        ]
        sources = [
            'apps/api/src/infrastructure/telemetry/metrics.rs',
            'apps/api/src/infrastructure/telemetry/phase3.rs',
            'apps/api/src/infrastructure/telemetry/combat.rs',
            'apps/api/src/application/checkpoint.rs',
        ]
        for filename, old, new in mutations:
            with self.subTest(file=filename, change=old), tempfile.TemporaryDirectory() as tmp:
                for relative in sources:
                    source = (ROOT / relative).read_text()
                    path = Path(tmp) / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(source)
                with patch.object(validator, 'ROOT', Path(tmp)):
                    self.assertEqual(validator.validate_phase3_dashboard(self.data), [])
                    path = next(Path(tmp) / rel for rel in sources if Path(rel).name == filename)
                    source = path.read_text()
                    self.assertIn(old, source)
                    path.write_text(source.replace(old, new))
                    self.assertTrue(validator.validate_phase3_dashboard(self.data))


if __name__ == '__main__':
    unittest.main()
