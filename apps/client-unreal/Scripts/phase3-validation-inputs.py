#!/usr/bin/env python3
"""Generate inputs for the API bootstrap/admission regression; no DB or API."""
import importlib.util
import json
from pathlib import Path
import sys

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parents[2]
spec = importlib.util.spec_from_file_location('packs', SCRIPTS / 'phase3-fixtures.py')
packs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packs)


def generate(destination):
    packs.create_overlay(destination / 'data', REPO)
    for fixture in ('phase1a-late-entry', 'phase1a-social-aggro'):
        packs.create_phase1a_overlay(destination / fixture, REPO, fixture)
    profiles = {pack: packs.profiles(pack, REPO, True) for pack in sorted(packs.PHASE3)}
    profiles['casting-soak'] = {'owner': packs.soak_profile(REPO, packs.OWNER, packs.OBSERVER, 0, 8)}
    (destination / 'packs.json').write_text(json.dumps(profiles, sort_keys=True))
    token_spec = importlib.util.spec_from_file_location('tokens', SCRIPTS / 'token-fixtures.py')
    tokens = importlib.util.module_from_spec(token_spec)
    token_spec.loader.exec_module(tokens)
    token_roles = {}
    for level in (20, 40):
        pack = f'phase2-token-level{level}'
        for class_id in (0, 1 if level == 20 else 2):
            token_roles[f'{level}-{class_id}'] = packs.profiles('phase3-trainer', REPO)['owner'] | {
                'class_profile': 'human_fighter', 'class_id': class_id, 'base_class_id': 0,
                'level': level, 'xp': tokens.PINNED_ORACLES[pack]['oracle']['first_kill_xp'],
                'position': [110, 100], 'learned_ranks': {}, 'hp': None, 'mp': None, 'cp': 0}
    (destination / 'token-v5-roles.json').write_text(json.dumps(token_roles, sort_keys=True))
    for pack in sorted(tokens.TOKEN_CROSSING):
        root = destination / pack
        tokens.create_overlay(root, pack, REPO)
        packs.upgrade_npc_overlays(root, REPO)


if __name__ == '__main__':
    generate(Path(sys.argv[1]))
