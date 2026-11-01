"""P5-D30: rewards.toml carries can_be_sown and harvest_skills for every pinned NPC template."""

from pathlib import Path
import tomllib
import unittest
import xml.etree.ElementTree as ET

from packages.data.scripts import gen_items

DATA = Path(__file__).resolve().parents[2]
DP = gen_items.DEFAULT_DP
NPCS = DP / 'src/main/resources/data/stats/npcs'
REWARDS = tomllib.loads((DATA / 'tables-phase5/rewards.toml').read_text())
DROPS = tomllib.loads((DATA / 'tables-phase5/drops.toml').read_text())
STRONG_TYPE = set(range(4303, 4311))  # L2Attackable.setSeeded cases 4303..4310 (abfde049)


class Manor(unittest.TestCase):
    def test_every_row_has_both_fields_and_the_counts_are_the_pinned_ones(self):
        rows = REWARDS['npc']
        self.assertTrue(all({'can_be_sown', 'harvest_skills'} <= set(r) for r in rows))
        sowable = {r['id'] for r in rows if r['can_be_sown']}
        self.assertEqual(len(sowable), 2700)
        self.assertEqual(len(sowable - {n['id'] for n in DROPS['npc']}), 563)
        # The pinned datapack lists no strong-type skill (4303-4310) on any NPC: every
        # harvest_skills list is empty, so every manor count multiplier is 1.
        self.assertEqual([r['id'] for r in rows if r['harvest_skills']], [])

    def test_rows_equal_the_pinned_xml(self):
        xml = {}
        for path in sorted(NPCS.glob('*.xml')):  # the generator's set (custom/ is off by default)
            for npc in ET.parse(path).getroot().findall('npc'):  # templates, not nested minion refs
                status = npc.find('status')
                skills = npc.find('skillList')
                ids = {int(s.get('id')) for s in (skills.findall('skill') if skills is not None else [])}
                xml[int(npc.get('id'))] = (status is not None and status.get('canBeSown', '').lower() == 'true',
                                           sorted(ids & STRONG_TYPE))
        self.assertEqual({r['id']: (r['can_be_sown'], r['harvest_skills']) for r in REWARDS['npc']}, xml)

    def test_the_drops_rows_agree(self):
        rewards = {r['id']: r for r in REWARDS['npc']}
        for n in DROPS['npc']:
            self.assertEqual((rewards[n['id']]['can_be_sown'], rewards[n['id']]['harvest_skills']), (n['can_be_sown'], n['harvest_skills']))

    def test_spot_checks_at_file_line(self):
        rewards = {r['id']: r for r in REWARDS['npc']}
        for npc, rel, line, sown in [(148, '00100-00199.xml', 1842, True), (18120, '18100-18199.xml', 1141, True),
                                     (20003, '20000-20099.xml', 110, True), (100, '00100-00199.xml', None, False)]:
            with self.subTest(npc=npc):
                self.assertEqual(rewards[npc]['can_be_sown'], sown)
                if line is not None:
                    self.assertIn('canBeSown="true"', (NPCS / rel).read_text().splitlines()[line - 1])


if __name__ == '__main__':
    unittest.main()
