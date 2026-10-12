#!/usr/bin/env python3
"""Validate eight-client gear/casting inputs before owned V6 admission; no live bypass."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
from uuid import UUID

spec=importlib.util.spec_from_file_location('phase4_soak_seed',Path(__file__).with_name('seed-phase4-fixture.py'))
seed=importlib.util.module_from_spec(spec);spec.loader.exec_module(seed)


def validated_roles(manifest):
    if manifest.get('schema_version')!=1 or manifest.get('casting') is not True or manifest.get('items') is not True or not isinstance(manifest.get('roles'),list) or not 1<=len(manifest['roles'])<=8:
        raise ValueError('Phase4 soak requires 1..8 explicit item/casting roles')
    result={};accounts=set();characters=set()
    for index,role in enumerate(manifest['roles']):
        supplied=role['seed'];account=str(UUID(supplied['account_id']));character=str(UUID(supplied['character_id']))
        expected=seed.contracts.soak_profile(seed.REPO,account,character,index,len(manifest['roles']))
        if role['name']!=f'client-{index+1:02}' or supplied!=expected or account in accounts or character in characters:
            raise ValueError('noncanonical/duplicate Phase4 soak inputs')
        result['role'+chr(ord('A')+index)]=expected;accounts.add(account);characters.add(character)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest',required=True,type=Path);parser.add_argument('--dry-run',action='store_true')
    parser.add_argument('--compose-project');parser.add_argument('--compose-file',type=Path)
    args=parser.parse_args()
    try:
        if os.environ.get('NIGHTFALL_PHASE4_FIXTURE')!='1':raise ValueError('explicit Phase4 fixture opt-in required')
        seed.guarded_database(os.environ.get('DATABASE_URL',''))
        roles=validated_roles(json.loads(args.manifest.read_text()));sql=seed.seed_sql(roles)
        if args.dry_run:print(sql)
        else:raise ValueError('Phase4 soak admission transport pending E2/E3; no writes before typed low-range sink')
        print(json.dumps(seed.manifest('phase4-soak',roles,args.dry_run),sort_keys=True))
    except (ValueError,KeyError,TypeError) as error:parser.error(str(error))
