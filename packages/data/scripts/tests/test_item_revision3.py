"""Bounded review-2 regression and evidence-deletion gates."""
import unittest

from packages.data.scripts import item_oracle as oracle


class Revision3Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src=oracle.Source()
        cls.vectors=oracle.make_vectors(cls.src)
        cls.by_id={v['id']:v for v in cls.vectors}

    def test_noop_preserves_above_cap_fraction(self):
        for current in [1000,1001]:
            for power,static in [(100,False),(0,True)]:
                row=oracle.heal(power,121,current_hp=current,remainder_q=500000,
                                recoverable_hp=1000,static=static)
                self.assertEqual((row['applied_q'],row['current_hp'],row['remainder_q']),(0,current,500000))
        # An actual positive clamp discards the fraction; a zero heal below cap does not.
        self.assertEqual(oracle.heal(100,121,current_hp=950,remainder_q=500000,recoverable_hp=1000)['remainder_q'],0)
        self.assertEqual(oracle.heal(0,121,static=True,current_hp=950,remainder_q=500000,recoverable_hp=1000)['remainder_q'],500000)

    def test_same_completion_tick_retains_main_and_consumes_once(self):
        row=self.by_id['consumable-same-tick-lanes'];out=row['expected'];trace=out['trace']
        self.assertEqual([e['tick'] for e in trace],[10,209,210,210])
        self.assertEqual([e['cast']['completion_tick'] for e in trace[:2]],[210,210])
        self.assertEqual(trace[0]['main_cast'],trace[1]['main_cast'])
        self.assertEqual([e['lane'] for e in trace[2:]],['main','item'])
        self.assertEqual(out['completion_order'],[1,2])
        self.assertEqual(out['total_consumed'],{'736':1,'1539':1})
        self.assertTrue(all(e['consumed']=={} for e in trace[2:]))
        self.assertEqual(out['inventory'],{'736':0,'1539':1})
        self.assertEqual(out['main_cast'],None)
        self.assertEqual(out['item_casts'],[])
        # Formula facts use the full sequence inputs rather than a golden-ID lookup.
        self.assertEqual(oracle.verify_facts([{'formula':'consumable_cast_sequence','inputs':row['inputs'],'expected':out}]),1)

    def test_published_same_priority_inputs_are_distinguishing_and_synthetic(self):
        rows=[self.by_id[f'attribute-same-priority-{sequence}'] for sequence in [1,3]]
        self.assertEqual([v['expected']['base_stats']['STR'] for v in rows],[40,42])
        self.assertNotEqual(rows[0]['expected']['q']['pAtk'],rows[1]['expected']['q']['pAtk'])
        self.assertTrue(all(v['inputs']['synthetic']['kind']=='synthetic' for v in rows))

    def test_required_evidence_deletion_mutations(self):
        groups=[lambda v:v['id'].startswith('consumable-shared-reuse-'),
                lambda v:v['id'].startswith('attribute-same-priority-'),
                lambda v:v['id']=='consumable-same-tick-lanes']
        for removed in groups:
            with self.assertRaisesRegex(ValueError,'missing vectors'):
                oracle.verify_coverage(self.src,[v for v in self.vectors if not removed(v)])
        names=[v['id'] for v in self.vectors if any(group(v) for group in groups)]
        self.assertEqual(len(names),9)
        for name in names:
            with self.subTest(name=name),self.assertRaisesRegex(ValueError,'missing vectors'):
                oracle.verify_coverage(self.src,[v for v in self.vectors if v['id']!=name])
