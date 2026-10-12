#!/usr/bin/env python3
"""Offline seed/dispatcher guards; owned-container calls are stubs, not live evidence."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS=Path(__file__).resolve().parents[1]
REPO=SCRIPTS.parents[2]
def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
fixture=load('fixture4',SCRIPTS/'phase2-fixture.py')
seed=load('seed4',REPO/'infra/scripts/seed-phase4-fixture.py')
scenarios=load('scenario4',SCRIPTS/'phase4-scenarios.py')


class Fixtures(unittest.TestCase):
    def test_published_inputs_match_and_ids_are_disjoint(self):
        data=json.loads((SCRIPTS.parent/'Scenarios/phase4-packs.json').read_text())
        self.assertEqual(data,fixture.phase4.publish())
        for pack,roles in data['packs'].items():
            ids=[r['instance_id'] for role in roles.values() for r in role['inventory']]
            self.assertEqual(len(set(ids)),len(ids),pack)
            for role in roles.values():
                self.assertGreater(role['allocator']['next_item_instance'],max(r['instance_id'] for r in role['inventory']))
                kit=next(k for k in fixture.phase4.table('starter_kits')['kits'] if k['class_id']==role['base_class_id'])
                for item in kit['items']:
                    self.assertTrue(any(r['template_id']==item['template_id'] and r['count']>=item['count'] for r in role['inventory']))
                self.assertEqual(role['paperdoll'],{r['slot']:r['instance_id'] for r in role['inventory'] if r['slot']})
                self.assertTrue(all(r['source']['revision']== '3ca488dd2bd0bfaca43e378886a3c2e37968153a' for r in role['inventory']))

    def test_armour_providers_and_combat_geometry_are_source_bound(self):
        row,templates=fixture.phase4.armor_set()
        caps=fixture.phase4.capabilities();skills={r['id']:r for r in fixture.phase4.table('capabilities')['skills']}
        self.assertTrue(row['shield_skills']);self.assertTrue(row['enchant6_skills'])
        self.assertTrue(all(caps[i]['admitted'] for i in templates))
        self.assertTrue(all(skills[r['id']]['admitted'] for k in ('skills','shield_skills','enchant6_skills') for r in row[k]))
        shirt=fixture.phase4.enchant4_item()
        self.assertTrue(caps[shirt['template_id']]['admitted']);self.assertTrue(shirt['enchant4_skills'])
        four=fixture.phase4.profiles('phase4-armor-enchant4')['owner']['inventory']
        self.assertTrue(any(i['template_id']==shirt['template_id'] and i['enchant']==4 for i in four))
        six=fixture.phase4.profiles('phase4-armor-enchant6')['owner']['inventory']
        self.assertTrue(all(any(i['template_id']==t and i['enchant']==6 for i in six) for t in templates))
        candidates=fixture.phase4.combat_skill_candidates()
        self.assertTrue(candidates)
        # A named source dependency, not admission of deferred learned skills.
        self.assertEqual([c for c in candidates if c['admitted']],[])
        self.assertTrue(all(c['blockers'] for c in candidates))
        kinds=(SCRIPTS.parent/'Scenarios/4-combat-kinds.nfs').read_text()
        self.assertNotIn('damage_numbers',kinds);self.assertIn('nf.Unequip rhand',kinds)
        self.assertEqual(kinds.count('nf.Mark\n'),kinds.count('nf.Attack\n'))
        with tempfile.TemporaryDirectory() as tmp:
            overlay=Path(tmp)/'data';fixture.phase4.create_overlay(overlay,REPO)
            zone=(overlay/'zones/test_zone.toml').read_text()
            for name,pos in [('pole_secondary_a','108, 101'),('pole_secondary_b','108, 99')]:
                self.assertIn('template = "'+name+'"',zone);self.assertIn('home = ['+pos+']',zone)
                self.assertIn('attackable = true',(overlay/('npcs/'+name+'.toml')).read_text())

    def test_all_scenarios_and_pair_dispatch(self):
        paths=scenarios.matrix();self.assertEqual(scenarios.validate(paths),[])
        for path in paths:
            pack=next(l.split(': ')[1] for l in path.read_text().splitlines() if l.startswith('# fixture:'))
            if pack in fixture.phase4.PAIRED:
                with self.assertRaises(ValueError): fixture.fixture([path])
            else:self.assertEqual(fixture.fixture([path]),pack)
        for a,b in fixture.phase4.declared_units(paths[0].parent).values():
            self.assertIn(fixture.fixture([paths[0].parent/a,paths[0].parent/b]),fixture.PHASE4)
        self.assertTrue(scenarios.client.validate(paths)) # E5 not merged; never silently pass.

    def test_sql_one_transaction_and_no_production_low_range(self):
        roles=fixture.phase4.profiles('phase4-shield',paired=True)
        roles={name:r|{'account_id':'01970000-0000-7000-8000-00000000004'+str(i+2)} for i,(name,r) in enumerate(roles.items())}
        sql=seed.seed_sql(roles)
        self.assertEqual(sql.count('BEGIN;'),1);self.assertEqual(sql.count('COMMIT;'),1)
        self.assertEqual(sql.count('INSERT INTO item_instances('),sum(len(r['inventory']) for r in roles.values()))
        self.assertEqual(sql.count('INSERT INTO item_ledger('),sum(len(r['inventory']) for r in roles.values()))
        self.assertNotIn('INSERT INTO item_id_ranges',sql)
        self.assertIn('pg_stat_activity',sql);self.assertIn('ACCESS EXCLUSIVE',sql)
        bad=copy.deepcopy(roles);bad['observer']['inventory'][0]['instance_id']=1
        with self.assertRaises(ValueError):seed.seed_sql(bad)
        bad=copy.deepcopy(roles);bad['owner']['inventory'][0]['count']=0
        with self.assertRaises(ValueError):seed.seed_sql(bad)

    def test_owned_database_and_real_auth_guards(self):
        seed.guarded_database('postgres://u:p@127.0.0.1:15432/nf_phase4_fixture_test')
        for url in ('postgres://u:p@localhost:5432/nf_phase4_fixture_test','postgres://u:p@remote:15432/nf_phase4_fixture_test','postgres://u:p@localhost:15432/nightfall','postgres://u:p@localhost:15432/nf_phase4_fixture_test?options=x'):
            with self.assertRaises(ValueError):seed.guarded_database(url)
        with tempfile.TemporaryDirectory() as tmp:
            token=Path(tmp)/'token';token.write_text('test:01970000-0000-7000-8000-000000000042')
            env=dict(SIM_PHASE4_AUTH_TOKEN_FILE=str(token),SIM_PHASE4_OIDC_ISSUER='http://fixture',SIM_PHASE4_OIDC_AUDIENCE='fixture',SIM_PHASE4_OWNER_ACCOUNT='01970000-0000-7000-8000-000000000042')
            with patch.dict(os.environ,env,clear=True):
                with self.assertRaises(ValueError):fixture.phase4.denied_auth_inputs()
                token.write_text('signed-fixture-token')
                self.assertEqual(fixture.phase4.denied_auth_inputs()['account'],env['SIM_PHASE4_OWNER_ACCOUNT'])

    def test_seed_uses_only_owned_postgres_container_exec(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'compose.json'
            path.write_text(json.dumps(dict(services=dict(postgres=dict(environment=dict(POSTGRES_DB='nf_phase4_fixture_test',POSTGRES_USER='u',POSTGRES_PASSWORD='p'),ports=[dict(target=5432,published=15432)])))))
            env=dict(DATABASE_URL='postgres://u:p@127.0.0.1:15432/nf_phase4_fixture_test')
            with patch.dict(os.environ,env),patch.object(seed.soak_base.subprocess,'run') as call:
                seed.seed_container('BEGIN;COMMIT;','nightfall-phase4-harness-abcd-123',path)
                command=call.call_args.args[0]
                self.assertEqual(command[:2],['docker','compose'])
                self.assertIn('exec',command);self.assertIn('-T',command)
                self.assertEqual(command[-5:],['postgres','psql','-X','--set=ON_ERROR_STOP=1','--quiet'])
                self.assertEqual(call.call_args.kwargs['env']['PGHOST'],'127.0.0.1')
                self.assertEqual(call.call_args.kwargs['env']['PGPORT'],'5432')
                with self.assertRaises(ValueError):seed.seed_container('SQL','nightfall-other-project',path)
                self.assertEqual(call.call_count,1)

    def test_seed_verdict_refuses_admission_pending(self):
        roles=fixture.phase4.profiles('phase4-equip')
        supplied=seed.manifest('phase4-equip',roles,False)
        for pack in sorted(fixture.PHASE4):
            selected=fixture.phase4.profiles(pack,paired=pack in fixture.phase4.PAIRED)
            selected={name:r|{'account_id':'01970000-0000-7000-8000-00000000004'+str(i+2)} for i,(name,r) in enumerate(selected.items())}
            self.assertNotIn('VALUES ;',seed.seed_sql(selected),pack)
        self.assertFalse(supplied['admission_ready'])
        with self.assertRaises(ValueError):fixture.phase4.validate_seed_manifest(supplied,'phase4-equip','owner','observer',REPO,False)


if __name__=='__main__':unittest.main()
