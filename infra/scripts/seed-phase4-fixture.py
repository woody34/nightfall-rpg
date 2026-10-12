#!/usr/bin/env python3
"""Generate guarded pre-admission item rows/ledger; apply only through owned Compose.

Admission sidecar recording is an explicit E2/E3 dependency, never a guessed checkpoint.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import urlparse
from uuid import UUID

REPO = Path(__file__).resolve().parents[2]

def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

contracts = load('phase4_seed_contracts', REPO / 'apps/client-unreal/Scripts/phase4-fixtures.py')
base = load('phase4_seed_base', Path(__file__).with_name('seed-phase3-fixture.py'))
soak_base = load('phase4_seed_container', Path(__file__).with_name('seed-phase3-soak.py'))
SLOTS = ('under head hair hair2 neck rhand chest lhand rear lear gloves legs feet rfinger lfinger '
         'lbracelet rbracelet deco1 deco2 deco3 deco4 deco5 deco6 cloak belt').split()


def guarded_database(url):
    parsed = urlparse(url)
    if (parsed.scheme not in ('postgres', 'postgresql') or parsed.hostname not in ('localhost', '127.0.0.1')
            or parsed.port in (None, 5432) or not re.fullmatch(r'/nf_phase4_fixture_[a-z0-9_]+', parsed.path)
            or parsed.query or parsed.fragment):
        raise ValueError('Phase4 requires owned loopback non-5432 nf_phase4_fixture_* DB')


def canonical_item(row):
    return dict(id=row['instance_id'], template_id=row['template_id'], count=row['count'],
                location=row['location'], slot=row['slot'], enchant=row['enchant'], augment_id=row['augment_id'],
                charges=row['charge_bits'], created_tick=0, revision=row['revision'])


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def seed_sql(roles):
    # UUIDs and exact integers are validated even for programmatic callers.
    for role in roles.values():
        UUID(role['character_id']); UUID(role['account_id'])
    sql = base.seed_sql(roles).removesuffix('COMMIT;\n')
    sql += '''LOCK TABLE item_instances, item_ledger, item_id_ranges, pending_item_requests IN ACCESS EXCLUSIVE MODE;
DO $$ BEGIN
 IF EXISTS (SELECT 1 FROM item_instances) OR EXISTS (SELECT 1 FROM item_ledger)
 OR EXISTS (SELECT 1 FROM item_id_ranges) OR EXISTS (SELECT 1 FROM pending_item_requests)
 THEN RAISE EXCEPTION 'Phase4 fixture requires empty item state before admission'; END IF;
END $$;
'''
    ids = set()
    for role in roles.values():
        char = quote(role['character_id'])
        if 'tokens' in role:
            first, second = role['tokens']
            if any(type(n) is not int or not 0 <= n <= 1 for n in (first, second)): raise ValueError('invalid token seed')
            sql += f'UPDATE characters SET token_tier_1_count={first},token_tier_2_count={second},milestone_claimed_mask={first+2*second} WHERE id={char};\n'
        for row in role['inventory']:
            if row['instance_id'] in ids or not 0 < row['instance_id'] < role['allocator']['next_item_instance'] < contracts.RANGE_END: raise ValueError('duplicate/out-of-range seed identity')
            ids.add(row['instance_id'])
            for key in ('instance_id', 'template_id', 'count', 'enchant', 'charge_bits', 'revision'):
                if type(row[key]) is not int or row[key] < 0: raise ValueError('invalid seed integer')
            if row['count'] == 0 or row['count'] >= 2**63 or row['enchant'] >= 2**32: raise ValueError('invalid count/enchant')
            slot = 'NULL' if row['slot'] is None else str(SLOTS.index(row['slot']))
            if (row['location'] == 'paperdoll') != (row['slot'] is not None): raise ValueError('seed location/slot mismatch')
            values = [row['instance_id'], char, quote(row['location']), slot, row['template_id'], row['count'], row['enchant'], 'NULL', row['charge_bits'], 0, row['revision']]
            sql += 'INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,charge_bits,created_tick,revision) VALUES (' + ','.join(map(str, values)) + ');\n'
            delta = dict(operation='grant', operation_ordinal=0, instance=row['instance_id'], before=None,
                         after=canonical_item(row), count_delta=row['count'], receipt=None, causes=[])
            sql += f"INSERT INTO item_ledger(character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES ({char},1,1,0,0,'grant',{row['instance_id']},0,{quote(json.dumps(delta,sort_keys=True,separators=(',',':')))}::jsonb);\n"
    # Production ranges start at2^32 (migration000010); low fixture range is recorded
    # by the admission adapter, never inserted into that production allocator table.
    return sql + 'COMMIT;\n'


def seed_container(sql, project, compose_file):
    if not re.fullmatch(r'nightfall-phase4-harness-(?:soak-)?[0-9a-f-]+', project or ''): raise ValueError('owned Phase4 Compose project required')
    soak_base.seed_container(sql, project, compose_file, r'nightfall-phase4-harness-(?:soak-)?[0-9a-f-]+')


def manifest(pack, roles, dry_run):
    return dict(schema_version=1, fixture=pack, fixture_enabled=True, dry_run=dry_run,
                schema_status='rows-ledger-000010; admission-transport-pending', admission_ready=False,
                roles=roles, data_sha256=contracts.phase3.data_hashes(REPO),
                allocator=dict(range_start=contracts.RANGE_START, range_end_exclusive=contracts.RANGE_END,
                               next_item_instance=max(r['allocator']['next_item_instance'] for r in roles.values())))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', required=True, choices=sorted(contracts.PHASE4))
    parser.add_argument('--owner-account', required=True, type=UUID)
    parser.add_argument('--observer-account', type=UUID)
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--compose-project')
    parser.add_argument('--compose-file', type=Path)
    parser.add_argument('--manifest-out', type=Path)
    args = parser.parse_args()
    try:
        if os.environ.get('NIGHTFALL_PHASE4_FIXTURE') != '1': raise ValueError('NIGHTFALL_PHASE4_FIXTURE=1 required')
        guarded_database(os.environ.get('DATABASE_URL', ''))
        if args.owner_account == args.observer_account: raise ValueError('distinct account roles required')
        roles = {name:r | {'account_id': str(args.owner_account if name == 'owner' else args.observer_account)}
                 for name,r in contracts.profiles(args.fixture, REPO, args.observer_account is not None).items()}
        sql = seed_sql(roles)
        result = manifest(args.fixture, roles, args.dry_run)
        if args.manifest_out: args.manifest_out.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
        if args.dry_run: print(sql)
        else:
            # Fail BEFORE writes until E2/E3 provides the typed low-range admission sink.
            raise ValueError('Phase4 fixture admission transport pending E2/E3; use --dry-run for SQL/input review')
    except (ValueError, KeyError, TypeError) as error: parser.error(str(error))
    print(json.dumps(result,sort_keys=True))


if __name__ == '__main__': main()
