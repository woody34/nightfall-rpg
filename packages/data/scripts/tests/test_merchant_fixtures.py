"""P5-D29: tables-phase5/merchant_fixtures.toml contract, references and pinned-XML anchors."""

from pathlib import Path
import tomllib
import unittest

from packages.data.scripts import gen_items, gen_merchant_fixtures

DATA = Path(__file__).resolve().parents[2]
DP = gen_items.DEFAULT_DP
TABLE = tomllib.loads((DATA / 'tables-phase5/merchant_fixtures.toml').read_text())
MERCHANTS = tomllib.loads((DATA / 'tables-phase5/merchants.toml').read_text())
MULTISELL = tomllib.loads((DATA / 'tables-phase5/multisell.toml').read_text())
ZONE = tomllib.loads((DATA / 'zones/test_zone.toml').read_text())


class Contract(unittest.TestCase):
    def test_schema_order_and_uniqueness(self):
        self.assertEqual(set(TABLE), {'sources', 'schema_version', 'merchants'})
        self.assertEqual(TABLE['schema_version'], 1)
        rows = TABLE['merchants']
        self.assertEqual([set(r) for r in rows], [{'npc', 'name', 'x', 'y'}] * len(rows))
        npcs = [r['npc'] for r in rows]
        self.assertEqual(npcs, sorted(set(npcs)))
        self.assertEqual(len({r['name'] for r in rows}), len(rows))
        self.assertEqual(len({(r['name'], r['x'], r['y']) for r in rows}), len(rows))
        self.assertTrue(all(isinstance(r[k], int) for r in rows for k in ('npc', 'x', 'y')))

    def test_every_npc_is_a_buylist_or_multisell_npc_and_both_kinds_are_covered(self):
        buylist = {row['npc'] for row in MERCHANTS['buylists']}
        multisell = {npc for row in MULTISELL['list'] for npc in row.get('npcs', [])}
        rows = TABLE['merchants']
        self.assertTrue(all(r['npc'] in buylist | multisell for r in rows))
        self.assertTrue(any(r['npc'] in buylist for r in rows))
        self.assertTrue(any(r['npc'] in multisell for r in rows))
        # The packs: phase5-vendors buylist 314130, phase5-multisell list 616.
        lists = {row['list_id']: row['npc'] for row in MERCHANTS['buylists']}
        self.assertIn(lists[314130], [r['npc'] for r in rows])
        self.assertIn(32347, next(row['npcs'] for row in MULTISELL['list'] if row['id'] == 616))

    def test_positions_are_free_tiles_in_the_zone(self):
        (x0, y0), (x1, y1) = ZONE['bounds']['min'], ZONE['bounds']['max']
        taken = gen_merchant_fixtures.occupied(DATA)
        for row in TABLE['merchants']:
            with self.subTest(npc=row['npc']):
                self.assertTrue(x0 <= row['x'] <= x1 and y0 <= row['y'] <= y1)
                self.assertNotIn((row['x'], row['y']), taken)
        self.assertEqual(len({(r['x'], r['y']) for r in TABLE['merchants']}), len(TABLE['merchants']))

    def test_sources_point_at_the_pinned_npc_lines(self):
        for src in TABLE['sources']:
            with self.subTest(npc=src['npc']):
                self.assertEqual(src['revision'], gen_items.DATAPACK_REVISION)
                line = (DP / src['file']).read_text().splitlines()[src['line'] - 1]
                self.assertIn(f'<npc id="{src["npc"]}"', line)
                self.assertIn(f'name="{src["pinned_name"]}"', line)

    def test_the_generator_check_passes(self):
        content, _ = gen_merchant_fixtures.generate(DP)
        self.assertEqual(content, (DATA / gen_merchant_fixtures.OUT).read_text())


if __name__ == '__main__':
    unittest.main()
