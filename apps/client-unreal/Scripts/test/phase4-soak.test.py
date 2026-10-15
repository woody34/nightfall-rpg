#!/usr/bin/env python3
"""Eight role item workload and synthetic participation guards; no Unreal/API run."""
import copy
import importlib.util
import json
from pathlib import Path
import re
import tempfile
import unittest

SCRIPTS=Path(__file__).resolve().parents[1];REPO=SCRIPTS.parents[2]
def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
fixtures=load('soak4_fixtures',SCRIPTS/'soak-fixtures.py')
seed=load('soak4_seed',REPO/'infra/scripts/seed-phase4-soak.py')
grammar=load('soak4_grammar',SCRIPTS/'phase4-grammar.py')
variants=load('soak4_variants',SCRIPTS/'phase4-variants.py')
evidence=load('soak4_evidence',SCRIPTS/'phase4-evidence.py')
fixtures_contracts=load('soak4_contracts',SCRIPTS/'phase4-fixtures.py')


class Soak(unittest.TestCase):
    def test_eight_canonical_roles_have_keyed_enchant_swaps_autoshots_and_casting(self):
        with tempfile.TemporaryDirectory() as tmp:
            out=Path(tmp);fixtures.prepare(REPO,out,8,casting=True,items=True)
            manifest=json.loads((out/'roles.json').read_text());roles=seed.validated_roles(manifest)
            self.assertEqual(len(roles),8)
            identities=set();keys=set()
            for role in manifest['roles']:
                source=Path(role['scenario']).read_text();supplied=role['seed']
                identities.add(supplied['character_id'])
                self.assertEqual(Path(role['token_file']).stat().st_mode&0o777,0o600)
                self.assertIn('nf.Cast 1068 self',source);self.assertIn('nf.Cast 1177',source)
                self.assertIn('nf.Equip 1',source);self.assertIn('nf.Equip 6',source)
                # T30: the shots toggled are the role pack's own templates (D97 soulshot 1835).
                picks=fixtures_contracts.selection(REPO);held={r['template_id'] for r in supplied['inventory']}
                for shot in (picks['soulshot'],picks['spiritshot']):
                    self.assertIn(f'nf.AutoShot {shot} on',source);self.assertIn(shot,held)
                self.assertEqual((picks['soulshot'],picks['spiritshot']),(1835,2509))
                self.assertNotIn('5789',source)
                commands=re.findall(r'^nf.Enchant .+$',source,re.M);self.assertEqual(len(commands),1)
                self.assertTrue(grammar.command(commands[0].split()));keys.add(commands[0].split()[-1])
                count=sum(int(l.split('#',1)[0].split()[-1]) for l in source.splitlines() if l.startswith('nf.WaitFor '))
                budget=int(re.search(r'^nf.Within (\d+)$',source,re.M)[1]);self.assertGreaterEqual(budget,count+30);self.assertLessEqual(budget,600)
                moves=[tuple(map(int,m)) for m in re.findall(r'^nf.ClickMove (\d+) (\d+)$',source,re.M)]
                previous=tuple(supplied['position'])
                for move in moves:
                    self.assertLessEqual((move[0]-previous[0])**2+(move[1]-previous[1])**2,60**2)
                    self.assertFalse(116<=move[0]<=136 and 116<=move[1]<=136)
                    previous=move
            self.assertEqual(len(identities),8);self.assertEqual(len(keys),8)
            bad=copy.deepcopy(manifest);bad['roles'][0]['seed']['inventory'][0]['count']=2
            with self.assertRaises(ValueError):seed.validated_roles(bad)

    def test_items_requires_casting(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):fixtures.prepare(REPO,Path(tmp),8,items=True)

    def test_variants_remain_strict_and_live_pending(self):
        data=variants.validate();self.assertEqual(len(data['runtime_cases']),26)
        for row in data['runtime_cases']:self.assertIn('pending',row['execution'])
        self.assertEqual(len({r['id'] for r in data['runtime_cases']}),26)

    def test_armour_reconnect_distinguishes_retained_history_from_full_set_and_shield(self):
        paths=[SCRIPTS.parent/'Scenarios/4-armor-set.nfs',
               *(SCRIPTS.parent/'Scenarios/variants').glob('4-armor-enchant*.nfs')]
        for path in paths:
            pack=re.search(r'^# fixture: (\S+)$',path.read_text(),re.M)[1]
            equipment={r['slot']:r['template_id'] for r in variants.scenarios.fixtures.profiles(pack)['owner']['inventory'] if r['slot']}
            phase=next(p for p in evidence.phases(path) if p['name']=='active-incomplete-history-reconnect')
            previous=False;checkpoints=[]
            for line in path.read_text().splitlines():
                tokens=line.split()
                if not tokens:continue
                mutation=None
                if tokens[0]=='nf.Equip':
                    template=int(tokens[1]);slot=variants.scenarios.fixtures.catalogue()[template]['bodypart']
                    equipment[slot]=template
                    mutation='equip_shield' if slot=='lhand' else 'equip_chest'
                elif tokens[0]=='nf.Unequip':
                    equipment.pop(tokens[1],None);mutation='remove'
                complete=all(equipment.get(slot)==template for slot,template in [('chest',58),('legs',59),('head',47)])
                shield=equipment.get('lhand')==628
                if mutation:
                    previous=evidence.oracle.set_shield_history(previous,mutation,equipment.get('chest')==58,shield,complete,
                        required_part=tokens[0]=='nf.Unequip' and tokens[1] in ('chest','legs','head'))
                if tokens[0]=='nf.DropSocket':
                    if not checkpoints:
                        self.assertTrue(all(equipment.get(slot)==template for slot,template in phase['equipment'].items()))
                    checkpoints.append((previous,complete and shield))
            self.assertEqual(checkpoints,[(True,False),(False,False),(True,True)],path.name)
            self.assertIn('nf.WaitFor owner_stats_match_mark 3',path.read_text())
            self.assertIn('"active-incomplete-history-reconnect"',path.read_text())
            # The full_set&&shield reconnect mutation loses provider3543 at the first checkpoint.
            self.assertNotEqual([retained for retained,_ in checkpoints],[rebuilt for _,rebuilt in checkpoints])


if __name__=='__main__':unittest.main()
