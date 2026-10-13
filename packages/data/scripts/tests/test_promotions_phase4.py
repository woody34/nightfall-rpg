"""D34/D47/D50 coordinated capability promotions: exact counts and allow-list deletion mutations."""
import copy
import json
import tomllib
import unittest
from packages.data.scripts import gen_items as gen, gen_skills as skills

# Admitted counts after the coordinated promotion (data-progress revision 7 preview, all four families),
# plus D97 (rev 9): for_npc is not a blocker (+77 weapons, +5 armour, +76 etc items).
EXPECTED = {'Weapon': 1613, 'Armor': 1839, 'EtcItem': 492, 'skills': 1562, 'options': 23764,
            'augmentation': 27, 'recipes': 925}


class PromotionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.caps = tomllib.loads((gen.ROOT/'tables-phase4/capabilities.toml').read_text())
        cls.counts = json.loads((gen.ROOT/'tables-phase4/capability_counts.json').read_text())
        cls.skill = {r['id']: r for r in cls.caps['skills']}
        cls.item = {r['template_id']: r for r in cls.caps['items']}
        cls.all = skills.load_all_datapack_skills(skills.Datapack(gen.DEFAULT_DP), phase4=True)

    def blockers(self, sid):
        return skills.compute_all_blockers(self.all, {sid}, phase4=True)[1][sid]

    def test_counts_match_published_prediction(self):
        for kind in ['Weapon', 'Armor', 'EtcItem']:
            self.assertEqual(self.counts['items'][kind]['admitted'], EXPECTED[kind], kind)
        for key in ['options', 'augmentation', 'recipes']:
            self.assertEqual(self.counts[key]['admitted'], EXPECTED[key], key)
        self.assertEqual(self.counts['skills']['phase4']['admitted'], EXPECTED['skills'])
        self.assertEqual(sum(r['admitted'] for r in self.caps['skills']), EXPECTED['skills'])

    def test_pvp_stats_and_enlarge_abnormal_slot(self):
        self.assertTrue(self.skill[3552]['admitted'])
        self.assertTrue(self.skill[1405]['admitted'])
        for book in [8618, 8619, 8620, 8621]:
            self.assertTrue(self.item[book]['admitted'])
            self.assertEqual(self.item[book]['use_blockers'], ['use:no_handler'])
        for stat, sid in [('pvpPhysDmg', 3552), ('pvpPhysSkillsDmg', 3552), ('pvpMagicalDmg', 3552), ('enlargeAbnormalSlot', 1405)]:
            with self.subTest(stat=stat):
                saved = set(skills.PHASE4_STATS)
                try:
                    skills.PHASE4_STATS.discard(stat)
                    self.assertIn('stat:'+stat, self.blockers(sid))
                finally:
                    skills.PHASE4_STATS.clear(); skills.PHASE4_STATS.update(saved)
                self.assertEqual(self.blockers(sid), set())

    def test_blow_parameters(self):
        for sid in [16, 263]:
            self.assertTrue(self.skill[sid]['admitted'], sid)
            saved = copy.deepcopy(skills.PHASE4_EFFECT_PARAMS)
            try:
                del skills.PHASE4_EFFECT_PARAMS['FatalBlow']['blowChance']
                self.assertEqual(self.blockers(sid), {'param:FatalBlow.blowChance'})
            finally:
                skills.PHASE4_EFFECT_PARAMS.clear(); skills.PHASE4_EFFECT_PARAMS.update(saved)
        # Independent blockers stay deferred (no substitute promotion).
        self.assertEqual(self.skill[30]['blockers'], ['effect:Backstab'])
        self.assertIn('section:selfEffects', self.skill[409]['blockers'])
        for sid, param in [(16, 'blowChance'), (263, 'blowChance')]:
            self.assertFalse(skills.param_allowed('1.5', skills.PHASE4_EFFECT_PARAMS['FatalBlow'][param]))


if __name__ == '__main__':
    unittest.main()
