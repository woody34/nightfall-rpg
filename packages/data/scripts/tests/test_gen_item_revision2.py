"""Typed source projection and deferred V6 condition contract regressions."""
from pathlib import Path
import tempfile
import tomllib
import unittest
import xml.etree.ElementTree as ET
from packages.data.scripts import gen_items as gen, gen_skills as skills


class GeneratorRevisionTests(unittest.TestCase):
    def test_java_shot_override_does_not_rewrite_literal_xml(self):
        for action in ['SOULSHOT','SPIRITSHOT','SUMMON_SOULSHOT','SUMMON_SPIRITSHOT']:
            with tempfile.TemporaryDirectory() as temp:
                path=Path(temp)/'item.xml'
                path.write_text(f'<list><item id="1" name="Shot" type="EtcItem"><set name="default_action" val="{action}"/><set name="etcitem_type" val="POTION"/></item></list>')
                node=gen.children(gen.xml(path),'item')[0]
                row=gen.project_item(node,'item.xml',{'none':0})
                self.assertEqual(row['etcitem_type'],'SHOT')
                self.assertEqual(row['properties']['etcitem_type'],'POTION')
                self.assertEqual(gen.attrs(gen.children(row['raw'],'set')[1])['val'],'POTION')

    def test_v6_expansion_and_deferred_runtime_contract(self):
        node=ET.fromstring('<skill id="1" levels="2"><table name="#r">1;19 20;39</table><set name="operateType" val="A1"/><set name="targetType" val="SELF"/><cond><player levelRange="#r"/></cond></skill>')
        cond=node.find('cond/player')
        expected={'kind':'player_level_range','ranges':[{'minimum':1,'maximum':19},{'minimum':20,'maximum':39}]}
        self.assertEqual(skills.parse_cond_node(cond,True,skills.parse_tables(node),2),expected)
        self.assertEqual(skills.parse_cond_node(cond)['kind'],'unsupported')
        self.assertEqual(skills.get_cond_blockers(cond,True),{'cond:player:levelRange'})
        parsed=tomllib.loads(skills.generate_skill_toml(1,'source.xml',node,{'cond:player:levelRange'},phase4=True))
        self.assertEqual(parsed['conditions'],[expected])
        self.assertEqual(parsed['runtime_status'],'deferred')
        for value in ['39;20','1','1;2;3','bad']:
            with self.assertRaises(skills.SourceError):skills.parse_cond_node(ET.fromstring(f'<player levelRange="{value}"/>'),True)

    def test_required_elixir_counts_remain_closed(self):
        counts=__import__('json').loads((gen.ROOT/'tables-phase4/capability_counts.json').read_text())
        pending=counts['required_condition_closure']
        self.assertEqual((pending['imported'],pending['admitted'],pending['deferred']),(36,0,36))
        self.assertEqual(len(pending['template_ids']),36)
        self.assertEqual(pending['runtime_support'],'pending_zone_publication')
