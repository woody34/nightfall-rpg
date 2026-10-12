"""D35 independent source, slot subtraction and hash binding checks."""
from copy import deepcopy
from fractions import Fraction as F
import tomllib
import unittest
from packages.data.scripts import gen_items as gen, item_oracle as oracle


class ClassSlotTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src=oracle.Source()
        cls.table=tomllib.loads((gen.ROOT/'tables-phase4/class_slots.toml').read_text())

    def test_all_base_rows_match_pinned_xml_and_getter_quirk(self):
        self.assertEqual([r['base_class_id'] for r in self.table['classes']],[0,10,18,25,31,38,44,49,53])
        for row in self.table['classes']:
            static=self.src.xml(row['source']['file']).find('staticData')
            pslots={('under' if n.tag=='underwear' else n.tag):int(n.text) for n in static.find('basePDef')}
            literal={n.tag:int(n.text) for n in static.find('baseMDef')}
            self.assertEqual(row['p_def_slots'],pslots)
            self.assertEqual(row['m_def_slots_xml'],literal)
            self.assertEqual(row['m_def_slots'],dict(literal,lfinger=literal['rfinger']))
            self.assertEqual(row['source']['revision'],oracle.DP_REV)
        # Unequal synthetic XML demonstrates the Java quirk even if pinned fingers match.
        self.assertIn('PAPERDOLL_LFINGER, set.getInt("baseMDefrfinger", 0)',self.src.read(oracle.G+'model/actor/templates/L2PcTemplate.java',True))

    def test_synthetic_unequal_fingers_prove_both_projection_and_subtraction(self):
        from types import SimpleNamespace
        def xml(file):
            root=gen.xml(self.src.dp/file)
            static=gen.children(root,'staticData')[0]
            for node in gen.children(static,'baseMDef')[0]['children']:
                if node['tag']=='rfinger':node['text']='7'
                if node['tag']=='lfinger':node['text']='999'
            return root
        rows=gen.class_slots(SimpleNamespace(xml=xml,read=self.src.read))['classes']
        self.assertTrue(all(r['m_def_slots']['lfinger']==7 and r['m_def_slots_xml']['lfinger']==999 for r in rows))
        file=rows[0]['source']['file'];root=self.src.xml(file)
        root.find('staticData/baseMDef/rfinger').text='7'
        root.find('staticData/baseMDef/lfinger').text='999'
        proxy=SimpleNamespace(xml=lambda file:root,bonuses=self.src.bonuses)
        result=oracle.defence_slot_stats(proxy,file,84,['lfinger'])
        self.assertEqual(result['m_def_subtraction'],7)
        self.assertEqual(result['m_def_base'],45)

    def test_empty_each_occupied_fullarmor_and_all_slots(self):
        cases=list(oracle.class_slot_cases(self.src));self.assertEqual(len(cases),153)
        by_id={r['base_class_id']:r for r in self.table['classes']}
        for name,inputs in cases:
            row=by_id[int(name.split('-')[2])];out=oracle.defence_slot_stats(self.src,**inputs)
            occupied=set(inputs['occupied_slots']);physical=occupied&row['p_def_slots'].keys()
            if 'chest' in occupied and inputs['chest_bodypart'] in {'fullarmor','onepiece'}:physical.add('legs')
            self.assertEqual(out['p_def_subtraction'],sum(row['p_def_slots'][k] for k in physical))
            self.assertEqual(out['m_def_subtraction'],sum(row['m_def_slots'][k] for k in occupied&row['m_def_slots'].keys()))
            self.assertEqual(out['p_def'],F(out['p_def_base']-out['p_def_subtraction'])*F(173,100))
            if name.endswith('-all'):self.assertEqual((out['p_def'],out['m_def']),(0,0))
            # Existing equipment oracle independently subtracts the same source slots.
            if name.endswith('-empty'):
                sheet=oracle.gear_stats(self.src,inputs['class_file'],84,[])
                self.assertEqual((out['p_def'],out['m_def']),(sheet['exact']['pDef'],sheet['exact']['mDef']))

    def test_table_is_hash_bound_and_sources_are_byte_hashed(self):
        manifest=tomllib.loads((gen.ROOT/'tables-phase4/rules_manifest.toml').read_text())
        digest=next(r['semantic_sha256'] for r in manifest['files'] if r['file']=='class_slots.toml')
        self.assertEqual(gen.digest(self.table),digest)
        for group,slot in [('p_def_slots','chest'),('m_def_slots','lfinger'),('m_def_slots_xml','lfinger')]:
            changed=deepcopy(self.table);changed['classes'][0][group][slot]+=1
            self.assertNotEqual(gen.digest(changed),digest)
        import hashlib,json
        inventory=json.loads((gen.ROOT/'tables-phase4/source_inventory.json').read_text())
        hashed={r['source']['file']:r['sha256'] for r in inventory['files']}
        for citation in self.table['sources']+[r['source'] for r in self.table['classes']]:
            root=self.src.dp if citation['revision']==oracle.DP_REV else self.src.game
            self.assertEqual(hashed[citation['file']],hashlib.sha256((root/citation['file']).read_bytes()).hexdigest())

    def test_slot_vector_deletion_rejects(self):
        vectors=oracle.make_vectors(self.src)
        with self.assertRaisesRegex(ValueError,'missing vectors'):
            oracle.verify_coverage(self.src,[v for v in vectors if not v['id'].startswith('class-slots-')])
