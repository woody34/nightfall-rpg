#!/usr/bin/env python3
"""Provision a Phase 4 fixture pack through the supported admission transport (D112).

Two stages, both against an owned fixture stack only:

* pre-admission (default, before API startup): the Phase 3 character/learned-skill seed
  plus a re-run of the frozen pending starter-kit backfill migration
  (m20261012_000011), applied with psql inside the owned Compose postgres. No item rows,
  ledger rows or allocator rows are written by this script.
* ``--grant`` (after the API is healthy, before any client logs in): the authenticated dev
  ``GameService/GrantItem`` gRPC (AUTH_DEV_TOKENS=1, ``Bearer test:<account>``), one call
  per (role, item) with deterministic UUIDv5 idempotency keys, serial per character. The
  first grant materializes the pending starter kit. Admission is confirmed from the
  offline ``GetInventory`` baseline: every pack template/count/enchant/slot must match.

Items the transport cannot express (enchant level, charges, augment) and pack items that
would have to be equipped beyond the starter kit are refused loudly; nothing is guessed.
``--dry-run`` prints the pre-admission SQL and the grant plan without any write or call.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
from urllib.parse import urlparse
import uuid
from uuid import UUID

REPO = Path(__file__).resolve().parents[2]
BACKFILL = REPO / 'apps/api/src/infrastructure/postgres/migrations/m20261012_000011_pending_item_backfill.sql'
PROTO = REPO / 'packages/proto'
SERVICE = 'nightfall.v1.GameService'
# D113: optional enchant level on the dev GrantItem request (proto field name; protobuf JSON
# accepts the proto name as well as its lowerCamel json_name).
ENCHANT_FIELD = 'enchant_level'
GRANT_ATTEMPTS = 3


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


def guarded_grpc(address):
    match = re.fullmatch(r'(127\.0\.0\.1|localhost):([0-9]{2,5})', address or '')
    if not match or int(match[2]) in (50051,) or not 1024 <= int(match[2]) <= 65535:
        raise ValueError('Phase4 grant requires an owned loopback gRPC address (not 50051)')
    return address


def seed_sql(roles):
    """Pre-admission SQL: Phase 3 characters + the frozen pending-kit backfill. No item rows."""
    for role in roles.values():
        UUID(role['character_id']); UUID(role['account_id'])
    sql = base.seed_sql(roles).removesuffix('COMMIT;\n')
    for role in roles.values():
        if 'tokens' in role:
            first, second = role['tokens']
            if any(type(n) is not int or not 0 <= n <= 1 for n in (first, second)): raise ValueError('invalid token seed')
            sql += (f"UPDATE characters SET token_tier_1_count={first},token_tier_2_count={second},"
                    f"milestone_claimed_mask={first+2*second} WHERE id='{role['character_id']}';\n")
    sql += 'COMMIT;\n'
    # The frozen migration's temp table is ON COMMIT DROP, so it runs in its own transaction.
    return sql + 'BEGIN;\n' + BACKFILL.read_text() + '\nCOMMIT;\n'


def grant_plan(fixture, name, role, repo=REPO):
    """Grants that turn the materialized starter kit into the pack inventory.

    Returns (grants, blockers). A grant is {index, template_id, count, enchant, key}; blockers
    name pack rows the GrantItem transport cannot produce. D113: enchanted non-stackable
    instances are granted at their level through ENCHANT_FIELD; kit instances stay at +0.
    """
    catalogue = contracts.catalogue(repo)
    kit = next(k for k in contracts.table('starter_kits', repo)['kits'] if k['class_id'] == role['base_class_id'])
    kit_total, kit_equipped, kit_instances = {}, {}, {}
    for row in kit['items']:
        kit_total[row['template_id']] = kit_total.get(row['template_id'], 0) + row['count']
        kit_instances[row['template_id']] = kit_instances.get(row['template_id'], 0) + 1
        if row['equipped']:
            kit_equipped[row['template_id']] = kit_equipped.get(row['template_id'], 0) + 1
    blockers, totals, enchants, equipped = [], {}, {}, {}
    for row in role['inventory']:
        template = row['template_id']
        if row['charge_bits'] or row['augment_id'] is not None:
            blockers.append(dict(instance_id=row['instance_id'], template_id=template,
                                 reason='charges/augment have no GrantItem transport'))
        if row['enchant'] and catalogue[template]['stackable']:
            blockers.append(dict(instance_id=row['instance_id'], template_id=template, reason='enchanted stackable'))
        totals[template] = totals.get(template, 0) + row['count']
        enchants.setdefault(template, []).append(row['enchant'])
        if row['slot'] is not None:
            equipped[template] = equipped.get(template, 0) + 1
    for template, count in equipped.items():
        if count > kit_equipped.get(template, 0):
            blockers.append(dict(template_id=template, reason='equipped beyond the starter kit needs the bot equip prologue'))
    grants = []
    for template in sorted(totals):
        if catalogue[template]['stackable']:
            missing = totals[template] - kit_total.get(template, 0)
            if missing < 0:
                raise ValueError(f'pack {fixture}/{name} holds less of template {template} than its starter kit')
            calls = [(missing, 0)] if missing > 0 else []
        else:
            levels = sorted(enchants[template])
            for _ in range(kit_instances.get(template, 0)):
                if 0 not in levels:
                    blockers.append(dict(template_id=template, reason='a starter-kit instance cannot be granted enchanted'))
                    break
                levels.remove(0)
            calls = [(1, level) for level in levels]
        for count, enchant in calls:
            index = len(grants)
            label = f'nightfall:phase4-fixture:{fixture}:{name}:{index}:{template}:{count}' + (f':+{enchant}' if enchant else '')
            grants.append(dict(index=index, template_id=template, count=count, enchant=enchant,
                               key=str(uuid.uuid5(uuid.NAMESPACE_URL, label))))
    return grants, blockers


def call(method, body, account, grpc, runner=subprocess.run):
    """One unary call through `buf curl` (gRPC over h2c); returns the decoded JSON reply."""
    buf = shutil.which('buf')
    if not buf: raise ValueError('buf is required for the GrantItem transport (buf curl)')
    command = [buf, 'curl', '--protocol', 'grpc', '--http2-prior-knowledge', '--emit-defaults',
               '--schema', str(PROTO), '-H', f'authorization: Bearer test:{account}',
               '-d', json.dumps(body), f'http://{grpc}/{SERVICE}/{method}']
    result = runner(command, capture_output=True, text=True, timeout=30)
    if result.returncode != 0:
        raise ValueError(f'{method} refused: {(result.stderr or result.stdout).strip()}')
    return json.loads(result.stdout)


def b64(identifier):
    import base64
    return base64.b64encode(UUID(identifier).bytes).decode()


def number(value):
    return int(value) if value not in (None, '') else 0


def grant_role(fixture, name, role, grpc, runner=subprocess.run):
    grants, blockers = grant_plan(fixture, name, role)
    if blockers:
        raise ValueError(f'pack {fixture}/{name} cannot be provisioned by GrantItem: {json.dumps(blockers)}')
    character, account = role['character_id'], role['account_id']
    receipts = []
    for grant in grants:  # serial per character
        body = dict(characterId=b64(character), templateId=grant['template_id'], count=str(grant['count']),
                    idempotencyKey=grant['key'])
        if grant['enchant']:
            body[ENCHANT_FIELD] = grant['enchant']
        for attempt in range(GRANT_ATTEMPTS):
            try:
                reply = call('GrantItem', body, account, grpc, runner)
                break
            except ValueError as error:
                # UNAVAILABLE "retry the same key": the same key replays, never grants twice.
                if 'unavailable' not in str(error).lower() or attempt == GRANT_ATTEMPTS - 1: raise
        result = reply.get('result', {})
        if result.get('receiptId') != grant['key'] or result.get('operation') != 'ITEM_OPERATION_GRANT':
            raise ValueError(f'GrantItem receipt mismatch for {fixture}/{name} {grant}')
        if not any(number(item.get('templateId')) == grant['template_id'] for item in reply.get('inventory', {}).get('items', [])):
            raise ValueError(f'GrantItem baseline lacks template {grant["template_id"]}')
        receipts.append(dict(grant, inventory_revision=number(result.get('inventoryRevision'))))
    if not grants:
        # Nothing to grant: the kit materializes at first login; there is no offline baseline yet.
        return dict(receipts=[], instance_map={}, inventory_revision=None, confirmed='kit-at-login')
    inventory = call('GetInventory', dict(characterId=b64(character)), account, grpc, runner)['inventory']
    return dict(receipts=receipts, instance_map=confirm(fixture, name, role, inventory),
                inventory_revision=number(inventory.get('inventoryRevision')), confirmed='GetInventory')


def confirm(fixture, name, role, inventory):
    """The offline baseline must equal the pack; returns pack instance id -> server instance id."""
    slots = {number(entry.get('instanceId')): entry.get('slot', '') for entry in inventory.get('paperdoll', [])}
    server = []
    for item in inventory.get('items', []):
        identity = number(item.get('instanceId'))
        slot = slots.get(identity)
        server.append(dict(id=identity, template_id=number(item.get('templateId')), count=number(item.get('count')),
                           enchant=number(item.get('enchant')),
                           slot=None if slot is None else slot.removeprefix('EQUIP_SLOT_').lower()))
    key = lambda row: (row['template_id'], row['enchant'], row['slot'] or '', row['count'])
    expected = sorted((dict(id=r['instance_id'], template_id=r['template_id'], count=r['count'], enchant=r['enchant'],
                            slot=r['slot']) for r in role['inventory']), key=lambda r: (key(r), r['id']))
    actual = sorted(server, key=lambda r: (key(r), r['id']))
    if [key(r) for r in expected] != [key(r) for r in actual]:
        raise ValueError(f'pack {fixture}/{name} admission mismatch: expected {[key(r) for r in expected]} got {[key(r) for r in actual]}')
    return {str(e['id']): a['id'] for e, a in zip(expected, actual)}


def apply_sql(sql, project, compose_file):
    """Wrapper runs install an owned psql shim (exec inside the fixture's own postgres);
    otherwise exec psql in the owned Compose project directly."""
    if os.environ.get('NIGHTFALL_FIXTURE_PSQL_SHIM') == '1':
        env = base.psql_env(os.environ['DATABASE_URL'], True)
        subprocess.run(['psql', '-X', '--set=ON_ERROR_STOP=1', '--quiet'], input=sql, text=True, env=env, check=True)
    else:
        seed_container(sql, project, compose_file)


def seed_container(sql, project, compose_file):
    if not re.fullmatch(r'nightfall-phase4-harness-(?:soak-)?[0-9a-f-]+', project or ''): raise ValueError('owned Phase4 Compose project required')
    soak_base.seed_container(sql, project, compose_file, r'nightfall-phase4-harness-(?:soak-)?[0-9a-f-]+')


def manifest(pack, roles, dry_run, stage='pre-admission', plans=None, granted=None):
    return dict(schema_version=1, fixture=pack, fixture_enabled=True, dry_run=dry_run, stage=stage,
                transport='dev GrantItem gRPC after pending-kit backfill (D112)',
                admission_ready=stage == 'granted', roles=roles, data_sha256=contracts.phase3.data_hashes(REPO),
                grant_plan=plans or {}, granted=granted or {})


def plans_for(fixture, roles):
    result = {}
    for name, role in roles.items():
        grants, blockers = grant_plan(fixture, name, role)
        result[name] = dict(grants=grants, blockers=blockers)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--fixture', required=True, choices=sorted(contracts.PHASE4))
    parser.add_argument('--owner-account', required=True, type=UUID)
    parser.add_argument('--observer-account', type=UUID)
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--grant', action='store_true', help='post-startup GrantItem stage')
    parser.add_argument('--grpc', help='owned API gRPC address for --grant (127.0.0.1:PORT)')
    parser.add_argument('--compose-project')
    parser.add_argument('--compose-file', type=Path)
    parser.add_argument('--manifest-out', type=Path)
    args = parser.parse_args()
    try:
        if os.environ.get('NIGHTFALL_PHASE4_FIXTURE') != '1': raise ValueError('NIGHTFALL_PHASE4_FIXTURE=1 required')
        guarded_database(os.environ.get('DATABASE_URL', ''))
        if args.owner_account == args.observer_account: raise ValueError('distinct account roles required')
        roles = {name: r | {'account_id': str(args.owner_account if name == 'owner' else args.observer_account)}
                 for name, r in contracts.profiles(args.fixture, REPO, args.observer_account is not None).items()}
        plans = plans_for(args.fixture, roles)
        if args.dry_run:
            print(seed_sql(roles))
            result = manifest(args.fixture, roles, True, plans=plans)
        elif args.grant:
            grpc = guarded_grpc(args.grpc)
            if os.environ.get('AUTH_DEV_TOKENS') != '1' and any(p['grants'] for p in plans.values()):
                raise ValueError('GrantItem requires AUTH_DEV_TOKENS=1')
            granted = {name: grant_role(args.fixture, name, role, grpc) for name, role in roles.items()}
            result = manifest(args.fixture, roles, False, 'granted', plans, granted)
        else:
            # Refuse before any write when the pack is not expressible through GrantItem.
            blocked = {name: plan['blockers'] for name, plan in plans.items() if plan['blockers']}
            if blocked: raise ValueError(f'pack {args.fixture} cannot be provisioned by GrantItem: {json.dumps(blocked)}')
            apply_sql(seed_sql(roles), args.compose_project, args.compose_file)
            result = manifest(args.fixture, roles, False, plans=plans)
        if args.manifest_out: args.manifest_out.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    except (ValueError, KeyError, TypeError, subprocess.SubprocessError) as error: parser.error(str(error))
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__': main()
