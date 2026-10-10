#!/usr/bin/env python3
"""Phase3 pre-admission seed contracts; fixtures specify inputs, never gameplay math.

Migration 000008 reconciled to C-API commit 0a37fb3. Fresh timer/effect child
rows stay empty; no typed retained payload or death record is fabricated.
"""
import hashlib
import json
import re
from pathlib import Path
import shutil
import tomllib

PHASE3 = frozenset(('phase3-trainer', 'phase3-pvp', 'phase3-resurrect', 'phase3-regen'))
OWNER = '01970000-0000-7000-8000-000000000030'
OBSERVER = '01970000-0000-7000-8000-000000000031'
PAIRED = frozenset(('phase3-pvp', 'phase3-resurrect'))


def declared_units(directory):
    path = directory / 'phase3-roles.json'
    if not path.exists():
        return {}
    data = json.loads(path.read_text())
    if data.get('schema_version') != 1 or not isinstance(data.get('units'), dict):
        raise ValueError('invalid Phase3 role manifest')
    seen = set()
    for name, roles in data['units'].items():
        if (not isinstance(roles, list) or len(roles) != 2 or roles[0] != name + '.nfs'
                or any(not isinstance(role, str) or Path(role).name != role or not role.endswith('.nfs')
                       or not (directory / role).is_file() for role in roles)
                or any(role in seen for role in roles) or len(set(roles)) != 2):
            raise ValueError('invalid/missing/duplicate Phase3 scenario role')
        seen.update(roles)
    return data['units']


def declared_pair(batch):
    if len(batch) != 2 or batch[0].parent.resolve() != batch[1].parent.resolve():
        return False
    names = {path.name for path in batch}
    return any(set(roles) == names for roles in declared_units(batch[0].parent).values())


def observer_role(path):
    if path.stem.endswith('-b'):
        return True
    return any(path.name == roles[1] for roles in declared_units(path.parent).values())


def profiles(pack, repo, paired=False):
    if pack not in PHASE3:
        raise ValueError('unknown Phase3 pack')
    if pack in PAIRED and not paired:
        raise ValueError('Phase3 pack requires cooperating roles')
    def role(character, profession, level, sp, pos, learned, hp=None, mp=None, cp=0):
        definition = tomllib.loads((repo / f'packages/data/professions/{profession}.toml').read_text())
        xp = next(r['xp'] for r in tomllib.loads((repo / 'packages/data/tables/experience.toml').read_text())['to_level'] if r['level'] == level)
        return dict(character_id=character, profession=profession, class_id=definition['id'],
                    base_class_id=definition['base_class_id'], class_profile='human_mystic', race=definition['race'],
                    base_stats=definition['base_stats'], level=level, xp=xp, sp=sp,
                    position=pos, learned_ranks={str(k): v for k, v in learned.items()},
                    hp=hp, mp=mp, cp=cp, alive=True, karma=0, pk_count=0, pvp_count=0,
                    flag_remaining_ticks=0, death_penalty_level=0, recoverable_death_xp_loss=0, phase3_timer_policy_version=1,
                    death_incarnation=None, pre_death_xp=None, hp_fraction_q=0, mp_fraction_q=0,
                    cp_fraction_q=0, modifier_install_order=[], reuse_remainders=[], effect_remainders=[])
    # Rank2 of Wind Strike is deliberately unlearned for the first learn→cast.
    # Its cost240/required level7 come from professions/human_mystic.toml skill_tree.
    owner = role(OWNER, 'human_mystic', 14, 10000, [126, 128],
                 {1177: 1, 1184: 1, 1068: 1, 1040: 1, 1164: 1})
    if pack == 'phase3-pvp':
        owner = role(OWNER, 'human_mystic', 14, 10000, [110, 100], {1177: 5, 1184: 4, 1068: 1})
    elif pack == 'phase3-resurrect':
        owner = role(OWNER, 'cleric', 30, 10000, [110, 100], {1016: 1, 1069: 1, 1068: 1, 1216: 1})
    elif pack == 'phase3-regen':
        owner = role(OWNER, 'human_mystic', 14, 0, [126, 128], {1177: 1}, hp=1, mp=1, cp=1)
    result = {'owner': owner}
    if paired:
        result['observer'] = role(OBSERVER, 'human_mystic', 14, 0, [112, 100], {1177: 1}, hp=100, cp=10)
    return result


def validate_seed_manifest(seed, selected, owner, observer, repo, paired):
    expected = profiles(selected, repo, paired)
    expected = {name: value | {'account_id': owner if name == 'owner' else observer} for name, value in expected.items()}
    if (seed.get('schema_version') != 1 or seed.get('fixture_enabled') is not True
            or seed.get('dry_run') is not False or seed.get('fixture') != selected
            or seed.get('roles') != expected or seed.get('schema_status') != 'reconciled-000008-0a37fb3'):
        raise ValueError('Phase3 seed manifest does not match exact pre-admission inputs')


def soak_profile(repo, account, character):
    # Level7 is a fixture input with both rank1 skills available in the shipped tree.
    # The level1 reward NPC remains the shipped template, with no XP/SP reward override.
    xp = next(row['xp'] for row in tomllib.loads(
        (repo / 'packages/data/tables/experience.toml').read_text())['to_level'] if row['level'] == 7)
    return profiles('phase3-trainer', repo)['owner'] | {
        'account_id': account, 'character_id': character, 'position': [0, 0],
        'level': 7, 'xp': xp, 'learned_ranks': {'1177': 1, '1068': 1}}


def tree_hashes(directory):
    return {str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(directory.rglob('*')) if path.is_file()}


def data_hashes(repo):
    return tree_hashes(repo / 'packages/data')


def create_overlay(destination, repo):
    """Copy shipped rules only; no synthesized skill powers, costs, or durations."""
    shutil.copytree(repo / 'packages/data', destination)
    # Reuse the shipped NPC; place fixtures outside the architect's peace square.
    zone = destination / 'zones/test_zone.toml'
    text = zone.read_text()
    text = re.sub(r'\[\[spawn_slots\]\]\nid = "keltir_b".*?(?=\[\[|\Z)', '', text, flags=re.S)
    text = text.replace('count = 2', 'count = 1')
    text = text.replace('home = [100, 100]', 'home = [110, 104]').replace('home = [104, 102]', 'home = [114, 104]')
    # Fixture-only numeric inputs, inherited from the audited Phase2 token pack.
    # The durable target survives casts; the sentinel kills through normal combat.
    spec = __import__('importlib.util', fromlist=['util']).spec_from_file_location('token_overlay', repo / 'apps/client-unreal/Scripts/token-fixtures.py')
    token = __import__('importlib.util', fromlist=['util']).module_from_spec(spec)
    spec.loader.exec_module(token)
    sentinel = token.sentinel_template_content().replace('token_sentinel', 'skill_sentinel')
    (destination / 'npcs/skill_sentinel.toml').write_text(sentinel)
    dummy = token.oracle_template_content(0).replace('token_oracle', 'skill_target').replace('max_hp = 1\n', 'max_hp = 1000000\n')
    (destination / 'npcs/skill_target.toml').write_text(dummy)
    text += '\n[[spawn_slots]]\nid = "skill_target"\ntemplate = "skill_target"\nhome = [108, 100]\ncount = 1\n'
    text += '\n[[spawn_slots]]\nid = "skill_sentinel"\ntemplate = "skill_sentinel"\nhome = [110, 106]\ncount = 1\n'
    # Preserve reward/template values; passive fixture isolates deliberate combat.
    npc = destination / 'npcs/keltir.toml'
    npc.write_text(npc.read_text().replace('aggressive = true', 'aggressive = false').replace('clan_help_range = 8000', 'clan_help_range = 0'))
    zone.write_text(text)
