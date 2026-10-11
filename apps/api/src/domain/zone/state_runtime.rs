//! Live V5 integration. The legacy driver never calls these paths. Admission builds
//! canonical sidecars; casts use the same scheduler, formulas and resource owners as replay.

#[path = "state_runtime_effects.rs"]
mod live_effects;

use super::*;
use crate::domain::zone::effective_stats::{EffectiveStats, NpcStatInputs, PlayerStatInputs};
use crate::domain::zone::effects::{EffectList, EffectSourceKind, TargetBinding};
use crate::domain::zone::skill::SkillDef;
use crate::domain::zone::skill_math::{self, ExactRatio, MagicResolution};
use crate::domain::zone::stat_functions::FunctionConditions;
use crate::domain::zone::state_damage::{
    self, DamageCause, DamageMaxima, DamageReduction, DamageResources,
};
use crate::domain::zone::state_pvp::{
    hostility_allowed, HostilityClassification, HostilityEndpoint, HostilityError,
};
use crate::domain::zone::state_regen::RegenSchedule;
use crate::domain::zone::state_skill::{
    self, CapturedTarget, CastCancelReason, CastError, CastStart, CastStatContext, StagedCast,
    StagedCasts,
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
    fn sidecar(&self, id: EntityId) -> Result<&Phase3Entity, TickError> {
        self.phase3
            .as_ref()
            .and_then(|p| p.canonical.entities.get(&id))
            .ok_or_else(missing)
    }
    fn sidecar_mut(&mut self, id: EntityId) -> Result<&mut Phase3Entity, TickError> {
        self.phase3
            .as_mut()
            .and_then(|p| p.canonical.entities.get_mut(&id))
            .ok_or_else(missing)
    }
    fn binding(&self, id: EntityId) -> Option<TargetBinding> {
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
        let stats = self.rules.as_deref().ok_or(StatError::Overflow)?;
        let rules = &self
            .phase3
            .as_ref()
            .ok_or(StatError::Overflow)?
            .canonical
            .rules;
        let combat = e.combat.as_ref().ok_or(StatError::Overflow)?;
        match &combat.role {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => PlayerStatInputs::from_class(
                stats,
                self.classes.as_deref().ok_or(StatError::Overflow)?,
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
    fn effective(&self, id: EntityId) -> Result<EffectiveStats, TickError> {
        self.effective_for(self.entities.get(&id).ok_or_else(missing)?, self.sidecar(id)?)
            .map_err(invariant)
    }
    fn rebuild_v5(&mut self, id: EntityId) -> Result<(), TickError> {
        let effective = self.effective(id)?;
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
        e.speed = effective.movement_speed().map_err(invariant)?;
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
        Ok(())
    }
    fn attach_v5(&mut self, tick: Tick, id: EntityId) -> Result<(), TickError> {
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
        for skill in p.rules.skills().skills().values().filter(|s| {
            s.operate_type == "P"
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
        let sidecar = Phase3Entity {
            ordinal: EntityOrdinal(p.next_entity_ordinal),
            cast: CastState::Idle,
            reuse: BTreeMap::new(),
            effects: EffectList::default(),
            modifier_install_order: sources,
            regen: RegenSchedule::start(tick, c.hp != 0, p.rules.regen()).map_err(invariant)?,
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
        p.next_entity_ordinal = p.next_entity_ordinal.checked_add(1).ok_or_else(missing)?;
        p.entities.insert(id, sidecar);
        self.rebuild_v5(id)
    }
    #[allow(clippy::too_many_lines)] // Full context validation stays adjacent to reconstruction.
    pub(super) fn validate_runtime_v5(&self) -> Result<(), SnapshotError> {
        let phase = self.phase3.as_ref().ok_or(SnapshotError::CombatMismatch)?;
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
                            alive: c.hp != 0,
                            player: e.kind == EntityKind::Player,
                            learned,
                        },
                    )
                })
            })
            .collect();
        phase
            .canonical
            .validate_live(&contexts)
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
            }) || c.swing.is_some()
                || side.player.as_ref().is_some_and(|p| {
                    p.death_penalty_level != 0
                        || p.last_death.is_some()
                        || p.resurrect_offer.is_some()
                        || p.pvp.flag_until.is_some()
                        || p.pvp.karma != 0
                        || p.pvp.pk_count != 0
                        || p.pvp.pvp_count != 0
                })
                || matches!(&side.cast, CastState::Casting(cast) if phase.canonical.rules.skills().get(cast.skill.id).is_none_or(|s| !supported_cast(s, cast.skill.rank)))
            {
                return Err(SnapshotError::CombatMismatch);
            }
            let effective = self
                .effective_for(e, side)
                .map_err(|_| SnapshotError::CombatMismatch)?;
            if c.sheet != effective.physical
                || e.speed
                    != effective
                        .movement_speed()
                        .map_err(|_| SnapshotError::CombatMismatch)?
                || c.hp > c.sheet.max_hp()
                || c.mp > c.sheet.max_mp()
                || e.targeting.dead != (c.hp == 0)
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
                    .validate_for_phase3(
                        self.classes
                            .as_deref()
                            .ok_or(SnapshotError::CombatMismatch)?,
                        &p.identity,
                        crate::domain::CharacterId::from_uuid(id.as_uuid()),
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
        let applied = candidate.execute_v5(draft)?;
        *self = candidate;
        Ok(applied)
    }
    #[allow(clippy::too_many_lines)] // Ordered live phase barriers remain visible in one driver.
    fn execute_v5(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError> {
        // Rollout remains fenced at actor bootstrap. Unimplemented policy combinations fail
        // before mutation rather than passing through legacy combat as if it were V5.
        if !self.slots.is_empty()
            || draft.commands.iter().any(|c| {
                matches!(
                    c.command,
                    ZoneCommand::Attack { .. }
                        | ZoneCommand::AddAggro { .. }
                        | ZoneCommand::AcceptResurrect { .. }
                        | ZoneCommand::LearnSkill { .. }
                        | ZoneCommand::ChangeClass { .. }
                        | ZoneCommand::Respawn { .. }
                )
            })
        {
            return Err(TickError::Phase3NotReady);
        }
        let tick = draft.tick;
        let before = self.entities.clone();
        let known = self.known.clone();
        let mut events = Vec::new();
        let mut dispositions = Vec::new();
        let mut staged = StagedCasts::default();
        for command in &draft.commands {
            let result = self.command_v5(tick, command, &mut staged, &mut events);
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
        let ids: Vec<_> = self.entities.keys().copied().collect();
        for id in ids {
            let fresh = self
                .phase3
                .as_ref()
                .is_some_and(|p| !p.canonical.entities.contains_key(&id));
            self.attach_v5(tick, id)?;
            if fresh {
                let load = draft
                    .commands
                    .iter()
                    .filter(|c| !dispositions.iter().any(|d| d.ordinal == c.ordinal))
                    .find_map(|c| {
                        if let ZoneCommand::SpawnPlayer { entity, load, .. } = &c.command {
                            if *entity == id {
                                return load.as_deref();
                            }
                        }
                        None
                    });
                if let Some(load) = load {
                    let c = self
                        .entities
                        .get_mut(&id)
                        .and_then(|e| e.combat.as_mut())
                        .ok_or_else(missing)?;
                    // Admission clamps the original load only against the final effective
                    // sheet; the temporary legacy base constructor must not lose HP/MP.
                    c.hp = if load.alive {
                        load.hp
                            .unwrap_or(c.sheet.max_hp())
                            .clamp(1, c.sheet.max_hp())
                    } else {
                        0
                    };
                    c.mp = load.mp.unwrap_or(c.sheet.max_mp()).min(c.sheet.max_mp());
                    if let (
                        CombatRole::Player {
                            progression: Some(p),
                            ..
                        },
                        Some(original),
                    ) = (&mut c.role, &load.progression)
                    {
                        p.class_state.cp = original.class_state.cp.min(p.max_cp);
                    }
                }
            }
        }
        self.expire_effects_v5(tick, &mut events)?;
        self.expire_reuse_v5(tick, &mut events)?;
        self.pulse_effects_v5(tick, &mut events)?;
        self.regen_v5(tick)?;
        events.extend(self.step(tick));
        self.complete_casts_v5(tick, &mut events)?;
        for entry in staged.drain() {
            if entry.canceled.is_some() {
                continue;
            }
            let ordinal = entry.ordinal;
            if let Err(reason) = self.start_cast_v5(tick, &entry, &mut events) {
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
        self.flush_progression(tick, &mut events);
        self.final_resources_v5(tick, &before, &mut events)?;
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .vitals_dirty
            .clear();
        dispositions.sort_by_key(|d| d.ordinal);
        let responses = responses(tick, &draft.commands, &dispositions);
        let outputs = self.observe_v5(tick, responses, &events, &known)?;
        self.next_tick = tick.next();
        let phase = self.phase3.as_mut().ok_or_else(missing)?;
        phase.schedule =
            crate::domain::zone::state_effects::Phase3Schedule::rebuild(&phase.canonical.entities)
                .map_err(invariant)?;
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
    fn command_v5(
        &mut self,
        tick: Tick,
        command: &AppliedCommand,
        staged: &mut StagedCasts,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        self.authorize(command.source, &command.command)?;
        self.refuse_dead_actor(command.source, &command.command)?;
        if let ZoneCommand::SpawnNpc {
            combat: Some(spec),
            speed,
            ..
        } = &command.command
        {
            let profile = self
                .phase3
                .as_ref()
                .and_then(|p| p.canonical.rules.profiles().npcs.get(&spec.template))
                .ok_or(RejectReason::InvalidLoad)?;
            let raw = &profile.physical;
            if spec.stats.level != raw.level
                || spec.stats.max_hp != raw.max_hp
                || spec.stats.max_mp != raw.max_mp
                || spec.stats.p_atk.raw() != raw.p_atk_q
                || spec.stats.p_def.raw() != raw.p_def_q
                || spec.stats.attack_speed.raw() != i64::from(raw.attack_speed).saturating_mul(Q)
                || *speed != raw.move_speed
            {
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
            ZoneCommand::AcceptResurrect { .. }
            | ZoneCommand::LearnSkill { .. }
            | ZoneCommand::ChangeClass { .. }
            | ZoneCommand::SpawnPlayer { .. }
            | ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::ReplaceSession { .. }
            | ZoneCommand::MoveTo { .. }
            | ZoneCommand::SetTarget { .. }
            | ZoneCommand::Attack { .. }
            | ZoneCommand::StopAttack { .. }
            | ZoneCommand::Respawn { .. }
            | ZoneCommand::StopMove { .. }
            | ZoneCommand::AddAggro { .. } => {
                let applied = self.apply(tick, command)?;
                events.extend(applied);
                match command.command {
                    ZoneCommand::MoveTo { entity, .. } | ZoneCommand::StopMove { entity } => {
                        staged.cancel(entity, None, CastCancelReason::Moved);
                        self.cancel_v5(tick, entity, None, CastCancelReason::Moved, events);
                    },
                    ZoneCommand::Despawn { entity } => {
                        if let Some(p) = &mut self.phase3 {
                            p.canonical.entities.remove(&entity);
                        }
                    },
                    ZoneCommand::CastSkill { .. }
                    | ZoneCommand::CancelCast { .. }
                    | ZoneCommand::AcceptResurrect { .. }
                    | ZoneCommand::LearnSkill { .. }
                    | ZoneCommand::ChangeClass { .. }
                    | ZoneCommand::SpawnPlayer { .. }
                    | ZoneCommand::SpawnNpc { .. }
                    | ZoneCommand::ReplaceSession { .. }
                    | ZoneCommand::SetTarget { .. }
                    | ZoneCommand::Attack { .. }
                    | ZoneCommand::StopAttack { .. }
                    | ZoneCommand::Respawn { .. }
                    | ZoneCommand::AddAggro { .. } => {},
                }
            },
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
        let Some(side) = self
            .phase3
            .as_mut()
            .and_then(|p| p.canonical.entities.get_mut(&entity))
        else {
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
                alive: c.hp != 0,
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
    fn cast_target_v5(
        &self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        range: i64,
        hostile: bool,
    ) -> Result<(), RejectReason> {
        if hostile {
            self.hostility_v5(tick, caster, target)?;
        } else if self.binding(target.entity) != Some(target)
            || self
                .entities
                .get(&target.entity)
                .is_none_or(|e| e.targeting.dead)
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
            let radius = i128::from(range)
                .checked_mul(1000)
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
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let e = self
            .entities
            .get(&entry.caster.entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let c = e.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        if c.incarnation != entry.caster.incarnation || c.hp == 0 {
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
        if !supported_cast(skill, reference.rank) {
            return Err(RejectReason::NotYetImplemented);
        }
        let skill = skill.clone();
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
        if hostile_skill(&skill)
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
        self.cast_target_v5(tick, entry.caster, target, range, hostile_skill(&skill))?;
        let effective = self
            .effective(entry.caster.entity)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let terms = state_skill::cast_terms(
            &skill,
            reference.rank,
            CastStatContext {
                calculator: &effective.calculator,
                conditions: FunctionConditions {
                    weapon_kind: "SWORD",
                    can_resurrect: true,
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
        let phase = self.phase3.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let side = phase
            .canonical
            .entities
            .get_mut(&entry.caster.entity)
            .ok_or(RejectReason::InvalidLoad)?;
        let c = self
            .entities
            .get_mut(&entry.caster.entity)
            .and_then(|e| e.combat.as_mut())
            .ok_or(RejectReason::InvalidLoad)?;
        let cast = side
            .begin_cast(
                &mut phase.canonical.next_cast_id,
                &mut c.mp,
                c.hp,
                CastStart {
                    now: tick,
                    ordinal: entry.ordinal,
                    incarnation: c.incarnation,
                    terms,
                    target: Some(target),
                    affect_limit: 0,
                    targets: vec![target],
                },
            )
            .map_err(cast_reject)?;
        if hostile_skill(&skill) {
            c.protected_until = None;
        }
        side.launch_cast(tick, cast.id, |_| true);
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
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let launches = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
            .cast_launches(tick);
        for work in launches {
            let CastState::Casting(cast) = self.sidecar(work.caster)?.cast.clone() else {
                continue;
            };
            let rules = &self.phase3.as_ref().ok_or_else(missing)?.canonical.rules;
            let skill = rules.skills().get(cast.skill.id).ok_or_else(missing)?;
            let range = *skill
                .effect_range
                .at(cast.skill.rank, skill.max_level)
                .ok_or_else(missing)?;
            let target = cast.target.ok_or_else(missing)?;
            let caster = TargetBinding {
                entity: work.caster,
                incarnation: cast.caster_incarnation,
            };
            // L2Character.java:4500-4579: the launch filters captured targets; an empty
            // target set aborts before MP2 or damage draws, rather than waiting for impact.
            if let Err(reason) =
                self.cast_target_v5(tick, caster, target, range, hostile_skill(skill))
            {
                self.cancel_v5(tick, work.caster, Some(cast.id), target_cancel(reason), events);
                continue;
            }
            self.sidecar_mut(work.caster)?
                .launch_cast(tick, work.cast, |_| true);
        }
        let impacts = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
            .cast_impacts(tick);
        for work in impacts {
            let CastState::Casting(cast) = self.sidecar(work.caster)?.cast.clone() else {
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
            let Some(target) = cast.target else {
                return Err(missing());
            };
            let range = *skill
                .effect_range
                .at(cast.skill.rank, skill.max_level)
                .ok_or_else(missing)?;
            if let Err(reason) =
                self.cast_target_v5(tick, caster, target, range, hostile_skill(&skill))
            {
                self.cancel_v5(tick, work.caster, Some(cast.id), target_cancel(reason), events);
                continue;
            }
            // L2Character.java:4600-4606: skill effectPoint is applied once per
            // eligible target before impact costs; it is separate from damage hate.
            let effect_point = *skill
                .effect_point
                .get(usize::try_from(cast.skill.rank.saturating_sub(1)).map_err(invariant)?)
                .ok_or_else(missing)?;
            if effect_point < 0 {
                self.add_hate(
                    tick,
                    target.entity,
                    caster.entity,
                    super::super::HateEntry {
                        hate: effect_point.unsigned_abs(),
                        damage: 0,
                    },
                    events,
                );
            }
            let stats = self.effective(work.caster)?;
            let (_, mp, hp) = state_skill::cast_costs(
                &skill,
                cast.skill.rank,
                CastStatContext {
                    calculator: &stats.calculator,
                    conditions: FunctionConditions {
                        weapon_kind: "SWORD",
                        can_resurrect: true,
                    },
                    attack_speed: 1,
                    cast_speed: stats.cast_speed,
                    flags: stats.flags,
                },
            )
            .map_err(invariant)?;
            let side = self
                .phase3
                .as_mut()
                .and_then(|p| p.canonical.entities.get_mut(&work.caster))
                .ok_or_else(missing)?;
            let c = self
                .entities
                .get_mut(&work.caster)
                .and_then(|e| e.combat.as_mut())
                .ok_or_else(missing)?;
            match side.pay_cast_impact(tick, cast.id, &mut c.mp, &mut c.hp, mp, hp) {
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
            let continuous = self.continuous_terms_v5(caster, target, &skill, cast.skill.rank)?;
            let mut results = Vec::new();
            for handler in skill
                .effects
                .iter()
                .filter(|h| !crate::domain::zone::effects::continuous_handler(&h.name))
            {
                let resolve = if handler.name == "DispelBySlot" {
                    Self::dispel_handler_v5
                } else if matches!(handler.name.as_str(), "Heal" | "HpPerMax" | "Mp" | "Cp") {
                    Self::restore_handler_v5
                } else {
                    Self::magic_handler_v5
                };
                results.push(resolve(
                    self,
                    tick,
                    caster,
                    target,
                    &skill,
                    cast.skill.rank,
                    handler,
                    events,
                )?);
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
                    results.push(self.install_effect_v5(
                        tick,
                        caster,
                        target,
                        &skill,
                        cast.skill.rank,
                        duration,
                        events,
                    )?);
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
            self.sidecar_mut(work.caster)?.finish_cast(cast.id);
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
    fn magic_handler_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        let a = self.effective(caster.entity)?;
        let b = self.effective(target.entity)?;
        let critical = self.roll_below(1000) < a.magic_crit;
        let magic_level = *skill
            .magic_level
            .get(usize::try_from(rank.saturating_sub(1)).map_err(invariant)?)
            .ok_or_else(missing)?;
        let difference = i32::try_from(b.physical.level())
            .map_err(invariant)?
            .checked_sub(if magic_level > 0 {
                i32::try_from(magic_level).map_err(invariant)?
            } else {
                i32::try_from(a.physical.level()).map_err(invariant)?
            })
            .ok_or_else(missing)?;
        let rate =
            skill_math::magic_success_rate(difference, ExactRatio::whole(1), ExactRatio::whole(1))
                .map_err(invariant)?;
        let success = i64::from(self.roll_below(100)) < i64::from(rate);
        let resolution = if success {
            MagicResolution::Success
        } else if i64::from(self.roll_below(100)) < i64::from(rate)
            && b.physical.level().saturating_sub(a.physical.level()) <= 9
        {
            MagicResolution::Half
        } else {
            MagicResolution::Resist
        };
        let radius = a.physical.random_damage();
        let spread = if radius == 0 {
            0
        } else {
            i64::from(self.roll_below(radius.saturating_mul(2).saturating_add(1)))
                .saturating_sub(i64::from(radius))
        };
        let power = ExactRatio::parse_literal(
            handler
                .params
                .get("power")
                .and_then(|p| p.at(rank, skill.max_level))
                .ok_or_else(missing)?,
        )
        .map_err(invariant)?;
        let damage = skill_math::magical_skill_base(
            power,
            q(a.m_atk_q).map_err(invariant)?,
            q(b.m_def_q).map_err(invariant)?,
            resolution,
            critical,
            self.sidecar(target.entity)?.player.is_some(),
        )
        .map_err(invariant)?
        .checked_mul(
            ExactRatio::new(i128::from(100_i64.saturating_add(spread)), 100).map_err(invariant)?,
        )
        .map_err(invariant)?;
        let positive = damage
            .checked_cmp(ExactRatio::whole(0))
            .map_err(invariant)?
            == std::cmp::Ordering::Greater;
        let rolled =
            u32::try_from(damage.floor_scaled(1).map_err(invariant)?).map_err(invariant)?;
        // MagicalAttackRange.java:83,89: attack-break (idle -> no draw), then vengeance
        // even at zero chance, then recipient status/stun/resource/death processing.
        if matches!(self.sidecar(target.entity)?.cast, CastState::Casting(_)) {
            return Err(missing());
        }
        let result = if positive {
            self.roll_below(100);
            self.apply_damage_v5(tick, caster, target, rolled, DamageCause::Direct, events)?
        } else {
            (
                0,
                0,
                self.entities
                    .get(&target.entity)
                    .and_then(|e| e.combat.as_ref())
                    .ok_or_else(missing)?
                    .hp,
            )
        };
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Magical,
            outcome: match resolution {
                MagicResolution::Half => SkillOutcome::Half,
                MagicResolution::Resist => SkillOutcome::Resist,
                MagicResolution::Success if critical => SkillOutcome::Crit,
                MagicResolution::Success => SkillOutcome::Hit,
            },
            rolled,
            cp_absorbed: result.0,
            hp_removed: result.1,
            target_hp_after: result.2,
            heal: 0,
            applied_effect_ids: Vec::new(),
        })
    }
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // One recipient transaction preserves status/RNG/death order.
    fn apply_damage_v5(
        &mut self,
        tick: Tick,
        source: TargetBinding,
        target: TargetBinding,
        rolled_damage: u32,
        cause: DamageCause,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(u32, u32, u32), TickError> {
        let classification = if cause == DamageCause::Periodic
            && (self.binding(source.entity) != Some(source)
                || self
                    .entities
                    .get(&source.entity)
                    .is_none_or(|e| e.targeting.dead))
        {
            // Retained attribution is valid after departure; pulses already checked geometry.
            HostilityClassification {
                absorb_cp: self
                    .entities
                    .get(&target.entity)
                    .is_some_and(|e| e.kind == EntityKind::Player),
            }
        } else {
            self.hostility_v5(tick, source, target).map_err(invariant)?
        };
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
        // Player death/PvP checkpoint integration is not yet exposed by bootstrap.
        if cp.is_some() {
            return Err(missing());
        }
        let side = self.sidecar(target.entity)?.clone();
        let rules = Arc::clone(&self.phase3.as_ref().ok_or_else(missing)?.canonical.rules);
        let mut rng = self.rng.clone();
        let mutation = state_damage::prepare_hp_reduction(
            &side,
            DamageResources {
                hp: c.hp,
                mp: c.mp,
                cp,
            },
            DamageReduction {
                cause,
                source_kind: EffectSourceKind::Player,
                classification,
                awake: true,
                rolled_damage,
            },
            &rules,
            |bound| {
                let span = 1_u64 << 32;
                let bound = u64::from(bound);
                let limit = span
                    .checked_sub(span.checked_rem(bound).ok_or(StatError::Overflow)?)
                    .ok_or(StatError::Overflow)?;
                loop {
                    let n = u64::from(rng.next_u32());
                    if n < limit {
                        return u32::try_from(n.checked_rem(bound).ok_or(StatError::Overflow)?)
                            .map_err(|_| StatError::Overflow);
                    }
                }
            },
            |side| {
                let effective = self.effective_for(&recipient, side)?;
                Ok(DamageMaxima {
                    hp: effective.physical.max_hp(),
                    mp: effective.physical.max_mp(),
                    cp: cp.map(|_| effective.max_cp),
                })
            },
        )
        .map_err(invariant)?;
        if mutation.hp_removed > 0 {
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .vitals_dirty
                .insert(target.entity);
        }
        self.rng = rng;
        *self.sidecar_mut(target.entity)? = mutation.phase3;
        for old in &mutation.removed_effects {
            events.push(ZoneEvent::EffectRemoved {
                tick,
                target,
                effect_id: old.id,
                reason: crate::domain::zone::effects::EffectRemoveReason::Damage,
            });
        }
        self.rebuild_v5(target.entity)?;
        let c = self
            .entities
            .get_mut(&target.entity)
            .and_then(|e| e.combat.as_mut())
            .ok_or_else(missing)?;
        c.hp = mutation.resources.hp;
        c.mp = mutation.resources.mp;
        let hp = c.hp;
        if mutation.lethal {
            self.cancel_v5(tick, target.entity, None, CastCancelReason::CasterDied, events);
            let death = crate::domain::zone::state_death::prepare_death(
                self.sidecar(target.entity)?,
                crate::domain::zone::state_death::DeathContext {
                    incarnation: target.incarnation,
                    was_alive: true,
                    killer_kind: Some(EntityKind::Player),
                    inside_peace: false,
                    pre_death_xp: 0,
                    xp_lost: 0,
                },
                &rules,
                None,
                self.phase3
                    .as_ref()
                    .ok_or_else(missing)?
                    .canonical
                    .next_install_sequence,
            )
            .map_err(invariant)?;
            *self.sidecar_mut(target.entity)? = death.phase3;
            for old in death.removed_effects {
                events.push(ZoneEvent::EffectRemoved {
                    tick,
                    target,
                    effect_id: old.id,
                    reason: crate::domain::zone::effects::EffectRemoveReason::Death,
                });
            }
            self.rebuild_v5(target.entity)?;
            self.kill(
                tick,
                target.entity,
                Some(super::combat_phase::Blow {
                    attacker: source.entity,
                }),
                events,
            );
            self.credit_sp_v5(tick, source, &recipient, events)?;
            self.sync_passives_v5(source.entity)?;
            self.rebuild_v5(source.entity)?;
        } else {
            self.add_hate(
                tick,
                target.entity,
                source.entity,
                super::super::HateEntry {
                    hate: u64::from(mutation.hp_removed),
                    damage: u64::from(mutation.hp_removed),
                },
                events,
            );
        }
        Ok((mutation.cp_absorbed, mutation.hp_removed, hp))
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
        let due = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
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
                con_bonus: q(stats
                    .bonus()
                    .bonus(crate::domain::zone::StatKind::Con, base.con)
                    .map_err(invariant)?
                    .raw())
                .map_err(invariant)?,
                men_bonus: q(stats
                    .bonus()
                    .bonus(crate::domain::zone::StatKind::Men, base.men)
                    .map_err(invariant)?
                    .raw())
                .map_err(invariant)?,
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
                    weapon_kind: if player { "SWORD" } else { "NONE" },
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
    fn owner_stats_v5(
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
        for (id, e) in &self.entities {
            let Some(c) = &e.combat else { continue };
            let old = before.get(id).and_then(|e| e.combat.as_ref());
            let effects_changed = events.iter().any(|event| matches!(event,
                ZoneEvent::EffectApplied { target, .. } | ZoneEvent::EffectRemoved { target, .. } if target.entity == *id));
            if e.kind == EntityKind::Player && (old != Some(c) || effects_changed) {
                events.push(self.owner_stats_v5(tick, *id, None)?);
            }
            // A regen pulse followed by damage can return HP to its starting value;
            // landed removal still marks the public pair dirty for one final projection.
            let damaged = events.iter().any(|event| matches!(event,
                ZoneEvent::SkillCastResult { targets, .. } if targets.iter().any(|row|
                    row.target.entity == *id && row.target.incarnation == c.incarnation && row.hp_removed != 0)));
            if old.is_some_and(|old| {
                old.hp != c.hp
                    || old.sheet.max_hp() != c.sheet.max_hp()
                    || old.incarnation != c.incarnation
                    || damaged
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
        for (observer, (_, known)) in &self.known {
            let sees = |id: EntityId| id == *observer || known.binary_search(&id).is_ok();
            let out = outputs.entry(*observer).or_default();
            // Replacement resets the observer cache during phase0 even though this id was
            // known in the tick-start copy. Detect its actual identity batch too.
            let owner_baseline = out.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) if entity == observer));
            let knew = |id: EntityId| {
                !owner_baseline
                    && previous
                        .get(observer)
                        .is_some_and(|(_, ids)| ids.binary_search(&id).is_ok())
            };
            // Legacy observe already establishes the final AOI. Move its ordinary facts
            // behind the complete V5 hydration batch without changing their relative order.
            let mut ordinary = Vec::new();
            out.retain(|output| {
                if matches!(
                    output,
                    ObserverOutput::Accepted { .. }
                        | ObserverOutput::Rejected { .. }
                        | ObserverOutput::Event(
                            ZoneEvent::EntitySpawn { .. } | ZoneEvent::EntityDespawn { .. }
                        )
                ) {
                    true
                } else {
                    ordinary.push(output.clone());
                    false
                }
            });
            if owner_baseline && self.binding(*observer).is_some() {
                ordinary.retain(|o| !matches!(o, ObserverOutput::Event(ZoneEvent::StatsChanged { entity, .. }) if entity == observer));
                out.push(ObserverOutput::Event(self.owner_stats_v5(
                    tick,
                    *observer,
                    Some(tick),
                )?));
                out.push(ObserverOutput::Event(self.owner_sync_v5(tick, *observer)?));
            }
            for id in known.iter().filter(|id| !knew(**id)) {
                if let Ok(side) = self.sidecar(*id) {
                    for effect in side.effects.entries().values() {
                        let mut event = self.effect_applied_v5(tick, *id, effect, Some(tick))?;
                        if let ZoneEvent::EffectApplied { source, .. } = &mut event {
                            *source = source
                                .filter(|s| sees(s.entity) && self.binding(s.entity) == Some(*s));
                        }
                        out.push(ObserverOutput::Event(event));
                    }
                    if let CastState::Casting(cast) = &side.cast {
                        let mut event = cast_started(tick, *id, cast, Some(tick));
                        if let ZoneEvent::SkillCastStarted { target, .. } = &mut event {
                            *target = target
                                .filter(|t| sees(t.entity) && self.binding(t.entity) == Some(*t));
                        }
                        out.push(ObserverOutput::Event(event));
                    }
                }
            }
            out.extend(ordinary);
            for event in events {
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
                    ZoneEvent::SpGained { entity, .. } | ZoneEvent::SkillReuse { entity, .. }
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
        || skill.effects.iter().any(|h| {
            matches!(h.name.as_str(), "MagicalAttack" | "MagicalAttackRange" | "PhysicalAttack")
        })
}
fn supported_cast(skill: &SkillDef, rank: u32) -> bool {
    skill
        .affect_scope
        .at(rank, skill.max_level)
        .is_some_and(|s| matches!(s.as_str(), "SINGLE" | "unset"))
        && skill
            .target_type
            .at(rank, skill.max_level)
            .is_some_and(|s| matches!(s.as_str(), "SELF" | "TARGET" | "ENEMY" | "ENEMY_ONLY"))
        && ((skill.id.0 == 1177 && skill.abnormal.is_none())
            || skill.abnormal.is_none()
                && skill.effects.iter().all(|h| {
                    matches!(h.name.as_str(), "Heal" | "HpPerMax" | "Mp" | "Cp" | "DispelBySlot")
                })
            || skill.abnormal.is_some()
                && skill.effects.iter().all(|h| {
                    matches!(
                        h.name.as_str(),
                        "Buff"
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
                    )
                }))
        && skill.effects.iter().all(|h| {
            h.section == "effects"
                && h.conditions.is_empty()
                && h.section_conditions.is_empty()
                && (!matches!(h.name.as_str(), "MaxHp" | "MaxMp")
                    || (skill.effects.len() == 1 && h.stat_children.is_empty()))
        })
}
