#!/usr/bin/env python3
"""Additive required item coverage; preserve every historical observed contract field."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import sys

SCRIPTS=Path(__file__).resolve().parent
REPO=SCRIPTS.parents[2]
def load(name,file):
    spec=importlib.util.spec_from_file_location(name,SCRIPTS/file);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
contract=load('coverage4_contract','sim-contract.py')
gates=load('coverage4_transitions','sim-gates.py')


def required_wire():
    source=(REPO/'packages/proto/nightfall/v1/world.proto').read_text()
    intent_source=re.search(r'message ClientMessage \{.*?oneof intent \{(.*?)\n  \}',source,re.S)[1]
    intents=dict(re.findall(r'^    \w+ (\w+) = (1[9]|2[0-8]);$',intent_source,re.M))
    event_source=re.search(r'message WorldEvent \{.*?oneof event \{(.*?)\n  \}',source,re.S)[1]
    events=dict(re.findall(r'^    \w+ (\w+) = (2[6-9]|3[0-3]);$',event_source,re.M))
    return dict(schema_version=1,kind='required future observations, not baseline evidence',
                intents=intents,events=events,
                item_operations=['grant','starter','backfill','equip','unequip','consume','enchant','destroy','crystallize','refine','cancel_refine','exchange'],
                enchant_outcomes=['success','safe_failed','blessed_reset','destroyed'],
                refine_outcomes=['applied','removed'],shot_payments=['soul','spirit','blessed_spirit'])


def check(contract_path,transitions_path,baseline_path,required):
    current=contract.read(contract_path);transitions=gates.read_transitions(transitions_path)
    errors=contract.regressions(current,contract.read(baseline_path))
    if transitions['schema_version']!=4:raise ValueError('V6 schema4 item transitions required')
    for group in ('intents','events'):
        for name,tag in required[group].items():
            if int(current['counts'][group].get(name,0))<1:errors.append('missing item wire '+group+'.'+name)
            # wire_numbers catalogs use integer/string tags; descriptor counts do not prove renumbering safety.
    for group in ('item_operations','enchant_outcomes','refine_outcomes','shot_payments'):
        for name in required[group]:
            if int(transitions[group].get(name,0))<1:errors.append('missing item transition '+group+'.'+name)
    for field in gates.V4_SCALARS:
        if int(transitions[field])<1:errors.append('missing '+field)
    return dict(schema_version=1,passed=not errors,errors=errors,
                historical_contract_preserved=not contract.regressions(current,contract.read(baseline_path)))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--publish',type=Path);parser.add_argument('--contract',type=Path)
    parser.add_argument('--transitions',type=Path);parser.add_argument('--out',type=Path)
    parser.add_argument('--baseline',type=Path,default=SCRIPTS.parent/'Scenarios/coverage.contract.baseline.json')
    args=parser.parse_args()
    try:
        if args.publish:args.publish.write_text(json.dumps(required_wire(),indent=2,sort_keys=True)+'\n');code=0
        else:
            result=check(args.contract,args.transitions,args.baseline,required_wire())
            args.out.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n');code=int(not result['passed'])
    except (ValueError,TypeError,KeyError,OSError) as error:print(str(error),file=sys.stderr);code=2
    sys.exit(code)
