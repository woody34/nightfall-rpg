#!/usr/bin/env python3
"""Seed a fresh isolated Phase3 DB BEFORE API startup. No admitted mutation path.

Schema reconciled to API migration 000008 commit 0a37fb3; runtime pending.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib
from urllib.parse import urlparse
from uuid import NAMESPACE_URL, UUID, uuid5

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('phase3_fixtures', REPO / 'apps/client-unreal/Scripts/phase3-fixtures.py')
contracts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contracts)


def guarded_database(url):
    parsed = urlparse(url)
    if (parsed.scheme not in ('postgres', 'postgresql') or parsed.hostname not in ('127.0.0.1', 'localhost')
            or parsed.port in (None, 5432) or not re.fullmatch(r'/nf_phase3_fixture_[a-z0-9_]+', parsed.path)
            or parsed.query or parsed.fragment):
        raise ValueError('explicit loopback non-5432 port and nf_phase3_fixture_* DB required; no URL options')


def psql_env(database_url, shim=False):
    from urllib.parse import unquote
    url = urlparse(database_url)
    # PGHOSTADDR/PGSERVICE/PGOPTIONS could override the guarded endpoint or SQL
    # session. Supply libpq inputs only from the validated URI, including standalone runs.
    env = {key: value for key, value in os.environ.items() if not key.startswith('PG')}
    env.update(PGCONNECT_TIMEOUT='3', PGAPPNAME='phase3_fixture_seed')
    if shim:
        env['PGDATABASE'] = database_url
    else:
        env.update(PGHOST=url.hostname, PGPORT=str(url.port), PGDATABASE=url.path[1:],
                   PGUSER=unquote(url.username or ''), PGPASSWORD=unquote(url.password or ''))
    return env


def professions(repo=REPO):
    return {row['id']: row for row in (tomllib.loads(path.read_text())
            for path in sorted((repo / 'packages/data/professions').glob('*.toml')))}


def transfer_receipts(name, role, repo=REPO):
    """D124/T25: a class-transferred seeded owner needs ONE change_class receipt per tier
    (character_progression.rs validate_for_skill_rules): parent -> child along
    professions/*.toml, each a frozen transfer result at a level its target allows, with the
    owner's identity and its BASE class stats; the last target is the current class. Mirrors the
    API test phase4_login_runtime::postgres_seeded_class_transferred_owner_needs_its_receipt_history."""
    if role['class_id'] == role['base_class_id']:
        return []
    table = professions(repo)
    chain, current = [], table[role['class_id']]
    while current['id'] != role['base_class_id']:
        chain.append(current)
        if current.get('parent') is None:
            raise ValueError(f"class {role['class_id']} does not descend from base {role['base_class_id']}")
        current = table[current['parent']]
    chain.reverse()
    if len(chain) != table[role['class_id']]['tier'] or not 1 <= len(chain) <= 2:
        raise ValueError(f"class {role['class_id']} needs {len(chain)} receipts; seeded transfers cover tiers 1..2")
    receipts = []
    for ordinal, target in enumerate(chain):
        if not target['min_level'] <= role['level'] <= 85:
            raise ValueError(f"seeded level {role['level']} below class {target['id']} minimum {target['min_level']}")
        key = str(uuid5(NAMESPACE_URL, f"nightfall:seed-transfer:{role['character_id']}:{ordinal}"))
        response = {
            'character_id': role['character_id'],
            'identity': {'account_id': role['account_id'], 'race': role['race'], 'base_class_id': role['base_class_id'],
                         'appearance': {'sex': 'Female', 'hair_style': 0, 'hair_color': 0, 'face': 0}},
            'name': f'Skill{name}', 'current_class_id': target['id'], 'level': target['min_level'],
            'xp': 0, 'sp': 0, 'stats': dict(role['base_stats']), 'position_millitiles': [0, 0],
            'hp': 1, 'mp': 1, 'cp': 0, 'max_hp': 1, 'max_mp': 1, 'max_cp': 0,
            'token_tier_1_count': 0, 'token_tier_2_count': 0, 'granted_skill_keys': []}
        receipts.append(dict(ordinal=ordinal, key=key, target_class_id=target['id'],
                             fingerprint=f"v1|{role['character_id']}|{target['id']}", response=response))
    return receipts


def transfer_sql(name, role, repo=REPO):
    sql = ''
    for r in transfer_receipts(name, role, repo):
        response = json.dumps(r['response'], separators=(',', ':'), sort_keys=True).replace("'", "''")
        sql += (f"INSERT INTO idempotency_keys(key,fingerprint,account_id,operation,response) VALUES "
                f"('{r['key']}','{r['fingerprint']}','{role['account_id']}','change_class','{response}'::jsonb);\n"
                f"INSERT INTO character_transfer_receipts(character_id,ordinal,account_id,key,target_class_id) VALUES "
                f"('{role['character_id']}',{r['ordinal']},'{role['account_id']}','{r['key']}',{r['target_class_id']});\n")
    return sql


def seed_sql(roles):
    rows, skills = [], []
    for name, role in roles.items():
        s = role['base_stats']
        installation = json.dumps(role['modifier_install_order'], separators=(',', ':'))
        number = lambda value: 'NULL' if value is None else str(value)
        rows.append(f"('{role['character_id']}','{role['account_id']}','Skill{name}','skill{name.lower()}','{role['race']}',"
                    f"{role['level']},{s['str']},{s['dex']},{s['con']},{s['int']},{s['wit']},{s['men']},"
                    f"{role['position'][0]},{role['position'][1]},{role['xp']},{number(role['hp'])},{number(role['mp'])},"
                    f"{number(role['cp'])},'{role['class_profile']}',true,0,{role['base_class_id']},{role['class_id']},"
                    f"{role['sp']},'female',{role['karma']},{role['pk_count']},{role['pvp_count']},"
                    f"{role['flag_remaining_ticks']},{role['death_penalty_level']},"
                    f"{role['recoverable_death_xp_loss']},{role['phase3_timer_policy_version']},'{installation}'::jsonb)")
        for skill, rank in sorted(role['learned_ranks'].items(), key=lambda item: int(item[0])):
            skills.append(f"('{role['character_id']}',0,'l2.skill.{skill}',{rank})")
    transfers = ''.join(transfer_sql(name, role) for name, role in roles.items())
    skill_sql = ('INSERT INTO character_learned_skills(character_id,slot,skill_key,skill_level)\nVALUES ' + ','.join(skills) + ';') if skills else ''
    accounts = ','.join(f"('{r['account_id']}')" for r in roles.values())
    sql = f"""BEGIN;
SET LOCAL lock_timeout = '3s';
LOCK TABLE characters, account_sessions, play_tickets, zone_epochs IN ACCESS EXCLUSIVE MODE;
DO $$ BEGIN
 IF EXISTS (SELECT 1 FROM characters) OR EXISTS (SELECT 1 FROM account_sessions)
 OR EXISTS (SELECT 1 FROM play_tickets) OR EXISTS (SELECT 1 FROM zone_epochs)
 OR EXISTS (SELECT 1 FROM zone_snapshots) OR EXISTS (SELECT 1 FROM idempotency_keys)
 OR EXISTS (SELECT 1 FROM character_skill_reuse) OR EXISTS (SELECT 1 FROM character_effects)
 OR EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'fixture requires a fresh empty database before API startup and admission'; END IF;
END $$;
INSERT INTO accounts(id) VALUES {accounts};
INSERT INTO characters(id,account_id,name,name_normalized,race,level,str,dex,con,"int",wit,men,
 pos_x,pos_y,xp,hp,mp,cp,class_profile,alive,revision,base_class_id,current_class_id,sp,sex,
 karma,pk_count,pvp_count,flag_remaining_ticks,death_penalty_level,recoverable_death_xp_loss,phase3_timer_policy_version,modifier_install_order)
VALUES {','.join(rows)};
INSERT INTO character_class_slots(character_id,slot,class_id,level,exp,sp)
SELECT id,0,current_class_id,level,xp,sp FROM characters;
{transfers}{skill_sql}
COMMIT;
"""

    if os.environ.get('PHASE4_ENABLED') == '1' and all('inventory' not in r for r in roles.values()):
        spec = importlib.util.spec_from_file_location('phase4_pending_kits', Path(__file__).with_name('phase4-pending-kits.py'))
        pending = importlib.util.module_from_spec(spec); spec.loader.exec_module(pending)
        sql = sql.removesuffix('COMMIT;\n') + pending.seed_sql(roles.values()) + 'COMMIT;\n'
    return sql


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--fixture', required=True, choices=sorted(contracts.PHASE3))
    p.add_argument('--owner-account', required=True, type=UUID)
    p.add_argument('--observer-account', type=UUID)
    p.add_argument('--dry-run', action='store_true')
    args = p.parse_args()
    try:
        if os.environ.get('AUTH_DEV_TOKENS') != '1' or os.environ.get('NIGHTFALL_PHASE3_FIXTURE') != '1':
            raise ValueError('AUTH_DEV_TOKENS=1 and NIGHTFALL_PHASE3_FIXTURE=1 required')
        guarded_database(os.environ.get('DATABASE_URL', ''))
        if args.owner_account == args.observer_account:
            raise ValueError('roles require distinct accounts')
        roles = contracts.profiles(args.fixture, REPO, args.observer_account is not None)
        roles = {name: role | {'account_id': str(args.owner_account if name == 'owner' else args.observer_account)} for name, role in roles.items()}
    except ValueError as error:
        p.error(str(error))
    sql = seed_sql(roles)
    if args.dry_run:
        print(sql)
    else:
        # libpq expands --dbname connection strings; PGDATABASE alone does not.
        # An owned psql shim consumes the environment URI in wrapper runs.
        env = psql_env(os.environ['DATABASE_URL'], os.environ.get('NIGHTFALL_FIXTURE_PSQL_SHIM') == '1')
        subprocess.run(['psql', '-X', '--set=ON_ERROR_STOP=1', '--quiet'], input=sql, text=True, env=env, check=True)
    print(json.dumps(dict(schema_version=1, fixture_enabled=True, fixture=args.fixture,
                         dry_run=args.dry_run, schema_status='reconciled-000008-0a37fb3', roles=roles,
                         data_sha256=contracts.data_hashes(REPO)), sort_keys=True))


if __name__ == '__main__':
    main()
