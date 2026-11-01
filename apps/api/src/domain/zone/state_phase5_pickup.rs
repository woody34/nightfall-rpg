//! V7 phase-0 economy intents, part two (story 3.5b): `PickUpItem` (design §4.1, pinned
//! `L2PcInstance.doPickupItem`, `L2PcInstance.java:3779-3912`), party distribution
//! (§4.2, `L2Party.distributeItem` / `getActualLooter` / `distributeItemEvenly`) and the
//! owner-kind ledger of §11.2–§11.3 with its tick flush into `EconomyCheckpoint` (P5-Q13).
//! Every check runs before any mutation: a refusal changes nothing.

use std::collections::BTreeMap;

use crate::domain::zone::item_inventory::{count_cap, inventory_totals};
use crate::domain::zone::phase5::GroundItem;
use crate::domain::zone::phase5_command::LootMode;
use crate::domain::zone::phase5_event::{
    transfer_link, EconomyBody, EconomyCheckpoint, EconomyEvent, EconomyLedgerRow, EconomyOutcome,
    EconomyScope, LedgerOp, OwnerKind, TransferMode, TransferRef,
};
use crate::domain::zone::{
    CombatRole, EntityId, EntityKind, ItemInstance, ItemInstanceId, ItemLocation, Phase4Projection,
    Phase4Snapshot, Phase5Command, RejectReason, Tick, ZoneEvent, ZoneState,
};

/// `maybeMoveToPawn(target, 36)` in milli-tiles (`m(36) = 1,125`, design conventions), plus
/// the engine's player collision radius 0 (adaptation X13).
const PICKUP_RANGE_MILLI: i64 = 1_125;

/// One recipient's payout of a pickup.
struct Payout {
    recipient: EntityId,
    count: u64,
}

/// Where a received count lands in a recipient's inventory.
enum Landing {
    /// The ground instance itself, unchanged id (identity transfer).
    Identity,
    /// An existing stack of the template (merge).
    Merge(ItemInstanceId),
    /// A freshly allocated instance (even split).
    Fresh,
}

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

fn square(x: i64) -> i64 {
    x.saturating_mul(x)
}

fn dist_sq(a: crate::domain::zone::Vec2Fixed, b: crate::domain::zone::Vec2Fixed) -> i64 {
    let dx = i64::from(a.x.raw()).saturating_sub(i64::from(b.x.raw()));
    let dy = i64::from(a.y.raw()).saturating_sub(i64::from(b.y.raw()));
    square(dx).saturating_add(square(dy))
}

/// The stack an addition of `template` merges into (`ItemContainer.addItem`): the same
/// stackable template with no enchant, augment or charges.
fn merge_target(
    inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
    template: u32,
) -> Option<ItemInstanceId> {
    inventory
        .values()
        .find(|item| {
            item.template_id == template
                && item.enchant == 0
                && item.augment_id.is_none()
                && item.charges == 0
        })
        .map(|item| item.id)
}

impl ZoneState {
    /// `PickUpItemRequest{ground_entity}` (design §4.1): checks in pinned order, then the
    /// ground entity is removed and the item goes to the picker, a party looter or an even
    /// split. Herbs are story 3.5c.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // Pinned order, one place.
    pub(super) fn pick_up_item(
        &mut self,
        tick: Tick,
        actor: EntityId,
        ground_entity: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        before_items: Option<&Phase4Snapshot>,
        admitted: &BTreeMap<EntityId, u64>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        // 1. The actor is a live player (`isAlikeDead`).
        let picker = self
            .entities
            .get(&actor)
            .ok_or(RejectReason::UnknownEntity)?;
        if picker.kind != EntityKind::Player {
            return Err(RejectReason::NotAPlayer);
        }
        if picker.targeting.dead {
            return Err(RejectReason::DeadActor);
        }
        let picker_pos = picker.pos;
        // 2. The target is a live ground entity with its record.
        let phase5 = self
            .phase5
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?;
        let ground = phase5
            .canonical
            .ground
            .get(&ground_entity)
            .cloned()
            .filter(|_| {
                self.entities
                    .get(&ground_entity)
                    .is_some_and(|e| e.kind == EntityKind::GroundItem)
            })
            .ok_or(RejectReason::GroundItemNotFound)?;
        // 3. Range: 2D, non-strict (`isInsideRadius(target, 36, false, false)`).
        if dist_sq(picker_pos, ground.pos) > square(PICKUP_RANGE_MILLI) {
            return Err(RejectReason::OutOfRange);
        }
        let rules = std::sync::Arc::clone(&phase5.rules);
        let template = ground.item.template_id;
        if rules.is_herb(template) {
            // Story 3.5c: herb skills execute on pickup (§4.3).
            return Err(RejectReason::NotYetImplemented);
        }
        let party = self
            .party_of_player(actor)
            .and_then(|id| phase5.canonical.parties.get(&id))
            .cloned();
        // 4a. DropProtection (`tryPickUp`): unprotected, the owner, or the owner's party.
        // z-review-3 P3-5: both protections are live while `tick < until` (they clear in
        // phase 2 at `until <= tick`; a phase-0 pickup at `until` already passes).
        if let Some(protect) = ground.drop_protection.filter(|p| tick < p.until) {
            if protect.owner != actor && !self.same_party(protect.owner, actor) {
                return Err(RejectReason::GroundItemProtected);
            }
        }
        let items = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?;
        let definition = items
            .rules
            .parts()
            .items
            .get(&template)
            .ok_or(RejectReason::InvalidLoad)?;
        let stackable = definition.stackable;
        let quest = definition
            .flags
            .get("is_questitem")
            .copied()
            .ok_or(RejectReason::InvalidLoad)?;
        // 4b. Capacity, solo or FINDERS_KEEPERS only: slots, not weight (rule P7).
        if party
            .as_ref()
            .is_none_or(|p| p.distribution == LootMode::FindersKeepers)
        {
            let player = items
                .canonical
                .players
                .get(&actor)
                .ok_or(RejectReason::InvalidLoad)?;
            let needs_slot = !stackable || merge_target(&player.inventory, template).is_none();
            if needs_slot {
                let (ordinary, quest_used, _) =
                    inventory_totals(&player.inventory, &items.rules).map_err(invalid)?;
                let (used, limit) = if quest {
                    (quest_used, player.quest_capacity)
                } else {
                    (ordinary, player.ordinary_capacity)
                };
                if used.checked_add(1).is_none_or(|n| n > u64::from(limit)) {
                    return Err(RejectReason::InventoryFull);
                }
            }
        }
        // 4c. Loot owner: owner ≠ 0 ∧ owner ≠ picker ∧ ¬isInLooterParty(owner), which needs
        // the owner online (in the zone) and in the picker's party.
        if let Some(owner) = ground.loot_owner.filter(|o| tick < o.until) {
            let online = self
                .entities
                .get(&owner.owner)
                .is_some_and(|e| e.kind == EntityKind::Player);
            if owner.owner != actor && !(online && self.same_party(owner.owner, actor)) {
                return Err(RejectReason::GroundItemProtected);
            }
        }
        // Distribution plan (`L2Party.distributeItem`, §4.2).
        let count = ground.item.count;
        let even = party.as_ref().is_some_and(|_| {
            let even = &rules.parts().party.party.even_distribute;
            count > 1
                && (even.items.contains(&template)
                    || (stackable && even.all_stackable_items)
                    || (!stackable && even.all_other_items))
        });
        let range2 = i64::try_from(rules.parts().party.party.party_range2)
            .map_err(invalid)?
            .saturating_mul(1000)
            / 32;
        let in_range = |zone: &Self, member: EntityId| {
            zone.entities.get(&member).is_some_and(|e| {
                // `checkIfInRange(PartyRange2, picker, member, true)` with radii 0 + 0.
                dist_sq(picker_pos, e.pos).saturating_mul(4) <= square(range2.saturating_mul(2))
            })
        };
        let mut payouts = Vec::new();
        let mut by_turn_cursor = None;
        let mode = party
            .as_ref()
            .map_or(LootMode::FindersKeepers, |p| p.distribution);
        if even {
            let members = party
                .as_ref()
                .map(|p| p.members.clone())
                .unwrap_or_default();
            let recipients: Vec<EntityId> =
                members.into_iter().filter(|m| in_range(self, *m)).collect();
            let k = u64::try_from(recipients.len()).map_err(invalid)?;
            if k == 0 {
                return Err(RejectReason::InvalidLoad);
            }
            let each = count.checked_div(k).ok_or(RejectReason::InvalidLoad)?;
            let leftover = count.checked_rem(k).ok_or(RejectReason::InvalidLoad)?;
            let mut shares: Vec<u64> = vec![each; recipients.len()];
            // `leftOver` draws with replacement (draw #9), then the per-recipient caps; a
            // refusal rewinds the draws, so it changes nothing.
            let rewind = (self.rng.clone(), self.evidence.clone());
            for _ in 0..leftover {
                let before = self.rng.get_word_pos();
                let chosen = self.roll_below(u32::try_from(k).map_err(invalid)?);
                self.label_raw_draw(before, "party_split_remainder");
                let slot = shares
                    .get_mut(usize::try_from(chosen).map_err(invalid)?)
                    .ok_or(RejectReason::InvalidLoad)?;
                *slot = slot.saturating_add(1);
            }
            for (recipient, share) in recipients.iter().zip(&shares) {
                if *share > 0 {
                    if let Err(reason) = self.check_receive_cap(*recipient, template, *share) {
                        (self.rng, self.evidence) = rewind;
                        return Err(reason);
                    }
                }
            }
            for (recipient, share) in recipients.into_iter().zip(shares) {
                if share > 0 {
                    payouts.push(Payout {
                        recipient,
                        count: share,
                    });
                }
            }
        } else {
            let mut rewind = None;
            let recipient = match (&party, mode) {
                (None, _) | (Some(_), LootMode::FindersKeepers) => actor,
                (Some(party), LootMode::Random | LootMode::RandomIncludingSpoil) => {
                    let candidates: Vec<EntityId> = party
                        .members
                        .iter()
                        .copied()
                        .filter(|m| in_range(self, *m))
                        .collect();
                    if candidates.is_empty() {
                        actor
                    } else {
                        // Draw #8; the looter's cap is checked after it and a refusal
                        // rewinds the draw.
                        rewind = Some((self.rng.clone(), self.evidence.clone()));
                        let before = self.rng.get_word_pos();
                        let index =
                            self.roll_below(u32::try_from(candidates.len()).map_err(invalid)?);
                        self.label_raw_draw(before, "party_random");
                        candidates
                            .get(usize::try_from(index).map_err(invalid)?)
                            .copied()
                            .ok_or(RejectReason::InvalidLoad)?
                    }
                },
                (Some(party), LootMode::ByTurn | LootMode::ByTurnIncludingSpoil) => {
                    // `getCheckedNextLooter`: pre-increment, wrap, test; a full lap with no
                    // winner leaves the cursor advanced by `len` and returns the picker.
                    let len = u32::try_from(party.members.len()).map_err(invalid)?;
                    let mut cursor = party.item_last_loot;
                    let mut winner = None;
                    for _ in 0..len {
                        cursor = cursor.saturating_add(1);
                        if cursor >= len {
                            cursor = 0;
                        }
                        let member = party
                            .members
                            .get(usize::try_from(cursor).map_err(invalid)?)
                            .copied()
                            .ok_or(RejectReason::InvalidLoad)?;
                        if in_range(self, member) {
                            winner = Some(member);
                            break;
                        }
                    }
                    by_turn_cursor = Some((party.id, cursor));
                    winner.unwrap_or(actor)
                },
            };
            let checked = if self.phase4_player_exists(recipient) {
                self.check_receive_cap(recipient, template, count)
            } else {
                Err(RejectReason::InvalidLoad)
            };
            if let Err(reason) = checked {
                if let Some(saved) = rewind {
                    (self.rng, self.evidence) = saved;
                }
                return Err(reason);
            }
            payouts.push(Payout { recipient, count });
        }
        // ---- commit: nothing below refuses on valid state.
        if let Some((id, cursor)) = by_turn_cursor {
            if let Some(party) = self
                .phase5
                .as_mut()
                .and_then(|p| p.canonical.parties.get_mut(&id))
            {
                party.item_last_loot = cursor;
            }
        }
        let resolution = command.request_key().to_string();
        let receipt = format!("receipt:pick_up_item:{resolution}");
        if let Some(phase5) = &mut self.phase5 {
            phase5.canonical.ground.remove(&ground_entity);
        }
        events.extend(self.despawn(tick, ground_entity));
        let ground_owner = ground_entity.as_uuid();
        let mut affected = Vec::new();
        let mut ordinal = 0_u32;
        if even {
            self.push_ledger(EconomyLedgerRow {
                owner_kind: OwnerKind::Ground,
                owner: ground_owner,
                instance: ground.item.id,
                op: LedgerOp::Destroy,
                operation_ordinal: 0,
                count_delta: i64::try_from(count).map_err(invalid)?.saturating_neg(),
                before: Some(ground.item.clone()),
                after: None,
                transfer: None,
                receipt: Some(receipt.clone()),
                causes: vec![format!("party_split:{resolution}")],
            });
        }
        for payout in &payouts {
            let landing = {
                let player = self
                    .phase4
                    .as_ref()
                    .and_then(|items| items.canonical.players.get(&payout.recipient))
                    .ok_or(RejectReason::InvalidLoad)?;
                match (stackable, merge_target(&player.inventory, template)) {
                    (true, Some(existing)) => Landing::Merge(existing),
                    _ if even => Landing::Fresh,
                    _ => Landing::Identity,
                }
            };
            let initial =
                super::runtime::tick_start_revision(before_items, admitted, payout.recipient);
            let (before, after) = self.receive(
                tick,
                payout.recipient,
                &ground.item,
                payout.count,
                &landing,
                initial,
            )?;
            affected.push(after.id);
            let link = transfer_link(&resolution, ordinal);
            let mode = match landing {
                Landing::Identity => TransferMode::Identity,
                Landing::Merge(_) => TransferMode::Merge,
                Landing::Fresh => TransferMode::Split,
            };
            let count_delta = i64::try_from(payout.count).map_err(invalid)?;
            if even {
                self.push_ledger(EconomyLedgerRow {
                    owner_kind: OwnerKind::Character,
                    owner: payout.recipient.as_uuid(),
                    instance: after.id,
                    op: LedgerOp::PartySplit,
                    operation_ordinal: 0,
                    count_delta,
                    before,
                    after: Some(after),
                    transfer: None,
                    receipt: Some(receipt.clone()),
                    causes: vec![format!("party_split:{resolution}")],
                });
            } else {
                let causes = party
                    .as_ref()
                    .map(|_| vec![format!("party:{resolution}")])
                    .unwrap_or_default();
                self.push_ledger(EconomyLedgerRow {
                    owner_kind: OwnerKind::Ground,
                    owner: ground_owner,
                    instance: ground.item.id,
                    op: LedgerOp::TransferOut,
                    operation_ordinal: 0,
                    count_delta: count_delta.saturating_neg(),
                    before: Some(ground.item.clone()),
                    after: None,
                    transfer: Some(TransferRef {
                        link,
                        mode,
                        peer_owner_kind: OwnerKind::Character,
                        peer_owner: payout.recipient.as_uuid(),
                        peer_instance: after.id,
                    }),
                    receipt: Some(receipt.clone()),
                    causes: causes.clone(),
                });
                self.push_ledger(EconomyLedgerRow {
                    owner_kind: OwnerKind::Character,
                    owner: payout.recipient.as_uuid(),
                    instance: after.id,
                    op: LedgerOp::TransferIn,
                    operation_ordinal: 0,
                    count_delta,
                    before,
                    after: Some(after),
                    transfer: Some(TransferRef {
                        link,
                        mode,
                        peer_owner_kind: OwnerKind::Ground,
                        peer_owner: ground_owner,
                        peer_instance: ground.item.id,
                    }),
                    receipt: Some(receipt.clone()),
                    causes,
                });
            }
            ordinal = ordinal.saturating_add(1);
        }
        // `LootDistributed` for a non-split party pickup: every member except the recipient.
        if let (Some(party), false) = (&party, even) {
            if let Some(payout) = payouts.first() {
                let audience: Vec<EntityId> = party
                    .members
                    .iter()
                    .copied()
                    .filter(|m| *m != payout.recipient)
                    .collect();
                events.push(ZoneEvent::Economy(Box::new(EconomyEvent {
                    tick,
                    entity: actor,
                    body: EconomyBody::LootDistributed {
                        recipient: payout.recipient,
                        template_id: template,
                        count,
                        mode: party.distribution,
                        spoil: false,
                    },
                    audience,
                })));
            }
        }
        let recipient = if even {
            actor
        } else {
            payouts.first().map_or(actor, |p| p.recipient)
        };
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::PickUpItem,
            affected,
            EconomyOutcome::Pickup {
                template_id: template,
                count,
                recipient,
                herb: false,
                split: even,
            },
            events,
        );
        Ok(())
    }

    fn party_of_player(&self, id: EntityId) -> Option<crate::domain::zone::phase5::PartyId> {
        self.phase5
            .as_ref()
            .and_then(|p| p.canonical.players.get(&id))
            .and_then(|p| p.party)
    }

    fn same_party(&self, a: EntityId, b: EntityId) -> bool {
        self.party_of_player(a)
            .is_some_and(|party| self.party_of_player(b) == Some(party))
    }

    fn phase4_player_exists(&self, id: EntityId) -> bool {
        self.phase4
            .as_ref()
            .is_some_and(|items| items.canonical.players.contains_key(&id))
    }

    /// D68/D74 on the receiving stack: a pickup that would exceed a cap is refused before
    /// any change (`ITEM_COUNT_CAP`).
    fn check_receive_cap(
        &self,
        recipient: EntityId,
        template: u32,
        count: u64,
    ) -> Result<(), RejectReason> {
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&recipient)
            .ok_or(RejectReason::InvalidLoad)?;
        let stackable = items
            .rules
            .parts()
            .items
            .get(&template)
            .ok_or(RejectReason::InvalidLoad)?
            .stackable;
        let existing = if stackable {
            merge_target(&player.inventory, template)
                .and_then(|id| player.inventory.get(&id))
                .map_or(0, |item| item.count)
        } else {
            0
        };
        if existing
            .checked_add(count)
            .is_none_or(|total| total > count_cap(template, stackable))
        {
            return Err(RejectReason::ItemCountCap);
        }
        Ok(())
    }

    /// Put `count` of the ground instance into `recipient`'s inventory (identity move, merge
    /// or a fresh split instance), step its revision once per owner-tick (D79) and make sure
    /// the owner's tick projection exists (P5-Q13: the projection carries the scalars, the
    /// ledger rows go to the economy checkpoint). Returns the row's `(before, after)`.
    #[allow(clippy::too_many_arguments)] // One payout's explicit inputs.
    fn receive(
        &mut self,
        tick: Tick,
        recipient: EntityId,
        source: &ItemInstance,
        count: u64,
        landing: &Landing,
        initial: Option<u64>,
    ) -> Result<(Option<ItemInstance>, ItemInstance), RejectReason> {
        let origin = crate::domain::zone::ItemCreationOrigin {
            zone: self.seed.zone,
            epoch: self.seed.epoch,
        };
        let items = &mut self
            .phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        let fresh_id = match landing {
            Landing::Fresh => Some(items.allocator.allocate().map_err(invalid)?),
            Landing::Identity | Landing::Merge(_) => None,
        };
        let player = items
            .players
            .get_mut(&recipient)
            .ok_or(RejectReason::InvalidLoad)?;
        let changed = initial.is_some_and(|initial| initial != player.inventory_revision);
        let revision = if changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let (before, after) = match landing {
            Landing::Identity => {
                let mut after = source.clone();
                after.location = ItemLocation::Inventory;
                after.slot = None;
                after.revision = revision;
                (None, after)
            },
            Landing::Merge(existing) => {
                let before = player
                    .inventory
                    .get(existing)
                    .cloned()
                    .ok_or(RejectReason::InvalidLoad)?;
                let mut after = before.clone();
                after.count = after
                    .count
                    .checked_add(count)
                    .ok_or(RejectReason::ItemCountCap)?;
                after.revision = revision;
                (Some(before), after)
            },
            Landing::Fresh => {
                let id = fresh_id.ok_or(RejectReason::InvalidLoad)?;
                (
                    None,
                    ItemInstance {
                        creation_origin: Some(origin),
                        id,
                        template_id: source.template_id,
                        count,
                        location: ItemLocation::Inventory,
                        slot: None,
                        enchant: 0,
                        augment_id: None,
                        charges: 0,
                        created_tick: tick,
                        revision,
                    },
                )
            },
        };
        after.validate().map_err(invalid)?;
        player.inventory.insert(after.id, after.clone());
        player.inventory_revision = revision;
        self.reweigh(recipient)?;
        self.rebuild_v5(recipient).map_err(invalid)?;
        Ok((before, after))
    }

    /// The owner's weight after an economy change (`InventoryMutation` keeps the same fold).
    fn reweigh(&mut self, id: EntityId) -> Result<(), RejectReason> {
        let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let rules = std::sync::Arc::clone(&items.rules);
        let player = items
            .canonical
            .players
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let (_, _, weight) = inventory_totals(&player.inventory, &rules).map_err(invalid)?;
        player.weight = weight;
        Ok(())
    }

    /// Ensure every live owner changed by an economy operation this tick has its
    /// `ItemCheckpoint` projection (refreshed at the tick boundary by `freeze_items_v6`).
    pub(super) fn ensure_economy_projections(
        &self,
        tick: Tick,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Some(phase5) = &self.phase5 else {
            return Ok(());
        };
        let owners: std::collections::BTreeSet<EntityId> = phase5
            .ledger
            .iter()
            .filter(|row| row.owner_kind == OwnerKind::Character)
            .map(|row| EntityId::from_uuid(row.owner))
            .collect();
        for id in owners {
            if events
                .iter()
                .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(p) if p.entity == id))
            {
                continue;
            }
            let Some(CombatRole::Player {
                progression: Some(progression),
                ..
            }) = self
                .entities
                .get(&id)
                .and_then(|e| e.combat.as_ref())
                .map(|c| &c.role)
            else {
                return Err(RejectReason::InvalidLoad);
            };
            let phase = &self
                .phase3
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical;
            let checkpoint = self
                .phase4
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical
                .checkpoint(id, tick, phase)
                .map_err(invalid)?;
            events.push(ZoneEvent::ItemCheckpoint(Box::new(Phase4Projection {
                enchant_attempts: Vec::new(),
                exchanges: Vec::new(),
                refines: Vec::new(),
                paused_core: None,
                tick,
                entity: id,
                account: progression.identity.account_id,
                checkpoint,
                ledger: Vec::new(),
                receipts: Vec::new(),
                materializations: Vec::new(),
                evidence: Vec::new(),
            })));
        }
        Ok(())
    }

    /// Stage one owner-kind ledger row; ordinals are assigned at the tick flush.
    pub(super) fn push_ledger(&mut self, row: EconomyLedgerRow) {
        if let Some(phase5) = &mut self.phase5 {
            phase5.ledger.push(row);
        }
    }

    /// The tick boundary: per `(owner, instance)` chain, rows take rule T1 order (stable:
    /// non-terminal, the Consume aggregate, the terminal row) and ordinals 0..n-1; chains
    /// keep their first-appearance order. Emits one `EconomyCheckpoint` when any row exists.
    pub(super) fn flush_economy_ledger(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        let Some(phase5) = &mut self.phase5 else {
            return;
        };
        let rows = std::mem::take(&mut phase5.ledger);
        if rows.is_empty() {
            return;
        }
        let mut order = Vec::new();
        let mut chains: BTreeMap<(OwnerKind, uuid::Uuid, ItemInstanceId), Vec<EconomyLedgerRow>> =
            BTreeMap::new();
        for row in rows {
            let key = (row.owner_kind, row.owner, row.instance);
            if !chains.contains_key(&key) {
                order.push(key);
            }
            chains.entry(key).or_default().push(row);
        }
        let mut ledger = Vec::new();
        for key in order {
            let Some(mut chain) = chains.remove(&key) else {
                continue;
            };
            chain.sort_by_key(EconomyLedgerRow::t1_class);
            for (position, mut row) in chain.into_iter().enumerate() {
                row.operation_ordinal = u32::try_from(position).unwrap_or(u32::MAX);
                ledger.push(row);
            }
        }
        events.push(ZoneEvent::EconomyCheckpoint(Box::new(EconomyCheckpoint {
            tick,
            lease_releases: Vec::new(),
            ledger,
        })));
    }

    /// `GroundItem` records leave the block with their entity; the ground owner's ledger
    /// row for an NPC loot creation (design §11.3 "NPC loot creation").
    pub(super) fn ledger_loot(&mut self, record: &GroundItem, resolution: &str) {
        self.push_ledger(EconomyLedgerRow {
            owner_kind: OwnerKind::Ground,
            owner: record.entity.as_uuid(),
            instance: record.item.id,
            op: LedgerOp::Loot,
            operation_ordinal: 0,
            count_delta: i64::try_from(record.item.count).unwrap_or(i64::MAX),
            before: None,
            after: Some(record.item.clone()),
            transfer: None,
            receipt: None,
            causes: vec![format!("loot:{resolution}")],
        });
    }

    /// Decay: the ground owner's `destroy` row with cause `decay:<tick>` (design §3.8).
    pub(super) fn ledger_decay(&mut self, tick: Tick, record: &GroundItem) {
        self.push_ledger(EconomyLedgerRow {
            owner_kind: OwnerKind::Ground,
            owner: record.entity.as_uuid(),
            instance: record.item.id,
            op: LedgerOp::Destroy,
            operation_ordinal: 0,
            count_delta: i64::try_from(record.item.count)
                .unwrap_or(i64::MAX)
                .saturating_neg(),
            before: Some(record.item.clone()),
            after: None,
            transfer: None,
            receipt: None,
            causes: vec![format!("decay:{}", tick.0)],
        });
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::super::item_materialize::tests::{fresh_zone_and_kit_load, spawn_kit};
    use super::super::Spawned;
    use super::*;
    use crate::domain::zone::phase5::{DropProtect, GroundSource, LootOwner, Party, PartyId};
    use crate::domain::zone::{
        ItemCreationOrigin, SessionGeneration, Speed, Vec2Fixed, ZoneCommand, ZoneInput,
    };

    const P1: u128 = 71;
    const P2: u128 = 81;

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    /// A V7 zone with two admitted kit players at (10, 10) tiles.
    fn zone() -> ZoneState {
        let (mut zone, load, first) = fresh_zone_and_kit_load(true);
        assert_eq!(first, id(P1));
        let mut load = load;
        load.phase5 = Some(crate::domain::zone::Phase5Checkpoint {
            schema_version: 1,
            rules_hash: zone.phase5.as_ref().unwrap().rules.hash_hex(),
            recipes: crate::domain::zone::RecipeBook::default(),
        });
        let mut second = load.clone();
        if let Some(progression) = second.progression.as_mut() {
            progression.identity.account_id =
                crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(82));
        }
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(id(P1), load), spawn_kit(id(P2), second)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        zone
    }

    /// NPC loot of `template × count` at `pos`, protected for `owner` when given.
    fn drop(
        zone: &mut ZoneState,
        template: u32,
        count: u64,
        pos: Vec2Fixed,
        owner: Option<EntityId>,
    ) -> EntityId {
        let dropped = Tick(zone.next_tick.0 - 1);
        let item_id = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .allocator
            .allocate()
            .unwrap();
        let item = ItemInstance {
            creation_origin: Some(ItemCreationOrigin {
                zone: zone.seed.zone,
                epoch: zone.seed.epoch,
            }),
            id: item_id,
            template_id: template,
            count,
            location: ItemLocation::Ground,
            slot: None,
            enchant: 0,
            augment_id: None,
            charges: 0,
            created_tick: dropped,
            revision: 0,
        };
        let entity = GroundItem::entity_id(zone.seed.zone, zone.seed.epoch, item_id, dropped);
        zone.spawn(
            dropped,
            Spawned {
                id: entity,
                kind: EntityKind::GroundItem,
                name: "",
                pos,
                speed: Speed::from_milli_tiles_per_tick(0),
                generation: SessionGeneration::default(),
                combat: None,
            },
        )
        .unwrap();
        let until = Tick(dropped.0 + 150);
        zone.phase5.as_mut().unwrap().canonical.ground.insert(
            entity,
            GroundItem {
                entity,
                item,
                pos,
                dropped_tick: dropped,
                source: GroundSource::NpcLoot { npc: id(900) },
                loot_owner: owner.map(|owner| LootOwner { owner, until }),
                drop_protection: owner.map(|owner| DropProtect { owner, until }),
                decay_due: None,
            },
        );
        entity
    }

    fn pickup(
        zone: &mut ZoneState,
        actor: u128,
        key: u128,
        ground: EntityId,
    ) -> crate::domain::zone::AppliedTick {
        let command: Phase5Command = serde_json::from_value(serde_json::json!({
            "kind": "pick_up_item",
            "entity": id(actor),
            "request_key": uuid::Uuid::from_u128(key),
            "ground": ground,
        }))
        .unwrap();
        zone.run_tick(zone.draft(vec![ZoneInput::session(
            id(actor),
            SessionGeneration(1),
            1,
            ZoneCommand::Phase5(Box::new(command)),
        )]))
        .unwrap()
    }

    fn party(zone: &mut ZoneState, mode: LootMode) {
        let block = &mut zone.phase5.as_mut().unwrap().canonical;
        block.parties.insert(
            PartyId(1),
            Party {
                id: PartyId(1),
                members: vec![id(P1), id(P2)],
                distribution: mode,
                item_last_loot: 0,
                loot_vote: None,
                pending_invite: None,
            },
        );
        block.next_party = 2;
        for member in [P1, P2] {
            block.players.get_mut(&id(member)).unwrap().party = Some(PartyId(1));
        }
    }

    fn ledger(tick: &crate::domain::zone::AppliedTick) -> Vec<EconomyLedgerRow> {
        tick.events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::EconomyCheckpoint(c) => Some(c.ledger.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn holding(zone: &ZoneState, who: u128, template: u32) -> u64 {
        zone.phase4.as_ref().unwrap().canonical.players[&id(who)]
            .inventory
            .values()
            .filter(|i| i.template_id == template)
            .map(|i| i.count)
            .sum()
    }

    fn here() -> Vec2Fixed {
        Vec2Fixed::from_tiles(10, 10)
    }

    /// §4.1 solo: an identity transfer of the ground instance, then a merge into the stack;
    /// the ground entity leaves with its record, the owner gets its receipt, its projection
    /// and one revision step, and the economy checkpoint carries the T1 pair.
    #[test]
    fn solo_pickup_moves_the_instance_then_merges_into_the_stack() {
        let mut zone = zone();
        let ground = drop(&mut zone, 57, 120, here(), Some(id(P1)));
        let revision = zone.phase4.as_ref().unwrap().canonical.players[&id(P1)].inventory_revision;
        let item = zone.phase5.as_ref().unwrap().canonical.ground[&ground]
            .item
            .clone();
        let tick = pickup(&mut zone, P1, 1, ground);
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert!(!zone.entities.contains_key(&ground));
        assert!(zone.phase5.as_ref().unwrap().canonical.ground.is_empty());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&id(P1)];
        assert_eq!(player.inventory_revision, revision + 1);
        let held = &player.inventory[&item.id];
        assert_eq!((held.count, held.location), (120, ItemLocation::Inventory));
        assert!(tick
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::EntityDespawn { entity, .. } if *entity == ground)));
        assert!(tick
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(p) if p.entity == id(P1))));
        let receipt = tick
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::Economy(ev) => match &ev.body {
                    EconomyBody::EconomyResult {
                        outcome,
                        receipt_id,
                        ..
                    } => Some((outcome.clone(), receipt_id.clone())),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        assert_eq!(
            receipt.0,
            EconomyOutcome::Pickup {
                template_id: 57,
                count: 120,
                recipient: id(P1),
                herb: false,
                split: false
            }
        );
        assert_eq!(receipt.1, format!("receipt:pick_up_item:{}", uuid::Uuid::from_u128(1)));
        let rows = ledger(&tick);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].owner_kind, rows[0].op, rows[0].count_delta),
            (OwnerKind::Ground, LedgerOp::TransferOut, -120)
        );
        assert_eq!(
            (rows[1].owner_kind, rows[1].op, rows[1].instance),
            (OwnerKind::Character, LedgerOp::TransferIn, item.id)
        );
        assert_eq!(
            rows[0].transfer.as_ref().unwrap().link,
            rows[1].transfer.as_ref().unwrap().link
        );
        assert_eq!(rows[1].transfer.as_ref().unwrap().mode, TransferMode::Identity);
        // A second adena pile merges into the stack.
        let second = drop(&mut zone, 57, 5, here(), None);
        let tick = pickup(&mut zone, P1, 2, second);
        assert!(tick.dispositions.is_empty());
        assert_eq!(holding(&zone, P1, 57), 125);
        let rows = ledger(&tick);
        assert_eq!(rows[1].transfer.as_ref().unwrap().mode, TransferMode::Merge);
        assert_eq!((rows[1].before.as_ref().unwrap().count, rows[1].count_delta), (120, 5));
    }

    /// §4.1 check order: not found, out of range, `DropProtection`, loot owner, dead actor; a
    /// refusal changes nothing.
    #[test]
    fn pickup_refusals_follow_the_pinned_order_and_change_nothing() {
        let mut zone = zone();
        let far = Vec2Fixed::from_tiles(12, 10);
        let protected = drop(&mut zone, 57, 10, here(), Some(id(P2)));
        let distant = drop(&mut zone, 57, 10, far, None);
        let digest = zone.state_digest();
        let reasons = |tick: &crate::domain::zone::AppliedTick| -> Vec<RejectReason> {
            tick.dispositions.iter().map(|d| d.reason).collect()
        };
        let tick = pickup(&mut zone, P1, 1, id(4242));
        assert_eq!(reasons(&tick), vec![RejectReason::GroundItemNotFound]);
        let tick = pickup(&mut zone, P1, 2, distant);
        assert_eq!(reasons(&tick), vec![RejectReason::OutOfRange]);
        let tick = pickup(&mut zone, P1, 3, protected);
        assert_eq!(reasons(&tick), vec![RejectReason::GroundItemProtected]);
        assert!(ledger(&tick).is_empty());
        // Only the tick counter moved.
        assert_eq!(zone.phase5.as_ref().unwrap().canonical.ground.len(), 2);
        assert_ne!(zone.state_digest(), digest, "ticks advance");
        // Loot owner only (protection expired): still refused for a non-member.
        zone.phase5
            .as_mut()
            .unwrap()
            .canonical
            .ground
            .get_mut(&protected)
            .unwrap()
            .drop_protection = None;
        let tick = pickup(&mut zone, P1, 4, protected);
        assert_eq!(reasons(&tick), vec![RejectReason::GroundItemProtected]);
        // In the owner's party both pass; adena then splits evenly in any mode (§4.2).
        party(&mut zone, LootMode::FindersKeepers);
        let tick = pickup(&mut zone, P1, 5, protected);
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!((holding(&zone, P1, 57), holding(&zone, P2, 57)), (5, 5));
    }

    /// z-review-3 P3-5: a protection is live while `tick < until`; a pickup in the `until`
    /// tick itself passes.
    #[test]
    fn protections_end_at_their_until_tick() {
        let mut zone = zone();
        let ground = drop(&mut zone, 57, 10, here(), Some(id(P2)));
        let until = Tick(zone.next_tick.0 + 1);
        let record = zone
            .phase5
            .as_mut()
            .unwrap()
            .canonical
            .ground
            .get_mut(&ground)
            .unwrap();
        record.drop_protection.as_mut().unwrap().until = until;
        record.loot_owner.as_mut().unwrap().until = until;
        let tick = pickup(&mut zone, P1, 1, ground);
        assert_eq!(
            tick.dispositions
                .iter()
                .map(|d| d.reason)
                .collect::<Vec<_>>(),
            vec![RejectReason::GroundItemProtected]
        );
        assert_eq!(zone.next_tick, until);
        assert!(pickup(&mut zone, P1, 2, ground).dispositions.is_empty());
        assert_eq!(holding(&zone, P1, 57), 10);
    }

    /// D68: a pickup that would exceed `MaxAdena` is refused before any change.
    #[test]
    fn pickup_over_the_adena_cap_is_refused() {
        let mut zone = zone();
        let first = drop(&mut zone, 57, 1, here(), None);
        assert!(pickup(&mut zone, P1, 1, first).dispositions.is_empty());
        let stack = zone.phase4.as_ref().unwrap().canonical.players[&id(P1)]
            .inventory
            .values()
            .find(|i| i.template_id == 57)
            .unwrap()
            .id;
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&id(P1))
            .unwrap()
            .inventory
            .get_mut(&stack)
            .unwrap()
            .count = crate::domain::zone::item_inventory::MAX_ADENA;
        super::super::skip_v6_restore_identity();
        let pile = drop(&mut zone, 57, 1, here(), None);
        let tick = pickup(&mut zone, P1, 2, pile);
        assert_eq!(
            tick.dispositions
                .iter()
                .map(|d| d.reason)
                .collect::<Vec<_>>(),
            vec![RejectReason::ItemCountCap]
        );
        assert!(zone.entities.contains_key(&pile));
    }

    /// §4.2 `RANDOM`: one `roll_below(2)`; `LootDistributed` to the other member; the looter's
    /// row is the identity transfer, with the `party:` cause.
    #[test]
    fn party_random_draws_the_looter_and_tells_the_others() {
        let mut zone = zone();
        party(&mut zone, LootMode::Random);
        let ground = drop(&mut zone, 1, 1, here(), None);
        let words = zone.rng.get_word_pos();
        let tick = pickup(&mut zone, P1, 1, ground);
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert!(zone.rng.get_word_pos() > words, "draw #8 taken");
        let rows = ledger(&tick);
        let recipient = EntityId::from_uuid(rows[1].owner);
        assert!(rows[1]
            .causes
            .contains(&format!("party:{}", uuid::Uuid::from_u128(1))));
        let (body, audience) = tick
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::Economy(ev)
                    if matches!(ev.body, EconomyBody::LootDistributed { .. }) =>
                {
                    Some((ev.body.clone(), ev.audience.clone()))
                },
                _ => None,
            })
            .unwrap();
        assert!(
            matches!(body, EconomyBody::LootDistributed { recipient: r, count: 1, .. } if r == recipient)
        );
        assert!(!audience.contains(&recipient) && audience.len() == 1);
        assert_eq!(holding(&zone, if recipient == id(P1) { P1 } else { P2 }, 1), 1);
    }

    /// §4.2 `BY_TURN`: the cursor pre-increments and wraps; the next looter takes it.
    #[test]
    fn party_by_turn_advances_the_cursor() {
        let mut zone = zone();
        party(&mut zone, LootMode::ByTurn);
        let words = zone.rng.get_word_pos();
        let first = drop(&mut zone, 1, 1, here(), None);
        assert!(pickup(&mut zone, P1, 1, first).dispositions.is_empty());
        let cursor = |zone: &ZoneState| {
            zone.phase5.as_ref().unwrap().canonical.parties[&PartyId(1)].item_last_loot
        };
        // 0 → 1: member P2 loots.
        assert_eq!(cursor(&zone), 1);
        assert_eq!(holding(&zone, P2, 1), 1);
        let second = drop(&mut zone, 1, 1, here(), None);
        assert!(pickup(&mut zone, P1, 2, second).dispositions.is_empty());
        // 1 → wraps to 0: P1.
        assert_eq!(cursor(&zone), 0);
        assert_eq!(holding(&zone, P1, 1), 1);
        assert_eq!(zone.rng.get_word_pos(), words, "BY_TURN draws nothing");
    }

    /// §4.2 even split of adena: `each = amount / k`, the remainder by draws; the ground
    /// instance is destroyed and each recipient gets a `party_split` row; no
    /// `LootDistributed`.
    #[test]
    fn party_even_split_divides_adena_without_loot_messages() {
        let mut zone = zone();
        party(&mut zone, LootMode::Random);
        let ground = drop(&mut zone, 57, 7, here(), None);
        let tick = pickup(&mut zone, P1, 1, ground);
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let (a, b) = (holding(&zone, P1, 57), holding(&zone, P2, 57));
        assert_eq!(a + b, 7);
        assert!(a >= 3 && b >= 3);
        let rows = ledger(&tick);
        assert_eq!((rows[0].owner_kind, rows[0].op), (OwnerKind::Ground, LedgerOp::Destroy));
        assert_eq!(rows.iter().filter(|r| r.op == LedgerOp::PartySplit).count(), 2);
        assert!(!tick.events.iter().any(|e| matches!(
            e,
            ZoneEvent::Economy(ev) if matches!(ev.body, EconomyBody::LootDistributed { .. })
        )));
        assert!(matches!(
            tick.events.iter().find_map(|e| match e {
                ZoneEvent::Economy(ev) => match &ev.body {
                    EconomyBody::EconomyResult { outcome, .. } => Some(outcome.clone()),
                    _ => None,
                },
                _ => None,
            }),
            Some(EconomyOutcome::Pickup {
                split: true,
                count: 7,
                ..
            })
        ));
    }
}
