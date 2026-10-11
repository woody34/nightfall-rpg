//! Live command-ordered learning (design v3 §5.3). The application resolves durable
//! receipts first and serializes mutations/lifecycle work; this module owns the debit,
//! passive installation and final immutable success fact, never a repository or reply.

use super::*;
use crate::domain::zone::skill::SkillRef;
use crate::domain::zone::state_learn::{prepare_learn, LearnContext, LearnError};
use crate::domain::zone::{FrozenLearnResult, LearnedEffectiveStats};
use crate::domain::{AccountId, CharacterId};

struct PreparedLearning {
    entity: Entity,
    sidecar: Phase3Entity,
    next_sequence: u64,
    spent: u64,
}

fn learn_reject(error: LearnError) -> RejectReason {
    match error {
        LearnError::InvalidState | LearnError::SequenceExhausted => RejectReason::InvalidLoad,
        LearnError::DeadActor => RejectReason::DeadActor,
        LearnError::InCombat => RejectReason::InCombat,
        LearnError::TrainerRequired => RejectReason::ClassMasterTooFar,
        LearnError::UnknownSkill => RejectReason::SkillUnknown,
        LearnError::NotLearnable => RejectReason::SkillNotLearnable,
        LearnError::WrongRank => RejectReason::SkillWrongRank,
        LearnError::LevelRequired => RejectReason::SkillLevelRequired,
        LearnError::ItemRequired => RejectReason::SkillItemRequired,
        LearnError::DeferredSkill => RejectReason::SkillDeferred,
        LearnError::InsufficientSp => RejectReason::InsufficientSp,
    }
}

impl ZoneState {
    /// Read-only exact-rank eligibility from current authoritative state. Checks account
    /// and all 64 generation bits, then the same guards/stat reconstruction as mutation.
    /// It consumes no SP, IDs, RNG, cache entries or receipts. The actor must recheck when
    /// executing the command; this query is not a reservation or a durable success.
    pub fn validate_learn_skill(
        &self,
        entity: EntityId,
        account: AccountId,
        generation: SessionGeneration,
        skill: SkillRef,
    ) -> Result<(), RejectReason> {
        self.prepare_live_learn(entity, account, generation, skill)
            .map(|_| ())
    }

    fn engaged_for_learning(&self, entity: EntityId) -> bool {
        let Some(e) = self.entities.get(&entity) else {
            return true;
        };
        let Some(c) = e.combat.as_ref() else {
            return true;
        };
        if c.auto_attack
            || c.chasing
            || c.swing.is_some()
            || c.ready_at > self.next_tick
            || self.hate.values().any(|h| h.get(entity).is_some())
            || self.entities.values().any(|other| {
                !other.targeting.dead
                    && other.combat.as_ref().is_some_and(|combat| {
                        (combat.auto_attack && other.targeting.target == Some(entity))
                            || combat.swing.is_some_and(|s| s.target == entity)
                    })
            })
        {
            return true;
        }
        let Some(phase) = &self.phase3 else {
            return true;
        };
        phase.canonical.entities.iter().any(|(id, side)| {
            // Hostile effects engage both the target and a still-present source. Captured
            // life bindings prevent an old NPC incarnation from engaging its replacement.
            side.effects.entries().values().any(|effect| {
                self.hostile_skill_v5(effect.skill.id)
                    && (*id == entity
                        || effect.source.binding.is_some_and(|source| {
                            source.entity == entity && self.binding(entity) == Some(source)
                        }))
            }) || match &side.cast {
                CastState::Idle => false,
                CastState::Casting(cast) => {
                    *id == entity
                        || (self.hostile_skill_v5(cast.skill.id)
                            && cast.targets.iter().any(|target| {
                                target.entity == entity && self.binding(entity) == Some(*target)
                            }))
                },
            }
        })
    }

    fn hostile_skill_v5(&self, skill: crate::domain::zone::skill::SkillId) -> bool {
        self.phase3
            .as_ref()
            .and_then(|phase| phase.canonical.rules.skills().get(skill))
            .is_some_and(hostile_skill)
    }

    fn prepare_live_learn(
        &self,
        id: EntityId,
        account: AccountId,
        generation: SessionGeneration,
        skill: SkillRef,
    ) -> Result<PreparedLearning, RejectReason> {
        let player = self.class_player(id, account, generation)?;
        let phase = self
            .phase3
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?;
        let e = self.entities.get(&id).ok_or(RejectReason::UnknownEntity)?;
        let c = e.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let side = phase
            .canonical
            .entities
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let mutation = prepare_learn(
            &player.class_state,
            side,
            LearnContext {
                character: CharacterId::from_uuid(id.as_uuid()),
                identity: &player.identity,
                classes: self.classes.as_deref().ok_or(RejectReason::InvalidLoad)?,
                rules: &phase.canonical.rules,
                level: c.sheet.level(),
                alive: !e.targeting.dead && c.hp != 0,
                engaged: self.engaged_for_learning(id),
                position: e.pos,
            },
            skill,
            phase.canonical.next_install_sequence,
        )
        .map_err(learn_reject)?;
        // Derive and narrow every fallible resource/speed projection before publishing any
        // ledger/allocation mutation. Only this entity and its sidecar are cloned.
        let mut entity = e.clone();
        let mut sidecar = mutation.phase3;
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut entity
            .combat
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .role
        else {
            return Err(RejectReason::InvalidLoad);
        };
        player.class_state = mutation.ledger;
        let effective = self
            .effective_for(&entity, &sidecar)
            .map_err(|_| RejectReason::InvalidLoad)?;
        entity.speed = effective
            .movement_speed()
            .map_err(|_| RejectReason::InvalidLoad)?;
        let combat = entity.combat.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut combat.role
        else {
            return Err(RejectReason::InvalidLoad);
        };
        player.max_cp = effective.max_cp;
        player.class_state.cp = player.class_state.cp.min(player.max_cp);
        combat.hp = combat.hp.min(effective.physical.max_hp());
        combat.mp = combat.mp.min(effective.physical.max_mp());
        combat.sheet = effective.physical;
        if combat.hp == combat.sheet.max_hp() {
            sidecar.fractions.hp = 0;
        }
        if combat.mp == combat.sheet.max_mp() {
            sidecar.fractions.mp = 0;
        }
        if player.class_state.cp == player.max_cp {
            sidecar.fractions.cp = 0;
        }
        Ok(PreparedLearning {
            entity,
            sidecar,
            next_sequence: mutation.next_modifier_sequence,
            spent: mutation.sp_spent,
        })
    }

    pub(super) fn learn_v5(
        &mut self,
        tick: Tick,
        command: &AppliedCommand,
        staged: &StagedCasts,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let ZoneCommand::LearnSkill {
            entity,
            account,
            request_key,
            skill_id,
            skill_level,
        } = command.command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        let CommandSource::Session {
            entity: source,
            generation,
        } = command.source
        else {
            return Err(RejectReason::NotPermitted);
        };
        if source != entity {
            return Err(RejectReason::NotPermitted);
        }
        // The application serializes this already; refusing a malformed recorded draft
        // prevents two exact-rank successes from sharing one final receipt projection.
        if events.iter().any(
            |e| matches!(e, ZoneEvent::SkillLearned { entity: learned, .. } if *learned == entity),
        ) {
            return Err(RejectReason::InCombat);
        }
        if staged.pending().any(|entry| {
            (entry.caster.entity == entity && self.binding(entity) == Some(entry.caster))
                || (self.hostile_skill_v5(entry.skill_id)
                    && self.binding(entry.caster.entity) == Some(entry.caster)
                    && entry.target.is_some_and(|t| {
                        t.entity == entity
                            && self
                                .binding(entity)
                                .is_some_and(|life| t.incarnation == Some(life.incarnation))
                    }))
        }) {
            return Err(RejectReason::InCombat);
        }
        let skill = SkillRef {
            id: skill_id,
            rank: skill_level,
        };
        let prepared = self.prepare_live_learn(entity, account, generation, skill)?;
        let old = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let old_combat = old.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let new_combat = prepared
            .entity
            .combat
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?;
        let vitals_changed = old_combat.hp != new_combat.hp
            || old_combat.sheet.max_hp() != new_combat.sheet.max_hp();
        let speed_changed = old.speed != prepared.entity.speed;
        // Seed the fact from the fully staged entity; end-of-tick replaces resources with
        // the final post-callback state. No key enters the canonical state or its digest.
        let result = self
            .learn_result_v5(tick, &prepared.entity, &prepared.sidecar, skill, prepared.spent)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let phase = self.phase3.as_mut().ok_or(RejectReason::InvalidLoad)?;
        phase.canonical.next_install_sequence = prepared.next_sequence;
        phase.canonical.entities.insert(entity, prepared.sidecar);
        if vitals_changed {
            phase.vitals_dirty.insert(entity);
        }
        if speed_changed {
            events.push(ZoneEvent::EntityMove {
                tick,
                entity,
                pos: prepared.entity.pos,
                dest: prepared.entity.dest,
                speed: prepared.entity.speed,
            });
        }
        self.entities.insert(entity, prepared.entity);
        events.push(ZoneEvent::SkillLearned {
            tick,
            entity,
            account,
            request_key,
            result: Box::new(result),
        });
        Ok(())
    }

    fn learn_result_v5(
        &self,
        tick: Tick,
        entity: &Entity,
        sidecar: &Phase3Entity,
        skill: SkillRef,
        spent: u64,
    ) -> Result<FrozenLearnResult, TickError> {
        let c = entity.combat.as_ref().ok_or_else(missing)?;
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            return Err(missing());
        };
        let stats = self.effective_for(entity, sidecar).map_err(invariant)?;
        Ok(FrozenLearnResult {
            character_id: CharacterId::from_uuid(entity.id.as_uuid()),
            skill,
            sp_spent: spent,
            sp_remaining: p.class_state.sp,
            tick,
            hp: c.hp,
            max_hp: c.sheet.max_hp(),
            mp: c.mp,
            max_mp: c.sheet.max_mp(),
            cp: p.class_state.cp,
            max_cp: p.max_cp,
            effective_stats: LearnedEffectiveStats {
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
            incarnation: c.incarnation,
        })
    }

    pub(super) fn freeze_learning_v5(
        &self,
        tick: Tick,
        events: &mut [ZoneEvent],
    ) -> Result<(), TickError> {
        for event in events {
            if let ZoneEvent::SkillLearned { entity, result, .. } = event {
                **result = self.learn_result_v5(
                    tick,
                    self.entities.get(entity).ok_or_else(missing)?,
                    self.sidecar(*entity)?,
                    result.skill,
                    result.sp_spent,
                )?;
            }
        }
        Ok(())
    }
}
