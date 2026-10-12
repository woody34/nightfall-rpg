#!/usr/bin/env python3
"""Independent Fraction oracle for V6 item formulas and actual-input replay gates.

Reads pinned XML/Java directly, never generated item tables or Rust expectations.
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
from types import SimpleNamespace
import xml.etree.ElementTree as ET

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


def magic_damage(power,m_atk,m_def,spirit=False,blessed=False,shield_def=0,perfect=False,critical=False):
    if perfect:return {'lower':F(1),'upper':F(1),'damage':1}
    factor=4 if blessed else 2 if spirit else 1
    coefficient=91*exact(power)/(exact(m_def)+exact(shield_def))*(3 if critical else 1)
    result,lo,hi=floor_a_plus_b_root(0,coefficient,exact(m_atk)*factor)
    return {'lower':coefficient*lo,'upper':coefficient*hi,'damage':max(1,result)}


def cast_time(hit_time,casting_speed,spirit=False,blessed=False,static=False,is_magic=True):
    time=exact(hit_time)
    if not static:time*=333/exact(casting_speed)
    if is_magic and not static and (spirit or blessed):time*=F(3,5)
    time=trunc(time)
    return max(500,time) if hit_time>500 else time


def heal(power,m_atk,mp_consume2=0,mage=False,npc=False,grade='NONE',spirit=False,blessed=False,static=False,heal_effect=1,critical=False):
    if static:return {'amount':exact(power),'coefficient':F(0),'static_bonus':F(0)}
    static_bonus=F(0)
    if (mage or npc) and (spirit or blessed):
        coefficient=4 if blessed or npc else 2
        static_bonus=exact(mp_consume2)*(F(12,5) if blessed or npc else 1)
    else:
        coefficient={'S84':4,'S80':2}.get(grade,1)
        coefficient=coefficient*4 if blessed else coefficient+1
    multiplier=exact(heal_effect)*(3 if critical else 1)
    amount,lo,hi=floor_a_plus_b_root((exact(power)+static_bonus)*multiplier,multiplier,coefficient*exact(m_atk))
    return {'amount':amount,'coefficient':F(coefficient),'static_bonus':static_bonus,'root_lower':lo,'root_upper':hi}


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
            'model/items/L2Item.java','model/actor/L2Character.java','model/actor/instance/L2PcInstance.java',
            'model/stats/Formulas.java','network/clientpackets/RequestEnchantItem.java','network/clientpackets/RequestRefineCancel.java',
            'network/clientpackets/AbstractRefinePacket.java','datatables/AugmentationData.java','model/itemcontainer/PcInventory.java',
            'model/stats/functions/formulas/FuncPDefMod.java','model/stats/functions/formulas/FuncMDefMod.java',
            'enums/StatFunction.java']:
            self.read(G+file,True)
        for path in sorted((game/(G+'model/stats/functions/formulas')).glob('*.java')):
            self.read(path.relative_to(game).as_posix(),True)
        self.read('src/main/java/com/l2jserver/datapack/handlers/effecthandlers/instant/Heal.java')
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


def condition_matches(node,context):
    if node.tag in {'and','cond'}:return all(condition_matches(c,context) for c in node)
    if node.tag=='or':return any(condition_matches(c,context) for c in node)
    if node.tag=='not':return not any(condition_matches(c,context) for c in node)
    if node.tag=='using' and set(node.attrib)=={'kind'}:return context['weapon_type'] in node.get('kind').split(',') or context.get('armor_type') in node.get('kind').split(',')
    if node.tag=='using' and set(node.attrib)=={'slot'}:return node.get('slot') in context['slots']
    if node.tag=='player' and set(node.attrib) in ({'resting'},{'standing'},{'moving'},{'running'}):
        key=next(iter(node.attrib));states={'resting':context.get('sitting',False),'moving':context.get('moving',False),
            'standing':not (context.get('sitting',False) or context.get('moving',False)),'running':context.get('running',True)}
        return states[key]==(node.get(key)=='true')
    raise ValueError('unsupported oracle fold condition '+node.tag+str(node.attrib))


def gear_stats(src,class_file,level,equipment,skill_refs=(),base_deltas=None):
    """Equipment entries are (template_id, enchant, concrete slot, install sequence).

    Skills are explicit winning providers; no synthetic learned mastery or expertise.
    Full replay callers supply the winning sources and retained install sequences.
    """
    root=src.xml(class_file);static=root.find('staticData');gain=next(n for n in root.findall('lvlUpgainData/level') if int(n.get('val'))==level)
    base={k:int(static.findtext('base'+k)) for k in ['STR','DEX','CON','INT','WIT','MEN']}
    for key,delta in (base_deltas or {}).items():base[key.upper()]+=delta
    bonus={k:src.bonuses[k][v] for k,v in base.items()}
    lm=F(level+89,100)
    pslots={n.tag:F(n.text) for n in static.find('basePDef')}
    mslots={n.tag:F(n.text) for n in static.find('baseMDef')};mslots['lfinger']=mslots['rfinger']
    values={'pAtk':F(static.findtext('basePAtk')),'mAtk':F(static.findtext('baseMAtk')),'pDef':sum(pslots.values()),
        'mDef':sum(mslots.values()),'critRate':F(static.findtext('baseCritRate')),'pAtkSpd':F(static.findtext('basePAtkSpd')),
        'maxHp':F(gain.findtext('hp')),'maxMp':F(gain.findtext('mp')),'maxCp':F(gain.findtext('cp')),
        'mAtkSpd':F(static.findtext('baseMAtkSpd','333')),'runSpd':F(static.findtext('baseMoveSpd/run')),
        'regHp':max(F(1),F(gain.findtext('hpRegen'))*bonus['CON']*lm),
        'regMp':max(F(1),F(gain.findtext('mpRegen'))*bonus['MEN']*lm),
        'regCp':max(F(1),F(gain.findtext('cpRegen'))*bonus['CON']*lm),
        'atkCountMax':F(1),'pAtkAngle':F(static.findtext('baseDamRange/width')),'pAtkRange':F(static.findtext('baseAtkRange')),
        'critDmg':F(1),'critDmgAdd':F(0),'sDef':F(0),'rShld':F(0),'shieldDefAngle':F(0),'rEvas':F(0),'weightLimit':F(69000)*bonus['CON']//1}
    slots={row[2] for row in equipment};weapon_type='FIST';armor_type='NONE'
    for sid,enchant,slot,seq in equipment:
        p=src.properties(sid)
        if slot=='rhand':
            weapon_type=p.get('weapon_type','NONE')
            values['pAtkRange']=F(p.get('attack_range','40'))
            damage_parts=p.get('damage_range','').split(';')
            values['pAtkAngle']=F(damage_parts[3]) if len(damage_parts)>=4 and damage_parts[3].isdigit() else F(120)
        if slot=='chest':armor_type=p.get('armor_type','NONE')
    context={'slots':slots,'weapon_type':weapon_type,'armor_type':armor_type};funcs=[];hp_bonus=0;traits=[]
    if len({sid for sid,rank,seq in skill_refs})!=len(skill_refs):raise ValueError('duplicate winning skill provider')
    orders={'set':0,'enchant':0,'add':30,'sub':30,'mul':20,'div':20}
    for sid,enchant,slot,seq in equipment:
        file,line,item=src.items[sid];p=src.properties(sid)
        for ordinal,node in enumerate(item.findall('for/*')):
            if node.tag not in orders:raise ValueError('unsupported oracle function '+node.tag)
            stat=node.get('stat');value=F(node.get('val','0'))
            if node.tag=='enchant':value=enchant_increment(stat,p.get('crystal_type','NONE'),p.get('bodypart','none'),p.get('weapon_type','NONE'),enchant)
            funcs.append((int(node.get('order',orders[node.tag])),seq,(0,ordinal),stat,'add' if node.tag=='enchant' else node.tag,value,list(node)))
        if item.get('type')=='Armor' and p.get('crystal_type','NONE')!='NONE' and p.get('bodypart') in {'chest','legs','feet','head','gloves','back','fullarmor','onepiece','shirt','underwear','lhand','belt','waist'}:
            hp_bonus+=enchant_hp(src.hp.get('S' if p['crystal_type'] in {'S80','S84'} else p['crystal_type'],[]),enchant,p.get('bodypart') in {'fullarmor','onepiece'})
    for sid,rank,seq in skill_refs:
        file,skill=src.skills[sid];tables={n.get('name'):n.text.split() for n in skill.findall('table')}
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
                funcs.append((int(node.get('order',orders[node.tag])),seq,(effect_ordinal,ordinal),node.get('stat'),node.tag,F(value),list(node)))
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
    trace=[];accuracy_a=accuracy_b=F(0)
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
        old=values.get(stat,F(0 if op!='mul' else 1))
        if op in {'set','evasion'}:new=value
        elif op=='add':new=old+value
        elif op=='sub':new=old-value
        elif op=='mul':new=old*value
        elif op=='submul':new=(old-(psub if stat=='pDef' else msub))*value
        elif op=='div':new=old/value
        else:raise ValueError('unknown fold operation')
        values[stat]=new;trace.append({'order':order,'sequence':seq,'handler_ordinal':ordinal[0],'ordinal':ordinal[1],'stat':stat,'operation':op,'before':old,'operand':value,'after':new})
    weapon=next((sid for sid,e,slot,seq in equipment if slot=='rhand'),None)
    radius=fist_spread(level)['radius'] if weapon is None else int(src.properties(weapon).get('random_damage','0'))
    qvalues={k:q(v) for k,v in values.items()};getters={k:trunc(v) for k,v in values.items()}
    qvalues['accCombat']=floor_a_plus_b_root(accuracy_a*Q,accuracy_b*Q,base['DEX'])[0]
    getters['accCombat']=floor_a_plus_b_root(accuracy_a,accuracy_b,base['DEX'])[0]
    return {'exact':values,'root_expressions':{'accCombat':{'rational':accuracy_a,'root_coefficient':accuracy_b,'radicand':base['DEX']}},'q':qvalues,'getters':getters,
            'trace':trace,'base_stats':base,'weapon_type':weapon_type,'random_damage':radius,'spread_bound':2*radius+1,
            'enchant_hp':hp_bonus,'traits':traits,'weight':sum(int(src.properties(sid).get('weight','0')) for sid,e,slot,seq in equipment)}


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
        effects=[]
        for effect in skill.findall('effects/effect'):
            effects.append({'name':effect.get('name'),'params':{key:resolve(value) for n in effect.findall('param') for key,value in n.attrib.items()}})
        result.append({'id':sid,'rank':rank,'hit_time_ms':int(sets.get('hitTime','0')),
            'static':sets.get('isMagic')=='2','simultaneous':sets.get('simultaneousCast','false')=='true',
            'item_consume_id':int(sets.get('itemConsumeId','0')),'item_consume_count':int(sets.get('itemConsumeCount','0')),
            'effects':effects,'source':file})
    return {'template_id':template_id,'handler':properties.get('handler',''),
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
        retail_distribution,category,pack_options,cancel_fee,set_shield_history,clamp_no_heal,charge_transition]}
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
    def eligible(p):return not any(k in p for k in ['duration','time','item_skill','oncrit_skill','onmagic_skill','unequip_skill'])
    for i,kind in enumerate(['SWORD','BLUNT','DAGGER','BOW','DUAL','DUALFIST','POLE'],1):
        sid=src.choose(lambda sid,n,p:p.get('weapon_type')==kind and eligible(p) and n.find('cond') is None)
        gear=[(sid,0,'rhand',1)]
        add('F1',f'gear-{i:02d}-{kind}',{'class_file':human,'level':84,'equipment':gear,'expertise':7,'masteries':[]},gear_stats(src,human,84,gear),[src.citation(sid),*core_sources])
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
    for i,class_id,class_name in [(22,10,'HumanMystic'),(23,44,'OrcFighter'),(24,53,'DwarvenFighter')]:
        kit=next(n for n in kits.findall('equipment') if int(n.get('classId'))==class_id);gear=[];weight=0
        for seq,n in enumerate(kit.findall('item'),1):
            sid=int(n.get('id'));p=src.properties(sid);weight+=int(p.get('weight','0'))*int(n.get('count'))
            if n.get('equipped')=='true':
                body=p['bodypart'];slot='rhand' if body=='lrhand' else body
                gear.append((sid,0,slot,seq))
        class_file=D+'stats/chars/baseStats/'+class_name+'.xml'
        result=gear_stats(src,class_file,1,gear);result['inventory_weight']=weight;result['capacity']=100 if class_id==53 else 80
        result['kit']=[dict(n.attrib) for n in kit.findall('item')]
        add('F12',f'gear-{i:02d}-kit-{class_name}',{'class_file':class_file,'class_id':class_id,'equipment':gear,'level':1},result,[D+'stats/initialEquipment.xml:4-109',*core_sources])
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
    proxy=SimpleNamespace(xml=lambda file:injected,bonuses=src.bonuses,items=src.items,skills=src.skills,hp=src.hp,properties=src.properties)
    add('F1','RFINGER-duplication-unequal-injected-bases',{'class_file':human,'level':84,'injected_lfinger':999,'injected_rfinger':7},
        gear_stats(proxy,human,84,[]),[human,'G/model/actor/templates/L2PcTemplate.java:71-75','design/12.1 injected unequal finger bases'])
    for sid in [4270,6209,6213,3599]:
        for rank in range(1,int(src.skills[sid][1].get('levels'))+1):
            refs=[(sid,rank,100)];sheet=gear_stats(src,human,84,[],refs)
            for lifecycle in ['living','death','dead_admission','resurrection']:
                add('F1',f'system-{sid}-{rank}-{lifecycle}',{'skill_refs':refs,'lifecycle':lifecycle},sheet,[src.skills[sid][0],'design/D16 retained System providers'])
    # Every branch/grade at source enchant boundaries, including u32 storage endpoint.
    for grade in src.grades:
        for body,kind in [('rhand','SWORD'),('lrhand','POLE'),('lrhand','BOW'),('lrhand','DUAL')]:
            for e in [0,3,4,19,20,65535,4294967295]:
                for stat in ['pAtk','mAtk','pDef','mDef']:
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
        for residue in [499,500,749,750,999]:add('F10',f'blow-{pos}-{residue}',{'chance':50,'dex':1,'proximity':pos,'residue':residue},blow_success(50,1,pos,residue),['G/model/stats/Formulas.java:1621-1640','design/A8 per-mille'])
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
    return vectors


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


def independent_cosine(degrees):
    degrees=F(degrees)%360
    if degrees>180:degrees=360-degrees
    if degrees==0:return F(1),F(1)
    if degrees==180:return F(-1),F(-1)
    lower,upper=independent_pi();lower*=degrees/180;upper*=degrees/180
    def series(argument):
        partial=sum(((-1)**k*argument**(2*k)/factorial(2*k) for k in range(56)),F())
        return partial,partial+argument**112/factorial(112)
    return series(upper)[0],series(lower)[1]


def generate(dp=DP,game=GAME):
    src=Source(dp,game);vectors=make_vectors(src)
    # Independent angle enclosure: direct full-angle Taylor rather than generator's half-angle values.
    try:from gen_item_angles import generate_angles
    except ModuleNotFoundError:from packages.data.scripts.gen_item_angles import generate_angles
    table=generate_angles();angles=[]
    bound=table['coordinate_bound_millitiles'];maximum=4*bound**4
    for row in table['angles']:
        deg=row['degrees'];lo,hi=independent_cosine(deg)
        lower,upper=F(row['cos2_lower']),F(row['cos2_upper'])
        if lower!=upper and not (lower<=(1+lo)/2<=(1+hi)/2<=upper):raise ValueError('angle interval not independently enclosing')
        if lower!=upper and lower<=((lower+upper)/2).limit_denominator(maximum)<=upper:raise ValueError('angle rational separation failed')
        angles.append({'degrees':deg,'certified':row['certified'],'cos2_lower':lower,'cos2_upper':upper})
    for deg,relative in [(0,(1,0)),(90,(1,1)),(120,(1,2)),(180,(0,1)),(270,(-1,1)),(360,(-1,0))]:
        vectors.append({'id':f'angle-{deg}-{relative}','family':'F9','inputs':{'degrees':deg,'facing':[1,0],'relative':relative},
                        'expected':angle_contains((1,0),relative,deg,table),'sources':['design/D21 bounded certified interval geometry'],'draws':[]})
    for vector in vectors:
        vector['vector_sha256']=hashlib.sha256(json.dumps({k:v for k,v in vector.items() if k!='vector_sha256'},sort_keys=True,separators=(',',':')).encode()).hexdigest()
    if len({vector['id'] for vector in vectors})!=len(vectors):raise ValueError('duplicate oracle vector identity')
    manifest={'schema':1,'source_revisions':{'datapack':DP_REV,'game':GAME_REV},'source_file_sha256':dict(sorted(src.hashes.items())),
              'policy':'Independent pinned-source exact-rational oracle; injected random residues; V6 integer adaptations, not Java binary64 identity.',
              'coverage':dict(sorted(Counter(r['family'] for r in vectors).items())),
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
