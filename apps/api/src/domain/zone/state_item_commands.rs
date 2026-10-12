//! V6 equipment commands: stage all scalar/provider/resource changes before publishing.
use super::*;
use crate::domain::zone::effects::StatusFlags;
use crate::domain::zone::item_conditions::ItemConditionContext;
use crate::domain::zone::item_equipment::PaperdollMutation;
use crate::domain::zone::phase4_projection::{
    FrozenItemResult, ItemDeltaPlan, ItemOperation, ItemReceiptFact, ItemReceiptRef,
    ItemResultDetail,
};
use crate::domain::zone::{CastState, EquipSlot, ItemCommand, ItemCommandKind, Phase4Projection};
use std::fmt::Write as _;

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}
impl ZoneState {
    pub(super) fn item_command_v6(
        &mut self,
        tick: Tick,
        command: &ItemCommand,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let operation = match command.kind {
            ItemCommandKind::Equip { .. } => ItemOperation::Equip,
            ItemCommandKind::Unequip { .. } => ItemOperation::Unequip,
            ItemCommandKind::Use { .. }
            | ItemCommandKind::Destroy { .. }
            | ItemCommandKind::Enchant { .. }
            | ItemCommandKind::ToggleAutoShot { .. }
            | ItemCommandKind::Refine { .. }
            | ItemCommandKind::CancelRefine { .. }
            | ItemCommandKind::ExchangeSpecialAbility { .. }
            | ItemCommandKind::Crystallize { .. } => return Err(RejectReason::NotYetImplemented),
        };
        let account = command.authenticated_account().map_err(invalid)?;
        let mut fingerprint = String::with_capacity(64);
        for byte in command.fingerprint().map_err(invalid)? {
            write!(&mut fingerprint, "{byte:02x}").map_err(invalid)?;
        }
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
        let known = events.iter().find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(projection) = event {
                if projection.account == account {
                    return projection
                        .receipts
                        .iter()
                        .find(|r| r.reference() == reference);
                }
            }
            None
        });
        if let Some(receipt) = known {
            if receipt.fingerprint != fingerprint {
                return Err(RejectReason::ItemOperationConflict);
            }
            events.push(ZoneEvent::ItemResult(Box::new(receipt.result.clone())));
            return Ok(());
        }
        let mut candidate = self.clone();
        let mut local = Vec::new();
        let projection = candidate.equip_command_v6(
            tick,
            command,
            existing.cloned(),
            reference,
            fingerprint,
            &mut local,
        )?;
        *self = candidate;
        events.retain(
            |event| !matches!(event, ZoneEvent::ItemCheckpoint(p) if p.entity == command.entity),
        );
        events.extend(local);
        events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        Ok(())
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // Explicit staged operation and receipt inputs.
    fn equip_command_v6(
        &mut self,
        tick: Tick,
        command: &ItemCommand,
        previous: Option<Phase4Projection>,
        reference: ItemReceiptRef,
        fingerprint: String,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Phase4Projection, RejectReason> {
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
        let items = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?;
        let player = items
            .canonical
            .players
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let already_changed = previous.as_ref().is_some_and(|p| !p.ledger.is_empty());
        let revision = if already_changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
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
            },
            ItemCommandKind::Unequip { slot } => PaperdollMutation::unequip(
                &player.inventory,
                &player.paperdoll,
                slot,
                &items.rules,
                revision,
            ),
            ItemCommandKind::Use { .. }
            | ItemCommandKind::Destroy { .. }
            | ItemCommandKind::Enchant { .. }
            | ItemCommandKind::ToggleAutoShot { .. }
            | ItemCommandKind::Refine { .. }
            | ItemCommandKind::CancelRefine { .. }
            | ItemCommandKind::ExchangeSpecialAbility { .. }
            | ItemCommandKind::Crystallize { .. } => return Err(RejectReason::NotYetImplemented),
        }
        .map_err(|_| RejectReason::InvalidEquipSlot)?;
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
        self.phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .players
            .insert(id, staged);
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
        let phase = self.phase3.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let checkpoint = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(id, tick, &phase.canonical)
            .map_err(invalid)?;
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
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
        let operation_ordinal = u32::try_from(projection.receipts.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or(RejectReason::InvalidLoad)?;
        let mut affected = std::collections::BTreeSet::new();
        for change in mutation.changes {
            let after = change.after.as_ref().ok_or(RejectReason::InvalidLoad)?;
            affected.insert(after.id);
            projection.ledger.push(ItemDeltaPlan {
                operation: if after.slot.is_some() {
                    ItemOperation::Equip
                } else {
                    ItemOperation::Unequip
                },
                operation_ordinal,
                instance: after.id,
                count_delta: 0,
                before: change.before,
                after: change.after,
                receipt: Some(reference),
                causes: Vec::new(),
            });
        }
        let result = FrozenItemResult {
            entity: id,
            incarnation,
            tick,
            inventory_revision: projection.checkpoint.inventory_revision,
            receipt: command.request_id,
            detail: ItemResultDetail::Operation {
                operation: reference.operation,
                affected: affected.into_iter().collect(),
                recipe: None,
            },
        };
        projection.receipts.push(ItemReceiptFact {
            request_id: command.request_id,
            fingerprint,
            result: result.clone(),
        });
        projection
            .validate_structure(
                phase
                    .canonical
                    .checkpoint(id, tick)
                    .map_err(invalid)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(invalid)?;
        events.push(ZoneEvent::ItemResult(Box::new(result)));
        Ok(projection)
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
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let Some(items) = &self.phase4 else {
            return Ok(());
        };
        let phase = self.phase3.as_ref().ok_or(TickError::Phase3Invariant)?;
        for event in events.iter_mut() {
            if let ZoneEvent::ItemCheckpoint(projection) = event {
                projection.checkpoint = items
                    .canonical
                    .checkpoint(projection.entity, tick, &phase.canonical)
                    .map_err(|_| TickError::Phase3Invariant)?;
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
