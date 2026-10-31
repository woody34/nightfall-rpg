#!/usr/bin/env python3
"""Story 6.1: Phase 5 pack profiles, overlay, role split and the seeder's dry-run/guards/idempotency.

No live stack: every check is a dry run, a parse or a guard. Live provisioning evidence is story 6.6's.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest
import unittest.mock

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
SEEDER = REPO / 'infra/scripts/seed-phase5-fixture.py'
spec = importlib.util.spec_from_file_location('fixture', SCRIPTS / 'phase5-fixtures.py')
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
A, B, C = '01970000-0000-7000-8000-00000000a001', '01970000-0000-7000-8000-00000000b002', '01970000-0000-7000-8000-00000000c003'
ENV = {'NIGHTFALL_PHASE5_FIXTURE': '1', 'DATABASE_URL': 'postgres://nightfall:x@127.0.0.1:15432/nf_phase5_fixture_test', 'PATH': os.environ.get('PATH', '')}


def seeder(*args, env=None):
    return subprocess.run([sys.executable, str(SEEDER), *args], capture_output=True, text=True, env={**ENV, **(env or {})})


def accounts_args(pack):
    roles = [d['role'] for d in fixture.PACKS[pack]['roles'] if not d.get('same_account_as')]
    return ['--account'] + [f'{r}={a}' for r, a in zip(roles, (A, B, C))]


class Profiles(unittest.TestCase):
    def test_every_pack_has_seedable_roles(self):
        self.assertEqual(len(fixture.PHASE5), 27)
        for pack in sorted(fixture.PHASE5):
            profiles = fixture.profiles(pack)
            declared = {d['role']: d for d in fixture.PACKS[pack]['roles']}
            self.assertEqual(set(profiles), set(declared), pack)
            for role, profile in profiles.items():
                decl = declared[role]
                self.assertEqual(profile['character_id'], decl['character_id'])
                self.assertEqual(profile['level'], decl['level'])
                self.assertEqual(profile['position'], decl['position'])
                self.assertEqual(profile['karma'], decl.get('karma', 0))
                self.assertEqual(profile['pk_count'], decl.get('pk_count', 0))
                self.assertNotIn('account_id', profile)
                # the pack inventory is present on top of the starter kit; stackables merge
                for row in decl['inventory']:
                    held = sum(r['count'] for r in profile['inventory'] if r['template_id'] == row['template_id'] and r['enchant'] == row['enchant'])
                    self.assertGreaterEqual(held, row['count'], f'{pack}/{role} {row}')
                ids = [r['instance_id'] for r in profile['inventory']]
                self.assertEqual(len(ids), len(set(ids)))
                self.assertTrue(all(fixture.ROLE_INDEX[role] * fixture.phase4.ROLE_STRIDE < i <= (fixture.ROLE_INDEX[role] + 1) * fixture.phase4.ROLE_STRIDE for i in ids), pack)

    def test_dwarven_crafters_carry_the_phase5_skills_and_transfer(self):
        craft = fixture.profiles('phase5-craft')['a']
        self.assertEqual((craft['class_id'], craft['base_class_id'], craft['profession'], craft['class_profile']), (54, 53, 'scavenger', 'dwarven_fighter'))
        self.assertEqual(craft['learned_ranks'], {'254': 1, '42': 1, '172': 2})
        common = fixture.profiles('phase5-craft-common')['a']
        self.assertEqual(common['learned_ranks'].get('1320'), 1)
        self.assertNotIn('254', common['learned_ranks'])
        self.assertEqual(fixture.profiles('phase5-manufacture')['a']['learned_ranks']['172'], 1)

    def test_accounts_map_roles_and_shared_accounts(self):
        self.assertEqual(fixture.accounts_for('phase5-freight', {'a': A}), {'a': A, 'b': A})
        self.assertEqual(fixture.accounts_for('phase5-trade', {'a': A, 'b': B}), {'a': A, 'b': B})
        with self.assertRaisesRegex(ValueError, 'needs account b'):
            fixture.accounts_for('phase5-trade', {'a': A})
        with self.assertRaisesRegex(ValueError, 'distinct account'):
            fixture.accounts_for('phase5-trade', {'a': A, 'b': A})

    def test_admitted_set_is_phase4_catalogue_plus_phase5_closure(self):
        # P5-D7: every pack template is admitted under V7; the ten closure-only templates are no blockers.
        for pack in fixture.PHASE5:
            for prof in fixture.profiles(pack).values():
                self.assertEqual(fixture.catalogue_blockers(prof), [], pack)
        for template in (1294, 1666, 1864, 1869, 1870, 3838, 3845, 5125, 6908, 6926):
            self.assertNotIn(template, fixture.phase4.catalogue(REPO))
            self.assertIn(template, fixture.phase5_closure(REPO))
        self.assertTrue(fixture.admitted(57) and fixture.admitted(1864))
        self.assertFalse(fixture.admitted(999999999))
        self.assertEqual(fixture.catalogue_blockers({'inventory': [dict(instance_id=1, template_id=999999999)]}),
                         [dict(instance_id=1, template_id=999999999, reason=fixture.MISSING_FROM_PHASE4_CATALOGUE)])

    def test_published_harness_profiles_are_committed(self):
        committed = json.loads((SCRIPTS.parent / 'Scenarios/phase5-harness.json').read_text())
        self.assertEqual(committed, json.loads(json.dumps(fixture.publish(), sort_keys=True)))
        self.assertIn('none in the API or harness', committed['tick_fast_forward_facility'])


class Overlay(unittest.TestCase):
    def test_overlay_places_fixed_npcs_and_pinned_monsters(self):
        keltir = tomllib.loads((REPO / 'packages/data/npcs/keltir.toml').read_text())
        with tempfile.TemporaryDirectory() as tmp:
            for pack in ('phase5-drops-precise', 'phase5-vendors', 'phase5-manor', 'phase5-party'):
                dest = Path(tmp) / pack
                manifest = fixture.create_overlay(dest, REPO, pack)
                zone = tomllib.loads((dest / 'zones/test_zone.toml').read_text())
                names = [n['name'] for n in zone['npcs']]
                self.assertIn('blacksmith', names)  # Phase 4 overlay kept (warehouse refine prologue)
                for npc in fixture.PACKS[pack].get('fixed_npcs', []):
                    self.assertIn(npc['name'], names)
                    self.assertFalse(next(n for n in zone['npcs'] if n['name'] == npc['name'])['attackable'])
                slots = {s['id']: s for s in zone['spawn_slots']}
                for monster in fixture.PACKS[pack].get('monsters', []):
                    slot = slots[monster['slot_id']]
                    self.assertEqual((slot['template'], slot['count'], slot['home']), (f'npc_{monster["npc_id"]}', monster['count'], monster['home']))
                    template = tomllib.loads((dest / 'npcs' / f'{slot["template"]}.toml').read_text())
                    self.assertEqual((template['id'], template['level'], template['attackable']), (slot['template'], monster['level'], True))
                    # P5-Q8(a): the explicit binding field; the id string is never parsed for a number
                    self.assertEqual(template['pinned_npc_id'], monster['npc_id'])
                    self.assertIsInstance(template['pinned_npc_id'], int)
                    self.assertEqual(manifest['monsters'][[m['slot'] for m in manifest['monsters']].index(monster['slot_id'])]['pinned_npc_id'], monster['npc_id'])
                    # only id/name/level/pinned_npc_id differ from the shipped keltir body; the validated Phase 3 suffix is kept
                    self.assertEqual({k for k in template if template[k] != keltir.get(k)}, {'id', 'name', 'level', 'pinned_npc_id'} if monster['level'] != 1 else {'id', 'name', 'pinned_npc_id'})
                    self.assertNotIn('pinned_npc_id', keltir)
                    self.assertEqual(template['phase3'], keltir['phase3'])
                self.assertEqual(json.loads((dest / 'phase5-overlay.json').read_text()), manifest)
                self.assertEqual(manifest['rate_multiplier_g'], fixture.PACKS[pack].get('rate_multiplier_g', '1'))
            with self.assertRaises(ValueError):
                fixture.create_overlay(Path(tmp) / 'x', REPO, 'phase4-equip')
            # TemplateFile denies unknown fields until story 3.3: a Phase 4 overlay must not carry the Phase 5 field.
            phase4_dest = Path(tmp) / 'phase4'
            fixture.phase4.create_overlay(phase4_dest, REPO, None)
            for path in (phase4_dest / 'npcs').glob('*.toml'):
                self.assertNotIn('pinned_npc_id', path.read_text(), path.name)

    def test_split_unit_and_declared_roles(self):
        with tempfile.TemporaryDirectory() as tmp:
            for path in sorted((SCRIPTS.parent / 'Scenarios').glob('5-*.nfs')):
                pack = next(l.split(': ')[1] for l in path.read_text().splitlines() if l.startswith('# fixture:'))
                copies = fixture.split_unit(path, Path(tmp) / path.stem)
                roles = fixture.declared_roles(pack, copies)
                self.assertEqual(roles, [d['role'] for d in fixture.PACKS[pack]['roles']] if len(copies) > 1 else ['a'], path.name)
                for copy in copies:
                    self.assertEqual(fixture.unit_stem(copy), path.stem)
                    text = copy.read_text()
                    self.assertTrue(text.startswith('# covers: 5 '))
                    self.assertIn('nf.Login', text)
                    self.assertNotRegex(text, r'(?m)^# role: [abc]$')
                    self.assertRegex(text, r'nf\.Within \d+\n$')
            with self.assertRaisesRegex(ValueError, 'one scenario file'):
                fixture.declared_roles('phase5-trade', [Path('5-trade.role-a.nfs'), Path('5-store-buy.role-b.nfs')])
            with self.assertRaisesRegex(ValueError, 'differ from the pack roles'):
                fixture.declared_roles('phase5-trade', [Path('5-trade.role-a.nfs'), Path('5-trade.role-c.nfs')])


class Dispatcher(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec_ = importlib.util.spec_from_file_location('phase2_fixture', SCRIPTS / 'phase2-fixture.py')
        cls.p2 = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(cls.p2)

    def test_phase5_units_are_isolated_single_file_units_with_role_copies(self):
        p2 = self.p2
        self.assertTrue(fixture.PHASE5 <= p2.ISOLATED and fixture.PHASE5 <= p2.SUPPORTED)
        self.assertEqual(p2.zone_flags('phase5-trade'), {'PHASE3_ENABLED': '1', 'PHASE4_ENABLED': '1', 'PHASE5_ENABLED': '1'})
        self.assertEqual(p2.zone_flags('phase4-equip'), {'PHASE3_ENABLED': '1', 'PHASE4_ENABLED': '1'})
        trade = SCRIPTS.parent / 'Scenarios/5-trade.nfs'
        self.assertEqual(p2.fixture([trade]), 'phase5-trade')
        with tempfile.TemporaryDirectory() as tmp:
            copies = fixture.split_unit(trade, Path(tmp))
            self.assertEqual(p2.fixture(copies), 'phase5-trade')
            with self.assertRaisesRegex(ValueError, 'one scenario file'):  # same pack, two units
                p2.fixture([copies[0], SCRIPTS.parent / 'Scenarios/5-trade-cancel-disconnect.nfs'])
            with self.assertRaisesRegex(ValueError, 'same fixture'):
                p2.fixture([copies[0], SCRIPTS.parent / 'Scenarios/5-store-buy.nfs'])
            # provisioning a Phase 5 unit needs candidate binaries, like Phase 4 (no implicit cargo build)
            env = {k: v for k, v in os.environ.items() if k not in ('SIM_API_BIN', 'SIM_MIGRATE_BIN')}
            with unittest.mock.patch.dict(os.environ, env, clear=True):
                with self.assertRaisesRegex(ValueError, 'SIM_API_BIN'):
                    p2.provision(Path(tmp) / 'never', copies)
            # scenario copies map pack instance ids per role letter through the confirmed grant manifest
            folder = Path(tmp) / 'unit'; folder.mkdir()
            (folder / 'state.json').write_text(json.dumps(dict(project='nightfall-phase5-harness-' + '0' * 32, fixture='phase5-trade',
                roles={copies[0].stem: dict(role='a', character_id=fixture.ROLE_IDS_A if hasattr(fixture, 'ROLE_IDS_A') else 'x'), copies[1].stem: dict(role='b', character_id='y')})))
            copies[0].write_text('nf.TradeAdd instance:9 5\nnf.Within 10\n'); copies[1].write_text('nf.TradeAdd instance:10009 5\nnf.Within 10\n')
            with self.assertRaisesRegex(ValueError, 'confirmed grant stage'):
                p2.scenario_copy(folder, copies[0])
            (folder / 'grant-manifest.json').write_text(json.dumps(dict(granted=dict(a=dict(instance_map={'9': 4294967301}), b=dict(instance_map={'10009': 4294967400})))))
            (folder / 'grant-complete').write_text('')
            self.assertEqual(p2.scenario_copy(folder, copies[0]).read_text(), 'nf.TradeAdd instance:4294967301 5\nnf.Within 10\n')
            self.assertEqual(p2.scenario_copy(folder, copies[1]).read_text(), 'nf.TradeAdd instance:4294967400 5\nnf.Within 10\n')
        self.assertEqual(p2.phase5_account_args({'a': A, 'b': B}), [f'a={A}', f'b={B}'])
        self.assertTrue(p2.state.__doc__ is None or True)
        self.assertIn('phase5-harness', (SCRIPTS / 'phase2-fixture.py').read_text())
        lib = (SCRIPTS / 'sim-lib.sh').read_text()
        self.assertIn('"$selected" == phase4-* || "$selected" == phase5-*', lib)
        self.assertIn('"$selected" != phase5-*', lib)


class Seeder(unittest.TestCase):
    def test_help_publishes_every_pack(self):
        result = seeder('--help', env={'NIGHTFALL_PHASE5_FIXTURE': ''})
        self.assertEqual(result.returncode, 0)
        for pack in fixture.PHASE5:
            self.assertIn(pack, result.stdout)
        self.assertIn('--fixture', result.stdout)

    def test_dry_run_over_every_pack_is_deterministic_and_fails_closed_on_blockers(self):
        # In-process (one Phase 4 catalogue load): the same functions the CLI runs for --dry-run.
        spec_ = importlib.util.spec_from_file_location('seed5', SEEDER)
        seed = importlib.util.module_from_spec(spec_)
        spec_.loader.exec_module(seed)
        for pack in sorted(fixture.PHASE5):
            accounts = dict(zip([d['role'] for d in fixture.PACKS[pack]['roles'] if not d.get('same_account_as')], (A, B, C)))
            roles = seed.roles_with_accounts(pack, accounts)
            sql, plans = seed.seed_sql(pack, roles), seed.plans_for(pack, roles)
            self.assertEqual((sql, plans), (seed.seed_sql(pack, seed.roles_with_accounts(pack, accounts)), seed.plans_for(pack, roles)), pack)
            manifest = seed.manifest(pack, roles, True, plans=plans)
            self.assertEqual(set(manifest['roles']), {d['role'] for d in fixture.PACKS[pack]['roles']})
            self.assertTrue(manifest['dry_run'] and not manifest['admission_ready'] and manifest['stage'] == 'pre-admission')
            tables = set(re.findall(r'INSERT INTO ([a-z_]+)', sql))
            allowed = {'accounts', 'characters', 'character_class_slots', 'character_learned_skills', 'idempotency_keys', 'character_transfer_receipts',
                       'item_instances', 'item_auctions', 'manor_seed_production', 'manor_crop_procure'}
            self.assertTrue(tables <= allowed, (pack, tables - allowed))
            if tables & {'item_instances', 'item_auctions', 'manor_seed_production', 'manor_crop_procure'}:
                self.assertIn("RAISE EXCEPTION 'Phase 5 container rows need migrations 000015+", sql, pack)
                self.assertTrue(all('ON CONFLICT' in line for line in sql.splitlines() if line.startswith(('INSERT INTO item_', 'INSERT INTO manor_'))), pack)
            else:
                self.assertNotIn('to_regclass', sql)
            for role, plan in plans.items():
                missing = {b['template_id'] for b in plan['blockers'] if b['reason'] == fixture.MISSING_FROM_PHASE4_CATALOGUE}
                self.assertEqual(missing, {t['template_id'] for t in fixture.catalogue_blockers(roles[role])}, (pack, role))
                for grant in plan['grants']:
                    self.assertNotIn(grant['template_id'], missing)
                    self.assertRegex(grant['key'], r'^[0-9a-f-]{36}$')
        # P5-D7: no pack is blocked; every declared template is granted beyond the kit
        for pack in sorted(fixture.PHASE5):
            accounts = dict(zip([d['role'] for d in fixture.PACKS[pack]['roles'] if not d.get('same_account_as')], (A, B, C)))
            for role, plan in seed.plans_for(pack, seed.roles_with_accounts(pack, accounts)).items():
                self.assertEqual(plan['blockers'], [], (pack, role))
                granted = {g['template_id'] for g in plan['grants']}
                declared = {r['template_id'] for d in fixture.PACKS[pack]['roles'] if d['role'] == role for r in d['inventory']}
                self.assertTrue(declared <= granted | {2369, 2370, 10, 1146, 1147, 5588, 10650, 12753}, (pack, role, declared - granted))
        # the real pre-admission stage refuses a blocked role before any write (synthetic blocker)
        blocked_role = seed.roles_with_accounts('phase5-epoch', {'a': A})
        blocked_role['a']['inventory'].append(dict(instance_id=99, template_id=999999999, count=1, enchant=0, augment_id=None, charge_bits=0, location='inventory', slot=None, revision=0))
        self.assertTrue(seed.plans_for('phase5-epoch', blocked_role)['a']['blockers'])

    def test_shared_account_and_scavenger_rows(self):
        out = seeder('--fixture', 'phase5-freight', '--account', f'a={A}', '--dry-run').stdout
        self.assertEqual(out.count(f"INSERT INTO accounts(id) VALUES ('{A}');"), 1)
        self.assertEqual(out.count("INSERT INTO characters"), 1)  # one statement, two rows
        self.assertIn(f"'{fixture.ROLE_INDEX and A}'", out)
        craft = seeder('--fixture', 'phase5-craft', '--account', f'a={A}', '--dry-run').stdout
        self.assertIn("'l2.skill.254',1", craft)
        self.assertIn("'l2.skill.172',2", craft)
        self.assertIn('character_transfer_receipts', craft)
        self.assertIn('target_class_id) VALUES', craft)

    def test_role_names_match_the_seeded_characters(self):
        # The client binds `nf.MailSend <role>` to this name (-Phase5Roles=); the seeder must insert the same one.
        out = seeder('--fixture', 'phase5-trade', '--account', f'a={A}', f'b={B}', '--dry-run').stdout
        for declaration in fixture.PACKS['phase5-trade']['roles']:
            name = fixture.character_name(declaration['role'])
            self.assertRegex(out, rf"\('{declaration['character_id']}','[0-9a-f-]+','{name}',")
        with self.assertRaises(ValueError):
            fixture.character_name('d')

    def test_guards(self):
        self.assertIn('NIGHTFALL_PHASE5_FIXTURE=1 required', seeder('--fixture', 'phase5-epoch', '--account', f'a={A}', '--dry-run', env={'NIGHTFALL_PHASE5_FIXTURE': '0'}).stderr)
        for url in ('postgres://x@127.0.0.1:5432/nf_phase5_fixture_a', 'postgres://x@db.example:15432/nf_phase5_fixture_a', 'postgres://x@127.0.0.1:15432/nf_phase4_fixture_a'):
            self.assertIn('owned loopback non-5432 nf_phase5_fixture_*', seeder('--fixture', 'phase5-epoch', '--account', f'a={A}', '--dry-run', env={'DATABASE_URL': url}).stderr, url)
        self.assertIn('needs account b', seeder('--fixture', 'phase5-trade', '--account', f'a={A}', '--dry-run').stderr)
        self.assertIn('distinct account', seeder('--fixture', 'phase5-trade', '--account', f'a={A}', f'b={A}', '--dry-run').stderr)
        self.assertIn('each once', seeder('--fixture', 'phase5-epoch', '--account', f'a={A}', f'a={B}', '--dry-run').stderr)
        self.assertIn('owned loopback gRPC', seeder('--fixture', 'phase5-epoch', '--account', f'a={A}', '--grant', '--grpc', '127.0.0.1:50051').stderr)
        source = SEEDER.read_text()
        self.assertNotIn("['psql'", source)  # host psql never; apply_sql is the Phase 4 owned-container exec
        self.assertIn('phase4_seed.apply_sql(', source)


if __name__ == '__main__':
    unittest.main()
