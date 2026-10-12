//! Paid-learning command-phase guards and atomic ledger staging. The actor authenticates
//! and fences the lane/generation, resolves receipts first, and serializes lifecycle work.
//! The caller installs the candidate only after deriving/clamping its effective resources;
//! log and checkpoint acknowledgements still precede reply/event release.

use thiserror::Error;

use super::skill::SkillRef;
use super::{CastState, Fixed, Phase3Entity, Phase3Rules, RestoredModifierSource, Vec2Fixed};
use crate::domain::character_progression::{CharacterIdentity, ClassState};
use crate::domain::class::{ClassRegistry, RuntimeSkillStatus};
use crate::domain::subclass::LearnedSkill;
use crate::domain::CharacterId;

/// Rechecked authoritative facts at the command phase, before any same-tick kill award.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct LearnContext<'a> {
    pub character: CharacterId,
    pub identity: &'a CharacterIdentity,
    pub classes: &'a ClassRegistry,
    pub rules: &'a Phase3Rules,
    pub level: u32,
    pub alive: bool,
    /// Includes incoming hate/swings/readiness/casts and hostile retained effects, not only
    /// the learner's own auto-attack bit. This is computed by the zone's engagement index.
    pub engaged: bool,
    pub position: Vec2Fixed,
}

/// Stable mutation failures; transport/status mapping belongs to C's adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(missing_docs)]
pub enum LearnError {
    #[error("invalid authoritative player ledger")]
    InvalidState,
    #[error("dead actor")]
    DeadActor,
    #[error("actor is engaged or casting")]
    InCombat,
    #[error("Class Master is outside radius three")]
    TrainerRequired,
    #[error("unknown skill")]
    UnknownSkill,
    #[error("skill is outside current class lineage or automatically granted")]
    NotLearnable,
    #[error("requested rank is not learned rank plus one")]
    WrongRank,
    #[error("required character level is not reached")]
    LevelRequired,
    #[error("learning requires an inventory item")]
    ItemRequired,
    #[error("skill runtime is deferred")]
    DeferredSkill,
    #[error("insufficient SP")]
    InsufficientSp,
    #[error("modifier installation sequence exhausted")]
    SequenceExhausted,
}

/// All canonical mutations together, staged without touching live state. No receipt key
/// belongs here or in the simulation digest; private committed facts carry that key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct LearnMutation {
    pub skill: SkillRef,
    pub sp_spent: u64,
    pub ledger: ClassState,
    pub phase3: Phase3Entity,
    pub next_modifier_sequence: u64,
    /// A newly installed/ranked passive requires declarative stat reconstruction before
    /// the caller freezes success resources. Active learning changes no timed effects.
    pub passive_changed: bool,
}

/// Validate exact rank/capability/item/SP/command-phase guards and stage the one purchase.
/// The source of costs/requirements is the current lineage's `SkillLearnDef`; free autoget
/// reconciliation has its own path and can never debit through this function.
pub fn prepare_learn(
    ledger: &ClassState,
    phase3: &Phase3Entity,
    context: LearnContext<'_>,
    request: SkillRef,
    next_modifier_sequence: u64,
) -> Result<LearnMutation, LearnError> {
    let classes = context.classes;
    let rules = context.rules;
    ledger
        .validate_for_skill_rules(classes, context.identity, context.character, rules)
        .map_err(|_| LearnError::InvalidState)?;
    if phase3.player.is_none()
        || ledger.sp > i64::MAX.unsigned_abs()
        || !(1..=85).contains(&context.level)
    {
        return Err(LearnError::InvalidState);
    }
    if !context.alive {
        return Err(LearnError::DeadActor);
    }
    if context.engaged || !matches!(phase3.cast, CastState::Idle) {
        return Err(LearnError::InCombat);
    }
    if !context
        .position
        .within(Vec2Fixed::from_tiles(126, 128), Fixed::from_tiles(3))
    {
        return Err(LearnError::TrainerRequired);
    }
    let skill = rules
        .skills()
        .get(request.id)
        .ok_or(LearnError::UnknownSkill)?;
    let current = ledger
        .learned_skills
        .iter()
        .find(|entry| entry.key == skill.key)
        .map_or(0, |entry| entry.level);
    if current.checked_add(1) != Some(request.rank) {
        return Err(LearnError::WrongRank);
    }
    let row = classes
        .skill_tree(ledger.current_class_id)
        .map_err(|_| LearnError::InvalidState)?
        .into_iter()
        .find(|row| row.skill_id == request.id.0 && row.skill_level == request.rank)
        .ok_or(LearnError::NotLearnable)?;
    if row.auto_get || row.key != skill.key {
        return Err(LearnError::NotLearnable);
    }
    if context.level < row.required_level {
        return Err(LearnError::LevelRequired);
    }
    if !row.required_items.is_empty() {
        return Err(LearnError::ItemRequired);
    }
    if skill.runtime_status != RuntimeSkillStatus::Executable {
        return Err(LearnError::DeferredSkill);
    }
    let sp = ledger
        .sp
        .checked_sub(row.sp_cost)
        .ok_or(LearnError::InsufficientSp)?;
    let passive_changed = skill.operate_type == "P";
    let mut candidate = LearnMutation {
        skill: request,
        sp_spent: row.sp_cost,
        ledger: ledger.clone(),
        phase3: phase3.clone(),
        next_modifier_sequence,
        passive_changed,
    };
    candidate.ledger.sp = sp;
    let learned = LearnedSkill {
        key: skill.key.clone(),
        level: request.rank,
    };
    match candidate
        .ledger
        .learned_skills
        .binary_search_by(|entry| entry.key.cmp(&learned.key))
    {
        Ok(index) => {
            *candidate
                .ledger
                .learned_skills
                .get_mut(index)
                .ok_or(LearnError::InvalidState)? = learned;
        },
        Err(index) => candidate.ledger.learned_skills.insert(index, learned),
    }
    if passive_changed {
        if next_modifier_sequence == 0
            || candidate
                .phase3
                .modifier_install_order
                .keys()
                .any(|seq| *seq >= next_modifier_sequence)
        {
            return Err(LearnError::SequenceExhausted);
        }
        candidate.next_modifier_sequence = next_modifier_sequence
            .checked_add(1)
            .ok_or(LearnError::SequenceExhausted)?;
        candidate.phase3.modifier_install_order.retain(|_, source| !matches!(source, RestoredModifierSource::Passive(old) if old.id == request.id));
        candidate
            .phase3
            .modifier_install_order
            .insert(next_modifier_sequence, RestoredModifierSource::Passive(request));
    }
    candidate
        .ledger
        .validate_for_skill_rules(classes, context.identity, context.character, rules)
        .map_err(|_| LearnError::InvalidState)?;
    Ok(candidate)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::super::{phase3_entity::tests::blank, skill::SkillId};
    use super::*;
    use crate::domain::character_progression::CharacterAppearance;
    use crate::domain::class::ClassId;
    use crate::domain::{AccountId, Race};
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        skill_data::{load_phase3_rules, Phase3DataSource},
    };
    use uuid::Uuid;

    fn identity() -> CharacterIdentity {
        CharacterIdentity {
            account_id: AccountId::from_uuid(Uuid::from_u128(2)),
            race: Race::Human,
            base_class_id: ClassId(0),
            appearance: CharacterAppearance::default(),
        }
    }
    fn context<'a>(
        identity: &'a CharacterIdentity,
        classes: &'a ClassRegistry,
        rules: &'a Phase3Rules,
    ) -> LearnContext<'a> {
        LearnContext {
            character: CharacterId::from_uuid(Uuid::from_u128(1)),
            identity,
            classes,
            rules,
            level: 5,
            alive: true,
            engaged: false,
            position: Vec2Fixed::from_tiles(129, 128),
        }
    }
    fn skill(id: u32, rank: u32) -> SkillRef {
        SkillRef {
            id: SkillId(id),
            rank,
        }
    }

    #[test]
    fn exact_rank_purchase_is_atomic_and_keeps_legacy_validation_strict() {
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let identity = identity();
        let ctx = context(&identity, &classes, &rules);
        let mut ledger = ClassState::new(ClassId(0));
        ledger.sp = 100;
        let state = blank();
        let before = ledger.clone();
        let before_state = state.clone();
        let first = prepare_learn(&ledger, &state, ctx, skill(3, 1), 1).unwrap();
        assert_eq!((first.sp_spent, first.ledger.sp), (50, 50));
        assert_eq!(
            first.ledger.learned_skills,
            vec![LearnedSkill {
                key: "l2.skill.3".into(),
                level: 1
            }]
        );
        assert!(!first.passive_changed);
        assert_eq!(first.phase3, state);
        assert_eq!(first.next_modifier_sequence, 1);
        assert_eq!(ledger, before);
        assert_eq!(state, before_state);
        assert!(first
            .ledger
            .validate_for(&classes, &identity, ctx.character)
            .is_err());
        first
            .ledger
            .validate_for_phase3(&classes, &identity, ctx.character)
            .unwrap();
        let encoded = serde_json::to_vec(&first.ledger).unwrap();
        assert_eq!(serde_json::from_slice::<ClassState>(&encoded).unwrap(), first.ledger);
        assert_eq!(
            prepare_learn(&first.ledger, &first.phase3, ctx, skill(3, 1), 1),
            Err(LearnError::WrongRank)
        );
        let second = prepare_learn(&first.ledger, &first.phase3, ctx, skill(3, 2), 1).unwrap();
        assert_eq!(second.ledger.sp, 0);
        assert_eq!(second.ledger.learned_skills[0].level, 2);
        assert_eq!(
            prepare_learn(&second.ledger, &second.phase3, ctx, skill(3, 3), 1),
            Err(LearnError::InsufficientSp)
        );
    }
    #[test]
    fn passive_upgrade_allocates_once_and_can_be_cold_reconstructed() {
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let identity = identity();
        let mut ctx = context(&identity, &classes, &rules);
        ctx.level = 10;
        let mut ledger = ClassState::new(ClassId(0));
        ledger.sp = 2000;
        let first = prepare_learn(&ledger, &blank(), ctx, skill(141, 1), 7).unwrap();
        assert!(first.passive_changed);
        assert_eq!(first.next_modifier_sequence, 8);
        assert_eq!(
            first.phase3.modifier_install_order.get(&7),
            Some(&RestoredModifierSource::Passive(skill(141, 1)))
        );
        let second = prepare_learn(&first.ledger, &first.phase3, ctx, skill(141, 2), 8).unwrap();
        assert_eq!(second.phase3.modifier_install_order.len(), 1);
        assert!(!second.phase3.modifier_install_order.contains_key(&7));
        assert_eq!(
            second.phase3.modifier_install_order.get(&8),
            Some(&RestoredModifierSource::Passive(skill(141, 2)))
        );
        assert_eq!(second.ledger.sp, 740);
        assert_eq!(second.next_modifier_sequence, 9);
        let before = first.clone();
        assert_eq!(
            prepare_learn(&first.ledger, &first.phase3, ctx, skill(141, 2), u64::MAX),
            Err(LearnError::SequenceExhausted)
        );
        assert_eq!(
            prepare_learn(&first.ledger, &first.phase3, ctx, skill(141, 2), 7),
            Err(LearnError::SequenceExhausted)
        );
        assert_eq!(first, before);
        let stats = crate::infrastructure::rules_data::load_rules(
            &crate::infrastructure::rules_data::RulesSource::embedded(),
        )
        .unwrap();
        let input = super::super::effective_stats::PlayerStatInputs::from_class(
            &stats.rules,
            &classes,
            &rules,
            ClassId(0),
            10,
        )
        .unwrap();
        let baseline = input.derive(&blank(), &rules, false).unwrap();
        let learned = input.derive(&second.phase3, &rules, false).unwrap();
        assert!(learned.physical.p_atk() > baseline.physical.p_atk());
        let restored: Phase3Entity =
            serde_json::from_slice(&serde_json::to_vec(&second.phase3).unwrap()).unwrap();
        assert_eq!(input.derive(&restored, &rules, false).unwrap(), learned);
    }
    #[test]
    fn command_phase_guards_reject_without_consuming_sp_or_install_order() {
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let identity = identity();
        let ctx = context(&identity, &classes, &rules);
        let state = blank();
        let mut ledger = ClassState::new(ClassId(0));
        ledger.sp = 1000;
        let before = ledger.clone();
        for (changed, expected) in [
            (
                LearnContext {
                    alive: false,
                    ..ctx
                },
                LearnError::DeadActor,
            ),
            (
                LearnContext {
                    engaged: true,
                    ..ctx
                },
                LearnError::InCombat,
            ),
            (LearnContext { level: 4, ..ctx }, LearnError::LevelRequired),
            (
                LearnContext {
                    position: Vec2Fixed::new(Fixed::from_raw(129_001), Fixed::from_tiles(128)),
                    ..ctx
                },
                LearnError::TrainerRequired,
            ),
        ] {
            assert_eq!(prepare_learn(&ledger, &state, changed, skill(3, 1), 1), Err(expected));
        }
        assert_eq!(
            prepare_learn(&ledger, &state, ctx, skill(u32::MAX, 1), 1),
            Err(LearnError::UnknownSkill)
        );
        assert_eq!(prepare_learn(&ledger, &state, ctx, skill(3, 2), 1), Err(LearnError::WrongRank));
        assert_eq!(
            prepare_learn(&ledger, &state, ctx, skill(142, 1), 1),
            Err(LearnError::DeferredSkill)
        );
        assert_eq!(
            prepare_learn(&ledger, &state, ctx, skill(1177, 1), 1),
            Err(LearnError::NotLearnable)
        );
        assert_eq!(ledger, before);
        let mut casting = state.clone();
        casting.cast = CastState::Casting(super::super::phase3_entity::tests::cast());
        assert_eq!(
            prepare_learn(&ledger, &casting, ctx, skill(3, 1), 1),
            Err(LearnError::InCombat)
        );
        let mut malformed = ledger.clone();
        malformed.sp = u64::MAX;
        assert_eq!(
            prepare_learn(&malformed, &state, ctx, skill(3, 1), 1),
            Err(LearnError::InvalidState)
        );
        // A valid source item prerequisite must be checked by the mutation, even if the
        // catalogue/UI preflight was bypassed. This changes an in-memory test registry only.
        let mut encoded = serde_json::to_value(classes.as_ref()).unwrap();
        let row = encoded["classes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|c| c["id"] == 0)
            .unwrap()["skill_tree"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|r| r["skill_id"] == 3 && r["skill_level"] == 1)
            .unwrap();
        row["required_items"] = serde_json::json!([{ "item":"l2.item.1", "count":1 }]);
        let with_items: ClassRegistry = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            prepare_learn(
                &ledger,
                &state,
                LearnContext {
                    classes: &with_items,
                    ..ctx
                },
                skill(3, 1),
                1
            ),
            Err(LearnError::ItemRequired)
        );
    }
}
