#!/usr/bin/env python3
"""Offline seed/dispatcher guards; owned-container calls are stubs, not live evidence."""
import copy
import importlib.util
import json
import os
import subprocess
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
        # A named source dependency, not admission of deferred learned skills. Rev 7 (the only
        # pre-D50 data, identified by its rules_manifest data_sha256) admits none; every later
        # revision (rev 8, rev 9, ...) admits at least blow skills 16 and 263 (D50 promotion).
        rev7=fixture.phase4.table('rules_manifest')['data_sha256']=='5b77911ec6bba0b186b9de16d63ea832726943475152a82ea40a5ebd203c4656'
        admitted={c['skill_id'] for c in candidates if c['admitted']}
        kinds_owner=fixture.phase4.profiles('phase4-kinds')['owner']
        if rev7:
            self.assertEqual(admitted,set())
            self.assertEqual((kinds_owner['profession'],kinds_owner['level'],kinds_owner['learned_ranks']),('human_fighter',14,{}))
        else:
            self.assertLessEqual({16,263},admitted)
            # D50 un-pended: source ranks of the human_fighter→rogue→treasure_hunter chain at level 40.
            self.assertEqual((kinds_owner['profession'],kinds_owner['level'],kinds_owner['learned_ranks']),('treasure_hunter',40,{'16':24,'263':3}))
        self.assertTrue(all(c['blockers'] for c in candidates if not c['admitted']))
        kinds=(SCRIPTS.parent/'Scenarios/4-combat-kinds.nfs').read_text()
        self.assertNotIn('damage_numbers',kinds);self.assertIn('nf.Unequip rhand',kinds)
        self.assertEqual(kinds.count('nf.Mark\n'),kinds.count('nf.Attack\n'))
        with tempfile.TemporaryDirectory() as tmp:
            overlay=Path(tmp)/'data';fixture.phase4.create_overlay(overlay,REPO)
            zone=(overlay/'zones/test_zone.toml').read_text()
            blacksmith=fixture.phase4.table('capacities')['blacksmith']
            self.assertEqual(zone.count('name = '+json.dumps(blacksmith['template'])),1)
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
        self.assertEqual(scenarios.client.validate(paths), []) # E5 must register every canonical scenario.

    def test_pre_admission_sql_seeds_characters_and_pending_kit_only(self):
        # D112: no item, ledger or allocator rows; the frozen 000011 backfill in its own transaction.
        roles=fixture.phase4.profiles('phase4-shield',paired=True)
        roles={name:r|{'account_id':'01970000-0000-7000-8000-00000000004'+str(i+2)} for i,(name,r) in enumerate(roles.items())}
        sql=seed.seed_sql(roles)
        self.assertEqual(sql.count('BEGIN;'),2);self.assertEqual(sql.count('COMMIT;'),2)
        for table in ('item_instances','item_ledger','item_id_ranges'):self.assertNotIn('INSERT INTO '+table,sql)
        self.assertIn('INSERT INTO pending_item_requests',sql);self.assertIn(seed.BACKFILL.read_text(),sql)
        self.assertLess(sql.index('INSERT INTO characters'),sql.index('INSERT INTO pending_item_requests'))
        self.assertIn('pg_stat_activity',sql)

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

    def test_dry_run_grant_plan_over_every_pack(self):
        """D112/D113 dry run over all packs: deterministic GrantItem plans; every pack provisions."""
        blocked_packs=set();planned=0
        for pack in sorted(fixture.PHASE4):
            paired=pack in fixture.phase4.PAIRED
            roles=fixture.phase4.profiles(pack,paired=paired)
            roles={name:r|{'account_id':'01970000-0000-7000-8000-00000000004'+str(i+2)} for i,(name,r) in enumerate(roles.items())}
            sql=seed.seed_sql(roles);self.assertNotIn('INSERT INTO item_instances',sql,pack)
            plans=seed.plans_for(pack,roles)
            self.assertEqual(plans,seed.plans_for(pack,roles),pack) # deterministic keys
            for name,plan in plans.items():
                keys=[g['key'] for g in plan['grants']];self.assertEqual(len(set(keys)),len(keys),pack)
                for grant in plan['grants']:
                    self.assertGreater(grant['count'],0)
                    if not fixture.phase4.catalogue()[grant['template_id']]['stackable']:self.assertEqual(grant['count'],1)
                planned+=len(plan['grants'])
                if plan['blockers']:blocked_packs.add(pack)
            # Kit + grants reproduce every pack template total and every enchant level (D113).
            for name,role in roles.items():
                kit=next(k for k in fixture.phase4.table('starter_kits')['kits'] if k['class_id']==role['base_class_id'])
                totals={}
                for row in kit['items']:totals[row['template_id']]=totals.get(row['template_id'],0)+row['count']
                for grant in plans[name]['grants']:totals[grant['template_id']]=totals.get(grant['template_id'],0)+grant['count']
                expected={}
                for row in role['inventory']:expected[row['template_id']]=expected.get(row['template_id'],0)+row['count']
                self.assertEqual(totals,expected,(pack,name))
                self.assertEqual(sorted(g['enchant'] for g in plans[name]['grants'] if g['enchant']),
                                 sorted(r['enchant'] for r in role['inventory'] if r['enchant']),(pack,name))
        enchanted={p for p in fixture.PHASE4 if any(r['enchant'] for role in fixture.phase4.profiles(p,paired=p in fixture.phase4.PAIRED).values() for r in role['inventory'])}
        self.assertEqual(enchanted,{'phase4-enchant','phase4-blacksmith','phase4-equip-onepiece','phase4-armor-enchant4','phase4-armor-enchant6'})
        # D125 (amended)/T27: the plan models the grant's slot capacity; dev grants may exceed
        # max load, so the overweight 4-overload pack provisions unchanged.
        self.assertEqual(blocked_packs,set()) # all 39 packs provision
        self.assertEqual(len(fixture.PHASE4),39);self.assertGreater(planned,0)
        print(f'D112/D113 dry run: all {len(fixture.PHASE4)} packs provision, {planned} grants planned (enchanted: {sorted(enchanted)})',flush=True)

    def test_grant_stage_calls_grantitem_then_confirms_offline(self):
        roles=fixture.phase4.profiles('phase4-equip')
        role=roles['owner']|{'account_id':'01970000-0000-7000-8000-000000000042'}
        grants,blockers=seed.grant_plan('phase4-equip','owner',role);self.assertFalse(blockers)
        calls=[]
        def server_inventory():
            items=[];paperdoll=[]
            for index,row in enumerate(role['inventory']):
                identity=4294967296+index
                items.append(dict(instanceId=str(identity),templateId=row['template_id'],count=str(row['count']),enchant=row['enchant']))
                if row['slot']:paperdoll.append(dict(slot='EQUIP_SLOT_'+row['slot'].upper(),instanceId=str(identity)))
            return dict(inventoryRevision=str(1+len(grants)),items=items,paperdoll=paperdoll)
        class Done:
            def __init__(self,body):self.returncode=0;self.stdout=json.dumps(body);self.stderr=''
        def runner(command,**kwargs):
            calls.append(command);body=json.loads(command[command.index('-d')+1]);method=command[-1].rsplit('/',1)[1]
            self.assertIn('authorization: Bearer test:01970000-0000-7000-8000-000000000042',command)
            if method=='GrantItem':
                # Receipt revisions advance per grant; a replayed key returns its original revision.
                index=next(g['index'] for g in grants if g['key']==body['idempotencyKey'])
                return Done(dict(result=dict(receiptId=body['idempotencyKey'],operation='ITEM_OPERATION_GRANT',inventoryRevision=str(2+index)),
                                 inventory=server_inventory()))
            return Done(dict(inventory=server_inventory()))
        with patch.object(seed.shutil,'which',return_value='/usr/bin/buf'):
            result=seed.grant_role('phase4-equip','owner',role,'127.0.0.1:15051',runner)
        self.assertEqual(len(calls),len(grants)+1);self.assertEqual(calls[-1][-1].rsplit('/',1)[1],'GetInventory')
        self.assertEqual(set(result['instance_map']),{str(r['instance_id']) for r in role['inventory']})
        # A baseline that differs from the pack (one item missing) refuses loudly.
        def short(command,**kwargs):
            reply=runner(command,**kwargs);body=json.loads(reply.stdout)
            body['inventory']['items']=body['inventory']['items'][1:];reply.stdout=json.dumps(body);return reply
        with patch.object(seed.shutil,'which',return_value='/usr/bin/buf'):
            with self.assertRaisesRegex(ValueError,'admission mismatch'):seed.grant_role('phase4-equip','owner',role,'127.0.0.1:15051',short)
            class Refused:
                returncode=1;stdout='';stderr='failed_precondition: GrantItem requires AUTH_DEV_TOKENS=1'
            with self.assertRaisesRegex(ValueError,'GrantItem refused'):seed.grant_role('phase4-equip','owner',role,'127.0.0.1:15051',lambda c,**k:Refused())
        # D113: an enchanted pack grants its +3 sword through ENCHANT_FIELD and confirms the level.
        enchanted=fixture.phase4.profiles('phase4-enchant')['owner']|{'account_id':'01970000-0000-7000-8000-000000000042'}
        bodies=[]
        def enchant_runner(command,**kwargs):
            body=json.loads(command[command.index('-d')+1]);method=command[-1].rsplit('/',1)[1]
            items=[dict(instanceId=str(4294967296+i),templateId=r['template_id'],count=str(r['count']),enchant=r['enchant'])
                   for i,r in enumerate(enchanted['inventory'])]
            paperdoll=[dict(slot='EQUIP_SLOT_'+r['slot'].upper(),instanceId=str(4294967296+i)) for i,r in enumerate(enchanted['inventory']) if r['slot']]
            if method=='GrantItem':
                bodies.append(body)
                return Done(dict(result=dict(receiptId=body['idempotencyKey'],operation='ITEM_OPERATION_GRANT',inventoryRevision=str(1+len(bodies))),
                                 inventory=dict(items=items,paperdoll=paperdoll)))
            return Done(dict(inventory=dict(inventoryRevision=str(1+len(bodies)),items=items,paperdoll=paperdoll)))
        with patch.object(seed.shutil,'which',return_value='/usr/bin/buf'):
            result=seed.grant_role('phase4-enchant','owner',enchanted,'127.0.0.1:15051',enchant_runner)
        self.assertEqual([b.get(seed.ENCHANT_FIELD) for b in bodies if b['templateId']==71],[None,3])
        self.assertEqual(seed.ENCHANT_FIELD,'enchant_level')
        self.assertEqual(set(result['instance_map']),{str(r['instance_id']) for r in enchanted['inventory']})
        # The server reporting +0 where the pack needs +3 refuses.
        def wrong(command,**kwargs):
            reply=enchant_runner(command,**kwargs);body=json.loads(reply.stdout)
            for item in body['inventory']['items']:item['enchant']=0
            reply.stdout=json.dumps(body);return reply
        with patch.object(seed.shutil,'which',return_value='/usr/bin/buf'):
            with self.assertRaisesRegex(ValueError,'admission mismatch'):seed.grant_role('phase4-enchant','owner',enchanted,'127.0.0.1:15051',wrong)
        for address in ('127.0.0.1:50051','10.0.0.1:15051','127.0.0.1','localhost:80'):
            with self.assertRaises(ValueError):seed.guarded_grpc(address)

    def grant_fixture(self,failures):
        """A one-grant pack (phase4-equip owner) whose GrantItem fails with `failures` first."""
        role=fixture.phase4.profiles('phase4-equip')['owner']|{'account_id':'01970000-0000-7000-8000-000000000042'}
        grants,_=seed.grant_plan('phase4-equip','owner',role)
        log=[];pending=list(failures)
        items=[dict(instanceId=str(4294967296+i),templateId=r['template_id'],count=str(r['count']),enchant=r['enchant']) for i,r in enumerate(role['inventory'])]
        paperdoll=[dict(slot='EQUIP_SLOT_'+r['slot'].upper(),instanceId=str(4294967296+i)) for i,r in enumerate(role['inventory']) if r['slot']]
        class Reply:
            def __init__(self,code,out,err):self.returncode=code;self.stdout=out;self.stderr=err
        def runner(command,**kwargs):
            body=json.loads(command[command.index('-d')+1]);method=command[-1].rsplit('/',1)[1]
            log.append((method,body.get('idempotencyKey')))
            if method=='GrantItem' and body['idempotencyKey']==grants[0]['key'] and pending:
                code,message=pending.pop(0)
                return Reply(1,'',json.dumps(dict(code=code,message=message)))
            if method=='GrantItem':
                index=next(g['index'] for g in grants if g['key']==body['idempotencyKey'])
                return Reply(0,json.dumps(dict(result=dict(receiptId=body['idempotencyKey'],operation='ITEM_OPERATION_GRANT',
                    inventoryRevision=str(2+index)),inventory=dict(items=items,paperdoll=paperdoll))),'')
            return Reply(0,json.dumps(dict(inventory=dict(inventoryRevision=str(1+len(grants)),items=items,paperdoll=paperdoll))),'')
        return role,grants,log,runner

    def test_grant_retries_only_grpc_unavailable_with_backoff_and_the_same_key(self):
        role,grants,log,runner=self.grant_fixture([('unavailable','zone paused'),('unavailable','zone paused')])
        slept=[]
        with patch.object(seed.shutil,'which',return_value='/usr/bin/buf'):
            result=seed.grant_role('phase4-equip','owner',role,'127.0.0.1:15051',runner,sleep=slept.append)
        self.assertEqual([k for m,k in log if m=='GrantItem'][:3],[grants[0]['key']]*3)
        self.assertEqual(slept,list(seed.GRANT_BACKOFF_SECONDS));self.assertEqual(result['confirmed'],'GetInventory')
        # Exhausted retries refuse; FAILED_PRECONDITION whose text says "unavailable" is not retried.
        for failures,attempts in (([('unavailable','x')]*3,3),([('failed_precondition','paused skill checkpoint unavailable')],1)):
            role,grants,log,runner=self.grant_fixture(failures);slept=[]
            with patch.object(seed.shutil,'which',return_value='/usr/bin/buf'):
                with self.assertRaises(seed.GrpcError) as caught:
                    seed.grant_role('phase4-equip','owner',role,'127.0.0.1:15051',runner,sleep=slept.append)
            self.assertEqual(len(log),attempts);self.assertEqual(len(slept),attempts-1)
            self.assertEqual(caught.exception.code,failures[0][0])
        self.assertEqual(seed.grpc_code('not json'),'')

    def test_final_inventory_revision_must_equal_the_last_receipt(self):
        role,grants,log,runner=self.grant_fixture([])
        def drift(command,**kwargs):
            reply=runner(command,**kwargs)
            if command[-1].endswith('/GetInventory'):
                body=json.loads(reply.stdout);body['inventory']['inventoryRevision']='99';reply.stdout=json.dumps(body)
            return reply
        def flat(command,**kwargs):
            reply=runner(command,**kwargs)
            if command[-1].endswith('/GrantItem') and reply.returncode==0:
                body=json.loads(reply.stdout);body['result']['inventoryRevision']='2';reply.stdout=json.dumps(body)
            return reply
        with patch.object(seed.shutil,'which',return_value='/usr/bin/buf'):
            with self.assertRaisesRegex(ValueError,'differs from the last GrantItem receipt'):
                seed.grant_role('phase4-equip','owner',role,'127.0.0.1:15051',drift)
            self.assertGreater(len(grants),1)
            with self.assertRaisesRegex(ValueError,'strictly increase'):
                seed.grant_role('phase4-equip','owner',role,'127.0.0.1:15051',flat)

    def test_scenario_copy_maps_pack_instances_per_role(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);scenarios=folder/'src';scenarios.mkdir()
            owner=scenarios/'4-x-a.nfs';owner.write_text('nf.Equip instance:7\nnf.Expect equipped instance:7 == 1\n')
            observer=scenarios/'4-x-b.nfs';observer.write_text('nf.Use instance:3\n')
            (folder/'state.json').write_text(json.dumps(dict(project='nightfall-phase4-harness-'+'0'*32,fixture='phase4-equip',
                roles={'4-x-a':dict(character_id=fixture.phase4.OWNER),'4-x-b':dict(character_id=fixture.phase4.OBSERVER)})))
            with self.assertRaisesRegex(ValueError,'confirmed grant stage'):fixture.scenario_copy(folder,owner)
            (folder/'grant-manifest.json').write_text(json.dumps(dict(granted=dict(owner=dict(instance_map={'7':4294967301}),
                                                                                  observer=dict(instance_map={'3':4294967400})))))
            (folder/'grant-complete').write_text('')
            self.assertEqual(fixture.scenario_copy(folder,owner).read_text(),'nf.Equip instance:4294967301\nnf.Expect equipped instance:4294967301 == 1\n')
            self.assertEqual(fixture.scenario_copy(folder,observer).read_text(),'nf.Use instance:4294967400\n')
            observer.write_text('nf.Use instance:7\n') # owner's id is not the observer's
            with self.assertRaisesRegex(ValueError,'was not provisioned'):fixture.scenario_copy(folder,observer)

    def test_grant_plan_models_slot_capacity(self):
        # D125 (amended)/T27: the dev grant enforces slot capacity; the plan refuses first.
        role=copy.deepcopy(fixture.phase4.profiles('phase4-equip')['owner'])
        catalogue=fixture.phase4.catalogue()
        self.assertEqual(seed.capacity_blockers(role,catalogue),[])
        # D125 amended: weight above max load is allowed for dev grants (605210 vs 67620 here).
        heavy=copy.deepcopy(fixture.phase4.profiles('phase4-overload')['owner'])
        self.assertEqual(seed.capacity_blockers(heavy,catalogue),[])
        self.assertGreater(sum(int(catalogue[r['template_id']]['weight'])*r['count'] for r in heavy['inventory']),67620)
        sword=next(i for i,r in sorted(catalogue.items()) if not r['stackable'] and r['kind']=='Weapon' and int(r['weight'])<=100)
        many=copy.deepcopy(role)
        many['inventory']+= [dict(many['inventory'][0],instance_id=1000+i,template_id=sword,slot=None,location='inventory',enchant=0) for i in range(80)]
        self.assertTrue(any('ordinary slots' in b['reason'] for b in seed.capacity_blockers(many,catalogue)))
        # Stackables take one slot per template regardless of count.
        arrows=copy.deepcopy(role)
        arrows['inventory'].append(dict(role['inventory'][0],instance_id=999,template_id=17,count=10,slot=None,location='inventory'))
        self.assertEqual(seed.capacity_blockers(arrows,catalogue),[])

    def test_item_evidence_reads_a_gracefully_closed_epoch(self):
        # D122/T23: stop the owned API with SIGTERM, then export the closed epoch non-live.
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);dest=root/'unit';dest.mkdir();log=root/'calls'
            (dest/'u.xml').write_text('<testsuites><properties><property name="own_entity_id" value="01970000-0000-7000-8000-000000000040"/></properties></testsuites>')
            replay=root/'replay';replay.write_text(f'''#!/usr/bin/env bash
echo "$*" >> "{log}"
[[ -e "{root}/flaky" ]] && {{ rm "{root}/flaky"; echo "jetstream request timed out" >&2; exit 2; }}
out=""; while (($#)); do [[ "$1" == --out ]] && out="$2"; shift; done
echo recording > "$out"
''');replay.chmod(0o755)
            script=f'''. "{SCRIPTS}/sim-lib.sh"
sleep 300 & SIM_STARTED_API_PID=$!
SIM_FIXTURE_DIR="{root}" SIM_REPLAY_BIN="{replay}" SIM_API_STOP_GRACE=5
sim_closed_epoch "{dest}" u; echo "rc=$? closed=$CLOSED_EPOCH pid=[$SIM_STARTED_API_PID]"
'''
            (root/'flaky').touch() # first export attempt times out (JetStream right after shutdown)
            out=subprocess.run(['bash','-c',script],capture_output=True,text=True,timeout=60)
            self.assertIn(f'rc=0 closed={dest}/closed-epoch/u.nfr pid=[]',out.stdout,out.stderr)
            calls=log.read_text().splitlines()
            self.assertEqual(len(calls),2)
            for call in calls:
                self.assertIn('export --zone 1 --latest --session 01970000-0000-7000-8000-000000000040 --out',call)
                self.assertNotIn('--live',call)
            # Without an owned fixture API there is no closed epoch to read.
            out=subprocess.run(['bash','-c',f'. "{SCRIPTS}/sim-lib.sh"; SIM_FIXTURE_DIR=""; sim_closed_epoch "{dest}" u; echo rc=$?'],
                               capture_output=True,text=True,timeout=30)
            self.assertIn('rc=1',out.stdout);self.assertIn('requires an owned fixture API',out.stderr)
        lib=(SCRIPTS/'sim-lib.sh').read_text()
        self.assertIn('sim_closed_epoch "$dest" "$name" || return 1\n    recording="$CLOSED_EPOCH"',lib)
        # The live capture stays for the replay/trace gates.
        self.assertIn('--latest --session "$entity" --live --out "$dest/$name.nfr"',lib)

    def test_zone_flags_are_per_unit(self):
        # D114: every unit boots V6; the per-unit manifest field and exec-api check remain.
        for pack in fixture.PHASE4|fixture.PHASE3|fixture.PHASE2:
            self.assertEqual(fixture.zone_flags(pack),{'PHASE3_ENABLED':'1','PHASE4_ENABLED':'1'},pack)
        source=(SCRIPTS/'phase2-fixture.py').read_text()
        self.assertIn("'zone_flags': zone_flags(selected)",source);self.assertIn("!= zone_flags(value['fixture'])",source)
        lib=(SCRIPTS/'sim-lib.sh').read_text();soak=(SCRIPTS/'run-soak.sh').read_text()
        self.assertEqual(lib.count('PHASE3_ENABLED=1 PHASE4_ENABLED=1 TELEMETRY_PRETTY=0 exec'),2);self.assertNotIn('unset PHASE4_ENABLED',lib)
        self.assertIn('zone-flags.json',lib)
        self.assertIn('ZONE_FLAGS=(env PHASE3_ENABLED=1 PHASE4_ENABLED=1)',soak);self.assertNotIn('PHASE3_ENABLED="$CASTING"',soak)
        self.assertNotIn('env -u PHASE4_ENABLED',soak)
        # Review-3 P1-3: soak replay steps run with the API's overlay RULES_DIR.
        rules=soak.index('export RULES_DIR="$OUT/fixtures/data"')
        for step in ('export --zone 1 --epoch 1','check --file "$OUT/soak.nfr"','sim-gates.py" coverage','--items-evidence-out'):
            self.assertGreater(soak.index(step),rules,step)
        # Review-3 P1-2: the soak seeder writes pending kits for the V6 soak API in every mode.
        self.assertRegex(soak,r'NIGHTFALL_PHASE4_FIXTURE="\$ITEMS" PHASE4_ENABLED=1 \\\n\s+python3 "\$SIM_REPO/infra/scripts/seed-phase\$\{SEED_PHASE\}-soak.py"')

    def test_stage_manifests_are_validated(self):
        roles={name:r|{'account_id':'owner'} for name,r in fixture.phase4.profiles('phase4-equip').items()}
        plans=seed.plans_for('phase4-equip',roles)
        pre=seed.manifest('phase4-equip',roles,False,plans=plans)
        fixture.phase4.validate_seed_manifest(pre,'phase4-equip','owner','observer',REPO,False,'pre-admission')
        with self.assertRaises(ValueError):fixture.phase4.validate_seed_manifest(pre,'phase4-equip','owner','observer',REPO,False)
        granted=seed.manifest('phase4-equip',roles,False,'granted',plans,{'owner':{}})
        fixture.phase4.validate_seed_manifest(granted,'phase4-equip','owner','observer',REPO,False)
        with self.assertRaises(ValueError):fixture.phase4.validate_seed_manifest(granted|{'granted':{}},'phase4-equip','owner','observer',REPO,False)
        with self.assertRaises(ValueError):fixture.phase4.validate_seed_manifest(seed.manifest('phase4-equip',roles,True,plans=plans),'phase4-equip','owner','observer',REPO,False,'pre-admission')


if __name__=='__main__':unittest.main()
