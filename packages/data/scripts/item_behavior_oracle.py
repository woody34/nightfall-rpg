"""Source-derived §12.1 state transitions; no engine or recorded expectations."""
from fractions import Fraction as F
import xml.etree.ElementTree as ET

D = 'src/main/resources/data/'
G = 'src/main/java/com/l2jserver/gameserver/'
Q = 1000000


def bow_definition(src, sid):
    p = src.properties(sid)
    chance, reduced = map(int, p.get('reduced_mp_consume', '0,0').split(','))
    return dict(template_id=sid, grade=p.get('crystal_type', 'NONE'),
                mp_cost=int(p.get('mp_consume', '0')), reduced_chance=chance, reduced_cost=reduced,
                spread_radius=int(p.get('random_damage', '0')))


def arrow_stack(src, sid, count, instance_id=1):
    p = src.properties(sid)
    return dict(instance_id=instance_id, template_id=sid, grade=p.get('crystal_type', 'NONE'),
                kind=p.get('etcitem_type', 'NONE'), count=count)


def bow_launch(weapon, inventory, lhand, mp, mp_modifier, reduction_residue, miss, reuse_remaining=0,
               hit_chance=800, hit_residue=None):
    # checkAndEquipArrows trusts occupied LHAND; grade matching is a search rule.
    inventory = [dict(item) for item in inventory]
    splus = lambda grade: 'S' if grade in {'S80', 'S84'} else grade
    arrow = next((i for i in inventory if i['instance_id'] == lhand), None) if lhand is not None else next(
        (i for i in inventory if i['count'] > 0 and i['kind'] == 'ARROW'
         and splus(i['grade']) == splus(weapon['grade'])), None)
    before = None if arrow is None else arrow['instance_id']
    result = dict(accepted=False, reason='arrows', inventory=inventory, lhand=before,
                  equipped_lhand=before, mp=F(mp), mp_q=int(F(mp)*Q), mp_paid=0,
                  arrows_consumed=0, draws=[], hit=None)
    if arrow is None:
        return result
    if reuse_remaining > 0:
        result['reason'] = 'reuse'
        return result
    cost = weapon['mp_cost']
    if weapon['reduced_cost'] > 0:
        if type(reduction_residue) is not int or not 0 <= reduction_residue < 100:
            raise ValueError('invalid bow MP residue')
        result['draws'].append(dict(site='bow_mp_reduction', bound=100, residue=reduction_residue))
        if reduction_residue < weapon['reduced_chance']:
            cost = weapon['reduced_cost']
    cost = int(F(cost)*F(mp_modifier))
    result['mp_cost'] = cost
    if F(mp) < cost:
        result['reason'] = 'mp'
        return result
    if hit_residue is None:
        hit_residue = hit_chance+1 if miss else hit_chance
    if not 0 <= hit_residue < 1000 or miss != (hit_residue > hit_chance):
        raise ValueError('inconsistent bow hit residue')
    result['draws'].append(dict(site='hit', bound=1000, residue=hit_residue))
    if not miss:
        # These launch-cost vectors isolate a shieldless, noncritical, neutral-spread caller.
        result['draws'].append(dict(site='critical', bound=1000, residue=999))
        radius = weapon['spread_radius']
        if radius > 0:
            result['draws'].append(dict(site='spread', bound=2*radius+1, residue=radius))
    result.update(accepted=True, reason='launched', mp=F(mp)-cost, mp_q=int((F(mp)-cost)*Q),
                  mp_paid=cost, arrows_consumed=1, hit=not miss)
    arrow['count'] -= 1
    if arrow['count'] == 0:
        inventory.remove(arrow)
        result['lhand'] = None
    return result


def bow_refill(weapon, arrow, mp):
    trace = []
    inventory = [dict(arrow)]
    lhand = None
    for step in range(4):
        if step == 2:
            inventory = [dict(arrow)]  # externally granted replacement stack
        result = bow_launch(weapon, inventory, lhand, mp, F(1), 0, step != 0)
        trace.append(result)
        inventory, lhand, mp = result['inventory'], result['lhand'], result['mp']
    return dict(trace=trace, consumed=sum(r['arrows_consumed'] for r in trace),
                mp_paid=sum(r['mp_paid'] for r in trace), draws=[d for r in trace for d in r['draws']])


def resurrection_scroll(definition, start_tick, available, death_incarnation, current_incarnation,
                        dead, response, response_tick, response_offer_id, existing_offer,
                        repeat, hp, max_hp, mp, cp, recoverable_xp, restore_percent, restore_hp):
    # I5 prepays at accepted start. Offers are created only at cast completion.
    skill = definition['skills'][0]
    cost = skill['item_consume_count']
    if available < cost:
        return dict(accepted=False, reason='items', inventory=available, consumed=0, draws=[])
    completion = start_tick + max(1, (skill['hit_time_ms']+99)//100)
    inventory = available-cost
    offer = None if existing_offer is None else dict(existing_offer)
    trace = [dict(event='start', tick=start_tick, consumed=cost, inventory=inventory),
             dict(event='complete', tick=completion, consumed=0, inventory=inventory)]
    if offer is not None and completion >= offer['expires_at']:
        offer = None
    if dead and current_incarnation == death_incarnation and offer is None:
        offer = dict(offer_id=1, target_incarnation=death_incarnation, created_at=completion,
                     expires_at=completion+600, restore_percent=F(restore_percent), recoverable_xp=recoverable_xp)
        trace.append(dict(event='offered', tick=completion, offer=dict(offer), inventory=inventory))
    elif offer is not None:
        trace.append(dict(event='retained', tick=completion, offer=dict(offer), inventory=inventory))
    else:
        trace.append(dict(event='invalid_target', tick=completion, inventory=inventory))
    refund = 0
    for attempt in range(1+repeat):
        if response == 'offer':
            break
        if offer is not None and response_tick >= offer['expires_at']:
            trace.append(dict(event='expired', tick=response_tick, inventory=inventory))
            offer = None
        if response == 'expire':
            break
        if (offer is None or not dead or current_incarnation != offer['target_incarnation']
                or response_offer_id != offer['offer_id']):
            trace.append(dict(event='NO_RESURRECT_OFFER', tick=response_tick, attempt=attempt, inventory=inventory))
            continue
        if response == 'accept':
            refund = int(F(offer['recoverable_xp'])*F(offer['restore_percent'])/100)
            recoverable_xp = 0
            hp = max(F(hp), F(max_hp)*F(restore_hp))
            dead = False
            current_incarnation += 1
            trace.append(dict(event='accepted', tick=response_tick, refund=refund, inventory=inventory))
        elif response == 'decline':
            trace.append(dict(event='declined', tick=response_tick, inventory=inventory))
        else:
            raise ValueError('unsupported resurrection response')
        offer = None
    return dict(accepted=True, consumed=cost, completion_consumed=0, inventory=inventory,
                offer=offer, trace=trace, dead=dead, incarnation=current_incarnation, hp=F(hp),
                hp_q=int(F(hp)*Q), mp=F(mp), cp=F(cp), xp_refund=refund,
                recoverable_xp=recoverable_xp, draws=[])


def dormant_use(template_id, properties, inventory, selected_instance, paperdoll, allocator_next, reuse):
    if properties.get('handler') != 'Book' and properties.get('item_skill') not in {'2531-1', '2525-1'}:
        raise ValueError('not a dormant starter capability')
    if not any(i['instance_id'] == selected_instance and i['template_id'] == template_id for i in inventory):
        raise ValueError('missing selected dormant item')
    return dict(accepted=False, reason='ITEM_CAPABILITY_DEFERRED', consumed={}, inventory=inventory,
                paperdoll=paperdoll, allocator_next=allocator_next, reuse=reuse, draws=[])


def learned_item_cost(skill_id, rank, consume_id, consume_count, inventory, phase, repeat):
    inventory = [dict(i) for i in inventory]
    if sum(i['count'] for i in inventory if i['template_id'] == consume_id) < consume_count:
        return dict(accepted=False, inventory=inventory, consumed={}, draws=[])
    remaining = consume_count
    consumed = {}
    for item in sorted(inventory, key=lambda i: i['instance_id']):
        if item['template_id'] != consume_id:
            continue
        take = min(item['count'], remaining)
        item['count'] -= take
        remaining -= take
        if take:
            consumed[item['instance_id']] = take
    inventory = [i for i in inventory if i['count']]
    return dict(accepted=True, inventory=inventory, consumed=consumed,
                completion_consumed={}, repeated_consumed={}, phase=phase, draws=[])


def escape_timing(definition, start_tick, casting_speed, available, resources, incarnation):
    skill = definition['skills'][0]
    hit = skill['hit_time_ms']
    animation = int(F(hit) if skill['static'] else F(hit*333, casting_speed))
    if animation < 500 and hit > 500:
        animation = 500
    cost = skill['item_consume_count']
    if available < cost:
        return dict(accepted=False, inventory=available, consumed=0, draws=[])
    return dict(accepted=True, animation_ms=animation, launch_ms=max(0, animation-400),
                launch_tick=start_tick+(max(0, animation-400)+99)//100,
                completion_tick=start_tick+max(1, (animation+99)//100),
                interrupt_cutoff_tick=start_tick+max(0, animation//100-2),
                consumed=cost, completion_consumed=0, inventory=available-cost,
                destination=[126, 126], resources=resources, incarnation=incarnation,
                movement_canceled=True, auto_canceled=True, town_respawn=False, draws=[])


def dual_plan(hits, formulas, second_valid, hit_residues, hit_chance, spread_radius):
    if hits != [r <= hit_chance for r in hit_residues]:
        raise ValueError('inconsistent dual hit residues')
    draws = [dict(site='hit'+str(i+1), bound=1000, residue=r) for i, r in enumerate(hit_residues)]
    damage = []
    for index, (hit, formula) in enumerate(zip(hits, formulas), 1):
        if hit:
            draws += [dict(site=f'critical{index}', bound=1000, residue=999),
                      dict(site=f'spread{index}', bound=2*spread_radius+1, residue=spread_radius)]
        damage.append(int(F(formula))//2 if hit else 0)
    return dict(plan=damage, stored_second=damage[1], applied=[damage[0], damage[1] if second_valid else None],
                second_recomputed=False, draws=draws, refunded_draws=[])


def enchant_safe(current, fullarmor, safe_enchant, safe_fullarmor, base, residue):
    chance = (F(100) if current < safe_enchant or fullarmor and current < safe_fullarmor else min(F(100), F(base)))*1000
    success = residue < chance
    return dict(success=success, enchant=current+int(success), chance=chance,
                draws=[dict(site='enchant', bound=100000, residue=residue)])


def quantized_enchant(base, bonus, support):
    percent = min(F(100), F(base)+F(bonus)+F(support))
    cut = percent*1000
    if cut.denominator != 1:
        raise ValueError('unrepresentable enchant chance')
    mismatches = sum((r < cut) != (100*F(r, 100000) < percent) for r in range(100000))
    quotient, extra = divmod(2**32, 100000)
    return dict(compared_residues=100000, mismatches=mismatches, successful_residues=int(cut),
                preimages=int(cut)*quotient+min(int(cut), extra), draws=[])


def augmentation_provider(option_id, option_xml, event, other_provider, hp, maximum, stat_baseline):
    node = ET.fromstring(option_xml)
    install = event in {'install', 'death', 'dead_admission', 'resurrection'}
    funcs = [ET.tostring(n, encoding='unicode') for n in node.findall('for/*')]
    skills = [dict(n.attrib, kind=n.tag) for n in node if n.tag in {'active_skill', 'passive_skill', 'attack_skill', 'critical_skill', 'magic_skill'}]
    stats = {k: F(v) for k, v in stat_baseline.items()}
    if install:
        for function in node.findall('for/*'):
            key, value = function.get('stat'), F(function.get('val'))
            if function.tag == 'add':
                stats[key] += value
            elif function.tag == 'mul':
                stats[key] *= value
            else:
                raise ValueError('unsupported option function')
    return dict(provider=option_id if install else None, funcs=funcs if install else [],
                skills=skills if install else [], other_provider=other_provider,
                stats=stats, stats_q={k: int(v*Q) for k, v in stats.items()},
                granted_active=[s for s in skills if s['kind'] == 'active_skill'] if install else [],
                system_passive=[s for s in skills if s['kind'] == 'passive_skill'] if install else [],
                triggers=[s for s in skills if s['kind'] in {'attack_skill', 'critical_skill', 'magic_skill'}] if install else [],
                hp=min(F(hp), F(maximum)), on_start_replayed=False, installs=1 if install else 0, draws=[])


def retail_draw_contract(accessory):
    return dict(draw_sites=[dict(site='option12', bound=10000), dict(site='category', bound=10000),
                           dict(site='option34', bound=10000)],
                category_weights=[5500, 3500, 900, 100] if accessory else [5500, 3500, 700, 300],
                non_retail_draws=[])


def item_regen_status(base, level_modifier, stat_bonus, posture_modifier, regen_modifier,
                      periodic_credit, current, maximum, remainder_q, dead, stunned, invulnerable=False, hp_blocked=False):
    # Existing P3 phase order: regen then completion; status gates admission, not regen.
    rate = max(F(1), F(base)*F(level_modifier)*F(stat_bonus))*F(posture_modifier)*F(regen_modifier)
    trace = []
    for site, credit in [('regen', rate), ('item_restore', F(periodic_credit))]:
        whole, carry = divmod(remainder_q+int(credit*Q), Q)
        blocked = dead or site == 'item_restore' and (invulnerable or hp_blocked)
        granted = 0 if blocked else min(whole, max(0, maximum-current))
        current += granted
        if not blocked:
            remainder_q = 0 if current >= maximum else carry
        trace.append(dict(site=site, credit=credit, credit_q=int(credit*Q), granted=granted,
                          current=current, remainder_q=remainder_q))
    return dict(rate=rate, trace=trace, current=current, remainder_q=remainder_q,
                new_use_allowed=not dead and not stunned, draws=[])


def add_behavior_vectors(src, add, o):
    bow_sources = ['G/model/itemcontainer/Inventory.java:679-694', 'G/model/items/L2Item.java:387-391',
                   'G/model/items/L2Weapon.java:91-106', 'G/model/actor/L2Character.java:724-757,1012-1021',
                   'G/model/actor/instance/L2PcInstance.java:5280-5360', 'design/2.1 projectile costs at accepted start']
    src.read(G+'model/itemcontainer/Inventory.java', True)
    src.read(G+'model/items/L2Weapon.java', True)
    bows = [i for i in sorted(src.items) if src.properties(i).get('weapon_type') == 'BOW'
            and src.properties(i).get('for_npc', 'false') != 'true' and src.items[i][2].find('cond') is None]
    arrows = [i for i in sorted(src.items) if src.properties(i).get('etcitem_type') == 'ARROW']
    for grade in src.grades:
        sid = next(i for i in bows if src.properties(i).get('crystal_type', 'NONE') == grade)
        weapon = bow_definition(src, sid)
        ag = 'S' if grade in {'S80', 'S84'} else grade
        aid = next(i for i in arrows if src.properties(i).get('crystal_type', 'NONE') == ag)
        wrong = next(i for i in arrows if src.properties(i).get('crystal_type', 'NONE') != ag)
        for state in ['equipped', 'refill', 'empty', 'wrong', 'wrong-equipped', 'exhausted']:
            inv = [] if state == 'empty' else [arrow_stack(src, wrong if state.startswith('wrong') else aid,
                                                        1 if state == 'exhausted' else 2)]
            for miss in [False, True]:
                inputs = dict(weapon=weapon, inventory=inv, lhand=1 if state in {'equipped', 'wrong-equipped'} else None,
                              mp=F(10), mp_modifier=F(1), reduction_residue=None, miss=miss, reuse_remaining=0, hit_chance=800, hit_residue=801 if miss else 800)
                result = bow_launch(**inputs)
                add('F10', f'bow-launch-{grade}-{state}-{miss}', inputs, result,
                    [src.citation(sid), src.citation(aid), src.citation(wrong), *bow_sources], result['draws'])
        inputs = dict(weapon=weapon, arrow=arrow_stack(src, aid, 1), mp=F(20))
        result = bow_refill(**inputs)
        add('F10', f'bow-refill-history-{grade}', inputs, result, [src.citation(sid), src.citation(aid), *bow_sources], result['draws'])
    sid = next(i for i in bows if int(src.properties(i).get('reduced_mp_consume', '0,0').split(',')[1]) > 0)
    weapon = bow_definition(src, sid)
    aid = next(i for i in arrows if src.properties(i).get('crystal_type', 'NONE') == weapon['grade'])
    for residue in [0, weapon['reduced_chance']-1, weapon['reduced_chance'], 99]:
        for cost in sorted({weapon['mp_cost'], weapon['reduced_cost']}):
            for delta in [F(-1, Q), F(0), F(1, Q)]:
                for modifier in [F(1), F(3, 2), F(0)]:
                    mp = max(F(0), int(F(cost)*modifier)+delta)
                    inputs = dict(weapon=weapon, inventory=[arrow_stack(src, aid, 2)], lhand=None,
                                  mp=mp, mp_modifier=modifier, reduction_residue=residue, miss=True, reuse_remaining=0, hit_chance=800, hit_residue=801)
                    result = bow_launch(**inputs)
                    add('F10', f'bow-mp-{sid}-{residue}-{cost}-{delta}-{modifier}', inputs, result,
                        [src.citation(sid), src.citation(aid), *bow_sources], result['draws'])
    for state in ['empty', 'reuse']:
        inputs = dict(weapon=weapon, inventory=[] if state == 'empty' else [arrow_stack(src, aid, 2)],
                      lhand=None, mp=F(10), mp_modifier=F(1), reduction_residue=0, miss=False,
                      reuse_remaining=1 if state == 'reuse' else 0, hit_chance=800, hit_residue=800)
        result = bow_launch(**inputs)
        add('F10', 'bow-mp-skip-'+state, inputs, result, [src.citation(sid), *bow_sources], result['draws'])
    definition = o.consumable_definition(src, 737)
    skill = src.skills[2014][1]
    effect = skill.find('effects/effect')
    power = F(next((n.get('power') for n in effect.findall('param') if n.get('power') is not None), '0'))
    config = src.read('src/main/resources/config/character.properties', True)
    import re
    restore_hp = F(re.search(r'^RespawnRestoreHP\s*=\s*(\S+)', config, re.M)[1])
    src.read('src/main/java/com/l2jserver/datapack/handlers/effecthandlers/instant/Resurrection.java')
    res_sources = [src.citation(737), D+'stats/skills/02000-02099.xml:189-211',
                   'H/effecthandlers/instant/Resurrection.java:37-41,54-67',
                   'G/model/stats/Formulas.java:1367-1382', 'G/model/actor/L2Character.java:2098-2132',
                   'GC:RespawnRestoreHP', 'design/8 scroll consumed before offer; no decline/expiry refund',
                   'design/phase-3-zone-design.md:132 first-valid/id/death incarnation/600-tick lifetime']
    base = dict(definition=definition, start_tick=10, available=2, death_incarnation=7,
                current_incarnation=7, dead=True, response='offer', response_tick=160,
                response_offer_id=1, existing_offer=None, repeat=0, hp=F(0), max_hp=101,
                mp=F(13, 2), cp=F(17, 2), recoverable_xp=101, restore_percent=power, restore_hp=restore_hp)
    branches = [('offer', {}), ('decline', dict(response='decline')), ('accept', dict(response='accept', repeat=1)),
                ('stale-id', dict(response='accept', response_offer_id=2)),
                ('stale-death', dict(current_incarnation=8, response='accept')),
                ('alive', dict(dead=False, response='accept')), ('empty', dict(available=0))]
    for response in ['accept', 'decline', 'expire']:
        for delta in [-1, 0, 1]:
            branches.append((f'{response}-expiry-{delta}', dict(response=response, response_tick=760+delta)))
    branches.append(('first-valid', dict(existing_offer=dict(offer_id=9, target_incarnation=7, created_at=1,
                    expires_at=601, restore_percent=F(50), recoverable_xp=101), response='accept', response_offer_id=9, repeat=1)))
    for name, changes in branches:
        inputs = dict(base, **changes)
        add('F12', 'resurrection-scroll-'+name, inputs, resurrection_scroll(**inputs), res_sources)
    for sid in [736, 1538]:
        for speed in [333, 1500]:
            inputs = dict(definition=o.consumable_definition(src, sid), start_tick=10, casting_speed=speed,
                          available=2, resources={'hp': 13, 'mp': 17, 'cp': 19, 'xp': 101}, incarnation=7)
            add('F12', f'consumable-escape-timing-{sid}-{speed}', inputs, escape_timing(**inputs),
                [src.citation(sid), D+'stats/skills/02000-02099.xml:163-187,440-462',
                 'G/model/actor/L2Character.java:1625-1643,4577-4581', 'design/8 Escape timing/destination; no town respawn'])
    # D12 names tutorial/teleport handlers outside the admitted world contexts.
    for kit in src.xml(D+'stats/initialEquipment.xml').findall('equipment'):
        cid = int(kit.get('classId'))
        if src.races[cid] == 'KAMAEL':
            continue
        for n in kit.findall('item'):
            sid = int(n.get('id'))
            p = src.properties(sid)
            if p.get('handler') != 'Book' and p.get('item_skill') not in {'2531-1', '2525-1'}:
                continue
            inputs = dict(template_id=sid, properties=p,
                          inventory=[dict(instance_id=1, template_id=sid, count=int(n.get('count'))),
                                     dict(instance_id=2, template_id=57, count=10)], selected_instance=1,
                          paperdoll={'rhand': 3}, allocator_next=4, reuse={'737': 800})
            add('F12', f'dormant-starter-use-{cid}-{sid}', inputs, dormant_use(**inputs),
                [src.citation(sid), D+'stats/initialEquipment.xml:4-109', 'design/9.1 D12 deferred UseItem; §12.1'])
    add_audit_vectors(src, add, o)


def add_audit_vectors(src, add, o):
    """Remaining separate clauses discovered by the line-by-line §12.1 audit."""
    config = src.read('src/main/resources/config/character.properties', True)
    import re
    safe = 1+max(hi for lo, hi, rate in src.rates['ARMOR_GROUP'] if rate == 100)
    full = 1+max(hi for lo, hi, rate in src.rates['FULL_ARMOR_GROUP'] if rate == 100)
    for body in [False, True]:
        for e in [2, 3, 4, 5]:
            inputs = dict(current=e, fullarmor=body, safe_enchant=safe, safe_fullarmor=full, base=F(0), residue=99999)
            result = enchant_safe(**inputs)
            add('F4', f'enchant-safe-{body}-{e}', inputs, result,
                ['G/model/items/enchant/EnchantScroll.java:137-157', 'D/enchantItemGroups.xml:4-59'], result['draws'])
    for base, bonus, support in [(F(0), F(0), F(0)), (F(6667, 100), F(0), F(0)), (F(100), F(0), F(0)), (F(0), F(10), F(5))]:
        inputs = dict(base=base, bonus=bonus, support=support)
        add('F4', f'enchant-quantized-{base}-{bonus}-{support}', inputs, quantized_enchant(**inputs),
            ['G/model/items/enchant/EnchantScroll.java:147-157', 'design/4.2 quantized inequality and modulo bias'])
    for con, bonus in src.bonuses['CON'].items():
        for limit in [F(1, 2), F(3, 2)]:
            inputs = dict(con=con, bonus=bonus, weight_limit=limit, multiplier=F(3, 2), add=F(1, 3))
            add('F7', f'load-modifier-{con}-{limit}', inputs,
                dict(pre_fold=int(bonus*69000*limit//1), maximum=o.max_load(bonus, limit, F(3, 2), F(1, 3))),
                ['G/model/actor/instance/L2PcInstance.java:9427-9438', 'design/6 floor pre-fold/truncate getter'])
    for slot in range(25):
        for expertise in range(8):
            entries = [dict(instance_id=1, slot=slot, grade=7, weapon=slot == 5)]
            add('F6', f'grade-slot-{slot}-{expertise}', dict(equipped=entries, expertise=expertise),
                o.grade_penalties(entries, expertise), ['G/model/actor/instance/L2PcInstance.java:1707-1745'])
    for speed in [379, 400, 1500]:
        inputs = dict(reuse_delay=1500, attack_speed=speed, weapon_reuse_modifier=F(3, 2))
        add('F10', f'bow-reuse-speed-{speed}', inputs, o.bow_reuse(**inputs), ['G/model/actor/L2Character.java:4353-4357'])
    for hits in [[False, False], [True, False], [False, True], [True, True]]:
        for valid in [False, True]:
            inputs = dict(hits=hits, formulas=[F(499, 10), F(10199, 100)], second_valid=valid,
                          hit_residues=[800 if h else 801 for h in hits], hit_chance=800, spread_radius=6)
            result = dual_plan(**inputs)
            add('F10', f'dual-plan-{hits}-{valid}', inputs, result,
                ['G/model/actor/L2Character.java:1137-1182,5034-5044', 'G/model/stats/Formulas.java:1054-1063',
                 'design/2.2 I3 stored second resolution; shieldless/noncritical/neutral-spread caller'], result['draws'])
    for count in [0, 1, 2]:
        candidates = [dict(eligible=True, hit=True, formula=F(499, 10)) for _ in range(4)]
        add('F10', f'pole-count-{count}', dict(candidates=candidates, maximum=count), o.pole_sequence(candidates, count),
            ['G/model/actor/L2Character.java:1237-1287,1335-1338'])
    for chance in [F(0), F(999, 10), F(100), F(101)]:
        for residue in [998, 999]:
            inputs = dict(chance=chance, dex=F(1), proximity='front', residue=residue)
            add('F10', f'blow-boundary-{chance}-{residue}', inputs, o.blow_success(**inputs), ['G/model/stats/Formulas.java:1621-1642'])
    for power in [F(-1), F(0), F(79, 77), F(80, 77), F(81, 77), F(199, 77)]:
        for critical in [False, True]:
            inputs = dict(p_atk=0, defence=80, power=power, caller='blow', critical=critical)
            formula = o.physical_damage(**dict(inputs, critical=False))
            value = formula*(2 if critical else 1)
            add('F10', f'blow-cap-rounding-{power}-{critical}', inputs,
                dict(formula=formula, after_handler_critical=value, getter=int(value), damage_q=int(value*Q)),
                ['G/model/stats/Formulas.java:502-578', 'H/effecthandlers/instant/FatalBlow.java:74-87'])
    for accessory in [False, True]:
        add('F11', f'retail-draw-contract-{accessory}', dict(accessory=accessory), retail_draw_contract(accessory),
            ['G/datatables/AugmentationData.java:447-558,689-737', 'GC:393,405 retail enabled; design/7.2 no non-retail draws'])
    src.read(G+'model/options/Options.java', True)
    options = {}
    for path in sorted((src.dp/(D+'stats/options')).glob('*.xml')):
        file = path.relative_to(src.dp).as_posix()
        for node in src.xml(file).findall('option'):
            for tag in ['for', 'active_skill', 'passive_skill', 'attack_skill', 'critical_skill', 'magic_skill']:
                if tag not in options and node.find(tag) is not None:
                    options[tag] = (int(node.get('id')), file, ET.tostring(node, encoding='unicode'))
    if set(options) != {'for', 'active_skill', 'passive_skill', 'attack_skill', 'critical_skill', 'magic_skill'}:
        raise ValueError('missing required option provider kind')
    for kind, (oid, file, xml) in options.items():
        for event in ['install', 'unequip', 'cancel', 'death', 'dead_admission', 'resurrection']:
            inputs = dict(option_id=oid, option_xml=xml, event=event, other_provider=99, hp=F(100), maximum=75,
                          stat_baseline={n.get('stat'): F(100) for n in ET.fromstring(xml).findall('for/*')})
            add('F11', f'augmentation-provider-{kind}-{event}', inputs, augmentation_provider(**inputs),
                [file, 'G/model/options/Options.java:61-74,136-178', 'design/3.2 D16 System lifecycle; §7.2'])
    sid = 1401
    node = src.skills[sid][1]
    sets = {n.get('name'): n.get('val') for n in node.findall('set')}
    for phase in ['complete', 'cancel']:
        for available in [0, 1, 2]:
            inputs = dict(skill_id=sid, rank=1, consume_id=int(sets['itemConsumeId']), consume_count=int(sets['itemConsumeCount']),
                          inventory=[] if available == 0 else [dict(instance_id=2, template_id=3031, count=available),
                                     dict(instance_id=1, template_id=3031, count=1)], phase=phase, repeat=True)
            add('F12', f'learned-item-consume-{phase}-{available}', inputs, learned_item_cost(**inputs),
                [src.skills[sid][0], 'G/model/actor/L2Character.java:1560-1566', 'design/8 lowest owned IDs/prepay once'])
    human = D+'stats/chars/baseStats/HumanFighter.xml'
    root = src.xml(human)
    base = F(root.find('lvlUpgainData').find('level').findtext('hpRegen'))
    bonus = src.bonuses['CON'][int(root.findtext('staticData/baseCON'))]
    potion = o.consumable_definition(src, 1540)
    restore = F(potion['skills'][0]['effects'][0]['params']['amount'])
    src.read('src/main/java/com/l2jserver/datapack/handlers/effecthandlers/instant/Hp.java')
    for posture, modifier in [('standing', F(11, 10)), ('running', F(7, 10)), ('sitting', F(3, 2))]:
        for dead, stunned in [(False, False), (False, True), (True, False)]:
            for current, maximum in [(0, 1000), (99, 100), (999, 1000)]:
                inputs = dict(base=base, level_modifier=F(9, 10), stat_bonus=bonus, posture_modifier=modifier,
                              regen_modifier=F(1, 2), periodic_credit=restore, current=current, maximum=maximum,
                              remainder_q=500000, dead=dead, stunned=stunned)
                add('F12', f'item-regen-status-{posture}-{dead}-{stunned}-{current}', inputs, item_regen_status(**inputs),
                    [human, src.citation(1540), potion['skills'][0]['source'], 'G/model/stats/Formulas.java:263-351', D+'stats/skills/04200-04299.xml:1623-1639',
                     'design/2.1 regen before item completion; P3 Q accumulation; §8 status admission'])
    for guard in ['invulnerable', 'hp_blocked']:
        inputs = dict(base=base, level_modifier=F(9, 10), stat_bonus=bonus, posture_modifier=F(11, 10),
                      regen_modifier=F(1, 2), periodic_credit=restore, current=0, maximum=1000,
                      remainder_q=500000, dead=False, stunned=False, invulnerable=guard == 'invulnerable', hp_blocked=guard == 'hp_blocked')
        add('F12', 'item-regen-status-'+guard, inputs, item_regen_status(**inputs),
            [human, src.citation(1540), potion['skills'][0]['source'], 'H/effecthandlers/instant/Hp.java:60-89',
             'G/model/stats/Formulas.java:263-351', 'design/2.1 regen and status guard'])
