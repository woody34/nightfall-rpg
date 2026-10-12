#!/usr/bin/env python3
"""Source-bound V6 fixture inputs; no gameplay results or live inventory mutation."""
import argparse
from functools import lru_cache
import importlib.util
import json
from pathlib import Path
import tomllib

REPO = Path(__file__).resolve().parents[3]
_spec = importlib.util.spec_from_file_location('phase4_phase3', Path(__file__).with_name('phase3-fixtures.py'))
phase3 = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(phase3)
soak_lane = phase3.soak_lane
PHASE4 = frozenset('phase4-' + name for name in ('starter equip armor shield bow grade overload enchant crystallize shots consumables scrolls resurrection blacksmith learn grant-denied kinds').split())
BASE_PACKS = PHASE4
VARIANT_PACKS = {
    'phase4-equip-ring': {'base': 'phase4-equip', 'equipment_case': 'ring'},
    'phase4-equip-onepiece': {'base': 'phase4-equip', 'equipment_case': 'onepiece'},
    'phase4-equip-twohand': {'base': 'phase4-equip', 'equipment_case': 'twohand'},
    'phase4-bow-last-arrow': {'base': 'phase4-bow', 'arrows': 1},
    **{'phase4-starter-' + name: {'base': 'phase4-starter', 'profession': name}
       for name in ('human_fighter', 'human_mystic', 'elven_fighter', 'elven_mystic', 'dark_fighter', 'dark_mystic', 'orc_fighter', 'orc_mystic', 'dwarven_fighter')},
    **{'phase4-equip-level' + str(level): {'base': 'phase4-equip', 'level': level}
       for level in (1, 3, 4, 8, 9, 15, 16)},
    **{'phase4-armor-enchant' + str(level): {'base': 'phase4-armor', 'enchant': level}
       for level in (4, 6)},
}
PHASE4 = PHASE4 | VARIANT_PACKS.keys()
PAIRED = frozenset(('phase4-shield', 'phase4-resurrection'))
OWNER = '01970000-0000-7000-8000-000000000040'
OBSERVER = '01970000-0000-7000-8000-000000000041'
# Harness policy, reserved by D8 / item.rs::ITEM_ID_RANGE_WIDTH. No production range row.
RANGE_START, RANGE_END = 1, 2**32
ROLE_STRIDE = 10000


@lru_cache(maxsize=None)
def table(name, repo=REPO):
    return tomllib.loads((repo / 'packages/data/tables-phase4' / (name + '.toml')).read_text())


@lru_cache(maxsize=None)
def catalogue(repo=REPO):
    return {r['template_id']: r for name in ('weapons', 'armors', 'etc_items') for r in table(name, repo)['items']}


@lru_cache(maxsize=None)
def capabilities(repo=REPO):
    return {r['template_id']: r for r in table('capabilities', repo)['items']}


def select(repo, **conditions):
    return next(r for i, r in sorted(catalogue(repo).items())
                if capabilities(repo)[i]['admitted'] and all(r[k] == v for k, v in conditions.items()))


def selection(repo=REPO):
    """Stable minimum eligible IDs (design §12.1); fail if source closure is absent."""
    result = {kind.lower(): select(repo, kind='Weapon', weapon_type=kind, grade='NONE')['template_id']
              for kind in ('SWORD', 'BLUNT', 'DAGGER', 'BOW', 'DUAL', 'DUALFIST', 'POLE')}
    for name, conditions in {
        'shield': dict(armor_type='SHIELD', grade='NONE'),
        'arrow': dict(etcitem_type='ARROW', grade='NONE'),
        'soulshot': dict(handler='SoulShots', grade='NONE'),
        'spiritshot': dict(handler='SpiritShot', grade='NONE'),
        'blessed_spiritshot': dict(handler='BlessedSpiritShot', grade='NONE'),
        'd_weapon': dict(kind='Weapon', weapon_type='SWORD', grade='D'),
        'ring': dict(bodypart='rfinger;lfinger', grade='NONE'),
        'onepiece': dict(bodypart='onepiece'),
        'twohand': dict(kind='Weapon', weapon_type='SWORD', bodypart='lrhand', grade='NONE'),
        'c_weapon': dict(kind='Weapon', weapon_type='SWORD', grade='C'),
        'overgrade': dict(kind='Weapon', weapon_type='SWORD', grade='S'),
    }.items(): result[name] = select(repo, **conditions)['template_id']
    # Exact named source rows for scrolls and player HP potion; validate admission below.
    for name, title in [('escape', 'Scroll of Escape'), ('resurrect', 'Scroll of Resurrection'),
                        ('potion', 'Healing Potion'), ('blessed_escape', 'Blessed Scroll of Escape')]:
        result[name] = next(i for i, r in sorted(catalogue(repo).items())
                            if r['name'] == title and capabilities(repo)[i]['admitted'])
    scrolls = table('enchant_scrolls', repo)['scrolls']
    for name, blessed, safe in [('scroll', False, False), ('blessed_scroll', True, False), ('safe_scroll', False, True)]:
        result[name] = min(r['template_id'] for r in scrolls if r['weapon'] and r['target_grade'] == 'C'
                           and r['blessed'] == blessed and r['safe'] == safe and capabilities(repo)[r['template_id']]['admitted'])
    return result


def armor_set(repo=REPO):
    """D20 provider closure, selected from E1 including admitted skills."""
    caps=capabilities(repo);skills={r['id']:r for r in table('capabilities',repo)['skills']}
    for row in sorted(table('armor_sets',repo)['sets'],key=lambda r:r['id']):
        if not row['shield_skills'] or not row['enchant6_skills']:continue
        required=[]
        for slot in ('chest','legs','head','gloves','feet','shield'):
            eligible=[i for i in row[slot] if caps[i]['admitted']]
            if row[slot] and not eligible:break
            if eligible:required.append(min(eligible))
        else:
            if all(skills[r['id']]['admitted'] for kind in ('skills','shield_skills','enchant6_skills') for r in row[kind]):return row,required
    raise ValueError('no admitted shield/enchant6 provider set')


def enchant4_item(repo=REPO):
    skills={r['id']:r for r in table('capabilities',repo)['skills']}
    return next(r for i,r in sorted(catalogue(repo).items()) if capabilities(repo)[i]['admitted']
                and r['enchant4_skills'] and all(skills[s['id']]['admitted'] for s in r['enchant4_skills']))


def combat_skill_candidates(repo=REPO):
    """Do not seed a skill until E1 admits its full source closure (D5)."""
    caps={r['id']:r for r in table('capabilities',repo)['skills']};result=[]
    for path in sorted((repo/'packages/data/skills-phase4').glob('*.toml')):
        text=path.read_text()
        if not any('name = "'+name+'"' in text for name in ('FatalBlow','Backstab','Lethal')):continue
        row=tomllib.loads(text)
        result.append(dict(skill_id=row['id'],source=row['provenance'],admitted=caps[row['id']]['admitted'],blockers=caps[row['id']]['blockers']))
    return result


def recipe(repo=REPO):
    admitted = {r['recipe_id'] for r in table('capabilities', repo)['recipes'] if r['admitted']}
    return next(r for r in table('sa_variants', repo)['recipes'] if r['recipe_id'] in admitted and r['base_template'] == 72 and r['products'][0]['template_id'] == 4682)


def role(repo, character=OWNER, profession='human_mystic', level=14, position=(110, 100)):
    definition = tomllib.loads((repo / f'packages/data/professions/{profession}.toml').read_text())
    xp = next(r['xp'] for r in tomllib.loads((repo / 'packages/data/tables/experience.toml').read_text())['to_level'] if r['level'] == level)
    base = phase3.profiles('phase3-trainer', repo)['owner']
    return base | dict(character_id=character, profession=profession, class_id=definition['id'],
                       base_class_id=definition['base_class_id'], class_profile=profession, race=definition['race'],
                       base_stats=definition['base_stats'], level=level, xp=xp, sp=100000,
                       position=list(position), facing=[1, 0], learned_ranks={'1177': 1, '1068': 1} if profession == 'human_mystic' else {})


def inventory(repo, base_class, index, extras=()):
    """Source kit counts/flags plus explicit test inputs; aggregate stackable extras."""
    kit = next(k for k in table('starter_kits', repo)['kits'] if k['class_id'] == base_class)
    inputs = [(r['template_id'], r['count'], 0, r['equipped'], r['source']) for r in kit['items']]
    inputs += [(i, n, e, False, catalogue(repo)[i]['source']) for i, n, e in extras]
    rows = []
    for template, count, enchant, equipped, source in inputs:
        definition = catalogue(repo)[template]
        if not capabilities(repo)[template]['storable']: raise ValueError(f'fixture item not storable: {template}')
        if type(count) is not int or count <= 0 or type(enchant) is not int or not 0 <= enchant < 2**32: raise ValueError('invalid fixture count/enchant')
        previous = next((r for r in rows if r['template_id'] == template), None)
        if previous and definition['stackable']:
            previous['count'] += count
            continue
        if not definition['stackable'] and count != 1: raise ValueError('nonstackable fixture count')
        slot = definition['bodypart'] if equipped else None
        slot = {'lrhand': 'rhand', 'fullarmor': 'chest', 'onepiece': 'chest'}.get(slot, slot)
        rows.append(dict(instance_id=index * ROLE_STRIDE + len(rows) + 1, template_id=template, count=count,
                         enchant=enchant, augment_id=None, charge_bits=0, location='paperdoll' if equipped else 'inventory',
                         slot=slot, revision=0, source=source))
    return rows


def profiles(pack, repo=REPO, paired=False):
    if pack not in PHASE4: raise ValueError('unknown Phase4 pack')
    if pack in PAIRED and not paired: raise ValueError('Phase4 pack requires cooperating roles')
    config = VARIANT_PACKS.get(pack, {})
    pack = config.get('base', pack)
    s = selection(repo)
    extras = []
    owner = role(repo, profession=config.get('profession', 'human_mystic'), level=config.get('level', 14))
    if pack == 'phase4-equip':
        extras = [(s['sword'], 1, 0), (s['dagger'], 1, 0)]
        case = config.get('equipment_case')
        if case == 'ring': extras += [(s['ring'], 1, 0), (s['ring'], 1, 0)]
        if case == 'onepiece': extras += [(s['onepiece'], 1, 4)]
        if case == 'twohand': extras += [(s['twohand'], 1, 0), (s['shield'], 1, 0)]
    elif pack == 'phase4-armor':
        extras=[(i,1,6 if config.get('enchant')==6 else 0) for i in armor_set(repo)[1]]
        if config.get('enchant')==4:extras.append((enchant4_item(repo)['template_id'],1,4))
    elif pack == 'phase4-shield': extras = [(s['sword'], 1, 0)]
    elif pack == 'phase4-bow': extras = [(s['bow'], 1, 0), (s['arrow'], config.get('arrows', 2), 0)]
    elif pack == 'phase4-grade': extras = [(s['overgrade'], 1, 0)]
    elif pack == 'phase4-overload':
        # Weight is an input. Exact overload ranks are checked externally with actual maxLoad.
        extras = [(s['arrow'], 100000, 0)]
    elif pack == 'phase4-enchant':
        extras = [(s['c_weapon'], 1, 0), (s['c_weapon'], 1, 3)]
        extras += [(s[k], 2, 0) for k in ('scroll', 'blessed_scroll', 'safe_scroll')]
    elif pack == 'phase4-crystallize':
        extras = [(s['d_weapon'], 1, 0), (s['d_weapon'], 1, 0)]
        owner = role(repo, profession='dwarven_fighter', level=20, position=(126, 128))
        owner['tokens'] = [1, 0]
    elif pack == 'phase4-learn':
        owner = role(repo, level=52, position=(126, 128))
        owner['tokens'] = [1, 1]
        owner['book_learn'] = dict(skill=1405, rank=1, template=8618, count=1, source='DP@3ca488dd/src/main/resources/data/skillTrees/classSkillTree.xml:11947-11949; professions/bishop.toml:48-55')
    elif pack == 'phase4-shots': extras = [(s[k], 100, 0) for k in ('soulshot', 'spiritshot', 'blessed_spiritshot')]
    elif pack == 'phase4-consumables':
        extras = [(s['potion'], 2, 0)]; owner.update(hp=1, mp=1, cp=1)
    elif pack == 'phase4-scrolls': extras = [(s[k], 2, 0) for k in ('escape', 'blessed_escape')]
    elif pack == 'phase4-resurrection': extras = [(s['resurrect'], 2, 0)]
    elif pack == 'phase4-blacksmith':
        r = recipe(repo)
        extras = [(x['template_id'], x['count'], 3 if x['template_id'] == r['base_template'] else 0) for x in r['ingredients']]
        stone = next(x for x in table('augmentation_chances', repo)['stones'] if x['kind'] == 'weapon')
        cost = next(x for x in table('augmentation_chances', repo)['costs'] if x['grade'] == catalogue(repo)[r['base_template']]['grade'])
        extras += [(stone['template_id'], 1, 0), (cost['gemstone_id'], cost['weapon_count'], 0), (57, 210000, 0)]
        owner = role(repo, level=stone['level'], position=(126, 128))
    elif pack == 'phase4-kinds':
        extras=[(s[k],1,0) for k in ('dual','dualfist','pole','dagger')]
        owner=role(repo,profession='human_fighter',level=14)
        owner['combat_capability_dependencies']=combat_skill_candidates(repo)
    result = {'owner': owner}
    if paired:
        result['observer'] = role(repo, OBSERVER, position=(112, 100))
    for index, (name, value) in enumerate(result.items()):
        supplied = extras if name == 'owner' else [(s['shield'], 1, 0)] if pack == 'phase4-shield' else []
        value['inventory'] = inventory(repo, value['base_class_id'], index, supplied)
        value['paperdoll'] = {r['slot']: r['instance_id'] for r in value['inventory'] if r['slot']}
        value['auto_shots'] = []
        value['ledger'] = [dict(op='grant', instance_id=r['instance_id'], before=None, after={k:v for k,v in r.items() if k != 'source'}) for r in value['inventory']]
        value['allocator'] = dict(range_start=RANGE_START, next_item_instance=max(r['instance_id'] for r in value['inventory']) + 1, range_end_exclusive=RANGE_END)
    return result


def soak_profile(repo, account, character, index, clients):
    value = phase3.soak_profile(repo, account, character, index, clients)
    s = selection(repo)
    value['inventory'] = inventory(repo, value['base_class_id'], index,
                                   [(s['sword'], 1, 0), (s['c_weapon'], 1, 0), (s['scroll'], 1, 0),
                                    (s['soulshot'], 1000, 0), (s['spiritshot'], 1000, 0)])
    value['paperdoll'] = {r['slot']: r['instance_id'] for r in value['inventory'] if r['slot']}
    value['auto_shots'] = []
    value['allocator'] = dict(range_start=RANGE_START, next_item_instance=max(r['instance_id'] for r in value['inventory'])+1, range_end_exclusive=RANGE_END)
    return value


def declared_units(directory):
    if not (directory / 'phase4-roles.json').is_file(): return {}
    data = json.loads((directory / 'phase4-roles.json').read_text())
    if data.get('schema_version') != 1: raise ValueError('invalid Phase4 roles')
    seen = set()
    for name, roles in data['units'].items():
        if len(roles) != 2 or roles[0] != name + '.nfs' or any(Path(r).name != r or not (directory / r).is_file() or r in seen for r in roles): raise ValueError('invalid Phase4 pair')
        seen.update(roles)
    return data['units']


def declared_pair(batch):
    return len(batch) == 2 and batch[0].parent == batch[1].parent and any(set(r) == {p.name for p in batch} for r in declared_units(batch[0].parent).values())


def publish(repo=REPO):
    return dict(schema_version=1, policy='pre_admission_only', selector='stable_minimum_admitted_template',
                inputs_are_test_policy=True, selections=selection(repo),
                data_sha256=table('rules_manifest', repo)['data_sha256'],
                packs={p: profiles(p, repo, p in PAIRED) for p in sorted(PHASE4)})


def validate_seed_manifest(seed, selected, owner, observer, repo, paired):
    expected = {name:r | {'account_id': owner if name == 'owner' else observer} for name,r in profiles(selected, repo, paired).items()}
    if seed.get('schema_version') != 1 or seed.get('fixture') != selected or seed.get('roles') != expected or seed.get('dry_run') is not False or seed.get('admission_ready') is not True:
        raise ValueError('Phase4 seed manifest does not match pre-admission contract')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--publish', type=Path, required=True)
    args = parser.parse_args()
    args.publish.write_text(json.dumps(publish(), indent=2, sort_keys=True) + '\n')


def denied_auth_inputs():
    import os
    from uuid import UUID
    path = Path(os.environ.get('SIM_PHASE4_AUTH_TOKEN_FILE', ''))
    if not path.is_absolute() or not path.is_file() or path.is_symlink():
        raise ValueError('grant-denied requires explicit absolute real-auth token file')
    token = path.read_text().strip()
    if not token or token.startswith('test:') or any(c.isspace() for c in token):
        raise ValueError('grant-denied refuses dev tokens; real authenticating credentials required')
    issuer, audience = os.environ.get('SIM_PHASE4_OIDC_ISSUER'), os.environ.get('SIM_PHASE4_OIDC_AUDIENCE')
    if not issuer or not audience: raise ValueError('grant-denied requires explicit OIDC issuer/audience')
    return dict(token=token, issuer=issuer, audience=audience, account=str(UUID(os.environ.get('SIM_PHASE4_OWNER_ACCOUNT', ''))))


def create_overlay(destination, repo):
    phase3.create_overlay(destination, repo)
    zone = destination / 'zones/test_zone.toml'
    blacksmith = table('capacities', repo)['blacksmith']
    # Stationary fixture targets preserve initial east facing (design §2.1/§6.2).
    target=destination/'npcs/skill_target.toml'
    target.write_text(target.read_text().replace('move_speed = 400','move_speed = 0'))
    # Eligible passive secondary recipients are fixture inputs, not source stats.
    for name,position in [('pole_secondary_a',(108,101)),('pole_secondary_b',(108,99))]:
        template=(destination/'npcs/skill_target.toml').read_text().replace('skill_target',name)
        (destination/('npcs/'+name+'.toml')).write_text(template)
        with zone.open('a') as out:
            out.write(f'\n[[spawn_slots]]\nid = "{name}"\ntemplate = "{name}"\nhome = [{position[0]}, {position[1]}]\ncount = 1\n')
    zone.write_text(zone.read_text() + '\n[[npcs]]\nname = "Blacksmith"\nattackable = false\npos = [' + str(blacksmith['x']) + ', ' + str(blacksmith['y']) + ']\nspeed = 0\n')
