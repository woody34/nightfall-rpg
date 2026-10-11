//! Canonical live Phase 3 state. Snapshot timers are absolute and never pass through the
//! paused-character checkpoint filter. Rebuildable calculators and status masks stay out.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::effects::{EffectCapacities, EffectError, EffectList, FrozenRatio, TargetBinding};
use super::skill::{SkillId, SkillRef};
use super::state_regen::RegenSchedule;
use super::{DeathRecord, Ordinal, Phase3Rules, ResourceFractions, RestoredModifierSource, Tick};
use crate::domain::class::RuntimeSkillStatus;
use crate::domain::subclass::LearnedSkill;

/// Allocation order of a combatant's admission/life; independent of UUID order and RNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityOrdinal(pub u64);

/// Zone-wide monotone cast identity. Zero is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CastId(pub u64);

/// Zone-wide monotone resurrection offer identity. Zero is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OfferId(pub u64);

/// Source timing/cost values frozen at accepted start. Strength is read at impact.
/// Animation retains its exact fractional part for the source truncation before launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct CastInFlight {
    pub id: CastId,
    pub caster_incarnation: u32,
    pub skill: SkillRef,
    pub command_ordinal: Ordinal,
    pub start_at: Tick,
    pub animation: FrozenRatio,
    pub launch_at: Tick,
    pub complete_at: Tick,
    pub interrupt_until: Tick,
    pub launched: bool,
    pub target: Option<TargetBinding>,
    pub affect_limit: u32,
    pub targets: Vec<TargetBinding>,
    pub mp_start: u32,
    pub mp_end: u32,
    pub hp_end: u32,
    pub reuse_until: Tick,
}

/// There is no invented coolTime/recovery state (pinned L2Character.java:4666-4690).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "cast",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CastState {
    /// No cast in flight.
    Idle,
    /// Launched or awaiting launch; impact always follows start by at least one tick.
    Casting(CastInFlight),
}

/// Independent flag deadline and bounded persisted counters; color is a projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PvpState {
    pub flag_until: Option<Tick>,
    pub karma: u32,
    pub pk_count: u32,
    pub pvp_count: u32,
}

/// Live offer bound to exactly one death, excluded from offline checkpoint persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ResurrectOffer {
    pub id: OfferId,
    pub caster: TargetBinding,
    pub target_incarnation: u32,
    pub skill: SkillRef,
    pub created_at: Tick,
    pub expires_at: Tick,
    pub restore_percent_q: i64,
    pub recoverable_xp: u64,
}

/// Player-only policy state. SP, CP and learned ranks remain in the class ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase3Player {
    pub pvp: PvpState,
    pub death_penalty_level: u8,
    pub last_death: Option<DeathRecord>,
    pub resurrect_offer: Option<ResurrectOffer>,
}

/// One canonical sidecar per V5 combatant. A single ordered map preserves installation
/// order across passives, timed effects and death penalty, without a duplicate ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase3Entity {
    pub ordinal: EntityOrdinal,
    pub cast: CastState,
    pub reuse: BTreeMap<SkillId, Tick>,
    pub effects: EffectList,
    pub modifier_install_order: BTreeMap<u64, RestoredModifierSource>,
    pub regen: RegenSchedule,
    pub fractions: ResourceFractions,
    pub player: Option<Phase3Player>,
}

/// Immutable entity facts needed to validate a live sidecar at a snapshot boundary.
/// `next_tick` is the first tick not yet executed; work due exactly then is permitted.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct Phase3EntityContext<'a> {
    pub next_tick: Tick,
    pub incarnation: u32,
    pub alive: bool,
    pub player: bool,
    pub learned: &'a [LearnedSkill],
}

impl Phase3Entity {
    /// Validate canonical timers and source identities without admitting, mutating, or
    /// replaying callbacks. Effective sheet/capacity validation is a separate rebuild step.
    pub fn validate_live(
        &self,
        rules: &Phase3Rules,
        context: Phase3EntityContext<'_>,
    ) -> Result<(), EffectError> {
        self.fractions.validate()?;
        if self.ordinal.0 == 0
            || context.incarnation == 0
            || self.player.is_some() != context.player
            || self.regen.next_at.is_some() != context.alive
            || self.regen.next_at.is_some_and(|at| at < context.next_tick)
            || (!context.alive && self.fractions != ResourceFractions::default())
            || (!context.player && self.fractions.cp != 0)
        {
            return Err(EffectError::Invalid("invalid live sidecar/life/regen"));
        }
        let last_tick = Tick(
            context
                .next_tick
                .0
                .checked_sub(1)
                .ok_or(EffectError::Invalid("combatant before first completed tick"))?,
        );
        if self.regen.next_at.is_some_and(|at| {
            at.0.saturating_sub(last_tick.0)
                > u64::from(rules.regen().period.hp_regenerate_period_ticks)
        }) {
            return Err(EffectError::Invalid("regen beyond next source period"));
        }
        for (id, deadline) in &self.reuse {
            let skill = rules
                .skills()
                .get(*id)
                .ok_or(EffectError::Invalid("unknown reuse"))?;
            if skill.runtime_status != RuntimeSkillStatus::Executable
                || skill.operate_type == "P"
                || *deadline < context.next_tick
            {
                return Err(EffectError::Invalid("invalid live reuse"));
            }
        }
        self.effects
            .validate_capacity(EffectCapacities::for_enlarge(4))?;
        for effect in self.effects.entries().values() {
            effect.validate_source(rules.skills())?;
            if effect.target_incarnation != context.incarnation
                || effect.applied_at >= context.next_tick
                || effect.expires_at.is_some_and(|at| at < context.next_tick)
                || (!context.alive && !effect.flags.stay_after_death)
            {
                return Err(EffectError::Invalid("invalid live effect lifetime"));
            }
            for handler in &effect.handlers {
                if let Some(periodic) = &handler.periodic {
                    if periodic.schedule.anchor > effect.applied_at
                        || periodic.schedule.due_tick()? < context.next_tick
                        || periodic.schedule.remaining_ms(last_tick)?
                            > u64::from(periodic.schedule.interval_ms)
                    {
                        return Err(EffectError::Invalid("overdue live periodic phase"));
                    }
                }
            }
        }
        self.validate_modifiers(rules, context.learned)?;
        if let CastState::Casting(cast) = &self.cast {
            cast.validate(rules, context)?;
        }
        if let Some(player) = &self.player {
            player.validate(rules, context)?;
        }
        Ok(())
    }

    fn validate_modifiers(
        &self,
        rules: &Phase3Rules,
        learned: &[LearnedSkill],
    ) -> Result<(), EffectError> {
        let expected: BTreeMap<_, _> = rules
            .skills()
            .skills()
            .values()
            .filter(|s| s.operate_type == "P" && s.runtime_status == RuntimeSkillStatus::Executable)
            .filter_map(|s| {
                learned
                    .iter()
                    .find(|l| l.key == s.key)
                    .map(|l| (s.id, l.level))
            })
            .collect();
        let mut passives = BTreeMap::new();
        let mut effects = BTreeSet::new();
        let mut penalty = false;
        for (sequence, source) in &self.modifier_install_order {
            if *sequence == 0 {
                return Err(EffectError::Invalid("zero modifier sequence"));
            }
            match source {
                RestoredModifierSource::Passive(skill) => {
                    let definition = rules
                        .skills()
                        .get(skill.id)
                        .ok_or(EffectError::Invalid("unknown passive"))?;
                    if skill.rank == 0
                        || skill.rank > definition.max_level
                        || passives.insert(skill.id, skill.rank).is_some()
                    {
                        return Err(EffectError::Invalid("invalid passive rank/source"));
                    }
                },
                RestoredModifierSource::Effect(id) => {
                    if !self.effects.entries().contains_key(id) || !effects.insert(*id) {
                        return Err(EffectError::Invalid("invalid effect modifier source"));
                    }
                },
                RestoredModifierSource::DeathPenalty => {
                    if penalty {
                        return Err(EffectError::Invalid("duplicate death penalty"));
                    }
                    penalty = true;
                },
            }
        }
        if passives != expected
            || effects.len() != self.effects.entries().len()
            || penalty
                != self
                    .player
                    .as_ref()
                    .is_some_and(|p| p.death_penalty_level != 0)
        {
            return Err(EffectError::Invalid("missing/mismatched modifier source"));
        }
        Ok(())
    }
}

impl CastInFlight {
    fn validate(
        &self,
        rules: &Phase3Rules,
        context: Phase3EntityContext<'_>,
    ) -> Result<(), EffectError> {
        let skill = rules
            .skills()
            .get(self.skill.id)
            .ok_or(EffectError::Invalid("unknown cast skill"))?;
        let deadlines = super::skill_math::cast_deadlines(
            self.start_at,
            self.animation.value(),
            skill.operate_type == "A2" || skill.operate_type == "A3",
        )
        .map_err(|_| EffectError::Invalid("invalid cast animation"))?;
        if !context.alive
            || self.id.0 == 0
            || self.caster_incarnation != context.incarnation
            || skill.runtime_status != RuntimeSkillStatus::Executable
            || skill.operate_type == "P"
            || self.skill.rank == 0
            || self.skill.rank > skill.max_level
            || self.start_at >= context.next_tick
            || self.complete_at < context.next_tick
            || self.launch_at != deadlines.launch_at
            || self.complete_at != deadlines.complete_at
            || self.interrupt_until != deadlines.interrupt_until
            || self.reuse_until < self.start_at
            || (self.launched && self.launch_at >= context.next_tick)
            || (!self.launched && self.launch_at < context.next_tick)
            || (context.player
                && !context
                    .learned
                    .iter()
                    .any(|l| l.key == skill.key && l.level == self.skill.rank))
        {
            return Err(EffectError::Invalid("invalid live cast/deadlines"));
        }
        let mut seen = BTreeSet::new();
        if self.target.is_some_and(|t| t.incarnation == 0)
            || self
                .targets
                .iter()
                .any(|t| t.incarnation == 0 || !seen.insert(t.entity))
            || (self.affect_limit != 0
                && u64::try_from(self.targets.len()).map_err(|_| EffectError::Overflow)?
                    > u64::from(self.affect_limit))
        {
            return Err(EffectError::Invalid("invalid cast target bindings"));
        }
        // Missing/stale target entities are legal at a boundary; impact revalidates and
        // cancels. Snapshot restoration must never retarget them to a later incarnation.
        Ok(())
    }
}

impl Phase3Player {
    fn validate(
        &self,
        rules: &Phase3Rules,
        context: Phase3EntityContext<'_>,
    ) -> Result<(), EffectError> {
        if self.death_penalty_level > 5
            || self.pvp.flag_until.is_some_and(|at| at < context.next_tick)
            || self.last_death.as_ref().is_some_and(|d| {
                d.incarnation == 0
                    || d.incarnation > context.incarnation
                    || d.recoverable_death_xp_loss > d.pre_death_xp
            })
        {
            return Err(EffectError::Invalid("invalid live player policy"));
        }
        if let Some(offer) = &self.resurrect_offer {
            let skill = rules
                .skills()
                .get(offer.skill.id)
                .ok_or(EffectError::Invalid("unknown offer skill"))?;
            if context.alive
                || offer.id.0 == 0
                || offer.caster.incarnation == 0
                || offer.target_incarnation != context.incarnation
                || offer.created_at >= context.next_tick
                || offer.expires_at < context.next_tick
                || offer.expires_at.0.checked_sub(offer.created_at.0) != Some(600)
                || !(0..=100_000_000).contains(&offer.restore_percent_q)
                || skill.runtime_status != RuntimeSkillStatus::Executable
                || offer.skill.rank == 0
                || offer.skill.rank > skill.max_level
                || !skill.effects.iter().any(|h| h.name == "Resurrection")
                || self.last_death.as_ref().is_none_or(|d| {
                    d.incarnation != context.incarnation
                        || offer.recoverable_xp != d.recoverable_death_xp_loss
                })
            {
                return Err(EffectError::Invalid("invalid live resurrection offer"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::zone::effects::*;
    use crate::domain::zone::{EntityId, Phase3Snapshot};
    use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};

    fn rules() -> Phase3Rules {
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap()
    }

    pub(crate) fn blank() -> Phase3Entity {
        Phase3Entity {
            ordinal: EntityOrdinal(1),
            cast: CastState::Idle,
            reuse: BTreeMap::new(),
            effects: EffectList::default(),
            modifier_install_order: BTreeMap::new(),
            regen: RegenSchedule {
                next_at: Some(Tick(30)),
            },
            fractions: ResourceFractions::default(),
            player: Some(Phase3Player {
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
        }
    }

    fn context(learned: &[LearnedSkill]) -> Phase3EntityContext<'_> {
        Phase3EntityContext {
            next_tick: Tick(1),
            incarnation: 1,
            alive: true,
            player: true,
            learned,
        }
    }

    pub(crate) fn source_effect(rules: &Phase3Rules, id: u64, skill_id: u32) -> ActiveEffect {
        let skill = rules.skills().get(SkillId(skill_id)).unwrap();
        let abnormal = skill.abnormal.as_ref().unwrap();
        let flag =
            |v: &crate::domain::zone::skill::RankValue<bool>| *v.at(1, skill.max_level).unwrap();
        ActiveEffect {
            id: EffectId(id),
            skill: SkillRef {
                id: skill.id,
                rank: 1,
            },
            source: EffectSource {
                binding: None,
                kind: EffectSourceKind::System,
            },
            target_incarnation: 1,
            applied_at: Tick(0),
            expires_at: Some(Tick(1)),
            pool: if abnormal.is_debuff {
                EffectPool::Debuff
            } else {
                EffectPool::Buff
            },
            abnormal_type: abnormal.r#type.clone(),
            abnormal_level: abnormal.level[0],
            flags: EffectFlags {
                stay_after_death: flag(&skill.stay_after_death),
                stay_on_subclass_change: flag(&skill.stay_on_subclass_change),
                removed_on_damage: flag(&skill.removed_on_damage),
                removed_on_action_except_move: flag(&skill.removed_on_action_except_move),
                irreplaceable: flag(&skill.irreplaceable_buff),
            },
            handlers: skill
                .effects
                .iter()
                .enumerate()
                .filter(|(_, h)| continuous_handler(&h.name))
                .map(|(i, h)| ActiveHandler {
                    xml_ordinal: u32::try_from(i).unwrap(),
                    periodic: compile_periodic(h, 1, skill.max_level).unwrap().map(
                        |(resource, power, interval)| ActivePeriodic {
                            resource,
                            power,
                            schedule: PulseSchedule::new(Tick(0), interval).unwrap(),
                        },
                    ),
                })
                .collect(),
        }
    }

    pub(crate) fn cast() -> CastInFlight {
        CastInFlight {
            id: CastId(1),
            caster_incarnation: 1,
            skill: SkillRef {
                id: SkillId(1177),
                rank: 1,
            },
            command_ordinal: Ordinal(0),
            start_at: Tick(0),
            animation: FrozenRatio::new(super::super::skill_math::ExactRatio::whole(1200)),
            launch_at: Tick(8),
            complete_at: Tick(12),
            interrupt_until: Tick(10),
            launched: false,
            target: Some(TargetBinding {
                entity: EntityId::from_uuid(uuid::Uuid::from_u128(77)),
                incarnation: 1,
            }),
            affect_limit: 0,
            targets: vec![TargetBinding {
                entity: EntityId::from_uuid(uuid::Uuid::from_u128(77)),
                incarnation: 1,
            }],
            mp_start: 1,
            mp_end: 2,
            hp_end: 0,
            reuse_until: Tick(100),
        }
    }

    #[test]
    fn snapshot_keeps_short_timers_and_due_boundary_without_admission_filtering() {
        let rules = rules();
        let mut entity = blank();
        entity.reuse.insert(SkillId(1177), Tick(1));
        let effect = source_effect(&rules, 1, 110);
        entity.effects = EffectList::from_entries(BTreeMap::from([(effect.id, effect)])).unwrap();
        entity
            .modifier_install_order
            .insert(1, RestoredModifierSource::Effect(EffectId(1)));
        entity.regen.next_at = Some(Tick(1));
        entity.validate_live(&rules, context(&[])).unwrap();
        let restored: Phase3Entity =
            serde_json::from_slice(&serde_json::to_vec(&entity).unwrap()).unwrap();
        assert_eq!(restored, entity);
        assert_eq!(restored.effects.expired(Tick(1)), [(Tick(1), EffectId(1))]);
        for field in 0..3 {
            let mut overdue = entity.clone();
            match field {
                0 => {
                    overdue.reuse.insert(SkillId(1177), Tick(0));
                },
                1 => {
                    overdue.regen.next_at = Some(Tick(0));
                },
                _ => {
                    overdue.fractions.hp = super::super::Q;
                },
            }
            assert!(overdue.validate_live(&rules, context(&[])).is_err());
        }
    }

    #[test]
    fn cast_restore_preserves_independent_interrupt_window_and_stale_binding() {
        let rules = rules();
        let learned = [LearnedSkill {
            key: "l2.skill.1177".into(),
            level: 1,
        }];
        let mut entity = blank();
        entity.cast = CastState::Casting(cast());
        entity.validate_live(&rules, context(&learned)).unwrap();
        for field in 0..5 {
            let mut broken = entity.clone();
            if let CastState::Casting(cast) = &mut broken.cast {
                match field {
                    0 => cast.interrupt_until = Tick(11),
                    1 => cast.launched = true,
                    2 => cast.caster_incarnation = 2,
                    3 => cast.targets.push(cast.targets[0]),
                    _ => cast.skill.rank = 0,
                }
            }
            assert!(broken.validate_live(&rules, context(&learned)).is_err());
        }
        let mut dead = context(&learned);
        dead.alive = false;
        entity.regen.stop();
        assert!(entity.validate_live(&rules, dead).is_err());
        entity.cast = CastState::Idle;
        entity.validate_live(&rules, dead).unwrap();
    }

    #[test]
    fn zone_allocation_and_sidecar_bijection_are_checked() {
        let mut snapshot = Phase3Snapshot::new(rules());
        snapshot.next_entity_ordinal = 3;
        let a = EntityId::from_uuid(uuid::Uuid::from_u128(2));
        let b = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        snapshot.entities.insert(a, blank());
        let mut other = blank();
        other.ordinal = EntityOrdinal(2);
        snapshot.entities.insert(b, other);
        let combatants = BTreeMap::from([(a, context(&[])), (b, context(&[]))]);
        snapshot.validate_live(&combatants).unwrap();
        let encoded = serde_json::to_vec(&snapshot).unwrap();
        let decoded: Phase3Snapshot = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(snapshot, decoded);
        assert!(snapshot
            .validate_live(&BTreeMap::from([(a, context(&[]))]))
            .is_err());
        snapshot.entities.get_mut(&b).unwrap().ordinal = EntityOrdinal(1);
        assert!(snapshot.validate_live(&combatants).is_err());
        snapshot.entities.get_mut(&b).unwrap().ordinal = EntityOrdinal(3);
        assert!(snapshot.validate_live(&combatants).is_err());
    }

    #[test]
    fn global_work_order_is_deadline_then_allocation_and_expiry_wins() {
        let mut snapshot = Phase3Snapshot::new(rules());
        let a = EntityId::from_uuid(uuid::Uuid::from_u128(2));
        let b = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let mut first = blank();
        first.regen.next_at = Some(Tick(30));
        first.cast = CastState::Casting(cast());
        let mut second = first.clone();
        second.ordinal = EntityOrdinal(2);
        second.regen.next_at = Some(Tick(20));
        if let CastState::Casting(c) = &mut second.cast {
            c.id = CastId(2);
        }
        for (entity, id) in [(&mut first, 1), (&mut second, 2)] {
            let mut effect = source_effect(&snapshot.rules, id, 110);
            effect.expires_at = Some(Tick(7));
            // Synthetic scheduling probe: source payload validation is tested separately.
            effect.handlers = vec![ActiveHandler {
                xml_ordinal: 0,
                periodic: Some(ActivePeriodic {
                    resource: PeriodicResource::Hp,
                    power: PeriodicPower::Diff {
                        amount: FrozenRatio::new(super::super::skill_math::ExactRatio::whole(1)),
                    },
                    schedule: PulseSchedule::new(Tick(0), 666).unwrap(),
                }),
            }];
            entity.effects =
                EffectList::from_entries(BTreeMap::from([(effect.id, effect)])).unwrap();
        }
        snapshot.entities = BTreeMap::from([(a, first), (b, second)]);
        assert_eq!(
            snapshot
                .effect_expiries(Tick(7))
                .iter()
                .map(|w| w.target)
                .collect::<Vec<_>>(),
            [a, b]
        );
        assert!(snapshot.effect_pulses(Tick(7)).unwrap().is_empty());
        assert_eq!(
            snapshot.regen_candidates(Tick(30)),
            [(EntityOrdinal(1), a), (EntityOrdinal(2), b)]
        );
        assert_eq!(
            snapshot
                .cast_launches(Tick(8))
                .iter()
                .map(|w| w.caster)
                .collect::<Vec<_>>(),
            [a, b]
        );
        assert!(snapshot.cast_impacts(Tick(12)).is_empty());
        for e in snapshot.entities.values_mut() {
            if let CastState::Casting(c) = &mut e.cast {
                c.launched = true;
            }
        }
        assert!(snapshot.cast_launches(Tick(12)).is_empty());
        assert_eq!(
            snapshot
                .cast_impacts(Tick(12))
                .iter()
                .map(|w| w.caster)
                .collect::<Vec<_>>(),
            [a, b]
        );
    }
}
