#!/usr/bin/env python3
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPTS=Path(__file__).resolve().parents[1]
def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
p4=load('coverage4',SCRIPTS/'phase4-coverage.py')


class Coverage(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.root=Path(self.tmp.name)
        self.data=json.loads((SCRIPTS/'test/fixtures/phase2-native-reconnect.transitions.json').read_text())
        self.data['schema_version']=4
        for name,(states,reachable) in p4.gates.V3_REACHABLE.items():
            self.data[name]=[dict(**{'from':a,'to':b},reachable=(a,b) in reachable,count=1 if (a,b) in reachable else 0) for a in sorted(states) for b in sorted(states)]
        for name in p4.gates.V3_COUNTERS:self.data[name]={}
        for name in p4.gates.V3_SCALARS:self.data[name]=1
        required=p4.required_wire()
        for name in p4.gates.V4_COUNTERS:self.data[name]={key:1 for key in required[name]}
        for name in p4.gates.V4_SCALARS:self.data[name]=1

    def test_wire_names_are_exact_and_not_cross_contaminated(self):
        required=p4.required_wire()
        self.assertEqual(len(required['intents']),10);self.assertEqual(len(required['events']),8)
        self.assertEqual(required['intents']['crystallize_item'],'28')
        self.assertNotIn('crystallize_item',required['events'])
        self.assertEqual(required,json.loads((SCRIPTS.parent/'Scenarios/phase4-coverage.required.json').read_text()))
        # D161: success_no_increment is a valid producer label but not a Phase 4 suite requirement.
        self.assertEqual(required['enchant_outcomes'],['success','safe_failed','blessed_reset','destroyed'])
        self.assertIn('success_no_increment',p4.gates.V4_LABELS['enchant_outcomes'])
        self.assertTrue(set(required['enchant_outcomes'])<p4.gates.V4_LABELS['enchant_outcomes'])
        for group in ('item_operations','refine_outcomes','shot_payments'):self.assertEqual(set(required[group]),p4.gates.V4_LABELS[group])

    def test_inherited_fields_missing_reachability_and_legacy_injection_fail(self):
        path=self.root/'current.json';path.write_text(json.dumps(self.data));p4.gates.read_transitions(path)
        for field in (*p4.gates.V3_TABLES,*p4.gates.V3_COUNTERS,*p4.gates.V3_SCALARS,*p4.gates.V4_COUNTERS,*p4.gates.V4_SCALARS):
            bad=copy.deepcopy(self.data);del bad[field];path.write_text(json.dumps(bad))
            with self.assertRaises((ValueError,KeyError),msg=field):p4.gates.read_transitions(path)
        bad=copy.deepcopy(self.data);bad['cast_states'][0]['reachable']=True;path.write_text(json.dumps(bad))
        with self.assertRaises(ValueError):p4.gates.read_transitions(path)
        bad=copy.deepcopy(self.data);bad['schema_version']=3;path.write_text(json.dumps(bad))
        with self.assertRaises(ValueError):p4.gates.read_transitions(path)

    def test_required_presence_and_frozen_tags_reject_missing_or_renumbered_observations(self):
        baseline=SCRIPTS.parent/'Scenarios/coverage.contract.baseline.json'
        current=json.loads(baseline.read_text());required=p4.required_wire()
        for group in ('intents','events'):
            for name,tag in required[group].items():
                current['counts'][group][name]='1';current['wire_numbers'][group][name]=int(tag)
        wire=self.root/'wire.json';transitions=self.root/'transitions.json'
        wire.write_text(json.dumps(current));transitions.write_text(json.dumps(self.data))
        self.assertTrue(p4.check(wire,transitions,baseline,required)['passed'])
        current['wire_numbers']['intents']['equip_item']=20;wire.write_text(json.dumps(current))
        self.assertFalse(p4.check(wire,transitions,baseline,required)['passed'])
        current['wire_numbers']['intents']['equip_item']=19;current['counts']['events']['inventory_sync']='0';wire.write_text(json.dumps(current))
        self.assertFalse(p4.check(wire,transitions,baseline,required)['passed'])
        bad=copy.deepcopy(self.data);bad['item_operations']['item_1234']=1;transitions.write_text(json.dumps(bad))
        with self.assertRaises(ValueError):p4.gates.read_transitions(transitions)

    def test_merge_preserves_all_old_and_new_fields(self):
        paths=[]
        for i in range(2):
            path=self.root/f'{i}.json';path.write_text(json.dumps(self.data));paths.append(path)
        merged=p4.gates.merge_transitions(paths)
        for field in p4.gates.V4_SCALARS:self.assertEqual(merged[field],2)
        for field in p4.gates.V3_SCALARS:self.assertEqual(merged[field],2)
        self.assertEqual(merged['item_operations']['equip'],2)


if __name__=='__main__':unittest.main()
