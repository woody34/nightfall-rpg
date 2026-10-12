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

    def test_source_bow_launch_inventory_records_and_count_maps(self):
        inputs=dict(weapon=gate.oracle.behavior.bow_definition(gate.source(),13),
                    inventory=[gate.oracle.behavior.arrow_stack(gate.source(),17,2)],
                    lhand=None,mp=10,mp_modifier=1,reduction_residue=0,miss=False,
                    hit_chance=800,hit_residue=800)
        actual=gate.oracle.behavior.bow_launch(**inputs)
        self.assertEqual((actual['arrows_consumed'],actual['inventory'][0]['count'],actual['mp']),(1,1,9))
        rows=copy.deepcopy(self.rows)
        rows[1]['data'].update(formula='bow_launch',inputs=inputs,actual=gate.oracle.rational(actual))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['bow_launch'],HASH)['passed'])
        for count in (-1,2**63):
            bad=copy.deepcopy(rows);bad[1]['data']['inputs']['inventory'][0]['count']=count
            bad[1]['data']['actual']=gate.oracle.rational(gate.oracle.behavior.bow_launch(**bad[1]['data']['inputs']))
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['bow_launch'],HASH)['passed'])
            with self.assertRaises(ValueError):gate.input_domains('learned_item_cost',dict(inventory={'17':count}))

    def test_item_record_scalar_substitutions_through_evaluate(self):
        vectors=gate.oracle.make_vectors(gate.source())
        learned=next(v for v in vectors if v['id']=='learned-item-consume-complete-1')
        dormant=next(v for v in vectors if v['id'].startswith('dormant-starter-use-'))
        for name,vector in [('learned_item_cost',learned),('dormant_use',dormant)]:
            inputs=copy.deepcopy(vector['inputs'])
            if name=='dormant_use':
                inputs['properties']={k:v for k,v in inputs['properties'].items() if k in ('handler','item_skill')}
            inputs['inventory'].append(dict(instance_id=3,template_id=57,count=10,
                                           enchant=0,charges=0,grade='NONE',kind='ADENA',augment_id=None))
            function=getattr(gate.oracle.behavior,name)
            rows=copy.deepcopy(self.rows)
            rows[1]['data'].update(formula=name,inputs=inputs,
                actual=gate.oracle.rational(function(**gate.oracle.decode_rationals(inputs))))
            control=gate.evaluate(rows,self.recording,OWNER,[name],HASH)
            self.assertTrue(control['passed'],control)
            for field in ('count','enchant','charges'):
                for value in ([-1],[2**63],{'bad':-1},None,-1,2**63,True,{'numerator':1,'denominator':1}):
                    with self.subTest(formula=name,field=field,value=value):
                        bad=copy.deepcopy(rows)
                        bad_inputs=bad[1]['data']['inputs']
                        bad_inputs['inventory'][-1][field]=value
                        bad[1]['data']['actual']=gate.oracle.rational(function(**gate.oracle.decode_rationals(bad_inputs)))
                        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,[name],HASH)['passed'])

    def test_shot_charge_lists_through_evaluate(self):
        for name,inputs in [('charge_transition',dict(charges=['soul','spirit'],use_soul=True)),
                            ('shot_payment',dict(charges=['spirit'],kind='soul',count=1,available=2,weapon_grade='D',shot_grade='D')),
                            ('shared_shot_bundle',dict(charges=['soul'],hits=[True,False],use_soul=True))]:
            rows=copy.deepcopy(self.rows)
            rows[1]['data'].update(formula=name,inputs=inputs,actual=gate.oracle.rational(getattr(gate.oracle,name)(**inputs)))
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,[name],HASH)['passed'],name)

    def test_source_resurrection_restore_ratio_through_evaluate(self):
        import re
        config=gate.source().read('src/main/resources/config/character.properties',True)
        ratio=gate.oracle.F(re.search(r'^RespawnRestoreHP\s*=\s*(\S+)',config,re.M)[1])
        self.assertEqual(ratio,gate.oracle.F(13,20))
        definition=gate.oracle.consumable_definition(gate.source(),737)
        inputs=dict(definition=definition,start_tick=10,available=2,death_incarnation=7,
                    current_incarnation=7,dead=True,response='accept',response_tick=160,
                    response_offer_id=1,existing_offer=None,repeat=False,hp=0,max_hp=101,
                    mp=13,cp=17,recoverable_xp=101,restore_percent=0,restore_hp=ratio)
        actual=gate.oracle.behavior.resurrection_scroll(**inputs)
        self.assertEqual(actual['hp'],gate.oracle.F(1313,20))
        rows=copy.deepcopy(self.rows)
        rows[1]['data'].update(formula='resurrection_scroll',inputs=gate.oracle.rational(inputs),actual=gate.oracle.rational(actual))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['resurrection_scroll'],HASH)['passed'])
        for value in (True,{'numerator':-13,'denominator':20}, {'numerator':21,'denominator':20}, {'numerator':13,'denominator':0}):
            bad=copy.deepcopy(rows);bad[1]['data']['inputs']['restore_hp']=value
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['resurrection_scroll'],HASH)['passed'])

    def test_signed_formula_controls(self):
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

    def formula_rows(self,name,inputs):
        rows=copy.deepcopy(self.rows)
        function=getattr(gate.oracle,name,None) or getattr(gate.oracle.behavior,name,None)
        try:actual=gate.oracle.rational(function(**gate.oracle.decode_rationals(inputs)))
        except (TypeError,ValueError,KeyError,IndexError,ZeroDivisionError,AttributeError):actual=None
        rows[1]['data'].update(formula=name,inputs=inputs,actual=actual)
        return rows

    def test_input_schema_completeness_matches_e1_registry_and_signatures(self):
        import ast
        import inspect
        # Read E1's exposed registry independently; do not reuse the harness list.
        tree=ast.parse(inspect.getsource(gate.oracle.verify_facts))
        exposed=set()
        for node in ast.walk(tree):
            if isinstance(node,ast.List):
                for entry in node.elts:
                    if isinstance(entry,ast.Name):exposed.add(entry.id)
                    elif isinstance(entry,ast.Attribute) and isinstance(entry.value,ast.Name) and entry.value.id=='behavior':
                        exposed.add(entry.attr)
        self.assertEqual(set(gate.INPUT_SCHEMAS),exposed|{'gear_stats','sa_exchange','grant_denied'})
        self.assertEqual(set(gate.INPUT_SCHEMAS),gate.FORMULAS)
        for name in exposed|{'gear_stats'}:
            function=getattr(gate.oracle,name,None) or getattr(gate.oracle.behavior,name,None)
            with self.subTest(formula=name):
                parameters=dict(inspect.signature(function).parameters);parameters.pop('src',None)
                self.assertEqual(set(gate.INPUT_SCHEMAS[name]),set(parameters))
                for key,param in parameters.items():
                    if param.default is None:self.assertEqual(gate.INPUT_SCHEMAS[name][key][0],'nullable')
                    elif type(param.default) is bool:self.assertEqual(gate.INPUT_SCHEMAS[name][key],gate.BOOL)

    def test_signature_compatible_e1_vector_inputs_preserve_shapes(self):
        import inspect
        vectors=json.loads((gate.REPO/'packages/data/fixtures/item-oracle.json').read_text())['vectors']
        controls=0;formulas=set()
        for name,schema in gate.INPUT_SCHEMAS.items():
            function=getattr(gate.oracle,name,None) or getattr(gate.oracle.behavior,name,None)
            if not function:continue
            required={k for k,p in inspect.signature(function).parameters.items()
                      if k!='src' and p.default is inspect.Parameter.empty}
            for vector in vectors:
                if not required<=set(vector['inputs']) or set(vector['inputs'])-set(schema):continue
                inputs=copy.deepcopy(vector['inputs'])
                # E1 expresses repeat as 0/1; the D46 flag is a JSON boolean.
                if name=='resurrection_scroll':inputs['repeat']=bool(inputs['repeat'])
                with self.subTest(formula=name,vector=vector['id']):gate.input_domains(name,inputs)
                controls+=1;formulas.add(name)
        self.assertGreaterEqual(controls,3202)
        self.assertGreaterEqual(len(formulas),43)

    def test_scalar_shape_mutations_on_perfect_block_branches(self):
        # Recompute output: these oracle branches deliberately never read operands.
        for name,inputs in [('physical_damage',dict(p_atk=10,defence=10,perfect=True)),
                            ('magic_damage',dict(power=10,m_atk=10,m_def=10,perfect=True))]:
            control=self.formula_rows(name,inputs)
            self.assertTrue(gate.evaluate(control,self.recording,OWNER,[name],HASH)['passed'])
            for key,kind in gate.INPUT_SCHEMAS[name].items():
                if kind[0] not in ('exact-numeric','int','nonneg-int','u32'):continue
                for value in ([],[1],None,{},True,1.0,'bad',2**63):
                    with self.subTest(formula=name,parameter=key,value=value):
                        bad=self.formula_rows(name,inputs|{key:value})
                        result=gate.evaluate(bad,self.recording,OWNER,[name],HASH)
                        self.assertFalse(result['passed'])
                        errors=[error for row in result['gates'] for error in row['errors']]
                        self.assertTrue(any('invalid formula input '+name+'.'+key in e for e in errors),errors)

    def test_all_shapes_are_checked_before_numeric_bounds(self):
        for p_atk in (2**63,{'numerator':2**63,'denominator':1}):
            rows=self.formula_rows('physical_damage',dict(p_atk=p_atk,defence=10,perfect=[True]))
            result=gate.evaluate(rows,self.recording,OWNER,['physical_damage'],HASH)
            self.assertFalse(result['passed'])
            errors=[e for r in result['gates'] for e in r['errors']]
            self.assertTrue(any('physical_damage.perfect (bool)' in e for e in errors),errors)

    def test_parameter_kinds_mutated_through_full_evaluate(self):
        from unittest.mock import patch
        cases=[
            # kind, valid control, parameter, invalid shapes/types/bounds
            ('pole_damage',dict(formula='499/10',percent=85,primary=False),'formula',
             ([],[1],None,{},True,1.0,'bad',2**63,{'numerator':1,'denominator':0},
              {'numerator':True,'denominator':1},{'numerator':1,'denominator':2**63})),
            ('bow_reuse',dict(reuse_delay=100,attack_speed=300),'reuse_delay',
             ([],[1],None,{},True,'1',-1,2**63)),
            ('enchant_valid',dict(current=0,max_enchant=0),'current',
             ([],[1],None,{},True,1.0,'0',-1,2**32)),
            ('bow_damage',dict(formula=100,dx_milli=-12500,dy_milli=0),'dx_milli',
             ([],[1],None,{},True,'0',-2**63-1,2**63)),
            ('physical_damage',dict(p_atk=10,defence=10,perfect=True),'perfect',
             ([],[True],None,{},0,1,'true')),
            ('physical_damage',dict(p_atk=10,defence=10,perfect=True),'caller',
             ([],['auto'],None,{},True,1,'bad')),
            ('enchant_hp',dict(bonuses=[1,'3/2',{'numerator':2,'denominator':1}],enchant=0),'bonuses',
             (None,1,'1',{},[None],[True],[1.0],['bad'],[[]],[-1],[2**63])),
            ('shared_shot_bundle',dict(charges=['soul'],hits=[True,False]),'hits',
             (None,True,1,{},[None],[1],['bad'],[[]])),
            ('heal',dict(power=0,m_atk=0,recoverable_hp=None),'recoverable_hp',
             ([],[1],{},True,'bad',-1,2**63)),
            ('magic_damage',dict(power=10,m_atk=10,m_def=10,perfect=True,is_pvp=None),'is_pvp',
             ([],[True],{},0,1,'bad')),
            ('shot_payment',dict(charges=[],kind='soul',count=1,available=0,weapon_grade=None,shot_grade='D'),
             'weapon_grade',([],['D'],{},True,1,'bad')),
            ('shot_payment',dict(charges=[],kind='soul',count=1,available=0,weapon_grade='D',shot_grade='D',reduction=[]),
             'reduction',(None,1,{},[1],[0,1,2],[None,1],[1,-1],[1,2**63],[101,1])),
            ('dual_plan',dict(hits=[True,False],formulas=[10,'3/2'],second_valid=True,
                             hit_residues=[800,801],hit_chance=800,spread_radius=6),'formulas',
             (None,1,{},[],[1],[1,2,3],[True,1],[None,1],[[],1],[2**63,1])),
            ('winning_providers',dict(providers=[dict(key='item:1',skill_id=1,rank=1,sequence=0)]),'providers',
             (None,{},1,[None],[{}],[dict(key='item:1',skill_id=1,rank=1,sequence=0,unknown=1)],
              [dict(key='item:1',skill_id=[],rank=1,sequence=0)])),
            ('consumable_use',dict(definition=gate.oracle.consumable_definition(gate.source(),737),tick=10,
                                  available={'737':2},condition_context={'canResurrect':True}),'available',
             (None,[],1,{'737':None},{'737':[]},{'737':True},{'737':-1},{'737':2**63},{'bad':1})),
            ('augmentation_provider',dict(option_id=1,option_xml='<option/>',event='install',other_provider=None,
                                         hp=1,maximum=1,stat_baseline={}), 'option_xml', (None,[],{},1,True)),
        ]
        kinds=set()
        for name,inputs,key,mutations in cases:
            kinds.add(gate.INPUT_SCHEMAS[name][key][0])
            control=self.formula_rows(name,inputs)
            result=gate.evaluate(control,self.recording,OWNER,[name],HASH)
            self.assertTrue(result['passed'],(name,key,result))
            for value in mutations:
                with self.subTest(formula=name,parameter=key,value=value):
                    bad=self.formula_rows(name,inputs|{key:value})
                    # Invalid inputs must reject before E1 is called, even if it
                    # could return an oracle-consistent result for the bad shape.
                    with patch.object(gate.oracle,'verify_facts',side_effect=AssertionError('oracle reached')):
                        result=gate.evaluate(bad,self.recording,OWNER,[name],HASH)
                    self.assertFalse(result['passed'])
                    self.assertTrue(any('invalid formula input '+name+'.'+key in error
                                        for row in result['gates'] for error in row['errors']),result)
        self.assertEqual(kinds,{'exact-numeric','nonneg-int','u32','int','bool','enum','list','nullable','list-tuple','tuple','map','string'})

    def test_shot_charge_members_and_unknown_parameters_through_evaluate(self):
        for name,inputs in [('charge_transition',dict(charges=['soul','spirit','blessed'],use_soul=True)),
                            ('shot_payment',dict(charges=[],kind='soul',count=1,available=0,weapon_grade='D',shot_grade='D')),
                            ('shared_shot_bundle',dict(charges=['soul'],hits=[True,False],use_soul=True))]:
            for charges in ([],['soul'],['spirit'],['blessed'],['soul','spirit','blessed']):
                control=self.formula_rows(name,inputs|{'charges':charges})
                self.assertTrue(gate.evaluate(control,self.recording,OWNER,[name],HASH)['passed'])
            for charges in (None,'soul',{},1,[-1],[2**63],[None],['bad'],[True],[[]],[{}]):
                with self.subTest(formula=name,charges=charges):
                    bad=self.formula_rows(name,inputs|{'charges':charges})
                    result=gate.evaluate(bad,self.recording,OWNER,[name],HASH)
                    self.assertFalse(result['passed'])
                    self.assertTrue(any('invalid formula input '+name+'.charges' in e for r in result['gates'] for e in r['errors']),result)
        for name,inputs in [('physical_damage',dict(p_atk=10,defence=10,perfect=True)),
                            ('magic_damage',dict(power=10,m_atk=10,m_def=10,perfect=True)),
                            ('charge_transition',dict(charges=[]))]:
            control=self.formula_rows(name,inputs)
            for key in ('unknown','src'):
                bad=copy.deepcopy(control);bad[1]['data']['inputs'][key]=None
                result=gate.evaluate(bad,self.recording,OWNER,[name],HASH)
                self.assertFalse(result['passed'])
                self.assertTrue(any('invalid formula input '+name in e for r in result['gates'] for e in r['errors']),result)

    def test_schema_preserves_nullable_and_structured_controls(self):
        cases=[('enchant_hp',dict(bonuses=[],enchant=0)),
               ('heal',dict(power=1,m_atk=1,recoverable_hp='3/2')),
               ('heal',dict(power=-1,m_atk=121,static=True)),
               ('physical_damage',dict(p_atk=0,defence=80,power=-1,caller='blow')),
               ('position',dict(facing=[1,0],relative=[-1,0])),
               ('grade_penalties',dict(equipped=[dict(instance_id=1,slot=0,grade=7,weapon=False)],expertise=0)),
               ('magic_damage',dict(power=1,m_atk=1,m_def=1,is_pvp=True,second_success=False)),
               ('overload',dict(weight=0,maximum=-1)),
               ('player_level_range',dict(condition={'kind':'player_level_range','ranges':[{'minimum':1,'maximum':84}]},rank=1,level=84))]
        # E1 vectors supply genuine definition, map, and nullable offer operands.
        vectors=json.loads((gate.REPO/'packages/data/fixtures/item-oracle.json').read_text())['vectors']
        for prefix,name in [('bow-launch-NONE-equipped-False','bow_launch'),('resurrection-scroll-','resurrection_scroll'),
                            ('consumable-escape-timing-','escape_timing'),('consumable-same-tick-lanes','consumable_cast_sequence'),
                            ('dormant-starter-use-','dormant_use')]:
            vector=next(v for v in vectors if v['id'].startswith(prefix))
            inputs=copy.deepcopy(vector['inputs'])
            if name=='resurrection_scroll':inputs['repeat']=bool(inputs['repeat'])
            cases.append((name,inputs))
        for name,inputs in cases:
            with self.subTest(formula=name):
                rows=self.formula_rows(name,inputs)
                result=gate.evaluate(rows,self.recording,OWNER,[name],HASH)
                self.assertTrue(result['passed'],result)

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
