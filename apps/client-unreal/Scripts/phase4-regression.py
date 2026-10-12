#!/usr/bin/env python3
"""Inventory every historical expectation; source-only math never attests a live V6 run."""
import argparse
from functools import lru_cache
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys
import tomllib

SCRIPTS=Path(__file__).resolve().parent
REPO=SCRIPTS.parents[2]
sys.path.insert(0,str(REPO/'packages/data/scripts'))
import item_oracle as oracle
spec=importlib.util.spec_from_file_location('regression4_fixtures',SCRIPTS/'phase4-fixtures.py')
fixtures=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixtures)


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def kit_inputs(class_id):
    kit=next(k for k in fixtures.table('starter_kits')['kits'] if k['class_id']==class_id)
    return dict(source_class_id=class_id,items=[{k:r[k] for k in ('template_id','count','equipped')} for r in kit['items']])


def pending_kit(character,class_id):
    # Match migration000011 exactly; no row identity/instance allocation is invented.
    payload=kit_inputs(class_id)
    manifest_hash=hashlib.sha256(json.dumps(payload,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    request=hashlib.md5(('nightfall.phase4.backfill.1|'+character).encode()).hexdigest()
    from uuid import UUID
    return dict(request_id=str(UUID(request)),kind='backfill',manifest_version=1,manifest_hash=manifest_hash,
                request_fingerprint='v1|backfill|'+character+'|'+manifest_hash,payload=payload)


def backfill_matrix():
    cases=[]
    for path in sorted((REPO/'packages/data/professions').glob('*.toml')):
        definition=tomllib.loads(path.read_text());base=definition['base_class_id']
        for life in ('alive','dead','never_admitted'):
            # Pre-admission migration inputs; descendants require real retained class history.
            cases.append(dict(id=definition['key']+'-'+life,class_id=definition['id'],base_class_id=base,
                              state=life,kit=kit_inputs(base),source=definition['source'],
                              required_gates=['migration_rerun','first_v6_admission','crash_resume','kit_once','allocator_no_collision'],
                              history_required=definition['tier']>0,execution='pending E3 adapter/live fixtures'))
    return dict(schema_version=1,cases=cases,execution='not executed')


def isolated_kit_stats():
    source=oracle.Source();rows=[]
    classes=[('HumanFighter',0),('HumanMystic',10),('OrcFighter',44),('DwarvenFighter',53)]
    for name,class_id in classes:
        for level in (1,20,40):
            equipment=[]
            for seq,row in enumerate(kit_inputs(class_id)['items']):
                if row['equipped']:
                    slot=fixtures.catalogue()[row['template_id']]['bodypart']
                    equipment.append([row['template_id'],0,{'lrhand':'rhand','fullarmor':'chest'}.get(slot,slot),seq])
            class_file=oracle.D+f'stats/chars/baseStats/{name}.xml'
            before=oracle.gear_stats(source,class_file,level,[[2369,0,'rhand',0]])
            after=oracle.gear_stats(source,class_file,level,equipment)
            weight=sum(fixtures.catalogue()[r['template_id']]['weight']*r['count'] for r in kit_inputs(class_id)['items'])
            con=after['base_stats']['CON'];maximum=oracle.max_load(source.bonuses['CON'][con])
            rows.append(dict(class_id=class_id,level=level,scope='isolated no passives/effects, not final live fixture stats',
                             inputs=dict(class_file=class_file,equipment_before=[[2369,0,'rhand',0]],equipment_after=equipment),
                             before={k:before['q'][k] for k in ('pAtk','mAtk','pDef','mDef','maxMp')},
                             after={k:after['q'][k] for k in ('pAtk','mAtk','pDef','mDef','maxMp')},
                             total_kit_weight=weight,max_load=maximum,overload=oracle.overload(weight,maximum),
                             citations=['DP@3ca488dd/stats/initialEquipment.xml:4-109','G@abfde049/model/stats/functions/formulas/FuncPDefMod.java:44-70','G@abfde049/model/stats/functions/formulas/FuncMDefMod.java:43-64']))
    return rows


def report():
    directory=SCRIPTS.parent/'Scenarios';entries=[]
    for path in sorted(directory.rglob('*.nfs')):
        if not re.match(r'(?:0b|1|2|3)-',path.name):continue
        text=path.read_text();pack=re.search(r'^# fixture: (\S+)$',text,re.M)
        pack=pack[1] if pack else 'default';roles=None
        if pack in fixtures.phase3.PHASE3:
            roles=fixtures.phase3.profiles(pack,REPO,pack in fixtures.phase3.PAIRED or path.stem.endswith(('-a','-b')))
        creation=re.findall(r'^nf.CreateClass (.+)$',text,re.M)
        expectations=[]
        for line,source in enumerate(text.splitlines(),1):
            if not source.startswith(('nf.Expect ','nf.WaitFor ')):continue
            predicate=source.split()[1]
            reason=('identity/receipt/learning/progression assertion retained; verify actual V6 context'
                    if predicate in ('learned_rank','own_sp','skill_learned_events','learn_receipt_matches_first','connected','in_world','reconnects')
                    else 'existing assertion retained; equipment/maxima/position/RNG/cadence context requires V6 export and oracle')
            expectations.append(dict(line=line,before=source,after=source,reason=reason,
                                     verification='pending V6 run; unchanged text is not outcome equivalence'))
        entries.append(dict(scenario=str(path.relative_to(directory)),scenario_sha256=sha(path),fixture=pack,
                            role_inputs=roles,creation_commands=creation,
                            role_input_status='explicit seed roles' if roles else 'public creation/login inputs; await captured admission load',
                            expectations=expectations,status='pending V6 replay/export/oracle receipt'))
    return dict(schema_version=1,policy='C15/C16: V6 production kits, no assertion relaxation',
                source_revisions=dict(datapack=oracle.DP_REV,game=oracle.GAME_REV),
                scenarios=entries,isolated_source_vectors=isolated_kit_stats(),
                historical_recordings={p.name:sha(p) for p in sorted((REPO/'apps/api/fixtures/sessions').glob('*.nfr'))},
                all_live_verified=False)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',required=True,type=Path);parser.add_argument('--backfill',type=Path)
    parser.add_argument('--require-live',action='store_true')
    args=parser.parse_args()
    result=report();args.out.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
    if args.backfill:args.backfill.write_text(json.dumps(backfill_matrix(),indent=2,sort_keys=True)+'\n')
    print(f"Historical scenarios/variants:{len(result['scenarios'])}; isolated kit vectors:{len(result['isolated_source_vectors'])}; live verification:PENDING")
    sys.exit(1 if args.require_live else 0)
