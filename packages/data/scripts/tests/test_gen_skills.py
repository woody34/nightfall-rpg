"""Unit tests for gen_skills.py: parser, tables, condition trees, stat children, and blockers."""

from __future__ import annotations

import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

SCRIPT = Path(__file__).resolve().parents[1] / "gen_skills.py"
SPEC = importlib.util.spec_from_file_location("gen_skills", SCRIPT)
GEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GEN)


class SourceProvenanceTests(unittest.TestCase):
    def test_pinned_head_does_not_authorize_modified_source_inputs(self):
        responses = [
            SimpleNamespace(stdout=GEN.DATAPACK_REVISION),
            SimpleNamespace(stdout=" M src/main/resources/data/stats/skills/00000-00099.xml\n"),
        ]
        with patch.object(GEN.subprocess, "run", side_effect=responses) as git:
            with self.assertRaisesRegex(GEN.SourceError, "source inputs are dirty"):
                GEN.Datapack(Path("/unused/local/source"))
            command = git.call_args.args[0]
            self.assertIn("--untracked-files=all", command)
            self.assertIn("--ignored=matching", command)
            self.assertIn(GEN.DATA_STATS_SKILLS, command)
            self.assertIn(GEN.LEARNING_SOURCE, command)

    def test_clean_pinned_sources_are_admitted(self):
        responses = [
            SimpleNamespace(stdout=GEN.DATAPACK_REVISION),
            SimpleNamespace(stdout=""),
        ]
        with patch.object(GEN.subprocess, "run", side_effect=responses):
            self.assertEqual(GEN.Datapack(Path("/unused/local/source")).root, Path("/unused/local/source"))

    def test_other_source_revision_is_refused_before_working_tree_checks(self):
        with patch.object(GEN.subprocess, "run", return_value=SimpleNamespace(stdout="f" * 40)) as git:
            with self.assertRaisesRegex(GEN.SourceError, "expected pinned"):
                GEN.Datapack(Path("/unused/local/source"))
            self.assertEqual(git.call_count, 1)


class TableExpansionTests(unittest.TestCase):
    def test_table_parsing(self):
        xml = """
        <skill id="9999" levels="3" name="Test Skill">
            <table name="#mpConsume2"> 10 20 30 </table>
            <table name="#power"> 1.1 2.2 3.3 </table>
        </skill>
        """
        elem = ET.fromstring(xml)
        tables = GEN.parse_tables(elem)
        self.assertEqual(tables["#mpConsume2"], ["10", "20", "30"])
        self.assertEqual(tables["#power"], ["1.1", "2.2", "3.3"])

    def test_per_level_array_generation(self):
        xml = """
        <skill id="9999" levels="3" name="Test Skill">
            <table name="#mp"> 15 25 35 </table>
            <set name="mpConsume2" val="#mp" />
            <set name="hitTime" val="1500" />
            <set name="operateType" val="A1" />
            <set name="targetType" val="ENEMY" />
        </skill>
        """
        elem = ET.fromstring(xml)
        toml_text = GEN.generate_skill_toml(9999, "test.xml", elem, set())
        self.assertIn("mp_consume_end = [15, 25, 35]", toml_text)
        self.assertIn("hit_time_ms = 1500", toml_text)


class ConditionTreeTests(unittest.TestCase):
    def test_using_kind_allowed(self):
        xml = '<using kind="SWORD,BLUNT" />'
        elem = ET.fromstring(xml)
        self.assertTrue(GEN.is_cond_allowed(elem))
        node = GEN.parse_cond_node(elem)
        self.assertEqual(node["kind"], "using_kind")
        self.assertEqual(node["weapon_kinds"], ["SWORD", "BLUNT"])

    def test_player_can_resurrect_allowed(self):
        xml = '<player canResurrect="true" />'
        elem = ET.fromstring(xml)
        self.assertTrue(GEN.is_cond_allowed(elem))
        node = GEN.parse_cond_node(elem)
        self.assertEqual(node["kind"], "player_can_resurrect")

    def test_recursive_boolean_wrappers(self):
        xml = """
        <and>
            <using kind="DAGGER" />
            <player canResurrect="true" />
        </and>
        """
        elem = ET.fromstring(xml)
        self.assertTrue(GEN.is_cond_allowed(elem))
        node = GEN.parse_cond_node(elem)
        self.assertEqual(node["kind"], "and")
        self.assertEqual(len(node["conditions"]), 2)

    def test_unsupported_condition_blocks(self):
        xml = """
        <cond>
            <player hp="20" />
        </cond>
        """
        elem = ET.fromstring(xml)
        blockers = GEN.get_cond_blockers(elem)
        self.assertEqual(blockers, {"cond:player:hp"})


class StatChildrenTests(unittest.TestCase):
    def test_whitelisted_stats_no_blockers(self):
        xml = """
        <skill id="9998" levels="1" name="Passives">
            <set name="operateType" val="P" />
            <set name="targetType" val="SELF" />
            <effects>
                <effect name="Passive">
                    <mul stat="pAtk" val="1.1" />
                    <add stat="pDef" val="20" />
                    <mul stat="runSpd" val="1.05" />
                    <add stat="darkRes" val="10" />
                </effect>
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        _, blockers = GEN.compute_all_blockers({9998: ("test.xml", elem)}, {9998})
        self.assertEqual(blockers[9998], set())

    def test_unsupported_stat_blocks(self):
        xml = """
        <skill id="9997" levels="1" name="Shield Passives">
            <set name="operateType" val="P" />
            <set name="targetType" val="SELF" />
            <effects>
                <effect name="Passive">
                    <mul stat="sDef" val="1.2" />
                    <add stat="rShld" val="15" />
                </effect>
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        _, blockers = GEN.compute_all_blockers({9997: ("test.xml", elem)}, {9997})
        self.assertEqual(blockers[9997], {"stat:sDef", "stat:rShld"})


class BlockerComputationTests(unittest.TestCase):
    def test_target_me_is_blocked(self):
        xml = """
        <skill id="9996" levels="1" name="Aura of Hate">
            <set name="operateType" val="A1" />
            <set name="targetType" val="ENEMY" />
            <effects>
                <effect name="TargetMe" />
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        _, blockers = GEN.compute_all_blockers({9996: ("test.xml", elem)}, {9996})
        self.assertIn("effect:TargetMe", blockers[9996])

    def test_item_consume_is_blocked(self):
        xml = """
        <skill id="9995" levels="1" name="Costly Spell">
            <set name="operateType" val="A1" />
            <set name="targetType" val="TARGET" />
            <set name="itemConsumeId" val="3031" />
            <set name="itemConsumeCount" val="1" />
            <effects>
                <effect name="Buff" />
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        _, blockers = GEN.compute_all_blockers({9995: ("test.xml", elem)}, {9995})
        self.assertIn("item_consume", blockers[9995])

    def test_transitive_blocker_propagation(self):
        trigger_xml = """
        <skill id="9001" levels="1" name="Trigger Effect">
            <set name="operateType" val="A1" />
            <set name="targetType" val="TARGET" />
            <effects>
                <effect name="TargetMe" />
            </effects>
        </skill>
        """
        parent_xml = """
        <skill id="9000" levels="1" name="Parent Skill">
            <set name="operateType" val="P" />
            <set name="targetType" val="SELF" />
            <effects>
                <effect name="Buff">
                    <param skillId="9001" />
                </effect>
            </effects>
        </skill>
        """
        skills = {
            9000: ("parent.xml", ET.fromstring(parent_xml)),
            9001: ("trigger.xml", ET.fromstring(trigger_xml)),
        }
        _, blockers = GEN.compute_all_blockers(skills, {9000})
        self.assertIn("effect:TargetMe", blockers[9001])
        self.assertIn("ref:9001", blockers[9000])

    def test_tick_hp_fatal_is_blocked(self):
        xml = """
        <skill id="9994" levels="1" name="Fatal Poison">
            <set name="operateType" val="A2" />
            <set name="targetType" val="ENEMY" />
            <effects>
                <effect name="TickHpFatal">
                    <param power="-50" />
                    <param ticks="5" />
                </effect>
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        _, blockers = GEN.compute_all_blockers({9994: ("test.xml", elem)}, {9994})
        self.assertIn("effect:TickHpFatal", blockers[9994])

    def test_get_agro_is_executable(self):
        xml = """
        <skill id="9993" levels="1" name="Agro Skill">
            <set name="operateType" val="A1" />
            <set name="targetType" val="ENEMY" />
            <effects>
                <effect name="GetAgro" />
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        _, blockers = GEN.compute_all_blockers({9993: ("test.xml", elem)}, {9993})
        self.assertEqual(blockers[9993], set())

    def test_affect_limit_structured_table(self):
        xml = """
        <skill id="9992" levels="1" name="AoE Sweep">
            <set name="operateType" val="A1" />
            <set name="targetType" val="ENEMY" />
            <set name="affectScope" val="RANGE" />
            <set name="affectRange" val="200" />
            <set name="affectLimit" val="5-12" />
            <effects>
                <effect name="PhysicalAttack">
                    <param power="100" />
                </effect>
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        toml_text = GEN.generate_skill_toml(9992, "test.xml", elem, set())
        self.assertIn("affect_limit = { offset = 5, width = 12 }", toml_text)

    def test_periodic_effect_ticks_and_mode_and_raw_conditions(self):
        xml = """
        <skill id="9991" levels="1" name="DoT and Buff">
            <set name="operateType" val="A2" />
            <set name="targetType" val="ENEMY" />
            <cond msgId="113" addName="1">
                <using kind="SWORD,BLUNT" />
            </cond>
            <effects>
                <effect name="TickHp">
                    <param power="-25" />
                    <param ticks="5" />
                </effect>
                <effect name="MaxHp">
                    <param power="20" />
                    <param type="PER" />
                    <param heal="true" />
                </effect>
            </effects>
        </skill>
        """
        elem = ET.fromstring(xml)
        toml_text = GEN.generate_skill_toml(9991, "test.xml", elem, set())
        # Check raw_conditions
        import tomllib
        parsed = tomllib.loads(toml_text)
        self.assertIn("raw_conditions", parsed)
        self.assertEqual(parsed["raw_conditions"], ['<cond msgId="113" addName="1"><using kind="SWORD,BLUNT" /></cond>'])
        # Check TickHp mode defaulted to DIFF and ticks preserved
        self.assertIn('mode = "DIFF"', toml_text)
        self.assertIn('ticks = "5"', toml_text)
        # Check MaxHp heal preserved verbatim
        self.assertIn('heal = "true"', toml_text)
        self.assertIn('type = "PER"', toml_text)


if __name__ == "__main__":
    unittest.main()
