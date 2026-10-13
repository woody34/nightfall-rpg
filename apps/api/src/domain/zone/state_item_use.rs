//! Accepted item-use planning; every skill cost is staged before any cast can start.
use super::*;
use crate::domain::zone::item_inventory::{InventoryBoundary, InventoryLimits, InventoryMutation};
use crate::domain::zone::item_rules::ItemDefinition;
use crate::domain::zone::phase4::ItemReuse;
use crate::domain::zone::phase4_checkpoint::ItemReuseKey;
use crate::domain::zone::skill::{SkillId, SkillRef};
use crate::domain::zone::skill_math::ExactRatio;
use crate::domain::zone::{ItemInstanceId, ItemLocation, Q};
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
        let plan = self.prepare_item_use_v6(
            id,
            instance,
            InventoryBoundary {
                tick,
                revision,
                origin: crate::domain::zone::ItemCreationOrigin {
                    zone: self.seed.zone,
                    epoch: self.seed.epoch,
                },
            },
        )?;
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
        let consumable = self.consumable_use_open_v6(id, tick, &plan);
        let rules = Arc::clone(
            &self
                .phase3
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical
                .rules,
        );
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
        let operation_ordinal = projection
            .next_operation_ordinal()
            .map_err(|_| RejectReason::InvalidLoad)?;
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
                Some((
                    &mut projection,
                    InventoryBoundary {
                        tick,
                        revision,
                        origin: crate::domain::zone::ItemCreationOrigin {
                            zone: self.seed.zone,
                            epoch: self.seed.epoch,
                        },
                    },
                )),
                events,
            )?;
        }
        self.rebuild_v5(id).map_err(|_| RejectReason::InvalidLoad)?;
        if let Some(inputs) = consumable {
            self.consumable_use_stage_v6(id, tick, &format!("use:{}", reference.key), inputs);
        }
        let phase = &self
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        projection.checkpoint = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(id, tick, phase)
            .map_err(|_| RejectReason::InvalidLoad)?;
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
            .validate_staged(
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
        let original_revision = player.inventory_revision;
        let had_previous = previous.is_some();
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
            InventoryBoundary {
                tick,
                revision,
                origin: crate::domain::zone::ItemCreationOrigin {
                    zone: self.seed.zone,
                    epoch: self.seed.epoch,
                },
            },
        )
        .map_err(|_| RejectReason::InsufficientItems)?;
        for change in &payments.changes {
            self.guard_item_removal_v6(
                id,
                change.before.as_ref().ok_or(RejectReason::InvalidLoad)?,
            )?;
        }
        let mut candidate = self.clone();
        let cast_id = crate::domain::zone::CastId(
            candidate
                .phase3
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical
                .next_cast_id,
        );
        // D46/D92 `learned_item_cost`: opened at the prepaid start, emitted at completion or
        // cancellation with its phase (evidence only; one cost template per learned skill).
        if let ([cost], true) = (skill.item_consume.as_slice(), candidate.items_evidence_enabled())
        {
            if let (Some(template), Some(count)) = (
                cost.item_id.at(reference.rank, skill.max_level).copied(),
                cost.count.at(reference.rank, skill.max_level).copied(),
            ) {
                let records = |inventory: &std::collections::BTreeMap<
                    crate::domain::zone::ItemInstanceId,
                    crate::domain::zone::ItemInstance,
                >| {
                    inventory
                        .values()
                        .filter(|item| item.template_id == template && item.count > 0)
                        .map(|item| {
                            serde_json::json!({"instance_id": item.id.get(),
                                "template_id": item.template_id, "count": item.count})
                        })
                        .collect::<Vec<_>>()
                };
                let mut consumed = serde_json::Map::new();
                for change in &payments.changes {
                    if let Some(before) = &change.before {
                        let take = before
                            .count
                            .saturating_sub(change.after.as_ref().map_or(0, |i| i.count));
                        consumed.insert(before.id.get().to_string(), serde_json::json!(take));
                    }
                }
                let inputs = serde_json::json!({"skill_id": reference.id.0, "rank": reference.rank,
                    "consume_id": template, "consume_count": count,
                    "inventory": records(&player.inventory), "repeat": false});
                let actual = serde_json::json!({"accepted": true,
                    "inventory": records(&payments.inventory), "consumed": consumed,
                    "completion_consumed": {}, "repeated_consumed": {}, "draws": []});
                if let Some(buffer) = candidate.evidence.as_mut() {
                    buffer.pending_cast.entry(cast_id.0).or_default().push((
                        id,
                        "learned_item_cost".into(),
                        format!("cast:{}", cast_id.0),
                        inputs,
                        actual,
                    ));
                }
            }
        }
        let items = candidate.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        player.inventory = payments.inventory;
        if !payments.changes.is_empty() {
            player.inventory_revision = revision;
        }
        items.canonical.allocator = payments.allocator;
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
        let mut local = Vec::new();
        candidate.start_defined_cast_v6(
            tick,
            entry,
            skill,
            reference,
            None,
            Some((
                &mut projection,
                InventoryBoundary {
                    tick,
                    revision,
                    origin: crate::domain::zone::ItemCreationOrigin {
                        zone: candidate.seed.zone,
                        epoch: candidate.seed.epoch,
                    },
                },
            )),
            &mut local,
        )?;
        candidate
            .rebuild_v5(id)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let phase = &candidate
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        projection.checkpoint = candidate
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(id, tick, phase)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let publish = had_previous
            || !projection.ledger.is_empty()
            || !projection.evidence.is_empty()
            || projection.checkpoint.inventory_revision != original_revision;
        projection
            .validate_staged(
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
        if publish {
            events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        }
        Ok(())
    }
}

/// Pre-payment `consumable_use` record: inputs, first cast id, item skill ids, debits, lanes.
struct ConsumableOpen {
    inputs: serde_json::Value,
    first_cast: u64,
    skill_ids: Vec<u32>,
    consumed: serde_json::Map<String, serde_json::Value>,
    lanes: Vec<&'static str>,
}

/// D46/D92 `consumable_use` evidence: opened before payment with the oracle's definition
/// record and the reuse/inventory state, staged after the casts start, emitted when the last
/// started cast reaches its terminal event. Evidence-only; skipped for conditioned skills.
impl ZoneState {
    fn reuse_key_number(key: ItemReuseKey) -> u32 {
        match key {
            ItemReuseKey::SharedGroup(group) => group,
            ItemReuseKey::Template(template) => template,
        }
    }

    fn consumable_use_open_v6(
        &self,
        id: EntityId,
        tick: Tick,
        plan: &PreparedItemUse,
    ) -> Option<ConsumableOpen> {
        if !self.items_evidence_enabled() || !self.phase4_player_v6(id) {
            return None;
        }
        let rules = &self.phase3.as_ref()?.canonical.rules;
        let definition = &plan.definition;
        let mut skills = Vec::new();
        let mut skill_ids = Vec::new();
        let mut conditioned = false;
        for (reference, _, simultaneous) in &plan.skills {
            let skill = rules.skills().get(reference.id)?;
            // Conditions export as their pinned XML; only canEscape and zone membership are
            // given a context (the accepted engine use already satisfied them).
            if skill.raw_conditions.iter().any(|raw| {
                [
                    "races", "race", "classId", "level", "weight", "invSize", "sex", "kind", "slot",
                ]
                .iter()
                .any(|key| raw.contains(&format!("{key}=")))
            }) {
                return None;
            }
            conditioned |= !skill.raw_conditions.is_empty();
            let rank = reference.rank;
            let (consume_id, consume_count) = match skill.item_consume.as_slice() {
                [] => (0, 0),
                [cost] => (
                    *cost.item_id.at(rank, skill.max_level)?,
                    *cost.count.at(rank, skill.max_level)?,
                ),
                _ => return None,
            };
            skill_ids.push(reference.id.0);
            skills.push(serde_json::json!({"id": reference.id.0, "rank": rank,
                "hit_time_ms": *skill.hit_time_ms.at(rank, skill.max_level)?,
                "static": skill.is_magic == 2, "simultaneous": simultaneous,
                "reuse_delay_ms": *skill.reuse_ms.at(rank, skill.max_level)?,
                "item_consume_id": consume_id, "item_consume_count": consume_count,
                "conditions": skill.raw_conditions, "effects": [], "blockers": []}));
        }
        let player = self.phase4.as_ref()?.canonical.players.get(&id)?;
        let mut available = serde_json::Map::new();
        for item in player.inventory.values() {
            if item.location == ItemLocation::Inventory {
                let entry = available
                    .entry(item.template_id.to_string())
                    .or_insert(serde_json::json!(0));
                *entry = serde_json::json!(entry.as_u64()?.checked_add(item.count)?);
            }
        }
        let item_deadlines: serde_json::Map<_, _> = player
            .item_reuse
            .iter()
            .filter(|reuse| reuse.until > tick)
            .map(|reuse| {
                (Self::reuse_key_number(reuse.key).to_string(), serde_json::json!(reuse.until.0))
            })
            .collect();
        let side = self.sidecar(id).ok()?;
        let skill_deadlines: serde_json::Map<_, _> = side
            .reuse
            .iter()
            .filter(|(skill, until)| skill_ids.contains(&skill.0) && **until > tick)
            .map(|(skill, until)| (skill.0.to_string(), serde_json::json!(until.0)))
            .collect();
        let main_casting = matches!(side.cast, crate::domain::zone::CastState::Casting(_));
        let inputs = serde_json::json!({
        "definition": {"template_id": definition.template_id, "handler": definition.handler,
            "default_action": definition.properties.get("default_action").map_or("NONE", String::as_str),
            "etcitem_type": definition.etcitem_type, "reuse_delay_ms": definition.reuse_delay,
            "reuse_group": definition.shared_reuse_group.max(0),
            "immediate": definition.flags.get("immediate_effect") == Some(&true),
            "ex_immediate": definition.flags.get("ex_immediate_effect") == Some(&true),
            "skills": skills},
        "tick": tick.0, "available": available,
        "item_deadlines": item_deadlines, "skill_deadlines": skill_deadlines,
        "main_casting": main_casting,
        "condition_context": if conditioned {
            serde_json::json!({"canEscape": true, "inside_zone_ids": []})
        } else {
            serde_json::Value::Null
        }});
        let first_cast = self.phase3.as_ref()?.canonical.next_cast_id;
        let mut consumed = serde_json::Map::new();
        for change in &plan.payments.changes {
            let before = change.before.as_ref()?;
            let take = before
                .count
                .checked_sub(change.after.as_ref().map_or(0, |i| i.count))?;
            let entry = consumed
                .entry(before.template_id.to_string())
                .or_insert(serde_json::json!(0));
            *entry = serde_json::json!(entry.as_u64()?.checked_add(take)?);
        }
        let lanes = plan
            .skills
            .iter()
            .map(|(_, _, simultaneous)| if *simultaneous { "item" } else { "main" })
            .collect();
        Some(ConsumableOpen {
            inputs,
            first_cast,
            skill_ids,
            consumed,
            lanes,
        })
    }

    fn consumable_use_stage_v6(
        &mut self,
        id: EntityId,
        tick: Tick,
        resolution: &str,
        open: ConsumableOpen,
    ) {
        let ConsumableOpen {
            inputs,
            first_cast,
            skill_ids,
            consumed,
            lanes,
        } = open;
        let Some(next_cast) = self.phase3.as_ref().map(|p| p.canonical.next_cast_id) else {
            return;
        };
        let Some(last_cast) = next_cast.checked_sub(1).filter(|last| *last >= first_cast) else {
            return;
        };
        let Some(items) = self.phase4.as_ref() else {
            return;
        };
        let Some(player) = items.canonical.players.get(&id) else {
            return;
        };
        let item_deadlines: serde_json::Map<_, _> = player
            .item_reuse
            .iter()
            .filter(|reuse| reuse.until > tick)
            .map(|reuse| {
                (Self::reuse_key_number(reuse.key).to_string(), serde_json::json!(reuse.until.0))
            })
            .collect();
        let Ok(side) = self.sidecar(id) else {
            return;
        };
        let skill_deadlines: serde_json::Map<_, _> = side
            .reuse
            .iter()
            .filter(|(skill, until)| skill_ids.contains(&skill.0) && **until > tick)
            .map(|(skill, until)| (skill.0.to_string(), serde_json::json!(until.0)))
            .collect();
        let actual = serde_json::json!({"accepted": true, "consumed": consumed, "lanes": lanes,
            "item_deadlines": item_deadlines, "skill_deadlines": skill_deadlines,
            "main_cast_preserved": lanes.iter().all(|lane| *lane == "item")});
        let escape = self.escape_timing_stage_v6(id, tick, &inputs, &skill_ids);
        if let Some(buffer) = self.evidence.as_mut() {
            // D92: one terminal fact per intent; an Escape scroll's is escape_timing.
            let pending = buffer.pending_cast.entry(last_cast).or_default();
            if let Some((escape_inputs, escape_actual)) = escape {
                pending.push((
                    id,
                    "escape_timing".into(),
                    resolution.to_owned(),
                    escape_inputs,
                    escape_actual,
                ));
            } else {
                pending.push((id, "consumable_use".into(), resolution.to_owned(), inputs, actual));
            }
        }
    }

    /// D46/D92 `escape_timing` for an item whose single skill is Escape: the definition
    /// record, start tick, casting speed, scroll availability, resources and incarnation at
    /// start; the actual is the engine's frozen cast schedule (D88 ticks) and debit.
    fn escape_timing_stage_v6(
        &self,
        id: EntityId,
        tick: Tick,
        consumable: &serde_json::Value,
        skill_ids: &[u32],
    ) -> Option<(serde_json::Value, serde_json::Value)> {
        use crate::domain::zone::state::item_evidence_hooks::ratio_json as r;
        let [skill_id] = skill_ids else {
            return None;
        };
        let rules = &self.phase3.as_ref()?.canonical.rules;
        let skill = rules.skills().get(SkillId(*skill_id))?;
        if !skill.effects.iter().any(|handler| handler.name == "Escape") {
            return None;
        }
        let definition = consumable.get("definition")?.clone();
        let template = definition.get("template_id")?.as_u64()?;
        let scroll_cost = definition
            .get("skills")?
            .get(0)?
            .get("item_consume_count")?
            .as_u64()?;
        let available = consumable
            .get("available")?
            .get(template.to_string())
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let side = self.sidecar(id).ok()?;
        let crate::domain::zone::CastState::Casting(cast) = &side.cast else {
            return None;
        };
        let entity = self.entities.get(&id)?;
        let combat = entity.combat.as_ref()?;
        let CombatRole::Player {
            xp,
            progression: Some(progression),
            ..
        } = &combat.role
        else {
            return None;
        };
        let effective = self.effective(id).ok()?;
        let exact = |whole: u32, remainder: i64| {
            ExactRatio::new(
                i128::from(whole)
                    .checked_mul(i128::from(Q))?
                    .checked_add(i128::from(remainder))?,
                i128::from(Q),
            )
            .ok()
        };
        let fractions = &side.fractions;
        let resources = serde_json::json!({"hp": r(exact(combat.hp, fractions.hp)?),
            "mp": r(exact(combat.mp, fractions.mp)?),
            "cp": r(exact(progression.class_state.cp, fractions.cp)?), "xp": xp});
        let animation = cast.animation.value().trunc().ok()?;
        let inputs = serde_json::json!({"definition": definition, "start_tick": tick.0,
            "casting_speed": effective.cast_speed, "available": available,
            "resources": resources, "incarnation": combat.incarnation});
        let actual = serde_json::json!({"accepted": true, "animation_ms": animation,
            "launch_ms": animation.saturating_sub(400).max(0), "launch_tick": cast.launch_at.0,
            "completion_tick": cast.complete_at.0, "interrupt_cutoff_tick": cast.interrupt_until.0,
            "consumed": scroll_cost, "completion_consumed": 0,
            "inventory": available.saturating_sub(scroll_cost), "resources": resources,
            "incarnation": combat.incarnation, "movement_canceled": true, "auto_canceled": true,
            "town_respawn": false, "draws": []});
        Some((inputs, actual))
    }
}
