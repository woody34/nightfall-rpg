"""Regression and full-corpus checks for DATA review 1, using pinned XML inputs."""
from pathlib import Path
import tomllib
import unittest
import xml.etree.ElementTree as ET

from test_gen_skills import GEN

DATA = Path(__file__).resolve().parents[2]


class GeneratedCorpusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.xml = GEN.load_all_datapack_skills(GEN.Datapack(GEN.DEFAULT_DATAPACK_PATH))
        cls.files = {int(p.stem): tomllib.loads(p.read_text()) for p in (DATA / 'skills').glob('*.toml')}
        cls.ids, cls.blockers = GEN.compute_all_blockers(cls.xml, set(cls.files))

    def test_all_389_generated_skills_parse_and_match_generator(self):
        self.assertEqual(len(self.files), 389)
        for sid, parsed in self.files.items():
            with self.subTest(skill=sid):
                source, elem = self.xml[sid]
                generated = GEN.generate_skill_toml(sid, source, elem, self.blockers[sid], sid == 286)
                self.assertEqual(tomllib.loads(generated), parsed)
                self.assertEqual(parsed['blockers'], sorted(self.blockers[sid]))
                self.assertEqual(parsed['runtime_status'], 'deferred' if self.blockers[sid] else 'executable')

    def test_all_rank_tables_and_condition_sites_preserved(self):
        for sid, parsed in self.files.items():
            elem = self.xml[sid][1]
            tables = GEN.parse_tables(elem)
            levels = parsed['max_level']
            for source_set in elem.findall('set'):
                value = source_set.attrib['val']
                self.assertEqual(parsed['source_sets'][source_set.attrib['name']],
                                 tables[value] if value.startswith('#') else value)
            source_effects = [(sec, eff) for sec in elem if sec.tag.endswith('ffects') and not sec.tag.startswith('enchant')
                              for eff in sec.findall('effect')]
            self.assertEqual(len(source_effects), len(parsed['effects']))
            for (sec, eff), out in zip(source_effects, parsed['effects']):
                sites = [(eff, out), (sec, {'conditions': out.get('section_conditions', [])})]
                stats = [ch for ch in eff if ch.tag in GEN.ALL_STAT_OPS]
                self.assertEqual(len(stats), len(out['stat_children']))
                sites.extend(zip(stats, out['stat_children']))
                for node, result in sites:
                    expected = [GEN.parse_cond_node(c) for c in GEN.condition_nodes(node)]
                    self.assertEqual(result.get('conditions', []), expected, (sid, node.tag))
                    if parsed['runtime_status'] == 'executable':
                        for cond in GEN.condition_nodes(node):
                            self.assertTrue(GEN.is_cond_allowed(cond), (sid, cond.tag))
                for node, result in zip(stats, out['stat_children']):
                    self.assertEqual(result['value'], GEN.expand(node.get('val'), tables, levels))

    def test_nine_unsupported_stat_conditions_defer(self):
        for sid in [94, 148, 171, 225, 290, 291, 293, 294, 1297]:
            skill = self.files[sid]
            self.assertEqual(skill['runtime_status'], 'deferred', sid)
            self.assertTrue(any(b.startswith('cond:') for b in skill['blockers']), sid)
        clear_mind = self.files[1297]['effects'][0]['stat_children']
        self.assertEqual([ch['value'][0] for ch in clear_mind], ['3.2', '2.6'])
        self.assertEqual([ch['conditions'][0]['attrs'] for ch in clear_mind],
                         [{'moving': 'true'}, {'standing': 'true'}])
        mastery = self.files[217]
        self.assertEqual(mastery['runtime_status'], 'executable')
        self.assertEqual(mastery['effects'][0]['stat_children'][0]['conditions'],
                         [{'kind': 'using_kind', 'weapon_kinds': ['SWORD', 'BLUNT']}])

    def test_review_range_tables(self):
        for sid, low_ranks in [(1083, 3), (1223, 1), (1224, 3)]:
            skill = self.files[sid]
            self.assertEqual(skill['cast_range'], [750] * low_ranks + [900] * (skill['max_level'] - low_ranks), sid)
            self.assertEqual(skill['effect_range'], 1400 if sid == 1224 else
                             [1250] * low_ranks + [1400] * (skill['max_level'] - low_ranks), sid)
        skill = self.files[28]
        self.assertEqual(skill['cast_range'], [400] * 12 + [600] * 21 + [800] * 16)
        self.assertEqual(skill['effect_range'], [900] * 12 + [1100] * 21 + [1300] * 16)
        self.assertEqual(skill['runtime_status'], 'deferred')

    def test_source_defaults_and_locked_reuse(self):
        skill = self.files[1430]
        self.assertTrue(skill['reuse_delay_lock'])
        self.assertEqual(skill['reuse_ms'], 900000)
        for sid, parsed in self.files.items():
            sets = parsed['source_sets']
            for source_key, key, default in [('magicLvl', 'magic_level', [0] * parsed['max_level']),
                                              ('castRange', 'cast_range', -1), ('effectRange', 'effect_range', -1),
                                              ('minChance', 'min_chance', 10), ('maxChance', 'max_chance', 90)]:
                if source_key not in sets:
                    self.assertEqual(parsed[key], default, (sid, key))
            if 'abnormal' in parsed and 'abnormalLvl' not in sets:
                self.assertEqual(parsed['abnormal']['level'], [0] * parsed['max_level'])


class SyntheticAdmissionTests(unittest.TestCase):
    def skill(self, body):
        return ET.fromstring('<skill id="9999" levels="2" name="Probe"><set name="operateType" val="A2" />'
                             '<set name="targetType" val="ENEMY" />' + body + '</skill>')

    def blockers(self, elem):
        return GEN.compute_all_blockers({9999: ('probe.xml', elem)}, {9999})[1][9999]

    def test_recursive_toml_and_unknown_attributes_round_trip(self):
        elem = self.skill('<cond><and><using kind="SWORD"/><or><not><player hp="30" mystery="a &quot;quote&quot;"/>'
                          '</not><player canResurrect="false"/></or></and></cond>')
        out = GEN.generate_skill_toml(9999, 'probe.xml', elem, self.blockers(elem))
        parsed = tomllib.loads(out)
        node = parsed['conditions'][0]['conditions'][1]['conditions'][0]['conditions'][0]
        self.assertEqual(node['attrs'], {'hp': '30', 'mystery': 'a "quote"'})
        self.assertIn('[[conditions.conditions.conditions.conditions]]', out)
        self.assertIn('cond:player:hp,mystery', parsed['blockers'])

    def test_effect_stat_and_section_conditions_all_block(self):
        for body in ['<effects><effect name="Buff"><cond><player moving="true"/></cond></effect></effects>',
                     '<effects><effect name="Buff"><add stat="regMp" val="1"><cond><player moving="true"/>'
                     '</cond></add></effect></effects>',
                     '<effects><cond><player moving="true"/></cond><effect name="Buff"/></effects>']:
            elem = self.skill(body)
            self.assertIn('cond:player:moving', self.blockers(elem))
            out = tomllib.loads(GEN.generate_skill_toml(9999, 'probe.xml', elem, self.blockers(elem)))
            self.assertEqual(out['runtime_status'], 'deferred')

    def test_unknown_parameters_values_ops_and_filters_defer(self):
        elem = self.skill('<set name="affectObject" val="MYSTERY"/><effects><effect name="TickHp">'
                          '<param power="not-a-number" ticks="0" mode="BROKEN" mystery="1"/></effect>'
                          '<effect name="Buff"><div stat="pAtk" val="2"/></effect></effects>')
        self.assertEqual(self.blockers(elem), {'filter:MYSTERY', 'param:TickHp.mode', 'param:TickHp.mystery',
                                              'param:TickHp.power', 'param:TickHp.ticks', 'op:div'})
        for op in ['set', 'share']:
            self.assertIn('op:' + op, self.blockers(self.skill(f'<effects><effect name="Buff"><{op} stat="pAtk" val="1"/>'
                                                            '</effect></effects>')))
        elem = self.skill('<effects><effect name="PhysicalAttack"><param ignoreShieldDefence="MAYBE"/></effect></effects>')
        self.assertIn('param:PhysicalAttack.ignoreShieldDefence', self.blockers(elem))

    def test_every_numeric_and_string_table_expands(self):
        numeric = ['castRange', 'effectRange', 'affectRange', 'hitTime', 'coolTime', 'reuseDelay', 'hitCancelTime',
                   'mpConsume1', 'mpConsume2', 'hpConsume', 'magicLvl', 'effectPoint', 'abnormalTime', 'abnormalLvl',
                   'activateRate', 'attributePower', 'lvlBonusRate', 'minChance', 'maxChance', 'isMagic']
        elem = self.skill('<table name="#numbers">1 2</table>' + ''.join(f'<set name="{k}" val="#numbers"/>' for k in numeric)
                          + '<effects><effect name="PhysicalAttack"><param power="#numbers"/></effect></effects>')
        out = tomllib.loads(GEN.generate_skill_toml(9999, 'probe.xml', elem, self.blockers(elem)))
        for key in ['cast_range', 'effect_range', 'affect_range', 'hit_time_ms', 'cool_time_ms', 'reuse_ms', 'hit_cancel_ms',
                    'mp_consume_start', 'mp_consume_end', 'hp_consume', 'magic_level', 'effect_point', 'activate_rate',
                    'level_bonus_rate', 'min_chance', 'max_chance', 'is_magic']:
            self.assertEqual(out[key], [1, 2], key)
        self.assertEqual(out['abnormal']['level'], [1, 2])
        self.assertEqual(out['abnormal']['time_s'], [1, 2])
        self.assertEqual(out['attribute']['power'], [1, 2])
        self.assertEqual(out['effects'][0]['params']['power'], ['1', '2'])

    def test_missing_and_wrong_length_tables_are_errors(self):
        for body in ['<set name="castRange" val="#missing"/>', '<set name="unusedField" val="#missing"/>',
                     '<effects><effect name="Buff"><param mystery="#missing"/></effect></effects>',
                     '<effects><effect name="Buff"><add stat="pAtk" val="#missing"/></effect></effects>']:
            with self.assertRaisesRegex(GEN.SourceError, 'unresolved table reference'):
                GEN.generate_skill_toml(9999, 'probe.xml', self.skill(body), set())
        with self.assertRaisesRegex(GEN.SourceError, 'expected 2'):
            GEN.generate_skill_toml(9999, 'probe.xml', self.skill('<table name="#short">1</table><set name="castRange" val="#short"/>'), set())
        with self.assertRaises(ValueError):
            GEN.generate_skill_toml(9999, 'probe.xml', self.skill('<set name="hitTime" val="oops"/>'), set())


if __name__ == '__main__':
    unittest.main()
