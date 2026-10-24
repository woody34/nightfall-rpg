#!/usr/bin/env python3
"""Source-bound V6 fixture inputs; no gameplay results or live inventory mutation."""
import argparse
from functools import lru_cache
import importlib.util
import json
from pathlib import Path
import re
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
    # T52: the stable minimum admitted C-grade MAGE weapon (MAGE_WEAPON_GROUP: 40% at +3..+14,
    # FIGHTER_WEAPON_GROUP: 70%). The failure units enchant it at +3 so the pinned zone draw
    # (ChaCha12 word 40 of zone 1 epoch 1 = 47096 of 100000) fails deterministically.
    result['c_magic_weapon'] = next(i for i, r in sorted(catalogue(repo).items())
                                    if capabilities(repo)[i]['admitted'] and r['kind'] == 'Weapon' and r['grade'] == 'C'
                                    and r['weapon_type'] == 'SWORD' and r['flags']['is_magic_weapon'])
    # success_no_increment (harness-review-10, D161 revoked): ARMOR_GROUP has base chance 0 from
    # +20; the ancient safe B armour scroll (bonus_rate 100) lifts the chance to 100000, so the
    # zone takes the success branch with no increment whatever the draw. Stable minimum admitted
    # B-grade body armour and the B armour scroll with bonus "100".
    result['b_armor'] = select(repo, kind='Armor', grade='B', bodypart='chest')['template_id']
    result['no_increment_scroll'] = min(r['template_id'] for r in table('enchant_scrolls', repo)['scrolls']
                                        if not r['weapon'] and r['target_grade'] == 'B' and str(r['bonus_rate']) == '100'
                                        and r['safe'] and capabilities(repo)[r['template_id']]['admitted'])
    # Exact named source rows for scrolls and player HP potion; each must be admitted AND usable
    # (no use blockers). D131: Healing Potion 727 has no V6 use handler (use:no_handler ->
    # ItemCapabilityDeferred); the positive potion case is Lesser Healing Potion 1060 (D97).
    for name, title in [('escape', 'Scroll of Escape'), ('resurrect', 'Scroll of Resurrection'),
                        ('potion', 'Lesser Healing Potion'), ('blessed_escape', 'Blessed Scroll of Escape')]:
        result[name] = next((i for i, r in sorted(catalogue(repo).items())
                             if r['name'] == title and capabilities(repo)[i]['admitted']
                             and not capabilities(repo)[i]['use_blockers']), None)
        if result[name] is None:
            raise ValueError(f'no admitted, usable {title!r} template for selection {name!r}')
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
    # The armour-enchant4 variant equips this item into the `under` slot, so the selector is
    # bound to underwear: E1 rev 8 admits weapons with enchant4 skills (e.g. 52) that sort lower.
    skills={r['id']:r for r in table('capabilities',repo)['skills']}
    return next(r for i,r in sorted(catalogue(repo).items()) if capabilities(repo)[i]['admitted']
                and r['bodypart']=='underwear'
                and r['enchant4_skills'] and all(skills[s['id']]['admitted'] for s in r['enchant4_skills']))


BLOW_SKILLS=frozenset((16,263))
ARMOR_LEVEL=20
EXPERTISE=239


def expertise_rank(repo,profession,level):
    """Highest Expertise (239) rank the source skill tree grants at this level (auto-get)."""
    rows=[r for r in tomllib.loads((repo/f'packages/data/professions/{profession}.toml').read_text()).get('skill_tree',[])
          if r['skill_id']==EXPERTISE and r['required_level']<=level and r['auto_get']]
    if not rows:raise ValueError('no expertise at this level')
    return {str(EXPERTISE):max(r['skill_level'] for r in rows)}


def blow_ranks(repo,chain,level):
    """Highest learnable rank of each blow skill over the profession chain (source skill trees)."""
    ranks={}
    for profession in chain:
        for row in tomllib.loads((repo/f'packages/data/professions/{profession}.toml').read_text()).get('skill_tree',[]):
            if row['skill_id'] in BLOW_SKILLS and row['required_level']<=level:
                ranks[str(row['skill_id'])]=max(ranks.get(str(row['skill_id']),0),row['skill_level'])
    if set(ranks)!={str(i) for i in BLOW_SKILLS}:raise ValueError('blow skills not learnable on this chain/level')
    return ranks


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
    # T25: `characters.class_profile` is the legacy Phase 1 combat profile of the LINEAGE, the
    # tier-0 profession of base_class_id (API character_progression::base_class_profile; the
    # repository writes it on creation and class transfer never changes it). A transferred
    # owner seeded with its current profession is refused by the zone load check
    # (state_class.rs loaded_player: base_class_profile(base_class_id) must equal load.class).
    class_profile = next(tomllib.loads(path.read_text())['key'] for path in sorted((repo / 'packages/data/professions').glob('*.toml'))
                         if tomllib.loads(path.read_text())['id'] == definition['base_class_id'] and tomllib.loads(path.read_text())['tier'] == 0)
    return base | dict(character_id=character, profession=profession, class_id=definition['id'],
                       base_class_id=definition['base_class_id'], class_profile=class_profile, race=definition['race'],
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
        # D111: inside the D110 gear_stats export guard. Level 20 gives Expertise D (skill 239,
        # auto-get rank from the source human_mystic tree), so the D-grade set carries no grade
        # penalty; still a base-class player with no timed effect, death penalty or overload.
        owner=role(repo,level=ARMOR_LEVEL)
        owner['learned_ranks']=owner['learned_ranks']|expertise_rank(repo,'human_mystic',ARMOR_LEVEL)
        if config.get('enchant')==4:extras.append((enchant4_item(repo)['template_id'],1,4))
    elif pack == 'phase4-shield':
        extras = [(s['sword'], 1, 0)]
        # T51/D157: the attacker (-a) stands EAST of the defender (-b at 112,100), who faces east
        # (new life) and steps one tile east toward it, so every hit is a front-facing shield roll.
        owner = role(repo, position=(114, 100))
    elif pack == 'phase4-bow': extras = [(s['bow'], 1, 0), (s['arrow'], config.get('arrows', 2), 0)]
    elif pack == 'phase4-grade': extras = [(s['overgrade'], 1, 0)]
    elif pack == 'phase4-overload':
        # Weight is an input. Exact overload ranks are checked externally with actual maxLoad.
        extras = [(s['arrow'], 100000, 0)]
    elif pack == 'phase4-enchant':
        extras = [(s['c_weapon'], 1, 0), (s['c_weapon'], 1, 3)]
        extras += [(s[k], 2, 0) for k in ('scroll', 'blessed_scroll', 'safe_scroll')]
        # T52: the C mage sword at +3 (instance 12) is the deterministic failure target.
        extras.append((s['c_magic_weapon'], 1, 3))
        # 4-enchant-no-increment: B body armour at +20 (instance 13) and one bonus-100 safe armour
        # scroll (instance 14); D113 grants a +20 birth enchant (the ceiling is MAX_GRANT_ENCHANT
        # because base + bonus never reaches 0 for this scroll).
        extras += [(s['b_armor'], 1, 20), (s['no_increment_scroll'], 1, 0)]
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
        # D123/T24: weapon refine stays deferred (D82: no weapon life stone is admitted), so
        # 4-augment refines an ACCESSORY: the stable minimum admitted C-grade accessory and the
        # minimum admitted accessory life stone, paid with the C-grade accessory gemstones.
        accessory, stone, cost = accessory_refine(repo)
        extras += [(accessory, 1, 0), (stone['template_id'], 1, 0), (cost['gemstone_id'], cost['accessory_count'], 0), (57, 210000, 0)]
        owner = role(repo, level=stone['level'], position=(126, 128))
    elif pack == 'phase4-kinds':
        extras=[(s[k],1,0) for k in ('dual','dualfist','pole','dagger')]
        candidates=combat_skill_candidates(repo)
        if BLOW_SKILLS<={c['skill_id'] for c in candidates if c['admitted']}:
            # D50 promotion (E1 rev 8+): Mortal Blow 16 (FatalBlow) and Deadly Blow 263
            # (FatalBlow + Lethal) are executable, so the role is a level-40 Treasure Hunter
            # holding the highest source ranks its class chain allows at that level.
            owner=role(repo,profession='treasure_hunter',level=40)
            owner['learned_ranks']=blow_ranks(repo,('human_fighter','rogue','treasure_hunter'),40)
        else:owner=role(repo,profession='human_fighter',level=14)
        owner['combat_capability_dependencies']=candidates
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


ACCESSORY_BODYPARTS = ('neck', 'rear;lear', 'rfinger;lfinger')


def accessory_refine(repo=REPO):
    """(accessory template, life stone row, gemstone cost row) for the D123 accessory refine."""
    admitted_stones = {a['stone_id'] for a in table('capabilities', repo)['augmentation']
                       if a['admitted'] and a['kind'].startswith('accessory')}
    stones = [x for x in table('augmentation_chances', repo)['stones']
              if x['kind'] == 'accessory' and x['template_id'] in admitted_stones]
    if not stones:
        raise ValueError('no admitted accessory life stone')
    stone = min(stones, key=lambda x: (x['level'], x['template_id']))
    accessory = next(i for i, r in sorted(catalogue(repo).items()) if capabilities(repo)[i]['admitted']
                     and r['kind'] == 'Armor' and r['grade'] == 'C' and r['bodypart'] in ACCESSORY_BODYPARTS)
    cost = next(x for x in table('augmentation_chances', repo)['costs'] if x['grade'] == 'C')
    return accessory, stone, cost


def publish(repo=REPO):
    return dict(schema_version=1, policy='pre_admission_only', selector='stable_minimum_admitted_template',
                inputs_are_test_policy=True, selections=selection(repo),
                data_sha256=table('rules_manifest', repo)['data_sha256'],
                packs={p: profiles(p, repo, p in PAIRED) for p in sorted(PHASE4)})


def validate_seed_manifest(seed, selected, owner, observer, repo, paired, stage='granted'):
    """D112: `pre-admission` (characters + pending kit, before API startup) or `granted`
    (GrantItem stage after startup, admission confirmed from the offline baseline)."""
    expected = {name:r | {'account_id': owner if name == 'owner' else observer} for name,r in profiles(selected, repo, paired).items()}
    if (seed.get('schema_version') != 1 or seed.get('fixture') != selected or seed.get('roles') != expected
            or seed.get('dry_run') is not False or seed.get('stage') != stage
            or seed.get('admission_ready') is not (stage == 'granted')
            or any(plan.get('blockers') for plan in seed.get('grant_plan', {}).values())):
        raise ValueError('Phase4 seed manifest does not match the ' + stage + ' contract')
    if stage == 'granted' and set(seed.get('granted', {})) != set(expected):
        raise ValueError('Phase4 grant manifest lacks a role')


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


STATIC_NPC_PACKS = frozenset(('phase4-enchant',))


def zone_rng_word(position, zone=1, epoch=1, stream=0):
    """The zone's ChaCha12 output word at `position` (apps/api domain/zone/state.rs ZoneSeed/RngState).

    Key = b"nightfall.zone.rng.1" || zone id LE u32 || epoch LE u64; rand_chacha layout
    (constants, key, 64-bit block counter, 64-bit stream), 12 rounds, block = 16 words.
    `roll_below(n)` takes consecutive words, rejects a word >= 2^32 - 2^32 % n and returns
    word % n. Verified against the i3e exports: words 40..43 of zone 1 epoch 1 are
    47096 (mod 100000), 26339, 39447 and 89321.
    """
    import struct
    mask = 0xffffffff
    def rotl(v, c): return ((v << c) | (v >> (32 - c))) & mask
    def quarter(s, a, b, c, d):
        s[a] = (s[a] + s[b]) & mask; s[d] ^= s[a]; s[d] = rotl(s[d], 16)
        s[c] = (s[c] + s[d]) & mask; s[b] ^= s[c]; s[b] = rotl(s[b], 12)
        s[a] = (s[a] + s[b]) & mask; s[d] ^= s[a]; s[d] = rotl(s[d], 8)
        s[c] = (s[c] + s[d]) & mask; s[b] ^= s[c]; s[b] = rotl(s[b], 7)
    key = b'nightfall.zone.rng.1' + struct.pack('<I', zone) + struct.pack('<Q', epoch)
    init = list(struct.unpack('<4I', b'expand 32-byte k') + struct.unpack('<8I', key)
                + struct.unpack('<2I', struct.pack('<Q', position // 16)) + struct.unpack('<2I', struct.pack('<Q', stream)))
    state = init[:]
    for _ in range(6):
        quarter(state, 0, 4, 8, 12); quarter(state, 1, 5, 9, 13); quarter(state, 2, 6, 10, 14); quarter(state, 3, 7, 11, 15)
        quarter(state, 0, 5, 10, 15); quarter(state, 1, 6, 11, 12); quarter(state, 2, 7, 8, 13); quarter(state, 3, 4, 9, 14)
    return (state[position % 16] + init[position % 16]) & mask


def zone_roll(position, bound, **seed):
    """`roll_below(bound)` at a stream position: (value, words consumed); None value = rejected word."""
    word = zone_rng_word(position, **seed)
    limit = (1 << 32) - ((1 << 32) % bound)
    return (word % bound if word < limit else None), 1


def pin_rng_cursor(zone):
    """T52: no NPC walks, so the zone RNG cursor at the first player intent is fixed.

    The zone's ChaCha12 stream is keyed by (zone 1, epoch 1) and every fresh fixture stack
    starts at word 0. The ten NPC spawns draw entity ids (4 words each, words 0..39); an idle
    NPC with a positive speed also rolls `npc_walk` on its own cadence, which moved the enchant
    draw of 4-enchant-blessed from word 40 to word 41 in i3e. Every spawn slot becomes a static
    zone NPC (speed 0: the walk roll is skipped before any draw) and the Wanderer stands still,
    so the entity count (10) and therefore the enchant draw (word 40, value 47096) are fixed.
    """
    text = zone.read_text()
    text = text.replace('name = "Wanderer"\nattackable = false\npos = [32, 32]\nspeed = 500', 'name = "Wanderer"\nattackable = false\npos = [32, 32]\nspeed = 0')
    slots = re.findall(r'\[\[spawn_slots\]\]\nid = "([^"]+)"\ntemplate = "[^"]+"\nhome = \[(\d+), (\d+)\]\ncount = 1\n(?:respawn_[a-z_]+ = \d+\n)*', text)
    text = re.sub(r'\[\[spawn_slots\]\]\nid = "[^"]+"\ntemplate = "[^"]+"\nhome = \[\d+, \d+\]\ncount = 1\n(?:respawn_[a-z_]+ = \d+\n)*', '', text)
    for name, x, y in slots:
        text += f'\n[[npcs]]\nname = "{name}"\nattackable = false\npos = [{x}, {y}]\nspeed = 0\n'
    zone.write_text(text)


def create_overlay(destination, repo, pack=None):
    phase3.create_overlay(destination, repo)
    zone = destination / 'zones/test_zone.toml'
    blacksmith = table('capacities', repo)['blacksmith']
    # Stationary fixture targets preserve initial east facing (design §2.1/§6.2).
    target=destination/'npcs/skill_target.toml'
    # The API rejects move_speed 0 (npc_data.rs: must be positive), so the slowest legal speed
    # is 1 milli-tile/tick. V6 facing is set at spawn and not updated by movement, so side/back
    # phases are unaffected; the target can still creep 1 milli-tile per tick while chasing an
    # attacker beyond its 1.5-tile attack range (bow scenarios), i.e. it is not strictly fixed.
    target.write_text(target.read_text().replace('move_speed = 400','move_speed = 1'))
    # Eligible passive secondary recipients are fixture inputs, not source stats.
    for name,position in [('pole_secondary_a',(108,101)),('pole_secondary_b',(108,99))]:
        template=(destination/'npcs/skill_target.toml').read_text().replace('skill_target',name)
        (destination/('npcs/'+name+'.toml')).write_text(template)
        with zone.open('a') as out:
            out.write(f'\n[[spawn_slots]]\nid = "{name}"\ntemplate = "{name}"\nhome = [{position[0]}, {position[1]}]\ncount = 1\n')
    zone.write_text(zone.read_text() + '\n[[npcs]]\nname = ' + json.dumps(blacksmith['template']) + '\nattackable = false\npos = [' + str(blacksmith['x']) + ', ' + str(blacksmith['y']) + ']\nspeed = 0\n')
    if pack in STATIC_NPC_PACKS:
        pin_rng_cursor(zone)
