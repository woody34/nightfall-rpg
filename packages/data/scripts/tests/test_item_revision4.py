"""Review-3 acceptance gate: deletion and dimension mutations, source domains."""
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

from packages.data.scripts import item_oracle as oracle
from packages.data.scripts.item_coverage import required_cases, verify_case


class Revision4Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src = oracle.Source()
        cls.required = required_cases(cls.src)
        cls.vectors = oracle.make_vectors(cls.src)
        cls.by_id = {v['id']: v for v in cls.vectors}

    def test_complete_corpus_passes(self):
        self.assertEqual(oracle.verify_coverage(self.src, self.vectors)['required_cases'], len(self.required))

    def test_each_mandatory_family_deletion_rejects(self):
        # Separate §12.1 obligations, including subfamilies sharing one F label.
        prefixes = ['gear-', 'starter-kit-', 'attribute-', 'provider-install-', 'provider-overlap-',
                    'class-slots-', 'RFINGER-', 'system-', 'enchant-', 'hp-', 'crystals-', 'chance-',
                    'scroll-', 'bonus-only-', 'failure-', 'blessed-support-', 'modulo-lattice-',
                    'load-CON-', 'overload-', 'grade-', 'fist-', 'pole-', 'dual-', 'bow-', 'damage-',
                    'magic-', 'cast-', 'heal-', 'shield-', 'position-', 'angle-', 'blow-', 'lethal-',
                    'retail-', 'categories-', 'packed-', 'cancel-', 'SA-', 'soul-', 'set-history-',
                    'clamp-', 'charge-', 'shot-payment-', 'shot-reduction-', 'shot-bundle-',
                    'consumable-', 'elixir-range-', 'weapon-enchant4-']
        for prefix in prefixes:
            with self.subTest(family=prefix):
                self.assertTrue(any(name.startswith(prefix) for name in self.required))
                with self.assertRaisesRegex(ValueError, 'missing vectors'):
                    oracle.verify_coverage(self.src, [v for v in self.vectors if not v['id'].startswith(prefix)])
        for name in ['gear-22-kit-HumanMystic', 'gear-23-kit-OrcFighter', 'gear-24-kit-DwarvenFighter']:
            with self.subTest(starter=name), self.assertRaisesRegex(ValueError, 'missing vectors'):
                oracle.verify_coverage(self.src, [v for v in self.vectors if v['id'] != name])

    def test_every_required_case_deletion_rejects(self):
        # Includes every individual boundary, source row, and scroll interval.
        for name in self.required:
            with self.subTest(case=name), self.assertRaisesRegex(ValueError, 'missing vectors'):
                oracle.verify_coverage(self.src, [v for v in self.vectors if v['id'] != name])

    def test_every_required_input_dimension_rejects_deletion_and_mutation(self):
        # Exercise the same row validator used by the gate without repeatedly
        # reparsing/validating thousands of unrelated rows for each dimension.
        for name, case in self.required.items():
            row = self.by_id[name]
            verify_case(name, case, row)
            for key in case['inputs']:
                with self.subTest(case=name, dimension=key):
                    inputs = dict(row['inputs']); del inputs[key]
                    with self.assertRaisesRegex(ValueError, 'input dimension'):
                        verify_case(name, case, dict(row, inputs=inputs))
                    inputs[key] = {'invalid': True}
                    with self.assertRaisesRegex(ValueError, 'input dimension'):
                        verify_case(name, case, dict(row, inputs=inputs))

    def test_whole_gate_rejects_preserved_ids_with_wrong_dimensions(self):
        names = ['gear-22-kit-HumanMystic', 'retail-weapon-MAGE-0-0-55', 'soul-4629',
                 'modulo-lattice-66670', 'fist-4-level_change-14', 'pole-499/10-85',
                 'cancel-C-1720', 'enchant-S84-lrhand-BOW-pAtk-4', 'shield-91-20',
                 'consumable-same-tick-lanes']
        # Selector/source identities vary by pinned XML; choose from source requirements.
        names[1] = next(n for n in self.required if n.startswith('retail-weapon-'))
        names[2] = next(n for n in self.required if n.startswith('soul-'))
        for name in names:
            row = self.by_id[name]; key = next(iter(self.required[name]['inputs']))
            inputs = dict(row['inputs']); inputs[key] = None
            mutated = [dict(v, inputs=inputs) if v['id'] == name else v for v in self.vectors]
            with self.subTest(case=name), self.assertRaisesRegex(ValueError, 'input dimension'):
                oracle.verify_coverage(self.src, mutated)
        name = 'enchant-D-rhand-SWORD-pAtk-0'; row = self.by_id[name]
        inputs = dict(row['inputs'], enchant=False)
        with self.assertRaisesRegex(ValueError, 'input dimension'):
            oracle.verify_coverage(self.src, [dict(v, inputs=inputs) if v['id'] == name else v for v in self.vectors])
        with self.assertRaisesRegex(ValueError, 'incorrect family'):
            oracle.verify_coverage(self.src, [dict(v, family='F1') if v['id'] == name else v for v in self.vectors])
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            oracle.verify_coverage(self.src, self.vectors+[row])

    def test_requirements_do_not_depend_on_generation_or_fixture(self):
        required_cases.cache_clear()
        with patch.object(oracle, 'make_vectors', side_effect=AssertionError('generation is not authority')):
            independently_required = required_cases(self.src)
        self.assertEqual(independently_required, self.required)
        with self.assertRaisesRegex(ValueError, 'missing vectors'):
            oracle.verify_coverage(self.src, [])

    def test_source_domain_census_and_design_boundaries(self):
        # Independent counts from pinned source XML, not generated vector IDs.
        kits = self.src.xml(oracle.D+'stats/initialEquipment.xml').findall('equipment')
        self.assertEqual(len([n for n in self.required if n.startswith('starter-kit-source-')]), len(kits))
        self.assertEqual(len([n for n in self.required if n.startswith('load-CON-')]), len(self.src.bonuses['CON']))
        soul = self.src.xml(oracle.D+'levelUpCrystalData.xml').findall('crystal/item')
        self.assertEqual(len([n for n in self.required if n.startswith('soul-')]), len(soul))
        retail_count = sum(len(self.src.xml(oracle.D+'stats/augmentation/'+file).findall('weapon/stone/variation/category'))
                           for file in ['retailchances.xml', 'retailchances_accessory.xml'])
        self.assertEqual(len([n for n in self.required if n.startswith(('retail-weapon-', 'retail-accessory-'))]), retail_count)
        self.assertEqual(len([n for n in self.required if n.startswith('system-')]),
                         4*sum(int(self.src.skills[s][1].get('levels')) for s in [4270, 6209, 6213, 3599]))
        self.assertEqual({self.required[n]['inputs']['enchant'] for n in self.required
                          if n.startswith('enchant-') and self.required[n]['family'] == 'F2'},
                         {0, 3, 4, 19, 20, 65535, 4294967295})
        for level, radius in [(1, 6), (3, 6), (4, 7), (8, 7), (9, 8), (15, 8), (16, 9)]:
            for path in ['unequip', 'level_change', 'restore_weapon']:
                for residue in [0, 2*radius]:
                    self.assertIn(f'fist-{level}-{path}-{residue}', self.required)
        # All source SA recipe candidates (including deferred recipes) are required.
        crystal_ids = {int(n.get(k)) for n in soul for k in ['itemId', 'leveledItemId']}
        sa_count = 0
        for path in (self.src.dp/(oracle.D+'multisell')).glob('*.xml'):
            for item in ET.parse(path).getroot().findall('item'):
                ingredients = [int(n.get('id')) for n in item.findall('ingredient')]
                products = [int(n.get('id')) for n in item.findall('production')]
                weapon = lambda sid: sid in self.src.items and self.src.items[sid][2].get('type') == 'Weapon'
                if sum(map(weapon, ingredients)) == 1 and crystal_ids.intersection(ingredients) and products and all(map(weapon, products)):
                    sa_count += 1
        self.assertEqual(len([n for n in self.required if n.startswith('SA-')]), sa_count)
