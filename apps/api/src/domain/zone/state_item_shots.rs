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
        kind(&definition.handler).ok_or(RejectReason::ItemNotUsable)?;
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
        let before_toggle = player.auto_shots.clone();
        if let Some(enabled) = enabled {
            if enabled {
                player.auto_shots.insert(stack.template_id);
            } else {
                player.auto_shots.remove(&stack.template_id);
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
            refines: Vec::new(),
            paused_core: None,
            tick,
            entity: id,
            account,
            checkpoint,
            ledger: Vec::new(),
            receipts: Vec::new(),
            materializations: Vec::new(),
            evidence: Vec::new(),
        });
        let mut affected = std::collections::BTreeSet::new();
        let mut refusal = None;
        if enabled != Some(false) {
            let templates = if enabled == Some(true) {
                player.auto_shots.iter().copied().collect::<Vec<_>>()
            } else {
                vec![stack.template_id]
            };
            for template in templates {
                let selected = if enabled.is_none() {
                    Some(stack.clone())
                } else {
                    player
                        .inventory
                        .values()
                        .find(|item| {
                            item.template_id == template && item.location == ItemLocation::Inventory
                        })
                        .cloned()
                };
                let Some(selected) = selected else {
                    // D128: the toggle-enable `missing` removal carries its entry too.
                    if player.auto_shots.remove(&template) {
                        let requirement = shot_requirement(
                            &player,
                            &self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?.rules,
                            template,
                        );
                        projection.evidence.push(auto_shot_exhausted(
                            template,
                            None,
                            0,
                            "missing",
                            &format!(
                                "shot:{:?}:{}:{}",
                                reference.operation, reference.key, template
                            )
                            .to_lowercase(),
                            requirement,
                        ));
                    }
                    continue;
                };
                let resolution = if enabled.is_some() {
                    format!("shot:{:?}:{}:{}", reference.operation, reference.key, template)
                        .to_lowercase()
                } else {
                    format!("shot:{:?}:{}", reference.operation, reference.key).to_lowercase()
                };
                let (outcome, changed) = self.apply_shot_v6(
                    subject,
                    &mut player,
                    &selected,
                    InventoryBoundary {
                        tick,
                        revision,
                        origin: crate::domain::zone::ItemCreationOrigin {
                            zone: self.seed.zone,
                            epoch: self.seed.epoch,
                        },
                    },
                    Some(reference),
                    &resolution,
                    &mut projection,
                )?;
                affected.extend(changed);
                if enabled.is_none() {
                    refusal = match outcome {
                        ChargeOutcome::Unavailable | ChargeOutcome::WrongGrade => {
                            Some(RejectReason::ItemNotUsable)
                        },
                        ChargeOutcome::Insufficient => Some(RejectReason::InsufficientItems),
                        ChargeOutcome::AlreadyCharged | ChargeOutcome::Charged => None,
                    };
                }
            }
        }
        if before_toggle != player.auto_shots {
            player.inventory_revision = revision;
        }
        self.phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .players
            .insert(id, player);
        self.rebuild_v5(id).map_err(invalid)?;
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
            .map_err(invalid)?;
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
            .validate_staged(
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

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // Shared source attempt keeps draw, payment, charge and their durable causes together.
    pub(super) fn apply_shot_v6(
        &mut self,
        subject: crate::domain::zone::effects::TargetBinding,
        player: &mut crate::domain::zone::phase4::Phase4Player,
        stack: &crate::domain::zone::ItemInstance,
        boundary: InventoryBoundary,
        reference: Option<ItemReceiptRef>,
        resolution: &str,
        projection: &mut Phase4Projection,
    ) -> Result<
        (ChargeOutcome, std::collections::BTreeSet<crate::domain::zone::ItemInstanceId>),
        RejectReason,
    > {
        let InventoryBoundary { revision, .. } = boundary;
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let definition = items
            .rules
            .parts()
            .items
            .get(&stack.template_id)
            .ok_or(RejectReason::InvalidLoad)?
            .clone();
        let shot = kind(&definition.handler).ok_or(RejectReason::ItemNotUsable)?;
        let operation_ordinal = projection.next_operation_ordinal().map_err(invalid)?;
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
        let mut residue = None;
        let mut draw = None;
        let resolved = terms
            .resolve(|bound| {
                let before = self.rng.get_word_pos();
                let value = self.roll_below(bound);
                residue = Some(value);
                draw = Some((before, self.rng.get_word_pos(), bound, value));
                Ok(value)
            })
            .map_err(invalid)?;
        // D128: every disable path (automatic recharge, manual Use, toggle) records the
        // removal with the weapon's unreduced per-shot requirement.
        if resolved.disable_auto && player.auto_shots.remove(&stack.template_id) {
            projection.evidence.push(auto_shot_exhausted(
                stack.template_id,
                Some(stack.id),
                stack.count,
                "insufficient",
                resolution,
                weapon.as_ref().map(|weapon| (weapon.id, count)),
            ));
        }
        let mut changes = Vec::new();
        if resolved.outcome == ChargeOutcome::Charged {
            let result = resolved;
            let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
            let payment = InventoryMutation::prepare(
                &player.inventory,
                items.canonical.allocator,
                &[InventoryEdit::TakeInstance {
                    instance: stack.id,
                    count: u64::from(result.consumed),
                }],
                &items.rules,
                InventoryLimits::from_player(player),
                boundary,
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
                        resolution_id: resolution.to_owned(),
                    },
                );
            }
            {
                let result = resolved;
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
                    reference,
                    resolution,
                );
            }
        }
        let mut affected = std::collections::BTreeSet::new();
        for change in changes {
            let before = change.before.as_ref().ok_or(RejectReason::InvalidLoad)?;
            affected.insert(before.id);
            if before.id == stack.id {
                let count_delta = i64::from(resolved.consumed)
                    .checked_neg()
                    .ok_or(RejectReason::InvalidLoad)?;
                let mut fields = BTreeMap::from([
                    ("operation_ordinal".into(), V::Unsigned(u64::from(operation_ordinal))),
                    ("count_delta".into(), V::Signed(count_delta)),
                    ("resolution_id".into(), V::Text(resolution.to_owned())),
                ]);
                if let Some(reference) = reference {
                    fields.insert("receipt".into(), V::Text(reference.key.to_string()));
                }
                let cause = ItemAuditEvidence {
                    family: if let Some(reference) = reference {
                        if reference.operation == ItemOperation::ToggleAutoShot {
                            "toggle_shot_payment"
                        } else if reference.operation == ItemOperation::Use {
                            "manual_shot_payment"
                        } else {
                            return Err(RejectReason::InvalidLoad);
                        }
                    } else {
                        "auto_shot_payment"
                    }
                    .into(),
                    fields,
                };
                super::item_commands::append_item_consumption(
                    projection,
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
                    receipt: reference,
                    receipt_refs: Vec::new(),
                    causes: Vec::new(),
                });
            }
        }
        if let Some((before, after, bound, value)) = draw {
            projection.evidence.push(ItemAuditEvidence {
                family: "soulshot_reduction".into(),
                fields: BTreeMap::from([
                    ("resolution_id".into(), V::Text(resolution.to_owned())),
                    ("word_before".into(), V::Text(before.to_string())),
                    ("word_after".into(), V::Text(after.to_string())),
                    ("bound".into(), V::Unsigned(u64::from(bound))),
                    ("residue".into(), V::Unsigned(u64::from(value))),
                ]),
            });
        }
        if !affected.is_empty() {
            player.inventory_revision = revision;
        }
        Ok((resolved.outcome, affected))
    }
    /// I2 accepted-start recharge visits every retained template, including missing stacks.
    pub(super) fn recharge_cast_shots_v6(
        &mut self,
        subject: crate::domain::zone::effects::TargetBinding,
        skill: &crate::domain::zone::skill::SkillDef,
        cast: crate::domain::zone::CastId,
        boundary: InventoryBoundary,
        projection: &mut Phase4Projection,
    ) -> Result<(), RejectReason> {
        let (magic, physical) = crate::domain::zone::item_shots::skill_kinds(skill);
        self.recharge_shots_v6(
            subject,
            magic,
            physical,
            &format!("cast:{}", cast.0),
            boundary,
            projection,
        )
    }

    #[allow(clippy::too_many_arguments)] // Action eligibility and its settlement boundary.
    /// Shared I2 recharge: every retained toggle in template order, physical/magical
    /// eligibility from the starting action (L2PcInstance.java:6993 rechargeShots).
    pub(super) fn recharge_shots_v6(
        &mut self,
        subject: crate::domain::zone::effects::TargetBinding,
        magic: bool,
        physical: bool,
        action: &str,
        boundary: InventoryBoundary,
        projection: &mut Phase4Projection,
    ) -> Result<(), RejectReason> {
        let mut player = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .players
            .get(&subject.entity)
            .ok_or(RejectReason::InvalidLoad)?
            .clone();
        let before = player.auto_shots.clone();
        for template in before.iter().copied() {
            let resolution = format!("{action}:shot:{template}");
            let stack = player
                .inventory
                .values()
                .find(|item| {
                    item.template_id == template && item.location == ItemLocation::Inventory
                })
                .cloned();
            let Some(stack) = stack else {
                player.auto_shots.remove(&template);
                let requirement = shot_requirement(
                    &player,
                    &self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?.rules,
                    template,
                );
                projection.evidence.push(auto_shot_exhausted(
                    template,
                    None,
                    0,
                    "missing",
                    &resolution,
                    requirement,
                ));
                continue;
            };
            let definition = self
                .phase4
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .rules
                .parts()
                .items
                .get(&template)
                .ok_or(RejectReason::InvalidLoad)?;
            let enabled = match definition
                .properties
                .get("default_action")
                .map(String::as_str)
            {
                Some("SOULSHOT") => physical,
                Some("SPIRITSHOT") => magic,
                Some(_) | None => false,
            };
            if !enabled {
                continue;
            }
            self.apply_shot_v6(
                subject,
                &mut player,
                &stack,
                boundary,
                None,
                &resolution,
                projection,
            )?;
            // D127: an automatic charge that empties the stack disables the automatic shot in
            // the same tick, so the receipt-less auto_shots change carries its projection audit
            // evidence (an `auto_shot_exhausted` entry, not a ledger row).
            let remaining = player
                .inventory
                .values()
                .filter(|item| {
                    item.template_id == template && item.location == ItemLocation::Inventory
                })
                .try_fold(0_u64, |sum, item| sum.checked_add(item.count))
                .ok_or(RejectReason::InvalidLoad)?;
            // `insufficient` is recorded by apply_shot_v6 itself (D128).
            if remaining == 0 && player.auto_shots.remove(&template) {
                let requirement = shot_requirement(
                    &player,
                    &self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?.rules,
                    template,
                );
                projection.evidence.push(auto_shot_exhausted(
                    template,
                    Some(stack.id),
                    0,
                    "exhausted",
                    &resolution,
                    requirement,
                ));
            }
        }
        if before != player.auto_shots {
            player.inventory_revision = boundary.revision;
        }
        self.phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .players
            .insert(subject.entity, player);
        Ok(())
    }
}

/// D127/D128 audit evidence for an automatic-shot removal without its own toggle: the
/// template, the stack the charge inspected or consumed (absent when no stack was left), the
/// template's remaining count, why it stopped (`exhausted`: this tick's charge consumed the
/// stack to zero, and the stack's Consume row carries the `auto_shot_payment` cause with the
/// same resolution id; `insufficient`: fewer shots than one charge; `missing`: no stack at the
/// attempt), the attempt's resolution id, and, when a weapon is held, that weapon instance and
/// its unreduced per-shot requirement for the template's shot kind.
fn auto_shot_exhausted(
    template: u32,
    instance: Option<crate::domain::zone::ItemInstanceId>,
    remaining: u64,
    reason: &str,
    resolution: &str,
    requirement: Option<(crate::domain::zone::ItemInstanceId, u32)>,
) -> ItemAuditEvidence {
    let mut fields = BTreeMap::from([
        ("template".into(), V::Unsigned(u64::from(template))),
        ("remaining".into(), V::Unsigned(remaining)),
        ("reason".into(), V::Text(reason.to_owned())),
        ("resolution_id".into(), V::Text(resolution.to_owned())),
    ]);
    if let Some(instance) = instance {
        fields.insert("instance".into(), V::Unsigned(instance.get()));
    }
    if let Some((weapon, required)) = requirement {
        fields.insert("weapon".into(), V::Unsigned(weapon.get()));
        fields.insert("required".into(), V::Unsigned(u64::from(required)));
    }
    ItemAuditEvidence {
        family: "auto_shot_exhausted".into(),
        fields,
    }
}

/// The Rhand weapon and its unreduced per-shot count for `template`'s shot kind (D128).
fn shot_requirement(
    player: &crate::domain::zone::phase4::Phase4Player,
    rules: &crate::domain::zone::item_rules::Phase4Rules,
    template: u32,
) -> Option<(crate::domain::zone::ItemInstanceId, u32)> {
    let weapon = player
        .paperdoll
        .get(&EquipSlot::Rhand)
        .and_then(|id| player.inventory.get(id))?;
    let definition = rules.parts().items.get(&weapon.template_id)?;
    let shot = kind(&rules.parts().items.get(&template)?.handler)?;
    Some((
        weapon.id,
        if shot == ShotKind::Soul {
            definition.soulshots
        } else {
            definition.spiritshots
        },
    ))
}
