//! Live continuous-container lifecycle. Source Skill.java:1005-1024 and
//! BuffInfo.java:260-278: success/duration precede instant handlers; installation follows.
use super::*;
use crate::domain::zone::effects::{
    self, ActiveEffect, ActiveHandler, ActivePeriodic, EffectAdmission, EffectCapacities,
    EffectFlags, EffectId, EffectPool, EffectRemoveReason, EffectSource, PulseSchedule,
};
use crate::domain::zone::skill::SkillRef;

impl ZoneState {
    pub(super) fn effect_applied_v5(
        &self,
        tick: Tick,
        id: EntityId,
        effect: &ActiveEffect,
        baseline: Option<Tick>,
    ) -> Result<ZoneEvent, TickError> {
        Ok(ZoneEvent::EffectApplied {
            tick,
            target: self.binding(id).ok_or_else(missing)?,
            source: effect.source.binding,
            effect_id: effect.id,
            skill: effect.skill,
            pool: effect.pool,
            applied_at: effect.applied_at,
            expires_at: effect.expires_at,
            abnormal_type: effect.abnormal_type.clone(),
            abnormal_level: effect.abnormal_level,
            status_flags: self.effective(id)?.flags,
            baseline,
        })
    }
    pub(super) fn remove_effect_v5(
        &mut self,
        tick: Tick,
        id: EntityId,
        effect: EffectId,
        reason: EffectRemoveReason,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        if self.sidecar_mut(id)?.effects.remove(effect).is_none() {
            return Ok(());
        }
        self.removed_effect_v5(tick, id, effect, reason, events)?;
        self.rebuild_v5(id)
    }
    fn removed_effect_v5(
        &mut self,
        tick: Tick,
        id: EntityId,
        effect: EffectId,
        reason: EffectRemoveReason,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        self.sidecar_mut(id)?.modifier_install_order.retain(
            |_, source| !matches!(source, RestoredModifierSource::Effect(old) if *old == effect),
        );
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .effective
            .remove(&id);
        events.push(ZoneEvent::EffectRemoved {
            tick,
            target: self.binding(id).ok_or_else(missing)?,
            effect_id: effect,
            reason,
        });
        Ok(())
    }
    pub(super) fn expire_effects_v5(
        &mut self,
        tick: Tick,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        self.refresh_schedule_v5()?;
        let due = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .schedule
            .effect_expiries(tick);
        for work in due {
            self.remove_effect_v5(
                tick,
                work.target,
                work.effect,
                EffectRemoveReason::Expired,
                events,
            )?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Source lifecycle ordering remains explicit.
    pub(super) fn install_effect_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        duration: u32,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        let abnormal = skill.abnormal.as_ref().ok_or_else(missing)?;
        let index = usize::try_from(rank.checked_sub(1).ok_or_else(missing)?).map_err(invariant)?;
        let duration = u64::from(duration);
        let phase = self.phase3.as_ref().ok_or_else(missing)?;
        let id = EffectId(phase.canonical.next_effect_id);
        let sequence = phase.canonical.next_install_sequence;
        let flag = |value: &crate::domain::zone::skill::RankValue<bool>| {
            value.at(rank, skill.max_level).copied().ok_or_else(missing)
        };
        let handlers = skill
            .effects
            .iter()
            .enumerate()
            .filter(|(_, h)| effects::continuous_handler(&h.name))
            .map(|(ordinal, handler)| {
                let periodic = effects::compile_periodic(handler, rank, skill.max_level)
                    .map_err(invariant)?
                    .map(|(resource, power, interval)| {
                        Ok(ActivePeriodic {
                            resource,
                            power,
                            schedule: PulseSchedule::new(tick, interval).map_err(invariant)?,
                        })
                    })
                    .transpose()?;
                Ok(ActiveHandler {
                    xml_ordinal: u32::try_from(ordinal).map_err(invariant)?,
                    periodic,
                })
            })
            .collect::<Result<Vec<_>, TickError>>()?;
        let effect = ActiveEffect {
            id,
            skill: SkillRef { id: skill.id, rank },
            source: EffectSource {
                binding: Some(caster),
                kind: EffectSourceKind::Player,
            },
            target_incarnation: target.incarnation,
            applied_at: tick,
            expires_at: Some(Tick(
                tick.0
                    .checked_add(duration.checked_mul(10).ok_or_else(missing)?)
                    .ok_or_else(missing)?,
            )),
            pool: if abnormal.is_debuff {
                EffectPool::Debuff
            } else if skill.is_magic == 3 {
                EffectPool::SongDance
            } else {
                EffectPool::Buff
            },
            abnormal_type: abnormal.r#type.clone(),
            abnormal_level: *abnormal.level.get(index).ok_or_else(missing)?,
            flags: EffectFlags {
                stay_after_death: flag(&skill.stay_after_death)?,
                stay_on_subclass_change: flag(&skill.stay_on_subclass_change)?,
                removed_on_damage: flag(&skill.removed_on_damage)?,
                removed_on_action_except_move: flag(&skill.removed_on_action_except_move)?,
                irreplaceable: flag(&skill.irreplaceable_buff)?,
            },
            handlers,
        };
        effect
            .validate_source(phase.canonical.rules.skills())
            .map_err(invariant)?;
        let admitted = self
            .sidecar_mut(target.entity)?
            .effects
            .admit(effect.clone(), EffectCapacities::for_enlarge(0))
            .map_err(invariant)?;
        let mut applied_effect_ids = Vec::new();
        if let EffectAdmission::Applied { removed } = admitted {
            for old in removed {
                let reason = if old.abnormal_type == effect.abnormal_type
                    && (effect.abnormal_type != "NONE" || old.skill.id == effect.skill.id)
                {
                    EffectRemoveReason::Replaced
                } else {
                    EffectRemoveReason::Overflow
                };
                self.removed_effect_v5(tick, target.entity, old.id, reason, events)?;
            }
            let before = self.effective(target.entity)?;
            self.sidecar_mut(target.entity)?
                .modifier_install_order
                .insert(sequence, RestoredModifierSource::Effect(id));
            let phase = &mut self.phase3.as_mut().ok_or_else(missing)?.canonical;
            phase.next_effect_id = id.0.checked_add(1).ok_or_else(missing)?;
            phase.next_install_sequence = sequence.checked_add(1).ok_or_else(missing)?;
            self.rebuild_v5(target.entity)?;
            for handler in &skill.effects {
                let parameter = |key: &str| {
                    handler
                        .params
                        .get(key)
                        .and_then(|v| v.at(rank, skill.max_level))
                        .map(String::as_str)
                };
                if matches!(handler.name.as_str(), "MaxHp" | "MaxMp")
                    && parameter("heal") == Some("true")
                {
                    let hp = handler.name == "MaxHp";
                    let power = ExactRatio::parse_literal(parameter("power").unwrap_or("0"))
                        .map_err(invariant)?;
                    let old_max = if hp {
                        before.physical.max_hp()
                    } else {
                        before.physical.max_mp()
                    };
                    let amount = if parameter("type") == Some("PER") {
                        power
                            .checked_mul(ExactRatio::whole(i64::from(old_max)))
                            .and_then(|v| v.checked_div(ExactRatio::whole(100)))
                            .map_err(invariant)?
                    } else {
                        power
                    };
                    let c = self
                        .entities
                        .get_mut(&target.entity)
                        .and_then(|e| e.combat.as_mut())
                        .ok_or_else(missing)?;
                    let current = if hp { c.hp } else { c.mp };
                    let maximum = if hp {
                        c.sheet.max_hp()
                    } else {
                        c.sheet.max_mp()
                    };
                    let fraction = if hp {
                        self.sidecar(target.entity)?.fractions.hp
                    } else {
                        self.sidecar(target.entity)?.fractions.mp
                    };
                    let restored = skill_math::restore_resource(
                        current,
                        maximum,
                        u32::try_from(fraction).map_err(invariant)?,
                        amount.to_q().map_err(invariant)?,
                    )
                    .map_err(invariant)?;
                    if self.evidence.is_some() {
                        self.evidence_fact("resource_hook", target.entity, serde_json::json!({
                            "skill_id": skill.id, "rank": rank, "handler": handler.name,
                            "resource": if hp { "hp" } else { "mp" }, "before": current,
                            "fraction_before": fraction, "amount": amount, "after": restored.current,
                            "whole_credit": restored.granted, "fraction_after": restored.remainder_q
                        }));
                    }
                    let c = self
                        .entities
                        .get_mut(&target.entity)
                        .and_then(|e| e.combat.as_mut())
                        .ok_or_else(missing)?;
                    if hp {
                        c.hp = restored.current;
                        self.sidecar_mut(target.entity)?.fractions.hp =
                            i64::from(restored.remainder_q);
                    } else {
                        c.mp = restored.current;
                        self.sidecar_mut(target.entity)?.fractions.mp =
                            i64::from(restored.remainder_q);
                    }
                }
            }
            self.status_barrier_v5(tick, target.entity, events)?;
            events.push(self.effect_applied_v5(tick, target.entity, &effect, None)?);
            applied_effect_ids.push(id);
        }
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Effect,
            outcome: SkillOutcome::Hit,
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
            applied_effect_ids,
        })
    }
}

impl ZoneState {
    pub(super) fn base_property_v5(
        &self,
        id: EntityId,
        name: &str,
    ) -> Result<(u32, ExactRatio), TickError> {
        use crate::domain::zone::StatKind;
        let kind = match name {
            "STR" => StatKind::Str,
            "DEX" => StatKind::Dex,
            "CON" => StatKind::Con,
            "INT" => StatKind::Int,
            "WIT" => StatKind::Wit,
            "MEN" => StatKind::Men,
            "NONE" => return Ok((0, ExactRatio::whole(1))),
            _ => return Err(missing()),
        };
        let c = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
        let rules = &self.phase3.as_ref().ok_or_else(missing)?.canonical.rules;
        let base = match &c.role {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => rules
                .profiles()
                .classes
                .get(&p.class_state.current_class_id)
                .ok_or_else(missing)?
                .base_stats
                .values(),
            CombatRole::Npc { template, .. } => rules
                .profiles()
                .npcs
                .get(template)
                .ok_or_else(missing)?
                .template
                .base_stats
                .values(),
            CombatRole::Player {
                progression: None, ..
            } => return Err(missing()),
        };
        let value = kind.of(&base);
        Ok((
            value,
            q(self
                .rules
                .as_ref()
                .ok_or_else(missing)?
                .bonus()
                .bonus(kind, value)
                .map_err(invariant)?
                .raw())
            .map_err(invariant)?,
        ))
    }
    /// Formulas.java:1850-1877 and TraitType.java:25-80; weapon traits are not resistance traits.
    pub(super) fn trait_modifier_v5(
        a: &EffectiveStats,
        b: &EffectiveStats,
        name: &str,
    ) -> Result<ExactRatio, TickError> {
        let target = b.traits.get(name).cloned().unwrap_or_default();
        if name == "NONE" {
            return Ok(ExactRatio::whole(1));
        }
        if target.immunity_providers != 0 {
            return Ok(ExactRatio::whole(0));
        }
        let attacker = a.traits.get(name).cloned().unwrap_or_default();
        let weakness = matches!(
            name,
            "BUG_WEAKNESS"
                | "ANIMAL_WEAKNESS"
                | "PLANT_WEAKNESS"
                | "BEAST_WEAKNESS"
                | "DRAGON_WEAKNESS"
                | "GIANT_WEAKNESS"
                | "CONSTRUCT_WEAKNESS"
                | "VALAKAS"
                | "ANESTHESIA"
                | "DEMONIC_WEAKNESS"
                | "DIVINE_WEAKNESS"
                | "ELEMENTAL_WEAKNESS"
                | "FAIRY_WEAKNESS"
                | "HUMAN_WEAKNESS"
                | "HUMANOID_WEAKNESS"
                | "UNDEAD_WEAKNESS"
        );
        if weakness && (attacker.attack_providers == 0 || target.defence_providers == 0)
            || matches!(
                name,
                "SWORD"
                    | "BLUNT"
                    | "DAGGER"
                    | "POLE"
                    | "FIST"
                    | "BOW"
                    | "ETC"
                    | "DUAL"
                    | "DUALFIST"
                    | "RAPIER"
                    | "CROSSBOW"
                    | "ANCIENTSWORD"
                    | "DUALDAGGER"
                    | "UNK_8"
            )
        {
            return Ok(ExactRatio::whole(1));
        }
        attacker
            .attack
            .checked_sub(target.defence)
            .and_then(|v| v.checked_add(ExactRatio::whole(1)))
            .and_then(|v| v.clamp(ExactRatio::new(1, 20)?, ExactRatio::whole(2)))
            .map_err(invariant)
    }
    /// Preserve caller RNG order even on a resisted container: reflection, success, duration.
    /// Duration has no mastery draw with the admitted Tier B capabilities.
    pub(super) fn continuous_terms_v5(
        &mut self,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
    ) -> Result<Option<(bool, u32)>, TickError> {
        let Some(abnormal) = &skill.abnormal else {
            return Ok(None);
        };
        let mark = self.evidence_mark();
        let mut chance_evidence = None;
        let a = self.effective(caster.entity)?;
        let b = self.effective(target.entity)?;
        let activate_rate = *skill
            .activate_rate
            .at(rank, skill.max_level)
            .ok_or_else(missing)?;
        if abnormal.is_debuff && activate_rate != -1 {
            self.evidence_roll("EffectReflection", Some(target), 100);
        }
        let trait_modifier = Self::trait_modifier_v5(&a, &b, &skill.r#trait)?;
        let (target_base_stat, stat_bonus) =
            self.base_property_v5(target.entity, &skill.basic_property)?;
        let index = usize::try_from(rank.checked_sub(1).ok_or_else(missing)?).map_err(invariant)?;
        let magic_level =
            i32::try_from(*skill.magic_level.get(index).ok_or_else(missing)?).map_err(invariant)?;
        let success = if activate_rate == -1 || skill.basic_property == "NONE" {
            true
        } else {
            let inputs = skill_math::EffectSuccessInputs {
                magic_level,
                target_level: b.physical.level(),
                activate_rate: i32::try_from(activate_rate).map_err(invariant)?,
                level_bonus_rate: i32::try_from(skill.level_bonus_rate).map_err(invariant)?,
                target_base_stat,
                basic_property_none: false,
                is_magic: skill.is_magic == 1,
                magic_attack: q(a.m_atk_q).map_err(invariant)?,
                magic_defence: q(b.m_def_q).map_err(invariant)?,
                trait_modifier,
                element_modifier: ExactRatio::whole(1),
                // Formulas.java:1195 starts the vulnerability Func fold at1.
                buff_debuff_modifier: ExactRatio::whole(1)
                    .checked_add(
                        q(b.calculator
                            .fold(
                                if abnormal.is_debuff {
                                    "debuffVuln"
                                } else {
                                    "buffVuln"
                                },
                                Q,
                                FunctionConditions {
                                    weapon_kind: self.item_weapon_kind(
                                        target.entity,
                                        if self.sidecar(target.entity)?.player.is_some() {
                                            "SWORD"
                                        } else {
                                            "NONE"
                                        },
                                    )?,
                                    can_resurrect: true,
                                },
                            )
                            .map_err(invariant)?)
                        .map_err(invariant)?
                        .checked_div(ExactRatio::whole(100))
                        .map_err(invariant)?,
                    )
                    .map_err(invariant)?,
                min_chance: skill.min_chance,
                max_chance: skill.max_chance,
            };
            let rate = skill_math::effect_success_rate(inputs).map_err(invariant)?;
            if self.evidence.is_some() {
                chance_evidence = Some(serde_json::json!({"inputs": inputs, "chance": rate}));
            }
            rate.checked_cmp(ExactRatio::whole(i64::from(self.evidence_roll(
                "EffectSuccess",
                Some(target),
                100,
            ))))
            .map_err(invariant)?
                == std::cmp::Ordering::Greater
        };
        let duration = skill_math::effect_duration_seconds(
            u32::try_from(*abnormal.time_s.get(index).ok_or_else(missing)?).map_err(invariant)?,
            abnormal.is_debuff,
            skill_math::duration_level_mod(
                magic_level,
                a.physical.level(),
                b.physical.level(),
                i32::try_from(skill.level_bonus_rate).map_err(invariant)?,
            )
            .map_err(invariant)?,
            stat_bonus,
            trait_modifier,
            ExactRatio::whole(1),
        )
        .map_err(invariant)?;
        if self.evidence.is_some() {
            self.evidence_fact("effect_formula", caster.entity, serde_json::json!({
                "skill_id": skill.id, "rank": rank, "target": target,
                "chance_evaluation": chance_evidence, "landed": success,
                "duration_seconds": duration, "duration_inputs": {
                    "base_seconds": abnormal.time_s.get(index), "is_debuff": abnormal.is_debuff,
                    "magic_level": magic_level, "caster_level": a.physical.level(),
                    "target_level": b.physical.level(), "level_bonus_rate": skill.level_bonus_rate,
                    "stat_bonus": stat_bonus, "trait_modifier": trait_modifier,
                    "element_modifier": "1/1" },
                "rng_draws": self.evidence_draws(mark)
            }));
        }
        Ok(Some((success, duration)))
    }
    pub(super) fn status_barrier_v5(
        &mut self,
        tick: Tick,
        id: EntityId,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        use crate::domain::zone::effects::StatusFlags as F;
        let flags = self.effective(id)?.flags;
        let hard = flags.intersects(F::STUN.union(F::SLEEP).union(F::PARALYZE));
        if hard {
            self.cancel_v5(tick, id, None, CastCancelReason::Incapacitated, events);
        }
        if hard || flags.intersects(F::ROOT) {
            let e = self.entities.get_mut(&id).ok_or_else(missing)?;
            if e.dest.take().is_some() {
                events.push(ZoneEvent::EntityMove {
                    tick,
                    entity: id,
                    pos: e.pos,
                    dest: None,
                    speed: e.speed,
                });
            }
            let c = e.combat.as_mut().ok_or_else(missing)?;
            c.chasing = false;
            if hard {
                if let Some(swing) = c.swing.take() {
                    events.push(ZoneEvent::AttackCancelled {
                        tick,
                        attacker: id,
                        target: swing.target,
                        reason: SwingCancel::Stopped,
                    });
                }
            }
        }
        Ok(())
    }
}

impl ZoneState {
    /// F6: exact ideal-ms phase advances on suppressed work too; F5: HP-1 before CP.
    #[allow(clippy::too_many_lines)] // Due-key validation and resource branches form one ordered transaction.
    pub(super) fn pulse_effects_v5(
        &mut self,
        tick: Tick,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        use effects::PeriodicResource;
        self.refresh_schedule_v5()?;
        let due = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .schedule
            .effect_pulses(tick);
        for work in due {
            let Some(effect) = self
                .sidecar(work.target)?
                .effects
                .entries()
                .get(&work.effect)
                .cloned()
            else {
                continue;
            };
            let Some(periodic) = effect
                .handlers
                .iter()
                .find(|h| h.xml_ordinal == work.handler_ordinal)
                .and_then(|h| h.periodic.as_ref())
            else {
                continue;
            };
            self.sidecar_mut(work.target)?
                .effects
                .advance_pulse(work.effect, work.handler_ordinal)
                .map_err(invariant)?;
            let target = self.binding(work.target).ok_or_else(missing)?;
            let c = self
                .entities
                .get(&work.target)
                .and_then(|e| e.combat.as_ref())
                .ok_or_else(missing)?;
            if c.hp == 0 || target.incarnation != effect.target_incarnation {
                continue;
            }
            let fractions = self.sidecar(work.target)?.fractions;
            let (current, fraction) = match periodic.resource {
                PeriodicResource::Hp => (c.hp, fractions.hp),
                PeriodicResource::Mp => (c.mp, fractions.mp),
            };
            let current_q = i64::from(current)
                .checked_mul(Q)
                .and_then(|v| v.checked_add(fraction))
                .ok_or_else(missing)?;
            let amount = periodic.power.amount(current_q).map_err(invariant)?;
            let negative = amount.parts().0 < 0;
            if negative {
                let peace = self
                    .phase3
                    .as_ref()
                    .ok_or_else(missing)?
                    .canonical
                    .rules
                    .geometry()
                    .peace_square(self.bounds)
                    .map_err(invariant)?;
                if peace.contains(self.entities.get(&work.target).ok_or_else(missing)?.pos)
                    || effect
                        .source
                        .binding
                        .filter(|source| self.binding(source.entity) == Some(*source))
                        .and_then(|source| self.entities.get(&source.entity))
                        .is_some_and(|e| peace.contains(e.pos))
                {
                    continue;
                }
            }
            if negative {
                if let Some(source) = effect.source.binding {
                    if self.hostility_v5(tick, source, target).is_ok() {
                        self.note_hostility_v5(tick, source, target)?;
                    }
                }
            }
            let stats = self.effective(work.target)?;
            match periodic.resource {
                PeriodicResource::Hp if negative => {
                    let rolled = skill_math::nonlethal_periodic_damage(
                        ExactRatio::whole(0)
                            .checked_sub(amount)
                            .map_err(invariant)?,
                        current,
                    )
                    .map_err(invariant)?;
                    // The source life/kind is frozen in the effect. A replacement entity
                    // with the same ID cannot take attribution or lend its peace position.
                    let request = DamageRequest {
                        source: effect.source,
                        target,
                        rolled_damage: rolled,
                        cause: DamageCause::Periodic,
                    };
                    if self.damage_classification_v5(tick, request).is_ok() {
                        self.apply_damage_v5(tick, request, events)?;
                    }
                },
                PeriodicResource::Mp if negative => {
                    let drain = u32::try_from(
                        ExactRatio::whole(0)
                            .checked_sub(amount)
                            .and_then(ExactRatio::trunc)
                            .map_err(invariant)?,
                    )
                    .map_err(invariant)?;
                    let c = self
                        .entities
                        .get_mut(&work.target)
                        .and_then(|e| e.combat.as_mut())
                        .ok_or_else(missing)?;
                    c.mp = c.mp.saturating_sub(drain);
                },
                resource @ (PeriodicResource::Hp | PeriodicResource::Mp) => {
                    let maximum = match resource {
                        PeriodicResource::Hp => stats.recoverable_hp,
                        PeriodicResource::Mp => stats.recoverable_mp,
                    };
                    let restored = skill_math::restore_resource(
                        current,
                        maximum,
                        u32::try_from(fraction).map_err(invariant)?,
                        amount.to_q().map_err(invariant)?,
                    )
                    .map_err(invariant)?;
                    let c = self
                        .entities
                        .get_mut(&work.target)
                        .and_then(|e| e.combat.as_mut())
                        .ok_or_else(missing)?;
                    match resource {
                        PeriodicResource::Hp => {
                            c.hp = restored.current;
                            if restored.granted > 0 {
                                self.phase3
                                    .as_mut()
                                    .ok_or_else(missing)?
                                    .vitals_dirty
                                    .insert(work.target);
                            }
                            self.sidecar_mut(work.target)?.fractions.hp =
                                i64::from(restored.remainder_q);
                        },
                        PeriodicResource::Mp => {
                            c.mp = restored.current;
                            self.sidecar_mut(work.target)?.fractions.mp =
                                i64::from(restored.remainder_q);
                        },
                    }
                },
            }
        }
        Ok(())
    }
}

impl ZoneState {
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Source lifecycle ordering remains explicit.
    pub(super) fn restore_handler_v5(
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
        let param = |name: &str, default: &str| -> String {
            handler
                .params
                .get(name)
                .and_then(|v| v.at(rank, skill.max_level))
                .map_or_else(|| default.to_owned(), Clone::clone)
        };
        let hp_handler = matches!(handler.name.as_str(), "Heal" | "HpPerMax");
        let cp_handler = handler.name == "Cp";
        let fractions = self.sidecar(target.entity)?.fractions;
        let c = self
            .entities
            .get(&target.entity)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
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
        let (current, fraction, maximum, recoverable) = if hp_handler {
            (c.hp, fractions.hp, b.physical.max_hp(), b.recoverable_hp)
        } else if cp_handler {
            (cp.unwrap_or(0), fractions.cp, b.max_cp, b.recoverable_cp)
        } else {
            (c.mp, fractions.mp, b.physical.max_mp(), b.recoverable_mp)
        };
        let mut outcome = SkillOutcome::Hit;
        let mut granted = 0;
        if c.hp != 0 && (!cp_handler || cp.is_some()) {
            let current_exact = ExactRatio::whole(i64::from(current))
                .checked_add(q(fraction).map_err(invariant)?)
                .map_err(invariant)?;
            let gap = ExactRatio::whole(i64::from(recoverable))
                .checked_sub(current_exact)
                .map_err(invariant)?;
            let amount = if handler.name == "Heal" {
                let power = ExactRatio::parse_literal(&param("power", "0")).map_err(invariant)?;
                let mut amount = skill_math::heal_base(
                    power,
                    q(a.m_atk_q).map_err(invariant)?,
                    2,
                    skill.is_magic == 2,
                )
                .map_err(invariant)?;
                // HEAL_EFFECT is absent from the executable Func manifest; coefficient stays1.
                if skill.is_magic == 1
                    && self.evidence_roll("HealCritical", Some(target), 1000) < a.magic_crit
                {
                    amount = amount
                        .checked_mul(ExactRatio::whole(3))
                        .map_err(invariant)?;
                    outcome = SkillOutcome::Crit;
                }
                amount
                    .clamp(
                        ExactRatio::whole(0),
                        gap.clamp(ExactRatio::whole(0), ExactRatio::whole(i64::from(maximum)))
                            .map_err(invariant)?,
                    )
                    .map_err(invariant)?
                    .floor_scaled(u32::try_from(Q).map_err(invariant)?)
                    .map_err(invariant)?
            } else if handler.name == "HpPerMax" {
                ExactRatio::whole(i64::from(maximum))
                    .checked_mul(
                        ExactRatio::parse_literal(&param("power", "0")).map_err(invariant)?,
                    )
                    .and_then(|v| v.checked_div(ExactRatio::whole(100)))
                    .and_then(|v| {
                        v.clamp(
                            ExactRatio::whole(0),
                            gap.clamp(ExactRatio::whole(0), ExactRatio::whole(i64::from(maximum)))?,
                        )
                    })
                    .and_then(ExactRatio::to_q)
                    .map_err(invariant)?
            } else {
                let input = ExactRatio::parse_literal(&param("amount", "0")).map_err(invariant)?;
                let mut amount = if param("mode", "DIFF") == "PER" {
                    (if input.parts().0 < 0 {
                        current_exact
                    } else {
                        ExactRatio::whole(i64::from(maximum))
                    })
                    .checked_mul(input)
                    .and_then(|v| v.checked_div(ExactRatio::whole(100)))
                    .map_err(invariant)?
                } else {
                    input
                };
                if (cp_handler && param("mode", "DIFF") == "DIFF" || input.parts().0 >= 0)
                    && amount.checked_cmp(gap).map_err(invariant)? == std::cmp::Ordering::Greater
                {
                    amount = gap;
                }
                amount.to_q().map_err(invariant)?
            };
            let (new, remainder) = if amount >= 0 {
                let restored = skill_math::restore_resource(
                    current,
                    maximum,
                    u32::try_from(fraction).map_err(invariant)?,
                    amount,
                )
                .map_err(invariant)?;
                granted = restored.granted;
                (restored.current, i64::from(restored.remainder_q))
            } else {
                let drain = u32::try_from(
                    amount
                        .checked_neg()
                        .and_then(|v| v.checked_div(Q))
                        .ok_or_else(missing)?,
                )
                .map_err(invariant)?;
                (current.saturating_sub(drain), fraction)
            };
            let c = self
                .entities
                .get_mut(&target.entity)
                .and_then(|e| e.combat.as_mut())
                .ok_or_else(missing)?;
            if hp_handler {
                c.hp = new;
                self.sidecar_mut(target.entity)?.fractions.hp = remainder;
            } else if cp_handler {
                if let CombatRole::Player {
                    progression: Some(p),
                    ..
                } = &mut c.role
                {
                    p.class_state.cp = new;
                }
                self.sidecar_mut(target.entity)?.fractions.cp = remainder;
            } else {
                c.mp = new;
                self.sidecar_mut(target.entity)?.fractions.mp = remainder;
            }
        }
        if hp_handler && granted > 0 {
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .vitals_dirty
                .insert(target.entity);
            self.heal_hate_v5(tick, caster, target, granted, events);
        }
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Heal,
            outcome,
            rolled: 0,
            cp_absorbed: 0,
            hp_removed: 0,
            heal: if hp_handler { granted } else { 0 },
            target_hp_after: self
                .entities
                .get(&target.entity)
                .and_then(|e| e.combat.as_ref())
                .ok_or_else(missing)?
                .hp,
            applied_effect_ids: Vec::new(),
        })
    }
    fn heal_hate_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        granted: u32,
        events: &mut Vec<ZoneEvent>,
    ) {
        let recipients: Vec<_> = self
            .entities
            .iter()
            .filter_map(|(id, e)| {
                let c = e.combat.as_ref()?;
                (e.kind == EntityKind::Npc
                    && c.hp > 0
                    && self
                        .hate
                        .get(id)
                        .is_some_and(|ledger| ledger.get(target.entity).is_some()))
                .then_some((*id, c.sheet.level()))
            })
            .collect();
        for (id, level) in recipients {
            self.add_hate_for(
                tick,
                id,
                caster.entity,
                super::super::super::HateEntry {
                    hate: u64::from(granted)
                        .saturating_mul(100)
                        .checked_div(u64::from(level).saturating_add(7))
                        .unwrap_or(0),
                    damage: 0,
                },
                "heal",
                events,
            );
        }
    }
}

impl ZoneState {
    /// DispelBySlot.java:75-96: named abnormal/rank removal has no chance draw.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispel_handler_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        let removals = if handler.name == "DispelByCategory" {
            self.category_dispel_v5(caster, target, skill, rank, handler)?
        } else {
            let text = handler
                .params
                .get("dispel")
                .and_then(|v| v.at(rank, skill.max_level))
                .ok_or_else(missing)?;
            let slots: BTreeMap<_, _> = text
                .split(';')
                .map(|entry| {
                    let (name, rank) = entry.split_once(',').unwrap_or((entry, "-1"));
                    Ok((name, rank.parse::<i64>().map_err(invariant)?))
                })
                .collect::<Result<_, TickError>>()?;
            let removals: Vec<_> = self
                .sidecar(target.entity)?
                .effects
                .entries()
                .values()
                .filter(|e| {
                    slots
                        .get(e.abnormal_type.as_str())
                        .is_some_and(|max| *max < 0 || i64::from(e.abnormal_level) <= *max)
                })
                .map(|e| e.id)
                .collect();
            removals
        };
        for id in removals {
            self.remove_effect_v5(tick, target.entity, id, EffectRemoveReason::Dispelled, events)?;
        }
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Effect,
            outcome: SkillOutcome::Hit,
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
        })
    }
    /// Formulas.java:1643-1712: snapshot candidates and rolls before removals/rebuilds.
    /// Buffs are concatenated before songs, then traversed in reverse insertion order.
    fn category_dispel_v5(
        &mut self,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
    ) -> Result<Vec<EffectId>, TickError> {
        let param = |name: &str, default: &str| -> String {
            handler
                .params
                .get(name)
                .and_then(|v| v.at(rank, skill.max_level))
                .map_or_else(|| default.to_owned(), Clone::clone)
        };
        let slot = param("slot", "BUFF");
        let maximum = param("max", "0").parse::<i32>().map_err(invariant)?;
        let rate = param("rate", "0").parse::<i32>().map_err(invariant)?;
        // DispelCategory.ALL is a source no-op (no matching switch arm).
        if slot == "ALL" {
            return Ok(Vec::new());
        }
        let buff = slot == "BUFF";
        let rate = if buff {
            let a = self.effective(caster.entity)?;
            let b = self.effective(target.entity)?;
            let target_conditions = FunctionConditions {
                weapon_kind: self.item_weapon_kind(target.entity, "SWORD")?,
                can_resurrect: true,
            };
            let caster_conditions = FunctionConditions {
                weapon_kind: self.item_weapon_kind(caster.entity, "SWORD")?,
                can_resurrect: true,
            };
            skill_math::cancel_base_rate(
                rate,
                b.calculator
                    .fold("cancelVuln", 0, target_conditions)
                    .map_err(invariant)?,
                a.calculator
                    .fold("cancelProf", 0, caster_conditions)
                    .map_err(invariant)?,
            )
            .map_err(invariant)?
        } else {
            rate
        };
        let level = *skill
            .magic_level
            .get(usize::try_from(rank.saturating_sub(1)).map_err(invariant)?)
            .ok_or_else(missing)?;
        let mut candidates: Vec<_> = self
            .sidecar(target.entity)?
            .effects
            .entries()
            .values()
            .filter(|e| {
                if buff {
                    matches!(e.pool, EffectPool::Buff | EffectPool::SongDance)
                } else {
                    e.pool == EffectPool::Debuff
                }
            })
            .cloned()
            .collect();
        candidates.sort_by_key(|e| (e.pool, e.applied_at, e.id));
        let mut removals = Vec::new();
        for effect in candidates.iter().rev() {
            let definition = self
                .phase3
                .as_ref()
                .ok_or_else(missing)?
                .canonical
                .rules
                .skills()
                .get(effect.skill.id)
                .ok_or_else(missing)?;
            let chance = if buff {
                if !definition.can_be_stolen(effect.skill.rank) {
                    continue;
                }
                let effect_level = *definition
                    .magic_level
                    .get(usize::try_from(effect.skill.rank.saturating_sub(1)).map_err(invariant)?)
                    .ok_or_else(missing)?;
                skill_math::cancel_buff_chance(
                    rate,
                    level,
                    effect_level,
                    skill.min_chance,
                    skill.max_chance,
                )
                .map_err(invariant)?
            } else {
                if effect.flags.irreplaceable
                    || !definition.abnormal.as_ref().is_some_and(|a| a.is_debuff)
                {
                    continue;
                }
                i64::from(rate)
            };
            let roll = i64::from(self.evidence_roll("Dispel", Some(target), 100));
            if (buff && roll < chance) || (!buff && roll <= chance) {
                removals.push(effect.id);
                // Source tests after insertion: even max=0 permits the first success.
                if i64::try_from(removals.len()).map_err(invariant)? >= i64::from(maximum) {
                    break;
                }
            }
        }
        Ok(removals)
    }
}
