#!/usr/bin/env python3
"""Traceability scope policy tests; no UE, server or browser required."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("traceability", SCRIPTS / "sim-traceability.py")
trace = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trace)


class TraceabilityTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.plans = self.root / "plans"
        self.scenarios = self.root / "Scenarios"
        self.plans.mkdir()
        self.scenarios.mkdir()
        (self.plans / "phase-0b-test.md").write_text(
            "## 4. Epics, stories, tasks\n"
            "| ✅ 1.5 Device login [client-visible] | work |\n"
            "| ✅ 6.2 Login and connect [client-visible] | work |\n"
            "| ✅ 7.1 Docs | work |\n"
            "## 9. Outcome\n| ✅ 9.9 Not a story [client-visible] | work |\n")
        (self.plans / "phase-1a-test.md").write_text(
            "## 4. Epics, stories, tasks\n| 3.6 Late entry | work |\n")
        (self.scenarios / "login.nfs").write_text("# covers: 0b E6.2\nnf.Login\n")
        self.manifest = self.scenarios / "traceability-exceptions.json"

    def entry(self, **changes):
        return {"story": "0b E1.5", "reason": "Headless uses dev tokens; browser approval remains manual.",
                **changes}

    def check(self, entries):
        self.manifest.write_text(json.dumps({"schema_version": 1, "exceptions": entries}))
        output, failed = trace.check(self.plans, self.scenarios, self.manifest)
        return "\n".join(output), failed

    def test_reasoned_exception_does_not_count_as_covered_or_remove_plan_tag(self):
        before = (self.plans / "phase-0b-test.md").read_text()
        output, failed = self.check([self.entry()])
        self.assertFalse(failed, output)
        self.assertIn("2 completed client-visible stories: 1 covered, 1 exception, 0 missing", output)
        self.assertIn("EXCEPTION 0b E1.5", output)
        self.assertEqual((self.plans / "phase-0b-test.md").read_text(), before)

    def test_missing_is_still_missing_with_other_exceptions(self):
        (self.scenarios / "login.nfs").unlink()
        output, failed = self.check([self.entry()])
        self.assertTrue(failed)
        self.assertIn("0 covered, 1 exception, 1 missing", output)
        self.assertIn("UNCOVERED 0b E6.2", output)

    def test_unknown_exception_and_outcome_reference_are_rejected(self):
        for story in ("0b E4.9", "0b E9.9", "1a E3.7", None, 15):
            with self.subTest(story=story):
                output, failed = self.check([self.entry(story=story)])
                self.assertTrue(failed)
                self.assertIn("unknown story reference", output)

    def test_missing_empty_and_nonstring_reasons_are_rejected(self):
        for reason in (None, "", " \n ", 123):
            with self.subTest(reason=reason):
                entry = self.entry(reason=reason)
                if reason is None:
                    del entry["reason"]
                output, failed = self.check([entry])
                self.assertTrue(failed)
                self.assertIn("nonempty reason is required", output)
                self.assertIn("1 covered, 0 exception, 1 missing", output)

    def test_duplicate_exception_is_rejected(self):
        output, failed = self.check([self.entry(), self.entry(story="0b 1.5")])
        self.assertTrue(failed)
        self.assertIn("duplicate exception", output)

    def test_cover_tag_with_exception_requires_subscope_and_never_counts_covered(self):
        entry = self.entry(story="0b E6.2")
        output, failed = self.check([entry, self.entry()])
        self.assertTrue(failed)
        self.assertIn("require an explicit subscope", output)
        entry["subscope"] = {"scope": "Dev-token admission only", "scenarios": ["login.nfs"]}
        output, failed = self.check([entry, self.entry()])
        self.assertFalse(failed, output)
        self.assertIn("0 covered, 2 exception, 0 missing", output)
        self.assertIn("SUBSCOPE Dev-token admission only", output)

    def test_all_tagged_scenarios_must_be_named_in_subscope(self):
        (self.scenarios / "login-again.nfs").write_text("# covers: 0b E6.2\n")
        output, failed = self.check([self.entry(story="0b E6.2", subscope={
            "scope": "Dev login", "scenarios": ["login.nfs"]})])
        self.assertTrue(failed)
        self.assertIn("must list every scenario", output)

    def test_unknown_or_untagged_scenario_evidence_is_rejected(self):
        for name in ("typo.nfs", "../Scenarios/login.nfs", 17, "login.nfs"):
            with self.subTest(name=name):
                output, failed = self.check([self.entry(subscope={
                    "scope": "Post-login only", "scenarios": [name]})])
                self.assertTrue(failed)
                self.assertIn("unknown or untagged scenario", output)

    def test_scope_shape_and_evidence_are_required(self):
        for scope in (None, "text", {}, {"scope": "partial"}, {"scope": " ", "scenarios": ["login.nfs"]},
                      {"scope": "partial", "scenarios": "login.nfs"}):
            with self.subTest(scope=scope):
                output, failed = self.check([self.entry(subscope=scope)])
                self.assertTrue(failed, output)
                self.assertIn("ERROR", output)

    def automation_source(self):
        source = self.root / "Source/Tests/CombatTest.cpp"
        source.parent.mkdir(parents=True)
        source.write_text('IMPLEMENT_SIMPLE_AUTOMATION_TEST(FReplayTest, "Nightfall.Replay", Flags)\n')
        return {"path": "Source/Tests/CombatTest.cpp", "test": "Nightfall.Replay"}

    def test_combined_evidence_for_current_incomplete_story_is_separate_from_totals(self):
        evidence = self.automation_source()
        (self.scenarios / "late.nfs").write_text("# covers: 1a E3.6\n")
        output, failed = self.check([self.entry(), self.entry(story="1a E3.6", subscope={
            "scope": "Reconnect integration plus decoder adversarial replay",
            "scenarios": ["late.nfs"], "automation": [evidence]})])
        self.assertFalse(failed, output)
        self.assertIn("1 covered, 1 exception, 0 missing", output)
        self.assertIn("EXCEPTION 1a E3.6 (outside completed client-visible totals)", output)
        self.assertIn("AUTOMATION Nightfall.Replay (Source/Tests/CombatTest.cpp)", output)

    def test_unknown_automation_path_or_test_is_rejected(self):
        good = self.automation_source()
        for evidence in ({**good, "test": "Nightfall.Typo"}, {**good, "path": "missing.cpp"},
                         {**good, "path": "/tmp/outside.cpp"}, {**good, "path": "../outside.cpp"},
                         {"path": good["path"]}, "bad"):
            with self.subTest(evidence=evidence):
                output, failed = self.check([self.entry(subscope={
                    "scope": "adversarial replay", "automation": [evidence]})])
                self.assertTrue(failed)
                self.assertIn("automation", output)

    def test_invalid_manifest_version_shape_json_or_missing_file_fail_closed(self):
        for data in ({"schema_version": 2, "exceptions": []},
                     {"schema_version": True, "exceptions": []}, [],
                     {"schema_version": 1, "exceptions": {}}, {"exceptions": []}):
            self.manifest.write_text(json.dumps(data))
            output, failed = trace.check(self.plans, self.scenarios, self.manifest)
            self.assertTrue(failed)
            self.assertTrue(any("expected schema_version" in line for line in output))
        self.manifest.write_text("{broken")
        self.assertTrue(trace.check(self.plans, self.scenarios, self.manifest)[1])
        self.manifest.unlink()
        self.assertTrue(trace.check(self.plans, self.scenarios, self.manifest)[1])

    def test_unknown_scenario_tags_remain_errors_despite_valid_exception(self):
        (self.scenarios / "bad.nfs").write_text("# covers: 0b E9.9\n")
        output, failed = self.check([self.entry()])
        self.assertTrue(failed)
        self.assertIn("bad.nfs: unknown story", output)

    def test_cli_uses_explicit_manifest_and_returns_failure_for_invalid_reason(self):
        for reason, expected in (("browser UI manual", 0), ("", 1)):
            self.check([self.entry(reason=reason)])
            result = subprocess.run([sys.executable, str(SCRIPTS / "sim-traceability.py"),
                                     "--plans", str(self.plans), "--scenarios", str(self.scenarios),
                                     "--manifest", str(self.manifest)],
                                    text=True, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)

    def test_current_repo_manifest_references_real_stories_and_test_declarations(self):
        project = SCRIPTS.parent
        output, failed = trace.check(project.parents[1] / "docs/plans", project / "Scenarios",
                                     project / "Scenarios/traceability-exceptions.json")
        self.assertFalse(failed, "\n".join(output))
        self.assertIn("2 exception, 0 missing", output[0])
        self.assertIn("EXCEPTION 0b E1.5", "\n".join(output))
        header = (project / "Scenarios/0b-login-enter-world.nfs").read_text().splitlines()[0]
        self.assertNotIn("0b E1.5", header)


if __name__ == "__main__":
    unittest.main()
