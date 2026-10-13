#!/usr/bin/env python3
"""Generate V6 item rules from pinned L2J sources; no historical V5 input is changed."""
from __future__ import annotations

import argparse
from collections import Counter
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from xml.parsers import expat
import xml.etree.ElementTree as ET

try:
    import gen_class_phase3 as provenance
    import gen_skills
    from gen_tables import STARTER_CLASSES
except ModuleNotFoundError:
    from packages.data.scripts import gen_class_phase3 as provenance, gen_skills
    from packages.data.scripts.gen_tables import STARTER_CLASSES

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_DP = gen_skills.DEFAULT_DATAPACK_PATH
DEFAULT_GAME = DEFAULT_DP.parent / 'l2j-game'
DATAPACK_REVISION = gen_skills.DATAPACK_REVISION
GAME_REVISION = provenance.GAME_REV
D = 'src/main/resources/data/'
G = provenance.GAME_SRC
DOMAIN = b'nightfall.phase4.rules.1\0'
HP_PARTS = ['chest', 'feet', 'gloves', 'head', 'legs', 'back', 'fullarmor', 'onepiece', 'shirt', 'underwear', 'lhand', 'belt', 'waist']
ORDER = {'set': 0, 'enchant': 0, 'enchanthp': 40, 'add': 30, 'sub': 30, 'mul': 20, 'div': 20, 'share': 30}
NUMERIC_KEYS = {'value', 'chance', 'bonus_rate', 'weight_limit', 'tax_rate', 'cos_lower', 'cos_upper', 'cos2_lower', 'cos2_upper'}


class SourceError(ValueError):
    """Missing, dirty, ambiguous or unsupported source input."""


def ratio(value: str) -> str:
    q = F(value)
    return f'{q.numerator}/{q.denominator}'


def canonical(value):
    if isinstance(value, dict):
        return {k: (ratio(v) if (k in NUMERIC_KEYS or k == 'val') and isinstance(v, str) and re.fullmatch(r'[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?(?:/\d+)?', v) else canonical(v))
                for k, v in sorted(value.items()) if k not in {'source', 'sources', 'provenance', 'line', 'lines'}}
    if isinstance(value, list):
        return [canonical(v) for v in value]
    if isinstance(value, float):
        raise SourceError('binary float is forbidden')
    return value


def digest(value) -> str:
    return hashlib.sha256(DOMAIN + json.dumps(canonical(value), sort_keys=True,
        separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()


def xml(path: Path) -> dict:
    """Expat supplies exact live-node lines without matching comments or duplicate text."""
    stack = []
    roots = []
    parser = expat.ParserCreate()
    def start(tag, attrs):
        children = stack[-1]['children'] if stack else roots
        node = {'tag': tag, 'attributes': {k: v for k, v in attrs.items() if not k.startswith(('xmlns', 'xsi:'))},
                'text': '', 'line': parser.CurrentLineNumber, 'xml_ordinal': len(children), 'children': []}
        children.append(node)
        stack.append(node)
    def end(tag):
        node = stack.pop()
        node['text'] = node['text'].strip()
    def text(value):
        if stack:
            stack[-1]['text'] += value
    parser.StartElementHandler, parser.EndElementHandler, parser.CharacterDataHandler = start, end, text
    parser.ExternalEntityRefHandler = lambda *args: 0
    parser.Parse(path.read_bytes(), True)
    if len(roots) != 1:
        raise SourceError(f'{path}: expected one root')
    return roots[0]


def children(node, tag):
    return [n for n in node['children'] if n['tag'] == tag]


def walk(node):
    yield node
    for n in node['children']:
        yield from walk(n)


def attrs(node):
    return node['attributes']


def source(file, line, symbol, game=False):
    return {'repository': provenance.GAME_REPO if game else gen_skills.DATAPACK_REPOSITORY,
            'revision': GAME_REVISION if game else DATAPACK_REVISION, 'file': file,
            'lines': str(line), 'symbol': symbol}


class Inputs:
    def __init__(self, dp, game):
        provenance.admit(dp, DATAPACK_REVISION, [D, 'src/main/java/com/l2jserver/datapack/handlers/'])
        provenance.admit(game, GAME_REVISION, [G, provenance.CONFIG, 'src/main/resources/config/customs.properties'])
        self.dp, self.game = dp, game
        self.files = {}
    def read(self, file, game=False):
        root = self.game if game else self.dp
        raw = (root / file).read_bytes()
        self.files[(game, file)] = {'source': source(file, f'1-{len(raw.splitlines())}', 'source file', game),
                                  'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)}
        return raw.decode('utf-8')
    def xml(self, file):
        self.read(file)
        return xml(self.dp / file)


def functions(node, file):
    result = []
    for block in children(node, 'for'):
        for n in block['children']:
            a = attrs(n)
            op = n['tag']
            if op not in ORDER:
                raise SourceError(f'{file}:{n["line"]}: unknown func {op}')
            value = a.get('val', '0')
            F(value)  # Admit exact numeric lexical form before output.
            result.append({'stat': a['stat'], 'operation': op.upper(), 'order': int(a.get('order', ORDER[op])),
                           'xml_ordinal': n['xml_ordinal'], 'value': value, 'conditions': n['children'],
                           'source': source(file, n['line'], op + '/' + a['stat'])})
    return result


def refs(literal, kind):
    result = []
    for i, val in enumerate(filter(None, literal.split(';'))):
        match = re.fullmatch(r'(\d+)-(\d+)', val)
        if not match:
            raise SourceError(f'invalid skill reference {literal!r}')
        sid, rank = map(int, match.groups())
        if sid and rank:
            result.append({'id': sid, 'level': rank, 'kind': kind, 'xml_ordinal': i})
    return result


def java_slots(inputs):
    constants = dict((name, int(val, 0)) for name, val in re.findall(
        r'public static final int (SLOT_\w+) = (-?0x[0-9A-Fa-f]+|-?\d+);', inputs.read(G+'model/items/L2Item.java', True)))
    result = {}
    for name, expression in re.findall(r'SLOTS.put\("([^"]+)", (.+?)\);', inputs.read(G+'datatables/ItemTable.java', True)):
        value = 0
        for part in expression.split('|'):
            value |= constants[part.strip().removeprefix('L2Item.')]
        result[name] = value
    return result


def java_classifications(inputs):
    text=inputs.read(G+'model/items/L2Item.java',True);result={}
    for field,key in [('_common','is_common'),('_heroItem','is_hero'),('_pvpItem','is_pvp')]:
        match=re.search(re.escape(field)+r' = (.*?);',text,re.S)
        if match is None:raise SourceError('missing derived item flag '+field)
        expression=match[1]
        ranges=[(int(m[1]),int(m[2])) for m in re.finditer(r'_itemId >= (\d+)\) && \(_itemId <= (\d+)',expression)]
        singles=[int(m[1]) for m in re.finditer(r'_itemId == (\d+)',expression)]
        residue=re.sub(r'_itemId (?:>=|<=|==) \d+','',expression)
        if re.sub(r'[()&|\s]','',residue) or expression.count('_itemId')!=2*len(ranges)+len(singles):raise SourceError('unsupported item flag expression '+expression)
        result[key]={'ranges':ranges,'ids':singles}
    return result


def project_item(n, file, slots, classifications=None):
    p = {attrs(c)['name']: attrs(c)['val'] for c in children(n, 'set')}
    # Pinned duplicates follow StatsSet last-write semantics; raw retains every occurrence.
    a = attrs(n)
    body = p.get('bodypart', 'none')
    if body not in slots:
        raise SourceError(f'{file}:{n["line"]}: unknown bodypart {body}')
    row = {'template_id': int(a['id']), 'name': a['name'], 'kind': a['type'], 'grade': p.get('crystal_type', 'NONE'),
           'bodypart': body, 'bodypart_mask': slots[body], 'weapon_type': p.get('weapon_type', 'NONE').upper(),
           'armor_type': p.get('armor_type', 'NONE').upper(), 'etcitem_type': p.get('etcitem_type', 'NONE').upper(),
           'icon': p.get('icon', ''), 'handler': p.get('handler', ''),
           'stackable': p.get('is_stackable', 'false') == 'true', 'properties': p,
           'source': source(file, n['line'], 'item '+a['id']), 'raw': n}
    # L2EtcItem.java:51-55 overrides the literal XML type for shot actions.
    if row['kind'] == 'EtcItem' and p.get('default_action') in {'SOULSHOT', 'SUMMON_SOULSHOT', 'SUMMON_SPIRITSHOT', 'SPIRITSHOT'}:
        row['etcitem_type'] = 'SHOT'
    if row['kind'] == 'Armor' and row['armor_type'] == 'NONE' and body == 'lhand':
        row['armor_type'] = 'SHIELD'
    for key, default in [('weight', 0), ('price', 0), ('crystal_count', 0), ('enchant_enabled', 0),
        ('soulshots', 0), ('spiritshots', 0), ('random_damage', 0), ('attack_range', 40),
        ('mp_consume', 0), ('reuse_delay', 0), ('equip_reuse_delay', 0), ('shared_reuse_group', 0)]:
        row[key] = int(p.get(key, default)) * (1000 if key == 'equip_reuse_delay' else 1)
    row['damage_range'] = p.get('damage_range', '')
    damage_range = row['damage_range'].split(';')
    row['attack_angle'] = int(damage_range[3]) if len(damage_range) >= 4 and damage_range[3].isdigit() else 120
    row['flags'] = {k: p.get(k, default) == 'true' for k, default in {
        'is_sellable':'true', 'is_droppable':'true', 'is_destroyable':'true', 'is_tradable':'true',
        'is_depositable':'true', 'element_enabled':'false', 'is_questitem':'false', 'is_freightable':'false',
        'allow_self_resurrection':'false', 'is_oly_restricted':'false', 'for_npc':'false',
        'immediate_effect':'false', 'ex_immediate_effect':'false', 'is_magic_weapon':'false'}.items()}
    for key,rule in (classifications or {}).items():
        row['flags'][key]=row['template_id'] in rule['ids'] or any(lower<=row['template_id']<=upper for lower,upper in rule['ranges'])
    row['flags']['is_crystallizable']=row['grade']!='NONE' and row['crystal_count']>0
    for key in ['reduced_soulshot', 'reduced_mp_consume']:
        values = p.get(key, '').split(',')
        row[key] = [int(x) for x in values] if len(values) == 2 else []
    row['functions'] = functions(n, file)
    row['skills'] = refs(p.get('item_skill', ''), 'item')
    row['enchant4_skills'] = refs(p.get('enchant4_skill', ''), 'enchant4')
    row['conditions'] = children(n, 'cond')
    row['enchant_hp'] = row['kind'] == 'Armor' and row['grade'] != 'NONE' and body in HP_PARTS
    return row


def side_tables(inputs, items):
    tables = {}
    def put(name, value, files):
        tables[name+'.toml'] = {'schema': 1, 'sources': [source(f, '1-'+str(len(inputs.read(f).splitlines())), name) for f in files], **value}
    f = D+'stats/initialEquipment.xml'
    kits = []
    for n in children(inputs.xml(f), 'equipment'):
        kits.append({'class_id': int(attrs(n)['classId']), 'source': source(f,n['line'],'equipment'),
                     'items': [{'template_id':int(attrs(c)['id']), 'count':int(attrs(c)['count']),
                                'equipped':attrs(c).get('equipped','false')=='true', 'xml_ordinal':c['xml_ordinal'],
                                'source':source(f,c['line'],'starter item')} for c in children(n,'item')]})
    put('starter_kits', {'kits':kits}, [f])
    f = D+'stats/enchantHPBonus.xml'
    put('enchant_hp', {'eligible_bodyparts':HP_PARTS, 'grades':[{'grade':attrs(n)['grade'],
        'bonuses':[int(c['text']) for c in children(n,'bonus')], 'source':source(f,n['line'],'enchantHP')}
        for n in children(inputs.xml(f),'enchantHP')]}, [f])
    f = D+'enchantItemGroups.xml'
    rates, bindings = [], []
    for n in inputs.xml(f)['children']:
        if n['tag']=='enchantRateGroup':
            ranges=[]
            for c in children(n,'current'):
                bounds=attrs(c)['enchant'].split('-'); chance=attrs(c)['chance']; scaled=F(chance)*1000
                if scaled.denominator != 1: raise SourceError('chance not representable on 1e5 lattice')
                ranges.append({'min':int(bounds[0]), 'max':int(bounds[-1]), 'chance':chance,
                               'chance_1e5':int(scaled),'source':source(f,c['line'],'current')})
            rates.append({'name':attrs(n)['name'],'ranges':ranges,'raw':n,'source':source(f,n['line'],'rate group')})
        else:
            bindings.append({'id':int(attrs(n).get('id','0')),'raw':n,'source':source(f,n['line'],n['tag'])})
    put('enchant_groups', {'rates':rates,'bindings':bindings}, [f])
    f = D+'enchantItemData.xml'
    scrolls,supports,blacklist=[],[],[]
    for n in inputs.xml(f)['children']:
        a=attrs(n)
        if n['tag'] in {'enchant','support'}:
            sid=int(a['id']); p=items[sid]['properties']; t=items[sid]['etcitem_type']
            allowed=[]
            for c in n['children']:
                if 'id' in attrs(c):allowed.append(int(attrs(c)['id']))
            row={'template_id':sid,'target_grade':a.get('targetGrade','NONE'), 'bonus_rate':a.get('bonusRate','0'),
                 'max_enchant':int(a.get('maxEnchant','65535')), 'group_id':int(a.get('scrollGroupId','0')),
                 'weapon':t in {'ANCIENT_CRYSTAL_ENCHANT_WP','BLESS_SCRL_ENCHANT_WP','SCRL_ENCHANT_WP','SCRL_INC_ENCHANT_PROP_WP'}, 'blessed':t.startswith('BLESS_'),
                 'safe':t in {'ANCIENT_CRYSTAL_ENCHANT_AM','ANCIENT_CRYSTAL_ENCHANT_WP'}, 'allowed_items':allowed,'source':source(f,n['line'],n['tag']),'raw':n}
            (scrolls if n['tag']=='enchant' else supports).append(row)
        else:
            blacklist += [int(attrs(c)['id']) for c in walk(n) if 'id' in attrs(c)]
    put('enchant_scrolls', {'scrolls':scrolls,'supports':supports,'blacklist':blacklist}, [f])
    f=D+'enchantItemOptions.xml'
    put('enchant_options', {'items':[{'template_id':int(attrs(n)['id']),
        'levels':[{'level':int(attrs(c)['level']), 'option_ids':[int(v) for k,v in attrs(c).items() if k.startswith('option')],
                   'source':source(f,c['line'],'options')} for c in children(n,'options')],
        'source':source(f,n['line'],'item options'),'raw':n} for n in children(inputs.xml(f),'item')]},[f])
    sets=[]; files=[]
    for path in sorted((inputs.dp / (D+'stats/armorsets')).glob('*.xml')):
        file=path.relative_to(inputs.dp).as_posix();files.append(file)
        for n in children(inputs.xml(file),'set'):
            row={'id':int(attrs(n)['id']),'source':source(file,n['line'],'armor set'),'raw':n}
            for slot in ['chest','legs','head','gloves','feet','shield']:
                row[slot]=[int(attrs(c)['id']) for c in children(n,slot)]
            for tag,key in [('skill','skills'),('shield_skill','shield_skills'),('enchant6skill','enchant6_skills')]:
                row[key]=[{'id':int(attrs(c)['id']),'level':int(attrs(c)['level']), 'kind':tag,
                           'xml_ordinal':c['xml_ordinal']} for c in children(n,tag)]
            row['base_stats']={c['tag']:int(attrs(c)['val']) for c in n['children'] if c['tag'] in {'str','dex','con','int','wit','men'}}
            sets.append(row)
    put('armor_sets', {'sets':sets},files)
    options=[];files=[]
    for path in sorted((inputs.dp/(D+'stats/options')).glob('*.xml')):
        file=path.relative_to(inputs.dp).as_posix();files.append(file)
        for n in children(inputs.xml(file),'option'):
            skills=[]
            for c in walk(n):
                a=attrs(c)
                if c['tag'] in {'skill','trigger','activeSkill','passiveSkill','active_skill','passive_skill','activation_skill','attack_skill','magic_skill'} and ('id' in a or 'skillId' in a):
                    skills.append({'id':int(a.get('skillId',a.get('id'))), 'level':int(a.get('skillLevel',a.get('level','1'))),
                                   'kind':c['tag'],'xml_ordinal':c['xml_ordinal']})
            options.append({'id':int(attrs(n)['id']), 'name':attrs(n).get('name',''), 'functions':functions(n,file),
                            'skills':skills,'source':source(file,n['line'],'option'),'raw':n})
    put('augmentation_options',{'options':options},files)
    f=D+'stats/augmentation/augmentation_skillmap.xml'
    skillmap=[]
    for n in children(inputs.xml(f),'augmentation'):
        fields={c['tag']:attrs(c)['val'] for c in n['children']}
        skillmap.append({'option_id':int(attrs(n)['id']), 'id':int(fields['skillId']), 'level':int(fields['skillLevel']),
                         'color':fields['type'],'source':source(f,n['line'],'augmentation'),'raw':n})
    put('augmentation_skills',{'skills':skillmap},[f])
    rows=[];files=[]
    for kind,name in [('weapon','retailchances.xml'),('accessory','retailchances_accessory.xml')]:
        file=D+'stats/augmentation/'+name;files.append(file)
        for n in inputs.xml(file)['children']:
            rows.append({'kind':kind,'xml_ordinal':n['xml_ordinal'],'source':source(file,n['line'],'retail chances'),'raw':n})
    ref=G+'network/clientpackets/AbstractRefinePacket.java';java=inputs.read(ref,True)
    levels=[int(x) for x in re.search(r'LEVELS = \{(.*?)\};',java,re.S)[1].replace('\n','').split(',') if x.strip()]
    stones=[]
    for m in re.finditer(r'_lifeStones.put\((\d+), new LifeStone\(GRADE_(\w+), (\d+)\)\)',java):
        sid,tier,index=m.groups();line=java[:m.start()].count('\n')+1
        stones.append({'template_id':int(sid),'kind':'accessory' if tier=='ACC' else 'weapon',
                       'tier':{'NONE':0,'MID':1,'HIGH':2,'TOP':3,'ACC':4}[tier], 'level':levels[int(index)],
                       'source':source(ref,line,'life stone',True)})
    costs=[]
    for grade,gem,w,a in [('C',2130,20,200),('B',2130,30,300),('A',2131,20,200),('S',2131,25,250),('S80',2132,36,360),('S84',2132,36,480)]:
        costs.append({'grade':grade,'gemstone_id':gem,'weapon_count':w,'accessory_count':a,'source':source(ref,'380-414','gemstone cost',True)})
    fees=[];fee_file=G+'network/clientpackets/RequestRefineCancel.java';inputs.read(fee_file,True)
    for grade,schedule in [('C',[(1720,95000),(2452,150000),(0,210000)]),('B',[(1746,240000),(0,270000)]),
                           ('A',[(2160,330000),(2824,390000),(0,420000)]),('S',[(0,480000)]),('S80',[(0,920000)]),('S84',[(0,920000)])]:
        for limit,adena in schedule:fees.append({'grade':grade,'crystal_count_below':limit,'adena':adena,'source':source(fee_file,'64-109','cancel fee',True)})
    put('augmentation_chances',{'rows':rows,'weapon_categories':[5500,3500,700,300], 'accessory_categories':[5500,3500,900,100],
        'selector':'xml_order_descending_tail','stones':stones,'costs':costs,'cancel_fees':fees,
        'inactive_policy':{'branch':'non_retail','stat_subblock':'91','stat_block':'3640','runtime':'disabled'}},files)
    f=D+'levelUpCrystalData.xml';root=inputs.xml(f)
    transitions=[];npcs=[]
    for n in root['children']:
        if n['tag']=='crystal':
            for c in children(n,'item'):
                a=attrs(c);transitions.append({'template_id':int(a['itemId']),'stage':int(a['level']),
                    'leveled_template':int(a['leveledItemId']),'source':source(f,c['line'],'crystal transition')})
        else:npcs.append({'source':source(f,n['line'],'soul absorb data'),'raw':n})
    put('soul_crystals',{'transitions':transitions,'npcs':npcs,'absorb_policy':'DeferredPhase6_no_rng'},[f])
    crystal_ids={r['template_id'] for r in transitions}|{r['leveled_template'] for r in transitions}
    recipes=[];files=[]
    for path in sorted((inputs.dp/(D+'multisell')).glob('*.xml')):
        # Parse independently first; record only relevant lists in the source inventory.
        root=xml(path);found=[]
        for ordinal,n in enumerate(children(root,'item')):
            ingredients=[{'template_id':int(attrs(c)['id']),'count':int(attrs(c)['count'])} for c in children(n,'ingredient')]
            products=[{'template_id':int(attrs(c)['id']),'count':int(attrs(c)['count'])} for c in children(n,'production')]
            weapons=[r['template_id'] for r in ingredients if r['template_id'] in items and items[r['template_id']]['kind']=='Weapon']
            if len(weapons)!=1 or not any(r['template_id'] in crystal_ids for r in ingredients):continue
            if not products or any(r['template_id'] not in items or items[r['template_id']]['kind']!='Weapon' for r in products):continue
            file=path.relative_to(inputs.dp).as_posix();list_id=int(path.stem);a=attrs(root)
            found.append({'recipe_id':list_id*4294967296+ordinal,'list_id':list_id,'xml_ordinal':ordinal,
                'base_template':weapons[0],'ingredients':ingredients,'products':products,
                'maintain_enchantment':a.get('maintainEnchantment','false')=='true', 'apply_taxes':a.get('applyTaxes','false')=='true',
                'source':source(file,n['line'],'SA exchange'),'raw':n})
        if found:
            inputs.read(path.relative_to(inputs.dp).as_posix());files.append(path.relative_to(inputs.dp).as_posix());recipes+=found
    put('sa_variants',{'recipes':recipes},files)
    return tables


def constants(inputs):
    tables={}
    file=G+'model/items/type/WeaponType.java';java=inputs.read(file,True)
    types=[]
    for i,m in enumerate(re.finditer(r'^\s*(\w+)\(TraitType\.(\w+)\)',java,re.M)):
        types.append({'kind':m[1],'ordinal':i,'mask':1<<i,'trait':m[2],
                      'source':source(file,java[:m.start()].count('\n')+1,'weapon type',True)})
    tables['weapon_types.toml']={'schema':1,'sources':[source(file,'27-63','weapon types',True)],'types':types}
    file=G+'model/items/type/CrystalType.java';java=inputs.read(file,True);grades=[]
    for m in re.finditer(r'^\s*(\w+)\((\d+), (\d+), (\d+), (\d+)\)',java,re.M):
        grades.append({'grade':m[1],'id':int(m[2]),'crystal_id':int(m[3]),'armor_bonus':int(m[4]),'weapon_bonus':int(m[5]),
            'shot_grade':'S' if m[1] in {'S80','S84'} else m[1], 'source':source(file,java[:m.start()].count('\n')+1,'crystal type',True)})
    tables['grades.toml']={'schema':1,'sources':[source(file,'26-33','grades',True)],'grades':grades}
    file=G+'model/stats/functions/FuncEnchant.java';java=inputs.read(file,True)
    attack=[]
    # Assert the pinned implementation before projecting its case table.
    for literal in ['case S:', 'case A:', 'case B:', 'case C:', 'getBodyPart() == L2Item.SLOT_LR_HAND']:
        if literal not in java:raise SourceError('FuncEnchant source changed: '+literal)
    for grade,units in [('NONE',(2,2,2,4)),('D',(2,2,2,4)),('C',(3,3,4,6)),('B',(3,3,4,6)),('A',(3,4,5,8)),('S',(4,5,6,10)),('S80',(4,5,6,10)),('S84',(4,5,6,10))]:
        attack.append(dict(zip(['grade','m_atk','p_atk','p_atk_two_hand','p_atk_bow'],[grade,*units]),source=source(file,'77-151','enchant units',True)))
    tables['func_enchant.toml']={'schema':1,'sources':[source(file,'43-155','FuncEnchant',True)],'attack':attack,
        'safe_levels':3,'attack_over_multiplier':2,'defence_over_multiplier':3,'hp_order':40}
    config=inputs.read(provenance.CONFIG,True)
    def setting(key):
        m=re.search(r'^'+key+r'\s*=\s*([^\r\n]+)',config,re.M)
        if not m:raise SourceError('missing config '+key)
        return m[1].strip()
    capacities={'schema':1,'sources':[source(provenance.CONFIG,'27,317-320','capacities',True),
         source(G+'model/actor/instance/L2PcInstance.java','1668-1693,9427-9438','load policy',True)],
        'inventory':int(setting('MaximumSlotsForNoDwarf')),'dwarf_inventory':int(setting('MaximumSlotsForDwarf')),
        'quest_inventory':int(setting('MaximumSlotsForQuestItems')),'weight_limit':setting('WeightLimit'),
        'load_multiplier':69000,'weight_penalty_base':1,'overload_thresholds':[500,666,800,1000],
        'blacksmith':{'template':'blacksmith','x':124,'y':128,'radius_millitiles':3000,'tax_rate':'0'},
        'policy':{'formula_revision':'1','facing':'primitive_last_nonzero_movement_initial_east',
                  'overloaded':'change_only_on_penalty_rank_transition','system_sources':'retain_equipment_across_death',
                  'augmentation':'retail_only','weapon_conversion':'deferred_ConvertItem_skill_mechanic','glow':'placeholder_weapon_7_15_armor_6'}}
    tables['capacities.toml']=capacities
    return tables


def condition_blockers(node):
    tag=node['tag'];a=attrs(node)
    if tag in {'cond','and','or','not'}:
        own=[] if tag=='cond' or not a else ['condition:'+tag]
        return own + [b for c in node['children'] for b in condition_blockers(c)]
    allowed={'player':{'races','race','classes','classId','sex','level','weight','invSize','canResurrect','flyMounted'},
             'using':{'kind','slot'}}
    if tag in allowed and a and set(a)<=allowed[tag]:return []
    return ['condition:'+tag+':'+','.join(sorted(a))]


def local_blockers(item):
    p=item['properties'];blockers=set()
    name=item['name'].lower()
    if item['flags'].get('is_common',False) or name.startswith('common item -'):blockers.add('context:common')
    if int(p.get('duration','-1'))!=-1:blockers.add('context:shadow')
    if int(p.get('time','-1'))!=-1:blockers.add('context:time_limited')
    if item['flags']['for_npc']:blockers.add('context:npc_only')
    if item['weapon_type'] not in gen_skills.ALLOWED_WEAPON_KINDS: blockers.add('weapon_kind:'+item['weapon_type'])
    if item['bodypart_mask']<0:blockers.add('context:pet')
    if item['bodypart'] in {'hair','hair2','dhair','alldress','rbracelet','lbracelet','deco1'}:blockers.add('slot:'+item['bodypart'])
    for key in ['oncrit_skill','onmagic_skill','unequip_skill','equip_condition','use_condition']:
        if key in p:blockers.add('property:'+key)
    for n in item['conditions']:blockers.update(condition_blockers(n))
    for f in item['functions']:
        if f['stat'] not in gen_skills.ALLOWED_STATS|gen_skills.PHASE4_STATS:blockers.add('stat:'+f['stat'])
        for n in f['conditions']:blockers.update(condition_blockers(n))
        if f['operation']=='SHARE':blockers.add('op:SHARE')
    return blockers


def skill_targets(items,tables):
    target={4270,6209,6213,3599,239,227,231,233,248}
    for row in items.values():
        for ref in row['skills']+row['enchant4_skills']:target.add(ref['id'])
        for key in ['oncrit_skill','onmagic_skill','unequip_skill']:
            for ref in refs(row['properties'].get(key,''),key):target.add(ref['id'])
    for row in tables['armor_sets.toml']['sets']:
        for ref in row['skills']+row['shield_skills']+row['enchant6_skills']:target.add(ref['id'])
    for row in tables['augmentation_options.toml']['options']:
        target.update(r['id'] for r in row['skills'])
    target.update(r['id'] for r in tables['augmentation_skills.toml']['skills'])
    return target


def skill_manifest(inputs,targets):
    dp=gen_skills.Datapack(inputs.dp);all_skills=gen_skills.load_all_datapack_skills(dp,phase4=True)
    catalog=tomllib.loads((ROOT/'skill_catalog.toml').read_text())['skills'];base={r['id'] for r in catalog}|{5076,286}
    base_ids,base_blockers=gen_skills.compute_all_blockers(gen_skills.load_all_datapack_skills(dp),base)
    # The V6 contract extensions are explicit; V5 generator defaults never mutate.
    ids,blockers=gen_skills.compute_all_blockers(all_skills,base|targets,phase4=True)
    if 286 in blockers:blockers[286].add('sample:out_of_scope')
    files={};rows=[]
    for sid in sorted(ids):
        if sid not in all_skills:
            rows.append({'id':sid,'max_level':0,'admitted':False,'blockers':sorted(blockers[sid])});continue
        file,node=all_skills[sid]
        inputs.read(file)
        files[f'skills-phase4/{sid}.toml']=gen_skills.generate_skill_toml(sid,file,node,blockers[sid],is_sample_only=sid==286,phase4=True)
        rows.append({'id':sid,'max_level':int(node.get('levels','1')),'admitted':not blockers[sid] and sid!=286,'blockers':sorted(blockers[sid])})
    changes=[{'id':sid,'before':sorted(base_blockers.get(sid,{'not_imported'})), 'after':sorted(blockers[sid])}
             for sid in sorted(ids) if base_blockers.get(sid,{'not_imported'})!=blockers[sid]]
    counts={'phase3':{'imported':len(base_ids),'admitted':sum(not base_blockers[s] for s in base_ids),
                     'deferred':sum(bool(base_blockers[s]) for s in base_ids)},
            'phase4':{'imported':len(ids),'admitted':sum(r['admitted'] for r in rows),'deferred':sum(not r['admitted'] for r in rows)},
            'changed_definitions':changes,'trees':[]}
    for path in sorted((ROOT/'professions').glob('*.toml')):
        p=tomllib.loads(path.read_text());tree={r['skill_id'] for r in p.get('skill_tree',[])}
        if tree:
            counts['trees'].append({'class':path.stem,'imported':len(tree),
                'phase3_admitted':sum(not base_blockers[s] for s in tree),'phase4_admitted':sum(not blockers[s] for s in tree),
                'phase4_deferred':sum(bool(blockers[s]) for s in tree)})
    return rows,files,counts


def closure(items,tables):
    etc={57,5575,1458,1459,1460,1461,1462,2130,2131,2132}
    for row in items.values():
        p=row['properties'];t=row['etcitem_type'];handler=row['handler'];action=p.get('default_action','')
        if row['kind']=='EtcItem' and (t in {'ARROW','BOLT','SHOT','POTION','ELIXIR'} or action in {'SOULSHOT','SPIRITSHOT'}
           or handler in {'SoulShots','SpiritShot','BlessedSpiritShot','Potion','Elixir','ScrollOfEscape','ScrollOfResurrection'}
           or t=='SCROLL' and any(name in row['name'] for name in ['Escape','Resurrection'])):etc.add(row['template_id'])
    for kit in tables['starter_kits.toml']['kits']:etc.update(r['template_id'] for r in kit['items'])
    for key in ['scrolls','supports']:etc.update(r['template_id'] for r in tables['enchant_scrolls.toml'][key])
    etc.update(r['template_id'] for r in tables['augmentation_chances.toml']['stones'])
    for row in tables['soul_crystals.toml']['transitions']:etc.update([row['template_id'],row['leveled_template']])
    for row in tables['sa_variants.toml']['recipes']:etc.update(r['template_id'] for r in row['ingredients']+row['products'])
    for path in sorted((ROOT/'professions').glob('*.toml')):
        p=tomllib.loads(path.read_text())
        for row in p.get('skill_tree',[]):
            for req in row.get('required_items',[]):
                item=req.get('item_id')
                if item is None and 'item' in req:item=int(req['item'].split('.')[-1])
                if item is not None:etc.add(item)
    unknown=etc-items.keys()
    if unknown:raise SourceError('closure contains missing items '+str(sorted(unknown)))
    return {sid:r for sid,r in items.items() if r['kind']!='EtcItem' or sid in etc}


def capabilities(items,tables,skills,skill_files):
    skill_index={r['id']:r for r in skills}
    skill_data={s['id']:s for s in (tomllib.loads(content) for content in skill_files.values())}
    def dependencies(refs):
        result=set()
        for ref in refs:
            s=skill_index.get(ref['id'])
            if s is None or not s['admitted'] or not 1<=ref['level']<=s['max_level']:
                result.add(f'skill:{ref["id"]}:{ref["level"]}')
        return result
    set_by_chest={}
    for row in tables['armor_sets.toml']['sets']:
        for sid in row['chest']:
            set_by_chest.setdefault(sid,[]).extend(row['skills']+row['shield_skills']+row['enchant6_skills'])
    option_rows=[]
    for row in tables['augmentation_options.toml']['options']:
        b=dependencies(row['skills'])
        for ref in row['skills']:
            if ref['kind'] in {'activation_skill','trigger','attack_skill','magic_skill'}:b.add('augment:trigger')
        for f in row['functions']:
            if f['stat'] not in gen_skills.ALLOWED_STATS|gen_skills.PHASE4_STATS:b.add('stat:'+f['stat'])
            for c in f['conditions']:b.update(condition_blockers(c))
        option_rows.append({'id':row['id'],'admitted':not b,'blockers':sorted(b)})
    option_index={r['id']:r for r in option_rows}
    enchant_options={r['template_id']:[oid for level in r['levels'] for oid in level['option_ids']] for r in tables['enchant_options.toml']['items']}
    augment=[]
    for row in tables['augmentation_chances.toml']['rows']:
        for stone in children(row['raw'],'stone'):
            b=set()
            for n in walk(stone):
                if n['tag']=='augment':
                    oid=int(attrs(n)['id'])
                    if oid not in option_index or not option_index[oid]['admitted']:b.add('option:'+str(oid))
            augment.append({'kind':row['kind']+':'+attrs(row['raw'])['type'],'stone_id':int(attrs(stone)['id']),
                            'admitted':not b,'blockers':sorted(b)})
    rows=[]
    supported_handlers={'','SoulShots','SpiritShot','BlessedSpiritShot','ItemSkills','ItemSkillsTemplate','EnchantScrolls','ManaPotion','Potion','Elixir','ScrollOfEscape','ScrollOfResurrection'}
    for sid,row in sorted(items.items()):
        b=local_blockers(row)|dependencies(row['skills']+row['enchant4_skills']+set_by_chest.get(sid,[]))
        for oid in enchant_options.get(sid,[]):
            if oid not in option_index or not option_index[oid]['admitted']:b.add('enchant_option:'+str(oid))
        if row['handler']=='ManaPotion' and tables['capacities.toml']['policy']['mana_potion_support']=='false':b.add('configuration:mana_potions_disabled')
        use=set()
        if row['handler'] not in supported_handlers:use.add('handler:'+row['handler'])
        # EnchantScrolls selects a target; EnchantItem performs the actual operation.
        # Passive/currency/material item without a handler remains storable; no fictitious UseItem.
        if row['kind']=='EtcItem' and row['handler']=='' and row['etcitem_type'] not in {'ARROW','BOLT'}:
            use.add('use:no_handler')
        if row['kind']=='EtcItem' and use and row['skills']:b.update(use)
        rows.append({'template_id':sid,'admitted':not b,
                     'storable':not bool(b & {'context:common','context:shadow','context:time_limited','context:npc_only','context:pet'}),'blockers':sorted(b),'use_blockers':sorted(b|use)})
    item_index={r['template_id']:r for r in rows}
    consumables={}
    for sid,item in sorted(items.items()):
        required=[]
        for ref in item['skills']+item['enchant4_skills']+set_by_chest.get(sid,[]):
            skill=skill_data.get(ref['id'],{})
            rank=ref['level']
            if not 1<=rank<=skill.get('max_level',0):continue
            for req in skill.get('item_consume',[]):
                target=req['item_id'][rank-1] if isinstance(req['item_id'],list) else req['item_id']
                count=req['count'][rank-1] if isinstance(req['count'],list) else req['count']
                if target and count>0:
                    required.append((target,f'consumable:skill:{ref["id"]}:{rank}:item:{target}'))
        consumables[sid]=required
    # D54: propagate deferral through consumed items, including self/cyclic costs.
    changed=True
    while changed:
        changed=False
        for row in rows:
            b={blocker for target,blocker in consumables[row['template_id']]
               if target not in item_index or not item_index[target]['storable'] or not item_index[target]['admitted']}
            if b:
                changed=changed or row['admitted']
                row['admitted']=False
                row['blockers']=sorted(set(row['blockers'])|b)
                row['use_blockers']=sorted(set(row['use_blockers'])|b)
    recipes=[]
    for recipe in tables['sa_variants.toml']['recipes']:
        b={'item:'+str(r['template_id']) for r in recipe['ingredients']+recipe['products']
           if r['template_id'] not in item_index or not item_index[r['template_id']]['admitted'] or not item_index[r['template_id']]['storable']}
        recipes.append({'recipe_id':recipe['recipe_id'],'admitted':not b,'blockers':sorted(b)})
    return {'schema':1,'sources':[],'items':rows,'skills':skills,'options':option_rows,'augmentation':augment,'recipes':recipes}


def encode_toml(data):
    lines=['# GENERATED by packages/data/scripts/gen_items.py from pinned L2J sources; see SOURCES.md.','']
    gen_skills.emit_fields(lines,data)
    content='\n'.join(lines)+'\n'
    if tomllib.loads(content)!=data:raise SourceError('TOML changed structure')
    canonical(data)
    return content


def class_slots(inputs):
    """D35: immutable base class slot getters, including the pinned RFINGER quirk."""
    game_sources=[source(G+'model/actor/templates/L2PcTemplate.java','64-75','base slot getters',True),
                  source(G+'data/xml/impl/PlayerTemplateData.java','106-108','slot sums; RFINGER twice',True)]
    for citation in game_sources:inputs.read(citation['file'],True)
    rows=[]
    for stem,key,name,race,archetype in STARTER_CLASSES:
        file=D+'stats/chars/baseStats/'+stem+'.xml';root=inputs.xml(file)
        static=children(root,'staticData')[0];physical=children(static,'basePDef')[0];magical=children(static,'baseMDef')[0]
        def slots(node,names):
            values={n['tag']:int(n['text']) for n in node['children']}
            if set(values)!=set(names):raise SourceError('unexpected defence slots: '+file)
            return {name:values[name] for name in names}
        pslots=slots(physical,['chest','legs','head','feet','gloves','underwear','cloak'])
        pslots['under']=pslots.pop('underwear')
        literal=slots(magical,['rear','lear','rfinger','lfinger','neck'])
        effective=dict(literal,lfinger=literal['rfinger'])
        lines=str(physical['line'])+'-'+str(max(n['line'] for n in walk(magical)))
        rows.append({'base_class_id':int(children(root,'classId')[0]['text']),'key':key,
                     'p_def_slots':pslots,'m_def_slots_xml':literal,'m_def_slots':effective,
                     'source':source(file,lines,'basePDef/baseMDef')})
    if len({r['base_class_id'] for r in rows})!=len(STARTER_CLASSES):raise SourceError('duplicate base class')
    return {'schema':1,'sources':game_sources,'classes':sorted(rows,key=lambda r:r['base_class_id'])}


COMBAT_FIELDS=[('baseAtkType','attack_type',str),('baseAtkRange','attack_range',int),('basePAtk','p_atk',int),
               ('basePAtkSpd','p_atk_spd',int),('baseCritRate','crit_rate',int)]


def class_combat(inputs):
    """D75: per base class unarmed (no active weapon) combat bases, with a lineage census."""
    game_sources=[source(G+'model/actor/templates/L2CharTemplate.java','107,111,114,116,118','template loader defaults',True),
                  source(G+'data/xml/impl/PlayerTemplateData.java','98-103','staticData leaf names',True),
                  source(G+'model/actor/instance/L2PcInstance.java','1018-1019,7668-7674','template follows current class',True),
                  source(G+'model/actor/stat/CharStat.java','424-435','no weapon: template baseAtkRange',True),
                  source(G+'model/actor/stat/CharStat.java','438-447','no weapon: attack angle 120',True),
                  source(G+'model/actor/stat/CharStat.java','145,399,410','template crit/pAtk/pAtkSpd bases',True),
                  source(G+'model/actor/L2Character.java','5034-5044','no weapon: random 5+(int)sqrt(level)',True),
                  source(G+'model/actor/L2Character.java','5560-5572','no weapon: template baseAtkType',True),
                  # D84: an unarmed player's active weapon is the class fists template.
                  source(G+'model/actor/instance/L2PcInstance.java','2141-2183','findFistsWeaponItem class ranges',True),
                  source(G+'model/actor/instance/L2PcInstance.java','4147-4155','getActiveWeaponItem falls back to fists',True)]
    text={citation['file']:inputs.read(citation['file'],True) for citation in game_sources}
    angle=re.search(r'baseAttackAngle = (\d+);',text[G+'model/actor/stat/CharStat.java'])
    radius=re.search(r'random = (\d+) \+ \(int\) Math\.sqrt\(getLevel\(\)\);',text[G+'model/actor/L2Character.java'])
    if angle is None or radius is None:raise SourceError('missing unarmed angle/random-damage source')
    pc=text[G+'model/actor/instance/L2PcInstance.java']
    body=re.search(r'public L2Weapon findFistsWeaponItem\(int classId\) \{(.*?)\n\t\treturn weaponItem;',pc,re.S)
    if body is None:raise SourceError('missing findFistsWeaponItem')
    fists=[{'class_min':int(lo,16),'class_max':int(hi,16),'template_id':int(tid)} for lo,hi,tid in re.findall(
        r'\(classId >= 0x([0-9a-f]+)\) && \(classId <= 0x([0-9a-f]+)\)\) \{[^{}]*?getTemplate\((\d+)\)',body[1])]
    if len(fists)!=9 or any(f['class_min']>f['class_max'] for f in fists):raise SourceError('unexpected fists ranges')
    graph_file=D+'stats/chars/classList.xml'
    parents={int(attrs(n)['classId']):(int(attrs(n)['parentClassId']) if 'parentClassId' in attrs(n) else None)
             for n in children(inputs.xml(graph_file),'class')}
    def root_of(cid):
        seen=set()
        while parents.get(cid) is not None:
            if cid in seen:raise SourceError('cyclic class graph')
            seen.add(cid);cid=parents[cid]
        return cid
    def values(root,file):
        static=children(root,'staticData')[0];row={};lines=[]
        for tag,key,kind in COMBAT_FIELDS:
            found=children(static,tag)
            if len(found)!=1 or found[0]['children']:raise SourceError(f'{file}: missing/complex {tag}')
            text=found[0]['text'].strip();row[key]=text if kind is str else int(text);lines.append(found[0]['line'])
        if row['attack_type']!='FIST':raise SourceError(f'{file}: unexpected baseAtkType')
        return row,lines
    census={}
    for path in sorted((inputs.dp/(D+'stats/chars/baseStats')).glob('*.xml')):
        file=path.relative_to(inputs.dp).as_posix();root=inputs.xml(file)
        cid=int(children(root,'classId')[0]['text']);census[cid]=(file,)+values(root,file)
    rows=[]
    for stem,key,name,race,archetype in STARTER_CLASSES:
        file=D+'stats/chars/baseStats/'+stem+'.xml';root=inputs.xml(file)
        base_id=int(children(root,'classId')[0]['text'])
        if parents.get(base_id,0) is not None:raise SourceError('starter class is not a graph root: '+file)
        row,lines=values(root,file)
        lineage=sorted(cid for cid in census if root_of(cid)==base_id)
        for cid in lineage:
            if census[cid][1]!=row:raise SourceError(f'{census[cid][0]}: unarmed bases differ from base class {base_id}')
        rows.append(dict({'base_class_id':base_id,'key':key},**row,lineage_class_ids=lineage,
                         source=source(file,','.join(map(str,sorted(lines))),'baseAtkType/baseAtkRange/basePAtk/basePAtkSpd/baseCritRate')))
    if len({r['base_class_id'] for r in rows})!=len(STARTER_CLASSES):raise SourceError('duplicate base class')
    return {'schema':1,'sources':game_sources+[source(graph_file,'1-'+str(max(n['line'] for n in walk(inputs.xml(graph_file)))),'class lineage')],
            'unarmed':{'attack_angle':int(angle[1]),'random_damage_radius_base':int(radius[1])},
            'fists':fists,
            'classes':sorted(rows,key=lambda r:r['base_class_id'])}


def generate(dp=DEFAULT_DP,game=DEFAULT_GAME):
    inputs=Inputs(dp,game);slots=java_slots(inputs);classifications=java_classifications(inputs);all_items={};ledger=[]
    for rel in ['model/items/L2Weapon.java','model/items/L2Armor.java','model/items/L2EtcItem.java','enums/StatFunction.java','model/actor/instance/L2PcInstance.java','network/clientpackets/RequestEnchantItem.java','model/items/enchant/AbstractEnchantItem.java','model/items/enchant/EnchantScroll.java','model/items/enchant/EnchantSupportItem.java','data/xml/impl/EnchantItemHPBonusData.java','datatables/AugmentationData.java']:
        inputs.read(G+rel,True)
    for path in sorted((dp/(D+'stats/items')).glob('*.xml')):
        file=path.relative_to(dp).as_posix()
        for n in children(inputs.xml(file),'item'):
            r=project_item(n,file,slots,classifications);sid=r['template_id']
            if sid in all_items:raise SourceError('duplicate template '+str(sid))
            all_items[sid]=r
            ledger.append({'template_id':sid,'kind':r['kind'],'source':r['source'],'node_sha256':digest(n)})
    tables=side_tables(inputs,all_items);tables.update(constants(inputs))
    tables['class_slots.toml']=class_slots(inputs)
    tables['class_combat.toml']=class_combat(inputs)
    blacklist=re.search(r'^EnchantBlacklist\s*=\s*([^\r\n]+)',inputs.read(provenance.CONFIG,True),re.M)
    if blacklist is None:raise SourceError('missing enchant blacklist')
    tables['enchant_scrolls.toml']['blacklist']=[int(x.strip()) for x in blacklist[1].split(',')]
    tables['enchant_scrolls.toml']['sources'].append(source(provenance.CONFIG,'364','EnchantBlacklist',True))
    config=inputs.read(provenance.CONFIG,True)
    def config_value(key):
        found=re.search(r'^'+key+r'\s*=\s*([^\r\n]+)',config,re.M)
        if found is None:raise SourceError('missing augmentation setting '+key)
        return found[1].strip()
    tables['augmentation_chances.toml']['blacklist']=[int(x.strip()) for x in config_value('AugmentationBlacklist').split(',')]
    tables['augmentation_chances.toml']['sources'].append(source(provenance.CONFIG,'393-415','augmentation eligibility/policies',True))
    tables['capacities.toml']['policy']['allow_augment_pvp_items']=config_value('AllowAugmentPvPItems').lower()
    custom_file='src/main/resources/config/customs.properties'
    custom=inputs.read(custom_file,True)
    mana=re.search(r'^EnableManaPotionSupport\s*=\s*(True|False)\s*$',custom,re.M)
    if mana is None:raise SourceError('missing mana potion support default')
    tables['capacities.toml']['policy']['mana_potion_support']=mana[1].lower()
    tables['capacities.toml']['sources'].append(source(custom_file,'234','EnableManaPotionSupport',True))
    tables['augmentation_chances.toml']['inactive_policy']['skill_chances']=','.join(config_value(key) for key in ['AugmentationNGSkillChance','AugmentationMidSkillChance','AugmentationHighSkillChance','AugmentationTopSkillChance'])
    tables['augmentation_chances.toml']['inactive_policy']['glow_chances']=','.join(config_value(key) for key in ['AugmentationNGGlowChance','AugmentationMidGlowChance','AugmentationHighGlowChance','AugmentationTopGlowChance'])
    items=closure(all_items,tables)
    # Item-consuming skill ingredients extend the closure to a fixed point.
    while True:
        previous_ids=set(items)
        skills,skill_files,skill_counts=skill_manifest(inputs,skill_targets(items,tables))
        for content in skill_files.values():
            s=tomllib.loads(content)
            for req in s.get('item_consume',[]):
                ids=req['item_id'] if isinstance(req['item_id'],list) else [req['item_id']]
                for sid in ids:
                    if sid and sid in all_items:items[sid]=all_items[sid]
        if set(items)==previous_ids:break
    caps=capabilities(items,tables,skills,skill_files);tables['capabilities.toml']=caps
    for kind,name in [('Weapon','weapons'),('Armor','armors'),('EtcItem','etc_items')]:
        rows=[r for sid,r in sorted(items.items()) if r['kind']==kind]
        tables[name+'.toml']={'schema':1,'sources':[source(G+'model/items/'+file+'.java',lines,'loader defaults',True) for file,lines in [('L2Item','157-218,252-255,352-353'),('L2Weapon','82-123'),('L2Armor','47-80'),('L2EtcItem','49-73')]],'items':rows}
    try:from gen_item_angles import generate_angles
    except ModuleNotFoundError:from packages.data.scripts.gen_item_angles import generate_angles
    tables['angle_table.toml']=generate_angles()
    skill_hash=digest([tomllib.loads(content) for name,content in sorted(skill_files.items(),key=lambda pair:int(Path(pair[0]).stem))])
    file_hashes=[{'file':name,'semantic_sha256':digest(value)} for name,value in sorted(tables.items())]
    profile_hash=digest({path.relative_to(ROOT).as_posix():tomllib.loads(path.read_text()) for directory in ['classes-phase3','tables-phase3'] for path in sorted((ROOT/directory).glob('*.toml'))})
    tables['rules_manifest.toml']={'schema':1,'sources':[],'domain_separator':'nightfall.phase4.rules.1\0',
        'formula_revision':1,'files':file_hashes,'skill_bundle_sha256':skill_hash,'profile_bundle_sha256':profile_hash,
        'data_sha256':digest({'files':file_hashes,'formula_revision':1,'skill_bundle_sha256':skill_hash,'profile_bundle_sha256':profile_hash})}
    files={f'tables-phase4/{name}':encode_toml(value) for name,value in tables.items()};files.update(skill_files)
    counts={'schema':1,'source_revisions':{'datapack':DATAPACK_REVISION,'game':GAME_REVISION},
            'skills':skill_counts,'items':{},'options':{},'augmentation':{},'blockers':dict(sorted(Counter(b for r in caps['items'] for b in r['blockers']).items()))}
    for kind in ['Weapon','Armor','EtcItem']:
        rows=[r for r in caps['items'] if items[r['template_id']]['kind']==kind]
        counts['items'][kind]={'source_total':sum(r['kind']==kind for r in all_items.values()),'imported':len(rows),
            'admitted':sum(r['admitted'] for r in rows),'deferred':sum(not r['admitted'] for r in rows),
            'not_imported':sum(r['kind']==kind for r in all_items.values())-len(rows)}
    for key in ['options','augmentation','recipes']:
        rows=caps[key];counts[key]={'imported':len(rows),'admitted':sum(r['admitted'] for r in rows),'deferred':sum(not r['admitted'] for r in rows)}
    elixir_skills={2287,2288,2289,2860,2861,2862}
    elixir_ids=sorted(sid for sid,row in items.items() if any(ref['id'] in elixir_skills for ref in row['skills']))
    admitted_ids={row['template_id'] for row in caps['items'] if row['admitted']}
    counts['required_condition_closure']={'condition':'player_level_range','architect':'D34',
        'runtime_support':'zone_published:4f9ac8c,c5229d4,f6d882b','skill_ids':sorted(elixir_skills),'template_ids':elixir_ids,
        'imported':len(elixir_ids),'admitted':sum(sid in admitted_ids for sid in elixir_ids),
        'deferred':sum(sid not in admitted_ids for sid in elixir_ids)}
    inventory={'schema':1,'counts':dict(Counter(r['kind'] for r in all_items.values())),
               'files':[row for key,row in sorted(inputs.files.items())],'items':ledger}
    files['tables-phase4/source_inventory.json']=json.dumps(inventory,indent=2,ensure_ascii=False)+'\n'
    files['tables-phase4/capability_counts.json']=json.dumps(counts,indent=2,ensure_ascii=False)+'\n'
    return files,counts


def write_outputs(out,files,check):
    stale=[]
    expected=set(files)
    for directory in ['tables-phase4','skills-phase4']:
        for path in sorted((out/directory).glob('*')):
            if path.suffix in {'.toml','.json'} and path.relative_to(out).as_posix() not in expected:
                stale.append('unexpected file: '+path.relative_to(out).as_posix())
    for name,content in sorted(files.items()):
        path=out/name
        if not path.exists() or path.read_text()!=content:
            if check:stale.append(name)
            else:path.parent.mkdir(parents=True,exist_ok=True);path.write_text(content)
    return stale


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--datapack',type=Path,default=DEFAULT_DP)
    parser.add_argument('--game',type=Path,default=DEFAULT_GAME)
    parser.add_argument('--out',type=Path,default=ROOT)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    try:
        files,counts=generate(args.datapack,args.game);stale=write_outputs(args.out,files,args.check)
    except (ValueError,OSError,KeyError,expat.ExpatError,subprocess.CalledProcessError,gen_skills.SourceError) as e:
        print('gen_items: '+str(e),file=sys.stderr);return 2
    if stale:print('gen_items: stale: '+', '.join(stale[:20]),file=sys.stderr);return 1
    print('gen_items: '+json.dumps({k:counts[k] for k in ['items','options','augmentation']},sort_keys=True))
    print('gen_items: '+('check passed' if args.check else 'generated'));return 0


if __name__=='__main__':sys.exit(main())
