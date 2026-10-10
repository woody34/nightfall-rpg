#!/usr/bin/env python3
"""Seed generated soak casting roles before API startup in a fresh isolated DB."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
from uuid import UUID

spec = importlib.util.spec_from_file_location('phase3_seed', Path(__file__).with_name('seed-phase3-fixture.py'))
seed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(seed)


def validated_roles(manifest):
    if (manifest.get('schema_version') != 1 or manifest.get('casting') is not True
            or not isinstance(manifest.get('roles'), list) or not 1 <= len(manifest['roles']) <= 8):
        raise ValueError('soak requires 1..8 declared casting roles')
    result = {}
    accounts, characters = set(), set()
    for index, role in enumerate(manifest['roles'], 1):
        name = f'client-{index:02}'
        supplied = role['seed']
        account, character = str(UUID(supplied['account_id'])), str(UUID(supplied['character_id']))
        expected = seed.contracts.soak_profile(seed.REPO, account, character)
        if role['name'] != name or supplied != expected or account in accounts or character in characters:
            raise ValueError('soak seed roles must match canonical inputs and use distinct identities')
        result[f'role{index:02}'] = expected
        accounts.add(account)
        characters.add(character)
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest', required=True, type=Path)
    p.add_argument('--dry-run', action='store_true')
    args = p.parse_args()
    try:
        if os.environ.get('AUTH_DEV_TOKENS') != '1' or os.environ.get('NIGHTFALL_PHASE3_FIXTURE') != '1':
            raise ValueError('explicit dev-token and Phase3 fixture opt-in required')
        seed.guarded_database(os.environ.get('DATABASE_URL', ''))
        roles = validated_roles(json.loads(args.manifest.read_text()))
    except (ValueError, KeyError, TypeError) as error:
        p.error(str(error))
    sql = seed.seed_sql(roles)
    if args.dry_run:
        print(sql)
    else:
        env = seed.psql_env(os.environ['DATABASE_URL'])
        subprocess.run(['psql', '-X', '--set=ON_ERROR_STOP=1', '--quiet'], input=sql, text=True, env=env, check=True)
    print(json.dumps({'schema_version': 1, 'schema_status': 'reconciled-000008-0a37fb3', 'roles': roles,
                      'fixture': 'phase3-soak', 'fixture_enabled': True, 'dry_run': args.dry_run}, sort_keys=True))


if __name__ == '__main__':
    main()
