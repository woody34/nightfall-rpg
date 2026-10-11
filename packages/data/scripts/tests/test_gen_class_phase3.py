"""Independent XML audit, source admission and additive-only generator checks."""
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import tomllib
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

from packages.data.scripts import gen_class_phase3 as GEN
from packages.data.scripts.skill_oracle import generate, generate_class_phase3

DATA = Path(__file__).resolve().parents[2]
DP = Path("/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp")
GAME = DP.parent / "l2j-game"


class SourceAdmissionTests(unittest.TestCase):
    def test_wrong_revision_and_dirty_paths_are_rejected(self):
        with patch.object(GEN.subprocess, "run", return_value=SimpleNamespace(stdout="f" * 40)) as run:
            with self.assertRaisesRegex(GEN.legacy.SourceError, "expected pinned"):
                GEN.admit(Path("/unused"), GEN.GAME_REV, [GEN.CHAR])
            self.assertEqual(run.call_count, 1)
        for dirty in (" M", "??", "!!"):
            with self.subTest(status=dirty), patch.object(GEN.subprocess, "run", side_effect=[
                    SimpleNamespace(stdout=GEN.GAME_REV), SimpleNamespace(stdout=f"{dirty} {GEN.CHAR}\n")]) as run:
                with self.assertRaisesRegex(GEN.legacy.SourceError, "source inputs are dirty"):
                    GEN.admit(Path("/unused"), GEN.GAME_REV, [GEN.CHAR])
                self.assertIn("--ignored=matching", run.call_args.args[0])
                self.assertIn(GEN.CHAR, run.call_args.args[0])

    def test_missing_regen_is_rejected_instead_of_defaulted(self):
        with self.assertRaisesRegex(GEN.legacy.SourceError, "missing/invalid"):
            GEN.decimal(None, "level 85 hpRegen")
        for value in ("NaN", "-1", "1e2"):
            with self.assertRaises(GEN.legacy.SourceError):
                GEN.decimal(value, "regen")


class ProfileCorpusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.files, cls.section = GEN.generate(DP, GAME)
        cls.profiles = {p['id']: p for rel, content in cls.files.items()
                        if rel.startswith('classes-phase3/') and (p := tomllib.loads(content))}

    def test_all_89_profiles_and_22695_regen_literals_match_xml(self):
        self.assertEqual(set(self.profiles), set(range(58)) | set(range(88, 119)))
        count = 0
        for cid, p in self.profiles.items():
            with self.subTest(class_id=cid):
                xml = ET.parse(DP / p['source']['file']).getroot()
                sd = xml.find('staticData')
                self.assertEqual(int(xml.findtext('classId')), cid)
                profession = tomllib.loads((DATA / f"professions/{p['key']}.toml").read_text())
                for key in ('id', 'key', 'parent', 'base_class_id', 'tier', 'min_level', 'base_stats'):
                    self.assertEqual(p.get(key), profession.get(key))
                self.assertEqual(p['magical']['base_m_atk'], int(sd.findtext('baseMAtk')))
                slots = {k: int(sd.findtext(f'baseMDef/{k}')) for k in GEN.SLOTS}
                self.assertEqual(p['magical']['m_def_slots_xml'], slots)
                self.assertEqual(p['magical']['base_m_def'], slots['rear'] + slots['lear'] + 2 * slots['rfinger'] + slots['neck'])
                self.assertEqual(p['magical']['m_def_slots']['lfinger'], slots['rfinger'])
                self.assertEqual(p['magical']['base_m_atk_speed'], 333)
                self.assertIsNone(sd.find('baseMAtkSpd'))
                self.assertEqual(p['combat_getter_bases']['accuracy'], 0)
                self.assertEqual(p['combat_getter_bases']['evasion'], 0)
                self.assertEqual(p['regen']['kind'], 'per_level')
                self.assertEqual(len(p['regen']['levels']), 85)
                for number, (row, node) in enumerate(zip(p['regen']['levels'], xml.findall('lvlUpgainData/level')), 1):
                    self.assertEqual(row['level'], number)
                    self.assertEqual(int(node.attrib['val']), number)
                    for key, tag in (('hp_regen', 'hpRegen'), ('mp_regen', 'mpRegen'), ('cp_regen', 'cpRegen')):
                        self.assertEqual(row[key], node.findtext(tag))
                        count += 1
        self.assertEqual(count, 22695)
        self.assertEqual(self.profiles[34]['parent'], 32)
        self.assertEqual(self.profiles[104]['parent'], 28)

    def test_regen_changes_with_level_despite_shared_class_curves(self):
        curves = {tuple((r['hp_regen'], r['mp_regen'], r['cp_regen']) for r in p['regen']['levels'])
                  for p in self.profiles.values()}
        self.assertEqual(len(curves), 1)
        for p in self.profiles.values():
            rows = p['regen']['levels']
            self.assertEqual([rows[l - 1]['hp_regen'] for l in (1, 20, 40, 85)], ['2.0', '3.4', '5.4', '9.9'])
            self.assertEqual([rows[l - 1]['mp_regen'] for l in (1, 20, 40, 85)], ['0.9', '1.2', '1.8', '3.0'])
            self.assertEqual([rows[l - 1]['cp_regen'] for l in (1, 20, 40, 85)], ['2.0', '2.5', '4.5', '8.5'])

    def test_missing_level_and_rfingers_source_quirk(self):
        c = GEN.legacy.collect_classes_and_growth(GEN.legacy.Datapack(DP))[0][0]
        xml = ET.parse(DP / c['source_file']).getroot()
        xml.find('staticData/baseMDef/lfinger').text = '99'
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / c['source_file']
            path.parent.mkdir(parents=True)
            # Retain the pinned line layout for per-value provenance checks.
            original = (DP / c['source_file']).read_text()
            path.write_text(original.replace('<lfinger>5</lfinger>', '<lfinger>99</lfinger>'))
            p = tomllib.loads(GEN.class_profile(c, root, GEN.game_defaults(GAME)))
            self.assertEqual(p['magical']['m_def_slots_xml']['lfinger'], 99)
            self.assertEqual(p['magical']['m_def_slots']['lfinger'], 5)
            self.assertEqual(p['magical']['base_m_def'], 41)
            xml.find('lvlUpgainData').remove(xml.findall('lvlUpgainData/level')[-1])
            path.write_text(ET.tostring(xml, encoding='unicode'))
            with self.assertRaisesRegex(GEN.legacy.SourceError, 'contiguous regen levels'):
                GEN.class_profile(c, root, GEN.game_defaults(GAME))

    def test_keltir_literal_and_template_getter_semantics(self):
        p = tomllib.loads(self.section)['phase3']
        self.assertEqual(p['xml']['m_atk'], '5.78704')
        self.assertEqual(p['xml']['m_def'], '29.5916164000214')
        self.assertEqual(p['template']['m_atk'], 5)
        self.assertEqual(p['template']['m_def'], 29)
        self.assertEqual(p['template']['m_atk_speed'], 333)
        f32 = struct.unpack('!f', struct.pack('!f', 0.9))[0]
        self.assertEqual(F(p['template']['mp_regen']), F(*f32.as_integer_ratio()))
        self.assertEqual(p['regen_kind'], 'scalar')
        self.assertEqual(p['cp_regen_kind'], 'not_applicable')
        self.assertNotIn('cp_regen', p['template'])

    def test_check_detects_drift_orphans_and_preserves_npc_prefix(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            (out / 'npcs').mkdir()
            prefix = '# existing custom template\nid = "keltir"\nmax_hp = 44\n'
            (out / 'npcs/keltir.toml').write_text(prefix)
            self.assertEqual(GEN.write_outputs(out, self.files, self.section, False), [])
            self.assertTrue((out / 'npcs/keltir.toml').read_text().startswith(prefix + GEN.MARKER))
            self.assertEqual(GEN.write_outputs(out, self.files, self.section, True), [])
            before = (out / 'npcs/keltir.toml').read_bytes()
            profile = out / 'classes-phase3/human_fighter.toml'
            profile.write_text(profile.read_text().replace('hp_regen = "2.0"', 'hp_regen = "99"', 1))
            (out / 'classes-phase3/orphan.toml').write_text('schema = 1\n')
            stale = GEN.write_outputs(out, self.files, self.section, True)
            self.assertIn('classes-phase3/human_fighter.toml', stale)
            self.assertIn('unexpected file: classes-phase3/orphan.toml', stale)
            self.assertEqual((out / 'npcs/keltir.toml').read_bytes(), before)
            weapon = out / 'tables-phase3/starter_weapon.toml'
            weapon.write_text(weapon.read_text().replace('value = "5"', 'value = "6"'))
            (out / 'tables-phase3/orphan.toml').write_text('schema = 1\n')
            stale = GEN.write_outputs(out, self.files, self.section, True)
            self.assertIn('tables-phase3/starter_weapon.toml', stale)
            self.assertIn('unexpected file: tables-phase3/orphan.toml', stale)
            self.assertIn('value = "6"', weapon.read_text())
            (out / 'npcs/keltir.toml').write_text(before.decode().replace('m_atk = 5\n', 'm_atk = 6\n'))
            self.assertIn('npcs/keltir.toml:[phase3]', GEN.write_outputs(out, self.files, self.section, True))


class TemplateOracleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.golden, cls.text = generate_class_phase3()

    def test_fixture_matches_and_hashes_cover_actual_inputs(self):
        self.assertEqual((DATA / 'fixtures/skill-oracle-class-phase3.json').read_text(), self.text)
        for rel, digest in self.golden['inputs_sha256'].items():
            self.assertEqual(hashlib.sha256((DATA / rel).read_bytes()).hexdigest(), digest)
        digest = hashlib.sha256(json.dumps(self.golden['formulas'], sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        self.assertEqual(digest, self.golden['vector_set_sha256'])

    def test_existing_skill_oracle_matches_including_manifest(self):
        calculated, text = generate()
        fixture = (DATA / 'fixtures/skill-oracle.json').read_text()
        self.assertEqual(calculated, json.loads(fixture))
        self.assertEqual(text, fixture)

    def test_every_requested_class_level_and_posture_is_present(self):
        expected = {(c, l) for c in self.golden['coverage']['class_keys'] for l in (1, 20, 40, 85)}
        for name, rows in self.golden['formulas'].items():
            players = [v for v in rows if v['inputs']['is_player']]
            self.assertEqual({(v['inputs']['class_key'], v['inputs']['level']) for v in players}, expected)
            self.assertEqual(len(players), 72 if name.startswith('calc') else 24)
            for v in players:
                self.assertEqual(v['inputs']['rolls'], [])

    def test_independent_source_formula_and_fractional_accumulation(self):
        bonus_table = tomllib.loads((DATA / 'tables/stat_bonus.toml').read_text())
        for name, rows in self.golden['formulas'].items():
            for v in rows:
                i = v['inputs']
                if i['is_player']:
                    profile = tomllib.loads((DATA / i['template_file']).read_text())
                    stat = i['stat_name'].lower()
                    bonus = F(bonus_table[stat]['bonus'][profile['base_stats'][stat]])
                    lm = F(i['level'] + 89, 100)
                    if name == 'FuncMAtkMod':
                        expected = profile['magical']['base_m_atk'] * bonus * bonus * lm * lm
                    elif name == 'FuncMDefMod':
                        expected = profile['magical']['base_m_def'] * bonus * lm
                    elif name == 'FuncMAtkSpeed':
                        expected = min(F(1999), profile['magical']['base_m_atk_speed'] * bonus)
                    else:
                        resource = {'calcHpRegen': 'hp', 'calcMpRegen': 'mp', 'calcCpRegen': 'cp'}[name]
                        base = F(profile['regen']['levels'][i['level'] - 1][resource + '_regen'])
                        factor = {'standing': F(11, 10), 'running': F(7, 10), 'walking': F(1)}[i['posture']]
                        expected = max(F(1), base * lm * bonus) * factor
                else:
                    expected = max(F(1), F(i['base_regen']))
                self.assertEqual(F(v['exact_rational']), expected)
                self.assertEqual(v['q_result'], expected.numerator * 1000000 // expected.denominator)
                if name.startswith('calc'):
                    for n, pulse in enumerate(v['pulse_trace'], 1):
                        total = n * v['q_result']
                        self.assertEqual(pulse['current'], total // 1000000)
                        self.assertEqual(pulse['remainder_q'], total % 1000000)


if __name__ == '__main__':
    unittest.main()
