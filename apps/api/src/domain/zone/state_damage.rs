//! Recipient-side V5 damage reduction, shared by auto attacks, skills and periodic HP.
//! Caller validation, magic pre-reduction attack-break/reflection and auto post-reduction
//! attack-break remain at their source callsites. This transaction has exactly the source
//! recipient status/stun -> rebuild -> CP -> HP order, with no invented interruption roll.
//! PcStatus.java:82-96,185-195; CharStatus.java:142-154; CharEffectList.java:847-875.

use super::effects::{ActiveEffect, EffectError, EffectSourceKind, StatusFlags};
use super::state_pvp::HostilityClassification;
use super::{Phase3Entity, Phase3Rules, ResourceFractions, RestoredModifierSource, StatError};

/// Calling path controls status removals and the nonlethal HP clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageCause {
    /// Ordinary hostile auto attack or direct skill, including zero-valued reduceHp calls.
    Direct,
    /// Non-toggle `TickHp`: cap at currentHP-1 BEFORE CP absorption, never kills.
    Periodic,
    /// Skill HP cost; cannot kill, bypasses CP and every damage/status/RNG hook.
    HpConsumption,
}

/// Authoritative whole resources after pre-call validation. CP exists only for players.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct DamageResources {
    pub hp: u32,
    pub mp: u32,
    pub cp: Option<u32>,
}

/// Effective maxima after removing damage-breakable/stun effects. The supplied stat
/// reconstruction closure must be cold: no onStart callbacks, heals, or RNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct DamageMaxima {
    pub hp: u32,
    pub mp: u32,
    pub cp: Option<u32>,
}

/// Hostility classification is supplied by the validated `PvP` hook, never inferred from CP
/// being nonzero. Frozen player attribution continues to absorb CP for retained periodic effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct DamageReduction {
    pub cause: DamageCause,
    pub source_kind: EffectSourceKind,
    pub classification: HostilityClassification,
    pub awake: bool,
    pub rolled_damage: u32,
}

/// Staged recipient facts. The zone must install these and perform the first lethal
/// transition immediately, before another resolution or any post-reduction caller draw.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct DamageMutation {
    pub phase3: Phase3Entity,
    pub resources: DamageResources,
    pub removed_effects: Vec<ActiveEffect>,
    pub rolled_damage: u32,
    pub nonlethal_prevented: u32,
    pub cp_absorbed: u32,
    pub hp_removed: u32,
    pub overkill: u32,
    pub lethal: bool,
}

/// Domain invariant or unavailable resource/stat computation. A rejected staged candidate
/// is never partially installed; RNG ownership remains with the enclosing immutable tick.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[allow(missing_docs)]
pub enum DamageError {
    #[error("invalid damage resources, classification or HP-consumption cost")]
    Invalid,
    #[error(transparent)]
    Effect(#[from] EffectError),
    #[error(transparent)]
    Math(#[from] StatError),
}

/// Read-only absorption projection for the magical caller's BEFORE-reduction attack-break
/// input. Overkill and any `TickHp` prevention are excluded; no status/RNG/resource mutation.
pub fn projected_absorption(
    resources: DamageResources,
    request: DamageReduction,
) -> Result<u32, DamageError> {
    validate_resources(resources, request)?;
    let damage = limited_damage(resources.hp, request);
    let cp = if request.classification.absorb_cp {
        resources.cp.unwrap_or(0).min(damage)
    } else {
        0
    };
    Ok(cp.saturating_add(damage.saturating_sub(cp).min(resources.hp)))
}

fn validate_resources(
    resources: DamageResources,
    request: DamageReduction,
) -> Result<(), DamageError> {
    let expected_cp = request.cause != DamageCause::HpConsumption
        && request.source_kind == EffectSourceKind::Player
        && resources.cp.is_some();
    if resources.hp == 0
        || request.classification.absorb_cp != expected_cp
        || (request.cause == DamageCause::HpConsumption && request.rolled_damage >= resources.hp)
        || (request.cause != DamageCause::HpConsumption
            && request.source_kind == EffectSourceKind::System)
    {
        return Err(DamageError::Invalid);
    }
    Ok(())
}
fn limited_damage(hp: u32, request: DamageReduction) -> u32 {
    if request.cause == DamageCause::Periodic {
        request.rolled_damage.min(hp.saturating_sub(1))
    } else {
        request.rolled_damage
    }
}
fn remove(
    sidecar: &mut Phase3Entity,
    id: super::effects::EffectId,
    removed: &mut Vec<ActiveEffect>,
) {
    if let Some(effect) = sidecar.effects.remove(id) {
        sidecar
            .modifier_install_order
            .retain(|_, source| *source != RestoredModifierSource::Effect(id));
        removed.push(effect);
    }
}

/// Recipient transaction, including zero/CP-only damage. `draw` is called exactly once with
/// bound10 iff stun remains after awake-dependent removals and this is a direct path.
/// No periodic/HP-consumption stun draw, and no paralyze-specific stun removal.
pub fn prepare_hp_reduction(
    original: &Phase3Entity,
    mut resources: DamageResources,
    request: DamageReduction,
    rules: &Phase3Rules,
    mut draw: impl FnMut(u32) -> Result<u32, StatError>,
    rebuild: impl FnOnce(&Phase3Entity) -> Result<DamageMaxima, StatError>,
) -> Result<DamageMutation, DamageError> {
    validate_resources(resources, request)?;
    if original.player.is_some() != resources.cp.is_some() {
        return Err(DamageError::Invalid);
    }
    original.fractions.validate()?;
    let damage = limited_damage(resources.hp, request);
    let mut phase3 = original.clone();
    let mut removed_effects = Vec::new();
    // Validating all sources before drawing also rejects corrupt no-status providers.
    for effect in original.effects.entries().values() {
        effect.validate_source(rules.skills())?;
    }
    if request.cause != DamageCause::HpConsumption
        && request.awake
        && (original.player.is_some() || request.cause != DamageCause::Periodic)
    {
        // CharEffectList.java:847-875 walks buffs, dances, then debuffs (unsupported
        // triggered/toggle pools are absent). Each pool retains insertion order.
        let mut candidates: Vec<_> = original
            .effects
            .entries()
            .values()
            .filter(|effect| effect.flags.removed_on_damage)
            .map(|effect| (effect.pool, effect.applied_at, effect.id))
            .collect();
        candidates.sort_unstable();
        for (_, _, id) in candidates {
            remove(&mut phase3, id, &mut removed_effects);
        }
    }
    if request.cause == DamageCause::Direct
        && phase3
            .effects
            .status_flags(rules.skills())?
            .intersects(StatusFlags::STUN)
    {
        let roll = draw(10)?;
        if roll >= 10 {
            return Err(DamageError::Invalid);
        }
        if roll == 0 {
            let mut stunned = phase3
                .effects
                .entries()
                .iter()
                .filter_map(|(id, effect)| {
                    let skill = rules.skills().get(effect.skill.id)?;
                    effect
                        .handlers
                        .iter()
                        .any(|h| {
                            usize::try_from(h.xml_ordinal)
                                .ok()
                                .and_then(|i| skill.effects.get(i))
                                .is_some_and(|h| h.name == "Stun")
                        })
                        .then_some((effect.pool, effect.applied_at, *id))
                })
                .collect::<Vec<_>>();
            stunned.sort_unstable();
            for (_, _, id) in stunned {
                remove(&mut phase3, id, &mut removed_effects);
            }
        }
    }
    if !removed_effects.is_empty() {
        let maxima = rebuild(&phase3)?;
        if maxima.hp == 0 || maxima.cp.is_some() != resources.cp.is_some() {
            return Err(DamageError::Invalid);
        }
        let clamp = |current: &mut u32, fraction: &mut i64, maximum: u32| {
            if *current >= maximum {
                *current = maximum;
                *fraction = 0;
            }
        };
        clamp(&mut resources.hp, &mut phase3.fractions.hp, maxima.hp);
        clamp(&mut resources.mp, &mut phase3.fractions.mp, maxima.mp);
        if let Some((cp, maximum)) = resources.cp.as_mut().zip(maxima.cp) {
            clamp(cp, &mut phase3.fractions.cp, maximum);
        }
    }
    let cp_absorbed = if request.classification.absorb_cp {
        resources.cp.unwrap_or(0).min(damage)
    } else {
        0
    };
    let after_cp = damage.saturating_sub(cp_absorbed);
    let hp_removed = after_cp.min(resources.hp);
    resources.hp = resources.hp.saturating_sub(hp_removed);
    if let Some(cp) = &mut resources.cp {
        *cp = cp.saturating_sub(cp_absorbed);
    }
    let lethal = resources.hp == 0;
    if lethal {
        phase3.fractions = ResourceFractions::default();
    }
    Ok(DamageMutation {
        phase3,
        resources,
        removed_effects,
        rolled_damage: request.rolled_damage,
        nonlethal_prevented: request.rolled_damage.saturating_sub(damage),
        cp_absorbed,
        hp_removed,
        overkill: after_cp.saturating_sub(hp_removed),
        lethal,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::super::effects::EffectList;
    use super::super::phase3_entity::tests::{blank, source_effect};
    use super::*;
    use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};
    fn request(
        cause: DamageCause,
        damage: u32,
        player_source: bool,
        player_target: bool,
    ) -> DamageReduction {
        DamageReduction {
            cause,
            source_kind: if player_source {
                EffectSourceKind::Player
            } else {
                EffectSourceKind::Npc
            },
            classification: HostilityClassification {
                absorb_cp: cause != DamageCause::HpConsumption && player_source && player_target,
            },
            awake: true,
            rolled_damage: damage,
        }
    }
    const MAX: DamageMaxima = DamageMaxima {
        hp: 1000,
        mp: 1000,
        cp: Some(1000),
    };
    #[test]
    fn damage_removal_walks_source_pools_before_application_ids() {
        let mut source = Phase3DataSource::embedded();
        let text = source.skills.0.get_mut("skills/1040.toml").unwrap();
        *text = text.replace("removed_on_damage = false", "removed_on_damage = true");
        let rules = load_phase3_rules(&source, &[]).unwrap();
        let mut original = blank();
        let buff = source_effect(&rules, 7, 1040);
        let sleep = source_effect(&rules, 2, 1069);
        original.effects =
            EffectList::from_entries([(buff.id, buff), (sleep.id, sleep)].into()).unwrap();
        for id in original.effects.entries().keys() {
            original
                .modifier_install_order
                .insert(id.0, RestoredModifierSource::Effect(*id));
        }
        let result = prepare_hp_reduction(
            &original,
            DamageResources {
                hp: 100,
                mp: 11,
                cp: Some(20),
            },
            request(DamageCause::Direct, 0, true, true),
            &rules,
            |_| unreachable!(),
            |_| Ok(MAX),
        )
        .unwrap();
        assert_eq!(
            result
                .removed_effects
                .iter()
                .map(|e| e.id.0)
                .collect::<Vec<_>>(),
            [7, 2]
        );
    }
    #[test]
    fn removal_barrier_precedes_subtraction_and_clamping_is_not_reported_as_damage() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let mut original = blank();
        let effect = source_effect(&rules, 1, 1069);
        original.effects = EffectList::from_entries([(effect.id, effect)].into()).unwrap();
        original
            .modifier_install_order
            .insert(1, RestoredModifierSource::Effect(super::super::effects::EffectId(1)));
        original.fractions = ResourceFractions {
            hp: 123,
            mp: 456,
            cp: 789,
        };
        let resources = DamageResources {
            hp: 100,
            mp: 11,
            cp: Some(20),
        };
        let request = request(DamageCause::Direct, 50, true, true);
        assert_eq!(projected_absorption(resources, request).unwrap(), 50);
        let reduced = prepare_hp_reduction(
            &original,
            resources,
            request,
            &rules,
            |_| unreachable!(),
            |state| {
                assert!(state.effects.entries().is_empty());
                assert!(state.modifier_install_order.is_empty());
                Ok(DamageMaxima {
                    hp: 80,
                    mp: 10,
                    cp: Some(5),
                })
            },
        )
        .unwrap();
        assert_eq!(
            reduced.resources,
            DamageResources {
                hp: 35,
                mp: 10,
                cp: Some(0)
            }
        );
        assert_eq!((reduced.hp_removed, reduced.cp_absorbed, reduced.overkill), (45, 5, 0));
        assert_eq!(reduced.phase3.fractions, ResourceFractions::default());
        assert_eq!(original.effects.entries().len(), 1);
        assert_eq!(original.fractions.hp, 123);
        let mut plain = blank();
        plain.fractions = original.fractions;
        let reduced = prepare_hp_reduction(
            &plain,
            resources,
            request,
            &rules,
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .unwrap();
        assert_eq!(reduced.phase3.fractions, plain.fractions);
        let lethal = prepare_hp_reduction(
            &plain,
            resources,
            DamageReduction {
                rolled_damage: 1000,
                ..request
            },
            &rules,
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(lethal.lethal);
        assert_eq!(lethal.phase3.fractions, ResourceFractions::default());
    }
    #[test]
    fn absorption_conserves_whole_damage_and_periodic_clamps_before_cp() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        for hp in [1, 10, 100] {
            for cp in [0, 20, 100] {
                for amount in [0, 1, 50, u32::MAX] {
                    for cause in [DamageCause::Direct, DamageCause::Periodic] {
                        let original = blank();
                        let resources = DamageResources {
                            hp,
                            mp: 11,
                            cp: Some(cp),
                        };
                        let request = request(cause, amount, true, true);
                        let projection = projected_absorption(resources, request).unwrap();
                        let result = prepare_hp_reduction(
                            &original,
                            resources,
                            request,
                            &rules,
                            |_| unreachable!(),
                            |_| unreachable!(),
                        )
                        .unwrap();
                        assert_eq!(projection, result.cp_absorbed + result.hp_removed);
                        assert_eq!(
                            u64::from(amount),
                            u64::from(result.nonlethal_prevented)
                                + u64::from(result.cp_absorbed)
                                + u64::from(result.hp_removed)
                                + u64::from(result.overkill)
                        );
                        if cause == DamageCause::Periodic {
                            assert!(!result.lethal);
                        }
                        assert_eq!(result.resources.mp, 11);
                        assert_eq!(original, blank());
                    }
                }
            }
        }
    }
    #[test]
    fn player_and_npc_periodic_status_paths_and_zero_direct_stun_draw_order() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        for player in [false, true] {
            for cause in [
                DamageCause::Direct,
                DamageCause::Periodic,
                DamageCause::HpConsumption,
            ] {
                for awake in [false, true] {
                    for roll in [0, 1, 9] {
                        let mut original = blank();
                        if !player {
                            original.player = None;
                        }
                        let sleep = source_effect(&rules, 2, 1069);
                        assert!(sleep.flags.removed_on_damage);
                        let stun = source_effect(&rules, 1, 100);
                        original.effects =
                            EffectList::from_entries([(sleep.id, sleep), (stun.id, stun)].into())
                                .unwrap();
                        for id in original.effects.entries().keys() {
                            original
                                .modifier_install_order
                                .insert(id.0, RestoredModifierSource::Effect(*id));
                        }
                        let mut calls = Vec::new();
                        let mut req = request(cause, 0, true, player);
                        req.awake = awake;
                        let result = prepare_hp_reduction(
                            &original,
                            DamageResources {
                                hp: 100,
                                mp: 11,
                                cp: player.then_some(20),
                            },
                            req,
                            &rules,
                            |bound| {
                                calls.push(bound);
                                Ok(roll)
                            },
                            |state| {
                                assert_eq!(
                                    state
                                        .effects
                                        .status_flags(rules.skills())
                                        .unwrap()
                                        .intersects(StatusFlags::STUN),
                                    cause != DamageCause::Direct || roll != 0
                                );
                                Ok(DamageMaxima {
                                    cp: player.then_some(1000),
                                    ..MAX
                                })
                            },
                        )
                        .unwrap();
                        assert_eq!(
                            calls,
                            if cause == DamageCause::Direct {
                                vec![10]
                            } else {
                                vec![]
                            }
                        );
                        let sleep_removed = awake
                            && cause != DamageCause::HpConsumption
                            && (player || cause != DamageCause::Periodic);
                        assert_eq!(
                            result
                                .phase3
                                .effects
                                .entries()
                                .contains_key(&super::super::effects::EffectId(2)),
                            !sleep_removed
                        );
                        assert_eq!(result.resources.hp, 100);
                        assert_eq!(result.hp_removed, 0);
                        assert_eq!(
                            result.phase3.modifier_install_order.len(),
                            result.phase3.effects.entries().len()
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn hp_cost_cannot_kill_or_consume_cp_and_bad_classification_never_draws() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let original = blank();
        let resources = DamageResources {
            hp: 10,
            mp: 11,
            cp: Some(20),
        };
        let reduction = request(DamageCause::HpConsumption, 9, true, true);
        let result = prepare_hp_reduction(
            &original,
            resources,
            reduction,
            &rules,
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .unwrap();
        assert_eq!(result.resources, DamageResources { hp: 1, ..resources });
        assert!(prepare_hp_reduction(
            &original,
            resources,
            DamageReduction {
                rolled_damage: 10,
                ..reduction
            },
            &rules,
            |_| unreachable!(),
            |_| unreachable!()
        )
        .is_err());
        assert!(projected_absorption(
            resources,
            DamageReduction {
                cause: DamageCause::Direct,
                ..reduction
            }
        )
        .is_err());
    }
}
