#!/usr/bin/env python3
"""C14 verified-export / C15 actual-input item oracle consumer.

D46 freezes the E2 producer contract; unknown schemas, facts and baseline-only
exports fail closed. Unit fixtures are synthetic, never live replay attestations.
"""
import argparse
from functools import lru_cache
import hashlib
import importlib.util
import json
import inspect
import tomllib
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


def phases(scenario):
    result=[]
    for line in scenario.read_text().splitlines():
        if not line.startswith('# item-phase: '):continue
        value=json.loads(line.removeprefix('# item-phase: '),object_pairs_hook=unique_object,parse_float=no_float)
        if not {'name','formula'}<=set(value) or set(value)-{'name','formula','context','inputs','actual','providers','absent_providers','mutation','equipment'} or value['formula'] not in FORMULAS:
            raise ValueError('invalid item phase specification')
        if any(r['name']==value['name'] for r in result):raise ValueError('duplicate item phase')
        result.append(value)
    return result


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


# E1 item_oracle/item_behavior_oracle signatures and design §12.1 operand families.
# Structured operands use records/maps/tuples; optional means absent, not nullable.
I64_MAX=2**63-1
NONNEG_INT=('nonneg-int',0,I64_MAX)
IDENTITY=('nonneg-int',1,I64_MAX)
SIGNED_INT=('int',-2**63,I64_MAX)
U32=('u32',0,2**32-1)
TEMPLATE=('u32',1,2**32-1)
NUMERIC=('exact-numeric',None,None)
NONNEG_NUMERIC=('exact-numeric',0,None)
POSITIVE_NUMERIC=('exact-numeric',0,None,'positive')
BOOL=('bool',)
TEXT=('string',)


def enum(*values):return ('enum',values)
def nullable(kind):return ('nullable',kind)
def list_of(kind):return ('list',kind)
def tuple_of(*kinds):return ('tuple',kinds)
def record(required,optional=None):return ('record',required,optional or {})
def map_of(key,value):return ('map',key,value)
def int_range(minimum,maximum):return ('nonneg-int',minimum,maximum)


GRADES=enum('NONE','D','C','B','A','S','S80','S84')
CHARGE=enum('soul','spirit','blessed')
PROXIMITY=enum('front','side','back')
SLOTS=set(('under head hair hair2 neck rhand chest lhand rear lear gloves legs feet '
           'rfinger lfinger lbracelet rbracelet deco1 deco2 deco3 deco4 deco5 deco6 cloak belt').split())
SLOT=enum(*sorted(SLOTS))
BODY_PART=enum(*sorted(SLOTS|{'none','lrhand','onepiece','fullarmor','alldress','hairall'}))
WEAPON_TYPE=enum('NONE','SWORD','BLUNT','DAGGER','BOW','CROSSBOW','DUAL','DUALFIST','POLE',
                 'FIST','ETC','BIGSWORD','BIGBLUNT','RAPIER','ANCIENTSWORD','DUALDAGGER')
COUNT_MAP=map_of(('id-key',),NONNEG_INT)
ITEM=record(dict(instance_id=IDENTITY,template_id=TEMPLATE,count=NONNEG_INT),
            dict(enchant=U32,charges=int_range(0,7),grade=GRADES,kind=TEXT,augment_id=nullable(U32)))
WEAPON=record(dict(template_id=TEMPLATE,grade=GRADES,mp_cost=NONNEG_INT,
                   reduced_chance=int_range(0,100),reduced_cost=NONNEG_INT,spread_radius=NONNEG_INT))
CONTEXT=record({},dict(canEscape=BOOL,canResurrect=BOOL,inside_zone_ids=list_of(U32),
    race=enum('HUMAN','ELF','DARK_ELF','ORC','DWARF','KAMAEL'),level=IDENTITY,
    weapon_type=WEAPON_TYPE,armor_type=enum('NONE','LIGHT','HEAVY','MAGIC'),slots=list_of(SLOT),
    sitting=BOOL,moving=BOOL,running=BOOL))
SKILL=record(dict(id=TEMPLATE,rank=TEMPLATE),dict(hit_time_ms=NONNEG_INT,static=BOOL,simultaneous=BOOL,
    reuse_delay_ms=NONNEG_INT,item_consume_id=U32,item_consume_count=NONNEG_INT,
    conditions=list_of(TEXT),effects=list_of(record(dict(name=TEXT,params=map_of(TEXT,TEXT)))),
    source=TEXT,blockers=list_of(TEXT)))
DEFINITION=record(dict(template_id=TEMPLATE,handler=TEXT,default_action=TEXT,etcitem_type=TEXT,
    reuse_delay_ms=NONNEG_INT,reuse_group=NONNEG_INT,immediate=BOOL,ex_immediate=BOOL,skills=list_of(SKILL)))
OFFER=record(dict(offer_id=IDENTITY,target_incarnation=IDENTITY,created_at=NONNEG_INT,
    expires_at=NONNEG_INT,restore_percent=NONNEG_NUMERIC,recoverable_xp=NONNEG_INT))
SA_INSTANCE=record(dict(id=IDENTITY,template_id=TEMPLATE,enchant=U32,augment_id=nullable(U32)))
RETAIL_ROWS=list_of(tuple_of(int_range(1,65535),('exact-numeric',0,100)))

INPUT_SCHEMAS={
    'enchant_increment':dict(stat=enum('pAtk','mAtk','pDef','mDef','sDef'),grade=GRADES,
        bodypart=BODY_PART,weapon_type=WEAPON_TYPE,enchant=U32),
    'enchant_hp':dict(bonuses=list_of(NONNEG_NUMERIC),enchant=U32,onepiece=BOOL),
    'crystal_count':dict(base=NONNEG_INT,bonus=NONNEG_INT,enchant=U32,weapon=BOOL),
    'failure_refund':dict(base=NONNEG_INT,bonus=NONNEG_INT,enchant=U32,weapon=BOOL,crystallizable=BOOL,crystal_id=U32),
    'enchant_chance':dict(base=NONNEG_NUMERIC,bonus=NONNEG_NUMERIC,support=NONNEG_NUMERIC),
    'enchant_outcome':dict(base=NONNEG_NUMERIC,bonus=NONNEG_NUMERIC,support=NONNEG_NUMERIC,
        residue=int_range(0,99999),enchant=U32,blessed=BOOL,safe=BOOL),
    'enchant_valid':dict(current=U32,max_enchant=U32,blessed=BOOL,support=BOOL),
    'max_load':dict(con_bonus=NONNEG_NUMERIC,weight_limit=NONNEG_NUMERIC,multiplier=NONNEG_NUMERIC,add=NUMERIC),
    'overload':dict(weight=NONNEG_INT,maximum=SIGNED_INT,bonus=NONNEG_NUMERIC,
        previous_level=int_range(0,4),previous_overloaded=BOOL),
    'grade_penalties':dict(equipped=list_of(record(dict(instance_id=IDENTITY,grade=int_range(0,7),weapon=BOOL),
        dict(arrow=BOOL,slot=NONNEG_INT))),expertise=NONNEG_INT,bonus=SIGNED_INT),
    'fist_spread':dict(level=IDENTITY,residue=nullable(NONNEG_INT),template_radius=nullable(NONNEG_INT)),
    'pole_damage':dict(formula=NUMERIC,percent=NONNEG_NUMERIC,primary=BOOL),
    'pole_sequence':dict(candidates=list_of(record(dict(eligible=BOOL,hit=BOOL,formula=NUMERIC))),maximum=NONNEG_INT),
    'bow_reuse':dict(reuse_delay=NONNEG_INT,attack_speed=POSITIVE_NUMERIC,weapon_reuse_modifier=NONNEG_NUMERIC),
    'bow_damage':dict(formula=NUMERIC,dx_milli=SIGNED_INT,dy_milli=SIGNED_INT),
    'dual_damage':dict(formula=NUMERIC,miss=BOOL),
    'dual_timing':dict(attack_speed=POSITIVE_NUMERIC),
    'skill_caller_damage':dict(formula=NUMERIC,critical=BOOL),
    'position':dict(facing=tuple_of(SIGNED_INT,SIGNED_INT),relative=tuple_of(SIGNED_INT,SIGNED_INT)),
    'shield':dict(rate=NONNEG_NUMERIC,dex_bonus=NONNEG_NUMERIC,bow=BOOL,eligible=BOOL,facing=BOOL,
        sigil=BOOL,perfect_residue=nullable(int_range(0,99)),normal_residue=nullable(int_range(0,99))),
    'physical_damage':dict(p_atk=NONNEG_NUMERIC,defence=NONNEG_NUMERIC,power=NUMERIC,
        caller=enum('auto','physical','blow'),shot=BOOL,proximity=PROXIMITY,shield_def=NONNEG_NUMERIC,
        perfect=BOOL,critical=BOOL,crit_dmg=NONNEG_NUMERIC,crit_pos=NONNEG_NUMERIC,
        def_crit_dmg=NONNEG_NUMERIC,crit_add=NUMERIC,def_crit_add=NUMERIC,trait=NONNEG_NUMERIC,
        spread=NONNEG_NUMERIC,pvp=NONNEG_NUMERIC,attribute=NONNEG_NUMERIC,npc=NONNEG_NUMERIC),
    'magic_damage':dict(power=NONNEG_NUMERIC,m_atk=NONNEG_NUMERIC,m_def=NONNEG_NUMERIC,spirit=BOOL,
        blessed=BOOL,shield_def=NONNEG_NUMERIC,perfect=BOOL,critical=BOOL,attacker_player=BOOL,
        target_player=BOOL,is_pvp=nullable(BOOL),skill_is_magic=BOOL,pvp_magic_def=NONNEG_NUMERIC,
        pvp_skill_def=NONNEG_NUMERIC,magic_failures=BOOL,first_success=BOOL,second_success=nullable(BOOL),
        level_difference=SIGNED_INT,magic_crit_damage=NONNEG_NUMERIC,spread=NONNEG_NUMERIC,
        pvp_magic_damage=NONNEG_NUMERIC,pvp_skill_damage=NONNEG_NUMERIC,attribute=NONNEG_NUMERIC,npc=NONNEG_NUMERIC),
    'cast_time':dict(hit_time=NONNEG_INT,casting_speed=POSITIVE_NUMERIC,spirit=BOOL,blessed=BOOL,static=BOOL,is_magic=BOOL),
    'heal':dict(power=NUMERIC,m_atk=NONNEG_NUMERIC,mp_consume2=NONNEG_NUMERIC,mage=BOOL,npc=BOOL,
        grade=GRADES,spirit=BOOL,blessed=BOOL,static=BOOL,heal_effect=NONNEG_NUMERIC,critical=BOOL,is_magic=BOOL,
        current_hp=NONNEG_INT,recoverable_hp=nullable(NONNEG_NUMERIC),remainder_q=int_range(0,oracle.Q-1)),
    'blow_success':dict(chance=NONNEG_NUMERIC,dex=NONNEG_NUMERIC,proximity=PROXIMITY,
        residue=int_range(0,999),modifier=NONNEG_NUMERIC),
    # D81: a residue is null iff its draw was not taken; 999 is a real residue, never a sentinel.
    'lethal':dict(full=NONNEG_NUMERIC,half=NONNEG_NUMERIC,full_residue=nullable(int_range(0,999)),
        half_residue=nullable(int_range(0,999)),eligible=BOOL,attribute=NONNEG_NUMERIC,trait=NONNEG_NUMERIC),
    'retail_select':dict(rows=RETAIL_ROWS,residue=int_range(0,9999)),
    'retail_distribution':dict(rows=RETAIL_ROWS),
    'category':dict(residue=int_range(0,9999),accessory=BOOL),
    'pack_options':dict(first=int_range(1,65535),second=int_range(1,65535)),
    'cancel_fee':dict(grade=enum('C','B','A','S','S80','S84'),crystals=NONNEG_INT),
    'set_shield_history':dict(previous=BOOL,mutation=enum('remove','equip_shield','equip_chest'),
        chest_present=BOOL,matching_shield=BOOL,complete=BOOL,required_part=BOOL),
    'clamp_no_heal':dict(current=NONNEG_NUMERIC,new_max=NONNEG_NUMERIC),
    'charge_transition':dict(charges=list_of(CHARGE),use_spirit=BOOL,use_soul=BOOL,landed=BOOL,unequip=BOOL),
    'shot_payment':dict(charges=list_of(CHARGE),kind=CHARGE,count=NONNEG_INT,available=NONNEG_INT,
        weapon_grade=nullable(GRADES),shot_grade=GRADES,toggle=BOOL,
        reduction=('list-tuple',tuple_of(int_range(0,100),NONNEG_INT)),residue=nullable(int_range(0,99))),
    'shared_shot_bundle':dict(charges=list_of(CHARGE),hits=list_of(BOOL),use_spirit=BOOL,use_soul=BOOL,canceled=BOOL,unequip=BOOL),
    'winning_providers':dict(providers=list_of(record(dict(key=TEXT,skill_id=TEMPLATE,rank=TEMPLATE,sequence=NONNEG_INT)))),
    'consumable_use':dict(definition=DEFINITION,tick=NONNEG_INT,available=COUNT_MAP,
        item_deadlines=nullable(COUNT_MAP),skill_deadlines=nullable(COUNT_MAP),main_casting=BOOL,condition_context=nullable(CONTEXT)),
    'consumable_cast_sequence':dict(starts=list_of(record(dict(definition=DEFINITION,tick=NONNEG_INT,
        cast_id=IDENTITY),dict(available=COUNT_MAP,condition_context=nullable(CONTEXT)))),caster_ordinal=NONNEG_INT,caster_id=IDENTITY),
    'player_level_range':dict(condition=record(dict(kind=enum('player_level_range'),
        ranges=list_of(record(dict(minimum=NONNEG_INT,maximum=NONNEG_INT))))),rank=TEMPLATE,level=IDENTITY),
    'bow_launch':dict(weapon=WEAPON,inventory=list_of(ITEM),lhand=nullable(IDENTITY),mp=NONNEG_NUMERIC,
        mp_modifier=NONNEG_NUMERIC,reduction_residue=nullable(int_range(0,99)),miss=BOOL,
        reuse_remaining=NONNEG_INT,hit_chance=int_range(0,1000),hit_residue=nullable(int_range(0,999))),
    'bow_refill':dict(weapon=WEAPON,arrow=ITEM,mp=NONNEG_NUMERIC),
    'resurrection_scroll':dict(definition=DEFINITION,start_tick=NONNEG_INT,available=NONNEG_INT,
        death_incarnation=IDENTITY,current_incarnation=IDENTITY,dead=BOOL,
        response=enum('offer','accept','decline','expire'),response_tick=NONNEG_INT,response_offer_id=nullable(IDENTITY),
        existing_offer=nullable(OFFER),repeat=BOOL,hp=NONNEG_NUMERIC,max_hp=NONNEG_NUMERIC,mp=NONNEG_NUMERIC,
        cp=NONNEG_NUMERIC,recoverable_xp=NONNEG_INT,restore_percent=NONNEG_NUMERIC,restore_hp=('exact-numeric',0,1)),
    'dormant_use':dict(template_id=TEMPLATE,properties=map_of(TEXT,TEXT),inventory=list_of(ITEM),
        selected_instance=IDENTITY,paperdoll=map_of(SLOT,IDENTITY),allocator_next=IDENTITY,reuse=COUNT_MAP),
    'learned_item_cost':dict(skill_id=TEMPLATE,rank=TEMPLATE,consume_id=U32,consume_count=NONNEG_INT,
        inventory=list_of(ITEM),phase=enum('complete','cancel'),repeat=BOOL),
    'escape_timing':dict(definition=DEFINITION,start_tick=NONNEG_INT,casting_speed=POSITIVE_NUMERIC,
        available=NONNEG_INT,resources=record(dict(hp=NONNEG_NUMERIC,mp=NONNEG_NUMERIC,cp=NONNEG_NUMERIC,xp=NONNEG_INT)),incarnation=IDENTITY),
    'dual_plan':dict(hits=tuple_of(BOOL,BOOL),formulas=tuple_of(NUMERIC,NUMERIC),second_valid=BOOL,
        hit_residues=tuple_of(int_range(0,999),int_range(0,999)),hit_chance=int_range(0,1000),spread_radius=NONNEG_INT),
    'enchant_safe':dict(current=U32,fullarmor=BOOL,safe_enchant=U32,safe_fullarmor=U32,
        base=NONNEG_NUMERIC,residue=int_range(0,99999)),
    'quantized_enchant':dict(base=NONNEG_NUMERIC,bonus=NONNEG_NUMERIC,support=NONNEG_NUMERIC),
    'augmentation_provider':dict(option_id=IDENTITY,option_xml=TEXT,
        event=enum('install','death','dead_admission','resurrection','unequip','cancel'),other_provider=nullable(IDENTITY),
        hp=NONNEG_NUMERIC,maximum=NONNEG_NUMERIC,stat_baseline=map_of(TEXT,NUMERIC)),
    'retail_draw_contract':dict(accessory=BOOL),
    'item_regen_status':dict(base=NONNEG_NUMERIC,level_modifier=NONNEG_NUMERIC,stat_bonus=NONNEG_NUMERIC,
        posture_modifier=NONNEG_NUMERIC,regen_modifier=NONNEG_NUMERIC,periodic_credit=NONNEG_NUMERIC,
        current=NONNEG_INT,maximum=NONNEG_INT,remainder_q=int_range(0,oracle.Q-1),dead=BOOL,stunned=BOOL,invulnerable=BOOL,hp_blocked=BOOL),
    'gear_stats':dict(class_file=TEXT,level=IDENTITY,equipment=list_of(tuple_of(TEMPLATE,U32,SLOT,NONNEG_INT)),
        skill_refs=list_of(tuple_of(TEMPLATE,TEMPLATE,NONNEG_INT)),base_deltas=nullable(map_of(enum('str','dex','con','int','wit','men'),SIGNED_INT)),
        base_delta_sequence=('int',-1,I64_MAX)),
    'sa_exchange':dict(recipe_id=IDENTITY,before_instance=SA_INSTANCE,after_instance=SA_INSTANCE,
        ingredients_before=COUNT_MAP,ingredients_after=COUNT_MAP,tax_rate=enum('0')),
    'grant_denied':dict(auth_dev_tokens=BOOL,status=enum('FAILED_PRECONDITION'),before=list_of(IDENTITY),
        after=list_of(IDENTITY),ledger_rows=NONNEG_INT,rng_before=NONNEG_INT,rng_after=NONNEG_INT),
}


# D80: evidence JSON encodes every integer outside ±2^53 as a canonical decimal
# string; integers inside that range stay JSON numbers. The consumer parses both
# forms for out-of-range integers, schema-directed, so text fields never convert.
SAFE_INTEGER=2**53
DECIMAL=re.compile(r'-?(?:0|[1-9][0-9]*)')
INSTANCE_INTEGERS=frozenset(('id','template_id','count','enchant','augment_id','charges','created_tick','revision'))


def wide(value):
    """One integer position: a decimal string must lie outside ±2^53 (D80)."""
    if type(value) is str and DECIMAL.fullmatch(value):
        number=int(value)
        if -SAFE_INTEGER<number<SAFE_INTEGER:raise ValueError('integer inside ±2^53 must be a JSON number (D80)')
        return number
    return value


def wide_any(value):
    """Schema-free formula outputs: only out-of-range canonical decimal strings convert."""
    if isinstance(value,dict):return {key:wide_any(child) for key,child in value.items()}
    if isinstance(value,list):return [wide_any(child) for child in value]
    if type(value) is str and DECIMAL.fullmatch(value) and not -SAFE_INTEGER<int(value)<SAFE_INTEGER:return int(value)
    return value


def wide_input(value,schema):
    """Schema-directed D80 decode; shapes that do not fit, including in-range decimal
    strings, are left unchanged so the input schema refuses them per formula gate."""
    kind=schema[0]
    if kind=='nullable':return value if value is None else wide_input(value,schema[1])
    if kind=='list' and type(value) is list:return [wide_input(child,schema[1]) for child in value]
    if kind=='list-tuple' and type(value) is list and value:return wide_input(value,schema[1])
    if kind=='tuple' and type(value) is list and len(value)==len(schema[1]):
        return [wide_input(child,child_schema) for child,child_schema in zip(value,schema[1])]
    if kind=='record' and type(value) is dict:
        kinds=schema[1]|schema[2]
        return {key:wide_input(child,kinds[key]) if key in kinds else child for key,child in value.items()}
    if kind=='map' and type(value) is dict:return {key:wide_input(child,schema[2]) for key,child in value.items()}
    if kind in ('int','nonneg-int','u32'):return wide_any(value)
    if kind=='exact-numeric' and type(value) is dict and set(value)=={'numerator','denominator'}:
        return {key:wide_any(child) for key,child in value.items()}
    if kind=='any':return wide_any(value)
    return value


def wide_instance(state):
    if not isinstance(state,dict):return state
    return {key:wide(child) if key in INSTANCE_INTEGERS else child for key,child in state.items()}


def wide_fields(mapping,keys):
    return {key:wide(child) if key in keys else child for key,child in mapping.items()}


def decode_rows(rows):
    """D80 decode of a whole export into a fresh list; malformed shapes pass through unchanged."""
    if not isinstance(rows,list):return rows
    result=[]
    for row in rows:
        if not isinstance(row,dict):result.append(row);continue
        kind=row.get('kind')
        if kind=='manifest':
            row=wide_fields(row,{'schema_version','zone','epoch','snapshot_tick','snapshot_schema','record_schema'})
            if isinstance(row.get('watermark'),dict):row['watermark']=wide_fields(row['watermark'],{'zone','epoch','records','last_tick'})
        elif kind=='complete':row=wide_fields(row,{'facts','ticks','last_tick'})
        else:
            row=wide_fields(row,{'incarnation','tick','sequence'});data=row.get('data')
            if isinstance(data,dict):row['data']=decode_data(kind,data)
        result.append(row)
    return result


def decode_data(kind,data):
    data=dict(data)
    if kind=='item_rng':return wide_fields(data,{'word_before','word_after','bound','value'})
    if kind=='item_delta':
        data=wide_fields(data,{'instance','count_delta','operation_ordinal'})
        for key in ('before','after'):
            if key in data:data[key]=wide_instance(data[key])
        if isinstance(data.get('causes'),list):
            data['causes']=[wide_fields(cause,{'count_delta'}) if isinstance(cause,dict) else cause for cause in data['causes']]
        return data
    if kind=='item_baseline':
        data=wide_fields(data,{'inventory_revision'})
        if isinstance(data.get('instances'),list):data['instances']=[wide_instance(state) for state in data['instances']]
        if isinstance(data.get('paperdoll'),dict):data['paperdoll']={slot:wide(identity) for slot,identity in data['paperdoll'].items()}
        if isinstance(data.get('auto_shots'),list):data['auto_shots']=[wide(template) for template in data['auto_shots']]
        return data
    if kind=='item_formula':
        name=data.get('formula')
        if name in INPUT_SCHEMAS and isinstance(data.get('inputs'),dict):
            data['inputs']=wide_input(data['inputs'],record({},INPUT_SCHEMAS[name]))
        if 'actual' in data:data['actual']=wide_any(data['actual'])
        if isinstance(data.get('context'),dict):data['context']=wide_fields(data['context'],{'weapon_template','skill_id','set_id'})
        if isinstance(data.get('operation'),dict):
            data['operation']={key:wide_instance(child) if key.endswith('_before') else child for key,child in data['operation'].items()}
    return data


def validate_input(value,schema,path,bounds=False):
    kind=schema[0]
    def invalid():raise ValueError('invalid formula input '+path+' ('+kind+')')
    if kind=='nullable':
        if value is not None:validate_input(value,schema[1],path,bounds)
    elif kind in ('list','tuple','list-tuple'):
        if type(value) is not list:invalid()
        if kind=='list':
            for index,child in enumerate(value):validate_input(child,schema[1],f'{path}[{index}]',bounds)
        elif kind=='list-tuple':
            if value:validate_input(value,schema[1],path,bounds)
        else:
            if len(value)!=len(schema[1]):invalid()
            for index,(child,child_schema) in enumerate(zip(value,schema[1])):
                validate_input(child,child_schema,f'{path}[{index}]',bounds)
    elif kind=='record':
        required,optional=schema[1:]
        if type(value) is not dict or not set(required)<=set(value) or set(value)-set(required)-set(optional):invalid()
        for key,child in value.items():validate_input(child,(required|optional)[key],path+'.'+key,bounds)
    elif kind=='map':
        if type(value) is not dict:invalid()
        for key,child in value.items():
            validate_input(key,schema[1],path+'.key',bounds)
            validate_input(child,schema[2],path+'['+str(key)+']',bounds)
    elif kind=='any':pass # Structure is pinned by exact oracle equality.
    elif kind=='bool':
        if type(value) is not bool:invalid()
    elif kind=='string':
        if type(value) is not str:invalid()
    elif kind=='enum':
        if type(value) is not str or value not in schema[1]:invalid()
    elif kind=='id-key':
        if not (type(value) is int or type(value) is str and re.fullmatch(r'[1-9][0-9]*',value)):invalid()
        if bounds and not 1<=int(value)<=2**32-1:invalid()
    elif kind in ('int','nonneg-int','u32'):
        if type(value) is not int:invalid()
        if bounds and not schema[1]<=value<=schema[2]:invalid()
    elif kind=='exact-numeric':
        if type(value) is dict:
            if set(value)!={'numerator','denominator'} or any(type(v) is not int for v in value.values()):invalid()
            if value['denominator']<=0:invalid()
            if bounds and (not -2**63<=value['numerator']<=I64_MAX or value['denominator']>I64_MAX):invalid()
            number=oracle.F(value['numerator'],value['denominator'])
        elif type(value) in (int,str,oracle.F):
            try:number=oracle.exact(value)
            except (ValueError,TypeError,ZeroDivisionError):invalid()
        else:invalid()
        if bounds and (not -2**63<=number.numerator<=I64_MAX or not 1<=number.denominator<=I64_MAX):invalid()
        if bounds and (schema[1] is not None and number<schema[1] or
                       schema[2] is not None and number>schema[2] or len(schema)>3 and number==0):invalid()
    else:raise ValueError('unknown input schema kind: '+kind)


def input_domains(name,inputs):
    if name not in INPUT_SCHEMAS or type(inputs) is not dict:raise ValueError('unknown formula input schema')
    function=getattr(oracle,name,None) or getattr(oracle.behavior,name,None)
    if function:
        parameters=inspect.signature(function).parameters
        required={key for key,p in parameters.items() if key!='src' and p.default is inspect.Parameter.empty}
    else:required=set(INPUT_SCHEMAS[name])
    schema=record({k:INPUT_SCHEMAS[name][k] for k in required},
                  {k:v for k,v in INPUT_SCHEMAS[name].items() if k not in required})
    # Complete shape validation precedes bounds and every oracle branch.
    validate_input(inputs,schema,name)
    validate_input(inputs,schema,name,bounds=True)


@lru_cache(maxsize=1)
def item_tables():
    directory=REPO/'packages/data/tables-phase4'
    return {r['template_id']:r for name in ('weapons','armors','etc_items')
            for r in tomllib.loads((directory/(name+'.toml')).read_text())['items']}


@lru_cache(maxsize=None)
def table(name):
    return tomllib.loads((REPO/'packages/data/tables-phase4'/(name+'.toml')).read_text())


@lru_cache(maxsize=1)
def shot_templates():
    admitted={r['template_id'] for r in table('capabilities')['items'] if r['admitted']}
    return {identity for identity,item in item_tables().items() if identity in admitted
            and item['etcitem_type']=='SHOT' and item['handler'] in ('SoulShots','SpiritShot','BlessedSpiritShot')}


@lru_cache(maxsize=1)
def slot_masks():
    # GAME@abfde049 EnchantRateItem.java:71-77; ItemTable.java:74-109 aliases.
    constants={name:int(value,0) for name,value in re.findall(
        r'public static final int (SLOT_\w+) = (-?0x[0-9A-Fa-f]+|-?\d+);',source().read(oracle.G+'model/items/L2Item.java',True))}
    slots={}
    for name,expression in re.findall(r'SLOTS.put\("([^"]+)", (.+?)\);',source().read(oracle.G+'datatables/ItemTable.java',True)):
        slots[name]=0
        for part in expression.split('|'):slots[name]|=constants[part.strip().removeprefix('L2Item.')]
    return slots


def enchant_source_inputs(target,scroll,support):
    # DP@3ca488dd enchantItemData/Groups.xml; GAME@abfde049
    # AbstractEnchantItem.java:111-145, EnchantScroll.java:100-157, L2Item.java:539-540.
    definitions=table('enchant_scrolls');item=item_tables()[target['template_id']]
    if not item['enchant_enabled'] or target['template_id'] in definitions['blacklist'] or item['kind'] not in ('Weapon','Armor'):
        raise ValueError('ineligible enchant target')
    weapon=item['kind']=='Weapon';grade='S' if item['grade'] in ('S80','S84') else item['grade']
    def resolve(instance,kind):
        matches=[r for r in definitions[kind] if r['template_id']==instance['template_id']]
        if len(matches)!=1:raise ValueError('unknown enchant '+kind+' template')
        definition=matches[0]
        if definition['weapon']!=weapon or definition['target_grade']!=grade or (definition['max_enchant'] and target['enchant']>definition['max_enchant']):
            raise ValueError('enchant grade/type/level mismatch')
        if definition['allowed_items'] and target['template_id'] not in definition['allowed_items']:
            raise ValueError('enchant target outside allowed items')
        return definition
    definition=resolve(scroll,'scrolls');support_rate=oracle.F(0)
    if support is not None:
        if definition['blessed']:raise ValueError('blessed scroll forbids support')
        support_rate=oracle.F(resolve(support,'supports')['bonus_rate'])
    groups=table('enchant_groups')
    binding=next((r['raw'] for r in groups['bindings'] if r['id']==definition['group_id']),None)
    if binding is None:raise ValueError('unknown enchant scroll group')
    group=None
    for rate in binding['children']:
        mask=0;magic=None;item_id=0
        for candidate in rate['children']:
            attributes=candidate['attributes']
            if 'slot' in attributes:mask|=slot_masks()[attributes['slot']]
            if 'magicWeapon' in attributes:magic=attributes['magicWeapon']=='true'
            if 'id' in attributes:item_id=int(attributes['id'])
        if (not item_id or item_id==target['template_id']) and (not mask or mask&item['bodypart_mask']) and (magic is None or magic==item['flags']['is_magic_weapon']):
            group=rate['attributes']['group'];break
    if group is None:raise ValueError('missing enchant item group')
    ranges=next(r['ranges'] for r in groups['rates'] if r['name']==group)
    # EnchantItemGroup.java:64-77 falls back to its final range when unmatched.
    chance=next((r['chance'] for r in ranges if r['min']<=target['enchant']<=r['max']),ranges[-1]['chance'])
    return dict(base=oracle.F(chance),bonus=oracle.F(definition['bonus_rate']),support=support_rate,
                blessed=definition['blessed'],safe=definition['safe'])


def formula(name,inputs,actual):
    if name not in FORMULAS:raise ValueError('unknown item formula')
    if name in INPUT_SCHEMAS and isinstance(inputs,dict):inputs=wide_input(inputs,record({},INPUT_SCHEMAS[name])) # D80
    actual=wide_any(actual)
    input_domains(name,inputs)
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
        recipe=sa_recipe(integer(inputs['recipe_id'],1))
        before,after=inputs['before_instance'],inputs['after_instance']
        if set(before)!= {'id','template_id','enchant','augment_id'} or set(after)!=set(before):raise ValueError('incomplete SA instance')
        if integer(after['id'],1)==integer(before['id'],1):raise ValueError('SA must allocate fresh ID (D37)')
        outputs=recipe['products']
        if len(outputs)!=1 or outputs[0]['count']!=1:raise ValueError('unsupported recipe product')
        same(after['template_id'],outputs[0]['template_id'])
        # maintainEnchantment is list config, not an ingredient heuristic.
        maintain=recipe['maintain_enchantment']
        same(after['enchant'],before['enchant'] if maintain else 0)
        same(after['augment_id'],before['augment_id'] if maintain else None)
        same(before['template_id'],recipe['base_template'])
        expected=dict(inputs['ingredients_before'])
        for ingredient in recipe['ingredients']:
            template=str(ingredient['template_id']);count=ingredient['count']
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
        if name=='lethal':lethal_residues(inputs)
        oracle.verify_facts([dict(formula=name,inputs=inputs,expected=actual)])
        function=getattr(oracle,name,None) or getattr(oracle.behavior,name,None)
        same(actual,oracle.rational(function(**oracle.decode_rationals(inputs))))


def lethal_residues(inputs):
    """D81: each residue is null iff the corresponding draw was not taken (Formulas.java
    lethal: the full draw only when eligible, the half draw only after a missed full)."""
    probe=dict(oracle.decode_rationals(inputs))
    for key in ('full_residue','half_residue'):
        if probe.get(key) is None:probe[key]=0 # placeholder: only decides which draws occur
    sites={draw['site'] for draw in oracle.lethal(**probe)['draws']}
    for key,site in (('full_residue','lethal_full'),('half_residue','lethal_half')):
        if (inputs.get(key) is None)!=(site not in sites):
            raise ValueError('lethal '+key+' must be null iff its draw was not taken (D81)')


@lru_cache(maxsize=None)
def sa_recipe(recipe_id):
    """Use the E1 recipe, cross-checked against its pinned multisell source."""
    recipe=next((r for r in table('sa_variants')['recipes'] if r['recipe_id']==recipe_id),None)
    if recipe is None:raise ValueError('unknown SA recipe')
    list_id,ordinal=divmod(recipe_id,2**32)
    same((recipe['list_id'],recipe['xml_ordinal']),(list_id,ordinal))
    configuration=source().xml(oracle.D+f'multisell/{list_id}.xml')
    raw=configuration.findall('item')[ordinal]
    for field,tag in (('ingredients','ingredient'),('products','production')):
        same(recipe[field],[dict(template_id=int(r.get('id')),count=int(r.get('count'))) for r in raw.findall(tag)])
    same(recipe['maintain_enchantment'],configuration.get('maintainEnchantment','false')=='true')
    return recipe


@lru_cache(maxsize=1)
def count_caps():
    """D68 caps read from the pinned sources at use (GAME@abfde049).

    L2ItemInstance.java:422 caps a stack at MaxAdena for ADENA_ID and at
    Integer.MAX_VALUE otherwise; Inventory.java:51 ADENA_ID = 57;
    character.properties:671 MaxAdena = 99900000000. Non-stackable instances
    (E1 items `stackable=false`) are single items, cap 1.
    """
    properties=source().read('src/main/resources/config/character.properties',True)
    instance=source().read(oracle.G+'model/items/instance/L2ItemInstance.java',True)
    inventory=source().read(oracle.G+'model/itemcontainer/Inventory.java',True)
    adena=re.search(r'^MaxAdena = (\d+)\s*$',properties,re.M)
    adena_id=re.search(r'public static final int ADENA_ID = (\d+);',inventory)
    if not adena or not adena_id or 'long max = getId() == ADENA_ID ? character().getMaxAdena() : Integer.MAX_VALUE;' not in instance:
        raise ValueError('pinned D68 count-cap source changed')
    return int(adena_id[1]),int(adena[1]),2**31-1


def count_cap(template):
    """D68: adena 99,900,000,000; other stackables Integer.MAX_VALUE; non-stackables 1."""
    adena_id,adena,stack=count_caps()
    if template==adena_id:return adena
    return stack if item_tables()[template]['stackable'] is True else 1


def validate_instance(value,cap=True):
    fields={'id','template_id','count','location','slot','enchant','augment_id','charges','created_tick','revision'}
    if not isinstance(value,dict) or set(value)!=fields:raise ValueError('missing/unknown canonical item scalar')
    for name in ('id','template_id','count'):integer(value[name],1)
    for name in ('enchant','charges','created_tick','revision'):integer(value[name])
    if value['id']>=2**63 or value['count']>=2**63 or value['template_id']>=2**31 or value['enchant']>=2**32 or value['charges']>7 or value['created_tick']>=2**63 or value['revision']>=2**63:raise ValueError('item scalar outside domain width')
    # Dormant starter items remain canonical catalogue members (D17).
    if value['template_id'] not in item_tables():raise ValueError('unknown canonical template')
    # D68/D74: baselines and live bindings are capped here; ledger endpoints are capped per chain.
    if cap and value['count']>count_cap(value['template_id']):raise ValueError('item count above pinned template cap (D68)')
    if value['location'] not in ('inventory','paperdoll') or (value['location']=='paperdoll')!=(value['slot'] is not None):raise ValueError('item location/slot mismatch')
    if value['slot'] is not None and value['slot'] not in SLOTS:raise ValueError('unknown canonical slot')
    if value['charges'] and value['slot']!='rhand':raise ValueError('charge on nonweapon slot')
    if value['augment_id'] is not None:
        augment=integer(value['augment_id'],1)
        if augment>=2**32 or augment>>16==0 or augment&65535==0:raise ValueError('invalid packed option pair')


# Revision is checked separately against the single owner/tick step (D55).
ITEM_TRANSITIONS={
    'consume':('consume',frozenset({'count'})),
    'equip':('retain',frozenset({'slot','location'})),
    'unequip':('retain',frozenset({'slot','location'})),
    'enchant':('retain',frozenset({'enchant'})),
    'refine':('retain',frozenset({'augment_id'})),
    'cancel_refine':('retain',frozenset({'augment_id'})),
    # D73: a shot discharge (or any use that changes only `charges`) is count-neutral.
    'use':('retain',frozenset({'charges'})),
    'destroy':('remove',frozenset()),
    'crystallize':('remove',frozenset()),
    'grant':('positive',frozenset({'count'})),
    'starter':('create',frozenset()),
    'backfill':('create',frozenset()),
}


PRE_CONSUME=frozenset(('grant','starter','backfill','equip','unequip','enchant','refine','cancel_refine','use'))
CAUSE=re.compile(r'(?:auto_shot|arrow|item_cast):\S+|receipt:[^:\s]+:\S+')


def validate_causes(causes,negative):
    """D62/D63 ordered `{cause,count_delta}` list; D73 use causes are count-neutral (0)."""
    if not isinstance(causes,list) or not causes:raise ValueError('ordered nonempty causes required')
    for cause in causes:
        if not isinstance(cause,dict) or set(cause)!={'cause','count_delta'}:raise ValueError('complete cause required')
        if not isinstance(cause['cause'],str) or not CAUSE.fullmatch(cause['cause']):raise ValueError('unknown/malformed cause')
        delta=cause['count_delta']
        if negative and (type(delta) is not int or not -(2**63)<delta<0):raise ValueError('negative signed-bigint consume cause delta required')
        if not negative and (type(delta) is not int or delta!=0):raise ValueError('use cause must be count-neutral (D73)')
    return [c['cause'] for c in causes if c['cause'].startswith('receipt:')]


def validate_transition(data):
    operation=data['operation'];shape,allowed=ITEM_TRANSITIONS[operation]
    before,after=data['before'],data['after']
    delta=data['count_delta']
    if type(delta) is not int or not -(2**63)<delta<2**63:raise ValueError('signed-bigint count_delta required')
    same(delta,(after['count'] if after else 0)-(before['count'] if before else 0))
    if shape=='create':
        if before is not None or after is None:raise ValueError(operation+' must create a canonical instance')
    elif shape=='positive':
        if after is None or delta<=0:raise ValueError('grant must add a positive count to a canonical instance')
    elif shape=='remove':
        if before is None or after is not None:raise ValueError(operation+' must remove the instance')
    elif shape=='consume':
        if before is None or delta>=0:raise ValueError('consume must debit an existing instance')
        remaining=before['count']+delta
        if remaining<0 or (after is None)!=(remaining==0):raise ValueError('consume endpoint differs from remaining count')
    elif before is None or after is None:raise ValueError(operation+' must retain the instance')
    if before is not None and after is not None:
        changed={key for key in before if key!='revision' and before[key]!=after[key]}
        if not changed or changed-allowed:raise ValueError(operation+' changed forbidden canonical fields')
    if operation=='equip' and after['location']!='paperdoll':raise ValueError('equip must enter paperdoll')
    if operation=='unequip' and (before['location']!='paperdoll' or after['location']!='inventory'):raise ValueError('unequip must leave paperdoll')
    if operation=='refine' and after['augment_id'] is None:raise ValueError('refine must install augmentation')
    if operation=='cancel_refine' and (before['augment_id'] is None or after['augment_id'] is not None):raise ValueError('cancel_refine must remove augmentation')


def validate_chain_caps(deltas):
    """D74 (amends D68): caps hold at the tick-start/tick-end endpoints of each
    (owner, incarnation, tick, instance) chain; under the D69 export order an
    earlier row's `after` may exceed the cap only by the same-tick consumption
    that follows it: after.count - later consumption <= cap."""
    first=deltas[0]['before']
    if first is not None and first['count']>count_cap(first['template_id']):raise ValueError('item count above pinned template cap (D68)')
    for index,delta in enumerate(deltas):
        after=delta['after']
        if after is None:continue
        later=-sum(d['count_delta'] for d in deltas[index+1:] if d['operation']=='consume')
        if after['count']-later>count_cap(after['template_id']):raise ValueError('item count above pinned template cap (D68/D74)')


def validate_exchanges(rows):
    """D11/D22/D52: bind every receipt's constituents to its source recipe."""
    exchanges={};resolutions={};bound=set()
    for row in rows[1:-1]:
        if row['kind']=='item_formula' and row['data']['formula']=='sa_exchange':
            data=row['data'];formula('sa_exchange',data['inputs'],data['actual'])
            resolutions[(row['entity'],row['incarnation'],data['resolution_id'])]=row
    for row in rows[1:-1]:
        if row['kind']!='item_delta':continue
        data=row['data']
        refs=data['receipt_refs'] if data['operation']=='consume' else [data['receipt_ref']]
        for ref in refs:
            if ref is None or not ref.startswith('receipt:exchange:'):continue
            if not re.fullmatch(r'receipt:exchange:\S+',ref):raise ValueError('invalid Exchange receipt')
            facts=exchanges.setdefault((row['entity'],row['incarnation'],ref),[])
            if row not in facts:facts.append(row)
    for (entity,incarnation,ref),facts in exchanges.items():
        consumes=[r for r in facts if r['data']['operation']=='consume']
        grants=[r for r in facts if r['data']['operation']=='grant']
        if len(grants)!=1 or not consumes or len(consumes)+len(grants)!=len(facts):raise ValueError('Exchange requires consume and one grant row')
        grant=grants[0]
        if grant['data']['before'] is not None:raise ValueError('Exchange must create a fresh product')
        identity=(entity,incarnation,grant['data']['resolution_id'])
        resolution=resolutions.get(identity)
        if resolution is None or identity in bound:raise ValueError('Exchange requires one bound source resolution')
        bound.add(identity)
        if resolution['tick']!=grant['tick']:raise ValueError('Exchange resolution differs from constituent tick')
        inputs=resolution['data']['inputs'];recipe=sa_recipe(inputs['recipe_id'])
        base=inputs['before_instance'];product=inputs['after_instance']
        bases=[r['data'] for r in consumes if r['data']['instance']==base['id']]
        if len(bases)!=1 or bases[0]['after'] is not None or bases[0]['before']['count']!=1:
            raise ValueError('Exchange must exhaust the bound base instance')
        same({k:bases[0]['before'][k] for k in base},base)
        after=grant['data']['after']
        same({k:after[k] for k in product},product)
        if after['count']!=1 or after['location']!='inventory' or after['slot'] is not None:
            raise ValueError('Exchange product must enter inventory')
        expected={}
        for ingredient in recipe['ingredients']:
            if not ingredient['count']:continue
            template=ingredient['template_id']
            expected[template]=expected.get(template,0)+ingredient['count']
        actual={}
        for consume in consumes:
            data=consume['data'];parts=[c['count_delta'] for c in data['causes'] if c['cause']==ref]
            if len(parts)!=1:raise ValueError('Exchange requires one contribution per constituent stack')
            template=data['before']['template_id']
            actual[template]=actual.get(template,0)-parts[0]
        same(actual,expected)
        if any(r['tick']!=grant['tick'] or r['sequence']>=grant['sequence'] or r['data']['instance']==grant['data']['instance'] for r in consumes):
            raise ValueError('Exchange must grant a fresh instance after its same-tick consumes')
    if bound!=set(resolutions):raise ValueError('missing Exchange receipt constituents')


def validate_instance_history(rows):
    """D67/D69/D71: IDs are allocated once across every owner, tick and incarnation.

    Every observation (baseline instance, enchant before binding, and the
    `before` of every ledger operation) is compared against the recorded
    identity: same owning entity while live, and every canonical field except
    count/revision unchanged. D71: at an incarnation boundary (the first
    observation of an instance under a later incarnation of its owner) exactly
    the transient `charges` field is exempt; slot/location and every other
    canonical field must still match. D72: at that boundary `charges` may only
    CLEAR (become 0 or stay unchanged), never appear, and the owner's first
    baseline of a later incarnation (after a live earlier incarnation was
    observed) carries `auto_shots=[]`. The stored record advances from `after`
    only once its `before` has matched.
    """
    canonical={};owners={};incarnations={};exhausted=set();lives={};baselines=set()
    def strip(state,boundary):
        transient={'count','revision'}|({'charges'} if boundary else set())
        return {k:v for k,v in state.items() if k not in transient}
    def bind(identity,entity):
        # Owner-binding part of an observation; also applied to malformed enchant bindings.
        if identity in exhausted:raise ValueError('exhausted instance reappeared')
        if identity in owners and owners[identity]!=entity:raise ValueError('instance observed under two owners')
        owners[identity]=entity
    def observe(state,row):
        identity=state['id'];bind(identity,row['entity'])
        if identity in canonical:
            previous=canonical[identity]
            boundary=row['incarnation']>incarnations[identity]
            if strip(state,boundary)!=strip(previous,boundary):raise ValueError('instance identity drifted between observations')
            if boundary and state['charges'] not in (0,previous['charges']):
                raise ValueError('shot charges appeared at re-admission (D72)')
        canonical[identity]=state;incarnations[identity]=row['incarnation']
    for row in rows[1:-1]:
        data=row['data'];life=(row['entity'],row['incarnation'])
        if row['kind']=='item_baseline':
            earlier=any(0<incarnation<row['incarnation'] for incarnation in lives.get(row['entity'],()))
            if life not in baselines and earlier and data['auto_shots']!=[]:
                raise ValueError('auto-shot toggles survived re-admission (D72)')
            baselines.add(life)
        lives.setdefault(row['entity'],set()).add(row['incarnation'])
        if row['kind']=='item_baseline':
            for state in data['instances']:observe(state,row)
        elif row['kind']=='item_formula' and data['formula']=='enchant_outcome':
            # Before bindings attest existing instances; formula outputs do not allocate.
            operation=data.get('operation',{})
            if not isinstance(operation,dict):continue
            for field in ('target_before','scroll_before','support_before'):
                state=operation.get(field)
                try:validate_instance(state)
                except (ValueError,TypeError,KeyError):
                    # Enchant correlation reports malformed bindings only for the evaluated
                    # entity, so owner binding and the D68 cap are applied here for every owner.
                    if isinstance(state,dict) and type(state.get('id')) is int and state['id']>0:
                        bind(state['id'],row['entity'])
                        template,count=state.get('template_id'),state.get('count')
                        if type(template) is int and type(count) is int and template in item_tables() and count>count_cap(template):
                            raise ValueError('item count above pinned template cap (D68)')
                    continue
                observe(state,row)
        elif row['kind']=='item_delta':
            identity=data['instance'];before,after=data['before'],data['after']
            if before is None:
                if identity in canonical or identity in owners:raise ValueError('positive row must allocate a fresh instance ID')
                owners[identity]=row['entity']
            else:observe(before,row)
            # Keep the last canonical metadata even after removal; absence is a tombstone.
            canonical[identity]=after if after is not None else before
            incarnations[identity]=row['incarnation']
            if after is None:exhausted.add(identity)


def validate(rows,recording,rules_hash):
    if not isinstance(rows,list) or len(rows)<3:raise ValueError('manifest, facts and complete trailer required')
    rows=decode_rows(rows) # D80
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
    previous=start;seen=set();cursor=None;instance_steps={};instance_rows={}
    for sequence,row in enumerate(rows[1:-1]):
        if set(row)-{'origin'}!= {'kind','entity','incarnation','tick','sequence','data'}:raise ValueError('missing/unknown raw item fields')
        if 'origin' in row and row['origin'] not in ('live','offline'):raise ValueError('unknown item fact origin')
        same(row['sequence'],sequence);same(str(UUID(row['entity'])),row['entity'])
        integer(row['incarnation'],0 if row.get('origin')=='offline' else 1)
        tick=integer(row['tick'])
        if not previous<=tick<=last:raise ValueError('unordered/out-of-scope fact')
        previous=tick;data=row['data']
        if row['kind'] in ('item_rng','item_delta'):
            if not isinstance(data.get('resolution_id'),str) or not data['resolution_id']:raise ValueError('operation resolution identity required')
        if row['kind']=='item_formula':
            if set(data)-{'context'}!= ({'formula','inputs','actual','resolution_id','baseline','operation'} if data.get('formula')=='enchant_outcome' else {'formula','inputs','actual','resolution_id','baseline'}) or data['formula'] not in FORMULAS:raise ValueError('unknown/malformed formula fact')
            same(data['baseline'],False)
            if 'context' in data:
                context=data['context']
                if not isinstance(context,dict) or set(context)-{'weapon_template','skill_id','target_entity','event','set_id'}:raise ValueError('invalid formula context')
                for key in ('weapon_template','skill_id','set_id'):
                    if key in context and integer(context[key])>=2**32:raise ValueError('context ID outside u32')
                if 'target_entity' in context:same(str(UUID(context['target_entity'])),context['target_entity'])
                if 'event' in context and context['event'] not in ('equip','unequip','enchant','admission','attack','cast'):raise ValueError('unknown formula event')
            if not isinstance(data['resolution_id'],str) or not data['resolution_id']:raise ValueError('resolution identity required')
            identity=(row['entity'],row['incarnation'],data['resolution_id'],data['formula'])
            if identity in seen:raise ValueError('repeated formula resolution')
            seen.add(identity)
            if not isinstance(data['inputs'],dict):raise ValueError('actual input object required')
        elif row['kind']=='item_baseline':
            if set(data)!={'inventory_revision','instances','paperdoll','auto_shots'}:raise ValueError('unknown/missing inventory baseline fields')
            integer(data['inventory_revision'])
            if not isinstance(data['instances'],list) or not isinstance(data['paperdoll'],dict) or not isinstance(data['auto_shots'],list):raise ValueError('invalid baseline collections')
            ids=set()
            for item in data['instances']:
                validate_instance(item)
                identity=item['id']
                if identity in ids:raise ValueError('duplicate baseline instance')
                ids.add(identity)
            expected={}
            for item in data['instances']:
                if item['slot'] is not None:
                    if item['slot'] in expected:raise ValueError('duplicate slot occupancy')
                    expected[item['slot']]=item['id']
            for slot,identity in data['paperdoll'].items():
                if slot not in SLOTS:raise ValueError('unknown paperdoll slot')
                integer(identity,1)
            same(data['paperdoll'],expected)
            for template in data['auto_shots']:
                if integer(template,1)>=2**32:raise ValueError('auto-shot ID outside u32')
                if template not in shot_templates():raise ValueError('unsupported auto-shot template')
            if len(set(data['auto_shots']))!=len(data['auto_shots']):raise ValueError('duplicate auto-shot')
        elif row['kind']=='item_rng':
            if set(data)!= {'callsite','word_before','word_after','bound','value','resolution_id'}:raise ValueError('unknown/missing RNG fields')
            before,after=integer(data['word_before']),integer(data['word_after']);bound=integer(data['bound'],1);value=integer(data['value'])
            if after<=before or cursor is not None and before!=cursor or value>=bound:raise ValueError('invalid RNG cursor/residue')
            cursor=after
            if not isinstance(data['callsite'],str) or not data['callsite']:raise ValueError('RNG callsite required')
        elif row['kind']=='item_delta':
            receipt_field='receipt_refs' if data.get('operation')=='consume' else 'receipt_ref'
            fields={'operation','instance','before','after','count_delta','operation_ordinal',receipt_field,'resolution_id'}
            if data.get('operation') in ('consume','use'):fields.add('causes')
            if set(data)!=fields:raise ValueError('unknown/missing ledger fields')
            if data['operation'] not in ITEM_TRANSITIONS:raise ValueError('unknown ledger operation')
            refs=data[receipt_field]
            if receipt_field=='receipt_refs':
                if not isinstance(refs,list) or any(not isinstance(ref,str) or not ref for ref in refs):raise ValueError('ordered consume receipt_refs required')
                causes=data['causes']
                same(refs,validate_causes(causes,True))
                same(data['count_delta'],sum(c['count_delta'] for c in causes))
                first=causes[0]['cause']
                if not first.startswith('receipt:'):same(data['resolution_id'],first.split(':',1)[1])
            elif refs is not None and (not isinstance(refs,str) or not refs):
                raise ValueError('single ledger receipt_ref required')
            if data['operation']=='use':
                # D73: the receipt projection of a use row is its single optional receipt_ref.
                same([] if refs is None else [refs],validate_causes(data['causes'],False))
            instance=integer(data['instance'],1)
            before,after=data['before'],data['after']
            if before==after or before is None and after is None:raise ValueError('ledger must contain actual change (D28)')
            for value in (before,after):
                if value is not None:
                    # Canonical domain instances, not partial count-only projections.
                    validate_instance(value,cap=False)
                    if value['id']!=instance:raise ValueError('ledger identity mismatch')
            ordinal=integer(data['operation_ordinal'])
            key=(row['entity'],row['incarnation'],data['resolution_id'],instance,ordinal)
            if key in seen:raise ValueError('repeated ledger fact')
            seen.add(key)
            validate_transition(data)
            if before is not None and after is not None and after['revision']<before['revision']:raise ValueError('item revision regressed')
            identity=(row['entity'],row['incarnation'],instance)
            prior=instance_steps.get(identity)
            if prior is not None and tick==prior[0] and ordinal<=prior[1]:raise ValueError('item operation ordinal did not increase within tick')
            instance_steps[identity]=(tick,ordinal)
            instance_rows.setdefault((*identity,tick),[]).append(data)
        else:raise ValueError('unknown item raw fact kind')
    for deltas in instance_rows.values():
        validate_chain_caps(deltas)
        consumes=[d for d in deltas if d['operation']=='consume']
        if not consumes:continue
        if len(consumes)!=1 or deltas[-1] is not consumes[0]:raise ValueError('Consume aggregate must be last for its instance')
        # D69 (amends D66): positive AND count-neutral retained rows precede the Consume
        # aggregate in cause order; continuity is checked by inventory_history. Removal
        # rows end the instance, so they can never precede it.
        if any(d['operation'] not in PRE_CONSUME for d in deltas[:-1]):raise ValueError('only positive or count-neutral rows may precede Consume')
        for ordinal,delta in enumerate(deltas):same(delta['operation_ordinal'],ordinal)
    validate_exchanges(rows)
    validate_instance_history(rows)
    inventory_history(rows)
    return manifest


def inventory_history(rows,targets=None):
    """D55/D66/D79: one owner step, ordered continuity and conservation per tick.

    D79: an item's `revision` is its LAST-CHANGE revision. The owner advances one
    step per changed tick and every post-state written in that tick carries the
    new step; untouched instances keep their older revision (no per-tick
    re-stamp). A `before` therefore equals the instance's retained state exactly,
    including that last-change revision: strictly below the tick's step on its
    first touch in the tick, equal to the step for later same-tick rows.
    """
    groups={};owners={};steps={}
    for row in rows[1:-1]:
        groups.setdefault((row['entity'],row['incarnation'],row['tick']),[]).append(row)
    for scope,facts in groups.items():
        owner=scope[:2];tick=scope[2]
        revision,states,complete,floor=owners.get(owner,(None,{},False,-1))
        balances={}
        for row in facts:
            if row['kind']=='item_baseline':
                baseline=row['data'];bound={item['id']:item for item in baseline['instances']}
                if any(item['revision']>baseline['inventory_revision'] for item in baseline['instances']):
                    raise ValueError('baseline item revision ahead of inventory revision (D79)')
                if revision is not None:
                    same(baseline['inventory_revision'],revision)
                    if complete:same(bound,{identity:state for identity,state in states.items() if state is not None})
                    else:
                        for identity,state in states.items():same(bound.get(identity),state)
                elif baseline['inventory_revision']<floor:raise ValueError('baseline inventory revision regressed')
                revision=baseline['inventory_revision'];states=bound;complete=True
        deltas=[row['data'] for row in facts if row['kind']=='item_delta']
        if not deltas:
            owners[owner]=(revision,states,complete,floor)
            continue
        afters={d['after']['revision'] for d in deltas if d['after'] is not None}
        if revision is None:
            # A slice without a baseline binds its step from the tick's post-states;
            # befores carry older last-change revisions and cannot bind it (D79).
            if len(afters)>1:raise ValueError('owner item revision changed within tick')
            current=afters.pop() if afters else None
            if current is not None and current<=floor:raise ValueError('item revision did not increase across ticks')
            revision=None if current is None else current-1
        else:current=revision+1
        steps[scope]=(revision,current)
        touched=set()
        for row in facts:
            if row['kind']=='item_formula' and row['data']['formula']=='enchant_outcome' and targets is not None:
                operation=row['data'].get('operation')
                target=operation.get('target_before') if isinstance(operation,dict) else None
                try:validate_instance(target)
                except (ValueError,TypeError,KeyError):continue # Enchant validation reports malformed bindings.
                identity=target['id']
                targets[row['sequence']]=(identity in states or complete,states.get(identity))
                if identity not in states and not complete:states[identity]=target
            if row['kind']!='item_delta':continue
            delta=row['data']
            before,after=delta['before'],delta['after']
            if after is not None and current is not None:
                if revision is not None and after['revision']<=revision:raise ValueError('item revision did not increase across ticks')
                if after['revision']!=current:raise ValueError('owner item revision changed within tick')
            identity=delta['instance']
            if before is not None:
                if identity in touched:
                    if current is not None and before['revision']!=current:raise ValueError('same-tick before differs from owner step')
                elif current is not None and before['revision']>=current:
                    raise ValueError('item before revision is not an earlier last-change revision (D79)')
                floor=max(floor,before['revision'])
            balance=balances.setdefault(identity,[before['count'] if before else 0,0])
            balance[1]+=delta['count_delta']
            if identity in states or complete:
                same(before,states.get(identity))
            states[identity]=after;touched.add(identity)
        for identity,(start_count,net_delta) in balances.items():
            end=states[identity]
            if (end['count'] if end else 0)!=start_count+net_delta:
                raise ValueError('per-instance tick count conservation mismatch')
        # D79: untouched instances keep their last-change revision.
        if current is not None:floor=max(floor,current)
        owners[owner]=(current,states,complete,floor)
    return steps


def refund_inputs_for(target):
    definition=item_tables()[target['template_id']]
    grade=source().grades[definition['grade']]
    return dict(base=definition['crystal_count'],bonus=grade['weapon_bonus' if definition['kind']=='Weapon' else 'armor_bonus'],
                enchant=target['enchant'],weapon=definition['kind']=='Weapon',
                crystallizable=definition['flags']['is_crystallizable'],crystal_id=grade['crystal_id'])


def enchant_evidence(row, rows, steps=None,targets=None):
    """RequestEnchantItem.java:123-139,154-271; EnchantScroll.java:149-155.

    Even guaranteed success draws; safe/reset/no-increment have no fake target
    ledger change. A noncrystallizable destruction legitimately has no refund.
    """
    data=row['data'];identity=(row['entity'],row['incarnation'],data['resolution_id'])
    correlated=[r for r in rows[1:-1] if r['kind'] in ('item_formula','item_rng','item_delta')
                and (r['entity'],r['incarnation'],r['data']['resolution_id'])==identity]
    op=data['operation']
    if set(op)!={'target_before','scroll_before','support_before','receipt_ref'}:raise ValueError('incomplete enchant operation binding')
    if not isinstance(op['receipt_ref'],str) or not op['receipt_ref']:raise ValueError('enchant receipt binding required')
    target,scroll,support=(op[k] for k in ('target_before','scroll_before','support_before'))
    for item in (target,scroll,support):
        if item is not None:validate_instance(item)
    if target is None or scroll is None or len({i['id'] for i in (target,scroll,support) if i})!=2+int(support is not None):raise ValueError('invalid enchant instance binding')
    expected=enchant_source_inputs(target,scroll,support)
    decoded=oracle.decode_rationals(data['inputs'])
    for key,value in expected.items():
        if key in ('safe','blessed'):same(data['inputs'].get(key,False),value)
        elif oracle.exact(decoded[key])!=value:raise ValueError('enchant source input mismatch: '+key)
    formula('enchant_outcome',data['inputs'],data['actual'])
    same(data['inputs']['enchant'],target['enchant'])
    chance=[r for r in correlated if r['kind']=='item_formula' and r['data']['formula']=='enchant_chance']
    if len(chance)!=1:raise ValueError('one correlated enchant chance required')
    formula('enchant_chance',chance[0]['data']['inputs'],chance[0]['data']['actual'])
    chance_inputs=oracle.decode_rationals(chance[0]['data']['inputs'])
    for key in ('base','bonus','support'):
        if oracle.exact(chance_inputs.get(key,0))!=expected[key]:raise ValueError('enchant chance source mismatch: '+key)
    same(chance[0]['data']['actual'],oracle.enchant_chance(**{k:decoded[k] for k in ('base','bonus','support')}))
    draws=[r['data'] for r in correlated if r['kind']=='item_rng']
    if len(draws)!=1 or draws[0]['callsite']!='enchant' or draws[0]['bound']!=100000:raise ValueError('one source enchant RNG draw required')
    same(draws[0]['value'],data['inputs']['residue'])
    deltas=[r['data'] for r in correlated if r['kind']=='item_delta']
    required=[]
    def require(instance,operation,before,after):
        matched=[d for d in deltas if d['instance']==instance and d['operation']==operation]
        if len(matched)!=1:raise ValueError('missing/duplicate enchant '+operation+' row')
        same(matched[0]['receipt_ref'],op['receipt_ref'])
        same(matched[0]['before'],before)
        if after is None:same(matched[0]['after'],None)
        else:
            actual=matched[0]['after']
            if actual is None:raise ValueError('missing enchant post-instance')
            same({k:v for k,v in actual.items() if k!='revision'},{k:v for k,v in after.items() if k!='revision'})
        required.append(matched[0])
    scope=(row['entity'],row['incarnation'],row['tick'])
    outcomes=[r for r in rows[1:-1] if (r['entity'],r['incarnation'],r['tick'])==scope
              and r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome']
    receipts=[r['data']['operation']['receipt_ref'] for r in outcomes]
    if any(not isinstance(ref,str) or not ref for ref in receipts) or len(set(receipts))!=len(receipts):
        raise ValueError('duplicate/invalid enchant resolution receipt')
    previous_step,tick_step=(inventory_history(rows) if steps is None else steps)[scope]
    if targets is None:
        targets={};inventory_history(rows,targets)
    known,established=targets[row['sequence']]
    # D79: a binding carries the instance's last-change revision, never beyond the tick's step.
    if tick_step is not None and target['revision']>tick_step:raise ValueError('enchant target revision ahead of owner revision step')
    if known:same(target,established)
    for spent in (scroll,support):
        if spent is None:continue
        contributions=[]
        for other in outcomes:
            binding=other['data']['operation']
            for key in ('scroll_before','support_before'):
                item=binding[key]
                if item is not None and item['id']==spent['id']:
                    validate_instance(item)
                    contributions.append((other['data']['resolution_id'],binding['receipt_ref'],item))
        receipts=[receipt for _,receipt,_ in contributions]
        if len(set(receipts))!=len(receipts):raise ValueError('duplicate enchant receipt contribution')
        aggregates=[r['data'] for r in rows[1:-1] if r['kind']=='item_delta'
                    and (r['entity'],r['incarnation'],r['tick'])==scope
                    and r['data']['operation']=='consume' and r['data']['instance']==spent['id']]
        if len(aggregates)!=1:raise ValueError('missing/duplicate enchant consume row')
        aggregate=aggregates[0]
        by_receipt={receipt:(resolution,before) for resolution,receipt,before in contributions}
        receipt_operations={receipt.split(':',2)[1] for receipt in by_receipt}
        causes=aggregate['causes']
        matched=[c['cause'] for c in causes if c['cause'] in by_receipt]
        same(matched,receipts)
        for cause in causes:
            ref=cause['cause']
            if ref.startswith('receipt:') and ref.split(':',2)[1] in receipt_operations and ref not in by_receipt:
                raise ValueError('missing enchant consume contribution')
        first=aggregate['before']
        if first is None:raise ValueError('missing enchant consume initial instance')
        if tick_step is not None and first['revision']>tick_step:raise ValueError('enchant contribution revision ahead of owner revision step')
        remaining=first['count'];previous_revision=first['revision']
        for index,cause in enumerate(causes):
            ref=cause['cause']
            if ref in by_receipt:
                resolution,before=by_receipt[ref]
                if index==0:same(aggregate['resolution_id'],resolution)
                same(cause['count_delta'],-1)
                validate_instance(before)
                same({k:v for k,v in before.items() if k!='revision'},
                     {k:v for k,v in (first|{'count':remaining}).items() if k!='revision'})
                revision=before['revision']
                if revision<previous_revision or tick_step is not None and revision>tick_step:raise ValueError('enchant contribution revision regressed')
                if index and tick_step is not None:same(revision,tick_step)
                previous_revision=revision
            elif index==0 and not ref.startswith('receipt:'):
                same(aggregate['resolution_id'],ref.split(':',1)[1])
            remaining+=cause['count_delta']
            if remaining<0:raise ValueError('insufficient enchant consume stack')
        if aggregate in deltas:required.append(aggregate)
    outcome=data['actual'];destroyed=outcome['destroyed']
    if destroyed:require(target['id'],'destroy',target,None)
    elif outcome['enchant']!=target['enchant']:require(target['id'],'enchant',target,target|{'enchant':outcome['enchant']})
    refund_inputs=refund_inputs_for(target)
    crystal=refund_inputs['crystal_id']
    refund=oracle.failure_refund(**refund_inputs) if destroyed else 0
    refunds=[r for r in correlated if r['kind']=='item_formula' and r['data']['formula']=='failure_refund']
    if refund:
        if len(refunds)!=1:raise ValueError('missing correlated refund computation')
        same(refunds[0]['data']['inputs'],refund_inputs);formula('failure_refund',refund_inputs,refunds[0]['data']['actual'])
    elif refunds:raise ValueError('refund on branch without source refund')
    if refund:
        candidates=[d for d in deltas if d['operation']=='grant' and d['after'] and d['after']['template_id']==crystal]
        if len(candidates)!=1 or candidates[0]['count_delta']!=refund:raise ValueError('missing exact enchant crystal refund')
        grant=candidates[0]
        same(grant['receipt_ref'],op['receipt_ref'])
        if grant['after']['location']!='inventory' or grant['after']['slot'] is not None:
            raise ValueError('refund must enter inventory')
        required.append(grant)
    if len(required)!=len(deltas):raise ValueError('unexpected correlated enchant ledger row')


def evaluate(rows,recording,entity,gates,rules_hash,phase_specs=()):
    rows=decode_rows(rows) # D80: wide integers arrive as decimal strings.
    manifest=validate(rows,recording,rules_hash)
    targets={};enchant_errors=[]
    try:steps=inventory_history(rows,targets)
    except ValueError as error:steps={};enchant_errors.append(str(error))
    results=[]
    for row in rows[1:-1]:
        if row['entity']==entity and row['kind']=='item_formula' and row['data']['formula']=='enchant_outcome':
            try:enchant_evidence(row,rows,steps,targets)
            except (ValueError,TypeError,KeyError,IndexError,ZeroDivisionError) as error:enchant_errors.append(str(error))
    resolutions={(r['entity'],r['incarnation'],r['data']['resolution_id']) for r in rows[1:-1] if r['entity']==entity and r['kind']=='item_formula' and r['data']['formula']=='enchant_outcome'}
    for row in rows[1:-1]:
        if row['entity']==entity and row['kind']=='item_formula' and row['data']['formula'] in ('enchant_chance','failure_refund') and (row['entity'],row['incarnation'],row['data']['resolution_id']) not in resolutions:
            enchant_errors.append('missing correlated enchant outcome')
    owner=[r for r in rows[1:-1] if r['entity']==entity and r['kind']=='item_formula']
    for gate in gates:
        if gate=='grant_denied':
            # Auth-dev gating happens BEFORE enqueue: no denied command/formula exists in the zone log.
            # Client JUnit proves the RPC status; complete C14 evidence proves no actor grant.
            baselines=[r for r in rows[1:-1] if r['entity']==entity and r['kind']=='item_baseline']
            grants=[r for r in rows[1:-1] if r['entity']==entity and r['kind']=='item_delta' and r['data']['operation']=='grant']
            errors=[]
            if not baselines:errors.append('missing received owner baseline binding')
            if grants:errors.append('disabled GrantItem nevertheless materialized a grant')
            results.append(dict(formula=gate,sequences=[r['sequence'] for r in baselines],applicable=True,errors=errors))
            continue
        matched=[r for r in owner if r['data']['formula']==gate];errors=list(enchant_errors) if gate in ('enchant_outcome','enchant_chance','failure_refund') else []
        applicable=True
        if gate=='failure_refund' and not matched:
            outcomes=[r for r in owner if r['data']['formula']=='enchant_outcome']
            if outcomes and not enchant_errors and all(not r['data']['actual']['destroyed'] or oracle.failure_refund(**refund_inputs_for(r['data']['operation']['target_before']))==0 for r in outcomes):
                # Only refundable destruction executes this source branch.
                for outcome in outcomes:formula('enchant_outcome',outcome['data']['inputs'],outcome['data']['actual'])
                applicable=False
        if not matched and applicable:errors.append('missing owner actual-input resolution fact')
        for row in matched:
            try:formula(gate,row['data']['inputs'],row['data']['actual'])
            except (ValueError,TypeError,KeyError,IndexError,ZeroDivisionError) as error:errors.append(str(error))
        results.append(dict(formula=gate,sequences=[r['sequence'] for r in matched],applicable=applicable,errors=errors))
    for phase in phase_specs:
        matched=[]
        for row in owner:
            data=row['data']
            if data['formula']!=phase['formula']:continue
            if any(data.get('context',{}).get(k)!=v for k,v in phase.get('context',{}).items()):continue
            if any(k not in data['inputs'] or data['inputs'][k]!=v for k,v in phase.get('inputs',{}).items()):continue
            equipment={r[2]:r[0] for r in data['inputs'].get('equipment',[])}
            if any(equipment.get(slot)!=template for slot,template in phase.get('equipment',{}).items()):continue
            if 'actual' in phase and data['actual']!=phase['actual']:continue
            providers={r[0] for r in data['inputs'].get('skill_refs',[])}
            if not set(phase.get('providers',[]))<=providers or set(phase.get('absent_providers',[]))&providers:continue
            if 'mutation' in phase:
                mutation=phase['mutation'];found=False
                for delta in rows[1:-1]:
                    if delta['kind']!='item_delta' or (delta['entity'],delta['incarnation'],delta['data']['resolution_id'])!=(row['entity'],row['incarnation'],data['resolution_id']):continue
                    d=delta['data'];item=d['after'] if mutation['operation']=='equip' else d['before']
                    if item and d['operation']==mutation['operation'] and item['template_id']==mutation['template'] and item['slot']==mutation['slot']:found=True
                if not found:continue
            try:formula(data['formula'],data['inputs'],data['actual'])
            except (ValueError,TypeError,KeyError,IndexError,ZeroDivisionError):continue
            matched.append(row)
        results.append(dict(formula=phase['formula'],phase=phase['name'],sequences=[r['sequence'] for r in matched],applicable=True,
                            errors=[] if matched else ['missing scoped owner phase resolution']))
    if enchant_errors and not any(r['errors'] for r in results):
        results.append(dict(formula='enchant_resolution_integrity',errors=enchant_errors))
    return dict(schema_version=1,recording_sha256=manifest['recording_sha256'],phase4_rules_hash=rules_hash,
                entity=entity,gates=results,passed=bool(results) and not enchant_errors and not any(r['errors'] for r in results))


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
    result=evaluate(read_jsonl(raw),recording,entity,gates,rules_hash,phases(scenario))
    out.write_text(json.dumps(result,sort_keys=True,indent=2)+'\n')
    return int(not result['passed'])


def soak(rows,recording,roles,rules_hash):
    rows=decode_rows(rows) # D80
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
