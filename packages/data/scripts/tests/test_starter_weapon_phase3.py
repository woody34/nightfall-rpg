"""Pinned XML/Java audit, replacement oracle and legacy-boundary regressions."""
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import re
import tempfile
import tomllib
import unittest
import xml.etree.ElementTree as ET

from packages.data.scripts import gen_class_phase3 as GEN
from packages.data.scripts import starter_weapon_oracle as ORACLE

DP = Path('/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp')
GAME = DP.parent / 'l2j-game'
DATA = ORACLE.ROOT


class WeaponSourceTests(unittest.TestCase):
    def test_generated_weapon_matches_every_xml_func_in_order(self):
        content = GEN.weapon_profile(DP, GAME)
        self.assertEqual(content, (DATA / 'tables-phase3/starter_weapon.toml').read_text())
        p = tomllib.loads(content)
        item = next(n for n in ET.parse(DP / GEN.WEAPON).getroot() if n.get('id') == str(p['item_id']))
        properties = {n.get('name'): n.get('val') for n in item.findall('set')}
        for key in ('weapon_type', 'bodypart', 'random_damage', 'attack_range'):
            self.assertEqual(str(p[key]), properties[key])
        nodes = list(item.find('for'))
        self.assertEqual(len(p['functions']), len(nodes))
        enum = (GAME / (GEN.GAME_SRC + 'enums/StatFunction.java')).read_text()
        default_order = int(re.search(r'SET\("Set", (\d+)\)', enum)[1])
        for ordinal, (func, node) in enumerate(zip(p['functions'], nodes)):
            self.assertEqual(func['stat'], node.get('stat'))
            self.assertEqual(func['operation'], node.tag.upper())
            self.assertEqual(func['order'], int(node.get('order', str(default_order))))
            self.assertEqual(func['xml_ordinal'], ordinal)
            self.assertEqual(func['value'], node.get('val'))
            self.assertEqual(func['value_q'], F(node.get('val')) * 1000000)
            line = (DP / GEN.WEAPON).read_text().splitlines()[int(func['source']['lines']) - 1]
            self.assertIn(f'stat="{func["stat"]}" val="{func["value"]}"', line)
        self.assertIn('return getValue();', (GAME / (GEN.GAME_SRC + 'model/stats/functions/FuncSet.java')).read_text())
        for provenance in p['ordering'].values():
            self.assertEqual(provenance['revision'], GEN.GAME_REV)
            self.assertTrue((GAME / provenance['file']).is_file())

    def test_missing_or_unexpected_weapon_functions_fail_closed(self):
        original = (DP / GEN.WEAPON).read_text()
        for before, after in (('<set stat="mAtk" val="5" />', ''),
                              ('<set stat="mAtk" val="5" />', '<add stat="mAtk" val="5" />'),
                              ('<set stat="mAtk" val="5" />', '<set stat="mAtk" val="NaN" />')):
            # Change only the requested item, not another item's matching Func.
            prefix, tail = original.split('<item id="2369"', 1)
            mutated = prefix + '<item id="2369"' + tail.replace(before, after, 1)
            with self.subTest(after=after), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                path = root / GEN.WEAPON
                path.parent.mkdir(parents=True)
                path.write_text(mutated)
                with self.assertRaises(GEN.legacy.SourceError):
                    GEN.weapon_profile(root, GAME)

    def test_constants_are_literal_pinned_config_with_line_provenance(self):
        content = GEN.phase3_constants(GAME)
        self.assertEqual(content, (DATA / 'tables-phase3/constants.toml').read_text())
        constants = tomllib.loads(content)['constants']
        self.assertEqual(set(constants), {'MaxMCritRate', 'MaxRunSpeed', 'RunSpeedBoost'})
        for key, row in constants.items():
            line = (GAME / row['file']).read_text().splitlines()[row['line'] - 1].strip()
            self.assertEqual(line, row['literal'])
            self.assertEqual(key, row['symbol'])
            self.assertEqual(row['value'], int(line.split('=')[1]))

    def test_legacy_table_and_oracle_manifest_remain_exact(self):
        # Pre-task SHA from the historical starter-weapon oracle manifest.
        digest = '73cc8e7f4a8242e9ef01272e38620ef56b3d15b721752a25d1420d823b926a33'
        self.assertEqual(hashlib.sha256((DATA / 'tables/starter_weapon.toml').read_bytes()).hexdigest(), digest)
        manifest = json.loads((DATA / 'fixtures/oracle.json').read_text())['inputs_sha256']
        self.assertEqual(manifest['tables/starter_weapon.toml'], digest)
        self.assertFalse(any(p.startswith('tables-phase3/') for p in manifest))
        for rel, expected in manifest.items():
            self.assertEqual(hashlib.sha256((DATA / rel).read_bytes()).hexdigest(), expected)


class EquippedOracleTests(unittest.TestCase):
    def test_fixture_and_independent_hashes(self):
        golden, content = ORACLE.generate()
        self.assertEqual(content, ORACLE.OUT.read_text())
        for rel, digest in golden['inputs_sha256'].items():
            self.assertEqual(hashlib.sha256((DATA / rel).read_bytes()).hexdigest(), digest)
        self.assertIn('tables-phase3/starter_weapon.toml', golden['inputs_sha256'])
        expected = hashlib.sha256(json.dumps(golden['formulas'], sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        self.assertEqual(golden['vector_set_sha256'], expected)

    def test_six_classes_four_levels_replace_rather_than_add_template_base(self):
        golden, _ = ORACLE.generate()
        rows = golden['formulas']['FuncMAtkMod']
        self.assertEqual(len(rows), 24)
        self.assertEqual({(v['inputs']['class_key'], v['inputs']['level']) for v in rows},
                         {(key, level) for key in ORACLE.KEYS for level in ORACLE.LEVELS})
        bonuses = tomllib.loads((DATA / 'tables/stat_bonus.toml').read_text())['int']['bonus']
        source_item = next(n for n in ET.parse(DP / GEN.WEAPON).getroot() if n.get('id') == '2369')
        weapon_base = F(source_item.find("for/set[@stat='mAtk']").get('val'))
        for v in rows:
            i = v['inputs']
            profile = tomllib.loads((DATA / i['template_file']).read_text())
            xml = ET.parse(DP / profile['source']['file']).getroot()
            template_base = F(xml.findtext('staticData/baseMAtk'))
            self.assertEqual(template_base, 6)  # baseStats XML cited in each profile
            self.assertEqual(i['template_base'], template_base)
            bonus = F(bonuses[profile['base_stats']['int']])
            lm = F(i['level'] + 89, 100)
            expected = weapon_base * bonus**2 * lm**2
            self.assertEqual(F(v['exact_rational']), expected)
            self.assertEqual(v['q_result'], expected.numerator * 1000000 // expected.denominator)
            self.assertEqual(v['rounded_result'], expected.numerator // expected.denominator)
            self.assertEqual(F(v['intermediate']['after_order_0_set']), weapon_base)
            self.assertEqual(F(v['intermediate']['unarmed_exact']), template_base * bonus**2 * lm**2)
            self.assertNotEqual(expected, (template_base + weapon_base) * bonus**2 * lm**2)
            self.assertEqual(i['rolls'], [])


if __name__ == '__main__':
    unittest.main()
