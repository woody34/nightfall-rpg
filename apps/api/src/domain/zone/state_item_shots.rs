//! Source manual/toggle shot payment, with randomized refusal retained in the record.
use super::*;
use crate::domain::zone::evidence::ItemEvidenceData;
use crate::domain::zone::item_inventory::{
    InventoryBoundary, InventoryChange, InventoryEdit, InventoryLimits, InventoryMutation,
};
use crate::domain::zone::item_shots::{ChargeOutcome, ChargeTerms};
use crate::domain::zone::phase4::ShotKind;
use crate::domain::zone::phase4_projection::{
    FrozenItemResult, ItemAuditEvidence, ItemDeltaPlan, ItemEvidenceValue as V, ItemOperation,
    ItemReceiptFact, ItemReceiptRef, ItemResultDetail,
};
use crate::domain::zone::state_skill::ItemCommandTiming;
use crate::domain::zone::{
    EquipSlot, ItemCommand, ItemCommandKind, ItemLocation, Phase4Projection,
};
use serde_json::json;

fn kind(handler: &str) -> Option<ShotKind> {
    match handler {
        "SoulShots" => Some(ShotKind::Soul),
        "SpiritShot" => Some(ShotKind::Spirit),
        "BlessedSpiritShot" => Some(ShotKind::BlessedSpirit),
        _ => None,
    }
}
fn names(charges: u8) -> Vec<&'static str> {
    [(4, "blessed"), (1, "soul"), (2, "spirit")]
        .into_iter()
        .filter_map(|(bit, name)| (charges & bit != 0).then_some(name))
        .collect()
}
fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

impl ZoneState {
    pub(super) fn is_shot_command_v6(&self, command: &ItemCommand) -> Result<bool, RejectReason> {
        match command.kind {
            ItemCommandKind::ToggleAutoShot { .. } => Ok(true),
            ItemCommandKind::Use { instance, .. } => {
                let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
                let item = items
                    .canonical
                    .players
                    .get(&command.entity)
                    .and_then(|player| player.inventory.get(&instance))
                    .ok_or(RejectReason::ItemNotOwned)?;
                Ok(items
                    .rules
                    .parts()
                    .items
                    .get(&item.template_id)
                    .is_some_and(|item| kind(&item.handler).is_some()))
            },
            ItemCommandKind::Equip { .. }
            | ItemCommandKind::Unequip { .. }
            | ItemCommandKind::Destroy { .. }
            | ItemCommandKind::Enchant { .. }
            | ItemCommandKind::Refine { .. }
            | ItemCommandKind::CancelRefine { .. }
            | ItemCommandKind::ExchangeSpecialAbility { .. }
            | ItemCommandKind::Crystallize { .. } => Ok(false),
        }
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // Stage one source handler, payment, header and receipt as one candidate.
    pub(super) fn resolve_shot_command_v6(
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
        let subject = self.binding(id).ok_or(RejectReason::InvalidLoad)?;
        let account = command.authenticated_account().map_err(invalid)?;
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let mut player = items
            .canonical
            .players
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?
            .clone();
        let (stack, enabled) = match command.kind {
            ItemCommandKind::Use { instance, .. } => {
                (player.inventory.get(&instance).cloned(), None)
            },
            ItemCommandKind::ToggleAutoShot { template, enabled } => (
                player
                    .inventory
                    .values()
                    .find(|item| {
                        item.template_id == template && item.location == ItemLocation::Inventory
                    })
                    .cloned(),
                Some(enabled),
            ),
            ItemCommandKind::Equip { .. }
            | ItemCommandKind::Unequip { .. }
            | ItemCommandKind::Destroy { .. }
            | ItemCommandKind::Enchant { .. }
            | ItemCommandKind::Refine { .. }
            | ItemCommandKind::CancelRefine { .. }
            | ItemCommandKind::ExchangeSpecialAbility { .. }
            | ItemCommandKind::Crystallize { .. } => return Err(RejectReason::InvalidLoad),
        };
        // RequestAutoSoulShot.java:63 checks ownership even for disabling a template.
        let stack = stack.ok_or(RejectReason::ItemNotOwned)?;
        if stack.location != ItemLocation::Inventory {
            return Err(RejectReason::ItemNotUsable);
        }
        let definition = items
            .rules
            .parts()
            .items
            .get(&stack.template_id)
            .ok_or(RejectReason::InvalidLoad)?
            .clone();
        let shot = kind(&definition.handler).ok_or(RejectReason::ItemNotUsable)?;
        if items
            .rules
            .parts()
            .capabilities
            .get(&stack.template_id)
            .is_none_or(|cap| !cap.admitted || !cap.use_blockers.is_empty())
        {
            return Err(RejectReason::ItemCapabilityDeferred);
        }
        let combat = self
            .entities
            .get(&id)
            .and_then(|entity| entity.combat.as_ref())
            .ok_or(RejectReason::InvalidLoad)?;
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &combat.role
        else {
            return Err(RejectReason::NotAPlayer);
        };
        let conditions = crate::domain::zone::item_conditions::ItemConditionContext {
            identity: &progression.identity,
            class_id: progression.class_state.current_class_id,
            level: combat.sheet.level(),
            player: &player,
            rules: &items.rules,
            can_resurrect: !self
                .effective(id)
                .map_err(invalid)?
                .flags
                .intersects(crate::domain::zone::effects::StatusFlags::RESURRECTION_BLOCK),
        };
        if !conditions
            .matches(&definition.conditions)
            .map_err(invalid)?
        {
            return Err(RejectReason::ItemNotUsable);
        }
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
        let operation_ordinal = u32::try_from(previous.as_ref().map_or(0, |p| p.receipts.len()))
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or(RejectReason::InvalidLoad)?;
        let before_toggle = player.auto_shots.clone();
        if let Some(enabled) = enabled {
            if enabled {
                player.auto_shots.insert(stack.template_id);
            } else {
                player.auto_shots.remove(&stack.template_id);
            }
        }
        let weapon = player
            .paperdoll
            .get(&EquipSlot::Rhand)
            .and_then(|id| player.inventory.get(id))
            .cloned();
        let weapon_definition = weapon
            .as_ref()
            .and_then(|weapon| items.rules.parts().items.get(&weapon.template_id))
            .cloned();
        let charged_bits = weapon.as_ref().map_or(0, |weapon| weapon.charges);
        let count = weapon_definition.as_ref().map_or(0, |weapon| {
            if shot == ShotKind::Soul {
                weapon.soulshots
            } else {
                weapon.spiritshots
            }
        });
        let reduction = weapon_definition
            .as_ref()
            .map_or(&[][..], |weapon| weapon.reduced_soulshot.as_slice());
        let weapon_grade = weapon_definition
            .as_ref()
            .map(|weapon| weapon.grade.as_str());
        let terms = ChargeTerms {
            kind: shot,
            charges: charged_bits,
            count,
            available: stack.count,
            weapon_grade,
            shot_grade: &definition.grade,
            correct_action: definition
                .properties
                .get("default_action")
                .map(String::as_str)
                == Some(if shot == ShotKind::Soul {
                    "SOULSHOT"
                } else {
                    "SPIRITSHOT"
                }),
            reduction,
        };
        let toggle = player.auto_shots.contains(&stack.template_id);
        let resolution = format!("shot:{:?}:{}", reference.operation, reference.key).to_lowercase();
        let mut residue = None;
        let mut draw = None;
        let resolved = if enabled == Some(false) {
            None
        } else {
            Some(
                terms
                    .resolve(|bound| {
                        let before = self.rng.get_word_pos();
                        let value = self.roll_below(bound);
                        residue = Some(value);
                        draw = Some((before, self.rng.get_word_pos(), bound, value));
                        Ok(value)
                    })
                    .map_err(invalid)?,
            )
        };
        if resolved.is_some_and(|result| result.disable_auto) {
            player.auto_shots.remove(&stack.template_id);
        }
        let refusal = if enabled.is_some() {
            None
        } else {
            match resolved.map(|result| result.outcome) {
                Some(ChargeOutcome::Unavailable | ChargeOutcome::WrongGrade) => {
                    Some(RejectReason::ItemNotUsable)
                },
                Some(ChargeOutcome::Insufficient) => Some(RejectReason::InsufficientItems),
                _ => None,
            }
        };
        let mut changes = Vec::new();
        if let Some(result) = resolved.filter(|result| result.outcome == ChargeOutcome::Charged) {
            let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
            let payment = InventoryMutation::prepare(
                &player.inventory,
                items.canonical.allocator,
                &[InventoryEdit::TakeInstance {
                    instance: stack.id,
                    count: u64::from(result.consumed),
                }],
                &items.rules,
                InventoryLimits::from_player(&player),
                InventoryBoundary { tick, revision },
            )
            .map_err(invalid)?;
            player.inventory = payment.inventory;
            changes = payment.changes;
            let before = weapon.ok_or(RejectReason::InvalidLoad)?;
            let after = player
                .inventory
                .get_mut(&before.id)
                .ok_or(RejectReason::InvalidLoad)?;
            after.charges = result.charges;
            after.revision = revision;
            changes.push(InventoryChange {
                before: Some(before),
                after: Some(after.clone()),
            });
        }
        if !changes.is_empty() || before_toggle != player.auto_shots {
            player.inventory_revision = revision;
        }
        self.phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .players
            .insert(id, player);
        self.rebuild_v5(id).map_err(invalid)?;
        if self.items_evidence_enabled() {
            if let Some((before, after, bound, value)) = draw {
                self.item_evidence(
                    subject,
                    ItemEvidenceData::Rng {
                        callsite: "soulshot_reduction".into(),
                        word_before: before,
                        word_after: after,
                        bound,
                        value,
                        resolution_id: resolution.clone(),
                    },
                );
            }
            if let Some(result) = resolved {
                let draws = residue
                    .map(|value| json!({"site":"soulshot_reduction","bound":100,"residue":value}))
                    .into_iter()
                    .collect::<Vec<_>>();
                self.item_evidence(subject, ItemEvidenceData::Formula(json!({
                    "formula":"shot_payment", "inputs": {"charges":names(charged_bits), "kind":match shot {ShotKind::Soul=>"soul", ShotKind::Spirit=>"spirit", ShotKind::BlessedSpirit=>"blessed"},
                        "count":count,"available":stack.count,"weapon_grade":weapon_grade,"shot_grade":definition.grade,
                        "toggle":toggle,"reduction":reduction,"residue":residue},
                    "actual":{"charges":names(result.charges),"available":stack.count.checked_sub(u64::from(result.consumed)).ok_or(RejectReason::InvalidLoad)?,
                        "consumed":result.consumed,"toggle":toggle && !result.disable_auto,"draws":draws,"outcome":result.outcome.evidence_name()},
                    "resolution_id":resolution,"baseline":false
                })));
            }
            for change in &changes {
                self.item_evidence_delta(
                    subject,
                    change,
                    if change.before.as_ref().map(|item| item.id) == Some(stack.id) {
                        ItemOperation::Consume
                    } else {
                        ItemOperation::Use
                    },
                    if change.before.as_ref().map(|item| item.id) == Some(stack.id) {
                        0
                    } else {
                        operation_ordinal
                    },
                    Some(reference),
                    &resolution,
                );
            }
        }
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
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
            enchant_attempts: Vec::new(),
            exchanges: Vec::new(),
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
        for change in changes {
            let before = change.before.as_ref().ok_or(RejectReason::InvalidLoad)?;
            affected.insert(before.id);
            if before.id == stack.id {
                let count_delta = i64::from(resolved.ok_or(RejectReason::InvalidLoad)?.consumed)
                    .checked_neg()
                    .ok_or(RejectReason::InvalidLoad)?;
                let cause = ItemAuditEvidence {
                    family: if enabled.is_some() {
                        "toggle_shot_payment"
                    } else {
                        "manual_shot_payment"
                    }
                    .into(),
                    fields: BTreeMap::from([
                        ("receipt".into(), V::Text(reference.key.to_string())),
                        ("operation_ordinal".into(), V::Unsigned(u64::from(operation_ordinal))),
                        ("count_delta".into(), V::Signed(count_delta)),
                        ("resolution_id".into(), V::Text(resolution.clone())),
                    ]),
                };
                super::item_commands::append_item_consumption(
                    &mut projection,
                    change,
                    count_delta,
                    cause,
                )?;
            } else {
                projection.ledger.push(ItemDeltaPlan {
                    operation: ItemOperation::Use,
                    operation_ordinal,
                    instance: before.id,
                    count_delta: 0,
                    before: change.before,
                    after: change.after,
                    receipt: Some(reference),
                    receipt_refs: Vec::new(),
                    causes: Vec::new(),
                });
            }
        }
        if let Some((before, after, bound, value)) = draw {
            projection.evidence.push(ItemAuditEvidence {
                family: "soulshot_reduction".into(),
                fields: BTreeMap::from([
                    ("resolution_id".into(), V::Text(resolution)),
                    ("word_before".into(), V::Text(before.to_string())),
                    ("word_after".into(), V::Text(after.to_string())),
                    ("bound".into(), V::Unsigned(u64::from(bound))),
                    ("residue".into(), V::Unsigned(u64::from(value))),
                ]),
            });
        }
        if refusal.is_none() {
            let result = FrozenItemResult {
                accepted_sequence: timing.accepted_sequence,
                request_fingerprint: fingerprint.clone(),
                entity: id,
                incarnation: subject.incarnation,
                tick,
                inventory_revision: projection.checkpoint.inventory_revision,
                receipt: reference.key,
                detail: ItemResultDetail::Operation {
                    operation: reference.operation,
                    affected: affected.into_iter().collect(),
                    recipe: None,
                },
            };
            projection.receipts.push(ItemReceiptFact {
                request_id: reference.key,
                fingerprint,
                result: result.clone(),
            });
            events.push(ZoneEvent::ItemResult(Box::new(result)));
        }
        projection
            .validate_structure(
                phase
                    .checkpoint(id, tick)
                    .map_err(invalid)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(invalid)?;
        // Only this fully validated attempt may publish its RNG/header when it refuses.
        Ok((projection, refusal))
    }
}
