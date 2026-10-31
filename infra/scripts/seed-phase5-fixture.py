#!/usr/bin/env python3
"""Provision a Phase 5 fixture pack (plan §5.3; story 6.1) through the supported transports.

Stages, all against an owned fixture stack only (loopback, non-default ports, `nf_phase5_fixture_*`):

* pre-admission (default, before API startup): the Phase 3 character seed for every pack role
  (accounts, characters, learned skills, class-transfer receipts) plus the Phase 5 container rows a
  pack declares (warehouse rows, the auction fixture schedule, the Gludio manor lists). Container
  rows go into the design §11.4 tables inside a DO block that refuses unless migrations 000015+
  (story 4.1) are present, so an older schema fails closed before any Phase 5 row is written.
* ``--grant`` (after the API is healthy, before any client logs in): the dev ``GrantItem`` gRPC
  per (role, template) with deterministic UUIDv5 keys (idempotent: a re-run replays the receipts),
  confirmed from the offline ``GetInventory`` baseline.

What the transports cannot express is refused loudly before any write: augments/charges, enchanted
stackables, equipment beyond the starter kit, slot overflow, and templates the Phase 4 item
catalogue lacks (E51 closure at integration). ``--dry-run`` prints the SQL and the grant plan.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid
from uuid import UUID

REPO = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


contracts = load('phase5_seed_contracts', REPO / 'apps/client-unreal/Scripts/phase5-fixtures.py')
phase4_seed = load('phase5_seed_phase4', Path(__file__).with_name('seed-phase4-fixture.py'))
base = load('phase5_seed_base', Path(__file__).with_name('seed-phase3-fixture.py'))
TABLES = REPO / 'packages/data/tables-phase5'
MIGRATIONS = REPO / 'apps/api/src/infrastructure/postgres/migrations'
PHASE5_TABLES = ('item_auctions', 'manor_seed_production', 'manor_crop_procure')
ROLE_ORDER = ('a', 'b', 'c')


def guarded_database(url):
    from urllib.parse import urlparse
    parsed = urlparse(url)
    if (parsed.scheme not in ('postgres', 'postgresql') or parsed.hostname not in ('localhost', '127.0.0.1')
            or parsed.port in (None, 5432) or not re.fullmatch(r'/nf_phase5_fixture_[a-z0-9_]+', parsed.path)
            or parsed.query or parsed.fragment):
        raise ValueError('Phase5 requires owned loopback non-5432 nf_phase5_fixture_* DB')


def roles_with_accounts(pack, accounts):
    mapping = contracts.accounts_for(pack, accounts)
    return {name: profile | {'account_id': mapping[name]} for name, profile in contracts.profiles(pack, REPO).items()}


def character_sql(roles):
    """Phase 3 character seed for every role (one account row per distinct account)."""
    for role in roles.values():
        UUID(role['character_id']); UUID(role['account_id'])
    sql = base.seed_sql(roles)
    # base.seed_sql inserts one accounts row per role; roles on one account must insert it once.
    accounts = []
    for role in roles.values():
        if role['account_id'] not in accounts:
            accounts.append(role['account_id'])
    sql = re.sub(r'INSERT INTO accounts\(id\) VALUES [^;]+;', 'INSERT INTO accounts(id) VALUES ' + ','.join(f"('{a}')" for a in accounts) + ';', sql, count=1)
    return sql


def migration_numbers():
    return sorted(int(m[1]) for p in MIGRATIONS.glob('m*_*.sql') for m in [re.match(r'm\d+_(\d{6})_', p.name)] if m and not p.name.endswith('_down.sql'))


def container_sql(pack, roles):
    """Phase 5 container rows of the pack, guarded by the presence of the story 4.1 tables."""
    declared = contracts.PACKS[pack]
    statements = []
    if declared.get('warehouse_rows'):
        owner = roles['a']
        for index, row in enumerate(declared['warehouse_rows']):
            instance = 900_000 + index + 1  # fixture range, disjoint from the roles' inventory strides
            statements.append(
                f"INSERT INTO item_instances(instance_id,character_id,template_id,count,enchant,location,owner_kind,revision)"
                f" VALUES ({instance},'{owner['character_id']}',{row['template_id']},{row['count']},0,'warehouse','warehouse',0)"
                f" ON CONFLICT (instance_id) DO NOTHING;")
    if 'auction' in declared:
        import tomllib
        auction = tomllib.loads((TABLES / 'auction.toml').read_text(encoding='utf-8'))
        for row in auction['fixture_schedule']['auctions']:
            if row['instance'] != declared['auction']['instance']:
                continue
            statements.append(
                f"INSERT INTO item_auctions(id,instance_npc,item_index,state,extend_state,start_at,end_at,revision)"
                f" VALUES ({row['auction_id']},{row['instance']},{row['round']},0,0,to_timestamp({row['start_epoch_ms']} / 1000.0),"
                f"to_timestamp({row['end_epoch_ms']} / 1000.0),0) ON CONFLICT (id) DO NOTHING;")
    if 'manor' in declared:
        import tomllib
        manor = tomllib.loads((TABLES / 'manor.toml').read_text(encoding='utf-8'))['fixture_gludio']
        for seed in manor['seed_production']:
            statements.append(f"INSERT INTO manor_seed_production(castle_id,seed_id,price,start_amount,amount,revision) VALUES "
                              f"({manor['castle_id']},{seed['seed_id']},{seed['price']},{seed['amount']},{seed['amount']},0) ON CONFLICT (castle_id,seed_id) DO NOTHING;")
        for crop in manor['crop_procurement']:
            statements.append(f"INSERT INTO manor_crop_procure(castle_id,crop_id,price,start_amount,amount,reward_type,revision) VALUES "
                              f"({manor['castle_id']},{crop['crop_id']},{crop['price']},{crop['amount']},{crop['amount']},{crop['reward_type']},0) ON CONFLICT (castle_id,crop_id) DO NOTHING;")
    if not statements:
        return ''
    guard = ' OR '.join(f"to_regclass('{t}') IS NULL" for t in PHASE5_TABLES)
    return ('BEGIN;\nDO $$ BEGIN\n IF ' + guard + " OR NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='item_instances' AND column_name='owner_kind')\n"
            " THEN RAISE EXCEPTION 'Phase 5 container rows need migrations 000015+ (story 4.1)'; END IF;\nEND $$;\n" + '\n'.join(statements) + '\nCOMMIT;\n')


def seed_sql(pack, roles):
    return character_sql(roles) + container_sql(pack, roles)


def grant_plan(pack, name, role):
    """Grants beyond the materialized starter kit, with deterministic keys; blockers name what the
    transport cannot produce (never guessed)."""
    catalogue = contracts.phase4.catalogue(REPO)
    kit = next(k for k in contracts.phase4.table('starter_kits', REPO)['kits'] if k['class_id'] == role['base_class_id'])
    kit_total = {}
    for row in kit['items']:
        kit_total[row['template_id']] = kit_total.get(row['template_id'], 0) + row['count']
    blockers = list(contracts.catalogue_blockers(role, REPO))
    known = {row['template_id'] for row in role['inventory'] if row['template_id'] in catalogue}
    if not blockers:
        blockers += phase4_seed.capacity_blockers(role, catalogue, REPO)
    totals, enchants = {}, {}
    for row in role['inventory']:
        template = row['template_id']
        if row['charge_bits'] or row['augment_id'] is not None:
            blockers.append(dict(instance_id=row['instance_id'], template_id=template, reason='charges/augment have no GrantItem transport'))
        if row['location'] == 'paperdoll' and template not in kit_total:
            blockers.append(dict(instance_id=row['instance_id'], template_id=template, reason='equipped beyond the starter kit needs the bot equip prologue'))
        totals[template] = totals.get(template, 0) + row['count']
        enchants.setdefault(template, []).append(row['enchant'])
    grants = []
    for template in sorted(totals):
        if template not in known:
            continue
        if catalogue[template]['stackable']:
            if any(enchants[template]):
                blockers.append(dict(template_id=template, reason='enchanted stackable'))
            missing = totals[template] - kit_total.get(template, 0)
            calls = [(missing, 0)] if missing > 0 else []
        else:
            levels = sorted(enchants[template])
            for _ in range(kit_total.get(template, 0)):
                if levels and levels[0] == 0:
                    levels.pop(0)
            calls = [(1, level) for level in levels]
        for count, enchant in calls:
            index = len(grants)
            label = f'nightfall:phase5-fixture:{pack}:{name}:{index}:{template}:{count}' + (f':+{enchant}' if enchant else '')
            grants.append(dict(index=index, template_id=template, count=count, enchant=enchant, key=str(uuid.uuid5(uuid.NAMESPACE_URL, label))))
    return grants, blockers


def grant_role(pack, name, role, grpc, runner=subprocess.run, sleep=None):
    grants, blockers = grant_plan(pack, name, role)
    if blockers:
        raise ValueError(f'pack {pack}/{name} cannot be provisioned by GrantItem: {json.dumps(blockers)}')
    character, account = role['character_id'], role['account_id']
    receipts = []
    for grant in grants:
        body = dict(characterId=phase4_seed.b64(character), templateId=grant['template_id'], count=str(grant['count']), idempotencyKey=grant['key'])
        if grant['enchant']:
            body[phase4_seed.ENCHANT_FIELD] = grant['enchant']
        reply = None
        for attempt in range(phase4_seed.GRANT_ATTEMPTS):
            if attempt:
                (sleep or __import__('time').sleep)(phase4_seed.GRANT_BACKOFF_SECONDS[attempt - 1])
            try:
                reply = phase4_seed.call('GrantItem', body, account, grpc, runner)
                break
            except phase4_seed.GrpcError as error:
                if error.code != 'unavailable' or attempt == phase4_seed.GRANT_ATTEMPTS - 1:
                    raise
        result = reply.get('result', {})
        if result.get('receiptId') != grant['key'] or result.get('operation') != 'ITEM_OPERATION_GRANT':
            raise ValueError(f'GrantItem receipt mismatch for {pack}/{name} {grant}')
        revision = phase4_seed.number(result.get('inventoryRevision'))
        if receipts and revision <= receipts[-1]['inventory_revision']:
            raise ValueError(f'GrantItem receipt revisions must strictly increase for {pack}/{name}')
        receipts.append(dict(grant, inventory_revision=revision))
    if not grants:
        return dict(receipts=[], instance_map={}, inventory_revision=None, confirmed='kit-at-login')
    inventory = phase4_seed.call('GetInventory', dict(characterId=phase4_seed.b64(character)), account, grpc, runner)['inventory']
    revision = phase4_seed.number(inventory.get('inventoryRevision'))
    if revision != receipts[-1]['inventory_revision']:
        raise ValueError(f'GetInventory revision {revision} differs from the last GrantItem receipt revision for {pack}/{name}')
    return dict(receipts=receipts, instance_map=phase4_seed.confirm(pack, name, role, inventory), inventory_revision=revision, confirmed='GetInventory')


def manifest(pack, roles, dry_run, stage='pre-admission', plans=None, granted=None):
    return dict(schema_version=1, fixture=pack, fixture_enabled=True, dry_run=dry_run, stage=stage,
                transport='Phase 3 character SQL + Phase 5 container rows (guarded on migrations 000015+) + dev GrantItem gRPC',
                admission_ready=stage == 'granted', roles=roles, data_sha256=contracts.phase3.data_hashes(REPO),
                migrations_present=migration_numbers()[-1] if migration_numbers() else None,
                grant_plan=plans or {}, granted=granted or {})


def plans_for(pack, roles):
    return {name: dict(grants=grants, blockers=blockers) for name, role in roles.items() for grants, blockers in [grant_plan(pack, name, role)]}


def parse_accounts(values):
    out = {}
    for value in values or []:
        role, _, account = value.partition('=')
        if role not in ROLE_ORDER or role in out:
            raise ValueError('accounts are a=<uuid> b=<uuid> c=<uuid>, each once')
        out[role] = str(UUID(account))
    if 'a' not in out:
        raise ValueError('role a needs an account')
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--fixture', required=True, choices=sorted(contracts.PHASE5))
    parser.add_argument('--account', action='extend', nargs='+', help='role=account UUID pairs: a=<uuid> [b=<uuid> c=<uuid>]')
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--grant', action='store_true', help='post-startup GrantItem stage')
    parser.add_argument('--grpc', help='owned API gRPC address for --grant (127.0.0.1:PORT)')
    parser.add_argument('--compose-project')
    parser.add_argument('--compose-file', type=Path)
    parser.add_argument('--manifest-out', type=Path)
    args = parser.parse_args()
    try:
        if os.environ.get('NIGHTFALL_PHASE5_FIXTURE') != '1':
            raise ValueError('NIGHTFALL_PHASE5_FIXTURE=1 required')
        guarded_database(os.environ.get('DATABASE_URL', ''))
        roles = roles_with_accounts(args.fixture, parse_accounts(args.account))
        plans = plans_for(args.fixture, roles)
        if args.dry_run:
            print(seed_sql(args.fixture, roles))
            result = manifest(args.fixture, roles, True, plans=plans)
        elif args.grant:
            grpc = phase4_seed.guarded_grpc(args.grpc)
            if os.environ.get('AUTH_DEV_TOKENS') != '1' and any(p['grants'] for p in plans.values()):
                raise ValueError('GrantItem requires AUTH_DEV_TOKENS=1')
            granted = {name: grant_role(args.fixture, name, role, grpc) for name, role in roles.items()}
            result = manifest(args.fixture, roles, False, 'granted', plans, granted)
        else:
            blocked = {name: plan['blockers'] for name, plan in plans.items() if plan['blockers']}
            if blocked:
                raise ValueError(f'pack {args.fixture} cannot be provisioned: {json.dumps(blocked)}')
            phase4_seed.apply_sql(seed_sql(args.fixture, roles), args.compose_project, args.compose_file)
            result = manifest(args.fixture, roles, False, plans=plans)
        if args.manifest_out:
            args.manifest_out.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    except (ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        parser.error(str(error))
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
