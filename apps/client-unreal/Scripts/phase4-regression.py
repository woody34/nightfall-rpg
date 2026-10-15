#!/usr/bin/env python3
"""E6.3 regression runner (D114): Phase 0b-3 units on V6 and the E3.3 backfill matrix.

The committed inventory (`Scenarios/phase4-regression.pending.json`, `phase4-backfill.pending.json`)
lists every historical expectation and every backfill case; it carries no verdict. Verdicts come
only from evidence: `--suite ARTIFACTS` reads a run-sim-ci.py artifact tree (per unit: JUnit,
pipeline-state, recording + replay-check log, zone flags), `--backfill-results FILE` reads the
API's phase4_backfill integration-test results. Source-only math never attests a live run.
"""
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


# D119: third-class (tier 3) professions are not admissible characters (ClassState refuses
# tier > 2), so their backfill cases are out of scope: 58 professions x 3 states remain.
ADMISSIBLE_TIER=2


def professions():
    return [tomllib.loads(path.read_text()) for path in sorted((REPO/'packages/data/professions').glob('*.toml'))]


def inapplicable_ids():
    return {d['key']+'-'+life for d in professions() if d['tier']>ADMISSIBLE_TIER for life in ('alive','dead','never_admitted')}


def backfill_matrix():
    cases=[]
    for definition in professions():
        if definition['tier']>ADMISSIBLE_TIER:continue # D119
        base=definition['base_class_id']
        for life in ('alive','dead','never_admitted'):
            # Pre-admission migration inputs; descendants require real retained class history.
            cases.append(dict(id=definition['key']+'-'+life,class_id=definition['id'],base_class_id=base,
                              state=life,kit=kit_inputs(base),source=definition['source'],
                              required_gates=['migration_rerun','first_v6_admission','crash_resume','kit_once','allocator_no_collision'],
                              history_required=definition['tier']>0))
    return dict(schema_version=1,cases=cases)


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
            expectations.append(dict(line=line,before=source,after=source,reason=reason))
        entries.append(dict(scenario=str(path.relative_to(directory)),scenario_sha256=sha(path),fixture=pack,
                            role_inputs=roles,creation_commands=creation,
                            role_input_status='explicit seed roles' if roles else 'public creation/login inputs; captured admission load',
                            expectations=expectations))
    return dict(schema_version=2,policy='C15/C16 + D114: Phase 0b-3 units run on V6 production kits, no assertion relaxation',
                source_revisions=dict(datapack=oracle.DP_REV,game=oracle.GAME_REV),
                scenarios=entries,isolated_source_vectors=isolated_kit_stats(),
                historical_recordings={p.name:sha(p) for p in sorted((REPO/'apps/api/fixtures/sessions').glob('*.nfr'))})


REGENERATE=('python3 apps/client-unreal/Scripts/phase4-regression.py'
            ' --publish apps/client-unreal/Scenarios/phase4-regression.pending.json'
            ' --backfill apps/client-unreal/Scenarios/phase4-backfill.pending.json')
INVENTORY=SCRIPTS.parent/'Scenarios/phase4-regression.pending.json'
BACKFILL_INVENTORY=SCRIPTS.parent/'Scenarios/phase4-backfill.pending.json'
V6_FLAGS={'PHASE3_ENABLED':'1','PHASE4_ENABLED':'1'}


def unit_flags(unit_dir,scenario_dir):
    """Zone flags the unit's API ran with: owned-fixture manifest, else the shared-stack record.

    sim-lib provisions one owned fixture per unit under its FIRST scenario's folder
    (`$ARTIFACTS/<first scenario>/fixture`); run-sim-ci resolves it as `*/fixture`. Every
    scenario of the unit shares it, so resolve exactly one manifest per unit.

    Review-3 P2-3: the flags are the API's ATTESTED ones, `zone-flags.json` written by sim-lib
    after health from the API's own V6 bootstrap log line (with its rules hashes); a fixture
    manifest's intended flags must agree with that record.
    """
    manifests=sorted(unit_dir.glob('*/fixture/manifest.json'))
    if len(manifests)>1:raise ValueError(f'unit {unit_dir.name} has more than one fixture manifest')
    if manifests:
        intended=json.loads(manifests[0].read_text()).get('zone_flags')
        record=manifests[0].with_name('zone-flags.json')
    else:intended=None;record=unit_dir/'zone-flags.json'
    if not record.is_file():return None
    attested=json.loads(record.read_text())
    if intended is not None and intended!={k:attested.get(k) for k in intended}:
        raise ValueError('fixture manifest zone flags differ from the API-attested flags')
    return attested


def is_v6(record):
    return isinstance(record,dict) and {k:record.get(k) for k in V6_FLAGS}==V6_FLAGS and all(
        isinstance(record.get(name),str) and re.fullmatch(r'sha256:[0-9a-f]{64}',record[name])
        for name in ('phase4_item_rules_hash','phase4_skill_rules_hash'))


REPLAY_MATCH=re.compile(r'^zone \S+ epoch \d+: match\. ',re.M)
EPOCH_LINE=re.compile(r'^zone \S+ epoch \d+: ',re.M)


def replay_matched(text):
    """At least one epoch line, and every epoch line is a match (no later divergence)."""
    return bool(REPLAY_MATCH.search(text)) and len(EPOCH_LINE.findall(text))==len(REPLAY_MATCH.findall(text))


def suite_units(suite):
    """scenario stem -> (unit, status) from run-sim-ci.py summary.json."""
    summary=json.loads((suite/'summary.json').read_text())
    if summary.get('schema_version')!=1:raise ValueError('unknown suite summary schema')
    return {name:(row['unit'],row['status']) for row in summary['units'] for name in row['scenarios']}


def scenario_evidence(suite,units,scenario):
    """Verdict of one Phase 0b-3 scenario from a run-sim-ci.py artifact tree (never inferred).

    Verified iff the unit PASSed (quarantine is not verification), the unit's finalized
    pipeline verdict and the scenario's pipeline state are clean, every recording of the
    scenario (own and unit-shared) replays to a match, and the API ran with V6 zone flags.
    """
    stem=Path(scenario).stem
    if stem not in units:return dict(verified=False,problems=['unit not run'])
    unit,status=units[stem];unit_dir=suite/unit;folder=unit_dir/stem;problems=[]
    if status!='PASS':problems.append(f'unit status {status}')
    try:
        verdict=json.loads((unit_dir/'pipeline-verdict.json').read_text())
        rows=[r for r in verdict.get('scenarios',[]) if r.get('scenario')==stem]
        if verdict.get('completed') is not True or verdict.get('exit_code')!=0 or verdict.get('infrastructure_failures') \
                or len(rows)!=1 or rows[0].get('failure_kinds'):
            problems.append('unit pipeline verdict not clean')
    except (OSError,ValueError) as error:problems.append(f'unit pipeline verdict unreadable: {error}')
    try:
        state=json.loads((folder/'pipeline-state.json').read_text())
        if state.get('scenario')!=stem or state.get('failure_kinds') or state.get('infrastructure_failures') or state.get('process_exit_code')!=0:
            problems.append('scenario pipeline state not clean: '+','.join(state.get('failure_kinds',[])+state.get('infrastructure_failures',[])))
    except (OSError,ValueError) as error:problems.append(f'scenario pipeline state unreadable: {error}')
    recordings=sorted(folder.glob('*.nfr'))+sorted(unit_dir.glob('*.nfr'))
    if not recordings:problems.append('no recording')
    for nfr in recordings:
        log=nfr.with_suffix('.replay.log')
        if not log.is_file() or not replay_matched(log.read_text()):
            problems.append(f'replay check of {nfr.relative_to(suite)} did not report a match')
    try:flags=unit_flags(unit_dir,folder)
    except (OSError,ValueError) as error:flags=None;problems.append(f'zone flags unreadable: {error}')
    if not is_v6(flags):problems.append(f'not an attested V6 run (zone flags {flags})')
    return dict(verified=not problems,problems=problems,zone_flags=flags,
                recordings={str(n.relative_to(suite)):sha(n) for n in recordings})


def regression_verdict(inventory,suite):
    """Every inventoried Phase 0b-3 scenario must be verified on V6; failures are zone-lane findings."""
    rows=[];units=suite_units(suite) if suite else {}
    for entry in inventory['scenarios']:
        path=SCRIPTS.parent/'Scenarios'/entry['scenario']
        if sha(path)!=entry['scenario_sha256']:raise ValueError('inventory is stale for '+entry['scenario'])
        evidence=scenario_evidence(suite,units,entry['scenario']) if suite else dict(verified=False,problems=['suite not run'])
        rows.append(dict(scenario=entry['scenario'],expectations=len(entry['expectations']),**evidence))
    return dict(scenarios=rows,all_verified=bool(rows) and all(r['verified'] for r in rows))


def check_committed(inventory,matrix,paths=None):
    """The verdict is over the COMMITTED inventories; regenerating them must not change them."""
    for path,current in zip(paths or (INVENTORY,BACKFILL_INVENTORY),(inventory,matrix)):
        committed=json.loads(path.read_text())
        if committed!=current:
            fields=sorted(k for k in set(committed)|set(current) if committed.get(k)!=current.get(k))
            raise ValueError(f'committed inventory {path.name} is stale (differs in: {", ".join(fields)}); regenerate from the '
                             f'repo root with `{REGENERATE}` and commit both files')


def backfill_verdict(matrix,results):
    """API phase4_backfill results (agreed in api-inbox.md): every case, every required gate.

    D119: a result for a tier-3 id is out of scope whatever its gates (the API emits them
    all-false with a note, or marks them not_applicable, or omits them); `not_applicable`
    on an admissible case is not verification.
    """
    if results is None:return dict(cases=[dict(id=c['id'],verified=False,problems=['backfill results not supplied']) for c in matrix['cases']],all_verified=False)
    if results.get('schema_version')!=1 or not isinstance(results.get('cases'),list):raise ValueError('unknown backfill results schema')
    by_id={}
    for case in results['cases']:
        if case.get('id') in by_id:raise ValueError('duplicate backfill result '+str(case.get('id')))
        by_id[case.get('id')]=case
    rows=[]
    for case in matrix['cases']:
        result=by_id.pop(case['id'],None);problems=[]
        if result is None:problems.append('no result')
        else:
            gates=result.get('gates',{})
            if result.get('not_applicable') is True:problems.append('admissible case marked not_applicable')
            for gate in case['required_gates']:
                if gates.get(gate) is not True:problems.append('gate '+gate+' not passed')
            if result.get('class_id')!=case['class_id'] or result.get('state')!=case['state']:problems.append('case identity differs')
        rows.append(dict(id=case['id'],verified=not problems,problems=problems))
    out_of_scope=sorted(set(by_id)&inapplicable_ids())
    for identity in out_of_scope:by_id.pop(identity)
    if by_id:raise ValueError('backfill results name unknown cases: '+', '.join(sorted(map(str,by_id))))
    return dict(cases=rows,all_verified=all(r['verified'] for r in rows),out_of_scope=out_of_scope,
                source=results.get('source'),commit=results.get('commit'))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__,formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--publish',type=Path,help='write the expectation inventory (Scenarios/phase4-regression.pending.json)')
    parser.add_argument('--backfill',type=Path,help='write the backfill case inventory (Scenarios/phase4-backfill.pending.json)')
    parser.add_argument('--suite',type=Path,help='run-sim-ci.py artifact root of a full V6 suite run')
    parser.add_argument('--backfill-results',type=Path,help='API phase4_backfill results JSON')
    parser.add_argument('--out',type=Path,help='verdict report')
    parser.add_argument('--require-live',action='store_true',help='exit 1 unless every scenario and backfill case is verified')
    args=parser.parse_args()
    inventory=report();matrix=backfill_matrix()
    if args.publish:args.publish.write_text(json.dumps(inventory,indent=2,sort_keys=True)+'\n')
    if args.backfill:args.backfill.write_text(json.dumps(matrix,indent=2,sort_keys=True)+'\n')
    try:
        check_committed(inventory,matrix)
        regression=regression_verdict(inventory,args.suite.resolve() if args.suite else None)
        backfill=backfill_verdict(matrix,json.loads(args.backfill_results.read_text()) if args.backfill_results else None)
    except (ValueError,KeyError,OSError) as error:
        print('Phase4 regression failed: '+str(error),file=sys.stderr);sys.exit(2)
    verdict=dict(schema_version=1,regression=regression,backfill=backfill,
                 all_live_verified=regression['all_verified'] and backfill['all_verified'])
    if args.out:args.out.write_text(json.dumps(verdict,indent=2,sort_keys=True)+'\n')
    failed=[r['scenario'] for r in regression['scenarios'] if not r['verified']]
    print(f"Phase 0b-3 on V6: {len(regression['scenarios'])-len(failed)}/{len(regression['scenarios'])} verified; "
          f"backfill {sum(r['verified'] for r in backfill['cases'])}/{len(backfill['cases'])} verified")
    sys.exit(1 if args.require_live and not verdict['all_live_verified'] else 0)
