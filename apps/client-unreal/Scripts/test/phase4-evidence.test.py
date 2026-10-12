#!/usr/bin/env python3
"""Synthetic exporter validation and independent actual-input Fraction gate tests."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPTS=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('evidence4',SCRIPTS/'phase4-evidence.py')
gate=importlib.util.module_from_spec(spec);spec.loader.exec_module(gate)
OWNER='01970000-0000-7000-8000-000000000040';OTHER='01970000-0000-7000-8000-000000000041';HASH='a'*64


def item(identity,count,template=17,slot=None):
    return dict(id=identity,count=count,template_id=template,slot=slot,location='paperdoll' if slot else 'inventory',enchant=0,augment_id=None,charges=0,created_tick=0,revision=0)


class Evidence(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.recording=Path(self.tmp.name)/'synthetic.nfr';self.recording.write_bytes(b'synthetic test recording')
        self.rows=[dict(kind='manifest',schema_version=1,scope='whole_recording',recording_sha256=gate.digest(self.recording),
                        zone=1,epoch=1,snapshot_tick=0,watermark=dict(zone=1,epoch=1,records=1,last_tick=0,reason='shutdown'),
                        phase4_rules_hash=HASH,snapshot_schema=10,record_schema=8,digest_version='BinaryV6'),
                   dict(kind='item_formula',entity=OWNER,incarnation=1,tick=0,sequence=0,
                        data=dict(formula='pole_damage',inputs=dict(formula='499/10',percent=85,primary=False),
                                  actual=dict(base=49,percent=dict(numerator=85,denominator=1),damage=41),resolution_id='attack:1:recipient:2',baseline=False)),
                   dict(kind='complete',facts=1,ticks=1,last_tick=0)]

    def repair(self, rows):
        for index,row in enumerate(rows[1:-1]):row['sequence']=index
        rows[-1]['facts']=len(rows)-2
        return rows

    def enchant_rows(self,base=100,residue=0,enchant=0,blessed=False,safe=False,entity=OWNER,target_id=8,scroll_id=7,support=False):
        rows=[copy.deepcopy(self.rows[0])]
        target=item(target_id,1,71);target['enchant']=enchant
        scroll=item(scroll_id,1,6573 if blessed else 22227 if safe else 951)
        support_item=item(9,1,12363) if support else None
        inputs=dict(base=base,bonus=0,support=18 if support else 0,residue=residue,enchant=enchant,blessed=blessed,safe=safe)
        result=gate.oracle.enchant_outcome(**inputs)
        def append(kind,data):
            data['resolution_id']='enchant:1'
            rows.append(dict(kind=kind,entity=entity,incarnation=1,tick=0,sequence=len(rows)-1,data=data))
        append('item_formula',dict(formula='enchant_outcome',inputs=inputs,actual=result,baseline=False,
               operation=dict(target_before=target,scroll_before=scroll,support_before=support_item)))
        chance_inputs={k:inputs[k] for k in ('base','bonus','support')}
        append('item_formula',dict(formula='enchant_chance',inputs=chance_inputs,actual=gate.oracle.enchant_chance(**chance_inputs),baseline=False))
        append('item_rng',dict(callsite='enchant',word_before=0,word_after=1,bound=100000,value=residue))
        for spent in (scroll,support_item):
            if spent:append('item_delta',dict(operation='consume',instance=spent['id'],before=spent,after=None,count_delta=-1,operation_ordinal=0,receipt='receipt:1'))
        if result['destroyed'] or result['enchant']!=enchant:
            after=None if result['destroyed'] else target|dict(enchant=result['enchant'],revision=1)
            append('item_delta',dict(operation='destroy' if result['destroyed'] else 'enchant',instance=target_id,before=target,after=after,count_delta=-1 if after is None else 0,operation_ordinal=0,receipt='receipt:1'))
        if result['destroyed']:
            definition=gate.item_tables()[71];grade=gate.source().grades[definition['grade']]
            refund_inputs=dict(base=definition['crystal_count'],bonus=grade['weapon_bonus'],enchant=enchant,weapon=True,crystallizable=definition['flags']['is_crystallizable'],crystal_id=grade['crystal_id'])
            refund=gate.oracle.failure_refund(**refund_inputs)
            append('item_formula',dict(formula='failure_refund',inputs=refund_inputs,actual=refund,baseline=False))
            if refund:append('item_delta',dict(operation='grant',instance=10,before=None,after=item(10,refund,grade['crystal_id']),count_delta=refund,operation_ordinal=0,receipt='receipt:1'))
        rows.append(dict(kind='complete',facts=len(rows)-1,ticks=1,last_tick=0))
        return rows

    def test_enchant_requires_each_correlated_row_after_repaired_deletion(self):
        for rows in (self.enchant_rows(support=True),self.enchant_rows(base=70,residue=99999,enchant=3)):
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance','failure_refund'],HASH)['passed'])
            for index in range(1,len(rows)-1):
                bad=copy.deepcopy(rows);del bad[index];self.repair(bad)
                self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome','enchant_chance','failure_refund'],HASH)['passed'],str(index))
        rows=self.enchant_rows()
        for field,value in [('entity',OTHER),('incarnation',2)]:
            bad=copy.deepcopy(rows);bad[3][field]=value
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        for kind in ('item_rng','item_delta','item_formula'):
            bad=copy.deepcopy(rows)
            next(r for r in bad[2:-1] if r['kind']==kind)['data']['resolution_id']='unrelated'
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        bad=copy.deepcopy(rows);bad[3]['data']['value']=1
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])

    def test_enchant_legitimate_no_refund_no_change_branches(self):
        for kw in (dict(base=70,residue=99999,enchant=3,safe=True),dict(base=70,residue=99999,enchant=3,blessed=True)):
            rows=self.enchant_rows(**kw)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','failure_refund'],HASH)['passed'])
            self.assertFalse(any(r['kind']=='item_delta' and r['data']['operation']=='grant' for r in rows))
        rows=self.enchant_rows(blessed=True)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])

    def test_enchant_source_template_flags_eligibility_and_chance(self):
        for field,value in [('safe',True),('blessed',True),('base',71),('bonus',1),('support',18)]:
            rows=self.enchant_rows(base=70,residue=99999,enchant=3)
            rows[1]['data']['inputs'][field]=value
            self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance','failure_refund'],HASH)['passed'],field)
        # Reproduce the false safe branch with an internally consistent ledger,
        # then substitute normal scroll951 (or arrow17) in every binding/debit.
        for template in (951,17):
            rows=self.enchant_rows(base=70,residue=99999,enchant=3,safe=True)
            rows[1]['data']['operation']['scroll_before']['template_id']=template
            next(r for r in rows if r['kind']=='item_delta')['data']['before']['template_id']=template
            self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','failure_refund'],HASH)['passed'])
        for target,scroll,support in [(17,951,None),(1,951,None),(71,955,None),(71,951,12362),(71,951,12368),(71,6573,12363),(71,951,17)]:
            with self.assertRaises(ValueError):gate.enchant_source_inputs(item(1,1,target),item(2,1,scroll),item(3,1,support) if support else None)
        for enchant,scroll,support in [(15,22227,None),(10,951,12363),(65536,951,None)]:
            with self.assertRaises(ValueError):gate.enchant_source_inputs(item(1,1,71)|{'enchant':enchant},item(2,1,scroll),item(3,1,support) if support else None)
        # A capped chance alone cannot prove the individual source contributions.
        rows=self.enchant_rows(support=True);rows[1]['data']['inputs']['support']=0
        self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        rows=self.enchant_rows(support=True);rows[2]['data']['inputs']['support']=0
        self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        # Source slot masks map both underwear/shirt and onepiece/fullarmor.
        shirt=gate.enchant_source_inputs(item(1,1,10491)|{'enchant':3},item(2,1,952),None)
        self.assertEqual(shirt['base'],gate.oracle.F('66.67'))
        armor=next(r for r in gate.item_tables().values() if r['grade']=='C' and r['bodypart']=='onepiece' and r['enchant_enabled'])
        full=gate.enchant_source_inputs(item(1,1,armor['template_id'])|{'enchant':3},item(2,1,952),None)
        self.assertEqual(full['base'],100)

    def test_canonical_slot_paperdoll_autoshot_mutations(self):
        rows=copy.deepcopy(self.rows);rows[1].update(kind='item_baseline',data=dict(inventory_revision=0,instances=[item(1,1,6,'rhand')],paperdoll={'rhand':1},auto_shots=[5789]))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['grant_denied'],HASH)['passed'])
        def invented(data):data['instances'][0]['slot']='invented'
        def other(data):data['paperdoll']={'other_invented_slot':1}
        def missing(data):data['paperdoll']={}
        def wrong(data):data['paperdoll']={'lhand':1}
        def duplicate(data):data['instances'].append(item(2,1,1,'rhand'))
        for change in (invented,other,missing,wrong,duplicate):
            bad=copy.deepcopy(rows);change(bad[1]['data'])
            with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        for value in ('bad-template',True,0,-1,2**32,2000000000,1):
            bad=copy.deepcopy(rows);bad[1]['data']['auto_shots']=[value]
            with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[1]['data']['instances'][0]['template_id']=2000000000
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        # D17 dormant starter maps/elixirs still exist in the bound catalogue.
        rows[1]['data']['instances'].append(item(2,1,5588))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['grant_denied'],HASH)['passed'])

    def test_formula_domains_reject_negative_and_overflow_through_evaluate(self):
        cases=[('cancel_fee',dict(grade='C',crystals=-1)),
               ('enchant_valid',dict(current=None,max_enchant=0)),
               ('enchant_valid',dict(current=-1,max_enchant=-1)),
               ('enchant_valid',dict(current=2**32,max_enchant=2**32)),
               ('crystal_count',dict(base=-1,bonus=45,enchant=0,weapon=True)),
               ('shot_payment',dict(charges=[],kind='soul',count=-1,available=100,weapon_grade='NONE',shot_grade='NONE')),
               ('lethal',dict(full=0,half=0,full_residue=-1,half_residue=0)),
               ('heal',dict(power=0,m_atk=0,current_hp=-1)),
               ('cancel_fee',dict(grade='C',crystals=2**63)),
               ('enchant_hp',dict(bonuses=[1],enchant=2**32))]
        for name,inputs in cases:
            rows=copy.deepcopy(self.rows)
            try:actual=getattr(gate.oracle,name)(**inputs)
            except ValueError:actual=None
            rows[1]['data'].update(formula=name,inputs=inputs,actual=gate.oracle.rational(actual))
            self.assertFalse(gate.evaluate(rows,self.recording,OWNER,[name],HASH)['passed'],(name,inputs))
        for name,inputs in [('grade_penalties',dict(equipped=[],expertise=0,bonus=-2)),
                            ('max_load',dict(con_bonus=1,add=-10)),
                            ('bow_damage',dict(formula='100',dx_milli=-12500,dy_milli=0))]:
            actual=gate.oracle.rational(getattr(gate.oracle,name)(**inputs))
            rows=copy.deepcopy(self.rows);rows[1]['data'].update(formula=name,inputs=inputs,actual=actual)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,[name],HASH)['passed'],name)

    def test_numeric_input_types_before_oracle(self):
        for inputs in ({'base':True},{'base':{'numerator':True,'denominator':1}},{'base':100,'bonus':False}):
            with self.assertRaises(ValueError):gate.formula('enchant_chance',inputs,1000)
        with self.assertRaises(ValueError):gate.formula('enchant_outcome',dict(base=100,bonus=0,support=True,residue=0,enchant=0),{})
        with self.assertRaises(ValueError):gate.formula('pole_damage',dict(formula='100',primary=1),{})
        for residue in ('0',{'numerator':0,'denominator':1},True):
            with self.assertRaises(ValueError):gate.formula('fist_spread',dict(level=14,residue=residue),{})
        gate.formula('pole_damage',dict(formula='100',primary=True),gate.oracle.rational(gate.oracle.pole_damage('100',primary=True)))

    def test_phase_binding_rejects_unrelated_weapon_or_deleted_mutation(self):
        rows=copy.deepcopy(self.rows);rows[1]['data']['context']={'weapon_template':15}
        phase=dict(name='secondary-pole',formula='pole_damage',context={'weapon_template':15},inputs={'primary':False})
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        rows[1]['data']['context']['weapon_template']=1299
        self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        phase=dict(name='equip-pole',formula='pole_damage',mutation=dict(operation='equip',template=15,slot='rhand'))
        rows[1]['data']['context']['weapon_template']=15
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=1,
            data=dict(resolution_id=rows[1]['data']['resolution_id'],operation='equip',instance=8,
                      before=item(8,1,15),after=item(8,1,15,'rhand'),count_delta=0,operation_ordinal=0,receipt='equip:1')))
        self.repair(rows)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        bad=copy.deepcopy(rows);del bad[2];self.repair(bad)
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        for field,value in [('entity',OTHER),('incarnation',2)]:
            bad=copy.deepcopy(rows);bad[2][field]=value
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        bad=copy.deepcopy(rows);bad[2]['data']['resolution_id']='foreign-operation'
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])

    def test_incomplete_armour_admission_rejects_stateless_history_mutation(self):
        phase=next(p for p in gate.phases(SCRIPTS.parent/'Scenarios/4-armor-set.nfs') if p['name']=='active-incomplete-history-reconnect')
        inputs=dict(class_file=gate.oracle.D+'stats/chars/baseStats/HumanMystic.xml',level=14,
                    equipment=[[58,0,'chest',1],[628,0,'lhand',2],[461,0,'legs',0]],skill_refs=[[3543,1,3]],base_deltas={},base_delta_sequence=0)
        rows=copy.deepcopy(self.rows)
        def fact(values):
            return dict(formula='gear_stats',inputs=values,actual=gate.oracle.rational(gate.oracle.gear_stats(gate.source(),**values)),
                        context={'set_id':3,'event':'admission'},baseline=False,resolution_id='admission:1')
        rows[1]['data']=fact(inputs)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['gear_stats'],HASH,[phase])['passed'])
        # Rebuild with full_set&&shield and recompute the sheet, so generic formula
        # correctness still passes but the required retained provider gate fails.
        bad=copy.deepcopy(rows);bad[1]['data']=fact(inputs|{'skill_refs':[]})
        self.assertTrue(gate.evaluate(bad,self.recording,OWNER,['gear_stats'],HASH)['passed'])
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['gear_stats'],HASH,[phase])['passed'])
        self.assertNotEqual(rows[1]['data']['actual'],bad[1]['data']['actual'])
        # A complete-set admission with the provider cannot satisfy this checkpoint.
        bad[1]['data']=fact(inputs|{'equipment':inputs['equipment'][:2]+[[59,0,'legs',4],[47,0,'head',5]]})
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['gear_stats'],HASH,[phase])['passed'])

    def test_actual_intermediates_d24_d25_and_bow(self):
        self.assertTrue(gate.evaluate(self.rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        for level,radius in [(1,6),(3,6),(4,7),(8,7),(9,8),(15,8),(16,9)]:
            gate.formula('fist_spread',dict(level=level,residue=0),dict(radius=radius,bound=radius*2+1,offset=-radius,multiplier=gate.oracle.rational(gate.oracle.F(100-radius,100))))
        for dx,damage in [(12499,89),(12500,90),(12501,90)]:
            expected=gate.oracle.rational(gate.oracle.bow_damage('100',dx,0))
            self.assertEqual(expected['damage'],damage)
            gate.formula('bow_damage',dict(formula='100',dx_milli=dx,dy_milli=0),expected)
        gate.formula('bow_reuse',dict(reuse_delay=1500,attack_speed=400,weapon_reuse_modifier=2),1248)
        bad=copy.deepcopy(self.rows);bad[1]['data']['actual']['damage']=42
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH)['passed'])

    def test_missing_foreign_owner_unknown_baseline_and_repeated_fail(self):
        self.assertFalse(gate.evaluate(self.rows,self.recording,OTHER,['pole_damage'],HASH)['passed'])
        self.assertFalse(gate.evaluate(self.rows,self.recording,OWNER,['bow_damage'],HASH)['passed'])
        for change in ('baseline','unknown','repeat','partial'):
            rows=copy.deepcopy(self.rows)
            if change=='baseline':rows[1]['data']['baseline']=True
            if change=='unknown':rows[1]['kind']='UNKNOWN'
            if change=='partial':del rows[1]['data']['inputs']
            if change=='repeat':
                rows.insert(2,copy.deepcopy(rows[1]));rows[2]['sequence']=1;rows[-1]['facts']=2
            with self.assertRaises(ValueError,msg=change):gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)

    def test_manifest_recording_hash_versions_watermark_and_bounds_fail(self):
        for key,value in [('phase4_rules_hash','b'*64),('record_schema',7),('snapshot_schema',9),('digest_version','BinaryV5'),('recording_sha256','c'*64),('scope','owner_baseline')]:
            rows=copy.deepcopy(self.rows);rows[0][key]=value
            with self.assertRaises(ValueError,msg=key):gate.validate(rows,self.recording,HASH)
        rows=copy.deepcopy(self.rows);rows[1]['tick']=1
        with self.assertRaises(ValueError):gate.validate(rows,self.recording,HASH)
        rows=copy.deepcopy(self.rows);rows[0]['watermark']['reason']='live'
        with self.assertRaises(ValueError):gate.validate(rows,self.recording,HASH)

    def test_json_rejects_duplicate_keys_float_and_nan(self):
        path=Path(self.tmp.name)/'bad.jsonl'
        for text in ('{"x":1,"x":2}','{"x":1.0}','{"x":NaN}'):
            path.write_text(text+'\n')
            with self.assertRaises(ValueError):gate.read_jsonl(path)

    def test_rng_and_ledger_conservation(self):
        rows=copy.deepcopy(self.rows)
        rows.insert(2,dict(kind='item_rng',entity=OWNER,incarnation=1,tick=0,sequence=1,data=dict(resolution_id='enchant:1',callsite='enchant',word_before=0,word_after=1,bound=100000,value=66670)))
        rows.insert(3,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=2,data=dict(resolution_id='enchant:1',operation='consume',instance=7,before=item(7,2),after=item(7,1),count_delta=-1,operation_ordinal=0,receipt=None)))
        rows[-1]['facts']=3;gate.validate(rows,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[3]['data']['count_delta']=0
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[2]['data']['value']=100000
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)

    def test_source_recipe_and_gate_denial(self):
        inputs=dict(recipe_id=4316442132481,before_instance=dict(id=7,template_id=72,enchant=3,augment_id=None),
                    after_instance=dict(id=20,template_id=4682,enchant=3,augment_id=None),
                    ingredients_before={'4645':1,'2131':97,'57':501000},ingredients_after={'4645':0,'2131':0,'57':210000},tax_rate='0')
        gate.formula('sa_exchange',inputs,dict(product_template=4682,new_instance_id=20))
        bad=copy.deepcopy(inputs);bad['after_instance']['id']=7
        with self.assertRaises(ValueError):gate.formula('sa_exchange',bad,dict(product_template=4682,new_instance_id=7))
        gate.formula('grant_denied',dict(auth_dev_tokens=False,status='FAILED_PRECONDITION',before=[1],after=[1],ledger_rows=0,rng_before=0,rng_after=0),{'unchanged':True})

    def test_denied_grant_uses_complete_log_absence_not_invented_actor_command(self):
        rows=copy.deepcopy(self.rows)
        rows[1].update(kind='item_baseline',data=dict(inventory_revision=0,instances=[item(1,1,6,'rhand')],paperdoll={'rhand':1},auto_shots=[]))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['grant_denied'],HASH)['passed'])
        self.assertFalse(gate.evaluate(rows,self.recording,OTHER,['grant_denied'],HASH)['passed'])
        self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        rows.insert(2,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=1,data=dict(resolution_id='enchant:1',operation='grant',instance=2,before=None,after=item(2,1,1),count_delta=1,operation_ordinal=0,receipt=None)))
        rows[-1]['facts']=2
        self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['grant_denied'],HASH)['passed'])

    def test_soak_requires_every_owner_positive_auto_payment_and_one_scroll(self):
        rows=[copy.deepcopy(self.rows[0])];roles={'items':True,'casting':True,'roles':[]}
        for index in range(8):
            entity=f'01970000-0000-7000-8000-{index+80:012d}'
            roles['roles'].append({'seed':{'character_id':entity}})
            def append(kind,data):
                if kind=='item_delta' and 'resolution_id' not in data:data['resolution_id']='equip:1'
                rows.append(dict(kind=kind,entity=entity,incarnation=1,tick=0,sequence=len(rows)-1,data=data))
            for fact in self.enchant_rows(entity=entity,target_id=index*100+4,scroll_id=index*100+3)[1:-1]:
                if fact['kind']=='item_rng':fact['data'].update(word_before=index,word_after=index+1)
                append(fact['kind'],fact['data'])
            append('item_formula',dict(formula='shot_payment',inputs=dict(charges=[],kind='spirit',count=1,available=100,weapon_grade='NONE',shot_grade='NONE',toggle=True),actual=dict(charges=['spirit'],available=99,toggle=True,consumed=1,draws=[],outcome='charged'),resolution_id='cast:1:charge',baseline=False))
            for offset,template in enumerate((1,6),1):
                identity=index*100+offset
                append('item_delta',dict(operation='equip',instance=identity,before=item(identity,1,template),after=item(identity,1,template,'rhand'),count_delta=0,operation_ordinal=0,receipt=None))

        rows.append(dict(kind='complete',facts=len(rows)-1,ticks=1,last_tick=0))
        self.assertTrue(gate.soak(rows,self.recording,roles,HASH)['passed'])
        bad=copy.deepcopy(rows)
        shot=next(r for r in bad if r.get('kind')=='item_formula' and r['data']['formula']=='shot_payment')
        shot['data']['inputs']['available']=0
        shot['data']['actual'].update(charges=[],available=0,toggle=False,consumed=0,outcome='insufficient')
        self.assertFalse(gate.soak(bad,self.recording,roles,HASH)['passed'])
        badroles=copy.deepcopy(roles);badroles['roles'][-1]['seed']['character_id']='01970000-0000-7000-8000-000000000099'
        self.assertFalse(gate.soak(rows,self.recording,badroles,HASH)['passed'])

    def test_all_scenarios_have_known_gate_requirements(self):
        for path in (SCRIPTS.parent/'Scenarios').glob('4-*.nfs'):
            self.assertTrue(gate.requirements(path),path.name)


if __name__=='__main__':unittest.main()
