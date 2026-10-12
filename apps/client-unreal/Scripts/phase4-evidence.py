#!/usr/bin/env python3
"""C14 verified-export / C15 actual-input item oracle consumer.

Proposed E2 producer contract is explicit; unknown schemas, facts and baseline-only
exports fail closed. Unit fixtures are synthetic, never live replay attestations.
"""
import argparse
from functools import lru_cache
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
from uuid import UUID
import xml.etree.ElementTree as ET

SCRIPTS=Path(__file__).resolve().parent
REPO=SCRIPTS.parents[2]
sys.path.insert(0,str(REPO/'packages/data/scripts'))
import item_oracle as oracle

FORMULAS=frozenset(('enchant_increment enchant_hp crystal_count failure_refund enchant_chance enchant_outcome '
    'enchant_valid max_load overload grade_penalties fist_spread pole_damage pole_sequence bow_reuse bow_damage '
    'dual_damage dual_timing skill_caller_damage position shield physical_damage magic_damage cast_time heal '
    'blow_success lethal retail_select retail_distribution category pack_options cancel_fee set_shield_history '
    'clamp_no_heal charge_transition shot_payment shared_shot_bundle winning_providers consumable_use '
    'consumable_cast_sequence player_level_range bow_launch bow_refill resurrection_scroll dormant_use '
    'learned_item_cost escape_timing dual_plan enchant_safe quantized_enchant augmentation_provider '
    'retail_draw_contract item_regen_status gear_stats sa_exchange grant_denied').split())


def unique_object(pairs):
    result={}
    for key,value in pairs:
        if key in result:raise ValueError('duplicate JSON key: '+key)
        result[key]=value
    return result


def no_float(value):raise ValueError('non-exact JSON number: '+value)


def read_jsonl(path):
    return [json.loads(line,object_pairs_hook=unique_object,parse_float=no_float,parse_constant=no_float)
            for line in path.read_text().splitlines()]


def same(actual,expected):
    if json.dumps(actual,sort_keys=True,separators=(',',':'))!=json.dumps(expected,sort_keys=True,separators=(',',':')):
        raise ValueError('item source expectation mismatch')


def integer(value,minimum=0):
    if type(value) is not int or value<minimum:raise ValueError('integer evidence required')
    return value


def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def requirements(scenario):
    result=[]
    for line in scenario.read_text().splitlines():
        if line.startswith('# item-gate:'):
            match=re.fullmatch(r'# item-gate: (\w+)',line)
            if not match or match[1] not in FORMULAS or match[1] in result:raise ValueError('unknown/duplicate item gate')
            result.append(match[1])
    return result


@lru_cache(maxsize=1)
def source():return oracle.Source()


def formula(name,inputs,actual):
    if name not in FORMULAS:raise ValueError('unknown item formula')
    # Match E1's finite function registry on arbitrary resolution inputs.
    if name=='gear_stats':
        required={'class_file','level','equipment','skill_refs','base_deltas','base_delta_sequence'}
        if set(inputs)!=required:raise ValueError('incomplete gear stat inputs')
        class_file=inputs['class_file']
        if not re.fullmatch(r'src/main/resources/data/stats/chars/baseStats/[A-Za-z]+\.xml',class_file):raise ValueError('invalid pinned class file')
        calculated=oracle.rational(oracle.gear_stats(source(),**oracle.decode_rationals(inputs)))
        same(actual,calculated)
    elif name=='sa_exchange':
        required={'recipe_id','before_instance','after_instance','ingredients_before','ingredients_after','tax_rate'}
        if set(inputs)!=required:raise ValueError('incomplete SA inputs')
        same(inputs['tax_rate'],'0') # D22 fixture policy, not universal source tax.
        recipe_id=integer(inputs['recipe_id'],1);list_id,ordinal=divmod(recipe_id,2**32)
        recipes=source().xml(oracle.D+f'multisell/{list_id}.xml').findall('item')
        if ordinal>=len(recipes):raise ValueError('unknown SA recipe')
        recipe=recipes[ordinal]
        before,after=inputs['before_instance'],inputs['after_instance']
        if set(before)!= {'id','template_id','enchant','augment_id'} or set(after)!=set(before):raise ValueError('incomplete SA instance')
        if integer(after['id'],1)==integer(before['id'],1):raise ValueError('SA must allocate fresh ID (D37)')
        outputs=recipe.findall('production')
        if len(outputs)!=1 or int(outputs[0].get('count'))!=1:raise ValueError('unsupported recipe product')
        same(after['template_id'],int(outputs[0].get('id')))
        # maintainEnchantment is list config, not an ingredient heuristic.
        configuration=source().xml(oracle.D+f'multisell/{list_id}.xml')
        maintain=configuration.get('maintainEnchantment','false')=='true'
        same(after['enchant'],before['enchant'] if maintain else 0)
        same(after['augment_id'],before['augment_id'] if maintain else None)
        if not any(int(r.get('id'))==before['template_id'] and int(r.get('count'))==1 for r in recipe.findall('ingredient')):raise ValueError('wrong base weapon')
        expected=dict(inputs['ingredients_before'])
        for ingredient in recipe.findall('ingredient'):
            template=ingredient.get('id');count=int(ingredient.get('count'))
            if template==str(before['template_id']):
                if count!=1:raise ValueError('unsupported base count')
                continue # Base consumed instance is separate from ingredient stacks.
            if count:
                expected[template]=integer(expected[template])-count
                if expected[template]<0:raise ValueError('insufficient SA ingredient')
        same(inputs['ingredients_after'],expected)
        same(actual,{'product_template':after['template_id'],'new_instance_id':after['id']})
    elif name=='grant_denied':
        if set(inputs)!={'auth_dev_tokens','status','before','after','ledger_rows','rng_before','rng_after'}:raise ValueError('incomplete grant-denial inputs')
        same(inputs['auth_dev_tokens'],False);same(inputs['status'],'FAILED_PRECONDITION')
        same(inputs['before'],inputs['after']);same(inputs['ledger_rows'],0);same(inputs['rng_before'],inputs['rng_after'])
        same(actual,{'unchanged':True})
    else:
        oracle.verify_facts([dict(formula=name,inputs=inputs,expected=actual)])
        function=getattr(oracle,name,None) or getattr(oracle.behavior,name,None)
        same(actual,oracle.rational(function(**oracle.decode_rationals(inputs))))


def validate(rows,recording,rules_hash):
    if not isinstance(rows,list) or len(rows)<3:raise ValueError('manifest, facts and complete trailer required')
    manifest,complete=rows[0],rows[-1]
    fields={'kind','schema_version','scope','recording_sha256','zone','epoch','snapshot_tick','watermark','phase4_rules_hash','snapshot_schema','record_schema','digest_version'}
    if set(manifest)!=fields:raise ValueError('unknown/missing Phase4 manifest fields')
    for key,expected in {'kind':'manifest','schema_version':1,'scope':'whole_recording','recording_sha256':digest(recording),'phase4_rules_hash':rules_hash,'snapshot_schema':10,'record_schema':8,'digest_version':'BinaryV6'}.items():same(manifest[key],expected)
    if not re.fullmatch(r'[0-9a-f]{64}',rules_hash):raise ValueError('runtime Phase4 hash required (not E1 data hash)')
    zone,epoch=integer(manifest['zone'],1),integer(manifest['epoch'],1)
    start=integer(manifest['snapshot_tick']);watermark=manifest['watermark']
    if set(watermark)!= {'zone','epoch','records','last_tick','reason'}:raise ValueError('missing complete epoch watermark')
    ticks=integer(watermark['records'],1);last=start+ticks-1
    for key,value in {'zone':zone,'epoch':epoch,'last_tick':last}.items():same(watermark[key],value)
    if watermark['reason'] not in ('shutdown','epoch_closed'):raise ValueError('incomplete watermark')
    same(complete,dict(kind='complete',facts=len(rows)-2,ticks=ticks,last_tick=last))
    previous=start;seen=set();cursor=None
    for sequence,row in enumerate(rows[1:-1]):
        if set(row)!= {'kind','entity','incarnation','tick','sequence','data'}:raise ValueError('missing/unknown raw item fields')
        same(row['sequence'],sequence);same(str(UUID(row['entity'])),row['entity']);integer(row['incarnation'],1)
        tick=integer(row['tick'])
        if not previous<=tick<=last:raise ValueError('unordered/out-of-scope fact')
        previous=tick;data=row['data']
        if row['kind']=='item_formula':
            if set(data)!= {'formula','inputs','actual','resolution_id','baseline'} or data['formula'] not in FORMULAS:raise ValueError('unknown/malformed formula fact')
            same(data['baseline'],False)
            if not isinstance(data['resolution_id'],str) or not data['resolution_id']:raise ValueError('resolution identity required')
            identity=(row['entity'],row['incarnation'],data['resolution_id'],data['formula'])
            if identity in seen:raise ValueError('repeated formula resolution')
            seen.add(identity)
            if not isinstance(data['inputs'],dict):raise ValueError('actual input object required')
        elif row['kind']=='item_rng':
            if set(data)!= {'callsite','word_before','word_after','bound','value'}:raise ValueError('unknown/missing RNG fields')
            before,after=integer(data['word_before']),integer(data['word_after']);bound=integer(data['bound'],1);value=integer(data['value'])
            if after<=before or cursor is not None and before!=cursor or value>=bound:raise ValueError('invalid RNG cursor/residue')
            cursor=after
            if not isinstance(data['callsite'],str) or not data['callsite']:raise ValueError('RNG callsite required')
        elif row['kind']=='item_delta':
            if set(data)!= {'operation','instance','before','after','count_delta','operation_ordinal','receipt'}:raise ValueError('unknown/missing ledger fields')
            if data['operation'] not in {'grant','starter','backfill','equip','unequip','consume','enchant','destroy','crystallize','refine','cancel_refine','exchange','use'}:raise ValueError('unknown ledger operation')
            instance=integer(data['instance'],1)
            before,after=data['before'],data['after']
            if before==after or before is None and after is None:raise ValueError('ledger must contain actual change (D28)')
            for value in (before,after):
                if value is not None:
                    if value.get('id')!=instance:raise ValueError('ledger identity mismatch')
                    integer(value['count'],1)
            net=(after['count'] if after else 0)-(before['count'] if before else 0)
            same(data['count_delta'],net)
            ordinal=integer(data['operation_ordinal'])
            key=(row['entity'],tick,instance,data['operation'],ordinal)
            if key in seen:raise ValueError('repeated ledger fact')
            seen.add(key)
            if data['operation']=='consume':same(ordinal,0)
        else:raise ValueError('unknown item raw fact kind')
    return manifest


def evaluate(rows,recording,entity,gates,rules_hash):
    manifest=validate(rows,recording,rules_hash)
    results=[]
    owner=[r for r in rows[1:-1] if r['entity']==entity and r['kind']=='item_formula']
    for gate in gates:
        matched=[r for r in owner if r['data']['formula']==gate];errors=[]
        applicable=True
        if gate=='failure_refund' and not matched:
            outcomes=[r for r in owner if r['data']['formula']=='enchant_outcome']
            if outcomes and all(isinstance(r['data']['actual'],dict) and r['data']['actual'].get('destroyed') is False for r in outcomes):
                # Only destruction executes the source refund branch; success/reset/safe do not.
                for outcome in outcomes:formula('enchant_outcome',outcome['data']['inputs'],outcome['data']['actual'])
                applicable=False
        if not matched and applicable:errors.append('missing owner actual-input resolution fact')
        for row in matched:
            try:formula(gate,row['data']['inputs'],row['data']['actual'])
            except (ValueError,TypeError,KeyError,IndexError,ZeroDivisionError) as error:errors.append(str(error))
        results.append(dict(formula=gate,sequences=[r['sequence'] for r in matched],applicable=applicable,errors=errors))
    return dict(schema_version=1,recording_sha256=manifest['recording_sha256'],phase4_rules_hash=rules_hash,
                entity=entity,gates=results,passed=bool(results) and not any(r['errors'] for r in results))


def run(scenario,recording,report,out,replay,rules_hash):
    gates=requirements(scenario)
    if not gates:return 0
    ids=ET.parse(report).getroot().findall(".//property[@name='own_entity_id']")
    if len(ids)!=1:raise ValueError('one current report owner required')
    entity=str(UUID(ids[0].get('value')));raw=out.with_suffix('.raw.jsonl')
    if out.exists() or raw.exists():raise ValueError('fresh evidence destinations required')
    command=shlex.split(replay)
    if not command or 'cargo' in command:raise ValueError('explicit candidate replay binary required; no implicit cargo')
    with out.with_suffix('.log').open('w') as log:
        for args in (['check','--file',str(recording),'--session',entity],
                     ['export','--file',str(recording),'--session',entity,'--items-evidence-out',str(raw)]):
            subprocess.run(command+args,cwd=REPO,stdout=log,stderr=subprocess.STDOUT,check=True)
    result=evaluate(read_jsonl(raw),recording,entity,gates,rules_hash)
    out.write_text(json.dumps(result,sort_keys=True,indent=2)+'\n')
    return int(not result['passed'])


def soak(rows,recording,roles,rules_hash):
    validate(rows,recording,rules_hash)
    if roles.get('items') is not True or roles.get('casting') is not True or len(roles.get('roles',[]))!=8:
        raise ValueError('acceptance soak requires all eight item/casting clients')
    results=[]
    identities=set()
    for role in roles['roles']:
        entity=str(UUID(role['seed']['character_id']))
        if entity in identities:raise ValueError('duplicate soak entity')
        identities.add(entity)
        result=evaluate(rows,recording,entity,['enchant_outcome','shot_payment'],rules_hash)
        owner=[r for r in rows[1:-1] if r['entity']==entity]
        equips=[r for r in owner if r['kind']=='item_delta' and r['data']['operation']=='equip' and r['data']['after'] is not None and r['data']['after'].get('slot')=='rhand']
        if not {1,6} <= {r['data']['after']['template_id'] for r in equips}:
            result['passed']=False;result['gates'].append(dict(formula='equip swaps',errors=['committed sword1 and wand6 equip required']))
        shots=[r for r in owner if r['kind']=='item_formula' and r['data']['formula']=='shot_payment']
        if not any(r['data']['actual'].get('consumed',0)>0 and r['data']['actual'].get('outcome')=='charged' and r['data']['inputs'].get('toggle') is True for r in shots):
            result['passed']=False;result['gates'].append(dict(formula='auto-shot payment',errors=['positive paid auto-shot required']))
        scrolls=[r for r in owner if r['kind']=='item_delta' and r['data']['operation']=='consume' and (r['data']['before'] or {}).get('template_id')==951]
        if sum(r['data']['count_delta'] for r in scrolls)!=-1:
            result['passed']=False;result['gates'].append(dict(formula='enchant scroll',errors=['exactly one consumed C scroll951 required']))
        results.append(result)
    return dict(schema_version=1,clients=results,passed=all(r['passed'] for r in results))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--list',type=Path);parser.add_argument('--count',type=Path)
    for name in ('scenario','recording','report','out'):parser.add_argument('--'+name,type=Path)
    parser.add_argument('--replay');parser.add_argument('--rules-hash')
    parser.add_argument('--soak-manifest',type=Path);parser.add_argument('--raw',type=Path)
    args=parser.parse_args()
    try:
        if args.list or args.count:
            gates=requirements(args.list or args.count);print(len(gates) if args.count else json.dumps(gates));code=0
        elif args.soak_manifest:
            result=soak(read_jsonl(args.raw),args.recording,json.loads(args.soak_manifest.read_text()),args.rules_hash)
            args.out.write_text(json.dumps(result,sort_keys=True,indent=2)+'\n');code=int(not result['passed'])
        else:code=run(args.scenario,args.recording,args.report,args.out,args.replay,args.rules_hash)
    except (ValueError,KeyError,TypeError,OSError,subprocess.SubprocessError) as error:
        print('Phase4 evidence failed: '+str(error),file=sys.stderr);code=2
    sys.exit(code)
