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


# Flags are typed per oracle parameter, not by a global name (support is numeric
# for enchant_outcome but boolean for enchant_valid).
BOOLEAN_REQUIRED = {
    'crystal_count': {'weapon'}, 'failure_refund': {'weapon'},
    'bow_launch': {'miss'}, 'resurrection_scroll': {'dead','repeat'},
    'learned_item_cost': {'repeat'}, 'dual_plan': {'second_valid'},
    'enchant_safe': {'fullarmor'}, 'retail_draw_contract': {'accessory'},
    'item_regen_status': {'dead','stunned'},
    'set_shield_history': {'previous','chest_present','matching_shield'},
}
INTEGER_INPUTS = {
    'enchant_increment': {'enchant'}, 'enchant_hp': {'enchant'},
    'crystal_count': {'base','bonus','enchant'}, 'failure_refund': {'base','bonus','enchant','crystal_id'},
    'enchant_outcome': {'enchant','residue'}, 'enchant_valid': {'current','max_enchant'},
    'fist_spread': {'level','residue','template_radius'}, 'pole_sequence': {'maximum'},
    'bow_reuse': {'reuse_delay'}, 'bow_damage': {'dx_milli','dy_milli'},
    'shield': {'perfect_residue','normal_residue'}, 'blow_success': {'residue'},
    'lethal': {'full_residue','half_residue'}, 'retail_select': {'residue'},
    'category': {'residue'}, 'pack_options': {'first','second'}, 'cancel_fee': {'crystals'},
    'shot_payment': {'count','available','residue'}, 'gear_stats': {'level','base_delta_sequence'},
    'grant_denied': {'ledger_rows','rng_before','rng_after'},
    'enchant_safe': {'current','safe_enchant','safe_fullarmor','residue'},
    'player_level_range': {'level','minimum','maximum'},
}
# Canonical counts/ticks are signed-bigint, enchant values are u32 (E1 item
# oracle checked_u32; D17/D28). Additive stat modifiers and coordinates stay signed.
INTEGER_INPUTS.update({
    'crystal_count': {'base','bonus','enchant'}, 'failure_refund': {'base','bonus','enchant','crystal_id'},
    'cast_time': {'hit_time'}, 'heal': {'current_hp','remainder_q'},
    'overload': {'weight','maximum','previous_level'}, 'grade_penalties': {'expertise','bonus'},
    'consumable_use': {'tick'}, 'consumable_cast_sequence': {'caster_ordinal'},
    'player_level_range': {'rank','level'},
    'resurrection_scroll': {'start_tick','available','death_incarnation','current_incarnation','response_tick','response_offer_id','recoverable_xp'},
    'learned_item_cost': {'skill_id','rank','consume_id','consume_count'},
    'escape_timing': {'start_tick','available','incarnation'},
    'dual_plan': {'hit_chance','spread_radius'},
    'dormant_use': {'template_id','selected_instance','allocator_next'},
    'augmentation_provider': {'option_id','other_provider'},
    'item_regen_status': {'current','maximum','remainder_q'},
    'bow_launch': {'lhand','reuse_remaining','hit_chance','hit_residue','reduction_residue'},
    'magic_damage': {'level_difference'},
})
NONNEGATIVE_INPUTS = {
    'enchant_hp': {'bonuses'}, 'enchant_chance': {'base','bonus','support'},
    'enchant_outcome': {'base','bonus','support'}, 'quantized_enchant': {'base','bonus','support'},
    'enchant_safe': {'base'}, 'max_load': {'con_bonus','weight_limit','multiplier'},
    'overload': {'bonus'}, 'pole_damage': {'percent'},
    'bow_reuse': {'weapon_reuse_modifier'}, 'shield': {'rate','dex_bonus'},
    'physical_damage': {'p_atk','defence','power','shield_def','crit_dmg','crit_pos','def_crit_dmg','trait','spread','pvp','attribute','npc'},
    'magic_damage': {'power','m_atk','m_def','shield_def','pvp_magic_def','pvp_skill_def','magic_crit_damage','spread','pvp_magic_damage','pvp_skill_damage','attribute','npc'},
    'heal': {'power','m_atk','mp_consume2','heal_effect','recoverable_hp'},
    'blow_success': {'chance','dex','modifier'}, 'lethal': {'full','half','attribute','trait'},
    'clamp_no_heal': {'current','new_max'}, 'bow_launch': {'mp','mp_modifier'},
    'bow_refill': {'mp'}, 'resurrection_scroll': {'hp','max_hp','mp','cp','restore_percent','restore_hp'},
    'augmentation_provider': {'hp','maximum'},
    'item_regen_status': {'base','level_modifier','stat_bonus','posture_modifier','regen_modifier','periodic_credit'},
}
POSITIVE_INPUTS = {'dual_timing': {'attack_speed'}, 'bow_reuse': {'attack_speed'},
                   'cast_time': {'casting_speed'}, 'escape_timing': {'casting_speed'}}
U32_INPUTS = {'enchant_increment': {'enchant'}, 'enchant_hp': {'enchant'},
    'crystal_count': {'enchant'}, 'failure_refund': {'enchant'}, 'enchant_outcome': {'enchant'},
    'enchant_valid': {'current','max_enchant'}, 'enchant_safe': {'current','safe_enchant','safe_fullarmor'}}


def input_domains(name,inputs):
    def bounded(value,minimum=0,maximum=2**63-1):
        integer(value,minimum)
        if value>maximum:raise ValueError('formula integer outside domain')
    for key in INTEGER_INPUTS.get(name,()):
        if inputs.get(key) is None:continue
        minimum=0
        if key in ('dx_milli','dy_milli','level_difference') or name=='grade_penalties' and key=='bonus':minimum=-2**63
        elif key=='base_delta_sequence':minimum=-1
        elif key in ('level','rank','incarnation','death_incarnation','current_incarnation','template_id','skill_id','selected_instance','allocator_next','option_id','other_provider'):minimum=1
        bounded(inputs[key],minimum,2**32-1 if key in U32_INPUTS.get(name,()) or key in ('crystal_id','template_id','skill_id','consume_id','rank') else 2**63-1)
    decoded=oracle.decode_rationals(inputs)
    for key in NONNEGATIVE_INPUTS.get(name,set()) | POSITIVE_INPUTS.get(name,set()):
        value=decoded.get(key)
        if value is None:continue
        for number in value if isinstance(value,list) else [value]:
            number=oracle.exact(number)
            if number<0 or key in POSITIVE_INPUTS.get(name,()) and number==0:
                raise ValueError('formula numeric outside domain: '+key)
            if name=='resurrection_scroll' and key=='restore_hp' and number>1:
                raise ValueError('restoration ratio outside domain')
    bounds={'enchant_outcome': {'residue':99999}, 'enchant_safe': {'residue':99999},
            'blow_success': {'residue':999}, 'lethal': {'full_residue':999,'half_residue':999},
            'shield': {'perfect_residue':99,'normal_residue':99},
            'retail_select': {'residue':9999}, 'category': {'residue':9999},
            'pack_options': {'first':65535,'second':65535},
            'heal': {'remainder_q':oracle.Q-1}, 'item_regen_status': {'remainder_q':oracle.Q-1},
            'overload': {'previous_level':4}, 'bow_launch': {'hit_residue':999,'reduction_residue':99,'hit_chance':1000}}
    bounds['shot_payment']={'residue':99}
    for key,maximum in bounds.get(name,{}).items():
        if inputs.get(key) is not None:bounded(inputs[key],1 if name=='pack_options' else 0,maximum)
    # Nested canonical operands, including inventories and gear enchant levels.
    def nested(value,key=''):
        if isinstance(value,dict):
            if key in ('available','inventory','ingredients_before','ingredients_after','item_deadlines','skill_deadlines'):
                for count in value.values():bounded(count)
            elif set(value)!={'numerator','denominator'}:
                for child_key,child in value.items():nested(child,child_key)
        elif isinstance(value,list):
            for child in value:nested(child)
        elif value is not None and key in ('count','enchant','tick','start_tick','expires_at','created_at','created_tick','revision','charges','sequence','hit_time_ms','reuse_delay_ms','item_consume_count','mp_cost','reduced_cost','spread_radius','crystal_count','instance_id','template_id'):
            bounded(value,1 if key in ('instance_id','template_id') else 0,
                    2**32-1 if key in ('enchant','template_id') else 2**63-1)
    nested(inputs)
    for template,enchant,slot,sequence in inputs.get('equipment',[]):
        bounded(template,1,2**32-1);bounded(enchant,0,2**32-1);bounded(sequence)
    for skill,rank,sequence in inputs.get('skill_refs',[]):
        bounded(skill,1,2**32-1);bounded(rank,1,2**32-1);bounded(sequence)
    for key in ('hit_residues',):
        for value in inputs.get(key,[]):bounded(value,0,999)
    if inputs.get('reduction'):
        chance,count=inputs['reduction'];bounded(chance,0,100);bounded(count)
    if name in ('retail_select','retail_distribution'):
        for option,chance in decoded['rows']:
            bounded(option,1,65535)
            if not 0<=oracle.exact(chance)<=100:raise ValueError('retail chance outside domain')
    if name=='winning_providers':
        for provider in inputs['providers']:
            bounded(provider['skill_id'],1,2**32-1);bounded(provider['rank'],1,2**32-1);bounded(provider['sequence'])
NESTED_FLAGS = set(('weapon arrow eligible hit admitted stackable equipped static simultaneous immediate '
    'ex_immediate ex_immediate_effect immediate_effect dead invulnerable hp_blocked is_magic_weapon '
    'is_crystallizable is_common is_hero is_pvp for_npc allow_self_resurrection is_oly_restricted '
    'is_sellable is_droppable is_destroyable is_tradable is_depositable element_enabled is_questitem '
    'is_freightable repeat').split())


def input_types(name, inputs):
    function=getattr(oracle,name,None) or getattr(oracle.behavior,name,None)
    parameters=inspect.signature(function).parameters if function else {}
    flags=set(BOOLEAN_REQUIRED.get(name,()))
    if function:
        flags.update(k for k,p in inspect.signature(function).parameters.items() if type(p.default) is bool)
    flags.update({'auth_dev_tokens'} if name=='grant_denied' else ())
    if name=='magic_damage':flags.update({'is_pvp','second_success'})
    def check(value, boolean=False, nested=False):
        if type(value) is bool:
            if not boolean:raise ValueError('boolean numeric input forbidden')
        elif boolean:raise ValueError('boolean parameter required')
        elif isinstance(value,float):raise ValueError('float input forbidden')
        elif isinstance(value,dict):
            if set(value)=={'numerator','denominator'}:
                integer(value['numerator'],minimum=-(2**63));integer(value['denominator'],1)
            else:
                for key,child in value.items():check(child,key in NESTED_FLAGS,True)
        elif isinstance(value,list):
            for child in value:check(child,False,nested)
    for key,value in inputs.items():
        if value is None and name=='magic_damage' and key in ('is_pvp','second_success'):continue
        if key in INTEGER_INPUTS.get(name,()):
            nullable=key in parameters and parameters[key].default is None or (name,key) in {
                ('bow_launch','lhand'),('bow_launch','reduction_residue'),
                ('resurrection_scroll','response_offer_id'),('augmentation_provider','other_provider')}
            if not (value is None and nullable) and type(value) is not int:
                raise ValueError('integer formula input required: '+key)
        if name in ('dual_plan','shared_shot_bundle') and key=='hits':
            if not isinstance(value,list):raise ValueError('boolean hit list required')
            for child in value:check(child,True)
        else:check(value,key in flags)


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
    input_types(name,inputs)
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


SLOTS=set(('under head hair hair2 neck rhand chest lhand rear lear gloves legs feet '
           'rfinger lfinger lbracelet rbracelet deco1 deco2 deco3 deco4 deco5 deco6 cloak belt').split())


def validate_instance(value):
    fields={'id','template_id','count','location','slot','enchant','augment_id','charges','created_tick','revision'}
    if not isinstance(value,dict) or set(value)!=fields:raise ValueError('missing/unknown canonical item scalar')
    for name in ('id','template_id','count'):integer(value[name],1)
    for name in ('enchant','charges','created_tick','revision'):integer(value[name])
    if value['id']>=2**63 or value['count']>=2**63 or value['template_id']>=2**31 or value['enchant']>=2**32 or value['charges']>7 or value['created_tick']>=2**63 or value['revision']>=2**63:raise ValueError('item scalar outside domain width')
    # Dormant starter items remain canonical catalogue members (D17).
    if value['template_id'] not in item_tables():raise ValueError('unknown canonical template')
    if value['location'] not in ('inventory','paperdoll') or (value['location']=='paperdoll')!=(value['slot'] is not None):raise ValueError('item location/slot mismatch')
    if value['slot'] is not None and value['slot'] not in SLOTS:raise ValueError('unknown canonical slot')
    if value['charges'] and value['slot']!='rhand':raise ValueError('charge on nonweapon slot')
    if value['augment_id'] is not None:
        augment=integer(value['augment_id'],1)
        if augment>=2**32 or augment>>16==0 or augment&65535==0:raise ValueError('invalid packed option pair')


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
            if set(data)!= {'operation','instance','before','after','count_delta','operation_ordinal','receipt','resolution_id'}:raise ValueError('unknown/missing ledger fields')
            if data['operation'] not in {'grant','starter','backfill','equip','unequip','consume','enchant','destroy','crystallize','refine','cancel_refine','exchange','use'}:raise ValueError('unknown ledger operation')
            instance=integer(data['instance'],1)
            before,after=data['before'],data['after']
            if before==after or before is None and after is None:raise ValueError('ledger must contain actual change (D28)')
            for value in (before,after):
                if value is not None:
                    # Canonical domain instances, not partial count-only projections.
                    validate_instance(value)
                    if value['id']!=instance:raise ValueError('ledger identity mismatch')
            net=(after['count'] if after else 0)-(before['count'] if before else 0)
            same(data['count_delta'],net)
            ordinal=integer(data['operation_ordinal'])
            key=(row['entity'],row['incarnation'],data['resolution_id'],instance,data['operation'],ordinal)
            if key in seen:raise ValueError('repeated ledger fact')
            seen.add(key)
            if data['operation']=='consume':same(ordinal,0)
        else:raise ValueError('unknown item raw fact kind')
    return manifest


def refund_inputs_for(target):
    definition=item_tables()[target['template_id']]
    grade=source().grades[definition['grade']]
    return dict(base=definition['crystal_count'],bonus=grade['weapon_bonus' if definition['kind']=='Weapon' else 'armor_bonus'],
                enchant=target['enchant'],weapon=definition['kind']=='Weapon',
                crystallizable=definition['flags']['is_crystallizable'],crystal_id=grade['crystal_id'])


def enchant_evidence(row, rows):
    """RequestEnchantItem.java:123-139,154-271; EnchantScroll.java:149-155.

    Even guaranteed success draws; safe/reset/no-increment have no fake target
    ledger change. A noncrystallizable destruction legitimately has no refund.
    """
    data=row['data'];identity=(row['entity'],row['incarnation'],data['resolution_id'])
    correlated=[r for r in rows[1:-1] if r['kind'] in ('item_formula','item_rng','item_delta')
                and (r['entity'],r['incarnation'],r['data']['resolution_id'])==identity]
    op=data['operation']
    if set(op)!={'target_before','scroll_before','support_before'}:raise ValueError('incomplete enchant operation binding')
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
        same(matched[0]['before'],before)
        if after is None:same(matched[0]['after'],None)
        else:
            actual=matched[0]['after']
            if actual is None:raise ValueError('missing enchant post-instance')
            same({k:v for k,v in actual.items() if k!='revision'},{k:v for k,v in after.items() if k!='revision'})
            if actual['revision']<=before['revision']:raise ValueError('enchant item revision did not advance')
        required.append(matched[0])
    for spent in (scroll,support):
        if spent is None:continue
        after=spent|{'count':spent['count']-1} if spent['count']>1 else None
        require(spent['id'],'consume',spent,after)
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
        if grant['before'] is not None:
            before,after=grant['before'],grant['after']
            same({k:v for k,v in before.items() if k not in ('count','revision')},{k:v for k,v in after.items() if k not in ('count','revision')})
            if after['revision']<=before['revision']:raise ValueError('refund revision did not advance')
        elif grant['after']['location']!='inventory' or grant['after']['slot'] is not None:
            raise ValueError('refund must enter inventory')
        required.append(grant)
    if len(required)!=len(deltas):raise ValueError('unexpected correlated enchant ledger row')


def evaluate(rows,recording,entity,gates,rules_hash,phase_specs=()):
    manifest=validate(rows,recording,rules_hash)
    results=[]
    enchant_errors=[]
    for row in rows[1:-1]:
        if row['entity']==entity and row['kind']=='item_formula' and row['data']['formula']=='enchant_outcome':
            try:enchant_evidence(row,rows)
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
