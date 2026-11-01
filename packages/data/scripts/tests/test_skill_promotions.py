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
        b11 = [a['skill']['id'] for a in TABLE['additions'] if a['skill']['id'] in (2097, 2098)]
        self.assertEqual(b11, [2097, 2098])
        self.assertEqual(len(TABLE['additions']), 29, 'B11 Sowing/Harvesting + the 27 clean herb skills')
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
            # A B11 addition is deferred with its blockers; a herb addition is clean (P5-D19 herbs).
            self.assertEqual(skill['runtime_status'], 'deferred' if row['clears'] else 'executable')
            self.assertEqual(row['clears'], skill['blockers'])
            self.assertLessEqual(set(row['clears']), gen_skill_promotions.V7_IMPLEMENTED_BLOCKERS)
        b11 = [a['clears'] for a in TABLE['additions'] if a['skill']['id'] in (2097, 2098)]
        self.assertEqual(b11, [['effect:Sow'], ['effect:Harvesting', 'target:NPC_BODY']])

    def test_sources_point_at_the_pinned_skill_lines(self):
        names = {42: 'Sweeper', 254: 'Spoil', 2097: 'Sowing', 2098: 'Harvesting'}
        for src in [s for s in TABLE['sources'] if s['skill_id'] in names]:
            with self.subTest(skill=src['skill_id']):
                self.assertEqual(src['revision'], gen_items.DATAPACK_REVISION)
                line = (DP / src['file']).read_text().splitlines()[src['line'] - 1]
                self.assertIn(f'<skill id="{src["skill_id"]}"', line)
                self.assertIn(f'name="{names[src["skill_id"]]}"', line)
        self.assertEqual({(s['skill_id'], s['line']) for s in TABLE['sources'] if s['skill_id'] in names}, {(42, 1465), (254, 1342), (2097, 1537), (2098, 1558)})
        self.assertEqual(len(TABLE['sources']), 31, '4 B11 skills + 27 herb skills')


class Herbs(unittest.TestCase):
    """P5-D17/P5-D19: the 48 herb item skills (Z lane, api-inbox 2026-11-01)."""

    BLOCKED = {
        2245: ['effect:MpPerMax'], 2246: ['cond:using_kind:ETC'], 2279: ['effect:MpPerMax'],
        2485: ['scope:PARTY', 'scope_filter:FRIEND'], 2486: ['effect:MpPerMax', 'scope:PARTY', 'scope_filter:FRIEND'],
        2512: ['stat:absorbDam'], 2513: ['cond:player:transformationId'], 2580: ['stat:vitalityConsumeRate'],
        2900: ['cond:player:level'], 2901: ['cond:player:level', 'effect:BlockDamage', 'effect:BlockDebuff'],
        2902: ['cond:player:level'], 2903: ['cond:player:level'], 9162: ['stat:absorbDam'],
        22004: ['stat:vitalityConsumeRate'], 22005: ['effect:VitalityPointUp'], 22030: ['stat:vitalityConsumeRate'],
        22097: ['effect:MaxCp'], 22098: ['effect:MaxCp'], 22158: ['operate:A4'], 22160: ['stat:absorbDam'],
        22162: ['stat:vitalityConsumeRate'],
    }

    @classmethod
    def setUpClass(cls):
        content, cls.counts = gen_skill_promotions.generate(DP, herbs=True)
        cls.table = tomllib.loads(content)
        assert cls.table == TABLE, 'HERBS_BOUND: the committed table carries the herb rows'

    def test_the_48_skills_split_into_clean_additions_and_reported_blockers(self):
        herbs = gen_skill_promotions.herb_skill_ids()
        self.assertEqual(len(herbs), 48)
        clean = [a['skill']['id'] for a in self.table['additions'] if a['skill']['id'] in herbs]
        self.assertEqual(len(clean), 27)
        self.assertEqual({int(k): v for k, v in self.counts['blocked_herbs'].items()}, self.BLOCKED)
        self.assertEqual(sorted(clean + list(self.BLOCKED)), herbs)
        ids = [a['skill']['id'] for a in self.table['additions']]
        self.assertEqual(ids, sorted(set(ids)))
        self.assertFalse(floats(self.table))

    def test_clean_rows_are_executable_v6_rows_with_empty_clears_for_every_level(self):
        for row in self.table['additions']:
            skill = row['skill']
            if skill['id'] in (2097, 2098):
                continue
            with self.subTest(skill=skill['id']):
                self.assertFalse((DATA / f'skills-phase4/{skill["id"]}.toml').exists())
                self.assertEqual((row['clears'], skill['blockers'], skill['runtime_status']), ([], [], 'executable'))
                self.assertLessEqual(SKILL_KEYS - {'conditions'}, set(skill) | {'conditions'})
        levels = {a['skill']['id']: a['skill']['max_level'] for a in self.table['additions']}
        self.assertEqual((levels[2278], levels[22128], levels[2244]), (3, 3, 1))

    def test_herb_sources_point_at_the_pinned_skill_lines(self):
        herbs = set(gen_skill_promotions.herb_skill_ids())
        anchors = {src['skill_id']: (src['file'], src['line']) for src in self.table['sources'] if src['skill_id'] in herbs}
        self.assertEqual(len(anchors), 27)
        for sid, (rel, line) in anchors.items():
            with self.subTest(skill=sid):
                self.assertIn(f'<skill id="{sid}"', (DP / rel).read_text().splitlines()[line - 1])
        # Spot checks against dp 3ca488dd (file:line, levels).
        self.assertEqual(anchors[2278][0], 'src/main/resources/data/stats/skills/02200-02299.xml')
        self.assertIn('levels="3"', (DP / anchors[2278][0]).read_text().splitlines()[anchors[2278][1] - 1])
        self.assertIn('levels="3"', (DP / anchors[22128][0]).read_text().splitlines()[anchors[22128][1] - 1])


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
