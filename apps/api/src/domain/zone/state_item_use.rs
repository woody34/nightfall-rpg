//! Accepted item-use planning; every skill cost is staged before any cast can start.
use super::*;
use crate::domain::zone::item_inventory::{InventoryBoundary, InventoryLimits, InventoryMutation};
use crate::domain::zone::item_rules::ItemDefinition;
use crate::domain::zone::phase4::ItemReuse;
use crate::domain::zone::phase4_checkpoint::ItemReuseKey;
use crate::domain::zone::skill::{SkillId, SkillRef};
use crate::domain::zone::{CastState, ItemInstanceId, ItemLocation};
use std::collections::BTreeSet;

struct PreparedItemUse {
    definition: ItemDefinition,
    skills: Vec<(SkillRef, u32, bool)>,
    payments: InventoryMutation,
    reuse: Vec<ItemReuse>,
    clicked_count: u64,
}

impl ZoneState {
    #[allow(clippy::too_many_lines)] // Validate the whole source skill/cost/reuse plan before publication.
    fn prepare_item_use_v6(
        &self,
        entity: EntityId,
        instance: ItemInstanceId,
        boundary: InventoryBoundary,
    ) -> Result<PreparedItemUse, RejectReason> {
        let phase = self.phase3.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&entity)
            .ok_or(RejectReason::InvalidLoad)?;
        let item = player
            .inventory
            .get(&instance)
            .ok_or(RejectReason::ItemNotOwned)?;
        if item.location != ItemLocation::Inventory {
            return Err(RejectReason::ItemNotUsable);
        }
        if items
            .rules
            .parts()
            .capabilities
            .get(&item.template_id)
            .is_none_or(|cap| !cap.admitted || !cap.use_blockers.is_empty())
        {
            return Err(RejectReason::ItemCapabilityDeferred);
        }
        let definition = items
            .rules
            .parts()
            .items
            .get(&item.template_id)
            .ok_or(RejectReason::InvalidLoad)?;
        if !matches!(definition.handler.as_str(), "ItemSkills" | "Elixir" | "ManaPotion") {
            return Err(RejectReason::NotYetImplemented);
        }
        if definition.handler == "ManaPotion"
            && items
                .rules
                .parts()
                .capacities
                .policy
                .get("mana_potion_support")
                .map(String::as_str)
                != Some("true")
        {
            return Err(RejectReason::ItemCapabilityDeferred);
        }
        if !matches!(definition.etcitem_type.as_str(), "POTION" | "ELIXIR" | "SCROLL")
            && definition.flags.get("ex_immediate_effect") != Some(&true)
            && matches!(
                self.sidecar(entity)
                    .map_err(|_| RejectReason::InvalidLoad)?
                    .cast,
                crate::domain::zone::CastState::Casting(_)
            )
        {
            return Err(RejectReason::AlreadyCasting);
        }
        let key = if definition.shared_reuse_group > 0 {
            ItemReuseKey::SharedGroup(
                u32::try_from(definition.shared_reuse_group)
                    .map_err(|_| RejectReason::InvalidLoad)?,
            )
        } else {
            ItemReuseKey::Template(item.template_id)
        };
        if player
            .item_reuse
            .iter()
            .any(|reuse| reuse.key == key && reuse.until > boundary.tick)
        {
            return Err(RejectReason::ItemReuse);
        }
        let mut reuse: Vec<_> = player
            .item_reuse
            .iter()
            .filter(|reuse| reuse.until > boundary.tick)
            .cloned()
            .collect();
        if definition.reuse_delay > 0 {
            let until = Tick(
                boundary
                    .tick
                    .0
                    .checked_add(definition.reuse_delay.div_ceil(100))
                    .ok_or(RejectReason::InvalidLoad)?,
            );
            reuse.push(ItemReuse { key, until });
            reuse.sort_by_key(|reuse| reuse.key);
        }
        let mut skills = Vec::new();
        for source in definition
            .skills
            .iter()
            .filter(|source| source.kind == "item")
        {
            let reference = SkillRef {
                id: SkillId(source.id),
                rank: source.level,
            };
            let skill = phase
                .canonical
                .rules
                .skills()
                .get(reference.id)
                .ok_or(RejectReason::InvalidLoad)?;
            if items
                .rules
                .parts()
                .skill_capabilities
                .get(&source.id)
                .is_none_or(|cap| !cap.admitted)
                || reference.rank == 0
                || reference.rank > skill.max_level
            {
                return Err(RejectReason::ItemCapabilityDeferred);
            }
            let simultaneous =
                crate::domain::zone::item_cast::simultaneous(definition, skill, source.level)
                    .map_err(|_| RejectReason::InvalidLoad)?;
            skills.push((reference, source.xml_ordinal, simultaneous));
        }
        if skills.is_empty() {
            return Err(RejectReason::ItemNotUsable);
        }
        let edits = crate::domain::zone::item_cast::payment_edits(
            definition,
            phase.canonical.rules.skills(),
            instance,
        )
        .map_err(|_| RejectReason::InvalidLoad)?;
        let payments = InventoryMutation::prepare(
            &player.inventory,
            items.canonical.allocator,
            &edits,
            &items.rules,
            InventoryLimits::from_player(player),
            boundary,
        )
        .map_err(|_| RejectReason::InsufficientItems)?;
        let clicked_count = item
            .count
            .checked_sub(
                payments
                    .inventory
                    .get(&instance)
                    .map_or(0, |item| item.count),
            )
            .ok_or(RejectReason::InvalidLoad)?;
        Ok(PreparedItemUse {
            definition: definition.clone(),
            skills,
            payments,
            reuse,
            clicked_count,
        })
    }
}

impl ZoneState {
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // One candidate owns casts, payment, reuse and its frozen receipt.
    pub(super) fn resolve_item_use_v6(
        &mut self,
        tick: Tick,
        command: &crate::domain::zone::ItemCommand,
        timing: crate::domain::zone::state_skill::ItemCommandTiming,
        previous: Option<crate::domain::zone::Phase4Projection>,
        reference: crate::domain::zone::phase4_projection::ItemReceiptRef,
        fingerprint: String,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<crate::domain::zone::Phase4Projection, RejectReason> {
        use crate::domain::zone::item_conditions::ItemConditionContext;
        use crate::domain::zone::phase4_projection::{
            FrozenItemResult, ItemAuditEvidence, ItemEvidenceValue as V, ItemReceiptFact,
            ItemResultDetail,
        };
        use crate::domain::zone::state_skill::StagedCast;
        use crate::domain::zone::{ItemCommandKind, Phase4Projection};
        let ItemCommandKind::Use { instance, .. } = command.kind else {
            return Err(RejectReason::InvalidLoad);
        };
        let id = command.entity;
        let caster = self.binding(id).ok_or(RejectReason::UnknownEntity)?;
        let account = command
            .authenticated_account()
            .map_err(|_| RejectReason::InvalidLoad)?;
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let changed = timing.initial_inventory_revision.map_or_else(
            || previous.as_ref().is_some_and(|p| !p.ledger.is_empty()),
            |initial| initial != player.inventory_revision,
        );
        let revision = if changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let plan = self.prepare_item_use_v6(id, instance, InventoryBoundary { tick, revision })?;
        let combat = self
            .entities
            .get(&id)
            .and_then(|entity| entity.combat.as_ref())
            .ok_or(RejectReason::NotAPlayer)?;
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &combat.role
        else {
            return Err(RejectReason::NotAPlayer);
        };
        let can_resurrect = timing
            .target
            .and_then(|target| {
                target
                    .incarnation
                    .map(|incarnation| crate::domain::zone::effects::TargetBinding {
                        entity: target.entity,
                        incarnation,
                    })
            })
            .map(|target| self.can_resurrect_target_v6(target))
            .transpose()
            .map_err(|_| RejectReason::InvalidLoad)?
            .unwrap_or(false);
        if !(ItemConditionContext {
            identity: &progression.identity,
            class_id: progression.class_state.current_class_id,
            level: combat.sheet.level(),
            player,
            rules: &items.rules,
            can_resurrect,
        })
        .matches(&plan.definition.conditions)
        .map_err(|_| RejectReason::ItemCapabilityDeferred)?
        {
            return Err(RejectReason::ItemNotUsable);
        }
        for change in &plan.payments.changes {
            self.guard_item_removal_v6(
                id,
                change.before.as_ref().ok_or(RejectReason::InvalidLoad)?,
            )?;
        }
        let rules = Arc::clone(
            &self
                .phase3
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical
                .rules,
        );
        // Keep the pre-payment weight/effective sheet through every start in this item.
        for (skill, ordinal, simultaneous) in &plan.skills {
            let definition = rules
                .skills()
                .get(skill.id)
                .ok_or(RejectReason::InvalidLoad)?
                .clone();
            self.start_defined_cast_v6(
                tick,
                &StagedCast {
                    ordinal: timing.ordinal,
                    caster,
                    skill_id: skill.id,
                    target: timing.target,
                    canceled: None,
                },
                &definition,
                *skill,
                Some((
                    crate::domain::zone::phase4::ItemCastOrigin {
                        instance,
                        template: plan.definition.template_id,
                        count: plan.clicked_count,
                        skill_ordinal: *ordinal,
                        receipt: reference,
                    },
                    *simultaneous,
                )),
                events,
            )?;
        }
        let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        if !plan.payments.changes.is_empty() || plan.reuse != player.item_reuse {
            player.inventory_revision = revision;
        }
        player.inventory = plan.payments.inventory;
        player.item_reuse = plan.reuse;
        items.canonical.allocator = plan.payments.allocator;
        self.rebuild_v5(id).map_err(|_| RejectReason::InvalidLoad)?;
        if self.items_evidence_enabled() {
            for change in &plan.payments.changes {
                self.item_evidence_delta(
                    caster,
                    change,
                    crate::domain::zone::phase4_projection::ItemOperation::Consume,
                    0,
                    Some(reference),
                    &format!("use:{}", reference.key),
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
            .map_err(|_| RejectReason::InvalidLoad)?;
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
            enchant_attempts: Vec::new(),
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
        let operation_ordinal = u32::try_from(projection.receipts.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or(RejectReason::InvalidLoad)?;
        let mut affected = BTreeSet::new();
        for change in plan.payments.changes {
            let before = change.before.as_ref().ok_or(RejectReason::InvalidLoad)?;
            affected.insert(before.id);
            let count_delta = i64::try_from(
                before
                    .count
                    .checked_sub(change.after.as_ref().map_or(0, |item| item.count))
                    .ok_or(RejectReason::InvalidLoad)?,
            )
            .ok()
            .and_then(i64::checked_neg)
            .ok_or(RejectReason::InvalidLoad)?;
            let cause = ItemAuditEvidence {
                family: "item_cast_payment".into(),
                fields: BTreeMap::from([
                    ("receipt".into(), V::Text(reference.key.to_string())),
                    ("operation_ordinal".into(), V::Unsigned(u64::from(operation_ordinal))),
                    ("count_delta".into(), V::Signed(count_delta)),
                    ("source_instance".into(), V::Unsigned(instance.get())),
                ]),
            };
            super::item_commands::append_item_consumption(
                &mut projection,
                change,
                count_delta,
                cause,
            )?;
        }
        let result = FrozenItemResult {
            accepted_sequence: timing.accepted_sequence,
            entity: id,
            incarnation: caster.incarnation,
            tick,
            inventory_revision: projection.checkpoint.inventory_revision,
            receipt: reference.key,
            request_fingerprint: fingerprint.clone(),
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
        projection
            .validate_structure(
                phase
                    .checkpoint(id, tick)
                    .map_err(|_| RejectReason::InvalidLoad)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(|_| RejectReason::InvalidLoad)?;
        events.push(ZoneEvent::ItemResult(Box::new(result)));
        Ok(projection)
    }
}

impl ZoneState {
    /// L2Character.java:1560–1566: learned casts debit itemConsume once at accepted start.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // One candidate publishes the cast and its exact inventory work together.
    pub(super) fn start_paid_skill_cast_v6(
        &mut self,
        tick: Tick,
        entry: &crate::domain::zone::state_skill::StagedCast,
        skill: &crate::domain::zone::skill::SkillDef,
        reference: SkillRef,
        initial_revision: Option<u64>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        use crate::domain::zone::item_inventory::InventoryEdit;
        use crate::domain::zone::phase4_projection::{ItemAuditEvidence, ItemEvidenceValue as V};
        use crate::domain::zone::Phase4Projection;
        let id = entry.caster.entity;
        let previous = events.iter().find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(projection) = event {
                (projection.entity == id).then(|| projection.as_ref().clone())
            } else {
                None
            }
        });
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let changed = initial_revision.map_or_else(
            || previous.as_ref().is_some_and(|p| !p.ledger.is_empty()),
            |initial| initial != player.inventory_revision,
        );
        let revision = if changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let mut edits = Vec::new();
        for cost in &skill.item_consume {
            let template = *cost
                .item_id
                .at(reference.rank, skill.max_level)
                .ok_or(RejectReason::InvalidLoad)?;
            let count = *cost
                .count
                .at(reference.rank, skill.max_level)
                .ok_or(RejectReason::InvalidLoad)?;
            if template == 0 || count == 0 {
                return Err(RejectReason::InvalidLoad);
            }
            edits.push(InventoryEdit::TakeTemplate {
                template,
                count: u64::from(count),
                preferred: None,
            });
        }
        let payments = InventoryMutation::prepare(
            &player.inventory,
            items.canonical.allocator,
            &edits,
            &items.rules,
            InventoryLimits::from_player(player),
            InventoryBoundary { tick, revision },
        )
        .map_err(|_| RejectReason::InsufficientItems)?;
        for change in &payments.changes {
            self.guard_item_removal_v6(
                id,
                change.before.as_ref().ok_or(RejectReason::InvalidLoad)?,
            )?;
        }
        let mut candidate = self.clone();
        let mut local = Vec::new();
        candidate.start_defined_cast_v6(tick, entry, skill, reference, None, &mut local)?;
        let CastState::Casting(cast) = &candidate
            .sidecar(id)
            .map_err(|_| RejectReason::InvalidLoad)?
            .cast
        else {
            return Err(RejectReason::InvalidLoad);
        };
        let cast_id = cast.id;
        let items = candidate.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        player.inventory = payments.inventory;
        player.inventory_revision = revision;
        items.canonical.allocator = payments.allocator;
        candidate
            .rebuild_v5(id)
            .map_err(|_| RejectReason::InvalidLoad)?;
        if candidate.items_evidence_enabled() {
            for change in &payments.changes {
                candidate.item_evidence_delta(
                    entry.caster,
                    change,
                    crate::domain::zone::phase4_projection::ItemOperation::Consume,
                    0,
                    None,
                    &format!("cast:{}", cast_id.0),
                );
            }
        }
        let phase = &candidate
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        let checkpoint = candidate
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(id, tick, phase)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let account = match &candidate
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or(RejectReason::InvalidLoad)?
            .role
        {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => p.identity.account_id,
            CombatRole::Player {
                progression: None, ..
            }
            | CombatRole::Npc { .. } => return Err(RejectReason::NotAPlayer),
        };
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
            enchant_attempts: Vec::new(),
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
        for change in payments.changes {
            let before = change.before.as_ref().ok_or(RejectReason::InvalidLoad)?;
            let count_delta = i64::try_from(
                before
                    .count
                    .checked_sub(change.after.as_ref().map_or(0, |i| i.count))
                    .ok_or(RejectReason::InvalidLoad)?,
            )
            .ok()
            .and_then(i64::checked_neg)
            .ok_or(RejectReason::InvalidLoad)?;
            let cause = ItemAuditEvidence {
                family: "skill_cast_payment".into(),
                fields: BTreeMap::from([
                    ("cast_id".into(), V::Unsigned(cast_id.0)),
                    ("command_ordinal".into(), V::Unsigned(entry.ordinal.0)),
                    ("skill_id".into(), V::Unsigned(u64::from(reference.id.0))),
                    ("rank".into(), V::Unsigned(u64::from(reference.rank))),
                    ("count_delta".into(), V::Signed(count_delta)),
                ]),
            };
            super::item_commands::append_item_consumption(
                &mut projection,
                change,
                count_delta,
                cause,
            )?;
        }
        projection
            .validate_structure(
                phase
                    .checkpoint(id, tick)
                    .map_err(|_| RejectReason::InvalidLoad)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(|_| RejectReason::InvalidLoad)?;
        *self = candidate;
        events.retain(|event| !matches!(event, ZoneEvent::ItemCheckpoint(p) if p.entity == id));
        events.extend(local);
        events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        Ok(())
    }
}
