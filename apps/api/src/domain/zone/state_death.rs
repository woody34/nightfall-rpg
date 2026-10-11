//! V5 death/offer/revival transactions. These stage canonical sidecar changes; the zone
//! installs them with its combat/progression mutation and refreshes the deadline index.
//! No callback, stat installation, or RNG is replayed during reconstruction.
//!
//! Policy: architecture R7/R12, D-R5/D-R5', design v3 sections 1.3/3. Source revival order:
//! L2Character.java:2098-2124, L2PcInstance.java:4700-4704,7984-7987. Death penalty's 20%
//! chance and maximum rank5 are explicit architecture A15 policy, not source defaults.

use super::effects::{ActiveEffect, EffectError, TargetBinding};
use super::skill::{SkillId, SkillRef};
use super::skill_math::{resurrect_percent, resurrect_xp, ExactRatio};
use super::state_regen::RegenSchedule;
use super::{
    CastInFlight, CastState, DeathRecord, EntityKind, OfferId, Phase3Entity, Phase3Rules,
    ResourceFractions, RestoredModifierSource, ResurrectOffer, StatError, Tick,
};
use crate::domain::class::RuntimeSkillStatus;

/// A gameplay refusal is distinct from malformed canonical state/arithmetic exhaustion.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[allow(missing_docs)]
pub enum DeathError {
    #[error("invalid death/offer state")]
    InvalidState,
    #[error("no current resurrection offer")]
    NoOffer,
    #[error("resurrection is blocked")]
    Blocked,
    #[error("death/offer allocation exhausted")]
    Exhausted,
    #[error(transparent)]
    Effect(#[from] EffectError),
    #[error(transparent)]
    Math(#[from] StatError),
}

/// Death facts captured before resource/karma/level changes. The caller guards the first
/// lethal transition using authoritative life state, then applies this mutation once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct DeathContext {
    pub incarnation: u32,
    pub was_alive: bool,
    pub killer_kind: Option<EntityKind>,
    pub inside_peace: bool,
    pub pre_death_xp: u64,
    /// Actual legacy-table loss, after the XP-floor clamp, never a nominal percentage.
    pub xp_lost: u64,
}

/// Ordered removals/cancellation and one canonical candidate. The zone must rebuild stats
/// after installing the candidate, before any subsequent same-tick resolution reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct DeathMutation {
    pub phase3: Phase3Entity,
    pub removed_effects: Vec<ActiveEffect>,
    pub cancelled_cast: Option<CastInFlight>,
    pub invalidated_offer: Option<ResurrectOffer>,
    pub penalty_increased: bool,
    pub next_install_sequence: u64,
}

/// Exact no-draw branches: non-player victim, ineligible source/karma/geometry, or rank5.
/// NPC death has no penalty; a player killed by a player qualifies only while chaotic.
pub fn death_penalty_roll_required(
    sidecar: &Phase3Entity,
    context: DeathContext,
) -> Result<bool, DeathError> {
    if !context.was_alive || context.incarnation == 0 || context.xp_lost > context.pre_death_xp {
        return Err(DeathError::InvalidState);
    }
    let Some(player) = &sidecar.player else {
        return if context.pre_death_xp == 0 && context.xp_lost == 0 {
            Ok(false)
        } else {
            Err(DeathError::InvalidState)
        };
    };
    if player.death_penalty_level > 5
        || player
            .last_death
            .as_ref()
            .is_some_and(|d| d.incarnation == context.incarnation)
    {
        return Err(DeathError::InvalidState);
    }
    Ok(!context.inside_peace
        && player.death_penalty_level < 5
        && (context.killer_kind == Some(EntityKind::Npc)
            || (context.killer_kind == Some(EntityKind::Player) && player.pvp.karma > 0)))
}

/// Stages removal in `EffectId` order, clears fractional credit/regen/cast/offer, records the
/// exact loss, and installs a new system penalty sequence on a successful eligible roll.
/// `roll` must be present iff `death_penalty_roll_required` is true; the zone owns its RNG.
pub fn prepare_death(
    sidecar: &Phase3Entity,
    context: DeathContext,
    rules: &Phase3Rules,
    roll: Option<u32>,
    next_install_sequence: u64,
) -> Result<DeathMutation, DeathError> {
    let needs_roll = death_penalty_roll_required(sidecar, context)?;
    if needs_roll != roll.is_some() || roll.is_some_and(|n| n >= 100) {
        return Err(DeathError::InvalidState);
    }
    let increased = roll.is_some_and(|n| n < 20);
    let mut candidate = sidecar.clone();
    let mut removed_effects = Vec::new();
    for effect in sidecar.effects.entries().values() {
        effect.validate_source(rules.skills())?;
        if effect.target_incarnation != context.incarnation {
            return Err(DeathError::InvalidState);
        }
        if !effect.flags.stay_after_death {
            candidate.effects.remove(effect.id);
            candidate
                .modifier_install_order
                .retain(|_, source| *source != RestoredModifierSource::Effect(effect.id));
            removed_effects.push(effect.clone());
        }
    }
    candidate.regen.stop();
    candidate.fractions = ResourceFractions::default();
    let cancelled_cast = candidate.cancel_cast(None);
    let mut invalidated_offer = None;
    let mut next_sequence = next_install_sequence;
    if let Some(player) = &mut candidate.player {
        invalidated_offer = player.resurrect_offer.take();
        player.last_death = Some(DeathRecord {
            incarnation: context.incarnation,
            pre_death_xp: context.pre_death_xp,
            recoverable_death_xp_loss: context.xp_lost,
        });
        if increased {
            let penalty = rules
                .skills()
                .get(SkillId(5076))
                .ok_or(DeathError::InvalidState)?;
            if penalty.runtime_status != RuntimeSkillStatus::Executable || penalty.max_level < 5 {
                return Err(DeathError::InvalidState);
            }
            if next_sequence == 0
                || sidecar
                    .modifier_install_order
                    .keys()
                    .any(|old| *old >= next_sequence)
            {
                return Err(DeathError::Exhausted);
            }
            next_sequence = next_sequence.checked_add(1).ok_or(DeathError::Exhausted)?;
            player.death_penalty_level = player
                .death_penalty_level
                .checked_add(1)
                .ok_or(DeathError::Exhausted)?;
            candidate
                .modifier_install_order
                .retain(|_, source| *source != RestoredModifierSource::DeathPenalty);
            candidate
                .modifier_install_order
                .insert(next_install_sequence, RestoredModifierSource::DeathPenalty);
        }
    }
    Ok(DeathMutation {
        phase3: candidate,
        removed_effects,
        cancelled_cast,
        invalidated_offer,
        penalty_increased: increased,
        next_install_sequence: next_sequence,
    })
}

/// Authoritative state at offer creation. Target-kind/range and caster-incarnation checks
/// belong to the completing skill; this interface cannot turn a live target into a corpse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct OfferContext {
    pub now: Tick,
    pub caster: TargetBinding,
    pub target_incarnation: u32,
    pub target_dead: bool,
    pub resurrection_blocked: bool,
    pub skill: SkillRef,
    pub wit_bonus: ExactRatio,
}

/// A current first offer is preserved verbatim and consumes no new identity. A stale offer
/// may be replaced only after expiry or death invalidation, which the zone reports first.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum OfferMutation {
    Existing(ResurrectOffer),
    Created {
        offer: ResurrectOffer,
        next_offer_id: u64,
    },
}

/// Read the actual Resurrection handler power; never accept client-supplied percentages.
/// The WIT quirk is in Formulas.java:1372-1392; offer lifetime600 is architecture R7.
pub fn prepare_resurrect_offer(
    sidecar: &Phase3Entity,
    context: OfferContext,
    rules: &Phase3Rules,
    next_offer_id: u64,
) -> Result<OfferMutation, DeathError> {
    if !context.target_dead || context.target_incarnation == 0 || context.caster.incarnation == 0 {
        return Err(DeathError::InvalidState);
    }
    if context.resurrection_blocked {
        return Err(DeathError::Blocked);
    }
    let player = sidecar.player.as_ref().ok_or(DeathError::InvalidState)?;
    let death = player.last_death.as_ref().ok_or(DeathError::InvalidState)?;
    if death.incarnation != context.target_incarnation
        || death.recoverable_death_xp_loss > death.pre_death_xp
    {
        return Err(DeathError::InvalidState);
    }
    let skill = rules
        .skills()
        .get(context.skill.id)
        .ok_or(DeathError::InvalidState)?;
    if skill.runtime_status != RuntimeSkillStatus::Executable
        || context.skill.rank == 0
        || context.skill.rank > skill.max_level
    {
        return Err(DeathError::InvalidState);
    }
    let handler = skill
        .effects
        .iter()
        .find(|h| h.name == "Resurrection")
        .ok_or(DeathError::InvalidState)?;
    let power = handler
        .params
        .get("power")
        .and_then(|p| p.at(context.skill.rank, skill.max_level))
        .ok_or(DeathError::InvalidState)?;
    let power = ExactRatio::parse_literal(power)?;
    let percent = resurrect_percent(power, context.wit_bonus)?.to_q()?;
    if !(0..=100_000_000).contains(&percent) {
        return Err(DeathError::InvalidState);
    }
    if let Some(existing) = &player.resurrect_offer {
        if existing.target_incarnation == context.target_incarnation
            && existing.created_at <= context.now
            && context.now < existing.expires_at
        {
            return Ok(OfferMutation::Existing(existing.clone()));
        }
    }
    if next_offer_id == 0 {
        return Err(DeathError::Exhausted);
    }
    Ok(OfferMutation::Created {
        offer: ResurrectOffer {
            id: OfferId(next_offer_id),
            caster: context.caster,
            target_incarnation: context.target_incarnation,
            skill: context.skill,
            created_at: context.now,
            expires_at: Tick(
                context
                    .now
                    .0
                    .checked_add(600)
                    .ok_or(DeathError::Exhausted)?,
            ),
            restore_percent_q: percent,
            recoverable_xp: death.recoverable_death_xp_loss,
        },
        next_offer_id: next_offer_id.checked_add(1).ok_or(DeathError::Exhausted)?,
    })
}

/// Guarded response names the exact original death and offer; no selection lookup occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct AcceptContext {
    pub now: Tick,
    pub incarnation: u32,
    pub dead: bool,
    pub resurrection_blocked: bool,
    pub offer_id: OfferId,
    pub accept: bool,
    pub current_xp: u64,
}

/// The caller revives at the corpse without protection, derives the final XP/level/passive
/// sheet, clamps resources, then installs the candidate and emits final public vitals.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct AcceptMutation {
    pub phase3: Phase3Entity,
    pub offer: ResurrectOffer,
    pub accepted: bool,
    pub incarnation: u32,
    pub xp_restored: u64,
    pub xp_after: u64,
}

/// Expiry wins at equality. No remaining loss survives successful consumption; declines
/// preserve it for a later offer. Incarnation and regen overflow leave input untouched.
pub fn prepare_accept_resurrect(
    sidecar: &Phase3Entity,
    context: AcceptContext,
    rules: &Phase3Rules,
) -> Result<AcceptMutation, DeathError> {
    let player = sidecar.player.as_ref().ok_or(DeathError::NoOffer)?;
    let offer = player.resurrect_offer.as_ref().ok_or(DeathError::NoOffer)?;
    let death = player.last_death.as_ref().ok_or(DeathError::NoOffer)?;
    if !context.dead
        || context.incarnation == 0
        || offer.id != context.offer_id
        || offer.target_incarnation != context.incarnation
        || death.incarnation != context.incarnation
        || context.now < offer.created_at
        || context.now >= offer.expires_at
        || offer.recoverable_xp != death.recoverable_death_xp_loss
    {
        return Err(DeathError::NoOffer);
    }
    if context.accept && context.resurrection_blocked {
        return Err(DeathError::Blocked);
    }
    if death.recoverable_death_xp_loss > death.pre_death_xp
        || !(0..=100_000_000).contains(&offer.restore_percent_q)
    {
        return Err(DeathError::InvalidState);
    }
    let mut phase3 = sidecar.clone();
    let candidate = phase3.player.as_mut().ok_or(DeathError::InvalidState)?;
    candidate.resurrect_offer = None;
    let mut incarnation = context.incarnation;
    let mut xp_restored = 0;
    if context.accept {
        incarnation = incarnation.checked_add(1).ok_or(DeathError::Exhausted)?;
        xp_restored = resurrect_xp(
            offer.recoverable_xp,
            ExactRatio::new(i128::from(offer.restore_percent_q), i128::from(super::Q))?,
        )?
        .min(death.pre_death_xp.saturating_sub(context.current_xp));
        candidate.last_death = None;
        phase3.cast = CastState::Idle;
        phase3.fractions = ResourceFractions::default();
        phase3.regen = RegenSchedule::start(context.now, true, rules.regen())?;
        // Stay-after-death effects retain IDs, schedule and install order, but belong to
        // the revived life. Attribution remains the original source incarnation.
        phase3.effects = super::effects::EffectList::from_entries(
            phase3
                .effects
                .entries()
                .iter()
                .map(|(id, effect)| {
                    let mut effect = effect.clone();
                    effect.target_incarnation = incarnation;
                    (*id, effect)
                })
                .collect(),
        )?;
    }
    Ok(AcceptMutation {
        phase3,
        offer: offer.clone(),
        accepted: context.accept,
        incarnation,
        xp_restored,
        xp_after: context
            .current_xp
            .checked_add(xp_restored)
            .ok_or(DeathError::Exhausted)?,
    })
}

impl Phase3Entity {
    /// Expiry-phase hook; returning the removed value supplies the exact owner event.
    pub fn expire_resurrect_offer(&mut self, now: Tick) -> Option<ResurrectOffer> {
        let player = self.player.as_mut()?;
        if player
            .resurrect_offer
            .as_ref()
            .is_some_and(|offer| offer.expires_at <= now)
        {
            player.resurrect_offer.take()
        } else {
            None
        }
    }

    /// Real departure invalidates the live offer while retaining recoverable XP. Socket
    /// replacement must not call this: it preserves live offer identity and deadlines.
    pub fn invalidate_resurrect_offer(&mut self) -> Option<ResurrectOffer> {
        self.player.as_mut()?.resurrect_offer.take()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::super::effects::*;
    use super::super::phase3_entity::tests::blank;
    use super::super::{EntityId, Phase3EntityContext, Q};
    use super::*;
    use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};

    fn rules() -> Phase3Rules {
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap()
    }
    fn death() -> DeathContext {
        DeathContext {
            incarnation: 1,
            was_alive: true,
            killer_kind: Some(EntityKind::Npc),
            inside_peace: false,
            pre_death_xp: 1_000,
            xp_lost: 100,
        }
    }
    fn offer() -> OfferContext {
        OfferContext {
            now: Tick(10),
            caster: TargetBinding {
                entity: EntityId::from_uuid(uuid::Uuid::from_u128(7)),
                incarnation: 2,
            },
            target_incarnation: 1,
            target_dead: true,
            resurrection_blocked: false,
            skill: SkillRef {
                id: SkillId(1016),
                rank: 3,
            },
            wit_bonus: ExactRatio::whole(2),
        }
    }
    fn accept() -> AcceptContext {
        AcceptContext {
            now: Tick(11),
            incarnation: 1,
            dead: true,
            resurrection_blocked: false,
            offer_id: OfferId(1),
            accept: true,
            current_xp: 900,
        }
    }
    fn corpse(rules: &Phase3Rules) -> Phase3Entity {
        prepare_death(&blank(), death(), rules, Some(20), 1)
            .unwrap()
            .phase3
    }
    fn offered(rules: &Phase3Rules) -> Phase3Entity {
        let mut state = corpse(rules);
        let OfferMutation::Created { offer, .. } =
            prepare_resurrect_offer(&state, offer(), rules, 1).unwrap()
        else {
            unreachable!()
        };
        state.player.as_mut().unwrap().resurrect_offer = Some(offer);
        state
    }
    fn effect(rules: &Phase3Rules, id: u64, skill_id: u32) -> ActiveEffect {
        let skill = rules.skills().get(SkillId(skill_id)).unwrap();
        let abnormal = skill.abnormal.as_ref().unwrap();
        let flag = |v: &super::super::skill::RankValue<bool>| *v.at(1, skill.max_level).unwrap();
        ActiveEffect {
            id: EffectId(id),
            skill: SkillRef {
                id: skill.id,
                rank: 1,
            },
            source: EffectSource {
                binding: Some(offer().caster),
                kind: EffectSourceKind::Player,
            },
            target_incarnation: 1,
            applied_at: Tick(1),
            expires_at: Some(Tick(1 + u64::try_from(abnormal.time_s[0]).unwrap() * 10)),
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
                .map(|(ordinal, _)| ActiveHandler {
                    xml_ordinal: u32::try_from(ordinal).unwrap(),
                    periodic: None,
                })
                .collect(),
        }
    }

    #[test]
    fn penalty_truth_table_roll_boundaries_and_cap_preserve_pvp_reuse() {
        let rules = rules();
        for source in [None, Some(EntityKind::Player), Some(EntityKind::Npc)] {
            for karma in [0, 1] {
                for peace in [false, true] {
                    for rank in [0, 4, 5] {
                        let mut sidecar = blank();
                        let player = sidecar.player.as_mut().unwrap();
                        player.pvp.karma = karma;
                        player.pvp.flag_until = Some(Tick(100));
                        player.death_penalty_level = rank;
                        if rank != 0 {
                            sidecar
                                .modifier_install_order
                                .insert(1, RestoredModifierSource::DeathPenalty);
                        }
                        sidecar.reuse.insert(SkillId(3), Tick(300));
                        let context = DeathContext {
                            killer_kind: source,
                            inside_peace: peace,
                            ..death()
                        };
                        let eligible = !peace
                            && rank < 5
                            && (source == Some(EntityKind::Npc)
                                || (source == Some(EntityKind::Player) && karma > 0));
                        assert_eq!(
                            death_penalty_roll_required(&sidecar, context).unwrap(),
                            eligible
                        );
                        for roll in [0, 19, 20, 99] {
                            let mutation = prepare_death(
                                &sidecar,
                                context,
                                &rules,
                                eligible.then_some(roll),
                                2,
                            )
                            .unwrap();
                            let result = mutation.phase3.player.as_ref().unwrap();
                            assert_eq!(
                                result.death_penalty_level,
                                rank + u8::from(eligible && roll < 20)
                            );
                            assert_eq!(result.pvp, sidecar.player.as_ref().unwrap().pvp);
                            assert_eq!(mutation.phase3.reuse, sidecar.reuse);
                            assert_eq!(
                                mutation.next_install_sequence,
                                2 + u64::from(eligible && roll < 20)
                            );
                        }
                    }
                }
            }
        }
        let mut npc = blank();
        npc.player = None;
        assert!(!death_penalty_roll_required(
            &npc,
            DeathContext {
                pre_death_xp: 0,
                xp_lost: 0,
                ..death()
            }
        )
        .unwrap());
    }

    #[test]
    fn death_is_atomic_on_exhaustion_invalid_roll_or_second_transition() {
        let rules = rules();
        let sidecar = blank();
        let original = sidecar.clone();
        for (roll, next) in [(None, 1), (Some(100), 1), (Some(0), 0), (Some(0), u64::MAX)] {
            assert!(prepare_death(&sidecar, death(), &rules, roll, next).is_err());
            assert_eq!(sidecar, original);
        }
        let dead = corpse(&rules);
        assert!(prepare_death(&dead, death(), &rules, Some(0), 1).is_err());
        assert!(prepare_death(
            &sidecar,
            DeathContext {
                was_alive: false,
                ..death()
            },
            &rules,
            Some(0),
            1
        )
        .is_err());
    }

    #[test]
    fn death_removes_source_flagged_effects_in_id_order_and_revival_rebinds_retained_life() {
        // Executable test profile explicitly supplies retention, without editing source data.
        let mut input = Phase3DataSource::embedded();
        let text = input.skills.0.get_mut("skills/1040.toml").unwrap();
        *text = text.replace("stay_after_death = false", "stay_after_death = true");
        let rules = load_phase3_rules(&input, &[]).unwrap();
        let mut sidecar = blank();
        sidecar.effects = EffectList::from_entries(
            [
                (EffectId(9), effect(&rules, 9, 1069)),
                (EffectId(2), effect(&rules, 2, 1201)),
                (EffectId(5), effect(&rules, 5, 1040)),
            ]
            .into(),
        )
        .unwrap();
        sidecar.modifier_install_order = [
            (1, RestoredModifierSource::Effect(EffectId(9))),
            (2, RestoredModifierSource::Effect(EffectId(2))),
            (3, RestoredModifierSource::Effect(EffectId(5))),
        ]
        .into();
        sidecar.fractions = ResourceFractions {
            hp: 3,
            mp: 4,
            cp: 5,
        };
        let mutation = prepare_death(&sidecar, death(), &rules, Some(0), 4).unwrap();
        assert_eq!(
            mutation
                .removed_effects
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            [EffectId(2), EffectId(9)]
        );
        assert_eq!(
            mutation.phase3.modifier_install_order,
            [
                (3, RestoredModifierSource::Effect(EffectId(5))),
                (4, RestoredModifierSource::DeathPenalty),
            ]
            .into()
        );
        assert_eq!(mutation.phase3.fractions, ResourceFractions::default());
        assert_eq!(mutation.phase3.regen.next_at, None);
        let mut dead = mutation.phase3;
        let OfferMutation::Created { offer, .. } =
            prepare_resurrect_offer(&dead, offer(), &rules, 1).unwrap()
        else {
            unreachable!()
        };
        dead.player.as_mut().unwrap().resurrect_offer = Some(offer);
        let alive = prepare_accept_resurrect(&dead, accept(), &rules).unwrap();
        assert_eq!(alive.phase3.effects.entries()[&EffectId(5)].target_incarnation, 2);
        assert_eq!(
            alive.phase3.effects.entries()[&EffectId(5)].source,
            sidecar.effects.entries()[&EffectId(5)].source
        );
        assert_eq!(alive.phase3.modifier_install_order, dead.modifier_install_order);
        alive
            .phase3
            .validate_live(
                &rules,
                Phase3EntityContext {
                    next_tick: Tick(12),
                    incarnation: 2,
                    alive: true,
                    player: true,
                    learned: &[],
                },
            )
            .unwrap();
    }

    #[test]
    fn first_offer_wins_and_expiry_equality_is_not_acceptable() {
        let rules = rules();
        let mut sidecar = offered(&rules);
        let original = sidecar
            .player
            .as_ref()
            .unwrap()
            .resurrect_offer
            .clone()
            .unwrap();
        assert_eq!(original.restore_percent_q, 80 * Q); // Source >20 quirk:30*2+20.
        assert_eq!(original.expires_at, Tick(610));
        assert_eq!(
            prepare_resurrect_offer(
                &sidecar,
                OfferContext {
                    skill: SkillRef {
                        id: SkillId(1016),
                        rank: 9
                    },
                    ..offer()
                },
                &rules,
                u64::MAX
            )
            .unwrap(),
            OfferMutation::Existing(original.clone())
        );
        assert!(sidecar.expire_resurrect_offer(Tick(609)).is_none());
        assert!(prepare_accept_resurrect(
            &sidecar,
            AcceptContext {
                now: Tick(610),
                ..accept()
            },
            &rules
        )
        .is_err());
        assert_eq!(sidecar.expire_resurrect_offer(Tick(610)), Some(original));
        assert!(sidecar.expire_resurrect_offer(Tick(611)).is_none());
        assert!(sidecar.player.as_ref().unwrap().last_death.is_some());
    }

    #[test]
    fn consume_once_decline_then_offer_again_and_snapshot_roundtrip() {
        let rules = rules();
        let original = offered(&rules);
        let encoded = serde_json::to_vec(&original).unwrap();
        let restored: Phase3Entity = serde_json::from_slice(&encoded).unwrap();
        let success = prepare_accept_resurrect(&restored, accept(), &rules).unwrap();
        assert_eq!(success, prepare_accept_resurrect(&original, accept(), &rules).unwrap());
        assert_eq!((success.xp_restored, success.xp_after, success.incarnation), (80, 980, 2));
        assert_eq!(success.phase3.regen.next_at, Some(Tick(41)));
        assert!(success.phase3.player.as_ref().unwrap().last_death.is_none());
        assert!(prepare_accept_resurrect(&success.phase3, accept(), &rules).is_err());
        let declined = prepare_accept_resurrect(
            &restored,
            AcceptContext {
                accept: false,
                ..accept()
            },
            &rules,
        )
        .unwrap();
        assert!(!declined.accepted);
        assert_eq!((declined.incarnation, declined.xp_restored, declined.xp_after), (1, 0, 900));
        assert_eq!(
            declined.phase3.player.as_ref().unwrap().last_death,
            original.player.as_ref().unwrap().last_death
        );
        assert_eq!(declined.phase3.regen.next_at, None);
        assert!(matches!(
            prepare_resurrect_offer(&declined.phase3, offer(), &rules, 2).unwrap(),
            OfferMutation::Created {
                next_offer_id: 3,
                ..
            }
        ));
        let capped = prepare_accept_resurrect(
            &original,
            AcceptContext {
                current_xp: 995,
                ..accept()
            },
            &rules,
        )
        .unwrap();
        assert_eq!((capped.xp_restored, capped.xp_after), (5, 1000));
    }

    #[test]
    fn stale_life_offer_identity_block_and_departure_never_consume_loss() {
        let rules = rules();
        let mut sidecar = offered(&rules);
        let original = sidecar.clone();
        for context in [
            AcceptContext {
                incarnation: 2,
                ..accept()
            },
            AcceptContext {
                offer_id: OfferId(2),
                ..accept()
            },
            AcceptContext {
                dead: false,
                ..accept()
            },
            AcceptContext {
                resurrection_blocked: true,
                ..accept()
            },
            AcceptContext {
                now: Tick(9),
                ..accept()
            },
        ] {
            assert!(prepare_accept_resurrect(&sidecar, context, &rules).is_err());
            assert_eq!(sidecar, original);
        }
        assert!(sidecar.invalidate_resurrect_offer().is_some());
        assert_eq!(
            sidecar.player.as_ref().unwrap().last_death,
            original.player.as_ref().unwrap().last_death
        );
        assert!(prepare_accept_resurrect(&sidecar, accept(), &rules).is_err());
        for next in [0, u64::MAX] {
            assert!(prepare_resurrect_offer(&sidecar, offer(), &rules, next).is_err());
        }
        assert!(prepare_resurrect_offer(
            &sidecar,
            OfferContext {
                now: Tick(u64::MAX),
                ..offer()
            },
            &rules,
            1
        )
        .is_err());
    }
}
