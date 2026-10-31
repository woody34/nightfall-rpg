#!/usr/bin/env python3
"""Phase 5 scenario DSL contract (plan §5.3; story 6.1): syntax validation and the command/predicate
-> received-carrier map. Syntax here is separate from client registration (story 5.6 registers the
same names). Every predicate is UNKNOWN until its carrier arrives and resets on the listed events
(C11); UNKNOWN is never allow-listed.

Argument forms are those of apps/client-unreal/Scripts/phase5-fixtures.md. `<item>` is a template
id, `template:<id>` or `instance:<positive bigint>`; `<role>` is a pack role letter; `<entries>` is a
comma list of `template:<id>:<count>[:<price>]` (buy stores add `:<enchant>` before the count;
manufacture entries are `<recipe>:<fee>`). A trailing UUID is always a request key.
"""
import re
from uuid import UUID

COMPARISONS = {'==', '!=', '>=', '<=', '>', '<'}
ROLES = {'a', 'b', 'c'}
LOOT_MODES = {'FINDERS_KEEPERS', 'RANDOM', 'RANDOM_INCLUDING_SPOIL', 'BY_TURN', 'BY_TURN_INCLUDING_SPOIL'}
TRADE_PHASES = {'STARTED', 'ITEM_ADDED', 'PARTNER_CONFIRMED', 'COMPLETED', 'CANCELLED'}
STORE_KINDS = {'NONE', 'SELL', 'PACKAGE_SELL', 'BUY', 'MANUFACTURE'}
VOTE_PHASES = {'REQUESTED', 'APPLIED', 'CANCELLED'}
AUCTION_STATES = {'CREATED', 'STARTED', 'FINISHED'}
# EconomyResult.operation names (ItemOperation 14..42, lower case) plus the item operations a Phase 5
# file may wait for through the same predicate.
ECONOMY_OPERATIONS = {
    'pick_up_item', 'vendor_buy', 'vendor_sell', 'vendor_refund', 'multisell_choose', 'register_recipe', 'delete_recipe',
    'craft_item', 'trade_confirm', 'store_buy', 'store_sell', 'private_manufacture', 'warehouse_deposit', 'warehouse_withdraw',
    'freight_send', 'freight_withdraw', 'mail_send', 'mail_take', 'mail_reject', 'mail_cancel', 'mail_delete', 'manor_buy_seed',
    'manor_sell_crop', 'auction_bid', 'auction_cancel_bid', 'party', 'trade_session', 'store_setup', 'mail_mark_read',
}
REJECT_REASONS = {
    'GROUND_ITEM_NOT_FOUND', 'GROUND_ITEM_PROTECTED', 'INVALID_MERCHANT', 'INVALID_QUANTITY', 'VENDOR_OUT_OF_STOCK', 'ADENA_LIMIT',
    'WEIGHT_LIMIT', 'INSUFFICIENT_ADENA', 'ITEM_NOT_SELLABLE', 'ITEM_COUNT_CAP', 'RECIPE_ALREADY_REGISTERED', 'RECIPE_BOOK_FULL',
    'CRAFT_SKILL_MISSING', 'CRAFT_LEVEL_TOO_LOW', 'MISSING_MATERIALS', 'IN_COMBAT', 'NO_PENDING_REQUEST', 'ALREADY_TRADING',
    'TARGET_BUSY', 'CANNOT_TRADE_IN_STORE', 'CANNOT_ADJUST_AFTER_CONFIRM', 'ITEM_NOT_TRADABLE', 'STORE_SLOT_LIMIT', 'STORE_NOT_OPEN',
    'PACKAGE_INCOMPLETE', 'PRICE_MISMATCH', 'PARTY_FULL', 'PARTY_ALREADY_IN_PARTY', 'NOT_PARTY_LEADER', 'NOT_PARTY_MEMBER',
    'PARTY_LOOT_VOTE_IN_PROGRESS', 'CONTAINER_BUSY', 'WAREHOUSE_FULL', 'ITEM_NOT_DEPOSITABLE', 'FREIGHT_TARGET_INVALID',
    'ITEM_NOT_FREIGHTABLE', 'FREIGHT_FULL', 'PEACE_ZONE_REQUIRED', 'MAIL_ATTACHMENTS_EXCEEDED', 'MAIL_RECEIVER_NOT_FOUND',
    'MAILBOX_FULL', 'MAIL_NOT_FOUND', 'MAIL_ALREADY_TAKEN', 'SWEEP_NOT_ALLOWED', 'NOT_SEEDABLE', 'HARVEST_NOT_ALLOWED',
    'MANOR_UNAVAILABLE', 'AUCTION_NOT_ACTIVE', 'BID_TOO_LOW', 'BID_TOO_HIGH', 'NO_BID_TO_CANCEL',
    # reused existing reasons (plan §2.1)
    'RATE_LIMITED', 'DEAD_ACTOR', 'OUT_OF_RANGE', 'INSUFFICIENT_ITEMS', 'INVENTORY_FULL', 'ITEM_CAPABILITY_DEFERRED', 'INVALID',
}

# command -> (argument form, carrier that confirms it)
COMMANDS = {
    'nf.PickUp': ('template:<id>|nearest [key]', 'EntityDespawn + EconomyResult(pick_up_item)'),
    'nf.Buy': ('merchant <list_id> <template> <count> [key]', 'EconomyResult(vendor_buy) + InventoryChanged'),
    'nf.Sell': ('merchant <list_id> <item> <count> [key]', 'EconomyResult(vendor_sell) + InventoryChanged'),
    'nf.Refund': ('merchant <list_id> <index> [key]', 'EconomyResult(vendor_refund) + InventoryChanged'),
    'nf.Multisell': ('exchanger <list_id> <entry_index> <amount> [enchant:<n>] [key]', 'EconomyResult(multisell_choose)'),
    'nf.RegisterRecipe': ('<item> [key]', 'RecipeBookSync + EconomyResult(register_recipe)'),
    'nf.DeleteRecipe': ('<recipe_id> [key]', 'RecipeBookSync + EconomyResult(delete_recipe)'),
    'nf.Craft': ('<recipe_id> [key]', 'EconomyResult(craft_item)'),
    'nf.ManufactureOrder': ('<role> <recipe_id> [key]', 'EconomyResult(private_manufacture) for both parties'),
    'nf.TradeRequest': ('<role> [key]', 'TradeRequestReceived (target) / TradeSessionChanged'),
    'nf.TradeAnswer': ('accept|decline [key]', 'TradeSessionChanged'),
    'nf.TradeAdd': ('<item> <count> [key]', 'TradeSessionChanged'),
    'nf.TradeConfirm': ('[key]', 'TradeSessionChanged'),
    'nf.TradeCancel': ('[key]', 'TradeSessionChanged'),
    'nf.StoreSell': ('<title> [package] <sell-entries> [key]', 'PrivateStoreChanged'),
    'nf.StoreBuy': ('<title> <buy-entries> [key]', 'PrivateStoreChanged'),
    'nf.StoreManufacture': ('<title> <manufacture-entries> [key]', 'PrivateStoreChanged'),
    'nf.StoreClose': ('[key]', 'PrivateStoreChanged(NONE)'),
    'nf.StoreBuyFrom': ('<role> <sell-entries> [key]', 'EconomyResult(store_buy)'),
    'nf.StoreSellTo': ('<role> <sell-entries> [key]', 'EconomyResult(store_sell)'),
    'nf.PartyInvite': ('<role> [mode] [key]', 'PartyStateChanged / PartyInviteReceived (target)'),
    'nf.PartyAnswer': ('-1|0|1 [key]', 'PartyStateChanged'),
    'nf.PartyLeave': ('[key]', 'PartyStateChanged'),
    'nf.PartyKick': ('<role> [key]', 'PartyStateChanged'),
    'nf.PartyLeader': ('<role> [key]', 'PartyStateChanged'),
    'nf.PartyLootVote': ('<mode> [key]', 'PartyLootVote'),
    'nf.PartyLootAnswer': ('accept|decline [key]', 'PartyLootVote'),
    'nf.Deposit': ('<container-entries> [key]', 'EconomyResult(warehouse_deposit)'),
    'nf.Withdraw': ('<template>:<count>[,...] [key]', 'EconomyResult(warehouse_withdraw)'),
    'nf.FreightSend': ('<role> <container-entries> [key]', 'EconomyResult(freight_send)'),
    'nf.FreightWithdraw': ('<template>:<count>[,...] [key]', 'EconomyResult(freight_withdraw)'),
    'nf.MailSend': ('<role> <subject> [cod <adena>] [container-entries] [key]', 'EconomyResult(mail_send)'),
    'nf.MailTake': ('latest [key]', 'EconomyResult(mail_take)'),
    'nf.MailReject': ('latest [key]', 'EconomyResult(mail_reject)'),
    'nf.MailCancel': ('latest [key]', 'EconomyResult(mail_cancel)'),
    'nf.MailDelete': ('latest [sent] [key]', 'EconomyResult(mail_delete)'),
    'nf.MailRead': ('latest [key]', 'EconomyResult(mail_mark_read)'),
    'nf.AuctionBid': ('<instance_npc_id> <bid> [key]', 'EconomyResult(auction_bid) + ItemAuctionStatusChanged'),
    'nf.AuctionCancel': ('<instance_npc_id> [key]', 'EconomyResult(auction_cancel_bid)'),
    'nf.ManorBuySeed': ('manor_manager <manor_id> <seed> <n> [key]', 'EconomyResult(manor_buy_seed)'),
    'nf.ManorSellCrop': ('manor_manager <item> <crop_id> <manor_id> <count>|all [key]', 'EconomyResult(manor_sell_crop)'),
}

# predicate -> (argument form, authoritative carrier, reset events)
RESET_IDENTITY = 'generation/incarnation/identity baseline'
PREDICATES = {
    'ground_item_exists': ('<template>', 'EntitySpawn kind GROUND_ITEM / EntityDespawn', RESET_IDENTITY),
    'ground_item_kind': ('<template> == 3', 'EntitySpawn.kind', RESET_IDENTITY),
    'ground_loot_owner': ('<template> ==|!= self', 'EntitySpawn.ground_item.loot_owner', RESET_IDENTITY),
    'ground_drop_protection': ('<template> ==|!= self', 'EntitySpawn.ground_item.drop_protect_owner', RESET_IDENTITY),
    'ground_drop_protection_none': ('', 'EntitySpawn.ground_item (both owners empty) for every received PK drop', RESET_IDENTITY),
    'ground_protection_expired': ('<template>', 'received tick >= loot_until_tick and drop_protect_until_tick', RESET_IDENTITY),
    'ground_item_count_between': ('<template> <lo> <hi>', 'EntitySpawn.ground_item.count', RESET_IDENTITY),
    'ground_item_gone': ('<template>', 'EntityDespawn of the known ground entity', RESET_IDENTITY),
    'ground_despawn_on_grid': ('<template>', 'EntityDespawn tick == 0 (mod 50) and > dropped_tick + limit (sidecar)', RESET_IDENTITY),
    'ground_item_any': ('', 'any EntitySpawn kind GROUND_ITEM', RESET_IDENTITY),
    'ground_item_count': ('<comparison> <n>', 'count of known ground entities', RESET_IDENTITY),
    'economy_result': ('== <operation>', 'last owner EconomyResult.operation', 'owner incarnation / baseline'),
    'economy_result_count': ('<operation> <comparison> <n>', 'owner EconomyResult envelopes by operation', 'owner incarnation / baseline'),
    'economy_export_marker': ('', 'EconomyResult or EntityDied of the kill (export binding point)', RESET_IDENTITY),
    'own_level_between': ('<lo> <hi>', 'owner StatsChanged.level', 'owner baseline'),
    'rules_rate_multiplier': ('== <decimal>', 'InventorySync.rules_hash resolved against the Phase5Rules manifest (G string)', 'owner baseline'),
    'herb_skill_early_abort': ('<template>', 'EconomyResult(pick_up_item).pickup.herb with no new EffectApplied for a skill in reuse', 'owner incarnation'),
    'pk_drop_decays': ('== false', 'no EntityDespawn of a PK-dropped ground entity before the limit', RESET_IDENTITY),
    'party_member_count': ('<comparison> <n>', 'PartyStateChanged.members', 'PartyStateChanged.disbanded / baseline'),
    'loot_mode': ('== <mode>', 'PartyStateChanged.mode', 'PartyStateChanged / baseline'),
    'party_invite_received': ('', 'PartyInviteReceived', 'PartyStateChanged / expires_tick'),
    'loot_recipient_in_party': ('', 'EconomyResult.pickup.recipient in PartyStateChanged.members', 'owner incarnation'),
    'loot_recipient': ('!= self', 'LootDistributed.recipient', RESET_IDENTITY),
    'by_turn_cursor_advanced': ('', 'two LootDistributed/EconomyResult recipients differ in member order', RESET_IDENTITY),
    'loot_distributed': ('', 'LootDistributed', RESET_IDENTITY),
    'loot_distributed_mode': ('== <mode>', 'LootDistributed.mode', RESET_IDENTITY),
    'loot_distributed_count': ('<comparison> <n>', 'LootDistributed envelopes', RESET_IDENTITY),
    'loot_distributed_spoil': ('== true|false', 'LootDistributed.spoil', RESET_IDENTITY),
    'own_out_of_party_range': ('', 'own EntitySpawn/EntityMove position vs the leader beyond PartyRange2', RESET_IDENTITY),
    'corpse_spoiled': ('template:npc_<id>', 'CorpseStateChanged.spoiled', RESET_IDENTITY),
    'corpse_sweepable': ('template:npc_<id>', 'CorpseStateChanged.sweepable', RESET_IDENTITY),
    'corpse_swept': ('template:npc_<id>', 'CorpseStateChanged.swept', RESET_IDENTITY),
    'corpse_until_delta': ('template:npc_<id> == <ticks>', 'CorpseStateChanged.tick at death vs EntityDespawn tick', RESET_IDENTITY),
    'sweep_recipient_in_party': ('', 'the sweep recipient is a party member: LootDistributed.recipient (spoil = true) in PartyStateChanged.members, or the owner itself when its inventory revision advanced since nf.MarkItemState (no LootDistributed reaches the recipient, D §4.2)', RESET_IDENTITY),
    'sweep_outcome_received': ('', 'LootDistributed (spoil = true) received, or the owner InventoryChanged revision advanced since nf.MarkItemState: recipient-agnostic (D §4.2)', 'mark / identity baseline'),
    'party_sweep_done': ('template:npc_<id>', 'CorpseStateChanged.swept for the known corpse', RESET_IDENTITY),
    'adena_split_share_between': ('<lo> <hi>', 'owner InventoryChanged adena delta since nf.MarkItemState', 'mark / owner baseline'),
    'adena_split_origin': ('== npc', 'LootDistributed/EconomyResult positions equal the corpse origin', RESET_IDENTITY),
    'loot_vote_phase': ('== <phase>', 'PartyLootVote.phase', 'PartyStateChanged / expires_tick'),
    'xp_gained_events': ('<comparison> <n>', 'owner XpGained envelopes after the identity baseline (Phase 5 registers it; no Phase 4 bot does)', 'owner incarnation'),
    'xp_gained_last': ('== <n>', 'last owner XpGained.amount', 'owner incarnation'),
    'sp_gained_last': ('== <n>', 'last owner SpGained.amount', 'owner incarnation'),
    'xp_gained_last_between': ('<lo> <hi>', 'last owner XpGained.amount', 'owner incarnation'),
    'xp_share_matches_export': ('', 'owner XpGained.amount equals the econ_formula solo_xp actual of the C14 export', 'owner incarnation'),
    'sp_gained_events_for': ('template:npc_<id> == <n>', 'owner SpGained envelopes after the kill of that template', 'owner incarnation'),
    'entity_gone': ('template:npc_<id>', 'EntityDespawn of the known entity', RESET_IDENTITY),
    'own_pk_count': ('<comparison> <n>', 'owner StatsChanged.pk_count', 'owner baseline'),
    'pk_drop_count_between': ('<lo> <hi>', 'owner InventoryChanged.removed_ids at death', 'owner incarnation'),
    'pk_dropped_excluded': ('== 0', 'removed_ids intersect the sidecar exclusion set', 'owner incarnation'),
    'pk_unequipped_before_roll': ('', 'EquipmentChanged clears precede InventoryChanged.removed_ids at the same tick', 'owner incarnation'),
    'merchant_known': ('', 'EntitySpawn of the noncombat entity named merchant', RESET_IDENTITY),
    'adena_delta': ('== <signed>', 'owner adena count minus the nf.MarkItemState sample', 'mark / owner baseline'),
    'refund_tab_count': ('== <n>', 'ListBuylistResponse.refund_tab', 'owner departure'),
    'refund_tab_oldest_destroyed': ('', 'refund_tab lacks the first sold entry after 13 sells', 'owner departure'),
    'vendor_stock': ('<list_id> <template> == <n>', 'ListBuylistResponse.entries.stock', 'state_tick'),
    'vendor_restock_due_delta': ('<list_id> <template> == <ticks>', 'C14 export vendor restock due tick minus the out-of-stock tick', 'export'),
    'exchanger_known': ('', 'EntitySpawn of the noncombat entity named exchanger', RESET_IDENTITY),
    'enchant_of': ('<template> == <n>', 'owner OwnedItem.enchant', 'owner baseline'),
    'craft_result_any': ('', 'EconomyResult.craft', 'owner incarnation'),
    'craft_result': ('== success|fail|masterwork', 'EconomyResult.craft.success / masterwork', 'owner incarnation'),
    'craft_materials_consumed': ('<recipe_id>', 'InventoryChanged removes the sidecar materials of the recipe', 'owner incarnation'),
    'craft_masterwork': ('== true|false', 'EconomyResult.craft.masterwork', 'owner incarnation'),
    'craft_fee_before_roll': ('', 'EconomyResult(private_manufacture).craft.fee > 0 on a failed craft', 'owner incarnation'),
    'store_state': ('== <kind>', 'own PrivateStoreChanged.kind', RESET_IDENTITY),
    'store_state_of': ('<role> == <kind>', 'PrivateStoreChanged.kind of that owner', RESET_IDENTITY),
    'manufacture_gate_ticks': ('== <n>', 'Ack tick difference between two accepted PrivateManufactureOrder intents', 'owner incarnation'),
    'own_trade_confirmed': ('', 'TradeSessionChanged.own_confirmed', RESET_IDENTITY),
    'trade_state': ('== <phase>', 'TradeSessionChanged.phase', RESET_IDENTITY),
    'trade_transfer_pairs': ('== <n>', 'EconomyResult(trade_confirm).trade given/received rows', 'owner incarnation'),
    'trade_request_received': ('', 'TradeRequestReceived', 'expires_tick / identity'),
    'trade_cancel_reason': ('== departure|out_of_range|use_item|death', 'TradeSessionChanged.cancel_reason', RESET_IDENTITY),
    'store_entries_skipped': ('== <n>', 'EconomyResult.store.skipped', 'owner incarnation'),
    'warehouse_keeper_known': ('', 'EntitySpawn of the noncombat entity named warehouse_keeper', RESET_IDENTITY),
    'container_fee': ('== <n>', 'EconomyResult.container.fee', 'owner incarnation'),
    'warehouse_has': ('<template> == <n>', 'GetWarehouseResponse.items', 'revision'),
    'container_busy_retry_observed': ('', 'IntentRejected CONTAINER_BUSY followed by an accepted retry with the same key', 'owner incarnation'),
    'freight_capacity': ('== <n>', 'GetFreightResponse.capacity', 'revision'),
    'freight_has': ('<template> == <n>', 'GetFreightResponse.items', 'revision'),
    'cod_credit_received': ('<adena>', 'owner InventoryChanged adena delta from a cod_credit delivery', 'owner incarnation'),
    'mail_received': ('<subject>', 'MailNotification.subject / ListMailResponse', 'ListMail'),
    'mail_expiry_hours': ('== <n>', 'ListMailResponse header expires_unix_ms minus created (DB time, 360 h)', 'ListMail'),
    'mail_gone': ('<subject>', 'ListMailResponse no longer lists the message', 'ListMail'),
    'mail_has_attachments': ('<subject>', 'MailHeader.has_attachments', 'ListMail'),
    'auctioneer_known': ('', 'EntitySpawn of the noncombat entity named auctioneer', RESET_IDENTITY),
    'auction_state': ('== <state>', 'ItemAuctionStatusChanged.state', RESET_IDENTITY),
    'auction_highest_bid': ('== <n>', 'ItemAuctionStatusChanged.highest_bid', RESET_IDENTITY),
    'auction_award_to_warehouse': ('<template>', 'GetWarehouseResponse lists the template after FINISHED', 'revision'),
    'auction_leader': ('!= self', 'AuctionView.own_bid < highest_bid in ListItemAuctionsResponse', 'ListItemAuctions'),
    'manor_manager_known': ('', 'EntitySpawn of the noncombat entity named manor_manager', RESET_IDENTITY),
    'corpse_seeded_or_sow_failed': ('template:npc_<id>', 'CorpseStateChanged.seeded or SkillCastResult of Sow', RESET_IDENTITY),
    'harvest_done': ('template:npc_<id>', 'SkillCastResult of Harvest + InventoryChanged', RESET_IDENTITY),
    'manor_drops_unchanged': ('', 'ground EntitySpawn set equals the death list expectation (E13)', RESET_IDENTITY),
    'manor_fee_equals_p_div_20': ('', 'EconomyResult.manor.fee == adena // 20', 'owner incarnation'),
    'manor_result': ('granted <template> >= <n>', 'EconomyResult.manor.granted', 'owner incarnation'),
    'epoch_reconcile_rows_before_admission': ('', 'C14 export ledger destroy rows with cause epoch_void precede the admission tick', 'export'),
    'recipe_registered': ('<recipe_id>', 'RecipeBookSync lists the recipe', 'owner baseline'),
    'rejected': ('== <reason>', 'IntentRejected.reason', 'next sent intent'),
}


def number(text, maximum=2**64 - 1, zero=False):
    return bool(re.fullmatch(r'[0-9]+', text)) and int(zero is False) <= int(text) <= maximum


def signed(text):
    return bool(re.fullmatch(r'-?[0-9]+', text))


def guid(text):
    try:
        return str(UUID(text)) == text.lower() and UUID(text).int != 0
    except ValueError:
        return False


def item(text):
    if text.startswith('instance:'):
        return number(text[9:], 2**63 - 1)
    return number(text.removeprefix('template:'), 2**32 - 1)


def template_ref(text):
    return text.startswith('template:') and number(text[9:], 2**32 - 1)


def npc_ref(text):
    """`template:npc_<pinned id>`: the zone template id of an overlay monster (phase5-fixtures.py), the same
    string `nf.Target template:<id>` resolves (ResolveTemplateTarget)."""
    return bool(re.fullmatch(r'template:npc_[1-9][0-9]{0,9}', text))


def entries(text, parts, allow_bare=False):
    """`template:<id>:<a>[:<b>]` comma list with exactly `parts` numeric fields after the template."""
    if not text:
        return False
    for entry in text.split(','):
        fields = entry.split(':')
        if fields[0] == 'template' and len(fields) == 2 + parts and number(fields[1], 2**32 - 1) and all(number(f, 2**63 - 1, zero=True) for f in fields[2:]):
            continue
        if allow_bare and len(fields) == 1 + parts and number(fields[0], 2**32 - 1) and all(number(f, 2**63 - 1) for f in fields[1:]):
            continue
        return False
    return True


def command(tokens):
    """None means outside this contract; False means recognized but malformed."""
    name = tokens[0] if tokens[0] in COMMANDS else None  # exact case, like the bot and the scenarios test
    if name is None:
        return None
    args = list(tokens[1:])
    if args and guid(args[-1]):
        args.pop()
    role = lambda s: s in ROLES  # noqa: E731
    n = len(args)
    if name == 'nf.PickUp': return n == 1 and (args[0] == 'nearest' or template_ref(args[0]))
    if name == 'nf.Buy': return n == 4 and args[0] == 'merchant' and number(args[1]) and number(args[2], 2**32 - 1) and number(args[3], 2**63 - 1)
    if name == 'nf.Sell': return n == 4 and args[0] == 'merchant' and number(args[1]) and item(args[2]) and number(args[3], 2**63 - 1)
    if name == 'nf.Refund': return n == 3 and args[0] == 'merchant' and number(args[1]) and number(args[2], 11, zero=True)
    if name == 'nf.Multisell':
        return n in (4, 5) and args[0] == 'exchanger' and number(args[1], 2**32 - 1) and number(args[2], 2**32 - 1, zero=True) and number(args[3], 5000) \
            and (n == 4 or args[4].startswith('enchant:') and number(args[4][8:], 2**32 - 1, zero=True))
    if name == 'nf.RegisterRecipe': return n == 1 and item(args[0])
    if name in ('nf.DeleteRecipe', 'nf.Craft'): return n == 1 and number(args[0], 2**32 - 1)
    if name == 'nf.ManufactureOrder': return n == 2 and role(args[0]) and number(args[1], 2**32 - 1)
    if name in ('nf.TradeRequest', 'nf.PartyKick', 'nf.PartyLeader'): return n == 1 and role(args[0])
    if name in ('nf.TradeAnswer', 'nf.PartyLootAnswer'): return n == 1 and args[0] in ('accept', 'decline')
    if name == 'nf.TradeAdd': return n == 2 and item(args[0]) and number(args[1], 2**63 - 1, zero=True)
    if name in ('nf.TradeConfirm', 'nf.TradeCancel', 'nf.StoreClose', 'nf.PartyLeave'): return n == 0
    if name == 'nf.StoreSell': return n in (2, 3) and (n == 2 or args[1] == 'package') and entries(args[-1], 2)
    if name == 'nf.StoreBuy': return n == 2 and entries(args[1], 3)
    if name == 'nf.StoreManufacture': return n == 2 and all(len(e.split(':')) == 2 and number(e.split(':')[0], 2**32 - 1) and number(e.split(':')[1], 2**63 - 1) for e in args[1].split(','))
    if name in ('nf.StoreBuyFrom', 'nf.StoreSellTo'): return n == 2 and role(args[0]) and entries(args[1], 2)
    if name == 'nf.PartyInvite': return n in (1, 2) and role(args[0]) and (n == 1 or args[1] in LOOT_MODES)
    if name == 'nf.PartyAnswer': return n == 1 and args[0] in ('-1', '0', '1')
    if name == 'nf.PartyLootVote': return n == 1 and args[0] in LOOT_MODES
    if name == 'nf.Deposit': return n == 1 and entries(args[0], 1)
    if name in ('nf.Withdraw', 'nf.FreightWithdraw'): return n == 1 and entries(args[0], 1, allow_bare=True)
    if name == 'nf.FreightSend': return n == 2 and role(args[0]) and entries(args[1], 1)
    if name == 'nf.MailSend':
        if n < 2 or not role(args[0]) or not re.fullmatch(r'[A-Za-z0-9_-]{1,128}', args[1]): return False
        rest = args[2:]
        if rest[:1] == ['cod']:
            if len(rest) < 2 or not number(rest[1], 99_900_000_000, zero=True): return False
            rest = rest[2:]
        return not rest or (len(rest) == 1 and entries(rest[0], 1))
    if name in ('nf.MailTake', 'nf.MailReject', 'nf.MailCancel', 'nf.MailRead'): return n == 1 and args[0] == 'latest'
    if name == 'nf.MailDelete': return n in (1, 2) and args[0] == 'latest' and (n == 1 or args[1] == 'sent')
    if name == 'nf.AuctionBid': return n == 2 and number(args[0], 2**32 - 1) and number(args[1], 100_000_000_000)
    if name == 'nf.AuctionCancel': return n == 1 and number(args[0], 2**32 - 1)
    if name == 'nf.ManorBuySeed': return n == 4 and args[0] == 'manor_manager' and number(args[1], 9) and number(args[2], 2**32 - 1) and number(args[3], 2**63 - 1)
    if name == 'nf.ManorSellCrop':
        return n == 5 and args[0] == 'manor_manager' and item(args[1]) and number(args[2], 2**32 - 1) and number(args[3], 9) and (args[4] == 'all' or number(args[4], 2**63 - 1))
    return False


def predicate(tokens):
    """None means outside this contract; False means recognized but malformed."""
    if not tokens or tokens[0] not in PREDICATES:
        return None
    name, args = tokens[0], list(tokens[1:])
    form = PREDICATES[name][0]
    n = len(args)
    if form == '':
        return n == 0
    if name in ('ground_item_exists', 'ground_protection_expired', 'ground_item_gone', 'ground_despawn_on_grid', 'herb_skill_early_abort',
                'auction_award_to_warehouse'):
        return n == 1 and number(args[0], 2**32 - 1)
    if name in ('corpse_spoiled', 'corpse_sweepable', 'corpse_swept', 'party_sweep_done', 'entity_gone', 'corpse_seeded_or_sow_failed', 'harvest_done'):
        return n == 1 and npc_ref(args[0])
    if name == 'ground_item_kind': return n == 3 and number(args[0], 2**32 - 1) and args[1] == '==' and args[2] == '3'
    if name in ('ground_loot_owner', 'ground_drop_protection'): return n == 3 and number(args[0], 2**32 - 1) and args[1] in ('==', '!=') and args[2] == 'self'
    if name == 'ground_item_count_between': return n == 3 and number(args[0], 2**32 - 1) and number(args[1], zero=True) and number(args[2], zero=True) and int(args[1]) <= int(args[2])
    if name in ('own_level_between', 'xp_gained_last_between', 'pk_drop_count_between', 'adena_split_share_between'):
        return n == 2 and number(args[0], zero=True) and number(args[1], zero=True) and int(args[0]) <= int(args[1])
    if name == 'economy_result': return n == 2 and args[0] == '==' and args[1] in ECONOMY_OPERATIONS
    if name == 'economy_result_count': return n == 3 and args[0] in ECONOMY_OPERATIONS and args[1] in COMPARISONS and number(args[2], zero=True)
    if name == 'rules_rate_multiplier': return n == 2 and args[0] == '==' and bool(re.fullmatch(r'[0-9]+(\.[0-9]+)?', args[1]))
    if name in ('pk_drop_decays', 'loot_distributed_spoil', 'craft_masterwork'): return n == 2 and args[0] == '==' and args[1] in ('true', 'false')
    if name in ('loot_mode', 'loot_distributed_mode'): return n == 2 and args[0] == '==' and args[1] in LOOT_MODES
    if name in ('loot_recipient', 'auction_leader'): return n == 2 and args[0] == '!=' and args[1] == 'self'
    if name == 'loot_vote_phase': return n == 2 and args[0] == '==' and args[1] in VOTE_PHASES
    if name == 'trade_state': return n == 2 and args[0] == '==' and args[1] in TRADE_PHASES
    if name == 'trade_cancel_reason': return n == 2 and args[0] == '==' and args[1] in ('departure', 'out_of_range', 'use_item', 'death')
    if name == 'store_state': return n == 2 and args[0] == '==' and args[1] in STORE_KINDS
    if name == 'store_state_of': return n == 3 and args[0] in ROLES and args[1] == '==' and args[2] in STORE_KINDS
    if name == 'auction_state': return n == 2 and args[0] == '==' and args[1] in AUCTION_STATES
    if name == 'adena_split_origin': return n == 2 and args[0] == '==' and args[1] == 'npc'
    if name == 'craft_result': return n == 2 and args[0] == '==' and args[1] in ('success', 'fail', 'masterwork')
    if name == 'rejected': return n == 2 and args[0] == '==' and args[1] in REJECT_REASONS
    if name == 'adena_delta': return n == 2 and args[0] == '==' and signed(args[1])
    if name in ('corpse_until_delta', 'sp_gained_events_for'): return n == 3 and npc_ref(args[0]) and args[1] == '==' and number(args[2], zero=True)
    if name in ('vendor_stock', 'vendor_restock_due_delta'): return n == 4 and number(args[0]) and number(args[1], 2**32 - 1) and args[2] == '==' and number(args[3], zero=True)
    if name in ('enchant_of', 'warehouse_has', 'freight_has'): return n == 3 and number(args[0], 2**32 - 1) and args[1] == '==' and number(args[2], zero=True)
    if name in ('craft_materials_consumed', 'recipe_registered'): return n == 1 and number(args[0], 2**32 - 1)
    if name == 'cod_credit_received': return n == 1 and number(args[0])
    if name in ('mail_received', 'mail_gone', 'mail_has_attachments'): return n == 1 and bool(re.fullmatch(r'[A-Za-z0-9_-]{1,128}', args[0]))
    if name == 'manor_result': return n == 4 and args[0] == 'granted' and number(args[1], 2**32 - 1) and args[2] == '>=' and number(args[3], zero=True)
    if name in ('xp_gained_last', 'sp_gained_last', 'refund_tab_count', 'manufacture_gate_ticks', 'trade_transfer_pairs', 'store_entries_skipped',
                'container_fee', 'freight_capacity', 'mail_expiry_hours', 'auction_highest_bid', 'pk_dropped_excluded'):
        return n == 2 and args[0] == '==' and number(args[1], zero=True) and (name != 'pk_dropped_excluded' or args[1] == '0')
    if name in ('ground_item_count', 'party_member_count', 'loot_distributed_count', 'own_pk_count', 'xp_gained_events'):
        return n == 2 and args[0] in COMPARISONS and number(args[1], zero=True)
    return False


def roles_of(text):
    """Role blocks of one scenario file: ordered role names (['a'] when the file has none)."""
    found = re.findall(r'^#\s*role:\s*([abc])\s*$', text, re.M)
    if len(found) != len(set(found)):
        raise ValueError('duplicate role block')
    return found or ['a']


def split_roles(text):
    """{role: lines} — shared header comments and nf.Within go to every role; commands go to their block."""
    lines = text.splitlines()
    header, body, within = [], {}, None
    current = None
    for line in lines:
        m = re.fullmatch(r'#\s*role:\s*([abc])\s*', line)
        if m:
            current = m[1]
            body.setdefault(current, [])
            continue
        if line.startswith('nf.Within'):
            within = line
            continue
        if current is None:
            header.append(line)
        else:
            body[current].append(line)
    if not body:
        commands = [l for l in header if l.startswith('nf.')]
        header = [l for l in header if not l.startswith('nf.')]
        body = {'a': commands}
    if within is None:
        raise ValueError('scenario budget nf.Within is required')
    return {role: header + lines_ + [within] for role, lines_ in body.items()}
