#!/usr/bin/env python3
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
        self.assertFalse(data['all_live_verified'])
        for row in data['scenarios']:
            path=files[row['scenario']];self.assertEqual(row['scenario_sha256'],hashlib.sha256(path.read_bytes()).hexdigest())
            actual=[line for line in path.read_text().splitlines() if line.startswith(('nf.WaitFor ','nf.Expect '))]
            self.assertEqual(actual,[r['before'] for r in row['expectations']])
            for entry in row['expectations']:
                self.assertEqual(entry['before'],entry['after']);self.assertIn('pending',entry['verification'])

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

    def test_isolated_math_and_backfill_cases_are_explicitly_pending(self):
        data=json.loads((SCRIPTS.parent/'Scenarios/phase4-regression.pending.json').read_text())
        rows=data['isolated_source_vectors'];self.assertEqual(len(rows),12)
        human=next(r for r in rows if r['class_id']==10 and r['level']==20)
        self.assertEqual(human['before']['mAtk'],8697486);self.assertEqual(human['after']['mAtk'],12176480)
        self.assertEqual(human['before']['pDef'],58860000);self.assertEqual(human['after']['pDef'],63220000)
        cases=json.loads((SCRIPTS.parent/'Scenarios/phase4-backfill.pending.json').read_text())['cases']
        self.assertEqual({r['state'] for r in cases},{'alive','dead','never_admitted'})
        self.assertTrue(all('pending' in r['execution'] for r in cases))


if __name__=='__main__':unittest.main()
