#!/usr/bin/env python3
"""Seed generated soak casting roles before API startup in a fresh isolated DB."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
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
        expected = seed.contracts.soak_profile(seed.REPO, account, character, index - 1, len(manifest['roles']))
        if role['name'] != name or supplied != expected or account in accounts or character in characters:
            raise ValueError('soak seed roles must match canonical inputs and use distinct identities')
        # Role keys become the seeded character name (`Skill<key>`), which must be ASCII
        # letters only (domain CharacterName); spell the index out as a letter.
        result[f'role{chr(ord("A") + index - 1)}'] = expected
        accounts.add(account)
        characters.add(character)
    return result


def seed_container(sql, project, compose_file):
    if not project or not re.fullmatch(r'nightfall-phase3-harness-soak-[0-9]+-[0-9]+', project):
        raise ValueError('soak seed requires its owned Compose project')
    if compose_file is None:
        raise ValueError('soak seed requires its resolved Compose file')
    compose_file = compose_file.resolve()
    postgres = json.loads(compose_file.read_text())['services']['postgres']
    env = seed.psql_env(os.environ['DATABASE_URL'])
    if any(any(ord(char) < 32 or ord(char) == 127 for char in env[key])
           for key in ('PGHOST', 'PGPORT', 'PGDATABASE', 'PGUSER', 'PGPASSWORD')):
        raise ValueError('soak seed refused control characters in connection components')
    connection = postgres['environment']
    if (connection['POSTGRES_DB'] != env['PGDATABASE']
            or connection['POSTGRES_USER'] != env['PGUSER']
            or connection['POSTGRES_PASSWORD'] != env['PGPASSWORD']
            or not any(str(port['target']) == '5432' and str(port['published']) == env['PGPORT']
                       for port in postgres['ports'])):
        raise ValueError('soak seed connection must match its owned Compose postgres')
    # Match phase2-fixture.py: exec psql inside the owned service, with split libpq inputs.
    env.update(PGHOST='127.0.0.1', PGPORT='5432')
    command = ['docker', 'compose', '--env-file', str(compose_file.parent / 'empty.env'),
               '-p', project, '-f', str(compose_file), 'exec', '-T']
    for key in ('PGHOST', 'PGPORT', 'PGDATABASE', 'PGUSER', 'PGPASSWORD', 'PGCONNECT_TIMEOUT', 'PGAPPNAME'):
        command += ['--env', key]
    command += ['postgres', 'psql', '-X', '--set=ON_ERROR_STOP=1', '--quiet']
    subprocess.run(command, cwd=compose_file.parent, input=sql, text=True, env=env, check=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest', required=True, type=Path)
    p.add_argument('--dry-run', action='store_true')
    p.add_argument('--compose-project')
    p.add_argument('--compose-file', type=Path)
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
        try:
            seed_container(sql, args.compose_project, args.compose_file)
        except (ValueError, KeyError, TypeError) as error:
            p.error(str(error))
    print(json.dumps({'schema_version': 1, 'schema_status': 'reconciled-000008-0a37fb3', 'roles': roles,
                      'fixture': 'phase3-soak', 'fixture_enabled': True, 'dry_run': args.dry_run}, sort_keys=True))


if __name__ == '__main__':
    main()
