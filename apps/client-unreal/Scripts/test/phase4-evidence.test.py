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
        rows.insert(2,dict(kind='item_rng',entity=OWNER,incarnation=1,tick=0,sequence=1,data=dict(callsite='enchant',word_before=0,word_after=1,bound=100000,value=66670)))
        rows.insert(3,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=2,data=dict(operation='consume',instance=7,before=item(7,2),after=item(7,1),count_delta=-1,operation_ordinal=0,receipt=None)))
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
        rows.insert(2,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=1,data=dict(operation='grant',instance=2,before=None,after=item(2,1,1),count_delta=1,operation_ordinal=0,receipt=None)))
        rows[-1]['facts']=2
        self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['grant_denied'],HASH)['passed'])

    def test_soak_requires_every_owner_positive_auto_payment_and_one_scroll(self):
        rows=[copy.deepcopy(self.rows[0])];roles={'items':True,'casting':True,'roles':[]}
        for index in range(8):
            entity=f'01970000-0000-7000-8000-{index+80:012d}'
            roles['roles'].append({'seed':{'character_id':entity}})
            def append(kind,data):rows.append(dict(kind=kind,entity=entity,incarnation=1,tick=0,sequence=len(rows)-1,data=data))
            append('item_formula',dict(formula='enchant_outcome',inputs=dict(base=100,bonus=0,support=0,residue=0,enchant=0),actual={'outcome':'success','enchant':1,'destroyed':False},resolution_id='enchant:1',baseline=False))
            append('item_formula',dict(formula='shot_payment',inputs=dict(charges=[],kind='spirit',count=1,available=100,weapon_grade='NONE',shot_grade='NONE',toggle=True),actual=dict(charges=['spirit'],available=99,toggle=True,consumed=1,draws=[],outcome='charged'),resolution_id='cast:1:charge',baseline=False))
            for offset,template in enumerate((1,6),1):
                identity=index*100+offset
                append('item_delta',dict(operation='equip',instance=identity,before=item(identity,1,template),after=item(identity,1,template,'rhand'),count_delta=0,operation_ordinal=0,receipt=None))
            identity=index*100+3
            append('item_delta',dict(operation='consume',instance=identity,before=item(identity,1,951),after=None,count_delta=-1,operation_ordinal=0,receipt=None))
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
