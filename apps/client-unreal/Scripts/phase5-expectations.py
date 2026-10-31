#!/usr/bin/env python3
"""Phase 5 story 1.8: pack declarations and oracle expectation sidecars for the 36 plan §2.8 files.

Nothing here is typed by hand: every expected number is a call into
`packages/data/scripts/economy_oracle.py` (stories 1.5-1.7) or a value read from the `--check`-verified
`tables-phase5/` tables, over the pack declarations below. The declarations carry only the
oracle-relevant inputs (plan E46: roles, classes, levels, positions, inventories, recipes, party,
container rows, fixture schedule, NPC placements, `next_item_instance`); story 6.1 extends them with
harness-only fields and seeds them.

    python3 apps/client-unreal/Scripts/phase5-expectations.py [--check]

Outputs (committed, deterministic):
  Scenarios/fixtures/phase5-packs.json         the 27 §5.3 pack declarations
  Scenarios/5-<name>.expect.json               one sidecar per scenario file (36)
`--check` regenerates in memory and fails if a committed file differs. Nothing is run live (6.6).
"""
from __future__ import annotations

import argparse
from fractions import Fraction as F
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

CLIENT = Path(__file__).resolve().parents[1]
REPO = CLIENT.parents[1]
SCENARIO_DIR = CLIENT / "Scenarios"
PACKS_OUT = SCENARIO_DIR / "fixtures" / "phase5-packs.json"

_spec = importlib.util.spec_from_file_location("economy_oracle", REPO / "packages/data/scripts/economy_oracle.py")
eo = importlib.util.module_from_spec(_spec)
sys.modules["economy_oracle"] = eo
_spec.loader.exec_module(eo)

TICK_MS = 100
# Harness character UUIDs (Phase 4 used ...40/...41; Phase 5 continues at ...50).
ROLE_IDS = {"a": "01970000-0000-7000-8000-000000000050", "b": "01970000-0000-7000-8000-000000000051",
            "c": "01970000-0000-7000-8000-000000000052"}
G15 = "1.5"


def tbl(name: str) -> dict:
    return eo.table(name + ".toml")


def npc_row(npc_id: int) -> dict:
    """A drops.toml row; SP-only templates (no drop list, drops.toml sp_only_templates) are read from the
    pinned NPC XML instead, with empty lists and no exp_rate (0 XP and 0 SP under B16)."""
    row = next((n for n in tbl("drops")["npc"] if n["id"] == npc_id), None)
    if row is not None:
        return row
    if npc_id not in tbl("drops")["sp_only_templates"]:
        raise eo.OracleError(f"NPC {npc_id} has no drops.toml row")
    import xml.etree.ElementTree as ET
    for path in sorted((eo.DP / eo.DPD / "stats/npcs").glob("*.xml")):
        for npc in ET.parse(path).getroot().iter():
            if npc.tag.split("}")[-1] == "npc" and npc.get("id") == str(npc_id):
                return {"id": npc_id, "level": int(npc.get("level") or 0), "is_raid": False, "corpse_time": 7, "exp_rate": None,
                        "sp": None, "death": [], "corpse": [], "source": f"{path.name} (SP-only: no expRate)"}
    raise eo.OracleError(f"NPC {npc_id} not in the pinned XML")


def ms_ticks(ms: int) -> int:
    return ms // TICK_MS


def s_ticks(seconds: int) -> int:
    return seconds * 1000 // TICK_MS


def bits(x: F) -> str:
    return eo.bits(x)


def dec(x: F) -> str:
    return f"{x.numerator}/{x.denominator}"


# ---------------------------------------------------------------------------------------------
# Drop node expectations (D §3.2-3.4 through the oracle's chance model)
# ---------------------------------------------------------------------------------------------


def node_expectations(npc: dict, killer_level: int, g: str = "1", scope: str = "death") -> list[dict]:
    """Per drop node: the binary64 chance (bits + reduced rational) and the E15 pass threshold, in
    document order, exactly as the engine must draw them (D §2.3 sites 2/3)."""
    d = npc["level"] - killer_level
    l_group = eo.group_killer_l(npc["is_raid"], d)
    out = []
    for node in npc[scope]:
        if node["kind"] == "item":
            m = eo.m_branch(node["id"], scope, npc["is_raid"], g)[1]
            l_item = eo.nongroup_killer_l(node["id"], npc["is_raid"], d)
            c = eo.fl(l_item * eo.fl(eo.cd(node["chance"]) * m))
            out.append({"kind": "item", "template": node["id"], "min": node["min"], "max": node["max"],
                        "L_bits": bits(l_item), "m_bits": bits(m), "C_bits": bits(c), "C": dec(c),
                        "pass_threshold": str(eo.chance_threshold(c)), "precise_branch": c > 100,
                        "mult_base": int(c) // 100 if c > 100 else 1,
                        "draws_on_pass": ["chance"] + (["remainder"] if c > 100 else []) + ["quantity"]})
        elif len(node["items"]) == 1:
            it = node["items"][0]
            m = eo.m_branch(it["id"], scope, npc["is_raid"], g)[1]
            c = eo.single_item_c(it["chance"], node["chance"], l_group, m)
            out.append({"kind": "single_item_group", "template": it["id"], "min": it["min"], "max": it["max"],
                        "group_chance": node["chance"], "item_chance": it["chance"], "L_bits": bits(l_group),
                        "m_bits": bits(m), "C_bits": bits(c), "C": dec(c), "pass_threshold": str(eo.chance_threshold(c)),
                        "precise_branch": c > 100, "draws_on_pass": ["chance"] + (["remainder"] if c > 100 else []) + ["quantity"]})
        else:
            m = eo.m_branch(node["items"][0]["id"], scope, npc["is_raid"], g)[1]
            ev = eo.group_eval(node, l_group, m)
            s = ev["S"]
            row = {"kind": "group", "group_chance": node["chance"], "L_bits": bits(l_group), "m_bits": bits(m),
                   "S_bits": bits(s) if s else "0", "S": dec(s), "pass_threshold": str(eo.chance_threshold(s)),
                   "precise_branch": s >= 100, "mult_base": int(s) // 100 if s >= 100 else 1,
                   "remainder_bits": bits(eo.remainder(s)) if s >= 100 else None,
                   "items": [{"template": it["id"], "min": it["min"], "max": it["max"], "share_bits": bits(sh), "T_bits": bits(t)}
                             for it, sh, t in zip(node["items"], ev["s"] or [], ev["T"] or [])],
                   "draws_on_pass": ["group", "selection"] + (["remainder"] if s >= 100 else []) + ["quantity"]}
            out.append(row)
    return out


def ground_timing(npc_loot: bool = True) -> dict:
    g = tbl("ground")["ground"]
    return {"loot_owner_ticks": ms_ticks(g["loot_owner_ms"]) if npc_loot else 0,
            "drop_protection_ticks": ms_ticks(g["drop_protection_ms"]) if npc_loot else 0,
            "decay_limit_ticks": s_ticks(g["decay_seconds"]) if npc_loot else None,
            "herb_decay_limit_ticks": s_ticks(g["herb_decay_seconds"]),
            "scanner_grid_ticks": ms_ticks(g["scanner_ms"]), "position_offset_npc": g["offset_npc"],
            "position_offset_player": [-g["offset_player"], g["offset_player_range"] - g["offset_player"] - 1]}


# ---------------------------------------------------------------------------------------------
# Pack declarations (plan §5.3; oracle-relevant inputs only, E46)
# ---------------------------------------------------------------------------------------------


def role(name: str, level: int, pos: tuple[int, int], class_id: int = 0, inventory: list | None = None, **extra) -> dict:
    out = {"role": name, "character_id": ROLE_IDS[name], "level": level, "class_id": class_id,
           "position": list(pos), "inventory": inventory or []}
    out.update(extra)
    return out


def inv(template: int, count: int = 1, enchant: int = 0, **extra) -> dict:
    out = {"template_id": template, "count": count, "enchant": enchant}
    out.update(extra)
    return out


def monster(npc_id: int, home: tuple[int, int], count: int = 1, slot: str | None = None) -> dict:
    n = npc_row(npc_id)
    return {"slot_id": slot or f"npc_{npc_id}", "npc_id": npc_id, "level": n["level"], "is_raid": n["is_raid"],
            "home": list(home), "count": count, "source": "tables-phase5/drops.toml npc[id]"}


def fixture_npc(kind: str, npc_id: int | None, pos: tuple[int, int], name: str) -> dict:
    return {"kind": kind, "npc_id": npc_id, "name": name, "position": list(pos), "attackable": False}


def herb_npc(herb_id: int) -> int:
    for n in tbl("drops")["npc"]:
        for node in n["death"]:
            items = [node] if node["kind"] == "item" else node["items"]
            if any(i["id"] == herb_id for i in items):
                return n["id"]
    raise eo.OracleError(f"no NPC drops herb {herb_id}")


def common_rare_recipe() -> dict:
    """The lowest common recipe with a rare production if the pinned data has one; otherwise the lowest
    common recipe (rarity 0: the masterwork draw is still taken and never passes, D §8.2)."""
    common = [r for r in tbl("recipes")["recipe"] if r["type"] == "common"]
    rare = [r for r in common if "production_rare" in r]
    return min(rare or common, key=lambda r: (r["craft_level"], r["id"]))


def decay_9849_npc() -> int | None:
    for n in tbl("drops")["npc"]:
        for scope in ("death",):
            for node in n[scope]:
                items = [node] if node["kind"] == "item" else node["items"]
                if any(i["id"] == 9849 for i in items):
                    return n["id"]
    return None


MERCHANT = {"npc": 31413, "list_id": 314130, "item": 1294, "ref_price": 75, "tax": "15",
            "source": "merchants.toml entries (Merchant/Antonio/Antonio.java _sellList0)"}
RESTOCK = {"npc": 35363, "list_id": 353630, "item": 16127, "ref_price": 23000, "tax": "10", "count": 10, "restock_min": 60,
           "source": "merchants.toml entries (Chamberlain/Alfred/Alfred.java _sellList0)"}
MULTISELL = {"list_id": 616, "npc": 32347}
RECIPE_DWARF = 1        # mk_wooden_arrow, 100 %, craft level 1 (recipes.toml recipe[id=1])
RECIPE_DWARF_FAIL = 322  # q_mk_titans_powerstone, 25 %, craft level 2
AUCTION = {"instance": 32320, "auction_id": 1, "item": 9901, "init_bid": 100000}
STORE_LIMITS = {"sell_or_package": 3, "buy": 4, "manufacture": 3, "dwarf_sell_or_package": 4, "dwarf_buy": 5, "dwarf_manufacture": 4,
                "source": "D §9.3 (pinned PrivateStoreLimits; Dwarf +1)"}


def recipe(rid: int) -> dict:
    return next(r for r in tbl("recipes")["recipe"] if r["id"] == rid)


def build_packs() -> dict[str, dict]:
    pos_a, pos_b, pos_c = (100, 100), (101, 100), (100, 101)
    far = (150, 150)  # beyond PartyRange2 (1400 L2 units = 43.75 tiles) and the trade radius
    adena = lambda n: inv(57, n)  # noqa: E731
    r1, rf = recipe(RECIPE_DWARF), recipe(RECIPE_DWARF_FAIL)
    rc = common_rare_recipe()
    mats = lambda r, k=1: [inv(i["id"], i["count"] * k) for i in r["ingredients"]]  # noqa: E731
    herb_multi = herb_npc(8612)
    n9849 = decay_9849_npc()
    packs = {
        "phase5-drops": {"roles": [role("a", 81, pos_a), role("b", 81, pos_b)], "party": None,
                         "monsters": [monster(22320, (103, 100))], "note": "22320 (level 81): ungrouped 9599, adena group, multi-item groups; b deals less damage than a (maxDealer tie rule asserted by lowest EntityId)"},
        "phase5-drops-precise": {"roles": [role("a", 20, pos_a)], "monsters": [monster(18478, (103, 100), slot="group_100"), monster(18352, (103, 102), slot="ungrouped_100")],
                                 "note": "18478 (level 15) group chance 100.0 (>= 100 branch, remainder draw) vs 18352 (level 22) ungrouped 9703 at exactly 100 (strict >, no remainder draw); killer level 20 keeps d >= -5 so L = 1 for both"},
        "phase5-drops-gap": {"roles": [role("a", 11 + k, pos_a) for k in range(1)] + [role("b", 15, pos_b)],
                             "killer_levels": [11, 12, 13, 14, 15], "monsters": [monster(20003, (103, 100), count=5)],
                             "note": "20003 (level 5) killed by level 11..15 -> d = -6..-10; the harness varies role a's level per run (killer_levels)"},
        "phase5-rates": {"roles": [role("a", 5, pos_a)], "rate_multiplier_g": G15, "monsters": [monster(20003, (103, 100))]},
        "phase5-protection": {"roles": [role("a", 81, pos_a), role("b", 81, pos_b), role("c", 81, pos_c)], "party": {"members": ["a", "b"], "leader": "a", "mode": "FINDERS_KEEPERS"},
                              "monsters": [monster(22320, (103, 100))], "note": "a kills; b is a's party member; c is a stranger"},
        "phase5-herbs": {"roles": [role("a", 15, pos_a)], "monsters": [monster(18478, (103, 100), slot="single_skill_herbs"), monster(herb_multi, (103, 102), slot="multi_skill_herb")]},
        "phase5-decay": {"roles": [role("a", 15, pos_a, karma=100, pk_count=6, inventory=[inv(1864, 5)])], "monsters": [monster(18478, (103, 100))] + ([monster(n9849, (103, 102), slot="auto_destroy_9849")] if n9849 else []),
                         "note": "herb (60 s), ordinary loot (600 s)" + (", item 9849 (10 s)" if n9849 else "; no pinned NPC drops 9849 so its 100-tick limit is asserted through D-T only") + "; a's PK drop never decays"},
        "phase5-party": {"roles": [role("a", 10, pos_a), role("b", 12, pos_b), role("c", 11, far)], "party": {"members": ["a", "b", "c"], "leader": "a", "mode": "FINDERS_KEEPERS"},
                         "monsters": [monster(20003, (103, 100), count=3)], "note": "c is outside PartyRange2 and is excluded from RANDOM/BY_TURN/split"},
        "phase5-party-spoil": {"roles": [role("a", 20, pos_a, class_id=54), role("b", 20, pos_b)], "party": {"members": ["a", "b"], "leader": "a", "mode": "RANDOM_INCLUDING_SPOIL"},
                               "monsters": [monster(20003, (103, 100), count=2)], "note": "a (Dwarven Scavenger 54, class minimum level 20) spoils; b sweeps as party member"},
        "phase5-party-xp": {"roles": [role("a", 10, pos_a), role("b", 12, pos_b)], "party": {"members": ["a", "b"], "leader": "a", "mode": "FINDERS_KEEPERS"}, "monsters": [monster(20003, (103, 100))]},
        "phase5-solo-xp": {"roles": [role("a", 10, pos_a), role("b", 10, pos_b)], "monsters": [monster(20003, (103, 100)), monster(4301, (103, 102), slot="sp_only")], "note": "two solo players split damage; 4301 is SP-only (exp_rate absent)"},
        "phase5-spoil": {"roles": [role("a", 20, pos_a, class_id=54)], "monsters": [monster(20003, (103, 100))], "note": "Scavenger 54 needs level >= 20 (class minimum); d = -15 against 20003"},
        "phase5-pk-drop": {"roles": [role("a", 20, pos_a, karma=100, pk_count=6, inventory=[inv(1864, 10), inv(57, 1000)] + [inv(1869, 1) for _ in range(12)]), role("b", 20, pos_b)],
                          "note": "a is chaotic with pk_count >= 6; b kills a. The equipped weapon (10 %) and armour (40 %) rows are the starter kit's own equipped items (GrantItem cannot equip beyond the kit)"},
        "phase5-vendors": {"roles": [role("a", 20, pos_a, inventory=[adena(100000)] + [inv(1294, 1) for _ in range(13)])], "fixed_npcs": [fixture_npc("merchant", MERCHANT["npc"], (102, 100), "merchant")], "merchant": MERCHANT},
        "phase5-vendor-restock": {"roles": [role("a", 20, pos_a, inventory=[adena(1000000)])], "fixed_npcs": [fixture_npc("merchant", RESTOCK["npc"], (102, 100), "merchant")], "merchant": RESTOCK},
        "phase5-multisell": {"roles": [role("a", 20, pos_a, inventory=[inv(9417, 1, enchant=3), inv(9417, 1, enchant=0)])], "fixed_npcs": [fixture_npc("multisell", MULTISELL["npc"], (102, 100), "exchanger")], "multisell": MULTISELL},
        "phase5-craft": {"roles": [role("a", 20, pos_a, class_id=54, inventory=[inv(r1["recipe_id"], 1), inv(rf["recipe_id"], 1)] + mats(r1, 2) + mats(rf, 1), create_item_rank=2)], "recipes": {"dwarven": [RECIPE_DWARF, RECIPE_DWARF_FAIL]}},
        "phase5-craft-common": {"roles": [role("a", 20, pos_a, inventory=[inv(rc["recipe_id"], 1)] + mats(rc, 3), create_common_item_rank=max(1, rc["craft_level"]))], "recipes": {"common": [rc["id"]]}},
        "phase5-manufacture": {"roles": [role("a", 20, pos_a, class_id=54, create_item_rank=1, registered_recipes={"dwarven": [RECIPE_DWARF]}), role("b", 20, pos_b, inventory=[adena(5000)] + mats(r1, 2))], "manufacture_fee": 1000},
        "phase5-trade": {"roles": [role("a", 20, pos_a, inventory=[inv(1864, 10), adena(500)]), role("b", 20, pos_b, inventory=[inv(1869, 5)])]},
        "phase5-stores": {"roles": [role("a", 20, pos_a, inventory=[inv(1864, 10), inv(1869, 10), inv(1870, 10)]), role("b", 20, pos_b, inventory=[adena(1000), inv(1864, 3)])], "store_limits": STORE_LIMITS},
        "phase5-warehouse": {"roles": [role("a", 20, pos_a, inventory=[adena(1000), inv(1864, 10), inv(1869, 4), inv(119, 1), inv(12754, 1)])], "fixed_npcs": [fixture_npc("warehouse_keeper", None, (102, 100), "warehouse_keeper")], "warehouse_rows": [{"template_id": 1864, "count": 5}],
                            "note": "119 is augmented live through the Phase 4 blacksmith (nf.Refine with life stone 12754) before the deposit: GrantItem has no augment transport"},
        "phase5-freight": {"roles": [role("a", 20, pos_a, inventory=[adena(5000), inv(20678, 1), inv(20725, 1)]), role("b", 20, far, same_account_as="a")], "fixed_npcs": [fixture_npc("warehouse_keeper", None, (102, 100), "warehouse_keeper")]},
        "phase5-mail": {"roles": [role("a", 20, pos_a, inventory=[adena(5000), inv(1864, 10)], peace_zone=True), role("b", 20, far, inventory=[adena(2000)], peace_zone=True)]},
        "phase5-auction": {"roles": [role("a", 20, pos_a, inventory=[adena(500000)]), role("b", 20, pos_b, inventory=[adena(500000)])], "fixed_npcs": [fixture_npc("auctioneer", AUCTION["instance"], (102, 100), "auctioneer")], "auction": AUCTION,
                           "fixture_schedule": tbl("auction")["fixture_schedule"]["anchor"]},
        "phase5-manor": {"roles": [role("a", 10, pos_a, inventory=[adena(1000), inv(5125, 1)])], "fixed_npcs": [fixture_npc("manor_manager", None, (102, 100), "manor_manager")], "monsters": [monster(20003, (103, 100))],
                         "manor": {"castle_id": 1, "castle_area": 1, "seed_id": 5016, "crop_id": 5073}},
        "phase5-epoch": {"roles": [role("a", 20, pos_a, inventory=[inv(1864, 3)])], "monsters": [monster(20003, (103, 100))]},
    }
    for name, pack in packs.items():
        next_instance = 1
        for r in pack["roles"]:
            for item in r["inventory"]:
                item["instance_id"] = next_instance
                next_instance += 1
        pack["next_item_instance"] = next_instance
        pack.setdefault("monsters", [])
        pack.setdefault("fixed_npcs", [])
        pack["zone"] = "test_zone"
    return packs


# ---------------------------------------------------------------------------------------------
# Scenario expectations
# ---------------------------------------------------------------------------------------------


def exp_drop_basic(p: dict) -> dict:
    npc = npc_row(22320)
    return {"nodes_death": node_expectations(npc, 81), "nodes_corpse_if_spoiled": node_expectations(npc, 81, scope="corpse"),
            "ground": ground_timing(), "drop_actor": "maxDealer; lastAttacker if the maxDealer is offline; ties -> lowest EntityId (D §3.6)",
            "ground_spawn": {"entity_kind": 3, "entity_spawn_field": "ground_item (30)"}, "ledger_ops": ["loot", "pickup"]}


def exp_drop_precise(p: dict) -> dict:
    killer = next(r for r in p["roles"] if r["role"] == "a")["level"]
    g = node_expectations(npc_row(18478), killer)[0]
    u = next(n for n in node_expectations(npc_row(18352), killer) if n["kind"] == "item" and n["template"] == 9703)
    assert g["precise_branch"] and g["S"] == "100/1" and g["remainder_bits"] == bits(F(0))
    assert not u["precise_branch"] and u["C"] == "100/1"
    return {"group_at_100": g, "ungrouped_at_100": u,
            "rule": "ungrouped: precise iff C > 100 (strict); group: precise iff S >= 100 (remainder draw taken, remainder 0 never passes)"}


def exp_drop_gap(p: dict) -> dict:
    npc = npc_row(20003)
    return {"per_killer_level": {str(k): {"d": npc["level"] - k, "L_item_bits": bits(eo.killer_l("item_window", npc["level"] - k)),
                                          "L_adena_bits": bits(eo.killer_l("adena_window", npc["level"] - k)), "nodes": node_expectations(npc, k)}
                                 for k in p["killer_levels"]},
            "l_table_source": "tables-phase5/adaptations.toml l_table (D §3.3)"}


def exp_rates(p: dict) -> dict:
    npc = npc_row(20003)
    return {"G": G15, "G_bits": bits(eo.fl(F(G15))), "base": node_expectations(npc, npc["level"], "1"),
            "multiplied": node_expectations(npc, npc["level"], G15),
            "m_per_branch": {b: bits(eo.m_branch(i, s, r, G15)[1]) for b, (i, s, r) in
                             {"override_57": (57, "death", False), "herb": (8600, "death", False), "death": (1864, "death", False), "corpse": (1864, "corpse", False)}.items()},
            "amounts_unchanged_by_G": True}


def exp_protection(p: dict) -> dict:
    t = ground_timing()
    return {"timing": t, "check_order": ["exists", "drop_protection", "capacity (slots only; solo or FINDERS_KEEPERS)", "loot_owner", "combat flag (vacuous)"],
            "cases": {"owner_picks": "accepted", "party_member_picks_while_owner_online": "accepted", "stranger_before_expiry": "GROUND_ITEM_PROTECTED",
                      "party_member_with_owner_offline": "GROUND_ITEM_PROTECTED (loot owner needs the owner online; DropProtection would pass)",
                      "stranger_after_expiry": "accepted", "expiry_tick": "dropped_tick + 150 (both mechanisms); cleared at until <= tick"},
            "source": "D §3.7 (ItemTable.java:203-216, DropProtection.java:35,52-70, L2PcInstance.java:3798-3856)"}


def exp_herb(p: dict) -> dict:
    h = tbl("herbs")
    one = next(x for x in h["handled"] if x["id"] == 8600)
    multi = next(x for x in h["handled"] if x["id"] == 8612)
    return {"single_skill": one, "multi_skill": multi, "herb_decay_limit_ticks": s_ticks(tbl("ground")["ground"]["herb_decay_seconds"]),
            "inventory_row": False, "early_abort": "stop at the first skill whose condition fails, is disabled or in reuse; the herb is consumed regardless (E40)",
            "nodes": node_expectations(npc_row(18478), 15)}


def exp_decay(p: dict) -> dict:
    g = tbl("ground")["ground"]
    limits = {"herb": s_ticks(g["herb_decay_seconds"]), "loot": s_ticks(g["decay_seconds"]), "auto_destroy_9849": g["auto_destroy_time"][0]["seconds"] * 10}
    return {"limits_ticks": limits, "decay_due_examples": {k: {str(t): eo.decay_due(t, lim) for t in (0, 1, 49, 50, 51)} for k, lim in limits.items()},
            "grid_ticks": ms_ticks(g["scanner_ms"]), "pk_drop_decays": not g["destroy_player_dropped_item"], "rule": "strict >: despawn at the first grid tick T with T - dropped_tick > limit"}


def party_members(p: dict) -> list[dict]:
    a = next(r for r in p["roles"] if r["role"] == "a")
    out = []
    for r in p["roles"]:
        dx, dy = r["position"][0] - a["position"][0], r["position"][1] - a["position"][1]
        out.append({"name": r["role"], "dist_sq": (dx * dx + dy * dy) * 1000 * 1000, "in_range": eo.in_range_party2((dx * dx + dy * dy) * 1000 * 1000)})
    return out


def exp_party_mode(mode: str):
    def inner(p: dict) -> dict:
        members = party_members(p)
        adena_node = next(n for n in node_expectations(npc_row(20003), 10) if n.get("template") == 57)
        return {"mode": mode, "members": members, "in_range_count": sum(m["in_range"] for m in members),
                "party_range2_tiles": "1400 L2 units = 43.75 tiles (checkIfInRange(PartyRange2) on squared distance, X1/X2)",
                "adena_group": adena_node, "capacity_check": "slots only; applied only when solo or FINDERS_KEEPERS (P7)",
                "loot_distributed_event": mode != "FINDERS_KEEPERS", "by_turn": {"cursor": "pre-increment, wrap, skip out-of-range, full lap -> picker"} if "BY_TURN" in mode else None}
    return inner


def exp_adena_split(p: dict) -> dict:
    members = party_members(p)
    in_range = [m for m in members if m["in_range"]]
    k = len(in_range)
    node = next(n for n in npc_row(20003)["death"] if n["kind"] == "group" and n["items"][0]["id"] == 57)
    lo, hi = node["items"][0]["min"], node["items"][0]["max"]
    return {"recipients": [m["name"] for m in in_range], "k": k,
            "splits": {str(n): {"base_each": n // k, "remainder": n % k, "remainder_draws": n % k} for n in range(lo, hi + 1)},
            "rule": "even_split: floor(n/k) each, then n mod k draws (party_split_remainder) each +1 with replacement (X9)",
            "loot_distributed_event": False, "ledger_op": "party_split", "even_distribute_items": tbl("party")["party"]["even_distribute"]["items"]}


def exp_party_vote(p: dict) -> dict:
    pt = tbl("party")["party"]
    return {"vote_timeout_ticks": s_ticks(pt["vote_timeout_seconds"]), "flow": "RequestPartyLootModification / AnswerPartyLootModification (B15): every member answers; one decline cancels; timeout cancels; a join/leave cancels",
            "size_cap": pt["size_cap"], "phases": ["REQUESTED", "APPLIED", "CANCELLED"]}


def exp_party_xp(p: dict) -> dict:
    npc = npc_row(20003)
    X, S = eo.base_rewards(npc["level"], npc["exp_rate"], npc["sp"])
    levels = [r["level"] for r in p["roles"]]
    top = max(levels)
    return {"npc": npc["id"], "X": X, "S": S, "levels": levels, "party_distribute_all_alive": eo.party_distribute(X, S, levels, [True] * len(levels), top),
            "party_distribute_b_dead": eo.party_distribute(X, S, levels, [True, False], top),
            "bonus_table": tbl("party")["party"]["bonus_exp_sp"], "bonus_n2_bits": bits(eo.bonus(2, "RatePartyXp")),
            "cutoff": tbl("party")["party"]["xp_cutoff"], "rule": "D §4.5: level-squared shares, Math.round XP / floor SP, highfive gap percent"}


def exp_solo_xp(p: dict) -> dict:
    npc = npc_row(20003)
    X, S = eo.base_rewards(npc["level"], npc["exp_rate"], npc["sp"])
    rows = {}
    for dmg_a in (1, 7, 10):
        total = 10
        for lvl in (10, 12, 20):
            rows[f"a_dmg{dmg_a}_of{total}_level{lvl}"] = eo.calc_exp_sp(lvl - npc["level"], dmg_a, total, X, S)
    sp_only = npc_row(4301)
    return {"npc": npc["id"], "X": X, "S": S, "shares": rows, "sp_only_npc": {"id": 4301, "level": sp_only["level"], "exp_rate": sp_only["exp_rate"], "sp": sp_only["sp"], "award": [0, 0], "source": sp_only["source"]},
            "rule": "B16: per-attacker damage share X*dmg/total, pow56 penalty above diff 5, 0 XP -> 0 SP"}


def exp_spoil(p: dict) -> dict:
    npc = npc_row(20003)
    killer = next(r for r in p["roles"] if r["role"] == "a")["level"]
    return {"corpse_time_ticks": s_ticks(npc["corpse_time"]), "spoiled_extension_ticks": s_ticks(10), "corpse_until_spoiled_ticks": s_ticks(npc["corpse_time"] + 10),
            "spoil_skill": 254, "sweeper_skill": 42, "sweeper": {"hit_time_ms": 500, "reuse_ms": 500, "mp": 3, "source": "A7 pinned"},
            "corpse_nodes": node_expectations(npc, killer, scope="corpse"), "despawn": "ConsumeBody ends the corpse; EntityDespawn in the next spawn phase (D §5.3)",
            "ledger_op": "sweep", "roll_order": "corpse list first (iff spoiled), then death list (L2Attackable.java:838-842)"}


def exp_sweep_party(p: dict) -> dict:
    out = exp_spoil(p)
    out["sweep_rights"] = "spoiler or any member of the spoiler's party (L2Attackable.java:1028)"
    out["adena_split_origin"] = "NPC position (party split from the corpse origin)"
    return out


def exp_pk(p: dict) -> dict:
    pk = tbl("ground")["pk"]
    a = next(r for r in p["roles"] if r["role"] == "a")
    kit = [{"id": 0, "template": 2369, "equipped": True, "weapon": True}, {"id": 0, "template": 1146, "equipped": True, "weapon": False},
           {"id": 0, "template": 1147, "equipped": True, "weapon": False}]  # human_fighter starter kit equipped rows (tables-phase4/starter_kits.toml)
    inv_ = kit + [{"id": i["instance_id"], "template": i["template_id"], "equipped": i.get("equipped", False), "weapon": i.get("slot") == "rhand"} for i in a["inventory"]]
    excluded = [i["template"] for i in inv_ if eo.pk_excluded(dict(i))]
    return {"gate": {"karma_rate_drop": pk["karma_rate_drop"], "threshold": str(eo.chance_threshold(F(pk["karma_rate_drop"])))},
            "per_item": {"equipped_weapon": pk["karma_rate_drop_equip_weapon"], "equipped_other": pk["karma_rate_drop_equip"], "inventory": pk["karma_rate_drop_item"]},
            "limit": pk["karma_drop_limit"], "minimum_pk": pk["minimum_pk_required_to_drop"], "excluded_templates": sorted(set(excluded)),
            "eligible_count": sum(1 for i in inv_ if not eo.pk_excluded(dict(i))), "iteration": "ascending ItemInstanceId (X4)",
            "unequip_before_roll": True, "protection": "none (E24)", "position_offsets": ground_timing(False)["position_offset_player"], "decays": False}


def exp_vendor(p: dict) -> dict:
    m = p["merchant"]
    ref = m["ref_price"]
    sells = {str(c): eo.sell_price(ref, c) for c in range(1, 6)}
    refunds = {str(c): eo.refund_price(ref, c) for c in range(1, 6)}
    tab: list = []
    destroyed = []
    for i in range(13):
        tab, dead = eo.refund_tab_add(tab, m["item"], 1, False)
        destroyed += dead
    return {"merchant": m, "unit_price_castle_0": eo.unit_price(ref, m["tax"], 0), "unit_price_table_row": next(u for u in tbl("merchants")["unit_prices"] if u["item"] == m["item"] and u["tax"] == m["tax"]),
            "sell_price_by_count": sells, "refund_price_by_count": refunds, "refund_tab_after_13_sells": {"entries": len(tab), "destroyed": destroyed},
            "transaction_flood_ticks": tbl("storage_mail")["flood_intervals_ticks"]["transaction"], "ledger_ops": ["vendor_buy", "vendor_sell", "vendor_refund"]}


def exp_restock(p: dict) -> dict:
    m = p["merchant"]
    return {"merchant": m, "unit_price_castle_0": eo.unit_price(m["ref_price"], m["tax"], 0), "stock": m["count"], "restock_ticks": m["restock_min"] * 60 * 10,
            "out_of_stock_reject": "VENDOR_OUT_OF_STOCK after `count` units", "live_refill": "soak lane only unless 6.1 finds a tick fast-forward facility (E43)",
            "restock_minutes_source": "merchants.toml entries restock_min (V-R: 60 is the shortest pinned)"}


def exp_multisell(p: dict) -> dict:
    lst = next(x for x in tbl("multisell")["list"] if x["id"] == MULTISELL["list_id"])
    entry = lst["entries"][0]
    a = next(r for r in p["roles"] if r["role"] == "a")
    instances = [{"template": i["template_id"], "enchant": i["enchant"], "id": i["instance_id"]} for i in a["inventory"]]
    taken = eo.multisell_take(instances, entry["ingredients"][0]["id"], 1, lst["maintain_enchantment"], 3)
    return {"list_id": lst["id"], "npc": MULTISELL["npc"], "maintain_enchantment": lst["maintain_enchantment"], "apply_taxes": lst["apply_taxes"],
            "entry_index": 0, "entry": entry, "consumed": taken, "products_enchant_3": eo.multisell_products(entry, 1, lst["maintain_enchantment"], taken),
            "validate_amount_2_non_stackable": eo.multisell_validate(entry, 2), "multisell_flood_ticks": tbl("storage_mail")["flood_intervals_ticks"]["multisell"],
            "events": "one ItemExchanged / EconomyResult(multisell) per exchange"}


def exp_craft(p: dict) -> dict:
    r1, rf = recipe(RECIPE_DWARF), recipe(RECIPE_DWARF_FAIL)
    crafter = {"ranks": {"create_item": 2}, "mp": 1000, "items": {i["id"]: i["count"] * 2 for i in r1["ingredients"]} | {i["id"]: i["count"] for i in rf["ingredients"]}, "adena": 0}
    ok = eo.craft(r1, crafter, crafter, 0, eo.Words(1))
    empty = {"ranks": {"create_item": 2}, "mp": 1000, "items": {}, "adena": 0}
    missing = eo.craft(rf, empty, empty, 0, eo.Words(1))  # self-craft: the crafter is the customer
    return {"recipes": {"sure": r1, "fallible": rf}, "register_consumes_recipe_item": True, "limits": tbl("recipes")["config"],
            "success_thresholds": {str(r["success_rate"]): str(eo.chance_threshold(F(r["success_rate"]))) for r in (r1, rf)},
            "sure_craft_trace": ok, "missing_materials": missing["result"], "failure": "100 % material loss; MP spent (D §8.2)",
            "create_item_ranks": tbl("recipes")["create_item"]["ranks"][:2]}


def exp_craft_common(p: dict) -> dict:
    rc = common_rare_recipe()
    rarity = rc["production_rare"]["rarity"] if "production_rare" in rc else 0
    common_rare = sum(1 for r in tbl("recipes")["recipe"] if r["type"] == "common" and "production_rare" in r)
    return {"recipe": rc, "rank_table": tbl("recipes")["create_common_item"], "success_threshold": str(eo.chance_threshold(F(rc["success_rate"]))),
            "rarity": rarity, "masterwork_threshold": str(eo.chance_threshold(F(rarity))), "masterwork_draw_always_taken": True,
            "common_recipes_with_rare_production": common_rare,
            "masterwork_outcome": "masterwork == false on every common craft: no pinned common recipe carries productionRare, so the always-taken rarity-0 draw never passes" if common_rare == 0 else "per rarity",
            "outcome_carrier": "EconomyResult.craft.masterwork"}


def exp_manufacture(p: dict) -> dict:
    r1 = recipe(RECIPE_DWARF)
    crafter = {"ranks": {"create_item": 1}, "mp": 1000, "items": {}, "adena": 0}
    customer = {"items": {i["id"]: i["count"] * 2 for i in r1["ingredients"]}, "adena": 5000}
    trace = eo.craft(r1, crafter, customer, p["manufacture_fee"], eo.Words(2))
    return {"recipe": r1, "fee": p["manufacture_fee"], "order": [t[0] for t in trace["trace"]], "fee_kept_on_failure": True,
            "manufacture_flood_ticks": tbl("storage_mail")["flood_intervals_ticks"]["manufacture"], "trace": trace}


def exp_trade(p: dict) -> dict:
    return {"radius_l2_units": 150, "request_expiry_ticks": s_ticks(15), "transaction_flood_ticks": tbl("storage_mail")["flood_intervals_ticks"]["transaction"],
            "phases": ["STARTED", "ITEM_ADDED", "PARTNER_CONFIRMED", "COMPLETED", "CANCELLED"], "add_invalidates": "the adder's own confirmation only",
            "ledger": "paired transfer_out/transfer_in rows linked by one receipt (B13)", "source": "E4/E10 (TradeRequest.java:168, TradeDone.java:91)"}


def exp_trade_cancel(p: dict) -> dict:
    out = exp_trade(p)
    out["cancel_causes"] = ["partner departs the zone", "UseItem by a participant", "beyond 150 units at confirm", "partner dies"]
    out["no_partial_transfer"] = True
    return out


def exp_store(p: dict) -> dict:
    return {"limits": STORE_LIMITS, "package_sale": "bought whole or refused PACKAGE_INCOMPLETE", "closes_when_empty": True, "closes_on_departure": "B2 (OfflineTradeEnable = False)",
            "buy_store_escrow": False, "beyond_owner_adena": "entry skipped (StoreOutcome.skipped)", "kinds": ["NONE", "SELL", "PACKAGE_SELL", "BUY", "MANUFACTURE"]}


def exp_warehouse(p: dict) -> dict:
    sm = tbl("storage_mail")["warehouse"]
    return {"fee_per_entry": sm["deposit_fee_per_entry"], "fees": {str(n): sm["deposit_fee_per_entry"] * n for n in (1, 2, 3)}, "slots": {"dwarf": sm["slots_dwarf"], "other": sm["slots_no_dwarf"]},
            "merge": "stackables merge into the existing warehouse stack", "augmented_allowed": True, "busy_reject": "CONTAINER_BUSY while the container lease is held (retryable)",
            "ledger_ops": ["deposit", "withdraw"], "owner_kind": "warehouse"}


def exp_freight(p: dict) -> dict:
    fr = tbl("storage_mail")["freight"]
    return {"fee_per_entry": fr["fee_per_entry"], "fees": {str(n): fr["fee_per_entry"] * n for n in (1, 2)}, "slots": fr["slots"], "same_account": True,
            "templates": {"20678": eo.freightable({"template": 20678}), "20725": eo.freightable({"template": 20725}), "57": eo.freightable({"template": 57})},
            "adena_refused": "ITEM_NOT_FREIGHTABLE (E56)", "ledger_ops": ["deposit", "withdraw"], "owner_kind": "freight"}


def exp_mail_cod(p: dict) -> dict:
    ml = tbl("storage_mail")["mail"]
    return {"fee": {str(n): ml["message_fee"] + ml["message_fee_per_slot"] * n for n in (0, 1, 3, 8)}, "max_attachments": ml["max_attachments"], "inbox_limit": ml["inbox_size"],
            "cod_expiration_hours": ml["cod_expiration_hours"], "expiration_hours": ml["expiration_hours"], "send_mail_flood_ticks": tbl("storage_mail")["flood_intervals_ticks"]["send_mail"],
            "peace_zone_required": "iff attachments", "cod_credit": "pending delivery to the sender (one per target per tick, E41)", "notification": "MailNotification (WorldEvent 42) from the API gateway after commit"}


def exp_mail_lifecycle(p: dict) -> dict:
    out = exp_mail_cod(p)
    out["reject"] = "attachments return to the sender as a new system message"
    out["cancel"] = "sender reclaims untaken attachments"
    out["expiry"] = "MailExpire (system kind 76) returns attachments as a warehouse delivery"
    return out


def exp_auction(p: dict) -> dict:
    a = eo.auction_new()
    init = AUCTION["init_bid"]
    steps = []
    for who, amount in ((1, init), (2, init), (1, init + 50000), (2, init + 50000)):
        res, pay = eo.auction_bid(a, who, amount, init, 200)
        steps.append({"bidder": who, "amount": amount, "result": res, "payment": pay, "highest": a["highest"]})
    cancel = eo.auction_cancel(a, 2)
    return {"auction": AUCTION, "config": tbl("auction")["config"], "bids": steps, "cancel_outbid_2": {"result": cancel[0], "refund": cancel[1]},
            "award": "winner's item is delivered to the WAREHOUSE (E16)", "item_auction_flood_ticks": tbl("storage_mail")["flood_intervals_ticks"]["item_auction"],
            "server_bypass_ticks": tbl("storage_mail")["flood_intervals_ticks"]["server_bypass"], "fixture_anchor": tbl("auction")["fixture_schedule"]["anchor"]}


def exp_manor(p: dict) -> dict:
    mn = tbl("manor")
    fx = mn["fixture_gludio"]
    seed = next(s for s in fx["seed_production"] if s["seed_id"] == 5016)
    crop = next(c for c in fx["crop_procurement"] if c["crop_id"] == 5073)
    row = next(c for c in mn["crops"] if c["crop"] == 5073)
    npc = npc_row(20003)
    a = next(r for r in p["roles"] if r["role"] == "a")
    P = crop["price"] * 3
    return {"castle": {"id": fx["castle_id"], "castle_area": fx["castle_area"]}, "seed": seed, "crop": crop, "crop_row": row,
            "sow_basic": eo.sow_basic(row["level"], row["alternative"], a["level"], npc["level"]), "sow_threshold": str(eo.chance_threshold(F(max(eo.sow_basic(row["level"], row["alternative"], a["level"], npc["level"]), 0)))),
            "harvest_basic": eo.harvest_basic(a["level"], npc["level"]), "harvester_items": mn["skills"]["harvester_items"], "skills": {"sow": 2097, "harvest": 2098},
            "crop_sale_3": {"reward_template": row["rew1"], "P": P, "fee": int(eo.fl(eo.fl(F(P)) * eo.fl(F("0.05")))), "fee_equals_P_div_20": int(eo.fl(eo.fl(F(P)) * eo.fl(F("0.05")))) == P // 20},
            "manor_flood_ticks": tbl("storage_mail")["flood_intervals_ticks"]["manor"], "normal_drops_unchanged": True}


def exp_epoch(p: dict) -> dict:
    return {"close": "graceful close destroys remaining ground items with ledger cause epoch_close:<epoch>", "abandoned": "EpochReconcile (system kind 75) destroys with epoch_void:<epoch> before admission",
            "events": "none (ledger only)", "container_rows": "lease releases and delivery changes in EconomyCheckpoint (PbOutput 55)"}


SCENARIOS: dict[str, tuple[str, object]] = {
    "5-drop-basic": ("phase5-drops", exp_drop_basic), "5-drop-precise-over-100": ("phase5-drops-precise", exp_drop_precise),
    "5-drop-level-gap": ("phase5-drops-gap", exp_drop_gap), "5-drop-rate-multiplier": ("phase5-rates", exp_rates),
    "5-pickup-protection": ("phase5-protection", exp_protection), "5-herb": ("phase5-herbs", exp_herb), "5-decay": ("phase5-decay", exp_decay),
    "5-party-loot-finders": ("phase5-party", exp_party_mode("FINDERS_KEEPERS")), "5-party-loot-random": ("phase5-party", exp_party_mode("RANDOM")),
    "5-party-loot-random-spoil": ("phase5-party-spoil", exp_party_mode("RANDOM_INCLUDING_SPOIL")), "5-party-loot-by-turn": ("phase5-party", exp_party_mode("BY_TURN")),
    "5-party-loot-by-turn-spoil": ("phase5-party-spoil", exp_party_mode("BY_TURN_INCLUDING_SPOIL")), "5-party-adena-split": ("phase5-party", exp_adena_split),
    "5-party-vote": ("phase5-party", exp_party_vote), "5-party-xp": ("phase5-party-xp", exp_party_xp), "5-solo-xp-damage-share": ("phase5-solo-xp", exp_solo_xp),
    "5-spoil-sweep": ("phase5-spoil", exp_spoil), "5-sweep-party": ("phase5-party-spoil", exp_sweep_party), "5-pk-drop": ("phase5-pk-drop", exp_pk),
    "5-vendor-buy-sell-refund": ("phase5-vendors", exp_vendor), "5-vendor-restock": ("phase5-vendor-restock", exp_restock), "5-multisell": ("phase5-multisell", exp_multisell),
    "5-craft-dwarf": ("phase5-craft", exp_craft), "5-craft-common-masterwork": ("phase5-craft-common", exp_craft_common), "5-private-manufacture": ("phase5-manufacture", exp_manufacture),
    "5-trade": ("phase5-trade", exp_trade), "5-trade-cancel-disconnect": ("phase5-trade", exp_trade_cancel), "5-store-sell-package": ("phase5-stores", exp_store),
    "5-store-buy": ("phase5-stores", exp_store), "5-warehouse": ("phase5-warehouse", exp_warehouse), "5-freight": ("phase5-freight", exp_freight),
    "5-mail-cod": ("phase5-mail", exp_mail_cod), "5-mail-reject-cancel-expiry": ("phase5-mail", exp_mail_lifecycle), "5-auction-bid-cancel-award": ("phase5-auction", exp_auction),
    "5-manor-sow-harvest-sell": ("phase5-manor", exp_manor), "5-epoch-reconcile": ("phase5-epoch", exp_epoch),
}


def canonical(obj) -> str:
    return json.dumps(obj, indent=1, sort_keys=True, default=str) + "\n"


def generate() -> dict[Path, str]:
    packs = build_packs()
    repos = tbl("sources")["repositories"]
    files: dict[Path, str] = {}
    packs_doc = {"schema_version": 1, "story": "5 E1.8", "policy": "pre_admission_only", "inputs_are_oracle_relevant_only": True,
                 "pinned": {"game": repos["game"]["revision"], "datapack": repos["datapack"]["revision"]}, "packs": packs}
    files[PACKS_OUT] = canonical(packs_doc)
    for name, (fixture, fn) in SCENARIOS.items():
        if fixture not in packs:
            raise eo.OracleError(f"{name}: unknown pack {fixture}")
        exp = fn(packs[fixture])
        body = {"schema_version": 1, "scenario": name + ".nfs", "fixture": fixture, "story": "5 E1.8",
                "derived_by": "apps/client-unreal/Scripts/phase5-expectations.py over economy_oracle.py (1.5-1.7) and tables-phase5",
                "expectations": exp}
        body["expectations_sha256"] = hashlib.sha256(canonical(exp).encode()).hexdigest()
        files[SCENARIO_DIR / f"{name}.expect.json"] = canonical(body)
    return files


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="fail if a committed file differs")
    args = parser.parse_args()
    try:
        files = generate()
    except eo.OracleError as err:
        print(f"phase5-expectations: FAILED: {err}", file=sys.stderr)
        return 1
    stale = [str(p.relative_to(REPO)) for p, content in files.items() if not p.exists() or p.read_text(encoding="utf-8") != content]
    if args.check:
        if stale:
            print("phase5-expectations: stale files: " + ", ".join(stale), file=sys.stderr)
            return 1
        print(f"phase5-expectations: {len(files)} files verified ({len(SCENARIOS)} sidecars, {len(build_packs())} packs).")
        return 0
    for path, content in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    print(f"phase5-expectations: wrote {len(files)} files ({len(stale)} changed).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
