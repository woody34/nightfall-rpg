"""D54 consumable admission, fixed-point propagation and deletion regressions."""
from pathlib import Path
import tempfile
import tomllib
import unittest
import xml.etree.ElementTree as ET
from packages.data.scripts import gen_items as gen


class ConsumableClosureTests(unittest.TestCase):
    def capabilities(self, costs, npc=(), deferred=(), deleted=()):
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/'items.xml'
            path.write_text('<list>'+''.join(
                f'<item id="{sid}" name="Item {sid}" type="EtcItem">'
                '<set name="handler" val="ItemSkills"/>'
                f'<set name="item_skill" val="{sid}-{rank}"/>'
                f'<set name="for_npc" val="{str(sid in npc).lower()}"/>'
                '</item>' for sid,rank in [(1,1),(2,1),(3,1),(4,2),(5,1),(6,1)])+'</list>')
            items={int(gen.attrs(n)['id']):gen.project_item(n,'items.xml',{'none':0})
                   for n in gen.children(gen.xml(path),'item') if int(gen.attrs(n)['id']) not in deleted}
        skills=[{'id':sid,'max_level':2,'admitted':sid not in deferred,
                 'blockers':['effect:unsupported'] if sid in deferred else []} for sid in range(1,7)]
        skill_files={f'{sid}.toml':gen.encode_toml({'id':sid,'max_level':2,
            'item_consume':[{'item_id':target,'count':count}]}) for sid,(target,count) in costs.items()}
        tables={'armor_sets.toml':{'sets':[]},'augmentation_options.toml':{'options':[]},
                'enchant_options.toml':{'items':[]},'augmentation_chances.toml':{'rows':[]},
                'capacities.toml':{'policy':{'mana_potion_support':'false'}},
                'sa_variants.toml':{'recipes':[{'recipe_id':1,'ingredients':[{'template_id':2}],
                                              'products':[{'template_id':5}]}]}}
        caps=gen.capabilities(items,tables,skills,skill_files)
        return {r['template_id']:r for r in caps['items']},caps['recipes']

    def test_consumable_closure_propagates_and_uses_resolved_rank(self):
        costs={1:(2,1),2:(3,1),3:(3,1),4:([3,5],[1,1]),5:(5,1),6:(3,0)}
        rows,recipes=self.capabilities(costs,npc={3})
        for sid,target in [(1,2),(2,3),(3,3)]:
            self.assertFalse(rows[sid]['admitted'])
            blocker=f'consumable:skill:{sid}:1:item:{target}'
            self.assertIn(blocker,rows[sid]['blockers'])
            self.assertIn(blocker,rows[sid]['use_blockers'])
        self.assertTrue(rows[1]['storable'])
        self.assertFalse(rows[3]['storable'])
        for sid in [4,5,6]:self.assertTrue(rows[sid]['admitted'],sid)
        self.assertEqual(recipes,[{'recipe_id':1,'admitted':False,'blockers':['item:2']}])
        rows,recipes=self.capabilities(costs)
        self.assertTrue(all(r['admitted'] for r in rows.values()))
        self.assertTrue(recipes[0]['admitted'])
        rows,_=self.capabilities(costs,deferred={3})
        self.assertTrue(rows[3]['storable'])
        self.assertFalse(rows[1]['admitted'])

    def test_deleting_consumable_fails_closed_like_non_storable_consumable(self):
        for kwargs in [{'npc':{2}},{'deleted':{2}}]:
            with self.subTest(**kwargs):
                rows,recipes=self.capabilities({1:(2,1)},**kwargs)
                self.assertFalse(rows[1]['admitted'])
                self.assertIn('consumable:skill:1:1:item:2',rows[1]['blockers'])
                self.assertFalse(recipes[0]['admitted'])

    def test_pinned_21040_regression_and_manifest_deletion(self):
        dp=gen.DEFAULT_DP/gen.D
        skill=next(s for s in ET.parse(dp/'stats/skills/02800-02899.xml').getroot()
                   if s.get('id')=='2864')
        sets={s.get('name'):s.get('val') for s in skill.findall('set')}
        self.assertEqual((sets['itemConsumeId'],sets['itemConsumeCount']),('14701','1'))
        npc=next(n for n in ET.parse(dp/'stats/items/14700-14799.xml').getroot()
                 if n.get('id')=='14701')
        self.assertEqual(npc.find("set[@name='for_npc']").get('val'),'true')
        directory=gen.ROOT/'tables-phase4'
        caps=tomllib.loads((directory/'capabilities.toml').read_text())
        by_id={r['template_id']:r for r in caps['items']}
        self.assertFalse(by_id[14701]['storable'])
        self.assertFalse(by_id[21040]['admitted'])
        self.assertIn('consumable:skill:2864:1:item:14701',by_id[21040]['blockers'])
        # Independent admitted-item audit catches deletion of the deferral result.
        items={r['template_id']:r for name in ['weapons','armors','etc_items']
               for r in tomllib.loads((directory/(name+'.toml')).read_text())['items']}
        def audit():
            for sid,row in by_id.items():
                if not row['admitted']:continue
                for ref in items[sid]['skills']+items[sid]['enchant4_skills']:
                    skill=tomllib.loads((gen.ROOT/f'skills-phase4/{ref["id"]}.toml').read_text())
                    for req in skill.get('item_consume',[]):
                        rank=ref['level']-1
                        target=req['item_id'][rank] if isinstance(req['item_id'],list) else req['item_id']
                        count=req['count'][rank] if isinstance(req['count'],list) else req['count']
                        if target and count>0:
                            self.assertTrue(by_id[target]['storable'],(sid,target))
                            self.assertTrue(by_id[target]['admitted'],(sid,target))
        audit()
        by_id[21040]['admitted']=True
        by_id[21040]['blockers']=[]
        by_id[21040]['use_blockers']=[]
        with self.assertRaises(AssertionError):audit()


if __name__=='__main__':unittest.main()
