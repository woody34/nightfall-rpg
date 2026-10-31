"""C-lane verification of the Phase 5 drop oracle families (plan story 1.5; design §14.1 D-C..D-T).

Three independent checks, none of which re-runs the oracle's own arithmetic:
1. a plain-float recount straight from the pinned NPC XML reproduces every D-C/D-E fact the
   committed golden `economy-oracle-drops.json` records (groups, exact-100 sets, fall-offs,
   off-sum groups, min==max rows);
2. every pinned source line the oracle cites still holds the expression it claims (file:line
   anchors against the pinned clones named in `tables-phase5/sources.toml`);
3. the oracle's module-level functions agree with CPython binary64 references on the
   boundaries that matter (strict > 100, E15 threshold, fmod remainder, killer windows,
   decay grid, PK exclusions).

Run from the repo root: `pytest packages/data/scripts/tests/test_economy_oracle_drops.py`.
Needs the pinned clones under ~/.cache/nightfall/l2j (the oracle fails closed without them).
"""
from fractions import Fraction as F
import glob
import json
import math
import struct
import unittest
import xml.etree.ElementTree as ET

from packages.data.scripts import economy_oracle as eo

GOLDEN = json.loads((eo.FIXTURES / "economy-oracle-drops.json").read_text(encoding="utf-8"))


def _tag(element) -> str:
    return element.tag.split("}")[-1]


def _umap_float(v: int, lo: int, hi: int, gap: int) -> float:
    """Util.map(double) :785-788 followed by the `/ 100` of IKillerChanceModifierStrategy :40/:48/:50."""
    x = float(min(max(v, lo), hi))
    return ((((x - lo) * (100.0 - gap)) / (hi - lo)) + gap) / 100


def recount_from_pinned_xml() -> dict:
    """Plain CPython floats over the pinned NPC XML; no oracle helper is used."""
    ls = set()
    for d in range(-20, 3):
        ls.add(_umap_float(d, -10, -5, 10))
        ls.add(_umap_float(d, -15, -8, 10))
        ls.add(max(0.0, min(1.0, (d * 0.15) + 1)))
    out = {"files": 0, "corpse_files": 0, "death_groups": 0, "multi": 0, "single": 0, "ungrouped_death": 0,
           "exact_100": 0, "below": set(), "above": set(), "falloff_L1": 0, "off_sum_multi": 0,
           "off_sum_single": 0, "min_eq_max": 0, "distinct_L": len(ls)}
    for path in sorted(glob.glob(str(eo.DP / eo.DPD / "stats/npcs/*.xml"))):
        out["files"] += 1
        has_corpse = False
        for npc in ET.parse(path).getroot().iter():
            if _tag(npc) != "npc":
                continue
            nid = int(npc.get("id"))
            drop_lists = next((c for c in npc if _tag(c) == "dropLists"), None)
            if drop_lists is None:
                continue
            for scope in drop_lists:
                scope_name = _tag(scope)
                has_corpse = has_corpse or scope_name == "corpse"
                for node in scope:
                    kind = _tag(node)
                    if kind == "item":
                        out["ungrouped_death"] += scope_name == "death"
                        out["min_eq_max"] += node.get("min") == node.get("max")
                        continue
                    if kind != "group":
                        continue
                    items = [c for c in node if _tag(c) == "item"]
                    out["min_eq_max"] += sum(1 for i in items if i.get("min") == i.get("max"))
                    if scope_name != "death":
                        continue
                    out["death_groups"] += 1
                    inner = sum(F(i.get("chance")) for i in items)
                    if len(items) == 1:
                        out["single"] += 1
                        out["off_sum_single"] += inner != 100
                        continue
                    out["multi"] += 1
                    out["off_sum_multi"] += inner != 100
                    # normalizeMe at L = 1, evaluated left to right as the Java does
                    g = float(node.get("chance"))
                    raw = [(float(i.get("chance")) * g) * 1.0 for i in items]
                    total = 0.0
                    for x in raw:
                        total += x / 100
                    exact = sum(F(i.get("chance")) * F(node.get("chance")) / 100 for i in items)
                    if exact == 100:
                        out["exact_100"] += 1
                        if total < 100:
                            out["below"].add(nid)
                        elif total > 100:
                            out["above"].add(nid)
                    if total > 0:
                        acc = 0.0
                        for x in raw:
                            acc += x / total
                        out["falloff_L1"] += acc < 100
        out["corpse_files"] += has_corpse
    return out


class PinnedXmlRecountTests(unittest.TestCase):
    """Check 1: the golden's D-C/D-E facts from an independent float pass over the pinned XML."""

    @classmethod
    def setUpClass(cls):
        cls.r = recount_from_pinned_xml()

    def test_design_1_10_counts(self):
        # design §1.10 / plan story 1.2 Done-When: 124 files, 14,589 death groups, 744 ungrouped death
        # items, 35 files with a corpse list (AD5 E1).
        self.assertEqual(self.r["files"], 124)
        self.assertEqual(self.r["death_groups"], 14589)
        self.assertEqual(self.r["ungrouped_death"], 744)
        self.assertEqual(self.r["corpse_files"], 35)

    def test_de_summary_matches_golden(self):
        de = GOLDEN["de_summary"]
        self.assertEqual(self.r["multi"], de["groups"])
        self.assertEqual(self.r["multi"], 11557)
        self.assertEqual(self.r["exact_100"], de["exact_100"])
        self.assertEqual(sorted(self.r["below"]), de["below_100_npcs"])
        self.assertEqual(sorted(self.r["above"]), de["above_100_npcs"])
        self.assertEqual(sorted(self.r["below"]), [29068, 29150, 29186])
        self.assertEqual(sorted(self.r["above"]), [29028, 29068, 29118, 29163])
        self.assertEqual(self.r["falloff_L1"], de["falloff_L1"])
        self.assertEqual(self.r["falloff_L1"], 1127)
        self.assertEqual(self.r["distinct_L"], len(de["L_values"]))
        self.assertEqual(self.r["distinct_L"], 19)

    def test_dc_facts_match_golden(self):
        self.assertEqual(self.r["single"], 3032)
        self.assertEqual(self.r["single"], GOLDEN["dm_single_item_g15"]["groups"])
        self.assertEqual(self.r["off_sum_multi"], len(GOLDEN["dc_inner_sum_not_100"]))
        self.assertEqual((self.r["off_sum_multi"], self.r["off_sum_single"]), (150, 1))
        min_eq_max = next(v for v in GOLDEN["dc_vectors"] if v["case"] == "min_equals_max")
        self.assertEqual(self.r["min_eq_max"], min_eq_max["rows"])
        self.assertEqual(self.r["min_eq_max"], 56117)


class PinnedCitationTests(unittest.TestCase):
    """Check 2: the pinned lines the oracle's D families cite say what the oracle claims."""

    def line(self, rel: str, number: int) -> str:
        return eo.game_text(rel).splitlines()[number - 1]

    def test_pinned_revisions(self):
        repos = eo.table("sources.toml")["repositories"]
        self.assertEqual(repos["game"]["revision"], "abfde0490ac52a2106e884b7242c0276c44cdf76")
        self.assertEqual(repos["datapack"]["revision"], "3ca488dd2bd0bfaca43e378886a3c2e37968153a")

    def test_ungrouped_strategy(self):
        rel = eo.GJ + "model/drops/strategy/IDropCalculationStrategy.java"
        self.assertIn("chance > (Rnd.nextDouble() * 100)", self.line(rel, 34))
        self.assertIn("item.isPreciseCalculated() && (chance > 100)", self.line(rel, 36))  # strict
        self.assertIn("(chance % 100) > (Rnd.nextDouble() * 100)", self.line(rel, 38))
        self.assertIn("Rnd.get(item.getMin(victim), item.getMax(victim)) * amountMultiply", self.line(rel, 43))

    def test_grouped_strategy(self):
        rel = eo.GJ + "model/drops/strategy/IGroupedItemDropCalculationStrategy.java"
        self.assertIn("(item1.getChance() * dropItem.getChance()) / 100", self.line(rel, 53))
        self.assertIn("dropItem.getKillerChanceModifierStrategy()", self.line(rel, 54))  # group's L
        self.assertIn("normalized.getChance() > (Rnd.nextDouble() * 100)", self.line(rel, 65))
        self.assertIn("totalChance > random", self.line(rel, 71))
        self.assertIn("(normalized.getChance() >= 100)", self.line(rel, 73))  # >= for groups

    def test_killer_modifier_strategy(self):
        rel = eo.GJ + "model/drops/strategy/IKillerChanceModifierStrategy.java"
        self.assertIn("Math.max(0, Math.min(1, (levelDifference * 0.15) + 1))", self.line(rel, 37))
        self.assertIn("Util.map(levelDifference, -npc().getDropItemMaxLevelDifference()", self.line(rel, 40))
        self.assertIn("Inventory.ADENA_ID", self.line(rel, 47))
        self.assertIn("getDropAdenaMinLevelGapChance(), 100.0) / 100", self.line(rel, 48))
        self.assertIn("(((input - inputMin) * (outputMax - outputMin)) / (inputMax - inputMin)) + outputMin",
                      self.line(eo.GJ + "util/Util.java", 787))
        for key, value in (("UseDeepBlueDropRules", "True"), ("UseDeepBlueDropRulesRaid", "True"),
                           ("DropItemMinLevelDifference", "5"), ("DropItemMaxLevelDifference", "10"),
                           ("DropItemMinLevelGapChance", "10"), ("DropAdenaMinLevelDifference", "8"),
                           ("DropAdenaMaxLevelDifference", "15"), ("DropAdenaMinLevelGapChance", "10")):
            self.assertEqual(eo.prop("npc.properties", key), value, key)

    def test_chance_multiplier_strategy(self):
        rel = eo.GJ + "model/drops/strategy/IChanceMultiplierStrategy.java"
        self.assertIn("float multiplier = 1;", self.line(rel, 50))
        self.assertIn("Float dropChanceMultiplier = rates().getDropChanceMultiplierByItemId()", self.line(rel, 54))
        self.assertIn("hasExImmediateEffect()", self.line(rel, 57))
        self.assertIn("victim.isRaid()", self.line(rel, 59))
        self.assertIn("multiplier *= defaultMultiplier", self.line(rel, 62))
        for key, value in (("DropChanceMultiplierByItemId", "57,1"), ("HerbDropChanceMultiplier", "1"),
                           ("RaidDropChanceMultiplier", "1"), ("DeathDropChanceMultiplier", "1"),
                           ("CorpseDropChanceMultiplier", "1"), ("DeathDropAmountMultiplier", "1")):
            self.assertEqual(eo.prop("rates.properties", key), value, key)

    def test_death_order_and_positions(self):
        attackable = eo.GJ + "model/actor/L2Attackable.java"
        self.assertIn("if (isSpoiled())", self.line(attackable, 838))
        self.assertIn("DropListScope.CORPSE", self.line(attackable, 839))
        self.assertIn("DropListScope.DEATH", self.line(attackable, 842))
        npc = eo.GJ + "model/actor/L2Npc.java"
        self.assertIn("RANDOM_ITEM_DROP_LIMIT = 70", self.line(npc, 116))
        self.assertIn("Rnd.get((RANDOM_ITEM_DROP_LIMIT * 2) + 1)) - RANDOM_ITEM_DROP_LIMIT", self.line(npc, 1462))
        self.assertIn("item.isStackable()", self.line(npc, 1491))

    def test_pk_drop_lines(self):
        rel = eo.GJ + "model/actor/instance/L2PcInstance.java"
        self.assertIn("(getKarma() > 0) && (getPkKills() >= pkLimit)", self.line(rel, 4496))
        self.assertIn("(dropPercent > 0) && (Rnd.get(100) < dropPercent)", self.line(rel, 4511))
        self.assertIn("itemDrop.isShadowItem() || itemDrop.isTimeLimitedItem() || !itemDrop.isDroppable()",
                      self.line(rel, 4525))
        self.assertIn("getNonDroppableItems().contains", self.line(rel, 4527))
        self.assertIn("unEquipItemInSlot", self.line(rel, 4534))
        self.assertIn("Rnd.get(100) < itemDropPercent", self.line(rel, 4540))
        self.assertIn("++dropCount >= dropLimit", self.line(rel, 4549))
        self.assertIn("(getX() + Rnd.get(50)) - 25", self.line(rel, 3179))
        self.assertIn("!isAugmented() && _item.isDroppable()",
                      self.line(eo.GJ + "model/items/instance/L2ItemInstance.java", 670))
        for name, key, value in (("rates.properties", "KarmaRateDrop", "40"), ("rates.properties", "KarmaDropLimit", "10"),
                                 ("rates.properties", "KarmaRateDropItem", "50"), ("rates.properties", "KarmaRateDropEquip", "40"),
                                 ("rates.properties", "KarmaRateDropEquipWeapon", "10"), ("rates.properties", "PlayerRateDrop", "0"),
                                 ("pvp.properties", "MinimumPKRequiredToDrop", "6")):
            self.assertEqual(eo.prop(name, key), value, key)

    def test_decay_lines(self):
        rel = eo.GJ + "ItemsAutoDestroy.java"
        self.assertIn("scheduleGeneralAtFixedRate(this::removeItems, 5000, 5000)", self.line(rel, 35))
        self.assertIn("(curtime - item.getDropTime()) > item.getItem().getAutoDestroyTime()", self.line(rel, 58))
        self.assertIn("(curtime - item.getDropTime()) > general().getAutoDestroyHerbTime()", self.line(rel, 67))
        self.assertIn("(curtime - item.getDropTime()) > sleep", self.line(rel, 78))
        for key, value in (("AutoDestroyDroppedItemAfter", "600"), ("AutoDestroyHerbTime", "60"),
                           ("DestroyPlayerDroppedItem", "False")):
            self.assertEqual(eo.prop("general.properties", key), value, key)
        self.assertEqual(eo.items_xml()[9849]["sets"]["auto_destroy_time"], "10")


class OracleFunctionTests(unittest.TestCase):
    """Check 3: the module-level functions against CPython binary64 references at the boundaries."""

    def test_e15_threshold_is_exact_at_the_percent_boundary(self):
        # success iff r / 2^64 < c / 100 for one u64 draw r
        for c in (F(1), F("37.5"), F(100), F("188.85499572753906")):
            t = eo.chance_threshold(c)
            self.assertEqual(t, min(eo.TWO64, -(-(c.numerator * eo.TWO64) // (100 * c.denominator))))
            if t < eo.TWO64:
                self.assertTrue(eo.chance(t - 1, c))
                self.assertFalse(eo.chance(t, c))
        self.assertEqual(eo.chance_threshold(F(100)), eo.TWO64)  # 100 % always passes
        self.assertEqual(eo.chance_threshold(F(0)), 0)
        self.assertTrue(eo.chance_bound(eo.chance_threshold(F(40)) - 1, 40, 100))
        self.assertFalse(eo.chance_bound(eo.chance_threshold(F(40)), 40, 100))

    def test_strict_above_100_and_fmod_remainder(self):
        for s in ("188.85499572753906", "120", "100", "99.99999999999999", "250.5"):
            c = eo.cd(s)
            self.assertEqual(c, F(float(s)))  # Double.parseDouble
            self.assertEqual(eo.remainder(c), F(math.fmod(float(s), 100.0)))
            self.assertEqual(int(c) // 100, int(float(s)) // 100)
        self.assertEqual(eo.remainder(F(100)), 0)

    def test_killer_windows_equal_the_java_double_expressions(self):
        for d in range(-20, 3):
            self.assertEqual(eo.killer_l("item_window", d), F(_umap_float(d, -10, -5, 10)), d)
            self.assertEqual(eo.killer_l("adena_window", d), F(_umap_float(d, -15, -8, 10)), d)
            self.assertEqual(eo.killer_l("raid_clamp", d), F(max(0.0, min(1.0, (d * 0.15) + 1))), d)
        self.assertEqual([eo.killer_l("item_window", d) for d in range(-4, -12, -1)],
                         [F(1), F(1), F(0.82), F(0.64), F(0.46), F(0.28), F(0.1), F(0.1)])
        self.assertEqual(eo.killer_l("raid_clamp", -7), 0)
        golden = GOLDEN["dl_vectors"]
        self.assertEqual(golden["item_window"]["-6"], struct.pack(">d", 0.82).hex())
        self.assertEqual(golden["raid_clamp"]["-7"], struct.pack(">d", 0.0).hex())

    def test_single_item_collapse_and_multiplier_at_g1(self):
        for item_chance, group_chance in (("70", "0.5"), ("100", "188.85499572753906"), ("33.3333", "0.0003000000142492354")):
            self.assertEqual(eo.single_collapse(item_chance, group_chance),
                             F((float(item_chance) * float(group_chance)) / 100))
        for item, scope, raid, branch in ((57, "death", False, "override"), (8600, "death", False, "herb"),
                                          (6656, "death", True, "raid"), (1864, "death", False, "death"),
                                          (1864, "corpse", False, "corpse")):
            got_branch, m = eo.m_branch(item, scope, raid, "1")
            self.assertEqual((got_branch, m), (branch, F(1)))

    def test_decay_grid(self):
        # ItemsAutoDestroy: 5 s scanner, strict `>`; first grid tick T with T - dropped > limit.
        self.assertEqual([eo.decay_due(t, 100) for t in (0, 1, 49, 50, 51)], [150, 150, 150, 200, 200])
        self.assertEqual([eo.decay_due(t, 600) for t in (0, 1, 49, 50, 51)], [650, 650, 650, 700, 700])
        self.assertEqual([eo.decay_due(t, 6000) for t in (0, 1, 49, 50, 51)], [6050, 6050, 6050, 6100, 6100])
        dt = GOLDEN["dt_vectors"]
        self.assertEqual(dt["auto_destroy_9849"]["50"], 200)
        self.assertEqual(dt["loot"]["1"], 6050)
        self.assertIsNone(dt["pk_drop"])

    def test_pk_exclusions_follow_l2pcinstance_4525_4527(self):
        pk = eo.table("ground.toml")["pk"]
        self.assertEqual((len(pk["non_droppable_items"]), len(set(pk["non_droppable_items"])), len(pk["pet_items"])),
                         (30, 29, 11))
        self.assertTrue(eo.pk_excluded({"template": 57}))                      # adena
        self.assertTrue(eo.pk_excluded({"template": 1864, "augmented": True})) # !isDroppable via isAugmented
        self.assertTrue(eo.pk_excluded({"template": pk["non_droppable_items"][0]}))
        self.assertTrue(eo.pk_excluded({"template": pk["pet_items"][0]}))
        self.assertFalse(eo.pk_excluded({"template": 1864}))
        vec = GOLDEN["dp_vectors"]["break_at_10"][0]
        self.assertEqual(len(vec["dropped"]), 10)
        self.assertTrue(all(-25 <= d[1] <= 24 and -25 <= d[2] <= 24 for d in vec["dropped"]))

    def test_width_golden_is_within_the_u256_bound(self):
        dw = GOLDEN["dw_widths"]
        self.assertEqual(dw["G=1"], 141)
        self.assertLessEqual(max(dw["G=2^32"], dw["G=2^-32"]), 255)
        # the width model: both sides of r*100*D < N*2^64 for a binary64 c = N/D
        c = eo.cd("188.85499572753906")
        self.assertEqual(eo._width_terms(c), max(((eo.TWO64 - 1) * 100 * c.denominator).bit_length(),
                                                 (c.numerator * eo.TWO64).bit_length()))


if __name__ == "__main__":
    unittest.main()
