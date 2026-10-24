"""Independent source and adversarial gates for data-review-1 findings 1–8."""
from copy import deepcopy
from fractions import Fraction as F
import inspect
import json
from pathlib import Path
from types import SimpleNamespace
import tomllib
import unittest
import xml.etree.ElementTree as ET

from packages.data.scripts import gen_items as gen, gen_skills as skills, item_oracle as oracle


class RevisionMathTests(unittest.TestCase):
    def test_magic_source_branches_and_precedence(self):
        args=dict(power=100,m_atk=100,m_def=100,blessed=True,critical=True)
        self.assertEqual(oracle.magic_damage(**args)['damage'],5460)
        self.assertEqual(oracle.magic_damage(**args,target_player=True)['damage'],4550)
        self.assertEqual(oracle.magic_damage(**args,spirit=True,target_player=True)['damage'],4550)
        self.assertEqual(oracle.magic_damage(**args,is_pvp=True,attacker_player=False,target_player=True)['damage'],5460)
        failure=dict(magic_failures=True,first_success=False,second_success=True)
        self.assertEqual(oracle.magic_damage(**args,**failure,level_difference=9)['damage'],910)
        self.assertEqual(oracle.magic_damage(**args,**failure,level_difference=10)['damage'],1)
        failure['second_success']=False
        self.assertEqual(oracle.magic_damage(**args,**failure,spread=2,attribute=3,npc=4)['damage'],24)
        self.assertEqual(oracle.magic_damage(**args,attacker_player=False,**failure)['damage'],1820)
        self.assertEqual(oracle.magic_damage(100,100,0,perfect=True,magic_failures=True,first_success=False)['damage'],1)
        with self.assertRaisesRegex(ValueError,'second magic'):oracle.magic_damage(100,100,100,magic_failures=True,first_success=False)
        self.assertEqual(oracle.magic_damage(**args,target_player=True,pvp_magic_def=2,pvp_magic_damage=3,magic_crit_damage=2)['damage'],13650)
        self.assertEqual(oracle.magic_damage(**args,target_player=True,skill_is_magic=False,pvp_magic_def=999,pvp_skill_def=2,pvp_skill_damage=3)['damage'],6825)

    def test_healing_q_remainder_and_clamp(self):
        hp=remainder=0
        for step in range(3):
            result=oracle.heal(100,121,current_hp=hp,remainder_q=remainder,recoverable_hp=1000)
            self.assertEqual(result['restoration_q'],115556349)
            hp,remainder=result['current_hp'],result['remainder_q']
            self.assertEqual(hp*oracle.Q+remainder,(step+1)*115556349)
        partial=oracle.heal(100,121,current_hp=950,remainder_q=500000,recoverable_hp=1000)
        self.assertEqual((partial['applied_q'],partial['current_hp'],partial['remainder_q']),(49500000,1000,0))
        self.assertEqual(oracle.heal(100,121,current_hp=1000,recoverable_hp=1000)['applied_q'],0)
        static=oracle.heal('1.25',121,static=True,blessed=True,heal_effect=5,critical=True)
        self.assertEqual((static['restoration_q'],static['current_hp'],static['remainder_q']),(1250000,1,250000))
        self.assertEqual(oracle.heal(-1,121,static=True)['applied_q'],0)
        with self.assertRaises(ValueError):oracle.heal(100,121,remainder_q=oracle.Q)

    def test_shot_payments_draws_and_bundles(self):
        args=dict(charges=[],kind='soul',count=10,available=7,weapon_grade='S84',shot_grade='S',toggle=True,reduction=[59,7])
        reduced=oracle.shot_payment(**args,residue=58)
        self.assertEqual((reduced['consumed'],reduced['charges'],reduced['toggle']),(7,['soul'],True))
        failed=oracle.shot_payment(**args,residue=59)
        self.assertEqual((failed['consumed'],failed['toggle'],len(failed['draws'])),(0,False,1))
        wrong=dict(args,shot_grade='D')
        self.assertEqual((oracle.shot_payment(**wrong)['toggle'],oracle.shot_payment(**wrong)['draws']),(True,[]))
        for hits in [[False,False],[False,True],[True,False],[True,True]]:
            result=oracle.shared_shot_bundle(['soul'],hits,use_soul=True)
            self.assertEqual(result['payments'],0)
            self.assertEqual(result['after'],[] if any(hits) else ['soul'])
        self.assertEqual(oracle.shared_shot_bundle(['soul'],[True],use_soul=True,canceled=True)['after'],['soul'])
        self.assertEqual(oracle.shared_shot_bundle(['soul'],[False],unequip=True)['after'],[])
        precedence=oracle.shared_shot_bundle(['spirit','blessed'],[True,True,True],use_spirit=True)
        self.assertEqual(precedence['after'],['spirit'])
        self.assertEqual(precedence['recipient_charges'],[['blessed','spirit']]*3)


class RevisionSourceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src=oracle.Source()
        cls.human=oracle.D+'stats/chars/baseStats/HumanFighter.xml'

    def test_all_shot_actions_source_override_literal_preserved(self):
        expected={'SOULSHOT','SPIRITSHOT','SUMMON_SOULSHOT','SUMMON_SPIRITSHOT'}
        seen=set();count=0
        for sid,(_,_,node) in self.src.items.items():
            p=self.src.properties(sid)
            if node.get('type')!='EtcItem' or p.get('default_action') not in expected:continue
            raw=gen.xml(self.src.dp/self.src.items[sid][0])
            actual=next(n for n in gen.children(raw,'item') if gen.attrs(n)['id']==str(sid))
            row=gen.project_item(actual,self.src.items[sid][0],{'none':0})
            self.assertEqual(row['etcitem_type'],'SHOT',sid)
            self.assertEqual(row['properties'],p,sid)
            seen.add(p['default_action']);count+=1
        self.assertEqual(seen,expected)
        for sid in [1463,1467,2509,3947,6645,6646,6647]:
            self.assertIn(self.src.properties(sid)['default_action'],expected)
        self.assertGreaterEqual(count,50)

    def test_effective_attribute_provider_and_same_priority_order(self):
        sheet=oracle.gear_stats(self.src,self.human,1,[(1,0,'rhand',1)],[(8498,1,2)])
        self.assertEqual(sheet['base_stats']['STR'],42)
        self.assertEqual(sheet['exact']['pAtk'],F(1161,125))
        self.assertNotIn('STR',sheet['exact'])
        # Explicit priority-1 SET operand tests the published sequence contract.
        proxy=SimpleNamespace(**vars(self.src));proxy.xml=self.src.xml;proxy.properties=self.src.properties
        proxy.skills=dict(self.src.skills)
        file,node=self.src.skills[8498];node=deepcopy(node)
        stat=node.find('effects/effect/add');stat.tag='set';stat.set('order','1');stat.set('val','40')
        proxy.skills[8498]=(file,node)
        before=oracle.gear_stats(proxy,self.human,1,[(1,0,'rhand',1)],[(8498,1,2)],{'str':2},base_delta_sequence=1)
        after=oracle.gear_stats(proxy,self.human,1,[(1,0,'rhand',1)],[(8498,1,2)],{'str':2},base_delta_sequence=3)
        self.assertEqual((before['base_stats']['STR'],after['base_stats']['STR']),(40,42))
        # D97: an NPC-flagged template (8205) is ordinary equipment for the stat fold.
        oracle.gear_stats(self.src,self.human,1,[(8205,0,'rhand',1)])
        stat.set('stat','unknown_stat')
        with self.assertRaisesRegex(ValueError,'unsupported oracle stat'):oracle.gear_stats(proxy,self.human,1,[],[(8498,1,2)])
        stat.set('stat','STR');stat.append(ET.fromstring('<player castle="1"/>'))
        with self.assertRaisesRegex(ValueError,'unsupported oracle fold condition'):oracle.gear_stats(proxy,self.human,1,[],[(8498,1,2)])

    def test_elixir_rank_expansion_boundaries_and_d34_promotion(self):
        caps=tomllib.loads((gen.ROOT/'tables-phase4/capabilities.toml').read_text())
        by_id={row['template_id']:row for row in caps['items']}
        elixirs=[]
        for sid in [2287,2288,2289,2860,2861,2862]:
            file,skill=self.src.skills[sid];table=skills.parse_tables(skill)
            self.assertTrue(next(r for r in caps['skills'] if r['id']==sid)['admitted'])
            parsed=tomllib.loads(skills.generate_skill_toml(sid,file,skill,set(),phase4=True))
            condition=parsed['conditions'][0]
            self.assertEqual(condition['kind'],'player_level_range')
            self.assertEqual(len(condition['ranges']),6)
            self.assertEqual(parsed['runtime_status'],'executable')
            for rank,bounds in enumerate(condition['ranges'],1):
                lo,hi=bounds['minimum'],bounds['maximum']
                node=ET.fromstring(f'<player levelRange="{lo};{hi}"/>')
                self.assertEqual([oracle.condition_matches(node,{'level':level}) for level in [lo-1,lo,hi,hi+1]],[False,True,True,False])
                self.assertEqual(table['#levelRange'][rank-1],f'{lo};{hi}')
                self.assertEqual([oracle.player_level_range(condition,rank,level) for level in [lo-1,lo,hi,hi+1]],[False,True,True,False])
            for bad_rank in [0,7,True]:
                with self.assertRaises(ValueError):oracle.player_level_range(condition,bad_rank,1)
            for item in self.src.items:
                if any(ref.split('-')[0]==str(sid) for ref in self.src.properties(item).get('item_skill','').split(';')):
                    elixirs.append(item);self.assertTrue(by_id[item]['admitted'])
        self.assertEqual(len(elixirs),36)
        with self.assertRaises(skills.SourceError):skills.parse_cond_node(ET.fromstring('<player levelRange="20;1"/>'),True)

    def test_consume_once_shared_reuse_and_simultaneous_lane(self):
        definition=oracle.consumable_definition(self.src,736)
        result=oracle.consumable_use(definition,10,{736:1},condition_context={'canEscape':True,'inside_zone_ids':[]})
        self.assertEqual(result['consumed'],{736:1})
        self.assertEqual(result['lanes'],['main'])
        potion=oracle.consumable_definition(self.src,1539)
        result=oracle.consumable_use(potion,10,{1539:2},main_casting=True)
        self.assertTrue(result['main_cast_preserved'])
        self.assertEqual(result['lanes'],['item'])
        # Source item template/shared group, independent of stack identity.
        self.assertTrue(result['item_deadlines'])
        reused=oracle.consumable_use(potion,11,{1539:1},result['item_deadlines'],result['skill_deadlines'],True)
        self.assertFalse(reused['accepted']);self.assertEqual(reused['consumed'],{})
        self.assertEqual(oracle.consumable_use(potion,max(result['item_deadlines'].values()),{1539:1})['accepted'],True)

    def test_source_vector_coverage_mutation_and_player_pole(self):
        vectors=oracle.make_vectors(self.src)
        self.assertGreater(oracle.verify_coverage(self.src,vectors)['covered_scroll_group_levels'],65536)
        for name in ['provider-overlap-2','consumable-same-tick-lanes','shot-payment-soul-True-wrong-grade']:
            with self.assertRaisesRegex(ValueError,'missing vectors'):
                oracle.verify_coverage(self.src,[v for v in vectors if v['id']!=name])
        first=next(v for v in vectors if v['id'].startswith('scroll-binding-'))
        with self.assertRaisesRegex(ValueError,'coverage'):
            oracle.verify_coverage(self.src,[v for v in vectors if v is not first])
        row=next(v for v in vectors if v['id']=='gear-07-POLE')
        self.assertEqual(row['inputs']['equipment'][0][0],15)
        self.assertEqual(row['expected']['getters']['atkCountMax'],5)


class IndependentAngleRevisionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):cls.table=tomllib.loads((gen.ROOT/'tables-phase4/angle_table.toml').read_text())

    def test_independent_validation_and_exact_value_mutations(self):
        self.assertNotIn('gen_item_angles',inspect.getsource(oracle))
        self.assertTrue(oracle.verify_angle_table(self.table))
        for degree in oracle.EXACT_ANGLES:
            changed=deepcopy(self.table)
            changed['angles'][degree]['cos2_lower']=changed['angles'][degree]['cos2_upper']='1/3'
            with self.assertRaisesRegex(ValueError,'exact angle'):oracle.verify_angle_table(changed)

    def test_certificate_coordinate_enclosure_and_separation_mutations(self):
        for field,value in [('coordinate_bound_millitiles',0),('coordinate_bound_millitiles',256001)]:
            changed=deepcopy(self.table);changed[field]=value
            with self.assertRaisesRegex(ValueError,'coordinate bound'):oracle.verify_angle_table(changed)
        for field,value in [('certified',False),('blockers',['bad']),('cos_lower','1'),('cos2_upper','0'),('cos2_lower','0')]:
            changed=deepcopy(self.table);changed['angles'][1][field]=value
            with self.assertRaises(ValueError):oracle.verify_angle_table(changed)
