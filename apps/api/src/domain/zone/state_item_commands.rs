//! V6 item commands: stage all scalar/provider/resource changes before publishing.
use super::*;
use crate::domain::zone::effects::StatusFlags;
use crate::domain::zone::item_conditions::ItemConditionContext;
use crate::domain::zone::item_equipment::PaperdollMutation;
use crate::domain::zone::phase4_projection::{
    FrozenItemResult, ItemDeltaPlan, ItemOperation, ItemReceiptFact, ItemReceiptRef,
    ItemResultDetail,
};
use crate::domain::zone::state_skill::{ItemCommandTiming, ItemReceiptState};
use crate::domain::zone::{CastState, EquipSlot, ItemCommand, ItemCommandKind, Phase4Projection};

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}
impl ZoneState {
    pub(super) fn item_account_v6(
        &self,
        command: &ItemCommand,
    ) -> Result<crate::domain::AccountId, RejectReason> {
        let account = command.authenticated_account().map_err(invalid)?;
        let entity = self
            .entities
            .get(&command.entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let Some(CombatRole::Player {
            progression: Some(player),
            ..
        }) = entity.combat.as_ref().map(|c| &c.role)
        else {
            return Err(RejectReason::NotAPlayer);
        };
        if player.identity.account_id != account {
            return Err(RejectReason::NotPermitted);
        }
        Ok(account)
    }

    #[cfg(test)]
    pub(super) fn item_command_v6(
        &mut self,
        tick: Tick,
        command: &ItemCommand,
        events: &mut Vec<ZoneEvent>,
        receipts: &mut BTreeMap<
            (crate::domain::account::AccountId, ItemReceiptRef),
            ItemReceiptState,
        >,
    ) -> Result<(), RejectReason> {
        self.item_command_at_v6(
            tick,
            command,
            ItemCommandTiming {
                ordinal: self.next_ordinal,
                accepted_sequence: None,
                target: if let ItemCommandKind::Use { target, .. } = command.kind {
                    target.map(|target| crate::domain::zone::state_skill::CapturedTarget {
                        entity: target.entity,
                        incarnation: Some(target.incarnation),
                    })
                } else {
                    None
                },
                initial_inventory_revision: None,
            },
            events,
            receipts,
        )
    }

    pub(super) fn item_command_at_v6(
        &mut self,
        tick: Tick,
        command: &ItemCommand,
        timing: ItemCommandTiming,
        events: &mut Vec<ZoneEvent>,
        receipts: &mut BTreeMap<
            (crate::domain::account::AccountId, ItemReceiptRef),
            ItemReceiptState,
        >,
    ) -> Result<(), RejectReason> {
        let account = self.item_account_v6(command)?;
        let operation = command.operation();
        let fingerprint = command.fingerprint_hex().map_err(invalid)?;
        let existing = events.iter().find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(projection) = event {
                if projection.entity == command.entity {
                    return Some(projection.as_ref());
                }
            }
            None
        });
        if existing.is_some_and(|projection| projection.account != account) {
            return Err(RejectReason::NotPermitted);
        }
        let reference = ItemReceiptRef {
            operation,
            key: command.request_id,
        };
        if let Some(stored) = command.receipt_conflict().map_err(invalid)? {
            let previous = receipts
                .get(&(account, reference))
                .map(|receipt| match receipt {
                    ItemReceiptState::Known(receipt) => receipt.fingerprint.as_str(),
                    ItemReceiptState::Conflict(fingerprint) => fingerprint.as_str(),
                });
            if previous.is_some_and(|fingerprint| fingerprint != stored) {
                return Err(RejectReason::InvalidLoad);
            }
            receipts
                .entry((account, reference))
                .or_insert_with(|| ItemReceiptState::Conflict(stored.to_owned()));
            return Err(RejectReason::ItemOperationConflict);
        }
        let recorded = command.known_result().map_err(invalid)?;
        if let Some(receipt) = receipts.get(&(account, reference)) {
            match receipt {
                ItemReceiptState::Known(receipt) => {
                    if receipt.fingerprint != fingerprint
                        || recorded
                            .as_ref()
                            .is_some_and(|result| result != &receipt.result)
                    {
                        return Err(RejectReason::ItemOperationConflict);
                    }
                    events.push(ZoneEvent::ItemResult(Box::new(receipt.result.clone())));
                    return Ok(());
                },
                ItemReceiptState::Conflict(stored) => {
                    if stored != &fingerprint || recorded.is_none() {
                        return Err(RejectReason::ItemOperationConflict);
                    }
                },
            }
        }
        if let Some(result) = recorded {
            receipts.insert(
                (account, reference),
                ItemReceiptState::Known(ItemReceiptFact {
                    request_id: command.request_id,
                    fingerprint,
                    result: result.clone(),
                }),
            );
            events.push(ZoneEvent::ItemResult(Box::new(result)));
            return Ok(());
        }
        let mut candidate = self.clone();
        let mut local = Vec::new();
        let (projection, refusal) = candidate.resolve_item_command_v6(
            tick,
            command,
            timing,
            existing.cloned(),
            reference,
            fingerprint,
            &mut local,
        )?;
        let receipt = projection
            .receipts
            .iter()
            .find(|r| r.reference() == reference)
            .cloned();
        if refusal.is_none() && receipt.is_none() {
            return Err(RejectReason::InvalidLoad);
        }
        *self = candidate;
        if let Some(receipt) = receipt {
            receipts.insert((account, reference), ItemReceiptState::Known(receipt));
        }
        events.retain(
            |event| !matches!(event, ZoneEvent::ItemCheckpoint(p) if p.entity == command.entity),
        );
        events.extend(local);
        events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        refusal.map_or(Ok(()), Err)
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // Explicit staged operation and receipt inputs.
    fn resolve_item_command_v6(
        &mut self,
        tick: Tick,
        command: &ItemCommand,
        timing: ItemCommandTiming,
        previous: Option<Phase4Projection>,
        reference: ItemReceiptRef,
        fingerprint: String,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(Phase4Projection, Option<RejectReason>), RejectReason> {
        let id = command.entity;
        let entity = self.entities.get(&id).ok_or(RejectReason::UnknownEntity)?;
        let combat = entity.combat.as_ref().ok_or(RejectReason::NotAPlayer)?;
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &combat.role
        else {
            return Err(RejectReason::NotAPlayer);
        };
        let account = command.authenticated_account().map_err(invalid)?;
        if progression.identity.account_id != account {
            return Err(RejectReason::NotPermitted);
        }
        if entity.targeting.dead {
            return Err(RejectReason::DeadActor);
        }
        let effective = self.effective(id).map_err(invalid)?;
        if effective.flags.intersects(
            StatusFlags::STUN
                .union(StatusFlags::SLEEP)
                .union(StatusFlags::PARALYZE),
        ) {
            return Err(RejectReason::Incapacitated);
        }
        if self.is_shot_command_v6(command)? {
            return self.resolve_shot_command_v6(
                tick,
                command,
                timing,
                previous,
                reference,
                fingerprint,
                events,
            );
        }
        if matches!(command.kind, ItemCommandKind::Use { .. }) {
            return self
                .resolve_item_use_v6(
                    tick,
                    command,
                    timing,
                    previous,
                    reference,
                    fingerprint,
                    events,
                )
                .map(|projection| (projection, None));
        }
        let items = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?;
        let player = items
            .canonical
            .players
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let already_changed = timing.initial_inventory_revision.map_or_else(
            || previous.as_ref().is_some_and(|p| !p.ledger.is_empty()),
            |initial| initial != player.inventory_revision,
        );
        let revision = if already_changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let operation_ordinal = previous
            .as_ref()
            .map_or(Ok(1), Phase4Projection::next_operation_ordinal)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let mut enchant = None;
        let mut exchange = None;
        let mut exchange_formula = None;
        let mut refine: Option<(
            crate::domain::zone::RefineAudit,
            Vec<crate::domain::zone::ItemInstanceId>,
        )> = None;
        let mut refine_evidence = None;
        let mut allocator = items.canonical.allocator;
        let mutation = match command.kind {
            ItemCommandKind::Equip {
                instance,
                preferred_slot,
            } => {
                let item = player
                    .inventory
                    .get(&instance)
                    .ok_or(RejectReason::ItemNotOwned)?;
                if !items
                    .rules
                    .parts()
                    .capabilities
                    .get(&item.template_id)
                    .is_some_and(|cap| cap.admitted)
                {
                    return Err(RejectReason::ItemCapabilityDeferred);
                }
                let definition = items
                    .rules
                    .parts()
                    .items
                    .get(&item.template_id)
                    .ok_or(RejectReason::InvalidLoad)?;
                let conditions = ItemConditionContext {
                    identity: &progression.identity,
                    class_id: progression.class_state.current_class_id,
                    level: combat.sheet.level(),
                    player,
                    rules: &items.rules,
                    can_resurrect: !effective.flags.intersects(StatusFlags::RESURRECTION_BLOCK),
                };
                if !conditions
                    .matches(&definition.conditions)
                    .map_err(|_| RejectReason::ItemCapabilityDeferred)?
                {
                    return Err(RejectReason::ItemNotUsable);
                }
                PaperdollMutation::equip(
                    &player.inventory,
                    &player.paperdoll,
                    instance,
                    preferred_slot,
                    &items.rules,
                    revision,
                )
                .map_err(|_| RejectReason::InvalidEquipSlot)?
            },
            ItemCommandKind::Unequip { slot } => PaperdollMutation::unequip(
                &player.inventory,
                &player.paperdoll,
                slot,
                &items.rules,
                revision,
            )
            .map_err(|_| RejectReason::InvalidEquipSlot)?,
            ItemCommandKind::Destroy { instance, .. }
            | ItemCommandKind::Crystallize { instance } => {
                let item = player
                    .inventory
                    .get(&instance)
                    .ok_or(RejectReason::ItemNotOwned)?;
                self.guard_item_removal_v6(id, item)?;
                let rank = self
                    .phase3
                    .as_ref()
                    .and_then(|phase| {
                        phase
                            .canonical
                            .rules
                            .skills()
                            .get(crate::domain::zone::skill::SkillId(248))
                    })
                    .and_then(|skill| {
                        progression
                            .class_state
                            .learned_skills
                            .iter()
                            .find(|entry| entry.key == skill.key)
                    })
                    .map_or(0, |entry| entry.level);
                let (plan, next) = removal_plan(
                    player,
                    &items.rules,
                    allocator,
                    &command.kind,
                    rank,
                    crate::domain::zone::item_inventory::InventoryBoundary {
                        tick,
                        revision,
                        origin: crate::domain::zone::ItemCreationOrigin {
                            zone: self.seed.zone,
                            epoch: self.seed.epoch,
                        },
                    },
                )?;
                allocator = next;
                plan
            },
            ItemCommandKind::Enchant {
                target,
                scroll,
                support,
            } => {
                for instance in [Some(target), Some(scroll), support].into_iter().flatten() {
                    self.guard_item_removal_v6(
                        id,
                        player
                            .inventory
                            .get(&instance)
                            .ok_or(RejectReason::ItemNotOwned)?,
                    )?;
                }
                let candidates = super::item_enchant::prepare(
                    player,
                    &items.rules,
                    allocator,
                    &command.kind,
                    crate::domain::zone::item_inventory::InventoryBoundary {
                        tick,
                        revision,
                        origin: crate::domain::zone::ItemCreationOrigin {
                            zone: self.seed.zone,
                            epoch: self.seed.epoch,
                        },
                    },
                    reference,
                    operation_ordinal,
                )?;
                if candidates.changes_weapon() {
                    self.guard_equipment_cast_v6(id)?;
                }
                let word_before = self.rng.get_word_pos();
                let resolved = candidates.resolve(self.rng.next_u32(), word_before);
                allocator = resolved.allocator;
                enchant = Some((resolved.detail, resolved.attempt, resolved.payments));
                resolved.mutation
            },
            ItemCommandKind::ExchangeSpecialAbility {
                npc,
                target,
                recipe,
            } => {
                self.guard_blacksmith_v6(id, npc)?;
                let plan = crate::domain::zone::item_exchange::prepare(
                    player,
                    &items.rules,
                    allocator,
                    target,
                    recipe,
                    crate::domain::zone::item_inventory::InventoryBoundary {
                        tick,
                        revision,
                        origin: crate::domain::zone::ItemCreationOrigin {
                            zone: self.seed.zone,
                            epoch: self.seed.epoch,
                        },
                    },
                    reference,
                    operation_ordinal,
                )?;
                for row in &plan.audit.rows {
                    if let Some(before) = &row.before {
                        self.guard_item_removal_v6(id, before)?;
                    }
                }
                if self.items_evidence_enabled() {
                    exchange_formula = Some(crate::domain::zone::item_exchange::evidence(
                        &plan.audit,
                        &player.inventory,
                        &plan.mutation.inventory,
                    )?);
                }
                allocator = plan.allocator;
                exchange = Some(plan.audit);
                plan.mutation
            },
            ItemCommandKind::Refine { npc, target, stone } => {
                self.guard_blacksmith_v6(id, npc)?;
                let prepared = crate::domain::zone::item_refine::prepare_refine(
                    player,
                    &items.rules,
                    allocator,
                    combat.sheet.level(),
                    target,
                    stone,
                    crate::domain::zone::item_inventory::InventoryBoundary {
                        tick,
                        revision,
                        origin: crate::domain::zone::ItemCreationOrigin {
                            zone: self.seed.zone,
                            epoch: self.seed.epoch,
                        },
                    },
                    reference,
                    operation_ordinal,
                )?;
                self.guard_refine_v6(id, &prepared)?;
                let selectors = prepared
                    .selectors
                    .clone()
                    .ok_or(RejectReason::InvalidLoad)?;
                let mut words = Vec::new();
                let rng = &mut self.rng;
                // Complete preflight precedes the three source draws (§3 phase 0).
                let selection = selectors
                    .select(|| {
                        let before = rng.get_word_pos();
                        let raw = rng.next_u32();
                        words.push((before, rng.get_word_pos(), raw));
                        raw
                    })
                    .map_err(invalid)?;
                let (mutation, next, audit, payments) =
                    prepared.finish(Some(selection), revision)?;
                allocator = next;
                refine_evidence = Some((selectors, words));
                refine = Some((audit, payments));
                mutation
            },
            ItemCommandKind::CancelRefine { npc, target } => {
                self.guard_blacksmith_v6(id, npc)?;
                let prepared = crate::domain::zone::item_refine::prepare_cancel(
                    player,
                    &items.rules,
                    allocator,
                    target,
                    crate::domain::zone::item_inventory::InventoryBoundary {
                        tick,
                        revision,
                        origin: crate::domain::zone::ItemCreationOrigin {
                            zone: self.seed.zone,
                            epoch: self.seed.epoch,
                        },
                    },
                    reference,
                    operation_ordinal,
                )?;
                self.guard_refine_v6(id, &prepared)?;
                let (mutation, next, audit, payments) = prepared.finish(None, revision)?;
                allocator = next;
                refine = Some((audit, payments));
                mutation
            },
            ItemCommandKind::Use { .. } | ItemCommandKind::ToggleAutoShot { .. } => {
                return Err(RejectReason::NotYetImplemented)
            },
        };
        // D55/D66: rows of one logical operation on the same instance (displacement then the
        // operation itself) take strictly increasing ordinals from the operation's ordinal.
        let row_ordinals = {
            let mut next = std::collections::BTreeMap::new();
            mutation
                .changes
                .iter()
                .map(|change| {
                    let instance = change
                        .after
                        .as_ref()
                        .or(change.before.as_ref())
                        .ok_or(RejectReason::InvalidLoad)?
                        .id;
                    let slot = next.entry(instance).or_insert(operation_ordinal);
                    let ordinal = *slot;
                    *slot = slot.checked_add(1).ok_or(RejectReason::InvalidLoad)?;
                    Ok(ordinal)
                })
                .collect::<Result<Vec<u32>, RejectReason>>()?
        };
        let weapon_changed = mutation.changes.iter().any(|change| {
            change
                .before
                .as_ref()
                .is_some_and(|item| item.slot == Some(EquipSlot::Rhand))
                || change
                    .after
                    .as_ref()
                    .is_some_and(|item| item.slot == Some(EquipSlot::Rhand))
        });
        if weapon_changed {
            self.guard_equipment_cast_v6(id)?;
        }
        let incarnation = combat.incarnation;
        let set_observations = if self.items_evidence_enabled() {
            crate::domain::zone::item_sources::set_shield_observations(
                player,
                &mutation,
                &items.rules,
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        let phase = self.phase3.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let (mut staged, next) = crate::domain::zone::item_sources::apply_paperdoll(
            player,
            &mutation,
            &items.rules,
            phase.canonical.next_install_sequence,
        )
        .map_err(invalid)?;
        if !mutation.changes.is_empty() {
            staged.inventory_revision = revision;
        }
        let items = &mut self
            .phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        items.allocator = allocator;
        items.players.insert(id, staged);
        self.phase3
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .next_install_sequence = next;
        self.rebuild_v5(id).map_err(invalid)?;
        if weapon_changed {
            self.disengage(tick, id, SwingCancel::Stopped, events);
            let items = &mut self
                .phase4
                .as_mut()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical;
            items.attack_plans.remove(&id);
            items.charge_bindings.retain(|binding| !matches!(binding.action, crate::domain::zone::phase4::ItemActionBinding::Attack { attacker, .. } if attacker == id));
        }
        if self.items_evidence_enabled() {
            let subject = self.binding(id).ok_or(RejectReason::InvalidLoad)?;
            let resolution = format!("{:?}:{}", reference.operation, reference.key).to_lowercase();
            // D46 `set_shield_history` (D20): one fact per set-shield transition.
            let event = format!("{:?}", reference.operation).to_lowercase();
            if matches!(event.as_str(), "equip" | "unequip" | "enchant") {
                self.gear_stats_evidence_v6(id, &event, &resolution);
            }
            // Same resolution as the command's ledger rows, so phases correlate the mutation.
            for observed in &set_observations {
                self.item_formula_v6(
                    id,
                    "set_shield_history",
                    serde_json::json!({"previous": observed.previous,
                        "mutation": observed.mutation, "chest_present": observed.chest_present,
                        "matching_shield": observed.matching_shield, "complete": observed.complete,
                        "required_part": observed.required_part}),
                    serde_json::json!(observed.active),
                    &resolution,
                    Some(serde_json::json!({"event": event, "set_id": observed.set_id})),
                );
            }
            if let Some(fact) = exchange_formula {
                self.item_evidence(
                    subject,
                    crate::domain::zone::evidence::ItemEvidenceData::Formula(fact),
                );
            }
            if let Some((detail, attempt, _)) = &enchant {
                self.item_evidence_enchant(subject, attempt, detail)?;
            }
            if let Some((audit, _)) = &refine {
                self.item_evidence_refine(subject, audit, refine_evidence.as_ref(), &resolution)?;
            }
            for (change, row_ordinal) in mutation.changes.iter().zip(&row_ordinals) {
                let instance = change
                    .after
                    .as_ref()
                    .or(change.before.as_ref())
                    .ok_or(RejectReason::InvalidLoad)?
                    .id;
                let before_count = change.before.as_ref().map_or(0, |i| i.count);
                let after_count = change.after.as_ref().map_or(0, |i| i.count);
                let refine_target = refine.as_ref().is_some_and(|(audit, _)| {
                    audit.after.id == instance
                        && change.before.as_ref().map(|i| i.augment_id)
                            != change.after.as_ref().map(|i| i.augment_id)
                });
                let refine_payment = refine
                    .as_ref()
                    .is_some_and(|(_, payments)| payments.contains(&instance))
                    && after_count < before_count;
                let operation = if refine_target {
                    reference.operation
                } else if refine_payment
                    || exchange.as_ref().is_some_and(|audit| {
                        audit.rows.iter().any(|row| {
                            row.instance == instance && row.operation == ItemOperation::Consume
                        })
                    }) && after_count < before_count
                    || enchant
                        .as_ref()
                        .is_some_and(|(_, _, payments)| payments.contains(&instance))
                {
                    ItemOperation::Consume
                } else if enchant
                    .as_ref()
                    .is_some_and(|(_, attempt, _)| attempt.target == instance)
                    && change.before.as_ref().and_then(|i| i.slot)
                        == change.after.as_ref().and_then(|i| i.slot)
                    && before_count == after_count
                {
                    ItemOperation::Enchant
                } else if after_count < before_count {
                    ItemOperation::Destroy
                } else if after_count > before_count {
                    ItemOperation::Grant
                } else if change.after.as_ref().is_some_and(|i| i.slot.is_some()) {
                    ItemOperation::Equip
                } else {
                    ItemOperation::Unequip
                };
                self.item_evidence_delta(
                    subject,
                    change,
                    operation,
                    if operation == ItemOperation::Consume {
                        0
                    } else {
                        *row_ordinal
                    },
                    Some(reference),
                    &resolution,
                );
            }
        }
        let phase = self.phase3.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let checkpoint = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(id, tick, &phase.canonical)
            .map_err(invalid)?;
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
            enchant_attempts: Vec::new(),
            exchanges: Vec::new(),
            refines: Vec::new(),
            paused_core: None,
            tick,
            entity: id,
            account,
            checkpoint: checkpoint.clone(),
            ledger: Vec::new(),
            receipts: Vec::new(),
            materializations: Vec::new(),
            evidence: Vec::new(),
        });
        projection.checkpoint = checkpoint;
        let mut affected = std::collections::BTreeSet::new();
        for (change, row_ordinal) in mutation.changes.into_iter().zip(row_ordinals) {
            let instance = change
                .after
                .as_ref()
                .or(change.before.as_ref())
                .ok_or(RejectReason::InvalidLoad)?
                .id;
            let count_delta = i64::try_from(
                i128::from(change.after.as_ref().map_or(0, |i| i.count))
                    .checked_sub(i128::from(change.before.as_ref().map_or(0, |i| i.count)))
                    .ok_or(RejectReason::InvalidLoad)?,
            )
            .map_err(invalid)?;
            affected.insert(instance);
            if let Some((_, attempt, payments)) = &enchant {
                if payments.contains(&instance) {
                    append_consumption(&mut projection, change, count_delta, attempt)?;
                    continue;
                }
            }
            if let Some((audit, payments)) = &refine {
                if payments.contains(&instance) && count_delta < 0 {
                    let cause = audit.payment_cause(count_delta);
                    append_item_consumption(&mut projection, change, count_delta, cause)?;
                    continue;
                }
            }
            if let Some(row) = exchange.as_ref().and_then(|audit| {
                audit.rows.iter().find(|row| {
                    row.instance == instance
                        && row.operation == ItemOperation::Consume
                        && count_delta < 0
                })
            }) {
                let cause = row.causes.first().ok_or(RejectReason::InvalidLoad)?.clone();
                append_item_consumption(&mut projection, change, count_delta, cause)?;
                continue;
            }
            let refine_target = refine.as_ref().is_some_and(|(audit, _)| {
                audit.after.id == instance
                    && change.before.as_ref().map(|i| i.augment_id)
                        != change.after.as_ref().map(|i| i.augment_id)
            });
            projection.ledger.push(ItemDeltaPlan {
                receipt_refs: Vec::new(),
                operation: if refine_target {
                    reference.operation
                } else if enchant
                    .as_ref()
                    .is_some_and(|(_, attempt, _)| attempt.target == instance)
                    && change.before.as_ref().and_then(|i| i.slot)
                        == change.after.as_ref().and_then(|i| i.slot)
                    && count_delta == 0
                {
                    ItemOperation::Enchant
                } else if count_delta < 0 {
                    ItemOperation::Destroy
                } else if count_delta > 0 {
                    ItemOperation::Grant
                } else if change
                    .after
                    .as_ref()
                    .is_some_and(|after| after.slot.is_some())
                {
                    ItemOperation::Equip
                } else {
                    ItemOperation::Unequip
                },
                operation_ordinal: row_ordinal,
                instance,
                count_delta,
                before: change.before,
                after: change.after,
                receipt: Some(reference),
                causes: Vec::new(),
            });
        }
        let detail = if let Some(audit) = exchange {
            let recipe = audit.recipe;
            projection.exchanges.push(audit);
            ItemResultDetail::Operation {
                operation: ItemOperation::Exchange,
                affected: affected.into_iter().collect(),
                recipe: Some(recipe),
            }
        } else if let Some((audit, _)) = refine {
            let detail = ItemResultDetail::Refine {
                target: audit.after.id,
                augment: audit.after.augment_id,
                outcome: audit.outcome,
            };
            projection.refines.push(audit);
            detail
        } else if let Some((detail, attempt, _)) = enchant {
            projection.evidence.push(attempt.cause.clone());
            projection.enchant_attempts.push(attempt);
            detail
        } else {
            ItemResultDetail::Operation {
                operation: reference.operation,
                affected: affected.into_iter().collect(),
                recipe: None,
            }
        };
        let result = FrozenItemResult {
            accepted_sequence: timing.accepted_sequence,
            request_fingerprint: fingerprint.clone(),
            entity: id,
            incarnation,
            tick,
            inventory_revision: projection.checkpoint.inventory_revision,
            receipt: command.request_id,
            detail,
        };
        projection.receipts.push(ItemReceiptFact {
            request_id: command.request_id,
            fingerprint,
            result: result.clone(),
        });
        projection
            .validate_staged(
                phase
                    .canonical
                    .checkpoint(id, tick)
                    .map_err(invalid)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(invalid)?;
        events.push(ZoneEvent::ItemResult(Box::new(result)));
        Ok((projection, None))
    }

    /// Refine/cancel displacement and payments obey the same cast guards as removal.
    fn guard_refine_v6(
        &self,
        owner: EntityId,
        prepared: &crate::domain::zone::item_refine::RefinePrepared,
    ) -> Result<(), RejectReason> {
        self.guard_item_removal_v6(owner, &prepared.audit.before)?;
        for change in &prepared.mutation.changes {
            if let Some(before) = &change.before {
                self.guard_item_removal_v6(owner, before)?;
            }
        }
        if prepared.changes_weapon() {
            self.guard_equipment_cast_v6(owner)?;
        }
        Ok(())
    }

    fn guard_blacksmith_v6(
        &self,
        owner: EntityId,
        binding: crate::domain::zone::effects::TargetBinding,
    ) -> Result<(), RejectReason> {
        let policy = &self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .rules
            .parts()
            .capacities
            .blacksmith;
        let npc = self
            .entities
            .get(&binding.entity)
            .ok_or(RejectReason::UnknownEntity)?;
        // Noncombat fixtures have no combat life; their wire incarnation is zero.
        if npc.kind != EntityKind::Npc
            || npc.name != policy.template
            || npc.combat.is_some()
            || npc.targeting.attackable
            || npc.targeting.dead
            || binding.incarnation != 0
            || npc.pos != Vec2Fixed::from_tiles(policy.x, policy.y)
        {
            return Err(RejectReason::NotPermitted);
        }
        let actor = self
            .entities
            .get(&owner)
            .ok_or(RejectReason::UnknownEntity)?;
        let radius = u128::from(policy.radius_millitiles);
        if actor.pos.distance_sq(npc.pos)
            > radius
                .checked_mul(radius)
                .ok_or(RejectReason::InvalidLoad)?
        {
            return Err(RejectReason::OutOfRange);
        }
        Ok(())
    }

    pub(super) fn guard_item_removal_v6(
        &self,
        id: EntityId,
        item: &crate::domain::zone::ItemInstance,
    ) -> Result<(), RejectReason> {
        let phase = self.phase3.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let mut casts = Vec::new();
        if let CastState::Casting(cast) = &self.sidecar(id).map_err(invalid)?.cast {
            casts.push(cast);
        }
        for lane in items
            .canonical
            .item_casts
            .values()
            .filter(|cast| cast.caster.entity == id)
        {
            if lane
                .origin
                .as_ref()
                .is_some_and(|payment| payment.instance == item.id)
                || lane.weapon == Some(item.id)
            {
                return Err(RejectReason::AlreadyCasting);
            }
            casts.push(&lane.cast);
        }
        for cast in casts {
            let skill = phase
                .canonical
                .rules
                .skills()
                .get(cast.skill.id)
                .ok_or(RejectReason::InvalidLoad)?;
            for cost in &skill.item_consume {
                if *cost
                    .item_id
                    .at(cast.skill.rank, skill.max_level)
                    .ok_or(RejectReason::InvalidLoad)?
                    == item.template_id
                {
                    return Err(RejectReason::AlreadyCasting);
                }
            }
        }
        Ok(())
    }

    fn guard_equipment_cast_v6(&self, id: EntityId) -> Result<(), RejectReason> {
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        if items
            .canonical
            .item_casts
            .values()
            .any(|cast| cast.caster.entity == id)
        {
            return Err(RejectReason::AlreadyCasting);
        }
        if let CastState::Casting(cast) = &self.sidecar(id).map_err(invalid)?.cast {
            let skill = self
                .phase3
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical
                .rules
                .skills()
                .get(cast.skill.id)
                .ok_or(RejectReason::InvalidLoad)?;
            if skill.conditions.iter().any(weapon_condition) {
                return Err(RejectReason::AlreadyCasting);
            }
        }
        Ok(())
    }
    pub(super) fn freeze_items_v6(
        &self,
        tick: Tick,
        before_items: Option<&crate::domain::zone::Phase4Snapshot>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let Some(items) = &self.phase4 else {
            return Ok(());
        };
        let phase = self.phase3.as_ref().ok_or(TickError::Phase3Invariant)?;
        for event in events.iter_mut() {
            if let ZoneEvent::ItemCheckpoint(projection) = event {
                if let Some(core) = &projection.paused_core {
                    projection
                        .validate_structure(
                            core.load
                                .phase3
                                .as_ref()
                                .ok_or(TickError::Phase3Invariant)?
                                .modifier_install_order
                                .len(),
                        )
                        .map_err(|_| TickError::Phase3Invariant)?;
                    continue;
                }
                projection.checkpoint = items
                    .canonical
                    .checkpoint(projection.entity, tick, &phase.canonical)
                    .map_err(|_| TickError::Phase3Invariant)?;
                // D133: one `auto_shot_exhausted` entry per template the whole tick net-removed
                // (tick-start set minus the final set), the last one recorded; none for a
                // template that ends the tick as it began (an enable that its own recharge
                // disabled, or a removal that a later operation re-enabled).
                let start = before_items
                    .and_then(|items| items.players.get(&projection.entity))
                    .map(|player| player.auto_shots.clone())
                    .unwrap_or_default();
                let removed: std::collections::BTreeSet<u32> = start
                    .difference(&projection.checkpoint.auto_shots)
                    .copied()
                    .collect();
                let template =
                    |entry: &crate::domain::zone::phase4_projection::ItemAuditEvidence| match entry
                        .fields
                        .get("template")
                    {
                        Some(
                            crate::domain::zone::phase4_projection::ItemEvidenceValue::Unsigned(id),
                        ) => u32::try_from(*id).ok(),
                        _ => None,
                    };
                let mut kept = std::collections::BTreeSet::new();
                let mut evidence = std::mem::take(&mut projection.evidence);
                evidence.reverse();
                evidence.retain(|entry| {
                    entry.family != "auto_shot_exhausted"
                        || template(entry)
                            .is_some_and(|id| removed.contains(&id) && kept.insert(id))
                });
                evidence.reverse();
                projection.evidence = evidence;
                // D94/P2-6: the frozen tick is normalized and renumbered, so the positional
                // validator (the one snapshots, decode and the API apply) runs here.
                projection
                    .validate_structure(
                        phase
                            .canonical
                            .checkpoint(projection.entity, tick)
                            .map_err(|_| TickError::Phase3Invariant)?
                            .modifier_install_order
                            .len(),
                    )
                    .map_err(|_| TickError::Phase3Invariant)?;
            }
        }
        // Preserve operation order, after all final resource/equipment facts.
        let mut results = Vec::new();
        events.retain(|event| {
            if matches!(event, ZoneEvent::ItemResult(_)) {
                results.push(event.clone());
                false
            } else {
                true
            }
        });
        events.extend(results);
        Ok(())
    }
}
fn weapon_condition(condition: &crate::domain::zone::skill::SkillCondition) -> bool {
    use crate::domain::zone::skill::SkillCondition as C;
    match condition {
        C::UsingKind { .. } | C::UsingSlot { .. } | C::Unsupported { .. } => true,
        C::And { conditions, .. } | C::Or { conditions, .. } | C::Not { conditions, .. } => {
            conditions.iter().any(weapon_condition)
        },
        C::PlayerLevelRange { .. }
        | C::PlayerInsideZoneIds { .. }
        | C::PlayerResting { .. }
        | C::PlayerStanding { .. }
        | C::PlayerMoving { .. }
        | C::PlayerRunning { .. }
        | C::PlayerCanEscape { .. }
        | C::PlayerFlyMounted { .. }
        | C::PlayerWeight { .. }
        | C::PlayerInventorySize { .. }
        | C::PlayerCanResurrect { .. } => false,
    }
}

// A complete plan pays inputs after unequipping, before allocating/merging the refund.
fn removal_plan(
    player: &crate::domain::zone::Phase4Player,
    rules: &crate::domain::zone::Phase4Rules,
    allocator: crate::domain::zone::ItemInstanceRange,
    kind: &ItemCommandKind,
    crystallize_rank: u32,
    boundary: crate::domain::zone::item_inventory::InventoryBoundary,
) -> Result<(PaperdollMutation, crate::domain::zone::ItemInstanceRange), RejectReason> {
    use crate::domain::zone::item_inventory::{InventoryEdit, InventoryLimits, InventoryMutation};
    let (instance, count, crystallize) = match *kind {
        ItemCommandKind::Destroy { instance, count } => (instance, count, false),
        ItemCommandKind::Crystallize { instance } => (instance, 1, true),
        ItemCommandKind::Equip { .. }
        | ItemCommandKind::Unequip { .. }
        | ItemCommandKind::Use { .. }
        | ItemCommandKind::Enchant { .. }
        | ItemCommandKind::ToggleAutoShot { .. }
        | ItemCommandKind::Refine { .. }
        | ItemCommandKind::CancelRefine { .. }
        | ItemCommandKind::ExchangeSpecialAbility { .. } => return Err(RejectReason::InvalidLoad),
    };
    let item = player
        .inventory
        .get(&instance)
        .ok_or(RejectReason::ItemNotOwned)?;
    let definition = rules
        .parts()
        .items
        .get(&item.template_id)
        .ok_or(RejectReason::InvalidLoad)?;
    if count == 0 || count > item.count || (!definition.stackable && count != 1) {
        return Err(RejectReason::InsufficientItems);
    }
    let mut edits = vec![InventoryEdit::TakeInstance { instance, count }];
    if crystallize {
        if !rules
            .parts()
            .capabilities
            .get(&item.template_id)
            .is_some_and(|cap| cap.admitted)
        {
            return Err(RejectReason::ItemCapabilityDeferred);
        }
        let grade = rules
            .parts()
            .grades
            .iter()
            .find(|g| g.grade == definition.grade)
            .ok_or(RejectReason::InvalidLoad)?;
        if !matches!(definition.kind.as_str(), "Weapon" | "Armor")
            || definition.stackable
            || definition.flags.get("is_crystallizable") != Some(&true)
            || definition.flags.get("is_hero") == Some(&true)
            || definition.crystal_count == 0
            || grade.id == 0
            || grade.crystal_id == 0
        {
            return Err(RejectReason::ItemNotUsable);
        }
        if crystallize_rank < u32::from(grade.id.min(5)) {
            return Err(RejectReason::CrystallizeLevelTooLow);
        }
        edits.push(InventoryEdit::Add {
            template: grade.crystal_id,
            count: crate::domain::zone::item_enchant::crystal_count(
                definition,
                item.enchant,
                rules,
            )
            .map_err(invalid)?,
        });
    } else if definition.flags.get("is_destroyable") != Some(&true)
        || definition.flags.get("is_hero") == Some(&true)
    {
        return Err(RejectReason::ItemNotUsable);
    }
    let mut plan = if let Some(slot) = item.slot {
        PaperdollMutation::unequip(
            &player.inventory,
            &player.paperdoll,
            slot,
            rules,
            boundary.revision,
        )
        .map_err(invalid)?
    } else {
        PaperdollMutation {
            inventory: player.inventory.clone(),
            paperdoll: player.paperdoll.clone(),
            changes: Vec::new(),
        }
    };
    let mutation = InventoryMutation::prepare(
        &plan.inventory,
        allocator,
        &edits,
        rules,
        InventoryLimits::from_player(player),
        boundary,
    )
    .map_err(|_| RejectReason::InventoryFull)?;
    plan.inventory = mutation.inventory;
    plan.changes.extend(mutation.changes);
    Ok((plan, mutation.allocator))
}

fn append_consumption(
    projection: &mut Phase4Projection,
    change: crate::domain::zone::item_inventory::InventoryChange,
    count_delta: i64,
    attempt: &crate::domain::zone::EnchantAttempt,
) -> Result<(), RejectReason> {
    use crate::domain::zone::phase4_projection::{ItemAuditEvidence, ItemEvidenceValue as V};
    let cause = ItemAuditEvidence {
        family: "enchant_payment".into(),
        fields: BTreeMap::from([
            ("receipt".into(), V::Text(attempt.receipt.key.to_string())),
            ("operation_ordinal".into(), V::Unsigned(u64::from(attempt.operation_ordinal))),
            ("count_delta".into(), V::Signed(count_delta)),
        ]),
    };
    append_item_consumption(projection, change, count_delta, cause)
}

pub(super) fn append_item_consumption(
    projection: &mut Phase4Projection,
    change: crate::domain::zone::item_inventory::InventoryChange,
    count_delta: i64,
    cause: crate::domain::zone::phase4_projection::ItemAuditEvidence,
) -> Result<(), RejectReason> {
    let instance = change.before.as_ref().ok_or(RejectReason::InvalidLoad)?.id;
    let receipt = cause.contributing_receipt().map_err(invalid)?;
    if let Some(index) = projection
        .ledger
        .iter()
        .position(|delta| delta.operation == ItemOperation::Consume && delta.instance == instance)
    {
        let mut aggregate = projection.ledger.remove(index);
        aggregate.count_delta = aggregate
            .count_delta
            .checked_add(count_delta)
            .ok_or(RejectReason::InvalidLoad)?;
        aggregate.after = change.after;
        if let Some(receipt) = receipt {
            if !aggregate.receipt_refs.contains(&receipt) {
                aggregate.receipt_refs.push(receipt);
            }
        }
        aggregate.causes.push(cause);
        // The last scalar must remain after intervening grants in the ordered ledger.
        projection.ledger.push(aggregate);
    } else {
        projection.ledger.push(ItemDeltaPlan {
            receipt_refs: receipt.into_iter().collect(),
            operation: ItemOperation::Consume,
            operation_ordinal: 0,
            instance,
            before: change.before,
            after: change.after,
            count_delta,
            receipt: None,
            causes: vec![cause],
        });
    }
    Ok(())
}
