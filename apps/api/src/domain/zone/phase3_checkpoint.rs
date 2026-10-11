//! Typed paused persistence and pure admission staging. No external reads, RNG or hooks.
//! These DTOs do not duplicate `ClassState`'s SP, CP or learned-skill ledger.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::effects::{
    EffectCapacities, EffectError, EffectId, EffectList, RetainedEffect, RetainedModifierSource,
};
use super::skill::{SkillId, SkillRef};
use super::{Phase3Rules, Tick, Q};
use crate::domain::class::RuntimeSkillStatus;
use crate::domain::subclass::LearnedSkill;

/// Restoration credit only, never fractional damage. Each component is in [0,Q).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ResourceFractions {
    pub hp: i64,
    pub mp: i64,
    pub cp: i64,
}
impl ResourceFractions {
    /// Validate before combining these remainders with whole resource values.
    pub fn validate(self) -> Result<(), EffectError> {
        if [self.hp, self.mp, self.cp]
            .into_iter()
            .all(|value| (0..Q).contains(&value))
        {
            Ok(())
        } else {
            Err(EffectError::Invalid("resource fraction outside Q"))
        }
    }
}

/// Actual XP loss from the most recent death, retained across a real disconnect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct DeathRecord {
    pub incarnation: u32,
    pub pre_death_xp: u64,
    pub recoverable_death_xp_loss: u64,
}

/// Versioned persisted subset. Empty is an explicit new Phase 3 character, never a legacy
/// default. Receipts live in private checkpoint facts, not in this simulation payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase3Checkpoint {
    pub timer_policy_version: u32,
    pub karma: u32,
    pub pk_count: u32,
    pub pvp_count: u32,
    pub flag_remaining_ticks: u64,
    pub death_penalty_level: u8,
    pub last_death: Option<DeathRecord>,
    pub resource_fractions: ResourceFractions,
    pub reuse_remainders: BTreeMap<SkillId, u64>,
    pub effect_remainders: Vec<RetainedEffect>,
    pub modifier_install_order: Vec<RetainedModifierSource>,
}

/// Reconstructed modifier identity; sequence keys preserve ordering across source kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RestoredModifierSource {
    /// Exact learned passive rank.
    Passive(SkillRef),
    /// Freshly allocated zone effect identity.
    Effect(EffectId),
    /// Skill5076 at the checkpoint's scalar rank.
    DeathPenalty,
}

/// Pure staged admission output. Caller derives effective stats/capacity, clamps resources,
/// and installs all state and these counters atomically. No state is mutated on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct RestoredPhase3Timers {
    pub flag_until: Option<Tick>,
    pub reuse: BTreeMap<SkillId, Tick>,
    pub effects: EffectList,
    pub modifier_install_order: BTreeMap<u64, RestoredModifierSource>,
    pub next_effect_id: u64,
    pub next_install_sequence: u64,
}

impl Phase3Checkpoint {
    /// Explicit empty V5 payload. Learned passives, if present, must be installed by the
    /// caller in SkillId/rank order before validating or persisting this new character.
    #[allow(clippy::new_without_default)] // Versioned state requires explicit construction.
    pub fn new() -> Self {
        Self {
            timer_policy_version: 1,
            karma: 0,
            pk_count: 0,
            pvp_count: 0,
            flag_remaining_ticks: 0,
            death_penalty_level: 0,
            last_death: None,
            resource_fractions: ResourceFractions::default(),
            reuse_remainders: BTreeMap::new(),
            effect_remainders: Vec::new(),
            modifier_install_order: Vec::new(),
        }
    }

    /// Repository validation with immutable rules and the one canonical learned ledger.
    /// Class lineage/receipts use `ClassState::validate_for_phase3` separately. Actual buff
    /// capacity is checked again after effective-stat reconstruction; 24 is the hard cap.
    pub fn validate(
        &self,
        rules: &Phase3Rules,
        learned: &[LearnedSkill],
        alive: bool,
    ) -> Result<(), EffectError> {
        self.stage_restore(rules, learned, alive, 0, Tick(0), 1, 1)
            .map(|_| ())
    }

    /// Rebase paused deadlines to actual admission and allocate fresh IDs/sequences in
    /// persisted relative order. There is intentionally no `saved_at` or elapsed-time input.
    #[allow(clippy::too_many_arguments)] // All deterministic admission bindings are explicit.
    pub fn stage_restore(
        &self,
        rules: &Phase3Rules,
        learned: &[LearnedSkill],
        alive: bool,
        incarnation: u32,
        admission: Tick,
        first_effect_id: u64,
        first_install_sequence: u64,
    ) -> Result<RestoredPhase3Timers, EffectError> {
        self.validate_scalars(rules, alive)?;
        if first_effect_id == 0 || first_install_sequence == 0 {
            return Err(EffectError::Invalid("zero allocation counter"));
        }
        let registry = rules.skills();
        let passive_ranks: BTreeMap<_, _> = registry
            .skills()
            .values()
            .filter(|skill| {
                skill.runtime_status == RuntimeSkillStatus::Executable && skill.operate_type == "P"
            })
            .filter_map(|skill| {
                learned
                    .iter()
                    .find(|entry| entry.key == skill.key)
                    .map(|entry| (skill.id, entry.level))
            })
            .collect();
        let mut seen_keys = BTreeSet::new();
        for entry in learned {
            if entry.level == 0 || !seen_keys.insert(&entry.key) {
                return Err(EffectError::Invalid("invalid learned ledger"));
            }
        }
        let mut reuse = BTreeMap::new();
        for (id, remaining) in &self.reuse_remainders {
            let skill = registry
                .get(*id)
                .ok_or(EffectError::Invalid("unknown reuse skill"))?;
            if *remaining <= 50
                || skill.runtime_status != RuntimeSkillStatus::Executable
                || skill.operate_type == "P"
            {
                return Err(EffectError::Invalid("invalid retained reuse"));
            }
            reuse.insert(*id, deadline(admission, *remaining)?);
        }
        let mut next_effect_id = first_effect_id;
        let mut entries = BTreeMap::new();
        let mut effect_ids = Vec::new();
        for retained in &self.effect_remainders {
            if !alive && !retained.flags.stay_after_death {
                return Err(EffectError::Invalid("nonpersistent effect on dead admission"));
            }
            let id = EffectId(next_effect_id);
            next_effect_id = next_effect_id.checked_add(1).ok_or(EffectError::Overflow)?;
            let effect = retained.restore(id, incarnation, admission, registry)?;
            entries.insert(id, effect);
            effect_ids.push(id);
        }
        let effects = EffectList::from_entries(entries)?;
        effects.validate_capacity(EffectCapacities::for_enlarge(4))?;
        let mut modifier_install_order = BTreeMap::new();
        let mut seen_passives = BTreeSet::new();
        let mut seen_effects = BTreeSet::new();
        let mut death_penalty_seen = false;
        let mut next_install_sequence = first_install_sequence;
        for source in &self.modifier_install_order {
            let source = match source {
                RetainedModifierSource::Passive { skill } => {
                    if passive_ranks.get(&skill.id) != Some(&skill.rank)
                        || registry
                            .get(skill.id)
                            .is_none_or(|definition| skill.rank > definition.max_level)
                        || !seen_passives.insert(skill.id)
                    {
                        return Err(EffectError::Invalid("invalid passive installation source"));
                    }
                    RestoredModifierSource::Passive(*skill)
                },
                RetainedModifierSource::Effect { effect_ordinal } => {
                    let id = usize::try_from(*effect_ordinal)
                        .ok()
                        .and_then(|index| effect_ids.get(index))
                        .ok_or(EffectError::Invalid(
                            "unknown retained effect installation source",
                        ))?;
                    if !seen_effects.insert(*id) {
                        return Err(EffectError::Invalid("duplicate effect installation source"));
                    }
                    RestoredModifierSource::Effect(*id)
                },
                RetainedModifierSource::DeathPenalty => {
                    if self.death_penalty_level == 0 || death_penalty_seen {
                        return Err(EffectError::Invalid(
                            "invalid death penalty installation source",
                        ));
                    }
                    death_penalty_seen = true;
                    RestoredModifierSource::DeathPenalty
                },
            };
            modifier_install_order.insert(next_install_sequence, source);
            next_install_sequence = next_install_sequence
                .checked_add(1)
                .ok_or(EffectError::Overflow)?;
        }
        if seen_passives.len() != passive_ranks.len()
            || seen_effects.len() != effect_ids.len()
            || death_penalty_seen != (self.death_penalty_level != 0)
        {
            return Err(EffectError::Invalid("missing modifier installation source"));
        }
        Ok(RestoredPhase3Timers {
            flag_until: (self.flag_remaining_ticks != 0)
                .then(|| deadline(admission, self.flag_remaining_ticks))
                .transpose()?,
            reuse,
            effects,
            modifier_install_order,
            next_effect_id,
            next_install_sequence,
        })
    }

    fn validate_scalars(&self, rules: &Phase3Rules, alive: bool) -> Result<(), EffectError> {
        self.resource_fractions.validate()?;
        if self.timer_policy_version != rules.timer_policy_version()
            || self.death_penalty_level > 5
            || self.last_death.as_ref().is_some_and(|record| {
                record.incarnation == 0 || record.recoverable_death_xp_loss > record.pre_death_xp
            })
            || (!alive && self.resource_fractions != ResourceFractions::default())
        {
            return Err(EffectError::Invalid("invalid Phase3 checkpoint scalar"));
        }
        Ok(())
    }
}

fn deadline(admission: Tick, remaining: u64) -> Result<Tick, EffectError> {
    admission
        .0
        .checked_add(remaining)
        .map(Tick)
        .ok_or(EffectError::Overflow)
}

/// Owner-only effective sheet frozen into a successful paid-learning receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct LearnedEffectiveStats {
    pub p_atk_q: i64,
    pub p_def_q: i64,
    pub m_atk_q: i64,
    pub m_def_q: i64,
    pub attack_speed: u32,
    pub cast_speed: u32,
    pub speed_q: i64,
    pub accuracy_q: i64,
    pub evasion_q: i64,
    pub crit_permille: u32,
}

/// Exact success fact, carried by private checkpoint work rather than the state digest.
/// Its shape matches C's application DTO; C may re-export this single domain definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct FrozenLearnResult {
    pub character_id: crate::domain::CharacterId,
    pub skill: SkillRef,
    pub sp_spent: u64,
    pub sp_remaining: u64,
    pub tick: Tick,
    pub hp: u32,
    pub max_hp: u32,
    pub mp: u32,
    pub max_mp: u32,
    pub cp: u32,
    pub max_cp: u32,
    pub effective_stats: LearnedEffectiveStats,
    pub incarnation: u32,
}

impl super::Phase3Snapshot {
    /// Freeze paused persistence after this tick's callbacks. Snapshot timers themselves
    /// remain exact: only this repository projection applies the >50/>100 thresholds.
    /// Casts/offers/regen deadlines are deliberately not retained across real disconnects.
    pub fn checkpoint(
        &self,
        entity: super::EntityId,
        tick: Tick,
    ) -> Result<Phase3Checkpoint, EffectError> {
        let side = self
            .entities
            .get(&entity)
            .ok_or(EffectError::Invalid("unknown player"))?;
        let player = side
            .player
            .as_ref()
            .ok_or(EffectError::Invalid("NPC checkpoint"))?;
        let mut effect_remainders = Vec::new();
        let mut ordinals = BTreeMap::new();
        for effect in side.effects.entries().values() {
            effect.validate_source(self.rules.skills())?;
            if let Some(retained) = effect.retain(tick)? {
                let ordinal =
                    u32::try_from(effect_remainders.len()).map_err(|_| EffectError::Overflow)?;
                ordinals.insert(effect.id, ordinal);
                effect_remainders.push(retained);
            }
        }
        let mut modifier_install_order = Vec::new();
        for source in side.modifier_install_order.values() {
            let source = match source {
                RestoredModifierSource::Passive(skill) => {
                    RetainedModifierSource::Passive { skill: *skill }
                },
                RestoredModifierSource::DeathPenalty => RetainedModifierSource::DeathPenalty,
                RestoredModifierSource::Effect(id) => {
                    if !side.effects.entries().contains_key(id) {
                        return Err(EffectError::Invalid("unknown checkpoint modifier source"));
                    }
                    let Some(ordinal) = ordinals.get(id) else {
                        continue;
                    };
                    RetainedModifierSource::Effect {
                        effect_ordinal: *ordinal,
                    }
                },
            };
            modifier_install_order.push(source);
        }
        side.fractions.validate()?;
        Ok(Phase3Checkpoint {
            timer_policy_version: self.rules.timer_policy_version(),
            karma: player.pvp.karma,
            pk_count: player.pvp.pk_count,
            pvp_count: player.pvp.pvp_count,
            flag_remaining_ticks: player
                .pvp
                .flag_until
                .map_or(0, |end| end.0.saturating_sub(tick.0)),
            death_penalty_level: player.death_penalty_level,
            last_death: player.last_death.clone(),
            resource_fractions: side.fractions,
            reuse_remainders: side
                .reuse
                .iter()
                .filter_map(|(id, end)| {
                    end.0
                        .checked_sub(tick.0)
                        .filter(|left| *left > 50)
                        .map(|left| (*id, left))
                })
                .collect(),
            effect_remainders,
            modifier_install_order,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::domain::zone::effects::{
        ActiveEffect, ActiveHandler, EffectFlags, EffectPool, EffectSource, EffectSourceKind,
    };

    fn rules() -> Phase3Rules {
        crate::infrastructure::skill_data::load_phase3_rules(
            &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
            &[],
        )
        .unwrap()
    }

    fn retained(rules: &Phase3Rules) -> RetainedEffect {
        let skill = rules.skills().get(SkillId(110)).unwrap();
        let abnormal = skill.abnormal.as_ref().unwrap();
        let flag =
            |value: &super::super::skill::RankValue<bool>| *value.at(1, skill.max_level).unwrap();
        let effect = ActiveEffect {
            id: EffectId(10),
            skill: SkillRef {
                id: skill.id,
                rank: 1,
            },
            source: EffectSource {
                binding: None,
                kind: EffectSourceKind::System,
            },
            target_incarnation: 8,
            applied_at: Tick(0),
            expires_at: Some(Tick(101)),
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
                .filter(|(_, h)| super::super::effects::continuous_handler(&h.name))
                .map(|(i, _)| ActiveHandler {
                    xml_ordinal: u32::try_from(i).unwrap(),
                    periodic: None,
                })
                .collect(),
        };
        effect.validate_source(rules.skills()).unwrap();
        effect.retain(Tick(0)).unwrap().unwrap()
    }

    #[test]
    fn paused_restore_has_strict_thresholds_and_checks_allocation_and_deadlines() {
        let rules = rules();
        let mut checkpoint = Phase3Checkpoint::new();
        checkpoint.reuse_remainders.insert(SkillId(3), 51);
        checkpoint.flag_remaining_ticks = 1;
        checkpoint.effect_remainders.push(retained(&rules));
        checkpoint
            .modifier_install_order
            .push(RetainedModifierSource::Effect { effect_ordinal: 0 });
        let bytes = serde_json::to_vec(&checkpoint).unwrap();
        let restored = checkpoint
            .stage_restore(&rules, &[], true, 12, Tick(1000), 42, 70)
            .unwrap();
        assert_eq!(restored.flag_until, Some(Tick(1001)));
        assert_eq!(restored.reuse[&SkillId(3)], Tick(1051));
        assert_eq!(restored.effects.entries()[&EffectId(42)].expires_at, Some(Tick(1101)));
        assert_eq!(restored.effects.entries()[&EffectId(42)].target_incarnation, 12);
        assert_eq!(
            restored.modifier_install_order[&70],
            RestoredModifierSource::Effect(EffectId(42))
        );
        assert_eq!((restored.next_effect_id, restored.next_install_sequence), (43, 71));
        for (tick, effect_id, sequence) in [
            (u64::MAX, 1, 1),
            (0, u64::MAX, 1),
            (0, 1, u64::MAX),
            (0, 0, 1),
        ] {
            assert!(checkpoint
                .stage_restore(&rules, &[], true, 12, Tick(tick), effect_id, sequence)
                .is_err());
            assert_eq!(serde_json::to_vec(&checkpoint).unwrap(), bytes);
        }
        assert!(checkpoint.validate(&rules, &[], false).is_err());
        checkpoint.reuse_remainders.insert(SkillId(3), 50);
        assert!(checkpoint.validate(&rules, &[], true).is_err());
        checkpoint.reuse_remainders.insert(SkillId(3), 51);
        checkpoint.effect_remainders[0].remaining_ticks = 100;
        assert!(checkpoint.validate(&rules, &[], true).is_err());
    }

    #[test]
    fn shared_install_order_requires_exactly_one_of_every_retained_source() {
        let rules = rules();
        let passive = rules
            .skills()
            .skills()
            .values()
            .find(|skill| {
                skill.operate_type == "P" && skill.runtime_status == RuntimeSkillStatus::Executable
            })
            .unwrap();
        let learned = vec![LearnedSkill {
            key: passive.key.clone(),
            level: 1,
        }];
        let mut checkpoint = Phase3Checkpoint::new();
        checkpoint.death_penalty_level = 2;
        checkpoint.effect_remainders.push(retained(&rules));
        checkpoint.modifier_install_order = vec![
            RetainedModifierSource::Effect { effect_ordinal: 0 },
            RetainedModifierSource::Passive {
                skill: SkillRef {
                    id: passive.id,
                    rank: 1,
                },
            },
            RetainedModifierSource::DeathPenalty,
        ];
        checkpoint.validate(&rules, &learned, true).unwrap();
        let decoded: Phase3Checkpoint =
            serde_json::from_slice(&serde_json::to_vec(&checkpoint).unwrap()).unwrap();
        assert_eq!(decoded, checkpoint);
        for index in 0..3 {
            let mut missing = checkpoint.clone();
            missing.modifier_install_order.remove(index);
            assert!(missing.validate(&rules, &learned, true).is_err());
            let mut duplicate = checkpoint.clone();
            duplicate
                .modifier_install_order
                .push(checkpoint.modifier_install_order[index].clone());
            assert!(duplicate.validate(&rules, &learned, true).is_err());
        }
        checkpoint.modifier_install_order[0] = RetainedModifierSource::Effect { effect_ordinal: 1 };
        assert!(checkpoint.validate(&rules, &learned, true).is_err());
    }

    #[test]
    fn fractions_death_loss_and_policy_are_validated_without_defaulting_legacy_payloads() {
        let rules = rules();
        let empty = Phase3Checkpoint::new();
        empty.validate(&rules, &[], true).unwrap();
        for fractions in [
            ResourceFractions {
                hp: -1,
                ..ResourceFractions::default()
            },
            ResourceFractions {
                mp: Q,
                ..ResourceFractions::default()
            },
            ResourceFractions {
                cp: Q,
                ..ResourceFractions::default()
            },
        ] {
            let mut changed = empty.clone();
            changed.resource_fractions = fractions;
            assert!(changed.validate(&rules, &[], true).is_err());
        }
        let mut changed = empty.clone();
        changed.resource_fractions.hp = Q - 1;
        changed.validate(&rules, &[], true).unwrap();
        assert!(changed.validate(&rules, &[], false).is_err());
        changed = empty.clone();
        changed.timer_policy_version = 0;
        assert!(changed.validate(&rules, &[], true).is_err());
        changed = empty;
        changed.last_death = Some(DeathRecord {
            incarnation: 8,
            pre_death_xp: 1,
            recoverable_death_xp_loss: 2,
        });
        assert!(changed.validate(&rules, &[], false).is_err());
        changed.last_death = Some(DeathRecord {
            incarnation: 0,
            pre_death_xp: 1,
            recoverable_death_xp_loss: 1,
        });
        assert!(changed.validate(&rules, &[], false).is_err());
        assert!(serde_json::from_str::<Phase3Checkpoint>("{}").is_err());
    }
}
