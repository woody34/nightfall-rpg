#!/usr/bin/env python3
"""Independent Fraction oracle for V6 item formulas and actual-input replay gates.

Expectations derive from pinned XML/Java and independent rational mathematics.
Generated angle certificates are untrusted validation targets only.
Unsupported source effects/conditions fail closed. Combat randomness is injected.
"""
from __future__ import annotations
import argparse
from collections import Counter
from fractions import Fraction as F
from functools import lru_cache
import hashlib
import json
from math import isqrt, factorial
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from types import SimpleNamespace
import xml.etree.ElementTree as ET

try:
    from . import item_behavior_oracle as behavior
except ImportError:
    import item_behavior_oracle as behavior

ROOT=Path(__file__).resolve().parents[1]
DP=Path('/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp')
GAME=DP.parent/'l2j-game'
DP_REV='3ca488dd2bd0bfaca43e378886a3c2e37968153a'
GAME_REV='abfde0490ac52a2106e884b7242c0276c44cdf76'
D='src/main/resources/data/'
G='src/main/java/com/l2jserver/gameserver/'
Q=1000000
OUT=ROOT/'fixtures/item-oracle.json'


def exact(value):
    if isinstance(value,float):raise ValueError('float input is forbidden')
    return F(value)


def trunc(value):return int(exact(value))


def q(value):return trunc(exact(value)*Q)


def rational(value):
    if isinstance(value,F):return {'numerator':value.numerator,'denominator':value.denominator}
    if isinstance(value,dict):return {str(k):rational(v) for k,v in value.items()}
    if isinstance(value,(list,tuple)):return [rational(v) for v in value]
    if isinstance(value,float):raise ValueError('float output is forbidden')
    return value


def checked_u32(value):
    if type(value) is not int or not 0<=value<=4294967295:raise ValueError('outside u32')
    return value


def validate_allocator(start,next_id,end):
    if any(type(v) is not int for v in [start,next_id,end]) or not 0<start<=next_id<=end<2**63:
        raise ValueError('invalid signed-bigint item range')
    return next_id<end


def enchant_increment(stat,grade,bodypart,weapon_type,enchant):
    checked_u32(enchant)
    safe=min(enchant,3);over=max(enchant-3,0)
    if stat in {'pDef','mDef'}:return F(safe+3*over)
    if stat=='sDef':return F(0)
    grade='S' if grade in {'S80','S84'} else grade
    if stat=='mAtk':unit={'NONE':2,'D':2,'C':3,'B':3,'A':3,'S':4}[grade]
    elif stat=='pAtk':
        # NONE/D applies its bow branch independently of bodypart.
        if grade in {'NONE','D'}:unit=4 if weapon_type in {'BOW','CROSSBOW'} else 2
        elif bodypart=='lrhand':unit=({'C':6,'B':6,'A':8,'S':10} if weapon_type in {'BOW','CROSSBOW'} else {'C':4,'B':4,'A':5,'S':6})[grade]
        else:unit={'C':3,'B':3,'A':4,'S':5}[grade]
    else:raise ValueError('unsupported FuncEnchant stat '+stat)
    return F(unit*(safe+2*over))


def enchant_hp(bonuses,enchant,onepiece=False):
    if enchant==0 or not bonuses:return 0
    checked_u32(enchant)
    return trunc(F(bonuses[min(enchant,len(bonuses))-1])*(F(3,2) if onepiece else 1))


def crystal_count(base,bonus,enchant,weapon):
    checked_u32(enchant)
    if enchant<=3:return base+bonus*enchant
    return base+bonus*((2*enchant-3) if weapon else (3*enchant-6))


def failure_refund(base,bonus,enchant,weapon,crystallizable=True,crystal_id=1458):
    if not crystallizable or not crystal_id:return 0
    return max(crystal_count(base,bonus,enchant,weapon)-(base+1)//2,1)


def enchant_chance(base,bonus=0,support=0):
    scaled=min(F(100),exact(base)+exact(bonus)+exact(support))*1000
    if scaled.denominator!=1 or scaled<0:raise ValueError('unrepresentable enchant chance')
    return int(scaled)


def enchant_outcome(base,bonus,support,residue,enchant,blessed=False,safe=False):
    if not 0<=residue<100000:raise ValueError('invalid enchant residue')
    success=residue<enchant_chance(base,bonus,support)
    if success:return {'outcome':'success','enchant':enchant+(exact(base)>0),'destroyed':False}
    if safe:return {'outcome':'safe_failure','enchant':enchant,'destroyed':False}
    if blessed:return {'outcome':'blessed_failure','enchant':0,'destroyed':False}
    return {'outcome':'destroyed','enchant':enchant,'destroyed':True}


def enchant_valid(current,max_enchant,blessed=False,support=False):
    return not (max_enchant and current>max_enchant) and not (blessed and support)


def max_load(con_bonus,weight_limit=1,multiplier=1,add=0):
    base=(exact(con_bonus)*69000*exact(weight_limit))//1
    return trunc(F(base)*exact(multiplier)+exact(add))


def overload(weight,maximum,bonus=1,previous_level=0,previous_overloaded=False):
    if maximum<=0:return {'per_mille':None,'level':previous_level,'overloaded':previous_overloaded,'changed':False}
    per=trunc(F((weight-bonus)*1000,maximum))
    level=next((i for i,t in enumerate([500,666,800,1000]) if per<t),4)
    changed=level!=previous_level
    return {'per_mille':per,'level':level,'overloaded':weight>maximum if changed else previous_overloaded,'changed':changed}


def grade_penalties(equipped,expertise,bonus=0):
    weapon=armor=0
    for item in sorted(equipped,key=lambda row:row['instance_id']):
        grade=item['grade']
        if item.get('arrow',False) or grade<=expertise:continue
        if item['weapon'] and grade>weapon:weapon=grade
        elif grade>armor:armor=grade
    return {'weapon':max(0,min(4,weapon-expertise-bonus)), 'armor':max(0,min(4,armor-expertise-bonus))}


def fist_spread(level,residue=None,template_radius=None):
    if type(level) is not int or level<1:raise ValueError('invalid player level')
    radius=5+isqrt(level) if template_radius is None else template_radius
    bound=2*radius+1
    if residue is not None and not 0<=residue<bound:raise ValueError('invalid spread residue')
    return {'radius':radius,'bound':bound,'offset':None if residue is None else residue-radius,
            'multiplier':None if residue is None else F(100+residue-radius,100)}


def pole_damage(formula,percent=85,primary=False):
    base=trunc(formula)
    return {'base':base,'percent':exact(percent),'damage':base if primary else trunc(F(base)*exact(percent)/100)}


def pole_sequence(candidates,maximum):
    percent=F(85);count=0;result=[]
    for candidate in candidates:
        if not candidate['eligible']:continue
        hit=candidate['hit'];row={'percent':percent,'hit':hit,'draws':[] if not hit else ['shield','critical','spread']}
        if hit:row.update(pole_damage(candidate['formula'],percent))
        result.append(row);percent/=F(23,20);count+=1
        if count>maximum:break
    return result


def dual_damage(formula,miss=False):
    base=0 if miss else trunc(formula)
    return {'base':base,'damage':base//2,'draws':[] if miss else ['shield','critical','spread']}


def dual_timing(attack_speed):
    if attack_speed<=0:raise ValueError('attack speed must be positive')
    attack_ms=trunc(F(500000)/exact(attack_speed));time_to_hit=attack_ms//2
    first_ms=time_to_hit//2;second_ms=time_to_hit
    return {'attack_ms':attack_ms,'time_to_hit_ms':time_to_hit,'first_ms':first_ms,'second_ms':second_ms,
        'first_ticks':(first_ms+99)//100,'second_ticks':(second_ms+99)//100}


def skill_caller_damage(formula,critical=False):
    return exact(formula)*(2 if critical else 1)


def bow_reuse(reuse_delay,attack_speed,weapon_reuse_modifier=1):
    if attack_speed<=0:raise ValueError('attack speed must be positive')
    return trunc(exact(reuse_delay)*333/exact(attack_speed))


def sqrt_bounds(value,bits=160):
    value=exact(value)
    if value<0:raise ValueError('negative root')
    scale=1<<bits
    lower=isqrt(value.numerator*scale*scale//value.denominator)
    if lower*lower*value.denominator==value.numerator*scale*scale:return F(lower,scale),F(lower,scale)
    return F(lower,scale),F(lower+1,scale)


def floor_a_plus_b_root(a,b,radicand):
    a,b,radicand=map(exact,[a,b,radicand])
    if b<0:raise ValueError('negative root coefficient is outside this caller')
    for bits in [64,128,256,512]:
        lower,upper=sqrt_bounds(radicand,bits)
        lo=(a+b*lower)//1;hi=(a+b*upper)//1
        if lo==hi:return int(lo),lower,upper
        # Exact integer tie can be certified without approximating sqrt.
        if b and ((F(hi)-a)/b)>=0 and ((F(hi)-a)/b)**2==radicand:return int(hi),lower,upper
    raise ValueError('root conversion could not be certified')


def bow_damage(formula,dx_milli,dy_milli):
    base=trunc(formula)
    radicand=dx_milli**2+dy_milli**2
    result,lower,upper=floor_a_plus_b_root(F(base)*F(4,5),F(base)*F(1,125000),radicand)
    return {'base':base,'distance_l2_lower':lower*F(32,1000),'distance_l2_upper':upper*F(32,1000),'damage':result}


def position(facing,relative):
    fx,fy=facing;vx,vy=relative;dot=fx*vx+fy*vy
    if not (fx*fx+fy*fy)*(vx*vx+vy*vy):return 'front'
    axial=4*dot*dot>=(fx*fx+fy*fy)*(vx*vx+vy*vy)
    return ('front' if dot>=0 else 'back') if axial else 'side'


def angle_contains(facing,relative,degrees,table):
    fx,fy=facing;vx,vy=relative
    if any(abs(v)>table['coordinate_bound_millitiles'] for v in [fx,fy,vx,vy]):raise ValueError('outside certified angle domain')
    if not 0<=degrees<=360:raise ValueError('angle outside integer table')
    row=table['angles'][degrees]
    if not row['certified']:raise ValueError('uncertified angle')
    if degrees==360 or relative==(0,0):return True
    dot=fx*vx+fy*vy;norm=(fx*fx+fy*fy)*(vx*vx+vy*vy)
    if not norm:return True
    if degrees==180:return dot>=0
    if degrees<180 and dot<0:return False
    if degrees>180 and dot>=0:return True
    value=F(dot*dot,norm);lo=F(row['cos2_lower']);hi=F(row['cos2_upper'])
    if lo==hi:return value>=lo if degrees<180 else value<=lo
    if lo<=value<=hi:raise ValueError('angle certificate violated')
    return value>hi if degrees<180 else value<lo


def shield(rate,dex_bonus=1,bow=False,eligible=True,facing=True,sigil=False,perfect_residue=None,normal_residue=None):
    effective=exact(rate)*exact(dex_bonus)
    if not eligible or sigil or not facing or effective<=F(1,Q):return {'outcome':'none','draws':[]}
    if bow:effective*=F(13,10)
    if perfect_residue is None or not 0<=perfect_residue<100:raise ValueError('missing perfect residue')
    draws=[{'site':'shield_perfect','bound':100,'residue':perfect_residue}]
    if 90<perfect_residue:return {'outcome':'perfect','draws':draws}
    if normal_residue is None or not 0<=normal_residue<100:raise ValueError('missing shield residue')
    draws.append({'site':'shield_normal','bound':100,'residue':normal_residue})
    return {'outcome':'normal' if normal_residue<effective else 'none','draws':draws}


def physical_damage(p_atk,defence,power=0,caller='auto',shot=False,proximity='front',shield_def=0,perfect=False,
                    critical=False,crit_dmg=1,crit_pos=1,def_crit_dmg=1,crit_add=0,def_crit_add=0,trait=1,spread=1,pvp=1,attribute=1,npc=1):
    if perfect:return F(1)
    p_atk,defence,power=map(exact,[p_atk,defence,power]);defence+=exact(shield_def)
    if defence<=0:raise ValueError('nonpositive defence')
    pos={'front':F(1),'side':F(11,10),'back':F(6,5)}[proximity]
    if caller=='auto':
        result=76*p_atk*(2 if shot else 1)*pos/defence
        if critical:result=2*exact(crit_dmg)*exact(crit_pos)*exact(def_crit_dmg)*result+exact(crit_add)*77/defence+exact(def_crit_add)
        result*=exact(trait)*exact(spread)*exact(pvp)*exact(attribute)*exact(npc)
    elif caller=='physical':
        result=77*(power+p_atk*(2 if shot else 1))/defence*pos*exact(pvp)*exact(trait)*exact(attribute)*exact(spread)*exact(npc)
    elif caller=='blow':
        base=77*(power+p_atk*(F(1458,1000) if shot else 1))/defence
        result=(base*exact(crit_dmg)*((exact(crit_pos)-1)/2+1)*exact(def_crit_dmg)*pos*exact(pvp)
                +exact(crit_add)*F(61,10)*77/defence+exact(def_crit_add))*exact(trait)*exact(attribute)*exact(spread)*exact(npc)
    else:raise ValueError('unknown physical caller')
    return max(F(1),result)


def magic_damage(power,m_atk,m_def,spirit=False,blessed=False,shield_def=0,perfect=False,critical=False,
                 attacker_player=True,target_player=False,is_pvp=None,skill_is_magic=True,
                 pvp_magic_def=1,pvp_skill_def=1,magic_failures=False,first_success=True,
                 second_success=None,level_difference=0,magic_crit_damage=1,spread=1,
                 pvp_magic_damage=1,pvp_skill_damage=1,attribute=1,npc=1):
    # Formulas.java:815-897: perfect block precedes all stat reads and draws.
    if perfect:return {'lower':F(1),'upper':F(1),'damage':1,'damage_q':Q,'branch':'perfect'}
    is_pvp=attacker_player and target_player if is_pvp is None else is_pvp
    defence=exact(m_def)+exact(shield_def)
    if is_pvp:defence*=exact(pvp_magic_def if skill_is_magic else pvp_skill_def)
    if defence<=0 or exact(m_atk)<0:raise ValueError('unsupported magic stats')
    factor=4 if blessed else 2 if spirit else 1
    coefficient=91*exact(power)/defence;constant=F(0);branch='success'
    if magic_failures and not first_success:
        branch='npc_failure'  # non-player failure leaves base damage unchanged
        if attacker_player:
            if second_success is None:raise ValueError('missing second magic success input')
            if second_success and level_difference<=9:coefficient/=2;branch='half'
            else:coefficient=F(0);constant=F(1);branch='resisted'
    elif critical:
        coefficient*=(F(5,2) if attacker_player and target_player else 3)*exact(magic_crit_damage)
        branch='critical'
    modifier=exact(spread)*(exact(pvp_magic_damage if skill_is_magic else pvp_skill_damage) if is_pvp else 1)*exact(attribute)*exact(npc)
    coefficient*=modifier;constant*=modifier
    damage,lo,hi=floor_a_plus_b_root(constant,coefficient,exact(m_atk)*factor)
    damage_q=floor_a_plus_b_root(constant*Q,coefficient*Q,exact(m_atk)*factor)[0]
    return {'lower':constant+coefficient*lo,'upper':constant+coefficient*hi,
            'damage':damage,'damage_q':damage_q,'branch':branch}


def cast_time(hit_time,casting_speed,spirit=False,blessed=False,static=False,is_magic=True):
    time=exact(hit_time)
    if not static:time*=333/exact(casting_speed)
    if is_magic and not static and (spirit or blessed):time*=F(3,5)
    time=trunc(time)
    return max(500,time) if hit_time>500 else time


def heal(power,m_atk,mp_consume2=0,mage=False,npc=False,grade='NONE',spirit=False,blessed=False,
         static=False,heal_effect=1,critical=False,is_magic=True,current_hp=0,recoverable_hp=None,remainder_q=0):
    if type(current_hp) is not int or type(remainder_q) is not int or not 0<=remainder_q<Q:
        raise ValueError('invalid restoration state')
    spirit=spirit and is_magic;blessed=blessed and is_magic
    static_bonus=F(0);coefficient=0
    if not static:
        if (mage or npc) and (spirit or blessed):
            coefficient=4 if blessed or npc else 2
            static_bonus=exact(mp_consume2)*(F(12,5) if blessed or npc else 1)
        else:
            coefficient={'S84':4,'S80':2}.get(grade,1)
            coefficient=coefficient*4 if blessed else coefficient+1
    multiplier=F(1) if static else exact(heal_effect)*(3 if critical and is_magic else 1)
    a=(exact(power)+static_bonus)*multiplier;b=F(0) if static else multiplier
    amount_q,lo,hi=floor_a_plus_b_root(a*Q,b*Q,coefficient*exact(m_atk))
    restoration_q=max(0,amount_q)
    current_q=current_hp*Q+remainder_q
    capacity_q=None if recoverable_hp is None else max(0,q(recoverable_hp)-current_q)
    applied_q=restoration_q if capacity_q is None else min(restoration_q,capacity_q)
    new_hp,new_remainder=divmod(current_q+applied_q,Q)
    if capacity_q is not None and applied_q>0 and applied_q==capacity_q:new_remainder=0
    return {'amount':F(applied_q,Q),'restoration_q':restoration_q,'applied_q':applied_q,
            'current_hp':new_hp,'remainder_q':new_remainder,'whole_credit':new_hp-current_hp,
            'coefficient':F(coefficient),'static_bonus':static_bonus,'root_lower':lo,'root_upper':hi}


def blow_success(chance,dex,proximity,residue,modifier=1):
    if not 0<=residue<1000:raise ValueError('invalid blow residue')
    rate=exact(chance)*exact(dex)*{'front':F(1),'side':F(3,2),'back':F(2)}[proximity]*exact(modifier)
    return residue<rate*10


def lethal(full,half,full_residue,half_residue,eligible=True,attribute=1,trait=1):
    if not eligible:return {'outcome':'none','draws':[]}
    scale=exact(attribute)*exact(trait)*10
    draws=[{'site':'lethal_full','bound':1000,'residue':full_residue}]
    if full_residue<exact(full)*scale:return {'outcome':'full','draws':draws}
    draws.append({'site':'lethal_half','bound':1000,'residue':half_residue})
    return {'outcome':'half' if half_residue<exact(half)*scale else 'none','draws':draws}


def retail_select(rows,residue):
    if not 0<=residue<10000:raise ValueError('invalid retail residue')
    threshold=F(10000);selected=0
    for option,chance in rows:
        if threshold>residue:
            threshold-=exact(chance)*100;selected=option
    if selected==0:raise ValueError('retail table leaves option zero')
    return selected


def retail_distribution(rows):
    # Exact interval enumeration equivalent to exhaustive residues, without O(N*10000).
    threshold=F(10000);counts=Counter()
    for index,(option,chance) in enumerate(rows):
        previous=threshold;threshold-=exact(chance)*100
        lower=0 if index==len(rows)-1 else max(0,-((-threshold.numerator)//threshold.denominator))
        upper=min(9999,-((-previous.numerator)//previous.denominator)-1)
        if upper>=lower:counts[option]+=upper-lower+1
    if sum(counts.values())!=10000:raise ValueError('retail row selector does not cover 10000 residues')
    return dict(sorted(counts.items()))


def category(residue,accessory=False):
    weights=[5500,3500,900,100] if accessory else [5500,3500,700,300]
    if not 0<=residue<10000:raise ValueError('invalid category residue')
    end=0
    for w in weights:
        end+=w
        if residue<end:return w//100
    raise ValueError('unreachable category')


def pack_options(first,second):
    if not 0<first<=65535 or not 0<second<=65535:raise ValueError('invalid option half')
    return checked_u32((second<<16)|first)


def cancel_fee(grade,crystals):
    schedules={'C':[(1720,95000),(2452,150000),(None,210000)],'B':[(1746,240000),(None,270000)],
               'A':[(2160,330000),(2824,390000),(None,420000)],'S':[(None,480000)],'S80':[(None,920000)],'S84':[(None,920000)]}
    for limit,cost in schedules[grade]:
        if limit is None or crystals<limit:return cost
    raise ValueError('no fee')


def set_shield_history(previous,mutation,chest_present,matching_shield,complete=False,required_part=False):
    if mutation=='remove' and (not chest_present or not matching_shield or required_part):return False
    if mutation=='equip_shield' and chest_present and matching_shield:return True
    if complete and matching_shield:return True
    return previous


def clamp_no_heal(current,new_max):return min(current,new_max)


def charge_transition(charges,use_spirit=False,use_soul=False,landed=True,unequip=False):
    result=set(charges)
    if unequip:return []
    if landed:
        if use_spirit:
            if 'blessed' in result:result.remove('blessed')
            else:result.discard('spirit')
        elif use_soul:result.discard('soul')
    return sorted(result)


def shot_payment(charges,kind,count,available,weapon_grade,shot_grade,toggle=False,reduction=(),residue=None):
    if kind not in {'soul','spirit','blessed'}:raise ValueError('unsupported shot kind')
    result={'charges':sorted(charges),'available':available,'toggle':toggle,'consumed':0,'draws':[],'outcome':'unavailable'}
    if count<=0 or weapon_grade is None:return result
    splus=lambda grade:'S' if grade in {'S80','S84'} else grade
    # Spirit handlers check existing charge first; SoulShots checks grade first.
    if kind!='soul' and kind in charges:result['outcome']='already_charged';return result
    if splus(weapon_grade)!=splus(shot_grade):result['outcome']='wrong_grade';return result
    if kind in charges:result['outcome']='already_charged';return result
    cost=count
    if kind=='soul' and reduction and reduction[1]>0:
        if residue is None or not 0<=residue<100:raise ValueError('missing reduction draw')
        result['draws']=[{'site':'soulshot_reduction','bound':100,'residue':residue}]
        if residue<reduction[0]:cost=reduction[1]
    if available<cost:result['toggle']=False;result['outcome']='insufficient';return result
    result.update(charges=sorted(set(charges)|{kind}),available=available-cost,consumed=cost,outcome='charged')
    return result


def shared_shot_bundle(charges,hits,use_spirit=False,use_soul=False,canceled=False,unequip=False):
    # I2: one paid bundle is frozen for every recipient, then discharged once.
    frozen=sorted(charges)
    return {'recipient_charges':[frozen for hit in hits if hit and not canceled],
            'after':charge_transition(frozen,use_spirit,use_soul,any(hits) and not canceled,unequip),
            'payments':0}


def winning_providers(providers):
    if len({p['key'] for p in providers})!=len(providers):raise ValueError('duplicate provider key')
    winners={}
    for provider in sorted(providers,key=lambda p:p['key']):
        sid=provider['skill_id']
        if sid not in winners or provider['rank']>winners[sid]['rank']:winners[sid]=provider
    return [(sid,p['rank'],p['sequence']) for sid,p in sorted(winners.items())]


def consumable_use(definition,tick,available,item_deadlines=None,skill_deadlines=None,main_casting=False,condition_context=None):
    available={int(key):count for key,count in available.items()}
    item_deadlines={int(key):deadline for key,deadline in (item_deadlines or {}).items()}
    skill_deadlines={int(key):deadline for key,deadline in (skill_deadlines or {}).items()}
    key=definition['reuse_group'] if definition['reuse_group']>0 else definition['template_id']
    if item_deadlines.get(key,0)>tick:return {'accepted':False,'reason':'item_reuse','consumed':{},'lanes':[]}
    consumed=Counter();lanes=[]
    for skill in definition['skills']:
        if skill.get('blockers'):raise ValueError('unsupported consumable skill')
        for condition in skill.get('conditions',[]):
            if condition_context is None:raise ValueError('missing consumable condition context')
            if not condition_matches(ET.fromstring(condition),condition_context):
                return {'accepted':False,'reason':'condition','consumed':{},'lanes':[]}
        if skill_deadlines.get(skill['id'],0)>tick:return {'accepted':False,'reason':'skill_reuse','consumed':{},'lanes':[]}
        if main_casting and definition['etcitem_type'] not in {'POTION','ELIXIR','SCROLL'} and not definition['ex_immediate']:
            return {'accepted':False,'reason':'casting_guard','consumed':{},'lanes':[]}
        if skill['item_consume_id']>0:consumed[skill['item_consume_id']]+=skill['item_consume_count']
        lanes.append('item' if skill['simultaneous'] or ((definition['immediate'] or definition['ex_immediate']) and skill['static']) else 'main')
    if not any(skill['item_consume_id']>0 for skill in definition['skills']) and definition['default_action'] in {'CAPSULE','SKILL_REDUCE'} and definition['immediate']:
        consumed[definition['template_id']]+=1
    if any(available.get(sid,0)<count for sid,count in consumed.items()):return {'accepted':False,'reason':'items','consumed':{},'lanes':[]}
    if definition['reuse_delay_ms']>0:item_deadlines[key]=tick+(definition['reuse_delay_ms']+99)//100
    for skill in definition['skills']:
        if skill['reuse_delay_ms']>0:skill_deadlines[skill['id']]=tick+(skill['reuse_delay_ms']+99)//100
    return {'accepted':True,'consumed':dict(consumed),'lanes':lanes,'item_deadlines':item_deadlines,
            'skill_deadlines':skill_deadlines,'main_cast_preserved':all(lane=='item' for lane in lanes)}


def consumable_cast_sequence(starts,caster_ordinal,caster_id):
    """V6 I5/2.1 schedule: prepaid starts, one-tick minimum, shared total order."""
    available={int(k):v for k,v in starts[0]['available'].items()}
    item_deadlines={};skill_deadlines={};main=None;pending=[];trace=[];consumed=Counter()
    for start in starts:
        definition=start['definition'];tick=start['tick']
        result=consumable_use(definition,tick,available,item_deadlines,skill_deadlines,
                              main_casting=main is not None,condition_context=start.get('condition_context'))
        if not result['accepted']:raise ValueError('rejected sequence start')
        if len(definition['skills'])!=1:raise ValueError('sequence requires one source skill')
        skill=definition['skills'][0];lane=result['lanes'][0]
        if lane=='main' and main is not None:raise ValueError('occupied main lane')
        hit_ms=cast_time(skill['hit_time_ms'],333,static=skill['static'])
        impact_ms=behavior.cast_impact_delay(hit_ms)
        cast={'cast_id':start['cast_id'],'lane':lane,'skill_id':skill['id'],
              'start_tick':tick,'completion_tick':tick+max(1,(impact_ms+99)//100),
              'caster_ordinal':caster_ordinal,'caster_id':caster_id,'prepaid':True}
        for sid,count in result['consumed'].items():available[sid]-=count;consumed[sid]+=count
        item_deadlines=result['item_deadlines'];skill_deadlines=result['skill_deadlines']
        if lane=='main':main=cast
        pending.append(cast)
        trace.append({'event':'start','tick':tick,'cast':dict(cast),'consumed':result['consumed'],
                      'inventory':dict(available),'main_cast':None if main is None else dict(main)})
    if len({c['completion_tick'] for c in pending})!=1:raise ValueError('casts are not due together')
    if len({c['cast_id'] for c in pending})!=len(pending):raise ValueError('duplicate cast id')
    ordered=sorted(pending,key=lambda c:(c['completion_tick'],c['caster_ordinal'],c['caster_id'],c['cast_id']))
    for cast in ordered:
        if cast['lane']=='main':main=None
        trace.append({'event':'complete','tick':cast['completion_tick'],'cast_id':cast['cast_id'],
                      'lane':cast['lane'],'consumed':{},'inventory':dict(available),
                      'main_cast':None if main is None else dict(main)})
    return {'trace':trace,'completion_order':[c['cast_id'] for c in ordered],
            'total_consumed':dict(consumed),'completion_consumed':{},'inventory':available,
            'main_cast':main,'item_casts':[],'item_deadlines':item_deadlines,'skill_deadlines':skill_deadlines}


class Source:
    def __init__(self,dp=DP,game=GAME):
        self.dp,self.game=dp,game;self.hashes={};self.items={};self.skills={};self.sets=[]
        for root,expected in [(dp,DP_REV),(game,GAME_REV)]:
            head=subprocess.run(['git','-C',str(root),'rev-parse','HEAD'],check=True,capture_output=True,text=True).stdout.strip()
            if head!=expected:raise ValueError('oracle source revision mismatch')
            paths=[D,'src/main/java/com/l2jserver/datapack/handlers/'] if root==dp else [G,'src/main/resources/config/character.properties','src/main/resources/config/customs.properties']
            dirty=subprocess.run(['git','-C',str(root),'status','--porcelain','--untracked-files=all','--ignored=matching','--',*paths],check=True,capture_output=True,text=True).stdout.strip()
            if dirty:raise ValueError('dirty oracle source inputs')
        for path in sorted((dp/(D+'stats/items')).glob('*.xml')):
            file=path.relative_to(dp).as_posix();text=self.read(file)
            lines={int(m[1]):text[:m.start()].count('\n')+1 for m in re.finditer(r'<item id="(\d+)"',text)}
            for item in ET.fromstring(text).findall('item'):
                sid=int(item.get('id'));self.items[sid]=(file,lines[sid],item)
        for path in sorted((dp/(D+'stats/skills')).rglob('*.xml')):
            file=path.relative_to(dp).as_posix();text=self.read(file)
            for skill in ET.fromstring(text).findall('skill'):self.skills[int(skill.get('id'))]=(file,skill)
        for path in sorted((dp/(D+'stats/armorsets')).glob('*.xml')):
            file=path.relative_to(dp).as_posix();text=self.read(file)
            for row in ET.fromstring(text).findall('set'):self.sets.append((file,row))
        # D84: pinned L2PcInstance.findFistsWeaponItem class ranges -> fists template.
        pc=self.read(G+'model/actor/instance/L2PcInstance.java',True)
        body=re.search(r'public L2Weapon findFistsWeaponItem\(int classId\) \{(.*?)\n\t\treturn weaponItem;',pc,re.S)
        if body is None:raise ValueError('missing findFistsWeaponItem')
        self.fists=[(int(lo,16),int(hi,16),int(tid)) for lo,hi,tid in re.findall(
            r'\(classId >= 0x([0-9a-f]+)\) && \(classId <= 0x([0-9a-f]+)\)\) \{[^{}]*?getTemplate\((\d+)\)',body[1])]
        if len(self.fists)!=9:raise ValueError('unexpected fists ranges')
        class_ids=self.read(G+'model/base/ClassId.java',True)
        self.races={int(sid,16):race for sid,race in re.findall(r'\w+\((0x[0-9a-fA-F]+), (?:true|false), (?:true, )?Race\.(\w+),',class_ids)}
        self.bonuses={s.tag:{int(r.get('value')):F(r.get('bonus')) for r in s.findall('stat')}
                      for s in self.xml(D+'stats/statBonus.xml')}
        self.hp={row.get('grade'):[int(b.text) for b in row.findall('bonus')]
                 for row in self.xml(D+'stats/enchantHPBonus.xml').findall('enchantHP')}
        self.rates={n.get('name'):[(int(c.get('enchant').split('-')[0]),int(c.get('enchant').split('-')[-1]),F(c.get('chance')))
                   for c in n.findall('current')] for n in self.xml(D+'enchantItemGroups.xml').findall('enchantRateGroup')}
        text=self.read(G+'model/items/type/CrystalType.java',True)
        self.grades={m[1]:{'id':int(m[2]),'crystal_id':int(m[3]),'armor_bonus':int(m[4]),'weapon_bonus':int(m[5])}
                     for m in re.finditer(r'^\s*(\w+)\((\d+), (\d+), (\d+), (\d+)\)',text,re.M)}
        for file in ['model/stats/functions/FuncEnchant.java','model/stats/functions/FuncEnchantHp.java',
            'model/items/L2Item.java','model/items/L2EtcItem.java','model/conditions/ConditionPlayerLevelRange.java','model/conditions/ConditionPlayerRace.java','model/actor/L2Character.java','model/actor/instance/L2PcInstance.java',
            'model/stats/Formulas.java','model/stats/BaseStats.java','network/clientpackets/RequestEnchantItem.java','network/clientpackets/RequestRefineCancel.java',
            'network/clientpackets/AbstractRefinePacket.java','datatables/AugmentationData.java','model/itemcontainer/PcInventory.java',
            'model/stats/functions/formulas/FuncPDefMod.java','model/stats/functions/formulas/FuncMDefMod.java',
            'model/actor/templates/L2PcTemplate.java','data/xml/impl/PlayerTemplateData.java','datatables/ItemTable.java',
            'enums/StatFunction.java']:
            self.read(G+file,True)
        for path in sorted((game/(G+'model/stats/functions/formulas')).glob('*.java')):
            self.read(path.relative_to(game).as_posix(),True)
        for file in ['effecthandlers/instant/Heal.java','itemhandlers/SoulShots.java','itemhandlers/SpiritShot.java','itemhandlers/BlessedSpiritShot.java','itemhandlers/ItemSkillsTemplate.java','itemhandlers/ManaPotion.java']:
            self.read('src/main/java/com/l2jserver/datapack/handlers/'+file)
        self.read('src/main/resources/config/character.properties',True)
        self.read('src/main/resources/config/customs.properties',True)
    def read(self,file,game=False):
        raw=((self.game if game else self.dp)/file).read_bytes();self.hashes[('game/' if game else 'datapack/')+file]=hashlib.sha256(raw).hexdigest()
        return raw.decode()
    def xml(self,file):return ET.fromstring(self.read(file))
    def properties(self,sid):return {n.get('name'):n.get('val') for n in self.items[sid][2].findall('set')}
    def citation(self,sid):
        file,line,_=self.items[sid];return f'{file}:{line}'
    def chance(self,name,enchant):
        for lower,upper,chance in self.rates[name]:
            if lower<=enchant<=upper:return chance
        raise ValueError('enchant outside source group')
    def choose(self,predicate):
        for sid,(_,_,node) in sorted(self.items.items()):
            p=self.properties(sid)
            if node.get('type')=='Weapon' and predicate(sid,node,p):return sid
        raise ValueError('no eligible source template')


def player_level_range(condition,rank,level):
    if set(condition)!={'kind','ranges'} or condition['kind']!='player_level_range':raise ValueError('invalid level-range node')
    if type(rank) is not int or not 1<=rank<=len(condition['ranges']) or type(level) is not int:raise ValueError('invalid level-range input')
    bounds=condition['ranges'][rank-1]
    if set(bounds)!={'minimum','maximum'} or any(type(v) is not int for v in bounds.values()) or bounds['minimum']>bounds['maximum']:
        raise ValueError('invalid level-range bounds')
    return bounds['minimum']<=level<=bounds['maximum']


def condition_matches(node,context):
    if node.tag in {'and','cond'}:return all(condition_matches(c,context) for c in node)
    if node.tag=='or':return any(condition_matches(c,context) for c in node)
    if node.tag=='not':return not any(condition_matches(c,context) for c in node)
    if node.tag=='player' and set(node.attrib) in ({'canEscape'},{'canResurrect'}):
        key=next(iter(node.attrib))
        return context[key]==(node.get(key)=='true')
    if node.tag=='player' and set(node.attrib)=={'insideZoneId'}:
        return bool(set(map(int,node.get('insideZoneId').split(','))) & set(context['inside_zone_ids']))
    if node.tag=='player' and set(node.attrib)=={'races'}:
        return context['race'] in node.get('races').split(',')
    if node.tag=='player' and set(node.attrib)=={'levelRange'}:
        parts=node.get('levelRange').split(';')
        if len(parts)!=2 or not all(p.isdigit() for p in parts):raise ValueError('unresolved level range')
        minimum,maximum=map(int,parts)
        if minimum>maximum:raise ValueError('inverted level range')
        return minimum<=context['level']<=maximum
    if node.tag=='using' and set(node.attrib)=={'kind'}:return context['weapon_type'] in node.get('kind').split(',') or context.get('armor_type') in node.get('kind').split(',')
    if node.tag=='using' and set(node.attrib)=={'slot'}:return node.get('slot') in context['slots']
    if node.tag=='player' and set(node.attrib) in ({'resting'},{'standing'},{'moving'},{'running'}):
        key=next(iter(node.attrib));states={'resting':context.get('sitting',False),'moving':context.get('moving',False),
            'standing':not (context.get('sitting',False) or context.get('moving',False)),'running':context.get('running',True)}
        return states[key]==(node.get(key)=='true')
    raise ValueError('unsupported oracle fold condition '+node.tag+str(node.attrib))


def gear_stats(src,class_file,level,equipment,skill_refs=(),base_deltas=None,base_delta_sequence=-1):
    """Equipment entries are (template_id, enchant, concrete slot, install sequence).

    Skills are explicit winning providers; no synthetic learned mastery or expertise.
    Full replay callers supply the winning sources and retained install sequences.
    """
    root=src.xml(class_file);static=root.find('staticData');gain=next(n for n in root.findall('lvlUpgainData/level') if int(n.get('val'))==level)
    base={k:int(static.findtext('base'+k)) for k in ['STR','DEX','CON','INT','WIT','MEN']}
    lm=F(level+89,100)
    pslots={n.tag:F(n.text) for n in static.find('basePDef')}
    mslots={n.tag:F(n.text) for n in static.find('baseMDef')};mslots['lfinger']=mslots['rfinger']
    values={'pAtk':F(static.findtext('basePAtk')),'mAtk':F(static.findtext('baseMAtk')),'pDef':sum(pslots.values()),
        'mDef':sum(mslots.values()),'critRate':F(static.findtext('baseCritRate')),'pAtkSpd':F(static.findtext('basePAtkSpd')),
        'maxHp':F(gain.findtext('hp')),'maxMp':F(gain.findtext('mp')),'maxCp':F(gain.findtext('cp')),
        'mAtkSpd':F(static.findtext('baseMAtkSpd','333')),'runSpd':F(static.findtext('baseMoveSpd/run')),
        'regHp':F(gain.findtext('hpRegen'))*lm,
        'regMp':F(gain.findtext('mpRegen'))*lm,
        'regCp':F(gain.findtext('cpRegen'))*lm,
        'atkCountMax':F(1),'pAtkAngle':F(static.findtext('baseDamRange/width')),'pAtkRange':F(static.findtext('baseAtkRange')),
        'critDmg':F(1),'critDmgAdd':F(0),'sDef':F(0),'rShld':F(0),'shieldDefAngle':F(0),'rEvas':F(0),'weightLimit':F(69000)}
    slots={row[2] for row in equipment};weapon_type='FIST';armor_type='NONE'
    fists=None
    if not any(slot=='rhand' for sid,enchant,slot,seq in equipment):
        # D84: an unarmed player's active weapon is the class fists template
        # (L2PcInstance.findFistsWeaponItem / getActiveWeaponItem): its reach, angle and radius.
        fists=fists_template(src,int(root.findtext('classId')))
        fp=src.properties(fists)
        values['pAtkRange']=F(fp.get('attack_range','40'))
        damage_parts=fp.get('damage_range','').split(';')
        values['pAtkAngle']=F(damage_parts[3]) if len(damage_parts)>=4 and damage_parts[3].isdigit() else F(120)
    for sid,enchant,slot,seq in equipment:
        p=src.properties(sid)
        if slot=='rhand':
            weapon_type=p.get('weapon_type','NONE')
            values['pAtkRange']=F(p.get('attack_range','40'))
            damage_parts=p.get('damage_range','').split(';')
            values['pAtkAngle']=F(damage_parts[3]) if len(damage_parts)>=4 and damage_parts[3].isdigit() else F(120)
        if slot=='chest':armor_type=p.get('armor_type','NONE')
    context={'slots':slots,'weapon_type':weapon_type,'armor_type':armor_type,'level':level,'race':src.races[int(root.findtext('classId'))]};funcs=[];hp_bonus=0;traits=[]
    if len({sid for sid,rank,seq in skill_refs})!=len(skill_refs):raise ValueError('duplicate winning skill provider')
    orders={'set':0,'enchant':0,'add':30,'sub':30,'mul':20,'div':20}
    for sid,enchant,slot,seq in equipment:
        file,line,item=src.items[sid];p=src.properties(sid)
        if item.find('cond') is not None and not condition_matches(item.find('cond'),context):raise ValueError('equipment condition failed')
        for ordinal,node in enumerate(item.findall('for/*')):
            if node.tag not in orders:raise ValueError('unsupported oracle function '+node.tag)
            stat=node.get('stat');value=F(node.get('val','0'))
            if node.tag=='enchant':value=enchant_increment(stat,p.get('crystal_type','NONE'),p.get('bodypart','none'),p.get('weapon_type','NONE'),enchant)
            funcs.append((int(node.get('order',orders[node.tag])),seq,(0,ordinal),stat,'add' if node.tag=='enchant' else node.tag,value,list(node)))
        if item.get('type')=='Armor' and p.get('crystal_type','NONE')!='NONE' and p.get('bodypart') in {'chest','legs','feet','head','gloves','back','fullarmor','onepiece','shirt','underwear','lhand','belt','waist'}:
            hp_bonus+=enchant_hp(src.hp.get('S' if p['crystal_type'] in {'S80','S84'} else p['crystal_type'],[]),enchant,p.get('bodypart') in {'fullarmor','onepiece'})
    for sid,rank,seq in skill_refs:
        file,skill=src.skills[sid]
        if not 1<=rank<=int(skill.get('levels','1')):raise ValueError('invalid provider rank')
        if skill.get('id') is None:raise ValueError('missing skill identity')
        tables={n.get('name'):n.text.split() for n in skill.findall('table')}
        def resolved_conditions(nodes):
            result=[]
            for node in nodes:
                copied=ET.fromstring(ET.tostring(node))
                for child in copied.iter():
                    for key,value in child.attrib.items():
                        if value.startswith('#'):child.set(key,tables[value][rank-1])
                result.append(copied)
            return result
        if not all(condition_matches(node,context) for node in resolved_conditions(skill.findall('cond'))):continue
        for effect_ordinal,effect in enumerate(skill.findall('effects/effect')):
            name=effect.get('name')
            if name in {'DefenceTrait','AttackTrait'}:
                traits.append({'skill_id':sid,'rank':rank,'sequence':seq,'kind':name,'params':{k:F(v) for n in effect.findall('param') for k,v in n.attrib.items()}})
                continue
            if name in {'MaxHp','MaxMp','MaxCp'}:
                params={k:v for n in effect.findall('param') for k,v in n.attrib.items()};value=params.get('power','0')
                value=F(tables[value][rank-1] if value.startswith('#') else value)
                operation='mul' if params.get('type','DIFF')=='PER' else 'add'
                if operation=='mul':value=1+value/100
                funcs.append((1,seq,(effect_ordinal,0),name[0].lower()+name[1:],operation,value,[]))
                continue
            if name not in {'Buff','Passive'}:raise ValueError('unsupported passive oracle effect '+name)
            for ordinal,node in enumerate(effect):
                if node.tag not in orders:raise ValueError('unsupported passive oracle child '+node.tag)
                value=node.get('val');value=tables[value][rank-1] if value.startswith('#') else value
                funcs.append((int(node.get('order',orders[node.tag])),seq,(effect_ordinal,ordinal),node.get('stat'),node.tag,F(value),resolved_conditions(list(node))))
    for ordinal,(key,delta) in enumerate((base_deltas or {}).items()):
        if key.upper() not in base:raise ValueError('unknown set attribute')
        funcs.append((1,base_delta_sequence,(0,ordinal),key.upper(),'add',exact(delta),[]))
    trace=[]
    ordered=sorted(funcs,key=lambda row:(row[0],row[1],row[2]))
    for order,seq,ordinal,stat,op,value,conditions in ordered:
        if stat not in base:continue
        if not all(condition_matches(c,context) for c in conditions):continue
        old=F(base[stat])
        if op=='set':new=value
        elif op=='add':new=old+value
        elif op=='sub':new=old-value
        elif op=='mul':new=old*value
        elif op=='div':new=old/value
        else:raise ValueError('unsupported attribute operation')
        if new.denominator!=1 or int(new) not in src.bonuses[stat]:raise ValueError('unsupported effective attribute')
        base[stat]=int(new)
        trace.append({'order':order,'sequence':seq,'handler_ordinal':ordinal[0],'ordinal':ordinal[1],
                      'stat':stat,'operation':op,'before':old,'operand':value,'after':new})
    funcs=[row for row in funcs if row[3] not in base]
    bonus={k:src.bonuses[k][v] for k,v in base.items()}
    values['regHp']=max(F(1),values['regHp']*bonus['CON'])
    values['regMp']=max(F(1),values['regMp']*bonus['MEN'])
    values['regCp']=max(F(1),values['regCp']*bonus['CON'])
    values['weightLimit']=values['weightLimit']*bonus['CON']//1
    psub=msub=F(0)
    for stat in pslots:
        concrete={'underwear':'under','cloak':'cloak'}.get(stat,stat)
        if concrete in slots or stat=='legs' and any(src.properties(s)['bodypart'] in {'fullarmor','onepiece'} for s,e,slot,seq in equipment if slot=='chest'):
            psub+=pslots[stat]
    for stat in mslots:
        if stat in slots:msub+=mslots[stat]
    lm=F(level+89,100)
    builtin={'pAtk':bonus['STR']*lm,'mAtk':bonus['INT']**2*lm**2,'pDef':lm,'mDef':bonus['MEN']*lm,
             'critRate':bonus['DEX']*10,'pAtkSpd':bonus['DEX'],'mAtkSpd':bonus['WIT'],'runSpd':bonus['DEX'],'maxHp':bonus['CON'],'maxCp':bonus['CON'],'maxMp':bonus['MEN']}
    for stat,value in builtin.items():funcs.append((1,-1,(0,0),stat,'submul' if stat in {'pDef','mDef'} else 'mul',value,[]))
    evasion_add=F(level)+(F(level-69)*(F(6,5) if level>=78 else 1) if level>=70 else 0)
    evasion=floor_a_plus_b_root(evasion_add,6,base['DEX'])[0]
    funcs.append((1,-1,(0,0),'rEvas','evasion',F(evasion),[]))
    accuracy_add=F(level+(level-76 if level>77 else 0)+(level-69 if level>69 else 0))
    funcs.append((1,-1,(0,0),'accCombat','accuracy',accuracy_add,[]))
    funcs.append((40,-1,(0,0),'maxHp','add',F(hp_bonus),[]))
    accuracy_a=accuracy_b=F(0)
    for order,seq,ordinal,stat,op,value,conditions in sorted(funcs,key=lambda row:(row[0],row[1],row[2])):
        if not all(condition_matches(c,context) for c in conditions):continue
        if stat=='accCombat':
            old={'rational':accuracy_a,'root_coefficient':accuracy_b,'radicand':base['DEX']}
            if op=='accuracy':accuracy_a+=value;accuracy_b+=6
            elif op=='add':accuracy_a+=value
            elif op=='sub':accuracy_a-=value
            elif op=='mul':accuracy_a*=value;accuracy_b*=value
            elif op=='div':accuracy_a/=value;accuracy_b/=value
            elif op=='set':accuracy_a=value;accuracy_b=F(0)
            else:raise ValueError('unsupported accuracy operation')
            new={'rational':accuracy_a,'root_coefficient':accuracy_b,'radicand':base['DEX']}
            trace.append({'order':order,'sequence':seq,'handler_ordinal':ordinal[0],'ordinal':ordinal[1],'stat':stat,'operation':op,'before':old,'operand':value,'after':new})
            continue
        if stat not in values:raise ValueError('unsupported oracle stat '+str(stat))
        old=values[stat]
        if op in {'set','evasion'}:new=value
        elif op=='add':new=old+value
        elif op=='sub':new=old-value
        elif op=='mul':new=old*value
        elif op=='submul':new=(old-(psub if stat=='pDef' else msub))*value
        elif op=='div':new=old/value
        else:raise ValueError('unknown fold operation')
        values[stat]=new;trace.append({'order':order,'sequence':seq,'handler_ordinal':ordinal[0],'ordinal':ordinal[1],'stat':stat,'operation':op,'before':old,'operand':value,'after':new})
    weapon=next((sid for sid,e,slot,seq in equipment if slot=='rhand'),None)
    radius=int(src.properties(weapon if weapon is not None else fists).get('random_damage','0'))
    qvalues={k:q(v) for k,v in values.items()};getters={k:trunc(v) for k,v in values.items()}
    qvalues['accCombat']=floor_a_plus_b_root(accuracy_a*Q,accuracy_b*Q,base['DEX'])[0]
    getters['accCombat']=floor_a_plus_b_root(accuracy_a,accuracy_b,base['DEX'])[0]
    return {'exact':values,'root_expressions':{'accCombat':{'rational':accuracy_a,'root_coefficient':accuracy_b,'radicand':base['DEX']}},'q':qvalues,'getters':getters,
            'trace':trace,'base_stats':base,'weapon_type':weapon_type,'random_damage':radius,'spread_bound':2*radius+1,
            'enchant_hp':hp_bonus,'traits':traits,'weight':sum(int(src.properties(sid).get('weight','0')) for sid,e,slot,seq in equipment)}


def fists_template(src,class_id):
    """The class fists template from the pinned findFistsWeaponItem ranges (D84)."""
    for lo,hi,tid in src.fists:
        if lo<=class_id<=hi:return tid
    raise ValueError('class without a fists template')


def defence_slot_stats(src,class_file,level,occupied_slots,chest_bodypart='none'):
    """Independent player FuncPDefMod/FuncMDefMod projection, before item additions."""
    static=src.xml(class_file).find('staticData')
    pslots={('under' if n.tag=='underwear' else n.tag):int(n.text) for n in static.find('basePDef')}
    mslots={n.tag:int(n.text) for n in static.find('baseMDef')}
    mslots['lfinger']=mslots['rfinger']
    occupied=set(occupied_slots)
    if len(occupied)!=len(occupied_slots) or not occupied<=pslots.keys()|mslots.keys():raise ValueError('invalid defence slot')
    poccupied=set(occupied)&pslots.keys()
    if 'chest' in occupied and chest_bodypart in {'fullarmor','onepiece'}:poccupied.add('legs')
    moccupied=occupied&mslots.keys()
    psub=sum(pslots[slot] for slot in poccupied);msub=sum(mslots[slot] for slot in moccupied)
    lm=F(level+89,100);men=src.bonuses['MEN'][int(static.findtext('baseMEN'))]
    pdef=(sum(pslots.values())-psub)*lm;mdef=(sum(mslots.values())-msub)*men*lm
    return {'p_def_base':sum(pslots.values()),'m_def_base':sum(mslots.values()),
            'p_def_subtracted_slots':sorted(poccupied),'m_def_subtracted_slots':sorted(moccupied),
            'p_def_subtraction':psub,'m_def_subtraction':msub,'level_modifier':lm,'men_bonus':men,
            'p_def':pdef,'m_def':mdef,'p_def_q':q(pdef),'m_def_q':q(mdef)}


def unarmed_reach(src,class_file,weapon=None):
    """Independent CharStat.getPhysicalAttackRange/getAttackType base (no transform, no pAtkRange mods)."""
    static=src.xml(class_file).find('staticData')
    if weapon is None:
        # L2CharTemplate loader defaults: baseAtkRange 40, baseAtkType FIST, when a node is absent.
        return {'weapon':None,'attack_type':static.findtext('baseAtkType','FIST'),
                'base_attack_range':int(static.findtext('baseAtkRange','40')),'attack_angle':120}
    p=src.properties(weapon)
    angle=p.get('damage_range','').split(';')
    return {'weapon':weapon,'attack_type':p.get('weapon_type','NONE'),'base_attack_range':int(p.get('attack_range','40')),
            'attack_angle':int(angle[3]) if len(angle)>=4 and angle[3].isdigit() else 120}


def class_combat_cases(src):
    """Every class whose classList root is a supported base class, selected from the graph."""
    graph={int(n.get('classId')):(int(n.get('parentClassId')) if n.get('parentClassId') is not None else None)
           for n in src.xml(D+'stats/chars/classList.xml').findall('class')}
    base_ids={cid for cid,parent in graph.items() if parent is None and src.races.get(cid) in {'HUMAN','ELF','DARK_ELF','ORC','DWARF'}}
    if len(base_ids)!=9:raise ValueError('unexpected supported base classes')
    def root(cid):
        while graph.get(cid) is not None:cid=graph[cid]
        return cid
    for path in sorted((src.dp/(D+'stats/chars/baseStats')).glob('*.xml')):
        file=path.relative_to(src.dp).as_posix();cid=int(src.xml(file).findtext('classId'))
        if root(cid) in base_ids:yield f'class-combat-{cid}-unarmed',{'class_file':file,'base_class_id':root(cid)}


def class_slot_cases(src):
    # Select base classes independently from the pinned class graph (parent absent).
    graph=src.xml(D+'stats/chars/classList.xml')
    base_ids={int(n.get('classId')) for n in graph.findall('class') if n.get('parentClassId') is None and src.races.get(int(n.get('classId'))) in {'HUMAN','ELF','DARK_ELF','ORC','DWARF'}}
    if len(base_ids)!=9:raise ValueError('unexpected supported base classes')
    for path in sorted((src.dp/(D+'stats/chars/baseStats')).glob('*.xml')):
        file=path.relative_to(src.dp).as_posix();root=src.xml(file);cid=int(root.findtext('classId'))
        if cid not in base_ids:continue
        physical=['chest','legs','head','feet','gloves','under','cloak'];magical=['rear','lear','rfinger','lfinger','neck']
        cases=[('empty',[],'none')]+[(slot,[slot],'none') for slot in physical+magical]
        cases += [('all',physical+magical,'none'),('fullarmor',['chest'],'fullarmor'),
                  ('onepiece',['chest'],'onepiece'),('fullarmor-legs',['chest','legs'],'fullarmor')]
        for name,occupied,body in cases:
            yield f'class-slots-{cid}-{name}',{'class_file':file,'level':84,'occupied_slots':occupied,'chest_bodypart':body}


def consumable_definition(src,template_id):
    properties=src.properties(template_id);result=[]
    for reference in properties.get('item_skill','').split(';'):
        if not reference:continue
        sid,rank=map(int,reference.split('-'))
        if sid not in src.skills:
            result.append({'id':sid,'rank':rank,'blockers':['missing_source_skill']});continue
        file,skill=src.skills[sid]
        tables={n.get('name'):n.text.split() for n in skill.findall('table')}
        def resolve(value):return tables[value][rank-1] if value.startswith('#') else value
        sets={n.get('name'):resolve(n.get('val')) for n in skill.findall('set')}
        conditions=[]
        for cond in skill.findall('cond'):
            copied=ET.fromstring(ET.tostring(cond))
            for node in copied.iter():
                for key,value in node.attrib.items():node.set(key,resolve(value))
            conditions.append(ET.tostring(copied,encoding='unicode'))
        effects=[]
        for effect in skill.findall('effects/effect'):
            effects.append({'name':effect.get('name'),'params':{key:behavior.exact_text(resolve(value)) for n in effect.findall('param') for key,value in n.attrib.items()}})
        result.append({'id':sid,'rank':rank,'hit_time_ms':int(sets.get('hitTime','0')),
            'static':sets.get('isMagic')=='2','simultaneous':sets.get('simultaneousCast','false')=='true',
            'reuse_delay_ms':int(sets.get('reuseDelay','0')),
            'item_consume_id':int(sets.get('itemConsumeId','0')),'item_consume_count':int(sets.get('itemConsumeCount','0')),'conditions':conditions,
            'effects':effects,'source':file})
    return {'template_id':template_id,'handler':properties.get('handler',''),
        'default_action':properties.get('default_action','NONE'),'etcitem_type':properties.get('etcitem_type','NONE'),
        'reuse_delay_ms':int(properties.get('reuse_delay','0')),'reuse_group':int(properties.get('shared_reuse_group','0')),
        'immediate':properties.get('immediate_effect','false')=='true','ex_immediate':properties.get('ex_immediate_effect','false')=='true',
        'skills':result}


def decode_rationals(value):
    if isinstance(value,dict):
        if set(value)=={'numerator','denominator'}:return F(value['numerator'],value['denominator'])
        return {key:decode_rationals(child) for key,child in value.items()}
    if isinstance(value,list):return [decode_rationals(child) for child in value]
    if isinstance(value,float):raise ValueError('binary float in evidence')
    return value


def verify_facts(facts):
    # Deliberately finite formula registry, arbitrary actual inputs, no fixture lookup.
    formulas={function.__name__:function for function in [enchant_increment,enchant_hp,crystal_count,failure_refund,
        enchant_chance,enchant_outcome,enchant_valid,max_load,overload,grade_penalties,fist_spread,pole_damage,pole_sequence,
        bow_reuse,bow_damage,dual_damage,dual_timing,skill_caller_damage,position,shield,physical_damage,magic_damage,cast_time,heal,blow_success,lethal,retail_select,
        retail_distribution,category,pack_options,cancel_fee,set_shield_history,clamp_no_heal,charge_transition,shot_payment,shared_shot_bundle,winning_providers,consumable_use,consumable_cast_sequence,player_level_range]}
    formulas.update({f.__name__: f for f in [behavior.bow_launch, behavior.bow_refill, behavior.resurrection_scroll,
        behavior.dormant_use, behavior.learned_item_cost, behavior.escape_timing, behavior.dual_plan, behavior.enchant_safe,
        behavior.quantized_enchant, behavior.augmentation_provider, behavior.retail_draw_contract, behavior.item_regen_status]})
    if not facts:raise ValueError('missing item facts')
    for fact in facts:
        if set(fact)!={'formula','inputs','expected'} or fact['formula'] not in formulas:raise ValueError('unsupported or malformed item fact')
        calculated=rational(formulas[fact['formula']](**decode_rationals(fact['inputs'])))
        if calculated!=fact['expected']:raise ValueError('item evidence mismatch: '+fact['formula'])
    return len(facts)


def make_vectors(src):
    vectors=[]
    def add(family,name,inputs,result,citations,draws=()):
        row={'id':name,'family':family,'inputs':rational(inputs),'expected':rational(result),
             'sources':citations,'draws':list(draws)}
        row['vector_sha256']=hashlib.sha256(json.dumps(row,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        vectors.append(row)
    human=D+'stats/chars/baseStats/HumanFighter.xml'
    core_sources=['G/model/stats/functions/FuncEnchant.java:43-155','G/model/stats/functions/formulas/FuncPDefMod.java:44-70',
                  'G/model/stats/functions/formulas/FuncMDefMod.java:44-66','G/model/actor/templates/L2PcTemplate.java:71-75']
    def eligible(p):return p.get('for_npc','false')!='true' and not any(k in p for k in ['duration','time','item_skill','oncrit_skill','onmagic_skill','unequip_skill'])
    for i,kind in enumerate(['SWORD','BLUNT','DAGGER','BOW','DUAL','DUALFIST','POLE'],1):
        sid=15 if kind=='POLE' else src.choose(lambda sid,n,p:p.get('weapon_type')==kind and eligible(p) and n.find('cond') is None)
        refs=[(3599,1,2)] if kind=='POLE' else []
        gear=[(sid,0,'rhand',1)]
        add('F1',f'gear-{i:02d}-{kind}',{'class_file':human,'level':84,'equipment':gear,'expertise':7,'masteries':[],'skill_refs':refs},gear_stats(src,human,84,gear,refs),[src.citation(sid),*core_sources])
    add('F1','gear-08-FIST',{'class_file':human,'level':1,'equipment':[],'masteries':[]},gear_stats(src,human,1,[]),core_sources+['G/model/actor/L2Character.java:5034-5044'])
    for i,grade in enumerate(['D','C','B','A','S','S80'],9):
        magic=grade in {'D','B','S'};twohand=grade in {'C','A','S80'}
        sid=src.choose(lambda sid,n,p:p.get('crystal_type')==grade and p.get('enchant_enabled')=='1' and eligible(p)
                       and n.find('cond') is None and (not magic or p.get('is_magic_weapon')=='true')
                       and (not twohand or p.get('bodypart')=='lrhand'))
        gear=[(sid,4,'rhand',1)]
        add('F2',f'gear-{i:02d}-{grade}-e4',{'class_file':human,'level':84,'equipment':gear,'expertise':7,'masteries':[]},gear_stats(src,human,84,gear),[src.citation(sid),*core_sources])
    sid=src.choose(lambda sid,n,p:p.get('crystal_type')=='S84' and p.get('enchant_enabled')=='1' and eligible(p) and n.find('cond') is None)
    add('F2','gear-15-S84-e6',{'equipment':[(sid,6,'rhand',1)],'level':84},gear_stats(src,human,84,[(sid,6,'rhand',1)]),[src.citation(sid),*core_sources])
    for i,body,slot,enchant in [(16,{'fullarmor','onepiece'},'chest',4),(17,{'lhand'},'lhand',6),(18,{'rfinger;lfinger'},'lfinger',6)]:
        sid=next(sid for sid,(_,_,node) in sorted(src.items.items()) if node.get('type')=='Armor'
            and src.properties(sid).get('bodypart') in body and src.properties(sid).get('crystal_type','NONE')!='NONE'
            and eligible(src.properties(sid)))
        add('F3',f'gear-{i:02d}-{slot}-e{enchant}',{'equipment':[(sid,enchant,slot,1)],'level':84},
            gear_stats(src,human,84,[(sid,enchant,slot,1)]),[src.citation(sid),*core_sources,'D/stats/enchantHPBonus.xml:3-94'])
    for i,kind in [(19,'HEAVY'),(20,'LIGHT'),(21,'MAGIC')]:
        found=False
        for file,set_node in sorted(src.sets,key=lambda pair:int(pair[1].get('id'))):
            chest=int(set_node.find('chest').get('id'))
            if src.properties(chest).get('armor_type')!=kind:continue
            gear=[]
            for seq,slot in enumerate(['chest','legs','head','gloves','feet'],1):
                candidates=[int(n.get('id')) for n in set_node.findall(slot)]
                if candidates:gear.append((min(candidates),6 if i==19 else 0,slot,seq))
            refs=[(int(n.get('id')),int(n.get('level')),100+idx) for idx,n in enumerate(set_node.findall('skill')+(set_node.findall('enchant6skill') if i==19 else []))]
            deltas={n.tag:int(n.get('val')) for n in set_node if n.tag in {'str','dex','con','int','wit','men'}}
            try:result=gear_stats(src,human,84,gear,refs,deltas)
            except (ValueError,KeyError):continue
            add('F12',f'gear-{i:02d}-{kind}-set',{'set_id':int(set_node.get('id')),'equipment':gear,'skill_refs':refs,'base_deltas':deltas,'level':84},result,[file,*core_sources])
            broken=gear[:-1]
            add('F12',f'set-{kind}-missing-piece',{'equipment':broken,'skill_refs':[],'level':84},gear_stats(src,human,84,broken),[file,*core_sources])
            found=True;break
        if not found:raise ValueError('no supported '+kind+' source set vector')
    kits=src.xml(D+'stats/initialEquipment.xml')
    class_files={int(src.xml(path.relative_to(src.dp).as_posix()).findtext('classId')):path.relative_to(src.dp).as_posix()
                 for path in sorted((src.dp/(D+'stats/chars/baseStats')).glob('*.xml'))}
    named_kits={10:(22,'HumanMystic'),44:(23,'OrcFighter'),53:(24,'DwarvenFighter')}
    for kit in kits.findall('equipment'):
        class_id=int(kit.get('classId'));items=[dict(n.attrib) for n in kit.findall('item')]
        add('F12',f'starter-kit-source-{class_id}',{'class_id':class_id,'items':items},
            {'items':items,'race_supported':src.races[class_id]!='KAMAEL','draws':[]},
            [D+'stats/initialEquipment.xml:4-109','design/12.1 starter source domains; Kamael remains deferred'])
        if src.races[class_id]=='KAMAEL':continue
        gear=[];weight=0
        for seq,n in enumerate(kit.findall('item'),1):
            sid=int(n.get('id'));p=src.properties(sid);weight+=int(p.get('weight','0'))*int(n.get('count'))
            if n.get('equipped')=='true':
                body=p['bodypart'];slot='rhand' if body=='lrhand' else body
                gear.append((sid,0,slot,seq))
        class_file=class_files[class_id]
        result=gear_stats(src,class_file,1,gear);result['inventory_weight']=weight;result['capacity']=100 if class_id==53 else 80
        result['kit']=[dict(n.attrib) for n in kit.findall('item')]
        name=f'gear-{named_kits[class_id][0]:02d}-kit-{named_kits[class_id][1]}' if class_id in named_kits else f'starter-kit-{class_id}'
        add('F12',name,{'class_file':class_file,'class_id':class_id,'equipment':gear,'level':1,'kit':items},
            result,[D+'stats/initialEquipment.xml:4-109',class_file+':15-31',*core_sources])
    sid=src.choose(lambda sid,n,p:'enchant4_skill' in p and 'time' not in p and 'duration' not in p)
    ref=src.properties(sid)['enchant4_skill'];skill_id,rank=map(int,ref.split('-'))
    add('F12','weapon-enchant4-provider',{'template_id':sid,'e3':3,'e4':4}, {'before':[],'after':[{'id':skill_id,'rank':rank}]},[src.citation(sid),'G/model/itemcontainer/PcInventory.java:969-976'])
    for sid,(file,line,node) in sorted(src.items.items()):
        properties=src.properties(sid)
        if node.get('type')=='EtcItem' and (properties.get('etcitem_type') in {'POTION','ELIXIR'}
           or properties.get('etcitem_type')=='SCROLL' and any(word in node.get('name') for word in ['Escape','Resurrection'])):
            add('F12',f'consumable-{sid}',{'template_id':sid},consumable_definition(src,sid),[src.citation(sid),'H/itemhandlers/ItemSkillsTemplate.java:60-119'])
    injected=ET.fromstring(src.read(human))
    injected.find('staticData/baseMDef/lfinger').text='999'
    injected.find('staticData/baseMDef/rfinger').text='7'
    proxy=SimpleNamespace(xml=lambda file:injected,bonuses=src.bonuses,items=src.items,skills=src.skills,hp=src.hp,properties=src.properties,races=src.races,fists=src.fists)
    add('F1','RFINGER-duplication-unequal-injected-bases',{'class_file':human,'level':84,'injected_lfinger':999,'injected_rfinger':7},
        gear_stats(proxy,human,84,[]),[human,'G/model/actor/templates/L2PcTemplate.java:71-75','design/12.1 injected unequal finger bases'])
    for sid in [4270,6209,6213,3599]:
        for rank in range(1,int(src.skills[sid][1].get('levels'))+1):
            refs=[(sid,rank,100)];sheet=gear_stats(src,human,84,[],refs)
            for lifecycle in ['living','death','dead_admission','resurrection']:
                add('F1',f'system-{sid}-{rank}-{lifecycle}',{'skill_refs':refs,'lifecycle':lifecycle},sheet,[src.skills[sid][0],'design/D16 retained System providers'])
    for name,inputs in class_slot_cases(src):
        add('F1',name,inputs,defence_slot_stats(src,**inputs),
            [inputs['class_file']+':15-31','G/model/actor/templates/L2PcTemplate.java:64-75',
             'G/data/xml/impl/PlayerTemplateData.java:106-108',
             'G/model/stats/functions/formulas/FuncPDefMod.java:44-70',
             'G/model/stats/functions/formulas/FuncMDefMod.java:44-66','G/model/stats/BaseStats.java:115-120',
             'G/datatables/ItemTable.java:79,104 fullarmor/onepiece share SLOT_FULL_ARMOR'])
    # D75: unarmed reach per class (template baseAtkRange) and a weapon-override contrast.
    reach_sources=['G/model/actor/stat/CharStat.java:424-435','G/model/actor/stat/CharStat.java:438-447',
                   'G/model/actor/L2Character.java:5560-5572','G/model/actor/templates/L2CharTemplate.java:114,116',
                   'G/model/actor/instance/L2PcInstance.java:1018-1019']
    for name,inputs in class_combat_cases(src):
        add('F1',name,inputs,unarmed_reach(src,inputs['class_file']),[inputs['class_file']]+reach_sources)
    orc=D+'stats/chars/baseStats/OrcFighter.xml'
    add('F1','class-combat-44-armed-2369',{'class_file':orc,'base_class_id':44,'weapon':2369},unarmed_reach(src,orc,2369),
        [orc,src.citation(2369),'G/model/items/L2Weapon.java:92']+reach_sources)
    # Pinned fingers are equal; unequal synthetic XML distinguishes the getter quirk.
    root=src.xml(human)
    root.find('staticData/baseMDef/rfinger').text='7'
    root.find('staticData/baseMDef/lfinger').text='999'
    proxy=SimpleNamespace(xml=lambda file:root,bonuses=src.bonuses)
    for slot in ['rfinger','lfinger']:
        inputs={'class_file':human,'level':84,'occupied_slots':[slot],'chest_bodypart':'none'}
        synthetic={'kind':'synthetic','description':'class XML RFINGER=7, LFINGER=999; other nodes unchanged',
                   'class_override_xml':ET.tostring(root,encoding='unicode')}
        add('F1','class-slots-unequal-'+slot,dict(inputs,synthetic=synthetic),defence_slot_stats(proxy,**inputs),
            [human+':24-30','G/model/actor/templates/L2PcTemplate.java:71-75',
             'G/data/xml/impl/PlayerTemplateData.java:106-108','G/model/stats/functions/formulas/FuncMDefMod.java:44-66',
             'design/12.1 deliberately unequal synthetic RFINGER/LFINGER'])
    # Every branch/grade at source enchant boundaries, including u32 storage endpoint.
    for grade in src.grades:
        for body,kind in [('rhand','SWORD'),('lrhand','POLE'),('lrhand','BOW'),('lrhand','DUAL'),
                          ('rhand','BOW'),('rhand','CROSSBOW'),('lrhand','CROSSBOW')]:
            for e in [0,3,4,19,20,65535,4294967295]:
                for stat in ['pAtk','mAtk','pDef','mDef','sDef']:
                    add('F2',f'enchant-{grade}-{body}-{kind}-{stat}-{e}',{'stat':stat,'grade':grade,'bodypart':body,'weapon_type':kind,'enchant':e},
                        enchant_increment(stat,grade,body,kind,e),['G/model/stats/functions/FuncEnchant.java:43-155'])
        for weapon in [False,True]:
            data=src.grades[grade];bonus=data['weapon_bonus' if weapon else 'armor_bonus']
            for e in [0,3,4,65535,4294967295]:
                add('F5',f'crystals-{grade}-{weapon}-{e}',{'base':101,'bonus':bonus,'enchant':e,'weapon':weapon},
                    {'full':crystal_count(101,bonus,e,weapon),'failure':failure_refund(101,bonus,e,weapon,crystal_id=data['crystal_id']),
                     'noncrystallizable':failure_refund(101,bonus,e,weapon,False)},['G/model/items/L2Item.java:405-420','G/model/items/type/CrystalType.java:26-33','G/network/clientpackets/RequestEnchantItem.java:259-271'])
    for grade,bonuses in src.hp.items():
        for e in [0,1,3,4,len(bonuses),len(bonuses)+1,4294967295]:
            for onepiece in [False,True]:
                add('F3',f'hp-{grade}-{e}-{onepiece}',{'bonuses':bonuses,'enchant':e,'onepiece':onepiece},enchant_hp(bonuses,e,onepiece),
                    ['D/stats/enchantHPBonus.xml:3-94','G/data/xml/impl/EnchantItemHPBonusData.java:116-126'])
    for name,rows in src.rates.items():
        for lower,upper,chance in rows:
            for e in sorted({lower,upper}):
                c=enchant_chance(chance);residues=sorted({0,99999,max(0,c-1),min(c,99999)})
                for r in residues:
                    add('F4',f'chance-{name}-{e}-{r}',{'base':chance,'enchant':e,'residue':r},enchant_outcome(chance,0,0,r,e),
                        ['D/enchantItemGroups.xml:4-59','G/model/items/enchant/EnchantScroll.java:147-157'],[{'site':'enchant','bound':100000,'residue':r}])
    for scroll in src.xml(D+'enchantItemData.xml').findall('enchant')+src.xml(D+'enchantItemData.xml').findall('support'):
        maximum=int(scroll.get('maxEnchant','65535'));bonus=F(scroll.get('bonusRate','0'))
        for e in [max(0,maximum-1),maximum,maximum+1]:
            add('F4',f'scroll-{scroll.tag}-{scroll.get("id")}-max-{e}',dict(scroll.attrib,current=e),
                {'valid':enchant_valid(e,maximum),'bonus_chance_1e5':enchant_chance(0,bonus)},['D/enchantItemData.xml','G/model/items/enchant/AbstractEnchantItem.java:109-130'])
    add('F4','bonus-only-zero-base',{'base':0,'bonus':100,'residue':0,'enchant':20},enchant_outcome(0,100,0,0,20),['G/network/clientpackets/RequestEnchantItem.java:163-168'])
    for blessed,safe in [(False,False),(True,False),(False,True)]:
        add('F4',f'failure-{blessed}-{safe}',{'base':0,'residue':99999,'blessed':blessed,'safe':safe},enchant_outcome(0,0,0,99999,7,blessed,safe),['G/network/clientpackets/RequestEnchantItem.java:199-271'])
    add('F4','blessed-support-refused',{'current':3,'max_enchant':16,'blessed':True,'support':True},enchant_valid(3,16,True,True),['G/model/items/enchant/EnchantScroll.java:102-114'])
    quotient,remainder=divmod(2**32,100000)
    for chance in [0,66670,100000]:
        add('F4',f'modulo-lattice-{chance}',{'chance_1e5':chance,'bound':100000}, {'successful_residues':sum(r<chance for r in range(100000)),
            'u32_success_preimages':chance*quotient+min(chance,remainder),'quotient':quotient,'remainder':remainder},['design/4.2 integer adaptation'])
    for con,bonus in src.bonuses['CON'].items():
        load=max_load(bonus)
        add('F7',f'load-CON-{con}',{'con':con,'bonus':bonus,'weight_limit':1}, {'max_load':load,'modified':max_load(bonus,1,F(3,2),100)},
            ['D/stats/statBonus.xml/CON','G/model/actor/instance/L2PcInstance.java:9427-9438'])
    for threshold in [500,666,800,1000]:
        for delta in [-1,0,1]:
            weight=threshold*100+1+delta
            add('F7',f'overload-{threshold}-{delta}',{'weight':weight,'maximum':100000,'bonus':1},overload(weight,100000),['G/model/actor/instance/L2PcInstance.java:1668-1693'])
    for weight in [99999,100000,100001]:
        for previous in [False,True]:
            add('F7',f'overload-history-{weight}-{previous}',{'weight':weight,'maximum':100000,'bonus':0,'previous_level':4,'previous_overloaded':previous},
                overload(weight,100000,0,4,previous),['G/model/actor/instance/L2PcInstance.java:1688-1692','design/I4'])
    add('F7','overload-max-zero',{'maximum':0,'previous_level':3,'previous_overloaded':True},overload(100,0,1,3,True),['G/model/actor/instance/L2PcInstance.java:1668-1671'])
    for expertise in range(8):
        for grade in range(8):
            entries=[{'instance_id':1,'grade':grade,'weapon':True},{'instance_id':2,'grade':max(0,grade-1),'weapon':True},{'instance_id':3,'grade':7,'weapon':False,'arrow':True}]
            add('F6',f'grade-{expertise}-{grade}-branch',{'equipped':entries,'expertise':expertise},grade_penalties(entries,expertise),['G/model/actor/instance/L2PcInstance.java:1707-1745'])
    for level in [1,3,4,8,9,15,16]:
        radius=5+isqrt(level)
        for path in ['unequip','level_change','restore_weapon']:
            for residue in [0,2*radius]:
                add('F10',f'fist-{level}-{path}-{residue}',{'level':level,'path':path,'residue':residue},fist_spread(level,residue),['G/model/actor/L2Character.java:5034-5044'])
        add('F10',f'fist-template-restored-{level}',{'level':level,'template_radius':10,'residue':20},fist_spread(level,20,10),['G/model/actor/L2Character.java:5034-5044'])
    for damage,percent in [(F(499,10),85),(F(999,50),85),(20,85),(F(1001,50),85),(1,85),(F(499,10),F(1700,23))]:
        add('F10',f'pole-{damage}-{percent}',{'formula':damage,'percent':percent},pole_damage(damage,percent),['G/model/actor/L2Character.java:1282-1283,1335-1338'])
    for formula in [F(499,10),F(1),F(2),F(10199,100)]:
        for miss in [False,True]:add('F10',f'dual-{formula}-{miss}',{'formula':formula,'miss':miss},dual_damage(formula,miss),['G/model/actor/L2Character.java:1158-1182'])
    for speed in [379,400,1500]:add('F10',f'dual-time-{speed}',{'attack_speed':speed},dual_timing(speed),['G/model/actor/L2Character.java:887,1176-1180'])
    for critical in [False,True]:add('F8',f'skill-post-critical-perfect-{critical}',{'formula':1,'critical':critical},skill_caller_damage(1,critical),['H/effecthandlers/instant/PhysicalAttack.java:86-104','H/effecthandlers/instant/FatalBlow.java:74-87'])
    for formula in [F(499,10),F(1)]:add('F10',f'pole-primary-{formula}',{'formula':formula},pole_damage(formula,primary=True),['G/model/actor/L2Character.java:1282-1283'])
    candidates=[{'eligible':False,'hit':True,'formula':100},{'eligible':True,'hit':False,'formula':100},{'eligible':True,'hit':True,'formula':F(499,10)}]
    add('F10','pole-miss-advances-only-eligible',{'candidates':candidates,'maximum':1},pole_sequence(candidates,1),['G/model/actor/L2Character.java:1237-1287,1335-1338'])
    for modifier in [1,2]:add('F10',f'bow-reuse-{modifier}',{'reuse_delay':1500,'attack_speed':400,'weapon_reuse_modifier':modifier},bow_reuse(1500,400,modifier),['G/model/actor/L2Character.java:4353-4357'])
    for formula,dx,dy in [(100,12499,0),(100,12500,0),(100,12501,0),(F(10199,100),12500,0),(100,7500,10000),(100,7501,10000),(1,0,0)]:
        add('F10',f'bow-{formula}-{dx}-{dy}',{'formula':formula,'dx_milli':dx,'dy_milli':dy},bow_damage(formula,dx,dy),['G/model/actor/L2Character.java:1029-1033','design/F10 planar units32/1000'])
    for caller in ['auto','physical','blow']:
        for shot in [False,True]:
            for proximity in ['front','side','back']:
                for block in ['none','normal','perfect']:
                    inputs={'p_atk':100,'defence':80,'power':200,'caller':caller,'shot':shot,'proximity':proximity,'shield_def':40 if block=='normal' else 0,'perfect':block=='perfect'}
                    add('F8',f'damage-{caller}-{shot}-{proximity}-{block}',inputs,physical_damage(**inputs),['G/model/stats/Formulas.java:502-555,667-808'])
    for spirit,blessed in [(False,False),(True,False),(False,True),(True,True)]:
        inputs={'power':100,'m_atk':121,'m_def':80,'spirit':spirit,'blessed':blessed}
        add('F8',f'magic-{spirit}-{blessed}',inputs,magic_damage(**inputs),['G/model/stats/Formulas.java:815-897'])
        for hit in [500,501,2000]:add('F8',f'cast-{spirit}-{blessed}-{hit}',{'hit_time':hit,'casting_speed':1000,'spirit':spirit,'blessed':blessed},cast_time(hit,1000,spirit,blessed),['G/model/stats/Formulas.java:1027-1045'])
        for mage,npc,grade in [(True,False,'NONE'),(False,True,'NONE'),(False,False,'NONE'),(False,False,'S80'),(False,False,'S84')]:
            for static in [False,True]:
                inputs={'power':100,'m_atk':121,'mp_consume2':10,'mage':mage,'npc':npc,'grade':grade,'spirit':spirit,'blessed':blessed,'static':static}
                add('F8',f'heal-{mage}-{npc}-{grade}-{spirit}-{blessed}-{static}',inputs,heal(**inputs),['H/effecthandlers/instant/Heal.java:68-104'])
    for static,is_magic,hit_time in [(True,False,20000),(True,False,0),(False,False,2000)]:
        inputs={'hit_time':hit_time,'casting_speed':1000,'spirit':True,'static':static,'is_magic':is_magic}
        add('F8',f'cast-static-{static}-{is_magic}-{hit_time}',inputs,cast_time(**inputs),['G/model/stats/Formulas.java:1027-1045'])
    for perfect in [90,91,99]:
        for normal in [19,20]:
            add('F9',f'shield-{perfect}-{normal}',{'rate':20,'perfect_residue':perfect,'normal_residue':normal},shield(20,perfect_residue=perfect,normal_residue=normal),['G/model/stats/Formulas.java:1077-1105','GC:136-140'])
    for guard in ['eligible','facing','sigil','zero','bow']:
        kwargs={'eligible':guard!='eligible','facing':guard!='facing','sigil':guard=='sigil','bow':guard=='bow','perfect_residue':90,'normal_residue':25}
        add('F9','shield-guard-'+guard,dict(kwargs,rate=0 if guard=='zero' else 20),shield(0 if guard=='zero' else 20,**kwargs),['G/model/stats/Formulas.java:1077-1105'])
    for relative in [(1,0),(-1,0),(0,1),(1,2),(0,0)]:add('F9','position-'+str(relative),{'facing':[1,0],'relative':relative},position((1,0),relative),['G/model/actor/L2Character.java:4930-4993'])
    for pos in ['front','side','back']:
        for residue in [499,500,749,750,999]:
            add('F10',f'blow-{pos}-{residue}',{'chance':50,'dex':1,'proximity':pos,'residue':residue},
                blow_success(50,1,pos,residue),behavior.BLOW_SOURCES,[{'site':'blow','bound':1000,'residue':residue}])
    for full_r,half_r in [(0,0),(10,0),(10,50),(999,999)]:
        add('F10',f'lethal-{full_r}-{half_r}',{'full':1,'half':5,'full_residue':full_r,'half_residue':half_r},lethal(1,5,full_r,half_r),['H/effecthandlers/instant/Lethal.java:60-96'])
    for kind,name in [('weapon','retailchances.xml'),('accessory','retailchances_accessory.xml')]:
        file=D+'stats/augmentation/'+name
        for weapon in src.xml(file).findall('weapon'):
            for stone in weapon.findall('stone'):
                for variation in stone.findall('variation'):
                    for group in variation.findall('category'):
                        rows=[(int(n.get('id')),F(n.get('chance'))) for n in group.findall('augment')]
                        distribution=retail_distribution(rows)
                        samples={r:retail_select(rows,r) for r in [0,1,5499,5500,8999,9000,9999]}
                        add('F11',f'retail-{kind}-{weapon.get("type")}-{stone.get("id")}-{variation.get("id")}-{group.get("probability")}',
                            {'kind':kind,'stone_id':int(stone.get('id')),'weapon_type':weapon.get('type'),'variation':int(variation.get('id')),'category':int(group.get('probability')),'rows':rows},
                            {'distribution':distribution,'samples':samples,'draw_bound':10000,'non_retail_draws':[]},[file,'G/datatables/AugmentationData.java:447-559,689-737'])
    for accessory in [False,True]:
        distribution=Counter(category(r,accessory) for r in range(10000))
        add('F11',f'categories-{accessory}',{'accessory':accessory},dict(distribution),['G/datatables/AugmentationData.java:447-559,689-737','design/A8 integer weights'])
    add('F11','packed-unsigned-high-half',{'option12':65535,'option34':65535},pack_options(65535,65535),['G/datatables/AugmentationData.java:558','design/D19'])
    for grade,limits in [('C',[1720,2452]),('B',[1746]),('A',[2160,2824]),('S',[1]),('S80',[1]),('S84',[1])]:
        for limit in limits:
            for delta in [-1,0,1]:add('F11',f'cancel-{grade}-{limit+delta}',{'grade':grade,'crystals':limit+delta},cancel_fee(grade,limit+delta),['G/network/clientpackets/RequestRefineCancel.java:64-109'])
    crystals={int(n.get(field)) for n in src.xml(D+'levelUpCrystalData.xml').findall('crystal/item') for field in ['itemId','leveledItemId']}
    for path in sorted((src.dp/(D+'multisell')).glob('*.xml')):
        root=ET.parse(path).getroot();file=path.relative_to(src.dp).as_posix()
        for ordinal,row in enumerate(root.findall('item')):
            ingredients=[(int(n.get('id')),int(n.get('count'))) for n in row.findall('ingredient')]
            weapons=[sid for sid,c in ingredients if sid in src.items and src.items[sid][2].get('type')=='Weapon']
            if len(weapons)!=1 or not any(sid in crystals for sid,c in ingredients):continue
            products=[(int(n.get('id')),int(n.get('count'))) for n in row.findall('production')]
            if not products or any(sid not in src.items or src.items[sid][2].get('type')!='Weapon' for sid,c in products):continue
            src.read(file)
            add('F12',f'SA-{path.stem}-{ordinal}',{'ingredients':ingredients,'products':products,'maintain_enchantment':root.get('maintainEnchantment','false')=='true','enchant':7,'augment':2147483649,'tax_rate':0},
                {'charged':ingredients,'products':products,'enchant':7 if root.get('maintainEnchantment')=='true' else 0,'augment':2147483649 if root.get('maintainEnchantment')=='true' else None,'equipped':False,'fresh_identity':True},[file,'G/network/clientpackets/MultiSellChoose.java:247-278,342-363','design/D22 fixture tax0'])
    for node in src.xml(D+'levelUpCrystalData.xml').findall('crystal/item'):
        add('F12','soul-'+node.get('itemId'),dict(node.attrib),{'stage':int(node.get('level')),'leveled_item':int(node.get('leveledItemId')),'absorb':'DeferredPhase6','draws':[]},[D+'levelUpCrystalData.xml:4-59'])
    for first,second in [('chest','shield'),('shield','chest')]:
        active=False;chest=False;matching=False;history=[]
        for event in [first,second]:
            chest|=event=='chest';matching|=event=='shield'
            active=set_shield_history(active,'equip_'+event,chest,matching,False);history.append(active)
        add('F12','set-history-'+first+'-'+second,{'mutations':[first,second],'complete':False}, {'history':history,'checkpoint_rebuild_active':active,'hook_draws':[]},['G/model/itemcontainer/PcInventory.java:1198-1208,1232-1285','design/D20'])
    for removal in ['chest','shield','required']:
        result=set_shield_history(True,'remove',removal!='chest',removal!='shield',False,removal=='required')
        add('F12','set-history-remove-'+removal,{'active':True,'remove':removal},result,['G/model/itemcontainer/PcInventory.java:1232-1285'])
    for current,maximum in [(100,150),(100,75)]:add('F1',f'clamp-{current}-{maximum}',{'current':current,'new_maximum':maximum},clamp_no_heal(current,maximum),['design/3.2 clamp-no-heal'])
    for landed,unequip in [(False,False),(True,False),(False,True)]:
        for use_spirit,use_soul in [(True,True),(False,True)]:
            inputs={'charges':['soul','spirit','blessed'],'use_spirit':use_spirit,'use_soul':use_soul,'landed':landed,'unequip':unequip}
            add('F8',f'charge-{landed}-{unequip}-{use_spirit}-{use_soul}',inputs,charge_transition(**inputs),['design/I2 shared charge, all miss retention and blessed precedence'])
    add_revision_vectors(src,add,human)
    behavior.add_behavior_vectors(src,add,sys.modules[__name__])
    table=independent_angle_table()
    for deg,relative in [(0,(1,0)),(90,(1,1)),(120,(1,2)),(180,(0,1)),(270,(-1,1)),(360,(-1,0))]:
        add('F9',f'angle-{deg}-{relative}',{'degrees':deg,'facing':[1,0],'relative':relative},
            angle_contains((1,0),relative,deg,table),['design/D21 bounded certified interval geometry'])
    return vectors


def scroll_bindings(src):
    groups=src.xml(D+'enchantItemGroups.xml')
    for scroll in src.xml(D+'enchantItemData.xml').findall('enchant'):
        sid=int(scroll.get('id'));kind=src.properties(sid).get('etcitem_type','NONE')
        weapon=kind in {'ANCIENT_CRYSTAL_ENCHANT_WP','BLESS_SCRL_ENCHANT_WP','SCRL_ENCHANT_WP'}
        group_id=int(scroll.get('scrollGroupId','0'))
        group=next((g for g in groups.findall('enchantScrollGroup') if int(g.get('id'))==group_id),None)
        if group is None:raise ValueError('unsupported scroll group')
        for binding in group.findall('enchantRate'):
            targets=[dict(n.attrib) for n in binding.findall('item') if (n.get('slot') in {'rhand','lrhand'})==weapon]
            if targets:yield scroll,binding.get('group'),targets


def add_revision_vectors(src,add,human):
    formula=['G/model/stats/Formulas.java:815-897']
    for name,kwargs in [
        ('pve-critical',{'critical':True}),
        ('pvp-critical',{'critical':True,'target_player':True}),
        ('playable-nonplayer-critical',{'critical':True,'is_pvp':True,'attacker_player':False,'target_player':True}),
        ('half-success',{'critical':True,'magic_failures':True,'first_success':False,'second_success':True,'level_difference':9}),
        ('resisted-level',{'magic_failures':True,'first_success':False,'second_success':True,'level_difference':10}),
        ('resisted-second',{'magic_failures':True,'first_success':False,'second_success':False}),
        ('nonplayer-failure',{'attacker_player':False,'magic_failures':True,'first_success':False,'critical':True}),
        ('disabled-failures',{'magic_failures':False,'first_success':False,'critical':True}),
        ('perfect-skips',{'perfect':True,'m_def':0,'magic_failures':True,'first_success':False}),
        ('modifiers-magic',{'target_player':True,'pvp_magic_def':2,'pvp_magic_damage':F(3,2),'spread':F(9,10),'attribute':F(6,5),'npc':F(4,5),'magic_crit_damage':F(5,4),'critical':True}),
        ('modifiers-physical',{'target_player':True,'skill_is_magic':False,'pvp_skill_def':3,'pvp_skill_damage':2,'attribute':F(7,5)})]:
        for spirit,blessed in [(False,False),(True,False),(False,True),(True,True)]:
            inputs=dict(power=100,m_atk=100,m_def=100,spirit=spirit,blessed=blessed);inputs.update(kwargs)
            add('F8',f'magic-branch-{name}-{spirit}-{blessed}',inputs,magic_damage(**inputs),formula)
    hp=0;remainder=0
    for step in range(3):
        inputs=dict(power=100,m_atk=121,current_hp=hp,remainder_q=remainder,recoverable_hp=1000)
        result=heal(**inputs);hp=result['current_hp'];remainder=result['remainder_q']
        add('F8',f'heal-fraction-repeat-{step}',inputs,result,['H/effecthandlers/instant/Heal.java:92-104','design/P3:128 restoration Q remainder'])
    for name,kwargs in [('partial',{'current_hp':950,'recoverable_hp':1000,'remainder_q':500000}),
                        ('full',{'current_hp':1000,'recoverable_hp':1000}),
                        ('over',{'current_hp':1001,'recoverable_hp':1000}),
                        ('over-fraction-positive',{'current_hp':1001,'recoverable_hp':1000,'remainder_q':500000}),
                        ('over-fraction-zero',{'static':True,'power':0,'current_hp':1001,'recoverable_hp':1000,'remainder_q':500000}),
                        ('at-cap-fraction',{'current_hp':1000,'recoverable_hp':1000,'remainder_q':500000}),
                        ('static',{'static':True,'power':'5/4','heal_effect':9,'critical':True,'current_hp':9,'recoverable_hp':10}),
                        ('negative-static',{'static':True,'power':-1})]:
        inputs=dict(power=100,m_atk=121);inputs.update(kwargs)
        add('F8','heal-clamp-'+name,inputs,heal(**inputs),['H/effecthandlers/instant/Heal.java:92-104'])
    inputs={'class_file':human,'level':1,'equipment':[(1,0,'rhand',1)],'skill_refs':[(8498,1,2)]}
    attribute_sources=['D/stats/skills/08400-08499.xml:1906-1916','G/model/stats/BaseStats.java:80-85',
        'G/model/stats/functions/formulas/FuncPAtkMod.java:42-43','G/model/stats/functions/formulas/FuncArmorSet.java:46-67']
    add('F1','attribute-provider',inputs,gear_stats(src,**inputs),attribute_sources)
    for file,set_node in src.sets:
        deltas={n.tag:int(n.get('val')) for n in set_node if n.tag in {'str','dex','con','int','wit','men'}}
        if not deltas.get('str'):continue
        gear=[(1,0,'rhand',1)]+[(int(set_node.find(slot).get('id')),0,slot,i+2)
            for i,slot in enumerate(['chest','legs','head','gloves','feet']) if set_node.find(slot) is not None]
        refs=[(int(n.get('id')),int(n.get('level')),100+i) for i,n in enumerate(set_node.findall('skill'))]+[(8498,1,200)]
        inputs={'class_file':human,'level':1,'equipment':gear,'skill_refs':refs,'base_deltas':deltas}
        try:sheet=gear_stats(src,**inputs)
        except (ValueError,KeyError):continue
        text=src.read(file);anchor=text[:text.index('<set id="'+set_node.get('id')+'"')].count('\n')+1
        add('F1','attribute-set-interaction',dict(inputs,set_id=int(set_node.get('id'))),sheet,[file+':'+str(anchor),*attribute_sources])
        break
    else:raise ValueError('no source-executable attribute/set interaction')
    # Synthetic priority-1 SET makes ADD/SET install ordering observable; not a DP skill.
    proxy=SimpleNamespace(**vars(src));proxy.xml=src.xml;proxy.properties=src.properties
    proxy.skills=dict(src.skills)
    file,node=src.skills[8498];node=ET.fromstring(ET.tostring(node))
    stat=node.find('effects/effect/add');stat.tag='set';stat.set('order','1');stat.set('val','40')
    proxy.skills[8498]=(file,node)
    for sequence in [1,3]:
        inputs={'class_file':human,'level':1,'equipment':[(1,0,'rhand',1)],'skill_refs':[(8498,1,2)],
                'base_deltas':{'str':2},'base_delta_sequence':sequence}
        synthetic={'kind':'synthetic','description':'skill8498 STR ADD replaced with priority-1 SET40; set STR+2 at priority1',
                   'skill_override_xml':ET.tostring(node,encoding='unicode')}
        add('F1',f'attribute-same-priority-{sequence}',dict(inputs,synthetic=synthetic),gear_stats(proxy,**inputs),
            ['design/3.2 FunctionKey install order; synthetic SET operand',*attribute_sources])
    # Source SET/enchant installation, complete removal, and fresh reinstallation.
    for step,gear in enumerate([[(69,4,'rhand',1)],[],[(69,4,'rhand',3)]]):
        inputs={'class_file':human,'level':1,'equipment':gear}
        add('F1',f'provider-install-remove-reinstall-{step}',inputs,gear_stats(src,**inputs),
            [src.citation(69),'design/3.2 retained sequence and no onStart'])
    providers=[];sequence=0
    events=[('install','item:15',3599,1),('install','learned:3599',3599,1),
            ('remove','item:15',3599,1),('install','item:15',3599,1),('remove','learned:3599',3599,1)]
    for step,(op,key,sid,rank) in enumerate(events):
        if op=='install':
            sequence+=1;providers.append({'key':key,'skill_id':sid,'rank':rank,'sequence':sequence})
        else:providers=[p for p in providers if p['key']!=key]
        refs=winning_providers(providers)
        inputs={'providers':list(providers),'event':events[step],'equipment':[(15,0,'rhand',10)]}
        result={'winning_refs':refs,'sheet':gear_stats(src,human,1,inputs['equipment'],refs),'on_start_hooks':[]}
        add('F1',f'provider-overlap-{step}',inputs,result,[src.citation(15),'D/stats/skills/03500-03599.xml:1695-1705','design/3.2 provider-key tie and rank winner'])
    for scroll,name,targets in scroll_bindings(src):
        sid=int(scroll.get('id'));maximum=int(scroll.get('maxEnchant','65535'));bonus=F(scroll.get('bonusRate','0'))
        for lower,upper,chance in src.rates[name]:
            # Compact intervals cover EVERY integer level, with maximum boundary split.
            stops=sorted({lower,upper+1,*([maximum+1] if lower<=maximum<upper else [])})
            for start,end in zip(stops,stops[1:]):
                valid=enchant_valid(start,maximum)
                inputs={'scroll_id':sid,'group_id':int(scroll.get('scrollGroupId','0')),'group':name,
                        'targets':targets,'current_min':start,'current_max':end-1,'bonus':bonus,'max_enchant':maximum}
                result={'valid':valid,'base':chance,'chance_1e5':enchant_chance(chance,bonus),
                        'success_enchant_delta':int(chance>0),'draw_bound':100000 if valid else None}
                add('F4',f'scroll-binding-{sid}-{name}-{start}-{end-1}',inputs,result,
                    ['D/enchantItemData.xml:4-136','D/enchantItemGroups.xml:4-99','G/model/items/enchant/EnchantScroll.java:49-56,102-157','G/model/items/enchant/AbstractEnchantItem.java:59-61,109-130'])
    for kind in ['soul','spirit','blessed']:
        for toggle in [False,True]:
            for name,kwargs in [('paid',{}),('splus',{'weapon_grade':'S84','shot_grade':'S'}),
                                ('wrong-grade',{'shot_grade':'C'}),('empty',{'available':0}),
                                ('already',{'charges':[kind]}),('no-weapon',{'weapon_grade':None})]:
                inputs=dict(charges=[],kind=kind,count=2,available=10,weapon_grade='D',shot_grade='D',toggle=toggle);inputs.update(kwargs)
                payment=shot_payment(**inputs)
                add('F8',f'shot-payment-{kind}-{toggle}-{name}',inputs,payment,
                    ['H/itemhandlers/SoulShots.java:61-98','H/itemhandlers/SpiritShot.java:60-92','H/itemhandlers/BlessedSpiritShot.java:63-95'],payment['draws'])
    reduced_sid=next(sid for sid in sorted(src.items) if 'reduced_soulshot' in src.properties(sid))
    p=src.properties(reduced_sid);reduction=list(map(int,p['reduced_soulshot'].split(',')))
    for residue in [0,reduction[0]-1,reduction[0],99]:
        for available in [0,reduction[1],int(p['soulshots'])]:
            inputs=dict(charges=[],kind='soul',count=int(p['soulshots']),available=available,
                        weapon_grade=p['crystal_type'],shot_grade=p['crystal_type'],toggle=True,reduction=reduction,residue=residue)
            payment=shot_payment(**inputs)
            add('F8',f'shot-reduction-{reduced_sid}-{residue}-{available}',inputs,payment,
                [src.citation(reduced_sid),'H/itemhandlers/SoulShots.java:85-98'],payment['draws'])
    for caller,hits in [('dual',[False,False]),('dual',[True,False]),('dual',[False,True]),('dual',[True,True]),('pole',[False,True,True]),('multi-target',[True,True,True])]:
        for canceled,unequip in [(False,False),(True,False),(False,True)]:
            inputs=dict(charges=['soul','spirit','blessed'],hits=hits,use_spirit=caller=='multi-target',use_soul=caller!='multi-target',canceled=canceled,unequip=unequip)
            add('F8',f'shot-bundle-{caller}-{hits}-{canceled}-{unequip}',inputs,shared_shot_bundle(**inputs),['design/I2 single charge/discharge per bundle'])
    for template in [736,1538,737,1539,1540,5591,5592,8628,14688]:
        definition=consumable_definition(src,template)
        inputs={'definition':definition,'tick':10,'available':{template:10},'condition_context':{'level':1,'canEscape':True,'canResurrect':True,'inside_zone_ids':[]}}
        result=consumable_use(**inputs)
        add('F12',f'consumable-consume-once-{template}',inputs,result,[src.citation(template),'H/itemhandlers/ItemSkillsTemplate.java:81-129,139-149','G/model/actor/L2Character.java:1560-1566'])
        if result['accepted'] and result['item_deadlines']:
            next_inputs=dict(inputs,tick=11,item_deadlines=result['item_deadlines'],skill_deadlines=result['skill_deadlines'])
            add('F12',f'consumable-shared-reuse-{template}',dict(next_inputs,stack_identity=2),consumable_use(**next_inputs),[src.citation(template),'design/8 reuse shared-group across stack identities'])
        inputs['main_casting']=True
        add('F12',f'consumable-lane-{template}',inputs,consumable_use(**inputs),[src.citation(template),'H/itemhandlers/ItemSkillsTemplate.java:98-115','design/I5 ordered lane'])
    # Real static Escape20s starts at10; immediate potion0ms at209 completes at210.
    lane_defs=[consumable_definition(src,sid) for sid in [736,1539]]
    starts=[{'tick':10,'cast_id':1,'definition':lane_defs[0],'available':{736:1,1539:2},
             'condition_context':{'canEscape':True,'inside_zone_ids':[]}},
            {'tick':209,'cast_id':2,'definition':lane_defs[1]}]
    inputs={'starts':starts,'caster_ordinal':0,'caster_id':100}
    add('F12','consumable-same-tick-lanes',inputs,consumable_cast_sequence(**inputs),
        [src.citation(736),src.citation(1539),'D/stats/skills/02000-02099.xml (2013/2037)',
         'H/itemhandlers/ItemSkillsTemplate.java:98-129','G/model/actor/L2Character.java:1560-1566',
         'design/2.1 phase5 (deadline,caster ordinal,caster id,cast id); 5.5 minimum one tick; I5 prepaid lanes'])
    for sid in [2287,2288,2289,2860,2861,2862]:
        if sid not in src.skills:raise ValueError('missing elixir skill')
        file,skill=src.skills[sid];tables={n.get('name'):n.text.split() for n in skill.findall('table')}
        node=skill.find('cond/player');values=tables[node.get('levelRange')]
        for rank,value in enumerate(values,1):
            minimum,maximum=map(int,value.split(';'));resolved=ET.fromstring(ET.tostring(node));resolved.set('levelRange',value)
            for level in [minimum-1,minimum,maximum,maximum+1]:
                add('F12',f'elixir-range-{sid}-{rank}-{level}',{'skill_id':sid,'rank':rank,'minimum':minimum,'maximum':maximum,'level':level},
                    condition_matches(resolved,{'level':level}),[file,'G/model/conditions/ConditionPlayerLevelRange.java:41-43'])


def verify_coverage(src,vectors):
    """Fail closed on independent §12.1 cases AND their source input dimensions."""
    try:
        from .item_coverage import required_cases, verify_case
    except ImportError:
        from item_coverage import required_cases, verify_case
    required=required_cases(src)
    by_id={v['id']:v for v in vectors}
    if len(by_id)!=len(vectors):raise ValueError('duplicate coverage vector identity')
    missing=required.keys()-by_id.keys()
    if missing:raise ValueError('design 12.1 coverage missing vectors: '+','.join(sorted(missing)))
    for name,case in required.items():
        verify_case(name,case,by_id[name])
    bindings={};level_count=0
    for name,case in required.items():
        if not name.startswith('scroll-binding-'):continue
        inputs=case['inputs'];key=(inputs['scroll_id'],inputs['group'])
        bindings.setdefault(key,[]).append((inputs['current_min'],inputs['current_max']))
    for intervals in bindings.values():
        cursor=0
        for lower,upper in sorted(intervals):
            if lower!=cursor:raise ValueError('scroll/group/level coverage gap')
            cursor=upper+1
        if cursor!=65536:raise ValueError('incomplete scroll/group/level coverage')
        level_count+=cursor
    return {'design':'12.1','required_cases':len(required),'scroll_group_bindings':len(bindings),
            'covered_scroll_group_levels':level_count,'level_encoding':'contiguous inclusive source-rate intervals',
            'live_integration_sections':['12.2','12.4']}


@lru_cache(maxsize=None)
def independent_pi():
    # pi/4 = 2*atan(1/3) + atan(1/7), separate identity from generator's Machin series.
    def bounds(denominator):
        total=F(0)
        for index in range(110):total+=F((-1)**index,(2*index+1)*denominator**(2*index+1))
        next_term=F(1,221*denominator**221)
        return total,total+next_term
    a,b=bounds(3);c,d=bounds(7);scale=1<<256
    lower,upper=8*a+4*c,8*b+4*d
    return F(lower*scale//1,scale),F(-((-upper*scale)//1),scale)


@lru_cache(maxsize=None)
def independent_cosine(degrees):
    degrees=F(degrees)%360
    if degrees>180:degrees=360-degrees
    if degrees==0:return F(1),F(1)
    if degrees==180:return F(-1),F(-1)
    if degrees==90:return F(0),F(0)
    lower,upper=independent_pi();lower*=degrees/180;upper*=degrees/180
    def series(argument):
        partial=sum(((-1)**k*argument**(2*k)/factorial(2*k) for k in range(56)),F())
        return partial,partial+argument**112/factorial(112)
    return series(upper)[0],series(lower)[1]


EXACT_ANGLES={0:F(1),60:F(3,4),90:F(1,2),120:F(1,4),180:F(0),240:F(1,4),270:F(1,2),300:F(3,4),360:F(1)}
ANGLE_BOUND=256000  # test_zone bounds: 0..256 tiles, design D21


def independent_angle_table():
    rows=[];scale=1<<192
    for degrees in range(361):
        lo,hi=independent_cosine(degrees)
        if degrees in EXACT_ANGLES:lower=upper=EXACT_ANGLES[degrees]
        else:lower=F(((1+lo)/2)*scale//1,scale);upper=F(-((-((1+hi)/2)*scale)//1),scale)
        clo,chi=independent_cosine(F(degrees,2))
        rows.append({'degrees':degrees,'cos2_lower':str(lower),'cos2_upper':str(upper),
                     'cos_lower':str(F(clo*scale//1,scale)),'cos_upper':str(F(-((-chi*scale)//1),scale)),
                     'certified':True,'blockers':[]})
    return {'coordinate_bound_millitiles':ANGLE_BOUND,'angles':rows}


def verify_angle_table(table):
    if type(table['coordinate_bound_millitiles']) is not int or table['coordinate_bound_millitiles']!=ANGLE_BOUND:
        raise ValueError('incorrect angle coordinate bound')
    if [row['degrees'] for row in table['angles']]!=list(range(361)):raise ValueError('incomplete angle table')
    maximum=4*ANGLE_BOUND**4
    for row in table['angles']:
        deg=row['degrees'];lower,upper=F(row['cos2_lower']),F(row['cos2_upper'])
        if row['certified'] is not True or row['blockers']:raise ValueError('invalid angle certificate')
        if deg in EXACT_ANGLES:
            if lower!=EXACT_ANGLES[deg] or upper!=EXACT_ANGLES[deg]:raise ValueError('incorrect exact angle boundary')
        else:
            lo,hi=independent_cosine(deg)
            if not lower<=(1+lo)/2<=(1+hi)/2<=upper:raise ValueError('angle interval not independently enclosing')
            if lower<=((lower+upper)/2).limit_denominator(maximum)<=upper:raise ValueError('angle rational separation failed')
        lo,hi=independent_cosine(F(deg,2))
        if not F(row['cos_lower'])<=lo<=hi<=F(row['cos_upper']):raise ValueError('cosine interval not independently enclosing')
    return True


def generate(dp=DP,game=GAME):
    src=Source(dp,game);vectors=make_vectors(src)
    # Generated output is evidence under test, never an expectation provider.
    verify_angle_table(tomllib.loads((ROOT/'tables-phase4/angle_table.toml').read_text()))
    table=independent_angle_table();angles=[]
    for row in table['angles']:
        angles.append({'degrees':row['degrees'],'certified':True,
                       'cos2_lower':F(row['cos2_lower']),'cos2_upper':F(row['cos2_upper'])})
    for vector in vectors:
        vector['vector_sha256']=hashlib.sha256(json.dumps({k:v for k,v in vector.items() if k!='vector_sha256'},sort_keys=True,separators=(',',':')).encode()).hexdigest()
    if len({vector['id'] for vector in vectors})!=len(vectors):raise ValueError('duplicate oracle vector identity')
    coverage_gate=verify_coverage(src,vectors)
    manifest={'schema':1,'source_revisions':{'datapack':DP_REV,'game':GAME_REV},'source_file_sha256':dict(sorted(src.hashes.items())),
              'policy':'Independent pinned-source exact-rational oracle; injected random residues; V6 integer adaptations, not Java binary64 identity.',
              'coverage':dict(sorted(Counter(r['family'] for r in vectors).items())),
              'coverage_gate':coverage_gate,
              'vectors':vectors,'angles':rational(angles)}
    manifest=rational(manifest)
    manifest['vector_set_sha256']=hashlib.sha256(json.dumps(manifest['vectors'],sort_keys=True,separators=(',',':')).encode()).hexdigest()
    return manifest,json.dumps(manifest,indent=2,ensure_ascii=False)+'\n'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--datapack',type=Path,default=DP);parser.add_argument('--game',type=Path,default=GAME)
    parser.add_argument('--out',type=Path,default=OUT);parser.add_argument('--check',action='store_true')
    parser.add_argument('--facts',type=Path,help='verify actual-input formula facts, without a golden-vector lookup')
    args=parser.parse_args()
    try:
        if args.facts:
            facts=json.loads(args.facts.read_text(),parse_float=lambda value: (_ for _ in ()).throw(ValueError('float in evidence')))
            print('item_oracle: '+str(verify_facts(facts))+' actual-input facts passed')
            return 0
        manifest,content=generate(args.datapack,args.game)
    except (ValueError,KeyError,OSError,ET.ParseError,subprocess.CalledProcessError) as e:
        print('item_oracle: '+str(e),file=sys.stderr);return 2
    if args.check:
        if not args.out.exists() or args.out.read_text()!=content:print('item_oracle: stale fixture '+str(args.out),file=sys.stderr);return 1
    else:args.out.parent.mkdir(parents=True,exist_ok=True);args.out.write_text(content)
    print('item_oracle: '+str(len(manifest['vectors']))+' vectors '+json.dumps(manifest['coverage'],sort_keys=True)+'; '+('check passed' if args.check else 'generated'))
    return 0


if __name__=='__main__':sys.exit(main())
