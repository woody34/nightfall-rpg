"""Independent pinned-source audits and additive fixture regressions."""
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import re
import tomllib
import unittest
import xml.etree.ElementTree as ET
from packages.data.scripts.npc_phase3 import skill_oracle as O

GAME = Path('/home/matt-woodruff/.cache/nightfall/l2j/l2j-game')
DP = GAME.parent / 'l2j-dp'
JAVA = GAME / 'src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java'


class NpcPhase3Tests(unittest.TestCase):
    def test_every_config_literal_and_effective_row(self):
        p = tomllib.loads((O.ROOT / 'tables-phase3/npc_level_penalty.toml').read_text())
        for key, row in p['config'].items():
            src = row['source']
            line = (GAME / src['file']).read_text().splitlines()[int(src['lines']) - 1]
            values = re.fullmatch(rf'{key}\s*=\s*(.*)', line).group(1).split(',')
            self.assertEqual(row['values'], values)
            self.assertEqual(row['values_q'], [int(F(v) * 1000000) for v in values])
        rows = p['rows']
        self.assertEqual([(r['gap_min'], r['gap_max']) for r in rows], [(2, 2), (3, 3), (4, 4), (5, 5), (6, -1)])
        self.assertEqual([r['physical_q'] for r in rows], [600000, 600000, 550000, 550000, 550000])
        self.assertEqual([r['physical_critical_q'] for r in rows], [650000, 600000, 580000, 580000, 580000])
        self.assertEqual([r['magic_q'] for r in rows], [700000, 650000, 620000, 620000, 620000])
        self.assertEqual([r['magic_failure_q'] for r in rows], [1000000, 3000000, 3250000, 3500000, 3500000])
        for r in rows:
            self.assertEqual(r['physical_skill_q'], r['magic_q'] * r['physical_q'] // 1000000)
            self.assertEqual(r['physical_skill_critical_q'], r['magic_q'] * r['physical_critical_q'] // 1000000)

    def test_source_index_and_threshold_quirks(self):
        lines = JAVA.read_text().splitlines()
        for lo, hi in ((713, 731), (776, 800), (887, 898)):
            branch = '\n'.join(lines[lo - 1:hi])
            self.assertIn('getLevel() - attacker.getActingPlayer().getLevel() - 1;', branch)
            self.assertIn('getMinNPCLevelForDmgPenalty()', branch)
            self.assertIn('>= 2', branch)
        chance = '\n'.join(lines[1285:1314])
        self.assertIn('getLevel() - attacker.getActingPlayer().getLevel() - 2;', chance)
        self.assertIn('getMinNPCLevelForDmgPenalty()', chance)
        self.assertNotIn('getMinNPCLevelForMagicPenalty()', chance)
        self.assertIn('>= 3', chance)
        self.assertIn('Math.round((float) (lvlModifier * targetModifier * resModifier))', chance)
        self.assertIn('Rnd.get(100) < rate', chance)

    def test_every_hit_xml_row_and_boundaries(self):
        t = tomllib.loads((O.ROOT / 'tables-phase3/hit_condition_bonus.toml').read_text())
        xml = DP / t['source']['file']
        expected = {n.tag: int(n.get('val')) for n in ET.parse(xml).getroot()}
        self.assertEqual({r['condition']: r['percent'] for r in t['rows']}, expected)
        for r in t['rows']:
            line = xml.read_text().splitlines()[int(r['source']['lines']) - 1]
            self.assertIn(f'<{r["condition"]} val="{r["percent"]}"', line)
        fixture, _ = O.generate()
        vectors = fixture['formulas']['getConditionBonus']
        self.assertEqual(len(vectors), 60)
        for v in vectors:
            i = v['inputs']
            extra = expected['high'] if i['z_difference'] > 50 else expected['low'] if i['z_difference'] < -50 else 0
            value = F(100 + extra + expected[i['position']] + (expected['dark'] if i['night'] else 0), 100)
            self.assertEqual(F(v['exact_rational']), value)
            self.assertEqual(v['multiplier_q'], value * 1000000)

    def test_oracle_every_gate_index_and_hash(self):
        fixture, content = O.generate()
        self.assertEqual(content, O.OUT.read_text())
        for rel, digest in fixture['inputs_sha256'].items():
            self.assertEqual(hashlib.sha256((O.ROOT / rel).read_bytes()).hexdigest(), digest)
        self.assertEqual(fixture['vector_set_sha256'], hashlib.sha256(json.dumps(fixture['formulas'], sort_keys=True, separators=(',', ':')).encode()).hexdigest())
        normal, critical, skill, chance = map(lambda a: list(map(F, a)),
            (['0.7', '0.6', '0.6', '0.55'], ['0.75', '0.65', '0.6', '0.58'],
             ['0.8', '0.7', '0.65', '0.62'], ['2.5', '3', '3.25', '3.5']))
        for name, vectors in fixture['formulas'].items():
            self.assertGreaterEqual(len(vectors), 20)
            if name == 'getConditionBonus':
                continue
            for v in vectors:
                i = v['inputs']
                gap = i['target_level'] - i['acting_player_level']
                is_chance = 'Chance' in name
                gate = (i['is_attackable'] and not i['is_raid'] and not i['is_raid_minion']
                        and i['has_acting_player'] and i['target_level'] >= 78 and gap >= (3 if is_chance else 2))
                idx = min(gap - (2 if is_chance else 1), 3)
                expected = F(1)
                if gate:
                    if is_chance:
                        expected = chance[idx]
                    elif name.startswith('calcMagic'):
                        expected = skill[idx]
                    else:
                        expected = (critical if i['critical'] else normal)[idx]
                        if name.startswith('calcSkill'):
                            expected *= skill[idx]
                self.assertEqual(v['eligible'], gate)
                self.assertEqual(F(v['exact_multiplier']), expected)
                if is_chance:
                    # Source values here have exact halves/quarters; Math.round is floor(x+1/2).
                    self.assertEqual(v['success_rate'], 100 - (expected + F(1, 2)).numerator // (expected + F(1, 2)).denominator)
                    self.assertFalse(v['lands_at_rate'])
                else:
                    self.assertEqual(F(v['exact_rational']), 1001 * expected)

    def test_legacy_outputs_byte_exact_at_accepted_phase4_base(self):
        # git object reads remain in this worktree; no other checkout is accessed.
        import subprocess
        # Phase 3 accepted later oracle-provenance metadata before Phase 4 began.
        paths = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', '56e55a6', 'packages/data'], text=True).splitlines()
        for rel in paths:
            if '/scripts/' in rel or rel.endswith('SOURCES.md'):
                continue
            with self.subTest(path=rel):
                old = subprocess.check_output(['git', 'show', f'56e55a6:{rel}'])
                self.assertEqual((O.ROOT.parents[1] / rel).read_bytes(), old)


if __name__ == '__main__':
    unittest.main()
