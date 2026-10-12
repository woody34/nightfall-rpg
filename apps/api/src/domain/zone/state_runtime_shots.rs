//! Frozen per-cast ownership of paid weapon charges.
use super::*;
use crate::domain::zone::item_inventory::InventoryChange;
use crate::domain::zone::phase4::{ChargeBinding, ItemActionBinding, ItemCastState, ShotKind};
use crate::domain::zone::phase4_projection::{
    ItemAuditEvidence, ItemDeltaPlan, ItemEvidenceValue as V, ItemOperation,
};
use crate::domain::zone::state_effects::CastWork;
use crate::domain::zone::{EquipSlot, ItemInstanceId, Phase4Projection};

impl ZoneState {
    pub(super) fn heal_shot_terms_v6(
        &self,
        caster: EntityId,
        skill: &SkillDef,
        rank: u32,
        shots: u8,
    ) -> Result<(u32, ExactRatio), TickError> {
        let Some(items) = &self.phase4 else {
            return Ok((2, ExactRatio::whole(0)));
        };
        let combat = self
            .entities
            .get(&caster)
            .and_then(|entity| entity.combat.as_ref())
            .ok_or_else(missing)?;
        let (mage, npc) = match &combat.role {
            CombatRole::Player {
                progression: Some(player),
                ..
            } => (
                self.classes
                    .as_ref()
                    .ok_or_else(missing)?
                    .get(player.class_state.current_class_id)
                    .ok_or_else(missing)?
                    .archetype
                    == crate::domain::zone::Archetype::Mystic,
                false,
            ),
            CombatRole::Npc { .. } => (false, true),
            CombatRole::Player {
                progression: None, ..
            } => return Err(missing()),
        };
        let grade = items
            .canonical
            .players
            .get(&caster)
            .and_then(|player| {
                player
                    .paperdoll
                    .get(&EquipSlot::Rhand)
                    .and_then(|id| player.inventory.get(id))
            })
            .and_then(|item| items.rules.parts().items.get(&item.template_id))
            .map(|item| item.grade.as_str());
        let mp = *skill
            .mp_consume_end
            .get(usize::try_from(rank.checked_sub(1).ok_or_else(missing)?).map_err(invariant)?)
            .ok_or_else(missing)?;
        crate::domain::zone::item_shots::heal_terms(
            mage,
            npc,
            grade,
            if skill.is_magic == 1 { shots } else { 0 },
            ExactRatio::whole(mp),
            skill.is_magic == 2,
        )
        .map_err(invariant)
    }

    pub(super) fn available_cast_shots_v6(
        &self,
        owner: EntityId,
        skill: &SkillDef,
    ) -> Result<(Option<ItemInstanceId>, u8), RejectReason> {
        let Some(items) = &self.phase4 else {
            return Ok((None, 0));
        };
        let Some(player) = items.canonical.players.get(&owner) else {
            return Ok((None, 0));
        };
        let Some(weapon) = player.paperdoll.get(&EquipSlot::Rhand) else {
            return Ok((None, 0));
        };
        let held = player
            .inventory
            .get(weapon)
            .ok_or(RejectReason::InvalidLoad)?;
        let (spirit, soul) = crate::domain::zone::item_shots::skill_kinds(skill);
        let mut available = held.charges;
        for binding in items
            .canonical
            .charge_bindings
            .iter()
            .filter(|binding| binding.weapon == *weapon)
        {
            available &= !binding.shot.bit();
        }
        let mask = (if soul { available & 1 } else { 0 })
            | if spirit {
                if available & 4 != 0 {
                    4
                } else {
                    available & 2
                }
            } else {
                0
            };
        Ok(((mask != 0).then_some(*weapon), mask))
    }

    pub(super) fn bind_cast_shots_v6(
        &mut self,
        context: ItemCastState,
    ) -> Result<(), RejectReason> {
        let items = &mut self
            .phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        if let Some(weapon) = context.weapon {
            for shot in [ShotKind::Soul, ShotKind::Spirit, ShotKind::BlessedSpirit] {
                if context.shots & shot.bit() != 0 {
                    if items
                        .charge_bindings
                        .iter()
                        .any(|binding| binding.weapon == weapon && binding.shot == shot)
                    {
                        return Err(RejectReason::InvalidLoad);
                    }
                    items.charge_bindings.push(ChargeBinding {
                        weapon,
                        shot,
                        action: ItemActionBinding::Cast {
                            id: context.cast.id,
                        },
                    });
                }
            }
            items
                .charge_bindings
                .sort_by_key(|binding| (binding.weapon, binding.shot));
        }
        items.item_casts.insert(context.cast.id, context);
        Ok(())
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // One actual charge transition and its private settlement fact.
    pub(super) fn discharge_cast_shots_v6(
        &mut self,
        tick: Tick,
        work: CastWork,
        skill: &SkillDef,
        used: bool,
        initial_revision: Option<u64>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let Some(context) = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.item_casts.get(&work.cast))
            .cloned()
        else {
            return Ok(());
        };
        let (spirit, soul) = crate::domain::zone::item_shots::skill_kinds(skill);
        let remaining =
            crate::domain::zone::item_shots::discharge(context.shots, used, spirit, soul)
                .map_err(invariant)?;
        let discharged = context.shots & !remaining;
        if discharged == 0 {
            return Ok(());
        }
        let weapon = context.weapon.ok_or_else(missing)?;
        let previous = events.iter().find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(projection) = event {
                (projection.entity == work.caster).then(|| projection.as_ref().clone())
            } else {
                None
            }
        });
        let items = self.phase4.as_mut().ok_or_else(missing)?;
        let player = items
            .canonical
            .players
            .get_mut(&work.caster)
            .ok_or_else(missing)?;
        let already_changed = initial_revision.map_or_else(
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
                .ok_or_else(missing)?
        };
        let item = player.inventory.get_mut(&weapon).ok_or_else(missing)?;
        if item.charges & discharged != discharged {
            return Err(missing());
        }
        let before = item.clone();
        item.charges &= !discharged;
        item.revision = revision;
        let change = InventoryChange {
            before: Some(before),
            after: Some(item.clone()),
        };
        player.inventory_revision = revision;
        let phase = &self.phase3.as_ref().ok_or_else(missing)?.canonical;
        let checkpoint = items
            .canonical
            .checkpoint(work.caster, tick, phase)
            .map_err(invariant)?;
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &self
            .entities
            .get(&work.caster)
            .and_then(|entity| entity.combat.as_ref())
            .ok_or_else(missing)?
            .role
        else {
            return Err(missing());
        };
        let account = progression.identity.account_id;
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
            exchanges: Vec::new(),
            enchant_attempts: Vec::new(),
            paused_core: None,
            tick,
            entity: work.caster,
            account,
            checkpoint: checkpoint.clone(),
            ledger: Vec::new(),
            receipts: Vec::new(),
            materializations: Vec::new(),
            evidence: Vec::new(),
        });
        projection.checkpoint = checkpoint;
        let ordinal = projection.next_operation_ordinal().map_err(invariant)?;
        let resolution = format!("cast:{}", work.cast.0);
        let cause = ItemAuditEvidence {
            family: "shot_discharge".into(),
            fields: BTreeMap::from([
                ("cast".into(), V::Unsigned(work.cast.0)),
                ("resolution_id".into(), V::Text(resolution.clone())),
                ("frozen_charges".into(), V::Unsigned(u64::from(context.shots))),
                ("discharged".into(), V::Unsigned(u64::from(discharged))),
            ]),
        };
        projection.evidence.push(cause.clone());
        projection.ledger.push(ItemDeltaPlan {
            operation: ItemOperation::Use,
            operation_ordinal: ordinal,
            instance: weapon,
            before: change.before.clone(),
            after: change.after.clone(),
            count_delta: 0,
            receipt: None,
            receipt_refs: Vec::new(),
            causes: vec![cause],
        });
        projection
            .validate_structure(
                phase
                    .checkpoint(work.caster, tick)
                    .map_err(invariant)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(invariant)?;
        self.item_evidence_delta(
            context.caster,
            &change,
            ItemOperation::Use,
            ordinal,
            None,
            &resolution,
        );
        events
            .retain(|event| !matches!(event,ZoneEvent::ItemCheckpoint(p) if p.entity==work.caster));
        events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        Ok(())
    }
}
