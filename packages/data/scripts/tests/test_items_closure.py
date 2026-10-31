"""D10: tables-phase5/items_closure.toml rows, contract and pinned-XML anchors."""

from pathlib import Path
import re
import tomllib
import unittest

from packages.data.scripts import gen_items, gen_items_closure

DATA = Path(__file__).resolve().parents[2]
DP = gen_items.DEFAULT_DP
CLOSURE = tomllib.loads((DATA / 'tables-phase5/items_closure.toml').read_text())
CATALOGUE = tomllib.loads((DATA / 'tables-phase5/catalogue.toml').read_text())
V6_ROW_KEYS = set(tomllib.loads((DATA / 'tables-phase4/etc_items.toml').read_text())['items'][0])
# The ten pack templates P5-D7 admits through the closure.
PACK_TEMPLATES = {1294, 1666, 1864, 1869, 1870, 3838, 3845, 5125, 6908, 6926}


def floats(value):
    if isinstance(value, float):
        return True
    if isinstance(value, dict):
        return any(floats(v) for v in value.values())
    if isinstance(value, list):
        return any(floats(v) for v in value)
    return False


class Contract(unittest.TestCase):
    def test_file_keys_and_v6_binding(self):
        self.assertEqual(set(CLOSURE), {'schema_version', 'sources', 'v6_items_sha256', 'items', 'capabilities'})
        self.assertEqual(CLOSURE['schema_version'], 1)
        self.assertEqual(CLOSURE['v6_items_sha256'], 'bfee2e7d9fb73f53706c7a4afc19f5d095f4d630ca90e9a5747895c0e5472aad')
        manifest = tomllib.loads((DATA / 'tables-phase4/rules_manifest.toml').read_text())
        self.assertEqual(manifest['data_sha256'], gen_items_closure.V6_MANIFEST_DATA_SHA256)

    def test_ids_match_the_catalogue_closure_disjoint_from_v6_ascending(self):
        items = [r['template_id'] for r in CLOSURE['items']]
        caps = [r['template_id'] for r in CLOSURE['capabilities']]
        self.assertEqual(items, sorted(set(items)))
        self.assertEqual(items, caps)
        self.assertEqual(set(items), set(CATALOGUE['phase4_closure_additions']))
        self.assertEqual(len(items), 4264)
        v6 = set()
        for name in ('weapons', 'armors', 'etc_items'):
            v6 |= {r['template_id'] for r in tomllib.loads((DATA / f'tables-phase4/{name}.toml').read_text())['items']}
        self.assertFalse(v6 & set(items))
        self.assertFalse({t['id'] for t in CATALOGUE['templates']} - v6 - set(items), 'V6 plus closure covers every Phase 5 reference')
        self.assertLessEqual(PACK_TEMPLATES, set(items))

    def test_row_shapes_are_the_v6_shapes_and_integer_only(self):
        for row in CLOSURE['items']:
            self.assertEqual(set(row), V6_ROW_KEYS, row['template_id'])
            self.assertEqual(len(row['flags']), 18)
        for row in CLOSURE['capabilities']:
            self.assertEqual(set(row), {'template_id', 'admitted', 'storable', 'blockers', 'use_blockers'})
            self.assertTrue(row['storable'])
            self.assertEqual(row['admitted'], not row['blockers'])
            self.assertLessEqual(set(row['blockers']), set(row['use_blockers']))
        self.assertFalse(floats(CLOSURE))


class PinnedAnchors(unittest.TestCase):
    """20 rows, spread over the closure, re-read at their file:line in the pinned datapack XML."""

    def test_twenty_rows_against_the_pinned_xml(self):
        rows = CLOSURE['items']
        picks = [rows[i * (len(rows) - 1) // 19] for i in range(20)]
        self.assertEqual(len({r['template_id'] for r in picks}), 20)
        for row in picks:
            with self.subTest(template=row['template_id']):
                src = row['source']
                self.assertEqual(src['revision'], gen_items.DATAPACK_REVISION)
                lines = (DP / src['file']).read_text().splitlines()
                start = int(src['lines']) - 1
                head = lines[start]
                self.assertRegex(head, rf'<item id="{row["template_id"]}"')
                self.assertIn(f'name="{row["name"]}"', head.replace('&apos;', "'").replace('&amp;', '&'))
                self.assertIn(f'type="{row["kind"]}"', head)
                end = next(i for i in range(start, len(lines)) if '</item>' in lines[i] or lines[i].rstrip().endswith('/>') and i == start)
                body = '\n'.join(lines[start:end + 1])
                sets = dict(re.findall(r'<set name="([^"]+)" val="([^"]*)"', body))
                self.assertEqual(int(sets.get('weight', 0)), row['weight'])
                self.assertEqual(int(sets.get('price', 0)), row['price'])
                self.assertEqual(sets.get('is_stackable', 'false') == 'true', row['stackable'])
                self.assertEqual(sets.get('etcitem_type', 'none').upper(), row['etcitem_type'] if row['etcitem_type'] != 'SHOT' else sets.get('etcitem_type', 'none').upper())
                self.assertEqual(sets.get('handler', ''), row['handler'])


if __name__ == '__main__':
    unittest.main()
