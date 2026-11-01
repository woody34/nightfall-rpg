"""P5-D19: tables-phase5/skill_promotions.toml contract, V6 guard and pinned-XML anchors."""

from pathlib import Path
import shutil
import tempfile
import tomllib
import unittest

from packages.data.scripts import gen_items, gen_skill_promotions

DATA = Path(__file__).resolve().parents[2]
DP = gen_items.DEFAULT_DP
TABLE = tomllib.loads((DATA / 'tables-phase5/skill_promotions.toml').read_text())
SKILL_KEYS = set(tomllib.loads((DATA / 'skills-phase4/254.toml').read_text()))


def floats(value):
    if isinstance(value, float):
        return True
    if isinstance(value, dict):
        return any(floats(v) for v in value.values())
    if isinstance(value, list):
        return any(floats(v) for v in value)
    return False


class Contract(unittest.TestCase):
    def test_keys_binding_and_rows(self):
        self.assertEqual(set(TABLE), {'sources', 'schema_version', 'v6_skills_sha256', 'formula_revision', 'promotions', 'additions'})
        self.assertEqual((TABLE['schema_version'], TABLE['formula_revision']), (1, 1))
        self.assertEqual(TABLE['v6_skills_sha256'], '5c2378c9eab24f26fdef161c4965dc3567fc05eefe04186cdf095d84cad7f6d1')
        self.assertEqual([r['id'] for r in TABLE['promotions']], [42, 254])
        self.assertEqual([a['skill']['id'] for a in TABLE['additions']], [2097, 2098])
        self.assertFalse(floats(TABLE))

    def test_promotions_clear_exactly_the_v6_blockers(self):
        for row in TABLE['promotions']:
            v6 = tomllib.loads((DATA / f'skills-phase4/{row["id"]}.toml').read_text())
            self.assertEqual(v6['runtime_status'], 'deferred')
            self.assertEqual((row['min_level'], row['max_level']), (1, v6['max_level']))
            self.assertEqual(row['clears'], sorted(v6['blockers']))
            self.assertLessEqual(set(row['clears']), gen_skill_promotions.V7_IMPLEMENTED_BLOCKERS)

    def test_additions_are_absent_from_v6_and_are_v6_skill_rows(self):
        for row in TABLE['additions']:
            skill = row['skill']
            self.assertFalse((DATA / f'skills-phase4/{skill["id"]}.toml').exists())
            self.assertLessEqual(SKILL_KEYS - {'conditions'}, set(skill) | {'conditions'})
            self.assertEqual(skill['runtime_status'], 'deferred')
            self.assertEqual(row['clears'], skill['blockers'])
            self.assertLessEqual(set(row['clears']), gen_skill_promotions.V7_IMPLEMENTED_BLOCKERS)
        self.assertEqual([a['clears'] for a in TABLE['additions']], [['effect:Sow'], ['effect:Harvesting', 'target:NPC_BODY']])

    def test_sources_point_at_the_pinned_skill_lines(self):
        names = {42: 'Sweeper', 254: 'Spoil', 2097: 'Sowing', 2098: 'Harvesting'}
        for src in TABLE['sources']:
            with self.subTest(skill=src['skill_id']):
                self.assertEqual(src['revision'], gen_items.DATAPACK_REVISION)
                line = (DP / src['file']).read_text().splitlines()[src['line'] - 1]
                self.assertIn(f'<skill id="{src["skill_id"]}"', line)
                self.assertIn(f'name="{names[src["skill_id"]]}"', line)
        self.assertEqual({(s['skill_id'], s['line']) for s in TABLE['sources']}, {(42, 1465), (254, 1342), (2097, 1537), (2098, 1558)})


class Guard(unittest.TestCase):
    def test_a_changed_v6_skill_bundle_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            shutil.copytree(DATA / 'tables-phase4', root / 'tables-phase4')
            shutil.copytree(DATA / 'skills-phase4', root / 'skills-phase4')
            manifest = root / 'tables-phase4/rules_manifest.toml'
            manifest.write_text(manifest.read_text().replace(gen_skill_promotions.V6_SKILL_BUNDLE_SHA256, '0' * 64))
            with self.assertRaisesRegex(gen_skill_promotions.SourceError, 'V6 skill bundle changed'):
                gen_skill_promotions.v6_skills(root)
            shutil.copy(DATA / 'tables-phase4/rules_manifest.toml', manifest)
            row = root / 'skills-phase4/254.toml'
            row.write_text(row.read_text().replace('max_level = 11', 'max_level = 12'))
            with self.assertRaisesRegex(gen_skill_promotions.SourceError, 'does not match the V6 skill bundle digest'):
                gen_skill_promotions.v6_skills(root)


if __name__ == '__main__':
    unittest.main()
