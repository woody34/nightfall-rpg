#!/usr/bin/env python3
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import unittest
from unittest.mock import patch
import os

SCRIPTS=Path(__file__).resolve().parents[1];REPO=SCRIPTS.parents[2]
def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
regression=load('regression4',SCRIPTS/'phase4-regression.py')
pending=load('pending4',REPO/'infra/scripts/phase4-pending-kits.py')
seed=load('regression4_seed',REPO/'infra/scripts/seed-phase3-fixture.py')


class Regression(unittest.TestCase):
    def test_report_lists_every_historical_assertion_without_relaxation(self):
        data=json.loads((SCRIPTS.parent/'Scenarios/phase4-regression.pending.json').read_text())
        files={p.relative_to(SCRIPTS.parent/'Scenarios').as_posix():p for p in (SCRIPTS.parent/'Scenarios').rglob('*.nfs') if re.match(r'(?:0b|1|2|3)-',p.name)}
        self.assertEqual(set(files),{r['scenario'] for r in data['scenarios']})
        self.assertNotIn('all_live_verified',data)
        for row in data['scenarios']:
            path=files[row['scenario']];self.assertEqual(row['scenario_sha256'],hashlib.sha256(path.read_bytes()).hexdigest())
            actual=[line for line in path.read_text().splitlines() if line.startswith(('nf.WaitFor ','nf.Expect '))]
            self.assertEqual(actual,[r['before'] for r in row['expectations']])
            for entry in row['expectations']:
                self.assertEqual(entry['before'],entry['after']);self.assertEqual(set(entry),{'line','before','after','reason'})
            self.assertNotIn('status',row)

    def test_pending_manifest_matches_accepted_migration_and_has_no_allocated_items(self):
        char='01970000-0000-7000-8000-000000000040'
        row=pending.manifest(char,10)
        self.assertEqual(row['manifest_hash'],'9e8c395654f2b4350c361c2862a4cdaddc2855f491d92e7c5fea72e4eb33c9de')
        self.assertEqual(row['payload']['items'][0],dict(template_id=6,count=1,equipped=True))
        self.assertNotIn('instance_id',json.dumps(row))
        roles=seed.contracts.profiles('phase3-trainer',REPO)
        roles['owner']['account_id']=char
        with patch.dict(os.environ,PHASE4_ENABLED='1'):
            sql=seed.seed_sql(roles)
        self.assertIn('INSERT INTO pending_item_requests(',sql)
        self.assertNotIn('INSERT INTO item_instances(',sql)
        self.assertEqual(sql.count('COMMIT;'),1)

    def test_isolated_math_and_backfill_inventory(self):
        data=json.loads((SCRIPTS.parent/'Scenarios/phase4-regression.pending.json').read_text())
        rows=data['isolated_source_vectors'];self.assertEqual(len(rows),12)
        human=next(r for r in rows if r['class_id']==10 and r['level']==20)
        self.assertEqual(human['before']['mAtk'],8697486);self.assertEqual(human['after']['mAtk'],12176480)
        self.assertEqual(human['before']['pDef'],58860000);self.assertEqual(human['after']['pDef'],63220000)
        cases=json.loads((SCRIPTS.parent/'Scenarios/phase4-backfill.pending.json').read_text())['cases']
        self.assertEqual({r['state'] for r in cases},{'alive','dead','never_admitted'})
        self.assertTrue(all('execution' not in r for r in cases))
        # D119: 58 admissible professions x 3 states; no tier-3 case is inventoried.
        self.assertEqual(len(cases),174);self.assertEqual(len({r['class_id'] for r in cases}),58)
        self.assertFalse({r['id'] for r in cases}&regression.inapplicable_ids());self.assertEqual(len(regression.inapplicable_ids()),93)
        self.assertEqual(json.loads((SCRIPTS.parent/'Scenarios/phase4-backfill.pending.json').read_text()),regression.backfill_matrix())


V6={'PHASE3_ENABLED':'1','PHASE4_ENABLED':'1'}
# What sim-lib writes after health from the API's V6 bootstrap line (review-3 P2-3).
ATTESTED=V6|dict(phase4_item_rules_hash='sha256:'+'b'*64,phase4_skill_rules_hash='sha256:'+'f'*64,evidence='api startup log')
MATCH='zone 1 epoch 1: match. 10 ticks replayed (10 recorded), 1 players, 5 outputs, 9 bytes compared, 0 digest-only, in 1.0 ms\n'


def write_suite(root,units,status='PASS',flags=ATTESTED,replay=MATCH,kinds=(),infra=(),fixture=True):
    """A run-sim-ci.py artifact tree: summary.json, unit verdicts, scenario state, recordings."""
    rows=[]
    for unit,names in units.items():
        folder=root/unit;folder.mkdir(parents=True)
        rows.append(dict(unit=unit,scenarios=names,status=status))
        (folder/'pipeline-verdict.json').write_text(json.dumps(dict(schema_version=1,completed=True,exit_code=0 if not kinds and not infra else 1,
            infrastructure_failures=list(infra),scenarios=[dict(scenario=n,failure_kinds=list(kinds)) for n in names])))
        if not fixture and flags is not None:(folder/'zone-flags.json').write_text(json.dumps(flags))
        for name in names:
            (folder/name).mkdir()
            (folder/name/'pipeline-state.json').write_text(json.dumps(dict(scenario=name,failure_kinds=list(kinds),
                process_exit_code=0 if not kinds else 1,infrastructure_failures=list(infra))))
            (folder/name/f'{name}.nfr').write_bytes(b'NFR'+name.encode())
            (folder/name/f'{name}.replay.log').write_text(replay)
            if fixture and name==names[0]:
                # Real layout: one owned fixture per unit, under its first scenario's folder.
                (folder/name/'fixture').mkdir()
                (folder/name/'fixture/manifest.json').write_text(json.dumps(dict(fixture='x',zone_flags=V6)))
                if flags is not None:(folder/name/'fixture/zone-flags.json').write_text(json.dumps(flags))
    (root/'summary.json').write_text(json.dumps(dict(schema_version=1,units=rows)))


class Runner(unittest.TestCase):
    """D114: verification comes only from V6 suite evidence and the API backfill results."""
    def setUp(self):
        import tempfile
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        self.inventory=dict(scenarios=[dict(scenario='2-a.nfs',scenario_sha256='s',expectations=[1]),
                                       dict(scenario='multi/3-pair-a.nfs',scenario_sha256='s',expectations=[]),
                                       dict(scenario='multi/3-pair-b.nfs',scenario_sha256='s',expectations=[])])
        self.units={'2-a':['2-a'],'3-pair':['3-pair-a','3-pair-b']}
        self.sha=patch.object(regression,'sha',lambda path:'s');self.sha.start()
    def tearDown(self):
        self.sha.stop();self.tmp.cleanup()

    def verdict(self,**kw):
        write_suite(self.root/'suite',self.units,**kw)
        return regression.regression_verdict(self.inventory,self.root/'suite')

    def test_clean_v6_suite_verifies_every_scenario(self):
        result=self.verdict();self.assertTrue(result['all_verified'],result)
        self.assertEqual(result['scenarios'][1]['zone_flags'],ATTESTED)

    def test_second_scenario_of_a_paired_fixture_unit_is_v6(self):
        result=self.verdict()
        pair=[r for r in result['scenarios'] if r['scenario'].startswith('multi/3-pair-')]
        self.assertEqual([(r['verified'],r['zone_flags']) for r in pair],[(True,ATTESTED),(True,ATTESTED)])
        self.assertFalse((self.root/'suite/3-pair/3-pair-b/fixture').exists())
        (self.root/'suite/3-pair/3-pair-b/fixture').mkdir()
        (self.root/'suite/3-pair/3-pair-b/fixture/manifest.json').write_text(json.dumps(dict(zone_flags=V6)))
        result=regression.regression_verdict(self.inventory,self.root/'suite')
        self.assertFalse(result['all_verified'])
        self.assertTrue(any('more than one fixture manifest' in p for p in result['scenarios'][1]['problems']))

    def test_api_startup_log_attests_v6(self):
        fixture=load('regression4_fixture',SCRIPTS/'phase2-fixture.py')
        log=SCRIPTS/'test/fixtures/phase4-v6-log/api-v6-startup.jsonl'
        record=fixture.api_zone_flags(log)
        self.assertEqual(record['phase4_item_rules_hash'],'sha256:b13bafb012d7c1cae7b503dbc8688b183f4d776b4f765f03951127d7ca10e411')
        self.assertTrue(regression.is_v6(record))
        plain=self.root/'plain.log'
        plain.write_text('INFO nightfall_api::zone_runtime: Phase3 bootstrap bundle validated with the Phase4 (V6) item extension '
                         'phase4_item_rules_hash=sha256:'+'1'*64+' phase4_skill_rules_hash=sha256:'+'2'*64+'\n')
        self.assertTrue(regression.is_v6(fixture.api_zone_flags(plain)))
        # T15: the repo-default TELEMETRY_PRETTY=1 layout (ANSI-coloured), captured from the i3 run.
        pretty=fixture.api_zone_flags(SCRIPTS/'test/fixtures/phase4-v6-log/api-v6-startup-pretty.log')
        self.assertEqual(pretty['phase4_skill_rules_hash'],'sha256:5c2378c9eab24f26fdef161c4965dc3567fc05eefe04186cdf095d84cad7f6d1')
        self.assertTrue(regression.is_v6(pretty))
        # Pretty multi-line layout: fields on the following lines, record ends at the next timestamp.
        plain.write_text('  \x1b[2m2026-10-15T01:06:34Z\x1b[0m \x1b[32m INFO\x1b[0m Phase3 bootstrap bundle validated with the Phase4 (V6) item extension\n'
                         '    \x1b[3mphase4_item_rules_hash\x1b[0m: sha256:'+'3'*64+'\n    \x1b[3mphase4_skill_rules_hash\x1b[0m: sha256:'+'4'*64+'\n'
                         '  2026-10-15T01:06:35Z  INFO nightfall-api starting\n')
        self.assertEqual(fixture.api_zone_flags(plain)['phase4_skill_rules_hash'],'sha256:'+'4'*64)
        # Hashes of a LATER record never complete a marker line that lacks them.
        plain.write_text('2026-10-15T01:06:34Z INFO Phase3 bootstrap bundle validated with the Phase4 (V6) item extension\n'
                         '2026-10-15T01:06:35Z INFO other phase4_item_rules_hash=sha256:'+'1'*64+' phase4_skill_rules_hash=sha256:'+'2'*64+'\n')
        self.assertIsNone(fixture.api_zone_flags(plain))
        lib=(SCRIPTS/'sim-lib.sh').read_text()
        self.assertEqual(lib.count('PHASE4_ENABLED=1 TELEMETRY_PRETTY=0 exec'),2)
        self.assertIn('has no V6 bootstrap line after health',lib)
        self.assertIn('TELEMETRY_PRETTY=0',(SCRIPTS/'run-soak.sh').read_text())
        plain.write_text('INFO Phase3 bootstrap bundle validated phase3_rules_hash=sha256:'+'1'*64+'\n') # V5 boot
        self.assertIsNone(fixture.api_zone_flags(plain))
        lib=(SCRIPTS/'sim-lib.sh').read_text()
        self.assertNotIn('printf \'{"PHASE3_ENABLED"',lib)
        self.assertIn('sim_record_zone_flags "${SIM_API_LOG:-}"',lib);self.assertIn('sim_record_zone_flags "$SIM_FIXTURE_DIR/api.log"',lib)

    def test_manifest_flags_must_agree_with_the_attested_record(self):
        write_suite(self.root/'suite',self.units)
        (self.root/'suite/2-a/2-a/fixture/manifest.json').write_text(json.dumps(dict(zone_flags={'PHASE3_ENABLED':'1'})))
        (self.root/'suite/2-a/2-a/fixture/zone-flags.json').write_text(json.dumps(ATTESTED|dict(PHASE3_ENABLED='0')))
        result=regression.regression_verdict(self.inventory,self.root/'suite')
        self.assertTrue(any('differ from the API-attested' in p for p in result['scenarios'][0]['problems']))

    def test_shared_stack_units_prove_v6_by_their_zone_flags_record(self):
        self.assertTrue(self.verdict(fixture=False)['all_verified'])

    def test_non_v6_failed_quarantined_or_diverged_units_are_findings(self):
        cases=[dict(flags={'PHASE3_ENABLED':'1'}),dict(flags=V6),dict(flags=None),dict(flags=None,fixture=False),
               dict(flags=ATTESTED|dict(phase4_item_rules_hash='b'*64)),dict(flags=ATTESTED|dict(PHASE4_ENABLED='0')),dict(status='QUARANTINED'),dict(kinds=['bot_expectation']),
               dict(infra=['replay_check']),dict(replay='zone 1 epoch 1: tick 4 output differs\n'),dict(replay='')]
        for i,case in enumerate(cases):
            self.root=Path(self.tmp.name)/str(i)
            result=self.verdict(**case)
            self.assertFalse(result['all_verified'],case)
            self.assertTrue(all(r['problems'] for r in result['scenarios']),case)

    def test_missing_unit_or_recording_is_not_verified(self):
        write_suite(self.root/'suite',{'2-a':['2-a']})
        result=regression.regression_verdict(self.inventory,self.root/'suite')
        self.assertFalse(result['all_verified']);self.assertEqual(result['scenarios'][1]['problems'],['unit not run'])
        (self.root/'suite/2-a/2-a/2-a.nfr').unlink()
        self.assertIn('no recording',regression.regression_verdict(self.inventory,self.root/'suite')['scenarios'][0]['problems'])
        self.assertFalse(regression.regression_verdict(self.inventory,None)['all_verified'])

    def test_committed_inventory_staleness_and_replay_divergence(self):
        first=self.root/'a.json';second=self.root/'b.json'
        first.write_text(json.dumps(dict(x=1)));second.write_text(json.dumps(dict(y=1)))
        regression.check_committed(dict(x=1),dict(y=1),(first,second))
        with self.assertRaisesRegex(ValueError,'a.json is stale'):regression.check_committed(dict(x=2),dict(y=1),(first,second))
        self.assertTrue(regression.replay_matched(MATCH))
        self.assertFalse(regression.replay_matched(MATCH+'zone 1 epoch 2: tick 4 output differs\n'))
        self.assertFalse(regression.replay_matched('error: load failed\n'))
        self.assertFalse(self.verdict(replay=MATCH+'zone 1 epoch 2: tick 4 output differs\n')['all_verified'])

    def test_stale_inventory_is_refused(self):
        write_suite(self.root/'suite',self.units)
        self.inventory['scenarios'][0]['scenario_sha256']='other'
        with self.assertRaisesRegex(ValueError,'stale'):regression.regression_verdict(self.inventory,self.root/'suite')


class Backfill(unittest.TestCase):
    MATRIX=dict(schema_version=1,cases=[dict(id='human_fighter-alive',class_id=0,state='alive',required_gates=['kit_once','crash_resume']),
                                        dict(id='human_fighter-dead',class_id=0,state='dead',required_gates=['kit_once'])])

    def results(self,**gates):
        return dict(schema_version=1,source='apps/api/tests/phase4_backfill.rs',commit='abc',
                    cases=[dict(id='human_fighter-alive',class_id=0,state='alive',gates=dict(kit_once=True,crash_resume=True)|gates),
                           dict(id='human_fighter-dead',class_id=0,state='dead',gates=dict(kit_once=True))])

    def test_every_case_and_gate_must_pass(self):
        self.assertTrue(regression.backfill_verdict(self.MATRIX,self.results())['all_verified'])
        failed=regression.backfill_verdict(self.MATRIX,self.results(crash_resume=False))
        self.assertFalse(failed['all_verified']);self.assertEqual(failed['cases'][0]['problems'],['gate crash_resume not passed'])
        self.assertFalse(regression.backfill_verdict(self.MATRIX,None)['all_verified'])
        missing=self.results();missing['cases'].pop()
        self.assertEqual(regression.backfill_verdict(self.MATRIX,missing)['cases'][1]['problems'],['no result'])
        wrong=self.results();wrong['cases'][1]['state']='alive'
        self.assertFalse(regression.backfill_verdict(self.MATRIX,wrong)['all_verified'])

    def test_unknown_duplicate_and_unversioned_results_are_refused(self):
        extra=self.results();extra['cases'].append(dict(id='elf-alive',gates={}))
        with self.assertRaisesRegex(ValueError,'unknown cases'):regression.backfill_verdict(self.MATRIX,extra)
        dup=self.results();dup['cases'].append(dup['cases'][0])
        with self.assertRaisesRegex(ValueError,'duplicate'):regression.backfill_verdict(self.MATRIX,dup)
        with self.assertRaisesRegex(ValueError,'schema'):regression.backfill_verdict(self.MATRIX,dict(schema_version=2,cases=[]))

    def test_not_applicable_admissible_case_and_unknown_ids(self):
        marked=self.results();marked['cases'][0]['not_applicable']=True
        self.assertEqual(regression.backfill_verdict(self.MATRIX,marked)['cases'][0]['problems'],['admissible case marked not_applicable'])
        tier3=sorted(regression.inapplicable_ids())[0]
        extra=self.results();extra['cases'].append(dict(id=tier3,class_id=0,state='alive',gates={},note='third-class'))
        result=regression.backfill_verdict(self.MATRIX,extra)
        self.assertTrue(result['all_verified']);self.assertEqual(result['out_of_scope'],[tier3])

    def test_api_session17_results_verify_every_admissible_case(self):
        # Written by apps/api/tests/phase4_backfill.rs (PHASE4_BACKFILL_RESULTS); commit pinned.
        results=json.loads((SCRIPTS/'test/fixtures/phase4-backfill/api-s17-results.json').read_text())
        self.assertEqual((results['source'],results['commit']),('apps/api/tests/phase4_backfill.rs','40ecdab06d504596f8296a9588784091e291bc9a'))
        result=regression.backfill_verdict(regression.backfill_matrix(),results)
        self.assertTrue(result['all_verified'],[c for c in result['cases'] if not c['verified']][:3])
        self.assertEqual((len(result['cases']),len(result['out_of_scope'])),(174,93))
        self.assertEqual(set(result['out_of_scope']),regression.inapplicable_ids())
        # A failed admissible gate still fails; dropping an admissible case is "no result".
        broken=copy.deepcopy(results);broken['cases'][0]['gates']['kit_once']=False
        self.assertFalse(regression.backfill_verdict(regression.backfill_matrix(),broken)['all_verified'])
        missing=copy.deepcopy(results);missing['cases']=[c for c in missing['cases'] if c['id']!='abyss_walker-alive']
        self.assertFalse(regression.backfill_verdict(regression.backfill_matrix(),missing)['all_verified'])
        # Tier-3 ids may also be absent.
        absent=copy.deepcopy(results);absent['cases']=[c for c in absent['cases'] if c['id'] not in regression.inapplicable_ids()]
        self.assertTrue(regression.backfill_verdict(regression.backfill_matrix(),absent)['all_verified'])

    def test_runner_source_has_no_hard_coded_verdict(self):
        source=(SCRIPTS/'phase4-regression.py').read_text()
        self.assertNotIn('all_live_verified=False',source);self.assertNotIn("'pending V6",source)


if __name__=='__main__':unittest.main()
