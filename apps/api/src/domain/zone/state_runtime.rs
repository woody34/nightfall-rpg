//! Live V5 integration. The legacy driver never calls these paths. Admission builds
//! canonical sidecars; casts use the same scheduler, formulas and resource owners as replay.

#[path = "state_runtime_damage.rs"]
mod live_damage;
#[path = "state_runtime_effects.rs"]
mod live_effects;
#[path = "state_runtime_item_cast.rs"]
mod live_item_cast;
#[path = "state_runtime_learn.rs"]
mod live_learning;
#[path = "state_runtime_life.rs"]
mod live_life;
#[path = "state_runtime_observe.rs"]
mod live_observe;
#[path = "state_runtime_shots.rs"]
mod live_shots;

use super::*;
use crate::domain::zone::effective_stats::{EffectiveStats, NpcStatInputs, PlayerStatInputs};
use crate::domain::zone::effects::{self, EffectList, EffectSourceKind, TargetBinding};
use crate::domain::zone::skill::SkillDef;
use crate::domain::zone::skill_math::{self, ExactRatio, MagicResolution};
use crate::domain::zone::stat_functions::FunctionConditions;
use crate::domain::zone::state_damage::{
    self, DamageCause, DamageMaxima, DamageOutcome, DamageReduction, DamageRequest,
    DamageResources, DamageRollSite,
};
use crate::domain::zone::state_pvp::{
    hostility_allowed, HostilityClassification, HostilityEndpoint, HostilityError,
};
use crate::domain::zone::state_regen::RegenSchedule;
use crate::domain::zone::state_skill::{
    self, CapturedTarget, CastCancelReason, CastError, CastStart, CastStatContext,
    ItemCommandTiming, ItemReceiptState, StagedAction, StagedActions, StagedCast, StagedItemUse,
};
use crate::domain::zone::{
    CastId, CastState, EntityOrdinal, Phase3Entity, Phase3EntityContext, Phase3Player, PvpState,
    ResourceFractions, RestoredModifierSource, SkillOutcome, SkillResolutionKind,
    SkillTargetResult, StatError, Q,
};

fn invariant<E>(_: E) -> TickError {
    TickError::Phase3Invariant
}
fn missing() -> TickError {
    TickError::Phase3Invariant
}
fn q(value: i64) -> Result<ExactRatio, StatError> {
    ExactRatio::new(i128::from(value), i128::from(Q))
}
// Every production caller uses the zone generator. Tests inject values at these same
// callsites while advancing that generator, exposing both ordering and the word cursor.
type DamageRoll<'a> =
    dyn FnMut(&mut ZoneState, DamageRollSite, TargetBinding, u32) -> Result<u32, StatError> + 'a;
fn draw_damage_v5(
    zone: &mut ZoneState,
    site: DamageRollSite,
    target: TargetBinding,
    bound: u32,
) -> Result<u32, StatError> {
    if bound == 0 {
        return Err(StatError::Overflow);
    }
    if zone.evidence.is_none() {
        return Ok(zone.roll_below(bound));
    }
    Ok(zone.evidence_roll(&format!("{site:?}"), Some(target), bound))
}
fn derive_effective_v5(
    stats: &StatRules,
    classes: &crate::domain::class::ClassRegistry,
    rules: &crate::domain::zone::Phase3Rules,
    entity: &Entity,
    sidecar: &Phase3Entity,
) -> Result<EffectiveStats, StatError> {
    let combat = entity.combat.as_ref().ok_or(StatError::Overflow)?;
    match &combat.role {
        CombatRole::Player {
            progression: Some(p),
            ..
        } => PlayerStatInputs::from_class(
            stats,
            classes,
            rules,
            p.class_state.current_class_id,
            combat.sheet.level(),
        )?
        .derive(sidecar, rules, true),
        CombatRole::Npc { template, .. } => {
            NpcStatInputs::from_profile(stats, rules, template)?.derive(sidecar, rules)
        },
        CombatRole::Player {
            progression: None, ..
        } => Err(StatError::Overflow),
    }
}
fn unsupported_player_pulse(
    resource: crate::domain::zone::effects::PeriodicResource,
    power: crate::domain::zone::effects::PeriodicPower,
) -> bool {
    resource == crate::domain::zone::effects::PeriodicResource::Hp
        && power.amount(Q).map_or(true, |amount| amount.parts().0 < 0)
}
fn cast_reject(error: CastError) -> RejectReason {
    match error {
        CastError::SkillUnknown => RejectReason::SkillUnknown,
        CastError::SkillNotLearned => RejectReason::SkillNotLearned,
        CastError::NotYetImplemented => RejectReason::NotYetImplemented,
        CastError::InvalidSkill => RejectReason::InvalidTargetType,
        CastError::DeadActor => RejectReason::DeadActor,
        CastError::SkillCooldown => RejectReason::SkillCooldown,
        CastError::InsufficientMp => RejectReason::InsufficientMp,
        CastError::InsufficientHp => RejectReason::InsufficientHp,
        CastError::AlreadyCasting => RejectReason::AlreadyCasting,
        CastError::Incapacitated => RejectReason::Incapacitated,
        CastError::Silenced => RejectReason::Silenced,
        CastError::Condition => RejectReason::WeaponCondition,
        CastError::InvalidState => RejectReason::InvalidLoad,
    }
}

#[allow(clippy::wildcard_enum_match_arm)] // Other target-validation refusals all invalidate the captured life.
fn target_cancel(reason: RejectReason) -> CastCancelReason {
    match reason {
        RejectReason::PeaceZone => CastCancelReason::PeaceZone,
        RejectReason::OutOfRange => CastCancelReason::OutOfRange,
        _ => CastCancelReason::TargetLost,
    }
}

impl ZoneState {
    pub(super) fn complete_runtime_v5(&self) -> bool {
        self.phase3
            .as_ref()
            .is_some_and(|p| p.canonical.rules.runtime_revision() >= 2)
    }
    pub(super) fn sidecar(&self, id: EntityId) -> Result<&Phase3Entity, TickError> {
        self.phase3
            .as_ref()
            .and_then(|p| p.canonical.entities.get(&id))
            .ok_or_else(missing)
    }
    fn sidecar_mut(&mut self, id: EntityId) -> Result<&mut Phase3Entity, TickError> {
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .schedule_dirty
            .insert(id);
        self.phase3
            .as_mut()
            .and_then(|p| p.canonical.entities.get_mut(&id))
            .ok_or_else(missing)
    }
    pub(super) fn binding(&self, id: EntityId) -> Option<TargetBinding> {
        self.entities
            .get(&id)?
            .combat
            .as_ref()
            .map(|c| TargetBinding {
                entity: id,
                incarnation: c.incarnation,
            })
    }
    fn effective_for(
        &self,
        e: &Entity,
        sidecar: &Phase3Entity,
    ) -> Result<EffectiveStats, StatError> {
        if let Some(items) = &self.phase4 {
            if e.kind == EntityKind::Player {
                return crate::domain::zone::item_modifiers::derive_player(
                    self.rules.as_deref().ok_or(StatError::Overflow)?,
                    self.classes.as_deref().ok_or(StatError::Overflow)?,
                    &self
                        .phase3
                        .as_ref()
                        .ok_or(StatError::Overflow)?
                        .canonical
                        .rules,
                    e,
                    sidecar,
                    crate::domain::zone::effective_stats::EquipmentStatContext {
                        player: items
                            .canonical
                            .players
                            .get(&e.id)
                            .ok_or(StatError::Overflow)?,
                        rules: &items.rules,
                    },
                );
            }
        }
        derive_effective_v5(
            self.rules.as_deref().ok_or(StatError::Overflow)?,
            self.classes.as_deref().ok_or(StatError::Overflow)?,
            &self
                .phase3
                .as_ref()
                .ok_or(StatError::Overflow)?
                .canonical
                .rules,
            e,
            sidecar,
        )
    }
    pub(super) fn effect_capacities_v6(
        &self,
        id: EntityId,
    ) -> Result<crate::domain::zone::effects::EffectCapacities, TickError> {
        use crate::domain::zone::effects::{EffectCapacities, StatusFlags};
        if self.phase4.is_none() {
            return Ok(EffectCapacities::for_enlarge(0));
        }
        let effective = self.effective(id)?;
        EffectCapacities::from_calculator(
            &effective.calculator,
            FunctionConditions {
                weapon_kind: self.item_weapon_kind(id, "SWORD")?,
                can_resurrect: !effective.flags.intersects(StatusFlags::RESURRECTION_BLOCK),
            },
        )
        .map_err(invariant)
    }
    pub(super) fn effective(&self, id: EntityId) -> Result<Arc<EffectiveStats>, TickError> {
        if let Some(cached) = self.phase3.as_ref().and_then(|p| p.effective.get(&id)) {
            return Ok(Arc::clone(cached));
        }
        self.effective_for(self.entities.get(&id).ok_or_else(missing)?, self.sidecar(id)?)
            .map(Arc::new)
            .map_err(invariant)
    }
    fn refresh_items_for(
        &self,
        id: EntityId,
    ) -> Result<Option<crate::domain::zone::item_refresh::RefreshedEquipment>, StatError> {
        let Some(items) = &self.phase4 else {
            return Ok(None);
        };
        let entity = self.entities.get(&id).ok_or(StatError::Overflow)?;
        if entity.kind != EntityKind::Player {
            return Ok(None);
        }
        let phase = self.phase3.as_ref().ok_or(StatError::Overflow)?;
        crate::domain::zone::item_refresh::refresh_player(
            self.rules.as_deref().ok_or(StatError::Overflow)?,
            self.classes.as_deref().ok_or(StatError::Overflow)?,
            &phase.canonical.rules,
            entity,
            phase
                .canonical
                .entities
                .get(&id)
                .ok_or(StatError::Overflow)?,
            crate::domain::zone::effective_stats::EquipmentStatContext {
                player: items
                    .canonical
                    .players
                    .get(&id)
                    .ok_or(StatError::Overflow)?,
                rules: &items.rules,
            },
            phase.canonical.next_install_sequence,
        )
        .map(Some)
    }
    pub(super) fn rebuild_v5(&mut self, id: EntityId) -> Result<(), TickError> {
        let before = self.evidence.as_ref().map(|_| {
            serde_json::json!({
                "entity": self.entities.get(&id), "word_cursor": self.rng.get_word_pos().to_string()
            })
        });
        let refreshed = self.refresh_items_for(id).map_err(invariant)?;
        let effective = Arc::new(if let Some(refreshed) = &refreshed {
            refreshed.effective.clone()
        } else {
            self.effective_for(self.entities.get(&id).ok_or_else(missing)?, self.sidecar(id)?)
                .map_err(invariant)?
        });
        let speed = effective.movement_speed().map_err(invariant)?;
        if let Some(refreshed) = refreshed {
            self.phase4
                .as_mut()
                .ok_or_else(missing)?
                .canonical
                .players
                .insert(id, refreshed.player);
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .canonical
                .next_install_sequence = refreshed.next_sequence;
        }
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .effective
            .insert(id, Arc::clone(&effective));
        if self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .is_some_and(|c| c.sheet.max_hp() != effective.physical.max_hp())
        {
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .vitals_dirty
                .insert(id);
        }
        let e = self.entities.get_mut(&id).ok_or_else(missing)?;
        let c = e.combat.as_mut().ok_or_else(missing)?;
        c.hp = c.hp.min(effective.physical.max_hp());
        c.mp = c.mp.min(effective.physical.max_mp());
        c.sheet = effective.physical;
        e.speed = speed;
        let health_full = c.hp == c.sheet.max_hp();
        let mana_full = c.mp == c.sheet.max_mp();
        let mut combat_points_full = true;
        if let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        {
            p.max_cp = effective.max_cp;
            p.class_state.cp = p.class_state.cp.min(p.max_cp);
            combat_points_full = p.class_state.cp == p.max_cp;
        }
        let fractions = &mut self.sidecar_mut(id)?.fractions;
        if health_full {
            fractions.hp = 0;
        }
        if mana_full {
            fractions.mp = 0;
        }
        if combat_points_full {
            fractions.cp = 0;
        }
        if let Some(before) = before {
            self.evidence_fact(
                "modifier_reconstruction",
                id,
                serde_json::json!({
                    "before": before, "after": self.evidence_context(id)?,
                    "word_cursor_after": self.rng.get_word_pos().to_string(),
                    "ordered_sources": self.sidecar(id)?.modifier_install_order.iter().collect::<Vec<_>>(),
                    "functions": effective.calculator
                }),
            );
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)] // Paused admission reconstructs once without replaying hooks.
    pub(super) fn attach_v5(
        &mut self,
        tick: Tick,
        id: EntityId,
        load: Option<&PlayerLoad>,
    ) -> Result<(), TickError> {
        let e = self.entities.get(&id).ok_or_else(missing)?;
        let Some(c) = &e.combat else { return Ok(()) };
        let player = e.kind == EntityKind::Player;
        let learned = match &c.role {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => p.class_state.learned_skills.as_slice(),
            CombatRole::Npc { .. } => &[],
            CombatRole::Player {
                progression: None, ..
            } => return Err(missing()),
        };
        let phase = self.phase3.as_mut().ok_or_else(missing)?;
        if phase.canonical.entities.contains_key(&id) {
            return Ok(());
        }
        let p = &mut phase.canonical;
        let mut sources = BTreeMap::new();
        let checkpoint = load.and_then(|load| load.phase3.as_ref());
        for skill in p.rules.skills().skills().values().filter(|s| {
            checkpoint.is_none()
                && s.operate_type == "P"
                && s.runtime_status == crate::domain::class::RuntimeSkillStatus::Executable
        }) {
            if let Some(rank) = learned.iter().find(|l| l.key == skill.key) {
                sources.insert(
                    p.next_install_sequence,
                    RestoredModifierSource::Passive(crate::domain::zone::skill::SkillRef {
                        id: skill.id,
                        rank: rank.level,
                    }),
                );
                p.next_install_sequence =
                    p.next_install_sequence.checked_add(1).ok_or_else(missing)?;
            }
        }
        let mut sidecar = Phase3Entity {
            ordinal: EntityOrdinal(p.next_entity_ordinal),
            cast: CastState::Idle,
            reuse: BTreeMap::new(),
            effects: EffectList::default(),
            modifier_install_order: sources,
            regen: RegenSchedule::start(tick, !e.targeting.dead, p.rules.regen())
                .map_err(invariant)?,
            fractions: ResourceFractions::default(),
            player: player.then_some(Phase3Player {
                pvp: PvpState {
                    flag_until: None,
                    karma: 0,
                    pk_count: 0,
                    pvp_count: 0,
                },
                death_penalty_level: 0,
                last_death: None,
                resurrect_offer: None,
            }),
        };
        if let Some(checkpoint) = checkpoint {
            let first_sequence = p.next_install_sequence;
            let mut restored = checkpoint
                .stage_restore(
                    &p.rules,
                    learned,
                    !e.targeting.dead,
                    c.incarnation,
                    tick,
                    p.next_effect_id,
                    p.next_install_sequence,
                )
                .map_err(invariant)?;
            if let Some(checkpoint) = load
                .and_then(|load| load.phase4.as_ref())
                .and_then(|items| items.checkpoint.as_ref())
            {
                let (player, timers) = crate::domain::zone::Phase4Player::restore(
                    checkpoint,
                    &restored,
                    tick,
                    first_sequence,
                )
                .map_err(invariant)?;
                restored = timers;
                self.phase4
                    .as_mut()
                    .ok_or_else(missing)?
                    .canonical
                    .players
                    .insert(id, player);
            }
            if self.phase4.is_none() {
                restored
                    .effects
                    .validate_capacity(crate::domain::zone::effects::EffectCapacities::for_enlarge(
                        0,
                    ))
                    .map_err(invariant)?;
            }
            p.next_effect_id = restored.next_effect_id;
            p.next_install_sequence = restored.next_install_sequence;
            sidecar.reuse = restored.reuse;
            sidecar.effects = restored.effects;
            sidecar.modifier_install_order = restored.modifier_install_order;
            sidecar.fractions = checkpoint.resource_fractions;
            sidecar.player = Some(Phase3Player {
                pvp: PvpState {
                    flag_until: restored.flag_until,
                    karma: checkpoint.karma,
                    pk_count: checkpoint.pk_count,
                    pvp_count: checkpoint.pvp_count,
                },
                death_penalty_level: checkpoint.death_penalty_level,
                // Offline admission allocates a new live incarnation. Retain exact XP
                // facts, but bind the resumable death to this corpse's new identity.
                last_death: checkpoint.last_death.clone().map(|mut death| {
                    death.incarnation = c.incarnation;
                    death
                }),
                resurrect_offer: None,
            });
        }
        p.next_entity_ordinal = p.next_entity_ordinal.checked_add(1).ok_or_else(missing)?;
        p.entities.insert(id, sidecar);
        let pending_kit = self.begin_pending_kit_v6(id, load)?;
        if let Some(items) = &mut self.phase4 {
            items
                .canonical
                .facing
                .insert(id, crate::domain::zone::item_angles::FacingVector::EAST);
            items
                .canonical
                .validate_allocations(
                    &items.rules,
                    &self.phase3.as_ref().ok_or_else(missing)?.canonical,
                )
                .map_err(invariant)?;
        }
        if let Some(refreshed) = self.refresh_items_for(id).map_err(invariant)? {
            let items = self.phase4.as_ref().ok_or_else(missing)?;
            let phase = self.phase3.as_ref().ok_or_else(missing)?;
            if !pending_kit
                && (items.canonical.players.get(&id) != Some(&refreshed.player)
                    || refreshed.next_sequence != phase.canonical.next_install_sequence)
            {
                return Err(missing());
            }
        }
        self.rebuild_v5(id)?;
        self.sidecar(id)?
            .effects
            .validate_capacity(self.effect_capacities_v6(id)?)
            .map_err(invariant)?;
        // Fresh unspecified resources mean the reconstructed maximum, not its effect-free base.
        if let Some(load) = load {
            let c = self
                .entities
                .get_mut(&id)
                .and_then(|e| e.combat.as_mut())
                .ok_or_else(missing)?;
            if load.alive && load.hp.is_none() {
                c.hp = c.sheet.max_hp();
            }
            if load.mp.is_none() {
                c.mp = c.sheet.max_mp();
            }
            self.rebuild_v5(id)?;
        }
        if player && self.phase4.is_some() {
            self.item_evidence_baseline(id)?;
        }
        if self.evidence.is_some() {
            self.evidence_fact(
                "admission_reconstruction",
                id,
                serde_json::json!({
                    "load": load, "after": self.evidence_context(id)?
                }),
            );
        }
        Ok(())
    }
    pub(super) fn validate_damage_profiles_v5(
        rules: &crate::domain::zone::Phase3Rules,
    ) -> Result<(), SnapshotError> {
        rules.npc().validate().map_err(SnapshotError::Rules)
    }
    #[allow(clippy::too_many_lines)] // Full context validation stays adjacent to reconstruction.
    pub(super) fn validate_runtime_v5(&self) -> Result<(), SnapshotError> {
        let phase = self.phase3.as_ref().ok_or(SnapshotError::CombatMismatch)?;
        Self::validate_damage_profiles_v5(&phase.canonical.rules)?;
        let contexts = self
            .entities
            .iter()
            .filter_map(|(id, e)| {
                e.combat.as_ref().map(|c| {
                    let learned = match &c.role {
                        CombatRole::Player {
                            progression: Some(p),
                            ..
                        } => p.class_state.learned_skills.as_slice(),
                        CombatRole::Player {
                            progression: None, ..
                        }
                        | CombatRole::Npc { .. } => &[],
                    };
                    (
                        *id,
                        Phase3EntityContext {
                            next_tick: self.next_tick,
                            incarnation: c.incarnation,
                            alive: !e.targeting.dead,
                            player: e.kind == EntityKind::Player,
                            learned,
                        },
                    )
                })
            })
            .collect();
        let item_skills = self
            .phase4
            .as_ref()
            .map(|items| {
                items
                    .canonical
                    .validate_casts(&items.rules, &phase.canonical, &contexts)
            })
            .transpose()
            .map_err(|_| SnapshotError::CombatMismatch)?
            .unwrap_or_default();
        phase
            .canonical
            .validate_live_with_item_casts(&contexts, &item_skills)
            .map_err(|_| SnapshotError::CombatMismatch)?;
        for (id, e) in &self.entities {
            let Some(c) = &e.combat else { continue };
            let side = phase
                .canonical
                .entities
                .get(id)
                .ok_or(SnapshotError::CombatMismatch)?;
            if side.effects.entries().values().any(|effect| {
                phase
                    .canonical
                    .rules
                    .skills()
                    .get(effect.skill.id)
                    .is_none_or(|skill| !supported_cast(skill, effect.skill.rank))
                    || (!self.complete_runtime_v5()
                        && e.kind == EntityKind::Player
                        && effect.handlers.iter().any(|h| {
                            h.periodic
                                .as_ref()
                                .is_some_and(|p| unsupported_player_pulse(p.resource, p.power))
                        }))
            }) || (c.swing.is_some() && !self.complete_runtime_v5())
                || (!self.complete_runtime_v5()
                    && side.player.as_ref().is_some_and(|p| {
                        p.resurrect_offer.is_some()
                            || p.pvp.flag_until.is_some()
                            || p.pvp.karma != 0
                            || p.pvp.pk_count != 0
                            || p.pvp.pvp_count != 0
                    }))
                || matches!(&side.cast, CastState::Casting(cast) if
                    e.kind != EntityKind::Player || phase.canonical.rules.skills().get(cast.skill.id).is_none_or(|skill|
                        !supported_cast(skill, cast.skill.rank) || (!self.complete_runtime_v5() && hostile_skill(skill) && cast.targets.iter().any(|target|
                            self.entities.get(&target.entity).is_some_and(|target_entity|
                                target_entity.kind == EntityKind::Player && self.binding(target.entity) == Some(*target))
                        ))
                    )
                )
            {
                return Err(SnapshotError::CombatMismatch);
            }
            let effective = self
                .effective_for(e, side)
                .map_err(|_| SnapshotError::CombatMismatch)?;
            if let Some(refreshed) = self
                .refresh_items_for(*id)
                .map_err(|_| SnapshotError::CombatMismatch)?
            {
                let player = self
                    .phase4
                    .as_ref()
                    .and_then(|items| items.canonical.players.get(id))
                    .ok_or(SnapshotError::CombatMismatch)?;
                if &refreshed.player != player
                    || refreshed.next_sequence != phase.canonical.next_install_sequence
                {
                    return Err(SnapshotError::CombatMismatch);
                }
            }
            if c.sheet != effective.physical
                || e.speed
                    != effective
                        .movement_speed()
                        .map_err(|_| SnapshotError::CombatMismatch)?
                || c.hp > c.sheet.max_hp()
                || c.mp > c.sheet.max_mp()
                || e.targeting.dead
                    != (c.hp == 0 && (self.phase4.is_none() || side.fractions.hp == 0))
            {
                return Err(SnapshotError::CombatMismatch);
            }
            if let CombatRole::Player {
                progression: Some(p),
                xp,
                class,
            } = &c.role
            {
                p.class_state
                    .validate_for_skill_rules(
                        self.classes
                            .as_deref()
                            .ok_or(SnapshotError::CombatMismatch)?,
                        &p.identity,
                        crate::domain::CharacterId::from_uuid(id.as_uuid()),
                        &phase.canonical.rules,
                    )
                    .map_err(|_| SnapshotError::CombatMismatch)?;
                let rules = self.rules.as_deref().ok_or(SnapshotError::CombatMismatch)?;
                let (_, _, _, radius) = crate::domain::zone::profession_stats(
                    rules,
                    self.classes
                        .as_deref()
                        .ok_or(SnapshotError::CombatMismatch)?,
                    &p.identity,
                    &p.class_state,
                    c.sheet.level(),
                )
                .map_err(|_| SnapshotError::CombatMismatch)?;
                if *xp > xp_cap(rules).map_err(|_| SnapshotError::CombatMismatch)?
                    || crate::domain::CharacterName::new(e.name.clone()).is_err()
                    || c.collision_radius != radius
                    || p.class_state.successful_transfer_receipts.iter().any(|r| {
                        r.result.character_id.as_uuid() != id.as_uuid()
                            || r.result.identity != p.identity
                            || r.result.current_class_id != r.target_class_id
                    })
                    || p.max_cp != effective.max_cp
                    || p.class_state.cp > p.max_cp
                    || self
                        .rules
                        .as_deref()
                        .is_none_or(|r| level_for_xp(r, *xp) != c.sheet.level())
                    || crate::domain::character_progression::base_class_profile(
                        p.identity.base_class_id,
                    ) != Some(class.as_str())
                {
                    return Err(SnapshotError::CombatMismatch);
                }
            }
        }
        Ok(())
    }
    pub(super) fn run_tick_v5(
        &mut self,
        draft: AppliedTickDraft,
    ) -> Result<AppliedTick, TickError> {
        self.check_draft(&draft)?;
        let mut candidate = self.clone();
        let applied = candidate.execute_v5(draft, self.phase4.as_ref().map(|p| &p.canonical))?;
        *self = candidate;
        Ok(applied)
    }
    #[allow(clippy::too_many_lines)] // Ordered live phase barriers remain visible in one driver.
    fn execute_v5(
        &mut self,
        draft: AppliedTickDraft,
        before_items: Option<&crate::domain::zone::Phase4Snapshot>,
    ) -> Result<AppliedTick, TickError> {
        let tick = draft.tick;
        // A restored snapshot carries only canonical inputs. Warm missing sheets once;
        // subsequent reads share them until a modifier, level or class mutation rebuilds.
        let cold: Vec<_> = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
            .entities
            .keys()
            .filter(|id| {
                !self
                    .phase3
                    .as_ref()
                    .is_some_and(|p| p.effective.contains_key(id))
            })
            .copied()
            .collect();
        for id in cold {
            self.rebuild_v5(id)?;
        }
        let before_policy: BTreeMap<_, _> = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
            .entities
            .iter()
            .filter_map(|(id, s)| s.player.as_ref().map(|p| (*id, p.pvp.clone())))
            .collect();
        let before = self.entities.clone();
        let known = self.known.clone();
        let mut events = Vec::new();
        let mut dispositions = Vec::new();
        let mut staged = StagedActions::default();
        for command in &draft.commands {
            let result = self.command_v5(tick, command, before_items, &mut staged, &mut events);
            if let Err(reason) = result {
                dispositions.push(Disposition {
                    ordinal: command.ordinal,
                    source: command.source,
                    seq: command.seq,
                    tick_seen: tick,
                    reason,
                });
            }
            self.next_ordinal = Ordinal(command.ordinal.0.checked_add(1).ok_or_else(missing)?);
        }
        self.spawn_phase_v5(tick, &mut events)?;
        self.expire_effects_v5(tick, &mut events)?;
        self.expire_reuse_v5(tick, &mut events)?;
        if self.complete_runtime_v5() {
            self.expire_policy_v5(tick, &mut events)?;
        }
        self.pulse_effects_v5(tick, &mut events)?;
        self.regen_v5(tick)?;
        if self.complete_runtime_v5() {
            self.chase(tick, &mut events);
        }
        events.extend(self.step(tick));
        // Binding V5 phases 5-9: completions, impacts, bookkeeping, starts, AI.
        // A due stun/lethal cast cancels a swing before any of its damage draws.
        self.complete_casts_v5(tick, before_items, &mut events)?;
        if self.complete_runtime_v5() {
            self.land_autos_v5(tick, before_items, &mut events)?;
            self.release_stale_attack_plans_v6();
        }
        self.note_hits_v5(tick)?;
        let mut item_receipts = std::mem::take(&mut staged.item_receipts);
        for entry in staged.drain() {
            let ordinal = entry.ordinal();
            if let Err(reason) =
                self.start_action_v5(tick, entry, before_items, &mut item_receipts, &mut events)
            {
                let c = draft
                    .commands
                    .iter()
                    .find(|c| c.ordinal == ordinal)
                    .ok_or_else(missing)?;
                dispositions.push(Disposition {
                    ordinal,
                    source: c.source,
                    seq: c.seq,
                    tick_seen: tick,
                    reason,
                });
            }
        }
        if self.complete_runtime_v5() {
            self.release_stale_attack_plans_v6();
            let first = events.len();
            self.start_swings(tick, &mut events);
            let started: Vec<_> = events
                .get(first..)
                .ok_or_else(missing)?
                .iter()
                .filter_map(|event| {
                    if let ZoneEvent::AttackStarted { attacker, .. } = event {
                        Some(*attacker)
                    } else {
                        None
                    }
                })
                .collect();
            for event in events.get(first..).ok_or_else(missing)? {
                if let ZoneEvent::AttackStarted {
                    attacker, target, ..
                } = event
                {
                    self.note_hostility_v5(
                        tick,
                        self.binding(*attacker).ok_or_else(missing)?,
                        self.binding(*target).ok_or_else(missing)?,
                    )?;
                }
            }
            if self.phase4.is_some() {
                for attacker in started {
                    let initial = before_items
                        .and_then(|items| items.players.get(&attacker))
                        .map(|player| player.inventory_revision);
                    self.start_attack_shots_v6(tick, attacker, initial, &mut events)?;
                }
            }
        }
        self.drain_ai_v5(tick, &mut events);
        self.ai_phase(tick, &mut events);
        self.drain_ai_v5(tick, &mut events);
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .forced_this_tick
            .clear();
        self.flush_progression(tick, &mut events);
        self.final_resources_v5(tick, &before, &mut events)?;
        if self.complete_runtime_v5() {
            for (id, side) in &self.phase3.as_ref().ok_or_else(missing)?.canonical.entities {
                if let Some(player) = &side.player {
                    if before_policy.get(id) != Some(&player.pvp) {
                        events.push(ZoneEvent::PvpStateChanged {
                            tick,
                            entity: *id,
                            incarnation: self.binding(*id).ok_or_else(missing)?.incarnation,
                            pvp: player.pvp.clone(),
                            baseline: (!before_policy.contains_key(id)).then_some(tick),
                        });
                    }
                }
            }
            for event in &mut events {
                if let ZoneEvent::TargetChanged { target, phase3, .. } = event {
                    *phase3 = Some(crate::domain::zone::TargetChangedPhase3 {
                        target_incarnation: target
                            .and_then(|id| self.binding(id).map(|b| b.incarnation)),
                        baseline: None,
                    });
                }
            }
        }
        self.final_items_v6(tick, before_items, &mut events)?;
        self.freeze_items_v6(tick, &mut events)?;
        self.freeze_learning_v5(tick, &mut events)?;
        self.freeze_transfers_v5(&mut events)?;
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .vitals_dirty
            .clear();
        dispositions.sort_by_key(|d| d.ordinal);
        let responses = responses(tick, &draft.commands, &dispositions);
        let outputs = if self.complete_runtime_v5() {
            self.observe_complete_v5(tick, responses, &events)?
        } else {
            self.observe_v5(tick, responses, &events, &known)?
        };
        self.next_tick = tick.next();
        self.refresh_schedule_v5()?;
        Ok(AppliedTick {
            epoch: draft.epoch,
            tick,
            server_time_ms: self.server_time_ms(tick),
            commands: draft.commands,
            dispositions,
            events,
            outputs,
            state_digest: self.state_digest(),
            digest_version: self.meta.digest_version,
        })
    }
    fn refresh_schedule_v5(&mut self) -> Result<(), TickError> {
        let phase = self.phase3.as_mut().ok_or_else(missing)?;
        for id in std::mem::take(&mut phase.schedule_dirty) {
            phase
                .schedule
                .refresh(id, phase.canonical.entities.get(&id))
                .map_err(invariant)?;
        }
        Ok(())
    }
    fn start_action_v5(
        &mut self,
        tick: Tick,
        action: StagedAction,
        before_items: Option<&crate::domain::zone::Phase4Snapshot>,
        item_receipts: &mut BTreeMap<
            (crate::domain::AccountId, crate::domain::zone::phase4_projection::ItemReceiptRef),
            ItemReceiptState,
        >,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        match action {
            StagedAction::Use(entry) => {
                if entry.canceled {
                    return Ok(());
                }
                if self.binding(entry.caster.entity) != Some(entry.caster) {
                    return Err(RejectReason::UnknownEntity);
                }
                self.item_command_at_v6(tick, &entry.command, entry.timing, events, item_receipts)
            },
            StagedAction::Cast(entry) => {
                if entry.canceled.is_some() {
                    Ok(())
                } else {
                    self.start_cast_v5(
                        tick,
                        &entry,
                        before_items
                            .and_then(|items| items.players.get(&entry.caster.entity))
                            .map(|p| p.inventory_revision),
                        events,
                    )
                }
            },
            StagedAction::Attack {
                caster,
                target,
                canceled,
                ..
            } => {
                if canceled {
                    return Ok(());
                }
                if self.binding(caster.entity) != Some(caster) {
                    return Err(RejectReason::UnknownEntity);
                }
                if self
                    .entities
                    .get(&caster.entity)
                    .is_some_and(|e| e.targeting.dead)
                {
                    return Err(RejectReason::DeadActor);
                }
                if self
                    .sidecar(caster.entity)
                    .is_ok_and(|s| matches!(s.cast, CastState::Casting(_)))
                {
                    return Err(RejectReason::AlreadyCasting);
                }
                if !self.auto_action_v5(caster.entity) {
                    return Err(RejectReason::Incapacitated);
                }
                let target = target
                    .and_then(|t| {
                        t.incarnation.map(|incarnation| TargetBinding {
                            entity: t.entity,
                            incarnation,
                        })
                    })
                    .ok_or(RejectReason::InvalidTargetType)?;
                self.cast_target_v5(tick, caster, target, -1, true, false, false)?;
                let c = self
                    .entities
                    .get_mut(&caster.entity)
                    .and_then(|e| e.combat.as_mut())
                    .ok_or(RejectReason::InvalidLoad)?;
                c.auto_attack = true;
                c.protected_until = None;
                self.note_hostility_v5(tick, caster, target)
                    .map_err(|_| RejectReason::InvalidLoad)
            },
            StagedAction::AcceptResurrect {
                target,
                offer_id,
                accept,
                present,
                ..
            } => {
                if !present || self.binding(target.entity) != Some(target) {
                    return Err(RejectReason::NoResurrectOffer);
                }
                events.extend(self.accept_resurrect_v5(tick, target.entity, offer_id, accept)?);
                Ok(())
            },
        }
    }

    /// L2Character.java:4595-4607: only playable -> attackable, never a player ledger.
    /// Positive effectPoint calls reduceHate, whose pinned implementation ADDS to an
    /// existing row (L2Attackable.java:658-669; AggroInfo.java:51-53), despite its name.
    fn effect_point_hate_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        point: i64,
        events: &mut Vec<ZoneEvent>,
    ) {
        if self
            .entities
            .get(&caster.entity)
            .is_none_or(|e| e.kind != EntityKind::Player)
            || self
                .entities
                .get(&target.entity)
                .is_none_or(|e| e.kind != EntityKind::Npc || !e.targeting.attackable)
            || point == 0
            || (point > 0
                && self
                    .hate
                    .get(&target.entity)
                    .and_then(|h| h.get(caster.entity))
                    .is_none())
        {
            return;
        }
        self.add_hate_for(
            tick,
            target.entity,
            caster.entity,
            super::super::HateEntry {
                hate: point.unsigned_abs(),
                damage: 0,
            },
            "effect_point",
            events,
        );
    }

    #[allow(clippy::too_many_lines)] // The command transaction retains explicit per-variant refusals.
    fn command_v5(
        &mut self,
        tick: Tick,
        command: &AppliedCommand,
        before_items: Option<&crate::domain::zone::Phase4Snapshot>,
        staged: &mut StagedActions,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        self.authorize(command.source, &command.command)?;
        if !matches!(
            command.command,
            ZoneCommand::CastSkill { .. }
                | ZoneCommand::Attack { .. }
                | ZoneCommand::AcceptResurrect { .. }
                | ZoneCommand::Item(_)
        ) {
            self.refuse_dead_actor(command.source, &command.command)?;
        }
        if let ZoneCommand::SpawnPlayer { entity, .. }
        | ZoneCommand::Despawn { entity }
        | ZoneCommand::ReplaceSession { entity, .. } = command.command
        {
            if events.iter().any(|event| matches!(event, ZoneEvent::SkillLearned { entity: learned, .. } | ZoneEvent::ClassTransfer { entity: learned, .. } if *learned == entity) || matches!(event, ZoneEvent::ItemCheckpoint(p) if p.entity == entity)) {
                return Err(RejectReason::InCombat);
            }
        }
        if let ZoneCommand::SpawnNpc {
            combat: Some(spec),
            speed,
            ..
        } = &command.command
        {
            if !self.npc_profile_v5(spec, *speed) {
                return Err(RejectReason::InvalidLoad);
            }
        }

        if let ZoneCommand::MoveTo { entity, .. } = &command.command {
            use crate::domain::zone::effects::StatusFlags as F;
            if self
                .effective(*entity)
                .map_err(|_| RejectReason::InvalidLoad)?
                .flags
                .intersects(F::STUN.union(F::SLEEP).union(F::PARALYZE).union(F::ROOT))
            {
                return Err(RejectReason::Incapacitated);
            }
        }
        match &command.command {
            ZoneCommand::SpawnPlayer { load, .. } => {
                self.validate_paused_load_v5(load.as_deref())?;
                events.extend(self.admit_v5(tick, command, load.as_deref())?);
            },
            ZoneCommand::SpawnNpc { .. } => {
                events.extend(self.admit_v5(tick, command, None)?);
            },
            ZoneCommand::CastSkill {
                entity,
                skill_id,
                target,
            } => {
                let caster = self.binding(*entity).ok_or(RejectReason::UnknownEntity)?;
                let target = target
                    .or_else(|| self.entities.get(entity).and_then(|e| e.targeting.target))
                    .map(|entity| CapturedTarget {
                        entity,
                        incarnation: self.binding(entity).map(|b| b.incarnation),
                    });
                staged
                    .stage(StagedCast {
                        ordinal: command.ordinal,
                        caster,
                        skill_id: *skill_id,
                        target,
                        canceled: None,
                    })
                    .map_err(cast_reject)?;
            },
            ZoneCommand::CancelCast { entity, cast_id } => {
                staged.cancel(*entity, *cast_id, CastCancelReason::Stopped);
                self.cancel_v5(tick, *entity, *cast_id, CastCancelReason::Stopped, events);
            },
            ZoneCommand::LearnSkill { .. } => self.learn_v5(tick, command, staged, events)?,
            ZoneCommand::ChangeClass { .. } if self.complete_runtime_v5() => {
                self.transfer_v5(tick, command, staged, events)?;
            },
            ZoneCommand::Respawn { entity } => events.extend(self.respawn_town_v5(tick, *entity)?),
            // Capability refusals belong to this command's recorded disposition. Holding
            // the whole draft would permanently block an experimental actor's FIFO.
            ZoneCommand::AcceptResurrect {
                entity,
                offer_id,
                accept,
            } if self.complete_runtime_v5() => {
                let target = self.binding(*entity).ok_or(RejectReason::UnknownEntity)?;
                let present = self
                    .sidecar(*entity)
                    .ok()
                    .and_then(|s| s.player.as_ref())
                    .and_then(|p| p.resurrect_offer.as_ref())
                    .is_some_and(|o| {
                        o.id == *offer_id && o.target_incarnation == target.incarnation
                    });
                staged
                    .stage_action(StagedAction::AcceptResurrect {
                        ordinal: command.ordinal,
                        target,
                        offer_id: *offer_id,
                        accept: *accept,
                        present,
                    })
                    .map_err(cast_reject)?;
            },
            ZoneCommand::Attack { entity } if self.complete_runtime_v5() => {
                let caster = self.binding(*entity).ok_or(RejectReason::UnknownEntity)?;
                let target = self
                    .entities
                    .get(entity)
                    .and_then(|e| e.targeting.target)
                    .map(|entity| CapturedTarget {
                        entity,
                        incarnation: self.binding(entity).map(|b| b.incarnation),
                    });
                staged
                    .stage_action(StagedAction::Attack {
                        ordinal: command.ordinal,
                        caster,
                        target,
                        canceled: false,
                    })
                    .map_err(cast_reject)?;
            },
            ZoneCommand::MaterializePendingItems(grant) => {
                self.grant_item_v6(tick, grant, events, &mut staged.item_receipts)?;
            },
            ZoneCommand::ExtendItemRange { range } => {
                self.phase4
                    .as_mut()
                    .ok_or(RejectReason::NotYetImplemented)?
                    .canonical
                    .allocator
                    .extend(*range)
                    .map_err(|_| RejectReason::InvalidLoad)?;
            },
            ZoneCommand::Item(item) => {
                let explicit =
                    if let crate::domain::zone::ItemCommandKind::Use { target, .. } = item.kind {
                        target
                    } else {
                        None
                    };
                let target = explicit
                    .map(|target| CapturedTarget {
                        entity: target.entity,
                        incarnation: Some(target.incarnation),
                    })
                    .or_else(|| {
                        self.entities
                            .get(&item.entity)
                            .and_then(|entity| entity.targeting.target)
                            .map(|entity| CapturedTarget {
                                entity,
                                incarnation: self
                                    .binding(entity)
                                    .map(|binding| binding.incarnation),
                            })
                    });
                let timing = ItemCommandTiming {
                    ordinal: command.ordinal,
                    accepted_sequence: command.seq,
                    target,
                    initial_inventory_revision: before_items
                        .and_then(|items| items.players.get(&item.entity))
                        .map(|player| player.inventory_revision),
                };
                if matches!(item.kind, crate::domain::zone::ItemCommandKind::Use { .. })
                    && !item.has_known_receipt()
                    && item
                        .receipt_conflict()
                        .map_err(|_| RejectReason::InvalidLoad)?
                        .is_none()
                {
                    self.item_account_v6(item)?;
                    let caster = self
                        .binding(item.entity)
                        .ok_or(RejectReason::UnknownEntity)?;
                    staged
                        .stage_action(StagedAction::Use(Box::new(StagedItemUse {
                            caster,
                            command: item.clone(),
                            timing,
                            canceled: false,
                        })))
                        .map_err(cast_reject)?;
                } else {
                    self.item_command_at_v6(tick, item, timing, events, &mut staged.item_receipts)?;
                }
            },
            ZoneCommand::AcceptResurrect { .. }
            | ZoneCommand::ChangeClass { .. }
            | ZoneCommand::Attack { .. }
            | ZoneCommand::AddAggro { .. } => return Err(RejectReason::NotYetImplemented),
            ZoneCommand::Despawn { .. }
            | ZoneCommand::ReplaceSession { .. }
            | ZoneCommand::MoveTo { .. }
            | ZoneCommand::SetTarget { .. }
            | ZoneCommand::StopAttack { .. }
            | ZoneCommand::StopMove { .. } => {
                let applied = self.apply(tick, command)?;
                events.extend(applied);
                match command.command {
                    ZoneCommand::MoveTo { entity, .. } | ZoneCommand::StopMove { entity } => {
                        staged.cancel_attacks(entity);
                        staged.cancel(entity, None, CastCancelReason::Moved);
                        self.cancel_v5(tick, entity, None, CastCancelReason::Moved, events);
                    },
                    ZoneCommand::SetTarget { entity, .. } | ZoneCommand::StopAttack { entity } => {
                        staged.cancel_attacks(entity);
                    },
                    ZoneCommand::Despawn { entity } => {
                        self.remove_items_v6(entity);
                        if let Some(p) = &mut self.phase3 {
                            if let Some(mut side) = p.canonical.entities.remove(&entity) {
                                if let Some(offer) = side.invalidate_resurrect_offer() {
                                    events.push(live_life::offer_event(
                                        tick,
                                        entity,
                                        &offer,
                                        crate::domain::zone::ResurrectOfferState::Expired,
                                    ));
                                }
                            }
                            p.effective.remove(&entity);
                            p.schedule_dirty.insert(entity);
                        }
                    },
                    ZoneCommand::CastSkill { .. }
                    | ZoneCommand::CancelCast { .. }
                    | ZoneCommand::AcceptResurrect { .. }
                    | ZoneCommand::LearnSkill { .. }
                    | ZoneCommand::Item(_)
                    | ZoneCommand::MaterializePendingItems(_)
                    | ZoneCommand::ExtendItemRange { .. }
                    | ZoneCommand::ChangeClass { .. }
                    | ZoneCommand::SpawnPlayer { .. }
                    | ZoneCommand::SpawnNpc { .. }
                    | ZoneCommand::ReplaceSession { .. }
                    | ZoneCommand::Attack { .. }
                    | ZoneCommand::Respawn { .. }
                    | ZoneCommand::AddAggro { .. } => {},
                }
            },
        }
        Ok(())
    }
    /// Admission is a command transaction: malformed paused state cannot leave a partial
    /// entity, consume allocation/RNG, or affect later commands in the same draft.
    fn admit_v5(
        &mut self,
        tick: Tick,
        command: &AppliedCommand,
        load: Option<&PlayerLoad>,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let mut candidate = self.clone();
        let mut events = candidate.apply(tick, command)?;
        let spawned: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let ZoneEvent::EntitySpawn { entity, .. } = event {
                    Some(*entity)
                } else {
                    None
                }
            })
            .collect();
        for entity in spawned {
            if candidate.phase4.is_some() {
                if let Some(load) = load {
                    candidate
                        .entities
                        .get_mut(&entity)
                        .ok_or(RejectReason::InvalidLoad)?
                        .targeting
                        .dead = !load.alive;
                    for event in &mut events {
                        if let ZoneEvent::EntitySpawn {
                            entity: id,
                            combat: Some(view),
                            ..
                        } = event
                        {
                            if *id == entity {
                                view.dead = !load.alive;
                            }
                        }
                    }
                }
            }
            candidate
                .attach_v5(tick, entity, load)
                .map_err(|_| RejectReason::InvalidLoad)?;
            candidate
                .materialize_kit_v6(tick, entity, load, &mut events)
                .map_err(|_| RejectReason::InvalidLoad)?;
        }
        *self = candidate;
        Ok(events)
    }
    pub(super) fn validate_paused_load_v5(
        &self,
        load: Option<&PlayerLoad>,
    ) -> Result<(), RejectReason> {
        if let Some(items) = &self.phase4 {
            let load = load.ok_or(RejectReason::InvalidLoad)?;
            load.validate_phase4()
                .map_err(|_| RejectReason::InvalidLoad)?;
            let retained = load.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
            self.validate_pending_kit_v6(load)
                .map_err(|_| RejectReason::InvalidLoad)?;
            if retained
                .checkpoint
                .as_ref()
                .is_some_and(|cp| cp.rules_hash != items.rules.hash())
            {
                return Err(RejectReason::InvalidLoad);
            }
        }
        let Some(load) = load else { return Ok(()) };
        let Some(checkpoint) = &load.phase3 else {
            return Ok(());
        };
        let phase = &self
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        let learned = &load
            .progression
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .class_state
            .learned_skills;
        checkpoint
            .validate(&phase.rules, learned, load.alive)
            .map_err(|_| RejectReason::InvalidLoad)?;
        // Same experimental boundary as snapshot restore. Persisted unsupported policy is
        // refused explicitly; none of its timers/scalars are silently dropped or defaulted.
        if (!self.complete_runtime_v5()
            && (checkpoint.karma != 0
                || checkpoint.pk_count != 0
                || checkpoint.pvp_count != 0
                || checkpoint.flag_remaining_ticks != 0))
            || checkpoint.effect_remainders.iter().any(|effect| {
                phase
                    .rules
                    .skills()
                    .get(effect.skill.id)
                    .is_none_or(|skill| !supported_cast(skill, effect.skill.rank))
                    || (!self.complete_runtime_v5()
                        && effect.handlers.iter().any(|h| {
                            h.periodic
                                .as_ref()
                                .is_some_and(|p| unsupported_player_pulse(p.resource, p.power))
                        }))
            })
        {
            return Err(RejectReason::NotYetImplemented);
        }
        Ok(())
    }
    fn cancel_v5(
        &mut self,
        tick: Tick,
        entity: EntityId,
        id: Option<CastId>,
        reason: CastCancelReason,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Ok(side) = self.sidecar_mut(entity) else {
            return;
        };
        if let Some(cast) = side.cancel_cast(id) {
            events.push(ZoneEvent::SkillCastCancelled {
                tick,
                caster: TargetBinding {
                    entity,
                    incarnation: cast.caster_incarnation,
                },
                cast_id: cast.id,
                reason,
            });
        }
        self.cancel_item_casts_v6(tick, entity, id, reason, events);
    }
    fn hostility_v5(
        &self,
        tick: Tick,
        source: TargetBinding,
        target: TargetBinding,
    ) -> Result<HostilityClassification, RejectReason> {
        let endpoint = |binding: TargetBinding| -> Result<HostilityEndpoint, RejectReason> {
            let e = self
                .entities
                .get(&binding.entity)
                .ok_or(RejectReason::UnknownEntity)?;
            let c = e.combat.as_ref().ok_or(RejectReason::NonAttackableTarget)?;
            Ok(HostilityEndpoint {
                binding: TargetBinding {
                    entity: e.id,
                    incarnation: c.incarnation,
                },
                kind: e.kind,
                pos: e.pos,
                alive: !e.targeting.dead,
                attackable: e.targeting.attackable,
                protected: c.protected_until.is_some_and(|until| until > tick),
            })
        };
        let peace = self
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .rules
            .geometry()
            .peace_square(self.bounds)
            .map_err(|_| RejectReason::InvalidLoad)?;
        hostility_allowed(endpoint(source)?, endpoint(target)?, source, target, peace).map_err(
            |e| match e {
                HostilityError::PeaceZone => RejectReason::PeaceZone,
                HostilityError::Protected => RejectReason::Protected,
                HostilityError::DeadActor => RejectReason::DeadActor,
                HostilityError::StaleLife | HostilityError::InvalidTarget => {
                    RejectReason::InvalidTargetType
                },
            },
        )
    }
    /// ConditionOpResurrection.java:40–51; external siege/pet modes are not admitted.
    pub(super) fn can_resurrect_target_v6(&self, target: TargetBinding) -> Result<bool, TickError> {
        if self.binding(target.entity) != Some(target)
            || self
                .entities
                .get(&target.entity)
                .is_none_or(|entity| !entity.targeting.dead || entity.kind != EntityKind::Player)
        {
            return Ok(false);
        }
        Ok(!self
            .effective(target.entity)?
            .flags
            .intersects(effects::StatusFlags::RESURRECTION_BLOCK)
            && self
                .sidecar(target.entity)?
                .player
                .as_ref()
                .is_some_and(|player| player.resurrect_offer.is_none()))
    }

    #[allow(clippy::too_many_arguments)] // Start/escape range rules share life/AOI/peace guards.
    fn cast_target_v5(
        &self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        range: i64,
        hostile: bool,
        escape: bool,
        corpse: bool,
    ) -> Result<(), RejectReason> {
        if hostile {
            self.hostility_v5(tick, caster, target)?;
        } else if self.binding(target.entity) != Some(target)
            || self.entities.get(&target.entity).is_none_or(|e| {
                if corpse {
                    !e.targeting.dead || e.kind != EntityKind::Player
                } else {
                    e.targeting.dead
                }
            })
        {
            return Err(RejectReason::InvalidTargetType);
        }
        let a = self
            .entities
            .get(&caster.entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let b = self
            .entities
            .get(&target.entity)
            .ok_or(RejectReason::UnknownEntity)?;
        if !self.aoi.in_aoi(a.pos).any(|id| id == b.id) {
            return Err(RejectReason::TargetNotInAoi);
        }
        if range >= 0 {
            // Skill ranges are L2 units; 32 L2 units = one tile. Cross multiply to avoid
            // a rounded radius admitting an out-of-range target.
            let dx = i128::from(a.pos.x.raw()).saturating_sub(i128::from(b.pos.x.raw()));
            let dy = i128::from(a.pos.y.raw()).saturating_sub(i128::from(b.pos.y.raw()));
            let distance = dx
                .checked_mul(dx)
                .and_then(|x| dy.checked_mul(dy).and_then(|y| x.checked_add(y)))
                .and_then(|v| v.checked_mul(1024))
                .ok_or(RejectReason::InvalidLoad)?;
            let collision = if escape {
                a.combat
                    .as_ref()
                    .map_or(0, |c| i128::from(c.collision_radius.raw()).saturating_mul(32))
            } else {
                0
            };
            let radius = i128::from(range)
                .checked_mul(1000)
                .and_then(|v| v.checked_add(collision))
                .and_then(|v| v.checked_mul(v))
                .ok_or(RejectReason::InvalidLoad)?;
            if distance > radius {
                return Err(RejectReason::OutOfRange);
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)] // Ordered validation, payment and publication.
    fn start_cast_v5(
        &mut self,
        tick: Tick,
        entry: &StagedCast,
        initial_revision: Option<u64>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let e = self
            .entities
            .get(&entry.caster.entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let c = e.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        if c.incarnation != entry.caster.incarnation || e.targeting.dead {
            return Err(RejectReason::DeadActor);
        }
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            return Err(RejectReason::NotAPlayer);
        };
        let phase = self.phase3.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let (skill, reference) = state_skill::learned_skill(
            phase.canonical.rules.skills(),
            &p.class_state.learned_skills,
            entry.skill_id,
        )
        .map_err(cast_reject)?;
        let has_auto_shots = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&entry.caster.entity))
            .is_some_and(|player| !player.auto_shots.is_empty());
        if self.phase4.is_some() && (!skill.item_consume.is_empty() || has_auto_shots) {
            return self.start_paid_skill_cast_v6(
                tick,
                entry,
                &skill.clone(),
                reference,
                initial_revision,
                events,
            );
        }
        self.start_defined_cast_v6(tick, entry, &skill.clone(), reference, None, None, events)
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // Shared validated start preserves learned and item lane ordering.
    pub(super) fn start_defined_cast_v6(
        &mut self,
        tick: Tick,
        entry: &StagedCast,
        skill: &SkillDef,
        reference: crate::domain::zone::skill::SkillRef,
        origin: Option<(crate::domain::zone::phase4::ItemCastOrigin, bool)>,
        item_payment: Option<(
            &mut crate::domain::zone::Phase4Projection,
            crate::domain::zone::item_inventory::InventoryBoundary,
        )>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let caster = self
            .entities
            .get(&entry.caster.entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let combat = caster.combat.as_ref().ok_or(RejectReason::NotAPlayer)?;
        if combat.incarnation != entry.caster.incarnation || caster.targeting.dead {
            return Err(RejectReason::DeadActor);
        }
        let simultaneous = origin
            .as_ref()
            .is_some_and(|(_, simultaneous)| *simultaneous);
        if !supported_cast(skill, reference.rank)
            || (!self.complete_runtime_v5()
                && skill.effects.iter().any(|h| h.name == "Resurrection"))
        {
            return Err(RejectReason::NotYetImplemented);
        }
        let target = if skill
            .target_type
            .at(reference.rank, skill.max_level)
            .is_some_and(|t| t == "SELF")
        {
            Some(entry.caster)
        } else {
            entry.target.and_then(|t| {
                t.incarnation.map(|incarnation| TargetBinding {
                    entity: t.entity,
                    incarnation,
                })
            })
        }
        .ok_or(RejectReason::InvalidTargetType)?;
        if skill.effects.iter().any(|handler| handler.name == "Escape")
            && (self.phase4.is_none() || self.safe_point.is_none() || target != entry.caster)
        {
            return Err(RejectReason::NotPermitted);
        }
        let point_blank = skill
            .affect_scope
            .at(reference.rank, skill.max_level)
            .is_some_and(|s| s == "POINT_BLANK");
        if !self.complete_runtime_v5()
            && !point_blank
            && hostile_skill(skill)
            && self
                .entities
                .get(&target.entity)
                .is_some_and(|e| e.kind == EntityKind::Player)
        {
            return Err(RejectReason::NotYetImplemented);
        }
        let range = *skill
            .cast_range
            .at(reference.rank, skill.max_level)
            .ok_or(RejectReason::InvalidLoad)?;
        self.cast_target_v5(
            tick,
            entry.caster,
            target,
            range,
            !point_blank && hostile_skill(skill),
            false,
            skill
                .target_type
                .at(reference.rank, skill.max_level)
                .is_some_and(|t| t == "PC_BODY"),
        )?;
        let effective = self
            .effective(entry.caster.entity)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let (mut weapon, mut shots) = self.available_cast_shots_v6(entry.caster.entity, skill)?;
        let mut terms = state_skill::cast_terms(
            skill,
            reference.rank,
            CastStatContext {
                world: self
                    .phase4
                    .as_ref()
                    .map(|_| state_skill::CastWorldConditions {
                        // The hash-validated test_zone has no L2 boss/instance zones; fear,
                        // duels, mounts, Olympiad and combat flags are not admitted in V6.
                        can_escape: caster.kind == EntityKind::Player,
                        inside_zone_ids: &[],
                    }),
                player_level: self.phase4.as_ref().map(|_| effective.physical.level()),
                calculator: &effective.calculator,
                conditions: FunctionConditions {
                    weapon_kind: self
                        .item_weapon_kind(entry.caster.entity, "SWORD")
                        .map_err(|_| RejectReason::InvalidLoad)?,
                    can_resurrect: self.phase4.is_none()
                        || self
                            .can_resurrect_target_v6(target)
                            .map_err(|_| RejectReason::InvalidLoad)?,
                },
                attack_speed: u32::try_from(
                    effective
                        .physical
                        .attack_speed()
                        .raw()
                        .checked_div(Q)
                        .ok_or(RejectReason::InvalidLoad)?,
                )
                .map_err(|_| RejectReason::InvalidLoad)?,
                cast_speed: effective.cast_speed,
                flags: effective.flags,
            },
        )
        .map_err(cast_reject)?;
        if skill.is_magic == 1 && shots & 6 != 0 {
            let hit_time = u32::try_from(
                *skill
                    .hit_time_ms
                    .at(reference.rank, skill.max_level)
                    .ok_or(RejectReason::InvalidLoad)?,
            )
            .map_err(|_| RejectReason::InvalidLoad)?;
            terms.animation = crate::domain::zone::effects::FrozenRatio::new(
                skill_math::cast_animation(hit_time, effective.cast_speed, false, true, true)
                    .map_err(|_| RejectReason::InvalidLoad)?,
            );
        }
        let mut side = self
            .sidecar(entry.caster.entity)
            .map_err(|_| RejectReason::InvalidLoad)?
            .clone();
        let original_main = side.cast.clone();
        if simultaneous {
            side.cast = CastState::Idle;
        }
        let phase = self.phase3.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let mut next_cast_id = phase.canonical.next_cast_id;
        let c = self
            .entities
            .get(&entry.caster.entity)
            .and_then(|e| e.combat.as_ref())
            .ok_or(RejectReason::InvalidLoad)?;
        let original_mp = c.mp;
        let original_next_id = next_cast_id;
        let hp = c.hp;
        let incarnation = c.incarnation;
        let unstarted_side = item_payment.as_ref().map(|_| side.clone());
        let mut mp = original_mp;
        let hp_remainder_q = if self.phase4.is_some() {
            side.fractions.hp
        } else {
            0
        };
        let mut start = CastStart {
            now: tick,
            ordinal: entry.ordinal,
            incarnation,
            terms,
            target: Some(target),
            affect_limit: 0,
            targets: Vec::new(),
        };
        let mut cast = side
            .begin_cast(&mut next_cast_id, &mut mp, hp, hp_remainder_q, start.clone())
            .map_err(cast_reject)?;
        if let Some((projection, boundary)) = item_payment {
            // Deterministic cast guards above precede the source's optional reduction draw.
            self.recharge_cast_shots_v6(entry.caster, skill, cast.id, boundary, projection)?;
            let available = self.available_cast_shots_v6(entry.caster.entity, skill)?;
            if available != (weapon, shots) {
                (weapon, shots) = available;
                if skill.is_magic == 1 && shots & 6 != 0 {
                    let hit_time = u32::try_from(
                        *skill
                            .hit_time_ms
                            .at(reference.rank, skill.max_level)
                            .ok_or(RejectReason::InvalidLoad)?,
                    )
                    .map_err(|_| RejectReason::InvalidLoad)?;
                    start.terms.animation = crate::domain::zone::effects::FrozenRatio::new(
                        skill_math::cast_animation(
                            hit_time,
                            effective.cast_speed,
                            false,
                            true,
                            true,
                        )
                        .map_err(|_| RejectReason::InvalidLoad)?,
                    );
                }
                side = unstarted_side.ok_or(RejectReason::InvalidLoad)?;
                next_cast_id = original_next_id;
                mp = original_mp;
                cast = side
                    .begin_cast(&mut next_cast_id, &mut mp, hp, hp_remainder_q, start)
                    .map_err(cast_reject)?;
            }
        }
        let (affect_limit, targets) =
            self.collect_targets_v5(tick, entry.caster, target, skill, reference.rank)?;
        cast.affect_limit = affect_limit;
        cast.targets = targets;
        side.cast = CastState::Casting(cast.clone());
        let launched = side.launch_cast(tick, cast.id, |_| true);
        let CastState::Casting(retained_cast) = &side.cast else {
            return Err(RejectReason::InvalidLoad);
        };
        let retained_cast = retained_cast.clone();
        if simultaneous {
            side.cast = original_main;
        }
        *self
            .sidecar_mut(entry.caster.entity)
            .map_err(|_| RejectReason::InvalidLoad)? = side;
        self.phase3
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .next_cast_id = next_cast_id;
        if origin.is_some() || shots != 0 {
            self.bind_cast_shots_v6(crate::domain::zone::phase4::ItemCastState {
                caster: entry.caster,
                cast: retained_cast,
                origin: origin.map(|(origin, _)| origin),
                simultaneous,
                weapon,
                shots,
            })?;
        }
        let c = self
            .entities
            .get_mut(&entry.caster.entity)
            .and_then(|e| e.combat.as_mut())
            .ok_or(RejectReason::InvalidLoad)?;
        c.mp = mp;
        if hostile_skill(skill) {
            c.protected_until = None;
        }
        if self.complete_runtime_v5() && !simultaneous {
            self.disengage(tick, entry.caster.entity, SwingCancel::Stopped, events);
        }
        if hostile_skill(skill) {
            for target in &cast.targets {
                self.note_hostility_v5(tick, entry.caster, *target)
                    .map_err(|_| RejectReason::InvalidLoad)?;
            }
        }
        if self.items_evidence_enabled() {
            let speed = if skill.is_magic == 1 {
                effective.cast_speed
            } else {
                u32::try_from(
                    effective
                        .physical
                        .attack_speed()
                        .raw()
                        .checked_div(Q)
                        .ok_or(RejectReason::InvalidLoad)?,
                )
                .map_err(|_| RejectReason::InvalidLoad)?
            };
            self.item_evidence(entry.caster, crate::domain::zone::evidence::ItemEvidenceData::Formula(serde_json::json!({
                "formula":"cast_time", "inputs":{"hit_time":skill.hit_time_ms.at(reference.rank,skill.max_level).ok_or(RejectReason::InvalidLoad)?,
                    "casting_speed":speed,"spirit":shots & 2 != 0,"blessed":shots & 4 != 0,"static":skill.is_magic==2,"is_magic":skill.is_magic==1},
                "actual":u64::try_from(cast.animation.value().trunc().map_err(|_| RejectReason::InvalidLoad)?).map_err(|_| RejectReason::InvalidLoad)?,
                "resolution_id":format!("cast:{}",cast.id.0),"baseline":false
            })));
        }
        events.push(cast_started(tick, entry.caster.entity, &cast, None));
        if cast.reuse_until > tick {
            events.push(ZoneEvent::SkillReuse {
                tick,
                entity: entry.caster.entity,
                incarnation: entry.caster.incarnation,
                skill_id: skill.id,
                ready_at: cast.reuse_until,
            });
        }
        if launched && cast.targets.is_empty() {
            self.cancel_v5(
                tick,
                entry.caster.entity,
                Some(cast.id),
                CastCancelReason::TargetLost,
                events,
            );
        }
        Ok(())
    }
    fn expire_reuse_v5(
        &mut self,
        tick: Tick,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let phase = self.phase3.as_mut().ok_or_else(missing)?;
        for (id, entity) in &mut phase.canonical.entities {
            let expired: Vec<_> = entity
                .reuse
                .iter()
                .filter(|(_, at)| **at <= tick)
                .map(|(id, _)| *id)
                .collect();
            for skill_id in expired {
                entity.reuse.remove(&skill_id);
                events.push(ZoneEvent::SkillReuse {
                    tick,
                    entity: *id,
                    incarnation: self
                        .entities
                        .get(id)
                        .and_then(|e| e.combat.as_ref())
                        .map_or(0, |c| c.incarnation),
                    skill_id,
                    ready_at: Tick(0),
                });
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)] // Source launch/payment/handler order kept in one driver.
    fn complete_casts_v5(
        &mut self,
        tick: Tick,
        before_items: Option<&crate::domain::zone::Phase4Snapshot>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        self.refresh_schedule_v5()?;
        let launches = self.cast_work_v6(tick, true)?;
        for work in launches {
            let Some(cast) = self.cast_for_work_v6(work)? else {
                continue;
            };
            let rules = &self.phase3.as_ref().ok_or_else(missing)?.canonical.rules;
            let skill = rules.skills().get(cast.skill.id).ok_or_else(missing)?;
            let range = effect_range_v5(skill, cast.skill.rank)?;
            let caster = TargetBinding {
                entity: work.caster,
                incarnation: cast.caster_incarnation,
            };
            let (targets, reason) = self.filter_targets_v5(
                tick,
                caster,
                &cast.targets,
                range,
                hostile_skill(skill),
                skill
                    .target_type
                    .at(cast.skill.rank, skill.max_level)
                    .is_some_and(|t| t == "PC_BODY"),
            );
            if targets.is_empty() {
                self.cancel_v5(tick, work.caster, Some(cast.id), target_cancel(reason), events);
                continue;
            }
            self.launch_cast_work_v6(tick, work, &targets)?;
        }
        self.refresh_schedule_v5()?;
        let impacts = self.cast_work_v6(tick, false)?;
        for work in impacts {
            let Some(cast) = self.cast_for_work_v6(work)? else {
                continue;
            };
            let skill = self
                .phase3
                .as_ref()
                .ok_or_else(missing)?
                .canonical
                .rules
                .skills()
                .get(cast.skill.id)
                .ok_or_else(missing)?
                .clone();
            let caster = TargetBinding {
                entity: work.caster,
                incarnation: cast.caster_incarnation,
            };
            let range = effect_range_v5(&skill, cast.skill.rank)?;
            let (targets, reason) = self.filter_targets_v5(
                tick,
                caster,
                &cast.targets,
                range,
                hostile_skill(&skill),
                skill
                    .target_type
                    .at(cast.skill.rank, skill.max_level)
                    .is_some_and(|t| t == "PC_BODY"),
            );
            if targets.is_empty() {
                self.cancel_v5(tick, work.caster, Some(cast.id), target_cancel(reason), events);
                continue;
            }
            // L2Character.java:4600-4606: skill effectPoint is applied once per
            // eligible target before impact costs; it is separate from damage hate.
            let effect_point = *skill
                .effect_point
                .get(usize::try_from(cast.skill.rank.saturating_sub(1)).map_err(invariant)?)
                .ok_or_else(missing)?;
            for target in &targets {
                self.effect_point_hate_v5(tick, caster, *target, effect_point, events);
            }
            let stats = self.effective(work.caster)?;
            let (_, mp, hp) = state_skill::cast_costs(
                &skill,
                cast.skill.rank,
                CastStatContext {
                    world: None,
                    player_level: self.phase4.as_ref().map(|_| stats.physical.level()),
                    calculator: &stats.calculator,
                    conditions: FunctionConditions {
                        weapon_kind: self.item_weapon_kind(work.caster, "SWORD")?,
                        can_resurrect: true,
                    },
                    attack_speed: 1,
                    cast_speed: stats.cast_speed,
                    flags: stats.flags,
                },
            )
            .map_err(invariant)?;
            match self.pay_cast_work_v6(tick, work, mp, hp)? {
                Err(reason) => {
                    events.push(ZoneEvent::SkillCastCancelled {
                        tick,
                        caster,
                        cast_id: cast.id,
                        reason,
                    });
                    continue;
                },
                Ok(None) => continue,
                Ok(Some(_)) => {},
            }
            let mut results = Vec::new();
            for target in targets {
                if hostile_skill(&skill) {
                    self.note_hostility_v5(tick, caster, target)?;
                }
                self.impact_target_v5(
                    tick,
                    caster,
                    target,
                    &skill,
                    cast.skill.rank,
                    cast.id,
                    events,
                    &mut results,
                )?;
            }
            self.discharge_cast_shots_v6(
                tick,
                work,
                &skill,
                !results.is_empty(),
                before_items
                    .and_then(|items| items.players.get(&work.caster))
                    .map(|player| player.inventory_revision),
                events,
            )?;
            self.finish_cast_work_v6(work)?;
            events.push(ZoneEvent::SkillCastResult {
                tick,
                caster,
                cast_id: cast.id,
                skill: cast.skill,
                targets: results,
            });
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn impact_target_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        cast_id: CastId,
        events: &mut Vec<ZoneEvent>,
        results: &mut Vec<SkillTargetResult>,
    ) -> Result<(), TickError> {
        let mark = self.evidence_mark();
        let before = if self.evidence.is_some() {
            serde_json::json!({"caster": self.evidence_context(caster.entity)?,
                "target": self.evidence_context(target.entity)?, "skill": skill, "rank": rank})
        } else {
            serde_json::Value::Null
        };
        let shots = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.item_casts.get(&cast_id))
            .map_or(0, |context| context.shots);
        let result_start = results.len();
        let continuous = self.continuous_terms_v5(caster, target, skill, rank)?;

        for handler in skill
            .effects
            .iter()
            .filter(|h| !crate::domain::zone::effects::continuous_handler(&h.name))
        {
            if handler.name == "Escape" {
                results.push(self.escape_v6(tick, target, events)?);
                continue;
            }
            if handler.name == "Resurrection" {
                results.push(self.resurrect_offer_v5(
                    tick,
                    caster,
                    target,
                    crate::domain::zone::skill::SkillRef { id: skill.id, rank },
                    events,
                )?);
                continue;
            }
            if handler.name == "FatalBlow" {
                results.push(
                    self.blow_handler_v6(
                        tick, caster, target, skill, rank, handler, shots, events,
                    )?,
                );
                if self
                    .entities
                    .get(&target.entity)
                    .is_none_or(|e| e.targeting.dead)
                {
                    break;
                }
                continue;
            }
            let resolved = match handler.name.as_str() {
                "Lethal" => {
                    self.lethal_handler_v6(tick, caster, target, skill, rank, handler, events)
                },
                "DispelBySlot" | "DispelByCategory" => {
                    self.dispel_handler_v5(tick, caster, target, skill, rank, handler, events)
                },
                "Heal" | "Hp" | "HpPerMax" | "Mp" | "Cp" => self
                    .restore_handler_v5(tick, caster, target, skill, rank, handler, shots, events),
                "GetAgro" | "AddHate" => {
                    self.agro_handler_v5(tick, caster, target, skill, rank, handler, events)
                },
                "PhysicalAttack" => self
                    .physical_handler_v5(tick, caster, target, skill, rank, handler, shots, events),
                _ => {
                    self.magic_handler_v5(tick, caster, target, skill, rank, handler, shots, events)
                },
            }?;
            results.push(resolved);
            if self
                .entities
                .get(&target.entity)
                .is_none_or(|e| e.targeting.dead)
            {
                break;
            }
        }
        if let Some((success, duration)) = continuous {
            if success
                && self
                    .entities
                    .get(&target.entity)
                    .is_some_and(|e| !e.targeting.dead)
            {
                results.push(
                    self.install_effect_v5(tick, caster, target, skill, rank, duration, events)?,
                );
            } else {
                results.push(SkillTargetResult {
                    target,
                    resolution_kind: SkillResolutionKind::Effect,
                    outcome: SkillOutcome::Resist,
                    rolled: 0,
                    cp_absorbed: 0,
                    hp_removed: 0,
                    heal: 0,
                    target_hp_after: self
                        .entities
                        .get(&target.entity)
                        .and_then(|e| e.combat.as_ref())
                        .ok_or_else(missing)?
                        .hp,
                    applied_effect_ids: Vec::new(),
                });
            }
        }
        if self.evidence.is_some() {
            self.evidence_fact(
                "formula_resolution",
                caster.entity,
                serde_json::json!({
                    "inputs": before, "target": target, "skill_id": skill.id, "rank": rank,
                    "continuous": continuous,
                    "outcomes": results.get(result_start..).ok_or_else(missing)?,
                    "rng_draws": self.evidence_draws(mark),
                    "after": self.evidence_context(target.entity)?
                }),
            );
        }
        Ok(())
    }
    /// Capture a live source once; periodic callers instead retain the effect's attribution.
    fn damage_source_v5(&self, source: TargetBinding) -> Result<effects::EffectSource, TickError> {
        if self.binding(source.entity) != Some(source) {
            return Err(missing());
        }
        Ok(effects::EffectSource {
            binding: Some(source),
            kind: match self.entities.get(&source.entity).ok_or_else(missing)?.kind {
                EntityKind::Player => EffectSourceKind::Player,
                EntityKind::Npc => EffectSourceKind::Npc,
            },
        })
    }

    fn damage_classification_v5(
        &self,
        tick: Tick,
        request: DamageRequest,
    ) -> Result<HostilityClassification, TickError> {
        let source = request.source.binding.ok_or_else(missing)?;
        let live = self.binding(source.entity) == Some(source)
            && self
                .entities
                .get(&source.entity)
                .is_some_and(|e| !e.targeting.dead);
        if live {
            if self.damage_source_v5(source)? != request.source {
                return Err(missing());
            }
            return self
                .hostility_v5(tick, source, request.target)
                .map_err(invariant);
        }
        if request.cause != DamageCause::Periodic || request.source.kind == EffectSourceKind::System
        {
            return Err(missing());
        }
        let victim = self
            .entities
            .get(&request.target.entity)
            .ok_or_else(missing)?;
        let combat = victim.combat.as_ref().ok_or_else(missing)?;
        let peace = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
            .rules
            .geometry()
            .peace_square(self.bounds)
            .map_err(invariant)?;
        if self.binding(victim.id) != Some(request.target)
            || victim.targeting.dead
            || combat.protected_at(tick)
            || peace.contains(victim.pos)
            || (victim.kind == EntityKind::Npc
                && (!victim.targeting.attackable || request.source.kind == EffectSourceKind::Npc))
            || self
                .entities
                .get(&source.entity)
                .filter(|_| self.binding(source.entity) == Some(source))
                .is_some_and(|e| peace.contains(e.pos))
        {
            return Err(missing());
        }
        Ok(HostilityClassification {
            absorb_cp: request.source.kind == EffectSourceKind::Player
                && victim.kind == EntityKind::Player,
        })
    }

    /// L2Attackable.java:232-274,556-572: `EVT_ATTACKED` then damage hate precede HP.
    /// L2AttackableAI.java:1856-1871: even zero damage refreshes timeout and adds one hate.
    fn damage_hate_v5(
        &mut self,
        tick: Tick,
        request: DamageRequest,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let source = request.source.binding.ok_or_else(missing)?;
        let victim = self
            .entities
            .get(&request.target.entity)
            .ok_or_else(missing)?;
        if victim.kind != EntityKind::Npc
            || self.binding(source.entity) != Some(source)
            || self
                .entities
                .get(&source.entity)
                .is_none_or(|e| e.targeting.dead)
        {
            return Ok(());
        }
        let level = victim.combat.as_ref().ok_or_else(missing)?.sheet.level();
        let damage = if request.cause == DamageCause::Periodic {
            request.rolled_damage.min(
                victim
                    .combat
                    .as_ref()
                    .ok_or_else(missing)?
                    .hp
                    .saturating_sub(1),
            )
        } else {
            request.rolled_damage
        };
        if let Some(ai) = self
            .entities
            .get_mut(&request.target.entity)
            .and_then(|e| e.ai.as_mut())
        {
            ai.last_hit = tick;
        }
        self.add_hate_for(
            tick,
            request.target.entity,
            source.entity,
            super::super::HateEntry { hate: 1, damage: 0 },
            "attacked",
            events,
        );
        let hate = super::super::combat_math::damage_hate(
            self.rules.as_ref().ok_or_else(missing)?.constants(),
            damage,
            level,
        )
        .map_err(invariant)?;
        self.add_hate_for(
            tick,
            request.target.entity,
            source.entity,
            super::super::HateEntry {
                // L2Attackable.java:606-621 adds one more hate when computed aggro is zero.
                hate: hate.max(1),
                damage: u64::from(damage),
            },
            "damage",
            events,
        );
        Ok(())
    }

    fn apply_damage_v5(
        &mut self,
        tick: Tick,
        request: DamageRequest,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<DamageOutcome, TickError> {
        self.apply_damage_rolled_v5(tick, request, events, &mut draw_damage_v5)
    }

    #[allow(clippy::too_many_lines)] // One recipient transaction preserves status/RNG/death order.
    fn apply_damage_rolled_v5(
        &mut self,
        tick: Tick,
        request: DamageRequest,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<DamageOutcome, TickError> {
        let DamageRequest {
            source: attribution,
            target,
            rolled_damage,
            cause,
        } = request;
        let source = attribution.binding.ok_or_else(missing)?;
        let classification = self.damage_classification_v5(tick, request)?;
        let recipient = self
            .entities
            .get(&target.entity)
            .ok_or_else(missing)?
            .clone();
        let c = recipient.combat.as_ref().ok_or_else(missing)?;
        let cp = match &c.role {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => Some(p.class_state.cp),
            CombatRole::Player {
                progression: None, ..
            }
            | CombatRole::Npc { .. } => None,
        };
        let side = self.sidecar(target.entity)?.clone();
        let rules = Arc::clone(&self.phase3.as_ref().ok_or_else(missing)?.canonical.rules);
        let stats = Arc::clone(self.rules.as_ref().ok_or_else(missing)?);
        let classes = Arc::clone(self.classes.as_ref().ok_or_else(missing)?);
        let equipment = self.phase4.as_ref().map(|items| {
            (items.canonical.players.get(&target.entity).cloned(), Arc::clone(&items.rules))
        });
        let mutation = state_damage::prepare_hp_reduction(
            &side,
            DamageResources {
                hp_remainder_q: self.phase4.as_ref().map(|_| side.fractions.hp),
                hp: c.hp,
                mp: c.mp,
                cp,
            },
            DamageReduction {
                cause,
                source_kind: attribution.kind,
                classification,
                awake: true,
                rolled_damage,
            },
            &rules,
            |bound| draw(self, DamageRollSite::StunRemoval, target, bound),
            |side| {
                let effective = if let Some((player, items)) = &equipment {
                    if recipient.kind == EntityKind::Player {
                        crate::domain::zone::item_modifiers::derive_player(
                            &stats,
                            &classes,
                            &rules,
                            &recipient,
                            side,
                            crate::domain::zone::effective_stats::EquipmentStatContext {
                                player: player.as_ref().ok_or(StatError::Overflow)?,
                                rules: items,
                            },
                        )?
                    } else {
                        derive_effective_v5(&stats, &classes, &rules, &recipient, side)?
                    }
                } else {
                    derive_effective_v5(&stats, &classes, &rules, &recipient, side)?
                };
                Ok(DamageMaxima {
                    hp: effective.physical.max_hp(),
                    mp: effective.physical.max_mp(),
                    cp: cp.map(|_| effective.max_cp),
                })
            },
        )
        .map_err(invariant)?;
        // Player-kill counters and their public PvpStateChanged projection remain a
        // coordinated capability boundary. NPC-sourced player death is complete here.
        if !self.complete_runtime_v5()
            && mutation.lethal
            && cp.is_some()
            && attribution.kind == EffectSourceKind::Player
        {
            return Err(missing());
        }
        self.note_hostility_v5(tick, source, target)?;
        self.damage_hate_v5(tick, request, events)?;
        if mutation.hp_removed > 0 || mutation.lethal {
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .vitals_dirty
                .insert(target.entity);
        }
        *self.sidecar_mut(target.entity)? = mutation.phase3;
        for old in &mutation.removed_effects {
            events.push(ZoneEvent::EffectRemoved {
                tick,
                target,
                effect_id: old.id,
                reason: crate::domain::zone::effects::EffectRemoveReason::Damage,
            });
        }
        if !mutation.removed_effects.is_empty() || !self.complete_runtime_v5() {
            self.rebuild_v5(target.entity)?;
        }
        let c = self
            .entities
            .get_mut(&target.entity)
            .and_then(|e| e.combat.as_mut())
            .ok_or_else(missing)?;
        c.hp = mutation.resources.hp;
        c.mp = mutation.resources.mp;
        if let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut c.role
        {
            player.class_state.cp = mutation.resources.cp.ok_or_else(missing)?;
        }
        let hp = c.hp;
        if mutation.lethal {
            self.finish_death_v5(tick, request, &recipient, events, draw)?;
        }
        if self.binding(source.entity) == Some(source) {
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .landed_hits
                .insert((source.entity, source.incarnation));
        }
        Ok(DamageOutcome {
            cp_absorbed: mutation.cp_absorbed,
            hp_removed: mutation.hp_removed,
            hp_after: hp,
            lethal: mutation.lethal,
        })
    }
    /// Phase seven consumes each landed source life once, independently of public rows.
    fn note_hits_v5(&mut self, tick: Tick) -> Result<(), TickError> {
        let hits = std::mem::take(&mut self.phase3.as_mut().ok_or_else(missing)?.landed_hits);
        for (entity, incarnation) in hits {
            let source = TargetBinding {
                entity,
                incarnation,
            };
            if self.binding(source.entity) != Some(source) {
                continue;
            }
            if let Some(ai) = self
                .entities
                .get_mut(&source.entity)
                .filter(|e| !e.targeting.dead)
                .and_then(|e| e.ai.as_mut())
            {
                if ai.intention == super::super::Intention::Attack {
                    ai.last_hit = tick;
                }
            }
        }
        Ok(())
    }
    fn credit_sp_v5(
        &mut self,
        tick: Tick,
        source: TargetBinding,
        recipient: &Entity,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let CombatRole::Npc { template, .. } = &recipient.combat.as_ref().ok_or_else(missing)?.role
        else {
            return Err(missing());
        };
        let reward = self
            .phase3
            .as_ref()
            .and_then(|p| p.canonical.rules.profiles().npcs.get(template))
            .ok_or_else(missing)?
            .sp_reward;
        let c = self
            .entities
            .get_mut(&source.entity)
            .and_then(|e| e.combat.as_mut())
            .ok_or_else(missing)?;
        if let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        {
            let old = p.class_state.sp;
            p.class_state.sp = old.saturating_add(reward).min(i64::MAX.unsigned_abs());
            events.push(ZoneEvent::SpGained {
                tick,
                entity: source.entity,
                incarnation: source.incarnation,
                amount: p.class_state.sp.saturating_sub(old),
                total: p.class_state.sp,
            });
        }
        Ok(())
    }
    fn sync_passives_v5(&mut self, id: EntityId) -> Result<(), TickError> {
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?
            .role
        else {
            return Err(missing());
        };
        let phase = self.phase3.as_mut().ok_or_else(missing)?;
        let expected: BTreeMap<_, _> = phase
            .canonical
            .rules
            .skills()
            .skills()
            .values()
            .filter(|s| {
                s.operate_type == "P"
                    && s.runtime_status == crate::domain::class::RuntimeSkillStatus::Executable
            })
            .filter_map(|s| {
                player
                    .class_state
                    .learned_skills
                    .iter()
                    .find(|l| l.key == s.key)
                    .map(|l| (s.id, l.level))
            })
            .collect();
        let side = phase.canonical.entities.get_mut(&id).ok_or_else(missing)?;
        side.modifier_install_order.retain(|_, source| {
            if let RestoredModifierSource::Passive(skill) = source {
                expected.get(&skill.id) == Some(&skill.rank)
            } else {
                true
            }
        });
        for (id, rank) in expected {
            let source =
                RestoredModifierSource::Passive(crate::domain::zone::skill::SkillRef { id, rank });
            if !side.modifier_install_order.values().any(|s| *s == source) {
                side.modifier_install_order
                    .insert(phase.canonical.next_install_sequence, source);
                phase.canonical.next_install_sequence = phase
                    .canonical
                    .next_install_sequence
                    .checked_add(1)
                    .ok_or_else(missing)?;
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)] // Paired immutable inputs and final resource installation.
    fn regen_v5(&mut self, tick: Tick) -> Result<(), TickError> {
        use crate::domain::zone::state_regen::{
            regen_amounts, restore_regen, RegenInputs, RegenPosture, RegenResource,
        };
        let rules = Arc::clone(&self.phase3.as_ref().ok_or_else(missing)?.canonical.rules);
        self.refresh_schedule_v5()?;
        let due = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .schedule
            .regen_candidates(tick);
        for (_, id) in due {
            let effective = self.effective(id)?;
            let e = self.entities.get(&id).ok_or_else(missing)?;
            let c = e.combat.as_ref().ok_or_else(missing)?;
            let stats = self.rules.as_deref().ok_or_else(missing)?;
            let (hp, mp, cp, base) = match &c.role {
                CombatRole::Player {
                    progression: Some(p),
                    ..
                } => {
                    let profile = rules
                        .profiles()
                        .classes
                        .get(&p.class_state.current_class_id)
                        .ok_or_else(missing)?;
                    let row = profile.regen_at(c.sheet.level()).ok_or_else(missing)?;
                    (
                        row.hp_regen.as_str(),
                        row.mp_regen.as_str(),
                        row.cp_regen.as_str(),
                        profile.base_stats.values(),
                    )
                },
                CombatRole::Npc { template, .. } => {
                    let profile = rules.profiles().npcs.get(template).ok_or_else(missing)?;
                    (
                        profile.template.template.hp_regen.as_str(),
                        profile.template.template.mp_regen.as_str(),
                        "0",
                        profile.template.base_stats.values(),
                    )
                },
                CombatRole::Player {
                    progression: None, ..
                } => return Err(missing()),
            };
            let player = e.kind == EntityKind::Player;
            let input = RegenInputs {
                player,
                base_hp: ExactRatio::parse_literal(hp).map_err(invariant)?,
                base_mp: ExactRatio::parse_literal(mp).map_err(invariant)?,
                base_cp: ExactRatio::parse_literal(cp).map_err(invariant)?,
                level_mod: q(crate::domain::zone::level_mod(stats.constants(), c.sheet.level())
                    .map_err(invariant)?
                    .raw())
                .map_err(invariant)?,
                con_bonus: if self.phase4.is_some() && player {
                    self.base_property_v5(id, "CON")?.1
                } else {
                    q(stats
                        .bonus()
                        .bonus(crate::domain::zone::StatKind::Con, base.con)
                        .map_err(invariant)?
                        .raw())
                    .map_err(invariant)?
                },
                men_bonus: if self.phase4.is_some() && player {
                    self.base_property_v5(id, "MEN")?.1
                } else {
                    q(stats
                        .bonus()
                        .bonus(crate::domain::zone::StatKind::Men, base.men)
                        .map_err(invariant)?
                        .raw())
                    .map_err(invariant)?
                },
                posture: if e.dest.is_some() {
                    RegenPosture::Moving
                } else {
                    RegenPosture::Standing
                },
            };
            let amounts = regen_amounts(
                input,
                &effective.calculator,
                FunctionConditions {
                    weapon_kind: self
                        .item_weapon_kind(id, if player { "SWORD" } else { "NONE" })?,
                    can_resurrect: player,
                },
                rules.regen(),
            )
            .map_err(invariant)?;
            let cp = match &c.role {
                CombatRole::Player {
                    progression: Some(p),
                    ..
                } => Some(RegenResource {
                    current: p.class_state.cp,
                    maximum: effective.max_cp,
                }),
                CombatRole::Player {
                    progression: None, ..
                }
                | CombatRole::Npc { .. } => None,
            };
            let result = restore_regen(
                RegenResource {
                    current: c.hp,
                    maximum: c.sheet.max_hp(),
                },
                RegenResource {
                    current: c.mp,
                    maximum: c.sheet.max_mp(),
                },
                cp,
                self.sidecar(id)?.fractions,
                amounts,
            )
            .map_err(invariant)?;
            if self.evidence.is_some() {
                self.evidence_fact(
                    "regen_pulse",
                    id,
                    serde_json::json!({
                        "cause": "scheduled_regeneration", "inputs": input,
                        "rules": rules.regen(), "functions": effective.calculator,
                        "before": {"hp": c.hp, "mp": c.mp, "cp": cp},
                        "max_hp": c.sheet.max_hp(), "max_mp": c.sheet.max_mp(),
                        "fractions_before": self.sidecar(id)?.fractions,
                        "amounts": amounts, "actual": result
                    }),
                );
            }
            let c = self
                .entities
                .get(&id)
                .and_then(|e| e.combat.as_ref())
                .ok_or_else(missing)?;
            if result.hp != c.hp {
                self.phase3
                    .as_mut()
                    .ok_or_else(missing)?
                    .vitals_dirty
                    .insert(id);
            }
            let side = self.sidecar_mut(id)?;
            side.regen
                .take_due(tick, rules.regen())
                .map_err(invariant)?;
            side.fractions = result.fractions;
            let c = self
                .entities
                .get_mut(&id)
                .and_then(|e| e.combat.as_mut())
                .ok_or_else(missing)?;
            c.hp = result.hp;
            c.mp = result.mp;
            if let CombatRole::Player {
                progression: Some(p),
                ..
            } = &mut c.role
            {
                p.class_state.cp = result.cp.ok_or_else(missing)?;
            }
        }
        Ok(())
    }
    pub(super) fn owner_stats_v5(
        &self,
        tick: Tick,
        id: EntityId,
        baseline: Option<Tick>,
    ) -> Result<ZoneEvent, TickError> {
        let c = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
        let stats = self.effective(id)?;
        let player = self.sidecar(id)?.player.as_ref().ok_or_else(missing)?;
        let mut event = stats_changed(tick, id, c);
        if let ZoneEvent::StatsChanged {
            class: Some(class), ..
        } = &mut event
        {
            class.phase3 = Some(Box::new(crate::domain::zone::OwnerPhase3View {
                incarnation: c.incarnation,
                baseline,
                pvp: player.pvp.clone(),
                death_penalty_level: player.death_penalty_level,
                effective: crate::domain::zone::LearnedEffectiveStats {
                    p_atk_q: stats.physical.p_atk().raw(),
                    p_def_q: stats.physical.p_def().raw(),
                    m_atk_q: stats.m_atk_q,
                    m_def_q: stats.m_def_q,
                    attack_speed: u32::try_from(
                        stats
                            .physical
                            .attack_speed()
                            .raw()
                            .checked_div(Q)
                            .ok_or_else(missing)?,
                    )
                    .map_err(invariant)?,
                    cast_speed: stats.cast_speed,
                    speed_q: stats.run_speed_q,
                    accuracy_q: stats.physical.accuracy().raw(),
                    evasion_q: stats.physical.evasion().raw(),
                    crit_permille: stats.physical.crit_permille(),
                },
            }));
        }
        Ok(event)
    }
    fn owner_sync_v5(&self, tick: Tick, id: EntityId) -> Result<ZoneEvent, TickError> {
        let c = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            return Err(missing());
        };
        let side = self.sidecar(id)?;
        let player = side.player.as_ref().ok_or_else(missing)?;
        let rules = &self.phase3.as_ref().ok_or_else(missing)?.canonical.rules;
        let mut learned: Vec<_> = rules
            .skills()
            .skills()
            .values()
            .filter_map(|s| {
                p.class_state
                    .learned_skills
                    .iter()
                    .find(|l| l.key == s.key)
                    .map(|l| crate::domain::zone::skill::SkillRef {
                        id: s.id,
                        rank: l.level,
                    })
            })
            .collect();
        learned.sort_by_key(|s| s.id);
        Ok(ZoneEvent::SkillStateSync {
            tick,
            entity: id,
            incarnation: c.incarnation,
            learned,
            reuse: side.reuse.clone(),
            sp: p.class_state.sp,
            pvp: player.pvp.clone(),
            death_penalty_level: player.death_penalty_level,
            baseline: Some(tick),
        })
    }
    fn final_resources_v5(
        &self,
        tick: Tick,
        before: &BTreeMap<EntityId, Entity>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        // Replace intermediate owner resource facts with final values; never broadcast MP/CP.
        events.retain(|e| !matches!(e, ZoneEvent::StatsChanged { .. }));
        // These facts do not change while final projections are appended. Index once,
        // preserving life-qualified damage even when a later heal restores the old HP.
        let mut effects_changed = std::collections::BTreeSet::new();
        let mut damaged = std::collections::BTreeSet::new();
        for event in events.iter() {
            if let ZoneEvent::EffectApplied { target, .. }
            | ZoneEvent::EffectRemoved { target, .. } = event
            {
                effects_changed.insert(target.entity);
            }
            if let ZoneEvent::SkillCastResult { targets, .. } = event {
                damaged.extend(
                    targets
                        .iter()
                        .filter(|row| row.hp_removed != 0)
                        .map(|row| (row.target.entity, row.target.incarnation)),
                );
            }
        }
        for (id, e) in &self.entities {
            let Some(c) = &e.combat else { continue };
            let old = before.get(id).and_then(|e| e.combat.as_ref());
            if e.kind == EntityKind::Player && (old != Some(c) || effects_changed.contains(id)) {
                events.push(self.owner_stats_v5(tick, *id, None)?);
            }
            // A regen pulse followed by damage can return HP to its starting value;
            // landed removal still marks the public pair dirty for one final projection.
            if old.is_some_and(|old| {
                old.hp != c.hp
                    || old.sheet.max_hp() != c.sheet.max_hp()
                    || old.incarnation != c.incarnation
                    || damaged.contains(&(*id, c.incarnation))
                    || self
                        .phase3
                        .as_ref()
                        .is_some_and(|p| p.vitals_dirty.contains(id))
            }) {
                events.push(ZoneEvent::VitalsChanged {
                    tick,
                    entity: *id,
                    hp: c.hp,
                    max_hp: c.sheet.max_hp(),
                    incarnation: c.incarnation,
                    baseline: None,
                });
            }
        }
        Ok(())
    }
    #[allow(clippy::wildcard_enum_match_arm)] // Guards omit invisible Phase3 facts; other events already routed by observe.
    #[allow(clippy::too_many_lines)] // Identity batch and recursively filtered hydration.
    fn observe_v5(
        &mut self,
        tick: Tick,
        responses: BTreeMap<CommandSource, Vec<ObserverOutput>>,
        events: &[ZoneEvent],
        previous: &BTreeMap<EntityId, (CellCoord, Arc<[EntityId]>)>,
    ) -> Result<BTreeMap<EntityId, Vec<ObserverOutput>>, TickError> {
        // Reuse only AOI identity/movement and legacy facts. Phase3 facts are recursively
        // filtered against the complete identity batch below.
        let mut outputs = self.observe(tick, responses, events);
        // Ordinary movement/legacy facts were already routed by the indexed observer
        // pass. Only these new variants need the recursive Phase3 reference filter.
        let phase3_events: Vec<_> = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    ZoneEvent::EffectApplied { .. }
                        | ZoneEvent::EffectRemoved { .. }
                        | ZoneEvent::SkillCastStarted { .. }
                        | ZoneEvent::SkillCastResult { .. }
                        | ZoneEvent::SkillCastCancelled { .. }
                        | ZoneEvent::SkillLearned { .. }
                        | ZoneEvent::SpGained { .. }
                        | ZoneEvent::SkillReuse { .. }
                        | ZoneEvent::VitalsChanged { .. }
                )
            })
            .collect();
        for (observer, (_, known)) in &self.known {
            let sees = |id: EntityId| id == *observer || known.binary_search(&id).is_ok();
            let out = outputs.entry(*observer).or_default();
            // Replacement resets the observer cache during phase0 even though this id was
            // known in the tick-start copy. Detect its actual identity batch too.
            let owner_baseline = out.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) if entity == observer));
            let hydrate = owner_baseline
                || previous
                    .get(observer)
                    .is_none_or(|(_, old)| old.as_ref() != known.as_ref());
            if !hydrate && phase3_events.is_empty() {
                // The existing indexed pass already produced responses then ordinary
                // facts. With no new identities or Phase3 deltas, that is final order.
                continue;
            }
            let knew = |id: EntityId| {
                !owner_baseline
                    && previous
                        .get(observer)
                        .is_some_and(|(_, ids)| ids.binary_search(&id).is_ok())
            };
            // Legacy observe already establishes the final AOI. Move its ordinary facts
            // behind the complete V5 hydration batch without changing their relative order.
            let mut ordinary = Vec::new();
            let routed = std::mem::take(out);
            out.reserve(routed.len());
            for output in routed {
                if matches!(
                    output,
                    ObserverOutput::Accepted { .. }
                        | ObserverOutput::Rejected { .. }
                        | ObserverOutput::Event(
                            ZoneEvent::EntitySpawn { .. } | ZoneEvent::EntityDespawn { .. }
                        )
                ) {
                    out.push(output);
                } else {
                    ordinary.push(output);
                }
            }
            if owner_baseline && self.binding(*observer).is_some() {
                ordinary.retain(|o| !matches!(o, ObserverOutput::Event(ZoneEvent::StatsChanged { entity, .. }) if entity == observer));
                out.push(ObserverOutput::Event(self.owner_stats_v5(
                    tick,
                    *observer,
                    Some(tick),
                )?));
                out.push(ObserverOutput::Event(self.owner_sync_v5(tick, *observer)?));
            }
            // Equal sorted known sets have no new hydration subjects. Replacement
            // still forces the whole baseline, even if every entity ID is unchanged.
            if hydrate {
                for id in known.iter().filter(|id| !knew(**id)) {
                    if let Ok(side) = self.sidecar(*id) {
                        for effect in side.effects.entries().values() {
                            let mut event =
                                self.effect_applied_v5(tick, *id, effect, Some(tick))?;
                            if let ZoneEvent::EffectApplied { source, .. } = &mut event {
                                *source = source.filter(|s| {
                                    sees(s.entity) && self.binding(s.entity) == Some(*s)
                                });
                            }
                            out.push(ObserverOutput::Event(event));
                        }
                        if let CastState::Casting(cast) = &side.cast {
                            let mut event = cast_started(tick, *id, cast, Some(tick));
                            if let ZoneEvent::SkillCastStarted { target, .. } = &mut event {
                                *target = target.filter(|t| {
                                    sees(t.entity) && self.binding(t.entity) == Some(*t)
                                });
                            }
                            out.push(ObserverOutput::Event(event));
                        }
                    }
                }
            }
            out.extend(ordinary);
            for event in phase3_events.iter().copied() {
                let projected = match event {
                    ZoneEvent::EffectApplied { target, source, .. }
                        if sees(target.entity) && knew(target.entity) =>
                    {
                        let mut e = event.clone();
                        if let ZoneEvent::EffectApplied { source: s, .. } = &mut e {
                            *s = source
                                .filter(|s| sees(s.entity) && self.binding(s.entity) == Some(*s));
                        }
                        Some(e)
                    },
                    ZoneEvent::EffectRemoved { target, .. }
                        if sees(target.entity) && knew(target.entity) =>
                    {
                        Some(event.clone())
                    },
                    ZoneEvent::SkillCastStarted { caster, target, .. }
                        if sees(caster.entity) && knew(caster.entity) =>
                    {
                        let mut e = event.clone();
                        if let ZoneEvent::SkillCastStarted { target: t, .. } = &mut e {
                            *t = target
                                .filter(|t| sees(t.entity) && self.binding(t.entity) == Some(*t));
                        }
                        Some(e)
                    },
                    ZoneEvent::SkillCastResult {
                        caster, targets, ..
                    } if sees(caster.entity) && knew(caster.entity) => {
                        let mut e = event.clone();
                        if let ZoneEvent::SkillCastResult { targets: t, .. } = &mut e {
                            *t = targets
                                .iter()
                                .filter(|t| {
                                    sees(t.target.entity)
                                        && self.binding(t.target.entity) == Some(t.target)
                                })
                                .cloned()
                                .collect();
                        }
                        Some(e)
                    },
                    ZoneEvent::SkillCastCancelled { caster, .. }
                        if sees(caster.entity) && knew(caster.entity) =>
                    {
                        Some(event.clone())
                    },
                    ZoneEvent::SkillLearned { entity, .. }
                    | ZoneEvent::SpGained { entity, .. }
                    | ZoneEvent::SkillReuse { entity, .. }
                        if entity == observer =>
                    {
                        Some(event.clone())
                    },
                    ZoneEvent::VitalsChanged { entity, .. } if sees(*entity) && knew(*entity) => {
                        Some(event.clone())
                    },
                    _ => None,
                };
                if let Some(event) = projected {
                    out.push(ObserverOutput::Event(event));
                }
            }
        }
        outputs.retain(|_, out| !out.is_empty());
        Ok(outputs)
    }
}
fn cast_started(
    tick: Tick,
    entity: EntityId,
    cast: &crate::domain::zone::CastInFlight,
    baseline: Option<Tick>,
) -> ZoneEvent {
    ZoneEvent::SkillCastStarted {
        tick,
        caster: TargetBinding {
            entity,
            incarnation: cast.caster_incarnation,
        },
        target: cast.target,
        skill: cast.skill,
        cast_id: cast.id,
        start: cast.start_at,
        launch_at: cast.launch_at,
        complete_at: cast.complete_at,
        baseline,
    }
}

// Incremental experimental runtime capability gate; registry capability metadata is unchanged.
fn hostile_skill(skill: &SkillDef) -> bool {
    skill.abnormal.as_ref().is_some_and(|a| a.is_debuff)
        || skill.effect_point.iter().any(|value| *value < 0)
        || skill.effects.iter().any(|h| {
            matches!(
                h.name.as_str(),
                "MagicalAttack"
                    | "MagicalAttackRange"
                    | "PhysicalAttack"
                    | "FatalBlow"
                    | "Lethal"
                    | "GetAgro"
                    | "AddHate"
            )
        })
}
fn supported_cast(skill: &SkillDef, rank: u32) -> bool {
    skill
        .affect_scope
        .at(rank, skill.max_level)
        .is_some_and(|s| matches!(s.as_str(), "SINGLE" | "unset" | "RANGE" | "POINT_BLANK"))
        && skill
            .target_type
            .at(rank, skill.max_level)
            .is_some_and(|s| {
                matches!(s.as_str(), "SELF" | "TARGET" | "ENEMY" | "ENEMY_ONLY" | "PC_BODY")
            })
        && skill.effects.iter().all(|h| {
            matches!(
                h.name.as_str(),
                "Resurrection"
                    | "Escape"
                    | "PhysicalAttack"
                    | "FatalBlow"
                    | "Lethal"
                    | "MagicalAttack"
                    | "MagicalAttackRange"
                    | "Heal"
                    | "Hp"
                    | "HpPerMax"
                    | "Mp"
                    | "Cp"
                    | "DispelBySlot"
                    | "DispelByCategory"
                    | "GetAgro"
                    | "AddHate"
                    | "Buff"
                    | "Debuff"
                    | "Stun"
                    | "Sleep"
                    | "Root"
                    | "TickHp"
                    | "TickMp"
                    | "MaxHp"
                    | "MaxMp"
                    | "DefenceTrait"
                    | "AttackTrait"
            ) && h.section == "effects"
                && (h.name != "Escape"
                    || h.params
                        .get("escapeType")
                        .and_then(|value| value.at(rank, skill.max_level))
                        .is_some_and(|value| value == "TOWN"))
                && h.conditions.is_empty()
                && h.section_conditions.is_empty()
                && (!matches!(h.name.as_str(), "MaxHp" | "MaxMp")
                    || (skill.effects.len() == 1 && h.stat_children.is_empty()))
        })
}

impl ZoneState {
    fn npc_profile_v5(&self, spec: &NpcCombat, speed: Speed) -> bool {
        let Some(profile) = self
            .phase3
            .as_ref()
            .and_then(|p| p.canonical.rules.profiles().npcs.get(&spec.template))
        else {
            return false;
        };
        let raw = &profile.physical;
        spec.stats.level == raw.level
            && spec.stats.max_hp == raw.max_hp
            && spec.stats.max_mp == raw.max_mp
            && spec.stats.p_atk.raw() == raw.p_atk_q
            && spec.stats.p_def.raw() == raw.p_def_q
            && spec.stats.attack_speed.raw() == i64::from(raw.attack_speed).saturating_mul(Q)
            && speed == raw.move_speed
    }

    #[allow(clippy::wildcard_enum_match_arm)] // Only lifecycle facts allocate/remove sidecars.
    fn spawn_phase_v5(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) -> Result<(), TickError> {
        if self
            .slots
            .iter()
            .any(|slot| !self.npc_profile_v5(&slot.combat, slot.speed))
        {
            return Err(TickError::Phase3NotReady);
        }
        if self
            .members
            .values()
            .any(|m| m.respawn_at.is_some_and(|at| at <= tick) && m.incarnation == u32::MAX)
        {
            return Err(TickError::Phase3Invariant);
        }
        let mut spawned = Vec::new();
        self.spawn_phase(tick, &mut spawned);
        for event in &spawned {
            match event {
                ZoneEvent::EntityDespawn { entity, .. } => {
                    self.remove_items_v6(*entity);
                    let phase = self.phase3.as_mut().ok_or_else(missing)?;
                    phase.canonical.entities.remove(entity);
                    phase.effective.remove(entity);
                    phase.schedule_dirty.insert(*entity);
                },
                ZoneEvent::EntitySpawn { entity, .. } => self.attach_v5(tick, *entity, None)?,
                _ => {},
            }
        }
        events.extend(spawned);
        Ok(())
    }

    /// Hard control suppresses AI actions; ROOT still allows in-range intention changes.
    pub(super) fn auto_can_walk_v5(&self, id: EntityId) -> bool {
        self.auto_action_v5(id)
            && self
                .effective(id)
                .is_ok_and(|s| !s.flags.intersects(effects::StatusFlags::ROOT))
    }
    pub(super) fn auto_action_v5(&self, id: EntityId) -> bool {
        use crate::domain::zone::effects::StatusFlags as F;
        self.entities.get(&id).is_some_and(|e| !e.targeting.dead)
            && self
                .sidecar(id)
                .is_ok_and(|s| matches!(s.cast, CastState::Idle))
            && self.effective(id).is_ok_and(|s| {
                !s.flags
                    .intersects(F::STUN.union(F::SLEEP).union(F::PARALYZE))
            })
    }
    pub(super) fn npc_action_v5(&self, npc: EntityId) -> bool {
        use crate::domain::zone::effects::StatusFlags as F;
        self.entities
            .get(&npc)
            .is_some_and(|e| e.kind == EntityKind::Npc && !e.targeting.dead)
            && self.effective(npc).is_ok_and(|s| {
                !s.flags
                    .intersects(F::STUN.union(F::SLEEP).union(F::PARALYZE))
            })
    }

    pub(super) fn npc_pair_v5(&self, tick: Tick, npc: EntityId, target: EntityId) -> bool {
        match (self.binding(npc), self.binding(target)) {
            (Some(a), Some(b)) => self.hostility_v5(tick, a, b).is_ok(),
            _ => false,
        }
    }

    pub(super) fn npc_can_walk_v5(&self, npc: EntityId) -> bool {
        use crate::domain::zone::effects::StatusFlags as F;
        self.npc_action_v5(npc)
            && self
                .effective(npc)
                .is_ok_and(|s| !s.flags.intersects(F::ROOT))
    }

    fn most_hated_v5(&self, tick: Tick, npc: EntityId) -> Option<EntityId> {
        let current = self.entities.get(&npc)?.targeting.target;
        self.hate
            .get(&npc)?
            .most_hated(current, |target| self.npc_pair_v5(tick, npc, target))
    }

    /// Immediate ledger mutation records its selection; physical intention changes wait
    /// until phase nine. Design F11 / GetAgro.java:49-54.
    pub(super) fn reselect_v5(&mut self, tick: Tick, npc: EntityId) {
        let Some(binding) = self.binding(npc) else {
            return;
        };
        let target = self
            .most_hated_v5(tick, npc)
            .and_then(|id| self.binding(id));
        if self.evidence.is_some() {
            let eligible: Vec<_> = self
                .entities
                .keys()
                .copied()
                .filter(|id| self.npc_pair_v5(tick, npc, *id))
                .collect();
            self.evidence_fact(
                "hate_selection",
                npc,
                serde_json::json!({
                    "ledger": self.hate.get(&npc), "eligible": eligible,
                    "current_target": self.entities.get(&npc).and_then(|e| e.targeting.target),
                    "most_hated": target
                }),
            );
        }
        if let Some(phase) = &mut self.phase3 {
            phase
                .ai_requests
                .push(crate::domain::zone::phase3::AiRequest {
                    npc: binding,
                    target,
                    force: false,
                });
        }
    }

    fn drain_ai_v5(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        loop {
            let requests = self
                .phase3
                .as_mut()
                .map(|p| std::mem::take(&mut p.ai_requests))
                .unwrap_or_default();
            if requests.is_empty() {
                break;
            }
            for request in requests {
                let npc = request.npc.entity;
                if self.binding(npc) != Some(request.npc)
                    || !self.npc_action_v5(npc)
                    || request.target.is_some_and(|target| {
                        self.binding(target.entity) != Some(target)
                            || !self.npc_pair_v5(tick, npc, target.entity)
                    })
                {
                    continue;
                }
                let chosen = request.target.map(|t| t.entity);
                let current = self.entities.get(&npc).and_then(|e| e.targeting.target);
                if chosen != current {
                    self.disengage(tick, npc, SwingCancel::TargetChanged, events);
                    if let Some(e) = self.entities.get_mut(&npc) {
                        e.targeting.target = chosen;
                    }
                    events.push(ZoneEvent::TargetChanged {
                        phase3: None,
                        tick,
                        entity: npc,
                        target: chosen,
                    });
                }
                if let Some(c) = self.entities.get_mut(&npc).and_then(|e| e.combat.as_mut()) {
                    c.auto_attack = chosen.is_some();
                }
                if let Some(phase) = &mut self.phase3 {
                    if request.force {
                        phase.forced_this_tick.insert(npc);
                    } else {
                        phase.forced_this_tick.remove(&npc);
                    }
                }
                self.ai_target_changed(tick, npc, chosen, events);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn agro_handler_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        let entity = self.entities.get(&target.entity).ok_or_else(missing)?;
        let hp = entity.combat.as_ref().ok_or_else(missing)?.hp;
        if entity.kind == EntityKind::Npc && hp != 0 {
            if handler.name == "GetAgro" {
                if entity.ai.is_some()
                    && self.most_hated_v5(tick, target.entity) != Some(caster.entity)
                {
                    self.phase3.as_mut().ok_or_else(missing)?.ai_requests.push(
                        crate::domain::zone::phase3::AiRequest {
                            npc: target,
                            target: Some(caster),
                            force: true,
                        },
                    );
                }
            } else {
                let power = handler
                    .params
                    .get("power")
                    .and_then(|p| p.at(rank, skill.max_level))
                    .map_or("0", String::as_str);
                let power = ExactRatio::parse_literal(power).map_err(invariant)?;
                let signed = power.parts().0;
                let amount = power
                    .trunc()
                    .map_err(invariant)?
                    .saturating_abs()
                    .min(i128::from(i32::MAX));
                // Pinned quirk: AddHate.java:54-59 passes positive -power to reduceHate;
                // L2Attackable.java:662-669 and AggroInfo.java:51-53 ADD it to an existing
                // row. Preserve that behavior; a missing row is a no-op on this branch.
                if signed > 0
                    || (signed < 0
                        && self
                            .hate
                            .get(&target.entity)
                            .and_then(|l| l.get(caster.entity))
                            .is_some())
                {
                    self.add_hate_for(
                        tick,
                        target.entity,
                        caster.entity,
                        super::super::HateEntry {
                            hate: u64::try_from(amount).map_err(invariant)?,
                            damage: 0,
                        },
                        "add_hate",
                        events,
                    );
                }
            }
        }
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Effect,
            outcome: SkillOutcome::Hit,
            rolled: 0,
            cp_absorbed: 0,
            hp_removed: 0,
            heal: 0,
            target_hp_after: hp,
            applied_effect_ids: Vec::new(),
        })
    }
}

impl ZoneState {
    /// AffectScope.java:379-403 and L2World.java:372-384: strict circular radius;
    /// `POINT_BLANK` excludes its origin, RANGE includes its living center. C1 says a
    /// zero width is unlimited and consumes no draw. Accepted sets retain ordinal order.
    fn collect_targets_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        anchor: TargetBinding,
        skill: &SkillDef,
        rank: u32,
    ) -> Result<(u32, Vec<TargetBinding>), RejectReason> {
        let scope = skill
            .affect_scope
            .at(rank, skill.max_level)
            .ok_or(RejectReason::InvalidLoad)?;
        if matches!(scope.as_str(), "SINGLE" | "unset") {
            return Ok((0, vec![anchor]));
        }
        let center = self
            .entities
            .get(&anchor.entity)
            .ok_or(RejectReason::InvalidTargetType)?
            .pos;
        let range = skill
            .affect_range
            .as_ref()
            .and_then(|v| v.at(rank, skill.max_level))
            .copied()
            .unwrap_or(0);
        let radius = i128::from(range)
            .checked_mul(1000)
            .and_then(|v| v.checked_mul(v))
            .ok_or(RejectReason::InvalidLoad)?;
        let mut ordered = Vec::new();
        for id in self.aoi.in_aoi(center) {
            if scope == "POINT_BLANK" && id == anchor.entity {
                continue;
            }
            let Some(entity) = self.entities.get(&id) else {
                continue;
            };
            let Some(binding) = self.binding(id) else {
                continue;
            };
            if entity.targeting.dead {
                continue;
            }
            let dx = i128::from(entity.pos.x.raw())
                .checked_sub(i128::from(center.x.raw()))
                .ok_or(RejectReason::InvalidLoad)?;
            let dy = i128::from(entity.pos.y.raw())
                .checked_sub(i128::from(center.y.raw()))
                .ok_or(RejectReason::InvalidLoad)?;
            let distance = dx
                .checked_mul(dx)
                .and_then(|x| dy.checked_mul(dy).and_then(|y| x.checked_add(y)))
                .and_then(|v| v.checked_mul(1024))
                .ok_or(RejectReason::InvalidLoad)?;
            if distance >= radius {
                continue;
            }
            let hostile = hostile_skill(skill) || skill.affect_object == "NOT_FRIEND";
            if self
                .cast_target_v5(tick, caster, binding, -1, hostile, false, false)
                .is_err()
            {
                continue;
            }
            // The experimental PvP fence applies to the whole action, never silently
            // executing only its PvE recipients. It precedes costs/limit RNG.
            if !self.complete_runtime_v5() && hostile && entity.kind == EntityKind::Player {
                return Err(RejectReason::NotYetImplemented);
            }
            let ordinal = self
                .sidecar(id)
                .map_err(|_| RejectReason::InvalidLoad)?
                .ordinal;
            ordered.push((ordinal, id, binding));
        }
        ordered.sort_by_key(|(ordinal, id, _)| (*ordinal, *id));
        let limit = skill.affect_limit;
        if limit.width > 0
            && limit
                .offset
                .checked_add(limit.width.saturating_sub(1))
                .is_none()
        {
            return Err(RejectReason::InvalidLoad);
        }
        let sampled = if limit.width == 0 {
            0
        } else {
            limit.offset.saturating_add(self.evidence_roll(
                "AffectLimit",
                Some(caster),
                limit.width,
            ))
        };
        let mut targets: Vec<_> = ordered.into_iter().map(|(_, _, target)| target).collect();
        if sampled != 0 {
            targets.truncate(usize::try_from(sampled).unwrap_or(usize::MAX));
        }
        Ok((sampled, targets))
    }

    #[allow(clippy::too_many_arguments)] // Life, range and hostile/corpse policies are independent guards.
    fn filter_targets_v5(
        &self,
        tick: Tick,
        caster: TargetBinding,
        captured: &[TargetBinding],
        range: i64,
        hostile: bool,
        corpse: bool,
    ) -> (Vec<TargetBinding>, RejectReason) {
        let mut targets = Vec::new();
        let mut reason = RejectReason::InvalidTargetType;
        for target in captured {
            match self.cast_target_v5(tick, caster, *target, range, hostile, true, corpse) {
                Ok(()) => targets.push(*target),
                Err(rejected) => {
                    reason = rejected;
                },
            }
        }
        (targets, reason)
    }
}

/// L2Character.java:4504-4510: a positive effectRange wins; otherwise self/point-blank
/// skills with negative castRange and affectRange>80 use that radius. Zero means no escape
/// check. The caller adds the caster template collision radius (source :4521).
fn effect_range_v5(skill: &SkillDef, rank: u32) -> Result<i64, TickError> {
    let effect = *skill
        .effect_range
        .at(rank, skill.max_level)
        .ok_or_else(missing)?;
    let cast = *skill
        .cast_range
        .at(rank, skill.max_level)
        .ok_or_else(missing)?;
    let affect = skill
        .affect_range
        .as_ref()
        .and_then(|v| v.at(rank, skill.max_level))
        .copied()
        .unwrap_or(0);
    Ok(if effect > 0 {
        effect
    } else if cast < 0 && affect > 80 {
        affect
    } else {
        -1
    })
}
