//! V7 NPC vendors (story 3.7, design §6): the merchant binding (P5-D29, D58 pattern), the
//! pinned flood gates (§2.4), `VendorBuy` (`RequestBuyItem.runImpl`), `VendorSell`
//! (`RequestSellItem`) into the refund tab (`PcRefund`), `VendorRefund`
//! (`RequestRefundItem`), limited stock with its phase-2 restock (§6.4), and the refund-tab
//! destroy at departure (B6). Every request is validated whole before anything changes
//! (rules P12/P13); only a passed flood gate survives a later refusal, as pinned.

use std::collections::{BTreeMap, BTreeSet};

use crate::domain::zone::item_inventory::{
    count_cap, inventory_totals, InventoryBoundary, InventoryChange, InventoryEdit,
    InventoryLimits, InventoryMutation, MAX_ADENA,
};
use crate::domain::zone::phase4_projection::{ItemAuditEvidence, ItemEvidenceValue, ItemOperation};
use crate::domain::zone::phase5::{FloodKind, VendorStock, REFUND_TAB_LIMIT};
use crate::domain::zone::phase5_event::{
    transfer_link, EconomyLedgerRow, EconomyOutcome, EconomyScope, LedgerOp, OwnerKind,
    TransferMode, TransferRef,
};
use crate::domain::zone::{
    EntityId, EntityKind, ItemCreationOrigin, ItemInstance, ItemInstanceId, ItemLocation,
    Phase5Command, RejectReason, Tick, Vec2Fixed, ZoneEvent, ZoneState,
};

use super::phase5_pickup::CharacterRow;

/// `MAX_ITEM_IN_PACKET` (`RequestBuyItem.java:56-73`).
const MAX_ITEMS: usize = 250;
/// `isInsideRadius(merchant, 150, false, false)` in milli-tiles: `m(150) = 4,687`.
pub(super) const INTERACTION_RANGE_MILLI: i64 = 4_687;
/// Adena.
const ADENA: u32 = 57;

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

fn square(x: i64) -> i64 {
    x.saturating_mul(x)
}

fn dist_sq(a: Vec2Fixed, b: Vec2Fixed) -> i64 {
    let dx = i64::from(a.x.raw()).saturating_sub(i64::from(b.x.raw()));
    let dy = i64::from(a.y.raw()).saturating_sub(i64::from(b.y.raw()));
    square(dx).saturating_add(square(dy))
}

/// The stack an addition merges into (`ItemContainer.addItem`).
fn stack_of(
    items: &BTreeMap<ItemInstanceId, ItemInstance>,
    template: u32,
) -> Option<&ItemInstance> {
    items.values().find(|i| {
        i.template_id == template && i.enchant == 0 && i.augment_id.is_none() && i.charges == 0
    })
}

impl ZoneState {
    /// A fixture merchant (P5-D29): a non-combat NPC bound by name and exact position to a
    /// pinned NPC id, within `m(150)` of the actor (2D, non-strict). Returns the NPC id.
    pub(super) fn merchant_v7(
        &self,
        actor: EntityId,
        merchant: EntityId,
    ) -> Result<u32, RejectReason> {
        let id = self.merchant_binding_v7(merchant)?;
        self.merchant_range_v7(actor, merchant)?;
        Ok(id)
    }

    /// The pinned NPC id a fixture merchant entity is bound to, else `INVALID_MERCHANT`.
    pub(super) fn merchant_binding_v7(&self, merchant: EntityId) -> Result<u32, RejectReason> {
        let rules = &self
            .phase5
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?
            .rules;
        let npc = self
            .entities
            .get(&merchant)
            .ok_or(RejectReason::InvalidMerchant)?;
        if npc.kind != EntityKind::Npc
            || npc.combat.is_some()
            || npc.targeting.attackable
            || npc.targeting.dead
        {
            return Err(RejectReason::InvalidMerchant);
        }
        rules
            .merchant_fixture(&npc.name, npc.pos)
            .ok_or(RejectReason::InvalidMerchant)
    }

    /// The `m(150)` interaction range to a merchant (2D, non-strict), else `OUT_OF_RANGE`.
    pub(super) fn merchant_range_v7(
        &self,
        actor: EntityId,
        merchant: EntityId,
    ) -> Result<(), RejectReason> {
        let npc = self
            .entities
            .get(&merchant)
            .ok_or(RejectReason::InvalidMerchant)?;
        let me = self
            .entities
            .get(&actor)
            .ok_or(RejectReason::UnknownEntity)?;
        if dist_sq(me.pos, npc.pos) > square(INTERACTION_RANGE_MILLI) {
            return Err(RejectReason::OutOfRange);
        }
        Ok(())
    }

    /// `FloodProtectorAction.tryPerformAction` (design §2.4): refuse while `tick < next`;
    /// otherwise `next := tick + interval`, kept even when a later check refuses.
    pub(super) fn flood_gate_v7(
        &mut self,
        tick: Tick,
        actor: EntityId,
        kind: FloodKind,
    ) -> Result<(), RejectReason> {
        let phase5 = self
            .phase5
            .as_mut()
            .ok_or(RejectReason::NotYetImplemented)?;
        let intervals = &phase5.rules.parts().storage_mail.flood_intervals_ticks;
        let interval = match kind {
            FloodKind::Transaction => intervals.transaction,
            FloodKind::MultiSell => intervals.multisell,
            FloodKind::Manufacture => intervals.manufacture,
            FloodKind::Manor => intervals.manor,
            FloodKind::SendMail => intervals.send_mail,
            FloodKind::ServerBypass => intervals.server_bypass,
        };
        let player = phase5
            .canonical
            .players
            .get_mut(&actor)
            .ok_or(RejectReason::InvalidLoad)?;
        if player.flood.get(&kind).is_some_and(|next| tick < *next) {
            return Err(RejectReason::RateLimited);
        }
        player
            .flood
            .insert(kind, Tick(tick.0.saturating_add(interval)));
        Ok(())
    }

    /// D79: the owner's revision for this tick's change (one step per changed owner-tick).
    pub(super) fn economy_revision(
        &self,
        owner: EntityId,
        events: &[ZoneEvent],
    ) -> Result<u64, RejectReason> {
        let player = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&owner))
            .ok_or(RejectReason::InvalidLoad)?;
        if events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(p) if p.entity == owner))
        {
            Ok(player.inventory_revision)
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)
        }
    }

    /// Apply `edits` to `owner`'s inventory all-or-nothing (dry run first), one Phase 4
    /// `InventoryMutation` per edit so each edit's changes stay attributable. Returns the
    /// changes per edit; on any failure nothing has changed.
    pub(super) fn economy_commit(
        &mut self,
        tick: Tick,
        owner: EntityId,
        edits: &[InventoryEdit],
        limits: InventoryLimits,
        events: &[ZoneEvent],
    ) -> Result<Vec<Vec<InventoryChange>>, RejectReason> {
        let revision = self.economy_revision(owner, events)?;
        let boundary = InventoryBoundary {
            tick,
            revision,
            origin: ItemCreationOrigin {
                zone: self.seed.zone,
                epoch: self.seed.epoch,
            },
        };
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        let mut inventory = player.inventory.clone();
        let mut allocator = items.canonical.allocator;
        let mut weight = player.weight;
        let mut per_edit = Vec::with_capacity(edits.len());
        for edit in edits {
            let mutation = InventoryMutation::prepare(
                &inventory,
                allocator,
                std::slice::from_ref(edit),
                &items.rules,
                limits,
                boundary,
            )
            .map_err(|_| RejectReason::InventoryFull)?;
            inventory = mutation.inventory;
            allocator = mutation.allocator;
            weight = mutation.weight;
            per_edit.push(mutation.changes);
        }
        let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        items.canonical.allocator = allocator;
        let player = items
            .canonical
            .players
            .get_mut(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        player.inventory = inventory;
        player.weight = weight;
        player.inventory_revision = revision;
        self.rebuild_v5(owner).map_err(invalid)?;
        Ok(per_edit)
    }

    /// One economy debit (P5-D26) inside the owner's D66 Consume aggregate.
    pub(super) fn push_economy_consume(
        &self,
        tick: Tick,
        owner: EntityId,
        change: InventoryChange,
        receipt: &str,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let before = change.before.as_ref().map_or(0, |i| i.count);
        let after = change.after.as_ref().map_or(0, |i| i.count);
        let count_delta = i64::try_from(after)
            .map_err(invalid)?
            .checked_sub(i64::try_from(before).map_err(invalid)?)
            .ok_or(RejectReason::InvalidLoad)?;
        let index = self.economy_projection(tick, owner, events)?;
        let Some(ZoneEvent::ItemCheckpoint(projection)) = events.get_mut(index) else {
            return Err(RejectReason::InvalidLoad);
        };
        super::item_commands::append_item_consumption(
            projection,
            change,
            count_delta,
            ItemAuditEvidence {
                family: "economy".into(),
                fields: BTreeMap::from([
                    ("receipt".to_owned(), ItemEvidenceValue::Text(receipt.to_owned())),
                    ("count_delta".to_owned(), ItemEvidenceValue::Signed(count_delta)),
                ]),
            },
        )
    }

    /// A positive economy change (grant or merge) as a character row of `operation`.
    #[allow(clippy::too_many_arguments)] // One row's explicit inputs.
    pub(super) fn push_economy_grant(
        &self,
        tick: Tick,
        owner: EntityId,
        change: InventoryChange,
        operation: ItemOperation,
        receipt: &str,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<ItemInstanceId, RejectReason> {
        let after = change.after.ok_or(RejectReason::InvalidLoad)?;
        let id = after.id;
        let count_delta = i64::try_from(after.count)
            .map_err(invalid)?
            .checked_sub(
                i64::try_from(change.before.as_ref().map_or(0, |i| i.count)).map_err(invalid)?,
            )
            .ok_or(RejectReason::InvalidLoad)?;
        self.push_character_row(
            tick,
            owner,
            CharacterRow {
                operation,
                before: change.before,
                after: Some(after),
                count_delta,
                receipt: Some(receipt),
                cause: None,
                transfer: None,
            },
            events,
        )?;
        Ok(id)
    }

    fn adena_of(&self, owner: EntityId) -> u64 {
        self.phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&owner))
            .map_or(0, |p| {
                p.inventory
                    .values()
                    .filter(|i| i.template_id == ADENA)
                    .map(|i| i.count)
                    .fold(0, u64::saturating_add)
            })
    }

    /// Weight and slot room for additions `(template, count)` (`validateWeight`,
    /// `validateCapacity`): `WEIGHT_LIMIT` first, then `INVENTORY_FULL`.
    pub(super) fn room_for_v7(
        &self,
        owner: EntityId,
        additions: &[(u32, u64)],
    ) -> Result<(), RejectReason> {
        let inventory = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&owner))
            .ok_or(RejectReason::InvalidLoad)?
            .inventory
            .clone();
        self.room_in_v7(owner, &inventory, additions)
    }

    /// [`Self::room_for_v7`] over `inventory` (e.g. the inventory after a choose's inputs
    /// are paid, design §7).
    pub(super) fn room_in_v7(
        &self,
        owner: EntityId,
        inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
        additions: &[(u32, u64)],
    ) -> Result<(), RejectReason> {
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        let (mut ordinary, mut quest, weight) =
            inventory_totals(inventory, &items.rules).map_err(invalid)?;
        let mut weight = i128::from(weight);
        let mut stacked = BTreeSet::new();
        for (template, count) in additions {
            let definition = items
                .rules
                .parts()
                .items
                .get(template)
                .ok_or(RejectReason::InvalidLoad)?;
            weight = weight
                .saturating_add(i128::from(definition.weight).saturating_mul(i128::from(*count)));
            let slots = if definition.stackable {
                u64::from(stack_of(inventory, *template).is_none() && stacked.insert(*template))
            } else {
                *count
            };
            if definition
                .flags
                .get("is_questitem")
                .copied()
                .unwrap_or(false)
            {
                quest = quest.saturating_add(slots);
            } else {
                ordinary = ordinary.saturating_add(slots);
            }
        }
        if weight > i128::from(player.max_load) {
            return Err(RejectReason::WeightLimit);
        }
        if ordinary > u64::from(player.ordinary_capacity)
            || quest > u64::from(player.quest_capacity)
        {
            return Err(RejectReason::InventoryFull);
        }
        Ok(())
    }

    /// D68/D74 on the receiving stacks of `additions`.
    pub(super) fn caps_for_v7(
        &self,
        owner: EntityId,
        additions: &[(u32, u64)],
    ) -> Result<(), RejectReason> {
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        let mut totals: BTreeMap<u32, u64> = BTreeMap::new();
        for (template, count) in additions {
            let stackable = items
                .rules
                .parts()
                .items
                .get(template)
                .ok_or(RejectReason::InvalidLoad)?
                .stackable;
            if !stackable {
                continue;
            }
            let entry = totals
                .entry(*template)
                .or_insert_with(|| stack_of(&player.inventory, *template).map_or(0, |i| i.count));
            *entry = entry.saturating_add(*count);
            if *entry > count_cap(*template, true) {
                return Err(RejectReason::ItemCountCap);
            }
        }
        Ok(())
    }

    /// `VendorBuy` (design §6.3).
    #[allow(clippy::too_many_lines)] // The pinned check order and the commit stay in one place.
    pub(super) fn vendor_buy(
        &mut self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Phase5Command::VendorBuy {
            merchant,
            list_id,
            items,
            ..
        } = command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        if items.is_empty() || items.len() > MAX_ITEMS || items.iter().any(|i| i.count == 0) {
            return Err(RejectReason::InvalidQuantity);
        }
        let mut seen = BTreeSet::new();
        if !items.iter().all(|i| seen.insert(i.template)) {
            return Err(RejectReason::NotPermitted);
        }
        self.flood_gate_v7(tick, actor, FloodKind::Transaction)?;
        let npc = self.merchant_v7(actor, *merchant)?;
        let phase5 = self.phase5.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let rules = std::sync::Arc::clone(&phase5.rules);
        if rules.buylist(*list_id).is_none_or(|b| b.npc != npc) {
            return Err(RejectReason::InvalidMerchant);
        }
        let castle = rules.castle_percent().ok_or(RejectReason::InvalidLoad)?;
        let item_rules =
            std::sync::Arc::clone(&self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?.rules);
        let mut sub_total: u64 = 0;
        let mut plan = Vec::with_capacity(items.len());
        for item in items {
            let product = rules
                .product(*list_id, item.template)
                .ok_or(RejectReason::NotPermitted)?;
            let definition = item_rules
                .parts()
                .items
                .get(&item.template)
                .ok_or(RejectReason::InvalidLoad)?;
            if !definition.stackable && item.count > 1 {
                return Err(RejectReason::InvalidQuantity);
            }
            // `RateSiegeGuardsPrice` (pinned double 1) for 3960–4026: `⌊fl(price × 1)⌋`.
            let price = product.ref_price;
            if price == 0 {
                return Err(RejectReason::NotPermitted);
            }
            if product.limited {
                let stock = phase5
                    .canonical
                    .stock
                    .get(&(*list_id, item.template))
                    .map_or(product.count, |s| s.count)
                    .max(0);
                if i64::try_from(item.count).map_or(true, |c| c > stock) {
                    return Err(RejectReason::VendorOutOfStock);
                }
            }
            if MAX_ADENA
                .checked_div(item.count)
                .is_none_or(|limit| limit < price)
            {
                return Err(RejectReason::AdenaLimit);
            }
            let unit = rules
                .unit_price(product, castle)
                .ok_or(RejectReason::InvalidLoad)?;
            sub_total = sub_total
                .checked_add(
                    item.count
                        .checked_mul(unit)
                        .ok_or(RejectReason::AdenaLimit)?,
                )
                .ok_or(RejectReason::AdenaLimit)?;
            if sub_total > MAX_ADENA {
                return Err(RejectReason::AdenaLimit);
            }
            plan.push((item.template, item.count, product.clone()));
        }
        let additions: Vec<(u32, u64)> = plan.iter().map(|(t, c, _)| (*t, *c)).collect();
        self.room_for_v7(actor, &additions)?;
        if self.adena_of(actor) < sub_total {
            return Err(RejectReason::InsufficientAdena);
        }
        self.caps_for_v7(actor, &additions)?;
        // Commit.
        let limits = InventoryLimits::from_player(
            self.phase4
                .as_ref()
                .and_then(|items| items.canonical.players.get(&actor))
                .ok_or(RejectReason::InvalidLoad)?,
        );
        let mut edits = vec![InventoryEdit::TakeTemplate {
            template: ADENA,
            count: sub_total,
            preferred: None,
        }];
        edits.extend(
            additions
                .iter()
                .map(|(template, count)| InventoryEdit::Add {
                    template: *template,
                    count: *count,
                }),
        );
        let receipt = format!("receipt:vendor_buy:{}", command.request_key());
        let mut changes = self
            .economy_commit(tick, actor, &edits, limits, events)?
            .into_iter();
        for change in changes.next().unwrap_or_default() {
            self.push_economy_consume(tick, actor, change, &receipt, events)?;
        }
        let mut affected = Vec::new();
        for per_item in changes {
            for change in per_item {
                affected.push(self.push_economy_grant(
                    tick,
                    actor,
                    change,
                    ItemOperation::Purchase,
                    &receipt,
                    events,
                )?);
            }
        }
        // Limited stock (§6.4): decrease, and start the restock timer if none is pending.
        let phase5 = self.phase5.as_mut().ok_or(RejectReason::InvalidLoad)?;
        for (template, count, product) in plan {
            if !product.limited {
                continue;
            }
            let max = u64::try_from(product.count).map_err(invalid)?;
            let restock_ticks = u64::try_from(product.restock_min)
                .map_err(invalid)?
                .saturating_mul(600);
            let stock = phase5
                .canonical
                .stock
                .entry((*list_id, template))
                .or_insert(VendorStock {
                    count: i64::try_from(max).map_err(invalid)?,
                    max,
                    restock_ticks,
                    due: None,
                });
            stock.count = stock
                .count
                .saturating_sub(i64::try_from(count).map_err(invalid)?);
            if stock.due.is_none() {
                stock.due = Some(Tick(tick.0.saturating_add(restock_ticks)));
            }
        }
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::VendorBuy,
            affected,
            EconomyOutcome::Vendor {
                adena_delta_abs: sub_total,
                adena_paid: true,
                refund_order: Vec::new(),
            },
            events,
        );
        Ok(())
    }

    fn refund_order_v7(&self, actor: EntityId) -> Vec<u32> {
        self.phase5
            .as_ref()
            .and_then(|p| p.canonical.players.get(&actor))
            .map(|p| p.refund.iter().map(|i| i.template_id).collect())
            .unwrap_or_default()
    }

    /// Stage a refund-owner row (design §11.1: `owner_kind = refund`, owner = the character).
    #[allow(clippy::too_many_arguments)] // One ledger row's fields.
    fn push_refund_row(
        &mut self,
        actor: EntityId,
        op: LedgerOp,
        before: Option<ItemInstance>,
        after: Option<ItemInstance>,
        transfer: Option<TransferRef>,
        receipt: Option<String>,
        causes: Vec<String>,
    ) -> Result<(), RejectReason> {
        let instance = after
            .as_ref()
            .or(before.as_ref())
            .ok_or(RejectReason::InvalidLoad)?
            .id;
        let count = |i: &Option<ItemInstance>| i64::try_from(i.as_ref().map_or(0, |i| i.count));
        let count_delta = count(&after)
            .map_err(invalid)?
            .checked_sub(count(&before).map_err(invalid)?)
            .ok_or(RejectReason::InvalidLoad)?;
        self.push_ledger(EconomyLedgerRow {
            owner_kind: OwnerKind::Refund,
            owner: actor.as_uuid(),
            instance,
            op,
            operation_ordinal: 0,
            count_delta,
            before,
            after,
            transfer,
            receipt,
            causes,
        });
        Ok(())
    }

    /// `VendorSell` (design §6.5): validated whole, then each item moves into the refund
    /// tab (identity, merge or split; a 13th entry destroys the oldest), then the proceeds.
    #[allow(clippy::too_many_lines)] // The pinned check order and the commit stay in one place.
    pub(super) fn vendor_sell(
        &mut self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Phase5Command::VendorSell {
            merchant,
            list_id,
            items,
            ..
        } = command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        if items.is_empty() || items.len() > MAX_ITEMS || items.iter().any(|i| i.count == 0) {
            return Err(RejectReason::InvalidQuantity);
        }
        let mut seen = BTreeSet::new();
        if !items.iter().all(|i| seen.insert(i.instance)) {
            return Err(RejectReason::NotPermitted);
        }
        self.flood_gate_v7(tick, actor, FloodKind::Transaction)?;
        let npc = self.merchant_v7(actor, *merchant)?;
        let rules =
            std::sync::Arc::clone(&self.phase5.as_ref().ok_or(RejectReason::InvalidLoad)?.rules);
        if rules.buylist(*list_id).is_none_or(|b| b.npc != npc) {
            return Err(RejectReason::InvalidMerchant);
        }
        let item_rules =
            std::sync::Arc::clone(&self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?.rules);
        let player = self
            .phase4
            .as_ref()
            .and_then(|i| i.canonical.players.get(&actor))
            .ok_or(RejectReason::InvalidLoad)?;
        let mut total: u64 = 0;
        let mut sold = Vec::with_capacity(items.len());
        for entry in items {
            // `checkItemManipulation(instance, count, "sell")` then `isSellable()`.
            let item = player
                .inventory
                .get(&entry.instance)
                .ok_or(RejectReason::ItemNotOwned)?;
            let definition = item_rules
                .parts()
                .items
                .get(&item.template_id)
                .ok_or(RejectReason::InvalidLoad)?;
            if (entry.count > 1 && !definition.stackable) || entry.count > item.count {
                return Err(RejectReason::InvalidQuantity);
            }
            if item.location != ItemLocation::Inventory
                || item.augment_id.is_some()
                || !definition
                    .flags
                    .get("is_sellable")
                    .copied()
                    .unwrap_or(false)
            {
                return Err(RejectReason::ItemNotSellable);
            }
            self.guard_item_removal_v6(actor, item)?;
            let price = sell_price(definition.price, 1);
            if MAX_ADENA
                .checked_div(entry.count)
                .is_none_or(|limit| limit < price)
            {
                return Err(RejectReason::AdenaLimit);
            }
            total = total
                .checked_add(sell_price(definition.price, entry.count))
                .ok_or(RejectReason::AdenaLimit)?;
            if total > MAX_ADENA {
                return Err(RejectReason::AdenaLimit);
            }
            sold.push((item.clone(), entry.count, definition.stackable));
        }
        if self.adena_of(actor).saturating_add(total) > MAX_ADENA {
            return Err(RejectReason::AdenaLimit);
        }
        // Refund-tab merge caps (D68 on the receiving tab stack).
        let tab = self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.players.get(&actor))
            .map(|p| p.refund.clone())
            .ok_or(RejectReason::InvalidLoad)?;
        for (item, count, stackable) in &sold {
            if *stackable {
                let held = tab
                    .iter()
                    .find(|t| t.template_id == item.template_id)
                    .map_or(0, |t| t.count);
                if held.saturating_add(*count) > count_cap(item.template_id, true) {
                    return Err(RejectReason::ItemCountCap);
                }
            }
        }
        if total > 0 {
            // The proceeds stack (merge or one fresh slot): an addition that must fit.
            self.room_for_v7(actor, &[(ADENA, total)])?;
        }
        let refund_fresh = sold
            .iter()
            .filter(|(item, count, stackable)| {
                *count < item.count
                    && !(*stackable && tab.iter().any(|t| t.template_id == item.template_id))
            })
            .count();
        let available = self.phase4.as_ref().map_or(0, |items| {
            let (_, next, end) = items.canonical.allocator.bounds();
            end.saturating_sub(next)
        });
        if u64::try_from(refund_fresh)
            .map_err(invalid)?
            .saturating_add(1)
            > available
        {
            return Err(RejectReason::InventoryFull);
        }
        // Commit.
        let limits = InventoryLimits::from_player(player);
        let mut edits: Vec<InventoryEdit> = sold
            .iter()
            .map(|(item, count, _)| InventoryEdit::TakeInstance {
                instance: item.id,
                count: *count,
            })
            .collect();
        if total > 0 {
            edits.push(InventoryEdit::Add {
                template: ADENA,
                count: total,
            });
        }
        let key = command.request_key().to_string();
        let receipt = format!("receipt:vendor_sell:{key}");
        let revision = self.economy_revision(actor, events)?;
        let per_edit = self.economy_commit(tick, actor, &edits, limits, events)?;
        let mut ordinal = 0_u32;
        let mut affected = Vec::new();
        for ((item, count, stackable), changes) in sold.iter().zip(per_edit.iter()) {
            let change = changes.first().cloned().ok_or(RejectReason::InvalidLoad)?;
            affected.push(item.id);
            let link = transfer_link(&key, ordinal);
            ordinal = ordinal.saturating_add(1);
            let whole = change.after.is_none();
            // Where it lands in the tab: merge, identity, or a fresh split instance.
            let tab = &mut self
                .phase5
                .as_mut()
                .and_then(|p| p.canonical.players.get_mut(&actor))
                .ok_or(RejectReason::InvalidLoad)?
                .refund;
            let merge_at = if *stackable {
                tab.iter().position(|t| t.template_id == item.template_id)
            } else {
                None
            };
            let (mode, before_tab, after_tab) = if let Some(at) = merge_at {
                let before = tab.get(at).cloned().ok_or(RejectReason::InvalidLoad)?;
                let mut after = before.clone();
                after.count = after.count.saturating_add(*count);
                after.revision = revision;
                if let Some(slot) = tab.get_mut(at) {
                    *slot = after.clone();
                }
                (TransferMode::Merge, Some(before), after)
            } else {
                let mut after = item.clone();
                after.location = ItemLocation::Refund;
                after.slot = None;
                after.count = *count;
                after.revision = revision;
                let mode = if whole {
                    TransferMode::Identity
                } else {
                    after.id = self
                        .phase4
                        .as_mut()
                        .ok_or(RejectReason::InvalidLoad)?
                        .canonical
                        .allocator
                        .allocate()
                        .map_err(|_| RejectReason::InventoryFull)?;
                    after.creation_origin = Some(ItemCreationOrigin {
                        zone: self.seed.zone,
                        epoch: self.seed.epoch,
                    });
                    after.created_tick = tick;
                    TransferMode::Split
                };
                self.phase5
                    .as_mut()
                    .and_then(|p| p.canonical.players.get_mut(&actor))
                    .ok_or(RejectReason::InvalidLoad)?
                    .refund
                    .push(after.clone());
                (mode, None, after)
            };
            let tab_id = after_tab.id;
            self.push_character_row(
                tick,
                actor,
                CharacterRow {
                    operation: ItemOperation::TransferOut,
                    before: change.before.clone(),
                    after: change.after.clone(),
                    count_delta: i64::try_from(*count).map_err(invalid)?.saturating_neg(),
                    receipt: Some(&receipt),
                    cause: None,
                    transfer: Some(TransferRef {
                        link,
                        mode,
                        peer_owner_kind: OwnerKind::Refund,
                        peer_owner: actor.as_uuid(),
                        peer_instance: tab_id,
                        peer_ordinal: None,
                    }),
                },
                events,
            )?;
            self.push_refund_row(
                actor,
                LedgerOp::TransferIn,
                before_tab,
                Some(after_tab),
                Some(TransferRef {
                    link,
                    mode,
                    peer_owner_kind: OwnerKind::Character,
                    peer_owner: actor.as_uuid(),
                    peer_instance: item.id,
                    peer_ordinal: None,
                }),
                Some(receipt.clone()),
                Vec::new(),
            )?;
            // `PcRefund.addItem`: past 12 entries the oldest is destroyed.
            let overflow = {
                let tab = &mut self
                    .phase5
                    .as_mut()
                    .and_then(|p| p.canonical.players.get_mut(&actor))
                    .ok_or(RejectReason::InvalidLoad)?
                    .refund;
                (tab.len() > REFUND_TAB_LIMIT).then(|| tab.remove(0))
            };
            if let Some(oldest) = overflow {
                self.push_refund_row(
                    actor,
                    LedgerOp::Destroy,
                    Some(oldest),
                    None,
                    None,
                    Some(receipt.clone()),
                    vec![format!("clear_refund:{receipt}")],
                )?;
            }
        }
        if let Some(changes) = per_edit.get(sold.len()) {
            for change in changes.clone() {
                self.push_economy_grant(
                    tick,
                    actor,
                    change,
                    ItemOperation::SaleProceeds,
                    &receipt,
                    events,
                )?;
            }
        }
        let refund_order = self.refund_order_v7(actor);
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::VendorSell,
            affected,
            EconomyOutcome::Vendor {
                adena_delta_abs: total,
                adena_paid: false,
                refund_order,
            },
            events,
        );
        Ok(())
    }

    /// `VendorRefund` (design §6.6): whole entries back at `Σ ⌊count × ref / 2⌋`.
    #[allow(clippy::too_many_lines)] // The pinned check order and the commit stay in one place.
    pub(super) fn vendor_refund(
        &mut self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Phase5Command::VendorRefund {
            merchant,
            list_id,
            refund_indices,
            ..
        } = command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        if refund_indices.is_empty() || refund_indices.len() > REFUND_TAB_LIMIT {
            return Err(RejectReason::InvalidQuantity);
        }
        let mut seen = BTreeSet::new();
        if !refund_indices.iter().all(|i| seen.insert(*i)) {
            return Err(RejectReason::NotPermitted);
        }
        self.flood_gate_v7(tick, actor, FloodKind::Transaction)?;
        let npc = self.merchant_v7(actor, *merchant)?;
        let rules =
            std::sync::Arc::clone(&self.phase5.as_ref().ok_or(RejectReason::InvalidLoad)?.rules);
        if rules.buylist(*list_id).is_none_or(|b| b.npc != npc) {
            return Err(RejectReason::InvalidMerchant);
        }
        let item_rules =
            std::sync::Arc::clone(&self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?.rules);
        let tab = self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.players.get(&actor))
            .map(|p| p.refund.clone())
            .ok_or(RejectReason::InvalidLoad)?;
        let mut price: u64 = 0;
        let mut entries = Vec::with_capacity(refund_indices.len());
        for index in refund_indices {
            let entry = tab
                .get(usize::try_from(*index).map_err(invalid)?)
                .ok_or(RejectReason::NotPermitted)?;
            let definition = item_rules
                .parts()
                .items
                .get(&entry.template_id)
                .ok_or(RejectReason::InvalidLoad)?;
            price = price
                .checked_add(refund_price(definition.price, entry.count))
                .ok_or(RejectReason::AdenaLimit)?;
            entries.push(entry.clone());
        }
        let additions: Vec<(u32, u64)> = entries.iter().map(|e| (e.template_id, e.count)).collect();
        self.room_for_v7(actor, &additions)?;
        if self.adena_of(actor) < price {
            return Err(RejectReason::InsufficientAdena);
        }
        self.caps_for_v7(actor, &additions)?;
        // Commit: pay, then each entry moves back (identity, or merge into the stack).
        let player = self
            .phase4
            .as_ref()
            .and_then(|i| i.canonical.players.get(&actor))
            .ok_or(RejectReason::InvalidLoad)?;
        let limits = InventoryLimits::from_player(player);
        let key = command.request_key().to_string();
        let receipt = format!("receipt:vendor_refund:{key}");
        if price > 0 {
            let changes = self.economy_commit(
                tick,
                actor,
                &[InventoryEdit::TakeTemplate {
                    template: ADENA,
                    count: price,
                    preferred: None,
                }],
                limits,
                events,
            )?;
            for change in changes.into_iter().flatten() {
                self.push_economy_consume(tick, actor, change, &receipt, events)?;
            }
        }
        let revision = self.economy_revision(actor, events)?;
        let mut affected = Vec::new();
        for (ordinal, entry) in entries.iter().enumerate() {
            let link = transfer_link(&key, u32::try_from(ordinal).map_err(invalid)?);
            let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
            let stackable = items
                .rules
                .parts()
                .items
                .get(&entry.template_id)
                .ok_or(RejectReason::InvalidLoad)?
                .stackable;
            let player = items
                .canonical
                .players
                .get_mut(&actor)
                .ok_or(RejectReason::InvalidLoad)?;
            let existing = if stackable {
                stack_of(&player.inventory, entry.template_id).cloned()
            } else {
                None
            };
            let (mode, before, after) = if let Some(before) = existing {
                let mut after = before.clone();
                after.count = after.count.saturating_add(entry.count);
                after.revision = revision;
                (TransferMode::Merge, Some(before), after)
            } else {
                let mut after = entry.clone();
                after.location = ItemLocation::Inventory;
                after.revision = revision;
                (TransferMode::Identity, None, after)
            };
            after.validate().map_err(invalid)?;
            player.inventory.insert(after.id, after.clone());
            player.inventory_revision = revision;
            affected.push(after.id);
            let character_id = after.id;
            self.push_refund_row(
                actor,
                LedgerOp::TransferOut,
                Some(entry.clone()),
                None,
                Some(TransferRef {
                    link,
                    mode,
                    peer_owner_kind: OwnerKind::Character,
                    peer_owner: actor.as_uuid(),
                    peer_instance: character_id,
                    peer_ordinal: None,
                }),
                Some(receipt.clone()),
                Vec::new(),
            )?;
            let count_delta = i64::try_from(entry.count).map_err(invalid)?;
            self.push_character_row(
                tick,
                actor,
                CharacterRow {
                    operation: ItemOperation::TransferIn,
                    before,
                    after: Some(after),
                    count_delta,
                    receipt: Some(&receipt),
                    cause: None,
                    transfer: Some(TransferRef {
                        link,
                        mode,
                        peer_owner_kind: OwnerKind::Refund,
                        peer_owner: actor.as_uuid(),
                        peer_instance: entry.id,
                        peer_ordinal: None,
                    }),
                },
                events,
            )?;
        }
        // The taken entries leave the tab; the rest keep their order.
        let taken: BTreeSet<ItemInstanceId> = entries.iter().map(|e| e.id).collect();
        if let Some(player) = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.players.get_mut(&actor))
        {
            player.refund.retain(|e| !taken.contains(&e.id));
        }
        self.reweigh_v7(actor)?;
        let refund_order = self.refund_order_v7(actor);
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::VendorRefund,
            affected,
            EconomyOutcome::Vendor {
                adena_delta_abs: price,
                adena_paid: true,
                refund_order,
            },
            events,
        );
        Ok(())
    }

    fn reweigh_v7(&mut self, id: EntityId) -> Result<(), RejectReason> {
        let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let rules = std::sync::Arc::clone(&items.rules);
        let player = items
            .canonical
            .players
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        player.weight = inventory_totals(&player.inventory, &rules)
            .map_err(invalid)?
            .2;
        self.rebuild_v5(id).map_err(invalid)
    }

    /// Phase 2 restock (design §6.4): a due product refills to its maximum at once; a full
    /// product leaves the map (absent = full).
    pub(super) fn restock_v7(&mut self, tick: Tick) {
        if let Some(phase5) = &mut self.phase5 {
            phase5
                .canonical
                .stock
                .retain(|_, stock| stock.due.is_none_or(|due| due > tick));
        }
    }

    /// B6: the refund tab is zone-only; at departure every entry is destroyed (refund
    /// `destroy` rows, cause `clear_refund:departure:<tick>`).
    pub(super) fn clear_refund_v7(&mut self, tick: Tick, actor: EntityId) {
        let tab = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.players.get_mut(&actor))
            .map(|p| std::mem::take(&mut p.refund))
            .unwrap_or_default();
        for entry in tab {
            let _ = self.push_refund_row(
                actor,
                LedgerOp::Destroy,
                Some(entry),
                None,
                None,
                None,
                vec![format!("clear_refund:departure:{}", tick.0)],
            );
        }
    }
}

/// `RequestSellItem`: `⌊ref / 2⌋ × count`.
#[must_use]
pub(super) fn sell_price(reference: u64, count: u64) -> u64 {
    (reference / 2).saturating_mul(count)
}

/// `RequestRefundItem`: `⌊count × ref / 2⌋` per whole entry (R19).
#[must_use]
pub(super) fn refund_price(reference: u64, count: u64) -> u64 {
    count.saturating_mul(reference) / 2
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::arithmetic_side_effects,
    clippy::panic
)]
mod tests {
    use super::*;
    use crate::domain::zone::phase5_command::{ItemCount, TemplateCount};
    use crate::domain::zone::{SessionGeneration, Speed, ZoneCommand, ZoneInput};

    use super::super::item_materialize::tests::{fresh_zone_and_kit_load, spawn_kit};
    use super::super::Spawned;

    const P1: u128 = 71;
    const LIST: u64 = 353_630;
    const CHEAP: u32 = 6316; // unlimited, ref 300, tax 10
    const LIMITED: u32 = 16_127; // limited 10, restock 60 min

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    /// A V7 zone with one admitted kit player holding `adena`, and the merchant beside it.
    fn zone(adena: u64) -> (ZoneState, EntityId) {
        let (mut zone, mut load, _) = fresh_zone_and_kit_load(true);
        load.phase5 = Some(crate::domain::zone::Phase5Checkpoint {
            schema_version: 1,
            rules_hash: zone.phase5.as_ref().unwrap().rules.hash_hex(),
            recipes: crate::domain::zone::RecipeBook::default(),
        });
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(id(P1), load)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        // The bound P5-D29 fixture: the player stands on its tile.
        let tiles = (99, 102);
        let here = Vec2Fixed::from_tiles(tiles.0, tiles.1);
        let from = zone.entities[&id(P1)].pos;
        zone.entities.get_mut(&id(P1)).unwrap().pos = here;
        zone.aoi.relocate(id(P1), from, here);
        let merchant = id(5_000);
        zone.spawn(
            Tick(zone.next_tick.0 - 1),
            Spawned {
                id: merchant,
                kind: EntityKind::Npc,
                name: "restock_merchant",
                pos: Vec2Fixed::from_tiles(tiles.0, tiles.1),
                speed: Speed::from_milli_tiles_per_tick(0),
                generation: SessionGeneration::default(),
                combat: None,
            },
        )
        .unwrap();
        give(&mut zone, ADENA, adena);
        (zone, merchant)
    }

    /// Put `count` of `template` in the player's inventory (a fresh stack), as an earlier
    /// grant would have left it.
    fn give(zone: &mut ZoneState, template: u32, count: u64) -> ItemInstanceId {
        let items = zone.phase4.as_mut().unwrap();
        let new = items.canonical.allocator.allocate().unwrap();
        let rules = std::sync::Arc::clone(&items.rules);
        let player = items.canonical.players.get_mut(&id(P1)).unwrap();
        player.inventory_revision += 1;
        player.inventory.insert(
            new,
            ItemInstance {
                creation_origin: Some(ItemCreationOrigin {
                    zone: zone.seed.zone,
                    epoch: zone.seed.epoch,
                }),
                id: new,
                template_id: template,
                count,
                location: ItemLocation::Inventory,
                slot: None,
                enchant: 0,
                augment_id: None,
                charges: 0,
                created_tick: Tick(0),
                revision: player.inventory_revision,
            },
        );
        player.weight = inventory_totals(&player.inventory, &rules).unwrap().2;
        zone.rebuild_v5(id(P1)).unwrap();
        new
    }

    fn held(zone: &ZoneState, template: u32) -> u64 {
        zone.phase4.as_ref().unwrap().canonical.players[&id(P1)]
            .inventory
            .values()
            .filter(|i| i.template_id == template)
            .map(|i| i.count)
            .sum()
    }

    fn run(
        zone: &mut ZoneState,
        key: u128,
        kind: serde_json::Value,
    ) -> crate::domain::zone::AppliedTick {
        let mut value = kind;
        value["entity"] = serde_json::json!(id(P1));
        value["request_key"] = serde_json::json!(uuid::Uuid::from_u128(key));
        let command: Phase5Command = serde_json::from_value(value).unwrap();
        zone.run_tick(zone.draft(vec![ZoneInput::session(
            id(P1),
            SessionGeneration(1),
            u32::try_from(key).unwrap(),
            ZoneCommand::Phase5(Box::new(command)),
        )]))
        .unwrap()
    }

    fn buy(
        zone: &mut ZoneState,
        key: u128,
        merchant: EntityId,
        items: &[(u32, u64)],
    ) -> Vec<RejectReason> {
        let items: Vec<TemplateCount> = items
            .iter()
            .map(|(template, count)| TemplateCount {
                template: *template,
                count: *count,
            })
            .collect();
        let tick = run(
            zone,
            key,
            serde_json::json!({"kind": "vendor_buy", "merchant": merchant, "list_id": LIST, "items": items}),
        );
        tick.dispositions.iter().map(|d| d.reason).collect()
    }

    fn wait(zone: &mut ZoneState, ticks: u64) {
        for _ in 0..ticks {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
    }

    fn unit(zone: &ZoneState, template: u32) -> u64 {
        let rules = &zone.phase5.as_ref().unwrap().rules;
        rules
            .unit_price(rules.product(LIST, template).unwrap(), 0)
            .unwrap()
    }

    /// §6.3: a buy consumes `count × unit` adena (one economy Consume cause) and grants a
    /// `purchase` row; the receipt reports the paid total.
    #[test]
    fn vendor_buy_pays_the_taxed_unit_price_and_grants_a_purchase() {
        let (mut zone, merchant) = zone(10_000);
        let cost = 3 * unit(&zone, CHEAP);
        assert_eq!(unit(&zone, CHEAP), 330, "fl(300 × fl(1 + 0.1)) = 330");
        let before = held(&zone, ADENA);
        let items: Vec<TemplateCount> = vec![TemplateCount {
            template: CHEAP,
            count: 3,
        }];
        let tick = run(
            &mut zone,
            1,
            serde_json::json!({"kind": "vendor_buy", "merchant": merchant, "list_id": LIST, "items": items}),
        );
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(held(&zone, ADENA), before - cost);
        assert_eq!(held(&zone, CHEAP), 3);
        let rows: Vec<_> = tick
            .events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == id(P1) => Some(p.ledger.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(rows.iter().any(|r| r.operation == ItemOperation::Purchase));
        let consume = rows
            .iter()
            .find(|r| r.operation == ItemOperation::Consume)
            .unwrap();
        assert_eq!(consume.count_delta, -i64::try_from(cost).unwrap());
        assert_eq!(consume.causes[0].family, "economy");
        assert!(tick.events.iter().any(|e| matches!(
            e,
            ZoneEvent::Economy(ev) if matches!(&ev.body,
                crate::domain::zone::EconomyBody::EconomyResult {
                    outcome: EconomyOutcome::Vendor { adena_delta_abs, adena_paid: true, .. }, ..
                } if *adena_delta_abs == cost)
        )));
    }

    /// §6.1/§6.3 refusals in pinned order; the flood gate survives a later refusal (§2.4).
    #[test]
    fn vendor_buy_refusals_and_the_transaction_flood_gate() {
        let (mut zone, merchant) = zone(100);
        assert_eq!(
            buy(&mut zone, 1, merchant, &[(CHEAP, 1), (CHEAP, 1)]),
            vec![RejectReason::NotPermitted]
        );
        // Passes the gate, then lacks adena: the gate is still set.
        assert_eq!(
            buy(&mut zone, 2, merchant, &[(CHEAP, 1)]),
            vec![RejectReason::InsufficientAdena]
        );
        assert_eq!(buy(&mut zone, 3, merchant, &[(CHEAP, 1)]), vec![RejectReason::RateLimited]);
        wait(&mut zone, 10);
        assert_eq!(
            buy(&mut zone, 4, merchant, &[(7015, 2)]),
            vec![RejectReason::InvalidQuantity],
            "non-stackable count 2"
        );
        wait(&mut zone, 10);
        assert_eq!(buy(&mut zone, 5, id(P1), &[(CHEAP, 1)]), vec![RejectReason::InvalidMerchant]);
        wait(&mut zone, 10);
        assert_eq!(
            buy(&mut zone, 6, merchant, &[(57, 1)]),
            vec![RejectReason::NotPermitted],
            "not a product"
        );
        wait(&mut zone, 10);
        // Out of range: the player stands 4,688 milli-tiles away (m(150) = 4,687).
        let from = zone.entities[&id(P1)].pos;
        let far =
            Vec2Fixed::new(crate::domain::zone::Fixed::from_raw(from.x.raw() + 4_688), from.y);
        zone.entities.get_mut(&id(P1)).unwrap().pos = far;
        zone.aoi.relocate(id(P1), from, far);
        assert_eq!(buy(&mut zone, 7, merchant, &[(CHEAP, 1)]), vec![RejectReason::OutOfRange]);
    }

    /// §6.4: limited stock decreases and starts one restock timer; a request over the stock
    /// refuses; the due phase-2 tick refills it (V-R: `due = tick + minutes × 600`).
    #[test]
    fn limited_stock_runs_out_and_restocks_on_its_due_tick() {
        let (mut zone, merchant) = zone(10_000_000);
        let tick = zone.next_tick;
        assert!(buy(&mut zone, 1, merchant, &[(LIMITED, 10)]).is_empty());
        let stock = zone.phase5.as_ref().unwrap().canonical.stock[&(LIST, LIMITED)];
        assert_eq!((stock.count, stock.max), (0, 10));
        assert_eq!(stock.due, Some(Tick(tick.0 + 60 * 600)));
        wait(&mut zone, 10);
        assert_eq!(
            buy(&mut zone, 2, merchant, &[(LIMITED, 1)]),
            vec![RejectReason::VendorOutOfStock]
        );
        // Jump to just before the due tick, then across it.
        let due = stock.due.unwrap();
        zone.phase5
            .as_mut()
            .unwrap()
            .canonical
            .stock
            .get_mut(&(LIST, LIMITED))
            .unwrap()
            .due = Some(Tick(zone.next_tick.0 + 2));
        wait(&mut zone, 1);
        assert!(zone
            .phase5
            .as_ref()
            .unwrap()
            .canonical
            .stock
            .contains_key(&(LIST, LIMITED)));
        wait(&mut zone, 2);
        assert!(
            !zone
                .phase5
                .as_ref()
                .unwrap()
                .canonical
                .stock
                .contains_key(&(LIST, LIMITED)),
            "refilled"
        );
        let _ = due;
        wait(&mut zone, 10);
        assert!(buy(&mut zone, 3, merchant, &[(LIMITED, 1)]).is_empty());
    }

    /// §6.5–6.6: sell into the refund tab (proceeds `⌊ref/2⌋ × count`), a 13th entry destroys
    /// the oldest, and a refund buys an entry back at `⌊count × ref / 2⌋`.
    #[test]
    fn sell_fills_the_refund_tab_and_refund_buys_back() {
        let (mut zone, merchant) = zone(1_000_000);
        let rules = std::sync::Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let price = |t: u32| rules.parts().items[&t].price;
        // Thirteen distinct sellable non-stackable templates.
        let templates: Vec<u32> = rules
            .parts()
            .items
            .values()
            .filter(|d| {
                !d.stackable
                    && d.price > 1
                    && d.flags.get("is_sellable") == Some(&true)
                    && rules
                        .parts()
                        .capabilities
                        .get(&d.template_id)
                        .is_some_and(|c| c.storable)
                    && d.kind == "EtcItem"
            })
            .map(|d| d.template_id)
            .take(13)
            .collect();
        assert_eq!(templates.len(), 13);
        let mut ids = Vec::new();
        for t in &templates {
            ids.push(give(&mut zone, *t, 1));
        }
        let before = held(&zone, ADENA);
        let mut key = 10;
        for (t, instance) in templates.iter().zip(&ids) {
            let items = vec![ItemCount {
                instance: *instance,
                count: 1,
            }];
            let tick = run(
                &mut zone,
                key,
                serde_json::json!({"kind": "vendor_sell", "merchant": merchant, "list_id": LIST, "items": items}),
            );
            assert!(tick.dispositions.is_empty(), "{t}: {:?}", tick.dispositions);
            key += 1;
            wait(&mut zone, 10);
        }
        let proceeds: u64 = templates.iter().map(|t| sell_price(price(*t), 1)).sum();
        assert_eq!(held(&zone, ADENA), before + proceeds);
        let tab = zone.phase5.as_ref().unwrap().canonical.players[&id(P1)]
            .refund
            .clone();
        assert_eq!(tab.len(), 12, "the oldest was destroyed");
        assert_eq!(tab[0].template_id, templates[1]);
        assert!(tab.iter().all(|i| i.location == ItemLocation::Refund));
        // Refund the newest entry (index 11).
        let paid = refund_price(price(templates[12]), 1);
        let before = held(&zone, ADENA);
        let tick = run(
            &mut zone,
            key,
            serde_json::json!({"kind": "vendor_refund", "merchant": merchant, "list_id": LIST, "refund_indices": [11]}),
        );
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(held(&zone, ADENA), before - paid);
        assert_eq!(held(&zone, templates[12]), 1);
        assert_eq!(
            zone.phase5.as_ref().unwrap().canonical.players[&id(P1)]
                .refund
                .len(),
            11
        );
        // Refund-owner rows ride the economy checkpoint, linked to the character rows.
        let rows: Vec<_> = tick
            .events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::EconomyCheckpoint(c) => Some(c.ledger.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(rows
            .iter()
            .any(|r| r.owner_kind == OwnerKind::Refund && r.op == LedgerOp::TransferOut));
        // B6: departure destroys the rest.
        zone.clear_refund_v7(zone.next_tick, id(P1));
        assert!(zone.phase5.as_ref().unwrap().canonical.players[&id(P1)]
            .refund
            .is_empty());
        let staged = &zone.phase5.as_ref().unwrap().ledger;
        assert_eq!(staged.iter().filter(|r| r.op == LedgerOp::Destroy).count(), 11);
    }

    /// Families V-S (sell/refund prices), V-R (restock minutes) and V-P (unit prices that
    /// differ from naive truncation) on the oracle inputs.
    #[test]
    fn vendor_formulas_match_the_v_families() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-vendor.json"
        ))
        .unwrap();
        for v in fixture["vs_vectors"]["prices"].as_array().unwrap() {
            let (p, c) = (v["p"].as_u64().unwrap(), v["c"].as_u64().unwrap());
            assert_eq!(sell_price(p, c), v["sell"].as_u64().unwrap(), "{v}");
            assert_eq!(refund_price(p, c), v["refund"].as_u64().unwrap(), "{v}");
        }
        let rules = crate::infrastructure::economy_data::embedded_phase5_rules().unwrap();
        let entries = &rules.parts().merchants.entries;
        for v in fixture["vr_vectors"].as_array().unwrap() {
            let minutes = v["minutes"].as_i64().unwrap();
            let delay = v["delay"].as_str().unwrap();
            assert!(
                entries
                    .iter()
                    .any(|e| e.limited && e.delay == delay && e.restock_min == minutes),
                "{v}"
            );
            assert_eq!(
                v["decrease_at"].as_i64().unwrap() + minutes * 600,
                v["due"].as_i64().unwrap()
            );
        }
        for v in fixture["vp_naive_differences"].as_array().unwrap() {
            let (item, tax) =
                (u32::try_from(v[0].as_u64().unwrap()).unwrap(), v[1].as_str().unwrap());
            let entry = entries
                .iter()
                .find(|e| e.item == item && e.tax == tax)
                .unwrap();
            assert_eq!(entry.ref_price, v[2].as_u64().unwrap());
            assert_eq!(rules.unit_price(entry, 0), Some(v[3].as_u64().unwrap()), "{v}");
        }
    }
}
