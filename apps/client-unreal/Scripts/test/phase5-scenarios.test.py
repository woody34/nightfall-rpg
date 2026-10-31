#!/usr/bin/env python3
"""Story 1.8 structure checks for the 36 Phase 5 scenario files (plan §2.8, §5.3; design §14.3).

Static only (no live run, 6.6 owns that): every file carries its plan §2.8 header verbatim and a
§5.3 pack; every command/predicate comes from the closed §5.3 + registered Phase 3/4 vocabulary;
every `# econ-gate:` label is a design §14.3 formula, chance callsite, select callsite or RNG
callsite; the budget equals the largest role's sum of waits + 30 s; role blocks name pack roles;
the oracle sidecars and pack declarations are present and `phase5-expectations.py --check` clean.
"""
import json
from pathlib import Path
import re
import subprocess
import sys
import unittest

CLIENT = Path(__file__).resolve().parents[2]
REPO = CLIENT.parents[1]
SCENARIOS = CLIENT / "Scenarios"
PLAN = REPO / "docs/plans/phase-5-economy-and-crafting.md"
DESIGN = REPO / "docs/plans/phase-5-zone-design.md"
PACKS = json.loads((SCENARIOS / "fixtures/phase5-packs.json").read_text())["packs"]

COMMANDS = {
    # registered Phase 0b-4 commands used here
    "Login", "EnterWorld", "WaitFor", "Expect", "Within", "ClickMove", "Target", "Attack", "StopAttack", "Cast", "UseItem", "DropSocket", "MarkItemState",
    # plan §5.3 Phase 5 commands (argument authority: story 6.1's parser)
    "PickUp", "Buy", "Sell", "Refund", "Multisell", "RegisterRecipe", "DeleteRecipe", "Craft", "ManufactureOrder",
    "TradeRequest", "TradeAnswer", "TradeAdd", "TradeConfirm", "TradeCancel",
    "StoreSell", "StoreBuy", "StoreManufacture", "StoreClose", "StoreBuyFrom", "StoreSellTo",
    "PartyInvite", "PartyAnswer", "PartyLeave", "PartyKick", "PartyLeader", "PartyLootVote", "PartyLootAnswer",
    "Deposit", "Withdraw", "FreightSend", "FreightWithdraw",
    "MailSend", "MailTake", "MailReject", "MailCancel", "MailDelete", "MailRead",
    "AuctionBid", "AuctionCancel", "ManorBuySeed", "ManorSellCrop",
}
PREDICATES = {
    # registered (Phase 0b-4)
    "connected", "in_world", "item_baseline", "target", "entity_dead", "own_dead", "inventory_has", "inventory_revision_since_mark", "cast_results",
    "buff_active", "rejected", "entity_in_view", "xp_gained_events",
    # plan §5.3 Phase 5 predicates
    "ground_item_exists", "recipe_registered", "craft_result", "trade_state", "store_state", "party_member_count", "loot_mode", "mail_received", "auction_state", "manor_result",
    # received-state refinements of the §5.3 carriers (EntitySpawn kind 3 / EconomyResult / PartyStateChanged / TradeSessionChanged / PrivateStoreChanged /
    # ItemAuctionStatusChanged / CorpseStateChanged / LootDistributed / PartyLootVote / RecipeBookSync); 6.1 binds each to its carrier field
    "ground_item_kind", "ground_loot_owner", "ground_drop_protection", "ground_drop_protection_none", "ground_protection_expired", "ground_item_count_between",
    "ground_item_gone", "ground_despawn_on_grid", "ground_item_any", "ground_item_count", "economy_result", "economy_result_count", "economy_export_marker",
    "own_level_between", "rules_rate_multiplier", "herb_skill_early_abort", "pk_drop_decays", "party_invite_received", "loot_recipient_in_party", "loot_recipient",
    "by_turn_cursor_advanced", "loot_distributed", "loot_distributed_mode", "loot_distributed_count", "loot_distributed_spoil", "own_out_of_party_range",
    "corpse_spoiled", "corpse_sweepable", "corpse_swept", "corpse_until_delta", "sweep_recipient_in_party", "party_sweep_done", "adena_split_share_between",
    "adena_split_origin", "loot_vote_phase", "xp_gained_last", "sp_gained_last", "xp_gained_last_between", "xp_share_matches_export", "sp_gained_events_for",
    "entity_gone", "own_karma", "own_pk_count", "pk_drop_count_between", "pk_dropped_excluded", "pk_unequipped_before_roll", "merchant_known", "adena_delta",
    "refund_tab_count", "refund_tab_oldest_destroyed", "vendor_stock", "vendor_restock_due_delta", "exchanger_known", "enchant_of", "craft_result_any",
    "craft_materials_consumed", "craft_masterwork", "craft_fee_before_roll", "store_state_of", "manufacture_gate_ticks", "own_trade_confirmed", "trade_transfer_pairs",
    "trade_request_received", "trade_cancel_reason", "store_entries_skipped", "warehouse_keeper_known", "container_fee", "warehouse_has", "container_busy_retry_observed",
    "freight_capacity", "freight_has", "cod_credit_received", "mail_expiry_hours", "mail_gone", "mail_has_attachments", "auctioneer_known", "auction_highest_bid",
    "auction_award_to_warehouse", "auction_leader", "manor_manager_known", "corpse_seeded_or_sow_failed", "harvest_done", "manor_drops_unchanged", "manor_fee_equals_p_div_20",
    "epoch_reconcile_rows_before_admission",
}
GATE_LABELS = {
    # design §14.3 econ_formula
    "drop_chance", "vendor_unit_price", "sell_price", "refund_price", "party_xp", "solo_xp", "even_split", "warehouse_fee", "freight_fee", "mail_fee", "crop_reward", "crop_fee",
    # econ_chance callsites
    "corpse_item", "death_item", "death_remainder", "group_chance", "group_remainder", "pk_gate", "pk_item", "craft_success", "craft_rare", "sow", "harvest",
    # econ_select / item_rng callsites
    "group_select", "drop_quantity", "npc_drop_dx", "npc_drop_dy", "pk_drop_dx", "pk_drop_dy", "party_random", "party_split_remainder", "auction_item",
}


def plan_rows() -> dict[str, tuple[str, str]]:
    rows = {}
    for line in PLAN.read_text().splitlines():
        m = re.match(r"\| `(5-[a-z0-9-]+)\.nfs` \| `(# covers: [^`]+)` \| (phase5-[a-z0-9-]+) \|", line)
        if m:
            rows[m[1]] = (m[2], m[3])
    return rows


def plan_packs() -> set[str]:
    text = PLAN.read_text()
    start = text.index("### 5.3 Fixture packs")
    block = text[start:text.index("**Commands and predicates.**", start)]
    return set(re.findall(r"`(phase5-[a-z0-9-]+)`", block))


def design_gate_labels() -> set[str]:
    text = DESIGN.read_text()
    section = text[text.index("### 14.3 Evidence contract v2"):text.index("### 14.4")]
    return set(re.findall(r"`([a-z_]+)`", section)) | set(re.findall(r"callsite: ([a-z_]+)", section))


class ScenarioFileTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rows = plan_rows()
        cls.files = {p.stem: p.read_text().splitlines() for p in sorted(SCENARIOS.glob("5-*.nfs"))}

    def test_exactly_the_36_plan_files_with_verbatim_headers(self):
        self.assertEqual(len(self.rows), 36)
        self.assertEqual(set(self.files), set(self.rows))
        for name, lines in self.files.items():
            covers, fixture = self.rows[name]
            self.assertEqual(lines[0], covers, name)
            self.assertEqual(lines[1], f"# fixture: {fixture}", name)
            self.assertIn("5 E1.8", covers)

    def test_fixtures_are_plan_5_3_packs_with_declarations(self):
        packs = plan_packs()
        self.assertEqual(len(packs), 27)
        self.assertEqual(set(PACKS), packs)
        for name, (_, fixture) in self.rows.items():
            self.assertIn(fixture, PACKS, name)
            sidecar = json.loads((SCENARIOS / f"{name}.expect.json").read_text())
            self.assertEqual(sidecar["fixture"], fixture)
            self.assertEqual(sidecar["scenario"], name + ".nfs")
            self.assertTrue(sidecar["expectations"], name)

    def test_vocabulary_is_closed(self):
        for name, lines in self.files.items():
            for ln in lines:
                if not ln.startswith("nf."):
                    continue
                parts = ln.split()
                cmd = parts[0][3:]
                self.assertIn(cmd, COMMANDS, f"{name}: {ln}")
                if cmd in ("WaitFor", "Expect"):
                    self.assertIn(parts[1], PREDICATES, f"{name}: {ln}")
                    if cmd == "WaitFor":
                        self.assertRegex(parts[-1], r"^\d+$", f"{name}: WaitFor needs a timeout: {ln}")

    def test_gate_labels_are_design_14_3_names(self):
        allowed = GATE_LABELS & design_gate_labels()
        self.assertEqual(allowed, GATE_LABELS, GATE_LABELS - design_gate_labels())
        for name, lines in self.files.items():
            seen = []
            for ln in lines:
                if ln.startswith("# econ-gate:"):
                    m = re.fullmatch(r"# econ-gate: ([a-z_]+)", ln)
                    self.assertIsNotNone(m, f"{name}: {ln}")
                    self.assertIn(m[1], GATE_LABELS, f"{name}: {ln}")
                    self.assertNotIn(m[1], seen, f"{name}: duplicate gate {m[1]}")
                    seen.append(m[1])

    def test_budget_is_largest_role_waits_plus_grace(self):
        for name, lines in self.files.items():
            roles: dict[str, int] = {}
            current = "a"
            for ln in lines:
                m = re.fullmatch(r"# role: ([abc])", ln)
                if m:
                    current = m[1]
                    roles.setdefault(current, 0)
                    continue
                m = re.match(r"nf\.WaitFor .* (\d+)$", ln)
                if m:
                    roles[current] = roles.get(current, 0) + int(m[1])
            within = [int(ln.split()[1]) for ln in lines if ln.startswith("nf.Within ")]
            self.assertEqual(len(within), 1, name)
            self.assertEqual(within[0], max(roles.values()) + 30, name)
            self.assertEqual(lines[-1], f"nf.Within {within[0]}", name)

    def test_role_blocks_name_pack_roles_and_every_role_asserts(self):
        for name, lines in self.files.items():
            fixture = self.rows[name][1]
            pack_roles = {r["role"] for r in PACKS[fixture]["roles"]}
            blocks = [ln.split()[-1] for ln in lines if ln.startswith("# role:")]
            self.assertEqual(len(blocks), len(set(blocks)), name)
            self.assertTrue(set(blocks) <= pack_roles, f"{name}: {set(blocks) - pack_roles}")
            # every role (or the implicit role a) logs in and makes at least one assertion
            body = "\n".join(lines)
            sections = re.split(r"^# role: [abc]\n", body + "\n", flags=re.M)[1:] if blocks else [body]
            for section in sections:
                self.assertIn("nf.Login", section, name)
                self.assertRegex(section, r"nf\.(Expect|WaitFor) ", name)
            if not blocks:
                self.assertIn("a", pack_roles, name)

    def test_expectations_generator_is_clean(self):
        result = subprocess.run([sys.executable, str(CLIENT / "Scripts/phase5-expectations.py"), "--check"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("36 sidecars, 27 packs", result.stdout)


if __name__ == "__main__":
    unittest.main()
