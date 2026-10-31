#!/usr/bin/env python3
"""Phase 5 harness fixture contract (story 6.1): story 1.8's pack declarations
(Scenarios/fixtures/phase5-packs.json, oracle-relevant inputs only) become seedable role profiles,
a zone overlay and per-role scenario copies. No gameplay result is computed here.

One Phase 5 scenario file is ONE unit (P5-D5): one owned stack, one client per `# role:` block,
one scheduling atom. Roles a/b/c map to accounts a/b/c (a role declared `same_account_as` shares
that role's account).

    python3 apps/client-unreal/Scripts/phase5-fixtures.py --publish Scenarios/phase5-harness.json
"""
import argparse
from functools import lru_cache
import importlib.util
import json
from pathlib import Path
import re
import tomllib

REPO = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


phase3 = _load('phase5_phase3', 'phase3-fixtures.py')
phase4 = _load('phase5_phase4', 'phase4-fixtures.py')
grammar = _load('phase5_grammar', 'phase5-grammar.py')

PACKS_PATH = REPO / 'apps/client-unreal/Scenarios/fixtures/phase5-packs.json'
PACKS = json.loads(PACKS_PATH.read_text())['packs']
PHASE5 = frozenset(PACKS)
ROLE_ORDER = ('a', 'b', 'c')
ROLE_INDEX = {'a': 0, 'b': 1, 'c': 2}
# Skills the Phase 5 engine admits through the cast pipeline (B11) that no Phase 3 class tree
# carries: seeded as learned ranks so the role can cast them. Z's E2/E3 skill admission binds them.
SPOIL, SWEEPER, CREATE_ITEM, CREATE_COMMON_ITEM = 254, 42, 172, 1320
DWARVEN_CRAFT_CLASSES = {54, 55, 56, 57}
# Fixed noncombat fixture NPCs by pack kind: zone [[npcs]] names the bot resolves (phase4 blacksmith pattern).
FIXED_NPC_KINDS = ('merchant', 'multisell', 'warehouse_keeper', 'manor_manager', 'auctioneer')
MISSING_FROM_PHASE4_CATALOGUE = ('template is in neither the Phase 4 item catalogue nor the Phase 5 catalogue closure '
                                 '(tables-phase5/catalogue.toml, storable); V7 admission refuses it (P5-D7)')


@lru_cache(maxsize=None)
def professions(repo=REPO):
    rows = {}
    for path in sorted((repo / 'packages/data/professions').glob('*.toml')):
        definition = tomllib.loads(path.read_text())
        rows[definition['id']] = definition['key']
    return rows


def profession_for(class_id, repo=REPO):
    table = professions(repo)
    if class_id not in table:
        raise ValueError(f'unknown class id {class_id}')
    return table[class_id]


def role_profile(pack, declaration, repo=REPO):
    """phase4.role(...) plus the pack's inventory (kit + declared rows), ranks and chaotic state."""
    profession = profession_for(declaration['class_id'], repo)
    base = phase4.role(repo, character=declaration['character_id'], profession=profession, level=declaration['level'],
                       position=tuple(declaration['position']))
    extras = [(row['template_id'], row['count'], row['enchant']) for row in declaration['inventory']]
    index = ROLE_INDEX[declaration['role']]
    inventory = []
    for row in phase4.inventory(repo, base['base_class_id'], index, ()):  # the starter kit the API materializes
        inventory.append(row)
    stride = index * phase4.ROLE_STRIDE
    catalogue = phase4.catalogue(repo)
    for template, count, enchant in extras:
        definition = catalogue.get(template)
        stackable = definition['stackable'] if definition else phase4_free_stackable(template, repo)
        previous = next((r for r in inventory if r['template_id'] == template and stackable and r['location'] == 'inventory'), None)
        if previous and stackable:
            previous['count'] += count
            continue
        inventory.append(dict(instance_id=stride + len(inventory) + 1, template_id=template, count=count, enchant=enchant,
                              augment_id=None, charge_bits=0, location='inventory', slot=None, revision=0,
                              source='fixtures/phase5-packs.json (oracle-relevant input, story 1.8)'))
    learned = dict(base['learned_ranks'])
    if declaration['class_id'] in DWARVEN_CRAFT_CLASSES:
        learned[str(SPOIL)] = 1
        learned[str(SWEEPER)] = 1
    if declaration.get('create_item_rank'):
        learned[str(CREATE_ITEM)] = declaration['create_item_rank']
    if declaration.get('create_common_item_rank'):
        learned[str(CREATE_COMMON_ITEM)] = declaration['create_common_item_rank']
    profile = base | dict(role=declaration['role'], inventory=inventory, learned_ranks=learned,
                          karma=declaration.get('karma', 0), pk_count=declaration.get('pk_count', 0),
                          same_account_as=declaration.get('same_account_as'), peace_zone=declaration.get('peace_zone', False),
                          registered_recipes=declaration.get('registered_recipes', {}), pack=pack)
    # The seeder owns these; a profile never carries secrets.
    profile.pop('account_id', None)
    return profile


@lru_cache(maxsize=None)
def _items_xml(repo=REPO):
    import xml.etree.ElementTree as ET
    root = Path.home() / '.cache/nightfall/l2j/l2j-dp/src/main/resources/data/stats/items'
    out = {}
    for path in sorted(root.glob('*.xml')):
        for item in ET.parse(path).getroot().findall('item'):
            sets = {s.get('name'): s.get('val') for s in item.findall('set')}
            out[int(item.get('id'))] = sets.get('is_stackable') == 'true'
    return out


def phase4_free_stackable(template, repo=REPO):
    """Stackability of a template the Phase 4 catalogue lacks: Phase 5 catalogue.toml carries no
    stackable flag, so read the pinned items XML directly (same source gen_items.py uses)."""
    table = _items_xml(repo)
    if template not in table:
        raise ValueError(f'template {template} is not in the pinned items XML')
    return table[template]


def profiles(pack, repo=REPO):
    if pack not in PHASE5:
        raise ValueError('unknown Phase5 pack')
    return {decl['role']: role_profile(pack, decl, repo) for decl in PACKS[pack]['roles']}


def accounts_for(pack, accounts):
    """Role -> account: `same_account_as` roles share; every other role needs its own account."""
    out = {}
    for decl in PACKS[pack]['roles']:
        source = decl.get('same_account_as') or decl['role']
        if source not in accounts:
            raise ValueError(f'pack {pack} role {decl["role"]} needs account {source}')
        out[decl['role']] = accounts[source]
    distinct = {r: a for r, a in out.items() if not next(d for d in PACKS[pack]['roles'] if d['role'] == r).get('same_account_as')}
    if len(set(distinct.values())) != len(distinct):
        raise ValueError('distinct account roles required')
    return out


@lru_cache(maxsize=None)
def phase5_closure(repo=REPO):
    """P5-D7: under V7 the admitted item set is the Phase 4 catalogue plus the Phase 5 catalogue closure
    (tables-phase5/catalogue.toml, storable templates; hashed into Phase5Rules)."""
    table = tomllib.loads((repo / 'packages/data/tables-phase5/catalogue.toml').read_text())
    return frozenset(t['id'] for t in table['templates'] if t['storable'])


def admitted(template, repo=REPO):
    return template in phase4.catalogue(repo) or template in phase5_closure(repo)


def catalogue_blockers(profile, repo=REPO):
    return [dict(instance_id=row['instance_id'], template_id=row['template_id'], reason=MISSING_FROM_PHASE4_CATALOGUE)
            for row in profile['inventory'] if not admitted(row['template_id'], repo)]


def npc_template_text(repo, npc_id, level):
    """A monster template for a pinned NPC id: the shipped keltir body (combat numbers and the
    validated Phase 3 suffix stay) with the id, name and level of the Phase 5 NPC. The drop-list
    binding of `npc_<id>` to drops.toml is the engine's (E3); the overlay records the pinned id."""
    text = (repo / 'packages/data/npcs/keltir.toml').read_text()
    text = re.sub(r'^id = "keltir"$', f'id = "npc_{npc_id}"', text, count=1, flags=re.M)
    text = re.sub(r'^name = "Keltir"$', f'name = "Npc_{npc_id}"', text, count=1, flags=re.M)  # one token: the bot splits on whitespace (BotPredicates.cpp:566-571)
    text = re.sub(r'^level = 1$', f'level = {level}', text, count=1, flags=re.M)
    header = (f'# Phase 5 fixture monster for pinned NPC {npc_id} (tables-phase5/drops.toml npc[id]); body from keltir.toml.\n'
              f'# The economy sidecar binds the drop lists by pinned id (design §1.3); combat numbers are the keltir starter values.\n')
    return header + text


def create_overlay(destination, repo, pack):
    """Phase 4 overlay (blacksmith, pinned RNG cursor policy) plus the pack's fixed NPCs and monsters."""
    if pack not in PHASE5:
        raise ValueError('unknown Phase5 pack')
    phase4.create_overlay(destination, repo, None)
    zone = destination / 'zones/test_zone.toml'
    text = zone.read_text()
    declared = PACKS[pack]
    for npc in declared.get('fixed_npcs', []):
        if npc['kind'] not in FIXED_NPC_KINDS:
            raise ValueError(f'unknown fixed NPC kind {npc["kind"]}')
        text += f'\n[[npcs]]\nname = "{npc["name"]}"\nattackable = false\npos = [{npc["position"][0]}, {npc["position"][1]}]\nspeed = 0\n'
    for monster in declared.get('monsters', []):
        template = f'npc_{monster["npc_id"]}'
        path = destination / 'npcs' / f'{template}.toml'
        if not path.exists():
            path.write_text(npc_template_text(repo, monster['npc_id'], monster['level']))
        text += (f'\n[[spawn_slots]]\nid = "{monster["slot_id"]}"\ntemplate = "{template}"\nhome = [{monster["home"][0]}, {monster["home"][1]}]\n'
                 f'count = {monster["count"]}\n')
    zone.write_text(text)
    manifest = dict(pack=pack, fixed_npcs=[n['name'] for n in declared.get('fixed_npcs', [])],
                    monsters=[dict(slot=m['slot_id'], template=f'npc_{m["npc_id"]}', pinned_npc_id=m['npc_id']) for m in declared.get('monsters', [])],
                    rate_multiplier_g=declared.get('rate_multiplier_g', '1'))
    (destination / 'phase5-overlay.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    return manifest


def unit_stem(path):
    """`5-trade.role-b.nfs` -> `5-trade`; a plain file is its own stem."""
    return re.sub(r'\.role-[abc]$', '', Path(path).stem)


def role_of(path):
    match = re.search(r'\.role-([abc])$', Path(path).stem)
    return match[1] if match else 'a'


def split_unit(scenario, folder):
    """Write one scenario copy per `# role:` block under folder/roles; returns the paths in role order."""
    text = Path(scenario).read_text()
    parts = grammar.split_roles(text)
    out = []
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    for role in ROLE_ORDER:
        if role not in parts:
            continue
        target = folder / f'{Path(scenario).stem}.role-{role}.nfs'
        target.write_text('\n'.join(parts[role]) + '\n')
        allow = Path(scenario).with_suffix('.allow')
        if allow.is_file():
            target.with_suffix('.allow').write_bytes(allow.read_bytes())
        out.append(target)
    return out


def declared_roles(pack, batch):
    """Roles of a provisioned batch (role copies of ONE unit) must equal the pack's roles."""
    stems = {unit_stem(p) for p in batch}
    if len(stems) != 1:
        raise ValueError('a Phase5 unit is one scenario file (role copies of the same file)')
    roles = [role_of(p) for p in batch]
    if len(roles) != len(set(roles)):
        raise ValueError('duplicate Phase5 role copy')
    expected = [d['role'] for d in PACKS[pack]['roles']]
    if sorted(roles) != sorted(expected) and len(batch) > 1:
        raise ValueError(f'Phase5 unit roles {sorted(roles)} differ from the pack roles {sorted(expected)}')
    return roles


def validate_seed_manifest(seed, pack, accounts, stage='pre-admission'):
    expected = {name: r | {'account_id': accounts[name]} for name, r in profiles(pack).items()}
    if (seed.get('schema_version') != 1 or seed.get('fixture') != pack or seed.get('roles') != expected
            or seed.get('dry_run') is not False or seed.get('stage') != stage
            or seed.get('admission_ready') is not (stage == 'granted')):
        raise ValueError('Phase5 seed manifest does not match the ' + stage + ' contract')
    if stage == 'granted' and set(seed.get('granted', {})) != set(expected):
        raise ValueError('Phase5 grant manifest lacks a role')


def publish(repo=REPO):
    return dict(schema_version=1, story='5 E6.1', policy='pre_admission_only', unit='one scenario file per unit; one client per # role: block (P5-D5)',
                tick_fast_forward_facility='none in the API or harness at 5abae32 (only ZONE_SIM_FIXTURE); 5-vendor-restock live refill is soak-only (E43)',
                data_sha256=phase3.data_hashes(repo),
                packs={p: dict(roles=profiles(p, repo), blockers={r: catalogue_blockers(prof, repo) for r, prof in profiles(p, repo).items()})
                       for p in sorted(PHASE5)})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--publish', type=Path, required=True)
    args = parser.parse_args()
    args.publish.write_text(json.dumps(publish(), indent=2, sort_keys=True) + '\n')
