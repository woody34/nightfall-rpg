#!/usr/bin/env python3
"""Synthetic exporter validation and independent actual-input Fraction gate tests."""
import copy
import importlib.util
import json
import random
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

    def assert_rejected(self,rows,gates=('enchant_outcome',)):
        try:result=gate.evaluate(rows,self.recording,OWNER,gates,HASH)
        except ValueError:return
        self.assertFalse(result['passed'])

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
               operation=dict(target_before=target,scroll_before=scroll,support_before=support_item,receipt_ref='receipt:1')))
        chance_inputs={k:inputs[k] for k in ('base','bonus','support')}
        append('item_formula',dict(formula='enchant_chance',inputs=chance_inputs,actual=gate.oracle.enchant_chance(**chance_inputs),baseline=False))
        append('item_rng',dict(callsite='enchant',word_before=0,word_after=1,bound=100000,value=residue))
        for spent in (scroll,support_item):
            if spent:append('item_delta',dict(operation='consume',instance=spent['id'],before=spent,after=None,count_delta=-1,operation_ordinal=0,receipt_refs=['receipt:1']))
        if result['destroyed'] or result['enchant']!=enchant:
            after=None if result['destroyed'] else target|dict(enchant=result['enchant'],revision=1)
            append('item_delta',dict(operation='destroy' if result['destroyed'] else 'enchant',instance=target_id,before=target,after=after,count_delta=-1 if after is None else 0,operation_ordinal=0,receipt_ref='receipt:1'))
        if result['destroyed']:
            definition=gate.item_tables()[71];grade=gate.source().grades[definition['grade']]
            refund_inputs=dict(base=definition['crystal_count'],bonus=grade['weapon_bonus'],enchant=enchant,weapon=True,crystallizable=definition['flags']['is_crystallizable'],crystal_id=grade['crystal_id'])
            refund=gate.oracle.failure_refund(**refund_inputs)
            append('item_formula',dict(formula='failure_refund',inputs=refund_inputs,actual=refund,baseline=False))
            if refund:append('item_delta',dict(operation='grant',instance=10,before=None,after=item(10,refund,grade['crystal_id'])|{'revision':1},count_delta=refund,operation_ordinal=0,receipt_ref='receipt:1'))
        rows.append(dict(kind='complete',facts=len(rows)-1,ticks=1,last_tick=0))
        return rows

    def same_tick_enchants(self):
        rows=[copy.deepcopy(self.rows[0])]
        for index in range(2):
            attempt=self.enchant_rows(enchant=index,scroll_id=20+index)
            attempt[1]['data']['operation']['target_before']['revision']=min(index,1)
            attempt[1]['data']['operation']['receipt_ref']=f'receipt:{index+1}'
            for row in attempt[1:-1]:
                row['data']['resolution_id']=f'enchant:{index+1}'
                if row['kind']=='item_rng':row['data'].update(word_before=index,word_after=index+1)
                if row['kind']=='item_delta':
                    if row['data']['operation']=='consume':row['data']['receipt_refs']=[f'receipt:{index+1}']
                    else:row['data']['receipt_ref']=f'receipt:{index+1}'
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
                          count_delta=-2,operation_ordinal=0,resolution_id='enchant:1',receipt_refs=['receipt:1','receipt:2'])))
        return self.repair(rows)

    def test_shared_scroll_stack_consumes_ordered_resolution_receipts(self):
        for count,support in ((2,False),(3,False),(2,True),(3,True)):
            rows=self.shared_stack_enchants(count,support)
            self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'],(count,support))
            for index in range(1,len(rows)-1):
                bad=copy.deepcopy(rows);del bad[index];self.repair(bad)
                self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'],(count,support,index))

    def test_shared_scroll_stack_rejects_contribution_order_and_total_mutations(self):
        rows=self.shared_stack_enchants()
        for receipts in (['receipt:1'],['receipt:2'],['receipt:2','receipt:1'],['receipt:1','receipt:1'],['receipt:1','receipt:2','receipt:3']):
            bad=copy.deepcopy(rows)
            next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')['receipt_refs']=receipts
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'],receipts)
        for total in (1,3):
            bad=copy.deepcopy(rows)
            aggregate=next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')
            if total==1:aggregate['after']=aggregate['before']|{'count':1,'revision':1}
            else:aggregate['before']=aggregate['before']|{'count':3}
            aggregate['count_delta']=-total
            gate.validate(bad,self.recording,HASH)
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'],total)
        for receipt in (None,'',[],{},False,'receipt:1','unrelated'):
            bad=copy.deepcopy(rows)
            [r for r in bad if r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome'][-1]['data']['operation']['receipt_ref']=receipt
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'],receipt)
        for field,value in (('entity',OTHER),('incarnation',2)):
            bad=copy.deepcopy(rows)
            next(r for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')[field]=value
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
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
            self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['enchant_outcome','enchant_chance'],HASH)['passed'],index)
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
            if row['kind']=='item_formula' and data['formula']=='enchant_outcome':data['operation']['receipt_ref']='receipt:1'
            elif row['kind']=='item_delta':
                data['receipt_refs' if data['operation']=='consume' else 'receipt_ref']=['receipt:1'] if data['operation']=='consume' else 'receipt:1'
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
                # Equal revisions remain valid; lowering the later binding then rolls back.
                outcomes[0]['operation'][key]['revision']=1
                next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['instance']==outcomes[0]['operation'][key]['id'])['before']['revision']=1
                self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
                outcomes[1]['operation'][key]['revision']=0
                self.assertFalse(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])

    def generated_enchants(self,rng):
        ticks=rng.randint(2,3);stack_ids=rng.sample(range(20,40),rng.randint(1,3))
        plans=[[rng.choice(stack_ids) for _ in range(rng.randint(3,4))] for _ in range(ticks)]
        support=rng.choice((False,True));initial=rng.randint(0,20)
        counts={identity:sum(plan.count(identity) for plan in plans)+rng.randint(0,3) for identity in stack_ids}
        if support:counts[9]=sum(map(len,plans))+rng.randint(0,3)
        revisions={identity:initial for identity in counts}
        incarnation=rng.randint(1,5);start=rng.randint(0,30);cursor=0;levels={8:0,10:0}
        rows=[copy.deepcopy(self.rows[0])]
        baseline=[item(identity,count,12363 if identity==9 else 951)|{'revision':initial} for identity,count in counts.items() if count]
        baseline.extend(item(identity,1,71)|{'revision':initial} for identity in levels)
        rows.append(dict(kind='item_baseline',entity=OWNER,incarnation=incarnation,tick=start,sequence=0,
                         data=dict(inventory_revision=initial,instances=baseline,paperdoll={},auto_shots=[])))
        for tick,plan in enumerate(plans):
            revisions={identity:initial+tick for identity in counts}
            contributions={};ordinals={identity:rng.randint(0,4) for identity in levels};changed=set()
            for index,scroll_id in enumerate(plan):
                target_id=8 if index==0 else 10 if index==1 else rng.choice((8,10))
                level=levels[target_id];target=item(target_id,1,71)|{'enchant':level}
                base=int(gate.enchant_source_inputs(target,item(scroll_id,1,951),item(9,1,12363) if support else None)['base'])
                attempt=self.enchant_rows(base=base,enchant=level,target_id=target_id,scroll_id=scroll_id,support=support)
                resolution=f'enchant:{rng.getrandbits(64):016x}:{cursor}'
                receipt=f'receipt:{rng.getrandbits(64):016x}:{cursor}'
                operation=attempt[1]['data']['operation']
                operation['receipt_ref']=receipt
                operation['target_before']['revision']=initial+tick+int(target_id in changed)
                for key in ('scroll_before','support_before'):
                    before=operation[key]
                    if before is None:continue
                    identity=before['id'];before.update(count=counts[identity],revision=revisions[identity])
                    contributions.setdefault(identity,[]).append((resolution,receipt,copy.deepcopy(before)))
                    counts[identity]-=1;revisions[identity]=initial+tick+1
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
                ordinals[target_id]+=rng.randint(1,3);changed.add(target_id);cursor+=1;levels[target_id]+=1
            for identity,parts in contributions.items():
                resolution,_,before=parts[0]
                rows.append(dict(kind='item_delta',entity=OWNER,incarnation=incarnation,tick=start+tick,sequence=0,
                    data=dict(operation='consume',instance=identity,before=copy.deepcopy(before),
                              after=before|{'count':counts[identity],'revision':initial+tick+1} if counts[identity] else None,
                              count_delta=-len(parts),operation_ordinal=0,resolution_id=resolution,
                              receipt_refs=[receipt for _,receipt,_ in parts])))
        rows[0].update(snapshot_tick=start)
        rows[0]['watermark'].update(records=ticks,last_tick=start+ticks-1)
        rows.append(dict(kind='complete',facts=0,ticks=ticks,last_tick=start+ticks-1))
        other=copy.deepcopy(rows[1:-1])
        for row in other:
            row['entity']=OTHER
            data=row['data']
            if row['kind']=='item_baseline':
                data['inventory_revision']+=100
                for state in data['instances']:state['revision']+=100
            elif row['kind']=='item_delta':
                for key in ('before','after'):
                    if data[key] is not None:data[key]['revision']+=100
            elif row['kind']=='item_formula' and data['formula']=='enchant_outcome':
                for state in data['operation'].values():
                    if isinstance(state,dict):state['revision']+=100
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
               dict(operation='grant',before=first,after=first|{'count':3},count_delta=1),
               dict(operation='destroy',before=first|{'count':3},after=None,count_delta=-3),
               dict(operation='grant',before=None,after=first,count_delta=2)]
        for ordinal,data in enumerate(facts):
            rows.insert(-1,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=0,
                data=data|dict(instance=70,operation_ordinal=ordinal,resolution_id=f'change:{ordinal}',receipt_ref=None)))
        self.repair(rows);gate.validate(rows,self.recording,HASH)
        for key in ('count','enchant','revision','template_id'):
            bad=copy.deepcopy(rows);bad[3]['data']['before']=bad[3]['data']['before']|{key:4 if key=='count' else 2 if key!='template_id' else 952}
            if key=='count':bad[3]['data']['count_delta']=-1
            with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)
        # An incarnation boundary starts a separate history, even for the same ID.
        for row in rows[3:-1]:row['incarnation']=2
        gate.validate(rows,self.recording,HASH)

    def test_seeded_aggregate_correlation_property_check_2000_cases(self):
        rng=random.Random(0xE609)
        mutations=('receipt reuse','swapped ordinals','cross-tick revision','wrong totals',
                   'missing contributions','duplicated aggregates','foreign-owner rows',
                   'per-instance revision drift','support/scroll endpoint mismatch','cross-owner contamination',
                   'canonical count discontinuity')
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
                            if data['operation']=='consume':data['receipt_refs']=[new if ref==old else ref for ref in data['receipt_refs']]
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
                    aggregate=next((r for r in aggregates if len(r['data']['receipt_refs'])>1),aggregates[0])
                    aggregate['data']['receipt_refs']=aggregate['data']['receipt_refs'][:-1]
                elif mutation=='duplicated aggregates':
                    aggregate=rng.choice(aggregates);bad.insert(bad.index(aggregate)+1,copy.deepcopy(aggregate));self.repair(bad)
                elif mutation=='foreign-owner rows':rng.choice(aggregates)['entity']='01970000-0000-7000-8000-000000000042'
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
                                 and r['data']['instance']==aggregate['data']['instance'])
                    for key in ('before','after'):aggregate['data'][key]=copy.deepcopy(foreign['data'][key])
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
                self.assertFalse(passes(bad),f'{mutation}, case {case}')
        print('Aggregate property check: seed=0xE609; 2000 valid two-owner sets pass; 22000 corruptions reject',flush=True)

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
        scroll=rows[1]['data']['operation']['scroll_before'];scroll.update(count=2,revision=1)
        consume=next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['operation']=='consume')
        consume['after']=scroll|{'count':1}
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome'],HASH)['passed'])
        bad=copy.deepcopy(rows)
        next(r['data'] for r in bad if r['kind']=='item_delta' and r['data']['operation']=='consume')['after']['revision']=0
        with self.assertRaisesRegex(ValueError,'revision regressed'):gate.validate(bad,self.recording,HASH)
        rows=self.enchant_rows(base=70,residue=99999,enchant=3)
        refund=next(r['data'] for r in rows if r['kind']=='item_delta' and r['data']['operation']=='grant')
        refund['before']=refund['after']|{'count':5,'revision':1}
        refund['after']=refund['after']|{'count':5+refund['count_delta'],'revision':1}
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['enchant_outcome','failure_refund'],HASH)['passed'])

    def test_every_later_same_tick_post_revision_must_advance_and_share_step(self):
        rows=self.same_tick_enchants()
        for index in (2,3):
            attempt=self.enchant_rows(base=100 if index==2 else 70,enchant=index,scroll_id=20+index)
            attempt[1]['data']['operation']['target_before']['revision']=index-1
            attempt[1]['data']['operation']['receipt_ref']=f'receipt:{index+1}'
            for row in attempt[1:-1]:
                row['tick']=1;row['data']['resolution_id']=f'enchant:{index+1}'
                if row['kind']=='item_rng':row['data'].update(word_before=index,word_after=index+1)
                if row['kind']=='item_delta':
                    row['data']['receipt_refs' if row['data']['operation']=='consume' else 'receipt_ref']=[f'receipt:{index+1}'] if row['data']['operation']=='consume' else f'receipt:{index+1}'
                    if row['data']['operation']=='consume':row['data']['before']['revision']=1
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
                      before=item(7,3),after=item(7,1),count_delta=-2,operation_ordinal=0,receipt_refs=['receipt:2','receipt:1'])))
        self.repair(rows)
        phase=dict(name='consume-aggregate',formula='pole_damage',mutation=dict(operation='consume',template=17,slot=None))
        self.assertTrue(gate.evaluate(rows,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        self.assertEqual(rows[2]['data']['receipt_refs'],['receipt:2','receipt:1'])
        bad=copy.deepcopy(rows);del bad[2];self.repair(bad)
        self.assertFalse(gate.evaluate(bad,self.recording,OWNER,['pole_damage'],HASH,[phase])['passed'])
        for value in ([],None,'receipt:1',{},[None],[''],[1],[False],[['receipt:1']]):
            bad=copy.deepcopy(rows);bad[2]['data']['receipt_refs']=value
            with self.assertRaises(ValueError,msg=value):gate.validate(bad,self.recording,HASH)
        for name in ('receipt','receipt_ref',None):
            bad=copy.deepcopy(rows);del bad[2]['data']['receipt_refs']
            if name:bad[2]['data'][name]='receipt:1'
            with self.assertRaises(ValueError,msg=name):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad.insert(-1,copy.deepcopy(bad[2]));bad[3]['data']['resolution_id']='other';self.repair(bad)
        with self.assertRaisesRegex(ValueError,'ordinal'):gate.validate(bad,self.recording,HASH)
        rows[2]['data'].update(operation='exchange',receipt_ref='exchange:1');del rows[2]['data']['receipt_refs']
        gate.validate(rows,self.recording,HASH)
        for value in (None,[],['exchange:1'],'',False):
            bad=copy.deepcopy(rows);bad[2]['data']['receipt_ref']=value
            with self.assertRaises(ValueError,msg=value):gate.validate(bad,self.recording,HASH)
        bad=copy.deepcopy(rows);bad[2]['data']['receipt_refs']=['exchange:1']
        with self.assertRaises(ValueError):gate.validate(bad,self.recording,HASH)

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
                      before=item(8,1,15),after=item(8,1,15,'rhand'),count_delta=0,operation_ordinal=0,receipt_ref='equip:1')))
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
        rows.insert(3,dict(kind='item_delta',entity=OWNER,incarnation=1,tick=0,sequence=2,data=dict(resolution_id='enchant:1',operation='consume',instance=7,before=item(7,2),after=item(7,1),count_delta=-1,operation_ordinal=0,receipt_refs=['receipt:1'])))
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
