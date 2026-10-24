"""Phase 4 contract, adversarial formula boundaries and source closure tests."""
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import tempfile
import tomllib
import unittest
import xml.etree.ElementTree as ET
from packages.data.scripts import gen_items as gen, gen_item_angles as angles, gen_skills as skills, item_oracle as oracle


class ItemMathTests(unittest.TestCase):
    def test_d17_bow_two_conversions_and_reuse(self):
        self.assertEqual([oracle.bow_damage(100,x,0)['damage'] for x in [12499,12500,12501]],[89,90,90])
        self.assertEqual(oracle.bow_damage(F(10199,100),12500,0)['damage'],90)
        self.assertEqual(oracle.bow_damage(100,7500,10000)['damage'],90)
        self.assertEqual(oracle.bow_reuse(1500,400,1),1248)
        self.assertEqual(oracle.bow_reuse(1500,400,2),1248)

    def test_d24_fist_paths_and_endpoints(self):
        for level,radius in [(1,6),(3,6),(4,7),(8,7),(9,8),(15,8),(16,9)]:
            for path in ['unequip','level_change','cold_restore']:
                with self.subTest(level=level,path=path):
                    self.assertEqual(oracle.fist_spread(level)['radius'],radius)
                    self.assertEqual(oracle.fist_spread(level)['bound'],2*radius+1)
                    self.assertEqual(oracle.fist_spread(level,0)['offset'],-radius)
                    self.assertEqual(oracle.fist_spread(level,2*radius)['offset'],radius)
            self.assertEqual(oracle.fist_spread(level,20,10)['offset'],10)

    def test_d25_pole_order_minimum_and_miss(self):
        for formula,percent,expected in [(F(499,10),85,41),(F(999,50),85,16),(20,85,17),(F(1001,50),85,17),(1,85,0),(F(499,10),F(1700,23),36)]:
            self.assertEqual(oracle.pole_damage(formula,percent)['damage'],expected)
        self.assertEqual(oracle.pole_damage(F(499,10),primary=True)['damage'],49)
        self.assertEqual(oracle.pole_damage(1,primary=True)['damage'],1)
        result=oracle.pole_sequence([{'eligible':False,'hit':True,'formula':100},{'eligible':True,'hit':False,'formula':100},{'eligible':True,'hit':True,'formula':F(499,10)}],1)
        self.assertEqual(len(result),2) # source break is count>max
        self.assertEqual(result[0]['draws'],[])
        self.assertEqual(result[1]['percent'],F(1700,23))
        self.assertEqual(result[1]['damage'],36)

    def test_dual_caller_minimum_skip_and_integer_deadlines(self):
        self.assertEqual(oracle.dual_damage(1)['damage'],0)
        self.assertEqual(oracle.dual_damage(F(499,10))['damage'],24)
        self.assertEqual(oracle.dual_damage(100,True)['draws'],[])
        self.assertEqual(oracle.dual_timing(400),{'attack_ms':1250,'time_to_hit_ms':625,'first_ms':312,'second_ms':625,'first_ticks':4,'second_ticks':7})
        self.assertEqual(oracle.skill_caller_damage(1,True),2)

    def test_enchant_bodypart_and_hp_cap(self):
        self.assertEqual(oracle.enchant_increment('pAtk','C','lrhand','POLE',4),20)
        self.assertEqual(oracle.enchant_increment('pAtk','C','rhand','POLE',4),15)
        self.assertEqual(oracle.enchant_increment('pAtk','S84','lrhand','BOW',4),50)
        self.assertEqual(oracle.enchant_increment('mAtk','S80','lrhand','POLE',4),20)
        self.assertEqual(oracle.enchant_increment('sDef','S','lhand','NONE',6),0)
        self.assertEqual(oracle.enchant_hp([0,0,0,9],4294967295,True),13)
        self.assertEqual(oracle.enchant_hp([0,0,0,9],0),0)

    def test_enchant_lattice_exhaustion_and_base_zero(self):
        for chance in [F(0),F('66.67'),F(100)]:
            threshold=oracle.enchant_chance(chance)
            self.assertEqual(sum(F(r,1000)<chance for r in range(100000)),threshold)
        self.assertEqual(oracle.enchant_outcome(0,100,0,0,20)['enchant'],20)
        self.assertTrue(oracle.enchant_valid(16,16))
        self.assertFalse(oracle.enchant_valid(17,16))
        self.assertFalse(oracle.enchant_valid(3,16,True,True))
        self.assertEqual(oracle.failure_refund(101,90,4,True),500)
        self.assertEqual(oracle.failure_refund(101,90,4,True,False),0)

    def test_weight_signed_truncation_history_and_grade_quirk(self):
        self.assertEqual(oracle.overload(0,69000)['per_mille'],0)
        self.assertEqual(oracle.overload(100000,100000,0,3,True)['overloaded'],False)
        self.assertEqual(oracle.overload(100001,100000,0,4,False)['overloaded'],False)
        self.assertEqual(oracle.overload(99999,100000,0,4,True)['overloaded'],False) # rank changes to3
        self.assertEqual(oracle.overload(100000,100000,0,4,True)['overloaded'],True)
        self.assertEqual(oracle.overload(100,0,1,3,True)['level'],3)
        self.assertEqual(oracle.grade_penalties([{'instance_id':1,'grade':7,'weapon':True},{'instance_id':2,'grade':6,'weapon':True}],3),{'weapon':4,'armor':3})

    def test_static_and_non_magic_cast_timing(self):
        self.assertEqual(oracle.cast_time(20000,1000,spirit=True,static=True),20000)
        self.assertEqual(oracle.cast_time(0,1000,static=True),0)
        self.assertEqual(oracle.cast_time(2000,1000,spirit=True,is_magic=False),666)

    def test_shield_draw_order_skip_and_shot_callers(self):
        self.assertEqual(oracle.shield(20,perfect_residue=91)['outcome'],'perfect')
        self.assertEqual(len(oracle.shield(20,perfect_residue=91)['draws']),1)
        self.assertEqual(oracle.shield(20,perfect_residue=90,normal_residue=20)['outcome'],'none')
        self.assertEqual(oracle.shield(20,bow=True,perfect_residue=90,normal_residue=25)['outcome'],'normal')
        for kwargs in [{'eligible':False},{'facing':False},{'sigil':True}]:self.assertEqual(oracle.shield(20,**kwargs)['draws'],[])
        self.assertEqual(oracle.physical_damage(100,100,caller='auto',shot=True),152)
        self.assertEqual(oracle.physical_damage(100,100,caller='blow',shot=True),F(112266,1000))
        self.assertEqual(oracle.physical_damage(100,100,shield_def=100),38)
        self.assertEqual(oracle.physical_damage(100,100,perfect=True),1)
        self.assertEqual(oracle.charge_transition(['soul','spirit','blessed'],True,True),['soul','spirit'])
        self.assertEqual(oracle.charge_transition(['soul'],False,True,False),['soul'])

    def test_retail_descending_tail_all_residues_and_unsigned_pack(self):
        rows=[(1,F('55.00')),(2,F('35.00')),(3,F('7')),(4,F('3'))]
        actual={}
        for residue in range(10000):
            option=oracle.retail_select(rows,residue);actual[option]=actual.get(option,0)+1
        self.assertEqual(oracle.retail_distribution(rows),actual)
        self.assertEqual(oracle.retail_select(rows,9999),1)
        self.assertEqual(oracle.retail_select(rows,0),4)
        self.assertEqual(oracle.retail_distribution([(1,F('99.999'))]),{1:10000})
        self.assertEqual(oracle.category(9900,True),1)
        self.assertEqual(oracle.category(9900,False),3)
        self.assertEqual(oracle.pack_options(65535,65535),4294967295)
        for bad in [0,65536]:
            with self.assertRaises(ValueError):oracle.pack_options(bad,1)

    def test_actual_input_gate_rejects_wrong_and_missing_evidence(self):
        facts=[{'formula':'pole_damage','inputs':{'formula':{'numerator':499,'denominator':10},'percent':85},
            'expected':{'base':49,'percent':{'numerator':85,'denominator':1},'damage':41}}]
        self.assertEqual(oracle.verify_facts(facts),1)
        facts[0]['expected']['damage']=42
        with self.assertRaisesRegex(ValueError,'mismatch'):oracle.verify_facts(facts)
        with self.assertRaises(ValueError):oracle.verify_facts([])
        facts[0]['formula']='unknown'
        with self.assertRaises(ValueError):oracle.verify_facts(facts)

    def test_allocator_and_d20_history(self):
        self.assertFalse(oracle.validate_allocator(1,2**32,2**32))
        self.assertTrue(oracle.validate_allocator(2**32,2**32,2**33))
        with self.assertRaises(ValueError):oracle.validate_allocator(1,1,2**63)
        self.assertTrue(oracle.set_shield_history(False,'equip_shield',True,True))
        self.assertFalse(oracle.set_shield_history(False,'equip_chest',True,True))
        self.assertFalse(oracle.set_shield_history(True,'remove',True,True,required_part=True))
        self.assertEqual(oracle.clamp_no_heal(100,200),100)
        with self.assertRaises(ValueError):oracle.exact(0.1)


class SourceContractTests(unittest.TestCase):
    def test_xml_live_lines_duplicates_and_defaults(self):
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/'source.xml';path.write_text('<list>\n<!-- <item id="999"/> -->\n<item id="1" type="Weapon" name="Test">\n<set name="weight" val="1"/>\n<set name="weight" val="2"/>\n<for><set stat="pAtk" val="1.25"/><enchant stat="pAtk" val="0"/></for>\n</item></list>')
            row=gen.project_item(gen.children(gen.xml(path),'item')[0],'source.xml',{'none':0})
            self.assertEqual(row['source']['lines'],'3')
            self.assertEqual(row['weight'],2)
            self.assertEqual(row['attack_range'],40)
            self.assertEqual([f['order'] for f in row['functions']],[0,0])
            self.assertEqual(row['functions'][0]['value'],'1.25')
            self.assertEqual(len(gen.children(row['raw'],'set')),2)

    def test_toml_roundtrip_hash_semantics_and_unknown_conditions(self):
        data={'schema':1,'sources':[],'rows':[{'value':'1.00','source':{'lines':'1'},'raw':{'tag':'node','line':4,'attributes':{'chance':'50.00'}}}]}
        self.assertEqual(tomllib.loads(gen.encode_toml(data)),data)
        changed=json.loads(json.dumps(data));changed['rows'][0]['value']='1';changed['rows'][0]['source']['lines']='99';changed['rows'][0]['raw']['line']=100
        self.assertEqual(gen.digest(data),gen.digest(changed))
        changed['rows'][0]['raw']['attributes']['chance']='49'
        self.assertNotEqual(gen.digest(data),gen.digest(changed))
        self.assertEqual(gen.condition_blockers({'tag':'player','attributes':{'castle':'1'},'children':[]}),['condition:player:castle'])
        with self.assertRaises(gen.SourceError):gen.canonical({'float':0.5})

    def test_v5_skill_capabilities_do_not_change(self):
        skill=ET.fromstring('<skill id="1" levels="1"><set name="operateType" val="P"/><set name="targetType" val="SELF"/><effects><effect name="Buff"><mul stat="weightLimit" val="1.2"><using kind="HEAVY"/></mul></effect></effects></skill>')
        _,old=skills.compute_all_blockers({1:('x.xml',skill)},{1})
        _,new=skills.compute_all_blockers({1:('x.xml',skill)},{1},phase4=True)
        self.assertEqual(old[1],{'stat:weightLimit','cond:using_kind:HEAVY'})
        self.assertEqual(new[1],set())
        node=ET.fromstring('<player weight="80"/>')
        self.assertEqual(skills.parse_cond_node(node,True),{'kind':'player_weight','value':80})
        self.assertEqual(skills.parse_cond_node(node)['kind'],'unsupported')

    def test_committed_closure_counts_and_pinned_kits(self):
        directory=gen.ROOT/'tables-phase4'
        counts=json.loads((directory/'capability_counts.json').read_text())
        self.assertEqual({kind:row['source_total'] for kind,row in counts['items'].items()},{'Weapon':3893,'Armor':3825,'EtcItem':11481})
        for row in counts['items'].values():self.assertEqual(row['imported'],row['admitted']+row['deferred'])
        caps=tomllib.loads((directory/'capabilities.toml').read_text());skills_by_id={s['id']:s for s in caps['skills']}
        for sid in [4270,6209,6213,3599]:self.assertTrue(skills_by_id[sid]['admitted'],sid)
        self.assertEqual(skills_by_id[286]['blockers'],['sample:out_of_scope'])
        for row in caps['items']:
            if not row['admitted']:self.assertTrue(row['blockers'])
        for row in caps['items']:
            if set(row['blockers']) & {'context:common','context:shadow','context:time_limited','context:pet'}:self.assertFalse(row['storable'])
        by_id={r['template_id']:r for r in caps['items']}
        for sid in [5588,10650,12753]:self.assertTrue(by_id[sid]['storable'])
        for sid in [729,955,6569,12362,72,79,4682,4717]:self.assertTrue(by_id[sid]['admitted'],sid)
        admitted={r['template_id'] for r in caps['items'] if r['admitted']}
        for name in ['weapons','armors','etc_items']:
            for row in tomllib.loads((directory/(name+'.toml')).read_text())['items']:
                if row['template_id'] not in admitted:continue
                for ref in row['skills']+row['enchant4_skills']:
                    self.assertTrue(skills_by_id[ref['id']]['admitted'])
                    self.assertLessEqual(ref['level'],skills_by_id[ref['id']]['max_level'])
        recipes=tomllib.loads((directory/'sa_variants.toml').read_text())['recipes']
        recipe_caps={r['recipe_id']:r for r in caps['recipes']}
        for row in recipes:
            if row['base_template'] in {72,79} and any(p['template_id'] in {4682,4717} for p in row['products']):self.assertTrue(recipe_caps[row['recipe_id']]['admitted'])
        kits=tomllib.loads((directory/'starter_kits.toml').read_text())['kits']
        self.assertEqual(len(kits),11)
        mystic=next(r for r in kits if r['class_id']==10)
        self.assertEqual([r['template_id'] for r in mystic['items'] if r['equipped']],[6,425,461])
        self.assertEqual([(r['template_id'],r['count']) for r in mystic['items'] if not r['equipped']],[(5588,1),(10650,5),(12753,10)])


class AngleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):cls.table=angles.generate_angles()
    def test_every_integer_angle_certified(self):
        bound=self.table['coordinate_bound_millitiles'];maximum=4*bound**4
        for row in self.table['angles']:
            self.assertTrue(row['certified'],row['degrees'])
            lo,hi=F(row['cos2_lower']),F(row['cos2_upper'])
            if lo!=hi:
                nearest=((lo+hi)/2).limit_denominator(maximum)
                self.assertFalse(lo<=nearest<=hi)
    def test_inclusive_boundaries_and_domain(self):
        for degrees,relative in [(0,(1,0)),(90,(1,1)),(180,(0,1)),(270,(-1,1)),(360,(-1,0))]:
            self.assertTrue(oracle.angle_contains((1,0),relative,degrees,self.table))
        self.assertFalse(oracle.angle_contains((1,0),(1,2),120,self.table))
        self.assertTrue(oracle.angle_contains((1,0),(1,1),120,self.table))
        self.assertFalse(oracle.angle_contains((1,0),(-1,0),359,self.table))
        with self.assertRaises(ValueError):oracle.angle_contains((1,0),(256001,0),120,self.table)
        table=json.loads(json.dumps(self.table));table['angles'][120]['certified']=False
        with self.assertRaises(ValueError):oracle.angle_contains((1,0),(1,0),120,table)


if __name__=='__main__':unittest.main()
