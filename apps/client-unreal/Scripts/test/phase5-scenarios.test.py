#!/usr/bin/env python3
"""Story 1.8 structure checks for the 36 Phase 5 scenario files (plan §2.8, §5.3; design §14.3).

Static only (no live run, 6.6 owns that): every file carries its plan §2.8 header verbatim and a
§5.3 pack; every command/predicate comes from the closed §5.3 + registered Phase 3/4 vocabulary;
every `# econ-gate:` label is a design §14.3 formula, chance callsite, select callsite or RNG
callsite; the budget equals the largest role's sum of waits + 30 s; role blocks name pack roles;
the oracle sidecars and pack declarations are present and `phase5-expectations.py --check` clean.
"""
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import unittest

CLIENT = Path(__file__).resolve().parents[2]
REPO = CLIENT.parents[1]
SCENARIOS = CLIENT / "Scenarios"
PLAN = REPO / "docs/plans/phase-5-economy-and-crafting.md"
DESIGN = REPO / "docs/plans/phase-5-zone-design.md"
PACKS = json.loads((SCENARIOS / "fixtures/phase5-packs.json").read_text())["packs"]

spec5 = importlib.util.spec_from_file_location('grammar5', CLIENT / 'Scripts/phase5-grammar.py')
grammar5 = importlib.util.module_from_spec(spec5)
spec5.loader.exec_module(grammar5)
spec4 = importlib.util.spec_from_file_location('grammar4', CLIENT / 'Scripts/phase4-grammar.py')
grammar4 = importlib.util.module_from_spec(spec4)
spec4.loader.exec_module(grammar4)
REGISTRY = {line.strip() for line in (CLIENT / 'Scripts/test/fixtures/bot-registry-76ac549.txt').read_text().splitlines() if line.strip() and not line.startswith('#')}
# The closed vocabulary: the Phase 5 contract (§5.3, phase5-grammar.py) plus names the client registers
# (phase4-grammar.py and the committed 76ac549 registry snapshot). Nothing is typed here.
COMMANDS = {n[3:] for n in grammar5.COMMANDS} | {n[3:] for n in grammar4.COMMANDS} | {n[3:] for n in REGISTRY if n.startswith('nf.')} | {'WaitFor', 'Expect', 'Within'}
PREDICATES = set(grammar5.PREDICATES) | set(grammar4.PREDICATES) | {n for n in REGISTRY if not n.startswith('nf.')}
GATE_LABELS = {
    # design §14.3 econ_formula
    "drop_chance", "vendor_unit_price", "sell_price", "refund_price", "party_xp", "solo_xp", "even_split", "warehouse_fee", "freight_fee", "mail_fee", "crop_reward", "crop_fee",
    # econ_chance callsites
    "corpse_item", "death_item", "death_remainder", "group_chance", "group_remainder", "pk_gate", "pk_item", "craft_success", "craft_rare", "sow", "harvest",
    # econ_select / item_rng callsites
    "group_select", "drop_quantity", "npc_drop_dx", "npc_drop_dy", "pk_drop_dx", "pk_drop_dy", "party_random", "party_split_remainder", "auction_item",
}


def plan_rows() -> dict[str, tuple[str, str]]:
    rows = {}
    for line in PLAN.read_text().splitlines():
        m = re.match(r"\| `(5-[a-z0-9-]+)\.nfs` \| `(# covers: [^`]+)` \| (phase5-[a-z0-9-]+) \|", line)
        if m:
            rows[m[1]] = (m[2], m[3])
    return rows


def plan_packs() -> set[str]:
    text = PLAN.read_text()
    start = text.index("### 5.3 Fixture packs")
    block = text[start:text.index("**Commands and predicates.**", start)]
    return set(re.findall(r"`(phase5-[a-z0-9-]+)`", block))


def design_gate_labels() -> set[str]:
    text = DESIGN.read_text()
    section = text[text.index("### 14.3 Evidence contract v2"):text.index("### 14.4")]
    return set(re.findall(r"`([a-z_]+)`", section)) | set(re.findall(r"callsite: ([a-z_]+)", section))


class ScenarioFileTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rows = plan_rows()
        cls.files = {p.stem: p.read_text().splitlines() for p in sorted(SCENARIOS.glob("5-*.nfs"))}

    def test_exactly_the_36_plan_files_with_verbatim_headers(self):
        self.assertEqual(len(self.rows), 36)
        self.assertEqual(set(self.files), set(self.rows))
        for name, lines in self.files.items():
            covers, fixture = self.rows[name]
            self.assertEqual(lines[0], covers, name)
            self.assertEqual(lines[1], f"# fixture: {fixture}", name)
            self.assertIn("5 E1.8", covers)

    def test_fixtures_are_plan_5_3_packs_with_declarations(self):
        packs = plan_packs()
        self.assertEqual(len(packs), 27)
        self.assertEqual(set(PACKS), packs)
        for name, (_, fixture) in self.rows.items():
            self.assertIn(fixture, PACKS, name)
            sidecar = json.loads((SCENARIOS / f"{name}.expect.json").read_text())
            self.assertEqual(sidecar["fixture"], fixture)
            self.assertEqual(sidecar["scenario"], name + ".nfs")
            self.assertTrue(sidecar["expectations"], name)

    def test_vocabulary_is_closed(self):
        for name, lines in self.files.items():
            for ln in lines:
                if not ln.startswith("nf."):
                    continue
                parts = ln.split()
                cmd = parts[0][3:]
                self.assertIn(cmd, COMMANDS, f"{name}: {ln}")
                if cmd in ("WaitFor", "Expect"):
                    self.assertIn(parts[1], PREDICATES, f"{name}: {ln}")
                    if cmd == "WaitFor":
                        self.assertRegex(parts[-1], r"^\d+$", f"{name}: WaitFor needs a timeout: {ln}")

    def test_gate_labels_are_design_14_3_names(self):
        allowed = GATE_LABELS & design_gate_labels()
        self.assertEqual(allowed, GATE_LABELS, GATE_LABELS - design_gate_labels())
        for name, lines in self.files.items():
            seen = []
            for ln in lines:
                if ln.startswith("# econ-gate:"):
                    m = re.fullmatch(r"# econ-gate: ([a-z_]+)", ln)
                    self.assertIsNotNone(m, f"{name}: {ln}")
                    self.assertIn(m[1], GATE_LABELS, f"{name}: {ln}")
                    self.assertNotIn(m[1], seen, f"{name}: duplicate gate {m[1]}")
                    seen.append(m[1])

    def test_budget_is_largest_role_waits_plus_grace(self):
        for name, lines in self.files.items():
            roles: dict[str, int] = {}
            current = "a"
            for ln in lines:
                m = re.fullmatch(r"# role: ([abc])", ln)
                if m:
                    current = m[1]
                    roles.setdefault(current, 0)
                    continue
                m = re.match(r"nf\.WaitFor .* (\d+)$", ln)
                if m:
                    roles[current] = roles.get(current, 0) + int(m[1])
            within = [int(ln.split()[1]) for ln in lines if ln.startswith("nf.Within ")]
            self.assertEqual(len(within), 1, name)
            self.assertEqual(within[0], max(roles.values()) + 30, name)
            self.assertEqual(lines[-1], f"nf.Within {within[0]}", name)

    def test_role_blocks_name_pack_roles_and_every_role_asserts(self):
        for name, lines in self.files.items():
            fixture = self.rows[name][1]
            pack_roles = {r["role"] for r in PACKS[fixture]["roles"]}
            blocks = [ln.split()[-1] for ln in lines if ln.startswith("# role:")]
            self.assertEqual(len(blocks), len(set(blocks)), name)
            self.assertTrue(set(blocks) <= pack_roles, f"{name}: {set(blocks) - pack_roles}")
            # every role (or the implicit role a) logs in and makes at least one assertion
            body = "\n".join(lines)
            sections = re.split(r"^# role: [abc]\n", body + "\n", flags=re.M)[1:] if blocks else [body]
            for section in sections:
                self.assertIn("nf.Login", section, name)
                self.assertRegex(section, r"nf\.(Expect|WaitFor) ", name)
            if not blocks:
                self.assertIn("a", pack_roles, name)

    def test_expectations_generator_is_clean(self):
        result = subprocess.run([sys.executable, str(CLIENT / "Scripts/phase5-expectations.py"), "--check"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("36 sidecars, 27 packs", result.stdout)


if __name__ == "__main__":
    unittest.main()
