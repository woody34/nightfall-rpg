#!/usr/bin/env python3
"""Synthetic exporter validation and independent actual-input Fraction gate tests."""
import copy
import importlib.util
import json
import random
import re
from pathlib import Path
import tempfile
import unittest

SCRIPTS=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('evidence4',SCRIPTS/'phase4-evidence.py')
gate=importlib.util.module_from_spec(spec);spec.loader.exec_module(gate)
OWNER='01970000-0000-7000-8000-000000000040';OTHER='01970000-0000-7000-8000-000000000041';HASH='a'*64
# Independent operation expectations; never derive mutation coverage from the validator.
ALLOWED_CHANGES={
    'consume':{'count'},'equip':{'slot','location'},'unequip':{'slot','location'},
    'enchant':{'enchant'},'refine':{'augment_id'},'cancel_refine':{'augment_id'},
    'destroy':set(),'crystallize':set(),'grant':{'count'},'starter':set(),'backfill':set(),
    'use':{'charges'},
}


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

    def assert_rejected(self,rows,gates=('enchant_outcome',)):
        try:result=gate.evaluate(rows,self.recording,OWNER,gates,HASH)
        except ValueError:return
        self.assertFalse(result['passed'])

    def transition_rows(self,operation,count=5,enchant=2,created_tick=0):
        rows=copy.deepcopy(self.rows)
        if operation!='consume':count=1
        before=item(100,count,2509 if operation=='consume' else 71)|dict(enchant=enchant,created_tick=created_tick)
        if operation=='use':before.update(slot='rhand',location='paperdoll',charges=1)
        after=before|{'revision':1}
        if operation=='consume':after['count']-=1
        elif operation=='equip':after.update(slot='rhand',location='paperdoll')
        elif operation=='unequip':before.update(slot='rhand',location='paperdoll')
        elif operation=='enchant':after['enchant']+=1
        elif operation=='refine':after['augment_id']=65537
        elif operation=='cancel_refine':before['augment_id']=65537
        elif operation in ('destroy','crystallize'):after=None
        elif operation in ('grant','starter','backfill'):before=None
        elif operation=='use':after['charges']=0 # D73: receipt-free soulshot discharge.
        else:raise AssertionError(operation)
        rows.insert(1,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(inventory_revision=0,instances=[copy.deepcopy(before)] if before else [],
                      paperdoll={before['slot']:before['id']} if before and before['slot'] else {},auto_shots=[])))
        delta=dict(operation=operation,instance=100,before=before,after=after,
                   count_delta=(after['count'] if after else 0)-(before['count'] if before else 0),
                   operation_ordinal=0,resolution_id='transition:1')
        if operation=='consume':delta.update(receipt_refs=[],causes=[dict(cause='auto_shot:transition:1',count_delta=-1)])
        else:delta['receipt_ref']=None
        if operation=='use':delta['causes']=[dict(cause='shot_discharge:transition:1',count_delta=0)] # D93
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,data=delta))
        return self.repair(rows)

    def corrupt_transition(self,rows,field,amount=1,delta_index=0):
        delta=[r['data'] for r in rows if r['kind']=='item_delta'][delta_index]
        # Creation/removal forbid an endpoint altogether, regardless of its scalars.
        if delta['before'] is None:delta['before']=delta['after']|{'revision':0}
        if delta['after'] is None:delta['after']=delta['before']|{'revision':1}
        after=delta['after']
        if field=='template_id':after[field]=2510 if after[field]==2509 else 71 if after[field]==72 else 72
        elif field=='slot':after.update(slot='lhand',location='paperdoll')
        elif field=='location':after.update(slot='lhand',location='paperdoll')
        elif field=='augment_id':after[field]=65538 if after[field]==65537 else 65537
        else:after[field]+=amount
        # Repair conservation to ensure field drift cannot hide behind count checks.
        delta['count_delta']=after['count']-delta['before']['count']

    def test_every_operation_transition_and_disallowed_canonical_fields(self):
        fields=('template_id','enchant','created_tick','slot','augment_id','id','count','charges','location')
        probes=0
        for operation,allowed in ALLOWED_CHANGES.items():
            rows=self.transition_rows(operation)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'],operation)
            for field in set(fields)-allowed:
                with self.subTest(operation=operation,field=field):
                    bad=copy.deepcopy(rows);self.corrupt_transition(bad,field)
                    with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
                    probes+=1
            # Coherently rewriting all row identities still conflicts with the baseline.
            bad=copy.deepcopy(rows);delta=next(r['data'] for r in bad if r['kind']=='item_delta')
            delta['instance']=999
            for endpoint in ('before','after'):
                if delta[endpoint] is not None:delta[endpoint]['id']=999
            if delta['before'] is not None:
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        print(f'Operation transitions: {len(ALLOWED_CHANGES)} positive controls; {probes} forbidden-field mutations reject',flush=True)

    def test_transition_endpoint_shapes_directions_and_exact_count_delta(self):
        for operation in ALLOWED_CHANGES:
            rows=self.transition_rows(operation)
            for value in (False,True,'0',0.0,[],{},None,2**63,-2**63):
                bad=copy.deepcopy(rows);bad[3]['data']['count_delta']=value
                with self.assertRaises(ValueError,msg=(operation,value)):gate.validate(bad,self.recording,HASH)
            if operation in ('equip','unequip','enchant','refine','cancel_refine'):
                for endpoint in ('before','after'):
                    bad=copy.deepcopy(rows);delta=bad[3]['data'];delta[endpoint]=None
                    delta['count_delta']=(delta['after']['count'] if delta['after'] else 0)-(delta['before']['count'] if delta['before'] else 0)
                    with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
                bad=copy.deepcopy(rows);bad[3]['data']['after']=bad[3]['data']['before']|{'revision':1}
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        # Canonical direction violations with otherwise permitted scalar changes.
        for operation in ('equip','unequip','refine','cancel_refine'):
            bad=self.transition_rows(operation);delta=bad[3]['data']
            if operation in ('equip','unequip'):
                delta['before'].update(slot='rhand',location='paperdoll')
                delta['after'].update(slot='lhand',location='paperdoll')
                if operation=='equip':delta['after'].update(slot=None,location='inventory')
            elif operation=='refine':delta['before']['augment_id']=65537;delta['after']['augment_id']=None
            else:delta['after']['augment_id']=65538
            del bad[1];self.repair(bad)
            with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        for operation in ('destroy','crystallize'):
            rows=self.transition_rows(operation)
            grant=self.transition_rows('grant')[3]
            grant['data'].update(instance=101,after=item(101,3,1458)|{'revision':1},count_delta=3)
            rows.insert(-1,grant);self.repair(rows);gate.validate(rows,self.recording,HASH)
        bad=self.transition_rows('consume');bad[3]['data']['operation']='use'
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)

    def test_d73_use_rows_change_only_charges_with_neutral_causes(self):
        rows=self.transition_rows('use');gate.validate(rows,self.recording,HASH)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        # Receipt-bound use (manual shot charge): the receipt cause is projected into receipt_ref.
        bound=copy.deepcopy(rows);delta=bound[3]['data']
        delta['before']['charges']=0;delta['after']['charges']=1;bound[1]['data']['instances'][0]['charges']=0
        delta.update(receipt_ref='receipt:use:k1',causes=[dict(cause='receipt:use:k1',count_delta=0)])
        gate.validate(bound,self.recording,HASH)
        # Three discharges on one weapon in one tick: positional ordinals 0/1/2 (D76/D83).
        chain=copy.deepcopy(rows);first=chain[3];chain[1]['data']['instances'][0]['charges']=7
        first['data']['before']['charges']=7;first['data']['after']['charges']=3
        for ordinal,(start,end) in enumerate(((3,1),(1,0)),1):
            row=copy.deepcopy(first);row['data'].update(operation_ordinal=ordinal,resolution_id=f'transition:{ordinal+1}',
                before=first['data']['after']|{'charges':start},after=first['data']['after']|{'charges':end},
                causes=[dict(cause=f'shot_discharge:transition:{ordinal+1}',count_delta=0)])
            chain.insert(-1,row)
        self.repair(chain);gate.validate(chain,self.recording,HASH)
        mutations={
            'count changes':lambda d:(d['after'].update(count=2),d.update(count_delta=1)),
            'enchant changes':lambda d:d['after'].update(enchant=3),
            'charges unchanged':lambda d:d['after'].update(charges=1),
            'causes missing':lambda d:d.pop('causes'),
            'causes empty':lambda d:d.update(causes=[]),
            'negative cause':lambda d:d.update(causes=[dict(cause='shot_discharge:transition:1',count_delta=-1)]),
            'boolean cause':lambda d:d.update(causes=[dict(cause='shot_discharge:transition:1',count_delta=False)]),
            'payment cause on use (D93)':lambda d:d.update(causes=[dict(cause='auto_shot:transition:1',count_delta=0)]),
            'discharge resolution mismatch (D93)':lambda d:d.update(resolution_id='transition:9'),
            'discharge adds charges (review-2 P3-2)':lambda d:(d['before'].update(charges=1),d['after'].update(charges=3)),
            'discharge swaps charge kind':lambda d:(d['before'].update(charges=1),d['after'].update(charges=2)),
            'malformed cast discharge (D93)':lambda d:d.update(causes=[dict(cause='cast_discharge:cast:4',count_delta=0)]),
            'discharge cause on consume (D93)':None,
            'malformed cause':lambda d:d.update(causes=[dict(cause='discharge',count_delta=0)]),
            'receipt_ref without cause':lambda d:d.update(receipt_ref='receipt:use:k1'),
            'receipt cause without ref':lambda d:d.update(causes=[dict(cause='receipt:use:k1',count_delta=0)]),
            'receipt_refs list':lambda d:(d.pop('receipt_ref'),d.update(receipt_refs=[])),
            'charge off rhand':lambda d:d['before'].update(slot='lhand'),
        }
        mutations.pop('discharge cause on consume (D93)')
        consume=self.transition_rows('consume');consume[3]['data']['causes'][0]['cause']='shot_discharge:transition:1'
        with self.assertRaises(ValueError):gate.validate(consume,self.recording,HASH)
        # A cast discharge names the skill; it is not tied to a resolution id (cast:<id>).
        cast=copy.deepcopy(rows);cast[3]['data'].update(causes=[dict(cause='cast_discharge:1011',count_delta=0)],resolution_id='cast:7')
        gate.validate(cast,self.recording,HASH)
        # D96: its resolution_id carries the cast: prefix.
        for resolution in ('banana','transition:1','cast:','attack:7'):
            bad=copy.deepcopy(cast);bad[3]['data']['resolution_id']=resolution
            with self.assertRaisesRegex(ValueError,'D96',msg=resolution):gate.validate(bad,self.recording,HASH)
        for name,mutate in mutations.items():
            with self.subTest(mutation=name):
                bad=copy.deepcopy(rows);mutate(bad[3]['data'])
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        print(f'D73: use/receipt-bound use/three-discharge chain pass; {len(mutations)} use mutations reject',flush=True)

    def last_change_rows(self,baseline=True):
        """D79 (zone session 7 Q1): baseline rev1 weapon untouched while the owner steps to 2."""
        rows=copy.deepcopy(self.rows)
        weapon=item(100,1,71)|{'revision':1};shots=item(101,5,2509)
        if baseline:
            rows.insert(1,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=0,sequence=0,
                data=dict(inventory_revision=1,instances=[copy.deepcopy(weapon),copy.deepcopy(shots)],paperdoll={},auto_shots=[])))
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=1,sequence=0,
            data=dict(operation='consume',instance=101,before=copy.deepcopy(shots),after=shots|{'count':4,'revision':2},
                      count_delta=-1,operation_ordinal=0,resolution_id='attack:1',receipt_refs=[],
                      causes=[dict(cause='auto_shot:attack:1',count_delta=-1)])))
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=2,sequence=0,
            data=dict(operation='equip',instance=100,before=copy.deepcopy(weapon),after=weapon|{'slot':'rhand','location':'paperdoll','revision':3},
                      count_delta=0,operation_ordinal=0,resolution_id='equip:2',receipt_ref='receipt:equip:k2')))
        rows[0]['watermark'].update(records=3,last_tick=2);rows[-1].update(ticks=3,last_tick=2)
        return self.repair(rows)

    def test_d79_untouched_items_keep_their_last_change_revision(self):
        equip=lambda rows:next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['operation']=='equip')
        for baseline in (True,False):
            with self.subTest(baseline=baseline):
                rows=self.last_change_rows(baseline);gate.validate(rows,self.recording,HASH)
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
                steps=gate.inventory_history(rows)
                self.assertEqual(steps[(OWNER,1,2)],(2,3))
                # The withdrawn re-stamp: the untouched weapon "advanced" to the owner's previous step.
                # Only a baseline establishes the last change; a baseline-free slice cannot tell.
                bad=copy.deepcopy(rows);equip(bad)['before']['revision']=2
                if baseline:
                    with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
                else:gate.validate(bad,self.recording,HASH)
                # First touch at the tick's own step, or ahead of it.
                for value in (3,4):
                    bad=copy.deepcopy(rows);equip(bad)['before']['revision']=value
                    with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
                # The post-state still carries the owner's step for that tick.
                for value in (1,2,4):
                    bad=copy.deepcopy(rows);equip(bad)['after']['revision']=value
                    with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        # A retained baseline cannot hold an item revised after the inventory revision.
        bad=self.last_change_rows();bad[1]['data']['instances'][0]['revision']=2
        with self.assertRaisesRegex(ValueError,'ahead of inventory revision'):gate.validate(bad,self.recording,HASH)
        # A later baseline (reconnect) carries the untouched weapon at revision 1 next to the rev2 stack.
        rows=self.last_change_rows()
        rows.insert(-2,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=2,sequence=0,
            data=dict(inventory_revision=2,instances=[item(100,1,71)|{'revision':1},item(101,4,2509)|{'revision':2}],paperdoll={},auto_shots=[])))
        self.repair(rows);gate.validate(rows,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[-3]['data']['instances'][0]['revision']=2
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        # Review-2 P2-1: a baseline-free removal-only tick still advances the owner step.
        def removal_then(kind,revision):
            rows=self.transition_rows('destroy')
            del rows[1] # baseline-free slice: tick 0 destroys instance 100 (before revision 0)
            if kind=='baseline':
                rows.insert(-1,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=1,sequence=0,
                    data=dict(inventory_revision=revision,instances=[],paperdoll={},auto_shots=[])))
            else:
                rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=1,sequence=0,
                    data=dict(operation='grant',instance=300,before=None,after=item(300,3,1458)|{'revision':revision},
                              count_delta=3,operation_ordinal=0,resolution_id='grant:300',receipt_ref=None)))
            rows[0]['watermark'].update(records=2,last_tick=1);rows[-1].update(ticks=2,last_tick=1)
            return self.repair(rows)
        for kind,bad_revision,good_revision in (('baseline',0,1),('grant',1,2)):
            with self.subTest(kind=kind):
                gate.validate(removal_then(kind,good_revision),self.recording,HASH)
                with self.assertRaises(ValueError):gate.validate(removal_then(kind,bad_revision),self.recording,HASH)
        print('D79: last-change revisions pass with and without baselines; re-stamped/ahead befores and wrong post steps reject',flush=True)

    def test_d83_positional_ordinals_for_every_chain(self):
        # A Consume-free chain: equip then unequip of one weapon in one tick (positions 0, 1).
        rows=self.transition_rows('equip');equip=rows[3]
        unequip=copy.deepcopy(equip);unequip['data'].update(operation='unequip',operation_ordinal=1,resolution_id='transition:2',
            before=copy.deepcopy(equip['data']['after']),after=equip['data']['before']|{'revision':1})
        rows.insert(-1,unequip);self.repair(rows);gate.validate(rows,self.recording,HASH)
        for ordinals in ((0,0),(1,2),(0,2),(1,0),(2,1)):
            with self.subTest(ordinals=ordinals):
                bad=copy.deepcopy(rows);bad[3]['data']['operation_ordinal'],bad[4]['data']['operation_ordinal']=ordinals
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        # Single-row chains are position 0 for every operation, with or without Consume.
        for operation in ALLOWED_CHANGES:
            with self.subTest(operation=operation):
                bad=self.transition_rows(operation);bad[3]['data']['operation_ordinal']=1
                with self.assertRaisesRegex(ValueError,'chain row position'):gate.validate(bad,self.recording,HASH)
        # Positions restart per tick: the same instance at tick 1 starts again at 0.
        later=copy.deepcopy(rows)
        again=copy.deepcopy(later[3]);again.update(tick=1);again['data'].update(resolution_id='transition:3',
            before=copy.deepcopy(later[4]['data']['after']),after=later[3]['data']['after']|{'revision':2})
        later.insert(-1,again);later[0]['watermark'].update(records=2,last_tick=1);later[-1].update(ticks=2,last_tick=1)
        self.repair(later);gate.validate(later,self.recording,HASH)
        later[-2]['data']['operation_ordinal']=2
        with self.assertRaisesRegex(ValueError,'chain row position'):gate.validate(later,self.recording,HASH)
        print('D83: positional ordinals hold for Consume-free and single-row chains and restart per tick',flush=True)

    def d94_rows(self,operations,count=5):
        """One stack (instance 100, template 2509) with the given ordered operations in one tick."""
        rows=self.transition_rows('consume',count=count);rows.pop(3)
        state=copy.deepcopy(rows[1]['data']['instances'][0])
        for ordinal,operation in enumerate(operations):
            before=copy.deepcopy(state)
            if operation=='consume':
                after=before|{'count':before['count']-2,'revision':1}
                data=dict(receipt_refs=[],causes=[dict(cause='auto_shot:attack:1',count_delta=-2)],resolution_id='attack:1')
            elif operation=='grant':
                after=before|{'count':before['count']+1,'revision':1};data=dict(receipt_ref='receipt:grant:1',resolution_id='grant:1')
            else:
                after=None;data=dict(receipt_ref=f'receipt:{operation}:1',resolution_id=f'{operation}:1')
            data.update(operation=operation,instance=100,before=before,after=after,
                        count_delta=(after['count'] if after else 0)-before['count'],operation_ordinal=ordinal)
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,data=data))
            # After a terminal row the mutations reuse the last live state, so the D94 shape
            # rule (not only continuity) is what must refuse them.
            state=after if after is not None else before
        return self.repair(rows)

    def test_d94_consume_then_one_terminal_row(self):
        controls=(['consume','destroy'],['consume','crystallize'],['grant','consume','destroy'],['grant','destroy'],
                  ['destroy'],['grant','consume'])
        for operations in controls:
            with self.subTest(control=operations):
                rows=self.d94_rows(operations);gate.validate(rows,self.recording,HASH)
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        mutations=(['destroy','consume'],['consume','grant'],['consume','destroy','destroy'],['destroy','destroy'],
                   ['consume','consume','destroy'],['consume','crystallize','consume'])
        for operations in mutations:
            with self.subTest(mutation=operations):
                with self.assertRaises(ValueError):gate.validate(self.d94_rows(operations,count=9),self.recording,HASH)
        bad=self.d94_rows(['consume','destroy'])
        bad[3]['data']['operation_ordinal'],bad[4]['data']['operation_ordinal']=1,0
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        print(f'D94: {len(controls)} [non-terminal..., Consume, terminal] controls pass; {len(mutations)+1} shape/order mutations reject',flush=True)

    def test_system_shot_review11_field_drift_without_enchant_correlation(self):
        rows=self.transition_rows('consume',enchant=0)
        # Exercise the shared transition rule without baseline/history comparisons.
        del rows[1];self.repair(rows)
        delta=rows[2]['data'];delta['after']['count']=2;delta['count_delta']=-3
        delta['causes']=[dict(cause='auto_shot:transition:1',count_delta=-3)]
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        for field in ('template_id','enchant','created_tick','slot','augment_id','id'):
            bad=copy.deepcopy(rows);self.corrupt_transition(bad,field)
            with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        exhausted=copy.deepcopy(rows);delta=exhausted[2]['data']
        delta.update(after=None,count_delta=-5,causes=[dict(cause='auto_shot:transition:1',count_delta=-5)])
        gate.validate(exhausted,self.recording,HASH)
        for after,count_delta in ((None,-3),(item(100,1,2509)|{'revision':1},-5),(None,-6)):
            bad=copy.deepcopy(rows);bad[2]['data'].update(after=after,count_delta=count_delta,
                causes=[dict(cause='auto_shot:transition:1',count_delta=count_delta)])
            with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)

    def exchange_rows(self,recipe_id=4316442132481,enchant=3,extra=0):
        rows=self.transition_rows('consume',count=1)
        recipe=next(r for r in gate.table('sa_variants')['recipes'] if r['recipe_id']==recipe_id)
        consume=next(r['data'] for r in rows if r['kind']=='item_delta')
        consume['before'].update(template_id=recipe['base_template'],enchant=enchant)
        rows[1]['data']['instances'][0]=copy.deepcopy(consume['before'])
        consume.update(after=None,resolution_id='exchange:1',receipt_refs=['receipt:exchange:key'],
                       causes=[dict(cause='receipt:exchange:key',count_delta=-1)])
        ingredients_before={};ingredients_after={}
        for index,ingredient in enumerate(recipe['ingredients']):
            template,count=ingredient['template_id'],ingredient['count']
            if template==recipe['base_template'] or not count:continue
            # D68: a non-stackable ingredient (soul crystal 4645) is one instance of count 1.
            available=count+(extra if gate.item_tables()[template]['stackable'] else 0)+(210000 if template==57 else 0)
            before=item(102+index-1,available,template)
            after=before|{'count':available-count,'revision':1} if available>count else None
            rows[1]['data']['instances'].append(copy.deepcopy(before))
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
                data=dict(operation='consume',instance=before['id'],before=before,after=after,count_delta=-count,
                          operation_ordinal=0,resolution_id='exchange:1',receipt_refs=['receipt:exchange:key'],
                          causes=[dict(cause='receipt:exchange:key',count_delta=-count)])))
            ingredients_before[str(template)]=available;ingredients_after[str(template)]=available-count
        product=item(101,1,recipe['products'][0]['template_id'])|{'enchant':enchant if recipe['maintain_enchantment'] else 0,'revision':1}
        grant=dict(operation='grant',instance=101,before=None,after=product,
                   count_delta=1,operation_ordinal=0,resolution_id='exchange:1',receipt_ref='receipt:exchange:key')
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,data=grant))
        rows[2]['data']=dict(formula='sa_exchange',resolution_id='exchange:1',baseline=False,
            inputs=dict(recipe_id=recipe_id,before_instance={k:consume['before'][k] for k in ('id','template_id','enchant','augment_id')},
                        after_instance={k:product[k] for k in ('id','template_id','enchant','augment_id')},
                        ingredients_before=ingredients_before,ingredients_after=ingredients_after,tax_rate='0'),
            actual=dict(product_template=product['template_id'],new_instance_id=101))
        return self.repair(rows)

    def test_exchange_constituents_require_linked_consumption_and_fresh_grant(self):
        rows=self.exchange_rows();gate.validate(rows,self.recording,HASH)
        for mutation in ('missing consume','missing grant','foreign receipt','foreign owner','fresh id','not exhausted','wrong operation','monolithic','reordered'):
            with self.subTest(mutation=mutation):
                bad=self.exchange_rows();consume=bad[3]['data'];grant=bad[-2]['data']
                if mutation=='missing consume':del bad[3]
                elif mutation=='missing grant':del bad[-2]
                elif mutation=='foreign receipt':grant['receipt_ref']='receipt:exchange:other'
                elif mutation=='foreign owner':bad[-2]['entity']=OTHER
                elif mutation=='fresh id':grant['instance']=100;grant['after']['id']=100
                elif mutation=='not exhausted':
                    bad[1]['data']['instances'][0]['count']=2;consume['before']['count']=2
                    consume['after']=consume['before']|{'count':1,'revision':1}
                elif mutation=='wrong operation':grant['operation']='starter'
                elif mutation=='monolithic':grant['operation']='exchange'
                else:bad[3],bad[-2]=bad[-2],bad[3]
                self.repair(bad)
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)

    def corrupt_exchange(self,rows,mutation):
        deltas=[r for r in rows if r['kind']=='item_delta']
        if mutation.startswith('delete '):
            template=int(mutation.split()[1])
            rows.remove(next(r for r in deltas if r['data']['before'] and r['data']['before']['template_id']==template))
        elif mutation=='underpay':
            debit=next(r['data'] for r in deltas if r['data']['before'] and r['data']['before']['template_id']==57)
            debit.update(after=debit['before']|{'count':debit['before']['count']-1,'revision':1},count_delta=-1)
            debit['causes']=[dict(cause='receipt:exchange:key',count_delta=-1)]
        elif mutation=='product template':deltas[-1]['data']['after']['template_id']=4681
        elif mutation=='product enchant':deltas[-1]['data']['after']['enchant']+=1
        elif mutation=='unbound resolution':deltas[-1]['data']['resolution_id']='unrelated'
        elif mutation=='missing resolution':rows.remove(next(r for r in rows if r['kind']=='item_formula'))
        else:raise AssertionError(mutation)
        return self.repair(rows)

    def test_review12_exact_recipe_control_and_five_conserved_corruptions(self):
        rows=self.exchange_rows()
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['sa_exchange'],HASH)['passed'])
        for mutation in ('delete 72','delete 4645','delete 2131','delete 57','underpay',
                         'product template','product enchant','unbound resolution','missing resolution'):
            with self.subTest(mutation=mutation):
                self.assert_rejected(self.corrupt_exchange(copy.deepcopy(rows),mutation),('sa_exchange',))
        print('Review12 Exchange: source recipe4316442132481 control passes; 5 repaired deletions/underpayment + 4 binding mutations reject',flush=True)

    def test_exchange_retained_stacks_split_ingredients_and_mixed_receipts(self):
        rows=self.exchange_rows(extra=3)
        adena=next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['instance']==104)
        adena['before']['count']+=3
        rows[1]['data']['instances'][-1]['count']+=3
        adena.update(resolution_id='cast:other',count_delta=adena['count_delta']-3,
                     receipt_refs=['receipt:use:other','receipt:exchange:key'])
        adena['causes'].insert(0,dict(cause='item_cast:cast:other',count_delta=-2))
        adena['causes'].insert(1,dict(cause='receipt:use:other',count_delta=-1))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['sa_exchange'],HASH)['passed'])
        # Split the gemstones across two retained stacks, with the exact recipe sum.
        gem=next(r for r in rows if r['kind']=='item_delta' and r['data']['instance']==103)
        part=copy.deepcopy(gem);part['data']['instance']=105
        for endpoint in ('before','after'):part['data'][endpoint]['id']=105
        for row,amount in ((gem,40),(part,57)):
            debit=row['data'];debit['count_delta']=-amount;debit['causes'][0]['count_delta']=-amount
            debit['after']['count']=debit['before']['count']-amount
        rows[1]['data']['instances'].append(copy.deepcopy(part['data']['before']))
        rows.insert(-2,part);self.repair(rows)
        inputs=rows[2]['data']['inputs']
        inputs['ingredients_before']['2131']=gem['data']['before']['count']+part['data']['before']['count']
        inputs['ingredients_after']['2131']=gem['data']['after']['count']+part['data']['after']['count']
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['sa_exchange'],HASH)['passed'])
        bad=copy.deepcopy(rows);debit=bad[-3]['data']
        debit['causes'][0]['count_delta']+=1;debit['count_delta']+=1;debit['after']['count']+=1
        self.assert_rejected(bad,('sa_exchange',))

    def test_seeded_exchange_constituent_corruptions_2000_cases(self):
        rng=random.Random(0xE613);mutations=('delete 72','delete 4645','delete 2131','delete 57','underpay',
                                          'product template','product enchant','unbound resolution','missing resolution')
        for case in range(2000):
            rows=self.exchange_rows(enchant=rng.randrange(30),extra=rng.randrange(100))
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['sa_exchange'],HASH)['passed'])
            for mutation in mutations:
                self.assert_rejected(self.corrupt_exchange(copy.deepcopy(rows),mutation),('sa_exchange',))
        print('Exchange property check: seed=0xE613; 2000 controls pass; 18000 constituent corruptions reject',flush=True)

    def grants_then_consume(self,count=3,debits=(1,1),credits=(3,)):
        rows=self.transition_rows('consume',count=count or 1,enchant=0)
        consume=rows.pop(-2)['data']
        before=consume['before'] if count else None
        if not count:rows[1]['data']['instances']=[]
        remaining=count
        for ordinal,amount in enumerate(credits):
            after=(before or item(100,1,2509))|{'count':remaining+amount,'revision':1}
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
                data=dict(operation='grant',instance=100,before=copy.deepcopy(before),after=after,count_delta=amount,
                          operation_ordinal=ordinal,resolution_id=f'grant:{ordinal}',receipt_ref=f'receipt:grant:{ordinal}')))
            before=after;remaining+=amount
        remaining-=sum(debits)
        consume.update(before=copy.deepcopy(before),after=before|{'count':remaining} if remaining else None,
                       count_delta=-sum(debits),operation_ordinal=len(credits),resolution_id='attack:0',
                       causes=[dict(cause=f'auto_shot:attack:{index}',count_delta=-amount) for index,amount in enumerate(debits)])
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,data=consume))
        return self.repair(rows)

    def corrupt_interleaving(self,rows,mutation):
        deltas=[r for r in rows if r['kind']=='item_delta']
        grant=deltas[0]['data'];consume=deltas[-1]['data']
        if mutation=='wrong order':
            # Build a locally valid consume-first chain, also repairing ordinals.
            count=grant['before']['count'] if grant['before'] else 0
            spent=-consume['count_delta']
            if count>=spent:
                before=copy.deepcopy(grant['before'])
                consume.update(before=before,after=before|{'count':count-spent,'revision':1} if count>spent else None)
                before=consume['after']
                for row in deltas[:-1]:
                    data=row['data'];count=(before['count'] if before else 0)+data['count_delta']
                    data.update(before=copy.deepcopy(before),after=data['after']|{'count':count})
                    before=data['after']
            rows.remove(deltas[-1]);rows.insert(rows.index(deltas[0]),deltas[-1])
            for ordinal,row in enumerate([r for r in rows if r['kind']=='item_delta']):row['data']['operation_ordinal']=ordinal
        elif mutation=='broken continuity':
            consume['before']['count']+=1
            consume['after']=(consume['after'] or consume['before'])|{'count':consume['before']['count']+consume['count_delta']}
        elif mutation=='conservation mismatch':grant['count_delta']+=1
        elif mutation=='consume ordinal':consume['operation_ordinal']+=1
        elif mutation=='positive ordinal':grant['operation_ordinal']+=1
        elif mutation=='tick-start revision':
            if grant['before']:grant['before']['revision']+=2
            else:grant['after']['revision']+=1
        elif mutation=='positive field drift':grant['after']['enchant']+=1
        else:raise AssertionError(mutation)
        return self.repair(rows)

    def test_d66_canonical_controls_and_order_continuity_conservation(self):
        for count,debits,expected in ((3,(1,1),[(3,6,3),(6,4,-2)]),
                                     (1,(2,),[(1,4,3),(4,2,-2)]),
                                     (0,(1,),[(None,3,3),(3,2,-1)])):
            rows=self.grants_then_consume(count=count,debits=debits)
            self.assertEqual([(r['data']['before']['count'] if r['data']['before'] else None,
                               r['data']['after']['count'],r['data']['count_delta'])
                              for r in rows if r['kind']=='item_delta'],expected)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
            for mutation in ('wrong order','broken continuity','conservation mismatch','consume ordinal',
                             'positive ordinal','tick-start revision','positive field drift'):
                with self.subTest(count=count,mutation=mutation):
                    with self.assertRaises(ValueError):gate.validate(self.corrupt_interleaving(copy.deepcopy(rows),mutation),self.recording,HASH)
        rows=self.grants_then_consume()
        for field in ('template_id','enchant','created_tick','slot','augment_id','id','charges','location'):
            bad=copy.deepcopy(rows);self.corrupt_transition(bad,field)
            with self.assertRaises(ValueError,msg=field):gate.validate(bad,self.recording,HASH)
        bad=self.corrupt_interleaving(copy.deepcopy(rows),'conservation mismatch')
        with self.assertRaisesRegex(ValueError,'tick count conservation'):gate.inventory_history(bad)
        # Exhaustion, multiple positives, next-tick carry and baseline-free slices.
        for count,debits,credits in ((1,(4,),(3,)),(0,(3,),(3,)),(3,(2,),(1,2,3))):
            control=self.grants_then_consume(count=count,debits=debits,credits=credits)
            gate.validate(control,self.recording,HASH)
            del control[1];self.repair(control);gate.validate(control,self.recording,HASH)
        later=copy.deepcopy(rows[-2]);later['tick']=1
        later['data'].update(before=item(100,4,2509)|{'revision':1},after=item(100,2,2509)|{'revision':2},
                             operation_ordinal=0,resolution_id='attack:next',causes=[dict(cause='auto_shot:attack:next',count_delta=-2)])
        rows.insert(-1,later);rows[0]['watermark'].update(records=2,last_tick=1)
        rows[-1].update(ticks=2,last_tick=1);self.repair(rows);gate.validate(rows,self.recording,HASH)
        print('D66: 3->6->4, 1->4->2, None->3->2 controls pass; consume-first/continuity/conservation mutations reject',flush=True)

    def test_d66_exchange_ingredients_accept_prior_grants_but_product_stays_fresh(self):
        rows=self.exchange_rows()
        debit_row=next(r for r in rows if r['kind']=='item_delta' and r['data']['instance']==104)
        debit=debit_row['data'];before=copy.deepcopy(debit['before']);after=before|{'count':before['count']+3,'revision':1}
        rows.insert(rows.index(debit_row),dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(operation='grant',instance=104,before=before,after=after,count_delta=3,
                      operation_ordinal=0,resolution_id='grant:other',receipt_ref='receipt:grant:other')))
        debit.update(before=copy.deepcopy(after),after=debit['after']|{'count':debit['after']['count']+3},operation_ordinal=1)
        inputs=rows[2]['data']['inputs']
        for field in ('ingredients_before','ingredients_after'):inputs[field]['57']+=3
        self.repair(rows);self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['sa_exchange'],HASH)['passed'])
        bad=copy.deepcopy(rows);product=bad[-2]['data']
        product['before']=product['after']|{'count':1};product['after']['count']=2
        # Non-stackable 4682 count 2 now also fails D68; isolate the freshness rule too.
        with self.assertRaisesRegex(ValueError,'fresh product|template cap'):gate.validate(bad,self.recording,HASH)
        with self.assertRaisesRegex(ValueError,'fresh product'):gate.validate_exchanges(bad)

    def test_seeded_d66_interleaving_corruptions_2000_cases(self):
        rng=random.Random(0xD66);mutations=('wrong order','broken continuity','conservation mismatch','consume ordinal',
                                         'positive ordinal','tick-start revision','positive field drift')
        for case in range(2000):
            credits=tuple(rng.randint(1,100) for _ in range(rng.randint(1,4)))
            count=0 if case%3==0 else rng.randint(1,100)
            spent=count+sum(credits) if case%5==0 else rng.randint(1,count+sum(credits))
            rows=self.grants_then_consume(count=count,debits=(spent,),credits=credits)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
            for mutation in mutations:
                bad=self.corrupt_interleaving(copy.deepcopy(rows),mutation)
                with self.assertRaises(ValueError,msg=(case,mutation)):gate.validate(bad,self.recording,HASH)
        print('D66 property check: seed=0xD66; 2000 fresh/retained/exhaustion controls pass; 14000 corruptions reject',flush=True)

    def neutral_then_consume(self,count=10,spent=3,grant=0,unequip=False):
        """D69: arrows (template 17) equipped (and optionally topped up) then shot in one tick."""
        rows=copy.deepcopy(self.rows);start=item(100,count,17,'lhand' if unequip else None)
        rows.insert(1,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(inventory_revision=0,instances=[copy.deepcopy(start)],paperdoll={'lhand':100} if unequip else {},auto_shots=[])))
        chain=[]
        if grant:chain.append(('grant',{'count':count+grant},None,'receipt:grant:top'))
        chain.append(('unequip',{'slot':None,'location':'inventory'},None,'receipt:unequip:1') if unequip
                     else ('equip',{'slot':'lhand','location':'paperdoll'},None,'receipt:equip:1'))
        before=start
        for ordinal,(operation,change,_,ref) in enumerate(chain):
            after=before|change|{'revision':1}
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
                data=dict(operation=operation,instance=100,before=copy.deepcopy(before),after=after,
                          count_delta=after['count']-before['count'],operation_ordinal=ordinal,
                          resolution_id=f'{operation}:1',receipt_ref=ref)))
            before=after
        remaining=before['count']-spent
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(operation='consume',instance=100,before=copy.deepcopy(before),
                      after=before|{'count':remaining} if remaining else None,count_delta=-spent,
                      operation_ordinal=len(chain),resolution_id='attack:7',receipt_refs=[],
                      causes=[dict(cause=f'arrow:attack:{7+index}',count_delta=-1) for index in range(spent)])))
        return self.repair(rows)

    def test_d69_count_neutral_rows_precede_consume_with_continuity(self):
        controls=(dict(),dict(unequip=True),dict(grant=5),dict(count=3,spent=3),dict(grant=2,count=1,spent=3))
        for kwargs in controls:
            rows=self.neutral_then_consume(**kwargs)
            with self.subTest(**kwargs):
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
                deltas=[r for r in rows if r['kind']=='item_delta']
                # Consume not last: move it ahead of the neutral row(s), repairing ordinals and endpoints locally.
                bad=copy.deepcopy(rows);moved=[r for r in bad if r['kind']=='item_delta']
                bad.remove(moved[-1]);bad.insert(bad.index(moved[0]),moved[-1])
                for ordinal,row in enumerate(r for r in bad if r['kind']=='item_delta'):row['data']['operation_ordinal']=ordinal
                self.repair(bad)
                with self.assertRaisesRegex(ValueError,'must be last'):gate.validate(bad,self.recording,HASH)
                # A neutral row after Consume (two-tick-style equip appended in the same tick).
                if deltas[-1]['data']['after'] is not None:
                    bad=copy.deepcopy(rows);last=bad[-2]['data']['after']
                    flip=dict(slot=None,location='inventory') if last['slot'] else dict(slot='lhand',location='paperdoll')
                    bad.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
                        data=dict(operation='unequip' if last['slot'] else 'equip',instance=100,before=copy.deepcopy(last),
                                  after=last|flip,count_delta=0,operation_ordinal=len(deltas),resolution_id='late:1',receipt_ref=None)))
                    self.repair(bad)
                    with self.assertRaisesRegex(ValueError,'must be last'):gate.validate(bad,self.recording,HASH)
                # Broken continuity: Consume starts from the pre-equip location.
                bad=copy.deepcopy(rows);consume=bad[-2]['data']
                consume['before'].update(slot=deltas[0]['data']['before']['slot'],location=deltas[0]['data']['before']['location'])
                if consume['after']:consume['after'].update(slot=consume['before']['slot'],location=consume['before']['location'])
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
                # Removal never precedes Consume; two Consume aggregates refuse.
                bad=copy.deepcopy(rows);bad[-3]['data'].update(operation='destroy',after=None,count_delta=-bad[-3]['data']['before']['count'])
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
                bad=copy.deepcopy(rows);extra=copy.deepcopy(bad[-2]);extra['data']['operation_ordinal']+=1
                bad.insert(-1,extra);self.repair(bad)
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        print('D69: equip/unequip(+grant) then Consume controls pass; consume-not-last, neutral-after-consume, discontinuity, removal and duplicate aggregate reject',flush=True)

    def exhausted_then_fresh_grant(self,entity=OWNER,incarnation=1):
        rows=self.transition_rows('consume',count=2,enchant=0)
        debit=rows[-2]['data']
        debit.update(after=None,count_delta=-2,causes=[dict(cause='auto_shot:transition:1',count_delta=-2)])
        rows.insert(-1,dict(kind='item_delta',entity=entity,incarnation=incarnation,tick=1,sequence=0,
            data=dict(operation='grant',instance=200,before=None,after=item(200,3,2509)|{'revision':2 if entity==OWNER and incarnation==1 else 1},
                      count_delta=3,operation_ordinal=0,resolution_id='grant:fresh',receipt_ref=None)))
        rows[0]['watermark'].update(records=2,last_tick=1);rows[-1].update(ticks=2,last_tick=1)
        return self.repair(rows)

    def reuse_exhausted_id(self,rows,field=None,value=None):
        grant=rows[-2]['data'];grant['instance']=100;grant['after']['id']=100
        if field:grant['after'][field]=value
        return rows

    def test_review13_exhausted_id_field_corruptions_and_global_freshness(self):
        fields=((None,None),('template_id',2510),('enchant',99),('created_tick',99),('augment_id',65537))
        for entity,incarnation in ((OWNER,1),(OTHER,1),(OWNER,2)):
            rows=self.exhausted_then_fresh_grant(entity,incarnation)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
            for field,value in fields:
                with self.subTest(entity=entity,incarnation=incarnation,field=field):
                    bad=self.reuse_exhausted_id(copy.deepcopy(rows),field,value)
                    self.assert_rejected(bad,('pole_damage',))
                    with self.assertRaisesRegex(ValueError,'fresh instance'):gate.validate_instance_history(bad)
        # Exact review13 same-tick consume-to-null/regrant is now illegal under D66/D67.
        rows=self.exhausted_then_fresh_grant();rows[-2]['tick']=0
        rows[-2]['data'].update(operation_ordinal=1,after=item(200,3,2509)|{'revision':1})
        for field,value in fields:
            bad=self.reuse_exhausted_id(copy.deepcopy(rows),field,value)
            self.assert_rejected(bad,('pole_damage',))
            with self.assertRaisesRegex(ValueError,'fresh instance'):gate.validate_instance_history(bad)
        # Retained-before resurrection and a new baseline must also honor the tombstone.
        rows=self.exhausted_then_fresh_grant();bad=self.reuse_exhausted_id(copy.deepcopy(rows))
        bad[-2]['data'].update(before=item(100,1,2509)|{'revision':1},count_delta=2)
        self.assert_rejected(bad,('pole_damage',))
        with self.assertRaisesRegex(ValueError,'exhausted'):gate.validate_instance_history(bad)
        bad=copy.deepcopy(rows);bad[-2].update(kind='item_baseline',entity=OTHER,
            data=dict(inventory_revision=1,instances=[item(100,1,2509)|{'revision':1}],paperdoll={},auto_shots=[]))
        self.assert_rejected(bad,('pole_damage',))
        print('D67/review13: fresh grants pass; exhausted ID reuse with template2509->2510/enchant0->99/created_tick0->99/augment null->65537 rejects across ticks/owners/incarnations',flush=True)

    def test_d67_seen_retained_ids_and_observed_before_bindings_are_not_fresh(self):
        rows=self.exhausted_then_fresh_grant(OTHER)
        rows[-3]['data'].update(after=item(100,1,2509)|{'revision':1},count_delta=-1,
                               causes=[dict(cause='auto_shot:transition:1',count_delta=-1)])
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        self.assert_rejected(self.reuse_exhausted_id(copy.deepcopy(rows)),('pole_damage',))
        # A baseline alone, even under another owner, already establishes allocation.
        del rows[-3];self.repair(rows)
        self.assert_rejected(self.reuse_exhausted_id(copy.deepcopy(rows)),('pole_damage',))
        for operation in ('starter','backfill'):
            bad=self.reuse_exhausted_id(copy.deepcopy(rows));bad[-2]['data']['operation']=operation
            self.assert_rejected(bad,('pole_damage',))
        # Before bindings establish existence even in a baseline-free projection.
        rows=self.enchant_rows();grant=copy.deepcopy(self.exhausted_then_fresh_grant()[-2])
        grant.update(tick=0);grant['data'].update(instance=8,after=item(8,3,71)|{'revision':1},operation_ordinal=1)
        rows.insert(-1,grant);self.repair(rows)
        with self.assertRaisesRegex(ValueError,'fresh instance'):gate.validate_instance_history(rows)

    def test_review13_exchange_exhausted_ingredient_cannot_be_product_id(self):
        rows=self.exchange_rows()
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['sa_exchange'],HASH)['passed'])
        bad=copy.deepcopy(rows);grant=bad[-2]['data'];inputs=bad[2]['data']['inputs']
        self.assertEqual(next(r['data']['before']['template_id'] for r in bad if r['kind']=='item_delta' and r['data']['instance']==102),4645)
        grant.update(instance=102,operation_ordinal=1);grant['after']['id']=102
        inputs['after_instance']['id']=102;bad[2]['data']['actual']['new_instance_id']=102
        self.assert_rejected(bad,('sa_exchange',))
        with self.assertRaisesRegex(ValueError,'fresh instance'):gate.validate_instance_history(bad)
        # Isolate history from same-tick order and Exchange's existing constituent checks.
        grant.update(receipt_ref=None,operation_ordinal=0);bad[-2]['tick']=1
        grant['after']['revision']=2;bad[0]['watermark'].update(records=2,last_tick=1);bad[-1].update(ticks=2,last_tick=1)
        self.assert_rejected(bad,('sa_exchange',))
        print('Review13 Exchange: fresh product control passes; exhausted ingredient102/template4645 -> product2510 rejects',flush=True)

    BOUNDARY_DRIFT=(('template_id',2510),('enchant',99),('created_tick',99),('augment_id',65537))

    def boundary_rows(self,variant,field=None,value=None,template=2509,count=2):
        """Review14 F1-F3: one live ID re-observed after an incarnation/owner boundary."""
        rows=copy.deepcopy(self.rows);last=1
        rows.insert(1,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(inventory_revision=0,instances=[item(100,count,template)],paperdoll={},auto_shots=[])))
        seen=item(100,count,template)
        if field:seen[field]=value
        entity,incarnation=(OTHER,1) if variant in ('F2','F2b') else (OWNER,2)
        def append(tick,kind,data):rows.insert(-1,dict(kind=kind,entity=entity,incarnation=incarnation,tick=tick,sequence=0,data=data))
        if variant in ('control','F1','F1d','F2','F2b'):
            append(1,'item_baseline',dict(inventory_revision=0,instances=[copy.deepcopy(seen)],paperdoll={},auto_shots=[]))
        if variant in ('control','control-no-baseline','F1','F1b','F1d'):
            append(1,'item_delta',dict(operation='consume',instance=100,before=copy.deepcopy(seen),after=seen|{'count':count-1,'revision':1},
                count_delta=-1,operation_ordinal=0,resolution_id='attack:9',receipt_refs=[],causes=[dict(cause='auto_shot:attack:9',count_delta=-1)]))
        if variant=='F1d':
            before=seen|{'count':count-1,'revision':1}
            append(2,'item_delta',dict(operation='grant',instance=100,before=before,after=before|{'count':count+3,'revision':2},
                count_delta=4,operation_ordinal=0,resolution_id='grant:9',receipt_ref='receipt:grant:9'))
            last=2
        if variant=='F3':
            append(1,'item_delta',dict(operation='refine',instance=100,before=copy.deepcopy(seen),after=seen|{'augment_id':65537,'revision':1},
                count_delta=0,operation_ordinal=0,resolution_id='refine:9',receipt_ref='receipt:refine:9'))
        rows[0]['watermark'].update(records=last+1,last_tick=last);rows[-1].update(ticks=last+1,last_tick=last)
        return self.repair(rows)

    def test_review14_identity_drift_across_incarnation_and_owner_boundaries(self):
        # Controls: unchanged re-admission (with and without baseline) and distinct fresh IDs per owner.
        for variant in ('control','control-no-baseline'):
            self.assertTrue(gate.evaluate(self.boundary_rows(variant),self.recording,OWNER,['pole_damage'],HASH)['passed'],variant)
        self.assertTrue(gate.evaluate(self.exhausted_then_fresh_grant(OTHER),self.recording,OWNER,['pole_damage'],HASH)['passed'])
        cases=0
        for variant in ('F1','F1b','F1d','F2'):
            for field,value in self.BOUNDARY_DRIFT:
                with self.subTest(variant=variant,field=field):
                    bad=self.boundary_rows(variant,field,value)
                    self.assert_rejected(bad,('pole_damage',))
                    with self.assertRaisesRegex(ValueError,'two owners' if variant=='F2' else 'identity drifted'):gate.validate_instance_history(bad)
                    cases+=1
        # F2b: identical metadata, but two live owners hold one ID at once.
        bad=self.boundary_rows('F2b');self.assert_rejected(bad,('pole_damage',))
        with self.assertRaisesRegex(ValueError,'two owners'):gate.validate_instance_history(bad)
        # F3: retained refine whose before rewrites the template (71 -> 72).
        self.assertTrue(gate.evaluate(self.boundary_rows('F3',template=71,count=1),self.recording,OWNER,['pole_damage'],HASH)['passed'])
        bad=self.boundary_rows('F3','template_id',72,template=71,count=1);self.assert_rejected(bad,('pole_damage',))
        with self.assertRaisesRegex(ValueError,'identity drifted'):gate.validate_instance_history(bad)
        print(f'Review14 F1/F1b/F1d/F2/F2b/F3: {cases+2} boundary identity drifts reject; unchanged re-admission and distinct-ID controls pass',flush=True)

    def readmission_rows(self,variant,field=None,value=None):
        """D71: a charged rhand weapon retained across a re-admission (incarnation 1 -> 2)."""
        rows=copy.deepcopy(self.rows)
        weapon=item(100,1,71,'rhand')|{'charges':1}
        rows.insert(1,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(inventory_revision=0,instances=[copy.deepcopy(weapon)],paperdoll={'rhand':100},auto_shots=[5789])))
        incarnation=1 if variant=='same-incarnation' else 2
        seen=weapon|{'charges':0}
        if field:seen[field]=value
        if field=='slot':seen['location']='paperdoll' if value else 'inventory'
        paperdoll={seen['slot']:100} if seen['slot'] else {}
        if variant!='no-baseline':
            rows.insert(-1,dict(kind='item_baseline',entity=OWNER,incarnation=incarnation,tick=1,sequence=0,
                data=dict(inventory_revision=0,instances=[copy.deepcopy(seen)],paperdoll=paperdoll,auto_shots=[])))
        before=copy.deepcopy(seen)
        if variant=='later-drift':before['charges']=1 # charges change again inside incarnation 2 with no row.
        # Every variant observes the instance through a ledger before as well (unequip, or equip when drifted out).
        operation,moved=('unequip',{'slot':None,'location':'inventory','charges':0}) if before['slot'] else ('equip',{'slot':'rhand','location':'paperdoll'})
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=incarnation,tick=1,sequence=0,
            data=dict(operation=operation,instance=100,before=before,after=before|moved|{'revision':1},
                      count_delta=0,operation_ordinal=0,resolution_id=f'{operation}:9',receipt_ref=None)))
        rows[0]['watermark'].update(records=2,last_tick=1);rows[-1].update(ticks=2,last_tick=1)
        return self.repair(rows)

    def test_d71_readmission_clears_only_shot_charges(self):
        # Controls: charges (and the auto-shot set) cleared on re-admission, with and without a baseline.
        for variant in ('control','no-baseline'):
            rows=self.readmission_rows(variant)
            gate.validate_instance_history(rows)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'],variant)
        cases=0
        # Slot/location drift at the boundary still rejects (the paperdoll is retained, D16).
        for variant,field,value in [('control','slot',None),('no-baseline','slot',None),('control','slot','lhand')]+[('control',f,v) for f,v in self.BOUNDARY_DRIFT if f!='template_id']:
            with self.subTest(variant=variant,field=field,value=value):
                bad=self.readmission_rows(variant,field,value)
                self.assert_rejected(bad,('pole_damage',))
                with self.assertRaisesRegex(ValueError,'identity drifted'):gate.validate_instance_history(bad)
                cases+=1
        # Charges drift inside one incarnation still rejects: same incarnation, and again after the boundary.
        for variant in ('same-incarnation','later-drift'):
            with self.subTest(variant=variant):
                bad=self.readmission_rows(variant);self.assert_rejected(bad,('pole_damage',))
                with self.assertRaisesRegex(ValueError,'identity drifted'):gate.validate_instance_history(bad)
                cases+=1
        print(f'D71: charges/auto-shots cleared on re-admission pass; {cases} slot/field/same-incarnation charge drifts reject',flush=True)

    def d72_rows(self,start,seen,auto_shots=(),baseline=True):
        """D72: inc1 weapon with `start` charges re-observed at inc2 with `seen` charges."""
        rows=self.readmission_rows('control' if baseline else 'no-baseline')
        first=next(r for r in rows if r['kind']=='item_baseline')
        first['data']['instances'][0]['charges']=start
        for row in rows[1:-1]:
            if row['incarnation']!=2:continue
            if row['kind']=='item_baseline':
                row['data']['instances'][0]['charges']=seen;row['data']['auto_shots']=list(auto_shots)
            elif row['kind']=='item_delta':row['data']['before']['charges']=seen
        if seen and baseline:
            # A still-charged weapon stays equipped: the inc2 baseline alone observes it.
            rows=self.repair([r for r in rows if r['kind']!='item_delta'])
        return rows

    def test_d72_readmission_charges_only_clear_and_toggles_reset(self):
        # Controls: cleared (1 -> 0), unchanged (1 -> 1), absent (0 -> 0), with and without the inc2 baseline.
        for start,seen,baseline in ((1,0,True),(1,0,False),(1,1,True),(4,4,True),(0,0,True)):
            with self.subTest(start=start,seen=seen,baseline=baseline):
                rows=self.d72_rows(start,seen,baseline=baseline);gate.validate_instance_history(rows)
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        cases=0
        # Mutations: a charge appears (0 -> 1, 0 -> 4) or changes kind (1 -> 2, 2 -> 6) at the boundary.
        for start,seen,baseline in ((0,1,True),(0,1,False),(0,4,True),(1,2,True),(2,6,False)):
            with self.subTest(start=start,seen=seen,baseline=baseline):
                bad=self.d72_rows(start,seen,baseline=baseline);self.assert_rejected(bad,('pole_damage',))
                with self.assertRaisesRegex(ValueError,'charges appeared at re-admission'):gate.validate_instance_history(bad)
                cases+=1
        # The owner's first baseline of a later incarnation must carry no auto-shot toggles.
        bad=self.d72_rows(1,0,auto_shots=[5789]);self.assert_rejected(bad,('pole_damage',))
        with self.assertRaisesRegex(ValueError,'auto-shot toggles survived'):gate.validate_instance_history(bad)
        cases+=1
        # A recording that starts at incarnation 2 (no earlier live life observed) keeps its toggles.
        rows=self.repair([r for r in self.d72_rows(0,0,auto_shots=[5789]) if r['kind'] in ('manifest','complete') or r['incarnation']==2])
        gate.validate_instance_history(rows)
        print(f'D72: cleared/unchanged charges pass; {cases} appearing charges / surviving toggles reject',flush=True)

    def foreign_binding_rows(self,mutation,identity=8,first=False):
        """Review15 F1: OTHER's malformed enchant binding names OWNER's live instance."""
        rows=copy.deepcopy(self.rows)
        formula=copy.deepcopy(self.enchant_rows(entity=OTHER)[1])
        target=formula['data']['operation']['target_before'];target['id']=identity
        if mutation=='extra-key':target['extra']=1
        elif mutation=='count':target['count']=2 # non-stackable template 71, cap 1 (D68)
        baseline=dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(inventory_revision=0,instances=[item(8,1,71)],paperdoll={},auto_shots=[]))
        rows[1:1]=[formula,baseline] if first else [baseline,formula]
        return self.repair(rows)

    def test_review15_foreign_malformed_enchant_binding_checks_owner_and_cap(self):
        self.assertTrue(gate.evaluate(self.foreign_binding_rows('extra-key',identity=99),self.recording,OWNER,['pole_damage'],HASH)['passed'])
        cases=0
        for mutation,message in (('extra-key','two owners'),('count','two owners|template cap'),('well-formed','two owners')):
            for first in (False,True):
                with self.subTest(mutation=mutation,first=first):
                    bad=self.foreign_binding_rows(mutation,first=first)
                    with self.assertRaisesRegex(ValueError,message):gate.validate_instance_history(bad)
                    with self.assertRaisesRegex(ValueError,message):gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH)
                    cases+=1
        # The cap applies to a foreign malformed binding even when no owner holds the ID.
        with self.assertRaisesRegex(ValueError,'template cap'):gate.validate_instance_history(self.foreign_binding_rows('count',identity=99))
        print(f'Review15 F1: {cases+1} foreign malformed/well-formed enchant bindings reject for the evaluated owner',flush=True)

    def fresh_grant(self,template,count,identity=400):
        rows=copy.deepcopy(self.rows)
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(operation='grant',instance=identity,before=None,after=item(identity,count,template)|{'revision':1},
                      count_delta=count,operation_ordinal=0,resolution_id='grant:cap',receipt_ref='receipt:grant:cap')))
        return self.repair(rows)

    def test_d68_pinned_count_caps_per_template_class(self):
        # character.properties:671 MaxAdena; L2ItemInstance.java:422 Integer.MAX_VALUE; E1 stackable=false -> 1.
        self.assertEqual(gate.count_caps(),(57,99_900_000_000,2**31-1))
        for template,cap in ((57,99_900_000_000),(17,2**31-1),(2509,2**31-1),(71,1),(4645,1)):
            with self.subTest(template=template):
                self.assertEqual(gate.count_cap(template),cap)
                self.assertTrue(gate.evaluate(self.fresh_grant(template,cap),self.recording,OWNER,['pole_damage'],HASH)['passed'])
                for count in (cap+1,cap*2):
                    bad=self.fresh_grant(template,count);self.assert_rejected(bad,('pole_damage',))
                    with self.assertRaisesRegex(ValueError,'template cap'):gate.validate(bad,self.recording,HASH)
        # Review14 F4 exact repros, plus a retained grant crossing the cap and an over-cap baseline.
        for template,count in ((57,10**15),(71,2)):
            with self.assertRaisesRegex(ValueError,'template cap'):gate.validate(self.fresh_grant(template,count),self.recording,HASH)
        rows=self.grants_then_consume(count=3,debits=(1,),credits=(3,))
        for delta in (r['data'] for r in rows if r['kind']=='item_delta'):
            for state in (delta['before'],delta['after']):
                if state:state['template_id']=57
        rows[1]['data']['instances'][0]['template_id']=57
        gate.validate(rows,self.recording,HASH)
        # D74: the legal C -> C+1 -> C intermediate (export order shows grant before the same-tick
        # consumption) passes; a tick end above the cap, or a positive row whose after exceeds the cap by
        # more than the consumption that follows it, rejects.
        legal=copy.deepcopy(rows);grant,consume=(r['data'] for r in legal if r['kind']=='item_delta')
        grant['after']['count']=99_900_000_001;grant['count_delta']=99_900_000_001-3
        consume['before']['count']=99_900_000_001;consume['after']['count']=99_900_000_000
        gate.validate(legal,self.recording,HASH)
        bad=copy.deepcopy(legal);grant,consume=(r['data'] for r in bad if r['kind']=='item_delta')
        grant['after']['count']=99_900_000_002;grant['count_delta']=99_900_000_002-3
        consume['before']['count']=99_900_000_002;consume['after']['count']=99_900_000_001
        with self.assertRaisesRegex(ValueError,'template cap'):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(legal);grant,consume=(r['data'] for r in bad if r['kind']=='item_delta')
        grant['after']['count']=99_900_000_003;grant['count_delta']=99_900_000_003-3
        consume['before']['count']=99_900_000_003;consume['after']['count']=99_900_000_002;consume['count_delta']=-1
        with self.assertRaisesRegex(ValueError,'template cap'):gate.validate(bad,self.recording,HASH)
        # Tick-start endpoint above the cap (baseline-free slice) rejects even when consumption follows.
        bad=copy.deepcopy(legal);del bad[1];self.repair(bad)
        grant,consume=(r['data'] for r in bad if r['kind']=='item_delta')
        grant['before']['count']=99_900_000_001;grant['after']['count']=99_900_000_004;grant['count_delta']=3
        consume['before']['count']=99_900_000_004;consume['after']['count']=99_900_000_000;consume['count_delta']=-4
        consume['causes']=[dict(cause='auto_shot:attack:0',count_delta=-4)]
        with self.assertRaisesRegex(ValueError,'template cap'):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[1]['data']['instances'][0]['count']=99_900_000_001
        with self.assertRaisesRegex(ValueError,'template cap'):gate.validate(bad,self.recording,HASH)
        print('D68: adena 99900000000 / stackable 2147483647 / non-stackable 1 at-cap controls pass; D74 C->C+1->C intermediate passes; over-cap grants, tick ends/starts and baselines reject',flush=True)

    def test_seeded_d67_id_reuse_corruptions_2000_cases(self):
        rng=random.Random(0xD67);corruptions=0
        fields=((None,None),('template_id',2510),('enchant',99),('created_tick',99),('augment_id',65537))
        for case in range(2000):
            entity,incarnation=((OWNER,1),(OTHER,1),(OWNER,2))[case%3]
            rows=self.exhausted_then_fresh_grant(entity,incarnation)
            rows[-2]['data']['after']['count']=rows[-2]['data']['count_delta']=rng.randint(1,100)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
            for field,value in fields:
                bad=self.reuse_exhausted_id(copy.deepcopy(rows),field,value)
                self.assert_rejected(bad,('pole_damage',));corruptions+=1
            # Review14: identity drift at an incarnation/owner boundary, any observation kind.
            variant=('F1','F1b','F1d','F2')[rng.randrange(4)];field,value=self.BOUNDARY_DRIFT[rng.randrange(4)]
            self.assert_rejected(self.boundary_rows(variant,field,value,count=rng.randint(2,100)),('pole_damage',));corruptions+=1
            self.assert_rejected(self.boundary_rows('F2b',count=rng.randint(2,100)),('pole_damage',));corruptions+=1
            exchange=self.exchange_rows(enchant=rng.randrange(30))
            self.assertTrue(gate.evaluate(exchange,self.recording,OWNER,['sa_exchange'],HASH)['passed'])
            exchange[-2]['data'].update(instance=102,operation_ordinal=1);exchange[-2]['data']['after']['id']=102
            exchange[2]['data']['inputs']['after_instance']['id']=102;exchange[2]['data']['actual']['new_instance_id']=102
            self.assert_rejected(exchange,('sa_exchange',));corruptions+=1
        print(f'D67/D69 property check: seed=0xD67; 4000 fresh grant/Exchange controls pass; {corruptions} ID/metadata reuse and boundary-drift corruptions reject',flush=True)

    def test_seeded_per_operation_field_corruptions_2000_cases(self):
        rng=random.Random(0xE612);controls=corruptions=0
        fields=('template_id','enchant','created_tick','slot','augment_id','id','count','charges','location')
        for case in range(2000):
            for operation,allowed in ALLOWED_CHANGES.items():
                rows=self.transition_rows(operation,count=rng.randint(2,100),enchant=rng.randrange(30),created_tick=rng.randrange(100))
                gate.validate(rows,self.recording,HASH);controls+=1
                forbidden=[field for field in fields if field not in allowed]
                field=forbidden[case%len(forbidden)]
                self.corrupt_transition(rows,field,rng.randint(1,100))
                with self.assertRaises(ValueError,msg=(case,operation,field)):gate.validate(rows,self.recording,HASH)
                corruptions+=1
            rows=self.exchange_rows();gate.validate(rows,self.recording,HASH);controls+=1
            # Corrupt alternate constituent operations, keeping the Exchange link.
            self.corrupt_transition(rows,fields[case%6],rng.randint(1,100),delta_index=0 if case%2==0 else -1)
            self.repair(rows)
            with self.assertRaises(ValueError,msg=('exchange',case)):gate.validate(rows,self.recording,HASH)
            corruptions+=1
        print(f'Operation property check: seed=0xE612; {controls} controls pass; {corruptions} per-operation field corruptions reject',flush=True)

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
               operation=dict(target_before=target,scroll_before=scroll,support_before=support_item,receipt_ref='receipt:enchant:1')))
        chance_inputs={k:inputs[k] for k in ('base','bonus','support')}
        append('item_formula',dict(formula='enchant_chance',inputs=chance_inputs,actual=gate.oracle.enchant_chance(**chance_inputs),baseline=False))
        append('item_rng',dict(callsite='enchant',word_before=0,word_after=1,bound=100000,value=residue))
        for spent in (scroll,support_item):
            if spent:append('item_delta',dict(operation='consume',instance=spent['id'],before=spent,after=None,count_delta=-1,operation_ordinal=0,receipt_refs=['receipt:enchant:1'],causes=[dict(cause='receipt:enchant:1',count_delta=-1)]))
        if result['destroyed'] or result['enchant']!=enchant:
            after=None if result['destroyed'] else target|dict(enchant=result['enchant'],revision=1)
            append('item_delta',dict(operation='destroy' if result['destroyed'] else 'enchant',instance=target_id,before=target,after=after,count_delta=-1 if after is None else 0,operation_ordinal=0,receipt_ref='receipt:enchant:1'))
        if result['destroyed']:
            definition=gate.item_tables()[71];grade=gate.source().grades[definition['grade']]
            refund_inputs=dict(base=definition['crystal_count'],bonus=grade['weapon_bonus'],enchant=enchant,weapon=True,crystallizable=definition['flags']['is_crystallizable'],crystal_id=grade['crystal_id'])
            refund=gate.oracle.failure_refund(**refund_inputs)
            append('item_formula',dict(formula='failure_refund',inputs=refund_inputs,actual=refund,baseline=False))
            if refund:append('item_delta',dict(operation='grant',instance=10,before=None,after=item(10,refund,grade['crystal_id'])|{'revision':1},count_delta=refund,operation_ordinal=0,receipt_ref='receipt:enchant:1'))
        rows.append(dict(kind='complete',facts=len(rows)-1,ticks=1,last_tick=0))
        return rows

    def same_tick_enchants(self):
        rows=[copy.deepcopy(self.rows[0])]
        for index in range(2):
            attempt=self.enchant_rows(enchant=index,scroll_id=20+index)
            attempt[1]['data']['operation']['target_before']['revision']=min(index,1)
            attempt[1]['data']['operation']['receipt_ref']=f'receipt:enchant:{index+1}'
            for row in attempt[1:-1]:
                row['data']['resolution_id']=f'enchant:{index+1}'
                if row['kind']=='item_rng':row['data'].update(word_before=index,word_after=index+1)
                if row['kind']=='item_delta':
                    if row['data']['operation']=='consume':
                        row['data']['receipt_refs']=[f'receipt:enchant:{index+1}']
                        row['data']['causes']=[dict(cause=f'receipt:enchant:{index+1}',count_delta=-1)]
                    else:row['data']['receipt_ref']=f'receipt:enchant:{index+1}'
                    if row['data']['instance']==8:row['data']['operation_ordinal']=index
                rows.append(row)
        rows.append(dict(kind='complete',facts=0,ticks=1,last_tick=0))
        return self.repair(rows)

    def shared_stack_enchants(self,count=2,support=False):
        rows=self.same_tick_enchants()
        outcomes=[r['data'] for r in rows if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome']
        for index,outcome in enumerate(outcomes):
            outcome['operation']['scroll_before'].update(id=20,count=count-index,revision=min(index,1))
            if support:
                outcome['operation']['support_before']=item(9,count-index,12363)|{'revision':min(index,1)}
                outcome['inputs']['support']=18
                outcome['actual']=gate.oracle.enchant_outcome(**outcome['inputs'])
        for row in rows:
            if support and row['kind']=='item_formula' and row['data']['formula']=='enchant_chance':
                row['data']['inputs']['support']=18
                row['data']['actual']=gate.oracle.enchant_chance(**row['data']['inputs'])
        rows=[r for r in rows if r['kind']!='item_delta' or r['data']['operation']!='consume']
        for key in ('scroll_before','support_before'):
            before=outcomes[0]['operation'][key]
            if before is None:continue
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
                data=dict(operation='consume',instance=before['id'],before=before,
                          after=before|{'count':count-2,'revision':1} if count>2 else None,
                          count_delta=-2,operation_ordinal=0,resolution_id='enchant:1',receipt_refs=['receipt:enchant:1','receipt:enchant:2'],
                          causes=[dict(cause=ref,count_delta=-1) for ref in ('receipt:enchant:1','receipt:enchant:2')])))
        return self.repair(rows)

    def test_shared_scroll_stack_consumes_ordered_resolution_receipts(self):
        for count,support in ((2,False),(3,False),(2,True),(3,True)):
            rows=self.shared_stack_enchants(count,support)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'],(count,support))
            for index in range(1,len(rows)-1):
                bad=copy.deepcopy(rows);del bad[index];self.repair(bad)
                # D69 identity continuity may reject a deleted enchant step outright.
                self.assert_rejected(bad,('enchant_outcome','enchant_chance'))

    def test_shared_scroll_stack_rejects_contribution_order_and_total_mutations(self):
        rows=self.shared_stack_enchants()
        for receipts in (['receipt:enchant:1'],['receipt:enchant:2'],['receipt:enchant:2','receipt:enchant:1'],['receipt:enchant:1','receipt:enchant:1'],['receipt:enchant:1','receipt:enchant:2','receipt:enchant:3']):
            bad=copy.deepcopy(rows)
            next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')['receipt_refs']=receipts
            self.assert_rejected(bad)
        for total in (1,3):
            bad=copy.deepcopy(rows)
            aggregate=next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')
            if total==1:aggregate['after']=aggregate['before']|{'count':1,'revision':1}
            else:aggregate['before']=aggregate['before']|{'count':3}
            aggregate['count_delta']=-total
            self.assert_rejected(bad)
        for receipt in (None,'',[],{},False,'receipt:enchant:1','unrelated'):
            bad=copy.deepcopy(rows)
            [r for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome'][-1]['data']['operation']['receipt_ref']=receipt
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'],receipt)
        for field,value in (('entity',OTHER),('incarnation',2)):
            bad=copy.deepcopy(rows)
            next(r for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')[field]=value
            # OTHER raises D69 'two owners'; incarnation 2 breaks the per-incarnation receipt correlation (passed=False).
            self.assert_rejected(bad)
        bad=copy.deepcopy(rows)
        del bad[1]['data']['operation']['receipt_ref']
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        bad=copy.deepcopy(rows)
        next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='enchant')['receipt_ref']='unrelated'
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        bad=copy.deepcopy(rows)
        bad[0]['watermark'].update(records=2,last_tick=1);bad[-1].update(ticks=2,last_tick=1)
        next(r for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')['tick']=1
        self.assert_rejected(bad)
        bad=copy.deepcopy(rows)
        next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')['resolution_id']='unrelated'
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        bad=copy.deepcopy(rows)
        [r for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome'][-1]['data']['operation']['scroll_before']['count']=2
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])

    def test_same_tick_enchants_correlate_ordinals_with_shared_revision(self):
        rows=self.same_tick_enchants()
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'])
        for index in range(1,len(rows)-1):
            bad=copy.deepcopy(rows);del bad[index];self.repair(bad)
            self.assert_rejected(bad,('enchant_outcome','enchant_chance'))
        for ordinal in (0,):
            bad=copy.deepcopy(rows)
            targets=[r['data'] for r in bad if r['kind']=='item_delta' and r['data']['instance']==8]
            targets[-1]['operation_ordinal']=ordinal
            with self.assertRaisesRegex(ValueError,'ordinal'):gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)
        bad=copy.deepcopy(rows)
        targets=[r['data'] for r in bad if r['kind']=='item_delta' and r['data']['instance']==8]
        targets[0]['operation_ordinal'],targets[1]['operation_ordinal']=1,0
        with self.assertRaisesRegex(ValueError,'ordinal'):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows)
        duplicate=copy.deepcopy(next(r for r in bad if r['kind']=='item_delta' and r['data']['instance']==8))
        duplicate['data']['operation']='refine';bad.insert(-1,duplicate);self.repair(bad)
        with self.assertRaisesRegex(ValueError,'repeated ledger'):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows)
        next(r for r in reversed(bad) if r['kind']=='item_delta' and r['data']['instance']==8)['data']['after']['revision']=0
        with self.assertRaisesRegex(ValueError,'revision regressed'):gate.validate(bad,self.recording,HASH)

    def test_distinct_stacks_cannot_reuse_one_resolution_receipt(self):
        rows=self.same_tick_enchants()
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'])
        bad=copy.deepcopy(rows)
        for row in bad[1:-1]:
            data=row['data']
            if row['kind']=='item_formula' and data['formula']=='enchant_outcome':data['operation']['receipt_ref']='receipt:enchant:1'
            elif row['kind']=='item_delta':
                data['receipt_refs' if data['operation']=='consume' else 'receipt_ref']=['receipt:enchant:1'] if data['operation']=='consume' else 'receipt:enchant:1'
                if data['operation']=='consume':data['causes']=[dict(cause='receipt:enchant:1',count_delta=-1)]
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'])

    def test_shared_scroll_and_support_bindings_preserve_tick_revision(self):
        for count in (2,3):
            for key in ('scroll_before','support_before'):
                rows=self.shared_stack_enchants(count,support=True)
                outcomes=[r['data'] for r in rows if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome']
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
                for revision in (0,99):
                    bad=copy.deepcopy(rows)
                    [r['data'] for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome'][1]['operation'][key]['revision']=revision
                    self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'],(count,key,revision))
                # D79: the first binding is the stack's earlier last-change revision, so a first
                # binding at the tick's own step refuses; a later binding below the step rolls back.
                outcomes[0]['operation'][key]['revision']=1
                next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['instance']==outcomes[0]['operation'][key]['id'])['before']['revision']=1
                self.assert_rejected(copy.deepcopy(rows))
                outcomes[0]['operation'][key]['revision']=0
                next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['instance']==outcomes[0]['operation'][key]['id'])['before']['revision']=0
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
                outcomes[1]['operation'][key]['revision']=0
                self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])

    def generated_enchants(self,rng):
        ticks=rng.randint(2,3);stack_ids=rng.sample(range(20,40),rng.randint(1,3))
        plans=[[rng.choice(stack_ids) for _ in range(rng.randint(3,4))] for _ in range(ticks)]
        automatic_counts=[[rng.randint(0,2) for _ in plan] for plan in plans]
        support=rng.choice((False,True));initial=rng.randint(0,20)
        counts={identity:sum(plan.count(identity) for plan in plans)+rng.randint(0,3)+
                sum(cost for plan,costs in zip(plans,automatic_counts) for spent,cost in zip(plan,costs) if spent==identity)
                for identity in stack_ids}
        if support:counts[9]=sum(map(len,plans))+rng.randint(0,3)
        revisions={identity:initial for identity in counts}
        incarnation=rng.randint(1,5);start=rng.randint(0,30);cursor=0;levels={8:0,10:3}
        rows=[copy.deepcopy(self.rows[0])]
        baseline=[item(identity,count,12363 if identity==9 else 22227)|{'revision':initial} for identity,count in counts.items() if count]
        baseline.extend(item(identity,1,71)|{'revision':initial,'enchant':level} for identity,level in levels.items())
        baseline.append(item(50,ticks*3+1,1835)|{'revision':initial})
        rows.append(dict(kind='item_baseline',entity=OWNER,incarnation=incarnation,tick=start,sequence=0,
                         data=dict(inventory_revision=initial,instances=baseline,paperdoll={},auto_shots=[])))
        target_revisions={identity:initial for identity in levels}
        for tick,plan in enumerate(plans):
            # D79: revisions are last-change revisions; untouched stacks keep theirs.
            # D83: target rows take their positional chain ordinal (0..n-1 per instance and tick).
            contributions={};ordinals={identity:0 for identity in levels};changed=set()
            def automatic(identity,amount):
                if not amount:return
                resolution=f'cast:{cursor}:cost'
                before=item(identity,counts[identity],22227)|{'revision':revisions[identity]}
                contributions.setdefault(identity,[]).append((resolution,'item_cast:'+resolution,before,-amount))
                counts[identity]-=amount;revisions[identity]=initial+tick+1
            for index,scroll_id in enumerate(plan):
                target_id=8 if index in (0,2) else 10 if index==1 else rng.choice((8,10))
                level=levels[target_id];target=item(target_id,1,71)|{'enchant':level}
                base=int(gate.enchant_source_inputs(target,item(scroll_id,1,22227),item(9,1,12363) if support else None)['base'])
                no_change=index==1 or index>2 and level>=3 and rng.choice((False,True))
                attempt=self.enchant_rows(base=base,residue=99999 if no_change else 0,safe=True,
                                          enchant=level,target_id=target_id,scroll_id=scroll_id,support=support)
                resolution=f'enchant:{rng.getrandbits(64):016x}:{cursor}'
                receipt=f'receipt:enchant:{rng.getrandbits(64):016x}:{cursor}'
                operation=attempt[1]['data']['operation']
                operation['receipt_ref']=receipt
                operation['target_before']['revision']=target_revisions[target_id]
                auto_before=rng.choice((False,True));cost=automatic_counts[tick][index]
                if auto_before:automatic(scroll_id,cost)
                for key in ('scroll_before','support_before'):
                    before=operation[key]
                    if before is None:continue
                    identity=before['id'];before.update(count=counts[identity],revision=revisions[identity])
                    contributions.setdefault(identity,[]).append((resolution,receipt,copy.deepcopy(before),-1))
                    counts[identity]-=1;revisions[identity]=initial+tick+1
                if not auto_before:automatic(scroll_id,cost)
                for row in attempt[1:-1]:
                    data=row['data']
                    if row['kind']=='item_delta' and data['operation']=='consume':continue
                    row.update(tick=start+tick,incarnation=incarnation)
                    data['resolution_id']=resolution
                    if row['kind']=='item_rng':data.update(word_before=cursor,word_after=cursor+1)
                    if row['kind']=='item_delta':
                        data.update(receipt_ref=receipt,operation_ordinal=ordinals[target_id])
                        data['after']['revision']=initial+tick+1
                    rows.append(copy.deepcopy(row))
                ordinals[target_id]+=int(not no_change);cursor+=1
                if not no_change:changed.add(target_id);levels[target_id]+=1;target_revisions[target_id]=initial+tick+1
            for identity,parts in contributions.items():
                resolution,_,before,_=parts[0]
                rows.append(dict(kind='item_delta',entity=OWNER,incarnation=incarnation,tick=start+tick,sequence=0,
                    data=dict(operation='consume',instance=identity,before=copy.deepcopy(before),
                              after=before|{'count':counts[identity],'revision':initial+tick+1} if counts[identity] else None,
                              count_delta=sum(delta for _,_,_,delta in parts),operation_ordinal=0,resolution_id=resolution,
                              receipt_refs=[ref for _,ref,_,_ in parts if ref.startswith('receipt:')],
                              causes=[dict(cause=ref,count_delta=delta) for _,ref,_,delta in parts])))
            shot=item(50,(ticks-tick)*3+1,1835)|{'revision':initial+tick}
            rows.append(dict(kind='item_delta',entity=OWNER,incarnation=incarnation,tick=start+tick,sequence=0,
                data=dict(operation='consume',instance=50,before=shot,after=shot|{'count':shot['count']-3,'revision':initial+tick+1},
                          count_delta=-3,operation_ordinal=0,resolution_id=f'attack:{tick}:0',receipt_refs=[],
                          causes=[dict(cause=f'auto_shot:attack:{tick}:{index}',count_delta=-delta) for index,delta in enumerate((1,2))])))
        rows[0].update(snapshot_tick=start)
        rows[0]['watermark'].update(records=ticks,last_tick=start+ticks-1)
        rows.append(dict(kind='complete',facts=0,ticks=ticks,last_tick=start+ticks-1))
        # D67 allocations are recording-wide, including the second owner's stacks.
        other=copy.deepcopy(rows[1:-1])
        for row in other:
            row['entity']=OTHER
            data=row['data']
            if row['kind']=='item_baseline':
                data['inventory_revision']+=100
                for state in data['instances']:state.update(id=state['id']+1000,revision=state['revision']+100)
                data['paperdoll']={slot:identity+1000 for slot,identity in data['paperdoll'].items()}
            elif row['kind']=='item_delta':
                data['instance']+=1000
                for key in ('before','after'):
                    if data[key] is not None:data[key].update(id=data[key]['id']+1000,revision=data[key]['revision']+100)
            elif row['kind']=='item_formula' and data['formula']=='enchant_outcome':
                for state in data['operation'].values():
                    if isinstance(state,dict):state.update(id=state['id']+1000,revision=state['revision']+100)
        rows[1:-1]=sorted(rows[1:-1]+other,key=lambda row:row['tick'])
        for index,row in enumerate(r for r in rows if r['kind']=='item_rng'):
            row['data'].update(word_before=index,word_after=index+1)
        return self.repair(rows)

    def test_review9_owner_tick_revision_and_target_rollback(self):
        for support in (False,True):
            rows=self.enchant_rows(support=support)
            scroll=rows[1]['data']['operation']['scroll_before'];scroll['count']=2
            next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['instance']==7)['after']=scroll|{'count':1,'revision':1}
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
            bad=copy.deepcopy(rows)
            aggregate=next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')
            aggregate['after']['revision']=99
            with self.assertRaisesRegex(ValueError,'within tick'):gate.validate(bad,self.recording,HASH)
        bad=self.shared_stack_enchants(3,True)
        outcomes=[r['data'] for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome']
        outcomes[1]['operation']['support_before']['revision']=2
        next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['instance']==9)['after']['revision']=2
        with self.assertRaisesRegex(ValueError,'within tick'):gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)
        bad=self.shared_stack_enchants(3,True)
        outcomes=[r['data'] for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome']
        outcomes[1]['operation']['target_before']['revision']=0
        with self.assertRaises(ValueError):gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)

    def test_review9_locally_conserved_count_discontinuity(self):
        rows=self.generated_enchants(random.Random(0xE609))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        aggregate=next(r for r in rows if r.get('entity')==OWNER and r['kind']=='item_delta'
                       and r['data']['operation']=='consume' and r['tick']>rows[0]['snapshot_tick'])
        for key in ('before','after'):
            if aggregate['data'][key] is not None:aggregate['data'][key]['count']+=10
        if aggregate['data']['after'] is None:
            aggregate['data']['after']=aggregate['data']['before']|{'count':10,'revision':aggregate['data']['before']['revision']+1}
        for row in rows:
            if row.get('entity')==OWNER and row['tick']==aggregate['tick'] and row['kind']=='item_formula' and row['data']['formula']=='enchant_outcome':
                for key in ('scroll_before','support_before'):
                    state=row['data']['operation'][key]
                    if state is not None and state['id']==aggregate['data']['instance']:state['count']+=10
        with self.assertRaises(ValueError):gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)

    def test_history_creation_removal_intervening_mutations_and_incarnation(self):
        rows=copy.deepcopy(self.rows)
        first=item(70,2,951)|{'revision':1}
        facts=[dict(operation='grant',before=None,after=first,count_delta=2),
               dict(operation='enchant',before=first,after=first|{'enchant':1},count_delta=0),
               dict(operation='destroy',before=first|{'enchant':1},after=None,count_delta=-2),
               dict(operation='grant',before=None,after=first|{'id':71},count_delta=2)]
        for ordinal,data in enumerate(facts):
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
                data=data|dict(instance=71 if ordinal==3 else 70,operation_ordinal=0 if ordinal==3 else ordinal,resolution_id=f'change:{ordinal}',receipt_ref=None)))
        self.repair(rows);gate.validate(rows,self.recording,HASH)
        for key in ('count','enchant','revision','template_id'):
            bad=copy.deepcopy(rows);bad[3]['data']['before']=bad[3]['data']['before']|{key:4 if key=='count' else 2 if key!='template_id' else 952}
            if key=='count':bad[3]['data']['count_delta']=-1
            with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        # Incarnations have separate state/revision histories; allocations stay unique.
        # D79: inc2's first touch of 70 sees its inc1 last-change revision 1; inc2 writes step 2.
        for row in rows[3:-1]:
            row['incarnation']=2;data=row['data']
            if data['instance']==70:data['operation_ordinal']-=1 # D83: positions restart in the inc2 chain.
            if data['after'] is not None:data['after']['revision']=2
            if data['operation']=='destroy':data['before']['revision']=2
        gate.validate(rows,self.recording,HASH)

    def test_seeded_aggregate_correlation_property_check_2000_cases(self):
        rng=random.Random(0xE609)
        mutations=('receipt reuse','swapped ordinals','cross-tick revision','wrong totals',
                   'missing contributions','duplicated aggregates','foreign-owner rows',
                   'per-instance revision drift','support/scroll endpoint mismatch','cross-owner contamination',
                   'canonical count discontinuity','unchanged target revision','unchanged target missing','unchanged target id',
                   'missing causes','orphan receipts','cause sum mismatch')
        def passes(rows):
            try:return all(gate.evaluate(rows,self.recording,entity,['enchant_outcome','enchant_chance'],HASH)['passed'] for entity in (OWNER,OTHER))
            except ValueError:return False
        for case in range(2000):
            rows=self.generated_enchants(rng)
            self.assertTrue(passes(rows),f'valid case {case}')
            for mutation in mutations:
                bad=copy.deepcopy(rows)
                outcomes=[r for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome']
                targets=[r for r in bad if r.get('entity')==OWNER and r['kind']=='item_delta' and r['data']['operation']=='enchant']
                aggregates=[r for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume']
                if mutation=='receipt reuse':
                    old=outcomes[1]['data']['operation']['receipt_ref'];new=outcomes[0]['data']['operation']['receipt_ref']
                    for row in bad[1:-1]:
                        data=row['data']
                        if row['kind']=='item_formula' and data['formula']=='enchant_outcome' and data['operation']['receipt_ref']==old:data['operation']['receipt_ref']=new
                        elif row['kind']=='item_delta':
                            if data['operation']=='consume':
                                data['receipt_refs']=[new if ref==old else ref for ref in data['receipt_refs']]
                                for cause in data['causes']:
                                    if cause['cause']==old:cause['cause']=new
                            elif data['receipt_ref']==old:data['receipt_ref']=new
                elif mutation=='swapped ordinals':
                    a,b=next((a,b) for index,a in enumerate(targets) for b in targets[index+1:]
                             if a['tick']==b['tick'] and a['data']['instance']==b['data']['instance'])
                    a['data']['operation_ordinal'],b['data']['operation_ordinal']=b['data']['operation_ordinal'],a['data']['operation_ordinal']
                elif mutation=='cross-tick revision':
                    first=targets[0];later=next(r for r in targets if r['tick']>first['tick'])
                    later['data']['after']['revision']=first['data']['after']['revision']
                elif mutation=='wrong totals':rng.choice(aggregates)['data']['count_delta']-=1
                elif mutation=='missing contributions':
                    aggregate=next(r for r in aggregates if r['data']['receipt_refs'])
                    aggregate['data']['receipt_refs']=aggregate['data']['receipt_refs'][:-1]
                elif mutation=='duplicated aggregates':
                    aggregate=rng.choice(aggregates);bad.insert(bad.index(aggregate)+1,copy.deepcopy(aggregate));self.repair(bad)
                elif mutation=='foreign-owner rows':rng.choice([r for r in aggregates if r['data']['receipt_refs']])['entity']='01970000-0000-7000-8000-000000000042'
                elif mutation=='per-instance revision drift':rng.choice(targets)['data']['after']['revision']+=10
                elif mutation=='support/scroll endpoint mismatch':
                    aggregate=next(r for r in aggregates if r['data']['after'] is not None)
                    aggregate['data']['after']['revision']+=10
                    for outcome in outcomes:
                        if (outcome['entity'],outcome['incarnation'],outcome['tick'])!=(aggregate['entity'],aggregate['incarnation'],aggregate['tick']):continue
                        for key in ('scroll_before','support_before'):
                            state=outcome['data']['operation'][key]
                            if state is not None and state['id']==aggregate['data']['instance'] and state['revision']!=aggregate['data']['before']['revision']:state['revision']+=10
                elif mutation=='cross-owner contamination':
                    aggregate=rng.choice(aggregates)
                    foreign=next(r for r in aggregates if r['entity']!=aggregate['entity'] and r['tick']==aggregate['tick']
                                 and r['data']['instance']==aggregate['data']['instance']+(1000 if aggregate['entity']==OWNER else -1000))
                    for key in ('before','after'):
                        state=copy.deepcopy(foreign['data'][key])
                        aggregate['data'][key]=state|{'id':aggregate['data']['instance']} if state else None
                elif mutation=='canonical count discontinuity':
                    aggregate=next(r for r in aggregates if r['tick']>rows[0]['snapshot_tick'])
                    for key in ('before','after'):
                        if aggregate['data'][key] is not None:aggregate['data'][key]['count']+=10
                    if aggregate['data']['after'] is None:
                        aggregate['data']['after']=aggregate['data']['before']|{'count':10,'revision':aggregate['data']['before']['revision']+1}
                    for outcome in outcomes:
                        if (outcome['entity'],outcome['incarnation'],outcome['tick'])!=(aggregate['entity'],aggregate['incarnation'],aggregate['tick']):continue
                        for key in ('scroll_before','support_before'):
                            state=outcome['data']['operation'][key]
                            if state is not None and state['id']==aggregate['data']['instance']:state['count']+=10
                elif mutation.startswith('unchanged target'):
                    outcome=next(r for r in outcomes if r['entity']==OWNER and r['data']['actual']['enchant']==r['data']['inputs']['enchant'])
                    target=outcome['data']['operation']['target_before']
                    if mutation=='unchanged target revision':target['revision']+=99
                    elif mutation=='unchanged target id':target['id']=999
                    else:
                        baseline=next(r['data'] for r in bad if r['kind']=='item_baseline' and r['entity']==OWNER)
                        baseline['instances']=[s for s in baseline['instances'] if s['id']!=target['id']]
                elif mutation in ('missing causes','orphan receipts','cause sum mismatch'):
                    aggregate=rng.choice([r for r in aggregates if not r['data']['receipt_refs']])['data']
                    if mutation=='missing causes':del aggregate['causes']
                    elif mutation=='orphan receipts':aggregate['receipt_refs']=['receipt:use:orphan']
                    else:aggregate['causes'][0]['count_delta']-=1
                self.assertFalse(passes(bad),f'{mutation}, case {case}')
        print('Aggregate property check: seed=0xE609; 2000 valid two-owner sets with no-change/system/mixed outcomes pass; 34000 corruptions reject',flush=True)

    def test_revisions_increase_across_ticks_with_ordinal_reset(self):
        rows=self.same_tick_enchants()
        third=self.enchant_rows(enchant=2,scroll_id=22)
        third[1]['data']['operation']['target_before']['revision']=1
        for row in third[1:-1]:
            row['tick']=1;row['data']['resolution_id']='enchant:3'
            if row['kind']=='item_rng':row['data'].update(word_before=2,word_after=3)
            if row['kind']=='item_delta' and row['data']['instance']==8:row['data']['after']['revision']=2
            if row['kind']=='item_delta' and row['data']['operation']=='consume':row['data']['before']['revision']=1
            rows.insert(-1,row)
        rows[0]['watermark'].update(records=2,last_tick=1)
        rows[-1].update(ticks=2,last_tick=1);self.repair(rows)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        for revision in (0,1):
            bad=copy.deepcopy(rows)
            target=next(r['data'] for r in reversed(bad) if r['kind']=='item_delta' and r['data']['instance']==8)
            target['before']['revision']=0;target['after']['revision']=revision
            with self.assertRaisesRegex(ValueError,'across ticks'):gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)

    def test_equal_revision_scroll_and_existing_refund_stack(self):
        rows=self.enchant_rows()
        scroll=rows[1]['data']['operation']['scroll_before'];scroll.update(count=2)
        consume=next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['operation']=='consume')
        consume['after']=scroll|{'count':1,'revision':1}
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        # D79: a first touch carries an EARLIER last-change revision; the tick's own step refuses.
        bad=copy.deepcopy(rows);bad[1]['data']['operation']['scroll_before']['revision']=1
        with self.assertRaisesRegex(ValueError,'earlier last-change revision'):gate.validate(bad,self.recording,HASH)
        next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')['after']['revision']=0
        with self.assertRaisesRegex(ValueError,'revision regressed'):gate.validate(bad,self.recording,HASH)
        rows=self.enchant_rows(base=70,residue=99999,enchant=3)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','failure_refund'],HASH)['passed'])
        refund=next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['operation']=='grant')
        refund['before']=refund['after']|{'count':5,'revision':0}
        refund['after']=refund['after']|{'count':5+refund['count_delta'],'revision':1}
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','failure_refund'],HASH)['passed'])
        bad=copy.deepcopy(rows)
        next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='grant')['after']['enchant']+=1
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)

    def test_every_later_same_tick_post_revision_must_advance_and_share_step(self):
        rows=self.same_tick_enchants()
        for index in (2,3):
            attempt=self.enchant_rows(base=100 if index==2 else 70,enchant=index,scroll_id=20+index)
            attempt[1]['data']['operation']['target_before']['revision']=index-1
            attempt[1]['data']['operation']['receipt_ref']=f'receipt:enchant:{index+1}'
            for row in attempt[1:-1]:
                row['tick']=1;row['data']['resolution_id']=f'enchant:{index+1}'
                if row['kind']=='item_rng':row['data'].update(word_before=index,word_after=index+1)
                if row['kind']=='item_delta':
                    row['data']['receipt_refs' if row['data']['operation']=='consume' else 'receipt_ref']=[f'receipt:enchant:{index+1}'] if row['data']['operation']=='consume' else f'receipt:enchant:{index+1}'
                    if row['data']['operation']=='consume':
                        row['data']['before']['revision']=1
                        row['data']['causes']=[dict(cause=f'receipt:enchant:{index+1}',count_delta=-1)]
                if row['kind']=='item_delta' and row['data']['instance']==8:
                    row['data']['after']['revision']=2;row['data']['operation_ordinal']=index-2
                rows.insert(-1,row)
        rows[0]['watermark'].update(records=2,last_tick=1)
        rows[-1].update(ticks=2,last_tick=1);self.repair(rows)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'])
        for revision in (0,1,3):
            bad=copy.deepcopy(rows)
            outcome=[r['data'] for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome'][-1]
            target=[r['data'] for r in bad if r['kind']=='item_delta' and r['data']['instance']==8][-1]
            outcome['operation']['target_before']['revision']=min(revision,1)
            target['after']['revision']=revision
            with self.assertRaisesRegex(ValueError,'across ticks' if revision<=1 else 'within tick'):
                gate.evaluate(bad,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)

    def test_offline_zero_incarnation_requires_explicit_origin_and_owner(self):
        for operation in ('grant','starter','backfill'):
            rows=copy.deepcopy(self.rows)
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=0,origin='offline',tick=0,sequence=1,
                data=dict(resolution_id='offline:1',operation=operation,instance=2,before=None,after=item(2,1,1),
                          count_delta=1,operation_ordinal=0,receipt_ref=None)))
            self.repair(rows)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
            self.assertFalse(gate.evaluate(rows,self.recording,OTHER,['pole_damage'],HASH)['passed'])
            for origin in (None,'live','invented',0,False,[],{}):
                bad=copy.deepcopy(rows)
                if origin is None:del bad[2]['origin']
                else:bad[2]['origin']=origin
                with self.assertRaises(ValueError,msg=(operation,origin)):gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH)
        rows=copy.deepcopy(self.rows);rows[1].update(incarnation=0,origin='offline')
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        del rows[1]['origin']
        with self.assertRaises(ValueError):gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)

    def test_consume_ordered_receipts_and_exchange_single_receipt_schema(self):
        rows=copy.deepcopy(self.rows)
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=1,
            data=dict(resolution_id=rows[1]['data']['resolution_id'],operation='consume',instance=7,
                      before=item(7,3),after=item(7,1)|{'revision':1},count_delta=-2,operation_ordinal=0,receipt_refs=['receipt:enchant:2','receipt:enchant:1'],
                      causes=[dict(cause=ref,count_delta=-1) for ref in ('receipt:enchant:2','receipt:enchant:1')])))
        self.repair(rows)
        phase=dict(name='consume-aggregate',formula='pole_damage',mutation=dict(operation='consume',template=17,slot=None))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        self.assertEqual(rows[2]['data']['receipt_refs'],['receipt:enchant:2','receipt:enchant:1'])
        bad=copy.deepcopy(rows);del bad[2];self.repair(bad)
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        for value in ([],None,'receipt:enchant:1',{},[None],[''],[1],[False],[['receipt:enchant:1']]):
            bad=copy.deepcopy(rows);bad[2]['data']['receipt_refs']=value
            with self.assertRaises(ValueError,msg=value):gate.validate(bad,self.recording,HASH)
        for name in ('receipt','receipt_ref',None):
            bad=copy.deepcopy(rows);del bad[2]['data']['receipt_refs']
            if name:bad[2]['data'][name]='receipt:enchant:1'
            with self.assertRaises(ValueError,msg=name):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad.insert(-1,copy.deepcopy(bad[2]));bad[3]['data']['resolution_id']='other';self.repair(bad)
        with self.assertRaisesRegex(ValueError,'ordinal'):gate.validate(bad,self.recording,HASH)
        # Exchange is a receipt over consume/grant constituents, never a mutable row.
        rows=self.exchange_rows()
        gate.validate(rows,self.recording,HASH)
        for value in (None,[],['exchange:1'],'',False):
            bad=copy.deepcopy(rows);bad[-2]['data']['receipt_ref']=value
            with self.assertRaises(ValueError,msg=value):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[-2]['data']['receipt_refs']=['exchange:1']
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)

    def test_system_only_and_mixed_consume_causes(self):
        rows=copy.deepcopy(self.rows)
        rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(operation='consume',instance=50,before=item(50,5,1835),after=item(50,2,1835)|{'revision':1},
                      count_delta=-3,operation_ordinal=0,resolution_id='attack:1',receipt_refs=[],
                      causes=[dict(cause='auto_shot:attack:1',count_delta=-1),dict(cause='auto_shot:attack:2',count_delta=-2)])))
        self.repair(rows)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        for causes,refs in (
            ([dict(cause='arrow:attack:1',count_delta=-3)],[]),
            ([dict(cause='item_cast:cast:1',count_delta=-3)],[]),
            ([dict(cause='auto_shot:attack:1',count_delta=-1),dict(cause='receipt:use:key',count_delta=-1),
              dict(cause='item_cast:cast:1',count_delta=-1)],['receipt:use:key'])):
            mixed=copy.deepcopy(rows);mixed[2]['data'].update(causes=causes,receipt_refs=refs)
            mixed[2]['data']['resolution_id']=causes[0]['cause'].split(':',1)[1]
            self.assertTrue(gate.evaluate(mixed,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        for causes in (None,[],{},['auto_shot:attack:1'],[{}],
                       [dict(cause='unknown:1',count_delta=-3)],
                       [dict(cause='auto_shot:',count_delta=-3)],
                       [dict(cause='receipt:key',count_delta=-3)],
                       [dict(cause='auto_shot:attack:1',count_delta=-2)]):
            bad=copy.deepcopy(rows);bad[2]['data']['causes']=causes
            self.assert_rejected(bad,['pole_damage'])
        for value in (True,0,1,-2**63):
            bad=copy.deepcopy(rows);bad[2]['data']['causes']=[dict(cause='auto_shot:attack:1',count_delta=value)]
            self.assert_rejected(bad,['pole_damage'])
        bad=copy.deepcopy(rows);del bad[2]['data']['causes'];self.assert_rejected(bad,['pole_damage'])
        bad=copy.deepcopy(rows);bad[2]['data']['receipt_refs']=['receipt:use:key'];self.assert_rejected(bad,['pole_damage'])
        bad=copy.deepcopy(rows);bad[2]['data']['causes']=[dict(cause='receipt:use:key',count_delta=-3)]
        self.assert_rejected(bad,['pole_damage'])

    def test_enchant_consume_causes_allow_interleaved_system_costs(self):
        for first_system in (False,True):
            rows=self.shared_stack_enchants(count=6)
            outcomes=[r['data'] for r in rows if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome']
            aggregate=next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['operation']=='consume')
            causes=[dict(cause='receipt:enchant:1',count_delta=-1),dict(cause='item_cast:cast:middle',count_delta=-2),
                    dict(cause='receipt:enchant:2',count_delta=-1),dict(cause='item_cast:cast:last',count_delta=-2)]
            if first_system:
                causes.insert(0,dict(cause='item_cast:cast:first',count_delta=-1));causes[-1]['count_delta']=-1
                aggregate['resolution_id']='cast:first'
                outcomes[0]['operation']['scroll_before']=aggregate['before']|{'count':5,'revision':1}
            outcomes[1]['operation']['scroll_before'].update(count=2 if first_system else 3,revision=1)
            aggregate.update(causes=causes,count_delta=-6,after=None)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'])
            bad=copy.deepcopy(rows)
            [r['data'] for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome'][1]['operation']['scroll_before']['count']+=1
            self.assert_rejected(bad)
            bad=copy.deepcopy(rows)
            data=next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')
            data['causes'].reverse();data['receipt_refs'].reverse()
            self.assert_rejected(bad)

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

    def test_unchanged_enchant_target_matches_complete_baseline(self):
        rows=self.enchant_rows(base=70,residue=99999,enchant=3,safe=True)
        op=rows[1]['data']['operation']
        rows.insert(1,dict(kind='item_baseline',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(inventory_revision=0,instances=copy.deepcopy([op['target_before'],op['scroll_before']]),paperdoll={},auto_shots=[])))
        self.repair(rows)
        gates=['enchant_outcome','enchant_chance','failure_refund']
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,gates,HASH)['passed'])
        self.assertFalse(any(r['kind']=='item_delta' and r['data']['instance']==8 for r in rows))
        for mutation in ('revision','membership','identity','canonical'):
            bad=copy.deepcopy(rows);target=bad[2]['data']['operation']['target_before']
            if mutation=='revision':target['revision']=99
            elif mutation=='identity':target['id']=999
            elif mutation=='canonical':target['created_tick']=1
            else:bad[1]['data']['instances']=[s for s in bad[1]['data']['instances'] if s['id']!=8]
            self.assert_rejected(bad,gates)

    def test_unchanged_enchant_target_follows_intervening_mutations(self):
        rows=self.enchant_rows(base=70,residue=99999,enchant=3,safe=True)
        target=rows[1]['data']['operation']['target_before']
        rows.insert(1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
            data=dict(operation='enchant',instance=8,before=target|{'enchant':2},after=target|{'revision':1},
                      count_delta=0,operation_ordinal=0,resolution_id='earlier:1',receipt_ref=None)))
        target['revision']=1;self.repair(rows)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        for field,value in (('revision',0),('created_tick',1)):
            bad=copy.deepcopy(rows);bad[2]['data']['operation']['target_before'][field]=value
            self.assert_rejected(bad)
        bad=copy.deepcopy(rows);bad[1]['data'].update(operation='destroy',after=None,count_delta=-1)
        self.assert_rejected(bad)

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
                    lhand=None,mp=10,mp_modifier=1,reduction_residue=None,miss=False,
                    hit_chance=800,hit_residue=799,critical_residue=17,spread_residue=3)
        actual=gate.residue_family('bow_launch',inputs)
        self.assertEqual((actual['arrows_consumed'],actual['inventory'][0]['count'],actual['mp']),(1,1,9))
        rows=copy.deepcopy(self.rows)
        rows[1]['data'].update(formula='bow_launch',inputs=inputs,actual=gate.oracle.rational(actual))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['bow_launch'],HASH)['passed'])
        for count in (-1,2**63):
            bad=copy.deepcopy(rows);bad[1]['data']['inputs']['inventory'][0]['count']=count
            bad[1]['data']['inputs']=self.with_residues('bow_launch',{k:v for k,v in bad[1]['data']['inputs'].items() if k not in gate.RESIDUE_INPUTS['bow_launch']})
            bad[1]['data']['actual']=gate.oracle.rational(gate.residue_family('bow_launch',bad[1]['data']['inputs']))
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['bow_launch'],HASH)['passed'])
            with self.assertRaises(ValueError):gate.input_domains('learned_item_cost',dict(inventory={'17':count}))

    def test_d87_bow_and_dual_residues_are_fact_inputs(self):
        bow=dict(weapon=gate.oracle.behavior.bow_definition(gate.source(),13),
                 inventory=[gate.oracle.behavior.arrow_stack(gate.source(),17,2)],lhand=None,mp=10,mp_modifier=1,
                 reduction_residue=None,miss=False,hit_chance=800,hit_residue=100,critical_residue=17,spread_residue=3)
        dual=dict(hits=[True,False],formulas=[10,'3/2'],second_valid=True,hit_residues=[100,900],hit_chance=800,
                  spread_radius=6,critical_residues=[4,None],spread_residues=[0,None])
        for name,inputs,sites in (('bow_launch',bow,{'critical':17,'spread':3}),('dual_plan',dual,{'critical1':4,'spread1':0})):
            with self.subTest(formula=name):
                actual=gate.oracle.rational(gate.residue_family(name,inputs))
                self.assertEqual({d['site']:d['residue'] for d in actual['draws'] if d['site'] in sites},sites)
                rows=copy.deepcopy(self.rows);rows[1]['data'].update(formula=name,inputs=inputs,actual=actual)
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,[name],HASH)['passed'])
                # The E1 neutral-caller output (critical 999, spread = radius) no longer matches live residues.
                neutral=gate.oracle.rational(getattr(gate.oracle.behavior,name)(**{k:v for k,v in gate.oracle.decode_rationals(inputs).items() if k not in gate.RESIDUE_INPUTS[name]}))
                with self.assertRaises(ValueError):gate.formula(name,inputs,neutral)
        mutations=[('bow_launch',dict(critical_residue=None)),('bow_launch',dict(spread_residue=None)),
                   ('bow_launch',dict(spread_residue=11)),('bow_launch',dict(critical_residue=1000)),
                   ('bow_launch',dict(miss=True,hit_residue=900)), # miss: residues present for untaken draws
                   ('dual_plan',dict(critical_residues=[None,None])),('dual_plan',dict(spread_residues=[13,None])),
                   ('dual_plan',dict(critical_residues=[4,999])),('dual_plan',dict(spread_residues=[0,6]))]
        for name,change in mutations:
            with self.subTest(formula=name,change=change):
                inputs=(bow if name=='bow_launch' else dual)|change
                with self.assertRaises(ValueError):gate.formula(name,inputs,None)
        for name,inputs in (('bow_launch',bow),('dual_plan',dual)):
            for key in gate.RESIDUE_INPUTS[name]:
                with self.assertRaises(ValueError):gate.input_domains(name,{k:v for k,v in inputs.items() if k!=key})
        print(f'D87: live critical/spread residues verified for bow_launch/dual_plan; {len(mutations)} residue mutations and missing fields reject',flush=True)

    def test_d88_hit_iff_residue_below_chance(self):
        self.assertTrue(gate.oracle_hit_inclusive()) # E1 rev 8 still uses <=; the consumer adapts.
        bow=dict(weapon=gate.oracle.behavior.bow_definition(gate.source(),13),
                 inventory=[gate.oracle.behavior.arrow_stack(gate.source(),17,2)],lhand=None,mp=10,mp_modifier=1,
                 reduction_residue=None,miss=False,hit_chance=800,hit_residue=799,critical_residue=17,spread_residue=3)
        miss=bow|dict(miss=True,critical_residue=None,spread_residue=None)
        for inputs,hit in ((bow,True),(miss|dict(hit_residue=800),False),(miss|dict(hit_residue=999),False),
                           (bow|dict(hit_chance=1,hit_residue=0),True),(miss|dict(hit_chance=0,hit_residue=0),False)):
            with self.subTest(inputs=(inputs['hit_chance'],inputs['hit_residue'],hit)):
                actual=gate.oracle.rational(gate.residue_family('bow_launch',inputs))
                self.assertEqual(actual['hit'],hit);gate.formula('bow_launch',inputs,actual)
        # The boundary residue == chance is a miss: claiming a hit there rejects, as does a miss below it.
        for inputs in (bow|dict(hit_residue=800),miss|dict(hit_residue=799)):
            with self.assertRaises(ValueError):gate.formula('bow_launch',inputs,None)
        dual=dict(hits=[True,False],formulas=[10,10],second_valid=True,hit_residues=[799,800],hit_chance=800,
                  spread_radius=0,critical_residues=[5,None],spread_residues=[0,None])
        gate.formula('dual_plan',dual,gate.oracle.rational(gate.residue_family('dual_plan',dual)))
        with self.assertRaises(ValueError):gate.formula('dual_plan',dual|dict(hits=[True,True],critical_residues=[5,5],spread_residues=[0,0]),None)
        print('D88: hit iff residue < hit_chance at the boundary for bow_launch and dual_plan',flush=True)

    def test_d89_draw_derived_fields_null_iff_draw_not_taken(self):
        magic=dict(power=10,m_atk=100,m_def=50,is_pvp=False)
        controls=[('magic_damage',magic),('magic_damage',magic|dict(magic_failures=True,first_success=True)),
                  ('magic_damage',magic|dict(magic_failures=True,first_success=False,second_success=True)),
                  ('magic_damage',magic|dict(magic_failures=True,first_success=False,attacker_player=False)),
                  ('magic_damage',magic|dict(perfect=True,magic_failures=True)),
                  ('shield',dict(rate=50,eligible=True,perfect_residue=95,normal_residue=None)),
                  ('shield',dict(rate=50,eligible=True,perfect_residue=10,normal_residue=20)),
                  ('shield',dict(rate=50,eligible=False)),
                  ('shot_payment',dict(charges=[],kind='soul',count=2,available=5,weapon_grade='D',shot_grade='D',reduction=[10,1],residue=50)),
                  ('shot_payment',dict(charges=[],kind='soul',count=2,available=5,weapon_grade='D',shot_grade='D'))]
        for name,inputs in controls:
            with self.subTest(control=(name,inputs)):
                rows=self.formula_rows(name,inputs);result=gate.evaluate(rows,self.recording,OWNER,[name],HASH)
                self.assertTrue(result['passed'],result)
        mutations=[('magic_damage',magic|dict(first_success=True)),          # no failure draw without magic_failures
                   ('magic_damage',magic|dict(second_success=False)),
                   ('magic_damage',magic|dict(magic_failures=True)),          # first draw taken but null
                   ('magic_damage',magic|dict(magic_failures=True,first_success=True,second_success=False)),
                   ('magic_damage',magic|dict(magic_failures=True,first_success=False,attacker_player=False,second_success=True)),
                   ('magic_damage',magic|dict(perfect=True,magic_failures=True,first_success=True)),
                   ('shield',dict(rate=50,eligible=True,perfect_residue=95,normal_residue=50)),
                   ('shield',dict(rate=50,eligible=False,perfect_residue=0)),
                   ('shield',dict(rate=50,eligible=True,perfect_residue=None)),
                   ('shot_payment',dict(charges=[],kind='soul',count=2,available=5,weapon_grade='D',shot_grade='D',residue=0))]
        for name,inputs in mutations:
            with self.subTest(mutation=(name,inputs)):
                rows=self.formula_rows(name,inputs)
                if rows[1]['data']['actual'] is None:rows[1]['data']['actual']={}
                self.assertFalse(gate.evaluate(rows,self.recording,OWNER,[name],HASH)['passed'])
                with self.assertRaises(ValueError):gate.formula(name,inputs,rows[1]['data']['actual'])
        with self.assertRaises(ValueError):gate.input_domains('magic_damage',magic|dict(is_pvp=None))
        # Review-2 P3-4: is_pvp is required, not merely non-null.
        with self.assertRaises(ValueError):gate.input_domains('magic_damage',{k:v for k,v in magic.items() if k!='is_pvp'})
        print(f'D89: {len(controls)} draw-derived controls pass; {len(mutations)} null/present mismatches and a null is_pvp reject',flush=True)

    def test_d90_gear_stats_full_dictionary_with_ordered_trace(self):
        vectors=json.loads((gate.REPO/'packages/data/fixtures/item-oracle.json').read_text())['vectors']
        controls=0
        for vector in vectors:
            if not {'class_file','level','equipment'}<=set(vector['inputs'])<=set(gate.INPUT_SCHEMAS['gear_stats']):continue
            inputs=dict(skill_refs=[],base_deltas=None,base_delta_sequence=-1)|copy.deepcopy(vector['inputs'])
            actual=gate.oracle.rational(gate.oracle.gear_stats(gate.source(),**gate.oracle.decode_rationals(inputs)))
            try:gate.formula('gear_stats',inputs,actual)
            except ValueError:continue
            controls+=1
            if len(actual['trace'])<2:continue
            mutations={'reversed trace':lambda a:a['trace'].reverse(),'dropped trace row':lambda a:a['trace'].pop(),
                       'missing trace':lambda a:a.pop('trace'),'missing traits':lambda a:a.pop('traits'),
                       'missing root_expressions':lambda a:a.pop('root_expressions'),'extra key':lambda a:a.update(note=1)}
            for label,mutate in mutations.items():
                with self.subTest(vector=vector['id'],mutation=label):
                    bad=copy.deepcopy(actual);mutate(bad)
                    with self.assertRaises(ValueError):gate.formula('gear_stats',inputs,bad)
        self.assertGreaterEqual(controls,1)
        print(f'D90: {controls} full gear_stats dictionaries pass; reordered/dropped trace rows and missing keys reject',flush=True)

    def test_d91_clamp_no_heal_resource_suffix(self):
        rows=copy.deepcopy(self.rows);base=rows[1]
        for index,(resource,current) in enumerate((('hp',120),('mp',40),('cp',0))):
            row=copy.deepcopy(base);row['data']=dict(formula='clamp_no_heal',inputs=dict(current=current,new_max=100),
                actual=min(current,100),resolution_id=f'load:rebuild:7:{resource}',baseline=False)
            rows.insert(-1,row)
        self.repair(rows)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['clamp_no_heal'],HASH)['passed'])
        for resolution in ('load:rebuild:7',':hp','load:rebuild:7:HP','load:rebuild:7:sp','load:rebuild:7:hp:x','hp','x::hp'):
            with self.subTest(resolution=resolution):
                bad=copy.deepcopy(rows);bad[2]['data']['resolution_id']=resolution
                with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[3]['data']['resolution_id']=bad[2]['data']['resolution_id']
        with self.assertRaisesRegex(ValueError,'repeated formula resolution'):gate.validate(bad,self.recording,HASH)
        print('D91: hp/mp/cp clamps of one rebuild pass with distinct suffixes; missing/unknown suffixes and reuse reject',flush=True)

    def test_d92_multi_tick_families_one_terminal_fact(self):
        vectors=json.loads((gate.REPO/'packages/data/fixtures/item-oracle.json').read_text())['vectors']
        vector=next(v for v in vectors if v['id'].startswith('consumable-escape-timing-'))
        rows=self.formula_rows('escape_timing',copy.deepcopy(vector['inputs']))
        rows[1].update(tick=2);rows[1]['data']['resolution_id']='receipt:use:k1'
        # The intent's start-tick draw (prepay/cast start) precedes the terminal teleport fact.
        rows.insert(1,dict(kind='item_rng',entity=OWNER,incarnation=1,tick=1,sequence=0,data=dict(resolution_id='receipt:use:k1',
            callsite='cast_start',word_before=0,word_after=1,bound=100,value=3)))
        rows[0]['watermark'].update(records=3,last_tick=2);rows[-1].update(ticks=3,last_tick=2);self.repair(rows)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['escape_timing'],HASH)['passed'])
        bad=copy.deepcopy(rows);bad[1].update(tick=2);bad[2].update(tick=1);bad[1],bad[2]=bad[2],bad[1];self.repair(bad)
        with self.assertRaisesRegex(ValueError,'D92'):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);extra=copy.deepcopy(bad[2]);extra['data']['formula']='consumable_use';bad.insert(-1,extra);self.repair(bad)
        with self.assertRaisesRegex(ValueError,'more than one terminal fact'):gate.validate(bad,self.recording,HASH)
        # Another intent's later facts are unaffected.
        ok=copy.deepcopy(rows);later=copy.deepcopy(ok[1]);later.update(tick=2);later['data'].update(resolution_id='receipt:use:k2',word_before=1,word_after=2)
        ok.insert(-1,later);self.repair(ok);gate.validate(ok,self.recording,HASH)
        # Review-2 P2-2: a prepay Consume correlates by its receipt cause even under another resolution_id.
        def with_consume(tick):
            out=self.transition_rows('consume');delta=out[3]
            delta.update(tick=tick);delta['data'].update(resolution_id='other:9',receipt_refs=['receipt:use:k1'],
                causes=[dict(cause='receipt:use:k1',count_delta=-1)])
            fact=copy.deepcopy(rows[2]);fact.update(tick=1);out.insert(-1,fact)
            out[0]['watermark'].update(records=3,last_tick=2);out[-1].update(ticks=3,last_tick=2)
            out[1:-1]=sorted(out[1:-1],key=lambda r:r['tick']);return self.repair(out)
        gate.validate(with_consume(0),self.recording,HASH)
        with self.assertRaisesRegex(ValueError,'D92'):gate.validate(with_consume(2),self.recording,HASH)
        print('D92: one terminal fact per intent on or after every correlated fact; earlier/duplicate terminal facts and a later prepay Consume (by receipt) reject',flush=True)

    def test_rev9_import_path_and_probe_verdicts(self):
        """Review-2 P3-1 / D96: adapters only on pre-D87 data; rev 9 oracles pass residues through."""
        from unittest.mock import patch
        dual=dict(hits=[True,False],formulas=[10,10],second_valid=True,hit_residues=[799,800],hit_chance=800,
                  spread_radius=2,critical_residues=[5,None],spread_residues=[1,None])
        expected=gate.oracle.rational(gate.residue_family('dual_plan',dual))
        rev8=gate.oracle.behavior.dual_plan
        def rev9(hits,formulas,second_valid,hit_residues,hit_chance,spread_radius,critical_residues,spread_residues):
            # A rev 9-shaped oracle: strict `<` and the residues as parameters.
            result=rev8(hits,formulas,second_valid,hit_residues,hit_chance-1,spread_radius)
            for draw in result['draws']:
                if draw['site'][:-1] in ('critical','spread'):
                    draw['residue']=(critical_residues if draw['site'].startswith('critical') else spread_residues)[int(draw['site'][-1])-1]
            return result
        with patch.object(gate.oracle.behavior,'dual_plan',rev9),patch.object(gate,'oracle_hit_inclusive',side_effect=AssertionError('probe used')):
            self.assertTrue(gate.oracle_adopted_d87('dual_plan'))
            gate.formula('dual_plan',dual,expected) # no adjustment, no probe, any data hash
            with patch.object(gate,'source_data_sha256',return_value='f'*64):gate.formula('dual_plan',dual,expected)
        # Rev 8-shaped oracle on unknown (newer) data: the adapter refuses as a verdict.
        with patch.object(gate,'source_data_sha256',return_value='f'*64):
            with self.assertRaisesRegex(ValueError,'D96'):gate.formula('dual_plan',dual,expected)
            rows=copy.deepcopy(self.rows)
            rows[1]['data'].update(formula='dual_plan',inputs=dual,actual=expected)
            result=gate.evaluate(rows,self.recording,OWNER,['dual_plan'],HASH)
            self.assertFalse(result['passed']);self.assertTrue(any('D96' in e for g in result['gates'] for e in g['errors']))
        # A probe that cannot classify the oracle is a ValueError verdict, not a TypeError crash.
        def odd(*args,critical_residues=None,**kwargs):raise TypeError('unexpected signature')
        gate.oracle_hit_inclusive.cache_clear()
        try:
            with patch.object(gate.oracle.behavior,'dual_plan',odd):
                with self.assertRaisesRegex(ValueError,'D88'):gate.oracle_hit_inclusive()
        finally:gate.oracle_hit_inclusive.cache_clear()
        self.assertTrue(gate.oracle_hit_inclusive())
        print('Rev 9 path: residue parameters detected by signature remove both adapters; unknown data or an unclassifiable probe is a verdict',flush=True)

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
                            ('bow_damage',dict(formula=100,dx_milli=-12500,dy_milli=0))]:
            actual=gate.oracle.rational(getattr(gate.oracle,name)(**inputs))
            rows=copy.deepcopy(self.rows);rows[1]['data'].update(formula=name,inputs=inputs,actual=actual)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,[name],HASH)['passed'],name)

    def test_numeric_input_types_before_oracle(self):
        for inputs in ({'base':True},{'base':{'numerator':True,'denominator':1}},{'base':100,'bonus':False}):
            with self.assertRaises(ValueError):gate.formula('enchant_chance',inputs,1000)
        with self.assertRaises(ValueError):gate.formula('enchant_outcome',dict(base=100,bonus=0,support=True,residue=0,enchant=0),{})
        with self.assertRaises(ValueError):gate.formula('pole_damage',dict(formula=100,primary=1),{})
        for residue in ('0',{'numerator':0,'denominator':1},True):
            with self.assertRaises(ValueError):gate.formula('fist_spread',dict(level=14,residue=residue),{})
        gate.formula('pole_damage',dict(formula=100,primary=True),gate.oracle.rational(gate.oracle.pole_damage('100',primary=True)))

    @staticmethod
    def with_residues(name,inputs):
        """D87: read the oracle's own critical/spread residues back as fact inputs."""
        def canonical(value):
            # Review-2 P3-3: E1 decimal-point strings ('1.25') become canonical n/d rationals.
            if isinstance(value,dict):return {k:canonical(v) for k,v in value.items()}
            if isinstance(value,list):return [canonical(v) for v in value]
            if isinstance(value,str) and re.fullmatch(r'-?[0-9]+\.[0-9]+',value):
                number=gate.oracle.F(value);return f'{number.numerator}/{number.denominator}'
            return value
        inputs=canonical(copy.deepcopy(inputs))
        if name=='magic_damage' and 'is_pvp' not in inputs:
            # Review-2 P3-4: is_pvp is required; E1 vectors leave it to the oracle default.
            inputs['is_pvp']=inputs.get('attacker_player',True) and inputs.get('target_player',False)
        if name not in gate.RESIDUE_INPUTS:return inputs
        function=getattr(gate.oracle.behavior,name)
        try:draws={d['site']:d['residue'] for d in function(**gate.oracle.decode_rationals(inputs))['draws']}
        except (ValueError,TypeError,KeyError):draws={}
        if name=='bow_launch':
            # D89: E1 vectors give untaken draws a placeholder; facts carry null instead.
            inputs.update(critical_residue=draws.get('critical'),spread_residue=draws.get('spread'),
                          reduction_residue=draws.get('bow_mp_reduction'),hit_residue=draws.get('hit'))
        else:inputs.update(critical_residues=[draws.get('critical1'),draws.get('critical2')],
                           spread_residues=[draws.get('spread1'),draws.get('spread2')])
        return inputs

    def formula_rows(self,name,inputs):
        rows=copy.deepcopy(self.rows)
        function=getattr(gate.oracle,name,None) or getattr(gate.oracle.behavior,name,None)
        if name in gate.RESIDUE_INPUTS:function=lambda **_:gate.residue_family(name,inputs)
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
                # D87 adds the critical/spread residues the E1 rev 8 oracle still hard-codes.
                self.assertEqual(set(gate.INPUT_SCHEMAS[name]),set(parameters)|set(gate.RESIDUE_INPUTS.get(name,())))
                for key,param in parameters.items():
                    # D89: always-evaluated booleans are never null; draw-derived flags may be.
                    if (name,key)==('magic_damage','is_pvp'):self.assertEqual(gate.INPUT_SCHEMAS[name][key],gate.BOOL)
                    elif (name,key)==('magic_damage','first_success'):self.assertEqual(gate.INPUT_SCHEMAS[name][key],gate.nullable(gate.BOOL))
                    elif param.default is None:self.assertEqual(gate.INPUT_SCHEMAS[name][key][0],'nullable')
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
                inputs=self.with_residues(name,inputs)
                with self.subTest(formula=name,vector=vector['id']):gate.input_domains(name,inputs)
                controls+=1;formulas.add(name)
        self.assertGreaterEqual(controls,3202)
        self.assertGreaterEqual(len(formulas),43)

    def test_next_tranche_actual_schemas_match_e1_outputs(self):
        """Revision 17: every `actual` shape of the 14 next-tranche families (+bow_damage, lethal)
        accepts the E1 oracle's own outputs on E1 vectors, and rejects shape mutations."""
        import inspect
        families={'bow_launch','bow_reuse','magic_damage','heal','gear_stats','set_shield_history','clamp_no_heal',
                  'charge_transition','consumable_use','escape_timing','resurrection_scroll','learned_item_cost',
                  'pole_sequence','dual_plan','bow_damage','lethal'}
        self.assertEqual(set(gate.ACTUAL_SCHEMAS),families)
        vectors=json.loads((gate.REPO/'packages/data/fixtures/item-oracle.json').read_text())['vectors']
        covered={};src=gate.source()
        for name in sorted(families):
            function=getattr(gate.oracle,name,None) or getattr(gate.oracle.behavior,name,None)
            parameters=inspect.signature(function).parameters
            required={k for k,p in parameters.items() if k!='src' and p.default is inspect.Parameter.empty}
            for vector in vectors:
                inputs=copy.deepcopy(vector['inputs'])
                if not required<=set(inputs) or set(inputs)-set(gate.INPUT_SCHEMAS[name]):continue
                if name=='resurrection_scroll':inputs['repeat']=bool(inputs['repeat'])
                inputs=self.with_residues(name,inputs)
                try:
                    gate.input_domains(name,inputs)
                    decoded=gate.oracle.decode_rationals(inputs)
                    if name in gate.RESIDUE_INPUTS:output=gate.residue_family(name,inputs)
                    else:output=function(src,**decoded) if 'src' in parameters else function(**decoded)
                except (ValueError,TypeError,KeyError,IndexError,ZeroDivisionError):continue
                actual=gate.oracle.rational(output)
                with self.subTest(formula=name,vector=vector['id']):
                    gate.validate_input(actual,gate.ACTUAL_SCHEMAS[name],name+'.actual')
                covered[name]=covered.get(name,0)+1
        # Families without a signature-compatible E1 vector get a direct oracle control.
        direct={'bow_reuse':dict(reuse_delay=1500,attack_speed=400),'clamp_no_heal':dict(current=120,new_max={'numerator':199,'denominator':2}),
                'set_shield_history':dict(previous=False,mutation='equip_shield',chest_present=True,matching_shield=True),
                'charge_transition':dict(charges=['soul','blessed'],use_spirit=True),
                'lethal':dict(full=10,half=20,full_residue=500,half_residue=150),
                'bow_damage':dict(formula=100,dx_milli=12499,dy_milli=7),
                'pole_sequence':dict(candidates=[dict(eligible=True,hit=True,formula='499/10'),dict(eligible=True,hit=False,formula=0)],maximum=3),
                'magic_damage':dict(power=10,m_atk=100,m_def=50,spirit=True,is_pvp=False),
                'heal':dict(power=100,m_atk=200,current_hp=10,recoverable_hp=500)}
        for name,inputs in direct.items():
            with self.subTest(formula=name,direct=True):
                actual=gate.oracle.rational((getattr(gate.oracle,name,None) or getattr(gate.oracle.behavior,name))(**gate.oracle.decode_rationals(inputs)))
                gate.formula(name,inputs,actual);covered[name]=covered.get(name,0)+1
        self.assertEqual(set(covered),families,covered)
        # Shape mutations reject before oracle equality (booleans/text masquerading as numbers, missing keys).
        actual=gate.oracle.rational(gate.oracle.bow_damage('100',12499,7))
        for mutate in (lambda a:a.update(damage=True),lambda a:a.update(damage='90'),lambda a:a.pop('base'),
                       lambda a:a.update(extra=1),lambda a:a.update(distance_l2_lower=1.5)):
            bad=copy.deepcopy(actual);mutate(bad)
            with self.assertRaises(ValueError):gate.formula('bow_damage',dict(formula=100,dx_milli=12499,dy_milli=7),bad)
        with self.assertRaisesRegex(ValueError,'set_shield_history.actual'):
            gate.formula('set_shield_history',direct['set_shield_history'],1)
        with self.assertRaisesRegex(ValueError,'charge_transition.actual'):
            gate.formula('charge_transition',direct['charge_transition'],['shot'])
        print('Revision 17 actual schemas: E1 outputs accepted '+', '.join(f'{k}:{v}' for k,v in sorted(covered.items())),flush=True)

    def test_scalar_shape_mutations_on_perfect_block_branches(self):
        # Recompute output: these oracle branches deliberately never read operands.
        for name,inputs in [('physical_damage',dict(p_atk=10,defence=10,perfect=True)),
                            ('magic_damage',dict(power=10,m_atk=10,m_def=10,perfect=True,is_pvp=False))]:
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
            ('magic_damage',dict(power=10,m_atk=10,m_def=10,perfect=True,is_pvp=True),'is_pvp',
             ([],[True],{},0,1,'bad',None)),
            ('shot_payment',dict(charges=[],kind='soul',count=1,available=0,weapon_grade=None,shot_grade='D'),
             'weapon_grade',([],['D'],{},True,1,'bad')),
            ('shot_payment',dict(charges=[],kind='soul',count=1,available=0,weapon_grade='D',shot_grade='D',reduction=[]),
             'reduction',(None,1,{},[1],[0,1,2],[None,1],[1,-1],[1,2**63],[101,1])),
            ('dual_plan',dict(hits=[True,False],formulas=[10,'3/2'],second_valid=True,
                             hit_residues=[799,800],hit_chance=800,spread_radius=6,
                             critical_residues=[999,None],spread_residues=[6,None]),'formulas',
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
                            ('magic_damage',dict(power=10,m_atk=10,m_def=10,perfect=True,is_pvp=False)),
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
               ('magic_damage',dict(power=1,m_atk=1,m_def=1,is_pvp=True,second_success=None)),
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
            # D88: E1's boundary hit (residue == chance) is a miss now; take the residue just below.
            if name=='bow_launch' and not inputs['miss'] and inputs['hit_residue']==inputs['hit_chance']:inputs['hit_residue']-=1
            cases.append((name,self.with_residues(name,inputs)))
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
                      before=item(8,1,15),after=item(8,1,15,'rhand')|{'revision':1},count_delta=0,operation_ordinal=0,receipt_ref='equip:1')))
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
            gate.formula('bow_damage',dict(formula=100,dx_milli=dx,dy_milli=0),expected)
        gate.formula('bow_reuse',dict(reuse_delay=1500,attack_speed=400,weapon_reuse_modifier=2),1248)
        bad=copy.deepcopy(self.rows);bad[1]['data']['actual']['damage']=42
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH)['passed'])

    def test_d80_wide_integers_parse_as_decimal_strings(self):
        def stringify(value):
            # D80 producer encoding: integers outside ±2^53 become decimal strings.
            if isinstance(value,dict):return {k:stringify(v) for k,v in value.items()}
            if isinstance(value,list):return [stringify(v) for v in value]
            if type(value) is int and abs(value)>2**53:return str(value)
            return value
        # RNG word positions are u128 cursors.
        rows=copy.deepcopy(self.rows)
        rows.insert(2,dict(kind='item_rng',entity=OWNER,incarnation=1,tick=0,sequence=1,data=dict(resolution_id='npc:shield:1',
            callsite='shield_perfect',word_before=str(2**64),word_after=str(2**64+1),bound=100,value=7)))
        self.repair(rows);gate.validate(rows,self.recording,HASH)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        numeric=copy.deepcopy(rows);numeric[2]['data'].update(word_before=2**64,word_after=2**64+1)
        gate.validate(numeric,self.recording,HASH) # the consumer parses both forms
        for value in ('5','0','-1',str(2**53-1),str(-(2**53-1))):
            bad=copy.deepcopy(rows);bad[2]['data']['word_before']=value
            with self.assertRaisesRegex(ValueError,'inside ±2\\^53',msg=value):gate.validate(bad,self.recording,HASH)
        for value in ('0x10',' 18446744073709551616','+18446744073709551616','018446744073709551616','1e30','1.0'):
            bad=copy.deepcopy(rows);bad[2]['data']['word_before']=value
            with self.assertRaises(ValueError,msg=value):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[2]['data']['word_after']=str(2**64-1)
        with self.assertRaisesRegex(ValueError,'RNG cursor'):gate.validate(bad,self.recording,HASH)
        # Canonical instance ids beyond 2^53 (allocator ranges reach 2^63): baseline, delta, causes.
        wide=2**60+100;rows=self.transition_rows('consume')
        for row in rows[1:-1]:
            data=row['data']
            if row['kind']=='item_baseline':data['instances'][0]['id']=wide
            elif row['kind']=='item_delta':
                data['instance']=wide
                for key in ('before','after'):
                    if data[key]:data[key]['id']=wide
        encoded=stringify(rows)
        self.assertEqual(encoded[3]['data']['instance'],str(wide))
        gate.validate(encoded,self.recording,HASH);gate.validate(rows,self.recording,HASH)
        self.assertTrue(gate.evaluate(encoded,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        bad=copy.deepcopy(encoded);bad[3]['data']['instance']=str(wide+1)
        with self.assertRaisesRegex(ValueError,'identity mismatch'):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(encoded);bad[3]['data']['before']['id']=str(2**63)
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(encoded);bad[3]['data']['count_delta']='-1'
        with self.assertRaisesRegex(ValueError,'inside ±2\\^53'):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(encoded);bad[3]['data']['causes'][0]['count_delta']='-1'
        with self.assertRaisesRegex(ValueError,'inside ±2\\^53'):gate.validate(bad,self.recording,HASH)
        # Text fields never convert: a decimal resolution identity stays text.
        digits=copy.deepcopy(rows);digits[2]['data']['resolution_id']=str(2**64)
        self.assertTrue(gate.evaluate(digits,self.recording,OWNER,['pole_damage'],HASH)['passed'])
        # bow_damage: certified 2^-bits sqrt bounds carry > i64 numerators in `actual`.
        inputs=dict(formula=100,dx_milli=12499,dy_milli=7)
        actual=gate.oracle.rational(gate.oracle.bow_damage(**inputs))
        self.assertGreater(actual['distance_l2_lower']['denominator'],2**63)
        encoded_actual=stringify(actual)
        self.assertIsInstance(encoded_actual['distance_l2_lower']['denominator'],str)
        gate.formula('bow_damage',inputs,encoded_actual)
        rows=copy.deepcopy(self.rows);rows[1]['data'].update(formula='bow_damage',inputs=inputs,actual=encoded_actual)
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['bow_damage'],HASH)['passed'])
        bad=copy.deepcopy(encoded_actual);bad['distance_l2_lower']['numerator']=str(int(bad['distance_l2_lower']['numerator'])+1)
        with self.assertRaises(ValueError):gate.formula('bow_damage',inputs,bad)
        # Wide inputs decode, then the signed-bigint input domain still applies.
        gate.formula('bow_damage',dict(formula=100,dx_milli=str(2**60),dy_milli=0),
                     stringify(gate.oracle.rational(gate.oracle.bow_damage('100',2**60,0))))
        for value in (str(2**63),'5'):
            with self.assertRaises(ValueError,msg=value):gate.formula('bow_damage',dict(formula=100,dx_milli=value,dy_milli=0),actual)
        with self.assertRaises(ValueError):
            gate.formula('pole_damage',dict(formula={'numerator':str(2**64),'denominator':'1'},percent=85,primary=False),None)
        print('D80: wide RNG cursors, instance ids, bow bounds and inputs parse as decimal strings; in-range strings and malformed decimals reject',flush=True)

    def test_d81_lethal_residues_null_iff_draw_not_taken(self):
        base=dict(full=10,half=20,eligible=True,attribute=1,trait=1)
        controls=[(dict(full_residue=50,half_residue=None),'full'),(dict(full_residue=500,half_residue=150),'half'),
                  (dict(full_residue=500,half_residue=999),'none'),(dict(full_residue=999,half_residue=0),'half'),
                  (dict(full_residue=None,half_residue=None,eligible=False),'none')]
        for change,outcome in controls:
            with self.subTest(control=change):
                rows=self.formula_rows('lethal',base|change)
                self.assertEqual(rows[1]['data']['actual']['outcome'],outcome)
                result=gate.evaluate(rows,self.recording,OWNER,['lethal'],HASH);self.assertTrue(result['passed'],result)
        # The withdrawn 999 sentinel (and any other value) for an untaken draw, and null for a taken draw.
        mutations=[dict(full_residue=50,half_residue=999),dict(full_residue=50,half_residue=0),
                   dict(full_residue=500,half_residue=None),dict(full_residue=None,half_residue=150),
                   dict(full_residue=None,half_residue=None),
                   dict(full_residue=999,half_residue=999,eligible=False),dict(full_residue=None,half_residue=999,eligible=False)]
        for change in mutations:
            with self.subTest(mutation=change):
                rows=self.formula_rows('lethal',base|change)
                if rows[1]['data']['actual'] is None:rows[1]['data']['actual']={'outcome':'none','draws':[]}
                result=gate.evaluate(rows,self.recording,OWNER,['lethal'],HASH);self.assertFalse(result['passed'])
                with self.assertRaises(ValueError):gate.formula('lethal',base|change,rows[1]['data']['actual'])
        with self.assertRaisesRegex(ValueError,'D81'):
            gate.formula('lethal',base|dict(full_residue=50,half_residue=999),gate.oracle.rational(gate.oracle.lethal(**base|dict(full_residue=50,half_residue=999))))
        print(f'D81: {len(controls)} lethal residue controls pass; {len(mutations)} sentinel/null mismatches reject',flush=True)

    def test_exact_numeric_strings_are_rationals_or_wide_decimals(self):
        """Review-2 P3-3 (D80): exact-numeric inputs accept n/d rationals and wide decimals only."""
        def run(formula,good=True):
            inputs=dict(formula=formula,percent=85,primary=False)
            actual=gate.oracle.rational(gate.oracle.pole_damage(**gate.oracle.decode_rationals(inputs))) if good else None
            gate.formula('pole_damage',inputs,actual)
        for good in (100,'499/10','-3/2','0/1',str(2**60),{'numerator':499,'denominator':10}):
            with self.subTest(good=good):run(good)
        for bad in ('100','1.25','1e2','+3/2','03/2','3/02','3/0',' 3/2','0x10','-0'):
            with self.subTest(bad=bad):
                with self.assertRaisesRegex(ValueError,'invalid formula input'):run(bad,False)
        print('Exact-numeric strings: n/d rationals and wide decimals pass; in-range integer, decimal-point and exponent strings reject',flush=True)

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
        rows.insert(3,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=2,data=dict(resolution_id='enchant:1',operation='consume',instance=7,before=item(7,2),after=item(7,1)|{'revision':1},count_delta=-1,operation_ordinal=0,receipt_refs=['receipt:enchant:1'],causes=[dict(cause='receipt:enchant:1',count_delta=-1)])))
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
        rows.insert(2,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=1,data=dict(resolution_id='enchant:1',operation='grant',instance=2,before=None,after=item(2,1,1)|{'revision':1},count_delta=1,operation_ordinal=0,receipt_ref=None)))
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
                append('item_delta',dict(operation='equip',instance=identity,before=item(identity,1,template),after=item(identity,1,template,'rhand')|{'revision':1},count_delta=0,operation_ordinal=0,receipt_ref=None))

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
