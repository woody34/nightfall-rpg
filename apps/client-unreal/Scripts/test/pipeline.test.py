#!/usr/bin/env python3
"""Failure-policy tests without Unreal, Docker or a GitHub runner."""
from datetime import date
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).resolve().parents[1] / name)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


ci = module("run-sim-ci.py")
trace = module("sim-traceability.py")


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name)

    def quarantine(self, entries):
        path = self.path / "quarantine.json"
        path.write_text(json.dumps({"schema_version": 1, "scenarios": entries}))
        return ci.quarantine(path, {"failure"}, date(2026, 10, 9))

    def entry(self, **kwargs):
        return dict(scenario="failure", owner="combat-team", reason="timing investigation", issue="NF-42",
                    expires="2026-10-10", **kwargs)

    def test_expiry_and_owner_enforced(self):
        entry = self.entry()
        self.assertIn("failure", self.quarantine([entry]))
        for bad in ({**entry, "expires": "2026-10-08"}, {**entry, "owner": ""},
                    {**entry, "scenario": "typo"}):
            with self.assertRaises(ValueError):
                self.quarantine([bad])
        with self.assertRaises(ValueError):
            self.quarantine([entry, entry])

    def test_roles_are_paired_and_orphans_rejected(self):
        batches = ci.units([Path("fight-b.nfs"), Path("solo.nfs"), Path("fight-a.nfs")])
        self.assertEqual([[p.stem for p in b] for b in batches], [["fight-a", "fight-b"], ["solo"]])
        for name in ("fight-a", "fight-b"):
            with self.assertRaises(ValueError):
                ci.units([Path(name + ".nfs")])

    def test_missing_and_failed_junit_enforced(self):
        folder = self.path / "failure"
        folder.mkdir()
        self.assertEqual(ci.junit_failures(self.path, ["failure"]), ["failure"])
        report = folder / "failure.xml"
        for content in ('<testsuite tests="1" failures="1"/>',
                        '<testsuite tests="1"><testcase><failure/></testcase></testsuite>', '<broken'):
            report.write_text(content)
            self.assertEqual(ci.junit_failures(self.path, ["failure"]), ["failure"])
        report.write_text('<testsuite tests="1" failures="0"><testcase name="step"/></testsuite>')
        self.assertEqual(ci.junit_failures(self.path, ["failure"]), [])
        report.write_text('<testsuite tests="0" failures="0"/>')
        self.assertEqual(ci.junit_failures(self.path, ["failure"]), ["failure"])
        report.write_text('<testsuite tests="1"><testcase name="pipeline"><failure/></testcase></testsuite>')
        self.assertFalse(ci.has_scenario_failure(self.path, "failure"))
        report.write_text('<testsuite tests="1"><testcase name="nf.Expect"><failure/></testcase></testsuite>')
        self.assertTrue(ci.has_scenario_failure(self.path, "failure"))

    def test_story_coverage_checks_completed_visible_rows(self):
        plans, scenarios = self.path / "plans", self.path / "scenarios"
        plans.mkdir(); scenarios.mkdir()
        plan = plans / "phase-1-combat.md"
        plan.write_text('## 4. Epics, stories, tasks\n| ✅ 2.3 Attack [client-visible] | work |\n'
                        '| 2.4 Death [client-visible] | work |\n| ✅ 3.1 Data | work |\n'
                        '## 8. Outcome\n| ✅ 4.9 Not a story [client-visible] | work |\n')
        scenario = scenarios / "fight.nfs"
        scenario.write_text('# covers: 1 E2.3\nnf.Login\n')
        self.assertFalse(trace.check(plans, scenarios)[1])
        plan.write_text(plan.read_text().replace('| 2.4', '| ✅ 2.4'))
        output, failed = trace.check(plans, scenarios)
        self.assertTrue(failed)
        self.assertTrue(any('UNCOVERED 1 E2.4' in line for line in output))
        scenario.write_text('# covers: 1 E9.9\n')
        self.assertTrue(any('unknown story' in line for line in trace.check(plans, scenarios)[0]))

    def test_missing_tags_cannot_succeed_vacuously(self):
        self.assertTrue(trace.check(self.path, self.path)[1])

    def test_roles_must_agree_on_supported_fixture(self):
        a, b = self.path / 'fight-a.nfs', self.path / 'fight-b.nfs'
        a.write_text('# fixture: phase1a-social-aggro\n')
        b.write_text('# fixture: phase1a-social-aggro\n')
        self.assertEqual(ci.fixture([a, b]), 'phase1a-social-aggro')
        b.write_text('# default fixture\n')
        with self.assertRaises(ValueError):
            ci.fixture([a, b])
        a.write_text('# fixture: arbitrary-server-config\n')
        with self.assertRaises(ValueError):
            ci.fixture([a])

    def test_real_scenario_catalogue_fixture_routing(self):
        batches = ci.units(sorted((ci.PROJECT / "Scenarios").glob("*.nfs")))
        routes = {tuple(path.stem for path in batch): ci.fixture(batch) for batch in batches}
        dedicated = {
            ("1-social-aggro-a", "1-social-aggro-b"): "phase1a-social-aggro",
            ("1-late-entry-a", "1-late-entry-b"): "phase1a-late-entry",
        }
        for roles in dedicated:
            self.assertIn(roles, routes)
        for roles, selected in routes.items():
            with self.subTest(roles=roles):
                self.assertEqual(selected, dedicated.get(roles, "default"))


if __name__ == "__main__":
    unittest.main()
