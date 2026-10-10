#!/usr/bin/env python3
"""Descriptor additions preserve historical observations without claiming new runtime coverage."""
import importlib.util
import json
from pathlib import Path
import re
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
SCENARIOS = SCRIPTS.parent / 'Scenarios'
REPO = SCRIPTS.parents[2]
spec = importlib.util.spec_from_file_location('contract', SCRIPTS / 'sim-contract.py')
contract = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contract)


class Phase3ContractTests(unittest.TestCase):
    def test_contract_baseline_additions_never_promote_historical_presence(self):
        before = json.loads((SCRIPTS / 'test/fixtures/phase2-contract-baseline.json').read_text())
        after = json.loads((SCENARIOS / 'coverage.contract.baseline.json').read_text())
        self.assertEqual(before['observed_scenarios'], after['observed_scenarios'])
        for group, entries in before['counts'].items():
            for name, count in entries.items(): self.assertEqual(after['counts'][group][name], count)
        for group, entries in before['wire_numbers'].items():
            for name, number in entries.items(): self.assertEqual(after['wire_numbers'][group][name], number)
        for group, entries in after['counts'].items():
            for name in set(entries) - set(before['counts'][group]): self.assertEqual(entries[name], '0')
        self.assertEqual(after['wire_numbers']['intents']['cast_skill'], 16)
        self.assertEqual(after['counts']['events']['skill_cast_result'], '0')
        self.assertEqual(after['counts']['reasons']['REJECT_REASON_PEACE_ZONE'], '0')

    def test_catalogue_matches_fixed_wire_tags(self):
        baseline = json.loads((SCENARIOS / 'coverage.contract.baseline.json').read_text())
        proto = (REPO / 'packages/proto/nightfall/v1/world.proto').read_text()
        for group, block in [('intents', 'intent'), ('events', 'event')]:
            body = re.search(r'oneof '+block+r' \{([^}]+)', proto)[1]
            fields = {name: int(number) for name, number in re.findall(r'\w+ (\w+) = (\d+);', body)}
            self.assertEqual(baseline['wire_numbers'][group], fields)
        body = re.search(r'enum RejectReason \{([^}]+)', proto)[1]
        reasons = {name: int(number) for name, number in re.findall(r'(REJECT_REASON_\w+) = (\d+);', body)}
        self.assertEqual(baseline['wire_numbers']['reasons'], reasons)

    def test_promotion_keeps_old_presence_numbers_and_uncovered_additions(self):
        before = contract.read(SCRIPTS / 'test/fixtures/phase2-contract-baseline.json')
        after = contract.read(SCENARIOS / 'coverage.contract.baseline.json')
        promoted = contract.promote(after, before)
        self.assertEqual(promoted['counts'], after['counts'])
        self.assertEqual(promoted['wire_numbers'], after['wire_numbers'])
        self.assertEqual(contract.regressions(promoted, before), [])
        partial = json.loads(json.dumps(after))
        del partial['counts']['events']['attack_result']
        del partial['wire_numbers']['events']['attack_result']
        restored = contract.promote(partial, before)
        self.assertEqual(restored['counts']['events']['attack_result'], '1')
        self.assertEqual(restored['wire_numbers']['events']['attack_result'], 4)
        partial['wire_numbers']['events']['attack_result'] = 99
        with self.assertRaises(ValueError): contract.promote(partial, before)


if __name__ == '__main__':
    unittest.main()
