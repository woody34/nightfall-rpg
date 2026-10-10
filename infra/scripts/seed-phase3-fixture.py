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
from urllib.parse import urlparse
from uuid import UUID

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


def seed_sql(roles):
    rows, skills = [], []
    for name, role in roles.items():
        s = role['base_stats']
        number = lambda value: 'NULL' if value is None else str(value)
        rows.append(f"('{role['character_id']}','{role['account_id']}','Skill{name}','skill{name}','{role['race']}',"
                    f"{role['level']},{s['str']},{s['dex']},{s['con']},{s['int']},{s['wit']},{s['men']},"
                    f"{role['position'][0]},{role['position'][1]},{role['xp']},{number(role['hp'])},{number(role['mp'])},"
                    f"{number(role['cp'])},'{role['class_profile']}',true,0,{role['base_class_id']},{role['class_id']},"
                    f"{role['sp']},'female',0,0,0,0,0,0,1)")
        for skill, rank in sorted(role['learned_ranks'].items(), key=lambda item: int(item[0])):
            skills.append(f"('{role['character_id']}',0,'l2.skill.{skill}',{rank})")
    accounts = ','.join(f"('{r['account_id']}')" for r in roles.values())
    return f"""BEGIN;
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
 karma,pk_count,pvp_count,flag_remaining_ticks,death_penalty_level,recoverable_death_xp_loss,phase3_timer_policy_version)
VALUES {','.join(rows)};
INSERT INTO character_class_slots(character_id,slot,class_id,level,exp,sp)
SELECT id,0,current_class_id,level,xp,sp FROM characters;
INSERT INTO character_learned_skills(character_id,slot,skill_key,skill_level)
VALUES {','.join(skills)};
COMMIT;
"""


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
        url = urlparse(os.environ['DATABASE_URL'])
        from urllib.parse import unquote
        env = dict(os.environ, PGCONNECT_TIMEOUT='3', PGAPPNAME='phase3_fixture_seed')
        if os.environ.get('NIGHTFALL_FIXTURE_PSQL_SHIM') == '1':
            env['PGDATABASE'] = os.environ['DATABASE_URL']
        else:
            env.update(PGHOST=url.hostname, PGPORT=str(url.port), PGDATABASE=url.path[1:],
                       PGUSER=unquote(url.username or ''), PGPASSWORD=unquote(url.password or ''))
        subprocess.run(['psql', '-X', '--set=ON_ERROR_STOP=1', '--quiet'], input=sql, text=True, env=env, check=True)
    print(json.dumps(dict(schema_version=1, fixture_enabled=True, fixture=args.fixture,
                         dry_run=args.dry_run, schema_status='reconciled-000008-0a37fb3', roles=roles,
                         data_sha256=contracts.data_hashes(REPO)), sort_keys=True))


if __name__ == '__main__':
    main()
