#!/usr/bin/env python3
"""Story 6.1: the Phase 5 DSL contract is parser-tested, every §5.3 command/predicate names its
received carrier and reset rule, every line of the 36 plan §2.8 files parses, and role blocks
split into per-role scenario copies that keep the shared header and budget."""
import importlib.util
from pathlib import Path
import shlex
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('grammar', SCRIPTS / 'phase5-grammar.py')
grammar = importlib.util.module_from_spec(spec)
spec.loader.exec_module(grammar)
spec4 = importlib.util.spec_from_file_location('grammar4', SCRIPTS / 'phase4-grammar.py')
grammar4 = importlib.util.module_from_spec(spec4)
spec4.loader.exec_module(grammar4)
# Names the client registers at codex/phase4-client 76ac549 (committed snapshot; see the fixture header).
REGISTRY = {line.strip() for line in (SCRIPTS / 'test/fixtures/bot-registry-76ac549.txt').read_text().splitlines() if line.strip() and not line.startswith('#')}
UUID_RE = r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
NO_ARG_COMMANDS = {'nf.Login', 'nf.EnterWorld', 'nf.Attack', 'nf.StopAttack', 'nf.DropSocket', 'nf.MarkItemState'}
NO_ARG_PREDICATES = {'connected', 'in_world', 'item_baseline', 'own_dead', 'blacksmith_known'}


def inherited_command_ok(tokens):
    """Registered Phase 0b-4 command forms (BotSteps.cpp, phase4-grammar.py) for the names the 36 files use."""
    import re
    name, args = tokens[0], tokens[1:]
    if name not in REGISTRY:
        return False
    verdict = grammar4.command(tokens)
    if verdict is not None:
        return bool(verdict)
    if name in NO_ARG_COMMANDS:
        return not args
    if name == 'nf.Target':  # nf.Target <entity id|nearest_attackable|attacker|last|template:<id>|none> (BotSteps.cpp:403)
        return len(args) == 1 and (re.fullmatch(UUID_RE, args[0]) or args[0] in ('nearest_attackable', 'attacker', 'last', 'none') or re.fullmatch(r'template:npc_[0-9]+', args[0]))
    if name == 'nf.Cast':  # nf.Cast <id> [self|target|uuid] (SkillBotPredicates.cpp:180)
        return len(args) in (1, 2) and args[0].isdigit() and (len(args) == 1 or args[1] in ('self', 'target') or re.fullmatch(UUID_RE, args[1]))
    if name == 'nf.ClickMove':
        return len(args) == 2 and all(re.fullmatch(r'[0-9]+(\.[0-9]+)?', a) for a in args)
    return False


def inherited_predicate_ok(args):
    """Registered predicate forms for the inherited names the 36 files use (BotPredicates.cpp:842 target ==,
    SkillBotPredicates.cpp:24 EntityArg own|self|target|<uuid>, :120 buff_active <skill> [entity], :155 counters)."""
    import re
    name, rest = args[0], args[1:]
    if name not in REGISTRY:
        return False
    verdict = grammar4.predicate(args)
    if verdict is not None:
        return bool(verdict)
    if name in NO_ARG_PREDICATES:
        return not rest
    if name == 'target':
        # <name|id|none>: a Spawn.Name (never a template:/entity: token, which the bot would compare literally)
        return len(rest) == 2 and rest[0] == '==' and (rest[1] == 'none' or re.fullmatch(UUID_RE, rest[1]) or (bool(rest[1]) and not rest[1].startswith(('template:', 'entity:'))))
    if name in ('entity_dead', 'entity_in_view', 'entity_alive'):
        return len(rest) == 1 and (rest[0] in ('own', 'self', 'target') or re.fullmatch(UUID_RE, rest[0]))
    if name in ('buff_active', 'buff_inactive', 'debuff_active'):
        return len(rest) in (1, 2) and rest[0].isdigit() and (len(rest) == 1 or rest[1] in ('own', 'self', 'target') or re.fullmatch(UUID_RE, rest[1]))
    if name in ('cast_results', 'cast_starts', 'sp_gained_events', 'own_karma', 'own_pk_count', 'own_hp', 'own_mp', 'own_sp'):  # Number predicates
        return len(rest) == 2 and rest[0] in grammar.COMPARISONS and rest[1].isdigit()
    return False


class Grammar(unittest.TestCase):
    def test_commands_valid_malformed_and_foreign(self):
        key = ' 01970000-0000-7000-8000-000000000050'
        for source in ('nf.PickUp template:57', 'nf.PickUp nearest', 'nf.Buy merchant 314130 1294 2', 'nf.Sell merchant 314130 template:1294 3',
                       'nf.Refund merchant 314130 0', 'nf.Multisell exchanger 616 0 1 enchant:3', 'nf.Multisell exchanger 616 0 5000',
                       'nf.RegisterRecipe template:1666', 'nf.DeleteRecipe 1', 'nf.Craft 322', 'nf.ManufactureOrder a 1', 'nf.TradeRequest b',
                       'nf.TradeAnswer accept', 'nf.TradeAdd template:1864 5', 'nf.TradeAdd template:1869 0', 'nf.TradeConfirm', 'nf.TradeCancel',
                       'nf.StoreSell shop package template:1864:2:10,template:1869:2:10', 'nf.StoreSell shop template:1864:1:5',
                       'nf.StoreBuy shop template:1870:0:5:300', 'nf.StoreManufacture shop 1:1000', 'nf.StoreClose',
                       'nf.StoreBuyFrom a template:1864:2:10', 'nf.StoreSellTo a template:1870:5:300', 'nf.PartyInvite b FINDERS_KEEPERS',
                       'nf.PartyInvite c', 'nf.PartyAnswer 1', 'nf.PartyAnswer -1', 'nf.PartyLeave', 'nf.PartyKick b', 'nf.PartyLeader b',
                       'nf.PartyLootVote RANDOM', 'nf.PartyLootAnswer decline', 'nf.Deposit template:1864:5,template:1:1', 'nf.Withdraw 1864:10',
                       'nf.FreightSend b template:20678:1', 'nf.FreightWithdraw 20678:1', 'nf.MailSend b paycheck cod 500 template:1864:3',
                       'nf.MailSend b hello', 'nf.MailSend b expiry template:1864:1', 'nf.MailTake latest', 'nf.MailReject latest',
                       'nf.MailCancel latest', 'nf.MailDelete latest sent', 'nf.MailRead latest', 'nf.AuctionBid 32320 100000',
                       'nf.AuctionCancel 32320', 'nf.ManorBuySeed manor_manager 1 5016 3', 'nf.ManorSellCrop manor_manager template:5073 5073 1 all'):
            self.assertTrue(grammar.command(shlex.split(source)), source)
            if not source.endswith(('Confirm', 'Cancel', 'Close', 'Leave')):
                self.assertTrue(grammar.command(shlex.split(source + key)), source + key)
        for source in ('nf.PickUp 57', 'nf.PickUp', 'nf.Buy merchant 314130 1294 0', 'nf.Buy vendor 314130 1294 1', 'nf.Refund merchant 314130 12',
                       'nf.Multisell exchanger 616 0 5001', 'nf.Multisell exchanger 616 0 1 enchant:x', 'nf.Craft 0', 'nf.ManufactureOrder d 1',
                       'nf.TradeRequest self', 'nf.TradeAnswer yes', 'nf.TradeConfirm now', 'nf.StoreSell shop parcel template:1:1:1',
                       'nf.StoreSell shop template:1864:2', 'nf.StoreBuy shop template:1870:5:300', 'nf.StoreManufacture shop 1', 'nf.PartyInvite b LOOT',
                       'nf.PartyAnswer 2', 'nf.PartyLootVote RANDOM FINDERS_KEEPERS', 'nf.Deposit 1864:5', 'nf.Withdraw template:1864',
                       'nf.FreightSend template:20678:1', 'nf.MailSend b', 'nf.MailSend b hello cod', 'nf.MailSend b cod 1 extra junk', 'nf.MailTake first',
                       'nf.MailDelete latest inbox', 'nf.AuctionBid 32320 100000000001', 'nf.AuctionCancel', 'nf.ManorBuySeed manor_manager 10 5016 3',
                       'nf.ManorSellCrop manor_manager template:5073 5073 1 some'):
            self.assertFalse(grammar.command(shlex.split(source)), source)
        for source in ('nf.Equip 72 rhand', 'nf.Login', 'nf.Invented'):
            self.assertIsNone(grammar.command(shlex.split(source)), source)

    def test_predicates_valid_malformed_and_foreign(self):
        for source in ('ground_item_exists 57', 'ground_item_kind 57 == 3', 'ground_loot_owner 57 == self', 'ground_loot_owner 57 != self',
                       'ground_drop_protection 57 == self', 'ground_drop_protection_none', 'ground_protection_expired 9599', 'ground_item_count_between 8600 3 10',
                       'ground_item_gone 8600', 'ground_despawn_on_grid 9849', 'ground_item_any', 'ground_item_count == 0', 'economy_result == pick_up_item',
                       'economy_result_count multisell_choose == 1', 'economy_export_marker', 'own_level_between 11 15',
                       'rules_rate_multiplier == 1.5', 'herb_skill_early_abort 8612', 'pk_drop_decays == false', 'party_member_count == 2', 'loot_mode == BY_TURN',
                       'party_invite_received', 'loot_recipient_in_party', 'loot_recipient != self', 'by_turn_cursor_advanced', 'loot_distributed',
                       'loot_distributed_mode == RANDOM', 'loot_distributed_count == 0', 'loot_distributed_spoil == true', 'own_out_of_party_range',
                       'corpse_spoiled template:npc_20003', 'corpse_sweepable template:npc_20003', 'corpse_swept template:npc_20003', 'corpse_until_delta template:npc_20003 == 170',
                       'sweep_recipient_in_party', 'party_sweep_done template:npc_20003', 'adena_split_share_between 15 21', 'adena_split_origin == npc', 'loot_vote_phase == APPLIED',
                       'xp_gained_last == 80', 'sp_gained_last == 2', 'xp_gained_last_between 1 178', 'xp_share_matches_export', 'sp_gained_events_for template:npc_4301 == 0',
                       'entity_gone template:npc_20003', 'own_pk_count >= 6', 'pk_drop_count_between 0 10', 'pk_dropped_excluded == 0',
                       'pk_unequipped_before_roll', 'merchant_known', 'adena_delta == -172', 'adena_delta == 40', 'refund_tab_count == 12',
                       'refund_tab_oldest_destroyed', 'vendor_stock 353630 16127 == 10', 'vendor_restock_due_delta 353630 16127 == 36000', 'exchanger_known',
                       'enchant_of 9416 == 3', 'craft_result_any', 'craft_result == success', 'craft_materials_consumed 322', 'craft_masterwork == false',
                       'craft_fee_before_roll', 'store_state == PACKAGE_SELL', 'store_state_of a == BUY', 'manufacture_gate_ticks == 3', 'own_trade_confirmed',
                       'trade_state == COMPLETED', 'trade_transfer_pairs == 2', 'trade_request_received', 'trade_cancel_reason == departure',
                       'store_entries_skipped == 1', 'warehouse_keeper_known', 'container_fee == 60', 'warehouse_has 1864 == 10', 'container_busy_retry_observed',
                       'freight_capacity == 200', 'freight_has 20678 == 1', 'cod_credit_received 500', 'mail_received cod', 'mail_expiry_hours == 360',
                       'mail_gone cancel', 'mail_has_attachments expiry', 'auctioneer_known', 'auction_state == FINISHED', 'auction_highest_bid == 150000',
                       'auction_award_to_warehouse 9901', 'auction_leader != self', 'manor_manager_known', 'corpse_seeded_or_sow_failed template:npc_20003',
                       'harvest_done template:npc_20003', 'manor_drops_unchanged', 'manor_fee_equals_p_div_20', 'manor_result granted 1864 >= 1',
                       'epoch_reconcile_rows_before_admission', 'recipe_registered 1', 'rejected == GROUND_ITEM_PROTECTED', 'rejected == RATE_LIMITED'):
            self.assertTrue(grammar.predicate(shlex.split(source)), source)
        for source in ('ground_item_exists', 'ground_item_kind 57 == 2', 'ground_loot_owner 57 == b', 'ground_item_count_between 8600 10 3',
                       'economy_result == equip', 'economy_result pick_up_item', 'rules_rate_multiplier == x', 'loot_mode == LOOT', 'loot_vote_phase == DONE',
                       'corpse_spoiled 20003', 'corpse_swept template:20003', 'adena_delta == +5', 'adena_delta >= 1', 'vendor_stock 353630 16127 == -1', 'craft_result == maybe',
                       'store_state_of d == BUY', 'trade_state == OPEN', 'trade_cancel_reason == boredom', 'rejected == NOPE', 'pk_dropped_excluded == 1',
                       'manor_result granted 1864 == 1', 'mail_received', 'auction_state == STARTED FINISHED', 'ground_drop_protection_none now'):
            self.assertFalse(grammar.predicate(shlex.split(source)), source)
        for source in ('inventory_has 72 >= 1', 'connected', 'invented_predicate'):
            self.assertIsNone(grammar.predicate(shlex.split(source)), source)

    def test_every_command_and_predicate_names_a_carrier_and_reset(self):
        for name, (form, carrier) in grammar.COMMANDS.items():
            self.assertTrue(name.startswith('nf.') and carrier, name)
        for name, (form, carrier, reset) in grammar.PREDICATES.items():
            self.assertTrue(carrier and reset, name)
        self.assertEqual(len(grammar.COMMANDS), 41)
        # §5.3 names all present
        for name in ('nf.PickUp', 'nf.Buy', 'nf.Sell', 'nf.Refund', 'nf.Multisell', 'nf.RegisterRecipe', 'nf.DeleteRecipe', 'nf.Craft', 'nf.ManufactureOrder',
                     'nf.TradeRequest', 'nf.TradeAnswer', 'nf.TradeAdd', 'nf.TradeConfirm', 'nf.TradeCancel', 'nf.StoreSell', 'nf.StoreBuy', 'nf.StoreManufacture',
                     'nf.StoreClose', 'nf.StoreBuyFrom', 'nf.StoreSellTo', 'nf.PartyInvite', 'nf.PartyAnswer', 'nf.PartyLeave', 'nf.PartyKick', 'nf.PartyLeader',
                     'nf.PartyLootVote', 'nf.PartyLootAnswer', 'nf.Deposit', 'nf.Withdraw', 'nf.FreightSend', 'nf.FreightWithdraw', 'nf.MailSend', 'nf.MailTake',
                     'nf.MailReject', 'nf.MailCancel', 'nf.MailDelete', 'nf.MailRead', 'nf.AuctionBid', 'nf.AuctionCancel', 'nf.ManorBuySeed', 'nf.ManorSellCrop'):
            self.assertIn(name, grammar.COMMANDS)
        for name in ('ground_item_exists', 'recipe_registered', 'craft_result', 'trade_state', 'store_state', 'party_member_count', 'loot_mode',
                     'mail_received', 'auction_state', 'manor_result'):
            self.assertIn(name, grammar.PREDICATES)

    def test_every_line_of_the_36_files_parses_or_is_a_registered_inherited_form(self):
        files = sorted((SCRIPTS.parent / 'Scenarios').glob('5-*.nfs'))
        self.assertEqual(len(files), 36)
        self.assertGreater(len(REGISTRY), 600)
        for path in files:
            for line in path.read_text().splitlines():
                if not line.startswith('nf.'):
                    continue
                self.assertNotIn('"', line, f'{path.name}: the bot tokenizes on whitespace only (BotPredicates.cpp:566-571): {line}')
                tokens = line.split()  # ParseIntoArrayWS: whitespace, no quote handling
                if tokens[0] in ('nf.WaitFor', 'nf.Expect'):
                    args = tokens[1:]
                    if tokens[0] == 'nf.WaitFor':
                        self.assertRegex(args[-1], r'^[0-9]+$', f'{path.name}: WaitFor needs a timeout: {line}')
                        args = args[:-1]
                    verdict = grammar.predicate(args)
                    if verdict is None:
                        self.assertTrue(inherited_predicate_ok(args), f'{path.name}: unregistered inherited predicate form: {line}')
                    else:
                        self.assertTrue(verdict, f'{path.name}: {line}')
                elif tokens[0] == 'nf.Within':
                    self.assertEqual(len(tokens), 2)
                else:
                    verdict = grammar.command(tokens)
                    if verdict is None:
                        self.assertTrue(inherited_command_ok(tokens), f'{path.name}: unregistered inherited command form: {line}')
                    else:
                        self.assertTrue(verdict, f'{path.name}: {line}')
        # Phase 5 names never shadow a registered name, except the ones the lane re-declares on purpose.
        # `rejected` is the registered Phase 1 predicate; Phase 5 re-declares it only to extend the reason set.
        self.assertEqual(set(grammar.PREDICATES) & REGISTRY, {'rejected'})
        self.assertEqual(set(grammar.COMMANDS) & REGISTRY, set())
        self.assertNotIn('xp_gained_events', REGISTRY)  # declared in PREDICATES for story 5.6 to register
        self.assertNotIn('sweep', grammar.ECONOMY_OPERATIONS)

    def test_inherited_form_checker_rejects_the_review_findings(self):
        for bad in ('nf.Target template:20003', 'nf.WaitFor target == template:20003 3', 'nf.WaitFor entity_dead template:20003 30', 'nf.WaitFor entity_in_view entity:b 15'):
            tokens = shlex.split(bad)
            if tokens[0] == 'nf.WaitFor':
                self.assertFalse(inherited_predicate_ok(tokens[1:-1]), bad)
            else:
                self.assertFalse(inherited_command_ok(tokens), bad)
        for good in ('nf.Target template:npc_20003', 'nf.Target 01970000-0000-7000-8000-000000000050', 'nf.Cast 254 target'):
            self.assertTrue(inherited_command_ok(shlex.split(good)), good)
        for good in ('target == Npc_20003', 'entity_dead target', 'entity_in_view 01970000-0000-7000-8000-000000000051', 'buff_active 2278', 'cast_results == 2', 'inventory_has 57 >= 1'):
            self.assertTrue(inherited_predicate_ok(shlex.split(good)), good)

    def test_role_split_keeps_header_and_budget(self):
        text = '# covers: 5 E1.8\n# fixture: phase5-trade\n# note\n# role: a\nnf.Login\nnf.Expect trade_state == STARTED\n# role: b\nnf.Login\nnf.TradeAnswer accept\n# Total\nnf.Within 60\n'
        self.assertEqual(grammar.roles_of(text), ['a', 'b'])
        parts = grammar.split_roles(text)
        self.assertEqual(set(parts), {'a', 'b'})
        self.assertEqual(parts['a'], ['# covers: 5 E1.8', '# fixture: phase5-trade', '# note', 'nf.Login', 'nf.Expect trade_state == STARTED', 'nf.Within 60'])
        self.assertEqual(parts['b'], ['# covers: 5 E1.8', '# fixture: phase5-trade', '# note', 'nf.Login', 'nf.TradeAnswer accept', '# Total', 'nf.Within 60'])
        solo = '# covers: 5 E1.8\n# fixture: phase5-herbs\nnf.Login\nnf.Within 40\n'
        self.assertEqual(grammar.roles_of(solo), ['a'])
        self.assertEqual(grammar.split_roles(solo), {'a': ['# covers: 5 E1.8', '# fixture: phase5-herbs', 'nf.Login', 'nf.Within 40']})
        with self.assertRaises(ValueError):
            grammar.roles_of('# role: a\n# role: a\nnf.Within 1\n')
        with self.assertRaises(ValueError):
            grammar.split_roles('# role: a\nnf.Login\n')
        for path in sorted((SCRIPTS.parent / 'Scenarios').glob('5-*.nfs')):
            text = path.read_text()
            self.assertNotIn('"', text, path.name)
            parts = grammar.split_roles(text)
            self.assertEqual(set(parts), set(grammar.roles_of(text)), path.name)
            for lines in parts.values():
                self.assertTrue(lines[0].startswith('# covers: 5 '), path.name)
                self.assertTrue(lines[-1].startswith('nf.Within '), path.name)
                self.assertIn('nf.Login', lines, path.name)


if __name__ == '__main__':
    unittest.main()
