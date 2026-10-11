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
    profiles = {pack: packs.profiles(pack, REPO, True) for pack in sorted(packs.PHASE3)}
    profiles['casting-soak'] = {'owner': packs.soak_profile(REPO, packs.OWNER, packs.OBSERVER)}
    (destination / 'packs.json').write_text(json.dumps(profiles, sort_keys=True))
    token_spec = importlib.util.spec_from_file_location('tokens', SCRIPTS / 'token-fixtures.py')
    tokens = importlib.util.module_from_spec(token_spec)
    token_spec.loader.exec_module(tokens)
    for pack in sorted(tokens.TOKEN_CROSSING):
        root = destination / pack
        tokens.create_overlay(root, pack, REPO)
        packs.upgrade_npc_overlays(root, REPO)


if __name__ == '__main__':
    generate(Path(sys.argv[1]))
