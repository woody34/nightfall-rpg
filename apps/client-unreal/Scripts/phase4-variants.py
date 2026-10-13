#!/usr/bin/env python3
"""Publish and audit source-bound runtime variants and separately pending precision cases."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re

SCRIPTS=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('variants4_scenarios',SCRIPTS/'phase4-scenarios.py')
scenarios=importlib.util.module_from_spec(spec);spec.loader.exec_module(scenarios)
DIRECTORY=SCRIPTS.parent/'Scenarios'


def armor_body(config):
    row,templates=scenarios.fixtures.armor_set()
    items=scenarios.fixtures.catalogue()
    by_slot={items[i]['bodypart']:i for i in templates}
    chest,shield=by_slot['chest'],by_slot['lhand']
    required=[i for i in templates if i!=shield]
    removed=next(i for i in required if i!=chest)
    slot=items[removed]['bodypart']
    shield_skills=[r['id'] for r in row['shield_skills']]
    six_skills=[r['id'] for r in row['enchant6_skills']]
    phase=lambda name,formula,**selectors:'# item-phase: '+json.dumps(dict(name=name,formula=formula,**selectors),separators=(',',':'))
    context={'set_id':row['id']}
    starter={r['slot']:r['template_id'] for r in scenarios.fixtures.profiles('phase4-armor')['owner']['inventory'] if r['slot']}
    declarations=[phase('chest-then-shield','set_shield_history',context=context,inputs={'mutation':'equip_shield','complete':False},actual=True,
                         mutation={'operation':'equip','template':shield,'slot':'lhand'}),
                  phase('shield-then-chest','set_shield_history',context=context,inputs={'mutation':'equip_chest','complete':False},actual=False,
                         mutation={'operation':'equip','template':chest,'slot':'chest'}),
                  # D111: gear_stats facts carry context {event: equip|unequip|enchant} under the
                  # command's resolution (no set_id, no admission fact), so the history phases bind
                  # the equip that creates each state; reconnects then assert unchanged stats.
                  phase('inactive-history-reconnect','gear_stats',context={'event':'equip'},absent_providers=shield_skills,
                        equipment={'chest':chest,'lhand':shield},mutation={'operation':'equip','template':chest,'slot':'chest'}),
                  phase('active-incomplete-history-reconnect','gear_stats',context={'event':'equip'},providers=shield_skills,
                        equipment={**{items[i]['bodypart']:starter.get(items[i]['bodypart']) for i in required if i!=chest},'chest':chest,'lhand':shield},
                        mutation={'operation':'equip','template':shield,'slot':'lhand'}),
                  phase('active-shield-provider','gear_stats',context={'event':'equip'},providers=shield_skills,
                         mutation={'operation':'equip','template':shield,'slot':'lhand'}),
                  phase('removed-shield-provider','gear_stats',context={'event':'unequip'},absent_providers=shield_skills,
                         mutation={'operation':'unequip','template':shield,'slot':'lhand'})]
    body=['nf.Login','nf.WaitFor connected 15','nf.EnterWorld','nf.WaitFor in_world 15','nf.WaitFor item_baseline 3',
          f'nf.Equip {chest}',f'nf.WaitFor equipped chest == {chest} 3','nf.MarkItemState',
          f'nf.Equip {shield}',f'nf.WaitFor equipped lhand == {shield} 3','nf.WaitFor inventory_revision_since_mark >= 1 3',
          'nf.WaitFor owner_stat_delta max_hp > 0 3','nf.MarkItemState',
          'nf.Unequip lhand','nf.WaitFor equipped lhand == 0 3','nf.WaitFor owner_stat_delta max_hp < 0 3',
          f'nf.Equip {shield}',f'nf.WaitFor equipped lhand == {shield} 3','nf.WaitFor owner_stat_delta max_hp == 0 3',
          'nf.MarkItemState','nf.DropSocket','nf.WaitFor reconnects >= 1 15','nf.WaitFor item_baseline 15',
          'nf.WaitFor owner_stats_match_mark 3',
          # Mark the active incomplete state; reverse order reaches identical paperdoll without the provider.
          'nf.MarkItemState','nf.Unequip chest','nf.WaitFor equipped chest == 0 3',
          'nf.Unequip lhand','nf.WaitFor equipped lhand == 0 3',
          f'nf.Equip {shield}',f'nf.WaitFor equipped lhand == {shield} 3',
          f'nf.Equip {chest}',f'nf.WaitFor equipped chest == {chest} 3','nf.WaitFor owner_stat_delta max_hp < 0 3',
          'nf.MarkItemState','nf.DropSocket','nf.WaitFor reconnects >= 2 15','nf.WaitFor item_baseline 15',
          'nf.WaitFor owner_stats_match_mark 3']
    for i in required:
        if i==chest:continue
        concrete=items[i]['bodypart']
        body += [f'nf.Equip {i}',f'nf.WaitFor equipped {concrete} == {i} 3']
    body += ['nf.MarkItemState','nf.DropSocket','nf.WaitFor reconnects >= 3 15','nf.WaitFor item_baseline 15','nf.WaitFor owner_stats_match_mark 3']
    if config.get('enchant')==6:
        # D111: producer gear facts carry {event} only (no set_id, no admission fact).
        declarations += [phase('enchant6-installed','gear_stats',context={'event':'equip'},providers=six_skills),
                         phase('enchant6-reconnect','gear_stats',context={'event':'equip'},providers=six_skills+shield_skills),
                         phase('enchant6-removed','gear_stats',context={'event':'unequip'},absent_providers=six_skills,
                               mutation={'operation':'unequip','template':removed,'slot':slot})]
        body += ['nf.Expect enchant_level chest == 6']
    body += [f'nf.Unequip {slot}',f'nf.WaitFor equipped {slot} == 0 3',f'nf.Expect inventory_has {removed} == 1']
    if config.get('enchant')==4:
        shirt=scenarios.fixtures.enchant4_item();sid=shirt['template_id'];providers=[r['id'] for r in shirt['enchant4_skills']]
        declarations += [phase('enchant4-installed','gear_stats',providers=providers,mutation={'operation':'equip','template':sid,'slot':'under'}),
                         phase('enchant4-removed','gear_stats',absent_providers=providers,mutation={'operation':'unequip','template':sid,'slot':'under'})]
        body += ['nf.MarkItemState',f'nf.Equip {sid} under',f'nf.WaitFor equipped under == {sid} 3',
                 'nf.WaitFor owner_stat_delta max_hp > 0 3','nf.Expect enchant_level under == 4',
                 'nf.MarkItemState','nf.Unequip under','nf.WaitFor equipped under == 0 3','nf.WaitFor owner_stat_delta max_hp < 0 3']
    budget=sum(int(l.split()[-1]) for l in body if l.startswith('nf.WaitFor '))+30
    return declarations+body+[f'nf.Within {budget}']


def generate(directory=DIRECTORY):
    variants=directory/'variants';variants.mkdir(exist_ok=True)
    rows=[]
    for pack,config in sorted(scenarios.fixtures.VARIANT_PACKS.items()):
        base=config['base']
        canonical={'phase4-starter':'4-starter-kit.nfs','phase4-equip':'4-equip-weapon.nfs' if config.get('equipment_case') else '4-unequip-fists.nfs','phase4-armor':'4-armor-set.nfs','phase4-bow':'4-bow-arrows.nfs'}[base]
        text=(directory/canonical).read_text();name='4-'+pack.removeprefix('phase4-').replace('_','-')
        text=text.replace('# fixture: '+base,'# fixture: '+pack+'\n# variant: '+name)
        if base=='phase4-starter':
            role=scenarios.fixtures.profiles(pack)['owner']
            body=[]
            for row in role['inventory']:
                body.append(f"nf.Expect inventory_has {row['template_id']} == {row['count']}")
                if row['slot']:body.append(f"nf.Expect equipped {row['slot']} == {row['template_id']}")
            header=text[:text.index('nf.Login')]
            text=header+'\n'.join(['nf.Login','nf.WaitFor connected 15','nf.EnterWorld','nf.WaitFor in_world 15','nf.WaitFor item_baseline 3',*body,'nf.DropSocket','nf.WaitFor reconnects >= 1 15','nf.WaitFor item_baseline 15',*body,'nf.Within 108'])+'\n'
        if config.get('equipment_case'):
            case=config['equipment_case'];role=scenarios.fixtures.profiles(pack)['owner'];selection=scenarios.fixtures.selection()
            body=['nf.Login','nf.WaitFor connected 15','nf.EnterWorld','nf.WaitFor in_world 15','nf.WaitFor item_baseline 3','nf.ItemCatalog','nf.WaitFor item_catalog_ready 15']
            if case=='ring':
                rings=[r for r in role['inventory'] if r['template_id']==selection['ring']]
                for row,slot in zip(rings,('rfinger','lfinger')):
                    body += [f'nf.Equip instance:{row["instance_id"]} {slot}',f'nf.WaitFor equipped {slot} == {row["template_id"]} 3']
                body += [f'nf.Expect inventory_has {selection["ring"]} == 2']
            elif case=='onepiece':
                body += [f'nf.Equip {selection["onepiece"]}','nf.WaitFor equipped chest == '+str(selection['onepiece'])+' 3','nf.WaitFor equipped legs == 0 3','nf.Expect enchant_level chest == 4','nf.Expect inventory_has 461 == 1']
            else:
                body += [f'nf.Equip {selection["shield"]}','nf.WaitFor equipped lhand == '+str(selection['shield'])+' 3',f'nf.Equip {selection["twohand"]}','nf.WaitFor equipped rhand == '+str(selection['twohand'])+' 3','nf.WaitFor equipped lhand == 0 3','nf.Expect inventory_has '+str(selection['shield'])+' == 1']
            body += ['nf.Target template:skill_target','nf.WaitFor target != none 3','nf.Attack','nf.WaitFor damage_numbers >= 1 30','nf.StopAttack']
            budget=sum(int(l.split()[-1]) for l in body if l.startswith('nf.WaitFor '))+30
            text=text[:text.index('nf.Login')]+'\n'.join(body+[f'nf.Within {budget}'])+'\n'
        if base=='phase4-bow':text=text.replace('arrows_remaining == 2','arrows_remaining == 1').replace('arrows_remaining < 2','arrows_remaining == 0')
        if base=='phase4-armor':text=text[:text.index('# D20 provider cases')]+'\n'.join(armor_body(config))+'\n'
        path=variants/(name+'.nfs');path.write_text(text)
        assertions=[l for l in text.splitlines() if l.startswith(('nf.Expect ','# item-gate:'))]
        rows.append(dict(id=name,fixture=pack,canonical=canonical,scenarios=[str(path.relative_to(directory))],assertions=assertions,execution='pending E2/E3/E5'))
    for name,scroll in [('blessed',6573),('safe',22227)]:
        canonical='4-enchant-fail.nfs';text=(directory/canonical).read_text()
        text=text.replace('# fixture: phase4-enchant','# fixture: phase4-enchant\n# variant: 4-enchant-'+name)
        text=text.replace('nf.Enchant 951 ',f'nf.Enchant {scroll} ').replace('inventory_has 951',f'inventory_has {scroll}')
        path=variants/f'4-enchant-{name}.nfs';path.write_text(text)
        rows.append(dict(id='4-enchant-'+name,fixture='phase4-enchant',canonical=canonical,scenarios=[str(path.relative_to(directory))],assertions=[l for l in text.splitlines() if l.startswith(('nf.Expect ','# item-gate:'))],execution='pending E2/E3/E5'))
    for name,canonical in [('manual-soulshot','4-soulshot.nfs'),('blessed-escape','4-escape-scroll.nfs')]:
        text=(directory/canonical).read_text();pack='phase4-shots' if name=='manual-soulshot' else 'phase4-scrolls'
        text=text.replace('# fixture: '+pack,'# fixture: '+pack+'\n# variant: 4-'+name)
        if name=='manual-soulshot':text=text.replace('nf.AutoShot 1835 on','nf.UseItem 1835').replace('nf.WaitFor autoshot_on 1835 3','nf.WaitFor inventory_has 1835 == 99 3')
        else:text=text.replace('nf.UseItem 736','nf.UseItem 1538').replace('inventory_has 736','inventory_has 1538')
        path=variants/f'4-{name}.nfs';path.write_text(text)
        rows.append(dict(id='4-'+name,fixture=pack,canonical=canonical,scenarios=[str(path.relative_to(directory))],assertions=[l for l in text.splitlines() if l.startswith(('nf.Expect ','# item-gate:'))],execution='pending E2/E3/E5'))
    oracle_path=scenarios.ROOT/'packages/data/fixtures/item-oracle.json'
    oracle=json.loads(oracle_path.read_text());families={}
    for vector in oracle['vectors']:families.setdefault(vector['family'],[]).append(vector['id'])
    # These are setup contracts for future raw actor/replay input captures, not finite-vector evidence lookup.
    precision=[dict(id='source-'+family.lower(),layer='actor-adapter-replay',owner='E2/E3',
                    source_input=oracle_path.relative_to(scenarios.ROOT).as_posix(),source_sha256=hashlib.sha256(oracle_path.read_bytes()).hexdigest(),
                    vector_ids=ids,execution='pending producer fixtures; gates must use actual resolution inputs') for family,ids in sorted(families.items())]
    return dict(schema_version=1,runtime_cases=rows,precision_cases=precision)


def validate(directory=DIRECTORY):
    manifest=json.loads((directory/'phase4-variants.json').read_text())
    if manifest.get('schema_version')!=1:raise ValueError('invalid variant schema')
    seen=set();paths=[]
    for row in manifest['runtime_cases']:
        if row['id'] in seen or row['fixture'] not in scenarios.fixtures.PHASE4:raise ValueError('duplicate/unknown runtime variant')
        seen.add(row['id']);canonical=(directory/row['canonical']).read_text().splitlines()[0]
        for name in row['scenarios']:
            path=directory/name
            if Path(name).is_absolute() or '..' in Path(name).parts or not path.is_file():raise ValueError('missing/escaping variant')
            lines=path.read_text().splitlines()
            if lines[0]!=canonical or '# fixture: '+row['fixture'] not in lines or '# variant: '+row['id'] not in lines:raise ValueError('variant header mismatch')
            if not row['assertions'] or any(a not in lines for a in row['assertions']):raise ValueError('missing variant assertions')
            paths.append(path)
    if set(paths)!=set((directory/'variants').glob('4-*.nfs')):raise ValueError('orphan/missing runtime variant')
    failures=scenarios.validate(paths)
    if failures:raise ValueError(str(failures))
    for row in manifest['precision_cases']:
        path=scenarios.ROOT/row['source_input']
        if not row['vector_ids'] or hashlib.sha256(path.read_bytes()).hexdigest()!=row['source_sha256']:raise ValueError('missing/drifted precision inputs')
    return manifest


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--publish',action='store_true');args=parser.parse_args()
    if args.publish:(DIRECTORY/'phase4-variants.json').write_text(json.dumps(generate(),indent=2,sort_keys=True)+'\n')
    data=validate();print(f"Runtime variants:{len(data['runtime_cases'])}; source precision families:{len(data['precision_cases'])}; live:PENDING")
