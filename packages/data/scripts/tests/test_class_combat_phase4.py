"""D75 independent source, lineage census, oracle reach and hash binding checks."""
from copy import deepcopy
import hashlib
import json
import tomllib
import unittest
from packages.data.scripts import gen_items as gen, item_oracle as oracle


class ClassCombatTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src=oracle.Source()
        cls.table=tomllib.loads((gen.ROOT/'tables-phase4/class_combat.toml').read_text())
        cls.vectors={v['id']:v for v in json.loads(oracle.OUT.read_text())['vectors']}

    def test_rows_match_pinned_xml_and_orc_lineage_is_25(self):
        self.assertEqual([r['base_class_id'] for r in self.table['classes']],[0,10,18,25,31,38,44,49,53])
        for row in self.table['classes']:
            static=self.src.xml(row['source']['file']).find('staticData')
            self.assertEqual(row['source']['revision'],oracle.DP_REV)
            self.assertEqual((row['attack_type'],row['attack_range'],row['p_atk'],row['p_atk_spd'],row['crit_rate']),
                             (static.findtext('baseAtkType'),int(static.findtext('baseAtkRange')),int(static.findtext('basePAtk')),
                              int(static.findtext('basePAtkSpd')),int(static.findtext('baseCritRate'))))
        ranges={r['key']:r['attack_range'] for r in self.table['classes']}
        self.assertEqual({k for k,v in ranges.items() if v==25},{'orc_fighter','orc_mystic'})
        self.assertTrue(all(v==20 for k,v in ranges.items() if not k.startswith('orc_')))
        self.assertEqual(self.table['unarmed'],{'attack_angle':120,'random_damage_radius_base':5})

    def test_fists_ranges_match_pinned_find_fists_weapon_item(self):
        # D84: L2PcInstance.findFistsWeaponItem (GAME@abfde049 2141-2183).
        self.assertEqual([(f['class_min'],f['class_max'],f['template_id']) for f in self.table['fists']],
                         [(0,9,246),(10,17,251),(18,24,244),(25,30,249),(31,37,245),(38,43,250),
                          (44,48,248),(49,52,252),(53,57,247)])

    def test_lineage_census_is_complete_and_uniform(self):
        cases=dict(oracle.class_combat_cases(self.src))
        by_base={}
        for name,inputs in cases.items():by_base.setdefault(inputs['base_class_id'],[]).append(int(name.split('-')[2]))
        for row in self.table['classes']:
            self.assertEqual(row['lineage_class_ids'],sorted(by_base[row['base_class_id']]))
        self.assertEqual(sum(len(r['lineage_class_ids']) for r in self.table['classes']),89)

    def test_generator_rejects_divergent_descendant(self):
        from types import SimpleNamespace
        inputs=gen.Inputs(gen.DEFAULT_DP,gen.DEFAULT_GAME)
        original=inputs.xml
        def xml(file):
            root=original(file)
            if file.endswith('/Destroyer.xml'):
                static=gen.children(root,'staticData')[0]
                gen.children(static,'baseAtkRange')[0]['text']='40'
            return root
        proxy=SimpleNamespace(xml=xml,read=inputs.read,dp=inputs.dp)
        with self.assertRaisesRegex(gen.SourceError,'differ from base class 44'):gen.class_combat(proxy)

    def test_oracle_vectors_agree_with_table(self):
        by_id={r['base_class_id']:r for r in self.table['classes']}
        for name,inputs in oracle.class_combat_cases(self.src):
            row=by_id[inputs['base_class_id']];vector=self.vectors[name]
            self.assertEqual(vector['expected']['base_attack_range'],row['attack_range'])
            self.assertEqual(vector['expected']['attack_type'],row['attack_type'])
            self.assertEqual(vector['expected']['attack_angle'],self.table['unarmed']['attack_angle'])
        armed=self.vectors['class-combat-44-armed-2369']['expected']
        self.assertEqual((armed['attack_type'],armed['base_attack_range']),('SWORD',40))

    def test_vector_deletion_rejects(self):
        vectors=oracle.make_vectors(self.src)
        for name in ['class-combat-44-unarmed','class-combat-114-unarmed','class-combat-0-unarmed','class-combat-44-armed-2369']:
            with self.subTest(name=name),self.assertRaisesRegex(ValueError,'missing vectors'):
                oracle.verify_coverage(self.src,[v for v in vectors if v['id']!=name])

    def test_table_is_hash_bound_and_sources_are_byte_hashed(self):
        manifest=tomllib.loads((gen.ROOT/'tables-phase4/rules_manifest.toml').read_text())
        digest=next(r['semantic_sha256'] for r in manifest['files'] if r['file']=='class_combat.toml')
        self.assertEqual(gen.digest(self.table),digest)
        for mutate in [lambda t:t['classes'][6].__setitem__('attack_range',20),lambda t:t['unarmed'].__setitem__('attack_angle',121)]:
            changed=deepcopy(self.table);mutate(changed)
            self.assertNotEqual(gen.digest(changed),digest)
        inventory=json.loads((gen.ROOT/'tables-phase4/source_inventory.json').read_text())
        hashed={r['source']['file']:r['sha256'] for r in inventory['files']}
        for citation in self.table['sources']+[r['source'] for r in self.table['classes']]:
            root=self.src.dp if citation['revision']==oracle.DP_REV else self.src.game
            self.assertEqual(hashed[citation['file']],hashlib.sha256((root/citation['file']).read_bytes()).hexdigest())


if __name__=='__main__':
    unittest.main()
