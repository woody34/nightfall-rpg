"""§12.1 requirements, independent of vector generation and fixture contents.

The design fixes boundary domains; pinned XML/Java fixes catalogue domains.
Only source parsing/formula eligibility is shared with the oracle. No generated
rows, counts, manifests, or expectations determine this registry.
"""
from fractions import Fraction as F
from functools import lru_cache
from itertools import product
import re
import xml.etree.ElementTree as ET

D = 'src/main/resources/data/'
G = 'src/main/java/com/l2jserver/gameserver/'


@lru_cache(maxsize=None)
def required_cases(src):
    try:
        from . import item_oracle as o
    except ImportError:
        import item_oracle as o
    cases = {}

    def need(family, name, draws=None, sources=None, **dimensions):
        if name in cases:
            raise ValueError('duplicate coverage requirement: ' + name)
        cases[name] = {'family': family, 'inputs': o.rational(dimensions)}
        if draws is not None:
            cases[name]['draws'] = draws
        if sources is not None:
            cases[name]['sources'] = sources

    human = D + 'stats/chars/baseStats/HumanFighter.xml'
    # §12.1 rows 01–18: choose minimum IDs from source predicates, never evidence.
    def eligible(sid):
        p = src.properties(sid)
        return p.get('for_npc', 'false') != 'true' and not any(
            k in p for k in ['duration', 'time', 'item_skill', 'oncrit_skill', 'onmagic_skill', 'unequip_skill'])

    weapons = [sid for sid in sorted(src.items) if src.items[sid][2].get('type') == 'Weapon'
               and eligible(sid) and src.items[sid][2].find('cond') is None]
    for index, kind in enumerate(['SWORD', 'BLUNT', 'DAGGER', 'BOW', 'DUAL', 'DUALFIST', 'POLE'], 1):
        sid = min(s for s in weapons if src.properties(s).get('weapon_type') == kind) if kind != 'POLE' else min(
            s for s in src.items if src.items[s][2].get('type') == 'Weapon'
            and src.properties(s).get('weapon_type') == 'POLE' and src.properties(s).get('for_npc', 'false') != 'true'
            and src.properties(s).get('item_skill') == '3599-1' and src.items[s][2].find('cond') is None)
        # Minimum player pole carries its intrinsic skill (review-1 correction).
        refs = [[3599, 1, 2]] if kind == 'POLE' else []
        need('F1', f'gear-{index:02d}-{kind}', class_file=human, level=84,
             equipment=[[sid, 0, 'rhand', 1]], expertise=7, masteries=[], skill_refs=refs)
    need('F1', 'gear-08-FIST', class_file=human, level=1, equipment=[], masteries=[])
    for index, grade in enumerate(['D', 'C', 'B', 'A', 'S', 'S80'], 9):
        sid = min(s for s in weapons if src.properties(s).get('crystal_type') == grade
                  and src.properties(s).get('enchant_enabled') == '1'
                  and (grade not in {'D', 'B', 'S'} or src.properties(s).get('is_magic_weapon') == 'true')
                  and (grade not in {'C', 'A', 'S80'} or src.properties(s).get('bodypart') == 'lrhand'))
        need('F2', f'gear-{index:02d}-{grade}-e4', class_file=human, level=84,
             equipment=[[sid, 4, 'rhand', 1]], expertise=7, masteries=[])
    sid = min(s for s in weapons if src.properties(s).get('crystal_type') == 'S84'
              and src.properties(s).get('enchant_enabled') == '1')
    need('F2', 'gear-15-S84-e6', equipment=[[sid, 6, 'rhand', 1]], level=84)
    for index, bodies, slot, e in [(16, {'fullarmor', 'onepiece'}, 'chest', 4),
                                  (17, {'lhand'}, 'lhand', 6), (18, {'rfinger;lfinger'}, 'lfinger', 6)]:
        sid = min(s for s in src.items if src.items[s][2].get('type') == 'Armor' and eligible(s)
                  and src.properties(s).get('bodypart') in bodies
                  and src.properties(s).get('crystal_type', 'NONE') != 'NONE')
        need('F3', f'gear-{index:02d}-{slot}-e{e}', equipment=[[sid, e, slot, 1]], level=84)

    # §12.1 rows 19–21 and set loss: source-executable set, all winning refs.
    for index, kind in [(19, 'HEAVY'), (20, 'LIGHT'), (21, 'MAGIC')]:
        for file, node in sorted(src.sets, key=lambda pair: int(pair[1].get('id'))):
            if src.properties(int(node.find('chest').get('id'))).get('armor_type') != kind:
                continue
            gear = [[min(int(n.get('id')) for n in node.findall(slot)), 6 if index == 19 else 0, slot, seq]
                    for seq, slot in enumerate(['chest', 'legs', 'head', 'gloves', 'feet'], 1) if node.findall(slot)]
            refs = [[int(n.get('id')), int(n.get('level')), 100+i] for i, n in enumerate(
                node.findall('skill') + (node.findall('enchant6skill') if index == 19 else []))]
            deltas = {n.tag: int(n.get('val')) for n in node if n.tag in {'str', 'dex', 'con', 'int', 'wit', 'men'}}
            try:
                o.gear_stats(src, human, 84, gear, refs, deltas)
            except (ValueError, KeyError):
                continue
            need('F12', f'gear-{index:02d}-{kind}-set', set_id=int(node.get('id')),
                 equipment=gear, skill_refs=refs, base_deltas=deltas, level=84)
            need('F12', f'set-{kind}-missing-piece', equipment=gear[:-1], skill_refs=[], level=84)
            break
        else:
            raise ValueError('no executable required set: ' + kind)

    # Every pinned starter kit, including dormant extras; named rows 22–24.
    class_files = {int(src.xml(p.relative_to(src.dp).as_posix()).findtext('classId')): p.relative_to(src.dp).as_posix()
                   for p in (src.dp / (D+'stats/chars/baseStats')).glob('*.xml')}
    kit_names = {10: (22, 'HumanMystic'), 44: (23, 'OrcFighter'), 53: (24, 'DwarvenFighter')}
    for kit in src.xml(D+'stats/initialEquipment.xml').findall('equipment'):
        cid = int(kit.get('classId')); items = [dict(n.attrib) for n in kit.findall('item')]
        need('F12', f'starter-kit-source-{cid}', class_id=cid, items=items)
        if src.races[cid] == 'KAMAEL':
            continue  # source census only; unsupported race is not admitted
        gear = [[int(n.get('id')), 0, 'rhand' if src.properties(int(n.get('id')))['bodypart'] == 'lrhand'
                 else src.properties(int(n.get('id')))['bodypart'], seq]
                for seq, n in enumerate(kit.findall('item'), 1) if n.get('equipped') == 'true']
        name = f'gear-{kit_names[cid][0]:02d}-kit-{kit_names[cid][1]}' if cid in kit_names else f'starter-kit-{cid}'
        need('F12', name, class_file=class_files[cid], class_id=cid, equipment=gear, level=1, kit=items)
    sid = min(s for s in src.items if src.items[s][2].get('type') == 'Weapon'
              and 'enchant4_skill' in src.properties(s) and not {'time', 'duration'} & src.properties(s).keys())
    need('F12', 'weapon-enchant4-provider', template_id=sid, e3=3, e4=4)
    for sid, (_, _, node) in src.items.items():
        p = src.properties(sid)
        if node.get('type') == 'EtcItem' and (p.get('etcitem_type') in {'POTION', 'ELIXIR'} or
            p.get('etcitem_type') == 'SCROLL' and any(w in node.get('name') for w in ['Escape', 'Resurrection'])):
            need('F12', f'consumable-{sid}', template_id=sid)

    # FuncEnchant.java:43–155; design's 0/3/4/19/20 + source/storage caps.
    branches = [('rhand', 'SWORD'), ('lrhand', 'POLE'), ('lrhand', 'BOW'), ('lrhand', 'DUAL'),
                ('rhand', 'BOW'), ('rhand', 'CROSSBOW'), ('lrhand', 'CROSSBOW')]
    for grade, (body, kind), stat, e in product(src.grades, branches, ['pAtk', 'mAtk', 'pDef', 'mDef', 'sDef'],
                                             [0, 3, 4, 19, 20, 65535, 4294967295]):
        need('F2', f'enchant-{grade}-{body}-{kind}-{stat}-{e}',
             stat=stat, grade=grade, bodypart=body, weapon_type=kind, enchant=e)
    for grade, weapon, e in product(src.grades, [False, True], [0, 3, 4, 65535, 4294967295]):
        need('F5', f'crystals-{grade}-{weapon}-{e}', base=101,
             bonus=src.grades[grade]['weapon_bonus' if weapon else 'armor_bonus'], enchant=e, weapon=weapon)
    for grade, bonuses in src.hp.items():
        for e, onepiece in product([0, 1, 3, 4, len(bonuses), len(bonuses)+1, 4294967295], [False, True]):
            need('F3', f'hp-{grade}-{e}-{onepiece}', bonuses=bonuses, enchant=e, onepiece=onepiece)

    # EnchantItemGroups/Data: every rate endpoint/draw inequality and binding.
    for name, rows in src.rates.items():
        for lo, hi, chance in rows:
            cut = int(chance*1000)
            for e, residue in product(sorted({lo, hi}), sorted({0, 99999, max(0, cut-1), min(cut, 99999)})):
                need('F4', f'chance-{name}-{e}-{residue}', base=chance, enchant=e, residue=residue)
    data = src.xml(D+'enchantItemData.xml'); groups = src.xml(D+'enchantItemGroups.xml')
    for scroll in list(data):
        if scroll.tag not in {'enchant', 'support'}:
            continue
        maximum = int(scroll.get('maxEnchant', '65535'))
        for e in [max(0, maximum-1), maximum, maximum+1]:
            need('F4', f'scroll-{scroll.tag}-{scroll.get("id")}-max-{e}', **dict(scroll.attrib, current=e))
        if scroll.tag != 'enchant':
            continue
        sid = int(scroll.get('id')); gid = int(scroll.get('scrollGroupId', '0'))
        weapon = src.properties(sid)['etcitem_type'] in {'ANCIENT_CRYSTAL_ENCHANT_WP', 'BLESS_SCRL_ENCHANT_WP', 'SCRL_ENCHANT_WP'}
        group = next(g for g in groups.findall('enchantScrollGroup') if int(g.get('id')) == gid)
        for binding in group.findall('enchantRate'):
            targets = [dict(n.attrib) for n in binding.findall('item') if (n.get('slot') in {'rhand', 'lrhand'}) == weapon]
            if not targets:
                continue
            name = binding.get('group')
            for lo, hi, chance in src.rates[name]:
                stops = sorted({lo, hi+1} | ({maximum+1} if lo <= maximum < hi else set()))
                for start, end in zip(stops, stops[1:]):
                    need('F4', f'scroll-binding-{sid}-{name}-{start}-{end-1}', scroll_id=sid, group_id=gid,
                         group=name, targets=targets, current_min=start, current_max=end-1,
                         bonus=F(scroll.get('bonusRate', '0')), max_enchant=maximum)
    need('F4', 'bonus-only-zero-base', base=0, bonus=100, residue=0, enchant=20)
    for blessed, safe in [(False, False), (True, False), (False, True)]:
        need('F4', f'failure-{blessed}-{safe}', base=0, residue=99999, blessed=blessed, safe=safe)
    need('F4', 'blessed-support-refused', current=3, max_enchant=16, blessed=True, support=True)
    for chance in [0, 66670, 100000]:
        need('F4', f'modulo-lattice-{chance}', chance_1e5=chance, bound=100000)

    for con, bonus in src.bonuses['CON'].items():
        need('F7', f'load-CON-{con}', con=con, bonus=bonus, weight_limit=1)
    for threshold, delta in product([500, 666, 800, 1000], [-1, 0, 1]):
        need('F7', f'overload-{threshold}-{delta}', weight=threshold*100+1+delta, maximum=100000, bonus=1)
    for weight, previous in product([99999, 100000, 100001], [False, True]):
        need('F7', f'overload-history-{weight}-{previous}', weight=weight, maximum=100000,
             bonus=0, previous_level=4, previous_overloaded=previous)
    need('F7', 'overload-max-zero', maximum=0, previous_level=3, previous_overloaded=True)
    for expertise, grade in product(range(8), range(8)):
        need('F6', f'grade-{expertise}-{grade}-branch', expertise=expertise, equipped=[
            {'instance_id': 1, 'grade': grade, 'weapon': True},
            {'instance_id': 2, 'grade': max(0, grade-1), 'weapon': True},
            {'instance_id': 3, 'grade': 7, 'weapon': False, 'arrow': True}])

    # D24/D25 tables are fixed by design, not the generator's loop bounds.
    for level, radius in [(1, 6), (3, 6), (4, 7), (8, 7), (9, 8), (15, 8), (16, 9)]:
        for path, residue in product(['unequip', 'level_change', 'restore_weapon'], [0, 2*radius]):
            need('F10', f'fist-{level}-{path}-{residue}', level=level, path=path, residue=residue)
        need('F10', f'fist-template-restored-{level}', level=level, template_radius=10, residue=20)
    for formula, percent in [(F(499, 10), 85), (F(999, 50), 85), (20, 85),
                             (F(1001, 50), 85), (1, 85), (F(499, 10), F(1700, 23))]:
        need('F10', f'pole-{formula}-{percent}', formula=formula, percent=percent)
    for formula in [F(499, 10), F(1)]:
        need('F10', f'pole-primary-{formula}', formula=formula)
    need('F10', 'pole-miss-advances-only-eligible', maximum=1, candidates=[
        {'eligible': False, 'hit': True, 'formula': 100}, {'eligible': True, 'hit': False, 'formula': 100},
        {'eligible': True, 'hit': True, 'formula': F(499, 10)}])
    for formula, miss in product([F(499, 10), F(1), F(2), F(10199, 100)], [False, True]):
        need('F10', f'dual-{formula}-{miss}', formula=formula, miss=miss)
    for speed in [379, 400, 1500]:
        need('F10', f'dual-time-{speed}', attack_speed=speed)
    for critical in [False, True]:
        need('F8', f'skill-post-critical-perfect-{critical}', formula=1, critical=critical)
    for modifier in [1, 2]:
        need('F10', f'bow-reuse-{modifier}', reuse_delay=1500, attack_speed=400, weapon_reuse_modifier=modifier)
    for formula, dx, dy in [(100, 12499, 0), (100, 12500, 0), (100, 12501, 0),
                             (F(10199, 100), 12500, 0), (100, 7500, 10000), (100, 7501, 10000), (1, 0, 0)]:
        need('F10', f'bow-{formula}-{dx}-{dy}', formula=formula, dx_milli=dx, dy_milli=dy)
    for pos, residue in product(['front', 'side', 'back'], [499, 500, 749, 750, 999]):
        need('F10', f'blow-{pos}-{residue}', chance=50, dex=1, proximity=pos, residue=residue,
             draws=[dict(site='blow', bound=1000, residue=residue)],
             sources=['G/model/stats/Formulas.java:1621-1642',
                      'docs/plans/phase-4-zone-design.md:269 (§5; A8): integer100 to per-mille; roll1000*Q < rate_q*10'])
    for full, half in [(0, 0), (10, 0), (10, 50), (999, 999)]:
        need('F10', f'lethal-{full}-{half}', full=1, half=5, full_residue=full, half_residue=half)

    # All physical/magical/heal callers and timing/positional/shield boundaries.
    for caller, shot, proximity, block in product(['auto', 'physical', 'blow'], [False, True],
                                                ['front', 'side', 'back'], ['none', 'normal', 'perfect']):
        need('F8', f'damage-{caller}-{shot}-{proximity}-{block}', p_atk=100, defence=80, power=200,
             caller=caller, shot=shot, proximity=proximity, shield_def=40 if block == 'normal' else 0, perfect=block == 'perfect')
    for spirit, blessed in product([False, True], repeat=2):
        need('F8', f'magic-{spirit}-{blessed}', power=100, m_atk=121, m_def=80, spirit=spirit, blessed=blessed)
        for hit in [500, 501, 2000]:
            need('F8', f'cast-{spirit}-{blessed}-{hit}', hit_time=hit, casting_speed=1000, spirit=spirit, blessed=blessed)
        for (mage, npc, grade), static in product([(True, False, 'NONE'), (False, True, 'NONE'),
                                                  (False, False, 'NONE'), (False, False, 'S80'), (False, False, 'S84')], [False, True]):
            need('F8', f'heal-{mage}-{npc}-{grade}-{spirit}-{blessed}-{static}', power=100, m_atk=121, mp_consume2=10,
                 mage=mage, npc=npc, grade=grade, spirit=spirit, blessed=blessed, static=static)
    for static, magic, hit in [(True, False, 20000), (True, False, 0), (False, False, 2000)]:
        need('F8', f'cast-static-{static}-{magic}-{hit}', hit_time=hit, casting_speed=1000, spirit=True, static=static, is_magic=magic)
    for perfect, normal in product([90, 91, 99], [19, 20]):
        need('F9', f'shield-{perfect}-{normal}', rate=20, perfect_residue=perfect, normal_residue=normal)
    for guard in ['eligible', 'facing', 'sigil', 'zero', 'bow']:
        need('F9', 'shield-guard-'+guard, rate=0 if guard == 'zero' else 20, eligible=guard != 'eligible',
             facing=guard != 'facing', sigil=guard == 'sigil', bow=guard == 'bow', perfect_residue=90, normal_residue=25)
    for relative in [(1, 0), (-1, 0), (0, 1), (1, 2), (0, 0)]:
        need('F9', 'position-'+str(relative), facing=[1, 0], relative=relative)
    for deg, relative in [(0, (1, 0)), (90, (1, 1)), (120, (1, 2)), (180, (0, 1)), (270, (-1, 1)), (360, (-1, 0))]:
        need('F9', f'angle-{deg}-{relative}', degrees=deg, facing=[1, 0], relative=relative)

    # Every retail selector, regardless of runtime admission; complete source rows.
    for kind, basename in [('weapon', 'retailchances.xml'), ('accessory', 'retailchances_accessory.xml')]:
        for weapon in src.xml(D+'stats/augmentation/'+basename).findall('weapon'):
            for stone in weapon.findall('stone'):
                for variation in stone.findall('variation'):
                    for category in variation.findall('category'):
                        tier, level, cat = int(stone.get('id')), int(variation.get('id')), int(category.get('probability'))
                        need('F11', f'retail-{kind}-{weapon.get("type")}-{tier}-{level}-{cat}', kind=kind, stone_id=tier,
                             weapon_type=weapon.get('type'), variation=level, category=cat,
                             rows=[(int(n.get('id')), F(n.get('chance'))) for n in category.findall('augment')])
    for accessory in [False, True]:
        need('F11', f'categories-{accessory}', accessory=accessory)
    need('F11', 'packed-unsigned-high-half', option12=65535, option34=65535)
    # Extract cancellation cut points independently from the pinned Java switch.
    java = src.read(G+'network/clientpackets/RequestRefineCancel.java', True)
    for grade in ['C', 'B', 'A', 'S', 'S80', 'S84']:
        if grade in {'C', 'B', 'A'}:
            section = java.split('case '+grade+':', 1)[1].split('break;', 1)[0]
            limits = [int(v) for v in re.findall(r'getCrystalCount\(\) < (\d+)', section)]
        else:
            limits = [1]
        for limit, delta in product(limits, [-1, 0, 1]):
            need('F11', f'cancel-{grade}-{limit+delta}', grade=grade, crystals=limit+delta)
    crystals = src.xml(D+'levelUpCrystalData.xml').findall('crystal/item')
    crystal_ids = {int(n.get(k)) for n in crystals for k in ['itemId', 'leveledItemId']}
    for node in crystals:
        need('F12', 'soul-'+node.get('itemId'), **node.attrib)
    for path in sorted((src.dp/(D+'multisell')).glob('*.xml')):
        root = ET.parse(path).getroot()
        for ordinal, node in enumerate(root.findall('item')):
            ingredients = [(int(n.get('id')), int(n.get('count'))) for n in node.findall('ingredient')]
            products = [(int(n.get('id')), int(n.get('count'))) for n in node.findall('production')]
            is_weapon = lambda sid: sid in src.items and src.items[sid][2].get('type') == 'Weapon'
            if sum(is_weapon(s) for s, _ in ingredients) == 1 and any(s in crystal_ids for s, _ in ingredients) and products and all(is_weapon(s) for s, _ in products):
                need('F12', f'SA-{path.stem}-{ordinal}', ingredients=ingredients, products=products,
                     maintain_enchantment=root.get('maintainEnchantment', 'false') == 'true', enchant=7, augment=2147483649, tax_rate=0)

    for first, second in [('chest', 'shield'), ('shield', 'chest')]:
        need('F12', f'set-history-{first}-{second}', mutations=[first, second], complete=False)
    for removal in ['chest', 'shield', 'required']:
        need('F12', 'set-history-remove-'+removal, active=True, remove=removal)
    for maximum in [150, 75]:
        need('F1', f'clamp-100-{maximum}', current=100, new_maximum=maximum)
    for (landed, unequip), (spirit, soul) in product([(False, False), (True, False), (False, True)], [(True, True), (False, True)]):
        need('F8', f'charge-{landed}-{unequip}-{spirit}-{soul}', charges=['soul', 'spirit', 'blessed'],
             use_spirit=spirit, use_soul=soul, landed=landed, unequip=unequip)
    _revision_requirements(src, need, o, human)
    _behavior_requirements(src, need, o)
    return cases


def _revision_requirements(src, need, o, human):
    # D35: independently enumerate supported base classes from classList, then slots.
    base = {int(n.get('classId')) for n in src.xml(D+'stats/chars/classList.xml').findall('class')
            if n.get('parentClassId') is None and src.races.get(int(n.get('classId'))) in {'HUMAN', 'ELF', 'DARK_ELF', 'ORC', 'DWARF'}}
    physical = ['chest', 'legs', 'head', 'feet', 'gloves', 'under', 'cloak']
    magical = ['rear', 'lear', 'rfinger', 'lfinger', 'neck']
    for path in sorted((src.dp/(D+'stats/chars/baseStats')).glob('*.xml')):
        file = path.relative_to(src.dp).as_posix(); cid = int(src.xml(file).findtext('classId'))
        if cid not in base:
            continue
        for name, slots, body in ([('empty', [], 'none')] + [(s, [s], 'none') for s in physical+magical] +
                                  [('all', physical+magical, 'none'), ('fullarmor', ['chest'], 'fullarmor'),
                                   ('onepiece', ['chest'], 'onepiece'), ('fullarmor-legs', ['chest', 'legs'], 'fullarmor')]):
            need('F1', f'class-slots-{cid}-{name}', class_file=file, level=84, occupied_slots=slots, chest_bodypart=body)
    # D75: every supported-lineage class's unarmed reach plus one weapon override.
    graph = {int(n.get('classId')): n.get('parentClassId') for n in src.xml(D+'stats/chars/classList.xml').findall('class')}
    for path in sorted((src.dp/(D+'stats/chars/baseStats')).glob('*.xml')):
        file = path.relative_to(src.dp).as_posix(); cid = top = int(src.xml(file).findtext('classId'))
        while graph.get(top) is not None:
            top = int(graph[top])
        if top in base:
            need('F1', f'class-combat-{cid}-unarmed', class_file=file, base_class_id=top)
    need('F1', 'class-combat-44-armed-2369', class_file=D+'stats/chars/baseStats/OrcFighter.xml', base_class_id=44, weapon=2369,
         sources=['G/model/actor/stat/CharStat.java:424-435', 'G/model/items/L2Weapon.java:92'])
    for slot in ['rfinger', 'lfinger']:
        root = src.xml(human); root.find('staticData/baseMDef/rfinger').text = '7'; root.find('staticData/baseMDef/lfinger').text = '999'
        need('F1', 'class-slots-unequal-'+slot, class_file=human, level=84, occupied_slots=[slot], chest_bodypart='none',
             synthetic={'kind': 'synthetic', 'description': 'class XML RFINGER=7, LFINGER=999; other nodes unchanged',
                        'class_override_xml': ET.tostring(root, encoding='unicode')})
    need('F1', 'RFINGER-duplication-unequal-injected-bases', class_file=human, level=84, injected_lfinger=999, injected_rfinger=7)
    for sid in [4270, 6209, 6213, 3599]:
        for rank, lifecycle in product(range(1, int(src.skills[sid][1].get('levels'))+1), ['living', 'death', 'dead_admission', 'resurrection']):
            need('F1', f'system-{sid}-{rank}-{lifecycle}', skill_refs=[[sid, rank, 100]], lifecycle=lifecycle)
    need('F1', 'attribute-provider', class_file=human, level=1, equipment=[[1, 0, 'rhand', 1]], skill_refs=[[8498, 1, 2]])
    for file, node in src.sets:
        deltas = {n.tag: int(n.get('val')) for n in node if n.tag in {'str', 'dex', 'con', 'int', 'wit', 'men'}}
        if not deltas.get('str'):
            continue
        gear = [[1, 0, 'rhand', 1]] + [[int(node.find(s).get('id')), 0, s, i+2]
            for i, s in enumerate(['chest', 'legs', 'head', 'gloves', 'feet']) if node.find(s) is not None]
        refs = [[int(n.get('id')), int(n.get('level')), 100+i] for i, n in enumerate(node.findall('skill'))] + [[8498, 1, 200]]
        try:
            o.gear_stats(src, human, 1, gear, refs, deltas)
        except (ValueError, KeyError):
            continue
        need('F1', 'attribute-set-interaction', class_file=human, level=1, equipment=gear, skill_refs=refs,
             base_deltas=deltas, set_id=int(node.get('id')))
        break
    else:
        raise ValueError('no required attribute/set interaction')
    for sequence in [1, 3]:
        node = ET.fromstring(ET.tostring(src.skills[8498][1])); stat = node.find('effects/effect/add')
        stat.tag = 'set'; stat.set('order', '1'); stat.set('val', '40')
        need('F1', f'attribute-same-priority-{sequence}', class_file=human, level=1, equipment=[[1, 0, 'rhand', 1]],
             skill_refs=[[8498, 1, 2]], base_deltas={'str': 2}, base_delta_sequence=sequence,
             synthetic={'kind': 'synthetic', 'description': 'skill8498 STR ADD replaced with priority-1 SET40; set STR+2 at priority1',
                        'skill_override_xml': ET.tostring(node, encoding='unicode')})
    for step, gear in enumerate([[[69, 4, 'rhand', 1]], [], [[69, 4, 'rhand', 3]]]):
        need('F1', f'provider-install-remove-reinstall-{step}', class_file=human, level=1, equipment=gear)
    provider_states = [[('item:15', 1)], [('item:15', 1), ('learned:3599', 2)], [('learned:3599', 2)],
                       [('learned:3599', 2), ('item:15', 3)], [('item:15', 3)]]
    for step, (providers, event) in enumerate(zip(provider_states, [('install', 'item:15'), ('install', 'learned:3599'),
                                                                  ('remove', 'item:15'), ('install', 'item:15'), ('remove', 'learned:3599')])):
        need('F1', f'provider-overlap-{step}', equipment=[[15, 0, 'rhand', 10]], event=[*event, 3599, 1],
             providers=[{'key': key, 'skill_id': 3599, 'rank': 1, 'sequence': seq} for key, seq in providers])
    branches = {
        'pve-critical': {'critical': True}, 'pvp-critical': {'critical': True, 'target_player': True},
        'playable-nonplayer-critical': {'critical': True, 'is_pvp': True, 'attacker_player': False, 'target_player': True},
        'half-success': {'critical': True, 'magic_failures': True, 'first_success': False, 'second_success': True, 'level_difference': 9},
        'resisted-level': {'magic_failures': True, 'first_success': False, 'second_success': True, 'level_difference': 10},
        'resisted-second': {'magic_failures': True, 'first_success': False, 'second_success': False},
        'nonplayer-failure': {'attacker_player': False, 'magic_failures': True, 'first_success': False, 'critical': True},
        'disabled-failures': {'magic_failures': False, 'first_success': False, 'critical': True},
        'perfect-skips': {'perfect': True, 'm_def': 0, 'magic_failures': True, 'first_success': False},
        'modifiers-magic': {'target_player': True, 'pvp_magic_def': 2, 'pvp_magic_damage': F(3, 2), 'spread': F(9, 10),
                            'attribute': F(6, 5), 'npc': F(4, 5), 'magic_crit_damage': F(5, 4), 'critical': True},
        'modifiers-physical': {'target_player': True, 'skill_is_magic': False, 'pvp_skill_def': 3, 'pvp_skill_damage': 2, 'attribute': F(7, 5)}}
    for (name, branch), (spirit, blessed) in product(branches.items(), product([False, True], repeat=2)):
        need('F8', f'magic-branch-{name}-{spirit}-{blessed}', **dict(
            dict(power=100, m_atk=100, m_def=100, spirit=spirit, blessed=blessed), **branch))
    # Exact Fraction/root interval restoration fixes these independently known states.
    for step, hp, remainder in [(0, 0, 0), (1, 115, 556349), (2, 231, 112698)]:
        need('F8', f'heal-fraction-repeat-{step}', power=100, m_atk=121, current_hp=hp, remainder_q=remainder, recoverable_hp=1000)
    for name, branch in {
        'partial': dict(current_hp=950, recoverable_hp=1000, remainder_q=500000),
        'full': dict(current_hp=1000, recoverable_hp=1000), 'over': dict(current_hp=1001, recoverable_hp=1000),
        'over-fraction-positive': dict(current_hp=1001, recoverable_hp=1000, remainder_q=500000),
        'over-fraction-zero': dict(static=True, power=0, current_hp=1001, recoverable_hp=1000, remainder_q=500000),
        'at-cap-fraction': dict(current_hp=1000, recoverable_hp=1000, remainder_q=500000),
        'static': dict(static=True, power='5/4', heal_effect=9, critical=True, current_hp=9, recoverable_hp=10),
        'negative-static': dict(static=True, power=-1)}.items():
        need('F8', 'heal-clamp-'+name, **dict(dict(power=100, m_atk=121), **branch))
    for kind, toggle in product(['soul', 'spirit', 'blessed'], [False, True]):
        for name, branch in {'paid': {}, 'splus': {'weapon_grade': 'S84', 'shot_grade': 'S'},
                             'wrong-grade': {'shot_grade': 'C'}, 'empty': {'available': 0},
                             'already': {'charges': [kind]}, 'no-weapon': {'weapon_grade': None}}.items():
            need('F8', f'shot-payment-{kind}-{toggle}-{name}', **dict(
                dict(charges=[], kind=kind, toggle=toggle, weapon_grade='D', shot_grade='D', available=10, count=2), **branch))
    sid = min(s for s in src.items if 'reduced_soulshot' in src.properties(s))
    p = src.properties(sid); reduction = list(map(int, p['reduced_soulshot'].split(',')))
    for residue, available in product([0, reduction[0]-1, reduction[0], 99], [0, reduction[1], int(p['soulshots'])]):
        need('F8', f'shot-reduction-{sid}-{residue}-{available}', kind='soul', weapon_grade=p['crystal_type'],
             shot_grade=p['crystal_type'], available=available, count=int(p['soulshots']), reduction=reduction, residue=residue, charges=[], toggle=True)
    callers = [('dual', [False, False]), ('dual', [True, False]), ('dual', [False, True]), ('dual', [True, True]),
               ('pole', [False, True, True]), ('multi-target', [True, True, True])]
    for (caller, hits), (canceled, unequip) in product(callers, [(False, False), (True, False), (False, True)]):
        need('F8', f'shot-bundle-{caller}-{hits}-{canceled}-{unequip}', charges=['soul', 'spirit', 'blessed'],
             hits=hits, use_spirit=caller == 'multi-target', use_soul=caller != 'multi-target', canceled=canceled, unequip=unequip)
    for sid in [736, 1538, 737, 1539, 1540, 5591, 5592, 8628, 14688]:
        definition = o.consumable_definition(src, sid)
        base = dict(definition=definition, tick=10, available={sid: 10},
                    condition_context={'level': 1, 'canEscape': True, 'canResurrect': True, 'inside_zone_ids': []})
        need('F12', f'consumable-consume-once-{sid}', **base)
        need('F12', f'consumable-lane-{sid}', **dict(base, main_casting=True))
        # Source reuse domains, not accepted rows from generation.
        if definition['reuse_delay_ms'] > 0:
            key = definition['reuse_group'] or sid
            skill_deadlines = {s['id']: 10+(s['reuse_delay_ms']+99)//100 for s in definition['skills'] if s['reuse_delay_ms'] > 0}
            need('F12', f'consumable-shared-reuse-{sid}', **dict(base, tick=11, stack_identity=2,
                 item_deadlines={key: 10+(definition['reuse_delay_ms']+99)//100}, skill_deadlines=skill_deadlines))
    need('F12', 'consumable-same-tick-lanes', caster_ordinal=0, caster_id=100, starts=[
        {'tick': 10, 'cast_id': 1, 'definition': o.consumable_definition(src, 736), 'available': {736: 1, 1539: 2},
         'condition_context': {'canEscape': True, 'inside_zone_ids': []}},
        {'tick': 209, 'cast_id': 2, 'definition': o.consumable_definition(src, 1539)}])
    for sid in [2287, 2288, 2289, 2860, 2861, 2862]:
        node = src.skills[sid][1]; table = {n.get('name'): n.text.split() for n in node.findall('table')}
        for rank, value in enumerate(table[node.find('cond/player').get('levelRange')], 1):
            lo, hi = map(int, value.split(';'))
            for level in [lo-1, lo, hi, hi+1]:
                need('F12', f'elixir-range-{sid}-{rank}-{level}', skill_id=sid, rank=rank, minimum=lo, maximum=hi, level=level)


def _behavior_requirements(src, need, o):
    # §12.1 source/state domains. Never consult emitted vectors or case builders.
    b = o.behavior
    bows = [sid for sid in sorted(src.items) if src.properties(sid).get('weapon_type') == 'BOW'
            and src.properties(sid).get('for_npc', 'false') != 'true' and src.items[sid][2].find('cond') is None]
    arrows = [sid for sid in sorted(src.items) if src.properties(sid).get('etcitem_type') == 'ARROW']
    for grade in src.grades:
        sid = min(s for s in bows if src.properties(s).get('crystal_type', 'NONE') == grade)
        weapon = b.bow_definition(src, sid)
        match = 'S' if grade in {'S80', 'S84'} else grade
        aid = min(s for s in arrows if src.properties(s).get('crystal_type', 'NONE') == match)
        wrong = min(s for s in arrows if src.properties(s).get('crystal_type', 'NONE') != match)
        for state, miss in product(['equipped', 'refill', 'empty', 'wrong', 'wrong-equipped', 'exhausted'], [False, True]):
            inventory = [] if state == 'empty' else [b.arrow_stack(src, wrong if state.startswith('wrong') else aid,
                                                                1 if state == 'exhausted' else 2)]
            need('F10', f'bow-launch-{grade}-{state}-{miss}', weapon=weapon, inventory=inventory,
                 lhand=1 if state in {'equipped', 'wrong-equipped'} else None, mp=F(10), mp_modifier=F(1),
                 reduction_residue=None, miss=miss,
                 # D87: live residues are inputs (non-neutral on the equipped case); D88 strict hit.
                 critical_residue=None if miss else (0 if state == 'equipped' else 999),
                 spread_residue=None if miss or weapon['spread_radius'] == 0 else (
                     0 if state == 'equipped' else weapon['spread_radius']),
                 reuse_remaining=0, hit_chance=800, hit_residue=800 if miss else 799)
        need('F10', 'bow-refill-history-'+grade, weapon=weapon, arrow=b.arrow_stack(src, aid, 1), mp=F(20))
    sid = min(s for s in bows if int(src.properties(s).get('reduced_mp_consume', '0,0').split(',')[1]) > 0)
    weapon = b.bow_definition(src, sid)
    aid = min(s for s in arrows if src.properties(s).get('crystal_type', 'NONE') == weapon['grade'])
    for residue, cost, delta, modifier in product([0, weapon['reduced_chance']-1, weapon['reduced_chance'], 99],
            sorted({weapon['mp_cost'], weapon['reduced_cost']}), [F(-1, 1000000), F(0), F(1, 1000000)], [F(1), F(3, 2), F(0)]):
        need('F10', f'bow-mp-{sid}-{residue}-{cost}-{delta}-{modifier}', weapon=weapon,
             inventory=[b.arrow_stack(src, aid, 2)], lhand=None, mp=max(F(0), int(F(cost)*modifier)+delta),
             mp_modifier=modifier, reduction_residue=residue, miss=True, critical_residue=None, spread_residue=None,
             reuse_remaining=0, hit_chance=800, hit_residue=800)
    for state in ['empty', 'reuse']:
        need('F10', 'bow-mp-skip-'+state, weapon=weapon,
             inventory=[] if state == 'empty' else [b.arrow_stack(src, aid, 2)], lhand=None,
             mp=F(10), mp_modifier=F(1), reduction_residue=0, miss=False, critical_residue=999,
             spread_residue=weapon['spread_radius'] or None, reuse_remaining=int(state == 'reuse'), hit_chance=800,
             hit_residue=799)
    definition = o.consumable_definition(src, 737)
    effect = src.skills[2014][1].find('effects/effect')
    power = F(next((n.get('power') for n in effect.findall('param') if n.get('power') is not None), '0'))
    hp_restore = F(re.search(r'^RespawnRestoreHP\s*=\s*(\S+)',
                           src.read('src/main/resources/config/character.properties', True), re.M)[1])
    base = dict(definition=definition, start_tick=10, available=2, death_incarnation=7,
                current_incarnation=7, dead=True, response='offer', response_tick=160, response_offer_id=1,
                existing_offer=None, repeat=0, hp=F(0), max_hp=101, mp=F(13, 2), cp=F(17, 2),
                recoverable_xp=101, restore_percent=power, restore_hp=hp_restore)
    for name, changes in [('offer', {}), ('decline', {'response': 'decline'}),
            ('accept', {'response': 'accept', 'repeat': 1}), ('stale-id', {'response': 'accept', 'response_offer_id': 2}),
            ('stale-death', {'current_incarnation': 8, 'response': 'accept'}),
            ('alive', {'dead': False, 'response': 'accept'}), ('empty', {'available': 0})]:
        need('F12', 'resurrection-scroll-'+name, **dict(base, **changes))
    for response, delta in product(['accept', 'decline', 'expire'], [-1, 0, 1]):
        need('F12', f'resurrection-scroll-{response}-expiry-{delta}', **dict(base, response=response, response_tick=760+delta))
    need('F12', 'resurrection-scroll-first-valid', **dict(base, existing_offer=dict(offer_id=9, target_incarnation=7,
         created_at=1, expires_at=601, restore_percent=F(50), recoverable_xp=101), response='accept', response_offer_id=9, repeat=1))
    for sid, speed in product([736, 1538], [333, 1500]):
        need('F12', f'consumable-escape-timing-{sid}-{speed}', definition=o.consumable_definition(src, sid),
             start_tick=10, casting_speed=speed, available=2, resources={'hp': 13, 'mp': 17, 'cp': 19, 'xp': 101}, incarnation=7)
    for kit in src.xml(D+'stats/initialEquipment.xml').findall('equipment'):
        cid = int(kit.get('classId'))
        if src.races[cid] == 'KAMAEL':
            continue
        for item in kit.findall('item'):
            sid = int(item.get('id')); p = src.properties(sid)
            if p.get('handler') == 'Book' or p.get('item_skill') in {'2531-1', '2525-1'}:
                need('F12', f'dormant-starter-use-{cid}-{sid}', template_id=sid, properties=p,
                     inventory=[dict(instance_id=1, template_id=sid, count=int(item.get('count'))),
                                dict(instance_id=2, template_id=57, count=10)], selected_instance=1,
                     paperdoll={'rhand': 3}, allocator_next=4, reuse={'737': 800})
    safe = 1+max(hi for lo, hi, rate in src.rates['ARMOR_GROUP'] if rate == 100)
    full = 1+max(hi for lo, hi, rate in src.rates['FULL_ARMOR_GROUP'] if rate == 100)
    for body, e in product([False, True], [2, 3, 4, 5]):
        need('F4', f'enchant-safe-{body}-{e}', current=e, fullarmor=body, safe_enchant=safe,
             safe_fullarmor=full, base=F(0), residue=99999)
    for base_rate, bonus, support in [(F(0), F(0), F(0)), (F(6667, 100), F(0), F(0)),
                                      (F(100), F(0), F(0)), (F(0), F(10), F(5))]:
        need('F4', f'enchant-quantized-{base_rate}-{bonus}-{support}', base=base_rate, bonus=bonus, support=support)
    for (con, bonus), limit in product(src.bonuses['CON'].items(), [F(1, 2), F(3, 2)]):
        need('F7', f'load-modifier-{con}-{limit}', con=con, bonus=bonus, weight_limit=limit, multiplier=F(3, 2), add=F(1, 3))
    for slot, expertise in product(range(25), range(8)):
        need('F6', f'grade-slot-{slot}-{expertise}', equipped=[dict(instance_id=1, slot=slot, grade=7, weapon=slot == 5)], expertise=expertise)
    for speed in [379, 400, 1500]:
        need('F10', f'bow-reuse-speed-{speed}', reuse_delay=1500, attack_speed=speed, weapon_reuse_modifier=F(3, 2))
    for hits, valid in product([[False, False], [True, False], [False, True], [True, True]], [False, True]):
        need('F10', f'dual-plan-{hits}-{valid}', hits=hits, formulas=[F(499, 10), F(10199, 100)], second_valid=valid,
             hit_residues=[799 if h else 800 for h in hits], hit_chance=800, spread_radius=6,
             critical_residues=[(0 if valid else 999) if h else None for h in hits],
             spread_residues=[(12 if valid else 6) if h else None for h in hits])
    for count in [0, 1, 2]:
        need('F10', f'pole-count-{count}', candidates=[dict(eligible=True, hit=True, formula=F(499, 10)) for _ in range(4)], maximum=count)
    for chance, residue in product([F(0), F(999, 10), F(100), F(101)], [998, 999]):
        need('F10', f'blow-boundary-{chance}-{residue}', chance=chance, dex=F(1), proximity='front', residue=residue,
             draws=[dict(site='blow', bound=1000, residue=residue)],
             sources=['G/model/stats/Formulas.java:1621-1642',
                      'docs/plans/phase-4-zone-design.md:269 (§5; A8): integer100 to per-mille; roll1000*Q < rate_q*10'])
    for power, critical in product([F(-1), F(0), F(79, 77), F(80, 77), F(81, 77), F(199, 77)], [False, True]):
        need('F10', f'blow-cap-rounding-{power}-{critical}', p_atk=0, defence=80, power=power, caller='blow', critical=critical)
    for accessory in [False, True]:
        need('F11', f'retail-draw-contract-{accessory}', accessory=accessory)
    # Minimum source option for each function/active/passive/trigger form.
    options = {}
    for path in sorted((src.dp/(D+'stats/options')).glob('*.xml')):
        for node in src.xml(path.relative_to(src.dp).as_posix()).findall('option'):
            for kind in ['for', 'active_skill', 'passive_skill', 'attack_skill', 'critical_skill', 'magic_skill']:
                if node.find(kind) is not None and kind not in options:
                    options[kind] = node
    for kind in ['for', 'active_skill', 'passive_skill', 'attack_skill', 'critical_skill', 'magic_skill']:
        node = options[kind]
        for event in ['install', 'unequip', 'cancel', 'death', 'dead_admission', 'resurrection']:
            need('F11', f'augmentation-provider-{kind}-{event}', option_id=int(node.get('id')),
                 option_xml=ET.tostring(node, encoding='unicode'), event=event, other_provider=99, hp=F(100), maximum=75,
                 stat_baseline={n.get('stat'): F(100) for n in node.findall('for/*')})
    sets = {n.get('name'): n.get('val') for n in src.skills[1401][1].findall('set')}
    for phase, available in product(['complete', 'cancel'], [0, 1, 2]):
        need('F12', f'learned-item-consume-{phase}-{available}', skill_id=1401, rank=1,
             consume_id=int(sets['itemConsumeId']), consume_count=int(sets['itemConsumeCount']),
             inventory=[] if available == 0 else [dict(instance_id=2, template_id=3031, count=available), dict(instance_id=1, template_id=3031, count=1)],
             phase=phase, repeat=True)
    root = src.xml(D+'stats/chars/baseStats/HumanFighter.xml')
    regen = F(root.find('lvlUpgainData').find('level').findtext('hpRegen'))
    bonus = src.bonuses['CON'][int(root.findtext('staticData/baseCON'))]
    for (posture, modifier), (dead, stunned) in product([('standing', F(11, 10)), ('running', F(7, 10)), ('sitting', F(3, 2))],
                                                      [(False, False), (False, True), (True, False)]):
        for current, maximum in [(0, 1000), (99, 100), (999, 1000)]:
            need('F12', f'item-regen-status-{posture}-{dead}-{stunned}-{current}', base=regen, level_modifier=F(9, 10),
                 stat_bonus=bonus, posture_modifier=modifier, regen_modifier=F(1, 2),
                 periodic_credit=F(o.consumable_definition(src, 1540)['skills'][0]['effects'][0]['params']['amount']),
                 current=current, maximum=maximum, remainder_q=500000, dead=dead, stunned=stunned)
    for guard in ['invulnerable', 'hp_blocked']:
        need('F12', 'item-regen-status-'+guard, base=regen, level_modifier=F(9, 10), stat_bonus=bonus,
             posture_modifier=F(11, 10), regen_modifier=F(1, 2),
             periodic_credit=F(o.consumable_definition(src, 1540)['skills'][0]['effects'][0]['params']['amount']),
             current=0, maximum=1000, remainder_q=500000, dead=False, stunned=False,
             invulnerable=guard == 'invulnerable', hp_blocked=guard == 'hp_blocked')


def verify_case(name, case, row):
    """Identity alone cannot prove a source selector or a boundary input."""
    import json
    if row.get('family') != case['family']:
        raise ValueError('design 12.1 incorrect family: ' + name)
    inputs = row.get('inputs', {})
    for key, value in case['inputs'].items():
        # bool/int compare equal in Python; coverage dimensions retain JSON types.
        if key not in inputs or json.dumps(inputs[key], sort_keys=True) != json.dumps(value, sort_keys=True):
            raise ValueError('design 12.1 incorrect input dimension: ' + name + '.' + key)
    if 'draws' in case and json.dumps(row.get('draws'), sort_keys=True) != json.dumps(case['draws'], sort_keys=True):
        raise ValueError('design 12.1 incorrect draw trace: ' + name)
    if any(source not in row.get('sources', []) for source in case.get('sources', [])):
        raise ValueError('design 12.1 missing source citation: ' + name)
